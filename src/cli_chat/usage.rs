//! Successful-sample usage, bound to the exact durable request audit.

use super::*;
use crate::domain::context::ProviderSessionRef;
use crate::domain::model::ModelUsage;

impl DurableChatSession {
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
