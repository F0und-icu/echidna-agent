//! Auditable, confined local file mutation Tools.
//!
//! `write_file` replaces one file only after an exact preimage check.
//! `apply_patch` deliberately accepts the same single-file boundary in this
//! Slice: all parsing, hunk application, preimage validation and staging
//! complete before its one atomic rename. Multi-file patch transactions are
//! deferred rather than claiming an unimplemented cross-file atomicity.

use super::ToolRegistry;
use crate::domain::tools::{
    ResourceKey, ToolDeterminism, ToolEffect, ToolExposure, ToolParallelism, ToolRetryPolicy,
    ToolSpec,
};
use crate::ports::tool::ToolExecutorError;
use serde_json::Value;

mod filesystem;
mod patch;
mod schema;

use std::path::Path;

use serde_json::json;

use crate::domain::tools::{FileMutationAudit, ToolExecutionOutcome, ToolExecutionPlan};

use super::{decode_arguments, invalid_arguments, invalid_plan};
use filesystem::{
    confined_path, durable_replace, mutation_audit, read_preimage, require_auditable_preimage,
};
use patch::{apply_hunks, parse_patch};
use schema::{ApplyPatchArgs, WriteFileArgs, expected_hash, validate_preimages};

use schema::{
    apply_patch_input_schema, normalize_apply_patch, normalize_write_file, output_schema,
    write_file_input_schema,
};

pub(super) async fn execute_write_file(
    project_root: &Path,
    plan: ToolExecutionPlan,
) -> ToolExecutionOutcome {
    let arguments: WriteFileArgs = match decode_arguments("write_file", plan.call.arguments.clone())
    {
        Ok(arguments) => arguments,
        Err(error) => return invalid_plan(plan, error),
    };
    let expected_hash = match expected_hash("write_file", &arguments.expected_hash) {
        Ok(expected_hash) => expected_hash,
        Err(error) => return invalid_plan(plan, error),
    };
    let target = match confined_path(project_root, &arguments.path) {
        Ok(path) => path,
        Err(message) => return invalid_plan(plan, invalid_arguments("write_file", message)),
    };
    let before = match read_preimage(&target) {
        Ok(preimage) => preimage,
        Err(message) => {
            return write_failure(
                plan,
                "write_file_preimage_read_failed",
                &arguments.path,
                message,
            );
        }
    };
    if before.hash != expected_hash {
        return preimage_mismatch(plan, &arguments.path, expected_hash, before.hash);
    }
    if let Err(error) = require_auditable_preimage(&before) {
        return write_failure(plan, "write_file_audit_unavailable", &arguments.path, error);
    }
    let after = arguments.content.into_bytes();
    if let Err(error) = durable_replace(&target, &after) {
        return write_failure(plan, "write_file_publish_failed", &arguments.path, error);
    }
    let final_state = match read_preimage(&target) {
        Ok(final_state) => final_state,
        Err(error) => {
            return write_failure(
                plan,
                "write_file_afterimage_read_failed",
                &arguments.path,
                error,
            );
        }
    };
    if final_state.bytes.as_deref() != Some(after.as_slice()) {
        return write_failure(
            plan,
            "write_file_afterimage_mismatch",
            &arguments.path,
            "published file no longer matches the staged bytes; final state is not claimed as successful",
        );
    }
    let audit = match mutation_audit(&arguments.path, before.hash.clone(), &before, &final_state) {
        Ok(audit) => audit,
        Err(error) => {
            return write_failure(plan, "write_file_audit_unavailable", &arguments.path, error);
        }
    };
    succeeded_mutation(plan, vec![audit])
}

pub(super) async fn execute_apply_patch(
    project_root: &Path,
    plan: ToolExecutionPlan,
) -> ToolExecutionOutcome {
    let arguments: ApplyPatchArgs =
        match decode_arguments("apply_patch", plan.call.arguments.clone()) {
            Ok(arguments) => arguments,
            Err(error) => return invalid_plan(plan, error),
        };
    let patches = match parse_patch(&arguments.patch) {
        Ok(patches) => patches,
        Err(message) => return invalid_plan(plan, invalid_arguments("apply_patch", message)),
    };
    if let Err(error) = validate_preimages(project_root, &patches, &arguments.preimages) {
        return invalid_plan(plan, error);
    }
    let patch = &patches[0];
    let expected_hash = match expected_hash("apply_patch", &arguments.preimages[0].expected_hash) {
        Ok(expected_hash) => expected_hash,
        Err(error) => return invalid_plan(plan, error),
    };
    let target = match confined_path(project_root, &patch.path) {
        Ok(path) => path,
        Err(message) => return invalid_plan(plan, invalid_arguments("apply_patch", message)),
    };
    let before = match read_preimage(&target) {
        Ok(preimage) => preimage,
        Err(message) => {
            return write_failure(plan, "patch_preimage_read_failed", &patch.path, message);
        }
    };
    if before.hash != expected_hash {
        return preimage_mismatch(plan, &patch.path, expected_hash, before.hash);
    }
    let after = match apply_hunks(&before.bytes, patch) {
        Ok(after) => after,
        Err(message) => return write_failure(plan, "patch_apply_failed", &patch.path, message),
    };
    if let Err(error) = durable_replace(&target, &after) {
        return write_failure(plan, "patch_publish_failed", &patch.path, error);
    }
    let final_state = match read_preimage(&target) {
        Ok(final_state) => final_state,
        Err(error) => {
            return write_failure(plan, "patch_afterimage_read_failed", &patch.path, error);
        }
    };
    if final_state.bytes.as_deref() != Some(after.as_slice()) {
        return write_failure(
            plan,
            "patch_afterimage_mismatch",
            &patch.path,
            "published file no longer matches the staged bytes; final state is not claimed as successful",
        );
    }
    let audit = match mutation_audit(&patch.path, before.hash.clone(), &before, &final_state) {
        Ok(audit) => audit,
        Err(error) => return write_failure(plan, "patch_audit_unavailable", &patch.path, error),
    };
    succeeded_mutation(plan, vec![audit])
}

