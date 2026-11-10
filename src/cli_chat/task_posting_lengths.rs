//! Ordinary process-local counts for disposable posting files.
//! Counts detect missing/incomplete writes; these caches are not a tamper boundary.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

#[derive(Default)]
pub(super) struct PostingLengths {
    counts: BTreeMap<(String, [u8; 32]), u64>,
}

impl PostingLengths {
    pub(super) fn len(&self) -> usize {
        self.counts.len()
    }

    pub(super) fn get(&self, name: &str, key: blake3::Hash) -> Option<u64> {
        self.counts
            .get(&(name.to_owned(), *key.as_bytes()))
            .copied()
    }

    pub(super) fn set(&mut self, name: &str, key: blake3::Hash, count: u64) {
        self.counts
            .insert((name.to_owned(), *key.as_bytes()), count);
    }

    pub(super) fn clear(&mut self, name: &str) {
        self.counts.retain(|(namespace, _), _| namespace != name);
    }

    pub(super) fn verify_length(
        &self,
        name: &str,
        key: blake3::Hash,
        path: &Path,
        entry_bytes: u64,
    ) -> io::Result<Option<u64>> {
        let Some(count) = self.get(name, key) else {
            return Ok(None);
        };
        let expected = count.checked_mul(entry_bytes).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "task posting length overflow")
        })?;
        if std::fs::metadata(path)?.len() != expected {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "task posting length changed",
            ));
        }
        Ok(Some(count))
    }
}
