//! Synthetic cold sources exercise exact ranges without replaying old tools.
use super::task_context::{
    definition, initial_selection_response, new_selection_response, selector_candidates,
};
use super::*;
use echidna::domain::model::ContentItem;
use echidna::domain::tools::{ToolCall, ToolCallId, ToolName, ToolResult, ToolResultStatus};

pub(super) fn append_source_items(
    project: &Project,
    session: &str,
    items: Vec<ContentItem>,
) -> Vec<String> {
    let mut writer = project.open_writer().unwrap();
    let session_id = StreamId::new(session);
    let stream = ProjectEventStream::Session(session_id.clone()).stream_id();
    let mut store = writer.open_session(session_id, None).unwrap();
    let mut ids = Vec::new();
    for item in items {
        let id = format!("cold-source-{}", store.next_sequence().unwrap().get());
        let event = SessionEvent::HistoryItemRecorded { item }
            .into_envelope(
                EventId::new(&id),
                stream.clone(),
                store.next_sequence().unwrap(),
                OccurredAt::new("2026-11-05T00:00:00Z"),
            )
            .unwrap();
        EventSink::append_event(&mut store, &event).unwrap();
        ids.push(id);
    }
    ids
}

pub(super) fn peer_reference(session: &str, event: &str, item: &ContentItem) -> String {
    let revision = blake3::hash(&serde_json::to_vec(item).unwrap()).to_hex();
    format!("session-revision:{session}:{event}:{revision}")
}

#[test]
fn cold_large_tool_sources_keep_exact_json_ranges_and_original_pairing() {
    let root = TemporaryDirectory::new("task-context-large-cold-json");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let offline = write_context_window_config(root.path(), "http://127.0.0.1:1/v1", 272_000);
    let created = run_chat(
        root.path(),
        &offline,
        "/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(created.status.success());
    let prior = session_id(root.path());
    let project = Project::discover(root.path()).unwrap();
    // These are explicitly seeded historical observations, not execution
    // evidence. The current Agent must only read their original bytes.
    let source_items = vec![
        ContentItem::user_text("Archive the synthetic large source."),
        ContentItem::ToolCall {
            call: ToolCall {
                call_id: ToolCallId::new("historical-large-call"),
                name: ToolName::new("read_file"),
                arguments: json!({
                    "path":"synthetic-source.json",
                    "long_string":format!("{}CALL-END-EXACT", "字\"\\\n".repeat(50_000)),
                    "nested":vec![json!([null, [null, null]]); 20_000]
                }),
            },
        },
        ContentItem::ToolResult {
            result: ToolResult {
                call_id: ToolCallId::new("historical-large-call"),
                tool_name: ToolName::new("read_file"),
                status: ToolResultStatus::Succeeded,
                output: json!({
                    "long_string":format!("{}RESULT-END-EXACT", "值\"\\\n".repeat(50_000)),
                    "nested":vec![json!([null, [null, null]]); 20_000]
                }),
            },
        },
    ];
    let exact = source_items[1..]
        .iter()
        .map(|item| serde_json::to_string(item).unwrap())
        .collect::<Vec<_>>();
    let ids = append_source_items(&project, &prior, source_items);
    let before = fs::read(
        root.path()
            .join(".echidna/sessions")
            .join(&prior)
            .join("events.jsonl"),
    )
    .unwrap();
    assert_eq!(
        reduced_session(&project, &prior)
            .history()
            .ensure_all_tool_calls_paired(),
        Ok(())
    );
    let mut replies = Vec::new();
    let mut expected = Vec::new();
    for (index, marker) in ["CALL-END-EXACT", "RESULT-END-EXACT"].iter().enumerate() {
        for offset in [0, exact[index].find(marker).unwrap()] {
            let call = format!("cold-range-{}", expected.len());
            let reference = format!("session:{prior}:{}", ids[index + 1]);
            replies.push((200, m4_long_flow_tool_response(
                &call, "context_read",
                json!({"operation":"read","reference":reference,"offset":offset,"max_bytes":64}),
                &format!("range-response-{}", expected.len()),
            )));
            expected.push((call, reference, index, offset));
        }
    }
    replies.push((
        200,
        text_response("EXACT-COLD-RANGES-COMPLETE", "cold-final"),
    ));
    let mut scripted = vec![super::task_context::ScriptedResponse::NewSelection];
    scripted.extend(
        replies
            .into_iter()
            .map(|(status, body)| super::task_context::ScriptedResponse::Response(status, body)),
    );
    let server = super::task_context::spawn_scripted_responses_server(scripted);
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "Read the archived exact ranges.\n/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let wires = (0..6)
        .map(|_| f2a_request_body(&server.requests, "cold exact source range"))
        .collect::<Vec<_>>();
    assert!(server.requests.try_recv().is_err());
    server.join();
    for (wire, (call, reference, index, offset)) in wires[2..].iter().zip(expected) {
        let item = wire["input"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["type"] == "function_call_output" && item["call_id"] == call)
            .unwrap();
        let result: Value = serde_json::from_str(item["output"].as_str().unwrap()).unwrap();
        assert_eq!(result["status"], "succeeded");
        let response = &result["output"];
        assert_eq!(response["status"], "found");
        let entry = &response["entries"][0];
        assert_eq!(entry["reference"], reference);
        assert_eq!(entry["offset"], offset);
        assert_eq!(entry["total_bytes"], exact[index].len());
        let mut end = offset + 64;
        while !exact[index].is_char_boundary(end) {
            end -= 1;
        }
        assert_eq!(entry["text"], &exact[index][offset..end]);
        assert_eq!(entry["next_offset"], end);
        assert_eq!(entry["truncated"], true);
    }
    assert_eq!(
        fs::read(
            root.path()
                .join(".echidna/sessions")
                .join(&prior)
                .join("events.jsonl")
        )
        .unwrap(),
        before
    );
    let current = session_ids(root.path())
        .into_iter()
        .find(|id| id != &prior)
        .unwrap();
    let state = reduced_session(&project, &current);
    assert_eq!(
        state.execution_receipt_count(),
        4,
        "only the four new read calls execute"
    );
    assert_eq!(state.history().ensure_all_tool_calls_paired(), Ok(()));
    assert_audits_reconstruct(root.path(), &state);
}

pub(super) fn paged_server(
    reply: impl FnMut(usize, &Value) -> (String, bool) + Send + 'static,
) -> LoopbackServer {
    paged_server_with_request_waits(F2A_WAIT, F2A_WAIT, reply)
}

pub(super) fn paged_server_with_request_waits(
    first_request_wait: Duration,
    subsequent_request_wait: Duration,
    reply: impl FnMut(usize, &Value) -> (String, bool) + Send + 'static,
) -> LoopbackServer {
    paged_server_with_limit(first_request_wait, subsequent_request_wait, 16, reply)
}

pub(super) fn paged_server_with_limit(
    first_request_wait: Duration,
    subsequent_request_wait: Duration,
    max_requests: usize,
    mut reply: impl FnMut(usize, &Value) -> (String, bool) + Send + 'static,
) -> LoopbackServer {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let (sender, requests) = mpsc::channel();
    let thread = thread::spawn(move || {
        for index in 0..max_requests {
            let wait = if index == 0 {
                first_request_wait
            } else {
                subsequent_request_wait
            };
            let deadline = Instant::now() + wait;
            let mut socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            Instant::now() < deadline,
                            "paging request {index} did not arrive"
                        );
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("paging fixture accept: {error}"),
                }
            };
            socket.set_nonblocking(false).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let request = read_http_request(&mut socket);
            let (body, done) = reply(index, &request_body(&request));
            sender.send(request).unwrap();
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            if done {
                return;
            }
        }
        panic!("fixture did not complete within {max_requests} explicitly scripted requests");
    });
    LoopbackServer {
        base_url: format!("http://{address}/v1"),
        requests,
        thread,
    }
}

