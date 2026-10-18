mod support;

use std::fs::{self, FileTimes, OpenOptions};

use echidna::adapters::project_event_store::{Project, ProjectEventStream};
use echidna::adapters::tools::ToolRegistry;
use echidna::domain::context::{ProviderSessionId, ProviderSessionRef, ProviderSessionRole};
use echidna::domain::events::{
    EventEnvelope, EventId, EventSequence, EventType, OccurredAt, StreamId,
};
use echidna::domain::session::SessionEvent;
use echidna::domain::tools::{
    ExecutionId, ExecutionReceipt, ToolCall, ToolCallId, ToolExposure, ToolExposurePlan, ToolName,
    ToolResultStatus,
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
    tools.execute(plan).await.expect("Tool execution completes")
}

fn update(event_id: &str, operation: Value) -> Value {
    json!({
        "event_id": event_id,
        "occurred_at": "2026-09-29T12:00:00Z",
        "operation": operation,
    })
}

fn target_snapshot(id: &str) -> Value {
    json!({
        "id": id,
        "target_id": "receipt.fixture",
        "kind": "source",
        "locator": "repository:receipt.fixture",
        "version": "1.0.0",
        "build_id": "build-1",
        "environment_fingerprint": "environment-1",
        "device_fingerprint": "device-1",
        "captured_at": "2026-09-29T11:59:00Z"
    })
}

fn hypothesis(id: &str) -> Value {
    json!({
        "id": id,
        "title": "Discovered receipt hypothesis",
        "target_id": "receipt.fixture",
        "attack_surface": "parser",
        "vulnerability_class": "memory safety",
        "entry_point": "parse_input",
        "sensitive_sink": "copy_bytes",
        "rationale": "controlled receipt-discovery fixture",
        "status": "active",
        "priority": 10
    })
}

fn experiment(receipt_id: &str, execution_id: &str, suffix: &str) -> Value {
    json!({
        "kind": "record_experiment",
        "experiment": {
            "id": format!("experiment-{suffix}"),
            "hypothesis_id": "hypothesis-receipt",
            "target_snapshot_id": "snapshot-receipt",
            "capsule_id": format!("capsule-{suffix}"),
            "goal": "bind a discovered execution receipt to the exact target",
            "result": "positive"
        },
        "capsule": {
            "id": format!("capsule-{suffix}"),
            "experiment_id": format!("experiment-{suffix}"),
            "target_snapshot_id": "snapshot-receipt",
            "action": format!("Run controlled receipt fixture {suffix}"),
            "arguments": {"case": suffix},
            "tool_version": "fixture-v1",
            "execution_ids": [execution_id],
            "execution_receipt_ids": [receipt_id],
            "toolchain_fingerprint": "fixture-toolchain"
        }
    })
}

fn finding(id: &str, receipt_id: &str) -> Value {
    json!({
        "kind": "record_finding",
        "keep_independent": true,
        "record": {
            "id": id,
            "title": "Receipt-backed finding",
            "status": "candidate",
            "target_id": "receipt.fixture",
            "target_version": "1.0.0",
            "vulnerability_class": "memory safety",
            "entry_point": "parse_input",
            "sensitive_sink": "copy_bytes",
            "target_snapshot_ids": ["snapshot-receipt"],
            "prerequisites": [],
            "impact": "controlled candidate impact",
            "evidence_links": [{
                "relation": "supports",
                "evidence": {"kind": "execution_receipt", "id": receipt_id}
            }]
        }
    })
}

fn append_receipt(project: &Project, session: &str, receipt_id: &str, execution_id: &str) {
    let session_id = StreamId::new(session);
    let stream_id = ProjectEventStream::Session(session_id.clone()).stream_id();
    let mut writer = project.open_writer().unwrap();
    let mut store = writer.open_session(session_id, None).unwrap();
    if store.next_sequence().unwrap() == EventSequence::new(1) {
        let started = SessionEvent::Started {
            primary_provider_session: ProviderSessionRef {
                id: ProviderSessionId::new(format!("primary-{session}")),
                role: ProviderSessionRole::Primary,
                generation: 1,
            },
        }
        .into_envelope(
            EventId::new(format!("started-{session}")),
            stream_id.clone(),
            EventSequence::new(1),
            OccurredAt::new("2026-09-29T11:59:59Z"),
        )
        .unwrap();
        EventSink::append_event(&mut store, &started).unwrap();
    }
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
            EventId::new(receipt_id),
            stream_id,
            store.next_sequence().unwrap(),
            OccurredAt::new("2026-09-29T12:00:00Z"),
        )
        .unwrap();
    EventSink::append_event(&mut store, &envelope).unwrap();
}

fn research_line_count(project: &Project) -> usize {
    fs::read_to_string(project.root().join(".cyber-agent/research.jsonl"))
        .unwrap()
        .lines()
        .count()
}

