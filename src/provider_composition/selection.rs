//! Comment-preserving, isolated model-selection configuration edits.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use thiserror::Error;
use toml_edit::{DocumentMut, Item, TableLike, Value, value};

/// Catalog metadata that an operator has explicitly chosen to import with a
/// model selection. It has one production consumer: the isolated writer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CatalogCapabilityImport {
    pub(crate) context_window: Option<u32>,
    pub(crate) input_modalities: Option<Vec<String>>,
    pub(crate) supports_reasoning_summary_parameter: Option<bool>,
}

#[derive(Debug, Error)]
pub(crate) enum ModelSelectionError {
    #[error("model selection configuration is malformed")]
    MalformedConfiguration,
    #[error("model selection references an unknown Provider profile")]
    UnknownProvider,
    #[error("model selection Provider configuration is malformed")]
    MalformedProvider,
    #[error(
        "model selection requires an explicitly configured API-key or Codex subscription Provider"
    )]
    UnsupportedProvider,
    #[error("model selection capability mapping is malformed")]
    MalformedCapabilityMapping,
    #[error("model selection generation cannot be advanced")]
    GenerationOverflow,
    #[error("model selection requires a regular configuration file")]
    UnsafeConfigurationPath,
    #[error("another Echidna model selection is already saving this configuration")]
    PublicationBusy,
    #[error("model selection configuration changed while preparing the update")]
    StaleConfiguration,
    #[error("model selection candidate failed existing configuration validation")]
    InvalidCandidate,
    #[error("model selection could not publish the configuration update")]
    PublicationFailed,
}

/// Produce an isolated selection edit without touching the original bytes.
/// The caller validates the returned candidate through the existing Config
/// owner before the publication function atomically replaces the source.
pub(super) fn edit_model_selection(
    source: &str,
    provider_id: &str,
    model: &str,
    reasoning_effort: Option<&str>,
    capabilities: Option<&CatalogCapabilityImport>,
) -> Result<String, ModelSelectionError> {
    let mut document = source
        .parse::<DocumentMut>()
        .map_err(|_| ModelSelectionError::MalformedConfiguration)?;
    let (mapping_name, provider_generation) = {
        let providers = table_like_mut(document.get_mut("providers"))?;
        let provider = providers
            .get_mut(provider_id)
            .and_then(Item::as_table_like_mut)
            .ok_or(ModelSelectionError::UnknownProvider)?;
        if provider
            .get("codex_subscription")
            .and_then(Item::as_table_like)
            .is_none()
            && provider.get("api_key").and_then(Item::as_str).is_none()
            && provider
                .get("credential_ref")
                .and_then(Item::as_str)
                .is_none()
        {
            return Err(ModelSelectionError::UnsupportedProvider);
        }
        (
            required_string(provider, "capability_mapping")?.to_owned(),
            positive_integer(provider, "generation")?,
        )
    };

    let mapping_item = {
        let mappings = table_like_mut(document.get_mut("capability_mappings"))?;
        mappings
            .get(&mapping_name)
            .cloned()
            .ok_or(ModelSelectionError::MalformedCapabilityMapping)?
    };
    let next_mapping_version = mapping_item
        .as_table_like()
        .ok_or(ModelSelectionError::MalformedCapabilityMapping)
        .and_then(|mapping| positive_integer(mapping, "version"))?
        .checked_add(1)
        .ok_or(ModelSelectionError::GenerationOverflow)?;

    let (new_mapping_name, next_provider_generation) = {
        let mappings = table_like_mut(document.get_mut("capability_mappings"))?;
        let mut generation = provider_generation
            .checked_add(1)
            .ok_or(ModelSelectionError::GenerationOverflow)?;
        loop {
            let candidate = format!("{provider_id}-selection-{generation}");
            if !mappings.contains_key(&candidate) {
                break (candidate, generation);
            }
            generation = generation
                .checked_add(1)
                .ok_or(ModelSelectionError::GenerationOverflow)?;
        }
    };

    {
        let mappings = table_like_mut(document.get_mut("capability_mappings"))?;
        mappings.insert(&new_mapping_name, mapping_item);
        let mapping = mappings
            .get_mut(&new_mapping_name)
            .and_then(Item::as_table_like_mut)
            .ok_or(ModelSelectionError::MalformedCapabilityMapping)?;
        replace_value(mapping, "version", next_mapping_version);
        match reasoning_effort {
            Some(effort) => replace_value(mapping, "reasoning_effort", effort),
            None => {
                mapping.remove("reasoning_effort");
            }
        }
        if let Some(capabilities) = capabilities {
            if let Some(context_window) = capabilities.context_window {
                replace_value(mapping, "context_window", i64::from(context_window));
            }
            if let Some(modalities) = &capabilities.input_modalities {
                replace_value(
                    mapping,
                    "text_input",
                    modalities.iter().any(|value| value == "text"),
                );
                replace_value(
                    mapping,
                    "image_input",
                    modalities.iter().any(|value| value == "image"),
                );
            }
        }
    }

    {
        let providers = table_like_mut(document.get_mut("providers"))?;
        let provider = providers
            .get_mut(provider_id)
            .and_then(Item::as_table_like_mut)
            .ok_or(ModelSelectionError::UnknownProvider)?;
        replace_value(provider, "model", model);
        replace_value(provider, "capability_mapping", new_mapping_name);
        replace_value(provider, "generation", next_provider_generation);
        if let Some(supports_summary) =
            capabilities.and_then(|capabilities| capabilities.supports_reasoning_summary_parameter)
        {
            let subscription = provider
                .get_mut("codex_subscription")
                .and_then(Item::as_table_like_mut)
                .ok_or(ModelSelectionError::UnsupportedProvider)?;
            replace_value(
                subscription,
                "supports_reasoning_summary_parameter",
                supports_summary,
            );
        }
    }

    Ok(document.to_string())
}

