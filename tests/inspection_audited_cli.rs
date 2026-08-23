mod support;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use cyber_agent::adapters::project_event_store::{Project, ProjectEventStream};
use cyber_agent::app::request_audit::RequestAuditService;
use cyber_agent::app::session::SessionProjection;
use cyber_agent::domain::context::{
    ModelIdentity, ModelProfile, ProviderSessionId, ProviderSessionRef, ProviderSessionRole,
};
use cyber_agent::domain::events::{EventId, EventSequence, OccurredAt, StreamId};
use cyber_agent::domain::model::{
    CapabilityMappingVersion, ContentItem, ModelCapabilities, ModelInputIR,
    ModelRequestRequirements,
};
use cyber_agent::domain::request_audit::{
    RequestAdapterVersion, RequestAuditAttempt, RequestAuditBinding, RequestAuditDelivery,
    RequestAuditRunContext,
};
use cyber_agent::domain::session::SessionEvent;
use cyber_agent::ports::EventSource;
use cyber_agent::ports::artifact::{ArtifactStore, StoredBlob};
use serde_json::Value;
use sha2::{Digest, Sha256};

use support::TemporaryDirectory;

struct FileBlobStore {
    root: PathBuf,
}

impl FileBlobStore {
    fn new(project_root: &Path) -> Self {
        let root = project_root.join(".cyber-agent/artifacts/blobs/sha256");
        fs::create_dir_all(&root).unwrap();
        Self { root }
    }
}

impl ArtifactStore for FileBlobStore {
    type Error = std::io::Error;

    fn publish_file(&self, _source: &Path) -> Result<StoredBlob, Self::Error> {
        unreachable!("the audit fixture publishes rendered bytes")
    }

    fn publish_bytes(&self, bytes: &[u8]) -> Result<StoredBlob, Self::Error> {
        let blob_id = format!("{:x}", Sha256::digest(bytes));
        let path = self.root.join(&blob_id);
        let already_present = path.exists();
        if !already_present {
            fs::write(path, bytes)?;
        }
        Ok(StoredBlob {
            blob_id: cyber_agent::domain::artifacts::BlobId::new(blob_id).unwrap(),
            size_bytes: bytes.len() as u64,
            already_present,
        })
    }

    fn read_blob(
        &self,
        blob_id: &cyber_agent::domain::artifacts::BlobId,
    ) -> Result<Vec<u8>, Self::Error> {
        fs::read(self.root.join(blob_id.as_str()))
    }
}

fn capabilities() -> ModelCapabilities {
    ModelCapabilities {
        text_input: true,
        image_input: false,
        streaming: true,
        tool_call: false,
        structured_output: false,
        incremental_input: true,
        usage_reporting: false,
        context_window: 4_096,
        capability_mapping_version: CapabilityMappingVersion::new(1),
    }
}

fn record_audit(root: &TemporaryDirectory) -> Vec<String> {
    let project = Project::initialize(root.path()).unwrap();
    let session_name = StreamId::new("inspection-session");
    let stream_id = ProjectEventStream::Session(session_name.clone()).stream_id();
    let blobs = FileBlobStore::new(root.path());
    let binding =
        RequestAuditBinding::new(project.project_id().clone(), stream_id.clone()).unwrap();
    let context = RequestAuditRunContext::new(
        binding.clone(),
        RequestAdapterVersion::new("fixture-adapter-v1").unwrap(),
    );
    let input = ModelInputIR::current(
        "inspect the local project",
        vec![ContentItem::user_text(
            "Authorization previous_response_id transport CYBER_AGENT_OPENAI_API_KEY",
        )],
        ModelRequestRequirements::default(),
    );
    let attempt = RequestAuditAttempt::new(
        &context,
        ModelIdentity::new("fixture", "model"),
        ModelProfile::new("fixture-profile", 1),
        capabilities(),
        input.clone(),
        input,
        RequestAuditDelivery::full(
            cyber_agent::domain::context::BaselineInvalidationReason::NoBaseline,
        ),
    )
    .unwrap();

    let mut writer = project.open_writer().unwrap();
    let mut store = writer.open_session(session_name, None).unwrap();
    let mut projection =
        SessionProjection::replay(EventSource::read_events(&store, &stream_id).unwrap()).unwrap();
    let started = SessionEvent::Started {
        primary_provider_session: ProviderSessionRef {
            id: ProviderSessionId::new("primary-inspection"),
            role: ProviderSessionRole::Primary,
            generation: 1,
        },
    }
    .into_envelope(
        EventId::new("inspection-started"),
        stream_id.clone(),
        EventSequence::new(1),
        OccurredAt::new("2026-08-23T00:00:00Z"),
    )
    .unwrap();
    projection.append(&mut store, started).unwrap();
    let service = RequestAuditService::new(&blobs, binding);
    let manifest = service
        .record(
            attempt,
            EventId::new("inspection-audit"),
            OccurredAt::new("2026-08-23T00:00:01Z"),
            &mut projection,
            &mut store,
        )
        .unwrap();
    manifest
        .components()
        .iter()
        .map(|component| component.blob_id().as_str().to_owned())
        .collect()
}

fn cli(root: &TemporaryDirectory, args: &[&str]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_cyber-agent"))
        .current_dir(root.path())
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "stderr: {:?}", output.stderr);
    serde_json::from_slice(&output.stdout).unwrap()
}

fn cli_failure(root: &TemporaryDirectory, args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_cyber-agent"))
        .current_dir(root.path())
        .args(args)
        .output()
        .unwrap();
    assert!(!output.status.success());
    String::from_utf8(output.stderr).unwrap()
}

