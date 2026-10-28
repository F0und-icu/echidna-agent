//! Append-before-apply Session composition through Event ports.

use crate::domain::events::{ApplyOutcome, EventEnvelope};
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
        blobs: &Store,
    ) -> Result<Self, SessionError>
    where
        Store::Error: std::fmt::Debug,
    {
        use crate::domain::session::SessionEvent;
        let mut graphs = std::collections::BTreeMap::new();
        let reducer = SessionReducer;
        let mut index = 0usize;
        let (state, mut compaction_source, current_source) = reducer
            .reduce_with_compaction_source_checked(
                project_id,
                events,
                project_events,
                |state, event| {
                    verify_completed_event(state, event, blobs)?;
                    if let SessionEvent::CompactionCreated { compaction } =
                        SessionEvent::decode(&event.event_type, event.payload.clone())?
                        && compaction.summary().graph_reference().is_some()
                    {
                        let graph = reconstruct_summary_graph(
                            compaction.summary(),
                            &events[..index],
                            project_events,
                            blobs,
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
        Ok(Self {
            reducer,
            state,
            compaction_source,
            current_source,
        })
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
        for event in events {
            self.apply_verified_event(&mut state, event, blobs)?;
        }
        self.state = state;
        self.current_source = None;
        Ok(())
    }

    fn apply_verified_event<Store: crate::ports::artifact::ArtifactStore>(
        &self,
        state: &mut SessionState,
        event: EventEnvelope<serde_json::Value>,
        blobs: &Store,
    ) -> Result<(), SessionError>
    where
        Store::Error: std::fmt::Debug,
    {
        verify_completed_event(state, &event, blobs)?;
        if let crate::domain::session::SessionEvent::CompactionCreated { compaction } =
            crate::domain::session::SessionEvent::decode(&event.event_type, event.payload.clone())?
            && compaction.summary().graph_reference().is_some()
        {
            return Err(SessionError::CompactionContextProvenance {
                message: "v6 Compaction requires project-aware replay".to_owned(),
            });
        }
        self.reducer.apply_in_place(state, event)?;
        Ok(())
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
        let state = reducer.reduce(events)?;
        Ok(Self {
            reducer,
            state,
            compaction_source: None,
            current_source: None,
        })
    }

    pub(crate) fn current_source(&self) -> Option<&crate::domain::session::SessionSourceIdentity> {
        self.current_source.as_ref()
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
        let outcome = append_after_durable(&self.reducer, &mut self.state, sink, event)?;
        if outcome != ApplyOutcome::DuplicateIgnored {
            self.current_source = None;
        }
        Ok(outcome)
    }
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
    blobs: &Store,
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
    let references = source_state
        .history_items_with_event_ids()
        .map(|(id, _)| id.clone())
        .rev()
        .take(capture.reference_limit)
        .collect::<Vec<_>>();
    let project_end = project_events
        .partition_point(|event| event.sequence.get() <= source.project_stream_watermark());
    let project_prefix = &project_events[..project_end];
    let (entries, contents) =
        CompactionSourcePlan::source_entries(source, &operation[..boundary], project_prefix)?;
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
                proofs.insert(sample.audit_event_id().clone(), proof);
            }
            SessionEvent::ModelUsageRecorded { .. } => {}
            _ => {
                return Err(invalid(
                    "unrelated Session event in Compaction graph".to_owned(),
                ));
            }
        }
    }
    let graph = plan.verify_model_node_audits(
        blob,
        &contents,
        operation,
        project_prefix,
        &proofs,
        &expected,
        &capture.capabilities,
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
