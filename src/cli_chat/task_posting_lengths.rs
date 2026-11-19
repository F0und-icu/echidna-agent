//! Ordinary process-local counts for disposable posting files.
//! Counts detect missing/incomplete writes; these caches are not a tamper boundary.

use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct PostingLengths {
    counts: BTreeMap<(String, [u8; 32]), u64>,
}

impl PostingLengths {
    pub(super) fn get(&self, name: &str, key: blake3::Hash) -> Option<u64> {
        self.counts
            .get(&(name.to_owned(), *key.as_bytes()))
            .copied()
    }

    pub(super) fn set(&mut self, name: &str, key: blake3::Hash, count: u64) {
        self.counts
            .insert((name.to_owned(), *key.as_bytes()), count);
    }
}
