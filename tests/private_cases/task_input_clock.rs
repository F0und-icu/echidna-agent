//! The real input boundary captures time before a queued correction waits.

use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::sync::Mutex;

use crate::app::input_time::{capture_input, queue_input};
use crate::domain::context::{
    AuthorityLevel, ContextOrigin, ContextProvenance, ProviderSession, ProviderSessionId,
    ProviderSessionRole, SemanticClass,
};
use crate::domain::events::{EventEnvelope, EventId, EventSequence, OccurredAt, StreamId};
use crate::domain::model::ContentItem;
use crate::domain::session::{
    OperatorTurnBoundary, SessionEvent, SessionReducer, TaskInputTime, TimeZoneObservation,
};
use crate::ports::input_clock::{InputClock, InputClockError};

#[test]
fn task_epoch_carries_only_an_unconsumed_compaction_reset() {
    use crate::domain::context::{
        BaselineAssessment, BaselineInvalidationReason, InternalRequestMetadata, ModelIdentity,
        ModelProfile, ProviderSessionRef, StepSnapshot, TurnContext, TurnId,
    };
    use crate::domain::model::{
        CapabilityMappingVersion, ModelCapabilities, ModelInputIR, ModelRequestRequirements,
    };

    let caps = ModelCapabilities {
        text_input: true,
        image_input: false,
        streaming: true,
        tool_call: true,
        structured_output: true,
        incremental_input: true,
        usage_reporting: true,
        max_output_tokens: None,
        reasoning_effort: None,
        context_window: 272_000,
        capability_mapping_version: CapabilityMappingVersion::new(1),
    };
    let next = ProviderSessionRef {
        id: ProviderSessionId::new("next-task-epoch"),
        role: ProviderSessionRole::Primary,
        generation: 2,
    };
    let turn = TurnContext::new(
        TurnId::new("next-turn"),
        ModelIdentity::new("fixture", "fixture"),
        ModelProfile::new("fixture", 1),
        next.clone(),
        caps.context_window,
        &[],
    )
    .unwrap();
    let input = ModelInputIR::current(
        "fixture instructions",
        vec![ContentItem::user_text("fixture input")],
        ModelRequestRequirements::default(),
    );
    let snapshot =
        StepSnapshot::capture(&turn, 0, &input, &caps, InternalRequestMetadata::default()).unwrap();
    for (prior_reset, expected) in [
        (None, BaselineInvalidationReason::NoBaseline),
        (
            Some(BaselineInvalidationReason::ToolsChanged),
            BaselineInvalidationReason::NoBaseline,
        ),
        (
            Some(BaselineInvalidationReason::CompactionReset),
            BaselineInvalidationReason::CompactionReset,
        ),
    ] {
        let mut previous = ProviderSession::new(
            ProviderSessionId::new("previous-task-epoch"),
            ProviderSessionRole::Primary,
            1,
        );
        if let Some(reason) = prior_reset {
            previous.reset(reason);
        }
        let mut epoch = previous.next_task_epoch(next.clone());
        assert_eq!(
            epoch.assess_delivery(&snapshot, true),
            BaselineAssessment::FullRequest { reason: expected }
        );
        epoch
            .record_completed_sample(&snapshot, Vec::new())
            .unwrap();
        let later = epoch.next_task_epoch(next.clone());
        assert_eq!(
            later.assess_delivery(&snapshot, true),
            BaselineAssessment::FullRequest {
                reason: BaselineInvalidationReason::NoBaseline,
            }
        );
    }
}

// Ordinary posting counts are tested through their private owner.
#[allow(clippy::duplicate_mod)]
#[path = "../../src/cli_chat/task_posting_lengths.rs"]
mod task_posting_lengths;

#[test]
fn task_posting_counts_preserve_namespaces_updates_and_incomplete_file_errors() {
    let root = crate::support::TemporaryDirectory::new("task-posting-proof");
    let mut proof = task_posting_lengths::PostingLengths::default();
    for (index, namespace) in ["history", "cards", "anchor-post", "card-ref"]
        .into_iter()
        .enumerate()
    {
        let key = blake3::hash(format!("term-{index}").as_bytes());
        let other = blake3::hash(format!("absent-{index}").as_bytes());
        let file = root.path().join(format!("posting-{index}"));
        std::fs::write(&file, [0u8; 120]).unwrap();
        proof.set(namespace, key, 3);
        assert_eq!(
            proof.verify_length(namespace, key, &file, 40).unwrap(),
            Some(3)
        );
        assert_eq!(proof.get(namespace, other), None);
        std::fs::write(&file, [0u8; 80]).unwrap();
        assert!(
            proof.verify_length(namespace, key, &file, 40).is_err(),
            "aligned truncation in {namespace}"
        );
        std::fs::remove_file(&file).unwrap();
        assert!(
            proof.verify_length(namespace, key, &file, 40).is_err(),
            "missing posting in {namespace}"
        );
    }
    for index in 0..64 {
        let key = blake3::hash(format!("additional-{index}").as_bytes());
        proof.set("history", key, index + 1);
    }
    for index in 0..64 {
        let key = blake3::hash(format!("additional-{index}").as_bytes());
        assert_eq!(proof.get("history", key), Some(index + 1));
    }
    let updated = blake3::hash(b"additional-0");
    assert_eq!(proof.len(), 68);
    proof.set("history", updated, 99);
    assert_eq!(proof.get("history", updated), Some(99));
    proof.clear("cards");
    assert_eq!(proof.get("cards", blake3::hash(b"term-1")), None);
    assert_eq!(proof.len(), 67);
}