pub(super) fn delivered_result(wire: &Value, call: &str) -> Value {
    let item = wire["input"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["type"] == "function_call_output" && item["call_id"] == call)
        .unwrap_or_else(|| panic!("missing result for {call}"));
    serde_json::from_str(item["output"].as_str().unwrap()).unwrap()
}

#[test]
fn cold_search_parallel_cursors_remain_independent_and_release_capacity() {
    let root = TemporaryDirectory::new("task-context-independent-searches");
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
    let mut expected = BTreeSet::new();
    for index in 0..12 {
        let name = format!("parallel-{index:02}.md");
        let text = format!("ALPHA529 BETA614 exact source {index:02}");
        fs::write(notes.join(&name), &text).unwrap();
        expected.insert(format!(
            "memory:notes/{name}:{}",
            blake3::hash(text.as_bytes()).to_hex()
        ));
    }
    let mut cursors = Vec::new();
    let mut seen = vec![BTreeSet::new(); 4];
    let server = paged_server(move |step, wire| {
        if step == 0 {
            return (
                initial_selection_response(wire, "parallel-initial-selection"),
                false,
            );
        }
        let step = step - 1;
        let search = |query: &str, cursor: Value, max_bytes| json!({"operation":"search", "query":query, "cursor":cursor, "max_bytes":max_bytes});
        let queries = ["ALPHA529", "ALPHA529", "BETA614", "BETA614"];
        let starts = [
            "parallel-start-0",
            "parallel-start-1",
            "parallel-start-2",
            "parallel-start-3",
        ];
        let resumes = [
            "parallel-resume-0",
            "parallel-resume-1",
            "parallel-resume-2",
            "parallel-resume-3",
        ];
        match step {
            0 => {
                let calls = (0..4)
                    .map(|i| {
                        (
                            starts[i],
                            "context_read",
                            search(queries[i], Value::Null, 1024 + i),
                        )
                    })
                    .collect::<Vec<_>>();
                (tool_calls_response(&calls, "parallel-starts"), false)
            }
            1 => {
                for i in 0..4 {
                    let result = delivered_result(wire, starts[i]);
                    assert_eq!(result["status"], "succeeded", "{result}");
                    let response = &result["output"];
                    let cursor = response["next_cursor"]
                        .as_str()
                        .expect("unfinished first page");
                    cursors.push(cursor.to_owned());
                    for entry in response["entries"].as_array().unwrap() {
                        let reference = entry["reference"].as_str().unwrap().to_owned();
                        assert!(expected.contains(&reference), "unexpected source: {entry}");
                        assert!(seen[i].insert(reference), "duplicate source: {entry}");
                    }
                    assert_eq!(seen[i].len(), 10);
                }
                assert_eq!(cursors.iter().collect::<BTreeSet<_>>().len(), 4);
                let calls = [
                    (
                        "parallel-wrong-query",
                        "context_read",
                        search("BETA614", json!(cursors[0]), 1024),
                    ),
                    (
                        "parallel-wrong-time",
                        "context_read",
                        json!({"operation":"search", "query":"ALPHA529", "cursor":cursors[1], "after_unix":0, "max_bytes":1024}),
                    ),
                    (
                        "parallel-full",
                        "context_read",
                        search("FIFTH927", Value::Null, 1024),
                    ),
                ];
                (
                    tool_calls_response(&calls, "parallel-invalid-and-full"),
                    false,
                )
            }
            2 => {
                for call in [
                    "parallel-wrong-query",
                    "parallel-wrong-time",
                    "parallel-full",
                ] {
                    let result = delivered_result(wire, call);
                    assert_eq!(result["status"], "failed", "{call}: {result}");
                    if call == "parallel-full" {
                        assert!(
                            result
                                .to_string()
                                .contains("too many unfinished context searches"),
                            "{result}"
                        );
                    }
                }
                (
                    m4_long_flow_tool_response(
                        resumes[0],
                        "context_read",
                        search(queries[0], json!(cursors[0]), 1024),
                        "resume-a",
                    ),
                    false,
                )
            }
            3..=5 => {
                let indices: &[usize] = match step {
                    3 => &[0],
                    4 => &[2],
                    5 => &[1, 3],
                    _ => unreachable!(),
                };
                for &i in indices {
                    let result = delivered_result(wire, resumes[i]);
                    assert_eq!(result["status"], "succeeded", "{result}");
                    let response = &result["output"];
                    assert!(response["next_cursor"].is_null(), "{response}");
                    for entry in response["entries"].as_array().unwrap() {
                        let reference = entry["reference"].as_str().unwrap().to_owned();
                        assert!(seen[i].insert(reference), "duplicate source: {entry}");
                    }
                    assert_eq!(seen[i], expected);
                }
                if step == 3 {
                    return (
                        m4_long_flow_tool_response(
                            resumes[2],
                            "context_read",
                            search(queries[2], json!(cursors[2]), 1026),
                            "resume-b",
                        ),
                        false,
                    );
                }
                if step == 4 {
                    let calls = [1, 3].map(|i| {
                        (
                            resumes[i],
                            "context_read",
                            search(queries[i], json!(cursors[i]), 1024 + i),
                        )
                    });
                    return (tool_calls_response(&calls, "parallel-resumes"), false);
                }
                let calls = [
                    (
                        "parallel-after-release",
                        "context_read",
                        search("ALPHA529", Value::Null, 1024),
                    ),
                    (
                        "parallel-consumed",
                        "context_read",
                        search("ALPHA529", json!(cursors[0]), 1024),
                    ),
                ];
                (tool_calls_response(&calls, "parallel-release"), false)
            }
            6 => {
                let released = delivered_result(wire, "parallel-after-release");
                assert_eq!(released["status"], "succeeded", "{released}");
                assert!(released["output"]["next_cursor"].is_string(), "{released}");
                let consumed = delivered_result(wire, "parallel-consumed");
                assert_eq!(consumed["status"], "failed", "{consumed}");
                (
                    text_response("INDEPENDENT-SEARCHES-COMPLETE", "parallel-done"),
                    true,
                )
            }
            _ => panic!("unexpected search fixture request {step}"),
        }
    });
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "Search the synthetic archive.\n/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.join();
    assert!(String::from_utf8_lossy(&output.stdout).contains("INDEPENDENT-SEARCHES-COMPLETE"));
    let project = Project::discover(root.path()).unwrap();
    let state = reduced_session(&project, &session_id(root.path()));
    assert_eq!(state.history().ensure_all_tool_calls_paired(), Ok(()));
    assert_audits_reconstruct(root.path(), &state);
}

