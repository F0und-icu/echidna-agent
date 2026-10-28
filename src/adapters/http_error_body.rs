//! Bounded HTTP error bodies for local diagnostics and protocol classification.

pub(crate) const MAX_HTTP_ERROR_BODY_BYTES: usize = 64 * 1024;

pub(crate) enum HttpErrorBody {
    Complete(String),
    Unreadable(String),
    Oversized,
}

impl std::fmt::Display for HttpErrorBody {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Complete(body) => formatter.write_str(body),
            Self::Unreadable(reason) => write!(formatter, "error body unavailable: {reason}"),
            Self::Oversized => formatter.write_str("error body exceeds the configured read limit"),
        }
    }
}

pub(crate) async fn read_bounded_http_error_body(
    mut response: reqwest::Response,
    maximum_bytes: usize,
) -> HttpErrorBody {
    let mut body = Vec::new();
    loop {
        let chunk = match response.chunk().await {
            Ok(chunk) => chunk,
            Err(error) => return HttpErrorBody::Unreadable(error.to_string()),
        };
        let Some(chunk) = chunk else {
            break;
        };
        if chunk.len() > maximum_bytes.saturating_sub(body.len()) {
            return HttpErrorBody::Oversized;
        }
        body.extend_from_slice(&chunk);
    }
    match String::from_utf8(body) {
        Ok(body) => HttpErrorBody::Complete(body),
        Err(error) => HttpErrorBody::Unreadable(error.to_string()),
    }
}
