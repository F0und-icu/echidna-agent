use crate::adapters::project_event_store::{
    Project, ProjectEventStream, read_existing_session_events,
};
use crate::adapters::research_projection::{
    load_compaction_validation_facts, load_compaction_validation_facts_with_references,
    load_reference_catalog, load_reference_catalog_with_session,
};
use crate::domain::context::{
    ContextWindowPolicy, ProviderSessionId, ProviderSessionRef, ProviderSessionRole,
};
use crate::domain::events::{
    EventEnvelope, EventId, EventSequence, EventType, OccurredAt, StreamId,
};
use crate::domain::model::ContentItem;
use crate::domain::research::{ResearchEvent, TargetSnapshotDraft, TargetSnapshotRecord};
use crate::domain::session::{SessionEvent, SessionReducer};
use crate::domain::tools::{ExecutionId, ExecutionReceipt, ToolResultStatus};
use crate::ports::EventSink;
use crate::support::TemporaryDirectory;
use serde_json::json;

fn started(id: &str) -> SessionEvent {
    SessionEvent::Started {
        primary_provider_session: ProviderSessionRef {
            id: ProviderSessionId::new(format!("provider-{id}")),
            role: ProviderSessionRole::Primary,
            generation: 1,
        },
    }
}

fn fixture(label: &str) -> (TemporaryDirectory, Project) {
    let root = TemporaryDirectory::new(label);
    let project = Project::initialize(root.path()).unwrap();
    let mut writer = project.open_writer().unwrap();
    for id in ["current", "peer"] {
        let session_id = StreamId::new(id);
        let stream = ProjectEventStream::Session(session_id.clone()).stream_id();
        let mut store = writer.open_session(session_id, None).unwrap();
        let receipt = ExecutionReceipt {
            execution_id: ExecutionId::new(format!("execution-{id}")),
            plan_fingerprint: format!("plan-{id}"),
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
        for (kind, event) in [
            ("started", started(id)),
            (
                "receipt",
                SessionEvent::ExecutionReceiptRecorded { receipt },
            ),
        ] {
            let event = event
                .into_envelope(
                    EventId::new(format!("{kind}-{id}")),
                    stream.clone(),
                    store.next_sequence().unwrap(),
                    OccurredAt::new("2026-10-26T00:00:00Z"),
                )
                .unwrap();
            store.append_event(&event).unwrap();
        }
    }
    let target: TargetSnapshotDraft = serde_json::from_value(json!({
        "id": "target", "target_id": "compaction-fixture", "kind": "source",
        "locator": "fixture:compaction", "version": "1", "build_id": "fixture",
        "environment_fingerprint": "fixture-env", "device_fingerprint": "fixture-device",
        "captured_at": "2026-10-26T00:00:00Z"
    }))
    .unwrap();
    let event =
        ResearchEvent::TargetSnapshotRecorded(TargetSnapshotRecord::from_draft(target).unwrap());
    let mut store = writer.open_research(None).unwrap();
    store
        .append_event(&EventEnvelope::current(
            EventId::new("target-event"),
            StreamId::new("research"),
            EventSequence::new(1),
            OccurredAt::new("2026-10-26T00:00:00Z"),
            EventType::new(event.event_type().as_str()),
            event.encode_payload().unwrap(),
        ))
        .unwrap();
    drop(store);
    drop(writer);
    (root, project)
}

#[test]
fn compaction_reuses_exact_current_receipts_and_produces_the_same_research_draft() {
    let (_root, project) = fixture("compaction-reference-reuse");
    let _writer = project.open_writer().unwrap();
    let current = StreamId::new("current");
    let events = read_existing_session_events(&project, &current).unwrap();
    let state = SessionReducer.reduce(events).unwrap();
    let baseline = load_reference_catalog(&project).unwrap();
    let reused = load_reference_catalog_with_session(&project, &current, &state).unwrap();
    assert_eq!(
        reused.execution_receipts().collect::<Vec<_>>(),
        baseline.execution_receipts().collect::<Vec<_>>()
    );
    assert_eq!(reused.execution_receipts().count(), 2);
    let baseline = load_compaction_validation_facts(&project).unwrap();
    let reused = load_compaction_validation_facts_with_references(&project, reused).unwrap();
    let history = [ContentItem::user_text("continue the fixture")];
    let policy = ContextWindowPolicy::for_window(32768);
    assert_eq!(
        reused
            .compose_session_summary_draft(&history, None, 32, policy)
            .unwrap(),
        baseline
            .compose_session_summary_draft(&history, None, 32, policy)
            .unwrap()
    );
}

#[test]
fn compaction_current_session_reuse_still_rejects_invalid_peer_transitions() {
    let (_root, project) = fixture("compaction-peer-integrity");
    let mut writer = project.open_writer().unwrap();
    let current = StreamId::new("current");
    let events = read_existing_session_events(&project, &current).unwrap();
    let state = SessionReducer.reduce(events).unwrap();
    let peer = StreamId::new("peer");
    let mut store = writer.open_session(peer.clone(), None).unwrap();
    let invalid = started("restarted-peer")
        .into_envelope(
            EventId::new("invalid-peer-start"),
            ProjectEventStream::Session(peer).stream_id(),
            store.next_sequence().unwrap(),
            OccurredAt::new("2026-10-26T00:00:00Z"),
        )
        .unwrap();
    store.append_event(&invalid).unwrap();
    drop(store);
    let baseline = load_reference_catalog(&project).unwrap_err();
    let reused = load_reference_catalog_with_session(&project, &current, &state).unwrap_err();
    assert_eq!(reused, baseline);
    assert!(
        reused.contains("Session `peer` cannot be reduced"),
        "{reused}"
    );
}
