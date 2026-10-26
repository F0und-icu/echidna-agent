//! The App's fixed one-sample service and retention sidecar stay crate-private.

mod continuation_regression {
    use std::sync::Mutex;

    use futures_util::StreamExt;
    use serde_json::{Value, json};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::mpsc;

    use crate::adapters::artifact_store::LocalArtifactStore;
    use crate::adapters::credentials::{
        AccountBoundCredential, CredentialResolution, CredentialSource, CredentialSourceError,
    };
    use crate::adapters::responses::{ResponsesHttpAdapter, ResponsesHttpConfig};
    use crate::adapters::tools::ToolRegistry;
    use crate::app::agent_loop::{
        AgentLoop, AgentLoopError, ChatDurabilityCheckpoint, ImprovementSampleInput,
    };
    use crate::app::cancellation::AgentCancellation;
    use crate::app::model_request_budget::ModelRequestBudget;
    use crate::domain::context::{ProviderSession, ProviderSessionId, ProviderSessionRole};
    use crate::domain::events::{EventId, StreamId};
    use crate::domain::model::{
        CapabilityMappingVersion, ContentItem, ModelCapabilities, ModelEvent, ModelInputIR,
        ModelRequestRequirements, OpaqueContinuationReference, OpaqueContinuationScope,
    };
    use crate::domain::project::ProjectId;
    use crate::domain::request_audit::{RequestAuditBinding, RequestAuditRunContext};
    use crate::domain::research_control::PromptComposition;
    use crate::domain::session::ImprovementStage;
    use crate::ports::model::{
        ContinuationRetention, ModelEventStream, ModelProvider, ModelProviderError, ModelRequest,
        ModelStreamEvent, PreparedModelRequest, ProviderResponseId,
    };
    use crate::*;

    struct FixtureCredential;

    impl CredentialSource for FixtureCredential {
        fn read(&self, _: &str) -> Result<Option<String>, CredentialSourceError> {
            panic!("subscription fixture uses only its explicit account-bound source")
        }

        fn resolve(&self, reference: &str) -> CredentialResolution {
            assert_eq!(reference, "codex_subscription:rsi-isolation");
            CredentialResolution::PendingAccountBound(Box::pin(async {
                Ok(AccountBoundCredential::new(
                    "fixture-bearer".to_owned(),
                    "fixture-account".to_owned(),
                ))
            }))
        }
    }

    // Observe the real Adapter events; cancellation happens only after it has
    // decoded and staged an actual scoped opaque item, without a timing sleep.
    struct ObservedProvider {
        adapter: ResponsesHttpAdapter,
        scoped_references: Arc<Mutex<Vec<OpaqueContinuationReference>>>,
        cancel: Option<AgentCancellation>,
    }

    impl ModelProvider for ObservedProvider {
        fn capabilities(&self) -> ModelCapabilities {
            self.adapter.capabilities()
        }

        fn stream(&self, request: ModelRequest) -> Result<ModelEventStream, ModelProviderError> {
            self.prepare(request)?.stream()
        }

