//! Append-before-apply Session composition through Event ports.

use crate::domain::events::{ApplyOutcome, EventEnvelope, EventId};
use crate::domain::session::{SessionError, SessionReducer, SessionState};
use crate::ports::EventSink;

/// Explicit turn-boundary requests; defaults come from the validated binding.
pub(crate) struct ResearchTurnRequest {
    pub(crate) started: bool,
    pub(crate) default_domain: crate::domain::domain_profile::DomainProfile,
    pub(crate) default_phase: crate::domain::research_control::ResearchPhase,
    pub(crate) domain: Option<crate::domain::domain_profile::DomainProfileSelection>,
    pub(crate) phase: Option<crate::domain::research_control::ResearchPhaseSelection>,
}

/// Validates one prospective state, appends its Event durably, then promotes
/// that state. A failed append never mutates the caller's `SessionState`.
fn append_after_durable<S>(
    reducer: &SessionReducer,
    state: &mut SessionState,
    sink: &mut S,
    event: EventEnvelope<serde_json::Value>,
) -> Result<ApplyOutcome, AppendAfterDurableError<S::Error>>
where
    S: EventSink<serde_json::Value>,
{
    let mut next_state = state.clone();
    let outcome = reducer
        .apply_in_place(&mut next_state, event.clone())
        .map_err(AppendAfterDurableError::Session)?;
    if outcome == ApplyOutcome::DuplicateIgnored {
        return Ok(outcome);
    }
    sink.append_event(&event)
        .map_err(AppendAfterDurableError::Sink)?;
    *state = next_state;
    Ok(outcome)
}

/// The app-level projection of one Session Event stream. The composition root
/// reads Events from an `EventSource`, assigns stream sequences and supplies
/// its durable `EventSink`; this service does not know the JSONL layout.
pub struct SessionProjection {
    reducer: SessionReducer,
    state: SessionState,
    compaction_source: Option<crate::domain::session::TaskCompactionValidationFacts>,
    current_source: Option<crate::domain::session::SessionSourceIdentity>,
    resume_source: Option<(crate::domain::session::SessionSourceIdentity, SessionState)>,
    source_hash: crate::domain::canonical_json::CanonicalArrayHasher,
    task_source_events: Vec<std::sync::Arc<EventEnvelope<serde_json::Value>>>,
}

struct TaskPlanReplay {
    session: crate::domain::context::ProviderSessionRef,
    plan: crate::domain::context::ContextPlan,
    requires_full: bool,
    first: bool,
    selection: crate::domain::session::TaskContextSelected,
    source: SessionState,
    capture: crate::domain::session::TaskContextCapture,
    research_units: Vec<crate::domain::task_context_research::ResearchTaskUnit>,
    delivery: Option<crate::app::agent_loop::TaskDeliveryReplay>,
}

/// Required image/observation references are derived from the verified source
/// facts. The Store revalidates their byte identities; this does not claim the
/// tool-free summary model read image bytes or ordinary capture contents.
pub(crate) fn verify_compaction_source_blobs<Store: crate::ports::artifact::ArtifactStore>(
    graph: &crate::domain::session::VerifiedCompactionGraph,
    blobs: &Store,
) -> Result<(), SessionError>
where
    Store::Error: std::fmt::Debug,
{
    for id in &graph.required_source_blobs {
        blobs
            .read_blob(id)
            .map_err(|error| SessionError::CompactionContextProvenance {
                message: format!(
                    "required Compaction source Blob `{id}` is unavailable or invalid: {error:?}"
                ),
            })?;
    }
    verify_vision_sources(
        graph
            .required_vision_sources
            .iter()
            .map(|(observation, audit)| (observation, audit.as_ref())),
        blobs,
    )?;
    Ok(())
}

pub(crate) fn verify_protected_vision_sources<Store: crate::ports::artifact::ArtifactStore>(
    state: &SessionState,
    blobs: &Store,
) -> Result<(), SessionError>
where
    Store::Error: std::fmt::Debug,
{
    verify_vision_sources(state.protected_vision_sources()?, blobs)
}

fn verify_vision_sources<'a, Store: crate::ports::artifact::ArtifactStore>(
    sources: impl IntoIterator<
        Item = (
            &'a crate::domain::vision::VisionObservation,
            Option<&'a crate::domain::request_audit::RequestAuditManifest>,
        ),
    >,
    blobs: &Store,
) -> Result<(), SessionError>
where
    Store::Error: std::fmt::Debug,
{
    let replay = crate::app::vision::VisionReplayService::new(blobs);
    for (observation, audit) in sources {
        replay
            .verify_observation(observation, audit)
            .map_err(|error| SessionError::CompactionContextProvenance {
                message: format!("protected Vision observation verification failed: {error}"),
            })?;
    }
    Ok(())
}

impl SessionProjection {
    pub(crate) fn replay_project_verified<Store: crate::ports::artifact::ArtifactStore>(
        project_id: crate::domain::project::ProjectId,
        events: &[EventEnvelope<serde_json::Value>],
        project_events: &[EventEnvelope<serde_json::Value>],
        load_research_references: impl FnMut() -> Result<
            crate::domain::research::ResearchReferenceCatalog,
            SessionError,
        >,
        blobs: &Store,
    ) -> Result<Self, SessionError>
    where
        Store::Error: std::fmt::Debug,
    {
        Self::replay_project(
            project_id,
            events,
            project_events,
            load_research_references,
            blobs,
            0,
            false,
        )
    }

    /// Restore current execution state without auditing every historical request.
    /// The latest selection supplies the complete source for the current task.
    pub(crate) fn replay_for_resume<Store: crate::ports::artifact::ArtifactStore>(
        project_id: crate::domain::project::ProjectId,
        events: &[EventEnvelope<serde_json::Value>],
        project_events: &[EventEnvelope<serde_json::Value>],
        load_research_references: impl FnMut() -> Result<
            crate::domain::research::ResearchReferenceCatalog,
            SessionError,
        >,
        blobs: &Store,
    ) -> Result<Self, SessionError>
    where
        Store::Error: std::fmt::Debug,
    {
        let latest = events
            .iter()
            .rposition(|event| event.event_type.as_str() == "session.task_context_selected");
        let start = latest.map_or(0, |end| {
            events[..end]
                .iter()
                .rposition(|event| event.event_type.as_str() == "session.task_context_captured")
                .unwrap_or(0)
        });
        Self::replay_project(
            project_id,
            events,
            project_events,
            load_research_references,
            blobs,
            start,
            true,
        )
    }

