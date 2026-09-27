use echidna::domain::events::{
    ApplyOutcome, CURRENT_SCHEMA_FAMILY, CURRENT_SCHEMA_VERSION, CurrentEventUpcaster,
    EventEnvelope, EventError, EventId, EventReducer, EventSequence, EventType, EventUpcaster,
    OccurredAt, SchemaFamily, SchemaVersion, State, StreamId,
};
use serde_json::json;

fn event<'a>(
    event_id: &str,
    stream_id: &str,
    sequence: u64,
    payload: &'a str,
) -> EventEnvelope<&'a str> {
    EventEnvelope::current(
        EventId::new(event_id),
        StreamId::new(stream_id),
        EventSequence::new(sequence),
        OccurredAt::new("2026-08-09T00:00:00Z"),
        EventType::new("test.event"),
        payload,
    )
}

#[test]
fn identity_upcaster_preserves_the_current_event() {
    let input = event("event-1", "session-1", 1, "payload");
    let output = CurrentEventUpcaster.upcast(input.clone());

    assert_eq!(output, Ok(input));
}

#[test]
fn current_event_json_has_stable_field_names_and_transparent_typed_values() {
    let serialized = serde_json::to_value(event("event-1", "session-1", 1, "payload"))
        .expect("current Event envelope should serialize");

    assert_eq!(
        serialized,
        json!({
            "schema_family": CURRENT_SCHEMA_FAMILY,
            "schema_version": CURRENT_SCHEMA_VERSION,
            "event_id": "event-1",
            "stream_id": "session-1",
            "sequence": 1,
            "occurred_at": "2026-08-09T00:00:00Z",
            "event_type": "test.event",
            "payload": "payload",
        })
    );
}

#[test]
fn upcaster_rejects_unknown_schema_family() {
    let input = EventEnvelope::new(
        SchemaFamily::new("unrecognized-family"),
        SchemaVersion::new(CURRENT_SCHEMA_VERSION),
        EventId::new("event-1"),
        StreamId::new("session-1"),
        EventSequence::new(1),
        OccurredAt::new("2026-08-09T00:00:00Z"),
        EventType::new("test.event"),
        "payload",
    );

    assert_eq!(
        CurrentEventUpcaster.upcast(input),
        Err(EventError::UnsupportedSchema {
            schema_family: SchemaFamily::new("unrecognized-family"),
            schema_version: SchemaVersion::new(CURRENT_SCHEMA_VERSION),
        })
    );
}

#[test]
fn upcaster_rejects_unknown_schema_version() {
    let input = EventEnvelope::new(
        SchemaFamily::new(CURRENT_SCHEMA_FAMILY),
        SchemaVersion::new(CURRENT_SCHEMA_VERSION + 1),
        EventId::new("event-1"),
        StreamId::new("session-1"),
        EventSequence::new(1),
        OccurredAt::new("2026-08-09T00:00:00Z"),
        EventType::new("test.event"),
        "payload",
    );

    assert_eq!(
        CurrentEventUpcaster.upcast(input),
        Err(EventError::UnsupportedSchema {
            schema_family: SchemaFamily::new(CURRENT_SCHEMA_FAMILY),
            schema_version: SchemaVersion::new(CURRENT_SCHEMA_VERSION + 1),
        })
    );
}

#[test]
fn reducer_is_deterministic_for_the_same_ordered_events() {
    let ordered_events = vec![
        event("event-1", "session-1", 1, "first"),
        event("event-2", "session-1", 2, "second"),
        event("event-3", "research-1", 1, "third"),
    ];
    let reducer = EventReducer;

    assert_eq!(
        reducer.reduce(ordered_events.clone()),
        reducer.reduce(ordered_events)
    );
}

#[test]
fn reducer_ignores_duplicate_event_ids_without_applying_them_twice() {
    let reducer = EventReducer;
    let mut state = reducer
        .reduce([event("event-1", "session-1", 1, "first")])
        .unwrap();

    let outcome = reducer
        .apply(&mut state, event("event-1", "research-1", 42, "duplicate"))
        .unwrap();

    assert_eq!(outcome, ApplyOutcome::DuplicateIgnored);
    assert_eq!(state.applied_event_count(), 1);
    assert_eq!(
        state.last_sequence_for(&StreamId::new("session-1")),
        Some(EventSequence::new(1))
    );
    assert_eq!(state.last_sequence_for(&StreamId::new("research-1")), None);
}

#[test]
fn reducer_rejects_unsupported_schema_at_its_public_entry_points() {
    let invalid_event = || {
        EventEnvelope::new(
            SchemaFamily::new("unrecognized-family"),
            SchemaVersion::new(CURRENT_SCHEMA_VERSION),
            EventId::new("event-1"),
            StreamId::new("session-1"),
            EventSequence::new(1),
            OccurredAt::new("2026-08-09T00:00:00Z"),
            EventType::new("test.event"),
            "payload",
        )
    };
    let expected_error = EventError::UnsupportedSchema {
        schema_family: SchemaFamily::new("unrecognized-family"),
        schema_version: SchemaVersion::new(CURRENT_SCHEMA_VERSION),
    };
    let reducer = EventReducer;

    assert_eq!(
        reducer.reduce([invalid_event()]),
        Err(expected_error.clone())
    );

    let mut state = State::default();
    assert_eq!(
        reducer.apply(&mut state, invalid_event()),
        Err(expected_error)
    );
    assert_eq!(state.applied_event_count(), 0);
    assert_eq!(state.last_sequence_for(&StreamId::new("session-1")), None);
}

#[test]
fn reducer_rejects_non_increasing_sequences_for_new_events() {
    let reducer = EventReducer;
    let mut state = reducer
        .reduce([event("event-1", "session-1", 2, "first")])
        .unwrap();

    let error = reducer
        .apply(&mut state, event("event-2", "session-1", 2, "invalid"))
        .unwrap_err();

    assert_eq!(
        error,
        EventError::NonIncreasingSequence {
            stream_id: StreamId::new("session-1"),
            previous: EventSequence::new(2),
            received: EventSequence::new(2),
        }
    );
    assert_eq!(state.applied_event_count(), 1);
    assert_eq!(
        state.last_sequence_for(&StreamId::new("session-1")),
        Some(EventSequence::new(2))
    );
}
