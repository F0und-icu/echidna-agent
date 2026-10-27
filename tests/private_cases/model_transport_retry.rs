//! Focused contracts for the one bounded prepared-request retry owner.

#[path = "model_transport_retry_cases/runtime.rs"]
mod chat_runtime_contracts;

mod retry_contracts {
    use std::collections::{BTreeMap, VecDeque};
    use std::convert::Infallible;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use crate::adapters::tools::ToolRegistry;
    use crate::app::agent_loop::{AgentLoop, AgentTurnOutcome};
    use crate::app::cancellation::AgentCancellation;
    use crate::app::model_request_budget::ModelRequestBudget;
    use crate::app::model_retry::{ModelRetryError, prepare_and_consume_with_retry};
    use crate::app::model_streaming::{ModelStreamOutcome, ModelStreamingError};
    use crate::app::request_audit::RequestAuditService;
    use crate::app::session::SessionProjection;
    use crate::app::vision::{
        VisionAnalysisError, VisionAnalysisInput, VisionEventStamp, VisionSecondaryRequest,
        VisionSessionError, VisionSessionService,
    };
    use crate::domain::artifacts::{ArtifactId, BlobId};
    use crate::domain::context::{
        ModelIdentity, ModelProfile, ProviderSessionId, ProviderSessionRef, ProviderSessionRole,
    };
    use crate::domain::events::{EventEnvelope, EventId, EventSequence, OccurredAt, StreamId};
    use crate::domain::model::{
        CapabilityMappingVersion, ContentItem, ModelCapabilities, ModelEvent, ModelInputIR,
    };
    use crate::domain::profile::{VisionMode, VisionRoute, VisionRouteReason};
    use crate::domain::project::ProjectId;
    use crate::domain::request_audit::{
        RequestAdapterVersion, RequestAuditBinding, RequestAuditRunContext,
    };
    use crate::domain::session::SessionEvent;
    use crate::domain::vision::{
        VISION_FACT_SCHEMA_VERSION, VisionImageGeometry, VisionObservationId, VisionRouteRecord,
    };
    use crate::ports::EventSink;
    use crate::ports::artifact::{ArtifactStore, StoredBlob};
    use crate::ports::model::{
        ModelEventStream, ModelProvider, ModelProviderError, ModelRequest, ModelResponseCompletion,
        ModelStreamEvent, PreparedModelRequest, ProviderResponseId,
    };
    use futures_util::stream;
    use sha2::{Digest, Sha256};

    type AttemptEvents = Vec<Result<ModelStreamEvent, ModelProviderError>>;

    fn capabilities() -> ModelCapabilities {
        ModelCapabilities {
            text_input: true,
            image_input: false,
            streaming: true,
            tool_call: true,
            structured_output: false,
            incremental_input: false,
            usage_reporting: false,
            max_output_tokens: None,
            reasoning_effort: None,
            context_window: 32_768,
            capability_mapping_version: CapabilityMappingVersion::new(1),
        }
    }

    fn request() -> ModelRequest {
        ModelRequest::full(ModelInputIR::current(
            "fixture",
            vec![ContentItem::user_text("retry this request")],
            Default::default(),
        ))
    }

    fn vision_capabilities() -> ModelCapabilities {
        let mut capabilities = capabilities();
        capabilities.image_input = true;
        capabilities.structured_output = true;
        capabilities
    }

    fn vision_binding() -> RequestAuditBinding {
        RequestAuditBinding::new(
            ProjectId::new("vision-transport-retry"),
            StreamId::new("session:vision-transport-retry"),
        )
        .unwrap()
    }

    fn vision_audit_context(binding: RequestAuditBinding) -> RequestAuditRunContext {
        RequestAuditRunContext::new(
            binding,
            RequestAdapterVersion::new("fixture-adapter-v1").unwrap(),
        )
    }

    fn occurred_at(sequence: u64) -> OccurredAt {
        OccurredAt::new(format!("2026-10-23T00:00:{sequence:02}Z"))
    }

