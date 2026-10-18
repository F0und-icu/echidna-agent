//! Permanent regression coverage for private, atomic configuration publication.

#[path = "../src/provider_composition/selection.rs"]
mod selection;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use echidna::adapters::credentials::{CredentialSource, CredentialSourceError};
use echidna::provider_composition::ProviderComposition;
use selection::{
    CatalogCapabilityImport, ModelSelectionError, edit_model_selection, publish_model_selection,
};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);

struct FixtureDirectory(PathBuf);

impl FixtureDirectory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "echidna-selection-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn config(&self) -> PathBuf {
        self.0.join("provider.toml")
    }
}

impl Drop for FixtureDirectory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

struct NoCredentials;

impl CredentialSource for NoCredentials {
    fn read(&self, _: &str) -> Result<Option<String>, CredentialSourceError> {
        panic!("configuration selection must not read credentials")
    }
}

fn validate(path: &Path) -> Result<(), ()> {
    ProviderComposition::load(path, Arc::new(NoCredentials))
        .map(|_| ())
        .map_err(|_| ())
}

fn source() -> String {
    r#"# Operator notes must survive.
schema_version = 1
active_agent_profile = "test"

[providers.codex]
binding = "http_sse"
wire_api = "responses"
base_url = "https://chatgpt.com/backend-api/codex"
model = "original-model" # Keep this model comment.
codex_subscription = { account = "fixture", auth_store = "/tmp/echidna-selection-auth-unused", catalog_client_version = "0.145.0" }
capability_mapping = "shared"
generation = 1

# Another Provider and its fake inline key are not selected.
[providers.other]
binding = "http_sse"
wire_api = "responses"
base_url = "https://example.invalid/v1"
model = "ordinary-model"
api_key = "fixture-inline-key-must-survive-byte-for-byte"
capability_mapping = "shared"
generation = 7

[capability_mappings.shared]
version = 4
text_input = true
image_input = false
streaming = true
tool_call = true
structured_output = false
incremental_input = false
usage_reporting = true
context_window = 32768 # Operator-declared, not inferred.
reasoning_effort = "high"

[agent_profiles.test]
primary = "codex"
vision_mode = "disabled"
generation = 1
"#
    .to_owned()
}

#[test]
fn selected_provider_gets_isolated_mapping_and_comments_and_other_provider_survive() {
    let before = source();
    let after =
        edit_model_selection(&before, "codex", "returned-model", Some("xhigh"), None).unwrap();
    assert!(after.starts_with("# Operator notes must survive.\n"));
    assert!(after.contains("# Keep this model comment."));
    // The new mapping is inserted between existing tables. Each untouched
    // table must retain its bytes; the entire suffix need not stay contiguous.
    let other_provider = before
        .split("# Another Provider")
        .nth(1)
        .unwrap()
        .split("[capability_mappings.shared]")
        .next()
        .unwrap();
    let shared_mapping = before
        .split("[capability_mappings.shared]")
        .nth(1)
        .unwrap()
        .split("[agent_profiles.test]")
        .next()
        .unwrap();
    let agent_profile = before.split("[agent_profiles.test]").nth(1).unwrap();
    assert!(after.contains(other_provider));
    assert!(after.contains(shared_mapping));
    assert!(after.contains(agent_profile));
    let parsed: toml::Value = toml::from_str(&after).unwrap();
    assert_eq!(
        parsed["providers"]["codex"]["model"].as_str(),
        Some("returned-model")
    );
    assert_eq!(
        parsed["providers"]["codex"]["generation"].as_integer(),
        Some(2)
    );
    assert_eq!(
        parsed["providers"]["codex"]["codex_subscription"]["catalog_client_version"].as_str(),
        Some("0.145.0")
    );
    let mapping = parsed["providers"]["codex"]["capability_mapping"]
        .as_str()
        .unwrap();
    assert_eq!(mapping, "codex-selection-2");
    assert_eq!(
        parsed["capability_mappings"][mapping]["version"].as_integer(),
        Some(5)
    );
    assert_eq!(
        parsed["capability_mappings"][mapping]["reasoning_effort"].as_str(),
        Some("xhigh")
    );
    assert_eq!(
        parsed["capability_mappings"][mapping]["context_window"].as_integer(),
        Some(32768)
    );
    assert_eq!(
        parsed["capability_mappings"]["shared"]["reasoning_effort"].as_str(),
        Some("high")
    );
    assert_eq!(
        parsed["providers"]["other"]["capability_mapping"].as_str(),
        Some("shared")
    );
}

