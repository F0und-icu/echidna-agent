//! Complete raw references and matched immutable local input copies.
use super::*;
use crate::adapters::tools::read_attested_input;
use crate::domain::agent_evolution::validate_input_files;
use crate::domain::tools::OutputStream;
use crate::ports::artifact::ArtifactStore;
use std::fs;
use std::path::{Path, PathBuf};

pub(super) struct Reference {
    pub(super) stdout: Vec<u8>,
    pub(super) stderr: Vec<u8>,
    pub(super) exit_code: i64,
}

impl DurableChatSession {
    pub(super) fn raw_reference(
        &self,
        state: &SessionState,
        call_id: &ToolCallId,
    ) -> Result<Option<Reference>, ChatError> {
        let Some(result) = state.history().items().iter().find_map(|item| match item {
            ContentItem::ToolResult { result } if &result.call_id == call_id => Some(result),
            _ => None,
        }) else {
            return Ok(None);
        };
        let Some(receipt) = state
            .execution_receipts()
            .iter()
            .find(|record| record.receipt.execution_id == ExecutionId::for_model_call(call_id))
        else {
            return Ok(None);
        };
        if result.status != ToolResultStatus::Succeeded
            || receipt.receipt.status != ToolResultStatus::Succeeded
            || result.output["exit_code"] != 0
        {
            return Ok(None);
        }
        self.raw_outcome(result, &receipt.receipt)
    }

    pub(super) fn raw_outcome(
        &self,
        result: &crate::domain::tools::ToolResult,
        receipt: &crate::domain::tools::ExecutionReceipt,
    ) -> Result<Option<Reference>, ChatError> {
        receipt
            .validate_output_captures(result)
            .map_err(definition::error)?;
        let mut streams = Vec::new();
        for stream in [OutputStream::Stdout, OutputStream::Stderr] {
            let Some(capture) = receipt
                .output_captures
                .iter()
                .find(|capture| capture.stream == stream && capture.child_index.is_none())
            else {
                return Ok(None);
            };
            if !capture.complete {
                return Ok(None);
            }
            let bytes = self
                .artifacts
                .read_blob(&capture.blob_id)
                .map_err(definition::error)?;
            if bytes.len() as u64 != capture.retained_bytes {
                return Err(definition::error("RSI raw capture size mismatch"));
            }
            streams.push(bytes);
        }
        let Some(exit_code) = result.output["exit_code"].as_i64() else {
            return Ok(None);
        };
        let stderr = streams.pop().expect("two fixed streams");
        let stdout = streams.pop().expect("two fixed streams");
        Ok(Some(Reference {
            stdout,
            stderr,
            exit_code,
        }))
    }
}

pub(super) struct FrozenInput {
    pub(super) file: ScriptFile,
    pub(super) bytes: Vec<u8>,
    permissions: fs::Permissions,
}
pub(super) fn freeze_inputs(
    root: &Path,
    paths: &[String],
    verified: &[ScriptFile],
) -> Result<Vec<FrozenInput>, String> {
    if paths.is_empty() {
        return Err("Probe requires retained input/code files.".to_owned());
    }
    let files = paths
        .iter()
        .map(|path| {
            verified
                .iter()
                .find(|file| &file.path == path)
                .cloned()
                .ok_or_else(|| format!("Input `{path}` has no reference command attestation."))
        })
        .collect::<Result<Vec<_>, _>>()?;
    validate_input_files(&files)?;
    let canonical_root = root.canonicalize().map_err(|error| error.to_string())?;
    let mut result = Vec::new();
    let mut total = 0;
    for file in files {
        let path = root.join(&file.path);
        if !path
            .canonicalize()
            .map_err(|error| error.to_string())?
            .starts_with(&canonical_root)
        {
            return Err("Probe inputs must be regular files inside the project.".to_owned());
        }
        let (bytes, permissions) = read_attested_input(&path)
            .map_err(|error| format!("Probe input `{}`: {error}", file.path))?;
        total += bytes.len();
        if total > 4 * 1024 * 1024 || blake3::hash(&bytes).to_hex().as_str() != file.blake3 {
            return Err(
                "Probe inputs changed since reference or exceed bounded copy limits.".to_owned(),
            );
        }
        result.push(FrozenInput {
            file,
            bytes,
            permissions,
        });
    }
    Ok(result)
}

pub(super) struct ScratchPair {
    root: PathBuf,
    pub(super) old: PathBuf,
    pub(super) new: PathBuf,
}
impl ScratchPair {
    pub(super) fn new(attempt: &str, inputs: &[FrozenInput]) -> Result<Self, ChatError> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let root = std::env::temp_dir().join(format!(
            "echidna-rsi-{}-{}-{}",
            std::process::id(),
            blake3::hash(attempt.as_bytes()).to_hex(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
        }
        let pair = Self {
            old: root.join("old"),
            new: root.join("new"),
            root,
        };
        for arm in [&pair.old, &pair.new] {
            fs::create_dir(arm)?;
            for input in inputs {
                let path = arm.join(&input.file.path);
                fs::create_dir_all(path.parent().expect("relative input has parent"))?;
                fs::write(&path, &input.bytes)?;
                fs::set_permissions(&path, input.permissions.clone())?;
            }
        }
        Ok(pair)
    }
}
impl Drop for ScratchPair {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.root) {
            eprintln!("[RSI2 scratch retained] {}: {error}", self.root.display());
        }
    }
}
