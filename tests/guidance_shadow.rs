mod support;

use std::fs;
use std::path::Path;
use std::process::Command;

use echidna::adapters::project_event_store::{Project, ProjectEventStream};
use echidna::domain::events::{EventEnvelope, EventId, EventType, OccurredAt};
use echidna::domain::research::{
    CapsuleId, ExperimentCapsuleDraft, ExperimentDraft, ExperimentId, ExperimentRecord,
    ExperimentResult, FindingDraft, FindingId, FindingRecord, FindingStatus, HypothesisDraft,
    HypothesisId, HypothesisRecord, HypothesisStatus, NextActionDraft, NextActionId,
    NextActionRecord, NextActionStatus, ResearchEvent, ResearchRecordRef, TargetKind,
    TargetSnapshotDraft, TargetSnapshotId, TargetSnapshotRecord,
};
use echidna::ports::EventSink;
use serde_json::{Value, json};

use support::TemporaryDirectory;

fn initialized_project(label: &str) -> (TemporaryDirectory, Project) {
    let root = TemporaryDirectory::new(label);
    let project = Project::initialize(root.path()).expect("project should initialize");
    (root, project)
}

fn append_research(project: &Project, event_id: &str, event: ResearchEvent) {
    let mut writer = project.open_writer().expect("writer should open");
    let mut store = writer
        .open_research(None)
        .expect("Research store should open");
    let envelope = EventEnvelope::current(
        EventId::new(event_id),
        ProjectEventStream::Research.stream_id(),
        store.next_sequence().expect("sequence should resolve"),
        OccurredAt::new("2026-08-31T00:00:00Z"),
        EventType::new(event.event_type().as_str()),
        event
            .encode_payload()
            .expect("Research Event should encode"),
    );
    EventSink::append_event(&mut store, &envelope).expect("Research Event should append");
}

fn target(project: &Project, id: &str, version: &str) {
    append_research(
        project,
        &format!("event-{id}"),
        ResearchEvent::TargetSnapshotRecorded(
            TargetSnapshotRecord::from_draft(TargetSnapshotDraft {
                id: TargetSnapshotId::new(id),
                target_id: "fixture.target".to_owned(),
                kind: TargetKind::Mobile,
                locator: "package:fixture.target".to_owned(),
                version: version.to_owned(),
                build_id: format!("build-{version}"),
                target_artifact_ids: Vec::new(),
                environment_fingerprint: "fixture-environment".to_owned(),
                device_fingerprint: "fixture-device".to_owned(),
                captured_at: "2026-08-31T00:00:00Z".to_owned(),
                notes: "guidance fixture target".to_owned(),
            })
            .expect("TargetSnapshot should be valid"),
        ),
    );
}

fn finding(project: &Project) {
    append_research(
        project,
        "event-finding-current",
        ResearchEvent::FindingRecorded {
            record: FindingRecord::from_draft(FindingDraft {
                id: FindingId::new("finding-current"),
                title: "Controlled guidance finding".to_owned(),
                status: FindingStatus::Candidate,
                target_id: "fixture.target".to_owned(),
                target_version: "2.0.0".to_owned(),
                vulnerability_class: "intent redirection".to_owned(),
                entry_point: "fixture.target/.Entry".to_owned(),
                sensitive_sink: "sendBroadcast".to_owned(),
                target_snapshot_ids: vec![TargetSnapshotId::new("snapshot-current")],
                prerequisites: Vec::new(),
                impact: "cross-app launch".to_owned(),
                evidence_links: Vec::new(),
            })
            .expect("Finding should be valid"),
            keep_independent: false,
        },
    );
}

fn finding_without_snapshot_binding(project: &Project) {
    append_research(
        project,
        "event-finding-unknown",
        ResearchEvent::FindingRecorded {
            record: FindingRecord::from_draft(FindingDraft {
                id: FindingId::new("finding-unknown"),
                title: "Controlled guidance unbound finding".to_owned(),
                status: FindingStatus::Candidate,
                target_id: "fixture.target".to_owned(),
                target_version: "2.0.0".to_owned(),
                vulnerability_class: "intent redirection".to_owned(),
                entry_point: "fixture.target/.Entry".to_owned(),
                sensitive_sink: "startActivity".to_owned(),
                target_snapshot_ids: Vec::new(),
                prerequisites: Vec::new(),
                impact: "cross-app launch".to_owned(),
                evidence_links: Vec::new(),
            })
            .expect("Finding should be valid"),
            keep_independent: false,
        },
    );
}

