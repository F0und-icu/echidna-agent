//! An opaque subscription response must retain a real Core Memory replacement.
//!
//! Exercises the production App entry point and existing Context refresh/Audit owners.

mod opaque_core_memory_continuation {
    use std::convert::Infallible;
    use std::sync::Arc;

    use serde_json::{Value, json};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::mpsc;

    use crate::adapters::agent_definition::{load_definition_binding, write_builtin_definition};
    use crate::adapters::artifact_store::LocalArtifactStore;
    use crate::adapters::credentials::{
        AccountBoundCredential, CredentialResolution, CredentialSource, CredentialSourceError,
    };
    use crate::adapters::project_event_store::Project;
    use crate::adapters::project_memory::LocalProjectMemory;
    use crate::adapters::project_snapshot::LocalProjectSnapshot;
    use crate::adapters::responses::{ResponsesHttpAdapter, ResponsesHttpConfig};
    use crate::adapters::tools::ToolRegistry;
    use crate::app::agent_loop::{
        AgentLoop, AgentLoopError, AssistantTextDeltaStage, ChatDurabilityCheckpoint,
        ChatDurabilityStage, CliRuntimeStages, CurrentContextState, ToolExecutionStartStage,
        ViewImageFollowUpFuture,
    };
    use crate::app::cancellation::AgentCancellation;
    use crate::app::model_request_budget::ModelRequestBudget;
    use crate::app::request_audit::{RequestAuditService, RequestAuditServiceError};
    use crate::app::session::SessionProjection;
    use crate::domain::agent_definition::{AgentId, BuiltinAgentTemplate};
    use crate::domain::context::{
        ContextFragment, ContextFragmentContent, ContextFragmentId, ContextFragmentPriority,
        ContextFragmentSource, FragmentCachePolicy, ProviderSessionId, ProviderSessionRef,
        ProviderSessionRole,
    };
    use crate::domain::events::{EventEnvelope, EventId, EventSequence, OccurredAt, StreamId};
    use crate::domain::memory::{MemoryPath, MemoryScope};
    use crate::domain::model::{
        CapabilityMappingVersion, ContentItem, MessageRole, ModelCapabilities,
    };
    use crate::domain::request_audit::{RequestAuditBinding, RequestAuditRunContext};
    use crate::domain::research_control::PromptComposition;
    use crate::domain::session::SessionEvent;
    use crate::domain::tools::{ExecutionId, ToolCall, ToolCallId, ToolName, ToolResultStatus};
    use crate::domain::world_state::ProjectSnapshotContext;
    use crate::ports::EventSink;
    use crate::ports::tool::ToolExecutor;
    use crate::support::TemporaryDirectory;

    const CIPHERTEXT: &str = "ciphertext-core-memory-canary";
    const INITIAL_MEMORY: &str = "OPAQUE_MEMORY_INITIAL";
    const CURRENT_MEMORY: &str = "OPAQUE_MEMORY_CURRENT";

    struct FixtureCredential;

    impl CredentialSource for FixtureCredential {
        fn read(&self, _: &str) -> Result<Option<String>, CredentialSourceError> {
            panic!("subscription fixture resolves only its explicit account-bound credential")
        }

        fn resolve(&self, reference: &str) -> CredentialResolution {
            assert_eq!(reference, "codex_subscription:opaque-core-memory");
            CredentialResolution::PendingAccountBound(Box::pin(async {
                Ok(AccountBoundCredential::new(
                    "fixture-bearer".to_owned(),
                    "fixture-account".to_owned(),
                ))
            }))
        }
    }

    #[derive(Default)]
    struct AuditSink {
        events: Vec<EventEnvelope<Value>>,
    }

    impl EventSink<Value> for AuditSink {
        type Error = Infallible;

        fn append_event(&mut self, event: &EventEnvelope<Value>) -> Result<(), Self::Error> {
            self.events.push(event.clone());
            Ok(())
        }
    }

    fn ignore_tool_start(_: &crate::domain::tools::ToolExecutionPlan) {}

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

    fn encrypted_reasoning() -> Value {
        json!({
            "type": "reasoning",
            "id": "opaque-core-memory-reasoning",
            "summary": [],
            "encrypted_content": CIPHERTEXT,
        })
    }

