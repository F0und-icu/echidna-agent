//! Stable model-facing schemas for `inspect_environment`.

use serde_json::{Value, json};

use super::SCHEMA_VERSION;

pub(super) fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {},
        "additionalProperties": false
    })
}

pub(super) fn output_schema() -> Value {
    let probe = json!({
        "type": "object",
        "properties": {
            "available": {"type": "boolean"},
            "status": {"type": "string", "enum": ["available", "missing", "version_failed", "probe_failed"]},
            "version": {"type": ["string", "null"]}
        },
        "required": ["available", "status", "version"],
        "additionalProperties": false
    });
    json!({
        "type": "object",
        "properties": {
            "schema_version": {"type": "integer", "const": SCHEMA_VERSION},
            "os": {"type": "object", "properties": {
                "family": {"type": "string", "minLength": 1},
                "name": {"type": "string", "minLength": 1},
                "arch": {"type": "string", "minLength": 1},
                "kernel": probe.clone()
            }, "required": ["family", "name", "arch", "kernel"], "additionalProperties": false},
            "runtime": {"type": "object", "properties": {
                "name": {"type": "string", "const": "cyber-agent"},
                "version": {"type": "string", "minLength": 1},
                "arch": {"type": "string", "minLength": 1}
            }, "required": ["name", "version", "arch"], "additionalProperties": false},
            "git": probe,
            "proxy": {"type": "object", "properties": {
                "http_configured": {"type": "boolean"},
                "https_configured": {"type": "boolean"},
                "all_configured": {"type": "boolean"},
                "no_proxy_configured": {"type": "boolean"}
            }, "required": ["http_configured", "https_configured", "all_configured", "no_proxy_configured"], "additionalProperties": false},
            "toolchain": {"type": "array", "minItems": 5, "maxItems": 5, "items": {
                "type": "object", "properties": {
                    "name": {"type": "string", "enum": ["cargo", "java", "node", "python3", "rustc"]},
                    "available": {"type": "boolean"},
                    "status": {"type": "string", "enum": ["available", "missing", "version_failed", "probe_failed"]},
                    "version": {"type": ["string", "null"]}
                }, "required": ["name", "available", "status", "version"], "additionalProperties": false
            }},
            "connected_devices": {"type": "array", "minItems": 2, "maxItems": 2, "items": {
                "type": "object", "properties": {
                    "provider": {"type": "string", "enum": ["adb", "xcrun"]},
                    "available": {"type": "boolean"},
                    "status": {"type": "string", "enum": ["available", "missing", "version_failed", "probe_failed"]},
                    "version": {"type": ["string", "null"]},
                    "count": {"type": "integer", "minimum": 0}
                }, "required": ["provider", "available", "status", "version", "count"], "additionalProperties": false
            }},
            "fingerprint": {"type": "string", "pattern": "^[0-9a-f]{64}$"}
        },
        "required": ["schema_version", "os", "runtime", "git", "proxy", "toolchain", "connected_devices", "fingerprint"],
        "additionalProperties": false
    })
}
