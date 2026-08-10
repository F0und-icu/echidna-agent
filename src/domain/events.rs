//! Versioned, provider-neutral Event contracts and pure reduction.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// The only Event schema family understood by the initial reducer.
pub const CURRENT_SCHEMA_FAMILY: &str = "cyber-agent.event";

/// The only Event schema version understood by the initial reducer.
pub const CURRENT_SCHEMA_VERSION: u16 = 1;

macro_rules! string_newtype {
    ($name:ident, $documentation:literal) => {
        #[doc = $documentation]
        #[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Constructs the typed value from an externally supplied value.
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            /// Returns the original value without changing its representation.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }
    };
}

string_newtype!(SchemaFamily, "A stable namespace for an Event schema.");
string_newtype!(EventId, "A globally unique Event identifier.");
string_newtype!(StreamId, "An identifier for one ordered Event stream.");
string_newtype!(
    OccurredAt,
    "An externally supplied Event occurrence timestamp."
);
string_newtype!(EventType, "A semantic Event type within one schema family.");

/// A version number paired with an Event schema family.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct SchemaVersion(u16);

impl SchemaVersion {
    /// Constructs a schema version.
    pub const fn new(value: u16) -> Self {
        Self(value)
    }

    /// Returns the serialized schema version.
    pub const fn get(self) -> u16 {
        self.0
    }
}

impl fmt::Display for SchemaVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// A sequence number that is strictly increasing within one stream.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct EventSequence(u64);

impl EventSequence {
    /// Constructs an externally assigned sequence number.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the serialized sequence number.
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for EventSequence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// The append-only Event record persisted by later Event Store adapters.
///
/// `occurred_at` is supplied by the caller. Domain deliberately does not read
/// a clock while constructing or reducing an Event.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EventEnvelope<T> {
    pub schema_family: SchemaFamily,
    pub schema_version: SchemaVersion,
    pub event_id: EventId,
    pub stream_id: StreamId,
    pub sequence: EventSequence,
    pub occurred_at: OccurredAt,
    pub event_type: EventType,
    pub payload: T,
}

impl<T> EventEnvelope<T> {
    /// Constructs an envelope with explicit schema and ordering metadata.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        schema_family: SchemaFamily,
        schema_version: SchemaVersion,
        event_id: EventId,
        stream_id: StreamId,
        sequence: EventSequence,
        occurred_at: OccurredAt,
        event_type: EventType,
        payload: T,
    ) -> Self {
        Self {
            schema_family,
            schema_version,
            event_id,
            stream_id,
            sequence,
            occurred_at,
            event_type,
            payload,
        }
    }

    /// Constructs an envelope for the current schema identity path.
    pub fn current(
        event_id: EventId,
        stream_id: StreamId,
        sequence: EventSequence,
        occurred_at: OccurredAt,
        event_type: EventType,
        payload: T,
    ) -> Self {
        Self::new(
            SchemaFamily::new(CURRENT_SCHEMA_FAMILY),
            SchemaVersion::new(CURRENT_SCHEMA_VERSION),
            event_id,
            stream_id,
            sequence,
            occurred_at,
            event_type,
            payload,
        )
    }
}

/// Errors surfaced while validating Event schema or stream ordering.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum EventError {
    #[error("unsupported Event schema family `{schema_family}` version `{schema_version}`")]
    UnsupportedSchema {
        schema_family: SchemaFamily,
        schema_version: SchemaVersion,
    },
    #[error(
        "Event sequence for stream `{stream_id}` must strictly increase: previous `{previous}`, received `{received}`"
    )]
    NonIncreasingSequence {
        stream_id: StreamId,
        previous: EventSequence,
        received: EventSequence,
    },
}

/// Converts one historical Event schema to the current schema.
///
/// Upcasters must return an explicit error for unsupported input instead of
/// dropping an Event or allowing later reducers to silently ignore it.
pub trait EventUpcaster<T> {
    fn upcast(&self, envelope: EventEnvelope<T>) -> Result<EventEnvelope<T>, EventError>;
}

/// The current-schema Upcaster. Its supported path is an identity transform.
#[derive(Clone, Copy, Debug, Default)]
pub struct CurrentEventUpcaster;

impl<T> EventUpcaster<T> for CurrentEventUpcaster {
    fn upcast(&self, envelope: EventEnvelope<T>) -> Result<EventEnvelope<T>, EventError> {
        if envelope.schema_family.as_str() != CURRENT_SCHEMA_FAMILY
            || envelope.schema_version.get() != CURRENT_SCHEMA_VERSION
        {
            return Err(EventError::UnsupportedSchema {
                schema_family: envelope.schema_family,
                schema_version: envelope.schema_version,
            });
        }

        Ok(envelope)
    }
}

/// Metadata-only State reconstructed from an ordered Event stream.
///
/// It intentionally contains no business state. The tracked identifiers and
/// stream sequence values provide deterministic deduplication and validation.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct State {
    applied_event_ids: BTreeSet<EventId>,
    last_sequences: BTreeMap<StreamId, EventSequence>,
}

impl State {
    /// Returns the count of unique Events applied to this State.
    pub fn applied_event_count(&self) -> usize {
        self.applied_event_ids.len()
    }

    /// Returns the last accepted sequence for `stream_id`, if any.
    pub fn last_sequence_for(&self, stream_id: &StreamId) -> Option<EventSequence> {
        self.last_sequences.get(stream_id).copied()
    }
}

/// The result of applying one Event to a State.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApplyOutcome {
    Applied,
    DuplicateIgnored,
}

/// A pure reducer for metadata-only State.
#[derive(Clone, Copy, Debug, Default)]
pub struct EventReducer;

impl EventReducer {
    /// Reduces an ordered Event sequence into a deterministic State.
    pub fn reduce<T, Events>(&self, events: Events) -> Result<State, EventError>
    where
        Events: IntoIterator<Item = EventEnvelope<T>>,
    {
        let mut state = State::default();

        for event in events {
            self.apply(&mut state, event)?;
        }

        Ok(state)
    }

    /// Applies a new Event once, or ignores a globally duplicated Event ID.
    pub fn apply<T>(
        &self,
        state: &mut State,
        event: EventEnvelope<T>,
    ) -> Result<ApplyOutcome, EventError> {
        let event = EventUpcaster::upcast(&CurrentEventUpcaster, event)?;

        if state.applied_event_ids.contains(&event.event_id) {
            return Ok(ApplyOutcome::DuplicateIgnored);
        }

        if let Some(previous) = state.last_sequences.get(&event.stream_id)
            && event.sequence <= *previous
        {
            return Err(EventError::NonIncreasingSequence {
                stream_id: event.stream_id,
                previous: *previous,
                received: event.sequence,
            });
        }

        state.applied_event_ids.insert(event.event_id);
        state.last_sequences.insert(event.stream_id, event.sequence);

        Ok(ApplyOutcome::Applied)
    }
}
