//! Fixed JSON Schemas for the two Research tools.

use serde_json::{Value, json};

pub(super) fn get_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {},
        "additionalProperties": false
    })
}

pub(super) fn update_input_schema() -> Value {
    json!({
        "type": "object",
        "$defs": {
            "evidence_ref": evidence_ref_schema(),
            "evidence_link": evidence_link_schema(),
            "text": string_schema(),
            "texts": string_array_schema(),
            "links": evidence_links_schema(),
            "merge": merge_ref_schema()
        },
        "properties": {
            "event_id": input_ref("text"),
            "occurred_at": input_ref("text"),
            "operation": {
                "oneOf": [
                    target_snapshot_operation_schema(),
                    hypothesis_operation_schema(),
                    hypothesis_update_operation_schema(),
                    experiment_operation_schema(),
                    experiment_update_operation_schema(),
                    finding_operation_schema(),
                    finding_update_operation_schema(),
                    next_action_operation_schema(),
                    next_action_update_operation_schema(),
                    merge_operation_schema()
                ]
            }
        },
        "required": ["event_id", "occurred_at", "operation"],
        "additionalProperties": false
    })
}

pub(super) fn get_output_schema() -> Value {
    json!({
        "$defs": research_output_defs(),
        "type": "object",
        "properties": {
            "status": {"type": "string", "const": "ok"},
            "state": research_state_schema(),
            "stream_event_count": {"type": "integer", "minimum": 0},
            "evidence_gap_reports": {
                "type": "array",
                "items": {"$ref": "#/$defs/evidence_gap_report"}
            }
        },
        "required": ["status", "state", "stream_event_count", "evidence_gap_reports"],
        "additionalProperties": false
    })
}

pub(super) fn update_output_schema() -> Value {
    json!({
        "$defs": research_output_defs(),
        "oneOf": [
            {
                "type": "object",
                "properties": {
                    "status": {"type": "string", "const": "recorded"},
                    "event_appended": {"type": "boolean", "const": true},
                    "event_type": string_schema(),
                    "event_id": string_schema(),
                    "research_event_count": {"type": "integer", "minimum": 0}
                },
                "required": ["status", "event_appended", "event_type", "event_id", "research_event_count"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": {
                    "status": {"type": "string", "const": "existing"},
                    "event_appended": {"type": "boolean", "const": false},
                    "record": {"$ref": "#/$defs/target_snapshot"},
                    "message": string_schema()
                },
                "required": ["status", "event_appended", "record", "message"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": {
                    "status": {"type": "string", "const": "dedup_conflict"},
                    "event_appended": {"type": "boolean", "const": false},
                    "dedup_key": string_schema(),
                    "existing_records": {"type": "array", "minItems": 1, "items": {
                        "oneOf": [
                            {"$ref": "#/$defs/hypothesis"},
                            {"$ref": "#/$defs/finding"}
                        ]
                    }},
                    "required_resolution": {"type": "array", "items": {
                        "type": "string", "enum": ["keep_independent", "records_merged"]
                    }, "minItems": 2, "maxItems": 2, "uniqueItems": true}
                },
                "required": ["status", "event_appended", "dedup_key", "existing_records", "required_resolution"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": {
                    "status": {"type": "string", "const": "action_conflict"},
                    "event_appended": {"type": "boolean", "const": false},
                    "action_fingerprint": string_schema(),
                    "existing_records": {"type": "array", "minItems": 1, "items": {"$ref": "#/$defs/experiment"}},
                    "required_resolution": {"type": "array", "items": {"const": "rerun_reason"}, "minItems": 1, "maxItems": 1}
                },
                "required": ["status", "event_appended", "action_fingerprint", "existing_records", "required_resolution"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": {
                    "status": {"type": "string", "const": "transition_rejected"},
                    "event_appended": {"type": "boolean", "const": false}
                },
                "required": ["status", "event_appended"],
                "additionalProperties": false
            }
        ]
    })
}

