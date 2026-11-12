use super::*;

fn delta(index: u64, part: u64, text: &str) -> Value {
    json!({"type":"response.output_text.delta", "output_index":index,
        "content_index":part, "item_id":format!("message-{index}"), "delta":text})
}

fn done(index: u64, part: u64, text: &str) -> Value {
    json!({"type":"response.output_text.done", "output_index":index,
        "content_index":part, "item_id":format!("message-{index}"), "text":text})
}

fn message(index: u64, texts: &[&str]) -> Value {
    json!({"type":"message", "id":format!("message-{index}"), "status":"completed",
        "role":"assistant", "content":texts.iter().map(|text|
            json!({"type":"output_text","text":text})).collect::<Vec<_>>()})
}

async fn consume(events: Vec<Value>) -> Result<ModelStreamOutcome, ModelStreamingError> {
    let body: String = events
        .into_iter()
        .map(|v| format!("data: {v}\n\n"))
        .collect();
    let (url, request, server) = spawn_sse_server(200, &body).await;
    let provider = adapter(url, FakeCredentialSource::value(Some(FIXTURE_SECRET)));
    let result = ModelStreamingService::new(&provider)
        .consume(
            full(fixture_input(ModelRequestRequirements::default())),
            ModelStreamCancellation::new(),
        )
        .await;
    request.await.unwrap();
    server.await.unwrap();
    result
}

fn completed(output: Value) -> Value {
    json!({"type":"response.completed", "response":{"id":"response-final",
        "status":"completed", "output":output}})
}

#[tokio::test]
async fn incomplete_message_waits_for_terminal_failure_reason() {
    for terminal in [
        json!({"type":"response.incomplete","response":{"status":"incomplete",
            "incomplete_details":{"reason":"max_output_tokens"}}}),
        json!({"type":"response.failed","response":{"status":"failed",
            "error":{"code":"server_error","message":"terminal failure detail"}}}),
    ] {
        let mut item = message(0, &["partial answer"]);
        item["status"] = json!("incomplete");
        let result = consume(vec![
            delta(0, 0, "partial answer"),
            json!({"type":"response.output_item.done","output_index":0,"item":item,
                "sequence_number":838}),
            terminal.clone(),
        ])
        .await;
        let Err(ModelStreamingError::Provider(ModelProviderError::ProviderFailure {
            code,
            message,
        })) = result
        else {
            panic!("terminal reason must survive item finalization: {result:?}")
        };
        if terminal["type"] == "response.incomplete" {
            assert_eq!(code.as_deref(), Some("response_incomplete"));
            assert!(message.contains("max_output_tokens"), "{message}");
        } else {
            assert_eq!(code.as_deref(), Some("server_error"));
            assert!(message.contains("terminal failure detail"), "{message}");
        }
    }
}

#[tokio::test]
async fn incomplete_message_cannot_be_promoted_by_successful_terminal_snapshot() {
    let mut item = message(0, &["partial answer"]);
    item["status"] = json!("incomplete");
    for terminal in [
        completed(json!([item.clone()])),
        completed(json!([message(0, &["partial answer"])])),
        json!({"type":"response.completed","response":{"id":"missing-output"}}),
    ] {
        let result = consume(vec![
            json!({"type":"response.output_item.done","output_index":0,"item":item}),
            terminal,
        ])
        .await;
        assert!(matches!(
            result,
            Err(ModelStreamingError::Provider(
                ModelProviderError::ProtocolFailure { .. }
            ))
        ));
    }
}

#[tokio::test]
async fn incomplete_message_without_terminal_response_remains_interrupted() {
    let mut item = message(0, &["partial answer"]);
    item["status"] = json!("incomplete");
    let result = consume(vec![
        delta(0, 0, "partial answer"),
        json!({"type":"response.output_item.done","output_index":0,"item":item}),
    ])
    .await;
    assert!(matches!(
        result,
        Err(ModelStreamingError::Provider(
            ModelProviderError::ResponseInterrupted { .. }
        ))
    ));
}

