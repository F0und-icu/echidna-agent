mod support;

use std::fs;

use echidna::adapters::project_event_store::Project;
use echidna::adapters::tools::ToolRegistry;
use echidna::domain::agent_definition::AgentId;
use echidna::domain::artifacts::{
    ArtifactEvent, ArtifactId, ArtifactRecord, ArtifactScope, BlobId,
};
use echidna::domain::events::{
    EventEnvelope, EventId, EventSequence, EventType, OccurredAt, StreamId,
};
use echidna::domain::memory::MemoryScope;
use echidna::domain::research::{
    ResearchError, ResearchEvent, ResearchReducer, TargetKind, TargetSnapshotDraft,
    TargetSnapshotId, TargetSnapshotRecord,
};
use echidna::domain::tools::{ExecutionId, ToolCall, ToolCallId, ToolName, ToolResultStatus};
use echidna::ports::tool::{ToolExecutor, ToolExecutorError};
use serde_json::{Value, json};
use support::TemporaryDirectory;

fn initialized_project(label: &str) -> (TemporaryDirectory, Project) {
    let directory = TemporaryDirectory::new(label);
    let project = Project::initialize(directory.path()).unwrap();
    (directory, project)
}

fn agent_memory_scope(agent_id: &str) -> MemoryScope {
    MemoryScope::Agent {
        agent_id: AgentId::new(agent_id).unwrap(),
    }
}

fn agent_tools(
    project: &Project,
    session_id: &str,
    memory_scope: Option<MemoryScope>,
) -> ToolRegistry {
    ToolRegistry::for_agent(
        project.root(),
        StreamId::new(session_id),
        memory_scope,
        false,
    )
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
            ToolCall::new(ToolCallId::new(call_id), ToolName::new(name), arguments),
        )
        .unwrap();
    tools.execute(plan).await
}

fn memory_write_args(event_id: &str, content: &str, expected_hash: Value) -> Value {
    json!({
        "path": "core.md",
        "content": content,
        "expected_hash": expected_hash,
        "event_id": event_id
    })
}

fn research_envelope(
    event_id: &str,
    sequence: u64,
    occurred_at: &str,
    event_type: &str,
    payload: Value,
) -> EventEnvelope<Value> {
    EventEnvelope::current(
        EventId::new(event_id),
        StreamId::new("research"),
        EventSequence::new(sequence),
        OccurredAt::new(occurred_at),
        EventType::new(event_type),
        payload,
    )
}

fn target_snapshot_envelope(id: &str, event_id: &str, sequence: u64) -> EventEnvelope<Value> {
    let event = ResearchEvent::TargetSnapshotRecorded(
        TargetSnapshotRecord::from_draft(TargetSnapshotDraft {
            id: TargetSnapshotId::new(id),
            target_id: "fixture.target".to_owned(),
            kind: TargetKind::Source,
            locator: "src/fixture".to_owned(),
            version: "1.0.0".to_owned(),
            build_id: "build-1".to_owned(),
            target_artifact_ids: Vec::new(),
            environment_fingerprint: "environment-1".to_owned(),
            device_fingerprint: "unknown".to_owned(),
            captured_at: "2026-10-12T00:00:00Z".to_owned(),
            notes: "fixture target".to_owned(),
        })
        .unwrap(),
    );
    research_envelope(
        event_id,
        sequence,
        "2026-10-12T00:00:00Z",
        event.event_type().as_str(),
        event.encode_payload().unwrap(),
    )
}

fn target_artifact_legacy_payload(event_id: &str, source_path: &str) -> Value {
    json!({
        "research_schema_version": 2,
        "data": {
            "artifact_schema_version": 1,
            "record": {
                "id": "legacy-target-artifact",
                "blob_id": "0000000000000000000000000000000000000000000000000000000000000000",
                "media_type": "application/octet-stream",
                "size_bytes": 0,
                "source_path": source_path,
                "created_by_event_id": event_id,
                "produced_by_tool_call_id": "legacy-call",
                "target_snapshot_id": "target-snapshot",
                "toolchain_fingerprint": "fixture-toolchain",
                "parent_artifact_ids": [],
                "transform": null
            }
        }
    })
}