fn research_output_defs() -> Value {
    json!({
        "evidence_ref": evidence_ref_schema(),
        "evidence_link": evidence_link_schema(),
        "evidence_gap_report": evidence_gap_report_schema(),
        "target_snapshot": target_snapshot_record_schema(),
        "hypothesis": hypothesis_record_schema(),
        "experiment": experiment_record_schema(),
        "capsule": capsule_record_schema(),
        "finding": finding_record_schema(),
        "next_action": next_action_record_schema(),
        "artifact": artifact_record_schema()
    })
}

fn research_state_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "target_snapshots": record_map_schema("target_snapshot"),
            "hypotheses": record_map_schema("hypothesis"),
            "experiments": record_map_schema("experiment"),
            "capsules": record_map_schema("capsule"),
            "findings": record_map_schema("finding"),
            "next_actions": record_map_schema("next_action"),
            "artifacts": {
                "type": "object",
                "properties": {"records": record_map_schema("artifact")},
                "required": ["records"],
                "additionalProperties": false
            }
        },
        "required": ["target_snapshots", "hypotheses", "experiments", "capsules", "findings", "next_actions", "artifacts"],
        "additionalProperties": false
    })
}

fn record_map_schema(definition: &str) -> Value {
    json!({
        "type": "object",
        "additionalProperties": {"$ref": format!("#/$defs/{definition}")}
    })
}

fn target_snapshot_record_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "id": string_schema(), "target_id": string_schema(),
            "kind": {"type": "string", "enum": ["source", "web", "mobile", "binary", "firmware"]},
            "locator": string_schema(), "version": string_schema(), "build_id": string_schema(),
            "target_artifact_ids": string_array_schema(),
            "environment_fingerprint": string_schema(), "device_fingerprint": string_schema(),
            "snapshot_fingerprint": {"type": "string", "pattern": "^[0-9a-f]{64}$"},
            "captured_at": string_schema(), "notes": {"type": "string"}
        },
        "required": ["id", "target_id", "kind", "locator", "version", "build_id", "target_artifact_ids", "environment_fingerprint", "device_fingerprint", "snapshot_fingerprint", "captured_at", "notes"],
        "additionalProperties": false
    })
}

fn hypothesis_record_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "id": string_schema(), "title": string_schema(), "target_id": string_schema(),
            "attack_surface": string_schema(), "vulnerability_class": string_schema(),
            "entry_point": string_schema(), "sensitive_sink": string_schema(),
            "dedup_key": {"type": "string", "pattern": "^[0-9a-f]{64}$"},
            "rationale": string_schema(),
            "status": {"type": "string", "enum": ["active", "testing", "supported", "rejected", "blocked"]},
            "priority": {"type": "integer"},
            "evidence_links": evidence_links_schema(),
            "last_action": nullable_string_schema(), "next_action": nullable_string_schema(),
            "merged_into": nullable_merge_ref_schema()
        },
        "required": ["id", "title", "target_id", "attack_surface", "vulnerability_class", "entry_point", "sensitive_sink", "dedup_key", "rationale", "status", "priority", "evidence_links", "last_action", "next_action", "merged_into"],
        "additionalProperties": false
    })
}

fn experiment_record_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "id": string_schema(), "hypothesis_id": string_schema(),
            "target_snapshot_id": string_schema(), "capsule_id": string_schema(),
            "goal": string_schema(),
            "action_fingerprint": {"type": "string", "pattern": "^[0-9a-f]{64}$"},
            "result": {"type": "string", "enum": ["pending", "positive", "negative", "inconclusive"]},
            "artifact_refs": string_array_schema(), "notes": {"type": "string"}
        },
        "required": ["id", "hypothesis_id", "target_snapshot_id", "capsule_id", "goal", "action_fingerprint", "result", "artifact_refs", "notes"],
        "additionalProperties": false
    })
}