    fn replay_project<Store: crate::ports::artifact::ArtifactStore>(
        project_id: crate::domain::project::ProjectId,
        events: &[EventEnvelope<serde_json::Value>],
        project_events: &[EventEnvelope<serde_json::Value>],
        mut load_research_references: impl FnMut() -> Result<
            crate::domain::research::ResearchReferenceCatalog,
            SessionError,
        >,
        blobs: &Store,
        verify_from: usize,
        recent_only: bool,
    ) -> Result<Self, SessionError>
    where
        Store::Error: std::fmt::Debug,
    {
        use crate::domain::session::SessionEvent;
        let mut graphs = std::collections::BTreeMap::new();
        let mut research_references = None;
        let empty_references = crate::domain::research::ResearchReferenceCatalog::default();
        let mut pending_task_plan: Option<TaskPlanReplay> = None;
        let reducer = SessionReducer;
        let mut index = 0usize;
        let expected_project = project_id.clone();
        let mut prefix_hash = crate::domain::canonical_json::CanonicalArrayHasher::new();
        let mut pending_capture = None::<EventId>;
        let mut captured_source = None::<(EventId, SessionState, String)>;
        let mut project_hash = None::<(usize, String)>;
        let (state, mut compaction_source, current_source) = reducer
            .reduce_with_compaction_source_checked(
                project_id,
                events,
                project_events,
                |state, event| {
                    if index != 0 {
                        prefix_hash.push(&events[index - 1]).map_err(|error| SessionError::TaskContext { message: format!("Session prefix hashing failed: {error}") })?;
                        if !recent_only && let Some(pending) = &mut pending_task_plan
                            && pending.selection.has_complete_request_proof()
                        {
                            match SessionEvent::decode(&events[index - 1].event_type, events[index - 1].payload.clone())? {
                                SessionEvent::AgentDefinitionAppliedAtCheckpoint { .. } => {
                                    pending.session = state.primary_provider_session()
                                        .ok_or_else(|| SessionError::TaskContext { message: "checkpoint primary session is absent".to_owned() })?.clone();
                                    pending.requires_full = true;
                                    if pending.selection.request_proof_version >= 4 {
                                        let mut delivery = crate::app::agent_loop::TaskDeliveryReplay::new(&pending.session);
                                        delivery.set_proof6(pending.selection.request_proof_version >= 6);
                                        pending.delivery = Some(delivery);
                                    }
                                }
                                SessionEvent::CompactionCreated { .. } => {
                                    pending.requires_full = true;
                                    if let Some(delivery) = &mut pending.delivery { delivery.reset_compaction(); }
                                }
                                _ => {}
                            }
                        }
                    }
                    if let Some(id) = pending_capture.take() {
                        captured_source = Some((id, state.clone(), prefix_hash.finish()));
                    }
                    if !recent_only {
                        verify_completed_event(state, event, blobs)?;
                    }
                    if index >= verify_from && event.event_type.as_str() == "session.task_context_captured" {
                        pending_capture = Some(event.event_id.clone());
                    }
                    if index >= verify_from && event.event_type.as_str() == "session.task_context_selected" && let SessionEvent::TaskContextSelected { selection } =
                        SessionEvent::decode(&event.event_type, event.payload.clone())?
                    {
                        let research = state.agent_definition_binding().is_some_and(|binding| binding.definition().research().is_some());
                        if research && research_references.is_none() { research_references = Some(load_research_references()?); }
                        let references = if research { research_references.as_ref().expect("Research catalog was loaded") } else { &empty_references };
                        let project_end = project_events.partition_point(|event| event.sequence.get() <= selection.source.project_stream_watermark());
                        if project_hash.as_ref().is_none_or(|(end, _)| *end != project_end) {
                            project_hash = Some((project_end, crate::domain::canonical_json::canonical_array_hash(&project_events[..project_end])
                                .map_err(|error| SessionError::TaskContext { message: format!("Project prefix hashing failed: {error}") })?));
                        }
                        let checkpoint = captured_source.as_ref().filter(|(id, _, _)| id == &selection.capture_event_id)
                            .map(|(_, state, hash)| (state, hash.as_str(), project_hash.as_ref().expect("Project hash was computed").1.as_str()));
                        let (plan, requires_full, source, capture, research_units) = verify_task_context_selection(state, &selection, &events[..index], project_events, references, blobs, &expected_project, checkpoint)?;
                        let requires_full = requires_full || (selection.has_complete_request_proof() && pending_task_plan.as_ref().is_some_and(|pending| pending.selection.has_complete_request_proof() && pending.requires_full));
                        let delivery = if selection.request_proof_version >= 4 {
                            let prior = pending_task_plan.take().filter(|pending| pending.selection.request_proof_version >= 4 && pending.session == selection.provider_session).and_then(|pending| pending.delivery);
                            let mut delivery = prior.unwrap_or_else(|| crate::app::agent_loop::TaskDeliveryReplay::new(&selection.provider_session));
                            delivery.set_proof6(selection.request_proof_version >= 6);
                            Some(delivery)
                        } else { None };
                        pending_task_plan = Some(TaskPlanReplay { session: selection.provider_session.clone(), plan, requires_full, first: true, selection, source, capture, research_units, delivery });
                    }
                    if !recent_only && event.event_type.as_str() == "session.request_audit_recorded" && let SessionEvent::RequestAuditRecorded { manifest } = SessionEvent::decode(&event.event_type, event.payload.clone())?
                        && let Some(pending) = &mut pending_task_plan
                        && manifest.provider_session() == Some(&pending.session)
                        && manifest.improvement_scope().is_none()
                    {
                        if pending.selection.request_proof_version >= 4 && manifest.sampling_step_id().is_none() {
                            return Err(SessionError::TaskContext { message: "primary task audit has no sampling step identity".to_owned() });
                        }
                        let replay_inputs = if (!pending.first && pending.selection.has_complete_request_proof()) || pending.selection.request_proof_version >= 4 {
                            let inputs = crate::app::agent_loop::compile_following_task_context(
                                &pending.source, state, &pending.capture, &pending.selection,
                                &events[..index], &pending.research_units,
                            ).map_err(|error| SessionError::TaskContext { message: format!("primary task suffix compilation failed: {error}") })?;
                            if pending.first && (inputs.plan.model_input() != pending.plan.model_input() || inputs.plan.delivery_map() != pending.plan.delivery_map()) {
                                return Err(SessionError::TaskContext { message: "initial task replay plan differs from captured plan".to_owned() });
                            }
                            pending.plan = inputs.plan.clone();
                            Some(inputs)
                        } else { None };
                        let plan = &pending.plan;
                        let actual = manifest.reconstruct_full_logical(|id| blobs.read_blob(id))
                            .map_err(|error| SessionError::TaskContext { message: format!("primary audit read failed: {error:?}") })?
                            .ok_or_else(|| SessionError::TaskContext { message: "primary audit has no full logical input".to_owned() })?;
                        if actual.input() != &plan.model_input() || actual.delivery_map() != plan.delivery_map() {
                            return Err(SessionError::TaskContext { message: "primary audit differs from independently compiled task source".to_owned() });
                        }
                        if pending.requires_full || pending.selection.request_proof_version >= 4 {
                            let delivered = manifest.reconstruct(|id| blobs.read_blob(id))
                                .map_err(|error| SessionError::TaskContext { message: format!("primary delivery read failed: {error:?}") })?;
                            if pending.requires_full && (manifest.delivery().mode() != crate::domain::request_audit::RequestAuditDeliveryMode::Full
                                || delivered != plan.model_input())
                            {
                                return Err(SessionError::TaskContext { message: "new task epoch did not begin with the independently compiled Full input".to_owned() });
                            }
                            if let Some(delivery) = &mut pending.delivery {
                                delivery.verify_attempt(&manifest, &delivered, replay_inputs.expect("proof4 compiles replay inputs"), &pending.capture.common.capabilities)
                                    .map_err(|error| SessionError::TaskContext { message: format!("primary delivery differs from independently reconstructed source and baseline: {error}") })?;
                            }
                        }
                        pending.first = false;
                    }
                    if !recent_only && event.event_type.as_str() == "session.model_delivery_committed" && let SessionEvent::ModelDeliveryCommitted { delivery } = SessionEvent::decode(&event.event_type, event.payload.clone())?
                        && pending_task_plan.as_ref().is_some_and(|pending| {
                            state.request_audits().iter().any(|audit| audit.audit_event_id() == delivery.audit_event_id() && audit.provider_session() == Some(&pending.session))
                        })
                    {
                        if pending_task_plan.as_ref().is_some_and(|pending| pending.selection.request_proof_version >= 4) {
                            let (blob, step) = delivery.output_source().ok_or_else(|| SessionError::TaskContext { message: "accepted task delivery has no output source".to_owned() })?;
                            let bytes = blobs.read_blob(blob).map_err(|error| SessionError::TaskContext { message: format!("accepted output read failed: {error:?}") })?;
                            let output: Vec<crate::domain::model::ContentItem> = serde_json::from_slice(&bytes).map_err(|error| SessionError::TaskContext { message: format!("accepted output decode failed: {error}") })?;
                            let output_history_items = state.verify_model_delivery_output(&output)?;
                            pending_task_plan.as_mut().expect("proof4 pending task exists").delivery.as_mut().expect("proof4 delivery replay exists")
                                .accept(delivery.audit_event_id(), step, output, output_history_items)
                                .map_err(|error| SessionError::TaskContext { message: format!("accepted task delivery reconstruction failed: {error}") })?;
                        }
                        if pending_task_plan.as_ref().is_some_and(|pending| pending.selection.request_proof_version == 0) {
                            pending_task_plan = None;
                        } else if let Some(pending) = &mut pending_task_plan {
                            pending.requires_full = false;
                        }
                    }
                    if event.event_type.as_str() == "session.compaction_created" && let SessionEvent::CompactionCreated { compaction } =
                        SessionEvent::decode(&event.event_type, event.payload.clone())?
                        && compaction.summary().graph_reference().is_some()
                    {
                        let research = compaction.summary().schema_version() == 7
                            && state.agent_definition_binding().is_some_and(|binding| binding.definition().research().is_some());
                        if research && research_references.is_none() { research_references = Some(load_research_references()?); }
                        let references = if research { research_references.as_ref().expect("Research catalog was loaded") } else { &empty_references };
                        let prior_graph = state.latest_compaction()
                            .and_then(|prior| prior.summary().graph_reference())
                            .and_then(|(root, _, _, _)| graphs.get(root));
                        let graph = reconstruct_summary_graph(
                            compaction.summary(),
                            &events[..index],
                            project_events,
                            references,
                            blobs,
                            prior_graph,
                        )?;
                        compaction.summary().validate_graph_reference(&graph)?;
                        graphs.insert(graph.root_event_id.clone(), graph);
                    }
                    index += 1;
                    Ok(())
                },
            )?;
        if let Some(root) = state
            .latest_compaction()
            .and_then(|value| value.summary().graph_reference())
            .map(|(root, _, _, _)| root)
        {
            let facts = compaction_source.take().ok_or_else(|| {
                SessionError::CompactionContextProvenance {
                    message: "latest v6 source facts are absent".to_owned(),
                }
            })?;
            let graph =
                graphs
                    .remove(root)
                    .ok_or_else(|| SessionError::CompactionContextProvenance {
                        message: "latest v6 graph was not verified".to_owned(),
                    })?;
            compaction_source = Some(facts.with_verified_graph(graph));
        }
        for id in state.legacy_vision_audit_ids()? {
            let audit = state
                .request_audits()
                .iter()
                .find(|audit| audit.audit_event_id() == &id)
                .ok_or_else(|| SessionError::CompactionContextProvenance {
                    message: "legacy Vision audit is absent".to_owned(),
                })?;
            audit
                .reconstruct(|blob| blobs.read_blob(blob))
                .map_err(|error| SessionError::CompactionContextProvenance {
                    message: format!("legacy Vision audit reconstruction failed: {error:?}"),
                })?;
        }
        verify_protected_vision_sources(&state, blobs)?;
        if let Some(last) = events.last() {
            prefix_hash
                .push(last)
                .map_err(|error| source_encoding_error(last, error))?;
        }
        Ok(Self {
            reducer,
            state,
            compaction_source,
            current_source,
            resume_source: if recent_only {
                pending_task_plan.map(|pending| (pending.selection.source, pending.source))
            } else {
                None
            },
            source_hash: prefix_hash,
            task_source_events: events
                .iter()
                .filter(|event| task_source_event(event))
                .cloned()
                .map(std::sync::Arc::new)
                .collect(),
        })
    }

