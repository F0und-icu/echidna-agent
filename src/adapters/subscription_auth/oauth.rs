use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use reqwest::StatusCode;
use ring::digest::{SHA256, digest};
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio::time::timeout;

use super::jwt::{
    JsonWebKeySet, OpenIdDiscovery, constant_time_equal, valid_chatgpt_account_id,
    verify_codex_identity_token, verify_identity_token,
};
use super::store::{
    AuthStore, CodexConnectionRecord, CodexIdentityRecord, CodexTokenRecord, ConnectionRecord,
    RegistrationRecord, TokenRecord,
};
use super::types::{
    CODEX_CLIENT_ID, CodexSubscriptionAuthStatus, DIRECT_USE_SCOPE, SubscriptionAuthError,
    SubscriptionAuthHttpOperation, SubscriptionAuthProtocol, SubscriptionAuthStatus,
    SubscriptionConnectionState, SubscriptionLogoutOutcome, SubscriptionModel,
    SubscriptionReasoningLevel, resolve_catalog_client_version, validate_constructor_input,
};
use super::{API_BASE_URL, AuthEndpoints, CODEX_API_BASE_URL, CODEX_CATALOG_COMPATIBILITY_VERSION};

const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);
const CALLBACK_READ_TIMEOUT: Duration = Duration::from_secs(10);
const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const CALLBACK_MAX_BYTES: usize = 16 * 1024;
/// Codex refresh needs only a small JSON envelope containing a fixed error
/// code. The bounded reader never retains diagnostic payloads.
const CODEX_REFRESH_ERROR_BODY_BYTES: usize = 8 * 1024;
const DYNAMIC_AGENT_CLIENT: &str = "dynamic_agent_client";
const AGENT_NAME_HINT: &str = "Echidna";
const REQUESTED_SCOPE: &str =
    "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";
const CODEX_REQUESTED_SCOPE: &str = "openid offline_access";
const CODEX_ORIGINATOR: &str = "codex_cli_rs";
const CODEX_AUTHORIZATION_URL: &str = "https://auth.openai.com/oauth/authorize";
const CODEX_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const CODEX_REVOKE_URL: &str = "https://auth.openai.com/oauth/revoke";
const CODEX_DISCOVERY_URL: &str = "https://auth.openai.com/.well-known/openid-configuration";
const CODEX_ISSUER: &str = "https://auth.openai.com";
const ECHIDNA_USER_AGENT: &str = concat!("echidna/", env!("CARGO_PKG_VERSION"));

/// App-owned OAuth lifecycle for one explicit local subscription account.
/// Construction is deliberately free of filesystem, credential, and network I/O.
#[derive(Clone)]
pub(crate) struct SubscriptionAuthService {
    store: AuthStore,
    endpoints: EndpointSet,
    protocol: SubscriptionAuthProtocol,
    catalog_client_version: String,
    http: reqwest::Client,
    refresh_mutex: Arc<Mutex<()>>,
}

#[derive(Clone, Debug)]
pub(crate) struct CodexAuthEndpoints {
    pub(crate) authorize_url: String,
    pub(crate) token_url: String,
    pub(crate) revoke_url: String,
    pub(crate) discovery_url: String,
    pub(crate) api_base_url: String,
    pub(crate) issuer: String,
}

#[derive(Clone, Debug)]
enum EndpointSet {
    Siwc(AuthEndpoints),
    Codex(CodexAuthEndpoints),
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum CodexRefreshMode {
    ExpiredOnly,
    Explicit,
}

impl EndpointSet {
    fn authorize_url(&self) -> &str {
        match self {
            Self::Siwc(endpoints) => &endpoints.authorize_url,
            Self::Codex(endpoints) => &endpoints.authorize_url,
        }
    }

    fn token_url(&self) -> &str {
        match self {
            Self::Siwc(endpoints) => &endpoints.token_url,
            Self::Codex(endpoints) => &endpoints.token_url,
        }
    }

    fn discovery_url(&self) -> &str {
        match self {
            Self::Siwc(endpoints) => &endpoints.discovery_url,
            Self::Codex(endpoints) => &endpoints.discovery_url,
        }
    }

    fn api_base_url(&self) -> &str {
        match self {
            Self::Siwc(endpoints) => &endpoints.api_base_url,
            Self::Codex(endpoints) => &endpoints.api_base_url,
        }
    }

    fn issuer(&self) -> &str {
        match self {
            Self::Siwc(endpoints) => &endpoints.issuer,
            Self::Codex(endpoints) => &endpoints.issuer,
        }
    }
}

/// The Composition Root converts this immediately into its public
/// account-bound credential. It intentionally exposes no inspectable token
/// fields or secret-derived debug output.
pub(crate) struct CodexSubscriptionCredential {
    bearer: String,
    account_id: String,
}

impl std::fmt::Debug for CodexSubscriptionCredential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("CodexSubscriptionCredential(..)")
    }
}

impl CodexSubscriptionCredential {
    pub(crate) fn into_parts(self) -> (String, String) {
        (self.bearer, self.account_id)
    }
}

impl SubscriptionAuthService {
    pub(crate) fn new(auth_store: PathBuf, account: String) -> Result<Self, SubscriptionAuthError> {
        Self::with_endpoints(auth_store, account, AuthEndpoints::production())
    }

    pub(crate) fn new_codex(
        auth_store: PathBuf,
        account: String,
        catalog_client_version: Option<&str>,
    ) -> Result<Self, SubscriptionAuthError> {
        Self::with_codex_endpoints(
            auth_store,
            account,
            CodexAuthEndpoints {
                authorize_url: CODEX_AUTHORIZATION_URL.to_owned(),
                token_url: CODEX_TOKEN_URL.to_owned(),
                revoke_url: CODEX_REVOKE_URL.to_owned(),
                discovery_url: CODEX_DISCOVERY_URL.to_owned(),
                api_base_url: CODEX_API_BASE_URL.to_owned(),
                issuer: CODEX_ISSUER.to_owned(),
            },
            catalog_client_version,
        )
    }

    /// The production constructor is the sole runtime caller. This supports
    /// local loopback fixtures without exposing an endpoint override in Config.
    pub(crate) fn with_endpoints(
        auth_store: PathBuf,
        account: String,
        endpoints: AuthEndpoints,
    ) -> Result<Self, SubscriptionAuthError> {
        Self::build(
            auth_store,
            account,
            SubscriptionAuthProtocol::SiwcDirect,
            EndpointSet::Siwc(endpoints),
            CODEX_CATALOG_COMPATIBILITY_VERSION.to_owned(),
        )
    }

    pub(crate) fn with_codex_endpoints(
        auth_store: PathBuf,
        account: String,
        endpoints: CodexAuthEndpoints,
        catalog_client_version: Option<&str>,
    ) -> Result<Self, SubscriptionAuthError> {
        let catalog_client_version = resolve_catalog_client_version(catalog_client_version)?;
        Self::build(
            auth_store,
            account,
            SubscriptionAuthProtocol::Codex,
            EndpointSet::Codex(endpoints),
            catalog_client_version,
        )
    }

