use std::convert::Infallible;
use std::fs;
use std::path::{Path, PathBuf};

use cyber_agent::domain::events::{
    ApplyOutcome, EventEnvelope, EventId, EventReducer, EventSequence, EventType, OccurredAt,
    State, StreamId,
};
use cyber_agent::ports::{EventSink, EventSource, EventStore};

#[derive(Default)]
struct MemoryEventStore<T> {
    events: Vec<EventEnvelope<T>>,
}

impl<T: Clone> EventSource<T> for MemoryEventStore<T> {
    type Error = Infallible;

    fn read_events(&self, stream_id: &StreamId) -> Result<Vec<EventEnvelope<T>>, Self::Error> {
        Ok(self
            .events
            .iter()
            .filter(|event| &event.stream_id == stream_id)
            .cloned()
            .collect())
    }
}

impl<T: Clone> EventSink<T> for MemoryEventStore<T> {
    type Error = Infallible;

    fn append_event(&mut self, event: &EventEnvelope<T>) -> Result<(), Self::Error> {
        self.events.push(event.clone());
        Ok(())
    }
}

fn assert_event_store<T, Store: EventStore<T>>(_store: &Store) {}

fn event(event_id: &str, stream_id: &str, sequence: u64) -> EventEnvelope<&'static str> {
    EventEnvelope::current(
        EventId::new(event_id),
        StreamId::new(stream_id),
        EventSequence::new(sequence),
        OccurredAt::new("2026-08-09T00:00:00Z"),
        EventType::new("test.event"),
        "payload",
    )
}

#[test]
fn ports_support_an_in_memory_event_store_without_an_adapter() {
    let mut store: MemoryEventStore<&str> = MemoryEventStore::default();
    assert_event_store(&store);

    let session_event = event("event-1", "session-1", 1);
    let next_session_event = event("event-3", "session-1", 2);
    let research_event = event("event-2", "research-1", 1);
    EventSink::append_event(&mut store, &session_event).unwrap();
    EventSink::append_event(&mut store, &next_session_event).unwrap();
    EventSink::append_event(&mut store, &research_event).unwrap();
    let reducer = EventReducer;
    let mut state = State::default();
    assert_eq!(
        reducer.apply(&mut state, session_event),
        Ok(ApplyOutcome::Applied)
    );
    let session_events = EventSource::read_events(&store, &StreamId::new("session-1")).unwrap();
    let research_events = EventSource::read_events(&store, &StreamId::new("research-1")).unwrap();

    assert_eq!(
        session_events,
        vec![
            event("event-1", "session-1", 1),
            event("event-3", "session-1", 2),
        ]
    );
    assert_eq!(research_events, vec![research_event]);
}

#[test]
fn domain_and_ports_keep_the_required_dependency_direction() {
    let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source_roots = ["src/domain", "src/ports"];
    let forbidden_references = [
        "provider",
        "std::fs",
        "std::net",
        "std::process",
        "std::time",
        "std::io",
        "tokio::fs",
        "tokio::net",
        "tokio::process",
        "tokio::time",
        "crate::app",
        "crate::adapters",
        "crate::ui",
        "super::app",
        "super::adapters",
        "super::ui",
        "reqwest",
        "openai",
    ];

    for source_root in source_roots {
        for file in rust_sources_under(&repository_root.join(source_root)) {
            let source = source_without_comments(&fs::read_to_string(&file).unwrap());
            let lowercase_source = source.to_ascii_lowercase();

            for forbidden in forbidden_references {
                assert!(
                    !lowercase_source.contains(forbidden),
                    "{} must not depend on {forbidden}",
                    file.display()
                );
            }
        }
    }
}

fn rust_sources_under(directory: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_rust_sources(directory, &mut files);
    files.sort();
    files
}

fn collect_rust_sources(directory: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();

        if path.is_dir() {
            collect_rust_sources(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

fn source_without_comments(source: &str) -> String {
    source
        .lines()
        .map(|line| line.split_once("//").map_or(line, |(code, _)| code))
        .collect::<Vec<_>>()
        .join("\n")
}
