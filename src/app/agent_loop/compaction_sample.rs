//! One isolated, tool-free model handoff using the ordinary sampling owners.

use serde::{Deserialize, Serialize};

use super::*;
use crate::domain::context::{
    ContextFragment, ContextFragmentContent, ContextFragmentId, ContextFragmentPriority,
    ContextFragmentSource, FragmentCachePolicy,
};
use crate::domain::model::{ModelRequestRequirements, ModelUsage};
use crate::domain::request_audit::RequestAuditDelivery;
use crate::domain::session::{CompactionModelSample, NarrativeSummary, SessionSourceIdentity};
use crate::ports::model::ContinuationRetention;

pub(crate) struct CompactionSampleInput<'a> {
    pub(crate) provider: &'a dyn ModelProvider,
    pub(crate) agent_model_profile: Option<&'a AgentModelProfile>,
    pub(crate) source: &'a SessionSourceIdentity,
    pub(crate) source_data: String,
    pub(crate) current_goal: String,
    pub(crate) references: Vec<EventId>,
    pub(crate) byte_limit: usize,
    pub(crate) request_budget: &'a ModelRequestBudget,
    pub(crate) cancellation: AgentCancellation,
    pub(crate) audit_context: &'a RequestAuditRunContext,
    pub(crate) graph: CompactionGraphSample<'a>,
}

pub(crate) struct CompactionGraphSample<'a> {
    pub(crate) capture: &'a crate::domain::session::CompactionContextCapture,
    pub(crate) plan: &'a crate::domain::session::CompactionSourcePlan,
    pub(crate) plan_blob_id: crate::domain::artifacts::BlobId,
    pub(crate) node_id: &'a str,
}

