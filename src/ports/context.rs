//! Narrow I/O boundary for rebuildable rendered Context Fragments.

use thiserror::Error;

use crate::domain::context::{ContextFragment, ContextFragmentCacheKey};

pub trait ContextFragmentCache: Send + Sync {
    fn lookup(
        &self,
        key: &ContextFragmentCacheKey,
    ) -> Result<Option<ContextFragment>, ContextFragmentCacheError>;

    fn store(
        &self,
        key: &ContextFragmentCacheKey,
        fragment: &ContextFragment,
    ) -> Result<(), ContextFragmentCacheError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContextFragmentCacheOperation {
    Lookup,
    Store,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
#[error("Context Fragment Cache {operation:?} failed: {message}")]
pub struct ContextFragmentCacheError {
    pub operation: ContextFragmentCacheOperation,
    pub message: String,
}

impl ContextFragmentCacheError {
    pub fn lookup(error: &impl std::error::Error) -> Self {
        Self {
            operation: ContextFragmentCacheOperation::Lookup,
            message: error.to_string(),
        }
    }

    pub fn store(error: &impl std::error::Error) -> Self {
        Self {
            operation: ContextFragmentCacheOperation::Store,
            message: error.to_string(),
        }
    }
}
