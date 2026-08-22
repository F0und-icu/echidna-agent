//! Adapter-owned resolution of concrete Tool arguments into typed resource keys.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

use crate::domain::tools::ResourceKey;

use super::filesystem::resolve_path;

/// Resolves only the resource identities that the built-in tools can prove.
/// Existing paths use filesystem canonicalization so aliases share a lock;
/// missing output paths canonicalize their longest existing parent before
/// appending the deterministic lexical suffix.
pub(super) struct ResourceKeyResolver<'a> {
    project_root: &'a Path,
}

impl<'a> ResourceKeyResolver<'a> {
    pub(super) fn new(project_root: &'a Path) -> Self {
        Self { project_root }
    }

    pub(super) fn file(&self, path: &str) -> ResourceKey {
        ResourceKey::file(self.normalized_path(path).display().to_string())
    }

    pub(super) fn output(&self, path: &str) -> ResourceKey {
        ResourceKey::output(self.normalized_path(path).display().to_string())
    }

    pub(super) fn device(&self, serial: &str) -> ResourceKey {
        ResourceKey::device(serial)
    }

    pub(super) fn pty(&self, session: &str) -> ResourceKey {
        ResourceKey::pty(session)
    }

    fn normalized_path(&self, raw_path: &str) -> PathBuf {
        let root = std::fs::canonicalize(self.project_root)
            .unwrap_or_else(|_| absolute_lexical(self.project_root));
        let resolved = resolve_path(&root, raw_path);
        canonicalize_existing_parent(&resolved)
    }
}

fn canonicalize_existing_parent(path: &Path) -> PathBuf {
    let mut suffix = Vec::<OsString>::new();
    let mut current = path;
    loop {
        if let Ok(existing) = std::fs::canonicalize(current) {
            let mut normalized = existing;
            for component in suffix.iter().rev() {
                normalized.push(component);
            }
            return lexical_normalize(&normalized);
        }
        let Some(name) = current.file_name() else {
            return lexical_normalize(path);
        };
        suffix.push(name.to_os_string());
        let Some(parent) = current.parent() else {
            return lexical_normalize(path);
        };
        current = parent;
    }
}

fn absolute_lexical(path: &Path) -> PathBuf {
    if path.is_absolute() {
        lexical_normalize(path)
    } else {
        std::env::current_dir()
            .map(|cwd| lexical_normalize(&cwd.join(path)))
            .unwrap_or_else(|_| lexical_normalize(path))
    }
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::RootDir | Component::Prefix(_) | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    normalized
}
