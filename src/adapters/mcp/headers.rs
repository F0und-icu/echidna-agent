use std::collections::BTreeSet;

use base64::Engine;
use serde_json::Value;

use super::McpError;

#[derive(Clone, Debug)]
pub(crate) struct HeaderBinding {
    pub(crate) name: String,
    path: Vec<String>,
    value_type: HeaderValueType,
}

#[derive(Clone, Copy, Debug)]
enum HeaderValueType {
    String,
    Integer,
    Boolean,
}

const NON_STATIC_SCHEMA_SUBTREE_KEYWORDS: &[&str] = &[
    "items",
    "prefixItems",
    "contains",
    "unevaluatedItems",
    "additionalProperties",
    "unevaluatedProperties",
    "propertyNames",
    "allOf",
    "anyOf",
    "oneOf",
    "not",
    "if",
    "then",
    "else",
    "contentSchema",
];

const NON_STATIC_SCHEMA_MAP_KEYWORDS: &[&str] = &[
    "$defs",
    "definitions",
    "patternProperties",
    "dependentSchemas",
];

pub(crate) fn bindings(schema: &Value, tool_name: &str) -> Result<Vec<HeaderBinding>, McpError> {
    let mut bindings = Vec::new();
    let mut names = BTreeSet::new();
    walk(
        schema,
        &mut Vec::new(),
        tool_name,
        &mut names,
        &mut bindings,
    )?;
    Ok(bindings)
}

pub(crate) fn request_headers(
    bindings: &[HeaderBinding],
    arguments: &Value,
) -> Result<Vec<(String, String)>, McpError> {
    bindings
        .iter()
        .filter_map(|binding| argument_at(arguments, &binding.path).map(|value| (binding, value)))
        .map(|(binding, value)| {
            let value = match binding.value_type {
                HeaderValueType::String => value.as_str().map(ToOwned::to_owned),
                HeaderValueType::Integer => safe_integer(value).map(|value| value.to_string()),
                HeaderValueType::Boolean => value.as_bool().map(|value| value.to_string()),
            }
            .ok_or_else(|| McpError::PreExecution {
                message: format!(
                    "annotated parameter `{}` has the wrong JSON type",
                    binding.path.join(".")
                ),
            })?;
            Ok((
                format!("Mcp-Param-{}", binding.name),
                encode_header_value(&value),
            ))
        })
        .collect()
}

