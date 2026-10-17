//! Explicit account operations outside a research Project or AgentLoop.

use std::io::{self, BufRead, IsTerminal, Write};
use std::path::Path;
use std::str::FromStr;

use clap::Subcommand;
use serde::Serialize;

use crate::adapters::subscription_auth::{
    SubscriptionAuthProtocol, SubscriptionModel, SubscriptionReasoningLevel,
};
use crate::domain::model::ReasoningEffort;
use crate::provider_composition::ProviderComposition;
use crate::terminal_content::escaped_text;
use crate::{CliError, print_json};

#[derive(Debug, Subcommand)]
pub(crate) enum AuthCommand {
    /// Sign in using Echidna's own browser OAuth flow.
    Login {
        #[arg(long, value_name = "PROVIDER_ID")]
        provider: String,
        /// Print the authorization URL without launching a browser.
        #[arg(long)]
        no_browser: bool,
    },
    /// Inspect local account status without refresh or network access.
    Status {
        #[arg(long, value_name = "PROVIDER_ID")]
        provider: String,
    },
    /// Refresh the selected Codex subscription grant without a model request.
    Refresh {
        #[arg(long, value_name = "PROVIDER_ID")]
        provider: String,
    },
    /// Revoke the selected grant and remove local tokens.
    Logout {
        #[arg(long, value_name = "PROVIDER_ID")]
        provider: String,
    },
    /// Fetch models available to the selected account; does not infer capabilities.
    Models {
        #[arg(long, value_name = "PROVIDER_ID")]
        provider: String,
    },
    /// Select one currently advertised Codex model and optional reasoning effort.
    SelectModel {
        #[arg(long, value_name = "PROVIDER_ID")]
        provider: String,
        /// A slug from the catalog fetched for this command.
        #[arg(long, value_name = "MODEL_SLUG")]
        model: Option<String>,
        /// A supported, explicitly selectable effort from that same catalog.
        #[arg(long, value_name = "EFFORT")]
        reasoning_effort: Option<String>,
    },
}

pub(crate) fn run(config: Option<&Path>, command: AuthCommand) -> Result<(), CliError> {
    let config = config
        .ok_or(CliError::AuthConfigurationRequired)?
        .to_path_buf();
    let provider = match &command {
        AuthCommand::Login { provider, .. }
        | AuthCommand::Status { provider }
        | AuthCommand::Refresh { provider }
        | AuthCommand::Logout { provider }
        | AuthCommand::Models { provider }
        | AuthCommand::SelectModel { provider, .. } => provider.clone(),
    };
    let service = ProviderComposition::subscription_auth(&config, &provider)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .map_err(CliError::AuthRuntime)?;
    runtime.block_on(async {
        match command {
            AuthCommand::Login { no_browser, .. } => {
                let present = |url: &str| {
                    eprintln!("Open this URL to authorize Echidna:\n{url}");
                    if !no_browser {
                        open_browser(url);
                    }
                    Ok(())
                };
                match service.protocol() {
                    SubscriptionAuthProtocol::SiwcDirect => {
                        print_json(&service.login(present).await.map_err(auth_error)?)
                    }
                    SubscriptionAuthProtocol::Codex => {
                        print_json(&service.login_codex(present).await.map_err(auth_error)?)
                    }
                }
            }
            AuthCommand::Status { .. } => match service.protocol() {
                SubscriptionAuthProtocol::SiwcDirect => {
                    print_json(&service.status().await.map_err(auth_error)?)
                }
                SubscriptionAuthProtocol::Codex => {
                    print_json(&service.codex_status().await.map_err(auth_error)?)
                }
            },
            AuthCommand::Refresh { .. } => {
                print_json(&service.refresh_codex().await.map_err(auth_error)?)
            }
            AuthCommand::Logout { .. } => print_json(&service.logout().await.map_err(auth_error)?),
            AuthCommand::Models { .. } => print_json(&service.models().await.map_err(auth_error)?),
            AuthCommand::SelectModel {
                model,
                reasoning_effort,
                ..
            } => {
                if service.protocol() != SubscriptionAuthProtocol::Codex {
                    return Err(selection_error(
                        "model selection requires a Codex subscription Provider",
                    ));
                }
                let catalog = service.models().await.map_err(auth_error)?;
                let (model, effort) =
                    select_catalog_model(&catalog, model.as_deref(), reasoning_effort.as_deref())?;
                ProviderComposition::select_codex_model(&config, &provider, &model.slug, effort)
                    .map_err(|error| selection_error(&error.to_string()))?;
                print_json(&SavedModelSelection {
                    provider,
                    model: model.slug.clone(),
                    reasoning_effort: effort.map(ReasoningEffort::as_str),
                })
            }
        }
    })
}

#[derive(Serialize)]
struct SavedModelSelection {
    provider: String,
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<&'static str>,
}

fn select_catalog_model<'a>(
    catalog: &'a [SubscriptionModel],
    requested_model: Option<&str>,
    requested_effort: Option<&str>,
) -> Result<(&'a SubscriptionModel, Option<ReasoningEffort>), CliError> {
    if catalog.is_empty() {
        return Err(selection_error("the current Codex catalog is empty"));
    }
    let model_slug = match requested_model {
        Some(model) => model.to_owned(),
        None => prompt_model(catalog)?,
    };
    let model = catalog
        .iter()
        .find(|candidate| candidate.slug == model_slug)
        .ok_or_else(|| selection_error("the requested model is not in the current catalog"))?;
    let effort = match requested_effort {
        Some(effort) => Some(parse_supported_effort(model, effort)?),
        None if requested_model.is_none() => prompt_effort(model)?,
        None => None,
    };
    Ok((model, effort))
}

