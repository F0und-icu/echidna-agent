//! Summary validation reads the original Session and project-stream prefixes.

use crate::adapters::project_event_store::{
    Project, read_existing_research_events, read_existing_session_events,
};
use crate::adapters::research_projection::{
    CompactionValidationFacts, CompactionValidationFactsError, ProjectArtifactCatalog,
    ProjectReadContext, continuation_text, load_compaction_validation_facts,
    load_compaction_validation_facts_with_references, load_reference_catalog_with_session,
};
use crate::domain::context::ContextWindowPolicy;
use crate::domain::events::StreamId;
use crate::domain::model::ContentItem;
use crate::domain::project::ProjectId;
use crate::domain::session::{
    CompactionValidationContext, NarrativeSummary, SessionError, SessionSourceIdentity,
    SessionState, SessionSummary, SessionSummaryDraft, TaskCompactionValidationFacts,
};

pub(crate) struct SessionCompactionFacts {
    project_id: ProjectId,
    research: Option<CompactionValidationFacts>,
    task: Option<TaskCompactionValidationFacts>,
    graph: Option<crate::domain::session::VerifiedCompactionGraph>,
}

impl SessionCompactionFacts {
    pub(crate) fn verified_graph(
        &self,
    ) -> Option<&crate::domain::session::VerifiedCompactionGraph> {
        self.graph.as_ref()
    }
    pub(crate) fn with_verified_graph(
        mut self,
        graph: crate::domain::session::VerifiedCompactionGraph,
    ) -> Result<Self, SessionError> {
        let task = self
            .task
            .take()
            .ok_or_else(|| SessionError::CompactionContextProvenance {
                message: "verified graph requires task facts".to_owned(),
            })?;
        self.task = Some(task.with_verified_graph(graph.clone()));
        self.graph = Some(graph);
        Ok(self)
    }
    pub(crate) fn task_source(&self) -> Result<&SessionSourceIdentity, SessionError> {
        self.task
            .as_ref()
            .map(TaskCompactionValidationFacts::source)
            .ok_or_else(|| SessionError::CompactionContextProvenance {
                message: "model compaction requires a durable Agent binding".to_owned(),
            })
    }
    pub(crate) fn task_projection(&self) -> Option<&TaskCompactionValidationFacts> {
        self.task.as_ref()
    }
    pub(crate) fn context(&self) -> CompactionValidationContext<'_> {
        match (&self.research, &self.task) {
            (Some(research), Some(task)) => research.context().with_task(task),
            (Some(research), None) => research.context(),
            (None, Some(task)) => CompactionValidationContext::for_task(&self.project_id, task),
            (None, None) => unreachable!("facts always contain Research or a bound task"),
        }
    }

    pub(crate) fn compose_session_summary_draft(
        &self,
        history: &[ContentItem],
        previous: Option<&SessionSummary>,
        reference_limit: usize,
        policy: ContextWindowPolicy,
    ) -> Result<SessionSummaryDraft, SessionError> {
        let research = self
            .research
            .as_ref()
            .map(|facts| {
                facts.compose_session_summary_draft(history, previous, reference_limit, policy)
            })
            .transpose()?;
        let Some(task) = &self.task else {
            return Ok(research.expect("unbound legacy facts contain Research"));
        };
        let (spine, narrative) = match research {
            Some(draft) => (draft.evidence_spine().cloned(), draft.narrative().clone()),
            None => {
                let (goal, progress) = continuation_text(history, previous, policy)?;
                (
                    None,
                    NarrativeSummary::new(goal, progress, Vec::new(), Vec::new())?,
                )
            }
        };
        // Task summaries use FactReference provenance. Even the smallest
        // candidate must retain a source event; an empty reference list would
        // violate that contract before preflight can report no useful reduction.
        let draft = SessionSummaryDraft::for_task(
            task.references(reference_limit.max(1)),
            spine,
            narrative,
        )?;
        if let Some(graph) = &self.graph {
            return Ok(draft.with_verified_graph(graph.clone()));
        }
        Ok(draft)
    }

    pub(crate) fn compose_selected_task_draft(
        &self,
        goal: &str,
        reference_limit: usize,
    ) -> Result<SessionSummaryDraft, SessionError> {
        let task = self
            .task
            .as_ref()
            .ok_or_else(|| SessionError::CompactionContextProvenance {
                message: "selected task Compaction requires a bound Session".to_owned(),
            })?;
        let narrative = NarrativeSummary::new(
            goal,
            "Task-scoped handoff pending the verified compaction sample.",
            Vec::new(),
            Vec::new(),
        )?;
        let draft = SessionSummaryDraft::for_task(
            task.references(reference_limit.max(1)),
            None,
            narrative,
        )?;
        Ok(match &self.graph {
            Some(graph) => draft.with_verified_graph(graph.clone()),
            None => draft,
        })
    }
}

