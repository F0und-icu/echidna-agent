use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cyber_agent::app::agent_loop::{AgentLoop, AgentTurnOutcome};
use cyber_agent::app::cancellation::AgentCancellation;
use cyber_agent::domain::model::{CapabilityMappingVersion, ModelCapabilities, ModelEvent};
use cyber_agent::domain::tools::{
    ExecutionId, ResourceKey, ToolCachePolicy, ToolCall, ToolCallId, ToolDeterminism, ToolEffect,
    ToolExecutionOutcome, ToolExecutionPlan, ToolExposure, ToolName, ToolOutputPolicy,
    ToolParallelism, ToolResultStatus, ToolRetryPolicy, ToolSpec,
};
use cyber_agent::ports::model::{
    ModelEventStream, ModelProvider, ModelProviderError, ModelRequest, ModelResponseCompletion,
    ModelStreamEvent,
};
use cyber_agent::ports::tool::{ToolExecutionFuture, ToolExecutor, ToolExecutorError};
use futures_util::stream;
use serde_json::{Value, json};

fn capabilities() -> ModelCapabilities {
    ModelCapabilities {
        text_input: true,
        image_input: false,
        streaming: true,
        tool_call: true,
        structured_output: false,
        incremental_input: false,
        usage_reporting: false,
        context_window: 128_000,
        capability_mapping_version: CapabilityMappingVersion::new(1),
    }
}

struct ScriptedProvider {
    responses: Mutex<VecDeque<Vec<ModelEvent>>>,
}

impl ScriptedProvider {
    fn new(responses: impl IntoIterator<Item = Vec<ModelEvent>>) -> Self {
        Self {
            responses: Mutex::new(responses.into_iter().collect()),
        }
    }
}

impl ModelProvider for ScriptedProvider {
    fn capabilities(&self) -> ModelCapabilities {
        capabilities()
    }

    fn stream(&self, request: ModelRequest) -> Result<ModelEventStream, ModelProviderError> {
        request.validate_capabilities(&self.capabilities())?;
        let events = self.responses.lock().unwrap().pop_front().ok_or_else(|| {
            ModelProviderError::ProviderFailure {
                code: Some("fixture_exhausted".to_owned()),
                message: "the scripted Provider has no response left".to_owned(),
            }
        })?;
        Ok(Box::pin(stream::iter(
            events
                .into_iter()
                .map(|event| Ok(ModelStreamEvent::Event(event)))
                .chain(std::iter::once(Ok(ModelStreamEvent::Completed(
                    ModelResponseCompletion::without_response_id(),
                )))),
        )))
    }
}

struct ExecutionTrace {
    active: AtomicUsize,
    maximum_active: AtomicUsize,
    executions: AtomicUsize,
}

impl ExecutionTrace {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            active: AtomicUsize::new(0),
            maximum_active: AtomicUsize::new(0),
            executions: AtomicUsize::new(0),
        })
    }
}

struct ProbeExecutor {
    spec: ToolSpec,
    trace: Arc<ExecutionTrace>,
}

impl ProbeExecutor {
    fn new(retry_policy: ToolRetryPolicy) -> Self {
        Self {
            spec: ToolSpec {
                name: ToolName::new("probe"),
                version: "1".to_owned(),
                description: "test scheduler execution".to_owned(),
                input_schema: json!({"type": "object"}),
                output_schema: json!({"type": "object"}),
                exposure: ToolExposure::Direct,
                parallelism: ToolParallelism::ResourceLocked,
                resource_key_template: Some("resource:{resource}".to_owned()),
                effect: ToolEffect::Interactive,
                retry_policy,
                determinism: ToolDeterminism::Nondeterministic,
                cache_policy: ToolCachePolicy::None,
                timeout_ms: 5_000,
                output_policy: ToolOutputPolicy { max_bytes: 1024 },
            },
            trace: ExecutionTrace::new(),
        }
    }

