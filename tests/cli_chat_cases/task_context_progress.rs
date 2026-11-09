//! Automatic progress remains derived from actual selected-task facts.
use super::task_context::{ScriptedResponse, definition, spawn_scripted_responses_server};
use super::*;

const COMPACTED_GOAL: &str =
    "COMPACTED-ORIGINAL-GOAL: execute the synthetic task; GLOBAL-LIMIT-17; LOCAL-LIMIT-23.";

const INITIAL_MULTIPLE: &str =
    "OBJECTIVE-A inspect alpha; OBJECTIVE-B inspect beta; GLOBAL-LIMIT always use UTC";

fn select_initial_objectives_and_global_constraint(candidates: &Value) -> Value {
    assert!(candidates["tasks"].as_array().unwrap().is_empty());
    assert_eq!(candidates["input"]["text"], INITIAL_MULTIPLE);
    let split = INITIAL_MULTIPLE.find(';').unwrap();
    let second_start = split + 2;
    let global_start = INITIAL_MULTIPLE.find("GLOBAL-LIMIT").unwrap();
    json!({"relation":"new", "focus":{"kind":"new_from_input", "input_ref":candidates["input"]["ref"]},
        "related_task_refs":[], "required_context_refs":[], "unresolved_references":[],
        "new_objectives":[
            {"start_byte":0,"end_byte":split,"description":"OBJECTIVE-A"},
            {"start_byte":second_start,"end_byte":global_start - 2,"description":"OBJECTIVE-B"}],
        "constraint_updates":[{"source_ref":candidates["input"]["ref"],"start_byte":global_start,"end_byte":INITIAL_MULTIPLE.len(),"scope":{"kind":"global"},"supersedes":null}]})
}

#[test]
fn initial_selection_records_multiple_objectives_and_global_constraint_for_later_task() {
    let root = TemporaryDirectory::new("task-first-semantic-selection");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let server = spawn_scripted_responses_server(vec![
        ScriptedResponse::SelectionDecision(select_initial_objectives_and_global_constraint),
        ScriptedResponse::Response(200, text_response("Initial work noted.", "first-main")),
        ScriptedResponse::NewSelection,
        ScriptedResponse::Response(
            200,
            text_response("Unrelated work noted.", "unrelated-main"),
        ),
    ]);
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        &format!("{INITIAL_MULTIPLE}\nInspect an unrelated third objective.\n/exit\n"),
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let wires = (0..4)
        .map(|_| f2a_request_body(&server.requests, "first semantic selection"))
        .collect::<Vec<_>>();
    server.join();
    assert!(json_contains_text(
        &wires[3]["input"],
        "GLOBAL-LIMIT always use UTC"
    ));
    let id = session_id(root.path());
    let events = session_events(root.path(), &id);
    let selection = events
        .iter()
        .find(|event| event["event_type"] == "session.task_context_selected")
        .unwrap();
    let revisions = selection["payload"]["task_revisions"].as_array().unwrap();
    assert_eq!(revisions.len(), 3);
    let split = INITIAL_MULTIPLE.find(';').unwrap();
    let global_start = INITIAL_MULTIPLE.find("GLOBAL-LIMIT").unwrap();
    for (revision, goal, range) in [
        (&revisions[1], "OBJECTIVE-A", [0, split]),
        (&revisions[2], "OBJECTIVE-B", [split + 2, global_start - 2]),
    ] {
        assert_eq!(revision["goal"], goal);
        assert_eq!(revision["input_range"], json!(range));
        assert!(
            revision["task_id"]
                .as_str()
                .unwrap()
                .contains(selection["payload"]["input_ref"].as_str().unwrap())
        );
    }
    assert_eq!(
        selection["payload"]["constraint_revisions"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        selection["payload"]["constraint_revisions"][0]["scope"]["kind"],
        "global"
    );
    assert_eq!(
        selection["payload"]["constraint_revisions"][0]["range"]["source_event_id"],
        selection["payload"]["input_ref"]
    );
    assert_eq!(
        selection["payload"]["constraint_revisions"][0]["range"]["start_byte"],
        global_start
    );
    assert_eq!(
        selection["payload"]["constraint_revisions"][0]["range"]["end_byte"],
        INITIAL_MULTIPLE.len()
    );
    let state = reduced_session(&Project::discover(root.path()).unwrap(), &id);
    assert_audits_reconstruct(root.path(), &state);
}

pub(super) fn logical_user_texts(root: &std::path::Path, audit: &Value) -> Vec<String> {
    let blobs = root.join(".echidna/artifacts/blobs/sha256");
    audit["payload"]["full_logical_components"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|component| component["identity"]["kind"] == "content_item")
        .filter_map(|component| {
            let item: Value = serde_json::from_slice(
                &fs::read(blobs.join(component["blob_id"].as_str().unwrap())).unwrap(),
            )
            .unwrap();
            (item["kind"] == "text" && item["role"] == "user")
                .then(|| item["text"].as_str().unwrap().to_owned())
        })
        .collect()
}

#[test]
fn changed_project_snapshot_is_frozen_before_later_main_without_resampling_or_replay() {
    let root = TemporaryDirectory::new("task-progress-snapshot-refresh");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let server = spawn_scripted_responses_server(vec![
        ScriptedResponse::InitialSelection,
        ScriptedResponse::Response(
            200,
            m4_long_flow_tool_response(
                "snapshot-write",
                "run_command",
                json!({"argv":["/bin/sh","-c","printf 'one\\n' >> snapshot-write.txt"]}),
                "snapshot-tool",
            ),
        ),
        ScriptedResponse::Response(200, text_response("Snapshot checked.", "snapshot-final")),
    ]);
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "Write the synthetic snapshot file once.\n/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for _ in 0..3 {
        f2a_request_body(&server.requests, "snapshot refresh");
    }
    assert!(
        server.requests.try_recv().is_err(),
        "a source refresh must not sample another selector"
    );
    server.join();
    assert_eq!(
        fs::read_to_string(root.path().join("snapshot-write.txt")).unwrap(),
        "one\n"
    );
    let events = session_events(root.path(), &session_id(root.path()));
    let selections = events
        .iter()
        .filter(|event| event["event_type"] == "session.task_context_selected")
        .collect::<Vec<_>>();
    assert_eq!(selections.len(), 2);
    assert_eq!(
        selections[1]["payload"]["refresh_of"],
        selections[0]["event_id"]
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event["event_type"] == "session.task_selection_sample_recorded")
            .count(),
        1
    );
    let audits = events
        .iter()
        .filter(|event| event["event_type"] == "session.request_audit_recorded")
        .collect::<Vec<_>>();
    assert_eq!(audits.len(), 3);
    let snapshot_revision = |audit: &Value| {
        audit["payload"]["full_context_delivery_map"]["content_items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["fragment_id"] == "project-snapshot:full")
            .unwrap()["provenance"]["freshness"]["source_revision_or_hash"]
            .clone()
    };
    assert_ne!(
        snapshot_revision(audits[1]),
        snapshot_revision(audits[2]),
        "the later audit must use the fresh snapshot"
    );
    let texts = logical_user_texts(root.path(), audits[2]);
    let goal = "Write the synthetic snapshot file once.";
    assert_eq!(
        texts
            .iter()
            .map(|text| text.matches(goal).count())
            .sum::<usize>(),
        1,
        "the exact current operator input must not be repeated by task progress"
    );
    assert!(
        texts
            .iter()
            .any(|text| text.contains("\"goal_in_operator_input\":true"))
    );
    let resumed = run_chat(root.path(), &config, "/exit\n", &["--resume", "latest"]);
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    assert_eq!(
        fs::read_to_string(root.path().join("snapshot-write.txt")).unwrap(),
        "one\n"
    );
}

