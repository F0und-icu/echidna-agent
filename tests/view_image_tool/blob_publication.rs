//! Exercise the private byte store through its existing image Tool consumer.

use super::*;
use std::path::{Path, PathBuf};

async fn publish(tools: &ToolRegistry, source: &Path, suffix: &str) -> Value {
    let outcome = execute(
        tools,
        &format!("call-{suffix}"),
        "view_image",
        view_args(
            &format!("event-{suffix}"),
            &format!("artifact-{suffix}"),
            &source.display().to_string(),
        ),
    )
    .await;
    assert_eq!(outcome.result.status, ToolResultStatus::Succeeded);
    assert_eq!(outcome.receipt.output_artifact_ids.len(), 1);
    outcome.result.output
}

fn blob_path(project: &Project, bytes: &[u8]) -> PathBuf {
    project
        .root()
        .join(".cyber-agent/artifacts/blobs/sha256")
        .join(format!("{:x}", Sha256::digest(bytes)))
}

#[cfg(unix)]
#[tokio::test]
async fn verified_byte_publication_reuses_blob_without_directory_writes() {
    let (_directory, project) = initialized_project("blob-verified-reuse");
    let tools = ToolRegistry::for_project(project.root());
    record_target_snapshot(&tools).await;
    let source = project.root().join("sample.png");
    let bytes = png_bytes(32, 32);
    fs::write(&source, &bytes).unwrap();
    let cold = publish(&tools, &source, "cold").await;
    assert_eq!(cold["original"]["blob"]["already_present"], false);
    let blob = blob_path(&project, &bytes);
    let directory = blob.parent().unwrap();
    // A fixed old timestamp detects create+unlink even when no temp survives.
    // This observes directory mutation, not a syscall count or timing metric.
    let stamp = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_234_567_890);
    fs::File::open(directory)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(stamp))
        .unwrap();
    let before = fs::metadata(directory).unwrap().modified().unwrap();
    assert_eq!(before, stamp);
    let blob_modified = fs::metadata(&blob).unwrap().modified().unwrap();
    for index in 0..3 {
        // No in-memory cache can be necessary: compose a fresh Tool Registry.
        let tools = ToolRegistry::for_project(project.root());
        let warm = publish(&tools, &source, &format!("warm-{index}")).await;
        assert_eq!(warm["original"]["blob"]["already_present"], true);
        assert_eq!(
            warm["original"]["blob"]["blob_id"],
            cold["original"]["blob"]["blob_id"]
        );
        assert_eq!(
            warm["original"]["artifact"]["id"],
            format!("artifact-warm-{index}")
        );
        assert_eq!(fs::read(&blob).unwrap(), bytes);
        assert_eq!(
            fs::metadata(&blob).unwrap().modified().unwrap(),
            blob_modified
        );
        assert_eq!(fs::metadata(directory).unwrap().modified().unwrap(), before);
        assert_eq!(fs::read_dir(directory).unwrap().count(), 1);
    }
    // Identical bytes still create independent provenance-bearing Artifact facts.
    assert_eq!(research_events(&project).len(), 5);
}

#[tokio::test]
async fn byte_publication_rejects_same_metadata_corruption_and_recreates_missing_blob() {
    let (_directory, project) = initialized_project("blob-integrity-reuse");
    let tools = ToolRegistry::for_project(project.root());
    record_target_snapshot(&tools).await;
    let source = project.root().join("sample.png");
    let bytes = png_bytes(8, 8);
    fs::write(&source, &bytes).unwrap();
    publish(&tools, &source, "cold").await;
    let blob = blob_path(&project, &bytes);
    let before = fs::metadata(&blob).unwrap();
    let mut corrupt = bytes.clone();
    corrupt[0] ^= 1;
    fs::write(&blob, &corrupt).unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(&blob)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(before.modified().unwrap()))
        .unwrap();
    assert_eq!(fs::metadata(&blob).unwrap().len(), before.len());
    assert_eq!(
        fs::metadata(&blob).unwrap().modified().unwrap(),
        before.modified().unwrap()
    );
    let failed = execute(
        &tools,
        "call-corrupt",
        "view_image",
        view_args(
            "event-corrupt",
            "artifact-corrupt",
            &source.display().to_string(),
        ),
    )
    .await;
    assert_eq!(failed.result.status, ToolResultStatus::Failed);
    assert!(failed.receipt.output_artifact_ids.is_empty());
    assert_eq!(research_events(&project).len(), 2);
    assert_eq!(fs::read(&blob).unwrap(), corrupt);
    assert_eq!(fs::read_dir(blob.parent().unwrap()).unwrap().count(), 1);

    // Only this disposable fixture Blob is removed; publication must rebuild it.
    fs::remove_file(&blob).unwrap();
    let rebuilt = publish(&tools, &source, "recreated").await;
    assert_eq!(rebuilt["original"]["blob"]["already_present"], false);
    assert_eq!(fs::read(&blob).unwrap(), bytes);
    assert_eq!(research_events(&project).len(), 3);
}

#[cfg(unix)]
#[tokio::test]
async fn byte_publication_does_not_accept_nonregular_destinations() {
    for kind in ["directory", "symlink"] {
        let (_directory, project) = initialized_project("blob-nonregular");
        let tools = ToolRegistry::for_project(project.root());
        record_target_snapshot(&tools).await;
        let source = project.root().join("sample.png");
        let bytes = png_bytes(8, 8);
        fs::write(&source, &bytes).unwrap();
        let blob = blob_path(&project, &bytes);
        fs::create_dir_all(blob.parent().unwrap()).unwrap();
        if kind == "directory" {
            fs::create_dir(&blob).unwrap();
        } else {
            std::os::unix::fs::symlink(&source, &blob).unwrap();
        }
        let failed = execute(
            &tools,
            "call-invalid",
            "view_image",
            view_args(
                "event-invalid",
                "artifact-invalid",
                &source.display().to_string(),
            ),
        )
        .await;
        assert_eq!(failed.result.status, ToolResultStatus::Failed, "{kind}");
        assert!(failed.receipt.output_artifact_ids.is_empty());
        assert_eq!(research_events(&project).len(), 1);
        assert_eq!(fs::read(&source).unwrap(), bytes);
        assert_eq!(fs::read_dir(blob.parent().unwrap()).unwrap().count(), 1);
    }
}