    fn resource_key(arguments: &Value) -> Result<ResourceKey, ToolExecutorError> {
        let resource = arguments
            .get("resource")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolExecutorError::InvalidArguments {
                tool_name: "probe".to_owned(),
                message: "missing string resource".to_owned(),
            })?;
        let key = resource
            .strip_prefix("file:")
            .map(ResourceKey::file)
            .or_else(|| resource.strip_prefix("device:").map(ResourceKey::device))
            .or_else(|| resource.strip_prefix("pty:").map(ResourceKey::pty))
            .or_else(|| resource.strip_prefix("job:").map(ResourceKey::job))
            .or_else(|| resource.strip_prefix("output:").map(ResourceKey::output))
            .ok_or_else(|| ToolExecutorError::InvalidArguments {
                tool_name: "probe".to_owned(),
                message: "unsupported resource kind".to_owned(),
            })?;
        Ok(key)
    }
}

impl ToolExecutor for ProbeExecutor {
    fn specs(&self) -> Vec<ToolSpec> {
        vec![self.spec.clone()]
    }

    fn plan(
        &self,
        execution_id: ExecutionId,
        call: ToolCall,
    ) -> Result<ToolExecutionPlan, ToolExecutorError> {
        let key = Self::resource_key(&call.arguments)?;
        ToolExecutionPlan::new(execution_id, call, &self.spec, vec![key]).map_err(|error| {
            ToolExecutorError::PlanFailure {
                tool_name: self.spec.name.to_string(),
                message: error.to_string(),
            }
        })
    }

    fn execute(&self, plan: ToolExecutionPlan) -> ToolExecutionFuture<'_> {
        let trace = self.trace.clone();
        Box::pin(async move {
            trace.executions.fetch_add(1, Ordering::SeqCst);
            let active = trace.active.fetch_add(1, Ordering::SeqCst) + 1;
            trace.maximum_active.fetch_max(active, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(30)).await;
            trace.active.fetch_sub(1, Ordering::SeqCst);
            if plan.call.arguments["cancel"] == true {
                ToolExecutionOutcome::cancelled(plan, "tool_timeout")
            } else if plan.call.arguments["fail"] == true {
                ToolExecutionOutcome::failed(plan, "fixture_failure", json!({"fixture": "failed"}))
            } else {
                ToolExecutionOutcome::succeeded(plan, json!({"fixture": "succeeded"}))
            }
        })
    }
}

fn call(id: &str, resource: &str) -> ToolCall {
    ToolCall::new(
        ToolCallId::new(id),
        ToolName::new("probe"),
        json!({"resource": resource}),
    )
}

fn rerun_call(id: &str, resource: &str, reason: &str) -> ToolCall {
    let mut call = call(id, resource);
    call.arguments["execution"] = json!({"rerun_reason": reason});
    call
}

fn batch_response(calls: Vec<ToolCall>) -> Vec<ModelEvent> {
    calls
        .into_iter()
        .map(|call| ModelEvent::ToolCall { call })
        .collect()
}

#[tokio::test]
async fn resource_scheduler_parallelizes_distinct_resources_and_serializes_conflicts() {
    for (first, second, parallelism, expected_peak) in [
        ("file:/one", "file:/two", ToolParallelism::ResourceLocked, 2),
        (
            "device:one",
            "device:two",
            ToolParallelism::ResourceLocked,
            2,
        ),
        (
            "output:/one",
            "output:/two",
            ToolParallelism::ResourceLocked,
            2,
        ),
        (
            "device:shared",
            "device:shared",
            ToolParallelism::ResourceLocked,
            1,
        ),
        (
            "pty:shared",
            "pty:shared",
            ToolParallelism::ResourceLocked,
            1,
        ),
        (
            "job:shared",
            "job:shared",
            ToolParallelism::ResourceLocked,
            1,
        ),
        (
            "output:/shared",
            "output:/shared",
            ToolParallelism::ResourceLocked,
            1,
        ),
        ("file:/shared", "file:/shared", ToolParallelism::Parallel, 2),
        (
            "device:shared",
            "device:shared",
            ToolParallelism::Parallel,
            1,
        ),
    ] {
        let provider = ScriptedProvider::new([
            batch_response(vec![call("call-1", first), call("call-2", second)]),
            vec![ModelEvent::TextDelta {
                text: "batch complete".to_owned(),
            }],
        ]);
        let mut tools = ProbeExecutor::new(ToolRetryPolicy::NeverAutoRetry);
        tools.spec.parallelism = parallelism;
        let trace = tools.trace.clone();
        let mut agent = AgentLoop::new(&provider, &tools);

        let outcome = agent
            .run_turn("run the batch", AgentCancellation::new())
            .await
            .unwrap();

        assert!(matches!(
            outcome,
            AgentTurnOutcome::Completed { executions, .. }
                if executions.len() == 2
        ));
        assert_eq!(trace.maximum_active.load(Ordering::SeqCst), expected_peak);
        assert_eq!(agent.history().ensure_all_tool_calls_paired(), Ok(()));
    }
}

