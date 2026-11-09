//! Replays accepted task deliveries through the live delivery and Baseline owners.

use super::context_state::{self, DeliveryChoiceInput, StateDiffDeliveryInput};
use super::task_context_sample::TaskReplayInputs;
use super::*;
use crate::domain::request_audit::{
    RequestAuditDeliveryMode, RequestAuditDeliveryReason, RequestAuditManifest,
};

struct PendingDelivery {
    audit: EventId,
    step: String,
    snapshot: StepSnapshot,
    input: ModelInputIR,
    map: ContextDeliveryMap,
    sources: TaskReplayInputs,
    incremental: bool,
}

pub(crate) struct TaskDeliveryReplay {
    provider: ProviderSession,
    committed: Option<CommittedPrimaryDelivery>,
    pending: Option<PendingDelivery>,
    proof6: bool,
}

impl TaskDeliveryReplay {
    pub(crate) fn new(reference: &crate::domain::context::ProviderSessionRef) -> Self {
        Self {
            provider: ProviderSession::new(
                reference.id.clone(),
                reference.role,
                reference.generation,
            ),
            committed: None,
            pending: None,
            proof6: false,
        }
    }

    pub(crate) fn set_proof6(&mut self, proof6: bool) {
        self.proof6 = proof6;
    }

    pub(crate) fn reset_compaction(&mut self) {
        self.provider
            .reset(BaselineInvalidationReason::CompactionReset);
        self.committed = None;
        self.pending = None;
    }

