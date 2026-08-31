//! One full, no-tool Vision request with pre-stream RequestAudit.

use std::fmt::Display;

use serde::Deserialize;
use thiserror::Error;

use crate::app::cancellation::AgentCancellation;
use crate::app::model_streaming::{ModelStreamOutcome, ModelStreamingService};
use crate::domain::artifacts::ArtifactId;
use crate::domain::canonical_json::canonical_blake3_hash;
use crate::domain::context::{
    AuthorityLevel, ContextFragment, ContextFragmentContent, ContextFragmentId,
    ContextFragmentPriority, ContextFragmentSource, ContextOrigin, ContextPlan, ContextPlanner,
    ContextProvenance, ContextReferences, ContextSlot, FragmentCachePolicy, FreshnessIdentity,
    ModelIdentity, ModelProfile, ProviderSessionRef, ProviderSessionRole, RetentionClass,
    SemanticClass,
};
use crate::domain::events::EventId;
use crate::domain::model::{
    ContentItem, MessageRole, ModelCapabilityError, ModelEvent, ModelRequestRequirements,
};
use crate::domain::request_audit::{
    RequestAuditAttempt, RequestAuditDelivery, RequestAuditManifest, RequestAuditRunContext,
};
use crate::domain::tools::{DeferredActivationSet, ResolvedToolPlan, ToolExposurePlan, ToolSpec};
use crate::domain::vision::{
    VISION_FACT_SCHEMA_VERSION, VisionAnalysisFailure, VisionObservation, VisionObservationId,
    VisionRegion,
};
use crate::ports::artifact::ArtifactStore;
use crate::ports::model::{ModelProvider, ModelRequest};

const VISION_INSTRUCTIONS: &str = "Analyze the supplied local image for vulnerability research. Return only one JSON object, without Markdown or commentary. The object must contain exactly these fields: schema_version (the integer 1), summary (string), extracted_text (string or null), regions (array), security_relevant_clues (array of strings), and uncertainty (array of strings). Every regions item must contain exactly label (string), x (non-negative integer), y (non-negative integer), width (positive integer), and height (positive integer); do not add description or any other field. Use an empty array when no bounded region or clue is available. Do not call tools and do not claim a verified finding.";

/// All caller-owned identities for a single targeted image analysis.
#[derive(Clone, Debug)]
pub struct VisionAnalysisInput {
    pub observation_id: VisionObservationId,
    pub route_event_id: EventId,
    pub source_artifact_id: ArtifactId,
    pub delivery_artifact_id: ArtifactId,
    pub derived_artifact_ids: Vec<ArtifactId>,
    pub task: String,
    pub model: ModelIdentity,
    pub provider_profile: ModelProfile,
    pub capability_hash: String,
    pub provider_session: ProviderSessionRef,
}

impl VisionAnalysisInput {
    fn validate(&self) -> Result<(), VisionAnalysisError> {
        for (field, value) in [
            ("observation_id", self.observation_id.as_str()),
            ("task", self.task.as_str()),
            ("capability_hash", self.capability_hash.as_str()),
            ("route_event_id", self.route_event_id.as_str()),
            ("source_artifact_id", self.source_artifact_id.as_str()),
            ("delivery_artifact_id", self.delivery_artifact_id.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(VisionAnalysisError::RequiredInput { field });
            }
        }
        if self.provider_session.role != ProviderSessionRole::Vision {
            return Err(VisionAnalysisError::VisionProviderSessionRequired);
        }
        for (index, artifact) in self.derived_artifact_ids.iter().enumerate() {
            if artifact.as_str().trim().is_empty() {
                return Err(VisionAnalysisError::RequiredInput {
                    field: "derived_artifact_ids",
                });
            }
            if self.derived_artifact_ids[..index].contains(artifact) {
                return Err(VisionAnalysisError::DuplicateDerivedArtifact);
            }
        }
        if self.derived_artifact_ids.contains(&self.source_artifact_id) {
            return Err(VisionAnalysisError::SourceRepeatedAsDerivative);
        }
        if self.delivery_artifact_id != self.source_artifact_id
            && !self
                .derived_artifact_ids
                .contains(&self.delivery_artifact_id)
        {
            return Err(VisionAnalysisError::DeliveryArtifactNotSelected);
        }
        Ok(())
    }

