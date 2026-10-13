#[path = "support/provider_config.rs"]
mod provider_config;
mod support;

use std::fs;
use std::process::Command;

use echidna::domain::agent_definition::AgentDefinitionBinding;
use echidna::domain::context::{ProviderSessionId, ProviderSessionRef, ProviderSessionRole};
use echidna::domain::domain_profile::{DomainProfile, DomainProfileSelection};
use echidna::domain::events::{EventId, EventSequence, OccurredAt, StreamId};
use echidna::domain::model::ContentItem;
use echidna::domain::research_control::{ResearchPhase, ResearchPhaseSelection};
use echidna::domain::session::{SessionEvent, SessionReducer, SessionState};
use echidna::domain::tools::{ToolCall, ToolCallId, ToolName};
use serde_json::{Value, json};
use support::TemporaryDirectory;

// Exercise the production loader without widening its API for test access.
fn binding(id: &str) -> AgentDefinitionBinding {
    let root = TemporaryDirectory::new("session-definition-binding");
    let config = root.path().join("config.toml");
    fs::write(&config, provider_config::fixture()).unwrap();
    let definition = root.path().join("writer");
    let exported = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .args(["agent", "init", definition.to_str().unwrap(), "--id", id])
        .output()
        .unwrap();
    assert!(
        exported.status.success(),
        "{}",
        String::from_utf8_lossy(&exported.stderr)
    );
    let inspected = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .args([
            "--config",
            config.to_str().unwrap(),
            "agent",
            "inspect",
            definition.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        inspected.status.success(),
        "{}",
        String::from_utf8_lossy(&inspected.stderr)
    );
    let value: Value = serde_json::from_slice(&inspected.stdout).unwrap();
    serde_json::from_value(value["binding"].clone()).unwrap()
}

fn provider(generation: u64) -> ProviderSessionRef {
    ProviderSessionRef {
        id: ProviderSessionId::new(format!("primary-{generation}")),
        role: ProviderSessionRole::Primary,
        generation,
    }
}

fn apply(
    state: &mut SessionState,
    sequence: u64,
    event: SessionEvent,
) -> Result<(), echidna::domain::session::SessionError> {
    SessionReducer
        .apply(
            state,
            event
                .into_envelope(
                    EventId::new(format!("event-{sequence}")),
                    StreamId::new("session:binding"),
                    EventSequence::new(sequence),
                    OccurredAt::new("2026-10-12T00:00:00Z"),
                )
                .unwrap(),
        )
        .map(|_| ())
}

#[test]
fn reload_atomically_binds_fresh_provider_and_preserves_history() {
    let binding = binding("writer");
    let mut state = SessionState::default();
    apply(
        &mut state,
        1,
        SessionEvent::StartedWithAgent {
            primary_provider_session: provider(1),
            binding: binding.clone(),
        },
    )
    .unwrap();
    apply(
        &mut state,
        2,
        SessionEvent::HistoryItemRecorded {
            item: ContentItem::user_text("Keep this task."),
        },
    )
    .unwrap();
    let history = state.history().clone();
    apply(
        &mut state,
        3,
        SessionEvent::AgentDefinitionApplied {
            primary_provider_session: provider(2),
            binding: binding.clone(),
        },
    )
    .unwrap();
    assert_eq!(state.history(), &history);
    assert_eq!(state.agent_definition_binding(), Some(&binding));
    assert_eq!(state.primary_provider_session(), Some(&provider(2)));

    let before = state.clone();
    assert!(
        apply(
            &mut state,
            4,
            SessionEvent::AgentDefinitionApplied {
                primary_provider_session: provider(2),
                binding
            }
        )
        .is_err()
    );
    assert_eq!(
        state, before,
        "reusing a Provider identity must not partially apply a definition"
    );
}

#[test]
fn incompatible_agent_identity_is_rejected_without_partial_state() {
    let original = binding("writer");
    let candidate = binding("reviewer");
    let mut state = SessionState::default();
    apply(
        &mut state,
        1,
        SessionEvent::StartedWithAgent {
            primary_provider_session: provider(1),
            binding: original,
        },
    )
    .unwrap();
    let before = state.clone();
    assert!(
        apply(
            &mut state,
            2,
            SessionEvent::AgentDefinitionApplied {
                primary_provider_session: provider(2),
                binding: candidate
            }
        )
        .is_err()
    );
    assert_eq!(state, before);
}

#[test]
fn generic_sessions_reject_research_events_and_unpaired_reload() {
    let binding = binding("writer");
    let mut state = SessionState::default();
    apply(
        &mut state,
        1,
        SessionEvent::StartedWithAgent {
            primary_provider_session: provider(1),
            binding: binding.clone(),
        },
    )
    .unwrap();
    let before = state.clone();
    for event in [
        SessionEvent::DomainProfileSelected {
            selection: DomainProfileSelection::operator(DomainProfile::Web),
        },
        SessionEvent::ResearchPhaseSelected {
            selection: ResearchPhaseSelection::operator(ResearchPhase::Verification),
        },
    ] {
        assert!(apply(&mut state, 2, event).is_err());
        assert_eq!(state, before);
    }
    apply(
        &mut state,
        2,
        SessionEvent::HistoryItemRecorded {
            item: ContentItem::ToolCall {
                call: ToolCall {
                    call_id: ToolCallId::new("unfinished"),
                    name: ToolName::new("read_file"),
                    arguments: json!({"path":"file.txt"}),
                },
            },
        },
    )
    .unwrap();
    let before = state.clone();
    assert!(
        apply(
            &mut state,
            3,
            SessionEvent::AgentDefinitionApplied {
                primary_provider_session: provider(2),
                binding
            }
        )
        .is_err()
    );
    assert_eq!(state, before);
}