    /// External suffixes with task delivery facts require the same Project and
    /// Blob-backed source reconstruction as an initial Session load.
    pub(crate) fn suffix_requires_project_replay(
        &self,
        events: &[EventEnvelope<serde_json::Value>],
    ) -> Result<bool, SessionError> {
        use crate::domain::session::SessionEvent;
        let proof4 = self
            .state
            .latest_task_context_selection()
            .is_some_and(|(_, selection)| selection.request_proof_version >= 4);
        for event in events {
            match SessionEvent::decode(&event.event_type, event.payload.clone())? {
                SessionEvent::TaskContextSelected { .. } => return Ok(true),
                SessionEvent::CompactionCreated { compaction }
                    if compaction.summary().graph_reference().is_some() =>
                {
                    return Ok(true);
                }
                SessionEvent::RequestAuditRecorded { manifest }
                    if proof4
                        && manifest.provider_session().is_some_and(|scope| {
                            scope.role == crate::domain::context::ProviderSessionRole::Primary
                        }) =>
                {
                    return Ok(true);
                }
                SessionEvent::ModelDeliveryCommitted { .. } if proof4 => return Ok(true),
                _ => {}
            }
        }
        Ok(false)
    }

    pub(crate) fn ingest_durable_verified<Store: crate::ports::artifact::ArtifactStore>(
        &mut self,
        events: Vec<EventEnvelope<serde_json::Value>>,
        blobs: &Store,
    ) -> Result<(), SessionError>
    where
        Store::Error: std::fmt::Debug,
    {
        if events.is_empty() {
            return Ok(());
        }
        let mut state = self.state.clone();
        let mut source_hash = self.source_hash.clone();
        let mut source_events = Vec::new();
        for event in events {
            let retained = task_source_event(&event).then(|| std::sync::Arc::new(event.clone()));
            let mut next_hash = source_hash.clone();
            next_hash
                .push(&event)
                .map_err(|error| source_encoding_error(&event, error))?;
            if self.apply_verified_event(&mut state, event, blobs)?
                != ApplyOutcome::DuplicateIgnored
            {
                source_hash = next_hash;
                source_events.extend(retained);
            }
        }
        self.state = state;
        self.source_hash = source_hash;
        self.task_source_events.extend(source_events);
        self.current_source = None;
        Ok(())
    }

