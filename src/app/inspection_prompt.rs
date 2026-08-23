//! Read-only projection of the most recent committed semantic request.

use serde::Serialize;

use crate::domain::context::BaselineInvalidationReason;
use crate::domain::request_audit::{
    RequestAuditComponentIdentity, RequestAuditComponentReference, RequestAuditComponentSource,
    RequestAuditDeliveryMode, RequestAuditDeliveryReason, RequestAuditManifest,
};

use super::inspection_shared::{
    AuditReferenceStatus, InspectionDiagnostic, diagnostic, estimate_bytes,
};

#[derive(Debug, Serialize)]
pub struct PromptInspectionReport {
    pub command: &'static str,
    pub status: &'static str,
    pub session_id: Option<String>,
    pub request: Option<PromptRequest>,
    pub diagnostics: Vec<InspectionDiagnostic>,
}

#[derive(Debug, Serialize)]
pub struct PromptRequest {
    pub event: EventReference,
    pub manifest_schema_version: u16,
    pub adapter_version: String,
    pub capability_mapping: InputIdentityView,
    pub delivery: DeliveryView,
    pub full_logical_input: InputIdentityView,
    pub delivered_input: InputIdentityView,
    pub components: Vec<PromptComponent>,
    pub estimated_tokens: TokenEstimate,
    pub provider_usage: &'static str,
}

#[derive(Debug, Serialize)]
pub struct EventReference {
    pub event_id: String,
    pub stream_id: String,
}

#[derive(Debug, Serialize)]
pub struct DeliveryView {
    pub mode: &'static str,
    pub reason: String,
}

#[derive(Debug, Serialize)]
pub struct InputIdentityView {
    pub schema_version: u16,
    pub semantic_hash: String,
}

#[derive(Debug, Serialize)]
pub struct PromptComponent {
    pub position: u32,
    pub identity: ComponentIdentityView,
    pub source: ComponentSourceView,
    pub blob_id: String,
    pub size_bytes: u64,
    pub semantic_hash: String,
    pub estimated_tokens: u64,
}

