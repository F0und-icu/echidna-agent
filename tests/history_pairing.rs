use cyber_agent::domain::history::{ConversationHistory, HistoryError, normalize_content_items};
use cyber_agent::domain::model::ContentItem;
use cyber_agent::domain::tools::{ToolCall, ToolCallId, ToolName, ToolResult, ToolResultStatus};
use serde_json::json;

#[test]
fn normalization_closes_open_calls_before_later_user_text_or_image() {
    use cyber_agent::domain::artifacts::ArtifactId;
    use cyber_agent::domain::model::MessageRole;

    for boundary in [
        ContentItem::user_text("resume without repeating the unknown execution"),
        ContentItem::ImageArtifact {
            role: MessageRole::User,
            artifact_id: ArtifactId::new("resume-image"),
        },
    ] {
        let input = vec![
            ContentItem::user_text("inspect"),
            ContentItem::ToolCall {
                call: call("open", "run_command"),
            },
            boundary.clone(),
        ];
        let original = input.clone();
        let normalized = normalize_content_items(&input).unwrap();
        assert_eq!(normalized.len(), 4);
        assert!(matches!(
            &normalized[2],
            ContentItem::ToolResult { result }
                if result.call_id.as_str() == "open"
                    && result.status == ToolResultStatus::Cancelled
                    && result.output["execution_status"] == "unknown"
                    && result.output["guidance"] == "do_not_assume_no_effect_or_auto_rerun"
        ));
        assert_eq!(normalized[3], boundary);
        assert_eq!(normalize_content_items(&normalized).unwrap(), normalized);
        assert_eq!(input, original);
    }
}