fn next_action(project: &Project, id: &str, status: NextActionStatus, priority: i32) {
    append_research(
        project,
        &format!("event-{id}"),
        ResearchEvent::NextActionRecorded(
            NextActionRecord::from_draft(NextActionDraft {
                id: NextActionId::new(id),
                research_id: ResearchRecordRef::Finding(FindingId::new("finding-current")),
                priority,
                action: format!("untrusted prose for {id}"),
                status,
            })
            .expect("NextAction should be valid"),
        ),
    );
}

fn experiment_matching_candidate_action(project: &Project, action_seed: &str) {
    let hypothesis_id = HypothesisId::new(format!("hypothesis-{action_seed}"));
    append_research(
        project,
        &format!("event-{}", hypothesis_id.as_str()),
        ResearchEvent::HypothesisRecorded {
            record: HypothesisRecord::from_draft(HypothesisDraft {
                id: hypothesis_id.clone(),
                title: "Controlled guidance hypothesis".to_owned(),
                target_id: "fixture.target".to_owned(),
                attack_surface: "intent entry".to_owned(),
                vulnerability_class: "intent redirection".to_owned(),
                entry_point: "fixture.target/.Entry".to_owned(),
                sensitive_sink: "startActivity".to_owned(),
                rationale: "controlled action identity".to_owned(),
                status: HypothesisStatus::Active,
                priority: 0,
                evidence_links: Vec::new(),
                last_action: None,
                next_action: None,
            })
            .expect("Hypothesis should be valid"),
            keep_independent: false,
        },
    );
    let experiment_id = ExperimentId::new(format!("experiment-{action_seed}"));
    let capsule_id = CapsuleId::new(format!("capsule-{action_seed}"));
    let (experiment, capsule) = ExperimentRecord::from_drafts(
        ExperimentDraft {
            id: experiment_id.clone(),
            hypothesis_id,
            target_snapshot_id: TargetSnapshotId::new("snapshot-current"),
            capsule_id: capsule_id.clone(),
            goal: "Controlled exact action identity".to_owned(),
            result: ExperimentResult::Pending,
            artifact_refs: Vec::new(),
            notes: String::new(),
        },
        ExperimentCapsuleDraft {
            id: capsule_id,
            experiment_id,
            target_snapshot_id: TargetSnapshotId::new("snapshot-current"),
            action: format!("Run Fixture --{action_seed}"),
            arguments: Value::Null,
            tool_version: "v".to_owned(),
            execution_ids: Vec::new(),
            input_artifact_ids: Vec::new(),
            environment_delta: String::new(),
            account_prerequisites: Vec::new(),
            toolchain_fingerprint: "t".to_owned(),
            execution_receipt_ids: Vec::new(),
            output_artifact_ids: Vec::new(),
        },
    )
    .expect("Experiment should be valid");
    append_research(
        project,
        &format!("event-{}", experiment.id.as_str()),
        ResearchEvent::ExperimentRecorded {
            experiment,
            capsule: Box::new(capsule),
            rerun_reason: None,
        },
    );
}

fn candidate(id: &str, action_seed: &str, closures: &[&str]) -> Value {
    json!({
        "id": id,
        "research_id": {"kind": "finding", "id": "finding-current"},
        "target_snapshot_id": "snapshot-current",
        "action_identity": {
            "hypothesis_id": format!("hypothesis-{action_seed}"),
            "normalized_action": format!("Run Fixture --{action_seed}"),
            "tool_version": "v",
            "toolchain_fingerprint": "t",
        },
        "validity": "valid",
        "prerequisite": "satisfied",
        "boundary_importance": 70,
        "competing_hypothesis_discrimination": 60,
        "expected_gap_closures": closures,
        "reproducibility": 80,
        "execution_cost": 50,
        "mutation_risk": 20,
        "version_drift_risk": 10,
    })
}

fn tree_contents(root: &Path) -> Vec<(String, Vec<u8>)> {
    fn visit(root: &Path, current: &Path, contents: &mut Vec<(String, Vec<u8>)>) {
        let mut entries = fs::read_dir(current)
            .unwrap()
            .map(|entry| entry.unwrap())
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let relative = path.strip_prefix(root).unwrap().display().to_string();
            if entry.file_type().unwrap().is_dir() {
                contents.push((format!("dir:{relative}"), Vec::new()));
                visit(root, &path, contents);
            } else {
                contents.push((format!("file:{relative}"), fs::read(path).unwrap()));
            }
        }
    }

    let mut contents = Vec::new();
    visit(root, root, &mut contents);
    contents
}

