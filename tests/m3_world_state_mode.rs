#[path = "m3_world_state_mode/session_compaction.rs"]
mod session_compaction;
mod support;

use std::collections::VecDeque;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use cyber_agent::adapters::fragment_cache::LocalFragmentCache;
use cyber_agent::adapters::mock_model::{MockModelFixture, MockModelProvider};
use cyber_agent::adapters::project_event_store::Project;
use cyber_agent::adapters::tools::ToolRegistry;
use cyber_agent::app::agent_loop::{AgentLoop, AgentTurnOutcome};
use cyber_agent::app::cancellation::AgentCancellation;
use cyber_agent::app::doctor::{AgentProfileDoctorStatus, inspect_agent_model_profile};
use cyber_agent::domain::context::{
    BaselineAssessment, BaselineInvalidationReason, ContextBudget, ContextFragment,
    ContextFragmentContent, ContextFragmentId, ContextFragmentPriority, ContextFragmentSource,
    ContextPlanner, FragmentCachePolicy, InternalRequestMetadata, ModelIdentity, ModelProfile,
    ProviderSession, ProviderSessionId, ProviderSessionRef, ProviderSessionRole, StepSnapshot,
    TurnContext, TurnId,
};
use cyber_agent::domain::events::{EventId, EventSequence, OccurredAt, StreamId};
use cyber_agent::domain::mode::{ModePack, PromptComposition};
use cyber_agent::domain::model::{
    CapabilityMappingVersion, ContentItem, ModelCapabilities, ModelEvent, ModelInputIR,
    ModelRequestRequirements,
};
use cyber_agent::domain::profile::{
    AgentModelProfile, AgentModelProfileError, ProviderProfile, VisionMode, VisionRoute,
    VisionRouteReason, resolve_vision_route,
};
use cyber_agent::domain::project::ProjectId;
use cyber_agent::domain::session::{SessionEvent, SessionReducer, SessionState};
use cyber_agent::domain::snapshot::{ProjectFingerprint, ProjectFingerprintKind, ProjectSnapshot};
use cyber_agent::domain::tools::{
    ExecutionId, ToolCall, ToolCallId, ToolExecutionOutcome, ToolExecutionPlan, ToolExposure,
    ToolExposurePlan, ToolName, ToolSpec,
};
use cyber_agent::domain::world_state::{
    ProjectSnapshotContext, ResearchWorldState, ResearchWorldStateSection,
};
use cyber_agent::ports::model::{
    ModelEventStream, ModelProvider, ModelProviderError, ModelRequest, ModelResponseCompletion,
    ModelStreamEvent, ProviderResponseId,
};
use cyber_agent::ports::tool::{ToolExecutionFuture, ToolExecutor, ToolExecutorError};
use futures_util::stream;
use serde_json::json;
use support::TemporaryDirectory;

fn capabilities(image_input: bool) -> ModelCapabilities {
    ModelCapabilities {
        text_input: true,
        image_input,
        streaming: true,
        tool_call: true,
        structured_output: true,
        incremental_input: false,
        usage_reporting: true,
        context_window: 32_768,
        capability_mapping_version: CapabilityMappingVersion::new(7),
    }
}

fn resumed_session_state(primary_id: &str, generation: u64) -> SessionState {
    SessionReducer
        .reduce([
            SessionEvent::Started {
                primary_provider_session: ProviderSessionRef {
                    id: ProviderSessionId::new("audit-start"),
                    role: ProviderSessionRole::Primary,
                    generation: 1,
                },
            }
            .into_envelope(
                EventId::new("session-started"),
                StreamId::new("session:resume-fixture"),
                EventSequence::new(1),
                OccurredAt::new("2026-08-23T00:00:00Z"),
            )
            .unwrap(),
            SessionEvent::Resumed {
                primary_provider_session: ProviderSessionRef {
                    id: ProviderSessionId::new(primary_id),
                    role: ProviderSessionRole::Primary,
                    generation,
                },
            }
            .into_envelope(
                EventId::new("session-resumed"),
                StreamId::new("session:resume-fixture"),
                EventSequence::new(2),
                OccurredAt::new("2026-08-23T00:00:01Z"),
            )
            .unwrap(),
        ])
        .unwrap()
}