pub(crate) fn load_session_compaction_facts(
    project: &Project,
    state: &SessionState,
    task_scoped: bool,
) -> Result<SessionCompactionFacts, CompactionValidationFactsError> {
    let Some(binding) = state.agent_definition_binding() else {
        return Ok(SessionCompactionFacts {
            project_id: project.project_id().clone(),
            research: Some(load_compaction_validation_facts(project)?),
            task: None,
            graph: None,
        });
    };
    let source_stream = state
        .stream_id()
        .ok_or_else(|| "Compaction requires a started Session".to_owned())?;
    let session_id = session_id(source_stream)?;
    let events =
        read_existing_session_events(project, &session_id).map_err(|error| error.to_string())?;
    let project_events =
        read_existing_research_events(project).map_err(|error| error.to_string())?;
    // The source identity owns strict replay. Reuse its validated state rather
    // than decoding/validating every durable RequestAudit a second time.
    let (source, reduced) =
        SessionSourceIdentity::from_events(project.project_id().clone(), &events, &project_events)
            .map_err(|error| error.to_string())?;
    if &reduced != state {
        return Err("Compaction Session source changed before validation"
            .to_owned()
            .into());
    }
    let references = load_reference_catalog_with_session(project, &session_id, &reduced)?;
    let catalog = ProjectArtifactCatalog::from_events_with_references(
        project_events.iter().cloned(),
        &references,
    )?;
    let task = task_facts(source, &reduced, &catalog, &session_id, &events)?;
    Ok(SessionCompactionFacts {
        project_id: project.project_id().clone(),
        research: if task_scoped {
            None
        } else {
            binding
                .definition()
                .research()
                .map(|_| load_compaction_validation_facts_with_references(project, references))
                .transpose()?
        },
        task: Some(task),
        graph: None,
    })
}

/// Current source captured by the synchronized startup projection. A missing
/// source is a real cache miss after later appends and uses the strict loader.
pub(crate) fn load_session_compaction_facts_from_read(
    project: &Project,
    state: &SessionState,
    source: Option<&SessionSourceIdentity>,
    read: &ProjectReadContext,
) -> Result<SessionCompactionFacts, CompactionValidationFactsError> {
    let Some(source) = source else {
        return load_session_compaction_facts(project, state, false);
    };
    let Some(binding) = state.agent_definition_binding() else {
        return Ok(SessionCompactionFacts {
            project_id: project.project_id().clone(),
            research: Some(read.compaction_facts(project.project_id(), None)?),
            task: None,
            graph: None,
        });
    };
    if source.project_id() != project.project_id()
        || Some(source.stream_id()) != state.stream_id()
        || source.watermark().get()
            != state
                .next_event_sequence()
                .map_err(|error| error.to_string())?
                .get()
                - 1
    {
        return Err(
            "Current Compaction source does not belong to its synchronized projection"
                .to_owned()
                .into(),
        );
    }
    let source = source
        .clone()
        .with_current_project(&read.events)
        .map_err(|error| error.to_string())?;
    let session_id = session_id(source.stream_id())?;
    let catalog = ProjectArtifactCatalog::from_events_with_references(
        read.events.iter().cloned(),
        &read.references,
    )?;
    let task = TaskCompactionValidationFacts::current_projection(
        source,
        state,
        catalog.session_artifact_ids(&session_id).cloned().collect(),
    )
    .map_err(|error| error.to_string())?;
    Ok(SessionCompactionFacts {
        project_id: project.project_id().clone(),
        research: binding
            .definition()
            .research()
            .map(|_| read.compaction_facts(project.project_id(), None))
            .transpose()?,
        task: Some(task),
        graph: None,
    })
}