fn write_candidates(root: &TemporaryDirectory, name: &str, candidates: Vec<Value>) -> String {
    let path = root.path().join(name);
    fs::write(
        &path,
        serde_json::to_vec(&json!({"candidates": candidates})).unwrap(),
    )
    .unwrap();
    path.display().to_string()
}

fn command(candidate_path: &str) -> Vec<String> {
    vec![
        "inspect".to_owned(),
        "guidance".to_owned(),
        "--target-snapshot".to_owned(),
        "snapshot-current".to_owned(),
        "--eval-candidates".to_owned(),
        candidate_path.to_owned(),
    ]
}

fn command_without_candidates() -> Vec<String> {
    vec![
        "inspect".to_owned(),
        "guidance".to_owned(),
        "--target-snapshot".to_owned(),
        "snapshot-current".to_owned(),
    ]
}

fn run(root: &TemporaryDirectory, args: &[String]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_echidna"))
        .current_dir(root.path())
        .args(args)
        .output()
        .expect("guidance command should run")
}

fn report_candidate<'a>(report: &'a Value, source_id: &str) -> &'a Value {
    report["report"]["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|candidate| candidate["source_id"] == source_id)
        .unwrap_or_else(|| panic!("candidate `{source_id}` should be reported"))
}

fn factor<'a>(candidate: &'a Value, name: &str) -> &'a Value {
    candidate["factors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|factor| factor["name"] == name)
        .unwrap_or_else(|| panic!("factor `{name}` should be reported"))
}

