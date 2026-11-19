//! Reused Session reads retain exact bytes, strict replay and Summary sources.

use std::fs;

use serde_json::{Value, json};

use crate::adapters::agent_definition::{bind_definition, builtin_definition};
use crate::adapters::project_event_store::{Project, ProjectEventStream};
use crate::adapters::research_projection::{
    ProjectReadContext, SessionReceiptCatalogCache, load_reference_catalog,
};
use crate::adapters::session_compaction::{
    load_historical_session_compaction_facts, load_resume_compaction_facts,
    load_session_compaction_facts, load_session_compaction_facts_from_read,
};
use crate::adapters::tools::ToolRegistry;
use crate::app::session::SessionProjection;
use crate::domain::agent_definition::BuiltinAgentTemplate;
use crate::domain::context::{
    ContextWindowPolicy, ProviderSessionId, ProviderSessionRef, ProviderSessionRole,
};
use crate::domain::events::{EventEnvelope, EventId, EventSequence, OccurredAt, StreamId};
use crate::domain::model::{CapabilityMappingVersion, ContentItem, ModelCapabilities};
use crate::domain::request_audit::{
    RequestAdapterVersion, RequestAuditBinding, RequestAuditRunContext,
};
use crate::domain::session::{SessionEvent, SessionReducer, SessionSourceIdentity, SessionSummary};
use crate::domain::tools::{ExecutionId, ExecutionReceipt, ToolResultStatus};
use crate::ports::EventSink;
use crate::ports::tool::ToolExecutor;
use crate::support::TemporaryDirectory;

fn provider(id: &str) -> ProviderSessionRef {
    ProviderSessionRef {
        id: ProviderSessionId::new(format!("provider-{id}")),
        role: ProviderSessionRole::Primary,
        generation: 1,
    }
}

fn push(events: &mut Vec<EventEnvelope<Value>>, id: &str, event: SessionEvent) {
    let sequence = events.len() as u64 + 1;
    events.push(
        event
            .into_envelope(
                EventId::new(format!("{id}-{sequence}")),
                ProjectEventStream::Session(StreamId::new(id)).stream_id(),
                EventSequence::new(sequence),
                OccurredAt::new("2026-10-26T00:00:00Z"),
            )
            .unwrap(),
    );
}

fn persist(project: &Project, id: &str, events: &[EventEnvelope<Value>]) {
    let mut writer = project.open_writer().unwrap();
    let mut store = writer.open_session(StreamId::new(id), None).unwrap();
    for event in events {
        store.append_event(event).unwrap();
    }
}

fn receipt(id: &str) -> SessionEvent {
    SessionEvent::ExecutionReceiptRecorded {
        receipt: ExecutionReceipt {
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
        },
    }
}

