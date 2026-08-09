//! Deterministic, no-network ModelProvider for tests and local development.

use std::collections::VecDeque;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures_core::Stream;

use crate::domain::model::{ModelCapabilities, ModelEvent, ModelInputIR};
use crate::ports::model::{ModelEventStream, ModelProvider, ModelProviderError};

/// An explicit, versioned capability fixture plus deterministic stream events.
#[derive(Clone, Debug)]
pub struct MockModelFixture {
    pub capabilities: ModelCapabilities,
    pub events: Vec<ModelEvent>,
}

/// A ModelProvider whose behavior is entirely supplied by `MockModelFixture`.
#[derive(Clone, Debug)]
pub struct MockModelProvider {
    fixture: MockModelFixture,
}

impl MockModelProvider {
    /// Creates a Mock provider from an explicit capability and event fixture.
    pub fn new(fixture: MockModelFixture) -> Self {
        Self { fixture }
    }
}

impl ModelProvider for MockModelProvider {
    fn capabilities(&self) -> ModelCapabilities {
        self.fixture.capabilities.clone()
    }

    fn stream(&self, input: ModelInputIR) -> Result<ModelEventStream, ModelProviderError> {
        self.fixture.capabilities.validate_input(&input)?;

        Ok(Box::pin(MockEventStream {
            events: self.fixture.events.clone().into_iter().map(Ok).collect(),
        }))
    }
}

struct MockEventStream {
    events: VecDeque<Result<ModelEvent, ModelProviderError>>,
}

impl Stream for MockEventStream {
    type Item = Result<ModelEvent, ModelProviderError>;

    fn poll_next(mut self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Ready(self.events.pop_front())
    }
}
