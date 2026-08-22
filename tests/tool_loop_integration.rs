use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};

use cyber_agent::adapters::project_event_store::Project;
use cyber_agent::adapters::tools::ToolRegistry;
use cyber_agent::app::agent_loop::{AgentLoop, AgentLoopError, AgentTurnOutcome};
use cyber_agent::app::cancellation::AgentCancellation;
use cyber_agent::domain::artifacts::ArtifactId;
use cyber_agent::domain::context::{
    BaselineAssessment, BaselineInvalidationReason, ModelIdentity, ModelProfile, ProviderSession,
    ProviderSessionId, ProviderSessionRole,
};
use cyber_agent::domain::events::{EventEnvelope, StreamId};
use cyber_agent::domain::model::{
    CapabilityMappingVersion, ContentItem, ModelCapabilities, ModelEvent, ModelInputIR,
};
use cyber_agent::domain::tools::{
    ExecutionId, ResourceKey, ToolCall, ToolCallId, ToolExecutionOutcome, ToolExecutionPlan,
    ToolName, ToolResultStatus, ToolSpec,
};
use cyber_agent::ports::model::{
    ModelEventStream, ModelProvider, ModelProviderError, ModelRequest, ModelResponseCompletion,
    ModelStreamEvent,
};
use cyber_agent::ports::tool::{ToolExecutionFuture, ToolExecutor, ToolExecutorError};
use futures_core::Stream;
use futures_util::stream;
use serde_json::{Value, json};

static NEXT_PROJECT_ID: AtomicU64 = AtomicU64::new(1);

struct TestProject {
    root: PathBuf,
}