fn select_identical_prior(candidates: &Value) -> Value {
    let source = candidates["context"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["excerpt"] == "IDENTICAL-TASK-GOAL")
        .unwrap();
    json!({"relation":"continue", "focus":{"kind":"existing", "task_ref":candidates["tasks"][0]["ref"]},
        "related_task_refs":[], "required_context_refs":[source["ref"]], "unresolved_references":[]})
}

#[test]
fn identical_old_and_current_inputs_remain_two_distinct_sources_after_snapshot_refresh() {
    let root = TemporaryDirectory::new("task-progress-identical-sources");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let server = spawn_scripted_responses_server(vec![
        ScriptedResponse::InitialSelection,
        ScriptedResponse::Response(200, text_response("Initial result.", "identical-initial")),
        ScriptedResponse::SelectionDecision(select_identical_prior),
        ScriptedResponse::Response(
            200,
            m4_long_flow_tool_response(
                "identical-write",
                "run_command",
                json!({"argv":["/bin/sh","-c","printf 'one\\n' >> identical-count.txt"]}),
                "identical-tool",
            ),
        ),
        ScriptedResponse::Response(200, text_response("Completed.", "identical-final")),
    ]);
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "IDENTICAL-TASK-GOAL\nIDENTICAL-TASK-GOAL\n/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for _ in 0..5 {
        f2a_request_body(&server.requests, "identical source control");
    }
    assert!(server.requests.try_recv().is_err());
    server.join();
    assert_eq!(
        fs::read_to_string(root.path().join("identical-count.txt")).unwrap(),
        "one\n"
    );
    let events = session_events(root.path(), &session_id(root.path()));
    let audits = events
        .iter()
        .filter(|event| event["event_type"] == "session.request_audit_recorded")
        .collect::<Vec<_>>();
    let texts = logical_user_texts(root.path(), audits.last().unwrap());
    assert_eq!(
        texts
            .iter()
            .map(|text| text.matches("IDENTICAL-TASK-GOAL").count())
            .sum::<usize>(),
        2,
        "distinct original and current operator inputs each appear exactly once"
    );
    assert!(
        texts
            .iter()
            .any(|text| text.contains("\"goal_in_selected_history\":true"))
    );
    let ids = events
        .iter()
        .filter(|event| {
            event["event_type"] == "session.history_item_recorded"
                && event["payload"]["item"]["text"] == "IDENTICAL-TASK-GOAL"
        })
        .map(|event| event["event_id"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[0], ids[1]);
    let map = &audits.last().unwrap()["payload"]["full_context_delivery_map"]["content_items"];
    for id in ids {
        assert!(map.as_array().unwrap().iter().any(|entry| {
            entry["provenance"]["references"]["source_event_refs"]
                .as_array()
                .is_some_and(|refs| refs.iter().any(|reference| reference == id))
        }));
    }
    let resumed = run_chat(root.path(), &config, "/exit\n", &["--resume", "latest"]);
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    assert_eq!(
        fs::read_to_string(root.path().join("identical-count.txt")).unwrap(),
        "one\n"
    );
}

fn select_goal_and_constraints(candidates: &Value) -> Value {
    let source = candidates["context"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["excerpt"] == COMPACTED_GOAL)
        .unwrap();
    let older_answer = candidates["context"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| {
            entry["excerpt"]
                .as_str()
                .is_some_and(|text| text.starts_with("OLDER-FINISHED "))
        })
        .unwrap();
    let range = |text: &str| {
        let start = COMPACTED_GOAL.find(text).unwrap();
        (start, start + text.len())
    };
    let (global_start, global_end) = range("GLOBAL-LIMIT-17");
    let (local_start, local_end) = range("LOCAL-LIMIT-23");
    json!({"relation":"continue", "focus":{"kind":"existing", "task_ref":candidates["tasks"][0]["ref"]},
    "related_task_refs":[], "required_context_refs":[source["ref"], older_answer["ref"]], "unresolved_references":[],
    "constraint_updates":[
        {"source_ref":source["ref"], "start_byte":global_start, "end_byte":global_end, "scope":{"kind":"global"}, "supersedes":null},
        {"source_ref":source["ref"], "start_byte":local_start, "end_byte":local_end, "scope":{"kind":"current_task"}, "supersedes":null}
    ]})
}

