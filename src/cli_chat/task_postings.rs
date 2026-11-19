//! Packed, disposable postings. Queries combine the published file with its
//! bounded pending tail; the worker never owns a Session or Project writer.

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;

const BLOCK_BYTES: usize = 65_536 * 8;

struct Segment {
    offset: u64,
    first: u64,
    count: u64,
}

struct Pending {
    offset: u64,
    bytes: Arc<[u8]>,
}

pub(super) struct PackedPostings {
    entries: BTreeMap<(String, [u8; 32]), Vec<Segment>>,
    reader: RefCell<File>,
    sender: Option<mpsc::SyncSender<Arc<[u8]>>>,
    worker: Option<JoinHandle<()>>,
    published: Arc<Mutex<Result<u64, String>>>,
    pending: RefCell<VecDeque<Pending>>,
    staged: Vec<u8>,
    queued: u64,
}

impl PackedPostings {
    pub(super) fn new(path: &Path) -> io::Result<Self> {
        let mut writer = OpenOptions::new().write(true).create_new(true).open(path)?;
        let reader = File::open(path)?;
        let (sender, receiver) = mpsc::sync_channel::<Arc<[u8]>>(1);
        let published = Arc::new(Mutex::new(Ok(0)));
        let progress = published.clone();
        let worker = std::thread::Builder::new()
            .name("task-postings".into())
            .spawn(move || {
                let mut end = 0;
                for bytes in receiver {
                    let result = writer
                        .write_all(&bytes)
                        .map(|()| {
                            end += bytes.len() as u64;
                            end
                        })
                        .map_err(|error| error.to_string());
                    let failed = result.is_err();
                    *progress.lock().expect("posting publication lock") = result;
                    if failed {
                        break;
                    }
                }
            })?;
        Ok(Self {
            entries: BTreeMap::new(),
            reader: RefCell::new(reader),
            sender: Some(sender),
            worker: Some(worker),
            published,
            pending: RefCell::new(VecDeque::new()),
            staged: Vec::with_capacity(BLOCK_BYTES),
            queued: 0,
        })
    }

    pub(super) fn published_bytes(&self) -> io::Result<u64> {
        self.published
            .lock()
            .map_err(|_| io::Error::other("posting publication lock poisoned"))?
            .clone()
            .map_err(io::Error::other)
    }

    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(super) fn pending_bytes(&self, published: u64) -> usize {
        (self.queued - published) as usize + self.staged.len()
    }

    pub(super) fn clear(&mut self, namespace: &str) {
        self.entries.retain(|(name, _), _| name != namespace);
    }

    pub(super) fn count(&self, namespace: &str, key: blake3::Hash) -> u64 {
        self.entries
            .get(&(namespace.to_owned(), *key.as_bytes()))
            .and_then(|segments| segments.last())
            .map_or(0, |last| last.first + last.count)
    }

    pub(super) fn append(
        &mut self,
        namespace: &str,
        key: blake3::Hash,
        values: &[usize],
    ) -> io::Result<()> {
        self.published_bytes()?;
        for chunk in values.chunks(BLOCK_BYTES / 8) {
            if self.staged.len() + chunk.len() * 8 > BLOCK_BYTES {
                self.flush()?;
            }
            let first = self.count(namespace, key);
            let offset = self.queued + self.staged.len() as u64;
            for value in chunk {
                self.staged
                    .extend_from_slice(&(*value as u64).to_le_bytes());
            }
            self.entries
                .entry((namespace.to_owned(), *key.as_bytes()))
                .or_default()
                .push(Segment {
                    offset,
                    first,
                    count: chunk.len() as u64,
                });
            if self.staged.len() == BLOCK_BYTES {
                self.flush()?;
            }
        }
        Ok(())
    }

    pub(super) fn flush(&mut self) -> io::Result<()> {
        self.published_bytes()?;
        if self.staged.is_empty() {
            return Ok(());
        }
        let bytes: Arc<[u8]> = std::mem::take(&mut self.staged).into();
        self.sender
            .as_ref()
            .expect("active posting writer")
            .send(bytes.clone())
            .map_err(|_| {
                self.published_bytes()
                    .err()
                    .unwrap_or_else(|| io::Error::other("posting writer stopped"))
            })?;
        self.pending.borrow_mut().push_back(Pending {
            offset: self.queued,
            bytes: bytes.clone(),
        });
        self.queued += bytes.len() as u64;
        self.discard_published()?;
        Ok(())
    }

    fn discard_published(&self) -> io::Result<u64> {
        let published = self.published_bytes()?;
        let mut pending = self.pending.borrow_mut();
        while pending
            .front()
            .is_some_and(|p| p.offset + p.bytes.len() as u64 <= published)
        {
            pending.pop_front();
        }
        Ok(published)
    }

    pub(super) fn ordinal(
        &self,
        namespace: &str,
        key: blake3::Hash,
        position: u64,
    ) -> io::Result<usize> {
        let segments = &self.entries[&(namespace.to_owned(), *key.as_bytes())];
        let segment = &segments[segments.partition_point(|s| s.first <= position) - 1];
        let offset = segment.offset + (position - segment.first) * 8;
        let published = self.discard_published()?;
        let mut bytes = [0; 8];
        if offset < published {
            let mut file = self.reader.borrow_mut();
            file.seek(SeekFrom::Start(offset))?;
            file.read_exact(&mut bytes)?;
        } else if offset >= self.queued {
            let start = (offset - self.queued) as usize;
            bytes.copy_from_slice(&self.staged[start..start + 8]);
        } else {
            let pending = self.pending.borrow();
            let block = pending
                .iter()
                .find(|p| offset >= p.offset && offset < p.offset + p.bytes.len() as u64)
                .expect("unpublished postings retain their pending bytes");
            let start = (offset - block.offset) as usize;
            bytes.copy_from_slice(&block.bytes[start..start + 8]);
        }
        usize::try_from(u64::from_le_bytes(bytes)).map_err(io::Error::other)
    }

    pub(super) fn close(&mut self) -> io::Result<()> {
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| io::Error::other("posting writer panicked"))?;
        }
        self.published_bytes().map(|_| ())
    }
}

impl Drop for PackedPostings {
    fn drop(&mut self) {
        if self.worker.is_some()
            && let Err(error) = self.close()
        {
            eprintln!("Task posting maintenance failed: {error}");
        }
    }
}
