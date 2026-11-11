use crate::adapters::tools::ToolRegistry;
use crate::domain::tools::{ExecutionId, ToolCall, ToolCallId, ToolName, ToolResultStatus};
use crate::ports::tool::{ToolCancellation, ToolExecutor};
use crate::support::TemporaryDirectory;
use serde_json::json;
use std::time::Duration;

fn cancellation_channel() -> (tokio::sync::watch::Sender<bool>, ToolCancellation) {
    let (sender, mut receiver) = tokio::sync::watch::channel(false);
    let requested = receiver.clone();
    let cancellation = ToolCancellation::new(move || *requested.borrow(), async move {
        receiver.wait_for(|cancelled| *cancelled).await.unwrap();
    });
    (sender, cancellation)
}

#[tokio::test]
async fn parallel_cancellation_settles_each_process_and_preserves_sparse_indices() {
    let root = TemporaryDirectory::new("parallel-command-cancellation");
    let tools = ToolRegistry::for_project(root.path());
    let plan = tools.plan(ExecutionId::new("parallel-stop"), ToolCall::new(
        ToolCallId::new("parallel-stop"), ToolName::new("run_commands"),
        json!({"mode":"parallel","commands":[
            {"argv":["/bin/sh","-c","printf FIRST; printf '%s' $$ > first.pid; exec /bin/sleep 30"],"resources":[{"kind":"output","path":"first.pid"}]},
            {"argv":["/bin/sh","-c","printf BAD > forbidden"],"resources":[{"kind":"output","path":"first.pid"}]},
            {"argv":["/bin/sh","-c","printf THIRD; printf '%s' $$ > third.pid; exec /bin/sleep 30"],"resources":[{"kind":"output","path":"third.pid"}]}
        ]}),
    )).unwrap();
    let (sender, cancellation) = cancellation_channel();
    let execution = tools.execute_interruptible(plan, None, cancellation);
    let stop = async {
        tokio::time::timeout(Duration::from_secs(5), async {
            while !root.path().join("first.pid").exists() || !root.path().join("third.pid").exists()
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        sender.send_replace(true);
    };
    let (outcome, ()) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(execution, stop)
    })
    .await
    .unwrap();
    let outcome = outcome.unwrap();
    assert_eq!(outcome.receipt.status, ToolResultStatus::Cancelled);
    let commands = outcome.result.output["commands"].as_array().unwrap();
    assert_eq!(commands.len(), 2);
    assert_eq!(commands[0]["index"], 0);
    assert_eq!(commands[1]["index"], 2);
    assert_eq!(commands[0]["result"]["stdout"], "FIRST");
    assert_eq!(commands[1]["result"]["stdout"], "THIRD");
    assert!(!root.path().join("forbidden").exists());
    for file in ["first.pid", "third.pid"] {
        let pid = std::fs::read_to_string(root.path().join(file)).unwrap();
        assert!(
            !std::process::Command::new("/bin/kill")
                .args(["-0", &pid])
                .output()
                .unwrap()
                .status
                .success(),
            "owned child {pid} was not reaped"
        );
    }
    assert_eq!(outcome.pending_captures.len(), 4);
    assert!(
        outcome
            .pending_captures
            .iter()
            .all(|capture| matches!(capture.child_index, Some(0 | 2)))
    );
}

#[tokio::test]
async fn cancelled_commands_do_not_spawn_when_execution_has_not_been_polled() {
    let root = TemporaryDirectory::new("cancel-before-command-poll");
    let tools = ToolRegistry::for_project(root.path());
    for mode in ["single", "serial", "parallel"] {
        let command = json!({"argv":["/bin/sh","-c","printf BAD > forbidden"]});
        let (name, arguments) = if mode == "single" {
            ("run_command", command)
        } else {
            ("run_commands", json!({"mode":mode,"commands":[command]}))
        };
        let plan = tools
            .plan(
                ExecutionId::new(mode),
                ToolCall::new(ToolCallId::new(mode), ToolName::new(name), arguments),
            )
            .unwrap();
        let (sender, cancellation) = cancellation_channel();
        sender.send_replace(true);
        let outcome = tools
            .execute_interruptible(plan, None, cancellation)
            .await
            .unwrap();
        assert_eq!(outcome.receipt.status, ToolResultStatus::Cancelled);
        assert!(!root.path().join("forbidden").exists());
    }
}

#[tokio::test]
async fn command_cancellation_waits_for_checkpoint_past_execution_deadline() {
    use crate::domain::tools::ToolExecutionPlan;
    use crate::ports::tool::ToolObservationSender;
    use std::sync::Arc;

    for cancel in [false, true] {
        let root = TemporaryDirectory::new("command-checkpoint-deadline");
        let tools = ToolRegistry::for_project(root.path());
        let normalized = tools
            .plan(
                ExecutionId::new("checkpoint-deadline"),
                ToolCall::new(
                    ToolCallId::new("checkpoint-deadline"),
                    ToolName::new("run_commands"),
                    json!({"mode":"serial","commands":[
                        {"argv":["/usr/bin/printf","RETAINED"]},
                        {"argv":["/bin/sh","-c","printf BAD > forbidden"]}
                    ]}),
                ),
            )
            .unwrap();
        let mut spec = tools
            .catalog()
            .into_iter()
            .find(|spec| spec.name.as_str() == "run_commands")
            .unwrap();
        spec.timeout_ms = 2000;
        let plan = ToolExecutionPlan::new(
            normalized.execution_id,
            normalized.call,
            &spec,
            normalized.resolved_resource_keys,
        )
        .unwrap();
        let (observed_sender, mut observed) = tokio::sync::mpsc::channel(1);
        let release = Arc::new(tokio::sync::Notify::new());
        let release_observer = release.clone();
        let observer = ToolObservationSender(Arc::new(move |checkpoint, _captures| {
            let sender = observed_sender.clone();
            let release = release_observer.clone();
            Box::pin(async move {
                sender.send(checkpoint).await.unwrap();
                release.notified().await;
                Vec::new()
            })
        }));
        let (sender, cancellation) = cancellation_channel();
        let mut execution = tools.execute_interruptible(plan, Some(observer), cancellation);
        let checkpoint = tokio::select! {
            checkpoint = observed.recv() => checkpoint.unwrap(),
            outcome = &mut execution => panic!("execution ended before checkpoint: {outcome:?}"),
        };
        assert!(
            serde_json::to_string(&checkpoint)
                .unwrap()
                .contains("RETAINED")
        );
        if cancel {
            sender.send_replace(true);
            // Acknowledgement crosses the original deadline. The operator
            // cancellation must keep polling this exact execution, not drop it.
            assert!(
                tokio::time::timeout(Duration::from_millis(2100), &mut execution)
                    .await
                    .is_err()
            );
            release.notify_one();
            let outcome = tokio::time::timeout(Duration::from_secs(1), execution)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(outcome.receipt.status, ToolResultStatus::Cancelled);
            assert_eq!(
                outcome.receipt.exit_reason.as_deref(),
                Some("agent_cancelled")
            );
            assert_eq!(
                outcome.result.output["commands"][0]["result"]["stdout"],
                "RETAINED"
            );
        } else {
            let outcome = tokio::time::timeout(Duration::from_secs(3), execution)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(outcome.receipt.status, ToolResultStatus::Cancelled);
            assert_eq!(outcome.receipt.exit_reason.as_deref(), Some("tool_timeout"));
        }
        assert!(!root.path().join("forbidden").exists());
    }
}
