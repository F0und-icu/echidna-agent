use cyber_agent::domain::events::{
    EventEnvelope, EventError, EventId, EventIdentityField, EventSchema, EventSequence, EventType,
    EventUpcasterRegistry, OccurredAt, SchemaFamily, SchemaVersion, StreamId,
    VersionedEventUpcaster,
};
use serde_json::{Value, json};

const FIXTURE_FAMILY: &str = "fixture.event";

struct FixtureUpcaster {
    from: u16,
    to: u16,
    marker: &'static str,
    family: SchemaFamily,
}

impl FixtureUpcaster {
    fn new(from: u16, to: u16, marker: &'static str) -> Self {
        Self {
            from,
            to,
            marker,
            family: SchemaFamily::new(FIXTURE_FAMILY),
        }
    }
}

impl VersionedEventUpcaster<Value> for FixtureUpcaster {
    fn schema_family(&self) -> &SchemaFamily {
        &self.family
    }

    fn source_version(&self) -> SchemaVersion {
        SchemaVersion::new(self.from)
    }

    fn to_version(&self) -> SchemaVersion {
        SchemaVersion::new(self.to)
    }

    fn upcast(
        &self,
        mut envelope: EventEnvelope<Value>,
    ) -> Result<EventEnvelope<Value>, EventError> {
        envelope.schema_version = SchemaVersion::new(self.to);
        envelope.event_type = EventType::new(format!("fixture.v{}", self.to));
        envelope.payload[self.marker] = json!(true);
        Ok(envelope)
    }
}

fn fixture_event() -> EventEnvelope<Value> {
    serde_json::from_str(include_str!("fixtures/events/historical-v1.json"))
        .expect("historical fixture should decode")
}

fn fixture_registry() -> EventUpcasterRegistry<Value> {
    let mut registry = EventUpcasterRegistry::new(EventSchema::new(
        SchemaFamily::new(FIXTURE_FAMILY),
        SchemaVersion::new(3),
    ));
    registry
        .register(FixtureUpcaster::new(1, 2, "upcast_v2"))
        .unwrap();
    registry
        .register(FixtureUpcaster::new(2, 3, "upcast_v3"))
        .unwrap();
    registry
}

#[test]
fn fixed_historical_fixture_traverses_every_registered_upcaster_link() {
    let event = fixture_registry().upcast(fixture_event()).unwrap();

    assert_eq!(event.schema_version, SchemaVersion::new(3));
    assert_eq!(event.payload["historical"], json!(true));
    assert_eq!(event.payload["upcast_v2"], json!(true));
    assert_eq!(event.payload["upcast_v3"], json!(true));
    assert_eq!(event.event_type, EventType::new("fixture.v3"));
}

#[test]
fn missing_historical_upcaster_link_is_an_explicit_failure() {
    let mut registry = EventUpcasterRegistry::new(EventSchema::new(
        SchemaFamily::new(FIXTURE_FAMILY),
        SchemaVersion::new(3),
    ));
    registry
        .register(FixtureUpcaster::new(1, 2, "upcast_v2"))
        .unwrap();

    assert_eq!(
        registry.upcast(fixture_event()),
        Err(EventError::MissingUpcaster {
            schema_family: SchemaFamily::new(FIXTURE_FAMILY),
            from_version: SchemaVersion::new(2),
            target_version: SchemaVersion::new(3),
        })
    );
}

#[test]
fn registry_rejects_a_non_contiguous_upcaster_step() {
    let mut registry = EventUpcasterRegistry::new(EventSchema::new(
        SchemaFamily::new(FIXTURE_FAMILY),
        SchemaVersion::new(3),
    ));

    assert_eq!(
        registry.register(FixtureUpcaster::new(1, 3, "invalid")),
        Err(EventError::InvalidUpcasterStep {
            from_version: SchemaVersion::new(1),
            to_version: SchemaVersion::new(3),
        })
    );
}

#[derive(Clone, Copy)]
enum IdentityMutation {
    EventId,
    StreamId,
    Sequence,
    OccurredAt,
}

struct MalformedUpcaster {
    declared_family: SchemaFamily,
    source: SchemaVersion,
    target: SchemaVersion,
    output_family: SchemaFamily,
    output_version: SchemaVersion,
    identity_mutation: Option<IdentityMutation>,
}

impl MalformedUpcaster {
    fn valid() -> Self {
        Self {
            declared_family: SchemaFamily::new(FIXTURE_FAMILY),
            source: SchemaVersion::new(1),
            target: SchemaVersion::new(2),
            output_family: SchemaFamily::new(FIXTURE_FAMILY),
            output_version: SchemaVersion::new(2),
            identity_mutation: None,
        }
    }
}

impl VersionedEventUpcaster<Value> for MalformedUpcaster {
    fn schema_family(&self) -> &SchemaFamily {
        &self.declared_family
    }

