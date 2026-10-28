use crate::domain::context::{
    BaselineInvalidationReason, ModelIdentity, ModelProfile, ProviderSession, ProviderSessionId,
    ProviderSessionRef, ProviderSessionRole,
};
use crate::domain::events::{EventEnvelope, EventId, EventSequence, OccurredAt, StreamId};
use crate::domain::model::{
    CapabilityMappingVersion, ContentItem, ModelCapabilities, ModelInputIR,
    ModelRequestRequirements,
};
use crate::domain::project::ProjectId;
use crate::domain::request_audit::{
    PublishedAuditBlob, RequestAdapterVersion, RequestAuditAttempt, RequestAuditBinding,
    RequestAuditDelivery, RequestAuditRunContext,
};
use crate::domain::session::{SessionEvent, SessionReducer, SessionState};
use serde_json::Value;
use sha2::{Digest, Sha256};

const STREAM: &str = "session:model-delivery";

fn envelope(event: SessionEvent, sequence: u64) -> EventEnvelope<Value> {
    event
        .into_envelope(
            EventId::new(format!("event-{sequence}")),
            StreamId::new(STREAM),
            EventSequence::new(sequence),
            OccurredAt::new("2026-10-28T00:00:00Z"),
        )
        .unwrap()
}

fn provider() -> ProviderSessionRef {
    ProviderSession::new(
        ProviderSessionId::new("primary"),
        ProviderSessionRole::Primary,
        0,
    )
    .reference()
    .clone()
}

fn initial() -> (SessionState, Vec<EventEnvelope<Value>>) {
    let events = vec![
        envelope(
            SessionEvent::Started {
                primary_provider_session: provider(),
            },
            1,
        ),
        envelope(
            SessionEvent::HistoryItemRecorded {
                item: ContentItem::user_text("input"),
            },
            2,
        ),
    ];
    (SessionReducer.reduce(events.clone()).unwrap(), events)
}

fn audit(id: &str, full: bool) -> crate::domain::request_audit::RequestAuditManifest {
    let caps = ModelCapabilities {
        text_input: true,
        image_input: false,
        streaming: true,
        tool_call: true,
        structured_output: true,
        incremental_input: true,
        usage_reporting: false,
        max_output_tokens: None,
        reasoning_effort: None,
        context_window: 32768,
        capability_mapping_version: CapabilityMappingVersion::new(1),
    };
    let context = RequestAuditRunContext::new(
        RequestAuditBinding::new(ProjectId::new("lineage"), StreamId::new(STREAM)).unwrap(),
        RequestAdapterVersion::new("fixture").unwrap(),
    );
    let input = ModelInputIR::current(
        "instruction",
        vec![ContentItem::user_text("input")],
        ModelRequestRequirements::default(),
    );
    let delivery = if full {
        RequestAuditDelivery::full(BaselineInvalidationReason::NoBaseline)
    } else {
        RequestAuditDelivery::incremental_append_only()
    };
    let rendered = RequestAuditAttempt::new(
        &context,
        ModelIdentity::new("fixture", "fixture"),
        ModelProfile::new("fixture", 1),
        caps,
        input.clone(),
        input,
        delivery,
    )
    .unwrap()
    .bind_provider_session(provider())
    .render()
    .unwrap();
    let blobs = rendered
        .component_bytes()
        .map(|bytes| PublishedAuditBlob {
            blob_id: crate::domain::artifacts::BlobId::new(format!("{:x}", Sha256::digest(bytes)))
                .unwrap(),
            size_bytes: bytes.len() as u64,
        })
        .collect::<Vec<_>>();
    rendered.materialize(EventId::new(id), blobs).unwrap()
}

fn append(
    state: &mut SessionState,
    events: &mut Vec<EventEnvelope<Value>>,
    event: SessionEvent,
    id: Option<&str>,
) {
    let sequence = events.len() as u64 + 1;
    let mut entry = envelope(event, sequence);
    if let Some(id) = id {
        entry.event_id = EventId::new(id);
    }
    SessionReducer.apply(state, entry.clone()).unwrap();
    events.push(entry);
}

