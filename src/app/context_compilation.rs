//! Cache-aware Fragment rendering before the pure Context Planner runs.

use thiserror::Error;

use crate::domain::context::{
    ContextFragment, ContextFragmentCacheKey, ContextPlannerError, FragmentCachePolicy,
};
use crate::ports::context::{ContextFragmentCache, ContextFragmentCacheError};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FragmentCacheDisposition {
    Disabled,
    Bypassed,
    Hit,
    Miss,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ContextCacheSummary {
    pub disabled: usize,
    pub bypassed: usize,
    pub hits: usize,
    pub misses: usize,
}

impl ContextCacheSummary {
    pub fn record(&mut self, disposition: FragmentCacheDisposition) {
        match disposition {
            FragmentCacheDisposition::Disabled => self.disabled += 1,
            FragmentCacheDisposition::Bypassed => self.bypassed += 1,
            FragmentCacheDisposition::Hit => self.hits += 1,
            FragmentCacheDisposition::Miss => self.misses += 1,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedContextFragment {
    pub fragment: ContextFragment,
    pub cache_disposition: FragmentCacheDisposition,
}

pub fn resolve_context_fragment(
    cache: Option<&dyn ContextFragmentCache>,
    key: &ContextFragmentCacheKey,
    renderer: impl FnOnce() -> Result<ContextFragment, ContextPlannerError>,
) -> Result<ResolvedContextFragment, ContextCompilationError> {
    if key.cache_policy() == FragmentCachePolicy::DoNotCache {
        return Ok(ResolvedContextFragment {
            fragment: renderer()?,
            cache_disposition: FragmentCacheDisposition::Bypassed,
        });
    }
    let Some(cache) = cache else {
        return Ok(ResolvedContextFragment {
            fragment: renderer()?,
            cache_disposition: FragmentCacheDisposition::Disabled,
        });
    };
    if let Some(fragment) = cache.lookup(key)?
        && key.matches_fragment(&fragment)
    {
        return Ok(ResolvedContextFragment {
            fragment,
            cache_disposition: FragmentCacheDisposition::Hit,
        });
    }
    let fragment = renderer()?;
    if !key.matches_fragment(&fragment) {
        return Err(ContextCompilationError::RenderedIdentityMismatch);
    }
    cache.store(key, &fragment)?;
    Ok(ResolvedContextFragment {
        fragment,
        cache_disposition: FragmentCacheDisposition::Miss,
    })
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ContextCompilationError {
    #[error(transparent)]
    Planner(#[from] ContextPlannerError),
    #[error(transparent)]
    Cache(#[from] ContextFragmentCacheError),
    #[error("rendered Context Fragment does not match its pre-render Cache key")]
    RenderedIdentityMismatch,
}
