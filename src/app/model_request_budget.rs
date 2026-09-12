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
    limit: usize,
    counts: Mutex<RequestCounts>,
}

impl ModelRequestBudget {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            limit,
            counts: Mutex::new(RequestCounts::default()),
        }
    }

    pub(crate) fn limit(&self) -> usize {
        self.limit
    }

    pub(crate) fn counts(&self) -> RequestCounts {
        *self.counts.lock().expect("request admission lock poisoned")
    }

    /// Reserve before audit/dispatch. A later local or transport failure is
    /// not refunded; admitted attempts are not claimed as delivered requests.
    pub(crate) fn admit(&self, role: ProviderSessionRole) -> Result<(), usize> {
        let mut counts = self.counts.lock().expect("request admission lock poisoned");
        if counts.total() >= self.limit {
            return Err(self.limit);
        }
        match role {
            ProviderSessionRole::Primary => counts.primary += 1,
            ProviderSessionRole::Vision => counts.vision += 1,
        }
        Ok(())
    }
}
