use super::*;
use std::os::unix::fs::PermissionsExt;

#[tokio::test]
async fn spawn_diagnostics_preserve_program_cwd_stage_and_os_error() {
    let project = TestProject::new("command-diagnostics");
    project.write(
        "missing-interpreter",
        "#!/cyber-agent-no-such-interpreter\n",
    );
    fs::set_permissions(
        project.root.join("missing-interpreter"),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    project.write("not-executable", "#!/bin/sh\nexit 0\n");
    fs::set_permissions(
        project.root.join("not-executable"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let tools = ToolRegistry::for_project(&project.root);
    let cases = [
        (
            "missing-program",
            "cyber-agent-no-such-program".to_owned(),
            ".",
        ),
        ("missing-cwd", "/bin/sh".to_owned(), "no-such-cwd"),
        (
            "missing-interpreter",
            project
                .root
                .join("missing-interpreter")
                .display()
                .to_string(),
            ".",
        ),
        (
            "permission",
            project.root.join("not-executable").display().to_string(),
            ".",
        ),
    ];
    for (id, program, cwd) in cases {
        let outcome = tools
            .execute(plan(
                &tools,
                id,
                "run_command",
                json!({"argv":[program],"cwd":cwd}),
            ))
            .await;
        assert_eq!(outcome.result.status, ToolResultStatus::Failed);
        assert_eq!(outcome.receipt.status, ToolResultStatus::Failed);
        assert!(outcome.result.output["exit_code"].is_null());
        let error = outcome.result.output["error"].as_str().unwrap();
        assert!(error.contains(&format!("program={program:?}")), "{error}");
        assert!(error.contains("process_stage=spawn"), "{error}");
        assert!(
            error.contains(&format!("effective_cwd={:?}", project.root.join(cwd))),
            "{error}"
        );
        assert!(
            error.contains("os error"),
            "original OS error missing: {error}"
        );
        // The same errno can represent different missing path components.
        assert!(!error.contains("program is missing"));
        assert_eq!(outcome.result.output["failure"]["stage"], "execution");
    }
}

#[tokio::test]
async fn ordinary_nonzero_exit_is_not_a_process_start_failure() {
    let project = TestProject::new("nonzero-diagnostics");
    let tools = ToolRegistry::for_project(&project.root);
    let outcome = tools
        .execute(plan(
            &tools,
            "nonzero",
            "run_command",
            json!({
                "argv":["/bin/sh","-c","printf x >> executed; printf diagnostic >&2; exit 7"]
            }),
        ))
        .await;
    assert_eq!(outcome.result.status, ToolResultStatus::Failed);
    assert_eq!(outcome.result.output["exit_code"], 7);
    assert_eq!(outcome.result.output["stderr"], "diagnostic");
    assert!(outcome.result.output.get("error").is_none());
    assert_eq!(fs::read(project.root.join("executed")).unwrap(), b"x");
}

#[tokio::test]
async fn command_cwd_defaults_and_explicit_paths_match_in_single_serial_and_parallel() {
    let project = TestProject::new("cwd-observations");
    fs::create_dir(project.root.join("subdir")).unwrap();
    let tools = ToolRegistry::for_project(&project.root);
    let arguments = [
        json!({"argv":["/bin/pwd"],"resources":[{"kind":"output","path":"a"}]}),
        json!({"argv":["/bin/pwd"],"cwd":null,"resources":[{"kind":"output","path":"b"}]}),
        json!({"argv":["/bin/pwd"],"cwd":".","resources":[{"kind":"output","path":"c"}]}),
        json!({"argv":["/bin/pwd"],"cwd":"subdir","resources":[{"kind":"output","path":"d"}]}),
    ];
    for mode in ["single", "serial", "parallel"] {
        let outputs = if mode == "single" {
            let mut outputs = Vec::new();
            for (index, arguments) in arguments.iter().enumerate() {
                let outcome = tools
                    .execute(plan(
                        &tools,
                        &format!("cwd-{index}"),
                        "run_command",
                        arguments.clone(),
                    ))
                    .await;
                assert_eq!(outcome.result.status, ToolResultStatus::Succeeded);
                outputs.push(outcome.result.output);
            }
            outputs
        } else {
            let outcome = tools
                .execute(plan(
                    &tools,
                    mode,
                    "run_commands",
                    json!({"mode":mode,"commands":arguments}),
                ))
                .await;
            assert_eq!(outcome.result.output["partial_failure"], false);
            outcome.result.output["commands"]
                .as_array()
                .unwrap()
                .iter()
                .map(|child| child["result"].clone())
                .collect()
        };
        for (index, output) in outputs.iter().enumerate() {
            let expected_cwd = if index == 3 { "subdir" } else { "." };
            assert_eq!(output["cwd"], expected_cwd, "mode={mode}");
            assert_eq!(
                output["stdout"].as_str().unwrap().trim(),
                project
                    .root
                    .join(expected_cwd)
                    .canonicalize()
                    .unwrap()
                    .to_str()
                    .unwrap()
            );
        }
    }
    let listed = tools
        .execute(plan(&tools, "list-root", "list_files", json!({"path":"."})))
        .await;
    assert_eq!(listed.result.output["path"], ".");
}
