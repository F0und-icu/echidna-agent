mod support;

use std::fs;

use echidna::adapters::project_event_store::Project;
use echidna::adapters::tools::ToolRegistry;
use echidna::domain::tools::{
    ExecutionId, ToolCachePolicy, ToolCall, ToolCallId, ToolDeterminism, ToolEffect, ToolName,
    ToolResultStatus,
};
use echidna::ports::tool::ToolExecutor;
use serde_json::{Value, json};
use support::TemporaryDirectory;

fn initialized_project(label: &str) -> (TemporaryDirectory, Project) {
    let root = TemporaryDirectory::new(label);
    let project = Project::initialize(root.path()).unwrap();
    (root, project)
}

fn call(id: &str, name: &str, arguments: Value) -> ToolCall {
    ToolCall::new(ToolCallId::new(id), ToolName::new(name), arguments)
}

async fn execute(
    tools: &ToolRegistry,
    id: &str,
    name: &str,
    arguments: Value,
) -> echidna::domain::tools::ToolExecutionOutcome {
    let plan = tools
        .plan(
            ExecutionId::new(format!("execution-{id}")),
            call(id, name, arguments),
        )
        .unwrap();
    tools.execute(plan).await.expect("Tool execution completes")
}

fn hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

#[tokio::test]
async fn patch_rejects_multibyte_prefixes_and_inconsistent_new_offsets_without_mutation() {
    let (_root, project) = initialized_project("patch-line-contract");
    fs::write(project.root().join("note.txt"), "one\ntwo\nthree\n").unwrap();
    let tools = ToolRegistry::for_project(project.root());
    for prefix in ["中文", "😀", "é"] {
        let result = tools.plan(
            ExecutionId::new("bad-prefix"),
            call(
                "bad-prefix",
                "apply_patch",
                json!({
                    "patch":format!("--- a/note.txt\n+++ b/note.txt\n@@ -1,1 +1,1 @@\n{prefix}\n"),
                    "preimages":[{"path":"note.txt","expected_hash":hash(b"one\ntwo\nthree\n")}]
                }),
            ),
        );
        assert!(
            result.is_err(),
            "invalid UTF-8 boundary prefix must be rejected, not panic"
        );
    }
    let before = hash(b"one\ntwo\nthree\n");
    let invalid = execute(
        &tools,
        "bad-offset",
        "apply_patch",
        json!({
            "patch":"--- a/note.txt\n+++ b/note.txt\n@@ -1,1 +8,1 @@\n-one\n+new\n",
            "preimages":[{"path":"note.txt","expected_hash":before}]
        }),
    )
    .await;
    assert_eq!(invalid.result.status, ToolResultStatus::Failed);
    assert_eq!(
        fs::read(project.root().join("note.txt")).unwrap(),
        b"one\ntwo\nthree\n"
    );
    // The second new offset must account for the first hunk's insertion.
    let valid = execute(&tools, "valid-offset", "apply_patch", json!({
        "patch":"--- a/note.txt\n+++ b/note.txt\n@@ -1,1 +1,2 @@\n one\n+inserted\n@@ -3,1 +4,1 @@\n-three\n+THREE\n",
        "preimages":[{"path":"note.txt","expected_hash":before}]
    })).await;
    assert_eq!(valid.result.status, ToolResultStatus::Succeeded);
    assert_eq!(
        fs::read(project.root().join("note.txt")).unwrap(),
        b"one\ninserted\ntwo\nTHREE\n"
    );
}

#[tokio::test]
async fn write_file_null_is_absence_not_unchecked_overwrite_and_after_hash_is_reusable() {
    let (_root, project) = initialized_project("file-null-contract");
    let tools = ToolRegistry::for_project(project.root());
    fs::write(project.root().join("empty.txt"), "").unwrap();
    let rejected = execute(
        &tools,
        "reject-null",
        "write_file",
        json!({"path":"empty.txt","content":"must not overwrite","expected_hash":null}),
    )
    .await;
    assert_eq!(rejected.result.status, ToolResultStatus::Failed);
    assert_eq!(fs::read(project.root().join("empty.txt")).unwrap(), b"");
    let first = execute(
        &tools,
        "empty-write",
        "write_file",
        json!({"path":"empty.txt","content":"first\n","expected_hash":hash(b"")}),
    )
    .await;
    assert_eq!(first.result.status, ToolResultStatus::Succeeded);
    let next_hash = first.result.output["changes"][0]["after_hash"].clone();
    let second = execute(
        &tools,
        "second-write",
        "write_file",
        json!({"path":"empty.txt","content":"second\n","expected_hash":next_hash}),
    )
    .await;
    assert_eq!(second.result.status, ToolResultStatus::Succeeded);
    assert_eq!(
        fs::read(project.root().join("empty.txt")).unwrap(),
        b"second\n"
    );
}

