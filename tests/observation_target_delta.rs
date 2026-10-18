mod support;

use std::process::Command;

use echidna::adapters::project_event_store::{Project, ProjectEventStream};
use echidna::adapters::tools::ToolRegistry;
use echidna::domain::context::{ProviderSessionId, ProviderSessionRef, ProviderSessionRole};
use echidna::domain::events::{EventEnvelope, EventId, EventType, OccurredAt, StreamId};
use echidna::domain::model::ContentItem;
use echidna::domain::research::{
    ResearchEvent, TargetKind, TargetSnapshotDraft, TargetSnapshotId, TargetSnapshotRecord,
};
use echidna::domain::session::SessionEvent;
use echidna::domain::tools::{
    ExecutionId, ToolCall, ToolCallId, ToolExecutionOutcome, ToolName, ToolResultStatus,
};
use echidna::ports::tool::ToolExecutor;
use echidna::ports::{EventSink, EventSource};
use serde_json::{Value, json};

use support::TemporaryDirectory;

const EXTRACTOR_ID: &str = "aapt2-manifest-json";
const EXTRACTOR_VERSION: &str = "2.19.0";

struct Source<'a> {
    session_id: &'a str,
    artifact_id: &'a str,
    snapshot_id: &'a str,
    call_id: &'a str,
    receipt_event_id: &'a str,
    extractor_version: &'a str,
}

fn execute_cli(directory: &TemporaryDirectory, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_echidna"))
        .current_dir(directory.path())
        .args(args)
        .output()
        .expect("CLI inspection should run")
}

fn batch(source: &Source<'_>, components: Vec<Value>) -> Value {
    json!({
        "schema_version": 1,
        "observations": components.into_iter().map(|mut component| {
            let fields = component.as_object_mut().unwrap();
            fields.insert("schema_version".to_owned(), json!(1));
            fields.insert("target_snapshot_id".to_owned(), json!(source.snapshot_id));
            fields.insert("raw_artifact_id".to_owned(), json!(source.artifact_id));
            fields.insert("produced_by_tool_call_id".to_owned(), json!(source.call_id));
            fields.insert("execution_receipt_event_id".to_owned(), json!(source.receipt_event_id));
            fields.insert("location".to_owned(), json!("AndroidManifest.xml"));
            fields.insert("source".to_owned(), json!("android_manifest"));
            fields.insert("sink".to_owned(), json!("attack_surface"));
            fields.insert("prerequisites".to_owned(), json!(["manifest_available"]));
            fields.insert("confidence".to_owned(), json!("high"));
            fields.insert("evidence_refs".to_owned(), json!([source.artifact_id]));
            fields.insert("extractor_id".to_owned(), json!(EXTRACTOR_ID));
            fields.insert("extractor_version".to_owned(), json!(source.extractor_version));
            component
        }).collect::<Vec<_>>(),
    })
}

fn component(
    observation_id: &str,
    subject: &str,
    exported: bool,
    permission: Option<&str>,
    intent_filters: Value,
) -> Value {
    json!({
        "observation_id": observation_id,
        "kind": "android_manifest_component",
        "subject": subject,
        "entry_point": "activity",
        "details": {
            "component_type": "activity",
            "exported": exported,
            "permission": permission,
            "intent_filters": intent_filters,
        }
    })
}

fn record_snapshot(project: &Project, id: &str, version: &str) {
    let record = TargetSnapshotRecord::from_draft(TargetSnapshotDraft {
        id: TargetSnapshotId::new(id),
        target_id: "com.example.fixture".to_owned(),
        kind: TargetKind::Mobile,
        locator: "fixture.apk".to_owned(),
        version: version.to_owned(),
        build_id: format!("build-{version}"),
        target_artifact_ids: Vec::new(),
        environment_fingerprint: "fixture-environment".to_owned(),
        device_fingerprint: "fixture-device".to_owned(),
        captured_at: "2026-08-30T00:00:00Z".to_owned(),
        notes: String::new(),
    })
    .unwrap();
    let event = ResearchEvent::TargetSnapshotRecorded(record);
    let mut writer = project.open_writer().unwrap();
    let mut store = writer.open_research(None).unwrap();
    let envelope = EventEnvelope::current(
        EventId::new(format!("event-{id}")),
        ProjectEventStream::Research.stream_id(),
        store.next_sequence().unwrap(),
        OccurredAt::new("2026-08-30T00:00:00Z"),
        EventType::new(event.event_type().as_str()),
        event.encode_payload().unwrap(),
    );
    EventSink::append_event(&mut store, &envelope).unwrap();
}

