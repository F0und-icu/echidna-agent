mod support;

use std::fs;

use echidna::adapters::project_event_store::Project;
use echidna::adapters::tools::ToolRegistry;
use echidna::domain::events::{
    EventEnvelope, EventId, EventSequence, EventType, OccurredAt, StreamId,
};
use echidna::domain::tools::{
    ExecutionId, ResourceKey, ToolCachePolicy, ToolCall, ToolCallId, ToolDeterminism, ToolEffect,
    ToolExposure, ToolExposurePlan, ToolName, ToolParallelism, ToolResultStatus, ToolRetryPolicy,
};
use echidna::ports::tool::ToolExecutor;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use support::TemporaryDirectory;

fn initialized_project(label: &str) -> (TemporaryDirectory, Project) {
    let directory = TemporaryDirectory::new(label);
    let project = Project::initialize(directory.path()).expect("project should initialize");
    (directory, project)
}

fn source_path(project: &Project, name: &str, bytes: &[u8]) -> String {
    let path = project.root().join(name);
    fs::write(&path, bytes).expect("source should be writable");
    path.display().to_string()
}

fn tool_call(id: &str, name: &str, arguments: Value) -> ToolCall {
    ToolCall::new(ToolCallId::new(id), ToolName::new(name), arguments)
}

async fn execute(
    tools: &ToolRegistry,
    call_id: &str,
    name: &str,
    arguments: Value,
) -> echidna::domain::tools::ToolExecutionOutcome {
    let plan = tools
        .plan(
            ExecutionId::new(format!("execution-{call_id}")),
            tool_call(call_id, name, arguments),
        )
        .expect("Tool plan should be valid");
    tools.execute(plan).await
}

async fn record_target_snapshot(tools: &ToolRegistry, snapshot_id: &str, event_id: &str) {
    let outcome = execute(
        tools,
        &format!("call-{event_id}"),
        "research_update",
        json!({
            "event_id": event_id,
            "occurred_at": "2026-08-23T00:00:00Z",
            "operation": {
                "kind": "record_target_snapshot",
                "record": {
                    "id": snapshot_id,
                    "target_id": "fixture.target",
                    "kind": "source",
                    "locator": "fixture/source",
                    "version": "1.0.0",
                    "build_id": "build-1",
                    "target_artifact_ids": ["artifact-root"],
                    "environment_fingerprint": "env-a",
                    "device_fingerprint": "device-a",
                    "captured_at": "2026-08-23T00:00:00Z"
                }
            }
        }),
    )
    .await;
    assert_eq!(outcome.result.status, ToolResultStatus::Succeeded);
    assert_eq!(outcome.result.output["status"], "recorded");
}

fn artifact_args(
    event_id: &str,
    artifact_id: &str,
    source_path: &str,
    target_snapshot_id: &str,
) -> Value {
    json!({
        "event_id": event_id,
        "occurred_at": "2026-08-23T00:01:00Z",
        "artifact_id": artifact_id,
        "source_path": source_path,
        "media_type": "application/octet-stream",
        "target_snapshot_id": target_snapshot_id,
        "toolchain_fingerprint": "fixture-toolchain-v1"
    })
}

fn research_line_count(project: &Project) -> usize {
    fs::read_to_string(project.root().join(".cyber-agent/research.jsonl"))
        .expect("Research stream should be readable")
        .lines()
        .count()
}

fn sha256_path(project: &Project, bytes: &[u8]) -> std::path::PathBuf {
    let digest = format!("{:x}", Sha256::digest(bytes));
    project
        .root()
        .join(".cyber-agent/artifacts/blobs/sha256")
        .join(digest)
}

fn artifact_event_envelope(
    event_id: &str,
    sequence: u64,
    artifact_id: &str,
    created_by_event_id: &str,
) -> EventEnvelope<Value> {
    EventEnvelope::current(
        EventId::new(event_id),
        StreamId::new("research"),
        EventSequence::new(sequence),
        OccurredAt::new("2026-08-23T00:01:00Z"),
        EventType::new("artifact.created"),
        json!({
            "research_schema_version": 1,
            "data": {
                "artifact_schema_version": 1,
                "record": {
                    "id": artifact_id,
                    "blob_id": "0000000000000000000000000000000000000000000000000000000000000000",
                    "media_type": "application/octet-stream",
                    "size_bytes": 0,
                    "source_path": "fixture.bin",
                    "created_by_event_id": created_by_event_id,
                    "produced_by_tool_call_id": "call-artifact",
                    "target_snapshot_id": "snapshot-a",
                    "toolchain_fingerprint": "fixture-toolchain"
                }
            }
        }),
    )
}

