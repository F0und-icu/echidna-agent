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
    json!({"type": "object"})
}
