mod support;

use std::fs;

use echidna::adapters::project_event_store::{Project, ProjectEventStream};
use echidna::adapters::tools::ToolRegistry;
use echidna::domain::events::{EventEnvelope, EventId, EventSequence, EventType, OccurredAt};
use echidna::domain::research::{
    HypothesisDraft, HypothesisId, HypothesisRecord, HypothesisStatus, MergeRecordRef,
    ResearchEvent,
};
use echidna::domain::tools::{
    ExecutionId, ToolCall, ToolCallId, ToolEffect, ToolExposure, ToolExposurePlan, ToolName,
    ToolParallelism, ToolResultStatus, ToolRetryPolicy,
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

async fn execute(tools: &ToolRegistry, id: &str, name: &str, arguments: Value) -> Value {
    let plan = tools
        .plan(
            ExecutionId::new(format!("execution-{id}")),
            call(id, name, arguments),
        )
        .unwrap();
    let outcome = tools.execute(plan).await.expect("Tool execution completes");
    assert_eq!(outcome.result.tool_name, ToolName::new(name));
    outcome.result.output
}

async fn execute_outcome(
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
    tools.execute(plan).await.expect("Tool execution completes")
}

fn update(event_id: &str, operation: Value) -> Value {
    json!({
        "event_id": event_id,
        "occurred_at": "2026-08-22T12:00:00Z",
        "operation": operation,
    })
}

fn target_snapshot(id: &str, version: &str, environment_fingerprint: &str) -> Value {
    json!({
        "id": id,
        "target_id": "com.example.target",
        "kind": "mobile",
        "locator": "package:com.example.target",
        "version": version,
        "build_id": "build-42",
        "target_artifact_ids": ["artifact-target"],
        "environment_fingerprint": environment_fingerprint,
        "device_fingerprint": "device-pixel",
        "captured_at": "2026-08-22T11:59:00Z",
        "notes": "analysis tool version must not enter the snapshot identity"
    })
}

fn hypothesis(id: &str, title: &str, sensitive_sink: &str) -> Value {
    json!({
        "id": id,
        "title": title,
        "target_id": "com.example.target",
        "attack_surface": "exported activity",
        "vulnerability_class": "intent redirection",
        "entry_point": "com.example.target/.ShareActivity",
        "sensitive_sink": sensitive_sink,
        "rationale": "controlled fixture rationale",
        "status": "active",
        "priority": 10
    })
}

fn experiment(id: &str, capsule_id: &str, snapshot_id: &str, tool_version: &str) -> Value {
    json!({
        "kind": "record_experiment",
        "experiment": {
            "id": id,
            "hypothesis_id": "hyp-1",
            "target_snapshot_id": snapshot_id,
            "capsule_id": capsule_id,
            "goal": "verify controlled intent delivery",
            "result": "pending"
        },
        "capsule": {
            "id": capsule_id,
            "experiment_id": id,
            "target_snapshot_id": snapshot_id,
            "action": "Run Fixture Tool --CaseSensitive",
            "arguments": {"nested": {"z": "PayloadCase", "a": ["first", "Second"]}},
            "tool_version": tool_version,
            "execution_ids": [format!("execution-{id}")],
            "input_artifact_ids": ["artifact-input"],
            "environment_delta": "locale=zh_CN",
            "toolchain_fingerprint": "toolchain-fixture"
        }
    })
}

fn research_state(output: Value) -> Value {
    assert_eq!(output["status"], "ok");
    output["state"].clone()
}

fn research_line_count(project: &Project) -> usize {
    fs::read_to_string(project.root().join(".cyber-agent/research.jsonl"))
        .unwrap()
        .lines()
        .count()
}

#[tokio::test]
async fn research_get_is_lock_free_and_missing_streams_are_never_recreated() {
    let (_root, project) = initialized_project("research-read-only");
    let tools = ToolRegistry::for_project(project.root());
    let control_directory = project.root().join(".cyber-agent");
    let stream_path = control_directory.join("research.jsonl");
    let writer_lock_path = control_directory.join("writer.lock");
    let mut entries_before = fs::read_dir(&control_directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    entries_before.sort();
    let before = fs::read(&stream_path).unwrap();

    let output = execute(&tools, "get-fresh", "research_get", json!({})).await;
    assert_eq!(output["status"], "ok");
    assert!(!writer_lock_path.exists());
    assert_eq!(fs::read(&stream_path).unwrap(), before);
    let mut entries_after = fs::read_dir(&control_directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    entries_after.sort();
    assert_eq!(entries_after, entries_before);

    let writer = project.open_writer().unwrap();
    let lock_before = fs::read(&writer_lock_path).unwrap();
    let output = execute(&tools, "get-while-locked", "research_get", json!({})).await;
    assert_eq!(output["status"], "ok");
    assert_eq!(fs::read(&writer_lock_path).unwrap(), lock_before);
    assert_eq!(fs::read(&stream_path).unwrap(), before);
    drop(writer);

    fs::remove_file(&stream_path).unwrap();
    fs::remove_file(&writer_lock_path).unwrap();
    let missing_get = execute_outcome(&tools, "get-missing", "research_get", json!({})).await;
    assert_eq!(missing_get.result.status, ToolResultStatus::Failed);
    assert_eq!(
        missing_get.receipt.exit_reason.as_deref(),
        Some("research_read_failed")
    );
    assert!(!stream_path.exists());
    assert!(!writer_lock_path.exists());

    let missing_update = execute_outcome(
        &tools,
        "update-missing",
        "research_update",
        update(
            "event-missing-stream",
            json!({
                "kind": "record_target_snapshot",
                "record": target_snapshot("snapshot-missing", "1.0.0", "env-a")
            }),
        ),
    )
    .await;
    assert_eq!(missing_update.result.status, ToolResultStatus::Failed);
    assert_eq!(
        missing_update.receipt.exit_reason.as_deref(),
        Some("research_stream_unavailable")
    );
    assert!(!stream_path.exists());
}

#[tokio::test]
async fn research_update_rejects_initial_hypothesis_missing_experiment_without_append() {
    let (_root, project) = initialized_project("research-missing-hypothesis-evidence");
    let tools = ToolRegistry::for_project(project.root());
    let mut record = hypothesis("hypothesis-missing-evidence", "Evidence boundary", "sink");
    record["evidence_links"] = json!([{
        "relation": "supports",
        "evidence": {"kind": "experiment", "id": "experiment-missing"}
    }]);

    let outcome = execute_outcome(
        &tools,
        "missing-hypothesis-evidence",
        "research_update",
        update(
            "event-hypothesis-missing-evidence",
            json!({"kind": "record_hypothesis", "record": record}),
        ),
    )
    .await;

    assert_eq!(outcome.result.status, ToolResultStatus::Failed);
    assert_eq!(
        outcome.receipt.exit_reason.as_deref(),
        Some("research_transition_rejected")
    );
    assert_eq!(research_line_count(&project), 0);
}

#[tokio::test]
async fn target_and_action_fingerprints_keep_their_semantic_boundaries() {
    let (_root, project) = initialized_project("research-fingerprint-boundaries");
    let tools = ToolRegistry::for_project(project.root());

    let recorded = execute(
        &tools,
        "snapshot-1",
        "research_update",
        update(
            "event-snapshot-1",
            json!({"kind": "record_target_snapshot", "record": target_snapshot("snapshot-1", "1.0.0", "env-a")}),
        ),
    )
    .await;
    assert_eq!(recorded["status"], "recorded");

    let reused = execute(
        &tools,
        "snapshot-duplicate",
        "research_update",
        update(
            "event-snapshot-duplicate",
            json!({"kind": "record_target_snapshot", "record": target_snapshot("snapshot-different-id", "1.0.0", "env-a")}),
        ),
    )
    .await;
    assert_eq!(reused["status"], "existing");
    assert_eq!(reused["event_appended"], false);
    assert_eq!(research_line_count(&project), 1);

    execute(
        &tools,
        "hypothesis-1",
        "research_update",
        update(
            "event-hypothesis-1",
            json!({"kind": "record_hypothesis", "record": hypothesis("hyp-1", "Intent redirect", "startActivity")}),
        ),
    )
    .await;
    execute(
        &tools,
        "experiment-v1",
        "research_update",
        update(
            "event-experiment-v1",
            experiment("experiment-v1", "capsule-v1", "snapshot-1", "tool-v1"),
        ),
    )
    .await;
    let line_count = research_line_count(&project);
    let action_conflict = execute(
        &tools,
        "experiment-duplicate",
        "research_update",
        update(
            "event-experiment-duplicate",
            experiment(
                "experiment-duplicate",
                "capsule-duplicate",
                "snapshot-1",
                "tool-v1",
            ),
        ),
    )
    .await;
    assert_eq!(action_conflict["status"], "action_conflict");
    assert_eq!(action_conflict["event_appended"], false);
    assert_eq!(
        action_conflict["required_resolution"],
        json!(["rerun_reason"])
    );
    assert_eq!(research_line_count(&project), line_count);
    execute(
        &tools,
        "experiment-rerun",
        "research_update",
        update(
            "event-experiment-rerun",
            json!({
                "kind": "record_experiment",
                "experiment": {
                    "id": "experiment-rerun", "hypothesis_id": "hyp-1", "target_snapshot_id": "snapshot-1",
                    "capsule_id": "capsule-rerun", "goal": "deliberate rerun", "result": "pending"
                },
                "capsule": {
                    "id": "capsule-rerun", "experiment_id": "experiment-rerun", "target_snapshot_id": "snapshot-1",
                    "action": "Run Fixture Tool --CaseSensitive",
                    "arguments": {"nested": {"z": "PayloadCase", "a": ["first", "Second"]}},
                    "tool_version": "tool-v1", "input_artifact_ids": ["artifact-input"],
                    "environment_delta": "locale=zh_CN", "toolchain_fingerprint": "toolchain-fixture"
                },
                "rerun_reason": "verify nondeterministic device behavior"
            }),
        ),
    )
    .await;
    execute(
        &tools,
        "experiment-v2",
        "research_update",
        update(
            "event-experiment-v2",
            experiment("experiment-v2", "capsule-v2", "snapshot-1", "tool-v2"),
        ),
    )
    .await;

    let state =
        research_state(execute(&tools, "get-before-change", "research_get", json!({})).await);
    assert_eq!(state["target_snapshots"].as_object().unwrap().len(), 1);
    assert_ne!(
        state["experiments"]["experiment-v1"]["action_fingerprint"],
        state["experiments"]["experiment-v2"]["action_fingerprint"]
    );
    assert_eq!(
        state["capsules"]["capsule-v1"]["normalized_action"],
        "Run Fixture Tool --CaseSensitive"
    );

    execute(
        &tools,
        "snapshot-semantic-change",
        "research_update",
        update(
            "event-snapshot-semantic-change",
            json!({"kind": "record_target_snapshot", "record": target_snapshot("snapshot-2", "1.0.1", "env-a")}),
        ),
    )
    .await;
    let state =
        research_state(execute(&tools, "get-after-change", "research_get", json!({})).await);
    assert_eq!(state["target_snapshots"].as_object().unwrap().len(), 2);
    assert_ne!(
        state["target_snapshots"]["snapshot-1"]["snapshot_fingerprint"],
        state["target_snapshots"]["snapshot-2"]["snapshot_fingerprint"]
    );
}

#[tokio::test]
async fn updates_replay_as_new_events_and_preserve_confirmed_finding_requirements() {
    let (_root, project) = initialized_project("research-updates");
    let tools = ToolRegistry::for_project(project.root());
    execute(
        &tools,
        "target",
        "research_update",
        update(
            "event-target",
            json!({"kind": "record_target_snapshot", "record": target_snapshot("snapshot-1", "1.0.0", "env-a")}),
        ),
    )
    .await;
    execute(
        &tools,
        "hypothesis",
        "research_update",
        update(
            "event-hypothesis",
            json!({"kind": "record_hypothesis", "record": hypothesis("hyp-1", "Intent redirect", "startActivity")}),
        ),
    )
    .await;
    execute(
        &tools,
        "experiment",
        "research_update",
        update(
            "event-experiment",
            experiment("experiment-1", "capsule-1", "snapshot-1", "tool-v1"),
        ),
    )
    .await;
    execute(
        &tools,
        "finding",
        "research_update",
        update(
            "event-finding",
            json!({
                "kind": "record_finding",
                "record": {
                    "id": "finding-1", "title": "Intent redirect", "status": "candidate",
                    "target_id": "com.example.target", "target_version": "1.0.0",
                    "vulnerability_class": "intent redirection",
                    "entry_point": "com.example.target/.ShareActivity", "sensitive_sink": "startActivity",
                    "impact": "cross-app intent execution"
                }
            }),
        ),
    )
    .await;
    execute(
        &tools,
        "next-action",
        "research_update",
        update(
            "event-next-action",
            json!({"kind": "record_next_action", "record": {
                "id": "action-1", "research_id": {"kind": "hypothesis", "id": "hyp-1"},
                "priority": 8, "action": "run fixture", "status": "pending"
            }}),
        ),
    )
    .await;

    for (event_id, operation) in [
        (
            "event-hypothesis-update",
            json!({"kind": "update_hypothesis", "update": {
                "id": "hyp-1", "status": "testing", "evidence_links_add": [{"relation": "supports", "evidence": {"kind": "experiment", "id": "experiment-1"}}], "next_action": "action-1"
            }}),
        ),
        (
            "event-experiment-update",
            json!({"kind": "update_experiment", "update": {
                "id": "experiment-1", "result": "positive", "artifact_refs_add": ["artifact-output"], "notes_append": "positive result"
            }}),
        ),
        (
            "event-finding-update",
            json!({"kind": "update_finding", "update": {
                "id": "finding-1", "status": "confirmed", "target_snapshot_ids_add": ["snapshot-1"],
                "evidence_links_add": [{"relation": "supports", "evidence": {"kind": "experiment", "id": "experiment-1"}}], "impact": "validated cross-app intent execution"
            }}),
        ),
        (
            "event-next-action-update",
            json!({"kind": "update_next_action", "update": {
                "id": "action-1", "status": "done"
            }}),
        ),
    ] {
        let output = execute(
            &tools,
            event_id,
            "research_update",
            update(event_id, operation),
        )
        .await;
        assert_eq!(output["status"], "recorded");
    }

    let state = research_state(execute(&tools, "get-updated", "research_get", json!({})).await);
    assert_eq!(state["hypotheses"]["hyp-1"]["status"], "testing");
    assert_eq!(state["experiments"]["experiment-1"]["result"], "positive");
    assert_eq!(state["findings"]["finding-1"]["status"], "confirmed");
    assert_eq!(state["next_actions"]["action-1"]["status"], "done");
    assert!(
        fs::read_to_string(project.root().join(".cyber-agent/research.jsonl"))
            .unwrap()
            .contains("research.finding_updated")
    );

    let replay_tools = ToolRegistry::for_project(project.root());
    let replayed =
        research_state(execute(&replay_tools, "get-replayed", "research_get", json!({})).await);
    assert_eq!(replayed, state);

    let line_count = research_line_count(&project);
    let invalid = execute_outcome(
        &tools,
        "invalid-confirmed",
        "research_update",
        update(
            "event-invalid-confirmed",
            json!({"kind": "record_finding", "record": {
                "id": "finding-invalid", "title": "Missing evidence", "status": "confirmed",
                "target_id": "com.example.target", "target_version": "1.0.0",
                "vulnerability_class": "intent redirection", "entry_point": "entry", "sensitive_sink": "sink",
                "impact": "unproven"
            }}),
        ),
    )
    .await;
    assert_eq!(invalid.result.status, ToolResultStatus::Failed);
    assert_eq!(
        invalid.receipt.exit_reason.as_deref(),
        Some("research_record_invalid")
    );
    let error = invalid.result.output["error"].as_str().unwrap();
    assert!(error.contains("Hint: Use research_get.receipts"), "{error}");
    assert!(
        error.contains("record a missing subject before updating it"),
        "{error}"
    );
    assert_eq!(research_line_count(&project), line_count);
}

#[tokio::test]
async fn exact_dedup_requires_an_explicit_choice_and_merge_preserves_both_records() {
    let (_root, project) = initialized_project("research-dedup-merge");
    let tools = ToolRegistry::for_project(project.root());
    execute(
        &tools,
        "h1",
        "research_update",
        update(
            "event-h1",
            json!({"kind": "record_hypothesis", "record": hypothesis("hyp-1", "First title", "startActivity")}),
        ),
    )
    .await;
    let line_count = research_line_count(&project);
    let conflict = execute(
        &tools,
        "h2-conflict",
        "research_update",
        update(
            "event-h2-conflict",
            json!({"kind": "record_hypothesis", "record": hypothesis("hyp-2", "Similar but independent title", "startActivity")}),
        ),
    )
    .await;
    assert_eq!(conflict["status"], "dedup_conflict");
    assert_eq!(conflict["event_appended"], false);
    assert_eq!(
        conflict["required_resolution"],
        json!(["keep_independent", "records_merged"])
    );
    assert_eq!(research_line_count(&project), line_count);

    execute(
        &tools,
        "h2-independent",
        "research_update",
        update(
            "event-h2-independent",
            json!({
                "kind": "record_hypothesis", "record": hypothesis("hyp-2", "Similar but independent title", "startActivity"),
                "keep_independent": true
            }),
        ),
    )
    .await;
    execute(
        &tools,
        "h3-similar-title",
        "research_update",
        update(
            "event-h3-similar-title",
            json!({"kind": "record_hypothesis", "record": hypothesis("hyp-3", "First title", "sendBroadcast")}),
        ),
    )
    .await;
    execute(
        &tools,
        "merge",
        "research_update",
        update(
            "event-merge",
            json!({
                "kind": "merge_records",
                "from": {"kind": "hypothesis", "id": "hyp-1"},
                "into": {"kind": "hypothesis", "id": "hyp-2"},
                "reason": "same validated root cause"
            }),
        ),
    )
    .await;

    let state = research_state(execute(&tools, "get-merged", "research_get", json!({})).await);
    assert_eq!(state["hypotheses"].as_object().unwrap().len(), 3);
    assert_eq!(
        state["hypotheses"]["hyp-1"]["merged_into"],
        json!({"kind": "hypothesis", "id": "hyp-2"})
    );
    assert_eq!(state["hypotheses"]["hyp-2"]["merged_into"], Value::Null);
}

#[tokio::test]
async fn invalid_persisted_research_is_read_only_and_projects_remain_isolated() {
    for (label, event_type, payload) in [
        (
            "research-unknown-event",
            "research.unknown",
            json!({"research_schema_version": 1, "data": {}}),
        ),
        (
            "research-malformed-payload",
            "research.target_snapshot_recorded",
            json!({"wrong": true}),
        ),
        (
            "research-unsupported-payload",
            "research.target_snapshot_recorded",
            json!({"research_schema_version": 3, "data": {}}),
        ),
        (
            "research-invalid-reference",
            "research.next_action_recorded",
            json!({"research_schema_version": 1, "data": {
                "id": "action-invalid", "research_id": {"kind": "hypothesis", "id": "missing"},
                "priority": 1, "action": "cannot resolve", "status": "pending"
            }}),
        ),
    ] {
        let (_root, project) = initialized_project(label);
        let mut writer = project.open_writer().unwrap();
        let mut store = writer.open_research(None).unwrap();
        EventSink::append_event(
            &mut store,
            &EventEnvelope::current(
                EventId::new("persisted-invalid"),
                ProjectEventStream::Research.stream_id(),
                EventSequence::new(1),
                OccurredAt::new("2026-08-22T12:00:00Z"),
                EventType::new(event_type),
                payload,
            ),
        )
        .unwrap();
        drop(store);
        drop(writer);

        let tools = ToolRegistry::for_project(project.root());
        let get = execute_outcome(&tools, "get-invalid", "research_get", json!({})).await;
        assert_eq!(get.result.status, ToolResultStatus::Failed);
        assert_eq!(
            get.receipt.exit_reason.as_deref(),
            Some("research_read_failed")
        );
        let update = execute_outcome(
            &tools,
            "update-invalid",
            "research_update",
            update(
                "event-should-not-append",
                json!({"kind": "record_hypothesis", "record": hypothesis("hyp-1", "blocked", "sink")}),
            ),
        )
        .await;
        assert_eq!(update.result.status, ToolResultStatus::Failed);
        assert_eq!(research_line_count(&project), 1);
    }

    let (_first_root, first) = initialized_project("research-isolation-a");
    let (_second_root, second) = initialized_project("research-isolation-b");
    for (project, id, target) in [
        (&first, "first", "first.target"),
        (&second, "second", "second.target"),
    ] {
        let tools = ToolRegistry::for_project(project.root());
        let mut record = target_snapshot("snapshot", "1.0.0", "env-a");
        record["target_id"] = json!(target);
        execute(
            &tools,
            id,
            "research_update",
            update(
                &format!("event-{id}"),
                json!({"kind": "record_target_snapshot", "record": record}),
            ),
        )
        .await;
    }
    let first_contents =
        fs::read_to_string(first.root().join(".cyber-agent/research.jsonl")).unwrap();
    let second_contents =
        fs::read_to_string(second.root().join(".cyber-agent/research.jsonl")).unwrap();
    assert!(first_contents.contains("first.target"));
    assert!(!first_contents.contains("second.target"));
    assert!(second_contents.contains("second.target"));
}

#[tokio::test]
async fn persisted_records_cannot_forge_merge_provenance() {
    let (_root, project) = initialized_project("research-forged-merge-provenance");
    let mut record = HypothesisRecord::from_draft(HypothesisDraft {
        id: HypothesisId::new("hypothesis-forged"),
        title: "Forged provenance".to_owned(),
        target_id: "com.example.target".to_owned(),
        attack_surface: "exported activity".to_owned(),
        vulnerability_class: "intent redirection".to_owned(),
        entry_point: "com.example.target/.ShareActivity".to_owned(),
        sensitive_sink: "startActivity".to_owned(),
        rationale: "fixture".to_owned(),
        status: HypothesisStatus::Active,
        priority: 1,
        evidence_links: Vec::new(),
        last_action: None,
        next_action: None,
    })
    .unwrap();
    record.merged_into = Some(MergeRecordRef::Hypothesis(HypothesisId::new(
        "hypothesis-into",
    )));
    let event = ResearchEvent::HypothesisRecorded {
        record,
        keep_independent: false,
    };
    let mut writer = project.open_writer().unwrap();
    let mut store = writer.open_research(None).unwrap();
    EventSink::append_event(
        &mut store,
        &EventEnvelope::current(
            EventId::new("persisted-forged-merge"),
            ProjectEventStream::Research.stream_id(),
            EventSequence::new(1),
            OccurredAt::new("2026-08-22T12:00:00Z"),
            EventType::new(event.event_type().as_str()),
            event.encode_payload().unwrap(),
        ),
    )
    .unwrap();
    drop(store);
    drop(writer);

    let tools = ToolRegistry::for_project(project.root());
    let get = execute_outcome(&tools, "get-forged", "research_get", json!({})).await;
    assert_eq!(get.result.status, ToolResultStatus::Failed);
    assert_eq!(
        get.receipt.exit_reason.as_deref(),
        Some("research_read_failed")
    );
    let update = execute_outcome(
        &tools,
        "update-forged",
        "research_update",
        update(
            "event-must-not-append",
            json!({"kind": "record_hypothesis", "record": hypothesis("hypothesis-new", "blocked", "sink")}),
        ),
    )
    .await;
    assert_eq!(update.result.status, ToolResultStatus::Failed);
    assert_eq!(research_line_count(&project), 1);
}

#[test]
fn research_tool_schemas_are_fixed_and_execution_metadata_is_exact() {
    let root = TemporaryDirectory::new("research-schemas");
    let tools = ToolRegistry::for_project(root.path());
    let resolved = tools
        .resolve_tool_plan(
            &ToolExposurePlan::new([(ToolName::new("research_update"), ToolExposure::Direct)]),
            &Default::default(),
        )
        .unwrap();
    let specs = resolved.advertised_specs();
    let get = specs
        .iter()
        .find(|spec| spec.name.as_str() == "research_get")
        .unwrap();
    let update = specs
        .iter()
        .find(|spec| spec.name.as_str() == "research_update")
        .unwrap();
    assert_eq!(
        (get.effect, get.retry_policy, get.parallelism),
        (
            ToolEffect::ReadOnly,
            ToolRetryPolicy::ReplaySafe,
            ToolParallelism::Parallel
        )
    );
    assert_eq!(
        (update.effect, update.retry_policy, update.parallelism),
        (
            ToolEffect::LocalMutation,
            ToolRetryPolicy::NeverAutoRetry,
            ToolParallelism::ResourceLocked
        )
    );
    assert_eq!(get.input_schema["additionalProperties"], false);
    assert_eq!(
        update.input_schema["required"],
        json!(["event_id", "occurred_at", "operation"])
    );
    assert_eq!(
        update.input_schema["properties"]["operation"]["oneOf"]
            .as_array()
            .unwrap()
            .len(),
        10
    );
    let evidence_ref = &update.input_schema["$defs"]["evidence_ref"];
    assert_eq!(
        evidence_ref["oneOf"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["properties"]["kind"]["const"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["artifact", "experiment", "execution_receipt"]
    );
    for entry in evidence_ref["oneOf"].as_array().unwrap() {
        assert_eq!(entry["required"], json!(["kind", "id"]));
        assert_eq!(entry["additionalProperties"], false);
        assert_eq!(entry["properties"]["id"]["minLength"], 1);
    }
    let evidence_link = &update.input_schema["$defs"]["evidence_link"];
    assert_eq!(
        evidence_link["properties"]["relation"]["enum"],
        json!([
            "supports",
            "contradicts",
            "positive_control",
            "negative_control",
            "environment_control",
            "reachability_proof",
            "prerequisite_proof",
            "impact_proof"
        ])
    );
    assert_eq!(
        evidence_link["properties"]["evidence"]["$ref"],
        "#/$defs/evidence_ref"
    );
    assert_eq!(evidence_link["required"], json!(["relation", "evidence"]));
    assert_eq!(evidence_link["additionalProperties"], false);
    let operation = &update.input_schema["properties"]["operation"]["oneOf"];
    for reference in [
        &operation[1]["properties"]["record"]["properties"]["evidence_links"],
        &operation[2]["properties"]["update"]["properties"]["evidence_links_add"],
        &operation[5]["properties"]["record"]["properties"]["evidence_links"],
        &operation[6]["properties"]["update"]["properties"]["evidence_links_add"],
    ] {
        assert_eq!(reference["$ref"], "#/$defs/links");
        assert_eq!(
            update.input_schema["$defs"]["links"]["items"]["$ref"],
            "#/$defs/evidence_link"
        );
    }
    assert_eq!(
        get.output_schema["required"],
        json!([
            "status",
            "state",
            "stream_event_count",
            "evidence_gap_reports"
        ])
    );
    assert!(update.input_schema["properties"].get("execution").is_some());
    assert_eq!(
        update.resource_key_template.as_deref(),
        Some("file:.cyber-agent/research.jsonl")
    );
}