/// MCP header annotations accept JSON Schema integers. Serde represents an
/// integral spelling such as `1.0` or `1e2` as a floating JSON number, so the
/// boundary accepts those values only when they are finite, integral, and in
/// the documented JavaScript-safe range. The original argument remains intact
/// for the remote Tool call.
fn safe_integer(value: &Value) -> Option<i64> {
    const MIN_SAFE_INTEGER: i64 = -9_007_199_254_740_991;
    const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

    value
        .as_i64()
        .or_else(|| {
            let value = value.as_f64()?;
            (value.is_finite()
                && value.fract() == 0.0
                && (MIN_SAFE_INTEGER as f64..=MAX_SAFE_INTEGER as f64).contains(&value))
            .then_some(value as i64)
        })
        .filter(|value| (MIN_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(value))
}

pub(crate) fn encode_header_value(value: &str) -> String {
    let sentinel = value.starts_with("=?base64?") && value.ends_with("?=");
    let unsafe_value = value.is_empty()
        || value.trim() != value
        || !value.bytes().all(|byte| (0x20..=0x7e).contains(&byte));
    if sentinel || unsafe_value {
        format!(
            "=?base64?{}?=",
            base64::engine::general_purpose::STANDARD.encode(value.as_bytes())
        )
    } else {
        value.to_owned()
    }
}

fn walk(
    schema: &Value,
    path: &mut Vec<String>,
    tool_name: &str,
    names: &mut BTreeSet<String>,
    bindings: &mut Vec<HeaderBinding>,
) -> Result<(), McpError> {
    if schema.is_boolean() {
        return Ok(());
    }
    let object = schema.as_object().ok_or_else(|| McpError::Catalog {
        message: format!("tool `{tool_name}` has a non-object inputSchema"),
    })?;
    if let Some(annotation) = object.get("x-mcp-header") {
        if path.is_empty() {
            return Err(McpError::InvalidAnnotatedTool {
                tool_name: tool_name.to_owned(),
                message: "x-mcp-header is not on a statically reachable property path".to_owned(),
            });
        }
        let name = annotation
            .as_str()
            .ok_or_else(|| McpError::InvalidAnnotatedTool {
                tool_name: tool_name.to_owned(),
                message: "x-mcp-header must be a string HTTP field-name token".to_owned(),
            })?;
        if name.is_empty() || !name.bytes().all(is_token) {
            return Err(McpError::InvalidAnnotatedTool {
                tool_name: tool_name.to_owned(),
                message: "x-mcp-header is not an HTTP field-name token".to_owned(),
            });
        }
        if !names.insert(name.to_ascii_lowercase()) {
            return Err(McpError::InvalidAnnotatedTool {
                tool_name: tool_name.to_owned(),
                message: "x-mcp-header values must be case-insensitively unique".to_owned(),
            });
        }
        let value_type = match object.get("type").and_then(Value::as_str) {
            Some("string") => HeaderValueType::String,
            Some("integer") => HeaderValueType::Integer,
            Some("boolean") => HeaderValueType::Boolean,
            _ => {
                return Err(McpError::InvalidAnnotatedTool {
                    tool_name: tool_name.to_owned(),
                    message: "x-mcp-header requires string, integer, or boolean type".to_owned(),
                });
            }
        };
        bindings.push(HeaderBinding {
            name: name.to_owned(),
            path: path.clone(),
            value_type,
        });
    }
    reject_annotations_in_non_static_subtrees(object, tool_name)?;
    if let Some(properties) = object.get("properties") {
        let properties = properties.as_object().ok_or_else(|| McpError::Catalog {
            message: format!("tool `{tool_name}` has non-object inputSchema.properties"),
        })?;
        for (name, property) in properties {
            path.push(name.clone());
            walk(property, path, tool_name, names, bindings)?;
            path.pop();
        }
    }
    Ok(())
}

fn reject_annotations_in_non_static_subtrees(
    object: &serde_json::Map<String, Value>,
    tool_name: &str,
) -> Result<(), McpError> {
    for key in NON_STATIC_SCHEMA_SUBTREE_KEYWORDS {
        if let Some(value) = object.get(*key) {
            reject_unreachable_annotation(value, tool_name)?;
        }
    }
    for key in NON_STATIC_SCHEMA_MAP_KEYWORDS {
        if let Some(schemas) = object.get(*key).and_then(Value::as_object) {
            for schema in schemas.values() {
                reject_unreachable_annotation(schema, tool_name)?;
            }
        }
    }
    Ok(())
}

fn reject_unreachable_annotation(schema: &Value, tool_name: &str) -> Result<(), McpError> {
    match schema {
        Value::Object(object) => {
            if object.contains_key("x-mcp-header") {
                return Err(McpError::InvalidAnnotatedTool {
                    tool_name: tool_name.to_owned(),
                    message: "x-mcp-header is not on a statically reachable property path"
                        .to_owned(),
                });
            }
            if let Some(properties) = object.get("properties").and_then(Value::as_object) {
                for property in properties.values() {
                    reject_unreachable_annotation(property, tool_name)?;
                }
            }
            reject_annotations_in_non_static_subtrees(object, tool_name)?;
        }
        Value::Array(schemas) => {
            for schema in schemas {
                reject_unreachable_annotation(schema, tool_name)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn argument_at<'a>(mut value: &'a Value, path: &[String]) -> Option<&'a Value> {
    for property in path {
        value = value.as_object()?.get(property)?;
    }
    Some(value)
}

fn is_token(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte)
}