impl TestProject {
    fn new(label: &str) -> Self {
        let id = NEXT_PROJECT_ID.fetch_add(1, Ordering::Relaxed);
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/m1a-s3-tests")
            .join(format!("{label}-{}-{id}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    fn write(&self, relative: &str, content: impl AsRef<[u8]>) {
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }
}

impl Drop for TestProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn tool_call(id: &str, name: &str, arguments: Value) -> ToolCall {
    ToolCall::new(ToolCallId::new(id), ToolName::new(name), arguments)
}

fn plan(tools: &impl ToolExecutor, id: &str, name: &str, arguments: Value) -> ToolExecutionPlan {
    tools
        .plan(
            ExecutionId::new(format!("execution-{id}")),
            tool_call(id, name, arguments),
        )
        .unwrap()
}

#[tokio::test]
async fn builtin_tools_cover_files_search_argv_failures_and_bounded_output() {
    let project = TestProject::new("builtins");
    project.write(
        "src/lib.rs",
        "// candidate: path traversal\npub fn normalize(path: &str) -> &str { path }\n",
    );
    project.write("large.txt", "x".repeat(140 * 1024));
    let tools = ToolRegistry::for_project(&project.root);

    let specs = tools.specs();
    assert_eq!(
        specs
            .iter()
            .map(|spec| spec.name.as_str())
            .collect::<Vec<_>>(),
        vec![
            "apply_patch",
            "artifact_store",
            "input_job",
            "input_pty",
            "inspect_project",
            "list_files",
            "memory_read",
            "memory_write",
            "poll_job",
            "poll_pty",
            "read_file",
            "read_files",
            "research_get",
            "research_update",
            "resize_pty",
            "run_command",
            "run_commands",
            "search_files",
            "start_job",
            "start_pty",
            "stop_job",
            "stop_pty",
            "write_file",
        ]
    );
    let run_spec = specs
        .iter()
        .find(|spec| spec.name.as_str() == "run_command")
        .unwrap();
    let resource_schema = &run_spec.input_schema["properties"]["resources"]["items"];
    assert_eq!(resource_schema["oneOf"].as_array().map(Vec::len), Some(3));
    assert_eq!(
        resource_schema["oneOf"][0]["properties"]["kind"]["const"],
        "device"
    );
    assert_eq!(
        resource_schema["oneOf"][1]["properties"]["kind"]["const"],
        "pty"
    );
    assert_eq!(
        resource_schema["oneOf"][2]["properties"]["kind"]["const"],
        "output"
    );
    let batch_spec = specs
        .iter()
        .find(|spec| spec.name.as_str() == "run_commands")
        .unwrap();
    assert_eq!(
        batch_spec.input_schema["properties"]["commands"]["items"]["properties"]["resources"]["items"],
        resource_schema.clone()
    );
    assert_eq!(
        (run_spec.effect, run_spec.retry_policy, run_spec.determinism),
        (
            cyber_agent::domain::tools::ToolEffect::Interactive,
            cyber_agent::domain::tools::ToolRetryPolicy::NeverAutoRetry,
            cyber_agent::domain::tools::ToolDeterminism::Nondeterministic,
        )
    );
    let normalized_output_plan = plan(
        &tools,
        "normalized-output",
        "run_command",
        json!({
            "argv": ["rustc", "--version"],
            "resources": [{"kind": "output", "path": "artifacts/../audit.txt"}]
        }),
    );
    assert_eq!(
        normalized_output_plan.resolved_resource_keys,
        vec![ResourceKey::output(
            project.root.join("audit.txt").display().to_string()
        )]
    );

    let read = tools
        .execute(plan(
            &tools,
            "read",
            "read_file",
            json!({"path": "src/lib.rs"}),
        ))
        .await;
    assert_eq!(read.result.status, ToolResultStatus::Succeeded);
    assert_eq!(
        read.result.output["content"],
        "// candidate: path traversal\npub fn normalize(path: &str) -> &str { path }\n"
    );

    let listed = tools
        .execute(plan(&tools, "list", "list_files", json!({"path": "."})))
        .await;
    assert_eq!(listed.result.status, ToolResultStatus::Succeeded);
    assert!(
        listed.result.output["paths"]
            .as_str()
            .unwrap()
            .contains("src/lib.rs")
    );

    let found = tools
        .execute(plan(
            &tools,
            "search",
            "search_files",
            json!({"pattern": "path traversal", "paths": ["src"]}),
        ))
        .await;
    assert_eq!(found.result.status, ToolResultStatus::Succeeded);
    assert!(
        found.result.output["matches"]
            .as_str()
            .unwrap()
            .contains("src/lib.rs")
    );

    let no_match = tools
        .execute(plan(
            &tools,
            "no-match",
            "search_files",
            json!({"pattern": "not-present", "paths": ["src"]}),
        ))
        .await;
    assert_eq!(no_match.result.status, ToolResultStatus::Succeeded);
    assert_eq!(no_match.result.output["matches"], "");

    let command = tools
        .execute(plan(
            &tools,
            "command",
            "run_command",
            json!({"argv": ["rustc", "--version"]}),
        ))
        .await;
    assert_eq!(command.result.status, ToolResultStatus::Succeeded);
    assert!(
        command.result.output["stdout"]
            .as_str()
            .unwrap()
            .starts_with("rustc ")
    );

    let nonzero = tools
        .execute(plan(
            &tools,
            "nonzero",
            "run_command",
            json!({"argv": ["rustc", "--definitely-invalid-cyber-agent-flag"]}),
        ))
        .await;
    assert_eq!(nonzero.result.status, ToolResultStatus::Failed);
    assert_ne!(nonzero.result.output["exit_code"], 0);

    let large = tools
        .execute(plan(
            &tools,
            "large",
            "read_file",
            json!({"path": "large.txt"}),
        ))
        .await;
    assert_eq!(large.result.status, ToolResultStatus::Succeeded);
    assert_eq!(large.result.output["truncated"], true);
    assert_eq!(
        large.result.output["content"].as_str().unwrap().len(),
        128 * 1024
    );

    assert!(matches!(
        tools.plan(
            ExecutionId::new("execution-shell-string"),
            tool_call(
                "shell-string",
                "run_command",
                json!({"command": "echo unsafe"}),
            ),
        ),
        Err(ToolExecutorError::InvalidArguments { .. })
    ));
    assert!(matches!(
        tools.plan(
            ExecutionId::new("execution-invalid-resource"),
            tool_call(
                "invalid-resource",
                "run_command",
                json!({
                    "argv": ["true"],
                    "resources": [{"kind": "device", "serial": "one", "extra": true}]
                }),
            ),
        ),
        Err(ToolExecutorError::InvalidArguments { .. })
    ));

    let read_batch = tools
        .execute(plan(
            &tools,
            "read-batch",
            "read_files",
            json!({
                "files": [
                    {"path": "src/lib.rs", "start_line": 1, "end_line": 1},
                    {"path": "does-not-exist.txt"}
                ]
            }),
        ))
        .await;
    assert_eq!(read_batch.result.status, ToolResultStatus::Succeeded);
    assert_eq!(read_batch.result.output["partial_failure"], true);
    assert_eq!(read_batch.result.output["files"][0]["status"], "succeeded");
    assert_eq!(read_batch.result.output["files"][1]["status"], "failed");

    let search_batch = tools
        .execute(plan(
            &tools,
            "search-batch",
            "search_files",
            json!({"patterns": ["path traversal", "not-present"], "paths": ["src"]}),
        ))
        .await;
    assert_eq!(search_batch.result.status, ToolResultStatus::Succeeded);
    assert_eq!(search_batch.result.output["partial_failure"], false);
    assert_eq!(
        search_batch.result.output["results"]
            .as_array()
            .map(Vec::len),
        Some(2)
    );

    let command_batch = tools
        .execute(plan(
            &tools,
            "command-batch",
            "run_commands",
            json!({
                "mode": "parallel",
                "commands": [
                    {"argv": ["rustc", "--version"]},
                    {"argv": ["rustc", "--definitely-invalid-cyber-agent-flag"]}
                ]
            }),
        ))
        .await;
    assert_eq!(command_batch.result.status, ToolResultStatus::Succeeded);
    assert_eq!(command_batch.result.output["partial_failure"], true);
    assert_eq!(
        command_batch.result.output["commands"][0]["status"],
        "succeeded"
    );
    assert_eq!(
        command_batch.result.output["commands"][1]["status"],
        "failed"
    );
}

#[cfg(unix)]
#[test]
fn output_resource_keys_resolve_missing_symlinked_outputs_to_one_key() {
    let project = TestProject::new("symlinked-output-resource");
    fs::create_dir_all(project.root.join("real")).unwrap();
    std::os::unix::fs::symlink("real", project.root.join("alias")).unwrap();
    let tools = ToolRegistry::for_project(&project.root);

    let alias = plan(
        &tools,
        "alias-output",
        "run_command",
        json!({
            "argv": ["true"],
            "resources": [{"kind": "output", "path": "alias/out.txt"}]
        }),
    );
    let real = plan(
        &tools,
        "real-output",
        "run_command",
        json!({
            "argv": ["true"],
            "resources": [{"kind": "output", "path": "real/out.txt"}]
        }),
    );

    assert_eq!(alias.resolved_resource_keys, real.resolved_resource_keys);
}

#[tokio::test]
async fn run_commands_parallel_mode_uses_explicit_resource_waves() {
    let project = TestProject::new("command-resource-waves");
    let tools = ToolRegistry::for_project(&project.root);
    let barrier =
        "touch \"$1\"; for _ in $(seq 1 50); do [ -f \"$2\" ] && exit 0; sleep 0.01; done; exit 73";

    for (index, (first_resource, second_resource, expected_statuses)) in [
        (
            json!({"kind": "device", "serial": "first"}),
            json!({"kind": "device", "serial": "second"}),
            ["succeeded", "succeeded"],
        ),
        (
            json!({"kind": "device", "serial": "shared"}),
            json!({"kind": "device", "serial": "shared"}),
            ["failed", "succeeded"],
        ),
        (
            json!({"kind": "pty", "session": "shared"}),
            json!({"kind": "pty", "session": "shared"}),
            ["failed", "succeeded"],
        ),
        (
            json!({"kind": "output", "path": "shared-output.txt"}),
            json!({"kind": "output", "path": "shared-output.txt"}),
            ["failed", "succeeded"],
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let first_marker = project.root.join(format!("marker-{index}-first"));
        let second_marker = project.root.join(format!("marker-{index}-second"));
        let command = |own: &PathBuf, peer: &PathBuf, resource: Value| {
            json!({
                "argv": [
                    "sh",
                    "-c",
                    barrier,
                    "cyber-agent-test-marker",
                    own.display().to_string(),
                    peer.display().to_string()
                ],
                "resources": [resource]
            })
        };
        let outcome = tools
            .execute(plan(
                &tools,
                &format!("resource-wave-{index}"),
                "run_commands",
                json!({
                    "mode": "parallel",
                    "commands": [
                        command(&first_marker, &second_marker, first_resource),
                        command(&second_marker, &first_marker, second_resource)
                    ]
                }),
            ))
            .await;

        assert_eq!(outcome.result.status, ToolResultStatus::Succeeded);
        assert_eq!(
            outcome.result.output["commands"][0]["status"],
            expected_statuses[0]
        );
        assert_eq!(
            outcome.result.output["commands"][1]["status"],
            expected_statuses[1]
        );
    }
}

fn capabilities() -> ModelCapabilities {
    ModelCapabilities {
        text_input: true,
        image_input: false,
        streaming: true,
        tool_call: true,
        structured_output: false,
        incremental_input: false,
        usage_reporting: false,
        context_window: 128_000,
        capability_mapping_version: CapabilityMappingVersion::new(1),
    }
}

fn incremental_capabilities() -> ModelCapabilities {
    ModelCapabilities {
        incremental_input: true,
        ..capabilities()
    }
}

struct ScriptedProvider {
    capabilities: ModelCapabilities,
    responses: Mutex<VecDeque<ScriptedResponse>>,
    requests: Mutex<Vec<ModelRequest>>,
}

struct ScriptedResponse {
    stream: Vec<Result<ModelStreamEvent, ModelProviderError>>,
    pending: bool,
}

impl ScriptedProvider {
    fn new(responses: impl IntoIterator<Item = Vec<ModelEvent>>) -> Self {
        Self::with_capabilities(
            capabilities(),
            responses
                .into_iter()
                .map(|events| completed_response(events, None)),
        )
    }

    fn with_capabilities(
        capabilities: ModelCapabilities,
        responses: impl IntoIterator<Item = ScriptedResponse>,
    ) -> Self {
        Self {
            capabilities,
            responses: Mutex::new(responses.into_iter().collect()),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn inputs(&self) -> Vec<ModelInputIR> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .map(|request| request.input().clone())
            .collect()
    }

    fn requests(&self) -> Vec<ModelRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl ModelProvider for ScriptedProvider {
    fn capabilities(&self) -> ModelCapabilities {
        self.capabilities.clone()
    }

    fn stream(&self, request: ModelRequest) -> Result<ModelEventStream, ModelProviderError> {
        request.validate_capabilities(&self.capabilities())?;
        self.requests.lock().unwrap().push(request);
        let response = self.responses.lock().unwrap().pop_front().ok_or_else(|| {
            ModelProviderError::ProviderFailure {
                code: Some("fixture_exhausted".to_owned()),
                message: "the scripted Provider has no response left".to_owned(),
            }
        })?;
        if response.pending {
            Ok(Box::pin(PendingScriptedStream))
        } else {
            Ok(Box::pin(stream::iter(response.stream)))
        }
    }
}

fn completed_response(events: Vec<ModelEvent>, response_id: Option<&str>) -> ScriptedResponse {
    let completion = response_id.map_or_else(
        ModelResponseCompletion::without_response_id,
        |response_id| {
            ModelResponseCompletion::new(cyber_agent::ports::model::ProviderResponseId::new(
                response_id,
            ))
        },
    );
    ScriptedResponse {
        stream: events
            .into_iter()
            .map(|event| Ok(ModelStreamEvent::Event(event)))
            .chain(std::iter::once(Ok(ModelStreamEvent::Completed(completion))))
            .collect(),
        pending: false,
    }
}

fn rejected_response(prefix: Vec<ModelEvent>) -> ScriptedResponse {
    ScriptedResponse {
        stream: prefix
            .into_iter()
            .map(|event| Ok(ModelStreamEvent::Event(event)))
            .chain(std::iter::once(Err(
                ModelProviderError::ContinuationRejected {
                    code: "previous_response_not_found".to_owned(),
                },
            )))
            .collect(),
        pending: false,
    }
}

fn failed_response(error: ModelProviderError) -> ScriptedResponse {
    ScriptedResponse {
        stream: vec![Err(error)],
        pending: false,
    }
}

fn pending_response() -> ScriptedResponse {
    ScriptedResponse {
        stream: Vec::new(),
        pending: true,
    }
}

struct PendingScriptedStream;

impl Stream for PendingScriptedStream {
    type Item = Result<ModelStreamEvent, ModelProviderError>;

    fn poll_next(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Pending
    }
}

#[tokio::test]
async fn agent_loop_keeps_full_resampling_after_local_baseline_comparison() {
    let project = TestProject::new("agent-loop");
    project.write(
        "src/parser.rs",
        "// candidate sink: path traversal\npub fn open(user_path: &str) { /* fixture */ }\n",
    );
    let requested_call = tool_call(
        "call-search",
        "search_files",
        json!({"pattern": "path traversal", "paths": ["src"]}),
    );
    let provider = ScriptedProvider::new([
        vec![
            ModelEvent::TextDelta {
                text: "I will search the local source.".to_owned(),
            },
            ModelEvent::ToolCall {
                call: requested_call.clone(),
            },
            ModelEvent::TextDelta {
                text: "The search result will drive the next sample.".to_owned(),
            },
        ],
        vec![ModelEvent::TextDelta {
            text: "Candidate identified: path traversal marker in src/parser.rs.".to_owned(),
        }],
    ]);
    let tools = ToolRegistry::for_project(&project.root);
    let mut agent = AgentLoop::with_primary_context(
        &provider,
        &tools,
        "Use local tools and keep candidates distinct from verified findings.",
        ModelIdentity::new("fixture-primary", "fixture-model-v1"),
        ModelProfile::new("fixture-research", 1),
        ProviderSession::new(
            ProviderSessionId::new("fixture-primary-session"),
            ProviderSessionRole::Primary,
            1,
        ),
    )
    .unwrap();

    let outcome = agent
        .run_turn(
            "Inspect the fixture for vulnerability candidates.",
            AgentCancellation::new(),
        )
        .await
        .unwrap();

    let AgentTurnOutcome::Completed {
        assistant_text,
        model_requests,
        executions,
    } = outcome
    else {
        panic!("the fixture turn should complete")
    };
    assert_eq!(model_requests, 2);
    assert_eq!(executions.len(), 1);
    assert_eq!(executions[0].result.status, ToolResultStatus::Succeeded);
    assert!(
        executions[0].result.output["matches"]
            .as_str()
            .unwrap()
            .contains("path traversal")
    );
    assert!(assistant_text.contains("Candidate identified"));
    assert_eq!(agent.history().ensure_all_tool_calls_paired(), Ok(()));

    let inputs = provider.inputs();
    assert_eq!(inputs.len(), 2);
    assert_eq!(inputs[0].tool_specs.len(), 23);
    assert_eq!(inputs[0].content_items.len(), 1);
    assert_eq!(
        inputs[1].content_items[1],
        ContentItem::assistant_text("I will search the local source.")
    );
    assert!(matches!(
        &inputs[1].content_items[2],
        ContentItem::ToolCall { call } if call == &requested_call
    ));
    assert_eq!(
        inputs[1].content_items[3],
        ContentItem::assistant_text("The search result will drive the next sample.")
    );
    assert!(matches!(
        &inputs[1].content_items[4],
        ContentItem::ToolResult { result }
            if result.status == ToolResultStatus::Succeeded
                && result.output["matches"].as_str().unwrap().contains("path traversal")
    ));
}

#[tokio::test]
async fn agent_loop_executes_research_update_through_the_fixed_registry() {
    let project = TestProject::new("agent-loop-research-update");
    Project::initialize(&project.root).unwrap();
    let requested_call = tool_call(
        "call-research-update",
        "research_update",
        json!({
            "event_id": "event-agent-loop-target",
            "occurred_at": "2026-08-22T12:00:00Z",
            "operation": {
                "kind": "record_target_snapshot",
                "record": {
                    "id": "snapshot-agent-loop",
                    "target_id": "fixture.target",
                    "kind": "source",
                    "locator": "fixture/src",
                    "version": "1.0.0",
                    "build_id": "fixture-build",
                    "environment_fingerprint": "fixture-environment",
                    "device_fingerprint": "unknown",
                    "captured_at": "2026-08-22T11:59:00Z"
                }
            }
        }),
    );
    let provider = ScriptedProvider::new([
        vec![ModelEvent::ToolCall {
            call: requested_call.clone(),
        }],
        vec![ModelEvent::TextDelta {
            text: "Research snapshot was recorded.".to_owned(),
        }],
    ]);
    let tools = ToolRegistry::for_project(&project.root);
    let mut agent = AgentLoop::new(&provider, &tools, "fixture");

    let outcome = agent
        .run_turn("Record the fixture target.", AgentCancellation::new())
        .await
        .unwrap();

    let AgentTurnOutcome::Completed { executions, .. } = outcome else {
        panic!("the fixture turn should complete")
    };
    assert_eq!(executions.len(), 1);
    assert_eq!(executions[0].result.status, ToolResultStatus::Succeeded);
    assert_eq!(
        executions[0].result.output["event_type"],
        "research.target_snapshot_recorded"
    );
    let persisted = fs::read_to_string(project.root.join(".cyber-agent/research.jsonl")).unwrap();
    assert!(persisted.contains("research.target_snapshot_recorded"));
    assert!(persisted.contains("snapshot-agent-loop"));
    assert_eq!(agent.history().ensure_all_tool_calls_paired(), Ok(()));
}

#[tokio::test]
async fn agent_loop_cache_hit_creates_a_fresh_receipt_linked_to_the_source_execution() {
    let project = TestProject::new("agent-loop-tool-cache");
    Project::initialize(&project.root).unwrap();
    let tools = ToolRegistry::for_project_with_session(
        &project.root,
        StreamId::new("agent-loop-tool-cache"),
    )
    .unwrap();
    let source = tools
        .execute(plan(&tools, "cache-source", "research_get", json!({})))
        .await;
    assert_eq!(source.result.status, ToolResultStatus::Succeeded);
    assert!(source.receipt.cache_source.is_none());

    let requested_call = tool_call("cache-hit", "research_get", json!({}));
    let provider = ScriptedProvider::new([
        vec![ModelEvent::ToolCall {
            call: requested_call,
        }],
        vec![ModelEvent::TextDelta {
            text: "cache result observed".to_owned(),
        }],
    ]);
    let mut agent = AgentLoop::new(&provider, &tools, "Use project research facts.");
    let outcome = agent
        .run_turn("Read the current Research state.", AgentCancellation::new())
        .await
        .unwrap();
    let AgentTurnOutcome::Completed { executions, .. } = outcome else {
        panic!("the scripted cache-hit turn should complete")
    };
    assert_eq!(executions.len(), 1);
    let hit = &executions[0];
    assert_eq!(hit.receipt.execution_id.as_str(), "execution:cache-hit");
    assert_ne!(hit.receipt.execution_id, source.receipt.execution_id);
    assert_eq!(
        hit.receipt
            .cache_source
            .as_ref()
            .map(|reference| reference.source_execution_id.as_str()),
        Some("execution-cache-source")
    );
    assert_eq!(hit.result.call_id.as_str(), "cache-hit");
    let events = fs::read_to_string(
        project
            .root
            .join(".cyber-agent/sessions/agent-loop-tool-cache/events.jsonl"),
    )
    .unwrap()
    .lines()
    .map(|line| serde_json::from_str::<EventEnvelope<Value>>(line).unwrap())
    .collect::<Vec<_>>();
    assert_eq!(events.len(), 2);
    assert_eq!(
        events[0].event_type.as_str(),
        "tool.execution_receipt_recorded"
    );
    assert_eq!(events[1].event_type.as_str(), "tool.result_cache_hit");
    assert_eq!(
        hit.receipt
            .cache_source
            .as_ref()
            .map(|source| source.source_receipt_event_id.clone()),
        Some(events[0].event_id.clone())
    );
    let source = hit.receipt.cache_source.as_ref().unwrap();
    assert_eq!(
        events[1].payload["receipt"]["execution_id"],
        "execution:cache-hit"
    );
    assert_eq!(
        events[1].payload["receipt"]["cache_source"]["source_stream_id"],
        source.source_stream_id.as_str()
    );
    assert_eq!(
        events[1].payload["receipt"]["cache_source"]["source_receipt_event_id"],
        events[0].event_id.as_str()
    );
    assert_eq!(
        events[1].payload["receipt"]["cache_source"]["source_execution_id"],
        source.source_execution_id.as_str()
    );
    assert_eq!(
        events[1].payload["receipt"]["cache_source"]["source_plan_fingerprint"],
        source.source_plan_fingerprint
    );
    assert_eq!(
        events[1].payload["receipt"]["cache_source"]["cache_key"],
        source.cache_key.as_str()
    );
}

#[tokio::test]
async fn agent_loop_executes_inspect_project_through_the_fixed_registry() {
    let project = TestProject::new("agent-loop-inspect-project");
    Project::initialize(&project.root).unwrap();
    project.write("target.rs", "fn fixture() {}\n");
    let requested_call = tool_call(
        "call-inspect-project",
        "inspect_project",
        json!({"observed_at": "2026-08-23T12:00:00Z"}),
    );
    let provider = ScriptedProvider::new([
        vec![ModelEvent::ToolCall {
            call: requested_call,
        }],
        vec![ModelEvent::TextDelta {
            text: "Project snapshot is ready.".to_owned(),
        }],
    ]);
    let tools = ToolRegistry::for_project(&project.root);
    let mut agent = AgentLoop::new(&provider, &tools, "fixture");

    let outcome = agent
        .run_turn("Inspect this project.", AgentCancellation::new())
        .await
        .unwrap();

    let AgentTurnOutcome::Completed { executions, .. } = outcome else {
        panic!("the fixture turn should complete")
    };
    assert_eq!(executions.len(), 1);
    assert_eq!(executions[0].result.status, ToolResultStatus::Succeeded);
    assert_eq!(executions[0].result.output["status"], "full_rebuild");
    assert!(
        project
            .root
            .join(".cyber-agent/cache/project-snapshot.json")
            .is_file()
    );
    assert_eq!(agent.history().ensure_all_tool_calls_paired(), Ok(()));
}

#[tokio::test]
async fn agent_loop_executes_artifact_store_through_the_fixed_registry() {
    let project = TestProject::new("agent-loop-artifact-store");
    Project::initialize(&project.root).unwrap();
    project.write("fixture.bin", b"fixture Artifact bytes");
    let tools = ToolRegistry::for_project(&project.root);
    let snapshot = tools
        .execute(plan(
            &tools,
            "artifact-snapshot",
            "research_update",
            json!({
                "event_id": "event-agent-loop-artifact-snapshot",
                "occurred_at": "2026-08-23T12:00:00Z",
                "operation": {
                    "kind": "record_target_snapshot",
                    "record": {
                        "id": "snapshot-agent-loop-artifact",
                        "target_id": "fixture.target",
                        "kind": "source",
                        "locator": "fixture/src",
                        "version": "1.0.0",
                        "build_id": "fixture-build",
                        "target_artifact_ids": ["artifact-agent-loop"],
                        "environment_fingerprint": "fixture-environment",
                        "device_fingerprint": "unknown",
                        "captured_at": "2026-08-23T11:59:00Z"
                    }
                }
            }),
        ))
        .await;
    assert_eq!(snapshot.result.status, ToolResultStatus::Succeeded);

    let requested_call = tool_call(
        "call-artifact-store",
        "artifact_store",
        json!({
            "event_id": "event-agent-loop-artifact",
            "occurred_at": "2026-08-23T12:01:00Z",
            "artifact_id": "artifact-agent-loop",
            "source_path": project.root.join("fixture.bin").display().to_string(),
            "media_type": "application/octet-stream",
            "target_snapshot_id": "snapshot-agent-loop-artifact",
            "toolchain_fingerprint": "fixture-toolchain-v1"
        }),
    );
    let provider = ScriptedProvider::new([
        vec![ModelEvent::ToolCall {
            call: requested_call.clone(),
        }],
        vec![ModelEvent::TextDelta {
            text: "Artifact was stored.".to_owned(),
        }],
    ]);
    let mut agent = AgentLoop::new(&provider, &tools, "fixture");

    let outcome = agent
        .run_turn("Store the fixture Artifact.", AgentCancellation::new())
        .await
        .unwrap();

    let AgentTurnOutcome::Completed { executions, .. } = outcome else {
        panic!("the fixture turn should complete")
    };
    assert_eq!(executions.len(), 1);
    assert_eq!(executions[0].result.status, ToolResultStatus::Succeeded);
    assert_eq!(
        executions[0].receipt.output_artifact_ids,
        vec![ArtifactId::new("artifact-agent-loop")]
    );
    assert_eq!(
        executions[0].result.output["event_type"],
        "artifact.created"
    );
    let persisted = fs::read_to_string(project.root.join(".cyber-agent/research.jsonl")).unwrap();
    assert!(persisted.contains("artifact.created"));
    assert!(persisted.contains("artifact-agent-loop"));
    assert_eq!(agent.history().ensure_all_tool_calls_paired(), Ok(()));
}

#[cfg(unix)]
#[tokio::test]
async fn completed_start_job_receipt_prevents_a_duplicate_background_launch_across_turns() {
    let project = TestProject::new("start-job-dedupe");
    let launch_log = project.root.join("launches.txt");
    let start_arguments = json!({
        "argv": [
            "sh",
            "-c",
            "printf x >> \"$1\"",
            "cyber-agent-job-dedupe",
            launch_log.display().to_string(),
        ]
    });
    let provider = ScriptedProvider::new([
        vec![ModelEvent::ToolCall {
            call: tool_call("start-first", "start_job", start_arguments.clone()),
        }],
        vec![ModelEvent::TextDelta {
            text: "first start recorded".to_owned(),
        }],
        vec![ModelEvent::ToolCall {
            call: tool_call("start-duplicate", "start_job", start_arguments),
        }],
        vec![ModelEvent::TextDelta {
            text: "duplicate start was not relaunched".to_owned(),
        }],
    ]);
    let tools = ToolRegistry::for_project(&project.root);
    let mut agent = AgentLoop::new(&provider, &tools, "fixture");

    let first = agent
        .run_turn("start a background job", AgentCancellation::new())
        .await
        .unwrap();
    let duplicate = agent
        .run_turn("repeat the same background job", AgentCancellation::new())
        .await
        .unwrap();

    let AgentTurnOutcome::Completed {
        executions: first_executions,
        ..
    } = first
    else {
        panic!("the first Job start should complete")
    };
    let AgentTurnOutcome::Completed {
        executions: duplicate_executions,
        ..
    } = duplicate
    else {
        panic!("the duplicate Job start should complete")
    };
    assert_eq!(first_executions.len(), 1);
    assert_eq!(duplicate_executions.len(), 1);
    assert_eq!(
        duplicate_executions[0].receipt.execution_id,
        first_executions[0].receipt.execution_id
    );
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    assert_eq!(fs::read_to_string(launch_log).unwrap(), "x");
}

#[tokio::test]
async fn agent_loop_audits_baseline_candidates_while_sending_complete_history() {
    let provider = ScriptedProvider::new([
        vec![ModelEvent::TextDelta {
            text: "first response".to_owned(),
        }],
        vec![ModelEvent::TextDelta {
            text: "second response".to_owned(),
        }],
    ]);
    let tools = ToolRegistry::for_project(env!("CARGO_MANIFEST_DIR"));
    let mut agent = AgentLoop::with_primary_context(
        &provider,
        &tools,
        "fixture",
        ModelIdentity::new("fixture-primary", "fixture-model-v1"),
        ModelProfile::new("fixture-research", 1),
        ProviderSession::new(
            ProviderSessionId::new("fixture-primary-session"),
            ProviderSessionRole::Primary,
            1,
        ),
    )
    .unwrap();

    agent
        .run_turn("first user input", AgentCancellation::new())
        .await
        .unwrap();
    assert_eq!(
        agent.last_baseline_assessment(),
        Some(&BaselineAssessment::FullRequest {
            reason: BaselineInvalidationReason::NoBaseline,
        })
    );

    agent
        .run_turn("second user input", AgentCancellation::new())
        .await
        .unwrap();
    assert_eq!(
        agent.last_baseline_assessment(),
        Some(&BaselineAssessment::FullRequest {
            reason: BaselineInvalidationReason::IncrementalInputUnsupported,
        })
    );

    let inputs = provider.inputs();
    assert_eq!(inputs.len(), 2);
    assert_eq!(
        inputs[1].content_items,
        vec![
            ContentItem::user_text("first user input"),
            ContentItem::assistant_text("first response"),
            ContentItem::user_text("second user input"),
        ]
    );
}

#[tokio::test]
async fn incremental_delivery_uses_tool_result_then_next_user_as_strict_deltas() {
    let project = TestProject::new("incremental-tool-loop");
    project.write("src/target.rs", "// fixture vulnerability marker\n");
    let call = tool_call(
        "call-search",
        "search_files",
        json!({"pattern": "fixture vulnerability", "paths": ["src"]}),
    );
    let provider = ScriptedProvider::with_capabilities(
        incremental_capabilities(),
        [
            completed_response(
                vec![ModelEvent::ToolCall { call: call.clone() }],
                Some("resp-first"),
            ),
            completed_response(
                vec![ModelEvent::TextDelta {
                    text: "tool result considered".to_owned(),
                }],
                Some("resp-tool"),
            ),
            completed_response(
                vec![ModelEvent::TextDelta {
                    text: "next user considered".to_owned(),
                }],
                Some("resp-next"),
            ),
        ],
    );
    let tools = ToolRegistry::for_project(&project.root);
    let mut agent = AgentLoop::new(&provider, &tools, "fixture");

    let first = agent
        .run_turn("inspect target", AgentCancellation::new())
        .await
        .unwrap();
    assert!(matches!(
        first,
        AgentTurnOutcome::Completed {
            model_requests: 2,
            executions,
            ..
        } if executions.len() == 1
    ));
    let second = agent
        .run_turn("what changed?", AgentCancellation::new())
        .await
        .unwrap();
    assert!(matches!(
        second,
        AgentTurnOutcome::Completed {
            model_requests: 1,
            ..
        }
    ));

    let requests = provider.requests();
    assert_eq!(requests.len(), 3);
    assert!(!requests[0].is_incremental());
    assert!(matches!(
        requests[1].delivery(),
        cyber_agent::ports::model::ModelRequestDelivery::Incremental { continuation }
            if continuation.as_str() == "resp-first"
    ));
    assert!(matches!(
        requests[1].input().content_items.as_slice(),
        [ContentItem::ToolResult { result }]
            if result.call_id == call.call_id && result.status == ToolResultStatus::Succeeded
    ));
    assert!(matches!(
        requests[2].delivery(),
        cyber_agent::ports::model::ModelRequestDelivery::Incremental { continuation }
            if continuation.as_str() == "resp-tool"
    ));
    assert_eq!(
        requests[2].input().content_items,
        vec![ContentItem::user_text("what changed?")]
    );
}

#[tokio::test]
async fn typed_continuation_rejection_retries_one_full_sample_without_replaying_tool() {
    let project = TestProject::new("continuation-fallback");
    project.write("src/target.rs", "// fallback marker\n");
    let call = tool_call(
        "call-search",
        "search_files",
        json!({"pattern": "fallback marker", "paths": ["src"]}),
    );
    let provider = ScriptedProvider::with_capabilities(
        incremental_capabilities(),
        [
            completed_response(
                vec![ModelEvent::ToolCall { call: call.clone() }],
                Some("resp-before-rejection"),
            ),
            rejected_response(vec![ModelEvent::TextDelta {
                text: "discard this partial response".to_owned(),
            }]),
            completed_response(
                vec![ModelEvent::TextDelta {
                    text: "full fallback answer".to_owned(),
                }],
                Some("resp-after-fallback"),
            ),
        ],
    );
    let tools = ToolRegistry::for_project(&project.root);
    let mut agent = AgentLoop::new(&provider, &tools, "fixture");

    let outcome = agent
        .run_turn("inspect target", AgentCancellation::new())
        .await
        .unwrap();
    let AgentTurnOutcome::Completed {
        assistant_text,
        model_requests,
        executions,
    } = outcome
    else {
        panic!("the full fallback should complete the Turn")
    };
    assert_eq!(model_requests, 3, "counts actual Provider requests");
    assert_eq!(executions.len(), 1, "the completed Tool is never replayed");
    assert_eq!(assistant_text, "full fallback answer");
    assert!(
        agent
            .history()
            .items()
            .iter()
            .all(|item| item != &ContentItem::assistant_text("discard this partial response"))
    );

    let requests = provider.requests();
    assert_eq!(requests.len(), 3);
    assert!(requests[1].is_incremental());
    assert!(
        !requests[2].is_incremental(),
        "fallback is a complete request"
    );
    assert!(
        requests[2]
            .input()
            .content_items
            .iter()
            .any(|item| matches!(
                item,
                ContentItem::ToolResult { result } if result.call_id == call.call_id
            ))
    );
}

#[tokio::test]
async fn generic_provider_failure_never_falls_back_or_replays_a_completed_tool() {
    let project = TestProject::new("generic-provider-failure");
    project.write("src/target.rs", "// no fallback marker\n");
    let call = tool_call(
        "call-search",
        "search_files",
        json!({"pattern": "no fallback marker", "paths": ["src"]}),
    );
    let provider = ScriptedProvider::with_capabilities(
        incremental_capabilities(),
        [
            completed_response(
                vec![ModelEvent::ToolCall { call }],
                Some("resp-before-generic-failure"),
            ),
            failed_response(ModelProviderError::ProviderFailure {
                code: Some("rate_limit_exceeded".to_owned()),
                message: "fixture rate limit".to_owned(),
            }),
        ],
    );
    let tools = ToolRegistry::for_project(&project.root);
    let mut agent = AgentLoop::new(&provider, &tools, "fixture");

    assert!(matches!(
        agent
            .run_turn("inspect target", AgentCancellation::new())
            .await,
        Err(AgentLoopError::Model(
            cyber_agent::app::model_streaming::ModelStreamingError::Provider(
                ModelProviderError::ProviderFailure { .. }
            )
        ))
    ));
    assert_eq!(provider.requests().len(), 2, "generic errors do not retry");
    assert_eq!(agent.history().ensure_all_tool_calls_paired(), Ok(()));
}

#[tokio::test]
async fn failed_full_fallback_keeps_a_provider_rejected_reset_until_a_later_full_success() {
    let provider = ScriptedProvider::with_capabilities(
        incremental_capabilities(),
        [
            completed_response(
                vec![ModelEvent::TextDelta {
                    text: "first response".to_owned(),
                }],
                Some("resp-before-rejection"),
            ),
            rejected_response(Vec::new()),
            failed_response(ModelProviderError::TransportFailure {
                message: "fixture fallback transport failure".to_owned(),
            }),
            completed_response(
                vec![ModelEvent::TextDelta {
                    text: "full recovery".to_owned(),
                }],
                Some("resp-after-recovery"),
            ),
        ],
    );
    let tools = ToolRegistry::for_project(env!("CARGO_MANIFEST_DIR"));
    let mut agent = AgentLoop::new(&provider, &tools, "fixture");
    agent
        .run_turn("first input", AgentCancellation::new())
        .await
        .unwrap();
    assert!(matches!(
        agent
            .run_turn("rejected input", AgentCancellation::new())
            .await,
        Err(AgentLoopError::Model(
            cyber_agent::app::model_streaming::ModelStreamingError::Provider(
                ModelProviderError::TransportFailure { .. }
            )
        ))
    ));
    agent
        .run_turn("recovery input", AgentCancellation::new())
        .await
        .unwrap();

    let requests = provider.requests();
    assert_eq!(requests.len(), 4);
    assert!(requests[1].is_incremental());
    assert!(!requests[2].is_incremental(), "first fallback is full");
    assert!(
        !requests[3].is_incremental(),
        "the pending ProviderRejected reset survives a failed full fallback"
    );
}

#[tokio::test]
async fn multiple_tool_calls_commit_one_sample_then_advance_continuation() {
    let provider = ScriptedProvider::with_capabilities(
        incremental_capabilities(),
        [
            completed_response(
                vec![ModelEvent::TextDelta {
                    text: "first response".to_owned(),
                }],
                Some("resp-committed"),
            ),
            completed_response(
                vec![
                    ModelEvent::ToolCall {
                        call: tool_call("call-one", "read_file", json!({"path": "Cargo.toml"})),
                    },
                    ModelEvent::ToolCall {
                        call: tool_call("call-two", "read_file", json!({"path": "src/lib.rs"})),
                    },
                ],
                Some("resp-batch"),
            ),
            completed_response(
                vec![ModelEvent::TextDelta {
                    text: "batch result considered".to_owned(),
                }],
                Some("resp-after-batch"),
            ),
            completed_response(
                vec![ModelEvent::TextDelta {
                    text: "recovery response".to_owned(),
                }],
                Some("resp-recovery"),
            ),
        ],
    );
    let tools = ToolRegistry::for_project(env!("CARGO_MANIFEST_DIR"));
    let mut agent = AgentLoop::new(&provider, &tools, "fixture");

    agent
        .run_turn("first input", AgentCancellation::new())
        .await
        .unwrap();
    let batch = agent
        .run_turn("batch input", AgentCancellation::new())
        .await
        .unwrap();
    assert!(matches!(
        batch,
        AgentTurnOutcome::Completed { executions, .. } if executions.len() == 2
    ));
    assert_eq!(agent.history().ensure_all_tool_calls_paired(), Ok(()));
    agent
        .run_turn("recovery input", AgentCancellation::new())
        .await
        .unwrap();

    let requests = provider.requests();
    assert_eq!(requests.len(), 4);
    assert!(matches!(
        requests[2].delivery(),
        cyber_agent::ports::model::ModelRequestDelivery::Incremental { continuation }
            if continuation.as_str() == "resp-batch"
    ));
    assert!(matches!(
        requests[2].input().content_items.as_slice(),
        [
            ContentItem::ToolResult { result: first },
            ContentItem::ToolResult { result: second },
        ] if first.call_id.as_str() == "call-one" && second.call_id.as_str() == "call-two"
    ));
    assert!(matches!(
        requests[3].delivery(),
        cyber_agent::ports::model::ModelRequestDelivery::Incremental { continuation }
            if continuation.as_str() == "resp-after-batch"
    ));
    assert_eq!(
        requests[3].input().content_items,
        vec![ContentItem::user_text("recovery input")]
    );
}

#[tokio::test]
async fn cancelled_model_sample_keeps_the_last_committed_continuation() {
    let provider = ScriptedProvider::with_capabilities(
        incremental_capabilities(),
        [
            completed_response(
                vec![ModelEvent::TextDelta {
                    text: "first response".to_owned(),
                }],
                Some("resp-before-cancel"),
            ),
            pending_response(),
            completed_response(
                vec![ModelEvent::TextDelta {
                    text: "recovered response".to_owned(),
                }],
                Some("resp-after-cancel"),
            ),
        ],
    );
    let tools = ToolRegistry::for_project(env!("CARGO_MANIFEST_DIR"));
    let mut agent = AgentLoop::new(&provider, &tools, "fixture");
    agent
        .run_turn("first input", AgentCancellation::new())
        .await
        .unwrap();

    let cancellation = AgentCancellation::new();
    let cancel_from_task = cancellation.clone();
    tokio::spawn(async move {
        tokio::task::yield_now().await;
        cancel_from_task.cancel();
    });
    assert!(matches!(
        agent.run_turn("cancelled input", cancellation).await,
        Ok(AgentTurnOutcome::Cancelled {
            model_requests: 1,
            ..
        })
    ));
    agent
        .run_turn("recovery input", AgentCancellation::new())
        .await
        .unwrap();

    let requests = provider.requests();
    assert_eq!(requests.len(), 3);
    assert!(matches!(
        requests[2].delivery(),
        cyber_agent::ports::model::ModelRequestDelivery::Incremental { continuation }
            if continuation.as_str() == "resp-before-cancel"
    ));
    assert_eq!(
        requests[2].input().content_items,
        vec![
            ContentItem::user_text("cancelled input"),
            ContentItem::user_text("recovery input"),
        ]
    );
}

#[tokio::test]
async fn empty_model_sample_keeps_the_last_committed_continuation() {
    let provider = ScriptedProvider::with_capabilities(
        incremental_capabilities(),
        [
            completed_response(
                vec![ModelEvent::TextDelta {
                    text: "first response".to_owned(),
                }],
                Some("resp-before-empty"),
            ),
            completed_response(Vec::new(), Some("resp-empty")),
            completed_response(
                vec![ModelEvent::TextDelta {
                    text: "recovered response".to_owned(),
                }],
                Some("resp-after-empty"),
            ),
        ],
    );
    let tools = ToolRegistry::for_project(env!("CARGO_MANIFEST_DIR"));
    let mut agent = AgentLoop::new(&provider, &tools, "fixture");
    agent
        .run_turn("first input", AgentCancellation::new())
        .await
        .unwrap();
    assert_eq!(
        agent
            .run_turn("empty input", AgentCancellation::new())
            .await,
        Err(AgentLoopError::EmptyModelResponse)
    );
    agent
        .run_turn("recovery input", AgentCancellation::new())
        .await
        .unwrap();

    let requests = provider.requests();
    assert!(matches!(
        requests[2].delivery(),
        cyber_agent::ports::model::ModelRequestDelivery::Incremental { continuation }
            if continuation.as_str() == "resp-before-empty"
    ));
}

#[tokio::test]
async fn fresh_provider_sessions_never_share_a_continuation_identity() {
    let provider = ScriptedProvider::with_capabilities(
        incremental_capabilities(),
        [
            completed_response(
                vec![ModelEvent::TextDelta {
                    text: "primary first".to_owned(),
                }],
                Some("resp-primary"),
            ),
            completed_response(
                vec![ModelEvent::TextDelta {
                    text: "fresh session first".to_owned(),
                }],
                Some("resp-fresh"),
            ),
            completed_response(
                vec![ModelEvent::TextDelta {
                    text: "primary second".to_owned(),
                }],
                Some("resp-primary-next"),
            ),
        ],
    );
    let tools = ToolRegistry::for_project(env!("CARGO_MANIFEST_DIR"));
    let mut primary = AgentLoop::with_primary_context(
        &provider,
        &tools,
        "fixture",
        ModelIdentity::new("fixture", "model"),
        ModelProfile::new("profile", 1),
        ProviderSession::new(
            ProviderSessionId::new("primary-session"),
            ProviderSessionRole::Primary,
            1,
        ),
    )
    .unwrap();
    let mut fresh = AgentLoop::with_primary_context(
        &provider,
        &tools,
        "fixture",
        ModelIdentity::new("fixture", "model"),
        ModelProfile::new("profile", 1),
        ProviderSession::new(
            ProviderSessionId::new("resumed-session"),
            ProviderSessionRole::Primary,
            2,
        ),
    )
    .unwrap();

    primary
        .run_turn("primary first", AgentCancellation::new())
        .await
        .unwrap();
    fresh
        .run_turn("fresh first", AgentCancellation::new())
        .await
        .unwrap();
    primary
        .run_turn("primary second", AgentCancellation::new())
        .await
        .unwrap();

    let requests = provider.requests();
    assert_eq!(requests.len(), 3);
    assert!(!requests[0].is_incremental());
    assert!(
        !requests[1].is_incremental(),
        "resume starts from a full request"
    );
    assert!(matches!(
        requests[2].delivery(),
        cyber_agent::ports::model::ModelRequestDelivery::Incremental { continuation }
            if continuation.as_str() == "resp-primary"
    ));
}

struct PendingToolExecutor {
    spec: ToolSpec,
}

impl ToolExecutor for PendingToolExecutor {
    fn specs(&self) -> Vec<ToolSpec> {
        vec![self.spec.clone()]
    }

    fn plan(
        &self,
        execution_id: ExecutionId,
        call: ToolCall,
    ) -> Result<ToolExecutionPlan, ToolExecutorError> {
        ToolExecutionPlan::new(execution_id, call, &self.spec, Vec::new()).map_err(|error| {
            ToolExecutorError::PlanFailure {
                tool_name: self.spec.name.to_string(),
                message: error.to_string(),
            }
        })
    }

    fn execute(&self, _plan: ToolExecutionPlan) -> ToolExecutionFuture<'_> {
        Box::pin(std::future::pending::<ToolExecutionOutcome>())
    }
}

#[tokio::test]
async fn cancellation_during_tool_execution_records_a_cancelled_pair() {
    let registry = ToolRegistry::for_project(env!("CARGO_MANIFEST_DIR"));
    let spec = registry
        .specs()
        .into_iter()
        .find(|spec| spec.name.as_str() == "read_file")
        .unwrap();
    let tools = PendingToolExecutor { spec };
    let provider = ScriptedProvider::new([vec![ModelEvent::ToolCall {
        call: tool_call("call-pending", "read_file", json!({"path": "Cargo.toml"})),
    }]]);
    let cancellation = AgentCancellation::new();
    let cancel_from_task = cancellation.clone();
    tokio::spawn(async move {
        tokio::task::yield_now().await;
        cancel_from_task.cancel();
    });
    let mut agent = AgentLoop::new(&provider, &tools, "fixture");

    let outcome = agent
        .run_turn("read the fixture", cancellation)
        .await
        .unwrap();
    let AgentTurnOutcome::Cancelled {
        model_requests,
        executions,
    } = outcome
    else {
        panic!("the pending Tool should be cancelled")
    };
    assert_eq!(model_requests, 1);
    assert_eq!(executions.len(), 1);
    assert_eq!(executions[0].result.status, ToolResultStatus::Cancelled);
    assert_eq!(agent.history().ensure_all_tool_calls_paired(), Ok(()));
}

#[tokio::test]
async fn multiple_tool_calls_are_staged_and_paired_in_model_order() {
    let provider = ScriptedProvider::new([
        vec![
            ModelEvent::TextDelta {
                text: "I will inspect two paths.".to_owned(),
            },
            ModelEvent::ToolCall {
                call: tool_call("call-1", "read_file", json!({"path": "Cargo.toml"})),
            },
            ModelEvent::ToolCall {
                call: tool_call("call-2", "read_file", json!({"path": "src/lib.rs"})),
            },
        ],
        vec![ModelEvent::TextDelta {
            text: "Both paths were inspected.".to_owned(),
        }],
    ]);
    let tools = ToolRegistry::for_project(env!("CARGO_MANIFEST_DIR"));
    let mut agent = AgentLoop::new(&provider, &tools, "fixture");

    let outcome = agent
        .run_turn("inspect", AgentCancellation::new())
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        AgentTurnOutcome::Completed { executions, .. } if executions.len() == 2
    ));
    assert!(matches!(
        agent.history().items(),
        [
            ContentItem::Text { .. },
            ContentItem::Text { .. },
            ContentItem::ToolCall { call: first },
            ContentItem::ToolCall { call: second },
            ContentItem::ToolResult { result: first_result },
            ContentItem::ToolResult { result: second_result },
            ContentItem::Text { .. },
        ] if first.call_id.as_str() == "call-1"
            && second.call_id.as_str() == "call-2"
            && first_result.call_id == first.call_id
            && second_result.call_id == second.call_id
    ));
    assert_eq!(agent.history().ensure_all_tool_calls_paired(), Ok(()));
}
