use cyber_agent::domain::tools::{ToolCall, ToolCallId, ToolName};
use serde_json::{Value, json};

pub fn call() -> ToolCall {
    ToolCall::new(
        ToolCallId::new("batch"),
        ToolName::new("run_commands"),
        json!({"mode":"serial","commands":[{"argv":["true"]},{"argv":["true"]}]}),
    )
}

pub fn payload() -> Value {
    json!({"schema_version":1,"execution_id":"execution:batch","call_id":"batch",
        "tool_name":"run_commands","plan_fingerprint":"a".repeat(64),"child_count":2,"index":0,
        "status":"succeeded","result":{"cwd":".","exit_code":0,"stdout":"KNOWN_CHILD","stderr":"","truncated":false},
        "output_captures":[{"stream":"stdout","child_index":0,"blob_id":"b".repeat(64),
            "observed_bytes":11,"retained_bytes":11,"eof":true,"complete":true,"read_error":null}]})
}