#[test]
fn navigation_prefix_preserves_exact_json_ranges_for_large_tool_items() {
    use crate::domain::tools::{ToolCall, ToolCallId, ToolName};

    for payload in ["small".to_owned(), "中\\\"".repeat(300_000)] {
        let item = ContentItem::ToolCall {
            call: ToolCall::new(
                ToolCallId::new("prefix-call"),
                ToolName::new("run_command"),
                serde_json::json!({"payload": payload}),
            ),
        };
        let full = serde_json::to_string(&item).unwrap();
        let mut end = full.len().min(1024);
        while !full.is_char_boundary(end) {
            end -= 1;
        }
        let (prefix, truncated) =
            crate::app::context_read::history_navigation_excerpt(&item).unwrap();
        assert_eq!(prefix, &full[..end]);
        assert_eq!(truncated, full.len() > 1024);
    }
}

#[test]
fn session_index_scanner_reports_verified_record_ranges_and_hashes() {
    use crate::adapters::project_event_store::{
        Project, ProjectEventStream, read_indexed_session_segment, visit_existing_session_records,
    };
    use crate::domain::events::EventType;
    use crate::ports::EventSink;

    let root = crate::support::TemporaryDirectory::new("task-index-record-ranges");
    let project = Project::initialize(root.path()).unwrap();
    let id = StreamId::new("index-records");
    let stream_id = ProjectEventStream::Session(id.clone()).stream_id();
    let mut writer = project.open_writer().unwrap();
    let mut store = writer.open_session(id.clone(), None).unwrap();
    for sequence in 1..=3 {
        let event = EventEnvelope::current(
            EventId::new(format!("source-{sequence}")),
            stream_id.clone(),
            EventSequence::new(sequence),
            OccurredAt::new("2026-11-06T00:00:00Z"),
            EventType::new("fixture.event"),
            serde_json::json!({"value": "测试", "sequence": sequence}),
        );
        store.append_event(&event).unwrap();
    }
    drop(store);
    drop(writer);
    let bytes = std::fs::read(project.session_stream_path_existing(&id).unwrap()).unwrap();
    let mut ranges = Vec::new();
    let (length, digest) =
        visit_existing_session_records(&project, &id, &mut |event, offset, len, hash, raw| {
            assert_eq!(blake3::hash(raw), hash);
            ranges.push((event.event_id.clone(), offset, len, hash));
            Ok(())
        })
        .unwrap();
    assert_eq!(length, bytes.len());
    assert_eq!(digest, blake3::hash(&bytes));
    assert_eq!(ranges.len(), 3);
    assert_eq!(ranges[0].1, 0);
    for (index, (event_id, offset, len, hash)) in ranges.iter().enumerate() {
        assert_eq!(event_id.as_str(), format!("source-{}", index + 1));
        assert_eq!(*hash, blake3::hash(&bytes[*offset..offset + len]));
        if index > 0 {
            assert_eq!(*offset, ranges[index - 1].1 + ranges[index - 1].2);
        }
    }
    let (_, offset, len, hash) = &ranges[1];
    assert_eq!(
        read_indexed_session_segment(&project, &id, *offset as u64, *len, *hash).unwrap(),
        bytes[*offset..offset + len]
    );
    assert!(
        read_indexed_session_segment(&project, &id, 0, 4 * 1024 * 1024 + 1, blake3::hash(b""),)
            .is_err()
    );
    let mut changed = bytes.clone();
    changed[*offset] ^= 1;
    std::fs::write(project.session_stream_path_existing(&id).unwrap(), &changed).unwrap();
    assert!(read_indexed_session_segment(&project, &id, *offset as u64, *len, *hash).is_err());
}

struct FixtureClock(Mutex<VecDeque<TaskInputTime>>);

impl InputClock for FixtureClock {
    fn now(&self) -> Result<TaskInputTime, InputClockError> {
        self.0
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| InputClockError::new(io::Error::other("fixture clock exhausted")))
    }
}

fn time(unix_seconds: i64) -> TaskInputTime {
    TaskInputTime {
        unix_seconds,
        nanoseconds: 123_456_789,
        timezone: TimeZoneObservation::HostSystem {
            offset_seconds: 8 * 3600,
            name: Some("fixture/UTC+08".to_owned()),
        },
    }
}

fn event(payload: SessionEvent, sequence: u64) -> EventEnvelope<serde_json::Value> {
    payload
        .into_envelope(
            EventId::new(format!("clock-{sequence}")),
            StreamId::new("session:clock"),
            EventSequence::new(sequence),
            OccurredAt::new("unix:1793894401.123456789"),
        )
        .unwrap()
}