async fn record_external_batch(project: &Project, source: &Source<'_>, batch: &Value) {
    let path = project.root().join(format!("{}.json", source.artifact_id));
    let batch_bytes = serde_json::to_vec(batch).unwrap();
    std::fs::write(&path, &batch_bytes).unwrap();
    let source_file = path.file_name().unwrap().to_string_lossy().into_owned();
    let tools = ToolRegistry::for_project(project.root());
    let call = ToolCall::new(
        ToolCallId::new(source.call_id),
        ToolName::new("run_commands"),
        json!({
            "mode": "serial",
            "commands": [{
                "argv": ["cat", source_file]
            }]
        }),
    );
    let outcome = tools
        .execute(
            tools
                .plan(
                    ExecutionId::new(format!("execution:{}", source.call_id)),
                    call.clone(),
                )
                .unwrap(),
        )
        .await
        .expect("builtin Tool completes");
    assert_eq!(outcome.result.status, ToolResultStatus::Succeeded);
    assert_eq!(
        outcome.result.output["commands"][0]["result"]["stdout"],
        String::from_utf8(batch_bytes).unwrap(),
        "the external fixture output must be the exact raw Observation batch"
    );
    record_source_session(project, source, call, outcome);

    let artifact = tools
        .execute(
            tools
                .plan(
                    ExecutionId::new(format!("artifact-execution-{}", source.artifact_id)),
                    ToolCall::new(
                        ToolCallId::new(format!("artifact-call-{}", source.artifact_id)),
                        ToolName::new("artifact_store"),
                        json!({
                            "event_id": format!("artifact-event-{}", source.artifact_id),
                            "occurred_at": "2026-08-30T00:00:01Z",
                            "artifact_id": source.artifact_id,
                            "source_path": source_file,
                            "media_type": "application/json",
                            "target_snapshot_id": source.snapshot_id,
                            "toolchain_fingerprint": format!("{EXTRACTOR_ID}@{}", source.extractor_version),
                        }),
                    ),
                )
                .unwrap(),
        )
        .await
        .expect("builtin Tool completes");
    assert_eq!(artifact.result.status, ToolResultStatus::Succeeded);
}

fn record_source_session(
    project: &Project,
    source: &Source<'_>,
    call: ToolCall,
    outcome: ToolExecutionOutcome,
) {
    let session_id = StreamId::new(source.session_id);
    let stream_id = ProjectEventStream::Session(session_id.clone()).stream_id();
    let mut writer = project.open_writer().unwrap();
    let mut store = writer.open_session(session_id, None).unwrap();
    let events = [
        (
            EventId::new("session-start"),
            SessionEvent::Started {
                primary_provider_session: ProviderSessionRef {
                    id: ProviderSessionId::new(format!("primary:{}", source.session_id)),
                    role: ProviderSessionRole::Primary,
                    generation: 1,
                },
            },
        ),
        (
            EventId::new("source-call"),
            SessionEvent::HistoryItemRecorded {
                item: ContentItem::ToolCall { call },
            },
        ),
        (
            EventId::new(source.receipt_event_id),
            SessionEvent::ExecutionReceiptRecorded {
                receipt: outcome.receipt,
            },
        ),
        (
            EventId::new("source-result"),
            SessionEvent::HistoryItemRecorded {
                item: ContentItem::ToolResult {
                    result: outcome.result,
                },
            },
        ),
    ];
    for (event_id, event) in events {
        let envelope = event
            .into_envelope(
                event_id,
                stream_id.clone(),
                store.next_sequence().unwrap(),
                OccurredAt::new("2026-08-30T00:00:00Z"),
            )
            .unwrap();
        EventSink::append_event(&mut store, &envelope).unwrap();
    }
    let events = EventSource::read_events(&store, &stream_id).unwrap();
    assert_eq!(events.len(), 4);
}

fn rewrite_jsonl(path: &std::path::Path, mut rewrite: impl FnMut(&mut Value)) {
    let source = std::fs::read_to_string(path).unwrap();
    let mut output = String::new();
    for line in source.lines() {
        let mut event: Value = serde_json::from_str(line).unwrap();
        rewrite(&mut event);
        output.push_str(&serde_json::to_string(&event).unwrap());
        output.push('\n');
    }
    std::fs::write(path, output).unwrap();
}

