#[path = "support/provider_config.rs"]
mod provider_config;
mod support;

use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use echidna::adapters::credentials::{CredentialSource, CredentialSourceError};
use echidna::app::doctor::{AgentProfileDoctorStatus, inspect_agent_model_profile};
use echidna::provider_composition::ProviderComposition;
use support::TemporaryDirectory;

struct NoCredentialReads;

impl CredentialSource for NoCredentialReads {
    fn read(&self, _: &str) -> Result<Option<String>, CredentialSourceError> {
        panic!("offline subscription composition must not read credentials")
    }
}

fn config(store: &Path) -> String {
    provider_config::fixture()
        .replacen("http://127.0.0.1:1/v1", "https://api.openai.com/v1", 1)
        .replacen(
            "credential_ref = \"CYBER_AGENT_PRIMARY_API_KEY\"",
            &format!(
                "chatgpt_subscription = {{ account = \"personal\", auth_store = {:?} }}",
                store.to_str().unwrap()
            ),
            1,
        )
        .replacen("incremental_input = true", "incremental_input = false", 1)
}

fn load(path: &Path, encoded: &str) -> Result<ProviderComposition, String> {
    fs::write(path, encoded).unwrap();
    ProviderComposition::load(path, Arc::new(NoCredentialReads)).map_err(|error| error.to_string())
}

#[test]
fn subscription_composition_and_doctor_are_offline_and_preserve_explicit_capabilities() {
    let directory = TemporaryDirectory::new("subscription-composition");
    let store = directory
        .path()
        .canonicalize()
        .unwrap()
        .join("uncreated-auth");
    let path = directory.path().join("config.toml");
    let composition = load(&path, &config(&store)).unwrap();
    assert_eq!(
        inspect_agent_model_profile(composition.agent_profile()).status,
        AgentProfileDoctorStatus::Ready
    );
    assert!(!composition.primary_provider().capabilities().image_input);
    assert!(
        !composition
            .primary_provider()
            .capabilities()
            .incremental_input
    );
    assert_eq!(
        composition
            .primary_provider()
            .capabilities()
            .max_output_tokens,
        None
    );
    assert_ne!(
        composition.primary_request_adapter_version().as_str(),
        "responses-http-sse-v1"
    );
    assert!(!store.exists());

    let output = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .args(["--config", path.to_str().unwrap(), "doctor"])
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!store.exists(), "doctor must not create a credential store");
}

#[test]
fn subscription_config_rejects_ambiguous_credentials_and_incompatible_wire_requirements() {
    let directory = TemporaryDirectory::new("subscription-invalid-config");
    let store = directory
        .path()
        .canonicalize()
        .unwrap()
        .join("uncreated-auth");
    let path = directory.path().join("config.toml");
    let valid = config(&store);
    let cases = [
        (
            valid.replacen(
                "[providers.primary]",
                "[providers.primary]\napi_key = \"fake-only\"",
                1,
            ),
            "exactly one",
        ),
        (
            valid.replacen(
                "[providers.primary]",
                "[providers.primary]\ncredential_ref = \"CYBER_AGENT_PRIMARY_API_KEY\"",
                1,
            ),
            "exactly one",
        ),
        (
            valid.replacen("https://api.openai.com/v1", "https://example.invalid/v1", 1),
            "official public HTTPS",
        ),
        (
            valid.replacen("https://api.openai.com/v1", "http://api.openai.com/v1", 1),
            "official public HTTPS",
        ),
        (
            valid.replacen("incremental_input = false", "incremental_input = true", 1),
            "incremental_input=false",
        ),
        (
            valid.replacen(
                "[capability_mappings.text-v1]",
                "[capability_mappings.text-v1]\nmax_output_tokens = 1024",
                1,
            ),
            "no max_output_tokens",
        ),
        (
            valid.replacen("account = \"personal\"", "account = \"../other\"", 1),
            "account",
        ),
        (
            valid.replacen(
                &format!("{:?}", store.to_str().unwrap()),
                "\"relative/auth\"",
                1,
            ),
            "store path",
        ),
        (
            valid.replacen(
                "account = \"personal\"",
                "account = \"personal\", extra = true",
                1,
            ),
            "unknown field",
        ),
    ];
    for (encoded, expected) in cases {
        let error = load(&path, &encoded)
            .err()
            .expect("invalid config accepted");
        assert!(error.contains(expected), "expected {expected}, got {error}");
    }
    assert!(!store.exists());
}

#[test]
fn auth_cli_requires_explicit_subscription_provider_and_works_without_project() {
    let directory = TemporaryDirectory::new("subscription-auth-cli");
    // The store hardening rejects symlinked ancestors. macOS commonly exposes
    // its temporary root through `/var`, so bind this fixture to the canonical
    // directory that was just created instead of weakening that production
    // rule for a system compatibility alias.
    let store = fs::canonicalize(directory.path())
        .expect("fixture temporary directory exists")
        .join("uncreated-auth");
    let path = directory.path().join("config.toml");
    fs::write(&path, config(&store)).unwrap();
    for provider in ["missing", "vision"] {
        let output = Command::new(env!("CARGO_BIN_EXE_echidna"))
            .args([
                "--config",
                path.to_str().unwrap(),
                "auth",
                "status",
                "--provider",
                provider,
            ])
            .current_dir(directory.path())
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("subscription"));
    }
    let output = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .args([
            "--config",
            path.to_str().unwrap(),
            "auth",
            "status",
            "--provider",
            "primary",
        ])
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let status: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(status["state"], "disconnected");
    assert_eq!(status["account"], "personal");
    assert_eq!(status["verified_identity"], false);
    assert!(status.get("access_token").is_none());
    assert!(status.get("refresh_token").is_none());
    assert!(!store.exists(), "status must remain read-only");
}