    fn build(
        auth_store: PathBuf,
        account: String,
        protocol: SubscriptionAuthProtocol,
        endpoints: EndpointSet,
        catalog_client_version: String,
    ) -> Result<Self, SubscriptionAuthError> {
        validate_constructor_input(&auth_store, &account)?;
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(HTTP_CONNECT_TIMEOUT)
            .timeout(HTTP_TIMEOUT)
            .build()
            .map_err(|_| SubscriptionAuthError::StorageUnavailable)?;
        Ok(Self {
            store: AuthStore::new(auth_store, account),
            endpoints,
            protocol,
            catalog_client_version,
            http,
            refresh_mutex: Arc::new(Mutex::new(())),
        })
    }

    pub(crate) fn protocol(&self) -> SubscriptionAuthProtocol {
        self.protocol
    }

    /// Binds the loopback listener before exposing the authorization URL. The
    /// callback receives only the display-safe URL and may decline to open it.
    pub(crate) async fn login<F>(
        &self,
        present_authorization_url: F,
    ) -> Result<SubscriptionAuthStatus, SubscriptionAuthError>
    where
        F: FnOnce(&str) -> Result<(), SubscriptionAuthError>,
    {
        if self.protocol != SubscriptionAuthProtocol::SiwcDirect {
            return Err(SubscriptionAuthError::StoreProtocolMismatch);
        }
        let (host, registration) = {
            let _refresh_guard = self.refresh_mutex.lock().await;
            let _store_lock = self.store.lock().await?;
            let host = self.store.load_or_create_host()?;
            let registration = self
                .valid_connection()?
                .map(|connection| connection.registration);
            (host, registration)
        };
        let requested_client_id = registration
            .as_ref()
            .map_or(DYNAMIC_AGENT_CLIENT, |registration| {
                registration.client_id.as_str()
            });
        let listener = LoopbackListener::bind().await?;
        let state = random_url_safe(32)?;
        let nonce = random_url_safe(32)?;
        let verifier = random_url_safe(48)?;
        let challenge = pkce_challenge(&verifier);
        let redirect_uri = listener.redirect_uri.clone();
        let url = authorization_url(
            self.endpoints.authorize_url(),
            requested_client_id,
            &host.host_id,
            &redirect_uri,
            &state,
            &nonce,
            &challenge,
        );
        present_authorization_url(&url)
            .map_err(|_| SubscriptionAuthError::LoginUrlPresentationFailed)?;
        let callback = validate_callback(listener.await_callback().await?, &state)?;
        let issued_client_id = match (registration.as_ref(), callback.client_id) {
            (Some(existing), Some(callback_client)) if callback_client == existing.client_id => {
                existing.client_id.clone()
            }
            (Some(existing), None) => existing.client_id.clone(),
            (Some(_), Some(_)) => return Err(SubscriptionAuthError::InvalidRegistration),
            (None, Some(callback_client))
                if callback_client != DYNAMIC_AGENT_CLIENT
                    && !callback_client.is_empty()
                    && callback_client.len() <= 512 =>
            {
                callback_client
            }
            (None, _) => return Err(SubscriptionAuthError::InvalidRegistration),
        };
        let code = callback
            .code
            .ok_or(SubscriptionAuthError::InvalidCallback)?;

        let token_response = self
            .exchange_code(&issued_client_id, &code, &verifier, &redirect_uri)
            .await?;
        let id_token = token_response
            .id_token
            .as_deref()
            .ok_or(SubscriptionAuthError::InvalidIdentityToken)?;
        let discovery = self.fetch_discovery().await?;
        let keys = self.fetch_jwks(&discovery).await?;
        let subject = verify_identity_token(
            id_token,
            &discovery,
            &keys,
            &issued_client_id,
            &nonce,
            now_unix()?,
        )?;
        let tokens = token_response.into_token_record(
            now_unix()?,
            None,
            &issued_client_id,
            &subject,
            &discovery.issuer,
        )?;
        require_direct_scope(&tokens.scopes)?;

        let _refresh_guard = self.refresh_mutex.lock().await;
        let _store_lock = self.store.lock().await?;
        let existing = self
            .valid_connection()?
            .map(|connection| connection.registration);
        if let Some(existing) = existing.as_ref()
            && (existing.client_id != issued_client_id
                || existing
                    .subject
                    .as_deref()
                    .is_some_and(|saved| saved != subject)
                || existing
                    .issuer
                    .as_deref()
                    .is_some_and(|saved| saved != discovery.issuer))
        {
            return Err(SubscriptionAuthError::AccountMismatch);
        }
        self.store.write_connection(&ConnectionRecord {
            version: 1,
            registration: RegistrationRecord {
                version: 1,
                account: self.store.account_label(),
                client_id: issued_client_id,
                subject: Some(subject),
                issuer: Some(discovery.issuer),
            },
            tokens: Some(tokens),
        })?;
        self.status().await
    }

    /// Reads only app-owned private metadata. It never creates a directory,
    /// takes a lock, refreshes a credential, or sends a network request.
    pub(crate) async fn status(&self) -> Result<SubscriptionAuthStatus, SubscriptionAuthError> {
        if self.protocol != SubscriptionAuthProtocol::SiwcDirect {
            return Err(SubscriptionAuthError::StoreProtocolMismatch);
        }
        let connection = self.valid_connection()?;
        let registration = connection
            .as_ref()
            .map(|connection| &connection.registration);
        let tokens = connection
            .as_ref()
            .and_then(|connection| connection.tokens.as_ref());
        let direct_access_granted = tokens
            .as_ref()
            .is_some_and(|tokens| has_direct_scope(&tokens.scopes));
        let state = if registration.is_some() && direct_access_granted {
            SubscriptionConnectionState::Connected
        } else if registration.is_some() {
            SubscriptionConnectionState::LoginRequired
        } else {
            SubscriptionConnectionState::Disconnected
        };
        Ok(SubscriptionAuthStatus {
            account: self.store.account_label(),
            state,
            verified_identity: registration
                .is_some_and(|registration| registration.subject.is_some()),
            direct_access_granted,
            expires_at_unix: tokens.as_ref().map(|tokens| tokens.expires_at_unix),
            earliest_refresh_at_unix: tokens
                .as_ref()
                .map(|tokens| tokens.earliest_refresh_at_unix),
        })
    }

    /// Obtains one bearer value immediately before the existing Responses
    /// transport sends its request. This is the only operation that refreshes.
    pub(crate) async fn access_token(&self) -> Result<String, SubscriptionAuthError> {
        if self.protocol != SubscriptionAuthProtocol::SiwcDirect {
            return Err(SubscriptionAuthError::StoreProtocolMismatch);
        }
        let now = now_unix()?;
        let tokens = self.require_usable_tokens()?;
        if now < tokens.expires_at_unix && now < tokens.earliest_refresh_at_unix {
            return Ok(tokens.access_token);
        }
        if now >= tokens.expires_at_unix && now < tokens.earliest_refresh_at_unix {
            return Err(SubscriptionAuthError::RefreshNotYetAllowed);
        }

        let _refresh_guard = self.refresh_mutex.lock().await;
        let _store_lock = self.store.lock().await?;
        let tokens = self.require_usable_tokens()?;
        let now = now_unix()?;
        if now < tokens.expires_at_unix && now < tokens.earliest_refresh_at_unix {
            return Ok(tokens.access_token);
        }
        if now >= tokens.expires_at_unix && now < tokens.earliest_refresh_at_unix {
            return Err(SubscriptionAuthError::RefreshNotYetAllowed);
        }

        // `tokens` was reread and registration-bound after taking the
        // cross-process store lock, so refresh can use its validated fields.
        match self.refresh(&tokens).await {
            Ok(refreshed) => {
                self.replace_tokens(&refreshed)?;
                Ok(refreshed.access_token)
            }
            Err(RefreshFailure::Temporary) => {
                // The request may have consumed the remaining lifetime. Never
                // return a bearer merely because it was valid before an
                // awaited refresh attempt.
                if now_unix()? < tokens.expires_at_unix {
                    Ok(tokens.access_token)
                } else {
                    Err(SubscriptionAuthError::TemporarilyUnavailable)
                }
            }
            Err(RefreshFailure::Terminal) => {
                self.store.clear_tokens()?;
                Err(SubscriptionAuthError::LoginRequired)
            }
            Err(RefreshFailure::InvalidClient) => Err(SubscriptionAuthError::InvalidRegistration),
            Err(RefreshFailure::Authentication(error)) => Err(error),
        }
    }

