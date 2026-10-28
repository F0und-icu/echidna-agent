use super::*;

async fn consume_usage(
    usage: Option<Value>,
    subscription: bool,
) -> Result<ModelStreamOutcome, ModelStreamingError> {
    let mut response = json!({"id": "response-usage", "status": "completed"});
    if let Some(usage) = usage {
        response["usage"] = usage;
    }
    let body = format!(
        "data: {}\n\n",
        json!({"type": "response.completed", "response": response})
    );
    let (url, request, server) = spawn_sse_server(200, &body).await;
    let source = FakeCredentialSource::value(Some(FIXTURE_SECRET));
    let provider = if subscription {
        subscription_adapter(url, source)
    } else {
        adapter(url, source)
    };
    let input = fixture_input(ModelRequestRequirements::default());
    let model_request = if subscription {
        subscription_full(input, "usage-scope")
    } else {
        full(input)
    };
    let result = ModelStreamingService::new(&provider)
        .consume(model_request, ModelStreamCancellation::new())
        .await;
    request.await.unwrap();
    server.await.unwrap();
    result
}

#[tokio::test]
async fn canonical_usage_subsets_survive_standard_and_subscription_decoding() {
    for subscription in [false, true] {
        for (cached, reasoning) in [(0, 0), (20, 10), (100, 50)] {
            let outcome = consume_usage(
                Some(json!({
                    "input_tokens": 100,
                    "output_tokens": 50,
                    "input_tokens_details": {"cached_tokens": cached},
                    "output_tokens_details": {"reasoning_tokens": reasoning},
                })),
                subscription,
            )
            .await
            .unwrap();
            assert_eq!(
                outcome,
                ModelStreamOutcome::Completed {
                    events: vec![ModelEvent::Usage {
                        usage: ModelUsage {
                            input_tokens: 100,
                            output_tokens: 50,
                            cached_input_tokens: Some(cached),
                            reasoning_output_tokens: Some(reasoning),
                        },
                    }],
                    completion: ModelResponseCompletion::new(ProviderResponseId::new(
                        "response-usage",
                    )),
                },
            );
        }
    }
}

#[tokio::test]
async fn absent_usage_and_missing_subsets_remain_unknown_without_alias_inference() {
    for subscription in [false, true] {
        let outcome = consume_usage(None, subscription).await.unwrap();
        assert!(
            matches!(outcome, ModelStreamOutcome::Completed { events, .. } if events.is_empty())
        );

        for (details, cached, reasoning) in [
            (json!({}), None, None),
            (
                json!({"input_tokens_details": {}, "output_tokens_details": {}}),
                None,
                None,
            ),
            (
                json!({"input_tokens_details": null, "output_tokens_details": null}),
                None,
                None,
            ),
            (
                json!({"input_tokens_details": {"cached_tokens": 20}}),
                Some(20),
                None,
            ),
            (
                json!({"output_tokens_details": {"reasoning_tokens": 10}}),
                None,
                Some(10),
            ),
            (
                json!({"cached_tokens": 20, "reasoning_tokens": 10,
                    "prompt_tokens_details": {"cached_tokens": 20},
                    "completion_tokens_details": {"reasoning_tokens": 10}}),
                None,
                None,
            ),
        ] {
            let mut usage = json!({"input_tokens": 100, "output_tokens": 50});
            usage
                .as_object_mut()
                .unwrap()
                .extend(details.as_object().unwrap().clone());
            let outcome = consume_usage(Some(usage), subscription).await.unwrap();
            assert!(
                matches!(outcome, ModelStreamOutcome::Completed { events, .. }
                if events == vec![ModelEvent::Usage { usage: ModelUsage {
                    input_tokens: 100, output_tokens: 50,
                    cached_input_tokens: cached, reasoning_output_tokens: reasoning,
                }}])
            );
        }
    }
}

#[tokio::test]
async fn inconsistent_usage_subsets_fail_at_the_responses_boundary() {
    for subscription in [false, true] {
        for (details, expected) in [
            (
                json!({"input_tokens_details": {"cached_tokens": 101}}),
                "`input_tokens_details.cached_tokens` 101 exceeds total 100",
            ),
            (
                json!({"output_tokens_details": {"reasoning_tokens": 51}}),
                "`output_tokens_details.reasoning_tokens` 51 exceeds total 50",
            ),
        ] {
            let mut usage = json!({"input_tokens": 100, "output_tokens": 50});
            usage
                .as_object_mut()
                .unwrap()
                .extend(details.as_object().unwrap().clone());
            assert!(matches!(consume_usage(Some(usage), subscription).await,
                Err(ModelStreamingError::Provider(ModelProviderError::ProtocolFailure { message }))
                    if message.contains(expected)));
        }
    }
}

#[tokio::test]
async fn invalid_usage_subset_types_preserve_the_wire_decode_error() {
    for field in ["input_tokens_details", "output_tokens_details"] {
        let counter = if field == "input_tokens_details" {
            "cached_tokens"
        } else {
            "reasoning_tokens"
        };
        for value in [json!(-1), json!(0.5), json!("1")] {
            let mut usage = json!({"input_tokens": 100, "output_tokens": 50});
            usage[field] = json!({counter: value});
            assert!(matches!(consume_usage(Some(usage), false).await,
                Err(ModelStreamingError::Provider(ModelProviderError::ProtocolFailure { message }))
                    if message.starts_with("invalid completed response:")
                        && (message.contains("invalid value") || message.contains("invalid type"))));
        }
    }
}
