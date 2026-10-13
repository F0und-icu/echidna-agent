use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use reqwest::StatusCode;
use ring::digest::{SHA256, digest};
use ring::rand::{SecureRandom, SystemRandom};
use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio::time::timeout;

use super::jwt::{JsonWebKeySet, OpenIdDiscovery, constant_time_equal, verify_identity_token};
use super::store::{AuthStore, ConnectionRecord, RegistrationRecord, TokenRecord};
use super::types::{
    DIRECT_USE_SCOPE, SubscriptionAuthError, SubscriptionAuthStatus, SubscriptionConnectionState,
    SubscriptionLogoutOutcome, SubscriptionModel, validate_constructor_input,
};
use super::{API_BASE_URL, AuthEndpoints};

const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);
const CALLBACK_READ_TIMEOUT: Duration = Duration::from_secs(10);
const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const CALLBACK_MAX_BYTES: usize = 16 * 1024;
const DYNAMIC_AGENT_CLIENT: &str = "dynamic_agent_client";
const AGENT_NAME_HINT: &str = "Echidna";
const REQUESTED_SCOPE: &str =
    "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";

/// App-owned OAuth lifecycle for one explicit local subscription account.
/// Construction is deliberately free of filesystem, credential, and network I/O.
#[derive(Clone)]
pub(crate) struct SubscriptionAuthService {
    store: AuthStore,
    endpoints: AuthEndpoints,
    http: reqwest::Client,
    refresh_mutex: Arc<Mutex<()>>,
}

impl SubscriptionAuthService {
    pub(crate) fn new(auth_store: PathBuf, account: String) -> Result<Self, SubscriptionAuthError> {
        Self::with_endpoints(auth_store, account, AuthEndpoints::production())
    }