    fn source_version(&self) -> SchemaVersion {
        self.source
    }

    fn to_version(&self) -> SchemaVersion {
        self.target
    }

    fn upcast(
        &self,
        mut envelope: EventEnvelope<Value>,
    ) -> Result<EventEnvelope<Value>, EventError> {
        envelope.schema_family = self.output_family.clone();
        envelope.schema_version = self.output_version;
        match self.identity_mutation {
            Some(IdentityMutation::EventId) => envelope.event_id = EventId::new("changed-event"),
            Some(IdentityMutation::StreamId) => {
                envelope.stream_id = StreamId::new("changed-stream")
            }
            Some(IdentityMutation::Sequence) => envelope.sequence = EventSequence::new(99),
            Some(IdentityMutation::OccurredAt) => {
                envelope.occurred_at = OccurredAt::new("2026-08-23T00:00:00Z")
            }
            None => {}
        }
        Ok(envelope)
    }
}

fn v2_registry() -> EventUpcasterRegistry<Value> {
    EventUpcasterRegistry::new(EventSchema::new(
        SchemaFamily::new(FIXTURE_FAMILY),
        SchemaVersion::new(2),
    ))
}

#[test]
fn registry_rejects_duplicate_and_wrong_family_registrations() {
    let mut duplicate_registry = v2_registry();
    duplicate_registry
        .register(FixtureUpcaster::new(1, 2, "first"))
        .unwrap();
    assert_eq!(
        duplicate_registry.register(FixtureUpcaster::new(1, 2, "second")),
        Err(EventError::DuplicateUpcaster {
            schema_family: SchemaFamily::new(FIXTURE_FAMILY),
            from_version: SchemaVersion::new(1),
        })
    );

    let mut wrong_family_registry = v2_registry();
    let mut wrong_family = MalformedUpcaster::valid();
    wrong_family.declared_family = SchemaFamily::new("wrong.family");
    assert_eq!(
        wrong_family_registry.register(wrong_family),
        Err(EventError::UnsupportedSchema {
            schema_family: SchemaFamily::new("wrong.family"),
            schema_version: SchemaVersion::new(1),
        })
    );
}

#[test]
fn registry_rejects_wrong_output_family_and_version() {
    let mut wrong_family_registry = v2_registry();
    let mut wrong_family = MalformedUpcaster::valid();
    wrong_family.output_family = SchemaFamily::new("wrong.family");
    wrong_family_registry.register(wrong_family).unwrap();
    assert!(matches!(
        wrong_family_registry.upcast(fixture_event()),
        Err(EventError::InvalidUpcasterOutput {
            actual_schema_family,
            actual_schema_version,
            ..
        }) if actual_schema_family == SchemaFamily::new("wrong.family")
            && actual_schema_version == SchemaVersion::new(2)
    ));

    let mut wrong_version_registry = v2_registry();
    let mut wrong_version = MalformedUpcaster::valid();
    wrong_version.output_version = SchemaVersion::new(1);
    wrong_version_registry.register(wrong_version).unwrap();
    assert!(matches!(
        wrong_version_registry.upcast(fixture_event()),
        Err(EventError::InvalidUpcasterOutput {
            actual_schema_family,
            actual_schema_version,
            ..
        }) if actual_schema_family == SchemaFamily::new(FIXTURE_FAMILY)
            && actual_schema_version == SchemaVersion::new(1)
    ));
}

#[test]
fn registry_rejects_each_immutable_envelope_identity_mutation() {
    for (mutation, field) in [
        (IdentityMutation::EventId, EventIdentityField::EventId),
        (IdentityMutation::StreamId, EventIdentityField::StreamId),
        (IdentityMutation::Sequence, EventIdentityField::Sequence),
        (IdentityMutation::OccurredAt, EventIdentityField::OccurredAt),
    ] {
        let mut registry = v2_registry();
        let mut upcaster = MalformedUpcaster::valid();
        upcaster.identity_mutation = Some(mutation);
        registry.register(upcaster).unwrap();
        assert_eq!(
            registry.upcast(fixture_event()),
            Err(EventError::UpcasterIdentityChanged { field })
        );
    }
}

#[test]
fn registry_rejects_a_u16_schema_version_overflow_step() {
    let mut registry = EventUpcasterRegistry::new(EventSchema::new(
        SchemaFamily::new(FIXTURE_FAMILY),
        SchemaVersion::new(u16::MAX),
    ));

    assert_eq!(
        registry.register(FixtureUpcaster::new(u16::MAX, u16::MAX, "overflow")),
        Err(EventError::InvalidUpcasterStep {
            from_version: SchemaVersion::new(u16::MAX),
            to_version: SchemaVersion::new(u16::MAX),
        })
    );
}
