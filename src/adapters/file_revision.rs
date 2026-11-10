//! Process-local reuse of already verified local files. This is freshness,
//! not a durable proof against coordinated filesystem/history rewriting.

use std::fs::Metadata;
use std::time::SystemTime;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FileRevision {
    length: u64,
    modified: SystemTime,
    #[cfg(unix)]
    identity: (u64, u64, i64, i64),
}

impl FileRevision {
    pub(crate) fn of(metadata: &Metadata) -> std::io::Result<Self> {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Ok(Self {
            length: metadata.len(),
            modified: metadata.modified()?,
            #[cfg(unix)]
            identity: (
                metadata.dev(),
                metadata.ino(),
                metadata.ctime(),
                metadata.ctime_nsec(),
            ),
        })
    }
}
