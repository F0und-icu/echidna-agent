use cyber_agent::domain::artifacts::ArtifactId;
use cyber_agent::domain::events::{
    EventEnvelope, EventId, EventSequence, EventType, OccurredAt, StreamId,
};
use cyber_agent::domain::research::{
    CapsuleId, EvidenceRef, ExecutionReceiptId, ExperimentCapsuleDraft, ExperimentDraft,
    ExperimentId, ExperimentRecord, ExperimentResult, FindingDraft, FindingId, FindingRecord,
    FindingStatus, FindingUpdate, HypothesisDraft, HypothesisId, HypothesisRecord,
    HypothesisStatus, NextActionId, NextActionUpdate, ResearchError, ResearchEvent,
    ResearchReducer, TargetKind, TargetSnapshotDraft, TargetSnapshotId, TargetSnapshotRecord,
};
use cyber_agent::domain::tools::ExecutionId;
use serde_json::{Value, json};

fn target_snapshot(id: &str, captured_at: &str, notes: &str) -> TargetSnapshotRecord {
    TargetSnapshotRecord::from_draft(TargetSnapshotDraft {
        id: TargetSnapshotId::new(id),
        target_id: "fixture.target".to_owned(),
        kind: TargetKind::Source,
        locator: "src/fixture".to_owned(),
        version: "1.0.0".to_owned(),
        build_id: "build-1".to_owned(),
        target_artifact_ids: Vec::new(),
        environment_fingerprint: "environment-a".to_owned(),
        device_fingerprint: "unknown".to_owned(),
        captured_at: captured_at.to_owned(),
        notes: notes.to_owned(),
    })
    .unwrap()
}

fn action_fingerprint(action: &str, arguments: Value) -> String {
    action_fingerprint_with_capsule(
        action,
        arguments,
        "toolchain",
        Vec::new(),
        "execution-a",
        "receipt-a",
        "output-a",
    )
}

fn action_fingerprint_with_capsule(
    action: &str,
    arguments: Value,
    toolchain_fingerprint: &str,
    account_prerequisites: Vec<&str>,
    execution_id: &str,
    receipt_id: &str,
    output_artifact_id: &str,
) -> String {
    let (experiment, _) = ExperimentRecord::from_drafts(
        ExperimentDraft {
            id: ExperimentId::new("experiment"),
            hypothesis_id: HypothesisId::new("hypothesis"),
            target_snapshot_id: TargetSnapshotId::new("snapshot"),
            capsule_id: CapsuleId::new("capsule"),
            goal: "fixture goal".to_owned(),
            result: ExperimentResult::Pending,
            artifact_refs: Vec::new(),
            notes: String::new(),
        },
        ExperimentCapsuleDraft {
            id: CapsuleId::new("capsule"),
            experiment_id: ExperimentId::new("experiment"),
            target_snapshot_id: TargetSnapshotId::new("snapshot"),
            action: action.to_owned(),
            arguments,
            tool_version: "tool-v1".to_owned(),
            execution_ids: vec![ExecutionId::new(execution_id)],
            input_artifact_ids: Vec::new(),
            environment_delta: "locale=C".to_owned(),
            account_prerequisites: account_prerequisites
                .into_iter()
                .map(str::to_owned)
                .collect(),
            toolchain_fingerprint: toolchain_fingerprint.to_owned(),
            execution_receipt_ids: vec![ExecutionReceiptId::new(receipt_id)],
            output_artifact_ids: vec![ArtifactId::new(output_artifact_id)],
        },
    )
    .unwrap();
    experiment.action_fingerprint
}