#[tokio::test]
async fn partial_batch_failure_keeps_ordered_pairs_for_all_other_results() {
    let mut failed = call("call-failed", "device:two");
    failed.arguments["fail"] = json!(true);
    let provider = ScriptedProvider::new([
        batch_response(vec![
            call("call-first", "device:one"),
            failed,
            call("call-last", "device:three"),
        ]),
        vec![ModelEvent::TextDelta {
            text: "partial results considered".to_owned(),
        }],
    ]);
    let tools = ProbeExecutor::new(ToolRetryPolicy::NeverAutoRetry);
    let mut agent = AgentLoop::new(&provider, &tools);

    let outcome = agent
        .run_turn("run the batch", AgentCancellation::new())
        .await
        .unwrap();
    let AgentTurnOutcome::Completed { executions, .. } = outcome else {
        panic!("the scripted batch should complete");
    };
    assert_eq!(
        executions
            .iter()
            .map(|outcome| outcome.result.call_id.as_str())
            .collect::<Vec<_>>(),
        vec!["call-first", "call-failed", "call-last"]
    );
    assert_eq!(
        executions
            .iter()
            .map(|outcome| outcome.result.status)
            .collect::<Vec<_>>(),
        vec![
            ToolResultStatus::Succeeded,
            ToolResultStatus::Failed,
            ToolResultStatus::Succeeded,
        ]
    );
    assert_eq!(agent.history().ensure_all_tool_calls_paired(), Ok(()));
}

#[tokio::test]
async fn completed_dedupe_receipt_prevents_reexecution_across_turns() {
    let provider = ScriptedProvider::new([
        batch_response(vec![call("call-first", "device:one")]),
        vec![ModelEvent::TextDelta {
            text: "first complete".to_owned(),
        }],
        batch_response(vec![call("call-duplicate", "device:one")]),
        vec![ModelEvent::TextDelta {
            text: "duplicate complete".to_owned(),
        }],
    ]);
    let tools = ProbeExecutor::new(ToolRetryPolicy::DedupeByKey);
    let trace = tools.trace.clone();
    let mut agent = AgentLoop::new(&provider, &tools);

    agent
        .run_turn("first execution", AgentCancellation::new())
        .await
        .unwrap();
    let duplicate = agent
        .run_turn("duplicate execution", AgentCancellation::new())
        .await
        .unwrap();

    let AgentTurnOutcome::Completed { executions, .. } = duplicate else {
        panic!("the duplicate Turn should complete");
    };
    assert_eq!(trace.executions.load(Ordering::SeqCst), 1);
    assert_eq!(executions.len(), 1);
    assert_eq!(executions[0].result.call_id.as_str(), "call-duplicate");
    assert_eq!(
        executions[0].receipt.execution_id.as_str(),
        "execution:call-first"
    );
    assert_eq!(agent.history().ensure_all_tool_calls_paired(), Ok(()));
}

