//! Exact model-visible schemas for the bounded HTTP Tool.

use serde_json::{Value, json};

use super::request::{
    MAX_BASE64_BODY_CHARS, MAX_CAPTURE_BYTES, MAX_HEADER_NAME_BYTES, MAX_HEADER_VALUE_BYTES,
    MAX_HEADERS, MAX_PREVIEW_BYTES, MAX_QUERY_NAME_BYTES, MAX_QUERY_PARAMETERS,
    MAX_QUERY_VALUE_BYTES, MAX_REDIRECTS, MAX_REQUEST_BODY_BYTES, MAX_TIMEOUT_MS, MAX_URL_BYTES,
};

pub(super) fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "method": {"type": "string", "enum": ["get", "head", "post", "put", "patch", "delete", "options", "trace"]},
            "url": {"type": "string", "minLength": 1, "maxLength": MAX_URL_BYTES},
            "headers": {"type": "object", "propertyNames": {"maxLength": MAX_HEADER_NAME_BYTES}, "maxProperties": MAX_HEADERS, "additionalProperties": {"type": "string", "maxLength": MAX_HEADER_VALUE_BYTES}},
            "query": {"type": "array", "maxItems": MAX_QUERY_PARAMETERS, "items": {"type": "object", "properties": {"name": {"type": "string", "minLength": 1, "maxLength": MAX_QUERY_NAME_BYTES}, "value": {"type": "string", "maxLength": MAX_QUERY_VALUE_BYTES}}, "required": ["name", "value"], "additionalProperties": false}},
            "body": {"oneOf": [
                {"type": "object", "properties": {"kind": {"const": "text"}, "value": {"type": "string", "maxLength": MAX_REQUEST_BODY_BYTES}}, "required": ["kind", "value"], "additionalProperties": false},
                {"type": "object", "properties": {"kind": {"const": "json"}, "value": {}}, "required": ["kind", "value"], "additionalProperties": false},
                {"type": "object", "properties": {"kind": {"const": "base64"}, "data": {"type": "string", "maxLength": MAX_BASE64_BODY_CHARS}}, "required": ["kind", "data"], "additionalProperties": false},
                {"type": "object", "properties": {"kind": {"const": "artifact_ref"}, "artifact_id": {"type": "string", "minLength": 1}}, "required": ["kind", "artifact_id"], "additionalProperties": false}
            ]},
            "timeout_ms": {"type": "integer", "minimum": 1, "maximum": MAX_TIMEOUT_MS},
            "max_redirects": {"type": "integer", "minimum": 0, "maximum": MAX_REDIRECTS},
            "preview_bytes": {"type": "integer", "minimum": 0, "maximum": MAX_PREVIEW_BYTES},
            "capture_bytes": {"type": "integer", "minimum": 1, "maximum": MAX_CAPTURE_BYTES},
            "artifact": {"type": "object", "properties": {
                "event_id": {"type": "string", "minLength": 1},
                "occurred_at": {"type": "string", "minLength": 1},
                "artifact_id": {"type": "string", "minLength": 1},
                "target_snapshot_id": {"type": "string", "minLength": 1},
                "toolchain_fingerprint": {"type": "string", "minLength": 1}
            }, "required": ["event_id", "occurred_at", "artifact_id", "target_snapshot_id", "toolchain_fingerprint"], "additionalProperties": false}
        },
        "required": ["method", "url", "artifact"],
        "additionalProperties": false
    })
}

pub(super) fn output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "status": {"type": "integer", "minimum": 100, "maximum": 599},
            "final_url": {"type": "string", "minLength": 1},
            "metadata": {"type": "object", "properties": {
                "media_type": {"type": "string", "minLength": 1},
                "content_length": {"type": ["integer", "null"], "minimum": 0},
                "captured_body_bytes": {"type": "integer", "minimum": 0},
                "body_truncated": {"type": "boolean"}
            }, "required": ["media_type", "content_length", "captured_body_bytes", "body_truncated"], "additionalProperties": false},
            "timing": {"type": "object", "properties": {"elapsed_ms": {"type": "integer", "minimum": 0}}, "required": ["elapsed_ms"], "additionalProperties": false},
            "body_preview": {"type": "object", "properties": {
                "encoding": {"type": "string", "enum": ["utf8", "base64"]},
                "data": {"type": "string"},
                "captured_bytes": {"type": "integer", "minimum": 0},
                "truncated": {"type": "boolean"}
            }, "required": ["encoding", "data", "captured_bytes", "truncated"], "additionalProperties": false},
            "body_artifact": {"type": "object", "properties": {
                "artifact_id": {"type": "string", "minLength": 1},
                "blob_id": {"type": "string", "pattern": "^[0-9a-f]{64}$"},
                "size_bytes": {"type": "integer", "minimum": 0},
                "already_present": {"type": "boolean"},
                "event_id": {"type": "string", "minLength": 1},
                "capture_complete": {"type": "boolean"}
            }, "required": ["artifact_id", "blob_id", "size_bytes", "already_present", "event_id", "capture_complete"], "additionalProperties": false}
        },
        "required": ["status", "final_url", "metadata", "timing", "body_preview", "body_artifact"],
        "additionalProperties": false
    })
}
