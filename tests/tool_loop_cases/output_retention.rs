use super::*;
use cyber_agent::domain::artifacts::BlobId;
use cyber_agent::ports::artifact::{ArtifactStore, StoredBlob};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Clone, Default)]
struct Captures {
    blobs: Arc<Mutex<BTreeMap<BlobId, Vec<u8>>>>,
    calls: Arc<AtomicU64>,
    fail_on: Option<u64>,
}

impl ArtifactStore for Captures {
    type Error = &'static str;
    fn publish_file(&self, _: &Path) -> Result<StoredBlob, Self::Error> {
        unreachable!()
    }
    fn publish_bytes(&self, bytes: &[u8]) -> Result<StoredBlob, Self::Error> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        if self.fail_on == Some(call) {
            return Err("injected disk failure");
        }
        let blob_id = BlobId::new(format!("{:x}", Sha256::digest(bytes))).unwrap();
        let previous = self
            .blobs
            .lock()
            .unwrap()
            .insert(blob_id.clone(), bytes.to_vec());
        Ok(StoredBlob {
            blob_id,
            size_bytes: bytes.len() as u64,
            already_present: previous.is_some(),
        })
    }
    fn read_blob(&self, id: &BlobId) -> Result<Vec<u8>, Self::Error> {
        self.blobs.lock().unwrap().get(id).cloned().ok_or("missing")
    }
}

fn provider(name: &str, args: Value) -> ScriptedProvider {
    let mut provider = ScriptedProvider::new([
        vec![ModelEvent::ToolCall {
            call: tool_call("capture", name, args),
        }],
        vec![ModelEvent::TextDelta {
            text: "done".to_owned(),
        }],
    ]);
    provider.capabilities.context_window = 1_048_576;
    provider
}

#[tokio::test]
async fn raw_capture_survives_preview_and_keeps_exact_non_utf8_bytes() {
    let project = TestProject::new("raw-capture");
    let tools = ToolRegistry::for_project(&project.root);
    let store = Captures::default();
    let provider = provider(
        "run_command",
        json!({"argv":["sh","-c","dd if=/dev/zero bs=1024 count=150 2>/dev/null; printf '\\377TAIL'; printf ERR >&2"]}),
    );
    let mut agent = AgentLoop::new(&provider, &tools).with_output_store(store.clone());
    let outcome = agent
        .run_turn("capture", AgentCancellation::new())
        .await
        .unwrap();
    let AgentTurnOutcome::Completed { executions, .. } = outcome else {
        panic!("completed")
    };
    let output = &executions[0];
    assert_eq!(output.receipt.output_captures.len(), 2);
    let capture = &output.receipt.output_captures[0];
    assert_eq!(capture.observed_bytes, 153605);
    assert!(capture.complete && capture.eof);
    let raw = store.read_blob(&capture.blob_id).unwrap();
    assert!(raw.ends_with(b"\xffTAIL"));
    assert_eq!(raw.len() as u64, capture.retained_bytes);
    assert!(
        output.result.output["stdout"]
            .as_str()
            .unwrap()
            .ends_with("TAIL")
    );
    assert!(output.result.output["stdout"].as_str().unwrap().len() <= 65536);
    assert_eq!(output.result.output["truncated"], true);
    assert_eq!(
        output.result.output["output_captures"],
        serde_json::to_value(&output.receipt.output_captures).unwrap()
    );
    assert!(
        !serde_json::to_value(output)
            .unwrap()
            .as_object()
            .unwrap()
            .contains_key("pending_captures")
    );
    assert_eq!(provider.requests().len(), 2);
}

