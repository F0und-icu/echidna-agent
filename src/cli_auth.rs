//! Explicit account operations outside a research Project or AgentLoop.

use std::path::Path;

use clap::Subcommand;

use crate::provider_composition::ProviderComposition;
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
}

pub(crate) fn run(config: Option<&Path>, command: AuthCommand) -> Result<(), CliError> {
    let config = config.ok_or(CliError::AuthConfigurationRequired)?;
    let provider = match &command {
        AuthCommand::Login { provider, .. }
        | AuthCommand::Status { provider }
        | AuthCommand::Logout { provider }
        | AuthCommand::Models { provider } => provider,
    };
    let service = ProviderComposition::subscription_auth(config, provider)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .map_err(CliError::AuthRuntime)?;
    runtime.block_on(async {
        match command {
            AuthCommand::Login { no_browser, .. } => {
                let status = service
                    .login(|url| {
                        eprintln!("Open this URL to authorize Echidna:\n{url}");
                        if !no_browser {
                            open_browser(url);
                        }
                        Ok(())
                    })
                    .await
                    .map_err(auth_error)?;
                print_json(&status)
            }
            AuthCommand::Status { .. } => print_json(&service.status().await.map_err(auth_error)?),
            AuthCommand::Logout { .. } => print_json(&service.logout().await.map_err(auth_error)?),
            AuthCommand::Models { .. } => print_json(&service.models().await.map_err(auth_error)?),
        }
    })
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