#[test]
fn queued_input_persists_the_receive_instant_across_local_midnight() {
    // UTC+08 crosses 2026-11-05 23:59:59 to 2026-11-06 00:00:01.
    let before = time(1_793_894_399);
    let after = time(1_793_894_401);
    let clock = FixtureClock(Mutex::new(VecDeque::from([before.clone(), after.clone()])));
    let mut queued = None;
    queue_input(
        &mut queued,
        capture_input("continue the earlier task".to_owned(), &clock).unwrap(),
    );
    let later = capture_input("a later line".to_owned(), &clock).unwrap();
    assert_eq!(later.time, after);
    let first = queued.take().unwrap();
    assert_eq!(first.time, before);

    let item = ContentItem::user_text(&first.text);
    let events = vec![
        event(
            SessionEvent::Started {
                primary_provider_session: ProviderSession::new(
                    ProviderSessionId::new("clock-primary"),
                    ProviderSessionRole::Primary,
                    0,
                )
                .reference()
                .clone(),
            },
            1,
        ),
        event(SessionEvent::HistoryItemRecorded { item: item.clone() }, 2),
    ];
    let mut state = SessionReducer.reduce(events).unwrap();
    let provenance = BTreeMap::from([(0, ContextProvenance::operator_input(&first.text).unwrap())]);
    let boundary = state
        .accepted_operator_input(&[item], &provenance, first.time.clone())
        .unwrap();
    assert!(
        matches!(&boundary, OperatorTurnBoundary::Accepted { received_at: Some(actual), .. } if actual == &before)
    );
    SessionReducer
        .apply(
            &mut state,
            event(SessionEvent::OperatorTurnBoundaryRecorded { boundary }, 3),
        )
        .unwrap();
    let (_, persisted) = state.task_input_time(&EventId::new("clock-2")).unwrap();
    assert_eq!(persisted, before);
    assert!(clock.0.lock().unwrap().is_empty());
}

