use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::Engine;
use ring::rand::{SecureRandom, SystemRandom};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::time::{Instant, sleep};

use super::types::{SubscriptionAuthError, account_path};

const STORE_MODE: u32 = 0o700;
const FILE_MODE: u32 = 0o600;
const LOCK_WAIT: Duration = Duration::from_secs(40);
const LOCK_RETRY: Duration = Duration::from_millis(25);

#[derive(Clone, Debug)]
pub(super) struct AuthStore {
    root: PathBuf,
    account: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub(super) struct HostRecord {
    pub(super) version: u8,
    pub(super) host_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct RegistrationRecord {
    pub(super) version: u8,
    pub(super) account: String,
    pub(super) client_id: String,
    pub(super) subject: Option<String>,
    pub(super) issuer: Option<String>,
}

#[derive(Clone, Deserialize, Serialize)]
pub(super) struct TokenRecord {
    pub(super) version: u8,
    pub(super) client_id: String,
    pub(super) subject: String,
    pub(super) issuer: String,
    pub(super) access_token: String,
    pub(super) refresh_token: String,
    pub(super) expires_at_unix: u64,
    pub(super) earliest_refresh_at_unix: u64,
    pub(super) scopes: Vec<String>,
}

#[derive(Clone, Deserialize, Serialize)]
pub(super) struct ConnectionRecord {
    pub(super) version: u8,
    pub(super) registration: RegistrationRecord,
    pub(super) tokens: Option<TokenRecord>,
}

pub(super) struct StoreLock(File);

impl Drop for StoreLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

impl AuthStore {
    pub(super) fn new(root: PathBuf, account: String) -> Self {
        Self { root, account }
    }

    pub(super) fn account_label(&self) -> String {
        self.account.clone()
    }

    pub(super) fn read_host(&self) -> Result<Option<HostRecord>, SubscriptionAuthError> {
        if !self.check_existing_directory()? {
            return Ok(None);
        }
        self.read_json(&self.root.join("host"))
    }

    pub(super) fn read_connection(
        &self,
    ) -> Result<Option<ConnectionRecord>, SubscriptionAuthError> {
        if !self.check_existing_directory()? {
            return Ok(None);
        }
        self.read_json(&self.path("connection"))
    }

    pub(super) fn load_or_create_host(&self) -> Result<HostRecord, SubscriptionAuthError> {
        self.ensure_directory()?;
        if let Some(host) = self.read_host()? {
            if host.version == 1 && valid_host_id(&host.host_id) {
                return Ok(host);
            }
            return Err(SubscriptionAuthError::MalformedStorage);
        }

        let host = HostRecord {
            version: 1,
            host_id: format!("urn:uuid:{}", random_uuid()?),
        };
        self.write_json(&self.root.join("host"), &host)?;
        Ok(host)
    }

    pub(super) fn write_connection(
        &self,
        connection: &ConnectionRecord,
    ) -> Result<(), SubscriptionAuthError> {
        self.ensure_directory()?;
        self.write_json(&self.path("connection"), connection)
    }

    pub(super) fn clear_tokens(&self) -> Result<(), SubscriptionAuthError> {
        let Some(mut connection) = self.read_connection()? else {
            return Ok(());
        };
        connection.tokens = None;
        self.write_connection(&connection)
    }

    pub(super) async fn lock(&self) -> Result<StoreLock, SubscriptionAuthError> {
        self.ensure_directory()?;
        let path = self.root.join("lock");
        let deadline = Instant::now() + LOCK_WAIT;
        loop {
            let file = self.open_lock_file(&path)?;
            match file.try_lock() {
                Ok(()) => return Ok(StoreLock(file)),
                Err(std::fs::TryLockError::WouldBlock) => {
                    if Instant::now() >= deadline {
                        return Err(SubscriptionAuthError::StorageLockTimedOut);
                    }
                    sleep(LOCK_RETRY).await;
                }
                Err(_) => return Err(SubscriptionAuthError::StorageUnavailable),
            }
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        account_path(&self.root, &self.account, name)
    }

    fn ensure_directory(&self) -> Result<(), SubscriptionAuthError> {
        ensure_existing_parents_are_not_symlinks(
            self.root
                .parent()
                .ok_or(SubscriptionAuthError::InvalidStorePath)?,
        )?;
        match fs::symlink_metadata(&self.root) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(SubscriptionAuthError::SymlinkStorage);
                }
                if !metadata.is_dir() || !is_private(&metadata) {
                    return Err(SubscriptionAuthError::InsecureStorage);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                match fs::create_dir(&self.root) {
                    Ok(()) => set_mode(&self.root, STORE_MODE)?,
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                        return self.ensure_directory();
                    }
                    Err(_) => return Err(SubscriptionAuthError::StorageUnavailable),
                }
            }
            Err(_) => return Err(SubscriptionAuthError::StorageUnavailable),
        }
        Ok(())
    }