fn succeeded_mutation(
    plan: ToolExecutionPlan,
    mutations: Vec<FileMutationAudit>,
) -> ToolExecutionOutcome {
    let changes = mutations
        .iter()
        .map(|mutation| {
            json!({
                "path": mutation.path,
                "expected_preimage_hash": mutation.expected_preimage_hash,
                "before_hash": mutation.before_hash,
                "before_size": mutation.before_size,
                "after_hash": mutation.after_hash,
                "after_size": mutation.after_size,
            })
        })
        .collect::<Vec<_>>();
    let mut outcome =
        ToolExecutionOutcome::succeeded(plan, json!({"status": "written", "changes": changes}));
    outcome.record_file_mutations(mutations);
    outcome
}

fn write_failure(
    plan: ToolExecutionPlan,
    code: &str,
    path: &str,
    message: impl Into<String>,
) -> ToolExecutionOutcome {
    ToolExecutionOutcome::failed(
        plan,
        code,
        json!({"error_code": code, "path": path, "error": message.into()}),
    )
}

fn preimage_mismatch(
    plan: ToolExecutionPlan,
    path: &str,
    expected_hash: Option<String>,
    current_hash: Option<String>,
) -> ToolExecutionOutcome {
    ToolExecutionOutcome::failed(
        plan,
        "file_preimage_mismatch",
        json!({
            "error_code": "file_preimage_mismatch",
            "path": path,
            "expected_hash": expected_hash,
            "current_hash": current_hash,
            "message": "file changed after the caller captured its preimage; no write was made"
        }),
    )
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum Tool {
    WriteFile,
    ApplyPatch,
}

impl Tool {
    pub(super) const ALL: [Self; 2] = [Self::WriteFile, Self::ApplyPatch];

    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::WriteFile => "write_file",
            Self::ApplyPatch => "apply_patch",
        }
    }

    pub(super) fn spec(self) -> ToolSpec {
        match self {
            Self::WriteFile => super::spec(
                self.name(),
                "1",
                super::description(
                    "one atomic expected-hash file replacement",
                    "a small patch or multi-file edit is needed.",
                    "apply_patch for small edits; write_file for new files or complete replacements. Prefer file tools over shell heredocs. Keep long edits bounded; reuse the returned after_hash for the next edit only if no intervening mutation occurred. See input schema for null/hash semantics.",
                    "a hashed file mutation.",
                    false,
                ),
                ToolExposure::Deferred,
                write_file_input_schema(),
                output_schema(),
                ToolParallelism::ResourceLocked,
                Some("file:{path}"),
                ToolEffect::LocalMutation,
                ToolRetryPolicy::NeverAutoRetry,
                ToolDeterminism::EnvironmentDependent,
                30_000,
            ),
            Self::ApplyPatch => super::spec(
                self.name(),
                "1",
                super::description(
                    "one validated file patch",
                    "the patch is invalid or spans files.",
                    "a small exact unified diff and verified preimage; see input schema for the supported format. Use write_file to create files or replace non-LF text, not shell heredocs to bypass JSON escaping.",
                    "a file mutation.",
                    false,
                ),
                ToolExposure::Direct,
                apply_patch_input_schema(),
                output_schema(),
                ToolParallelism::ResourceLocked,
                Some("file:{preimages[*].path}"),
                ToolEffect::LocalMutation,
                ToolRetryPolicy::NeverAutoRetry,
                ToolDeterminism::EnvironmentDependent,
                30_000,
            ),
        }
    }

    pub(super) fn normalize(
        self,
        registry: &ToolRegistry,
        arguments: Value,
    ) -> Result<(Value, Vec<ResourceKey>), ToolExecutorError> {
        let project_root = &registry.project_root;
        match self {
            Self::WriteFile => normalize_write_file(project_root, arguments),
            Self::ApplyPatch => normalize_apply_patch(project_root, arguments),
        }
    }

    pub(super) async fn execute(
        self,
        registry: &ToolRegistry,
        plan: ToolExecutionPlan,
    ) -> ToolExecutionOutcome {
        match self {
            Self::WriteFile => execute_write_file(&registry.project_root, plan).await,
            Self::ApplyPatch => execute_apply_patch(&registry.project_root, plan).await,
        }
    }
}
