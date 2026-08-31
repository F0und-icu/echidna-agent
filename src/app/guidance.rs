//! Read-only composition for the deterministic evidence-gap guidance Shadow.
//!
//! The Domain owner evaluates typed facts and controlled candidates. This App
//! boundary exists only to make the production CLI's read-only use explicit.

use crate::domain::guidance::{
    GuidanceError, GuidanceEvalCandidates, GuidanceShadowReport, shadow_report,
};
use crate::domain::research::{ResearchReferenceCatalog, ResearchState, TargetSnapshotId};

pub(crate) fn project_shadow_report(
    research: &ResearchState,
    references: &ResearchReferenceCatalog,
    current_target: &TargetSnapshotId,
    candidates: GuidanceEvalCandidates,
) -> Result<GuidanceShadowReport, GuidanceError> {
    shadow_report(research, references, current_target, candidates)
}
