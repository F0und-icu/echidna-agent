use serde_json::{Value, json};

pub(super) fn read_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "path": memory_path_schema(),
            "event_id": {"type": "string", "minLength": 1}
        },
        "required": ["path", "event_id"],
        "additionalProperties": false
    })
}

pub(super) fn write_input_schema(max_core_characters: usize) -> Value {
    json!({
        "type": "object",
        "properties": {
            "path": memory_path_schema(),
            "content": {"type": "string"},
            "expected_hash": {"type": ["string", "null"]},
            "event_id": {"type": "string", "minLength": 1}
        },
        "required": ["path", "content", "expected_hash", "event_id"],
        "allOf": [{
            "if": {
                "properties": {"path": {"const": "core.md"}},
                "required": ["path"]
            },
            "then": {
                "properties": {
                    "content": {
                        "maxLength": max_core_characters,
                        "description": format!(
                            "core.md is limited to {max_core_characters} UTF-8 bytes; non-ASCII content may reach the byte limit before this character limit"
                        )
                    }
                }
            }
        }],
        "additionalProperties": false
    })
}

pub(super) fn output_schema() -> Value {
    json!({
        "oneOf": [
            {
                "type": "object", "properties": {
                    "status": {"type": "string", "const": "ok"}, "path": {"type": "string", "minLength": 1},
                    "content": {"type": "string"}, "hash": {"type": ["string", "null"]},
                    "external_change_detected": {"type": "boolean"}, "stream_event_count": {"type": "integer", "minimum": 0}
                }, "required": ["status", "path", "content", "hash", "external_change_detected", "stream_event_count"], "additionalProperties": false
            },
            {
                "type": "object", "properties": {
                    "status": {"type": "string", "enum": ["recorded", "unchanged"]}, "event_appended": {"type": "boolean"},
                    "event_type": {"type": "string", "const": "memory.changed"}, "event_id": {"type": "string", "minLength": 1},
                    "path": {"type": "string", "minLength": 1}, "old_hash": {"type": ["string", "null"]},
                    "new_hash": {"type": ["string", "null"]}, "diff_blob_id": {"type": ["string", "null"]},
                    "hash": {"type": ["string", "null"]}, "stream_event_count": {"type": "integer", "minimum": 0}
                }, "required": ["status", "event_appended", "path", "stream_event_count"], "additionalProperties": false
            }
        ]
    })
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