#[test]
fn cold_search_pages_beyond_hot_history_and_binds_cursor_to_query() {
    let root = TemporaryDirectory::new("task-context-cold-pages");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let offline = write_context_window_config(root.path(), "http://127.0.0.1:1/v1", 272_000);
    let created = run_chat(
        root.path(),
        &offline,
        "/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(created.status.success());
    let prior = session_id(root.path());
    let project = Project::discover(root.path()).unwrap();
    let early = "MARKER947NEEDLE early source, exact threshold 17.25.";
    let late = "MARKER947NEEDLE late source, exact threshold 23.50.";
    let items = (0..620)
        .map(|index| {
            ContentItem::assistant_text(match index {
                1 => early.to_owned(),
                600 => late.to_owned(),
                _ => format!("unrelated historical observation {index}"),
            })
        })
        .collect();
    let ids = append_source_items(&project, &prior, items);
    let early_ref = peer_reference(&prior, &ids[1], &ContentItem::assistant_text(early));
    let late_ref = peer_reference(&prior, &ids[600], &ContentItem::assistant_text(late));
    let source_path = root
        .path()
        .join(".echidna/sessions")
        .join(&prior)
        .join("events.jsonl");
    let before = fs::read(&source_path).unwrap();
    let mut last_call = String::new();
    let mut saved_cursor = Value::Null;
    let mut seen = BTreeSet::new();
    let mut empty_incomplete_page = false;
    let server = paged_server(move |step, wire| {
        if step == 0 {
            return (new_selection_response(wire, "archive-selection"), false);
        }
        let step = step - 1;
        let request = |call: &str, query: &str, cursor: Value| {
            m4_long_flow_tool_response(
                call,
                "context_read",
                json!({"operation":"search","query":query,"cursor":cursor,"max_bytes":1024}),
                &format!("page-response-{step}"),
            )
        };
        if step == 0 {
            last_call = "page-0".to_owned();
            return (request(&last_call, "MARKER947NEEDLE", Value::Null), false);
        }
        let result = delivered_result(wire, &last_call);
        if last_call == "bad-query" {
            assert!(
                result["status"] == "failed" || result["output"]["status"] == "stale",
                "cursor accepted a different query: {result}"
            );
            last_call = "bad-time".to_owned();
            return (
                m4_long_flow_tool_response(
                    &last_call,
                    "context_read",
                    json!({"operation":"search","query":"MARKER947NEEDLE","cursor":saved_cursor,"after_unix":0,"max_bytes":1024}),
                    "bad-time-response",
                ),
                false,
            );
        }
        if last_call == "bad-time" {
            assert!(
                result["status"] == "failed" || result["output"]["status"] == "stale",
                "cursor accepted a different time scope: {result}"
            );
            last_call = format!("page-{step}");
            return (
                request(&last_call, "MARKER947NEEDLE", saved_cursor.clone()),
                false,
            );
        }
        assert_eq!(result["status"], "succeeded", "{result}");
        let response = &result["output"];
        if last_call == "read-old-source" {
            assert_eq!(response["status"], "found");
            assert_eq!(response["entries"][0]["reference"], early_ref);
            assert_eq!(response["entries"][0]["text"], early);
            assert!(
                empty_incomplete_page,
                "the fixture must exercise a page without a matching record before the scan completes"
            );
            return (
                text_response("PAGED-COLD-READ-COMPLETE", "paging-final"),
                true,
            );
        }
        let entries = response["entries"].as_array().unwrap();
        for entry in entries {
            let reference = entry["reference"].as_str().unwrap().to_owned();
            assert!(
                reference == early_ref || reference == late_ref,
                "unrelated source matched: {entry}"
            );
            assert!(
                seen.insert(reference),
                "a frozen source was returned on two pages"
            );
        }
        assert!(
            response.get("next_cursor").is_some(),
            "search must distinguish a completed scan from an incomplete page: {response}"
        );
        let next = response["next_cursor"].clone();
        if !next.is_null() {
            assert!(next.is_string());
            assert_eq!(response["truncated"], true);
            empty_incomplete_page |= entries.is_empty();
            if step == 1 {
                saved_cursor = next;
                last_call = "bad-query".to_owned();
                return (
                    request(&last_call, "DIFFERENT-QUERY", saved_cursor.clone()),
                    false,
                );
            }
            last_call = format!("page-{step}");
            return (request(&last_call, "MARKER947NEEDLE", next), false);
        }
        assert_eq!(seen, BTreeSet::from([early_ref.clone(), late_ref.clone()]));
        last_call = "read-old-source".to_owned();
        (
            m4_long_flow_tool_response(
                &last_call,
                "context_read",
                json!({
                    "operation":"read", "reference":early_ref, "max_bytes":1024
                }),
                "old-source-read",
            ),
            false,
        )
    });
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "Execute the bounded archive lookup.\n/exit\n",
        &["--agent", agent.to_str().unwrap(), "--debug"],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let diagnostics = String::from_utf8_lossy(&output.stderr);
    let pages = diagnostics
        .lines()
        .filter(|line| line.contains("phase=context_read "))
        .collect::<Vec<_>>();
    assert!(
        pages.len() >= 3,
        "missing actual page work diagnostics: {diagnostics}"
    );
    let counter = |line: &str, name: &str| -> usize {
        line.split_whitespace()
            .find_map(|field| field.strip_prefix(&format!("{name}=")))
            .unwrap_or_else(|| panic!("missing {name}: {line}"))
            .parse()
            .unwrap()
    };
    assert!(
        pages
            .iter()
            .all(|line| counter(line, "scan_bytes") <= 4 * 1024 * 1024
                && counter(line, "records") <= 256)
    );
    assert!(pages.iter().any(|line| counter(line, "records") == 256));
    assert!(
        pages
            .iter()
            .any(|line| counter(line, "source_verification_bytes") > 0)
    );
    assert!(
        pages
            .iter()
            .any(
                |line| counter(line, "initial_indexed_events") + counter(line, "indexed_events")
                    >= 620
                    && counter(line, "initial_indexed_bytes") + counter(line, "indexed_bytes") > 0
            )
    );
    selector_candidates(&f2a_request_body(&server.requests, "archive selection"));
    let request_count = server.requests.try_iter().count();
    server.join();
    assert!((5..=16).contains(&request_count));
    assert_eq!(fs::read(source_path).unwrap(), before);
    let current = session_ids(root.path())
        .into_iter()
        .find(|id| id != &prior)
        .unwrap();
    let state = reduced_session(&project, &current);
    assert_eq!(state.execution_receipt_count(), (request_count - 1) as u64);
    assert_eq!(state.history().ensure_all_tool_calls_paired(), Ok(()));
    assert_audits_reconstruct(root.path(), &state);
}

#[test]
fn oversized_cold_record_continues_to_late_match_and_following_record() {
    let root = TemporaryDirectory::new("task-context-oversized-pages");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let offline = write_context_window_config(root.path(), "http://127.0.0.1:1/v1", 272_000);
    assert!(
        run_chat(
            root.path(),
            &offline,
            "/exit\n",
            &["--agent", agent.to_str().unwrap()]
        )
        .status
        .success()
    );
    let prior = session_id(root.path());
    let project = Project::discover(root.path()).unwrap();
    let marker = "LATE742SOURCE";
    let text = format!(
        "{} {marker} exact late value 17.25.",
        "z".repeat(8 * 1024 * 1024 + 123)
    );
    let marker_offset = text.find(marker).unwrap();
    let ids = append_source_items(
        &project,
        &prior,
        vec![
            ContentItem::assistant_text(&text),
            ContentItem::assistant_text(format!("{marker} following record value 23.50.")),
        ],
    );
    let huge_ref = peer_reference(&prior, &ids[0], &ContentItem::assistant_text(&text));
    let next_ref = peer_reference(
        &prior,
        &ids[1],
        &ContentItem::assistant_text(format!("{marker} following record value 23.50.")),
    );
    let source = root
        .path()
        .join(".echidna/sessions")
        .join(&prior)
        .join("events.jsonl");
    let before = fs::read(&source).unwrap();
    let raw_len = before
        .split_inclusive(|byte| *byte == b'\n')
        .find(|line| {
            line.windows(marker.len())
                .any(|window| window == marker.as_bytes())
        })
        .unwrap()
        .len();
    let mut last_call = String::new();
    let mut seen = BTreeSet::new();
    let mut pages = 0;
    let mut empty_pending = false;
    // This bounds fixture liveness; it is not a product latency target.
    let wait = Duration::from_secs(60);
    let server = paged_server_with_request_waits(wait, wait, move |step, wire| {
        if step == 0 {
            return (new_selection_response(wire, "oversized-selection"), false);
        }
        let step = step - 1;
        let mut cursor = Value::Null;
        if step > 0 {
            let result = delivered_result(wire, &last_call);
            assert_eq!(result["status"], "succeeded", "{result}");
            let response = &result["output"];
            if last_call == "read-late-exact" {
                assert_eq!(response["entries"][0]["reference"], huge_ref);
                assert_eq!(response["entries"][0]["offset"], marker_offset);
                assert_eq!(response["entries"][0]["text"], &text[marker_offset..]);
                return (
                    text_response("OVERSIZED-PAGES-COMPLETE", "oversized-done"),
                    true,
                );
            }
            pages += 1;
            let entries = response["entries"].as_array().unwrap();
            for entry in entries {
                let reference = entry["reference"].as_str().unwrap().to_owned();
                assert!(reference == huge_ref || reference == next_ref, "{entry}");
                assert!(entry["text"].as_str().unwrap().contains(marker), "{entry}");
                assert!(
                    seen.insert(reference),
                    "one source must not be repeated across pages"
                );
            }
            assert!(response.get("next_cursor").is_some(), "{response}");
            cursor = response["next_cursor"].clone();
            if cursor.is_null() {
                assert_eq!(seen, BTreeSet::from([huge_ref.clone(), next_ref.clone()]));
                assert!(
                    pages >= 3 && empty_pending,
                    "an oversized record must advance through bounded incomplete pages"
                );
                last_call = "read-late-exact".to_owned();
                return (
                    m4_long_flow_tool_response(
                        &last_call,
                        "context_read",
                        json!({
                            "operation":"read","reference":huge_ref,"offset":marker_offset,"max_bytes":128
                        }),
                        "oversized-exact-read",
                    ),
                    false,
                );
            }
            assert_eq!(response["truncated"], true);
            empty_pending |= entries.is_empty();
        }
        last_call = format!("oversized-page-{step}");
        (
            m4_long_flow_tool_response(
                &last_call,
                "context_read",
                json!({
                    "operation":"search","query":marker,"cursor":cursor,"max_bytes":512 + marker.len()
                }),
                &format!("oversized-response-{step}"),
            ),
            false,
        )
    });
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "Search the synthetic oversized archive.\n/exit\n",
        &["--agent", agent.to_str().unwrap(), "--debug"],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let diagnostics = String::from_utf8_lossy(&output.stderr);
    let counter = |line: &str, name: &str| -> usize {
        line.split_whitespace()
            .find_map(|field| field.strip_prefix(&format!("{name}=")))
            .unwrap()
            .parse()
            .unwrap()
    };
    let pages = diagnostics
        .lines()
        .filter(|line| line.contains("phase=context_read "))
        .collect::<Vec<_>>();
    assert!(
        pages
            .iter()
            .any(|line| counter(line, "source_materialization_read_bytes") >= raw_len)
    );
    assert!(
        pages
            .iter()
            .any(|line| counter(line, "source_materialization_write_bytes") >= 8 * 1024 * 1024)
    );
    assert!(
        pages
            .iter()
            .any(|line| counter(line, "source_verification_bytes") >= raw_len)
    );
    assert!(
        pages.iter().any(
            |line| counter(line, "source_materialization_read_bytes") == 0
                && counter(line, "source_verification_bytes") == 0
        ),
        "unchanged source pages should reuse their completed verification"
    );
    selector_candidates(&f2a_request_body(&server.requests, "oversized selection"));
    let count = server.requests.try_iter().count();
    server.join();
    assert_eq!(fs::read(source).unwrap(), before);
    let current = session_ids(root.path())
        .into_iter()
        .find(|id| id != &prior)
        .unwrap();
    let state = reduced_session(&project, &current);
    assert_eq!(state.execution_receipt_count(), (count - 1) as u64);
    assert_eq!(state.history().ensure_all_tool_calls_paired(), Ok(()));
    assert_audits_reconstruct(root.path(), &state);
}

#[test]
fn cold_cursor_rejects_another_records_authenticated_text_cache() {
    let root = TemporaryDirectory::new("task-context-record-cache-swap");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let offline = write_context_window_config(root.path(), "http://127.0.0.1:1/v1", 272_000);
    assert!(
        run_chat(
            root.path(),
            &offline,
            "/exit\n",
            &["--agent", agent.to_str().unwrap()]
        )
        .status
        .success()
    );
    let prior = session_id(root.path());
    let project = Project::discover(root.path()).unwrap();
    append_source_items(
        &project,
        &prior,
        ["CACHE-A", "CACHE-B"]
            .into_iter()
            .map(|name| {
                ContentItem::assistant_text(format!(
                    "{name} {} TARGET928NOVA",
                    "z".repeat(5 * 1024 * 1024)
                ))
            })
            .collect(),
    );
    let scratch = root.path().join("isolated-tmp");
    fs::create_dir(&scratch).unwrap();
    let server_scratch = scratch.clone();
    let mut old_cache: Option<(Vec<u8>, Vec<u8>)> = None;
    let mut changed = false;
    let mut last_call = String::new();
    let server = paged_server(move |step, wire| {
        if step == 0 {
            return (new_selection_response(wire, "cache-swap-selection"), false);
        }
        let step = step - 1;
        let mut cursor = Value::Null;
        if step > 0 {
            let result = delivered_result(wire, &last_call);
            if result["status"] == "failed" || result["output"]["status"] == "stale" {
                assert!(
                    changed,
                    "cache substitution must precede rejection: {result}"
                );
                return (
                    text_response("CACHE-SWAP-REJECTED", "cache-swap-final"),
                    true,
                );
            }
            assert_eq!(result["status"], "succeeded", "{result}");
            cursor = result["output"]["next_cursor"].clone();
            assert!(
                cursor.is_string(),
                "cache swap fixture must continue: {result}"
            );
            for directory in fs::read_dir(&server_scratch).unwrap() {
                let directory = directory.unwrap().path();
                if !directory.is_dir() {
                    continue;
                }
                for entry in fs::read_dir(directory).unwrap() {
                    let path = entry.unwrap().path();
                    if path
                        .extension()
                        .is_none_or(|extension| extension != "record-text")
                    {
                        continue;
                    }
                    let contents = fs::read(&path).unwrap();
                    let hashes = path.with_extension("record-hashes");
                    if contents.starts_with(b"CACHE-A") && old_cache.is_none() {
                        old_cache = Some((contents, fs::read(hashes).unwrap()));
                    } else if contents.starts_with(b"CACHE-B") && !changed {
                        let (text, hash) = old_cache.as_ref().expect("first source cache observed");
                        fs::write(&path, text).unwrap();
                        fs::write(hashes, hash).unwrap();
                        changed = true;
                    }
                }
            }
        }
        last_call = format!("cache-swap-{step}");
        (
            m4_long_flow_tool_response(
                &last_call,
                "context_read",
                json!({
                "operation":"search", "query":"TARGET928NOVA", "cursor":cursor, "max_bytes":1024
                }),
                &format!("cache-swap-response-{step}"),
            ),
            false,
        )
    });
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let mut child = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .current_dir(root.path())
        .args([
            "chat",
            "--config",
            config.to_str().unwrap(),
            "--agent",
            agent.to_str().unwrap(),
        ])
        .env("TMPDIR", &scratch)
        .env("CYBER_AGENT_PRIMARY_API_KEY", "loopback-primary-secret")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"Search the synthetic archive.\n/exit\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.join();
    assert!(String::from_utf8_lossy(&output.stdout).contains("CACHE-SWAP-REJECTED"));
}

#[test]
fn cold_cursor_rejects_changed_accessed_source_even_with_same_size_and_mtime() {
    let root = TemporaryDirectory::new("task-context-paging-source-change");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let offline = write_context_window_config(root.path(), "http://127.0.0.1:1/v1", 272_000);
    assert!(
        run_chat(
            root.path(),
            &offline,
            "/exit\n",
            &["--agent", agent.to_str().unwrap()]
        )
        .status
        .success()
    );
    let prior = session_id(root.path());
    let project = Project::discover(root.path()).unwrap();
    append_source_items(
        &project,
        &prior,
        (0..620)
            .map(|index| {
                ContentItem::assistant_text(if index == 600 {
                    "TOKEN841TARGET original value".to_owned()
                } else {
                    format!("unrelated archived record {index}")
                })
            })
            .collect(),
    );
    let source = root
        .path()
        .join(".echidna/sessions")
        .join(&prior)
        .join("events.jsonl");
    let before = fs::read(&source).unwrap();
    let modified = fs::metadata(&source).unwrap().modified().unwrap();
    let original = before.clone();
    let source_for_server = source.clone();
    let mut last_call = String::new();
    let mut changed = false;
    let server = paged_server(move |step, wire| {
        if step == 0 {
            return (
                new_selection_response(wire, "source-change-selection"),
                false,
            );
        }
        let step = step - 1;
        let mut cursor = Value::Null;
        if step > 0 {
            let result = delivered_result(wire, &last_call);
            if result["status"] == "failed" || result["output"]["status"] == "stale" {
                assert!(
                    changed,
                    "the original source must be accepted before tampering: {result}"
                );
                fs::write(&source_for_server, &original).unwrap();
                fs::File::open(&source_for_server)
                    .unwrap()
                    .set_times(fs::FileTimes::new().set_modified(modified))
                    .unwrap();
                return (
                    text_response("CHANGED-SOURCE-REJECTED", "source-change-final"),
                    true,
                );
            }
            assert_eq!(result["status"], "succeeded", "{result}");
            let response = &result["output"];
            assert!(
                response["entries"].as_array().unwrap().is_empty(),
                "modified late source cannot be returned as verified: {response}"
            );
            cursor = response["next_cursor"].clone();
            assert!(
                cursor.is_string(),
                "the changed source cannot silently finish as a miss: {response}"
            );
            if !changed {
                let altered = String::from_utf8(original.clone())
                    .unwrap()
                    .replace("TOKEN841TARGET", "TOKEN841FORGED");
                assert_eq!(altered.len(), original.len());
                assert_ne!(altered.as_bytes(), original);
                fs::write(&source_for_server, altered).unwrap();
                fs::File::open(&source_for_server)
                    .unwrap()
                    .set_times(fs::FileTimes::new().set_modified(modified))
                    .unwrap();
                assert_eq!(
                    fs::metadata(&source_for_server)
                        .unwrap()
                        .modified()
                        .unwrap(),
                    modified
                );
                changed = true;
            }
        }
        last_call = format!("source-page-{step}");
        (
            m4_long_flow_tool_response(
                &last_call,
                "context_read",
                json!({
                    "operation":"search","query":"TOKEN841TARGET","cursor":cursor,"max_bytes":1024
                }),
                &format!("source-page-response-{step}"),
            ),
            false,
        )
    });
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "Search the synthetic archive with source validation.\n/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    selector_candidates(&f2a_request_body(
        &server.requests,
        "source-change selection",
    ));
    let count = server.requests.try_iter().count();
    server.join();
    assert!(String::from_utf8_lossy(&output.stdout).contains("CHANGED-SOURCE-REJECTED"));
    assert_eq!(fs::read(source).unwrap(), before);
    let current = session_ids(root.path())
        .into_iter()
        .find(|id| id != &prior)
        .unwrap();
    let state = reduced_session(&project, &current);
    assert_eq!(state.execution_receipt_count(), (count - 1) as u64);
    assert_eq!(state.history().ensure_all_tool_calls_paired(), Ok(()));
    assert_audits_reconstruct(root.path(), &state);
}

