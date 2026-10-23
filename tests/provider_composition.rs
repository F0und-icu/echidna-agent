#[path = "support/provider_config.rs"]
mod provider_config;
mod support;

use std::fs;
use std::sync::{Arc, Mutex};

use echidna::adapters::credentials::{CredentialSource, CredentialSourceError};
use echidna::app::doctor::{AgentProfileDoctorStatus, inspect_agent_model_profile};
use echidna::domain::model::{
    ContentItem, ModelCapabilityError, ModelInputIR, ModelRequestRequirements, ReasoningEffort,
};
use echidna::domain::profile::{VisionMode, VisionRoute};
use echidna::ports::model::{ModelProviderError, ModelRequest, ProviderResponseId};
use echidna::provider_composition::{ProviderComposition, ProviderCompositionError};

use support::TemporaryDirectory;

const INLINE_API_KEY_FIXTURE: &str = "fixture-inline-api-key";

#[derive(Default)]
struct RecordingCredentialSource {
    references: Mutex<Vec<String>>,
}

impl RecordingCredentialSource {
    fn references(&self) -> Vec<String> {
        self.references
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

impl CredentialSource for RecordingCredentialSource {
    fn read(&self, reference: &str) -> Result<Option<String>, CredentialSourceError> {
        self.references
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(reference.to_owned());
        Ok(Some("fixture-secret".to_owned()))
    }
}

fn valid_config() -> String {
    provider_config::fixture()
}

#[test]
fn output_limit_config_is_explicit_role_scoped_and_offline_validated() {
    for value in [0, 32_768, 32_769, 30_000, 28_672] {
        let credentials = Arc::new(RecordingCredentialSource::default());
        let config = valid_config().replacen(
            "[capability_mappings.text-v1]",
            &format!("[capability_mappings.text-v1]\nmax_output_tokens = {value}"),
            1,
        );
        assert!(matches!(
            load("invalid-output-budget", &config, credentials.clone()),
            Err(ProviderCompositionError::InvalidContextBudget { .. })
        ));
        assert!(credentials.references().is_empty());
    }
    let credentials = Arc::new(RecordingCredentialSource::default());
    let config = valid_config()
        .replacen(
            "[capability_mappings.text-v1]",
            "[capability_mappings.text-v1]\nmax_output_tokens = 8192",
            1,
        )
        .replacen(
            "[capability_mappings.vision-v1]",
            "[capability_mappings.vision-v1]\nmax_output_tokens = 2048",
            1,
        );
    let composed = load("configured-output-budget", &config, credentials.clone()).unwrap();
    assert_eq!(
        composed.primary_provider().capabilities().max_output_tokens,
        Some(8192)
    );
    assert_eq!(
        composed
            .vision_provider()
            .unwrap()
            .capabilities()
            .max_output_tokens,
        Some(2048)
    );
    let report = inspect_agent_model_profile(composed.agent_profile());
    assert_eq!(report.primary_max_output_tokens, Some(8192));
    assert_eq!(report.vision_max_output_tokens, Some(2048));
    let missing_limit =
        composed
            .primary_provider()
            .stream(ModelRequest::full(ModelInputIR::current(
                "fixture",
                vec![ContentItem::user_text("fixture")],
                ModelRequestRequirements::default(),
            )));
    assert!(matches!(
        missing_limit,
        Err(ModelProviderError::Capability(
            ModelCapabilityError::OutputTokenLimitRequired
        ))
    ));
    assert!(credentials.references().is_empty());
}

#[test]
fn reasoning_effort_config_is_role_scoped_audited_and_validated_before_credentials() {
    let credentials = Arc::new(RecordingCredentialSource::default());
    let config = valid_config()
        .replacen(
            "[capability_mappings.text-v1]",
            "[capability_mappings.text-v1]\nreasoning_effort = \"xhigh\"",
            1,
        )
        .replacen(
            "[capability_mappings.vision-v1]",
            "[capability_mappings.vision-v1]\nreasoning_effort = \"high\"",
            1,
        );
    let composition = load("configured-reasoning-effort", &config, credentials.clone()).unwrap();
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
        Some(ReasoningEffort::High)
    );
    let report = inspect_agent_model_profile(composition.agent_profile());
    assert_eq!(
        report.primary_reasoning_effort,
        Some(ReasoningEffort::XHigh)
    );
    assert_eq!(report.vision_reasoning_effort, Some(ReasoningEffort::High));

    let missing = composition
        .primary_provider()
        .stream(ModelRequest::full(ModelInputIR::current(
            "fixture",
            vec![ContentItem::user_text("fixture")],
            ModelRequestRequirements::default(),
        )));
    assert!(matches!(
        missing,
        Err(ModelProviderError::Capability(
            ModelCapabilityError::ReasoningEffortRequired
        ))
    ));
    let mismatched =
        composition
            .primary_provider()
            .stream(ModelRequest::full(ModelInputIR::current(
                "fixture",
                vec![ContentItem::user_text("fixture")],
                ModelRequestRequirements {
                    reasoning_effort: Some(ReasoningEffort::High),
                    ..ModelRequestRequirements::default()
                },
            )));
    assert!(matches!(
        mismatched,
        Err(ModelProviderError::Capability(
            ModelCapabilityError::ReasoningEffortUnsupported
        ))
    ));
    assert!(credentials.references().is_empty());
}

#[test]
fn empty_prompt_path_is_rejected_before_relative_path_resolution() {
    let credentials = Arc::new(RecordingCredentialSource::default());
    let config = format!("{}\n[prompts]\npersonal_file = \"\"\n", valid_config());
    assert!(load("empty-prompt-path", &config, credentials.clone()).is_err());
    assert!(credentials.references().is_empty());
}

#[test]
fn request_budget_config_validates_without_credentials_or_vendor_binding() {
    for value in [0, 10_001] {
        let credentials = Arc::new(RecordingCredentialSource::default());
        let config = format!(
            "{}\n[runtime]\nmodel_request_limit_per_turn = {value}\n",
            valid_config()
        );
        let error = load("invalid-request-budget", &config, credentials.clone())
            .err()
            .unwrap();
        assert!(matches!(
            error,
            ProviderCompositionError::InvalidModelRequestLimit
        ));
        assert!(credentials.references().is_empty());
    }
    for value in [1, 2, 32, 33, 10_000] {
        let credentials = Arc::new(RecordingCredentialSource::default());
        let config = format!(
            "{}\n[runtime]\nmodel_request_limit_per_turn = {value}\n",
            valid_config()
        );
        assert!(load("valid-request-budget", &config, credentials.clone()).is_ok());
        assert!(credentials.references().is_empty());
    }
}

#[test]
fn auto_compact_config_is_optional_boolean_and_does_not_resolve_credentials() {
    for value in ["false", "true"] {
        let credentials = Arc::new(RecordingCredentialSource::default());
        let config = format!("{}\n[runtime]\nauto_compact = {value}\n", valid_config());
        assert!(load("auto-compact", &config, credentials.clone()).is_ok());
        assert!(credentials.references().is_empty());
    }
}

#[test]
fn transport_deadline_config_rejects_zero_and_unbounded_values_before_credentials() {
    for field in ["connect_timeout_ms", "read_timeout_ms"] {
        for value in [0, 86_400_001] {
            let credentials = Arc::new(RecordingCredentialSource::default());
            let config = valid_config().replacen(
                "[providers.primary]",
                &format!("[providers.primary]\n{field} = {value}"),
                1,
            );
            let error = load("invalid-deadline", &config, credentials.clone())
                .err()
                .unwrap();
            assert!(error.to_string().contains(field));
            assert!(credentials.references().is_empty());
        }
    }
}

#[tokio::test]
async fn configured_deadlines_retry_header_and_partial_sse_timeouts() {
    use echidna::adapters::tools::ToolRegistry;
    use echidna::app::agent_loop::{AgentLoop, AgentLoopError};
    use echidna::app::cancellation::AgentCancellation;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::time::{Duration, timeout};

    for send_partial_sse in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            for _ in 0..3 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = [0; 8192];
                assert!(socket.read(&mut bytes).await.unwrap() > 0);
                if send_partial_sse {
                    socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\"}\n\n").await.unwrap();
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        });
        let credentials = Arc::new(RecordingCredentialSource::default());
        let config = valid_config()
            .replacen("http://127.0.0.1:1/v1", &endpoint, 1)
            .replacen(
                "[providers.primary]",
                "[providers.primary]\nconnect_timeout_ms = 1000\nread_timeout_ms = 80",
                1,
            );
        let composition = load("provider-deadline", &config, credentials.clone()).unwrap();
        let directory = TemporaryDirectory::new("provider-deadline-retry");
        let tools = ToolRegistry::for_project(directory.path());
        let mut agent = AgentLoop::new(composition.primary_provider(), &tools);
        let error = timeout(
            Duration::from_secs(5),
            agent.run_turn("fixture", AgentCancellation::new()),
        )
        .await
        .expect("configured deadline must bound and retry the stalled request")
        .unwrap_err();
        assert!(matches!(
            error,
            AgentLoopError::Model(
                echidna::app::model_streaming::ModelStreamingError::Provider(
                    ModelProviderError::ConnectionFailure { timeout: true, .. }
                        | ModelProviderError::ResponseInterrupted { timeout: true, .. }
                )
            )
        ));
        assert_eq!(
            credentials.references(),
            vec!["CYBER_AGENT_PRIMARY_API_KEY"]
        );
        assert_eq!(
            agent.history().items(),
            &[ContentItem::user_text("fixture")]
        );
        server.await.unwrap();
    }
}

