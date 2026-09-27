mod support;

use std::fs;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use echidna::adapters::project_event_store::{
    Project, ProjectEventStoreError, ProjectEventStream, RecoveryMetadata,
};
use echidna::domain::events::{
    EventEnvelope, EventId, EventSchema, EventSequence, EventType, OccurredAt, StreamId,
};
use echidna::ports::{EventSink, EventSource, EventStore};
use serde_json::{Value, json};
use support::TemporaryDirectory;

fn research_path(project: &Project) -> std::path::PathBuf {
    project.root().join(".cyber-agent/research.jsonl")
}

fn migration_manifest_path(destination: &std::path::Path) -> std::path::PathBuf {
    let filename = destination.file_name().unwrap().to_str().unwrap();
    destination.with_file_name(format!("{filename}.migration.json"))
}

fn event(
    event_id: &str,
    stream_id: &StreamId,
    sequence: u64,
    payload: Value,
) -> EventEnvelope<Value> {
    EventEnvelope::current(
        EventId::new(event_id),
        stream_id.clone(),
        EventSequence::new(sequence),
        OccurredAt::new("2026-08-22T00:00:00Z"),
        EventType::new("test.event"),
        payload,
    )
}

fn assert_event_store<Store: EventStore<Value>>(_store: &Store) {}

fn error<T>(result: Result<T, ProjectEventStoreError>) -> ProjectEventStoreError {
    match result {
        Ok(_) => panic!("operation should fail"),
        Err(error) => error,
    }
}

fn assert_empty_recovery_directory(project: &Project) {
    assert_eq!(
        fs::read_dir(project.root().join(".cyber-agent/artifacts/recovery"))
            .unwrap()
            .count(),
        0
    );
}

fn assert_no_durable_temporary_files(directory: &std::path::Path) {
    for entry in fs::read_dir(directory).unwrap() {
        let filename = entry.unwrap().file_name();
        assert!(
            !filename
                .to_string_lossy()
                .starts_with(".cyber-agent-durable-"),
            "durable temporary file remained in {}",
            directory.display()
        );
    }
}

fn recovery_metadata() -> RecoveryMetadata {
    RecoveryMetadata::new(
        EventId::new("recovery-1"),
        OccurredAt::new("2026-08-22T00:01:00Z"),
    )
}

#[test]
fn initialized_projects_discover_upward_and_keep_research_isolated() {
    let first_root = TemporaryDirectory::new("project-a");
    let second_root = TemporaryDirectory::new("project-b");
    let first = Project::initialize(first_root.path()).unwrap();
    let second = Project::initialize(second_root.path()).unwrap();

    let nested = first.root().join("nested/inside");
    fs::create_dir_all(&nested).unwrap();
    let discovered = Project::discover(&nested).unwrap();
    assert_eq!(discovered.root(), first.root());
    assert_eq!(discovered.project_id(), first.project_id());
    assert_ne!(first.project_id(), second.project_id());

    for (project, value) in [(&first, "first"), (&second, "second")] {
        let mut writer = project.open_writer().unwrap();
        let mut store = writer.open_research(None).unwrap();
        let stream = ProjectEventStream::Research.stream_id();
        EventSink::append_event(
            &mut store,
            &event(value, &stream, 1, json!({"project": value})),
        )
        .unwrap();
    }

    assert!(
        String::from_utf8(fs::read(research_path(&first)).unwrap())
            .unwrap()
            .contains("first")
    );
    assert!(
        !String::from_utf8(fs::read(research_path(&first)).unwrap())
            .unwrap()
            .contains("second")
    );
    assert!(
        String::from_utf8(fs::read(research_path(&second)).unwrap())
            .unwrap()
            .contains("second")
    );
}

#[test]
fn project_sentinel_is_parseable_and_leaves_no_durable_temporary_file() {
    let root = TemporaryDirectory::new("project-sentinel");
    let project = Project::initialize(root.path()).unwrap();
    let control_directory = project.root().join(".cyber-agent");
    let configuration: toml::Value = fs::read_to_string(control_directory.join("project.toml"))
        .unwrap()
        .parse()
        .unwrap();

    assert_eq!(
        configuration["project_id"].as_str(),
        Some(project.project_id().as_str())
    );
    assert_eq!(
        configuration["canonical_root"].as_str(),
        Some(project.root().to_str().unwrap())
    );
    assert_no_durable_temporary_files(&control_directory);
}

