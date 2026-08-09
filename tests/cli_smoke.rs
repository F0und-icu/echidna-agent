use std::process::Command;

#[test]
fn help_lists_m0_commands() {
    let output = Command::new(env!("CARGO_BIN_EXE_cyber-agent"))
        .arg("--help")
        .output()
        .expect("cyber-agent binary should run");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("help should be UTF-8");
    assert!(stdout.contains("init"));
    assert!(stdout.contains("chat"));
    assert!(stdout.contains("doctor"));
}

#[test]
fn doctor_without_configuration_returns_structured_diagnostics() {
    let output = Command::new(env!("CARGO_BIN_EXE_cyber-agent"))
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