#[test]
fn project_research_cache_reuses_exact_prefix_and_rejects_rewritten_history() {
    use crate::domain::artifacts::BlobId;
    use crate::domain::events::EventType;
    use crate::domain::memory::{MemoryEvent, MemoryPath, MemoryRevision, MemoryScope};
    use crate::domain::research::ResearchEvent;
    let root = TemporaryDirectory::new("research-projection-frontier");
    let project = Project::initialize(root.path()).unwrap();
    let mut cache = SessionReceiptCatalogCache::default();
    let empty = ProjectReadContext::load(&project, &mut cache, None).unwrap();
    let warm = ProjectReadContext::load(&project, &mut cache, None).unwrap();
    assert!(std::ptr::eq(empty.research_state(), warm.research_state()));
    assert_eq!(warm.research_state().applied_event_count(), 0);
    drop((empty, warm));
    let memory = ResearchEvent::Memory(MemoryEvent::Changed(MemoryRevision {
        scope: MemoryScope::Project,
        path: MemoryPath::parse("decisions.md").unwrap(),
        old_hash: None,
        new_hash: Some("a".repeat(64)),
        old_blob_id: None,
        new_blob_id: Some(BlobId::new("b".repeat(64)).unwrap()),
        diff_blob_id: Some(BlobId::new("c".repeat(64)).unwrap()),
    }));
    let mut writer = project.open_writer().unwrap();
    let mut store = writer.open_research(None).unwrap();
    let envelope = EventEnvelope::current(
        EventId::new("cache-original"),
        ProjectEventStream::Research.stream_id(),
        store.next_sequence().unwrap(),
        OccurredAt::new("2026-10-26T00:00:00Z"),
        EventType::new(memory.event_type().as_str()),
        memory.encode_payload().unwrap(),
    );
    store.append_event(&envelope).unwrap();
    drop(store);
    drop(writer);
    let appended = ProjectReadContext::load(&project, &mut cache, None).unwrap();
    assert_eq!(appended.research_state().applied_event_count(), 1);
    assert_eq!(
        appended.project_hash,
        crate::domain::session::SessionSourceIdentity::project_hash_for_events(&appended.events)
            .unwrap()
    );
    assert!(
        appended
            .memory_state()
            .unwrap()
            .latest(
                &MemoryScope::Project,
                &MemoryPath::parse("decisions.md").unwrap()
            )
            .is_some()
    );
    let unchanged = ProjectReadContext::load(&project, &mut cache, None).unwrap();
    assert!(std::ptr::eq(
        appended.research_state(),
        unchanged.research_state()
    ));
    let path = project.research_stream_path_existing().unwrap();
    let original = fs::read_to_string(&path).unwrap();
    let tampered = original.replace("cache-original", "cache-modified");
    assert_ne!(tampered, original);
    fs::write(&path, &tampered).unwrap();
    for _ in 0..2 {
        assert!(ProjectReadContext::load(&project, &mut cache, None).is_err());
    }
    fs::write(&path, &original).unwrap();
    ProjectReadContext::load(&project, &mut cache, None).unwrap();
    // A valid JSON suffix can fail semantic reduction after scanning succeeds.
    // That failure must not discard the previously validated prefix either.
    let mut conflicting = envelope.clone();
    conflicting.sequence = EventSequence::new(2);
    conflicting.payload["data"]["data"]["revision"]["path"] = json!("core.md");
    fs::write(
        &path,
        format!(
            "{original}{}\n",
            serde_json::to_string(&conflicting).unwrap()
        ),
    )
    .unwrap();
    assert!(ProjectReadContext::load(&project, &mut cache, None).is_err());
    fs::write(&path, &tampered).unwrap();
    assert!(ProjectReadContext::load(&project, &mut cache, None).is_err());
    fs::write(&path, &original).unwrap();
    let restored = ProjectReadContext::load(&project, &mut cache, None).unwrap();
    assert_eq!(restored.project_hash, appended.project_hash);
    let reformatted = original.replacen("\":", "\": ", 1);
    assert_ne!(reformatted, original);
    assert_eq!(
        serde_json::from_str::<Value>(&original).unwrap(),
        serde_json::from_str::<Value>(&reformatted).unwrap()
    );
    fs::write(&path, reformatted).unwrap();
    let _error = ProjectReadContext::load(&project, &mut cache, None)
        .err()
        .expect("reformatted prefix bytes must be rejected");
}

