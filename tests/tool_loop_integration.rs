use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use cyber_agent::adapters::tools::ToolRegistry;
use cyber_agent::app::agent_loop::{AgentLoop, AgentLoopError, AgentTurnOutcome};
use cyber_agent::app::cancellation::AgentCancellation;
use cyber_agent::domain::context::{
    BaselineAssessment, BaselineInvalidationReason, ModelIdentity, ModelProfile, ProviderSession,
    ProviderSessionId, ProviderSessionRole,
};
use cyber_agent::domain::model::{
    CapabilityMappingVersion, ContentItem, ModelCapabilities, ModelEvent, ModelInputIR,
};
use cyber_agent::domain::tools::{
    ExecutionId, ToolCall, ToolCallId, ToolExecutionOutcome, ToolExecutionPlan, ToolName,
    ToolResultStatus, ToolSpec,
};
use cyber_agent::ports::model::{ModelEventStream, ModelProvider, ModelProviderError};
use cyber_agent::ports::tool::{ToolExecutionFuture, ToolExecutor, ToolExecutorError};
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
        vec!["list_files", "read_file", "run_command", "search_files"]
    );
    let run_spec = specs
        .iter()
        .find(|spec| spec.name.as_str() == "run_command")
        .unwrap();
    assert_eq!(
        (run_spec.effect, run_spec.retry_policy, run_spec.determinism),
        (
            cyber_agent::domain::tools::ToolEffect::Interactive,
            cyber_agent::domain::tools::ToolRetryPolicy::NeverAutoRetry,
            cyber_agent::domain::tools::ToolDeterminism::Nondeterministic,
        )
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
    assert!(
        read.result.output["content"]
            .as_str()
            .unwrap()
            .contains("path traversal")
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

struct ScriptedProvider {
    responses: Mutex<VecDeque<Vec<ModelEvent>>>,
    inputs: Mutex<Vec<ModelInputIR>>,
}

impl ScriptedProvider {
    fn new(responses: impl IntoIterator<Item = Vec<ModelEvent>>) -> Self {
        Self {
            responses: Mutex::new(responses.into_iter().collect()),
            inputs: Mutex::new(Vec::new()),
        }
    }

    fn inputs(&self) -> Vec<ModelInputIR> {
        self.inputs.lock().unwrap().clone()
    }
}

impl ModelProvider for ScriptedProvider {
    fn capabilities(&self) -> ModelCapabilities {
        capabilities()
    }

    fn stream(&self, input: ModelInputIR) -> Result<ModelEventStream, ModelProviderError> {
        self.capabilities().validate_input(&input)?;
        self.inputs.lock().unwrap().push(input);
        let response = self.responses.lock().unwrap().pop_front().ok_or_else(|| {
            ModelProviderError::ProviderFailure {
                code: Some("fixture_exhausted".to_owned()),
                message: "the scripted Provider has no response left".to_owned(),
            }
        })?;
        Ok(Box::pin(stream::iter(response.into_iter().map(Ok))))
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
            ModelEvent::Completed,
        ],
        vec![
            ModelEvent::TextDelta {
                text: "Candidate identified: path traversal marker in src/parser.rs.".to_owned(),
            },
            ModelEvent::Completed,
        ],
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
    assert_eq!(inputs[0].tool_specs.len(), 4);
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
async fn agent_loop_audits_baseline_candidates_while_sending_complete_history() {
    let provider = ScriptedProvider::new([
        vec![
            ModelEvent::TextDelta {
                text: "first response".to_owned(),
            },
            ModelEvent::Completed,
        ],
        vec![
            ModelEvent::TextDelta {
                text: "second response".to_owned(),
            },
            ModelEvent::Completed,
        ],
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
        Some(&BaselineAssessment::AppendOnlyCandidate {
            delta_items: vec![ContentItem::user_text("second user input")],
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
    let provider = ScriptedProvider::new([vec![
        ModelEvent::ToolCall {
            call: tool_call("call-pending", "read_file", json!({"path": "Cargo.toml"})),
        },
        ModelEvent::Completed,
    ]]);
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
async fn multiple_tool_calls_are_rejected_without_committing_partial_model_output() {
    let provider = ScriptedProvider::new([vec![
        ModelEvent::TextDelta {
            text: "I will inspect two paths.".to_owned(),
        },
        ModelEvent::ToolCall {
            call: tool_call("call-1", "read_file", json!({"path": "Cargo.toml"})),
        },
        ModelEvent::ToolCall {
            call: tool_call("call-2", "read_file", json!({"path": "src/lib.rs"})),
        },
        ModelEvent::Completed,
    ]]);
    let tools = ToolRegistry::for_project(env!("CARGO_MANIFEST_DIR"));
    let mut agent = AgentLoop::new(&provider, &tools, "fixture");

    assert_eq!(
        agent.run_turn("inspect", AgentCancellation::new()).await,
        Err(AgentLoopError::MultipleToolCalls { count: 2 })
    );
    assert_eq!(
        agent.history().items(),
        &[ContentItem::user_text("inspect")]
    );
    assert_eq!(agent.history().ensure_all_tool_calls_paired(), Ok(()));
}