    fn context_plan(
        &self,
        capabilities: &crate::domain::model::ModelCapabilities,
    ) -> Result<ContextPlan, VisionAnalysisError> {
        let mut artifact_refs = vec![self.source_artifact_id.clone()];
        if !artifact_refs.contains(&self.delivery_artifact_id) {
            artifact_refs.push(self.delivery_artifact_id.clone());
        }
        let image_provenance = ContextProvenance::new(
            ContextSlot::UserInput,
            ContextOrigin::ArtifactProjection,
            SemanticClass::AuthoritativeReference,
            AuthorityLevel::FactReference,
            RetentionClass::Stable,
            FreshnessIdentity::new(self.delivery_artifact_id.as_str())
                .map_err(context_error)?
                .with_producer_version("vision-analysis:1"),
            ContextReferences::new(
                Vec::new(),
                artifact_refs,
                Vec::new(),
                vec![self.route_event_id.clone()],
            )
            .map_err(context_error)?,
        )
        .map_err(context_error)?;
        let input_fragment = ContextFragment::new(
            ContextFragmentId::new(format!("vision:input:{}", self.observation_id.as_str()))
                .map_err(context_error)?,
            ContextFragmentSource::UserInput,
            ContextFragmentPriority::Required,
            FragmentCachePolicy::DoNotCache,
            ContextFragmentContent::ContentItems {
                items: vec![
                    ContentItem::Text {
                        role: MessageRole::User,
                        text: self.task.clone(),
                    },
                    ContentItem::ImageArtifact {
                        role: MessageRole::User,
                        artifact_id: self.delivery_artifact_id.clone(),
                    },
                ],
            },
        )
        .and_then(|fragment| fragment.with_content_item_provenance(1, image_provenance))
        .map_err(context_error)?;
        let fragments = vec![
            ContextFragment::new(
                ContextFragmentId::new("vision:instructions:v1").map_err(context_error)?,
                ContextFragmentSource::BasePrompt,
                ContextFragmentPriority::Required,
                FragmentCachePolicy::ContentAddressed,
                ContextFragmentContent::Instructions {
                    text: VISION_INSTRUCTIONS.to_owned(),
                },
            )
            .map_err(context_error)?,
            input_fragment,
        ];
        let planner = ContextPlanner::current();
        let budget = planner
            .default_budget(capabilities)
            .map_err(context_error)?;
        planner
            .compile(
                fragments,
                budget,
                capabilities,
                Vec::new(),
                ModelRequestRequirements {
                    structured_output: true,
                    ..ModelRequestRequirements::default()
                },
                None,
            )
            .map_err(context_error)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VisionAnalysisOutput {
    pub observation: VisionObservation,
}

/// A fixed service with no History, tools, continuation, Memory or Research.
pub struct VisionAnalysisService<'provider, 'blobs, Provider: ?Sized, Blobs: ?Sized> {
    provider: &'provider Provider,
    blobs: &'blobs Blobs,
}

impl<'provider, 'blobs, Provider, Blobs> VisionAnalysisService<'provider, 'blobs, Provider, Blobs>
where
    Provider: ModelProvider + ?Sized,
    Blobs: ArtifactStore + ?Sized,
    Blobs::Error: std::fmt::Debug,
{
    pub fn new(provider: &'provider Provider, blobs: &'blobs Blobs) -> Self {
        Self { provider, blobs }
    }

    /// Audits one exact full request before the sole Provider call. Every
    /// failure after that audit carries a terminal failure fact; no fallback
    /// or continuation is attempted.
    pub async fn analyze_image<Record, RecordError>(
        &self,
        input: VisionAnalysisInput,
        audit_context: &RequestAuditRunContext,
        cancellation: AgentCancellation,
        mut record_audit: Record,
    ) -> Result<VisionAnalysisOutput, VisionAnalysisError>
    where
        Record: FnMut(RequestAuditAttempt) -> Result<RequestAuditManifest, RecordError>,
        RecordError: Display,
    {
        input.validate()?;
        let task_hash =
            canonical_blake3_hash(&input.task).map_err(|error| VisionAnalysisError::TaskHash {
                message: error.to_string(),
            })?;
        let capabilities = self.provider.capabilities();
        let actual_capability_hash = capabilities.semantic_hash();
        if actual_capability_hash != input.capability_hash {
            return Err(VisionAnalysisError::CapabilityIdentityMismatch);
        }
        let context_plan = input.context_plan(&capabilities)?;
        let model_input = context_plan.model_input();
        capabilities.validate_input(&model_input)?;
        let no_tool_plan = ResolvedToolPlan::resolve(
            Vec::<ToolSpec>::new(),
            &ToolExposurePlan::default(),
            &DeferredActivationSet::default(),
        )
        .map_err(|error| VisionAnalysisError::AuditShape {
            message: error.to_string(),
        })?;
        let attempt = RequestAuditAttempt::new(
            audit_context,
            input.model.clone(),
            input.provider_profile.clone(),
            capabilities,
            model_input.clone(),
            model_input.clone(),
            RequestAuditDelivery::full(
                crate::domain::context::BaselineInvalidationReason::NoBaseline,
            ),
        )
        .and_then(|attempt| attempt.bind_scoped_no_tool_context(&context_plan, &no_tool_plan))
        .map_err(|error| VisionAnalysisError::AuditShape {
            message: error.to_string(),
        })?;
        let audit = record_audit(attempt).map_err(|error| VisionAnalysisError::AuditCommit {
            message: error.to_string(),
        })?;
        let request_input_hash = audit
            .full_logical_input()
            .semantic_hash()
            .as_str()
            .to_owned();
        if audit.model() != &input.model
            || audit.profile() != &input.provider_profile
            || audit.capability_mapping().semantic_hash().as_str() != input.capability_hash
        {
            return Err(attempt_failed(
                &input,
                &task_hash,
                audit.audit_event_id(),
                &request_input_hash,
                "audit_identity_mismatch",
                VisionAnalysisError::AuditIdentityMismatch,
            ));
        }
        let outcome = ModelStreamingService::new(self.provider)
            .consume(ModelRequest::full(model_input), cancellation)
            .await
            .map_err(|error| {
                attempt_failed(
                    &input,
                    &task_hash,
                    audit.audit_event_id(),
                    &request_input_hash,
                    "provider_failure",
                    error,
                )
            })?;
        let ModelStreamOutcome::Completed { events, .. } = outcome else {
            return Err(attempt_failed(
                &input,
                &task_hash,
                audit.audit_event_id(),
                &request_input_hash,
                "cancelled",
                VisionAnalysisError::Cancelled,
            ));
        };
        let raw = collect_json_response(events).map_err(|error| {
            attempt_failed(
                &input,
                &task_hash,
                audit.audit_event_id(),
                &request_input_hash,
                "response_invalid",
                error,
            )
        })?;
        let reply = decode_model_reply(&raw).map_err(|error| {
            attempt_failed(
                &input,
                &task_hash,
                audit.audit_event_id(),
                &request_input_hash,
                "model_reply_invalid",
                error,
            )
        })?;
        let stored = self
            .blobs
            .publish_bytes(raw.as_bytes())
            .map_err(|error| VisionAnalysisError::RawObservationBlob {
                message: format!("{error:?}"),
            })
            .map_err(|error| {
                attempt_failed(
                    &input,
                    &task_hash,
                    audit.audit_event_id(),
                    &request_input_hash,
                    "raw_blob_publish_failed",
                    error,
                )
            })?;
        let observation = VisionObservation {
            schema_version: VISION_FACT_SCHEMA_VERSION,
            id: input.observation_id.clone(),
            route_event_id: input.route_event_id.clone(),
            source_artifact_id: input.source_artifact_id.clone(),
            derived_artifact_ids: input.derived_artifact_ids.clone(),
            task_hash,
            summary: reply.summary,
            extracted_text: reply.extracted_text,
            regions: reply.regions,
            security_relevant_clues: reply.security_relevant_clues,
            uncertainty: reply.uncertainty,
            provider_profile: input.provider_profile.clone(),
            capability_hash: input.capability_hash.clone(),
            provider_session: input.provider_session.clone(),
            request_audit_event_id: audit.audit_event_id().clone(),
            request_input_hash,
            raw_observation_blob_id: stored.blob_id,
        };
        observation
            .validate()
            .map_err(|error| VisionAnalysisError::Observation {
                message: error.to_string(),
            })
            .map_err(|error| {
                attempt_failed(
                    &input,
                    &observation.task_hash,
                    audit.audit_event_id(),
                    &observation.request_input_hash,
                    "observation_invalid",
                    error,
                )
            })?;
        Ok(VisionAnalysisOutput { observation })
    }
}

fn collect_json_response(events: Vec<ModelEvent>) -> Result<String, VisionAnalysisError> {
    let mut response = String::new();
    for event in events {
        match event {
            ModelEvent::TextDelta { text } => response.push_str(&text),
            ModelEvent::ReasoningDelta { .. } => {}
            ModelEvent::ReasoningItem { .. } => {}
            ModelEvent::Usage { .. } => {}
            ModelEvent::ToolCall { .. } => return Err(VisionAnalysisError::ToolCallRejected),
        }
    }
    if response.trim().is_empty() {
        return Err(VisionAnalysisError::EmptyResponse);
    }
    Ok(response)
}

fn attempt_failed(
    input: &VisionAnalysisInput,
    task_hash: &str,
    request_audit_event_id: &EventId,
    request_input_hash: &str,
    error_code: &str,
    error: impl Display,
) -> VisionAnalysisError {
    VisionAnalysisError::AttemptFailed {
        failure: Box::new(VisionAnalysisFailure {
            schema_version: VISION_FACT_SCHEMA_VERSION,
            route_event_id: input.route_event_id.clone(),
            source_artifact_id: input.source_artifact_id.clone(),
            task_hash: task_hash.to_owned(),
            provider_profile: input.provider_profile.clone(),
            capability_hash: input.capability_hash.clone(),
            provider_session: input.provider_session.clone(),
            request_audit_event_id: request_audit_event_id.clone(),
            request_input_hash: request_input_hash.to_owned(),
            error_code: error_code.to_owned(),
            message: error.to_string(),
        }),
    }
}

#[derive(Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct VisionModelReply {
    schema_version: u16,
    pub(super) summary: String,
    #[serde(default)]
    pub(super) extracted_text: Option<String>,
    #[serde(default)]
    pub(super) regions: Vec<VisionRegion>,
    #[serde(default)]
    pub(super) security_relevant_clues: Vec<String>,
    #[serde(default)]
    pub(super) uncertainty: Vec<String>,
}

impl VisionModelReply {
    fn validate(&self) -> Result<(), VisionAnalysisError> {
        if self.schema_version != VISION_FACT_SCHEMA_VERSION {
            return Err(VisionAnalysisError::ModelReplySchema {
                received: self.schema_version,
            });
        }
        if self.summary.trim().is_empty() {
            return Err(VisionAnalysisError::ModelReplySummaryRequired);
        }
        Ok(())
    }