#[test]
fn committed_session_receipts_are_reused_by_memory_without_losing_new_receipts() {
    let root = TemporaryDirectory::new("committed-receipt-memory-cache");
    let project = Project::initialize(root.path()).unwrap();
    let id = StreamId::new("current");
    let mut events = Vec::new();
    push(
        &mut events,
        "current",
        SessionEvent::Started {
            primary_provider_session: provider("current"),
        },
    );
    persist(&project, "current", &events);
    let mut state = SessionReducer.reduce(events.clone()).unwrap();
    let mut cache = SessionReceiptCatalogCache::default();
    let mut writer = project.open_writer().unwrap();
    let project_before = ProjectReadContext::load(&project, &mut cache, None).unwrap();
    let (mut store, _) = writer.open_session_projection(id.clone(), None).unwrap();
    for next in [
        receipt("first"),
        SessionEvent::HistoryItemRecorded {
            item: ContentItem::user_text("Continue"),
        },
        receipt("second"),
    ] {
        push(&mut events, "current", next);
        let event = events.last().unwrap();
        store.append_event(event).unwrap();
        SessionReducer.apply(&mut state, event.clone()).unwrap();
        cache.remember_validated_session(
            &id,
            &state,
            store.projection_cursor().file_revision().clone(),
        );
        let (reused, digest) = cache.refresh_with_session(&project, None).unwrap();
        let project_after = ProjectReadContext::load(&project, &mut cache, None).unwrap();
        assert!(
            std::ptr::eq(
                project_before.research_state(),
                project_after.research_state()
            ),
            "new receipts must not replay an unchanged Project stream"
        );
        let strict = load_reference_catalog(&project).unwrap();
        assert_eq!(
            reused.execution_receipts().collect::<Vec<_>>(),
            strict.execution_receipts().collect::<Vec<_>>()
        );
        assert_eq!(
            reused.execution_receipts().count(),
            state.execution_receipt_count() as usize
        );
        cache.remember_validated_session(
            &id,
            &state,
            store.projection_cursor().file_revision().clone(),
        );
        let (again, unchanged) = cache.refresh_with_session(&project, None).unwrap();
        assert_eq!(digest, unchanged);
        assert_eq!(
            again.execution_receipts().collect::<Vec<_>>(),
            reused.execution_receipts().collect::<Vec<_>>()
        );
    }
}

#[test]
fn memory_content_cache_observes_external_edits_deletion_and_scope_changes() {
    use crate::adapters::project_memory::{LocalProjectMemory, MemoryReadCache};
    use crate::domain::memory::{MemoryPath, MemoryScope};
    let root = TemporaryDirectory::new("memory-content-reuse");
    let project = Project::initialize(root.path()).unwrap();
    let memory = LocalProjectMemory::with_scope(&project, MemoryScope::Project);
    let path = MemoryPath::parse("core.md").unwrap();
    let mut cache = MemoryReadCache::default();
    for content in ["first", "second", "second"] {
        memory.replace(&path, content).unwrap();
        assert_eq!(
            memory.read_cached(&path, &mut cache).unwrap(),
            memory.read(&path).unwrap()
        );
    }
    fs::remove_file(project.control_directory().join("memory/core.md")).unwrap();
    assert_eq!(memory.read_cached(&path, &mut cache).unwrap().content, None);
    memory.replace(&path, "recreated").unwrap();
    assert_eq!(
        memory
            .read_cached(&path, &mut cache)
            .unwrap()
            .content
            .as_deref(),
        Some("recreated")
    );
    let other_root = TemporaryDirectory::new("memory-content-other");
    let other = Project::initialize(other_root.path()).unwrap();
    let other_memory = LocalProjectMemory::with_scope(&other, MemoryScope::Project);
    assert_eq!(
        other_memory.read_cached(&path, &mut cache).unwrap().content,
        None
    );
}

