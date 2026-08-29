use serde_json::{Value, json};

pub(super) fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "event_id": {"type": "string", "minLength": 1},
            "occurred_at": {"type": "string", "minLength": 1},
            "artifact_id": {"type": "string", "minLength": 1},
            "source_path": {"type": "string", "minLength": 1},
            "media_type": {"type": "string", "minLength": 1},
            "target_snapshot_id": {"type": "string", "minLength": 1},
            "toolchain_fingerprint": {"type": "string", "minLength": 1},
            "parent_artifact_ids": {
                "type": "array",
                "items": {"type": "string", "minLength": 1}
            },
            "transform": {"type": "string", "minLength": 1}
        },
        "required": [
            "event_id", "occurred_at", "artifact_id", "source_path", "media_type",
            "target_snapshot_id", "toolchain_fingerprint"
        ],
        "additionalProperties": false
    })
}

pub(super) fn output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "status": {"type": "string", "const": "recorded"},
            "event_appended": {"type": "boolean", "const": true},
            "event_type": {"type": "string", "enum": ["artifact.created", "artifact.derived"]},
            "event_id": {"type": "string", "minLength": 1},
            "blob": {"type": "object", "properties": {
                "blob_id": {"type": "string", "pattern": "^[0-9a-f]{64}$"},
                "size_bytes": {"type": "integer", "minimum": 0},
                "already_present": {"type": "boolean"}
            }, "required": ["blob_id", "size_bytes", "already_present"], "additionalProperties": false},
            "artifact": {"type": "object", "properties": {
                "id": {"type": "string", "minLength": 1}, "blob_id": {"type": "string", "pattern": "^[0-9a-f]{64}$"},
                "media_type": {"type": "string", "minLength": 1}, "size_bytes": {"type": "integer", "minimum": 0},
                "source_path": {"type": "string", "minLength": 1}, "created_by_event_id": {"type": "string", "minLength": 1},
                "produced_by_tool_call_id": {"type": "string", "minLength": 1}, "target_snapshot_id": {"type": "string", "minLength": 1},
                "toolchain_fingerprint": {"type": "string", "minLength": 1}, "parent_artifact_ids": {"type": "array", "items": {"type": "string", "minLength": 1}},
                "transform": {"type": ["string", "null"]}
            }, "required": ["id", "blob_id", "media_type", "size_bytes", "source_path", "created_by_event_id", "produced_by_tool_call_id", "target_snapshot_id", "toolchain_fingerprint", "parent_artifact_ids", "transform"], "additionalProperties": false},
            "research_event_count": {"type": "integer", "minimum": 0}
        },
        "required": ["status", "event_appended", "event_type", "event_id", "blob", "artifact", "research_event_count"],
        "additionalProperties": false
    })
}
