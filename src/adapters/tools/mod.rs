//! Minimal FullAccess Tool Registry for the M1a single-Agent loop.

mod active;
mod artifact;
mod command;
mod file_write;
mod filesystem;
mod job;
mod memory;
mod output;
mod pty;
mod research;
mod resources;
mod search;
mod snapshot;
mod tool_search;
mod view_image;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::adapters::project_event_store::ProjectEventStoreError;
use crate::adapters::tool_result_cache::LocalToolResultCache;
use crate::domain::events::StreamId;
use crate::domain::tools::{
    ExecutionId, ResolvedToolPlan, ResourceKey, ToolCachePolicy, ToolCall, ToolDeterminism,
    ToolEffect, ToolExecutionOutcome, ToolExecutionPlan, ToolExposure, ToolName, ToolOutputPolicy,
    ToolParallelism, ToolRetryPolicy, ToolSpec, split_execution_control,
};
use crate::ports::tool::{ToolExecutionFuture, ToolExecutor, ToolExecutorError};

use active::{ActiveProcessId, ActiveProcessLeases};

const TOOL_VERSION: &str = "1";
const DEFAULT_OUTPUT_BYTES: usize = 128 * 1024;

#[derive(Clone, Copy, Eq, PartialEq)]
enum BuiltinTool {
    ReadFile,
    ReadFiles,
    ListFiles,
    SearchFiles,
    WriteFile,
    ApplyPatch,
    RunCommand,
    RunCommands,
    StartJob,
    PollJob,
    InputJob,
    StopJob,
    StartPty,
    PollPty,
    InputPty,
    ResizePty,
    StopPty,
    ArtifactStore,
    ViewImage,
    MemoryRead,
    MemoryWrite,
    InspectProject,
    ResearchGet,
    ResearchUpdate,
    ToolSearch,
}

pub struct ToolRegistry {
    project_root: PathBuf,
    tools: BTreeMap<ToolName, BuiltinTool>,
    jobs: job::BackgroundJobs,
    ptys: pty::PtySessions,
    active_leases: ActiveProcessLeases,
    result_cache: Option<LocalToolResultCache>,
}

impl ToolRegistry {
    pub fn for_project(project_root: impl Into<PathBuf>) -> Self {
        let project_root = project_root.into();
        let tools = [
            (ToolName::new("read_file"), BuiltinTool::ReadFile),
            (ToolName::new("read_files"), BuiltinTool::ReadFiles),
            (ToolName::new("list_files"), BuiltinTool::ListFiles),
            (ToolName::new("search_files"), BuiltinTool::SearchFiles),
            (ToolName::new("write_file"), BuiltinTool::WriteFile),
            (ToolName::new("apply_patch"), BuiltinTool::ApplyPatch),
            (ToolName::new("run_command"), BuiltinTool::RunCommand),
            (ToolName::new("run_commands"), BuiltinTool::RunCommands),
            (ToolName::new("start_job"), BuiltinTool::StartJob),
            (ToolName::new("poll_job"), BuiltinTool::PollJob),
            (ToolName::new("input_job"), BuiltinTool::InputJob),
            (ToolName::new("stop_job"), BuiltinTool::StopJob),
            (ToolName::new("start_pty"), BuiltinTool::StartPty),
            (ToolName::new("poll_pty"), BuiltinTool::PollPty),
            (ToolName::new("input_pty"), BuiltinTool::InputPty),
            (ToolName::new("resize_pty"), BuiltinTool::ResizePty),
            (ToolName::new("stop_pty"), BuiltinTool::StopPty),
            (ToolName::new("artifact_store"), BuiltinTool::ArtifactStore),
            (ToolName::new("view_image"), BuiltinTool::ViewImage),
            (ToolName::new("memory_read"), BuiltinTool::MemoryRead),
            (ToolName::new("memory_write"), BuiltinTool::MemoryWrite),
            (
                ToolName::new("inspect_project"),
                BuiltinTool::InspectProject,
            ),
            (ToolName::new("research_get"), BuiltinTool::ResearchGet),
            (
                ToolName::new("research_update"),
                BuiltinTool::ResearchUpdate,
            ),
            (ToolName::new("tool_search"), BuiltinTool::ToolSearch),
        ]
        .into_iter()
        .collect();
        Self {
            result_cache: None,
            project_root,
            tools,
            jobs: job::BackgroundJobs::new(),
            ptys: pty::PtySessions::new(),
            active_leases: ActiveProcessLeases::new(),
        }
    }

    /// Enables the persistent Tool Result Cache only for one caller-bound
    /// Session. Ordinary registries remain cache-free so a hit can
    /// never outlive its mandatory Session audit chain.
    pub fn for_project_with_session(
        project_root: impl Into<PathBuf>,
        session_id: StreamId,
    ) -> Result<Self, ProjectEventStoreError> {
        let project_root = project_root.into();
        let result_cache = LocalToolResultCache::bind(&project_root, session_id)?;
        let mut registry = Self::for_project(project_root);
        registry.result_cache = Some(result_cache);
        Ok(registry)
    }

    fn canonical_spec(name: &ToolName, tool: BuiltinTool) -> ToolSpec {
        let mut builtin_spec = tool.spec();
        debug_assert_eq!(&builtin_spec.name, name);
        builtin_spec.exposure = tool.default_exposure();
        builtin_spec
    }

