#[path = "memory_snapshot/freshness.rs"]
mod freshness;
#[path = "memory_snapshot/measurement.rs"]
mod measurement;
mod support;

use std::fs;
use std::process::Command;

use echidna::adapters::project_event_store::Project;
use echidna::adapters::tools::ToolRegistry;
use echidna::domain::tools::{
    ExecutionId, ToolCall, ToolCallId, ToolEffect, ToolExposure, ToolExposurePlan, ToolName,
    ToolParallelism, ToolResultStatus, ToolRetryPolicy,
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

fn read_args(path: &str, event_id: &str) -> Value {
    json!({
        "path": path,
        "event_id": event_id,
        "occurred_at": "2026-08-23T12:00:00Z"
    })
}

fn write_args(path: &str, content: &str, expected_hash: Value, event_id: &str) -> Value {
    json!({
        "path": path,
        "content": content,
        "expected_hash": expected_hash,
        "event_id": event_id,
        "occurred_at": "2026-08-23T12:00:00Z"
    })
}

#[tokio::test]
async fn memory_write_is_cas_protected_and_external_edits_are_audited_with_blobs() {
    let (_root, project) = initialized_project("memory-audit");
    let tools = ToolRegistry::for_project(project.root());

    let created = execute(
        &tools,
        "memory-create",
        "memory_write",
        write_args(
            "core.md",
            "# Target\nfixture\n",
            Value::Null,
            "memory-create",
        ),
    )
    .await;
    assert_eq!(created.result.status, ToolResultStatus::Succeeded);
    let initial_hash = created.result.output["new_hash"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        fs::read_to_string(project.root().join(".cyber-agent/memory/core.md")).unwrap(),
        "# Target\nfixture\n"
    );
    assert_eq!(
        created.result.output["diff_blob_id"]
            .as_str()
            .unwrap()
            .len(),
        64
    );
    assert_eq!(created.result.output["old_hash"], Value::Null);
    assert_eq!(created.result.output["new_hash"], initial_hash);
    let initial_event: Value = serde_json::from_str(
        fs::read_to_string(project.root().join(".cyber-agent/research.jsonl"))
            .unwrap()
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    let revision = &initial_event["payload"]["data"]["data"]["revision"];
    let blob_root = project.root().join(".cyber-agent/artifacts/blobs/sha256");
    assert_eq!(
        fs::read_to_string(blob_root.join(revision["new_blob_id"].as_str().unwrap())).unwrap(),
        "# Target\nfixture\n"
    );
    assert_eq!(
        fs::read_to_string(blob_root.join(revision["diff_blob_id"].as_str().unwrap())).unwrap(),
        "--- previous\n+++ current\n+# Target\n+fixture\n"
    );
    let blob_count = fs::read_dir(&blob_root).unwrap().count();
    let unchanged = execute(
        &tools,
        "memory-unchanged",
        "memory_write",
        write_args(
            "core.md",
            "# Target\nfixture\n",
            json!(initial_hash),
            "memory-unchanged",
        ),
    )
    .await;
    assert_eq!(unchanged.result.status, ToolResultStatus::Succeeded);
    assert_eq!(unchanged.result.output["status"], "unchanged");
    assert_eq!(unchanged.result.output["event_appended"], false);
    assert_eq!(fs::read_dir(&blob_root).unwrap().count(), blob_count);
    assert_eq!(
        fs::read_to_string(project.root().join(".cyber-agent/research.jsonl"))
            .unwrap()
            .lines()
            .count(),
        1
    );

    let duplicate_identity = execute(
        &tools,
        "memory-duplicate-identity",
        "memory_write",
        write_args(
            "core.md",
            "must not reuse the Event identity\n",
            json!(initial_hash),
            "memory-create",
        ),
    )
    .await;
    assert_eq!(duplicate_identity.result.status, ToolResultStatus::Failed);
    assert_eq!(
        duplicate_identity.receipt.exit_reason.as_deref(),
        Some("memory_event_id_duplicate")
    );
    assert_eq!(
        fs::read_to_string(project.root().join(".cyber-agent/memory/core.md")).unwrap(),
        "# Target\nfixture\n"
    );

    let stale = execute(
        &tools,
        "memory-stale",
        "memory_write",
        write_args(
            "core.md",
            "replacement\n",
            json!("0".repeat(64)),
            "memory-stale",
        ),
    )
    .await;
    assert_eq!(stale.result.status, ToolResultStatus::Failed);
    assert_eq!(
        stale.receipt.exit_reason.as_deref(),
        Some("memory_expected_hash_mismatch")
    );
    assert_eq!(
        fs::read_to_string(project.root().join(".cyber-agent/memory/core.md")).unwrap(),
        "# Target\nfixture\n"
    );

    fs::write(
        project.root().join(".cyber-agent/memory/core.md"),
        "# Target\nexternal editor\n",
    )
    .unwrap();
    let read = execute(
        &tools,
        "memory-read-external",
        "memory_read",
        read_args("core.md", "memory-read-external"),
    )
    .await;
    assert_eq!(read.result.status, ToolResultStatus::Succeeded);
    assert_eq!(read.result.output["external_change_detected"], true);
    assert_ne!(read.result.output["hash"], initial_hash);
    let rejected_external_overwrite = execute(
        &tools,
        "memory-overwrite-external",
        "memory_write",
        write_args(
            "core.md",
            "must not replace external edit\n",
            json!(initial_hash),
            "memory-overwrite-external",
        ),
    )
    .await;
    assert_eq!(
        rejected_external_overwrite.receipt.exit_reason.as_deref(),
        Some("memory_expected_hash_mismatch")
    );
    assert_eq!(
        fs::read_to_string(project.root().join(".cyber-agent/memory/core.md")).unwrap(),
        "# Target\nexternal editor\n"
    );
    let stream = fs::read_to_string(project.root().join(".cyber-agent/research.jsonl")).unwrap();
    assert!(stream.contains("memory.changed"));
    assert!(stream.contains("memory.external_change_detected"));
    assert!(stream.contains("diff_blob_id"));
}

#[tokio::test]
async fn core_memory_enforces_its_bounded_context_contract_before_blob_or_event_writes() {
    let (_root, project) = initialized_project("memory-core-limit");
    let tools = ToolRegistry::for_project(project.root());
    let oversized = "x".repeat(64 * 1024 + 1);
    let rejected = execute(
        &tools,
        "memory-core-oversized",
        "memory_write",
        write_args("core.md", &oversized, Value::Null, "memory-core-oversized"),
    )
    .await;
    assert_eq!(rejected.result.status, ToolResultStatus::Failed);
    assert_eq!(
        rejected.receipt.exit_reason.as_deref(),
        Some("memory_content_too_large")
    );
    assert!(!project.root().join(".cyber-agent/memory/core.md").exists());
    assert!(
        !project
            .root()
            .join(".cyber-agent/artifacts/blobs/sha256")
            .exists()
    );
    assert_eq!(
        fs::read_to_string(project.root().join(".cyber-agent/research.jsonl")).unwrap(),
        ""
    );

    fs::create_dir_all(project.root().join(".cyber-agent/memory")).unwrap();
    fs::write(
        project.root().join(".cyber-agent/memory/core.md"),
        &oversized,
    )
    .unwrap();
    let external = execute(
        &tools,
        "memory-core-external-oversized",
        "memory_read",
        read_args("core.md", "memory-core-external-oversized"),
    )
    .await;
    assert_eq!(external.result.status, ToolResultStatus::Failed);
    assert_eq!(
        external.receipt.exit_reason.as_deref(),
        Some("memory_read_failed")
    );
    assert_eq!(
        fs::read_to_string(project.root().join(".cyber-agent/research.jsonl")).unwrap(),
        ""
    );

    let note = execute(
        &tools,
        "memory-note-large",
        "memory_write",
        write_args(
            "notes/large.md",
            &oversized,
            Value::Null,
            "memory-note-large",
        ),
    )
    .await;
    assert_eq!(note.result.status, ToolResultStatus::Succeeded);
}

#[tokio::test]
async fn memory_paths_are_confined_and_research_replay_strictly_accepts_typed_memory_events() {
    let (_root, project) = initialized_project("memory-paths");
    let tools = ToolRegistry::for_project(project.root());
    assert!(
        tools
            .plan(
                ExecutionId::new("invalid-memory-plan"),
                call(
                    "invalid-memory-plan",
                    "memory_read",
                    read_args("notes/../escape.md", "memory-invalid-path"),
                ),
            )
            .is_err()
    );

    let written = execute(
        &tools,
        "memory-note",
        "memory_write",
        write_args(
            "notes/http.md",
            "request facts\n",
            Value::Null,
            "memory-note",
        ),
    )
    .await;
    assert_eq!(written.result.status, ToolResultStatus::Succeeded);
    let research = execute(&tools, "research-memory", "research_get", json!({})).await;
    assert_eq!(research.result.status, ToolResultStatus::Succeeded);
    assert_eq!(research.result.output["stream_event_count"], 1);

    let stream = project.root().join(".cyber-agent/research.jsonl");
    let mut bytes = fs::read(&stream).unwrap();
    bytes.extend_from_slice(
        br#"{"schema_family":"cyber-agent.event","schema_version":1,"event_id":"unknown-memory","stream_id":"research","sequence":2,"occurred_at":"2026-08-23T12:00:01Z","event_type":"memory.unknown","payload":{}}"#,
    );
    bytes.push(b'\n');
    fs::write(&stream, bytes).unwrap();
    let invalid_replay =
        execute(&tools, "research-invalid-memory", "research_get", json!({})).await;
    assert_eq!(invalid_replay.result.status, ToolResultStatus::Failed);
    assert_eq!(
        invalid_replay.receipt.exit_reason.as_deref(),
        Some("research_read_failed")
    );

    let (_unknown_root, unknown_project) = initialized_project("memory-unknown-field");
    let unknown_tools = ToolRegistry::for_project(unknown_project.root());
    let unknown_write = execute(
        &unknown_tools,
        "memory-unknown-write",
        "memory_write",
        write_args(
            "core.md",
            "strict payload\n",
            Value::Null,
            "memory-unknown-write",
        ),
    )
    .await;
    assert_eq!(unknown_write.result.status, ToolResultStatus::Succeeded);
    let unknown_stream = unknown_project.root().join(".cyber-agent/research.jsonl");
    let mut unknown_event: Value =
        serde_json::from_str(&fs::read_to_string(&unknown_stream).unwrap()).unwrap();
    unknown_event["payload"]["data"]["data"]["unexpected"] = json!(true);
    fs::write(
        &unknown_stream,
        format!("{}\n", serde_json::to_string(&unknown_event).unwrap()),
    )
    .unwrap();
    let unknown_replay = execute(
        &unknown_tools,
        "memory-unknown-replay",
        "research_get",
        json!({}),
    )
    .await;
    assert_eq!(unknown_replay.result.status, ToolResultStatus::Failed);
    assert_eq!(
        unknown_replay.receipt.exit_reason.as_deref(),
        Some("research_read_failed")
    );

    let (_version_root, version_project) = initialized_project("memory-version");
    let version_tools = ToolRegistry::for_project(version_project.root());
    let version_write = execute(
        &version_tools,
        "memory-version-write",
        "memory_write",
        write_args(
            "core.md",
            "version fixture\n",
            Value::Null,
            "memory-version-write",
        ),
    )
    .await;
    assert_eq!(version_write.result.status, ToolResultStatus::Succeeded);
    let version_stream = version_project.root().join(".cyber-agent/research.jsonl");
    let mut event: Value =
        serde_json::from_str(&fs::read_to_string(&version_stream).unwrap()).unwrap();
    event["payload"]["data"]["memory_schema_version"] = json!(2);
    fs::write(
        &version_stream,
        format!("{}\n", serde_json::to_string(&event).unwrap()),
    )
    .unwrap();
    let unsupported_version = execute(
        &version_tools,
        "research-version-memory",
        "research_get",
        json!({}),
    )
    .await;
    assert_eq!(unsupported_version.result.status, ToolResultStatus::Failed);
    assert_eq!(
        unsupported_version.receipt.exit_reason.as_deref(),
        Some("research_read_failed")
    );

    let (_chain_root, chain_project) = initialized_project("memory-chain");
    let chain_tools = ToolRegistry::for_project(chain_project.root());
    let chain_first = execute(
        &chain_tools,
        "memory-chain-first",
        "memory_write",
        write_args(
            "core.md",
            "chain first\n",
            Value::Null,
            "memory-chain-first",
        ),
    )
    .await;
    assert_eq!(chain_first.result.status, ToolResultStatus::Succeeded);
    let first_hash = chain_first.result.output["new_hash"].clone();
    let chain_second = execute(
        &chain_tools,
        "memory-chain-second",
        "memory_write",
        write_args(
            "core.md",
            "chain second\n",
            first_hash,
            "memory-chain-second",
        ),
    )
    .await;
    assert_eq!(chain_second.result.status, ToolResultStatus::Succeeded);
    let chain_stream = chain_project.root().join(".cyber-agent/research.jsonl");
    let mut chain_lines = fs::read_to_string(&chain_stream)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    chain_lines[1]["payload"]["data"]["data"]["revision"]["old_hash"] = json!("0".repeat(64));
    fs::write(
        &chain_stream,
        chain_lines
            .iter()
            .map(|line| serde_json::to_string(line).unwrap())
            .collect::<Vec<_>>()
            .join("\n")
            + "\n",
    )
    .unwrap();
    let broken_chain = execute(
        &chain_tools,
        "memory-chain-replay",
        "memory_read",
        read_args("core.md", "memory-chain-replay"),
    )
    .await;
    assert_eq!(broken_chain.result.status, ToolResultStatus::Failed);
    assert_eq!(
        broken_chain.receipt.exit_reason.as_deref(),
        Some("memory_state_invalid")
    );

    let (_duplicate_root, duplicate_project) = initialized_project("memory-duplicate-replay");
    let duplicate_tools = ToolRegistry::for_project(duplicate_project.root());
    let first = execute(
        &duplicate_tools,
        "memory-duplicate-first",
        "memory_write",
        write_args(
            "core.md",
            "duplicate fixture\n",
            Value::Null,
            "memory-duplicate-first",
        ),
    )
    .await;
    assert_eq!(first.result.status, ToolResultStatus::Succeeded);
    let duplicate_stream = duplicate_project.root().join(".cyber-agent/research.jsonl");
    let first_line = fs::read_to_string(&duplicate_stream).unwrap();
    let mut duplicate_event: Value = serde_json::from_str(first_line.trim()).unwrap();
    duplicate_event["sequence"] = json!(2);
    fs::write(
        &duplicate_stream,
        format!(
            "{}{}\n",
            first_line,
            serde_json::to_string(&duplicate_event).unwrap()
        ),
    )
    .unwrap();
    let duplicate_replay = execute(
        &duplicate_tools,
        "memory-duplicate-read",
        "memory_read",
        read_args("core.md", "memory-duplicate-read"),
    )
    .await;
    assert_eq!(duplicate_replay.result.status, ToolResultStatus::Succeeded);
    assert_eq!(duplicate_replay.result.output["stream_event_count"], 1);
}

#[cfg(unix)]
#[tokio::test]
async fn memory_adapter_rejects_symlinked_note_directories() {
    let (_root, project) = initialized_project("memory-symlink");
    let outside = TemporaryDirectory::new("memory-symlink-outside");
    fs::create_dir_all(project.root().join(".cyber-agent/memory")).unwrap();
    std::os::unix::fs::symlink(
        outside.path(),
        project.root().join(".cyber-agent/memory/notes"),
    )
    .unwrap();
    let tools = ToolRegistry::for_project(project.root());
    let outcome = execute(
        &tools,
        "memory-symlink",
        "memory_write",
        write_args(
            "notes/blocked.md",
            "must not escape\n",
            Value::Null,
            "memory-symlink",
        ),
    )
    .await;
    assert_eq!(outcome.result.status, ToolResultStatus::Failed);
    assert_eq!(
        outcome.receipt.exit_reason.as_deref(),
        Some("memory_read_failed")
    );
    assert!(!outside.path().join("blocked.md").exists());

    fs::write(outside.path().join("secret.md"), "outside\n").unwrap();
    let inspected = execute(
        &tools,
        "inspect-symlinked-memory",
        "inspect_project",
        json!({"observed_at": "2026-08-23T12:00:00Z"}),
    )
    .await;
    assert_eq!(inspected.result.status, ToolResultStatus::Succeeded);
    let entries = inspected.result.output["file_manifest"]["entries"]
        .as_array()
        .unwrap();
    assert!(entries.iter().any(|entry| {
        entry["relative_path"] == ".cyber-agent/memory" && entry["kind"] == "directory"
    }));
    assert!(!entries.iter().any(|entry| {
        entry["relative_path"]
            .as_str()
            .is_some_and(|path| path.ends_with("secret.md"))
    }));
}

#[tokio::test]
async fn inspect_project_reuses_complete_cache_and_rebuilds_changed_deleted_or_corrupt_state() {
    let (_root, project) = initialized_project("snapshot-cache");
    fs::write(project.root().join("alpha.txt"), "one\n").unwrap();
    fs::write(project.root().join("beta.txt"), "two\n").unwrap();
    let tools = ToolRegistry::for_project(project.root());
    let arguments = |hash_paths: Value| json!({"observed_at": "2026-08-23T12:00:00Z", "hash_paths": hash_paths});

    let first = execute(
        &tools,
        "inspect-first",
        "inspect_project",
        arguments(json!([])),
    )
    .await;
    assert_eq!(first.result.status, ToolResultStatus::Succeeded);
    assert_eq!(first.result.output["status"], "full_rebuild");
    assert_eq!(first.result.output["hashed_paths"], json!([]));
    assert_eq!(
        first.result.output["file_manifest"]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["relative_path"] == "alpha.txt")
            .unwrap()["blake3"],
        Value::Null
    );
    let cache = project.root().join(".cyber-agent/cache");
    for name in [
        "file-manifest.json",
        "tool-capabilities.json",
        "project-snapshot.json",
    ] {
        assert!(cache.join(name).is_file());
    }

    let hit = execute(
        &tools,
        "inspect-hit",
        "inspect_project",
        arguments(json!([])),
    )
    .await;
    assert_eq!(hit.result.status, ToolResultStatus::Succeeded);
    assert_eq!(hit.result.output["status"], "hit");
    assert_eq!(hit.result.output["hashed_paths"], json!([]));

    let missing_hash = execute(
        &tools,
        "inspect-missing-hash",
        "inspect_project",
        arguments(json!(["missing.txt"])),
    )
    .await;
    assert_eq!(missing_hash.result.status, ToolResultStatus::Failed);
    assert_eq!(
        missing_hash.receipt.exit_reason.as_deref(),
        Some("snapshot_inspection_failed")
    );

    let explicitly_hashed = execute(
        &tools,
        "inspect-hash-alpha",
        "inspect_project",
        arguments(json!(["./alpha.txt"])),
    )
    .await;
    assert_eq!(
        explicitly_hashed.result.output["status"],
        "incremental_refresh"
    );
    assert_eq!(
        explicitly_hashed.result.output["hashed_paths"],
        json!(["alpha.txt"])
    );
    assert_eq!(
        explicitly_hashed.result.output["file_manifest"]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["relative_path"] == "alpha.txt")
            .unwrap()["blake3"]
            .as_str()
            .unwrap()
            .len(),
        64
    );

    let reused_hash = execute(
        &tools,
        "inspect-reuse-alpha-hash",
        "inspect_project",
        arguments(json!(["alpha.txt"])),
    )
    .await;
    assert_eq!(reused_hash.result.output["status"], "hit");
    assert_eq!(
        reused_hash.result.output["hashed_paths"],
        json!(["alpha.txt"])
    );

    fs::write(
        project.root().join("alpha.txt"),
        "changed but same length\n",
    )
    .unwrap();
    let changed = execute(
        &tools,
        "inspect-changed",
        "inspect_project",
        arguments(json!([])),
    )
    .await;
    assert_eq!(changed.result.output["status"], "full_rebuild");
    assert_eq!(changed.result.output["hashed_paths"], json!([]));
    assert_eq!(
        changed.result.output["file_manifest"]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["relative_path"] == "alpha.txt")
            .unwrap()["blake3"],
        Value::Null
    );

    let rehashed = execute(
        &tools,
        "inspect-rehash-alpha",
        "inspect_project",
        arguments(json!(["alpha.txt"])),
    )
    .await;
    assert_eq!(rehashed.result.output["status"], "incremental_refresh");
    assert_eq!(rehashed.result.output["hashed_paths"], json!(["alpha.txt"]));

    fs::remove_file(project.root().join("beta.txt")).unwrap();
    let deleted = execute(
        &tools,
        "inspect-deleted",
        "inspect_project",
        arguments(json!([])),
    )
    .await;
    assert_eq!(deleted.result.output["status"], "full_rebuild");
    assert!(
        !deleted.result.output["file_manifest"]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["relative_path"] == "beta.txt")
    );

    fs::write(cache.join("tool-capabilities.json"), "not json").unwrap();
    let rebuilt = execute(
        &tools,
        "inspect-corrupt",
        "inspect_project",
        arguments(json!([])),
    )
    .await;
    assert_eq!(rebuilt.result.output["status"], "full_rebuild");
    assert!(
        fs::read_to_string(cache.join("tool-capabilities.json"))
            .unwrap()
            .contains("inspect_project")
    );

    fs::remove_dir_all(&cache).unwrap();
    let deleted_cache = execute(
        &tools,
        "inspect-deleted-cache",
        "inspect_project",
        arguments(json!([])),
    )
    .await;
    assert_eq!(deleted_cache.result.output["status"], "full_rebuild");
    assert_eq!(
        deleted_cache.result.output["rebuild_reason"],
        "cache_absent"
    );

    let snapshot_path = cache.join("project-snapshot.json");
    let mut snapshot: Value =
        serde_json::from_str(&fs::read_to_string(&snapshot_path).unwrap()).unwrap();
    snapshot["cache_schema_version"] = json!(999);
    fs::write(
        &snapshot_path,
        serde_json::to_vec_pretty(&snapshot).unwrap(),
    )
    .unwrap();
    let incompatible = execute(
        &tools,
        "inspect-incompatible-cache",
        "inspect_project",
        arguments(json!([])),
    )
    .await;
    assert_eq!(incompatible.result.output["status"], "full_rebuild");
    assert_eq!(
        incompatible.result.output["rebuild_reason"],
        "snapshot_schema_incompatible"
    );
}

