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

/// One concrete Event schema identity.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EventSchema {
    pub family: SchemaFamily,
    pub version: SchemaVersion,
}

impl EventSchema {
    /// Constructs a schema identity from explicit family and version values.
    pub fn new(family: SchemaFamily, version: SchemaVersion) -> Self {
        Self { family, version }
    }

    /// Returns the current published Event schema identity.
    pub fn current() -> Self {
        Self::new(
            SchemaFamily::new(CURRENT_SCHEMA_FAMILY),
            SchemaVersion::new(CURRENT_SCHEMA_VERSION),
        )
    }

    /// Extracts the schema identity from an Event envelope.
    pub fn from_envelope<T>(envelope: &EventEnvelope<T>) -> Self {
        Self::new(envelope.schema_family.clone(), envelope.schema_version)
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
    #[error(
        "Event sequence for stream `{stream_id}` must be the next value: expected `{expected}`, received `{received}`"
    )]
    UnexpectedSequence {
        stream_id: StreamId,
        expected: EventSequence,
        received: EventSequence,
    },
    #[error("Event sequence for stream `{stream_id}` cannot advance beyond `{last}`")]
    SequenceOverflow {
        stream_id: StreamId,
        last: EventSequence,
    },
    #[error(
        "missing Event Upcaster for schema family `{schema_family}` from version `{from_version}` toward `{target_version}`"
    )]
    MissingUpcaster {
        schema_family: SchemaFamily,
        from_version: SchemaVersion,
        target_version: SchemaVersion,
    },
    #[error(
        "Event Upcaster must advance exactly one version, received `{from_version}` to `{to_version}`"
    )]
    InvalidUpcasterStep {
        from_version: SchemaVersion,
        to_version: SchemaVersion,
    },
    #[error(
        "an Event Upcaster is already registered for schema family `{schema_family}` version `{from_version}`"
    )]
    DuplicateUpcaster {
        schema_family: SchemaFamily,
        from_version: SchemaVersion,
    },
    #[error(
        "Event Upcaster output schema does not match its declared target: expected `{expected_schema_family}` version `{expected_schema_version}`, received `{actual_schema_family}` version `{actual_schema_version}`"
    )]
    InvalidUpcasterOutput {
        expected_schema_family: SchemaFamily,
        expected_schema_version: SchemaVersion,
        actual_schema_family: SchemaFamily,
        actual_schema_version: SchemaVersion,
    },
    #[error("Event Upcaster changed immutable Event identity field `{field:?}`")]
    UpcasterIdentityChanged { field: EventIdentityField },
}

/// One immutable Event-envelope field that an Upcaster must preserve.
///
/// `event_type` is deliberately absent: changing it is a valid schema
/// migration, while the event identity and ordering coordinates are not.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventIdentityField {
    EventId,
    StreamId,
    Sequence,
    OccurredAt,
}

/// Converts one historical Event schema to the current schema.
///
/// Upcasters must return an explicit error for unsupported input instead of
/// dropping an Event or allowing later reducers to silently ignore it.
pub trait EventUpcaster<T> {
    fn upcast(&self, envelope: EventEnvelope<T>) -> Result<EventEnvelope<T>, EventError>;
}

/// A pure, one-step Event schema transformation used by a chained registry.
///
/// Every registered transform must advance exactly from version `N` to
/// version `N + 1`. It has no project state, clock, or I/O access.
pub trait VersionedEventUpcaster<T> {
    /// The schema family accepted and emitted by this transformation.
    fn schema_family(&self) -> &SchemaFamily;

    /// The version accepted by this transformation.
    fn source_version(&self) -> SchemaVersion;

    /// The version emitted by this transformation.
    fn to_version(&self) -> SchemaVersion;

    /// Transforms one envelope without mutating its persisted source bytes.
    fn upcast(&self, envelope: EventEnvelope<T>) -> Result<EventEnvelope<T>, EventError>;
}

/// The sole pure owner of ordered, version-by-version Event Upcasting.
///
/// The registry never rewrites JSONL. File adapters use it only while reading
/// raw records into the current in-memory representation.
pub struct EventUpcasterRegistry<T> {
    current_schema: EventSchema,
    upcasters: BTreeMap<SchemaVersion, Box<dyn VersionedEventUpcaster<T>>>,
}