#[test]
fn discovery_rejects_a_project_id_that_does_not_match_the_canonical_root() {
    let root = TemporaryDirectory::new("project-id-mismatch");
    let project = Project::initialize(root.path()).unwrap();
    let project_file = project.root().join(".cyber-agent/project.toml");
    let mut configuration: toml::Value =
        toml::from_str(&fs::read_to_string(&project_file).unwrap()).unwrap();
    configuration["project_id"] = toml::Value::String("project-tampered".to_owned());
    let tampered = toml::to_string_pretty(&configuration).unwrap();
    fs::write(&project_file, &tampered).unwrap();

    assert!(matches!(
        error(Project::discover(project.root())),
        ProjectEventStoreError::ProjectIdentityMismatch { configured, expected, .. }
            if configured.as_str() == "project-tampered" && expected == *project.project_id()
    ));
    assert_eq!(fs::read_to_string(&project_file).unwrap(), tampered);
}

#[test]
fn repeat_init_does_not_truncate_existing_project_facts() {
    let root = TemporaryDirectory::new("repeat-init");
    let project = Project::initialize(root.path()).unwrap();
    let research = research_path(&project);
    fs::write(&research, b"existing-fact\n").unwrap();

    assert!(matches!(
        error(Project::initialize(root.path())),
        ProjectEventStoreError::AlreadyInitialized { .. }
    ));
    assert_eq!(fs::read(&research).unwrap(), b"existing-fact\n");
}

#[test]
fn incomplete_initialization_is_rejected_without_modifying_existing_control_directory() {
    let root = TemporaryDirectory::new("incomplete-init");
    let control_directory = root.path().join(".cyber-agent");
    fs::create_dir(&control_directory).unwrap();
    let canonical_control_directory = fs::canonicalize(&control_directory).unwrap();
    let partial = control_directory.join("partial-init-note");
    fs::write(&partial, b"do-not-reuse-or-clean").unwrap();

    assert!(matches!(
        error(Project::initialize(root.path())),
        ProjectEventStoreError::IncompleteInitialization { control_directory: received }
            if received == canonical_control_directory
    ));
    assert_eq!(fs::read(&partial).unwrap(), b"do-not-reuse-or-clean");
    assert!(!control_directory.join("project.toml").exists());
    assert!(matches!(
        error(Project::discover(root.path())),
        ProjectEventStoreError::IncompleteInitialization { .. }
    ));
}

#[test]
fn project_writer_lock_rejects_a_second_writer() {
    let root = TemporaryDirectory::new("writer-lock");
    let project = Project::initialize(root.path()).unwrap();
    let _first_writer = project.open_writer().unwrap();

    assert!(matches!(
        error(project.open_writer()),
        ProjectEventStoreError::WriterLocked { .. }
    ));
}

#[test]
fn project_writer_can_be_reacquired_immediately_after_drop() {
    let root = TemporaryDirectory::new("writer-lock-reacquire");
    let project = Project::initialize(root.path()).unwrap();

    let first_writer = project.open_writer().unwrap();
    drop(first_writer);

    let second_writer = project.open_writer().unwrap();
    drop(second_writer);
    assert!(project.open_writer().is_ok());
}