fn artifact_blob_id(project: &Project, artifact_id: &str) -> String {
    let source =
        std::fs::read_to_string(project.root().join(".cyber-agent/research.jsonl")).unwrap();
    source
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .find_map(|event| {
            let record = &event["payload"]["data"]["record"];
            (record["id"] == artifact_id).then(|| record["blob_id"].as_str().unwrap().to_owned())
        })
        .unwrap()
}

fn session_events_path(project: &Project, session_id: &str) -> std::path::PathBuf {
    project
        .root()
        .join(".cyber-agent/sessions")
        .join(session_id)
        .join("events.jsonl")
}

#[tokio::test]
async fn external_android_manifest_batches_use_run_commands_and_produce_stable_provenance_delta() {
    let directory = TemporaryDirectory::new("m4-s3d-observation-delta");
    let project = Project::initialize(directory.path()).unwrap();
    record_snapshot(&project, "snapshot-before", "1.0.0");
    record_snapshot(&project, "snapshot-after", "2.0.0");
    let before = Source {
        session_id: "session-before",
        artifact_id: "artifact-before",
        snapshot_id: "snapshot-before",
        call_id: "extract-before",
        receipt_event_id: "receipt-extract-before",
        extractor_version: EXTRACTOR_VERSION,
    };
    let after = Source {
        session_id: "session-after",
        artifact_id: "artifact-after",
        snapshot_id: "snapshot-after",
        call_id: "extract-after",
        receipt_event_id: "receipt-extract-after",
        extractor_version: EXTRACTOR_VERSION,
    };
    let before_batch = batch(
        &before,
        vec![
            component(
                "observation-old-before",
                "com.example.OldActivity",
                true,
                Some("com.example.permission.OLD"),
                json!([]),
            ),
            component(
                "observation-main-before",
                "com.example.MainActivity",
                false,
                None,
                json!([
                    {"actions": ["android.intent.action.VIEW"], "categories": [], "data_schemes": ["https"]},
                    {"actions": ["com.example.OLD"], "categories": [], "data_schemes": []}
                ]),
            ),
        ],
    );
    let after_batch = batch(
        &after,
        vec![
            component(
                "observation-new-after",
                "com.example.NewActivity",
                true,
                None,
                json!([{"actions": ["android.intent.action.VIEW"], "categories": ["android.intent.category.BROWSABLE"], "data_schemes": ["example"]}]),
            ),
            component(
                "observation-main-after",
                "com.example.MainActivity",
                true,
                Some("com.example.permission.PRIVATE"),
                json!([
                    {"actions": ["android.intent.action.VIEW"], "categories": [], "data_schemes": ["https"]},
                    {"actions": ["android.intent.action.SEND"], "categories": ["android.intent.category.DEFAULT"], "data_schemes": []}
                ]),
            ),
        ],
    );
    record_external_batch(&project, &before, &before_batch).await;
    record_external_batch(&project, &after, &after_batch).await;
    let research_before =
        std::fs::read(project.root().join(".cyber-agent/research.jsonl")).unwrap();

    let delta = execute_cli(
        &directory,
        &[
            "inspect",
            "target-delta",
            "--baseline-session",
            before.session_id,
            "--baseline-artifact",
            before.artifact_id,
            "--candidate-session",
            after.session_id,
            "--candidate-artifact",
            after.artifact_id,
        ],
    );
    assert!(delta.status.success(), "stderr: {:?}", delta.stderr);
    let delta: Value = serde_json::from_slice(&delta.stdout).unwrap();
    let changes = delta["delta"]["changes"].as_array().unwrap();
    assert_eq!(
        changes
            .iter()
            .map(|change| change["kind"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec![
            "exported_changed",
            "permission_changed",
            "intent_filter_added",
            "intent_filter_removed",
            "component_added",
            "component_removed"
        ]
    );
    assert_eq!(
        changes[0]["before_provenance"]["execution_receipt_event_id"],
        before.receipt_event_id
    );
    assert_eq!(
        changes[0]["after_provenance"]["raw_artifact_id"],
        after.artifact_id
    );
    assert_eq!(
        changes[2]["before_provenance"]["raw_artifact_id"], before.artifact_id,
        "an intent-filter addition must retain the source that proves prior absence"
    );
    assert_eq!(
        changes[4]["after_manifest_component"]["intent_filters"][0]["data_schemes"],
        json!(["example"]),
        "an added component must carry its complete typed manifest details"
    );
    assert_eq!(
        changes[5]["before_manifest_component"]["permission"], "com.example.permission.OLD",
        "a removed component must carry its prior typed manifest details"
    );
    assert_eq!(
        delta["delta"]["baseline_source"]["raw_artifact_id"],
        before.artifact_id
    );
    assert_eq!(
        delta["delta"]["candidate_source"]["execution_receipt_event_id"],
        after.receipt_event_id
    );
    assert_eq!(
        std::fs::read(project.root().join(".cyber-agent/research.jsonl")).unwrap(),
        research_before,
        "the read-only Observation/Delta path must not promote a Finding or write Research"
    );
    assert!(
        !project.root().join(".cyber-agent/memory").exists(),
        "the read-only Observation/Delta path must not write Memory"
    );
}

#[tokio::test]
async fn strict_observation_batches_reject_unknown_missing_and_forged_references() {
    let directory = TemporaryDirectory::new("m4-s3d-observation-rejections");
    let project = Project::initialize(directory.path()).unwrap();
    record_snapshot(&project, "snapshot-one", "1.0.0");
    record_snapshot(&project, "snapshot-other", "2.0.0");
    let cross_target = Source {
        session_id: "session-cross-target",
        artifact_id: "artifact-cross-target",
        snapshot_id: "snapshot-other",
        call_id: "extract-cross-target",
        receipt_event_id: "receipt-cross-target",
        extractor_version: EXTRACTOR_VERSION,
    };
    let cross_target_batch = batch(
        &cross_target,
        vec![component(
            "observation-cross-target",
            "com.example.CrossTargetActivity",
            true,
            None,
            json!([]),
        )],
    );
    record_external_batch(&project, &cross_target, &cross_target_batch).await;

    for (label, expected) in [
        ("unknown", "unknown field"),
        ("missing", "missing field"),
        ("batch-schema", "batch schema version"),
        ("envelope-schema", "ObservationEnvelope schema version"),
        ("kind", "unknown variant"),
        ("duplicate-id", "duplicates observation ID"),
        ("duplicate-subject", "duplicates Android manifest component"),
        ("batch-source", "one TargetSnapshot, raw Artifact, ToolCall"),
        ("duplicate-filter", "duplicate intent filter"),
        ("extractor-format", "extractor_version must use only ASCII"),
        ("snapshot", "binds TargetSnapshot"),
        ("evidence-snapshot", "evidence Artifact"),
        ("receipt", "Receipt Event"),
        ("artifact", "declares raw Artifact"),
    ] {
        let artifact_id = format!("artifact-{label}");
        let session_id = format!("session-{label}");
        let call_id = format!("extract-{label}");
        let receipt_id = format!("receipt-{call_id}");
        let source = Source {
            session_id: &session_id,
            artifact_id: &artifact_id,
            snapshot_id: "snapshot-one",
            call_id: &call_id,
            receipt_event_id: &receipt_id,
            extractor_version: EXTRACTOR_VERSION,
        };
        let mut invalid = batch(
            &source,
            vec![component(
                "observation-one",
                "com.example.MainActivity",
                true,
                None,
                json!([]),
            )],
        );
        match label {
            "unknown" => invalid["observations"][0]["unknown"] = json!(true),
            "missing" => {
                invalid["observations"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("subject");
            }
            "batch-schema" => invalid["schema_version"] = json!(2),
            "envelope-schema" => invalid["observations"][0]["schema_version"] = json!(2),
            "kind" => invalid["observations"][0]["kind"] = json!("unknown_manifest_kind"),
            "duplicate-id" => {
                let mut duplicate = invalid["observations"][0].clone();
                duplicate["subject"] = json!("com.example.OtherActivity");
                invalid["observations"]
                    .as_array_mut()
                    .unwrap()
                    .push(duplicate);
            }
            "duplicate-subject" => {
                let mut duplicate = invalid["observations"][0].clone();
                duplicate["observation_id"] = json!("observation-two");
                invalid["observations"]
                    .as_array_mut()
                    .unwrap()
                    .push(duplicate);
            }
            "batch-source" => {
                let mut duplicate = invalid["observations"][0].clone();
                duplicate["observation_id"] = json!("observation-two");
                duplicate["subject"] = json!("com.example.OtherActivity");
                duplicate["produced_by_tool_call_id"] = json!("another-call");
                invalid["observations"]
                    .as_array_mut()
                    .unwrap()
                    .push(duplicate);
            }
            "duplicate-filter" => {
                let filter = json!({
                    "actions": ["android.intent.action.VIEW"],
                    "categories": [],
                    "data_schemes": ["https"]
                });
                invalid["observations"][0]["details"]["intent_filters"] =
                    json!([filter.clone(), filter]);
            }
            "extractor-format" => {
                invalid["observations"][0]["extractor_version"] = json!("2.19.0@forged")
            }
            "snapshot" => invalid["observations"][0]["target_snapshot_id"] = json!("forged"),
            "evidence-snapshot" => {
                invalid["observations"][0]["evidence_refs"] =
                    json!([source.artifact_id, cross_target.artifact_id])
            }
            "receipt" => invalid["observations"][0]["execution_receipt_event_id"] = json!("forged"),
            "artifact" => {
                invalid["observations"][0]["raw_artifact_id"] = json!("forged");
                invalid["observations"][0]["evidence_refs"] = json!(["forged"]);
            }
            _ => unreachable!(),
        }
        record_external_batch(&project, &source, &invalid).await;
        let output = execute_cli(
            &directory,
            &[
                "inspect",
                "android-manifest",
                "--session",
                source.session_id,
                "--artifact",
                source.artifact_id,
            ],
        );
        assert!(!output.status.success(), "{label} unexpectedly passed");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(expected),
            "{label} stderr: {:?}",
            output.stderr
        );
    }
}

#[tokio::test]
async fn observation_ingestion_revalidates_artifact_size_hash_and_extractor_toolchain() {
    let directory = TemporaryDirectory::new("m4-s3d-observation-artifact-integrity");
    let project = Project::initialize(directory.path()).unwrap();
    record_snapshot(&project, "snapshot-integrity", "1.0.0");

    let size_source = Source {
        session_id: "session-size",
        artifact_id: "artifact-size",
        snapshot_id: "snapshot-integrity",
        call_id: "extract-size",
        receipt_event_id: "receipt-extract-size",
        extractor_version: EXTRACTOR_VERSION,
    };
    let size_batch = batch(
        &size_source,
        vec![component(
            "observation-size",
            "com.example.SizeActivity",
            true,
            None,
            json!([]),
        )],
    );
    record_external_batch(&project, &size_source, &size_batch).await;
    rewrite_jsonl(
        &project.root().join(".cyber-agent/research.jsonl"),
        |event| {
            if event["event_type"] == "artifact.created" {
                let record = &mut event["payload"]["data"]["record"];
                if record["id"] == size_source.artifact_id {
                    let declared = record["size_bytes"].as_u64().unwrap();
                    record["size_bytes"] = json!(declared + 1);
                }
            }
        },
    );
    let size = execute_cli(
        &directory,
        &[
            "inspect",
            "android-manifest",
            "--session",
            size_source.session_id,
            "--artifact",
            size_source.artifact_id,
        ],
    );
    assert!(!size.status.success());
    assert!(
        String::from_utf8_lossy(&size.stderr).contains("verified Blob contains"),
        "size stderr: {:?}",
        size.stderr
    );

    let toolchain_source = Source {
        session_id: "session-toolchain",
        artifact_id: "artifact-toolchain",
        snapshot_id: "snapshot-integrity",
        call_id: "extract-toolchain",
        receipt_event_id: "receipt-extract-toolchain",
        extractor_version: EXTRACTOR_VERSION,
    };
    let mut toolchain_batch = batch(
        &toolchain_source,
        vec![component(
            "observation-toolchain",
            "com.example.ToolchainActivity",
            true,
            None,
            json!([]),
        )],
    );
    toolchain_batch["observations"][0]["extractor_version"] = json!("2.20.0");
    record_external_batch(&project, &toolchain_source, &toolchain_batch).await;
    let toolchain = execute_cli(
        &directory,
        &[
            "inspect",
            "android-manifest",
            "--session",
            toolchain_source.session_id,
            "--artifact",
            toolchain_source.artifact_id,
        ],
    );
    assert!(!toolchain.status.success());
    assert!(String::from_utf8_lossy(&toolchain.stderr).contains("does not match Artifact"));

    let hash_source = Source {
        session_id: "session-hash",
        artifact_id: "artifact-hash",
        snapshot_id: "snapshot-integrity",
        call_id: "extract-hash",
        receipt_event_id: "receipt-extract-hash",
        extractor_version: EXTRACTOR_VERSION,
    };
    let hash_batch = batch(
        &hash_source,
        vec![component(
            "observation-hash",
            "com.example.HashActivity",
            true,
            None,
            json!([]),
        )],
    );
    record_external_batch(&project, &hash_source, &hash_batch).await;
    let blob_id = artifact_blob_id(&project, hash_source.artifact_id);
    std::fs::write(
        project
            .root()
            .join(".cyber-agent/artifacts/blobs/sha256")
            .join(blob_id),
        b"{}",
    )
    .unwrap();
    let hash = execute_cli(
        &directory,
        &[
            "inspect",
            "android-manifest",
            "--session",
            hash_source.session_id,
            "--artifact",
            hash_source.artifact_id,
        ],
    );
    assert!(!hash.status.success());
    assert!(String::from_utf8_lossy(&hash.stderr).contains("Blob could not be read"));
}

#[tokio::test]
async fn observation_ingestion_rejects_wrong_extractor_tool_and_receipt_facts() {
    let directory = TemporaryDirectory::new("m4-s3d-observation-session-provenance");
    let project = Project::initialize(directory.path()).unwrap();
    record_snapshot(&project, "snapshot-provenance", "1.0.0");

    for (label, expected) in [
        ("tool", "must be `run_commands`"),
        ("execution", "expected `execution:"),
        ("status", "is not successful"),
    ] {
        let session_id = format!("session-{label}");
        let artifact_id = format!("artifact-{label}");
        let call_id = format!("extract-{label}");
        let receipt_event_id = format!("receipt-{label}");
        let source = Source {
            session_id: &session_id,
            artifact_id: &artifact_id,
            snapshot_id: "snapshot-provenance",
            call_id: &call_id,
            receipt_event_id: &receipt_event_id,
            extractor_version: EXTRACTOR_VERSION,
        };
        let source_batch = batch(
            &source,
            vec![component(
                &format!("observation-{label}"),
                &format!("com.example.{label}Activity"),
                true,
                None,
                json!([]),
            )],
        );
        record_external_batch(&project, &source, &source_batch).await;
        rewrite_jsonl(&session_events_path(&project, &session_id), |event| match (
            label,
            event["event_id"].as_str(),
        ) {
            ("tool", Some("source-call")) => {
                event["payload"]["item"]["call"]["name"] = json!("run_command")
            }
            ("tool", Some("source-result")) => {
                event["payload"]["item"]["result"]["tool_name"] = json!("run_command")
            }
            ("execution", Some(event_id)) if event_id == receipt_event_id => {
                event["payload"]["receipt"]["execution_id"] = json!("execution:forged")
            }
            ("status", Some(event_id)) if event_id == receipt_event_id => {
                event["payload"]["receipt"]["status"] = json!("failed")
            }
            _ => {}
        });
        let output = execute_cli(
            &directory,
            &[
                "inspect",
                "android-manifest",
                "--session",
                source.session_id,
                "--artifact",
                source.artifact_id,
            ],
        );
        assert!(!output.status.success(), "{label} unexpectedly passed");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(expected),
            "{label} stderr: {:?}",
            output.stderr
        );
    }
}

#[tokio::test]
async fn target_delta_rejects_extractor_version_changes() {
    let directory = TemporaryDirectory::new("m4-s3d-observation-extractor-version");
    let project = Project::initialize(directory.path()).unwrap();
    record_snapshot(&project, "snapshot-old", "1.0.0");
    record_snapshot(&project, "snapshot-new", "2.0.0");
    let old = Source {
        session_id: "session-old",
        artifact_id: "artifact-old",
        snapshot_id: "snapshot-old",
        call_id: "extract-old",
        receipt_event_id: "receipt-extract-old",
        extractor_version: "1.0.0",
    };
    let new = Source {
        session_id: "session-new",
        artifact_id: "artifact-new",
        snapshot_id: "snapshot-new",
        call_id: "extract-new",
        receipt_event_id: "receipt-extract-new",
        extractor_version: "2.0.0",
    };
    record_external_batch(
        &project,
        &old,
        &batch(
            &old,
            vec![component(
                "old",
                "com.example.MainActivity",
                false,
                None,
                json!([]),
            )],
        ),
    )
    .await;
    record_external_batch(
        &project,
        &new,
        &batch(
            &new,
            vec![component(
                "new",
                "com.example.MainActivity",
                false,
                None,
                json!([]),
            )],
        ),
    )
    .await;
    let output = execute_cli(
        &directory,
        &[
            "inspect",
            "target-delta",
            "--baseline-session",
            old.session_id,
            "--baseline-artifact",
            old.artifact_id,
            "--candidate-session",
            new.session_id,
            "--candidate-artifact",
            new.artifact_id,
        ],
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Extractor identity/version"));
}