fn receipt_items(output: &Value) -> &Vec<Value> {
    output["receipt_page"]["items"].as_array().unwrap()
}

#[tokio::test]
async fn empty_get_keeps_the_legacy_shape_and_receipt_pages_are_stable_and_bounded() {
    let (_root, project) = initialized_project("receipt-discovery-pages");
    append_receipt(&project, "session-z", "receipt-z", "execution-z");
    append_receipt(&project, "session-a", "receipt-a", "execution-a");
    append_receipt(&project, "session-m", "receipt-m", "execution-m");
    let tools = ToolRegistry::for_project(project.root());

    let legacy = execute(&tools, "legacy", "research_get", json!({})).await;
    assert_eq!(legacy.result.status, ToolResultStatus::Succeeded);
    assert_eq!(legacy.result.output.as_object().unwrap().len(), 4);
    assert!(legacy.result.output.get("status").is_some());
    assert!(legacy.result.output.get("state").is_some());
    assert!(legacy.result.output.get("stream_event_count").is_some());
    assert!(legacy.result.output.get("evidence_gap_reports").is_some());
    assert!(legacy.result.output.get("receipt_page").is_none());

    let defaults = execute(&tools, "defaults", "research_get", json!({"receipts": {}})).await;
    assert_eq!(defaults.result.status, ToolResultStatus::Succeeded);
    assert_eq!(defaults.result.output["receipt_page"]["total"], 3);
    assert_eq!(defaults.result.output["receipt_page"]["offset"], 0);
    assert_eq!(
        defaults.result.output["receipt_page"]["next_offset"],
        Value::Null
    );
    assert_eq!(
        receipt_items(&defaults.result.output)
            .iter()
            .map(|item| item["receipt_id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["receipt-a", "receipt-m", "receipt-z"]
    );
    assert!(
        receipt_items(&defaults.result.output)
            .iter()
            .all(|item| item["status"] == "succeeded")
    );

    let middle = execute(
        &tools,
        "middle",
        "research_get",
        json!({"receipts": {"offset": 1, "limit": 1}}),
    )
    .await;
    assert_eq!(middle.result.output["receipt_page"]["total"], 3);
    assert_eq!(middle.result.output["receipt_page"]["offset"], 1);
    assert_eq!(middle.result.output["receipt_page"]["next_offset"], 2);
    assert_eq!(
        receipt_items(&middle.result.output)[0]["receipt_id"],
        "receipt-m"
    );

    let filtered = execute(
        &tools,
        "filtered",
        "research_get",
        json!({"receipts": {"execution_id": "execution-z"}}),
    )
    .await;
    assert_eq!(filtered.result.output["receipt_page"]["total"], 1);
    assert_eq!(
        receipt_items(&filtered.result.output)[0]["receipt_id"],
        "receipt-z"
    );
    assert_eq!(
        receipt_items(&filtered.result.output)[0]["execution_id"],
        "execution-z"
    );

    let beyond = execute(
        &tools,
        "beyond",
        "research_get",
        json!({"receipts": {"offset": 99, "limit": 2}}),
    )
    .await;
    assert_eq!(beyond.result.output["receipt_page"]["total"], 3);
    assert_eq!(beyond.result.output["receipt_page"]["offset"], 99);
    assert_eq!(
        beyond.result.output["receipt_page"]["next_offset"],
        Value::Null
    );
    assert!(receipt_items(&beyond.result.output).is_empty());
}

#[test]
fn receipt_query_schema_is_precise_and_invalid_ranges_fail_before_execution() {
    let root = TemporaryDirectory::new("receipt-discovery-schema");
    let tools = ToolRegistry::for_project(root.path());
    let resolved = tools
        .resolve_tool_plan(
            &ToolExposurePlan::new([(ToolName::new("research_get"), ToolExposure::Direct)]),
            &Default::default(),
        )
        .unwrap();
    let spec = resolved
        .advertised_specs()
        .iter()
        .find(|spec| spec.name.as_str() == "research_get")
        .unwrap();
    assert_eq!(spec.version, "2");
    assert_eq!(spec.input_schema["additionalProperties"], false);
    let receipts = &spec.input_schema["properties"]["receipts"];
    assert_eq!(receipts["additionalProperties"], false);
    assert_eq!(receipts["properties"]["offset"]["minimum"], 0);
    assert_eq!(receipts["properties"]["limit"]["minimum"], 1);
    assert_eq!(receipts["properties"]["limit"]["maximum"], 32);
    assert_eq!(receipts["properties"]["execution_id"]["minLength"], 1);
    assert_eq!(
        spec.output_schema["properties"]["receipt_page"]["$ref"],
        "#/$defs/receipt_page"
    );
    let page = &spec.output_schema["$defs"]["receipt_page"];
    assert_eq!(page["additionalProperties"], false);
    assert_eq!(
        page["required"],
        json!(["items", "total", "offset", "next_offset"])
    );
    assert_eq!(
        page["properties"]["items"]["items"]["required"],
        json!(["receipt_id", "execution_id", "status"])
    );
    assert_eq!(
        page["properties"]["items"]["items"]["additionalProperties"],
        false
    );

    for (case, arguments) in [
        ("unknown-root", json!({"unknown": true})),
        ("null-query", json!({"receipts": null})),
        ("unknown-query", json!({"receipts": {"unknown": true}})),
        ("negative-offset", json!({"receipts": {"offset": -1}})),
        (
            "overflow-offset",
            json!({"receipts": {"offset": usize::MAX, "limit": 2}}),
        ),
        ("zero-limit", json!({"receipts": {"limit": 0}})),
        ("large-limit", json!({"receipts": {"limit": 33}})),
        (
            "null-execution",
            json!({"receipts": {"execution_id": null}}),
        ),
        ("empty-execution", json!({"receipts": {"execution_id": ""}})),
    ] {
        assert!(
            tools
                .plan(
                    ExecutionId::new(format!("execution-{case}")),
                    call(case, "research_get", arguments),
                )
                .is_err(),
            "{case} must fail during planning"
        );
    }
}

#[tokio::test]
async fn a_discovered_receipt_can_be_bound_explicitly_and_support_a_finding() {
    let (_root, project) = initialized_project("receipt-discovery-evidence");
    append_receipt(&project, "session-real", "receipt-real", "execution-real");
    let tools = ToolRegistry::for_project(project.root());

    let discovered = execute(
        &tools,
        "discover-real",
        "research_get",
        json!({"receipts": {"execution_id": "execution-real"}}),
    )
    .await;
    assert_eq!(discovered.result.status, ToolResultStatus::Succeeded);
    let item = &receipt_items(&discovered.result.output)[0];
    let receipt_id = item["receipt_id"].as_str().unwrap();
    let execution_id = item["execution_id"].as_str().unwrap();
    assert_eq!(
        (receipt_id, execution_id),
        ("receipt-real", "execution-real")
    );

    for (id, operation) in [
        (
            "target",
            json!({"kind": "record_target_snapshot", "record": target_snapshot("snapshot-receipt")}),
        ),
        (
            "hypothesis",
            json!({"kind": "record_hypothesis", "record": hypothesis("hypothesis-receipt")}),
        ),
        (
            "experiment",
            experiment(receipt_id, execution_id, "receipt"),
        ),
        ("finding", finding("finding-receipt", receipt_id)),
    ] {
        let outcome = execute(
            &tools,
            id,
            "research_update",
            update(&format!("event-{id}"), operation),
        )
        .await;
        assert_eq!(
            outcome.result.status,
            ToolResultStatus::Succeeded,
            "{id}: {}",
            outcome.result.output
        );
    }

    let state = execute(&tools, "get-final", "research_get", json!({})).await;
    assert_eq!(
        state.result.output["state"]["findings"]["finding-receipt"]["evidence_links"],
        json!([{
            "relation": "supports",
            "evidence": {"kind": "execution_receipt", "id": "receipt-real"}
        }])
    );
}

#[tokio::test]
async fn invented_cross_project_and_unbound_receipts_reject_without_research_append() {
    let (_root, project) = initialized_project("receipt-discovery-rejections");
    append_receipt(
        &project,
        "session-local",
        "receipt-unbound",
        "execution-unbound",
    );
    let tools = ToolRegistry::for_project(project.root());
    for (id, operation) in [
        (
            "target",
            json!({"kind": "record_target_snapshot", "record": target_snapshot("snapshot-receipt")}),
        ),
        (
            "hypothesis",
            json!({"kind": "record_hypothesis", "record": hypothesis("hypothesis-receipt")}),
        ),
    ] {
        let outcome = execute(
            &tools,
            id,
            "research_update",
            update(&format!("event-{id}"), operation),
        )
        .await;
        assert_eq!(outcome.result.status, ToolResultStatus::Succeeded);
    }

    let (_foreign_root, foreign_project) = initialized_project("receipt-discovery-foreign");
    append_receipt(
        &foreign_project,
        "session-foreign",
        "receipt-foreign",
        "execution-foreign",
    );

    for (case, operation) in [
        (
            "path-is-not-receipt",
            experiment("/tmp/fixture/output.json", "execution-unbound", "path"),
        ),
        (
            "execution-id-is-not-receipt",
            experiment("execution-unbound", "execution-unbound", "execution-id"),
        ),
        (
            "foreign-receipt",
            experiment("receipt-foreign", "execution-foreign", "foreign"),
        ),
        (
            "unbound-finding",
            finding("finding-unbound", "receipt-unbound"),
        ),
    ] {
        let before = research_line_count(&project);
        let outcome = execute(
            &tools,
            case,
            "research_update",
            update(&format!("event-{case}"), operation),
        )
        .await;
        assert_eq!(outcome.result.status, ToolResultStatus::Failed, "{case}");
        assert_eq!(
            outcome.receipt.exit_reason.as_deref(),
            Some("research_transition_rejected"),
            "{case}"
        );
        assert_eq!(research_line_count(&project), before, "{case}");
    }
}

#[tokio::test]
async fn unknown_session_events_reject_discovery_and_updates_without_append() {
    let (_root, project) = initialized_project("receipt-discovery-unknown-session");
    append_receipt(
        &project,
        "session-invalid",
        "receipt-before-unknown",
        "execution-before-unknown",
    );
    let session_id = StreamId::new("session-invalid");
    let mut writer = project.open_writer().unwrap();
    let mut store = writer.open_session(session_id.clone(), None).unwrap();
    let unknown = EventEnvelope::current(
        EventId::new("event-unknown-session"),
        ProjectEventStream::Session(session_id).stream_id(),
        store.next_sequence().unwrap(),
        OccurredAt::new("2026-09-29T12:01:00Z"),
        EventType::new("session.future_unknown"),
        json!({"schema_version": 1}),
    );
    EventSink::append_event(&mut store, &unknown).unwrap();
    drop(store);
    drop(writer);

    let tools = ToolRegistry::for_project(project.root());
    let get = execute(
        &tools,
        "get-invalid-session",
        "research_get",
        json!({"receipts": {}}),
    )
    .await;
    assert_eq!(get.result.status, ToolResultStatus::Failed);
    assert_eq!(
        get.receipt.exit_reason.as_deref(),
        Some("research_read_failed")
    );

    let before = research_line_count(&project);
    let update_outcome = execute(
        &tools,
        "update-invalid-session",
        "research_update",
        update(
            "event-must-not-append",
            json!({"kind": "record_target_snapshot", "record": target_snapshot("snapshot-receipt")}),
        ),
    )
    .await;
    assert_eq!(update_outcome.result.status, ToolResultStatus::Failed);
    assert_eq!(
        update_outcome.receipt.exit_reason.as_deref(),
        Some("research_receipts_invalid")
    );
    assert_eq!(research_line_count(&project), before);
}

#[tokio::test]
async fn same_size_session_tampering_with_preserved_mtime_is_revalidated_and_rejected() {
    let (_root, project) = initialized_project("receipt-discovery-byte-freshness");
    append_receipt(
        &project,
        "session-tamper",
        "receipt-tamper",
        "execution-good",
    );
    let tools = ToolRegistry::for_project(project.root());
    let warm = execute(
        &tools,
        "warm",
        "research_get",
        json!({"receipts": {"execution_id": "execution-good"}}),
    )
    .await;
    assert_eq!(warm.result.output["receipt_page"]["total"], 1);

    let path = project
        .root()
        .join(".cyber-agent/sessions/session-tamper/events.jsonl");
    let metadata = fs::metadata(&path).unwrap();
    let modified = metadata.modified().unwrap();
    let mut bytes = fs::read(&path).unwrap();
    let before_len = bytes.len();
    let from = b"\"schema_version\":2";
    let to = b"\"schema_version\":9";
    assert_eq!(from.len(), to.len());
    let mut replacements = 0;
    for index in 0..=bytes.len() - from.len() {
        if bytes[index..].starts_with(from) {
            bytes[index..index + from.len()].copy_from_slice(to);
            replacements += 1;
        }
    }
    assert!(replacements >= 1);
    fs::write(&path, &bytes).unwrap();
    assert_eq!(fs::metadata(&path).unwrap().len(), before_len as u64);
    OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(FileTimes::new().set_modified(modified))
        .unwrap();

    let get = execute(
        &tools,
        "get-after-tamper",
        "research_get",
        json!({"receipts": {}}),
    )
    .await;
    assert_eq!(get.result.status, ToolResultStatus::Failed);
    assert_eq!(
        get.receipt.exit_reason.as_deref(),
        Some("research_read_failed")
    );

    let before = research_line_count(&project);
    let update_outcome = execute(
        &tools,
        "update-after-tamper",
        "research_update",
        update(
            "event-must-not-append-after-tamper",
            json!({"kind": "record_target_snapshot", "record": target_snapshot("snapshot-receipt")}),
        ),
    )
    .await;
    assert_eq!(update_outcome.result.status, ToolResultStatus::Failed);
    assert_eq!(
        update_outcome.receipt.exit_reason.as_deref(),
        Some("research_receipts_invalid")
    );
    assert_eq!(research_line_count(&project), before);
}
