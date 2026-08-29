use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use cyber_agent::adapters::mock_model::{MockModelFixture, MockModelProvider};
use cyber_agent::adapters::tools::ToolRegistry;
use cyber_agent::app::agent_loop::AgentLoop;
use cyber_agent::app::cancellation::AgentCancellation;
use cyber_agent::app::session::{AppendAfterDurableError, SessionProjection};
use cyber_agent::domain::context::{
    ContextBudget, ContextFragmentSource, ContextPlanner, ProviderSession, ProviderSessionId,
    ProviderSessionRef, ProviderSessionRole,
};
use cyber_agent::domain::domain_profile::DomainProfile;
use cyber_agent::domain::events::{
    EventEnvelope, EventId, EventSequence, EventType, OccurredAt, StreamId,
};
use cyber_agent::domain::model::{CapabilityMappingVersion, ModelCapabilities, ModelEvent};
use cyber_agent::domain::research::TargetKind;
use cyber_agent::domain::research_control::{
    BaseContract, PromptComposition, ResearchPhase, ResearchPhaseSelection,
};
use cyber_agent::domain::session::{SessionError, SessionEvent, SessionReducer, SessionState};
use cyber_agent::ports::model::{ModelResponseCompletion, ProviderResponseId};
use cyber_agent::ports::{EventSink, tool::ToolExecutor};
use serde::Deserialize;
use serde_json::{Value, json};
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

fn control_context_fingerprint(profile: DomainProfile, phase: ResearchPhase) -> String {
    let composition = PromptComposition::current()
        .with_domain_profile(profile)
        .with_research_phase(phase);
    let fragments = PromptComposition::current()
        .with_domain_profile(profile)
        .with_research_phase(phase)
        .fragments()
        .unwrap();
    let tools = ToolRegistry::for_project(Path::new(env!("CARGO_MANIFEST_DIR")));
    let resolved = tools
        .resolve_tool_plan(&composition.tool_exposure(), &Default::default())
        .unwrap();
    ContextPlanner::current()
        .compile(
            fragments,
            ContextBudget::new(32_768, 0, 0).unwrap(),
            &capabilities(),
            resolved.advertised_specs().to_vec(),
            Default::default(),
            None,
        )
        .unwrap()
        .context_fingerprint()
        .to_owned()
}

#[test]
fn stable_base_domain_and_phase_compile_into_one_orthogonal_contract() {
    let base = BaseContract;
    for required_boundary in [
        "Pragmatic Vulnerability Researcher",
        "Candidate, Reproduced and Confirmed impact",
        "positive, negative and environmental controls",
        "Never invent tool output",
    ] {
        assert!(base.prompt().contains(required_boundary));
    }

    let expected = [
        (DomainProfile::Source, "source", TargetKind::Source),
        (DomainProfile::Web, "web", TargetKind::Web),
        (DomainProfile::Mobile, "mobile", TargetKind::Mobile),
        (DomainProfile::Binary, "binary", TargetKind::Binary),
    ];

    for (profile, id, target_kind) in expected {
        assert_eq!(profile.id(), id);
        assert_eq!(profile.generation(), 1);
        assert_eq!(profile.target_kind(), target_kind);
        assert!(profile.supports_target_kind(target_kind));
        assert!(!profile.supports_target_kind(TargetKind::Firmware));

        let prompt = profile.prompt();
        let domain_audit_keyword = match profile {
            DomainProfile::Source => "attacker-controlled inputs",
            DomainProfile::Web => "authentication, authorization, tenant, Origin, CSRF and CORS",
            DomainProfile::Mobile => "exported components, Intent and IPC boundaries",
            DomainProfile::Binary => "length and lifetime fields",
        };
        assert!(prompt.contains(domain_audit_keyword));
        if profile == DomainProfile::Mobile {
            assert!(prompt.contains("Root, ADB and Frida"));
        }
        if profile == DomainProfile::Binary {
            assert!(prompt.contains("Distinguish a crash"));
        }

        let composition = PromptComposition::current()
            .with_domain_profile(profile)
            .with_research_phase(ResearchPhase::Mapping);
        let fragments = composition.fragments().unwrap();
        assert_eq!(
            fragments
                .iter()
                .map(|fragment| fragment.id().as_str().to_owned())
                .collect::<Vec<_>>(),
            vec![
                "prompt:base:pragmatic-vulnerability-researcher:1".to_owned(),
                format!("prompt:domain:{id}:1"),
                "prompt:phase:mapping:1".to_owned(),
            ]
        );
        assert_eq!(fragments[1].source(), ContextFragmentSource::DomainPrompt);
        assert_eq!(fragments[2].source(), ContextFragmentSource::PhasePrompt);

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
                    PromptComposition::current()
                        .with_domain_profile(profile)
                        .with_research_phase(ResearchPhase::Mapping)
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
        let tool_registry = ToolRegistry::for_project(Path::new(env!("CARGO_MANIFEST_DIR")));
        let specs = tool_registry
            .resolve_tool_plan(&composition.tool_exposure(), &Default::default())
            .unwrap();
        let visible_names = specs
            .advertised_specs()
            .iter()
            .map(|spec| spec.name.as_str())
            .collect::<BTreeSet<_>>();
        assert!(visible_names.contains("run_command"));
        assert!(visible_names.contains("run_commands"));
        assert_eq!(
            visible_names.contains("view_image"),
            profile != DomainProfile::Source
        );
    }
}

