//! Minimal FullAccess Tool Registry for the M1a single-Agent loop.

mod active;
mod artifact;
mod command;
mod context_read;
mod environment;
mod file_write;
mod filesystem;
mod http_request;
mod job;
mod mcp;
mod memory;
mod operation_time;
mod output;
mod process_identity;
mod pty;
mod read_output;
mod research;
mod resources;
mod search;
mod snapshot;
mod tool_search;
mod view_image;

pub(crate) use command::read_attested_input;
pub(crate) use view_image::ViewImageToolInput;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::adapters::mcp::{McpCatalogTool, McpError};
use crate::adapters::project_event_store::Project;
use crate::domain::context::ContextWindowPolicy;
use crate::domain::events::StreamId;
use crate::domain::memory::MemoryScope;
use crate::domain::tools::{
    ExecutionId, ResolvedToolPlan, ResourceKey, ToolCachePolicy, ToolCall, ToolDeterminism,
    ToolEffect, ToolExecutionOutcome, ToolExecutionPlan, ToolExposure, ToolName, ToolOutputPolicy,
    ToolParallelism, ToolRetryPolicy, ToolSpec, split_execution_control,
};
use crate::ports::tool::{ToolExecutionFuture, ToolExecutor, ToolExecutorError};

use active::{ActiveProcessId, ActiveProcessLeases};

#[derive(Clone, Copy, Eq, PartialEq)]
enum BuiltinTool {
    Filesystem(filesystem::Tool),
    FileWrite(file_write::Tool),
    Command(command::Tool),
    Job(job::Tool),
    Pty(pty::Tool),
    Memory(memory::Tool),
    Research(research::Tool),
    SearchFiles,
    ReadOutput,
    ContextRead,
    ArtifactStore,
    ViewImage,
    HttpRequest,
    InspectEnvironment,
    InspectProject,
    ToolSearch,
}

enum RegisteredTool {
    Builtin(BuiltinTool),
    Mcp {
        tool: McpCatalogTool,
        spec: Box<ToolSpec>,
    },
}

pub struct ToolRegistry {
    project_root: PathBuf,
    control_directory_name: &'static str,
    tools: BTreeMap<ToolName, RegisteredTool>,
    jobs: job::BackgroundJobs,
    ptys: pty::PtySessions,
    active_leases: ActiveProcessLeases,
    artifact_scope: artifact::ArtifactScopeBinding,
    memory_scope: Option<MemoryScope>,
    context_reader: Option<Arc<dyn crate::ports::context_read::ContextReadPort>>,
    input_verification_enabled: bool,
    automatic_verification_enabled: bool,
    window_policy: ContextWindowPolicy,
}

impl ToolRegistry {
    pub fn for_project(project_root: impl Into<PathBuf>) -> Self {
        Self::with_bindings(
            project_root,
            artifact::ArtifactScopeBinding::TargetFromArguments,
            Some(MemoryScope::Project),
            true,
        )
    }

    pub fn for_agent(
        project_root: impl Into<PathBuf>,
        session_id: StreamId,
        memory_scope: Option<MemoryScope>,
        research_enabled: bool,
    ) -> Self {
        let artifact_scope = if research_enabled {
            artifact::ArtifactScopeBinding::TargetFromArguments
        } else {
            artifact::ArtifactScopeBinding::Session(session_id)
        };
        Self::with_bindings(project_root, artifact_scope, memory_scope, research_enabled)
    }

    pub(crate) fn for_project_with_mcp(
        project_root: impl Into<PathBuf>,
        discovered: &[McpCatalogTool],
    ) -> Result<Self, McpError> {
        let mut registry = Self::for_project(project_root);
        registry.register_mcp(discovered)?;
        Ok(registry)
    }

    pub(crate) fn for_agent_with_mcp(
        project_root: impl Into<PathBuf>,
        session_id: StreamId,
        memory_scope: Option<MemoryScope>,
        research_enabled: bool,
        discovered: &[McpCatalogTool],
    ) -> Result<Self, McpError> {
        let mut registry =
            Self::for_agent(project_root, session_id, memory_scope, research_enabled);
        registry.register_mcp(discovered)?;
        Ok(registry)
    }