#[tokio::test]
async fn inspect_project_never_uses_cache_to_replace_a_missing_research_fact() {
    let (_root, project) = initialized_project("snapshot-required-facts");
    fs::write(project.root().join("target.txt"), "fixture\n").unwrap();
    let tools = ToolRegistry::for_project(project.root());
    let arguments = json!({"observed_at": "2026-08-23T12:00:00Z"});

    let first = execute(
        &tools,
        "inspect-required-facts-first",
        "inspect_project",
        arguments.clone(),
    )
    .await;
    assert_eq!(first.result.status, ToolResultStatus::Succeeded);
    assert!(
        project
            .root()
            .join(".cyber-agent/cache/project-snapshot.json")
            .is_file()
    );

    fs::remove_file(project.root().join(".cyber-agent/research.jsonl")).unwrap();
    let missing = execute(
        &tools,
        "inspect-required-facts-missing",
        "inspect_project",
        arguments,
    )
    .await;
    assert_eq!(missing.result.status, ToolResultStatus::Failed);
    assert_eq!(
        missing.receipt.exit_reason.as_deref(),
        Some("snapshot_inspection_failed")
    );
}

#[tokio::test]
async fn git_fingerprint_excludes_cache_writes_but_detects_tracked_changes() {
    let root = TemporaryDirectory::new("snapshot-git");
    assert!(
        Command::new("git")
            .arg("init")
            .arg(root.path())
            .status()
            .unwrap()
            .success()
    );
    fs::write(root.path().join("tracked.txt"), "initial\n").unwrap();
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(root.path())
            .args(["add", "tracked.txt"])
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(root.path())
            .args([
                "-c",
                "user.name=Cyber Agent Test",
                "-c",
                "user.email=cyber-agent@example.invalid",
                "commit",
                "-m",
                "fixture",
            ])
            .status()
            .unwrap()
            .success()
    );
    let project = Project::initialize(root.path()).unwrap();
    let tools = ToolRegistry::for_project(project.root());
    let arguments = json!({"observed_at": "2026-08-23T12:00:00Z"});

    let first = execute(
        &tools,
        "git-inspect-first",
        "inspect_project",
        arguments.clone(),
    )
    .await;
    assert_eq!(first.result.output["status"], "full_rebuild");
    let second = execute(
        &tools,
        "git-inspect-hit",
        "inspect_project",
        arguments.clone(),
    )
    .await;
    assert_eq!(second.result.output["status"], "hit");

    fs::write(
        project.root().join(".cyber-agent/cache/derived-noise"),
        "ignored derived bytes\n",
    )
    .unwrap();
    let cache_noise = execute(
        &tools,
        "git-inspect-cache-noise",
        "inspect_project",
        arguments.clone(),
    )
    .await;
    assert_eq!(cache_noise.result.output["status"], "hit");

    fs::write(project.root().join("tracked.txt"), "changed\n").unwrap();
    let changed = execute(
        &tools,
        "git-inspect-changed",
        "inspect_project",
        arguments.clone(),
    )
    .await;
    assert_eq!(changed.result.output["status"], "full_rebuild");

    // Porcelain still reports `.M` after a second edit. The cheap Git
    // fingerprint must also include metadata for dirty paths so that this does
    // not become a false cache hit without traversing the clean project tree.
    fs::write(
        project.root().join("tracked.txt"),
        "changed again with a different size\n",
    )
    .unwrap();
    let changed_again = execute(
        &tools,
        "git-inspect-changed-again",
        "inspect_project",
        arguments,
    )
    .await;
    assert_eq!(changed_again.result.output["status"], "full_rebuild");
}