#[test]
fn service_default_removes_effort_only_in_the_new_mapping_and_avoids_collisions() {
    let before = source() + "\n[capability_mappings.codex-selection-2]\nversion = 1\n";
    let after = edit_model_selection(&before, "codex", "returned-model", None, None).unwrap();
    let parsed: toml::Value = toml::from_str(&after).unwrap();
    assert_eq!(
        parsed["providers"]["codex"]["generation"].as_integer(),
        Some(3)
    );
    assert_eq!(
        parsed["providers"]["codex"]["capability_mapping"].as_str(),
        Some("codex-selection-3")
    );
    assert!(
        parsed["capability_mappings"]["codex-selection-3"]
            .get("reasoning_effort")
            .is_none()
    );
    assert_eq!(
        parsed["capability_mappings"]["shared"]["reasoning_effort"].as_str(),
        Some("high")
    );
    assert_eq!(
        parsed["capability_mappings"]["codex-selection-2"]["version"].as_integer(),
        Some(1)
    );
}

#[test]
fn explicit_capability_import_updates_only_advertised_selection_fields() {
    let before = source();
    let after = edit_model_selection(
        &before,
        "codex",
        "returned-model",
        Some("future"),
        Some(&CatalogCapabilityImport {
            context_window: Some(128_000),
            input_modalities: Some(vec!["image".to_owned(), "unknown".to_owned()]),
            supports_reasoning_summary_parameter: Some(false),
        }),
    )
    .unwrap();
    let parsed: toml::Value = toml::from_str(&after).unwrap();
    let mapping = parsed["providers"]["codex"]["capability_mapping"]
        .as_str()
        .unwrap();
    assert_eq!(
        parsed["capability_mappings"][mapping]["context_window"].as_integer(),
        Some(128_000)
    );
    assert_eq!(
        parsed["capability_mappings"][mapping]["text_input"].as_bool(),
        Some(false)
    );
    assert_eq!(
        parsed["capability_mappings"][mapping]["image_input"].as_bool(),
        Some(true)
    );
    assert_eq!(
        parsed["providers"]["codex"]["codex_subscription"]["supports_reasoning_summary_parameter"]
            .as_bool(),
        Some(false)
    );
    assert!(
        parsed["capability_mappings"][mapping]
            .get("max_context_window")
            .is_none()
    );
    assert_eq!(
        parsed["capability_mappings"]["shared"]["context_window"].as_integer(),
        Some(32_768)
    );
}

#[test]
fn explicit_capability_import_preserves_the_existing_window_when_catalog_metadata_is_unknown() {
    let before = source();
    let after = edit_model_selection(
        &before,
        "codex",
        "returned-model",
        Some("future"),
        Some(&CatalogCapabilityImport {
            context_window: None,
            input_modalities: None,
            supports_reasoning_summary_parameter: None,
        }),
    )
    .unwrap();
    let parsed: toml::Value = toml::from_str(&after).unwrap();
    let mapping = parsed["providers"]["codex"]["capability_mapping"]
        .as_str()
        .unwrap();
    assert_eq!(
        parsed["capability_mappings"][mapping]["context_window"].as_integer(),
        Some(32_768)
    );
}

#[test]
fn invalid_combined_capability_import_does_not_publish_any_source_bytes() {
    let directory = FixtureDirectory::new();
    let path = directory.config();
    let before = source().replace(
        "context_window = 32768",
        "context_window = 32768\nmax_output_tokens = 32767",
    );
    fs::write(&path, &before).unwrap();
    let result = publish_model_selection(
        &path,
        "codex",
        "returned-model",
        Some("future"),
        Some(&CatalogCapabilityImport {
            context_window: Some(1),
            input_modalities: None,
            supports_reasoning_summary_parameter: None,
        }),
        validate,
    );
    assert!(matches!(result, Err(ModelSelectionError::InvalidCandidate)));
    assert_eq!(fs::read_to_string(&path).unwrap(), before);
}