pub(crate) struct CompactionSampleOutput {
    pub(crate) sample: CompactionModelSample,
    pub(crate) usage: Option<ModelUsage>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Handoff {
    concise_progress: String,
    unresolved_questions: Vec<String>,
    next_step_notes: Vec<String>,
}

pub(crate) fn minimum_compaction_handoff_bytes() -> usize {
    serde_json::to_vec(&Handoff {
        concise_progress: "a".to_owned(),
        unresolved_questions: Vec::new(),
        next_step_notes: Vec::new(),
    })
    .expect("fixed handoff serializes")
    .len()
}

impl<Provider: ModelProvider + ?Sized, Tools: ToolExecutor + ?Sized>
    AgentLoop<'_, '_, Provider, Tools>
{
    pub(crate) fn capture_compaction_context<Scoped: ModelProvider + ?Sized>(
        &self,
        provider: &Scoped,
        reference_limit: usize,
    ) -> Result<crate::domain::session::CompactionContextCapture, AgentLoopError> {
        self.capture_compaction_context_for_task(provider, reference_limit, false, None)
    }

    pub(crate) fn capture_compaction_context_for_task<Scoped: ModelProvider + ?Sized>(
        &self,
        provider: &Scoped,
        reference_limit: usize,
        selected_task: bool,
        protected: Option<&crate::domain::session::ProtectedOperatorContext>,
    ) -> Result<crate::domain::session::CompactionContextCapture, AgentLoopError> {
        let capabilities = provider.capabilities();
        let policy = self.context_planner.window_policy(&capabilities)?;
        let mut fragments = Vec::new();
        if selected_task {
            let protected = protected.ok_or(AgentLoopError::ContextStateMismatch)?;
            fragments.extend(self.selected_constraints.for_history(
                self.selected_history.as_ref(),
                &[
                    &protected.retained_provenance,
                    &protected.detached_provenance,
                ],
            ));
        } else if let Some(world) = &self.world_state {
            fragments.push(world.full_fragment_with_limit(policy.state_section_bytes())?);
        }
        if let Some(snapshot) = &self.project_snapshot {
            fragments.push(snapshot.full_fragment_with_limit(policy.state_section_bytes())?);
        }
        let memory = if selected_task {
            &self.selected_core_memory
        } else {
            &self.project_memory_core
        };
        if let Some(memory) = memory {
            fragments.push(memory.clone());
        }
        Ok(if selected_task {
            crate::domain::session::CompactionContextCapture::new_task(
                capabilities,
                self.context_planner.budget_policy(),
                reference_limit,
                fragments,
            )
        } else {
            crate::domain::session::CompactionContextCapture::new(
                capabilities,
                self.context_planner.budget_policy(),
                reference_limit,
                fragments,
            )
        })
    }
}

pub(crate) fn compile_captured_compaction_source(
    capture: &crate::domain::session::CompactionContextCapture,
    source: &SessionSourceIdentity,
    source_data: &str,
    references: &[EventId],
    output_limit: usize,
) -> Result<crate::domain::context::ContextPlan, AgentLoopError> {
    let provenance = ContextProvenance::new(
        ContextSlot::UserInput,
        ContextOrigin::SessionProjection,
        SemanticClass::DerivedProjection,
        AuthorityLevel::DerivedNonEvidence,
        RetentionClass::Ephemeral,
        FreshnessIdentity::new(source.stream_hash())?.with_producer_version("model-compaction:1"),
        ContextReferences::new(Vec::new(), Vec::new(), Vec::new(), references.to_vec())?,
    )?;
    let mut fragments = vec![
        ContextFragment::new(
            ContextFragmentId::new("compaction:instructions")?,
            ContextFragmentSource::BasePrompt,
            ContextFragmentPriority::Required,
            FragmentCachePolicy::ContentAddressed,
            ContextFragmentContent::Instructions {
                text: crate::domain::session::compaction_instructions(output_limit),
            },
        )?,
        ContextFragment::new(
            ContextFragmentId::new("compaction:source")?,
            ContextFragmentSource::UserInput,
            ContextFragmentPriority::Required,
            FragmentCachePolicy::DoNotCache,
            ContextFragmentContent::ContentItems {
                items: vec![ContentItem::user_text(source_data)],
            },
        )?
        .with_provenance(provenance.clone())?
        .with_content_item_provenance(0, provenance)?,
    ];
    fragments.extend(capture.fragments()?);
    let planner =
        crate::domain::context::ContextPlanner::current().with_budget_policy(capture.policy);
    Ok(planner.compile(
        fragments,
        planner.tool_free_compaction_budget(&capture.capabilities)?,
        &capture.capabilities,
        Vec::new(),
        ModelRequestRequirements::default(),
        None,
    )?)
}

impl<Provider: ModelProvider + ?Sized, Tools: ToolExecutor + ?Sized>
    AgentLoop<'_, '_, Provider, Tools>
{
    pub(crate) async fn sample_compaction(
        &self,
        input: CompactionSampleInput<'_>,
        record_attempt: &mut dyn FnMut(RequestAuditAttempt) -> Result<EventId, AgentLoopError>,
    ) -> Result<CompactionSampleOutput, AgentLoopError> {
        if input.cancellation.is_cancelled() {
            return Err(AgentLoopError::CompactionCancelled);
        }
        self.history.ensure_all_tool_calls_paired()?;
        let capabilities = input.provider.capabilities();
        let minimum_output_bytes = minimum_compaction_handoff_bytes();
        let graph = &input.graph;
        let limit = graph
            .plan
            .nodes()
            .iter()
            .find(|node| node.id == graph.node_id)
            .map(|node| node.output_limit);
        let output_limit = limit
            .filter(|remaining| *remaining >= minimum_output_bytes)
            .ok_or_else(|| AgentLoopError::CompactionSummaryRejected {
                reason: "node leaves less than the minimum valid handoff capacity".to_owned(),
            })?;
        let scope = graph.plan.node_provider_session(graph.node_id)?;
        let plan = compile_captured_compaction_source(
            graph.capture,
            input.source,
            &input.source_data,
            &input.references,
            output_limit,
        )?;
        let no_tools = ResolvedToolPlan::resolve(
            Vec::new(),
            &ToolExposurePlan::default(),
            &DeferredActivationSet::default(),
        )?;
        let step_id = scope.id.to_string();
        let turn = TurnContext::new(
            TurnId::new(&step_id),
            self.model.clone(),
            self.profile.clone(),
            scope.clone(),
            capabilities.context_window,
            &[],
        )?;
        let step = SamplingStepPlan::new(
            step_id,
            TurnConfigurationSnapshot::new(turn, None, None, input.agent_model_profile.cloned()),
            plan,
            no_tools,
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
        .bind_sampling_step(&step, step.context_plan().delivery_map().clone())?;
        let request = ModelRequest::full(model_input)
            .with_opaque_continuation_scope(OpaqueContinuationScope::for_provider_session(&scope))
            .with_continuation_retention(ContinuationRetention::SingleSample);
        let mut final_audit = None;
        let bytes = std::cell::Cell::new(0usize);
        let mut observe = |event: &crate::domain::model::ModelEvent| {
            if let crate::domain::model::ModelEvent::TextDelta { text } = event {
                bytes.set(bytes.get() + text.len());
                if bytes.get() > output_limit {
                    return Err(ModelStreamingError::CompactionSummaryTooLarge {
                        actual: bytes.get(),
                        maximum: output_limit,
                    });
                }
            }
            Ok(())
        };
        let outcome = prepare_and_consume_with_retry(
            input.provider,
            input.cancellation.clone(),
            || {
                input
                    .provider
                    .prepare(request.clone())
                    .map_err(ModelStreamingError::Provider)
            },
            || {
                input
                    .request_budget
                    .admit(ProviderSessionRole::Primary)
                    .map_err(|limit| AgentLoopError::ModelRequestLimit { limit })?;
                bytes.set(0);
                final_audit = Some(record_attempt(attempt.clone())?);
                Ok(())
            },
            &mut observe,
        )
        .await
        .map_err(|error| match error {
            ModelRetryError::Attempt(error) => error,
            ModelRetryError::Streaming(error) => AgentLoopError::Model(error),
        })?;
        let ModelStreamOutcome::Completed { events, .. } = outcome else {
            return Err(AgentLoopError::CompactionCancelled);
        };
        if input.cancellation.is_cancelled() {
            return Err(AgentLoopError::CompactionCancelled);
        }
        let sample = ModelSample::from_events(events);
        if !sample.tool_calls().is_empty() {
            return Err(AgentLoopError::CompactionSummaryRejected {
                reason: "summary returned a Tool Call; none executed".to_owned(),
            });
        }
        let handoff: Handoff = serde_json::from_str(&sample.assistant_text()).map_err(|error| {
            AgentLoopError::CompactionSummaryRejected {
                reason: format!("invalid summary JSON ({:?})", error.classify()),
            }
        })?;
        let narrative = NarrativeSummary::new(
            input.current_goal,
            handoff.concise_progress,
            handoff.unresolved_questions,
            handoff.next_step_notes,
        )
        .map_err(|error| AgentLoopError::CompactionSummaryRejected {
            reason: error.to_string(),
        })?;
        let audit = final_audit.expect("completed compaction has its durable request audit");
        let sample_record = CompactionModelSample::new_v2(
            input.source.clone(),
            audit,
            narrative,
            input.byte_limit,
            graph.plan_blob_id.clone(),
            graph.plan,
            graph.node_id,
            sample.assistant_text(),
        )
        .map_err(|error| AgentLoopError::CompactionSummaryRejected {
            reason: error.to_string(),
        })?;
        Ok(CompactionSampleOutput {
            sample: sample_record,
            usage: sample.usage(),
        })
    }
}