#[tokio::test]
async fn project_memory_and_snapshot_caches_are_isolated() {
    let (_first_root, first) = initialized_project("memory-isolation-first");
    let (_second_root, second) = initialized_project("memory-isolation-second");
    let first_tools = ToolRegistry::for_project(first.root());
    let second_tools = ToolRegistry::for_project(second.root());
    let first_write = execute(
        &first_tools,
        "first-memory",
        "memory_write",
        write_args("decisions.md", "first only\n", Value::Null, "first-memory"),
    )
    .await;
    assert_eq!(first_write.result.status, ToolResultStatus::Succeeded);
    let second_read = execute(
        &second_tools,
        "second-memory",
        "memory_read",
        read_args("decisions.md", "second-memory"),
    )
    .await;
    assert_eq!(second_read.result.output["content"], Value::Null);

    let first_snapshot = execute(
        &first_tools,
        "first-inspect",
        "inspect_project",
        json!({"observed_at": "2026-08-23T12:00:00Z"}),
    )
    .await;
    assert_eq!(first_snapshot.result.status, ToolResultStatus::Succeeded);
    assert!(
        first
            .root()
            .join(".cyber-agent/cache/project-snapshot.json")
            .is_file()
    );
    assert!(
        !second
            .root()
            .join(".cyber-agent/cache/project-snapshot.json")
            .exists()
    );
    let second_snapshot = execute(
        &second_tools,
        "second-inspect",
        "inspect_project",
        json!({"observed_at": "2026-08-23T12:00:00Z"}),
    )
    .await;
    assert_eq!(second_snapshot.result.status, ToolResultStatus::Succeeded);
    assert_ne!(
        first_snapshot.result.output["snapshot"]["project_id"],
        second_snapshot.result.output["snapshot"]["project_id"]
    );
    assert_ne!(
        fs::read(
            first
                .root()
                .join(".cyber-agent/cache/project-snapshot.json")
        )
        .unwrap(),
        fs::read(
            second
                .root()
                .join(".cyber-agent/cache/project-snapshot.json")
        )
        .unwrap()
    );
}

