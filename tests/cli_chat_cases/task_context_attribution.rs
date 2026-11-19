//! Multi-objective attribution uses synthetic real Tool outcomes.
use super::task_context::{definition, selector_candidates};
use super::task_context_paging::{delivered_result, paged_server};
use super::*;

const MULTI: &str = "OBJECTIVE A: inspect alpha; OBJECTIVE B: inspect beta";
const GAP: &str = "unresolved task attribution";

fn task(candidates: &Value, goal: &str) -> String {
    candidates["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|candidate| candidate["goal"] == goal)
        .unwrap_or_else(|| panic!("missing {goal}: {}", candidates["tasks"]))["ref"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn decision(focus: &str, selected: Vec<Value>) -> String {
    text_response(
        &json!({"relation":"return", "focus":{"kind":"existing", "task_ref":focus},
            "related_task_refs":[], "required_context_refs":selected, "unresolved_references":[]})
        .to_string(),
        "attribution-selection",
    )
}

fn progress_call(name: &str, task_ref: &str) -> String {
    m4_long_flow_tool_response(
        name,
        "context_read",
        json!({"operation":"progress", "task_ref":task_ref, "max_bytes":16384}),
        &format!("response-{name}"),
    )
}

fn progress(result: &Value) -> Value {
    assert_eq!(result["status"], "succeeded", "{result}");
    assert_eq!(result["output"]["status"], "found", "{result}");
    serde_json::from_str(result["output"]["entries"][0]["text"].as_str().unwrap()).unwrap()
}

#[test]
fn multi_objective_results_remain_unattributed_until_exact_source_selection() {
    let root = TemporaryDirectory::new("task-progress-multi-attribution");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let mut alpha = String::new();
    let mut beta = String::new();
    let mut source_refs = Vec::new();
    let server = paged_server(move |step, wire| {
        if step == 0 {
            return (
                super::task_context::initial_selection_response(
                    wire,
                    "attribution-initial-selection",
                ),
                false,
            );
        }
        let step = step - 1;
        match step {
            0 => (text_response("Seed complete.", "attribution-seed"), false),
            1 => {
                let candidates = selector_candidates(wire);
                assert_eq!(candidates["input"]["text"], MULTI);
                let split = MULTI.find(';').unwrap();
                let selection = json!({"relation":"new", "focus":{"kind":"new_from_input","input_ref":candidates["input"]["ref"]},
                "related_task_refs":[], "required_context_refs":[], "unresolved_references":[],
                "new_objectives":[
                    {"start_byte":0,"end_byte":split,"description":"OBJECTIVE A"},
                    {"start_byte":split + 2,"end_byte":MULTI.len(),"description":"OBJECTIVE B"}
                ]});
                (
                    text_response(&selection.to_string(), "multi-selection"),
                    false,
                )
            }
            2 => {
                let calls = [
                    (
                        "independent-alpha",
                        "run_command",
                        json!({"argv":["/bin/sh","-c","printf 'a\\n' >> attribution-count.txt; printf 'A_ONLY_VALUE'"]}),
                    ),
                    (
                        "independent-beta",
                        "run_command",
                        json!({"argv":["/bin/sh","-c","printf 'b\\n' >> attribution-count.txt; printf 'B_ONLY_VALUE'"]}),
                    ),
                    (
                        "explicit-shared",
                        "run_command",
                        json!({"argv":["/bin/sh","-c","printf 's\\n' >> attribution-count.txt; printf 'SHARED_ONLY_VALUE'"]}),
                    ),
                ];
                (tool_calls_response(&calls, "multi-real-batch"), false)
            }
            3 => (
                text_response("Both observations recorded.", "multi-finished"),
                false,
            ),
            4 => {
                let candidates = selector_candidates(wire);
                alpha = task(&candidates, "OBJECTIVE A");
                beta = task(&candidates, "OBJECTIVE B");
                (decision(&alpha, Vec::new()), false)
            }
            5 => {
                assert!(wire["previous_response_id"].is_null());
                assert!(!wire.to_string().contains("A_ONLY_VALUE"));
                assert!(!wire.to_string().contains("B_ONLY_VALUE"));
                assert!(wire.to_string().contains(GAP));
                (progress_call("alpha-progress", &alpha), false)
            }
            6 => {
                let value = progress(&delivered_result(wire, "alpha-progress"));
                assert!(!value.to_string().contains("A_ONLY_VALUE"));
                assert!(!value.to_string().contains("B_ONLY_VALUE"));
                let attribution = &value["unresolved_attribution"];
                assert!(attribution["gap"].as_str().unwrap().contains(GAP));
                source_refs = attribution["source_event_refs"].as_array().unwrap().clone();
                assert_eq!(source_refs.len(), 3);
                let calls = source_refs
                    .iter()
                    .enumerate()
                    .map(|(i, reference)| {
                        (
                            format!("read-original-{i}"),
                            json!({"operation":"read", "reference":reference, "max_bytes":4096}),
                        )
                    })
                    .collect::<Vec<_>>();
                let calls = calls
                    .iter()
                    .map(|(name, args)| (name.as_str(), "context_read", args.clone()))
                    .collect::<Vec<_>>();
                (tool_calls_response(&calls, "read-exact-results"), false)
            }
            7 => {
                let mut original = String::new();
                for i in 0..source_refs.len() {
                    let result = delivered_result(wire, &format!("read-original-{i}"));
                    assert_eq!(result["output"]["status"], "found", "{result}");
                    original.push_str(result["output"]["entries"][0]["text"].as_str().unwrap());
                }
                for marker in ["A_ONLY_VALUE", "B_ONLY_VALUE", "SHARED_ONLY_VALUE"] {
                    assert!(
                        original.contains(marker),
                        "{marker} not in original sources"
                    );
                }
                (text_response("Alpha inspected.", "alpha-done"), false)
            }
            8 => {
                let candidates = selector_candidates(wire);
                assert_eq!(task(&candidates, "OBJECTIVE B"), beta);
                (decision(&beta, Vec::new()), false)
            }
            9 => {
                assert!(wire["previous_response_id"].is_null());
                assert!(!wire.to_string().contains("A_ONLY_VALUE"));
                assert!(!wire.to_string().contains("B_ONLY_VALUE"));
                assert!(wire.to_string().contains(GAP));
                (progress_call("beta-progress", &beta), false)
            }
            10 => {
                let value = progress(&delivered_result(wire, "beta-progress"));
                assert!(!value.to_string().contains("A_ONLY_VALUE"));
                assert!(!value.to_string().contains("B_ONLY_VALUE"));
                assert_eq!(
                    value["unresolved_attribution"]["source_event_refs"],
                    json!(source_refs)
                );
                (text_response("Beta inspected.", "beta-done"), false)
            }
            11 => {
                let candidates = selector_candidates(wire);
                let shared = candidates["context"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|candidate| {
                        source_refs.contains(&candidate["ref"])
                            && candidate["excerpt"]
                                .as_str()
                                .is_some_and(|text| text.contains("SHARED_ONLY_VALUE"))
                    })
                    .expect("the explicitly selected shared original is a candidate");
                assert!(shared.get("source_event_ref").is_none());
                (decision(&alpha, vec![shared["ref"].clone()]), false)
            }
            12 => {
                assert!(wire.to_string().contains("SHARED_ONLY_VALUE"));
                assert!(!wire.to_string().contains("B_ONLY_VALUE"));
                let items = wire["input"].as_array().unwrap();
                for kind in ["function_call", "function_call_output"] {
                    assert!(
                        items.iter().any(
                            |item| item["type"] == kind && item["call_id"] == "explicit-shared"
                        ),
                        "selected shared source must retain its original paired {kind}"
                    );
                }
                (
                    text_response("Shared source inspected.", "shared-done"),
                    true,
                )
            }
            _ => panic!("unexpected attribution fixture request {step}"),
        }
    });
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        &format!(
            "Seed a prior task.\n{MULTI}\nReturn to objective A.\nReturn to objective B.\nReturn to objective A with the shared source.\n/exit\n"
        ),
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(server.requests.try_iter().count(), 14);
    server.join();
    let mut effects = fs::read_to_string(root.path().join("attribution-count.txt"))
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    effects.sort();
    assert_eq!(effects, ["a", "b", "s"]);
    let events = session_events(root.path(), &session_id(root.path()));
    assert_eq!(
        events
            .iter()
            .filter(
                |event| event["event_type"] == "tool.execution_outcome_recorded"
                    && matches!(
                        event["payload"]["result"]["call_id"].as_str(),
                        Some("independent-alpha" | "independent-beta" | "explicit-shared")
                    )
            )
            .count(),
        3
    );
    assert!(
        events
            .iter()
            .filter(|event| event["event_type"] == "session.task_context_selected")
            .all(|event| event["payload"]["request_proof_version"] == 10)
    );
    let mut unsupported = events;
    let selection = unsupported
        .iter_mut()
        .find(|event| event["event_type"] == "session.task_context_selected")
        .unwrap();
    selection["payload"]["request_proof_version"] = json!(11);
    let envelopes = unsupported
        .into_iter()
        .map(|event| serde_json::from_value::<EventEnvelope<Value>>(event).unwrap())
        .collect::<Vec<_>>();
    assert!(SessionReducer.reduce(envelopes).is_err());
}

#[test]
fn tiny_progress_reads_keep_large_tool_result_exactly_recoverable() {
    use echidna::domain::model::ContentItem;
    use echidna::domain::tools::{ToolCall, ToolCallId, ToolName, ToolResult, ToolResultStatus};

    let root = TemporaryDirectory::new("task-progress-large-readback");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let setup = super::task_context::spawn_scripted_responses_server(vec![
        super::task_context::ScriptedResponse::InitialSelection,
        super::task_context::ScriptedResponse::Response(
            200,
            text_response("Seed complete.", "large-seed"),
        ),
    ]);
    let setup_config = write_context_window_config(root.path(), &setup.base_url, 4_000_000);
    let output = run_chat(
        root.path(),
        &setup_config,
        "Preserve a large synthetic result.\n/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    selector_candidates(&f2a_request_body(
        &setup.requests,
        "large progress setup selection",
    ));
    let _ = f2a_request_body(&setup.requests, "large progress setup main");
    assert!(setup.requests.try_recv().is_err());
    setup.join();
    let session = session_id(root.path());
    let project = Project::discover(root.path()).unwrap();
    let call = ToolCall::new(
        ToolCallId::new("synthetic-large-result"),
        ToolName::new("run_command"),
        json!({"argv":["fixture"]}),
    );
    let item = ContentItem::ToolResult {
        result: ToolResult {
            call_id: call.call_id.clone(),
            tool_name: call.name.clone(),
            status: ToolResultStatus::Succeeded,
            output: json!({"payload":format!("{}LARGE-ORIGINAL-END", "z".repeat(2_100_000))}),
        },
    };
    let original = serde_json::to_string(&item).unwrap();
    assert!(original.len() > 2 * 1024 * 1024);
    let refs = super::task_context_paging::append_source_items(
        &project,
        &session,
        vec![ContentItem::ToolCall { call }, item],
    );
    let result_ref = refs[1].clone();
    let expected = original.clone();
    let source_ref = result_ref.clone();
    let mut selected_task = String::new();
    let server = paged_server(move |step, wire| match step {
        0 => {
            let candidates = selector_candidates(wire);
            selected_task = candidates["tasks"][0]["ref"].as_str().unwrap().to_owned();
            (decision(&selected_task, Vec::new()), false)
        }
        1 => {
            let calls = [
                (
                    "tiny-progress",
                    "context_read",
                    json!({"operation":"progress", "task_ref":selected_task, "max_bytes":16}),
                ),
                (
                    "whole-progress",
                    "context_read",
                    json!({"operation":"progress", "task_ref":selected_task, "max_bytes":16384}),
                ),
            ];
            (tool_calls_response(&calls, "large-progress-calls"), false)
        }
        2 => {
            let tiny = delivered_result(wire, "tiny-progress");
            assert_eq!(tiny["output"]["status"], "found", "{tiny}");
            assert_eq!(
                tiny["output"]["entries"][0]["text"].as_str().unwrap().len(),
                16
            );
            let value = progress(&delivered_result(wire, "whole-progress"));
            let observation = value["observations"]
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| entry["source_event_ref"] == source_ref)
                .unwrap();
            assert_eq!(observation["excerpt"], &expected[..512]);
            assert_eq!(observation["truncated"], true);
            let chunk = 400_000;
            let calls = (0..expected.len().div_ceil(chunk))
                .map(|index| {
                    (
                        format!("large-source-{index}"),
                        json!({"operation":"read", "reference":source_ref,
                    "offset":index * chunk, "max_bytes":chunk}),
                    )
                })
                .collect::<Vec<_>>();
            let calls = calls
                .iter()
                .map(|(name, args)| (name.as_str(), "context_read", args.clone()))
                .collect::<Vec<_>>();
            (tool_calls_response(&calls, "large-source-ranges"), false)
        }
        3 => {
            let chunk = 400_000;
            let mut recovered = String::new();
            for index in 0..expected.len().div_ceil(chunk) {
                let result = delivered_result(wire, &format!("large-source-{index}"));
                assert_eq!(result["output"]["status"], "found", "{result}");
                recovered.push_str(result["output"]["entries"][0]["text"].as_str().unwrap());
            }
            assert_eq!(recovered, expected);
            (
                text_response("Large original recovered.", "large-progress-done"),
                true,
            )
        }
        _ => panic!("unexpected large progress fixture request {step}"),
    });
    let config = write_context_window_config(root.path(), &server.base_url, 4_000_000);
    let output = run_chat(
        root.path(),
        &config,
        "Continue the same synthetic task.\n/exit\n",
        &["--resume", &session],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.join();
    let events = session_events(root.path(), &session);
    let recorded = events
        .iter()
        .find(|event| event["event_id"] == result_ref)
        .unwrap();
    assert_eq!(
        recorded["payload"]["item"],
        serde_json::from_str::<Value>(&original).unwrap()
    );
}