#[test]
fn subprocess_holds_project_writer_lock_until_released() {
    let Ok(root) = std::env::var("CYBER_AGENT_TEST_LOCK_HOLDER_ROOT") else {
        return;
    };
    let ready = std::env::var("CYBER_AGENT_TEST_LOCK_HOLDER_READY").unwrap();
    let release = std::env::var("CYBER_AGENT_TEST_LOCK_HOLDER_RELEASE").unwrap();
    let project = Project::discover(std::path::Path::new(&root)).unwrap();
    let _writer = project.open_writer().unwrap();
    fs::write(&ready, b"locked").unwrap();

    let deadline = Instant::now() + Duration::from_secs(10);
    while !std::path::Path::new(&release).exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn project_writer_lock_is_exclusive_across_processes_and_releases_after_exit() {
    let root = TemporaryDirectory::new("cross-process-lock");
    let project = Project::initialize(root.path()).unwrap();
    let ready = root.path().join("lock-ready");
    let release = root.path().join("lock-release");
    let mut child = Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("subprocess_holds_project_writer_lock_until_released")
        .arg("--nocapture")
        .env("CYBER_AGENT_TEST_LOCK_HOLDER_ROOT", project.root())
        .env("CYBER_AGENT_TEST_LOCK_HOLDER_READY", &ready)
        .env("CYBER_AGENT_TEST_LOCK_HOLDER_RELEASE", &release)
        .spawn()
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    let locked = project.open_writer();
    fs::write(&release, b"release").unwrap();
    let child_status = child.wait().unwrap();

    assert!(ready.exists());
    assert!(child_status.success());
    assert!(matches!(
        error(locked),
        ProjectEventStoreError::WriterLocked { .. }
    ));
    let _released_writer = project.open_writer().unwrap();
}

#[test]
fn event_store_port_enforces_strict_sequence_and_session_research_streams() {
    let root = TemporaryDirectory::new("strict-streams");
    let project = Project::initialize(root.path()).unwrap();
    let mut writer = project.open_writer().unwrap();
    let research_stream = ProjectEventStream::Research.stream_id();

    {
        let mut research = writer.open_research(None).unwrap();
        assert_event_store(&research);
        assert_eq!(research.next_sequence().unwrap(), EventSequence::new(1));
        EventSink::append_event(
            &mut research,
            &event(
                "research-1",
                &research_stream,
                1,
                json!({"kind": "research"}),
            ),
        )
        .unwrap();
        assert_eq!(research.next_sequence().unwrap(), EventSequence::new(2));
        assert!(matches!(
            error(EventSink::append_event(
                &mut research,
                &event("research-3", &research_stream, 3, json!({}))
            )),
            ProjectEventStoreError::Event(
                echidna::domain::events::EventError::UnexpectedSequence { .. }
            )
        ));
        assert_eq!(
            EventSource::read_events(&research, &research_stream)
                .unwrap()
                .len(),
            1
        );
    }

    let session_id = StreamId::new("session-a");
    let session_stream = ProjectEventStream::Session(session_id.clone()).stream_id();
    let mut session = writer.open_session(session_id, None).unwrap();
    EventSink::append_event(
        &mut session,
        &event("session-1", &session_stream, 1, json!({"kind": "session"})),
    )
    .unwrap();
    assert!(
        project
            .root()
            .join(".cyber-agent/sessions/session-a/events.jsonl")
            .is_file()
    );
}

#[test]
fn session_stream_namespace_never_collides_with_research() {
    let research_stream = ProjectEventStream::Research.stream_id();
    let session_id = StreamId::new("research");
    let session_stream = ProjectEventStream::Session(session_id.clone()).stream_id();

    assert_eq!(research_stream, StreamId::new("research"));
    assert_eq!(session_stream, StreamId::new("session:research"));
    assert_ne!(research_stream, session_stream);

    let root = TemporaryDirectory::new("stream-namespaces");
    let project = Project::initialize(root.path()).unwrap();
    let mut writer = project.open_writer().unwrap();
    {
        let mut research = writer.open_research(None).unwrap();
        EventSink::append_event(
            &mut research,
            &event("research-1", &research_stream, 1, json!({})),
        )
        .unwrap();
    }
    {
        let mut session = writer.open_session(session_id, None).unwrap();
        EventSink::append_event(
            &mut session,
            &event("session-1", &session_stream, 1, json!({})),
        )
        .unwrap();
    }

    let session_events = project
        .root()
        .join(".cyber-agent/sessions/research/events.jsonl");
    let stored: EventEnvelope<Value> =
        serde_json::from_slice(&fs::read(session_events).unwrap()).unwrap();
    assert_eq!(stored.stream_id, session_stream);
}

#[test]
fn corrupt_final_record_requires_metadata_before_any_mutation_then_recovers() {
    let root = TemporaryDirectory::new("tail-recovery");
    let project = Project::initialize(root.path()).unwrap();
    let stream = ProjectEventStream::Research.stream_id();
    let valid = event("event-1", &stream, 1, json!({"kept": true}));
    let mut source = serde_json::to_vec(&valid).unwrap();
    source.push(b'\n');
    let tail = b"{not valid}\n";
    let offset = source.len() as u64;
    source.extend_from_slice(tail);
    fs::write(research_path(&project), &source).unwrap();

    let mut writer = project.open_writer().unwrap();
    assert!(matches!(
        error(writer.open_research(None)),
        ProjectEventStoreError::RecoveryMetadataRequired { offset: received, .. } if received == offset
    ));
    assert_eq!(fs::read(research_path(&project)).unwrap(), source);
    assert_empty_recovery_directory(&project);

    let store = writer.open_research(Some(recovery_metadata())).unwrap();
    let events = EventSource::read_events(&store, &stream).unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[1].sequence, EventSequence::new(2));
    assert_eq!(events[1].event_type.as_str(), "StreamRecovered");
    assert_eq!(events[1].payload["truncated_from_offset"], json!(offset));
    let artifact = events[1].payload["recovery_artifact"].as_str().unwrap();
    assert_eq!(
        fs::read(project.root().join(".cyber-agent").join(artifact)).unwrap(),
        tail
    );
    assert!(
        !fs::read(research_path(&project))
            .unwrap()
            .windows(tail.len())
            .any(|window| window == tail)
    );
    assert_no_durable_temporary_files(&project.root().join(".cyber-agent/artifacts/recovery"));
}

