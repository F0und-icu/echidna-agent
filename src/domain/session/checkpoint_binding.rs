//! Instruction application at the just-committed model handoff, without Resume.

use super::*;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CheckpointDefinitionPayload {
    pub(super) schema_version: u16,
    pub(super) primary_provider_session: ProviderSessionRef,
    pub(super) binding: AgentDefinitionBinding,
    pub(super) attempt_id: String,
    pub(super) parent_digest: String,
    pub(super) compaction_revision: u64,
    pub(super) compaction_fingerprint: String,
}

pub(super) fn apply(
    state: &mut SessionState,
    payload: CheckpointDefinitionPayload,
    previous_sequence: Option<EventSequence>,
) -> Result<(), SessionError> {
    state.ensure_no_unknown_improvement()?;
    state.ensure_completed_history()?;
    let invalid = || SessionError::InvalidImprovementActivity;
    let previous = state
        .agent_definition_binding
        .as_ref()
        .ok_or_else(invalid)?;
    let compaction = state.latest_compaction.as_ref().ok_or_else(invalid)?;
    if state.lifecycle != SessionLifecycle::Active
        || previous.definition().digest().as_str() != payload.parent_digest
        || !previous.definition().rsi().is_some_and(|rsi| rsi.improvement)
        || !state.improvement_records.iter().any(|record| record.activity.attempt_id == payload.attempt_id
            && matches!(&record.activity.fact, ImprovementFact::CheckpointStarted {parent_digest, ..} if parent_digest == &payload.parent_digest))
        || !state.improvement_records.iter().any(|record| record.activity.attempt_id == payload.attempt_id
            && matches!(&record.activity.fact, ImprovementFact::CandidateBound { definition_digest }
                if definition_digest == payload.binding.definition().digest().as_str()))
        || compaction.summary().schema_version() != 5
        || compaction.summary().revision() != payload.compaction_revision
        || compaction.fingerprint()? != payload.compaction_fingerprint
        || previous_sequence.map(EventSequence::get) != compaction.source_event_sequence().get().checked_add(1)
    { return Err(invalid()); }
    state.improvement_evidence(&payload.attempt_id, &payload.parent_digest)?;
    let next = payload.binding.definition();
    let evolution = next
        .rsi()
        .and_then(|rsi| rsi.state.as_ref())
        .ok_or_else(invalid)?;
    if evolution.parent_digest != payload.parent_digest
        || !evolution
            .experiences
            .iter()
            .any(|experience| experience.attempt_id == payload.attempt_id)
    {
        return Err(invalid());
    }
    // Reuse the one allowed-scope producer: rebuilding through with_evolution
    // proves fixed/work/context/tools/policy and operator snapshots did not change.
    let expected = previous
        .definition()
        .with_evolution(
            (previous.definition().instructions().persona() != next.instructions().persona())
                .then(|| next.instructions().persona().content().to_owned()),
            (previous.definition().instructions().methods() != next.instructions().methods()).then(
                || {
                    next.instructions()
                        .methods()
                        .iter()
                        .map(|method| method.content().to_owned())
                        .collect()
                },
            ),
            evolution.clone(),
        )
        .and_then(|definition| previous.replacing_definition(definition))
        .map_err(|error| SessionError::AgentDefinition {
            message: error.to_string(),
        })?;
    if expected != payload.binding {
        return Err(invalid());
    }
    require_primary_provider_session(&payload.primary_provider_session)?;
    record_fresh_primary_provider_session_id(state, &payload.primary_provider_session)?;
    state.primary_provider_session = Some(payload.primary_provider_session);
    state.agent_definition_binding = Some(payload.binding);
    state.consecutive_rejection = None;
    state.correction_reset_anchor = None;
    Ok(())
}
