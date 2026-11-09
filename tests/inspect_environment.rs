#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use echidna::adapters::tools::ToolRegistry;
use echidna::domain::tools::{
    ExecutionId, ToolCachePolicy, ToolCall, ToolCallId, ToolDeterminism, ToolEffect, ToolExposure,
    ToolName, ToolParallelism, ToolResultStatus, ToolRetryPolicy,
};
use echidna::ports::tool::ToolExecutor;
use serde_json::{Value, json};

mod support;

const FIXTURE_ENV: &str = "CYBER_AGENT_INSPECT_ENV_FIXTURE";

#[test]
fn inspect_environment_contract_is_read_only_noncacheable_and_direct() {
    let tools = ToolRegistry::for_project(".");
    let spec = tools
        .catalog()
        .into_iter()
        .find(|spec| spec.name.as_str() == "inspect_environment")
        .unwrap();
    assert_eq!(spec.exposure, ToolExposure::Direct);
    assert_eq!(spec.parallelism, ToolParallelism::Parallel);
    assert_eq!(spec.effect, ToolEffect::ReadOnly);
    assert_eq!(spec.retry_policy, ToolRetryPolicy::ReplaySafe);
    assert_eq!(spec.determinism, ToolDeterminism::EnvironmentDependent);
    assert_eq!(spec.cache_policy, ToolCachePolicy::None);
    assert_eq!(spec.resource_key_template, None);
    let properties = spec.input_schema["properties"].as_object().unwrap();
    assert_eq!(properties.len(), 1);
    assert!(properties.contains_key("execution"));
    assert!(!properties.contains_key("probe_path"));
}

/// This test is launched in a separate process by the tests below. The
/// process inherits a fixture-only PATH, so production code never needs an
/// environment override or a test-only public entry point.
#[tokio::test]
async fn inspect_environment_fixture_child() {
    let Ok(fixture) = std::env::var(FIXTURE_ENV) else {
        return;
    };
    let root = if fixture == "bad-cwd" {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("nonexistent-environment-fixture-cwd")
    } else {
        Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
    };
    let tools = ToolRegistry::for_project(root);
    let plan = tools
        .plan(
            ExecutionId::new("inspect-environment-fixture-execution"),
            ToolCall::new(
                ToolCallId::new("inspect-environment-fixture-call"),
                ToolName::new("inspect_environment"),
                json!({}),
            ),
        )
        .expect("inspect_environment fixture arguments must be valid");
    let outcome = tools.execute(plan).await.expect("Tool execution completes");
    assert_eq!(
        outcome.result.status,
        ToolResultStatus::Succeeded,
        "{fixture}"
    );
    // Serial libtest output may leave its test-name prefix on the current line.
    println!(
        "\nENV_RESULT={}",
        serde_json::to_string(&outcome.result.output).expect("environment output is JSON")
    );
}