    fn apply_verified_event<Store: crate::ports::artifact::ArtifactStore>(
        &self,
        state: &mut SessionState,
        event: EventEnvelope<serde_json::Value>,
        blobs: &Store,
    ) -> Result<ApplyOutcome, SessionError>
    where
        Store::Error: std::fmt::Debug,
    {
        verify_completed_event(state, &event, blobs)?;
        if matches!(
            crate::domain::session::SessionEvent::decode(&event.event_type, event.payload.clone())?,
            crate::domain::session::SessionEvent::TaskContextSelected { .. }
        ) {
            return Err(SessionError::TaskContext {
                message: "task selection requires project-aware replay".to_owned(),
            });
        }
        if let crate::domain::session::SessionEvent::CompactionCreated { compaction } =
            crate::domain::session::SessionEvent::decode(&event.event_type, event.payload.clone())?
            && compaction.summary().graph_reference().is_some()
        {
            return Err(SessionError::CompactionContextProvenance {
                message: "v6 Compaction requires project-aware replay".to_owned(),
            });
        }
        self.reducer.apply_in_place(state, event)
    }

    /// Resolve and durably apply the original domain-then-phase boundary from
    /// one synchronized projection. UI supplies identity/time, never policy.
    pub(crate) fn prepare_research_turn<S: EventSink<serde_json::Value>>(
        &mut self,
        sink: &mut S,
        stream: crate::domain::events::StreamId,
        request: ResearchTurnRequest,
        mut stamp: impl FnMut(
            crate::domain::events::EventSequence,
            &'static str,
        ) -> (
            crate::domain::events::EventId,
            crate::domain::events::OccurredAt,
        ),
    ) -> Result<
        (
            crate::domain::domain_profile::DomainProfile,
            crate::domain::research_control::ResearchPhase,
        ),
        AppendAfterDurableError<S::Error>,
    > {
        use crate::domain::domain_profile::{DomainProfile, DomainProfileSelection};
        use crate::domain::research_control::{ResearchPhase, ResearchPhaseSelection};
        use crate::domain::session::SessionEvent;
        let (domain, domain_change) = match request.domain {
            Some(selection) => (selection.profile(), Some(selection)),
            None => match self.state.domain_profile_selection() {
                Some(selection) => (selection.profile(), None),
                None => {
                    let selection =
                        if request.started || request.default_domain != DomainProfile::Source {
                            DomainProfileSelection::definition_default(request.default_domain)
                        } else {
                            DomainProfileSelection::legacy_default()
                        };
                    (selection.profile(), Some(selection))
                }
            },
        };
        let (phase, phase_change) = match request.phase {
            Some(selection) => (selection.phase(), Some(selection)),
            None => match self.state.research_phase_selection() {
                Some(selection) => (selection.phase(), None),
                None => {
                    let selection =
                        if request.started || request.default_phase != ResearchPhase::Mapping {
                            ResearchPhaseSelection::definition_default(request.default_phase)
                        } else {
                            ResearchPhaseSelection::legacy_default()
                        };
                    (selection.phase(), Some(selection))
                }
            },
        };
        for (event, kind) in [
            domain_change.map(|selection| {
                (
                    SessionEvent::DomainProfileSelected { selection },
                    "domain-profile",
                )
            }),
            phase_change.map(|selection| {
                (
                    SessionEvent::ResearchPhaseSelected { selection },
                    "research-phase",
                )
            }),
        ]
        .into_iter()
        .flatten()
        {
            let sequence = self
                .next_event_sequence()
                .map_err(AppendAfterDurableError::Session)?;
            let (id, at) = stamp(sequence, kind);
            let event = event
                .into_envelope(id, stream.clone(), sequence, at)
                .map_err(AppendAfterDurableError::Session)?;
            self.append(sink, event)?;
        }
        Ok((domain, phase))
    }