#[test]
fn only_committed_samples_form_the_exact_full_incremental_chain() {
    let (mut state, mut events) = initial();
    append(
        &mut state,
        &mut events,
        SessionEvent::RequestAuditRecorded {
            manifest: audit("full", true),
        },
        Some("full"),
    );
    assert!(
        state
            .new_model_delivery(&EventId::new("full"), None)
            .is_err(),
        "audit alone is not a successful sample"
    );
    let mut empty_state = state.clone();
    let mut empty_events = events.clone();
    append(
        &mut empty_state,
        &mut empty_events,
        SessionEvent::HistoryItemRecorded {
            item: ContentItem::assistant_text(""),
        },
        None,
    );
    assert!(
        empty_state
            .new_model_delivery(&EventId::new("full"), None)
            .is_err()
    );
    append(
        &mut state,
        &mut events,
        SessionEvent::HistoryItemRecorded {
            item: ContentItem::assistant_text("first answer"),
        },
        None,
    );
    let first = state
        .new_model_delivery(&EventId::new("full"), None)
        .unwrap();
    append(
        &mut state,
        &mut events,
        SessionEvent::ModelDeliveryCommitted {
            delivery: first.clone(),
        },
        None,
    );
    assert!(
        state
            .new_model_delivery(&EventId::new("full"), None)
            .is_err()
    );
    append(
        &mut state,
        &mut events,
        SessionEvent::RequestAuditRecorded {
            manifest: audit("failed-attempt", false),
        },
        Some("failed-attempt"),
    );
    append(
        &mut state,
        &mut events,
        SessionEvent::ModelSampleRejected {
            request_audit_event_id: EventId::new("failed-attempt"),
            diagnostic: crate::domain::model::ToolArgumentDiagnostic {
                category: crate::domain::model::ToolArgumentErrorCategory::Syntax,
                line: 1,
                column: 1,
                arguments_bytes: 1,
            },
            corrections_used: 0,
        },
        None,
    );
    append(
        &mut state,
        &mut events,
        SessionEvent::RequestAuditRecorded {
            manifest: audit("incremental", false),
        },
        Some("incremental"),
    );
    append(
        &mut state,
        &mut events,
        SessionEvent::HistoryItemRecorded {
            item: ContentItem::assistant_text("second answer"),
        },
        None,
    );
    assert!(
        state
            .new_model_delivery(&EventId::new("incremental"), None)
            .is_err()
    );
    assert!(
        state
            .new_model_delivery(
                &EventId::new("incremental"),
                Some(EventId::new("failed-attempt"))
            )
            .is_err()
    );
    let second = state
        .new_model_delivery(&EventId::new("incremental"), Some(EventId::new("full")))
        .unwrap();
    let mut forged = serde_json::to_value(&second).unwrap();
    forged["full_logical_hash"] = "a".repeat(64).into();
    let forged = serde_json::from_value(forged).unwrap();
    assert!(
        SessionReducer
            .apply(
                &mut state,
                envelope(
                    SessionEvent::ModelDeliveryCommitted { delivery: forged },
                    events.len() as u64 + 1
                )
            )
            .is_err()
    );
    let mut wrong_cutoff = serde_json::to_value(&second).unwrap();
    wrong_cutoff["history_cutoff_event_id"] = "event-2".into();
    assert!(
        SessionReducer
            .apply(
                &mut state,
                envelope(
                    SessionEvent::ModelDeliveryCommitted {
                        delivery: serde_json::from_value(wrong_cutoff).unwrap()
                    },
                    events.len() as u64 + 1
                )
            )
            .is_err()
    );
    let mut wrong_schema = serde_json::to_value(&second).unwrap();
    wrong_schema["schema_version"] = 2.into();
    assert!(
        SessionReducer
            .apply(
                &mut state,
                envelope(
                    SessionEvent::ModelDeliveryCommitted {
                        delivery: serde_json::from_value(wrong_schema).unwrap()
                    },
                    events.len() as u64 + 1
                )
            )
            .is_err()
    );
    append(
        &mut state,
        &mut events,
        SessionEvent::ModelDeliveryCommitted { delivery: second },
        None,
    );
    append(
        &mut state,
        &mut events,
        SessionEvent::RequestAuditRecorded {
            manifest: audit("replacement-full", true),
        },
        Some("replacement-full"),
    );
    append(
        &mut state,
        &mut events,
        SessionEvent::HistoryItemRecorded {
            item: ContentItem::assistant_text("third answer"),
        },
        None,
    );
    assert!(
        state
            .new_model_delivery(
                &EventId::new("replacement-full"),
                Some(EventId::new("incremental"))
            )
            .is_err()
    );
    let replacement = state
        .new_model_delivery(&EventId::new("replacement-full"), None)
        .unwrap();
    append(
        &mut state,
        &mut events,
        SessionEvent::ModelDeliveryCommitted {
            delivery: replacement,
        },
        None,
    );
    let replayed = SessionReducer.reduce(events).unwrap();
    let chain = replayed
        .model_delivery_chain(&EventId::new("incremental"))
        .unwrap()
        .unwrap();
    assert_eq!(
        chain
            .iter()
            .map(|item| item.audit_event_id().as_str())
            .collect::<Vec<_>>(),
        ["full", "incremental"]
    );
    assert!(
        replayed
            .model_delivery_chain(&EventId::new("failed-attempt"))
            .unwrap()
            .is_none()
    );
    assert_eq!(
        replayed
            .model_delivery_chain(&EventId::new("replacement-full"))
            .unwrap()
            .unwrap()
            .len(),
        1
    );
}
