//! Append-before-apply Session composition through Event ports.

use crate::domain::events::{ApplyOutcome, EventEnvelope};
use crate::domain::session::{SessionError, SessionReducer, SessionState};
use crate::ports::EventSink;

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
        .apply(&mut next_state, event.clone())
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
    main_v5: Option<VerifiedMainV5>,
}

pub(crate) struct VerifiedMainV5 {
    project_id: crate::domain::project::ProjectId,
    research: crate::domain::session::ValidatedResearchSource,
    receipts: std::collections::BTreeSet<crate::domain::research::ExecutionReceiptId>,
    facts: crate::domain::session::MainV5ValidationFacts,
}

impl VerifiedMainV5 {
    pub(crate) fn research_source(&self) -> &crate::domain::session::ResearchSourceIdentity {
        self.research.identity()
    }

    pub(crate) fn graph(&self) -> &crate::domain::session::VerifiedCompactionGraph {
        &self.facts.graph
    }

    pub(crate) fn context(&self) -> crate::domain::session::CompactionValidationContext<'_> {
        let stream = match self.research.identity() {
            crate::domain::session::ResearchSourceIdentity::Empty { .. } => None,
            crate::domain::session::ResearchSourceIdentity::Nonempty { source } => Some(source),
        };
        crate::domain::session::CompactionValidationContext::for_main_v5(
            &self.project_id,
            stream,
            self.research.state(),
            &self.receipts,
            &self.facts,
        )
    }
}

impl SessionProjection {
    pub(crate) fn replay_project_verified<Store: crate::ports::artifact::ArtifactStore>(
        project_id: crate::domain::project::ProjectId,
        events: &[EventEnvelope<serde_json::Value>],
        research_events: &[EventEnvelope<serde_json::Value>],
        references: &crate::domain::research::ResearchReferenceCatalog,
        receipt_ids: &std::collections::BTreeSet<crate::domain::research::ExecutionReceiptId>,
        blobs: &Store,
    ) -> Result<Self, SessionError>
    where
        Store::Error: std::fmt::Debug,
    {
        use crate::domain::session::{
            CompactionSourcePlan, CompactionValidationContext, MainV5ValidationFacts, SessionEvent,
            ValidatedResearchSource,
        };
        let mut projection = Self {
            reducer: SessionReducer,
            state: SessionState::default(),
            main_v5: None,
        };
        for (index, event) in events.iter().enumerate() {
            verify_completed_event(&projection.state, event, blobs)?;
            if let SessionEvent::CompactionCreated { compaction } =
                SessionEvent::decode(&event.event_type, event.payload.clone())?
                && let Some((session_source, research_source)) =
                    compaction.summary().main_v5_sources()
            {
                let (_, _, plan_blob, _) = compaction
                    .summary()
                    .graph_reference()
                    .expect("main v5 has a graph reference");
                let bytes = blobs.read_blob(plan_blob).map_err(|error| {
                    SessionError::CompactionContextProvenance {
                        message: format!("Compaction plan Blob read failed: {error:?}"),
                    }
                })?;
                let plan: CompactionSourcePlan =
                    serde_json::from_slice(&bytes).map_err(|error| {
                        SessionError::CompactionContextProvenance {
                            message: format!("Compaction plan invalid: {error}"),
                        }
                    })?;
                let historical_events = research_prefix(plan.research_source(), research_events);
                let historical = ValidatedResearchSource::from_events(
                    project_id.clone(),
                    historical_events,
                    references,
                )?;
                if historical.identity() != plan.research_source() {
                    return Err(SessionError::CompactionSourceInvalid {
                        message: "historical Research source identity mismatch".to_owned(),
                    });
                }
                let graph = reconstruct_summary_graph(
                    compaction.summary(),
                    &events[..index],
                    historical_events,
                    &historical,
                    blobs,
                )?;
                let current_events = research_prefix(research_source, research_events);
                let current = ValidatedResearchSource::from_events(
                    project_id.clone(),
                    current_events,
                    references,
                )?;
                if current.identity() != research_source {
                    return Err(SessionError::CompactionSourceInvalid {
                        message: "Summary Research source identity mismatch".to_owned(),
                    });
                }
                let source_end = events[..index]
                    .partition_point(|value| value.sequence <= session_source.watermark());
                let facts = MainV5ValidationFacts::from_verified_sources(
                    project_id.clone(),
                    &events[..source_end],
                    &current,
                    graph,
                )?;
                let stream = match current.identity() {
                    crate::domain::session::ResearchSourceIdentity::Empty { .. } => None,
                    crate::domain::session::ResearchSourceIdentity::Nonempty { source } => {
                        Some(source)
                    }
                };
                let context = CompactionValidationContext::for_main_v5(
                    &project_id,
                    stream,
                    current.state(),
                    receipt_ids,
                    &facts,
                );
                compaction.validate_references(&context)?;
                projection.main_v5 = Some(VerifiedMainV5 {
                    project_id: project_id.clone(),
                    research: current,
                    receipts: receipt_ids.clone(),
                    facts,
                });
            } else if matches!(
                SessionEvent::decode(&event.event_type, event.payload.clone())?,
                SessionEvent::CompactionCreated { .. }
            ) {
                projection.main_v5 = None;
            }
            projection
                .reducer
                .apply_in_place(&mut projection.state, event.clone())?;
        }
        verify_protected_vision_sources(&projection.state, blobs)?;
        Ok(projection)
    }