#[tokio::test]
async fn agent_loop_consumes_each_domain_and_phase_at_the_turn_boundary() {
    for profile in DomainProfile::ALL {
        for phase in ResearchPhase::ALL {
            let provider = MockModelProvider::new(MockModelFixture {
                capabilities: capabilities(),
                events: vec![ModelEvent::TextDelta {
                    text: format!("{} {} fixture response", profile.id(), phase.id()),
                }],
                completion: ModelResponseCompletion::new(ProviderResponseId::new(format!(
                    "{}-{}-response",
                    profile.id(),
                    phase.id()
                ))),
            });
            let tools = ToolRegistry::for_project(Path::new(env!("CARGO_MANIFEST_DIR")));
            let pending_composition = PromptComposition::current()
                .with_personal("Pending personal instructions.")
                .unwrap()
                .with_project("Pending project instructions.")
                .unwrap();
            let mut agent = AgentLoop::new(&provider, &tools)
                .with_prompt_composition(pending_composition)
                .with_domain_profile(profile)
                .with_research_phase(phase);

            agent
                .run_turn("Use only the synthetic fixture.", AgentCancellation::new())
                .await
                .unwrap();
            let plan = agent.last_context_plan().unwrap();
            assert!(plan.base_instructions().contains(profile.prompt()));
            assert!(plan.base_instructions().contains(phase.prompt()));
            assert!(
                plan.base_instructions()
                    .contains("Pending personal instructions.")
            );
            assert!(
                plan.base_instructions()
                    .contains("Pending project instructions.")
            );
            assert!(
                !plan
                    .full_logical_manifest()
                    .entries()
                    .iter()
                    .any(|entry| entry.fragment_id().as_str()
                        == format!("prompt:mode:{}:{}", profile.id(), profile.generation()))
            );
            assert!(
                plan.full_logical_manifest()
                    .entries()
                    .iter()
                    .any(|entry| entry.fragment_id().as_str()
                        == format!("prompt:domain:{}:{}", profile.id(), profile.generation()))
            );
            assert!(
                plan.full_logical_manifest()
                    .entries()
                    .iter()
                    .any(|entry| entry.fragment_id().as_str()
                        == format!("prompt:phase:{}:{}", phase.id(), phase.generation()))
            );
            assert_eq!(
                agent
                    .last_sampling_step_plan()
                    .unwrap()
                    .turn_configuration()
                    .domain_profile(),
                profile
            );
            assert_eq!(
                agent
                    .last_sampling_step_plan()
                    .unwrap()
                    .turn_configuration()
                    .research_phase(),
                phase
            );
        }
    }
}

#[test]
fn four_by_five_context_fingerprints_are_stable_and_unique() {
    let fingerprints = DomainProfile::ALL
        .into_iter()
        .flat_map(|profile| {
            ResearchPhase::ALL
                .into_iter()
                .map(move |phase| control_context_fingerprint(profile, phase))
        })
        .collect::<Vec<_>>();
    let repeated = DomainProfile::ALL
        .into_iter()
        .flat_map(|profile| {
            ResearchPhase::ALL
                .into_iter()
                .map(move |phase| control_context_fingerprint(profile, phase))
        })
        .collect::<Vec<_>>();
    assert_eq!(fingerprints, repeated);
    assert_eq!(
        fingerprints.iter().collect::<BTreeSet<_>>().len(),
        DomainProfile::ALL.len() * ResearchPhase::ALL.len()
    );
}

