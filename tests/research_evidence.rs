mod support;

use std::fs;

use echidna::adapters::project_event_store::{Project, ProjectEventStream};
use echidna::adapters::tools::ToolRegistry;
use echidna::domain::artifacts::{ArtifactEvent, ArtifactId, ArtifactRecord, BlobId};
use echidna::domain::events::{EventEnvelope, EventId, EventType, OccurredAt, StreamId};
use echidna::domain::research::{
    FindingDraft, FindingId, FindingRecord, FindingStatus, HypothesisDraft, HypothesisId,
    HypothesisRecord, HypothesisStatus, ResearchEvent,
};
use echidna::domain::session::SessionEvent;
use echidna::domain::tools::{
    ExecutionId, ExecutionReceipt, ToolCall, ToolCallId, ToolName, ToolResultStatus,
};
use echidna::ports::EventSink;
use echidna::ports::tool::ToolExecutor;
use serde_json::{Value, json};
use support::TemporaryDirectory;

fn initialized_project(label: &str) -> (TemporaryDirectory, Project) {
    let root = TemporaryDirectory::new(label);
    let project = Project::initialize(root.path()).unwrap();
    (root, project)
}

fn call(id: &str, name: &str, arguments: Value) -> ToolCall {
    ToolCall::new(ToolCallId::new(id), ToolName::new(name), arguments)
}

async fn execute(
    tools: &ToolRegistry,
    id: &str,
    name: &str,
    arguments: Value,
) -> echidna::domain::tools::ToolExecutionOutcome {
    let plan = tools
        .plan(
            ExecutionId::new(format!("execution-{id}")),
            call(id, name, arguments),
        )
        .unwrap();
    tools.execute(plan).await.expect("builtin Tool completes")
}

fn update(event_id: &str, operation: Value) -> Value {
    json!({
        "event_id": event_id,
        "occurred_at": "2026-08-30T12:00:00Z",
        "operation": operation,
    })
}

fn target_snapshot(id: &str) -> Value {
    json!({
        "id": id,
        "target_id": "fixture.target",
        "kind": "mobile",
        "locator": "package:fixture.target",
        "version": "1.0.0",
        "build_id": "build-1",
        "environment_fingerprint": "environment-1",
        "device_fingerprint": "device-1",
        "captured_at": "2026-08-30T11:59:00Z"
    })
}

fn hypothesis(id: &str) -> Value {
    json!({
        "id": id,
        "title": format!("Hypothesis {id}"),
        "target_id": "fixture.target",
        "attack_surface": "exported activity",
        "vulnerability_class": "intent redirection",
        "entry_point": "fixture.target/.Entry",
        "sensitive_sink": "startActivity",
        "rationale": "controlled typed-evidence fixture",
        "status": "active",
        "priority": 10
    })
}

fn experiment_for(
    id: &str,
    hypothesis_id: &str,
    snapshot_id: &str,
    tool_version: &str,
    result: &str,
) -> Value {
    json!({
        "kind": "record_experiment",
        "experiment": {
            "id": id,
            "hypothesis_id": hypothesis_id,
            "target_snapshot_id": snapshot_id,
            "capsule_id": format!("capsule-{id}"),
            "goal": "exercise typed evidence",
            "result": result
        },
        "capsule": {
            "id": format!("capsule-{id}"),
            "experiment_id": id,
            "target_snapshot_id": snapshot_id,
            "action": "Run Controlled Evidence Fixture",
            "arguments": {"case": tool_version},
            "tool_version": tool_version,
            "execution_ids": [format!("execution-{id}")],
            "execution_receipt_ids": [format!("receipt-{id}")],
            "toolchain_fingerprint": "fixture-toolchain"
        }
    })
}

fn experiment(id: &str, tool_version: &str, result: &str) -> Value {
    experiment_for(id, "hyp-base", "snapshot-1", tool_version, result)
}

fn evidence_link(relation: &str, kind: &str, id: &str) -> Value {
    json!({
        "relation": relation,
        "evidence": {"kind": kind, "id": id}
    })
}