#[tokio::test]
async fn raw_capture_limit_and_32_children_are_explicit_and_bounded() {
    for count in [1, 32] {
        let project = TestProject::new("raw-cap");
        let tools = ToolRegistry::for_project(&project.root);
        let store = Captures::default();
        let command = json!({"argv":["sh","-c","dd if=/dev/zero bs=1024 count=1100 2>/dev/null; printf LOST_TAIL"]});
        let provider = if count == 1 {
            provider("run_command", command)
        } else {
            provider(
                "run_commands",
                json!({"mode":"serial","commands":vec![command; count]}),
            )
        };
        let mut agent = AgentLoop::new(&provider, &tools).with_output_store(store.clone());
        let outcome = agent
            .run_turn("capture", AgentCancellation::new())
            .await
            .unwrap();
        let AgentTurnOutcome::Completed { executions, .. } = outcome else {
            panic!("completed")
        };
        let refs = &executions[0].receipt.output_captures;
        assert_eq!(refs.len(), count * 2);
        assert!(
            refs.iter()
                .map(|capture| capture.retained_bytes)
                .sum::<u64>()
                <= 8 * 1024 * 1024
        );
        for (index, capture) in refs.iter().step_by(2).enumerate() {
            assert_eq!(capture.observed_bytes, 1126409);
            assert!(!capture.complete && capture.eof);
            assert_eq!(
                capture.retained_bytes,
                if count == 1 { 1048576 } else { 131072 }
            );
            assert_eq!(capture.child_index, (count > 1).then_some(index as u8));
            assert!(
                !store
                    .read_blob(&capture.blob_id)
                    .unwrap()
                    .ends_with(b"LOST_TAIL")
            );
        }
    }
}

#[tokio::test]
async fn missing_or_failed_capture_store_never_retries_or_appends_tool_result() {
    for failure in [None, Some(1), Some(2)] {
        let project = TestProject::new("capture-failure");
        let tools = ToolRegistry::for_project(&project.root);
        let provider = provider(
            "run_command",
            json!({"argv":["sh","-c","printf x >> marker; printf OUT; printf ERR >&2"]}),
        );
        let mut agent = AgentLoop::new(&provider, &tools);
        if failure.is_some() {
            agent = agent.with_output_store(Captures {
                fail_on: failure,
                ..Default::default()
            });
        }
        let audit_context = RequestAuditRunContext::new(
            RequestAuditBinding::new(
                ProjectId::new("capture-failure"),
                StreamId::new("session:capture-failure"),
            )
            .unwrap(),
            RequestAdapterVersion::new("fixture-v1").unwrap(),
        );
        let mut receipt_calls = 0;
        let error = agent
            .run_turn_audited_with_stages(
                "capture",
                AgentCancellation::new(),
                &audit_context,
                &mut |_| Ok(()),
                &mut |_| Ok(()),
                &mut |_| {
                    receipt_calls += 1;
                    Ok(())
                },
            )
            .await
            .unwrap_err();
        assert!(
            matches!(error, AgentLoopError::ToolOutputPublication { .. }),
            "{error}"
        );
        assert_eq!(provider.requests().len(), 1);
        assert_eq!(receipt_calls, 0, "no Receipt callback before publication");
        assert_eq!(fs::read(project.root.join("marker")).unwrap(), b"x");
        assert!(
            !agent
                .history()
                .items()
                .iter()
                .any(|item| matches!(item, ContentItem::ToolResult { .. }))
        );
    }
}

#[tokio::test]
async fn all_32_child_tails_within_retention_can_be_recovered() {
    let project = TestProject::new("batch-retained-tails");
    let tools = ToolRegistry::for_project(&project.root);
    let store = Captures::default();
    let commands: Vec<_> = (0..32).map(|index| json!({"argv":["sh","-c",format!("dd if=/dev/zero bs=1024 count=8 2>/dev/null; printf TAIL_{index}")]})).collect();
    let provider = provider(
        "run_commands",
        json!({"mode":"parallel","commands":commands}),
    );
    let mut agent = AgentLoop::new(&provider, &tools).with_output_store(store.clone());
    let result = agent
        .run_turn("capture all tails", AgentCancellation::new())
        .await
        .unwrap();
    let AgentTurnOutcome::Completed { executions, .. } = result else {
        panic!("completed")
    };
    for (index, capture) in executions[0]
        .receipt
        .output_captures
        .iter()
        .step_by(2)
        .enumerate()
    {
        assert_eq!(capture.child_index, Some(index as u8));
        assert!(capture.complete);
        assert!(
            store
                .read_blob(&capture.blob_id)
                .unwrap()
                .ends_with(format!("TAIL_{index}").as_bytes())
        );
    }
}