/// Task references always use their saved Session/project prefix. Research
/// references use current facts here so existing stale-Research rebuild rules
/// remain visible to the caller.
pub(crate) fn load_resume_compaction_facts(
    project: &Project,
    summary: &SessionSummary,
    read: &ProjectReadContext,
    cached: Option<&TaskCompactionValidationFacts>,
) -> Result<SessionCompactionFacts, String> {
    load_summary_facts(project, summary, false, read, cached)
}

pub(crate) fn load_historical_session_compaction_facts(
    project: &Project,
    summary: &SessionSummary,
    read: &ProjectReadContext,
    cached: Option<&TaskCompactionValidationFacts>,
) -> Result<SessionCompactionFacts, String> {
    load_summary_facts(project, summary, true, read, cached)
}

fn load_summary_facts(
    project: &Project,
    summary: &SessionSummary,
    historical_research: bool,
    read: &ProjectReadContext,
    cached: Option<&TaskCompactionValidationFacts>,
) -> Result<SessionCompactionFacts, String> {
    let research = summary
        .evidence_spine()
        .map(|spine| {
            if historical_research {
                read.historical_compaction_facts(project.project_id(), spine.source())
            } else {
                read.compaction_facts(project.project_id(), None)
                    .map_err(|error| error.to_string())
            }
        })
        .transpose()?;
    let task = summary
        .task_references()
        .map(|references| {
            let source = references.source();
            if source.project_id() != project.project_id() {
                return Err("Compaction task belongs to a different project".to_owned());
            }
            let session_id = session_id(source.stream_id())?;
            let project_events = read.event_prefix(source.project_stream_watermark());
            source.validate_project_prefix(project_events).map_err(|error| error.to_string())?;
            let catalog = ProjectArtifactCatalog::from_events_with_references(
                project_events.iter().cloned(), &read.references,
            )?;
            if let Some(cached) = cached.filter(|value| value.source() == source) {
                return Ok(cached.clone().with_artifacts(catalog.session_artifact_ids(&session_id).cloned().collect()));
            }
            let mut events = read_existing_session_events(project, &session_id)
                .map_err(|error| error.to_string())?;
            events.retain(|event| event.sequence.get() <= source.watermark().get());
            let (actual, state) = SessionSourceIdentity::from_events(
                project.project_id().clone(),
                &events,
                project_events,
            ).map_err(|error| error.to_string())?;
            if &actual != source {
                return Err("Compaction task historical Session/project prefix identity/hash does not match".to_owned());
            }
            task_facts(actual, &state, &catalog, &session_id, &events)
        })
        .transpose()?;
    if research.is_none() && task.is_none() {
        return Err("Summary has no typed references to validate".to_owned());
    }
    let graph = if summary.graph_reference().is_some() {
        let graph = task
            .as_ref()
            .and_then(TaskCompactionValidationFacts::verified_graph)
            .ok_or("v6 Summary requires project-aware verified Session replay")?
            .clone();
        summary
            .validate_graph_reference(&graph)
            .map_err(|error| error.to_string())?;
        Some(graph)
    } else {
        None
    };
    Ok(SessionCompactionFacts {
        project_id: project.project_id().clone(),
        research,
        task,
        graph,
    })
}

fn task_facts(
    source: SessionSourceIdentity,
    state: &SessionState,
    catalog: &ProjectArtifactCatalog,
    session_id: &StreamId,
    events: &[crate::domain::events::EventEnvelope<serde_json::Value>],
) -> Result<TaskCompactionValidationFacts, String> {
    TaskCompactionValidationFacts::from_session_source(
        source,
        state,
        catalog.session_artifact_ids(session_id).cloned().collect(),
        events,
    )
    .map_err(|error| error.to_string())
}

fn session_id(stream: &StreamId) -> Result<StreamId, String> {
    stream
        .as_str()
        .strip_prefix("session:")
        .filter(|id| !id.is_empty())
        .map(StreamId::new)
        .ok_or_else(|| format!("invalid Session source stream `{stream}`"))
}