    /// Strictly reconstructs Session State from durable Events supplied by an
    /// EventSource at the composition boundary.
    pub fn replay<Events>(events: Events) -> Result<Self, SessionError>
    where
        Events: IntoIterator<Item = EventEnvelope<serde_json::Value>>,
    {
        let reducer = SessionReducer;
        let events = events.into_iter().collect::<Vec<_>>();
        let mut source_hash = crate::domain::canonical_json::CanonicalArrayHasher::new();
        for event in &events {
            source_hash
                .push(event)
                .map_err(|error| source_encoding_error(event, error))?;
        }
        let state = reducer.reduce(events.iter().cloned())?;
        Ok(Self {
            reducer,
            state,
            compaction_source: None,
            current_source: None,
            resume_source: None,
            source_hash,
            task_source_events: events
                .into_iter()
                .filter(task_source_event)
                .map(std::sync::Arc::new)
                .collect(),
        })
    }

    pub(crate) fn task_source_identity(
        &self,
        project_id: crate::domain::project::ProjectId,
        project_events: &[EventEnvelope<serde_json::Value>],
    ) -> Result<crate::domain::session::SessionSourceIdentity, SessionError> {
        crate::domain::session::SessionSourceIdentity::from_tracked_prefix(
            project_id,
            &self.state,
            self.source_hash.finish(),
            project_events,
        )
    }

    pub(crate) fn task_source_events(&self) -> &[std::sync::Arc<EventEnvelope<serde_json::Value>>] {
        &self.task_source_events
    }

    pub(crate) fn current_source(&self) -> Option<&crate::domain::session::SessionSourceIdentity> {
        self.current_source.as_ref()
    }

    pub(crate) fn take_resume_source(
        &mut self,
    ) -> Option<(crate::domain::session::SessionSourceIdentity, SessionState)> {
        self.resume_source.take()
    }

    pub(crate) fn compaction_source_for(
        &self,
        summary: &crate::domain::session::SessionSummary,
    ) -> Option<&crate::domain::session::TaskCompactionValidationFacts> {
        self.compaction_source.as_ref().filter(|facts| {
            summary
                .task_references()
                .is_some_and(|refs| refs.source() == facts.source())
        })
    }

    pub(crate) fn retain_compaction_source(
        &mut self,
        facts: crate::domain::session::TaskCompactionValidationFacts,
    ) -> Result<(), SessionError> {
        if self
            .state
            .latest_compaction()
            .and_then(|value| value.summary().task_references())
            .is_none_or(|refs| refs.source() != facts.source())
        {
            return Err(SessionError::CompactionContextProvenance {
                message: "retained source does not match the durable Compaction".to_owned(),
            });
        }
        self.compaction_source = Some(facts);
        Ok(())
    }

    pub fn state(&self) -> &SessionState {
        &self.state
    }

    pub(crate) fn next_event_sequence(
        &self,
    ) -> Result<crate::domain::events::EventSequence, SessionError> {
        self.state.next_event_sequence()
    }

    /// Persists the supplied complete Event through the caller's EventSink.
    /// State promotion remains conditional on a successful durable append.
    pub fn append<S>(
        &mut self,
        sink: &mut S,
        event: EventEnvelope<serde_json::Value>,
    ) -> Result<ApplyOutcome, AppendAfterDurableError<S::Error>>
    where
        S: EventSink<serde_json::Value>,
    {
        let mut source_hash = self.source_hash.clone();
        source_hash.push(&event).map_err(|error| {
            AppendAfterDurableError::Session(source_encoding_error(&event, error))
        })?;
        let retained = task_source_event(&event).then(|| std::sync::Arc::new(event.clone()));
        let outcome = append_after_durable(&self.reducer, &mut self.state, sink, event)?;
        if outcome != ApplyOutcome::DuplicateIgnored {
            self.current_source = None;
            self.source_hash = source_hash;
            self.task_source_events.extend(retained);
        }
        Ok(outcome)
    }
}

fn source_encoding_error(
    event: &EventEnvelope<serde_json::Value>,
    error: serde_json::Error,
) -> SessionError {
    SessionError::PayloadEncode {
        event_type: event.event_type.clone(),
        message: error.to_string(),
    }
}

/// Current-task compilation needs History IDs, recovery boundaries and argument
/// feedback. Historical request/response payloads remain in their durable owner.
fn task_source_event(event: &EventEnvelope<serde_json::Value>) -> bool {
    matches!(
        event.event_type.as_str(),
        "session.history_item_recorded"
            | "session.resumed"
            | "session.agent_definition_applied"
            | "session.compaction_created"
            | "model.sample_rejected"
    )
}

#[allow(clippy::too_many_arguments)]
fn verify_task_context_selection<Store: crate::ports::artifact::ArtifactStore>(
    state: &SessionState,
    selection: &crate::domain::session::TaskContextSelected,
    events: &[EventEnvelope<serde_json::Value>],
    project_events: &[EventEnvelope<serde_json::Value>],
    research_references: &crate::domain::research::ResearchReferenceCatalog,
    blobs: &Store,
    project_id: &crate::domain::project::ProjectId,
    checkpoint: Option<(&SessionState, &str, &str)>,
) -> Result<
    (
        crate::domain::context::ContextPlan,
        bool,
        SessionState,
        crate::domain::session::TaskContextCapture,
        Vec<crate::domain::task_context_research::ResearchTaskUnit>,
    ),
    SessionError,