impl<T> EventUpcasterRegistry<T> {
    /// Creates an empty registry for one target schema.
    pub fn new(current_schema: EventSchema) -> Self {
        Self {
            current_schema,
            upcasters: BTreeMap::new(),
        }
    }

    /// Returns the in-memory schema produced by this registry.
    pub fn current_schema(&self) -> &EventSchema {
        &self.current_schema
    }

    /// Registers one exact `vN -> vN+1` transform.
    pub fn register<U>(&mut self, upcaster: U) -> Result<(), EventError>
    where
        U: VersionedEventUpcaster<T> + 'static,
    {
        let schema_family = upcaster.schema_family().clone();
        let from_version = upcaster.source_version();
        let to_version = upcaster.to_version();

        if schema_family != self.current_schema.family {
            return Err(EventError::UnsupportedSchema {
                schema_family,
                schema_version: from_version,
            });
        }

        if from_version
            .get()
            .checked_add(1)
            .is_none_or(|expected| to_version.get() != expected)
            || to_version > self.current_schema.version
        {
            return Err(EventError::InvalidUpcasterStep {
                from_version,
                to_version,
            });
        }

        if self.upcasters.contains_key(&from_version) {
            return Err(EventError::DuplicateUpcaster {
                schema_family,
                from_version,
            });
        }

        self.upcasters.insert(from_version, Box::new(upcaster));
        Ok(())
    }

    /// Converts one raw historical envelope through every required link.
    pub fn upcast(&self, mut envelope: EventEnvelope<T>) -> Result<EventEnvelope<T>, EventError> {
        let input_schema = EventSchema::from_envelope(&envelope);
        if input_schema.family != self.current_schema.family
            || input_schema.version > self.current_schema.version
        {
            return Err(EventError::UnsupportedSchema {
                schema_family: input_schema.family,
                schema_version: input_schema.version,
            });
        }

        while envelope.schema_version != self.current_schema.version {
            let from_version = envelope.schema_version;
            let upcaster =
                self.upcasters
                    .get(&from_version)
                    .ok_or_else(|| EventError::MissingUpcaster {
                        schema_family: self.current_schema.family.clone(),
                        from_version,
                        target_version: self.current_schema.version,
                    })?;
            let expected_version = upcaster.to_version();
            let expected_family = upcaster.schema_family().clone();
            let event_id = envelope.event_id.clone();
            let stream_id = envelope.stream_id.clone();
            let sequence = envelope.sequence;
            let occurred_at = envelope.occurred_at.clone();
            let next = upcaster.upcast(envelope)?;

            if next.schema_family != expected_family || next.schema_version != expected_version {
                return Err(EventError::InvalidUpcasterOutput {
                    expected_schema_family: expected_family,
                    expected_schema_version: expected_version,
                    actual_schema_family: next.schema_family,
                    actual_schema_version: next.schema_version,
                });
            }

            if next.event_id != event_id {
                return Err(EventError::UpcasterIdentityChanged {
                    field: EventIdentityField::EventId,
                });
            }
            if next.stream_id != stream_id {
                return Err(EventError::UpcasterIdentityChanged {
                    field: EventIdentityField::StreamId,
                });
            }
            if next.sequence != sequence {
                return Err(EventError::UpcasterIdentityChanged {
                    field: EventIdentityField::Sequence,
                });
            }
            if next.occurred_at != occurred_at {
                return Err(EventError::UpcasterIdentityChanged {
                    field: EventIdentityField::OccurredAt,
                });
            }

            envelope = next;
        }

        Ok(envelope)
    }
}

impl<T> EventUpcasterRegistry<T> {
    /// Creates the identity-only registry for the current published schema.
    pub fn current() -> Self {
        Self::new(EventSchema::current())
    }
}

/// The current-schema Upcaster. Its supported path is an identity transform.
#[derive(Clone, Copy, Debug, Default)]
pub struct CurrentEventUpcaster;

impl<T> EventUpcaster<T> for CurrentEventUpcaster {
    fn upcast(&self, envelope: EventEnvelope<T>) -> Result<EventEnvelope<T>, EventError> {
        EventUpcasterRegistry::current().upcast(envelope)
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

    /// Whether this exact Event identity has already been reduced. App
    /// services use this narrow query to reject caller-supplied identity reuse
    /// before performing irreversible side effects.
    pub(crate) fn contains_event_id(&self, event_id: &EventId) -> bool {
        self.applied_event_ids.contains(event_id)
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