    fn canonical_specs(&self) -> Vec<ToolSpec> {
        self.tools
            .iter()
            .map(|(name, tool)| Self::canonical_spec(name, *tool))
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
        let arguments = std::mem::take(&mut call.arguments);
        let (arguments, rerun_reason) = split_execution_control(arguments).map_err(|error| {
            ToolExecutorError::InvalidArguments {
                tool_name: call.name.to_string(),
                message: error.to_string(),
            }
        })?;
        let (normalized_arguments, resource_keys) =
            tool.normalize(&self.project_root, arguments)?;
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
        if let Some(fingerprint) =
            tool.cache_resource_fingerprint(&self.project_root, &plan.call.arguments)
        {
            plan.bind_cache_resource_fingerprint(fingerprint)
                .map_err(|error| ToolExecutorError::PlanFailure {
                    tool_name: spec.name.to_string(),
                    message: error.to_string(),
                })?;
        }
        Ok(plan)
    }
}

impl ToolExecutor for ToolRegistry {
    fn catalog(&self) -> Vec<ToolSpec> {
        self.canonical_specs()
    }

    fn plan(
        &self,
        execution_id: ExecutionId,
        call: ToolCall,
    ) -> Result<ToolExecutionPlan, ToolExecutorError> {
        let tool =
            self.tools
                .get(&call.name)
                .copied()
                .ok_or_else(|| ToolExecutorError::UnknownTool {
                    tool_name: call.name.to_string(),
                })?;
        let spec = Self::canonical_spec(&call.name, tool);
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
        let tool = self.tools.get(&plan.call.name).copied();
        Box::pin(async move {
            let Some(tool) = tool else {
                return ToolExecutionOutcome::failed(
                    plan,
                    "tool_not_registered",
                    json!({"error": "execution plan names an unregistered Tool"}),
                );
            };
            if !is_process_lifecycle_tool(plan.call.name.as_str())
                && let Some(conflict) = self.active_leases.conflicting(&plan)
            {
                return active_resource_busy(plan, conflict);
            }
            // An explicit rerun is a deliberate fresh execution. It must not
            // be silently satisfied by a persistent derived result.
            let cache_plan = (self.result_cache.is_some()
                && plan.rerun_reason.is_none()
                && plan.result_cache_eligible())
            .then(|| plan.clone());
            if let (Some(cache), Some(_)) = (self.result_cache.as_ref(), cache_plan.as_ref())
                && tool
                    .cache_resource_fingerprint(&self.project_root, &plan.call.arguments)
                    .as_deref()
                    == plan.cache_resource_fingerprint.as_deref()
                && let Some(outcome) = cache.lookup(&plan)
            {
                return outcome;
            }
            let timeout_plan = plan.clone();
            if tool == BuiltinTool::ToolSearch {
                let deferred = plan.deferred_tool_specs().to_vec();
                return tool_search::execute(plan, &deferred);
            }
            let specs = if tool == BuiltinTool::InspectProject {
                if plan.resolved_tool_plan_fingerprint().is_some() {
                    plan.resolved_tool_specs().to_vec()
                } else {
                    self.specs()
                }
            } else {
                Vec::new()
            };
            let outcome = match tokio::time::timeout(
                Duration::from_millis(plan.timeout_ms),
                tool.execute(
                    &self.project_root,
                    &self.jobs,
                    &self.ptys,
                    &self.active_leases,
                    &specs,
                    plan,
                ),
            )
            .await
            {
                Ok(outcome) => outcome,
                Err(_) => ToolExecutionOutcome::cancelled(timeout_plan, "tool_timeout"),
            };
            if let (Some(cache), Some(cache_plan)) = (self.result_cache.as_ref(), cache_plan) {
                cache.store(&cache_plan, &outcome);
            }
            outcome
        })
    }
}