fn project_file_bytes(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn collect(root: &Path, current: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(current).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).unwrap();
            if metadata.is_dir() {
                collect(root, &path, files);
            } else if metadata.is_file() {
                files.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }

    let mut files = BTreeMap::new();
    collect(root, root, &mut files);
    files
}

#[test]
fn audited_prompt_metrics_are_read_only_and_missing_blob_is_explicit() {
    let root = TemporaryDirectory::new("inspection-audited");
    let blob_ids = record_audit(&root);
    let before_inspection = project_file_bytes(root.path());

    let prompt = cli(&root, &["prompt", "inspect"]);
    assert_eq!(prompt["status"], "ok");
    assert_eq!(prompt["request"]["event"]["event_id"], "inspection-audit");
    assert_eq!(prompt["request"]["components"].as_array().unwrap().len(), 3);
    assert_eq!(prompt["request"]["provider_usage"], "unavailable");
    assert_eq!(prompt["request"]["manifest_schema_version"], 1);
    assert_eq!(prompt["request"]["capability_mapping"]["schema_version"], 1);
    let prompt_text = prompt.to_string();
    for sentinel in [
        "Authorization",
        "previous_response_id",
        "transport",
        "CYBER_AGENT_OPENAI_API_KEY",
    ] {
        assert!(!prompt_text.contains(sentinel));
    }

    let metrics = cli(&root, &["metrics"]);
    assert_eq!(metrics["status"], "ok");
    assert_eq!(metrics["requests"], 1);
    assert_eq!(metrics["full_requests"], 1);
    assert_eq!(metrics["incremental_requests"], 0);
    assert_eq!(metrics["fallbacks"], 0);
    assert_eq!(metrics["provider_usage"], "unavailable");
    assert_eq!(
        cli(
            &root,
            &["prompt", "inspect", "--session", "inspection-session"]
        ),
        prompt
    );
    assert_eq!(
        cli(&root, &["metrics", "--session", "inspection-session"]),
        metrics
    );
    assert_eq!(project_file_bytes(root.path()), before_inspection);
    assert!(!root.path().join(".cyber-agent/cache").exists());

    fs::remove_file(
        root.path()
            .join(".cyber-agent/artifacts/blobs/sha256")
            .join(&blob_ids[0]),
    )
    .unwrap();
    let corrupt = cli(&root, &["prompt", "inspect"]);
    assert_eq!(corrupt["status"], "corrupt_reference");
    assert_eq!(
        corrupt["diagnostics"][0]["code"],
        "CORRUPT_REQUEST_AUDIT_REFERENCE"
    );
    let corrupt_metrics = cli(&root, &["metrics"]);
    assert_eq!(corrupt_metrics["status"], "corrupt_reference");
}

#[test]
fn corrupt_session_stream_is_reported_without_repair_or_mutation() {
    let root = TemporaryDirectory::new("inspection-corrupt-session");
    record_audit(&root);
    let stream = root
        .path()
        .join(".cyber-agent/sessions/inspection-session/events.jsonl");
    let mut bytes = fs::read(&stream).unwrap();
    bytes.extend_from_slice(b"{corrupt-tail\n");
    fs::write(&stream, &bytes).unwrap();

    let prompt = cli(&root, &["prompt", "inspect"]);
    assert_eq!(prompt["status"], "corrupt_session");
    let metrics = cli(&root, &["metrics"]);
    assert_eq!(metrics["status"], "corrupt_session");
    assert_eq!(fs::read(&stream).unwrap(), bytes);
}

#[test]
fn session_selection_is_explicit_for_missing_or_ambiguous_sessions() {
    let root = TemporaryDirectory::new("inspection-session-selection");
    record_audit(&root);

    let missing = cli_failure(&root, &["prompt", "inspect", "--session", "missing"]);
    assert!(missing.contains("Session `missing` does not exist"));

    let project = Project::discover(root.path()).unwrap();
    let second_session = StreamId::new("second-session");
    let second_stream = ProjectEventStream::Session(second_session.clone()).stream_id();
    let mut writer = project.open_writer().unwrap();
    let mut store = writer.open_session(second_session, None).unwrap();
    let mut projection =
        SessionProjection::replay(EventSource::read_events(&store, &second_stream).unwrap())
            .unwrap();
    let started = SessionEvent::Started {
        primary_provider_session: ProviderSessionRef {
            id: ProviderSessionId::new("primary-second"),
            role: ProviderSessionRole::Primary,
            generation: 1,
        },
    }
    .into_envelope(
        EventId::new("second-started"),
        second_stream,
        EventSequence::new(1),
        OccurredAt::new("2026-08-23T00:00:02Z"),
    )
    .unwrap();
    projection.append(&mut store, started).unwrap();

    let ambiguous = cli_failure(&root, &["metrics"]);
    assert!(ambiguous.contains("select one explicitly with `--session <session-id>`"));
    let selected = cli(&root, &["metrics", "--session", "inspection-session"]);
    assert_eq!(selected["session_id"], "inspection-session");
}

#[cfg(unix)]
#[test]
fn inspection_rejects_a_symlinked_session_event_stream() {
    use std::os::unix::fs::symlink;

    let root = TemporaryDirectory::new("inspection-session-symlink");
    record_audit(&root);
    let session = root.path().join(".cyber-agent/sessions/inspection-session");
    let stream = session.join("events.jsonl");
    let retained = session.join("retained-events.jsonl");
    fs::rename(&stream, &retained).unwrap();
    symlink(&retained, &stream).unwrap();

    let before = fs::read(&retained).unwrap();
    let error = cli_failure(&root, &["prompt", "inspect"]);
    assert!(error.contains("not a regular file"));
    assert_eq!(fs::read(&retained).unwrap(), before);
}
