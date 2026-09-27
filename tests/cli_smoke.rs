#[path = "support/provider_config.rs"]
mod provider_config;
mod support;

use std::process::Command;
use std::{fs, path::Path};

use support::TemporaryDirectory;

#[test]
fn echidna_version_separates_release_version_and_source_base() {
    let short = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .arg("-V")
        .output()
        .unwrap();
    assert!(short.status.success());
    assert_eq!(
        String::from_utf8(short.stdout).unwrap(),
        concat!("echidna ", env!("CARGO_PKG_VERSION"), "\n")
    );
    let long = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(long.status.success());
    assert_eq!(
        String::from_utf8(long.stdout).unwrap(),
        concat!(
            "echidna ",
            env!("CARGO_PKG_VERSION"),
            "\nsource base: ",
            env!("ECHIDNA_SOURCE_REVISION"),
            "\n"
        )
    );
}

#[test]
fn help_lists_m0_commands() {
    let output = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .arg("--help")
        .output()
        .expect("cyber-agent binary should run");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("help should be UTF-8");
    assert!(stdout.contains("init"));
    assert!(stdout.contains("chat"));
    assert!(stdout.contains("doctor"));
    assert!(stdout.contains("inspect"));
    assert!(stdout.contains("session"));
    assert!(stdout.contains("Echidna"));
    assert!(stdout.contains("Usage: echidna"));
}