/// Validate and publish a candidate from one exact source snapshot. A stale or
/// unsafe source is never replaced, and a publication error leaves it intact.
pub(super) fn publish_model_selection<F>(
    path: &Path,
    provider_id: &str,
    model: &str,
    reasoning_effort: Option<&str>,
    capabilities: Option<&CatalogCapabilityImport>,
    validate_candidate: F,
) -> Result<(), ModelSelectionError>
where
    F: FnOnce(&Path) -> Result<(), ()>,
{
    let path =
        std::path::absolute(path).map_err(|_| ModelSelectionError::UnsafeConfigurationPath)?;
    let _lock_file = acquire_selection_lock(&path)?;
    let source = SourceSnapshot::read(&path)?;
    let candidate = edit_model_selection(
        &source.text,
        provider_id,
        model,
        reasoning_effort,
        capabilities,
    )?;
    let (temporary, mut file) = create_temporary(&path, &source.permissions)?;
    let publication = (|| {
        file.write_all(candidate.as_bytes())
            .and_then(|_| file.sync_all())
            .map_err(|_| ModelSelectionError::PublicationFailed)?;
        validate_candidate(&temporary).map_err(|_| ModelSelectionError::InvalidCandidate)?;
        if SourceSnapshot::read(&path)? != source {
            return Err(ModelSelectionError::StaleConfiguration);
        }
        fs::rename(&temporary, &path).map_err(|_| ModelSelectionError::PublicationFailed)?;
        sync_parent_directory(&path)
    })();
    if publication.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    publication
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SourceSnapshot {
    text: String,
    length: u64,
    modified: SystemTime,
    permissions: fs::Permissions,
}

impl SourceSnapshot {
    fn read(path: &Path) -> Result<Self, ModelSelectionError> {
        let metadata =
            fs::symlink_metadata(path).map_err(|_| ModelSelectionError::UnsafeConfigurationPath)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(ModelSelectionError::UnsafeConfigurationPath);
        }
        let text =
            fs::read_to_string(path).map_err(|_| ModelSelectionError::UnsafeConfigurationPath)?;
        let modified = metadata
            .modified()
            .map_err(|_| ModelSelectionError::UnsafeConfigurationPath)?;
        if text.len() as u64 != metadata.len() {
            return Err(ModelSelectionError::StaleConfiguration);
        }
        Ok(Self {
            text,
            length: metadata.len(),
            modified,
            permissions: metadata.permissions(),
        })
    }
}

