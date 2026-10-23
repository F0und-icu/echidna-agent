//! Bounded retries of one prepared, uncompleted model request.

use std::time::Duration;

use crate::ports::model::{ModelProvider, PreparedModelRequest};

use super::cancellation::AgentCancellation;
use super::model_streaming::{ModelStreamOutcome, ModelStreamingError, ModelStreamingService};

const RETRY_BACKOFFS: [Duration; 2] = [Duration::from_millis(500), Duration::from_millis(1000)];

#[derive(Debug)]
pub(crate) enum ModelRetryError<AttemptError> {
    Attempt(AttemptError),
    Streaming(ModelStreamingError),
}

/// Executes one prepared request and at most two retries. The caller owns
/// admission and audit through `before_attempt`; this service owns only the
/// retryable transport boundary and cancellable backoff.
pub(crate) async fn prepare_and_consume_with_retry<'provider, Provider, AttemptError>(
    provider: &Provider,
    cancellation: AgentCancellation,
    mut prepare: impl FnMut() -> Result<Box<dyn PreparedModelRequest + 'provider>, ModelStreamingError>,
    mut before_attempt: impl FnMut() -> Result<(), AttemptError>,
    event_stage: &mut super::model_streaming::ModelStreamingEventStage<'_>,
) -> Result<ModelStreamOutcome, ModelRetryError<AttemptError>>
where
    Provider: ModelProvider + ?Sized + 'provider,
{
    let mut prepared: Option<Box<dyn PreparedModelRequest + 'provider>> = None;
    for retry in 0..=RETRY_BACKOFFS.len() {
        if cancellation.is_cancelled() {
            return Ok(ModelStreamOutcome::Cancelled { events: Vec::new() });
        }
        if retry > 0 {
            prepared
                .as_deref()
                .expect("the prepared request exists before a retry")
                .retrying(retry);
            if wait_for_backoff_or_cancellation(RETRY_BACKOFFS[retry - 1], &cancellation).await {
                return Ok(ModelStreamOutcome::Cancelled { events: Vec::new() });
            }
        }
        before_attempt().map_err(ModelRetryError::Attempt)?;
        if prepared.is_none() {
            prepared = Some(prepare().map_err(ModelRetryError::Streaming)?);
        }
        let prepared = prepared
            .as_deref()
            .expect("the prepared request is initialized after its first audit");
        let outcome = ModelStreamingService::new(provider)
            .consume_prepared_with_event_stage(prepared, cancellation.clone(), event_stage)
            .await;
        match outcome {
            Ok(outcome) => return Ok(outcome),
            Err(error) if retry < RETRY_BACKOFFS.len() && retryable(&error) => continue,
            Err(error) => return Err(ModelRetryError::Streaming(error)),
        }
    }
    unreachable!("the bounded retry sequence always returns")
}

fn retryable(error: &ModelStreamingError) -> bool {
    matches!(
        error,
        ModelStreamingError::Provider(
            crate::ports::model::ModelProviderError::ConnectionFailure { .. }
                | crate::ports::model::ModelProviderError::ResponseInterrupted { .. }
        )
    )
}

async fn wait_for_backoff_or_cancellation(
    backoff: Duration,
    cancellation: &AgentCancellation,
) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(backoff) => false,
        _ = cancellation.wait_for_cancellation() => true,
    }
}