#[tokio::test]
async fn write_file_is_atomic_auditable_and_rejects_a_stale_preimage() {
    let (_root, project) = initialized_project("file-write-audit");
    let tools = ToolRegistry::for_project(project.root());
    let created = execute(
        &tools,
        "write-created",
        "write_file",
        json!({"path": "notes.txt", "content": "first\n", "expected_hash": null}),
    )
    .await;
    assert_eq!(created.result.status, ToolResultStatus::Succeeded);
    assert_eq!(
        created.receipt.environment_change.as_deref(),
        Some("local_files_mutated")
    );
    assert_eq!(
        fs::read(project.root().join("notes.txt")).unwrap(),
        b"first\n"
    );
    let audit = &created.receipt.file_mutations[0];
    assert_eq!(audit.expected_preimage_hash, None);
    assert_eq!(audit.before_hash, None);
    assert_eq!(audit.after_hash.as_deref(), Some(hash(b"first\n").as_str()));
    assert_eq!(audit.after_size, Some(6));
    assert_eq!(audit.actual_diff, "--- notes.txt\n+++ notes.txt\n+first\n");
    assert_eq!(created.result.output["changes"][0]["after_size"], 6);
    assert!(
        created.result.output["changes"][0]
            .get("actual_diff")
            .is_none()
    );
    let spec = tools
        .catalog()
        .into_iter()
        .find(|spec| spec.name.as_str() == "write_file")
        .unwrap();
    let change_schema = &spec.output_schema["properties"]["changes"]["items"];
    assert!(change_schema["properties"]["actual_diff"].is_null());
    assert!(
        !change_schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!("actual_diff"))
    );

    let stale = execute(
        &tools,
        "write-stale",
        "write_file",
        json!({
            "path": "notes.txt",
            "content": "must not appear\n",
            "expected_hash": "0".repeat(64)
        }),
    )
    .await;
    assert_eq!(stale.result.status, ToolResultStatus::Failed);
    assert_eq!(
        stale.receipt.exit_reason.as_deref(),
        Some("file_preimage_mismatch")
    );
    assert_eq!(
        fs::read(project.root().join("notes.txt")).unwrap(),
        b"first\n"
    );
}

#[tokio::test]
async fn write_file_reply_omits_large_diff_while_receipt_retains_it() {
    let (_root, project) = initialized_project("file-write-compact-reply");
    let tools = ToolRegistry::for_project(project.root());
    let content = (0..256)
        .map(|i| format!("payload-line-{i:04}\n"))
        .collect::<String>();
    let expected_diff = format!(
        "--- large.txt\n+++ large.txt\n{}",
        content
            .lines()
            .map(|line| format!("+{line}\n"))
            .collect::<String>()
    );
    let outcome = execute(
        &tools,
        "large-write",
        "write_file",
        json!({"path":"large.txt", "content":content, "expected_hash":null}),
    )
    .await;
    assert_eq!(outcome.result.status, ToolResultStatus::Succeeded);
    assert!(serde_json::to_vec(&outcome.result.output).unwrap().len() < expected_diff.len());
    assert!(!outcome.result.output.to_string().contains("payload-line-"));
    assert_eq!(outcome.receipt.file_mutations[0].actual_diff, expected_diff);
}

#[tokio::test]
async fn write_file_audit_marks_a_nonterminated_final_line() {
    let (_root, project) = initialized_project("file-write-nonterminated-audit");
    let tools = ToolRegistry::for_project(project.root());
    let created = execute(
        &tools,
        "write-nonterminated",
        "write_file",
        json!({"path": "note.txt", "content": "last line", "expected_hash": null}),
    )
    .await;
    assert_eq!(created.result.status, ToolResultStatus::Succeeded);
    assert_eq!(
        created.receipt.file_mutations[0].actual_diff,
        "--- note.txt\n+++ note.txt\n+last line\n\\ No newline at end of file\n"
    );
}