#[test]
fn cold_search_rechecks_earlier_segments_before_returning_late_hit() {
    let root = TemporaryDirectory::new("task-context-earlier-segment-change");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let offline = write_context_window_config(root.path(), "http://127.0.0.1:1/v1", 272_000);
    assert!(
        run_chat(
            root.path(),
            &offline,
            "/exit\n",
            &["--agent", agent.to_str().unwrap()]
        )
        .status
        .success()
    );
    let prior = session_id(root.path());
    let project = Project::discover(root.path()).unwrap();
    append_source_items(
        &project,
        &prior,
        vec![ContentItem::assistant_text(format!(
            "EARLY-SOURCE-ORIGINAL {} LATESEGMENT729MATCH exact value",
            "z".repeat(8 * 1024 * 1024 + 123)
        ))],
    );
    let source = root
        .path()
        .join(".echidna/sessions")
        .join(&prior)
        .join("events.jsonl");
    let original = fs::read(&source).unwrap();
    let modified = fs::metadata(&source).unwrap().modified().unwrap();
    let source_for_server = source.clone();
    let original_for_server = original.clone();
    let mut last_call = String::new();
    let mut pages = 0;
    let server = paged_server(move |step, wire| {
        if step == 0 {
            return (
                new_selection_response(wire, "earlier-segment-selection"),
                false,
            );
        }
        let step = step - 1;
        let mut cursor = Value::Null;
        if step > 0 {
            let result = delivered_result(wire, &last_call);
            if result["status"] == "failed" || result["output"]["status"] == "stale" {
                assert!(
                    pages > 0,
                    "the original first page must complete before tampering"
                );
                fs::write(&source_for_server, &original_for_server).unwrap();
                fs::File::open(&source_for_server)
                    .unwrap()
                    .set_times(fs::FileTimes::new().set_modified(modified))
                    .unwrap();
                return (
                    text_response("EARLIER-SEGMENT-REJECTED", "earlier-change-final"),
                    true,
                );
            }
            assert_eq!(result["status"], "succeeded", "{result}");
            let response = &result["output"];
            assert!(
                response["entries"].as_array().unwrap().is_empty(),
                "changed source returned as verified: {response}"
            );
            cursor = response["next_cursor"].clone();
            assert!(
                cursor.is_string(),
                "large record must still have unread data: {response}"
            );
            pages += 1;
            if pages == 1 {
                let altered = String::from_utf8(original_for_server.clone())
                    .unwrap()
                    .replace("EARLY-SOURCE-ORIGINAL", "EARLY-SOURCE-MODIFIED");
                assert_eq!(altered.len(), original_for_server.len());
                assert_ne!(altered.as_bytes(), original_for_server);
                fs::write(&source_for_server, altered).unwrap();
                fs::File::open(&source_for_server)
                    .unwrap()
                    .set_times(fs::FileTimes::new().set_modified(modified))
                    .unwrap();
            }
        }
        last_call = format!("early-segment-page-{step}");
        (
            m4_long_flow_tool_response(
                &last_call,
                "context_read",
                json!({"operation":"search", "query":"LATESEGMENT729MATCH", "cursor":cursor, "max_bytes":1024}),
                &format!("early-segment-response-{step}"),
            ),
            false,
        )
    });
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "Search the large synthetic archive.\n/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    selector_candidates(&f2a_request_body(
        &server.requests,
        "earlier-segment selection",
    ));
    let count = server.requests.try_iter().count();
    server.join();
    assert!(String::from_utf8_lossy(&output.stdout).contains("EARLIER-SEGMENT-REJECTED"));
    assert_eq!(fs::read(source).unwrap(), original);
    let current = session_ids(root.path())
        .into_iter()
        .find(|id| id != &prior)
        .unwrap();
    let state = reduced_session(&project, &current);
    assert_eq!(state.execution_receipt_count(), (count - 1) as u64);
    assert_eq!(state.history().ensure_all_tool_calls_paired(), Ok(()));
    assert_audits_reconstruct(root.path(), &state);
}

