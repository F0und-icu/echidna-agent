//! One input-receive clock boundary, sampled once before a line is queued.

use crate::domain::session::TaskInputTime;

#[derive(Debug, thiserror::Error)]
#[error("input clock failed: {0}")]
pub(crate) struct InputClockError(#[source] Box<dyn std::error::Error + Send + Sync>);

impl InputClockError {
    pub(crate) fn new(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self(Box::new(error))
    }
}

pub(crate) trait InputClock: Send + Sync {
    fn now(&self) -> Result<TaskInputTime, InputClockError>;
}