    pub(super) fn matches_observation(&self, observation: &VisionObservation) -> bool {
        self.schema_version == observation.schema_version
            && self.summary == observation.summary
            && self.extracted_text == observation.extracted_text
            && self.regions == observation.regions
            && self.security_relevant_clues == observation.security_relevant_clues
            && self.uncertainty == observation.uncertainty
    }
}

pub(super) fn decode_model_reply(raw: &str) -> Result<VisionModelReply, VisionAnalysisError> {
    let reply: VisionModelReply =
        serde_json::from_str(raw).map_err(|error| VisionAnalysisError::ModelReplyDecode {
            message: error.to_string(),
        })?;
    reply.validate()?;
    Ok(reply)
}

fn context_error(error: impl Display) -> VisionAnalysisError {
    VisionAnalysisError::Context {
        message: error.to_string(),
    }
}

#[derive(Debug, Error)]
pub enum VisionAnalysisError {
    #[error("Vision input `{field}` must not be empty")]
    RequiredInput { field: &'static str },
    #[error("Vision analysis requires a Vision ProviderSession")]
    VisionProviderSessionRequired,
    #[error("Vision derived Artifact IDs must be unique")]
    DuplicateDerivedArtifact,
    #[error("the Vision source Artifact cannot also be a derived Artifact")]
    SourceRepeatedAsDerivative,
    #[error("the Vision delivery Artifact must be the source or one selected derivative")]
    DeliveryArtifactNotSelected,
    #[error("the selected Vision Provider capability identity does not match the request")]
    CapabilityIdentityMismatch,
    #[error("Vision Context cannot be compiled through generic-v1: {message}")]
    Context { message: String },
    #[error(transparent)]
    Capability(#[from] ModelCapabilityError),
    #[error("failed to calculate canonical Vision task hash: {message}")]
    TaskHash { message: String },
    #[error("Vision request cannot be represented for RequestAudit: {message}")]
    AuditShape { message: String },
    #[error("Vision RequestAudit commit failed: {message}")]
    AuditCommit { message: String },
    #[error("Vision RequestAudit identity does not match the selected Vision provider")]
    AuditIdentityMismatch,
    #[error("Vision analysis was cancelled before a completed response")]
    Cancelled,
    #[error("Vision model emitted a Tool Call despite the fixed no-tool contract")]
    ToolCallRejected,
    #[error("Vision model returned no structured text")]
    EmptyResponse,
    #[error("Vision model reply is not the fixed JSON object: {message}")]
    ModelReplyDecode { message: String },
    #[error("Vision model reply uses unsupported schema version {received}")]
    ModelReplySchema { received: u16 },
    #[error("Vision model reply summary must not be empty")]
    ModelReplySummaryRequired,
    #[error("failed to publish raw Vision observation Blob: {message}")]
    RawObservationBlob { message: String },
    #[error("Vision observation is invalid: {message}")]
    Observation { message: String },
    #[error(
        "Vision attempt failed after RequestAudit Event `{}`: {}",
        failure.request_audit_event_id,
        failure.message
    )]
    AttemptFailed { failure: Box<VisionAnalysisFailure> },
}
