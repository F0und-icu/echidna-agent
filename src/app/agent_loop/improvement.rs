//! Fixed one-sample RSI scopes; never mutate the parent continuation or History.

use super::*;
use crate::domain::context::{
    ContextFragment, ContextFragmentContent, ContextFragmentId, ContextFragmentPriority,
    ContextFragmentSource, FragmentCachePolicy,
};
use crate::domain::model::{ModelRequestRequirements, ModelUsage};
use crate::domain::request_audit::RequestAuditDelivery;
use crate::domain::session::{ImprovementStage, scoped_call_id};
use crate::domain::tools::{ToolExposure, ToolName};
use crate::ports::model::ContinuationRetention;

pub(crate) struct ImprovementSampleInput<'a, Tools: ?Sized> {
    pub(crate) attempt_id: &'a str,
    pub(crate) stage: ImprovementStage,
    pub(crate) prompt: &'a PromptComposition,
    pub(crate) task: String,
    pub(crate) references: Vec<EventId>,
    pub(crate) tools: &'a Tools,
    pub(crate) request_budget: &'a ModelRequestBudget,
    pub(crate) cancellation: AgentCancellation,
    pub(crate) audit_context: &'a RequestAuditRunContext,
}

pub(crate) struct ImprovementSampleOutput {
    pub(crate) text: String,
    pub(crate) executions: Vec<ToolExecutionOutcome>,
    pub(crate) audit_event_id: EventId,
}

type ImprovementSampleCommit<'a> = dyn FnMut(
        EventId,
        crate::domain::context::ProviderSessionRef,
        Vec<ContentItem>,
        Vec<crate::domain::tools::ToolCallId>,
        Option<ModelUsage>,
    ) -> Result<(), AgentLoopError>
    + 'a;

