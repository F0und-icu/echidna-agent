use echidna::adapters::tools::ToolRegistry;
use echidna::ports::tool::ToolExecutor;
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
fn complete_catalog_preserves_baseline_outside_explicit_file_contract_additions() {
    let before: Value = serde_json::from_str(include_str!(
        "fixtures/tools/input_schema_before_sharing.json"
    ))
    .unwrap();
    let tools = ToolRegistry::for_project(".");
    let catalog = tools.catalog();
    assert_eq!(catalog, tools.catalog(), "deterministic construction");
    assert_eq!(catalog.len(), before.as_object().unwrap().len());
    let mut baseline_saved = 0;
    for spec in catalog {
        let original = &before[spec.name.as_str()];
        // The frozen R6 sharing baseline must not be regenerated for R7's
        // intentional file-contract additions. Validate and remove only those
        // additions, then compare every remaining Schema byte/constraint.
        let schema = without_r7_file_contract_additions(spec.name.as_str(), &spec.input_schema);
        let expanded = expanded(&schema, &schema, 0);
        let hash = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&expanded).unwrap())
        );
        let old_size = original["input_bytes"].as_u64().unwrap() as usize;
        let size = serde_json::to_vec(&schema).unwrap().len();
        if spec.name.as_str() == "research_get" {
            assert_eq!(spec.version, "2");
            assert_eq!(
                original["expanded_sha256"],
                json!("d12b99d4d23a26c49f74040f41a5004299df59a4eed146f6b5ad657aae7c4b78")
            );
            assert_eq!(old_size, 373, "historical v1 fixture must stay frozen");
            assert_eq!(
                hash,
                "589a8bc61f482a741da61facbd4bd29e816f43ae7424de1072925099464abbc7"
            );
            assert_eq!(size, 585);
            continue;
        }
        assert_eq!(hash, original["expanded_sha256"], "{}", spec.name);
        if spec.name.as_str() == "research_update" {
            assert_eq!(old_size, 9711);
            assert_eq!(size, 8690);
        } else {
            assert_eq!(size, old_size, "unchanged other input: {}", spec.name);
        }
        baseline_saved += old_size - size;
    }
    // Historical sharing benefit only; not the net size of today's catalog.
    assert_eq!(baseline_saved, 1021);
}

fn without_r7_file_contract_additions(name: &str, schema: &Value) -> Value {
    let mut baseline = schema.clone();
    let (descriptions, hash_path): (&[(&str, &str)], &str) = match name {
        "write_file" => (
            &[
                ("/properties/path", "Project-relative UTF-8 file"),
                (
                    "/properties/content",
                    "heredoc still requires JSON escaping",
                ),
            ],
            "/properties/expected_hash",
        ),
        "apply_patch" => (
            &[(
                "/properties/patch",
                "@@ -old_start,old_count +new_start,new_count @@",
            )],
            "/properties/preimages/items/properties/expected_hash",
        ),
        _ => return baseline,
    };
    for (path, required_text) in descriptions {
        let field = baseline.pointer_mut(path).unwrap().as_object_mut().unwrap();
        let description = field.remove("description").unwrap();
        assert!(description.as_str().unwrap().contains(required_text));
    }
    let hash = baseline
        .pointer_mut(hash_path)
        .unwrap()
        .as_object_mut()
        .unwrap();
    assert_eq!(hash.remove("pattern"), Some(json!("^[0-9a-f]{64}$")));
    let description = hash.remove("description").unwrap();
    let text = description.as_str().unwrap();
    for required in [
        "BLAKE3",
        "including an empty file",
        "does NOT exist",
        "after_hash",
    ] {
        assert!(text.contains(required));
    }
    baseline
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
