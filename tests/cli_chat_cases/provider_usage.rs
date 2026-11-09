//! Actual completed-sample usage over the offline CLI request/audit path.
use super::task_context::{
    ScriptedResponse::{InitialSelection, Response},
    selector_candidates, spawn_scripted_responses_server,
};
use super::*;

fn reported_usage(input: u64, output: u64) -> Value {
    json!({
        "input_tokens":input,"output_tokens":output,
        "input_tokens_details":{"cached_tokens":input / 2},
        "output_tokens_details":{"reasoning_tokens":output / 2}
    })
}

fn with_usage(body: String, usage: Value) -> String {
    let mut completed = 0;
    let response = body
        .split("\n\n")
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut event: Value =
                serde_json::from_str(part.strip_prefix("data: ").unwrap()).unwrap();
            if event["type"] == "response.completed" {
                event["response"]["usage"] = usage.clone();
                completed += 1;
            }
            format!("data: {event}\n\n")
        })
        .collect();
    assert_eq!(completed, 1);
    response
}

fn usage_events(events: &[Value]) -> Vec<&Value> {
    events
        .iter()
        .filter(|event| event["event_type"] == "model.usage_recorded")
        .collect()
}

fn assert_bound_usage(events: &[Value], usage: &Value) -> Value {
    let audit = events
        .iter()
        .find(|event| {
            event["event_id"] == usage["payload"]["request_audit_event_id"]
                && event["event_type"] == "session.request_audit_recorded"
        })
        .unwrap();
    assert_eq!(
        usage["payload"]["provider_session"],
        audit["payload"]["provider_session"]
    );
    assert!(audit["sequence"].as_u64().unwrap() < usage["sequence"].as_u64().unwrap());
    audit.clone()
}

