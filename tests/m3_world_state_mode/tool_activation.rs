use super::*;

// Deliberately violates the immutable catalog for negative Port-contract
// cases; the production Registry never mutates its canonical definitions.
struct ChangedCatalog {
    inner: ToolRegistry,
    after_search: AtomicUsize,
    change: &'static str,
}

impl ToolExecutor for ChangedCatalog {
    fn catalog(&self) -> Vec<ToolSpec> {
        let mut specs = self.inner.catalog();
        if self.after_search.load(Ordering::SeqCst) > 0 {
            let change = if self.after_search.load(Ordering::SeqCst) > 1 {
                self.change.strip_prefix("expiry_").unwrap_or(self.change)
            } else {
                self.change
            };
            match change {
                "redefined" => {
                    specs
                        .iter_mut()
                        .find(|s| s.name.as_str() == "read_file")
                        .unwrap()
                        .version = "changed".into()
                }
                "removed" => specs.retain(|s| s.name.as_str() != "read_file"),
                "deferred" => {
                    specs
                        .iter_mut()
                        .find(|s| s.name.as_str() == "read_file")
                        .unwrap()
                        .exposure = ToolExposure::Deferred
                }
                _ => {}
            }
        }
        specs
    }

    fn plan(
        &self,
        id: ExecutionId,
        call: ToolCall,
    ) -> Result<ToolExecutionPlan, ToolExecutorError> {
        self.inner.plan(id, call)
    }

    fn execute(&self, plan: ToolExecutionPlan) -> ToolExecutionFuture<'_> {
        Box::pin(async move {
            let outcome = self.inner.execute(plan).await;
            self.after_search.fetch_add(1, Ordering::SeqCst);
            outcome
        })
    }
}

#[tokio::test]
async fn state_diff_tool_activation_keeps_prior_definitions_and_completed_reasoning() {
    for change in [
        "additive",
        "redefined",
        "removed",
        "expiry_redefined",
        "expiry_removed",
        "expiry_deferred",
    ] {
        let root = TemporaryDirectory::new("activation-contract");
        let tools = ChangedCatalog {
            inner: ToolRegistry::for_project(root.path()),
            after_search: AtomicUsize::new(0),
            change,
        };
        let mut warm = completed_response("warm", "warm-response");
        warm.insert(
            0,
            Ok(ModelStreamEvent::Event(ModelEvent::ReasoningItem {
                text: "KEEP_WARM_REASONING".into(),
            })),
        );
        let mut search = completed_tool_call_response(
            ToolCall::new(
                ToolCallId::new("find-reader"),
                ToolName::new("tool_search"),
                json!({"query":"read_output"}),
            ),
            "search-response",
        );
        search.insert(
            0,
            Ok(ModelStreamEvent::Event(ModelEvent::ReasoningItem {
                text: "KEEP_SEARCH_REASONING".into(),
            })),
        );
        let provider = ScriptedProvider::new(
            ModelCapabilities {
                context_window: 1_048_576,
                ..capabilities(false)
            },
            [
                warm,
                search,
                completed_response("done", "done-response"),
                completed_response("next", "next-response"),
            ],
        );
        let mut agent = AgentLoop::new(&provider, &tools).with_world_state(world_state("before"));
        agent
            .run_turn("warm up", AgentCancellation::new())
            .await
            .unwrap();
        agent.set_world_state(world_state("after"));
        let result = agent
            .run_turn("discover output reader", AgentCancellation::new())
            .await;
        let requests = provider.requests();
        assert!(requests[1].input().content_items.iter().any(|i| matches!(i, ContentItem::Text { text, .. } if text.contains("research_world_state_diff"))));
        if change == "additive" || change.starts_with("expiry_") {
            assert!(result.is_ok(), "{result:?}");
            assert_eq!(requests.len(), 3);
            for text in ["KEEP_WARM_REASONING", "KEEP_SEARCH_REASONING"] {
                assert!(
                    requests[2].input().content_items.iter().any(
                        |i| matches!(i, ContentItem::Reasoning { text: value } if value == text)
                    )
                );
            }
            assert!(!requests[2].is_incremental());
            assert_eq!(
                agent.last_baseline_assessment(),
                Some(&BaselineAssessment::FullRequest {
                    reason: BaselineInvalidationReason::ToolsChanged
                })
            );
            if change.starts_with("expiry_") {
                tools.after_search.store(2, Ordering::SeqCst);
            }
            let next = agent
                .run_turn(
                    "next turn with activation expired",
                    AgentCancellation::new(),
                )
                .await;
            if change == "additive" {
                assert!(next.is_ok(), "{next:?}");
                let next_requests = provider.requests();
                assert_eq!(next_requests.len(), 4);
                assert!(
                    !next_requests[3]
                        .input()
                        .tool_specs
                        .iter()
                        .any(|s| s.name.as_str() == "read_output")
                );
                assert!(next_requests[3].input().content_items.iter().any(
                    |i| matches!(i,ContentItem::Reasoning {text} if text == "KEEP_SEARCH_REASONING")
                ));
            } else {
                assert!(next.unwrap_err().to_string().contains("ToolsChanged"));
                assert_eq!(
                    provider.requests().len(),
                    3,
                    "no sample after unrelated catalog change"
                );
            }
        } else {
            let error = result.unwrap_err();
            assert!(
                error.to_string().contains("ToolsChanged"),
                "{change}: {error}"
            );
            assert_eq!(
                requests.len(),
                2,
                "no sample after incompatible definition change"
            );
        }
        assert_eq!(
            tools.after_search.load(Ordering::SeqCst),
            if change.starts_with("expiry_") { 2 } else { 1 }
        );
        assert_eq!(agent.history().ensure_all_tool_calls_paired(), Ok(()));
    }
}
