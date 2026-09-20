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
