//! Composed offline boundary test: real CLI selections and source verification,
//! then their exact main IR/epochs through the real subscription Adapter.
//! The CLI uses local API-key transport; this is not a subscription login/Live test.
use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;

use crate::adapters::project_event_store::{Project, read_existing_session_events};
use crate::adapters::responses::presentation::{
    RequestObserver, ResponseNotice, ResponsePresentation, ResponsesObserver,
};
use crate::adapters::responses::{ResponsesHttpAdapter, ResponsesHttpConfig};
use crate::app::session::SessionProjection;
use crate::domain::model::{ContentItem, ModelEvent, OpaqueContinuationScope};
use crate::ports::model::{ModelProvider, ModelRequest, ModelStreamEvent};
use crate::rsi_subscription_continuation::continuation_regression::{
    FixtureCredential, capabilities, completed, encrypted, read_request, server,
};
use crate::support::TemporaryDirectory;

fn candidate(value: &Value) -> Option<Value> {
    match value {
        Value::String(text) => text
            .strip_prefix("TASK_CONTEXT_CANDIDATES_V1\n")
            .map(|text| serde_json::from_str(text).unwrap()),
        Value::Array(values) => values.iter().find_map(candidate),
        Value::Object(values) => values.values().find_map(candidate),
        _ => None,
    }
}

