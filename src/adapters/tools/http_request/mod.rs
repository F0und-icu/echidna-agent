//! One bounded, provider-neutral HTTP research Tool.
//!
//! This adapter deliberately exposes a single request, not discovery or
//! scanning. Request/response bytes are bounded, while durable response-body
//! provenance stays with the existing Artifact publisher.

use super::ToolRegistry;
use crate::domain::tools::{
    ToolDeterminism, ToolEffect, ToolExposure, ToolParallelism, ToolRetryPolicy, ToolSpec,
};

mod body;
mod request;
mod response;
mod schema;

use std::time::{Duration, Instant};

use reqwest::header::CONTENT_LENGTH;
use serde_json::{Value, json};

use crate::adapters::artifact_store::LocalArtifactStore;
use crate::domain::artifacts::ArtifactScope;
use crate::domain::context::ContextWindowPolicy;
use crate::domain::tools::{ResourceKey, ToolExecutionOutcome, ToolExecutionPlan};
use crate::ports::artifact::ArtifactStore;
use crate::ports::tool::ToolExecutorError;

use super::artifact::{
    ArtifactScopeBinding, ArtifactStoreArgs, PublishedArtifact, preflight_artifact,
    publish_artifact,
};
use super::resources::ResourceKeyResolver;
use super::{decode_arguments, invalid_plan};

use body::request_body;
use request::{
    HttpRequestArgs, MAX_URL_BYTES, is_safe_http_url, prepare, redirect_policy, validate,
};
use response::{capture_response, preview_json, response_media_type};

fn output_schema() -> Value {
    schema::output_schema()
}

pub(super) fn normalize_for_scope(
    project_root: &std::path::Path,
    control_directory: &str,
    mut value: Value,
    binding: &ArtifactScopeBinding,
    policy: ContextWindowPolicy,
) -> Result<(Value, Vec<ResourceKey>), ToolExecutorError> {
    if let Some(arguments) = value.as_object_mut() {
        arguments
            .entry("preview_bytes")
            .or_insert(json!(policy.readback_default_bytes()));
    }
    let mut arguments: HttpRequestArgs = decode_arguments("http_request", value)?;
    if arguments.artifact_scope.is_some() {
        return Err(ToolExecutorError::InvalidArguments {
            tool_name: "http_request".to_owned(),
            message: "artifact_scope is runtime-bound and must not be supplied".to_owned(),
        });
    }
    validate(&arguments, policy.readback_max_bytes())?;
    arguments.artifact_scope = Some(scope_from_input(
        binding,
        arguments.artifact.target_snapshot_id.as_deref(),
    )?);
    Ok((
        serde_json::to_value(arguments).expect("HttpRequestArgs serialization cannot fail"),
        vec![
            ResourceKeyResolver::new(project_root)
                .project_file(control_directory, "research.jsonl"),
        ],
    ))
}

pub(super) async fn execute(
    project_root: &std::path::Path,
    plan: ToolExecutionPlan,
    policy: ContextWindowPolicy,
) -> ToolExecutionOutcome {
    let arguments: HttpRequestArgs =
        match decode_arguments("http_request", plan.call.arguments.clone()) {
            Ok(arguments) => arguments,
            Err(error) => return invalid_plan(plan, error),
        };
    if let Err(error) = validate(&arguments, policy.readback_max_bytes()) {
        return invalid_plan(plan, error);
    }

    let prepared_request = match prepare(&arguments) {
        Ok(request) => request,
        Err(error) => return invalid_plan(plan, error),
    };
    let preflight = artifact_arguments(
        &arguments,
        prepared_request.url.as_str(),
        "application/octet-stream",
    );
    if let Err(error) = preflight_artifact(project_root, &plan, &preflight) {
        return http_failure(plan, error.code, error.message);
    }
    let body = match request_body(project_root, arguments.body.as_ref()) {
        Ok(body) => body,
        Err(error) => return http_failure(plan, error.code, error.message),
    };
    let client = match reqwest::Client::builder()
        .redirect(redirect_policy(arguments.max_redirects))
        .timeout(Duration::from_millis(arguments.timeout_ms))
        .build()
    {
        Ok(client) => client,
        Err(_) => {
            return http_failure(
                plan,
                "http_client_unavailable",
                "HTTP client construction failed",
            );
        }
    };

    let started = Instant::now();
    let response = match client
        .request(prepared_request.method, prepared_request.url)
        .headers(prepared_request.headers)
        .body(body.unwrap_or_default())
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) if error.is_timeout() => {
            return http_failure(plan, "http_request_timeout", "HTTP request timed out");
        }
        Err(_) => return http_failure(plan, "http_request_failed", "HTTP request failed"),
    };
    let status = response.status().as_u16();
    let final_url = response.url().clone();
    if !is_safe_http_url(&final_url) || final_url.as_str().len() > MAX_URL_BYTES {
        return http_failure(
            plan,
            "http_final_url_rejected",
            "HTTP request completed with an unsupported final URL",
        );
    }
    let media_type = response_media_type(response.headers());
    let content_length = response
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    let (body_bytes, body_truncated) =
        match capture_response(response, arguments.capture_bytes).await {
            Ok(captured) => captured,
            Err(()) => {
                return http_failure(
                    plan,
                    "http_response_capture_failed",
                    "HTTP request completed, but response capture failed",
                );
            }
        };
    let artifact_arguments = artifact_arguments(
        &arguments,
        &capture_source_path(final_url.as_str(), body_bytes.len(), body_truncated),
        &media_type,
    );
    let publication = match publish_artifact(project_root, &plan, &artifact_arguments, |project| {
        LocalArtifactStore::for_project(project).publish_bytes(&body_bytes)
    }) {
        Ok(publication) => publication,
        Err(error) => {
            return http_failure(
                plan,
                "http_artifact_publication_failed",
                format!(
                    "HTTP request completed, but response Artifact publication failed after the external request: {}",
                    error.message
                ),
            );
        }
    };
    let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    ToolExecutionOutcome::succeeded_with_artifacts(
        plan,
        json!({
            "status": status,
            "final_url": final_url.as_str(),
            "metadata": {
                "media_type": media_type,
                "content_length": content_length,
                "captured_body_bytes": body_bytes.len(),
                "body_truncated": body_truncated,
            },
            "timing": {"elapsed_ms": elapsed_ms},
            "body_preview": preview_json(&body_bytes, arguments.preview_bytes, body_truncated),
            "body_artifact": publication_json(&publication, !body_truncated),
        }),
        vec![publication.record.id],
    )
}