#[test]
fn publication_validates_through_existing_composition_and_preserves_permissions() {
    let directory = FixtureDirectory::new();
    let path = directory.config();
    fs::write(&path, source()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    }
    let before_mode = fs::metadata(&path).unwrap().permissions();
    publish_model_selection(
        &path,
        "codex",
        "returned-model",
        Some("xhigh"),
        None,
        validate,
    )
    .unwrap();
    let composition = ProviderComposition::load(&path, Arc::new(NoCredentials)).unwrap();
    assert_eq!(
        composition.agent_profile().primary().profile().generation,
        2
    );
    let saved: toml::Value = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        saved["providers"]["codex"]["codex_subscription"]["catalog_client_version"].as_str(),
        Some("0.145.0")
    );
    assert_eq!(fs::metadata(&path).unwrap().permissions(), before_mode);
    assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 2);
}

#[test]
fn relative_config_filename_publishes_successfully_without_changing_working_directory() {
    use std::io::Write;

    struct RelativeConfig(PathBuf);
    impl Drop for RelativeConfig {
        fn drop(&mut self) {
            fs::remove_file(&self.0).unwrap();
            let lock = self.0.with_file_name(format!(
                ".{}.selection.lock",
                self.0.file_name().unwrap().to_str().unwrap()
            ));
            fs::remove_file(lock).unwrap();
        }
    }

    let path = PathBuf::from(format!(
        ".echidna-selection-relative-{}-{}.toml",
        std::process::id(),
        NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .unwrap();
    let fixture = RelativeConfig(path);
    file.write_all(source().as_bytes()).unwrap();
    drop(file);
    publish_model_selection(
        &fixture.0,
        "codex",
        "returned-model",
        Some("xhigh"),
        None,
        validate,
    )
    .unwrap();
    assert!(
        fs::read_to_string(&fixture.0)
            .unwrap()
            .contains("model = \"returned-model\"")
    );
    validate(&fixture.0).unwrap();
}

#[test]
fn invalid_candidate_and_generation_overflow_leave_original_bytes_untouched() {
    let directory = FixtureDirectory::new();
    let path = directory.config();
    let before = source();
    fs::write(&path, &before).unwrap();
    assert!(matches!(
        publish_model_selection(
            &path,
            "codex",
            "returned-model",
            Some("future unknown"),
            None,
            validate
        ),
        Err(ModelSelectionError::InvalidCandidate)
    ));
    assert_eq!(fs::read_to_string(&path).unwrap(), before);
    assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 2);
    let overflow = before.replace("generation = 1", "generation = 9223372036854775807");
    fs::write(&path, &overflow).unwrap();
    assert!(matches!(
        publish_model_selection(&path, "codex", "returned-model", None, None, validate),
        Err(ModelSelectionError::GenerationOverflow)
    ));
    assert_eq!(fs::read_to_string(&path).unwrap(), overflow);
}

#[test]
fn intervening_edit_is_retained_instead_of_overwritten() {
    let directory = FixtureDirectory::new();
    let path = directory.config();
    fs::write(&path, source()).unwrap();
    let external = source() + "\n# Concurrent operator change.\n";
    let result =
        publish_model_selection(&path, "codex", "returned-model", None, None, |candidate| {
            validate(candidate)?;
            fs::write(&path, &external).unwrap();
            Ok(())
        });
    assert!(matches!(
        result,
        Err(ModelSelectionError::StaleConfiguration)
    ));
    assert_eq!(fs::read_to_string(&path).unwrap(), external);
    assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 2);
}

#[test]
fn existing_temporary_file_is_never_deleted_or_replaced() {
    let directory = FixtureDirectory::new();
    let path = directory.config();
    let before = source();
    fs::write(&path, &before).unwrap();
    let collision = directory
        .0
        .join(format!(".provider.toml.{}.tmp", std::process::id()));
    fs::write(&collision, "another operation's data").unwrap();
    assert!(matches!(
        publish_model_selection(&path, "codex", "returned-model", None, None, validate),
        Err(ModelSelectionError::PublicationFailed)
    ));
    assert_eq!(fs::read_to_string(&path).unwrap(), before);
    assert_eq!(
        fs::read_to_string(collision).unwrap(),
        "another operation's data"
    );
}

