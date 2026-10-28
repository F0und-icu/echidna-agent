//! Disposable Session projection synchronized with exact durable bytes.

use crate::adapters::project_event_store::{
    JsonlEventStore, ProjectEventStream, SessionStreamCursor,
};
use crate::app::session::SessionProjection;
use crate::domain::events::StreamId;

use super::{ChatError, DurableChatSession};

pub(super) struct CachedSessionProjection {
    projection: SessionProjection,
    cursor: SessionStreamCursor,
}

impl DurableChatSession {
    pub(super) fn replay_session_events(
        &self,
        events: Vec<crate::domain::events::EventEnvelope<serde_json::Value>>,
    ) -> Result<SessionProjection, ChatError> {
        let has_v5 = events.iter().try_fold(false, |found, event| {
            Ok::<_, crate::domain::session::SessionError>(found || matches!(
                crate::domain::session::SessionEvent::decode(&event.event_type, event.payload.clone())?,
                crate::domain::session::SessionEvent::CompactionCreated { ref compaction } if compaction.summary().schema_version() == 5
            ))
        })?;
        if !has_v5 {
            return Ok(SessionProjection::replay_verified(events, &self.artifacts)?);
        }
        let research =
            crate::adapters::project_event_store::read_existing_research_events(&self.project)?;
        let (references, receipts) = crate::adapters::research_projection::load_reference_catalog_and_receipt_ids(&self.project)
            .map_err(|error| ChatError::CompactionFacts(crate::adapters::research_projection::CompactionValidationFactsError::Projection(error)))?;
        Ok(SessionProjection::replay_project_verified(
            self.project.project_id().clone(),
            &events,
            &research,
            &references,
            &receipts,
            &self.artifacts,
        )?)
    }
    /// Under the project writer lease, verify the complete old byte prefix and
    /// reduce every independently appended suffix before the operation runs.
    /// Any failure discards this derived cache rather than keeping stale state.
    pub(super) fn with_projection<T>(
        &self,
        operation: impl FnOnce(
            &mut SessionProjection,
            &mut JsonlEventStore<'_>,
            StreamId,
        ) -> Result<T, ChatError>,
    ) -> Result<T, ChatError> {
        let mut cache = self
            .projection
            .lock()
            .map_err(|_| std::io::Error::other("Session projection lock poisoned"))?;
        let previous = cache.take();
        let mut writer = self.project.open_writer()?;
        let (mut store, events) = writer.open_session_projection(
            self.session_id.clone(),
            previous.as_ref().map(|cached| &cached.cursor),
        )?;
        let mut projection = match previous {
            Some(mut cached)
                if !events
                    .iter()
                    .any(|event| event.event_type.as_str() == "session.compaction_created") =>
            {
                cached
                    .projection
                    .ingest_durable_verified(events, &self.artifacts)?;
                cached.projection
            }
            Some(_) => self.replay_session_events(
                crate::adapters::project_event_store::read_existing_session_events(
                    &self.project,
                    &self.session_id,
                )?,
            )?,
            None => self.replay_session_events(events)?,
        };
        let stream = ProjectEventStream::Session(self.session_id.clone()).stream_id();
        let value = operation(&mut projection, &mut store, stream)?;
        *cache = Some(CachedSessionProjection {
            projection,
            cursor: store.projection_cursor(),
        });
        Ok(value)
    }
}