fn artifact_arguments(
    arguments: &HttpRequestArgs,
    source_path: &str,
    media_type: &str,
) -> ArtifactStoreArgs {
    ArtifactStoreArgs {
        event_id: arguments.artifact.event_id.clone(),
        occurred_at: arguments.artifact.occurred_at.clone(),
        artifact_id: arguments.artifact.artifact_id.clone(),
        source_path: source_path.to_owned(),
        media_type: media_type.to_owned(),
        scope: arguments
            .artifact_scope
            .clone()
            .expect("HTTP Artifact scope is bound during normalization"),
        toolchain_fingerprint: arguments.artifact.toolchain_fingerprint.clone(),
        parent_artifact_ids: Vec::new(),
        transform: None,
    }
}

fn input_schema_for_scope(binding: &ArtifactScopeBinding) -> Value {
    schema::input_schema(matches!(binding, ArtifactScopeBinding::TargetFromArguments))
}

fn scope_from_input(
    binding: &ArtifactScopeBinding,
    target_snapshot_id: Option<&str>,
) -> Result<ArtifactScope, ToolExecutorError> {
    match binding {
        ArtifactScopeBinding::Session(session_id) => {
            if target_snapshot_id.is_some() {
                return Err(ToolExecutorError::InvalidArguments {
                    tool_name: "http_request".to_owned(),
                    message:
                        "artifact.target_snapshot_id is not accepted for a session-scoped Artifact"
                            .to_owned(),
                });
            }
            Ok(ArtifactScope::Session {
                session_id: session_id.clone(),
            })
        }
        ArtifactScopeBinding::TargetFromArguments => {
            let target_snapshot_id = target_snapshot_id
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| ToolExecutorError::InvalidArguments {
                    tool_name: "http_request".to_owned(),
                    message: "artifact.target_snapshot_id must not be empty".to_owned(),
                })?;
            Ok(ArtifactScope::Target {
                target_snapshot_id: crate::domain::research::TargetSnapshotId::new(
                    target_snapshot_id,
                ),
            })
        }
    }
}

/// A truncated Blob is a response prefix, not the complete resource. The
/// immutable Artifact record has no completion field, so its local source
/// label carries that durable distinction in addition to the Tool Result.
fn capture_source_path(final_url: &str, captured_bytes: usize, truncated: bool) -> String {
    if truncated {
        format!("{final_url}#cyber-agent-captured-prefix={captured_bytes}")
    } else {
        final_url.to_owned()
    }
}

fn publication_json(publication: &PublishedArtifact, capture_complete: bool) -> Value {
    json!({
        "artifact_id": publication.record.id,
        "blob_id": publication.blob.blob_id,
        "size_bytes": publication.blob.size_bytes,
        "already_present": publication.blob.already_present,
        "event_id": publication.record.created_by_event_id,
        "capture_complete": capture_complete,
    })
}

fn http_failure(
    plan: ToolExecutionPlan,
    code: &'static str,
    message: impl Into<String>,
) -> ToolExecutionOutcome {
    ToolExecutionOutcome::failed(
        plan,
        code,
        json!({"error_code": code, "error": message.into()}),
    )
}

pub(super) const NAME: &str = "http_request";

pub(super) fn spec(registry: &ToolRegistry) -> ToolSpec {
    let mut spec = super::spec(
        NAME,
        "3",
        super::description(
            "one bounded HTTP request",
            "a crawl, scanner, Provider request, or vendor client is needed.",
            "a known URL addressing a concrete knowledge gap: version-matched documentation, exact source or original advisory; retain source and target applicability. Use structured method, bounded body and explicit Artifact provenance. This is not a search engine; respect project exclusions on solution material.",
            "status, final URL, safe metadata, timing, preview, and response Artifact.",
            true,
        ),
        ToolExposure::Deferred,
        input_schema_for_scope(&registry.artifact_scope),
        output_schema(),
        ToolParallelism::ResourceLocked,
        Some("file:.echidna/research.jsonl"),
        ToolEffect::ExternalMutation,
        ToolRetryPolicy::NeverAutoRetry,
        ToolDeterminism::Nondeterministic,
        35_000,
    );
    spec.input_schema["properties"]["preview_bytes"]["maximum"] =
        json!(registry.window_policy.readback_max_bytes());
    spec
}
