//! Bounded runtime feedback; the normal Context path owns all delivery.

use crate::domain::context::{
    AuthorityLevel, ContextFragment, ContextFragmentContent, ContextFragmentId,
    ContextFragmentPriority, ContextFragmentSource, ContextOrigin, ContextProvenance,
    ContextReferences, ContextSlot, FragmentCachePolicy, FreshnessIdentity, RetentionClass,
    SemanticClass,
};
use crate::domain::model::{ContentItem, ToolArgumentDiagnostic};

use super::AgentLoopError;

pub(super) fn feedback(
    step_id: &str,
    diagnostic: &ToolArgumentDiagnostic,
) -> Result<ContextFragment, AgentLoopError> {
    let text = format!(
        "Runtime validation notice (not operator input): sample {step_id} was rejected before dispatch: invalid Tool argument JSON ({diagnostic}). No Tool from that sample executed. Continue the current task from retained successful results; regenerate still-needed calls using valid JSON and the advertised schema. Do not repeat completed calls."
    );
    let fragment = ContextFragment::new(
        ContextFragmentId::new(format!("correction:{step_id}"))?,
        ContextFragmentSource::TurnLocalUpdates,
        ContextFragmentPriority::Required,
        FragmentCachePolicy::DoNotCache,
        ContextFragmentContent::ContentItems {
            items: vec![ContentItem::user_text(text)],
        },
    )?;
    let provenance = ContextProvenance::new(
        ContextSlot::TurnUpdates,
        ContextOrigin::SessionProjection,
        SemanticClass::DerivedProjection,
        AuthorityLevel::DerivedNonEvidence,
        RetentionClass::Ephemeral,
        FreshnessIdentity::new(fragment.content_hash().as_str())?
            .with_producer_version("argument-correction:1"),
        ContextReferences::default(),
    )?;
    // Explicit item provenance avoids inferring operator authority from the
    // protocol's user-message representation of runtime Context.
    Ok(fragment.with_content_item_provenance(0, provenance)?)
}