#[test]
fn locked_current_reuse_and_warm_peer_cache_preserve_strict_byte_validation() {
    for change in [
        "receipt_rewrite",
        "invalid_transition",
        "corrupt_json",
        "append",
        "invalid_append",
    ] {
        let root = TemporaryDirectory::new("session-read-peer");
        let project = Project::initialize(root.path()).unwrap();
        let mut current_events = Vec::new();
        let mut peer_events = Vec::new();
        for (id, events) in [("current", &mut current_events), ("peer", &mut peer_events)] {
            push(
                events,
                id,
                SessionEvent::Started {
                    primary_provider_session: provider(id),
                },
            );
            push(events, id, receipt(id));
            persist(&project, id, events);
        }
        let current_id = StreamId::new("current");
        let current = SessionReducer.reduce(current_events).unwrap();
        let peer_path = project
            .session_stream_path_existing(&StreamId::new("peer"))
            .unwrap();
        let original = fs::read(&peer_path).unwrap();
        let metadata = fs::metadata(&peer_path).unwrap();
        let mut cache = SessionReceiptCatalogCache::default();
        let mut writer = project.open_writer().unwrap();
        let expected = load_reference_catalog(&project).unwrap();
        let (cold, original_digest) = cache
            .refresh_with_session(&project, Some((&current_id, &current)))
            .unwrap();
        assert_eq!(
            cold.execution_receipts().collect::<Vec<_>>(),
            expected.execution_receipts().collect::<Vec<_>>()
        );
        let warm =
            ProjectReadContext::load(&project, &mut cache, Some((&current_id, &current))).unwrap();
        assert_eq!(warm.receipt_digest, original_digest);
        assert_eq!(warm.references.execution_receipts().count(), 2);

        if change.ends_with("append") {
            push(
                &mut peer_events,
                "peer",
                if change == "append" {
                    receipt("peer-extra")
                } else {
                    SessionEvent::Started {
                        primary_provider_session: provider("invalid-peer"),
                    }
                },
            );
            let mut store = writer.open_session(StreamId::new("peer"), None).unwrap();
            store.append_event(peer_events.last().unwrap()).unwrap();
        } else {
            let text = String::from_utf8(original.clone()).unwrap();
            let rewritten = match change {
                "receipt_rewrite" => text.replace("plan-peer", "plan-PEER"),
                "invalid_transition" => text.replace("session.started", "session.resumed"),
                "corrupt_json" => format!("!{}", &text[1..]),
                _ => unreachable!(),
            };
            assert_ne!(rewritten.as_bytes(), original);
            fs::write(&peer_path, rewritten).unwrap();
            fs::OpenOptions::new()
                .write(true)
                .open(&peer_path)
                .unwrap()
                .set_times(fs::FileTimes::new().set_modified(metadata.modified().unwrap()))
                .unwrap();
            let changed = fs::metadata(&peer_path).unwrap();
            assert_eq!(changed.len(), metadata.len());
            assert_eq!(changed.modified().unwrap(), metadata.modified().unwrap());
        }
        let changed_bytes = fs::read(&peer_path).unwrap();
        let strict = load_reference_catalog(&project);
        let reused = ProjectReadContext::load(&project, &mut cache, Some((&current_id, &current)));
        match change {
            "receipt_rewrite" | "append" => {
                let strict = strict.unwrap();
                let reused = reused.unwrap();
                assert_ne!(reused.receipt_digest, original_digest, "{change}");
                assert_eq!(
                    reused.references.execution_receipts().collect::<Vec<_>>(),
                    strict.execution_receipts().collect::<Vec<_>>()
                );
                assert_eq!(
                    reused.references.execution_receipts().count(),
                    if change == "append" { 3 } else { 2 }
                );
            }
            _ => {
                let strict_error = strict.err().unwrap();
                let reused_error = reused.err().unwrap();
                assert!(strict_error.contains("Session `peer`"), "{strict_error}");
                assert!(reused_error.contains("Session `peer`"), "{reused_error}");
            }
        }
        assert_eq!(
            fs::read(&peer_path).unwrap(),
            changed_bytes,
            "read changed peer facts"
        );
        fs::write(&peer_path, original).unwrap();
        let (restored, restored_digest) = cache
            .refresh_with_session(&project, Some((&current_id, &current)))
            .unwrap();
        assert_eq!(restored.execution_receipts().count(), 2);
        assert_eq!(restored_digest, original_digest);
    }
}

