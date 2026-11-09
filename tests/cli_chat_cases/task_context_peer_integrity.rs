//! Synthetic pre-definition peers and revision-bound source expansion.
use super::task_context::definition;
use super::task_context_paging::{delivered_result, paged_server};
use super::*;
use echidna::domain::context::{ProviderSessionId, ProviderSessionRef, ProviderSessionRole};
use echidna::domain::events::EventSequence;
use echidna::domain::model::ContentItem;

fn read_call(call: &str, reference: &str) -> String {
    m4_long_flow_tool_response(
        call,
        "context_read",
        json!({"operation":"read", "reference":reference, "max_bytes":1024}),
        &format!("response-{call}"),
    )
}

fn search_call(call: &str) -> String {
    m4_long_flow_tool_response(
        call,
        "context_read",
        json!({"operation":"search", "query":"LEGACYPEER594", "max_bytes":1024}),
        &format!("response-{call}"),
    )
}

#[test]
fn legacy_peer_read_is_scoped_and_search_references_bind_source_revision() {
    const PEER: &str = "synthetic-legacy-peer";
    const EVENT: &str = "synthetic-legacy-history";
    const ORIGINAL: &str = "LEGACYPEER594 original synthetic observation";
    const CHANGED: &str = "LEGACYPEER594 modified synthetic observation";
    let root = TemporaryDirectory::new("task-legacy-peer-revision");
    initialize_project(root.path());
    let project = Project::discover(root.path()).unwrap();
    let peer = StreamId::new(PEER);
    let stream = ProjectEventStream::Session(peer.clone()).stream_id();
    {
        // Actual pre-definition schema; do not remove a modern binding event.
        let mut writer = project.open_writer().unwrap();
        let mut store = writer.open_session(peer.clone(), None).unwrap();
        let events = [
            SessionEvent::Started {
                primary_provider_session: ProviderSessionRef {
                    id: ProviderSessionId::new("synthetic-legacy-primary"),
                    role: ProviderSessionRole::Primary,
                    generation: 1,
                },
            },
            SessionEvent::HistoryItemRecorded {
                item: ContentItem::assistant_text(ORIGINAL),
            },
        ];
        for (index, event) in events.into_iter().enumerate() {
            let id = if index == 0 {
                "synthetic-legacy-start"
            } else {
                EVENT
            };
            let envelope = event
                .into_envelope(
                    EventId::new(id),
                    stream.clone(),
                    store.next_sequence().unwrap(),
                    OccurredAt::new("2026-11-05T00:00:00Z"),
                )
                .unwrap();
            EventSink::append_event(&mut store, &envelope).unwrap();
        }
    }
    let state = reduced_session(&project, PEER);
    assert!(state.agent_definition_binding().is_none());
    assert_eq!(
        state.history().items(),
        &[ContentItem::assistant_text(ORIGINAL)]
    );
    let path = root
        .path()
        .join(".echidna/sessions")
        .join(PEER)
        .join("events.jsonl");
    let original_bytes = fs::read(&path).unwrap();
    let append = SessionEvent::HistoryItemRecorded {
        item: ContentItem::assistant_text("Independent synthetic append"),
    }
    .into_envelope(
        EventId::new("synthetic-legacy-append"),
        stream,
        EventSequence::new(3),
        OccurredAt::new("2026-11-05T00:00:01Z"),
    )
    .unwrap();
    let path_for_server = path.clone();
    let bytes_for_server = original_bytes.clone();
    let bare = format!("session:{PEER}:{EVENT}");
    let bare_for_server = bare.clone();
    let mut reference = String::new();
    let mut revision = String::new();
    let server = paged_server(move |step, wire| {
        if step == 0 {
            return (
                super::task_context::new_selection_response(wire, "legacy-peer-initial-selection"),
                false,
            );
        }
        match step - 1 {
            0 => (search_call("peer-search"), false),
            1 => {
                let result = delivered_result(wire, "peer-search");
                assert_eq!(result["status"], "succeeded", "{result}");
                assert_eq!(result["output"]["status"], "found");
                let entries = result["output"]["entries"].as_array().unwrap();
                assert_eq!(entries.len(), 1, "{entries:?}");
                let entry = &entries[0];
                assert_eq!(entry["text"], ORIGINAL);
                revision = entry["source_revision"].as_str().unwrap().to_owned();
                reference = entry["reference"].as_str().unwrap().to_owned();
                assert_eq!(
                    reference,
                    format!("session-revision:{PEER}:{EVENT}:{revision}")
                );
                assert_eq!(fs::read(&path_for_server).unwrap(), bytes_for_server);
                (read_call("peer-unchanged", &reference), false)
            }
            2 => {
                let result = delivered_result(wire, "peer-unchanged");
                assert_eq!(result["output"]["status"], "found", "{result}");
                assert_eq!(result["output"]["entries"][0]["text"], ORIGINAL);
                assert_eq!(result["output"]["entries"][0]["source_revision"], revision);
                assert_eq!(fs::read(&path_for_server).unwrap(), bytes_for_server);
                let mut file = fs::OpenOptions::new()
                    .append(true)
                    .open(&path_for_server)
                    .unwrap();
                writeln!(file, "{}", serde_json::to_string(&append).unwrap()).unwrap();
                (read_call("peer-after-append", &reference), false)
            }
            3 => {
                let result = delivered_result(wire, "peer-after-append");
                assert_eq!(result["output"]["status"], "found", "{result}");
                assert_eq!(result["output"]["entries"][0]["text"], ORIGINAL);
                assert_eq!(result["output"]["entries"][0]["source_revision"], revision);
                let current = fs::read_to_string(&path_for_server).unwrap();
                assert_eq!(current.matches(ORIGINAL).count(), 1);
                fs::write(&path_for_server, current.replacen(ORIGINAL, CHANGED, 1)).unwrap();
                (read_call("peer-changed", &reference), false)
            }
            4 => {
                let result = delivered_result(wire, "peer-changed");
                assert_eq!(result["status"], "succeeded", "{result}");
                assert_eq!(result["output"]["status"], "stale", "{result}");
                assert!(result["output"]["entries"].as_array().unwrap().is_empty());
                (read_call("peer-bare-compat", &bare_for_server), false)
            }
            5 => {
                let result = delivered_result(wire, "peer-bare-compat");
                assert_eq!(result["output"]["status"], "found", "{result}");
                assert_eq!(result["output"]["entries"][0]["text"], CHANGED);
                assert_ne!(result["output"]["entries"][0]["source_revision"], revision);
                (text_response("PEER-REVISION-VERIFIED", "peer-final"), true)
            }
            _ => panic!("unexpected peer fixture request"),
        }
    });
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "Read the synthetic peer.\n/exit\n",
        &[],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.join();
    assert!(String::from_utf8_lossy(&output.stdout).contains("PEER-REVISION-VERIFIED"));

    let agent = definition(root.path(), None);
    let bytes_before_foreign = fs::read(&path).unwrap();
    let changed_revision =
        blake3::hash(&serde_json::to_vec(&ContentItem::assistant_text(CHANGED)).unwrap()).to_hex();
    let foreign_versioned = format!("session-revision:{PEER}:{EVENT}:{changed_revision}");
    let server = paged_server(move |step, wire| {
        if step == 0 {
            return (
                super::task_context::new_selection_response(wire, "foreign-peer-initial-selection"),
                false,
            );
        }
        match step - 1 {
            0 => (search_call("foreign-search"), false),
            1 => {
                let result = delivered_result(wire, "foreign-search");
                assert_eq!(result["output"]["status"], "not_found", "{result}");
                (read_call("foreign-read", &bare), false)
            }
            2 => {
                let result = delivered_result(wire, "foreign-read");
                assert_eq!(result["output"]["status"], "not_found", "{result}");
                (read_call("foreign-versioned", &foreign_versioned), false)
            }
            3 => {
                let result = delivered_result(wire, "foreign-versioned");
                assert_eq!(result["output"]["status"], "not_found", "{result}");
                (
                    text_response("FOREIGN-PEER-EXCLUDED", "foreign-final"),
                    true,
                )
            }
            _ => panic!("unexpected foreign fixture request"),
        }
    });
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "Search the synthetic peer.\n/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.join();
    assert!(String::from_utf8_lossy(&output.stdout).contains("FOREIGN-PEER-EXCLUDED"));
    assert_eq!(fs::read(path).unwrap(), bytes_before_foreign);
}
