//! Black-box cold-source discovery beyond one page and cursor scope changes.
use super::task_context::{
    ScriptedResponse, definition, initial_selection_response, new_selection_response,
    selector_candidates, spawn_scripted_responses_server,
};
use super::task_context_paging::{
    append_source_items, delivered_result, paged_server, paged_server_with_request_waits,
    peer_reference,
};
use super::*;
use echidna::domain::artifacts::{
    ArtifactEvent, ArtifactId, ArtifactRecord, ArtifactScope, BlobId,
};
use echidna::domain::model::ContentItem;
use echidna::domain::tools::ToolCallId;
use std::collections::BTreeMap;
use std::sync::Arc;

const ARTIFACT_METADATA_MARKER: &str = "隼炬穹烬霁簇岚";
const ARTIFACT_METADATA_FUZZY: &str = "隼炬穹烬霁簇霞";

#[test]
fn rebuilt_navigation_streams_real_same_input_source_refreshes() {
    let root = TemporaryDirectory::new("task-navigation-source-refresh-rebuild");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let server = spawn_scripted_responses_server(vec![
        ScriptedResponse::InitialSelection,
        ScriptedResponse::Response(
            200,
            m4_long_flow_tool_response(
                "navigation-refresh-write",
                "run_command",
                json!({"argv":["/bin/sh", "-c", "printf 'one\\n' > navigation-refresh.txt"]}),
                "navigation-refresh-tool",
            ),
        ),
        ScriptedResponse::Response(
            200,
            m4_long_flow_tool_response(
                "navigation-refresh-write-2",
                "run_command",
                json!({"argv":["/bin/sh", "-c", "printf 'two\\n' >> navigation-refresh.txt"]}),
                "navigation-refresh-tool-2",
            ),
        ),
        ScriptedResponse::Response(
            200,
            m4_long_flow_tool_response(
                "navigation-refresh-write-3",
                "run_command",
                json!({"argv":["/bin/sh", "-c", "printf 'three\\n' >> navigation-refresh.txt"]}),
                "navigation-refresh-tool-3",
            ),
        ),
        ScriptedResponse::Response(200, text_response("First done.", "navigation-first")),
        ScriptedResponse::NewSelection,
        ScriptedResponse::Response(200, text_response("Second done.", "navigation-second")),
    ]);
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "Write the NAVIGATION-ORIGINAL synthetic file.\nStart a different synthetic task.\n/exit\n",
        &["--agent", agent.to_str().unwrap(), "--debug"],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let wires = (0..7)
        .map(|_| f2a_request_body(&server.requests, "navigation source refresh"))
        .collect::<Vec<_>>();
    server.join();
    let events = session_events(root.path(), &session_id(root.path()));
    let selections = events
        .iter()
        .filter(|event| event["event_type"] == "session.task_context_selected")
        .collect::<Vec<_>>();
    assert!(selections.len() >= 5);
    assert_eq!(
        selections[1]["payload"]["refresh_of"],
        selections[0]["event_id"]
    );
    assert_eq!(
        selections[1]["payload"]["input_ref"],
        selections[0]["payload"]["input_ref"]
    );
    for pair in selections[..4].windows(2) {
        assert_eq!(pair[1]["payload"]["refresh_of"], pair[0]["event_id"]);
        assert_eq!(
            pair[1]["payload"]["task_revisions"],
            pair[0]["payload"]["task_revisions"]
        );
    }
    let id = selections[0]["payload"]["task_revisions"][0]["task_id"]
        .as_str()
        .unwrap();
    let candidates = selector_candidates(&wires[5]);
    let cards = candidates["tasks"].as_array().unwrap();
    assert_eq!(cards.iter().filter(|card| card["ref"] == id).count(), 1);
    assert!(cards.iter().any(|card| {
        card["ref"] == id
            && card["goal"]
                .as_str()
                .unwrap()
                .contains("NAVIGATION-ORIGINAL")
    }));
    let diagnostics = String::from_utf8_lossy(&output.stderr);
    let work = diagnostics
        .lines()
        .filter(|line| line.contains("phase=task_index "))
        .collect::<Vec<_>>();
    assert_eq!(work.len(), 5, "{diagnostics}");
    assert!(work[0].contains("metadata_rebuilt=true"), "{}", work[0]);
    for line in &work[1..] {
        assert!(line.contains("metadata_rebuilt=false"), "{line}");
    }
    let counter = |line: &str, name: &str| -> usize {
        line.split_whitespace()
            .find_map(|field| field.strip_prefix(&format!("{name}=")))
            .unwrap()
            .parse()
            .unwrap()
    };
    // Only the previous answer and the new input are added to the existing
    // source index. A fresh TaskIndex would re-index the earlier tool pair too.
    assert_eq!(counter(work[4], "source_rows"), 2, "{}", work[4]);
    for line in &work[2..] {
        for name in [
            "navigation_stage_read_bytes",
            "navigation_stage_write_bytes",
            "cards_written",
            "anchors_written",
        ] {
            assert_eq!(counter(line, name), 0, "{line}");
        }
    }
    let project = Project::discover(root.path()).unwrap();
    reduced_session(&project, &session_id(root.path()));
    // Resume uses verified Session replay, which independently reconstructs
    // each captured candidate set through the pure source policy and compares
    // it to the set produced by the actual disk-backed index above.
    let source = root
        .path()
        .join(".echidna/sessions")
        .join(session_id(root.path()))
        .join("events.jsonl");
    let before = fs::read(&source).unwrap();
    let resumed = run_chat(root.path(), &config, "/exit\n", &["--resume", "latest"]);
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    assert!(fs::read(source).unwrap().starts_with(&before));
    assert_eq!(
        fs::read_to_string(root.path().join("navigation-refresh.txt")).unwrap(),
        "one\ntwo\nthree\n"
    );
}

