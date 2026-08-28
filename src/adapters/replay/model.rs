//! Fixed model-side semantic samples for Trace Replay.

use std::collections::VecDeque;
use std::sync::Mutex;

use futures_util::stream;

use crate::domain::model::ModelCapabilities;
use crate::domain::trace_eval::TraceReplayModelStep;
use crate::ports::model::{
    ModelEventStream, ModelProvider, ModelProviderError, ModelRequest, ModelRequestDelivery,
    ModelResponseCompletion, ModelStreamEvent,
};

use super::ReplayAdapterError;

fn replay_provider_error(code: &str, message: &str) -> ModelProviderError {
    ModelProviderError::ProviderFailure {
        code: Some(code.to_owned()),
        message: message.to_owned(),
    }
}

/// A `ModelProvider` that accepts only full requests matching the next
/// recorded semantic input and returns only reconstructed local events.
///
/// A recorded source capability mapping may advertise incremental input, but
/// Replay cannot supply a source continuation identity. Its effective Port
/// capability is therefore fixed to full delivery so the AgentLoop never
/// attempts a continuation for a Replay sample.
pub struct ReplayModelProvider {
    capabilities: ModelCapabilities,
    steps: Mutex<VecDeque<TraceReplayModelStep>>,
}

impl ReplayModelProvider {
    pub fn new(
        mut capabilities: ModelCapabilities,
        steps: impl IntoIterator<Item = TraceReplayModelStep>,
    ) -> Result<Self, ReplayAdapterError> {
        capabilities.incremental_input = false;
        let steps = steps.into_iter().collect::<VecDeque<_>>();
        if steps.is_empty() {
            return Err(ReplayAdapterError::ModelStepsRequired);
        }
        Ok(Self {
            capabilities,
            steps: Mutex::new(steps),
        })
    }

    pub fn remaining_steps(&self) -> usize {
        self.steps
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }
}

impl ModelProvider for ReplayModelProvider {
    fn capabilities(&self) -> ModelCapabilities {
        self.capabilities.clone()
    }

    fn stream(&self, request: ModelRequest) -> Result<ModelEventStream, ModelProviderError> {
        if !matches!(request.delivery(), ModelRequestDelivery::Full) {
            return Err(ModelProviderError::ContinuationRejected {
                code: "replay_incremental_unsupported".to_owned(),
            });
        }
        request.validate_capabilities(&self.capabilities)?;
        let mut steps = self
            .steps
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let expected = steps.front().ok_or_else(|| {
            replay_provider_error(
                "replay_trace_exhausted",
                "ReplayModelProvider has no remaining semantic sample",
            )
        })?;
        if expected.expected_input() != request.input() {
            return Err(replay_provider_error(
                "replay_request_mismatch",
                "ReplayModelProvider request differs from the next recorded semantic input",
            ));
        }
        let step = steps
            .pop_front()
            .expect("the checked Replay step is present");
        let events = step
            .into_events()
            .into_iter()
            .map(|event| Ok(ModelStreamEvent::Event(event)))
            .chain(std::iter::once(Ok(ModelStreamEvent::Completed(
                ModelResponseCompletion::without_response_id(),
            ))));
        Ok(Box::pin(stream::iter(events)))
    }
}