#[test]
fn four_by_five_tool_exposure_matrix_is_deterministic_and_phase_bounded() {
    let tools = ToolRegistry::for_project(Path::new(env!("CARGO_MANIFEST_DIR")));
    let mut matrix = BTreeMap::new();
    for domain in DomainProfile::ALL {
        for phase in ResearchPhase::ALL {
            let composition = PromptComposition::current()
                .with_domain_profile(domain)
                .with_research_phase(phase);
            let exposed = tools
                .resolve_tool_plan(&composition.tool_exposure(), &Default::default())
                .unwrap()
                .advertised_specs()
                .iter()
                .map(|spec| spec.name.as_str().to_owned())
                .collect::<BTreeSet<_>>();
            assert_eq!(
                exposed.contains("view_image"),
                domain != DomainProfile::Source,
                "{} / {}",
                domain.id(),
                phase.id(),
            );
            assert_eq!(
                exposed.contains("write_file"),
                phase == ResearchPhase::Experiment,
                "{} / {}",
                domain.id(),
                phase.id(),
            );
            assert_eq!(
                exposed.contains("apply_patch"),
                phase == ResearchPhase::Experiment,
                "{} / {}",
                domain.id(),
                phase.id(),
            );
            assert_eq!(
                exposed.contains("memory_write"),
                phase == ResearchPhase::EvidenceCuration,
                "{} / {}",
                domain.id(),
                phase.id(),
            );
            if phase == ResearchPhase::Mapping {
                for mutation in [
                    "write_file",
                    "apply_patch",
                    "start_job",
                    "poll_job",
                    "input_job",
                    "stop_job",
                    "start_pty",
                    "poll_pty",
                    "input_pty",
                    "resize_pty",
                    "stop_pty",
                    "memory_write",
                ] {
                    assert!(
                        !exposed.contains(mutation),
                        "{} / mapping must defer mutation-capable `{mutation}`",
                        domain.id(),
                    );
                }
                for inspection in [
                    "read_file",
                    "search_files",
                    "run_command",
                    "run_commands",
                    "research_get",
                    "research_update",
                    "tool_search",
                ] {
                    assert!(
                        exposed.contains(inspection),
                        "{} / mapping must expose the bounded inspection `{inspection}`",
                        domain.id(),
                    );
                }
            }
            assert!(matrix.insert((domain.id(), phase.id()), exposed).is_none());
        }
    }
    assert_eq!(
        matrix.len(),
        DomainProfile::ALL.len() * ResearchPhase::ALL.len()
    );
}

fn phase_session_stream() -> StreamId {
    StreamId::new("session:m4-research-control")
}

fn phase_started_event(sequence: u64) -> EventEnvelope<Value> {
    SessionEvent::Started {
        primary_provider_session: ProviderSessionRef {
            id: ProviderSessionId::new("primary:m4:one"),
            role: ProviderSessionRole::Primary,
            generation: 1,
        },
    }
    .into_envelope(
        EventId::new(format!("m4-started-{sequence}")),
        phase_session_stream(),
        EventSequence::new(sequence),
        OccurredAt::new("2026-08-29T00:00:00Z"),
    )
    .unwrap()
}

fn raw_phase_event(sequence: u64, payload: Value) -> EventEnvelope<Value> {
    EventEnvelope::current(
        EventId::new(format!("m4-phase-{sequence}")),
        phase_session_stream(),
        EventSequence::new(sequence),
        OccurredAt::new("2026-08-29T00:00:00Z"),
        EventType::new("session.research_phase_selected"),
        payload,
    )
}

fn valid_phase_payload(selection: Value) -> Value {
    json!({"schema_version": 1, "selection": selection})
}