async fn record_target_and_hypothesis(tools: &ToolRegistry) {
    for (id, operation) in [
        (
            "target",
            json!({"kind": "record_target_snapshot", "record": target_snapshot("snapshot-1")}),
        ),
        (
            "hyp-base",
            json!({"kind": "record_hypothesis", "record": hypothesis("hyp-base")}),
        ),
    ] {
        let outcome = execute(
            tools,
            id,
            "research_update",
            update(&format!("event-{id}"), operation),
        )
        .await;
        assert_eq!(outcome.result.status, ToolResultStatus::Succeeded);
    }
}

async fn record_baseline(tools: &ToolRegistry, project: &Project) {
    record_target_and_hypothesis(tools).await;
    append_receipt(project, "receipt-exp-1", "execution-exp-1");
    let outcome = execute(
        tools,
        "exp-1",
        "research_update",
        update("event-exp-1", experiment("exp-1", "tool-v1", "positive")),
    )
    .await;
    assert_eq!(outcome.result.status, ToolResultStatus::Succeeded);
}

fn append_event(project: &Project, event_id: &str, event_type: &str, payload: Value) {
    let mut writer = project.open_writer().unwrap();
    let mut store = writer.open_research(None).unwrap();
    let sequence = store.next_sequence().unwrap();
    EventSink::append_event(
        &mut store,
        &EventEnvelope::current(
            EventId::new(event_id),
            ProjectEventStream::Research.stream_id(),
            sequence,
            OccurredAt::new("2026-08-30T12:00:00Z"),
            EventType::new(event_type),
            payload,
        ),
    )
    .unwrap();
}

fn append_receipt(project: &Project, event_id: &str, execution_id: &str) {
    append_receipt_to_session(project, "evidence-receipts", event_id, execution_id);
}