#[test]
fn source_goal_and_constraints_survive_lossy_batch_compaction_and_resume() {
    let root = TemporaryDirectory::new("task-progress-lossy-compaction");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let (sender, requests) = mpsc::channel();
    let (stop, stopping) = mpsc::channel();
    let thread = thread::spawn(move || {
        let mut main_count = 0;
        let mut compaction_count = 0;
        let mut request_count = 0;
        let mut deadline = Instant::now() + F2A_WAIT;
        loop {
            let mut socket = match listener.accept() {
                Ok((socket, _)) => socket,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if stopping.try_recv().is_ok() {
                        break;
                    }
                    assert!(Instant::now() < deadline, "lossy fixture request timed out");
                    thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(error) => panic!("lossy fixture accept failed: {error}"),
            };
            request_count += 1;
            assert!(request_count <= 16, "lossy fixture request bound exceeded");
            deadline = Instant::now() + F2A_WAIT;
            socket.set_nonblocking(false).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let request = read_http_request(&mut socket);
            let wire = request_body(&request);
            let instructions = wire["instructions"].as_str().unwrap_or("");
            let response = if instructions.contains("continuation handoff") {
                compaction_count += 1;
                compaction_summary_response(
                    "Neutral tool work completed. Continue from the recorded results.",
                    &format!("lossy-summary-{compaction_count}"),
                )
            } else if wire.get("tools").is_none() {
                let candidates = super::task_context::selector_candidates(&wire);
                if candidates["tasks"].as_array().unwrap().is_empty() {
                    super::task_context::initial_selection_response(
                        &wire,
                        "lossy-initial-selection",
                    )
                } else {
                    text_response(
                        &select_goal_and_constraints(&candidates).to_string(),
                        "lossy-selection",
                    )
                }
            } else {
                let index = main_count;
                main_count += 1;
                match index {
                    0 => text_response(
                        &format!("OLDER-FINISHED {}", "historical answer ".repeat(10_000)),
                        "old-task",
                    ),
                    1..=3 => {
                        let batch = index - 1;
                        let mut response = format!(
                            "data: {}\n\n",
                            json!({"type":"response.output_item.done", "output_index":0,
                            "item":{"type":"reasoning", "content":[{"type":"reasoning_text", "text":format!("batch-{batch}-reasoning ").repeat(if batch == 0 { 12_000 } else { 20_000 })}]}})
                        );
                        response.push_str(&m4_long_flow_tool_response(&format!("compact-count-{batch}"), "run_command",
                            json!({"argv":["/bin/sh", "-c", format!("printf '%s\\n' {batch} >> actual-count.txt; printf '%s' \"$1\""), "fixture", "neutral-output ".repeat(2_000)]}),
                            &format!("compact-response-{batch}")));
                        response
                    }
                    4 => text_response("Synthetic work finished.", "compact-final"),
                    _ => panic!("unexpected extra main request"),
                }
            };
            sender.send(request).unwrap();
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).unwrap();
        }
    });
    let server = LoopbackServer {
        base_url: format!("http://{address}/v1"),
        requests,
        thread,
    };
    let config = write_context_window_config(root.path(), &server.base_url, 1_048_576);
    fs::write(&config, format!("{}\n[runtime]\nauto_compact = true\n[runtime.context_budget]\npressure_bps = 1000\ntarget_bps = 250\n", fs::read_to_string(&config).unwrap())).unwrap();
    let output = run_chat(
        root.path(),
        &config,
        &format!("{COMPACTED_GOAL}\nContinue the original synthetic task.\n/exit\n"),
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    stop.send(()).unwrap();
    let received = server
        .requests
        .try_iter()
        .map(|request| request_body(&request))
        .collect::<Vec<_>>();
    let scopes = received
        .iter()
        .map(|wire| {
            let instructions = wire["instructions"].as_str().unwrap_or("");
            (
                if instructions.contains("continuation handoff") {
                    "compaction"
                } else if wire.get("tools").is_some() {
                    "main"
                } else {
                    "selection"
                },
                wire["previous_response_id"].as_str().map(str::to_owned),
            )
        })
        .collect::<Vec<_>>();
    let stderr = String::from_utf8_lossy(&output.stderr);
    let tail = stderr
        .char_indices()
        .rev()
        .nth(1500)
        .map_or(stderr.as_ref(), |(index, _)| &stderr[index..]);
    assert!(
        received.len() <= 16,
        "request scopes {scopes:?}; stderr tail: {tail}"
    );
    let wires = received;
    server.join();
    assert_eq!(
        fs::read_to_string(root.path().join("actual-count.txt")).unwrap(),
        "0\n1\n2\n"
    );
    let events = session_events(root.path(), &session_id(root.path()));
    let audits = events
        .iter()
        .enumerate()
        .filter(|(_, event)| event["event_type"] == "session.request_audit_recorded")
        .collect::<Vec<_>>();
    assert_eq!(
        audits.len(),
        wires.len(),
        "every wire must have one durable audit"
    );
    let first_continued_main = scopes
        .iter()
        .enumerate()
        .filter(|(_, (scope, _))| *scope == "main")
        .nth(1)
        .unwrap()
        .0;
    assert_eq!(
        wires[first_continued_main]["input"]
            .to_string()
            .matches(COMPACTED_GOAL)
            .count(),
        1
    );
    let goal_event = events
        .iter()
        .find(|event| {
            event["event_type"] == "session.history_item_recorded"
                && event["payload"]["item"]["text"] == COMPACTED_GOAL
        })
        .unwrap()["event_id"]
        .as_str()
        .unwrap();
    let compactions = events
        .iter()
        .enumerate()
        .filter(|(_, event)| event["event_type"] == "session.compaction_created")
        .collect::<Vec<_>>();
    assert!(
        compactions.len() >= 2,
        "at least two real compactions required; scopes {scopes:?}"
    );
    let mut source_in_history = true;
    let mut removed = false;
    for (compaction_index, compaction) in &compactions {
        let (wire_index, (_, audit)) = audits
            .iter()
            .enumerate()
            .find(|(wire_index, (event_index, _))| {
                event_index > compaction_index && scopes[*wire_index].0 == "main"
            })
            .unwrap_or_else(|| {
                panic!(
                    "compaction {} has no later main audit; scopes {scopes:?}",
                    compaction["event_id"]
                )
            });
        let wire = &wires[wire_index];
        assert_eq!(audit["event_type"], "session.request_audit_recorded");
        let retained = compaction["payload"]["retained_event_ids"]
            .as_array()
            .unwrap();
        let summarized = compaction["payload"]["summary"]["summarized_history"].as_array();
        if retained.iter().any(|id| id == goal_event) {
            source_in_history = true;
        } else if summarized.is_some_and(|ids| ids.iter().any(|id| id == goal_event)) {
            source_in_history = false;
            removed = true;
        }
        for required in [
            "COMPACTED-ORIGINAL-GOAL",
            "GLOBAL-LIMIT-17",
            "LOCAL-LIMIT-23",
        ] {
            assert!(
                json_contains_text(&wire["input"], required),
                "missing {required} after compaction"
            );
        }
        assert_eq!(
            json_contains_text(&wire["input"], "\"goal_in_selected_history\":true"),
            source_in_history,
            "progress must agree with the actual source projection; goal event {goal_event}, retained_from={}, retained_goal={}, summarized_goal={}",
            compaction["payload"]["retained_from_normalized_history_index"],
            retained.iter().any(|id| id == goal_event),
            summarized.is_some_and(|ids| ids.iter().any(|id| id == goal_event))
        );
        if source_in_history {
            assert_eq!(wire["input"].to_string().matches(COMPACTED_GOAL).count(), 1);
        } else {
            assert!(json_contains_text(
                &wire["input"],
                "\"goal_in_selected_history\":false"
            ));
            assert!(json_contains_text(&wire["input"], goal_event));
        }
    }
    assert!(
        removed,
        "fixture never removed the selected goal; goal event {goal_event}, compaction retention {:?}",
        compactions
            .iter()
            .map(|(_, event)| {
                let payload = &event["payload"];
                let retained = payload["retained_event_ids"].as_array().unwrap();
                let summarized = payload["summary"]["summarized_history"].as_array();
                (
                    payload["retained_from_normalized_history_index"].clone(),
                    retained.len(),
                    retained.iter().any(|id| id == goal_event),
                    summarized.map_or(0, Vec::len),
                    summarized.is_some_and(|ids| ids.iter().any(|id| id == goal_event)),
                    payload["summary"]["model_sample_event_ref"].clone(),
                )
            })
            .collect::<Vec<_>>()
    );
    let resume_server = spawn_responses_server(vec![(
        200,
        compaction_summary_response("Another neutral handoff.", "resume-lossy"),
    )]);
    let resume_config =
        write_context_window_config(root.path(), &resume_server.base_url, 1_048_576);
    let resumed = run_chat(
        root.path(),
        &resume_config,
        "/compact\n/exit\n",
        &["--resume", "latest"],
    );
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    let handoff = f2a_request_body(&resume_server.requests, "resumed lossy compaction");
    resume_server.join();
    for required in [
        "COMPACTED-ORIGINAL-GOAL",
        "GLOBAL-LIMIT-17",
        "LOCAL-LIMIT-23",
    ] {
        assert!(
            json_contains_text(&handoff["input"], required),
            "missing {required} after Resume"
        );
    }
    assert_eq!(
        fs::read_to_string(root.path().join("actual-count.txt")).unwrap(),
        "0\n1\n2\n"
    );
}

fn continue_without_history(candidates: &Value) -> Value {
    let tasks = candidates["tasks"].as_array().unwrap();
    assert_eq!(tasks.len(), 1);
    json!({"relation":"continue", "focus":{"kind":"existing", "task_ref":tasks[0]["ref"]},
        "related_task_refs":[], "required_context_refs":[], "unresolved_references":[]})
}

fn continue_with_unresolved_gap(candidates: &Value) -> Value {
    let mut decision = continue_without_history(candidates);
    decision["unresolved_references"] = json!(["PROGRESS-UNRESOLVED-SOURCE"]);
    decision
}

fn continue_with_original_history(candidates: &Value) -> Value {
    let mut decision = continue_without_history(candidates);
    let source = candidates["context"]
        .as_array()
        .unwrap()
        .iter()
        .find(|candidate| candidate["excerpt"] == "GOAL-ALREADY-IN-SELECTED-HISTORY")
        .unwrap();
    decision["required_context_refs"] = json!([source["ref"]]);
    decision
}

