use cyber_agent::adapters::tools::ToolRegistry;
use cyber_agent::ports::tool::ToolExecutor;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

// Test oracle only: expand root-local references at Schema positions, not in
// literal values. Hash fixtures were captured from the complete pre-change
// production catalog, not generated from the optimized implementation.
fn expanded(schema: &Value, root: &Value, depth: usize) -> Value {
    assert!(depth < 32, "unexpected recursive Schema");
    let Some(object) = schema.as_object() else {
        return schema.clone();
    };
    if let Some(reference) = object.get("$ref") {
        assert_eq!(object.len(), 1, "unaccounted reference siblings");
        let name = reference
            .as_str()
            .unwrap()
            .strip_prefix("#/$defs/")
            .unwrap();
        assert!(!name.contains(['/', '~']));
        let target = root["$defs"].get(name).expect("resolved definition");
        return expanded(target, root, depth + 1);
    }
    let mut result = object.clone();
    result.remove("$defs");
    for (keyword, value) in &mut result {
        match keyword.as_str() {
            "properties" | "patternProperties" | "dependentSchemas" => {
                for child in value.as_object_mut().unwrap().values_mut() {
                    *child = expanded(child, root, depth + 1);
                }
            }
            "allOf" | "anyOf" | "oneOf" | "prefixItems" => {
                for child in value.as_array_mut().unwrap() {
                    *child = expanded(child, root, depth + 1);
                }
            }
            "items"
            | "additionalProperties"
            | "contains"
            | "not"
            | "if"
            | "then"
            | "else"
            | "propertyNames"
            | "unevaluatedProperties"
            | "unevaluatedItems" => {
                *value = expanded(value, root, depth + 1);
            }
            _ => {}
        }
    }
    Value::Object(result)
}

#[test]
fn complete_builtin_input_catalog_preserves_expanded_contracts_and_reduces_bytes() {
    let before: Value = serde_json::from_str(include_str!(
        "fixtures/tools/input_schema_before_sharing.json"
    ))
    .unwrap();
    let tools = ToolRegistry::for_project(".");
    let catalog = tools.catalog();
    assert_eq!(catalog, tools.catalog(), "deterministic construction");
    assert_eq!(catalog.len(), before.as_object().unwrap().len());
    let mut saved = 0;
    for spec in catalog {
        let original = &before[spec.name.as_str()];
        let expanded = expanded(&spec.input_schema, &spec.input_schema, 0);
        let hash = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&expanded).unwrap())
        );
        assert_eq!(hash, original["expanded_sha256"], "{}", spec.name);
        let old_size = original["input_bytes"].as_u64().unwrap() as usize;
        let size = serde_json::to_vec(&spec.input_schema).unwrap().len();
        if spec.name.as_str() == "research_update" {
            assert_eq!(old_size, 9711);
            assert_eq!(size, 8690);
        } else {
            assert_eq!(size, old_size, "unchanged other input: {}", spec.name);
        }
        saved += old_size - size;
    }
    assert_eq!(saved, 1021);
}

#[test]
fn expansion_oracle_preserves_literal_objects_and_detects_constraint_changes() {
    let literal = json!({"$ref":"literal, not a Schema", "$defs":{"id":1}});
    let schema = json!({
        "$defs":{"text":{"type":"string","minLength":1}},
        "type":"object", "properties":{"name":{"$ref":"#/$defs/text"}},
        "const":literal, "default":literal, "examples":[literal], "enum":[literal]
    });
    let result = expanded(&schema, &schema, 0);
    assert_eq!(result["properties"]["name"]["minLength"], 1);
    for keyword in ["const", "default", "examples", "enum"] {
        assert_eq!(result[keyword], schema[keyword]);
    }
    let mut changed = schema.clone();
    changed["$defs"]["text"]["minLength"] = json!(0);
    assert_ne!(result, expanded(&changed, &changed, 0));
}
