//! Read-only reconstruction of the actual delivered semantic request.

use serde::Serialize;

use crate::domain::artifacts::BlobId;
use crate::domain::context::ContextDeliveryMap;
use crate::domain::events::EventId;
use crate::domain::model::{ContentItem, ModelInputIR, ModelRequestRequirements};
use crate::domain::tools::ToolSpec;

use super::{
    RequestAuditComponentIdentity, RequestAuditComponentReference, RequestAuditError,
    RequestAuditManifest, RequestAuditModelInput, RequestAuditReconstructionError, SemanticHash,
};

/// A verified logical request and its source map, bound to the real audit fact.
/// This verifies bytes and provenance shape; it does not establish an incremental
/// baseline was delivered to the provider.
pub(crate) struct VerifiedRequestAuditInput {
    audit_event_id: EventId,
    input: ModelInputIR,
    delivery_map: ContextDeliveryMap,
}

impl VerifiedRequestAuditInput {
    pub(crate) fn audit_event_id(&self) -> &EventId {
        &self.audit_event_id
    }
    pub(crate) fn input(&self) -> &ModelInputIR {
        &self.input
    }
    pub(crate) fn delivery_map(&self) -> &ContextDeliveryMap {
        &self.delivery_map
    }
}

impl RequestAuditManifest {
    /// Rebuilds the actual delivered semantic IR exclusively from the ordered
    /// exact Blob bytes. The supplied reader must not invoke a Provider, Tool,
    /// credential source, clock or mutable Project state.
    pub fn reconstruct<E>(
        &self,
        mut read_blob: impl FnMut(&BlobId) -> Result<Vec<u8>, E>,
    ) -> Result<ModelInputIR, RequestAuditReconstructionError<E>> {
        self.validate()
            .map_err(RequestAuditReconstructionError::Audit)?;
        let input = reconstruct_components(
            &self.components,
            &self.delivered_input,
            &mut read_blob,
            RequestAuditError::DeliveredInputHashMismatch,
        )?;
        if let Some(map) = self.context_delivery_map() {
            map.validate_against(&input).map_err(|error| {
                RequestAuditReconstructionError::Audit(RequestAuditError::ContextDeliveryMap {
                    message: error.to_string(),
                })
            })?;
        }
        Ok(input)
    }

    pub(crate) fn reconstruct_full_logical<E>(
        &self,
        mut read_blob: impl FnMut(&BlobId) -> Result<Vec<u8>, E>,
    ) -> Result<Option<VerifiedRequestAuditInput>, RequestAuditReconstructionError<E>> {
        self.validate()
            .map_err(RequestAuditReconstructionError::Audit)?;
        let (components, map) = match (
            &self.full_logical_components,
            &self.full_context_delivery_map,
        ) {
            (Some(components), Some(map)) => (components.as_slice(), map),
            (None, None) if self.full_logical_input == self.delivered_input => {
                let Some(map) = self.context_delivery_map() else {
                    return Ok(None);
                };
                (self.components.as_slice(), map)
            }
            _ => return Ok(None),
        };
        let input = reconstruct_components(
            components,
            &self.full_logical_input,
            &mut read_blob,
            RequestAuditError::FullLogicalInputHashMismatch,
        )?;
        map.validate_against(&input).map_err(|error| {
            RequestAuditReconstructionError::Audit(RequestAuditError::ContextDeliveryMap {
                message: error.to_string(),
            })
        })?;
        Ok(Some(VerifiedRequestAuditInput {
            audit_event_id: self.audit_event_id.clone(),
            input,
            delivery_map: map.clone(),
        }))
    }
}

fn reconstruct_components<E>(
    references: &[RequestAuditComponentReference],
    expected: &RequestAuditModelInput,
    read_blob: &mut impl FnMut(&BlobId) -> Result<Vec<u8>, E>,
    hash_error: RequestAuditError,
) -> Result<ModelInputIR, RequestAuditReconstructionError<E>> {
    let mut components = references.iter();
    let instructions = decode_component(
        components
            .next()
            .expect("validated RequestAudit layout contains instructions"),
        read_blob,
        |bytes| {
            String::from_utf8(bytes.to_vec()).map_err(|error| RequestAuditError::ComponentDecode {
                position: 0,
                message: error.to_string(),
            })
        },
    )?;
    let mut content_items = Vec::new();
    let mut tool_specs = Vec::new();
    let mut requirements = None;
    for component in components {
        match component.identity {
            RequestAuditComponentIdentity::ContentItem(_) => {
                content_items.push(decode_component(component, read_blob, |bytes| {
                    decode_json::<ContentItem>(component.position, bytes)
                })?)
            }
            RequestAuditComponentIdentity::ToolSpec(_) => {
                tool_specs.push(decode_component(component, read_blob, |bytes| {
                    decode_json::<ToolSpec>(component.position, bytes)
                })?)
            }
            RequestAuditComponentIdentity::RequestRequirements => {
                requirements = Some(decode_component(component, read_blob, |bytes| {
                    decode_json::<ModelRequestRequirements>(component.position, bytes)
                })?);
            }
            RequestAuditComponentIdentity::Instructions => {
                return Err(RequestAuditReconstructionError::Audit(
                    RequestAuditError::ComponentOrderMismatch {
                        position: component.position,
                        expected: "no second instructions component",
                    },
                ));
            }
        }
    }
    let input = ModelInputIR {
        schema_version: expected.schema_version,
        instructions,
        content_items,
        tool_specs,
        requirements: requirements.expect("validated RequestAudit layout contains requirements"),
    };
    let reconstructed = RequestAuditModelInput::from_input(&input)
        .map_err(RequestAuditReconstructionError::Audit)?;
    if reconstructed != *expected {
        return Err(RequestAuditReconstructionError::Audit(hash_error));
    }
    Ok(input)
}

fn decode_component<T: Serialize, E>(
    component: &RequestAuditComponentReference,
    read_blob: &mut impl FnMut(&BlobId) -> Result<Vec<u8>, E>,
    decode: impl FnOnce(&[u8]) -> Result<T, RequestAuditError>,
) -> Result<T, RequestAuditReconstructionError<E>> {
    let bytes = read_blob(&component.blob_id).map_err(RequestAuditReconstructionError::Blob)?;
    let size_bytes = u64::try_from(bytes.len()).map_err(|_| {
        RequestAuditReconstructionError::Audit(RequestAuditError::ComponentSizeOverflow {
            position: component.position,
        })
    })?;
    if size_bytes != component.size_bytes {
        return Err(RequestAuditReconstructionError::Audit(
            RequestAuditError::ComponentSizeMismatch {
                position: component.position,
                expected: component.size_bytes,
                received: size_bytes,
            },
        ));
    }
    let decoded = decode(&bytes).map_err(RequestAuditReconstructionError::Audit)?;
    let hash =
        SemanticHash::from_canonical(&decoded).map_err(RequestAuditReconstructionError::Audit)?;
    if hash != component.semantic_hash {
        return Err(RequestAuditReconstructionError::Audit(
            RequestAuditError::ComponentHashMismatch {
                position: component.position,
            },
        ));
    }
    Ok(decoded)
}

fn decode_json<T>(position: u32, bytes: &[u8]) -> Result<T, RequestAuditError>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_slice(bytes).map_err(|error| RequestAuditError::ComponentDecode {
        position,
        message: error.to_string(),
    })
}
