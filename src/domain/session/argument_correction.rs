//! Versioned consecutive rejection chains, anchored to actual accepted History.

use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ConsecutiveRejection {
    audit: EventId,
    provider: ProviderSessionRef,
    used: u8,
    history_len: usize,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ConsecutiveRejectionPayload {
    pub(super) schema_version: u16,
    pub(super) request_audit_event_id: EventId,
    pub(super) diagnostic: ToolArgumentDiagnostic,
    pub(super) corrections_used: u8,
    pub(super) reset_anchor: Option<EventId>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CorrectionResetPayload {
    pub(super) schema_version: u16,
    pub(super) accepted_request_audit_event_id: EventId,
    pub(super) prior_rejection_audit_event_id: EventId,
    pub(super) accepted_history_event_refs: Vec<EventId>,
}

impl SessionState {
    pub(crate) fn correction_reset_anchor(&self) -> Option<&EventId> {
        self.correction_reset_anchor.as_ref()
    }

    pub(crate) fn correction_reset_source(&self) -> Result<(EventId, Vec<EventId>), SessionError> {
        let chain = self
            .consecutive_rejection
            .as_ref()
            .ok_or(SessionError::InvalidModelSampleRejection)?;
        Ok((
            chain.audit.clone(),
            self.history_event_ids[self.latest_audit_history_len..].to_vec(),
        ))
    }
}

pub(super) fn reject(
    state: &mut SessionState,
    audit_id: EventId,
    used: u8,
    anchor: Option<EventId>,
) -> Result<(), SessionError> {
    state.ensure_completed_history()?;
    let invalid = || SessionError::InvalidModelSampleRejection;
    let audit = state.request_audits.last().ok_or_else(invalid)?;
    let provider = state
        .primary_provider_session
        .as_ref()
        .ok_or_else(invalid)?;
    let expected = state
        .consecutive_rejection
        .as_ref()
        .map_or(Some(0), |chain| {
            (chain.provider == *provider)
                .then(|| chain.used.checked_add(1))
                .flatten()
        });
    if state.lifecycle != SessionLifecycle::Active
        || used > super::super::model::MAX_TOOL_ARGUMENT_CORRECTIONS_PER_CHAIN
        || expected != Some(used)
        || anchor != state.correction_reset_anchor
        || audit.audit_event_id() != &audit_id
        || audit.provider_session() != Some(provider)
        || state.history.items().len() != state.latest_audit_history_len
        || state.rejected_requests.contains(&audit_id)
    {
        return Err(invalid());
    }
    state.rejected_requests.insert(audit_id.clone());
    state.consecutive_rejection = Some(ConsecutiveRejection {
        audit: audit_id,
        provider: provider.clone(),
        used,
        history_len: state.history.items().len(),
    });
    Ok(())
}

pub(super) fn reset(
    state: &mut SessionState,
    event_id: &EventId,
    accepted: EventId,
    rejected: EventId,
    history_refs: Vec<EventId>,
) -> Result<(), SessionError> {
    let invalid = || SessionError::InvalidModelSampleRejection;
    let chain = state.consecutive_rejection.as_ref().ok_or_else(invalid)?;
    let audit = state.request_audits.last().ok_or_else(invalid)?;
    let suffix = &state.history.items()[state.latest_audit_history_len..];
    if state.lifecycle != SessionLifecycle::Active
        || chain.audit != rejected
        || state.latest_audit_history_len < chain.history_len
        || accepted == rejected
        || audit.audit_event_id() != &accepted
        || audit.provider_session() != Some(&chain.provider)
        || state.primary_provider_session.as_ref() != Some(&chain.provider)
        || state.rejected_requests.contains(&accepted)
        || suffix.is_empty()
        || suffix.iter().any(|item| {
            !matches!(
                item,
                ContentItem::Text {
                    role: super::super::model::MessageRole::Assistant,
                    ..
                } | ContentItem::ToolCall { .. }
            )
        })
        || history_refs != state.history_event_ids[state.latest_audit_history_len..]
    {
        return Err(invalid());
    }
    state.consecutive_rejection = None;
    state.correction_reset_anchor = Some(event_id.clone());
    Ok(())
}