fn provider_profile(id: &str, generation: u64, image_input: bool) -> ProviderProfile {
    ProviderProfile::new(
        ModelIdentity::new("fixture-provider", format!("model-{id}")),
        ModelProfile::new(id, generation),
        capabilities(image_input),
    )
}

fn agent_profile(
    id: &str,
    generation: u64,
    primary_image: bool,
    vision: Option<ProviderProfile>,
    mode: VisionMode,
) -> AgentModelProfile {
    AgentModelProfile::new(
        ModelProfile::new(id, generation),
        provider_profile("primary", generation, primary_image),
        vision,
        mode,
    )
    .unwrap()
}

fn incremental_agent_profile(
    id: &str,
    generation: u64,
    vision: Option<ProviderProfile>,
    mode: VisionMode,
) -> AgentModelProfile {
    AgentModelProfile::new(
        ModelProfile::new(id, generation),
        ProviderProfile::new(
            ModelIdentity::new("fixture-provider", "model-primary"),
            ModelProfile::new("primary", 1),
            incremental_capabilities(),
        ),
        vision,
        mode,
    )
    .unwrap()
}

fn context_metrics(composition: &PromptComposition, tool_specs: Vec<ToolSpec>) -> (String, u32) {
    let mut fragments = composition.fragments().unwrap();
    fragments.push(
        ContextFragment::new(
            ContextFragmentId::new("input:test").unwrap(),
            ContextFragmentSource::UserInput,
            ContextFragmentPriority::Required,
            FragmentCachePolicy::DoNotCache,
            ContextFragmentContent::ContentItems {
                items: vec![ContentItem::user_text("test")],
            },
        )
        .unwrap(),
    );
    let plan = ContextPlanner::current()
        .compile(
            fragments,
            ContextBudget::new(32_768, 0, 0).unwrap(),
            &capabilities(false),
            tool_specs,
            ModelRequestRequirements::default(),
            None,
        )
        .unwrap();
    (
        plan.context_fingerprint().to_owned(),
        plan.context_decision_manifest().fixed_input_tokens(),
    )
}

struct ScriptedProvider {
    capabilities: ModelCapabilities,
    responses: Mutex<VecDeque<Vec<Result<ModelStreamEvent, ModelProviderError>>>>,
    requests: Mutex<Vec<ModelRequest>>,
}

impl ScriptedProvider {
    fn new(
        capabilities: ModelCapabilities,
        responses: impl IntoIterator<Item = Vec<Result<ModelStreamEvent, ModelProviderError>>>,
    ) -> Self {
        Self {
            capabilities,
            responses: Mutex::new(responses.into_iter().collect()),
            requests: Mutex::new(Vec::new()),
        }
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
        request.validate_capabilities(&self.capabilities)?;
        self.requests.lock().unwrap().push(request);
        let response = self.responses.lock().unwrap().pop_front().ok_or_else(|| {
            ModelProviderError::ProviderFailure {
                code: Some("fixture_exhausted".to_owned()),
                message: "the scripted Provider has no response left".to_owned(),
            }
        })?;
        Ok(Box::pin(stream::iter(response)))
    }
}

fn completed_response(
    text: &str,
    response_id: &str,
) -> Vec<Result<ModelStreamEvent, ModelProviderError>> {
    vec![
        Ok(ModelStreamEvent::Event(ModelEvent::TextDelta {
            text: text.to_owned(),
        })),
        Ok(ModelStreamEvent::Completed(ModelResponseCompletion::new(
            ProviderResponseId::new(response_id),
        ))),
    ]
}

