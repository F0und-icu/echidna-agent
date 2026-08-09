//! I/O boundaries owned by the application layer.
//!
//! Ports may depend on Domain contracts but never on application, adapter, or
//! user-interface modules. Concrete storage implementations arrive later.

use crate::domain::events::EventEnvelope;
use crate::domain::events::StreamId;

/// Reads one stream in strictly increasing sequence order.
///
/// Implementations return only Events whose `stream_id` matches the requested
/// stream. Storage and recovery details remain outside the Port contract.
pub trait EventSource<T> {
    type Error;

    fn read_events(&self, stream_id: &StreamId) -> Result<Vec<EventEnvelope<T>>, Self::Error>;
}

/// Persists one Event without taking ownership from the caller.
///
/// The caller retains the Event so it can apply the same value to the pure
/// Reducer only after persistence succeeds.
pub trait EventSink<T> {
    type Error;

    fn append_event(&mut self, event: &EventEnvelope<T>) -> Result<(), Self::Error>;
}

/// A paired Event source and sink with one shared error type.
pub trait EventStore<T>:
    EventSource<T> + EventSink<T, Error = <Self as EventSource<T>>::Error>
{
}

impl<T, Store> EventStore<T> for Store where
    Store: EventSource<T> + EventSink<T, Error = <Store as EventSource<T>>::Error>
{
}