fn chat(root: &std::path::Path, config: &std::path::Path, input: &str, args: &[&str]) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .current_dir(root)
        .args(["chat", "--config", config.to_str().unwrap()])
        .args(args)
        .env("CYBER_AGENT_PRIMARY_API_KEY", "synthetic-loopback-key")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(120);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let output = child.wait_with_output().unwrap();
            panic!(
                "synthetic CLI timed out: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!String::from_utf8_lossy(&output.stderr).contains("[turn stopped]"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_selected_sources_and_epochs_bind_real_subscription_opaque_delivery() {
    struct Observation {
        value: Value,
        omitted: Vec<&'static str>,
    }
    struct Recorder(Arc<Mutex<Vec<Observation>>>);
    impl ResponsesObserver for Recorder {
        fn role(&self) -> &'static str {
            "primary"
        }
        fn begin(
            &self,
            _: crate::ports::model::ModelRequestPresentation,
        ) -> Box<dyn RequestObserver> {
            Box::new(Recorder(self.0.clone()))
        }
        fn notify(&self, _: ResponseNotice) {}
        fn full_debug(&self) -> bool {
            true
        }
    }
    impl RequestObserver for Recorder {
        fn observe(&mut self, event: ResponsePresentation) {
            if let ResponsePresentation::Request { value, omitted } = event {
                self.0.lock().unwrap().push(Observation { value, omitted });
            }
        }
    }
    let root = TemporaryDirectory::new("task-cli-subscription-boundaries");
    let init = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .current_dir(root.path())
        .arg("init")
        .output()
        .unwrap();
    assert!(init.status.success());
    let definition = root.path().join("agent");
    fs::create_dir(&definition).unwrap();
    fs::write(
        definition.join("persona.md"),
        "A neutral synthetic task assistant.",
    )
    .unwrap();
    fs::write(
        definition.join("work.md"),
        "Preserve source identity and current goals.",
    )
    .unwrap();
    fs::write(definition.join("agent.toml"), "schema_version = 1\nid = \"task-wire-fixture\"\nname = \"Task wire fixture\"\n[instructions]\npersona_file = \"persona.md\"\nwork_file = \"work.md\"\nmethod_files = []\n[context]\npersonal = \"none\"\nproject = \"none\"\nmemory = \"none\"\n[tools]\nread_file = \"direct\"\n").unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let cli_server = tokio::spawn(async move {
        let mut wires = Vec::new();
        for index in 0..4 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let wire = read_request(&mut socket).await;
            let answer = match index {
                0 | 2 => {
                    assert!(wire.get("tools").is_none());
                    let source = candidate(&wire).unwrap();
                    assert_eq!(source["tasks"].as_array().unwrap().len(), index / 2);
                    assert_eq!(
                        source["input"]["text"],
                        if index == 0 {
                            "A_ONLY_ORIGINAL_TASK"
                        } else {
                            "B_ONLY_UNRELATED_TASK"
                        }
                    );
                    json!({"relation":"new","focus":{"kind":"new_from_input","input_ref":source["input"]["ref"]},
                        "related_task_refs":[],"required_context_refs":[],"unresolved_references":[]}).to_string()
                }
                1 => "A_ONLY_ORIGINAL_CONCLUSION".to_owned(),
                _ => "B_ONLY_RESULT".to_owned(),
            };
            let body = completed(&format!("cli-{index}"), Vec::new(), Some(&answer));
            let headers = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            socket.write_all(headers.as_bytes()).await.unwrap();
            socket.write_all(body.as_bytes()).await.unwrap();
            socket.shutdown().await.unwrap();
            wires.push(wire);
        }
        wires
    });
    let config = root.path().join("provider.toml");
    fs::write(
        &config,
        crate::provider_config::fixture()
            .replace("http://127.0.0.1:1/v1", &url)
            .replace("incremental_input = true", "incremental_input = false")
            .replace("context_window = 32768", "context_window = 272000"),
    )
    .unwrap();
    chat(
        root.path(),
        &config,
        "A_ONLY_ORIGINAL_TASK\nB_ONLY_UNRELATED_TASK\n/exit\n",
        &["--agent", definition.to_str().unwrap()],
    );
    let cli_wires = cli_server.await.unwrap();
    assert!(!cli_wires[3].to_string().contains("A_ONLY"));
    // The actual CLI source verifier, rather than audit self-reconstruction,
    // validates this selected Session before its IR is used at the Adapter seam.
    chat(
        root.path(),
        &config,
        "/context\n/exit\n",
        &["--resume", "latest"],
    );
    let project = Project::discover(root.path()).unwrap();
    let session = fs::read_dir(root.path().join(".echidna/sessions"))
        .unwrap()
        .map(|entry| entry.unwrap())
        .find(|entry| entry.file_type().unwrap().is_dir())
        .unwrap();
    let id = crate::domain::events::StreamId::new(session.file_name().to_str().unwrap());
    let events = read_existing_session_events(&project, &id).unwrap();
    let state = SessionProjection::replay(events).unwrap();
    let audits = state.state().request_audits();
    assert_eq!(audits.len(), 4);
    let blobs = root.path().join(".echidna/artifacts/blobs/sha256");
    let first = audits[1]
        .reconstruct(|id| fs::read(blobs.join(id.as_str())))
        .unwrap();
    let second = audits[3]
        .reconstruct(|id| fs::read(blobs.join(id.as_str())))
        .unwrap();
    let scope_a =
        OpaqueContinuationScope::for_provider_session(audits[1].provider_session().unwrap());
    let scope_b =
        OpaqueContinuationScope::for_provider_session(audits[3].provider_session().unwrap());
    assert_ne!(scope_a, scope_b);
    assert!(
        serde_json::to_string(&second)
            .unwrap()
            .contains("B_ONLY_UNRELATED_TASK")
    );
    assert!(!serde_json::to_string(&second).unwrap().contains("A_ONLY"));
    let (url, mut requests, adapter_server) = server(vec![
        completed(
            "opaque-first",
            vec![encrypted("cli-parent")],
            Some("A_ONLY_ORIGINAL_CONCLUSION"),
        ),
        completed("opaque-same", Vec::new(), Some("same epoch")),
        completed("opaque-new", Vec::new(), Some("B_ONLY_RESULT")),
    ])
    .await;
    let mut mapping = capabilities();
    mapping.context_window = 272_000;
    let observed = Arc::new(Mutex::new(Vec::new()));
    let provider = ResponsesHttpAdapter::new(
        ResponsesHttpConfig::new(
            url,
            "fixture-subscription",
            "codex_subscription:rsi-isolation",
            mapping,
        )
        .with_codex_subscription_profile(),
        Arc::new(FixtureCredential),
    )
    .with_observer(Some(Arc::new(Recorder(observed.clone()))));
    let mut stream = provider
        .stream(ModelRequest::full(first.clone()).with_opaque_continuation_scope(scope_a.clone()))
        .unwrap();
    let mut opaque = None;
    while let Some(event) = stream.next().await {
        if let ModelStreamEvent::Event(ModelEvent::OpaqueContinuation { reference }) =
            event.unwrap()
        {
            assert!(opaque.replace(reference).is_none());
        }
    }
    let opaque = opaque.expect("the subscription Adapter actually decoded encrypted reasoning");
    let mut same = first;
    same.content_items.push(ContentItem::OpaqueContinuation {
        reference: opaque.clone(),
    });
    let mut stream = provider
        .stream(ModelRequest::full(same).with_opaque_continuation_scope(scope_a))
        .unwrap();
    while let Some(event) = stream.next().await {
        event.unwrap();
    }
    let mut illegal = second.clone();
    illegal
        .content_items
        .push(ContentItem::OpaqueContinuation { reference: opaque });
    assert!(
        provider
            .prepare(ModelRequest::full(illegal).with_opaque_continuation_scope(scope_b.clone()))
            .is_err()
    );
    let mut stream = provider
        .stream(ModelRequest::full(second).with_opaque_continuation_scope(scope_b))
        .unwrap();
    while let Some(event) = stream.next().await {
        event.unwrap();
    }
    adapter_server.await.unwrap();
    let mut wires = Vec::new();
    while let Some(wire) = requests.recv().await {
        wires.push(wire);
    }
    assert_eq!(wires.len(), 3);
    assert!(wires[1].to_string().contains("ciphertext-cli-parent"));
    let observed = observed.lock().unwrap();
    assert_eq!(observed.len(), 3);
    assert!(
        !observed[1]
            .value
            .to_string()
            .contains("ciphertext-cli-parent")
    );
    assert!(observed[1].omitted.contains(&"opaque_continuation"));
    assert!(
        observed[1].value["input"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["type"] == "reasoning" && item["omitted"] == "opaque_continuation")
    );
    assert!(!wires[2].to_string().contains("ciphertext-cli-parent"));
    assert!(!wires[2].to_string().contains("A_ONLY"));
    assert!(wires[2].to_string().contains("B_ONLY_UNRELATED_TASK"));
    assert!(
        wires
            .iter()
            .all(|wire| wire.get("previous_response_id").is_none())
    );
}
