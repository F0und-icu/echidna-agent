//! Conservative request admission shared by the fixed Primary/Vision path.

use std::sync::Mutex;

use crate::domain::context::ProviderSessionRole;

pub(crate) const DEFAULT_MODEL_REQUEST_LIMIT: usize = 32;
pub(crate) const MAX_MODEL_REQUEST_LIMIT: usize = 10_000;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RequestCounts {
    pub(crate) primary: usize,
    pub(crate) vision: usize,
}

impl RequestCounts {
    pub(crate) fn total(self) -> usize {
        self.primary + self.vision
    }
}

pub(crate) struct ModelRequestBudget {
    limit: Option<usize>,
    counts: Mutex<RequestCounts>,
}

impl ModelRequestBudget {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            limit: (limit != 0).then_some(limit),
            counts: Mutex::new(RequestCounts::default()),
        }
    }

    pub(crate) fn limit(&self) -> Option<usize> {
        self.limit
    }

    pub(crate) fn remaining(&self) -> Option<usize> {
        self.limit
            .map(|limit| limit.saturating_sub(self.counts().total()))
    }

    pub(crate) fn counts(&self) -> RequestCounts {
        *self.counts.lock().expect("request admission lock poisoned")
    }

    /// Reserve before audit/dispatch. A later local or transport failure is
    /// not refunded; admitted attempts are not claimed as delivered requests.
    pub(crate) fn admit(&self, role: ProviderSessionRole) -> Result<(), usize> {
        let mut counts = self.counts.lock().expect("request admission lock poisoned");
        if let Some(limit) = self.limit
            && counts.total() >= limit
        {
            return Err(limit);
        }
        match role {
            ProviderSessionRole::Primary => counts.primary += 1,
            ProviderSessionRole::Vision => counts.vision += 1,
        }
        Ok(())
    }
}
