//! Explicit failed-attempt diagnostics, separate from durable request audits.

use std::fs::{DirBuilder, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use super::presentation::{ResponseNotice, ResponsesObserver};
use crate::ports::model::ModelProviderError;

static NEXT_CAPTURE: AtomicU64 = AtomicU64::new(1);

pub(super) struct FailedRequestCapture {
    directory: PathBuf,
    role: Option<&'static str>,
    endpoint: String,
    body: Vec<u8>,
}

impl FailedRequestCapture {
    pub(super) fn new(
        directory: PathBuf,
        role: Option<&'static str>,
        endpoint: String,
        body: Vec<u8>,
    ) -> Self {
        Self {
            directory,
            role,
            endpoint,
            body,
        }
    }

    pub(super) fn record(
        &self,
        error: &ModelProviderError,
        observer: Option<&dyn ResponsesObserver>,
    ) {
        let outcome = self.save(error);
        if let Some(observer) = observer {
            observer.notify(match outcome {
                Ok(path) => ResponseNotice::CaptureSaved {
                    path: path.to_string_lossy().into_owned(),
                },
                Err(error) => ResponseNotice::CaptureFailed { kind: error.kind() },
            });
        }
    }

    fn save(&self, error: &ModelProviderError) -> io::Result<PathBuf> {
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
        let encoded_body = std::str::from_utf8(&self.body).map_err(io::Error::other)?;
        let snapshot = json!({
            "format": "echidna-failed-request-v2",
            "notice": "Diagnostic snapshot; not proof of transmission, RequestAudit, or an automatically replayable packet.",
            "capture_id": identity,
            "captured_at_unix_ms": timestamp.as_millis(),
            "role": self.role,
            "request": {
                "method": "POST",
                "endpoint": self.endpoint,
                "headers": {"accept":"text/event-stream", "content-type":"application/json"},
                "original_encoded_body_bytes": self.body.len(),
                "body": body,
                "encoded_body": encoded_body,
            },
            "failure": error.to_string(),
        });
        let encoded = serde_json::to_vec_pretty(&snapshot).map_err(io::Error::other)?;
        let mut directory = DirBuilder::new();
        directory.recursive(true);
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        directory.create(&self.directory)?;
        let path = self
            .directory
            .join(format!("failed-request-{identity}.json"));
        options.open(&path)?.write_all(&encoded)?;
        Ok(path)
    }
}