    /// Strictly reconstructs Session State from durable Events supplied by an
    /// EventSource at the composition boundary.
    pub fn replay<Events>(events: Events) -> Result<Self, SessionError>
    where
        Events: IntoIterator<Item = EventEnvelope<serde_json::Value>>,
    {
        let reducer = SessionReducer;
        let mut state = SessionState::default();
        for event in events {
            reject_unverified_main_compaction(&event)?;
            reducer.apply_in_place(&mut state, event)?;
        }
        Ok(Self {
            reducer,
            state,
            main_v5: None,
        })
    }

    pub(crate) fn replay_verified<Events, Store>(
        events: Events,
        blobs: &Store,
    ) -> Result<Self, SessionError>
    where
        Events: IntoIterator<Item = EventEnvelope<serde_json::Value>>,
        Store: crate::ports::artifact::ArtifactStore,
        Store::Error: std::fmt::Debug,
    {
        let mut projection = Self {
            reducer: SessionReducer,
            state: SessionState::default(),
            main_v5: None,
        };
        projection.ingest_durable_verified(events, blobs)?;
        Ok(projection)
    }

    pub fn state(&self) -> &SessionState {
        &self.state
    }

    pub(crate) fn take_verified_main_v5(&mut self) -> Option<VerifiedMainV5> {
        self.main_v5.take()
    }

    pub(crate) fn ingest_durable_verified<Events, Store>(
        &mut self,
        events: Events,
        blobs: &Store,
    ) -> Result<(), SessionError>
    where
        Events: IntoIterator<Item = EventEnvelope<serde_json::Value>>,
        Store: crate::ports::artifact::ArtifactStore,
        Store::Error: std::fmt::Debug,
    {
        let mut state = self.state.clone();
        for event in events {
            reject_unverified_main_compaction(&event)?;
            verify_completed_event(&state, &event, blobs)?;
            self.reducer.apply_in_place(&mut state, event)?;
        }
        verify_protected_vision_sources(&state, blobs)?;
        self.state = state;
        Ok(())
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
        reject_unverified_main_compaction(&event).map_err(AppendAfterDurableError::Session)?;
        append_after_durable(&self.reducer, &mut self.state, sink, event)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn append_project_verified<S, Store>(
        &mut self,
        sink: &mut S,
        event: EventEnvelope<serde_json::Value>,
        project_id: crate::domain::project::ProjectId,
        mut events: Vec<EventEnvelope<serde_json::Value>>,
        research_events: &[EventEnvelope<serde_json::Value>],
        references: &crate::domain::research::ResearchReferenceCatalog,
        receipt_ids: &std::collections::BTreeSet<crate::domain::research::ExecutionReceiptId>,
        blobs: &Store,
    ) -> Result<ApplyOutcome, AppendAfterDurableError<S::Error>>
    where
        S: EventSink<serde_json::Value>,
        Store: crate::ports::artifact::ArtifactStore,
        Store::Error: std::fmt::Debug,
    {
        if self.state.contains_event_id(&event.event_id) {
            return Err(AppendAfterDurableError::Session(
                SessionError::CompactionContextProvenance {
                    message: "Compaction append requires a new Event identity".to_owned(),
                },
            ));
        }
        events.push(event.clone());
        let next = Self::replay_project_verified(
            project_id,
            &events,
            research_events,
            references,
            receipt_ids,
            blobs,
        )
        .map_err(AppendAfterDurableError::Session)?;
        sink.append_event(&event)
            .map_err(AppendAfterDurableError::Sink)?;
        *self = next;
        Ok(ApplyOutcome::Applied)
    }
}

fn verify_protected_vision_sources<Store: crate::ports::artifact::ArtifactStore>(
    state: &SessionState,
    blobs: &Store,
) -> Result<(), SessionError>
where
    Store::Error: std::fmt::Debug,
{
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
    let replay = crate::app::vision::VisionReplayService::new(blobs);
    for (observation, audit) in state.protected_vision_sources()? {
        replay
            .verify_observation(observation, audit)
            .map_err(|error| SessionError::CompactionContextProvenance {
                message: format!("protected Vision observation verification failed: {error}"),
            })?;
    }
    Ok(())
}

fn research_prefix<'a>(
    source: &crate::domain::session::ResearchSourceIdentity,
    events: &'a [EventEnvelope<serde_json::Value>],
) -> &'a [EventEnvelope<serde_json::Value>] {
    match source {
        crate::domain::session::ResearchSourceIdentity::Empty { .. } => &events[..0],
        crate::domain::session::ResearchSourceIdentity::Nonempty { source } => {
            &events[..events.partition_point(|event| event.sequence <= source.watermark())]
        }
    }
}

