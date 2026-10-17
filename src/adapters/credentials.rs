//! Explicit credential injection shared by live Provider bindings.

use std::env;

use futures_util::future::BoxFuture;
use thiserror::Error;

/// Injected source for resolving one configured Cyber Agent credential
/// reference. Implementations return only the requested value and never
/// inspect Codex or generic Provider login state.
pub trait CredentialSource: Send + Sync {
    fn read(&self, reference: &str) -> Result<Option<String>, CredentialSourceError>;

    /// Static credentials retain their immediate validation. Sources that
    /// maintain rotating credentials may defer resolution until stream polling.
    fn resolve(&self, reference: &str) -> CredentialResolution {
        CredentialResolution::Ready(self.read(reference))
    }
}

/// An owned pending resolution never borrows a Provider request or starts a
/// second runtime. Awaiting it is credential maintenance, not request retry.
pub enum CredentialResolution {
    Ready(Result<Option<String>, CredentialSourceError>),
    Pending(BoxFuture<'static, Result<Option<String>, CredentialSourceError>>),
    PendingAccountBound(BoxFuture<'static, Result<AccountBoundCredential, CredentialSourceError>>),
}

/// One bearer credential paired with its verified ChatGPT account identity.
///
/// The paired value is required by the finite Codex subscription profile. Its
/// fields remain private so only the Responses transport validates and emits
/// the account header.
pub struct AccountBoundCredential {
    bearer: String,
    account_id: String,
}

impl AccountBoundCredential {
    pub fn new(bearer: String, account_id: String) -> Self {
        Self { bearer, account_id }
    }

    pub(crate) fn into_parts(self) -> (String, String) {
        (self.bearer, self.account_id)
    }
}

impl std::fmt::Debug for AccountBoundCredential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("AccountBoundCredential(<redacted>)")
    }
}

/// Reads only the exact environment reference selected by configuration.
#[derive(Clone, Copy, Debug, Default)]
pub struct ProcessEnvironmentCredentialSource;

impl CredentialSource for ProcessEnvironmentCredentialSource {
    fn read(&self, reference: &str) -> Result<Option<String>, CredentialSourceError> {
        match env::var(reference) {
            Ok(value) => Ok(Some(value)),
            Err(env::VarError::NotPresent) => Ok(None),
            Err(env::VarError::NotUnicode(_)) => Err(CredentialSourceError::ReadFailed {
                message: "credential value is not valid Unicode".to_string(),
            }),
        }
    }
}

/// Failure reported by an injected credential source without exposing a
/// secret.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum CredentialSourceError {
    #[error("credential source failed: {message}")]
    ReadFailed { message: String },
}
