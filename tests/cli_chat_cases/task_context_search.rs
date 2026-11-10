//! Scoped search uses the existing lexical and CJK ranking owner.
use super::task_context::{
    ScriptedResponse, definition, initial_selection_response, selector_candidates,
    spawn_scripted_responses_server,
};
use super::task_context_paging::{
    append_source_items, delivered_result, paged_server, paged_server_with_request_waits,
    peer_reference,
};
use super::*;

#[test]
fn task_index_recalls_old_source_and_updates_only_the_new_suffix() {
    let root = TemporaryDirectory::new("task-postings-incremental");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    // The batching workload retains a 512 KiB Tool result plus 529 lexical rows.
    // Its explicit fixture capacity must admit that source before indexing is tested.
    let offline = write_context_window_config(root.path(), "http://127.0.0.1:1/v1", 1_000_000);
    let created = run_chat(
        root.path(),
        &offline,
        "/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    let prior = session_id(root.path());
    let project = Project::discover(root.path()).unwrap();
    let call = echidna::domain::tools::ToolCall {
        call_id: echidna::domain::tools::ToolCallId::new("large-index-call"),
        name: echidna::domain::tools::ToolName::new("read_file"),
        arguments: json!({"path":"synthetic-large"}),
    };
    let result = echidna::domain::tools::ToolResult {
        call_id: call.call_id.clone(),
        tool_name: call.name.clone(),
        status: echidna::domain::tools::ToolResultStatus::Succeeded,
        output: json!({"large": "z".repeat(512 * 1024)}),
    };
    append_source_items(
        &project,
        &prior,
        vec![
            echidna::domain::model::ContentItem::assistant_text(
                "COLDTASKUNIQUE789 original observation",
            ),
            echidna::domain::model::ContentItem::ToolCall { call },
        ],
    );
    {
        let session = StreamId::new(&prior);
        let stream = ProjectEventStream::Session(session.clone()).stream_id();
        let mut writer = project.open_writer().unwrap();
        let mut store = writer.open_session(session, None).unwrap();
        let receipt = serde_json::from_value(json!({
            "execution_id":"execution:large-index-call", "plan_fingerprint":"synthetic-plan",
            "status":"succeeded", "exit_reason":null, "output_artifact_ids":[], "environment_change":null
        })).unwrap();
        for event in [
            SessionEvent::ExecutionOutcomeRecorded {
                receipt,
                result: result.clone(),
            },
            SessionEvent::HistoryItemRecorded {
                item: echidna::domain::model::ContentItem::ToolResult { result },
            },
        ] {
            let event = event
                .into_envelope(
                    EventId::new(format!(
                        "large-index-{}",
                        store.next_sequence().unwrap().get()
                    )),
                    stream.clone(),
                    store.next_sequence().unwrap(),
                    OccurredAt::new("2026-11-05T00:00:00Z"),
                )
                .unwrap();
            EventSink::append_event(&mut store, &event).unwrap();
        }
    }
    let repeated_terms = (0..70)
        .map(|index| format!("batchword{index:04}"))
        .collect::<Vec<_>>()
        .join(" ");
    append_source_items(
        &project,
        &prior,
        (0..529)
            .map(|_| echidna::domain::model::ContentItem::assistant_text(&repeated_terms))
            .collect(),
    );
    let server = spawn_scripted_responses_server(vec![
        ScriptedResponse::NewSelection,
        ScriptedResponse::Response(200, text_response("first complete", "postings-first")),
        ScriptedResponse::NewSelection,
        ScriptedResponse::Response(200, text_response("second complete", "postings-second")),
    ]);
    let config = write_context_window_config(root.path(), &server.base_url, 1_000_000);
    let output = run_chat(
        root.path(),
        &config,
        "Find COLDTASKUNIQUE789.\nFind COLDTASKUNIQUE789 again.\n/exit\n",
        &["--resume", "latest", "--debug"],
    );
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let wires = (0..4)
        .map(|_| f2a_request_body(&server.requests, "incremental task index"))
        .collect::<Vec<_>>();
    server.join();
    for wire in [&wires[0], &wires[2]] {
        let candidates = selector_candidates(wire);
        let context = candidates["context"].as_array().unwrap();
        assert!(context.len() <= 512);
        assert!(context.iter().any(|candidate| {
            candidate["excerpt"]
                .as_str()
                .is_some_and(|text| text.contains("COLDTASKUNIQUE789"))
        }));
        assert!(
            candidates["gaps"]
                .to_string()
                .contains("older context candidates require expansion")
        );
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let work = stderr
        .lines()
        .filter(|line| line.contains("phase=task_index "))
        .collect::<Vec<_>>();
    assert_eq!(work.len(), 2, "{stderr}");
    let field = |line: &str, name: &str| -> usize {
        line.split_whitespace()
            .find_map(|item| item.strip_prefix(&format!("{name}=")))
            .unwrap()
            .parse()
            .unwrap()
    };
    assert!(field(work[0], "source_rows") >= 530, "{}", work[0]);
    assert!(field(work[0], "source_index_read_bytes") >= 512 * 1024);
    assert!(
        field(work[0], "source_index_text_write_bytes") <= field(work[0], "source_rows") * 1024
    );
    assert!(field(work[0], "source_index_text_write_bytes") < 512 * 1024);
    assert!(field(work[0], "posting_rows_written") > 65_536);
    assert_eq!(field(work[0], "posting_batch_peak_rows"), 65_536);
    assert!(field(work[0], "posting_batches") > 1);
    assert!(field(work[0], "source_index_hash_write_bytes") > 0);
    assert!(field(work[0], "source_index_record_write_bytes") > 0);
    assert!(work[0].contains("metadata_rebuilt=true"));
    assert!(field(work[1], "source_rows") <= 4, "{}", work[1]);
    assert!(
        field(work[1], "source_index_read_bytes") < field(work[0], "source_index_read_bytes"),
        "{}",
        work[1]
    );
    assert!(work[1].contains("metadata_rebuilt=false"), "{}", work[1]);
    assert!(field(work[1], "posting_keys") > 0, "{}", work[1]);
    assert!(
        field(work[1], "posting_keys")
            <= field(work[0], "posting_keys") + field(work[1], "posting_rows_written"),
        "{}",
        work[1]
    );
    for line in work {
        assert!(field(line, "posting_batch_peak_rows") <= 65_536);
        assert_eq!(
            field(line, "posting_bytes_written"),
            field(line, "posting_rows_written") * 8
        );
        assert!(field(line, "semantics_rows_read") <= 513, "{line}");
        assert!(field(line, "posting_rows_read") <= 12_000, "{line}");
    }
    // The persisted task selection is replayed from original facts on resume.
    let resumed = run_chat(
        root.path(),
        &offline,
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
fn context_search_recalls_separated_terms_and_cjk_ngrams() {
    let root = TemporaryDirectory::new("task-ranked-search");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let source =
        "alpha was measured at noon; the separate beta result was retained. 强势个股午后突破。";
    let server = spawn_scripted_responses_server(vec![
        ScriptedResponse::InitialSelection,
        ScriptedResponse::Response(200, text_response(source, "rank-source")),
        ScriptedResponse::NewSelection,
        ScriptedResponse::Response(
            200,
            m4_long_flow_tool_response(
                "rank-en",
                "context_read",
                json!({"operation":"search", "query":"alpha beta", "max_bytes":4096}),
                "rank-en-response",
            ),
        ),
        ScriptedResponse::Response(
            200,
            m4_long_flow_tool_response(
                "rank-cjk",
                "context_read",
                json!({"operation":"search", "query":"强势突破", "max_bytes":4096}),
                "rank-cjk-response",
            ),
        ),
        ScriptedResponse::Response(200, text_response("search complete", "rank-final")),
    ]);
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "Record the source.\nSearch the earlier material.\n/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for _ in 0..6 {
        f2a_request_body(&server.requests, "ranked context search");
    }
    assert!(server.requests.try_recv().is_err());
    server.join();
    let state = reduced_session(
        &Project::discover(root.path()).unwrap(),
        &session_id(root.path()),
    );
    for call in ["rank-en", "rank-cjk"] {
        let result = state
            .history()
            .items()
            .iter()
            .find_map(|item| match item {
                echidna::domain::model::ContentItem::ToolResult { result }
                    if result.call_id.as_str() == call =>
                {
                    Some(result)
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(result.output["status"], "found", "{}", result.output);
        assert!(
            result.output["entries"]
                .as_array()
                .unwrap()
                .iter()
                .any(|entry| entry["text"].as_str().unwrap().contains(source)),
            "{}",
            result.output
        );
    }
}

#[test]
fn context_search_finds_terms_in_verified_artifact_body_after_first_chunk() {
    let root = TemporaryDirectory::new("task-ranked-artifact-search");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let manifest = agent.join("agent.toml");
    fs::write(
        &manifest,
        format!(
            "{}artifact_store = \"direct\"\n",
            fs::read_to_string(&manifest).unwrap()
        ),
    )
    .unwrap();
    let source = format!(
        "{}galaxy observation retained a separate spectrum result.",
        " ".repeat(16_382)
    );
    fs::write(root.path().join("search-source.txt"), source).unwrap();
    let server = spawn_scripted_responses_server(vec![
        ScriptedResponse::InitialSelection,
        ScriptedResponse::Response(
            200,
            m4_long_flow_tool_response(
                "publish-search",
                "artifact_store",
                json!({
                    "event_id":"search-artifact-event", "occurred_at":"2026-11-05T00:00:00Z", "artifact_id":"search-artifact",
                    "source_path":"search-source.txt", "media_type":"text/plain", "toolchain_fingerprint":"fixture-v1"
                }),
                "search-published",
            ),
        ),
        ScriptedResponse::Response(
            200,
            m4_long_flow_tool_response(
                "search-artifact-body",
                "context_read",
                json!({"operation":"search", "query":"galaxy spectrum", "max_bytes":4096}),
                "search-request",
            ),
        ),
        ScriptedResponse::Response(200, text_response("Search completed.", "search-done")),
    ]);
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "Publish and search the synthetic artifact.\n/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for _ in 0..4 {
        f2a_request_body(&server.requests, "artifact body search");
    }
    assert!(server.requests.try_recv().is_err());
    server.join();
    let state = reduced_session(
        &Project::discover(root.path()).unwrap(),
        &session_id(root.path()),
    );
    let result = state
        .history()
        .items()
        .iter()
        .find_map(|item| match item {
            echidna::domain::model::ContentItem::ToolResult { result }
                if result.call_id.as_str() == "search-artifact-body" =>
            {
                Some(result)
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(result.output["status"], "found", "{}", result.output);
    let entry = result.output["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["scope"] == "artifact")
        .unwrap();
    assert!(entry["offset"].as_u64().unwrap() > 12_288);
    assert!(entry["text"].as_str().unwrap().contains("galaxy"));
}

#[test]
fn context_search_finds_separated_terms_in_late_tool_result() {
    let root = TemporaryDirectory::new("task-ranked-tool-search");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let server = spawn_scripted_responses_server(vec![
        ScriptedResponse::InitialSelection,
        ScriptedResponse::Response(
            200,
            m4_long_flow_tool_response(
                "make-long-output",
                "run_command",
                json!({"argv":["/bin/sh", "-c", "printf '%20000s' ''; printf 'mars observation and separate satellite result'"]}),
                "long-output",
            ),
        ),
        ScriptedResponse::Response(
            200,
            m4_long_flow_tool_response(
                "search-long-output",
                "context_read",
                json!({"operation":"search", "query":"mars satellite", "max_bytes":4096}),
                "long-search",
            ),
        ),
        ScriptedResponse::Response(
            200,
            text_response("Long output found.", "long-search-final"),
        ),
    ]);
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "Produce and search the synthetic output.\n/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for _ in 0..4 {
        f2a_request_body(&server.requests, "long tool output search");
    }
    assert!(server.requests.try_recv().is_err());
    server.join();
    let state = reduced_session(
        &Project::discover(root.path()).unwrap(),
        &session_id(root.path()),
    );
    let result = state
        .history()
        .items()
        .iter()
        .find_map(|item| match item {
            echidna::domain::model::ContentItem::ToolResult { result }
                if result.call_id.as_str() == "search-long-output" =>
            {
                Some(result)
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(result.output["status"], "found", "{}", result.output);
    assert!(
        result.output["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["offset"]
                .as_u64()
                .is_some_and(|offset| offset > 16_384)
                && entry["text"]
                    .as_str()
                    .is_some_and(|text| text.contains("mars"))),
        "{}",
        result.output
    );
}

#[test]
fn context_search_advances_past_weak_candidates_and_preserves_unicode_offsets() {
    use echidna::domain::model::ContentItem;
    use std::collections::BTreeSet;

    let root = TemporaryDirectory::new("task-search-candidate-offsets");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let manifest = agent.join("agent.toml");
    fs::write(
        &manifest,
        format!(
            "{}artifact_store = \"direct\"\n",
            fs::read_to_string(&manifest)
                .unwrap()
                .replace("memory = \"none\"", "memory = \"project\"")
        ),
    )
    .unwrap();
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
    let source = format!(
        "ab {} abcdefx ÉÉ İİ MixCase 强势个股午后突破",
        "z".repeat(2_000)
    );
    let weak = format!("ab {} unrelated", "z".repeat(2_000));
    let dense = format!("{} abcdefx", "ab ".repeat(5_000));
    let boundary = format!("{}ab {} abcdefx", " ".repeat(14_381), "z".repeat(2_000));
    let ids = append_source_items(
        &project,
        &prior,
        vec![
            ContentItem::assistant_text(source.clone()),
            ContentItem::assistant_text(weak.clone()),
            ContentItem::assistant_text(boundary.clone()),
            ContentItem::assistant_text(dense.clone()),
        ],
    );
    let session_ref = peer_reference(
        &prior,
        &ids[0],
        &ContentItem::assistant_text(source.clone()),
    );
    let weak_ref = peer_reference(&prior, &ids[1], &ContentItem::assistant_text(weak));
    let boundary_ref = peer_reference(
        &prior,
        &ids[2],
        &ContentItem::assistant_text(boundary.clone()),
    );
    let dense_ref = peer_reference(&prior, &ids[3], &ContentItem::assistant_text(dense.clone()));
    let notes = root.path().join(".echidna/memory/notes");
    fs::create_dir_all(&notes).unwrap();
    fs::write(notes.join("candidate.md"), &source).unwrap();
    fs::write(root.path().join("candidate-source.txt"), &source).unwrap();
    let memory_ref = format!(
        "memory:notes/candidate.md:{}",
        blake3::hash(source.as_bytes()).to_hex()
    );
    let mut query_index = 0;
    let mut last_call = String::new();
    let mut seen = BTreeSet::new();
    let queries = ["abcdefg", "éé", "i\u{307}i\u{307}", "mixcase", "强势突破"];
    let markers = ["abcdefx", "ÉÉ", "İİ", "MixCase", "强势"];
    let mut search_started: Option<std::time::Instant> = None;
    let server = paged_server(move |step, wire| {
        if step == 0 {
            return (initial_selection_response(wire, "offset-selection"), false);
        }
        if step == 1 {
            return (
                m4_long_flow_tool_response(
                    "offset-publish",
                    "artifact_store",
                    json!({
                        "event_id":"offset-artifact-event", "occurred_at":"2026-11-05T00:00:00Z",
                        "artifact_id":"offset-artifact", "source_path":"candidate-source.txt",
                        "media_type":"text/plain", "toolchain_fingerprint":"fixture-v1"
                    }),
                    "offset-published",
                ),
                false,
            );
        }
        let mut cursor = Value::Null;
        if step > 2 {
            if query_index == 0 {
                eprintln!(
                    "dense weak-prefix search page elapsed: {:?}",
                    search_started.take().unwrap().elapsed()
                );
            }
            let result = delivered_result(wire, &last_call);
            assert_eq!(result["status"], "succeeded", "{result}");
            let response = &result["output"];
            for entry in response["entries"].as_array().unwrap() {
                let reference = entry["reference"].as_str().unwrap();
                assert_ne!(reference, weak_ref, "weak gram was accepted: {entry}");
                let original = if reference == session_ref
                    || reference == memory_ref
                    || (entry["scope"] == "artifact"
                        && reference.starts_with("artifact:offset-artifact:"))
                {
                    Some(source.as_str())
                } else if reference == boundary_ref {
                    Some(boundary.as_str())
                } else if reference == dense_ref {
                    Some(dense.as_str())
                } else {
                    None
                };
                if let Some(original) = original {
                    let offset = entry["offset"].as_u64().unwrap() as usize;
                    let text = entry["text"].as_str().unwrap();
                    assert!(original.is_char_boundary(offset), "{entry}");
                    assert_eq!(&original[offset..offset + text.len()], text, "{entry}");
                    assert!(text.contains(markers[query_index]), "{entry}");
                    seen.insert(reference.to_owned());
                }
            }
            cursor = response["next_cursor"].clone();
            if cursor.is_null() {
                assert!(
                    seen.contains(&session_ref),
                    "missing session for {}: {seen:?}",
                    queries[query_index]
                );
                assert!(
                    seen.contains(&memory_ref),
                    "missing memory for {}: {seen:?}",
                    queries[query_index]
                );
                assert!(
                    seen.iter()
                        .any(|item| item.starts_with("artifact:offset-artifact:")),
                    "missing artifact for {}: {seen:?}",
                    queries[query_index]
                );
                if query_index == 0 {
                    assert!(
                        seen.contains(&boundary_ref),
                        "missing cross-window source: {seen:?}"
                    );
                    assert!(
                        seen.contains(&dense_ref),
                        "missing dense-prefix source: {seen:?}"
                    );
                }
                seen.clear();
                query_index += 1;
                if query_index == queries.len() {
                    return (
                        text_response("Search offsets verified.", "offset-final"),
                        true,
                    );
                }
            }
        }
        last_call = format!("offset-search-{step}");
        if query_index == 0 {
            search_started = Some(std::time::Instant::now());
        }
        (
            m4_long_flow_tool_response(
                &last_call,
                "context_read",
                json!({"operation":"search", "query":queries[query_index], "cursor":cursor, "max_bytes":8192}),
                &format!("offset-response-{step}"),
            ),
            false,
        )
    });
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "Publish and search the synthetic sources.\n/exit\n",
        &["--agent", agent.to_str().unwrap(), "--debug"],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.join();
    assert!(String::from_utf8_lossy(&output.stdout).contains("Search offsets verified."));
    // This measures full-window eligibility/ranking calls, not the separate
    // bounded positional-index construction or total CPU time.
    let diagnostics = String::from_utf8_lossy(&output.stderr);
    let checks = diagnostics
        .lines()
        .filter(|line| line.contains("phase=context_read "))
        .map(|line| {
            line.split_whitespace()
                .find_map(|field| field.strip_prefix("rank_window_checks="))
                .unwrap()
                .parse::<usize>()
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert!(!checks.is_empty(), "{diagnostics}");
    assert!(
        checks.iter().sum::<usize>() < 128,
        "same-source dense prefix repeatedly ranked: {checks:?}"
    );
}

#[test]
fn context_search_exact_late_token_returns_readable_source_ranges() {
    check_late_search_token("abcdefg", "ab", "abcdefg", 2_000, 1024);
}

#[test]
fn context_search_fuzzy_late_token_returns_readable_small_ranges() {
    check_late_search_token("abcdefg", "ab", "abcdefx", 200, 64);
}

#[test]
fn context_search_fuzzy_late_token_fits_minimal_source_range() {
    check_late_search_token("abcdefg", "ab", "abcdefx", 200, 8);
}

#[test]
fn context_search_short_gap_fuzzy_ranges_cover_peer_memory_and_artifact() {
    check_late_search_token("abcdefghij", "ef", "abcdexghij", 20, 8);
}

#[test]
fn context_search_short_gap_fuzzy_range_covers_current_session() {
    check_current_fuzzy_range("abcdefghij", "ef", 20, "abcdexghij", 24, "abcdexgh");
}

#[test]
fn context_search_insertion_keeps_later_supported_range() {
    check_current_fuzzy_range(
        "fchbgdhfbfjaijibadhacifigaeg",
        "ib",
        8,
        "fchbgxxcifigaeg",
        19,
        "cifigaeg",
    );
}

#[test]
fn context_search_keeps_fuzzy_evidence_split_by_insertion() {
    // Neither four-character cluster alone reaches the ranker's 40% gram
    // threshold; the combined original source is nevertheless eligible.
    check_current_fuzzy_range("abcdefghij", "ij", 8, "abcdxxghij", 12, "abcdxxgh");
}

#[test]
fn context_search_prefers_locally_combined_split_evidence_to_weak_prefix() {
    check_current_fuzzy_range("abcdefghij", "abcd", 20, "abcxdefxghij", 30, "defxghij");
}

#[test]
fn context_search_fuzzy_evidence_survives_three_character_insertion() {
    check_current_fuzzy_range("abcdefghij", "ij", 8, "abcdxxxghij", 12, "abcdxxxg");
}

#[test]
fn context_search_rejects_separated_weak_clusters_before_later_current_hit() {
    check_current_separated_weak_clusters(format!(
        "abc {} def {} ghi",
        "z".repeat(200),
        "z".repeat(200)
    ));
}

#[test]
fn context_search_rejects_repeated_weak_cluster_before_later_current_hit() {
    check_current_separated_weak_clusters(format!(
        "{} {} def {} ghi",
        "abc".repeat(80),
        "z".repeat(200),
        "z".repeat(200)
    ));
}

#[test]
fn context_search_rejects_whitespace_separated_clusters_before_later_hit() {
    check_current_separated_weak_clusters("abc     def     ghi     ".to_owned());
}

fn check_current_separated_weak_clusters(weak: String) {
    use echidna::domain::model::ContentItem;

    let root = TemporaryDirectory::new("task-search-separated-current-clusters");
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
    let session = session_id(root.path());
    let project = Project::discover(root.path()).unwrap();
    let source = format!("{weak} {} abcdefghij", "z".repeat(20_000));
    let hit = source.find("abcdefghij").unwrap();
    let ids = append_source_items(
        &project,
        &session,
        vec![
            ContentItem::assistant_text(source.clone()),
            ContentItem::assistant_text(weak),
        ],
    );
    let positive = ids[0].clone();
    let negative = ids[1].clone();
    let mut call = String::new();
    let mut found: Option<(usize, String)> = None;
    let mut reading = false;
    let wait = Duration::from_secs(60);
    let server = paged_server_with_request_waits(wait, wait, move |step, wire| {
        if step == 0 {
            return (
                initial_selection_response(wire, "clusters-selection"),
                false,
            );
        }
        let mut cursor = Value::Null;
        if step > 1 {
            let result = delivered_result(wire, &call);
            assert_eq!(result["status"], "succeeded", "{result}");
            let response = &result["output"];
            if reading {
                let (offset, text) = found.as_ref().unwrap();
                let entry = &response["entries"][0];
                assert_eq!(entry["reference"], positive, "{entry}");
                assert_eq!(entry["scope"], "session", "{entry}");
                assert_eq!(entry["offset"], *offset, "{entry}");
                assert_eq!(entry["text"], *text, "{entry}");
                return (
                    text_response("Current cluster search verified.", "clusters-final"),
                    true,
                );
            }
            for entry in response["entries"].as_array().unwrap() {
                assert_ne!(
                    entry["reference"], negative,
                    "separated weak clusters matched: {entry}"
                );
                if entry["reference"] == positive {
                    assert_eq!(entry["scope"], "session", "{entry}");
                    let offset = entry["offset"].as_u64().unwrap() as usize;
                    let text = entry["text"].as_str().unwrap();
                    assert_eq!(&source[offset..offset + text.len()], text, "{entry}");
                    assert!(
                        offset >= hit && offset + text.len() <= hit + 10,
                        "weak prefix borrowed distant evidence: {entry}"
                    );
                    assert_eq!(text, "abcdefgh", "{entry}");
                    assert!(
                        found.replace((offset, text.to_owned())).is_none(),
                        "duplicate source: {entry}"
                    );
                }
            }
            cursor = response["next_cursor"].clone();
            if cursor.is_null() {
                let (offset, text) = found.as_ref().expect("later current Session hit missing");
                reading = true;
                call = format!("clusters-read-{step}");
                return (
                    m4_long_flow_tool_response(
                        &call,
                        "context_read",
                        json!({"operation":"read", "reference":positive, "offset":offset, "max_bytes":text.len()}),
                        &format!("clusters-read-response-{step}"),
                    ),
                    false,
                );
            }
        }
        call = format!("clusters-search-{step}");
        (
            m4_long_flow_tool_response(
                &call,
                "context_read",
                json!({"operation":"search", "query":"abcdefghij", "cursor":cursor, "max_bytes":8}),
                &format!("clusters-search-response-{step}"),
            ),
            false,
        )
    });
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "Search the synthetic current sources.\n/exit\n",
        &["--resume", "latest"],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.join();
    assert!(String::from_utf8_lossy(&output.stdout).contains("Current cluster search verified."));
}

#[test]
fn context_search_rejects_separated_weak_clusters_in_peer_memory_and_artifact() {
    use echidna::domain::model::ContentItem;
    use std::collections::{BTreeMap, BTreeSet};

    let root = TemporaryDirectory::new("task-search-separated-other-clusters");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let manifest = agent.join("agent.toml");
    fs::write(
        &manifest,
        format!(
            "{}artifact_store = \"direct\"\n",
            fs::read_to_string(&manifest)
                .unwrap()
                .replace("memory = \"none\"", "memory = \"project\"")
        ),
    )
    .unwrap();
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
    let weak = format!("abc {} def {} ghi", "z".repeat(200), "z".repeat(200));
    let source = format!("{weak} {} abcdefghij", "z".repeat(20_000));
    let hit = source.find("abcdefghij").unwrap();
    let items = [
        ContentItem::assistant_text(source.clone()),
        ContentItem::assistant_text(weak.clone()),
    ];
    let ids = append_source_items(&project, &prior, items.to_vec());
    let peer = peer_reference(&prior, &ids[0], &items[0]);
    let weak_peer = peer_reference(&prior, &ids[1], &items[1]);
    let notes = root.path().join(".echidna/memory/notes");
    fs::create_dir_all(&notes).unwrap();
    fs::write(notes.join("clusters-positive.md"), &source).unwrap();
    fs::write(notes.join("clusters-negative.md"), &weak).unwrap();
    let memory = format!(
        "memory:notes/clusters-positive.md:{}",
        blake3::hash(source.as_bytes()).to_hex()
    );
    let weak_memory = format!(
        "memory:notes/clusters-negative.md:{}",
        blake3::hash(weak.as_bytes()).to_hex()
    );
    let artifact = seed_search_artifact(
        &project,
        root.path(),
        &prior,
        "01",
        "clusters-positive",
        &source,
    );
    let weak_artifact = seed_search_artifact(
        &project,
        root.path(),
        &prior,
        "02",
        "clusters-negative",
        &weak,
    );
    let expected = BTreeMap::from([
        (peer, "agent_session"),
        (memory, "memory"),
        (artifact, "artifact"),
    ]);
    let negatives = BTreeSet::from([weak_peer, weak_memory, weak_artifact]);
    let mut seen = BTreeSet::new();
    let mut reads: Vec<(String, String, usize, String, String)> = Vec::new();
    let mut pending_read: Option<(String, String, usize, String, String)> = None;
    let mut search_pages = 0;
    let mut call = String::new();
    let wait = Duration::from_secs(60);
    let server = paged_server_with_request_waits(wait, wait, move |step, wire| {
        if step == 0 {
            return (
                initial_selection_response(wire, "other-clusters-selection"),
                false,
            );
        }
        let mut cursor = Value::Null;
        if step > 1 {
            let result = delivered_result(wire, &call);
            assert_eq!(result["status"], "succeeded", "{result}");
            let response = &result["output"];
            if let Some((reference, scope, offset, text, revision)) = pending_read.take() {
                let entry = &response["entries"][0];
                assert_eq!(entry["reference"], reference, "{entry}");
                assert_eq!(entry["scope"], scope, "{entry}");
                assert_eq!(entry["offset"], offset, "{entry}");
                assert_eq!(entry["text"], text, "{entry}");
                assert_eq!(entry["source_revision"], revision, "{entry}");
            } else {
                search_pages += 1;
                for entry in response["entries"].as_array().unwrap() {
                    let reference = entry["reference"].as_str().unwrap();
                    assert!(
                        !negatives.contains(reference),
                        "separated weak clusters matched: {entry}"
                    );
                    if let Some(scope) = expected.get(reference) {
                        assert_eq!(entry["scope"], *scope, "{entry}");
                        let offset = entry["offset"].as_u64().unwrap() as usize;
                        let text = entry["text"].as_str().unwrap();
                        assert_eq!(&source[offset..offset + text.len()], text, "{entry}");
                        assert!(
                            offset >= hit && offset + text.len() <= hit + 10,
                            "weak prefix borrowed distant evidence: {entry}"
                        );
                        assert_eq!(text, "abcdefgh", "{entry}");
                        assert!(
                            seen.insert(reference.to_owned()),
                            "duplicate source: {entry}"
                        );
                        reads.push((
                            reference.to_owned(),
                            (*scope).to_owned(),
                            offset,
                            text.to_owned(),
                            entry["source_revision"].as_str().unwrap().to_owned(),
                        ));
                    }
                }
                cursor = response["next_cursor"].clone();
                if cursor.is_null() {
                    assert_eq!(seen.len(), expected.len(), "missing scopes: {seen:?}");
                    assert!(
                        search_pages >= 3,
                        "the 8-byte budget must require continuation"
                    );
                }
            }
        }
        if step > 1 && cursor.is_null() {
            if let Some((reference, scope, offset, text, revision)) = reads.pop() {
                call = format!("other-clusters-read-{step}");
                let length = text.len();
                let response = m4_long_flow_tool_response(
                    &call,
                    "context_read",
                    json!({"operation":"read", "reference":reference, "offset":offset, "max_bytes":length}),
                    &format!("other-clusters-read-response-{step}"),
                );
                pending_read = Some((reference, scope, offset, text, revision));
                return (response, false);
            }
            return (
                text_response("Other cluster scopes verified.", "other-clusters-final"),
                true,
            );
        }
        call = format!("other-clusters-search-{step}");
        (
            m4_long_flow_tool_response(
                &call,
                "context_read",
                json!({"operation":"search", "query":"abcdefghij", "cursor":cursor, "max_bytes":8}),
                &format!("other-clusters-search-response-{step}"),
            ),
            false,
        )
    });
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "Search the synthetic archived sources.\n/exit\n",
        &["--agent", agent.to_str().unwrap(), "--debug"],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.join();
    let diagnostics = String::from_utf8_lossy(&output.stderr);
    let checks = diagnostics
        .lines()
        .filter(|line| line.contains("phase=context_read "))
        .map(|line| {
            line.split_whitespace()
                .find_map(|field| field.strip_prefix("rank_window_checks="))
                .unwrap()
                .parse::<usize>()
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert!(!checks.is_empty(), "{diagnostics}");
    assert!(
        checks.iter().sum::<usize>() < 128,
        "unexpected repeated ranking: {checks:?}"
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Other cluster scopes verified."));
}

#[test]
fn context_search_enumerates_more_than_ten_distinct_artifacts_across_pages() {
    use std::collections::{BTreeMap, BTreeSet};

    let root = TemporaryDirectory::new("task-search-many-distinct-artifacts");
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
    let marker = "KQJXARTIFACTMARKER";
    let mut expected = BTreeMap::new();
    for index in 0..11 {
        let body = format!("prefix {marker} synthetic source {index}");
        let reference = seed_search_artifact(
            &project,
            root.path(),
            &prior,
            &format!("{index:02}"),
            &format!("enumerated-{index:02}"),
            &body,
        );
        assert!(expected.insert(reference, body).is_none());
    }
    let mut seen = BTreeSet::new();
    let mut pages_with_artifacts = 0;
    let mut saved: Option<(String, usize, String)> = None;
    let mut reading = false;
    let mut call = String::new();
    let wait = Duration::from_secs(60);
    let server = paged_server_with_request_waits(wait, wait, move |step, wire| {
        if step == 0 {
            return (
                initial_selection_response(wire, "many-artifacts-selection"),
                false,
            );
        }
        let mut cursor = Value::Null;
        if step > 1 {
            let result = delivered_result(wire, &call);
            assert_eq!(result["status"], "succeeded", "{result}");
            let response = &result["output"];
            if reading {
                let (reference, offset, text) = saved.as_ref().unwrap();
                let entry = &response["entries"][0];
                assert_eq!(entry["reference"], *reference, "{entry}");
                assert_eq!(entry["scope"], "artifact", "{entry}");
                assert_eq!(entry["offset"], *offset, "{entry}");
                assert_eq!(entry["text"], *text, "{entry}");
                return (
                    text_response("Artifact enumeration verified.", "many-artifacts-final"),
                    true,
                );
            }
            let mut matched = 0;
            for entry in response["entries"].as_array().unwrap() {
                let reference = entry["reference"].as_str().unwrap();
                if let Some(body) = expected.get(reference) {
                    assert_eq!(entry["scope"], "artifact", "{entry}");
                    let offset = entry["offset"].as_u64().unwrap() as usize;
                    let text = entry["text"].as_str().unwrap();
                    assert_eq!(&body[offset..offset + text.len()], text, "{entry}");
                    assert!(text.contains(marker), "{entry}");
                    assert!(
                        seen.insert(reference.to_owned()),
                        "duplicate Artifact: {entry}"
                    );
                    saved = Some((reference.to_owned(), offset, text.to_owned()));
                    matched += 1;
                }
            }
            if matched > 0 {
                pages_with_artifacts += 1;
            }
            cursor = response["next_cursor"].clone();
            if cursor.is_null() {
                assert_eq!(seen.len(), 11, "missing Artifact sources: {seen:?}");
                assert!(
                    pages_with_artifacts >= 2,
                    "the 10-entry page limit was not crossed"
                );
                let (reference, offset, text) = saved.as_ref().unwrap();
                reading = true;
                call = format!("many-artifacts-read-{step}");
                return (
                    m4_long_flow_tool_response(
                        &call,
                        "context_read",
                        json!({"operation":"read", "reference":reference, "offset":offset, "max_bytes":text.len()}),
                        &format!("many-artifacts-read-response-{step}"),
                    ),
                    false,
                );
            }
        }
        call = format!("many-artifacts-search-{step}");
        (
            m4_long_flow_tool_response(
                &call,
                "context_read",
                json!({"operation":"search", "query":marker, "cursor":cursor, "max_bytes":1024}),
                &format!("many-artifacts-search-response-{step}"),
            ),
            false,
        )
    });
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "Search the synthetic artifact collection.\n/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.join();
    assert!(String::from_utf8_lossy(&output.stdout).contains("Artifact enumeration verified."));
}

fn seed_search_artifact(
    project: &Project,
    root: &std::path::Path,
    session: &str,
    label: &str,
    id: &str,
    body: &str,
) -> String {
    use echidna::domain::artifacts::{ArtifactEvent, ArtifactRecord, ArtifactScope, BlobId};
    use echidna::domain::tools::ToolCallId;

    let digest = format!("{:x}", Sha256::digest(body.as_bytes()));
    let blobs = root.join(".echidna/artifacts/blobs/sha256");
    fs::create_dir_all(&blobs).unwrap();
    fs::write(blobs.join(&digest), body).unwrap();
    append_research_event(
        project,
        label,
        ResearchEvent::Artifact(ArtifactEvent::Created(ArtifactRecord {
            id: ArtifactId::new(id),
            blob_id: BlobId::new(&digest).unwrap(),
            media_type: "text/plain".to_owned(),
            size_bytes: body.len() as u64,
            source_path: format!("{id}.txt"),
            created_by_event_id: EventId::new(format!("research-{label}")),
            produced_by_tool_call_id: ToolCallId::new(format!("{id}-call")),
            scope: ArtifactScope::Session {
                session_id: StreamId::new(session),
            },
            toolchain_fingerprint: "fixture".to_owned(),
            parent_artifact_ids: Vec::new(),
            transform: None,
        })),
    );
    format!("artifact:{id}:{digest}")
}

fn check_current_fuzzy_range(
    query: &'static str,
    weak: &'static str,
    gap: usize,
    marker: &'static str,
    expected_offset: usize,
    expected: &'static str,
) {
    use echidna::domain::model::ContentItem;

    let root = TemporaryDirectory::new("task-search-short-gap-current");
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
    let session = session_id(root.path());
    let project = Project::discover(root.path()).unwrap();
    let source = format!("{weak} {} {marker}", "z".repeat(gap));
    let ids = append_source_items(
        &project,
        &session,
        vec![ContentItem::assistant_text(source)],
    );
    let reference = ids[0].clone();
    let mut call = String::new();
    let mut found = false;
    let mut reading = false;
    let wait = Duration::from_secs(60);
    let server = paged_server_with_request_waits(wait, wait, move |step, wire| {
        if step == 0 {
            return (
                initial_selection_response(wire, "short-gap-selection"),
                false,
            );
        }
        let mut cursor = Value::Null;
        if step > 1 {
            let result = delivered_result(wire, &call);
            assert_eq!(result["status"], "succeeded", "{result}");
            let response = &result["output"];
            if reading {
                assert_eq!(response["entries"][0]["text"], expected, "{response}");
                return (
                    text_response("Short gap verified.", "short-gap-final"),
                    true,
                );
            }
            for entry in response["entries"].as_array().unwrap() {
                if entry["reference"] == reference {
                    assert_eq!(entry["offset"], expected_offset, "{entry}");
                    assert_eq!(entry["text"], expected, "{entry}");
                    found = true;
                }
            }
            cursor = response["next_cursor"].clone();
            if cursor.is_null() {
                assert!(found, "current Session source missing: {response}");
                reading = true;
                call = format!("short-gap-read-{step}");
                return (
                    m4_long_flow_tool_response(
                        &call,
                        "context_read",
                        json!({"operation":"read", "reference":reference, "offset":expected_offset, "max_bytes":8}),
                        &format!("short-gap-read-response-{step}"),
                    ),
                    false,
                );
            }
        }
        call = format!("short-gap-search-{step}");
        (
            m4_long_flow_tool_response(
                &call,
                "context_read",
                json!({"operation":"search", "query":query, "cursor":cursor, "max_bytes":8}),
                &format!("short-gap-search-response-{step}"),
            ),
            false,
        )
    });
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "Search the synthetic source.\n/exit\n",
        &["--resume", "latest"],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.join();
    assert!(String::from_utf8_lossy(&output.stdout).contains("Short gap verified."));
}

fn check_late_search_token(
    query: &'static str,
    weak: &'static str,
    marker: &'static str,
    gap: usize,
    max_bytes: usize,
) {
    use echidna::domain::model::ContentItem;
    use std::collections::BTreeSet;

    let root = TemporaryDirectory::new("task-search-exact-late-token");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let manifest = agent.join("agent.toml");
    fs::write(
        &manifest,
        format!(
            "{}artifact_store = \"direct\"\n",
            fs::read_to_string(&manifest)
                .unwrap()
                .replace("memory = \"none\"", "memory = \"project\"")
        ),
    )
    .unwrap();
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
    let source = format!("{weak} {} {marker}", "z".repeat(gap));
    let dense = format!("{} {marker}", format!("{weak} ").repeat(5_000));
    let ids = append_source_items(
        &project,
        &prior,
        vec![
            ContentItem::assistant_text(source.clone()),
            ContentItem::assistant_text(dense.clone()),
        ],
    );
    let session_ref = peer_reference(
        &prior,
        &ids[0],
        &ContentItem::assistant_text(source.clone()),
    );
    let dense_ref = peer_reference(&prior, &ids[1], &ContentItem::assistant_text(dense.clone()));
    let notes = root.path().join(".echidna/memory/notes");
    fs::create_dir_all(&notes).unwrap();
    fs::write(notes.join("exact.md"), &source).unwrap();
    fs::write(root.path().join("exact-source.txt"), &source).unwrap();
    let memory_ref = format!(
        "memory:notes/exact.md:{}",
        blake3::hash(source.as_bytes()).to_hex()
    );
    let mut call = String::new();
    let mut seen = BTreeSet::new();
    let mut reads: Vec<(String, usize, String)> = Vec::new();
    let mut pending_read: Option<String> = None;
    // This bounds fixture liveness; it is not a product latency target.
    let wait = Duration::from_secs(60);
    let server = paged_server_with_request_waits(wait, wait, move |step, wire| {
        if step == 0 {
            return (initial_selection_response(wire, "exact-selection"), false);
        }
        if step == 1 {
            return (
                m4_long_flow_tool_response(
                    "exact-publish",
                    "artifact_store",
                    json!({
                        "event_id":"exact-artifact-event", "occurred_at":"2026-11-05T00:00:00Z",
                        "artifact_id":"exact-artifact", "source_path":"exact-source.txt",
                        "media_type":"text/plain", "toolchain_fingerprint":"fixture-v1"
                    }),
                    "exact-published",
                ),
                false,
            );
        }
        let mut cursor = Value::Null;
        if step > 2 {
            let result = delivered_result(wire, &call);
            assert_eq!(result["status"], "succeeded", "{result}");
            let response = &result["output"];
            if let Some(expected) = pending_read.take() {
                assert_eq!(response["entries"][0]["text"], expected, "{response}");
            } else {
                for entry in response["entries"].as_array().unwrap() {
                    let reference = entry["reference"].as_str().unwrap();
                    let original = if reference == session_ref
                        || reference == memory_ref
                        || reference.starts_with("artifact:exact-artifact:")
                    {
                        Some(source.as_str())
                    } else if reference == dense_ref {
                        Some(dense.as_str())
                    } else {
                        None
                    };
                    if let Some(original) = original {
                        let offset = entry["offset"].as_u64().unwrap() as usize;
                        let text = entry["text"].as_str().unwrap();
                        assert_eq!(&original[offset..offset + text.len()], text, "{entry}");
                        assert!(
                            text.contains(&marker[..marker.len().min(max_bytes)]),
                            "{entry}"
                        );
                        if seen.insert(reference.to_owned()) {
                            reads.push((reference.to_owned(), offset, text.to_owned()));
                        }
                    }
                }
                cursor = response["next_cursor"].clone();
                if cursor.is_null() {
                    assert!(
                        seen.contains(&session_ref)
                            && seen.contains(&memory_ref)
                            && seen.contains(&dense_ref),
                        "{seen:?}"
                    );
                    assert!(
                        seen.iter()
                            .any(|reference| reference.starts_with("artifact:exact-artifact:")),
                        "{seen:?}"
                    );
                }
            }
        }
        if step > 2 && cursor.is_null() {
            if let Some((reference, offset, text)) = reads.pop() {
                call = format!("exact-read-{step}");
                let length = text.len();
                pending_read = Some(text);
                return (
                    m4_long_flow_tool_response(
                        &call,
                        "context_read",
                        json!({
                            "operation":"read", "reference":reference, "offset":offset, "max_bytes":length
                        }),
                        &format!("exact-read-response-{step}"),
                    ),
                    false,
                );
            }
            return (
                text_response("Exact source ranges verified.", "exact-final"),
                true,
            );
        }
        call = format!("exact-search-{step}");
        (
            m4_long_flow_tool_response(
                &call,
                "context_read",
                json!({
                    "operation":"search", "query":query, "cursor":cursor, "max_bytes":max_bytes
                }),
                &format!("exact-search-response-{step}"),
            ),
            false,
        )
    });
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "Publish and search synthetic sources.\n/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.join();
    assert!(String::from_utf8_lossy(&output.stdout).contains("Exact source ranges verified."));
}

#[test]
fn context_search_finds_single_character_tokens_without_substring_hits() {
    use echidna::domain::model::ContentItem;

    let root = TemporaryDirectory::new("task-search-single-character");
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
    let ids = append_source_items(
        &project,
        &prior,
        vec![
            ContentItem::assistant_text("x Été İ 强势 😀 @"),
            ContentItem::assistant_text("axb y e i 弱 🔥 #"),
        ],
    );
    let positive = ids[0].clone();
    let negative = ids[1].clone();
    let queries = ["x", "X", "é", "İ", "强", "😀", "@", "q"];
    let mut call = String::new();
    let mut query_index = 0;
    let server = paged_server(move |step, wire| {
        if step == 0 {
            return (initial_selection_response(wire, "single-selection"), false);
        }
        if step > 1 {
            let result = delivered_result(wire, &call);
            assert_eq!(result["status"], "succeeded", "{result}");
            let response = &result["output"];
            let references = response["entries"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|entry| entry["reference"].as_str())
                .collect::<Vec<_>>();
            if query_index < queries.len() - 1 {
                assert!(
                    references.contains(&positive.as_str()),
                    "query {}: {response}",
                    queries[query_index]
                );
            } else {
                assert!(!references.contains(&positive.as_str()), "{response}");
            }
            assert!(
                !references.contains(&negative.as_str()),
                "query {}: {response}",
                queries[query_index]
            );
            assert!(response["next_cursor"].is_null(), "{response}");
            query_index += 1;
            if query_index == queries.len() {
                return (
                    text_response("Single characters verified.", "single-final"),
                    true,
                );
            }
        }
        call = format!("single-search-{step}");
        (
            m4_long_flow_tool_response(
                &call,
                "context_read",
                json!({
                    "operation":"search", "query":queries[query_index], "max_bytes":8192
                }),
                &format!("single-response-{step}"),
            ),
            false,
        )
    });
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "Search synthetic symbols.\n/exit\n",
        &["--resume", "latest"],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.join();
    assert!(String::from_utf8_lossy(&output.stdout).contains("Single characters verified."));
}

#[test]
fn context_search_single_character_skips_long_non_token_and_finds_later_token() {
    use echidna::domain::model::ContentItem;

    let root = TemporaryDirectory::new("task-search-long-single-character");
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
    let repeated = "x".repeat(128 * 1024);
    let ids = append_source_items(
        &project,
        &prior,
        vec![
            ContentItem::assistant_text(repeated.clone()),
            ContentItem::assistant_text(format!("{repeated} x")),
        ],
    );
    let negative = ids[0].clone();
    let positive = ids[1].clone();
    let mut call = String::new();
    let mut seen_positive = false;
    let server = paged_server(move |step, wire| {
        if step == 0 {
            return (
                initial_selection_response(wire, "long-single-selection"),
                false,
            );
        }
        let mut cursor = Value::Null;
        if step > 1 {
            let result = delivered_result(wire, &call);
            assert_eq!(result["status"], "succeeded", "{result}");
            let response = &result["output"];
            for entry in response["entries"].as_array().unwrap() {
                let reference = entry["reference"].as_str().unwrap();
                assert_ne!(
                    reference, negative,
                    "repeated character is not a token: {entry}"
                );
                if reference == positive {
                    assert!(
                        entry["offset"].as_u64().unwrap() > 128 * 1024 - 64,
                        "{entry}"
                    );
                    assert!(entry["text"].as_str().unwrap().ends_with(" x"), "{entry}");
                    seen_positive = true;
                }
            }
            cursor = response["next_cursor"].clone();
            if cursor.is_null() {
                assert!(seen_positive, "later token was omitted: {response}");
                return (
                    text_response("Long single character verified.", "long-single-final"),
                    true,
                );
            }
        }
        call = format!("long-single-search-{step}");
        (
            m4_long_flow_tool_response(
                &call,
                "context_read",
                json!({
                    "operation":"search", "query":"x", "cursor":cursor, "max_bytes":64
                }),
                &format!("long-single-response-{step}"),
            ),
            false,
        )
    });
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "Search the synthetic source.\n/exit\n",
        &["--resume", "latest"],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.join();
    assert!(String::from_utf8_lossy(&output.stdout).contains("Long single character verified."));
}

#[test]
fn context_search_dense_negative_limits_rank_windows_and_finds_later_fuzzy_hit() {
    use echidna::domain::model::ContentItem;

    let root = TemporaryDirectory::new("task-search-dense-negative-work");
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
    let negative = format!("{} c!d!e!f!g", "ab".repeat(512 * 1024));
    let positive = format!("{} abcdefx", "ab".repeat(100));
    let ids = append_source_items(
        &project,
        &prior,
        vec![
            ContentItem::assistant_text(negative),
            ContentItem::assistant_text(positive),
        ],
    );
    let negative_ref = ids[0].clone();
    let positive_ref = ids[1].clone();
    let mut call = String::new();
    let mut seen_positive = false;
    // The 1 MiB debug fixture has observed 32-51 seconds of source work;
    // this bounded wait checks the mechanism, not latency acceptance.
    let server = paged_server_with_request_waits(
        Duration::from_secs(60),
        Duration::from_secs(60),
        move |step, wire| {
            if step == 0 {
                return (
                    initial_selection_response(wire, "dense-negative-selection"),
                    false,
                );
            }
            let mut cursor = Value::Null;
            if step > 1 {
                let result = delivered_result(wire, &call);
                assert_eq!(result["status"], "succeeded", "{result}");
                let response = &result["output"];
                for entry in response["entries"].as_array().unwrap() {
                    let reference = entry["reference"].as_str().unwrap();
                    assert_ne!(
                        reference, negative_ref,
                        "insufficient n-gram evidence: {entry}"
                    );
                    if reference == positive_ref {
                        assert!(
                            entry["text"].as_str().unwrap().contains("abcdefx"),
                            "{entry}"
                        );
                        seen_positive = true;
                    }
                }
                cursor = response["next_cursor"].clone();
                if cursor.is_null() {
                    assert!(seen_positive, "later fuzzy hit omitted: {response}");
                    return (
                        text_response("Dense negative verified.", "dense-negative-final"),
                        true,
                    );
                }
            }
            call = format!("dense-negative-search-{step}");
            (
                m4_long_flow_tool_response(
                    &call,
                    "context_read",
                    json!({
                        "operation":"search", "query":"abcdefg", "cursor":cursor, "max_bytes":64
                    }),
                    &format!("dense-negative-response-{step}"),
                ),
                false,
            )
        },
    );
    // The 1 MiB preserved History exceeds the 272k Resume preflight with
    // its ordinary reserves; use an explicit large fixture capability so the
    // test reaches the cold-search work it is intended to measure.
    let config = write_context_window_config(root.path(), &server.base_url, 1_000_000);
    let output = run_chat(
        root.path(),
        &config,
        "Search the synthetic archive.\n/exit\n",
        &["--resume", "latest", "--debug"],
    );
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    server.join();
    let diagnostics = String::from_utf8_lossy(&output.stderr);
    let counts = diagnostics
        .lines()
        .filter(|line| line.contains("phase=context_read "))
        .map(|line| {
            line.split_whitespace()
                .find_map(|field| field.strip_prefix("rank_window_checks="))
                .unwrap()
                .parse::<usize>()
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert!(!counts.is_empty(), "{diagnostics}");
    assert!(
        counts.iter().sum::<usize>() < 128,
        "unexpected repeated full-window ranking: {counts:?}"
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Dense negative verified."));
}

#[test]
fn context_search_reuses_windows_for_sparse_evidence_and_strict_identifier_suffixes() {
    use echidna::domain::model::ContentItem;

    let root = TemporaryDirectory::new("task-search-sparse-gram-windows");
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
    let sparse_weak = format!(
        "{} ab {}bc {}cd {}de {}ef",
        "ab ".repeat(3_000),
        "z ".repeat(80),
        "z ".repeat(80),
        "z ".repeat(80),
        "z ".repeat(80)
    );
    // The original sparse-only source borrowed eligibility from clusters
    // farther apart than the 64-byte readback. Keep those exact bytes as a
    // negative and add an independently supported later hit as the positive.
    let sparse = format!("{sparse_weak} {} abcdxefg", "z ".repeat(80));
    let strict_negative = "abcdef".repeat(200);
    let partial = format!("cve-{}abcdefx", "z".repeat(180));
    let ids = append_source_items(
        &project,
        &prior,
        vec![
            ContentItem::assistant_text(sparse.clone()),
            ContentItem::assistant_text(sparse_weak),
            ContentItem::assistant_text(strict_negative),
            ContentItem::assistant_text(partial.clone()),
        ],
    );
    let sparse_ref = ids[0].clone();
    let sparse_weak_ref = ids[1].clone();
    let negative_ref = ids[2].clone();
    let partial_ref = ids[3].clone();
    let mut call = String::new();
    let mut saw_sparse = false;
    let mut saw_partial = false;
    let mut sparse_range: Option<(usize, String)> = None;
    let mut reading = false;
    let server = paged_server(move |step, wire| {
        if step == 0 {
            return (
                initial_selection_response(wire, "sparse-window-selection"),
                false,
            );
        }
        let mut cursor = Value::Null;
        if step > 1 {
            let result = delivered_result(wire, &call);
            assert_eq!(result["status"], "succeeded", "{result}");
            let response = &result["output"];
            if reading {
                let (offset, text) = sparse_range.as_ref().unwrap();
                let entry = &response["entries"][0];
                assert_eq!(entry["reference"], sparse_ref, "{entry}");
                assert_eq!(entry["scope"], "session", "{entry}");
                assert_eq!(entry["offset"], *offset, "{entry}");
                assert_eq!(entry["text"], *text, "{entry}");
                assert!(entry["text"].as_str().unwrap().contains("abcdxefg"));
                return (
                    text_response("Sparse windows verified.", "sparse-window-final"),
                    true,
                );
            }
            for entry in response["entries"].as_array().unwrap() {
                let reference = entry["reference"].as_str().unwrap();
                assert_ne!(
                    reference, sparse_weak_ref,
                    "separated weak clusters must not borrow eligibility: {entry}"
                );
                assert_ne!(
                    reference, negative_ref,
                    "whole strict hex token must stay excluded: {entry}"
                );
                if reference == sparse_ref || reference == partial_ref {
                    let source = if reference == sparse_ref {
                        &sparse
                    } else {
                        &partial
                    };
                    let offset = entry["offset"].as_u64().unwrap() as usize;
                    let text = entry["text"].as_str().unwrap();
                    assert_eq!(&source[offset..offset + text.len()], text, "{entry}");
                    if reference == sparse_ref {
                        assert!(offset > 7_000 && text.contains("ab"), "{entry}");
                        assert!(text.contains("abcdxefg"), "{entry}");
                        saw_sparse = true;
                        sparse_range = Some((offset, text.to_owned()));
                    } else {
                        assert!(text.contains("abcdefx"), "{entry}");
                        saw_partial = true;
                    }
                }
            }
            cursor = response["next_cursor"].clone();
            if cursor.is_null() {
                assert!(
                    saw_sparse && saw_partial,
                    "sparse or identifier suffix omitted: {response}"
                );
                let (offset, text) = sparse_range.as_ref().unwrap();
                reading = true;
                call = format!("sparse-window-read-{step}");
                return (
                    m4_long_flow_tool_response(
                        &call,
                        "context_read",
                        json!({"operation":"read", "reference":sparse_ref, "offset":offset, "max_bytes":text.len()}),
                        &format!("sparse-window-read-response-{step}"),
                    ),
                    false,
                );
            }
        }
        call = format!("sparse-window-search-{step}");
        (
            m4_long_flow_tool_response(
                &call,
                "context_read",
                json!({
                    "operation":"search", "query":"abcdefg", "cursor":cursor, "max_bytes":64
                }),
                &format!("sparse-window-response-{step}"),
            ),
            false,
        )
    });
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "Search the synthetic sources.\n/exit\n",
        &["--resume", "latest", "--debug"],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.join();
    let diagnostics = String::from_utf8_lossy(&output.stderr);
    let checks = diagnostics
        .lines()
        .filter(|line| line.contains("phase=context_read "))
        .map(|line| {
            line.split_whitespace()
                .find_map(|field| field.strip_prefix("rank_window_checks="))
                .unwrap()
                .parse::<usize>()
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert!(!checks.is_empty(), "{diagnostics}");
    assert!(
        checks.iter().sum::<usize>() < 128,
        "unexpected repeated full-window ranking: {checks:?}"
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Sparse windows verified."));
}

#[test]
fn context_search_long_query_bounds_local_preparation_with_later_hits() {
    use echidna::domain::model::ContentItem;

    let root = TemporaryDirectory::new("task-search-long-query-work");
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
    let query = format!(
        "{}{}",
        "ab".repeat(300),
        "ghijklmnopqrstuvwxyz0123456789".repeat(8)
    );
    let fuzzy = format!("{}x", &query[..query.len() - 1]);
    let negative = "ab".repeat(8192);
    let strict_positive = format!("{negative} {fuzzy}");
    let nonhex_positive = format!("{negative}z {fuzzy}");
    let substring_positive = format!("{negative}z {query}z");
    let ids = append_source_items(
        &project,
        &prior,
        vec![
            ContentItem::assistant_text(negative),
            ContentItem::assistant_text(strict_positive.clone()),
            ContentItem::assistant_text(nonhex_positive.clone()),
            ContentItem::assistant_text(substring_positive.clone()),
        ],
    );
    let negative_ref = ids[0].clone();
    let strict_ref = ids[1].clone();
    let nonhex_ref = ids[2].clone();
    let substring_ref = ids[3].clone();
    let mut call = String::new();
    let mut seen = std::collections::BTreeSet::new();
    let server = paged_server_with_request_waits(
        Duration::from_secs(60),
        Duration::from_secs(60),
        move |step, wire| {
            if step == 0 {
                return (
                    initial_selection_response(wire, "long-query-selection"),
                    false,
                );
            }
            let mut cursor = Value::Null;
            if step > 1 {
                let result = delivered_result(wire, &call);
                assert_eq!(result["status"], "succeeded", "{result}");
                let response = &result["output"];
                for entry in response["entries"].as_array().unwrap() {
                    let reference = entry["reference"].as_str().unwrap();
                    assert_ne!(
                        reference, negative_ref,
                        "strict hex prefix alone must not match: {entry}"
                    );
                    let original = if reference == strict_ref {
                        Some(&strict_positive)
                    } else if reference == nonhex_ref {
                        Some(&nonhex_positive)
                    } else if reference == substring_ref {
                        Some(&substring_positive)
                    } else {
                        None
                    };
                    if let Some(original) = original {
                        let offset = entry["offset"].as_u64().unwrap() as usize;
                        let text = entry["text"].as_str().unwrap();
                        assert_eq!(&original[offset..offset + text.len()], text, "{entry}");
                        assert!(
                            offset > 16_000,
                            "the returned range missed the later fuzzy evidence: {entry}"
                        );
                        seen.insert(reference.to_owned());
                    }
                }
                cursor = response["next_cursor"].clone();
                if cursor.is_null() {
                    assert!(
                        seen.contains(&strict_ref)
                            && seen.contains(&nonhex_ref)
                            && seen.contains(&substring_ref),
                        "{response}"
                    );
                    return (
                        text_response("Long query work verified.", "long-query-final"),
                        true,
                    );
                }
            }
            call = format!("long-query-search-{step}");
            (
                m4_long_flow_tool_response(
                    &call,
                    "context_read",
                    json!({
                        "operation":"search", "query":query, "cursor":cursor, "max_bytes":64
                    }),
                    &format!("long-query-response-{step}"),
                ),
                false,
            )
        },
    );
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "Search the synthetic long-query sources.\n/exit\n",
        &["--resume", "latest", "--debug"],
    );
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    server.join();
    let diagnostics = String::from_utf8_lossy(&output.stderr);
    let mut totals = std::collections::BTreeMap::<&str, usize>::new();
    for line in diagnostics
        .lines()
        .filter(|line| line.contains("phase=context_read "))
    {
        for name in [
            "retrieval_source_window_bytes",
            "retrieval_index_source_bytes",
            "retrieval_needle_scan_bytes",
            "retrieval_candidate_anchors",
            "retrieval_local_gram_probes",
            "retrieval_window_gram_probes",
            "retrieval_rank_window_bytes",
            "rank_window_checks",
        ] {
            let amount = line
                .split_whitespace()
                .find_map(|field| field.strip_prefix(&format!("{name}=")))
                .unwrap()
                .parse::<usize>()
                .unwrap();
            *totals.entry(name).or_default() += amount;
        }
    }
    assert!(!totals.is_empty(), "{diagnostics}");
    assert!(
        totals["retrieval_source_window_bytes"] < 256 * 1024,
        "{totals:?}"
    );
    assert!(
        totals["retrieval_index_source_bytes"] < 256 * 1024,
        "{totals:?}"
    );
    assert!(
        totals["retrieval_needle_scan_bytes"] < 16 * 1024 * 1024,
        "{totals:?}"
    );
    assert!(
        totals["retrieval_candidate_anchors"] > 8_000
            && totals["retrieval_candidate_anchors"] < 100_000,
        "{totals:?}"
    );
    assert!(
        totals["retrieval_local_gram_probes"] < 2_000_000,
        "{totals:?}"
    );
    assert!(
        totals["retrieval_window_gram_probes"] < 2_000_000,
        "{totals:?}"
    );
    assert!(
        totals["retrieval_rank_window_bytes"] < 8 * 1024 * 1024,
        "{totals:?}"
    );
    assert!(totals["rank_window_checks"] < 128, "{totals:?}");
    assert!(String::from_utf8_lossy(&output.stdout).contains("Long query work verified."));
}

#[test]
fn context_search_high_diversity_grams_bound_work_and_keep_later_ranges() {
    use echidna::domain::model::ContentItem;

    let root = TemporaryDirectory::new("task-search-diverse-gram-windows");
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
    let mut state = 7_u32;
    let query = (0..768)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            char::from(b'g' + ((state >> 16) % 20) as u8)
        })
        .collect::<String>();
    let positive = query.repeat(22)[..16_384].to_owned();
    let negative = query[..200].repeat(82)[..16_384].to_owned();
    let grams = |text: &str| {
        [2, 3]
            .into_iter()
            .flat_map(|width| text.as_bytes().windows(width))
            .map(<[u8]>::to_vec)
            .collect::<std::collections::BTreeSet<_>>()
    };
    let query_grams = grams(&query);
    let negative_grams = grams(&negative);
    assert!(query_grams.len() > 1_000);
    assert!(query_grams.intersection(&negative_grams).count() * 5 < query_grams.len() * 2);
    let fuzzy = format!("{}x", &query[..767]);
    let later = format!("{} {fuzzy}", &negative[..12_000]);
    let ids = append_source_items(
        &project,
        &prior,
        vec![
            ContentItem::assistant_text(positive.clone()),
            ContentItem::assistant_text(negative),
            ContentItem::assistant_text(later.clone()),
        ],
    );
    let positive_ref = ids[0].clone();
    let negative_ref = ids[1].clone();
    let later_ref = ids[2].clone();
    let mut seen = std::collections::BTreeSet::new();
    let mut call = String::new();
    let server = paged_server_with_request_waits(
        Duration::from_secs(60),
        Duration::from_secs(60),
        move |step, wire| {
            if step == 0 {
                return (
                    initial_selection_response(wire, "diverse-gram-selection"),
                    false,
                );
            }
            let mut cursor = Value::Null;
            if step > 1 {
                let result = delivered_result(wire, &call);
                assert_eq!(result["status"], "succeeded", "{result}");
                let response = &result["output"];
                for entry in response["entries"].as_array().unwrap() {
                    let reference = entry["reference"].as_str().unwrap();
                    assert_ne!(
                        reference, negative_ref,
                        "sub-threshold grams matched: {entry}"
                    );
                    let original = if reference == positive_ref {
                        Some(&positive)
                    } else if reference == later_ref {
                        Some(&later)
                    } else {
                        None
                    };
                    if let Some(original) = original {
                        let offset = entry["offset"].as_u64().unwrap() as usize;
                        let text = entry["text"].as_str().unwrap();
                        assert_eq!(&original[offset..offset + text.len()], text, "{entry}");
                        if reference == later_ref {
                            assert!(
                                offset < original.len() && offset + text.len() > 12_001,
                                "{entry}"
                            );
                        }
                        seen.insert(reference.to_owned());
                    }
                }
                cursor = response["next_cursor"].clone();
                if cursor.is_null() {
                    assert!(
                        seen.contains(&positive_ref) && seen.contains(&later_ref),
                        "{response}"
                    );
                    return (
                        text_response("Diverse gram windows verified.", "diverse-gram-final"),
                        true,
                    );
                }
            }
            call = format!("diverse-gram-search-{step}");
            (
                m4_long_flow_tool_response(
                    &call,
                    "context_read",
                    json!({
                        "operation":"search", "query":query, "cursor":cursor, "max_bytes":64
                    }),
                    &format!("diverse-gram-response-{step}"),
                ),
                false,
            )
        },
    );
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "Search the synthetic sources.\n/exit\n",
        &["--resume", "latest", "--debug"],
    );
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    server.join();
    let diagnostics = String::from_utf8_lossy(&output.stderr);
    let mut totals = std::collections::BTreeMap::<&str, usize>::new();
    for line in diagnostics
        .lines()
        .filter(|line| line.contains("phase=context_read "))
    {
        for name in [
            "retrieval_candidate_anchors",
            "retrieval_local_gram_probes",
            "retrieval_window_gram_probes",
        ] {
            let amount = line
                .split_whitespace()
                .find_map(|field| field.strip_prefix(&format!("{name}=")))
                .unwrap()
                .parse::<usize>()
                .unwrap();
            *totals.entry(name).or_default() += amount;
        }
    }
    assert!(totals["retrieval_candidate_anchors"] > 20_000, "{totals:?}");
    // Probes count occurrences entering/leaving a maintained range,
    // cross-segment lookups and the bounded construction of local grams.
    // C40 scanned 361-1098 distinct posting keys at each of thousands of
    // anchors for these same sources.
    assert!(
        (1..2_000_000).contains(&totals["retrieval_local_gram_probes"]),
        "{totals:?}"
    );
    assert!(
        (1..2_000_000).contains(&totals["retrieval_window_gram_probes"]),
        "{totals:?}"
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Diverse gram windows verified."));
}

#[test]
fn context_search_exact_prepass_keeps_late_components_and_unicode_ranges() {
    use echidna::domain::model::ContentItem;

    let root = TemporaryDirectory::new("task-search-exact-prepass");
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
    let marker = "NXZQJRV984724TOKEN";
    let source = format!(
        "{} p{marker} {} pkg.{marker}.tail İİ",
        "z".repeat(8_192),
        "z".repeat(1_000)
    );
    let ids = append_source_items(
        &project,
        &prior,
        vec![
            ContentItem::assistant_text("z".repeat(32_768)),
            ContentItem::assistant_text(source.clone()),
        ],
    );
    let negative = ids[0].clone();
    let positive = ids[1].clone();
    let marker_offset = source.rfind(marker).unwrap();
    let unicode_offset = source.find("İİ").unwrap();
    let queries = [marker, "i\u{307}i\u{307}"];
    let offsets = [marker_offset, unicode_offset];
    let lengths = [marker.len(), "İİ".len()];
    let mut query_index = 0;
    let mut call = String::new();
    let mut seen = false;
    let server = paged_server(move |step, wire| {
        if step == 0 {
            return (
                initial_selection_response(wire, "exact-prepass-selection"),
                false,
            );
        }
        let mut cursor = Value::Null;
        if step > 1 {
            let result = delivered_result(wire, &call);
            assert_eq!(result["status"], "succeeded", "{result}");
            let response = &result["output"];
            for entry in response["entries"].as_array().unwrap() {
                let reference = entry["reference"].as_str().unwrap();
                assert_ne!(reference, negative, "unrelated filler matched: {entry}");
                if reference == positive {
                    let offset = entry["offset"].as_u64().unwrap() as usize;
                    let text = entry["text"].as_str().unwrap();
                    assert_eq!(&source[offset..offset + text.len()], text, "{entry}");
                    assert!(
                        offset <= offsets[query_index]
                            && offset + text.len() >= offsets[query_index] + lengths[query_index],
                        "{entry}"
                    );
                    seen = true;
                }
            }
            cursor = response["next_cursor"].clone();
            if cursor.is_null() {
                assert!(
                    seen,
                    "missing exact source for {}: {response}",
                    queries[query_index]
                );
                seen = false;
                query_index += 1;
                if query_index == queries.len() {
                    return (
                        text_response("Exact prepass verified.", "exact-prepass-final"),
                        true,
                    );
                }
            }
        }
        call = format!("exact-prepass-search-{step}");
        (
            m4_long_flow_tool_response(
                &call,
                "context_read",
                json!({
                    "operation":"search", "query":queries[query_index], "cursor":cursor, "max_bytes":64
                }),
                &format!("exact-prepass-response-{step}"),
            ),
            false,
        )
    });
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "Search the synthetic sources.\n/exit\n",
        &["--resume", "latest", "--debug"],
    );
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    server.join();
    let diagnostics = String::from_utf8_lossy(&output.stderr);
    let mut totals = std::collections::BTreeMap::<&str, usize>::new();
    for line in diagnostics
        .lines()
        .filter(|line| line.contains("phase=context_read "))
    {
        for name in [
            "retrieval_source_window_bytes",
            "retrieval_needle_scan_bytes",
            "retrieval_candidate_anchors",
        ] {
            let amount = line
                .split_whitespace()
                .find_map(|field| field.strip_prefix(&format!("{name}=")))
                .unwrap()
                .parse::<usize>()
                .unwrap();
            *totals.entry(name).or_default() += amount;
        }
    }
    // These counters establish that the negative filler was scanned with few
    // fuzzy anchors. They do not count the exact-prepass substring guard.
    assert!(
        totals["retrieval_source_window_bytes"] >= 32_768,
        "{totals:?}"
    );
    assert!(
        totals["retrieval_needle_scan_bytes"] > totals["retrieval_source_window_bytes"],
        "{totals:?}"
    );
    assert!(totals["retrieval_candidate_anchors"] < 1_000, "{totals:?}");
    assert!(String::from_utf8_lossy(&output.stdout).contains("Exact prepass verified."));
}
