#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use echidna::adapters::tools::ToolRegistry;
use echidna::domain::tools::{
    ExecutionId, ToolCall, ToolCallId, ToolExecutionOutcome, ToolExecutionPlan, ToolName,
    ToolResultStatus,
};
use echidna::ports::tool::{ToolExecutor, ToolExecutorError};
use serde_json::{Value, json};

static NEXT_PROJECT_ID: AtomicU64 = AtomicU64::new(1);

#[tokio::test]
async fn global_background_lease_preserves_discovery_and_model_bound_stop() {
    use echidna::domain::research_control::ResearchPhase;
    use echidna::domain::tools::DeferredActivationSet;
    for (start_name, stop_name, id_field) in [
        ("start_job", "stop_job", "job_id"),
        ("start_pty", "stop_pty", "pty_session_id"),
    ] {
        let project = TestProject::new(start_name);
        let tools = ToolRegistry::for_project(&project.root);
        let mut arguments = json!({"argv":["sh","-c","read line"]});
        if start_name == "start_job" {
            arguments["timeout_ms"] = json!(10000);
        }
        let started = tools
            .execute(plan(&tools, "start", start_name, arguments))
            .await
            .expect("Tool execution completes");
        assert_eq!(started.result.status, ToolResultStatus::Succeeded);
        let handle = started.result.output[id_field].as_str().unwrap();
        let blocked = tools
            .execute(plan(
                &tools,
                "blocked",
                "run_command",
                json!({"argv":["sh","-c","printf forbidden > forbidden"]}),
            ))
            .await
            .expect("Tool execution completes");
        assert_eq!(blocked.result.status, ToolResultStatus::Failed);
        assert!(!project.root.join("forbidden").exists());

        let exposure = ResearchPhase::Mapping.tool_exposure();
        let mut activation = DeferredActivationSet::default();
        let before = tools.resolve_tool_plan(&exposure, &activation).unwrap();
        assert!(before.advertised_spec(&ToolName::new(stop_name)).is_none());
        let call = ToolCall::new(
            ToolCallId::new("discover"),
            ToolName::new("tool_search"),
            json!({"query":stop_name}),
        );
        let search = tools
            .execute(
                tools
                    .plan_model_call(&before, ExecutionId::new("search"), call)
                    .unwrap(),
            )
            .await
            .expect("Tool execution completes");
        assert_eq!(search.result.status, ToolResultStatus::Succeeded);
        activation
            .apply_search_result(&search.result, &before)
            .unwrap();
        let after = tools.resolve_tool_plan(&exposure, &activation).unwrap();
        let stop_call = ToolCall::new(
            ToolCallId::new("stop"),
            ToolName::new(stop_name),
            json!({(id_field):handle}),
        );
        assert!(
            tools
                .plan_model_call(&before, ExecutionId::new("early"), stop_call.clone())
                .is_err()
        );
        let stopped = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            tools.execute(
                tools
                    .plan_model_call(&after, ExecutionId::new("stop"), stop_call)
                    .unwrap(),
            ),
        )
        .await
        .expect("stop must not wait for the Job deadline")
        .expect("Tool execution completes");
        assert_eq!(stopped.result.status, ToolResultStatus::Succeeded);
        assert_eq!(stopped.result.output["status"], "cancelled");
        let recovered = tools
            .execute(plan(
                &tools,
                "after-stop",
                "run_command",
                json!({"argv":["sh","-c","printf recovered"]}),
            ))
            .await
            .expect("Tool execution completes");
        assert_eq!(recovered.result.status, ToolResultStatus::Succeeded);
        let next_turn = tools
            .resolve_tool_plan(&exposure, &DeferredActivationSet::default())
            .unwrap();
        assert!(
            next_turn
                .advertised_spec(&ToolName::new(stop_name))
                .is_none()
        );
    }
}

#[test]
fn ctf_r3a_fresh_processes_have_distinct_job_and_pty_handles() {
    let probe = || {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "runtime_handle_subprocess_probe", "--nocapture"])
            .env("CYBER_AGENT_TEST_HANDLE_PROBE", "1")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while child.try_wait().unwrap().is_none() {
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("runtime identity probe exceeded its deadline");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .find_map(|line| {
                line.split_once("RUNTIME_HANDLES=")
                    .map(|(_, value)| value.to_owned())
            })
            .expect("child probe must return the actual issued handles")
    };
    let first = probe();
    let second = probe();
    let (first_job, first_pty) = first.split_once('|').unwrap();
    let (second_job, second_pty) = second.split_once('|').unwrap();
    assert!(first_job.starts_with("job:v2:"));
    assert!(first_pty.starts_with("pty:v2:"));
    assert_ne!(first_job, second_job);
    assert_ne!(first_pty, second_pty);
}