>
where
    Store::Error: std::fmt::Debug,
{
    use crate::domain::session::{SessionEvent, SessionSourceIdentity, TaskContextCapture};
    let invalid = |message: String| SessionError::TaskContext { message };
    let end = events.partition_point(|event| event.sequence <= selection.source.watermark());
    let project_end = project_events.partition_point(|event| {
        event.sequence.get() <= selection.source.project_stream_watermark()
    });
    let capture_event = events
        .get(
            end.checked_sub(1)
                .ok_or_else(|| invalid("task source is empty".to_owned()))?,
        )
        .ok_or_else(|| invalid("task context capture is absent".to_owned()))?;
    if capture_event.event_id != selection.capture_event_id
        || !matches!(SessionEvent::decode(&capture_event.event_type, capture_event.payload.clone())?,
            SessionEvent::TaskContextCaptured { blob_id } if blob_id == selection.capture_blob)
    {
        return Err(invalid(
            "task context capture does not match its source prefix".to_owned(),
        ));
    }
    let bytes = blobs
        .read_blob(&selection.capture_blob)
        .map_err(|error| invalid(format!("task context capture read failed: {error:?}")))?;
    let capture: TaskContextCapture = serde_json::from_slice(&bytes)
        .map_err(|error| invalid(format!("task context capture is invalid: {error}")))?;
    let owned_source_state;
    let (source, source_state) =
        if let Some((source_state, session_hash, project_hash)) = checkpoint {
            let project_watermark = project_events[..project_end]
                .last()
                .map_or(0, |event| event.sequence.get());
            if selection.source.project_id() != project_id
                || source_state.stream_id() != Some(selection.source.stream_id())
                || capture_event.sequence != selection.source.watermark()
                || &capture_event.stream_id != selection.source.stream_id()
                || session_hash != selection.source.stream_hash()
                || project_watermark != selection.source.project_stream_watermark()
                || project_hash != selection.source.project_stream_hash()
            {
                return Err(invalid("task context source prefix changed".to_owned()));
            }
            (selection.source.clone(), source_state)
        } else {
            // Older accepted streams can reference a prior capture. Rebuild that
            // exact prefix rather than treating the latest checkpoint as its source.
            let (source, state) = SessionSourceIdentity::from_events(
                selection.source.project_id().clone(),
                &events[..end],
                &project_events[..project_end],
            )?;
            owned_source_state = state;
            (source, &owned_source_state)
        };
    if capture.compile.research_enabled
        != source_state
            .agent_definition_binding()
            .is_some_and(|binding| binding.definition().research().is_some())
    {
        return Err(invalid(
            "Research capture does not match the bound AgentDefinition".to_owned(),
        ));
    }
    let (research_candidates, research_omitted) = source_state.task_research_candidates(
        source.project_id().as_str(),
        &selection.input_ref,
        source.project_stream_hash(),
        &capture.compile,
        &project_events[..project_end],
        research_references,
        capture.expansion.as_ref(),
        capture.source_refresh,
    )?;
    if research_candidates != capture.research_candidates
        || research_omitted != capture.research_omitted
    {
        return Err(invalid(
            "Research navigation differs from its Project source".to_owned(),
        ));
    }
    let mut expected_candidates = source_state.task_selection_candidates_with_capture(
        source.project_id().as_str(),
        &selection.input_ref,
        &capture.common,
        &capture.research_candidates,
        capture.research_omitted,
        &capture.memory,
        capture.expansion.as_ref(),
        capture.source_refresh,
        capture.candidate_policy_version,
        &crate::app::context_read::history_navigation_excerpt,
    )?;
    if selection.request_proof_version >= 5 {
        source_state
            .append_current_visual_candidates(&selection.input_ref, &mut expected_candidates)?;
    }
    if (selection.request_proof_version >= 4 && capture.schema_version != 2)
        || (selection.request_proof_version < 4 && capture.schema_version != 1)
        || capture.candidate_policy_version
            != if selection.request_proof_version >= 7 {
                4
            } else if selection.request_proof_version >= 6 {
                3
            } else if selection.request_proof_version >= 5 {
                2
            } else {
                1
            }
        || capture.source_refresh != selection.refresh_of.is_some()
        || (capture.source_refresh && capture.expansion.is_some())
        || source != selection.source
        || expected_candidates != capture.candidates
    {
        return Err(invalid(
            "task context capture does not match its original source".to_owned(),
        ));
    }
    let fragments = capture.common.fragments()?;
    if capture.schema_version == 2 {
        use crate::domain::context::ContextFragmentSource;
        let project_fragments = fragments
            .iter()
            .filter(|fragment| fragment.source() == ContextFragmentSource::ProjectSnapshot)
            .collect::<Vec<_>>();
        let expected = capture
            .compile
            .project_snapshot
            .as_ref()
            .map(|snapshot| {
                if &snapshot.project_id != source.project_id() {
                    return Err(invalid(
                        "captured Project Snapshot belongs to a different Project".to_owned(),
                    ));
                }
                let context =
                    crate::domain::world_state::ProjectSnapshotContext::new(snapshot.clone())
                        .map_err(|error| {
                            invalid(format!("captured Project Snapshot is invalid: {error}"))
                        })?;
                let policy = capture
                    .common
                    .policy
                    .for_capabilities(&capture.common.capabilities)
                    .map_err(|error| {
                        invalid(format!(
                            "captured Project Snapshot budget is invalid: {error}"
                        ))
                    })?;
                context
                    .full_fragment_with_limit(policy.state_section_bytes())
                    .map_err(|error| {
                        invalid(format!(
                            "captured Project Snapshot rendering failed: {error}"
                        ))
                    })
            })
            .transpose()?;
        if project_fragments.as_slice()
            != expected.as_ref().into_iter().collect::<Vec<_>>().as_slice()
        {
            return Err(invalid(
                "captured Project Snapshot differs from its common fragment".to_owned(),
            ));
        }
    } else if capture.compile.project_snapshot.is_some() {
        return Err(invalid(
            "legacy task capture contains a new Project Snapshot source".to_owned(),
        ));
    }
    let mut selected_research = capture
        .candidates
        .context
        .iter()
        .filter_map(|candidate| {
            candidate
                .research_ref
                .as_ref()
                .filter(|_| {
                    selection
                        .decision
                        .required_context_refs
                        .contains(&candidate.reference)
                })
                .cloned()
        })
        .collect::<Vec<_>>();
    selected_research.extend(
        source_state
            .selected_summary_research_refs(&selection.decision, source.project_stream_hash())?,
    );
    selected_research.sort();
    selected_research.dedup();
    let research_units = source_state.task_research_units(
        source.project_stream_hash(),
        &capture.compile,
        &project_events[..project_end],
        research_references,
        &selected_research,
    )?;
    let (mut expected, _) = source_state.prepare_task_selection(
        source.clone(),
        selection.capture_event_id.clone(),
        selection.capture_blob.clone(),
        selection.input_ref.clone(),
        selection.sample_event_id.clone(),
        &capture.candidates,
        &research_units,
        &capture.compile.live_processes,
        selection.decision.clone(),
    )?;
    if let Some(refresh_of) = &selection.refresh_of {
        let (id, decision) = source_state.deterministic_task_refresh(
            &selection.input_ref,
            &capture.candidates,
            selection.has_complete_request_proof(),
            selection.request_proof_version,
        )?;
        let previous = source_state
            .latest_task_context_selection()
            .ok_or_else(|| invalid("source refresh prior selection is absent".to_owned()))?
            .1;
        if &id != refresh_of
            || decision != selection.decision
            || selection.sample_event_id.is_some()
        {
            return Err(invalid(
                "source refresh differs from its independently derived dependencies".to_owned(),
            ));
        }
        expected.refresh_of = Some(id);
        expected.task_revisions = previous.task_revisions.clone();
    }
    expected.request_proof_version = selection.request_proof_version;
    expected.planned_context_hash = selection.planned_context_hash.clone();
    if expected != *selection {
        return Err(invalid(
            "task selection differs from its independently reconstructed sources".to_owned(),
        ));
    }
    if let Some(sample_id) = &selection.sample_event_id {
        let sample = state
            .task_selection_sample(sample_id)
            .ok_or_else(|| invalid("selection sample is absent".to_owned()))?;
        let audit = state
            .request_audits()
            .iter()
            .find(|audit| audit.audit_event_id() == &sample.audit_event_id)
            .ok_or_else(|| invalid("selection audit is absent".to_owned()))?;
        let planner = crate::domain::context::ContextPlanner::current()
            .with_budget_policy(capture.common.policy);
        let plan = crate::app::agent_loop::compile_selection(
            &planner,
            &capture
                .compile
                .selection_capabilities(&capture.common.capabilities),
            &source,
            &capture.candidates,
            capture.compile.selection_prompt_version,
        )
        .map_err(|error| invalid(format!("selection source compilation failed: {error}")))?;
        let actual = audit
            .reconstruct_full_logical(|id| blobs.read_blob(id))
            .map_err(|error| invalid(format!("selection audit read failed: {error:?}")))?
            .ok_or_else(|| invalid("selection audit has no full logical input".to_owned()))?;
        if actual.input() != &plan.model_input() || actual.delivery_map() != plan.delivery_map() {
            return Err(invalid(
                "selection audit differs from independently compiled source".to_owned(),
            ));
        }
    }
    let requires_full = source_state.primary_provider_session()
        != Some(&selection.provider_session)
        || !source_state.has_primary_model_delivery(&selection.provider_session);
    let plan = crate::app::agent_loop::compile_captured_task_context(
        source_state,
        &capture,
        selection,
        events[..end].iter(),
        &research_units,
    )
    .map_err(|error| invalid(format!("primary task source compilation failed: {error}")))?;
    Ok((
        plan,
        requires_full,
        source_state.clone(),
        capture,
        research_units,
    ))
}

