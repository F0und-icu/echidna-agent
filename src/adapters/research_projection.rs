//! One read-only composition of Research facts with Project Session Receipt facts.

use crate::adapters::project_event_store::{
    Project, read_existing_research_events, read_existing_session_events,
};
use crate::domain::research::{
    ExecutionReceiptId, ResearchReducer, ResearchReferenceCatalog, ResearchState,
};
use crate::domain::session::SessionReducer;

pub(crate) fn load_reference_catalog(
    project: &Project,
) -> Result<ResearchReferenceCatalog, String> {
    let mut references = ResearchReferenceCatalog::default();
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
            references
                .insert_execution_receipt(
                    ExecutionReceiptId::new(record.event_id.as_str()),
                    record.receipt.clone(),
                )
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(references)
}

pub(crate) fn load_research_state(project: &Project) -> Result<ResearchState, String> {
    let references = load_reference_catalog(project)?;
    let events = read_existing_research_events(project).map_err(|error| error.to_string())?;
    ResearchReducer
        .reduce_with_references(events, &references)
        .map_err(|error| error.to_string())
}