#[test]
fn primary_usage_preserves_each_completed_samples_totals_subsets_and_unknown_details() {
    let root = TemporaryDirectory::new("provider-usage-primary");
    initialize_project(root.path());
    fs::write(root.path().join("note.txt"), "fixture contents").unwrap();
    let server = spawn_scripted_responses_server(vec![
        InitialSelection,
        Response(
            200,
            with_usage(
                m4_long_flow_tool_response(
                    "usage-read",
                    "read_file",
                    json!({"path":"note.txt"}),
                    "usage-tool",
                ),
                reported_usage(100, 20),
            ),
        ),
        Response(
            200,
            with_usage(
                text_response("Done.", "usage-final"),
                json!({"input_tokens":200,"output_tokens":40}),
            ),
        ),
    ]);
    let config = write_config(root.path(), &server.base_url);
    let output = run_chat(
        root.path(),
        &config,
        "Read note.txt and report.\n/exit\n",
        &[],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let requests = server.requests.try_iter().collect::<Vec<_>>();
    assert_eq!(requests.len(), 3);
    selector_candidates(&request_body(&requests[0]));
    server.join();
    let events = session_events(root.path(), &session_id(root.path()));
    let usage = usage_events(&events);
    assert_eq!(usage.len(), 2);
    for event in &usage {
        let audit = assert_bound_usage(&events, event);
        assert!(audit["payload"].get("improvement_scope").is_none());
        assert_eq!(event["payload"]["provider_session"]["role"], "primary");
    }
    assert_ne!(
        usage[0]["payload"]["request_audit_event_id"],
        usage[1]["payload"]["request_audit_event_id"]
    );
    assert_eq!(
        usage[0]["payload"]["usage"],
        json!({"input_tokens":100,"output_tokens":20,"cached_input_tokens":50,"reasoning_output_tokens":10})
    );
    assert_eq!(
        usage[1]["payload"]["usage"],
        json!({"input_tokens":200,"output_tokens":40})
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("input=100 output=20 cached_input=50 reasoning_output=10"));
    assert!(stderr.contains("input=200 output=40 cached_input=unknown reasoning_output=unknown"));
    let state = reduced_session(
        &Project::discover(root.path()).unwrap(),
        &session_id(root.path()),
    );
    assert_eq!(state.execution_receipt_count(), 1);
    assert!(
        !serde_json::to_string(state.history().items())
            .unwrap()
            .contains("cached_input_tokens")
    );
}

#[test]
fn rejected_tool_sample_never_persists_its_reported_usage_or_executes_its_call() {
    let root = TemporaryDirectory::new("provider-usage-rejected");
    initialize_project(root.path());
    let invalid = [
        json!({"type":"response.output_item.done","item":{
            "type":"function_call","call_id":"rejected-write","name":"write_file","arguments":"{"}}),
        json!({"type":"response.completed","response":{"id":"rejected","usage":reported_usage(900,90)}}),
    ].iter().map(|event| format!("data: {event}\n\n")).collect();
    let server = spawn_scripted_responses_server(vec![
        InitialSelection,
        Response(200, invalid),
        Response(
            200,
            with_usage(
                text_response("Corrected answer.", "accepted-correction"),
                reported_usage(100, 20),
            ),
        ),
    ]);
    let config = write_config(root.path(), &server.base_url);
    let output = run_chat(root.path(), &config, "Report once.\n/exit\n", &[]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let requests = server.requests.try_iter().collect::<Vec<_>>();
    assert_eq!(requests.len(), 3);
    selector_candidates(&request_body(&requests[0]));
    server.join();
    let events = session_events(root.path(), &session_id(root.path()));
    let rejected = events
        .iter()
        .find(|event| event["event_type"] == "model.sample_rejected")
        .unwrap();
    let usage = usage_events(&events);
    assert_eq!(usage.len(), 1);
    assert_bound_usage(&events, usage[0]);
    assert_ne!(
        usage[0]["payload"]["request_audit_event_id"],
        rejected["payload"]["request_audit_event_id"]
    );
    assert_eq!(usage[0]["payload"]["usage"]["input_tokens"], 100);
    let state = reduced_session(
        &Project::discover(root.path()).unwrap(),
        &session_id(root.path()),
    );
    assert_eq!(state.execution_receipt_count(), 0);
    assert!(
        !serde_json::to_string(state.history().items())
            .unwrap()
            .contains("rejected-write")
    );
}

#[test]
fn rsi_diagnosis_usage_keeps_its_scope_and_exact_audit_separate_from_the_parent() {
    let root = TemporaryDirectory::new("provider-usage-rsi");
    initialize_project(root.path());
    let definition = super::agent_evolution::improvement_definition(root.path());
    let server = spawn_scripted_responses_server(vec![
        InitialSelection,
        Response(
            200,
            with_usage(
                text_response("Task completed.", "parent-completed"),
                reported_usage(300, 60),
            ),
        ),
        Response(
            200,
            with_usage(
                text_response(
                    &json!({"decision":"no_change","reason":"No supported reusable change."})
                        .to_string(),
                    "diagnosis-completed",
                ),
                reported_usage(10, 2),
            ),
        ),
    ]);
    let config = write_config(root.path(), &server.base_url);
    let output = run_chat(
        root.path(),
        &config,
        "[rsi-feedback] Keep future replies concise.\n/exit\n",
        &["--agent", definition.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let requests = server.requests.try_iter().collect::<Vec<_>>();
    assert_eq!(requests.len(), 3);
    selector_candidates(&request_body(&requests[0]));
    server.join();
    let events = session_events(root.path(), &session_id(root.path()));
    let usage = usage_events(&events);
    assert_eq!(usage.len(), 2);
    let parent = assert_bound_usage(&events, usage[0]);
    let scoped = assert_bound_usage(&events, usage[1]);
    assert!(parent["payload"].get("improvement_scope").is_none());
    assert_eq!(
        scoped["payload"]["improvement_scope"],
        usage[1]["payload"]["provider_session"]
    );
    assert_ne!(
        usage[0]["payload"]["provider_session"],
        usage[1]["payload"]["provider_session"]
    );
    assert_eq!(usage[0]["payload"]["usage"]["input_tokens"], 300);
    assert_eq!(usage[1]["payload"]["usage"]["input_tokens"], 10);
    let accepted_sample = events
        .iter()
        .find(|event| {
            event["event_type"] == "session.rsi_activity_recorded"
                && event["payload"]["fact"]["kind"] == "sample"
        })
        .unwrap();
    assert_eq!(
        accepted_sample["payload"]["fact"]["audit_event_id"],
        usage[1]["payload"]["request_audit_event_id"]
    );
    assert!(accepted_sample["sequence"].as_u64().unwrap() < usage[1]["sequence"].as_u64().unwrap());
    reduced_session(
        &Project::discover(root.path()).unwrap(),
        &session_id(root.path()),
    );
}

#[test]
fn vision_usage_follows_the_durable_observation_and_keeps_its_own_audit() {
    let root = TemporaryDirectory::new("provider-usage-vision");
    initialize_project(root.path());
    let project = Project::discover(root.path()).unwrap();
    append_research_snapshot(&project, "usage-vision");
    fs::write(root.path().join("screen.png"), png_bytes(16, 12)).unwrap();
    let vision = spawn_responses_server(vec![(
        200,
        with_usage(
            vision_response("Fixture observation", "vision-completed"),
            reported_usage(50, 10),
        ),
    )]);
    let primary = spawn_scripted_responses_server(vec![
        InitialSelection,
        Response(
            200,
            with_usage(
                text_response("Observed.", "primary-completed"),
                reported_usage(200, 40),
            ),
        ),
    ]);
    let config = write_dual_config(root.path(), &primary.base_url, &vision.base_url);
    let output = run_chat(
        root.path(),
        &config,
        "/image screen.png\nInspect the image.\n/exit\n",
        &[],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(vision.requests.try_iter().count(), 1);
    selector_candidates(&f2a_request_body(
        &primary.requests,
        "Vision usage source selection",
    ));
    assert_eq!(primary.requests.try_iter().count(), 1);
    vision.join();
    primary.join();
    let events = session_events(root.path(), &session_id(root.path()));
    let usage = usage_events(&events);
    assert_eq!(usage.len(), 2);
    assert_bound_usage(&events, usage[0]);
    assert_bound_usage(&events, usage[1]);
    assert_eq!(usage[0]["payload"]["provider_session"]["role"], "vision");
    assert_eq!(usage[1]["payload"]["provider_session"]["role"], "primary");
    let observation = events
        .iter()
        .find(|event| event["event_type"] == "vision.observation_recorded")
        .unwrap();
    assert_eq!(
        observation["payload"]["request_audit_event_id"],
        usage[0]["payload"]["request_audit_event_id"]
    );
    assert!(observation["sequence"].as_u64().unwrap() < usage[0]["sequence"].as_u64().unwrap());
    assert_eq!(usage[0]["payload"]["usage"]["input_tokens"], 50);
    assert_eq!(usage[1]["payload"]["usage"]["input_tokens"], 200);
    reduced_session(&project, &session_id(root.path()));
}
