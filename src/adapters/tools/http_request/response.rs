//! Bounded HTTP response capture and safe model-visible rendering.

use base64::Engine;
use futures_util::StreamExt;
use reqwest::header::{CONTENT_TYPE, HeaderMap};
use serde_json::{Value, json};

pub(super) async fn capture_response(
    response: reqwest::Response,
    maximum_bytes: usize,
) -> Result<(Vec<u8>, bool), ()> {
    let mut stream = response.bytes_stream();
    let mut captured = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| ())?;
        let remaining = maximum_bytes.saturating_sub(captured.len());
        if chunk.len() > remaining {
            captured.extend_from_slice(&chunk[..remaining]);
            return Ok((captured, true));
        }
        captured.extend_from_slice(&chunk);
        if captured.len() == maximum_bytes
            && let Some(next) = stream.next().await
        {
            next.map_err(|_| ())?;
            return Ok((captured, true));
        }
    }
    Ok((captured, false))
}

pub(super) fn response_media_type(headers: &HeaderMap) -> String {
    headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(normalize_media_type)
        .unwrap_or_else(|| "application/octet-stream".to_owned())
}

pub(super) fn preview_json(bytes: &[u8], max_bytes: usize, body_truncated: bool) -> Value {
    let selected = &bytes[..bytes.len().min(max_bytes)];
    let preview_truncated = body_truncated || selected.len() < bytes.len();
    match std::str::from_utf8(selected) {
        Ok(text) => json!({
            "encoding": "utf8",
            "data": text,
            "captured_bytes": selected.len(),
            "truncated": preview_truncated,
        }),
        Err(_) => json!({
            "encoding": "base64",
            "data": base64::engine::general_purpose::STANDARD.encode(selected),
            "captured_bytes": selected.len(),
            "truncated": preview_truncated,
        }),
    }
}

fn normalize_media_type(raw: &str) -> Option<String> {
    let media_type = raw.split(';').next()?.trim().to_ascii_lowercase();
    let (kind, subtype) = media_type.split_once('/')?;
    (is_media_token(kind) && is_media_token(subtype)).then_some(media_type)
}

fn is_media_token(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'!' | b'#' | b'$' | b'&' | b'^' | b'_' | b'.' | b'+' | b'-'
                )
        })
}
