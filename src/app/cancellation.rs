//! One cancellation authority shared by model Streaming and Tool execution.

use std::future::poll_fn;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Poll, Waker};

#[derive(Clone)]
pub struct AgentCancellation {
    state: Arc<CancellationState>,
}

struct CancellationState {
    cancelled: AtomicBool,
    waiting_waker: Mutex<Option<Waker>>,
}

impl AgentCancellation {
    pub fn new() -> Self {
        Self {
            state: Arc::new(CancellationState {
                cancelled: AtomicBool::new(false),
                waiting_waker: Mutex::new(None),
            }),
        }
    }

    pub fn cancel(&self) {
        if !self.state.cancelled.swap(true, Ordering::AcqRel) {
            let waker = self
                .state
                .waiting_waker
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take();
            if let Some(waker) = waker {
                waker.wake();
            }
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.state.cancelled.load(Ordering::Acquire)
    }

    /// Resolves once cancellation is requested. A batch scheduler uses this as
    /// its one outer cancellation wait, then broadcasts to active executors
    /// and observes their settlement before returning.
    pub(crate) async fn wait_for_cancellation(&self) {
        poll_fn(|context| {
            if self.is_cancelled() {
                return Poll::Ready(());
            }
            self.register(context.waker());
            if self.is_cancelled() {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        })
        .await
    }

    pub(crate) fn register(&self, waker: &Waker) {
        let mut waiting_waker = self
            .state
            .waiting_waker
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *waiting_waker = Some(waker.clone());
    }
}

impl Default for AgentCancellation {
    fn default() -> Self {
        Self::new()
    }
}