fn project_memory_legacy_payload(new_hash: &str) -> Value {
    json!({
        "research_schema_version": 2,
        "data": {
            "memory_schema_version": 1,
            "data": {
                "kind": "changed",
                "revision": {
                    "path": "core.md",
                    "old_hash": null,
                    "new_hash": new_hash,
                    "old_blob_id": null,
                    "new_blob_id": new_hash,
                    "diff_blob_id": "2222222222222222222222222222222222222222222222222222222222222222"
                }
            }
        }
    })
}

#[tokio::test]
async fn generic_registry_binds_artifacts_to_its_session_and_rejects_target_attribution() {
    let (_directory, project) = initialized_project("generic-session-artifacts");
    let tools = agent_tools(
        &project,
        "generic-session",
        Some(agent_memory_scope("writer")),
    );

    let catalog = tools.catalog();
    assert!(
        catalog
            .iter()
            .all(|spec| !matches!(spec.name.as_str(), "research_get" | "research_update"))
    );
    for name in ["artifact_store", "view_image", "http_request"] {
        let spec = catalog
            .iter()
            .find(|spec| spec.name.as_str() == name)
            .unwrap();
        let schema = &spec.input_schema;
        assert_eq!(spec.version, if name == "http_request" { "3" } else { "2" });
        assert!(schema["properties"]["execution"]["properties"]["rerun_reason"].is_object());
        if name == "artifact_store" {
            assert!(spec.description.contains("no TargetSnapshot is required"));
            assert!(spec.description.contains("Session Artifact"));
        }
        let target_property = if name == "http_request" {
            &schema["properties"]["artifact"]["properties"]["target_snapshot_id"]
        } else {
            &schema["properties"]["target_snapshot_id"]
        };
        assert!(
            target_property.is_null(),
            "{name} exposes target attribution"
        );
    }

    let source = project.root().join("generic.txt");
    fs::write(&source, "generic artifact bytes\n").unwrap();
    let stored = execute(
        &tools,
        "store-generic-artifact",
        "artifact_store",
        json!({
            "event_id": "event-generic-artifact",
            "occurred_at": "2026-10-12T00:00:00Z",
            "artifact_id": "generic-artifact",
            "source_path": source,
            "media_type": "text/plain",
            "toolchain_fingerprint": "fixture-toolchain"
        }),
    )
    .await;
    assert_eq!(stored.result.status, ToolResultStatus::Succeeded);
    assert_eq!(
        stored.result.output["artifact"]["scope"],
        json!({"kind": "session", "session_id": "generic-session"})
    );

    let raw_event: Value = serde_json::from_str(
        fs::read_to_string(project.root().join(".cyber-agent/research.jsonl"))
            .unwrap()
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(raw_event["payload"]["data"]["artifact_schema_version"], 2);
    assert_eq!(
        raw_event["payload"]["data"]["record"]["scope"],
        json!({"kind": "session", "session_id": "generic-session"})
    );

    let rejected_artifact = tools.plan(
        ExecutionId::new("execution-rejected-artifact"),
        ToolCall::new(
            ToolCallId::new("rejected-artifact"),
            ToolName::new("artifact_store"),
            json!({
                "event_id": "event-rejected-artifact",
                "occurred_at": "2026-10-12T00:00:00Z",
                "artifact_id": "rejected-artifact",
                "source_path": source,
                "media_type": "text/plain",
                "target_snapshot_id": "forged-target",
                "toolchain_fingerprint": "fixture-toolchain"
            }),
        ),
    );
    assert!(matches!(
        rejected_artifact,
        Err(ToolExecutorError::InvalidArguments { .. })
    ));

    let rejected_view = tools.plan(
        ExecutionId::new("execution-rejected-view"),
        ToolCall::new(
            ToolCallId::new("rejected-view"),
            ToolName::new("view_image"),
            json!({
                "event_id": "event-rejected-view",
                "occurred_at": "2026-10-12T00:00:00Z",
                "artifact_id": "rejected-view",
                "source_path": source,
                "target_snapshot_id": "forged-target",
                "toolchain_fingerprint": "fixture-toolchain"
            }),
        ),
    );
    assert!(matches!(
        rejected_view,
        Err(ToolExecutorError::InvalidArguments { .. })
    ));

    let rejected_http = tools.plan(
        ExecutionId::new("execution-rejected-http"),
        ToolCall::new(
            ToolCallId::new("rejected-http"),
            ToolName::new("http_request"),
            json!({
                "method": "get",
                "url": "http://127.0.0.1:1/never-contacted",
                "artifact": {
                    "event_id": "event-rejected-http",
                    "occurred_at": "2026-10-12T00:00:00Z",
                    "artifact_id": "rejected-http",
                    "target_snapshot_id": "forged-target",
                    "toolchain_fingerprint": "fixture-toolchain"
                }
            }),
        ),
    );
    assert!(matches!(
        rejected_http,
        Err(ToolExecutorError::InvalidArguments { .. })
    ));

    let research_tools = ToolRegistry::for_project(project.root());
    let target = execute(
        &research_tools,
        "record-generic-target",
        "research_update",
        json!({
            "event_id": "event-generic-target",
            "occurred_at": "2026-10-12T00:00:00Z",
            "operation": {
                "kind": "record_target_snapshot",
                "record": {
                    "id": "generic-target",
                    "target_id": "fixture.target",
                    "kind": "source",
                    "locator": "src/fixture",
                    "version": "1.0.0",
                    "build_id": "build-1",
                    "environment_fingerprint": "environment-1",
                    "device_fingerprint": "unknown",
                    "captured_at": "2026-10-12T00:00:00Z",
                    "notes": "generic artifact target applicability fixture"
                }
            }
        }),
    )
    .await;
    assert_eq!(target.result.status, ToolResultStatus::Succeeded);

    let evidence_rejected = execute(
        &research_tools,
        "generic-artifact-evidence",
        "research_update",
        json!({
            "event_id": "event-generic-artifact-evidence",
            "occurred_at": "2026-10-12T00:00:00Z",
            "operation": {
                "kind": "record_hypothesis",
                "record": {
                    "id": "generic-artifact-hypothesis",
                    "title": "Generic session Artifact is not target evidence",
                    "target_id": "fixture.target",
                    "attack_surface": "fixture surface",
                    "vulnerability_class": "fixture class",
                    "entry_point": "fixture entry",
                    "sensitive_sink": "fixture sink",
                    "rationale": "the Artifact is scoped to a generic Session",
                    "status": "active",
                    "priority": 1,
                    "evidence_links": [{
                        "relation": "supports",
                        "evidence": {"kind": "artifact", "id": "generic-artifact"}
                    }]
                }
            }
        }),
    )
    .await;
    assert_eq!(evidence_rejected.result.status, ToolResultStatus::Failed);
    assert_eq!(
        evidence_rejected.receipt.exit_reason.as_deref(),
        Some("research_transition_rejected")
    );
}

#[tokio::test]
async fn agent_memory_is_namespaced_and_has_an_independent_revision_chain() {
    let (_directory, project) = initialized_project("agent-memory-scope");
    let project_tools = ToolRegistry::for_project(project.root());
    let writer = agent_tools(
        &project,
        "writer-session",
        Some(agent_memory_scope("writer")),
    );
    let reviewer = agent_tools(
        &project,
        "reviewer-session",
        Some(agent_memory_scope("reviewer")),
    );

    let project_write = execute(
        &project_tools,
        "project-memory",
        "memory_write",
        memory_write_args("event-project-memory", "project memory\n", Value::Null),
    )
    .await;
    assert_eq!(project_write.result.status, ToolResultStatus::Succeeded);

    let first_agent_write = execute(
        &writer,
        "writer-memory-first",
        "memory_write",
        memory_write_args("event-writer-memory-first", "writer first\n", Value::Null),
    )
    .await;
    assert_eq!(first_agent_write.result.status, ToolResultStatus::Succeeded);
    let writer_hash = first_agent_write.result.output["new_hash"].clone();

    let second_agent_write = execute(
        &writer,
        "writer-memory-second",
        "memory_write",
        memory_write_args("event-writer-memory-second", "writer second\n", writer_hash),
    )
    .await;
    assert_eq!(
        second_agent_write.result.status,
        ToolResultStatus::Succeeded
    );

    let reviewer_read = execute(
        &reviewer,
        "reviewer-memory-read",
        "memory_read",
        json!({
                "path": "core.md",
                "event_id": "event-reviewer-memory-read"
        }),
    )
    .await;
    assert_eq!(reviewer_read.result.status, ToolResultStatus::Succeeded);
    assert!(reviewer_read.result.output["content"].is_null());
    assert_eq!(
        fs::read_to_string(project.root().join(".cyber-agent/memory/core.md")).unwrap(),
        "project memory\n"
    );
    assert_eq!(
        fs::read_to_string(
            project
                .root()
                .join(".cyber-agent/memory/agents/writer/core.md"),
        )
        .unwrap(),
        "writer second\n"
    );
    assert!(
        !project
            .root()
            .join(".cyber-agent/memory/agents/reviewer/core.md")
            .exists()
    );

    let scopes = fs::read_to_string(project.root().join(".cyber-agent/research.jsonl"))
        .unwrap()
        .lines()
        .map(|line| {
            serde_json::from_str::<Value>(line).unwrap()["payload"]["data"]["data"]
                ["revision"]["scope"]
                .clone()
        })
        .collect::<Vec<_>>();
    assert!(scopes.contains(&json!({"kind": "project"})));
    assert!(scopes.contains(&json!({"kind": "agent", "agent_id": "writer"})));
}

#[cfg(unix)]
#[tokio::test]
async fn agent_memory_read_rejects_symlinked_namespace_ancestors_without_events() {
    use std::os::unix::fs::symlink;

    for (label, namespace, outside_document_path) in [
        ("memory-root", "memory", "agents/writer/core.md"),
        ("agents-root", "memory/agents", "writer/core.md"),
    ] {
        let (_directory, project) = initialized_project(&format!("agent-memory-{label}"));
        let outside = TemporaryDirectory::new(&format!("agent-memory-{label}-outside"));
        let outside_document = outside.path().join(outside_document_path);
        fs::create_dir_all(outside_document.parent().unwrap()).unwrap();
        fs::write(&outside_document, "outside agent memory\n").unwrap();

        let namespace = project.root().join(".cyber-agent").join(namespace);
        fs::create_dir_all(namespace.parent().unwrap()).unwrap();
        symlink(outside.path(), &namespace).unwrap();

        let stream = project.root().join(".cyber-agent/research.jsonl");
        let before_stream = fs::read(&stream).unwrap();
        let tools = agent_tools(
            &project,
            &format!("{label}-session"),
            Some(agent_memory_scope("writer")),
        );
        let call_id = format!("{label}-read");
        let outcome = execute(
            &tools,
            &call_id,
            "memory_read",
            json!({
                "path": "core.md",
                "event_id": format!("event-{label}-read")
            }),
        )
        .await;

        assert_eq!(outcome.result.status, ToolResultStatus::Failed, "{label}");
        assert_eq!(
            outcome.receipt.exit_reason.as_deref(),
            Some("memory_read_failed"),
            "{label}"
        );
        assert!(
            !outcome
                .result
                .output
                .to_string()
                .contains("outside agent memory"),
            "{label} returned outside content"
        );
        assert_eq!(fs::read(&stream).unwrap(), before_stream, "{label}");
        assert_eq!(
            fs::read(&outside_document).unwrap(),
            b"outside agent memory\n",
            "{label}"
        );
    }
}

#[test]
fn no_memory_generic_registry_omits_memory_and_research_tools() {
    let (_directory, project) = initialized_project("generic-no-memory");
    let tools = agent_tools(&project, "generic-session", None);
    let catalog = tools.catalog();
    assert!(catalog.iter().all(|spec| {
        !matches!(
            spec.name.as_str(),
            "memory_read" | "memory_write" | "research_get" | "research_update"
        )
    }));
    let error = tools
        .plan(
            ExecutionId::new("execution-missing-memory"),
            ToolCall::new(
                ToolCallId::new("missing-memory"),
                ToolName::new("memory_read"),
                json!({}),
            ),
        )
        .unwrap_err();
    assert!(matches!(error, ToolExecutorError::UnknownTool { .. }));
}

#[test]
fn legacy_artifact_payload_stays_target_scoped_while_current_payload_is_v2() {
    let event = ArtifactEvent::decode(
        "artifact.created",
        json!({
            "artifact_schema_version": 1,
            "record": {
                "id": "legacy-artifact",
                "blob_id": "0000000000000000000000000000000000000000000000000000000000000000",
                "media_type": "application/octet-stream",
                "size_bytes": 0,
                "source_path": "fixture.bin",
                "created_by_event_id": "legacy-event",
                "produced_by_tool_call_id": "legacy-call",
                "target_snapshot_id": "legacy-snapshot",
                "toolchain_fingerprint": "fixture-toolchain"
            }
        }),
    )
    .unwrap();
    assert_eq!(
        event.record().scope,
        ArtifactScope::Target {
            target_snapshot_id: echidna::domain::research::TargetSnapshotId::new("legacy-snapshot"),
        }
    );
    let current = event.encode_payload().unwrap();
    assert_eq!(current["artifact_schema_version"], 2);
    assert_eq!(
        current["record"]["scope"],
        json!({"kind": "target", "target_snapshot_id": "legacy-snapshot"})
    );
}

#[test]
fn target_artifact_v1_and_v2_share_historical_event_identity() {
    let legacy_payload = target_artifact_legacy_payload("event-target-artifact", "fixture.bin");
    let current_payload = ResearchEvent::decode("artifact.created", legacy_payload.clone())
        .unwrap()
        .encode_payload()
        .unwrap();
    assert_eq!(current_payload["data"]["artifact_schema_version"], 2);

    let state = ResearchReducer
        .reduce([
            target_snapshot_envelope("target-snapshot", "event-target", 1),
            research_envelope(
                "event-target-artifact",
                2,
                "2026-10-12T00:00:00Z",
                "artifact.created",
                legacy_payload,
            ),
            research_envelope(
                "event-target-artifact",
                3,
                "2026-10-12T00:00:00Z",
                "artifact.created",
                current_payload.clone(),
            ),
        ])
        .unwrap();
    assert_eq!(state.applied_event_count(), 2);

    let mut changed_payload = current_payload;
    changed_payload["data"]["record"]["source_path"] = json!("other-fixture.bin");
    assert!(matches!(
        ResearchReducer.reduce([
            target_snapshot_envelope("target-snapshot", "event-target", 1),
            research_envelope(
                "event-target-artifact",
                2,
                "2026-10-12T00:00:00Z",
                "artifact.created",
                target_artifact_legacy_payload("event-target-artifact", "fixture.bin"),
            ),
            research_envelope(
                "event-target-artifact",
                3,
                "2026-10-12T00:00:00Z",
                "artifact.created",
                changed_payload,
            ),
        ]),
        Err(ResearchError::EventIdConflict { .. })
    ));
}

#[test]
fn project_memory_v1_and_v2_share_historical_event_identity() {
    let legacy_payload = project_memory_legacy_payload(&"1".repeat(64));
    let current_payload = ResearchEvent::decode("memory.changed", legacy_payload.clone())
        .unwrap()
        .encode_payload()
        .unwrap();
    assert_eq!(current_payload["data"]["memory_schema_version"], 2);
    assert_eq!(
        current_payload["data"]["data"]["revision"]["scope"],
        json!({"kind": "project"})
    );

    let state = ResearchReducer
        .reduce([
            research_envelope(
                "event-project-memory",
                1,
                "2026-10-12T00:00:00Z",
                "memory.changed",
                legacy_payload,
            ),
            research_envelope(
                "event-project-memory",
                2,
                "2026-10-12T00:00:00Z",
                "memory.changed",
                current_payload.clone(),
            ),
        ])
        .unwrap();
    assert_eq!(state.applied_event_count(), 1);

    let mut changed_payload = current_payload;
    changed_payload["data"]["data"]["revision"]["new_hash"] = json!("3".repeat(64));
    changed_payload["data"]["data"]["revision"]["new_blob_id"] = json!("3".repeat(64));
    assert!(matches!(
        ResearchReducer.reduce([
            research_envelope(
                "event-project-memory",
                1,
                "2026-10-12T00:00:00Z",
                "memory.changed",
                project_memory_legacy_payload(&"1".repeat(64)),
            ),
            research_envelope(
                "event-project-memory",
                2,
                "2026-10-12T00:00:00Z",
                "memory.changed",
                changed_payload,
            ),
        ]),
        Err(ResearchError::EventIdConflict { .. })
    ));
}

#[test]
fn target_scoped_derived_artifact_rejects_a_session_scoped_parent() {
    let session_artifact = ResearchEvent::Artifact(ArtifactEvent::Created(ArtifactRecord {
        id: ArtifactId::new("session-artifact"),
        blob_id: BlobId::new("4".repeat(64)).unwrap(),
        media_type: "application/octet-stream".to_owned(),
        size_bytes: 1,
        source_path: "session.bin".to_owned(),
        created_by_event_id: EventId::new("event-session-artifact"),
        produced_by_tool_call_id: ToolCallId::new("session-artifact-call"),
        scope: ArtifactScope::Session {
            session_id: StreamId::new("generic-session"),
        },
        toolchain_fingerprint: "fixture-toolchain".to_owned(),
        parent_artifact_ids: Vec::new(),
        transform: None,
    }));
    let target_derived = ResearchEvent::Artifact(ArtifactEvent::Derived(ArtifactRecord {
        id: ArtifactId::new("target-derived-artifact"),
        blob_id: BlobId::new("5".repeat(64)).unwrap(),
        media_type: "application/octet-stream".to_owned(),
        size_bytes: 1,
        source_path: "target.bin".to_owned(),
        created_by_event_id: EventId::new("event-target-derived-artifact"),
        produced_by_tool_call_id: ToolCallId::new("target-derived-artifact-call"),
        scope: ArtifactScope::Target {
            target_snapshot_id: TargetSnapshotId::new("target-snapshot"),
        },
        toolchain_fingerprint: "fixture-toolchain".to_owned(),
        parent_artifact_ids: vec![ArtifactId::new("session-artifact")],
        transform: Some("fixture transform".to_owned()),
    }));

    assert!(matches!(
        ResearchReducer.reduce([
            target_snapshot_envelope("target-snapshot", "event-target", 1),
            research_envelope(
                "event-session-artifact",
                2,
                "2026-10-12T00:00:00Z",
                session_artifact.event_type().as_str(),
                session_artifact.encode_payload().unwrap(),
            ),
            research_envelope(
                "event-target-derived-artifact",
                3,
                "2026-10-12T00:00:00Z",
                target_derived.event_type().as_str(),
                target_derived.encode_payload().unwrap(),
            ),
        ]),
        Err(ResearchError::InvalidRecord { .. })
    ));
}