    /// Performs the explicit, account-scoped model discovery path after
    /// obtaining a usable subscription bearer.
    pub(crate) async fn models(&self) -> Result<Vec<SubscriptionModel>, SubscriptionAuthError> {
        if self.protocol == SubscriptionAuthProtocol::Codex {
            return self.codex_models().await;
        }
        let token = self.access_token().await?;
        let response = self
            .http
            .get(format!(
                "{}/models",
                self.endpoints.api_base_url().trim_end_matches('/')
            ))
            .bearer_auth(token)
            .send()
            .await
            .map_err(|_| SubscriptionAuthError::TemporarilyUnavailable)?;
        if !response.status().is_success() {
            return Err(classify_http_status(response.status()));
        }
        let models: SiwcModelsResponse = response
            .json()
            .await
            .map_err(|_| SubscriptionAuthError::InvalidResponse)?;
        siwc_subscription_models(
            models
                .models
                .into_iter()
                .filter(|model| model.visibility == "list")
                .collect(),
        )
    }

    /// Clears app-owned tokens even when revocation cannot be confirmed. Host
    /// and registration records remain so a later reconnect reuses the client.
    pub(crate) async fn logout(&self) -> Result<SubscriptionLogoutOutcome, SubscriptionAuthError> {
        if self.protocol == SubscriptionAuthProtocol::Codex {
            return self.logout_codex().await;
        }
        if self
            .valid_connection()?
            .and_then(|connection| connection.tokens)
            .is_none()
        {
            return Ok(SubscriptionLogoutOutcome::NotConnected);
        }
        let _refresh_guard = self.refresh_mutex.lock().await;
        let _store_lock = self.store.lock().await?;
        let tokens = match self
            .valid_connection()?
            .and_then(|connection| connection.tokens)
        {
            Some(tokens) => tokens,
            None => return Ok(SubscriptionLogoutOutcome::NotConnected),
        };
        let confirmed = match self.fetch_discovery().await {
            Ok(discovery) => match discovery.revocation_endpoint {
                Some(endpoint) => self.revoke(&endpoint, &tokens.refresh_token).await,
                None => false,
            },
            Err(_) => false,
        };
        self.store.clear_tokens()?;
        Ok(if confirmed {
            SubscriptionLogoutOutcome::RevokedAndCleared
        } else {
            SubscriptionLogoutOutcome::RemoteUnconfirmedCleared
        })
    }

    /// Codex's fixed OAuth flow has no dynamic registration, SIWC nonce, or
    /// direct-use scope. The callback remains state- and PKCE-bound.
    pub(crate) async fn login_codex<F>(
        &self,
        present_authorization_url: F,
    ) -> Result<CodexSubscriptionAuthStatus, SubscriptionAuthError>
    where
        F: FnOnce(&str) -> Result<(), SubscriptionAuthError>,
    {
        if self.protocol != SubscriptionAuthProtocol::Codex {
            return Err(SubscriptionAuthError::StoreProtocolMismatch);
        }
        {
            let _refresh_guard = self.refresh_mutex.lock().await;
            let _store_lock = self.store.lock().await?;
            self.valid_codex_connection()?;
        }
        let listener = LoopbackListener::bind_codex().await?;
        let state = random_url_safe(32)?;
        let verifier = random_url_safe(48)?;
        let challenge = pkce_challenge(&verifier);
        let redirect_uri = listener.redirect_uri.clone();
        let url = codex_authorization_url(
            self.endpoints.authorize_url(),
            &redirect_uri,
            &state,
            &challenge,
        );
        present_authorization_url(&url)
            .map_err(|_| SubscriptionAuthError::LoginUrlPresentationFailed)?;
        let callback = validate_callback(listener.await_callback().await?, &state)?;
        let code = callback
            .code
            .ok_or(SubscriptionAuthError::InvalidCallback)?;
        let tokens = self
            .exchange_codex_code(&code, &verifier, &redirect_uri)
            .await?;
        let identity = self.verify_codex_identity(&tokens.id_token).await?;
        let connection = CodexConnectionRecord {
            version: 2,
            protocol: "codex".to_owned(),
            account: self.store.account_label(),
            identity: CodexIdentityRecord {
                client_id: CODEX_CLIENT_ID.to_owned(),
                subject: identity.subject,
                issuer: identity.issuer,
                id_token_expires_at_unix: identity.expires_at_unix,
                chatgpt_account_id: identity.chatgpt_account_id,
            },
            tokens: Some(CodexTokenRecord {
                access_token_expires_at_unix: access_token_expiry(&tokens.access_token)?,
                access_token: tokens.access_token,
                refresh_token: tokens.refresh_token,
            }),
        };
        let _refresh_guard = self.refresh_mutex.lock().await;
        let _store_lock = self.store.lock().await?;
        if let Some(existing) = self.valid_codex_connection()?
            && (existing.identity.subject != connection.identity.subject
                || existing.identity.issuer != connection.identity.issuer
                || existing.identity.chatgpt_account_id != connection.identity.chatgpt_account_id)
        {
            return Err(SubscriptionAuthError::AccountMismatch);
        }
        self.store.write_codex_connection(&connection)?;
        self.codex_status().await
    }

    /// Reads only the tagged Codex record. It does not create a store, lock,
    /// refresh, or send a request.
    pub(crate) async fn codex_status(
        &self,
    ) -> Result<CodexSubscriptionAuthStatus, SubscriptionAuthError> {
        if self.protocol != SubscriptionAuthProtocol::Codex {
            return Err(SubscriptionAuthError::StoreProtocolMismatch);
        }
        let connection = self.valid_codex_connection()?;
        let state = match connection.as_ref() {
            Some(connection) if connection.tokens.is_some() => {
                SubscriptionConnectionState::Connected
            }
            Some(_) => SubscriptionConnectionState::LoginRequired,
            None => SubscriptionConnectionState::Disconnected,
        };
        Ok(CodexSubscriptionAuthStatus {
            account: self.store.account_label(),
            state,
            verified_identity: connection.is_some(),
            account_header_ready: connection.is_some(),
            access_token_expires_at_unix: connection
                .and_then(|connection| connection.tokens)
                .map(|tokens| tokens.access_token_expires_at_unix),
        })
    }

    /// Returns one token/account pair from one reread record after any needed
    /// refresh. The pair is never assembled from separate store observations.
    pub(crate) async fn access_codex_credential(
        &self,
    ) -> Result<CodexSubscriptionCredential, SubscriptionAuthError> {
        if self.protocol != SubscriptionAuthProtocol::Codex {
            return Err(SubscriptionAuthError::StoreProtocolMismatch);
        }
        let now = now_unix()?;
        let connection = self.require_codex_connection()?;
        if connection
            .tokens
            .as_ref()
            .is_some_and(|tokens| now < tokens.access_token_expires_at_unix)
        {
            return Ok(codex_credential(&connection));
        }

        let refreshed = self
            .refresh_and_publish_codex_connection(CodexRefreshMode::ExpiredOnly)
            .await?;
        Ok(codex_credential(&refreshed))
    }