#[test]
fn real_refresh_cards_keep_unique_slots_under_repeated_navigation() {
    use crate::domain::session::TaskNavigationChange;
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/task_context_proof_v1");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(fixture.join("manifest.json")).unwrap()).unwrap();
    let hash = manifest["files"]
        .as_object()
        .unwrap()
        .iter()
        .find(|(path, _)| path.ends_with("/events.jsonl"))
        .unwrap()
        .1
        .as_str()
        .unwrap();
    let bytes = std::fs::read_to_string(fixture.join("blobs").join(hash)).unwrap();
    let events = bytes
        .lines()
        .map(|line| serde_json::from_str::<EventEnvelope<serde_json::Value>>(line).unwrap())
        .collect::<Vec<_>>();
    let selections = events
        .iter()
        .filter(|event| event.event_type.as_str() == "session.task_context_selected")
        .collect::<Vec<_>>();
    assert_eq!(
        selections[1].payload["task_revisions"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(
        selections[2].payload["refresh_of"],
        selections[1].event_id.as_str()
    );
    assert_eq!(
        selections[2].payload["task_revisions"],
        selections[1].payload["task_revisions"]
    );
    let input = selections[1].payload["input_ref"]
        .as_str()
        .unwrap()
        .to_owned();
    let state = SessionReducer.reduce(events).unwrap();
    let index = state
        .history_items_with_event_ids()
        .position(|(id, _)| id.as_str() == input)
        .unwrap();
    let mut slot = 0;
    let mut cards = BTreeMap::new();
    let mut recent = VecDeque::new();
    // The first two calls are real adjacent selection and refresh facts. The
    // remaining calls stress the same legal emitted revisions without adding
    // fabricated durable events or performing thousands of tool executions.
    for selection in std::iter::once(1).chain(std::iter::repeat_n(2, 21_846)) {
        state
            .task_navigation_rebuild_selection(index, selection, &mut slot, |change| match change {
                TaskNavigationChange::Card { ordinal, card, .. } => {
                    if let std::collections::btree_map::Entry::Vacant(entry) =
                        cards.entry(card.reference)
                    {
                        entry.insert(ordinal);
                        Ok(true)
                    } else {
                        Ok(false)
                    }
                }
                TaskNavigationChange::Recent(id) => {
                    recent.push_back(id);
                    if recent.len() > 64 {
                        recent.pop_front();
                    }
                    Ok(false)
                }
                TaskNavigationChange::Anchor { .. } => unreachable!(),
            })
            .unwrap();
    }
    assert_eq!(slot, 3);
    assert_eq!(cards.len(), 3);
    assert_eq!(recent.len(), 64);
    assert_eq!(
        cards
            .values()
            .copied()
            .collect::<std::collections::BTreeSet<_>>(),
        (index * 65536..index * 65536 + 3).collect()
    );
}

#[test]
fn task_source_authority_distinguishes_legacy_user_assistant_and_accepted_input() {
    let legacy = ContentItem::user_text("legacy user-role text");
    let answer = ContentItem::assistant_text("assistant observation");
    let accepted = ContentItem::user_text("current operator instruction");
    let events = vec![
        event(
            SessionEvent::Started {
                primary_provider_session: ProviderSession::new(
                    ProviderSessionId::new("source-primary"),
                    ProviderSessionRole::Primary,
                    0,
                )
                .reference()
                .clone(),
            },
            1,
        ),
        event(
            SessionEvent::HistoryItemRecorded {
                item: legacy.clone(),
            },
            2,
        ),
        event(
            SessionEvent::HistoryItemRecorded {
                item: answer.clone(),
            },
            3,
        ),
        event(
            SessionEvent::HistoryItemRecorded {
                item: accepted.clone(),
            },
            4,
        ),
    ];
    let mut state = SessionReducer.reduce(events).unwrap();
    let provenance = BTreeMap::from([(
        0,
        ContextProvenance::operator_input("current operator instruction").unwrap(),
    )]);
    let boundary = state
        .accepted_operator_input(
            std::slice::from_ref(&accepted),
            &provenance,
            time(1_793_894_401),
        )
        .unwrap();
    SessionReducer
        .apply(
            &mut state,
            event(SessionEvent::OperatorTurnBoundaryRecorded { boundary }, 5),
        )
        .unwrap();
    let legacy = state
        .history_source_semantics(&EventId::new("clock-2"), &legacy)
        .unwrap();
    assert_eq!(
        (legacy.origin, legacy.class, legacy.authority),
        (
            ContextOrigin::SessionProjection,
            SemanticClass::Conversation,
            AuthorityLevel::UnclassifiedLegacy
        )
    );
    let answer = state
        .history_source_semantics(&EventId::new("clock-3"), &answer)
        .unwrap();
    assert_eq!(
        (answer.origin, answer.class, answer.authority),
        (
            ContextOrigin::ModelOutput,
            SemanticClass::ModelObservation,
            AuthorityLevel::ModelObservation
        )
    );
    let accepted = state
        .history_source_semantics(&EventId::new("clock-4"), &accepted)
        .unwrap();
    assert_eq!(
        (accepted.origin, accepted.authority),
        (
            ContextOrigin::OperatorInput,
            AuthorityLevel::OperatorAssertion
        )
    );
}

#[test]
fn task_navigation_hot_index_updates_only_the_suffix_and_evicts_at_capacity() {
    let mut index = crate::domain::session::TaskNavigationIndex::default();
    for ordinal in 0..512 {
        index.append(
            ordinal,
            &ContentItem::user_text(format!("source-{ordinal}")),
        );
    }
    let prior = index.clone();
    assert_eq!(prior.get(0), Some(("source-0", false)));
    assert_eq!(prior.get(511), Some(("source-511", false)));
    let long = format!("{}你", "x".repeat(1023));
    index.append(512, &ContentItem::user_text(&long));
    assert_eq!(index.get(0), None);
    assert_eq!(index.get(1), prior.get(1));
    assert_eq!(index.get(511), prior.get(511));
    let (cached, truncated) = index.get(512).unwrap();
    assert!(truncated);
    assert_eq!(cached.len(), 1023);
    assert!(long.starts_with(cached));
    assert_eq!(
        prior.get(0),
        Some(("source-0", false)),
        "updated cache does not mutate the prior projection"
    );
}

#[test]
fn task_navigation_cold_query_recalls_original_event_after_hot_eviction() {
    let mut events = vec![event(
        SessionEvent::Started {
            primary_provider_session: ProviderSession::new(
                ProviderSessionId::new("cold-primary"),
                ProviderSessionRole::Primary,
                0,
            )
            .reference()
            .clone(),
        },
        1,
    )];
    let cold = "COLDUNIQUE789 original old task material";
    for index in 0..513 {
        let text = if index == 0 {
            cold.to_owned()
        } else {
            format!("unrelated history item {index}")
        };
        let item = if index % 2 == 0 {
            ContentItem::user_text(text)
        } else {
            ContentItem::assistant_text(text)
        };
        events.push(event(
            SessionEvent::HistoryItemRecorded { item },
            index as u64 + 2,
        ));
    }
    let current = ContentItem::user_text("Find COLDUNIQUE789");
    events.push(event(
        SessionEvent::HistoryItemRecorded {
            item: current.clone(),
        },
        515,
    ));
    let mut state = SessionReducer.reduce(events).unwrap();
    let provenance = BTreeMap::from([(
        0,
        ContextProvenance::operator_input("Find COLDUNIQUE789").unwrap(),
    )]);
    let accepted = state
        .accepted_operator_input(&[current], &provenance, time(1_793_894_401))
        .unwrap();
    SessionReducer
        .apply(
            &mut state,
            event(
                SessionEvent::OperatorTurnBoundaryRecorded { boundary: accepted },
                516,
            ),
        )
        .unwrap();
    let candidates = state
        .task_selection_candidates(
            "project-cold",
            &EventId::new("clock-515"),
            None,
            &crate::app::context_read::history_navigation_excerpt,
        )
        .unwrap();
    assert!((65..=512).contains(&candidates.context.len()));
    assert!(candidates.gaps.iter().any(|gap| gap
        == &format!(
            "{} older context candidates require expansion",
            513 - candidates.context.len()
        )));
    for sequence in 451..=514 {
        assert!(
            candidates
                .context
                .iter()
                .any(|candidate| candidate.reference == format!("clock-{sequence}")),
            "the recent source tail remains available"
        );
    }
    assert!(
        !candidates
            .context
            .iter()
            .any(|candidate| candidate.reference == "clock-3"),
        "the old pool is not silently presented as complete"
    );
    let assistant = candidates
        .context
        .iter()
        .find(|candidate| candidate.reference == "clock-451")
        .unwrap();
    assert_eq!(assistant.source_origin, Some(ContextOrigin::ModelOutput));
    assert_eq!(
        assistant.source_authority,
        Some(AuthorityLevel::ModelObservation)
    );
    assert!(
        candidates
            .context
            .iter()
            .all(|candidate| candidate.task_ref.is_none())
    );
    let original = candidates
        .context
        .iter()
        .find(|candidate| candidate.reference == "clock-2")
        .expect("query recalls the evicted original Event");
    assert_eq!(original.source_event_ref.as_deref(), Some("clock-2"));
    assert_eq!(original.excerpt, cold);
    assert_eq!(
        original.source_origin,
        Some(ContextOrigin::SessionProjection)
    );
    assert_eq!(
        original.source_authority,
        Some(AuthorityLevel::UnclassifiedLegacy)
    );
    assert!(!original.truncated);
}

#[test]
fn legacy_large_task_progress_is_bounded_and_preserves_original_source() {
    use crate::domain::artifacts::BlobId;
    use crate::domain::project::ProjectId;
    use crate::domain::session::{SessionSourceIdentity, TaskContextSelected};
    let provider = ProviderSession::new(
        ProviderSessionId::new("legacy-progress-primary"),
        ProviderSessionRole::Primary,
        0,
    );
    let mut events = vec![event(
        SessionEvent::Started {
            primary_provider_session: provider.reference().clone(),
        },
        1,
    )];
    let original = format!("LEGACY-OBJECTIVE {} LEGACY-TAIL", "中".repeat(20_000));
    for (index, text) in [original.as_str(), "Continue"].into_iter().enumerate() {
        let sequence = 2 + index as u64 * 2;
        let item = ContentItem::user_text(text);
        events.push(event(
            SessionEvent::HistoryItemRecorded { item: item.clone() },
            sequence,
        ));
        let state = SessionReducer.reduce(events.iter().cloned()).unwrap();
        let provenance = BTreeMap::from([(0, ContextProvenance::operator_input(text).unwrap())]);
        let boundary = state
            .accepted_operator_input(&[item], &provenance, time(1_793_894_401))
            .unwrap();
        events.push(event(
            SessionEvent::OperatorTurnBoundaryRecorded { boundary },
            sequence + 1,
        ));
    }
    let (source, state) =
        SessionSourceIdentity::from_events(ProjectId::new("legacy-progress"), &events, &[])
            .unwrap();
    let decision = serde_json::from_value(serde_json::json!({"relation":"return", "focus":{"kind":"existing","task_ref":"legacy:clock-2"},
        "related_task_refs":[],"required_context_refs":[],"unresolved_references":[]})).unwrap();
    let selection = TaskContextSelected {
        schema_version: 1,
        request_proof_version: 0,
        source,
        capture_event_id: EventId::new("capture"),
        capture_blob: BlobId::new("0".repeat(64)).unwrap(),
        input_ref: EventId::new("clock-4"),
        input_boundary_event_id: EventId::new("clock-5"),
        input_time: time(1_793_894_401),
        sample_event_id: None,
        refresh_of: None,
        decision,
        selected_history: vec![],
        selected_excerpts: vec![],
        selected_core: vec![],
        selected_research: vec![],
        selected_memory: vec![],
        selected_summary: None,
        core_scopes: vec![],
        constraint_revisions: vec![],
        selected_constraints: vec![],
        unresolved_gaps: vec![],
        live_processes: vec![],
        task_revisions: vec![],
        policy_version: 1,
        binding_digest: "0".repeat(64),
        provider_session: provider.reference().clone(),
        planned_context_hash: "0".repeat(64),
    };
    let progress = state
        .selected_task_progress(
            &selection,
            &crate::app::context_read::history_navigation_excerpt,
        )
        .unwrap();
    assert_eq!(progress.len(), 1);
    let value: serde_json::Value = serde_json::from_str(&progress[0].text).unwrap();
    let goal = value["goal"].as_str().unwrap();
    assert!(goal.len() <= 1024);
    assert_eq!(goal, &original[..goal.len()]);
    assert_eq!(value["goal_truncated"], true);
    assert_eq!(
        value["goal_source_range"],
        serde_json::json!([0, original.len()])
    );
    assert_eq!(
        value["goal_inspected_range"],
        serde_json::json!([0, goal.len()])
    );
    assert_eq!(progress[0].event_refs, vec![EventId::new("clock-2")]);
    assert_eq!(state.history().items()[0], ContentItem::user_text(original));
}

#[test]
fn task_source_anchor_closure_preserves_old_policy_and_bounded_original_text() {
    let mut state = SessionReducer
        .reduce([event(
            SessionEvent::Started {
                primary_provider_session: ProviderSession::new(
                    ProviderSessionId::new("anchor-policy"),
                    ProviderSessionRole::Primary,
                    0,
                )
                .reference()
                .clone(),
            },
            1,
        )])
        .unwrap();
    let original = format!("ARCHIVED-ZEBRA {}", "x".repeat(2048));
    let items = std::iter::once(ContentItem::user_text(&original))
        .chain((0..530).map(|index| ContentItem::assistant_text(format!("filler-{index}"))))
        .chain(std::iter::once(ContentItem::user_text("量子纠缠实验")));
    let mut sequence = 2;
    let mut input = EventId::new("unused");
    for item in items {
        input = EventId::new(format!("clock-{sequence}"));
        SessionReducer
            .apply(
                &mut state,
                event(
                    SessionEvent::HistoryItemRecorded { item: item.clone() },
                    sequence,
                ),
            )
            .unwrap();
        sequence += 1;
        if let ContentItem::Text {
            role: crate::domain::model::MessageRole::User,
            text,
        } = &item
        {
            let provenance =
                BTreeMap::from([(0, ContextProvenance::operator_input(text).unwrap())]);
            let boundary = state
                .accepted_operator_input(&[item], &provenance, time(1_793_894_401))
                .unwrap();
            SessionReducer
                .apply(
                    &mut state,
                    event(
                        SessionEvent::OperatorTurnBoundaryRecorded { boundary },
                        sequence,
                    ),
                )
                .unwrap();
            sequence += 1;
        }
    }
    let capabilities = crate::domain::model::ModelCapabilities {
        text_input: true,
        image_input: false,
        streaming: true,
        tool_call: true,
        structured_output: false,
        incremental_input: false,
        usage_reporting: false,
        context_window: 272_000,
        max_output_tokens: None,
        reasoning_effort: None,
        capability_mapping_version: crate::domain::model::CapabilityMappingVersion::new(1),
    };
    let common = crate::domain::session::CompactionContextCapture::new_task(
        capabilities,
        crate::domain::context::ContextPlanner::current().budget_policy(),
        1,
        Vec::new(),
    );
    let candidates = |policy| {
        state
            .task_selection_candidates_with_capture(
                "anchor-policy",
                &input,
                &common,
                &[],
                0,
                &[],
                None,
                false,
                policy,
                &crate::app::context_read::history_navigation_excerpt,
            )
            .unwrap()
    };
    let old = candidates(4);
    let current = candidates(5);
    assert_eq!(old.tasks, current.tasks);
    assert_eq!(old.tasks[0].source_ref, "clock-2");
    assert!(!old.context.iter().any(|c| c.reference == "clock-2"));
    assert_eq!(current.context.len(), old.context.len() + 1);
    assert_eq!(
        &current.context[1..],
        old.context.as_slice(),
        "ranked candidates are not displaced"
    );
    let anchor = &current.context[0];
    assert_eq!(anchor.reference, "clock-2");
    assert_eq!(anchor.source_event_ref.as_deref(), Some("clock-2"));
    assert_eq!(anchor.excerpt, original[..1024]);
    assert!(anchor.truncated);
    assert_eq!(anchor.source_origin, Some(ContextOrigin::OperatorInput));
    assert_eq!(
        anchor.source_authority,
        Some(AuthorityLevel::OperatorAssertion)
    );
    let mut decision: crate::domain::task_context::TaskSelectionDecision =
        serde_json::from_value(serde_json::json!({
            "relation":"continue", "focus":{"kind":"existing","task_ref":old.tasks[0].reference},
            "related_task_refs":[], "required_context_refs":["clock-2"], "unresolved_references":[]
        }))
        .unwrap();
    assert!(old.validate_decision(&decision).is_err());
    current.validate_decision(&decision).unwrap();
    decision
        .required_context_refs
        .push("missing-source".to_owned());
    assert!(
        matches!(current.validate_decision(&decision), Err(crate::domain::task_context::TaskSelectionError::UnknownReference { reference, field: "required_context_refs" }) if reference == "missing-source")
    );
}

#[test]
fn task_cards_are_bounded_and_query_recalls_an_old_legacy_task() {
    let mut state = SessionReducer
        .reduce([event(
            SessionEvent::Started {
                primary_provider_session: ProviderSession::new(
                    ProviderSessionId::new("cold-card-primary"),
                    ProviderSessionRole::Primary,
                    0,
                )
                .reference()
                .clone(),
            },
            1,
        )])
        .unwrap();
    for index in 0..530_u64 {
        let text = if index == 0 {
            "COLDTASKUNIQUE789 original task".to_owned()
        } else {
            format!("unrelated task {index}")
        };
        let item = ContentItem::user_text(&text);
        let sequence = 2 + index * 2;
        SessionReducer
            .apply(
                &mut state,
                event(
                    SessionEvent::HistoryItemRecorded { item: item.clone() },
                    sequence,
                ),
            )
            .unwrap();
        let provenance = BTreeMap::from([(0, ContextProvenance::operator_input(&text).unwrap())]);
        let boundary = state
            .accepted_operator_input(&[item], &provenance, time(1_793_894_401))
            .unwrap();
        SessionReducer
            .apply(
                &mut state,
                event(
                    SessionEvent::OperatorTurnBoundaryRecorded { boundary },
                    sequence + 1,
                ),
            )
            .unwrap();
    }
    let current_text = "Find COLDTASKUNIQUE789";
    let current = ContentItem::user_text(current_text);
    SessionReducer
        .apply(
            &mut state,
            event(
                SessionEvent::HistoryItemRecorded {
                    item: current.clone(),
                },
                1062,
            ),
        )
        .unwrap();
    let provenance =
        BTreeMap::from([(0, ContextProvenance::operator_input(current_text).unwrap())]);
    let boundary = state
        .accepted_operator_input(&[current], &provenance, time(1_793_894_401))
        .unwrap();
    SessionReducer
        .apply(
            &mut state,
            event(
                SessionEvent::OperatorTurnBoundaryRecorded { boundary },
                1063,
            ),
        )
        .unwrap();
    // Each source boundary is visited once and can be persisted immediately.
    let positions = state
        .history_items_with_event_ids()
        .enumerate()
        .map(|(index, (id, _))| (id.clone(), index))
        .collect::<BTreeMap<_, _>>();
    let mut boundary_count = 0;
    let mut previous = None;
    state
        .task_navigation_rebuild_sources(
            &EventId::new("clock-1062"),
            |id| Ok(positions[id]),
            |_, _| panic!("legacy fixture has no selection events"),
            |index| {
                assert!(previous.is_none_or(|prior| prior < index));
                previous = Some(index);
                boundary_count += 1;
                let (_, _, card) = state.task_navigation_rebuild_legacy(index)?.unwrap();
                assert_eq!(card.reference, format!("legacy:clock-{}", 2 + index * 2));
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(boundary_count, 530);
    let candidates = state
        .task_selection_candidates(
            "project-cold-card",
            &EventId::new("clock-1062"),
            None,
            &crate::app::context_read::history_navigation_excerpt,
        )
        .unwrap();
    assert!(candidates.tasks.len() <= 512);
    assert!(
        candidates
            .gaps
            .iter()
            .any(|gap| gap.contains("Task candidate limit reached"))
    );
    assert!(
        candidates
            .tasks
            .iter()
            .any(|task| task.reference == "legacy:clock-2"
                && task.goal.contains("COLDTASKUNIQUE789"))
    );
    for candidate in &candidates.context {
        assert_eq!(
            candidate.task_ref.as_deref(),
            Some(format!("legacy:{}", candidate.reference).as_str()),
            "each accepted input belongs to its own original legacy task"
        );
    }

    // A real task outside the initial pool may become valid after expansion.
    // Feedback must reflect the expanded pool, not insist it is still absent.
    let omitted = (0..530)
        .map(|i| format!("legacy:clock-{}", 2 + i * 2))
        .find(|id| !candidates.tasks.iter().any(|task| &task.reference == id))
        .unwrap();
    let proposal: crate::domain::task_context::TaskSelectionDecision =
        serde_json::from_value(serde_json::json!({
            "relation":"new", "focus":{"kind":"new_from_input","input_ref":"clock-1062"},
            "related_task_refs":[omitted], "required_context_refs":[], "unresolved_references":[]
        }))
        .unwrap();
    assert!(matches!(
        candidates.validate_decision(&proposal),
        Err(crate::domain::task_context::TaskSelectionError::UnknownReference { .. })
    ));
    let expanded = state
        .task_selection_candidates(
            "project-cold-card",
            &EventId::new("clock-1062"),
            Some(&proposal),
            &crate::app::context_read::history_navigation_excerpt,
        )
        .unwrap();
    expanded.validate_decision(&proposal).unwrap();

    // Exercise the same production candidate policy with a finite posting
    // reader, then independently rebuild from Session facts for comparison.
    struct Indexed<'a> {
        state: &'a crate::domain::session::SessionState,
        cards: Vec<crate::domain::task_context::TaskCandidate>,
        card_reads: std::cell::Cell<usize>,
        semantics_reads: std::cell::Cell<usize>,
    }
    impl crate::domain::session::TaskNavigationAccess for Indexed<'_> {
        fn task_count(&self) -> usize {
            self.cards.len()
        }
        fn recent_cards(&self) -> Result<Vec<usize>, crate::domain::session::SessionError> {
            Ok((self.cards.len().saturating_sub(64)..self.cards.len())
                .rev()
                .map(|i| i * 65536)
                .collect())
        }
        fn all_cards(&self) -> Result<Vec<usize>, crate::domain::session::SessionError> {
            panic!("large task catalog must use bounded postings")
        }
        fn task_ordinal(
            &self,
            id: &str,
        ) -> Result<Option<usize>, crate::domain::session::SessionError> {
            Ok(self
                .cards
                .iter()
                .position(|card| card.reference == id)
                .map(|i| i * 65536))
        }
        fn card(
            &self,
            ordinal: usize,
        ) -> Result<crate::domain::task_context::TaskCandidate, crate::domain::session::SessionError>
        {
            self.card_reads.set(self.card_reads.get() + 1);
            Ok(self.cards[ordinal / 65536].clone())
        }
        fn task_at(
            &self,
            index: usize,
        ) -> Result<Option<String>, crate::domain::session::SessionError> {
            Ok(self.cards.get(index).map(|card| card.reference.clone()))
        }
        fn semantics(
            &self,
            index: usize,
        ) -> Result<
            Option<crate::domain::session::HistorySourceSemantics>,
            crate::domain::session::SessionError,
        > {
            self.semantics_reads.set(self.semantics_reads.get() + 1);
            let (id, item) = self
                .state
                .history_items_with_event_ids()
                .nth(index)
                .unwrap();
            Ok(Some(self.state.history_source_semantics(id, item)?))
        }
        fn related_history(
            &self,
            _ids: &[String],
            _before: usize,
            _limit: usize,
        ) -> Result<Vec<usize>, crate::domain::session::SessionError> {
            Ok(Vec::new())
        }
    }
    let cards = (0..530)
        .map(|index| {
            let (id, item) = state.history_items_with_event_ids().nth(index).unwrap();
            let ContentItem::Text { text, .. } = item else {
                unreachable!()
            };
            crate::domain::task_context::TaskCandidate {
                reference: format!("legacy:{id}"),
                goal: text.clone(),
                source_ref: id.to_string(),
            }
        })
        .collect::<Vec<_>>();
    let indexed = Indexed {
        state: &state,
        cards,
        card_reads: std::cell::Cell::new(0),
        semantics_reads: std::cell::Cell::new(0),
    };
    let mut postings = BTreeMap::<(bool, String), Vec<usize>>::new();
    for (index, card) in indexed.cards.iter().enumerate() {
        for key in crate::domain::retrieval::task_posting_keys(&card.goal) {
            postings
                .entry((false, key.clone()))
                .or_default()
                .push(index);
            postings.entry((true, key)).or_default().push(index * 65536);
        }
    }
    let posting_calls = std::cell::Cell::new(0);
    let reader = |card: bool,
                  key: &str,
                  before: usize,
                  limit: usize|
     -> Result<(Vec<usize>, bool), crate::domain::session::SessionError> {
        posting_calls.set(posting_calls.get() + 1);
        assert_eq!(limit, 16);
        let values = postings
            .get(&(card, key.to_owned()))
            .map(Vec::as_slice)
            .unwrap_or_default();
        let end = values.partition_point(|value| *value < before);
        let start = end.saturating_sub(limit);
        Ok((values[start..end].to_vec(), start > 0))
    };
    let capabilities = crate::domain::model::ModelCapabilities {
        text_input: true,
        image_input: false,
        streaming: true,
        tool_call: true,
        structured_output: false,
        incremental_input: false,
        usage_reporting: false,
        context_window: 272_000,
        max_output_tokens: None,
        reasoning_effort: None,
        capability_mapping_version: crate::domain::model::CapabilityMappingVersion::new(1),
    };
    let common = crate::domain::session::CompactionContextCapture::new_task(
        capabilities,
        crate::domain::context::ContextPlanner::current().budget_policy(),
        1,
        Vec::new(),
    );
    let source = state
        .task_selection_candidates_with_capture(
            "project-cold-card",
            &EventId::new("clock-1062"),
            &common,
            &[],
            0,
            &[],
            None,
            false,
            crate::domain::session::TASK_CANDIDATE_POLICY_VERSION,
            &crate::app::context_read::history_navigation_excerpt,
        )
        .unwrap();
    let cached = state
        .task_selection_candidates_with_index(
            "project-cold-card",
            &EventId::new("clock-1062"),
            &common,
            &[],
            0,
            &[],
            None,
            false,
            crate::domain::session::TASK_CANDIDATE_POLICY_VERSION,
            &crate::app::context_read::history_navigation_excerpt,
            Some(&reader),
            Some(&indexed),
        )
        .unwrap();
    assert_eq!(cached, source);
    assert!(posting_calls.get() <= 128);
    assert!(indexed.card_reads.get() <= 512);
    assert!(indexed.semantics_reads.get() <= 513);

    let long_text = format!("Find COLDTASKUNIQUE789 {}", "A".repeat(8000));
    let long_item = ContentItem::user_text(&long_text);
    SessionReducer
        .apply(
            &mut state,
            event(
                SessionEvent::HistoryItemRecorded {
                    item: long_item.clone(),
                },
                1064,
            ),
        )
        .unwrap();
    let provenance = BTreeMap::from([(0, ContextProvenance::operator_input(&long_text).unwrap())]);
    let boundary = state
        .accepted_operator_input(&[long_item], &provenance, time(1_793_894_402))
        .unwrap();
    SessionReducer
        .apply(
            &mut state,
            event(
                SessionEvent::OperatorTurnBoundaryRecorded { boundary },
                1065,
            ),
        )
        .unwrap();
    let long_candidates = state
        .task_selection_candidates(
            "project-cold-card",
            &EventId::new("clock-1064"),
            None,
            &crate::app::context_read::history_navigation_excerpt,
        )
        .unwrap();
    assert_eq!(long_candidates.input.text, long_text);
    assert!(
        long_candidates
            .gaps
            .iter()
            .any(|gap| gap.contains("task navigation query bytes 4096.."))
    );
    assert!(long_candidates.context.len() <= 512);
    assert!(
        long_candidates
            .tasks
            .iter()
            .any(|task| task.reference == "legacy:clock-2")
    );
}

#[test]
fn context_time_filter_reads_known_rfc3339_offsets_without_using_host_timezone() {
    use crate::app::context_read::unix_seconds;
    assert_eq!(unix_seconds("unix:1793894399.123"), Some(1_793_894_399));
    assert_eq!(unix_seconds("2026-11-05T15:59:59Z"), Some(1_793_894_399));
    assert_eq!(
        unix_seconds("2026-11-05T23:59:59.123+08:00"),
        Some(1_793_894_399)
    );
    assert_eq!(
        unix_seconds("2026-11-05t11:59:59-04:00"),
        Some(1_793_894_399)
    );
    for bad in [
        "2025-02-29T00:00:00Z",
        "2026-11-05T00:00:00+24:00",
        "2026-11-05T00:00:00.Z",
        "2026-1 -05T00:00:00Z",
    ] {
        assert_eq!(unix_seconds(bad), None, "{bad}");
    }
}
