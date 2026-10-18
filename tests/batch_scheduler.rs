use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use echidna::app::agent_loop::{AgentLoop, AgentLoopError, AgentTurnOutcome};
use echidna::app::cancellation::AgentCancellation;
use echidna::domain::events::StreamId;
use echidna::domain::model::ContentItem;
use echidna::domain::model::{CapabilityMappingVersion, ModelCapabilities, ModelEvent};
use echidna::domain::project::ProjectId;
use echidna::domain::request_audit::{
    RequestAdapterVersion, RequestAuditBinding, RequestAuditRunContext,
};
use echidna::domain::tools::{
    ExecutionId, ResourceKey, ToolCachePolicy, ToolCall, ToolCallId, ToolDeterminism, ToolEffect,
    ToolExecutionOutcome, ToolExecutionPlan, ToolExposure, ToolName, ToolOutputPolicy,
    ToolParallelism, ToolResultStatus, ToolRetryPolicy, ToolSpec,
};
use echidna::ports::model::{
    ModelEventStream, ModelProvider, ModelProviderError, ModelRequest, ModelResponseCompletion,
    ModelStreamEvent,
};
use echidna::ports::tool::{
    ToolExecutionError, ToolExecutionFuture, ToolExecutor, ToolExecutorError,
};
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
        max_output_tokens: None,
        reasoning_effort: None,
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

struct HeldUnknownExecutor {
    spec: ToolSpec,
    known_waits_for_held: Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
    held_started: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    executions: Arc<Mutex<Vec<String>>>,
}

