//! Minimal streaming consumption with cancellation that wakes pending polls.

use std::future::poll_fn;
use std::task::Poll;

use thiserror::Error;

use crate::domain::model::ModelEvent;
use crate::ports::model::{
    ModelProvider, ModelProviderError, ModelRequest, ModelResponseCompletion, ModelStreamEvent,
    PreparedModelRequest,
};

use super::cancellation::AgentCancellation;

/// Backward-compatible name for the shared Agent cancellation authority.
pub type ModelStreamCancellation = AgentCancellation;

/// The one fixed observer stage for provider-neutral events while a request
/// is still streaming. This is deliberately a typed callback boundary rather
/// than a Hook registry: the owning AgentLoop decides which event kinds, if
/// any, become user-visible output.
pub type ModelStreamingEventStage<'a> =
    dyn FnMut(&ModelEvent) -> Result<(), ModelStreamingError> + 'a;

/// The complete or cancelled sequence observed by the streaming service.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ModelStreamOutcome {
    Completed {
        events: Vec<ModelEvent>,
        completion: ModelResponseCompletion,
    },
    Cancelled {
        events: Vec<ModelEvent>,
    },
}

/// Errors from consuming a Model Port stream.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ModelStreamingError {
    #[error(transparent)]
    Provider(#[from] ModelProviderError),
    #[error("model event stream ended before a completed event")]
    EndedBeforeCompletion,
    #[error("model event stage failed: {message}")]
    EventStage { message: String },
}

/// Application service for one provider-neutral streaming model request.
pub struct ModelStreamingService<'provider, Provider: ?Sized> {
    provider: &'provider Provider,
}

impl<'provider, Provider: ModelProvider + ?Sized> ModelStreamingService<'provider, Provider> {
    /// Creates a service using the supplied Model Port implementation.
    pub fn new(provider: &'provider Provider) -> Self {
        Self { provider }
    }

    /// Consumes one stream, preserving event order or ending pending work on cancellation.
    pub async fn consume(
        &self,
        request: ModelRequest,
        cancellation: ModelStreamCancellation,
    ) -> Result<ModelStreamOutcome, ModelStreamingError> {
        let mut discard = |_event: &ModelEvent| Ok(());
        self.consume_with_event_stage(request, cancellation, &mut discard)
            .await
    }

    /// Consumes one stream while passing each provider-neutral event to the
    /// one caller-supplied fixed stage in wire order. Event observation never
    /// commits History or a Provider Baseline; those remain atomic in the
    /// AgentLoop after a completed response.
    pub async fn consume_with_event_stage(
        &self,
        request: ModelRequest,
        cancellation: ModelStreamCancellation,
        event_stage: &mut ModelStreamingEventStage<'_>,
    ) -> Result<ModelStreamOutcome, ModelStreamingError> {
        if cancellation.is_cancelled() {
            return Ok(ModelStreamOutcome::Cancelled { events: Vec::new() });
        }
        let prepared = self.provider.prepare(request)?;
        self.consume_prepared_with_event_stage(prepared.as_ref(), cancellation, event_stage)
            .await
    }

    /// Consumes one physical attempt of a previously prepared logical request.
    /// Preparation may retain credential and request-encoding state across
    /// bounded App-owned retries.
    pub(crate) async fn consume_prepared_with_event_stage(
        &self,
        prepared: &dyn PreparedModelRequest,
        cancellation: ModelStreamCancellation,
        event_stage: &mut ModelStreamingEventStage<'_>,
    ) -> Result<ModelStreamOutcome, ModelStreamingError> {
        if cancellation.is_cancelled() {
            return Ok(ModelStreamOutcome::Cancelled { events: Vec::new() });
        }

        let mut stream = prepared.stream()?;
        let mut events = Vec::new();

        loop {
            let next = poll_fn(|context| {
                if cancellation.is_cancelled() {
                    return Poll::Ready(NextStreamItem::Cancelled);
                }

                match stream.as_mut().poll_next(context) {
                    Poll::Ready(item) => Poll::Ready(NextStreamItem::Item(item)),
                    Poll::Pending => {
                        cancellation.register(context.waker());

                        if cancellation.is_cancelled() {
                            Poll::Ready(NextStreamItem::Cancelled)
                        } else {
                            Poll::Pending
                        }
                    }
                }
            })
            .await;

            match next {
                NextStreamItem::Cancelled => return Ok(ModelStreamOutcome::Cancelled { events }),
                NextStreamItem::Item(Some(Ok(ModelStreamEvent::Event(event)))) => {
                    event_stage(&event)?;
                    events.push(event);
                }
                NextStreamItem::Item(Some(Ok(ModelStreamEvent::Completed(completion)))) => {
                    return Ok(ModelStreamOutcome::Completed { events, completion });
                }
                NextStreamItem::Item(Some(Err(error))) => return Err(error.into()),
                NextStreamItem::Item(None) => {
                    return Err(ModelStreamingError::EndedBeforeCompletion);
                }
            }
        }
    }
}

enum NextStreamItem {
    Item(Option<Result<ModelStreamEvent, ModelProviderError>>),
    Cancelled,
}