#[test]
fn snapshot_excludes_capture_metadata_and_action_canonicalizes_nested_object_keys_only() {
    let first = target_snapshot("snapshot-1", "2026-08-22T00:00:00Z", "first note");
    let second = target_snapshot("snapshot-2", "2026-08-22T01:00:00Z", "second note");
    assert_eq!(first.snapshot_fingerprint, second.snapshot_fingerprint);

    let first_arguments = serde_json::from_str(
        r#"{"nested":{"z":"PayloadCase","a":[{"right":2,"left":1}]},"flag":true}"#,
    )
    .unwrap();
    let second_arguments = serde_json::from_str(
        r#"{"flag":true,"nested":{"a":[{"left":1,"right":2}],"z":"PayloadCase"}}"#,
    )
    .unwrap();
    assert_eq!(
        action_fingerprint("Run Tool --CaseSensitive", first_arguments),
        action_fingerprint("Run Tool --CaseSensitive", second_arguments)
    );
    assert_ne!(
        action_fingerprint("Run Tool --CaseSensitive", json!({"arg": "PayloadCase"})),
        action_fingerprint("run tool --casesensitive", json!({"arg": "PayloadCase"}))
    );
}

#[test]
fn action_fingerprint_captures_toolchain_and_account_environment_not_results() {
    let base = action_fingerprint_with_capsule(
        "Run Fixture Tool",
        json!({"payload": "same"}),
        "toolchain-v1",
        vec!["authenticated", "entitled"],
        "execution-a",
        "receipt-a",
        "output-a",
    );
    let reordered_accounts = action_fingerprint_with_capsule(
        "Run Fixture Tool",
        json!({"payload": "same"}),
        "toolchain-v1",
        vec!["entitled", "authenticated"],
        "execution-a",
        "receipt-a",
        "output-a",
    );
    let changed_toolchain = action_fingerprint_with_capsule(
        "Run Fixture Tool",
        json!({"payload": "same"}),
        "toolchain-v2",
        vec!["authenticated", "entitled"],
        "execution-a",
        "receipt-a",
        "output-a",
    );
    let changed_account_environment = action_fingerprint_with_capsule(
        "Run Fixture Tool",
        json!({"payload": "same"}),
        "toolchain-v1",
        vec!["authenticated", "different-entitlement"],
        "execution-a",
        "receipt-a",
        "output-a",
    );
    let changed_execution_results = action_fingerprint_with_capsule(
        "Run Fixture Tool",
        json!({"payload": "same"}),
        "toolchain-v1",
        vec!["authenticated", "entitled"],
        "execution-b",
        "receipt-b",
        "output-b",
    );

    assert_eq!(base, reordered_accounts);
    assert_ne!(base, changed_toolchain);
    assert_ne!(base, changed_account_environment);
    assert_eq!(base, changed_execution_results);
}

fn candidate_finding() -> FindingRecord {
    FindingRecord::from_draft(FindingDraft {
        id: FindingId::new("finding-candidate"),
        title: "Candidate finding".to_owned(),
        status: FindingStatus::Candidate,
        target_id: "fixture.target".to_owned(),
        target_version: "1.0.0".to_owned(),
        vulnerability_class: "intent redirection".to_owned(),
        entry_point: "fixture.entry".to_owned(),
        sensitive_sink: "fixture.sink".to_owned(),
        target_snapshot_ids: Vec::new(),
        prerequisites: Vec::new(),
        impact: "candidate impact".to_owned(),
        evidence_refs: Vec::new(),
        negative_refs: Vec::new(),
    })
    .unwrap()
}

fn hypothesis_with_evidence(evidence_refs: Vec<EvidenceRef>) -> HypothesisRecord {
    HypothesisRecord::from_draft(HypothesisDraft {
        id: HypothesisId::new("hypothesis-evidence"),
        title: "Evidence-bearing hypothesis".to_owned(),
        target_id: "fixture.target".to_owned(),
        attack_surface: "fixture.surface".to_owned(),
        vulnerability_class: "intent redirection".to_owned(),
        entry_point: "fixture.entry".to_owned(),
        sensitive_sink: "fixture.sink".to_owned(),
        rationale: "fixture rationale".to_owned(),
        status: HypothesisStatus::Active,
        priority: 1,
        evidence_refs,
        last_action: None,
        next_action: None,
    })
    .unwrap()
}

#[test]
fn initial_hypothesis_rejects_missing_experiment_evidence_without_mutation() {
    let reducer = ResearchReducer;
    let mut state = Default::default();
    let before = serde_json::to_value(&state).unwrap();

    let result = reducer.apply_event(
        &mut state,
        ResearchEvent::HypothesisRecorded {
            record: hypothesis_with_evidence(vec![EvidenceRef::Experiment(ExperimentId::new(
                "experiment-missing",
            ))]),
            keep_independent: false,
        },
    );

    assert!(matches!(
        result,
        Err(ResearchError::MissingReference { .. })
    ));
    assert_eq!(serde_json::to_value(&state).unwrap(), before);
}