fn append_raw_events(project: &Project, events: &[EventEnvelope<Value>]) {
    let stream = project.root().join(".cyber-agent/research.jsonl");
    let mut bytes = fs::read(&stream).unwrap();
    for event in events {
        bytes.extend_from_slice(&serde_json::to_vec(event).unwrap());
        bytes.push(b'\n');
    }
    fs::write(stream, bytes).unwrap();
}

#[test]
fn artifact_store_schema_effect_retry_and_resources_are_fixed() {
    let (_directory, project) = initialized_project("artifact-tool-contract");
    let source = source_path(&project, "source.bin", b"source bytes");
    let tools = ToolRegistry::for_project(project.root());
    assert!(
        tools
            .specs()
            .iter()
            .all(|spec| spec.name.as_str() != "artifact_store")
    );
    let resolved = tools
        .resolve_tool_plan(
            &ToolExposurePlan::new([(ToolName::new("artifact_store"), ToolExposure::Direct)]),
            &Default::default(),
        )
        .unwrap();
    let spec = resolved
        .advertised_specs()
        .iter()
        .find(|spec| spec.name.as_str() == "artifact_store")
        .expect("artifact_store should be model-visible only after the Mode exposes it");
    assert_eq!(spec.version, "1");
    assert_eq!(spec.exposure, ToolExposure::Direct);
    assert_eq!(spec.parallelism, ToolParallelism::ResourceLocked);
    assert_eq!(spec.effect, ToolEffect::LocalMutation);
    assert_eq!(spec.retry_policy, ToolRetryPolicy::NeverAutoRetry);
    assert_eq!(spec.determinism, ToolDeterminism::EnvironmentDependent);
    assert_eq!(spec.cache_policy, ToolCachePolicy::None);
    assert_eq!(
        spec.resource_key_template.as_deref(),
        Some("file:{source_path},file:.cyber-agent/research.jsonl")
    );
    assert_eq!(
        spec.input_schema["required"],
        json!([
            "event_id",
            "occurred_at",
            "artifact_id",
            "source_path",
            "media_type",
            "target_snapshot_id",
            "toolchain_fingerprint"
        ])
    );
    assert_eq!(spec.input_schema["additionalProperties"], false);
    assert_eq!(
        spec.input_schema["properties"]["parent_artifact_ids"]["items"]["minLength"],
        1
    );

    let plan = tools
        .plan(
            ExecutionId::new("artifact-plan"),
            tool_call(
                "artifact-call",
                "artifact_store",
                artifact_args("event-artifact", "artifact-one", &source, "snapshot-a"),
            ),
        )
        .expect("artifact_store plan should normalize its fixed resources");
    assert_eq!(
        plan.resolved_resource_keys,
        vec![
            ResourceKey::file(
                project
                    .root()
                    .join(".cyber-agent/research.jsonl")
                    .display()
                    .to_string()
            ),
            ResourceKey::file(source),
        ]
    );
}

