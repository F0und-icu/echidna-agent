mod support;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use echidna::adapters::project_event_store::Project;
use echidna::adapters::tools::ToolRegistry;
use echidna::domain::tools::{ExecutionId, ToolCall, ToolCallId, ToolName, ToolResultStatus};
use echidna::ports::tool::ToolExecutor;
use serde_json::{Value, json};

use support::TemporaryDirectory;

fn run(directory: &TemporaryDirectory, args: &[&str]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .current_dir(directory.path())
        .args(args)
        .output()
        .expect("inspection command should run");
    assert!(output.status.success(), "stderr: {:?}", output.stderr);
    serde_json::from_slice(&output.stdout).expect("inspection output should be JSON")
}

fn file_bytes(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    std::fs::read_dir(root)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().into();
            (name, std::fs::read(path).unwrap())
        })
        .collect()
}

#[test]
fn inspection_commands_report_uninitialized_projects_without_secrets_or_side_effects() {
    let directory = TemporaryDirectory::new("inspection-uninitialized");
    let before = std::fs::read_dir(directory.path()).unwrap().count();

    for args in [
        ["prompt", "inspect"].as_slice(),
        ["snapshot"].as_slice(),
        ["metrics"].as_slice(),
    ] {
        let report = run(&directory, args);
        assert_eq!(report["status"], "uninitialized");
        let encoded = report.to_string();
        assert!(!encoded.contains("Authorization"));
        assert!(!encoded.contains("api_key"));
        assert!(!encoded.contains("OPENAI_API_KEY"));
    }

    assert_eq!(
        std::fs::read_dir(directory.path()).unwrap().count(),
        before,
        "inspection must not initialize a project"
    );
}

#[test]
fn inspection_commands_read_initialized_project_without_building_snapshot_or_session() {
    let directory = TemporaryDirectory::new("inspection-empty-project");
    let initialized = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .current_dir(directory.path())
        .arg("init")
        .output()
        .expect("init should run");
    assert!(initialized.status.success());

    let prompt = run(&directory, &["prompt", "inspect"]);
    assert_eq!(prompt["status"], "no_request");
    let metrics = run(&directory, &["metrics"]);
    assert_eq!(metrics["status"], "no_session");
    let snapshot = run(&directory, &["snapshot"]);
    assert_eq!(snapshot["status"], "ok");
    assert_eq!(snapshot["disposition"], "full_rebuild");
    assert_eq!(snapshot["reason"], "cache_absent");
    assert!(snapshot["project_id"].as_str().is_some());
    assert!(!directory.path().join(".echidna/cache").exists());
}

#[tokio::test]
async fn snapshot_inspection_validates_cache_without_rebuilding_or_rewriting_it() {
    let directory = TemporaryDirectory::new("inspection-snapshot-cache");
    let project = Project::initialize(directory.path()).unwrap();
    std::fs::write(project.root().join("target.txt"), "version one\n").unwrap();
    let tools = ToolRegistry::for_project(project.root());
    let call = ToolCall::new(
        ToolCallId::new("build-snapshot-cache"),
        ToolName::new("inspect_project"),
        json!({"observed_at": "2026-08-23T12:00:00Z"}),
    );
    let plan = tools
        .plan(ExecutionId::new("build-snapshot-cache"), call)
        .unwrap();
    let outcome = tools.execute(plan).await.expect("Tool execution completes");
    assert_eq!(outcome.result.status, ToolResultStatus::Succeeded);

    let cache = project.root().join(".echidna/cache");
    let before = file_bytes(&cache);
    let hit = run(&directory, &["snapshot"]);
    assert_eq!(hit["status"], "ok");
    assert_eq!(hit["disposition"], "hit");
    assert_eq!(hit["reason"], "cache_valid");
    assert_eq!(file_bytes(&cache), before);

    std::fs::write(project.root().join("target.txt"), "version two\n").unwrap();
    let stale = run(&directory, &["snapshot"]);
    assert_eq!(stale["status"], "ok");
    assert_eq!(stale["disposition"], "full_rebuild");
    assert_eq!(stale["reason"], "source_fingerprint_changed");
    assert_eq!(file_bytes(&cache), before);
}

#[tokio::test]
async fn snapshot_inspection_reports_a_current_hit_after_an_incremental_cache_refresh() {
    let directory = TemporaryDirectory::new("inspection-incremental-cache");
    let project = Project::initialize(directory.path()).unwrap();
    std::fs::write(project.root().join("target.txt"), "fixture\n").unwrap();
    let tools = ToolRegistry::for_project(project.root());

    for (execution_id, hash_paths) in [
        ("build-cache", json!([])),
        ("extend-cache", json!(["target.txt"])),
    ] {
        let call = ToolCall::new(
            ToolCallId::new(execution_id),
            ToolName::new("inspect_project"),
            json!({
                "observed_at": "2026-08-23T12:00:00Z",
                "hash_paths": hash_paths,
            }),
        );
        let plan = tools.plan(ExecutionId::new(execution_id), call).unwrap();
        let outcome = tools.execute(plan).await.expect("Tool execution completes");
        assert_eq!(outcome.result.status, ToolResultStatus::Succeeded);
    }

    let cache = project.root().join(".echidna/cache");
    let before = file_bytes(&cache);
    let report = run(&directory, &["snapshot"]);
    assert_eq!(report["status"], "ok");
    assert_eq!(report["disposition"], "hit");
    assert_eq!(report["reason"], "cache_valid");
    assert_eq!(file_bytes(&cache), before);
}
