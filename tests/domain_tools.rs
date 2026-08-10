use cyber_agent::domain::tools::{
    ExecutionId, ToolCachePolicy, ToolCall, ToolCallId, ToolDeterminism, ToolEffect,
    ToolExecutionOutcome, ToolExecutionPlan, ToolExposure, ToolName, ToolOutputPolicy,
    ToolParallelism, ToolResultStatus, ToolRetryPolicy, ToolSpec,
};
use serde_json::{Map, Value, json};

fn spec() -> ToolSpec {
    ToolSpec {
        name: ToolName::new("read_file"),
        version: "1".to_owned(),
        description: "Read one file.".to_owned(),
        input_schema: json!({"type": "object"}),
        output_schema: json!({"type": "object"}),
        exposure: ToolExposure::Direct,
        parallelism: ToolParallelism::Parallel,
        resource_key_template: Some("file:{path}".to_owned()),
        effect: ToolEffect::ReadOnly,
        retry_policy: ToolRetryPolicy::ReplaySafe,
        determinism: ToolDeterminism::EnvironmentDependent,
        cache_policy: ToolCachePolicy::None,
        timeout_ms: 10_000,
        output_policy: ToolOutputPolicy { max_bytes: 1024 },
    }
}

fn plan(arguments: Value) -> ToolExecutionPlan {
    ToolExecutionPlan::new(
        ExecutionId::new("execution-1"),
        ToolCall::new(
            ToolCallId::new("call-1"),
            ToolName::new("read_file"),
            arguments,
        ),
        &spec(),
        vec!["file:/project/Cargo.toml".to_owned()],
    )
    .expect("the fixture plan must be fingerprintable")
}

#[test]
fn plan_and_input_fingerprints_are_stable_for_semantically_equal_json() {
    let mut first = Map::new();
    first.insert("path".to_owned(), json!("Cargo.toml"));
    first.insert("start_line".to_owned(), json!(1));
    let mut second = Map::new();
    second.insert("start_line".to_owned(), json!(1));
    second.insert("path".to_owned(), json!("Cargo.toml"));

    let first = plan(Value::Object(first));
    let second = plan(Value::Object(second));

    assert_eq!(first.input_fingerprint, second.input_fingerprint);
    assert_eq!(first.plan_fingerprint, second.plan_fingerprint);

    let changed = plan(json!({"path": "src/lib.rs", "start_line": 1}));
    assert_ne!(first.input_fingerprint, changed.input_fingerprint);
    assert_ne!(first.plan_fingerprint, changed.plan_fingerprint);
}

#[test]
fn outcome_constructors_keep_receipt_and_result_statuses_consistent() {
    let succeeded = ToolExecutionOutcome::succeeded(
        plan(json!({"path": "Cargo.toml"})),
        json!({
            "content": "fixture"
        }),
    );
    assert_eq!(succeeded.receipt.status, ToolResultStatus::Succeeded);
    assert_eq!(succeeded.result.status, ToolResultStatus::Succeeded);
    assert_eq!(succeeded.receipt.exit_reason, None);
    assert_eq!(succeeded.receipt.plan_fingerprint.len(), 64);

    let failed = ToolExecutionOutcome::failed(
        plan(json!({"path": "missing"})),
        "read_failed",
        json!({"error": "missing"}),
    );
    assert_eq!(failed.receipt.status, ToolResultStatus::Failed);
    assert_eq!(failed.result.status, ToolResultStatus::Failed);
    assert_eq!(failed.receipt.exit_reason.as_deref(), Some("read_failed"));

    let cancelled =
        ToolExecutionOutcome::cancelled(plan(json!({"path": "slow"})), "agent_cancelled");
    assert_eq!(cancelled.receipt.status, ToolResultStatus::Cancelled);
    assert_eq!(cancelled.result.status, ToolResultStatus::Cancelled);
    assert_eq!(
        cancelled.receipt.exit_reason.as_deref(),
        Some("agent_cancelled")
    );
}
