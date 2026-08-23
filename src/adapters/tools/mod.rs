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

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::adapters::project_event_store::ProjectEventStoreError;
use crate::adapters::tool_result_cache::LocalToolResultCache;
use crate::domain::events::StreamId;
use crate::domain::tools::{
    ExecutionId, ResourceKey, ToolCachePolicy, ToolCall, ToolDeterminism, ToolEffect,
    ToolExecutionOutcome, ToolExecutionPlan, ToolExposure, ToolExposurePlan, ToolName,
    ToolOutputPolicy, ToolParallelism, ToolRetryPolicy, ToolSpec, split_execution_control,
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
    exposure: Mutex<ToolExposureState>,
}

struct ToolExposureState {
    plan: ToolExposurePlan,
    discovered_deferred: BTreeSet<ToolName>,
}

impl ToolRegistry {
    pub fn for_project(project_root: impl Into<PathBuf>) -> Self {
        Self::for_project_with_exposure(project_root, ToolExposurePlan::default())
            .expect("the built-in Tool catalog accepts its empty exposure plan")
    }

    pub fn for_project_with_exposure(
        project_root: impl Into<PathBuf>,
        exposure_plan: ToolExposurePlan,
    ) -> Result<Self, ToolExecutorError> {
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
        validate_exposure_plan(&tools, &exposure_plan)?;
        Ok(Self {
            result_cache: None,
            project_root,
            tools,
            jobs: job::BackgroundJobs::new(),
            ptys: pty::PtySessions::new(),
            active_leases: ActiveProcessLeases::new(),
            exposure: Mutex::new(ToolExposureState {
                plan: exposure_plan,
                discovered_deferred: BTreeSet::new(),
            }),
        })
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

    fn configured_spec(&self, name: &ToolName, tool: BuiltinTool) -> ToolSpec {
        let exposure = self
            .exposure
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Self::configured_spec_for_state(name, tool, &exposure)
    }

    fn configured_spec_for_state(
        name: &ToolName,
        tool: BuiltinTool,
        exposure: &ToolExposureState,
    ) -> ToolSpec {
        let mut builtin_spec = tool.spec();
        builtin_spec.exposure = tool.default_exposure();
        let mut spec = exposure.plan.apply_to_spec(builtin_spec);
        let deferred_discovered =
            spec.exposure == ToolExposure::Deferred && exposure.discovered_deferred.contains(name);
        if deferred_discovered {
            spec.exposure = ToolExposure::Direct;
        }
        spec
    }

    fn all_specs(&self) -> Vec<ToolSpec> {
        let exposure = self
            .exposure
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.tools
            .iter()
            .map(|(name, tool)| Self::configured_spec_for_state(name, *tool, &exposure))
            .collect()
    }

    fn deferred_specs(&self) -> Vec<ToolSpec> {
        let exposure = self
            .exposure
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.tools
            .iter()
            .map(|(name, tool)| Self::configured_spec_for_state(name, *tool, &exposure))
            .filter(|spec| spec.exposure == ToolExposure::Deferred)
            .collect()
    }

    fn plan_with_exposure(
        &self,
        execution_id: ExecutionId,
        mut call: ToolCall,
        require_direct: bool,
    ) -> Result<ToolExecutionPlan, ToolExecutorError> {
        let tool = self
            .tools
            .get(&call.name)
            .ok_or_else(|| ToolExecutorError::UnknownTool {
                tool_name: call.name.to_string(),
            })?;
        let spec = self.configured_spec(&call.name, *tool);
        if require_direct && spec.exposure != ToolExposure::Direct {
            return Err(ToolExecutorError::ToolNotExposed {
                tool_name: call.name.to_string(),
            });
        }
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
            &spec,
            resource_keys,
            rerun_reason,
        )
        .map_err(|error| ToolExecutorError::PlanFailure {
            tool_name: spec.name.to_string(),
            message: error.to_string(),
        })?;
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
    fn specs(&self) -> Vec<ToolSpec> {
        self.all_specs()
            .into_iter()
            .filter(|spec| spec.exposure == ToolExposure::Direct)
            .collect()
    }

    fn plan(
        &self,
        execution_id: ExecutionId,
        call: ToolCall,
    ) -> Result<ToolExecutionPlan, ToolExecutorError> {
        self.plan_with_exposure(execution_id, call, false)
    }

    fn plan_exposed(
        &self,
        execution_id: ExecutionId,
        call: ToolCall,
    ) -> Result<ToolExecutionPlan, ToolExecutorError> {
        self.plan_with_exposure(execution_id, call, true)
    }

    fn configure_exposure(&self, plan: ToolExposurePlan) -> Result<(), ToolExecutorError> {
        validate_exposure_plan(&self.tools, &plan)?;
        let mut exposure = self
            .exposure
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if exposure.plan == plan {
            return Ok(());
        }
        exposure.plan = plan;
        exposure.discovered_deferred.clear();
        Ok(())
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
                let search = tool_search::execute(plan, &self.deferred_specs());
                if !search.activated.is_empty() {
                    self.exposure
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .discovered_deferred
                        .extend(search.activated);
                }
                return search.outcome;
            }
            let specs = if tool == BuiltinTool::InspectProject {
                self.specs()
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

fn validate_exposure_plan(
    tools: &BTreeMap<ToolName, BuiltinTool>,
    plan: &ToolExposurePlan,
) -> Result<(), ToolExecutorError> {
    if let Some(tool_name) = plan
        .overrides()
        .keys()
        .find(|tool_name| !tools.contains_key(*tool_name))
    {
        return Err(ToolExecutorError::UnknownTool {
            tool_name: tool_name.to_string(),
        });
    }
    Ok(())
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
            Self::MemoryRead => memory::execute_read(project_root, plan).await,
            Self::MemoryWrite => memory::execute_write(project_root, plan).await,
            Self::InspectProject => snapshot::execute(project_root, specs, plan).await,
            Self::ResearchGet => research::execute_get(project_root, plan).await,
            Self::ResearchUpdate => research::execute_update(project_root, plan).await,
            Self::ToolSearch => unreachable!("Tool search executes through ToolRegistry state"),
        }
    }

    fn spec(self) -> ToolSpec {
        let common_output = json!({"type": "object"});
        match self {
            Self::ReadFile => spec(
                "read_file",
                "Read one local file as bounded text.",
                json!({
                    "type": "object",
                    "properties": {"path": {"type": "string"}},
                    "required": ["path"],
                    "additionalProperties": false
                }),
                common_output,
                ToolParallelism::Parallel,
                Some("file:{path}"),
                ToolEffect::ReadOnly,
                ToolRetryPolicy::ReplaySafe,
                ToolDeterminism::EnvironmentDependent,
                10_000,
            ),
            Self::ReadFiles => spec(
                "read_files",
                "Read multiple local files as bounded text, preserving per-file failures.",
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
                common_output,
                ToolParallelism::Parallel,
                Some("file:{files[*].path}"),
                ToolEffect::ReadOnly,
                ToolRetryPolicy::ReplaySafe,
                ToolDeterminism::EnvironmentDependent,
                15_000,
            ),
            Self::ListFiles => spec(
                "list_files",
                "List local paths recursively with a bounded depth and output.",
                json!({
                    "type": "object",
                    "properties": {
                        "path": {"type": "string"},
                        "max_depth": {"type": "integer", "minimum": 0, "maximum": 64}
                    },
                    "additionalProperties": false
                }),
                common_output,
                ToolParallelism::Parallel,
                Some("file:{path}"),
                ToolEffect::ReadOnly,
                ToolRetryPolicy::ReplaySafe,
                ToolDeterminism::EnvironmentDependent,
                15_000,
            ),
            Self::SearchFiles => spec(
                "search_files",
                "Search local files with ripgrep and return bounded line matches.",
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
                        "paths": {"type": "array", "items": {"type": "string"}}
                    },
                    "additionalProperties": false
                }),
                common_output,
                ToolParallelism::Parallel,
                Some("file:{paths}"),
                ToolEffect::ReadOnly,
                ToolRetryPolicy::ReplaySafe,
                ToolDeterminism::EnvironmentDependent,
                30_000,
            ),
            Self::WriteFile => spec(
                "write_file",
                "Atomically replace one project-relative regular file after an exact preimage Hash check.",
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
                "Apply one confined unified-Diff file patch after full preimage and hunk validation.",
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
                "Run one argv array directly with FullAccess; no shell string is accepted.",
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
                common_output,
                ToolParallelism::ResourceLocked,
                Some("resource:{resources}"),
                ToolEffect::Interactive,
                ToolRetryPolicy::NeverAutoRetry,
                ToolDeterminism::Nondeterministic,
                120_000,
            ),
            Self::RunCommands => spec(
                "run_commands",
                "Run several argv arrays with an explicit serial or parallel mode; each result is retained.",
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
                common_output,
                ToolParallelism::ResourceLocked,
                Some("resource:{commands[*].resources}"),
                ToolEffect::Interactive,
                ToolRetryPolicy::NeverAutoRetry,
                ToolDeterminism::Nondeterministic,
                120_000,
            ),
            Self::StartJob => spec(
                "start_job",
                "Start one ordinary non-PTY argv process in the background and return its Job ID.",
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
                common_output,
                ToolParallelism::ResourceLocked,
                Some("resource:{resources}"),
                ToolEffect::Interactive,
                ToolRetryPolicy::DedupeByKey,
                ToolDeterminism::Nondeterministic,
                10_000,
            ),
            Self::PollJob => spec(
                "poll_job",
                "Read only new stdout and stderr bytes after caller-supplied offsets from one non-PTY Job.",
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
                common_output,
                ToolParallelism::ResourceLocked,
                Some("job:{job_id}"),
                ToolEffect::ReadOnly,
                ToolRetryPolicy::ReplaySafe,
                ToolDeterminism::EnvironmentDependent,
                10_000,
            ),
            Self::InputJob => spec(
                "input_job",
                "Write input to one running non-PTY Job; set close to close its stdin after writing.",
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
                common_output,
                ToolParallelism::ResourceLocked,
                Some("job:{job_id}"),
                ToolEffect::Interactive,
                ToolRetryPolicy::NeverAutoRetry,
                ToolDeterminism::Nondeterministic,
                10_000,
            ),
            Self::StopJob => spec(
                "stop_job",
                "Stop and reap one running non-PTY Job, then return its stable terminal completion Receipt.",
                json!({
                    "type": "object",
                    "properties": {
                        "job_id": {"type": "string", "minLength": 1}
                    },
                    "required": ["job_id"],
                    "additionalProperties": false
                }),
                common_output,
                ToolParallelism::ResourceLocked,
                Some("job:{job_id}"),
                ToolEffect::Interactive,
                ToolRetryPolicy::NeverAutoRetry,
                ToolDeterminism::Nondeterministic,
                10_000,
            ),
            Self::StartPty => spec(
                "start_pty",
                "Start one current-platform PTY argv process and return its PTY Session ID.",
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
                common_output,
                ToolParallelism::ResourceLocked,
                Some("resource:{resources}"),
                ToolEffect::Interactive,
                ToolRetryPolicy::DedupeByKey,
                ToolDeterminism::Nondeterministic,
                10_000,
            ),
            Self::PollPty => spec(
                "poll_pty",
                "Read only newly captured PTY bytes after a caller-supplied offset.",
                json!({
                    "type": "object",
                    "properties": {
                        "pty_session_id": {"type": "string", "minLength": 1},
                        "offset": {"type": "integer", "minimum": 0}
                    },
                    "required": ["pty_session_id"],
                    "additionalProperties": false
                }),
                common_output,
                ToolParallelism::ResourceLocked,
                Some("pty:{pty_session_id}"),
                ToolEffect::ReadOnly,
                ToolRetryPolicy::ReplaySafe,
                ToolDeterminism::EnvironmentDependent,
                10_000,
            ),
            Self::InputPty => spec(
                "input_pty",
                "Write input bytes to one running PTY session.",
                json!({
                    "type": "object",
                    "properties": {
                        "pty_session_id": {"type": "string", "minLength": 1},
                        "input": {"type": "string"}
                    },
                    "required": ["pty_session_id", "input"],
                    "additionalProperties": false
                }),
                common_output,
                ToolParallelism::ResourceLocked,
                Some("pty:{pty_session_id}"),
                ToolEffect::Interactive,
                ToolRetryPolicy::NeverAutoRetry,
                ToolDeterminism::Nondeterministic,
                10_000,
            ),
            Self::ResizePty => spec(
                "resize_pty",
                "Resize one running PTY terminal on the current platform.",
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
                common_output,
                ToolParallelism::ResourceLocked,
                Some("pty:{pty_session_id}"),
                ToolEffect::Interactive,
                ToolRetryPolicy::NeverAutoRetry,
                ToolDeterminism::Nondeterministic,
                10_000,
            ),
            Self::StopPty => spec(
                "stop_pty",
                "Stop and reap one PTY direct child, then return its stable terminal completion.",
                json!({
                    "type": "object",
                    "properties": {
                        "pty_session_id": {"type": "string", "minLength": 1}
                    },
                    "required": ["pty_session_id"],
                    "additionalProperties": false
                }),
                common_output,
                ToolParallelism::ResourceLocked,
                Some("pty:{pty_session_id}"),
                ToolEffect::Interactive,
                ToolRetryPolicy::NeverAutoRetry,
                ToolDeterminism::Nondeterministic,
                10_000,
            ),
            Self::ArtifactStore => spec(
                "artifact_store",
                "Store one local regular file as a SHA-256 Blob and append a provenance-preserving Artifact fact.",
                artifact::input_schema(),
                artifact::output_schema(),
                ToolParallelism::ResourceLocked,
                Some("file:{source_path},file:.cyber-agent/research.jsonl"),
                ToolEffect::LocalMutation,
                ToolRetryPolicy::NeverAutoRetry,
                ToolDeterminism::EnvironmentDependent,
                120_000,
            ),
            Self::MemoryRead => spec(
                "memory_read",
                "Read one project Markdown Memory document and audit any externally edited revision.",
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
                "Replace one project Markdown Memory document with expected-Hash protection and an audited Diff.",
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
                "Build or reuse the project-isolated metadata Snapshot, File Manifest, and fixed Tool Capability Cache.",
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
                "Read and strictly reduce the current project Research Event stream.",
                research::get_input_schema(),
                research::output_schema(),
                ToolParallelism::Parallel,
                Some("file:.cyber-agent/research.jsonl"),
                ToolEffect::ReadOnly,
                ToolRetryPolicy::ReplaySafe,
                ToolDeterminism::Deterministic,
                10_000,
            )),
            Self::ResearchUpdate => spec(
                "research_update",
                "Append one versioned Research record, update, or explicit merge after strict replay validation.",
                research::update_input_schema(),
                research::output_schema(),
                ToolParallelism::ResourceLocked,
                Some("file:.cyber-agent/research.jsonl"),
                ToolEffect::LocalMutation,
                ToolRetryPolicy::NeverAutoRetry,
                ToolDeterminism::EnvironmentDependent,
                10_000,
            ),
            Self::ToolSearch => spec(
                "tool_search",
                "Discover matching currently Deferred Tools by name or description and expose their schemas for this Agent session.",
                tool_search::input_schema(),
                common_output,
                ToolParallelism::Parallel,
                None,
                ToolEffect::ReadOnly,
                ToolRetryPolicy::ReplaySafe,
                ToolDeterminism::Deterministic,
                5_000,
            ),
        }
    }

    fn cache_resource_fingerprint(self, project_root: &Path, arguments: &Value) -> Option<String> {
        match self {
            Self::ResearchGet => research::cache_resource_fingerprint(project_root, arguments),
            _ => None,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn spec(
    name: &str,
    description: &str,
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
        description: description.to_owned(),
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