#[test]
fn guidance_shadow_is_byte_stable_read_only_and_ranks_typed_critical_gap_closure_first() {
    let (root, project) = initialized_project("guidance-shadow");
    target(&project, "snapshot-old", "1.0.0");
    target(&project, "snapshot-current", "2.0.0");
    finding(&project);
    finding_without_snapshot_binding(&project);
    experiment_matching_candidate_action(&project, "prior");
    next_action(&project, "next-pending", NextActionStatus::Pending, 999);
    next_action(&project, "next-priority-a", NextActionStatus::Pending, -999);
    next_action(&project, "next-priority-z", NextActionStatus::Pending, 999);
    next_action(&project, "next-done", NextActionStatus::Done, -999);
    next_action(&project, "next-blocked", NextActionStatus::Blocked, 999);
    next_action(&project, "next-running", NextActionStatus::Running, 999);

    let mut high_closure = candidate(
        "closure-high",
        "MIXED",
        &["entry_unproven", "sink_unproven"],
    );
    high_closure["execution_cost"] = json!(95);
    let mut low_cost = candidate("cost-low", "cost-low", &["entry_unproven"]);
    low_cost["execution_cost"] = json!(1);
    let tie_alpha = candidate("tie-alpha", "tie-alpha", &["positive_control_missing"]);
    let tie_beta = candidate("tie-beta", "tie-beta", &["positive_control_missing"]);
    let mut missing_prerequisite = candidate("prerequisite-missing", "prerequisite-missing", &[]);
    missing_prerequisite["prerequisite"] = json!("missing");
    let mut unknown_prerequisite = candidate("prerequisite-unknown", "prerequisite-unknown", &[]);
    unknown_prerequisite["prerequisite"] = json!("unknown");
    let mut invalid = candidate("explicit-invalid", "invalid", &[]);
    invalid["validity"] = json!("invalid");
    let mut cross_snapshot = candidate("cross-snapshot", "cross-snapshot", &[]);
    cross_snapshot["target_snapshot_id"] = json!("snapshot-old");
    let mut missing_reference = candidate("missing-reference", "missing-reference", &[]);
    missing_reference["research_id"] = json!({"kind": "finding", "id": "finding-missing"});
    let mut unknown_reference = candidate("unknown-reference", "unknown-reference", &[]);
    unknown_reference["research_id"] = json!({"kind": "finding", "id": "finding-unknown"});
    let unknown_gap = candidate("unknown-gap", "unknown-gap", &["prerequisite_unproven"]);
    let duplicate_alpha = candidate("duplicate-alpha", "duplicate", &[]);
    let duplicate_beta = candidate("duplicate-beta", "duplicate", &[]);
    let prior_action = candidate("prior-action", "prior", &[]);
    let duplicate_gap = candidate(
        "duplicate-gap",
        "duplicate-gap",
        &["entry_unproven", "entry_unproven"],
    );
    let mut mutation_low = candidate("mutation-low", "mutation-low", &[]);
    mutation_low["mutation_risk"] = json!(1);
    mutation_low["version_drift_risk"] = json!(50);
    let mut mutation_high = candidate("mutation-high", "mutation-high", &[]);
    mutation_high["mutation_risk"] = json!(99);
    mutation_high["version_drift_risk"] = json!(50);
    let mut drift_low = candidate("drift-low", "drift-low", &[]);
    drift_low["mutation_risk"] = json!(50);
    drift_low["version_drift_risk"] = json!(1);
    let mut drift_high = candidate("drift-high", "drift-high", &[]);
    drift_high["mutation_risk"] = json!(50);
    drift_high["version_drift_risk"] = json!(99);
    let candidates = vec![
        tie_beta,
        low_cost,
        missing_prerequisite,
        duplicate_alpha,
        high_closure,
        invalid,
        unknown_gap,
        tie_alpha,
        cross_snapshot,
        missing_reference,
        duplicate_beta,
        prior_action,
        unknown_prerequisite,
        unknown_reference,
        duplicate_gap,
        mutation_high,
        mutation_low,
        drift_high,
        drift_low,
    ];
    let candidate_path = write_candidates(&root, "guidance-candidates.json", candidates.clone());
    let reversed_path = write_candidates(
        &root,
        "guidance-candidates-reversed.json",
        candidates.clone().into_iter().rev().collect(),
    );
    let research_path = root.path().join(".cyber-agent/research.jsonl");
    let research_before = fs::read(&research_path).unwrap();
    let tree_before = tree_contents(&root.path().join(".cyber-agent"));

    let first = run(&root, &command(&candidate_path));
    assert!(first.status.success(), "stderr: {:?}", first.stderr);
    let second = run(&root, &command(&candidate_path));
    assert!(second.status.success(), "stderr: {:?}", second.stderr);
    assert_eq!(
        first.stdout, second.stdout,
        "same input must be byte-stable"
    );
    assert_eq!(fs::read(&research_path).unwrap(), research_before);
    assert_eq!(
        tree_contents(&root.path().join(".cyber-agent")),
        tree_before,
        "guidance must not change any .cyber-agent file or file list"
    );
    let reordered = run(&root, &command(&reversed_path));
    assert!(reordered.status.success(), "stderr: {:?}", reordered.stderr);
    assert_eq!(
        first.stdout, reordered.stdout,
        "input order must not change report bytes"
    );

    let report: Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(report["shadow"], json!(true));
    assert_eq!(report["model_context_changed"], json!(false));
    for value in report["effects"].as_object().unwrap().values() {
        assert_eq!(value, &json!(0));
    }
    assert_eq!(
        report["report"]["deferred_metrics"]["actual_critical_gap_closure"]["state"],
        json!("unavailable")
    );
    assert_eq!(
        report["report"]["deferred_metrics"]["cost_effectiveness"]["state"],
        json!("unavailable")
    );

    let closure_high = report_candidate(&report, "closure-high");
    let cost_low = report_candidate(&report, "cost-low");
    let computed_fingerprint = closure_high["action_fingerprint"]["value"]
        .as_str()
        .unwrap();
    assert!(computed_fingerprint.starts_with("action-"));
    assert_eq!(computed_fingerprint.len(), "action-".len() + 64);
    assert!(
        computed_fingerprint["action-".len()..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte.is_ascii_lowercase()),
        "computed action hash must use canonical lowercase hex"
    );
    assert_eq!(
        closure_high["action_fingerprint"]["reason_codes"],
        json!(["computed_controlled_action_fingerprint"])
    );
    assert!(closure_high["rank"].as_u64() < cost_low["rank"].as_u64());
    assert_eq!(
        factor(closure_high, "expected_gap_closure")["state"],
        json!("available")
    );
    assert!(
        factor(closure_high, "expected_gap_closure")["value"].as_u64()
            > factor(cost_low, "expected_gap_closure")["value"].as_u64()
    );

    let tie_alpha = report_candidate(&report, "tie-alpha");
    let tie_beta = report_candidate(&report, "tie-beta");
    assert!(
        tie_alpha["candidate_identity"]
            .as_str()
            .unwrap()
            .starts_with("guidance-candidate-")
    );
    assert!(
        (tie_alpha["candidate_identity"].as_str() < tie_beta["candidate_identity"].as_str())
            == (tie_alpha["rank"].as_u64() < tie_beta["rank"].as_u64())
    );
    assert_eq!(
        tie_alpha["tie_break"],
        json!("deterministic_candidate_identity_tie_break")
    );

    let unknown_prerequisite = report_candidate(&report, "prerequisite-unknown");
    assert_eq!(
        unknown_prerequisite["prerequisite"]["state"],
        json!("unknown")
    );
    assert_eq!(unknown_prerequisite["exclusion_reasons"], json!([]));
    let missing_prerequisite = report_candidate(&report, "prerequisite-missing");
    assert!(
        missing_prerequisite["exclusion_reasons"]
            .as_array()
            .unwrap()
            .contains(&json!("candidate_prerequisite_missing"))
    );
    assert_eq!(missing_prerequisite["rank"], Value::Null);

    let unknown_gap = report_candidate(&report, "unknown-gap");
    assert_eq!(
        factor(unknown_gap, "expected_gap_closure")["state"],
        json!("unknown")
    );
    assert!(
        unknown_gap["associated_gaps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|gap| gap["kind"] == "prerequisite_unproven" && gap["state"] == "unknown")
    );
    let unknown_reference = report_candidate(&report, "unknown-reference");
    assert_eq!(unknown_reference["exclusion_reasons"], json!([]));
    assert_eq!(
        factor(unknown_reference, "target_snapshot_applicability")["state"],
        json!("unknown")
    );

    for (identity, reason) in [
        ("explicit-invalid", "candidate_explicitly_invalid"),
        (
            "cross-snapshot",
            "candidate_target_snapshot_does_not_match_current",
        ),
        ("missing-reference", "existing_reference_missing"),
        ("duplicate-alpha", "duplicate_candidate_action_fingerprint"),
        ("duplicate-beta", "duplicate_candidate_action_fingerprint"),
        ("duplicate-gap", "duplicate_declared_critical_gap"),
        ("prior-action", "prior_action_fingerprint_exists"),
        ("next-done", "next_action_lifecycle_done"),
        ("next-blocked", "next_action_lifecycle_blocked"),
        ("next-running", "next_action_lifecycle_running"),
    ] {
        assert!(
            report_candidate(&report, identity)["exclusion_reasons"]
                .as_array()
                .unwrap()
                .contains(&json!(reason))
        );
    }
    let pending = report_candidate(&report, "next-pending");
    assert_eq!(pending["source_priority"], json!(999));
    assert_eq!(
        factor(pending, "boundary_importance")["state"],
        json!("unavailable")
    );
    assert_eq!(pending["action_fingerprint"]["state"], json!("unavailable"));

    assert!(
        report_candidate(&report, "mutation-low")["rank"]
            .as_u64()
            .unwrap()
            < report_candidate(&report, "mutation-high")["rank"]
                .as_u64()
                .unwrap()
    );
    assert!(
        report_candidate(&report, "drift-low")["rank"]
            .as_u64()
            .unwrap()
            < report_candidate(&report, "drift-high")["rank"]
                .as_u64()
                .unwrap()
    );
    assert!(
        report_candidate(&report, "next-priority-a")["rank"]
            .as_u64()
            .unwrap()
            < report_candidate(&report, "next-priority-z")["rank"]
                .as_u64()
                .unwrap(),
        "same incomplete inputs tie-break by identity rather than NextAction priority"
    );
}

#[test]
fn guidance_shadow_allows_an_empty_controlled_pool_without_effects() {
    let (root, project) = initialized_project("guidance-empty-pool");
    target(&project, "snapshot-current", "2.0.0");

    let result = run(&root, &command_without_candidates());
    assert!(result.status.success(), "stderr: {:?}", result.stderr);
    let report: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["report"]["considered_candidate_count"], json!(0));
    assert_eq!(report["report"]["ranked_candidate_count"], json!(0));
    for value in report["effects"].as_object().unwrap().values() {
        assert_eq!(value, &json!(0));
    }
}