#[test]
fn conflicting_existing_recovery_artifact_never_changes_the_source_stream() {
    let root = TemporaryDirectory::new("recovery-artifact-collision");
    let project = Project::initialize(root.path()).unwrap();
    let stream = ProjectEventStream::Research.stream_id();
    let valid = event("event-1", &stream, 1, json!({}));
    let mut source = serde_json::to_vec(&valid).unwrap();
    source.push(b'\n');
    let tail = b"{invalid-tail\n";
    source.extend_from_slice(tail);
    fs::write(research_path(&project), &source).unwrap();

    let digest = blake3::hash(tail).to_hex().to_string();
    let artifact = project
        .root()
        .join(".cyber-agent/artifacts/recovery")
        .join(format!("{digest}.bin"));
    fs::write(&artifact, b"partial-or-conflicting-artifact").unwrap();

    let mut writer = project.open_writer().unwrap();
    assert!(matches!(
        error(writer.open_research(Some(recovery_metadata()))),
        ProjectEventStoreError::RecoveryArtifactCollision { path } if path == artifact
    ));
    assert_eq!(fs::read(research_path(&project)).unwrap(), source);
    assert_eq!(
        fs::read(&artifact).unwrap(),
        b"partial-or-conflicting-artifact"
    );
}

#[test]
fn non_tail_corruption_reports_its_exact_offset_without_mutating_source() {
    let root = TemporaryDirectory::new("middle-corruption");
    let project = Project::initialize(root.path()).unwrap();
    let stream = ProjectEventStream::Research.stream_id();
    let mut source = serde_json::to_vec(&event("event-1", &stream, 1, json!({}))).unwrap();
    source.push(b'\n');
    let offset = source.len() as u64;
    source.extend_from_slice(b"not-json\n");
    source
        .extend_from_slice(&serde_json::to_vec(&event("event-2", &stream, 2, json!({}))).unwrap());
    source.push(b'\n');
    fs::write(research_path(&project), &source).unwrap();

    let mut writer = project.open_writer().unwrap();
    assert!(matches!(
        error(writer.open_research(None)),
        ProjectEventStoreError::NonTailCorruption { offset: received, .. } if received == offset
    ));
    assert_eq!(fs::read(research_path(&project)).unwrap(), source);
    assert_empty_recovery_directory(&project);
}

