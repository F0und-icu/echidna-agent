//! Durable accepted Primary samples and their actual continuation lineage.

use serde::{Deserialize, Serialize};

use crate::domain::context::ProviderSessionRef;
use crate::domain::events::EventId;
use crate::domain::model::{ContentItem, MessageRole};
use crate::domain::request_audit::{RequestAuditDeliveryMode, SemanticHash};

use super::{SessionError, SessionState};

/// An accepted sample, recorded after its real History checkpoint. The
/// predecessor identifies the in-process baseline actually used, not an
/// inferred predecessor in the list of audited attempts.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelDeliveryCommitted {
    schema_version: u16,
    audit_event_id: EventId,
    provider_session: ProviderSessionRef,
    full_logical_hash: SemanticHash,
    previous_accepted_audit: Option<EventId>,
    history_cutoff_event_id: EventId,
}

impl ModelDeliveryCommitted {
    pub(crate) fn audit_event_id(&self) -> &EventId {
        &self.audit_event_id
    }
    pub(crate) fn previous_accepted_audit(&self) -> Option<&EventId> {
        self.previous_accepted_audit.as_ref()
    }
}

fn invalid(message: &'static str) -> SessionError {
    SessionError::CompactionContextProvenance {
        message: message.to_owned(),
    }
}

impl SessionState {
    /// Creates the fact only at the accepted-sample durability stage. The
    /// caller supplies the predecessor selected by the existing baseline owner.
    pub(crate) fn new_model_delivery(
        &self,
        audit_id: &EventId,
        previous_accepted_audit: Option<EventId>,
    ) -> Result<ModelDeliveryCommitted, SessionError> {
        let audit = self
            .request_audits
            .last()
            .filter(|audit| audit.audit_event_id() == audit_id)
            .ok_or_else(|| invalid("accepted model delivery has no current audit"))?;
        let delivery = ModelDeliveryCommitted {
            schema_version: 1,
            audit_event_id: audit_id.clone(),
            provider_session: audit
                .provider_session()
                .cloned()
                .ok_or_else(|| invalid("accepted model delivery has no provider session"))?,
            full_logical_hash: audit.full_logical_input().semantic_hash().clone(),
            previous_accepted_audit,
            history_cutoff_event_id: self
                .history_event_ids
                .last()
                .cloned()
                .ok_or_else(|| invalid("accepted model delivery has no durable History"))?,
        };
        validate(self, &delivery)?;
        Ok(delivery)
    }

    /// Returns oldest Full to final accepted sample. Missing legacy facts are
    /// unavailable; malformed durable chains are errors, never guessed.
    pub(crate) fn model_delivery_chain(
        &self,
        final_audit: &EventId,
    ) -> Result<Option<Vec<&ModelDeliveryCommitted>>, SessionError> {
        let Some(mut delivery) = self
            .model_deliveries
            .iter()
            .find(|entry| entry.audit_event_id() == final_audit)
        else {
            return Ok(None);
        };
        let mut chain = Vec::new();
        loop {
            chain.push(delivery);
            let Some(previous) = delivery.previous_accepted_audit() else {
                break;
            };
            delivery = self
                .model_deliveries
                .iter()
                .find(|entry| entry.audit_event_id() == previous)
                .ok_or_else(|| invalid("accepted model delivery predecessor is missing"))?;
            if chain.len() > self.model_deliveries.len() {
                return Err(invalid("accepted model delivery chain contains a cycle"));
            }
        }
        chain.reverse();
        Ok(Some(chain))
    }
}

fn validate(state: &SessionState, delivery: &ModelDeliveryCommitted) -> Result<(), SessionError> {
    let audit = state
        .request_audits
        .last()
        .filter(|audit| audit.audit_event_id() == &delivery.audit_event_id)
        .ok_or_else(|| invalid("accepted model delivery audit is not current"))?;
    let history = &state.history.items()[state.latest_audit_history_len..];
    if delivery.schema_version != 1
        || audit.provider_session() != Some(&delivery.provider_session)
        || state.primary_provider_session.as_ref() != Some(&delivery.provider_session)
        || audit.improvement_scope().is_some()
        || audit.full_logical_input().semantic_hash() != &delivery.full_logical_hash
        || state.rejected_requests.contains(&delivery.audit_event_id)
        || state
            .model_deliveries
            .iter()
            .any(|old| old.audit_event_id == delivery.audit_event_id)
        || state.history_event_ids.last() != Some(&delivery.history_cutoff_event_id)
        || history.is_empty()
        || history.iter().any(|item| match item {
            ContentItem::Text {
                role: MessageRole::Assistant,
                text,
            } => text.is_empty(),
            ContentItem::ToolCall { .. } => false,
            _ => true,
        })
    {
        return Err(invalid(
            "accepted model delivery does not match the audited sample and durable History",
        ));
    }
    let expected_previous = state
        .model_deliveries
        .iter()
        .rev()
        .find(|old| old.provider_session == delivery.provider_session)
        .map(|old| &old.audit_event_id);
    match audit.delivery().mode() {
        RequestAuditDeliveryMode::Full if delivery.previous_accepted_audit.is_none() => Ok(()),
        RequestAuditDeliveryMode::Incremental
            if expected_previous.is_some()
                && delivery.previous_accepted_audit.as_ref() == expected_previous =>
        {
            Ok(())
        }
        _ => Err(invalid(
            "accepted model delivery does not match its actual baseline lineage",
        )),
    }
}

pub(super) fn apply(
    state: &mut SessionState,
    delivery: ModelDeliveryCommitted,
) -> Result<(), SessionError> {
    validate(state, &delivery)?;
    state.model_deliveries.push(delivery);
    Ok(())
}
