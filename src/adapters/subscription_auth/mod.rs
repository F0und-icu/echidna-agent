//! App-owned, explicit ChatGPT subscription authentication.
//!
//! This adapter never reads Codex/browser state. Its fixed production endpoints
//! and private, account-scoped store are intentionally separate from provider
//! configuration and the existing Responses protocol owner.

mod jwt;
mod oauth;
mod store;
mod types;

pub(crate) use oauth::SubscriptionAuthService;
pub(crate) use types::{API_BASE_URL, AuthEndpoints};