        fn prepare(
            &self,
            request: ModelRequest,
        ) -> Result<Box<dyn PreparedModelRequest + '_>, ModelProviderError> {
            let single = request.continuation_retention() == ContinuationRetention::SingleSample;
            Ok(Box::new(ObservedPrepared {
                inner: self.adapter.prepare(request)?,
                single,
                scoped_references: self.scoped_references.clone(),
                cancel: self.cancel.clone(),
            }))
        }
    }

    struct ObservedPrepared<'provider> {
        inner: Box<dyn PreparedModelRequest + 'provider>,
        single: bool,
        scoped_references: Arc<Mutex<Vec<OpaqueContinuationReference>>>,
        cancel: Option<AgentCancellation>,
    }

    impl PreparedModelRequest for ObservedPrepared<'_> {
        fn stream(&self) -> Result<ModelEventStream, ModelProviderError> {
            let single = self.single;
            let references = self.scoped_references.clone();
            let cancel = self.cancel.clone();
            Ok(Box::pin(
                self.inner.stream()?.inspect(move |event| {
                    if single
                        && let Ok(ModelStreamEvent::Event(ModelEvent::OpaqueContinuation {
                            reference,
                        })) = event
                    {
                        references.lock().unwrap().push(reference.clone());
                        if let Some(cancel) = &cancel {
                            cancel.cancel();
                        }
                    }
                }),
            ))
        }

        fn retrying(&self, retry: usize) {
            self.inner.retrying(retry);
        }
    }

    fn capabilities() -> ModelCapabilities {
        ModelCapabilities {
            text_input: true,
            image_input: false,
            streaming: true,
            tool_call: true,
            structured_output: true,
            incremental_input: false,
            usage_reporting: false,
            max_output_tokens: None,
            reasoning_effort: None,
            context_window: 128_000,
            capability_mapping_version: CapabilityMappingVersion::new(1),
        }
    }

    fn encrypted(id: &str) -> Value {
        json!({"type":"reasoning","id":id,
            "summary":[],"encrypted_content":format!("ciphertext-{id}")})
    }

    fn command(id: &str) -> Value {
        json!({"type":"function_call","call_id":id,"name":"run_command",
            "arguments":json!({"argv":["/usr/bin/printf","actual-rsi-command"]}).to_string()})
    }

    fn completed(id: &str, mut items: Vec<Value>, text: Option<&str>) -> String {
        if let Some(text) = text {
            items.push(json!({"type":"message","id":format!("message-{id}"),
                "role":"assistant","status":"completed","content":[{
                    "type":"output_text","text":text}]}));
        }
        let mut events = items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                json!({"type":"response.output_item.done",
                "output_index":index,"item":item})
            })
            .collect::<Vec<_>>();
        events.push(json!({"type":"response.completed","response":{
        "id":id,"status":"completed","output":items,"usage":{
            "input_tokens":100,"output_tokens":50,
            "input_tokens_details":{"cached_tokens":75},
            "output_tokens_details":{"reasoning_tokens":20}
        }}}));
        events
            .into_iter()
            .map(|event| format!("data: {event}\n\n"))
            .collect()
    }

    async fn read_request(socket: &mut TcpStream) -> Value {
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 4096];
        loop {
            let read = socket.read(&mut buffer).await.unwrap();
            assert_ne!(read, 0);
            bytes.extend_from_slice(&buffer[..read]);
            assert!(bytes.len() < 1024 * 1024);
            if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                let end = end + 4;
                let headers = std::str::from_utf8(&bytes[..end]).unwrap();
                let length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap();
                if bytes.len() >= end + length {
                    return serde_json::from_slice(&bytes[end..end + length]).unwrap();
                }
            }
        }
    }

    async fn server(
        bodies: Vec<String>,
    ) -> (String, mpsc::Receiver<Value>, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (sender, receiver) = mpsc::channel(bodies.len());
        let task = tokio::spawn(async move {
            for body in bodies {
                let (mut socket, _) = listener.accept().await.unwrap();
                sender.send(read_request(&mut socket).await).await.unwrap();
                let headers = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                socket.write_all(headers.as_bytes()).await.unwrap();
                let _ = socket.write_all(body.as_bytes()).await;
                let _ = socket.shutdown().await;
            }
        });
        (format!("http://{address}/v1"), receiver, task)
    }

    #[derive(Clone, Copy, Debug)]
    enum Case {
        NoChange,
        Commands,
        Failure,
        Cancelled,
    }

    async fn isolation(case: Case) {
        let cancellation = AgentCancellation::new();
        let scope_body = match case {
            Case::NoChange => completed(
                "diagnosis",
                vec![encrypted("diagnosis")],
                Some(r#"{"decision":"no_change","reason":"no supported gain"}"#),
            ),
            Case::Commands => completed(
                "old",
                vec![encrypted("old"), command("same-provider-call")],
                None,
            ),
            Case::Failure => format!(
                "data: {}\n\ndata: {}\n\n",
                json!({"type":"response.output_item.done","output_index":0,"item":encrypted("failed")}),
                json!({"type":"response.failed","response":{"error":{"code":"fixture_failure","message":"fixture"}}})
            ),
            Case::Cancelled => completed("cancelled", vec![encrypted("cancelled")], None),
        };
        let mut bodies = vec![
            completed(
                "parent-call",
                vec![encrypted("parent"), command("main-call")],
                None,
            ),
            completed("parent-final", Vec::new(), Some("main task complete")),
            scope_body,
        ];
        if matches!(case, Case::Commands) {
            bodies.push(completed(
                "new",
                vec![encrypted("new"), command("same-provider-call")],
                None,
            ));
        }
        bodies.push(completed("next-turn", Vec::new(), Some("parent continued")));
        let (url, mut requests, server) = server(bodies).await;
        let provider = ObservedProvider {
            adapter: ResponsesHttpAdapter::new(
                ResponsesHttpConfig::new(
                    url,
                    "fixture-subscription",
                    "codex_subscription:rsi-isolation",
                    capabilities(),
                )
                .with_codex_subscription_profile(),
                Arc::new(FixtureCredential),
            ),
            scoped_references: Arc::new(Mutex::new(Vec::new())),
            cancel: matches!(case, Case::Cancelled).then(|| cancellation.clone()),
        };
        let root = std::env::temp_dir().join(format!(
            "echidna-rsi-opaque-{}-{case:?}",
            std::process::id()
        ));
        std::fs::create_dir(&root).unwrap();
        let project = Project::initialize(&root).unwrap();
        let tools = ToolRegistry::for_project(&root);
        let mut agent = AgentLoop::new(&provider, &tools)
            .with_output_store(LocalArtifactStore::for_project(&project));
        let audit_context = RequestAuditRunContext::new(
            RequestAuditBinding::new(
                ProjectId::new("project-isolation"),
                StreamId::new("session:isolation"),
            )
            .unwrap(),
            ResponsesHttpAdapter::request_adapter_version(),
        );
        let main = agent
            .run_turn_audited(
                "execute command",
                AgentCancellation::new(),
                &audit_context,
                &mut |_| Ok(()),
            )
            .await
            .unwrap();
        assert!(
            matches!(main, crate::app::agent_loop::AgentTurnOutcome::Completed {
            model_requests:2, executions, .. } if executions.len()==1)
        );
        let parent_history = agent.history().items().to_vec();
        assert!(parent_history.iter().all(|item| !matches!(
            item,
            ContentItem::Reasoning { .. } | ContentItem::OpaqueContinuation { .. }
        )));
        let prompt = PromptComposition::current();
        let budget = ModelRequestBudget::new(3);
        let mut observed_samples = Vec::new();
        let mut receipts = 0;
        let stages = if matches!(case, Case::Commands) {
            vec![ImprovementStage::Old, ImprovementStage::New]
        } else {
            vec![ImprovementStage::Diagnosis]
        };
        for stage in &stages {
            let result = agent
                .sample_improvement(
                    ImprovementSampleInput {
                        attempt_id: "opaque-attempt",
                        stage: *stage,
                        prompt: &prompt,
                        task: format!("RSI2 isolated {stage:?}"),
                        references: Vec::new(),
                        tools: &tools,
                        request_budget: &budget,
                        cancellation: cancellation.clone(),
                        audit_context: &audit_context,
                    },
                    &mut |_| Ok(EventId::new(format!("audit-{stage:?}"))),
                    &mut |_, _, items, _, usage| {
                        assert_eq!(
                            usage,
                            Some(crate::domain::model::ModelUsage {
                                input_tokens: 100,
                                output_tokens: 50,
                                cached_input_tokens: Some(75),
                                reasoning_output_tokens: Some(20),
                            })
                        );
                        observed_samples.extend(items);
                        Ok(())
                    },
                    &mut |checkpoint| {
                        if matches!(checkpoint, ChatDurabilityCheckpoint::ExecutionReceipt(_)) {
                            receipts += 1;
                        }
                        Ok(())
                    },
                )
                .await;
            match case {
                Case::Failure => assert!(matches!(result, Err(AgentLoopError::Model(_)))),
                Case::Cancelled => {
                    assert!(matches!(result, Err(AgentLoopError::ImprovementCancelled)))
                }
                Case::Commands => {
                    let executions = result.unwrap().executions;
                    assert_eq!(executions.len(), 1);
                    assert_eq!(executions[0].result.output["exit_code"], 0);
                    assert_eq!(executions[0].result.output["stdout"], "actual-rsi-command");
                }
                Case::NoChange => assert_eq!(
                    serde_json::from_str::<Value>(&result.unwrap().text).unwrap()["decision"],
                    "no_change"
                ),
            }
            assert_eq!(agent.history().items(), parent_history);
        }
        assert_eq!(budget.counts().total(), stages.len());
        assert_eq!(receipts, if matches!(case, Case::Commands) { 2 } else { 0 });
        assert!(observed_samples.iter().all(|item| !matches!(
            item,
            ContentItem::Reasoning { .. } | ContentItem::OpaqueContinuation { .. }
        )));
        agent
            .run_turn_audited(
                "continue parent",
                AgentCancellation::new(),
                &audit_context,
                &mut |_| Ok(()),
            )
            .await
            .unwrap();
        let mut wires = Vec::new();
        while let Some(request) = requests.recv().await {
            wires.push(request);
        }
        server.await.unwrap();
        assert_eq!(wires.len(), 3 + stages.len());
        let reasoning = |wire: &Value| {
            wire["input"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|item| item["type"] == "reasoning")
                .cloned()
                .collect::<Vec<_>>()
        };
        assert!(reasoning(&wires[0]).is_empty());
        assert_eq!(reasoning(&wires[1]), vec![encrypted("parent")]);
        for wire in &wires[2..wires.len() - 1] {
            assert!(
                reasoning(wire).is_empty(),
                "scope must not inherit parent bytes"
            );
            assert!(wire.get("previous_response_id").is_none());
        }
        assert_eq!(
            reasoning(wires.last().unwrap()),
            vec![encrypted("parent")],
            "successful, failed and cancelled scopes must preserve parent encrypted replay"
        );
        let references = provider.scoped_references.lock().unwrap().clone();
        assert_eq!(references.len(), stages.len());
        for reference in references {
            let scope = reference.scope().clone();
            let request = ModelRequest::full(ModelInputIR::current(
                "unused",
                vec![ContentItem::OpaqueContinuation { reference }],
                ModelRequestRequirements::default(),
            ))
            .with_opaque_continuation_scope(scope);
            assert!(
                matches!(provider.adapter.stream(request), Err(
                ModelProviderError::RequestEncodingFailure {message}) if message.contains("unavailable")),
                "single-sample opaque bytes must never accumulate in the parent store"
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn encrypted_parent_continues_after_actual_no_change_diagnosis() {
        isolation(Case::NoChange).await;
    }
    #[tokio::test]
    async fn encrypted_parent_continues_after_actual_old_new_command_scopes() {
        isolation(Case::Commands).await;
    }
    #[tokio::test]
    async fn encrypted_parent_continues_after_a_staged_scope_fails() {
        isolation(Case::Failure).await;
    }
    #[tokio::test]
    async fn encrypted_parent_continues_after_a_staged_scope_is_cancelled() {
        isolation(Case::Cancelled).await;
    }

    #[test]
    fn single_sample_contract_rejects_incremental_and_opaque_inputs_before_credentials() {
        let input = ModelInputIR::current(
            "fixture",
            vec![ContentItem::user_text("fixture")],
            ModelRequestRequirements::default(),
        );
        let default = ModelRequest::full(input.clone());
        assert_eq!(
            default.continuation_retention(),
            ContinuationRetention::Session
        );
        assert_eq!(
            ModelRequest::incremental(input.clone(), ProviderResponseId::new("prior"))
                .continuation_retention(),
            ContinuationRetention::Session
        );
        let ordinary_parts = default.clone().into_parts();
        let single = default.with_continuation_retention(ContinuationRetention::SingleSample);
        assert_eq!(
            single.into_parts(),
            ordinary_parts,
            "non-wire sidecar preserves public tuple"
        );
        let session = ProviderSession::new(
            ProviderSessionId::new("fixture"),
            ProviderSessionRole::Primary,
            0,
        );
        let reference = OpaqueContinuationReference::new(
            OpaqueContinuationScope::for_provider_session(session.reference()),
            "fixture-id".to_owned(),
            1,
        );
        let invalid = [
            ModelRequest::incremental(input, ProviderResponseId::new("prior")),
            ModelRequest::full(ModelInputIR::current(
                "fixture",
                vec![ContentItem::OpaqueContinuation { reference }],
                ModelRequestRequirements::default(),
            )),
        ];
        let adapter = ResponsesHttpAdapter::new(
            ResponsesHttpConfig::new(
                "http://127.0.0.1:1/v1",
                "fixture",
                "must-not-resolve",
                capabilities(),
            )
            .with_codex_subscription_profile(),
            Arc::new(FixtureCredential),
        );
        for request in invalid {
            assert!(
                matches!(adapter.stream(request.with_continuation_retention(ContinuationRetention::SingleSample)),
                Err(ModelProviderError::RequestEncodingFailure {message}) if message.contains("single-sample"))
            );
        }
    }
}
