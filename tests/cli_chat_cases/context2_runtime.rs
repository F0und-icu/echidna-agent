//! Real CLI, loopback Provider and durable multi-node handoff regression.

use super::automatic_compaction::continue_from_selected_summary;
use super::task_context::{
    ScriptedResponse::{ContinueSelection, InitialSelection, Response, SelectionDecision},
    selector_candidates, spawn_scripted_responses_server,
};
use super::*;

fn canonical_fixture_hash(value: &(impl serde::Serialize + ?Sized)) -> String {
    let value = serde_json::to_value(value).unwrap();
    blake3::hash(&serde_json::to_vec(&value).unwrap())
        .to_hex()
        .to_string()
}

/// Rebuild a mutually consistent forged plan and audited inputs while leaving
/// the previously captured Session prefix untouched.
fn forge_common_context(root: &std::path::Path, events: &[Value]) -> Vec<Value> {
    use echidna::domain::context::{ContextContentHash, ContextFragmentContent};
    use echidna::domain::model::{ContentItem, ModelInputIR};
    use echidna::domain::request_audit::RequestAuditManifest;
    use sha2::{Digest, Sha256};
    let blobs = root.join(".echidna/artifacts/blobs/sha256");
    let mut forged = events.to_vec();
    let created = forged
        .iter()
        .position(|e| e["event_type"] == "session.compaction_created")
        .unwrap();
    let original_blob = forged[created]["payload"]["summary"]["source_plan_blob_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let old_plan_hash = forged[created]["payload"]["summary"]["source_plan_hash"]
        .as_str()
        .unwrap()
        .to_owned();
    let original_plan = fs::read_to_string(blobs.join(original_blob)).unwrap();
    let mut plan: Value = serde_json::from_str(&original_plan).unwrap();
    let old_template = plan["nodes"][0]["template_hash"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        plan["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|node| node["template_hash"] == old_template)
    );
    let first = forged
        .iter()
        .position(|e| {
            e["event_type"] == "session.request_audit_recorded"
                && e["payload"]["provider_session"]["id"]
                    .as_str()
                    .is_some_and(|id| id.starts_with("compaction:"))
        })
        .unwrap();
    let manifest: RequestAuditManifest =
        serde_json::from_value(forged[first]["payload"].clone()).unwrap();
    let mut input = serde_json::to_value(
        manifest
            .reconstruct(|id| fs::read(blobs.join(id.as_str())))
            .unwrap(),
    )
    .unwrap();
    let mut map = forged[first]["payload"]["full_context_delivery_map"].clone();
    let common = map["content_items"]
        .as_array()
        .unwrap()
        .iter()
        .position(|entry| entry["provenance"]["slot"] == "project_state")
        .unwrap();
    let source = map["content_items"]
        .as_array()
        .unwrap()
        .iter()
        .position(|entry| entry["fragment_id"] == "compaction:source")
        .unwrap();
    let mut common_item = input["content_items"][common].clone();
    let text = common_item["text"].as_str().unwrap();
    common_item["text"] = format!("{text} FORGED-COMMON-CONTEXT").into();
    let typed_common: ContentItem = serde_json::from_value(common_item.clone()).unwrap();
    let freshness = ContextContentHash::for_fragment(
        &ContextFragmentContent::ContentItems {
            items: vec![typed_common],
        },
        None,
    )
    .unwrap()
    .as_str()
    .to_owned();
    input["content_items"][common] = common_item.clone();
    map["content_items"][common]["content_hash"] = canonical_fixture_hash(&common_item).into();
    map["content_items"][common]["provenance"]["freshness"]["source_revision_or_hash"] =
        freshness.clone().into();
    input["content_items"][source] = json!({"compaction_source_placeholder": true});
    map["content_items"][source]["content_hash"] = "compaction-source-placeholder".into();
    let template = canonical_fixture_hash(&(input, map));
    let plan_bytes = original_plan.replace(&old_template, &template).into_bytes();
    for node in plan["nodes"].as_array_mut().unwrap() {
        node["template_hash"] = template.clone().into();
    }
    let new_plan_hash = canonical_fixture_hash(&plan);
    let new_plan_blob = format!("{:x}", Sha256::digest(&plan_bytes));
    fs::write(blobs.join(&new_plan_blob), plan_bytes).unwrap();
    let publish_item = |item: &Value| {
        let bytes = serde_json::to_vec(item).unwrap();
        let blob = format!("{:x}", Sha256::digest(&bytes));
        fs::write(blobs.join(&blob), &bytes).unwrap();
        (blob, bytes.len(), canonical_fixture_hash(item))
    };
    for event in &mut forged[first..created] {
        if event["event_type"] == "session.request_audit_recorded" {
            let payload = &mut event["payload"];
            let manifest: RequestAuditManifest = serde_json::from_value(payload.clone()).unwrap();
            let original = manifest
                .reconstruct(|id| fs::read(blobs.join(id.as_str())))
                .unwrap();
            let mut input = serde_json::to_value(original).unwrap();
            input["content_items"][common] = common_item.clone();
            let source_text = input["content_items"][source]["text"]
                .as_str()
                .unwrap()
                .replace(&old_plan_hash, &new_plan_hash);
            input["content_items"][source]["text"] = source_text.into();
            for index in [common, source] {
                let item = &input["content_items"][index];
                let (blob, size, hash) = publish_item(item);
                for key in ["components", "full_logical_components"] {
                    let component = payload[key]
                        .as_array_mut()
                        .unwrap()
                        .iter_mut()
                        .find(|c| {
                            c["identity"]["kind"] == "content_item"
                                && c["identity"]["index"] == index
                        })
                        .unwrap();
                    component["blob_id"] = blob.clone().into();
                    component["size_bytes"] = size.into();
                    component["semantic_hash"] = hash.clone().into();
                }
                for key in ["context_delivery_map", "full_context_delivery_map"] {
                    payload[key]["content_items"][index]["content_hash"] = hash.clone().into();
                    if index == common {
                        payload[key]["content_items"][index]["provenance"]["freshness"]["source_revision_or_hash"] =
                            freshness.clone().into();
                    }
                }
            }
            let typed_input: ModelInputIR = serde_json::from_value(input).unwrap();
            let hash = canonical_fixture_hash(&typed_input);
            payload["delivered_input"]["semantic_hash"] = hash.clone().into();
            payload["full_logical_input"]["semantic_hash"] = hash.into();
            let scope = payload["provider_session"]["id"]
                .as_str()
                .unwrap()
                .replace(&old_plan_hash, &new_plan_hash);
            payload["provider_session"]["id"] = scope.into();
            let rebuilt: RequestAuditManifest = serde_json::from_value(payload.clone()).unwrap();
            assert_eq!(
                rebuilt
                    .reconstruct(|id| fs::read(blobs.join(id.as_str())))
                    .unwrap(),
                typed_input
            );
        } else if event["event_type"] == "session.compaction_sample_recorded" {
            let sample = &mut event["payload"];
            sample["graph"]["plan_blob_id"] = new_plan_blob.clone().into();
            sample["graph"]["plan_hash"] = new_plan_hash.clone().into();
            sample["graph"]["node"]["template_hash"] = template.clone().into();
            let scope = sample["provider_session"]["id"]
                .as_str()
                .unwrap()
                .replace(&old_plan_hash, &new_plan_hash);
            sample["provider_session"]["id"] = scope.into();
        } else if event["event_type"] == "model.usage_recorded" {
            let scope = event["payload"]["provider_session"]["id"]
                .as_str()
                .unwrap()
                .replace(&old_plan_hash, &new_plan_hash);
            event["payload"]["provider_session"]["id"] = scope.into();
        }
    }
    let prefix_hash = canonical_fixture_hash(&forged[..created]);
    let summary = &mut forged[created]["payload"]["summary"];
    summary["source_plan_blob_id"] = new_plan_blob.into();
    summary["source_plan_hash"] = new_plan_hash.into();
    summary["task_references"]["source"]["stream_hash"] = prefix_hash.into();
    summary["fingerprint"] = "".into();
    summary["fingerprint"] = canonical_fixture_hash(summary).into();
    forged
}

fn writer_definition(root: &std::path::Path) -> std::path::PathBuf {
    let definition = root.join("writer");
    fs::create_dir(&definition).unwrap();
    fs::write(definition.join("persona.md"), "A careful technical writer.").unwrap();
    fs::write(definition.join("work.md"), "Keep the task constraints.").unwrap();
    fs::write(
        definition.join("agent.toml"),
        r#"schema_version = 1
id = "context2-writer"
name = "Context2 writer"
[instructions]
persona_file = "persona.md"
work_file = "work.md"
method_files = []
[context]
personal = "none"
project = "none"
memory = "none"
[tools]
view_image = "direct"
"#,
    )
    .unwrap();
    definition.join("agent.toml")
}

#[test]
fn long_completed_source_uses_bounded_nodes_and_one_verified_root() {
    let root = TemporaryDirectory::new("context2-segmented-handoff");
    initialize_project(root.path());
    let definition = writer_definition(root.path());
    let original = "NEUTRAL-SOURCE-".repeat(10_000);
    let server = spawn_scripted_responses_server(vec![
        InitialSelection,
        Response(200, text_response(&original, "first")),
        Response(
            200,
            compaction_summary_response("First bounded source section.", "leaf-1"),
        ),
        Response(
            200,
            compaction_summary_response("Second bounded source section.", "leaf-2"),
        ),
        Response(
            200,
            compaction_summary_response("Merged durable source sections.", "root"),
        ),
        SelectionDecision(continue_from_selected_summary),
        SelectionDecision(continue_from_selected_summary),
        Response(200, text_response("Completed follow-up.", "last")),
    ]);
    let config = write_config(root.path(), &server.base_url);
    let output = run_chat(
        root.path(),
        &config,
        "Write the document.\n/compact\nContinue.\n/exit\n",
        &["--agent", definition.to_str().unwrap()],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let tail = stdout
        .char_indices()
        .rev()
        .nth(3000)
        .map_or(stdout.as_ref(), |(index, _)| &stdout[index..]);
    if !output.status.success() {
        let project = Project::discover(root.path()).unwrap();
        let state = reduced_session(&project, &session_id(root.path()));
        eprintln!(
            "compaction diagnostics: audits={} history_items={} latest={:?} received_requests={}",
            state.request_audits().len(),
            state.history().items().len(),
            state
                .latest_compaction()
                .map(|item| item.summary().schema_version()),
            server.requests.try_iter().count()
        );
    }
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        tail,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Compaction revision 1 applied"));
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains("[turn stopped]"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let wires = (0..8)
        .map(|_| request_body(&server.requests.recv_timeout(F2A_WAIT).unwrap()))
        .collect::<Vec<_>>();
    server.join();
    selector_candidates(&wires[0]);
    for wire in &wires[2..5] {
        assert!(
            wire.get("tools")
                .and_then(|value| value.as_array())
                .is_none_or(Vec::is_empty)
        );
        assert!(wire.get("previous_response_id").is_none());
    }
    selector_candidates(&wires[5]);
    selector_candidates(&wires[6]);
    assert_eq!(wires[1]["tools"], wires[7]["tools"]);
    let project = Project::discover(root.path()).unwrap();
    let state = reduced_session(&project, &session_id(root.path()));
    let compaction = state
        .latest_compaction()
        .expect("verified root was adopted");
    assert_eq!(compaction.summary().schema_version(), 7);
    assert!(
        serde_json::to_string(state.history().items())
            .unwrap()
            .contains(&original)
    );
    assert_eq!(state.request_audits().len(), 8);
    assert_audits_reconstruct(root.path(), &state);
    let resumed = run_chat(
        root.path(),
        &config,
        "/context\n/exit\n",
        &["--resume", "latest"],
    );
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    let resumed_context = String::from_utf8_lossy(&resumed.stdout);
    assert!(
        !resumed_context.contains("required Context fragments need"),
        "Resume must compile the durable selected view: {resumed_context}"
    );
    assert!(!resumed_context.contains(&original));
    let session = session_id(root.path());
    let before = session_events(root.path(), &session);
    let forged = forge_common_context(root.path(), &before);
    let capture_index = before
        .iter()
        .position(|event| event["event_type"] == "session.compaction_context_captured")
        .unwrap();
    assert_eq!(&forged[..=capture_index], &before[..=capture_index]);
    let decoded = forged
        .iter()
        .cloned()
        .map(|event| serde_json::from_value::<EventEnvelope<Value>>(event).unwrap())
        .collect::<Vec<_>>();
    SessionReducer
        .reduce(decoded)
        .expect("forged audit and plan retain a valid Session projection");
    let event_path = root
        .path()
        .join(".echidna/sessions")
        .join(&session)
        .join("events.jsonl");
    let original_events = fs::read(&event_path).unwrap();
    let altered_events = forged
        .iter()
        .map(|event| format!("{}\n", serde_json::to_string(event).unwrap()))
        .collect::<String>();
    fs::write(&event_path, altered_events).unwrap();
    let rejected_forgery = run_chat(root.path(), &config, "/exit\n", &["--resume", "latest"]);
    assert!(!rejected_forgery.status.success());
    assert!(
        String::from_utf8_lossy(&rejected_forgery.stderr)
            .contains("Compaction node audit scope or full input mismatch"),
        "{}",
        String::from_utf8_lossy(&rejected_forgery.stderr)
    );
    assert_eq!(session_events(root.path(), &session), forged);
    fs::write(&event_path, original_events).unwrap();
    let capture_blob = before
        .iter()
        .find(|event| event["event_type"] == "session.compaction_context_captured")
        .unwrap()["payload"]["blob_id"]
        .as_str()
        .unwrap();
    let capture_path = root
        .path()
        .join(".echidna/artifacts/blobs/sha256")
        .join(capture_blob);
    let capture_bytes = fs::read(&capture_path).unwrap();
    fs::remove_file(&capture_path).unwrap();
    let missing = run_chat(root.path(), &config, "/exit\n", &["--resume", "latest"]);
    assert!(!missing.status.success());
    assert!(
        String::from_utf8_lossy(&missing.stderr).contains("Compaction context Blob read failed")
    );
    assert_eq!(session_events(root.path(), &session), before);
    fs::write(&capture_path, b"corrupt capture").unwrap();
    let corrupt = run_chat(root.path(), &config, "/exit\n", &["--resume", "latest"]);
    assert!(!corrupt.status.success());
    assert!(
        String::from_utf8_lossy(&corrupt.stderr).contains("Compaction context Blob read failed")
    );
    assert_eq!(session_events(root.path(), &session), before);
    fs::write(&capture_path, capture_bytes).unwrap();
    let plan_blob = before
        .iter()
        .find(|event| event["event_type"] == "session.compaction_created")
        .unwrap()["payload"]["summary"]["source_plan_blob_id"]
        .as_str()
        .unwrap();
    fs::write(
        root.path()
            .join(".echidna/artifacts/blobs/sha256")
            .join(plan_blob),
        b"corrupt fixture plan",
    )
    .unwrap();
    let rejected = run_chat(root.path(), &config, "/exit\n", &["--resume", "latest"]);
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("Compaction plan Blob read failed"));
    assert_eq!(session_events(root.path(), &session), before);
}

#[test]
fn explicit_capacity_rejection_resumes_to_commands_without_replaying_or_saving_new_task() {
    let root = TemporaryDirectory::new("context2-capacity-state");
    initialize_project(root.path());
    let definition = writer_definition(root.path());
    let server = spawn_scripted_responses_server(vec![
        InitialSelection,
        Response(
            400,
            r#"{"error":{"code":"context_length_exceeded","message":"too long"}}"#.to_owned(),
        ),
    ]);
    let config = write_config(root.path(), &server.base_url);
    let output = run_chat(
        root.path(),
        &config,
        "ORIGINAL-CAPACITY-TASK\n/resume\n/context\nMUST-NOT-BE-SAVED\n/exit\n",
        &["--agent", definition.to_str().unwrap()],
    );
    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("use /context, /compact or /exit"),
        "{stdout}"
    );
    assert!(
        stdout.contains("Task input not accepted or saved"),
        "{stdout}"
    );
    assert!(
        stdout.contains("No successful model plan is available"),
        "{stdout}"
    );
    assert!(!stdout.contains("[last successful plan]"), "{stdout}");
    selector_candidates(&f2a_request_body(
        &server.requests,
        "capacity initial selection",
    ));
    server.requests.recv_timeout(F2A_WAIT).unwrap();
    assert!(server.requests.try_recv().is_err());
    server.join();
    let project = Project::discover(root.path()).unwrap();
    let state = reduced_session(&project, &session_id(root.path()));
    let history = serde_json::to_string(state.history().items()).unwrap();
    assert!(history.contains("ORIGINAL-CAPACITY-TASK"));
    assert!(!history.contains("MUST-NOT-BE-SAVED"));
    assert_eq!(state.request_audits().len(), 2);
    assert_eq!(state.execution_receipt_count(), 0);
}

#[test]
fn context_keeps_accepted_plan_after_a_later_capacity_rejection() {
    let root = TemporaryDirectory::new("context2-successful-plan");
    initialize_project(root.path());
    let server = spawn_scripted_responses_server(vec![
        InitialSelection,
        Response(200, text_response("First answer.", "accepted")),
        ContinueSelection { usage: None },
        Response(
            400,
            r#"{"error":{"code":"context_length_exceeded","message":"too long"}}"#.to_owned(),
        ),
    ]);
    let config = write_config(root.path(), &server.base_url);
    let output = run_chat(
        root.path(),
        &config,
        "First input.\n/context\nSecond input.\n/context\n/exit\n",
        &[],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let plans = stdout
        .lines()
        .filter(|line| line.contains("[last successful plan]"))
        .collect::<Vec<_>>();
    assert_eq!(plans.len(), 2, "{stdout}");
    assert_eq!(plans[0], plans[1], "{stdout}");
    for index in 0..4 {
        let request = f2a_request_body(&server.requests, "accepted plan request");
        if index == 0 || index == 2 {
            selector_candidates(&request);
        }
    }
    server.join();
}

#[test]
fn incremental_cross_turn_reasoning_records_real_baseline_and_closes_inputs() {
    let root = TemporaryDirectory::new("context2-incremental-delivery");
    initialize_project(root.path());
    let definition = writer_definition(root.path());
    let reasoning = json!({"type":"response.output_item.done", "output_index":0,
        "item":{"type":"reasoning", "status":null,
            "content":[{"type":"reasoning_text","text":"Private fixture reasoning."}],
            "encrypted_content":null}});
    let first = format!(
        "data: {reasoning}\n\n{}",
        text_response("First complete.", "accepted-first")
    );
    let server = spawn_scripted_responses_server(vec![
        InitialSelection,
        Response(200, first),
        ContinueSelection { usage: None },
        Response(200, text_response("Second complete.", "accepted-second")),
    ]);
    let config = write_config(root.path(), &server.base_url);
    let output = run_chat(
        root.path(),
        &config,
        "FIRST-INPUT\nSECOND-INPUT\n/exit\n",
        &["--agent", definition.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    selector_candidates(&f2a_request_body(
        &server.requests,
        "reasoning initial selection",
    ));
    let first_wire = request_body(&server.requests.recv_timeout(F2A_WAIT).unwrap());
    selector_candidates(&request_body(
        &server.requests.recv_timeout(F2A_WAIT).unwrap(),
    ));
    let second_wire = request_body(&server.requests.recv_timeout(F2A_WAIT).unwrap());
    server.join();
    assert!(
        first_wire
            .get("previous_response_id")
            .is_none_or(Value::is_null)
    );
    assert_eq!(second_wire["previous_response_id"], "accepted-first");
    let events = session_events(root.path(), &session_id(root.path()));
    let deliveries = events
        .iter()
        .filter(|event| event["event_type"] == "session.model_delivery_committed")
        .collect::<Vec<_>>();
    assert_eq!(deliveries.len(), 2);
    assert!(deliveries[0]["payload"]["previous_accepted_audit"].is_null());
    assert_eq!(
        deliveries[1]["payload"]["previous_accepted_audit"],
        deliveries[0]["payload"]["audit_event_id"]
    );
    let completed = events
        .iter()
        .filter(|event| {
            event["event_type"] == "session.operator_turn_boundary_recorded"
                && event["payload"]["kind"] == "completed"
        })
        .collect::<Vec<_>>();
    assert_eq!(completed.len(), 2);
    assert_eq!(
        completed[0]["payload"]["closed_input_anchors"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        completed[1]["payload"]["closed_input_anchors"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let project = Project::discover(root.path()).unwrap();
    let state = reduced_session(&project, &session_id(root.path()));
    assert!(
        !serde_json::to_string(state.history().items())
            .unwrap()
            .contains("Private fixture reasoning.")
    );
    let resumed = run_chat(
        root.path(),
        &config,
        "/context\n/exit\n",
        &["--resume", "latest"],
    );
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
}

#[test]
fn oversized_stopped_turn_compacts_then_restarts_with_exact_protected_input_once() {
    let root = TemporaryDirectory::new("context2-stopped-protected-input");
    initialize_project(root.path());
    let definition = writer_definition(root.path());
    let original_output = "NEUTRAL-SOURCE-".repeat(10_000);
    let server = spawn_scripted_responses_server(vec![
        InitialSelection,
        Response(200, text_response(&original_output, "first")),
        ContinueSelection { usage: None },
        Response(
            200,
            compaction_summary_response("First bounded source section.", "leaf-1"),
        ),
        Response(
            200,
            compaction_summary_response("Second bounded source section.", "leaf-2"),
        ),
        Response(
            200,
            compaction_summary_response("Merged durable source sections.", "root"),
        ),
        SelectionDecision(continue_from_selected_summary),
        SelectionDecision(continue_from_selected_summary),
        Response(200, text_response("Resumed work completed.", "last")),
    ]);
    let config = write_config(root.path(), &server.base_url);
    let initial = run_chat(
        root.path(),
        &config,
        "First task.\nEXACT-UNFINISHED-RESTRICTION\n/resume\n/context\n/compact\n/exit\n",
        &["--agent", definition.to_str().unwrap()],
    );
    assert!(
        initial.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&initial.stdout)
            .chars()
            .rev()
            .take(3000)
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>(),
        String::from_utf8_lossy(&initial.stderr)
    );
    let stdout = String::from_utf8_lossy(&initial.stdout);
    let current = stdout
        .split("[current refreshed candidate]")
        .nth(1)
        .expect("current failed candidate diagnostic");
    let number = |label: &str| -> u64 {
        current
            .split(label)
            .nth(1)
            .expect(label)
            .chars()
            .take_while(|ch| ch.is_ascii_digit())
            .collect::<String>()
            .parse()
            .unwrap()
    };
    let input_budget = number("full input budget B=");
    let tools = number("tool schema=");
    let fragment_budget = number("fragment budget after tools=");
    assert_eq!(fragment_budget, input_budget - tools);
    for label in [
        "instructions=",
        "Memory=",
        "Summary=",
        "current/protected input=",
        "History/results=",
        "Turn updates/results=",
        "Research/project facts=",
        "required fragments=",
        "image items=",
        "opaque references=",
    ] {
        assert!(current.contains(label), "missing {label}");
    }
    assert!(current.contains("overlaps the logical candidate; do not add them"));
    assert!(current.contains("not Provider token counts"));
    assert!(current.contains("[original stopped candidate] breakdown unavailable"));
    let resumed = run_chat(
        root.path(),
        &config,
        "Continue with new material.\n/exit\n",
        &["--resume", "latest"],
    );
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    let wires = (0..9)
        .map(|_| request_body(&server.requests.recv_timeout(F2A_WAIT).unwrap()))
        .collect::<Vec<_>>();
    server.join();
    assert_eq!(
        wires[8]
            .to_string()
            .matches("EXACT-UNFINISHED-RESTRICTION")
            .count(),
        1
    );
    selector_candidates(&wires[0]);
    assert_eq!(wires[1]["tools"], wires[8]["tools"]);
    let project = Project::discover(root.path()).unwrap();
    let state = reduced_session(&project, &session_id(root.path()));
    let history = serde_json::to_string(state.history().items()).unwrap();
    assert_eq!(history.matches("EXACT-UNFINISHED-RESTRICTION").count(), 1);
    assert!(history.contains(&original_output));
    assert_eq!(state.execution_receipt_count(), 0);
    assert_eq!(state.request_audits().len(), 9);
}

#[test]
fn stale_research_refresh_preserves_task_model_source_without_sampling() {
    let root = TemporaryDirectory::new("context2-stale-v6");
    initialize_project(root.path());
    let project = Project::discover(root.path()).unwrap();
    append_research_snapshot(&project, "original");
    let server = spawn_scripted_responses_server(vec![
        InitialSelection,
        Response(
            200,
            text_response(&"Prior research observation. ".repeat(1000), "before"),
        ),
        Response(
            200,
            compaction_summary_response("Observed source with uncertainty.", "summary"),
        ),
    ]);
    let config = write_config(root.path(), &server.base_url);
    let first = run_chat(
        root.path(),
        &config,
        "Assess the target.\n/compact\n/exit\n",
        &[],
    );
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(String::from_utf8_lossy(&first.stdout).contains("Compaction revision 1 applied"));
    for _ in 0..3 {
        server.requests.recv_timeout(F2A_WAIT).unwrap();
    }
    server.join();
    let session = session_id(root.path());
    let before = session_events(root.path(), &session);
    let summary = before
        .iter()
        .find(|event| event["event_type"] == "session.compaction_created")
        .unwrap()["payload"]["summary"]
        .clone();
    assert_eq!(summary["schema_version"], 7);
    append_research_snapshot(&project, "newer");
    let resumed = run_chat(root.path(), &config, "/exit\n", &["--resume", "latest"]);
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    let after = session_events(root.path(), &session);
    let summaries = after
        .iter()
        .filter(|event| event["event_type"] == "session.compaction_created")
        .map(|event| &event["payload"]["summary"])
        .collect::<Vec<_>>();
    assert_eq!(summaries.len(), 1);
    // V7 remains bound to the selected task's exact sources. An unrelated
    // append cannot relabel its old observations as the new current Target.
    assert_eq!(*summaries[0], summary);
    assert_eq!(
        after
            .iter()
            .filter(|event| event["event_type"] == "session.request_audit_recorded")
            .count(),
        3
    );
}

#[test]
fn stale_task_handoff_preserves_recorded_limit_after_window_shrinks() {
    let root = TemporaryDirectory::new("context2-stale-v6-narrow-window");
    initialize_project(root.path());
    let project = Project::discover(root.path()).unwrap();
    append_research_snapshot(&project, "before");
    let progress = "Original retained observation. ".repeat(160);
    let server = spawn_scripted_responses_server(vec![
        InitialSelection,
        Response(
            200,
            text_response(&"Prior observation. ".repeat(1500), "before"),
        ),
        Response(200, compaction_summary_response(&progress, "summary")),
        SelectionDecision(continue_from_selected_summary),
        SelectionDecision(continue_from_selected_summary),
        Response(
            400,
            r#"{"error":{"code":"context_length_exceeded","message":"too long"}}"#.to_owned(),
        ),
    ]);
    let base_url = server.base_url.clone();
    let large = write_context_window_config(root.path(), &base_url, 131_072);
    let initial = run_chat(root.path(), &large, "Initial task.\n/compact\n/exit\n", &[]);
    assert!(
        initial.status.success(),
        "{}",
        String::from_utf8_lossy(&initial.stderr)
    );
    let id = session_id(root.path());
    let stopped = run_chat(
        root.path(),
        &large,
        &format!("{}\n/exit\n", "PROTECTED-OVERSIZED-".repeat(6000)),
        &["--resume", &id],
    );
    assert!(!stopped.status.success());
    for _ in 0..6 {
        server.requests.recv_timeout(F2A_WAIT).unwrap();
    }
    assert!(server.requests.try_recv().is_err());
    server.join();
    let before_events = session_events(root.path(), &id);
    let old = before_events
        .iter()
        .find(|event| event["event_type"] == "session.compaction_created")
        .unwrap()["payload"]["summary"]
        .clone();
    assert_eq!(old["schema_version"], 7);
    assert!(old["byte_limit"].as_u64().unwrap() > 1966);
    assert!(old["narrative"].to_string().len() > 1966);
    let before = reduced_session(&project, &id);
    append_research_snapshot(&project, "after");
    let small = write_context_window_config(root.path(), &base_url, 32_768);
    let resumed = run_chat(
        root.path(),
        &small,
        "/context\n/compact\nFORBIDDEN-NEW-TASK\n/exit\n",
        &["--resume", &id],
    );
    let stdout = String::from_utf8_lossy(&resumed.stdout);
    assert!(!resumed.status.success());
    assert!(
        stdout.contains("[current refreshed candidate]"),
        "{stdout}; stderr={}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    assert!(
        stdout.contains("Compaction could not find a smaller admissible Context"),
        "{stdout}"
    );
    assert!(
        stdout.contains("Task input not accepted or saved"),
        "{stdout}"
    );
    let after = reduced_session(&project, &id);
    assert_eq!(after.history(), before.history());
    assert_eq!(after.request_audits(), before.request_audits());
    assert_eq!(
        after.execution_receipt_count(),
        before.execution_receipt_count()
    );
    let after_events = session_events(root.path(), &id);
    let refreshed = &after_events
        .iter()
        .rfind(|event| event["event_type"] == "session.compaction_created")
        .unwrap()["payload"]["summary"];
    assert_eq!(refreshed["revision"], 1);
    for field in [
        "byte_limit",
        "narrative",
        "narrative_source",
        "model_sample_event_ref",
        "source_plan_blob_id",
        "source_plan_hash",
        "protected_inputs",
    ] {
        assert_eq!(refreshed[field], old[field], "{field}");
    }
}

#[test]
fn stale_task_summary_with_over_budget_protected_input_restarts_into_recovery_commands() {
    let root = TemporaryDirectory::new("context2-stale-oversized");
    initialize_project(root.path());
    let project = Project::discover(root.path()).unwrap();
    append_research_snapshot(&project, "before");
    let server = spawn_scripted_responses_server(vec![
        InitialSelection,
        Response(
            200,
            text_response(&"Prior observation. ".repeat(500), "before"),
        ),
        Response(
            200,
            compaction_summary_response("Preserved handoff.", "summary"),
        ),
    ]);
    let config = write_config(root.path(), &server.base_url);
    let initial = run_chat(
        root.path(),
        &config,
        "Initial task.\n/compact\n/exit\n",
        &[],
    );
    assert!(
        initial.status.success(),
        "{}",
        String::from_utf8_lossy(&initial.stderr)
    );
    assert!(
        String::from_utf8_lossy(&initial.stdout).contains("Compaction revision 1 applied"),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&initial.stdout),
        String::from_utf8_lossy(&initial.stderr)
    );
    for _ in 0..3 {
        server.requests.recv_timeout(F2A_WAIT).unwrap();
    }
    assert!(server.requests.try_recv().is_err());
    server.join();
    let id = session_id(root.path());
    let task = "PROTECTED-OVERSIZED-".repeat(6000);
    let stopped = run_chat(
        root.path(),
        &config,
        &format!("{task}\n/exit\n"),
        &["--resume", &id],
    );
    assert!(!stopped.status.success());
    let before = reduced_session(&project, &id);
    assert_eq!(
        before
            .latest_compaction()
            .unwrap()
            .summary()
            .schema_version(),
        7
    );
    append_research_snapshot(&project, "after");
    let resumed = run_chat(
        root.path(),
        &config,
        "/context\n/compact\nFORBIDDEN-NEW-TASK\n/exit\n",
        &["--resume", &id],
    );
    let stdout = String::from_utf8_lossy(&resumed.stdout);
    assert!(!resumed.status.success());
    assert!(stdout.contains("[current refreshed candidate]"), "{stdout}");
    assert!(
        stdout.contains("Compaction could not find a smaller admissible Context"),
        "{stdout}"
    );
    assert!(
        stdout.contains("Task input not accepted or saved"),
        "{stdout}"
    );
    let after = reduced_session(&project, &id);
    assert_eq!(after.request_audits().len(), before.request_audits().len());
    assert_eq!(
        after.execution_receipt_count(),
        before.execution_receipt_count()
    );
    assert_eq!(after.history(), before.history());
    assert_eq!(after.latest_compaction(), before.latest_compaction());
}

#[test]
fn stale_task_summary_with_over_budget_protected_input_resumes_in_same_process() {
    let root = TemporaryDirectory::new("context2-stale-oversized-live-process");
    initialize_project(root.path());
    let project = Project::discover(root.path()).unwrap();
    append_research_snapshot(&project, "before");
    let server = spawn_scripted_responses_server(vec![
        InitialSelection,
        Response(
            200,
            text_response(&"Prior observation. ".repeat(500), "before"),
        ),
        Response(
            200,
            compaction_summary_response("Preserved handoff.", "summary"),
        ),
    ]);
    let config = write_config(root.path(), &server.base_url);
    let initial = run_chat(
        root.path(),
        &config,
        "Initial task.\n/compact\n/exit\n",
        &[],
    );
    assert!(
        initial.status.success(),
        "{}",
        String::from_utf8_lossy(&initial.stderr)
    );
    assert!(
        String::from_utf8_lossy(&initial.stdout).contains("Compaction revision 1 applied"),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&initial.stdout),
        String::from_utf8_lossy(&initial.stderr)
    );
    for _ in 0..3 {
        server.requests.recv_timeout(F2A_WAIT).unwrap();
    }
    server.join();
    let id = session_id(root.path());
    let mut child = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .current_dir(root.path())
        .args([
            "chat",
            "--config",
            config.to_str().unwrap(),
            "--resume",
            &id,
        ])
        .env("CYBER_AGENT_PRIMARY_API_KEY", "loopback-primary-secret")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    output.read_line(&mut line).unwrap();
    assert!(line.contains("Resumed session"), "{line}");
    writeln!(
        child.stdin.as_mut().unwrap(),
        "{}",
        "PROTECTED-OVERSIZED-".repeat(6000)
    )
    .unwrap();
    loop {
        line.clear();
        assert_ne!(
            output.read_line(&mut line).unwrap(),
            0,
            "process exited before recovery"
        );
        if line.contains("Durable checkpoint retained") {
            break;
        }
    }
    let before = reduced_session(&project, &id);
    append_research_snapshot(&project, "after");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"/resume\n/context\n/compact\nFORBIDDEN-NEW-TASK\n/exit\n")
        .unwrap();
    let mut rest = String::new();
    output.read_to_string(&mut rest).unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(!result.status.success());
    assert!(
        rest.contains("ProviderSession rebuilt after source validation"),
        "{rest}"
    );
    assert!(rest.contains("[current refreshed candidate]"), "{rest}");
    assert!(
        rest.contains("Compaction could not find a smaller admissible Context"),
        "{rest}"
    );
    assert!(rest.contains("Task input not accepted or saved"), "{rest}");
    let after = reduced_session(&project, &id);
    assert_eq!(after.request_audits().len(), before.request_audits().len());
    assert_eq!(
        after.execution_receipt_count(),
        before.execution_receipt_count()
    );
    assert_eq!(after.history(), before.history());
    assert_eq!(after.latest_compaction(), before.latest_compaction());
}
