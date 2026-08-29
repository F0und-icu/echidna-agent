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

mod cli_chat;
mod cli_inspection;

#[derive(Debug, Parser)]
#[command(
    name = "cyber-agent",
    version,
    about = "Local-first vulnerability research agent"
)]
pub struct Cli {
    /// Load the explicit Cyber Agent Provider configuration from this path.
    #[arg(long, global = true, value_name = "PATH")]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<CliCommand>,
}

#[derive(Debug, Subcommand)]
enum CliCommand {
    /// Initialize the current directory as a Cyber Agent project.
    Init,
    /// Start an interactive research session.
    Chat {
        /// Explicitly resume this durable Session instead of starting a new one.
        #[arg(long, value_name = "SESSION_ID")]
        session: Option<String>,
        /// Fixed vulnerability-research target domain for this Chat process.
        #[arg(long, value_enum, default_value_t = CliDomainProfile::Source)]
        domain: CliDomainProfile,
        /// Explicit initial research objective. Omit to use or restore Mapping.
        #[arg(long, value_enum)]
        phase: Option<CliResearchPhase>,
    },
    /// Inspect local configuration and runtime readiness.
    Doctor,
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
    #[error("failed to build the local chat runtime: {0}")]
    ChatRuntime(#[source] io::Error),
    #[error("chat failed: {message}")]
    Chat { message: String },
    #[error("Session `{session_id}` does not exist in the current project")]
    SessionNotFound { session_id: String },
    #[error(
        "the current project has {session_count} Sessions; select one explicitly with `--session <session-id>`"
    )]
    SessionSelectionRequired { session_count: usize },
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
                    message: "no .cyber-agent/project.toml is configured for this project",
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
    session: Option<String>,
    domain: CliDomainProfile,
    phase: Option<CliResearchPhase>,
) -> Result<(), CliError> {
    let config = config.ok_or(CliError::ChatConfigurationRequired)?;
    let composition = load_provider_composition(Some(config))?
        .expect("an explicit chat configuration always composes one Provider");
    let project = discover_current_project()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .map_err(CliError::ChatRuntime)?;
    runtime
        .block_on(cli_chat::run(
            project,
            &composition,
            session,
            domain.into(),
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
            session,
            domain,
            phase,
        }) => run_chat(cli.config.as_deref(), session, domain, phase),
        Some(CliCommand::Doctor) => {
            let composition = load_provider_composition(cli.config.as_deref())?;
            print_json(&doctor_report(composition.as_ref()))
        }
        Some(CliCommand::Prompt {
            inspection: PromptCommand::Inspect { session },
        }) => cli_inspection::print_prompt_inspection(session.as_deref()),
        Some(CliCommand::Snapshot) => cli_inspection::print_snapshot_inspection(),
        Some(CliCommand::Metrics { session }) => cli_inspection::print_metrics(session.as_deref()),
        Some(CliCommand::Inspect {
            inspection: InspectCommand::Context,
        }) => print_no_active_context(),
        None => {
            let mut command = Cli::command();
            command.print_help()?;
            println!();
            Ok(())
        }
    }
}