#[tokio::test]
async fn explicit_rerun_reexecutes_and_replaces_the_completed_receipt() {
    let provider = ScriptedProvider::new([
        batch_response(vec![call("call-first", "device:one")]),
        vec![ModelEvent::TextDelta {
            text: "first complete".to_owned(),
        }],
        batch_response(vec![rerun_call(
            "call-rerun",
            "device:one",
            "user_requested_rerun",
        )]),
        vec![ModelEvent::TextDelta {
            text: "rerun complete".to_owned(),
        }],
        batch_response(vec![call("call-duplicate", "device:one")]),
        vec![ModelEvent::TextDelta {
            text: "latest duplicate complete".to_owned(),
        }],
    ]);
    let tools = ProbeExecutor::new(ToolRetryPolicy::DedupeByKey);
    let trace = tools.trace.clone();
    let mut agent = AgentLoop::new(&provider, &tools);

    let first = agent
        .run_turn("first execution", AgentCancellation::new())
        .await
        .unwrap();
    let rerun = agent
        .run_turn("rerun explicitly", AgentCancellation::new())
        .await
        .unwrap();
    let duplicate = agent
        .run_turn("ordinary duplicate", AgentCancellation::new())
        .await
        .unwrap();

    let AgentTurnOutcome::Completed {
        executions: first_executions,
        ..
    } = first
    else {
        panic!("the first execution should complete");
    };
    let AgentTurnOutcome::Completed {
        executions: rerun_executions,
        ..
    } = rerun
    else {
        panic!("the explicit rerun should complete");
    };
    let AgentTurnOutcome::Completed {
        executions: duplicate_executions,
        ..
    } = duplicate
    else {
        panic!("the ordinary duplicate should complete");
    };

    assert_eq!(trace.executions.load(Ordering::SeqCst), 2);
    assert_eq!(
        first_executions[0].receipt.execution_id.as_str(),
        "execution:call-first"
    );
    assert_eq!(
        rerun_executions[0].receipt.execution_id.as_str(),
        "execution:call-rerun"
    );
    assert_ne!(
        first_executions[0].receipt.execution_id,
        rerun_executions[0].receipt.execution_id
    );
    assert_eq!(
        rerun_executions[0]
            .receipt
            .rerun_reason
            .as_ref()
            .map(|reason| reason.as_str()),
        Some("user_requested_rerun")
    );
    assert_eq!(
        duplicate_executions[0].receipt.execution_id,
        rerun_executions[0].receipt.execution_id
    );
    assert_eq!(agent.history().ensure_all_tool_calls_paired(), Ok(()));
}

#[tokio::test]
async fn cancelled_receipts_do_not_block_a_later_same_resource_execution() {
    let mut timed_out = call("call-timeout", "device:one");
    timed_out.arguments["cancel"] = json!(true);
    let mut retry = call("call-retry", "device:one");
    retry.arguments["cancel"] = json!(true);
    let provider = ScriptedProvider::new([
        batch_response(vec![timed_out]),
        vec![ModelEvent::TextDelta {
            text: "timeout recorded".to_owned(),
        }],
        batch_response(vec![retry]),
        vec![ModelEvent::TextDelta {
            text: "retry attempted".to_owned(),
        }],
    ]);
    let tools = ProbeExecutor::new(ToolRetryPolicy::DedupeByKey);
    let trace = tools.trace.clone();
    let mut agent = AgentLoop::new(&provider, &tools);

    let first = agent
        .run_turn("first execution", AgentCancellation::new())
        .await
        .unwrap();
    let second = agent
        .run_turn("second execution", AgentCancellation::new())
        .await
        .unwrap();

    assert!(matches!(
        first,
        AgentTurnOutcome::Completed { executions, .. }
            if executions[0].result.status == ToolResultStatus::Cancelled
    ));
    assert!(matches!(
        second,
        AgentTurnOutcome::Completed { executions, .. }
            if executions[0].result.status == ToolResultStatus::Cancelled
    ));
    assert_eq!(trace.executions.load(Ordering::SeqCst), 2);
    assert_eq!(agent.history().ensure_all_tool_calls_paired(), Ok(()));
}
