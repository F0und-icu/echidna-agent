#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use cyber_agent::adapters::tools::ToolRegistry;
use cyber_agent::domain::tools::{
    ExecutionId, ResourceKey, ToolCall, ToolCallId, ToolExecutionOutcome, ToolExecutionPlan,
    ToolName, ToolParallelism, ToolResultStatus, ToolRetryPolicy,
};
use cyber_agent::ports::tool::{ToolExecutor, ToolExecutorError};
use serde_json::{Value, json};

static NEXT_PROJECT_ID: AtomicU64 = AtomicU64::new(1);

struct TestProject {
    root: PathBuf,
}

impl TestProject {
    fn new(label: &str) -> Self {
        let id = NEXT_PROJECT_ID.fetch_add(1, Ordering::Relaxed);
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/m1c-s3-tests")
            .join(format!("{label}-{}-{id}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        Self { root }
    }
}

impl Drop for TestProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn plan(tools: &impl ToolExecutor, id: &str, name: &str, arguments: Value) -> ToolExecutionPlan {
    tools
        .plan(
            ExecutionId::new(format!("execution-{id}")),
            ToolCall::new(ToolCallId::new(id), ToolName::new(name), arguments),
        )
        .unwrap()
}

async fn poll_until_terminal(
    tools: &ToolRegistry,
    session_id: &str,
    offset: u64,
) -> ToolExecutionOutcome {
    for attempt in 0..100 {
        let outcome = tools
            .execute(plan(
                tools,
                &format!("poll-{attempt}"),
                "poll_pty",
                json!({"pty_session_id": session_id, "offset": offset}),
            ))
            .await;
        if outcome.result.output["status"] != "running"
            && outcome.result.output["status"] != "stopping"
        {
            return outcome;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("PTY session did not reach a terminal state")
}

#[tokio::test]
async fn interactive_echo_uses_caller_offsets_and_stable_completion_linkage() {
    let project = TestProject::new("interactive-echo");
    let tools = ToolRegistry::for_project(&project.root);
    let start = tools
        .execute(plan(
            &tools,
            "start-echo",
            "start_pty",
            json!({
                "argv": ["sh", "-c", "read line; printf 'echo:%s' \"$line\""],
                "rows": 30,
                "cols": 90,
            }),
        ))
        .await;
    assert_eq!(start.result.status, ToolResultStatus::Succeeded);
    let session_id = start.result.output["pty_session_id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(start.result.output["rows"], 30);
    assert_eq!(start.result.output["cols"], 90);

    let input = tools
        .execute(plan(
            &tools,
            "input-echo",
            "input_pty",
            json!({"pty_session_id": session_id, "input": "fixture\n"}),
        ))
        .await;
    assert_eq!(input.result.status, ToolResultStatus::Succeeded);
    assert_eq!(input.result.output["bytes_written"], 8);

    let completed = poll_until_terminal(&tools, &session_id, 0).await;
    assert_eq!(completed.result.status, ToolResultStatus::Succeeded);
    assert_eq!(completed.result.output["status"], "completed");
    assert!(
        completed.result.output["output"]["data"]
            .as_str()
            .unwrap()
            .contains("echo:fixture")
    );
    assert_eq!(
        completed.result.output["completion"]["start_execution_id"],
        "execution-start-echo"
    );
    assert_eq!(
        completed.result.output["completion"]["start_plan_fingerprint"],
        start.receipt.plan_fingerprint
    );
    let next_offset = completed.result.output["output"]["next_offset"]
        .as_u64()
        .unwrap();
    let repeated = tools
        .execute(plan(
            &tools,
            "poll-echo-repeated",
            "poll_pty",
            json!({"pty_session_id": session_id, "offset": next_offset}),
        ))
        .await;
    assert_eq!(repeated.result.status, ToolResultStatus::Succeeded);
    assert_eq!(repeated.result.output["output"]["data"], "");
    assert_eq!(
        repeated.result.output["completion"],
        completed.result.output["completion"]
    );
}

#[tokio::test]
async fn resize_reaches_the_interactive_child() {
    let project = TestProject::new("resize");
    let tools = ToolRegistry::for_project(&project.root);
    let start = tools
        .execute(plan(
            &tools,
            "start-resize",
            "start_pty",
            json!({"argv": ["sh", "-c", "read line; stty size"]}),
        ))
        .await;
    let session_id = start.result.output["pty_session_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let resize = tools
        .execute(plan(
            &tools,
            "resize",
            "resize_pty",
            json!({"pty_session_id": session_id, "rows": 40, "cols": 100}),
        ))
        .await;
    assert_eq!(resize.result.status, ToolResultStatus::Succeeded);
    assert_eq!(resize.result.output["rows"], 40);
    assert_eq!(resize.result.output["cols"], 100);
    let input = tools
        .execute(plan(
            &tools,
            "input-resize",
            "input_pty",
            json!({"pty_session_id": session_id, "input": "go\n"}),
        ))
        .await;
    assert_eq!(input.result.status, ToolResultStatus::Succeeded);
    let completed = poll_until_terminal(&tools, &session_id, 0).await;
    assert!(
        completed.result.output["output"]["data"]
            .as_str()
            .unwrap()
            .contains("40 100")
    );
}

#[tokio::test]
async fn poll_preserves_utf8_boundaries_and_binary_byte_offsets() {
    let project = TestProject::new("offsets");
    let tools = ToolRegistry::for_project(&project.root);
    let utf8 = tools
        .execute(plan(
            &tools,
            "start-utf8",
            "start_pty",
            json!({
                "argv": ["sh", "-c", "printf '\\303'; sleep 0.1; printf '\\251'"],
            }),
        ))
        .await;
    let utf8_id = utf8.result.output["pty_session_id"]
        .as_str()
        .unwrap()
        .to_owned();
    tokio::time::sleep(Duration::from_millis(20)).await;
    let partial = tools
        .execute(plan(
            &tools,
            "poll-utf8-partial",
            "poll_pty",
            json!({"pty_session_id": utf8_id, "offset": 0}),
        ))
        .await;
    assert_eq!(partial.result.status, ToolResultStatus::Succeeded);
    assert_eq!(partial.result.output["output"]["data"], "");
    assert_eq!(partial.result.output["output"]["next_offset"], 0);
    let completed = poll_until_terminal(&tools, &utf8_id, 0).await;
    assert_eq!(completed.result.output["output"]["data"], "é");
    let continuation = tools
        .execute(plan(
            &tools,
            "poll-utf8-continuation",
            "poll_pty",
            json!({"pty_session_id": utf8_id, "offset": 1}),
        ))
        .await;
    assert_eq!(continuation.result.status, ToolResultStatus::Failed);
    assert_eq!(
        continuation.result.output["output"]["error"],
        "offset_not_utf8_boundary"
    );

    let binary = tools
        .execute(plan(
            &tools,
            "start-binary",
            "start_pty",
            json!({"argv": ["sh", "-c", "printf '\\200\\277'"]}),
        ))
        .await;
    let binary_id = binary.result.output["pty_session_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let completed = poll_until_terminal(&tools, &binary_id, 0).await;
    for (index, offset) in [0, 1].into_iter().enumerate() {
        let polled = tools
            .execute(plan(
                &tools,
                &format!("poll-binary-{index}"),
                "poll_pty",
                json!({"pty_session_id": binary_id, "offset": offset}),
            ))
            .await;
        assert_eq!(polled.result.status, ToolResultStatus::Succeeded);
        assert_eq!(polled.result.output["output"]["offset"], offset);
        assert_ne!(
            polled.result.output["output"]["error"],
            "offset_not_utf8_boundary"
        );
    }
    assert_eq!(completed.result.output["status"], "completed");
}

#[tokio::test]
async fn stop_racing_natural_completion_publishes_once_after_reaping_the_child() {
    let project = TestProject::new("stop-race");
    let tools = ToolRegistry::for_project(&project.root);
    let raced = tools
        .execute(plan(
            &tools,
            "start-race",
            "start_pty",
            json!({"argv": ["sh", "-c", "exit 0"]}),
        ))
        .await;
    let raced_id = raced.result.output["pty_session_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let raced_stop = tokio::time::timeout(
        Duration::from_millis(500),
        tools.execute(plan(
            &tools,
            "stop-race",
            "stop_pty",
            json!({"pty_session_id": raced_id}),
        )),
    )
    .await
    .expect("stop racing natural completion must wait for one terminal publication");
    assert_eq!(raced_stop.result.status, ToolResultStatus::Succeeded);
    assert!(matches!(
        raced_stop.result.output["status"].as_str(),
        Some("completed") | Some("cancelled")
    ));
    assert!(raced_stop.result.output["completion"].is_object());

    let stopped = tools
        .execute(plan(
            &tools,
            "start-stop",
            "start_pty",
            json!({"argv": ["sh", "-c", "sleep 5"]}),
        ))
        .await;
    let stopped_id = stopped.result.output["pty_session_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let stopped = tokio::time::timeout(
        Duration::from_secs(1),
        tools.execute(plan(
            &tools,
            "stop",
            "stop_pty",
            json!({"pty_session_id": stopped_id}),
        )),
    )
    .await
    .expect("stop must reap the direct PTY child before terminal publication");
    assert_eq!(stopped.result.status, ToolResultStatus::Succeeded);
    assert_eq!(stopped.result.output["status"], "cancelled");
    let repeated = tools
        .execute(plan(
            &tools,
            "stop-repeated",
            "stop_pty",
            json!({"pty_session_id": stopped_id}),
        ))
        .await;
    assert_eq!(repeated.result.output["already_terminal"], true);
    assert_eq!(
        repeated.result.output["completion"],
        stopped.result.output["completion"]
    );
}

#[tokio::test]
async fn terminal_publication_seals_output_from_a_descendant_held_pty() {
    let project = TestProject::new("descendant-output-seal");
    let tools = ToolRegistry::for_project(&project.root);
    let start = tools
        .execute(plan(
            &tools,
            "start-descendant",
            "start_pty",
            json!({
                "argv": [
                    "sh",
                    "-c",
                    "(trap '' HUP; sleep 0.3; printf late) & printf ready"
                ],
            }),
        ))
        .await;
    let session_id = start.result.output["pty_session_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let terminal = poll_until_terminal(&tools, &session_id, 0).await;
    assert_eq!(terminal.result.output["status"], "completed");
    assert!(
        terminal.result.output["output"]["data"]
            .as_str()
            .unwrap()
            .contains("ready")
    );
    let terminal_offset = terminal.result.output["output"]["next_offset"]
        .as_u64()
        .unwrap();

    tokio::time::sleep(Duration::from_millis(350)).await;
    let after_descendant = tools
        .execute(plan(
            &tools,
            "poll-after-descendant",
            "poll_pty",
            json!({"pty_session_id": session_id, "offset": terminal_offset}),
        ))
        .await;
    assert_eq!(after_descendant.result.output["output"]["data"], "");
    assert_eq!(
        after_descendant.result.output["output"]["next_offset"],
        terminal_offset
    );
    assert_eq!(
        after_descendant.result.output["completion"],
        terminal.result.output["completion"]
    );
}

#[tokio::test]
async fn active_job_and_pty_leases_share_conflict_and_release_rules() {
    let project = TestProject::new("shared-leases");
    let tools = ToolRegistry::for_project(&project.root);
    let resource = json!([{"kind": "device", "serial": "fixture-device"}]);
    let job = tools
        .execute(plan(
            &tools,
            "start-job",
            "start_job",
            json!({"argv": ["sh", "-c", "sleep 5"], "resources": resource}),
        ))
        .await;
    let job_id = job.result.output["job_id"].as_str().unwrap().to_owned();
    let blocked_pty = tools
        .execute(plan(
            &tools,
            "blocked-pty",
            "start_pty",
            json!({
                "argv": ["sh", "-c", "printf spawned > blocked-pty-ran"],
                "resources": resource
            }),
        ))
        .await;
    assert_eq!(blocked_pty.result.status, ToolResultStatus::Failed);
    assert_eq!(
        blocked_pty.receipt.exit_reason.as_deref(),
        Some("job_resource_busy")
    );
    assert_eq!(blocked_pty.result.output["conflicting_job_id"], job_id);
    assert!(!project.root.join("blocked-pty-ran").exists());
    let stopped_job = tools
        .execute(plan(
            &tools,
            "stop-job",
            "stop_job",
            json!({"job_id": job_id}),
        ))
        .await;
    assert_eq!(stopped_job.result.output["status"], "cancelled");

    let pty = tools
        .execute(plan(
            &tools,
            "start-pty",
            "start_pty",
            json!({"argv": ["sh", "-c", "sleep 5"], "resources": resource}),
        ))
        .await;
    assert_eq!(pty.result.status, ToolResultStatus::Succeeded);
    let session_id = pty.result.output["pty_session_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let blocked_job = tools
        .execute(plan(
            &tools,
            "blocked-job",
            "start_job",
            json!({
                "argv": ["sh", "-c", "printf spawned > blocked-job-ran"],
                "resources": resource
            }),
        ))
        .await;
    assert_eq!(blocked_job.result.status, ToolResultStatus::Failed);
    assert_eq!(
        blocked_job.receipt.exit_reason.as_deref(),
        Some("pty_resource_busy")
    );
    assert_eq!(
        blocked_job.result.output["conflicting_pty_session_id"],
        session_id
    );
    assert!(!project.root.join("blocked-job-ran").exists());

    for name in ["poll_pty", "input_pty", "resize_pty", "stop_pty"] {
        let arguments = match name {
            "poll_pty" => json!({"pty_session_id": session_id}),
            "input_pty" => json!({"pty_session_id": session_id, "input": "x"}),
            "resize_pty" => json!({"pty_session_id": session_id, "rows": 24, "cols": 80}),
            "stop_pty" => json!({"pty_session_id": session_id}),
            _ => unreachable!(),
        };
        let control_plan = plan(&tools, &format!("same-session-{name}"), name, arguments);
        assert_eq!(control_plan.parallelism, ToolParallelism::ResourceLocked);
        assert_eq!(
            control_plan.resolved_resource_keys,
            vec![ResourceKey::pty(session_id.clone())]
        );
    }

    let stopped_pty = tools
        .execute(plan(
            &tools,
            "stop-pty",
            "stop_pty",
            json!({"pty_session_id": session_id}),
        ))
        .await;
    assert_eq!(stopped_pty.result.output["status"], "cancelled");
    let released_job = tools
        .execute(plan(
            &tools,
            "released-job",
            "start_job",
            json!({"argv": ["sh", "-c", "exit 0"], "resources": resource}),
        ))
        .await;
    assert_eq!(released_job.result.status, ToolResultStatus::Succeeded);
}

#[test]
fn pty_schemas_and_control_ids_are_exact_and_model_facing() {
    let project = TestProject::new("schemas");
    let tools = ToolRegistry::for_project(&project.root);
    let specs = tools.specs();
    for name in [
        "start_pty",
        "poll_pty",
        "input_pty",
        "resize_pty",
        "stop_pty",
    ] {
        assert!(specs.iter().any(|spec| spec.name.as_str() == name));
    }
    let start = specs
        .iter()
        .find(|spec| spec.name.as_str() == "start_pty")
        .unwrap();
    assert_eq!(start.retry_policy, ToolRetryPolicy::DedupeByKey);
    assert_eq!(start.input_schema["required"], json!(["argv"]));
    assert_eq!(start.input_schema["properties"]["rows"]["minimum"], 1);
    assert_eq!(start.input_schema["properties"]["cols"]["maximum"], 65535);
    let input = specs
        .iter()
        .find(|spec| spec.name.as_str() == "input_pty")
        .unwrap();
    assert_eq!(
        input.input_schema["required"],
        json!(["pty_session_id", "input"])
    );
    assert!(input.input_schema["properties"].get("close").is_none());
    let poll = specs
        .iter()
        .find(|spec| spec.name.as_str() == "poll_pty")
        .unwrap();
    assert_eq!(poll.input_schema["properties"]["offset"]["minimum"], 0);

    for (index, (name, arguments)) in [
        ("poll_pty", json!({"pty_session_id": ""})),
        ("input_pty", json!({"pty_session_id": "", "input": "x"})),
        (
            "resize_pty",
            json!({"pty_session_id": "", "rows": 24, "cols": 80}),
        ),
        (
            "resize_pty",
            json!({"pty_session_id": "pty:1", "rows": 0, "cols": 80}),
        ),
        ("stop_pty", json!({"pty_session_id": ""})),
    ]
    .into_iter()
    .enumerate()
    {
        assert!(matches!(
            tools.plan(
                ExecutionId::new(format!("execution-invalid-{index}")),
                ToolCall::new(
                    ToolCallId::new(format!("invalid-{index}")),
                    ToolName::new(name),
                    arguments
                ),
            ),
            Err(ToolExecutorError::InvalidArguments { .. })
        ));
    }
}