#[test]
fn cold_search_matches_cjk_and_emoji_across_a_split_utf8_chunk() {
    let root = TemporaryDirectory::new("task-context-utf8-chunk");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let offline = write_context_window_config(root.path(), "http://127.0.0.1:1/v1", 272_000);
    assert!(
        run_chat(
            root.path(),
            &offline,
            "/exit\n",
            &["--agent", agent.to_str().unwrap()]
        )
        .status
        .success()
    );
    let prior = session_id(root.path());
    let project = Project::discover(root.path()).unwrap();
    let query = "😀跨界检索";
    let text = format!("{} {query} 精确来源", "z".repeat(1024 * 1024 - 3));
    let offset = text.find(query).unwrap();
    assert_eq!(offset, 1024 * 1024 - 2);
    assert!(
        !text.is_char_boundary(1024 * 1024),
        "the chunk boundary must split the emoji"
    );
    let item = ContentItem::assistant_text(text);
    let ids = append_source_items(&project, &prior, vec![item.clone()]);
    let reference = peer_reference(&prior, &ids[0], &item);
    let mut last_call = String::new();
    let server = paged_server(move |step, wire| {
        if step == 0 {
            return (
                new_selection_response(wire, "utf8-boundary-selection"),
                false,
            );
        }
        let step = step - 1;
        let mut cursor = Value::Null;
        if step > 0 {
            let result = delivered_result(wire, &last_call);
            assert_eq!(result["status"], "succeeded", "{result}");
            let response = &result["output"];
            if response["entries"].as_array().unwrap().iter().any(|entry| {
                entry["reference"] == reference && entry["text"].as_str().unwrap().contains(query)
            }) {
                return (
                    text_response("UTF8-BOUNDARY-FOUND", "utf8-boundary-final"),
                    true,
                );
            }
            cursor = response["next_cursor"].clone();
            assert!(
                cursor.is_string(),
                "search ended before the cross-boundary source was found: {response}"
            );
        }
        last_call = format!("utf8-boundary-page-{step}");
        (
            m4_long_flow_tool_response(
                &last_call,
                "context_read",
                json!({"operation":"search", "query":query, "cursor":cursor, "max_bytes":1024}),
                &format!("utf8-boundary-response-{step}"),
            ),
            false,
        )
    });
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "Find the synthetic cross-boundary value.\n/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.join();
    assert!(String::from_utf8_lossy(&output.stdout).contains("UTF8-BOUNDARY-FOUND"));
}