fn capsule_record_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "id": string_schema(), "experiment_id": string_schema(), "target_snapshot_id": string_schema(),
            "normalized_action": string_schema(), "normalized_arguments": {}, "tool_version": string_schema(),
            "execution_ids": string_array_schema(), "input_artifact_ids": string_array_schema(),
            "environment_delta": {"type": "string"}, "account_prerequisites": string_array_schema(),
            "toolchain_fingerprint": string_schema(), "execution_receipt_ids": string_array_schema(),
            "output_artifact_ids": string_array_schema()
        },
        "required": ["id", "experiment_id", "target_snapshot_id", "normalized_action", "normalized_arguments", "tool_version", "execution_ids", "input_artifact_ids", "environment_delta", "account_prerequisites", "toolchain_fingerprint", "execution_receipt_ids", "output_artifact_ids"],
        "additionalProperties": false
    })
}

fn finding_record_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "id": string_schema(), "title": string_schema(),
            "status": {"type": "string", "enum": ["candidate", "reproduced", "confirmed", "discarded"]},
            "target_id": string_schema(), "target_version": string_schema(),
            "vulnerability_class": string_schema(), "entry_point": string_schema(),
            "sensitive_sink": string_schema(), "dedup_key": {"type": "string", "pattern": "^[0-9a-f]{64}$"},
            "target_snapshot_ids": string_array_schema(), "prerequisites": string_array_schema(),
            "impact": string_schema(), "evidence_links": evidence_links_schema(),
            "merged_into": nullable_merge_ref_schema()
        },
        "required": ["id", "title", "status", "target_id", "target_version", "vulnerability_class", "entry_point", "sensitive_sink", "dedup_key", "target_snapshot_ids", "prerequisites", "impact", "evidence_links", "merged_into"],
        "additionalProperties": false
    })
}

fn next_action_record_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "id": string_schema(), "research_id": research_ref_schema(),
            "priority": {"type": "integer"}, "action": string_schema(),
            "status": {"type": "string", "enum": ["pending", "running", "done", "blocked"]}
        },
        "required": ["id", "research_id", "priority", "action", "status"],
        "additionalProperties": false
    })
}

fn artifact_record_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "id": string_schema(), "blob_id": {"type": "string", "pattern": "^[0-9a-f]{64}$"},
            "media_type": string_schema(), "size_bytes": {"type": "integer", "minimum": 0},
            "source_path": string_schema(), "created_by_event_id": string_schema(),
            "produced_by_tool_call_id": string_schema(), "target_snapshot_id": string_schema(),
            "toolchain_fingerprint": string_schema(), "parent_artifact_ids": string_array_schema(),
            "transform": nullable_string_schema()
        },
        "required": ["id", "blob_id", "media_type", "size_bytes", "source_path", "created_by_event_id", "produced_by_tool_call_id", "target_snapshot_id", "toolchain_fingerprint", "parent_artifact_ids", "transform"],
        "additionalProperties": false
    })
}

fn nullable_string_schema() -> Value {
    json!({"type": ["string", "null"], "minLength": 1})
}

fn nullable_merge_ref_schema() -> Value {
    json!({"oneOf": [merge_ref_schema(), {"type": "null"}]})
}

// Input-only references reuse the original type builders in the root $defs.
// Outputs retain their original representation; no runtime schema optimizer.
fn input_ref(name: &str) -> Value {
    json!({"$ref": format!("#/$defs/{name}")})
}

fn target_snapshot_operation_schema() -> Value {
    tagged_operation(
        "record_target_snapshot",
        json!({
            "type": "object",
            "properties": {
                "id": input_ref("text"), "target_id": input_ref("text"),
                "kind": {"type": "string", "enum": ["source", "web", "mobile", "binary", "firmware"]},
                "locator": input_ref("text"), "version": input_ref("text"), "build_id": input_ref("text"),
                "target_artifact_ids": input_ref("texts"),
                "environment_fingerprint": input_ref("text"), "device_fingerprint": input_ref("text"),
                "captured_at": input_ref("text"), "notes": {"type": "string"}
            },
            "required": ["id", "target_id", "kind", "locator", "version", "build_id", "environment_fingerprint", "device_fingerprint", "captured_at"],
            "additionalProperties": false
        }),
    )
}

