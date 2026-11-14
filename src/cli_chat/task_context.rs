//! Durable source preparation and adoption for the fixed task-context stage.

use crate::app::agent_loop::{
    AgentLoopError, TaskContextPrepared, TaskContextStage, TaskRefreshDecision, TaskResumeMaterial,
    TaskSelectionMaterial, TaskSelectionSampleOutput,
};
use crate::domain::context::ProviderSessionRef;
use crate::domain::events::EventId;
use crate::domain::request_audit::RequestAuditAttempt;
use crate::domain::session::{
    CompactionContextCapture, SessionEvent, SessionSourceIdentity, TaskContextCapture,
    TaskContextSelected, TaskSelectionSample,
};
use crate::domain::task_context::TaskSelectionDecision;
use crate::ports::artifact::ArtifactStore;

use super::{ChatError, DurableChatSession, event_id, occurred_at};

pub(super) struct TaskContextRuntime<'a> {
    session: &'a DurableChatSession,
    prepared: Option<PreparedSource>,
    diagnostic: bool,
    selection_provider: Option<(
        std::sync::Arc<dyn crate::ports::model::ModelProvider>,
        crate::domain::profile::AgentModelProfile,
    )>,
}

struct PreparedSource {
    source: SessionSourceIdentity,
    state: crate::domain::session::SessionState,
    capture: TaskContextCapture,
    events: Vec<std::sync::Arc<crate::domain::events::EventEnvelope<serde_json::Value>>>,
    cursor: crate::adapters::project_event_store::SessionStreamCursor,
    project_revision: crate::adapters::file_revision::FileRevision,
}

fn stage_error(error: impl std::fmt::Display) -> AgentLoopError {
    AgentLoopError::HistoryCheckpointCommit {
        message: error.to_string(),
    }
}

