//! Versioned request-audit identity and persisted Manifest shape.

use serde::{Deserialize, Serialize};

use crate::domain::context::{
    BaselineInvalidationReason, ModelIdentity, ModelProfile, PromptRendererIdentity,
};
use crate::domain::events::{EventId, StreamId};
use crate::domain::model::{
    CapabilityMappingVersion, MODEL_INPUT_IR_SCHEMA_VERSION, ModelCapabilities, ModelInputIR,
    ModelInputIrSchemaVersion,
};
use crate::domain::project::ProjectId;
use crate::domain::sampling::SamplingStepPlan;
use crate::domain::tools::ResolvedToolPlanFingerprint;

use super::components::{SemanticHash, validate_component_layout};
use super::{
    REQUEST_AUDIT_MANIFEST_SCHEMA_VERSION, RequestAuditComponentReference, RequestAuditError,
};

/// The project and Session Event stream which jointly own one audit fact.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RequestAuditBinding {
    project_id: ProjectId,
    session_stream_id: StreamId,
}

impl RequestAuditBinding {
    pub fn new(
        project_id: ProjectId,
        session_stream_id: StreamId,
    ) -> Result<Self, RequestAuditError> {
        let binding = Self {
            project_id,
            session_stream_id,
        };
        binding.validate()?;
        Ok(binding)
    }

    pub fn project_id(&self) -> &ProjectId {
        &self.project_id
    }

    pub fn session_stream_id(&self) -> &StreamId {
        &self.session_stream_id
    }

    fn validate(&self) -> Result<(), RequestAuditError> {
        if self.project_id.as_str().trim().is_empty() {
            return Err(RequestAuditError::BindingFieldRequired {
                field: "project_id",
            });
        }
        if self.session_stream_id.as_str().trim().is_empty() {
            return Err(RequestAuditError::BindingFieldRequired {
                field: "session_stream_id",
            });
        }
        Ok(())
    }
}

/// Caller/configuration supplied Adapter version. It is never inferred from
/// a Provider type or model name.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct RequestAdapterVersion(String);

impl RequestAdapterVersion {
    pub fn new(value: impl Into<String>) -> Result<Self, RequestAuditError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(RequestAuditError::AdapterVersionRequired);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Fixed metadata supplied to the explicit audited AgentLoop path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequestAuditRunContext {
    binding: RequestAuditBinding,
    adapter_version: RequestAdapterVersion,
}

impl RequestAuditRunContext {
    pub fn new(binding: RequestAuditBinding, adapter_version: RequestAdapterVersion) -> Self {
        Self {
            binding,
            adapter_version,
        }
    }

    pub fn binding(&self) -> &RequestAuditBinding {
        &self.binding
    }

    pub fn adapter_version(&self) -> &RequestAdapterVersion {
        &self.adapter_version
    }
}

/// The actual Provider delivery choice, excluding response and continuation
/// identifiers because those are transport/session-only state.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestAuditDeliveryMode {
    Full,
    Incremental,
}

/// The deterministic local explanation for a delivery mode.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestAuditDeliveryReason {
    Full(BaselineInvalidationReason),
    AppendOnlyCandidate,
}

/// Mode plus reason captured for an actual Provider attempt.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RequestAuditDelivery {
    mode: RequestAuditDeliveryMode,
    reason: RequestAuditDeliveryReason,
}

impl RequestAuditDelivery {
    pub const fn full(reason: BaselineInvalidationReason) -> Self {
        Self {
            mode: RequestAuditDeliveryMode::Full,
            reason: RequestAuditDeliveryReason::Full(reason),
        }
    }

    pub const fn incremental_append_only() -> Self {
        Self {
            mode: RequestAuditDeliveryMode::Incremental,
            reason: RequestAuditDeliveryReason::AppendOnlyCandidate,
        }
    }

    pub const fn mode(&self) -> RequestAuditDeliveryMode {
        self.mode
    }

    pub const fn reason(&self) -> RequestAuditDeliveryReason {
        self.reason
    }

    pub(super) fn validate(&self) -> Result<(), RequestAuditError> {
        match (self.mode, self.reason) {
            (RequestAuditDeliveryMode::Full, RequestAuditDeliveryReason::Full(_))
            | (
                RequestAuditDeliveryMode::Incremental,
                RequestAuditDeliveryReason::AppendOnlyCandidate,
            ) => Ok(()),
            _ => Err(RequestAuditError::DeliveryReasonMismatch),
        }
    }
}