fn table_like_mut(item: Option<&mut Item>) -> Result<&mut dyn TableLike, ModelSelectionError> {
    item.and_then(Item::as_table_like_mut)
        .ok_or(ModelSelectionError::MalformedConfiguration)
}

fn required_string<'a>(
    table: &'a dyn TableLike,
    key: &str,
) -> Result<&'a str, ModelSelectionError> {
    table
        .get(key)
        .and_then(Item::as_str)
        .filter(|value| !value.is_empty())
        .ok_or(ModelSelectionError::MalformedProvider)
}

fn positive_integer(table: &dyn TableLike, key: &str) -> Result<i64, ModelSelectionError> {
    table
        .get(key)
        .and_then(Item::as_integer)
        .filter(|value| *value > 0)
        .ok_or(ModelSelectionError::MalformedCapabilityMapping)
}

fn replace_value<T>(table: &mut dyn TableLike, key: &str, new_value: T)
where
    T: Into<Value>,
{
    let decoration = table
        .get(key)
        .and_then(Item::as_value)
        .map(|existing| existing.decor().clone());
    let mut replacement = value(new_value);
    if let Some(decoration) = decoration
        && let Some(replacement) = replacement.as_value_mut()
    {
        *replacement.decor_mut() = decoration;
    }
    table.insert(key, replacement);
}

fn temporary_path(path: &Path) -> Result<PathBuf, ModelSelectionError> {
    let parent = path
        .parent()
        .ok_or(ModelSelectionError::UnsafeConfigurationPath)?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or(ModelSelectionError::UnsafeConfigurationPath)?;
    Ok(parent.join(format!(".{name}.{}.tmp", std::process::id())))
}

fn selection_lock_path(path: &Path) -> Result<PathBuf, ModelSelectionError> {
    let parent = path
        .parent()
        .ok_or(ModelSelectionError::UnsafeConfigurationPath)?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or(ModelSelectionError::UnsafeConfigurationPath)?;
    Ok(parent.join(format!(".{name}.selection.lock")))
}

fn acquire_selection_lock(path: &Path) -> Result<File, ModelSelectionError> {
    let lock_path = selection_lock_path(path)?;
    match fs::symlink_metadata(&lock_path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err(ModelSelectionError::UnsafeConfigurationPath);
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(ModelSelectionError::PublicationFailed),
    }
    let lock_file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|_| ModelSelectionError::PublicationFailed)?;
    match lock_file.try_lock() {
        Ok(()) => Ok(lock_file),
        Err(std::fs::TryLockError::WouldBlock) => Err(ModelSelectionError::PublicationBusy),
        Err(std::fs::TryLockError::Error(_)) => Err(ModelSelectionError::PublicationFailed),
    }
}

fn create_temporary(
    path: &Path,
    permissions: &fs::Permissions,
) -> Result<(PathBuf, File), ModelSelectionError> {
    let temporary = temporary_path(path)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    set_temporary_mode(&mut options, permissions);
    let file = options
        .open(&temporary)
        .map_err(|_| ModelSelectionError::PublicationFailed)?;
    if fs::set_permissions(&temporary, permissions.clone()).is_err() {
        let _ = fs::remove_file(&temporary);
        return Err(ModelSelectionError::PublicationFailed);
    }
    Ok((temporary, file))
}

#[cfg(unix)]
fn set_temporary_mode(options: &mut OpenOptions, permissions: &fs::Permissions) {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    options.mode(permissions.mode());
}

#[cfg(not(unix))]
fn set_temporary_mode(_: &mut OpenOptions, _: &fs::Permissions) {}

#[cfg(unix)]
fn sync_parent_directory(path: &Path) -> Result<(), ModelSelectionError> {
    File::open(
        path.parent()
            .ok_or(ModelSelectionError::UnsafeConfigurationPath)?,
    )
    .and_then(|directory| directory.sync_all())
    .map_err(|_| ModelSelectionError::PublicationFailed)
}

#[cfg(not(unix))]
fn sync_parent_directory(_: &Path) -> Result<(), ModelSelectionError> {
    Ok(())
}