impl<Provider: ModelProvider + ?Sized, Tools: ToolExecutor + ?Sized>
    AgentLoop<'_, '_, Provider, Tools>
{
    /// Uses the existing planner, immutable Step and execution owners in a fresh Full scope.
    pub(crate) async fn sample_improvement(
        &self,
        input: ImprovementSampleInput<'_, Tools>,
        record_attempt: &mut dyn FnMut(RequestAuditAttempt) -> Result<EventId, AgentLoopError>,
        record_sample: &mut ImprovementSampleCommit<'_>,
        durability: &mut ChatDurabilityStage<'_>,
    ) -> Result<ImprovementSampleOutput, AgentLoopError> {
        if input.cancellation.is_cancelled() {
            return Err(AgentLoopError::ImprovementCancelled);
        }
        let capabilities = self.provider.capabilities();
        let completed = self
            .last_sampling_step_plan
            .as_ref()
            .ok_or(AgentLoopError::ContextStateMismatch)?;
        if completed.turn_configuration().turn().turn_id
            != TurnId::new(format!("turn:{}", self.next_turn_sequence))
            || self.committed_history_len != self.history.items().len()
        {
            return Err(AgentLoopError::ContextStateMismatch);
        }
        let exposure = ToolExposurePlan::new(input.tools.catalog().iter().map(|spec| {
            (
                spec.name.clone(),
                if input.stage != ImprovementStage::Diagnosis
                    && spec.name == ToolName::new("run_command")
                    && completed
                        .resolved_tool_plan()
                        .advertised_spec(&spec.name)
                        .is_some()
                {
                    // Retain only availability captured by the original
                    // resolver in this operator Turn, including real deferred
                    // activation. Nothing is activated for a later Turn.
                    ToolExposure::Direct
                } else {
                    ToolExposure::Hidden
                },
            )
        }));
        let resolved = input
            .tools
            .resolve_tool_plan(&exposure, &DeferredActivationSet::default())?;
        let scope = format!("rsi:{}:{:?}", input.attempt_id, input.stage);
        let provider_session = ProviderSession::new(
            ProviderSessionId::new(&scope),
            ProviderSessionRole::Primary,
            0,
        );
        let provenance = ContextProvenance::new(
            ContextSlot::TurnUpdates,
            ContextOrigin::SessionProjection,
            SemanticClass::DerivedProjection,
            AuthorityLevel::DerivedNonEvidence,
            RetentionClass::Ephemeral,
            FreshnessIdentity::new(blake3::hash(input.task.as_bytes()).to_hex().to_string())?
                .with_producer_version("rsi-improvement:2"),
            ContextReferences::new(Vec::new(), Vec::new(), Vec::new(), input.references)?,
        )?;
        let host_task = ContextFragment::new(
            ContextFragmentId::new(format!("{scope}:host-task"))?,
            ContextFragmentSource::TurnLocalUpdates,
            ContextFragmentPriority::Required,
            FragmentCachePolicy::DoNotCache,
            ContextFragmentContent::ContentItems {
                items: vec![ContentItem::user_text(input.task)],
            },
        )?
        .with_provenance(provenance)?;
        let mut fragments = input.prompt.fragments()?;
        fragments.push(host_task);
        let plan = self.context_planner.compile(
            fragments,
            self.context_planner.default_budget(&capabilities)?,
            &capabilities,
            resolved.advertised_specs().to_vec(),
            ModelRequestRequirements::default(),
            None,
        )?;
        let turn = TurnContext::new(
            TurnId::new(&scope),
            self.model.clone(),
            self.profile.clone(),
            provider_session.reference().clone(),
            capabilities.context_window,
            resolved.advertised_specs(),
        )?;
        let step = SamplingStepPlan::new(
            &scope,
            TurnConfigurationSnapshot::new(
                turn,
                None,
                None,
                self.active_agent_model_profile.clone(),
            ),
            plan,
            resolved,
            capabilities.clone(),
        )?;
        let model_input = step.context_plan().model_input();
        let attempt = RequestAuditAttempt::new(
            input.audit_context,
            self.model.clone(),
            self.profile.clone(),
            capabilities,
            model_input.clone(),
            model_input.clone(),
            RequestAuditDelivery::full(BaselineInvalidationReason::NoBaseline),
        )?
        .bind_sampling_step(&step, step.context_plan().delivery_map().clone())?
        .bind_improvement_scope(provider_session.reference().clone());
        let request = ModelRequest::full(model_input)
            .with_opaque_continuation_scope(OpaqueContinuationScope::for_provider_session(
                provider_session.reference(),
            ))
            .with_continuation_retention(ContinuationRetention::SingleSample);
        let mut final_audit = None;
        let mut discard_events =
            |_event: &crate::domain::model::ModelEvent| Ok::<(), ModelStreamingError>(());
        let provider = self.provider;
        let outcome = prepare_and_consume_with_retry(
            provider,
            input.cancellation.clone(),
            || {
                provider
                    .prepare(request.clone())
                    .map_err(ModelStreamingError::Provider)
            },
            || {
                input
                    .request_budget
                    .admit(ProviderSessionRole::Primary)
                    .map_err(|limit| AgentLoopError::ModelRequestLimit { limit })?;
                final_audit = Some(record_attempt(attempt.clone())?);
                Ok(())
            },
            &mut discard_events,
        )
        .await
        .map_err(|error| match error {
            ModelRetryError::Attempt(error) => error,
            ModelRetryError::Streaming(error) => AgentLoopError::Model(error),
        })?;
        let ModelStreamOutcome::Completed { events, .. } = outcome else {
            return Err(AgentLoopError::ImprovementCancelled);
        };
        let audit_event_id = final_audit
            .expect("a completed RSI sample has a successful attempt's durable RequestAudit");
        let sample = ModelSample::from_events(events);
        let original_ids = sample
            .tool_calls()
            .iter()
            .map(|call| call.call_id.clone())
            .collect::<Vec<_>>();
        if original_ids.len() > usize::from(input.stage != ImprovementStage::Diagnosis) {
            return Err(AgentLoopError::ImprovementResponse {
                message: "a scope allows at most one command; diagnosis allows none".to_owned(),
            });
        }
        let mut history = ConversationHistory::default();
        for mut item in sample.content_items() {
            match &mut item {
                ContentItem::Reasoning { .. } | ContentItem::OpaqueContinuation { .. } => continue,
                ContentItem::ToolCall { call } => {
                    if call.name != ToolName::new("run_command") {
                        return Err(AgentLoopError::ImprovementResponse {
                            message: "only run_command is supported in a probe".to_owned(),
                        });
                    }
                    call.call_id = scoped_call_id(input.attempt_id, input.stage, &call.call_id);
                }
                _ => {}
            }
            history.append_content_item(item)?;
        }
        record_sample(
            audit_event_id.clone(),
            provider_session.reference().clone(),
            history.items().to_vec(),
            original_ids,
            sample.usage(),
        )?;
        let calls = history
            .items()
            .iter()
            .filter_map(|item| {
                if let ContentItem::ToolCall { call } = item {
                    Some(call.clone())
                } else {
                    None
                }
            })
            .collect();
        let mut idempotency = BTreeMap::new();
        let scheduled = execution::execute(
            execution::ExecutionContext {
                tools: input.tools,
                output_publisher: self.output_publisher.as_deref(),
                history: &mut history,
                completed_idempotency: &mut idempotency,
            },
            calls,
            step.resolved_tool_plan(),
            &input.cancellation,
            &mut None,
            &mut None,
            &mut Some(durability),
        )
        .await?;
        if scheduled.cancelled {
            return Err(AgentLoopError::ImprovementCancelled);
        }
        Ok(ImprovementSampleOutput {
            text: sample.assistant_text(),
            executions: scheduled.executions,
            audit_event_id,
        })
    }
}
