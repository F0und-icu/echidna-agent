//! OpenAI credential injection without Codex or generic environment fallbacks.

use std::env;

use thiserror::Error;

/// The only environment credential reference used by the MVP OpenAI Adapter.
pub const OPENAI_API_KEY_REFERENCE: &str = "CYBER_AGENT_OPENAI_API_KEY";

/// Injected source for resolving the dedicated OpenAI credential reference.
pub trait OpenAiCredentialSource: Send + Sync {
    fn read(&self, reference: &str) -> Result<Option<String>, OpenAiCredentialSourceError>;
}

/// Reads only the exact reference requested by the OpenAI Adapter.
#[derive(Clone, Copy, Debug, Default)]
pub struct ProcessEnvironmentCredentialSource;

impl OpenAiCredentialSource for ProcessEnvironmentCredentialSource {
    fn read(&self, reference: &str) -> Result<Option<String>, OpenAiCredentialSourceError> {
        match env::var(reference) {
            Ok(value) => Ok(Some(value)),
            Err(env::VarError::NotPresent) => Ok(None),
            Err(env::VarError::NotUnicode(_)) => Err(OpenAiCredentialSourceError::ReadFailed {
                message: "credential value is not valid Unicode".to_string(),
            }),
        }
    }
}

/// Failure reported by an injected credential source without exposing a secret.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum OpenAiCredentialSourceError {
    #[error("credential source failed: {message}")]
    ReadFailed { message: String },
}
