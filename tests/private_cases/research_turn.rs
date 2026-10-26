use crate::app::session::{AppendAfterDurableError, ResearchTurnRequest, SessionProjection};
use crate::domain::context::{ProviderSessionId, ProviderSessionRef, ProviderSessionRole};
use crate::domain::domain_profile::{DomainProfile, DomainProfileSelection};
use crate::domain::events::{EventEnvelope, EventId, EventSequence, OccurredAt, StreamId};
use crate::domain::research_control::{ResearchPhase, ResearchPhaseSelection};
use crate::domain::session::SessionEvent;
use crate::ports::EventSink;
use serde_json::Value;

#[derive(Default)]
struct Sink {
    records: Vec<EventEnvelope<Value>>,
    fail_sequence: Option<u64>,
}
impl EventSink<Value> for Sink {
    type Error = &'static str;
    fn append_event(&mut self, event: &EventEnvelope<Value>) -> Result<(), Self::Error> {
        if self.fail_sequence == Some(event.sequence.get()) {
            return Err("injected durable failure");
        }
        self.records.push(event.clone());
        Ok(())
    }
}
fn stamp(sequence: EventSequence, kind: &str) -> (EventId, OccurredAt) {
    (
        EventId::new(format!("{kind}-{}", sequence.get())),
        OccurredAt::new("2026-10-26T00:00:00Z"),
    )
}
fn projection() -> SessionProjection {
    let stream = StreamId::new("session:research-turn");
    let event = SessionEvent::Started {
        primary_provider_session: ProviderSessionRef {
            id: ProviderSessionId::new("research-turn-provider"),
            role: ProviderSessionRole::Primary,
            generation: 1,
        },
    }
    .into_envelope(
        EventId::new("started"),
        stream,
        EventSequence::new(1),
        OccurredAt::new("2026-10-26T00:00:00Z"),
    )
    .unwrap();
    SessionProjection::replay([event]).unwrap()
}
fn request() -> ResearchTurnRequest {
    ResearchTurnRequest {
        started: true,
        default_domain: DomainProfile::Web,
        default_phase: ResearchPhase::Verification,
        domain: None,
        phase: None,
    }
}

#[test]
fn app_research_boundary_persists_domain_then_phase_and_reuses_existing_selections() {
    let mut projection = projection();
    let mut sink = Sink::default();
    let selected = projection
        .prepare_research_turn(
            &mut sink,
            StreamId::new("session:research-turn"),
            request(),
            stamp,
        )
        .unwrap();
    assert_eq!(selected, (DomainProfile::Web, ResearchPhase::Verification));
    assert_eq!(sink.records.len(), 2);
    assert_eq!(
        sink.records[0].event_type.as_str(),
        "session.domain_profile_selected"
    );
    assert_eq!(
        sink.records[1].event_type.as_str(),
        "session.research_phase_selected"
    );
    let mut different_defaults = request();
    different_defaults.default_domain = DomainProfile::Binary;
    different_defaults.default_phase = ResearchPhase::Mapping;
    assert_eq!(
        projection
            .prepare_research_turn(
                &mut sink,
                StreamId::new("session:research-turn"),
                different_defaults,
                |_, _| panic!("existing selections must not append another event")
            )
            .unwrap(),
        selected
    );
    assert_eq!(sink.records.len(), 2);
}

#[test]
fn app_research_boundary_keeps_only_the_real_durable_prefix_when_phase_append_fails() {
    let mut projection = projection();
    let mut sink = Sink {
        fail_sequence: Some(3),
        ..Sink::default()
    };
    assert!(matches!(
        projection.prepare_research_turn(
            &mut sink,
            StreamId::new("session:research-turn"),
            request(),
            stamp
        ),
        Err(AppendAfterDurableError::Sink("injected durable failure"))
    ));
    assert_eq!(sink.records.len(), 1);
    assert_eq!(
        projection
            .state()
            .domain_profile_selection()
            .unwrap()
            .profile(),
        DomainProfile::Web
    );
    assert!(projection.state().research_phase_selection().is_none());
    assert_eq!(projection.next_event_sequence().unwrap().get(), 3);
}

#[test]
fn app_research_boundary_preserves_operator_selection_provenance() {
    let mut projection = projection();
    let mut sink = Sink::default();
    let domain = DomainProfileSelection::operator(DomainProfile::Binary);
    let phase = ResearchPhaseSelection::operator(ResearchPhase::Experiment);
    let mut input = request();
    input.domain = Some(domain);
    input.phase = Some(phase);
    assert_eq!(
        projection
            .prepare_research_turn(
                &mut sink,
                StreamId::new("session:research-turn"),
                input,
                stamp
            )
            .unwrap(),
        (DomainProfile::Binary, ResearchPhase::Experiment)
    );
    assert_eq!(projection.state().domain_profile_selection(), Some(domain));
    assert_eq!(projection.state().research_phase_selection(), Some(phase));
}
