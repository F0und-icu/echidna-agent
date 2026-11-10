use crate::adapters::artifact_store::LocalArtifactStore;
use crate::adapters::project_event_store::Project;
use crate::ports::artifact::ArtifactStore;
use crate::support::TemporaryDirectory;

#[test]
fn artifact_pages_reuse_verification_but_detect_same_length_rewrites() {
    let root = TemporaryDirectory::new("verified-artifact-pages");
    let project = Project::initialize(root.path()).unwrap();
    let store = LocalArtifactStore::for_project(&project);
    let bytes = vec![b'a'; 256 * 1024];
    let blob = store.publish_bytes(&bytes).unwrap().blob_id;
    let first = store.read_artifact_range(&blob, 0, 16).unwrap();
    let next = store.read_artifact_range(&blob, 128 * 1024, 16).unwrap();
    assert_eq!(first, next);
    assert_eq!(next.0, vec![b'a'; 16]);
    let path = root
        .path()
        .join(".echidna/artifacts/blobs/sha256")
        .join(blob.as_str());
    let mut changed = bytes;
    changed[0] = b'b';
    std::fs::write(path, changed).unwrap();
    assert!(store.read_artifact_range(&blob, 128 * 1024, 16).is_err());
}

#[test]
fn resume_checks_current_sources_without_auditing_retired_requests() {
    use crate::app::session::SessionProjection;
    use crate::domain::{
        artifacts::BlobId, events::EventEnvelope, project::ProjectId,
        research::ResearchReferenceCatalog,
    };
    use crate::ports::artifact::StoredBlob;
    use serde_json::Value;
    use std::{collections::BTreeSet, fs, path::Path, sync::Mutex};

    struct Reads<'a> {
        store: &'a LocalArtifactStore,
        ids: Mutex<BTreeSet<BlobId>>,
    }
    impl ArtifactStore for Reads<'_> {
        type Error = String;
        fn read_blob(&self, id: &BlobId) -> Result<Vec<u8>, String> {
            self.ids.lock().unwrap().insert(id.clone());
            self.store.read_blob(id).map_err(|e| e.to_string())
        }
        fn publish_bytes(&self, _: &[u8]) -> Result<StoredBlob, String> {
            panic!("recovery must not publish Blobs")
        }
        fn publish_file(&self, _: &Path) -> Result<StoredBlob, String> {
            panic!("recovery must not publish Blobs")
        }
    }
    let root = TemporaryDirectory::new("resume-recent-proof");
    let project = Project::initialize(root.path()).unwrap();
    let store = LocalArtifactStore::for_project(&project);
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/task_context_proof_v2");
    let manifest: Value =
        serde_json::from_slice(&fs::read(fixture.join("manifest.json")).unwrap()).unwrap();
    let mut events = Vec::<EventEnvelope<Value>>::new();
    for (path, hash) in manifest["files"].as_object().unwrap() {
        let bytes = fs::read(fixture.join("blobs").join(hash.as_str().unwrap())).unwrap();
        if path.starts_with(".echidna/artifacts/blobs/sha256/") {
            store.publish_bytes(&bytes).unwrap();
        } else if path.ends_with("/events.jsonl") {
            events = std::str::from_utf8(&bytes)
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
        }
    }
    let id = ProjectId::new(manifest["project_id"].as_str().unwrap());
    let reads = Reads {
        store: &store,
        ids: Mutex::new(BTreeSet::new()),
    };
    let full = SessionProjection::replay_project_verified(
        id.clone(),
        &events,
        &[],
        || Ok(ResearchReferenceCatalog::default()),
        &reads,
    )
    .unwrap();
    let full_reads = std::mem::take(&mut *reads.ids.lock().unwrap());
    let recent = SessionProjection::replay_for_resume(
        id.clone(),
        &events,
        &[],
        || Ok(ResearchReferenceCatalog::default()),
        &reads,
    )
    .unwrap();
    assert_eq!(full.state(), recent.state());
    let (source, _) =
        crate::domain::session::SessionSourceIdentity::from_events(id.clone(), &events, &[])
            .unwrap();
    assert_eq!(full.task_source_identity(id.clone(), &[]).unwrap(), source);
    assert_eq!(
        recent.task_source_identity(id.clone(), &[]).unwrap(),
        source
    );
    assert_eq!(full.task_source_events(), recent.task_source_events());
    assert!(recent.task_source_events().len() < events.len());
    let recent_reads = std::mem::take(&mut *reads.ids.lock().unwrap());
    let old = full_reads
        .difference(&recent_reads)
        .next()
        .expect("Resume must omit retired request Blobs");
    let blobs = root.path().join(".echidna/artifacts/blobs/sha256");
    fs::remove_file(blobs.join(old.as_str())).unwrap();
    assert!(
        SessionProjection::replay_for_resume(
            id.clone(),
            &events,
            &[],
            || Ok(ResearchReferenceCatalog::default()),
            &reads
        )
        .is_ok()
    );
    assert!(
        SessionProjection::replay_project_verified(
            id.clone(),
            &events,
            &[],
            || Ok(ResearchReferenceCatalog::default()),
            &reads
        )
        .is_err()
    );
    let capture = &recent
        .state()
        .latest_task_context_selection()
        .unwrap()
        .1
        .capture_blob;
    fs::remove_file(blobs.join(capture.as_str())).unwrap();
    assert!(
        SessionProjection::replay_for_resume(
            id,
            &events,
            &[],
            || Ok(ResearchReferenceCatalog::default()),
            &reads
        )
        .is_err()
    );
}
