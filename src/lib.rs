use std::ffi::OsString;
use std::io;

use clap::{CommandFactory, Parser, Subcommand};
use serde::Serialize;
use thiserror::Error;

use crate::adapters::project_event_store::{Project, ProjectEventStoreError};

pub mod adapters;
pub mod app;
pub mod domain;
pub mod ports;
pub mod trace_eval_runtime;

mod cli_inspection;

#[derive(Debug, Parser)]
#[command(
    name = "cyber-agent",
    version,
    about = "Local-first vulnerability research agent"
)]
pub struct Cli {
    #[command(subcommand)]
    command: Option<CliCommand>,
}

#[derive(Debug, Subcommand)]
enum CliCommand {
    /// Initialize the current directory as a Cyber Agent project.
    Init,
    /// Start an interactive research session.
    Chat,
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

fn doctor_report() -> DoctorReport {
    DoctorReport {
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
                message: "no Cyber Agent ModelProvider profile is configured",
            },
        ],
    }
}

fn print_json<T: Serialize>(value: &T) -> Result<(), CliError> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn print_shell_response(command: &'static str, message: &'static str) -> Result<(), CliError> {
    print_json(&ShellResponse {
        command,
        status: "shell_only",
        message,
    })
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

pub fn run<I, T>(args: I) -> Result<(), CliError>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = Cli::parse_from(args);

    match cli.command {
        Some(CliCommand::Init) => initialize_current_project(),
        Some(CliCommand::Chat) => print_shell_response(
            "chat",
            "interactive chat is reserved for a later implementation slice",
        ),
        Some(CliCommand::Doctor) => print_json(&doctor_report()),
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
