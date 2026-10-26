use std::path::{Component, Path, PathBuf};

use reqwest::StatusCode;
use serde::Serialize;
use thiserror::Error;

/// The fixed resource audience used by the ChatGPT subscription profile.
pub(crate) const API_BASE_URL: &str = "https://api.openai.com/v1";
pub(crate) const CODEX_API_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";
/// Declared catalog compatibility is intentionally distinct from Echidna's
/// package version and application identity. It comes from the highest
/// `minimal_client_version` in the pinned Codex model catalog recorded by
/// EC-A3; it does not claim to be an installed Codex release.
pub(crate) const CODEX_CATALOG_COMPATIBILITY_VERSION: &str = "0.144.0";
pub(super) const CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub(super) const DIRECT_USE_SCOPE: &str = "chatgpt.tokens.use.direct";
pub(super) const AUTHORIZE_URL: &str = "https://auth.openai.com/api/accounts/authorize";
pub(super) const TOKEN_URL: &str = "https://auth.openai.com/api/accounts/oauth/token";
pub(super) const DISCOVERY_URL: &str = "https://auth.openai.com/.well-known/openid-configuration";

#[derive(Clone, Debug, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SubscriptionConnectionState {
    Disconnected,
    Connected,
    LoginRequired,
}

/// Local-only account information suitable for CLI output. It deliberately
/// contains no OAuth code, token, verifier, host identifier, or raw subject.
#[derive(Clone, Debug, Serialize, Eq, PartialEq)]
pub(crate) struct SubscriptionAuthStatus {
    pub(crate) account: String,
    pub(crate) state: SubscriptionConnectionState,
    pub(crate) verified_identity: bool,
    pub(crate) direct_access_granted: bool,
    pub(crate) expires_at_unix: Option<u64>,
    pub(crate) earliest_refresh_at_unix: Option<u64>,
}

/// Codex access has a signed workspace identity instead of SIWC's direct-use
/// scope. Keeping its CLI result separate preserves the SIWC JSON contract.
#[derive(Clone, Debug, Serialize, Eq, PartialEq)]
pub(crate) struct CodexSubscriptionAuthStatus {
    pub(crate) account: String,
    pub(crate) state: SubscriptionConnectionState,
    pub(crate) verified_identity: bool,
    pub(crate) account_header_ready: bool,
    pub(crate) access_token_expires_at_unix: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SubscriptionAuthProtocol {
    SiwcDirect,
    Codex,
}

#[derive(Clone, Debug, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SubscriptionLogoutOutcome {
    NotConnected,
    RevokedAndCleared,
    RemoteUnconfirmedCleared,
}

/// Fixed subscription requests whose HTTP status may safely identify a failed
/// operation without retaining a remote response body.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SubscriptionAuthHttpOperation {
    TokenExchange,
    OpenIdDiscovery,
    JsonWebKeySet,
    ModelCatalog,
}

impl std::fmt::Display for SubscriptionAuthHttpOperation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::TokenExchange => "token exchange",
            Self::OpenIdDiscovery => "OpenID discovery",
            Self::JsonWebKeySet => "JWKS retrieval",
            Self::ModelCatalog => "model catalog",
        })
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub(crate) enum SubscriptionAuthError {
    #[error("subscription authentication store path must be an absolute private path")]
    InvalidStorePath,
    #[error("subscription account label is invalid")]
    InvalidAccount,
    #[error("subscription catalog compatibility version is invalid")]
    InvalidCatalogCompatibilityVersion,
    #[error("subscription authentication storage is unavailable")]
    StorageUnavailable,
    #[error("subscription authentication storage is not private")]
    InsecureStorage,
    #[error("subscription authentication storage contains a symbolic link")]
    SymlinkStorage,
    #[error("subscription authentication storage is malformed")]
    MalformedStorage,
    #[error("subscription authentication storage belongs to another protocol")]
    StoreProtocolMismatch,
    #[error("explicit credential refresh requires a Codex subscription Provider")]
    ExplicitCodexRefreshRequired,
    #[error("subscription authentication storage lock timed out")]
    StorageLockTimedOut,
    #[error("subscription login URL could not be presented")]
    LoginUrlPresentationFailed,
    #[error("subscription login timed out")]
    LoginTimedOut,
    #[error("subscription login callback is invalid")]
    InvalidCallback,
    #[error("subscription login was denied")]
    AuthorizationDenied,
    #[error("subscription client registration is invalid")]
    InvalidRegistration,
    #[error("subscription identity token is invalid")]
    InvalidIdentityToken,
    #[error("subscription identity does not match the selected account")]
    AccountMismatch,
    #[error("subscription direct token permission was not granted")]
    DirectTokenScopeMissing,
    #[error("subscription authentication {operation} request failed with HTTP {status}")]
    AuthHttpFailure {
        operation: SubscriptionAuthHttpOperation,
        status: u16,
    },
    #[error("subscription authentication service is temporarily unavailable")]
    TemporarilyUnavailable,
    #[error("subscription credentials require login")]
    LoginRequired,
    #[error("subscription credentials cannot yet be refreshed")]
    RefreshNotYetAllowed,
    #[error("subscription access token has expired")]
    AccessTokenExpired,
    #[error("subscription response is invalid")]
    InvalidResponse,
}

