//! Typed arguments and fixed schemas for the confined file mutation Tools.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::domain::tools::ResourceKey;
use crate::ports::tool::ToolExecutorError;

use super::super::resources::ResourceKeyResolver;
use super::super::{decode_arguments, invalid_arguments};
use super::filesystem::confined_path;
use super::patch::ParsedPatch;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WriteFileArgs {
    pub(super) path: String,
    pub(super) content: String,
    pub(super) expected_hash: Value,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ApplyPatchArgs {
    pub(super) patch: String,
    pub(super) preimages: Vec<PatchPreimage>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PatchPreimage {
    pub(super) path: String,
    pub(super) expected_hash: Value,
}

pub(in super::super) fn write_file_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "path": {"type": "string", "minLength": 1, "description": "Project-relative UTF-8 file; parent directory must already exist. No symlinks or parent traversal."},
            "content": {"type": "string", "description": "Complete replacement text, not a shell command. Encode quotes, backslashes and newlines as a valid JSON string; a shell heredoc still requires JSON escaping."},
            "expected_hash": expected_hash_schema()
        },
        "required": ["path", "content", "expected_hash"],
        "additionalProperties": false
    })
}

pub(in super::super) fn apply_patch_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "patch": {"type": "string", "minLength": 1, "description": "One existing UTF-8 LF-terminated file only. LF-only newline-terminated unified diff: --- a/<path> then +++ b/<same-path>, then @@ -old_start,old_count +new_start,new_count @@ and space/+/- prefixed lines with exact counts. All starts/counts must be positive. No Begin Patch markers, diff --git preamble, /dev/null, create/delete/rename, CRLF or no-newline markers. Example: --- a/note.txt\n+++ b/note.txt\n@@ -1,1 +1,1 @@\n-old\n+new\n"},
            "preimages": {
                "type": "array",
                "minItems": 1,
                "maxItems": 1,
                "items": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "minLength": 1},
                        "expected_hash": expected_hash_schema()
                    },
                    "required": ["path", "expected_hash"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["patch", "preimages"],
        "additionalProperties": false
    })
}

fn expected_hash_schema() -> Value {
    json!({
        "type": ["string", "null"],
        "pattern": "^[0-9a-f]{64}$",
        "description": "Exact current BLAKE3 content hash (64 lowercase hex) for an existing file, including an empty file. null asserts the file does NOT exist; it never means unchecked overwrite. Obtain a current hash via inspect_project(hash_paths=[path]) or reuse a just-observed mutation after_hash when no intervening edit occurred."
    })
}

pub(in super::super) fn output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "status": {"type": "string", "const": "written"},
            "changes": {"type": "array", "minItems": 1, "maxItems": 1, "items": {
                "type": "object",
                "properties": {
                    "path": {"type": "string", "minLength": 1},
                    "expected_preimage_hash": {"type": ["string", "null"]},
                    "before_hash": {"type": ["string", "null"]},
                    "before_size": {"type": ["integer", "null"], "minimum": 0},
                    "after_hash": {"type": ["string", "null"]},
                    "after_size": {"type": ["integer", "null"], "minimum": 0},
                    "actual_diff": {"type": "string"}
                },
                "required": ["path", "expected_preimage_hash", "before_hash", "before_size", "after_hash", "after_size", "actual_diff"],
                "additionalProperties": false
            }}
        },
        "required": ["status", "changes"],
        "additionalProperties": false
    })
}

pub(in super::super) fn normalize_write_file(
    project_root: &Path,
    value: Value,
) -> Result<(Value, Vec<ResourceKey>), ToolExecutorError> {
    let arguments: WriteFileArgs = decode_arguments("write_file", value)?;
    expected_hash("write_file", &arguments.expected_hash)?;
    confined_path(project_root, &arguments.path)
        .map_err(|message| invalid_arguments("write_file", message))?;
    Ok((
        serde_json::to_value(&arguments).expect("WriteFileArgs serialization cannot fail"),
        vec![ResourceKeyResolver::new(project_root).file(&arguments.path)],
    ))
}

pub(in super::super) fn normalize_apply_patch(
    project_root: &Path,
    value: Value,
) -> Result<(Value, Vec<ResourceKey>), ToolExecutorError> {
    let arguments: ApplyPatchArgs = decode_arguments("apply_patch", value)?;
    let patches = super::patch::parse_patch(&arguments.patch)
        .map_err(|message| invalid_arguments("apply_patch", message))?;
    validate_preimages(project_root, &patches, &arguments.preimages)?;
    Ok((
        serde_json::to_value(&arguments).expect("ApplyPatchArgs serialization cannot fail"),
        patches
            .iter()
            .map(|patch| ResourceKeyResolver::new(project_root).file(&patch.path))
            .collect(),
    ))
}

pub(super) fn validate_preimages(
    project_root: &Path,
    patches: &[ParsedPatch],
    preimages: &[PatchPreimage],
) -> Result<(), ToolExecutorError> {
    if patches.len() != 1 {
        return Err(invalid_arguments(
            "apply_patch",
            "this Slice accepts exactly one file per apply_patch call",
        ));
    }
    if preimages.len() != 1 || preimages[0].path != patches[0].path {
        return Err(invalid_arguments(
            "apply_patch",
            "`preimages` must contain exactly one entry for the patched path",
        ));
    }
    expected_hash("apply_patch", &preimages[0].expected_hash)?;
    confined_path(project_root, &patches[0].path)
        .map_err(|message| invalid_arguments("apply_patch", message))?;
    Ok(())
}

pub(super) fn expected_hash(
    tool_name: &str,
    value: &Value,
) -> Result<Option<String>, ToolExecutorError> {
    let value = match value {
        Value::Null => return Ok(None),
        Value::String(value) => value,
        _ => {
            return Err(invalid_arguments(
                tool_name,
                "`expected_hash` must be null or a 64-character lowercase hexadecimal BLAKE3 hash",
            ));
        }
    };
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(Some(value.clone()))
    } else {
        Err(invalid_arguments(
            tool_name,
            "`expected_hash` must be null or a 64-character lowercase hexadecimal BLAKE3 hash",
        ))
    }
}