    /// Refreshes the selected Codex grant once, regardless of local expiry,
    /// and returns only the existing sanitized account status.
    pub(crate) async fn refresh_codex(
        &self,
    ) -> Result<CodexSubscriptionAuthStatus, SubscriptionAuthError> {
        if self.protocol != SubscriptionAuthProtocol::Codex {
            return Err(SubscriptionAuthError::ExplicitCodexRefreshRequired);
        }
        self.refresh_and_publish_codex_connection(CodexRefreshMode::Explicit)
            .await?;
        self.codex_status().await
    }

    async fn refresh_and_publish_codex_connection(
        &self,
        mode: CodexRefreshMode,
    ) -> Result<CodexConnectionRecord, SubscriptionAuthError> {
        let _refresh_guard = self.refresh_mutex.lock().await;
        let _store_lock = self.store.lock().await?;
        let connection = self.require_codex_connection()?;
        if mode == CodexRefreshMode::ExpiredOnly {
            let now = now_unix()?;
            if connection
                .tokens
                .as_ref()
                .is_some_and(|tokens| now < tokens.access_token_expires_at_unix)
            {
                return Ok(connection);
            }
        }
        match self.request_codex_refresh(&connection).await {
            Ok(refreshed) => {
                self.store.write_codex_connection(&refreshed)?;
                let now = now_unix()?;
                if refreshed
                    .tokens
                    .as_ref()
                    .is_none_or(|tokens| now >= tokens.access_token_expires_at_unix)
                {
                    return Err(SubscriptionAuthError::AccessTokenExpired);
                }
                Ok(refreshed)
            }
            Err(RefreshFailure::Temporary) => Err(SubscriptionAuthError::TemporarilyUnavailable),
            Err(RefreshFailure::Terminal) => {
                self.store.clear_codex_tokens()?;
                Err(SubscriptionAuthError::LoginRequired)
            }
            Err(RefreshFailure::InvalidClient) => Err(SubscriptionAuthError::InvalidRegistration),
            Err(RefreshFailure::Authentication(error)) => Err(error),
        }
    }

    async fn codex_models(&self) -> Result<Vec<SubscriptionModel>, SubscriptionAuthError> {
        let credential = self.access_codex_credential().await?;
        let (bearer, account_id) = credential.into_parts();
        let response = self
            .http
            .get(format!(
                "{}/models",
                self.endpoints.api_base_url().trim_end_matches('/')
            ))
            .query(&[("client_version", self.catalog_client_version.as_str())])
            .bearer_auth(bearer)
            .header("ChatGPT-Account-ID", account_id)
            .header("version", self.catalog_client_version.as_str())
            .header("User-Agent", ECHIDNA_USER_AGENT)
            .send()
            .await
            .map_err(|_| SubscriptionAuthError::TemporarilyUnavailable)?;
        if !response.status().is_success() {
            return Err(auth_http_failure(
                SubscriptionAuthHttpOperation::ModelCatalog,
                response.status(),
            ));
        }
        let models: CodexModelsResponse = response
            .json()
            .await
            .map_err(|_| SubscriptionAuthError::InvalidResponse)?;
        codex_subscription_models(models.models)
    }

    async fn logout_codex(&self) -> Result<SubscriptionLogoutOutcome, SubscriptionAuthError> {
        if self
            .valid_codex_connection()?
            .and_then(|connection| connection.tokens)
            .is_none()
        {
            return Ok(SubscriptionLogoutOutcome::NotConnected);
        }
        let _refresh_guard = self.refresh_mutex.lock().await;
        let _store_lock = self.store.lock().await?;
        let tokens = match self
            .valid_codex_connection()?
            .and_then(|connection| connection.tokens)
        {
            Some(tokens) => tokens,
            None => return Ok(SubscriptionLogoutOutcome::NotConnected),
        };
        let confirmed = match &self.endpoints {
            EndpointSet::Codex(endpoints) => {
                self.revoke_codex(&endpoints.revoke_url, &tokens.refresh_token)
                    .await
            }
            EndpointSet::Siwc(_) => false,
        };
        self.store.clear_codex_tokens()?;
        Ok(if confirmed {
            SubscriptionLogoutOutcome::RevokedAndCleared
        } else {
            SubscriptionLogoutOutcome::RemoteUnconfirmedCleared
        })
    }

    fn require_usable_tokens(&self) -> Result<TokenRecord, SubscriptionAuthError> {
        let connection = self
            .valid_connection()?
            .ok_or(SubscriptionAuthError::LoginRequired)?;
        let tokens = connection
            .tokens
            .ok_or(SubscriptionAuthError::LoginRequired)?;
        require_direct_scope(&tokens.scopes)?;
        Ok(tokens)
    }

    fn require_codex_connection(&self) -> Result<CodexConnectionRecord, SubscriptionAuthError> {
        let connection = self
            .valid_codex_connection()?
            .ok_or(SubscriptionAuthError::LoginRequired)?;
        if connection.tokens.is_none() {
            return Err(SubscriptionAuthError::LoginRequired);
        }
        Ok(connection)
    }

    fn valid_connection(&self) -> Result<Option<ConnectionRecord>, SubscriptionAuthError> {
        let connection = self.store.read_connection()?;
        let Some(connection) = connection else {
            return Ok(None);
        };
        let registration = &connection.registration;
        let (Some(subject), Some(issuer)) = (
            registration.subject.as_deref(),
            registration.issuer.as_deref(),
        ) else {
            return Err(SubscriptionAuthError::MalformedStorage);
        };
        if connection.version != 1
            || registration.version != 1
            || registration.account != self.store.account_label()
            || registration.client_id.is_empty()
            || registration.client_id.len() > 512
            || subject.is_empty()
            || issuer.is_empty()
        {
            return Err(SubscriptionAuthError::MalformedStorage);
        }
        if let Some(tokens) = connection.tokens.as_ref()
            && (tokens.version != 1
                || tokens.client_id != registration.client_id
                || tokens.subject != subject
                || tokens.issuer != issuer
                || tokens.access_token.is_empty()
                || tokens.refresh_token.is_empty()
                || tokens.scopes.iter().any(|scope| scope.is_empty()))
        {
            return Err(SubscriptionAuthError::MalformedStorage);
        }
        Ok(Some(connection))
    }

    fn valid_codex_connection(
        &self,
    ) -> Result<Option<CodexConnectionRecord>, SubscriptionAuthError> {
        let connection = self.store.read_codex_connection()?;
        let Some(connection) = connection else {
            return Ok(None);
        };
        let identity = &connection.identity;
        if connection.version != 2
            || connection.protocol != "codex"
            || connection.account != self.store.account_label()
            || identity.client_id != CODEX_CLIENT_ID
            || identity.subject.is_empty()
            || identity.subject.len() > 512
            || identity.issuer != self.endpoints.issuer()
            || identity.id_token_expires_at_unix == 0
            || !valid_chatgpt_account_id(&identity.chatgpt_account_id)
        {
            return Err(SubscriptionAuthError::MalformedStorage);
        }
        if let Some(tokens) = connection.tokens.as_ref()
            && (tokens.access_token.is_empty()
                || tokens.refresh_token.is_empty()
                || tokens.access_token_expires_at_unix == 0)
        {
            return Err(SubscriptionAuthError::MalformedStorage);
        }
        Ok(Some(connection))
    }