#[test]
fn phase_selection_event_is_versioned_strict_and_source_reason_paired() {
    let selection = ResearchPhaseSelection::operator(ResearchPhase::Verification);
    let event = SessionEvent::ResearchPhaseSelected { selection }
        .into_envelope(
            EventId::new("phase-selected"),
            phase_session_stream(),
            EventSequence::new(2),
            OccurredAt::new("2026-08-29T00:00:00Z"),
        )
        .unwrap();
    assert_eq!(
        event.payload,
        valid_phase_payload(json!({
            "phase": "verification",
            "phase_generation": 1,
            "source": "operator",
            "reason_code": "OPERATOR_SELECTED"
        }))
    );

    let invalid_payloads = [
        (
            valid_phase_payload(json!({
                "phase": "verification",
                "phase_generation": 2,
                "source": "operator",
                "reason_code": "OPERATOR_SELECTED"
            })),
            "generation",
        ),
        (
            valid_phase_payload(json!({
                "phase": "experiment",
                "phase_generation": 1,
                "source": "default",
                "reason_code": "LEGACY_DEFAULT_MAPPING"
            })),
            "default phase",
        ),
        (
            valid_phase_payload(json!({
                "phase": "mapping",
                "phase_generation": 1,
                "source": "operator",
                "reason_code": "NEW_SESSION_DEFAULT_MAPPING"
            })),
            "operator reason",
        ),
        (
            json!({
                "schema_version": 1,
                "selection": {
                    "phase": "mapping",
                    "phase_generation": 1,
                    "source": "default",
                    "reason_code": "NEW_SESSION_DEFAULT_MAPPING"
                },
                "unexpected": true
            }),
            "outer unknown field",
        ),
        (
            valid_phase_payload(json!({
                "phase": "mapping",
                "phase_generation": 1,
                "source": "default",
                "reason_code": "NEW_SESSION_DEFAULT_MAPPING",
                "unexpected": true
            })),
            "nested unknown field",
        ),
    ];

    for (payload, label) in invalid_payloads {
        let mut state = SessionState::default();
        SessionReducer
            .apply(&mut state, phase_started_event(1))
            .unwrap();
        let before = state.clone();
        assert!(
            matches!(
                SessionReducer.apply(&mut state, raw_phase_event(2, payload)),
                Err(SessionError::MalformedPayload { .. })
            ),
            "{label}"
        );
        assert_eq!(state, before, "{label} must not mutate the projection");
    }

    let mut state = SessionState::default();
    SessionReducer
        .apply(&mut state, phase_started_event(1))
        .unwrap();
    assert!(matches!(
        SessionReducer.apply(
            &mut state,
            raw_phase_event(
                2,
                json!({
                    "schema_version": 2,
                    "selection": {
                        "phase": "mapping",
                        "phase_generation": 1,
                        "source": "default",
                        "reason_code": "NEW_SESSION_DEFAULT_MAPPING"
                    }
                })
            )
        ),
        Err(SessionError::UnsupportedPayloadSchema {
            schema_version: 2,
            ..
        })
    ));
}

#[test]
fn legacy_phase_replay_stays_read_only_and_phase_append_is_atomic() {
    let mut projection = SessionProjection::replay([phase_started_event(1)]).unwrap();
    assert_eq!(projection.state().research_phase_selection(), None);
    assert_eq!(
        projection.state().effective_research_phase(),
        ResearchPhase::Mapping
    );
    assert_eq!(projection.state().applied_event_count(), 1);

    struct FailingSink;
    impl EventSink<Value> for FailingSink {
        type Error = &'static str;

        fn append_event(&mut self, _event: &EventEnvelope<Value>) -> Result<(), Self::Error> {
            Err("phase journal unavailable")
        }
    }

    let before = projection.state().clone();
    let event = SessionEvent::ResearchPhaseSelected {
        selection: ResearchPhaseSelection::legacy_default(),
    }
    .into_envelope(
        EventId::new("legacy-default"),
        phase_session_stream(),
        EventSequence::new(2),
        OccurredAt::new("2026-08-29T00:00:00Z"),
    )
    .unwrap();
    assert!(matches!(
        projection.append(&mut FailingSink, event),
        Err(AppendAfterDurableError::Sink("phase journal unavailable"))
    ));
    assert_eq!(projection.state(), &before);
    assert_eq!(projection.state().research_phase_selection(), None);

    #[derive(Default)]
    struct RecordingSink(Vec<EventEnvelope<Value>>);
    impl EventSink<Value> for RecordingSink {
        type Error = std::convert::Infallible;

        fn append_event(&mut self, event: &EventEnvelope<Value>) -> Result<(), Self::Error> {
            self.0.push(event.clone());
            Ok(())
        }
    }
    let mut sink = RecordingSink::default();
    let durable = SessionEvent::ResearchPhaseSelected {
        selection: ResearchPhaseSelection::legacy_default(),
    }
    .into_envelope(
        EventId::new("legacy-default-durable"),
        phase_session_stream(),
        EventSequence::new(2),
        OccurredAt::new("2026-08-29T00:00:00Z"),
    )
    .unwrap();
    projection.append(&mut sink, durable).unwrap();
    assert_eq!(sink.0.len(), 1);
    assert_eq!(
        projection.state().research_phase_selection(),
        Some(ResearchPhaseSelection::legacy_default())
    );
}

