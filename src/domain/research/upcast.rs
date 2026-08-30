//! Pure chained migration of immutable Research payload projections.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::reducer::ResearchError;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct VersionedPayload {
    pub(super) research_schema_version: u16,
    pub(super) data: Value,
}

pub(super) fn decode_and_upcast(
    event_type: &str,
    payload: Value,
    current_version: u16,
) -> Result<VersionedPayload, ResearchError> {
    let mut versioned: VersionedPayload =
        serde_json::from_value(payload).map_err(|error| ResearchError::MalformedPayload {
            message: error.to_string(),
        })?;
    if versioned.research_schema_version == 0 || versioned.research_schema_version > current_version
    {
        return Err(ResearchError::UnsupportedPayloadVersion {
            received: versioned.research_schema_version,
        });
    }
    while versioned.research_schema_version < current_version {
        versioned = match versioned.research_schema_version {
            1 => upcast_v1_to_v2(event_type, versioned)?,
            received => return Err(ResearchError::UnsupportedPayloadVersion { received }),
        };
    }
    Ok(versioned)
}

fn upcast_v1_to_v2(
    event_type: &str,
    mut payload: VersionedPayload,
) -> Result<VersionedPayload, ResearchError> {
    match event_type {
        "research.hypothesis_recorded" => {
            let data = object_mut(&mut payload.data, "HypothesisRecorded payload")?;
            let record = required_object_mut(data, "record", "HypothesisRecorded record")?;
            upcast_hypothesis_links(record, "evidence_refs", "evidence_links")?;
        }
        "research.hypothesis_updated" => {
            let update = object_mut(&mut payload.data, "HypothesisUpdated payload")?;
            upcast_hypothesis_links(update, "evidence_refs_add", "evidence_links_add")?;
        }
        "research.finding_recorded" => {
            let data = object_mut(&mut payload.data, "FindingRecorded payload")?;
            let record = required_object_mut(data, "record", "FindingRecorded record")?;
            upcast_finding_links(record, "evidence_refs", "negative_refs", "evidence_links")?;
        }
        "research.finding_updated" => {
            let update = object_mut(&mut payload.data, "FindingUpdated payload")?;
            upcast_finding_links(
                update,
                "evidence_refs_add",
                "negative_refs_add",
                "evidence_links_add",
            )?;
        }
        _ => {}
    }
    payload.research_schema_version = 2;
    Ok(payload)
}

fn upcast_hypothesis_links(
    object: &mut Map<String, Value>,
    legacy_field: &str,
    current_field: &str,
) -> Result<(), ResearchError> {
    reject_current_field(object, current_field)?;
    let legacy = object
        .remove(legacy_field)
        .unwrap_or_else(|| Value::Array(Vec::new()));
    object.insert(
        current_field.to_owned(),
        Value::Array(evidence_links(legacy, "supports", legacy_field)?),
    );
    Ok(())
}

fn upcast_finding_links(
    object: &mut Map<String, Value>,
    positive_field: &str,
    negative_field: &str,
    current_field: &str,
) -> Result<(), ResearchError> {
    reject_current_field(object, current_field)?;
    let positive = object
        .remove(positive_field)
        .unwrap_or_else(|| Value::Array(Vec::new()));
    let negative = object
        .remove(negative_field)
        .unwrap_or_else(|| Value::Array(Vec::new()));
    let mut links = evidence_links(positive, "supports", positive_field)?;
    links.extend(evidence_links(negative, "contradicts", negative_field)?);
    object.insert(current_field.to_owned(), Value::Array(links));
    Ok(())
}

fn evidence_links(legacy: Value, relation: &str, field: &str) -> Result<Vec<Value>, ResearchError> {
    let references = legacy
        .as_array()
        .ok_or_else(|| malformed(format!("legacy `{field}` must be an array")))?;
    Ok(references
        .iter()
        .map(|evidence| json!({"relation": relation, "evidence": evidence}))
        .collect())
}

fn reject_current_field(
    object: &Map<String, Value>,
    current_field: &str,
) -> Result<(), ResearchError> {
    if object.contains_key(current_field) {
        Err(malformed(format!(
            "Research payload v1 cannot contain v2 field `{current_field}`"
        )))
    } else {
        Ok(())
    }
}

fn object_mut<'a>(
    value: &'a mut Value,
    subject: &str,
) -> Result<&'a mut Map<String, Value>, ResearchError> {
    value
        .as_object_mut()
        .ok_or_else(|| malformed(format!("{subject} must be an object")))
}

fn required_object_mut<'a>(
    object: &'a mut Map<String, Value>,
    field: &str,
    subject: &str,
) -> Result<&'a mut Map<String, Value>, ResearchError> {
    object
        .get_mut(field)
        .and_then(Value::as_object_mut)
        .ok_or_else(|| malformed(format!("{subject} must be an object")))
}

fn malformed(message: String) -> ResearchError {
    ResearchError::MalformedPayload { message }
}