#[test]
fn guidance_shadow_rejects_invalid_bounded_and_strict_controlled_input_through_its_own_cli_error() {
    let (root, project) = initialized_project("guidance-invalid-input");
    target(&project, "snapshot-current", "2.0.0");
    finding(&project);

    let mut invalid_factor = candidate("invalid-factor", "invalid-factor", &[]);
    invalid_factor["boundary_importance"] = json!(101);
    let invalid_factor_path = write_candidates(&root, "invalid-factor.json", vec![invalid_factor]);
    let invalid_factor_result = run(&root, &command(&invalid_factor_path));
    assert!(!invalid_factor_result.status.success());
    let stderr = String::from_utf8(invalid_factor_result.stderr).unwrap();
    assert!(stderr.contains("evidence-gap guidance Shadow"));
    assert!(stderr.contains("exceeds the 0..=100 bound"));
    assert!(!stderr.contains("retrieval Shadow"));

    let duplicate_id_path = write_candidates(
        &root,
        "duplicate-id.json",
        vec![
            candidate("same-id", "duplicate-a", &[]),
            candidate("same-id", "duplicate-b", &[]),
        ],
    );
    let duplicate_id_result = run(&root, &command(&duplicate_id_path));
    assert!(!duplicate_id_result.status.success());
    assert!(
        String::from_utf8(duplicate_id_result.stderr)
            .unwrap()
            .contains("controlled candidate ID `same-id` is duplicated")
    );

    let unknown_field_path = root.path().join("unknown-field.json");
    fs::write(
        &unknown_field_path,
        br#"{"candidates":[],"unexpected":true}"#,
    )
    .unwrap();
    let unknown_field_result = run(&root, &command(&unknown_field_path.display().to_string()));
    assert!(!unknown_field_result.status.success());
    assert!(
        String::from_utf8(unknown_field_result.stderr)
            .unwrap()
            .contains("unknown field `unexpected`")
    );

    let mut nested_unknown = candidate("nested-unknown", "nested-unknown", &[]);
    nested_unknown["research_id"]["unexpected"] = json!(true);
    let nested_unknown_path = write_candidates(&root, "nested-unknown.json", vec![nested_unknown]);
    let nested_unknown_result = run(&root, &command(&nested_unknown_path));
    assert!(!nested_unknown_result.status.success());
    assert!(
        String::from_utf8(nested_unknown_result.stderr)
            .unwrap()
            .contains("unknown field `unexpected`")
    );

    let mut witness_unknown = candidate("witness-unknown", "witness-unknown", &[]);
    witness_unknown["action_identity"]["unexpected"] = json!(true);
    let witness_unknown_path =
        write_candidates(&root, "witness-unknown.json", vec![witness_unknown]);
    let witness_unknown_result = run(&root, &command(&witness_unknown_path));
    assert!(!witness_unknown_result.status.success());
    assert!(
        String::from_utf8(witness_unknown_result.stderr)
            .unwrap()
            .contains("unknown field `unexpected`")
    );

    let mut arbitrary_fingerprint = candidate("arbitrary-fingerprint", "arbitrary", &[]);
    arbitrary_fingerprint["action_fingerprint"] = json!("ACTION-not-a-computed-fingerprint");
    let arbitrary_fingerprint_path = write_candidates(
        &root,
        "arbitrary-fingerprint.json",
        vec![arbitrary_fingerprint],
    );
    let arbitrary_fingerprint_result = run(&root, &command(&arbitrary_fingerprint_path));
    assert!(!arbitrary_fingerprint_result.status.success());
    assert!(
        String::from_utf8(arbitrary_fingerprint_result.stderr)
            .unwrap()
            .contains("unknown field `action_fingerprint`")
    );

    let too_long_candidate_id = "x".repeat(129);
    let candidate_bound_path = write_candidates(
        &root,
        "candidate-bound.json",
        vec![candidate(&too_long_candidate_id, "candidate-bound", &[])],
    );
    let candidate_bound_result = run(&root, &command(&candidate_bound_path));
    assert!(!candidate_bound_result.status.success());
    assert!(
        String::from_utf8(candidate_bound_result.stderr)
            .unwrap()
            .contains("candidate ID must be non-empty, at most 128 bytes")
    );

    let mut reference_bound = candidate("reference-bound", "reference-bound", &[]);
    reference_bound["target_snapshot_id"] = json!("s".repeat(129));
    let reference_bound_path =
        write_candidates(&root, "reference-bound.json", vec![reference_bound]);
    let reference_bound_result = run(&root, &command(&reference_bound_path));
    assert!(!reference_bound_result.status.success());
    assert!(
        String::from_utf8(reference_bound_result.stderr)
            .unwrap()
            .contains("references an identifier above the 128-byte bound")
    );

    let mut witness_string_bound = candidate("witness-string-bound", "witness-string", &[]);
    witness_string_bound["action_identity"]["tool_version"] = json!("t".repeat(4097));
    let witness_string_bound_path = write_candidates(
        &root,
        "witness-string-bound.json",
        vec![witness_string_bound],
    );
    let witness_string_bound_result = run(&root, &command(&witness_string_bound_path));
    assert!(!witness_string_bound_result.status.success());
    assert!(
        String::from_utf8(witness_string_bound_result.stderr)
            .unwrap()
            .contains(
                "action identity field `tool_version` must be non-empty and at most 4096 bytes"
            )
    );

    let mut witness_array_bound = candidate("witness-array-bound", "witness-array", &[]);
    witness_array_bound["action_identity"]["input_artifact_ids"] = Value::Array(
        (0..25)
            .map(|index| json!(format!("artifact-{index}")))
            .collect(),
    );
    let witness_array_bound_path =
        write_candidates(&root, "witness-array-bound.json", vec![witness_array_bound]);
    let witness_array_bound_result = run(&root, &command(&witness_array_bound_path));
    assert!(!witness_array_bound_result.status.success());
    assert!(
        String::from_utf8(witness_array_bound_result.stderr)
            .unwrap()
            .contains("action identity array `input_artifact_ids` has 25 entries")
    );

    let mut witness_value_bound = candidate("witness-value-bound", "witness-value", &[]);
    witness_value_bound["action_identity"]["normalized_arguments"] =
        Value::Array((0..33).map(|index| json!(index)).collect());
    let witness_value_bound_path =
        write_candidates(&root, "witness-value-bound.json", vec![witness_value_bound]);
    let witness_value_bound_result = run(&root, &command(&witness_value_bound_path));
    assert!(!witness_value_bound_result.status.success());
    assert!(
        String::from_utf8(witness_value_bound_result.stderr)
            .unwrap()
            .contains("action identity arguments exceed the bounded JSON shape")
    );

    let oversized_path = root.path().join("over-64-kib.json");
    fs::write(&oversized_path, vec![b' '; 64 * 1024 + 1]).unwrap();
    let oversized_result = run(&root, &command(&oversized_path.display().to_string()));
    assert!(!oversized_result.status.success());
    assert!(
        String::from_utf8(oversized_result.stderr)
            .unwrap()
            .contains("exceeds the 65536-byte limit")
    );
}

