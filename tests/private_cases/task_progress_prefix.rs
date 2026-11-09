//! The explicit progress reader uses the bounded original-JSON prefix writer.
use crate::app::context_read::{SourceRangeWriter, history_navigation_excerpt};
use crate::domain::model::ContentItem;
use crate::domain::tools::{ToolCallId, ToolName, ToolResult, ToolResultStatus};

#[test]
fn large_tool_result_prefix_limits_written_bytes_and_preserves_json() {
    let item = ContentItem::ToolResult {
        result: ToolResult {
            call_id: ToolCallId::new("large-progress-result"),
            tool_name: ToolName::new("fixture"),
            status: ToolResultStatus::Succeeded,
            output: serde_json::json!({"payload": format!("{}尾", "中\\\"".repeat(700_000))}),
        },
    };
    let original = serde_json::to_string(&item).unwrap();
    assert!(original.len() > 2 * 1024 * 1024);
    let mut writer = SourceRangeWriter {
        hash: blake3::Hasher::new(),
        offset: 0,
        end: 1024,
        position: 0,
        bytes: Vec::new(),
        stop_at_end: true,
        stopped: false,
    };
    assert!(serde_json::to_writer(&mut writer, &item).is_err());
    assert!(writer.stopped);
    assert_eq!(writer.position, 1024);
    assert_eq!(writer.bytes.len(), 1024);
    let (prefix, truncated) = history_navigation_excerpt(&item).unwrap();
    let mut end = 1024;
    while !original.is_char_boundary(end) {
        end -= 1;
    }
    assert_eq!(prefix, original[..end]);
    assert!(truncated);
}
