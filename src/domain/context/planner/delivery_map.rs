//! Exact non-model-visible delivery coverage for one rendered Context input.

mod provenance;

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::domain::artifacts::ArtifactId;
use crate::domain::model::{ContentItem, ModelInputIR};

use super::error::ContextPlannerError;
use super::estimator::artifact_reference_item;
use super::fingerprint::hash;
use super::fragment::{ContextContentHash, ContextFragment, ContextFragmentId};
use super::render::PromptRendererIdentity;

pub use provenance::{
    AuthorityLevel, ContextProvenance, ContextReferences, ContextSubitemId, FreshnessIdentity,
    RetentionClass, SemanticClass,
};

pub const CONTEXT_DELIVERY_MAP_SCHEMA_VERSION: u16 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ContextTransform {
    Identity,
    Truncated { retained_tokens: u32 },
    ArtifactReference { artifact_id: ArtifactId },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContextInstructionSpan {
    byte_start: u64,
    byte_end: u64,
    fragment_id: ContextFragmentId,
    subitem_ordinal: u32,
    subitem_id: ContextSubitemId,
    content_hash: ContextContentHash,
    transform: ContextTransform,
    provenance: ContextProvenance,
}

impl ContextInstructionSpan {
    pub const fn byte_start(&self) -> u64 {
        self.byte_start
    }

    pub const fn byte_end(&self) -> u64 {
        self.byte_end
    }

    pub fn fragment_id(&self) -> &ContextFragmentId {
        &self.fragment_id
    }

    pub fn subitem_id(&self) -> &ContextSubitemId {
        &self.subitem_id
    }

    pub fn content_hash(&self) -> &ContextContentHash {
        &self.content_hash
    }

    pub fn transform(&self) -> &ContextTransform {
        &self.transform
    }

    pub fn provenance(&self) -> &ContextProvenance {
        &self.provenance
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContextContentDelivery {
    final_position: u32,
    fragment_id: ContextFragmentId,
    subitem_ordinal: u32,
    subitem_id: ContextSubitemId,
    content_hash: ContextContentHash,
    transform: ContextTransform,
    provenance: ContextProvenance,
}

impl ContextContentDelivery {
    pub const fn final_position(&self) -> u32 {
        self.final_position
    }

    pub fn fragment_id(&self) -> &ContextFragmentId {
        &self.fragment_id
    }

    pub fn subitem_id(&self) -> &ContextSubitemId {
        &self.subitem_id
    }

    pub fn content_hash(&self) -> &ContextContentHash {
        &self.content_hash
    }

    pub fn transform(&self) -> &ContextTransform {
        &self.transform
    }

    pub fn provenance(&self) -> &ContextProvenance {
        &self.provenance
    }

    pub(super) fn set_position(&mut self, position: usize) -> Result<(), ContextPlannerError> {
        self.final_position =
            u32::try_from(position).map_err(|_| ContextPlannerError::ContextPositionOverflow)?;
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContextDeliveryMap {
    schema_version: u16,
    renderer: PromptRendererIdentity,
    instruction_byte_length: u64,
    content_item_count: u32,
    instruction_spans: Vec<ContextInstructionSpan>,
    content_items: Vec<ContextContentDelivery>,
}

impl ContextDeliveryMap {
    pub(super) fn new(
        renderer: PromptRendererIdentity,
        instruction_byte_length: usize,
        instruction_spans: Vec<ContextInstructionSpan>,
        content_items: Vec<ContextContentDelivery>,
    ) -> Result<Self, ContextPlannerError> {
        let map = Self {
            schema_version: CONTEXT_DELIVERY_MAP_SCHEMA_VERSION,
            renderer,
            instruction_byte_length: u64::try_from(instruction_byte_length)
                .map_err(|_| ContextPlannerError::ContextPositionOverflow)?,
            content_item_count: u32::try_from(content_items.len())
                .map_err(|_| ContextPlannerError::ContextPositionOverflow)?,
            instruction_spans,
            content_items,
        };
        map.validate_intrinsic()?;
        Ok(map)
    }

    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }

    pub fn renderer(&self) -> &PromptRendererIdentity {
        &self.renderer
    }

    pub const fn instruction_byte_length(&self) -> u64 {
        self.instruction_byte_length
    }

    pub fn instruction_spans(&self) -> &[ContextInstructionSpan] {
        &self.instruction_spans
    }

    pub fn content_items(&self) -> &[ContextContentDelivery] {
        &self.content_items
    }

    pub fn validate_against(&self, input: &ModelInputIR) -> Result<(), ContextPlannerError> {
        self.validate_intrinsic()?;
        if self.instruction_byte_length
            != u64::try_from(input.instructions.len())
                .map_err(|_| ContextPlannerError::ContextPositionOverflow)?
            || usize::try_from(self.content_item_count).ok() != Some(input.content_items.len())
        {
            return Err(ContextPlannerError::ContextDeliveryCoverageMismatch);
        }

        let mut cursor = 0_usize;
        for span in &self.instruction_spans {
            let start = usize::try_from(span.byte_start)
                .map_err(|_| ContextPlannerError::ContextPositionOverflow)?;
            let end = usize::try_from(span.byte_end)
                .map_err(|_| ContextPlannerError::ContextPositionOverflow)?;
            if start < cursor || end <= start || end > input.instructions.len() {
                return Err(ContextPlannerError::ContextDeliveryCoverageMismatch);
            }
            if start > cursor && &input.instructions.as_bytes()[cursor..start] != b"\n\n" {
                return Err(ContextPlannerError::ContextDeliverySeparatorMismatch);
            }
            let text = input
                .instructions
                .get(start..end)
                .ok_or(ContextPlannerError::ContextDeliveryCoverageMismatch)?;
            if ContextContentHash::for_value(&text)? != span.content_hash {
                return Err(ContextPlannerError::ContextDeliveryContentHashMismatch);
            }
            cursor = end;
        }
        if cursor != input.instructions.len() && !(cursor == 0 && input.instructions.is_empty()) {
            return Err(ContextPlannerError::ContextDeliveryCoverageMismatch);
        }

        for (index, (entry, item)) in self
            .content_items
            .iter()
            .zip(&input.content_items)
            .enumerate()
        {
            if usize::try_from(entry.final_position).ok() != Some(index)
                || ContextContentHash::for_value(item)? != entry.content_hash
            {
                return Err(ContextPlannerError::ContextDeliveryContentHashMismatch);
            }
            if let ContextTransform::ArtifactReference { artifact_id } = &entry.transform
                && item != &artifact_reference_item(artifact_id)
            {
                return Err(ContextPlannerError::InvalidContextTransform);
            }
        }
        Ok(())
    }

    pub(crate) fn for_incremental_delta(
        &self,
        full_candidate: &ModelInputIR,
        delta: &ModelInputIR,
    ) -> Result<Self, ContextPlannerError> {
        self.validate_against(full_candidate)?;
        if delta.instructions != full_candidate.instructions
            || delta.tool_specs != full_candidate.tool_specs
            || delta.requirements != full_candidate.requirements
            || delta.content_items.len() > full_candidate.content_items.len()
        {
            return Err(ContextPlannerError::ContextDeliveryProjectionMismatch);
        }
        let start = full_candidate.content_items.len() - delta.content_items.len();
        if full_candidate.content_items[start..] != delta.content_items {
            return Err(ContextPlannerError::ContextDeliveryProjectionMismatch);
        }
        let mut content_items = self.content_items[start..].to_vec();
        for (index, entry) in content_items.iter_mut().enumerate() {
            entry.set_position(index)?;
        }
        let map = Self::new(
            self.renderer.clone(),
            delta.instructions.len(),
            self.instruction_spans.clone(),
            content_items,
        )?;
        map.validate_against(delta)?;
        Ok(map)
    }

    pub(crate) fn replace_instructions_from(&mut self, current: &Self) {
        self.renderer = current.renderer.clone();
        self.instruction_byte_length = current.instruction_byte_length;
        self.instruction_spans = current.instruction_spans.clone();
    }

    pub(crate) fn append_content_entries(
        &mut self,
        entries: Vec<ContextContentDelivery>,
    ) -> Result<(), ContextPlannerError> {
        for mut entry in entries {
            entry.set_position(self.content_items.len())?;
            self.content_items.push(entry);
        }
        self.content_item_count = u32::try_from(self.content_items.len())
            .map_err(|_| ContextPlannerError::ContextPositionOverflow)?;
        Ok(())
    }

    pub(crate) fn append_model_output_items(
        &mut self,
        step_id: &str,
        items: &[ContentItem],
    ) -> Result<(), ContextPlannerError> {
        if items.is_empty() {
            return Ok(());
        }
        let fragment_id = ContextFragmentId::new(format!("provider-output:{step_id}"))?;
        let source_hash = hash(&(step_id, items))?;
        let provenance = ContextProvenance::model_output(source_hash)?;
        let mut entries = Vec::with_capacity(items.len());
        for (ordinal, item) in items.iter().enumerate() {
            let ordinal =
                u32::try_from(ordinal).map_err(|_| ContextPlannerError::ContextPositionOverflow)?;
            entries.push(ContextContentDelivery {
                final_position: 0,
                fragment_id: fragment_id.clone(),
                subitem_ordinal: ordinal,
                subitem_id: ContextSubitemId::for_ordinal(&fragment_id, ordinal)?,
                content_hash: ContextContentHash::for_value(item)?,
                transform: ContextTransform::Identity,
                provenance: provenance.clone(),
            });
        }
        self.append_content_entries(entries)
    }

    pub(crate) fn validate_intrinsic(&self) -> Result<(), ContextPlannerError> {
        if self.schema_version != CONTEXT_DELIVERY_MAP_SCHEMA_VERSION
            || !self.renderer.is_generic_v1()
        {
            return Err(ContextPlannerError::UnsupportedContextDeliveryMap);
        }
        if usize::try_from(self.content_item_count).ok() != Some(self.content_items.len()) {
            return Err(ContextPlannerError::ContextDeliveryCoverageMismatch);
        }
        let mut prior_end = 0_u64;
        let mut identities = BTreeSet::new();
        for (index, span) in self.instruction_spans.iter().enumerate() {
            let expected_start = if index == 0 {
                0
            } else {
                prior_end
                    .checked_add(2)
                    .ok_or(ContextPlannerError::ContextPositionOverflow)?
            };
            if span.byte_start != expected_start
                || span.byte_end <= span.byte_start
                || span.byte_end > self.instruction_byte_length
            {
                return Err(ContextPlannerError::ContextDeliveryCoverageMismatch);
            }
            span.subitem_id
                .validate(&span.fragment_id, span.subitem_ordinal)?;
            span.provenance.validate()?;
            if !identities.insert((span.fragment_id.clone(), span.subitem_id.clone())) {
                return Err(ContextPlannerError::DuplicateContextSubitem);
            }
            if matches!(span.transform, ContextTransform::ArtifactReference { .. }) {
                return Err(ContextPlannerError::InvalidContextTransform);
            }
            prior_end = span.byte_end;
        }
        if prior_end != self.instruction_byte_length {
            return Err(ContextPlannerError::ContextDeliveryCoverageMismatch);
        }
        for (index, entry) in self.content_items.iter().enumerate() {
            if usize::try_from(entry.final_position).ok() != Some(index) {
                return Err(ContextPlannerError::ContextDeliveryCoverageMismatch);
            }
            entry
                .subitem_id
                .validate(&entry.fragment_id, entry.subitem_ordinal)?;
            entry.provenance.validate()?;
            if !identities.insert((entry.fragment_id.clone(), entry.subitem_id.clone())) {
                return Err(ContextPlannerError::DuplicateContextSubitem);
            }
            if let ContextTransform::ArtifactReference { artifact_id } = &entry.transform
                && !entry
                    .provenance
                    .references()
                    .artifact_refs()
                    .contains(artifact_id)
            {
                return Err(ContextPlannerError::InvalidContextTransform);
            }
        }
        Ok(())
    }
}

pub(super) fn instruction_span(
    byte_start: usize,
    byte_end: usize,
    fragment: &ContextFragment,
    ordinal: u32,
    text: &str,
    transform: ContextTransform,
) -> Result<ContextInstructionSpan, ContextPlannerError> {
    Ok(ContextInstructionSpan {
        byte_start: u64::try_from(byte_start)
            .map_err(|_| ContextPlannerError::ContextPositionOverflow)?,
        byte_end: u64::try_from(byte_end)
            .map_err(|_| ContextPlannerError::ContextPositionOverflow)?,
        fragment_id: fragment.id().clone(),
        subitem_ordinal: ordinal,
        subitem_id: ContextSubitemId::for_ordinal(fragment.id(), ordinal)?,
        content_hash: ContextContentHash::for_value(&text)?,
        transform,
        provenance: fragment.provenance().clone(),
    })
}

pub(super) fn content_delivery(
    position: usize,
    fragment: &ContextFragment,
    ordinal: u32,
    item: &ContentItem,
    transform: ContextTransform,
) -> Result<ContextContentDelivery, ContextPlannerError> {
    Ok(ContextContentDelivery {
        final_position: u32::try_from(position)
            .map_err(|_| ContextPlannerError::ContextPositionOverflow)?,
        fragment_id: fragment.id().clone(),
        subitem_ordinal: ordinal,
        subitem_id: ContextSubitemId::for_ordinal(fragment.id(), ordinal)?,
        content_hash: ContextContentHash::for_value(item)?,
        transform,
        provenance: ContextProvenance::for_content_item(fragment, ordinal, item)?,
    })
}
