use super::*;
use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

fn stop_held_pipe_descendant(project: &TestProject) {
    let pid_path = project.root.join("held-pipe.pid");
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while !pid_path.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let pid = fs::read_to_string(&pid_path)
        .expect("held-pipe helper must record its descendant PID")
        .trim()
        .to_owned();
    let _ = std::process::Command::new("/bin/kill")
        .args(["-TERM", &pid])
        .output();
    while std::time::Instant::now() < deadline {
        let running = std::process::Command::new("/bin/kill")
            .args(["-0", &pid])
            .output()
            .is_ok_and(|output| output.status.success());
        if !running {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("held-pipe descendant {pid} did not stop after TERM");
}

#[tokio::test]
async fn direct_exit_with_held_stdout_stderr_or_both_pipes_keeps_real_exit_and_prefixes() {
    for (stream, exit_code, child_redirection) in [
        ("stdout", 0, "2>/dev/null"),
        ("stdout", 7, "2>/dev/null"),
        ("stderr", 0, ">/dev/null"),
        ("stderr", 7, ">/dev/null"),
        ("both", 0, ""),
        ("both", 7, ""),
    ] {
        let project = TestProject::new(&format!("direct-command-held-{stream}-{exit_code}"));
        project.write(
            "hold-pipes.sh",
            format!(
                r#"(
  : > held-pipe-ready
  exec /bin/sleep 30
) {child_redirection} &
descendant=$!
printf '%s\n' "$descendant" > held-pipe.pid
while [ ! -f held-pipe-ready ]; do
  /bin/sleep 0.01
done
case "{stream}" in
  stdout) printf PREFIX_STDOUT ;;
  stderr) printf PREFIX_STDERR >&2 ;;
  both) printf PREFIX_STDOUT; printf PREFIX_STDERR >&2 ;;
esac
exit {exit_code}
"#,
            ),
        );
        let tools = ToolRegistry::for_project(&project.root);
        let started = std::time::Instant::now();
        let outcome = tokio::time::timeout(
            Duration::from_millis(750),
            tools.execute(plan(
                &tools,
                "held-pipes",
                "run_command",
                json!({"argv":["/bin/sh", "hold-pipes.sh"]}),
            )),
        )
        .await;
        stop_held_pipe_descendant(&project);

        let elapsed = started.elapsed();
        let outcome = outcome
            .expect("direct child exit must not wait for its descendant pipe holders")
            .expect("run_command completes");
        assert!(
            elapsed >= Duration::from_millis(80),
            "{stream} exit {exit_code} returned before the residual drain grace: {elapsed:?}"
        );
        assert_eq!(outcome.result.status, ToolResultStatus::Failed);
        assert_eq!(outcome.result.output["exit_code"], exit_code);
        assert_eq!(
            outcome.result.output["stdout"],
            if matches!(stream, "stdout" | "both") {
                "PREFIX_STDOUT"
            } else {
                ""
            }
        );
        assert_eq!(
            outcome.result.output["stderr"],
            if matches!(stream, "stderr" | "both") {
                "PREFIX_STDERR"
            } else {
                ""
            }
        );
        let expected_stage = if stream == "stderr" {
            "process_stage=read_stderr"
        } else {
            "process_stage=read_stdout"
        };
        assert!(
            outcome.result.output["error"]
                .as_str()
                .is_some_and(|error| error.contains(expected_stage))
        );
    }
}

#[tokio::test]
async fn direct_child_keeps_draining_without_a_post_exit_grace_deadline() {
    let project = TestProject::new("direct-command-live-child");
    let tools = ToolRegistry::for_project(&project.root);
    let started = std::time::Instant::now();
    let outcome = tools
        .execute(plan(
            &tools,
            "live-child",
            "run_command",
            json!({"argv":["/bin/sh", "-c", "printf EARLY; /bin/sleep 0.2; printf LATE"]}),
        ))
        .await
        .expect("run_command completes");
    assert!(
        started.elapsed() >= Duration::from_millis(180),
        "the direct child must be allowed to outlive the post-exit reader grace"
    );
    assert_eq!(outcome.result.status, ToolResultStatus::Succeeded);
    assert_eq!(outcome.result.output["stdout"], "EARLYLATE");
    assert!(outcome.result.output.get("error").is_none());
}

#[tokio::test]
async fn ordinary_high_volume_output_drains_completely_after_the_direct_child_exits() {
    let project = TestProject::new("direct-command-high-volume");
    let tools = ToolRegistry::for_project(&project.root);
    let outcome = tokio::time::timeout(
        Duration::from_secs(5),
        tools.execute(plan(
            &tools,
            "high-volume",
            "run_command",
            json!({"argv":["/bin/sh", "-c", "dd if=/dev/zero bs=1024 count=512 2>/dev/null; printf COMPLETE_TAIL"]}),
        )),
    )
    .await
    .expect("ordinary high-volume child must not block the command drain")
    .expect("run_command completes");
    assert_eq!(outcome.result.status, ToolResultStatus::Succeeded);
    assert!(
        outcome.result.output["stdout"]
            .as_str()
            .is_some_and(|stdout| stdout.ends_with("COMPLETE_TAIL"))
    );
    assert_eq!(outcome.result.output["truncated"], true);
}

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
            .await
            .expect("Tool execution completes");
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
        .await
        .expect("Tool execution completes");
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
                    .await
                    .expect("Tool execution completes");
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
                .await
                .expect("Tool execution completes");
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
        .await
        .expect("Tool execution completes");
    assert_eq!(listed.result.output["path"], ".");
}