/// One supplied actual Provider attempt before any Blob or Event I/O.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequestAuditAttempt {
    pub(super) binding: RequestAuditBinding,
    pub(super) model: ModelIdentity,
    pub(super) profile: ModelProfile,
    pub(super) adapter_version: RequestAdapterVersion,
    pub(super) capabilities: ModelCapabilities,
    pub(super) full_logical_input: ModelInputIR,
    pub(super) delivered_input: ModelInputIR,
    pub(super) delivery: RequestAuditDelivery,
    pub(super) prompt_renderer: Option<PromptRendererIdentity>,
    pub(super) resolved_tool_plan_fingerprint: Option<ResolvedToolPlanFingerprint>,
}

impl RequestAuditAttempt {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        context: &RequestAuditRunContext,
        model: ModelIdentity,
        profile: ModelProfile,
        capabilities: ModelCapabilities,
        full_logical_input: ModelInputIR,
        delivered_input: ModelInputIR,
        delivery: RequestAuditDelivery,
    ) -> Result<Self, RequestAuditError> {
        context.binding.validate()?;
        if context.adapter_version.as_str().trim().is_empty() {
            return Err(RequestAuditError::AdapterVersionRequired);
        }
        delivery.validate()?;
        let full_logical_audit_input = RequestAuditModelInput::from_input(&full_logical_input)?;
        full_logical_audit_input
            .validate("full_logical_input", "full_logical_input.semantic_hash")?;
        let delivered_audit_input = RequestAuditModelInput::from_input(&delivered_input)?;
        delivered_audit_input.validate("delivered_input", "delivered_input.semantic_hash")?;
        if delivery.mode() == RequestAuditDeliveryMode::Full
            && full_logical_input != delivered_input
        {
            return Err(RequestAuditError::FullDeliveryInputMismatch);
        }
        Ok(Self {
            binding: context.binding.clone(),
            model,
            profile,
            adapter_version: context.adapter_version.clone(),
            capabilities,
            full_logical_input,
            delivered_input,
            delivery,
            prompt_renderer: None,
            resolved_tool_plan_fingerprint: None,
        })
    }

    pub fn binding(&self) -> &RequestAuditBinding {
        &self.binding
    }

    pub fn delivery(&self) -> RequestAuditDelivery {
        self.delivery
    }

    pub fn bind_sampling_step(
        mut self,
        step: &SamplingStepPlan,
    ) -> Result<Self, RequestAuditError> {
        if self.full_logical_input.tool_specs != step.resolved_tool_plan().advertised_specs()
            || self.delivered_input.tool_specs != step.resolved_tool_plan().advertised_specs()
        {
            return Err(RequestAuditError::SamplingToolPlanMismatch);
        }
        self.prompt_renderer = Some(step.prompt_renderer().clone());
        self.resolved_tool_plan_fingerprint = Some(step.resolved_tool_plan().fingerprint().clone());
        Ok(self)
    }
}

/// Schema and semantic identity for one logical or actually delivered IR.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RequestAuditModelInput {
    pub(super) schema_version: ModelInputIrSchemaVersion,
    semantic_hash: SemanticHash,
}

impl RequestAuditModelInput {
    pub(super) fn from_input(input: &ModelInputIR) -> Result<Self, RequestAuditError> {
        Ok(Self {
            schema_version: input.schema_version,
            semantic_hash: SemanticHash::from_canonical(input)?,
        })
    }

    pub const fn schema_version(&self) -> ModelInputIrSchemaVersion {
        self.schema_version
    }

    pub fn semantic_hash(&self) -> &SemanticHash {
        &self.semantic_hash
    }

    fn validate(
        &self,
        input_field: &'static str,
        semantic_hash_field: &'static str,
    ) -> Result<(), RequestAuditError> {
        if self.schema_version.get() != MODEL_INPUT_IR_SCHEMA_VERSION {
            return Err(RequestAuditError::UnsupportedModelInputIrSchema {
                input_field,
                schema_version: self.schema_version.get(),
            });
        }
        self.semantic_hash.validate(semantic_hash_field)
    }
}

/// Versioned capability-map identity used when making this request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RequestAuditCapabilityMapping {
    version: CapabilityMappingVersion,
    semantic_hash: SemanticHash,
}

impl RequestAuditCapabilityMapping {
    pub(super) fn from_capabilities(
        capabilities: &ModelCapabilities,
    ) -> Result<Self, RequestAuditError> {
        Ok(Self {
            version: capabilities.capability_mapping_version,
            semantic_hash: SemanticHash::from_canonical(capabilities)?,
        })
    }