#[test]
fn memory_and_snapshot_tool_specs_are_fixed_and_non_cacheable() {
    let (_root, project) = initialized_project("memory-snapshot-tool-contract");
    let tools = ToolRegistry::for_project(project.root());
    let resolved = tools
        .resolve_tool_plan(
            &ToolExposurePlan::new([
                (ToolName::new("memory_read"), ToolExposure::Direct),
                (ToolName::new("memory_write"), ToolExposure::Direct),
            ]),
            &Default::default(),
        )
        .unwrap();
    for name in ["memory_read", "memory_write", "inspect_project"] {
        let spec = resolved
            .advertised_specs()
            .iter()
            .find(|spec| spec.name.as_str() == name)
            .unwrap();
        assert_eq!(spec.version, "1");
        assert_eq!(spec.parallelism, ToolParallelism::ResourceLocked);
        assert_eq!(spec.effect, ToolEffect::LocalMutation);
        assert_eq!(spec.retry_policy, ToolRetryPolicy::NeverAutoRetry);
        assert_eq!(
            spec.cache_policy,
            echidna::domain::tools::ToolCachePolicy::None
        );
    }
    let memory_write = resolved
        .advertised_specs()
        .iter()
        .find(|spec| spec.name.as_str() == "memory_write")
        .unwrap();
    assert_eq!(
        memory_write.input_schema["allOf"][0]["then"]["properties"]["content"]["maxLength"],
        64 * 1024
    );
}