#[test]
fn cooperating_publishers_cannot_overlap_and_stable_lock_releases_after_commit() {
    let directory = FixtureDirectory::new();
    let path = directory.config();
    fs::write(&path, source()).unwrap();
    publish_model_selection(
        &path,
        "codex",
        "first-selection",
        Some("xhigh"),
        None,
        |candidate| {
            validate(candidate)?;
            let child = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "lock_child_observes_busy_before_candidate_validation",
                    "--nocapture",
                ])
                .env("ECHIDNA_SELECTION_LOCK_CHILD_PATH", &path)
                .output()
                .unwrap();
            assert!(
                child.status.success(),
                "{}",
                String::from_utf8_lossy(&child.stderr)
            );
            assert!(
                fs::read_to_string(&path)
                    .unwrap()
                    .contains("original-model")
            );
            Ok(())
        },
    )
    .unwrap();
    let lock = directory.0.join(".provider.toml.selection.lock");
    assert_eq!(fs::metadata(&lock).unwrap().len(), 0);
    #[cfg(unix)]
    let before_inode = {
        use std::os::unix::fs::MetadataExt;
        let metadata = fs::metadata(&lock).unwrap();
        metadata.ino()
    };
    publish_model_selection(&path, "codex", "later-selection", None, None, validate).unwrap();
    let parsed: toml::Value = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        parsed["providers"]["codex"]["generation"].as_integer(),
        Some(3)
    );
    assert_eq!(
        parsed["providers"]["codex"]["model"].as_str(),
        Some("later-selection")
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(fs::metadata(lock).unwrap().ino(), before_inode);
    }
}

#[test]
fn lock_child_observes_busy_before_candidate_validation() {
    let Some(path) = std::env::var_os("ECHIDNA_SELECTION_LOCK_CHILD_PATH") else {
        return;
    };
    let result = publish_model_selection(
        Path::new(&path),
        "codex",
        "overlapping-selection",
        None,
        None,
        |_| panic!("a competing process must fail before candidate validation"),
    );
    assert!(matches!(result, Err(ModelSelectionError::PublicationBusy)));
}

#[cfg(unix)]
#[test]
fn lock_symlink_is_rejected_and_does_not_change_either_file() {
    use std::os::unix::fs::symlink;

    let directory = FixtureDirectory::new();
    let path = directory.config();
    let before = source();
    fs::write(&path, &before).unwrap();
    let target = directory.0.join("unrelated-file");
    fs::write(&target, "unrelated data").unwrap();
    symlink(&target, directory.0.join(".provider.toml.selection.lock")).unwrap();
    assert!(matches!(
        publish_model_selection(&path, "codex", "returned-model", None, None, validate),
        Err(ModelSelectionError::UnsafeConfigurationPath)
    ));
    assert_eq!(fs::read_to_string(path).unwrap(), before);
    assert_eq!(fs::read_to_string(target).unwrap(), "unrelated data");
}

#[test]
fn failed_atomic_publication_retains_original_without_secret_diagnostics() {
    let directory = FixtureDirectory::new();
    let path = directory.config();
    let before = source();
    fs::write(&path, &before).unwrap();
    let result =
        publish_model_selection(&path, "codex", "returned-model", None, None, |candidate| {
            validate(candidate)?;
            fs::remove_file(candidate).unwrap();
            Ok(())
        });
    let error = result.unwrap_err();
    assert!(matches!(error, ModelSelectionError::PublicationFailed));
    assert!(!error.to_string().contains("fixture-inline-key"));
    assert_eq!(fs::read_to_string(&path).unwrap(), before);
}

#[cfg(unix)]
#[test]
fn final_symlink_is_rejected_without_touching_its_target() {
    use std::os::unix::fs::symlink;
    let directory = FixtureDirectory::new();
    let target = directory.0.join("target.toml");
    let path = directory.config();
    let before = source();
    fs::write(&target, &before).unwrap();
    symlink(&target, &path).unwrap();
    assert!(matches!(
        publish_model_selection(&path, "codex", "returned-model", None, None, validate),
        Err(ModelSelectionError::UnsafeConfigurationPath)
    ));
    assert_eq!(fs::read_to_string(target).unwrap(), before);
    assert!(fs::symlink_metadata(path).unwrap().file_type().is_symlink());
}
