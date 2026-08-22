use serde_json::{Value, json};

pub(super) fn read_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "path": memory_path_schema(),
            "event_id": {"type": "string", "minLength": 1},
            "occurred_at": {"type": "string", "minLength": 1}
        },
        "required": ["path", "event_id", "occurred_at"],
        "additionalProperties": false
    })
}

pub(super) fn write_input_schema() -> Value {
    let max_core_characters = crate::adapters::project_memory::MAX_CORE_MEMORY_BYTES;
    json!({
        "type": "object",
        "properties": {
            "path": memory_path_schema(),
            "content": {"type": "string"},
            "expected_hash": {"type": ["string", "null"]},
            "event_id": {"type": "string", "minLength": 1},
            "occurred_at": {"type": "string", "minLength": 1}
        },
        "required": ["path", "content", "expected_hash", "event_id", "occurred_at"],
        "allOf": [{
            "if": {
                "properties": {"path": {"const": "core.md"}},
                "required": ["path"]
            },
            "then": {
                "properties": {
                    "content": {
                        "maxLength": max_core_characters,
                        "description": "core.md is limited to 65536 UTF-8 bytes; non-ASCII content may reach the byte limit before this character limit"
                    }
                }
            }
        }],
        "additionalProperties": false
    })
}

pub(super) fn output_schema() -> Value {
    json!({"type": "object"})
}

fn memory_path_schema() -> Value {
    json!({
        "type": "string",
        "oneOf": [
            {"enum": ["core.md", "decisions.md", "failed-paths.md"]},
            {"pattern": "^notes/[A-Za-z0-9._-]+\\.md$"}
        ]
    })
}
