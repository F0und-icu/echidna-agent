//! Real CLI, loopback Provider and durable multi-node handoff regression.

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
    summary["session_source"]["stream_hash"] = prefix_hash.into();
    summary["fingerprint"] = "".into();
    summary["fingerprint"] =
        canonical_fixture_hash(&("echidna-research-main-v5", &*summary)).into();
    forged
}

fn compaction_summary_response(progress: &str, id: &str) -> String {
    text_response(
        &json!({"concise_progress": progress, "unresolved_questions": [], "next_step_notes": []})
            .to_string(),
        id,
    )
}

#[test]
fn research_suffix_refreshes_references_after_verifying_original_graph() {
    let root = TemporaryDirectory::new("main-v5-research-refresh");
    initialize_project(root.path());
    let project = Project::discover(root.path()).unwrap();
    let old_target = append_research_snapshot(&project, "handoff-old");
    let long_answer = "Earlier durable observation. ".repeat(250);
    let server = spawn_responses_server(vec![
        (200, text_response(&long_answer, "first")),
        (
            200,
            compaction_summary_response("The previous work remains unresolved.", "handoff"),
        ),
    ]);
    let config = write_config(root.path(), &server.base_url);
    let output = run_chat(
        root.path(),
        &config,
        "Investigate this target.\n/compact\n/exit\n",
        &[],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Compaction revision 1 applied"));
    server.join();
    let session = session_id(root.path());
    let before = session_events(root.path(), &session);
    let original = before
        .iter()
        .find(|event| event["event_type"] == "session.compaction_created")
        .unwrap()["payload"]["summary"]
        .clone();
    assert_eq!(
        original["evidence_spine"]["target_snapshot_refs"][0],
        old_target.as_str()
    );
    let new_target = append_research_snapshot(&project, "handoff-current");
    let resumed = run_chat(root.path(), &config, "/exit\n", &["--resume", "latest"]);
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    let after = session_events(root.path(), &session);
    let refreshed = &after
        .iter()
        .rfind(|event| event["event_type"] == "session.compaction_created")
        .unwrap()["payload"]["summary"];
    assert_eq!(refreshed["revision"], 2);
    assert_eq!(
        refreshed["evidence_spine"]["target_snapshot_refs"][0],
        new_target.as_str()
    );
    assert_eq!(refreshed["narrative"], original["narrative"]);
    assert_eq!(refreshed["narrative_source"], original["narrative_source"]);
    assert_eq!(
        refreshed["model_sample_event_ref"],
        original["model_sample_event_ref"]
    );
    let research_path = root.path().join(".echidna/research.jsonl");
    let original_research = fs::read_to_string(&research_path).unwrap();
    let mut research_events = original_research
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    research_events[0]["payload"]["data"]["notes"] = "altered historical source".into();
    fs::write(
        &research_path,
        research_events
            .iter()
            .map(|event| format!("{}\n", serde_json::to_string(event).unwrap()))
            .collect::<String>(),
    )
    .unwrap();
    let rejected = run_chat(root.path(), &config, "/exit\n", &["--resume", "latest"]);
    assert!(!rejected.status.success());
    assert!(
        String::from_utf8_lossy(&rejected.stderr)
            .contains("historical Research source identity mismatch")
    );
    assert_eq!(session_events(root.path(), &session), after);
    fs::write(&research_path, original_research).unwrap();
}

#[test]
fn long_completed_source_uses_bounded_nodes_and_one_verified_root() {
    let root = TemporaryDirectory::new("context2-segmented-handoff");
    initialize_project(root.path());
    let original = "NEUTRAL-SOURCE-".repeat(10_000);
    let server = spawn_responses_server(vec![
        (200, text_response(&original, "first")),
        (
            200,
            compaction_summary_response("First bounded source section.", "leaf-1"),
        ),
        (
            200,
            compaction_summary_response("Second bounded source section.", "leaf-2"),
        ),
        (
            200,
            compaction_summary_response("Merged durable source sections.", "root"),
        ),
        (200, text_response("Completed follow-up.", "last")),
    ]);
    let config = write_config(root.path(), &server.base_url);
    // Keep the declared 272K window and explicitly reserve more output space
    // in this graph-boundary fixture, so the source requires two fitting leaves.
    let graph_budget = format!(
        "{}\n[runtime.context_budget]\noutput_reserve_bps = 9000\ntool_loop_reserve_bps = 100\n",
        fs::read_to_string(&config).unwrap()
    );
    fs::write(&config, graph_budget).unwrap();
    let output = run_chat(
        root.path(),
        &config,
        "Write the document.\n/compact\nContinue.\n/exit\n",
        &[],
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
    let wires = (0..5)
        .map(|_| request_body(&server.requests.recv_timeout(F2A_WAIT).unwrap()))
        .collect::<Vec<_>>();
    server.join();
    for wire in &wires[1..4] {
        assert!(
            wire.get("tools")
                .and_then(|value| value.as_array())
                .is_none_or(Vec::is_empty)
        );
        assert!(wire.get("previous_response_id").is_none());
    }
    assert_eq!(wires[0]["tools"], wires[4]["tools"]);
    let project = Project::discover(root.path()).unwrap();
    let state = reduced_session(&project, &session_id(root.path()));
    let compaction = state
        .latest_compaction()
        .expect("verified root was adopted");
    assert_eq!(compaction.summary().schema_version(), 5);
    assert!(
        serde_json::to_string(state.history().items())
            .unwrap()
            .contains(&original)
    );
    assert_eq!(state.request_audits().len(), 5);
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
    fs::write(&event_path, &original_events).unwrap();
    // Keep a structurally valid stream and Summary fingerprint while removing
    // one leaf sample. The merge must not use the missing accepted child.
    let mut partial = before.clone();
    let leaf = partial
        .iter()
        .position(|event| {
            event["event_type"] == "session.compaction_sample_recorded"
                && event["payload"]["graph"]["node"]["is_root"] == false
        })
        .unwrap();
    partial.remove(leaf);
    for (index, event) in partial.iter_mut().enumerate() {
        event["sequence"] = (index as u64 + 1).into();
    }
    let created = partial
        .iter()
        .position(|event| event["event_type"] == "session.compaction_created")
        .unwrap();
    let source_hash = canonical_fixture_hash(&partial[..created]);
    let source_sequence = partial[created - 1]["sequence"].clone();
    partial[created]["payload"]["source_event_sequence"] = source_sequence.clone();
    let summary = &mut partial[created]["payload"]["summary"];
    summary["session_source"]["watermark"] = source_sequence;
    summary["session_source"]["stream_hash"] = source_hash.into();
    summary["fingerprint"] = "".into();
    summary["fingerprint"] =
        canonical_fixture_hash(&("echidna-research-main-v5", &*summary)).into();
    let decoded = partial
        .iter()
        .cloned()
        .map(|event| serde_json::from_value::<EventEnvelope<Value>>(event).unwrap())
        .collect::<Vec<_>>();
    SessionReducer
        .reduce(decoded)
        .expect("partial graph still has a valid Session event projection");
    fs::write(
        &event_path,
        partial
            .iter()
            .map(|event| format!("{}\n", serde_json::to_string(event).unwrap()))
            .collect::<String>(),
    )
    .unwrap();
    let rejected_partial = run_chat(root.path(), &config, "/exit\n", &["--resume", "latest"]);
    assert!(!rejected_partial.status.success());
    assert!(
        String::from_utf8_lossy(&rejected_partial.stderr)
            .contains("Compaction child sample is absent"),
        "{}",
        String::from_utf8_lossy(&rejected_partial.stderr)
    );
    assert_eq!(session_events(root.path(), &session), partial);
    fs::write(&event_path, &original_events).unwrap();
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