fn inline_primary_config() -> String {
    valid_config().replacen(
        "credential_ref = \"CYBER_AGENT_PRIMARY_API_KEY\"",
        &format!("api_key = \"{INLINE_API_KEY_FIXTURE}\""),
        1,
    )
}

fn load(
    label: &str,
    encoded: &str,
    credentials: Arc<dyn CredentialSource>,
) -> Result<ProviderComposition, ProviderCompositionError> {
    let directory = TemporaryDirectory::new(label);
    let path = directory.path().join("config.toml");
    fs::write(&path, encoded).unwrap();
    ProviderComposition::load(path, credentials)
}

#[test]
fn one_composition_compiles_primary_vision_and_doctor_without_reading_credentials() {
    let credentials = Arc::new(RecordingCredentialSource::default());
    let composition = load("provider-composition", &valid_config(), credentials.clone()).unwrap();

    assert!(credentials.references().is_empty());
    assert_eq!(
        composition.agent_profile().vision_mode(),
        VisionMode::SecondaryTool
    );
    assert_eq!(
        composition.agent_profile().resolve_vision_route(true).route,
        VisionRoute::SecondaryTool
    );
    assert_eq!(
        composition.agent_profile().primary().model().provider_id,
        "primary"
    );
    assert_eq!(
        composition.primary_request_adapter_version().as_str(),
        "responses-http-sse-v1"
    );
    assert_eq!(
        composition
            .agent_profile()
            .vision()
            .unwrap()
            .capabilities()
            .capability_mapping_version
            .get(),
        3
    );
    let report = inspect_agent_model_profile(composition.agent_profile());
    assert_eq!(report.status, AgentProfileDoctorStatus::Ready);
    assert_eq!(report.primary_reasoning_effort, None);
    assert_eq!(report.vision_reasoning_effort, None);
    assert!(
        serde_json::to_value(&report)
            .unwrap()
            .get("primary_reasoning_effort")
            .is_none()
    );
    assert!(
        serde_json::to_value(&report)
            .unwrap()
            .get("vision_reasoning_effort")
            .is_none()
    );
    assert!(credentials.references().is_empty());

    let stream = composition
        .primary_provider()
        .stream(ModelRequest::full(ModelInputIR::current(
            "fixed fixture",
            vec![ContentItem::user_text("do not connect")],
            ModelRequestRequirements::default(),
        )));
    assert!(stream.is_ok());
    drop(stream);
    assert_eq!(
        credentials.references(),
        vec!["CYBER_AGENT_PRIMARY_API_KEY"]
    );

    let vision_stream = composition
        .vision_provider()
        .unwrap()
        .stream(ModelRequest::full(ModelInputIR::current(
            "fixed vision fixture",
            vec![ContentItem::user_text("do not connect")],
            ModelRequestRequirements::default(),
        )));
    assert!(vision_stream.is_ok());
    drop(vision_stream);
    assert_eq!(
        credentials.references(),
        vec!["CYBER_AGENT_PRIMARY_API_KEY", "CYBER_AGENT_VISION_API_KEY"]
    );
}

