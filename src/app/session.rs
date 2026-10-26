//! Append-before-apply Session composition through Event ports.

use crate::domain::events::{ApplyOutcome, EventEnvelope};
use crate::domain::session::{SessionError, SessionReducer, SessionState};
use crate::ports::EventSink;

/// Explicit turn-boundary requests; defaults come from the validated binding.
pub(crate) struct ResearchTurnRequest {
    pub(crate) started: bool,
    pub(crate) default_domain: crate::domain::domain_profile::DomainProfile,
    pub(crate) default_phase: crate::domain::research_control::ResearchPhase,
    pub(crate) domain: Option<crate::domain::domain_profile::DomainProfileSelection>,
    pub(crate) phase: Option<crate::domain::research_control::ResearchPhaseSelection>,
}

/// Validates one prospective state, appends its Event durably, then promotes
/// that state. A failed append never mutates the caller's `SessionState`.
fn append_after_durable<S>(
    reducer: &SessionReducer,
    state: &mut SessionState,
    sink: &mut S,
    event: EventEnvelope<serde_json::Value>,
) -> Result<ApplyOutcome, AppendAfterDurableError<S::Error>>
where
    S: EventSink<serde_json::Value>,
{
    let mut next_state = state.clone();
    let outcome = reducer
        .apply_in_place(&mut next_state, event.clone())
        .map_err(AppendAfterDurableError::Session)?;
    if outcome == ApplyOutcome::DuplicateIgnored {
        return Ok(outcome);
    }
    sink.append_event(&event)
        .map_err(AppendAfterDurableError::Sink)?;
    *state = next_state;
    Ok(outcome)
}

/// The app-level projection of one Session Event stream. The composition root
/// reads Events from an `EventSource`, assigns stream sequences and supplies
/// its durable `EventSink`; this service does not know the JSONL layout.
pub struct SessionProjection {
    reducer: SessionReducer,
    state: SessionState,
}

impl SessionProjection {
    /// Resolve and durably apply the original domain-then-phase boundary from
    /// one synchronized projection. UI supplies identity/time, never policy.
    pub(crate) fn prepare_research_turn<S: EventSink<serde_json::Value>>(
        &mut self,
        sink: &mut S,
        stream: crate::domain::events::StreamId,
        request: ResearchTurnRequest,
        mut stamp: impl FnMut(
            crate::domain::events::EventSequence,
            &'static str,
        ) -> (
            crate::domain::events::EventId,
            crate::domain::events::OccurredAt,
        ),
    ) -> Result<
        (
            crate::domain::domain_profile::DomainProfile,
            crate::domain::research_control::ResearchPhase,
        ),
        AppendAfterDurableError<S::Error>,
    > {
        use crate::domain::domain_profile::{DomainProfile, DomainProfileSelection};
        use crate::domain::research_control::{ResearchPhase, ResearchPhaseSelection};
        use crate::domain::session::SessionEvent;
        let (domain, domain_change) = match request.domain {
            Some(selection) => (selection.profile(), Some(selection)),
            None => match self.state.domain_profile_selection() {
                Some(selection) => (selection.profile(), None),
                None => {
                    let selection =
                        if request.started || request.default_domain != DomainProfile::Source {
                            DomainProfileSelection::definition_default(request.default_domain)
                        } else {
                            DomainProfileSelection::legacy_default()
                        };
                    (selection.profile(), Some(selection))
                }
            },
        };
        let (phase, phase_change) = match request.phase {
            Some(selection) => (selection.phase(), Some(selection)),
            None => match self.state.research_phase_selection() {
                Some(selection) => (selection.phase(), None),
                None => {
                    let selection =
                        if request.started || request.default_phase != ResearchPhase::Mapping {
                            ResearchPhaseSelection::definition_default(request.default_phase)
                        } else {
                            ResearchPhaseSelection::legacy_default()
                        };
                    (selection.phase(), Some(selection))
                }
            },
        };
        for (event, kind) in [
            domain_change.map(|selection| {
                (
                    SessionEvent::DomainProfileSelected { selection },
                    "domain-profile",
                )
            }),
            phase_change.map(|selection| {
                (
                    SessionEvent::ResearchPhaseSelected { selection },
                    "research-phase",
                )
            }),
        ]
        .into_iter()
        .flatten()
        {
            let sequence = self
                .next_event_sequence()
                .map_err(AppendAfterDurableError::Session)?;
            let (id, at) = stamp(sequence, kind);
            let event = event
                .into_envelope(id, stream.clone(), sequence, at)
                .map_err(AppendAfterDurableError::Session)?;
            self.append(sink, event)?;
        }
        Ok((domain, phase))
    }

    /// Strictly reconstructs Session State from durable Events supplied by an
    /// EventSource at the composition boundary.
    pub fn replay<Events>(events: Events) -> Result<Self, SessionError>
    where
        Events: IntoIterator<Item = EventEnvelope<serde_json::Value>>,
    {
        let reducer = SessionReducer;
        let state = reducer.reduce(events)?;
        Ok(Self { reducer, state })
    }

    pub fn state(&self) -> &SessionState {
        &self.state
    }

    /// Incorporates an already durable, strictly ordered suffix through the
    /// same transition owner. Any invalid suffix leaves this projection intact.
    pub(crate) fn ingest_durable(
        &mut self,
        events: Vec<EventEnvelope<serde_json::Value>>,
    ) -> Result<(), SessionError> {
        if events.is_empty() {
            return Ok(());
        }
        let mut next = self.state.clone();
        for event in events {
            self.reducer.apply_in_place(&mut next, event)?;
        }
        self.state = next;
        Ok(())
    }

    pub(crate) fn next_event_sequence(
        &self,
    ) -> Result<crate::domain::events::EventSequence, SessionError> {
        self.state.next_event_sequence()
    }

    /// Persists the supplied complete Event through the caller's EventSink.
    /// State promotion remains conditional on a successful durable append.
    pub fn append<S>(
        &mut self,
        sink: &mut S,
        event: EventEnvelope<serde_json::Value>,
    ) -> Result<ApplyOutcome, AppendAfterDurableError<S::Error>>
    where
        S: EventSink<serde_json::Value>,
    {
        append_after_durable(&self.reducer, &mut self.state, sink, event)
    }
}

/// A Session append failed either before durable persistence or while
/// persisting the Event. It intentionally retains the Sink's typed error.
#[derive(Debug)]
pub enum AppendAfterDurableError<E> {
    Session(SessionError),
    Sink(E),
}
