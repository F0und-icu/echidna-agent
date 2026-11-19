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
        let project_events =
            crate::adapters::project_event_store::read_existing_research_events(project)?;
        Ok(Self {
            projection: SessionProjection::replay_for_resume(
                project.project_id().clone(),
                &events,
                &project_events,
                || {
                    crate::adapters::research_projection::load_reference_catalog(project).map_err(
                        |message| crate::domain::session::SessionError::TaskContext { message },
                    )
                },
                &crate::adapters::artifact_store::LocalArtifactStore::for_project(project),
            )?,
            cursor: store.projection_cursor(),
        })
    }

    pub(super) fn state(&self) -> &crate::domain::session::SessionState {
        self.projection.state()
    }
}

impl DurableChatSession {
    pub(super) fn project_read(
        &self,
        state: &crate::domain::session::SessionState,
    ) -> Result<crate::adapters::research_projection::ProjectReadContext, ChatError> {
        let mut receipts = self
            .receipt_cache
            .lock()
            .map_err(|_| std::io::Error::other("Receipt projection lock poisoned"))?;
        let current = state.stream_id().map(|_| (&self.session_id, state));
        crate::adapters::research_projection::ProjectReadContext::load(
            &self.project,
            &mut receipts,
            current,
        )
        .map_err(ChatError::ResearchProjection)
    }

    pub(super) fn refresh_context(
        &self,
        cache: &mut super::context::ChatContextCache,
        tools: &crate::adapters::tools::ToolRegistry,
    ) -> Result<crate::app::agent_loop::CurrentContextState, ChatError> {
        self.with_projection(|projection, _, _| {
            let read = self.project_read(projection.state())?;
            cache
                .refresh_from_read_context(
                    &self.project,
                    tools,
                    &self.binding,
                    &read,
                    &self.memory_cache,
                )
                .map_err(ChatError::from)
        })
    }
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
                if cached.projection.suffix_requires_project_replay(&events)? {
                    let all_events =
                        crate::adapters::project_event_store::read_existing_session_events(
                            &self.project,
                            &self.session_id,
                        )?;
                    let project_events =
                        crate::adapters::project_event_store::read_existing_research_events(
                            &self.project,
                        )?;
                    SessionProjection::replay_for_resume(
                        self.project.project_id().clone(),
                        &all_events,
                        &project_events,
                        || {
                            crate::adapters::research_projection::load_reference_catalog(
                                &self.project,
                            )
                            .map_err(|message| {
                                crate::domain::session::SessionError::TaskContext { message }
                            })
                        },
                        &self.artifacts,
                    )?
                } else {
                    cached
                        .projection
                        .ingest_durable_verified(events, &self.artifacts)?;
                    cached.projection
                }
            }
            None => {
                let project_events =
                    crate::adapters::project_event_store::read_existing_research_events(
                        &self.project,
                    )?;
                SessionProjection::replay_for_resume(
                    self.project.project_id().clone(),
                    &events,
                    &project_events,
                    || {
                        crate::adapters::research_projection::load_reference_catalog(&self.project)
                            .map_err(
                                |message| crate::domain::session::SessionError::TaskContext {
                                    message,
                                },
                            )
                    },
                    &self.artifacts,
                )?
            }
        };
        let stream = ProjectEventStream::Session(self.session_id.clone()).stream_id();
        let value = operation(&mut projection, &mut store, stream)?;
        let cursor = store.projection_cursor();
        self.receipt_cache
            .lock()
            .map_err(|_| std::io::Error::other("Receipt projection lock poisoned"))?
            .remember_validated_session(
                &self.session_id,
                projection.state(),
                cursor.file_revision().clone(),
            );
        *cache = Some(CachedSessionProjection { projection, cursor });
        Ok(value)
    }

    pub(super) fn durable_state(&self) -> Result<crate::domain::session::SessionState, ChatError> {
        self.with_projection(|projection, _, _| Ok(projection.state().clone()))
    }
}