#[test]
fn inline_api_key_reuses_the_composition_and_adapter_without_environment_resolution() {
    let credentials = Arc::new(RecordingCredentialSource::default());
    let composition = load(
        "provider-inline-api-key",
        &inline_primary_config(),
        credentials.clone(),
    )
    .unwrap();

    assert_eq!(
        inspect_agent_model_profile(composition.agent_profile()).status,
        AgentProfileDoctorStatus::Ready
    );
    assert!(credentials.references().is_empty());
    let stream = composition
        .primary_provider()
        .stream(ModelRequest::full(ModelInputIR::current(
            "fixed fixture",
            vec![ContentItem::user_text("do not connect")],
            ModelRequestRequirements::default(),
        )))
        .unwrap();
    drop(stream);
    assert!(
        credentials.references().is_empty(),
        "an inline credential must not fall through to the environment source"
    );
}

#[test]
fn config_is_strict_and_rejects_unsafe_or_dangling_bindings() {
    let cases = [
        (
            "schema",
            valid_config().replacen("schema_version = 1", "schema_version = 9", 1),
            "unsupported Provider configuration schema",
        ),
        (
            "active-profile",
            valid_config().replacen(
                "active_agent_profile = \"default\"",
                "active_agent_profile = \"missing\"",
                1,
            ),
            "active Agent profile `missing` does not exist",
        ),
        (
            "unknown-field",
            valid_config().replacen("generation = 1", "generation = 1\nunused = true", 1),
            "unknown field",
        ),
        (
            "credential",
            valid_config().replacen("CYBER_AGENT_PRIMARY_API_KEY", "OPENAI_API_KEY", 1),
            "dedicated CYBER_AGENT_*_API_KEY",
        ),
        (
            "credential-both",
            valid_config().replacen(
                "credential_ref = \"CYBER_AGENT_PRIMARY_API_KEY\"",
                "credential_ref = \"CYBER_AGENT_PRIMARY_API_KEY\"\napi_key = \"fixture-inline\"",
                1,
            ),
            "exactly one of `api_key`, `credential_ref`, `chatgpt_subscription`, or `codex_subscription`",
        ),
        (
            "credential-missing",
            valid_config().replacen("credential_ref = \"CYBER_AGENT_PRIMARY_API_KEY\"\n", "", 1),
            "exactly one of `api_key`, `credential_ref`, `chatgpt_subscription`, or `codex_subscription`",
        ),
        (
            "credential-empty-inline",
            valid_config().replacen(
                "credential_ref = \"CYBER_AGENT_PRIMARY_API_KEY\"",
                "api_key = \"   \"",
                1,
            ),
            "inline `api_key` must not be empty",
        ),
        (
            "binding",
            valid_config().replacen("binding = \"http_sse\"", "binding = \"unknown\"", 1),
            "unknown variant",
        ),
        (
            "base-url",
            valid_config().replacen(
                "http://127.0.0.1:1/v1",
                "https://user:password@example.invalid/v1?secret=value",
                1,
            ),
            "invalid base URL",
        ),
        (
            "mapping-version",
            valid_config().replacen(
                "[capability_mappings.text-v1]\nversion = 1",
                "[capability_mappings.text-v1]\nversion = 0",
                1,
            ),
            "must have a non-zero generation",
        ),
        (
            "context-window",
            valid_config().replacen("context_window = 32768", "context_window = 0", 1),
            "non-zero context window",
        ),
        (
            "mapping",
            valid_config().replacen(
                "capability_mapping = \"text-v1\"",
                "capability_mapping = \"missing-v1\"",
                1,
            ),
            "unknown capability mapping",
        ),
        (
            "provider",
            valid_config().replacen("primary = \"primary\"", "primary = \"missing\"", 1),
            "unknown primary Provider profile",
        ),
        (
            "inactive-profile",
            format!(
                "{}\n[agent_profiles.broken]\nprimary = \"primary\"\nvision_mode = \"primary_direct\"\ngeneration = 1\n",
                valid_config()
            ),
            "Agent profile `broken` is invalid",
        ),
    ];

    for (label, encoded, expected) in cases {
        let error = match load(
            label,
            &encoded,
            Arc::new(RecordingCredentialSource::default()),
        ) {
            Ok(_) => panic!("{label} unexpectedly composed"),
            Err(error) => error,
        };
        assert!(
            error.to_string().contains(expected),
            "{label}: expected `{expected}`, got `{error}`"
        );
    }
}

