//! Durable Session Compaction application for the single AgentLoop.

use crate::domain::context::BaselineInvalidationReason;
use crate::domain::events::StreamId;
use crate::domain::history::ConversationHistory;
use crate::domain::session::{
    CompactionValidationContext, SessionCompaction, SessionLifecycle, SessionState,
};
use crate::ports::model::ModelProvider;
use crate::ports::tool::ToolExecutor;

use super::{AgentLoop, AgentLoopConfigurationError};

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
