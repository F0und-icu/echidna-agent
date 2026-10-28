//! Durable Session Compaction application for the single AgentLoop.

use crate::domain::context::BaselineInvalidationReason;
use crate::domain::events::StreamId;
use crate::domain::history::ConversationHistory;
use crate::domain::model::ContentItem;
use crate::domain::session::{
    CompactionValidationContext, SessionCompaction, SessionLifecycle, SessionState, SessionSummary,
};
use crate::domain::tools::DeferredActivationSet;
use crate::ports::model::ModelProvider;
use crate::ports::tool::ToolExecutor;

use super::{AgentLoop, AgentLoopConfigurationError, AgentLoopError, context_state};

/// Original operator input is retained only in the active Turn's derived
/// Context after its durable History prefix is compacted. It is never appended
/// as a second History event.
pub(crate) struct ToolBatchContext<'a> {
    pub(super) turn_id: &'a crate::domain::context::TurnId,
    pub(super) history_end: usize,
    pub(super) history_start: usize,
    pub(super) original_input: &'a [ContentItem],
    pub(super) input_provenance:
        &'a std::collections::BTreeMap<usize, crate::domain::context::ContextProvenance>,
    pub(super) detached: bool,
    pub(super) tool_specs: &'a [crate::domain::tools::ToolSpec],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct AppliedCompaction {
    stream_id: StreamId,
    revision: u64,
    fingerprint: String,
}

/// A fully admissible candidate's estimated complete logical demand.
pub(crate) struct CompactionFit {
    pub(crate) demand: u64,
    pub(crate) target: u32,
}

fn plan_demand(plan: &crate::domain::context::ContextPlan) -> u64 {
    let manifest = plan.context_decision_manifest();
    u64::from(manifest.fixed_input_tokens())
        + manifest
            .candidates()
            .iter()
            .map(|entry| u64::from(entry.token_estimate()))
            .sum::<u64>()
}

impl AppliedCompaction {
    pub(super) fn from_compaction(
        stream_id: StreamId,
        compaction: &SessionCompaction,
    ) -> Result<Self, crate::domain::session::SessionError> {
        Ok(Self {
            stream_id,
            revision: compaction.summary().revision(),
            fingerprint: compaction.fingerprint()?,
        })
    }
}