#[test]
fn m5_session_list_show_export_and_end_use_the_existing_durable_stream() {
    let directory = TemporaryDirectory::new("m5-session-cli");
    let config = write_provider_config(directory.path());
    let initialized = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .current_dir(directory.path())
        .arg("init")
        .output()
        .unwrap();
    assert!(initialized.status.success());
    let chat = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .current_dir(directory.path())
        .args(["chat", "--config", config.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(chat.status.success());
    let session = fs::read_dir(directory.path().join(".cyber-agent/sessions"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .file_name()
        .into_string()
        .unwrap();

    let list = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .current_dir(directory.path())
        .args(["session", "list"])
        .output()
        .unwrap();
    assert!(list.status.success());
    let list: serde_json::Value = serde_json::from_slice(&list.stdout).unwrap();
    assert_eq!(list["sessions"][0]["session_id"], session);
    assert_eq!(list["sessions"][0]["lifecycle"], "active");

    let show = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .current_dir(directory.path())
        .args(["session", "show", &session])
        .output()
        .unwrap();
    assert!(show.status.success());
    let show: serde_json::Value = serde_json::from_slice(&show.stdout).unwrap();
    assert_eq!(show["domain"], "source");
    assert_eq!(show["domain_selection_recorded"], false);

    let export = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .current_dir(directory.path())
        .args(["session", "export", &session, "--format", "json"])
        .output()
        .unwrap();
    assert!(export.status.success());
    let export: serde_json::Value = serde_json::from_slice(&export.stdout).unwrap();
    assert_eq!(export["schema_version"], 1);
    assert_eq!(
        export["authority_boundary"]["vision"],
        "derived model observations; not Finding evidence"
    );

    let export_path = directory.path().join("session-export.md");
    let markdown = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .current_dir(directory.path())
        .args([
            "session",
            "export",
            &session,
            "--format",
            "markdown",
            "--output",
            export_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(markdown.status.success());
    let retained = fs::read_to_string(&export_path).unwrap();
    assert!(retained.contains("Research references remain authoritative only"));
    assert!(retained.contains("Vision (derived non-evidence)"));
    assert!(retained.contains("Latest Compaction (derived continuation)"));

    let rejected_overwrite = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .current_dir(directory.path())
        .args([
            "session",
            "export",
            &session,
            "--format",
            "markdown",
            "--output",
            export_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!rejected_overwrite.status.success());
    assert!(String::from_utf8_lossy(&rejected_overwrite.stderr).contains("cannot create export"));
    assert_eq!(fs::read_to_string(&export_path).unwrap(), retained);

    let ended = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .current_dir(directory.path())
        .args(["session", "end", &session])
        .output()
        .unwrap();
    assert!(ended.status.success());
    let ended: serde_json::Value = serde_json::from_slice(&ended.stdout).unwrap();
    assert_eq!(ended["lifecycle"], "ended");

    let resume = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .current_dir(directory.path())
        .args([
            "chat",
            "--config",
            config.to_str().unwrap(),
            "--resume",
            "latest",
        ])
        .output()
        .unwrap();
    assert!(!resume.status.success());
    assert!(
        String::from_utf8_lossy(&resume.stderr).contains("no strictly replayable Active Session")
    );
}

#[test]
fn inspect_context_in_a_standalone_process_reports_no_active_context() {
    let directory = TemporaryDirectory::new("cli-inspect-context");
    let initialized = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .current_dir(directory.path())
        .arg("init")
        .output()
        .expect("cyber-agent init should run");
    assert!(initialized.status.success());

    let output = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .current_dir(directory.path())
        .args(["inspect", "context"])
        .output()
        .expect("cyber-agent inspect context should run");

    assert!(output.status.success());

    let response: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("context inspection output should be JSON");
    assert_eq!(response["command"], "inspect context");
    assert_eq!(response["status"], "no_active_context");
    assert_eq!(
        response["message"],
        "no active AgentLoop Context is available in this standalone process; RequestAudit persistence is deferred"
    );
    assert_eq!(response.as_object().map(|value| value.len()), Some(3));
}

#[test]
fn doctor_without_configuration_returns_structured_diagnostics() {
    let output = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .arg("doctor")
        .output()
        .expect("cyber-agent binary should run");

    assert!(output.status.success());

    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("doctor output should be JSON");
    assert_eq!(report["command"], "doctor");
    assert_eq!(report["status"], "not_ready");
    assert_eq!(report["diagnostics"].as_array().map(Vec::len), Some(2));
}

fn write_provider_config(directory: &Path) -> std::path::PathBuf {
    let path = directory.join("provider-config.toml");
    fs::write(&path, provider_config::fixture()).unwrap();
    path
}

#[test]
fn doctor_accepts_an_inline_api_key_without_displaying_it() {
    let directory = TemporaryDirectory::new("cli-provider-inline-api-key");
    let secret = "fixture-inline-doctor-secret";
    let config = directory.path().join("provider-config.toml");
    fs::write(
        &config,
        provider_config::fixture().replacen(
            "credential_ref = \"CYBER_AGENT_PRIMARY_API_KEY\"",
            &format!("api_key = \"{secret}\""),
            1,
        ),
    )
    .unwrap();

    let doctor = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .args(["--config", config.to_str().unwrap(), "doctor"])
        .env_remove("CYBER_AGENT_PRIMARY_API_KEY")
        .output()
        .unwrap();
    assert!(doctor.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&doctor.stdout).unwrap()["status"],
        "ready"
    );
    assert!(!String::from_utf8_lossy(&doctor.stdout).contains(secret));
    assert!(!String::from_utf8_lossy(&doctor.stderr).contains(secret));
}

#[test]
fn doctor_and_chat_share_the_explicit_provider_configuration_path_without_a_credential() {
    let directory = TemporaryDirectory::new("cli-provider-config");
    let config = write_provider_config(directory.path());

    let doctor = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .args(["--config", config.to_str().unwrap(), "doctor"])
        .env_remove("CYBER_AGENT_PRIMARY_API_KEY")
        .env("OPENAI_API_KEY", "must-not-be-read")
        .output()
        .expect("configured doctor should run");
    assert!(doctor.status.success());
    let report: serde_json::Value = serde_json::from_slice(&doctor.stdout).unwrap();
    assert_eq!(report["status"], "ready");
    assert_eq!(report["diagnostics"], serde_json::json!([]));
    assert_eq!(report["agent_profile"]["profile_id"], "default");
    assert_eq!(report["agent_profile"]["primary_profile_id"], "primary");
    assert_eq!(report["agent_profile"]["vision_profile_id"], "vision");

    let initialized = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .current_dir(directory.path())
        .arg("init")
        .output()
        .expect("cyber-agent init should run");
    assert!(initialized.status.success());

    let chat = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .current_dir(directory.path())
        .args(["chat", "--config", config.to_str().unwrap()])
        .env_remove("CYBER_AGENT_PRIMARY_API_KEY")
        .env("OPENAI_API_KEY", "must-not-be-read")
        .output()
        .expect("configured shell-only chat should run");
    assert!(chat.status.success());
    let output = String::from_utf8(chat.stdout).unwrap();
    assert!(output.contains("Started session"));
    assert!(output.contains("EOF received"));
    assert!(!output.contains("Question. Observe. Verify."));
    assert!(!output.contains('\u{1b}'));
    assert!(directory.path().join(".cyber-agent/project.toml").is_file());
}

#[test]
fn chat_requires_an_initialized_project_and_does_not_create_one() {
    let directory = TemporaryDirectory::new("cli-chat-project-required");
    let config = write_provider_config(directory.path());

    let chat = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .current_dir(directory.path())
        .args(["chat", "--config", config.to_str().unwrap()])
        .output()
        .expect("cyber-agent chat should run");

    assert!(!chat.status.success());
    assert!(
        String::from_utf8(chat.stderr)
            .unwrap()
            .contains("no Cyber Agent project was found")
    );
    assert!(!directory.path().join(".cyber-agent").exists());
}

#[test]
fn doctor_and_chat_reject_the_same_invalid_provider_configuration() {
    let directory = TemporaryDirectory::new("cli-invalid-provider-config");
    let config = directory.path().join("provider-config.toml");
    fs::write(
        &config,
        provider_config::fixture().replacen(
            "generation = 1",
            "generation = 1\nunknown_setting = true",
            1,
        ),
    )
    .unwrap();

    for command in ["doctor", "chat"] {
        let output = Command::new(env!("CARGO_BIN_EXE_echidna"))
            .args([command, "--config", config.to_str().unwrap()])
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "{command} accepted invalid config"
        );
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .contains("unknown field")
        );
    }
}

#[test]
fn init_creates_a_real_isolated_project_in_the_current_directory() {
    let directory = TemporaryDirectory::new("cli-init");
    let output = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .current_dir(directory.path())
        .arg("init")
        .output()
        .expect("cyber-agent init should run");

    assert!(output.status.success());
    let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["command"], "init");
    assert_eq!(response["status"], "initialized");
    assert!(
        response["project_id"]
            .as_str()
            .unwrap()
            .starts_with("project-")
    );
    assert!(directory.path().join(".cyber-agent/project.toml").is_file());
    assert!(
        directory
            .path()
            .join(".cyber-agent/research.jsonl")
            .is_file()
    );
}