#[tokio::test]
async fn runtime_handle_subprocess_probe() {
    if std::env::var_os("CYBER_AGENT_TEST_HANDLE_PROBE").is_none() {
        return;
    }
    let project = TestProject::new("handle-child");
    let tools = ToolRegistry::for_project(&project.root);
    let job = tools
        .execute(plan(
            &tools,
            "job",
            "start_job",
            json!({"argv":["sh","-c","exit 0"]}),
        ))
        .await
        .expect("Tool execution completes");
    let job_id = job.result.output["job_id"].as_str().unwrap();
    poll_until_terminal(&tools, job_id, 0, 0).await;
    let pty = tools
        .execute(plan(
            &tools,
            "pty",
            "start_pty",
            json!({"argv":["sh","-c","exit 0"]}),
        ))
        .await
        .expect("Tool execution completes");
    let pty_id = pty.result.output["pty_session_id"].as_str().unwrap();
    let stopped = tools
        .execute(plan(
            &tools,
            "stop",
            "stop_pty",
            json!({"pty_session_id":pty_id}),
        ))
        .await
        .expect("Tool execution completes");
    assert_eq!(stopped.result.status, ToolResultStatus::Succeeded);
    println!("RUNTIME_HANDLES={job_id}|{pty_id}");
}

#[tokio::test]
async fn ctf_r3a_old_job_handles_cannot_control_a_new_registry() {
    let project = TestProject::new("runtime-handles");
    let old = ToolRegistry::for_project(&project.root);
    let started = old
        .execute(plan(
            &old,
            "old",
            "start_job",
            json!({"argv":["sh","-c","exit 0"]}),
        ))
        .await
        .expect("Tool execution completes");
    let old_id = started.result.output["job_id"].as_str().unwrap();
    poll_until_terminal(&old, old_id, 0, 0).await;
    let fresh = ToolRegistry::for_project(&project.root);
    let started = fresh
        .execute(plan(
            &fresh,
            "fresh",
            "start_job",
            json!({
                "argv":["sh","-c","read line; printf 'fresh:%s' \"$line\""], "timeout_ms":10000
            }),
        ))
        .await
        .expect("Tool execution completes");
    let fresh_id = started.result.output["job_id"].as_str().unwrap();
    assert_ne!(old_id, fresh_id);
    for stale in [old_id, "job:1"] {
        for (name, arguments) in [
            ("poll_job", json!({"job_id":stale})),
            (
                "input_job",
                json!({"job_id":stale,"input":"wrong\n","close":true}),
            ),
            ("stop_job", json!({"job_id":stale})),
        ] {
            let outcome = fresh
                .execute(plan(&fresh, "stale", name, arguments))
                .await
                .expect("Tool execution completes");
            assert_eq!(outcome.result.status, ToolResultStatus::Failed);
        }
    }
    let input = fresh
        .execute(plan(
            &fresh,
            "valid",
            "input_job",
            json!({"job_id":fresh_id,"input":"safe\n","close":true}),
        ))
        .await
        .expect("Tool execution completes");
    assert_eq!(input.result.status, ToolResultStatus::Succeeded);
    let output = poll_until_terminal(&fresh, fresh_id, 0, 0).await;
    assert_eq!(output.result.output["stdout"]["data"], "fresh:safe");
}

struct TestProject {
    root: PathBuf,
}