impl HeldUnknownExecutor {
    fn new() -> Self {
        let (held_started, known_waits_for_held) = tokio::sync::oneshot::channel();
        Self {
            spec: ToolSpec {
                name: ToolName::new("held_unknown"),
                version: "1".to_owned(),
                description: "deterministic unknown-outcome scheduler fixture".to_owned(),
                input_schema: json!({"type": "object"}),
                output_schema: json!({"type": "object"}),
                exposure: ToolExposure::Direct,
                parallelism: ToolParallelism::ResourceLocked,
                resource_key_template: Some("resource:{resource}".to_owned()),
                effect: ToolEffect::ReadOnly,
                retry_policy: ToolRetryPolicy::NeverAutoRetry,
                determinism: ToolDeterminism::Nondeterministic,
                cache_policy: ToolCachePolicy::None,
                timeout_ms: 5_000,
                output_policy: ToolOutputPolicy { max_bytes: 1024 },
            },
            known_waits_for_held: Mutex::new(Some(known_waits_for_held)),
            held_started: Mutex::new(Some(held_started)),
            executions: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl ToolExecutor for HeldUnknownExecutor {
    fn specs(&self) -> Vec<ToolSpec> {
        vec![self.spec.clone()]
    }

    fn plan(
        &self,
        execution_id: ExecutionId,
        call: ToolCall,
    ) -> Result<ToolExecutionPlan, ToolExecutorError> {
        let resource = call
            .arguments
            .get("resource")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .ok_or_else(|| ToolExecutorError::InvalidArguments {
                tool_name: self.spec.name.to_string(),
                message: "missing resource".to_owned(),
            })?;
        ToolExecutionPlan::new(
            execution_id,
            call,
            &self.spec,
            vec![ResourceKey::file(&resource)],
        )
        .map_err(|error| ToolExecutorError::PlanFailure {
            tool_name: self.spec.name.to_string(),
            message: error.to_string(),
        })
    }

    fn execute(&self, plan: ToolExecutionPlan) -> ToolExecutionFuture<'_> {
        let call_id = plan.call.call_id.as_str().to_owned();
        let executions = Arc::clone(&self.executions);
        match call_id.as_str() {
            "known" => {
                let held_started = self
                    .known_waits_for_held
                    .lock()
                    .expect("known-wait mutex")
                    .take()
                    .expect("known Tool has one start signal");
                Box::pin(async move {
                    executions
                        .lock()
                        .expect("execution trace mutex")
                        .push(call_id);
                    held_started.await.expect("held Tool must start first");
                    Ok(ToolExecutionOutcome::succeeded(
                        plan,
                        json!({"known": true}),
                    ))
                })
            }
            "held" => {
                let held_started = self
                    .held_started
                    .lock()
                    .expect("held-start mutex")
                    .take()
                    .expect("held Tool starts once");
                Box::pin(async move {
                    executions
                        .lock()
                        .expect("execution trace mutex")
                        .push(call_id);
                    held_started.send(()).expect("known Tool is waiting");
                    std::future::pending::<()>().await;
                    unreachable!("a held Tool never establishes an outcome")
                })
            }
            "unknown" => Box::pin(async move {
                executions
                    .lock()
                    .expect("execution trace mutex")
                    .push(call_id);
                Err(ToolExecutionError::OutcomeUnknown {
                    safe_reason: "fixture remote outcome was not observed".to_owned(),
                })
            }),
            "pending" => Box::pin(async move {
                executions
                    .lock()
                    .expect("execution trace mutex")
                    .push(call_id);
                Ok(ToolExecutionOutcome::succeeded(
                    plan,
                    json!({"unexpected": true}),
                ))
            }),
            _ => unreachable!("the fixture has four fixed calls"),
        }
    }
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
                Ok(ToolExecutionOutcome::cancelled(plan, "tool_timeout"))
            } else if plan.call.arguments["unknown"] == true {
                Err(ToolExecutionError::OutcomeUnknown {
                    safe_reason: "fixture remote outcome was not observed".to_owned(),
                })
            } else if plan.call.arguments["fail"] == true {
                Ok(ToolExecutionOutcome::failed(
                    plan,
                    "fixture_failure",
                    json!({"fixture": "failed"}),
                ))
            } else {
                Ok(ToolExecutionOutcome::succeeded(
                    plan,
                    json!({"fixture": "succeeded"}),
                ))
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

fn held_unknown_call(id: &str, resource: &str) -> ToolCall {
    ToolCall::new(
        ToolCallId::new(id),
        ToolName::new("held_unknown"),
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

#[tokio::test]
async fn unobserved_active_cancellation_has_no_receipt_and_pending_plans_stay_pre_execution() {
    let provider = ScriptedProvider::new([batch_response(vec![
        call("call-started", "device:shared"),
        call("call-pending", "device:shared"),
    ])]);
    let tools = ProbeExecutor::new(ToolRetryPolicy::NeverAutoRetry);
    let trace = tools.trace.clone();
    let mut agent = AgentLoop::new(&provider, &tools);
    let cancellation = AgentCancellation::new();
    let signal = cancellation.clone();

    let turn = agent.run_turn("cancel the resource wave", cancellation);
    let cancel = async move {
        tokio::time::sleep(Duration::from_millis(5)).await;
        signal.cancel();
    };
    let (outcome, ()) = tokio::join!(turn, cancel);
    assert!(matches!(
        outcome,
        Err(AgentLoopError::ToolExecutionInterrupted)
    ));
    assert_eq!(trace.executions.load(Ordering::SeqCst), 1);

    let results = agent
        .history()
        .items()
        .iter()
        .filter_map(|item| match item {
            ContentItem::ToolResult { result } => Some(result),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].call_id.as_str(), "call-pending");
    assert_eq!(results[0].status, ToolResultStatus::Failed);
    assert_eq!(results[0].output["failure"]["stage"], "pre_execution");
    assert!(results[0].output["failure"].get("execution_id").is_none());
    assert!(agent.history().ensure_all_tool_calls_paired().is_err());
}

#[tokio::test]
async fn unknown_execution_stops_the_batch_after_known_completion_without_pairing_the_unknown_call()
{
    let mut unknown = call("call-unknown", "device:shared");
    unknown.arguments["unknown"] = json!(true);
    let provider = ScriptedProvider::new([batch_response(vec![
        call("call-complete", "device:shared"),
        unknown,
        call("call-pending", "device:shared"),
    ])]);
    let tools = ProbeExecutor::new(ToolRetryPolicy::NeverAutoRetry);
    let trace = tools.trace.clone();
    let mut agent = AgentLoop::new(&provider, &tools);

    assert!(matches!(
        agent.run_turn("stop after unknown remote execution", AgentCancellation::new()).await,
        Err(AgentLoopError::ToolExecutionOutcomeUnknown { ref safe_reason })
            if safe_reason.contains("not observed")
    ));
    assert_eq!(trace.executions.load(Ordering::SeqCst), 2);

    let results = agent
        .history()
        .items()
        .iter()
        .filter_map(|item| match item {
            ContentItem::ToolResult { result } => Some(result),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        results
            .iter()
            .map(|result| result.call_id.as_str())
            .collect::<Vec<_>>(),
        ["call-complete", "call-pending"]
    );
    assert_eq!(results[0].status, ToolResultStatus::Succeeded);
    assert_eq!(results[1].status, ToolResultStatus::Failed);
    assert_eq!(results[1].output["failure"]["stage"], "pre_execution");
    assert!(results[1].output["failure"].get("execution_id").is_none());
    assert!(agent.history().ensure_all_tool_calls_paired().is_err());
}

#[tokio::test]
async fn unknown_execution_drops_a_held_active_call_and_keeps_known_and_pending_boundaries() {
    let provider = ScriptedProvider::new([batch_response(vec![
        held_unknown_call("known", "shared"),
        held_unknown_call("held", "other"),
        held_unknown_call("unknown", "shared"),
        held_unknown_call("pending", "shared"),
    ])]);
    let tools = HeldUnknownExecutor::new();
    let executions = Arc::clone(&tools.executions);
    let mut agent = AgentLoop::new(&provider, &tools);
    let audit_context = RequestAuditRunContext::new(
        RequestAuditBinding::new(
            ProjectId::new("project-held-unknown"),
            StreamId::new("session:held-unknown"),
        )
        .expect("fixture audit binding"),
        RequestAdapterVersion::new("fixture-adapter-v1").expect("fixture adapter version"),
    );
    let mut attempts = Vec::new();
    let mut receipts = Vec::new();

    assert!(matches!(
        agent
            .run_turn_audited_with_stages(
                "stop after an unknown concurrent Tool",
                AgentCancellation::new(),
                &audit_context,
                &mut |attempt| {
                    attempts.push(attempt);
                    Ok(())
                },
                &mut |_text| Ok(()),
                &mut |outcome| {
                    receipts.push(outcome.clone());
                    Ok(())
                },
            )
            .await,
        Err(AgentLoopError::ToolExecutionOutcomeUnknown { ref safe_reason })
            if safe_reason.contains("not observed")
    ));
    assert_eq!(attempts.len(), 1, "unknown outcome prevents a later sample");
    assert_eq!(receipts.len(), 1, "only the known completion has a Receipt");
    assert_eq!(receipts[0].result.call_id.as_str(), "known");

    let executions = executions.lock().expect("execution trace mutex");
    assert!(executions.contains(&"known".to_owned()));
    assert!(executions.contains(&"held".to_owned()));
    assert!(executions.contains(&"unknown".to_owned()));
    assert!(
        !executions.contains(&"pending".to_owned()),
        "the same-resource pending Tool never crosses the start boundary"
    );

    let results = agent
        .history()
        .items()
        .iter()
        .filter_map(|item| match item {
            ContentItem::ToolResult { result } => Some(result),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        results
            .iter()
            .map(|result| result.call_id.as_str())
            .collect::<Vec<_>>(),
        ["known", "pending"]
    );
    assert_eq!(results[0].status, ToolResultStatus::Succeeded);
    assert_eq!(results[1].status, ToolResultStatus::Failed);
    assert_eq!(results[1].output["failure"]["stage"], "pre_execution");
    assert!(results[1].output["failure"].get("execution_id").is_none());
    assert!(agent.history().ensure_all_tool_calls_paired().is_err());
}
