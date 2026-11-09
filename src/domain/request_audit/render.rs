//! Canonical rendering and Blob-reference materialization for one attempt.

use serde::Serialize;

use crate::domain::artifacts::BlobId;
use crate::domain::canonical_json::canonical_json;
use crate::domain::events::EventId;
use crate::domain::model::ModelInputIR;

use super::manifest::{
    RequestAuditAttempt, RequestAuditCapabilityMapping, RequestAuditManifest,
    RequestAuditModelInput,
};
use super::{
    REQUEST_AUDIT_MANIFEST_SCHEMA_VERSION, RequestAuditComponentIdentity,
    RequestAuditComponentReference, RequestAuditComponentSource, RequestAuditError, SemanticHash,
    TASK_REQUEST_AUDIT_MANIFEST_SCHEMA_VERSION,
};

pub(crate) struct RenderedRequestAudit {
    attempt: RequestAuditAttempt,
    components: Vec<RenderedRequestAuditComponent>,
    full_components: Vec<RenderedRequestAuditComponent>,
}

impl RequestAuditAttempt {
    pub(crate) fn render(&self) -> Result<RenderedRequestAudit, RequestAuditError> {
        let components = render_input(&self.delivered_input)?;
        let full_components = if self.full_context_delivery_map.is_some() {
            render_input(&self.full_logical_input)?
        } else {
            Vec::new()
        };
        Ok(RenderedRequestAudit {
            attempt: self.clone(),
            components,
            full_components,
        })
    }
}

fn render_input(
    input: &ModelInputIR,
) -> Result<Vec<RenderedRequestAuditComponent>, RequestAuditError> {
    let mut components = vec![RenderedRequestAuditComponent::new(
        0,
        RequestAuditComponentIdentity::instructions(),
        RequestAuditComponentSource::RenderedInstructions,
        input.instructions.as_bytes().to_vec(),
        &input.instructions,
    )?];
    let mut position = 1_u32;
    for (index, item) in input.content_items.iter().enumerate() {
        components.push(RenderedRequestAuditComponent::new(
            position,
            RequestAuditComponentIdentity::content_item(index)?,
            RequestAuditComponentSource::RenderedContentItem,
            canonical_bytes(item)?,
            item,
        )?);
        position = position
            .checked_add(1)
            .ok_or(RequestAuditError::ComponentPositionOverflow)?;
    }
    for (index, spec) in input.tool_specs.iter().enumerate() {
        components.push(RenderedRequestAuditComponent::new(
            position,
            RequestAuditComponentIdentity::tool_spec(index)?,
            RequestAuditComponentSource::RenderedToolSpec,
            canonical_bytes(spec)?,
            spec,
        )?);
        position = position
            .checked_add(1)
            .ok_or(RequestAuditError::ComponentPositionOverflow)?;
    }
    components.push(RenderedRequestAuditComponent::new(
        position,
        RequestAuditComponentIdentity::requirements(),
        RequestAuditComponentSource::RenderedRequestRequirements,
        canonical_bytes(&input.requirements)?,
        &input.requirements,
    )?);
    Ok(components)
}

impl RenderedRequestAudit {
    pub(crate) fn component_bytes(&self) -> impl Iterator<Item = &[u8]> {
        self.components
            .iter()
            .chain(&self.full_components)
            .map(|component| component.bytes.as_slice())
    }

