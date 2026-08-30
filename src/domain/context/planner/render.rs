//! The sole conversion from a validated ContextPlan to ModelInputIR.

use serde::{Deserialize, Deserializer, Serialize, de};

use crate::domain::history::validate_content_items;
use crate::domain::model::{ContentItem, ModelInputIR, ModelRequestRequirements};
use crate::domain::tools::ToolSpec;

use super::delivery_map::{
    ContextContentDelivery, ContextDeliveryMap, ContextTransform, content_delivery,
    instruction_span,
};
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

pub(super) struct RenderedContext {
    pub(super) model_input: ModelInputIR,
    pub(super) delivery_map: ContextDeliveryMap,
}

pub(super) fn render_model_input(
    fragments: &[ContextFragment],
    decisions: &[ContextDecisionEntry],
    tool_specs: Vec<ToolSpec>,
    requirements: ModelRequestRequirements,
    renderer: &PromptRendererIdentity,
) -> Result<RenderedContext, ContextPlannerError> {
    if fragments.len() != decisions.len() {
        return Err(ContextPlannerError::InvalidHistoryPairing {
            message: "Context fragment and decision counts differ".to_owned(),
        });
    }
    let mut instructions = String::new();
    let mut content_items = Vec::new();
    let mut instruction_spans = Vec::new();
    let mut content_deliveries = Vec::new();
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
            &mut instruction_spans,
            &mut content_deliveries,
            fragment,
            &decision.decision,
        )?;
    }
    validate_content_items(&content_items).map_err(|error| {
        ContextPlannerError::InvalidHistoryPairing {
            message: error.to_string(),
        }
    })?;
    let model_input = ModelInputIR::current(instructions, content_items, requirements)
        .with_tool_specs(tool_specs);
    let delivery_map = ContextDeliveryMap::new(
        renderer.clone(),
        model_input.instructions.len(),
        instruction_spans,
        content_deliveries,
    )?;
    delivery_map.validate_against(&model_input)?;
    Ok(RenderedContext {
        model_input,
        delivery_map,
    })
}

fn append_rendered_fragment(
    instructions: &mut String,
    content_items: &mut Vec<ContentItem>,
    instruction_spans: &mut Vec<super::delivery_map::ContextInstructionSpan>,
    content_deliveries: &mut Vec<ContextContentDelivery>,
    fragment: &ContextFragment,
    decision: &ContextDecision,
) -> Result<(), ContextPlannerError> {
    match decision {
        ContextDecision::Exclude => {}
        ContextDecision::ArtifactRef { artifact_id } => {
            let item = artifact_reference_item(artifact_id);
            append_item(
                content_items,
                content_deliveries,
                fragment,
                0,
                item,
                ContextTransform::ArtifactReference {
                    artifact_id: artifact_id.clone(),
                },
            )?;
        }
        ContextDecision::Include => append_content(
            instructions,
            content_items,
            instruction_spans,
            content_deliveries,
            fragment,
            &fragment.content,
            ContextTransform::Identity,
        )?,
        ContextDecision::Truncate { retained_tokens } => {
            let content = truncate_content_to_token_limit(&fragment.content, *retained_tokens)?
                .ok_or(ContextPlannerError::TruncationUnavailable {
                    retained_tokens: *retained_tokens,
                })?;
            append_content(
                instructions,
                content_items,
                instruction_spans,
                content_deliveries,
                fragment,
                &content,
                ContextTransform::Truncated {
                    retained_tokens: *retained_tokens,
                },
            )?;
        }
    }
    Ok(())
}

fn append_content(
    instructions: &mut String,
    content_items: &mut Vec<ContentItem>,
    instruction_spans: &mut Vec<super::delivery_map::ContextInstructionSpan>,
    content_deliveries: &mut Vec<ContextContentDelivery>,
    fragment: &ContextFragment,
    content: &ContextFragmentContent,
    transform: ContextTransform,
) -> Result<(), ContextPlannerError> {
    match content {
        ContextFragmentContent::Instructions { text } => {
            if !instructions.is_empty() {
                instructions.push_str("\n\n");
            }
            let start = instructions.len();
            instructions.push_str(text);
            instruction_spans.push(instruction_span(
                start,
                instructions.len(),
                fragment,
                0,
                text,
                transform,
            )?);
        }
        ContextFragmentContent::ContentItems { items } => {
            for (ordinal, item) in items.iter().cloned().enumerate() {
                let ordinal = u32::try_from(ordinal)
                    .map_err(|_| ContextPlannerError::ContextPositionOverflow)?;
                append_item(
                    content_items,
                    content_deliveries,
                    fragment,
                    ordinal,
                    item,
                    transform.clone(),
                )?;
            }
        }
    }
    Ok(())
}

fn append_item(
    content_items: &mut Vec<ContentItem>,
    content_deliveries: &mut Vec<ContextContentDelivery>,
    fragment: &ContextFragment,
    ordinal: u32,
    item: ContentItem,
    transform: ContextTransform,
) -> Result<(), ContextPlannerError> {
    content_deliveries.push(content_delivery(
        content_items.len(),
        fragment,
        ordinal,
        &item,
        transform,
    )?);
    content_items.push(item);
    Ok(())
}

pub(super) fn render_included_content_fragment(
    fragment: &ContextFragment,
) -> Result<(Vec<ContentItem>, Vec<ContextContentDelivery>), ContextPlannerError> {
    let mut items = Vec::new();
    let mut deliveries = Vec::new();
    let ContextFragmentContent::ContentItems { items: source } = fragment.content() else {
        return Err(ContextPlannerError::InvalidFragmentChannel {
            fragment_source: fragment.source(),
        });
    };
    for (ordinal, item) in source.iter().cloned().enumerate() {
        let ordinal =
            u32::try_from(ordinal).map_err(|_| ContextPlannerError::ContextPositionOverflow)?;
        append_item(
            &mut items,
            &mut deliveries,
            fragment,
            ordinal,
            item,
            ContextTransform::Identity,
        )?;
    }
    Ok((items, deliveries))
}