fn is_process_lifecycle_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "start_job"
            | "poll_job"
            | "input_job"
            | "stop_job"
            | "start_pty"
            | "poll_pty"
            | "input_pty"
            | "resize_pty"
            | "stop_pty"
    )
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
    const fn default_exposure(self) -> ToolExposure {
        match self {
            Self::ReadFile
            | Self::ReadFiles
            | Self::ListFiles
            | Self::SearchFiles
            | Self::ApplyPatch
            | Self::RunCommand
            | Self::RunCommands
            | Self::InspectProject
            | Self::ResearchGet
            | Self::ResearchUpdate
            | Self::ToolSearch => ToolExposure::Direct,
            Self::WriteFile
            | Self::StartJob
            | Self::PollJob
            | Self::InputJob
            | Self::StopJob
            | Self::StartPty
            | Self::PollPty
            | Self::InputPty
            | Self::ResizePty
            | Self::StopPty
            | Self::ArtifactStore
            | Self::MemoryRead
            | Self::MemoryWrite => ToolExposure::Deferred,
            Self::ViewImage => ToolExposure::Direct,
        }
    }

    fn normalize(
        self,
        project_root: &Path,
        arguments: Value,
    ) -> Result<(Value, Vec<ResourceKey>), ToolExecutorError> {
        match self {
            Self::ReadFile => filesystem::normalize_read_file(project_root, arguments),
            Self::ReadFiles => filesystem::normalize_read_files(project_root, arguments),
            Self::ListFiles => filesystem::normalize_list_files(project_root, arguments),
            Self::SearchFiles => search::normalize(project_root, arguments),
            Self::WriteFile => file_write::normalize_write_file(project_root, arguments),
            Self::ApplyPatch => file_write::normalize_apply_patch(project_root, arguments),
            Self::RunCommand => command::normalize_one(project_root, arguments),
            Self::RunCommands => command::normalize_batch(project_root, arguments),
            Self::StartJob => job::normalize_start(project_root, arguments),
            Self::PollJob => job::normalize_poll(arguments),
            Self::InputJob => job::normalize_input(arguments),
            Self::StopJob => job::normalize_stop(arguments),
            Self::StartPty => pty::normalize_start(project_root, arguments),
            Self::PollPty => pty::normalize_poll(arguments),
            Self::InputPty => pty::normalize_input(arguments),
            Self::ResizePty => pty::normalize_resize(arguments),
            Self::StopPty => pty::normalize_stop(arguments),
            Self::ArtifactStore => artifact::normalize(project_root, arguments),
            Self::ViewImage => view_image::normalize(project_root, arguments),
            Self::MemoryRead => memory::normalize_read(project_root, arguments),
            Self::MemoryWrite => memory::normalize_write(project_root, arguments),
            Self::InspectProject => snapshot::normalize(project_root, arguments),
            Self::ResearchGet => research::normalize_get(project_root, arguments),
            Self::ResearchUpdate => research::normalize_update(project_root, arguments),
            Self::ToolSearch => tool_search::normalize(arguments),
        }
    }

    async fn execute(
        self,
        project_root: &Path,
        jobs: &job::BackgroundJobs,
        ptys: &pty::PtySessions,
        active_leases: &ActiveProcessLeases,
        specs: &[ToolSpec],
        plan: ToolExecutionPlan,
    ) -> ToolExecutionOutcome {
        match self {
            Self::ReadFile => filesystem::execute_read_file(project_root, plan).await,
            Self::ReadFiles => filesystem::execute_read_files(project_root, plan).await,
            Self::ListFiles => filesystem::execute_list_files(project_root, plan).await,
            Self::SearchFiles => search::execute(project_root, plan).await,
            Self::WriteFile => file_write::execute_write_file(project_root, plan).await,
            Self::ApplyPatch => file_write::execute_apply_patch(project_root, plan).await,
            Self::RunCommand => command::execute_one(project_root, plan).await,
            Self::RunCommands => command::execute_batch(project_root, plan).await,
            Self::StartJob => job::execute_start(project_root, jobs, active_leases, plan).await,
            Self::PollJob => job::execute_poll(jobs, plan).await,
            Self::InputJob => job::execute_input(jobs, plan).await,
            Self::StopJob => job::execute_stop(jobs, plan).await,
            Self::StartPty => pty::execute_start(project_root, ptys, active_leases, plan).await,
            Self::PollPty => pty::execute_poll(ptys, plan).await,
            Self::InputPty => pty::execute_input(ptys, plan).await,
            Self::ResizePty => pty::execute_resize(ptys, plan).await,
            Self::StopPty => pty::execute_stop(ptys, plan).await,
            Self::ArtifactStore => artifact::execute(project_root, plan).await,
            Self::ViewImage => view_image::execute(project_root, plan).await,
            Self::MemoryRead => memory::execute_read(project_root, plan).await,
            Self::MemoryWrite => memory::execute_write(project_root, plan).await,
            Self::InspectProject => snapshot::execute(project_root, specs, plan).await,
            Self::ResearchGet => research::execute_get(project_root, plan).await,
            Self::ResearchUpdate => research::execute_update(project_root, plan).await,
            Self::ToolSearch => unreachable!("Tool search executes through ToolRegistry state"),
        }
    }

    fn spec(self) -> ToolSpec {
        match self {
            Self::ReadFile => spec(
                "read_file",
                json!({
                    "type": "object",
                    "properties": {"path": {"type": "string"}},
                    "required": ["path"],
                    "additionalProperties": false
                }),
                self.output_schema(),
                ToolParallelism::Parallel,
                Some("file:{path}"),
                ToolEffect::ReadOnly,
                ToolRetryPolicy::ReplaySafe,
                ToolDeterminism::EnvironmentDependent,
                10_000,
            ),
            Self::ReadFiles => spec(
                "read_files",
                json!({
                    "type": "object",
                    "properties": {
                        "files": {
                            "type": "array",
                            "minItems": 1,
                            "maxItems": 32,
                            "items": {
                                "type": "object",
                                "properties": {
                                    "path": {"type": "string"},
                                    "start_line": {"type": "integer", "minimum": 1},
                                    "end_line": {"type": "integer", "minimum": 1}
                                },
                                "required": ["path"],
                                "additionalProperties": false
                            }
                        }
                    },
                    "required": ["files"],
                    "additionalProperties": false
                }),
                self.output_schema(),
                ToolParallelism::Parallel,
                Some("file:{files[*].path}"),
                ToolEffect::ReadOnly,
                ToolRetryPolicy::ReplaySafe,
                ToolDeterminism::EnvironmentDependent,
                15_000,
            ),
            Self::ListFiles => spec(
                "list_files",
                json!({
                    "type": "object",
                    "properties": {
                        "path": {"type": "string"},
                        "max_depth": {"type": "integer", "minimum": 0, "maximum": 64}
                    },
                    "additionalProperties": false
                }),
                self.output_schema(),
                ToolParallelism::Parallel,
                Some("file:{path}"),
                ToolEffect::ReadOnly,
                ToolRetryPolicy::ReplaySafe,
                ToolDeterminism::EnvironmentDependent,
                15_000,
            ),
            Self::SearchFiles => spec(
                "search_files",
                json!({
                    "type": "object",
                    "properties": {
                        "pattern": {"type": "string"},
                        "patterns": {
                            "type": "array",
                            "minItems": 1,
                            "maxItems": 32,
                            "items": {"type": "string"}
                        },
                        "paths": {"type": "array", "minItems": 1, "maxItems": 32, "items": {"type": "string"}}
                    },
                    "oneOf": [
                        {"required": ["pattern"], "not": {"required": ["patterns"]}},
                        {"required": ["patterns"], "not": {"required": ["pattern"]}}
                    ],
                    "additionalProperties": false
                }),
                self.output_schema(),
                ToolParallelism::Parallel,
                Some("file:{paths}"),
                ToolEffect::ReadOnly,
                ToolRetryPolicy::ReplaySafe,
                ToolDeterminism::EnvironmentDependent,
                30_000,
            ),
            Self::WriteFile => spec(
                "write_file",
                file_write::write_file_input_schema(),
                file_write::output_schema(),
                ToolParallelism::ResourceLocked,
                Some("file:{path}"),
                ToolEffect::LocalMutation,
                ToolRetryPolicy::NeverAutoRetry,
                ToolDeterminism::EnvironmentDependent,
                30_000,
            ),
            Self::ApplyPatch => spec(
                "apply_patch",
                file_write::apply_patch_input_schema(),
                file_write::output_schema(),
                ToolParallelism::ResourceLocked,
                Some("file:{preimages[*].path}"),
                ToolEffect::LocalMutation,
                ToolRetryPolicy::NeverAutoRetry,
                ToolDeterminism::EnvironmentDependent,
                30_000,
            ),
            Self::RunCommand => spec(
                "run_command",
                json!({
                    "type": "object",
                    "properties": {
                        "argv": {"type": "array", "items": {"type": "string"}, "minItems": 1},
                        "cwd": {"type": "string"},
                        "resources": {"type": "array", "items": command::resource_schema()}
                    },
                    "required": ["argv"],
                    "additionalProperties": false
                }),
                self.output_schema(),
                ToolParallelism::ResourceLocked,
                Some("resource:{resources}"),
                ToolEffect::Interactive,
                ToolRetryPolicy::NeverAutoRetry,
                ToolDeterminism::Nondeterministic,
                120_000,
            ),
            Self::RunCommands => spec(
                "run_commands",
                json!({
                    "type": "object",
                    "properties": {
                        "mode": {"type": "string", "enum": ["serial", "parallel"]},
                        "commands": {
                            "type": "array",
                            "minItems": 1,
                            "maxItems": 32,
                            "items": {
                                "type": "object",
                                "properties": {
                                    "argv": {"type": "array", "items": {"type": "string"}, "minItems": 1},
                                    "cwd": {"type": "string"},
                                    "resources": {"type": "array", "items": command::resource_schema()}
                                },
                                "required": ["argv"],
                                "additionalProperties": false
                            }
                        }
                    },
                    "required": ["mode", "commands"],
                    "additionalProperties": false
                }),
                self.output_schema(),
                ToolParallelism::ResourceLocked,
                Some("resource:{commands[*].resources}"),
                ToolEffect::Interactive,
                ToolRetryPolicy::NeverAutoRetry,
                ToolDeterminism::Nondeterministic,
                120_000,
            ),
            Self::StartJob => spec(
                "start_job",
                json!({
                    "type": "object",
                    "properties": {
                        "argv": {"type": "array", "items": {"type": "string"}, "minItems": 1},
                        "cwd": {"type": "string"},
                        "resources": {"type": "array", "items": command::resource_schema()},
                        "timeout_ms": {"type": "integer", "minimum": 1}
                    },
                    "required": ["argv"],
                    "additionalProperties": false
                }),
                self.output_schema(),
                ToolParallelism::ResourceLocked,
                Some("resource:{resources}"),
                ToolEffect::Interactive,
                ToolRetryPolicy::DedupeByKey,
                ToolDeterminism::Nondeterministic,
                10_000,
            ),
            Self::PollJob => spec(
                "poll_job",
                json!({
                    "type": "object",
                    "properties": {
                        "job_id": {"type": "string", "minLength": 1},
                        "stdout_offset": {"type": "integer", "minimum": 0},
                        "stderr_offset": {"type": "integer", "minimum": 0}
                    },
                    "required": ["job_id"],
                    "additionalProperties": false
                }),
                self.output_schema(),
                ToolParallelism::ResourceLocked,
                Some("job:{job_id}"),
                ToolEffect::ReadOnly,
                ToolRetryPolicy::ReplaySafe,
                ToolDeterminism::EnvironmentDependent,
                10_000,
            ),
            Self::InputJob => spec(
                "input_job",
                json!({
                    "type": "object",
                    "properties": {
                        "job_id": {"type": "string", "minLength": 1},
                        "input": {"type": "string"},
                        "close": {"type": "boolean"}
                    },
                    "required": ["job_id", "input"],
                    "additionalProperties": false
                }),
                self.output_schema(),
                ToolParallelism::ResourceLocked,
                Some("job:{job_id}"),
                ToolEffect::Interactive,
                ToolRetryPolicy::NeverAutoRetry,
                ToolDeterminism::Nondeterministic,
                10_000,
            ),
            Self::StopJob => spec(
                "stop_job",
                json!({
                    "type": "object",
                    "properties": {
                        "job_id": {"type": "string", "minLength": 1}
                    },
                    "required": ["job_id"],
                    "additionalProperties": false
                }),
                self.output_schema(),
                ToolParallelism::ResourceLocked,
                Some("job:{job_id}"),
                ToolEffect::Interactive,
                ToolRetryPolicy::NeverAutoRetry,
                ToolDeterminism::Nondeterministic,
                10_000,
            ),
            Self::StartPty => spec(
                "start_pty",
                json!({
                    "type": "object",
                    "properties": {
                        "argv": {"type": "array", "items": {"type": "string"}, "minItems": 1},
                        "cwd": {"type": "string"},
                        "resources": {"type": "array", "items": command::resource_schema()},
                        "rows": {"type": "integer", "minimum": 1, "maximum": 65535},
                        "cols": {"type": "integer", "minimum": 1, "maximum": 65535}
                    },
                    "required": ["argv"],
                    "additionalProperties": false
                }),
                self.output_schema(),
                ToolParallelism::ResourceLocked,
                Some("resource:{resources}"),
                ToolEffect::Interactive,
                ToolRetryPolicy::DedupeByKey,
                ToolDeterminism::Nondeterministic,
                10_000,
            ),
            Self::PollPty => spec(
                "poll_pty",
                json!({
                    "type": "object",
                    "properties": {
                        "pty_session_id": {"type": "string", "minLength": 1},
                        "offset": {"type": "integer", "minimum": 0}
                    },
                    "required": ["pty_session_id"],
                    "additionalProperties": false
                }),
                self.output_schema(),
                ToolParallelism::ResourceLocked,
                Some("pty:{pty_session_id}"),
                ToolEffect::ReadOnly,
                ToolRetryPolicy::ReplaySafe,
                ToolDeterminism::EnvironmentDependent,
                10_000,
            ),
            Self::InputPty => spec(
                "input_pty",
                json!({
                    "type": "object",
                    "properties": {
                        "pty_session_id": {"type": "string", "minLength": 1},
                        "input": {"type": "string"}
                    },
                    "required": ["pty_session_id", "input"],
                    "additionalProperties": false
                }),
                self.output_schema(),
                ToolParallelism::ResourceLocked,
                Some("pty:{pty_session_id}"),
                ToolEffect::Interactive,
                ToolRetryPolicy::NeverAutoRetry,
                ToolDeterminism::Nondeterministic,
                10_000,
            ),
            Self::ResizePty => spec(
                "resize_pty",
                json!({
                    "type": "object",
                    "properties": {
                        "pty_session_id": {"type": "string", "minLength": 1},
                        "rows": {"type": "integer", "minimum": 1, "maximum": 65535},
                        "cols": {"type": "integer", "minimum": 1, "maximum": 65535}
                    },
                    "required": ["pty_session_id", "rows", "cols"],
                    "additionalProperties": false
                }),
                self.output_schema(),
                ToolParallelism::ResourceLocked,
                Some("pty:{pty_session_id}"),
                ToolEffect::Interactive,
                ToolRetryPolicy::NeverAutoRetry,
                ToolDeterminism::Nondeterministic,
                10_000,
            ),
            Self::StopPty => spec(
                "stop_pty",
                json!({
                    "type": "object",
                    "properties": {
                        "pty_session_id": {"type": "string", "minLength": 1}
                    },
                    "required": ["pty_session_id"],
                    "additionalProperties": false
                }),
                self.output_schema(),
                ToolParallelism::ResourceLocked,
                Some("pty:{pty_session_id}"),
                ToolEffect::Interactive,
                ToolRetryPolicy::NeverAutoRetry,
                ToolDeterminism::Nondeterministic,
                10_000,
            ),
            Self::ArtifactStore => spec(
                "artifact_store",
                artifact::input_schema(),
                artifact::output_schema(),
                ToolParallelism::ResourceLocked,
                Some("file:{source_path},file:.cyber-agent/research.jsonl"),
                ToolEffect::LocalMutation,
                ToolRetryPolicy::NeverAutoRetry,
                ToolDeterminism::EnvironmentDependent,
                120_000,
            ),
            Self::ViewImage => spec(
                "view_image",
                view_image::input_schema(),
                view_image::output_schema(),
                ToolParallelism::ResourceLocked,
                Some("file:{source_path},file:.cyber-agent/research.jsonl"),
                ToolEffect::LocalMutation,
                ToolRetryPolicy::NeverAutoRetry,
                ToolDeterminism::EnvironmentDependent,
                120_000,
            ),
            Self::MemoryRead => spec(
                "memory_read",
                memory::read_input_schema(),
                memory::output_schema(),
                ToolParallelism::ResourceLocked,
                Some("file:.cyber-agent/memory/{path},file:.cyber-agent/research.jsonl"),
                ToolEffect::LocalMutation,
                ToolRetryPolicy::NeverAutoRetry,
                ToolDeterminism::EnvironmentDependent,
                10_000,
            ),
            Self::MemoryWrite => spec(
                "memory_write",
                memory::write_input_schema(),
                memory::output_schema(),
                ToolParallelism::ResourceLocked,
                Some("file:.cyber-agent/memory/{path},file:.cyber-agent/research.jsonl"),
                ToolEffect::LocalMutation,
                ToolRetryPolicy::NeverAutoRetry,
                ToolDeterminism::EnvironmentDependent,
                30_000,
            ),
            Self::InspectProject => spec(
                "inspect_project",
                snapshot::input_schema(),
                snapshot::output_schema(),
                ToolParallelism::ResourceLocked,
                Some("file:.cyber-agent/cache"),
                ToolEffect::LocalMutation,
                ToolRetryPolicy::NeverAutoRetry,
                ToolDeterminism::EnvironmentDependent,
                30_000,
            ),
            Self::ResearchGet => cacheable(spec(
                "research_get",
                research::get_input_schema(),
                research::get_output_schema(),
                ToolParallelism::Parallel,
                Some("file:.cyber-agent/research.jsonl"),
                ToolEffect::ReadOnly,
                ToolRetryPolicy::ReplaySafe,
                ToolDeterminism::Deterministic,
                10_000,
            )),
            Self::ResearchUpdate => spec(
                "research_update",
                research::update_input_schema(),
                research::update_output_schema(),
                ToolParallelism::ResourceLocked,
                Some("file:.cyber-agent/research.jsonl"),
                ToolEffect::LocalMutation,
                ToolRetryPolicy::NeverAutoRetry,
                ToolDeterminism::EnvironmentDependent,
                10_000,
            ),
            Self::ToolSearch => spec(
                "tool_search",
                tool_search::input_schema(),
                self.output_schema(),
                ToolParallelism::Parallel,
                None,
                ToolEffect::ReadOnly,
                ToolRetryPolicy::ReplaySafe,
                ToolDeterminism::Deterministic,
                5_000,
            ),
        }
    }

    fn output_schema(self) -> Value {
        let string = |nullable: bool| {
            if nullable {
                json!({"type": ["string", "null"]})
            } else {
                json!({"type": "string", "minLength": 1})
            }
        };
        let integer = json!({"type": "integer", "minimum": 0});
        let boolean = json!({"type": "boolean"});
        let status = |values: &[&str]| json!({"type": "string", "enum": values});
        let stream = json!({
            "type": "object",
            "properties": {
                "offset": {"type": "integer", "minimum": 0},
                "next_offset": {"type": "integer", "minimum": 0},
                "data": {"type": "string"},
                "truncated": {"type": "boolean"},
                "read_error": {"type": ["string", "null"]}
            },
            "required": ["offset", "next_offset", "data", "truncated", "read_error"],
            "additionalProperties": false
        });
        match self {
            Self::ReadFile => read_file_output_schema(),
            Self::ReadFiles => json!({
                "type": "object",
                "properties": {
                    "files": {"type": "array", "minItems": 1, "maxItems": 32, "items": {"oneOf": [
                        {"type": "object", "properties": {
                            "status": {"type": "string", "const": "succeeded"},
                            "result": read_file_output_schema()
                        }, "required": ["status", "result"], "additionalProperties": false},
                        {"type": "object", "properties": {
                            "status": {"type": "string", "const": "failed"},
                            "result": file_read_failure_schema()
                        }, "required": ["status", "result"], "additionalProperties": false}
                    ]}},
                    "partial_failure": boolean
                }, "required": ["files", "partial_failure"], "additionalProperties": false
            }),
            Self::ListFiles => json!({
                "type": "object", "properties": {
                    "path": string(false), "paths": {"type": "string"},
                    "path_count": integer, "truncated": boolean
                }, "required": ["path", "paths", "path_count", "truncated"], "additionalProperties": false
            }),
            Self::SearchFiles => json!({
                "oneOf": [
                    search_result_schema(),
                    {"type": "object", "properties": {
                        "results": {"type": "array", "minItems": 1, "maxItems": 32, "items": {"oneOf": [
                            {"type": "object", "properties": {
                                "pattern": string(false), "status": {"type": "string", "const": "succeeded"},
                                "result": search_result_schema()
                            }, "required": ["pattern", "status", "result"], "additionalProperties": false},
                            {"type": "object", "properties": {
                                "pattern": string(false), "status": {"type": "string", "const": "failed"},
                                "result": search_failure_schema()
                            }, "required": ["pattern", "status", "result"], "additionalProperties": false}
                        ]}},
                        "partial_failure": boolean
                    }, "required": ["results", "partial_failure"], "additionalProperties": false}
                ]
            }),
            Self::RunCommand => command_output_schema(),
            Self::RunCommands => json!({
                "type": "object", "properties": {
                    "commands": {"type": "array", "minItems": 1, "maxItems": 32, "items": {"type": "object", "properties": {
                        "index": {"type": "integer", "minimum": 0}, "status": {"type": "string", "enum": ["succeeded", "failed"]}, "result": command_output_schema()
                    }, "required": ["index", "status", "result"], "additionalProperties": false}},
                    "partial_failure": boolean
                }, "required": ["commands", "partial_failure"], "additionalProperties": false
            }),
            Self::StartJob => json!({
                "type": "object", "properties": {"job_id": string(false), "status": status(&["running"]), "pid": {"type": "integer", "minimum": 1}},
                "required": ["job_id", "status", "pid"], "additionalProperties": false
            }),
            Self::PollJob => json!({
                "type": "object", "properties": {"job_id": string(false), "status": status(&["running", "stopping", "completed", "failed", "cancelled", "timed_out"]), "stdout": stream.clone(), "stderr": stream, "completion": {"oneOf": [job_completion_schema(), {"type": "null"}]}},
                "required": ["job_id", "status", "stdout", "stderr", "completion"], "additionalProperties": false
            }),
            Self::InputJob => json!({
                "type": "object", "properties": {"job_id": string(false), "bytes_written": integer, "input_closed": boolean},
                "required": ["job_id", "bytes_written", "input_closed"], "additionalProperties": false
            }),
            Self::StopJob => json!({
                "type": "object", "properties": {"job_id": string(false), "status": status(&["completed", "failed", "cancelled", "timed_out"]), "completion": job_completion_schema(), "already_terminal": boolean},
                "required": ["job_id", "status", "completion", "already_terminal"], "additionalProperties": false
            }),
            Self::StartPty => json!({
                "type": "object", "properties": {"pty_session_id": string(false), "status": status(&["running"])},
                "required": ["pty_session_id", "status"], "additionalProperties": false
            }),
            Self::PollPty => json!({
                "type": "object", "properties": {"pty_session_id": string(false), "status": status(&["running", "stopping", "completed", "failed", "cancelled"]), "output": stream, "completion": {"oneOf": [pty_completion_schema(), {"type": "null"}]}},
                "required": ["pty_session_id", "status", "output", "completion"], "additionalProperties": false
            }),
            Self::InputPty => json!({
                "type": "object", "properties": {"pty_session_id": string(false), "bytes_written": integer},
                "required": ["pty_session_id", "bytes_written"], "additionalProperties": false
            }),
            Self::ResizePty => json!({
                "type": "object", "properties": {"pty_session_id": string(false), "rows": {"type": "integer", "minimum": 1, "maximum": 65535}, "cols": {"type": "integer", "minimum": 1, "maximum": 65535}},
                "required": ["pty_session_id", "rows", "cols"], "additionalProperties": false
            }),
            Self::StopPty => json!({
                "type": "object", "properties": {"pty_session_id": string(false), "status": status(&["completed", "failed", "cancelled"]), "completion": pty_completion_schema(), "already_terminal": boolean},
                "required": ["pty_session_id", "status", "completion", "already_terminal"], "additionalProperties": false
            }),
            Self::ToolSearch => json!({
                "type": "object", "properties": {"query": string(false), "matches": {"type": "array", "maxItems": 16, "items": {"type": "object", "properties": {"name": string(false), "description": string(false), "exposure": {"type": "string", "enum": ["direct", "deferred", "hidden"]}, "input_schema": {"type": "object"}, "output_schema": {"type": "object"}}, "required": ["name", "description", "exposure", "input_schema", "output_schema"], "additionalProperties": false}}, "truncated": boolean},
                "required": ["query", "matches", "truncated"], "additionalProperties": false
            }),
            _ => json!({
                "type": "object", "properties": {"status": {"type": "string"}, "event_appended": boolean, "event_id": {"type": "string"}, "message": {"type": "string"}},
                "additionalProperties": false
            }),
        }
    }

    fn cache_resource_fingerprint(self, project_root: &Path, arguments: &Value) -> Option<String> {
        match self {
            Self::ResearchGet => research::cache_resource_fingerprint(project_root, arguments),
            _ => None,
        }
    }
}