#[test]
fn target_artifact_owner_excludes_other_agent_and_rejects_duplicate_call_ids() {
    use super::task_context::{
        ScriptedResponse, selector_candidates, spawn_scripted_responses_server,
    };
    let root = TemporaryDirectory::new("task-context-target-artifact-owner");
    initialize_project(root.path());
    fs::write(
        root.path().join("owner-source.txt"),
        format!(
            "{} NXZQJRV984724TOKEN exact value 31.75",
            "z".repeat(9 * 1024 * 1024)
        ),
    )
    .unwrap();
    let publisher = spawn_scripted_responses_server(vec![
        ScriptedResponse::InitialSelection,
        ScriptedResponse::Response(
            200,
            m4_long_flow_tool_response(
                "owner-snapshot-call",
                "research_update",
                json!({
                    "event_id":"owner-snapshot-event", "occurred_at":"2026-11-05T00:00:00Z",
                    "operation":{"kind":"record_target_snapshot", "record":{
                        "id":"owner-snapshot", "target_id":"fixture.owner", "kind":"source",
                        "locator":"owner-source.txt", "version":"1", "build_id":"fixture",
                        "target_artifact_ids":["owner-target-artifact"], "environment_fingerprint":"offline",
                        "device_fingerprint":"offline", "captured_at":"2026-11-05T00:00:00Z", "notes":"synthetic"
                    }}
                }),
                "owner-snapshot-response",
            ),
        ),
        ScriptedResponse::Response(
            200,
            m4_long_flow_tool_response(
                "shared-owner-call",
                "artifact_store",
                json!({
                    "event_id":"owner-artifact-event", "occurred_at":"2026-11-05T00:00:01Z",
                    "artifact_id":"owner-target-artifact", "source_path":"owner-source.txt",
                    "media_type":"text/plain", "target_snapshot_id":"owner-snapshot", "toolchain_fingerprint":"fixture"
                }),
                "owner-artifact-response",
            ),
        ),
        ScriptedResponse::Response(200, text_response("OWNER-PUBLISHED", "owner-published")),
    ]);
    let config = write_context_window_config(root.path(), &publisher.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "Publish the synthetic target artifact.\n/exit\n",
        &["--domain", "source", "--phase", "experiment"],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    selector_candidates(&f2a_request_body(
        &publisher.requests,
        "publisher selection",
    ));
    for _ in 0..3 {
        f2a_request_body(&publisher.requests, "target Artifact publication");
    }
    publisher.join();
    let producer = session_id(root.path());
    let project = Project::discover(root.path()).unwrap();
    let state = reduced_session(&project, &producer);
    let result = |call_id: &str| {
        state
            .history()
            .items()
            .iter()
            .find_map(|item| match item {
                ContentItem::ToolResult { result } if result.call_id.as_str() == call_id => {
                    Some(result)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing publisher ToolResult for {call_id}"))
    };
    for call_id in ["owner-snapshot-call", "shared-owner-call"] {
        let result = result(call_id);
        assert_eq!(
            result.status,
            ToolResultStatus::Succeeded,
            "{call_id} failed: {}",
            result.output
        );
    }
    let published = result("shared-owner-call");
    assert_eq!(
        published.output["artifact"]["scope"],
        json!({"kind":"target", "target_snapshot_id":"owner-snapshot"})
    );
    let blob = published.output["blob"]["blob_id"]
        .as_str()
        .expect("successful Target Artifact publication must return a Blob ID")
        .to_owned();
    let reference = format!("artifact:owner-target-artifact:{blob}");
    let other = definition(root.path(), None);
    let foreign = spawn_scripted_responses_server(vec![
        ScriptedResponse::NewSelection,
        ScriptedResponse::Response(
            200,
            m4_long_flow_tool_response(
                "foreign-target-read",
                "context_read",
                json!({"operation":"read","reference":reference,"max_bytes":128}),
                "foreign-target-response",
            ),
        ),
        ScriptedResponse::Response(
            200,
            text_response("FOREIGN-READ-COMPLETE", "foreign-target-final"),
        ),
    ]);
    let config = write_context_window_config(root.path(), &foreign.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "Read the synthetic target source.\n/exit\n",
        &["--agent", other.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    selector_candidates(&f2a_request_body(
        &foreign.requests,
        "foreign target selection",
    ));
    let _ = f2a_request_body(&foreign.requests, "foreign target first request");
    let wire = f2a_request_body(&foreign.requests, "foreign target result");
    assert_eq!(
        delivered_result(&wire, "foreign-target-read")["output"]["status"],
        "not_found"
    );
    foreign.join();
    // Establish the search before the collision. Both intervening calls are
    // executed in the current Session; the producer is the earlier Session.
    let mut cursor = Value::Null;
    // This bounds fixture liveness; it is not a product latency target.
    let wait = Duration::from_secs(60);
    let cursor_server = paged_server_with_request_waits(wait, wait, move |step, wire| match step {
        0 => (
            super::task_context::new_selection_response(wire, "owner-cursor-selection"),
            false,
        ),
        1 => (
            m4_long_flow_tool_response(
                "owner-search-first",
                "context_read",
                json!({
                    "operation":"search", "query":"NXZQJRV984724TOKEN", "max_bytes":1024
                }),
                "owner-search-first-response",
            ),
            false,
        ),
        2 => {
            let result = delivered_result(wire, "owner-search-first");
            assert_eq!(result["status"], "succeeded", "{result}");
            assert!(
                result["output"]["entries"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|entry| entry["scope"] != "artifact"),
                "marker unexpectedly matched the Artifact reference: {result}"
            );
            cursor = result["output"]["next_cursor"].clone();
            assert!(
                cursor.is_string(),
                "the late artifact must require another page: {result}"
            );
            (
                m4_long_flow_tool_response(
                    "unrelated-owner-call",
                    "run_command",
                    json!({
                        "argv":["/bin/echo", "unrelated append"]
                    }),
                    "unrelated-owner-response",
                ),
                false,
            )
        }
        3 => {
            let result = delivered_result(wire, "unrelated-owner-call");
            assert_eq!(result["status"], "succeeded", "{result}");
            (
                m4_long_flow_tool_response(
                    "owner-search-second",
                    "context_read",
                    json!({
                        "operation":"search", "query":"NXZQJRV984724TOKEN", "cursor":cursor, "max_bytes":1024
                    }),
                    "owner-search-second-response",
                ),
                false,
            )
        }
        4 => {
            let result = delivered_result(wire, "owner-search-second");
            assert_eq!(
                result["status"], "succeeded",
                "unrelated append invalidated cursor: {result}"
            );
            cursor = result["output"]["next_cursor"].clone();
            assert!(
                cursor.is_string(),
                "the artifact marker should still be ahead: {result}"
            );
            (
                m4_long_flow_tool_response(
                    "shared-owner-call",
                    "run_command",
                    json!({
                        "argv":["/bin/echo", "new actual call with duplicate id"]
                    }),
                    "duplicate-owner-call-response",
                ),
                false,
            )
        }
        5 => {
            let result = delivered_result(wire, "shared-owner-call");
            assert_eq!(
                result["status"], "succeeded",
                "duplicate ID is legal in a different Session: {result}"
            );
            (
                m4_long_flow_tool_response(
                    "owner-search-after-collision",
                    "context_read",
                    json!({
                        "operation":"search", "query":"NXZQJRV984724TOKEN", "cursor":cursor, "max_bytes":1024
                    }),
                    "owner-search-collision-response",
                ),
                false,
            )
        }
        _ => {
            let last = if step == 6 {
                "owner-search-after-collision".to_owned()
            } else {
                format!("owner-search-collision-page-{}", step - 1)
            };
            let result = delivered_result(wire, &last);
            if result["status"] == "failed" {
                assert!(
                    result.to_string().contains("Artifact source scope changed"),
                    "{result}"
                );
                (
                    text_response("CURSOR-SCOPE-CHANGE-REJECTED", "owner-cursor-final"),
                    true,
                )
            } else {
                assert_eq!(result["status"], "succeeded", "{result}");
                assert!(
                    result["output"]["entries"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .all(|entry| entry["scope"] != "artifact"),
                    "stale owner disclosed: {result}"
                );
                cursor = result["output"]["next_cursor"].clone();
                assert!(
                    cursor.is_string(),
                    "changed Artifact ownership was not checked: {result}"
                );
                let call = format!("owner-search-collision-page-{step}");
                (
                    m4_long_flow_tool_response(
                        &call,
                        "context_read",
                        json!({
                            "operation":"search", "query":"NXZQJRV984724TOKEN", "cursor":cursor, "max_bytes":1024
                        }),
                        &format!("{call}-response"),
                    ),
                    false,
                )
            }
        }
    });
    let config = write_context_window_config(root.path(), &cursor_server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "Search the synthetic target source.\n/exit\n",
        &["--debug"],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    cursor_server.join();
    assert!(String::from_utf8_lossy(&output.stdout).contains("CURSOR-SCOPE-CHANGE-REJECTED"));
    let reader = spawn_scripted_responses_server(vec![
        // The source is requested through context_read below; selecting every
        // unrelated Research unit here would request a separate expansion.
        ScriptedResponse::NewSelection,
        ScriptedResponse::Response(
            200,
            m4_long_flow_tool_response(
                "duplicate-owner-read",
                "context_read",
                json!({"operation":"read","reference":reference,"max_bytes":128}),
                "duplicate-owner-response",
            ),
        ),
        ScriptedResponse::Response(
            200,
            text_response("DUPLICATE-OWNER-COMPLETE", "duplicate-owner-final"),
        ),
    ]);
    let config = write_context_window_config(root.path(), &reader.base_url, 272_000);
    let prior_sessions = session_ids(root.path());
    let output = run_chat(
        root.path(),
        &config,
        "Read the synthetic target source.\n/exit\n",
        &[],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let selector = f2a_request_body(&reader.requests, "duplicate target selection");
    let candidates = selector_candidates(&selector);
    assert!(candidates["tasks"].as_array().unwrap().is_empty());
    let main = f2a_request_body(&reader.requests, "duplicate target first request");
    assert!(
        main["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["name"] == "context_read")
    );
    let wire = f2a_request_body(&reader.requests, "duplicate target result");
    let result = delivered_result(&wire, "duplicate-owner-read");
    assert_eq!(result["status"], "succeeded", "{result}");
    assert_eq!(result["output"]["status"], "ambiguous");
    assert!(reader.requests.try_recv().is_err());
    reader.join();
    let reader_session = session_ids(root.path())
        .into_iter()
        .find(|id| !prior_sessions.contains(id))
        .expect("new reader Session");
    let reader_state = reduced_session(&project, &reader_session);
    assert_eq!(
        reader_state.execution_receipt_count(),
        1,
        "only the explicit context_read executes"
    );
    assert_eq!(
        reader_state.history().ensure_all_tool_calls_paired(),
        Ok(())
    );
}