#[tokio::test]
async fn apply_patch_stages_the_complete_single_file_change_before_publishing() {
    let (_root, project) = initialized_project("file-patch");
    let target = project.root().join("source.txt");
    fs::write(&target, "alpha\nbeta\n").unwrap();
    let tools = ToolRegistry::for_project(project.root());
    let before = hash(b"alpha\nbeta\n");

    let invalid = execute(
        &tools,
        "patch-invalid-hunk",
        "apply_patch",
        json!({
            "patch": "--- a/source.txt\n+++ b/source.txt\n@@ -1,2 +1,2 @@\n alpha\n-missing\n+gamma\n",
            "preimages": [{"path": "source.txt", "expected_hash": before}]
        }),
    )
    .await;
    assert_eq!(invalid.result.status, ToolResultStatus::Failed);
    assert_eq!(
        invalid.receipt.exit_reason.as_deref(),
        Some("patch_apply_failed")
    );
    assert_eq!(fs::read(&target).unwrap(), b"alpha\nbeta\n");

    let applied = execute(
        &tools,
        "patch-success",
        "apply_patch",
        json!({
            "patch": "--- a/source.txt\n+++ b/source.txt\n@@ -1,2 +1,2 @@\n alpha\n-beta\n+gamma\n",
            "preimages": [{"path": "source.txt", "expected_hash": before}]
        }),
    )
    .await;
    assert_eq!(applied.result.status, ToolResultStatus::Succeeded);
    assert_eq!(fs::read(&target).unwrap(), b"alpha\ngamma\n");
    let audit = &applied.receipt.file_mutations[0];
    assert_eq!(audit.before_hash.as_deref(), Some(before.as_str()));
    assert_eq!(
        audit.after_hash.as_deref(),
        Some(hash(b"alpha\ngamma\n").as_str())
    );
    assert!(audit.actual_diff.contains("-beta\n"));
    assert!(audit.actual_diff.contains("+gamma\n"));
}

#[tokio::test]
async fn write_paths_are_confined_and_patch_refuses_multi_file_transactions() {
    let (_root, project) = initialized_project("file-write-confinement");
    let tools = ToolRegistry::for_project(project.root());
    assert!(
        tools
            .plan(
                ExecutionId::new("execution-parent"),
                call(
                    "parent",
                    "write_file",
                    json!({"path": "../outside.txt", "content": "no", "expected_hash": null}),
                ),
            )
            .is_err()
    );
    assert!(tools
        .plan(
            ExecutionId::new("execution-multi"),
            call(
                "multi",
                "apply_patch",
                json!({
                    "patch": "--- a/one.txt\n+++ b/one.txt\n@@ -1 +1 @@\n-old\n+new\n--- a/two.txt\n+++ b/two.txt\n@@ -1 +1 @@\n-old\n+new\n",
                    "preimages": [{"path": "one.txt", "expected_hash": "0".repeat(64)}]
                }),
            ),
        )
        .is_err());
    assert!(
        tools
            .plan(
                ExecutionId::new("execution-missing-preimage"),
                call(
                    "missing-preimage",
                    "write_file",
                    json!({"path": "required.txt", "content": "no explicit preimage"}),
                ),
            )
            .is_err()
    );
    assert!(
        tools
            .plan(
                ExecutionId::new("execution-uppercase-preimage"),
                call(
                    "uppercase-preimage",
                    "write_file",
                    json!({
                        "path": "required.txt",
                        "content": "bad hash",
                        "expected_hash": "A".repeat(64)
                    }),
                ),
            )
            .is_err()
    );
    assert!(!project.root().join("outside.txt").exists());
}

