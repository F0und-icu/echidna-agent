//! Read-only reconstruction of the actual delivered semantic request.

use serde::Serialize;

use crate::domain::artifacts::BlobId;
use crate::domain::model::{ContentItem, ModelInputIR, ModelRequestRequirements};
use crate::domain::tools::ToolSpec;

use super::{
    RequestAuditComponentIdentity, RequestAuditComponentReference, RequestAuditError,
    RequestAuditManifest, RequestAuditModelInput, RequestAuditReconstructionError, SemanticHash,
};

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
        let mut components = self.components.iter();
        let instructions = decode_component(
            components
                .next()
                .expect("validated RequestAudit layout contains instructions"),
            &mut read_blob,
            |bytes| {
                String::from_utf8(bytes.to_vec()).map_err(|error| {
                    RequestAuditError::ComponentDecode {
                        position: 0,
                        message: error.to_string(),
                    }
                })
            },
        )?;
        let mut content_items = Vec::new();
        let mut tool_specs = Vec::new();
        let mut requirements = None;
        for component in components {
            match component.identity {
                RequestAuditComponentIdentity::ContentItem(_) => {
                    content_items.push(decode_component(component, &mut read_blob, |bytes| {
                        decode_json::<ContentItem>(component.position, bytes)
                    })?)
                }
                RequestAuditComponentIdentity::ToolSpec(_) => {
                    tool_specs.push(decode_component(component, &mut read_blob, |bytes| {
                        decode_json::<ToolSpec>(component.position, bytes)
                    })?)
                }
                RequestAuditComponentIdentity::RequestRequirements => {
                    requirements = Some(decode_component(component, &mut read_blob, |bytes| {
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
            schema_version: self.delivered_input.schema_version,
            instructions,
            content_items,
            tool_specs,
            requirements: requirements
                .expect("validated RequestAudit layout contains requirements"),
        };
        let reconstructed = RequestAuditModelInput::from_input(&input)
            .map_err(RequestAuditReconstructionError::Audit)?;
        if reconstructed != self.delivered_input {
            return Err(RequestAuditReconstructionError::Audit(
                RequestAuditError::DeliveredInputHashMismatch,
            ));
        }
        Ok(input)
    }
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