async fn append_compaction(
    project: &Project,
    events: &mut Vec<EventEnvelope<Value>>,
) -> SessionSummary {
    let (source, _) =
        SessionSourceIdentity::from_events(project.project_id().clone(), events, &[]).unwrap();
    let capabilities = ModelCapabilities {
        text_input: true,
        image_input: false,
        streaming: true,
        tool_call: true,
        structured_output: false,
        incremental_input: false,
        usage_reporting: false,
        max_output_tokens: None,
        reasoning_effort: None,
        context_window: 32768,
        capability_mapping_version: CapabilityMappingVersion::new(1),
    };
    let context = RequestAuditRunContext::new(
        RequestAuditBinding::new(
            project.project_id().clone(),
            StreamId::new("session:current"),
        )
        .unwrap(),
        RequestAdapterVersion::new("fixture-read-reuse:1").unwrap(),
    );
    let previous_len = events.len();
    let audit_id = EventId::new(format!("current-{}", events.len() + 1));
    let manifest = crate::legacy_v5_fixture::historical_audit(
        &source,
        audit_id.clone(),
        &context,
        capabilities,
    );
    push(
        events,
        "current",
        SessionEvent::RequestAuditRecorded { manifest },
    );
    let narrative = crate::domain::session::NarrativeSummary::new(
        "Inspect ordinary notes",
        "The ordinary note was inspected.",
        vec![],
        vec!["Continue from the preserved observations.".into()],
    )
    .unwrap();
    let sample = crate::legacy_v5_fixture::historical_sample(&source, audit_id, narrative.clone());
    push(
        events,
        "current",
        SessionEvent::CompactionSampleRecorded { sample },
    );
    let sample_id = events.last().unwrap().event_id.clone();
    persist(project, "current", &events[previous_len..]);
    let state = SessionReducer.reduce(events.clone()).unwrap();
    let facts = load_session_compaction_facts(project, &state, false).unwrap();
    let draft = facts
        .compose_session_summary_draft(
            state.history().items(),
            state.latest_compaction().map(|value| value.summary()),
            32,
            ContextWindowPolicy::for_window(32768),
        )
        .unwrap();
    let compaction = crate::legacy_v5_fixture::legacy_v5_candidate(
        &state,
        draft,
        &facts.context(),
        sample_id,
        narrative,
    );
    compaction.validate_references(&facts.context()).unwrap();
    let summary = compaction.summary().clone();
    push(
        events,
        "current",
        SessionEvent::CompactionCreated { compaction },
    );
    persist(project, "current", &events[events.len() - 1..]);
    summary
}

