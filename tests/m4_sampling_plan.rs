use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use cyber_agent::adapters::tools::ToolRegistry;
use cyber_agent::app::agent_loop::{AgentLoop, AgentTurnOutcome};
use cyber_agent::app::cancellation::AgentCancellation;
use cyber_agent::domain::context::{
    ContextBudget, ContextFragment, ContextFragmentContent, ContextFragmentId,
    ContextFragmentPriority, ContextFragmentSource, ContextOrigin, ContextPlanner, ContextSlot,
    FragmentCachePolicy, ModelIdentity, ModelProfile, PromptRendererIdentity, ProviderSessionId,
    ProviderSessionRef, ProviderSessionRole, TurnContext, TurnId,
};
use cyber_agent::domain::domain_profile::DomainProfile;
use cyber_agent::domain::model::{
    CapabilityMappingVersion, ModelCapabilities, ModelEvent, ModelRequestRequirements,
};
use cyber_agent::domain::research_control::{PromptComposition, ResearchPhase};
use cyber_agent::domain::sampling::{SamplingStepPlan, TurnConfigurationSnapshot};
use cyber_agent::domain::tools::{
    DeferredActivationSet, ExecutionId, ResolvedToolPlan, ToolCachePolicy, ToolCall, ToolCallId,
    ToolDeterminism, ToolEffect, ToolExecutionOutcome, ToolExecutionPlan, ToolExposure,
    ToolExposurePlan, ToolName, ToolOutputPolicy, ToolParallelism, ToolRetryPolicy, ToolSpec,
};
use cyber_agent::ports::model::{
    ModelEventStream, ModelProvider, ModelProviderError, ModelRequest, ModelResponseCompletion,
    ModelStreamEvent,
};
use cyber_agent::ports::tool::{ToolExecutionFuture, ToolExecutor, ToolExecutorError};
use futures_core::Stream;
use serde_json::json;

fn capabilities() -> ModelCapabilities {
    ModelCapabilities {
        text_input: true,
        image_input: false,
        streaming: true,
        tool_call: true,
        structured_output: true,
        incremental_input: false,
        usage_reporting: true,
        max_output_tokens: None,
        context_window: 32_768,
        capability_mapping_version: CapabilityMappingVersion::new(1),
    }
}

struct ScriptedProvider {
    responses: Mutex<VecDeque<Vec<ModelEvent>>>,
    requests: Mutex<Vec<ModelRequest>>,
}