    pub const fn version(&self) -> CapabilityMappingVersion {
        self.version
    }

    pub fn semantic_hash(&self) -> &SemanticHash {
        &self.semantic_hash
    }

    fn validate(&self) -> Result<(), RequestAuditError> {
        self.semantic_hash
            .validate("capability_mapping.semantic_hash")
    }
}

/// The durable, versioned fact that reconstructs one actual semantic request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RequestAuditManifest {
    pub(super) schema_version: u16,
    pub(super) audit_event_id: EventId,
    pub(super) binding: RequestAuditBinding,
    pub(super) model: ModelIdentity,
    pub(super) profile: ModelProfile,
    pub(super) adapter_version: RequestAdapterVersion,
    pub(super) capability_mapping: RequestAuditCapabilityMapping,
    pub(super) full_logical_input: RequestAuditModelInput,
    pub(super) delivered_input: RequestAuditModelInput,
    pub(super) delivery: RequestAuditDelivery,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) prompt_renderer: Option<PromptRendererIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) resolved_tool_plan_fingerprint: Option<ResolvedToolPlanFingerprint>,
    pub(super) components: Vec<RequestAuditComponentReference>,
}

impl RequestAuditManifest {
    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }

    pub fn audit_event_id(&self) -> &EventId {
        &self.audit_event_id
    }

    pub fn binding(&self) -> &RequestAuditBinding {
        &self.binding
    }

    /// The configured model identity selected for the audited Provider call.
    pub fn model(&self) -> &ModelIdentity {
        &self.model
    }

    /// The named profile selected for the audited Provider call.
    pub fn profile(&self) -> &ModelProfile {
        &self.profile
    }

    pub fn adapter_version(&self) -> &RequestAdapterVersion {
        &self.adapter_version
    }

    pub fn capability_mapping(&self) -> &RequestAuditCapabilityMapping {
        &self.capability_mapping
    }

    pub fn full_logical_input(&self) -> &RequestAuditModelInput {
        &self.full_logical_input
    }

    pub fn delivered_input(&self) -> &RequestAuditModelInput {
        &self.delivered_input
    }

    pub const fn delivery(&self) -> RequestAuditDelivery {
        self.delivery
    }

    pub fn prompt_renderer(&self) -> Option<&PromptRendererIdentity> {
        self.prompt_renderer.as_ref()
    }

    pub fn resolved_tool_plan_fingerprint(&self) -> Option<&ResolvedToolPlanFingerprint> {
        self.resolved_tool_plan_fingerprint.as_ref()
    }

    pub fn components(&self) -> &[RequestAuditComponentReference] {
        &self.components
    }

    /// Validates every persisted structural invariant without reading Blob
    /// bytes. Reconstruction validates component content after the Port reads
    /// exact bytes.
    pub fn validate(&self) -> Result<(), RequestAuditError> {
        if !matches!(
            self.schema_version,
            1 | REQUEST_AUDIT_MANIFEST_SCHEMA_VERSION
        ) {
            return Err(RequestAuditError::UnsupportedManifestSchema {
                schema_version: self.schema_version,
            });
        }
        match (
            self.schema_version,
            &self.prompt_renderer,
            &self.resolved_tool_plan_fingerprint,
        ) {
            (1, None, None) => {}
            (REQUEST_AUDIT_MANIFEST_SCHEMA_VERSION, Some(renderer), Some(fingerprint))
                if renderer.is_generic_v1()
                    && fingerprint.as_str().len() == 64
                    && fingerprint
                        .as_str()
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f')) => {}
            _ => return Err(RequestAuditError::SamplingIdentityMismatch),
        }
        if self.audit_event_id.as_str().trim().is_empty() {
            return Err(RequestAuditError::BindingFieldRequired {
                field: "audit_event_id",
            });
        }
        self.binding.validate()?;
        if self.adapter_version.as_str().trim().is_empty() {
            return Err(RequestAuditError::AdapterVersionRequired);
        }
        self.capability_mapping.validate()?;
        self.full_logical_input
            .validate("full_logical_input", "full_logical_input.semantic_hash")?;
        self.delivered_input
            .validate("delivered_input", "delivered_input.semantic_hash")?;
        self.delivery.validate()?;
        if self.delivery.mode() == RequestAuditDeliveryMode::Full
            && self.full_logical_input != self.delivered_input
        {
            return Err(RequestAuditError::FullDeliveryInputMismatch);
        }
        validate_component_layout(&self.components)
    }
}
