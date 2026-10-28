//! Durable input identity must follow the actual History, including on replay.

use std::collections::BTreeMap;

use crate::domain::context::{
    ContextProvenance, ProviderSession, ProviderSessionId, ProviderSessionRole,
};
use crate::domain::events::{EventId, EventSequence, OccurredAt, StreamId};
use crate::domain::model::ContentItem;
use crate::domain::project::ProjectId;
use crate::domain::session::{
    CompactionSourceNode, CompactionSourceNodeInput, CompactionSourcePlan, CompactionSourceRange,
    NarrativeSummary, SessionEvent, SessionReducer, SessionSourceIdentity, SessionSummaryDraft,
};

#[test]
fn v6_narrative_allowance_accounts_for_the_actual_encoded_goal() {
    let goal = "task \"quoted\" \u{1}";
    let minimum = crate::app::agent_loop::minimum_compaction_handoff_bytes();
    let narrative = NarrativeSummary::new(goal, "a", Vec::new(), Vec::new()).unwrap();
    let encoded = serde_json::to_vec(&narrative).unwrap().len();
    assert!(SessionSummaryDraft::v6_handoff_byte_limit(goal, encoded - 1, minimum).is_err());
    assert_eq!(
        SessionSummaryDraft::v6_handoff_byte_limit(goal, encoded, minimum).unwrap(),
        minimum
    );
    assert_eq!(
        SessionSummaryDraft::v6_handoff_byte_limit(goal, encoded + 20, minimum).unwrap(),
        minimum + 20
    );
}

fn envelope(
    event: SessionEvent,
    sequence: u64,
) -> crate::domain::events::EventEnvelope<serde_json::Value> {
    event
        .into_envelope(
            EventId::new(format!("input-event-{sequence}")),
            StreamId::new("session:context2-input"),
            EventSequence::new(sequence),
            OccurredAt::new("2026-10-27T00:00:00Z"),
        )
        .unwrap()
}

#[test]
fn source_tree_requires_exact_utf8_coverage_and_a_single_reachable_root() {
    let events = vec![
        envelope(
            SessionEvent::Started {
                primary_provider_session: ProviderSession::new(
                    ProviderSessionId::new("source-test"),
                    ProviderSessionRole::Primary,
                    0,
                )
                .reference()
                .clone(),
            },
            1,
        ),
        envelope(
            SessionEvent::HistoryItemRecorded {
                item: ContentItem::user_text("protected"),
            },
            2,
        ),
        envelope(
            SessionEvent::HistoryItemRecorded {
                item: ContentItem::assistant_text("中a"),
            },
            3,
        ),
    ];
    let (source, _) =
        SessionSourceIdentity::from_events(ProjectId::new("context2"), &events, &[]).unwrap();
    let (entries, contents) = CompactionSourcePlan::source_entries(&source, &events, &[]).unwrap();
    let chinese = contents[1].find('中').unwrap();
    let split = chinese + '中'.len_utf8();
    let nodes = vec![
        CompactionSourceNode {
            id: "left".into(),
            input: CompactionSourceNodeInput::Source(vec![CompactionSourceRange {
                entry: 1,
                start: 0,
                end: split,
            }]),
            is_root: false,
            output_limit: 100,
            template_hash: None,
        },
        CompactionSourceNode {
            id: "right".into(),
            input: CompactionSourceNodeInput::Source(vec![CompactionSourceRange {
                entry: 1,
                start: split,
                end: contents[1].len(),
            }]),
            is_root: false,
            output_limit: 100,
            template_hash: None,
        },
        CompactionSourceNode {
            id: "root".into(),
            input: CompactionSourceNodeInput::Merge(vec!["left".into(), "right".into()]),
            is_root: true,
            output_limit: 100,
            template_hash: None,
        },
    ];
    let plan = CompactionSourcePlan::new(source.clone(), entries.clone(), nodes.clone(), &contents)
        .unwrap();
    plan.validate(&contents).unwrap();
    let mut changed = contents.clone();
    changed[1].push('x');
    assert!(plan.validate(&changed).is_err());
    let mut missing = nodes.clone();
    if let CompactionSourceNodeInput::Source(ranges) = &mut missing[1].input {
        ranges[0].start = split + 1;
    }
    assert!(
        CompactionSourcePlan::new(source.clone(), entries.clone(), missing, &contents).is_err()
    );
    let mut split_utf8 = nodes.clone();
    if let CompactionSourceNodeInput::Source(ranges) = &mut split_utf8[0].input {
        ranges[0].end = chinese + 1;
    }
    assert!(
        CompactionSourcePlan::new(source.clone(), entries.clone(), split_utf8, &contents).is_err()
    );
    let mut duplicate = nodes;
    if let CompactionSourceNodeInput::Merge(children) = &mut duplicate[2].input {
        children[1] = "left".into();
    }
    assert!(
        CompactionSourcePlan::new(
            source.clone(),
            entries.clone(),
            duplicate.clone(),
            &contents
        )
        .is_err()
    );
    if let CompactionSourceNodeInput::Merge(children) = &mut duplicate[2].input {
        *children = vec!["right".into(), "left".into()];
    }
    assert!(
        CompactionSourcePlan::new(
            source.clone(),
            entries.clone(),
            duplicate.clone(),
            &contents
        )
        .is_err()
    );
    duplicate[2].is_root = false;
    assert!(CompactionSourcePlan::new(source, entries, duplicate, &contents).is_err());
}

#[test]
fn accepted_input_binds_exact_durable_history_and_rejects_tampering() {
    let input = ContentItem::user_text("original task");
    let events = vec![
        envelope(
            SessionEvent::Started {
                primary_provider_session: ProviderSession::new(
                    ProviderSessionId::new("input-test"),
                    ProviderSessionRole::Primary,
                    0,
                )
                .reference()
                .clone(),
            },
            1,
        ),
        envelope(
            SessionEvent::HistoryItemRecorded {
                item: input.clone(),
            },
            2,
        ),
    ];
    let reducer = SessionReducer;
    let mut state = reducer.reduce(events.clone()).unwrap();
    let provenance = BTreeMap::from([(
        0,
        ContextProvenance::operator_input("original task").unwrap(),
    )]);
    let boundary = state
        .accepted_operator_input(&[input], &provenance)
        .unwrap();
    assert!(
        matches!(&boundary, crate::domain::session::OperatorTurnBoundary::Accepted { anchor, .. } if anchor == &events[1].event_id)
    );
    let bound = boundary.input_provenance().unwrap();
    assert_eq!(
        bound[&0].references().source_event_refs(),
        &[events[1].event_id.clone()]
    );
    let accepted = envelope(SessionEvent::OperatorTurnBoundaryRecorded { boundary }, 3);
    reducer.apply(&mut state, accepted.clone()).unwrap();
    assert!(
        state
            .accepted_operator_input(&[ContentItem::user_text("original task")], &provenance)
            .is_err()
    );
    let mut tampered = accepted;
    tampered.payload["inputs"][0]["content_hash"] = serde_json::json!("forged");
    assert!(
        reducer
            .reduce(events.into_iter().chain([tampered]))
            .is_err()
    );
}