impl ScriptedProvider {
    fn new(responses: impl IntoIterator<Item = Vec<ModelEvent>>) -> Self {
        Self {
            responses: Mutex::new(responses.into_iter().collect()),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<ModelRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl ModelProvider for ScriptedProvider {
    fn capabilities(&self) -> ModelCapabilities {
        capabilities()
    }

    fn stream(&self, request: ModelRequest) -> Result<ModelEventStream, ModelProviderError> {
        self.requests.lock().unwrap().push(request);
        let events = self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("the fixture provides one response per request");
        Ok(Box::pin(FixedStream {
            events: events
                .into_iter()
                .map(|event| Ok(ModelStreamEvent::Event(event)))
                .chain(std::iter::once(Ok(ModelStreamEvent::Completed(
                    ModelResponseCompletion::without_response_id(),
                ))))
                .collect(),
        }))
    }
}

struct FixedStream {
    events: VecDeque<Result<ModelStreamEvent, ModelProviderError>>,
}

impl Stream for FixedStream {
    type Item = Result<ModelStreamEvent, ModelProviderError>;

    fn poll_next(mut self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Ready(self.events.pop_front())
    }
}

fn call(id: &str, name: &str, arguments: serde_json::Value) -> ModelEvent {
    ModelEvent::ToolCall {
        call: ToolCall::new(ToolCallId::new(id), ToolName::new(name), arguments),
    }
}

fn contains_tool(request: &ModelRequest, name: &str) -> bool {
    request
        .input()
        .tool_specs
        .iter()
        .any(|spec| spec.name.as_str() == name)
}

#[tokio::test]
async fn deferred_activation_changes_only_later_steps_and_clears_at_the_next_turn() {
    let root = std::env::temp_dir().join(format!("cyber-agent-m4-sampling-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let provider = ScriptedProvider::new([
        vec![
            ModelEvent::ReasoningItem {
                text: "find the deferred write tool".to_owned(),
            },
            call("search", "tool_search", json!({"query": "atomic file"})),
        ],
        vec![call(
            "write-first-turn",
            "write_file",
            json!({"path": "turn.txt", "content": "first turn\n", "expected_hash": null}),
        )],
        vec![ModelEvent::TextDelta {
            text: "first turn complete".to_owned(),
        }],
        vec![call(
            "write-second-turn",
            "write_file",
            json!({"path": "must-not-run.txt", "content": "blocked\n", "expected_hash": null}),
        )],
        vec![ModelEvent::TextDelta {
            text: "second turn complete".to_owned(),
        }],
    ]);
    let tools = ToolRegistry::for_project(&root);
    let mut agent =
        AgentLoop::new(&provider, &tools).with_prompt_composition(PromptComposition::current());

    let first = agent
        .run_turn("first", AgentCancellation::new())
        .await
        .unwrap();
    let AgentTurnOutcome::Completed { executions, .. } = first else {
        panic!("first fixture Turn must complete")
    };
    assert_eq!(executions.len(), 2);
    assert!(
        executions
            .iter()
            .all(|outcome| outcome.receipt.resolved_tool_plan_fingerprint.is_some())
    );

    let second = agent
        .run_turn("second", AgentCancellation::new())
        .await
        .unwrap();
    let AgentTurnOutcome::Completed { executions, .. } = second else {
        panic!("second fixture Turn must complete")
    };
    assert!(executions.is_empty());
    assert!(!root.join("must-not-run.txt").exists());
    assert_eq!(agent.history().ensure_all_tool_calls_paired(), Ok(()));

    let requests = provider.requests();
    assert_eq!(requests.len(), 5);
    assert!(!contains_tool(&requests[0], "write_file"));
    assert!(contains_tool(&requests[1], "write_file"));
    assert!(requests[1].input().content_items.iter().any(|item| {
        matches!(
            item,
            cyber_agent::domain::model::ContentItem::Reasoning { text }
                if text == "find the deferred write tool"
        )
    }));
    assert!(contains_tool(&requests[2], "write_file"));
    assert!(!contains_tool(&requests[3], "write_file"));
    assert!(!contains_tool(&requests[4], "write_file"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn sampling_plan_binds_exact_context_tools_renderer_and_independent_context_axes() {
    let root = std::env::temp_dir();
    let tools = ToolRegistry::for_project(&root);
    let resolved = tools
        .resolve_tool_plan(
            &ToolExposurePlan::new([(ToolName::new("write_file"), ToolExposure::Hidden)]),
            &DeferredActivationSet::default(),
        )
        .unwrap();
    let fragments = vec![
        ContextFragment::new(
            ContextFragmentId::new("prompt:base").unwrap(),
            ContextFragmentSource::BasePrompt,
            ContextFragmentPriority::Required,
            FragmentCachePolicy::DoNotCache,
            ContextFragmentContent::Instructions {
                text: "base bytes".to_owned(),
            },
        )
        .unwrap(),
        ContextFragment::new(
            ContextFragmentId::new("prompt:personal").unwrap(),
            ContextFragmentSource::PersonalPreferences,
            ContextFragmentPriority::Required,
            FragmentCachePolicy::DoNotCache,
            ContextFragmentContent::Instructions {
                text: "personal bytes".to_owned(),
            },
        )
        .unwrap(),
    ];
    assert_eq!(fragments[0].slot(), ContextSlot::BaseInstructions);
    assert_eq!(fragments[0].origin(), ContextOrigin::PromptResource);
    let planner = ContextPlanner::current();
    let context = planner
        .compile(
            fragments,
            ContextBudget::new(32_768, 0, 0).unwrap(),
            &capabilities(),
            resolved.advertised_specs().to_vec(),
            ModelRequestRequirements::default(),
            None,
        )
        .unwrap();
    assert_eq!(context.base_instructions(), "base bytes\n\npersonal bytes");
    assert_eq!(
        context.prompt_renderer(),
        &PromptRendererIdentity::generic_v1()
    );
    let turn = TurnContext::new(
        TurnId::new("turn:fixture"),
        ModelIdentity::new("fixture", "primary"),
        ModelProfile::new("fixture", 1),
        ProviderSessionRef {
            id: ProviderSessionId::new("primary:fixture"),
            role: ProviderSessionRole::Primary,
            generation: 1,
        },
        capabilities().context_window,
        resolved.advertised_specs(),
    )
    .unwrap();
    let step = SamplingStepPlan::new(
        "turn:fixture:step:0",
        TurnConfigurationSnapshot::new(turn, DomainProfile::Source, ResearchPhase::Mapping, None),
        context,
        resolved.clone(),
        capabilities(),
    )
    .unwrap();
    assert_eq!(
        step.context_plan().tool_specs(),
        step.resolved_tool_plan().advertised_specs()
    );
    assert_eq!(step.prompt_renderer().id(), "generic");
    assert_eq!(step.prompt_renderer().version(), 1);

    let mut execution = tools
        .plan_model_call(
            step.resolved_tool_plan(),
            ExecutionId::new("execution:read"),
            ToolCall::new(
                ToolCallId::new("read"),
                ToolName::new("read_file"),
                json!({"path": "Cargo.toml"}),
            ),
        )
        .unwrap();
    assert!(
        step.resolved_tool_plan()
            .validates_execution_plan(&execution)
    );
    let original_fingerprint = execution.resolved_tool_plan_fingerprint().unwrap().clone();
    let changed = tools
        .resolve_tool_plan(
            &ToolExposurePlan::new([(ToolName::new("read_file"), ToolExposure::Hidden)]),
            &DeferredActivationSet::default(),
        )
        .unwrap();
    assert_ne!(original_fingerprint, *changed.fingerprint());
    assert!(
        step.resolved_tool_plan()
            .validates_execution_plan(&execution)
    );
    assert!(!changed.validates_execution_plan(&execution));
    execution
        .bind_resolved_tool_plan(step.resolved_tool_plan())
        .unwrap();
}

fn visible_spec(name: &str) -> ToolSpec {
    ToolSpec {
        name: ToolName::new(name),
        version: "1".to_owned(),
        description: "fixture".to_owned(),
        input_schema: json!({"type": "object"}),
        output_schema: json!({"type": "object"}),
        exposure: ToolExposure::Direct,
        parallelism: ToolParallelism::Serial,
        resource_key_template: None,
        effect: ToolEffect::ReadOnly,
        retry_policy: ToolRetryPolicy::ReplaySafe,
        determinism: ToolDeterminism::Deterministic,
        cache_policy: ToolCachePolicy::None,
        timeout_ms: 1000,
        output_policy: ToolOutputPolicy { max_bytes: 1024 },
    }
}

struct MismatchedExecutor {
    spec: ToolSpec,
    executions: Arc<AtomicUsize>,
}

impl ToolExecutor for MismatchedExecutor {
    fn catalog(&self) -> Vec<ToolSpec> {
        vec![self.spec.clone()]
    }

    fn plan(
        &self,
        execution_id: ExecutionId,
        call: ToolCall,
    ) -> Result<ToolExecutionPlan, ToolExecutorError> {
        ToolExecutionPlan::new(execution_id, call, &self.spec, Vec::new()).map_err(|error| {
            ToolExecutorError::PlanFailure {
                tool_name: self.spec.name.to_string(),
                message: error.to_string(),
            }
        })
    }

    fn plan_model_call(
        &self,
        _resolved: &ResolvedToolPlan,
        execution_id: ExecutionId,
        call: ToolCall,
    ) -> Result<ToolExecutionPlan, ToolExecutorError> {
        let foreign = ResolvedToolPlan::resolve(
            [self.spec.clone(), {
                let mut hidden = visible_spec("hidden_fixture");
                hidden.exposure = ToolExposure::Hidden;
                hidden
            }],
            &ToolExposurePlan::default(),
            &DeferredActivationSet::default(),
        )
        .unwrap();
        let mut plan = self.plan(execution_id, call)?;
        plan.bind_resolved_tool_plan(&foreign).map_err(|error| {
            ToolExecutorError::ResolvedPlanFailure {
                message: error.to_string(),
            }
        })?;
        Ok(plan)
    }

    fn execute(&self, plan: ToolExecutionPlan) -> ToolExecutionFuture<'_> {
        let executions = self.executions.clone();
        Box::pin(async move {
            executions.fetch_add(1, Ordering::SeqCst);
            ToolExecutionOutcome::succeeded(plan, json!({"unexpected": true}))
        })
    }
}

#[tokio::test]
async fn mismatched_dispatch_plan_is_paired_before_execution_without_a_receipt() {
    let provider = ScriptedProvider::new([
        vec![call("probe", "probe", json!({}))],
        vec![ModelEvent::TextDelta {
            text: "corrected".to_owned(),
        }],
    ]);
    let executions = Arc::new(AtomicUsize::new(0));
    let tools = MismatchedExecutor {
        spec: visible_spec("probe"),
        executions: executions.clone(),
    };
    let mut agent = AgentLoop::new(&provider, &tools);
    let outcome = agent
        .run_turn("probe", AgentCancellation::new())
        .await
        .unwrap();
    let AgentTurnOutcome::Completed {
        executions: outcome_executions,
        ..
    } = outcome
    else {
        panic!("the correction fixture must complete")
    };
    assert!(outcome_executions.is_empty());
    assert_eq!(executions.load(Ordering::SeqCst), 0);
    assert_eq!(agent.history().ensure_all_tool_calls_paired(), Ok(()));
}
