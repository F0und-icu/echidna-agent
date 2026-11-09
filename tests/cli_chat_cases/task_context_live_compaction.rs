//! Settled-batch compaction preserves real live-process and refreshed sources.
use super::task_context::definition;
use super::task_context::selector_candidates;
use super::task_context_paging::{delivered_result, paged_server_with_request_waits};
use super::*;

struct ProcessCleanup(std::path::PathBuf);

impl Drop for ProcessCleanup {
    fn drop(&mut self) {
        if let Ok(pid) = fs::read_to_string(&self.0)
            && let Ok(pid) = pid.trim().parse::<u32>()
        {
            let _ = Command::new("/bin/kill")
                .args(["-TERM", &pid.to_string()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
}

fn assert_live_handle(
    wire: &Value,
    handle: &str,
    pid_path: &std::path::Path,
    acknowledged: Instant,
    message: &str,
) {
    if wire.to_string().contains(handle) {
        return;
    }
    let pid = fs::read_to_string(pid_path).ok();
    let process_alive = pid.as_ref().and_then(|pid| {
        Command::new("/bin/kill")
            .args(["-0", pid.trim()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .ok()
            .map(|status| status.success())
    });
    panic!(
        "{message}; elapsed_since_ack_ms={}; fixture_pid={pid:?}; process_alive={process_alive:?}",
        acknowledged.elapsed().as_millis()
    );
}

#[test]
fn running_job_and_pty_remain_source_bound_after_lossy_same_turn_compaction() {
    for (kind, key) in [("job", "job_id"), ("pty", "pty_session_id")] {
        let root = TemporaryDirectory::new("task-live-compaction-project");
        let markers = TemporaryDirectory::new("task-live-compaction-markers");
        initialize_project(root.path());
        let agent = definition(root.path(), None);
        let manifest = agent.join("agent.toml");
        fs::write(
            &manifest,
            format!(
                "{}start_{kind} = \"direct\"\nstop_{kind} = \"direct\"\n",
                fs::read_to_string(&manifest).unwrap()
            ),
        )
        .unwrap();
        let count_path = markers.path().join("process-count");
        let pid_path = markers.path().join("process-pid");
        let bulk_path = markers.path().join("bulk-count");
        let _cleanup = ProcessCleanup(pid_path.clone());
        let process = format!(
            "printf 'start\\n' >> '{}'; printf '%s' $$ > '{}'; exec sleep 90",
            count_path.display(),
            pid_path.display()
        );
        let bulk_one = format!(
            "printf 'one\\n' >> '{}'; printf '%s' \"$1\"",
            bulk_path.display()
        );
        let bulk_two = format!(
            "printf 'two\\n' >> '{}'; printf '%s' \"$1\"",
            bulk_path.display()
        );
        let cleanup_pid = pid_path.clone();
        let mut main = 0;
        let mut handle = String::new();
        let mut handoffs = 0;
        let mut acknowledged = None;
        let server = paged_server_with_request_waits(
            Duration::from_secs(60),
            Duration::from_secs(60),
            move |_, wire| {
                if wire["instructions"]
                    .as_str()
                    .is_some_and(|value| value.contains("continuation handoff"))
                {
                    handoffs += 1;
                    return (
                        compaction_summary_response(
                            "A synthetic command batch completed.",
                            &format!("live-handoff-{handoffs}"),
                        ),
                        false,
                    );
                }
                if wire.get("tools").is_none() {
                    return (
                        super::task_context::initial_selection_response(
                            wire,
                            "live-initial-selection",
                        ),
                        false,
                    );
                }
                let step = main;
                main += 1;
                match step {
                    0 => (
                        m4_long_flow_tool_response(
                            "start-once",
                            &format!("start_{kind}"),
                            json!({"argv":["/bin/sh","-c",process],"resources":[{"kind":"output","path":"background-resource"}]}),
                            "live-start",
                        ),
                        false,
                    ),
                    1 => {
                        let result = delivered_result(wire, "start-once");
                        assert_eq!(result["status"], "succeeded", "{result}");
                        handle = result["output"][key].as_str().unwrap().to_owned();
                        let deadline = Instant::now() + Duration::from_secs(5);
                        while !cleanup_pid.exists() {
                            assert!(
                                Instant::now() < deadline,
                                "the synthetic process did not start"
                            );
                            thread::sleep(Duration::from_millis(10));
                        }
                        acknowledged = Some(Instant::now());
                        let reasoning = json!({"type":"response.output_item.done", "output_index":0,
                        "item":{"type":"reasoning", "content":[{"type":"reasoning_text", "text":"live-reasoning ".repeat(22_000)}]}});
                        let call = m4_long_flow_tool_response(
                            "bulk-one",
                            "run_command",
                            json!({"argv":["/bin/sh","-c",bulk_one,"fixture","neutral-output ".repeat(20_000)], "resources":[{"kind":"output","path":"bulk-resource"}]}),
                            "live-bulk-one",
                        );
                        (format!("data: {reasoning}\n\n{call}"), false)
                    }
                    2 => {
                        assert_live_handle(
                            wire,
                            &handle,
                            &cleanup_pid,
                            acknowledged.expect("start was acknowledged"),
                            "the next main must receive the live handle",
                        );
                        let reasoning = json!({"type":"response.output_item.done", "output_index":0,
                        "item":{"type":"reasoning", "content":[{"type":"reasoning_text", "text":"live-reasoning ".repeat(22_000)}]}});
                        let call = m4_long_flow_tool_response(
                            "bulk-two",
                            "run_command",
                            json!({"argv":["/bin/sh","-c",bulk_two,"fixture","neutral-output ".repeat(20_000)], "resources":[{"kind":"output","path":"bulk-resource"}]}),
                            "live-bulk-two",
                        );
                        (format!("data: {reasoning}\n\n{call}"), false)
                    }
                    3 => {
                        assert!(handoffs > 0, "the fixture must exercise a model handoff");
                        assert!(
                            wire["previous_response_id"].is_null(),
                            "compaction must rebuild a Full request"
                        );
                        assert_live_handle(
                            wire,
                            &handle,
                            &cleanup_pid,
                            acknowledged.expect("start was acknowledged"),
                            "lossy compaction must not discard the running handle",
                        );
                        (
                            m4_long_flow_tool_response(
                                "stop-once",
                                &format!("stop_{kind}"),
                                json!({key:handle}),
                                "live-stop",
                            ),
                            false,
                        )
                    }
                    4 => {
                        let result = delivered_result(wire, "stop-once");
                        assert_eq!(result["status"], "succeeded", "{result}");
                        fs::remove_file(&cleanup_pid).unwrap();
                        (
                            text_response("Synthetic process stopped.", "live-done"),
                            true,
                        )
                    }
                    _ => panic!("unexpected live process request"),
                }
            },
        );
        let config = write_context_window_config(root.path(), &server.base_url, 1_048_576);
        fs::write(&config, format!("{}\n[runtime]\nauto_compact = true\n[runtime.context_budget]\npressure_bps = 1000\ntarget_bps = 500\n", fs::read_to_string(&config).unwrap())).unwrap();
        let output = run_chat(
            root.path(),
            &config,
            "Start one background observation, inspect two outputs, then stop it.\n/exit\n",
            &["--agent", agent.to_str().unwrap()],
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        server.join();
        assert_eq!(fs::read_to_string(count_path).unwrap(), "start\n");
        assert_eq!(fs::read_to_string(bulk_path).unwrap(), "one\ntwo\n");
        let id = session_id(root.path());
        let events = session_events(root.path(), &id);
        let start = events
            .iter()
            .find(|event| {
                event["event_type"] == "session.history_item_recorded"
                    && event["payload"]["item"]["kind"] == "tool_result"
                    && event["payload"]["item"]["result"]["call_id"] == "start-once"
            })
            .unwrap();
        let receipt = events
            .iter()
            .find(|event| {
                event["event_type"] == "tool.execution_outcome_recorded"
                    && event["payload"]["result"]["call_id"] == "start-once"
            })
            .unwrap();
        let handle = start["payload"]["item"]["result"]["output"][key]
            .as_str()
            .unwrap();
        let compaction_index = events
            .iter()
            .position(|event| {
                event["event_type"] == "session.compaction_created"
                    && !event["payload"]["retained_event_ids"]
                        .as_array()
                        .unwrap()
                        .contains(&start["event_id"])
                    && !event["payload"]["summary"].to_string().contains(handle)
            })
            .expect("an actual lossy compaction must omit the original start result and handle");
        let audit = events[compaction_index + 1..]
            .iter()
            .find(|event| {
                event["event_type"] == "session.request_audit_recorded"
                    && event["payload"]["full_context_delivery_map"]["content_items"]
                        .as_array()
                        .is_some_and(|items| {
                            items.iter().any(|item| {
                                item["fragment_id"]
                                    .as_str()
                                    .is_some_and(|id| id.starts_with("task-live-processes:"))
                            })
                        })
            })
            .expect("a later main audit must include live-source provenance");
        let live = audit["payload"]["full_context_delivery_map"]["content_items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| {
                item["fragment_id"]
                    .as_str()
                    .is_some_and(|id| id.starts_with("task-live-processes:"))
            })
            .unwrap();
        assert!(
            live["provenance"]["references"]["source_event_refs"]
                .as_array()
                .unwrap()
                .contains(&start["event_id"])
        );
        assert!(
            live["provenance"]["references"]["execution_receipt_refs"]
                .as_array()
                .unwrap()
                .contains(&receipt["event_id"])
        );
        let resumed = run_chat(root.path(), &config, "/exit\n", &["--resume", "latest"]);
        assert!(
            resumed.status.success(),
            "{}",
            String::from_utf8_lossy(&resumed.stderr)
        );
    }
}

#[test]
fn repeated_selected_memory_changes_compact_before_refresh_without_replay() {
    for fail_handoff in [false, true] {
        let root = TemporaryDirectory::new("task-memory-pressure-project");
        let markers = TemporaryDirectory::new("task-memory-pressure-markers");
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
        fs::write(notes.join("alpha.md"), "ALPHA-PRESSURE-ORIGINAL").unwrap();
        let effects = markers.path().join("effects");
        let effects_command = format!(
            "printf 'once\\n' >> '{}'; printf '%s' \"$1\"",
            effects.display()
        );
        let mut hash = String::new();
        let mut main = 0;
        let mut handoffs = 0;
        let server = paged_server_with_request_waits(
            Duration::from_secs(60),
            Duration::from_secs(60),
            move |_, wire| {
                if wire["instructions"]
                    .as_str()
                    .is_some_and(|value| value.contains("continuation handoff"))
                {
                    handoffs += 1;
                    if fail_handoff {
                        return (
                            format!(
                                "data: {}\n\n",
                                json!({"type":"response.failed","response":{"error":{"code":"fixture_failure","message":"synthetic handoff failure"}}})
                            ),
                            false,
                        );
                    }
                    return (
                        compaction_summary_response(
                            "Settled synthetic batches completed.",
                            &format!("memory-handoff-{handoffs}"),
                        ),
                        false,
                    );
                }
                if wire.get("tools").is_none() {
                    let candidates = selector_candidates(wire);
                    let reference = candidates["context"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find_map(|entry| {
                            entry["ref"]
                                .as_str()
                                .filter(|value| value.starts_with("memory:notes/alpha.md:"))
                        })
                        .unwrap();
                    hash = reference.rsplit_once(':').unwrap().1.to_owned();
                    return (text_response(&json!({"relation":"new","focus":{"kind":"new_from_input","input_ref":candidates["input"]["ref"]},"related_task_refs":[],"required_context_refs":[reference],"unresolved_references":[]}).to_string(), "memory-selection"), false);
                }
                if main > 0 {
                    let result = delivered_result(wire, &format!("memory-write-{main}"));
                    assert_eq!(result["status"], "succeeded", "{result}");
                    hash = result["output"]["new_hash"].as_str().unwrap().to_owned();
                    assert!(
                        wire.to_string()
                            .contains(&format!("ALPHA-PRESSURE-VERSION-{main}")),
                        "the next main must use the refreshed Memory revision"
                    );
                } else {
                    assert!(wire.to_string().contains("ALPHA-PRESSURE-ORIGINAL"));
                }
                if main == 3 {
                    return (
                        text_response("Synthetic updates complete.", "memory-done"),
                        true,
                    );
                }
                main += 1;
                let calls = [
                    (
                        format!("memory-write-{main}"),
                        "memory_write",
                        json!({"path":"notes/alpha.md","content":format!("ALPHA-PRESSURE-VERSION-{main}"), "expected_hash":hash,"event_id":format!("memory-pressure-{main}")}),
                    ),
                    (
                        format!("memory-bulk-{main}"),
                        "run_command",
                        json!({"argv":["/bin/sh","-c",effects_command,"fixture","neutral-output ".repeat(2_000)],"resources":[{"kind":"output","path":"bulk-resource"}]}),
                    ),
                ];
                let calls = calls
                    .iter()
                    .map(|(id, name, args)| (id.as_str(), *name, args.clone()))
                    .collect::<Vec<_>>();
                let reasoning = json!({"type":"response.output_item.done","output_index":0,"item":{"type":"reasoning","content":[{"type":"reasoning_text","text":format!("memory-batch-{main}-reasoning ").repeat(18_000)}]}});
                (
                    format!(
                        "data: {reasoning}\n\n{}",
                        tool_calls_response(&calls, &format!("memory-batch-{main}"))
                    ),
                    false,
                )
            },
        );
        let config = write_context_window_config(root.path(), &server.base_url, 1_048_576);
        fs::write(&config, format!("{}\n[runtime]\nauto_compact = true\n[runtime.context_budget]\npressure_bps = 50\ntarget_bps = 25\n", fs::read_to_string(&config).unwrap())).unwrap();
        let output = run_chat(
            root.path(),
            &config,
            "Update the selected synthetic note three times and inspect each result.\n/exit\n",
            &["--agent", agent.to_str().unwrap()],
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        server.join();
        assert_eq!(fs::read_to_string(&effects).unwrap(), "once\nonce\nonce\n");
        assert_eq!(
            fs::read_to_string(notes.join("alpha.md")).unwrap(),
            "ALPHA-PRESSURE-VERSION-3"
        );
        let id = session_id(root.path());
        let events = session_events(root.path(), &id);
        assert_eq!(
            events
                .iter()
                .filter(|event| event["event_type"] == "tool.execution_outcome_recorded")
                .count(),
            6
        );
        let refreshes = events
            .iter()
            .filter(|event| {
                event["event_type"] == "session.task_context_selected"
                    && !event["payload"]["refresh_of"].is_null()
            })
            .count();
        assert_eq!(refreshes, 3);
        let compactions = events
            .iter()
            .filter(|event| event["event_type"] == "session.compaction_created")
            .count();
        if fail_handoff {
            assert_eq!(
                compactions, 0,
                "a failed handoff cannot install a compaction"
            );
        } else {
            assert!(
                compactions > 0,
                "verified prior selection should allow pressured settled batches to compact"
            );
        }
        let project = Project::discover(root.path()).unwrap();
        let state = reduced_session(&project, &id);
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
        let resumed_server = paged_server_with_request_waits(
            Duration::from_secs(60),
            Duration::from_secs(60),
            |index, wire| {
                assert!(
                    wire["previous_response_id"].is_null(),
                    "process Resume must establish fresh Provider delivery"
                );
                if index == 0 {
                    let candidates = selector_candidates(wire);
                    let source = candidates["context"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|source| {
                            source["ref"].as_str().is_some_and(|reference| {
                                reference.starts_with("memory:notes/alpha.md:")
                            })
                        })
                        .expect("the current selected note must remain a candidate");
                    assert!(source.to_string().contains("ALPHA-PRESSURE-VERSION-3"));
                    let decision = json!({
                        "relation":"continue",
                        "focus":{"kind":"existing","task_ref":candidates["tasks"][0]["ref"]},
                        "related_task_refs":[],
                        "required_context_refs":[source["ref"]],
                        "unresolved_references":[]
                    });
                    return (
                        text_response(&decision.to_string(), "memory-resume-selection"),
                        false,
                    );
                }
                assert_eq!(index, 1, "Resume must not replay prior Tool batches");
                assert!(wire.get("tools").is_some());
                assert!(wire.to_string().contains("ALPHA-PRESSURE-VERSION-3"));
                assert!(
                    wire.to_string()
                        .contains("Describe the current selected note.")
                );
                (
                    text_response("The note is at version three.", "memory-resume-main"),
                    true,
                )
            },
        );
        let resumed_config =
            write_context_window_config(root.path(), &resumed_server.base_url, 1_048_576);
        let continued = run_chat(
            root.path(),
            &resumed_config,
            "Describe the current selected note.\n/exit\n",
            &["--resume", &id],
        );
        assert!(
            continued.status.success(),
            "{}",
            String::from_utf8_lossy(&continued.stderr)
        );
        for _ in 0..2 {
            f2a_request_body(&resumed_server.requests, "post-Resume Memory continuation");
        }
        assert!(resumed_server.requests.try_recv().is_err());
        resumed_server.join();
        assert_eq!(fs::read_to_string(&effects).unwrap(), "once\nonce\nonce\n");
        assert_eq!(
            fs::read_to_string(notes.join("alpha.md")).unwrap(),
            "ALPHA-PRESSURE-VERSION-3"
        );
        let resumed_events = session_events(root.path(), &id);
        let outcomes = |values: &[Value]| {
            values
                .iter()
                .filter(|event| event["event_type"] == "tool.execution_outcome_recorded")
                .cloned()
                .collect::<Vec<_>>()
        };
        assert_eq!(
            outcomes(&resumed_events),
            outcomes(&events),
            "all six real outcomes must retain their original identities after Resume"
        );
        let resumed_state = reduced_session(&project, &id);
        assert_eq!(
            resumed_state.history().ensure_all_tool_calls_paired(),
            Ok(())
        );
        assert_audits_reconstruct(root.path(), &resumed_state);
    }
}