#[test]
fn selected_goal_history_is_delivered_once_and_progress_keeps_unresolved_gaps() {
    let root = TemporaryDirectory::new("task-progress-selected-goal");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let server = spawn_scripted_responses_server(vec![
        ScriptedResponse::InitialSelection,
        ScriptedResponse::Response(
            200,
            text_response("Initial observation.", "selected-goal-first"),
        ),
        ScriptedResponse::SelectionDecision(continue_with_unresolved_gap),
        ScriptedResponse::SelectionDecision(continue_with_unresolved_gap),
        ScriptedResponse::Response(
            200,
            text_response("Source remains missing.", "selected-goal-gap"),
        ),
        ScriptedResponse::SelectionDecision(continue_with_original_history),
        ScriptedResponse::Response(200, text_response("Continuing.", "selected-goal-follow")),
    ]);
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let goal = "GOAL-ALREADY-IN-SELECTED-HISTORY";
    let output = run_chat(
        root.path(),
        &config,
        &format!("{goal}\nFind the missing evidence.\nContinue\n/exit\n"),
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let wires = (0..7)
        .map(|_| f2a_request_body(&server.requests, "selected goal history"))
        .collect::<Vec<_>>();
    assert!(server.requests.try_recv().is_err());
    server.join();
    assert_eq!(wires[6]["input"].to_string().matches(goal).count(), 1);
    assert!(json_contains_text(
        &wires[6]["input"],
        "PROGRESS-UNRESOLVED-SOURCE"
    ));
    assert!(json_contains_text(
        &wires[6]["input"],
        "Source-bound task progress"
    ));
    let state = reduced_session(
        &Project::discover(root.path()).unwrap(),
        &session_id(root.path()),
    );
    assert_audits_reconstruct(root.path(), &state);
    let resumed = run_chat(
        root.path(),
        &config,
        "/context\n/exit\n",
        &["--resume", "latest"],
    );
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
}

const SHARED_TASK_TEXT: &str = "SHARED-EXACT-TEXT-17";
const SHARED_B_INPUT: &str = "BETA-IDENTITY: SHARED-EXACT-TEXT-17 applies only to beta.";
const SHARED_B_CORRECTION: &str = "BETA-CORRECTION: use 19 only for beta.";

fn shared_task_ref(candidates: &Value, marker: &str) -> Value {
    candidates["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|task| {
            task["goal"]
                .as_str()
                .is_some_and(|goal| goal.contains(marker))
        })
        .unwrap()["ref"]
        .clone()
}

fn record_beta_shared_constraint(candidates: &Value) -> Value {
    let task = shared_task_ref(candidates, "BETA-IDENTITY");
    let source = candidates["context"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["excerpt"] == SHARED_B_INPUT)
        .unwrap();
    let start = SHARED_B_INPUT.find(SHARED_TASK_TEXT).unwrap();
    json!({"relation":"continue", "focus":{"kind":"existing", "task_ref":task},
        "related_task_refs":[],"required_context_refs":[],"unresolved_references":[],
        "constraint_updates":[{"source_ref":source["ref"],"start_byte":start,"end_byte":start+SHARED_TASK_TEXT.len(),
            "scope":{"kind":"task","task_ref":task},"supersedes":null}]})
}

fn correct_beta_shared_constraint(candidates: &Value) -> Value {
    let task = shared_task_ref(candidates, "BETA-IDENTITY");
    let prior = candidates["context"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["excerpt"] == SHARED_TASK_TEXT && entry["constraint_range"].is_object())
        .unwrap();
    json!({"relation":"continue", "focus":{"kind":"existing", "task_ref":task},
        "related_task_refs":[],"required_context_refs":[],"unresolved_references":[],
        "constraint_updates":[{"source_ref":candidates["input"]["ref"],"start_byte":0,"end_byte":SHARED_B_CORRECTION.len(),
            "scope":{"kind":"task","task_ref":task},"supersedes":prior["ref"]}]})
}

fn return_alpha_shared(candidates: &Value) -> Value {
    json!({"relation":"return", "focus":{"kind":"existing", "task_ref":shared_task_ref(candidates,"ALPHA-IDENTITY")},
        "related_task_refs":[],"required_context_refs":[],"unresolved_references":[]})
}