    fn vision_projection(binding: &RequestAuditBinding) -> SessionProjection {
        SessionProjection::replay([SessionEvent::Started {
            primary_provider_session: ProviderSessionRef {
                id: ProviderSessionId::new("primary:vision-transport-retry"),
                role: ProviderSessionRole::Primary,
                generation: 1,
            },
        }
        .into_envelope(
            EventId::new("session-started"),
            binding.session_stream_id().clone(),
            EventSequence::new(1),
            occurred_at(1),
        )
        .unwrap()])
        .unwrap()
    }

    fn vision_request(vision_capabilities: &ModelCapabilities) -> VisionSecondaryRequest {
        VisionSecondaryRequest {
            route: VisionRouteRecord {
                schema_version: VISION_FACT_SCHEMA_VERSION,
                route_event_id: EventId::new("vision-route"),
                mode: VisionMode::SecondaryTool,
                route: VisionRoute::SecondaryTool,
                reason: Some(VisionRouteReason::ForcedSecondaryTool),
                source_artifact_ids: vec![ArtifactId::new("artifact:original")],
                primary_profile: ModelProfile::new("primary", 1),
                vision_profile: Some(ModelProfile::new("vision", 1)),
                primary_capability_hash: capabilities().semantic_hash(),
                vision_capability_hash: Some(vision_capabilities.semantic_hash()),
            },
            route_occurred_at: occurred_at(2),
            analysis: VisionAnalysisInput {
                observation_id: VisionObservationId::new("observation:vision-transport-retry"),
                route_event_id: EventId::new("vision-route"),
                source_artifact_id: ArtifactId::new("artifact:original"),
                delivery_artifact_id: ArtifactId::new("artifact:delivery"),
                derived_artifact_ids: vec![ArtifactId::new("artifact:delivery")],
                image_geometry: VisionImageGeometry {
                    delivery_artifact_id: ArtifactId::new("artifact:delivery"),
                    source_width: 1200,
                    source_height: 800,
                    delivery_width: 600,
                    delivery_height: 400,
                },
                task: "Inspect this image for a bounded retry fixture.".to_owned(),
                model: ModelIdentity::new("fixture-vision", "vision-v1"),
                provider_profile: ModelProfile::new("vision", 1),
                capability_hash: vision_capabilities.semantic_hash(),
                provider_session: ProviderSessionRef {
                    id: ProviderSessionId::new("vision:transport-retry"),
                    role: ProviderSessionRole::Vision,
                    generation: 1,
                },
            },
        }
    }

    #[derive(Default)]
    struct VisionEventLog {
        events: Vec<EventEnvelope<serde_json::Value>>,
    }

    impl EventSink<serde_json::Value> for VisionEventLog {
        type Error = Infallible;

        fn append_event(
            &mut self,
            event: &EventEnvelope<serde_json::Value>,
        ) -> Result<(), Self::Error> {
            self.events.push(event.clone());
            Ok(())
        }
    }

    #[derive(Default)]
    struct VisionMemoryBlobs {
        bytes: Mutex<BTreeMap<BlobId, Vec<u8>>>,
    }

    impl ArtifactStore for VisionMemoryBlobs {
        type Error = Infallible;

        fn publish_file(&self, _source: &std::path::Path) -> Result<StoredBlob, Self::Error> {
            unreachable!("the Vision retry fixtures publish only request bytes")
        }

        fn publish_bytes(&self, bytes: &[u8]) -> Result<StoredBlob, Self::Error> {
            let blob_id = BlobId::new(format!("{:x}", Sha256::digest(bytes))).unwrap();
            let mut stored = self.bytes.lock().unwrap();
            let already_present = stored.contains_key(&blob_id);
            stored
                .entry(blob_id.clone())
                .or_insert_with(|| bytes.to_vec());
            Ok(StoredBlob {
                blob_id,
                size_bytes: bytes.len().try_into().unwrap(),
                already_present,
            })
        }

        fn read_blob(&self, blob_id: &BlobId) -> Result<Vec<u8>, Self::Error> {
            Ok(self.bytes.lock().unwrap()[blob_id].clone())
        }
    }

