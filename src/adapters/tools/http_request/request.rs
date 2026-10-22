//! Closed HTTP request DTOs plus pure normalization and request construction.

use std::collections::BTreeMap;
use std::str::FromStr;

use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use reqwest::redirect::Policy;
use reqwest::{Method, Url};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ports::tool::ToolExecutorError;

use super::super::invalid_arguments;
use super::body::validate_body;

pub(super) const MAX_TIMEOUT_MS: u64 = 30_000;
pub(super) const MAX_REDIRECTS: usize = 5;
pub(super) const MAX_PREVIEW_BYTES: usize = 16 * 1024;
pub(super) const MAX_CAPTURE_BYTES: usize = 4 * 1024 * 1024;
pub(super) const MAX_REQUEST_BODY_BYTES: usize = 1024 * 1024;
pub(super) const MAX_BASE64_BODY_CHARS: usize = MAX_REQUEST_BODY_BYTES.div_ceil(3) * 4;
pub(super) const MAX_HEADERS: usize = 32;
pub(super) const MAX_QUERY_PARAMETERS: usize = 64;
pub(super) const MAX_URL_BYTES: usize = 8 * 1024;
pub(super) const MAX_HEADER_NAME_BYTES: usize = 128;
pub(super) const MAX_HEADER_VALUE_BYTES: usize = 4 * 1024;
pub(super) const MAX_QUERY_NAME_BYTES: usize = 1024;
pub(super) const MAX_QUERY_VALUE_BYTES: usize = 4 * 1024;

const DEFAULT_TIMEOUT_MS: u64 = 10_000;
const DEFAULT_MAX_REDIRECTS: usize = 3;
const DEFAULT_PREVIEW_BYTES: usize = 4 * 1024;
const DEFAULT_CAPTURE_BYTES: usize = 128 * 1024;

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum HttpMethod {
    Get,
    Head,
    Post,
    Put,
    Patch,
    Delete,
    Options,
    Trace,
}