fn hypothesis_operation_schema() -> Value {
    tagged_operation_with_optional_fields(
        "record_hypothesis",
        json!({
            "type": "object",
            "properties": {
                "id": input_ref("text"), "title": input_ref("text"), "target_id": input_ref("text"),
                "attack_surface": input_ref("text"), "vulnerability_class": input_ref("text"),
                "entry_point": input_ref("text"), "sensitive_sink": input_ref("text"),
                "rationale": input_ref("text"),
                "status": {"type": "string", "enum": ["active", "testing", "supported", "rejected", "blocked"]},
                "priority": {"type": "integer"}, "evidence_links": input_ref("links"),
                "last_action": input_ref("text"),
                "next_action": input_ref("text")
            },
            "required": ["id", "title", "target_id", "attack_surface", "vulnerability_class", "entry_point", "sensitive_sink", "rationale", "status", "priority"],
            "additionalProperties": false
        }),
        json!({"keep_independent": {"type": "boolean"}}),
    )
}

fn hypothesis_update_operation_schema() -> Value {
    tagged_update_operation(
        "update_hypothesis",
        json!({
            "type": "object",
            "properties": {
                "id": input_ref("text"),
                "status": {"type": "string", "enum": ["active", "testing", "supported", "rejected", "blocked"]},
                "evidence_links_add": input_ref("links"),
                "last_action": input_ref("text"),
                "next_action": input_ref("text")
            },
            "required": ["id"],
            "additionalProperties": false
        }),
    )
}

fn experiment_operation_schema() -> Value {
    let mut properties = serde_json::Map::new();
    properties.insert("kind".to_owned(), json!({"const": "record_experiment"}));
    properties.insert("experiment".to_owned(), json!({
        "type": "object",
        "properties": {
            "id": input_ref("text"), "hypothesis_id": input_ref("text"),
            "target_snapshot_id": input_ref("text"), "capsule_id": input_ref("text"),
            "goal": input_ref("text"),
            "result": {"type": "string", "enum": ["pending", "positive", "negative", "inconclusive"]},
            "artifact_refs": input_ref("texts"), "notes": {"type": "string"}
        },
        "required": ["id", "hypothesis_id", "target_snapshot_id", "capsule_id", "goal", "result"],
        "additionalProperties": false
    }));
    properties.insert("capsule".to_owned(), json!({
        "type": "object",
        "properties": {
            "id": input_ref("text"), "experiment_id": input_ref("text"), "target_snapshot_id": input_ref("text"),
            "action": input_ref("text"), "arguments": {}, "tool_version": input_ref("text"),
            "execution_ids": input_ref("texts"), "input_artifact_ids": input_ref("texts"),
            "environment_delta": {"type": "string"}, "account_prerequisites": input_ref("texts"),
            "toolchain_fingerprint": input_ref("text"), "execution_receipt_ids": input_ref("texts"),
            "output_artifact_ids": input_ref("texts")
        },
        "required": ["id", "experiment_id", "target_snapshot_id", "action", "tool_version", "toolchain_fingerprint"],
        "additionalProperties": false
    }));
    properties.insert("rerun_reason".to_owned(), input_ref("text"));
    json!({
        "type": "object",
        "properties": properties,
        "required": ["kind", "experiment", "capsule"],
        "additionalProperties": false
    })
}

fn experiment_update_operation_schema() -> Value {
    tagged_update_operation(
        "update_experiment",
        json!({
            "type": "object",
            "properties": {
                "id": input_ref("text"),
                "result": {"type": "string", "enum": ["pending", "positive", "negative", "inconclusive"]},
                "artifact_refs_add": input_ref("texts"),
                "notes_append": input_ref("text")
            },
            "required": ["id"],
            "additionalProperties": false
        }),
    )
}