fn verify_completed_event<Store: crate::ports::artifact::ArtifactStore>(
    state: &SessionState,
    event: &EventEnvelope<serde_json::Value>,
    blobs: &Store,
) -> Result<(), SessionError>
where
    Store::Error: std::fmt::Debug,
{
    use crate::domain::session::{OperatorTurnBoundary, SessionEvent};
    if event.event_type.as_str() != "session.operator_turn_boundary_recorded" {
        return Ok(());
    }
    if let SessionEvent::OperatorTurnBoundaryRecorded {
        boundary: boundary @ OperatorTurnBoundary::Completed { .. },
    } = SessionEvent::decode(&event.event_type, event.payload.clone())?
    {
        let audit = state.request_audits().last().ok_or_else(|| {
            SessionError::CompactionContextProvenance {
                message: "completed boundary has no request audit".to_owned(),
            }
        })?;
        let proof = audit
            .reconstruct_full_logical(|id| blobs.read_blob(id))
            .map_err(|error| SessionError::CompactionContextProvenance {
                message: format!("completed audit reconstruction failed: {error:?}"),
            })?
            .ok_or_else(|| SessionError::CompactionContextProvenance {
                message: "completed audit has no verified full logical input".to_owned(),
            })?;
        let mut delivered = std::collections::BTreeMap::new();
        for delivery in state
            .model_delivery_chain(audit.audit_event_id())?
            .ok_or_else(|| SessionError::CompactionContextProvenance {
                message: "completed boundary has no accepted delivery chain".to_owned(),
            })?
        {
            let step_audit = state
                .request_audits()
                .iter()
                .find(|candidate| candidate.audit_event_id() == delivery.audit_event_id())
                .ok_or_else(|| SessionError::CompactionContextProvenance {
                    message: "accepted delivery audit is absent".to_owned(),
                })?;
            let input = step_audit
                .reconstruct(|id| blobs.read_blob(id))
                .map_err(|error| SessionError::CompactionContextProvenance {
                    message: format!("accepted delivery reconstruction failed: {error:?}"),
                })?;
            delivered.insert(delivery.audit_event_id().clone(), input);
        }
        state.verify_completed_operator_boundary(&boundary, &proof, &delivered)?;
    }
    Ok(())
}

