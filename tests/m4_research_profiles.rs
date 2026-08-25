use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use cyber_agent::adapters::mock_model::{MockModelFixture, MockModelProvider};
use cyber_agent::adapters::tools::ToolRegistry;
use cyber_agent::app::agent_loop::AgentLoop;
use cyber_agent::app::cancellation::AgentCancellation;
use cyber_agent::domain::context::{ContextBudget, ContextFragmentSource, ContextPlanner};
use cyber_agent::domain::mode::PromptComposition;
use cyber_agent::domain::model::{CapabilityMappingVersion, ModelCapabilities, ModelEvent};
use cyber_agent::domain::research::TargetKind;
use cyber_agent::domain::research_profile::ResearchProfile;
use cyber_agent::ports::model::{ModelResponseCompletion, ProviderResponseId};
use cyber_agent::ports::tool::ToolExecutor;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

const FIXTURE_DIRECTORY: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/m4_profiles");
const MANIFEST_SHA256: &str = "7bee1a2304ecb0a1100bc5b5183b51a0f384fac03b695d0cda8cd00fbfefa6c8";

fn capabilities() -> ModelCapabilities {
    ModelCapabilities {
        text_input: true,
        image_input: true,
        streaming: true,
        tool_call: true,
        structured_output: true,
        incremental_input: false,
        usage_reporting: true,
        context_window: 32_768,
        capability_mapping_version: CapabilityMappingVersion::new(1),
    }
}

fn profile_context_fingerprint(profile: ResearchProfile) -> String {
    let fragments = PromptComposition::new("Base research instructions.")
        .unwrap()
        .with_mode(profile.mode_pack())
        .fragments()
        .unwrap();
    let tools = ToolRegistry::for_project_with_exposure(
        Path::new(env!("CARGO_MANIFEST_DIR")),
        profile.mode_pack().tool_exposure().clone(),
    )
    .unwrap();
    ContextPlanner::current()
        .compile(
            fragments,
            ContextBudget::new(32_768, 0, 0).unwrap(),
            &capabilities(),
            tools.specs(),
            Default::default(),
            None,
        )
        .unwrap()
        .context_fingerprint()
        .to_owned()
}

#[test]
fn fixed_profiles_compile_versioned_markdown_into_stable_context_contracts() {
    let expected = [
        (ResearchProfile::Source, "source", TargetKind::Source),
        (ResearchProfile::Web, "web", TargetKind::Web),
        (ResearchProfile::Mobile, "mobile", TargetKind::Mobile),
        (ResearchProfile::Binary, "binary", TargetKind::Binary),
    ];

    for (profile, id, target_kind) in expected {
        assert_eq!(profile.id(), id);
        assert_eq!(profile.generation(), 1);
        assert_eq!(profile.target_kind(), target_kind);
        assert!(profile.supports_target_kind(target_kind));
        assert!(!profile.supports_target_kind(TargetKind::Firmware));

        let mode = profile.mode_pack();
        let prompt = mode.prompt();
        for required_boundary in [
            "Model output, reasoning, plans, and summaries are not facts.",
            "Candidate",
            "Reproduced",
            "Confirmed",
            "Only an explicit `research_update` Tool result can create or advance a Research record.",
            "real security impact",
            "target version, environment, prerequisites, positive result, and a relevant negative control",
            "Do not invent Tool output",
        ] {
            assert!(
                prompt.contains(required_boundary),
                "{} must retain `{required_boundary}`",
                profile.id()
            );
        }
        let domain_audit_keyword = match profile {
            ResearchProfile::Source => {
                "input validation, trust boundaries, entry points, sensitive sinks, and build configuration"
            }
            ResearchProfile::Web => {
                "authentication, authorization, Origin, CSRF, CORS, tenant isolation, response impact"
            }
            ResearchProfile::Mobile => {
                "exported components, Intent and IPC boundaries, deep links, WebView and JSBridge, permission and storage boundaries"
            }
            ResearchProfile::Binary => "length fields, memory boundaries, permissions",
        };
        assert!(prompt.contains(domain_audit_keyword));
        if profile == ResearchProfile::Mobile {
            assert!(
                prompt.contains("Root, ADB, and Frida may observe or deploy experiments but are not ordinary attacker prerequisites.")
            );
        }
        if profile == ResearchProfile::Binary {
            assert!(prompt.contains("Distinguish a crash from demonstrated security impact."));
        }

        let composition = PromptComposition::new("Base research instructions.")
            .unwrap()
            .with_mode(profile.mode_pack());
        let fragments = composition.fragments().unwrap();
        assert_eq!(
            fragments
                .iter()
                .map(|fragment| fragment.id().as_str().to_owned())
                .collect::<Vec<_>>(),
            vec!["prompt:base".to_owned(), format!("prompt:mode:{id}:1")]
        );
        assert_eq!(fragments[1].source(), ContextFragmentSource::ModePrompt);

        let plan = ContextPlanner::current()
            .compile(
                fragments,
                ContextBudget::new(32_768, 0, 0).unwrap(),
                &capabilities(),
                Vec::new(),
                Default::default(),
                None,
            )
            .unwrap();
        assert!(plan.base_instructions().contains(prompt));
        assert_eq!(
            plan.context_fingerprint(),
            ContextPlanner::current()
                .compile(
                    PromptComposition::new("Base research instructions.")
                        .unwrap()
                        .with_mode(profile.mode_pack())
                        .fragments()
                        .unwrap(),
                    ContextBudget::new(32_768, 0, 0).unwrap(),
                    &capabilities(),
                    Vec::new(),
                    Default::default(),
                    None,
                )
                .unwrap()
                .context_fingerprint()
        );

        // This construction validates Profile overrides against the existing
        // fixed ToolRegistry without executing a Tool or touching the network.
        let tool_registry = ToolRegistry::for_project_with_exposure(
            Path::new(env!("CARGO_MANIFEST_DIR")),
            profile.mode_pack().tool_exposure().clone(),
        )
        .unwrap();
        let specs = tool_registry.specs();
        let visible_names = specs
            .iter()
            .map(|spec| spec.name.as_str())
            .collect::<BTreeSet<_>>();
        assert!(visible_names.contains("run_command"));
        assert!(visible_names.contains("run_commands"));
        assert_eq!(
            visible_names.contains("view_image"),
            profile != ResearchProfile::Source
        );
    }
}