fn finding_operation_schema() -> Value {
    tagged_operation_with_optional_fields(
        "record_finding",
        json!({
            "type": "object",
            "properties": {
                "id": input_ref("text"), "title": input_ref("text"),
                "status": {"type": "string", "enum": ["candidate", "reproduced", "confirmed", "discarded"]},
                "target_id": input_ref("text"), "target_version": input_ref("text"),
                "vulnerability_class": input_ref("text"), "entry_point": input_ref("text"), "sensitive_sink": input_ref("text"),
                "target_snapshot_ids": input_ref("texts"), "prerequisites": input_ref("texts"),
                "impact": input_ref("text"), "evidence_links": input_ref("links")
            },
            "required": ["id", "title", "status", "target_id", "target_version", "vulnerability_class", "entry_point", "sensitive_sink", "impact"],
            "additionalProperties": false
        }),
        json!({"keep_independent": {"type": "boolean"}}),
    )
}

fn finding_update_operation_schema() -> Value {
    tagged_update_operation(
        "update_finding",
        json!({
            "type": "object",
            "properties": {
                "id": input_ref("text"),
                "status": {"type": "string", "enum": ["candidate", "reproduced", "confirmed", "discarded"]},
                "target_snapshot_ids_add": input_ref("texts"),
                "prerequisites_add": input_ref("texts"),
                "evidence_links_add": input_ref("links"),
                "impact": input_ref("text")
            },
            "required": ["id"],
            "additionalProperties": false
        }),
    )
}

fn next_action_operation_schema() -> Value {
    tagged_operation(
        "record_next_action",
        json!({
            "type": "object",
            "properties": {
                "id": input_ref("text"), "research_id": research_ref_schema(), "priority": {"type": "integer"},
                "action": input_ref("text"),
                "status": {"type": "string", "enum": ["pending", "running", "done", "blocked"]}
            },
            "required": ["id", "research_id", "priority", "action", "status"],
            "additionalProperties": false
        }),
    )
}

fn next_action_update_operation_schema() -> Value {
    tagged_update_operation(
        "update_next_action",
        json!({
            "type": "object",
            "properties": {
                "id": input_ref("text"), "priority": {"type": "integer"},
                "action": input_ref("text"),
                "status": {"type": "string", "enum": ["pending", "running", "done", "blocked"]}
            },
            "required": ["id"],
            "additionalProperties": false
        }),
    )
}

fn merge_operation_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "kind": {"const": "merge_records"}, "from": input_ref("merge"),
            "into": input_ref("merge"), "reason": input_ref("text")
        },
        "required": ["kind", "from", "into", "reason"],
        "additionalProperties": false
    })
}

fn tagged_operation(kind: &str, record_schema: Value) -> Value {
    tagged_operation_with_extra(kind, record_schema, json!({}))
}

fn tagged_operation_with_extra(kind: &str, record_schema: Value, extra: Value) -> Value {
    let mut properties = serde_json::Map::new();
    properties.insert("kind".to_owned(), json!({"const": kind}));
    properties.insert("record".to_owned(), record_schema);
    if let Some(extra) = extra.as_object() {
        properties.extend(extra.clone());
    }
    json!({
        "type": "object",
        "properties": properties,
        "required": ["kind", "record"],
        "additionalProperties": false
    })
}

fn tagged_operation_with_optional_fields(kind: &str, record_schema: Value, extra: Value) -> Value {
    tagged_operation_with_extra(kind, record_schema, extra)
}

fn tagged_update_operation(kind: &str, update_schema: Value) -> Value {
    let mut properties = serde_json::Map::new();
    properties.insert("kind".to_owned(), json!({"const": kind}));
    properties.insert("update".to_owned(), update_schema);
    json!({
        "type": "object",
        "properties": properties,
        "required": ["kind", "update"],
        "additionalProperties": false
    })
}

fn string_schema() -> Value {
    json!({"type": "string", "minLength": 1})
}