fn read_file_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "path": {"type": "string", "minLength": 1},
            "content": {"type": "string"},
            "original_bytes": {"type": "integer", "minimum": 0},
            "selected_bytes": {"type": "integer", "minimum": 0},
            "truncated": {"type": "boolean"},
            "start_line": {"type": ["integer", "null"], "minimum": 1},
            "end_line": {"type": ["integer", "null"], "minimum": 1}
        },
        "required": ["path", "content", "original_bytes", "selected_bytes", "truncated", "start_line", "end_line"],
        "additionalProperties": false
    })
}

fn file_read_failure_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "error": {"type": "string", "minLength": 1},
            "path": {"type": "string", "minLength": 1}
        },
        "required": ["error", "path"],
        "additionalProperties": false
    })
}

fn search_result_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "matches": {"type": "string"},
            "original_bytes": {"type": "integer", "minimum": 0},
            "truncated": {"type": "boolean"}
        },
        "required": ["matches", "original_bytes", "truncated"],
        "additionalProperties": false
    })
}

fn search_failure_schema() -> Value {
    json!({
        "oneOf": [
            {
                "type": "object",
                "properties": {"error": {"type": "string", "minLength": 1}},
                "required": ["error"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": {
                    "exit_code": {"type": ["integer", "null"]},
                    "stdout": {"type": "string"},
                    "stderr": {"type": "string"},
                    "truncated": {"type": "boolean"}
                },
                "required": ["exit_code", "stdout", "stderr", "truncated"],
                "additionalProperties": false
            }
        ]
    })
}