#[tokio::test]
async fn artifact_event_versions_and_unknown_types_are_strict_on_replay() {
    let (_directory, project) = initialized_project("artifact-event-version");
    let tools = ToolRegistry::for_project(project.root());
    record_target_snapshot(&tools, "snapshot-a", "event-snapshot-a").await;
    let stream = project.root().join(".cyber-agent/research.jsonl");
    let malformed_artifact = EventEnvelope::current(
        EventId::new("event-unsupported-artifact-version"),
        StreamId::new("research"),
        EventSequence::new(2),
        OccurredAt::new("2026-08-23T00:01:00Z"),
        EventType::new("artifact.created"),
        json!({
            "research_schema_version": 1,
            "data": {
                "artifact_schema_version": 2,
                "record": {
                    "id": "artifact-versioned",
                    "blob_id": "0000000000000000000000000000000000000000000000000000000000000000",
                    "media_type": "application/octet-stream",
                    "size_bytes": 0,
                    "source_path": "fixture.bin",
                    "created_by_event_id": "event-unsupported-artifact-version",
                    "produced_by_tool_call_id": "call-artifact",
                    "target_snapshot_id": "snapshot-a",
                    "toolchain_fingerprint": "fixture-toolchain"
                }
            }
        }),
    );
    let mut bytes = fs::read(&stream).unwrap();
    bytes.extend_from_slice(&serde_json::to_vec(&malformed_artifact).unwrap());
    bytes.push(b'\n');
    fs::write(&stream, bytes).unwrap();
    let unsupported_version =
        execute(&tools, "artifact-version-get", "research_get", json!({})).await;
    assert_eq!(unsupported_version.result.status, ToolResultStatus::Failed);
    assert_eq!(
        unsupported_version.receipt.exit_reason.as_deref(),
        Some("research_read_failed")
    );

    let (_directory, project) = initialized_project("artifact-event-unknown");
    let tools = ToolRegistry::for_project(project.root());
    record_target_snapshot(&tools, "snapshot-a", "event-snapshot-a").await;
    let stream = project.root().join(".cyber-agent/research.jsonl");
    let unknown_artifact = EventEnvelope::current(
        EventId::new("event-unknown-artifact-type"),
        StreamId::new("research"),
        EventSequence::new(2),
        OccurredAt::new("2026-08-23T00:01:00Z"),
        EventType::new("artifact.unknown"),
        json!({"research_schema_version": 1, "data": {}}),
    );
    let mut bytes = fs::read(&stream).unwrap();
    bytes.extend_from_slice(&serde_json::to_vec(&unknown_artifact).unwrap());
    bytes.push(b'\n');
    fs::write(&stream, bytes).unwrap();
    let unknown_type = execute(&tools, "artifact-unknown-get", "research_get", json!({})).await;
    assert_eq!(unknown_type.result.status, ToolResultStatus::Failed);
    assert_eq!(
        unknown_type.receipt.exit_reason.as_deref(),
        Some("research_read_failed")
    );

    let (_directory, project) = initialized_project("artifact-event-identity-mismatch");
    let tools = ToolRegistry::for_project(project.root());
    record_target_snapshot(&tools, "snapshot-a", "event-snapshot-a").await;
    append_raw_events(
        &project,
        &[artifact_event_envelope(
            "event-artifact-envelope",
            2,
            "artifact-mismatched-event",
            "different-created-by-event",
        )],
    );
    let mismatched_identity = execute(
        &tools,
        "artifact-identity-mismatch-get",
        "research_get",
        json!({}),
    )
    .await;
    assert_eq!(mismatched_identity.result.status, ToolResultStatus::Failed);

    let (_directory, project) = initialized_project("artifact-event-id-conflict");
    let tools = ToolRegistry::for_project(project.root());
    record_target_snapshot(&tools, "snapshot-a", "event-snapshot-a").await;
    append_raw_events(
        &project,
        &[
            artifact_event_envelope(
                "event-artifact-conflict",
                2,
                "artifact-conflict-one",
                "event-artifact-conflict",
            ),
            artifact_event_envelope(
                "event-artifact-conflict",
                3,
                "artifact-conflict-two",
                "event-artifact-conflict",
            ),
        ],
    );
    let event_id_conflict = execute(
        &tools,
        "artifact-event-id-conflict-get",
        "research_get",
        json!({}),
    )
    .await;
    assert_eq!(event_id_conflict.result.status, ToolResultStatus::Failed);
    assert!(
        event_id_conflict.result.output["error"]
            .as_str()
            .unwrap()
            .contains("conflicts with different validated semantics")
    );
}