#[tokio::test]
async fn agent_loop_consumes_each_fixed_profile_at_the_existing_mode_boundary() {
    for profile in ResearchProfile::ALL {
        let provider = MockModelProvider::new(MockModelFixture {
            capabilities: capabilities(),
            events: vec![ModelEvent::TextDelta {
                text: format!("{} fixture response", profile.id()),
            }],
            completion: ModelResponseCompletion::new(ProviderResponseId::new(format!(
                "{}-response",
                profile.id()
            ))),
        });
        let tools = ToolRegistry::for_project(Path::new(env!("CARGO_MANIFEST_DIR")));
        let pending_composition = PromptComposition::new("Pending base instructions.")
            .unwrap()
            .with_personal("Pending personal instructions.")
            .unwrap()
            .with_project("Pending project instructions.")
            .unwrap();
        let mut agent = AgentLoop::new(&provider, &tools, "Original base instructions.")
            .with_prompt_composition(pending_composition)
            .with_research_profile(profile);

        agent
            .run_turn("Use only the synthetic fixture.", AgentCancellation::new())
            .await
            .unwrap();
        let plan = agent.last_context_plan().unwrap();
        assert!(
            plan.base_instructions()
                .contains(profile.mode_pack().prompt())
        );
        assert!(
            plan.base_instructions()
                .contains("Pending personal instructions.")
        );
        assert!(
            plan.base_instructions()
                .contains("Pending project instructions.")
        );
        assert!(
            plan.full_logical_manifest()
                .entries()
                .iter()
                .any(|entry| entry.fragment_id().as_str()
                    == format!("prompt:mode:{}:{}", profile.id(), profile.generation()))
        );
    }
}