#[test]
fn task_posting_namespaces_keep_distinct_ordinals_and_reject_incomplete_rows() {
    for incomplete in [false, true] {
        let root = TemporaryDirectory::new("task-posting-namespace-swap");
        initialize_project(root.path());
        let agent = definition(root.path(), None);
        let scratch = root.path().join("scratch");
        fs::create_dir(&scratch).unwrap();
        let scratch_for_server = scratch.clone();
        let project_path = root.path().to_path_buf();
        let server = paged_server(move |step, wire| {
            if step == 0 {
                return (
                    new_selection_response(wire, "swap-initial-selection"),
                    false,
                );
            }
            match step - 1 {
                0 => (text_response("Seed complete.", "swap-seed"), false),
                1 => (new_selection_response(wire, "swap-select-target"), false),
                2 => (
                    m4_long_flow_tool_response(
                        "swap-write",
                        "run_command",
                        json!({"argv":["/bin/sh", "-c", "printf 'once\\n' > swap-effect.txt"]}),
                        "swap-tool",
                    ),
                    false,
                ),
                3 => {
                    let id = session_id(&project_path);
                    let events = session_events(&project_path, &id);
                    let selection = events
                        .iter()
                        .rev()
                        .find(|event| event["event_type"] == "session.task_context_selected")
                        .unwrap();
                    let task_id = selection["payload"]["task_revisions"][0]["task_id"]
                        .as_str()
                        .unwrap();
                    let digest = blake3::hash(task_id.as_bytes()).to_hex().to_string();
                    let indexes = fs::read_dir(&scratch_for_server)
                        .unwrap()
                        .map(|entry| entry.unwrap().path())
                        .filter(|path| {
                            path.file_name()
                                .unwrap()
                                .to_string_lossy()
                                .starts_with("echidna-task-index-")
                        })
                        .collect::<Vec<_>>();
                    assert_eq!(indexes.len(), 1);
                    let card = indexes[0].join("card-ref").join(&digest);
                    let anchor = indexes[0].join("anchor-post").join(&digest);
                    let card_bytes = fs::read(&card).unwrap();
                    let anchor_bytes = fs::read(&anchor).unwrap();
                    assert_eq!(card_bytes.len(), 8);
                    assert_eq!(anchor_bytes.len(), 8);
                    assert_ne!(&card_bytes[..8], &anchor_bytes[..8]);
                    if incomplete {
                        fs::write(card, &card_bytes[..7]).unwrap();
                    }
                    (
                        text_response("Target complete.", "swap-target-done"),
                        incomplete,
                    )
                }
                4 => (new_selection_response(wire, "swap-select-next"), false),
                5 => (
                    text_response("Control complete.", "swap-control-done"),
                    true,
                ),
                _ => panic!("unexpected namespace swap request"),
            }
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
                "--debug",
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
            .write_all(b"Seed task.\nWrite target file.\nStart next task.\n/exit\n")
            .unwrap();
        let output = child.wait_with_output().unwrap();
        server.join();
        let stderr = String::from_utf8_lossy(&output.stderr);
        if incomplete {
            assert!(!output.status.success(), "{stderr}");
            assert!(stderr.contains("task posting length changed"), "{stderr}");
        } else {
            assert!(output.status.success(), "{stderr}");
            let resumed = run_chat(root.path(), &config, "/exit\n", &["--resume", "latest"]);
            assert!(
                resumed.status.success(),
                "{}",
                String::from_utf8_lossy(&resumed.stderr)
            );
        }
        assert_eq!(
            fs::read_to_string(root.path().join("swap-effect.txt")).unwrap(),
            "once\n"
        );
    }
}

fn search_response(call: &str, query: &str, cursor: Value) -> String {
    m4_long_flow_tool_response(
        call,
        "context_read",
        json!({"operation":"search", "query":query, "cursor":cursor, "max_bytes":1024}),
        &format!("response-{call}"),
    )
}

fn seed_large_artifact_metadata(
    project: &Project,
    root: &std::path::Path,
    session: &str,
    media_type: &str,
) -> String {
    let bytes = b"small synthetic artifact body";
    let digest = format!("{:x}", Sha256::digest(bytes));
    let blob_dir = root.join(".echidna/artifacts/blobs/sha256");
    fs::create_dir_all(&blob_dir).unwrap();
    fs::write(blob_dir.join(&digest), bytes).unwrap();
    let mut large_metadata = String::new();
    for (label, id, fingerprint) in [
        (
            "01",
            "a-large-metadata",
            format!("{} {ARTIFACT_METADATA_MARKER}", "\"".repeat(2_200_000)),
        ),
        (
            "02",
            "z-later-metadata",
            format!("{ARTIFACT_METADATA_MARKER} small later source"),
        ),
    ] {
        let record = ArtifactRecord {
            id: ArtifactId::new(id),
            blob_id: BlobId::new(&digest).unwrap(),
            media_type: media_type.to_owned(),
            size_bytes: bytes.len() as u64,
            source_path: format!("fixture-{id}"),
            created_by_event_id: EventId::new(format!("research-{label}")),
            produced_by_tool_call_id: ToolCallId::new(format!("call-{id}")),
            scope: ArtifactScope::Session {
                session_id: StreamId::new(session),
            },
            toolchain_fingerprint: fingerprint,
            parent_artifact_ids: Vec::new(),
            transform: None,
        };
        let metadata = serde_json::to_string(&record).unwrap();
        let marker_offset = metadata.find(ARTIFACT_METADATA_MARKER).unwrap();
        let chars = ARTIFACT_METADATA_MARKER.chars().collect::<Vec<_>>();
        for width in [2, 3] {
            for part in chars.windows(width) {
                let ngram = part.iter().collect::<String>();
                assert!(!metadata[..marker_offset].contains(&ngram));
                assert!(!format!("artifact:{id}:{digest}").contains(&ngram));
            }
        }
        if label == "01" {
            assert!(metadata.len() > 4 * 1024 * 1024);
            large_metadata = metadata;
        }
        append_research_event(
            project,
            label,
            ResearchEvent::Artifact(ArtifactEvent::Created(record)),
        );
    }
    large_metadata
}

#[test]
fn cold_search_chunks_large_escaped_artifact_metadata_and_reaches_later_source() {
    for media in ["text/plain", "application/octet-stream"] {
        let root = TemporaryDirectory::new("task-cold-large-artifact-metadata");
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
        let project = Project::discover(root.path()).unwrap();
        seed_large_artifact_metadata(&project, root.path(), &session_id(root.path()), media);
        let mut call = String::new();
        let mut seen = BTreeSet::new();
        let mut pages = 0;
        let mut fuzzy = false;
        let metadata_prefix = "artifact-metadata:";
        let server = paged_server_with_request_waits(
            Duration::from_secs(60),
            Duration::from_secs(60),
            move |step, wire| {
                if step == 0 {
                    return (
                        new_selection_response(wire, "metadata-initial-selection"),
                        false,
                    );
                }
                let step = step - 1;
                if step > 0 {
                    let result = delivered_result(wire, &call);
                    assert_eq!(result["status"], "succeeded", "{result}");
                    let response = &result["output"];
                    pages += 1;
                    for entry in response["entries"].as_array().unwrap() {
                        assert_eq!(entry["scope"], "artifact_metadata");
                        assert!(
                            entry["text"]
                                .as_str()
                                .unwrap()
                                .contains(ARTIFACT_METADATA_MARKER),
                            "{entry}"
                        );
                        let reference = entry["reference"].as_str().unwrap();
                        assert!(seen.insert(reference.to_owned()), "duplicate: {entry}");
                    }
                    let cursor = response["next_cursor"].clone();
                    if cursor.is_null() {
                        assert!(pages >= 2, "large metadata must span pages");
                        assert_eq!(seen.len(), 2);
                        assert!(seen.iter().any(|reference| {
                            reference.starts_with(&format!("{metadata_prefix}a-large-metadata:"))
                        }));
                        assert!(seen.iter().any(|reference| {
                            reference.starts_with(&format!("{metadata_prefix}z-later-metadata:"))
                        }));
                        if !fuzzy {
                            fuzzy = true;
                            seen.clear();
                            pages = 0;
                            call = format!("metadata-fuzzy-{step}");
                            return (
                                search_response(&call, ARTIFACT_METADATA_FUZZY, Value::Null),
                                false,
                            );
                        }
                        return (
                            text_response("LARGE-METADATA-FOUND", "large-metadata-done"),
                            true,
                        );
                    }
                    call = format!("metadata-page-{step}");
                    return (
                        search_response(
                            &call,
                            if fuzzy {
                                ARTIFACT_METADATA_FUZZY
                            } else {
                                ARTIFACT_METADATA_MARKER
                            },
                            cursor,
                        ),
                        false,
                    );
                }
                call = "metadata-page-0".to_owned();
                (
                    search_response(&call, ARTIFACT_METADATA_MARKER, Value::Null),
                    false,
                )
            },
        );
        let config = write_context_window_config(root.path(), &server.base_url, 272_000);
        let output = run_chat(
            root.path(),
            &config,
            "Search the archived artifacts.\n/exit\n",
            &["--agent", agent.to_str().unwrap()],
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        server.join();
        assert!(String::from_utf8_lossy(&output.stdout).contains("LARGE-METADATA-FOUND"));
    }
}

#[test]
fn artifact_metadata_search_reference_reads_same_json_across_ranges() {
    for media in ["text/plain", "application/octet-stream"] {
        let root = TemporaryDirectory::new("task-artifact-metadata-reference");
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
        let project = Project::discover(root.path()).unwrap();
        let metadata =
            seed_large_artifact_metadata(&project, root.path(), &session_id(root.path()), media);
        let digest = format!("{:x}", Sha256::digest(b"small synthetic artifact body"));
        let metadata_revision = blake3::hash(metadata.as_bytes()).to_hex().to_string();
        let body_reference = format!("artifact:a-large-metadata:{digest}");
        let expected_reference =
            format!("artifact-metadata:a-large-metadata:{digest}:{metadata_revision}");
        let is_text = media == "text/plain";
        let mut call = String::new();
        let mut phase = 0;
        let mut offset = 0;
        let server = paged_server_with_request_waits(
            Duration::from_secs(60),
            Duration::from_secs(60),
            move |step, wire| {
                if step == 0 {
                    return (
                        new_selection_response(wire, "metadata-read-initial-selection"),
                        false,
                    );
                }
                let step = step - 1;
                let mut cursor = Value::Null;
                if step > 0 {
                    let result = delivered_result(wire, &call);
                    assert_eq!(result["status"], "succeeded", "{result}");
                    let response = &result["output"];
                    if phase == 0 {
                        if let Some(entry) = response["entries"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .find(|entry| entry["reference"] == expected_reference)
                        {
                            assert_eq!(entry["scope"], "artifact_metadata");
                            assert_eq!(entry["source_revision"], metadata_revision);
                            assert_eq!(entry["total_bytes"], metadata.len());
                            offset = entry["offset"].as_u64().unwrap() as usize;
                            let text = entry["text"].as_str().unwrap();
                            assert_eq!(text, &metadata[offset..offset + text.len()]);
                            phase = 1;
                            call = "metadata-read-first".to_owned();
                            return (
                                m4_long_flow_tool_response(
                                    &call,
                                    "context_read",
                                    json!({
                                        "operation":"read", "reference":expected_reference, "offset":offset, "max_bytes":64
                                    }),
                                    "metadata-read-first-response",
                                ),
                                false,
                            );
                        }
                        cursor = response["next_cursor"].clone();
                        assert!(cursor.is_string(), "{response}");
                    } else if phase <= 2 {
                        let entry = &response["entries"][0];
                        assert_eq!(entry["reference"], expected_reference);
                        assert_eq!(entry["scope"], "artifact_metadata");
                        assert_eq!(entry["source_revision"], metadata_revision);
                        assert_eq!(entry["total_bytes"], metadata.len());
                        assert_eq!(entry["offset"], offset);
                        let text = entry["text"].as_str().unwrap();
                        assert_eq!(text, &metadata[offset..offset + text.len()]);
                        let next = entry["next_offset"].as_u64().unwrap() as usize;
                        assert_eq!(next, offset + text.len());
                        if phase == 1 {
                            offset = next;
                            phase = 2;
                            call = "metadata-read-next".to_owned();
                            return (
                                m4_long_flow_tool_response(
                                    &call,
                                    "context_read",
                                    json!({
                                        "operation":"read", "reference":expected_reference, "offset":offset, "max_bytes":64
                                    }),
                                    "metadata-read-next-response",
                                ),
                                false,
                            );
                        }
                        phase = 3;
                        call = "legacy-artifact-read".to_owned();
                        return (
                            m4_long_flow_tool_response(
                                &call,
                                "context_read",
                                json!({
                                    "operation":"read", "reference":body_reference, "offset":0, "max_bytes":64
                                }),
                                "legacy-artifact-read-response",
                            ),
                            false,
                        );
                    } else if phase == 3 {
                        let entry = &response["entries"][0];
                        assert_eq!(entry["reference"], body_reference);
                        assert_eq!(entry["source_revision"], digest);
                        if is_text {
                            assert_eq!(entry["scope"], "artifact");
                            assert_eq!(entry["text"], "small synthetic artifact body");
                        } else {
                            assert_eq!(entry["scope"], "artifact_metadata");
                            let text = entry["text"].as_str().unwrap();
                            assert_eq!(text, &metadata[..text.len()]);
                        }
                        phase = 4;
                        call = "stale-metadata-read".to_owned();
                        let stale = format!(
                            "artifact-metadata:a-large-metadata:{digest}:{}",
                            "0".repeat(64)
                        );
                        return (
                            m4_long_flow_tool_response(
                                &call,
                                "context_read",
                                json!({
                                    "operation":"read", "reference":stale, "offset":0, "max_bytes":64
                                }),
                                "stale-metadata-response",
                            ),
                            false,
                        );
                    } else {
                        assert_eq!(response["status"], "stale", "{response}");
                        assert!(response["entries"].as_array().unwrap().is_empty());
                        return (
                            text_response("ARTIFACT-METADATA-READ-COMPLETE", "metadata-read-done"),
                            true,
                        );
                    }
                }
                call = format!("metadata-search-{step}");
                (
                    search_response(&call, ARTIFACT_METADATA_MARKER, cursor),
                    false,
                )
            },
        );
        let config = write_context_window_config(root.path(), &server.base_url, 272_000);
        let output = run_chat(
            root.path(),
            &config,
            "Search and read the archived metadata.\n/exit\n",
            &["--agent", agent.to_str().unwrap()],
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        server.join();
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("ARTIFACT-METADATA-READ-COMPLETE")
        );
    }
}

#[test]
fn small_search_budget_preserves_hits_and_empty_memory_identifier() {
    let root = TemporaryDirectory::new("task-context-small-excerpt");
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
    let session_text = format!("{} Kx exact session source", "z".repeat(999));
    let ids = append_source_items(
        &project,
        &prior,
        vec![ContentItem::assistant_text(&session_text)],
    );
    let session_reference =
        peer_reference(&prior, &ids[0], &ContentItem::assistant_text(&session_text));
    let notes = root.path().join(".echidna/memory/notes");
    fs::create_dir_all(&notes).unwrap();
    let memory_text = format!("{} MEMORYBOUND987 exact memory source", "z".repeat(999));
    fs::write(notes.join("small.md"), &memory_text).unwrap();
    fs::write(notes.join("empty.md"), "").unwrap();
    let memory_reference = format!(
        "memory:notes/small.md:{}",
        blake3::hash(memory_text.as_bytes()).to_hex()
    );
    let empty_reference = format!("memory:notes/empty.md:{}", blake3::hash(b"").to_hex());
    let artifact_text = format!("{} ARTIFACTBOUND987 exact artifact source", "z".repeat(999));
    let digest = format!("{:x}", Sha256::digest(artifact_text.as_bytes()));
    let blobs = root.path().join(".echidna/artifacts/blobs/sha256");
    fs::create_dir_all(&blobs).unwrap();
    fs::write(blobs.join(&digest), &artifact_text).unwrap();
    let artifact_reference = format!("artifact:small-budget-source:{digest}");
    append_research_event(
        &project,
        "01",
        ResearchEvent::Artifact(ArtifactEvent::Created(ArtifactRecord {
            id: ArtifactId::new("small-budget-source"),
            blob_id: BlobId::new(&digest).unwrap(),
            media_type: "text/plain".to_owned(),
            size_bytes: artifact_text.len() as u64,
            source_path: "small-budget-source.txt".to_owned(),
            created_by_event_id: EventId::new("research-01"),
            produced_by_tool_call_id: ToolCallId::new("small-budget-call"),
            scope: ArtifactScope::Session {
                session_id: StreamId::new(&prior),
            },
            toolchain_fingerprint: "fixture".to_owned(),
            parent_artifact_ids: Vec::new(),
            transform: None,
        })),
    );
    let cases = [
        ("kx", "Kx", session_reference, session_text, "agent_session"),
        (
            "MEMORYBOUND987",
            "MEMORYBOUND987",
            memory_reference,
            memory_text,
            "memory",
        ),
        (
            "ARTIFACTBOUND987",
            "ARTIFACTBOUND987",
            artifact_reference,
            artifact_text,
            "artifact",
        ),
        (
            "notes/empty.md",
            "",
            empty_reference,
            String::new(),
            "memory",
        ),
    ];
    let mut call = String::new();
    let mut case_index = 0;
    let mut empty_read = false;
    let server = paged_server(move |step, wire| {
        if step == 0 {
            return (
                new_selection_response(wire, "small-excerpt-selection"),
                false,
            );
        }
        let mut cursor = Value::Null;
        if step > 1 {
            let result = delivered_result(wire, &call);
            assert_eq!(result["status"], "succeeded", "{result}");
            if empty_read {
                let response = &result["output"];
                assert_eq!(response["status"], "found", "{response}");
                let entry = &response["entries"][0];
                assert_eq!(entry["reference"], cases[3].2);
                assert_eq!(entry["scope"], "memory");
                assert_eq!(entry["text"], "");
                assert_eq!(entry["total_bytes"], 0);
                assert!(entry["next_offset"].is_null());
                return (
                    text_response("SMALL-EXCERPT-COMPLETE", "small-excerpt-done"),
                    true,
                );
            }
            let (query, needle, reference, original, scope) = &cases[case_index];
            let response = &result["output"];
            let entry = response["entries"]
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| entry["reference"] == *reference);
            if let Some(entry) = entry {
                assert_eq!(entry["scope"], *scope);
                let offset = entry["offset"].as_u64().unwrap() as usize;
                let text = entry["text"].as_str().unwrap();
                assert!(
                    text.len() <= 64 && text.contains(*needle),
                    "{query}: {entry}"
                );
                assert_eq!(text, &original[offset..offset + text.len()]);
                assert_eq!(entry["total_bytes"], original.len());
                if case_index + 1 == cases.len() {
                    assert_eq!(text, "");
                    assert!(entry["next_offset"].is_null());
                    empty_read = true;
                    call = "empty-memory-read".to_owned();
                    return (
                        m4_long_flow_tool_response(
                            &call,
                            "context_read",
                            json!({
                                "operation":"read", "reference":reference, "offset":0, "max_bytes":64
                            }),
                            "empty-memory-read-response",
                        ),
                        false,
                    );
                }
                case_index += 1;
            } else {
                cursor = response["next_cursor"].clone();
                assert!(cursor.is_string(), "missing {query}: {response}");
            }
        }
        let query = cases[case_index].0;
        call = format!("small-excerpt-{step}");
        (
            m4_long_flow_tool_response(
                &call,
                "context_read",
                json!({
                    "operation":"search", "query":query, "cursor":cursor, "max_bytes":64
                }),
                &format!("small-excerpt-response-{step}"),
            ),
            false,
        )
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
    assert!(String::from_utf8_lossy(&output.stdout).contains("SMALL-EXCERPT-COMPLETE"));
}

#[test]
fn utf8_hit_is_deferred_when_an_earlier_result_uses_the_page_budget() {
    for memory in [true, false] {
        let root = TemporaryDirectory::new("task-utf8-page-budget");
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
        let memory_root = root.path().join(".echidna/memory");
        fs::create_dir_all(&memory_root).unwrap();
        if memory {
            fs::write(memory_root.join("decisions.md"), "kneedle").unwrap();
            fs::write(memory_root.join("failed-paths.md"), "Kneedle").unwrap();
        } else {
            fs::write(memory_root.join("decisions.md"), "unrelated fixture").unwrap();
            append_source_items(
                &project,
                &prior,
                vec![
                    ContentItem::assistant_text("kneedle"),
                    ContentItem::assistant_text("Kneedle"),
                ],
            );
        }
        let expected_scope = if memory { "memory" } else { "agent_session" };
        let mut call = String::new();
        let server = paged_server(move |step, wire| {
            if step == 0 {
                return (new_selection_response(wire, "utf8-budget-selection"), false);
            }
            let mut cursor = Value::Null;
            let mut maximum = 9;
            if step > 1 {
                let result = delivered_result(wire, &call);
                if step == 5 {
                    assert_eq!(result["status"], "failed", "{result}");
                    assert!(
                        result
                            .to_string()
                            .contains("too small for the next UTF-8 character"),
                        "{result}"
                    );
                    return (
                        text_response("UTF8-PAGE-BUDGET-COMPLETE", "utf8-budget-done"),
                        true,
                    );
                }
                assert_eq!(result["status"], "succeeded", "{result}");
                let response = &result["output"];
                let entries = response["entries"].as_array().unwrap();
                assert_eq!(entries.len(), 1, "{response}");
                assert_eq!(entries[0]["scope"], expected_scope);
                if step == 2 || step == 4 {
                    assert_eq!(entries[0]["text"], "kneedle");
                    cursor = response["next_cursor"].clone();
                    assert!(cursor.is_string(), "{response}");
                    if step == 4 {
                        maximum = 1;
                    }
                } else {
                    assert_eq!(entries[0]["text"], "Kneedle");
                    assert_eq!(entries[0]["offset"], 0);
                }
            }
            call = format!("utf8-page-{step}");
            (
                m4_long_flow_tool_response(
                    &call,
                    "context_read",
                    json!({
                        "operation":"search", "query":"kneedle", "cursor":cursor, "max_bytes":maximum
                    }),
                    &format!("utf8-page-response-{step}"),
                ),
                false,
            )
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
        assert!(String::from_utf8_lossy(&output.stdout).contains("UTF8-PAGE-BUDGET-COMPLETE"));
    }
}

#[test]
fn artifact_structured_media_types_read_and_search_original_body() {
    let root = TemporaryDirectory::new("task-artifact-structured-media");
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
    let blobs = root.path().join(".echidna/artifacts/blobs/sha256");
    fs::create_dir_all(&blobs).unwrap();
    let mut cases = Vec::new();
    for (index, media) in [
        "application/problem+json",
        "application/vnd.test+json",
        "application/vnd.test+xml",
        "application/json; charset=utf-8",
        "application/octet-stream",
    ]
    .into_iter()
    .enumerate()
    {
        let id = format!("media-{index}");
        let marker = format!("BODYMARKER742VAR{index}");
        let body = format!("prefix {marker} exact data");
        let digest = format!("{:x}", Sha256::digest(body.as_bytes()));
        fs::write(blobs.join(&digest), &body).unwrap();
        let label = format!("{index:02}");
        let record = ArtifactRecord {
            id: ArtifactId::new(&id),
            blob_id: BlobId::new(&digest).unwrap(),
            media_type: media.to_owned(),
            size_bytes: body.len() as u64,
            source_path: format!("{id}.dat"),
            created_by_event_id: EventId::new(format!("research-{label}")),
            produced_by_tool_call_id: ToolCallId::new(format!("{id}-call")),
            scope: ArtifactScope::Session {
                session_id: StreamId::new(&prior),
            },
            toolchain_fingerprint: "fixture".to_owned(),
            parent_artifact_ids: Vec::new(),
            transform: None,
        };
        append_research_event(
            &project,
            &label,
            ResearchEvent::Artifact(ArtifactEvent::Created(record)),
        );
        cases.push((
            format!("artifact:{id}:{digest}"),
            digest,
            marker,
            body,
            media.to_owned(),
        ));
    }
    let mut call = String::new();
    let server = paged_server(move |step, wire| {
        if step == 0 {
            return (
                new_selection_response(wire, "structured-media-initial-selection"),
                false,
            );
        }
        let step = step - 1;
        if step > 0 {
            let result = delivered_result(wire, &call);
            assert_eq!(result["status"], "succeeded", "{result}");
            let index = (step - 1) / 2;
            let (reference, digest, marker, body, media) = &cases[index];
            let response = &result["output"];
            if step % 2 == 1 {
                let entry = &response["entries"][0];
                assert_eq!(entry["reference"], *reference);
                assert_eq!(entry["source_revision"], *digest);
                if media == "application/octet-stream" {
                    assert_eq!(entry["scope"], "artifact_metadata");
                    assert!(entry["text"].as_str().unwrap().contains(media));
                } else {
                    assert_eq!(entry["scope"], "artifact");
                    assert_eq!(entry["offset"], 7);
                    assert_eq!(entry["text"], &body[7..]);
                }
            } else if media == "application/octet-stream" {
                assert!(
                    !response["entries"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|entry| entry["reference"] == *reference),
                    "{response}"
                );
            } else {
                let entry = response["entries"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|entry| entry["reference"] == *reference)
                    .unwrap_or_else(|| panic!("{response}"));
                assert_eq!(entry["scope"], "artifact");
                assert_eq!(entry["source_revision"], *digest);
                assert!(entry["text"].as_str().unwrap().contains(marker.as_str()));
            }
            if step == cases.len() * 2 {
                return (
                    text_response("STRUCTURED-MEDIA-COMPLETE", "structured-media-done"),
                    true,
                );
            }
        }
        let index = step / 2;
        let (reference, _, marker, _, media) = &cases[index];
        call = format!("media-{step}");
        let args = if step % 2 == 0 {
            json!({"operation":"read", "reference":reference, "offset":if media == "application/octet-stream" { 0 } else { 7 }, "max_bytes":512})
        } else {
            json!({"operation":"search", "query":marker, "max_bytes":512})
        };
        (
            m4_long_flow_tool_response(
                &call,
                "context_read",
                args,
                &format!("media-response-{step}"),
            ),
            false,
        )
    });
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "Read and search the synthetic artifacts.\n/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.join();
    assert!(String::from_utf8_lossy(&output.stdout).contains("STRUCTURED-MEDIA-COMPLETE"));
}

#[test]
fn current_index_rebuild_counts_discarded_suffix_scan_and_writes() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let root = TemporaryDirectory::new("task-current-index-rebuild-cost");
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
        vec![ContentItem::assistant_text("x".repeat(20_000))],
    );
    let source = root
        .path()
        .join(".echidna/sessions")
        .join(&prior)
        .join("events.jsonl");
    let observed = Arc::new(AtomicUsize::new(0));
    let measured = Arc::clone(&observed);
    let server = paged_server(move |step, wire| match step {
        0 => (
            new_selection_response(wire, "rebuild-first-selection"),
            false,
        ),
        1 => {
            measured.store(
                fs::metadata(&source).unwrap().len() as usize,
                Ordering::SeqCst,
            );
            (text_response("", "rebuild-empty"), false)
        }
        2 => (
            new_selection_response(wire, "rebuild-second-selection"),
            false,
        ),
        3 => (
            text_response("INDEX-REBUILD-COMPLETE", "rebuild-second"),
            true,
        ),
        _ => panic!("unexpected index rebuild request"),
    });
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "First task.\n/resume\nSecond task.\n/exit\n",
        &["--resume", "latest", "--debug"],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.join();
    let diagnostics = String::from_utf8_lossy(&output.stderr);
    let work = diagnostics
        .lines()
        .filter(|line| line.contains("phase=task_index "))
        .collect::<Vec<_>>();
    assert_eq!(work.len(), 2, "{diagnostics}");
    let counter = |line: &str, name: &str| -> usize {
        line.split_whitespace()
            .find_map(|field| field.strip_prefix(&format!("{name}=")))
            .unwrap()
            .parse()
            .unwrap()
    };
    assert!(
        counter(work[1], "source_index_read_bytes")
            >= 2 * observed.load(Ordering::SeqCst) - counter(work[0], "source_index_read_bytes"),
        "{}",
        work[1]
    );
    assert!(
        counter(work[1], "source_index_text_write_bytes")
            >= counter(work[0], "source_index_text_write_bytes") + 2 * "Second task.".len(),
        "{}",
        work[1]
    );
    assert!(
        counter(work[1], "source_index_hash_write_bytes")
            >= counter(work[0], "source_index_hash_write_bytes") + 2 * 128,
        "{}",
        work[1]
    );
    assert!(
        counter(work[1], "source_index_record_write_bytes")
            >= counter(work[0], "source_index_record_write_bytes") + 2 * 128,
        "{}",
        work[1]
    );
}

#[test]
fn peer_index_rebuild_counts_discarded_suffix_scan_and_writes() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let root = TemporaryDirectory::new("task-peer-index-rebuild-cost");
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
        vec![ContentItem::assistant_text("PEERINDEX733 original")],
    );
    let prior_for_filter = prior.clone();
    let project_path = root.path().to_path_buf();
    let peer_path = project_path
        .join(".echidna/sessions")
        .join(&prior)
        .join("events.jsonl");
    let peer_path_for_bounds = peer_path.clone();
    let observed = Arc::new(AtomicUsize::new(0));
    let measured = Arc::clone(&observed);
    let server = paged_server(move |step, wire| {
        if step == 0 {
            return (
                new_selection_response(wire, "peer-rebuild-initial-selection"),
                false,
            );
        }
        match step - 1 {
            0 => (
                search_response("peer-before-rebuild", "PEERINDEX733", Value::Null),
                false,
            ),
            1 => {
                assert_eq!(
                    delivered_result(wire, "peer-before-rebuild")["status"],
                    "succeeded"
                );
                let project = Project::discover(&project_path).unwrap();
                append_source_items(
                    &project,
                    &prior,
                    (0..128)
                        .map(|index| {
                            ContentItem::assistant_text(format!(
                                "fixture suffix {index:03} {}",
                                "x".repeat(980)
                            ))
                        })
                        .collect(),
                );
                let old = reduced_session(&project, &prior)
                    .primary_provider_session()
                    .unwrap()
                    .clone();
                let session = StreamId::new(&prior);
                let stream = ProjectEventStream::Session(session.clone()).stream_id();
                let mut writer = project.open_writer().unwrap();
                let mut store = writer.open_session(session, None).unwrap();
                let resumed = SessionEvent::Resumed {
                    primary_provider_session: echidna::domain::context::ProviderSessionRef {
                        id: echidna::domain::context::ProviderSessionId::new(
                            "synthetic-peer-rebuild-primary",
                        ),
                        role: echidna::domain::context::ProviderSessionRole::Primary,
                        generation: old.generation + 1,
                    },
                }
                .into_envelope(
                    EventId::new("synthetic-peer-rebuild-resumed"),
                    stream,
                    store.next_sequence().unwrap(),
                    OccurredAt::new("2026-11-06T00:00:00Z"),
                )
                .unwrap();
                EventSink::append_event(&mut store, &resumed).unwrap();
                drop(store);
                drop(writer);
                assert_eq!(
                    reduced_session(&project, &prior)
                        .primary_provider_session()
                        .unwrap()
                        .id,
                    echidna::domain::context::ProviderSessionId::new(
                        "synthetic-peer-rebuild-primary"
                    )
                );
                measured.store(
                    fs::metadata(&peer_path).unwrap().len() as usize,
                    Ordering::SeqCst,
                );
                (
                    search_response("peer-after-rebuild", "PEERINDEX733", Value::Null),
                    false,
                )
            }
            2 => {
                assert_eq!(
                    delivered_result(wire, "peer-after-rebuild")["status"],
                    "succeeded"
                );
                (
                    text_response("PEER-REBUILD-COMPLETE", "peer-rebuild-done"),
                    true,
                )
            }
            _ => panic!("unexpected peer rebuild request"),
        }
    });
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "Search the synthetic peer.\n/exit\n",
        &["--agent", agent.to_str().unwrap(), "--debug"],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.join();
    let diagnostics = String::from_utf8_lossy(&output.stderr);
    let work = diagnostics
        .lines()
        .filter(|line| line.contains("phase=context_read "))
        .collect::<Vec<_>>();
    assert_eq!(work.len(), 2, "{diagnostics}");
    let counter = |line: &str, name: &str| -> usize {
        line.split_whitespace()
            .find_map(|field| field.strip_prefix(&format!("{name}=")))
            .unwrap()
            .parse()
            .unwrap()
    };
    // Each small peer suffix event has one raw hash and one text-prefix hash.
    // Allow one full Peer rebuild and twice all current History writes; the
    // discarded suffix must still supply a second copy of its own writes.
    let current_ids = session_ids(root.path())
        .into_iter()
        .filter(|id| id != &prior_for_filter)
        .collect::<Vec<_>>();
    assert_eq!(current_ids.len(), 1);
    let current_path = root
        .path()
        .join(".echidna/sessions")
        .join(&current_ids[0])
        .join("events.jsonl");
    let current = fs::read_to_string(current_path).unwrap();
    let current_history = current
        .lines()
        .filter(|line| {
            let value: Value = serde_json::from_str(line).unwrap();
            value["event_type"] == "session.history_item_recorded"
        })
        .collect::<Vec<_>>();
    let current_hash_bound: usize = current_history
        .iter()
        .map(|line| 64 * ((line.len() + 1).div_ceil(1024 * 1024) + 1))
        .sum();
    let current_text_bound: usize = current_history
        .iter()
        .map(|line| (line.len() + 1).min(1024))
        .sum();
    let suffix_hash = 128 * 128;
    let suffix_text = 128 * ("fixture suffix 000 ".len() + 980);
    let peer_original_text = "PEERINDEX733 original".len();
    assert!(2 * current_hash_bound + suffix_hash + 128 < 2 * suffix_hash);
    assert!(2 * current_text_bound + suffix_text + peer_original_text < 2 * suffix_text);
    // Lower-bound a frame with its mandatory serialized keys, source IDs,
    // revision strings and this fixture's exact semantics. Unknown numbers
    // and tag bytes take their shortest JSON representations. A frame's tag
    // adds at most two digits per byte; each of its eight numeric fields adds
    // at most 19 digits. Current-frame overhead is bounded separately by 2048
    // bytes, including keys, 32 three-digit tag bytes, two revisions, eight
    // 20-digit numbers and the longest source semantics used by this fixture.
    const MIN_PEER_FRAME_WITHOUT_TAG: &str = concat!(
        "{\"record\":{\"session\":\"\",\"ordinal\":0,",
        "\"reference\":\"session-revision:::\",\"event_id\":\"\",",
        "\"searchable\":true,\"peer\":true,\"raw_offset\":0,\"raw_len\":0,",
        "\"hashes_offset\":0,\"text_hashes_offset\":0,\"text_offset\":0,",
        "\"text_len\":0,\"text_stored_len\":0,\"revision\":\"\",",
        "\"role\":\"assistant\",\"origin\":\"model_output\",",
        "\"class\":\"model_observation\",\"authority\":\"model_observation\",",
        "\"observed\":\"2026-11-05T00:00:00Z\"},\"tag\":"
    );
    let peer_events = fs::read_to_string(peer_path_for_bounds).unwrap();
    let peer_history = peer_events
        .lines()
        .filter_map(|line| {
            let event: Value = serde_json::from_str(line).unwrap();
            (event["event_type"] == "session.history_item_recorded").then_some(event)
        })
        .collect::<Vec<_>>();
    assert_eq!(peer_history.len(), 129);
    assert!(
        prior_for_filter
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
    );
    for event in &peer_history {
        assert_eq!(event["occurred_at"], "2026-11-05T00:00:00Z");
        assert_eq!(event["payload"]["item"]["role"], "assistant");
        let id = event["event_id"].as_str().unwrap();
        assert!(
            id.strip_prefix("cold-source-")
                .is_some_and(|number| number.bytes().all(|byte| byte.is_ascii_digit()))
        );
    }
    let minimum_frame = |event: &Value| {
        MIN_PEER_FRAME_WITHOUT_TAG.len()
            + 2 * prior_for_filter.len()
            + 2 * event["event_id"].as_str().unwrap().len()
            + 2 * 64 // two hexadecimal revisions
            + 65 // array brackets, 32 one-digit tag values and 31 commas
            + 2 // closing brace and newline
    };
    let suffix_frame_minimum: usize = peer_history[1..].iter().map(minimum_frame).sum();
    let peer_frame_upper: usize = peer_history
        .iter()
        .map(|event| minimum_frame(event) + 64 + 8 * 19)
        .sum();
    let current_frame_upper: usize = current_history
        .iter()
        .map(|line| 2 * line.len() + current_ids[0].len() + 2048)
        .sum();
    assert!(peer_frame_upper + 2 * current_frame_upper < 2 * suffix_frame_minimum);
    let checks = [
        (
            "source_index_scan_bytes",
            2 * observed.load(Ordering::SeqCst),
        ),
        ("source_index_hash_write_bytes", 2 * suffix_hash),
        ("source_index_text_write_bytes", 2 * suffix_text),
        ("source_index_record_write_bytes", 2 * suffix_frame_minimum),
    ]
    .map(|(name, minimum)| {
        let actual = counter(work[1], name);
        (name, actual, minimum, actual >= minimum)
    });
    assert!(checks.iter().all(|check| check.3), "{checks:?}");
}

#[test]
fn unicode_lowercase_source_spans_cross_window_and_segment_boundaries() {
    let root = TemporaryDirectory::new("task-unicode-source-span");
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
    let first = format!("CVE-{}", "K".repeat(2000));
    let second = format!("CWE-{}", "K".repeat(2000));
    let query_first = format!("cve-{}", "k".repeat(2000));
    let query_second = format!("cwe-{}", "k".repeat(2000));
    let first_offset = 14_000;
    let second_offset = 1024 * 1024 - 2000;
    let source = format!(
        "{}{}{}{} end",
        " ".repeat(first_offset),
        first,
        " ".repeat(second_offset - first_offset - first.len()),
        second
    );
    assert_eq!(source.find(&first), Some(first_offset));
    assert_eq!(source.find(&second), Some(second_offset));
    let item = ContentItem::assistant_text(&source);
    let ids = append_source_items(&project, &prior, vec![item.clone()]);
    let session_reference = peer_reference(&prior, &ids[0], &item);
    let memory_root = root.path().join(".echidna/memory");
    fs::create_dir_all(&memory_root).unwrap();
    fs::write(memory_root.join("core.md"), &source).unwrap();
    let memory_reference = format!(
        "memory:core.md:{}",
        blake3::hash(source.as_bytes()).to_hex()
    );
    let digest = format!("{:x}", Sha256::digest(source.as_bytes()));
    let blobs = root.path().join(".echidna/artifacts/blobs/sha256");
    fs::create_dir_all(&blobs).unwrap();
    fs::write(blobs.join(&digest), &source).unwrap();
    let artifact_reference = format!("artifact:unicode-window:{digest}");
    append_research_event(
        &project,
        "01",
        ResearchEvent::Artifact(ArtifactEvent::Created(ArtifactRecord {
            id: ArtifactId::new("unicode-window"),
            blob_id: BlobId::new(&digest).unwrap(),
            media_type: "text/plain".to_owned(),
            size_bytes: source.len() as u64,
            source_path: "unicode-window.txt".to_owned(),
            created_by_event_id: EventId::new("research-01"),
            produced_by_tool_call_id: ToolCallId::new("unicode-window-call"),
            scope: ArtifactScope::Session {
                session_id: StreamId::new(&prior),
            },
            toolchain_fingerprint: "fixture".to_owned(),
            parent_artifact_ids: Vec::new(),
            transform: None,
        })),
    );
    let expected = BTreeSet::from([session_reference, memory_reference, artifact_reference]);
    let mut seen = BTreeSet::new();
    let mut phase = 0;
    let mut call = String::new();
    let server = paged_server_with_request_waits(
        Duration::from_secs(60),
        Duration::from_secs(60),
        move |step, wire| {
            if step < 2 {
                return (
                    new_selection_response(wire, &format!("unicode-span-selection-{step}")),
                    false,
                );
            }
            let mut cursor = Value::Null;
            if step > 2 {
                let result = delivered_result(wire, &call);
                assert_eq!(result["status"], "succeeded", "{result}");
                let response = &result["output"];
                for entry in response["entries"].as_array().unwrap() {
                    let reference = entry["reference"].as_str().unwrap();
                    if !expected.contains(reference) {
                        continue;
                    }
                    let offset = entry["offset"].as_u64().unwrap() as usize;
                    let text = entry["text"].as_str().unwrap();
                    let (needle, expected_offset) = if phase == 0 {
                        (&first, first_offset)
                    } else {
                        (&second, second_offset)
                    };
                    assert!(text.contains(needle), "{entry}");
                    assert_eq!(text, &source[offset..offset + text.len()]);
                    assert_eq!(offset + text.find(needle).unwrap(), expected_offset);
                    assert!(seen.insert(reference.to_owned()), "duplicate: {entry}");
                }
                cursor = response["next_cursor"].clone();
                if cursor.is_null() {
                    assert_eq!(
                        seen, expected,
                        "missing source at phase {phase}: {response}"
                    );
                    if phase == 1 {
                        return (
                            text_response("UNICODE-SPAN-COMPLETE", "unicode-span-done"),
                            true,
                        );
                    }
                    phase = 1;
                    seen.clear();
                }
            }
            call = format!("unicode-span-{step}");
            let query = if phase == 0 {
                &query_first
            } else {
                &query_second
            };
            (
                m4_long_flow_tool_response(
                    &call,
                    "context_read",
                    json!({
                        "operation":"search", "query":query, "cursor":cursor, "max_bytes":6004
                    }),
                    &format!("unicode-span-response-{step}"),
                ),
                false,
            )
        },
    );
    // This boundary fixture deliberately loads 1 MiB of Core, not a model-window claim.
    let config = write_context_window_config(root.path(), &server.base_url, 8_000_000);
    fs::write(
        &config,
        format!(
            "{}\n[runtime.context_budget]\nmax_core_file_bytes = 2097152\n",
            fs::read_to_string(&config).unwrap()
        ),
    )
    .unwrap();
    let output = run_chat(
        root.path(),
        &config,
        "Search the synthetic Unicode archive.\n/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.join();
    assert!(String::from_utf8_lossy(&output.stdout).contains("UNICODE-SPAN-COMPLETE"));
}

#[test]
fn cold_search_rejects_changed_early_artifact_metadata_before_late_match() {
    let root = TemporaryDirectory::new("task-cold-artifact-metadata-change");
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
    let project = Project::discover(root.path()).unwrap();
    let prior = session_id(root.path());
    seed_large_artifact_metadata(&project, root.path(), &prior, "application/octet-stream");
    let research_path = root.path().join(".echidna/research.jsonl");
    let original = fs::read(&research_path).unwrap();
    let path = research_path.clone();
    let original_for_server = original.clone();
    let mut call = String::new();
    let server = paged_server_with_request_waits(
        Duration::from_secs(60),
        Duration::from_secs(60),
        move |step, wire| {
            if step == 0 {
                return (
                    new_selection_response(wire, "metadata-change-initial-selection"),
                    false,
                );
            }
            let step = step - 1;
            if step == 0 {
                call = "metadata-change-first".to_owned();
                return (
                    search_response(&call, ARTIFACT_METADATA_MARKER, Value::Null),
                    false,
                );
            }
            let result = delivered_result(wire, &call);
            if step == 1 {
                assert_eq!(result["status"], "succeeded", "{result}");
                assert!(
                    result["output"]["entries"].as_array().unwrap().is_empty(),
                    "{result}"
                );
                let cursor = result["output"]["next_cursor"].clone();
                assert!(cursor.is_string(), "{result}");
                let mut changed = original_for_server.clone();
                let position = changed
                    .windows(b"fixture-a-large-metadata".len())
                    .position(|part| part == b"fixture-a-large-metadata")
                    .unwrap();
                changed[position] = b'F';
                fs::write(&path, changed).unwrap();
                return (
                    search_response("metadata-change-next", ARTIFACT_METADATA_MARKER, cursor),
                    true,
                );
            }
            panic!("unexpected metadata mutation request {step}")
        },
    );
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "Search the archived artifacts.\n/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    server.join();
    fs::write(&research_path, &original).unwrap();
    assert!(
        !output.status.success(),
        "changed Research prefix must be fatal"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Execution Receipt commit failed"),
        "{stderr}"
    );
    assert!(
        stderr.contains("task context source prefix changed"),
        "{stderr}"
    );
    assert_eq!(fs::read(research_path).unwrap(), original);
    let current = session_ids(root.path())
        .into_iter()
        .find(|id| id != &prior)
        .unwrap();
    let state = reduced_session(&project, &current);
    assert_eq!(state.execution_receipt_count(), 1);
    assert!(!state.history().items().iter().any(|item| matches!(
        item,
        ContentItem::ToolResult { result } if result.call_id.as_str() == "metadata-change-next"
    )));
}

#[test]
fn cold_search_pages_all_memory_sources_beyond_the_shadow_limit() {
    let root = TemporaryDirectory::new("task-cold-many-memory");
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
    let expected = (0..75)
        .map(|index| {
            let text = format!(
                "AUXILIARYNOTE917 exact source {index:02}. {}{}",
                "x".repeat(16),
                if index == 74 {
                    " LATEONLY738MEMORY"
                } else {
                    ""
                }
            );
            let name = format!("note-{index:02}.md");
            fs::write(notes.join(&name), &text).unwrap();
            (
                format!(
                    "memory:notes/{name}:{}",
                    blake3::hash(text.as_bytes()).to_hex()
                ),
                text,
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut seen = BTreeSet::new();
    let mut call = String::new();
    let mut pages = 0;
    let mut unique_checked = false;
    let server = paged_server(move |step, wire| {
        if step == 0 {
            return (
                initial_selection_response(wire, "memory-initial-selection"),
                false,
            );
        }
        if step == 1 {
            call = "memory-unique".to_owned();
            return (
                search_response(&call, "LATEONLY738MEMORY", Value::Null),
                false,
            );
        }
        let result = delivered_result(wire, &call);
        assert_eq!(result["status"], "succeeded", "{result}");
        let response = &result["output"];
        if !unique_checked {
            let entries = response["entries"].as_array().unwrap();
            assert_eq!(
                entries.len(),
                1,
                "late unique source was omitted: {response}"
            );
            let reference = entries[0]["reference"].as_str().unwrap();
            assert!(
                reference.starts_with("memory:notes/note-74.md:"),
                "{response}"
            );
            assert_eq!(entries[0]["scope"], "memory");
            assert_eq!(
                entries[0]["source_revision"],
                blake3::hash(expected[reference].as_bytes())
                    .to_hex()
                    .to_string()
            );
            assert!(response["next_cursor"].is_null(), "{response}");
            unique_checked = true;
            call = format!("memory-page-{step}");
            return (
                search_response(&call, "AUXILIARYNOTE917", Value::Null),
                false,
            );
        }
        pages += 1;
        for entry in response["entries"].as_array().unwrap() {
            let reference = entry["reference"].as_str().unwrap();
            let original = &expected[reference];
            let offset = entry["offset"].as_u64().unwrap() as usize;
            let text = entry["text"].as_str().unwrap();
            assert_eq!(text, &original[offset..offset + text.len()]);
            assert_eq!(entry["total_bytes"], original.len());
            assert!(
                seen.insert(reference.to_owned()),
                "duplicate source {entry}"
            );
        }
        let cursor = response["next_cursor"].clone();
        if !cursor.is_null() {
            call = format!("memory-page-{step}");
            return (search_response(&call, "AUXILIARYNOTE917", cursor), false);
        }
        assert_eq!(seen, expected.keys().cloned().collect());
        assert!(pages >= 2);
        (
            text_response("ALL-AUXILIARY-SOURCES-FOUND", "memory-pages-done"),
            true,
        )
    });
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "Search the archived notes.\n/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.join();
    assert!(String::from_utf8_lossy(&output.stdout).contains("ALL-AUXILIARY-SOURCES-FOUND"));
}

#[test]
fn cold_search_prepares_memory_metadata_without_copying_document_bodies() {
    let root = TemporaryDirectory::new("task-cold-memory-preparation-cost");
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
    for index in 0..40 {
        let suffix = if index == 39 {
            " MEMORYTARGET741NOVA"
        } else {
            ""
        };
        fs::write(
            notes.join(format!("note-{index:02}.md")),
            format!("{}{suffix}", "z".repeat(60_000)),
        )
        .unwrap();
    }
    let mut call = String::new();
    let server = paged_server(move |step, wire| {
        if step == 0 {
            let candidates = selector_candidates(wire);
            let context = candidates["context"].as_array().unwrap();
            assert!(
                context
                    .iter()
                    .any(|unit| unit["ref"].as_str().unwrap().starts_with("memory:notes/"))
            );
            assert!(
                !context
                    .iter()
                    .any(|unit| unit["ref"].as_str().unwrap().starts_with("memory:core.md:"))
            );
            return (new_selection_response(wire, "memory-cost-selection"), false);
        }
        let cursor = if step == 1 {
            Value::Null
        } else {
            let result = delivered_result(wire, &call);
            assert_eq!(result["status"], "succeeded", "{result}");
            let response = &result["output"];
            if response["entries"].as_array().unwrap().iter().any(|entry| {
                entry["text"]
                    .as_str()
                    .unwrap()
                    .contains("MEMORYTARGET741NOVA")
            }) {
                return (
                    text_response("MEMORY-COST-COMPLETE", "memory-cost-done"),
                    true,
                );
            }
            assert!(response["next_cursor"].is_string(), "{response}");
            response["next_cursor"].clone()
        };
        call = format!("memory-cost-{step}");
        (search_response(&call, "MEMORYTARGET741NOVA", cursor), false)
    });
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "Search the synthetic notes.\n/exit\n",
        &["--agent", agent.to_str().unwrap(), "--debug"],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.join();
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
    assert!(pages.iter().any(
        |line| counter(line, "auxiliary_preparation_memory_read_bytes") >= 2_400_000
            && counter(line, "auxiliary_preparation_write_bytes") < 100_000
    ));
    assert!(
        pages
            .iter()
            .any(|line| counter(line, "source_verification_bytes") >= 60_000)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("MEMORY-COST-COMPLETE"));
}

#[test]
fn cold_search_pages_large_escaped_core_after_nonempty_memory_pages() {
    let root = TemporaryDirectory::new("task-cold-large-core");
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
    let marker = "😀跨界核心729";
    let mut expected = BTreeMap::new();
    for index in 0..12 {
        let text = format!("{marker} note {index:02}");
        let name = format!("small-{index:02}.md");
        fs::write(notes.join(&name), &text).unwrap();
        expected.insert(
            format!(
                "memory:notes/{name}:{}",
                blake3::hash(text.as_bytes()).to_hex()
            ),
            text,
        );
    }
    // The final emoji begins two bytes before a 1 MiB source boundary. Quotes
    // also make the original JSON-encoded body larger than the page budget.
    let core = format!(
        "{}{marker} exact core ending",
        "\"".repeat(9 * 1024 * 1024 - 2)
    );
    let marker_offset = core.find(marker).unwrap();
    assert_eq!(marker_offset, 9 * 1024 * 1024 - 2);
    assert!(!core.is_char_boundary(9 * 1024 * 1024));
    let core_reference = format!("memory:core.md:{}", blake3::hash(core.as_bytes()).to_hex());
    fs::write(root.path().join(".echidna/memory/core.md"), &core).unwrap();
    let mut call = String::new();
    let mut seen = BTreeSet::new();
    let mut nonempty_before_core = false;
    let mut empty_pending = false;
    let mut pages = 0;
    // The prior run exceeded the 15s first-request deadline and failed in
    // 22.60s total. After fixing the declared capacity, preparing the first
    // 18.9 MB main request also exceeded 15s (42.03s failed-run total). This
    // isolated huge-source allowance tests paging, not interactive latency.
    let server = paged_server_with_request_waits(
        Duration::from_secs(60),
        Duration::from_secs(60),
        move |step, wire| {
            if step < 2 {
                return (
                    new_selection_response(wire, &format!("large-core-selection-{step}")),
                    false,
                );
            }
            let mut cursor = Value::Null;
            if step > 2 {
                let result = delivered_result(wire, &call);
                assert_eq!(result["status"], "succeeded", "{result}");
                let response = &result["output"];
                pages += 1;
                let entries = response["entries"].as_array().unwrap();
                for entry in entries {
                    let reference = entry["reference"].as_str().unwrap();
                    let text = entry["text"].as_str().unwrap();
                    let offset = entry["offset"].as_u64().unwrap() as usize;
                    assert_eq!(entry["scope"], "memory");
                    assert!(
                        seen.insert(reference.to_owned()),
                        "duplicate Memory source: {entry}"
                    );
                    if reference == core_reference {
                        assert!(
                            nonempty_before_core,
                            "Core must follow a nonempty auxiliary page"
                        );
                        assert!(text.contains(marker), "{entry}");
                        assert_eq!(text, &core[offset..offset + text.len()]);
                        assert_eq!(entry["total_bytes"], core.len());
                        assert_eq!(
                            entry["source_revision"],
                            blake3::hash(core.as_bytes()).to_hex().to_string()
                        );
                    } else {
                        let original = &expected[reference];
                        assert_eq!(text, &original[offset..offset + text.len()]);
                        nonempty_before_core = true;
                    }
                }
                cursor = response["next_cursor"].clone();
                if cursor.is_null() {
                    assert_eq!(seen.len(), expected.len() + 1);
                    assert!(seen.contains(&core_reference));
                    assert!(
                        pages >= 3 && empty_pending,
                        "oversized source must produce an empty incomplete page"
                    );
                    return (
                        text_response("LARGE-CORE-PAGES-COMPLETE", "large-core-done"),
                        true,
                    );
                }
                assert_eq!(response["truncated"], true);
                empty_pending |= entries.is_empty();
            }
            call = format!("large-core-page-{step}");
            (search_response(&call, marker, cursor), false)
        },
    );
    // This oversized-source mechanism fixture needs 4.72M estimated input tokens
    // after quote encoding. Its synthetic capacity is not a real-model claim.
    let config = write_context_window_config(root.path(), &server.base_url, 8_000_000);
    let mut config_text = fs::read_to_string(&config).unwrap();
    config_text.push_str("\n[runtime.context_budget]\nmax_core_file_bytes = 12582912\n");
    fs::write(&config, config_text).unwrap();
    let output = run_chat(
        root.path(),
        &config,
        "Search the synthetic Memory archive.\n/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.join();
    assert!(String::from_utf8_lossy(&output.stdout).contains("LARGE-CORE-PAGES-COMPLETE"));
}

#[test]
fn cold_search_rejects_changed_core_before_returning_cached_late_hit() {
    let root = TemporaryDirectory::new("task-cold-core-source-change");
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
    let core_path = root.path().join(".echidna/memory/core.md");
    fs::create_dir_all(core_path.parent().unwrap()).unwrap();
    let core = format!(
        "CORE-EARLY-ORIGINAL {} CORE-LATE-CHANGED-CHECK728",
        "x".repeat(6 * 1024 * 1024)
    );
    fs::write(&core_path, &core).unwrap();
    let modified = fs::metadata(&core_path).unwrap().modified().unwrap();
    let source = core_path.clone();
    let original = core.clone();
    let mut call = String::new();
    let mut changed = false;
    // Both selection and the first main request prepare the synthetic 6 MiB
    // Core. Keep the normal paging helper's deadline unchanged for other tests.
    let server = paged_server_with_request_waits(
        Duration::from_secs(60),
        Duration::from_secs(60),
        move |step, wire| {
            if step < 2 {
                return (
                    new_selection_response(wire, &format!("core-change-selection-{step}")),
                    false,
                );
            }
            let mut cursor = Value::Null;
            if step > 2 {
                let result = delivered_result(wire, &call);
                if result["status"] == "failed" {
                    assert!(
                        changed,
                        "the source must first be scanned before modification"
                    );
                    fs::write(&source, &original).unwrap();
                    fs::File::open(&source)
                        .unwrap()
                        .set_times(fs::FileTimes::new().set_modified(modified))
                        .unwrap();
                    return (
                        text_response("CHANGED-CORE-REJECTED", "core-change-done"),
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
                    "late Core source cannot finish before validation: {response}"
                );
                if !changed {
                    let altered =
                        original.replacen("CORE-EARLY-ORIGINAL", "CORE-EARLY-MODIFIED", 1);
                    assert_eq!(altered.len(), original.len());
                    fs::write(&source, altered).unwrap();
                    fs::File::open(&source)
                        .unwrap()
                        .set_times(fs::FileTimes::new().set_modified(modified))
                        .unwrap();
                    changed = true;
                }
            }
            call = format!("core-change-page-{step}");
            (
                search_response(&call, "CORE-LATE-CHANGED-CHECK728", cursor),
                false,
            )
        },
    );
    let config = write_context_window_config(root.path(), &server.base_url, 4_000_000);
    let mut config_text = fs::read_to_string(&config).unwrap();
    config_text.push_str("\n[runtime.context_budget]\nmax_core_file_bytes = 8388608\n");
    fs::write(&config, config_text).unwrap();
    let output = run_chat(
        root.path(),
        &config,
        "Search the synthetic Core archive.\n/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.join();
    assert!(String::from_utf8_lossy(&output.stdout).contains("CHANGED-CORE-REJECTED"));
    assert_eq!(fs::read_to_string(core_path).unwrap(), core);
}

#[test]
fn cold_search_reaches_late_large_artifact_body_across_pages() {
    let root = TemporaryDirectory::new("task-cold-large-artifact");
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
    let marker = "LARGEBODYNEEDLE739 exact observed value 42.75.";
    let offset = 9 * 1024 * 1024 + 13;
    let source = format!("{}{marker}", " ".repeat(offset));
    fs::write(root.path().join("large-source.txt"), &source).unwrap();
    let total = source.len();
    let mut call = String::new();
    let mut pages = 0;
    let server = paged_server(move |step, wire| {
        if step == 0 {
            return (
                new_selection_response(wire, "large-artifact-initial-selection"),
                false,
            );
        }
        let step = step - 1;
        if step == 0 {
            return (
                m4_long_flow_tool_response(
                    "publish-large",
                    "artifact_store",
                    json!({
                        "event_id":"large-source-event", "occurred_at":"2026-11-05T00:00:00Z",
                        "artifact_id":"large-source", "source_path":"large-source.txt",
                        "media_type":"text/plain", "toolchain_fingerprint":"fixture-v1"
                    }),
                    "large-source-published",
                ),
                false,
            );
        }
        if step == 1 {
            assert_eq!(
                delivered_result(wire, "publish-large")["status"],
                "succeeded"
            );
            call = "artifact-page-1".to_owned();
            return (
                search_response(&call, "LARGEBODYNEEDLE739", Value::Null),
                false,
            );
        }
        let result = delivered_result(wire, &call);
        assert_eq!(result["status"], "succeeded", "{result}");
        let response = &result["output"];
        if call == "exact-artifact" {
            assert_eq!(response["status"], "found", "{response}");
            assert_eq!(response["entries"][0]["text"], marker);
            assert_eq!(response["entries"][0]["offset"], offset);
            assert_eq!(response["entries"][0]["total_bytes"], total);
            return (
                text_response("LARGE-ARTIFACT-FOUND", "large-artifact-final"),
                true,
            );
        }
        pages += 1;
        if let Some(entry) = response["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| {
                entry["reference"]
                    .as_str()
                    .unwrap()
                    .starts_with("artifact:large-source:")
            })
        {
            assert!(
                pages >= 3,
                "oversized artifact must be searched incrementally"
            );
            assert!(entry["text"].as_str().unwrap().contains(marker));
            call = "exact-artifact".to_owned();
            return (
                m4_long_flow_tool_response(
                    &call,
                    "context_read",
                    json!({
                        "operation":"read", "reference":entry["reference"], "offset":offset, "max_bytes":1024
                    }),
                    "exact-artifact-read",
                ),
                false,
            );
        }
        let cursor = response["next_cursor"].clone();
        assert!(cursor.is_string(), "late source skipped: {response}");
        call = format!("artifact-page-{step}");
        (search_response(&call, "LARGEBODYNEEDLE739", cursor), false)
    });
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "Publish and search the archived artifact.\n/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.join();
    assert!(String::from_utf8_lossy(&output.stdout).contains("LARGE-ARTIFACT-FOUND"));
}

#[test]
fn cold_search_discovers_peer_added_after_index_initialization() {
    let root = TemporaryDirectory::new("task-cold-new-peer");
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
    let text = "NEWPEER428 original source value 73.25.";
    let ids = append_source_items(&project, &prior, vec![ContentItem::assistant_text(text)]);
    let expected_ref = peer_reference(&prior, &ids[0], &ContentItem::assistant_text(text));
    let destination = root.path().join(".echidna/sessions").join(&prior);
    let hidden = root.path().join("withheld-synthetic-peer");
    // Restore a complete, genuine synthetic Session after the first lookup.
    // No event is rewritten and no old Tool is executed.
    fs::rename(&destination, &hidden).unwrap();
    let source_before = fs::read(hidden.join("events.jsonl")).unwrap();
    let server =
        paged_server(move |step, wire| {
            if step == 0 {
                return (
                    new_selection_response(wire, "new-peer-initial-selection"),
                    false,
                );
            }
            match step - 1 {
                0 => (
                    search_response("before-peer", "NEWPEER428", Value::Null),
                    false,
                ),
                1 => {
                    let result = delivered_result(wire, "before-peer");
                    assert_eq!(result["status"], "succeeded", "{result}");
                    assert_eq!(result["output"]["status"], "not_found", "{result}");
                    assert!(result["output"]["next_cursor"].is_null());
                    fs::rename(&hidden, &destination).unwrap();
                    (
                        search_response("after-peer", "NEWPEER428", Value::Null),
                        false,
                    )
                }
                2 => {
                    let result = delivered_result(wire, "after-peer");
                    assert_eq!(result["status"], "succeeded", "{result}");
                    assert!(
                        result["output"]["entries"].as_array().unwrap().iter().any(
                            |entry| entry["reference"] == expected_ref && entry["text"] == text
                        ),
                        "{result}"
                    );
                    assert_eq!(
                        fs::read(destination.join("events.jsonl")).unwrap(),
                        source_before
                    );
                    (text_response("NEW-PEER-DISCOVERED", "peer-done"), true)
                }
                _ => panic!("unexpected peer request"),
            }
        });
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "Search the archived peer material.\n/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.join();
}

#[test]
fn cold_cursor_rejects_definition_reload_and_other_project() {
    let root = TemporaryDirectory::new("task-cold-cursor-binding");
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
    let project = Project::discover(root.path()).unwrap();
    append_source_items(
        &project,
        &session_id(root.path()),
        (0..300)
            .map(|_| ContentItem::assistant_text("unrelated archive entry"))
            .collect(),
    );
    let retained = Arc::new(std::sync::Mutex::new(Value::Null));
    let captured = Arc::clone(&retained);
    let persona = agent.join("persona.md");
    let mut cursors = Vec::new();
    let server = paged_server(move |step, wire| {
        if step == 0 {
            return (
                new_selection_response(wire, "cursor-scope-initial-selection"),
                false,
            );
        }
        match step - 1 {
            0 => {
                let calls = [
                    "original-scope-0",
                    "original-scope-1",
                    "original-scope-2",
                    "original-scope-3",
                ]
                .map(|id| {
                    (
                        id,
                        "context_read",
                        json!({"operation":"search", "query":id, "max_bytes":1024}),
                    )
                });
                (tool_calls_response(&calls, "original-searches"), false)
            }
            1 => {
                for id in [
                    "original-scope-0",
                    "original-scope-1",
                    "original-scope-2",
                    "original-scope-3",
                ] {
                    let result = delivered_result(wire, id);
                    assert_eq!(result["status"], "succeeded", "{result}");
                    let cursor = result["output"]["next_cursor"].clone();
                    assert!(cursor.is_string(), "{result}");
                    cursors.push(cursor);
                }
                *captured.lock().unwrap() = cursors[0].clone();
                let calls = [
                    (
                        "same-scope-mismatch",
                        "context_read",
                        json!({"operation":"search", "query":"wrong-query", "cursor":cursors[0], "max_bytes":1024}),
                    ),
                    (
                        "still-full",
                        "context_read",
                        json!({"operation":"search", "query":"another-search", "max_bytes":1024}),
                    ),
                ];
                (
                    tool_calls_response(&calls, "scope-negative-controls"),
                    false,
                )
            }
            2 => {
                let mismatch = delivered_result(wire, "same-scope-mismatch");
                assert_eq!(mismatch["status"], "failed", "{mismatch}");
                let full = delivered_result(wire, "still-full");
                assert_eq!(full["status"], "failed", "{full}");
                assert!(
                    full.to_string()
                        .contains("too many unfinished context searches"),
                    "{full}"
                );
                fs::write(&persona, "A revised synthetic assistant definition.\n").unwrap();
                (
                    text_response("FIRST-SCOPE-COMPLETE", "scope-first-done"),
                    false,
                )
            }
            3 => (new_selection_response(wire, "scope-new-selection"), false),
            4 => {
                let calls = [
                    (
                        "changed-scope",
                        "context_read",
                        json!({"operation":"search", "query":"original-scope-0", "cursor":cursors[0], "max_bytes":1024}),
                    ),
                    (
                        "new-scope",
                        "context_read",
                        json!({"operation":"search", "query":"new-scope-search", "max_bytes":1024}),
                    ),
                ];
                (tool_calls_response(&calls, "scope-reload-searches"), false)
            }
            5 => {
                let result = delivered_result(wire, "changed-scope");
                assert!(
                    result["status"] == "failed" || result["output"]["status"] == "stale",
                    "{result}"
                );
                let fresh = delivered_result(wire, "new-scope");
                assert_eq!(fresh["status"], "succeeded", "{fresh}");
                (text_response("CHANGED-SCOPE-REJECTED", "scope-done"), true)
            }
            _ => panic!("unexpected scope request"),
        }
    });
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "Search the archive.\n/agent reload\nTry the previous search cursor.\n/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.join();
    let other = TemporaryDirectory::new("task-cold-cursor-other-project");
    initialize_project(other.path());
    let other_agent = definition(other.path(), None);
    let cursor = retained.lock().unwrap().clone();
    let server = paged_server(move |step, wire| {
        if step == 0 {
            return (
                new_selection_response(wire, "foreign-cursor-initial-selection"),
                false,
            );
        }
        match step - 1 {
            0 => (
                search_response("foreign-cursor", "original-scope-0", cursor.clone()),
                false,
            ),
            1 => {
                let result = delivered_result(wire, "foreign-cursor");
                assert!(
                    result["status"] == "failed" || result["output"]["status"] == "stale",
                    "{result}"
                );
                (
                    text_response("FOREIGN-CURSOR-REJECTED", "foreign-done"),
                    true,
                )
            }
            _ => panic!("unexpected foreign request"),
        }
    });
    let config = write_context_window_config(other.path(), &server.base_url, 272_000);
    let output = run_chat(
        other.path(),
        &config,
        "Search using the supplied cursor.\n/exit\n",
        &["--agent", other_agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.join();
}