impl<'provider, 'tools, Provider, Tools> AgentLoop<'provider, 'tools, Provider, Tools>
where
    Provider: ModelProvider + ?Sized,
    Tools: ToolExecutor + ?Sized,
{
    pub(super) fn protected_without_active(
        &self,
        active: &std::collections::BTreeMap<usize, crate::domain::context::ContextProvenance>,
    ) -> (
        Vec<ContentItem>,
        std::collections::BTreeMap<usize, crate::domain::context::ContextProvenance>,
    ) {
        let active_ids = active
            .values()
            .filter_map(|p| p.references().source_event_refs().last())
            .collect::<std::collections::BTreeSet<_>>();
        let mut items = Vec::new();
        let mut provenance = std::collections::BTreeMap::new();
        for (index, item) in self.protected_input.detached_items.iter().enumerate() {
            let p = &self.protected_input.detached_provenance[&index];
            if p.references()
                .source_event_refs()
                .last()
                .is_some_and(|id| active_ids.contains(id))
            {
                continue;
            }
            provenance.insert(items.len(), p.clone());
            items.push(item.clone());
        }
        (items, provenance)
    }

    pub(crate) fn delivery_input_estimate(&self) -> Result<Option<u32>, AgentLoopError> {
        let Some(committed) = &self.committed_delivery else {
            return Ok(None);
        };
        let mut input = committed.input.clone();
        input
            .content_items
            .extend_from_slice(&self.history.items()[committed.history_len..]);
        Ok(Some(
            self.context_planner.estimate_rendered_model_input(&input)?,
        ))
    }

    pub(crate) fn delivery_needs_compaction(&self) -> Result<bool, AgentLoopError> {
        let threshold = self
            .context_planner
            .default_budget(&self.provider.capabilities())?
            .working_set_input_tokens();
        Ok(self
            .delivery_input_estimate()?
            .is_some_and(|estimate| estimate >= threshold))
    }

    pub(crate) fn batch_needs_compaction(
        &self,
        active: &ToolBatchContext<'_>,
    ) -> Result<bool, AgentLoopError> {
        let budget = self
            .context_planner
            .default_budget(&self.provider.capabilities())?;
        let Some(plan) = self.current_batch_context(active)? else {
            return Ok(true);
        };
        let demand =
            plan_demand(&plan).max(u64::from(self.delivery_input_estimate()?.unwrap_or(0)));
        Ok(demand >= u64::from(budget.working_set_input_tokens()))
    }

    fn current_batch_context(
        &self,
        active: &ToolBatchContext<'_>,
    ) -> Result<Option<crate::domain::context::ContextPlan>, AgentLoopError> {
        use crate::domain::context::ContextPlannerError;
        let capabilities = self.provider.capabilities();
        let budget = self.context_planner.default_budget(&capabilities)?;
        let protected = self.protected_without_active(active.input_provenance);
        match context_state::compile_context(context_state::ContextCompilationInput {
            prompt_composition: &self.prompt_composition,
            world_state: self.world_state.as_ref(),
            project_snapshot: self.project_snapshot.as_ref(),
            project_memory_core: self.project_memory_core.as_ref(),
            session_summary: self.session_summary.as_ref(),
            history: self.history.items(),
            history_provenance: &self.history_provenance,
            protected_input: Some((&protected.0, &protected.1)),
            turn_history_start: active.history_start,
            detached_input: active
                .detached
                .then_some((active.original_input, active.input_provenance)),
            planner: &self.context_planner,
            cache: None,
            budget,
            capabilities: &capabilities,
            tool_specs: active.tool_specs,
            prior_manifest: None,
            correction_feedback: &[],
            capacity_breakdown: None,
        }) {
            Ok((plan, _)) => Ok(Some(plan)),
            Err(AgentLoopError::ContextPlanner(
                ContextPlannerError::FixedInputExceedsBudget { .. }
                | ContextPlannerError::RequiredFragmentsExceedBudget { .. }
                | ContextPlannerError::RenderedInputExceedsBudget { .. },
            )) => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub(crate) fn preflight_tool_batch_compaction(
        &self,
        candidate: &SessionCompaction,
        active: &ToolBatchContext<'_>,
        protected: &crate::domain::session::ProtectedOperatorContext,
    ) -> Result<Option<CompactionFit>, AgentLoopError> {
        self.preflight_batch_context(candidate, active, &self.prompt_composition, true, protected)
    }

    pub(crate) fn preflight_checkpoint_definition(
        &self,
        candidate: &SessionCompaction,
        active: &ToolBatchContext<'_>,
        prompt: &crate::domain::research_control::PromptComposition,
        protected: &crate::domain::session::ProtectedOperatorContext,
    ) -> Result<Option<CompactionFit>, AgentLoopError> {
        self.preflight_batch_context(candidate, active, prompt, false, protected)
    }

    fn preflight_batch_context(
        &self,
        candidate: &SessionCompaction,
        active: &ToolBatchContext<'_>,
        prompt: &crate::domain::research_control::PromptComposition,
        require_reduction: bool,
        protected: &crate::domain::session::ProtectedOperatorContext,
    ) -> Result<Option<CompactionFit>, AgentLoopError> {
        use crate::domain::context::{ContextDecision, ContextPlannerError};
        let retained = candidate.retained_history();
        // A historical compaction may have omitted the latest complete batch.
        // Its durable suffix cannot be reintroduced inside an active Turn.
        if retained.len() > self.history.items().len() {
            return Ok(None);
        }
        // The Session owner selects a suffix. Keep the active input whole, or
        // detach it whole; never duplicate only its remaining image suffix.
        if !self.history.items().ends_with(retained) {
            return Err(AgentLoopError::ContextStateMismatch);
        }
        let dropped = self.history.items().len() - retained.len();
        let (start, detached) = if active.detached {
            (0, true)
        } else if dropped <= active.history_start {
            (active.history_start - dropped, false)
        } else if dropped >= active.history_start + active.original_input.len() {
            (0, true)
        } else {
            return Ok(None);
        };
        let capabilities = self.provider.capabilities();
        let budget = self.context_planner.default_budget(&capabilities)?;
        let active_ids = active
            .input_provenance
            .values()
            .filter_map(|p| p.references().source_event_refs().last())
            .collect::<std::collections::BTreeSet<_>>();
        let mut protected_items = Vec::new();
        let mut protected_provenance = std::collections::BTreeMap::new();
        for (index, item) in protected.detached_items.iter().enumerate() {
            let p = &protected.detached_provenance[&index];
            if p.references()
                .source_event_refs()
                .last()
                .is_some_and(|id| active_ids.contains(id))
            {
                continue;
            }
            protected_provenance.insert(protected_items.len(), p.clone());
            protected_items.push(item.clone());
        }
        let candidate =
            match context_state::compile_context(context_state::ContextCompilationInput {
                prompt_composition: prompt,
                world_state: self.world_state.as_ref(),
                project_snapshot: self.project_snapshot.as_ref(),
                project_memory_core: self.project_memory_core.as_ref(),
                session_summary: Some(candidate.summary()),
                history: retained,
                history_provenance: &protected.retained_provenance,
                protected_input: Some((&protected_items, &protected_provenance)),
                turn_history_start: start,
                detached_input: detached
                    .then_some((active.original_input, active.input_provenance)),
                planner: &self.context_planner,
                cache: None,
                budget,
                capabilities: &capabilities,
                tool_specs: active.tool_specs,
                prior_manifest: None,
                correction_feedback: &[],
                capacity_breakdown: None,
            }) {
                Ok((plan, _)) => plan,
                Err(AgentLoopError::ContextPlanner(
                    ContextPlannerError::FixedInputExceedsBudget { .. }
                    | ContextPlannerError::RequiredFragmentsExceedBudget { .. }
                    | ContextPlannerError::RenderedInputExceedsBudget { .. },
                )) => return Ok(None),
                Err(error) => return Err(error),
            };
        if !candidate
            .context_decision_manifest()
            .candidates()
            .iter()
            .all(|item| matches!(item.decision(), ContextDecision::Include))
        {
            return Ok(None);
        }
        let demand = plan_demand(&candidate);
        if require_reduction {
            let previous = self
                .current_batch_context(active)?
                .as_ref()
                .map(plan_demand);
            let delivery = self.delivery_input_estimate()?.map(u64::from);
            if let Some(previous) = previous.map(|value| value.max(delivery.unwrap_or(0)))
                && demand >= previous
            {
                return Ok(None);
            }
        }
        Ok(Some(CompactionFit {
            demand,
            target: if require_reduction {
                budget.compaction_target_input_tokens()
            } else {
                u32::MAX
            },
        }))
    }

    /// Recompile the next-Turn logical input, including the final completed
    /// assistant output, without sampling or mutating Provider delivery state.
    /// Known required/rendered budget failures mean compaction may be useful;
    /// all other errors retain their original failure path.
    pub(crate) fn fresh_compaction_context_probe(
        &self,
    ) -> Result<Option<crate::domain::context::ContextPlan>, AgentLoopError> {
        Ok(self.fresh_compaction_context_diagnostic()?.ok())
    }

    pub(crate) fn fresh_compaction_context_diagnostic(
        &self,
    ) -> Result<
        Result<
            crate::domain::context::ContextPlan,
            (
                crate::domain::context::ContextPlannerError,
                Option<crate::domain::context::ContextCapacityBreakdown>,
            ),
        >,
        AgentLoopError,
    > {
        self.compile_compaction_context(
            self.history.items(),
            self.session_summary.as_ref(),
            &self.protected_input,
            true,
        )
    }

    pub(crate) fn context_budget(
        &self,
    ) -> Result<crate::domain::context::ContextBudget, AgentLoopError> {
        Ok(self
            .context_planner
            .default_budget(&self.provider.capabilities())?)
    }

    /// Compile a proposed next-Turn context without changing runtime state or
    /// contacting a Provider. The regular Context owner budgets all prompts,
    /// tools, facts and retained History; a tiny future input is only a probe,
    /// never a guarantee that an arbitrarily large next input will fit.
    pub(crate) fn preflight_compaction(
        &self,
        compaction: &SessionCompaction,
        require_reduction: bool,
        protected: &crate::domain::session::ProtectedOperatorContext,
    ) -> Result<Option<CompactionFit>, AgentLoopError> {
        use crate::domain::context::{ContextDecision, ContextFragmentSource};
        let candidate = match self.compile_compaction_context(
            compaction.retained_history(),
            Some(compaction.summary()),
            protected,
            false,
        ) {
            Ok(Ok(plan)) => plan,
            Ok(Err(_)) => return Ok(None),
            Err(error) => return Err(error),
        };
        let complete = candidate
            .context_decision_manifest()
            .candidates()
            .iter()
            .all(|entry| {
                !matches!(
                    entry.source(),
                    ContextFragmentSource::SessionSummary | ContextFragmentSource::RecentHistory
                ) || matches!(entry.decision(), ContextDecision::Include)
            });
        if !complete {
            return Ok(None);
        }
        let demand = plan_demand(&candidate);
        if require_reduction && let Some(previous) = self.fresh_compaction_context_probe()? {
            let previous_demand =
                plan_demand(&previous).max(u64::from(self.delivery_input_estimate()?.unwrap_or(0)));
            if demand >= previous_demand {
                return Ok(None);
            }
        }
        let budget = self
            .context_planner
            .default_budget(&self.provider.capabilities())?;
        Ok(Some(CompactionFit {
            demand,
            target: if require_reduction {
                budget.compaction_target_input_tokens()
            } else {
                u32::MAX
            },
        }))
    }

    fn compile_compaction_context(
        &self,
        items: &[ContentItem],
        summary: Option<&SessionSummary>,
        protected: &crate::domain::session::ProtectedOperatorContext,
        collect_breakdown: bool,
    ) -> Result<
        Result<
            crate::domain::context::ContextPlan,
            (
                crate::domain::context::ContextPlannerError,
                Option<crate::domain::context::ContextCapacityBreakdown>,
            ),
        >,
        AgentLoopError,
    > {
        use crate::domain::context::ContextPlannerError;
        let capabilities = self.provider.capabilities();
        let budget = self.context_planner.default_budget(&capabilities)?;
        let prompt = self
            .pending_prompt_composition
            .as_ref()
            .unwrap_or(&self.prompt_composition);
        let exposure = prompt.tool_exposure_for_catalog(&self.tools.catalog())?;
        let tool_plan = self
            .tools
            .resolve_tool_plan(&exposure, &DeferredActivationSet::default())?;
        let mut history = items.to_vec();
        let (turn_history_start, detached_input, protected_input) =
            if let Some(index) = protected.retained_provenance.keys().next_back() {
                (
                    *index,
                    None,
                    Some((
                        &protected.detached_items[..],
                        &protected.detached_provenance,
                    )),
                )
            } else if !protected.detached_items.is_empty() {
                (
                    history.len(),
                    Some((
                        &protected.detached_items[..],
                        &protected.detached_provenance,
                    )),
                    None,
                )
            } else {
                history.push(ContentItem::user_text("Continue."));
                (items.len(), None, None)
            };
        let mut breakdown = None;
        match context_state::compile_context(context_state::ContextCompilationInput {
            correction_feedback: &[],
            prompt_composition: prompt,
            world_state: self.world_state.as_ref(),
            project_snapshot: self.project_snapshot.as_ref(),
            project_memory_core: self.project_memory_core.as_ref(),
            session_summary: summary,
            history: &history,
            history_provenance: &protected.retained_provenance,
            protected_input,
            turn_history_start,
            detached_input,
            planner: &self.context_planner,
            cache: None,
            budget,
            capabilities: &capabilities,
            tool_specs: tool_plan.advertised_specs(),
            prior_manifest: None,
            capacity_breakdown: collect_breakdown.then_some(&mut breakdown),
        }) {
            Ok((plan, _)) => Ok(Ok(plan)),
            Err(AgentLoopError::ContextPlanner(
                error @ (ContextPlannerError::FixedInputExceedsBudget { .. }
                | ContextPlannerError::RequiredFragmentsExceedBudget { .. }
                | ContextPlannerError::RenderedInputExceedsBudget { .. }),
            )) => Ok(Err((error, breakdown))),
            Err(error) => Err(error),
        }
    }

    /// Verifies the active Session/Primary binding before a caller appends a
    /// new Compaction fact. The durable Compaction itself is still validated
    /// and applied only by [`Self::apply_compaction`].
    pub(crate) fn validate_compaction_session(
        &self,
        session: &SessionState,
    ) -> Result<(), AgentLoopConfigurationError> {
        self.compaction_session_stream(session).map(|_| ())
    }

    /// Applies a durable Compaction projection. Repeating the same persisted
    /// fact is a no-op; a new fact replaces the active derived Summary and
    /// forces exactly one subsequent Full request.
    pub fn apply_compaction(
        &mut self,
        session: &SessionState,
        validation: Option<&CompactionValidationContext<'_>>,
    ) -> Result<(), AgentLoopConfigurationError> {
        let stream_id = self.compaction_session_stream(session)?;
        let compaction = session
            .latest_compaction()
            .ok_or(AgentLoopConfigurationError::CompactionRequired)?;
        if compaction.summary().requires_reference_validation() {
            let validation = validation
                .ok_or(AgentLoopConfigurationError::CompactionValidationContextRequired)?;
            compaction.validate_references(validation)?;
        }
        if let Some(candidate) = &self.pending_session_summary
            && !compaction.summary().matches_draft(
                candidate,
                validation
                    .ok_or(AgentLoopConfigurationError::CompactionValidationContextRequired)?,
            )?
        {
            return Err(AgentLoopConfigurationError::CompactionSummaryCandidateMismatch);
        }
        let applied = AppliedCompaction::from_compaction(stream_id.clone(), compaction)?;
        if let Some(previous) = &self.applied_compaction {
            if previous == &applied {
                return Ok(());
            }
            if previous.stream_id == applied.stream_id && previous.revision == applied.revision {
                return Err(AgentLoopConfigurationError::ConflictingCompaction {
                    stream_id,
                    revision: applied.revision,
                });
            }
            if previous.stream_id == applied.stream_id && applied.revision < previous.revision {
                return Err(AgentLoopConfigurationError::StaleCompaction {
                    stream_id,
                    applied_revision: previous.revision,
                    received_revision: applied.revision,
                });
            }
        }
        let normalized_items = session.compaction_source_history()?;
        let history = ConversationHistory::from_complete_content_items(&normalized_items)?;
        self.history = history;
        self.protected_input = session.current_protected_operator_context()?;
        self.history_provenance = self.protected_input.retained_provenance.clone();
        self.session_summary = Some(compaction.summary().clone());
        self.pending_session_summary = None;
        self.bound_session_stream = Some(stream_id);
        self.applied_compaction = Some(applied);
        self.provider_session
            .reset(BaselineInvalidationReason::CompactionReset);
        self.clear_committed_primary_delivery_state();
        Ok(())
    }

    fn compaction_session_stream(
        &self,
        session: &SessionState,
    ) -> Result<StreamId, AgentLoopConfigurationError> {
        if session.lifecycle() != SessionLifecycle::Active {
            return Err(AgentLoopConfigurationError::ActiveSessionCompactionRequired);
        }
        let stream_id = session
            .stream_id()
            .cloned()
            .ok_or(AgentLoopConfigurationError::CompactionSessionStreamMissing)?;
        if let Some(expected) = &self.bound_session_stream
            && expected != &stream_id
        {
            return Err(
                AgentLoopConfigurationError::CompactionSessionStreamMismatch {
                    expected: expected.clone(),
                    received: stream_id,
                },
            );
        }
        let expected_provider = session
            .primary_provider_session()
            .ok_or(AgentLoopConfigurationError::ActiveSessionCompactionRequired)?;
        if expected_provider != self.provider_session.reference() {
            return Err(AgentLoopConfigurationError::CompactionProviderMismatch {
                expected: expected_provider.clone(),
                received: self.provider_session.reference().clone(),
            });
        }
        Ok(stream_id)
    }
}
