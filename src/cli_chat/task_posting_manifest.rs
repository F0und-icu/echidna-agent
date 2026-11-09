//! Bounded authenticated membership and length proofs for disposable postings.

use std::cell::{Cell, RefCell};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const NODE_BYTES: u64 = 83;

#[derive(Clone, Copy, Default)]
pub(super) struct ManifestWork {
    pub(super) read_nodes: usize,
    pub(super) read_bytes: usize,
    pub(super) written_nodes: usize,
    pub(super) written_bytes: usize,
}

#[derive(Clone, Copy)]
struct NodeRef {
    offset: u64,
    digest: blake3::Hash,
}

#[derive(Clone, Copy)]
enum Node {
    Leaf {
        key: [u8; 32],
        count: u64,
    },
    Branch {
        bit: u16,
        left: NodeRef,
        right: NodeRef,
    },
}

pub(super) struct PostingManifest {
    file: RefCell<File>,
    path: PathBuf,
    key: [u8; 32],
    roots: [Option<NodeRef>; 6],
    work: Cell<ManifestWork>,
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn namespace(name: &str) -> usize {
    match name {
        "history" => 0,
        "cards" => 1,
        "anchor-post" => 2,
        "card-ref" => 3,
        "card-data" => 4,
        "artifact-owner" => 5,
        _ => unreachable!("fixed task posting namespace"),
    }
}

fn bit(key: &[u8; 32], position: u16) -> bool {
    key[(position / 8) as usize] & (0x80 >> (position % 8)) != 0
}

fn differing_bit(left: &[u8; 32], right: &[u8; 32]) -> u16 {
    for (index, (left, right)) in left.iter().zip(right).enumerate() {
        let difference = left ^ right;
        if difference != 0 {
            return (index as u16) * 8 + difference.leading_zeros() as u16;
        }
    }
    256
}

fn write_ref(bytes: &mut [u8], reference: NodeRef) {
    bytes[..8].copy_from_slice(&reference.offset.to_le_bytes());
    bytes[8..40].copy_from_slice(reference.digest.as_bytes());
}

fn read_ref(bytes: &[u8]) -> NodeRef {
    NodeRef {
        offset: u64::from_le_bytes(bytes[..8].try_into().expect("fixed width")),
        digest: blake3::Hash::from_bytes(bytes[8..40].try_into().expect("fixed width")),
    }
}

impl PostingManifest {
    pub(super) fn new(path: PathBuf, key: [u8; 32]) -> io::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)?;
        Ok(Self {
            file: RefCell::new(file),
            path,
            key,
            roots: [None; 6],
            work: Cell::new(ManifestWork::default()),
        })
    }

    pub(super) fn reset_work(&self) {
        self.work.set(ManifestWork::default());
    }
    pub(super) fn work(&self) -> ManifestWork {
        self.work.get()
    }

    fn measure(&self, update: impl FnOnce(&mut ManifestWork)) {
        let mut work = self.work.get();
        update(&mut work);
        self.work.set(work);
    }

    fn check_file(&self) -> io::Result<()> {
        let path_length = fs::metadata(&self.path)?.len();
        let open_length = self.file.borrow().metadata()?.len();
        if path_length != open_length || !open_length.is_multiple_of(NODE_BYTES) {
            return Err(invalid("task posting proof length changed"));
        }
        Ok(())
    }

    fn digest(&self, space: usize, offset: u64, bytes: &[u8]) -> blake3::Hash {
        let mut hash = blake3::Hasher::new_keyed(&self.key);
        hash.update(b"task-posting-manifest-v2");
        hash.update(&[space as u8]);
        hash.update(&offset.to_le_bytes());
        hash.update(bytes);
        hash.finalize()
    }

    fn encode(node: Node) -> [u8; NODE_BYTES as usize] {
        let mut bytes = [0; NODE_BYTES as usize];
        match node {
            Node::Leaf { key, count } => {
                bytes[1..33].copy_from_slice(&key);
                bytes[33..41].copy_from_slice(&count.to_le_bytes());
            }
            Node::Branch { bit, left, right } => {
                bytes[0] = 1;
                bytes[1..3].copy_from_slice(&bit.to_le_bytes());
                write_ref(&mut bytes[3..43], left);
                write_ref(&mut bytes[43..83], right);
            }
        }
        bytes
    }

    fn read(&self, space: usize, reference: NodeRef) -> io::Result<Node> {
        if !reference.offset.is_multiple_of(NODE_BYTES) {
            return Err(invalid("task posting proof offset is invalid"));
        }
        let mut bytes = [0; NODE_BYTES as usize];
        {
            let mut file = self.file.borrow_mut();
            file.seek(SeekFrom::Start(reference.offset))?;
            file.read_exact(&mut bytes)?;
        }
        self.measure(|work| {
            work.read_nodes += 1;
            work.read_bytes += bytes.len();
        });
        if self.digest(space, reference.offset, &bytes) != reference.digest {
            return Err(invalid("task posting proof changed"));
        }
        match bytes[0] {
            0 if bytes[41..].iter().all(|byte| *byte == 0) => Ok(Node::Leaf {
                key: bytes[1..33].try_into().expect("fixed width"),
                count: u64::from_le_bytes(bytes[33..41].try_into().expect("fixed width")),
            }),
            1 => {
                let bit = u16::from_le_bytes(bytes[1..3].try_into().expect("fixed width"));
                if bit >= 256 {
                    return Err(invalid("task posting proof bit is invalid"));
                }
                Ok(Node::Branch {
                    bit,
                    left: read_ref(&bytes[3..43]),
                    right: read_ref(&bytes[43..83]),
                })
            }
            _ => Err(invalid("task posting proof encoding changed")),
        }
    }

    fn write(&self, space: usize, node: Node) -> io::Result<NodeRef> {
        let bytes = Self::encode(node);
        let offset;
        {
            let mut file = self.file.borrow_mut();
            offset = file.seek(SeekFrom::End(0))?;
            if !offset.is_multiple_of(NODE_BYTES) {
                return Err(invalid("task posting proof was truncated"));
            }
            file.write_all(&bytes)?;
        }
        self.measure(|work| {
            work.written_nodes += 1;
            work.written_bytes += bytes.len();
        });
        Ok(NodeRef {
            offset,
            digest: self.digest(space, offset, &bytes),
        })
    }

    fn leaf(&self, space: usize, root: NodeRef, key: &[u8; 32]) -> io::Result<([u8; 32], u64)> {
        let mut current = root;
        let mut previous = None;
        for _ in 0..=256 {
            match self.read(space, current)? {
                Node::Leaf { key, count } => return Ok((key, count)),
                Node::Branch {
                    bit: position,
                    left,
                    right,
                } => {
                    if previous.is_some_and(|previous| position <= previous) {
                        return Err(invalid("task posting proof order changed"));
                    }
                    previous = Some(position);
                    current = if bit(key, position) { right } else { left };
                }
            }
        }
        Err(invalid("task posting proof exceeds its depth"))
    }

    pub(super) fn get(&self, name: &str, key: blake3::Hash) -> io::Result<Option<u64>> {
        self.check_file()?;
        let space = namespace(name);
        let Some(root) = self.roots[space] else {
            return Ok(None);
        };
        let (found, count) = self.leaf(space, root, key.as_bytes())?;
        Ok((found == *key.as_bytes()).then_some(count))
    }

    /// A positive proof binds the complete posting length. A missing file or
    /// an aligned truncation therefore fails before any candidates are used.
    pub(super) fn verify_length(
        &self,
        name: &str,
        key: blake3::Hash,
        path: &Path,
        entry_bytes: u64,
    ) -> io::Result<Option<u64>> {
        let Some(count) = self.get(name, key)? else {
            return Ok(None);
        };
        let expected = count
            .checked_mul(entry_bytes)
            .ok_or_else(|| invalid("task posting length overflow"))?;
        if fs::metadata(path)?.len() != expected {
            return Err(invalid("task posting length changed"));
        }
        Ok(Some(count))
    }

    fn insert(
        &self,
        space: usize,
        current: NodeRef,
        key: &[u8; 32],
        split: u16,
        leaf: NodeRef,
    ) -> io::Result<NodeRef> {
        match self.read(space, current)? {
            Node::Branch {
                bit: position,
                left,
                right,
            } if position < split => {
                let (left, right) = if bit(key, position) {
                    (left, self.insert(space, right, key, split, leaf)?)
                } else {
                    (self.insert(space, left, key, split, leaf)?, right)
                };
                self.write(
                    space,
                    Node::Branch {
                        bit: position,
                        left,
                        right,
                    },
                )
            }
            Node::Leaf { key: existing, .. } if split == 256 && &existing == key => Ok(leaf),
            _ if split < 256 => {
                let (left, right) = if bit(key, split) {
                    (current, leaf)
                } else {
                    (leaf, current)
                };
                self.write(
                    space,
                    Node::Branch {
                        bit: split,
                        left,
                        right,
                    },
                )
            }
            _ => Err(invalid("task posting proof replacement failed")),
        }
    }

    pub(super) fn set(&mut self, name: &str, key: blake3::Hash, count: u64) -> io::Result<()> {
        self.check_file()?;
        let space = namespace(name);
        let key = *key.as_bytes();
        let split = match self.roots[space] {
            Some(root) => differing_bit(&self.leaf(space, root, &key)?.0, &key),
            None => 256,
        };
        let leaf = self.write(space, Node::Leaf { key, count })?;
        self.roots[space] = Some(match self.roots[space] {
            Some(root) => self.insert(space, root, &key, split, leaf)?,
            None => leaf,
        });
        Ok(())
    }

    pub(super) fn clear(&mut self, name: &str) -> io::Result<()> {
        self.check_file()?;
        self.roots[namespace(name)] = None;
        Ok(())
    }
}