fn prompt_model(catalog: &[SubscriptionModel]) -> Result<String, CliError> {
    require_terminal()?;
    eprintln!("Available Codex models:");
    for (index, model) in catalog.iter().enumerate() {
        eprintln!(
            "  {}. {} — {}",
            index + 1,
            escaped_text(&model.slug),
            escaped_text(&model.display_name)
        );
    }
    let choice = read_choice("Model number or slug: ")?;
    select_model_choice(catalog, &choice)
}

pub(crate) fn select_model_choice(
    catalog: &[SubscriptionModel],
    choice: &str,
) -> Result<String, CliError> {
    if catalog.iter().any(|model| model.slug == choice) {
        return Ok(choice.to_owned());
    }
    let index = choice
        .parse::<usize>()
        .map_err(|_| selection_error("the requested model is not in the current catalog"))?;
    catalog
        .get(index.checked_sub(1).ok_or_else(|| {
            selection_error("the selected model number is outside the current catalog")
        })?)
        .map(|model| model.slug.clone())
        .ok_or_else(|| selection_error("the selected model number is outside the current catalog"))
}

fn prompt_effort(model: &SubscriptionModel) -> Result<Option<ReasoningEffort>, CliError> {
    require_terminal()?;
    eprintln!(
        "Supported reasoning efforts for {}:",
        escaped_text(&model.slug)
    );
    eprintln!("  0. Service default");
    if model.supported_reasoning_levels.is_empty() {
        eprintln!("  (none advertised)");
    } else {
        for (index, level) in model.supported_reasoning_levels.iter().enumerate() {
            print_reasoning_level(index + 1, level);
        }
    }
    let choice =
        read_choice_or_empty("Reasoning effort number or name (0 keeps the service default): ")?;
    select_effort_choice(model, choice.as_deref())
}

pub(crate) fn select_effort_choice(
    model: &SubscriptionModel,
    choice: Option<&str>,
) -> Result<Option<ReasoningEffort>, CliError> {
    let Some(choice) = choice else {
        return Ok(None);
    };
    if choice == "0" {
        return Ok(None);
    }
    match choice.parse::<usize>() {
        Ok(index) => model
            .supported_reasoning_levels
            .get(index.checked_sub(1).ok_or_else(|| {
                selection_error(
                    "the selected reasoning effort number is outside the current catalog",
                )
            })?)
            .map(|level| parse_supported_effort(model, &level.effort))
            .transpose()?
            .ok_or_else(|| {
                selection_error(
                    "the selected reasoning effort number is outside the current catalog",
                )
            })
            .map(Some),
        Err(_) => parse_supported_effort(model, choice).map(Some),
    }
}

fn print_reasoning_level(index: usize, level: &SubscriptionReasoningLevel) {
    eprintln!(
        "  {}. {} — {}",
        index,
        escaped_text(&level.effort),
        escaped_text(&level.description)
    );
}

fn parse_supported_effort(
    model: &SubscriptionModel,
    effort: &str,
) -> Result<ReasoningEffort, CliError> {
    if !model
        .supported_reasoning_levels
        .iter()
        .any(|candidate| candidate.effort == effort)
    {
        return Err(selection_error(
            "the requested reasoning effort is not supported by the selected model",
        ));
    }
    ReasoningEffort::from_str(effort).map_err(|_| {
        selection_error(
            "the selected reasoning effort is advertised but not executable by this version of Echidna",
        )
    })
}

fn require_terminal() -> Result<(), CliError> {
    if io::stdin().is_terminal() && io::stderr().is_terminal() {
        Ok(())
    } else {
        Err(selection_error(
            "non-interactive model selection requires --model <slug>",
        ))
    }
}

fn read_choice(prompt: &str) -> Result<String, CliError> {
    read_choice_or_empty(prompt)?.ok_or_else(|| selection_error("model selection was cancelled"))
}

fn read_choice_or_empty(prompt: &str) -> Result<Option<String>, CliError> {
    eprint!("{prompt}");
    io::stderr()
        .flush()
        .map_err(|_| selection_error("model selection input is unavailable"))?;
    read_selection_choice(&mut io::stdin().lock())
}

pub(crate) fn read_selection_choice(input: &mut impl BufRead) -> Result<Option<String>, CliError> {
    let mut line = String::new();
    let read = input
        .read_line(&mut line)
        .map_err(|_| selection_error("model selection input is unavailable"))?;
    if read == 0 {
        return Err(selection_error("model selection was cancelled"));
    }
    let choice = line.trim();
    Ok((!choice.is_empty()).then(|| choice.to_owned()))
}

fn selection_error(message: &str) -> CliError {
    CliError::ModelSelection {
        message: message.to_owned(),
    }
}

fn auth_error(error: impl std::fmt::Display) -> CliError {
    CliError::SubscriptionAuthentication {
        message: error.to_string(),
    }
}

fn open_browser(url: &str) {
    // The browser opener receives a single argv value, never a shell command.
    // Failure keeps the already printed URL usable for manual authorization.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        let opener = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        match tokio::process::Command::new(opener)
            .arg(url)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(mut child) => {
                tokio::spawn(async move {
                    let _ = child.wait().await;
                });
            }
            Err(_) => eprintln!("Browser launch unavailable; open the URL manually."),
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let _ = url;
}