#[test]
fn inspect_environment_reports_available_versions_and_device_counts_without_raw_output() {
    let fixture = support::TemporaryDirectory::new("inspect-environment-available");
    write_available_fixture(fixture.path());
    let output = run_fixture_child("available", fixture.path());

    assert_eq!(output["runtime"]["name"], "cyber-agent");
    assert_eq!(output["runtime"]["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(output["git"]["status"], "available");
    assert_eq!(output["git"]["version"], "2.45.1");
    assert_eq!(output["os"]["kernel"]["status"], "available");
    assert_eq!(output["proxy"]["http_configured"], true);

    let toolchain = output["toolchain"].as_array().expect("toolchain array");
    let names = toolchain
        .iter()
        .map(|entry| entry["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(names, ["cargo", "java", "node", "python3", "rustc"]);
    assert!(toolchain.iter().all(|entry| entry["status"] == "available"));
    assert_eq!(
        toolchain
            .iter()
            .find(|entry| entry["name"] == "node")
            .unwrap()["version"],
        "22.3.0"
    );
    assert_eq!(
        toolchain
            .iter()
            .find(|entry| entry["name"] == "rustc")
            .unwrap()["version"],
        "1.93.1"
    );

    let devices = output["connected_devices"]
        .as_array()
        .expect("connected device array");
    assert_eq!(
        devices
            .iter()
            .map(|entry| entry["provider"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["adb", "xcrun"]
    );
    assert_eq!(devices[0]["count"], 1);
    assert_eq!(devices[1]["count"], 2);
    assert_eq!(devices[0]["status"], "available", "{output}");
    assert_eq!(devices[1]["status"], "available", "{output}");
    assert_eq!(output["fingerprint"].as_str().unwrap().len(), 64);
    assert!(
        !serde_json::to_string(&output)
            .unwrap()
            .contains("fixture-proxy-secret")
    );
}

#[test]
fn inspect_environment_does_not_infer_program_absence_from_enoent() {
    let fixture = support::TemporaryDirectory::new("inspect-environment-missing");
    let output = run_fixture_child("missing", fixture.path());

    assert_eq!(output["git"]["status"], "probe_failed");
    assert_eq!(output["os"]["kernel"]["status"], "probe_failed");
    assert!(
        output["toolchain"]
            .as_array()
            .unwrap()
            .iter()
            .all(|entry| entry["status"] == "probe_failed" && entry["version"].is_null())
    );
    assert!(
        output["connected_devices"]
            .as_array()
            .unwrap()
            .iter()
            .all(|entry| entry["status"] == "probe_failed" && entry["count"] == 0)
    );
}

#[test]
fn inspect_environment_preserves_unknown_for_bad_cwd_interpreter_and_permissions() {
    let fixture = support::TemporaryDirectory::new("environment-ambiguous-spawn");
    write_available_fixture(fixture.path());
    let bad_cwd = run_fixture_child("bad-cwd", fixture.path());
    assert_eq!(bad_cwd["git"]["status"], "probe_failed");
    assert_eq!(bad_cwd["git"]["available"], false);

    write_executable(fixture.path(), "git", "#!/no-such-fixture-interpreter\n");
    let interpreter = run_fixture_child("interpreter", fixture.path());
    assert_eq!(interpreter["git"]["status"], "probe_failed");
    assert!(interpreter["git"]["version"].is_null());
    fs::set_permissions(
        fixture.path().join("git"),
        fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    let denied = run_fixture_child("permission", fixture.path());
    assert_eq!(denied["git"]["status"], "probe_failed");
    assert_eq!(denied["git"]["available"], false);
}

#[test]
fn inspect_environment_distinguishes_version_failure_without_leaking_probe_stderr() {
    let fixture = support::TemporaryDirectory::new("inspect-environment-version-failure");
    write_executable(
        fixture.path(),
        "rustc",
        "#!/bin/sh\nprintf 'proxy-user:fixture-proxy-secret@proxy.invalid\\n' >&2\nexit 9\n",
    );
    let output = run_fixture_child("version-failure", fixture.path());

    let rustc = output["toolchain"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["name"] == "rustc")
        .unwrap();
    assert_eq!(rustc["status"], "version_failed");
    assert!(
        !serde_json::to_string(&output)
            .unwrap()
            .contains("fixture-proxy-secret")
    );
}

#[test]
fn inspect_environment_fingerprint_and_order_are_stable() {
    let first = support::TemporaryDirectory::new("inspect-environment-order-first");
    let second = support::TemporaryDirectory::new("inspect-environment-order-second");
    write_available_fixture_in_order(first.path(), false);
    write_available_fixture_in_order(second.path(), true);
    let first_output = run_fixture_child("order-first", first.path());
    let second_output = run_fixture_child("order-second", second.path());

    assert_eq!(first_output["fingerprint"], second_output["fingerprint"]);
    assert_eq!(first_output["toolchain"], second_output["toolchain"]);
    assert_eq!(
        first_output["connected_devices"],
        second_output["connected_devices"]
    );
}

fn run_fixture_child(fixture: &str, path: &Path) -> Value {
    let mut command = Command::new(std::env::current_exe().expect("current test executable"));
    command
        .args([
            "--exact",
            "inspect_environment_fixture_child",
            "--nocapture",
        ])
        .env(FIXTURE_ENV, fixture)
        .env("PATH", path);
    if fixture == "available" {
        command.env(
            "HTTP_PROXY",
            "http://user:fixture-proxy-secret@proxy.invalid",
        );
    }
    let output = command.output().expect("fixture child should launch");
    assert!(
        output.status.success(),
        "fixture child failed: {}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout
        .lines()
        .find_map(|line| line.strip_prefix("ENV_RESULT="))
        .expect("fixture child should print one environment result");
    serde_json::from_str(line).expect("fixture result should be JSON")
}

fn write_available_fixture(path: &Path) {
    write_available_fixture_in_order(path, false);
}

fn write_available_fixture_in_order(path: &Path, reverse: bool) {
    let scripts = [
        ("uname", "#!/bin/sh\nprintf 'Darwin 24.6.0\\n'\n"),
        ("git", "#!/bin/sh\nprintf 'git version 2.45.1\\n'\n"),
        ("cargo", "#!/bin/sh\nprintf 'cargo 1.93.1 (fixture)\\n'\n"),
        (
            "java",
            "#!/bin/sh\nprintf 'openjdk version \\\"21.0.1\\\"\\n' >&2\n",
        ),
        ("node", "#!/bin/sh\nprintf 'v22.3.0\\n'\n"),
        ("python3", "#!/bin/sh\nprintf 'Python 3.12.4\\n'\n"),
        (
            "rustc",
            "#!/bin/sh\nprintf 'fixture-proxy-secret123\\nrustc 1.93.1 (fixture)\\n' >&2\n",
        ),
        (
            "adb",
            "#!/bin/sh\nif [ \"$1\" = version ]; then printf 'Android Debug Bridge version 1.0.41\\n'; else printf 'List of devices attached\\nserial-online\\tdevice product:fixture\\nserial-offline\\toffline\\n'; fi\n",
        ),
        (
            "xcrun",
            "#!/bin/sh\nif [ \"$1\" = --version ]; then printf 'xcrun version 1.0.0\\n'; else printf '== Devices ==\\niPhone 15 (iOS 17.5) (00008030-001C195E0A10802E) (Booted)\\n== Simulators ==\\niPhone 15 Simulator (17.5) (00112233-4455-6677-8899-aabbccddeeff) (Shutdown)\\n'; fi\n",
        ),
    ];
    let mut indices = (0..scripts.len()).collect::<Vec<_>>();
    if reverse {
        indices.reverse();
    }
    for index in indices {
        let (name, body) = scripts[index];
        write_executable(path, name, body);
    }
}

fn write_executable(path: &Path, name: &str, body: &str) {
    let executable = path.join(name);
    fs::write(&executable, body).expect("fixture executable should be written");
    let mut permissions = fs::metadata(&executable)
        .expect("fixture executable metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(executable, permissions).expect("fixture executable should be runnable");
}