#[test]
fn parsed_semantic_tail_errors_are_read_only_even_with_recovery_metadata() {
    let root = TemporaryDirectory::new("semantic-tail-errors");
    let project = Project::initialize(root.path()).unwrap();
    let stream = ProjectEventStream::Research.stream_id();
    let first = event("event-1", &stream, 1, json!({}));
    let mut future_schema = event("event-1", &stream, 1, json!({}));
    future_schema.schema_version = echidna::domain::events::SchemaVersion::new(2);
    let mut missing_upcaster = event("event-1", &stream, 1, json!({}));
    missing_upcaster.schema_version = echidna::domain::events::SchemaVersion::new(0);
    let wrong_sequence = event("event-3", &stream, 3, json!({}));
    let wrong_stream = event("event-2", &StreamId::new("another-stream"), 2, json!({}));

    let cases = [
        (serde_json::to_vec(&future_schema).unwrap(), 0_u64),
        (serde_json::to_vec(&missing_upcaster).unwrap(), 0_u64),
        (
            [
                serde_json::to_vec(&first).unwrap(),
                serde_json::to_vec(&wrong_sequence).unwrap(),
            ]
            .join(&b'\n')
            .into_iter()
            .chain(std::iter::once(b'\n'))
            .collect::<Vec<_>>(),
            serde_json::to_vec(&first).unwrap().len() as u64 + 1,
        ),
        (
            [
                serde_json::to_vec(&first).unwrap(),
                serde_json::to_vec(&wrong_stream).unwrap(),
            ]
            .join(&b'\n')
            .into_iter()
            .chain(std::iter::once(b'\n'))
            .collect::<Vec<_>>(),
            serde_json::to_vec(&first).unwrap().len() as u64 + 1,
        ),
    ];

    for (source, offset) in cases {
        fs::write(research_path(&project), &source).unwrap();
        let mut writer = project.open_writer().unwrap();
        assert!(matches!(
            error(writer.open_research(Some(recovery_metadata()))),
            ProjectEventStoreError::InvalidEventRecord { offset: received, .. } if received == offset
        ));
        drop(writer);
        assert_eq!(fs::read(research_path(&project)).unwrap(), source);
        assert_empty_recovery_directory(&project);
    }
}

#[test]
fn first_sequence_must_be_one_and_cannot_force_tail_recovery() {
    let root = TemporaryDirectory::new("invalid-first-sequence");
    let project = Project::initialize(root.path()).unwrap();
    let stream = ProjectEventStream::Research.stream_id();
    let maximum = event("event-max", &stream, u64::MAX, json!({}));
    let mut source = serde_json::to_vec(&maximum).unwrap();
    source.push(b'\n');
    source.extend_from_slice(b"{invalid-tail\n");
    fs::write(research_path(&project), &source).unwrap();

    let mut writer = project.open_writer().unwrap();
    assert!(matches!(
        error(writer.open_research(Some(recovery_metadata()))),
        ProjectEventStoreError::InvalidEventRecord { offset: 0, .. }
    ));
    drop(writer);
    assert_eq!(fs::read(research_path(&project)).unwrap(), source);
    assert_empty_recovery_directory(&project);
}

#[test]
fn export_migrate_writes_metadata_and_preserves_the_source_stream() {
    let root = TemporaryDirectory::new("export-migrate");
    let project = Project::initialize(root.path()).unwrap();
    let stream = ProjectEventStream::Research.stream_id();
    {
        let mut writer = project.open_writer().unwrap();
        let mut store = writer.open_research(None).unwrap();
        EventSink::append_event(
            &mut store,
            &event("event-1", &stream, 1, json!({"preserve": true})),
        )
        .unwrap();
    }

    let source = research_path(&project);
    let before = fs::read(&source).unwrap();
    let destination = project.root().join("exports/research-current.jsonl");
    let manifest_path = migration_manifest_path(&destination);
    let mut writer = project.open_writer().unwrap();
    let manifest = writer
        .export_migrate_stream(
            &ProjectEventStream::Research,
            &destination,
            EventSchema::current(),
            "m2-s1-test",
        )
        .unwrap();

    assert_eq!(fs::read(&source).unwrap(), before);
    assert_eq!(fs::read(&destination).unwrap(), before);
    assert_eq!(
        manifest.source_hash,
        blake3::hash(&before).to_hex().to_string()
    );
    assert_eq!(manifest.target_schema, EventSchema::current());
    assert_eq!(manifest.tool_version, "m2-s1-test");
    let destination_bytes = fs::read(&destination).unwrap();
    let manifest_bytes = fs::read(&manifest_path).unwrap();
    let persisted: serde_json::Value = serde_json::from_slice(&manifest_bytes).unwrap();
    assert_eq!(persisted["source_hash"], manifest.source_hash);
    assert_eq!(persisted["target_schema"]["version"], 1);
    assert_no_durable_temporary_files(destination.parent().unwrap());

    assert!(matches!(
        error(writer.export_migrate_stream(
            &ProjectEventStream::Research,
            &destination,
            EventSchema::current(),
            "m2-s1-test",
        )),
        ProjectEventStoreError::ExportTargetExists { .. }
    ));
    assert_eq!(fs::read(&source).unwrap(), before);
    assert_eq!(fs::read(&destination).unwrap(), destination_bytes);
    assert_eq!(fs::read(&manifest_path).unwrap(), manifest_bytes);

    fs::remove_file(&manifest_path).unwrap();
    assert!(matches!(
        error(writer.export_migrate_stream(
            &ProjectEventStream::Research,
            &destination,
            EventSchema::current(),
            "m2-s1-test",
        )),
        ProjectEventStoreError::IncompleteMigration { .. }
    ));
    assert_eq!(fs::read(&source).unwrap(), before);
}

