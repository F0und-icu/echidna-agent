//! The sole conversion from a validated ContextPlan to ModelInputIR.

use serde::{Deserialize, Deserializer, Serialize, de};

use crate::domain::history::validate_content_items;
use crate::domain::model::{ContentItem, ModelInputIR, ModelRequestRequirements};
use crate::domain::tools::ToolSpec;

use super::error::ContextPlannerError;
use super::estimator::{artifact_reference_item, truncate_content_to_token_limit};
use super::fragment::{ContextFragment, ContextFragmentContent};
use super::manifest::{ContextDecision, ContextDecisionEntry};

/// Identity of the sole current prompt renderer.
///
/// Current persisted schemas accept only `generic-v1`. Historical manifests
/// without a renderer identity retain their explicit `None` representation;
/// accepting an arbitrary ID here would silently create a second renderer
/// contract before an ADR and an implementation exist.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PromptRendererIdentity {
    id: String,
    version: u16,
}

impl PromptRendererIdentity {
    pub fn generic_v1() -> Self {
        Self {
            id: "generic".to_owned(),
            version: 1,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub const fn version(&self) -> u16 {
        self.version
    }

    pub(crate) fn is_generic_v1(&self) -> bool {
        self.id == "generic" && self.version == 1
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SerializedPromptRendererIdentity {
    id: String,
    version: u16,
}

impl<'de> Deserialize<'de> for PromptRendererIdentity {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let identity = SerializedPromptRendererIdentity::deserialize(deserializer)?;
        if identity.id == "generic" && identity.version == 1 {
            Ok(Self::generic_v1())
        } else {
            Err(de::Error::custom(
                "only the fixed generic-v1 prompt renderer is supported",
            ))
        }
    }
}

pub(super) fn render_model_input(
    fragments: &[ContextFragment],
    decisions: &[ContextDecisionEntry],
    tool_specs: Vec<ToolSpec>,
    requirements: ModelRequestRequirements,
) -> Result<ModelInputIR, ContextPlannerError> {
    if fragments.len() != decisions.len() {
        return Err(ContextPlannerError::InvalidHistoryPairing {
            message: "Context fragment and decision counts differ".to_owned(),
        });
    }
    let mut instructions = Vec::new();
    let mut content_items = Vec::new();
    for (index, fragment) in fragments.iter().enumerate() {
        let decision = &decisions[index];
        if fragment.id != decision.fragment_id
            || fragment.source != decision.source
            || fragment.slot != decision.slot
            || fragment.origin != decision.origin
            || fragment.content_hash != decision.content_hash
        {
            return Err(ContextPlannerError::InvalidHistoryPairing {
                message: "Context fragment and decision identities differ".to_owned(),
            });
        }
        append_rendered_fragment(
            &mut instructions,
            &mut content_items,
            fragment,
            &decision.decision,
        )?;
    }
    validate_content_items(&content_items).map_err(|error| {
        ContextPlannerError::InvalidHistoryPairing {
            message: error.to_string(),
        }
    })?;
    Ok(
        ModelInputIR::current(instructions.join("\n\n"), content_items, requirements)
            .with_tool_specs(tool_specs),
    )
}

fn append_rendered_fragment(
    instructions: &mut Vec<String>,
    content_items: &mut Vec<ContentItem>,
    fragment: &ContextFragment,
    decision: &ContextDecision,
) -> Result<(), ContextPlannerError> {
    match decision {
        ContextDecision::Exclude => {}
        ContextDecision::ArtifactRef { artifact_id } => {
            content_items.push(artifact_reference_item(artifact_id));
        }
        ContextDecision::Include => append_content(instructions, content_items, &fragment.content),
        ContextDecision::Truncate { retained_tokens } => {
            let content = truncate_content_to_token_limit(&fragment.content, *retained_tokens)?
                .ok_or(ContextPlannerError::TruncationUnavailable {
                    retained_tokens: *retained_tokens,
                })?;
            append_content(instructions, content_items, &content);
        }
    }
    Ok(())
}

fn append_content(
    instructions: &mut Vec<String>,
    content_items: &mut Vec<ContentItem>,
    content: &ContextFragmentContent,
) {
    match content {
        ContextFragmentContent::Instructions { text } => instructions.push(text.clone()),
        ContextFragmentContent::ContentItems { items } => content_items.extend(items.clone()),
    }
}
