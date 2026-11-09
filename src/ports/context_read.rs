//! Finite scoped source-reading boundary used by the context_read Tool.

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Debug, Deserialize, Serialize)]
// The advertised Tool schema is flat across operations. Serde ignores fields
// irrelevant to the selected operation; its required fields stay typed.
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum ContextReadRequest {
    Search {
        query: String,
        after_unix: Option<u64>,
        before_unix: Option<u64>,
        max_bytes: usize,
        #[serde(default)]
        cursor: Option<String>,
    },
    Read {
        reference: String,
        #[serde(default)]
        offset: usize,
        max_bytes: usize,
    },
    Progress {
        task_ref: String,
        max_bytes: usize,
    },
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ContextReadStatus {
    Found,
    NotFound,
    Stale,
    Ambiguous,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct ContextReadEntry {
    pub(crate) reference: String,
    pub(crate) scope: String,
    pub(crate) source_revision: String,
    pub(crate) source_role: Option<crate::domain::model::MessageRole>,
    pub(crate) source_origin: crate::domain::context::ContextOrigin,
    pub(crate) semantic_class: crate::domain::context::SemanticClass,
    pub(crate) source_authority: crate::domain::context::AuthorityLevel,
    pub(crate) observed_at: Option<String>,
    pub(crate) valid_time: Option<String>,
    pub(crate) offset: usize,
    pub(crate) next_offset: Option<usize>,
    pub(crate) total_bytes: usize,
    pub(crate) text: String,
    pub(crate) truncated: bool,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct ContextReadResponse {
    pub(crate) status: ContextReadStatus,
    pub(crate) entries: Vec<ContextReadEntry>,
    pub(crate) truncated: bool,
    pub(crate) next_cursor: Option<String>,
}

#[derive(Debug, Error)]
#[error("context source read failed: {message}")]
pub(crate) struct ContextReadError {
    pub(crate) message: String,
}

pub(crate) trait ContextReadPort: Send + Sync {
    fn read(&self, request: ContextReadRequest) -> Result<ContextReadResponse, ContextReadError>;
}