#[test]
fn guidance_shadow_accepts_128_candidates_within_64_kib_and_rejects_129_by_count() {
    let (root, project) = initialized_project("guidance-candidate-limits");
    target(&project, "snapshot-current", "2.0.0");
    finding(&project);

    let candidates = (0..128)
        .map(|index| {
            let id = format!("limit-{index:03}");
            candidate(&id, &id, &[])
        })
        .collect::<Vec<_>>();
    let allowed_path = write_candidates(&root, "128-candidates.json", candidates.clone());
    assert!(
        fs::metadata(&allowed_path).unwrap().len() <= 64 * 1024,
        "the 128-candidate regression must reach the cardinality validator under the 64KiB cap"
    );
    let allowed_result = run(&root, &command(&allowed_path));
    assert!(
        allowed_result.status.success(),
        "stderr: {:?}",
        allowed_result.stderr
    );
    let allowed_report: Value = serde_json::from_slice(&allowed_result.stdout).unwrap();
    assert_eq!(
        allowed_report["report"]["considered_candidate_count"],
        json!(128)
    );

    let mut one_too_many = candidates;
    one_too_many.push(candidate("limit-128", "limit-128", &[]));
    let rejected_path = write_candidates(&root, "129-candidates.json", one_too_many);
    assert!(
        fs::metadata(&rejected_path).unwrap().len() <= 64 * 1024,
        "the 129-candidate regression must not trip the outer file-size cap first"
    );
    let rejected_result = run(&root, &command(&rejected_path));
    assert!(!rejected_result.status.success());
    assert!(
        String::from_utf8(rejected_result.stderr)
            .unwrap()
            .contains("received 129 controlled candidates, above the 128-candidate limit")
    );
}