#[tokio::test]
async fn strict_replay_captures_only_latest_v5_prefix_and_reuses_exact_validation_facts() {
    let root = TemporaryDirectory::new("session-read-summary");
    let project = Project::initialize(root.path()).unwrap();
    let tools = ToolRegistry::for_project(root.path());
    let binding = bind_definition(
        builtin_definition(BuiltinAgentTemplate::TechnicalWriting, &tools.catalog()).unwrap(),
        None,
        None,
    )
    .unwrap();
    let mut events = Vec::new();
    push(
        &mut events,
        "current",
        SessionEvent::StartedWithAgent {
            primary_provider_session: provider("current"),
            binding,
        },
    );
    push(
        &mut events,
        "current",
        SessionEvent::HistoryItemRecorded {
            item: ContentItem::user_text("Inspect ordinary notes"),
        },
    );
    push(
        &mut events,
        "current",
        SessionEvent::HistoryItemRecorded {
            item: ContentItem::assistant_text("Recorded neutral observation. ".repeat(100)),
        },
    );
    persist(&project, "current", &events);
    let earlier = append_compaction(&project, &mut events).await;
    push(
        &mut events,
        "current",
        SessionEvent::HistoryItemRecorded {
            item: ContentItem::assistant_text("Further recorded neutral observation. ".repeat(100)),
        },
    );
    persist(&project, "current", &events[events.len() - 1..]);
    let latest = append_compaction(&project, &mut events).await;
    assert_eq!(earlier.schema_version(), 5);
    assert_eq!(latest.schema_version(), 5);
    let mut projection = SessionProjection::replay_project_verified(
        project.project_id().clone(),
        &events,
        &[],
        || Ok(crate::domain::research::ResearchReferenceCatalog::default()),
        &crate::adapters::artifact_store::LocalArtifactStore::for_project(&project),
    )
    .unwrap();
    let (current_source, strict) =
        SessionSourceIdentity::from_events(project.project_id().clone(), &events, &[]).unwrap();
    assert_eq!(projection.state(), &strict);
    assert_eq!(projection.current_source(), Some(&current_source));
    assert!(projection.compaction_source_for(&earlier).is_none());
    let captured = projection.compaction_source_for(&latest).unwrap();
    assert_eq!(
        captured.source(),
        latest.task_references().unwrap().source()
    );
    let boundary = captured.source().watermark().get() as usize;
    let (historical_source, _) =
        SessionSourceIdentity::from_events(project.project_id().clone(), &events[..boundary], &[])
            .unwrap();
    assert_eq!(captured.source(), &historical_source);
    let id = StreamId::new("current");
    let mut cache = SessionReceiptCatalogCache::default();
    let read =
        ProjectReadContext::load(&project, &mut cache, Some((&id, projection.state()))).unwrap();
    for summary in [&earlier, &latest] {
        for cached in [None, projection.compaction_source_for(summary)] {
            summary
                .validate_references(
                    &load_resume_compaction_facts(&project, summary, &read, cached)
                        .unwrap()
                        .context(),
                )
                .unwrap();
            summary
                .validate_references(
                    &load_historical_session_compaction_facts(&project, summary, &read, cached)
                        .unwrap()
                        .context(),
                )
                .unwrap();
        }
    }
    let baseline = load_session_compaction_facts(&project, projection.state(), false).unwrap();
    let cached = load_session_compaction_facts_from_read(
        &project,
        projection.state(),
        projection.current_source(),
        &read,
    )
    .unwrap();
    assert_eq!(
        cached.task_source().unwrap(),
        baseline.task_source().unwrap()
    );
    assert_eq!(
        cached
            .compose_session_summary_draft(
                projection.state().history().items(),
                Some(&latest),
                32,
                ContextWindowPolicy::for_window(32768)
            )
            .unwrap(),
        baseline
            .compose_session_summary_draft(
                projection.state().history().items(),
                Some(&latest),
                32,
                ContextWindowPolicy::for_window(32768)
            )
            .unwrap()
    );
    for change in ["prefix", "unknown_event"] {
        let mut changed = events.clone();
        if change == "prefix" {
            changed[1].payload["item"]["text"] = json!("Rewritten ordinary note goal");
        } else {
            changed[1].event_type = crate::domain::events::EventType::new("unknown.session.fact");
        }
        assert!(
            SessionProjection::replay_project_verified(
                project.project_id().clone(),
                &changed,
                &[],
                || Ok(crate::domain::research::ResearchReferenceCatalog::default()),
                &crate::adapters::artifact_store::LocalArtifactStore::for_project(&project),
            )
            .is_err(),
            "{change}"
        );
    }
    push(
        &mut events,
        "current",
        SessionEvent::HistoryItemRecorded {
            item: ContentItem::assistant_text("A later durable observation."),
        },
    );
    let suffix = events.last().unwrap().clone();
    persist(&project, "current", std::slice::from_ref(&suffix));
    projection
        .ingest_durable_verified(
            vec![suffix],
            &crate::adapters::artifact_store::LocalArtifactStore::for_project(&project),
        )
        .unwrap();
    assert!(projection.current_source().is_none());
    assert!(projection.compaction_source_for(&latest).is_some());
    assert_eq!(projection.state(), &SessionReducer.reduce(events).unwrap());
    let read =
        ProjectReadContext::load(&project, &mut cache, Some((&id, projection.state()))).unwrap();
    let refreshed = load_session_compaction_facts_from_read(
        &project,
        projection.state(),
        projection.current_source(),
        &read,
    )
    .unwrap();
    assert_ne!(refreshed.task_source().unwrap(), &current_source);
    latest
        .validate_references(
            &load_resume_compaction_facts(
                &project,
                &latest,
                &read,
                projection.compaction_source_for(&latest),
            )
            .unwrap()
            .context(),
        )
        .unwrap();
}
