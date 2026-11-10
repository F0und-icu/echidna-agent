//! Actual audited delivery lineage, persistence and negative replay controls.

use super::task_context::{
    ScriptedResponse::{InitialSelection, Response, TruncatedResponse},
    selector_candidates, spawn_scripted_responses_server,
};
use super::*;
use sha2::{Digest, Sha256};

pub(super) fn canonical_bytes(value: &impl serde::Serialize) -> Vec<u8> {
    serde_json::to_vec(&serde_json::to_value(value).unwrap()).unwrap()
}

pub(super) fn canonical_hash(value: &impl serde::Serialize) -> String {
    blake3::hash(&canonical_bytes(value)).to_hex().to_string()
}

#[test]
fn tool_incremental_retry_uses_only_accepted_baseline_and_requires_real_delivered_input() {
    let root = TemporaryDirectory::new("delivery-lineage-cli");
    initialize_project(root.path());
    fs::write(root.path().join("proof.txt"), "real fixture content").unwrap();
    let first = tool_calls_response(
        &[("inspect", "read_file", json!({"path":"proof.txt"}))],
        "accepted-tool",
    );
    let malformed = format!(
        "data: {}\n\ndata: {}\n\n",
        json!({"type":"response.output_item.done","item":{"type":"function_call","call_id":"invalid-args","name":"read_file","arguments":"{"}}),
        json!({"type":"response.completed","response":{"id":"rejected-sample"}}),
    );
    let partial = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"uncommitted\"}\n\n";
    let final_response = text_response("Verified answer.", "accepted-final");
    let server = spawn_scripted_responses_server(vec![
        InitialSelection,
        Response(200, first),
        Response(200, malformed),
        TruncatedResponse(partial.to_owned()),
        Response(200, final_response),
    ]);
    let config = write_config(root.path(), &server.base_url);
    let output = run_chat(root.path(), &config, "EXACT-INPUT-ANCHOR\n/exit\n", &[]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    selector_candidates(&f2a_request_body(&server.requests, "initial selection"));
    let wires = (0..4)
        .map(|_| request_body(&server.requests.recv_timeout(F2A_WAIT).unwrap()))
        .collect::<Vec<_>>();
    server.join();
    assert!(
        wires[0]
            .get("previous_response_id")
            .is_none_or(Value::is_null)
    );
    assert_eq!(wires[1]["previous_response_id"], "accepted-tool");
    assert_eq!(wires[2]["previous_response_id"], "accepted-tool");
    assert_eq!(wires[3]["previous_response_id"], "accepted-tool");
    let session = session_id(root.path());
    let events = session_events(root.path(), &session);
    let audits = events
        .iter()
        .filter(|event| event["event_type"] == "session.request_audit_recorded")
        .collect::<Vec<_>>();
    let deliveries = events
        .iter()
        .filter(|event| event["event_type"] == "session.model_delivery_committed")
        .collect::<Vec<_>>();
    assert_eq!(audits.len(), 5);
    let (selection_audit, audits) = audits.split_first().unwrap();
    assert!(events.iter().any(|event| {
        event["event_type"] == "session.task_selection_sample_recorded"
            && event["payload"]["audit_event_id"] == selection_audit["event_id"]
    }));
    assert_eq!(deliveries.len(), 2);
    assert_eq!(audits[0]["payload"]["delivery"]["mode"], "full");
    assert_eq!(audits[3]["payload"]["delivery"]["mode"], "incremental");
    assert_eq!(
        deliveries[0]["payload"]["audit_event_id"],
        audits[0]["event_id"]
    );
    assert_eq!(
        deliveries[1]["payload"]["audit_event_id"],
        audits[3]["event_id"]
    );
    assert_eq!(
        deliveries[1]["payload"]["previous_accepted_audit"],
        audits[0]["event_id"]
    );
    assert_ne!(
        deliveries[1]["payload"]["previous_accepted_audit"],
        audits[1]["event_id"]
    );
    assert_ne!(
        deliveries[1]["payload"]["previous_accepted_audit"],
        audits[2]["event_id"]
    );
    let rejected = events
        .iter()
        .find(|event| event["event_type"] == "model.sample_rejected")
        .unwrap();
    assert_eq!(
        rejected["payload"]["request_audit_event_id"],
        audits[1]["event_id"]
    );
    let complete_index = events
        .iter()
        .position(|event| {
            event["event_type"] == "session.operator_turn_boundary_recorded"
                && event["payload"]["kind"] == "completed"
        })
        .expect("actual Full/tool/Incremental completion closes the task");
    let anchor = events[complete_index]["payload"]["anchor"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        events[complete_index]["payload"]["closed_input_anchors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|id| id == &anchor)
    );
    let final_audit = audits[3];
    assert!(
        final_audit["payload"]["full_context_delivery_map"]["content_items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["provenance"]["references"]["source_event_refs"]
                .as_array()
                .is_some_and(|refs| refs.iter().any(|id| id == &anchor)))
    );

    // Change only the actual Full delivered view to another valid user item.
    // The final full logical map still contains the original input, but no
    // actual delivered component in its committed chain contains that input.
    let mut altered = events.clone();
    let first_index = altered
        .iter()
        .position(|event| event["event_id"] == audits[0]["event_id"])
        .unwrap();
    let payload = &mut altered[first_index]["payload"];
    let mut manifest: echidna::domain::request_audit::RequestAuditManifest =
        serde_json::from_value(payload.clone()).unwrap();
    let blob_directory = root.path().join(".echidna/artifacts/blobs/sha256");
    let mut delivered = manifest
        .reconstruct(|id| fs::read(blob_directory.join(id.as_str())))
        .unwrap();
    let entry_index = payload["context_delivery_map"]["content_items"]
        .as_array()
        .unwrap()
        .iter()
        .position(|entry| {
            let position = entry["final_position"].as_u64().unwrap() as usize;
            delivered.content_items[position]
                == echidna::domain::model::ContentItem::user_text("EXACT-INPUT-ANCHOR")
                && entry["provenance"]["references"]["source_event_refs"]
                    .as_array()
                    .is_some_and(|refs| refs.iter().any(|id| id == &anchor))
        })
        .unwrap();
    let position = payload["context_delivery_map"]["content_items"][entry_index]["final_position"]
        .as_u64()
        .unwrap() as usize;
    let original = delivered.content_items[position].clone();
    let replacement =
        echidna::domain::model::ContentItem::user_text("A DIFFERENT ACTUALLY DELIVERED INPUT");
    delivered.content_items[position] = replacement.clone();
    let bytes = canonical_bytes(&replacement);
    let blob_id = format!("{:x}", Sha256::digest(&bytes));
    fs::write(blob_directory.join(&blob_id), &bytes).unwrap();
    let semantic_hash = canonical_hash(&replacement);
    let component_index = payload["components"]
        .as_array()
        .unwrap()
        .iter()
        .position(|component| {
            component["identity"]["kind"] == "content_item"
                && component["identity"]["index"] == position
        })
        .unwrap();
    let component = &mut payload["components"][component_index];
    component["blob_id"] = blob_id.into();
    component["size_bytes"] = bytes.len().into();
    component["semantic_hash"] = semantic_hash.clone().into();
    let entry = &mut payload["context_delivery_map"]["content_items"][entry_index];
    entry["content_hash"] = semantic_hash.into();
    entry["provenance"]["references"]
        .as_object_mut()
        .unwrap()
        .remove("source_event_refs");
    payload["delivered_input"]["semantic_hash"] = canonical_hash(&delivered).into();
    payload["delivery"]["reason"] = json!({"full":"PROVIDER_REJECTED"});
    manifest = serde_json::from_value(payload.clone()).unwrap();
    manifest.validate().unwrap();
    assert_eq!(
        manifest
            .reconstruct(|id| fs::read(blob_directory.join(id.as_str())))
            .unwrap(),
        delivered
    );
    let final_manifest: echidna::domain::request_audit::RequestAuditManifest =
        serde_json::from_value(final_audit["payload"].clone()).unwrap();
    let final_delivered = final_manifest
        .reconstruct(|id| fs::read(blob_directory.join(id.as_str())))
        .unwrap();
    assert!(!final_delivered.content_items.contains(&original));
    let decode = |items: Vec<Value>| {
        items
            .into_iter()
            .map(|event| serde_json::from_value::<EventEnvelope<Value>>(event).unwrap())
            .collect::<Vec<_>>()
    };
    assert!(
        SessionReducer
            .reduce(decode(altered[..complete_index].to_vec()))
            .is_ok()
    );
    assert!(
        SessionReducer.reduce(decode(altered)).is_err(),
        "a logical anchor without actual chain delivery cannot be closed"
    );

    // Explicit historical audit must reject a missing/corrupt original wire
    // Blob without mutating the Session. Resume retires this Provider chain.
    let before = session_events(root.path(), &session);
    let required_id = audits[0]["payload"]["components"][0]["blob_id"]
        .as_str()
        .unwrap();
    let required_path = blob_directory.join(required_id);
    let original_blob = fs::read(&required_path).unwrap();
    fs::remove_file(&required_path).unwrap();
    let audit = || {
        Command::new(env!("CARGO_BIN_EXE_echidna"))
            .current_dir(root.path())
            .args(["session", "show", &session])
            .output()
            .unwrap()
    };
    let missing = audit();
    assert!(!missing.status.success());
    let missing_error = String::from_utf8_lossy(&missing.stderr);
    assert!(
        missing_error.contains("primary audit read failed"),
        "{missing_error}"
    );
    assert_eq!(session_events(root.path(), &session), before);
    fs::write(&required_path, b"corrupt audit blob").unwrap();
    let corrupt = audit();
    assert!(!corrupt.status.success());
    let corrupt_error = String::from_utf8_lossy(&corrupt.stderr);
    assert!(
        corrupt_error.contains("primary audit read failed"),
        "{corrupt_error}"
    );
    assert_eq!(session_events(root.path(), &session), before);
    fs::write(required_path, original_blob).unwrap();
}