#[tokio::test]
async fn unified_hunk_data_beginning_with_three_dashes_is_not_mistaken_for_a_next_header() {
    let (_root, project) = initialized_project("patch-dash-content");
    let target = project.root().join("dash.txt");
    fs::write(&target, "--- before\nkeep\n").unwrap();
    let tools = ToolRegistry::for_project(project.root());
    let outcome = execute(
        &tools,
        "patch-dash-content",
        "apply_patch",
        json!({
            "patch": "--- a/dash.txt\n+++ b/dash.txt\n@@ -1,2 +1,2 @@\n---- before\n+--- after\n keep\n",
            "preimages": [{"path": "dash.txt", "expected_hash": hash(b"--- before\nkeep\n")}]
        }),
    )
    .await;
    assert_eq!(outcome.result.status, ToolResultStatus::Succeeded);
    assert_eq!(fs::read(&target).unwrap(), b"--- after\nkeep\n");
}

#[cfg(unix)]
#[tokio::test]
async fn write_file_preserves_existing_unix_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let (_root, project) = initialized_project("file-write-permissions");
    let target = project.root().join("script.sh");
    fs::write(&target, "#!/bin/sh\necho before\n").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).unwrap();
    let tools = ToolRegistry::for_project(project.root());
    let outcome = execute(
        &tools,
        "write-permissions",
        "write_file",
        json!({
            "path": "script.sh",
            "content": "#!/bin/sh\necho after\n",
            "expected_hash": hash(b"#!/bin/sh\necho before\n")
        }),
    )
    .await;
    assert_eq!(outcome.result.status, ToolResultStatus::Succeeded);
    assert_eq!(
        fs::metadata(target).unwrap().permissions().mode() & 0o777,
        0o755
    );
}

#[cfg(unix)]
#[tokio::test]
async fn write_paths_reject_absolute_and_symlinked_targets() {
    use std::os::unix::fs::symlink;

    let (_root, project) = initialized_project("file-write-unix-path-safety");
    let outside = project.root().join("outside.txt");
    let parent_link = project.root().join("linked-parent");
    let target_link = project.root().join("linked-target.txt");
    fs::create_dir(project.root().join("real-parent")).unwrap();
    symlink(project.root().join("real-parent"), &parent_link).unwrap();
    fs::write(&outside, "outside\n").unwrap();
    symlink(&outside, &target_link).unwrap();
    let tools = ToolRegistry::for_project(project.root());

    for (id, path) in [
        ("absolute", outside.to_string_lossy().into_owned()),
        ("parent-symlink", "linked-parent/child.txt".to_owned()),
        ("target-symlink", "linked-target.txt".to_owned()),
    ] {
        assert!(
            tools
                .plan(
                    ExecutionId::new(format!("execution-{id}")),
                    call(
                        id,
                        "write_file",
                        json!({"path": path, "content": "must not write\n", "expected_hash": null}),
                    ),
                )
                .is_err()
        );
    }
    assert_eq!(fs::read(&outside).unwrap(), b"outside\n");
    assert!(!project.root().join("real-parent/child.txt").exists());
}

#[tokio::test]
async fn research_get_is_never_persistently_cached_and_rerun_stays_explicit() {
    let (_root, project) = initialized_project("research-get-no-cache");
    let tools = ToolRegistry::for_project(project.root());
    let first = execute(&tools, "research-first", "research_get", json!({})).await;
    assert_eq!(first.result.status, ToolResultStatus::Succeeded);
    assert!(first.receipt.cache_source.is_none());
    assert!(first.receipt.cache_validation.is_none());

    let second = execute(&tools, "research-second", "research_get", json!({})).await;
    assert_eq!(second.result.status, ToolResultStatus::Succeeded);
    assert!(second.receipt.cache_source.is_none());
    assert!(second.receipt.cache_validation.is_none());
    assert_ne!(second.receipt.execution_id, first.receipt.execution_id);
    assert_eq!(second.result.output, first.result.output);

    let specs = tools.specs();
    let research = specs
        .iter()
        .find(|spec| spec.name.as_str() == "research_get")
        .unwrap();
    assert_eq!(
        (research.effect, research.determinism, research.cache_policy),
        (
            ToolEffect::ReadOnly,
            ToolDeterminism::Deterministic,
            ToolCachePolicy::None,
        )
    );
    assert!(
        specs
            .iter()
            .all(|spec| spec.cache_policy == ToolCachePolicy::None)
    );
    assert!(!project.root().join(".echidna/cache/tool-results").exists());

    let rerun = execute(
        &tools,
        "rerun-fresh",
        "research_get",
        json!({"execution": {"rerun_reason": "verify current stream"}}),
    )
    .await;
    assert_eq!(rerun.result.status, ToolResultStatus::Succeeded);
    assert!(rerun.receipt.cache_source.is_none());
    assert_eq!(
        rerun
            .receipt
            .rerun_reason
            .as_ref()
            .map(|reason| reason.as_str()),
        Some("verify current stream")
    );
}