#[tokio::test]
async fn binary_text_expansion_preserves_preview_tail_and_byte_bound() {
    let project = TestProject::new("binary-preview");
    let mut bytes = vec![0xff; 40000];
    bytes.extend_from_slice(b"BINARY_TAIL");
    project.write("binary.bin", &bytes);
    let tools = ToolRegistry::for_project(&project.root);
    let store = Captures::default();
    let provider = provider("run_command", json!({"argv":["cat","binary.bin"]}));
    let mut agent = AgentLoop::new(&provider, &tools).with_output_store(store.clone());
    let result = agent
        .run_turn("capture", AgentCancellation::new())
        .await
        .unwrap();
    let AgentTurnOutcome::Completed { executions, .. } = result else {
        panic!("completed")
    };
    let output = &executions[0].result.output;
    assert!(output["stdout"].as_str().unwrap().ends_with("BINARY_TAIL"));
    assert!(output["stdout"].as_str().unwrap().len() <= 65536);
    assert_eq!(output["truncated"], true);
    let capture = &executions[0].receipt.output_captures[0];
    assert!(capture.complete);
    assert_eq!(store.read_blob(&capture.blob_id).unwrap(), bytes);
}

#[tokio::test]
async fn terminal_job_and_pty_snapshots_use_the_same_durable_capture_path() {
    for pty in [false, true] {
        let project = TestProject::new("terminal-capture");
        let tools = ToolRegistry::for_project(&project.root);
        let start = tools
            .execute(plan(
                &tools,
                "launch",
                if pty { "start_pty" } else { "start_job" },
                if pty {
                    json!({"argv":["sh","-c","printf TERMINAL"],"rows":24,"cols":80})
                } else {
                    json!({"argv":["sh","-c","printf TERMINAL"]})
                },
            ))
            .await;
        let key = if pty { "pty_session_id" } else { "job_id" };
        let id = start.result.output[key].clone();
        let args = json!({key:id});
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let poll = tools
                    .execute(plan(
                        &tools,
                        "poll",
                        if pty { "poll_pty" } else { "poll_job" },
                        args.clone(),
                    ))
                    .await;
                if !poll.result.output["completion"].is_null() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let provider = provider(if pty { "stop_pty" } else { "stop_job" }, args);
        let store = Captures::default();
        let mut agent = AgentLoop::new(&provider, &tools)
            .with_output_store(store.clone())
            .with_prompt_composition(
                PromptComposition::current().with_research_phase(ResearchPhase::Experiment),
            );
        let result = agent
            .run_turn("inspect terminal", AgentCancellation::new())
            .await
            .unwrap();
        let AgentTurnOutcome::Completed { executions, .. } = result else {
            panic!("completed")
        };
        let refs = &executions[0].receipt.output_captures;
        assert_eq!(refs.len(), if pty { 1 } else { 2 });
        assert!(refs[0].complete);
        assert_eq!(store.read_blob(&refs[0].blob_id).unwrap(), b"TERMINAL");
        assert_eq!(executions[0].result.output["already_terminal"], true);
    }
}

#[tokio::test]
async fn job_reader_grace_expiry_keeps_observed_bytes_without_claiming_eof() {
    let project = TestProject::new("capture-incomplete");
    let tools = ToolRegistry::for_project(&project.root);
    let started = tools
        .execute(plan(
            &tools,
            "launch",
            "start_job",
            json!({"argv":["sh","-c","printf RETAINED; sleep 2 &"]}),
        ))
        .await;
    let id = started.result.output["job_id"].clone();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let poll = tools
                .execute(plan(&tools, "poll", "poll_job", json!({"job_id":id})))
                .await;
            if !poll.result.output["completion"].is_null() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let provider = provider("stop_job", json!({"job_id":id}));
    let store = Captures::default();
    let mut agent = AgentLoop::new(&provider, &tools)
        .with_output_store(store.clone())
        .with_prompt_composition(
            PromptComposition::current().with_research_phase(ResearchPhase::Experiment),
        );
    let result = agent
        .run_turn("inspect incomplete", AgentCancellation::new())
        .await
        .unwrap();
    let AgentTurnOutcome::Completed { executions, .. } = result else {
        panic!("completed")
    };
    let capture = &executions[0].receipt.output_captures[0];
    assert!(!capture.complete && !capture.eof);
    assert!(capture.read_error.is_some());
    assert_eq!(capture.observed_bytes, 8);
    assert_eq!(store.read_blob(&capture.blob_id).unwrap(), b"RETAINED");
    // The deliberately inherited descriptor belongs to this two-second child.
    // Let it exit within the bounded fixture lifetime, not as a leaked process.
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
}
