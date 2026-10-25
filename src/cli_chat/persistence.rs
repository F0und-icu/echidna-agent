//! One process-local Session projection, always checked against durable bytes.

use crate::adapters::project_event_store::{
    JsonlEventStore, Project, ProjectEventStream, SessionStreamCursor,
};
use crate::app::session::SessionProjection;
use crate::domain::events::StreamId;

use super::{ChatError, DurableChatSession};

pub(super) struct CachedSessionProjection {
    projection: SessionProjection,
    cursor: SessionStreamCursor,
}

impl CachedSessionProjection {
    pub(super) fn load(project: &Project, session_id: &StreamId) -> Result<Self, ChatError> {
        let mut writer = project.open_writer()?;
        let (store, events) = writer.open_session_projection(session_id.clone(), None)?;
        Ok(Self {
            projection: SessionProjection::replay(events)?,
            cursor: store.projection_cursor(),
        })
    }

    pub(super) fn state(&self) -> &crate::domain::session::SessionState {
        self.projection.state()
    }
}

impl DurableChatSession {
    /// The Project lease serializes all appenders. Recheck the exact cached
    /// prefix and reduce any independent Tool append before allocating IDs.
    /// Failed synchronization or writes invalidate the disposable projection.
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
        let mut previous = cache.take();
        let mut writer = self.project.open_writer()?;
        let (mut store, events) = writer.open_session_projection(
            self.session_id.clone(),
            previous.as_ref().map(|cached| &cached.cursor),
        )?;
        let mut projection = match previous.take() {
            Some(mut cached) => {
                cached.projection.ingest_durable(events)?;
                cached.projection
            }
            None => SessionProjection::replay(events)?,
        };
        let stream = ProjectEventStream::Session(self.session_id.clone()).stream_id();
        let value = operation(&mut projection, &mut store, stream)?;
        *cache = Some(CachedSessionProjection {
            projection,
            cursor: store.projection_cursor(),
        });
        Ok(value)
    }

    pub(super) fn durable_state(&self) -> Result<crate::domain::session::SessionState, ChatError> {
        self.with_projection(|projection, _, _| Ok(projection.state().clone()))
    }
}