    fn replace_tokens(&self, tokens: &TokenRecord) -> Result<(), SubscriptionAuthError> {
        let mut connection = self
            .valid_connection()?
            .ok_or(SubscriptionAuthError::LoginRequired)?;
        connection.tokens = Some(tokens.clone());
        self.store.write_connection(&connection)
    }

    async fn exchange_code(
        &self,
        client_id: &str,
        code: &str,
        verifier: &str,
        redirect_uri: &str,
    ) -> Result<TokenResponse, SubscriptionAuthError> {
        let response = self
            .http
            .post(self.endpoints.token_url())
            .form(&[
                ("grant_type", "authorization_code"),
                ("client_id", client_id),
                ("code", code),
                ("code_verifier", verifier),
                ("redirect_uri", redirect_uri),
                ("resource", API_BASE_URL),
            ])
            .send()
            .await
            .map_err(|_| SubscriptionAuthError::TemporarilyUnavailable)?;
        if !response.status().is_success() {
            return Err(auth_http_failure(
                SubscriptionAuthHttpOperation::TokenExchange,
                response.status(),
            ));
        }
        response
            .json()
            .await
            .map_err(|_| SubscriptionAuthError::InvalidResponse)
    }

    async fn exchange_codex_code(
        &self,
        code: &str,
        verifier: &str,
        redirect_uri: &str,
    ) -> Result<CodexInitialTokenResponse, SubscriptionAuthError> {
        let response = self
            .http
            .post(self.endpoints.token_url())
            .form(&[
                ("grant_type", "authorization_code"),
                ("client_id", CODEX_CLIENT_ID),
                ("code", code),
                ("code_verifier", verifier),
                ("redirect_uri", redirect_uri),
            ])
            .send()
            .await
            .map_err(|_| SubscriptionAuthError::TemporarilyUnavailable)?;
        if !response.status().is_success() {
            return Err(auth_http_failure(
                SubscriptionAuthHttpOperation::TokenExchange,
                response.status(),
            ));
        }
        let response: CodexInitialTokenResponse = response
            .json()
            .await
            .map_err(|_| SubscriptionAuthError::InvalidResponse)?;
        if response.id_token.is_empty()
            || response.access_token.is_empty()
            || response.refresh_token.is_empty()
        {
            return Err(SubscriptionAuthError::InvalidResponse);
        }
        Ok(response)
    }

    async fn refresh(&self, previous: &TokenRecord) -> Result<TokenRecord, RefreshFailure> {
        let response = self
            .http
            .post(self.endpoints.token_url())
            .form(&[
                ("grant_type", "refresh_token"),
                ("client_id", previous.client_id.as_str()),
                ("refresh_token", previous.refresh_token.as_str()),
                ("resource", API_BASE_URL),
            ])
            .send()
            .await
            .map_err(|_| RefreshFailure::Temporary)?;
        if !response.status().is_success() {
            return Err(classify_refresh_response(response).await);
        }
        let response: TokenResponse = response
            .json()
            .await
            .map_err(|_| RefreshFailure::Temporary)?;
        let scopes = response
            .scope
            .as_deref()
            .map(parse_scopes)
            .transpose()
            .map_err(|_| RefreshFailure::Temporary)?
            .unwrap_or_else(|| previous.scopes.clone());
        let refreshed = response
            .into_token_record(
                now_unix().map_err(|_| RefreshFailure::Temporary)?,
                Some(scopes),
                &previous.client_id,
                &previous.subject,
                &previous.issuer,
            )
            .map_err(|_| RefreshFailure::Temporary)?;
        require_direct_scope(&refreshed.scopes).map_err(|_| RefreshFailure::Terminal)?;
        Ok(refreshed)
    }

    async fn request_codex_refresh(
        &self,
        previous: &CodexConnectionRecord,
    ) -> Result<CodexConnectionRecord, RefreshFailure> {
        let tokens = previous
            .tokens
            .as_ref()
            .expect("validated Codex connection has tokens");
        let response = self
            .http
            .post(self.endpoints.token_url())
            .json(&CodexRefreshRequest {
                client_id: CODEX_CLIENT_ID,
                grant_type: "refresh_token",
                refresh_token: &tokens.refresh_token,
            })
            .send()
            .await
            .map_err(|_| RefreshFailure::Temporary)?;
        if !response.status().is_success() {
            return Err(classify_codex_refresh_response(response).await);
        }
        let response: CodexRefreshResponse = response
            .json()
            .await
            .map_err(|_| RefreshFailure::Temporary)?;
        if response.access_token.as_deref().is_some_and(str::is_empty)
            || response.refresh_token.as_deref().is_some_and(str::is_empty)
            || response.id_token.as_deref().is_some_and(str::is_empty)
        {
            return Err(RefreshFailure::Temporary);
        }
        let identity = match response.id_token {
            Some(token) => {
                let identity = self
                    .verify_codex_identity(&token)
                    .await
                    .map_err(RefreshFailure::Authentication)?;
                if identity.subject != previous.identity.subject
                    || identity.issuer != previous.identity.issuer
                    || identity.chatgpt_account_id != previous.identity.chatgpt_account_id
                {
                    return Err(RefreshFailure::Authentication(
                        SubscriptionAuthError::AccountMismatch,
                    ));
                }
                CodexIdentityRecord {
                    client_id: CODEX_CLIENT_ID.to_owned(),
                    subject: identity.subject,
                    issuer: identity.issuer,
                    id_token_expires_at_unix: identity.expires_at_unix,
                    chatgpt_account_id: identity.chatgpt_account_id,
                }
            }
            None => previous.identity.clone(),
        };
        let access_token = response
            .access_token
            .unwrap_or_else(|| tokens.access_token.clone());
        let refresh_token = response
            .refresh_token
            .unwrap_or_else(|| tokens.refresh_token.clone());
        let access_token_expires_at_unix =
            access_token_expiry(&access_token).map_err(RefreshFailure::Authentication)?;
        Ok(CodexConnectionRecord {
            version: 2,
            protocol: "codex".to_owned(),
            account: previous.account.clone(),
            identity,
            tokens: Some(CodexTokenRecord {
                access_token,
                refresh_token,
                access_token_expires_at_unix,
            }),
        })
    }

    async fn fetch_discovery(&self) -> Result<OpenIdDiscovery, SubscriptionAuthError> {
        let response = self
            .http
            .get(self.endpoints.discovery_url())
            .send()
            .await
            .map_err(|_| SubscriptionAuthError::TemporarilyUnavailable)?;
        if !response.status().is_success() {
            return Err(auth_http_failure(
                SubscriptionAuthHttpOperation::OpenIdDiscovery,
                response.status(),
            ));
        }
        let discovery: OpenIdDiscovery = response
            .json()
            .await
            .map_err(|_| SubscriptionAuthError::InvalidResponse)?;
        if discovery.issuer != self.endpoints.issuer() || discovery.jwks_uri.is_empty() {
            return Err(SubscriptionAuthError::InvalidResponse);
        }
        Ok(discovery)
    }