    pub(crate) fn verify_attempt(
        &mut self,
        manifest: &RequestAuditManifest,
        actual: &ModelInputIR,
        sources: TaskReplayInputs,
        capabilities: &crate::domain::model::ModelCapabilities,
    ) -> Result<(), AgentLoopError> {
        if manifest.capability_mapping()
            != &crate::domain::request_audit::RequestAuditCapabilityMapping::from_capabilities(
                capabilities,
            )?
        {
            return Err(AgentLoopError::ContextStateMismatch);
        }
        let step = manifest
            .sampling_step_id()
            .ok_or(AgentLoopError::ContextStateMismatch)?;
        let (turn_id, index) = step
            .rsplit_once(":step:")
            .ok_or(AgentLoopError::ContextStateMismatch)?;
        let request_index: usize = index
            .parse()
            .map_err(|_| AgentLoopError::ContextStateMismatch)?;
        if turn_id.is_empty() || index != request_index.to_string() {
            return Err(AgentLoopError::ContextStateMismatch);
        }
        let reference = manifest
            .provider_session()
            .ok_or(AgentLoopError::ContextStateMismatch)?;
        if reference != self.provider.reference() {
            return Err(AgentLoopError::ContextStateMismatch);
        }
        let turn = TurnContext::new(
            TurnId::new(turn_id),
            manifest.model().clone(),
            manifest.profile().clone(),
            reference.clone(),
            capabilities.context_window,
            sources.tools.advertised_specs(),
        )?;
        let full = sources.plan.model_input();
        let full_map = sources.plan.delivery_map();
        let prior_turn = self
            .committed
            .as_ref()
            .and_then(|prior| prior.step_id.rsplit_once(":step:"))
            .map(|(turn, _)| turn);
        let expired = self
            .committed
            .as_ref()
            .filter(|prior| {
                prior_turn != Some(turn_id) && prior.unactivated_tool_plan == sources.tools
            })
            .map(|prior| &prior.unactivated_tool_plan);
        let activation_or_expiry = sources.tools != sources.unactivated_tools || expired.is_some();
        let (mut candidate, mut candidate_map, mut snapshot, assessment) =
            context_state::choose_delivery(DeliveryChoiceInput {
                state_diff: StateDiffDeliveryInput {
                    correction_feedback: &sources.correction_feedback,
                    selected_constraints: &sources.selected_constraints,
                    step_id: step,
                    full_input: &full,
                    full_delivery_map: full_map,
                    current_plan: &sources.plan,
                    planner: &sources.planner,
                    budget: sources.planner.default_budget(capabilities)?,
                    committed_delivery: self.committed.as_ref(),
                    world_state: None,
                    project_snapshot: sources.project_snapshot.as_ref(),
                    project_memory_core: sources.project_memory_core.as_ref(),
                    history: &sources.history,
                    history_provenance: &sources.history_provenance,
                    turn_history_start: sources.turn_history_start,
                },
                provider_session: &self.provider,
                turn: &turn,
                request_index,
                capabilities,
                renderer: sources.plan.prompt_renderer(),
                activation_or_expiry,
                expired_plan: expired,
            })?;
        let provider_rejected = manifest.delivery().reason()
            == RequestAuditDeliveryReason::Full(BaselineInvalidationReason::ProviderRejected);
        let (expected, expected_map) = if provider_rejected {
            if self.proof6 {
                match assessment {
                    BaselineAssessment::AppendOnlyCandidate { .. } => {
                        if !self
                            .pending
                            .as_ref()
                            .is_some_and(|pending| pending.incremental && pending.step == step)
                        {
                            return Err(AgentLoopError::ContextStateMismatch);
                        }
                        self.provider
                            .reset(BaselineInvalidationReason::ProviderRejected);
                    }
                    BaselineAssessment::FullRequest {
                        reason: BaselineInvalidationReason::ProviderRejected,
                    } => {}
                    _ => return Err(AgentLoopError::ContextStateMismatch),
                }
            } else if !matches!(assessment, BaselineAssessment::AppendOnlyCandidate { .. }) {
                return Err(AgentLoopError::ContextStateMismatch);
            }
            if !candidate.content_items.iter().any(|item| {
                matches!(
                    item,
                    ContentItem::Reasoning { .. } | ContentItem::OpaqueContinuation { .. }
                )
            }) {
                candidate = full.clone();
                candidate_map = full_map.clone();
                snapshot = StepSnapshot::capture_with_renderer(
                    &turn,
                    request_index,
                    &candidate,
                    capabilities,
                    sources.plan.prompt_renderer(),
                    InternalRequestMetadata::default(),
                )?;
            }
            (candidate.clone(), candidate_map.clone())
        } else {
            let mode_matches = matches!(
                (&assessment, manifest.delivery().mode()),
                (
                    BaselineAssessment::FullRequest { .. },
                    RequestAuditDeliveryMode::Full
                ) | (
                    BaselineAssessment::AppendOnlyCandidate { .. },
                    RequestAuditDeliveryMode::Incremental
                )
            );
            if !mode_matches {
                return Err(AgentLoopError::ContextStateMismatch);
            }
            if self.proof6
                && matches!(
                    assessment,
                    BaselineAssessment::FullRequest {
                        reason: BaselineInvalidationReason::ProviderRejected,
                    }
                )
            {
                return Err(AgentLoopError::ContextStateMismatch);
            }
            context_state::project_delivery(&candidate, &candidate_map, &assessment)?
        };
        if actual != &expected || manifest.context_delivery_map() != Some(&expected_map) {
            return Err(AgentLoopError::ContextStateMismatch);
        }
        self.pending = Some(PendingDelivery {
            audit: manifest.audit_event_id().clone(),
            step: step.to_owned(),
            snapshot,
            input: candidate,
            map: candidate_map,
            sources,
            incremental: manifest.delivery().mode() == RequestAuditDeliveryMode::Incremental,
        });
        Ok(())
    }

    pub(crate) fn accept(
        &mut self,
        audit: &EventId,
        step: &str,
        output: Vec<ContentItem>,
        output_history_items: usize,
    ) -> Result<(), AgentLoopError> {
        let mut pending = self
            .pending
            .take()
            .ok_or(AgentLoopError::ContextStateMismatch)?;
        if &pending.audit != audit || pending.step != step {
            return Err(AgentLoopError::ContextStateMismatch);
        }
        self.provider
            .record_completed_sample(&pending.snapshot, output.clone())?;
        pending.input.content_items.extend(output.clone());
        pending.map.append_model_output_items(step, &output)?;
        pending.map.validate_against(&pending.input)?;
        self.committed = Some(CommittedPrimaryDelivery {
            step_id: pending.step,
            continuation: None,
            context_manifest: pending.sources.plan.full_logical_manifest().clone(),
            input: pending.input,
            map: pending.map,
            unactivated_tool_plan: pending.sources.unactivated_tools,
            history_len: pending.sources.history.len() + output_history_items,
            world_state: None,
            project_snapshot: pending.sources.project_snapshot,
            project_memory_core: pending.sources.project_memory_core,
        });
        Ok(())
    }
}