fn append_receipt_to_session(project: &Project, session: &str, event_id: &str, execution_id: &str) {
    let session_id = StreamId::new(session);
    let stream_id = ProjectEventStream::Session(session_id.clone()).stream_id();
    let mut writer = project.open_writer().unwrap();
    let mut store = writer.open_session(session_id, None).unwrap();
    let event = SessionEvent::ExecutionReceiptRecorded {
        receipt: ExecutionReceipt {
            execution_id: ExecutionId::new(execution_id),
            plan_fingerprint: format!("plan-{execution_id}"),
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
    };
    let envelope = event
        .into_envelope(
            EventId::new(event_id),
            stream_id,
            store.next_sequence().unwrap(),
            OccurredAt::new("2026-08-30T12:00:00Z"),
        )
        .unwrap();
    EventSink::append_event(&mut store, &envelope).unwrap();
}

fn append_artifact(project: &Project) {
    append_artifact_for(project, "artifact-1", "snapshot-1");
}

fn append_artifact_for(project: &Project, artifact_id: &str, snapshot_id: &str) {
    let event_id = EventId::new(format!("event-{artifact_id}"));
    let event = ResearchEvent::Artifact(ArtifactEvent::Created(ArtifactRecord {
        id: ArtifactId::new(artifact_id),
        blob_id: BlobId::new("a".repeat(64)).unwrap(),
        media_type: "application/json".to_owned(),
        size_bytes: 2,
        source_path: "fixture/artifact.json".to_owned(),
        created_by_event_id: event_id.clone(),
        produced_by_tool_call_id: ToolCallId::new("artifact-tool-call"),
        target_snapshot_id: echidna::domain::research::TargetSnapshotId::new(snapshot_id),
        toolchain_fingerprint: "fixture-toolchain".to_owned(),
        parent_artifact_ids: Vec::new(),
        transform: None,
    }));
    append_event(
        project,
        event_id.as_str(),
        event.event_type().as_str(),
        event.encode_payload().unwrap(),
    );
}

fn research_line_count(project: &Project) -> usize {
    fs::read_to_string(project.root().join(".echidna/research.jsonl"))
        .unwrap()
        .lines()
        .count()
}

#[tokio::test]
async fn typed_links_cover_every_relation_and_reject_invalid_or_duplicate_references() {
    let (_root, project) = initialized_project("typed-evidence-validation");
    let tools = ToolRegistry::for_project(project.root());
    record_baseline(&tools, &project).await;
    append_artifact(&project);

    let links = json!([
        evidence_link("supports", "experiment", "exp-1"),
        evidence_link("contradicts", "experiment", "exp-1"),
        evidence_link("positive_control", "artifact", "artifact-1"),
        evidence_link("negative_control", "execution_receipt", "receipt-exp-1"),
        evidence_link("environment_control", "experiment", "exp-1"),
        evidence_link("reachability_proof", "artifact", "artifact-1"),
        evidence_link("prerequisite_proof", "execution_receipt", "receipt-exp-1"),
        evidence_link("impact_proof", "experiment", "exp-1")
    ]);
    let outcome = execute(
        &tools,
        "all-relations",
        "research_update",
        update(
            "event-all-relations",
            json!({"kind": "update_hypothesis", "update": {
                "id": "hyp-base", "evidence_links_add": links
            }}),
        ),
    )
    .await;
    assert_eq!(outcome.result.status, ToolResultStatus::Succeeded);

    let get = execute(&tools, "get-relations", "research_get", json!({})).await;
    let stored = &get.result.output["state"]["hypotheses"]["hyp-base"]["evidence_links"];
    assert_eq!(stored.as_array().unwrap().len(), 8);
    assert_eq!(
        stored
            .as_array()
            .unwrap()
            .iter()
            .map(|link| link["relation"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "supports",
            "contradicts",
            "positive_control",
            "negative_control",
            "environment_control",
            "reachability_proof",
            "prerequisite_proof",
            "impact_proof"
        ]
    );

    for (case, additions) in [
        (
            "existing-duplicate",
            json!([evidence_link("supports", "experiment", "exp-1")]),
        ),
        (
            "request-duplicate",
            json!([
                evidence_link("negative_control", "artifact", "artifact-1"),
                evidence_link("negative_control", "artifact", "artifact-1")
            ]),
        ),
        (
            "missing-artifact",
            json!([evidence_link("supports", "artifact", "artifact-missing")]),
        ),
        (
            "missing-receipt",
            json!([evidence_link(
                "supports",
                "execution_receipt",
                "receipt-missing"
            )]),
        ),
        (
            "wrong-reference-kind",
            json!([evidence_link("supports", "artifact", "exp-1")]),
        ),
    ] {
        let before = research_line_count(&project);
        let outcome = execute(
            &tools,
            case,
            "research_update",
            update(
                &format!("event-{case}"),
                json!({"kind": "update_hypothesis", "update": {
                    "id": "hyp-base", "evidence_links_add": additions
                }}),
            ),
        )
        .await;
        assert_eq!(outcome.result.status, ToolResultStatus::Failed, "{case}");
        assert_eq!(research_line_count(&project), before, "{case}");
    }

    let different_relation = execute(
        &tools,
        "same-reference-new-relation",
        "research_update",
        update(
            "event-same-reference-new-relation",
            json!({"kind": "update_hypothesis", "update": {
                "id": "hyp-base",
                "evidence_links_add": [evidence_link("positive_control", "experiment", "exp-1")]
            }}),
        ),
    )
    .await;
    assert_eq!(
        different_relation.result.status,
        ToolResultStatus::Succeeded
    );

    let (_other_root, other_project) = initialized_project("typed-evidence-cross-project");
    let other_tools = ToolRegistry::for_project(other_project.root());
    let cross_project = execute(
        &other_tools,
        "cross-project-artifact",
        "research_update",
        update(
            "event-cross-project-artifact",
            json!({"kind": "record_hypothesis", "record": {
                "id": "hyp-cross-project", "title": "Cross project", "target_id": "fixture.target",
                "attack_surface": "surface", "vulnerability_class": "class", "entry_point": "entry",
                "sensitive_sink": "sink", "rationale": "must reject foreign Artifact",
                "status": "active", "priority": 1,
                "evidence_links": [evidence_link("supports", "artifact", "artifact-1")]
            }}),
        ),
    )
    .await;
    assert_eq!(cross_project.result.status, ToolResultStatus::Failed);
    assert_eq!(research_line_count(&other_project), 0);

    let before = research_line_count(&project);
    for kind in ["observation", "vision", "replay", "eval"] {
        let planned = tools.plan(
            ExecutionId::new(format!("execution-derived-{kind}")),
            call(
                &format!("derived-{kind}"),
                "research_update",
                update(
                    &format!("event-derived-{kind}"),
                    json!({"kind": "update_hypothesis", "update": {
                        "id": "hyp-base",
                        "evidence_links_add": [evidence_link("supports", kind, "derived-1")]
                    }}),
                ),
            ),
        );
        assert!(planned.is_err(), "derived kind `{kind}` must be rejected");
    }
    assert_eq!(research_line_count(&project), before);

    let nested_unknown = update(
        "event-nested-unknown",
        json!({"kind": "update_hypothesis", "update": {
            "id": "hyp-base",
            "evidence_links_add": [{
                "relation": "supports",
                "evidence": {"kind": "experiment", "id": "exp-1", "extra": true}
            }]
        }}),
    );
    assert!(
        tools
            .plan(
                ExecutionId::new("execution-nested-unknown"),
                call("nested-unknown", "research_update", nested_unknown),
            )
            .is_err()
    );
    assert!(
        ResearchEvent::decode(
            "research.hypothesis_updated",
            json!({"research_schema_version": 2, "data": {
                "id": "hyp-base",
                "evidence_links_add": [{
                    "relation": "supports",
                    "evidence": {"kind": "experiment", "id": "exp-1", "extra": true}
                }]
            }}),
        )
        .is_err()
    );
}

#[tokio::test]
async fn experiment_capsules_require_real_matching_session_receipts() {
    let (_root, project) = initialized_project("typed-evidence-receipt-binding");
    let tools = ToolRegistry::for_project(project.root());
    record_target_and_hypothesis(&tools).await;

    let before = research_line_count(&project);
    let missing = execute(
        &tools,
        "exp-missing",
        "research_update",
        update(
            "event-exp-missing",
            experiment("exp-missing", "tool-v1", "positive"),
        ),
    )
    .await;
    assert_eq!(missing.result.status, ToolResultStatus::Failed);
    assert_eq!(research_line_count(&project), before);

    append_receipt(&project, "receipt-exp-mismatch", "execution-unrelated");
    let mismatch = execute(
        &tools,
        "exp-mismatch",
        "research_update",
        update(
            "event-exp-mismatch",
            experiment("exp-mismatch", "tool-v1", "positive"),
        ),
    )
    .await;
    assert_eq!(mismatch.result.status, ToolResultStatus::Failed);
    assert_eq!(research_line_count(&project), before);
}

#[tokio::test]
async fn research_get_reduces_the_current_session_receipt_catalog_without_a_stale_cache() {
    let (_root, project) = initialized_project("typed-evidence-receipt-catalog-refresh");
    let tools = ToolRegistry::for_project(project.root());
    record_baseline(&tools, &project).await;

    let first = execute(&tools, "catalog-first", "research_get", json!({})).await;
    assert_eq!(first.result.status, ToolResultStatus::Succeeded);
    assert!(first.receipt.cache_source.is_none());
    assert!(first.receipt.cache_validation.is_none());

    append_receipt_to_session(
        &project,
        "evidence-receipts-duplicate",
        "receipt-exp-1",
        "execution-exp-1",
    );
    let before = research_line_count(&project);
    let duplicate = execute(&tools, "catalog-duplicate", "research_get", json!({})).await;
    assert_eq!(duplicate.result.status, ToolResultStatus::Failed);
    assert_eq!(
        duplicate.receipt.exit_reason.as_deref(),
        Some("research_read_failed")
    );
    assert!(duplicate.receipt.cache_source.is_none());
    assert_eq!(research_line_count(&project), before);
}

#[tokio::test]
async fn finding_evidence_is_bound_to_the_exact_target_version_and_updates_are_atomic() {
    let (_root, project) = initialized_project("typed-evidence-target-binding");
    let tools = ToolRegistry::for_project(project.root());
    record_baseline(&tools, &project).await;

    let mut foreign_snapshot = target_snapshot("snapshot-foreign");
    foreign_snapshot["target_id"] = json!("foreign.target");
    foreign_snapshot["locator"] = json!("package:foreign.target");
    foreign_snapshot["build_id"] = json!("build-foreign");
    let recorded = execute(
        &tools,
        "target-foreign",
        "research_update",
        update(
            "event-target-foreign",
            json!({"kind": "record_target_snapshot", "record": foreign_snapshot}),
        ),
    )
    .await;
    assert_eq!(recorded.result.status, ToolResultStatus::Succeeded);

    let before_foreign_finding = research_line_count(&project);
    let foreign_finding = execute(
        &tools,
        "finding-foreign-snapshot",
        "research_update",
        update(
            "event-finding-foreign-snapshot",
            json!({"kind": "record_finding", "keep_independent": true, "record": {
                "id": "finding-foreign-snapshot", "title": "Foreign snapshot", "status": "candidate",
                "target_id": "fixture.target", "target_version": "1.0.0",
                "vulnerability_class": "class", "entry_point": "entry", "sensitive_sink": "sink",
                "target_snapshot_ids": ["snapshot-foreign"], "prerequisites": [],
                "impact": "candidate impact"
            }}),
        ),
    )
    .await;
    assert_eq!(foreign_finding.result.status, ToolResultStatus::Failed);
    assert_eq!(research_line_count(&project), before_foreign_finding);

    let mut newer_snapshot = target_snapshot("snapshot-v2");
    newer_snapshot["version"] = json!("2.0.0");
    newer_snapshot["build_id"] = json!("build-2");
    let recorded = execute(
        &tools,
        "target-v2",
        "research_update",
        update(
            "event-target-v2",
            json!({"kind": "record_target_snapshot", "record": newer_snapshot}),
        ),
    )
    .await;
    assert_eq!(recorded.result.status, ToolResultStatus::Succeeded);
    append_receipt(&project, "receipt-exp-v2", "execution-exp-v2");
    let experiment_v2 = execute(
        &tools,
        "exp-v2",
        "research_update",
        update(
            "event-exp-v2",
            experiment_for("exp-v2", "hyp-base", "snapshot-v2", "tool-v2", "positive"),
        ),
    )
    .await;
    assert_eq!(experiment_v2.result.status, ToolResultStatus::Succeeded);

    append_artifact_for(&project, "artifact-foreign", "snapshot-foreign");
    let mut no_receipt_experiment = experiment_for(
        "exp-foreign-artifact",
        "hyp-base",
        "snapshot-1",
        "tool-foreign-artifact",
        "positive",
    );
    no_receipt_experiment["capsule"]["execution_ids"] = json!([]);
    no_receipt_experiment["capsule"]["execution_receipt_ids"] = json!([]);
    let recorded = execute(
        &tools,
        "exp-foreign-artifact",
        "research_update",
        update("event-exp-foreign-artifact", no_receipt_experiment),
    )
    .await;
    assert_eq!(recorded.result.status, ToolResultStatus::Succeeded);
    let updated = execute(
        &tools,
        "exp-foreign-artifact-update",
        "research_update",
        update(
            "event-exp-foreign-artifact-update",
            json!({"kind": "update_experiment", "update": {
                "id": "exp-foreign-artifact", "artifact_refs_add": ["artifact-foreign"]
            }}),
        ),
    )
    .await;
    assert_eq!(
        updated.result.status,
        ToolResultStatus::Succeeded,
        "{}",
        updated.result.output
    );
    let provenance_finding = execute(
        &tools,
        "finding-provenance-filter",
        "research_update",
        update(
            "event-finding-provenance-filter",
            json!({"kind": "record_finding", "keep_independent": true, "record": {
                "id": "finding-provenance-filter", "title": "Foreign provenance filter",
                "status": "candidate", "target_id": "fixture.target", "target_version": "1.0.0",
                "vulnerability_class": "class", "entry_point": "entry", "sensitive_sink": "sink",
                "target_snapshot_ids": ["snapshot-1"], "prerequisites": [],
                "impact": "candidate impact",
                "evidence_links": [evidence_link("supports", "experiment", "exp-foreign-artifact")]
            }}),
        ),
    )
    .await;
    assert_eq!(
        provenance_finding.result.status,
        ToolResultStatus::Succeeded
    );

    let finding = execute(
        &tools,
        "finding-v1",
        "research_update",
        update(
            "event-finding-v1",
            json!({"kind": "record_finding", "keep_independent": true, "record": {
                "id": "finding-v1", "title": "Version one finding", "status": "candidate",
                "target_id": "fixture.target", "target_version": "1.0.0",
                "vulnerability_class": "class", "entry_point": "entry", "sensitive_sink": "sink",
                "target_snapshot_ids": ["snapshot-1"], "prerequisites": [],
                "impact": "candidate impact",
                "evidence_links": [evidence_link("supports", "experiment", "exp-1")]
            }}),
        ),
    )
    .await;
    assert_eq!(finding.result.status, ToolResultStatus::Succeeded);

    let before_update = research_line_count(&project);
    let cross_version_update = execute(
        &tools,
        "finding-v1-cross-version",
        "research_update",
        update(
            "event-finding-v1-cross-version",
            json!({"kind": "update_finding", "update": {
                "id": "finding-v1", "status": "confirmed",
                "evidence_links_add": [evidence_link("supports", "experiment", "exp-v2")]
            }}),
        ),
    )
    .await;
    assert_eq!(cross_version_update.result.status, ToolResultStatus::Failed);
    assert_eq!(research_line_count(&project), before_update);

    let get = execute(&tools, "target-binding-get", "research_get", json!({})).await;
    assert_eq!(get.result.status, ToolResultStatus::Succeeded);
    assert_eq!(
        get.result.output["state"]["findings"]["finding-v1"]["status"],
        "candidate"
    );
    assert_eq!(
        get.result.output["state"]["findings"]["finding-v1"]["evidence_links"],
        json!([evidence_link("supports", "experiment", "exp-1")])
    );
    let report = get.result.output["evidence_gap_reports"]
        .as_array()
        .unwrap()
        .iter()
        .find(|report| report["research_id"] == json!({"kind": "finding", "id": "finding-v1"}))
        .unwrap();
    assert!(
        report["gaps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|gap| gap["kind"] == "repeatability_missing")
    );
    let provenance_report = get.result.output["evidence_gap_reports"]
        .as_array()
        .unwrap()
        .iter()
        .find(|report| {
            report["research_id"] == json!({"kind": "finding", "id": "finding-provenance-filter"})
        })
        .unwrap();
    assert!(
        provenance_report["gaps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|gap| gap["kind"] == "artifact_or_receipt_provenance_missing")
    );
}

#[tokio::test]
async fn v1_evidence_vectors_upcast_conservatively_without_rewriting_the_stream() {
    let (_root, project) = initialized_project("legacy-evidence-upcast");
    let tools = ToolRegistry::for_project(project.root());
    record_baseline(&tools, &project).await;
    append_artifact(&project);

    let hypothesis_record = HypothesisRecord::from_draft(HypothesisDraft {
        id: HypothesisId::new("hyp-legacy"),
        title: "Legacy hypothesis".to_owned(),
        target_id: "fixture.target".to_owned(),
        attack_surface: "surface".to_owned(),
        vulnerability_class: "class".to_owned(),
        entry_point: "entry".to_owned(),
        sensitive_sink: "sink".to_owned(),
        rationale: "legacy fixture".to_owned(),
        status: HypothesisStatus::Active,
        priority: 1,
        evidence_links: Vec::new(),
        last_action: None,
        next_action: None,
    })
    .unwrap();
    let mut hypothesis_value = serde_json::to_value(hypothesis_record).unwrap();
    hypothesis_value
        .as_object_mut()
        .unwrap()
        .remove("evidence_links");
    hypothesis_value["evidence_refs"] = json!([{"kind": "experiment", "id": "exp-1"}]);
    append_event(
        &project,
        "event-hyp-legacy",
        "research.hypothesis_recorded",
        json!({"research_schema_version": 1, "data": {
            "record": hypothesis_value, "keep_independent": false
        }}),
    );
    append_event(
        &project,
        "event-hyp-legacy-update",
        "research.hypothesis_updated",
        json!({"research_schema_version": 1, "data": {
            "id": "hyp-legacy",
            "evidence_refs_add": [{"kind": "execution_receipt", "id": "receipt-exp-1"}]
        }}),
    );

    let finding_record = FindingRecord::from_draft(FindingDraft {
        id: FindingId::new("finding-legacy"),
        title: "Legacy finding".to_owned(),
        status: FindingStatus::Candidate,
        target_id: "fixture.target".to_owned(),
        target_version: "1.0.0".to_owned(),
        vulnerability_class: "class".to_owned(),
        entry_point: "entry".to_owned(),
        sensitive_sink: "sink".to_owned(),
        target_snapshot_ids: vec![echidna::domain::research::TargetSnapshotId::new(
            "snapshot-1",
        )],
        prerequisites: vec!["authenticated".to_owned()],
        impact: "legacy impact".to_owned(),
        evidence_links: Vec::new(),
    })
    .unwrap();
    let mut finding_value = serde_json::to_value(finding_record).unwrap();
    finding_value
        .as_object_mut()
        .unwrap()
        .remove("evidence_links");
    finding_value["evidence_refs"] = json!([{"kind": "experiment", "id": "exp-1"}]);
    finding_value["negative_refs"] = json!([{"kind": "execution_receipt", "id": "receipt-exp-1"}]);
    append_event(
        &project,
        "event-finding-legacy",
        "research.finding_recorded",
        json!({"research_schema_version": 1, "data": {
            "record": finding_value, "keep_independent": false
        }}),
    );
    append_event(
        &project,
        "event-finding-legacy-update",
        "research.finding_updated",
        json!({"research_schema_version": 1, "data": {
            "id": "finding-legacy",
            "evidence_refs_add": [{"kind": "artifact", "id": "artifact-1"}],
            "negative_refs_add": [{"kind": "experiment", "id": "exp-1"}]
        }}),
    );

    let stream_path = project.root().join(".echidna/research.jsonl");
    let before = fs::read(&stream_path).unwrap();
    let get = execute(&tools, "legacy-get", "research_get", json!({})).await;
    assert_eq!(get.result.status, ToolResultStatus::Succeeded);
    assert_eq!(fs::read(&stream_path).unwrap(), before);

    let state = &get.result.output["state"];
    assert_eq!(
        state["hypotheses"]["hyp-legacy"]["evidence_links"],
        json!([
            evidence_link("supports", "experiment", "exp-1"),
            evidence_link("supports", "execution_receipt", "receipt-exp-1")
        ])
    );
    assert_eq!(
        state["findings"]["finding-legacy"]["evidence_links"],
        json!([
            evidence_link("supports", "experiment", "exp-1"),
            evidence_link("contradicts", "execution_receipt", "receipt-exp-1"),
            evidence_link("supports", "artifact", "artifact-1"),
            evidence_link("contradicts", "experiment", "exp-1")
        ])
    );
    assert!(
        state["findings"]["finding-legacy"]["evidence_links"]
            .as_array()
            .unwrap()
            .iter()
            .all(|link| !matches!(
                link["relation"].as_str(),
                Some("positive_control" | "negative_control" | "environment_control")
            ))
    );

    let explicit_controls = execute(
        &tools,
        "explicit-controls",
        "research_update",
        update(
            "event-explicit-controls",
            json!({"kind": "update_finding", "update": {
                "id": "finding-legacy",
                "evidence_links_add": [
                    evidence_link("positive_control", "experiment", "exp-1"),
                    evidence_link("negative_control", "artifact", "artifact-1"),
                    evidence_link("environment_control", "execution_receipt", "receipt-exp-1")
                ]
            }}),
        ),
    )
    .await;
    assert_eq!(explicit_controls.result.status, ToolResultStatus::Succeeded);

    for payload in [
        json!({"research_schema_version": 1, "data": {
            "id": "hyp-legacy", "evidence_links_add": []
        }}),
        json!({"research_schema_version": 2, "data": {
            "id": "hyp-legacy", "evidence_refs_add": []
        }}),
    ] {
        assert!(ResearchEvent::decode("research.hypothesis_updated", payload).is_err());
    }
}

#[tokio::test]
async fn evidence_gap_projection_is_stable_read_only_and_distinguishes_unknown_from_missing() {
    let (_root, project) = initialized_project("evidence-gap-projection");
    let tools = ToolRegistry::for_project(project.root());
    record_baseline(&tools, &project).await;
    append_receipt(&project, "receipt-exp-2", "execution-exp-2");
    let second = execute(
        &tools,
        "exp-2",
        "research_update",
        update("event-exp-2", experiment("exp-2", "tool-v2", "positive")),
    )
    .await;
    assert_eq!(second.result.status, ToolResultStatus::Succeeded);

    let complete_finding = execute(
        &tools,
        "complete-finding",
        "research_update",
        update(
            "event-complete-finding",
            json!({"kind": "record_finding", "record": {
                "id": "finding-complete", "title": "Complete finding", "status": "confirmed",
                "target_id": "fixture.target", "target_version": "1.0.0",
                "vulnerability_class": "intent redirection", "entry_point": "fixture.target/.Entry",
                "sensitive_sink": "startActivity", "target_snapshot_ids": ["snapshot-1"],
                "prerequisites": ["authenticated"], "impact": "controlled impact",
                "evidence_links": [
                    evidence_link("supports", "experiment", "exp-1"),
                    evidence_link("supports", "experiment", "exp-2"),
                    evidence_link("positive_control", "experiment", "exp-1"),
                    evidence_link("negative_control", "experiment", "exp-1"),
                    evidence_link("environment_control", "experiment", "exp-1"),
                    evidence_link("reachability_proof", "experiment", "exp-1"),
                    evidence_link("prerequisite_proof", "experiment", "exp-1"),
                    evidence_link("impact_proof", "experiment", "exp-1")
                ]
            }}),
        ),
    )
    .await;
    assert_eq!(complete_finding.result.status, ToolResultStatus::Succeeded);

    let stream_path = project.root().join(".echidna/research.jsonl");
    let before = fs::read(&stream_path).unwrap();
    let first = execute(&tools, "gap-first", "research_get", json!({})).await;
    let second = execute(&tools, "gap-second", "research_get", json!({})).await;
    assert_eq!(first.result.output, second.result.output);
    assert_eq!(fs::read(&stream_path).unwrap(), before);

    let reports = first.result.output["evidence_gap_reports"]
        .as_array()
        .unwrap();
    let hypothesis_report = reports
        .iter()
        .find(|report| report["research_id"] == json!({"kind": "hypothesis", "id": "hyp-base"}))
        .unwrap();
    let finding_report = reports
        .iter()
        .find(|report| {
            report["research_id"] == json!({"kind": "finding", "id": "finding-complete"})
        })
        .unwrap();
    assert_eq!(finding_report["gaps"], json!([]));
    assert!(
        hypothesis_report["gaps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|gap| gap["kind"] == "prerequisite_unproven" && gap["state"] == "unknown")
    );
    assert!(
        hypothesis_report["gaps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|gap| gap["kind"] == "impact_unproven" && gap["state"] == "unknown")
    );
    assert!(
        hypothesis_report["gaps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|gap| gap["kind"] == "positive_control_missing" && gap["state"] == "missing")
    );
}