    fn with_bindings(
        project_root: impl Into<PathBuf>,
        artifact_scope: artifact::ArtifactScopeBinding,
        memory_scope: Option<MemoryScope>,
        research_enabled: bool,
    ) -> Self {
        let project_root = project_root.into();
        let builtins = filesystem::Tool::ALL
            .map(BuiltinTool::Filesystem)
            .into_iter()
            .chain(file_write::Tool::ALL.map(BuiltinTool::FileWrite))
            .chain(command::Tool::ALL.map(BuiltinTool::Command))
            .chain(job::Tool::ALL.map(BuiltinTool::Job))
            .chain(pty::Tool::ALL.map(BuiltinTool::Pty))
            .chain([
                BuiltinTool::SearchFiles,
                BuiltinTool::ReadOutput,
                BuiltinTool::ContextRead,
                BuiltinTool::ArtifactStore,
                BuiltinTool::ViewImage,
                BuiltinTool::HttpRequest,
                BuiltinTool::InspectEnvironment,
                BuiltinTool::InspectProject,
                BuiltinTool::ToolSearch,
            ])
            .chain(
                memory_scope
                    .as_ref()
                    .into_iter()
                    .flat_map(|_| memory::Tool::ALL.map(BuiltinTool::Memory)),
            )
            .chain(
                research_enabled
                    .then_some(research::Tool::ALL)
                    .into_iter()
                    .flatten()
                    .map(BuiltinTool::Research),
            );
        let tools = builtins
            .map(|tool| (ToolName::new(tool.name()), RegisteredTool::Builtin(tool)))
            .collect();
        Self {
            project_root,
            control_directory_name: Project::DEFAULT_CONTROL_DIRECTORY,
            tools,
            jobs: job::BackgroundJobs::new(),
            ptys: pty::PtySessions::new(),
            active_leases: ActiveProcessLeases::new(),
            artifact_scope,
            memory_scope,
            context_reader: None,
            input_verification_enabled: false,
            automatic_verification_enabled: false,
            window_policy: ContextWindowPolicy::for_window(32768),
        }
    }

    pub(crate) fn with_input_verification(mut self, enabled: bool) -> Self {
        self.input_verification_enabled = enabled;
        self
    }

    pub(crate) fn with_context_reader(
        mut self,
        reader: Arc<dyn crate::ports::context_read::ContextReadPort>,
    ) -> Self {
        self.context_reader = Some(reader);
        self
    }

    pub(crate) fn with_automatic_verification(mut self, enabled: bool) -> Self {
        self.automatic_verification_enabled = enabled;
        self
    }

    /// Initialized runtime callers pass the Project's single selected layout.
    /// Root-only construction remains available for ordinary file tools.
    pub(crate) fn with_project(mut self, project: &Project) -> Self {
        self.project_root = project.root().to_path_buf();
        self.control_directory_name = project.control_directory_name();
        self
    }

    pub(crate) fn with_window_policy(mut self, policy: ContextWindowPolicy) -> Self {
        self.window_policy = policy;
        self
    }

    pub(crate) fn window_policy(&self) -> ContextWindowPolicy {
        self.window_policy
    }

    fn register_mcp(&mut self, discovered: &[McpCatalogTool]) -> Result<(), McpError> {
        for tool in discovered {
            let spec = mcp::spec(tool)?;
            let name = spec.name.clone();
            if self
                .tools
                .insert(
                    name.clone(),
                    RegisteredTool::Mcp {
                        tool: tool.clone(),
                        spec: Box::new(spec),
                    },
                )
                .is_some()
            {
                return Err(McpError::Catalog {
                    message: format!("MCP tool local name `{name}` collides with an existing tool"),
                });
            }
        }
        Ok(())
    }

    fn canonical_spec(&self, name: &ToolName, tool: &RegisteredTool) -> ToolSpec {
        if let RegisteredTool::Mcp { spec, .. } = tool {
            debug_assert_eq!(&spec.name, name);
            return spec.as_ref().clone();
        }
        let RegisteredTool::Builtin(tool) = tool else {
            unreachable!("registered tool is builtin or MCP")
        };
        let tool = *tool;
        let mut builtin_spec = tool.spec(self);
        if let Some(template) = &mut builtin_spec.resource_key_template {
            *template = template.replace(
                Project::DEFAULT_CONTROL_DIRECTORY,
                self.control_directory_name,
            );
        }
        builtin_spec.output_policy.max_bytes = self.window_policy.tool_output_bytes();
        debug_assert_eq!(&builtin_spec.name, name);
        builtin_spec
    }

