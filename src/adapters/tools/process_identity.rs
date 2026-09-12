//! Opaque, runtime-local namespaces for the two live process tables.

use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_INSTANCE: AtomicU64 = AtomicU64::new(0);

pub(super) struct ProcessHandleSequence {
    namespace: String,
    next: AtomicU64,
}

impl ProcessHandleSequence {
    pub(super) fn new() -> Self {
        // RandomState supplies fresh standard-library random seeds without
        // another RNG dependency. These are identifiers, not security tokens.
        let seed = (std::process::id(), next_checked(&NEXT_INSTANCE));
        let first = RandomState::new().hash_one(seed);
        let second = RandomState::new().hash_one(seed);
        Self {
            namespace: format!("{}-{first:016x}{second:016x}-{}", seed.0, seed.1),
            next: AtomicU64::new(0),
        }
    }

    pub(super) fn next(&self, prefix: &str) -> String {
        format!(
            "{prefix}:v2:{}:{}",
            self.namespace,
            next_checked(&self.next)
        )
    }
}

fn next_checked(counter: &AtomicU64) -> u64 {
    counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            value.checked_add(1)
        })
        .expect("process handle identity space exhausted; refusing to reuse a handle")
        + 1
}