fn reconstruct_summary_graph<Store: crate::ports::artifact::ArtifactStore>(
    summary: &crate::domain::session::SessionSummary,
    events: &[EventEnvelope<serde_json::Value>],
    project_events: &[EventEnvelope<serde_json::Value>],
    research_references: &crate::domain::research::ResearchReferenceCatalog,
    blobs: &Store,
    prior_graph: Option<&crate::domain::session::VerifiedCompactionGraph>,
) -> Result<crate::domain::session::VerifiedCompactionGraph, SessionError>
where
    Store::Error: std::fmt::Debug,
{
    use crate::domain::session::{CompactionSourcePlan, SessionEvent};
    let invalid = |message: String| SessionError::CompactionContextProvenance { message };
    let (root, source, blob, expected_hash) = summary
        .graph_reference()
        .ok_or_else(|| invalid("Summary has no graph reference".to_owned()))?;
    let bytes = blobs
        .read_blob(blob)
        .map_err(|error| invalid(format!("Compaction plan Blob read failed: {error:?}")))?;
    let plan: CompactionSourcePlan =
        serde_json::from_slice(&bytes).map_err(|error| invalid(error.to_string()))?;
    if plan.source() != source || plan.hash()? != expected_hash {
        return Err(invalid("Compaction plan identity mismatch".to_owned()));
    }
    let root_index = events
        .iter()
        .position(|event| &event.event_id == root)
        .ok_or_else(|| invalid("Compaction root event is absent".to_owned()))?;
    let mut end = root_index + 1;
    if let Some(event) = events.get(end)
        && matches!(
            SessionEvent::decode(&event.event_type, event.payload.clone())?,
            SessionEvent::ModelUsageRecorded { .. }
        )
    {
        end += 1;
    }
    let operation = &events[..end];
    let boundary = operation.partition_point(|event| event.sequence <= source.watermark());
    let capture_blob = match operation.get(boundary.saturating_sub(1)) {
        Some(event) => match SessionEvent::decode(&event.event_type, event.payload.clone())? {
            SessionEvent::CompactionContextCaptured { blob_id } => blob_id,
            _ => {
                return Err(invalid(
                    "Compaction context capture is absent at source watermark".to_owned(),
                ));
            }
        },
        None => return Err(invalid("Compaction context capture is absent".to_owned())),
    };
    let capture_bytes = blobs
        .read_blob(&capture_blob)
        .map_err(|error| invalid(format!("Compaction context Blob read failed: {error:?}")))?;
    let capture: crate::domain::session::CompactionContextCapture =
        serde_json::from_slice(&capture_bytes)
            .map_err(|error| invalid(format!("Compaction context Blob invalid: {error}")))?;
    let source_state = SessionReducer.reduce(operation[..boundary].iter().cloned())?;
    match plan.task_selection_event() {
        Some(expected) => {
            let (id, selection) = source_state
                .latest_task_context_selection()
                .ok_or_else(|| invalid("task Compaction selection is absent".to_owned()))?;
            if id != expected {
                return Err(invalid("task Compaction selection changed".to_owned()));
            }
            let bytes = blobs
                .read_blob(&selection.capture_blob)
                .map_err(|error| invalid(format!("task capture read failed: {error:?}")))?;
            let task_capture: crate::domain::session::TaskContextCapture =
                serde_json::from_slice(&bytes)
                    .map_err(|error| invalid(format!("task capture is invalid: {error}")))?;
            let end = project_events.partition_point(|event| {
                event.sequence.get() <= selection.source.project_stream_watermark()
            });
            let hash = crate::domain::session::SessionSourceIdentity::project_hash_for_events(
                &project_events[..end],
            )?;
            if hash != selection.source.project_stream_hash() {
                return Err(invalid("selected Research source changed".to_owned()));
            }
            let units = source_state.task_research_units(
                &hash,
                &task_capture.compile,
                &project_events[..end],
                research_references,
                &selection.selected_research,
            )?;
            crate::app::agent_loop::verify_task_compaction_capture(
                &source_state,
                selection,
                &operation[..boundary],
                &task_capture,
                &capture,
                &units,
            )
            .map_err(|error| invalid(format!("task Compaction capture mismatch: {error}")))?;
        }
        None if capture.is_task_scoped() => {
            return Err(invalid(
                "legacy Compaction has task-scoped capture".to_owned(),
            ));
        }
        None => {}
    }
    let references = source_state
        .history_items_with_event_ids()
        .map(|(id, _)| id.clone())
        .rev()
        .take(capture.reference_limit)
        .collect::<Vec<_>>();
    let project_end = project_events
        .partition_point(|event| event.sequence.get() <= source.project_stream_watermark());
    let project_prefix = &project_events[..project_end];
    let (entries, contents) = match plan.task_selection_event() {
        Some(event) => CompactionSourcePlan::source_entries_for_selection(
            source,
            &operation[..boundary],
            project_prefix,
            Some(event),
        )?,
        None => {
            CompactionSourcePlan::source_entries(source, &operation[..boundary], project_prefix)?
        }
    };
    if entries != plan.entries() {
        return Err(invalid("Compaction source manifest changed".to_owned()));
    }
    let mut audits = std::collections::BTreeMap::new();
    let mut proofs = std::collections::BTreeMap::new();
    let mut expected = std::collections::BTreeMap::new();
    let mut children = std::collections::BTreeMap::new();
    for event in &operation[boundary..] {
        match SessionEvent::decode(&event.event_type, event.payload.clone())? {
            SessionEvent::RequestAuditRecorded { manifest } => {
                audits.insert(event.event_id.clone(), manifest);
            }
            SessionEvent::CompactionSampleRecorded { sample } => {
                let (_, _, node, output) = sample
                    .graph_node()
                    .ok_or_else(|| invalid("legacy sample in Compaction graph".to_owned()))?;
                let audit = audits
                    .get(sample.audit_event_id())
                    .ok_or_else(|| invalid("Compaction node audit absent".to_owned()))?;
                let proof = audit
                    .reconstruct_full_logical(|id| blobs.read_blob(id))
                    .map_err(|error| {
                        invalid(format!(
                            "Compaction node audit reconstruction failed: {error:?}"
                        ))
                    })?
                    .ok_or_else(|| invalid("Compaction node full audit unavailable".to_owned()))?;
                proofs.insert(sample.audit_event_id().clone(), proof);
                let inputs = match &node.input {
                    crate::domain::session::CompactionSourceNodeInput::Source(_) => {
                        std::collections::BTreeMap::new()
                    }
                    crate::domain::session::CompactionSourceNodeInput::Merge(ids) => ids
                        .iter()
                        .map(|id| {
                            children
                                .get(id)
                                .cloned()
                                .map(|value| (id.clone(), value))
                                .ok_or_else(|| {
                                    invalid("Compaction child sample is absent".to_owned())
                                })
                        })
                        .collect::<Result<std::collections::BTreeMap<_, _>, _>>()?,
                };
                let source_data = plan.render_node(&node.id, &contents, &inputs)?;
                let compiled = crate::app::agent_loop::compile_captured_compaction_source(
                    &capture,
                    source,
                    &source_data,
                    &references,
                    node.output_limit,
                )
                .map_err(|error| {
                    invalid(format!("Compaction context reconstruction failed: {error}"))
                })?;
                expected.insert(
                    node.id.clone(),
                    (compiled.model_input(), compiled.delivery_map().clone()),
                );
                children.insert(node.id.clone(), (event.event_id.clone(), output.to_owned()));
            }
            SessionEvent::ModelUsageRecorded { .. } => {}
            _ => {
                return Err(invalid(
                    "unrelated Session event in Compaction graph".to_owned(),
                ));
            }
        }
    }
    let task_fragments = plan
        .task_selection_event()
        .map(|_| capture.fragments())
        .transpose()?;
    let graph = plan.verify_model_node_audits(
        blob,
        &contents,
        operation,
        project_prefix,
        &proofs,
        &expected,
        &capture.capabilities,
        task_fragments.as_deref(),
        capture.reference_limit,
        prior_graph,
    )?;
    verify_compaction_source_blobs(&graph, blobs)?;
    Ok(graph)
}

/// A Session append failed either before durable persistence or while
/// persisting the Event. It intentionally retains the Sink's typed error.
#[derive(Debug)]
pub enum AppendAfterDurableError<E> {
    Session(SessionError),
    Sink(E),
}