    fn canonical_specs(&self) -> Vec<ToolSpec> {
        self.tools
            .iter()
            .map(|(name, tool)| self.canonical_spec(name, tool))
            .collect()
    }

    pub(crate) fn discovered_mcp_specs(&self) -> Vec<ToolSpec> {
        self.tools
            .values()
            .filter_map(|tool| match tool {
                RegisteredTool::Mcp { spec, .. } => Some(spec.as_ref().clone()),
                RegisteredTool::Builtin(_) => None,
            })
            .collect()
    }

    fn plan_with_spec(
        &self,
        execution_id: ExecutionId,
        mut call: ToolCall,
        spec: &ToolSpec,
        resolved: Option<&ResolvedToolPlan>,
    ) -> Result<ToolExecutionPlan, ToolExecutorError> {
        let tool = self
            .tools
            .get(&call.name)
            .ok_or_else(|| ToolExecutorError::UnknownTool {
                tool_name: call.name.to_string(),
            })?;
        if let RegisteredTool::Mcp { tool, .. } = tool {
            return mcp::plan(execution_id, call, tool, spec, resolved);
        }
        let RegisteredTool::Builtin(tool) = tool else {
            unreachable!("registered tool is builtin or MCP")
        };
        let arguments = std::mem::take(&mut call.arguments);
        let (arguments, rerun_reason) = split_execution_control(arguments).map_err(|error| {
            ToolExecutorError::InvalidArguments {
                tool_name: call.name.to_string(),
                message: error.to_string(),
            }
        })?;
        let (normalized_arguments, resource_keys) = tool.normalize(self, arguments)?;
        call.arguments = normalized_arguments;
        let mut plan = ToolExecutionPlan::new_with_rerun_reason(
            execution_id,
            call,
            spec,
            resource_keys,
            rerun_reason,
        )
        .map_err(|error| ToolExecutorError::PlanFailure {
            tool_name: spec.name.to_string(),
            message: error.to_string(),
        })?;
        if let Some(resolved) = resolved {
            plan.bind_resolved_tool_plan(resolved).map_err(|error| {
                ToolExecutorError::PlanFailure {
                    tool_name: spec.name.to_string(),
                    message: error.to_string(),
                }
            })?;
        }
        Ok(plan)
    }
}

impl ToolExecutor for ToolRegistry {
    fn live_processes(&self) -> Option<Vec<crate::domain::tools::LiveProcessObservation>> {
        use crate::domain::tools::LiveProcessObservation;
        let mut observed = self
            .jobs
            .live_processes()
            .into_iter()
            .map(|(job_id, status)| {
                let resources = self
                    .active_leases
                    .resources_for(&ActiveProcessId::Job(job_id.clone()));
                LiveProcessObservation::Job {
                    job_id,
                    status,
                    resources,
                }
            })
            .collect::<Vec<_>>();
        observed.extend(
            self.ptys
                .live_processes()
                .into_iter()
                .map(|(pty_session_id, status)| {
                    let resources = self
                        .active_leases
                        .resources_for(&ActiveProcessId::Pty(pty_session_id.clone()));
                    LiveProcessObservation::Pty {
                        pty_session_id,
                        status,
                        resources,
                    }
                }),
        );
        Some(observed)
    }
    fn catalog(&self) -> Vec<ToolSpec> {
        self.canonical_specs()
    }

    fn resolve_tool_plan(
        &self,
        exposure: &crate::domain::tools::ToolExposurePlan,
        activations: &crate::domain::tools::DeferredActivationSet,
    ) -> Result<ResolvedToolPlan, ToolExecutorError> {
        let unavailable =
            crate::domain::tools::ToolExposurePlan::new(if self.context_reader.is_none() {
                BTreeMap::from([(ToolName::new(context_read::NAME), ToolExposure::Hidden)])
            } else {
                BTreeMap::new()
            });
        let plan = crate::domain::tools::ToolExposurePlan::layered([exposure.clone(), unavailable]);
        ResolvedToolPlan::resolve(self.catalog(), &plan, activations).map_err(|error| match error {
            crate::domain::tools::ToolContractError::UnknownToolExposure { tool_name } => {
                ToolExecutorError::UnknownTool { tool_name }
            }
            error => ToolExecutorError::ResolvedPlanFailure {
                message: error.to_string(),
            },
        })
    }