#[test]
fn normalization_preserves_parallel_partial_results_and_repairs_each_user_group() {
    let prefix = vec![
        ContentItem::user_text("inspect"),
        ContentItem::ToolCall {
            call: call("a", "read_file"),
        },
        ContentItem::assistant_text("also inspect the other file"),
        ContentItem::ToolCall {
            call: call("b", "read_file"),
        },
        ContentItem::ToolCall {
            call: call("c", "read_file"),
        },
        ContentItem::ToolResult {
            result: result("c", "read_file", ToolResultStatus::Failed),
        },
    ];
    let mut input = prefix.clone();
    input.extend([
        ContentItem::user_text("continue"),
        ContentItem::ToolCall {
            call: call("d", "list_files"),
        },
        ContentItem::user_text("continue again"),
    ]);
    let normalized = normalize_content_items(&input).unwrap();
    assert_eq!(&normalized[..prefix.len()], &prefix);
    let ids: Vec<_> = normalized
        .iter()
        .filter_map(|item| match item {
            ContentItem::ToolResult { result } => Some(result.call_id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(ids, ["c", "a", "b", "d"]);
    assert_eq!(normalized[8], ContentItem::user_text("continue"));
    assert_eq!(normalized[11], ContentItem::user_text("continue again"));
    assert_eq!(normalize_content_items(&normalized).unwrap(), normalized);
}

#[test]
fn normalization_rejects_late_real_results_without_reordering_or_replacing_them() {
    use cyber_agent::domain::artifacts::ArtifactId;
    use cyber_agent::domain::model::MessageRole;

    for boundary in [
        ContentItem::user_text("later user"),
        ContentItem::ImageArtifact {
            role: MessageRole::User,
            artifact_id: ArtifactId::new("image"),
        },
    ] {
        let input = vec![
            ContentItem::ToolCall {
                call: call("late", "read_file"),
            },
            boundary,
            ContentItem::ToolResult {
                result: result("late", "read_file", ToolResultStatus::Succeeded),
            },
        ];
        assert_eq!(
            normalize_content_items(&input),
            Err(HistoryError::ToolResultAfterUserMessage {
                call_id: ToolCallId::new("late"),
            })
        );
    }
}

#[test]
fn normalization_retains_legal_out_of_order_parallel_results_and_assistant_items() {
    let input = vec![
        ContentItem::user_text("inspect"),
        ContentItem::ToolCall {
            call: call("a", "read_file"),
        },
        ContentItem::ToolCall {
            call: call("b", "read_file"),
        },
        ContentItem::assistant_text("inspecting both"),
        ContentItem::ToolResult {
            result: result("b", "read_file", ToolResultStatus::Succeeded),
        },
        ContentItem::ToolResult {
            result: result("a", "read_file", ToolResultStatus::Succeeded),
        },
        ContentItem::user_text("next turn"),
    ];
    assert_eq!(normalize_content_items(&input).unwrap(), input);
}

fn call(id: &str, name: &str) -> ToolCall {
    ToolCall::new(
        ToolCallId::new(id),
        ToolName::new(name),
        json!({"path": "Cargo.toml"}),
    )
}

fn result(id: &str, name: &str, status: ToolResultStatus) -> ToolResult {
    ToolResult {
        call_id: ToolCallId::new(id),
        tool_name: ToolName::new(name),
        status,
        output: json!({"status": format!("{status:?}")}),
    }
}

#[test]
fn history_rejects_missing_and_duplicate_tool_calls() {
    let mut history = ConversationHistory::default();
    assert_eq!(
        history.append_tool_result(result(
            "call-missing",
            "read_file",
            ToolResultStatus::Failed
        )),
        Err(HistoryError::MissingToolCall {
            call_id: ToolCallId::new("call-missing")
        })
    );

    history
        .append_tool_call(call("call-1", "read_file"))
        .unwrap();
    assert_eq!(
        history.append_tool_call(call("call-1", "read_file")),
        Err(HistoryError::DuplicateToolCall {
            call_id: ToolCallId::new("call-1")
        })
    );
    assert_eq!(
        history.ensure_all_tool_calls_paired(),
        Err(HistoryError::MissingToolResult {
            call_id: ToolCallId::new("call-1")
        })
    );
}

#[test]
fn history_rejects_name_mismatch_and_duplicate_results_without_losing_pair_state() {
    let mut history = ConversationHistory::default();
    history
        .append_tool_call(call("call-1", "read_file"))
        .unwrap();

    assert_eq!(
        history.append_tool_result(result("call-1", "list_files", ToolResultStatus::Succeeded)),
        Err(HistoryError::ToolNameMismatch {
            call_id: ToolCallId::new("call-1"),
            expected: ToolName::new("read_file"),
            received: ToolName::new("list_files"),
        })
    );
    assert!(matches!(
        history.ensure_all_tool_calls_paired(),
        Err(HistoryError::MissingToolResult { .. })
    ));

    let valid = result("call-1", "read_file", ToolResultStatus::Succeeded);
    history.append_tool_result(valid.clone()).unwrap();
    assert_eq!(
        history.append_tool_result(valid),
        Err(HistoryError::DuplicateToolResult {
            call_id: ToolCallId::new("call-1")
        })
    );
    assert_eq!(history.ensure_all_tool_calls_paired(), Ok(()));
}

#[test]
fn failed_and_cancelled_results_are_complete_pairs() {
    for (id, status) in [
        ("failed", ToolResultStatus::Failed),
        ("cancelled", ToolResultStatus::Cancelled),
    ] {
        let mut history = ConversationHistory::default();
        history.append_tool_call(call(id, "run_command")).unwrap();
        history
            .append_tool_result(result(id, "run_command", status))
            .unwrap();
        assert_eq!(history.ensure_all_tool_calls_paired(), Ok(()));
    }
}

#[test]
fn provider_reasoning_cannot_be_normalized_as_conversation_history() {
    assert_eq!(
        normalize_content_items(&[
            ContentItem::user_text("inspect the target"),
            ContentItem::Reasoning {
                text: "provider continuation only".to_owned(),
            },
        ]),
        Err(HistoryError::ProviderContinuationItem)
    );
}

#[test]
fn normalization_preserves_complete_pairs_drops_orphans_and_repairs_open_calls_in_call_order() {
    let completed_call = call("complete", "read_file");
    let completed_result = result("complete", "read_file", ToolResultStatus::Succeeded);
    let first_open_call = call("first-open", "list_files");
    let second_open_call = call("second-open", "search_files");
    let normalized = normalize_content_items(&[
        ContentItem::user_text("inspect the workspace"),
        ContentItem::ToolCall {
            call: completed_call.clone(),
        },
        ContentItem::ToolResult {
            result: completed_result.clone(),
        },
        ContentItem::ToolResult {
            result: result("orphan", "read_file", ToolResultStatus::Failed),
        },
        ContentItem::ToolCall {
            call: first_open_call.clone(),
        },
        ContentItem::assistant_text("the remaining calls were interrupted"),
        ContentItem::ToolCall {
            call: second_open_call.clone(),
        },
    ])
    .unwrap();

    assert_eq!(normalized.len(), 8);
    assert_eq!(
        normalized[1],
        ContentItem::ToolCall {
            call: completed_call
        }
    );
    assert_eq!(
        normalized[2],
        ContentItem::ToolResult {
            result: completed_result
        }
    );
    assert!(!normalized.iter().any(|item| matches!(
        item,
        ContentItem::ToolResult { result } if result.call_id.as_str() == "orphan"
    )));
    assert_eq!(
        normalized[6],
        ContentItem::ToolResult {
            result: ToolResult {
                call_id: first_open_call.call_id,
                tool_name: first_open_call.name,
                status: ToolResultStatus::Cancelled,
                output: json!({
                    "error": "aborted",
                    "execution_status": "unknown",
                    "guidance": "do_not_assume_no_effect_or_auto_rerun"
                }),
            }
        }
    );
    assert_eq!(
        normalized[7],
        ContentItem::ToolResult {
            result: ToolResult {
                call_id: second_open_call.call_id,
                tool_name: second_open_call.name,
                status: ToolResultStatus::Cancelled,
                output: json!({
                    "error": "aborted",
                    "execution_status": "unknown",
                    "guidance": "do_not_assume_no_effect_or_auto_rerun"
                }),
            }
        }
    );
}

#[test]
fn normalization_rejects_duplicate_calls_results_and_name_mismatches() {
    let duplicate_call = || {
        normalize_content_items(&[
            ContentItem::ToolCall {
                call: call("duplicate", "read_file"),
            },
            ContentItem::ToolCall {
                call: call("duplicate", "read_file"),
            },
        ])
    };
    assert!(matches!(
        duplicate_call(),
        Err(HistoryError::DuplicateToolCall { call_id }) if call_id.as_str() == "duplicate"
    ));

    assert!(matches!(
        normalize_content_items(&[
            ContentItem::ToolCall {
                call: call("duplicate-result", "read_file"),
            },
            ContentItem::ToolResult {
                result: result("duplicate-result", "read_file", ToolResultStatus::Succeeded),
            },
            ContentItem::ToolResult {
                result: result("duplicate-result", "read_file", ToolResultStatus::Succeeded),
            },
        ]),
        Err(HistoryError::DuplicateToolResult { call_id }) if call_id.as_str() == "duplicate-result"
    ));

    assert!(matches!(
        normalize_content_items(&[
            ContentItem::ToolCall {
                call: call("mismatch", "read_file"),
            },
            ContentItem::ToolResult {
                result: result("mismatch", "list_files", ToolResultStatus::Succeeded),
            },
        ]),
        Err(HistoryError::ToolNameMismatch { call_id, .. }) if call_id.as_str() == "mismatch"
    ));
}
