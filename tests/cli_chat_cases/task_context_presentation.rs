use super::task_context::{ScriptedResponse, definition, spawn_scripted_responses_server};
use super::*;

#[test]
fn selection_reasoning_and_json_are_debug_only_and_labeled() {
    for debug in [false, true] {
        let root = TemporaryDirectory::new("task-selection-presentation");
        initialize_project(root.path());
        let agent = definition(root.path(), None);
        let primary = format!(
            "data: {}\n\ndata: {}\n\n{}",
            json!({"type":"response.reasoning_text.delta","delta":"PRIMARY_REASONING"}),
            json!({"type":"response.output_item.done","item":{"type":"reasoning","content":[{"type":"reasoning_text","text":"PRIMARY_REASONING"}]}}),
            text_response("PRIMARY_ANSWER", "presentation-primary")
        );
        let server = spawn_scripted_responses_server(vec![
            ScriptedResponse::InitialSelection,
            ScriptedResponse::Response(200, text_response("FIRST_ANSWER", "presentation-first")),
            ScriptedResponse::ContinueSelectionWithReasoning,
            ScriptedResponse::Response(200, primary),
        ]);
        let config = write_context_window_config(root.path(), &server.base_url, 272_000);
        let mut args = vec!["--agent", agent.to_str().unwrap()];
        if debug {
            args.push("--debug");
        }
        let output = run_chat(
            root.path(),
            &config,
            "first task\ncontinue it\n/exit\n",
            &args,
        );
        let stdout = String::from_utf8(output.stdout).unwrap();
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(output.status.success(), "stdout={stdout} stderr={stderr}");
        for _ in 0..4 {
            f2a_request_body(&server.requests, "presentation request");
        }
        assert!(server.requests.try_recv().is_err());
        server.join();
        assert!(stdout.contains("PRIMARY_ANSWER"));
        assert!(!stdout.contains("PRIMARY_REASONING"));
        assert!(stderr.contains("[reasoning primary #"));
        assert!(stderr.contains("PRIMARY_REASONING"));
        for marker in [
            "SELECTOR_REASONING_DELTA",
            "SELECTOR_REASONING_ITEM",
            "required_context_refs",
        ] {
            assert!(!stdout.contains(marker));
            assert_eq!(stderr.contains(marker), debug, "{marker}: {stderr}");
        }
        if debug {
            assert!(stderr.contains("[debug sending task selection #"));
            assert!(stderr.contains("[reasoning task selection #"));
            assert!(stderr.contains("[debug task selection output · pending]"));
        }
    }
}

#[test]
fn incomplete_selector_payload_is_debug_only_but_failure_reason_remains_visible() {
    for debug in [false, true] {
        let root = TemporaryDirectory::new("task-selection-incomplete-presentation");
        initialize_project(root.path());
        let agent = definition(root.path(), None);
        let event = json!({"type":"response.incomplete", "response":{
            "status":"incomplete", "incomplete_details":{"reason":"max_output_tokens"},
            "output":[{"type":"message", "content":[{"type":"output_text", "text":"SELECTOR_INCOMPLETE_PRIVATE_MARKER"}]}]
        }});
        let server = spawn_scripted_responses_server(vec![ScriptedResponse::Response(
            200,
            format!("data: {event}\n\n"),
        )]);
        let config = write_context_window_config(root.path(), &server.base_url, 272_000);
        let mut args = vec!["--agent", agent.to_str().unwrap()];
        if debug {
            args.push("--debug");
        }
        let output = run_chat(root.path(), &config, "first task\n/exit\n", &args);
        let stdout = String::from_utf8(output.stdout).unwrap();
        let stderr = String::from_utf8(output.stderr).unwrap();
        super::task_context::selector_candidates(&f2a_request_body(
            &server.requests,
            "incomplete selector",
        ));
        assert!(server.requests.try_recv().is_err());
        server.join();
        assert!(!stdout.contains("SELECTOR_INCOMPLETE_PRIVATE_MARKER"));
        assert_eq!(
            stderr.contains("SELECTOR_INCOMPLETE_PRIVATE_MARKER"),
            debug,
            "{stderr}"
        );
        assert!(format!("{stdout}\n{stderr}").contains("max_output_tokens"));
        let state = reduced_session(
            &Project::discover(root.path()).unwrap(),
            &session_id(root.path()),
        );
        assert_eq!(state.execution_receipt_count(), 0);
    }
}
