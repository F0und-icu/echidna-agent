//! Fixed offline Tool-boundary measurements, not product telemetry or wire usage.

use std::io::Write;
use std::path::Path;
use std::time::Instant;

use super::{
    Command, TemporaryDirectory, ToolRegistry, ToolResultStatus, Value, execute, fs,
    initialized_project, json,
};

const CACHE_FILES: [&str; 3] = [
    "project-snapshot.json",
    "file-manifest.json",
    "tool-capabilities.json",
];
const SOURCE: &str = "source-000.txt";
const MEMORY: &str = ".cyber-agent/memory/core.md";

pub(super) fn cache_contents(root: &Path) -> Vec<Vec<u8>> {
    CACHE_FILES
        .iter()
        .map(|name| fs::read(root.join(".cyber-agent/cache").join(name)).unwrap())
        .collect()
}

pub(super) fn git_fixture(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "-c",
            "user.name=Cyber Agent Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn measure(tools: &ToolRegistry, id: &str) -> (Value, u128) {
    // The existing helper includes plan validation and real Tool execution.
    // Setup, mutation, test assertions and report serialization are outside timing.
    let start = Instant::now();
    let outcome = execute(
        tools,
        id,
        "inspect_project",
        json!({
            "hash_paths": [SOURCE, MEMORY]
        }),
    )
    .await;
    let elapsed_ns = start.elapsed().as_nanos();
    assert_eq!(outcome.result.status, ToolResultStatus::Succeeded);
    (outcome.result.output, elapsed_ns)
}

async fn four_cases(repetitions: usize, file_count: usize, file_bytes: usize, emit: bool) {
    for git in [false, true] {
        for repetition in 0..repetitions {
            // Rotate order to avoid always measuring the same case first.
            let cases = [
                "unchanged",
                "source_edit",
                "memory_edit",
                "external_log_growth",
            ];
            for offset in 0..cases.len() {
                let case = cases[(offset + repetition) % cases.len()];
                let (_root, project) = initialized_project("snapshot-measurement");
                let logs = TemporaryDirectory::new("snapshot-external-log");
                assert!(!logs.path().starts_with(project.root()));
                for index in 0..file_count {
                    fs::write(
                        project.root().join(format!("source-{index:03}.txt")),
                        vec![b'a'; file_bytes],
                    )
                    .unwrap();
                }
                fs::create_dir_all(project.root().join(".cyber-agent/memory")).unwrap();
                fs::write(project.root().join(MEMORY), "currency=USD\n").unwrap();
                fs::write(logs.path().join("run.log"), b"initial log\n").unwrap();
                if git {
                    git_fixture(project.root(), &["init", "--quiet"]);
                    git_fixture(project.root(), &["add", "--", "source-*.txt"]);
                    git_fixture(
                        project.root(),
                        &["commit", "--quiet", "-m", "ordinary fixture"],
                    );
                }
                let tools = ToolRegistry::for_project(project.root());
                let mut prior_fingerprint = Value::Null;
                let mut prior_cache = Vec::new();
                for phase in ["cold", "warm", "after_change", "warm_after_change"] {
                    if phase == "after_change" {
                        let changed = match case {
                            "source_edit" => Some(project.root().join(SOURCE)),
                            "memory_edit" => Some(project.root().join(MEMORY)),
                            "external_log_growth" => Some(logs.path().join("run.log")),
                            "unchanged" => None,
                            _ => unreachable!(),
                        };
                        if let Some(path) = changed {
                            fs::OpenOptions::new()
                                .append(true)
                                .open(path)
                                .unwrap()
                                .write_all(&vec![b'b'; 4096])
                                .unwrap();
                        }
                    }
                    let (output, elapsed_ns) = measure(&tools, phase).await;
                    let changes_project =
                        phase == "after_change" && matches!(case, "source_edit" | "memory_edit");
                    let expected_status = if phase == "cold" || changes_project {
                        "full_rebuild"
                    } else {
                        "hit"
                    };
                    assert_eq!(output["status"], expected_status, "{git} {case} {phase}");
                    let fingerprint = &output["snapshot"]["source_fingerprint"];
                    assert_eq!(fingerprint["kind"], if git { "git" } else { "filesystem" });
                    if phase != "cold" {
                        assert_eq!(fingerprint != &prior_fingerprint, changes_project);
                    }
                    let entries = output["file_manifest"]["entries"].as_array().unwrap();
                    assert!(
                        !entries
                            .iter()
                            .any(|entry| entry["relative_path"] == "run.log")
                    );
                    for path in [SOURCE, MEMORY] {
                        let entry = entries
                            .iter()
                            .find(|entry| entry["relative_path"] == path)
                            .unwrap();
                        let actual = fs::read(project.root().join(path)).unwrap();
                        assert_eq!(entry["size_bytes"], actual.len());
                        assert_eq!(entry["blake3"], blake3::hash(&actual).to_hex().to_string());
                    }
                    // R5b3: explicit requests verify both paths, independently
                    // of whether the persisted metadata cache is reused.
                    assert_eq!(output["hashed_paths"], json!([MEMORY, SOURCE]));
                    let cache = cache_contents(project.root());
                    if expected_status == "hit" {
                        assert_eq!(cache, prior_cache, "cache hit rewrote bytes");
                    }
                    if emit {
                        println!(
                            "SNAPSHOT_MEASUREMENT {}",
                            json!({
                                "schema": 1, "case": case, "git": git, "repetition": repetition,
                                "phase": phase, "fixture_files": file_count, "fixture_file_bytes": file_bytes,
                                "explicit_hash_paths": [SOURCE, MEMORY],
                                "tool_boundary_elapsed_ns": elapsed_ns,
                                "tool_result_json_bytes": serde_json::to_vec(&output).unwrap().len(),
                                "persisted_cache_bytes": cache.iter().map(Vec::len).sum::<usize>(),
                                "disposition": output["status"], "hashed_paths": output["hashed_paths"],
                                "unavailable": ["internal_hash_calls", "internal_hash_bytes", "internal_hash_time",
                                    "directory_scans", "request_audit_bytes", "delivery_estimate", "wire_bytes",
                                    "provider_usage", "ttft", "agent_repeated_tools"]
                            })
                        );
                    }
                    prior_fingerprint = fingerprint.clone();
                    prior_cache = cache;
                }
            }
        }
    }
}

#[tokio::test]
async fn snapshot_four_case_contract_keeps_external_logs_outside_project() {
    four_cases(1, 4, 256, false).await;
}

#[tokio::test]
#[ignore = "offline measurement; run explicitly in release with --nocapture --test-threads=1"]
async fn snapshot_four_case_measurement() {
    four_cases(5, 64, 16 * 1024, true).await;
}