    fn completed(text: &str) -> AttemptEvents {
        vec![
            Ok(ModelStreamEvent::Event(ModelEvent::TextDelta {
                text: text.to_owned(),
            })),
            Ok(ModelStreamEvent::Completed(ModelResponseCompletion::new(
                ProviderResponseId::new("retry-success"),
            ))),
        ]
    }

    struct PreparedFixtureProvider {
        capabilities: ModelCapabilities,
        attempts: Arc<Mutex<VecDeque<AttemptEvents>>>,
        prepare_calls: Arc<AtomicUsize>,
        stream_calls: Arc<AtomicUsize>,
        retry_markers: Arc<AtomicUsize>,
        requests: Arc<Mutex<Vec<ModelRequest>>>,
    }

    impl PreparedFixtureProvider {
        fn new(attempts: impl IntoIterator<Item = AttemptEvents>) -> Self {
            Self::with_capabilities(capabilities(), attempts)
        }

        fn with_capabilities(
            capabilities: ModelCapabilities,
            attempts: impl IntoIterator<Item = AttemptEvents>,
        ) -> Self {
            Self {
                attempts: Arc::new(Mutex::new(attempts.into_iter().collect())),
                capabilities,
                prepare_calls: Arc::new(AtomicUsize::new(0)),
                stream_calls: Arc::new(AtomicUsize::new(0)),
                retry_markers: Arc::new(AtomicUsize::new(0)),
                requests: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    impl ModelProvider for PreparedFixtureProvider {
        fn capabilities(&self) -> ModelCapabilities {
            self.capabilities.clone()
        }

        fn stream(&self, _: ModelRequest) -> Result<ModelEventStream, ModelProviderError> {
            panic!("the prepared retry fixture must use ModelProvider::prepare")
        }

        fn prepare(
            &self,
            request: ModelRequest,
        ) -> Result<Box<dyn PreparedModelRequest + '_>, ModelProviderError> {
            self.prepare_calls.fetch_add(1, Ordering::SeqCst);
            self.requests.lock().unwrap().push(request);
            Ok(Box::new(PreparedFixtureRequest {
                attempts: Arc::clone(&self.attempts),
                stream_calls: Arc::clone(&self.stream_calls),
                retry_markers: Arc::clone(&self.retry_markers),
            }))
        }
    }

    struct PreparedFixtureRequest {
        attempts: Arc<Mutex<VecDeque<AttemptEvents>>>,
        stream_calls: Arc<AtomicUsize>,
        retry_markers: Arc<AtomicUsize>,
    }

    impl PreparedModelRequest for PreparedFixtureRequest {
        fn stream(&self) -> Result<ModelEventStream, ModelProviderError> {
            self.stream_calls.fetch_add(1, Ordering::SeqCst);
            let events = self
                .attempts
                .lock()
                .unwrap()
                .pop_front()
                .expect("the fixture has one response per physical attempt");
            Ok(Box::pin(stream::iter(events)))
        }

        fn retrying(&self, _: usize) {
            self.retry_markers.fetch_add(1, Ordering::SeqCst);
        }
    }

    async fn run(
        provider: &PreparedFixtureProvider,
        cancellation: AgentCancellation,
        before_attempts: &mut usize,
    ) -> Result<ModelStreamOutcome, ModelRetryError<()>> {
        let mut discard = |_event: &ModelEvent| Ok(());
        prepare_and_consume_with_retry(
            provider,
            cancellation,
            || {
                provider
                    .prepare(request())
                    .map_err(ModelStreamingError::Provider)
            },
            || {
                *before_attempts += 1;
                Ok(())
            },
            &mut discard,
        )
        .await
    }

    async fn run_with_budget(
        provider: &PreparedFixtureProvider,
        budget: &ModelRequestBudget,
        audits: &mut usize,
    ) -> Result<ModelStreamOutcome, ModelRetryError<usize>> {
        let mut discard = |_event: &ModelEvent| Ok(());
        prepare_and_consume_with_retry(
            provider,
            AgentCancellation::new(),
            || {
                provider
                    .prepare(request())
                    .map_err(ModelStreamingError::Provider)
            },
            || {
                budget.admit(ProviderSessionRole::Primary)?;
                *audits += 1;
                Ok(())
            },
            &mut discard,
        )
        .await
    }

    #[tokio::test]
    async fn prepared_connect_failure_retries_once_without_repreparing() {
        let provider = PreparedFixtureProvider::new([
            vec![Err(ModelProviderError::ConnectionFailure {
                timeout: false,
                io_kind: "Some(ConnectionReset)".to_owned(),
                message: "fixture connection reset".to_owned(),
            })],
            completed("recovered"),
        ]);
        let mut before_attempts = 0;

        let outcome = run(&provider, AgentCancellation::new(), &mut before_attempts)
            .await
            .unwrap();

        assert!(matches!(
            outcome,
            ModelStreamOutcome::Completed { events, .. }
                if events == vec![ModelEvent::TextDelta { text: "recovered".to_owned() }]
        ));
        assert_eq!(before_attempts, 2);
        assert_eq!(provider.prepare_calls.load(Ordering::SeqCst), 1);
        assert_eq!(provider.stream_calls.load(Ordering::SeqCst), 2);
        assert_eq!(provider.retry_markers.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn agent_loop_audits_each_physical_attempt_and_commits_only_the_successful_sample() {
        let provider = PreparedFixtureProvider::new([
            vec![
                Ok(ModelStreamEvent::Event(ModelEvent::TextDelta {
                    text: "discarded before reconnect".to_owned(),
                })),
                Err(ModelProviderError::ConnectionFailure {
                    timeout: false,
                    io_kind: "Some(ConnectionReset)".to_owned(),
                    message: "fixture connection reset".to_owned(),
                }),
            ],
            completed("committed after reconnect"),
        ]);
        let tools = ToolRegistry::for_project(env!("CARGO_MANIFEST_DIR"));
        let mut agent = AgentLoop::new(&provider, &tools);
        let audit_context = RequestAuditRunContext::new(
            RequestAuditBinding::new(
                ProjectId::new("model-transport-retry"),
                StreamId::new("session:model-transport-retry"),
            )
            .unwrap(),
            RequestAdapterVersion::new("fixture-adapter-v1").unwrap(),
        );
        let mut audits = 0;

        let outcome = agent
            .run_turn_audited(
                "retry the same request",
                AgentCancellation::new(),
                &audit_context,
                &mut |_| {
                    audits += 1;
                    Ok(())
                },
            )
            .await
            .unwrap();

        assert!(matches!(
            outcome,
            AgentTurnOutcome::Completed {
                assistant_text,
                model_requests: 2,
                executions,
            } if assistant_text == "committed after reconnect" && executions.is_empty()
        ));
        assert_eq!(audits, 2, "every physical dispatch has its own audit");
        assert_eq!(provider.prepare_calls.load(Ordering::SeqCst), 1);
        assert_eq!(provider.stream_calls.load(Ordering::SeqCst), 2);
        assert_eq!(
            agent.history().items(),
            [
                ContentItem::user_text("retry the same request"),
                ContentItem::assistant_text("committed after reconnect"),
            ],
            "failed temporary events cannot enter History"
        );
    }

    #[tokio::test]
    async fn shared_request_budget_limits_each_physical_retry_before_audit_or_dispatch() {
        let interruption = ModelProviderError::ConnectionFailure {
            timeout: false,
            io_kind: "Some(ConnectionReset)".to_owned(),
            message: "fixture connection reset".to_owned(),
        };
        for limit in 1..=3 {
            let mut responses = vec![vec![Err(interruption.clone())]; limit];
            if limit == 3 {
                responses[2] = completed("recovered within the budget");
            }
            let provider = PreparedFixtureProvider::new(responses);
            let budget = ModelRequestBudget::new(limit);
            let mut audits = 0;

            let result = run_with_budget(&provider, &budget, &mut audits).await;

            match limit {
                1 | 2 => assert!(matches!(
                    result,
                    Err(ModelRetryError::Attempt(error)) if error == limit
                )),
                3 => assert!(matches!(result, Ok(ModelStreamOutcome::Completed { .. }))),
                _ => unreachable!(),
            }
            assert_eq!(budget.counts().primary, limit);
            assert_eq!(audits, limit);
            assert_eq!(provider.stream_calls.load(Ordering::SeqCst), limit);
            assert_eq!(provider.prepare_calls.load(Ordering::SeqCst), 1);
        }
    }

    #[tokio::test]
    async fn unlimited_admission_keeps_the_three_attempt_transport_retry_bound() {
        let interruption = ModelProviderError::ConnectionFailure {
            timeout: false,
            io_kind: "Some(ConnectionReset)".to_owned(),
            message: "fixture connection reset".to_owned(),
        };
        let provider = PreparedFixtureProvider::new(vec![vec![Err(interruption.clone())]; 3]);
        let budget = ModelRequestBudget::new(0);
        let mut audits = 0;
        let error = run_with_budget(&provider, &budget, &mut audits)
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            ModelRetryError::Streaming(ModelStreamingError::Provider(error)) if error == interruption
        ));
        assert_eq!(budget.remaining(), None);
        assert_eq!(budget.counts().primary, 3);
        assert_eq!(audits, 3);
        assert_eq!(provider.prepare_calls.load(Ordering::SeqCst), 1);
        assert_eq!(provider.stream_calls.load(Ordering::SeqCst), 3);
        assert_eq!(provider.retry_markers.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn interrupted_response_exhaustion_keeps_the_typed_terminal_error() {
        let interruption = ModelProviderError::ResponseInterrupted {
            status: 200,
            elapsed_ms: 7,
            timeout: false,
            io_kind: "Some(UnexpectedEof)".to_owned(),
        };
        let provider = PreparedFixtureProvider::new([
            vec![Err(interruption.clone())],
            vec![Err(interruption.clone())],
            vec![Err(interruption.clone())],
        ]);
        let mut before_attempts = 0;

        let error = run(&provider, AgentCancellation::new(), &mut before_attempts)
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            ModelRetryError::Streaming(ModelStreamingError::Provider(error)) if error == interruption
        ));
        assert_eq!(before_attempts, 3);
        assert_eq!(provider.prepare_calls.load(Ordering::SeqCst), 1);
        assert_eq!(provider.stream_calls.load(Ordering::SeqCst), 3);
        assert_eq!(provider.retry_markers.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn generic_transport_and_clean_stream_end_remain_single_attempt_failures() {
        for attempt in [
            vec![Err(ModelProviderError::TransportFailure {
                message: "fixture generic transport failure".to_owned(),
            })],
            vec![Err(ModelProviderError::ProviderFailure {
                code: Some("server_error".to_owned()),
                message: "Responses SSE error: fixture terminal failure".to_owned(),
            })],
            Vec::new(),
        ] {
            let provider = PreparedFixtureProvider::new([attempt]);
            let mut before_attempts = 0;
            assert!(
                run(&provider, AgentCancellation::new(), &mut before_attempts)
                    .await
                    .is_err()
            );
            assert_eq!(before_attempts, 1);
            assert_eq!(provider.prepare_calls.load(Ordering::SeqCst), 1);
            assert_eq!(provider.stream_calls.load(Ordering::SeqCst), 1);
            assert_eq!(provider.retry_markers.load(Ordering::SeqCst), 0);
        }
    }

    #[tokio::test]
    async fn cancellation_during_backoff_does_not_admit_or_dispatch_another_attempt() {
        let provider =
            PreparedFixtureProvider::new([vec![Err(ModelProviderError::ConnectionFailure {
                timeout: true,
                io_kind: "None".to_owned(),
                message: "fixture connection timeout".to_owned(),
            })]]);
        let cancellation = AgentCancellation::new();
        let mut before_attempts = 0;
        let outcome = {
            let future = run(&provider, cancellation.clone(), &mut before_attempts);
            tokio::pin!(future);

            tokio::select! {
                _ = &mut future => panic!("retry backoff finished unexpectedly"),
                _ = tokio::time::sleep(std::time::Duration::from_millis(20)) => cancellation.cancel(),
            }
            future.await.unwrap()
        };
        assert!(matches!(outcome, ModelStreamOutcome::Cancelled { .. }));
        assert_eq!(before_attempts, 1);
        assert_eq!(provider.prepare_calls.load(Ordering::SeqCst), 1);
        assert_eq!(provider.stream_calls.load(Ordering::SeqCst), 1);
        assert_eq!(provider.retry_markers.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn cancellation_before_the_first_attempt_does_not_prepare_or_audit() {
        let provider = PreparedFixtureProvider::new([completed("unused")]);
        let cancellation = AgentCancellation::new();
        cancellation.cancel();
        let mut before_attempts = 0;

        assert!(matches!(
            run(&provider, cancellation, &mut before_attempts).await.unwrap(),
            ModelStreamOutcome::Cancelled { events } if events.is_empty()
        ));
        assert_eq!(before_attempts, 0);
        assert_eq!(provider.prepare_calls.load(Ordering::SeqCst), 0);
        assert_eq!(provider.stream_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn vision_cancellation_before_its_first_attempt_does_not_fabricate_an_audit() {
        let capabilities = vision_capabilities();
        let provider = PreparedFixtureProvider::with_capabilities(
            capabilities.clone(),
            [completed("unused Vision response")],
        );
        let binding = vision_binding();
        let audit_context = vision_audit_context(binding.clone());
        let blobs = VisionMemoryBlobs::default();
        let service = VisionSessionService::new(&blobs, binding.clone());
        let mut projection = vision_projection(&binding);
        let mut sink = VisionEventLog::default();
        let cancellation = AgentCancellation::new();
        cancellation.cancel();

        let error = service
            .analyze_image(
                &provider,
                vision_request(&capabilities),
                &audit_context,
                cancellation,
                &mut projection,
                &mut sink,
                || VisionEventStamp::new(EventId::new("vision-audit-unused"), occurred_at(3)),
                || VisionEventStamp::new(EventId::new("vision-terminal-unused"), occurred_at(4)),
            )
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            VisionSessionError::Analysis(VisionAnalysisError::Cancelled)
        ));
        assert_eq!(provider.prepare_calls.load(Ordering::SeqCst), 0);
        assert_eq!(provider.stream_calls.load(Ordering::SeqCst), 0);
        assert_eq!(projection.state().vision_routes().len(), 1);
        assert!(projection.state().request_audits().is_empty());
        assert!(projection.state().vision_failures().is_empty());
        assert_eq!(
            sink.events
                .iter()
                .map(|event| event.event_type.as_str())
                .collect::<Vec<_>>(),
            ["vision.route_decided"]
        );
    }

    #[tokio::test]
    async fn vision_retry_budget_stop_records_one_failure_against_the_last_audit() {
        let capabilities = vision_capabilities();
        let provider = PreparedFixtureProvider::with_capabilities(
            capabilities.clone(),
            [vec![Err(ModelProviderError::ConnectionFailure {
                timeout: false,
                io_kind: "Some(ConnectionReset)".to_owned(),
                message: "fixture connection reset".to_owned(),
            })]],
        );
        let binding = vision_binding();
        let audit_context = vision_audit_context(binding.clone());
        let blobs = VisionMemoryBlobs::default();
        let service = VisionSessionService::new(&blobs, binding.clone())
            .with_request_budget(Arc::new(ModelRequestBudget::new(1)));
        let mut projection = vision_projection(&binding);
        let mut sink = VisionEventLog::default();
        let mut audit_sequence = 0_u64;

        let error = service
            .analyze_image(
                &provider,
                vision_request(&capabilities),
                &audit_context,
                AgentCancellation::new(),
                &mut projection,
                &mut sink,
                || {
                    audit_sequence += 1;
                    VisionEventStamp::new(
                        EventId::new(format!("vision-audit-{audit_sequence}")),
                        occurred_at(2 + audit_sequence),
                    )
                },
                || VisionEventStamp::new(EventId::new("vision-limit-terminal"), occurred_at(4)),
            )
            .await
            .unwrap_err();

        assert!(matches!(
            &error,
            VisionSessionError::Analysis(VisionAnalysisError::RequestLimit { limit: 1 })
        ));
        assert!(matches!(
            error.into_recoverable_stop(),
            Ok(crate::app::agent_loop::AgentLoopError::ModelRequestLimit { limit: 1 })
        ));
        assert_eq!(provider.prepare_calls.load(Ordering::SeqCst), 1);
        assert_eq!(provider.stream_calls.load(Ordering::SeqCst), 1);
        assert_eq!(provider.retry_markers.load(Ordering::SeqCst), 1);
        assert_eq!(projection.state().request_audits().len(), 1);
        assert_eq!(projection.state().vision_failures().len(), 1);
        let failure = &projection.state().vision_failures()[0];
        assert_eq!(failure.error_code, "request_limit");
        assert_eq!(
            failure.request_audit_event_id,
            EventId::new("vision-audit-1")
        );
        assert_eq!(
            sink.events
                .iter()
                .filter(|event| event.event_type.as_str() == "vision.analysis_failed")
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn vision_retry_audit_commit_failure_stays_fatal_without_a_terminal() {
        let capabilities = vision_capabilities();
        let provider = PreparedFixtureProvider::with_capabilities(
            capabilities.clone(),
            [vec![Err(ModelProviderError::ConnectionFailure {
                timeout: false,
                io_kind: "Some(ConnectionReset)".to_owned(),
                message: "fixture connection reset".to_owned(),
            })]],
        );
        let binding = vision_binding();
        let audit_context = vision_audit_context(binding.clone());
        let blobs = VisionMemoryBlobs::default();
        let service = VisionSessionService::new(&blobs, binding.clone());
        let mut projection = vision_projection(&binding);
        let mut sink = VisionEventLog::default();
        let prepared = service
            .prepare_secondary(vision_request(&capabilities), &mut projection, &mut sink)
            .unwrap();
        let audit_service = RequestAuditService::new(&blobs, binding);
        let mut audit_attempts = 0;

        let error = service
            .analyze_prepared(
                &provider,
                prepared,
                &audit_context,
                AgentCancellation::new(),
                |attempt| {
                    audit_attempts += 1;
                    if audit_attempts == 2 {
                        return Err("second Vision audit write failed".to_owned());
                    }
                    audit_service
                        .record(
                            attempt,
                            EventId::new("vision-audit-first"),
                            occurred_at(3),
                            &mut projection,
                            &mut sink,
                        )
                        .map_err(|error| format!("{error:?}"))
                },
            )
            .await;
        let error = match error {
            Ok(_) => panic!("a failed retry audit must not produce a Vision terminal"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            VisionSessionError::Analysis(VisionAnalysisError::AuditCommit { message })
                if message.contains("second Vision audit write failed")
        ));
        assert_eq!(provider.prepare_calls.load(Ordering::SeqCst), 1);
        assert_eq!(provider.stream_calls.load(Ordering::SeqCst), 1);
        assert_eq!(provider.retry_markers.load(Ordering::SeqCst), 1);
        assert_eq!(projection.state().request_audits().len(), 1);
        assert!(projection.state().vision_failures().is_empty());
        assert!(
            sink.events
                .iter()
                .all(|event| event.event_type.as_str() != "vision.analysis_failed")
        );
    }

    #[tokio::test]
    async fn rsi_retry_preserves_single_sample_scope_final_audit_and_executes_one_command() {
        use crate::adapters::artifact_store::LocalArtifactStore;
        use crate::adapters::project_event_store::Project;
        use crate::app::agent_loop::{ChatDurabilityCheckpoint, ImprovementSampleInput};
        use crate::app::model_request_budget::ModelRequestBudget;
        use crate::domain::events::EventId;
        use crate::domain::research_control::PromptComposition;
        use crate::domain::session::{ImprovementStage, scoped_call_id};
        use crate::domain::tools::{ToolCall, ToolCallId, ToolName};
        use crate::ports::model::ContinuationRetention;

        let root = crate::support::TemporaryDirectory::new("rsi-prepared-retry");
        let project = Project::initialize(root.path()).unwrap();
        let script = root.path().join("once.sh");
        std::fs::write(&script, "printf x >> \"$0.count\"\n").unwrap();
        let call = ToolCall::new(
            ToolCallId::new("same-provider-call"),
            ToolName::new("run_command"),
            serde_json::json!({"argv":["/bin/sh",script]}),
        );
        let provider = PreparedFixtureProvider::new([
            completed("ordinary task completed"),
            vec![
                Ok(ModelStreamEvent::Event(ModelEvent::ToolCall {
                    call: call.clone(),
                })),
                Err(ModelProviderError::ResponseInterrupted {
                    status: 200,
                    elapsed_ms: 3,
                    timeout: false,
                    io_kind: "None".to_owned(),
                }),
            ],
            vec![
                Ok(ModelStreamEvent::Event(ModelEvent::ToolCall {
                    call: call.clone(),
                })),
                Ok(ModelStreamEvent::Completed(
                    ModelResponseCompletion::without_response_id(),
                )),
            ],
        ]);
        let tools = ToolRegistry::for_project(root.path());
        let mut agent = AgentLoop::new(&provider, &tools)
            .with_output_store(LocalArtifactStore::for_project(&project));
        let audit = RequestAuditRunContext::new(
            RequestAuditBinding::new(
                project.project_id().clone(),
                StreamId::new("session:rsi-retry"),
            )
            .unwrap(),
            RequestAdapterVersion::new("fixture-retry-v1").unwrap(),
        );
        agent
            .run_turn_audited(
                "finish ordinary work",
                AgentCancellation::new(),
                &audit,
                &mut |_| Ok(()),
            )
            .await
            .unwrap();
        let parent = agent.history().items().to_vec();
        let budget = ModelRequestBudget::new(2);
        let prompt = PromptComposition::current();
        let mut audit_ids = Vec::new();
        let mut samples = Vec::new();
        let mut receipts = 0;
        let result = agent
            .sample_improvement(
                ImprovementSampleInput {
                    boundary: crate::app::agent_loop::ImprovementBoundary::CompletedTurn,
                    attempt_id: "retry-attempt",
                    stage: ImprovementStage::Old,
                    prompt: &prompt,
                    task: "Execute the checked script once".to_owned(),
                    references: Vec::new(),
                    tools: &tools,
                    request_budget: &budget,
                    cancellation: AgentCancellation::new(),
                    audit_context: &audit,
                },
                &mut |_| {
                    let id = EventId::new(format!("rsi-audit-{}", audit_ids.len() + 1));
                    audit_ids.push(id.clone());
                    Ok(id)
                },
                &mut |audit, scope, items, original_ids, _usage| {
                    samples.push((audit, scope, items, original_ids));
                    Ok(())
                },
                &mut |checkpoint| {
                    if matches!(checkpoint, ChatDurabilityCheckpoint::ExecutionReceipt(_)) {
                        receipts += 1;
                    }
                    Ok(())
                },
            )
            .await
            .unwrap();
        assert_eq!(budget.counts().total(), 2);
        assert_eq!(audit_ids.len(), 2);
        assert_eq!(result.audit_event_id, audit_ids[1]);
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].0, audit_ids[1]);
        assert_eq!(samples[0].3, vec![call.call_id.clone()]);
        let scoped = scoped_call_id("retry-attempt", ImprovementStage::Old, &call.call_id);
        assert!(matches!(&samples[0].2[0],ContentItem::ToolCall {call} if call.call_id==scoped));
        assert_eq!(result.executions.len(), 1);
        assert_eq!(receipts, 1);
        assert_eq!(
            std::fs::read(script.with_extension("sh.count")).unwrap(),
            b"x"
        );
        assert_eq!(agent.history().items(), parent);
        assert_eq!(
            provider.prepare_calls.load(Ordering::SeqCst),
            2,
            "one parent and one RSI prepare"
        );
        assert_eq!(provider.stream_calls.load(Ordering::SeqCst), 3);
        let requests = provider.requests.lock().unwrap();
        assert_eq!(
            requests[1].continuation_retention(),
            ContinuationRetention::SingleSample
        );
        assert!(requests[1].opaque_continuation_scope().is_some());
        assert!(!requests[1].is_incremental());
    }
}
