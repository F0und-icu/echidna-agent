//! Successful-sample usage, bound to the exact durable request audit.

use super::*;
use crate::domain::context::ProviderSessionRef;
use crate::domain::model::ModelUsage;

impl DurableChatSession {
    pub(super) fn record_operator_input(
        &self,
        items: &[crate::domain::model::ContentItem],
        provenance: &mut std::collections::BTreeMap<
            usize,
            crate::domain::context::ContextProvenance,
        >,
    ) -> Result<(), ChatError> {
        self.with_projection(|projection, store, stream_id| {
            let boundary = projection
                .state()
                .accepted_operator_input(items, provenance)?;
            let input_provenance = boundary.input_provenance()?;
            let sequence = store.next_sequence()?;
            let event = SessionEvent::OperatorTurnBoundaryRecorded { boundary }.into_envelope(
                event_id(
                    self.project.project_id(),
                    &self.session_id,
                    sequence.get(),
                    "operator-input-accepted",
                ),
                stream_id,
                sequence,
                occurred_at(),
            )?;
            projection
                .append(store, event)
                .map_err(ChatError::from_session_append)?;
            *provenance = input_provenance;
            Ok(())
        })
    }

    pub(super) fn record_operator_completed(&self, audit_id: &EventId) -> Result<(), ChatError> {
        self.with_projection(|projection, store, stream_id| {
            let audit = projection
                .state()
                .request_audits()
                .last()
                .filter(|audit| audit.audit_event_id() == audit_id)
                .ok_or(ChatError::RuntimeHistoryMismatch)?;
            let service = RequestAuditService::new(
                &self.artifacts,
                RequestAuditBinding::new(self.project.project_id().clone(), stream_id.clone())?,
            );
            let invalid = |message: String| SessionError::CompactionContextProvenance { message };
            let Some(verified) = service.reconstruct_full_logical(audit).map_err(|error| {
                invalid(format!("completed audit reconstruction failed: {error:?}"))
            })?
            else {
                return Ok(());
            };
            let Some(chain) = projection.state().model_delivery_chain(audit_id)? else {
                return Ok(());
            };
            let mut delivered = std::collections::BTreeMap::new();
            for accepted in chain {
                let manifest = projection
                    .state()
                    .request_audits()
                    .iter()
                    .find(|audit| audit.audit_event_id() == accepted.audit_event_id())
                    .ok_or(ChatError::RuntimeHistoryMismatch)?;
                let input = service.reconstruct(manifest).map_err(|error| {
                    invalid(format!(
                        "accepted delivery reconstruction failed: {error:?}"
                    ))
                })?;
                let map = manifest
                    .context_delivery_map()
                    .ok_or_else(|| invalid("accepted delivery has no provenance map".to_owned()))?;
                map.validate_against(&input).map_err(|error| {
                    invalid(format!("accepted delivery provenance mismatch: {error}"))
                })?;
                delivered.insert(accepted.audit_event_id().clone(), input);
            }
            let Some(boundary) = projection
                .state()
                .completed_operator_turn(&verified, &delivered)?
            else {
                return Ok(());
            };
            let sequence = store.next_sequence()?;
            let event = SessionEvent::OperatorTurnBoundaryRecorded { boundary }.into_envelope(
                event_id(
                    self.project.project_id(),
                    &self.session_id,
                    sequence.get(),
                    "operator-turn-completed",
                ),
                stream_id,
                sequence,
                occurred_at(),
            )?;
            projection
                .append(store, event)
                .map_err(ChatError::from_session_append)?;
            Ok(())
        })
    }
    pub(super) fn record_model_delivery(
        &self,
        audit_id: &EventId,
        previous: Option<EventId>,
    ) -> Result<(), ChatError> {
        self.with_projection(|projection, store, stream_id| {
            let delivery = projection.state().new_model_delivery(audit_id, previous)?;
            let sequence = store.next_sequence()?;
            let event = SessionEvent::ModelDeliveryCommitted { delivery }.into_envelope(
                event_id(
                    self.project.project_id(),
                    &self.session_id,
                    sequence.get(),
                    "model-delivery",
                ),
                stream_id,
                sequence,
                occurred_at(),
            )?;
            projection
                .append(store, event)
                .map_err(ChatError::from_session_append)?;
            Ok(())
        })
    }
    pub(super) fn record_model_usage(
        &self,
        request_audit_event_id: &EventId,
        provider_session: &ProviderSessionRef,
        usage: ModelUsage,
    ) -> Result<(), ChatError> {
        self.with_projection(|projection, store, stream_id| {
            let sequence = store.next_sequence()?;
            let event = SessionEvent::ModelUsageRecorded {
                request_audit_event_id: request_audit_event_id.clone(),
                provider_session: provider_session.clone(),
                usage,
            }
            .into_envelope(
                event_id(
                    self.project.project_id(),
                    &self.session_id,
                    sequence.get(),
                    "model-usage",
                ),
                stream_id,
                sequence,
                occurred_at(),
            )?;
            projection
                .append(store, event)
                .map_err(ChatError::from_session_append)?;
            Ok(())
        })?;
        let cached = usage
            .cached_input_tokens
            .map_or_else(|| "unknown".to_owned(), |tokens| tokens.to_string());
        let reasoning = usage
            .reasoning_output_tokens
            .map_or_else(|| "unknown".to_owned(), |tokens| tokens.to_string());
        console_eprintln!(
            "[provider usage {:?}] input={} output={} cached_input={} reasoning_output={} (subsets of totals)",
            provider_session.role,
            usage.input_tokens,
            usage.output_tokens,
            cached,
            reasoning,
        );
        Ok(())
    }
}
