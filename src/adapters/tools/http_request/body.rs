//! Closed HTTP request-body encoding and Artifact-reference resolution.

use base64::Engine;

use crate::domain::artifacts::ArtifactId;
use crate::ports::tool::ToolExecutorError;

use super::super::artifact::{ArtifactPublicationError, read_artifact_bytes_bounded};
use super::request::{HttpRequestBody, MAX_REQUEST_BODY_BYTES};

/// Validates every body form that needs no project I/O. Calling this during
/// plan construction keeps malformed or oversized model arguments on the
/// paired pre-execution path, before an ExecutionReceipt exists.
pub(super) fn validate_body(body: Option<&HttpRequestBody>) -> Result<(), ToolExecutorError> {
    let Some(body) = body else {
        return Ok(());
    };
    let size_bytes = match body {
        HttpRequestBody::Text { value } => value.len(),
        HttpRequestBody::Json { value } => serde_json::to_vec(value)
            .map_err(|_| invalid_body("JSON request body could not be encoded"))?
            .len(),
        HttpRequestBody::Base64 { data } => base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|_| invalid_body("base64 request body is invalid"))?
            .len(),
        HttpRequestBody::ArtifactRef { artifact_id } => {
            if artifact_id.trim().is_empty() {
                return Err(invalid_body("artifact request-body ID must not be empty"));
            }
            return Ok(());
        }
    };
    if size_bytes > MAX_REQUEST_BODY_BYTES {
        return Err(invalid_body("request body exceeds the 1048576-byte limit"));
    }
    Ok(())
}

pub(super) fn request_body(
    project_root: &std::path::Path,
    body: Option<&HttpRequestBody>,
) -> Result<Option<Vec<u8>>, ArtifactPublicationError> {
    let Some(body) = body else {
        return Ok(None);
    };
    let bytes = match body {
        HttpRequestBody::Text { value } => value.as_bytes().to_vec(),
        HttpRequestBody::Json { value } => {
            serde_json::to_vec(value).expect("JSON body passed plan-time validation")
        }
        HttpRequestBody::Base64 { data } => base64::engine::general_purpose::STANDARD
            .decode(data)
            .expect("base64 body passed plan-time validation"),
        HttpRequestBody::ArtifactRef { artifact_id } => read_artifact_bytes_bounded(
            project_root,
            &ArtifactId::new(artifact_id.clone()),
            MAX_REQUEST_BODY_BYTES,
        )?,
    };
    if bytes.len() > MAX_REQUEST_BODY_BYTES {
        return Err(ArtifactPublicationError {
            code: "http_request_body_too_large",
            message: "request body exceeds the 1048576-byte limit".to_owned(),
        });
    }
    Ok(Some(bytes))
}

fn invalid_body(message: impl Into<String>) -> ToolExecutorError {
    ToolExecutorError::InvalidArguments {
        tool_name: "http_request".to_owned(),
        message: message.into(),
    }
}
