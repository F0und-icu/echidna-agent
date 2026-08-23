//! Canonical rendering and Blob-reference materialization for one attempt.

use serde::Serialize;

use crate::domain::artifacts::BlobId;
use crate::domain::canonical_json::canonical_json;
use crate::domain::events::EventId;

use super::manifest::{
    RequestAuditAttempt, RequestAuditCapabilityMapping, RequestAuditManifest,
    RequestAuditModelInput,
};
use super::{
    REQUEST_AUDIT_MANIFEST_SCHEMA_VERSION, RequestAuditComponentIdentity,
    RequestAuditComponentReference, RequestAuditComponentSource, RequestAuditError, SemanticHash,
};

pub(crate) struct RenderedRequestAudit {
    attempt: RequestAuditAttempt,
    components: Vec<RenderedRequestAuditComponent>,
}

impl RequestAuditAttempt {
    pub(crate) fn render(&self) -> Result<RenderedRequestAudit, RequestAuditError> {
        let mut components = vec![RenderedRequestAuditComponent::new(
            0,
            RequestAuditComponentIdentity::instructions(),
            RequestAuditComponentSource::RenderedInstructions,
            self.delivered_input.instructions.as_bytes().to_vec(),
            &self.delivered_input.instructions,
        )?];
        let mut position = 1_u32;
        for (index, item) in self.delivered_input.content_items.iter().enumerate() {
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
        for (index, spec) in self.delivered_input.tool_specs.iter().enumerate() {
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
            canonical_bytes(&self.delivered_input.requirements)?,
            &self.delivered_input.requirements,
        )?);
        Ok(RenderedRequestAudit {
            attempt: self.clone(),
            components,
        })
    }
}

impl RenderedRequestAudit {
    pub(crate) fn component_bytes(&self) -> impl Iterator<Item = &[u8]> {
        self.components
            .iter()
            .map(|component| component.bytes.as_slice())
    }

    pub(crate) fn materialize(
        self,
        audit_event_id: EventId,
        blobs: impl IntoIterator<Item = PublishedAuditBlob>,
    ) -> Result<RequestAuditManifest, RequestAuditError> {
        let blobs = blobs.into_iter().collect::<Vec<_>>();
        if blobs.len() != self.components.len() {
            return Err(RequestAuditError::PublishedBlobCountMismatch {
                expected: self.components.len(),
                received: blobs.len(),
            });
        }
        let components = self
            .components
            .into_iter()
            .zip(blobs)
            .map(|(rendered, blob)| RequestAuditComponentReference {
                position: rendered.position,
                identity: rendered.identity,
                source: rendered.source,
                blob_id: blob.blob_id,
                size_bytes: blob.size_bytes,
                semantic_hash: rendered.semantic_hash,
            })
            .collect();
        let manifest = RequestAuditManifest {
            schema_version: REQUEST_AUDIT_MANIFEST_SCHEMA_VERSION,
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
            components,
        };
        manifest.validate()?;
        Ok(manifest)
    }
}

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