    fn memory_write_call(expected_hash: &str) -> Value {
        json!({
            "type": "function_call",
            "call_id": "opaque-core-memory-write",
            "name": "memory_write",
            "arguments": json!({
                "path": "core.md",
                "content": CURRENT_MEMORY,
                "expected_hash": expected_hash,
                "event_id": "opaque-core-memory-change",
            }).to_string(),
        })
    }

    fn completed(id: &str, mut items: Vec<Value>, text: Option<&str>) -> String {
        if let Some(text) = text {
            items.push(json!({
                "type": "message",
                "id": format!("message-{id}"),
                "role": "assistant",
                "status": "completed",
                "content": [{"type": "output_text", "text": text}],
            }));
        }
        let mut events = items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                json!({"type": "response.output_item.done", "output_index": index, "item": item})
            })
            .collect::<Vec<_>>();
        events.push(json!({
            "type": "response.completed",
            "response": {"id": id, "status": "completed", "output": items},
        }));
        events
            .into_iter()
            .map(|event| format!("data: {event}\n\n"))
            .collect()
    }

    async fn read_request(socket: &mut TcpStream) -> Value {
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 4096];
        loop {
            let read = socket.read(&mut buffer).await.expect("read request bytes");
            assert_ne!(read, 0, "request connection closed before complete body");
            bytes.extend_from_slice(&buffer[..read]);
            assert!(bytes.len() < 1024 * 1024, "fixture request stays bounded");
            if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                let end = end + 4;
                let headers = std::str::from_utf8(&bytes[..end]).expect("valid request headers");
                let length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length").then(|| {
                            value
                                .trim()
                                .parse::<usize>()
                                .expect("numeric content length")
                        })
                    })
                    .expect("request supplies Content-Length");
                if bytes.len() >= end + length {
                    return serde_json::from_slice(&bytes[end..end + length])
                        .expect("request body is JSON");
                }
            }
        }
    }

    async fn server(
        bodies: Vec<String>,
    ) -> (String, mpsc::Receiver<Value>, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback Responses server");
        let address = listener.local_addr().expect("loopback address");
        let (sender, receiver) = mpsc::channel(bodies.len());
        let task = tokio::spawn(async move {
            for body in bodies {
                let (mut socket, _) = listener.accept().await.expect("accept Responses request");
                sender
                    .send(read_request(&mut socket).await)
                    .await
                    .expect("test receives each request");
                let headers = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                socket
                    .write_all(headers.as_bytes())
                    .await
                    .expect("write headers");
                socket.write_all(body.as_bytes()).await.expect("write body");
                socket.shutdown().await.expect("close response");
            }
        });
        (format!("http://{address}/v1"), receiver, task)
    }

    async fn write_memory(
        tools: &ToolRegistry,
        call_id: &str,
        content: &str,
        expected_hash: Value,
        event_id: &str,
    ) -> crate::domain::tools::ToolExecutionOutcome {
        let call = ToolCall::new(
            ToolCallId::new(call_id),
            ToolName::new("memory_write"),
            json!({
                "path": "core.md",
                "content": content,
                "expected_hash": expected_hash,
                "event_id": event_id,
            }),
        );
        let plan = tools
            .plan(ExecutionId::new(format!("execution-{call_id}")), call)
            .expect("plan known Memory write");
        tools.execute(plan).await.expect("Memory write completes")
    }

    fn current_context_state(
        project: &Project,
        tools: &ToolRegistry,
        memory_scope: &MemoryScope,
    ) -> Result<CurrentContextState, AgentLoopError> {
        let snapshot = LocalProjectSnapshot::new(project)
            .inspect(&tools.specs(), &[], "context-refresh")
            .map_err(|error| AgentLoopError::ContextRefresh {
                message: error.to_string(),
            })?;
        let project_snapshot = ProjectSnapshotContext::new(snapshot.snapshot).map_err(|error| {
            AgentLoopError::ContextRefresh {
                message: error.to_string(),
            }
        })?;
        Ok(CurrentContextState::new(
            None,
            project_snapshot,
            core_memory_fragment(project, memory_scope)?,
        ))
    }

    fn core_memory_fragment(
        project: &Project,
        memory_scope: &MemoryScope,
    ) -> Result<Option<ContextFragment>, AgentLoopError> {
        let path = MemoryPath::parse("core.md").expect("core.md is a fixed valid Memory path");
        let namespace = match memory_scope {
            MemoryScope::Project => "project".to_owned(),
            MemoryScope::Agent { agent_id } => format!("agent:{}", agent_id.as_str()),
        };
        let document = LocalProjectMemory::with_scope(project, memory_scope.clone())
            .read(&path)
            .map_err(|error| AgentLoopError::ContextRefresh {
                message: error.to_string(),
            })?;
        let Some(content) = document.content else {
            return Ok(None);
        };
        let (revision, content) = if content.is_empty() {
            (
                "empty".to_owned(),
                "Core Memory is explicitly empty.".to_owned(),
            )
        } else {
            (
                document
                    .hash
                    .expect("present LocalProjectMemory documents have a content hash"),
                content,
            )
        };
        ContextFragment::new(
            ContextFragmentId::new(format!("memory-core:{namespace}:{revision}")).map_err(
                |error| AgentLoopError::ContextRefresh {
                    message: error.to_string(),
                },
            )?,
            ContextFragmentSource::ProjectMemoryCore,
            ContextFragmentPriority::High,
            FragmentCachePolicy::ContentAddressed,
            ContextFragmentContent::ContentItems {
                items: vec![ContentItem::Text {
                    role: MessageRole::User,
                    text: content,
                }],
            },
        )
        .map(Some)
        .map_err(|error| AgentLoopError::ContextRefresh {
            message: error.to_string(),
        })
    }

    fn audit_projection(stream_id: StreamId) -> SessionProjection {
        SessionProjection::replay([SessionEvent::Started {
            primary_provider_session: ProviderSessionRef {
                id: ProviderSessionId::new("primary:0"),
                role: ProviderSessionRole::Primary,
                generation: 0,
            },
        }
        .into_envelope(
            EventId::new("opaque-core-memory-session-start"),
            stream_id,
            EventSequence::new(1),
            OccurredAt::new("2026-10-21T00:00:00Z"),
        )
        .expect("valid session started event")])
        .expect("valid session projection")
    }

    async fn assert_subscription_opaque_reasoning_preserves_a_real_core_memory_replacement(
        label: &str,
        memory_scope: MemoryScope,
    ) {
        let root = TemporaryDirectory::new(&format!("opaque-core-memory-continuation-{label}"));
        let project = Project::initialize(root.path()).expect("initialize project");
        let session_id = StreamId::new(format!("session:opaque-core-memory:{label}"));
        let definition_path = root.path().join("technical-writing");
        let definition_id = match &memory_scope {
            MemoryScope::Project => "opaque-core-memory-project",
            MemoryScope::Agent { agent_id } => agent_id.as_str(),
        };
        write_builtin_definition(
            BuiltinAgentTemplate::TechnicalWriting,
            &definition_path,
            Some(definition_id),
        )
        .expect("write a real generic AgentDefinition fixture");
        if memory_scope == MemoryScope::Project {
            let manifest = definition_path.join("agent.toml");
            let content = std::fs::read_to_string(&manifest)
                .expect("read generic AgentDefinition fixture")
                .replacen("memory = \"agent\"", "memory = \"project\"", 1);
            std::fs::write(&manifest, content).expect("select project Core Memory in fixture");
        }
        let catalog = ToolRegistry::for_project(project.root());
        let binding = load_definition_binding(&definition_path, None, None, &catalog.catalog())
            .expect("load generic AgentDefinition fixture against the full catalog");
        assert_eq!(
            crate::cli_agent::memory_scope(&binding),
            Some(memory_scope.clone()),
            "fixture binding selects the requested Memory scope"
        );
        assert!(binding.definition().research().is_none());
        assert!(binding.definition().rsi().is_none());
        let prompt = PromptComposition::from_binding(binding.clone())
            .expect("compose the bound generic Agent prompt");
        let tools = ToolRegistry::for_agent(
            project.root(),
            session_id.clone(),
            crate::cli_agent::memory_scope(&binding),
            binding.definition().research().is_some(),
        )
        .with_project(&project);
        let initial = write_memory(
            &tools,
            "opaque-core-memory-initial",
            INITIAL_MEMORY,
            Value::Null,
            "opaque-core-memory-initial",
        )
        .await;
        assert_eq!(initial.result.status, ToolResultStatus::Succeeded);
        let initial_hash = initial.result.output["new_hash"]
            .as_str()
            .expect("initial write reports its content hash")
            .to_owned();

        let (url, mut requests, server) = server(vec![
            completed(
                "opaque-core-memory-first",
                vec![encrypted_reasoning(), memory_write_call(&initial_hash)],
                None,
            ),
            completed(
                "opaque-core-memory-final",
                Vec::new(),
                Some("opaque core memory continuation complete"),
            ),
        ])
        .await;
        let provider = ResponsesHttpAdapter::new(
            ResponsesHttpConfig::new(
                url,
                "fixture-subscription",
                "codex_subscription:opaque-core-memory",
                capabilities(),
            )
            .with_codex_subscription_profile(),
            Arc::new(FixtureCredential),
        );
        let mut agent = AgentLoop::new(&provider, &tools)
            .with_prompt_composition(prompt)
            .with_output_store(LocalArtifactStore::for_project(&project));

        let binding = RequestAuditBinding::new(project.project_id().clone(), session_id)
            .expect("valid audit binding");
        let audit_context = RequestAuditRunContext::new(
            binding.clone(),
            ResponsesHttpAdapter::request_adapter_version(),
        );
        let audit_store = LocalArtifactStore::for_project(&project);
        let audit_service = RequestAuditService::new(&audit_store, binding.clone());
        let mut projection = audit_projection(binding.session_stream_id().clone());
        let mut sink = AuditSink::default();
        let mut manifests = Vec::new();
        let mut audit_sequence = 0_u8;
        let mut record_attempt = |attempt| {
            audit_sequence += 1;
            audit_service
                .record(
                    attempt,
                    EventId::new(format!("opaque-core-memory-audit-{audit_sequence}")),
                    OccurredAt::new(format!("2026-10-21T00:00:0{audit_sequence}Z")),
                    &mut projection,
                    &mut sink,
                )
                .map(|manifest| manifests.push(manifest))
                .map_err(RequestAuditServiceError::into_agent_loop_error)
        };
        let mut follow_up = |_selection| -> ViewImageFollowUpFuture<'_> {
            Box::pin(async { unreachable!("memory_write does not produce an image selection") })
        };
        let mut text_delta = |_text: &str| Ok(());
        let mut tool_start = ignore_tool_start;
        let mut durability = |_checkpoint: ChatDurabilityCheckpoint<'_>| Ok(());
        let context_memory_scope = memory_scope.clone();
        let mut context_refresh = || current_context_state(&project, &tools, &context_memory_scope);

        let outcome = agent
            .run_turn_audited_with_context_refresh_and_vision_stages(
                "replace Core Memory",
                Vec::new(),
                AgentCancellation::new(),
                &audit_context,
                &mut record_attempt,
                CliRuntimeStages::new(
                    &mut follow_up,
                    &mut text_delta as &mut AssistantTextDeltaStage<'_>,
                    &mut tool_start as &mut ToolExecutionStartStage<'_>,
                    &mut durability as &mut ChatDurabilityStage<'_>,
                    &mut context_refresh,
                    Arc::new(ModelRequestBudget::new(3)),
                ),
                None,
            )
            .await
            .expect("opaque continuation with a proven Core Memory replacement completes");
        let crate::app::agent_loop::AgentTurnOutcome::Completed {
            model_requests,
            executions,
            ..
        } = &outcome
        else {
            panic!("{outcome:?}\n{:?}", agent.history());
        };
        assert_eq!(*model_requests, 2);
        assert_eq!(executions.len(), 1);
        assert_eq!(executions[0].result.status, ToolResultStatus::Succeeded);
        assert_eq!(agent.history().ensure_all_tool_calls_paired(), Ok(()));
        assert!(agent.history().items().iter().all(|item| {
            !matches!(
                item,
                ContentItem::Reasoning { .. } | ContentItem::OpaqueContinuation { .. }
            )
        }));
        assert!(!format!("{:?}", agent.history()).contains(CIPHERTEXT));

        let first_wire = requests.recv().await.expect("first wire request");
        let second_wire = requests.recv().await.expect("continuation wire request");
        assert!(
            requests.try_recv().is_err(),
            "no retry or Tool replay request"
        );
        server.await.expect("loopback server completes");
        assert!(!first_wire.to_string().contains(CIPHERTEXT));
        assert!(
            first_wire["tools"]
                .as_array()
                .expect("initial request advertises its Tool plan")
                .iter()
                .any(|tool| tool["name"] == "memory_write"),
            "the bound AgentDefinition advertises its direct Memory write Tool"
        );
        assert_eq!(
            [first_wire.clone(), second_wire.clone()]
                .iter()
                .filter(|wire| wire.to_string().contains(CIPHERTEXT))
                .count(),
            1,
            "ciphertext exists only in the Adapter's encrypted continuation replay"
        );
        let input = second_wire["input"]
            .as_array()
            .expect("continuation request contains input items");
        let opaque_index = input
            .iter()
            .position(|item| item["type"] == "reasoning" && item["encrypted_content"] == CIPHERTEXT)
            .expect("Adapter replays encrypted reasoning");
        let call_index = input
            .iter()
            .position(|item| {
                item["type"] == "function_call" && item["call_id"] == "opaque-core-memory-write"
            })
            .expect("continuation retains the original Memory Tool Call");
        let result_index = input
            .iter()
            .position(|item| {
                item["type"] == "function_call_output"
                    && item["call_id"] == "opaque-core-memory-write"
            })
            .expect("continuation retains the paired Memory Tool Result");
        assert!(
            input[result_index].to_string().contains("new_hash"),
            "the continuation carries the successful Memory write result"
        );
        let replacement_index = input
            .iter()
            .position(|item| {
                item["type"] == "message"
                    && item
                        .to_string()
                        .contains("Core Memory replacement for the current state.")
            })
            .expect("continuation appends the current Core Memory replacement");
        assert!(
            opaque_index < call_index
                && call_index < result_index
                && result_index < replacement_index
        );
        assert!(
            input[replacement_index]
                .to_string()
                .contains("supersedes earlier Core Memory snapshots")
                && input[replacement_index]
                    .to_string()
                    .contains(CURRENT_MEMORY)
        );

        assert_eq!(
            std::fs::read_to_string(
                project
                    .root()
                    .join(".echidna")
                    .join(memory_scope.storage_directory())
                    .join("core.md")
            )
            .expect("read current Core Memory"),
            CURRENT_MEMORY
        );
        let research = std::fs::read_to_string(project.root().join(".echidna/research.jsonl"))
            .expect("read durable Research stream");
        assert!(research.contains("memory.changed"));
        assert!(research.contains("opaque-core-memory-change"));
        assert!(
            !project.root().join(".echidna/evolution").exists(),
            "RSI must remain inactive for {label}"
        );

        assert_eq!(manifests.len(), 2);
        assert_eq!(projection.state().request_audits(), manifests);
        assert_eq!(sink.events.len(), manifests.len());
        let mut audited_opaque_references = Vec::new();
        for manifest in &manifests {
            let reconstructed = audit_service
                .reconstruct(manifest)
                .expect("audit reconstructs actual semantic delivery");
            assert!(
                !format!("{manifest:?}{reconstructed:?}").contains(CIPHERTEXT),
                "ciphertext never enters a semantic request audit"
            );
            audited_opaque_references.push(
                reconstructed
                    .content_items
                    .iter()
                    .filter(|item| matches!(item, ContentItem::OpaqueContinuation { .. }))
                    .count(),
            );
        }
        assert_eq!(audited_opaque_references, vec![0, 1]);
    }

    #[tokio::test]
    async fn subscription_opaque_reasoning_preserves_project_and_agent_core_memory_replacements() {
        for (label, memory_scope) in [
            ("project", MemoryScope::Project),
            (
                "report-agent",
                MemoryScope::Agent {
                    agent_id: AgentId::new("report-agent").expect("valid fixture Agent ID"),
                },
            ),
        ] {
            assert_subscription_opaque_reasoning_preserves_a_real_core_memory_replacement(
                label,
                memory_scope,
            )
            .await;
        }
    }
}
