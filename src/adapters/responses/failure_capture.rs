//! Explicit failed-attempt diagnostics, separate from durable request audits.

use std::fs::{DirBuilder, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use super::redaction::WireRedaction;
use crate::ports::model::ModelProviderError;
use crate::terminal_console as console;
use crate::terminal_content::escaped_text;

static NEXT_CAPTURE: AtomicU64 = AtomicU64::new(1);

pub(super) struct FailedRequestCapture {
    directory: PathBuf,
    role: Option<&'static str>,
    endpoint: String,
    body: Vec<u8>,
    bearer: String,
    account: Option<String>,
}

impl FailedRequestCapture {
    pub(super) fn new(
        directory: PathBuf,
        role: Option<&'static str>,
        endpoint: String,
        body: Vec<u8>,
        bearer: String,
        account: Option<String>,
    ) -> Self {
        Self {
            directory,
            role,
            endpoint,
            body,
            bearer,
            account,
        }
    }

    pub(super) fn record(&self, error: &ModelProviderError) {
        let redaction = WireRedaction::new(&[&self.bearer, self.account.as_deref().unwrap_or("")]);
        match self.save(error, &redaction) {
            Ok(path) => {
                let _ = writeln!(
                    console::stderr(),
                    "[debug-failures] saved redacted, non-replayable request snapshot: {}",
                    escaped_text(&redaction.text(&path.to_string_lossy()))
                );
            }
            Err(error) => {
                // Never expose filesystem/source error text or replace the Provider error.
                let _ = writeln!(
                    console::stderr(),
                    "[debug-failures] capture failed ({:?}); original Provider error preserved",
                    error.kind()
                );
            }
        }
    }

    fn save(&self, error: &ModelProviderError, redaction: &WireRedaction) -> io::Result<PathBuf> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?;
        let identity = format!(
            "{}-{}-{}",
            timestamp.as_nanos(),
            std::process::id(),
            NEXT_CAPTURE.fetch_add(1, Ordering::Relaxed)
        );
        let body: Value = serde_json::from_slice(&self.body).map_err(io::Error::other)?;
        let mut snapshot = json!({
            "format": "echidna-redacted-failed-request-v1",
            "notice": "Redacted diagnostic snapshot; not raw network bytes, proof of transmission, RequestAudit, or an automatically replayable packet. Input and tool content are retained.",
            "capture_id": identity,
            "captured_at_unix_ms": timestamp.as_millis(),
            "role": self.role,
            "request": {
                "method": "POST",
                "endpoint": safe_endpoint(&self.endpoint),
                "headers": {"accept":"text/event-stream", "content-type":"application/json"},
                "original_encoded_body_bytes": self.body.len(),
                "body": body,
            },
            "failure": error.to_string(),
        });
        redaction.sanitize(&mut snapshot);
        let encoded = serde_json::to_vec_pretty(&snapshot).map_err(io::Error::other)?;
        let mut directory = DirBuilder::new();
        directory.recursive(true);
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
            directory.mode(0o700);
            options.mode(0o600);
        }
        directory.create(&self.directory)?;
        let path = self
            .directory
            .join(format!("failed-request-{identity}.json"));
        options.open(&path)?.write_all(&encoded)?;
        Ok(path)
    }
}

fn safe_endpoint(endpoint: &str) -> String {
    match reqwest::Url::parse(endpoint) {
        Ok(mut url) => {
            let _ = url.set_username("");
            let _ = url.set_password(None);
            url.set_query(None);
            url.set_fragment(None);
            url.to_string()
        }
        Err(_) => "[invalid endpoint omitted]".to_owned(),
    }
}