pub(super) fn resolve_catalog_client_version(
    value: Option<&str>,
) -> Result<String, SubscriptionAuthError> {
    let Some(value) = value else {
        return Ok(CODEX_CATALOG_COMPATIBILITY_VERSION.to_owned());
    };
    let components = value.split('.').collect::<Vec<_>>();
    if !value.is_ascii()
        || components.len() != 3
        || components.iter().any(|component| {
            component.is_empty()
                || (component.len() > 1 && component.starts_with('0'))
                || !component.bytes().all(|byte| byte.is_ascii_digit())
                || component.parse::<u32>().is_err()
        })
    {
        return Err(SubscriptionAuthError::InvalidCatalogCompatibilityVersion);
    }
    Ok(value.to_owned())
}

/// Internal endpoint set. `new` always supplies the fixed production values;
/// the shared constructor keeps loopback OAuth fixtures out of configuration.
#[derive(Clone, Debug)]
pub(crate) struct AuthEndpoints {
    pub(crate) authorize_url: String,
    pub(crate) token_url: String,
    pub(crate) discovery_url: String,
    pub(crate) api_base_url: String,
    pub(crate) issuer: String,
}

impl AuthEndpoints {
    pub(super) fn production() -> Self {
        Self {
            authorize_url: AUTHORIZE_URL.to_owned(),
            token_url: TOKEN_URL.to_owned(),
            discovery_url: DISCOVERY_URL.to_owned(),
            api_base_url: API_BASE_URL.to_owned(),
            issuer: "https://auth.openai.com".to_owned(),
        }
    }
}

pub(super) fn validate_constructor_input(
    auth_store: &Path,
    account: &str,
) -> Result<(), SubscriptionAuthError> {
    if !auth_store.is_absolute()
        || auth_store
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
        || auth_store.components().any(|component| {
            matches!(
                component,
                Component::Normal(name)
                    if matches!(name.to_str(), Some(".codex" | ".cyber-agent" | ".echidna" | ".git"))
            )
        })
    {
        return Err(SubscriptionAuthError::InvalidStorePath);
    }

    let allowed = |byte: u8| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.');
    if account.is_empty()
        || account.len() > 64
        || matches!(account, "." | "..")
        || !account.bytes().all(allowed)
    {
        return Err(SubscriptionAuthError::InvalidAccount);
    }

    Ok(())
}

pub(super) fn account_path(root: &Path, account: &str, name: &str) -> PathBuf {
    root.join(format!("{account}.{name}"))
}

pub(super) fn classify_http_status(status: StatusCode) -> SubscriptionAuthError {
    if temporary_status(status) {
        SubscriptionAuthError::TemporarilyUnavailable
    } else if matches!(
        status,
        StatusCode::BAD_REQUEST | StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
    ) {
        SubscriptionAuthError::LoginRequired
    } else {
        SubscriptionAuthError::InvalidResponse
    }
}

pub(super) fn auth_http_failure(
    operation: SubscriptionAuthHttpOperation,
    status: StatusCode,
) -> SubscriptionAuthError {
    SubscriptionAuthError::AuthHttpFailure {
        operation,
        status: status.as_u16(),
    }
}

pub(super) fn temporary_status(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}
