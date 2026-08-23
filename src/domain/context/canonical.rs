//! Turn and Step capture plus canonical request hashing.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::domain::canonical_json::{canonical_blake3_hash, canonical_json};
use crate::domain::model::{ContentItem, ModelCapabilities, ModelInputIR};
use crate::domain::tools::ToolSpec;

macro_rules! string_id {
    ($name:ident) => {
        #[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }
    };
}

string_id!(TurnId);
string_id!(ProviderSessionId);
string_id!(ModelId);
string_id!(ProfileId);

/// A stable BLAKE3 digest of one canonical request component.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct RequestHash(String);

/// The request role owned by a Provider session.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderSessionRole {
    Primary,
    Vision,
}

/// The stable identity captured by a Turn and stored by its Baseline.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProviderSessionRef {
    pub id: ProviderSessionId,
    pub role: ProviderSessionRole,
    pub generation: u64,
}

/// The selected Provider model, separated from the named profile that chose it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ModelIdentity {
    pub provider_id: String,
    pub model_id: ModelId,
}

impl ModelIdentity {
    pub fn new(provider_id: impl Into<String>, model_id: impl Into<String>) -> Self {
        Self {
            provider_id: provider_id.into(),
            model_id: ModelId::new(model_id),
        }
    }
}

/// A named configuration generation which selected the current model.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ModelProfile {
    pub id: ProfileId,
    pub generation: u64,
}

impl ModelProfile {
    pub fn new(id: impl Into<String>, generation: u64) -> Self {
        Self {
            id: ProfileId::new(id),
            generation,
        }
    }
}

/// Turn-stable state captured when a user input begins.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TurnContext {
    pub turn_id: TurnId,
    pub model: ModelIdentity,
    pub profile: ModelProfile,
    pub provider_session: ProviderSessionRef,
    pub context_budget: u32,
    pub tool_exposure_hash: RequestHash,
}

impl TurnContext {
    pub fn new(
        turn_id: TurnId,
        model: ModelIdentity,
        profile: ModelProfile,
        provider_session: ProviderSessionRef,
        context_budget: u32,
        tool_specs: &[ToolSpec],
    ) -> Result<Self, ContextError> {
        Ok(Self {
            turn_id,
            model,
            profile,
            provider_session,
            context_budget,
            tool_exposure_hash: canonical_tool_schema_hash(tool_specs)?,
        })
    }
}

/// Local bookkeeping which must never affect Provider request equivalence.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct InternalRequestMetadata {
    pub local_request_id: Option<String>,
    pub local_timestamp: Option<String>,
    pub annotations: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct OutputContract {
    structured_output: bool,
}