#[derive(Debug, Serialize)]
pub struct ComponentIdentityView {
    pub kind: &'static str,
    pub index: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct ComponentSourceView {
    pub kind: &'static str,
    pub schema_version: u16,
}

#[derive(Debug, Serialize)]
pub struct TokenEstimate {
    pub estimator: &'static str,
    pub total: u64,
    pub components: usize,
}

pub fn prompt(
    session_id: Option<&str>,
    manifest: Option<&RequestAuditManifest>,
    audit_status: AuditReferenceStatus,
) -> PromptInspectionReport {
    let Some(manifest) = manifest else {
        return PromptInspectionReport {
            command: "prompt inspect",
            status: "no_request",
            session_id: session_id.map(str::to_owned),
            request: None,
            diagnostics: vec![diagnostic(
                "NO_COMMITTED_REQUEST",
                "info",
                "no committed RequestAudit manifest is available for this Session",
            )],
        };
    };
    let mut diagnostics = Vec::new();
    let status = match (manifest.validate(), audit_status) {
        (Ok(()), AuditReferenceStatus::Verified) => "ok",
        _ => {
            diagnostics.push(diagnostic(
                "CORRUPT_REQUEST_AUDIT_REFERENCE",
                "error",
                "the committed RequestAudit manifest or one referenced Blob is invalid",
            ));
            "corrupt_reference"
        }
    };
    PromptInspectionReport {
        command: "prompt inspect",
        status,
        session_id: session_id.map(str::to_owned),
        request: Some(prompt_request(manifest)),
        diagnostics,
    }
}

pub fn uninitialized_prompt() -> PromptInspectionReport {
    PromptInspectionReport {
        command: "prompt inspect",
        status: "uninitialized",
        session_id: None,
        request: None,
        diagnostics: vec![diagnostic(
            "PROJECT_NOT_INITIALIZED",
            "info",
            "no initialized Cyber Agent project was found",
        )],
    }
}

pub fn corrupt_session_prompt(session_id: &str) -> PromptInspectionReport {
    PromptInspectionReport {
        command: "prompt inspect",
        status: "corrupt_session",
        session_id: Some(session_id.to_owned()),
        request: None,
        diagnostics: vec![diagnostic(
            "CORRUPT_SESSION_STREAM",
            "error",
            "the Session Event stream could not be replayed without mutation",
        )],
    }
}

fn prompt_request(manifest: &RequestAuditManifest) -> PromptRequest {
    let components = manifest
        .components()
        .iter()
        .map(prompt_component)
        .collect::<Vec<_>>();
    let estimated_tokens = TokenEstimate {
        estimator: "audit_blob_bytes_div_4_ceiling_v1",
        total: components
            .iter()
            .map(|component| component.estimated_tokens)
            .sum(),
        components: components.len(),
    };
    PromptRequest {
        event: EventReference {
            event_id: manifest.audit_event_id().as_str().to_owned(),
            stream_id: manifest.binding().session_stream_id().as_str().to_owned(),
        },
        manifest_schema_version: manifest.schema_version(),
        adapter_version: manifest.adapter_version().as_str().to_owned(),
        capability_mapping: InputIdentityView {
            schema_version: manifest.capability_mapping().version().get(),
            semantic_hash: manifest
                .capability_mapping()
                .semantic_hash()
                .as_str()
                .to_owned(),
        },
        delivery: DeliveryView {
            mode: match manifest.delivery().mode() {
                RequestAuditDeliveryMode::Full => "full",
                RequestAuditDeliveryMode::Incremental => "incremental",
            },
            reason: delivery_reason(manifest.delivery().reason()),
        },
        full_logical_input: input_identity(manifest.full_logical_input()),
        delivered_input: input_identity(manifest.delivered_input()),
        components,
        estimated_tokens,
        provider_usage: "unavailable",
    }
}

fn prompt_component(component: &RequestAuditComponentReference) -> PromptComponent {
    let (kind, index) = match component.identity() {
        RequestAuditComponentIdentity::Instructions => ("instructions", None),
        RequestAuditComponentIdentity::ContentItem(index) => ("content_item", Some(*index)),
        RequestAuditComponentIdentity::ToolSpec(index) => ("tool_spec", Some(*index)),
        RequestAuditComponentIdentity::RequestRequirements => ("request_requirements", None),
    };
    let source_kind = match component.source() {
        RequestAuditComponentSource::RenderedInstructions => "rendered_instructions",
        RequestAuditComponentSource::RenderedContentItem => "rendered_content_item",
        RequestAuditComponentSource::RenderedToolSpec => "rendered_tool_spec",
        RequestAuditComponentSource::RenderedRequestRequirements => "rendered_request_requirements",
    };
    PromptComponent {
        position: component.position(),
        identity: ComponentIdentityView { kind, index },
        source: ComponentSourceView {
            kind: source_kind,
            schema_version: crate::domain::request_audit::REQUEST_AUDIT_MANIFEST_SCHEMA_VERSION,
        },
        blob_id: component.blob_id().as_str().to_owned(),
        size_bytes: component.size_bytes(),
        semantic_hash: component.semantic_hash().as_str().to_owned(),
        estimated_tokens: estimate_bytes(component.size_bytes()),
    }
}

fn input_identity(
    input: &crate::domain::request_audit::RequestAuditModelInput,
) -> InputIdentityView {
    InputIdentityView {
        schema_version: input.schema_version().get(),
        semantic_hash: input.semantic_hash().as_str().to_owned(),
    }
}

fn delivery_reason(reason: RequestAuditDeliveryReason) -> String {
    match reason {
        RequestAuditDeliveryReason::AppendOnlyCandidate => "append_only_candidate".to_owned(),
        RequestAuditDeliveryReason::Full(reason) => format!("full_{}", baseline_reason(reason)),
    }
}

fn baseline_reason(reason: BaselineInvalidationReason) -> &'static str {
    match reason {
        BaselineInvalidationReason::NoBaseline => "no_baseline",
        BaselineInvalidationReason::ProviderSessionChanged => "provider_session_changed",
        BaselineInvalidationReason::ModelChanged => "model_changed",
        BaselineInvalidationReason::ProfileChanged => "profile_changed",
        BaselineInvalidationReason::ModelInputIrSchemaChanged => "model_input_ir_schema_changed",
        BaselineInvalidationReason::CapabilityMappingChanged => "capability_mapping_changed",
        BaselineInvalidationReason::PromptChanged => "prompt_changed",
        BaselineInvalidationReason::ToolsChanged => "tools_changed",
        BaselineInvalidationReason::OutputContractChanged => "output_contract_changed",
        BaselineInvalidationReason::RequestPropertiesChanged => "request_properties_changed",
        BaselineInvalidationReason::InputNotAppendOnly => "input_not_append_only",
        BaselineInvalidationReason::IncrementalInputUnsupported => "incremental_input_unsupported",
        BaselineInvalidationReason::CompactionReset => "compaction_reset",
        BaselineInvalidationReason::SessionResumed => "session_resumed",
        BaselineInvalidationReason::ProviderRejected => "provider_rejected",
    }
}