#[test]
fn fixed_profile_context_fingerprints_are_stable_and_unique() {
    let fingerprints = ResearchProfile::ALL.map(profile_context_fingerprint);
    assert_eq!(
        fingerprints,
        [
            "0590cfffcb56a948c59ff189e6586596706c579ae1dcdc3660314e866200ef48",
            "a2c81d766b27dfe82cccab73713b665bbf5317ded4bf2697aa6233de8a78603c",
            "039c54ca82986efd7ba411e2c93f818c3c3293b0bfcf1b1a6bdf1cdabbf69e62",
            "a4b17dbf094c2507aacd0066203f4fc44713b789c3fa4dbe7c360208cdd7e85e",
        ]
    );
    assert_eq!(
        fingerprints.iter().collect::<BTreeSet<_>>().len(),
        ResearchProfile::ALL.len()
    );
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ControlledCaseManifest {
    schema_version: u16,
    suite_id: String,
    provenance: String,
    network_access: String,
    cases: Vec<ControlledCase>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ControlledCase {
    id: String,
    profile_id: String,
    target_kind: TargetKind,
    role: ControlledCaseRole,
    expected_stage: ControlledStage,
    fixture: String,
    sha256: String,
    relationship: ControlledCaseRelationship,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ControlledCaseRole {
    Positive,
    NegativeControl,
    TamperedControl,
    EnvironmentControl,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(rename_all = "snake_case")]
enum ControlledStage {
    Candidate,
    Reproduced,
    Confirmed,
    NoFinding,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ControlledCaseRelationship {
    positive_case_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ControlledFixture {
    case_id: String,
    synthetic_target: String,
    environment: String,
    expected_stage: ControlledStage,
    model_output_is_fact: bool,
    requires_research_update: bool,
    sample: Value,
}

struct FixtureSuite {
    root: PathBuf,
    manifest: ControlledCaseManifest,
}

impl FixtureSuite {
    fn load() -> Self {
        let root = PathBuf::from(FIXTURE_DIRECTORY);
        let manifest = fs::read(root.join("manifest.json")).unwrap();
        assert_eq!(sha256(&manifest), MANIFEST_SHA256.trim());
        Self {
            root,
            manifest: serde_json::from_slice(&manifest).unwrap(),
        }
    }

    fn load_case(&self, case: &ControlledCase) -> Result<ControlledFixture, FixtureLoadError> {
        let bytes = fs::read(self.fixture_path(case)?).map_err(|_| FixtureLoadError::Read)?;
        self.parse_case(case, &bytes)
    }

    fn parse_case(
        &self,
        case: &ControlledCase,
        bytes: &[u8],
    ) -> Result<ControlledFixture, FixtureLoadError> {
        let actual = sha256(bytes);
        if actual != case.sha256 {
            return Err(FixtureLoadError::DigestMismatch);
        }
        serde_json::from_slice(bytes).map_err(|_| FixtureLoadError::Decode)
    }

    fn fixture_path(&self, case: &ControlledCase) -> Result<PathBuf, FixtureLoadError> {
        if case.fixture.is_empty()
            || Path::new(&case.fixture)
                .components()
                .any(|component| !matches!(component, std::path::Component::Normal(_)))
        {
            return Err(FixtureLoadError::InvalidFixturePath);
        }
        Ok(self.root.join(&case.fixture))
    }
}

#[derive(Debug)]
enum FixtureLoadError {
    Read,
    Decode,
    DigestMismatch,
    InvalidFixturePath,
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[test]
fn controlled_cases_are_complete_related_and_digest_verified() {
    let suite = FixtureSuite::load();
    assert_eq!(suite.manifest.schema_version, 1);
    assert_eq!(suite.manifest.suite_id, "m4-fixed-research-profiles-v1");
    assert_eq!(suite.manifest.provenance, "synthetic-local-offline");
    assert_eq!(suite.manifest.network_access, "disabled");
    assert_eq!(suite.manifest.cases.len(), 12);

    let profiles = ResearchProfile::ALL
        .into_iter()
        .map(|profile| (profile.id(), profile))
        .collect::<BTreeMap<_, _>>();
    let cases_by_id = suite
        .manifest
        .cases
        .iter()
        .map(|case| (case.id.as_str(), case))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(cases_by_id.len(), suite.manifest.cases.len());
    assert_eq!(
        suite
            .manifest
            .cases
            .iter()
            .map(|case| case.fixture.as_str())
            .collect::<BTreeSet<_>>()
            .len(),
        suite.manifest.cases.len()
    );

    let mut positive_stages = BTreeSet::new();
    for profile in ResearchProfile::ALL {
        let cases = suite
            .manifest
            .cases
            .iter()
            .filter(|case| case.profile_id == profile.id())
            .collect::<Vec<_>>();
        assert_eq!(cases.len(), 3, "{} case count", profile.id());
        let positive = cases
            .iter()
            .find(|case| matches!(case.role, ControlledCaseRole::Positive))
            .expect("each Profile has one positive");
        assert_eq!(
            cases
                .iter()
                .filter(|case| matches!(case.role, ControlledCaseRole::Positive))
                .count(),
            1
        );
        assert!(
            cases
                .iter()
                .any(|case| matches!(case.role, ControlledCaseRole::NegativeControl))
        );
        assert!(cases.iter().any(|case| matches!(
            case.role,
            ControlledCaseRole::TamperedControl | ControlledCaseRole::EnvironmentControl
        )));
        positive_stages.insert(positive.expected_stage);
        let positive_fixture = suite.load_case(positive).unwrap();
        assert_eq!(positive_fixture.sample["kind"], profile.id());

        for case in cases.iter().copied() {
            let declared_profile = profiles.get(case.profile_id.as_str()).unwrap();
            assert_eq!(case.target_kind, declared_profile.target_kind());
            let linked_positive = cases_by_id
                .get(case.relationship.positive_case_id.as_str())
                .unwrap();
            assert_eq!(linked_positive.id, positive.id);
            assert_eq!(linked_positive.profile_id, case.profile_id);
            assert_eq!(linked_positive.target_kind, case.target_kind);

            let fixture = suite.load_case(case).unwrap();
            assert_eq!(fixture.case_id, case.id);
            assert_eq!(fixture.expected_stage, case.expected_stage);
            assert!(fixture.synthetic_target.starts_with("local-"));
            assert!(fixture.environment.starts_with("offline-"));
            assert!(!fixture.model_output_is_fact);
            assert!(fixture.requires_research_update);
            assert_eq!(fixture.sample["kind"], profile.id());
            assert!(fixture.sample["decision"].as_str().is_some());
            if !matches!(case.role, ControlledCaseRole::Positive) {
                assert_ne!(fixture.sample, positive_fixture.sample);
            }

            if matches!(case.role, ControlledCaseRole::Positive) {
                assert_ne!(case.expected_stage, ControlledStage::NoFinding);
            } else {
                assert_eq!(case.expected_stage, ControlledStage::NoFinding);
            }
        }

        let negative = cases
            .iter()
            .find(|case| matches!(case.role, ControlledCaseRole::NegativeControl))
            .unwrap();
        let negative_fixture = suite.load_case(negative).unwrap();
        match profile {
            ResearchProfile::Source => {
                assert!(
                    positive_fixture.sample["file"]["content"]
                        .as_str()
                        .unwrap()
                        .contains("unvalidated path reaches file sink")
                );
                assert!(
                    negative_fixture.sample["file"]["content"]
                        .as_str()
                        .unwrap()
                        .contains("reject_parent_segments")
                );
            }
            ResearchProfile::Web => {
                assert_eq!(positive_fixture.sample["response"]["status"], 200);
                assert!(
                    positive_fixture.sample["response"]["body"]
                        .as_str()
                        .unwrap()
                        .contains("acme")
                );
                assert_eq!(negative_fixture.sample["response"]["status"], 403);
            }
            ResearchProfile::Mobile => {
                assert!(
                    positive_fixture.sample["manifest"]
                        .as_str()
                        .unwrap()
                        .contains("android:exported=\"true\"")
                );
                assert!(
                    negative_fixture.sample["manifest"]
                        .as_str()
                        .unwrap()
                        .contains("android:exported=\"false\"")
                );
                let impact = &positive_fixture.sample["impact_proof"];
                assert_eq!(impact["target_version"], "fixture-mobile-1.0.0");
                assert_eq!(impact["environment"], "offline-android-api-35");
                assert_eq!(impact["positive_case_id"], positive.id);
                assert_eq!(impact["negative_control_case_id"], negative.id);
                assert!(impact["reproducible_impact"].as_str().is_some());
            }
            ResearchProfile::Binary => {
                assert!(
                    positive_fixture.sample["parser"]
                        .as_str()
                        .unwrap()
                        .contains("memcpy")
                );
                assert!(
                    negative_fixture.sample["parser"]
                        .as_str()
                        .unwrap()
                        .contains("ERR_TRUNCATED")
                );
                let environment_control = cases
                    .iter()
                    .find(|case| matches!(case.role, ControlledCaseRole::EnvironmentControl))
                    .unwrap();
                let environment_fixture = suite.load_case(environment_control).unwrap();
                assert!(
                    environment_fixture.sample["observed_result"]
                        .as_str()
                        .unwrap()
                        .contains("crash alone is not security impact")
                );
            }
        }
    }
    assert_eq!(
        positive_stages,
        BTreeSet::from([
            ControlledStage::Candidate,
            ControlledStage::Reproduced,
            ControlledStage::Confirmed,
        ])
    );
}

#[test]
fn controlled_fixture_loader_rejects_tampered_bytes() {
    let suite = FixtureSuite::load();
    let case = suite
        .manifest
        .cases
        .iter()
        .find(|case| case.id == "source-positive-candidate")
        .unwrap();
    let mut bytes = fs::read(suite.fixture_path(case).unwrap()).unwrap();
    bytes.extend_from_slice(b"\ntrailing tamper");
    assert!(matches!(
        suite.parse_case(case, &bytes),
        Err(FixtureLoadError::DigestMismatch)
    ));
}
