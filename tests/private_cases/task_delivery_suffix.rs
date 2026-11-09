//! Independently appended delivery facts must not bypass Project-aware replay.
use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;

use crate::adapters::artifact_store::LocalArtifactStore;
use crate::adapters::project_event_store::{
    Project, ProjectEventStream, read_existing_research_events, read_existing_session_events,
};
use crate::app::session::SessionProjection;
use crate::domain::events::{EventId, OccurredAt, StreamId};
use crate::domain::session::{SessionEvent, SessionReducer};
use crate::domain::tools::{ExecutionId, ExecutionReceipt, ToolResultStatus};
use crate::ports::EventSink;
use crate::rsi_subscription_continuation::continuation_regression::{completed, read_request};
use crate::support::TemporaryDirectory;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn independent_delivery_suffix_uses_project_replay_and_tool_suffix_stays_cheap() {
    let root = TemporaryDirectory::new("task-delivery-independent-suffix");
    assert!(
        Command::new(env!("CARGO_BIN_EXE_echidna"))
            .current_dir(root.path())
            .arg("init")
            .status()
            .unwrap()
            .success()
    );
    let agent = root.path().join("agent");
    fs::create_dir(&agent).unwrap();
    fs::write(agent.join("persona.md"), "A neutral synthetic assistant.").unwrap();
    fs::write(agent.join("work.md"), "Preserve the original task.").unwrap();
    fs::write(agent.join("agent.toml"), "schema_version = 1\nid = \"suffix-fixture\"\nname = \"Suffix fixture\"\n[instructions]\npersona_file = \"persona.md\"\nwork_file = \"work.md\"\nmethod_files = []\n[context]\npersonal = \"none\"\nproject = \"none\"\nmemory = \"none\"\n[tools]\n").unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let selector = read_request(&mut socket).await;
        let decision = crate::task_subscription_epoch::selection_for_wire(&selector);
        let body = completed("suffix-selector", Vec::new(), Some(&decision));
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        socket.write_all(header.as_bytes()).await.unwrap();
        socket.write_all(body.as_bytes()).await.unwrap();
        let (mut socket, _) = listener.accept().await.unwrap();
        let main = read_request(&mut socket).await;
        assert!(!main.to_string().contains("TASK_CONTEXT_CANDIDATES_V1"));
        let body = completed("suffix-main", Vec::new(), Some("FIRST_DONE"));
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        socket.write_all(header.as_bytes()).await.unwrap();
        socket.write_all(body.as_bytes()).await.unwrap();
    });
    let config = root.path().join("provider.toml");
    fs::write(
        &config,
        crate::provider_config::fixture()
            .replace("http://127.0.0.1:1/v1", &base_url)
            .replace("context_window = 32768", "context_window = 272000"),
    )
    .unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .current_dir(root.path())
        .args([
            "chat",
            "--config",
            config.to_str().unwrap(),
            "--agent",
            agent.to_str().unwrap(),
        ])
        .env("CYBER_AGENT_PRIMARY_API_KEY", "synthetic-loopback-key")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"First task.\n/exit\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.await.unwrap();
    let project = Project::discover(root.path()).unwrap();
    let session_name = fs::read_dir(root.path().join(".echidna/sessions"))
        .unwrap()
        .map(|entry| entry.unwrap())
        .find(|entry| entry.file_type().unwrap().is_dir())
        .unwrap()
        .file_name()
        .into_string()
        .unwrap();
    let session = StreamId::new(session_name);
    let events = read_existing_session_events(&project, &session).unwrap();
    let research = read_existing_research_events(&project).unwrap();
    let blobs = LocalArtifactStore::for_project(&project);
    let mut projection = SessionProjection::replay_project_verified(
        project.project_id().clone(),
        &events,
        &research,
        || Ok(Default::default()),
        &blobs,
    )
    .unwrap();
    assert_eq!(
        projection
            .state()
            .latest_task_context_selection()
            .unwrap()
            .1
            .request_proof_version,
        crate::domain::session::TASK_REQUEST_PROOF_VERSION
    );
    let cursor = {
        let mut writer = project.open_writer().unwrap();
        let (store, _) = writer
            .open_session_projection(session.clone(), None)
            .unwrap();
        store.projection_cursor()
    };
    // The supported independent Tool suffix remains incrementally ingestible.
    {
        let mut writer = project.open_writer().unwrap();
        let mut store = writer.open_session(session.clone(), None).unwrap();
        let receipt = ExecutionReceipt {
            execution_id: ExecutionId::new("independent-fixture-execution"),
            plan_fingerprint: "fixture-plan".to_owned(),
            resolved_tool_plan_fingerprint: None,
            status: ToolResultStatus::Succeeded,
            exit_reason: None,
            output_artifact_ids: Vec::new(),
            output_captures: Vec::new(),
            environment_change: None,
            rerun_reason: None,
            cache_source: None,
            cache_validation: None,
            file_mutations: Vec::new(),
        };
        let event = SessionEvent::ExecutionReceiptRecorded { receipt }
            .into_envelope(
                EventId::new("independent-receipt"),
                ProjectEventStream::Session(session.clone()).stream_id(),
                store.next_sequence().unwrap(),
                OccurredAt::new("2026-11-07T00:00:00Z"),
            )
            .unwrap();
        store.append_event(&event).unwrap();
    }
    let cursor = {
        let mut writer = project.open_writer().unwrap();
        let (store, suffix) = writer
            .open_session_projection(session.clone(), Some(&cursor))
            .unwrap();
        assert!(!projection.suffix_requires_project_replay(&suffix).unwrap());
        projection.ingest_durable_verified(suffix, &blobs).unwrap();
        assert_eq!(projection.state().execution_receipt_count(), 1);
        store.projection_cursor()
    };
    // Append a structurally valid but stale primary audit under the writer
    // lease. Its logical request predates the already completed answer.
    let audit = events
        .iter()
        .find(|event| {
            event.event_type.as_str() == "session.request_audit_recorded"
                && event.payload["sampling_step_id"].is_string()
        })
        .unwrap();
    let mut payload: Value = audit.payload.clone();
    payload["audit_event_id"] = json!("independent-stale-audit");
    let manifest = serde_json::from_value(payload).unwrap();
    {
        let mut writer = project.open_writer().unwrap();
        let mut store = writer.open_session(session.clone(), None).unwrap();
        let event = SessionEvent::RequestAuditRecorded { manifest }
            .into_envelope(
                EventId::new("independent-stale-audit"),
                ProjectEventStream::Session(session.clone()).stream_id(),
                store.next_sequence().unwrap(),
                OccurredAt::new("2026-11-07T00:00:01Z"),
            )
            .unwrap();
        store.append_event(&event).unwrap();
    }
    let mut writer = project.open_writer().unwrap();
    let (store, suffix) = writer
        .open_session_projection(session.clone(), Some(&cursor))
        .unwrap();
    assert!(projection.suffix_requires_project_replay(&suffix).unwrap());
    drop(store);
    drop(writer);
    let all_events = read_existing_session_events(&project, &session).unwrap();
    SessionReducer
        .reduce(all_events.clone())
        .expect("the independent suffix passes structural reduction");
    let error = SessionProjection::replay_project_verified(
        project.project_id().clone(),
        &all_events,
        &research,
        || Ok(Default::default()),
        &blobs,
    )
    .err()
    .expect("stale independent audit must fail Project-aware replay");
    assert!(
        error
            .to_string()
            .contains("primary audit differs from independently compiled task source"),
        "{error}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn provider_rejection_reset_survives_physical_retries_and_argument_correction() {
    use std::time::Duration;

    let root = TemporaryDirectory::new("task-delivery-rejection-correction");
    assert!(
        Command::new(env!("CARGO_BIN_EXE_echidna"))
            .current_dir(root.path())
            .arg("init")
            .status()
            .unwrap()
            .success()
    );
    let agent = root.path().join("agent");
    fs::create_dir(&agent).unwrap();
    fs::write(agent.join("persona.md"), "A neutral synthetic assistant.").unwrap();
    fs::write(
        agent.join("work.md"),
        "Keep the requested task and observed outcomes.",
    )
    .unwrap();
    fs::write(agent.join("agent.toml"), "schema_version = 1\nid = \"fallback-fixture\"\nname = \"Fallback fixture\"\n[instructions]\npersona_file = \"persona.md\"\nwork_file = \"work.md\"\nmethod_files = []\n[context]\npersonal = \"none\"\nproject = \"none\"\nmemory = \"none\"\n[tools]\nrun_command = \"direct\"\n").unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let mut wires = Vec::new();
        for index in 0..7 {
            let (mut socket, _) = tokio::time::timeout(Duration::from_secs(60), listener.accept())
                .await
                .expect("expected physical attempt")
                .unwrap();
            let wire = read_request(&mut socket).await;
            let (status, body) = match index {
                0 => {
                    let decision = crate::task_subscription_epoch::selection_for_wire(&wire);
                    ("200 OK", completed("selector", Vec::new(), Some(&decision)))
                }
                1 => (
                    "200 OK",
                    completed(
                        "accepted-tool",
                        vec![json!({"type":"function_call", "call_id":"once",
                            "name":"run_command", "arguments":json!({"argv":["/bin/sh", "-c", "printf x >> effect-count.txt"]}).to_string()})],
                        None,
                    ),
                ),
                // A stream ending before completion exercises the existing transport retry.
                2 | 4 => ("200 OK", "data: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\"}\n\n".to_owned()),
                3 => ("400 Bad Request", json!({"error":{"code":"previous_response_not_found","message":"synthetic rejected predecessor"}}).to_string()),
                5 => (
                    "200 OK",
                    completed(
                        "malformed",
                        vec![json!({"type":"function_call", "call_id":"invalid",
                            "name":"run_command", "arguments":"{"})],
                        None,
                    ),
                ),
                6 => ("200 OK", completed("accepted-final", Vec::new(), Some("DONE"))),
                _ => unreachable!(),
            };
            let content_type = if index == 3 {
                "application/json"
            } else {
                "text/event-stream"
            };
            socket.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes()).await.unwrap();
            socket.write_all(body.as_bytes()).await.unwrap();
            socket.shutdown().await.unwrap();
            wires.push(wire);
        }
        wires
    });
    let config = root.path().join("provider.toml");
    fs::write(
        &config,
        crate::provider_config::fixture()
            .replace("http://127.0.0.1:1/v1", &base_url)
            .replace("context_window = 32768", "context_window = 272000"),
    )
    .unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .current_dir(root.path())
        .args([
            "chat",
            "--config",
            config.to_str().unwrap(),
            "--agent",
            agent.to_str().unwrap(),
        ])
        .env("CYBER_AGENT_PRIMARY_API_KEY", "synthetic-loopback-key")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"Perform the synthetic task once.\n/exit\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let wires = server.await.unwrap();
    assert_eq!(wires[2]["previous_response_id"], "accepted-tool");
    assert_eq!(wires[3]["previous_response_id"], "accepted-tool");
    for wire in &wires[4..] {
        assert!(wire.get("previous_response_id").is_none_or(Value::is_null));
    }
    assert!(wires[6].to_string().contains("Runtime validation notice"));

    let project = Project::discover(root.path()).unwrap();
    let session_name = fs::read_dir(root.path().join(".echidna/sessions"))
        .unwrap()
        .map(|entry| entry.unwrap())
        .find(|entry| entry.file_type().unwrap().is_dir())
        .unwrap()
        .file_name()
        .into_string()
        .unwrap();
    let session = StreamId::new(session_name);
    let events = read_existing_session_events(&project, &session).unwrap();
    let research = read_existing_research_events(&project).unwrap();
    let blobs = LocalArtifactStore::for_project(&project);
    let first_incremental = events
        .iter()
        .position(|event| {
            event.event_type.as_str() == "session.request_audit_recorded"
                && event.payload["delivery"]["mode"] == "incremental"
        })
        .expect("an incremental attempt was actually made");
    SessionProjection::replay_project_verified(
        project.project_id().clone(),
        &events[..=first_incremental],
        &research,
        || Ok(Default::default()),
        &blobs,
    )
    .expect("the original unaccepted incremental prefix is valid");
    // Keep every source and the original accepted output unchanged. Replace
    // the first incremental with a coherent Full audit, and stop at that
    // unaccepted attempt so no later fact can independently justify a reset.
    let mut forged = events[..=first_incremental].to_vec();
    let payload = &mut forged[first_incremental].payload;
    payload["delivery"] = json!({"mode":"full", "reason":{"full":"PROVIDER_REJECTED"}});
    payload["components"] = payload["full_logical_components"].clone();
    payload["context_delivery_map"] = payload["full_context_delivery_map"].clone();
    payload["delivered_input"] = payload["full_logical_input"].clone();
    let manifest: crate::domain::request_audit::RequestAuditManifest =
        serde_json::from_value(payload.clone()).unwrap();
    manifest
        .validate()
        .expect("forged audit is internally coherent");
    SessionReducer
        .reduce(forged.clone())
        .expect("structural Session validation alone is insufficient");
    assert!(
        SessionProjection::replay_project_verified(
            project.project_id().clone(),
            &forged,
            &research,
            || Ok(Default::default()),
            &blobs,
        )
        .is_err(),
        "an audit cannot establish its own ProviderRejected reset"
    );
    let projection = SessionProjection::replay_project_verified(
        project.project_id().clone(),
        &events,
        &research,
        || Ok(Default::default()),
        &blobs,
    )
    .expect("verified incremental, Full retries and correction must replay");
    assert_eq!(projection.state().execution_receipt_count(), 1);
    let resumed = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .current_dir(root.path())
        .args([
            "chat",
            "--config",
            config.to_str().unwrap(),
            "--resume",
            session.as_str(),
        ])
        .env("CYBER_AGENT_PRIMARY_API_KEY", "synthetic-loopback-key")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut resumed = resumed;
    resumed.stdin.take().unwrap().write_all(b"/exit\n").unwrap();
    let output = resumed.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let after = read_existing_session_events(&project, &session).unwrap();
    assert_eq!(
        SessionReducer
            .reduce(after)
            .unwrap()
            .execution_receipt_count(),
        1
    );
    assert_eq!(
        fs::read(root.path().join("effect-count.txt")).unwrap(),
        b"x"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn proof6_compaction_references_and_retry_source_prefix_are_verified() {
    use crate::domain::canonical_json::canonical_blake3_hash;
    use std::time::Duration;

    let root = TemporaryDirectory::new("proof6-compaction-source-attempts");
    assert!(
        Command::new(env!("CARGO_BIN_EXE_echidna"))
            .current_dir(root.path())
            .arg("init")
            .status()
            .unwrap()
            .success()
    );
    let agent = root.path().join("agent");
    fs::create_dir(&agent).unwrap();
    fs::write(agent.join("persona.md"), "A neutral synthetic assistant.").unwrap();
    fs::write(
        agent.join("work.md"),
        "Keep original constraints and observed outcomes.",
    )
    .unwrap();
    fs::write(agent.join("agent.toml"), "schema_version = 1\nid = \"graph-fixture\"\nname = \"Graph fixture\"\n[instructions]\npersona_file = \"persona.md\"\nwork_file = \"work.md\"\nmethod_files = []\n[context]\npersonal = \"none\"\nproject = \"none\"\nmemory = \"none\"\n[tools]\nrun_command = \"direct\"\n").unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        for index in 0..10 {
            let (mut socket, _) = tokio::time::timeout(Duration::from_secs(60), listener.accept())
                .await
                .expect("expected synthetic compaction request")
                .unwrap();
            let wire = read_request(&mut socket).await;
            let body = match index {
                0 | 3 => completed(&format!("selection-{index}"), Vec::new(),
                    Some(&crate::task_subscription_epoch::selection_for_wire(&wire))),
                1 | 4 | 5 | 6 => completed(&format!("response-{index}"),
                    vec![json!({"type":"function_call", "call_id":format!("call-{index}"),
                        "name":"run_command", "arguments":json!({"argv":["/bin/echo", format!("observed-{index}")]}).to_string()})], None),
                2 => completed("first-done", Vec::new(), Some("First task complete.")),
                7 => completed("second-done", Vec::new(), Some(&format!("Second task. {}", "observed history ".repeat(8000)))),
                8 => "data: {\"type\":\"response.output_text.delta\",\"delta\":\"partial handoff\"}\n\n".to_owned(),
                9 => completed("handoff", Vec::new(), Some(&json!({
                    "concise_progress":"Observed synthetic work.", "unresolved_questions":[],
                    "next_step_notes":["Keep original sources."]}).to_string())),
                _ => unreachable!(),
            };
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes()).await.unwrap();
            socket.write_all(body.as_bytes()).await.unwrap();
            socket.shutdown().await.unwrap();
        }
    });
    let config = root.path().join("provider.toml");
    fs::write(
        &config,
        crate::provider_config::fixture()
            .replace("http://127.0.0.1:1/v1", &base_url)
            .replace("context_window = 32768", "context_window = 272000"),
    )
    .unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .current_dir(root.path())
        .args([
            "chat",
            "--config",
            config.to_str().unwrap(),
            "--agent",
            agent.to_str().unwrap(),
        ])
        .env("CYBER_AGENT_PRIMARY_API_KEY", "synthetic-loopback-key")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"Do the first unrelated task.\nDo the second task.\n/compact\n/exit\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.await.unwrap();
    let project = Project::discover(root.path()).unwrap();
    let session = StreamId::new(
        fs::read_dir(root.path().join(".echidna/sessions"))
            .unwrap()
            .map(|entry| entry.unwrap())
            .find(|entry| entry.file_type().unwrap().is_dir())
            .unwrap()
            .file_name()
            .into_string()
            .unwrap(),
    );
    let events = read_existing_session_events(&project, &session).unwrap();
    let research = read_existing_research_events(&project).unwrap();
    let blobs = LocalArtifactStore::for_project(&project);
    let replay = |source: &[crate::domain::events::EventEnvelope<Value>]| {
        SessionProjection::replay_project_verified(
            project.project_id().clone(),
            source,
            &research,
            || Ok(Default::default()),
            &blobs,
        )
    };
    let projection = replay(&events).expect("original task compaction must replay");
    assert_eq!(projection.state().execution_receipt_count(), 4);
    let summary = projection.state().latest_compaction().unwrap().summary();
    let selected = serde_json::to_value(summary).unwrap();
    let receipts = selected["task_references"]["execution_receipt_refs"]
        .as_array()
        .unwrap();
    assert_eq!(receipts.len(), 3);
    let all_receipts = events
        .iter()
        .filter(|event| event.event_type.as_str() == "tool.execution_outcome_recorded")
        .map(|event| event.event_id.as_str().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(all_receipts.len(), 4);
    assert_eq!(
        receipts,
        &all_receipts[1..]
            .iter()
            .rev()
            .map(|id| json!(id))
            .collect::<Vec<_>>()
    );

    let summary_index = events
        .iter()
        .position(|event| event.event_type.as_str() == "session.compaction_created")
        .unwrap();
    let check_forged_summary = |references: Value, valid: bool| {
        let mut changed = events.clone();
        let summary = &mut changed[summary_index].payload["summary"];
        summary["task_references"] = references;
        summary["fingerprint"] = json!("");
        let hash = canonical_blake3_hash(summary).unwrap();
        summary["fingerprint"] = json!(hash);
        SessionReducer
            .reduce(changed.clone())
            .expect("forged Summary remains structurally valid");
        let result = replay(&changed);
        if valid {
            result.expect("legal reduced cap must replay");
        } else {
            assert!(
                result.is_err(),
                "graph mismatch must reject the coherent Summary"
            );
        }
    };
    let facts = projection.compaction_source_for(summary).unwrap();
    check_forged_summary(serde_json::to_value(facts.references(1)).unwrap(), true);
    let mut omission = selected["task_references"].clone();
    omission["execution_receipt_refs"]
        .as_array_mut()
        .unwrap()
        .remove(1);
    check_forged_summary(omission, false);
    let mut substitution = selected["task_references"].clone();
    substitution["execution_receipt_refs"][1] = json!(all_receipts[0]);
    check_forged_summary(substitution, false);

    let capture = events
        .iter()
        .position(|event| event.event_type.as_str() == "session.compaction_context_captured")
        .unwrap();
    let sample = events
        .iter()
        .position(|event| event.event_type.as_str() == "session.compaction_sample_recorded")
        .unwrap();
    let attempts = (capture..sample)
        .filter(|index| events[*index].event_type.as_str() == "session.request_audit_recorded")
        .collect::<Vec<_>>();
    assert_eq!(attempts.len(), 2);
    let mut changed = events.clone();
    changed[attempts[0]].payload["full_logical_components"][0]["blob_id"] = json!("0".repeat(64));
    let error = replay(&changed)
        .err()
        .expect("changing a recorded retry audit must fail");
    assert!(
        error
            .to_string()
            .contains("Compaction source prefix changed during model sampling"),
        "{error}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn proof6_compaction_keeps_latest_receipts_across_the_64_item_bound() {
    use std::time::Duration;

    let root = TemporaryDirectory::new("proof6-compaction-receipt-order");
    assert!(
        Command::new(env!("CARGO_BIN_EXE_echidna"))
            .current_dir(root.path())
            .arg("init")
            .status()
            .unwrap()
            .success()
    );
    let agent = root.path().join("agent");
    fs::create_dir(&agent).unwrap();
    fs::write(agent.join("persona.md"), "A neutral synthetic assistant.").unwrap();
    fs::write(agent.join("work.md"), "Retain actual observed outcomes.").unwrap();
    fs::write(agent.join("agent.toml"), "schema_version = 1\nid = \"order-fixture\"\nname = \"Order fixture\"\n[instructions]\npersona_file = \"persona.md\"\nwork_file = \"work.md\"\nmethod_files = []\n[context]\npersonal = \"none\"\nproject = \"none\"\nmemory = \"none\"\n[tools]\nrun_command = \"direct\"\n").unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        for index in 0..4 {
            let (mut socket, _) = tokio::time::timeout(Duration::from_secs(120), listener.accept())
                .await
                .expect("expected bounded order request")
                .unwrap();
            let wire = read_request(&mut socket).await;
            let body = match index {
                0 => completed("selector", Vec::new(), Some(&crate::task_subscription_epoch::selection_for_wire(&wire))),
                1 => completed("batch", (0..70).map(|id| json!({"type":"function_call", "call_id":format!("ordered-{id}"), "name":"run_command", "arguments":json!({"argv":["/bin/echo",format!("batch-{id}")]}).to_string()})).collect(), None),
                2 => completed("batch-done", Vec::new(), Some(&format!("Observed batch. {}", "synthetic historical material ".repeat(8000)))),
                3 => completed("handoff", Vec::new(), Some(&json!({"concise_progress":"Observed completed batch.","unresolved_questions":[],"next_step_notes":["Keep sources."]}).to_string())),
                _ => unreachable!(),
            };
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes()).await.unwrap();
            socket.write_all(body.as_bytes()).await.unwrap();
            socket.shutdown().await.unwrap();
        }
    });
    let config = root.path().join("provider.toml");
    fs::write(
        &config,
        crate::provider_config::fixture()
            .replace("http://127.0.0.1:1/v1", &url)
            .replace("context_window = 32768", "context_window = 272000"),
    )
    .unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .current_dir(root.path())
        .args([
            "chat",
            "--config",
            config.to_str().unwrap(),
            "--agent",
            agent.to_str().unwrap(),
        ])
        .env("CYBER_AGENT_PRIMARY_API_KEY", "synthetic-loopback-key")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"Observe all synthetic batches.\n/compact\n/exit\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.await.unwrap();
    let project = Project::discover(root.path()).unwrap();
    let session = StreamId::new(
        fs::read_dir(root.path().join(".echidna/sessions"))
            .unwrap()
            .map(|entry| entry.unwrap())
            .find(|entry| entry.file_type().unwrap().is_dir())
            .unwrap()
            .file_name()
            .into_string()
            .unwrap(),
    );
    let events = read_existing_session_events(&project, &session).unwrap();
    let research = read_existing_research_events(&project).unwrap();
    let blobs = LocalArtifactStore::for_project(&project);
    let projection = SessionProjection::replay_project_verified(
        project.project_id().clone(),
        &events,
        &research,
        || Ok(Default::default()),
        &blobs,
    )
    .expect("bounded graph must replay");
    assert_eq!(projection.state().execution_receipt_count(), 70);
    let original = events
        .iter()
        .filter(|event| event.event_type.as_str() == "tool.execution_outcome_recorded")
        .map(|event| event.event_id.as_str().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(original.len(), 70);
    let summary =
        serde_json::to_value(projection.state().latest_compaction().unwrap().summary()).unwrap();
    let actual = summary["task_references"]["execution_receipt_refs"]
        .as_array()
        .unwrap();
    assert_eq!(actual.len(), 64);
    assert_eq!(
        actual,
        &original
            .iter()
            .rev()
            .take(64)
            .map(|id| json!(id))
            .collect::<Vec<_>>()
    );
    let output = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .current_dir(root.path())
        .args([
            "chat",
            "--config",
            config.to_str().unwrap(),
            "--resume",
            session.as_str(),
        ])
        .env("CYBER_AGENT_PRIMARY_API_KEY", "synthetic-loopback-key")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        SessionReducer
            .reduce(read_existing_session_events(&project, &session).unwrap())
            .unwrap()
            .execution_receipt_count(),
        70
    );
}