// The ordinary projection does not have the historical Research prefix or
// graph proof required by main v5. Do not promote a shape-only v5 here.
fn reject_unverified_main_compaction(
    event: &EventEnvelope<serde_json::Value>,
) -> Result<(), SessionError> {
    if let crate::domain::session::SessionEvent::CompactionCreated { compaction } =
        crate::domain::session::SessionEvent::decode(&event.event_type, event.payload.clone())?
        && compaction.summary().schema_version() == 5
    {
        return Err(SessionError::CompactionContextProvenance {
            message: "main v5 requires verified historical Session and Research graph proofs"
                .to_owned(),
        });
    }
    Ok(())
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
            let map = step_audit.context_delivery_map().ok_or_else(|| {
                SessionError::CompactionContextProvenance {
                    message: "accepted delivery provenance is absent".to_owned(),
                }
            })?;
            map.validate_against(&input).map_err(|error| {
                SessionError::CompactionContextProvenance {
                    message: error.to_string(),
                }
            })?;
            delivered.insert(delivery.audit_event_id().clone(), input);
        }
        state.verify_completed_operator_boundary(&boundary, &proof, &delivered)?;
    }
    Ok(())
}

/// Reconstruct each expected node independently from the captured common
/// context and the exact source, then compare it with the actual audit Blobs.
pub(crate) fn reconstruct_summary_graph<Store: crate::ports::artifact::ArtifactStore>(
    summary: &crate::domain::session::SessionSummary,
    events: &[EventEnvelope<serde_json::Value>],
    research_events: &[EventEnvelope<serde_json::Value>],
    research: &crate::domain::session::ValidatedResearchSource,
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
    if plan.source() != source
        || plan.hash()? != expected_hash
        || plan.research_source() != research.identity()
    {
        return Err(invalid(
            "Compaction plan source identity mismatch".to_owned(),
        ));
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
    let (entries, contents) = CompactionSourcePlan::source_entries(
        source,
        research,
        &operation[..boundary],
        research_events,
    )?;
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
        research_events,
        research,
        &proofs,
        &expected,
        &capture.capabilities,
    )?;
    verify_compaction_source_blobs(&graph, blobs)?;
    summary.validate_graph_reference(&graph)?;
    Ok(graph)
}

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
    let replay = crate::app::vision::VisionReplayService::new(blobs);
    for (observation, audit) in &graph.required_vision_sources {
        replay
            .verify_observation(observation, audit.as_ref())
            .map_err(|error| SessionError::CompactionContextProvenance {
                message: format!("protected Vision observation verification failed: {error}"),
            })?;
    }
    Ok(())
}

/// A Session append failed either before durable persistence or while
/// persisting the Event. It intentionally retains the Sink's typed error.
#[derive(Debug)]
pub enum AppendAfterDurableError<E> {
    Session(SessionError),
    Sink(E),
}
