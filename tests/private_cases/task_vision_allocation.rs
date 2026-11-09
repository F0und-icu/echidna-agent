//! Allocation evidence for the real view_image follow-up source owner.
use sha2::{Digest, Sha256};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::collections::BTreeMap;

use crate::domain::context::{AuthorityLevel, ContextOrigin, ContextProvenance};
use crate::domain::events::{EventEnvelope, EventId, EventSequence, OccurredAt};
use crate::domain::model::ContentItem;
use crate::domain::session::{SessionEvent, SessionReducer, TaskInputTime, TimeZoneObservation};

struct CountingAllocator;

thread_local! {
    static ENABLED: Cell<bool> = const { Cell::new(false) };
    static ALLOCATED: Cell<usize> = const { Cell::new(0) };
}

fn count(size: usize) {
    let _ = ENABLED.try_with(|enabled| {
        if enabled.get() {
            let _ = ALLOCATED.try_with(|total| total.set(total.get() + size));
        }
    });
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            count(layout.size());
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let result = unsafe { System.realloc(pointer, layout, size) };
        if !result.is_null() {
            count(size);
        }
        result
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

#[test]
fn vision_followup_batch_allocation_does_not_grow_with_accepted_history() {
    let fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/task_context_legacy");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(fixture.join("manifest.json")).unwrap()).unwrap();
    let files = manifest["cases"]["secondary"]["files"].as_object().unwrap();
    let hash = files
        .iter()
        .find(|(name, _)| name.ends_with("/events.jsonl"))
        .unwrap()
        .1
        .as_str()
        .unwrap();
    let bytes = std::fs::read(fixture.join("blobs").join(hash)).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&bytes)), hash);
    let events = std::str::from_utf8(&bytes)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<EventEnvelope<serde_json::Value>>(line).unwrap())
        .collect::<Vec<_>>();
    for kind in [
        "tool.execution_outcome_recorded",
        "vision.route_decided",
        "vision.observation_recorded",
    ] {
        assert!(events.iter().any(|event| event.event_type.as_str() == kind));
    }
    let followup = events
        .iter()
        .find(|event| {
            event.event_type.as_str() == "session.history_item_recorded"
                && event.payload["item"]["text"]
                    .as_str()
                    .is_some_and(|text| text.contains("fixture screenshot"))
        })
        .unwrap()
        .event_id
        .clone();
    let stream = events[0].stream_id.clone();
    let mut sequence = events.last().unwrap().sequence.get();
    let mut state = SessionReducer.reduce(events).unwrap();
    let mut append = |state: &mut crate::domain::session::SessionState, payload: SessionEvent| {
        sequence += 1;
        let event = payload
            .into_envelope(
                EventId::new(format!("vision-allocation-{sequence}")),
                stream.clone(),
                EventSequence::new(sequence),
                OccurredAt::new("2026-11-07T00:00:00Z"),
            )
            .unwrap();
        let id = event.event_id.clone();
        SessionReducer.apply_in_place(state, event).unwrap();
        id
    };
    let legacy = append(
        &mut state,
        SessionEvent::HistoryItemRecorded {
            item: ContentItem::user_text("an unrelated legacy input"),
        },
    );
    let small = state.clone();
    for index in 0..2048 {
        let text = format!("accepted synthetic operator input {index}");
        let item = ContentItem::user_text(&text);
        append(
            &mut state,
            SessionEvent::HistoryItemRecorded { item: item.clone() },
        );
        let provenance = BTreeMap::from([(0, ContextProvenance::operator_input(&text).unwrap())]);
        let boundary = state
            .accepted_operator_input(
                &[item],
                &provenance,
                TaskInputTime {
                    unix_seconds: 1_793_894_401,
                    nanoseconds: 0,
                    timezone: TimeZoneObservation::Unknown,
                },
            )
            .unwrap();
        append(
            &mut state,
            SessionEvent::OperatorTurnBoundaryRecorded { boundary },
        );
    }
    let measure = |state: &crate::domain::session::SessionState| {
        let items = [&followup, &legacy]
            .into_iter()
            .map(|id| {
                state
                    .history_items_with_event_ids()
                    .find(|(event, _)| *event == id)
                    .unwrap()
            })
            .collect::<Vec<_>>();
        // Warm up any runtime initialization outside the measured call.
        state.history_source_semantics_many(&items).unwrap();
        ALLOCATED.with(|total| total.set(0));
        ENABLED.with(|enabled| enabled.set(true));
        let semantics = state.history_source_semantics_many(&items).unwrap();
        ENABLED.with(|enabled| enabled.set(false));
        let bytes = ALLOCATED.with(Cell::get);
        (semantics, bytes)
    };
    let (expected, small_bytes) = measure(&small);
    let (actual, large_bytes) = measure(&state);
    assert_eq!(actual, expected);
    assert_eq!(actual[0].origin, ContextOrigin::ModelOutput);
    assert_eq!(actual[0].authority, AuthorityLevel::ModelObservation);
    assert_eq!(actual[1].origin, ContextOrigin::SessionProjection);
    assert_eq!(actual[1].authority, AuthorityLevel::UnclassifiedLegacy);
    assert!(
        large_bytes <= small_bytes + 4096,
        "bounded batch allocated {small_bytes} bytes before and {large_bytes} after 2048 accepted inputs"
    );
}