#[test]
fn reducer_update_errors_leave_serialized_state_unchanged() {
    let reducer = ResearchReducer;
    let mut state = Default::default();
    reducer
        .apply_event(
            &mut state,
            ResearchEvent::FindingRecorded {
                record: candidate_finding(),
                keep_independent: false,
            },
        )
        .unwrap();
    let before = serde_json::to_value(&state).unwrap();

    assert!(
        reducer
            .apply_event(
                &mut state,
                ResearchEvent::FindingUpdated(FindingUpdate {
                    id: FindingId::new("finding-candidate"),
                    status: Some(FindingStatus::Confirmed),
                    target_snapshot_ids_add: Vec::new(),
                    prerequisites_add: Vec::new(),
                    evidence_refs_add: Vec::new(),
                    negative_refs_add: Vec::new(),
                    impact: None,
                }),
            )
            .is_err()
    );
    assert_eq!(serde_json::to_value(&state).unwrap(), before);

    assert!(
        reducer
            .apply_event(
                &mut state,
                ResearchEvent::NextActionUpdated(NextActionUpdate {
                    id: NextActionId::new("missing-action"),
                    priority: None,
                    action: None,
                    status: None,
                }),
            )
            .is_err()
    );
    assert_eq!(serde_json::to_value(&state).unwrap(), before);
}

fn target_envelope(
    event_id: &str,
    sequence: u64,
    occurred_at: &str,
    snapshot_id: &str,
) -> EventEnvelope<Value> {
    let event = ResearchEvent::TargetSnapshotRecorded(target_snapshot(
        snapshot_id,
        "2026-08-22T00:00:00Z",
        "note",
    ));
    EventEnvelope::current(
        EventId::new(event_id),
        StreamId::new("research"),
        EventSequence::new(sequence),
        OccurredAt::new(occurred_at),
        EventType::new(event.event_type().as_str()),
        event.encode_payload().unwrap(),
    )
}

#[test]
fn reducer_ignores_only_an_exact_valid_duplicate_event_id() {
    let first = target_envelope("event-duplicate", 1, "2026-08-22T00:00:00Z", "snapshot-1");
    let duplicate = target_envelope("event-duplicate", 2, "2026-08-22T00:00:00Z", "snapshot-1");

    let state = ResearchReducer.reduce([first, duplicate]).unwrap();
    assert_eq!(state.applied_event_count(), 1);
    assert_eq!(
        serde_json::to_value(state).unwrap()["target_snapshots"]
            .as_object()
            .unwrap()
            .len(),
        1
    );
}

fn assert_duplicate_event_is_rejected(second: EventEnvelope<Value>) {
    let reducer = ResearchReducer;
    let mut state = reducer
        .reduce([target_envelope(
            "event-duplicate",
            1,
            "2026-08-22T00:00:00Z",
            "snapshot-1",
        )])
        .unwrap();
    let before = serde_json::to_value(&state).unwrap();
    assert!(reducer.apply(&mut state, second).is_err());
    assert_eq!(state.applied_event_count(), 1);
    assert_eq!(serde_json::to_value(&state).unwrap(), before);
}

