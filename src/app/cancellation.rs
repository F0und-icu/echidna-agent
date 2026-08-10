//! One cancellation authority shared by model Streaming and Tool execution.

use std::future::{Future, poll_fn};
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

pub(crate) async fn run_until_cancelled<Output>(
    future: impl Future<Output = Output>,
    cancellation: &AgentCancellation,
) -> Option<Output> {
    let mut future = Box::pin(future);
    poll_fn(|context| {
        if cancellation.is_cancelled() {
            return Poll::Ready(None);
        }
        match future.as_mut().poll(context) {
            Poll::Ready(output) => Poll::Ready(Some(output)),
            Poll::Pending => {
                cancellation.register(context.waker());
                if cancellation.is_cancelled() {
                    Poll::Ready(None)
                } else {
                    Poll::Pending
                }
            }
        }
    })
    .await
}