fn job_completion_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "job_id": {"type": "string", "minLength": 1},
            "status": {"type": "string", "enum": ["completed", "failed", "cancelled", "timed_out"]},
            "exit_code": {"type": ["integer", "null"]},
            "start_execution_id": {"type": "string", "minLength": 1},
            "start_plan_fingerprint": {"type": "string", "minLength": 1},
            "exit_reason": {"type": "string", "minLength": 1}
        },
        "required": ["job_id", "status", "exit_code", "start_execution_id", "start_plan_fingerprint", "exit_reason"],
        "additionalProperties": false
    })
}

fn pty_completion_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "pty_session_id": {"type": "string", "minLength": 1},
            "status": {"type": "string", "enum": ["completed", "failed", "cancelled"]},
            "exit_code": {"type": ["integer", "null"]},
            "start_execution_id": {"type": "string", "minLength": 1},
            "start_plan_fingerprint": {"type": "string", "minLength": 1},
            "exit_reason": {"type": "string", "minLength": 1}
        },
        "required": ["pty_session_id", "status", "exit_code", "start_execution_id", "start_plan_fingerprint", "exit_reason"],
        "additionalProperties": false
    })
}

fn command_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "cwd": {"type": "string", "minLength": 1}, "exit_code": {"type": ["integer", "null"]},
            "stdout": {"type": "string"}, "stderr": {"type": "string"}, "truncated": {"type": "boolean"}
        },
        "required": ["cwd", "exit_code", "stdout", "stderr", "truncated"],
        "additionalProperties": false
    })
}