#[test]
fn duplicate_event_ids_revalidate_structure_and_semantics_before_idempotence() {
    assert_duplicate_event_is_rejected(EventEnvelope::current(
        EventId::new("event-duplicate"),
        StreamId::new("session:other"),
        EventSequence::new(2),
        OccurredAt::new("2026-08-22T00:00:00Z"),
        EventType::new("research.target_snapshot_recorded"),
        target_envelope("other", 1, "2026-08-22T00:00:00Z", "snapshot-1").payload,
    ));
    assert_duplicate_event_is_rejected(EventEnvelope::current(
        EventId::new("event-duplicate"),
        StreamId::new("research"),
        EventSequence::new(2),
        OccurredAt::new("2026-08-22T00:00:00Z"),
        EventType::new("research.future_unknown"),
        json!({"research_schema_version": 1, "data": {}}),
    ));
    assert_duplicate_event_is_rejected(EventEnvelope::current(
        EventId::new("event-duplicate"),
        StreamId::new("research"),
        EventSequence::new(2),
        OccurredAt::new("2026-08-22T00:00:00Z"),
        EventType::new("research.target_snapshot_recorded"),
        json!({"wrong": true}),
    ));
    assert_duplicate_event_is_rejected(EventEnvelope::current(
        EventId::new("event-duplicate"),
        StreamId::new("research"),
        EventSequence::new(2),
        OccurredAt::new("2026-08-22T00:00:00Z"),
        EventType::new("research.target_snapshot_recorded"),
        json!({"research_schema_version": 2, "data": {}}),
    ));
    assert_duplicate_event_is_rejected(EventEnvelope::current(
        EventId::new("event-duplicate"),
        StreamId::new("research"),
        EventSequence::new(2),
        OccurredAt::new("2026-08-22T00:00:00Z"),
        EventType::new("StreamRecovered"),
        json!({
            "recovery_artifact": "",
            "truncated_from_offset": 42,
            "resumed_after_sequence": 1
        }),
    ));

    let reducer = ResearchReducer;
    let mut state = reducer
        .reduce([target_envelope(
            "event-duplicate",
            1,
            "2026-08-22T00:00:00Z",
            "snapshot-1",
        )])
        .unwrap();
    let before = serde_json::to_value(&state).unwrap();
    let changed_payload = reducer.apply(
        &mut state,
        target_envelope("event-duplicate", 2, "2026-08-22T00:00:00Z", "snapshot-2"),
    );
    assert!(matches!(
        changed_payload,
        Err(ResearchError::EventIdConflict { .. })
    ));
    assert_eq!(serde_json::to_value(&state).unwrap(), before);

    let changed_event_type = reducer.apply(
        &mut state,
        EventEnvelope::current(
            EventId::new("event-duplicate"),
            StreamId::new("research"),
            EventSequence::new(2),
            OccurredAt::new("2026-08-22T00:00:00Z"),
            EventType::new("StreamRecovered"),
            json!({
                "recovery_artifact": "artifacts/recovery/tail.bin",
                "truncated_from_offset": 42,
                "resumed_after_sequence": 1
            }),
        ),
    );
    assert!(matches!(
        changed_event_type,
        Err(ResearchError::EventIdConflict { .. })
    ));
    assert_eq!(serde_json::to_value(&state).unwrap(), before);

    let changed_occurrence = reducer.apply(
        &mut state,
        target_envelope("event-duplicate", 2, "2026-08-22T00:01:00Z", "snapshot-1"),
    );
    assert!(matches!(
        changed_occurrence,
        Err(ResearchError::EventIdConflict { .. })
    ));
    assert_eq!(serde_json::to_value(&state).unwrap(), before);
}

#[test]
fn reducer_accepts_the_existing_strict_stream_recovery_control_event() {
    let record = target_snapshot("snapshot-1", "2026-08-22T00:00:00Z", "note");
    let event = ResearchEvent::TargetSnapshotRecorded(record);
    let state = ResearchReducer
        .reduce([
            EventEnvelope::current(
                EventId::new("event-recovered"),
                StreamId::new("research"),
                EventSequence::new(1),
                OccurredAt::new("2026-08-22T00:00:00Z"),
                EventType::new("StreamRecovered"),
                json!({
                    "recovery_artifact": "artifacts/recovery/tail.bin",
                    "truncated_from_offset": 42,
                    "resumed_after_sequence": null
                }),
            ),
            EventEnvelope::current(
                EventId::new("event-target"),
                StreamId::new("research"),
                EventSequence::new(2),
                OccurredAt::new("2026-08-22T00:01:00Z"),
                EventType::new(event.event_type().as_str()),
                event.encode_payload().unwrap(),
            ),
        ])
        .unwrap();
    assert_eq!(state.applied_event_count(), 2);
    assert_eq!(
        serde_json::to_value(state).unwrap()["target_snapshots"]
            .as_object()
            .unwrap()
            .len(),
        1
    );
}