impl TestProject {
    fn new(label: &str) -> Self {
        let id = NEXT_PROJECT_ID.fetch_add(1, Ordering::Relaxed);
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/m1c-s2-tests")
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
    job_id: &str,
    stdout_offset: u64,
    stderr_offset: u64,
) -> ToolExecutionOutcome {
    for attempt in 0..50 {
        let outcome = tools
            .execute(plan(
                tools,
                &format!("poll-{attempt}"),
                "poll_job",
                json!({
                    "job_id": job_id,
                    "stdout_offset": stdout_offset,
                    "stderr_offset": stderr_offset,
                }),
            ))
            .await
            .expect("Tool execution completes");
        if outcome.result.output["status"] != "running"
            && outcome.result.output["status"] != "stopping"
        {
            return outcome;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("background Job did not reach a terminal state")
}

#[tokio::test]
async fn poll_uses_caller_offsets_and_keeps_start_completion_linkage_stable() {
    let project = TestProject::new("offsets-and-receipt");
    let tools = ToolRegistry::for_project(&project.root);
    assert!(matches!(
        tools.plan(
            ExecutionId::new("execution-invalid-shell"),
            ToolCall::new(
                ToolCallId::new("invalid-shell"),
                ToolName::new("start_job"),
                json!({"command": "echo unsafe"}),
            ),
        ),
        Err(ToolExecutorError::InvalidArguments { .. })
    ));
    let start = tools
        .execute(plan(
            &tools,
            "start",
            "start_job",
            json!({
                "argv": [
                    "sh",
                    "-c",
                    "read line; printf 'out:%s' \"$line\"; printf 'err:%s' \"$line\" >&2",
                ],
                "timeout_ms": 1_000,
            }),
        ))
        .await
        .expect("Tool execution completes");
    assert_eq!(start.result.status, ToolResultStatus::Succeeded);
    let job_id = start.result.output["job_id"].as_str().unwrap().to_owned();

    let input = tools
        .execute(plan(
            &tools,
            "input",
            "input_job",
            json!({"job_id": job_id, "input": "fixture", "close": true}),
        ))
        .await
        .expect("Tool execution completes");
    assert_eq!(input.result.status, ToolResultStatus::Succeeded);
    assert_eq!(input.result.output["input_closed"], true);

    let completed = poll_until_terminal(&tools, &job_id, 0, 0).await;
    assert_eq!(completed.result.status, ToolResultStatus::Succeeded);
    assert_eq!(completed.result.output["status"], "completed");
    assert_eq!(completed.result.output["stdout"]["data"], "out:fixture");
    assert_eq!(completed.result.output["stderr"]["data"], "err:fixture");
    assert_eq!(
        completed.result.output["completion"]["start_execution_id"],
        "execution-start"
    );
    assert_eq!(
        completed.result.output["completion"]["start_plan_fingerprint"],
        start.receipt.plan_fingerprint
    );
    assert_eq!(completed.result.output["completion"]["status"], "completed");
    let stdout_offset = completed.result.output["stdout"]["next_offset"]
        .as_u64()
        .unwrap();
    let stderr_offset = completed.result.output["stderr"]["next_offset"]
        .as_u64()
        .unwrap();

    let repeated = tools
        .execute(plan(
            &tools,
            "poll-repeated",
            "poll_job",
            json!({
                "job_id": job_id,
                "stdout_offset": stdout_offset,
                "stderr_offset": stderr_offset,
            }),
        ))
        .await
        .expect("Tool execution completes");
    assert_eq!(repeated.result.status, ToolResultStatus::Succeeded);
    assert_eq!(repeated.result.output["stdout"]["data"], "");
    assert_eq!(repeated.result.output["stderr"]["data"], "");
    assert_eq!(
        repeated.result.output["completion"],
        completed.result.output["completion"]
    );

    let invalid_offset = tools
        .execute(plan(
            &tools,
            "poll-invalid-offset",
            "poll_job",
            json!({"job_id": job_id, "stdout_offset": stdout_offset + 1}),
        ))
        .await
        .expect("Tool execution completes");
    assert_eq!(invalid_offset.result.status, ToolResultStatus::Failed);
    assert_eq!(
        invalid_offset.result.output["stdout"]["error"],
        "offset_out_of_range"
    );
}

#[tokio::test]
async fn timeout_and_stop_reap_jobs_into_one_terminal_state() {
    let project = TestProject::new("timeout-and-stop");
    let tools = ToolRegistry::for_project(&project.root);
    let timed_out_start = tools
        .execute(plan(
            &tools,
            "start-timeout",
            "start_job",
            json!({"argv": ["sh", "-c", "sleep 5"], "timeout_ms": 20}),
        ))
        .await
        .expect("Tool execution completes");
    let timed_out_id = timed_out_start.result.output["job_id"].as_str().unwrap();
    let timed_out = poll_until_terminal(&tools, timed_out_id, 0, 0).await;
    assert_eq!(timed_out.result.output["status"], "timed_out");
    assert_eq!(timed_out.result.output["completion"]["status"], "timed_out");
    assert!(
        timed_out.result.output["completion"]["exit_reason"]
            .as_str()
            .unwrap()
            .starts_with("job_timeout")
    );

    let stopped_start = tools
        .execute(plan(
            &tools,
            "start-stop",
            "start_job",
            json!({"argv": ["sh", "-c", "sleep 5"], "timeout_ms": 1_000}),
        ))
        .await
        .expect("Tool execution completes");
    let stopped_id = stopped_start.result.output["job_id"].as_str().unwrap();
    let stopped = tools
        .execute(plan(
            &tools,
            "stop",
            "stop_job",
            json!({"job_id": stopped_id}),
        ))
        .await
        .expect("Tool execution completes");
    assert_eq!(stopped.result.status, ToolResultStatus::Succeeded);
    assert_eq!(stopped.result.output["status"], "cancelled");
    assert_eq!(
        stopped.result.output["completion"]["start_execution_id"],
        "execution-start-stop"
    );
    let repeat_stop = tools
        .execute(plan(
            &tools,
            "stop-again",
            "stop_job",
            json!({"job_id": stopped_id}),
        ))
        .await
        .expect("Tool execution completes");
    assert_eq!(repeat_stop.result.status, ToolResultStatus::Succeeded);
    assert_eq!(repeat_stop.result.output["already_terminal"], true);
    assert_eq!(repeat_stop.result.output["status"], "cancelled");

    let raced_start = tools
        .execute(plan(
            &tools,
            "start-race",
            "start_job",
            json!({"argv": ["sh", "-c", "exit 0"], "timeout_ms": 1_000}),
        ))
        .await
        .expect("Tool execution completes");
    let raced_id = raced_start.result.output["job_id"].as_str().unwrap();
    let raced_stop = tokio::time::timeout(
        std::time::Duration::from_millis(500),
        tools.execute(plan(
            &tools,
            "stop-race",
            "stop_job",
            json!({"job_id": raced_id}),
        )),
    )
    .await
    .expect("a stop racing natural completion must not wait for an orphaned acknowledgement")
    .expect("Tool execution completes");
    assert_eq!(raced_stop.result.status, ToolResultStatus::Succeeded);
    assert!(matches!(
        raced_stop.result.output["status"].as_str(),
        Some("completed") | Some("cancelled")
    ));
}

#[tokio::test]
async fn job_control_plans_reject_empty_ids() {
    let project = TestProject::new("empty-job-id");
    let tools = ToolRegistry::for_project(&project.root);
    for (index, (tool_name, arguments)) in [
        ("poll_job", json!({"job_id": ""})),
        ("input_job", json!({"job_id": "", "input": "x"})),
        ("stop_job", json!({"job_id": ""})),
    ]
    .into_iter()
    .enumerate()
    {
        let result = tools.plan(
            ExecutionId::new(format!("execution-empty-{index}")),
            ToolCall::new(
                ToolCallId::new(format!("empty-{index}")),
                ToolName::new(tool_name),
                arguments,
            ),
        );
        assert!(
            matches!(result, Err(ToolExecutorError::InvalidArguments { .. })),
            "{tool_name} must reject an empty Job ID during planning"
        );
    }
}

#[tokio::test]
async fn stop_waits_for_terminal_when_descendant_holds_job_pipes() {
    let project = TestProject::new("descendant-pipe-race");
    let tools = ToolRegistry::for_project(&project.root);
    let start = tools
        .execute(plan(
            &tools,
            "start-descendant-pipe",
            "start_job",
            json!({
                "argv": ["sh", "-c", "sleep 1 & printf ready; sleep 0.05; exit 0"],
                "timeout_ms": 2_000,
            }),
        ))
        .await
        .expect("Tool execution completes");
    assert_eq!(start.result.status, ToolResultStatus::Succeeded);
    let job_id = start.result.output["job_id"].as_str().unwrap().to_owned();

    // Observe an explicit marker written only after the background descendant
    // has inherited the pipes. Fixed sleeps cannot prove this precondition on
    // a loaded CI runner.
    let mut descendant_started = false;
    for attempt in 0..100 {
        let observed = tools
            .execute(plan(
                &tools,
                &format!("poll-descendant-ready-{attempt}"),
                "poll_job",
                json!({"job_id": job_id}),
            ))
            .await
            .expect("Tool execution completes");
        if observed.result.output["stdout"]["data"] == "ready" {
            descendant_started = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        descendant_started,
        "descendant start marker was not observed"
    );

    // Whether stop or natural exit wins, the descendant keeps the inherited
    // pipes open and the supervisor must publish one stable terminal result
    // after its bounded reader grace period.
    let stopped = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        tools.execute(plan(
            &tools,
            "stop-descendant-pipe",
            "stop_job",
            json!({"job_id": job_id}),
        )),
    )
    .await
    .expect("stop must finish after bounded reader grace")
    .expect("Tool execution completes");
    assert_eq!(stopped.result.status, ToolResultStatus::Succeeded);
    assert_ne!(stopped.result.output["error"], "job_stop_unavailable");
    assert!(matches!(
        stopped.result.output["status"].as_str(),
        Some("completed") | Some("cancelled")
    ));
    assert!(stopped.result.output["completion"].is_object());

    let polled = tokio::time::timeout(
        std::time::Duration::from_millis(500),
        tools.execute(plan(
            &tools,
            "poll-descendant-pipe",
            "poll_job",
            json!({"job_id": job_id}),
        )),
    )
    .await
    .expect("poll after stop must be immediate")
    .expect("Tool execution completes");
    assert_eq!(polled.result.status, ToolResultStatus::Succeeded);
    assert_eq!(
        polled.result.output["status"],
        stopped.result.output["status"]
    );
    assert_eq!(
        polled.result.output["completion"],
        stopped.result.output["completion"]
    );
    let streams = ["stdout", "stderr"];
    assert!(streams.iter().any(|stream| {
        polled.result.output[*stream]["truncated"] == true
            && polled.result.output[*stream]["read_error"]
                .as_str()
                .is_some_and(|error| error.ends_with("_reader_grace_timeout"))
    }));
}

#[tokio::test]
async fn active_job_resource_leases_block_conflicts_until_terminal() {
    let project = TestProject::new("resource-leases");
    let tools = ToolRegistry::for_project(&project.root);
    let shared_resource = json!([{"kind": "output", "path": "shared.log"}]);
    let start = tools
        .execute(plan(
            &tools,
            "start-resource-lease",
            "start_job",
            json!({
                "argv": ["sh", "-c", "sleep 5"],
                "resources": shared_resource,
                "timeout_ms": 1_000,
            }),
        ))
        .await
        .expect("Tool execution completes");
    assert_eq!(start.result.status, ToolResultStatus::Succeeded);
    let job_id = start.result.output["job_id"].as_str().unwrap().to_owned();

    let conflicting_command = tools
        .execute(plan(
            &tools,
            "run-conflicting-resource",
            "run_command",
            json!({
                "argv": ["sh", "-c", "printf blocked"],
                "resources": [{"kind": "output", "path": "shared.log"}],
            }),
        ))
        .await
        .expect("Tool execution completes");
    assert_eq!(conflicting_command.result.status, ToolResultStatus::Failed);
    assert_eq!(
        conflicting_command.receipt.exit_reason.as_deref(),
        Some("job_resource_busy")
    );
    assert_eq!(
        conflicting_command.result.output["conflicting_job_id"],
        job_id
    );

    let conflicting_start = tools
        .execute(plan(
            &tools,
            "start-conflicting-resource",
            "start_job",
            json!({
                "argv": ["sh", "-c", "sleep 5"],
                "resources": [{"kind": "output", "path": "shared.log"}],
                "timeout_ms": 1_000,
            }),
        ))
        .await
        .expect("Tool execution completes");
    assert_eq!(conflicting_start.result.status, ToolResultStatus::Failed);
    assert_eq!(
        conflicting_start.receipt.exit_reason.as_deref(),
        Some("job_resource_busy")
    );

    let independent_command = tools
        .execute(plan(
            &tools,
            "run-independent-resource",
            "run_command",
            json!({
                "argv": ["sh", "-c", "printf independent"],
                "resources": [{"kind": "output", "path": "independent.log"}],
            }),
        ))
        .await
        .expect("Tool execution completes");
    assert_eq!(
        independent_command.result.status,
        ToolResultStatus::Succeeded
    );

    let stopped = tools
        .execute(plan(
            &tools,
            "stop-resource-lease",
            "stop_job",
            json!({"job_id": job_id}),
        ))
        .await
        .expect("Tool execution completes");
    assert_eq!(stopped.result.status, ToolResultStatus::Succeeded);
    assert_eq!(stopped.result.output["status"], "cancelled");

    let released_command = tools
        .execute(plan(
            &tools,
            "run-released-resource",
            "run_command",
            json!({
                "argv": ["sh", "-c", "printf released"],
                "resources": [{"kind": "output", "path": "shared.log"}],
            }),
        ))
        .await
        .expect("Tool execution completes");
    assert_eq!(released_command.result.status, ToolResultStatus::Succeeded);
}

#[tokio::test]
async fn poll_offsets_wait_for_utf8_boundaries_and_reject_continuation_bytes() {
    let project = TestProject::new("utf8-offsets");
    let tools = ToolRegistry::for_project(&project.root);
    let start = tools
        .execute(plan(
            &tools,
            "start-utf8",
            "start_job",
            json!({
                "argv": ["sh", "-c", "printf '\\303'; sleep 0.1; printf '\\251'"],
                "timeout_ms": 1_000,
            }),
        ))
        .await
        .expect("Tool execution completes");
    assert_eq!(start.result.status, ToolResultStatus::Succeeded);
    let job_id = start.result.output["job_id"].as_str().unwrap().to_owned();

    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    let partial = tools
        .execute(plan(
            &tools,
            "poll-utf8-partial",
            "poll_job",
            json!({"job_id": job_id, "stdout_offset": 0}),
        ))
        .await
        .expect("Tool execution completes");
    assert_eq!(partial.result.status, ToolResultStatus::Succeeded);
    assert_eq!(partial.result.output["stdout"]["data"], "");
    assert_eq!(partial.result.output["stdout"]["next_offset"], 0);

    let completed = poll_until_terminal(&tools, &job_id, 0, 0).await;
    assert_eq!(completed.result.output["status"], "completed");
    assert_eq!(completed.result.output["stdout"]["data"], "é");
    assert_eq!(completed.result.output["stdout"]["next_offset"], 2);

    let continuation_offset = tools
        .execute(plan(
            &tools,
            "poll-utf8-continuation",
            "poll_job",
            json!({"job_id": job_id, "stdout_offset": 1}),
        ))
        .await
        .expect("Tool execution completes");
    assert_eq!(continuation_offset.result.status, ToolResultStatus::Failed);
    assert_eq!(
        continuation_offset.result.output["stdout"]["error"],
        "offset_not_utf8_boundary"
    );
    assert_eq!(
        continuation_offset.result.output["stdout"]["reason"],
        "offset_not_utf8_boundary"
    );
}

#[tokio::test]
async fn poll_binary_output_allows_offsets_at_independent_continuation_bytes() {
    let project = TestProject::new("binary-offsets");
    let tools = ToolRegistry::for_project(&project.root);
    let start = tools
        .execute(plan(
            &tools,
            "start-binary-output",
            "start_job",
            json!({
                "argv": ["sh", "-c", "printf '\\200\\277'"],
                "timeout_ms": 1_000,
            }),
        ))
        .await
        .expect("Tool execution completes");
    assert_eq!(start.result.status, ToolResultStatus::Succeeded);
    let job_id = start.result.output["job_id"].as_str().unwrap().to_owned();
    let completed = poll_until_terminal(&tools, &job_id, 0, 0).await;
    assert_eq!(completed.result.output["status"], "completed");

    for (index, stdout_offset) in [0, 1].into_iter().enumerate() {
        let polled = tools
            .execute(plan(
                &tools,
                &format!("poll-binary-{index}"),
                "poll_job",
                json!({"job_id": job_id, "stdout_offset": stdout_offset}),
            ))
            .await
            .expect("Tool execution completes");
        assert_eq!(polled.result.status, ToolResultStatus::Succeeded);
        assert_eq!(polled.result.output["stdout"]["offset"], stdout_offset);
        assert_ne!(
            polled.result.output["stdout"]["error"],
            "offset_not_utf8_boundary"
        );
    }
}
