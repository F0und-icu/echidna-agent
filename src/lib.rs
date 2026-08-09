use std::ffi::OsString;
use std::io;

use clap::{CommandFactory, Parser, Subcommand};
use serde::Serialize;
use thiserror::Error;

pub mod adapters;
pub mod app;
pub mod domain;
pub mod ports;

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
}

#[derive(Debug, Error)]
pub enum CliError {
    #[error("failed to render CLI help: {0}")]
    Help(#[from] io::Error),
    #[error("failed to encode diagnostic output: {0}")]
    Json(#[from] serde_json::Error),
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

pub fn run<I, T>(args: I) -> Result<(), CliError>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = Cli::parse_from(args);

    match cli.command {
        Some(CliCommand::Init) => print_shell_response(
            "init",
            "project initialization is reserved for a later implementation slice",
        ),
        Some(CliCommand::Chat) => print_shell_response(
            "chat",
            "interactive chat is reserved for a later implementation slice",
        ),
        Some(CliCommand::Doctor) => print_json(&doctor_report()),
        None => {
            let mut command = Cli::command();
            command.print_help()?;
            println!();
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::doctor_report;

    #[test]
    fn doctor_report_is_structured_without_configuration() {
        let report = doctor_report();

        assert_eq!(report.command, "doctor");
        assert_eq!(report.status, "not_ready");
        assert_eq!(report.diagnostics.len(), 2);
        assert_eq!(report.diagnostics[0].code, "PROJECT_NOT_INITIALIZED");
        assert_eq!(report.diagnostics[1].code, "PROVIDER_NOT_CONFIGURED");
    }
}
