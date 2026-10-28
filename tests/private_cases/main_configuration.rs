use std::sync::Arc;

use crate::adapters::credentials::{CredentialSource, CredentialSourceError};
use crate::domain::context::ContextBudgetPolicy;
use crate::domain::model::ReasoningEffort;
use crate::provider_composition::ProviderComposition;

struct NoCredentials;

impl CredentialSource for NoCredentials {
    fn read(&self, _: &str) -> Result<Option<String>, CredentialSourceError> {
        panic!("offline configuration must not resolve credentials")
    }
}

#[test]
fn main_examples_compose_offline_with_exact_default_budget_and_no_persona_settings() {
    for name in [
        "provider.toml",
        "provider-text-vision.toml",
        "provider-multimodal.toml",
        "provider-mcp.toml",
        "provider-chatgpt-subscription.toml",
        "provider-codex-subscription.toml",
    ] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("examples")
            .join(name);
        let text = std::fs::read_to_string(&path).unwrap();
        let value: toml::Value = toml::from_str(&text).unwrap();
        let runtime = value["runtime"].as_table().unwrap();
        assert_eq!(runtime.len(), 3, "{name}");
        let budget = runtime["context_budget"].as_table().unwrap();
        assert_eq!(budget.len(), 14, "{name}");
        assert!(!text.contains("agent_definition"), "{name}");
        assert!(!text.contains("diagnostic_"), "{name}");
        let composition = ProviderComposition::load(path, Arc::new(NoCredentials)).unwrap();
        assert_eq!(
            composition.runtime().context_budget,
            ContextBudgetPolicy::default(),
            "{name}"
        );
        assert_eq!(
            composition.runtime().auto_compact,
            name == "provider-codex-subscription.toml"
        );
        assert_eq!(composition.runtime().model_request_limit_per_turn, 32);
    }
}

#[test]
fn compaction_effort_changes_only_the_isolated_primary_compaction_profile() {
    for effort in [None, Some("high")] {
        let root = crate::support::TemporaryDirectory::new("main-compaction-effort");
        let mut config = crate::provider_config::fixture()
            .replacen(
                "[capability_mappings.text-v1]",
                "[capability_mappings.text-v1]\nreasoning_effort = \"xhigh\"",
                1,
            )
            .replacen(
                "[capability_mappings.vision-v1]",
                "[capability_mappings.vision-v1]\nreasoning_effort = \"medium\"",
                1,
            );
        if let Some(effort) = effort {
            config.push_str(&format!(
                "\n[runtime]\ncompaction_reasoning_effort = \"{effort}\"\n"
            ));
        }
        let path = root.path().join("config.toml");
        std::fs::write(&path, config).unwrap();
        let composition = ProviderComposition::load(path, Arc::new(NoCredentials)).unwrap();
        assert_eq!(
            composition
                .primary_provider()
                .capabilities()
                .reasoning_effort,
            Some(ReasoningEffort::XHigh)
        );
        assert_eq!(
            composition
                .vision_provider()
                .unwrap()
                .capabilities()
                .reasoning_effort,
            Some(ReasoningEffort::Medium)
        );
        let (provider, profile) = composition.compaction_provider();
        let expected = if effort.is_some() {
            ReasoningEffort::High
        } else {
            ReasoningEffort::XHigh
        };
        assert_eq!(
            provider.capabilities().reasoning_effort,
            Some(expected.clone())
        );
        assert_eq!(
            profile.primary().capabilities().reasoning_effort,
            Some(expected)
        );
        assert_eq!(
            profile.primary().model(),
            composition.agent_profile().primary().model()
        );
    }
}

#[test]
fn main_rejects_persona_and_unused_diagnostic_configuration() {
    for suffix in [
        "\n[runtime.context_budget]\ndiagnostic_bps = 1\n",
        "\n[runtime.context_budget]\ndiagnostic_task_bps = 1\n",
        "\n[runtime.context_budget]\ndiagnostic_handoff_bps = 1\n",
        "\n[agent_definition]\npath = \"persona.toml\"\n",
    ] {
        let root = crate::support::TemporaryDirectory::new("main-config-boundary");
        let path = root.path().join("config.toml");
        std::fs::write(
            &path,
            format!("{}{suffix}", crate::provider_config::fixture()),
        )
        .unwrap();
        assert!(ProviderComposition::load(path, Arc::new(NoCredentials)).is_err());
    }
}