impl From<HttpMethod> for Method {
    fn from(value: HttpMethod) -> Self {
        match value {
            HttpMethod::Get => Self::GET,
            HttpMethod::Head => Self::HEAD,
            HttpMethod::Post => Self::POST,
            HttpMethod::Put => Self::PUT,
            HttpMethod::Patch => Self::PATCH,
            HttpMethod::Delete => Self::DELETE,
            HttpMethod::Options => Self::OPTIONS,
            HttpMethod::Trace => Self::TRACE,
        }
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct QueryParameter {
    name: String,
    value: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum HttpRequestBody {
    Text { value: String },
    Json { value: Value },
    Base64 { data: String },
    ArtifactRef { artifact_id: String },
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct HttpArtifactProvenance {
    pub(super) event_id: String,
    pub(super) occurred_at: String,
    pub(super) artifact_id: String,
    pub(super) target_snapshot_id: String,
    pub(super) toolchain_fingerprint: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct HttpRequestArgs {
    pub(super) method: HttpMethod,
    pub(super) url: String,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    query: Vec<QueryParameter>,
    #[serde(default)]
    pub(super) body: Option<HttpRequestBody>,
    #[serde(default = "default_timeout_ms")]
    pub(super) timeout_ms: u64,
    #[serde(default = "default_max_redirects")]
    pub(super) max_redirects: usize,
    #[serde(default = "default_preview_bytes")]
    pub(super) preview_bytes: usize,
    #[serde(default = "default_capture_bytes")]
    pub(super) capture_bytes: usize,
    pub(super) artifact: HttpArtifactProvenance,
}

pub(super) struct PreparedRequest {
    pub(super) method: Method,
    pub(super) url: Url,
    pub(super) headers: HeaderMap,
}

pub(super) fn validate(
    arguments: &HttpRequestArgs,
    max_preview_bytes: usize,
) -> Result<(), ToolExecutorError> {
    if arguments.url.len() > MAX_URL_BYTES {
        return Err(invalid_arguments(
            "http_request",
            "`url` exceeds the 8192-byte limit",
        ));
    }
    let url = Url::parse(&arguments.url)
        .map_err(|_| invalid_arguments("http_request", "`url` must be an absolute HTTP URL"))?;
    if !is_safe_http_url(&url) {
        return Err(invalid_arguments(
            "http_request",
            "`url` must use http or https without user credentials",
        ));
    }
    if arguments.headers.len() > MAX_HEADERS {
        return Err(invalid_arguments(
            "http_request",
            "`headers` may contain at most 32 entries",
        ));
    }
    for (name, value) in &arguments.headers {
        if name.len() > MAX_HEADER_NAME_BYTES || value.len() > MAX_HEADER_VALUE_BYTES {
            return Err(invalid_arguments(
                "http_request",
                "header name or value exceeds its bounded limit",
            ));
        }
    }
    let _ = request_headers(&arguments.headers)?;
    if arguments.query.len() > MAX_QUERY_PARAMETERS {
        return Err(invalid_arguments(
            "http_request",
            "`query` may contain at most 64 entries",
        ));
    }
    for parameter in &arguments.query {
        if parameter.name.trim().is_empty() {
            return Err(invalid_arguments(
                "http_request",
                "every query parameter `name` must not be empty",
            ));
        }
        if parameter.name.len() > MAX_QUERY_NAME_BYTES
            || parameter.value.len() > MAX_QUERY_VALUE_BYTES
        {
            return Err(invalid_arguments(
                "http_request",
                "query name or value exceeds its bounded limit",
            ));
        }
    }
    if !(1..=MAX_TIMEOUT_MS).contains(&arguments.timeout_ms) {
        return Err(invalid_arguments(
            "http_request",
            "`timeout_ms` must be between 1 and 30000",
        ));
    }
    if arguments.max_redirects > MAX_REDIRECTS {
        return Err(invalid_arguments(
            "http_request",
            "`max_redirects` must be at most 5",
        ));
    }
    if arguments.preview_bytes > max_preview_bytes {
        return Err(invalid_arguments(
            "http_request",
            format!("`preview_bytes` must be at most {max_preview_bytes}"),
        ));
    }
    if arguments.capture_bytes > MAX_CAPTURE_BYTES {
        return Err(invalid_arguments(
            "http_request",
            "`capture_bytes` must be at most 4194304",
        ));
    }
    if arguments.capture_bytes == 0 || arguments.preview_bytes > arguments.capture_bytes {
        return Err(invalid_arguments(
            "http_request",
            "`capture_bytes` must be non-zero and at least `preview_bytes`",
        ));
    }
    validate_body(arguments.body.as_ref())?;
    for (name, value) in [
        ("artifact.event_id", &arguments.artifact.event_id),
        ("artifact.occurred_at", &arguments.artifact.occurred_at),
        ("artifact.artifact_id", &arguments.artifact.artifact_id),
        (
            "artifact.target_snapshot_id",
            &arguments.artifact.target_snapshot_id,
        ),
        (
            "artifact.toolchain_fingerprint",
            &arguments.artifact.toolchain_fingerprint,
        ),
    ] {
        if value.trim().is_empty() {
            return Err(invalid_arguments(
                "http_request",
                format!("`{name}` must not be empty"),
            ));
        }
    }
    let _ = request_url(arguments)?;
    Ok(())
}

pub(super) fn prepare(arguments: &HttpRequestArgs) -> Result<PreparedRequest, ToolExecutorError> {
    let mut headers = request_headers(&arguments.headers)?;
    if matches!(arguments.body.as_ref(), Some(HttpRequestBody::Json { .. }))
        && !headers.contains_key(CONTENT_TYPE)
    {
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    }
    Ok(PreparedRequest {
        method: arguments.method.into(),
        url: request_url(arguments)?,
        headers,
    })
}

pub(super) fn is_safe_http_url(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
}

/// Applies the same URL boundary to every redirect before reqwest can issue
/// the next request. A final-response check remains useful as defense in
/// depth, but cannot replace this pre-follow decision.
pub(super) fn redirect_policy(max_redirects: usize) -> Policy {
    Policy::custom(move |attempt| {
        if !is_safe_http_url(attempt.url()) || attempt.url().as_str().len() > MAX_URL_BYTES {
            attempt.error("HTTP redirect target is outside the supported URL boundary")
        } else if attempt.previous().len() > max_redirects {
            attempt.error("HTTP redirect limit reached")
        } else {
            attempt.follow()
        }
    })
}

fn request_url(arguments: &HttpRequestArgs) -> Result<Url, ToolExecutorError> {
    let mut url = Url::parse(&arguments.url)
        .map_err(|_| invalid_arguments("http_request", "`url` must be an absolute HTTP URL"))?;
    {
        let mut query = url.query_pairs_mut();
        for parameter in &arguments.query {
            query.append_pair(&parameter.name, &parameter.value);
        }
    }
    if url.as_str().len() > MAX_URL_BYTES {
        return Err(invalid_arguments(
            "http_request",
            "URL plus query exceeds the 8192-byte limit",
        ));
    }
    Ok(url)
}

fn request_headers(headers: &BTreeMap<String, String>) -> Result<HeaderMap, ToolExecutorError> {
    let mut parsed = HeaderMap::new();
    for (name, value) in headers {
        let name = HeaderName::from_str(name)
            .map_err(|_| invalid_arguments("http_request", "`headers` contains an invalid name"))?;
        let value = HeaderValue::from_str(value).map_err(|_| {
            invalid_arguments("http_request", "`headers` contains an invalid value")
        })?;
        parsed.insert(name, value);
    }
    Ok(parsed)
}

fn default_timeout_ms() -> u64 {
    DEFAULT_TIMEOUT_MS
}

fn default_max_redirects() -> usize {
    DEFAULT_MAX_REDIRECTS
}

fn default_preview_bytes() -> usize {
    DEFAULT_PREVIEW_BYTES
}

fn default_capture_bytes() -> usize {
    DEFAULT_CAPTURE_BYTES
}