impl TaskContextRuntime<'_> {
    pub(super) fn new(session: &DurableChatSession) -> TaskContextRuntime<'_> {
        TaskContextRuntime {
            session,
            prepared: None,
            diagnostic: false,
            selection_provider: None,
        }
    }
    pub(super) fn with_diagnostics(mut self, enabled: bool) -> Self {
        self.diagnostic = enabled;
        self
    }
    pub(super) fn with_selection_provider(
        mut self,
        provider: (
            std::sync::Arc<dyn crate::ports::model::ModelProvider>,
            crate::domain::profile::AgentModelProfile,
        ),
    ) -> Self {
        self.selection_provider = Some(provider);
        self
    }
    fn current_memory(&self) -> Result<Vec<crate::domain::session::TaskMemoryDocument>, ChatError> {
        Ok(super::context::load_task_memory(
            &self.session.project,
            &self.session.binding,
        )?)
    }
    pub(super) fn restore_after_resume<Provider: crate::ports::model::ModelProvider + ?Sized>(
        &self,
        agent: &mut crate::app::agent_loop::AgentLoop<
            '_,
            '_,
            Provider,
            crate::adapters::tools::ToolRegistry,
        >,
    ) -> Result<(), ChatError> {
        let restored = self.session.with_projection(|projection, _, _| {
            let validated_source = projection.take_resume_source();
            let state = projection.state();
            let Some((selection_event, selection)) = state.latest_task_context_selection() else {
                return Ok(None);
            };
            let events = crate::adapters::project_event_store::read_existing_session_events(
                &self.session.project,
                &self.session.session_id,
            )?;
            let project_events =
                crate::adapters::project_event_store::read_existing_research_events(
                    &self.session.project,
                )?;
            let end =
                events.partition_point(|event| event.sequence <= selection.source.watermark());
            let project_end = project_events.partition_point(|event| {
                event.sequence.get() <= selection.source.project_stream_watermark()
            });
            let (actual, source_state) = match validated_source {
                Some(source) if source.0 == selection.source => {
                    if SessionSourceIdentity::project_hash_for_events(
                        &project_events[..project_end],
                    )? != selection.source.project_stream_hash()
                    {
                        return Err(ChatError::CompactionSummarySource {
                            message: "resumed task Research prefix changed".to_owned(),
                        });
                    }
                    source
                }
                _ => SessionSourceIdentity::from_events(
                    self.session.project.project_id().clone(),
                    &events[..end],
                    &project_events[..project_end],
                )?,
            };
            if actual != selection.source {
                return Err(ChatError::CompactionSummarySource {
                    message: "resumed task source prefix changed".to_owned(),
                });
            }
            let bytes = self
                .session
                .artifacts
                .read_blob(&selection.capture_blob)
                .map_err(|error| ChatError::CompactionSummarySource {
                    message: format!("task context capture read failed: {error}"),
                })?;
            let capture: TaskContextCapture = serde_json::from_slice(&bytes).map_err(|error| {
                ChatError::CompactionSummarySource {
                    message: format!("task context capture is invalid: {error}"),
                }
            })?;
            let research_units = if capture.compile.research_enabled {
                let read = self.session.project_read(state)?;
                source_state.task_research_units(
                    selection.source.project_stream_hash(),
                    &capture.compile,
                    &project_events[..project_end],
                    &read.references,
                    &selection.selected_research,
                )?
            } else {
                Vec::new()
            };
            crate::app::agent_loop::compile_captured_task_context(
                &source_state,
                &capture,
                selection,
                events[..end].iter(),
                &research_units,
            )?;
            let (history, selected_summary) =
                state.task_resume_projection(selection_event, selection, &events)?;
            let constraints = source_state.constraint_contents(selection)?;
            let excerpts = source_state.excerpt_contents(selection)?;
            let progress = source_state.selected_task_progress(
                selection,
                &crate::app::context_read::history_navigation_excerpt,
            )?;
            let core = capture.common.fragments()?.into_iter().find(|fragment| {
                fragment.source()
                    == crate::domain::context::ContextFragmentSource::ProjectMemoryCore
            });
            let current_hash = crate::domain::canonical_json::canonical_array_hash(&events)
                .map_err(|error| ChatError::CompactionSummarySource {
                    message: error.to_string(),
                })?;
            Ok(Some((
                TaskResumeMaterial {
                    selection: selection.clone(),
                    history,
                    constraints,
                    excerpts,
                    progress,
                    research_units,
                    memory: capture.memory.clone(),
                },
                core,
                selected_summary,
                current_hash,
            )))
        })?;
        if let Some((material, core, summary, source_hash)) = restored {
            agent.restore_task_context_for_compaction(
                material,
                core.as_ref(),
                summary,
                &source_hash,
            )?;
        }
        Ok(())
    }

    /// Called only inside `with_projection`, which synchronizes the validated
    /// Session prefix before checking newly appended selection events.
    // Borrow the three existing source components without another wrapper type.
    #[allow(clippy::type_complexity)]
    fn verify_source(
        &self,
        prepared: &TaskContextPrepared,
        store: &crate::adapters::project_event_store::JsonlEventStore<'_>,
    ) -> Result<
        (
            &crate::domain::session::SessionState,
            &TaskContextCapture,
            &[std::sync::Arc<crate::domain::events::EventEnvelope<serde_json::Value>>],
        ),
        ChatError,
    > {
        let cached = self
            .prepared
            .as_ref()
            .filter(|cached| cached.source == prepared.source)
            .ok_or_else(|| ChatError::CompactionSummarySource {
                message: "task selection prepared source is absent".to_owned(),
            })?;
        let project_path = self.session.project.research_stream_path_existing()?;
        if crate::adapters::file_revision::FileRevision::of(&std::fs::metadata(project_path)?)?
            != cached.project_revision
        {
            return Err(ChatError::CompactionSummarySource {
                message: "task selection source prefix changed".to_owned(),
            });
        }
        for event in store.read_prepared_suffix(&cached.cursor)? {
            match SessionEvent::decode(&event.event_type, event.payload.clone())? {
                SessionEvent::RequestAuditRecorded { manifest }
                    if manifest.provider_session().is_some_and(|scope| {
                        scope
                            .id
                            .to_string()
                            .starts_with(&format!("task-selection:{}:", prepared.capture_event_id))
                    }) => {}
                SessionEvent::TaskSelectionSampleRecorded { sample }
                    if sample.capture_event_id == prepared.capture_event_id
                        && sample.source == prepared.source => {}
                SessionEvent::TaskSelectionUsageRecorded {
                    capture_event_id,
                    provider_session,
                    ..
                } if capture_event_id == prepared.capture_event_id
                    && provider_session
                        .id
                        .to_string()
                        .starts_with(&format!("task-selection:{}:", prepared.capture_event_id)) => {
                }
                SessionEvent::ModelUsageRecorded {
                    provider_session, ..
                } if provider_session
                    .id
                    .to_string()
                    .starts_with(&format!("task-selection:{}:", prepared.capture_event_id)) => {}
                _ => {
                    return Err(ChatError::CompactionSummarySource {
                        message: "task selection source has an unexpected durable suffix"
                            .to_owned(),
                    });
                }
            }
        }
        let capture = &cached.capture;
        if capture.schema_version != 2
            || capture.candidate_policy_version
                != crate::domain::session::TASK_CANDIDATE_POLICY_VERSION
            || capture.candidates != prepared.candidates
            || capture.expansion != prepared.expansion
            || self.current_memory()? != capture.memory
        {
            return Err(ChatError::CompactionSummarySource {
                message: "task selection candidates do not match the frozen source".to_owned(),
            });
        }
        Ok((&cached.state, capture, &cached.events))
    }

    fn research_units(
        &self,
        state: &crate::domain::session::SessionState,
        prepared: &TaskContextPrepared,
        capture: &TaskContextCapture,
        decision: &TaskSelectionDecision,
    ) -> Result<Vec<crate::domain::task_context_research::ResearchTaskUnit>, ChatError> {
        let mut selected = prepared
            .candidates
            .context
            .iter()
            .filter_map(|candidate| {
                candidate
                    .research_ref
                    .as_ref()
                    .filter(|_| {
                        decision
                            .required_context_refs
                            .contains(&candidate.reference)
                    })
                    .cloned()
            })
            .collect::<Vec<_>>();
        selected.extend(
            state
                .selected_summary_research_refs(decision, prepared.source.project_stream_hash())?,
        );
        selected.sort();
        selected.dedup();
        if !capture.compile.research_enabled {
            return Ok(Vec::new());
        }
        let read = self.session.project_read(state)?;
        let end = read.events.partition_point(|event| {
            event.sequence.get() <= prepared.source.project_stream_watermark()
        });
        let actual = SessionSourceIdentity::project_hash_for_events(&read.events[..end])?;
        if end != read.events.len() || actual != prepared.source.project_stream_hash() {
            return Err(ChatError::CompactionSummarySource {
                message: "Research source changed before expansion".to_owned(),
            });
        }
        let research = read.research_state();
        crate::domain::task_context_research::resolve_selection(
            research,
            &read.references,
            prepared.source.project_stream_hash(),
            &selected,
        )
        .map_err(|error| ChatError::CompactionSummarySource {
            message: format!("Research source dependency expansion failed: {error}"),
        })
    }
}