#[test]
fn empty_migration_tool_version_creates_no_output_pair() {
    let root = TemporaryDirectory::new("empty-migration-tool-version");
    let project = Project::initialize(root.path()).unwrap();
    let destination = project.root().join("exports/research-current.jsonl");
    let manifest_path = migration_manifest_path(&destination);
    let mut writer = project.open_writer().unwrap();

    assert!(matches!(
        error(writer.export_migrate_stream(
            &ProjectEventStream::Research,
            &destination,
            EventSchema::current(),
            "  ",
        )),
        ProjectEventStoreError::EmptyMigrationToolVersion
    ));
    assert!(!destination.exists());
    assert!(!manifest_path.exists());
}

#[test]
fn corrupt_completed_migration_outputs_are_incomplete_without_touching_source() {
    let root = TemporaryDirectory::new("corrupt-migration-output");
    let project = Project::initialize(root.path()).unwrap();
    let stream = ProjectEventStream::Research.stream_id();
    {
        let mut writer = project.open_writer().unwrap();
        let mut research = writer.open_research(None).unwrap();
        EventSink::append_event(
            &mut research,
            &event("event-1", &stream, 1, json!({"preserve": true})),
        )
        .unwrap();
    }

    let source = research_path(&project);
    let source_before = fs::read(&source).unwrap();
    let export = |destination: &std::path::Path| {
        let mut writer = project.open_writer().unwrap();
        writer
            .export_migrate_stream(
                &ProjectEventStream::Research,
                destination,
                EventSchema::current(),
                "m2-s1-test",
            )
            .unwrap();
    };
    let assert_incomplete = |destination: &std::path::Path| {
        let mut writer = project.open_writer().unwrap();
        assert!(matches!(
            error(writer.export_migrate_stream(
                &ProjectEventStream::Research,
                destination,
                EventSchema::current(),
                "m2-s1-test",
            )),
            ProjectEventStoreError::IncompleteMigration { .. }
        ));
    };

    let malformed_manifest_destination = project.root().join("exports/malformed-manifest.jsonl");
    export(&malformed_manifest_destination);
    fs::write(
        migration_manifest_path(&malformed_manifest_destination),
        b"not-json",
    )
    .unwrap();
    assert_incomplete(&malformed_manifest_destination);
    fs::write(
        migration_manifest_path(&malformed_manifest_destination),
        serde_json::to_vec(&json!({
            "source_hash": "not-a-blake3-digest",
            "target_schema": {"family": "wrong.family", "version": 1},
            "tool_version": ""
        }))
        .unwrap(),
    )
    .unwrap();
    assert_incomplete(&malformed_manifest_destination);

    let truncated_destination = project.root().join("exports/truncated-target.jsonl");
    export(&truncated_destination);
    fs::write(&truncated_destination, b"{\"truncated\"").unwrap();
    assert_incomplete(&truncated_destination);

    let semantic_destination = project.root().join("exports/semantic-target.jsonl");
    export(&semantic_destination);
    let invalid_stream = StreamId::new("not-research");
    let mut semantic_invalid =
        serde_json::to_vec(&event("invalid-target", &invalid_stream, 1, json!({}))).unwrap();
    semantic_invalid.push(b'\n');
    fs::write(&semantic_destination, semantic_invalid).unwrap();
    assert_incomplete(&semantic_destination);

    assert_eq!(fs::read(&source).unwrap(), source_before);
}