#[test]
fn real_same_turn_secondary_followups_skip_unrelated_large_observations() {
    use crate::adapters::project_event_store::Project;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    fn server(
        count: usize,
        mut reply: impl FnMut(usize, &serde_json::Value) -> String + Send + 'static,
    ) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let handle = std::thread::spawn(move || {
            for index in 0..count {
                let deadline = Instant::now() + Duration::from_secs(60);
                let mut socket = loop {
                    match listener.accept() {
                        Ok((socket, _)) => break socket,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(
                                Instant::now() < deadline,
                                "Vision fixture request timed out"
                            );
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => panic!("Vision fixture listener: {error}"),
                    }
                };
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(10)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0; 4096];
                let (header_end, length) = loop {
                    let read = socket.read(&mut buffer).unwrap();
                    assert_ne!(read, 0);
                    bytes.extend_from_slice(&buffer[..read]);
                    if let Some(end) = bytes.windows(4).position(|value| value == b"\r\n\r\n") {
                        let headers = std::str::from_utf8(&bytes[..end]).unwrap();
                        let length = headers
                            .lines()
                            .find_map(|line| {
                                let (name, value) = line.split_once(':')?;
                                name.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().unwrap())
                            })
                            .unwrap();
                        break (end + 4, length);
                    }
                };
                while bytes.len() < header_end + length {
                    let read = socket.read(&mut buffer).unwrap();
                    assert_ne!(read, 0);
                    bytes.extend_from_slice(&buffer[..read]);
                }
                let request: serde_json::Value =
                    serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap();
                let body = reply(index, &request);
                write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        (format!("http://{address}/v1"), handle)
    }
    fn text_response(text: &str, id: &str) -> String {
        format!(
            "data: {}\n\ndata: {}\n\n",
            serde_json::json!({"type":"response.output_text.delta","delta":text}),
            serde_json::json!({"type":"response.completed","response":{"id":id}})
        )
    }
    fn find_candidates(value: &serde_json::Value) -> Option<serde_json::Value> {
        match value {
            serde_json::Value::String(text) => text
                .strip_prefix("TASK_CONTEXT_CANDIDATES_V1\n")
                .map(|value| serde_json::from_str(value).unwrap()),
            serde_json::Value::Array(values) => values.iter().find_map(find_candidates),
            serde_json::Value::Object(values) => values.values().find_map(find_candidates),
            _ => None,
        }
    }

    let root = crate::support::TemporaryDirectory::new("task-vision-real-many");
    let project = Project::initialize(root.path()).unwrap();
    let fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/task_context_legacy");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(fixture.join("manifest.json")).unwrap()).unwrap();
    let case = &manifest["cases"]["secondary"];
    for (relative, hash) in case["files"].as_object().unwrap() {
        if relative == ".echidna/project.toml" {
            continue;
        }
        let hash = hash.as_str().unwrap();
        let bytes = std::fs::read(fixture.join("blobs").join(hash)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(&bytes)), hash);
        let bytes = if relative.ends_with("/events.jsonl") {
            std::str::from_utf8(&bytes)
                .unwrap()
                .lines()
                .map(|line| {
                    let mut event: serde_json::Value = serde_json::from_str(line).unwrap();
                    if event["event_type"] == "session.request_audit_recorded" {
                        event["payload"]["binding"]["project_id"] =
                            serde_json::json!(project.project_id().as_str());
                        format!("{event}\n")
                    } else {
                        format!("{line}\n")
                    }
                })
                .collect::<String>()
                .into_bytes()
        } else {
            bytes
        };
        let path = root.path().join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
    let session = case["session_id"].as_str().unwrap();
    let event_path = root
        .path()
        .join(".echidna/sessions")
        .join(session)
        .join("events.jsonl");
    let read_state = || {
        let data = std::fs::read_to_string(&event_path).unwrap();
        SessionReducer
            .reduce(data.lines().map(|line| {
                serde_json::from_str::<EventEnvelope<serde_json::Value>>(line).unwrap()
            }))
            .unwrap()
    };
    let original = read_state();
    let original_followup = original.history_items_with_event_ids().find(|(_, item)| {
        matches!(item, ContentItem::Text { text, .. } if text.contains("fixture screenshot"))
    }).unwrap().0.clone();
    let (primary_url, primary) = server(5, |index, request| match index {
        0 => {
            let candidates = find_candidates(&request["input"]).expect("task selector candidates");
            text_response(&serde_json::json!({"relation":"new", "focus":{"kind":"new_from_input","input_ref":candidates["input"]["ref"]}, "related_task_refs":[],"required_context_refs":[],"unresolved_references":[]}).to_string(), &format!("vision-select-{index}"))
        }
        1..=3 => {
            let args = serde_json::json!({"event_id":format!("vision-event-{index}"), "occurred_at":"2026-09-12T00:00:00Z", "artifact_id":format!("vision-artifact-{index}"), "source_path":"screen.png", "target_snapshot_id":"snapshot-legacy-followup-source", "toolchain_fingerprint":"fixture"});
            format!(
                "data: {}\n\ndata: {}\n\n",
                serde_json::json!({"type":"response.output_item.done","item":{"type":"function_call","call_id":format!("vision-call-{index}"),"name":"view_image","arguments":args.to_string()}}),
                serde_json::json!({"type":"response.completed","response":{"id":format!("vision-call-response-{index}")}})
            )
        }
        _ => text_response("Vision turn complete.", &format!("vision-answer-{index}")),
    });
    let (vision_url, vision) = server(3, |index, _| {
        let summary = if index == 0 {
            format!("VISION-LARGE-{}", "x".repeat(120_000))
        } else if index == 1 {
            "VISION-MIDDLE".to_owned()
        } else {
            "VISION-LAST".to_owned()
        };
        text_response(&serde_json::json!({"schema_version":2,"summary":summary,"extracted_text":"fixture screenshot","regions":[],"task_observations":["fixture boundary marker"],"uncertainty":["synthetic input only"]}).to_string(), &format!("vision-provider-{index}"))
    });
    let config = root.path().join("vision-provider.toml");
    std::fs::write(
        &config,
        crate::provider_config::fixture()
            .replacen("http://127.0.0.1:1/v1", &primary_url, 1)
            .replacen("http://127.0.0.1:1/v1", &vision_url, 1)
            .replacen("context_window = 32768", "context_window = 272000", 1),
    )
    .unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .current_dir(root.path())
        .args([
            "chat",
            "--resume",
            session,
            "--config",
            config.to_str().unwrap(),
        ])
        .env("CYBER_AGENT_PRIMARY_API_KEY", "loopback-primary-secret")
        .env("CYBER_AGENT_VISION_API_KEY", "loopback-vision-secret")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"Inspect three screenshot views.\n/exit\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    primary.join().unwrap();
    vision.join().unwrap();
    let events = std::fs::read_to_string(&event_path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    let first = events
        .iter()
        .position(|event| {
            event["event_type"] == "session.history_item_recorded"
                && event["payload"]["item"]["text"]
                    .as_str()
                    .is_some_and(|text| text.contains("VISION-LARGE-"))
        })
        .unwrap();
    let last = events
        .iter()
        .position(|event| {
            event["event_type"] == "session.history_item_recorded"
                && event["payload"]["item"]["text"]
                    .as_str()
                    .is_some_and(|text| text.contains("VISION-LAST"))
        })
        .unwrap();
    assert!(last > first);
    assert!(
        !events[first + 1..last]
            .iter()
            .any(
                |event| event["event_type"] == "session.operator_turn_boundary_recorded"
                    && event["payload"]["kind"] == "accepted"
            )
    );
    let extended = read_state();
    assert_eq!(extended.vision_observations().len(), 4);
    assert!(
        extended
            .vision_observations()
            .iter()
            .any(|observation| observation.summary.len() >= 120_000)
    );
    let late_followup = extended.history_items_with_event_ids().find(|(_, item)| {
        matches!(item, ContentItem::Text { text, .. } if text.contains("VISION-LAST"))
    }).unwrap().0.clone();
    let measure = |state: &crate::domain::session::SessionState, target: &EventId| {
        let item = state
            .history_items_with_event_ids()
            .find(|(id, _)| *id == target)
            .unwrap();
        state.history_source_semantics_many(&[item]).unwrap();
        ALLOCATED.with(|total| total.set(0));
        ENABLED.with(|enabled| enabled.set(true));
        let semantics = state.history_source_semantics_many(&[item]).unwrap();
        ENABLED.with(|enabled| enabled.set(false));
        (semantics, ALLOCATED.with(Cell::get))
    };
    let (before, small_bytes) = measure(&original, &original_followup);
    let (after, large_bytes) = measure(&extended, &original_followup);
    assert_eq!(after, before);
    assert_eq!(after[0].authority, AuthorityLevel::ModelObservation);
    assert!(
        large_bytes <= small_bytes + 4096,
        "source allocation grew from {small_bytes} to {large_bytes} bytes"
    );
    let (_, late_bytes) = measure(&extended, &late_followup);
    assert!(
        late_bytes <= large_bytes + 8192,
        "late followup allocated {late_bytes} bytes versus {large_bytes} for the earlier source"
    );
    let full = extended.followup_vision_inputs().unwrap();
    for marker in ["VISION-LARGE-", "VISION-MIDDLE", "VISION-LAST"] {
        let item = extended
            .history_items_with_event_ids()
            .find(
                |(_, item)| matches!(item, ContentItem::Text { text, .. } if text.contains(marker)),
            )
            .unwrap();
        let provenance = full.iter().find(|input| &input.event_id == item.0).unwrap();
        let semantics = extended.history_source_semantics_many(&[item]).unwrap()[0];
        assert_eq!(semantics.origin, provenance.provenance.origin());
        assert_eq!(semantics.class, provenance.provenance.semantic_class());
        assert_eq!(semantics.authority, provenance.provenance.authority_level());
    }
}