#[tokio::test]
async fn resume_restores_the_latest_durable_phase_before_capturing_a_step() {
    let primary = ProviderSessionRef {
        id: ProviderSessionId::new("primary:m4:two"),
        role: ProviderSessionRole::Primary,
        generation: 2,
    };
    let state = SessionReducer
        .reduce([
            phase_started_event(1),
            SessionEvent::ResearchPhaseSelected {
                selection: ResearchPhaseSelection::operator(ResearchPhase::Hypothesis),
            }
            .into_envelope(
                EventId::new("m4-phase-before-resume"),
                phase_session_stream(),
                EventSequence::new(2),
                OccurredAt::new("2026-08-29T00:00:00Z"),
            )
            .unwrap(),
            SessionEvent::Resumed {
                primary_provider_session: primary.clone(),
            }
            .into_envelope(
                EventId::new("m4-resumed"),
                phase_session_stream(),
                EventSequence::new(3),
                OccurredAt::new("2026-08-29T00:00:00Z"),
            )
            .unwrap(),
            SessionEvent::ResearchPhaseSelected {
                selection: ResearchPhaseSelection::operator(ResearchPhase::EvidenceCuration),
            }
            .into_envelope(
                EventId::new("m4-phase-latest"),
                phase_session_stream(),
                EventSequence::new(4),
                OccurredAt::new("2026-08-29T00:00:00Z"),
            )
            .unwrap(),
        ])
        .unwrap();
    assert_eq!(
        state.effective_research_phase(),
        ResearchPhase::EvidenceCuration
    );

    let provider = MockModelProvider::new(MockModelFixture {
        capabilities: capabilities(),
        events: vec![ModelEvent::TextDelta {
            text: "resumed fixture".to_owned(),
        }],
        completion: ModelResponseCompletion::without_response_id(),
    });
    let tools = ToolRegistry::for_project(Path::new(env!("CARGO_MANIFEST_DIR")));
    let mut agent = AgentLoop::new(&provider, &tools);
    agent
        .resume_from_session(
            &state,
            ProviderSession::new(primary.id, ProviderSessionRole::Primary, primary.generation),
        )
        .unwrap();
    agent
        .run_turn("resume phase fixture", AgentCancellation::new())
        .await
        .unwrap();
    assert_eq!(
        agent
            .last_sampling_step_plan()
            .unwrap()
            .turn_configuration()
            .research_phase(),
        ResearchPhase::EvidenceCuration
    );
}

#[tokio::test]
async fn queued_domain_and_phase_leave_the_captured_step_immutable_until_the_next_turn() {
    let provider = MockModelProvider::new(MockModelFixture {
        capabilities: capabilities(),
        events: vec![ModelEvent::TextDelta {
            text: "queue fixture".to_owned(),
        }],
        completion: ModelResponseCompletion::without_response_id(),
    });
    let tools = ToolRegistry::for_project(Path::new(env!("CARGO_MANIFEST_DIR")));
    let mut agent = AgentLoop::new(&provider, &tools);
    agent
        .run_turn("first turn", AgentCancellation::new())
        .await
        .unwrap();
    let captured = agent.last_sampling_step_plan().unwrap().clone();
    assert_eq!(
        captured.turn_configuration().domain_profile(),
        DomainProfile::Source
    );
    assert_eq!(
        captured.turn_configuration().research_phase(),
        ResearchPhase::Mapping
    );

    agent.queue_domain_profile(DomainProfile::Mobile);
    agent.queue_research_phase(ResearchPhase::Experiment);
    assert_eq!(agent.last_sampling_step_plan(), Some(&captured));

    agent
        .run_turn("second turn", AgentCancellation::new())
        .await
        .unwrap();
    let next = agent.last_sampling_step_plan().unwrap();
    assert_eq!(
        next.turn_configuration().domain_profile(),
        DomainProfile::Mobile
    );
    assert_eq!(
        next.turn_configuration().research_phase(),
        ResearchPhase::Experiment
    );
    assert_ne!(
        captured.resolved_tool_plan().fingerprint(),
        next.resolved_tool_plan().fingerprint()
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

    let profiles = DomainProfile::ALL
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
    for profile in DomainProfile::ALL {
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
            DomainProfile::Source => {
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
            DomainProfile::Web => {
                assert_eq!(positive_fixture.sample["response"]["status"], 200);
                assert!(
                    positive_fixture.sample["response"]["body"]
                        .as_str()
                        .unwrap()
                        .contains("acme")
                );
                assert_eq!(negative_fixture.sample["response"]["status"], 403);
            }
            DomainProfile::Mobile => {
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
            DomainProfile::Binary => {
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