#[tokio::test]
async fn reasoning_content_part_finalization_stays_out_of_assistant_text() {
    let reasoning = json!({"type":"reasoning","id":"reasoning-0",
        "content":[{"type":"reasoning_text","text":"reasoning only"}]});
    let result = consume(vec![
        json!({"type":"response.reasoning_text.delta","delta":"reasoning only"}),
        json!({"type":"response.content_part.done","output_index":0,"content_index":0,
            "item_id":"reasoning-0","part":{"type":"reasoning_text","text":"reasoning only"}}),
        json!({"type":"response.output_item.done","output_index":0,"item":reasoning}),
        delta(1, 0, "answer"),
        done(1, 0, "answer"),
        completed(json!([reasoning, message(1, &["answer"])])),
    ])
    .await
    .unwrap();
    let ModelStreamOutcome::Completed { events, .. } = result else {
        panic!()
    };
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, ModelEvent::ReasoningItem { .. }))
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter_map(|e| match e {
                ModelEvent::TextDelta { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<String>(),
        "answer"
    );
}

#[tokio::test]
async fn nullable_reasoning_content_matches_missing_and_empty_without_reconstructing_deltas() {
    for (case, status, content) in [
        ("missing", None, None),
        ("null", Some(Value::Null), Some(Value::Null)),
        ("empty", Some(json!("completed")), Some(json!([]))),
    ] {
        let mut reasoning = json!({"type":"reasoning","id":format!("reasoning-{case}")});
        if let Some(status) = status {
            reasoning["status"] = status;
        }
        if let Some(content) = content {
            reasoning["content"] = content;
        }
        let answer = message(1, &["answer"]);
        let result = consume(vec![
            json!({"type":"response.reasoning_text.delta","delta":"transient only"}),
            json!({"type":"response.output_item.done","output_index":0,
                "item":reasoning}),
            delta(1, 0, "answer"),
            done(1, 0, "answer"),
            completed(json!([reasoning, answer])),
        ])
        .await
        .unwrap_or_else(|error| panic!("{case}: {error}"));
        let ModelStreamOutcome::Completed { events, .. } = result else {
            panic!("{case}: expected completed response")
        };
        assert_eq!(
            events,
            vec![
                ModelEvent::ReasoningDelta {
                    text: "transient only".to_owned(),
                },
                ModelEvent::TextDelta {
                    text: "answer".to_owned(),
                },
            ],
            "{case}: a delta must not become completed reasoning"
        );
    }
}

#[tokio::test]
async fn nullable_reasoning_content_preserves_strict_content_validation() {
    let mut cases = Vec::new();
    for (case, content) in [
        ("string", json!("not-an-array")),
        ("object", json!({"type":"reasoning_text","text":"x"})),
        ("number", json!(1)),
        ("boolean", json!(true)),
    ] {
        cases.push((
            format!("non-array-{case}"),
            json!({"type":"reasoning","status":null,"content":content}),
            "invalid completed reasoning item:",
            Some("expected a sequence"),
        ));
    }
    cases.extend([
        (
            "null-child".to_owned(),
            json!({"type":"reasoning","status":null,"content":[null]}),
            "reasoning content omitted string `type`",
            None,
        ),
        (
            "non-string-type".to_owned(),
            json!({"type":"reasoning","status":null,
                "content":[{"type":7,"text":"x"}]}),
            "reasoning content omitted string `type`",
            None,
        ),
        (
            "unknown-type".to_owned(),
            json!({"type":"reasoning","status":null,
                "content":[{"type":"future_reasoning","text":"x"}]}),
            "unsupported completed reasoning content `future_reasoning`",
            None,
        ),
        (
            "missing-text".to_owned(),
            json!({"type":"reasoning","status":null,
                "content":[{"type":"reasoning_text"}]}),
            "reasoning text content omitted string `text`",
            None,
        ),
        (
            "non-string-text".to_owned(),
            json!({"type":"reasoning","status":null,
                "content":[{"type":"reasoning_text","text":7}]}),
            "reasoning text content omitted string `text`",
            None,
        ),
        (
            "encrypted-only".to_owned(),
            json!({"type":"reasoning","status":null,"content":null,
                "encrypted_content":"opaque"}),
            "completed reasoning item contains only unsupported encrypted content",
            None,
        ),
    ]);

    for (case, item, expected, additional) in cases {
        let result = consume(vec![
            json!({"type":"response.output_item.done","output_index":0,
                "item":item}),
            completed(json!([item])),
        ])
        .await;
        match result {
            Err(ModelStreamingError::Provider(ModelProviderError::ProtocolFailure { message })) => {
                assert!(message.contains(expected), "{case}: {message}");
                if let Some(additional) = additional {
                    assert!(message.contains(additional), "{case}: {message}");
                }
            }
            other => panic!("{case}: expected content protocol failure, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn nullable_and_nonempty_reasoning_content_cannot_disagree_across_snapshots() {
    let empty = json!({"type":"reasoning","status":null,"content":null});
    let nonempty = json!({"type":"reasoning","status":null,
        "content":[{"type":"reasoning_text","text":"completed"}]});
    for (case, item_done, snapshot) in [
        ("null-to-nonempty", empty.clone(), nonempty.clone()),
        ("nonempty-to-null", nonempty, empty),
    ] {
        let result = consume(vec![
            json!({"type":"response.output_item.done","output_index":0,
                "item":item_done}),
            completed(json!([snapshot])),
        ])
        .await;
        match result {
            Err(ModelStreamingError::Provider(ModelProviderError::ProtocolFailure { message })) => {
                assert!(
                    message.contains("completed output disagrees with completed non-text items"),
                    "{case}: {message}"
                )
            }
            other => panic!("{case}: expected snapshot mismatch, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn final_text_recovers_only_missing_suffix_and_deduplicates_all_final_forms() {
    for prefix in ["", "答复", "答复 complete"] {
        let item = message(0, &["答复 complete"]);
        let events = vec![
            delta(0, 0, prefix),
            done(0, 0, "答复 complete"),
            json!({"type":"response.content_part.done", "output_index":0,"content_index":0,
                "item_id":"message-0", "part":{"type":"output_text","text":"答复 complete"}}),
            json!({"type":"response.output_item.done","output_index":0,"item":item}),
            completed(json!([item])),
        ];
        let ModelStreamOutcome::Completed { events, .. } = consume(events).await.unwrap() else {
            panic!()
        };
        let text: String = events
            .into_iter()
            .filter_map(|e| match e {
                ModelEvent::TextDelta { text } => Some(text),
                _ => None,
            })
            .collect();
        assert_eq!(text, "答复 complete");
    }
}

#[tokio::test]
async fn final_message_and_completion_snapshots_supply_missing_text_without_duplication() {
    for at_completion in [false, true] {
        let first = message(0, &["one", "two"]);
        let second = message(1, &["three"]);
        let mut events = vec![delta(0, 0, "o")];
        if !at_completion {
            events.push(json!({"type":"response.output_item.done","output_index":0,"item":first}));
            events.push(json!({"type":"response.output_item.done","output_index":1,"item":second}));
        }
        events.push(completed(json!([first, second])));
        let ModelStreamOutcome::Completed { events, .. } = consume(events).await.unwrap() else {
            panic!()
        };
        let text: String = events
            .into_iter()
            .filter_map(|e| match e {
                ModelEvent::TextDelta { text } => Some(text),
                _ => None,
            })
            .collect();
        assert_eq!(text, "onetwothree");
    }
    let result = consume(vec![
        json!({"type":"response.output_text.delta","delta":"legacy"}),
        done(0, 0, "legacy suffix"),
        completed(json!([message(0, &["legacy suffix"])])),
    ])
    .await
    .unwrap();
    assert!(matches!(result, ModelStreamOutcome::Completed { .. }));
}

#[tokio::test]
async fn contradictory_or_out_of_order_final_text_never_completes() {
    let cases = vec![
        vec![
            delta(0, 0, "a"),
            json!({"type":"response.output_text.delta","output_index":0,
            "content_index":1,"item_id":"different","delta":"b"}),
        ],
        vec![delta(0, 0, "abc"), done(0, 0, "axc")],
        vec![done(0, 0, "a"), done(0, 0, "ab")],
        vec![done(0, 0, "a"), delta(0, 0, "b")],
        vec![delta(0, 0, "a"), delta(1, 0, "b"), done(0, 0, "ab")],
        vec![
            delta(0, 0, "a"),
            json!({"type":"response.output_item.done","output_index":1,
            "item":{"type":"function_call","call_id":"c","name":"read_file","arguments":"{}"}}),
            done(0, 0, "ab"),
        ],
        vec![
            delta(1, 0, "b"),
            completed(json!([message(0, &["a"]), message(1, &["b"])])),
        ],
        vec![
            delta(0, 0, "a"),
            json!({"type":"response.output_text.done","output_index":0,
            "content_index":0,"item_id":"different","text":"a"}),
        ],
        vec![json!({"type":"response.output_item.done","output_index":0,
            "item":{"type":"message","content":[{"type":"unknown","text":"x"}]}})],
        vec![json!({"type":"response.output_text.done","output_index":0,"content_index":0})],
    ];
    for mut events in cases {
        events.push(json!({"type":"response.completed","response":{"id":"invalid"}}));
        assert!(
            matches!(
                consume(events.clone()).await,
                Err(ModelStreamingError::Provider(
                    ModelProviderError::ProtocolFailure { .. }
                ))
            ),
            "{events:?}"
        );
    }
}

#[tokio::test]
async fn final_snapshots_preserve_text_tool_text_order() {
    let call = json!({"type":"function_call","call_id":"c","name":"read_file","arguments":"{}"});
    let events = vec![
        delta(0, 0, "before"),
        done(0, 0, "before"),
        json!({"type":"response.output_item.done","output_index":1,"item":call}),
        delta(2, 0, "aft"),
        done(2, 0, "after"),
        completed(json!([
            message(0, &["before"]),
            call,
            message(2, &["after"])
        ])),
    ];
    let ModelStreamOutcome::Completed { events, .. } = consume(events).await.unwrap() else {
        panic!()
    };
    assert!(matches!(&events[0],ModelEvent::TextDelta {text} if text == "before"));
    assert!(matches!(&events[1],ModelEvent::ToolCall {call} if call.call_id.as_str() == "c"));
    assert!(matches!(&events[2],ModelEvent::TextDelta {text} if text == "aft"));
    assert!(matches!(&events[3],ModelEvent::TextDelta {text} if text == "er"));
    assert_eq!(events.len(), 4);
}

#[tokio::test]
async fn completed_event_cannot_mask_noncompleted_status() {
    for status in ["incomplete", "failed", "in_progress", "cancelled"] {
        let result = consume(vec![
            delta(0, 0, "partial"),
            json!({"type":"response.completed",
            "response":{"id":"invalid","status":status}}),
        ])
        .await;
        assert!(result.is_err(), "{status}: {result:?}");
    }
}

#[tokio::test]
async fn nullable_status_is_rejected_outside_completed_reasoning_items() {
    let reasoning_content = json!([{"type":"reasoning_text","text":"bounded"}]);
    let mut cases = Vec::new();
    for (case, status) in [
        ("in-progress", json!("in_progress")),
        ("failed", json!("failed")),
        ("incomplete", json!("incomplete")),
        ("cancelled", json!("cancelled")),
        ("unknown-string", json!("future_status")),
        ("number", json!(1)),
        ("boolean", json!(true)),
        ("array", json!([])),
        ("object", json!({"unexpected":true})),
    ] {
        cases.push((
            format!("reasoning-{case}"),
            json!({"type":"reasoning","status":status,
                "content":reasoning_content}),
            "completed output item has non-completed status",
        ));
    }
    cases.extend([
        (
            "message-null".to_owned(),
            json!({"type":"message","status":null,
                "content":[{"type":"output_text","text":"answer"}]}),
            "completed message has non-completed status",
        ),
        (
            "function-call-null".to_owned(),
            json!({"type":"function_call","status":null,"call_id":"call-null",
                "name":"read_file","arguments":"{}"}),
            "completed output item has non-completed status",
        ),
    ]);

    for (case, item, expected_error) in cases {
        let result = consume(vec![
            json!({"type":"response.output_item.done","output_index":0,
                "item":item}),
            completed(json!([item])),
        ])
        .await;
        match result {
            Err(ModelStreamingError::Provider(ModelProviderError::ProtocolFailure { message })) => {
                assert!(message.contains(expected_error), "{case}: {message}")
            }
            other => panic!("{case}: expected status protocol failure, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn invalid_completed_item_status_uses_fixed_diagnostics_at_both_wire_entrypoints() {
    let cases = [
        (
            json!({"type":"reasoning","id":"reasoning-id",
                "status":"in_progress","content":[]}),
            "item_family=reasoning status_class=known_non_completed",
        ),
        (
            json!({"type":"function_call","id":"call-id","status":null,
                "call_id":"call-id","name":"read_file","arguments":"{}"}),
            "item_family=function_call status_class=null_outside_reasoning",
        ),
        (
            json!({"type":"REMOTE_TYPE_CANARY","id":"REMOTE_ID_CANARY",
                "status":"REMOTE_STATUS_CANARY"}),
            "item_family=other status_class=unknown_string",
        ),
        (
            json!({"type":"function_call","id":"REMOTE_ID_CANARY",
                "status":{"REMOTE_STATUS_OBJECT_CANARY":true},"call_id":"call-id",
                "name":"read_file","arguments":"{}"}),
            "item_family=function_call status_class=non_string",
        ),
    ];

    for (item, expected) in cases {
        for (source, events) in [
            (
                "output_item_done",
                vec![json!({"type":"response.output_item.done","output_index":0,
                    "item":item.clone()})],
            ),
            ("completed_snapshot", vec![completed(json!([item.clone()]))]),
        ] {
            let message = match consume(events).await {
                Err(ModelStreamingError::Provider(ModelProviderError::ProtocolFailure {
                    message,
                })) => message,
                other => panic!("{source}: expected status protocol failure, got {other:?}"),
            };
            assert_eq!(
                message,
                format!(
                    "completed output item has non-completed status (source={source} {expected})"
                )
            );
            for canary in [
                "REMOTE_TYPE_CANARY",
                "REMOTE_ID_CANARY",
                "REMOTE_STATUS_CANARY",
                "REMOTE_STATUS_OBJECT_CANARY",
            ] {
                assert!(!message.contains(canary), "{source}: {message}");
            }
        }
    }
}

#[tokio::test]
async fn completed_snapshots_cannot_drop_text_or_introduce_unobserved_semantic_items() {
    let tool = json!({"type":"function_call","call_id":"c","name":"read_file","arguments":"{}"});
    for events in [
        vec![delta(0, 0, "kept"), completed(json!([]))],
        vec![
            delta(0, 0, "a"),
            json!({"type":"response.output_text.done","output_index":1,"content_index":0,"item_id":"message-0","text":"a"}),
        ],
        vec![completed(json!([tool]))],
        vec![
            json!({"type":"response.output_item.done","output_index":0,"item":tool}),
            completed(json!([])),
        ],
        vec![completed(json!([{"type":"future_item"}]))],
    ] {
        assert!(consume(events).await.is_err());
    }
}

#[tokio::test]
async fn completed_snapshot_must_preserve_streamed_text_identity_and_type() {
    let reasoning = json!({"type":"reasoning","id":"reasoning-0",
        "content":[{"type":"reasoning_text","text":"a"}]});
    let mut missing_id = message(0, &["a"]);
    missing_id.as_object_mut().unwrap().remove("id");
    for events in [
        vec![
            delta(0, 0, "a"),
            done(0, 0, "a"),
            json!({"type":"response.output_item.done","output_index":0,"item":reasoning}),
            completed(json!([reasoning])),
        ],
        vec![delta(0, 0, "a"), completed(json!([missing_id]))],
        vec![delta(0, 0, "a"), completed(json!([message(1, &["a"])]))],
        vec![delta(0, 0, "a"), completed(json!([message(0, &["b"])]))],
        vec![
            delta(0, 0, "a"),
            completed(json!([{
                "type":"message","id":"message-0","content":[{"type":"reasoning_text","text":"a"}]
            }])),
        ],
    ] {
        assert!(
            matches!(
                consume(events.clone()).await,
                Err(ModelStreamingError::Provider(
                    ModelProviderError::ProtocolFailure { .. }
                ))
            ),
            "{events:?}"
        );
    }
}