#[tokio::test]
async fn same_blob_keeps_two_artifact_provenance_records_and_a_derived_chain() {
    let (_directory, project) = initialized_project("artifact-dedup-lineage");
    let tools = ToolRegistry::for_project(project.root());
    record_target_snapshot(&tools, "snapshot-a", "event-snapshot-a").await;
    let first_source = source_path(&project, "first.bin", b"same bytes");
    let second_source = source_path(&project, "second.bin", b"same bytes");
    let derived_source = source_path(&project, "derived.bin", b"derived bytes");

    let first = execute(
        &tools,
        "artifact-call-one",
        "artifact_store",
        artifact_args(
            "event-artifact-one",
            "artifact-one",
            &first_source,
            "snapshot-a",
        ),
    )
    .await;
    assert_eq!(first.result.status, ToolResultStatus::Succeeded);
    assert_eq!(first.result.output["blob"]["already_present"], false);
    assert_eq!(
        first.receipt.output_artifact_ids,
        vec![echidna::domain::artifacts::ArtifactId::new("artifact-one")]
    );

    let second = execute(
        &tools,
        "artifact-call-two",
        "artifact_store",
        artifact_args(
            "event-artifact-two",
            "artifact-two",
            &second_source,
            "snapshot-a",
        ),
    )
    .await;
    assert_eq!(second.result.status, ToolResultStatus::Succeeded);
    assert_eq!(second.result.output["blob"]["already_present"], true);
    assert_eq!(
        first.result.output["blob"]["blob_id"],
        second.result.output["blob"]["blob_id"]
    );
    let blob_path = sha256_path(&project, b"same bytes");
    assert!(blob_path.is_file());
    assert_eq!(fs::read(&blob_path).unwrap(), b"same bytes");

    let mut derived = artifact_args(
        "event-artifact-derived",
        "artifact-derived",
        &derived_source,
        "snapshot-a",
    );
    derived["parent_artifact_ids"] = json!(["artifact-one"]);
    derived["transform"] = json!("normalized-fixture-v1");
    let derived = execute(&tools, "artifact-call-derived", "artifact_store", derived).await;
    assert_eq!(derived.result.status, ToolResultStatus::Succeeded);
    assert_eq!(derived.result.output["event_type"], "artifact.derived");
    assert_eq!(
        derived.result.output["artifact"]["parent_artifact_ids"],
        json!(["artifact-one"])
    );
    assert_eq!(
        derived.result.output["artifact"]["produced_by_tool_call_id"],
        "artifact-call-derived"
    );
    assert_eq!(
        derived.result.output["artifact"]["target_snapshot_id"],
        "snapshot-a"
    );

    let before_conflicts = research_line_count(&project);
    let conflicting_source = source_path(&project, "conflicting.bin", b"conflicting bytes");
    let duplicate_event = execute(
        &tools,
        "artifact-duplicate-event-call",
        "artifact_store",
        artifact_args(
            "event-artifact-one",
            "artifact-duplicate-event",
            &conflicting_source,
            "snapshot-a",
        ),
    )
    .await;
    assert_eq!(duplicate_event.result.status, ToolResultStatus::Failed);
    assert_eq!(
        duplicate_event.receipt.exit_reason.as_deref(),
        Some("artifact_event_id_duplicate")
    );
    let conflicting_id = execute(
        &tools,
        "artifact-conflicting-id-call",
        "artifact_store",
        artifact_args(
            "event-artifact-conflicting-id",
            "artifact-one",
            &conflicting_source,
            "snapshot-a",
        ),
    )
    .await;
    assert_eq!(conflicting_id.result.status, ToolResultStatus::Failed);
    assert_eq!(
        conflicting_id.receipt.exit_reason.as_deref(),
        Some("artifact_provenance_rejected")
    );
    assert_eq!(research_line_count(&project), before_conflicts);
    assert!(!sha256_path(&project, b"conflicting bytes").exists());

    let state = execute(&tools, "artifact-get", "research_get", json!({})).await;
    assert_eq!(state.result.status, ToolResultStatus::Succeeded);
    assert_eq!(
        state.result.output["state"]["artifacts"]["records"]
            .as_object()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(research_line_count(&project), 4);
}

#[tokio::test]
async fn invalid_artifact_provenance_rejects_without_an_event_or_blob() {
    let (_directory, project) = initialized_project("artifact-provenance-rejection");
    let tools = ToolRegistry::for_project(project.root());
    record_target_snapshot(&tools, "snapshot-a", "event-snapshot-a").await;
    let source = source_path(&project, "source.bin", b"source bytes");
    let before = research_line_count(&project);

    let missing_target = execute(
        &tools,
        "missing-target-call",
        "artifact_store",
        artifact_args(
            "event-missing-target",
            "artifact-missing-target",
            &source,
            "missing",
        ),
    )
    .await;
    assert_eq!(missing_target.result.status, ToolResultStatus::Failed);
    assert_eq!(
        missing_target.receipt.exit_reason.as_deref(),
        Some("artifact_provenance_rejected")
    );

    let mut missing_parent = artifact_args(
        "event-missing-parent",
        "artifact-missing-parent",
        &source,
        "snapshot-a",
    );
    missing_parent["parent_artifact_ids"] = json!(["absent-parent"]);
    missing_parent["transform"] = json!("derive");
    let missing_parent = execute(
        &tools,
        "missing-parent-call",
        "artifact_store",
        missing_parent,
    )
    .await;
    assert_eq!(missing_parent.result.status, ToolResultStatus::Failed);

    let mut self_parent =
        artifact_args("event-self-parent", "artifact-self", &source, "snapshot-a");
    self_parent["parent_artifact_ids"] = json!(["artifact-self"]);
    self_parent["transform"] = json!("derive");
    let self_parent = execute(&tools, "self-parent-call", "artifact_store", self_parent).await;
    assert_eq!(self_parent.result.status, ToolResultStatus::Failed);

    let mut duplicated_parent = artifact_args(
        "event-duplicate-parent",
        "artifact-duplicate",
        &source,
        "snapshot-a",
    );
    duplicated_parent["parent_artifact_ids"] = json!(["artifact-root", "artifact-root"]);
    duplicated_parent["transform"] = json!("derive");
    let duplicated_parent = execute(
        &tools,
        "duplicate-parent-call",
        "artifact_store",
        duplicated_parent,
    )
    .await;
    assert_eq!(duplicated_parent.result.status, ToolResultStatus::Failed);

    let mut derived_without_transform = artifact_args(
        "event-derived-without-transform",
        "artifact-derived-without-transform",
        &source,
        "snapshot-a",
    );
    derived_without_transform["parent_artifact_ids"] = json!(["artifact-root"]);
    let derived_without_transform = execute(
        &tools,
        "derived-without-transform-call",
        "artifact_store",
        derived_without_transform,
    )
    .await;
    assert_eq!(
        derived_without_transform.result.status,
        ToolResultStatus::Failed
    );

    let mut root_with_transform = artifact_args(
        "event-root-with-transform",
        "artifact-root-with-transform",
        &source,
        "snapshot-a",
    );
    root_with_transform["transform"] = json!("invalid-root-transform");
    let root_with_transform = execute(
        &tools,
        "root-with-transform-call",
        "artifact_store",
        root_with_transform,
    )
    .await;
    assert_eq!(root_with_transform.result.status, ToolResultStatus::Failed);

    assert_eq!(research_line_count(&project), before);
    assert!(!sha256_path(&project, b"source bytes").exists());
}

#[tokio::test]
async fn conflicting_existing_blob_and_non_regular_sources_never_append_events() {
    let (_directory, project) = initialized_project("artifact-blob-conflicts");
    let tools = ToolRegistry::for_project(project.root());
    record_target_snapshot(&tools, "snapshot-a", "event-snapshot-a").await;
    let source = source_path(&project, "source.bin", b"source bytes");
    let blob_path = sha256_path(&project, b"source bytes");
    fs::create_dir_all(blob_path.parent().unwrap()).unwrap();
    fs::write(&blob_path, b"wrong bytes").unwrap();

    let conflicting = execute(
        &tools,
        "conflicting-blob-call",
        "artifact_store",
        artifact_args(
            "event-conflicting",
            "artifact-conflicting",
            &source,
            "snapshot-a",
        ),
    )
    .await;
    assert_eq!(conflicting.result.status, ToolResultStatus::Failed);
    assert_eq!(
        conflicting.receipt.exit_reason.as_deref(),
        Some("artifact_blob_publish_failed")
    );
    assert_eq!(fs::read(&blob_path).unwrap(), b"wrong bytes");
    assert_eq!(research_line_count(&project), 1);

    fs::remove_file(&blob_path).unwrap();
    fs::create_dir(&blob_path).unwrap();
    let non_regular = execute(
        &tools,
        "non-regular-blob-call",
        "artifact_store",
        artifact_args(
            "event-non-regular",
            "artifact-non-regular",
            &source,
            "snapshot-a",
        ),
    )
    .await;
    assert_eq!(non_regular.result.status, ToolResultStatus::Failed);
    fs::remove_dir(&blob_path).unwrap();

    let source_directory = project.root().join("source-directory");
    fs::create_dir(&source_directory).unwrap();
    let non_regular_source = execute(
        &tools,
        "non-regular-source-call",
        "artifact_store",
        artifact_args(
            "event-non-regular-source",
            "artifact-non-regular-source",
            &source_directory.display().to_string(),
            "snapshot-a",
        ),
    )
    .await;
    assert_eq!(non_regular_source.result.status, ToolResultStatus::Failed);
    assert_eq!(research_line_count(&project), 1);
}

#[cfg(unix)]
#[tokio::test]
async fn symlinked_sources_and_blob_paths_are_rejected_without_leaving_artifact_facts() {
    use std::os::unix::fs::symlink;

    let (_directory, project) = initialized_project("artifact-symlink-rejection");
    let tools = ToolRegistry::for_project(project.root());
    record_target_snapshot(&tools, "snapshot-a", "event-snapshot-a").await;
    let target = source_path(&project, "target.bin", b"source bytes");
    let source_link = project.root().join("source-link.bin");
    symlink(&target, &source_link).unwrap();
    let source_link_outcome = execute(
        &tools,
        "source-link-call",
        "artifact_store",
        artifact_args(
            "event-source-link",
            "artifact-source-link",
            &source_link.display().to_string(),
            "snapshot-a",
        ),
    )
    .await;
    assert_eq!(source_link_outcome.result.status, ToolResultStatus::Failed);

    let blob_path = sha256_path(&project, b"source bytes");
    fs::create_dir_all(blob_path.parent().unwrap()).unwrap();
    let external = project.root().join("external.bin");
    fs::write(&external, b"source bytes").unwrap();
    symlink(&external, &blob_path).unwrap();
    let blob_link_outcome = execute(
        &tools,
        "blob-link-call",
        "artifact_store",
        artifact_args(
            "event-blob-link",
            "artifact-blob-link",
            &target,
            "snapshot-a",
        ),
    )
    .await;
    assert_eq!(blob_link_outcome.result.status, ToolResultStatus::Failed);
    assert_eq!(research_line_count(&project), 1);

    fs::remove_file(&blob_path).unwrap();
    let artifacts_directory = project.root().join(".cyber-agent/artifacts");
    fs::remove_dir_all(&artifacts_directory).unwrap();
    let external_directory = project.root().join("external-artifacts");
    fs::create_dir(&external_directory).unwrap();
    symlink(&external_directory, &artifacts_directory).unwrap();
    let ancestor_link_outcome = execute(
        &tools,
        "ancestor-link-call",
        "artifact_store",
        artifact_args(
            "event-ancestor-link",
            "artifact-ancestor-link",
            &target,
            "snapshot-a",
        ),
    )
    .await;
    assert_eq!(
        ancestor_link_outcome.result.status,
        ToolResultStatus::Failed
    );
    assert_eq!(fs::read_dir(&external_directory).unwrap().count(), 0);
    fs::remove_file(&artifacts_directory).unwrap();
    fs::write(&artifacts_directory, b"not a directory").unwrap();
    let ancestor_file_outcome = execute(
        &tools,
        "ancestor-file-call",
        "artifact_store",
        artifact_args(
            "event-ancestor-file",
            "artifact-ancestor-file",
            &target,
            "snapshot-a",
        ),
    )
    .await;
    assert_eq!(
        ancestor_file_outcome.result.status,
        ToolResultStatus::Failed
    );
    assert_eq!(research_line_count(&project), 1);
}

#[tokio::test]
async fn artifacts_are_project_isolated_and_remain_facts_after_cache_deletion() {
    let (_first_directory, first) = initialized_project("artifact-project-first");
    let (_second_directory, second) = initialized_project("artifact-project-second");
    let first_tools = ToolRegistry::for_project(first.root());
    let second_tools = ToolRegistry::for_project(second.root());
    record_target_snapshot(&first_tools, "snapshot-a", "event-first-snapshot").await;
    record_target_snapshot(&second_tools, "snapshot-a", "event-second-snapshot").await;
    let first_source = source_path(&first, "source.bin", b"same bytes");
    let second_source = source_path(&second, "source.bin", b"same bytes");

    let first_outcome = execute(
        &first_tools,
        "first-artifact-call",
        "artifact_store",
        artifact_args(
            "event-first-artifact",
            "artifact-one",
            &first_source,
            "snapshot-a",
        ),
    )
    .await;
    let second_outcome = execute(
        &second_tools,
        "second-artifact-call",
        "artifact_store",
        artifact_args(
            "event-second-artifact",
            "artifact-one",
            &second_source,
            "snapshot-a",
        ),
    )
    .await;
    assert_eq!(first_outcome.result.status, ToolResultStatus::Succeeded);
    assert_eq!(second_outcome.result.status, ToolResultStatus::Succeeded);
    assert_ne!(
        sha256_path(&first, b"same bytes"),
        sha256_path(&second, b"same bytes")
    );

    let cache = first.root().join(".cyber-agent/cache");
    fs::create_dir_all(&cache).unwrap();
    fs::write(cache.join("derived.json"), b"not a fact").unwrap();
    fs::remove_dir_all(cache).unwrap();
    let replay = execute(&first_tools, "first-replay", "research_get", json!({})).await;
    assert_eq!(replay.result.status, ToolResultStatus::Succeeded);
    assert_eq!(
        replay.result.output["state"]["artifacts"]["records"]
            .as_object()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        fs::read(sha256_path(&first, b"same bytes")).unwrap(),
        b"same bytes"
    );
}