#[test]
fn correcting_beta_constraint_does_not_erase_identical_alpha_goal_text() {
    let root = TemporaryDirectory::new("task-progress-same-text-different-source");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let server = spawn_scripted_responses_server(vec![
        ScriptedResponse::InitialSelection,
        ScriptedResponse::Response(
            200,
            m4_long_flow_tool_response(
                "shared-alpha-observation",
                "run_command",
                json!({"argv":["/bin/sh","-c","printf 'ALPHA-OBS SHARED-EXACT-TEXT-17'"]}),
                "shared-a-tool",
            ),
        ),
        ScriptedResponse::Response(200, text_response("Alpha observed.", "shared-a")),
        ScriptedResponse::NewSelection,
        ScriptedResponse::Response(200, text_response("Beta observed.", "shared-b")),
        ScriptedResponse::SelectionDecision(record_beta_shared_constraint),
        ScriptedResponse::Response(200, text_response("Beta limit recorded.", "shared-limit")),
        ScriptedResponse::SelectionDecision(correct_beta_shared_constraint),
        ScriptedResponse::Response(200, text_response("Beta corrected.", "shared-corrected")),
        ScriptedResponse::SelectionDecision(return_alpha_shared),
        ScriptedResponse::Response(200, text_response("Alpha continued.", "shared-return")),
    ]);
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let original = "ALPHA-IDENTITY: analyze SHARED-EXACT-TEXT-17 as alpha's objective.";
    let output = run_chat(
        root.path(),
        &config,
        &format!(
            "{original}\n{SHARED_B_INPUT}\nContinue beta.\n{SHARED_B_CORRECTION}\nReturn to alpha.\n/exit\n"
        ),
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let wires = (0..11)
        .map(|_| f2a_request_body(&server.requests, "same text across tasks"))
        .collect::<Vec<_>>();
    assert!(server.requests.try_recv().is_err());
    server.join();
    assert!(json_contains_text(&wires[10]["input"], original));
    assert!(json_contains_text(
        &wires[10]["input"],
        "ALPHA-OBS SHARED-EXACT-TEXT-17"
    ));
    assert!(!json_contains_text(
        &wires[10]["input"],
        SHARED_B_CORRECTION
    ));
    let state = reduced_session(
        &Project::discover(root.path()).unwrap(),
        &session_id(root.path()),
    );
    assert_audits_reconstruct(root.path(), &state);
    let resumed = run_chat(
        root.path(),
        &config,
        "/context\n/exit\n",
        &["--resume", "latest"],
    );
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
}

#[test]
fn continuation_keeps_original_goal_and_distinct_equal_gap_sources_without_selected_history() {
    let root = TemporaryDirectory::new("task-progress-goal-and-gap");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let server = spawn_scripted_responses_server(vec![
        ScriptedResponse::InitialSelection,
        ScriptedResponse::Response(200, text_response("Initial observation.", "goal-first")),
        ScriptedResponse::SelectionDecision(continue_with_unresolved_gap),
        ScriptedResponse::SelectionDecision(continue_with_unresolved_gap),
        ScriptedResponse::Response(200, text_response("Source remains missing.", "goal-gap")),
        ScriptedResponse::SelectionDecision(continue_with_unresolved_gap),
        ScriptedResponse::SelectionDecision(continue_with_unresolved_gap),
        ScriptedResponse::Response(
            200,
            text_response("Another source is missing.", "goal-gap-two"),
        ),
        ScriptedResponse::SelectionDecision(continue_without_history),
        ScriptedResponse::Response(200, text_response("Continuing.", "goal-follow")),
    ]);
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let original = "FIX-ORIGINAL-OBJECTIVE: diagnose the synthetic broken parser.";
    let output = run_chat(
        root.path(),
        &config,
        &format!(
            "{original}\nFind the missing evidence.\nCheck another independent source.\nContinue\n/exit\n"
        ),
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let wires = (0..10)
        .map(|_| f2a_request_body(&server.requests, "goal and gap"))
        .collect::<Vec<_>>();
    assert!(server.requests.try_recv().is_err());
    server.join();
    assert!(json_contains_text(&wires[9]["input"], original));
    assert!(json_contains_text(
        &wires[9]["input"],
        "PROGRESS-UNRESOLVED-SOURCE"
    ));
    let id = session_id(root.path());
    let events = session_events(root.path(), &id);
    let original_id = events
        .iter()
        .find(|event| {
            event["event_type"] == "session.history_item_recorded"
                && event["payload"]["item"]["text"] == original
        })
        .unwrap()["event_id"]
        .as_str()
        .unwrap();
    let gap_ids = events
        .iter()
        .filter(|event| {
            event["event_type"] == "session.task_context_selected"
                && event["payload"]["unresolved_gaps"]
                    .to_string()
                    .contains("PROGRESS-UNRESOLVED-SOURCE")
        })
        .map(|event| event["event_id"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(gap_ids.len(), 2);
    assert_ne!(gap_ids[0], gap_ids[1]);
    let state = reduced_session(&Project::discover(root.path()).unwrap(), &id);
    let map = serde_json::to_string(
        state
            .request_audits()
            .last()
            .unwrap()
            .context_delivery_map()
            .unwrap(),
    )
    .unwrap();
    assert!(map.contains(original_id));
    for gap_id in gap_ids {
        assert!(map.contains(gap_id));
        assert!(json_contains_text(&wires[9]["input"], gap_id));
    }
    assert_audits_reconstruct(root.path(), &state);
    let resumed = run_chat(
        root.path(),
        &config,
        "/context\n/exit\n",
        &["--resume", "latest"],
    );
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
}

fn continue_beta_with_alpha_method(candidates: &Value) -> Value {
    let task = candidates["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|task| task["goal"].as_str().unwrap().contains("BETA-OBJECTIVE"))
        .unwrap();
    let source = candidates["context"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| {
            entry["excerpt"]
                .as_str()
                .is_some_and(|text| text.contains("ALPHA-METHOD-ONLY"))
        })
        .unwrap();
    let text = source["excerpt"].as_str().unwrap();
    let start = text.find("ALPHA-METHOD-ONLY").unwrap();
    json!({"relation":"continue", "focus":{"kind":"existing", "task_ref":task["ref"]},
        "related_task_refs":[], "required_context_refs":[], "unresolved_references":[],
        "source_ranges":[{"context_ref":source["ref"], "start_byte":start, "end_byte":start+"ALPHA-METHOD-ONLY".len()}]})
}

#[test]
fn other_task_method_range_keeps_focal_task_observations() {
    let root = TemporaryDirectory::new("task-progress-other-range");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let server = spawn_scripted_responses_server(vec![
        ScriptedResponse::InitialSelection,
        ScriptedResponse::Response(
            200,
            text_response(
                "ALPHA-PRIVATE-BODY ALPHA-METHOD-ONLY ALPHA-PRIVATE-END",
                "range-a",
            ),
        ),
        ScriptedResponse::NewSelection,
        ScriptedResponse::Response(
            200,
            m4_long_flow_tool_response(
                "beta-one",
                "run_command",
                json!({"argv":["/bin/sh","-c","printf BETA-OBSERVATION-ONE"]}),
                "range-b-one",
            ),
        ),
        ScriptedResponse::Response(
            200,
            m4_long_flow_tool_response(
                "beta-two",
                "run_command",
                json!({"argv":["/bin/sh","-c","printf BETA-OBSERVATION-TWO"]}),
                "range-b-two",
            ),
        ),
        ScriptedResponse::Response(
            200,
            text_response("Beta progress recorded.", "range-b-done"),
        ),
        ScriptedResponse::SelectionDecision(continue_beta_with_alpha_method),
        ScriptedResponse::Response(200, text_response("Beta continued.", "range-follow")),
    ]);
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "ALPHA-OBJECTIVE-PRIVATE\nBETA-OBJECTIVE: execute both observations.\nContinue beta using the alpha method.\n/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let wires = (0..8)
        .map(|_| f2a_request_body(&server.requests, "range and focal progress"))
        .collect::<Vec<_>>();
    assert!(server.requests.try_recv().is_err());
    server.join();
    for expected in [
        "BETA-OBJECTIVE",
        "BETA-OBSERVATION-ONE",
        "BETA-OBSERVATION-TWO",
        "ALPHA-METHOD-ONLY",
    ] {
        assert!(
            json_contains_text(&wires[7]["input"], expected),
            "missing {expected}"
        );
    }
    for excluded in [
        "ALPHA-OBJECTIVE-PRIVATE",
        "ALPHA-PRIVATE-BODY",
        "ALPHA-PRIVATE-END",
    ] {
        assert!(
            !json_contains_text(&wires[7]["input"], excluded),
            "unexpected {excluded}"
        );
    }
    let state = reduced_session(
        &Project::discover(root.path()).unwrap(),
        &session_id(root.path()),
    );
    assert_eq!(state.execution_receipt_count(), 2);
    assert_audits_reconstruct(root.path(), &state);
    let resumed = run_chat(
        root.path(),
        &config,
        "/context\n/exit\n",
        &["--resume", "latest"],
    );
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
}

#[test]
fn shared_input_method_excerpt_preserves_later_focal_result_and_receipt() {
    use super::task_context::selector_candidates;
    use super::task_context_paging::paged_server;

    const SHARED: &str = "A-PRIVATE-BODY ALPHA-METHOD-ONLY; BETA-OBJECTIVE";
    let root = TemporaryDirectory::new("task-progress-shared-range");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let server = paged_server(move |step, wire| {
        if step == 0 {
            return (
                super::task_context::initial_selection_response(wire, "shared-initial-selection"),
                false,
            );
        }
        let step = step - 1;
        match step {
            0 => (text_response("Seed complete.", "shared-seed"), false),
            1 => {
                let candidates = selector_candidates(wire);
                let split = SHARED.find(';').unwrap();
                let selection = json!({"relation":"new", "focus":{"kind":"new_from_input", "input_ref":candidates["input"]["ref"]},
                "related_task_refs":[], "required_context_refs":[], "unresolved_references":[],
                "new_objectives":[{"start_byte":0,"end_byte":split,"description":"OBJECTIVE A"},
                    {"start_byte":split+2,"end_byte":SHARED.len(),"description":"OBJECTIVE B"}]});
                (
                    text_response(&selection.to_string(), "shared-select"),
                    false,
                )
            }
            2 => (
                text_response("Both objectives recorded.", "shared-finish"),
                false,
            ),
            3 | 6 => {
                let candidates = selector_candidates(wire);
                let beta = candidates["tasks"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|task| task["goal"] == "OBJECTIVE B")
                    .unwrap()["ref"]
                    .as_str()
                    .unwrap()
                    .to_owned();
                let mut selection = json!({"relation":"return", "focus":{"kind":"existing", "task_ref":beta},
                "related_task_refs":[], "required_context_refs":[], "unresolved_references":[]});
                if step == 6 {
                    let source = candidates["context"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|entry| {
                            entry["excerpt"]
                                .as_str()
                                .is_some_and(|text| text.contains(SHARED))
                        })
                        .expect("original shared input candidate");
                    let start = source["excerpt"]
                        .as_str()
                        .unwrap()
                        .find("ALPHA-METHOD-ONLY")
                        .unwrap();
                    selection["source_ranges"] = json!([{"context_ref":source["ref"], "start_byte":start,
                    "end_byte":start + "ALPHA-METHOD-ONLY".len()}]);
                }
                (
                    text_response(&selection.to_string(), "shared-beta-select"),
                    false,
                )
            }
            4 => (
                m4_long_flow_tool_response(
                    "shared-beta-result",
                    "run_command",
                    json!({"argv":["/bin/sh","-c","printf 'once\\n' >> shared-beta-count.txt; printf BETA-LATER-RESULT"]}),
                    "shared-beta-tool",
                ),
                false,
            ),
            5 => (text_response("Beta completed.", "shared-beta-done"), false),
            7 => {
                let input = wire["input"].to_string();
                assert!(input.contains("ALPHA-METHOD-ONLY"));
                assert!(input.contains("BETA-LATER-RESULT"));
                assert!(!input.contains("A-PRIVATE-BODY"));
                (text_response("Beta continued.", "shared-beta-final"), true)
            }
            _ => panic!("unexpected shared-range request {step}"),
        }
    });
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        &format!("Seed.\n{SHARED}\nReturn to beta.\nContinue beta with alpha method.\n/exit\n"),
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.join();
    let session = session_id(root.path());
    let events = session_events(root.path(), &session);
    let receipt = events
        .iter()
        .find(|event| {
            event["event_type"] == "tool.execution_outcome_recorded"
                && event["payload"]["result"]["call_id"] == "shared-beta-result"
        })
        .unwrap();
    let state = reduced_session(&Project::discover(root.path()).unwrap(), &session);
    let map = serde_json::to_string(
        state
            .request_audits()
            .last()
            .unwrap()
            .context_delivery_map()
            .unwrap(),
    )
    .unwrap();
    assert!(
        map.contains(receipt["event_id"].as_str().unwrap()),
        "original receipt must be referenced"
    );
    assert_audits_reconstruct(root.path(), &state);
    let resumed = run_chat(root.path(), &config, "/exit\n", &["--resume", "latest"]);
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    assert_eq!(
        fs::read_to_string(root.path().join("shared-beta-count.txt")).unwrap(),
        "once\n"
    );
}

#[test]
fn selected_summary_keeps_later_and_unsummarized_results_with_receipts() {
    use super::task_context::selector_candidates;
    use super::task_context_paging::paged_server;

    let root = TemporaryDirectory::new("task-progress-summary-coverage");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let mut main_count = 0;
    let server = paged_server(move |_step, wire| {
        if wire["instructions"]
            .as_str()
            .unwrap_or("")
            .contains("continuation handoff")
        {
            return (
                compaction_summary_response(
                    "SUMMARY-COVERAGE: earlier task work.",
                    "coverage-summary",
                ),
                false,
            );
        }
        if wire.get("tools").is_none() {
            let candidates = selector_candidates(wire);
            if candidates["tasks"].as_array().unwrap().is_empty() {
                return (
                    super::task_context::initial_selection_response(
                        wire,
                        "coverage-initial-selection",
                    ),
                    false,
                );
            }
            let task = &candidates["tasks"][0]["ref"];
            let refs = if candidates["input"]["text"]
                .as_str()
                .unwrap()
                .contains("SUMMARY-ONLY")
            {
                vec![
                    candidates["context"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|unit| unit["ref"].as_str().unwrap().starts_with("summary:"))
                        .expect("selected summary candidate")["ref"]
                        .clone(),
                ]
            } else {
                Vec::new()
            };
            let selection = json!({"relation":"continue", "focus":{"kind":"existing", "task_ref":task},
                "related_task_refs":[], "required_context_refs":refs, "unresolved_references":[]});
            return (
                text_response(&selection.to_string(), "coverage-selection"),
                false,
            );
        }
        let index = main_count;
        main_count += 1;
        match index {
            0 => (
                text_response(
                    &format!("Old task details {}", "historical answer ".repeat(10_000)),
                    "coverage-old",
                ),
                false,
            ),
            1..=3 => {
                let call = format!("coverage-before-{index}");
                let label = format!("BEFORE-SUMMARY-{index}");
                let reasoning = json!({"type":"response.output_item.done", "output_index":0,
                    "item":{"type":"reasoning", "content":[{"type":"reasoning_text",
                        "text":format!("batch-{index}-reasoning ").repeat(if index == 1 { 12_000 } else { 20_000 })}]}});
                (
                    format!(
                        "data: {reasoning}\n\n{}",
                        m4_long_flow_tool_response(
                            &call,
                            "run_command",
                            json!({"argv":["/bin/sh","-c",
                            format!("printf '{label}\\n' >> summary-count.txt; printf '{label}'; printf '%s' \"$1\""),
                            "fixture", "neutral-output ".repeat(2_000)]}),
                            &format!("response-{call}")
                        ),
                    ),
                    false,
                )
            }
            4 => (
                text_response("Earlier results recorded.", "coverage-before-done"),
                false,
            ),
            5 => (
                m4_long_flow_tool_response(
                    "coverage-after",
                    "run_command",
                    json!({"argv":["/bin/sh","-c", "printf 'AFTER-SUMMARY\\n' >> summary-count.txt; printf AFTER-SUMMARY"]}),
                    "response-coverage-after",
                ),
                false,
            ),
            6 => (
                text_response("Later result recorded.", "coverage-later"),
                false,
            ),
            7 => {
                for marker in ["BEFORE-SUMMARY-3", "AFTER-SUMMARY"] {
                    assert!(
                        wire["input"].to_string().contains(marker),
                        "{marker} must reach actual main wire"
                    );
                }
                (
                    text_response("Summary and later result consumed.", "coverage-final"),
                    true,
                )
            }
            _ => panic!("unexpected summary coverage main request {index}"),
        }
    });
    let config = write_context_window_config(root.path(), &server.base_url, 1_048_576);
    fs::write(&config, format!("{}\n[runtime]\nauto_compact = true\n[runtime.context_budget]\npressure_bps = 1000\ntarget_bps = 250\n", fs::read_to_string(&config).unwrap())).unwrap();
    let output = run_chat(
        root.path(),
        &config,
        "Execute the summary coverage task.\nExecute earlier observations.\nExecute the later observation.\nSUMMARY-ONLY continue.\n/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.join();
    let session = session_id(root.path());
    let events = session_events(root.path(), &session);
    let selected = events
        .iter()
        .rev()
        .find(|event| {
            event["event_type"] == "session.task_context_selected"
                && event["payload"]["selected_summary"].is_string()
        })
        .expect("the continuation must select the summary");
    let selected_summary = selected["payload"]["selected_summary"].as_str().unwrap();
    let compaction = events
        .iter()
        .rev()
        .find(|event| {
            event["event_type"] == "session.compaction_created"
                && event["payload"]["summary"]["fingerprint"]
                    .as_str()
                    .is_some_and(|fingerprint| format!("summary:{fingerprint}") == selected_summary)
        })
        .expect("selected summary must match an original compaction");
    let summary = &compaction["payload"]["summary"];
    assert_eq!(summary["schema_version"], 7);
    let summarized = summary
        .get("summarized_history")
        .map(|value| {
            value
                .as_array()
                .expect("summary coverage must be an array")
                .as_slice()
        })
        .unwrap_or(&[]);
    let result_event = |call: &str| {
        events
            .iter()
            .find(|event| {
                event["event_type"] == "session.history_item_recorded"
                    && event["payload"]["item"]["result"]["call_id"] == call
            })
            .unwrap()
    };
    let before = result_event("coverage-before-3");
    let after = result_event("coverage-after");
    assert!(
        compaction["payload"]["retained_event_ids"]
            .as_array()
            .unwrap()
            .contains(&before["event_id"]),
        "latest complete batch must really be retained"
    );
    assert!(
        !summarized.contains(&before["event_id"]),
        "retained result must not be summarized"
    );
    assert!(after["sequence"].as_u64().unwrap() > compaction["sequence"].as_u64().unwrap());
    assert!(!summarized.contains(&after["event_id"]));
    assert!(
        selected["payload"]["selected_history"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        selected["payload"]["decision"]["required_context_refs"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let state = reduced_session(&Project::discover(root.path()).unwrap(), &session);
    let audit = state.request_audits().last().unwrap();
    let map = serde_json::to_string(audit.context_delivery_map().unwrap()).unwrap();
    for call in ["coverage-before-3", "coverage-after"] {
        let receipt = events
            .iter()
            .find(|event| {
                event["event_type"] == "tool.execution_outcome_recorded"
                    && event["payload"]["result"]["call_id"] == call
            })
            .unwrap();
        assert!(
            map.contains(receipt["event_id"].as_str().unwrap()),
            "missing original receipt for {call}"
        );
    }
    assert_audits_reconstruct(root.path(), &state);
    let resumed = run_chat(root.path(), &config, "/exit\n", &["--resume", "latest"]);
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    assert_eq!(
        fs::read_to_string(root.path().join("summary-count.txt")).unwrap(),
        "BEFORE-SUMMARY-1\nBEFORE-SUMMARY-2\nBEFORE-SUMMARY-3\nAFTER-SUMMARY\n"
    );
}

#[test]
fn selected_summary_survives_repeated_same_turn_compactions_and_resume() {
    use super::task_context::selector_candidates;
    use super::task_context_paging::paged_server;

    let root = TemporaryDirectory::new("task-progress-repeated-summary");
    let effects = TemporaryDirectory::new("task-progress-repeated-effects");
    let count_path = effects.path().join("count.txt");
    let count_for_server = count_path.clone();
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let mut main_count = 0;
    let mut handoff_count = 0;
    let server = paged_server(move |_step, wire| {
        if wire["instructions"]
            .as_str()
            .unwrap_or("")
            .contains("continuation handoff")
        {
            handoff_count += 1;
            return (
                compaction_summary_response(
                    "REPEATED-SUMMARY: consult the original task and its real observations.",
                    &format!("repeated-handoff-{handoff_count}"),
                ),
                false,
            );
        }
        if wire.get("tools").is_none() {
            let candidates = selector_candidates(wire);
            if candidates["tasks"].as_array().unwrap().is_empty() {
                return (
                    super::task_context::initial_selection_response(
                        wire,
                        "repeated-initial-selection",
                    ),
                    false,
                );
            }
            let summary = candidates["context"]
                .as_array()
                .unwrap()
                .iter()
                .find(|unit| {
                    unit["ref"]
                        .as_str()
                        .is_some_and(|value| value.starts_with("summary:"))
                })
                .expect("the existing task summary must be offered");
            let decision = json!({"relation":"continue", "focus":{"kind":"existing", "task_ref":candidates["tasks"][0]["ref"]},
                "related_task_refs":[], "required_context_refs":[summary["ref"]], "unresolved_references":[]});
            return (
                text_response(&decision.to_string(), "repeated-selection"),
                false,
            );
        }
        let index = main_count;
        main_count += 1;
        match index {
            0 => (
                text_response(
                    &format!(
                        "Original task details {}",
                        "historical answer ".repeat(10_000)
                    ),
                    "repeated-original",
                ),
                false,
            ),
            1..=5 => {
                let reasoning = json!({"type":"response.output_item.done", "output_index":0,
                    "item":{"type":"reasoning", "content":[{"type":"reasoning_text",
                        "text":format!("repeated-{index}-reasoning ").repeat(20_000)}]}});
                let call = format!("repeated-batch-{index}");
                let command = format!(
                    "printf '{index}\\n' >> '{}'; printf 'RESULT-{index} '; printf '%s' \"$1\"",
                    count_for_server.display()
                );
                (
                    format!(
                        "data: {reasoning}\n\n{}",
                        m4_long_flow_tool_response(
                            &call,
                            "run_command",
                            json!({"argv":["/bin/sh","-c",command,"fixture","neutral-output ".repeat(2_000)]}),
                            &format!("repeated-tool-{index}")
                        )
                    ),
                    false,
                )
            }
            6 => (
                text_response("All synthetic batches finished.", "repeated-final"),
                true,
            ),
            _ => panic!("unexpected repeated-compaction main request {index}"),
        }
    });
    let config = write_context_window_config(root.path(), &server.base_url, 1_048_576);
    fs::write(&config, format!("{}\n[runtime]\nauto_compact = true\n[runtime.context_budget]\npressure_bps = 1000\ntarget_bps = 250\n", fs::read_to_string(&config).unwrap())).unwrap();
    let output = run_chat(
        root.path(),
        &config,
        "Execute the original summary task.\n/compact\nContinue the original task using its selected summary.\n/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.join();
    let id = session_id(root.path());
    let events = session_events(root.path(), &id);
    let selection = events
        .iter()
        .find(|event| {
            event["event_type"] == "session.task_context_selected"
                && event["payload"]["selected_summary"].is_string()
        })
        .expect("one selection must bind S1");
    let selected_ref = selection["payload"]["selected_summary"].as_str().unwrap();
    let subsequent = events
        .iter()
        .filter(|event| {
            event["event_type"] == "session.compaction_created"
                && event["sequence"].as_u64().unwrap() > selection["sequence"].as_u64().unwrap()
        })
        .collect::<Vec<_>>();
    assert!(
        subsequent.len() >= 2,
        "S1 must be followed by S2 and another actual compaction"
    );
    for compaction in &subsequent {
        assert_eq!(
            compaction["payload"]["summary"]["task_selection_event"],
            selection["event_id"]
        );
        assert_ne!(
            format!(
                "summary:{}",
                compaction["payload"]["summary"]["fingerprint"]
                    .as_str()
                    .unwrap()
            ),
            selected_ref
        );
        let receipt = events
            .iter()
            .rev()
            .find(|event| {
                event["event_type"] == "tool.execution_outcome_recorded"
                    && event["sequence"].as_u64().unwrap()
                        < compaction["sequence"].as_u64().unwrap()
                    && event["payload"]["result"]["call_id"]
                        .as_str()
                        .is_some_and(|id| id.starts_with("repeated-batch-"))
            })
            .expect("the latest complete Tool batch has its real receipt");
        let call = &receipt["payload"]["result"]["call_id"];
        let result = events
            .iter()
            .find(|event| {
                event["event_type"] == "session.history_item_recorded"
                    && event["payload"]["item"]["result"]["call_id"] == *call
            })
            .unwrap();
        assert!(
            compaction["payload"]["retained_event_ids"]
                .as_array()
                .unwrap()
                .contains(&result["event_id"]),
            "latest complete Tool result must be retained"
        );
        assert!(
            compaction["payload"]["summary"]["task_references"]["execution_receipt_refs"]
                .as_array()
                .unwrap()
                .contains(&receipt["event_id"]),
            "latest complete Tool result must retain its original receipt"
        );
    }
    assert_eq!(
        events
            .iter()
            .filter(
                |event| event["event_type"] == "tool.execution_outcome_recorded"
                    && event["payload"]["result"]["call_id"]
                        .as_str()
                        .is_some_and(|id| id.starts_with("repeated-batch-"))
            )
            .count(),
        5
    );
    let state = reduced_session(&Project::discover(root.path()).unwrap(), &id);
    assert_audits_reconstruct(root.path(), &state);
    let resumed = run_chat(root.path(), &config, "/exit\n", &["--resume", "latest"]);
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    assert_eq!(fs::read_to_string(count_path).unwrap(), "1\n2\n3\n4\n5\n");
}

fn choose_one_memory(candidates: &Value) -> Value {
    let reference = candidates["context"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| {
            entry["ref"]
                .as_str()
                .unwrap()
                .starts_with("memory:notes/alpha.md:")
        })
        .unwrap()["ref"]
        .as_str()
        .unwrap();
    json!({"relation":"new", "focus":{"kind":"new_from_input", "input_ref":candidates["input"]["ref"]},
        "related_task_refs":[], "required_context_refs":[reference], "unresolved_references":[]})
}

#[test]
fn three_non_core_memory_writes_refresh_without_resampling_or_replay() {
    let root = TemporaryDirectory::new("task-updated-memory");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let manifest = agent.join("agent.toml");
    fs::write(
        &manifest,
        format!(
            "{}memory_write = \"direct\"\n",
            fs::read_to_string(&manifest)
                .unwrap()
                .replace("memory = \"none\"", "memory = \"project\"")
        ),
    )
    .unwrap();
    let notes = root.path().join(".echidna/memory/notes");
    fs::create_dir_all(&notes).unwrap();
    let old = "ALPHA-ORIGINAL-VALUE";
    fs::write(notes.join("alpha.md"), old).unwrap();
    let old_hash = blake3::hash(old.as_bytes()).to_hex().to_string();
    let first_hash = blake3::hash(b"ALPHA-UPDATED-ONE").to_hex().to_string();
    let second_hash = blake3::hash(b"ALPHA-UPDATED-TWO").to_hex().to_string();
    let server = spawn_scripted_responses_server(vec![
        ScriptedResponse::SelectionDecision(choose_one_memory),
        ScriptedResponse::Response(
            200,
            m4_long_flow_tool_response(
                "update-alpha-1",
                "memory_write",
                json!({
                    "path":"notes/alpha.md", "content":"ALPHA-UPDATED-ONE", "expected_hash":old_hash, "event_id":"alpha-updated-1"
                }),
                "memory-update",
            ),
        ),
        ScriptedResponse::Response(
            200,
            m4_long_flow_tool_response(
                "update-alpha-2",
                "memory_write",
                json!({
                    "path":"notes/alpha.md", "content":"ALPHA-UPDATED-TWO", "expected_hash":first_hash, "event_id":"alpha-updated-2"
                }),
                "memory-update-2",
            ),
        ),
        ScriptedResponse::Response(
            200,
            m4_long_flow_tool_response(
                "update-alpha-3",
                "memory_write",
                json!({
                    "path":"notes/alpha.md", "content":"ALPHA-UPDATED-THREE", "expected_hash":second_hash, "event_id":"alpha-updated-3"
                }),
                "memory-update-3",
            ),
        ),
        ScriptedResponse::Response(
            200,
            text_response("Updated source used.", "memory-update-done"),
        ),
    ]);
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "Update the alpha note and continue.\n/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let wires = (0..5)
        .map(|_| f2a_request_body(&server.requests, "updated task Memory"))
        .collect::<Vec<_>>();
    assert!(server.requests.try_recv().is_err());
    server.join();
    assert!(json_contains_text(&wires[1]["input"], old));
    for (index, value) in [
        (2, "ALPHA-UPDATED-ONE"),
        (3, "ALPHA-UPDATED-TWO"),
        (4, "ALPHA-UPDATED-THREE"),
    ] {
        assert!(json_contains_text(&wires[index]["input"], value));
        assert!(
            wires[index]
                .get("previous_response_id")
                .is_none_or(Value::is_null)
        );
    }
    let state = reduced_session(
        &Project::discover(root.path()).unwrap(),
        &session_id(root.path()),
    );
    assert_eq!(state.execution_receipt_count(), 3);
    let events = session_events(root.path(), &session_id(root.path()));
    assert_eq!(
        events
            .iter()
            .filter(|event| event["event_type"] == "session.task_selection_sample_recorded")
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(
                |event| event["event_type"] == "session.task_context_selected"
                    && !event["payload"]["refresh_of"].is_null()
            )
            .count(),
        3
    );
    assert_audits_reconstruct(root.path(), &state);
    let resumed = run_chat(
        root.path(),
        &config,
        "/context\n/exit\n",
        &["--resume", "latest"],
    );
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
}

#[test]
fn selected_non_core_memory_is_frozen_scoped_and_excluded_on_task_switch() {
    let root = TemporaryDirectory::new("task-selected-memory");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let manifest = agent.join("agent.toml");
    fs::write(
        &manifest,
        fs::read_to_string(&manifest)
            .unwrap()
            .replace("memory = \"none\"", "memory = \"project\""),
    )
    .unwrap();
    let notes = root.path().join(".echidna/memory/notes");
    fs::create_dir_all(&notes).unwrap();
    fs::write(notes.join("alpha.md"), "ALPHA-SCOPED-MEMORY").unwrap();
    fs::write(notes.join("beta.md"), "BETA-UNRELATED-MEMORY").unwrap();
    let server = spawn_scripted_responses_server(vec![
        ScriptedResponse::SelectionDecision(choose_one_memory),
        ScriptedResponse::Response(200, text_response("Alpha done.", "memory-alpha")),
        ScriptedResponse::NewSelection,
        ScriptedResponse::Response(200, text_response("Beta done.", "memory-beta")),
    ]);
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "Use the alpha note.\nStart a completely unrelated task.\n/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let wires = (0..4)
        .map(|_| f2a_request_body(&server.requests, "selected task Memory"))
        .collect::<Vec<_>>();
    assert!(server.requests.try_recv().is_err());
    server.join();
    assert!(json_contains_text(
        &wires[1]["input"],
        "ALPHA-SCOPED-MEMORY"
    ));
    assert!(!json_contains_text(
        &wires[1]["input"],
        "BETA-UNRELATED-MEMORY"
    ));
    assert!(!json_contains_text(
        &wires[3]["input"],
        "ALPHA-SCOPED-MEMORY"
    ));
    assert!(
        wires[3]
            .get("previous_response_id")
            .is_none_or(Value::is_null)
    );
    let state = reduced_session(
        &Project::discover(root.path()).unwrap(),
        &session_id(root.path()),
    );
    assert_audits_reconstruct(root.path(), &state);
}

#[test]
fn continuing_task_automatically_keeps_bounded_progress_and_original_receipt() {
    let root = TemporaryDirectory::new("task-auto-progress");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let server = spawn_scripted_responses_server(vec![
        ScriptedResponse::InitialSelection,
        ScriptedResponse::Response(
            200,
            m4_long_flow_tool_response(
                "progress-once",
                "run_command",
                json!({"argv":["/bin/sh","-c","printf 'PROGRESS-RESULT'; printf 'once\\n' >> progress-count.txt"]}),
                "progress-tool",
            ),
        ),
        ScriptedResponse::Response(200, text_response("First step complete.", "progress-first")),
        ScriptedResponse::SelectionDecision(continue_without_history),
        ScriptedResponse::Response(
            200,
            text_response("Continuing from recorded progress.", "progress-second"),
        ),
    ]);
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "Run the counted first step.\nContinue that same task.\n/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let wires = (0..5)
        .map(|_| f2a_request_body(&server.requests, "automatic task progress"))
        .collect::<Vec<_>>();
    assert!(server.requests.try_recv().is_err());
    server.join();
    assert!(json_contains_text(
        &wires[4]["input"],
        "Source-bound task progress"
    ));
    assert!(json_contains_text(&wires[4]["input"], "PROGRESS-RESULT"));
    assert_eq!(
        fs::read_to_string(root.path().join("progress-count.txt")).unwrap(),
        "once\n"
    );
    let project = Project::discover(root.path()).unwrap();
    let id = session_id(root.path());
    let events = session_events(root.path(), &id);
    let receipt = events
        .iter()
        .find(|event| event["event_type"] == "tool.execution_outcome_recorded")
        .unwrap()["event_id"]
        .as_str()
        .unwrap();
    let result = events
        .iter()
        .find(|event| {
            event["event_type"] == "session.history_item_recorded"
                && event["payload"]["item"]["kind"] == "tool_result"
                && event["payload"]["item"]
                    .to_string()
                    .contains("PROGRESS-RESULT")
        })
        .unwrap()["event_id"]
        .as_str()
        .unwrap();
    let state = reduced_session(&project, &id);
    assert_eq!(state.execution_receipt_count(), 1);
    let map = serde_json::to_value(
        state
            .request_audits()
            .last()
            .unwrap()
            .context_delivery_map()
            .unwrap(),
    )
    .unwrap();
    let progress = map["content_items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["provenance"]["freshness"]["producer_version"] == "task-progress:1")
        .expect("progress is source-bound in the delivery map");
    assert_eq!(
        progress["provenance"]["authority_level"],
        "derived_non_evidence"
    );
    assert!(
        progress["provenance"]["references"]
            .to_string()
            .contains(result)
    );
    assert!(
        progress["provenance"]["references"]
            .to_string()
            .contains(receipt)
    );
}
