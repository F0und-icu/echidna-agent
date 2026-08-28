//! Explicit credential injection shared by live Provider bindings.

use std::env;

use thiserror::Error;

/// Injected source for resolving one configured Cyber Agent credential
/// reference. Implementations return only the requested value and never
/// inspect Codex or generic Provider login state.
pub trait CredentialSource: Send + Sync {
    fn read(&self, reference: &str) -> Result<Option<String>, CredentialSourceError>;
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