impl TaskContextStage for TaskContextRuntime<'_> {
    fn control_directory(&self) -> &'static str {
        self.session.project.control_directory_name()
    }

    fn current_protected_context(
        &mut self,
    ) -> Result<crate::domain::session::ProtectedOperatorContext, AgentLoopError> {
        self.session
            .with_projection(|projection, _, _| {
                Ok(projection.state().current_protected_operator_context()?)
            })
            .map_err(stage_error)
    }

    fn refresh_decision(
        &mut self,
        prepared: &TaskContextPrepared,
    ) -> Result<TaskRefreshDecision, AgentLoopError> {
        self.session
            .with_projection(|_, store, _| {
                let (state, _, _) = self.verify_source(prepared, store)?;
                let (id, decision) = state.deterministic_task_refresh(
                    &prepared.input_ref,
                    &prepared.candidates,
                    true,
                    crate::domain::session::TASK_REQUEST_PROOF_VERSION,
                )?;
                let previous = state
                    .latest_task_context_selection()
                    .ok_or(AgentLoopError::ContextStateMismatch)?
                    .1
                    .clone();
                Ok(TaskRefreshDecision {
                    prior_event_id: id,
                    prior: previous,
                    decision,
                })
            })
            .map_err(stage_error)
    }

    fn prepare(
        &mut self,
        common: CompactionContextCapture,
        compile: crate::domain::session::TaskCompileCapture,
        input: &EventId,
        expansion: Option<TaskSelectionDecision>,
        source_refresh: bool,
    ) -> Result<TaskContextPrepared, AgentLoopError> {
        let timing = super::timing::TurnTiming::new(self.diagnostic);
        timing.checkpoint("task_prepare_start");
        // Acquire before the projection lock: building the source index itself
        // verifies and advances that projection.
        let source_index =
            super::context_read::index::ColdIndex::acquire(self.session).map_err(stage_error)?;
        timing.checkpoint("task_source_index");
        let mut task_cache =
            super::task_index::update(self.session, &source_index).map_err(stage_error)?;
        timing.checkpoint("task_postings_ready");
        let result = self
            .session
            .with_projection(|projection, store, stream_id| {
                let task_index = task_cache.as_mut().expect("task index was installed");
                let state = projection.state();
                let last = state
                    .history_items_with_event_ids()
                    .last()
                    .map(|(id, _)| id.as_str());
                if !task_index.matches(state.history().items().len(), last) {
                    return Err(ChatError::CompactionSummarySource {
                        message: "task index source changed before candidate preparation"
                            .to_owned(),
                    });
                }
                task_index.update_navigation(state, input)?;
                if compile.research_enabled
                    != projection
                        .state()
                        .agent_definition_binding()
                        .is_some_and(|binding| binding.definition().research().is_some())
                {
                    return Err(ChatError::CompactionSummarySource {
                        message: "Research capture does not match the bound AgentDefinition"
                            .to_owned(),
                    });
                }
                let project_path = self.session.project.research_stream_path_existing()?;
                let project_revision = crate::adapters::file_revision::FileRevision::of(
                    &std::fs::metadata(&project_path)?,
                )?;
                let read = if compile.research_enabled {
                    Some(self.session.project_read(projection.state())?)
                } else {
                    None
                };
                let raw_events = if read.is_none() {
                    Some(
                        crate::adapters::project_event_store::read_existing_research_events(
                            &self.session.project,
                        )?,
                    )
                } else {
                    None
                };
                let project_events = match &read {
                    Some(read) => read.events.as_slice(),
                    None => raw_events
                        .as_deref()
                        .expect("non-Research source was loaded"),
                };
                let project_hash = SessionSourceIdentity::project_hash_for_events(project_events)?;
                let (research_candidates, research_omitted) = match &read {
                    Some(read) => projection.state().task_research_candidates_from_state(
                        self.session.project.project_id().as_str(),
                        input,
                        &project_hash,
                        read.research_state(),
                        &read.references,
                        expansion.as_ref(),
                        source_refresh,
                    )?,
                    None => (Vec::new(), 0),
                };
                let memory = self.current_memory()?;
                let mut candidates = projection.state().task_selection_candidates_with_index(
                    self.session.project.project_id().as_str(),
                    input,
                    &common,
                    &research_candidates,
                    research_omitted,
                    &memory,
                    expansion.as_ref(),
                    source_refresh,
                    crate::domain::session::TASK_CANDIDATE_POLICY_VERSION,
                    &crate::app::context_read::history_navigation_excerpt,
                    Some(&|card, key, before, limit| {
                        task_index
                            .posting(card, key, before, limit)
                            .map_err(|error| crate::domain::session::SessionError::TaskContext {
                                message: format!("task posting read failed: {error}"),
                            })
                    }),
                    Some(&task_index.view(state)),
                )?;
                projection
                    .state()
                    .append_current_visual_candidates(input, &mut candidates)?;
                timing.checkpoint("task_candidates_ready");
                let capture = TaskContextCapture {
                    schema_version: 2,
                    candidate_policy_version: crate::domain::session::TASK_CANDIDATE_POLICY_VERSION,
                    common,
                    candidates: candidates.clone(),
                    expansion: expansion.clone(),
                    source_refresh,
                    compile,
                    research_candidates,
                    research_omitted,
                    memory,
                };
                capture.common.fragments()?;
                let bytes = serde_json::to_vec(&capture).map_err(|error| {
                    ChatError::CompactionSummarySource {
                        message: error.to_string(),
                    }
                })?;
                let blob = self
                    .session
                    .artifacts
                    .publish_bytes(&bytes)
                    .map_err(|error| ChatError::CompactionSummarySource {
                        message: format!("task context capture publication failed: {error}"),
                    })?
                    .blob_id;
                let sequence = store.next_sequence()?;
                let id = event_id(
                    self.session.project.project_id(),
                    &self.session.session_id,
                    sequence.get(),
                    "task-context-capture",
                );
                let event = SessionEvent::TaskContextCaptured {
                    blob_id: blob.clone(),
                }
                .into_envelope(id.clone(), stream_id, sequence, occurred_at())?;
                projection
                    .append(store, event)
                    .map_err(ChatError::from_session_append)?;
                let source = projection.task_source_identity(
                    self.session.project.project_id().clone(),
                    project_events,
                )?;
                let state = projection.state().clone();
                let events = projection.task_source_events().to_vec();
                let samples_used = projection.state().task_selection_sample_count(input);
                if !events.iter().any(|event| &event.event_id == input) {
                    return Err(ChatError::CompactionSummarySource {
                        message: "task input Event is absent from its prepared source".to_owned(),
                    });
                }
                if crate::adapters::file_revision::FileRevision::of(&std::fs::metadata(
                    &project_path,
                )?)? != project_revision
                {
                    return Err(ChatError::CompactionSummarySource {
                        message: "Research source changed during task preparation".to_owned(),
                    });
                }
                let cached = PreparedSource {
                    source: source.clone(),
                    state,
                    capture,
                    events,
                    cursor: store.projection_cursor(),
                    project_revision,
                };
                timing.checkpoint("task_source_ready");
                Ok((
                    TaskContextPrepared {
                        source,
                        capture_event_id: id,
                        capture_blob: blob,
                        input_ref: input.clone(),
                        candidates,
                        samples_used,
                        expansion,
                    },
                    cached,
                ))
            });
        if let Some(index) = task_cache.as_ref() {
            super::timing::task_index_work(self.diagnostic, &index.stats());
        }
        if result.is_err() {
            *task_cache = None;
        }
        let (prepared, cached) = result.map_err(stage_error)?;
        self.prepared = Some(cached);
        Ok(prepared)
    }

    fn record_attempt(&mut self, attempt: RequestAuditAttempt) -> Result<EventId, AgentLoopError> {
        self.session
            .record_request_audit_manifest(attempt, None)
            .map(|manifest| manifest.audit_event_id().clone())
            .map_err(stage_error)
    }

    fn record_completed_usage(
        &mut self,
        capture: &EventId,
        audit: &EventId,
        scope: &ProviderSessionRef,
        usage: crate::domain::model::ModelUsage,
    ) -> Result<(), AgentLoopError> {
        self.session
            .record_task_selection_usage(capture, audit, scope, usage)
            .map_err(stage_error)
    }

    fn selection_provider(
        &self,
    ) -> (
        std::sync::Arc<dyn crate::ports::model::ModelProvider>,
        Option<crate::domain::profile::AgentModelProfile>,
    ) {
        let (provider, profile) = self
            .selection_provider
            .as_ref()
            .expect("task selection provider is installed for a turn");
        (provider.clone(), Some(profile.clone()))
    }

    fn record_sample(
        &mut self,
        prepared: &TaskContextPrepared,
        scope: &ProviderSessionRef,
        output: &TaskSelectionSampleOutput,
    ) -> Result<EventId, AgentLoopError> {
        let sample = TaskSelectionSample {
            schema_version: if output.rejected { 2 } else { 1 },
            source: prepared.source.clone(),
            capture_event_id: prepared.capture_event_id.clone(),
            capture_blob: prepared.capture_blob.clone(),
            input_ref: prepared.input_ref.clone(),
            audit_event_id: output.audit_event_id.clone(),
            provider_session: scope.clone(),
            decision: output.decision.clone(),
        };
        let id =
            self.session
                .with_projection(|projection, store, stream_id| {
                    self.verify_source(prepared, store)?;
                    let sequence = store.next_sequence()?;
                    let id = event_id(
                        self.session.project.project_id(),
                        &self.session.session_id,
                        sequence.get(),
                        "task-selection-sample",
                    );
                    let event = SessionEvent::TaskSelectionSampleRecorded { sample }
                        .into_envelope(id.clone(), stream_id, sequence, occurred_at())?;
                    projection
                        .append(store, event)
                        .map_err(ChatError::from_session_append)?;
                    Ok(id)
                })
                .map_err(stage_error)?;
        Ok(id)
    }

    fn selection(
        &mut self,
        prepared: &TaskContextPrepared,
        sample_event_id: Option<EventId>,
        decision: TaskSelectionDecision,
    ) -> Result<TaskSelectionMaterial, AgentLoopError> {
        self.session
            .with_projection(|projection, store, _| {
                let (source_state, capture, events) = self.verify_source(prepared, store)?;
                let research_units =
                    self.research_units(source_state, prepared, capture, &decision)?;
                let (selection, _) = projection.state().prepare_task_selection(
                    prepared.source.clone(),
                    prepared.capture_event_id.clone(),
                    prepared.capture_blob.clone(),
                    prepared.input_ref.clone(),
                    sample_event_id,
                    &prepared.candidates,
                    &research_units,
                    &capture.compile.live_processes,
                    decision,
                )?;
                let constraints = projection.state().constraint_contents(&selection)?;
                let excerpts = projection.state().excerpt_contents(&selection)?;
                let progress = source_state.selected_task_progress(
                    &selection,
                    &crate::app::context_read::history_navigation_excerpt,
                )?;
                let history = source_state.task_resume_history(
                    &selection.selected_history,
                    events.iter().map(AsRef::as_ref),
                    selection.request_proof_version,
                )?;
                Ok(TaskSelectionMaterial {
                    selection,
                    history,
                    constraints,
                    excerpts,
                    progress,
                    research_units,
                    memory: capture.memory.clone(),
                })
            })
            .map_err(stage_error)
    }

    fn commit(&mut self, selection: TaskContextSelected) -> Result<(), AgentLoopError> {
        let bytes = self
            .session
            .artifacts
            .read_blob(&selection.capture_blob)
            .map_err(stage_error)?;
        let capture: TaskContextCapture = serde_json::from_slice(&bytes).map_err(stage_error)?;
        let samples_used = self
            .session
            .durable_state()
            .map_err(stage_error)?
            .task_selection_sample_count(&selection.input_ref);
        let prepared = TaskContextPrepared {
            source: selection.source.clone(),
            capture_event_id: selection.capture_event_id.clone(),
            capture_blob: selection.capture_blob.clone(),
            input_ref: selection.input_ref.clone(),
            candidates: capture.candidates,
            samples_used,
            expansion: capture.expansion,
        };
        self.session
            .with_projection(|projection, store, stream_id| {
                let (source_state, capture, events) = self.verify_source(&prepared, store)?;
                let research_units =
                    self.research_units(source_state, &prepared, capture, &selection.decision)?;
                crate::app::agent_loop::compile_captured_task_context(
                    source_state,
                    capture,
                    &selection,
                    events.iter().map(AsRef::as_ref),
                    &research_units,
                )
                .map_err(ChatError::from)?;
                let sequence = store.next_sequence()?;
                let event = SessionEvent::TaskContextSelected { selection }.into_envelope(
                    event_id(
                        self.session.project.project_id(),
                        &self.session.session_id,
                        sequence.get(),
                        "task-context-selected",
                    ),
                    stream_id,
                    sequence,
                    occurred_at(),
                )?;
                projection
                    .append(store, event)
                    .map_err(ChatError::from_session_append)?;
                Ok(())
            })
            .map_err(stage_error)
    }
}
