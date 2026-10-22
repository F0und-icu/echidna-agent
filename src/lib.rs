use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use serde::Serialize;
use thiserror::Error;

use crate::adapters::project_event_store::{Project, ProjectEventStoreError};
use crate::app::doctor::{AgentProfileDoctorReport, inspect_agent_model_profile};
use crate::domain::domain_profile::DomainProfile;
use crate::domain::research_control::ResearchPhase;
use crate::provider_composition::{ProviderComposition, ProviderCompositionError};

pub mod adapters;
pub mod app;
pub mod domain;
pub mod ports;
pub mod provider_composition;
pub mod trace_eval_runtime;

mod cli_agent;
mod cli_auth;
mod cli_chat;
mod cli_inspection;
mod cli_session;
mod terminal_content;

#[derive(Debug, Parser)]
#[command(
    name = "echidna",
    version,
    long_version = concat!(env!("CARGO_PKG_VERSION"), "\nsource base: ", env!("ECHIDNA_SOURCE_REVISION")),
    about = "Echidna — Local Agent with editable task definitions"
)]
pub struct Cli {
    /// Load the explicit Echidna Provider configuration from this path.
    #[arg(long, global = true, value_name = "PATH")]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<CliCommand>,
}

#[derive(Debug, Subcommand)]
enum CliCommand {
    /// Initialize the current directory as an Echidna project.
    Init,
    /// Start an interactive research session.
    Chat {
        /// Select a local Agent definition directory or agent.toml for a new Session.
        #[arg(long, value_name = "PATH", conflicts_with = "session")]
        agent: Option<PathBuf>,
        /// Show sanitized sent prompts and model content, without SSE envelopes (development builds only).
        #[cfg(debug_assertions)]
        #[arg(long)]
        debug: bool,
        /// Resume one durable Session ID, or `latest`; `--session` is kept as an alias.
        #[arg(
            long = "resume",
            visible_alias = "session",
            value_name = "SESSION_ID|latest"
        )]
        session: Option<String>,
        /// Explicit target domain. Omit on Resume to restore the durable selection.
        #[arg(long, value_enum)]
        domain: Option<CliDomainProfile>,
        /// Explicit initial research objective. Omit to use or restore Mapping.
        #[arg(long, value_enum)]
        phase: Option<CliResearchPhase>,
    },
    /// Inspect local configuration and runtime readiness.
    Doctor,
    /// Manage the explicitly configured Echidna subscription account.
    Auth {
        #[command(subcommand)]
        command: cli_auth::AuthCommand,
    },
    /// Create, validate, or inspect an editable Agent definition without a model request.
    Agent {
        #[command(subcommand)]
        command: cli_agent::AgentCommand,
    },
    /// Inspect the most recently committed audited request.
    Prompt {
        #[command(subcommand)]
        inspection: PromptCommand,
    },
    /// Inspect the existing Project Snapshot cache without rebuilding it.
    Snapshot,
    /// Display local Session and project metrics.
    Metrics {
        /// Inspect one exact Session directory identity.
        #[arg(long)]
        session: Option<String>,
    },
    /// Inspect locally available derived state.
    Inspect {
        #[command(subcommand)]
        inspection: InspectCommand,
    },
    /// Inspect or end durable local Sessions without starting a model runtime.
    Session {
        #[command(subcommand)]
        command: SessionCommand,
    },
}

