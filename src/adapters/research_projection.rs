//! One read-only composition of Research facts with Project Session Receipt facts.

use std::collections::BTreeSet;

use crate::adapters::project_event_store::{
    Project, read_existing_research_events, read_existing_session_events,
};
use crate::domain::research::{
    ExecutionReceiptId, ResearchReducer, ResearchReferenceCatalog, ResearchState,
};
use crate::domain::session::{CompactionValidationContext, ResearchStreamIdentity, SessionReducer};

/// Ephemeral read-only facts borrowed by Summary v2 validation. Every field is
/// rebuilt from the authoritative Project Research and Session streams.
pub(crate) struct CompactionValidationFacts {
    project_id: crate::domain::project::ProjectId,
    research_stream: ResearchStreamIdentity,
    research_state: ResearchState,
    execution_receipt_ids: BTreeSet<ExecutionReceiptId>,
}

impl CompactionValidationFacts {
    pub(crate) fn context(&self) -> CompactionValidationContext<'_> {
        CompactionValidationContext::new(
            &self.project_id,
            &self.research_stream,
            &self.research_state,
            &self.execution_receipt_ids,
        )
    }
}

pub(crate) fn load_reference_catalog(
    project: &Project,
) -> Result<ResearchReferenceCatalog, String> {
    load_reference_catalog_and_receipt_ids(project).map(|(catalog, _)| catalog)
}

fn load_reference_catalog_and_receipt_ids(
    project: &Project,
) -> Result<(ResearchReferenceCatalog, BTreeSet<ExecutionReceiptId>), String> {
    let mut references = ResearchReferenceCatalog::default();
    let mut receipt_ids = BTreeSet::new();
    for session_id in project
        .existing_session_ids()
        .map_err(|error| error.to_string())?
    {
        let events = read_existing_session_events(project, &session_id)
            .map_err(|error| format!("Session `{session_id}` cannot be read: {error}"))?;
        let session = SessionReducer
            .reduce(events)
            .map_err(|error| format!("Session `{session_id}` cannot be reduced: {error}"))?;
        for record in session.execution_receipts() {
            let receipt_id = ExecutionReceiptId::new(record.event_id.as_str());
            references
                .insert_execution_receipt(receipt_id.clone(), record.receipt.clone())
                .map_err(|error| error.to_string())?;
            receipt_ids.insert(receipt_id);
        }
    }
    Ok((references, receipt_ids))
}

pub(crate) fn load_research_state(project: &Project) -> Result<ResearchState, String> {
    let references = load_reference_catalog(project)?;
    let events = read_existing_research_events(project).map_err(|error| error.to_string())?;
    ResearchReducer
        .reduce_with_references(events, &references)
        .map_err(|error| error.to_string())
}

pub(crate) fn load_compaction_validation_facts(
    project: &Project,
) -> Result<CompactionValidationFacts, String> {
    let (references, execution_receipt_ids) = load_reference_catalog_and_receipt_ids(project)?;
    let events = read_existing_research_events(project).map_err(|error| error.to_string())?;
    let research_stream =
        ResearchStreamIdentity::from_events(project.project_id().clone(), &events)
            .map_err(|error| error.to_string())?;
    let research_state = ResearchReducer
        .reduce_with_references(events, &references)
        .map_err(|error| error.to_string())?;
    Ok(CompactionValidationFacts {
        project_id: project.project_id().clone(),
        research_stream,
        research_state,
        execution_receipt_ids,
    })
}
