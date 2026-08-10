use cyber_agent::domain::history::{ConversationHistory, HistoryError};
use cyber_agent::domain::tools::{ToolCall, ToolCallId, ToolName, ToolResult, ToolResultStatus};
use serde_json::json;

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