impl OutputContract {
    fn from_input(input: &ModelInputIR) -> Self {
        Self {
            structured_output: input.requirements.structured_output,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct RequestProperties {
    tool_call: bool,
    incremental_input: bool,
    usage_reporting: bool,
}

impl RequestProperties {
    fn from_input(input: &ModelInputIR) -> Self {
        Self {
            tool_call: input.requirements.tool_call,
            incremental_input: input.requirements.incremental_input,
            usage_reporting: input.requirements.usage_reporting,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(super) struct CanonicalInput {
    pub(super) items: Vec<ContentItem>,
}

impl CanonicalInput {
    pub(super) fn from_items(items: Vec<ContentItem>) -> Self {
        Self {
            items: items.iter().map(canonical_content_item).collect(),
        }
    }

    pub(super) fn items(&self) -> &[ContentItem] {
        &self.items
    }
}

/// Every semantic request component used by Baseline comparison.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CanonicalRequest {
    pub(super) model_hash: RequestHash,
    pub(super) profile_hash: RequestHash,
    pub(super) model_input_ir_schema_hash: RequestHash,
    pub(super) provider_capability_hash: RequestHash,
    pub(super) prompt_hash: RequestHash,
    pub(super) tool_schema_hash: RequestHash,
    pub(super) output_contract_hash: RequestHash,
    pub(super) request_properties_hash: RequestHash,
    pub(super) canonical_input: CanonicalInput,
}

impl CanonicalRequest {
    fn from_input(
        turn: &TurnContext,
        input: &ModelInputIR,
        capabilities: &ModelCapabilities,
    ) -> Result<Self, ContextError> {
        Ok(Self {
            model_hash: hash_value(&turn.model)?,
            profile_hash: hash_value(&turn.profile)?,
            model_input_ir_schema_hash: hash_value(&input.schema_version)?,
            provider_capability_hash: hash_value(capabilities)?,
            prompt_hash: hash_value(&input.instructions)?,
            tool_schema_hash: canonical_tool_schema_hash(&input.tool_specs)?,
            output_contract_hash: hash_value(&OutputContract::from_input(input))?,
            request_properties_hash: hash_value(&RequestProperties::from_input(input))?,
            canonical_input: CanonicalInput::from_items(input.content_items.clone()),
        })
    }
}

/// The per-sampling snapshot. Metadata remains inspectable but non-semantic.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StepSnapshot {
    pub turn_id: TurnId,
    pub provider_session: ProviderSessionRef,
    pub sample_index: usize,
    pub context_budget: u32,
    pub tool_exposure_hash: RequestHash,
    pub request: CanonicalRequest,
    pub internal_metadata: InternalRequestMetadata,
}

impl StepSnapshot {
    pub fn capture(
        turn: &TurnContext,
        sample_index: usize,
        input: &ModelInputIR,
        capabilities: &ModelCapabilities,
        internal_metadata: InternalRequestMetadata,
    ) -> Result<Self, ContextError> {
        Ok(Self {
            turn_id: turn.turn_id.clone(),
            provider_session: turn.provider_session.clone(),
            sample_index,
            context_budget: turn.context_budget,
            tool_exposure_hash: turn.tool_exposure_hash.clone(),
            request: CanonicalRequest::from_input(turn, input, capabilities)?,
            internal_metadata,
        })
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ContextError {
    #[error("failed to encode a canonical Provider request component: {message}")]
    CanonicalEncoding { message: String },
    #[error("ProviderSession `{received:?}` cannot update `{expected:?}`")]
    ProviderSessionMismatch {
        expected: ProviderSessionRef,
        received: ProviderSessionRef,
    },
}

#[derive(Serialize)]
struct RequestToolSchema<'a> {
    name: &'a crate::domain::tools::ToolName,
    version: &'a str,
    description: &'a str,
    input_schema: Value,
    output_schema: Value,
    exposure: crate::domain::tools::ToolExposure,
}

pub(super) fn canonical_tool_schema_hash(specs: &[ToolSpec]) -> Result<RequestHash, ContextError> {
    Ok(canonical_tool_schemas(specs)?.hash)
}

pub(super) struct CanonicalToolSchemas {
    pub(super) ordered_specs: Vec<ToolSpec>,
    pub(super) hash: RequestHash,
    pub(super) encoded_len: usize,
}

pub(super) fn canonical_tool_schemas(
    specs: &[ToolSpec],
) -> Result<CanonicalToolSchemas, ContextError> {
    let mut ordered_specs = specs
        .iter()
        .map(|spec| {
            let tool = request_tool_schema(spec);
            let sort_key =
                serde_json::to_vec(&tool).map_err(|error| ContextError::CanonicalEncoding {
                    message: error.to_string(),
                })?;
            Ok((sort_key, spec.clone()))
        })
        .collect::<Result<Vec<_>, ContextError>>()?;
    ordered_specs.sort_by(|left, right| left.0.cmp(&right.0));
    let ordered_specs = ordered_specs
        .into_iter()
        .map(|(_, spec)| spec)
        .collect::<Vec<_>>();
    let request_tools = ordered_specs
        .iter()
        .map(request_tool_schema)
        .collect::<Vec<_>>();
    let encoded_len = serde_json::to_vec(&request_tools)
        .map_err(|error| ContextError::CanonicalEncoding {
            message: error.to_string(),
        })?
        .len();
    Ok(CanonicalToolSchemas {
        hash: hash_value(&request_tools)?,
        ordered_specs,
        encoded_len,
    })
}

fn request_tool_schema(spec: &ToolSpec) -> RequestToolSchema<'_> {
    RequestToolSchema {
        name: &spec.name,
        version: &spec.version,
        description: &spec.description,
        input_schema: canonical_json(&spec.input_schema),
        output_schema: canonical_json(&spec.output_schema),
        exposure: spec.exposure,
    }
}

fn canonical_content_item(item: &ContentItem) -> ContentItem {
    match item {
        ContentItem::Text { role, text } => ContentItem::Text {
            role: *role,
            text: text.clone(),
        },
        ContentItem::ImageArtifact { role, artifact_id } => ContentItem::ImageArtifact {
            role: *role,
            artifact_id: artifact_id.clone(),
        },
        ContentItem::ToolCall { call } => ContentItem::ToolCall {
            call: crate::domain::tools::ToolCall::new(
                call.call_id.clone(),
                call.name.clone(),
                canonical_json(&call.arguments),
            ),
        },
        ContentItem::ToolResult { result } => ContentItem::ToolResult {
            result: crate::domain::tools::ToolResult {
                call_id: result.call_id.clone(),
                tool_name: result.tool_name.clone(),
                status: result.status,
                output: canonical_json(&result.output),
            },
        },
        ContentItem::Unsupported { content_kind } => ContentItem::Unsupported {
            content_kind: content_kind.clone(),
        },
    }
}

fn hash_value(value: &impl Serialize) -> Result<RequestHash, ContextError> {
    canonical_blake3_hash(value)
        .map(RequestHash)
        .map_err(|error| ContextError::CanonicalEncoding {
            message: error.to_string(),
        })
}