    fn plan(
        &self,
        execution_id: ExecutionId,
        call: ToolCall,
    ) -> Result<ToolExecutionPlan, ToolExecutorError> {
        let tool = self
            .tools
            .get(&call.name)
            .ok_or_else(|| ToolExecutorError::UnknownTool {
                tool_name: call.name.to_string(),
            })?;
        let spec = self.canonical_spec(&call.name, tool);
        self.plan_with_spec(execution_id, call, &spec, None)
    }

    fn plan_model_call(
        &self,
        resolved: &ResolvedToolPlan,
        execution_id: ExecutionId,
        call: ToolCall,
    ) -> Result<ToolExecutionPlan, ToolExecutorError> {
        let spec = resolved.advertised_spec(&call.name).ok_or_else(|| {
            ToolExecutorError::ToolNotExposed {
                tool_name: call.name.to_string(),
            }
        })?;
        self.plan_with_spec(execution_id, call, spec, Some(resolved))
    }

    fn execute(&self, plan: ToolExecutionPlan) -> ToolExecutionFuture<'_> {
        self.execute_with_observer(plan, None, crate::ports::tool::ToolCancellation::never())
    }

    fn execute_observed(
        &self,
        plan: ToolExecutionPlan,
        observer: crate::ports::tool::ToolObservationSender,
    ) -> ToolExecutionFuture<'_> {
        self.execute_with_observer(
            plan,
            Some(observer),
            crate::ports::tool::ToolCancellation::never(),
        )
    }

    fn execute_interruptible(
        &self,
        plan: ToolExecutionPlan,
        observer: Option<crate::ports::tool::ToolObservationSender>,
        cancellation: crate::ports::tool::ToolCancellation,
    ) -> ToolExecutionFuture<'_> {
        let cooperative = matches!(
            self.tools.get(&plan.call.name),
            Some(RegisteredTool::Builtin(BuiltinTool::Command(_)))
        );
        let execution = self.execute_with_observer(plan, observer, cancellation.clone());
        if cooperative {
            execution
        } else {
            Box::pin(cancellation.interrupt(execution))
        }
    }
}

impl ToolRegistry {
    fn execute_with_observer(
        &self,
        plan: ToolExecutionPlan,
        observer: Option<crate::ports::tool::ToolObservationSender>,
        cancellation: crate::ports::tool::ToolCancellation,
    ) -> ToolExecutionFuture<'_> {
        let tool = self.tools.get(&plan.call.name);
        Box::pin(async move {
            let Some(tool) = tool else {
                return Ok(ToolExecutionOutcome::failed(
                    plan,
                    "tool_not_registered",
                    json!({"error": "execution plan names an unregistered Tool"}),
                ));
            };
            if let RegisteredTool::Mcp { tool, .. } = tool {
                return mcp::execute(tool, plan).await;
            }
            let RegisteredTool::Builtin(tool) = tool else {
                unreachable!("registered tool is builtin or MCP")
            };
            let tool = *tool;
            // Catalog-only discovery does not touch the leased environment.
            // Keep deferred lifecycle controls reachable, without exempting
            // arbitrary read-only filesystem/process tools from resource locks.
            if tool == BuiltinTool::ToolSearch {
                return tool.execute(self, plan, observer, cancellation).await;
            }
            if !matches!(tool, BuiltinTool::Job(_) | BuiltinTool::Pty(_))
                && let Some(conflict) = self.active_leases.conflicting(&plan)
            {
                return Ok(active_resource_busy(plan, conflict));
            }
            let timeout_plan = plan.clone();
            let execution = tool.execute(self, plan, observer, cancellation.clone());
            tokio::pin!(execution);
            tokio::select! {
                biased;
                // Once operator cancellation starts command settlement, the
                // original execution deadline must not drop its wait/drain/ack.
                () = cancellation.wait(), if matches!(tool, BuiltinTool::Command(_)) => execution.await,
                outcome = &mut execution => outcome,
                () = tokio::time::sleep(Duration::from_millis(timeout_plan.timeout_ms)) => Ok(ToolExecutionOutcome::cancelled(
                    timeout_plan,
                    "tool_timeout",
                )),
            }
        })
    }

    pub fn has_active_processes(&self) -> bool {
        self.jobs.has_active_processes() || self.ptys.has_active_processes()
    }
}

