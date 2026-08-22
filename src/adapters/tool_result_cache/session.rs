//! Fixed Session Event audit for the Tool Result Cache.

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::adapters::project_event_store::{
    Project, ProjectEventStream, read_existing_session_events,
};
use crate::domain::events::{EventEnvelope, EventId, EventType, OccurredAt, StreamId};
use crate::domain::tools::{ExecutionReceipt, ExecutionReceiptEventPayload, ToolExecutionOutcome};
use crate::ports::EventSink;

const RECEIPT_EVENT_TYPE: &str = "tool.execution_receipt_recorded";
const CACHE_HIT_EVENT_TYPE: &str = "tool.result_cache_hit";

pub(super) struct SessionToolCacheAudit {
    project: Project,
    session_id: StreamId,
    append_gate: Mutex<()>,
}

impl SessionToolCacheAudit {
    pub(super) fn new(
        project: Project,
        session_id: StreamId,
    ) -> Result<Self, crate::adapters::project_event_store::ProjectEventStoreError> {
        // Bind the caller's chosen Session now, then release the project-wide
        // Writer lock before ordinary Tools need it.
        let mut writer = project.open_writer()?;
        writer.open_session(session_id.clone(), None)?;
        drop(writer);
        Ok(Self {
            project,
            session_id,
            append_gate: Mutex::new(()),
        })
    }

    pub(super) fn record_source(
        &self,
        source: &ToolExecutionOutcome,
    ) -> Option<ReceiptEventReference> {
        let event = self.append(
            RECEIPT_EVENT_TYPE,
            ExecutionReceiptEventPayload::current(source.receipt.clone()),
        )?;
        Some(ReceiptEventReference {
            stream_id: event.stream_id,
            event_id: event.event_id,
        })
    }

    pub(super) fn record_hit(&self, receipt: &ExecutionReceipt) -> bool {
        if receipt.cache_source.is_none() {
            return false;
        }
        self.append(
            CACHE_HIT_EVENT_TYPE,
            ExecutionReceiptEventPayload::current(receipt.clone()),
        )
        .is_some()
    }

    pub(super) fn source_is_valid(
        &self,
        reference: &ReceiptEventReference,
        source: &ToolExecutionOutcome,
    ) -> bool {
        if reference.stream_id != self.stream_id() {
            return false;
        }
        let Ok(events) = read_existing_session_events(&self.project, &self.session_id) else {
            return false;
        };
        let mut matching = events
            .into_iter()
            .filter(|event| event.event_id == reference.event_id);
        let Some(event) = matching.next() else {
            return false;
        };
        if matching.next().is_some() || event.event_type.as_str() != RECEIPT_EVENT_TYPE {
            return false;
        }
        serde_json::from_value::<ExecutionReceiptEventPayload>(event.payload)
            .ok()
            .is_some_and(|payload| payload.is_current_for(&source.receipt))
    }

    pub(super) fn stream_id(&self) -> StreamId {
        ProjectEventStream::Session(self.session_id.clone()).stream_id()
    }

    fn append<T: Serialize>(
        &self,
        event_type: &'static str,
        payload: T,
    ) -> Option<EventEnvelope<serde_json::Value>> {
        let _gate = self.append_gate.lock().ok()?;
        let mut writer = self.project.open_writer().ok()?;
        let mut store = writer.open_session(self.session_id.clone(), None).ok()?;
        let sequence = store.next_sequence().ok()?;
        let stream_id = self.stream_id();
        let event = EventEnvelope::current(
            EventId::new(format!(
                "tool-cache:{}:{}",
                stream_id.as_str(),
                sequence.get()
            )),
            stream_id,
            sequence,
            occurred_at(),
            EventType::new(event_type),
            serde_json::to_value(payload).ok()?,
        );
        EventSink::append_event(&mut store, &event).ok()?;
        Some(event)
    }
}

#[derive(Clone, Deserialize, Serialize)]
pub(super) struct ReceiptEventReference {
    pub(super) stream_id: StreamId,
    pub(super) event_id: EventId,
}

fn occurred_at() -> OccurredAt {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    OccurredAt::new(format!(
        "unix:{}.{}",
        duration.as_secs(),
        duration.subsec_nanos()
    ))
}