#[derive(Debug, Subcommand)]
enum SessionCommand {
    /// List every strictly replayable Session in the current Project.
    List,
    /// Show one strictly replayed Session summary.
    Show {
        #[arg(value_name = "SESSION_ID")]
        session: String,
    },
    /// Durably end one Active Session.
    End {
        #[arg(value_name = "SESSION_ID")]
        session: String,
    },
    /// Export one Session transcript and its durable references.
    Export {
        #[arg(value_name = "SESSION_ID")]
        session: String,
        #[arg(long, value_enum, default_value_t = CliSessionExportFormat::Markdown)]
        format: CliSessionExportFormat,
        /// Write a new file instead of stdout; an existing path is never overwritten.
        #[arg(long, value_name = "PATH")]
        output: Option<PathBuf>,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum CliSessionExportFormat {
    Markdown,
    Json,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum CliDomainProfile {
    Source,
    Web,
    Mobile,
    Binary,
}

impl From<CliDomainProfile> for DomainProfile {
    fn from(value: CliDomainProfile) -> Self {
        match value {
            CliDomainProfile::Source => Self::Source,
            CliDomainProfile::Web => Self::Web,
            CliDomainProfile::Mobile => Self::Mobile,
            CliDomainProfile::Binary => Self::Binary,
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum CliResearchPhase {
    Mapping,
    Hypothesis,
    Experiment,
    Verification,
    EvidenceCuration,
}

impl From<CliResearchPhase> for ResearchPhase {
    fn from(value: CliResearchPhase) -> Self {
        match value {
            CliResearchPhase::Mapping => Self::Mapping,
            CliResearchPhase::Hypothesis => Self::Hypothesis,
            CliResearchPhase::Experiment => Self::Experiment,
            CliResearchPhase::Verification => Self::Verification,
            CliResearchPhase::EvidenceCuration => Self::EvidenceCuration,
        }
    }
}

#[derive(Debug, Subcommand)]
enum InspectCommand {
    /// Inspect the most recently successful Context compilation.
    Context,
    /// Validate and render one externally produced AndroidManifest Observation Artifact.
    AndroidManifest {
        /// Session containing the source run_commands Call, Result, and Receipt.
        #[arg(long)]
        session: String,
        /// Raw Artifact containing the strict Observation batch JSON.
        #[arg(long)]
        artifact: String,
    },
    /// Compare two validated AndroidManifest Observation Artifacts.
    TargetDelta {
        /// Session containing the baseline external Extractor execution.
        #[arg(long)]
        baseline_session: String,
        /// Raw baseline Artifact containing strict Observation batch JSON.
        #[arg(long)]
        baseline_artifact: String,
        /// Session containing the candidate external Extractor execution.
        #[arg(long)]
        candidate_session: String,
        /// Raw candidate Artifact containing strict Observation batch JSON.
        #[arg(long)]
        candidate_artifact: String,
    },
    /// Run the deterministic local security-retrieval Shadow without changing Context or facts.
    Retrieval {
        /// The local query to rank against bounded project facts.
        #[arg(long)]
        query: String,
        /// The exact current TargetSnapshot ID used for eligibility filtering.
        #[arg(long = "target-snapshot")]
        target_snapshot: String,
        /// One explicitly selected Session whose derived historical Summary may be considered.
        #[arg(long = "historical-session", value_name = "SESSION_ID", num_args = 0..=16)]
        historical_sessions: Vec<String>,
        /// Permit an explicitly selected historical Summary bound to a non-current TargetSnapshot.
        #[arg(long)]
        historical_comparison: bool,
    },
    /// Rank existing typed NextActions and optional controlled typed candidates without execution.
    Guidance {
        /// The exact current TargetSnapshot ID used for applicability filtering.
        #[arg(long = "target-snapshot")]
        target_snapshot: String,
        /// Strict bounded JSON document containing caller-supplied controlled Eval candidates.
        #[arg(long = "eval-candidates", value_name = "PATH")]
        eval_candidates: Option<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
enum PromptCommand {
    /// Inspect the most recently committed audited request.
    Inspect {
        /// Inspect one exact Session directory identity.
        #[arg(long)]
        session: Option<String>,
    },
}

#[derive(Debug, Error)]
pub enum CliError {
    #[error("Agent definition: {message}")]
    AgentDefinition { message: String },
    #[error("failed to render CLI help: {0}")]
    Help(#[from] io::Error),
    #[error("failed to encode diagnostic output: {0}")]
    Json(#[from] serde_json::Error),
    #[error("failed to resolve the current working directory: {0}")]
    WorkingDirectory(#[source] io::Error),
    #[error(transparent)]
    Project(#[from] ProjectEventStoreError),
    #[error(transparent)]
    ProviderComposition(#[from] ProviderCompositionError),
    #[error("chat requires one explicit --config <path> Provider configuration")]
    ChatConfigurationRequired,
    #[error("auth requires one explicit --config <path> Provider configuration")]
    AuthConfigurationRequired,
    #[error("failed to build the local authentication runtime: {0}")]
    AuthRuntime(#[source] io::Error),
    #[error("subscription authentication failed: {message}")]
    SubscriptionAuthentication { message: String },
    #[error("model selection failed: {message}")]
    ModelSelection { message: String },
    #[error("failed to build the local chat runtime: {0}")]
    ChatRuntime(#[source] io::Error),
    #[error("chat failed: {message}")]
    Chat { message: String },
    #[error("Session `{session_id}` does not exist in the current project")]
    SessionNotFound { session_id: String },
    #[error("Session operation failed: {message}")]
    SessionOperation { message: String },
    #[error(
        "the current project has {session_count} Sessions; select one explicitly with `--session <session-id>`"
    )]
    SessionSelectionRequired { session_count: usize },
    #[error("AndroidManifest Observation ingestion failed: {message}")]
    Observation { message: String },
    #[error("deterministic retrieval Shadow inspection failed: {message}")]
    Retrieval { message: String },
    #[error("deterministic evidence-gap guidance Shadow inspection failed: {message}")]
    Guidance { message: String },
}

#[derive(Debug, Serialize)]
struct Diagnostic {
    code: &'static str,
    severity: &'static str,
    message: &'static str,
}

#[derive(Debug, Serialize)]
struct DoctorReport {
    command: &'static str,
    status: &'static str,
    diagnostics: Vec<Diagnostic>,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_profile: Option<AgentProfileDoctorReport>,
}

#[derive(Debug, Serialize)]
struct ShellResponse {
    command: &'static str,
    status: &'static str,
    message: &'static str,
}

#[derive(Debug, Serialize)]
struct InitResponse {
    command: &'static str,
    status: &'static str,
    project_id: String,
    project_root: String,
}

fn doctor_report(composition: Option<&ProviderComposition>) -> DoctorReport {
    match composition {
        Some(composition) => DoctorReport {
            command: "doctor",
            status: "ready",
            diagnostics: Vec::new(),
            agent_profile: Some(inspect_agent_model_profile(composition.agent_profile())),
        },
        None => DoctorReport {
            command: "doctor",
            status: "not_ready",
            diagnostics: vec![
                Diagnostic {
                    code: "PROJECT_NOT_INITIALIZED",
                    severity: "info",
                    message: "no .echidna/project.toml or legacy .cyber-agent/project.toml is configured for this project",
                },
                Diagnostic {
                    code: "PROVIDER_NOT_CONFIGURED",
                    severity: "info",
                    message: "no explicit Provider configuration path was selected",
                },
            ],
            agent_profile: None,
        },
    }
}

fn load_provider_composition(
    path: Option<&Path>,
) -> Result<Option<ProviderComposition>, ProviderCompositionError> {
    path.map(|path| {
        ProviderComposition::load(
            path,
            Arc::new(adapters::credentials::ProcessEnvironmentCredentialSource),
        )
    })
    .transpose()
}

fn print_json<T: Serialize>(value: &T) -> Result<(), CliError> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn print_no_active_context() -> Result<(), CliError> {
    print_json(&ShellResponse {
        command: "inspect context",
        status: "no_active_context",
        message: "no active AgentLoop Context is available in this standalone process; RequestAudit persistence is deferred",
    })
}

fn initialize_current_project() -> Result<(), CliError> {
    let root = std::env::current_dir().map_err(CliError::WorkingDirectory)?;
    let project = Project::initialize(&root)?;
    print_json(&InitResponse {
        command: "init",
        status: "initialized",
        project_id: project.project_id().as_str().to_owned(),
        project_root: project.root().display().to_string(),
    })
}

fn discover_current_project() -> Result<Project, CliError> {
    let root = std::env::current_dir().map_err(CliError::WorkingDirectory)?;
    Project::discover(&root).map_err(CliError::from)
}

fn run_chat(
    config: Option<&Path>,
    agent: Option<PathBuf>,
    session: Option<String>,
    domain: Option<CliDomainProfile>,
    phase: Option<CliResearchPhase>,
    #[cfg(debug_assertions)] debug: bool,
) -> Result<(), CliError> {
    let config = config.ok_or(CliError::ChatConfigurationRequired)?;
    let config = ProviderComposition::load_config(config)?;
    let project = discover_current_project()?;
    let session = cli_session::resolve_resume_target(&project, session.as_deref())?;
    let composition = ProviderComposition::compose_for_project(
        config,
        Arc::new(adapters::credentials::ProcessEnvironmentCredentialSource),
        project.clone(),
        #[cfg(debug_assertions)]
        debug,
    )?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .map_err(CliError::ChatRuntime)?;
    runtime
        .block_on(cli_chat::run(
            project,
            &composition,
            agent,
            session,
            domain.map(DomainProfile::from),
            phase.map(ResearchPhase::from),
        ))
        .map_err(|error| CliError::Chat {
            message: error.to_string(),
        })
}

pub fn run<I, T>(args: I) -> Result<(), CliError>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = Cli::parse_from(args);

    match cli.command {
        Some(CliCommand::Init) => initialize_current_project(),
        Some(CliCommand::Chat {
            agent,
            session,
            domain,
            phase,
            #[cfg(debug_assertions)]
            debug,
        }) => run_chat(
            cli.config.as_deref(),
            agent,
            session,
            domain,
            phase,
            #[cfg(debug_assertions)]
            debug,
        ),
        Some(CliCommand::Doctor) => {
            let composition = load_provider_composition(cli.config.as_deref())?;
            print_json(&doctor_report(composition.as_ref()))
        }
        Some(CliCommand::Auth { command }) => cli_auth::run(cli.config.as_deref(), command),
        Some(CliCommand::Agent { command }) => cli_agent::run(cli.config.as_deref(), command),
        Some(CliCommand::Prompt {
            inspection: PromptCommand::Inspect { session },
        }) => cli_inspection::print_prompt_inspection(session.as_deref()),
        Some(CliCommand::Snapshot) => cli_inspection::print_snapshot_inspection(),
        Some(CliCommand::Metrics { session }) => cli_inspection::print_metrics(session.as_deref()),
        Some(CliCommand::Inspect {
            inspection: InspectCommand::Context,
        }) => print_no_active_context(),
        Some(CliCommand::Inspect {
            inspection: InspectCommand::AndroidManifest { session, artifact },
        }) => cli_inspection::print_android_manifest_observations(&session, &artifact),
        Some(CliCommand::Inspect {
            inspection:
                InspectCommand::TargetDelta {
                    baseline_session,
                    baseline_artifact,
                    candidate_session,
                    candidate_artifact,
                },
        }) => cli_inspection::print_android_manifest_target_delta(
            &baseline_session,
            &baseline_artifact,
            &candidate_session,
            &candidate_artifact,
        ),
        Some(CliCommand::Inspect {
            inspection:
                InspectCommand::Retrieval {
                    query,
                    target_snapshot,
                    historical_sessions,
                    historical_comparison,
                },
        }) => cli_inspection::print_retrieval_shadow(
            &query,
            &target_snapshot,
            &historical_sessions,
            historical_comparison,
        ),
        Some(CliCommand::Inspect {
            inspection:
                InspectCommand::Guidance {
                    target_snapshot,
                    eval_candidates,
                },
        }) => cli_inspection::print_guidance_shadow(&target_snapshot, eval_candidates.as_deref()),
        Some(CliCommand::Session { command }) => match command {
            SessionCommand::List => cli_session::print_list(),
            SessionCommand::Show { session } => cli_session::print_show(&session),
            SessionCommand::End { session } => cli_session::end(&session),
            SessionCommand::Export {
                session,
                format,
                output,
            } => cli_session::export(
                &session,
                match format {
                    CliSessionExportFormat::Markdown => cli_session::ExportFormat::Markdown,
                    CliSessionExportFormat::Json => cli_session::ExportFormat::Json,
                },
                output.as_deref(),
            ),
        },
        None => {
            let mut command = Cli::command();
            command.print_help()?;
            println!();
            Ok(())
        }
    }
}