    async fn fetch_jwks(
        &self,
        discovery: &OpenIdDiscovery,
    ) -> Result<JsonWebKeySet, SubscriptionAuthError> {
        let response = self
            .http
            .get(&discovery.jwks_uri)
            .send()
            .await
            .map_err(|_| SubscriptionAuthError::TemporarilyUnavailable)?;
        if !response.status().is_success() {
            return Err(auth_http_failure(
                SubscriptionAuthHttpOperation::JsonWebKeySet,
                response.status(),
            ));
        }
        let keys: JsonWebKeySet = response
            .json()
            .await
            .map_err(|_| SubscriptionAuthError::InvalidResponse)?;
        if keys.keys.is_empty() || keys.keys.len() > 32 {
            return Err(SubscriptionAuthError::InvalidResponse);
        }
        Ok(keys)
    }

    async fn verify_codex_identity(
        &self,
        token: &str,
    ) -> Result<super::jwt::CodexIdentity, SubscriptionAuthError> {
        let discovery = self.fetch_discovery().await?;
        let keys = self.fetch_jwks(&discovery).await?;
        verify_codex_identity_token(token, &discovery, &keys, CODEX_CLIENT_ID, now_unix()?)
    }

    async fn revoke(&self, endpoint: &str, refresh_token: &str) -> bool {
        self.http
            .post(endpoint)
            .form(&[
                ("token", refresh_token),
                ("token_type_hint", "refresh_token"),
            ])
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
    }

    async fn revoke_codex(&self, endpoint: &str, refresh_token: &str) -> bool {
        self.http
            .post(endpoint)
            .json(&CodexRevokeRequest {
                client_id: CODEX_CLIENT_ID,
                token: refresh_token,
                token_type_hint: "refresh_token",
            })
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
    expires_in: u64,
    /// The fixture-supported server not-before value is a JSON integer Unix
    /// timestamp. Text coercions are intentionally not accepted.
    earliest_refresh_at: Option<u64>,
    scope: Option<String>,
    id_token: Option<String>,
}

#[derive(Deserialize)]
struct CodexInitialTokenResponse {
    id_token: String,
    access_token: String,
    refresh_token: String,
}

#[derive(Deserialize)]
struct CodexRefreshResponse {
    id_token: Option<String>,
    access_token: Option<String>,
    refresh_token: Option<String>,
}

#[derive(Serialize)]
struct CodexRefreshRequest<'a> {
    client_id: &'a str,
    grant_type: &'static str,
    refresh_token: &'a str,
}

#[derive(Serialize)]
struct CodexRevokeRequest<'a> {
    client_id: &'a str,
    token: &'a str,
    token_type_hint: &'static str,
}

impl TokenResponse {
    fn into_token_record(
        self,
        now: u64,
        replacement_scopes: Option<Vec<String>>,
        client_id: &str,
        subject: &str,
        issuer: &str,
    ) -> Result<TokenRecord, SubscriptionAuthError> {
        if self.access_token.is_empty()
            || self.refresh_token.is_empty()
            || self.expires_in == 0
            || self.expires_in > 31_536_000
        {
            return Err(SubscriptionAuthError::InvalidResponse);
        }
        let expires_at_unix = now
            .checked_add(self.expires_in)
            .ok_or(SubscriptionAuthError::InvalidResponse)?;
        let local_earliest_refresh_at = expires_at_unix.saturating_sub(self.expires_in.min(60));
        let earliest_refresh_at_unix = self
            .earliest_refresh_at
            .map(|server_earliest| server_earliest.max(local_earliest_refresh_at))
            .unwrap_or(local_earliest_refresh_at);
        let scopes = match replacement_scopes {
            Some(scopes) => scopes,
            None => parse_scopes(
                self.scope
                    .as_deref()
                    .ok_or(SubscriptionAuthError::InvalidResponse)?,
            )?,
        };
        Ok(TokenRecord {
            version: 1,
            client_id: client_id.to_owned(),
            subject: subject.to_owned(),
            issuer: issuer.to_owned(),
            access_token: self.access_token,
            refresh_token: self.refresh_token,
            expires_at_unix,
            earliest_refresh_at_unix,
            scopes,
        })
    }
}

#[derive(Debug, Deserialize)]
struct SiwcModelsResponse {
    models: Vec<SiwcModelResponse>,
}

#[derive(Debug, Deserialize)]
struct SiwcModelResponse {
    slug: String,
    display_name: String,
    visibility: String,
}

#[derive(Debug, Deserialize)]
struct CodexModelsResponse {
    models: Vec<ModelResponse>,
}

#[derive(Debug, Deserialize)]
struct ModelResponse {
    slug: String,
    display_name: String,
    #[serde(default)]
    default_reasoning_level: Option<String>,
    #[serde(default)]
    supported_reasoning_levels: Vec<ReasoningLevelResponse>,
    #[serde(default, deserialize_with = "optional_positive_u32")]
    context_window: Option<u32>,
    #[serde(default, deserialize_with = "optional_positive_u32")]
    max_context_window: Option<u32>,
    #[serde(default, deserialize_with = "optional_modalities")]
    input_modalities: Option<Vec<String>>,
    #[serde(default, deserialize_with = "optional_bool")]
    supports_reasoning_summary_parameter: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ReasoningLevelResponse {
    effort: String,
    description: String,
}

fn siwc_subscription_models(
    models: Vec<SiwcModelResponse>,
) -> Result<Vec<SubscriptionModel>, SubscriptionAuthError> {
    if models.iter().any(|model| {
        model.slug.is_empty()
            || model.slug.len() > 256
            || model.display_name.is_empty()
            || model.display_name.len() > 256
    }) {
        return Err(SubscriptionAuthError::InvalidResponse);
    }
    Ok(models
        .into_iter()
        .map(|model| SubscriptionModel {
            slug: model.slug,
            display_name: model.display_name,
            default_reasoning_level: None,
            supported_reasoning_levels: Vec::new(),
            context_window: None,
            max_context_window: None,
            input_modalities: None,
            supports_reasoning_summary_parameter: None,
            reasoning_levels_advertised: true,
        })
        .collect())
}

fn codex_subscription_models(
    models: Vec<ModelResponse>,
) -> Result<Vec<SubscriptionModel>, SubscriptionAuthError> {
    let mut slugs = BTreeSet::new();
    for model in &models {
        if model.slug.is_empty()
            || model.slug.len() > 256
            || model.display_name.is_empty()
            || model.display_name.len() > 256
            || !slugs.insert(&model.slug)
        {
            return Err(SubscriptionAuthError::InvalidResponse);
        }
        if let Some(default) = &model.default_reasoning_level
            && !valid_reasoning_metadata(default, 64)
        {
            return Err(SubscriptionAuthError::InvalidResponse);
        }
        let mut efforts = BTreeSet::new();
        for level in &model.supported_reasoning_levels {
            if !valid_reasoning_metadata(&level.effort, 64)
                || level.description.is_empty()
                || level.description.len() > 512
                || !efforts.insert(&level.effort)
            {
                return Err(SubscriptionAuthError::InvalidResponse);
            }
        }
        if model.input_modalities.as_ref().is_some_and(|modalities| {
            modalities.len() > 32
                || modalities
                    .iter()
                    .any(|value| !valid_reasoning_metadata(value, 64))
        }) {
            return Err(SubscriptionAuthError::InvalidResponse);
        }
    }
    Ok(models
        .into_iter()
        .map(|model| SubscriptionModel {
            slug: model.slug,
            display_name: model.display_name,
            default_reasoning_level: model.default_reasoning_level,
            supported_reasoning_levels: model
                .supported_reasoning_levels
                .into_iter()
                .map(|level| SubscriptionReasoningLevel {
                    effort: level.effort,
                    description: level.description,
                })
                .collect(),
            context_window: model.context_window,
            max_context_window: model.max_context_window,
            input_modalities: model.input_modalities,
            supports_reasoning_summary_parameter: model.supports_reasoning_summary_parameter,
            reasoning_levels_advertised: true,
        })
        .collect())
}

fn optional_positive_u32<'de, D>(deserializer: D) -> Result<Option<u32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    match Option::<u32>::deserialize(deserializer)? {
        Some(value) if value > 0 => Ok(Some(value)),
        Some(_) => Err(serde::de::Error::custom(
            "expected a positive unsigned integer",
        )),
        None => Ok(None),
    }
}

