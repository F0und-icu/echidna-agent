use super::task_context::{
    ScriptedResponse::{InitialSelection, Response},
    selector_candidates, spawn_scripted_responses_server,
};
use super::*;

#[test]
fn memory_disabled_still_reports_independent_rsi_without_writes_or_extra_requests() {
    let root = TemporaryDirectory::new("memory-disabled-rsi-visibility");
    initialize_project(root.path());
    let definition = root.path().join("definition");
    fs::create_dir(&definition).unwrap();
    let source =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/agents/self-improving");
    for name in [
        "agent.toml",
        "persona.md",
        "work.md",
        "method.md",
        "fixed.md",
    ] {
        fs::copy(source.join(name), definition.join(name)).unwrap();
    }
    let manifest = definition.join("agent.toml");
    let content = fs::read_to_string(&manifest)
        .unwrap()
        .replace("memory = \"agent\"", "memory = \"none\"")
        .replace("memory_read = \"direct\"", "memory_read = \"hidden\"")
        .replace("memory_write = \"direct\"", "memory_write = \"hidden\"");
    fs::write(manifest, content).unwrap();
    let server = spawn_scripted_responses_server(vec![
        InitialSelection,
        Response(
            200,
            text_response("ORDINARY_TASK_DONE", "memory-rsi-visibility"),
        ),
    ]);
    let config = write_config(root.path(), &server.base_url);
    let output = run_chat(
        root.path(),
        &config,
        "/memory\nhello\n/memory\n/exit\n",
        &["--agent", definition.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let statuses = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let value: Value = serde_json::from_str(&line[line.find('{')?..]).ok()?;
            value.get("automatic_extraction").map(|_| value.clone())
        })
        .collect::<Vec<_>>();
    assert_eq!(statuses.len(), 2);
    for status in &statuses {
        assert_eq!(status["memory"], "disabled");
        assert_eq!(status["automatic_extraction"], false);
        assert_eq!(status["rsi"]["enabled"], true);
        assert_eq!(status["rsi"]["improvement_enabled"], true);
        assert_eq!(status["rsi"]["experience_count"], 0);
        assert_eq!(status["rsi"]["model_adoption"], "unobserved");
        assert!(status.get("core_path").is_none());
    }
    assert_eq!(
        statuses[0]["rsi"]["last_context"]["status"],
        "no_compiled_plan"
    );
    assert_eq!(
        statuses[1]["rsi"]["last_context"]["status"],
        "not_selected_in_last_compiled_plan"
    );
    assert!(!root.path().join(".echidna/memory").exists());
    let project = Project::discover(root.path()).unwrap();
    let state = reduced_session(&project, &session_id(root.path()));
    assert_eq!(state.request_audits().len(), 2);
    assert_eq!(state.execution_receipt_count(), 0);
    assert_eq!(state.history().items().len(), 2);
    let selection = f2a_request_body(&server.requests, "memory visibility initial selection");
    assert_eq!(selector_candidates(&selection)["input"]["text"], "hello");
    assert!(
        server
            .requests
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .contains("hello")
    );
    assert!(server.requests.try_recv().is_err());
    f2a_join_server(server, "memory-disabled RSI visibility");
}
