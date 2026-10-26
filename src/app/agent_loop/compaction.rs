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
    fn delivery_input_estimate(&self) -> Result<Option<u32>, AgentLoopError> {
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

    pub(crate) fn preflight_tool_batch_compaction(
        &self,
        candidate: &SessionCompaction,
        active: &ToolBatchContext<'_>,
    ) -> Result<bool, AgentLoopError> {
        use crate::domain::context::{ContextDecision, ContextPlannerError};
        let retained = candidate.retained_history();
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
            return Ok(false);
        };
        let capabilities = self.provider.capabilities();
        let budget = self.context_planner.default_budget(&capabilities)?;
        let empty = std::collections::BTreeMap::new();
        let candidate =
            match context_state::compile_context(context_state::ContextCompilationInput {
                prompt_composition: &self.prompt_composition,
                world_state: self.world_state.as_ref(),
                project_snapshot: self.project_snapshot.as_ref(),
                project_memory_core: self.project_memory_core.as_ref(),
                session_summary: Some(candidate.summary()),
                history: retained,
                history_provenance: &empty,
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
            }) {
                Ok((plan, _)) => plan,
                Err(AgentLoopError::ContextPlanner(
                    ContextPlannerError::RequiredFragmentsExceedBudget { .. }
                    | ContextPlannerError::RenderedInputExceedsBudget { .. },
                )) => return Ok(false),
                Err(error) => return Err(error),
            };
        if !candidate
            .context_decision_manifest()
            .candidates()
            .iter()
            .all(|item| matches!(item.decision(), ContextDecision::Include))
        {
            return Ok(false);
        }
        let target = budget.working_set_input_tokens() / 2;
        Ok(self
            .context_planner
            .estimate_rendered_model_input(&candidate.model_input())?
            <= target)
    }

    /// Recompile the next-Turn logical input, including the final completed
    /// assistant output, without sampling or mutating Provider delivery state.
    /// Known required/rendered budget failures mean compaction may be useful;
    /// all other errors retain their original failure path.
    pub(crate) fn fresh_compaction_context_probe(
        &self,
    ) -> Result<Option<crate::domain::context::ContextPlan>, AgentLoopError> {
        self.compile_compaction_context(self.history.items(), self.session_summary.as_ref())
    }

    /// Compile a proposed next-Turn context without changing runtime state or
    /// contacting a Provider. The regular Context owner budgets all prompts,
    /// tools, facts and retained History; a tiny future input is only a probe,
    /// never a guarantee that an arbitrarily large next input will fit.
    pub(crate) fn preflight_compaction(
        &self,
        compaction: &SessionCompaction,
        require_reduction: bool,
    ) -> Result<bool, AgentLoopError> {
        use crate::domain::context::{ContextDecision, ContextFragmentSource};
        let candidate = match self
            .compile_compaction_context(compaction.retained_history(), Some(compaction.summary()))
        {
            Ok(Some(plan)) => plan,
            Ok(None) => return Ok(false),
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
            return Ok(false);
        }
        if !require_reduction {
            return Ok(true);
        }
        let previous = match self.fresh_compaction_context_probe() {
            Ok(Some(plan)) => plan,
            Ok(None) => return Ok(true),
            Err(error) => return Err(error),
        };
        // Compare planner demand rather than serialized event bytes or item
        // count: a compact-looking Summary can cost more than short History.
        let demand = |plan: &crate::domain::context::ContextPlan| {
            let manifest = plan.context_decision_manifest();
            u64::from(manifest.fixed_input_tokens())
                + manifest
                    .candidates()
                    .iter()
                    .map(|entry| u64::from(entry.token_estimate()))
                    .sum::<u64>()
        };
        let previous_demand =
            demand(&previous).max(u64::from(self.delivery_input_estimate()?.unwrap_or(0)));
        Ok(demand(&candidate) < previous_demand)
    }

    fn compile_compaction_context(
        &self,
        items: &[ContentItem],
        summary: Option<&SessionSummary>,
    ) -> Result<Option<crate::domain::context::ContextPlan>, AgentLoopError> {
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
        history.push(ContentItem::user_text("Continue."));
        let empty_provenance = std::collections::BTreeMap::new();
        match context_state::compile_context(context_state::ContextCompilationInput {
            correction_feedback: &[],
            prompt_composition: prompt,
            world_state: self.world_state.as_ref(),
            project_snapshot: self.project_snapshot.as_ref(),
            project_memory_core: self.project_memory_core.as_ref(),
            session_summary: summary,
            history: &history,
            history_provenance: &empty_provenance,
            turn_history_start: items.len(),
            detached_input: None,
            planner: &self.context_planner,
            cache: None,
            budget,
            capabilities: &capabilities,
            tool_specs: tool_plan.advertised_specs(),
            prior_manifest: None,
        }) {
            Ok((plan, _)) => Ok(Some(plan)),
            Err(AgentLoopError::ContextPlanner(
                ContextPlannerError::RequiredFragmentsExceedBudget { .. }
                | ContextPlannerError::RenderedInputExceedsBudget { .. },
            )) => Ok(None),
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
        let normalized_items = session.normalized_history_items()?;
        let history = ConversationHistory::from_complete_content_items(&normalized_items)?;
        self.history = history;
        self.history_provenance.clear();
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
