//! Explicit hash requests verify bytes; metadata-only snapshots remain cheap.

use super::measurement::{cache_contents, git_fixture};
use super::*;
use std::fs::FileTimes;
use std::path::Path;

fn replace_same_metadata(path: &Path) {
    let before = fs::metadata(path).unwrap();
    fs::write(path, "alpha=11\n").unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(path)
        .unwrap()
        .set_times(FileTimes::new().set_modified(before.modified().unwrap()))
        .unwrap();
    let after = fs::metadata(path).unwrap();
    assert_eq!(before.len(), after.len());
    assert_eq!(before.modified().unwrap(), after.modified().unwrap());
}

async fn inspect(root: &Path, paths: Value, observed_at: &str) -> Value {
    // Recreate the Registry on every call: no prior in-memory state is required.
    let tools = ToolRegistry::for_project(root);
    let outcome = execute(
        &tools,
        "freshness",
        "inspect_project",
        json!({"observed_at":observed_at, "hash_paths":paths}),
    )
    .await;
    assert_eq!(outcome.result.status, ToolResultStatus::Succeeded);
    outcome.result.output
}

fn entry<'a>(output: &'a Value, path: &str) -> &'a Value {
    output["file_manifest"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["relative_path"] == path)
        .unwrap()
}

async fn changed_case(case: &str) {
    let (_temp, project) = initialized_project(case);
    let root = project.root();
    let path = if case == "memory" {
        ".cyber-agent/memory/core.md"
    } else {
        "source.txt"
    };
    fs::create_dir_all(root.join(".cyber-agent/memory")).unwrap();
    fs::write(root.join(path), "alpha=10\n").unwrap();
    if case.starts_with("git") {
        fs::write(root.join("anchor.txt"), "anchor\n").unwrap();
        git_fixture(root, &["init", "--quiet"]);
        git_fixture(root, &["add", "--", "anchor.txt"]);
        if case != "git_untracked" {
            if case == "git_dirty" {
                fs::write(root.join(path), "alpha=09\n").unwrap();
            }
            git_fixture(root, &["add", "--", path]);
        }
        git_fixture(root, &["commit", "--quiet", "-m", "fixture"]);
        if case == "git_dirty" {
            fs::write(root.join(path), "alpha=10\n").unwrap();
        }
    }
    let first = inspect(root, json!([path]), "cold").await;
    let old_hash = blake3::hash(b"alpha=10\n").to_hex().to_string();
    assert_eq!(entry(&first, path)["blake3"], old_hash);
    replace_same_metadata(&root.join(path));
    if case == "filesystem_rebuild" {
        fs::write(root.join("new.txt"), "unrelated metadata change\n").unwrap();
    }
    let result = inspect(root, json!([path, format!("./{path}"), path]), "changed").await;
    assert_eq!(
        entry(&result, path)["blake3"],
        blake3::hash(b"alpha=11\n").to_hex().to_string(),
        "{case}"
    );
    assert_eq!(result["hashed_paths"], json!([path]));
    assert_eq!(
        result["status"],
        if matches!(case, "git_clean" | "filesystem_rebuild") {
            "full_rebuild"
        } else {
            "incremental_refresh"
        }
    );
    assert_eq!(
        result["snapshot"]["source_fingerprint"] == first["snapshot"]["source_fingerprint"],
        !matches!(case, "git_clean" | "filesystem_rebuild")
    );
    let persisted: Value = serde_json::from_slice(
        &fs::read(root.join(".cyber-agent/cache/file-manifest.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(persisted, result["file_manifest"]);
    let bytes = cache_contents(root);
    let warm = inspect(root, json!([path]), "later-warm").await;
    assert_eq!(warm["status"], "hit");
    assert_eq!(warm["hashed_paths"], json!([path]));
    assert_eq!(entry(&warm, path), entry(&result, path));
    assert_eq!(cache_contents(root), bytes);
}

#[tokio::test]
async fn explicit_hash_revalidates_same_metadata_on_cache_hit() {
    for case in ["filesystem", "git_dirty", "git_untracked", "memory"] {
        changed_case(case).await;
    }
}

#[tokio::test]
async fn explicit_hash_revalidates_same_metadata_on_rebuild() {
    for case in ["filesystem_rebuild", "git_clean"] {
        changed_case(case).await;
    }
}

#[tokio::test]
async fn metadata_only_reuse_does_not_claim_explicit_hash_work() {
    let (_temp, project) = initialized_project("metadata-only");
    let root = project.root();
    fs::write(root.join("source.txt"), "alpha=10\n").unwrap();
    fs::write(root.join("other.txt"), "unrequested\n").unwrap();
    let first = inspect(root, json!(["source.txt"]), "cold").await;
    assert!(entry(&first, "other.txt")["blake3"].is_null());
    let cache = cache_contents(root);
    replace_same_metadata(&root.join("source.txt"));
    let cheap = inspect(root, json!([]), "cheap").await;
    assert_eq!(cheap["status"], "hit");
    assert_eq!(cheap["hashed_paths"], json!([]));
    assert_eq!(cache_contents(root), cache);
    let explicit = inspect(root, json!(["source.txt"]), "explicit").await;
    assert_ne!(
        entry(&explicit, "source.txt")["blake3"],
        entry(&cheap, "source.txt")["blake3"]
    );
    assert!(entry(&explicit, "other.txt")["blake3"].is_null());
}

#[tokio::test]
async fn invalid_explicit_hashes_do_not_publish_partial_cache() {
    let (_temp, project) = initialized_project("invalid-hash");
    let root = project.root();
    fs::write(root.join("source.txt"), "alpha=10\n").unwrap();
    fs::create_dir(root.join("directory")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(root.join("source.txt"), root.join("link")).unwrap();
    inspect(root, json!(["source.txt"]), "cold").await;
    let cache = cache_contents(root);
    replace_same_metadata(&root.join("source.txt"));
    let mut paths = vec!["missing.txt", "directory", "../source.txt"];
    #[cfg(unix)]
    paths.push("link");
    for path in paths {
        let tools = ToolRegistry::for_project(root);
        let outcome = execute(
            &tools,
            "invalid",
            "inspect_project",
            json!({"observed_at":"fixture", "hash_paths":["source.txt", path]}),
        )
        .await;
        assert_eq!(outcome.result.status, ToolResultStatus::Failed);
        assert_eq!(cache_contents(root), cache);
    }
}