fn optional_modalities<'de, D>(deserializer: D) -> Result<Option<Vec<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let values = Vec::<String>::deserialize(deserializer)?;
    Ok(Some(values))
}

fn optional_bool<'de, D>(deserializer: D) -> Result<Option<bool>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    bool::deserialize(deserializer).map(Some)
}

fn valid_reasoning_metadata(value: &str, maximum_length: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum_length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

enum RefreshFailure {
    Temporary,
    Terminal,
    InvalidClient,
    Authentication(SubscriptionAuthError),
}

#[derive(Deserialize)]
struct OAuthErrorResponse {
    error: Option<String>,
}

async fn classify_refresh_response(response: reqwest::Response) -> RefreshFailure {
    if temporary_status(response.status()) {
        return RefreshFailure::Temporary;
    }
    let code = response
        .json::<OAuthErrorResponse>()
        .await
        .ok()
        .and_then(|body| body.error)
        .filter(|code| {
            code.len() <= 128
                && code
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
        });
    match code.as_deref() {
        Some(
            "invalid_grant"
            | "invalid_refresh_token"
            | "token_expired"
            | "refresh_token_expired"
            | "refresh_token_invalidated"
            | "refresh_token_reused",
        ) => RefreshFailure::Terminal,
        Some("invalid_client") => RefreshFailure::InvalidClient,
        _ => RefreshFailure::Temporary,
    }
}

async fn classify_codex_refresh_response(response: reqwest::Response) -> RefreshFailure {
    let status = response.status();
    if status == StatusCode::UNAUTHORIZED {
        return RefreshFailure::Terminal;
    }
    let code = bounded_codex_refresh_error_code(response).await;
    match code.as_deref() {
        Some("refresh_token_expired" | "refresh_token_invalidated" | "refresh_token_reused") => {
            RefreshFailure::Terminal
        }
        Some("invalid_grant") if status == StatusCode::BAD_REQUEST => RefreshFailure::Terminal,
        _ => RefreshFailure::Temporary,
    }
}

async fn bounded_codex_refresh_error_code(mut response: reqwest::Response) -> Option<String> {
    let mut body = Vec::new();
    loop {
        let chunk = match response.chunk().await {
            Ok(Some(chunk)) => chunk,
            Ok(None) => break,
            Err(_) => return None,
        };
        if chunk.len() > CODEX_REFRESH_ERROR_BODY_BYTES.saturating_sub(body.len()) {
            return None;
        }
        body.extend_from_slice(&chunk);
    }
    let value: serde_json::Value = serde_json::from_slice(&body).ok()?;
    let code = value
        .get("error")
        .and_then(|error| match error {
            serde_json::Value::Object(error) => {
                error.get("code").and_then(serde_json::Value::as_str)
            }
            serde_json::Value::String(code) => Some(code.as_str()),
            _ => None,
        })
        .or_else(|| value.get("code").and_then(serde_json::Value::as_str))?;
    (code.len() <= 128
        && code
            .bytes()
            .all(|byte| byte.is_ascii_alphabetic() || byte == b'_'))
    .then(|| code.to_ascii_lowercase())
}

struct LoopbackListener {
    listener: TcpListener,
    redirect_uri: String,
}

impl LoopbackListener {
    async fn bind() -> Result<Self, SubscriptionAuthError> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(|_| SubscriptionAuthError::StorageUnavailable)?;
        let address = listener
            .local_addr()
            .map_err(|_| SubscriptionAuthError::StorageUnavailable)?;
        Ok(Self {
            listener,
            redirect_uri: format!("http://127.0.0.1:{}/auth/callback", address.port()),
        })
    }

    async fn bind_codex() -> Result<Self, SubscriptionAuthError> {
        for port in [1455, 1457] {
            match TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await {
                Ok(listener) => {
                    return Ok(Self {
                        listener,
                        redirect_uri: format!("http://localhost:{port}/auth/callback"),
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => continue,
                Err(_) => return Err(SubscriptionAuthError::StorageUnavailable),
            }
        }
        Err(SubscriptionAuthError::StorageUnavailable)
    }

    async fn await_callback(self) -> Result<OAuthCallback, SubscriptionAuthError> {
        timeout(LOGIN_TIMEOUT, async {
            loop {
                let (stream, _) = self
                    .listener
                    .accept()
                    .await
                    .map_err(|_| SubscriptionAuthError::StorageUnavailable)?;
                match read_callback(stream).await? {
                    Some(callback) => return Ok(callback),
                    None => continue,
                }
            }
        })
        .await
        .map_err(|_| SubscriptionAuthError::LoginTimedOut)?
    }
}

struct OAuthCallback {
    code: Option<String>,
    state: Option<String>,
    client_id: Option<String>,
    denied: bool,
}

fn validate_callback(
    callback: OAuthCallback,
    expected_state: &str,
) -> Result<OAuthCallback, SubscriptionAuthError> {
    let callback_state = callback
        .state
        .as_deref()
        .ok_or(SubscriptionAuthError::InvalidCallback)?;
    if !constant_time_equal(expected_state.as_bytes(), callback_state.as_bytes()) {
        return Err(SubscriptionAuthError::InvalidCallback);
    }
    if callback.denied {
        return Err(SubscriptionAuthError::AuthorizationDenied);
    }
    Ok(callback)
}

async fn read_callback(
    mut stream: TcpStream,
) -> Result<Option<OAuthCallback>, SubscriptionAuthError> {
    let mut bytes = Vec::with_capacity(1024);
    let mut buffer = [0; 1024];
    loop {
        let read = timeout(CALLBACK_READ_TIMEOUT, stream.read(&mut buffer))
            .await
            .map_err(|_| SubscriptionAuthError::InvalidCallback)?
            .map_err(|_| SubscriptionAuthError::InvalidCallback)?;
        if read == 0 {
            return Err(SubscriptionAuthError::InvalidCallback);
        }
        bytes.extend_from_slice(&buffer[..read]);
        if bytes.len() > CALLBACK_MAX_BYTES {
            respond_callback(&mut stream, false).await?;
            return Ok(None);
        }
        if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    let request =
        std::str::from_utf8(&bytes).map_err(|_| SubscriptionAuthError::InvalidCallback)?;
    let Some(line) = request.lines().next() else {
        return Err(SubscriptionAuthError::InvalidCallback);
    };
    let mut fields = line.split_whitespace();
    let Some("GET") = fields.next() else {
        respond_callback(&mut stream, false).await?;
        return Ok(None);
    };
    let Some(target) = fields.next() else {
        respond_callback(&mut stream, false).await?;
        return Ok(None);
    };
    if fields.next().is_none() || !target.starts_with("/auth/callback?") {
        respond_callback(&mut stream, false).await?;
        return Ok(None);
    }
    let parameters = parse_query(&target["/auth/callback?".len()..])?;
    respond_callback(&mut stream, true).await?;
    Ok(Some(OAuthCallback {
        code: parameters.get("code").cloned(),
        state: parameters.get("state").cloned(),
        client_id: parameters.get("client_id").cloned(),
        denied: parameters.contains_key("error"),
    }))
}

async fn respond_callback(
    stream: &mut TcpStream,
    accepted: bool,
) -> Result<(), SubscriptionAuthError> {
    let (status, body) = if accepted {
        (
            "200 OK",
            "Echidna authorization callback received. Return to the terminal for the sign-in result.",
        )
    } else {
        ("400 Bad Request", "Invalid Echidna sign-in callback.")
    };
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .await
        .map_err(|_| SubscriptionAuthError::InvalidCallback)
}

fn parse_query(
    input: &str,
) -> Result<std::collections::BTreeMap<String, String>, SubscriptionAuthError> {
    let mut values = std::collections::BTreeMap::new();
    for pair in input.split('&') {
        let (key, value) = pair
            .split_once('=')
            .ok_or(SubscriptionAuthError::InvalidCallback)?;
        let key = percent_decode(key)?;
        let value = percent_decode(value)?;
        if key.is_empty() || value.len() > 4096 || values.insert(key, value).is_some() {
            return Err(SubscriptionAuthError::InvalidCallback);
        }
    }
    Ok(values)
}

fn percent_decode(value: &str) -> Result<String, SubscriptionAuthError> {
    let mut output = Vec::with_capacity(value.len());
    let mut bytes = value.as_bytes().iter().copied();
    while let Some(byte) = bytes.next() {
        match byte {
            b'+' => output.push(b' '),
            b'%' => {
                let high = hex(bytes.next().ok_or(SubscriptionAuthError::InvalidCallback)?)?;
                let low = hex(bytes.next().ok_or(SubscriptionAuthError::InvalidCallback)?)?;
                output.push(high << 4 | low);
            }
            _ => output.push(byte),
        }
    }
    String::from_utf8(output).map_err(|_| SubscriptionAuthError::InvalidCallback)
}

fn hex(byte: u8) -> Result<u8, SubscriptionAuthError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(SubscriptionAuthError::InvalidCallback),
    }
}

fn authorization_url(
    endpoint: &str,
    client_id: &str,
    host_id: &str,
    redirect_uri: &str,
    state: &str,
    nonce: &str,
    challenge: &str,
) -> String {
    let parameters = [
        ("client_id", client_id),
        ("agent_name_hint", AGENT_NAME_HINT),
        ("ext_agent_host_id", host_id),
        ("response_type", "code"),
        ("redirect_uri", redirect_uri),
        ("scope", REQUESTED_SCOPE),
        ("resource", API_BASE_URL),
        ("state", state),
        ("nonce", nonce),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
    ];
    let query = parameters
        .iter()
        .map(|(key, value)| format!("{key}={}", percent_encode(value)))
        .collect::<Vec<_>>()
        .join("&");
    format!("{endpoint}?{query}")
}

fn codex_authorization_url(
    endpoint: &str,
    redirect_uri: &str,
    state: &str,
    challenge: &str,
) -> String {
    let parameters = [
        ("response_type", "code"),
        ("client_id", CODEX_CLIENT_ID),
        ("redirect_uri", redirect_uri),
        ("scope", CODEX_REQUESTED_SCOPE),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
        ("id_token_add_organizations", "true"),
        ("codex_cli_simplified_flow", "true"),
        ("state", state),
        ("originator", CODEX_ORIGINATOR),
    ];
    let query = parameters
        .iter()
        .map(|(key, value)| format!("{key}={}", percent_encode(value)))
        .collect::<Vec<_>>()
        .join("&");
    format!("{endpoint}?{query}")
}

fn percent_encode(value: &str) -> String {
    value
        .bytes()
        .flat_map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                vec![byte as char].into_iter().collect::<Vec<_>>()
            }
            _ => format!("%{byte:02X}").chars().collect(),
        })
        .collect()
}