    /// A read-only lifecycle operation must reject an existing unsafe root,
    /// without creating it or taking the write lock.
    fn check_existing_directory(&self) -> Result<bool, SubscriptionAuthError> {
        ensure_existing_parents_are_not_symlinks(
            self.root
                .parent()
                .ok_or(SubscriptionAuthError::InvalidStorePath)?,
        )?;
        match fs::symlink_metadata(&self.root) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                Err(SubscriptionAuthError::SymlinkStorage)
            }
            Ok(metadata) if !metadata.is_dir() || !is_private(&metadata) => {
                Err(SubscriptionAuthError::InsecureStorage)
            }
            Ok(_) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(_) => Err(SubscriptionAuthError::StorageUnavailable),
        }
    }

    fn read_json<T: DeserializeOwned>(
        &self,
        path: &Path,
    ) -> Result<Option<T>, SubscriptionAuthError> {
        let bytes = match self.read_private_file(path)? {
            Some(bytes) => bytes,
            None => return Ok(None),
        };
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| SubscriptionAuthError::MalformedStorage)
    }

    fn write_json<T: Serialize>(
        &self,
        path: &Path,
        value: &T,
    ) -> Result<(), SubscriptionAuthError> {
        validate_private_path(path)?;
        let bytes =
            serde_json::to_vec(value).map_err(|_| SubscriptionAuthError::StorageUnavailable)?;
        let temporary = unique_temporary_path(path)?;
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        set_file_mode(&mut options, FILE_MODE);
        let mut file = options
            .open(&temporary)
            .map_err(|_| SubscriptionAuthError::StorageUnavailable)?;
        let write_result = (|| {
            file.write_all(&bytes)
                .map_err(|_| SubscriptionAuthError::StorageUnavailable)?;
            file.sync_all()
                .map_err(|_| SubscriptionAuthError::StorageUnavailable)?;
            fs::rename(&temporary, path).map_err(|_| SubscriptionAuthError::StorageUnavailable)?;
            sync_parent_directory(path)
        })();
        if write_result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        write_result
    }

    fn read_private_file(&self, path: &Path) -> Result<Option<Vec<u8>>, SubscriptionAuthError> {
        validate_private_path(path)?;
        match fs::read(path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err(SubscriptionAuthError::StorageUnavailable),
        }
    }

    fn open_lock_file(&self, path: &Path) -> Result<File, SubscriptionAuthError> {
        validate_private_path(path)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true);
        set_file_mode(&mut options, FILE_MODE);
        options
            .open(path)
            .map_err(|_| SubscriptionAuthError::StorageUnavailable)
    }
}

#[cfg(unix)]
fn sync_parent_directory(path: &Path) -> Result<(), SubscriptionAuthError> {
    File::open(
        path.parent()
            .ok_or(SubscriptionAuthError::InvalidStorePath)?,
    )
    .and_then(|directory| directory.sync_all())
    .map_err(|_| SubscriptionAuthError::StorageUnavailable)
}

#[cfg(not(unix))]
fn sync_parent_directory(_: &Path) -> Result<(), SubscriptionAuthError> {
    Err(SubscriptionAuthError::InsecureStorage)
}

fn ensure_existing_parents_are_not_symlinks(path: &Path) -> Result<(), SubscriptionAuthError> {
    let mut current = PathBuf::from(std::path::MAIN_SEPARATOR.to_string());
    for component in path.components() {
        let std::path::Component::Normal(component) = component else {
            continue;
        };
        current.push(component);
        let metadata = fs::symlink_metadata(&current)
            .map_err(|_| SubscriptionAuthError::StorageUnavailable)?;
        if metadata.file_type().is_symlink() {
            return Err(SubscriptionAuthError::SymlinkStorage);
        }
        if !metadata.is_dir() {
            return Err(SubscriptionAuthError::StorageUnavailable);
        }
    }
    Ok(())
}

fn validate_private_path(path: &Path) -> Result<(), SubscriptionAuthError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(SubscriptionAuthError::SymlinkStorage)
        }
        Ok(metadata) if !metadata.is_file() || !is_private(&metadata) => {
            Err(SubscriptionAuthError::InsecureStorage)
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(SubscriptionAuthError::StorageUnavailable),
    }
}

fn unique_temporary_path(path: &Path) -> Result<PathBuf, SubscriptionAuthError> {
    let random = random_bytes::<12>()?;
    let suffix = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(SubscriptionAuthError::InvalidStorePath)?;
    Ok(path.with_file_name(format!(".{name}.{suffix}.tmp")))
}

fn random_uuid() -> Result<String, SubscriptionAuthError> {
    let random = random_bytes::<16>()?;
    Ok(format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        random[0],
        random[1],
        random[2],
        random[3],
        random[4],
        random[5],
        random[6],
        random[7],
        random[8],
        random[9],
        random[10],
        random[11],
        random[12],
        random[13],
        random[14],
        random[15]
    ))
}

fn valid_host_id(value: &str) -> bool {
    value.len() == 45
        && value.starts_with("urn:uuid:")
        && value.strip_prefix("urn:uuid:").is_some_and(|suffix| {
            suffix.bytes().enumerate().all(|(index, byte)| {
                matches!(index, 8 | 13 | 18 | 23) && byte == b'-' || byte.is_ascii_hexdigit()
            })
        })
}

fn random_bytes<const N: usize>() -> Result<[u8; N], SubscriptionAuthError> {
    let mut bytes = [0; N];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| SubscriptionAuthError::StorageUnavailable)?;
    Ok(bytes)
}

#[cfg(unix)]
fn is_private(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;

    metadata.permissions().mode() & 0o077 == 0
}

#[cfg(not(unix))]
fn is_private(_: &fs::Metadata) -> bool {
    false
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<(), SubscriptionAuthError> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(|_| SubscriptionAuthError::StorageUnavailable)
}

#[cfg(not(unix))]
fn set_mode(_: &Path, _: u32) -> Result<(), SubscriptionAuthError> {
    Err(SubscriptionAuthError::InsecureStorage)
}

#[cfg(unix)]
fn set_file_mode(options: &mut OpenOptions, mode: u32) {
    use std::os::unix::fs::OpenOptionsExt;

    options.mode(mode);
}

#[cfg(not(unix))]
fn set_file_mode(_: &mut OpenOptions, _: u32) {}