#[allow(clippy::too_many_arguments)]
fn spec(
    name: &str,
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
        version: TOOL_VERSION.to_owned(),
        description: canonical_description(name),
        input_schema: with_execution_control(input_schema),
        output_schema,
        exposure: ToolExposure::Direct,
        parallelism,
        resource_key_template: resource_key_template.map(str::to_owned),
        effect,
        retry_policy,
        determinism,
        cache_policy: ToolCachePolicy::None,
        timeout_ms,
        output_policy: ToolOutputPolicy {
            max_bytes: DEFAULT_OUTPUT_BYTES,
        },
    }
}

/// Canonical model-facing guidance.  Keeping the four headings in the
/// ToolSpec owner makes descriptions searchable, auditable and stable across
/// every Adapter (including `tool_search`) without duplicating prompt prose.
fn canonical_description(name: &str) -> String {
    let use_when = match name {
        "read_file" => "one local file",
        "read_files" => "several independent local files",
        "list_files" => "paths or a bounded tree",
        "search_files" => "bounded text matches in local files",
        "write_file" => "one atomic expected-hash file replacement",
        "apply_patch" => "one validated file patch",
        "run_command" => "one bounded local command",
        "run_commands" => "independent bounded commands",
        "start_job" => "long non-interactive work",
        "poll_job" => "new Job output or completion",
        "input_job" => "stdin for a running Job",
        "stop_job" => "stop a running Job",
        "start_pty" => "a terminal-required child",
        "poll_pty" => "new PTY output or completion",
        "input_pty" => "input for a running PTY",
        "resize_pty" => "resize a running PTY",
        "stop_pty" => "stop a running PTY",
        "artifact_store" => "durable raw-output provenance",
        "view_image" => "one supported local image",
        "memory_read" => "bounded project Memory",
        "memory_write" => "a focused project Memory update",
        "inspect_project" => "current project metadata",
        "research_get" => "existing typed Research state",
        "research_update" => "supported typed Research facts",
        "tool_search" => "find Deferred tools",
        _ => "the exact Tool contract.",
    };
    let (do_not_use, prefer, produces) = match name {
        "read_file" => (
            "a batch or search is needed.",
            "line ranges.",
            "content, bytes and truncation.",
        ),
        "read_files" => (
            "one file or a text search is enough.",
            "a bounded independent batch.",
            "per-file content and failures.",
        ),
        "list_files" => (
            "content or text matches are needed.",
            "an explicit depth.",
            "paths and truncation.",
        ),
        "search_files" => (
            "file content or a command result is needed.",
            "narrow patterns.",
            "matches, bytes and truncation.",
        ),
        "write_file" => (
            "a patch or multi-file edit is needed.",
            "apply_patch for hunks.",
            "a hashed file mutation.",
        ),
        "apply_patch" => (
            "the patch is invalid or spans files.",
            "a verified preimage.",
            "a file mutation.",
        ),
        "run_command" => (
            "a batch or interactive terminal is needed.",
            "argv, not a shell string.",
            "output and truncation.",
        ),
        "run_commands" => (
            "one command or dependent ordering is needed.",
            "independent batches.",
            "ordered results and failures.",
        ),
        "start_job" => (
            "the command is short or terminal-interactive.",
            "long non-PTY work.",
            "Job ID and status.",
        ),
        "poll_job" => (
            "there is no Job ID or offset is stale.",
            "increasing offsets.",
            "status, content and truncation.",
        ),
        "input_job" => (
            "the process is a PTY or stdin is closed.",
            "one bounded write.",
            "Job ID and bytes.",
        ),
        "stop_job" => (
            "start or polling is needed.",
            "natural completion.",
            "terminal status.",
        ),
        "start_pty" => (
            "non-terminal work is sufficient.",
            "run_command or start_job.",
            "PTY ID and status.",
        ),
        "poll_pty" => (
            "there is no PTY ID or offset is stale.",
            "increasing offsets.",
            "status, content and truncation.",
        ),
        "input_pty" => (
            "the process is closed or not a PTY.",
            "one bounded input.",
            "PTY ID and bytes.",
        ),
        "resize_pty" => (
            "the PTY is closed or input is needed.",
            "positive dimensions.",
            "PTY ID and dimensions.",
        ),
        "stop_pty" => (
            "polling, resizing or input is needed.",
            "natural completion.",
            "terminal status.",
        ),
        "artifact_store" => (
            "text is transient or lacks source provenance.",
            "reproducible raw output.",
            "Blob and Artifact.",
        ),
        "view_image" => (
            "format validation or proof is needed.",
            "a local supported image.",
            "source and selected Artifacts.",
        ),
        "memory_read" => (
            "a mutation or verified Research fact is needed.",
            "smallest document.",
            "path, content and hash.",
        ),
        "memory_write" => (
            "there is no expected hash or content is transient.",
            "focused update.",
            "hashes, Blob and status.",
        ),
        "inspect_project" => (
            "file content or evidence is needed.",
            "changed paths.",
            "Snapshot metadata.",
        ),
        "research_get" => (
            "a mutation or transition is needed.",
            "the narrowest query.",
            "typed Research state.",
        ),
        "research_update" => (
            "the source is unbacked.",
            "typed facts with references.",
            "record status.",
        ),
        "tool_search" => (
            "a Tool is Hidden or already Direct.",
            "a narrow query.",
            "matching schemas.",
        ),
        _ => (
            "Do not use when a more specific Tool contract applies.",
            "Prefer the narrowest existing Tool and bounded arguments.",
            "Produces a typed Tool result and the corresponding execution Receipt.",
        ),
    };
    let strip_heading = |text: &str, heading: &str| {
        text.strip_prefix(heading)
            .unwrap_or(text)
            .trim_start()
            .to_owned()
    };
    format!(
        "Use when: {use_when}\nDo not use when: {}\nPrefer: {}\nProduces: {}",
        strip_heading(do_not_use, "Do not use when:"),
        strip_heading(prefer, "Prefer:"),
        strip_heading(produces, "Produces:"),
    )
}

fn cacheable(mut spec: ToolSpec) -> ToolSpec {
    spec.cache_policy = ToolCachePolicy::ContentAddressed;
    spec
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
            "properties": {
                "rerun_reason": {"type": "string", "minLength": 1}
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