#[tokio::test]
async fn apply_patch_rejects_crlf_content_before_publishing() {
    let (_root, project) = initialized_project("file-patch-crlf");
    let target = project.root().join("source.txt");
    fs::write(&target, "old\n").unwrap();
    let tools = ToolRegistry::for_project(project.root());
    let plan = tools.plan(
        ExecutionId::new("execution-patch-crlf"),
        call(
            "patch-crlf",
            "apply_patch",
            json!({
                "patch": "--- a/source.txt\n+++ b/source.txt\n@@ -1 +1 @@\n-old\n+new\r\n",
                "preimages": [{"path": "source.txt", "expected_hash": hash(b"old\n")}]
            }),
        ),
    );
    assert!(plan.is_err());
    assert_eq!(fs::read(target).unwrap(), b"old\n");
}

#[tokio::test]
async fn research_get_observes_every_research_update_without_cache() {
    let (_root, project) = initialized_project("research-get-fresh-reduction");
    let tools = ToolRegistry::for_project(project.root());
    let before = execute(&tools, "research-before-update", "research_get", json!({})).await;
    assert!(before.receipt.cache_source.is_none());

    let update = json!({
        "event_id": "event-cache-invalidation",
        "occurred_at": "2026-08-23T12:00:00Z",
        "operation": {
            "kind": "record_target_snapshot",
            "record": {
                "id": "snapshot-cache-invalidation",
                "target_id": "com.example.target",
                "kind": "mobile",
                "locator": "package:com.example.target",
                "version": "1.0.0",
                "build_id": "build-cache",
                "target_artifact_ids": [],
                "environment_fingerprint": "environment-cache",
                "device_fingerprint": "device-cache",
                "captured_at": "2026-08-23T11:59:00Z",
                "notes": "cache invalidation fixture"
            }
        }
    });
    let recorded = execute(&tools, "update-recorded", "research_update", update.clone()).await;
    assert_eq!(recorded.result.status, ToolResultStatus::Succeeded);
    assert!(recorded.receipt.cache_source.is_none());

    let duplicate = execute(&tools, "update-duplicate", "research_update", update).await;
    assert_eq!(duplicate.result.status, ToolResultStatus::Failed);
    assert_eq!(
        duplicate.receipt.exit_reason.as_deref(),
        Some("research_event_id_duplicate")
    );
    assert!(duplicate.receipt.cache_source.is_none());

    let changed = execute(&tools, "research-after-update", "research_get", json!({})).await;
    assert_eq!(changed.result.status, ToolResultStatus::Succeeded);
    assert!(changed.receipt.cache_source.is_none());
    assert_eq!(changed.result.output["stream_event_count"], 1);
    assert!(
        changed.result.output["state"]["target_snapshots"]
            .get("snapshot-cache-invalidation")
            .is_some()
    );

    let repeated = execute(
        &tools,
        "research-after-update-repeated",
        "research_get",
        json!({}),
    )
    .await;
    assert_eq!(repeated.result.status, ToolResultStatus::Succeeded);
    assert!(repeated.receipt.cache_source.is_none());
    assert_eq!(repeated.result.output, changed.result.output);

    let after_cache = execute(
        &tools,
        "update-after-cache",
        "research_update",
        json!({
            "event_id": "event-after-cache",
            "occurred_at": "2026-08-23T12:01:00Z",
            "operation": {"kind": "record_target_snapshot", "record": {
                "id": "snapshot-after-cache", "target_id": "com.example.target", "kind": "mobile",
                "locator": "package:com.example.target", "version": "1.0.1", "build_id": "build-after-cache",
                "target_artifact_ids": [], "environment_fingerprint": "environment-after-cache",
                "device_fingerprint": "device-after-cache", "captured_at": "2026-08-23T12:01:00Z", "notes": "writer-lock regression"
            }}
        }),
    )
    .await;
    assert_eq!(after_cache.result.status, ToolResultStatus::Succeeded);
}
