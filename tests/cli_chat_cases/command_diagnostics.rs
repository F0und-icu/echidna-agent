use super::task_context::{
    ScriptedResponse::{InitialSelection, Response},
    selector_candidates, spawn_scripted_responses_server,
};
use super::*;

#[test]
fn command_failure_diagnostics_survive_child_checkpoint_history_and_provider_delivery() {
    let root = TemporaryDirectory::new("command-diagnostics-chat");
    let harness = TemporaryDirectory::new("command-diagnostics-provider");
    initialize_project(root.path());
    let server = spawn_scripted_responses_server(vec![
        InitialSelection,
        Response(
            200,
            m4_long_flow_tool_response(
                "diagnostics",
                "run_commands",
                json!({
                    "mode":"serial", "commands":[
                        {"argv":["cyber-agent-no-such-program"]},
                        {"argv":["/bin/pwd"],"cwd":"missing-directory"},
                        {"argv":["/bin/sh","-c","printf x >> executed; exit 7"]},
                        {"argv":["/bin/pwd"]}
                    ]
                }),
                "diagnostics-response",
            ),
        ),
        Response(
            200,
            text_response("observations retained", "diagnostics-done"),
        ),
    ]);
    let config = write_config(harness.path(), &server.base_url);
    let output = run_chat(
        root.path(),
        &config,
        "inspect command observations\n/exit\n",
        &[],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let selector = f2a_request_body(&server.requests, "diagnostics selection");
    selector_candidates(&selector);
    let _ = f2a_request_body(&server.requests, "diagnostics initial");
    let followup = f2a_request_body(&server.requests, "diagnostics results");
    let delivered = followup["input"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["type"] == "function_call_output")
        .unwrap();
    let delivered: Value = serde_json::from_str(delivered["output"].as_str().unwrap()).unwrap();
    f2a_join_server(server, "diagnostics");
    assert_eq!(fs::read(root.path().join("executed")).unwrap(), b"x");

    let events = session_events(root.path(), &session_id(root.path()));
    let children = events
        .iter()
        .filter(|event| event["event_type"] == "tool.subcommand_checkpointed")
        .collect::<Vec<_>>();
    assert_eq!(children.len(), 4);
    for (index, child) in children[..2].iter().enumerate() {
        assert_eq!(child["payload"]["status"], "failed");
        let error = child["payload"]["result"]["error"].as_str().unwrap();
        assert!(error.contains("process_stage=spawn"), "{error}");
        assert!(error.contains("effective_cwd="), "{error}");
        assert!(error.contains("os error"), "{error}");
        assert_eq!(
            delivered["output"]["commands"][index]["result"]["error"],
            error
        );
    }
    assert_eq!(children[2]["payload"]["result"]["exit_code"], 7);
    assert!(children[2]["payload"]["result"].get("error").is_none());
    assert_eq!(children[3]["payload"]["result"]["cwd"], ".");
    assert_eq!(
        events
            .iter()
            .filter(|event| event["event_type"] == "tool.execution_outcome_recorded")
            .count(),
        1
    );
    let projection = echidna::app::session::SessionProjection::replay(
        events
            .into_iter()
            .map(|event| serde_json::from_value(event).unwrap()),
    )
    .unwrap();
    assert!(
        projection
            .state()
            .history()
            .items()
            .iter()
            .any(|item| matches!(item, echidna::domain::model::ContentItem::ToolResult { .. }))
    );
}
