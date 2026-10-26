//! Summary validation reads the original Session and project-stream prefixes.

use crate::adapters::project_event_store::{
    Project, read_existing_research_events, read_existing_session_events,
};
use crate::adapters::research_projection::{
    CompactionValidationFacts, CompactionValidationFactsError, ProjectArtifactCatalog,
    continuation_text, load_compaction_validation_facts,
    load_compaction_validation_facts_with_references, load_historical_compaction_facts,
    load_reference_catalog, load_reference_catalog_with_session,
};
use crate::domain::context::ContextWindowPolicy;
use crate::domain::events::StreamId;
use crate::domain::model::ContentItem;
use crate::domain::project::ProjectId;
use crate::domain::research::ExecutionReceiptId;
use crate::domain::session::{
    CompactionValidationContext, NarrativeSummary, SessionError, SessionSourceIdentity,
    SessionState, SessionSummary, SessionSummaryDraft, TaskCompactionValidationFacts,
};

pub(crate) struct SessionCompactionFacts {
    project_id: ProjectId,
    research: Option<CompactionValidationFacts>,
    task: Option<TaskCompactionValidationFacts>,
}

impl SessionCompactionFacts {
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
        SessionSummaryDraft::for_task(task.references(reference_limit.max(1)), spine, narrative)
    }
}

pub(crate) fn load_session_compaction_facts(
    project: &Project,
    state: &SessionState,
) -> Result<SessionCompactionFacts, CompactionValidationFactsError> {
    let Some(binding) = state.agent_definition_binding() else {
        return Ok(SessionCompactionFacts {
            project_id: project.project_id().clone(),
            research: Some(load_compaction_validation_facts(project)?),
            task: None,
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
    let task = task_facts(source, &reduced, &catalog, &session_id)?;
    Ok(SessionCompactionFacts {
        project_id: project.project_id().clone(),
        research: binding
            .definition()
            .research()
            .map(|_| load_compaction_validation_facts_with_references(project, references))
            .transpose()?,
        task: Some(task),
    })
}

/// Task references always use their saved Session/project prefix. Research
/// references use current facts here so existing stale-Research rebuild rules
/// remain visible to the caller.
pub(crate) fn load_resume_compaction_facts(
    project: &Project,
    summary: &SessionSummary,
) -> Result<SessionCompactionFacts, String> {
    load_summary_facts(project, summary, false)
}

pub(crate) fn load_historical_session_compaction_facts(
    project: &Project,
    summary: &SessionSummary,
) -> Result<SessionCompactionFacts, String> {
    load_summary_facts(project, summary, true)
}

fn load_summary_facts(
    project: &Project,
    summary: &SessionSummary,
    historical_research: bool,
) -> Result<SessionCompactionFacts, String> {
    let research = summary
        .evidence_spine()
        .map(|spine| {
            if historical_research {
                load_historical_compaction_facts(project, spine.source())
            } else {
                load_compaction_validation_facts(project).map_err(|error| error.to_string())
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
            let mut events = read_existing_session_events(project, &session_id)
                .map_err(|error| error.to_string())?;
            events.retain(|event| event.sequence.get() <= source.watermark().get());
            let mut project_events = read_existing_research_events(project)
                .map_err(|error| error.to_string())?;
            project_events.retain(|event| event.sequence.get() <= source.project_stream_watermark());
            let catalog = ProjectArtifactCatalog::from_events_with_references(
                project_events.iter().cloned(),
                &load_reference_catalog(project)?,
            )?;
            let (actual, state) = SessionSourceIdentity::from_events(
                project.project_id().clone(),
                &events,
                &project_events,
            ).map_err(|error| error.to_string())?;
            if &actual != source {
                return Err("Compaction task historical Session/project prefix identity/hash does not match".to_owned());
            }
            task_facts(actual, &state, &catalog, &session_id)
        })
        .transpose()?;
    if research.is_none() && task.is_none() {
        return Err("Summary has no typed references to validate".to_owned());
    }
    Ok(SessionCompactionFacts {
        project_id: project.project_id().clone(),
        research,
        task,
    })
}

fn task_facts(
    source: SessionSourceIdentity,
    state: &SessionState,
    catalog: &ProjectArtifactCatalog,
    session_id: &StreamId,
) -> Result<TaskCompactionValidationFacts, String> {
    let binding = state
        .agent_definition_binding()
        .ok_or_else(|| "Summary v3 source has no durable Agent definition".to_owned())?;
    TaskCompactionValidationFacts::new(
        source,
        binding,
        catalog.session_artifact_ids(session_id).cloned().collect(),
        state
            .execution_receipts()
            .iter()
            .map(|record| ExecutionReceiptId::new(record.event_id.as_str()))
            .collect(),
        state
            .history_items_with_event_ids()
            .map(|(id, _)| id.clone())
            .collect(),
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