fn completed_tool_call_response(
    call: ToolCall,
    response_id: &str,
) -> Vec<Result<ModelStreamEvent, ModelProviderError>> {
    vec![
        Ok(ModelStreamEvent::Event(ModelEvent::ToolCall { call })),
        Ok(ModelStreamEvent::Completed(ModelResponseCompletion::new(
            ProviderResponseId::new(response_id),
        ))),
    ]
}

fn rejected_response() -> Vec<Result<ModelStreamEvent, ModelProviderError>> {
    vec![Err(ModelProviderError::ContinuationRejected {
        code: "fixture_rejected".to_owned(),
    })]
}

fn failed_response(message: &str) -> Vec<Result<ModelStreamEvent, ModelProviderError>> {
    vec![Err(ModelProviderError::TransportFailure {
        message: message.to_owned(),
    })]
}

fn incremental_capabilities() -> ModelCapabilities {
    ModelCapabilities {
        incremental_input: true,
        ..capabilities(false)
    }
}

fn world_state(version: &str) -> ResearchWorldState {
    ResearchWorldState::from_sections([(
        ResearchWorldStateSection::Target,
        json!({"target": "fixture", "version": version}),
    )])
    .unwrap()
}

fn bounded_large_world_state(version: &str) -> ResearchWorldState {
    ResearchWorldState::from_sections(ResearchWorldStateSection::ALL.into_iter().map(|section| {
        (
            section,
            json!({"version": version, "notes": "x".repeat(4_096)}),
        )
    }))
    .unwrap()
}

fn diff_heavy_world_state(version: usize) -> ResearchWorldState {
    ResearchWorldState::from_sections([(
        ResearchWorldStateSection::Target,
        json!({
            "target": "fixture",
            "notes": format!("version-{version}-{}", "x".repeat(1_024)),
        }),
    )])
    .unwrap()
}

fn project_snapshot_context(
    file_count: usize,
    created_at: &str,
    rebuild_reason: &str,
) -> ProjectSnapshotContext {
    ProjectSnapshotContext::new(ProjectSnapshot {
        cache_schema_version: 1,
        project_id: ProjectId::new("project-agent-loop-diff"),
        source_fingerprint: ProjectFingerprint {
            kind: ProjectFingerprintKind::Filesystem,
            digest: "project-agent-loop-diff".to_owned(),
            cacheable: true,
        },
        created_at: created_at.to_owned(),
        rebuild_reason: rebuild_reason.to_owned(),
        file_manifest_hash: "files-agent-loop-diff".to_owned(),
        tool_capability_hash: "tools-agent-loop-diff".to_owned(),
        file_count,
    })
    .unwrap()
}

struct PermissiveExecutor {
    visible_spec: ToolSpec,
    executions: Arc<AtomicUsize>,
}

impl ToolExecutor for PermissiveExecutor {
    fn specs(&self) -> Vec<ToolSpec> {
        vec![self.visible_spec.clone()]
    }

    fn plan(
        &self,
        execution_id: ExecutionId,
        call: ToolCall,
    ) -> Result<ToolExecutionPlan, ToolExecutorError> {
        ToolExecutionPlan::new(execution_id, call, &self.visible_spec, Vec::new()).map_err(
            |error| ToolExecutorError::PlanFailure {
                tool_name: self.visible_spec.name.to_string(),
                message: error.to_string(),
            },
        )
    }

    fn execute(&self, plan: ToolExecutionPlan) -> ToolExecutionFuture<'_> {
        let executions = self.executions.clone();
        Box::pin(async move {
            executions.fetch_add(1, Ordering::SeqCst);
            ToolExecutionOutcome::succeeded(plan, json!({"must_not_run": true}))
        })
    }
}

#[path = "m3_world_state_mode/agent_loop.rs"]
mod agent_loop;
#[path = "m3_world_state_mode/domain.rs"]
mod domain;
#[path = "m3_world_state_mode/profile_exposure.rs"]
mod profile_exposure;