fn string_array_schema() -> Value {
    json!({"type": "array", "items": string_schema()})
}

fn evidence_links_schema() -> Value {
    json!({"type": "array", "items": {"$ref": "#/$defs/evidence_link"}})
}

fn evidence_link_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "relation": {
                "type": "string",
                "enum": [
                    "supports", "contradicts", "positive_control", "negative_control",
                    "environment_control", "reachability_proof", "prerequisite_proof",
                    "impact_proof"
                ]
            },
            "evidence": {"$ref": "#/$defs/evidence_ref"}
        },
        "required": ["relation", "evidence"],
        "additionalProperties": false
    })
}

fn evidence_gap_report_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "research_id": evidence_gap_research_ref_schema(),
            "gaps": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "kind": {
                            "type": "string",
                            "enum": [
                                "entry_unproven", "sink_unproven", "reachability_unproven",
                                "prerequisite_unproven", "positive_control_missing",
                                "negative_control_missing", "environment_control_missing",
                                "impact_unproven", "repeatability_missing",
                                "target_or_version_binding_missing",
                                "artifact_or_receipt_provenance_missing",
                                "contradiction_unresolved"
                            ]
                        },
                        "state": {"type": "string", "enum": ["missing", "unknown"]},
                        "reason_code": {
                            "type": "string",
                            "enum": [
                                "entry_reachability_proof_absent",
                                "sink_reachability_proof_absent",
                                "reachability_proof_absent",
                                "prerequisite_fact_unavailable",
                                "prerequisite_proof_absent",
                                "positive_control_absent",
                                "negative_control_absent",
                                "environment_control_absent",
                                "impact_fact_unavailable",
                                "impact_proof_absent",
                                "fewer_than_two_positive_experiments",
                                "target_snapshot_binding_absent",
                                "artifact_or_receipt_reference_absent",
                                "contradiction_resolution_unmodeled"
                            ]
                        }
                    },
                    "required": ["kind", "state", "reason_code"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["research_id", "gaps"],
        "additionalProperties": false
    })
}

fn evidence_ref_schema() -> Value {
    json!({
        "oneOf": [
            {"type": "object", "properties": {"kind": {"const": "artifact"}, "id": string_schema()}, "required": ["kind", "id"], "additionalProperties": false},
            {"type": "object", "properties": {"kind": {"const": "experiment"}, "id": string_schema()}, "required": ["kind", "id"], "additionalProperties": false},
            {"type": "object", "properties": {"kind": {"const": "execution_receipt"}, "id": string_schema()}, "required": ["kind", "id"], "additionalProperties": false}
        ]
    })
}

fn evidence_gap_research_ref_schema() -> Value {
    json!({"oneOf": [
        {"type": "object", "properties": {"kind": {"const": "hypothesis"}, "id": string_schema()}, "required": ["kind", "id"], "additionalProperties": false},
        {"type": "object", "properties": {"kind": {"const": "finding"}, "id": string_schema()}, "required": ["kind", "id"], "additionalProperties": false}
    ]})
}

fn research_ref_schema() -> Value {
    json!({"oneOf": [
        {"type": "object", "properties": {"kind": {"const": "hypothesis"}, "id": string_schema()}, "required": ["kind", "id"], "additionalProperties": false},
        {"type": "object", "properties": {"kind": {"const": "experiment"}, "id": string_schema()}, "required": ["kind", "id"], "additionalProperties": false},
        {"type": "object", "properties": {"kind": {"const": "finding"}, "id": string_schema()}, "required": ["kind", "id"], "additionalProperties": false}
    ]})
}

fn merge_ref_schema() -> Value {
    json!({"oneOf": [
        {"type": "object", "properties": {"kind": {"const": "hypothesis"}, "id": string_schema()}, "required": ["kind", "id"], "additionalProperties": false},
        {"type": "object", "properties": {"kind": {"const": "finding"}, "id": string_schema()}, "required": ["kind", "id"], "additionalProperties": false}
    ]})
}
