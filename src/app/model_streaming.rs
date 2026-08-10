//! Minimal streaming consumption with cancellation that wakes pending polls.

use std::future::poll_fn;
use std::task::Poll;

use thiserror::Error;

use crate::domain::model::{ModelEvent, ModelInputIR};
use crate::ports::model::{ModelProvider, ModelProviderError};

use super::cancellation::AgentCancellation;

/// Backward-compatible name for the shared Agent cancellation authority.
pub type ModelStreamCancellation = AgentCancellation;

/// The complete or cancelled sequence observed by the streaming service.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ModelStreamOutcome {
    Completed { events: Vec<ModelEvent> },
    Cancelled { events: Vec<ModelEvent> },
}

/// Errors from consuming a Model Port stream.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ModelStreamingError {
    #[error(transparent)]
    Provider(#[from] ModelProviderError),
    #[error("model event stream ended before a completed event")]
    EndedBeforeCompletion,
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
        input: ModelInputIR,
        cancellation: ModelStreamCancellation,
    ) -> Result<ModelStreamOutcome, ModelStreamingError> {
        if cancellation.is_cancelled() {
            return Ok(ModelStreamOutcome::Cancelled { events: Vec::new() });
        }

        let mut stream = self.provider.stream(input)?;
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
                NextStreamItem::Item(Some(Ok(event))) => {
                    let completed = matches!(event, ModelEvent::Completed);
                    events.push(event);

                    if completed {
                        return Ok(ModelStreamOutcome::Completed { events });
                    }
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
    Item(Option<Result<ModelEvent, ModelProviderError>>),
    Cancelled,
}