#[test]
fn malformed_inline_api_key_is_redacted_from_decode_errors() {
    let secret = "fixture-inline-value-must-never-appear";
    let encoded = valid_config().replacen(
        "credential_ref = \"CYBER_AGENT_PRIMARY_API_KEY\"",
        &format!("api_key = \"{secret}\" invalid"),
        1,
    );
    let error = match load(
        "provider-inline-api-key-redaction",
        &encoded,
        Arc::new(RecordingCredentialSource::default()),
    ) {
        Ok(_) => panic!("malformed inline API Key unexpectedly composed"),
        Err(error) => error,
    };
    let rendered = error.to_string();
    assert!(rendered.contains("<redacted>"), "{rendered}");
    assert!(!rendered.contains(secret), "{rendered}");
}

#[test]
fn model_names_never_override_the_explicit_capability_mapping() {
    let encoded = valid_config()
        .replacen(
            "model = \"fixture-text-model\"",
            "model = \"vision-ultra\"",
            1,
        )
        .replacen(
            "vision_mode = \"secondary_tool\"",
            "vision_mode = \"primary_direct\"",
            1,
        );

    let error = match load(
        "provider-capability-not-slug",
        &encoded,
        Arc::new(RecordingCredentialSource::default()),
    ) {
        Ok(_) => panic!("model name unexpectedly granted image capability"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        ProviderCompositionError::InvalidAgentProfile { .. }
    ));
    assert!(error.to_string().contains("requires primary image input"));
}

