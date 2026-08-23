//! Append-before-apply Session composition through Event ports.

use crate::domain::events::{ApplyOutcome, EventEnvelope};
use crate::domain::session::{SessionError, SessionReducer, SessionState};
use crate::ports::EventSink;

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
        .apply(&mut next_state, event.clone())
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