fn active_resource_busy(
    plan: ToolExecutionPlan,
    conflict: ActiveProcessId,
) -> ToolExecutionOutcome {
    match conflict {
        ActiveProcessId::Job(job_id) => ToolExecutionOutcome::failed(
            plan,
            "job_resource_busy",
            json!({
                "error": "Tool resources conflict with an active background Job",
                "conflicting_job_id": job_id,
            }),
        ),
        ActiveProcessId::Pty(pty_session_id) => ToolExecutionOutcome::failed(
            plan,
            "pty_resource_busy",
            json!({
                "error": "Tool resources conflict with an active PTY session",
                "conflicting_pty_session_id": pty_session_id,
            }),
        ),
    }
}

impl BuiltinTool {
    fn name(self) -> &'static str {
        match self {
            Self::Filesystem(tool) => tool.name(),
            Self::FileWrite(tool) => tool.name(),
            Self::Command(tool) => tool.name(),
            Self::Job(tool) => tool.name(),
            Self::Pty(tool) => tool.name(),
            Self::Memory(tool) => tool.name(),
            Self::Research(tool) => tool.name(),
            Self::SearchFiles => search::NAME,
            Self::ReadOutput => read_output::NAME,
            Self::ContextRead => context_read::NAME,
            Self::ArtifactStore => artifact::NAME,
            Self::ViewImage => view_image::NAME,
            Self::HttpRequest => http_request::NAME,
            Self::InspectEnvironment => environment::NAME,
            Self::InspectProject => snapshot::NAME,
            Self::ToolSearch => tool_search::NAME,
        }
    }
    fn spec(self, registry: &ToolRegistry) -> ToolSpec {
        match self {
            Self::Filesystem(tool) => tool.spec(),
            Self::FileWrite(tool) => tool.spec(),
            Self::Command(tool) => tool.spec(registry),
            Self::Job(tool) => tool.spec(),
            Self::Pty(tool) => tool.spec(),
            Self::Memory(tool) => tool.spec(registry),
            Self::Research(tool) => tool.spec(),
            Self::SearchFiles => search::spec(),
            Self::ReadOutput => read_output::spec(registry),
            Self::ContextRead => context_read::spec(registry),
            Self::ArtifactStore => artifact::spec(registry),
            Self::ViewImage => view_image::spec(registry),
            Self::HttpRequest => http_request::spec(registry),
            Self::InspectEnvironment => environment::spec(),
            Self::InspectProject => snapshot::spec(),
            Self::ToolSearch => tool_search::spec(),
        }
    }
    fn normalize(
        self,
        registry: &ToolRegistry,
        arguments: Value,
    ) -> Result<(Value, Vec<ResourceKey>), ToolExecutorError> {
        let project_root = &registry.project_root;
        let control = registry.control_directory_name;
        let artifact_scope = &registry.artifact_scope;
        let window_policy = registry.window_policy;
        match self {
            Self::Filesystem(tool) => tool.normalize(registry, arguments),
            Self::FileWrite(tool) => tool.normalize(registry, arguments),
            Self::Command(tool) => tool.normalize(registry, arguments),
            Self::Job(tool) => tool.normalize(registry, arguments),
            Self::Pty(tool) => tool.normalize(registry, arguments),
            Self::Memory(tool) => tool.normalize(registry, arguments),
            Self::Research(tool) => tool.normalize(registry, arguments),
            Self::SearchFiles => search::normalize(project_root, arguments),
            Self::ReadOutput => read_output::normalize(arguments, window_policy),
            Self::ContextRead => context_read::normalize(arguments, window_policy),
            Self::ArtifactStore => {
                artifact::normalize_for_scope(project_root, control, arguments, artifact_scope)
            }
            Self::ViewImage => {
                view_image::normalize_for_scope(project_root, control, arguments, artifact_scope)
            }
            Self::HttpRequest => http_request::normalize_for_scope(
                project_root,
                control,
                arguments,
                artifact_scope,
                window_policy,
            ),
            Self::InspectEnvironment => environment::normalize(project_root, arguments),
            Self::InspectProject => snapshot::normalize(project_root, control, arguments),
            Self::ToolSearch => tool_search::normalize(arguments),
        }
    }
    async fn execute(
        self,
        registry: &ToolRegistry,
        plan: ToolExecutionPlan,
        observer: Option<crate::ports::tool::ToolObservationSender>,
        cancellation: crate::ports::tool::ToolCancellation,
    ) -> Result<ToolExecutionOutcome, crate::ports::tool::ToolExecutionError> {
        Ok(match self {
            Self::Filesystem(tool) => tool.execute(registry, plan).await,
            Self::FileWrite(tool) => tool.execute(registry, plan).await,
            Self::Command(tool) => {
                return tool.execute(registry, plan, observer, cancellation).await;
            }
            Self::Job(tool) => tool.execute(registry, plan).await,
            Self::Pty(tool) => tool.execute(registry, plan).await,
            Self::Memory(tool) => tool.execute(registry, plan).await,
            Self::Research(tool) => tool.execute(registry, plan).await,
            Self::SearchFiles => search::execute(&registry.project_root, plan).await,
            Self::ReadOutput => {
                read_output::execute(&registry.project_root, plan, registry.window_policy).await
            }
            Self::ContextRead => context_read::execute(registry, plan).await,
            Self::ArtifactStore => artifact::execute(&registry.project_root, plan).await,
            Self::ViewImage => view_image::execute(&registry.project_root, plan).await,
            Self::HttpRequest => {
                http_request::execute(&registry.project_root, plan, registry.window_policy).await
            }
            Self::InspectEnvironment => environment::execute(&registry.project_root, plan).await,
            Self::InspectProject => snapshot::execute_registered(registry, plan).await,
            Self::ToolSearch => {
                let deferred = plan.deferred_tool_specs().to_vec();
                tool_search::execute(plan, &deferred)
            }
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn spec(
    name: &str,
    version: &str,
    description: String,
    exposure: ToolExposure,
    input_schema: Value,
    output_schema: Value,
    parallelism: ToolParallelism,
    resource_key_template: Option<&str>,
    effect: ToolEffect,
    retry_policy: ToolRetryPolicy,
    determinism: ToolDeterminism,
    timeout_ms: u64,
) -> ToolSpec {
    ToolSpec {
        name: ToolName::new(name),
        version: version.to_owned(),
        description,
        input_schema: with_execution_control(input_schema),
        output_schema,
        exposure,
        parallelism,
        resource_key_template: resource_key_template.map(str::to_owned),
        effect,
        retry_policy,
        determinism,
        cache_policy: ToolCachePolicy::None,
        timeout_ms,
        output_policy: ToolOutputPolicy {
            max_bytes: ContextWindowPolicy::for_window(32768).tool_output_bytes(),
        },
    }
}

/// Canonical model-facing guidance.  Keeping the four headings in the
/// ToolSpec owner makes descriptions searchable, auditable and stable across
/// every Adapter (including `tool_search`) without duplicating prompt prose.
fn description(
    use_when: &str,
    do_not_use: &str,
    prefer: &str,
    produces: &str,
    observed_time: bool,
) -> String {
    let time_guidance = if observed_time {
        " Use observed timestamps, never guessed dates. For current UTC time use run_command argv [\"date\",\"-u\",\"+%Y-%m-%dT%H:%M:%SZ\"] when needed; preserve source timestamps for historical facts."
    } else {
        ""
    };
    format!(
        "Use when: {use_when}\nDo not use when: {do_not_use}\nPrefer: {prefer}{time_guidance}\nProduces: {produces}"
    )
}

fn with_execution_control(mut schema: Value) -> Value {
    let Some(properties) = schema
        .as_object_mut()
        .and_then(|object| object.get_mut("properties"))
        .and_then(Value::as_object_mut)
    else {
        return schema;
    };
    properties.insert(
        "execution".to_owned(),
        json!({
            "type": "object",
            "description": "Omit on first execution. Only for deliberate reruns of completed actions.",
            "properties": {
                "rerun_reason": {"type": "string", "minLength": 1,
                    "description": "Why repeat a completed action; never an automatic retry."}
            },
            "required": ["rerun_reason"],
            "additionalProperties": false
        }),
    );
    schema
}

fn decode_arguments<Arguments: for<'de> Deserialize<'de>>(
    tool_name: &str,
    value: Value,
) -> Result<Arguments, ToolExecutorError> {
    serde_json::from_value(value).map_err(|error| invalid_arguments(tool_name, error.to_string()))
}

fn invalid_arguments(tool_name: &str, message: impl Into<String>) -> ToolExecutorError {
    ToolExecutorError::InvalidArguments {
        tool_name: tool_name.to_owned(),
        message: message.into(),
    }
}

fn invalid_plan(plan: ToolExecutionPlan, error: ToolExecutorError) -> ToolExecutionOutcome {
    ToolExecutionOutcome::failed(
        plan,
        "invalid_execution_plan",
        json!({"error": error.to_string()}),
    )
}