#[test]
fn one_responses_binding_accepts_any_configured_provider_without_name_inference() {
    let encoded = valid_config()
        .replacen(
            "base_url = \"http://127.0.0.1:1/v1\"",
            "base_url = \"https://api.example.invalid/v1\"",
            1,
        )
        .replacen(
            "model = \"fixture-text-model\"",
            "model = \"reasoning-provider-model\"",
            1,
        )
        .replacen(
            "credential_ref = \"CYBER_AGENT_PRIMARY_API_KEY\"",
            "credential_ref = \"CYBER_AGENT_ANY_RESPONSES_API_KEY\"",
            1,
        )
        .replacen("incremental_input = true", "incremental_input = false", 1);
    let credentials = Arc::new(RecordingCredentialSource::default());
    let composition = load("provider-protocol-first", &encoded, credentials.clone()).unwrap();

    assert_eq!(
        composition
            .agent_profile()
            .primary()
            .model()
            .model_id
            .to_string(),
        "reasoning-provider-model"
    );
    assert!(
        !composition
            .primary_provider()
            .capabilities()
            .incremental_input
    );
    let incremental = ModelRequest::incremental(
        ModelInputIR::current(
            "fixed fixture",
            vec![ContentItem::user_text("do not connect")],
            ModelRequestRequirements::default(),
        ),
        ProviderResponseId::new("must-not-cross-a-full-only-provider"),
    );
    assert!(matches!(
        composition.primary_provider().stream(incremental),
        Err(ModelProviderError::Capability(
            ModelCapabilityError::IncrementalInputUnsupported
        ))
    ));
    assert!(credentials.references().is_empty());

    let stream = composition
        .primary_provider()
        .stream(ModelRequest::full(ModelInputIR::current(
            "fixed fixture",
            vec![ContentItem::user_text("do not connect")],
            ModelRequestRequirements::default(),
        )))
        .unwrap();
    drop(stream);
    assert_eq!(
        credentials.references(),
        vec!["CYBER_AGENT_ANY_RESPONSES_API_KEY"]
    );
}

#[test]
fn configured_credential_failure_names_only_the_reference() {
    struct FailingSource;

    impl CredentialSource for FailingSource {
        fn read(&self, _reference: &str) -> Result<Option<String>, CredentialSourceError> {
            Err(CredentialSourceError::ReadFailed {
                message: "fixture failure".to_owned(),
            })
        }
    }

    let composition = load(
        "provider-credential-error",
        &valid_config(),
        Arc::new(FailingSource),
    )
    .unwrap();
    let result = composition
        .primary_provider()
        .stream(ModelRequest::full(ModelInputIR::current(
            "fixed fixture",
            vec![ContentItem::user_text("do not connect")],
            ModelRequestRequirements::default(),
        )));
    assert!(matches!(
        result,
        Err(ModelProviderError::CredentialSourceFailure { credential_ref, .. })
            if credential_ref == "CYBER_AGENT_PRIMARY_API_KEY"
    ));
}