    /// The production constructor is the sole runtime caller. This supports
    /// local loopback fixtures without exposing an endpoint override in Config.
    pub(crate) fn with_endpoints(
        auth_store: PathBuf,
        account: String,
        endpoints: AuthEndpoints,
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
            http,
            refresh_mutex: Arc::new(Mutex::new(())),
        })
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
            &self.endpoints.authorize_url,
            requested_client_id,
            &host.host_id,
            &redirect_uri,
            &state,
            &nonce,
            &challenge,
        );
        present_authorization_url(&url)
            .map_err(|_| SubscriptionAuthError::LoginUrlPresentationFailed)?;
        let callback = listener.await_callback().await?;
        let callback_state = callback
            .state
            .ok_or(SubscriptionAuthError::InvalidCallback)?;
        if !constant_time_equal(state.as_bytes(), callback_state.as_bytes()) {
            return Err(SubscriptionAuthError::InvalidCallback);
        }
        if callback.denied {
            return Err(SubscriptionAuthError::AuthorizationDenied);
        }
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
                account: self.account_label()?,
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
            account: self.account_label()?,
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
        }
    }

    /// Performs the explicit, account-scoped model discovery path after
    /// obtaining a usable subscription bearer.
    pub(crate) async fn models(&self) -> Result<Vec<SubscriptionModel>, SubscriptionAuthError> {
        let token = self.access_token().await?;
        let response = self
            .http
            .get(format!(
                "{}/models",
                self.endpoints.api_base_url.trim_end_matches('/')
            ))
            .bearer_auth(token)
            .send()
            .await
            .map_err(|_| SubscriptionAuthError::TemporarilyUnavailable)?;
        if !response.status().is_success() {
            return Err(classify_http_status(response.status()));
        }
        let models: ModelsResponse = response
            .json()
            .await
            .map_err(|_| SubscriptionAuthError::InvalidResponse)?;
        if models.models.iter().any(|model| {
            model.slug.is_empty()
                || model.slug.len() > 256
                || model.display_name.is_empty()
                || model.display_name.len() > 256
        }) {
            return Err(SubscriptionAuthError::InvalidResponse);
        }
        Ok(models
            .models
            .into_iter()
            .filter(|model| model.visibility == "list")
            .map(|model| SubscriptionModel {
                slug: model.slug,
                display_name: model.display_name,
            })
            .collect())
    }

    /// Clears app-owned tokens even when revocation cannot be confirmed. Host
    /// and registration records remain so a later reconnect reuses the client.
    pub(crate) async fn logout(&self) -> Result<SubscriptionLogoutOutcome, SubscriptionAuthError> {
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

    fn valid_connection(&self) -> Result<Option<ConnectionRecord>, SubscriptionAuthError> {
        let connection = self.store.read_connection()?;
        let Some(connection) = connection else {
            return Ok(None);
        };
        let registration = &connection.registration;
        if connection.version != 1
            || registration.version != 1
            || registration.account != self.account_label()?
            || registration.client_id.is_empty()
            || registration.client_id.len() > 512
            || registration.subject.as_deref().is_none_or(str::is_empty)
            || registration.issuer.as_deref().is_none_or(str::is_empty)
        {
            return Err(SubscriptionAuthError::MalformedStorage);
        }
        if let Some(tokens) = connection.tokens.as_ref()
            && (tokens.version != 1
                || tokens.client_id != registration.client_id
                || tokens.subject != registration.subject.as_deref().unwrap_or_default()
                || tokens.issuer != registration.issuer.as_deref().unwrap_or_default()
                || tokens.access_token.is_empty()
                || tokens.refresh_token.is_empty()
                || tokens.scopes.iter().any(|scope| scope.is_empty()))
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

    fn account_label(&self) -> Result<String, SubscriptionAuthError> {
        Ok(self.store.account_label())
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
            .post(&self.endpoints.token_url)
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
            return Err(classify_http_status(response.status()));
        }
        response
            .json()
            .await
            .map_err(|_| SubscriptionAuthError::InvalidResponse)
    }

    async fn refresh(&self, previous: &TokenRecord) -> Result<TokenRecord, RefreshFailure> {
        let connection = self
            .valid_connection()
            .map_err(|_| RefreshFailure::Terminal)?
            .ok_or(RefreshFailure::Terminal)?;
        let registration = connection.registration;
        let response = self
            .http
            .post(&self.endpoints.token_url)
            .form(&[
                ("grant_type", "refresh_token"),
                ("client_id", registration.client_id.as_str()),
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
                &registration.client_id,
                registration
                    .subject
                    .as_deref()
                    .ok_or(RefreshFailure::Terminal)?,
                registration
                    .issuer
                    .as_deref()
                    .ok_or(RefreshFailure::Terminal)?,
            )
            .map_err(|_| RefreshFailure::Temporary)?;
        require_direct_scope(&refreshed.scopes).map_err(|_| RefreshFailure::Terminal)?;
        Ok(refreshed)
    }

    async fn fetch_discovery(&self) -> Result<OpenIdDiscovery, SubscriptionAuthError> {
        let response = self
            .http
            .get(&self.endpoints.discovery_url)
            .send()
            .await
            .map_err(|_| SubscriptionAuthError::TemporarilyUnavailable)?;
        if !response.status().is_success() {
            return Err(classify_http_status(response.status()));
        }
        let discovery: OpenIdDiscovery = response
            .json()
            .await
            .map_err(|_| SubscriptionAuthError::InvalidResponse)?;
        if discovery.issuer != self.endpoints.issuer || discovery.jwks_uri.is_empty() {
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
            return Err(classify_http_status(response.status()));
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
struct ModelsResponse {
    models: Vec<ModelResponse>,
}

#[derive(Debug, Deserialize)]
struct ModelResponse {
    slug: String,
    display_name: String,
    visibility: String,
}

enum RefreshFailure {
    Temporary,
    Terminal,
    InvalidClient,
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
            "Echidna sign-in complete. You may return to the terminal.",
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

fn temporary_status(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}