    pub(crate) fn materialize(
        self,
        audit_event_id: EventId,
        blobs: impl IntoIterator<Item = PublishedAuditBlob>,
    ) -> Result<RequestAuditManifest, RequestAuditError> {
        let blobs = blobs.into_iter().collect::<Vec<_>>();
        let expected = self.components.len() + self.full_components.len();
        if blobs.len() != expected {
            return Err(RequestAuditError::PublishedBlobCountMismatch {
                expected,
                received: blobs.len(),
            });
        }
        let mut blobs = blobs.into_iter();
        let mut materialize = |rendered: RenderedRequestAuditComponent| {
            let blob = blobs.next().expect("published Blob count checked");
            RequestAuditComponentReference {
                position: rendered.position,
                identity: rendered.identity,
                source: rendered.source,
                blob_id: blob.blob_id,
                size_bytes: blob.size_bytes,
                semantic_hash: rendered.semantic_hash,
            }
        };
        let components = self
            .components
            .into_iter()
            .map(&mut materialize)
            .collect::<Vec<_>>();
        let full_logical_components = if self.full_components.is_empty() {
            None
        } else {
            Some(
                self.full_components
                    .into_iter()
                    .map(&mut materialize)
                    .collect(),
            )
        };
        let schema_version = if self.attempt.prompt_renderer.is_some()
            && self.attempt.resolved_tool_plan_fingerprint.is_some()
            && self.attempt.context_delivery_map.is_some()
        {
            if self.attempt.persist_sampling_step_id {
                TASK_REQUEST_AUDIT_MANIFEST_SCHEMA_VERSION
            } else {
                REQUEST_AUDIT_MANIFEST_SCHEMA_VERSION
            }
        } else if self.attempt.prompt_renderer.is_some()
            && self.attempt.resolved_tool_plan_fingerprint.is_some()
        {
            2
        } else {
            1
        };
        let manifest = RequestAuditManifest {
            sampling_step_id: if self.attempt.persist_sampling_step_id {
                self.attempt.sampling_step_id
            } else {
                None
            },
            improvement_scope: self.attempt.improvement_scope,
            provider_session: self.attempt.provider_session,
            schema_version,
            audit_event_id,
            binding: self.attempt.binding,
            model: self.attempt.model,
            profile: self.attempt.profile,
            adapter_version: self.attempt.adapter_version,
            capability_mapping: RequestAuditCapabilityMapping::from_capabilities(
                &self.attempt.capabilities,
            )?,
            full_logical_input: RequestAuditModelInput::from_input(
                &self.attempt.full_logical_input,
            )?,
            delivered_input: RequestAuditModelInput::from_input(&self.attempt.delivered_input)?,
            delivery: self.attempt.delivery,
            prompt_renderer: self.attempt.prompt_renderer,
            resolved_tool_plan_fingerprint: self.attempt.resolved_tool_plan_fingerprint,
            context_delivery_map: self.attempt.context_delivery_map,
            components,
            full_context_delivery_map: self.attempt.full_context_delivery_map,
            full_logical_components,
        };
        manifest.validate()?;
        Ok(manifest)
    }
}

#[derive(Clone)]
pub(crate) struct PublishedAuditBlob {
    pub(crate) blob_id: BlobId,
    pub(crate) size_bytes: u64,
}

struct RenderedRequestAuditComponent {
    position: u32,
    identity: RequestAuditComponentIdentity,
    source: RequestAuditComponentSource,
    bytes: Vec<u8>,
    semantic_hash: SemanticHash,
}

impl RenderedRequestAuditComponent {
    fn new(
        position: u32,
        identity: RequestAuditComponentIdentity,
        source: RequestAuditComponentSource,
        bytes: Vec<u8>,
        semantic_value: &impl Serialize,
    ) -> Result<Self, RequestAuditError> {
        Ok(Self {
            position,
            identity,
            source,
            bytes,
            semantic_hash: SemanticHash::from_canonical(semantic_value)?,
        })
    }
}

fn canonical_bytes(value: &impl Serialize) -> Result<Vec<u8>, RequestAuditError> {
    let value =
        serde_json::to_value(value).map_err(|error| RequestAuditError::CanonicalEncoding {
            message: error.to_string(),
        })?;
    serde_json::to_vec(&canonical_json(&value)).map_err(|error| {
        RequestAuditError::CanonicalEncoding {
            message: error.to_string(),
        }
    })
}
