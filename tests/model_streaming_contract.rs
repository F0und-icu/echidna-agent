use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll, Wake, Waker};

use futures_core::Stream;
use serde_json::json;

use cyber_agent::adapters::mock_model::{MockModelFixture, MockModelProvider};
use cyber_agent::app::model_streaming::{
    ModelStreamCancellation, ModelStreamOutcome, ModelStreamingError, ModelStreamingService,
};
use cyber_agent::domain::artifacts::ArtifactId;
use cyber_agent::domain::model::{
    CapabilityMappingVersion, ContentItem, MessageRole, ModelCapabilities, ModelCapabilityError,
    ModelEvent, ModelInputIR, ModelRequestRequirements, ModelUsage,
};
use cyber_agent::domain::tools::{ToolCall, ToolCallId, ToolName};
use cyber_agent::ports::model::{ModelEventStream, ModelProvider, ModelProviderError};

fn all_capabilities() -> ModelCapabilities {
    ModelCapabilities {
        text_input: true,
        image_input: true,
        streaming: true,
        tool_call: true,
        structured_output: true,
        incremental_input: true,
        usage_reporting: true,
        context_window: 128_000,
        capability_mapping_version: CapabilityMappingVersion::new(7),
    }
}

fn text_input(requirements: ModelRequestRequirements) -> ModelInputIR {
    ModelInputIR::current(
        "test instructions",
        vec![ContentItem::user_text("inspect this target")],
        requirements,
    )
}

fn fixture(capabilities: ModelCapabilities, events: Vec<ModelEvent>) -> MockModelFixture {
    MockModelFixture {
        capabilities,
        events,
    }
}

fn poll_to_completion<Output>(future: impl Future<Output = Output>) -> Output {
    let mut future = Box::pin(future);
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);

    match future.as_mut().poll(&mut context) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("fixture stream must resolve without an executor"),
    }
}

#[test]
fn mock_fixture_preserves_text_tool_usage_and_completion_order() {
    let expected_events = vec![
        ModelEvent::TextDelta {
            text: "first finding".into(),
        },
        ModelEvent::ToolCall {
            call: ToolCall::new(
                ToolCallId::new("tool-call-1"),
                ToolName::new("inspect_manifest"),
                json!({
                    "path": "AndroidManifest.xml",
                    "include_permissions": true,
                }),
            ),
        },
        ModelEvent::Usage {
            usage: ModelUsage {
                input_tokens: 21,
                output_tokens: 34,
            },
        },
        ModelEvent::Completed,
    ];
    let provider = MockModelProvider::new(fixture(all_capabilities(), expected_events.clone()));
    let service = ModelStreamingService::new(&provider);

    let outcome = poll_to_completion(service.consume(
        text_input(ModelRequestRequirements {
            tool_call: true,
            usage_reporting: true,
            ..ModelRequestRequirements::default()
        }),
        ModelStreamCancellation::new(),
    ));

    assert_eq!(
        outcome,
        Ok(ModelStreamOutcome::Completed {
            events: expected_events
        })
    );
}

#[test]
fn streaming_service_accepts_a_dynamic_model_provider_port() {
    let provider = MockModelProvider::new(fixture(all_capabilities(), vec![ModelEvent::Completed]));
    let provider: &dyn ModelProvider = &provider;
    let service = ModelStreamingService::new(provider);

    assert_eq!(
        poll_to_completion(service.consume(
            text_input(ModelRequestRequirements::default()),
            ModelStreamCancellation::new(),
        )),
        Ok(ModelStreamOutcome::Completed {
            events: vec![ModelEvent::Completed],
        })
    );
}

#[test]
fn mock_provider_uses_only_the_explicit_versioned_capability_fixture() {
    let mut text_only = all_capabilities();
    text_only.image_input = false;
    text_only.capability_mapping_version = CapabilityMappingVersion::new(8);
    let provider = MockModelProvider::new(fixture(text_only.clone(), vec![ModelEvent::Completed]));
    assert_eq!(provider.capabilities(), text_only);
    let image_input = ModelInputIR::current(
        "test instructions",
        vec![ContentItem::ImageArtifact {
            role: MessageRole::User,
            artifact_id: ArtifactId::new("artifact-1"),
        }],
        ModelRequestRequirements::default(),
    );

    match provider.stream(image_input) {
        Err(error) => assert_eq!(
            error,
            ModelProviderError::Capability(ModelCapabilityError::ImageInputUnsupported {
                artifact_id: ArtifactId::new("artifact-1"),
            })
        ),
        Ok(_) => panic!("image input must be rejected by the explicit fixture"),
    }
}

#[derive(Clone, Debug)]
struct PendingProvider {
    capabilities: ModelCapabilities,
}

impl ModelProvider for PendingProvider {
    fn capabilities(&self) -> ModelCapabilities {
        self.capabilities.clone()
    }

    fn stream(&self, input: ModelInputIR) -> Result<ModelEventStream, ModelProviderError> {
        self.capabilities.validate_input(&input)?;
        Ok(Box::pin(PendingStream))
    }
}

struct PendingStream;

impl Stream for PendingStream {
    type Item = Result<ModelEvent, ModelProviderError>;

    fn poll_next(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Pending
    }
}

#[derive(Default)]
struct WakeCounter {
    count: AtomicUsize,
}

impl Wake for WakeCounter {
    fn wake(self: Arc<Self>) {
        self.count.fetch_add(1, Ordering::SeqCst);
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.count.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn cancellation_ends_a_pending_stream_poll_without_waiting_for_an_event() {
    let provider = PendingProvider {
        capabilities: all_capabilities(),
    };
    let service = ModelStreamingService::new(&provider);
    let cancellation = ModelStreamCancellation::new();
    let mut future = Box::pin(service.consume(
        text_input(ModelRequestRequirements::default()),
        cancellation.clone(),
    ));
    let wake_counter = Arc::new(WakeCounter::default());
    let waker = Waker::from(wake_counter.clone());
    let mut context = Context::from_waker(&waker);

    assert!(matches!(future.as_mut().poll(&mut context), Poll::Pending));
    cancellation.cancel();
    assert_eq!(wake_counter.count.load(Ordering::SeqCst), 1);

    assert_eq!(
        future.as_mut().poll(&mut context),
        Poll::Ready(Ok(ModelStreamOutcome::Cancelled { events: vec![] }))
    );
}

#[test]
fn service_rejects_a_stream_that_ends_without_completed() {
    let provider = MockModelProvider::new(fixture(
        all_capabilities(),
        vec![ModelEvent::TextDelta {
            text: "partial response".into(),
        }],
    ));
    let service = ModelStreamingService::new(&provider);

    assert_eq!(
        poll_to_completion(service.consume(
            text_input(ModelRequestRequirements::default()),
            ModelStreamCancellation::new(),
        )),
        Err(ModelStreamingError::EndedBeforeCompletion)
    );
}

#[test]
fn service_surfaces_typed_capability_errors_from_the_port() {
    let mut no_usage = all_capabilities();
    no_usage.usage_reporting = false;
    let provider = MockModelProvider::new(fixture(no_usage, vec![ModelEvent::Completed]));
    let service = ModelStreamingService::new(&provider);

    let result = poll_to_completion(service.consume(
        text_input(ModelRequestRequirements {
            usage_reporting: true,
            ..ModelRequestRequirements::default()
        }),
        ModelStreamCancellation::new(),
    ));

    assert_eq!(
        result,
        Err(ModelStreamingError::Provider(
            ModelProviderError::Capability(ModelCapabilityError::UsageReportingUnsupported)
        ))
    );
}