fn pkce_challenge(verifier: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest(&SHA256, verifier.as_bytes()))
}

fn random_url_safe(bytes: usize) -> Result<String, SubscriptionAuthError> {
    let mut value = vec![0_u8; bytes];
    SystemRandom::new()
        .fill(&mut value)
        .map_err(|_| SubscriptionAuthError::StorageUnavailable)?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value))
}

fn access_token_expiry(token: &str) -> Result<u64, SubscriptionAuthError> {
    #[derive(Deserialize)]
    struct AccessTokenClaims {
        exp: u64,
    }

    let mut segments = token.split('.');
    let _header = segments
        .next()
        .filter(|segment| !segment.is_empty())
        .ok_or(SubscriptionAuthError::InvalidResponse)?;
    let payload = segments
        .next()
        .filter(|segment| !segment.is_empty())
        .ok_or(SubscriptionAuthError::InvalidResponse)?;
    let _signature = segments
        .next()
        .filter(|segment| !segment.is_empty())
        .ok_or(SubscriptionAuthError::InvalidResponse)?;
    if segments.next().is_some() {
        return Err(SubscriptionAuthError::InvalidResponse);
    }
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(payload))
        .map_err(|_| SubscriptionAuthError::InvalidResponse)?;
    let claims: AccessTokenClaims =
        serde_json::from_slice(&payload).map_err(|_| SubscriptionAuthError::InvalidResponse)?;
    if claims.exp == 0 {
        return Err(SubscriptionAuthError::InvalidResponse);
    }
    Ok(claims.exp)
}

fn codex_credential(connection: &CodexConnectionRecord) -> CodexSubscriptionCredential {
    let tokens = connection
        .tokens
        .as_ref()
        .expect("validated Codex connection has tokens");
    CodexSubscriptionCredential {
        bearer: tokens.access_token.clone(),
        account_id: connection.identity.chatgpt_account_id.clone(),
    }
}

fn parse_scopes(scope: &str) -> Result<Vec<String>, SubscriptionAuthError> {
    let scopes: Vec<String> = scope
        .split_ascii_whitespace()
        .map(ToOwned::to_owned)
        .collect();
    if scopes.is_empty() || scopes.iter().any(|scope| scope.len() > 128) {
        return Err(SubscriptionAuthError::InvalidResponse);
    }
    Ok(scopes)
}

fn has_direct_scope(scopes: &[String]) -> bool {
    scopes.iter().any(|scope| scope == DIRECT_USE_SCOPE)
}

fn require_direct_scope(scopes: &[String]) -> Result<(), SubscriptionAuthError> {
    if has_direct_scope(scopes) {
        Ok(())
    } else {
        Err(SubscriptionAuthError::DirectTokenScopeMissing)
    }
}

fn now_unix() -> Result<u64, SubscriptionAuthError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| SubscriptionAuthError::StorageUnavailable)
}

fn classify_http_status(status: StatusCode) -> SubscriptionAuthError {
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

fn auth_http_failure(
    operation: SubscriptionAuthHttpOperation,
    status: StatusCode,
) -> SubscriptionAuthError {
    SubscriptionAuthError::AuthHttpFailure {
        operation,
        status: status.as_u16(),
    }
}

fn temporary_status(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}
