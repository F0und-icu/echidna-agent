//! Construct the historical v5 wire shape without adding a production v5 writer.

use crate::domain::events::EventId;
use crate::domain::session::{
    CompactionModelSample, CompactionValidationContext, NarrativeSummary, SessionCompaction,
    SessionSourceIdentity, SessionState, SessionSummaryDraft,
};

pub(super) fn historical_sample(
    source: &SessionSourceIdentity,
    audit: EventId,
    narrative: NarrativeSummary,
) -> CompactionModelSample {
    serde_json::from_value(serde_json::json!({
        "schema_version": 1,
        "source": source,
        "request_audit_event_id": audit,
        "provider_session": source.compaction_provider_session(),
        "narrative": narrative,
        "byte_limit": 8192,
    }))
    .unwrap()
}

pub(super) fn historical_audit(
    source: &SessionSourceIdentity,
    id: EventId,
    context: &crate::domain::request_audit::RequestAuditRunContext,
    capabilities: crate::domain::model::ModelCapabilities,
) -> crate::domain::request_audit::RequestAuditManifest {
    use crate::domain::context::{
        BaselineInvalidationReason, ModelIdentity, ModelProfile, TurnContext, TurnId,
    };
    use crate::domain::request_audit::{
        PublishedAuditBlob, RequestAuditAttempt, RequestAuditDelivery,
    };
    use crate::domain::sampling::{SamplingStepPlan, TurnConfigurationSnapshot};
    use crate::domain::tools::{DeferredActivationSet, ResolvedToolPlan, ToolExposurePlan};
    use sha2::{Digest, Sha256};
    let capture = crate::domain::session::CompactionContextCapture::new(
        capabilities.clone(),
        crate::domain::context::ContextPlanner::current().budget_policy(),
        1,
        vec![],
    );
    let plan = crate::app::agent_loop::compile_captured_compaction_source(
        &capture,
        source,
        "historical source",
        &[],
        4000,
    )
    .unwrap();
    let scope = source.compaction_provider_session();
    let step_id = scope.id.to_string();
    let model = ModelIdentity::new("fixture", "fixture");
    let profile = ModelProfile::new("fixture", 0);
    let turn = TurnContext::new(
        TurnId::new(&step_id),
        model.clone(),
        profile.clone(),
        scope,
        capabilities.context_window,
        &[],
    )
    .unwrap();
    let tools = ResolvedToolPlan::resolve(
        Vec::new(),
        &ToolExposurePlan::default(),
        &DeferredActivationSet::default(),
    )
    .unwrap();
    let step = SamplingStepPlan::new(
        step_id,
        TurnConfigurationSnapshot::new(turn, None, None, None),
        plan,
        tools,
        capabilities.clone(),
    )
    .unwrap();
    let input = step.context_plan().model_input();
    let rendered = RequestAuditAttempt::new(
        context,
        model,
        profile,
        capabilities,
        input.clone(),
        input,
        RequestAuditDelivery::full(BaselineInvalidationReason::NoBaseline),
    )
    .unwrap()
    .bind_sampling_step(&step, step.context_plan().delivery_map().clone())
    .unwrap()
    .render()
    .unwrap();
    let blobs: Vec<PublishedAuditBlob> = rendered
        .component_bytes()
        .map(|bytes| PublishedAuditBlob {
            blob_id: crate::domain::artifacts::BlobId::new(format!("{:x}", Sha256::digest(bytes)))
                .unwrap(),
            size_bytes: bytes.len() as u64,
        })
        .collect();
    rendered.materialize(id, blobs).unwrap()
}

pub(super) fn sample_plan(
    source: &SessionSourceIdentity,
    output_limit: usize,
) -> crate::domain::session::CompactionSourcePlan {
    use crate::domain::session::{
        CompactionSourceEntry, CompactionSourceNode, CompactionSourceNodeInput,
        CompactionSourcePlan, CompactionSourceRange,
    };
    let content = "fixture source".to_owned();
    let entry: CompactionSourceEntry = serde_json::from_value(serde_json::json!({
        "source": {"kind": "history", "id": "fixture-event"},
        "content_hash": crate::domain::canonical_json::canonical_blake3_hash(&content).unwrap(),
        "byte_len": content.len(), "protected_exact": false
    }))
    .unwrap();
    let contents = vec![content];
    let node = CompactionSourceNode {
        id: "root".into(),
        input: CompactionSourceNodeInput::Source(vec![CompactionSourceRange {
            entry: 0,
            start: 0,
            end: contents[0].len(),
        }]),
        is_root: true,
        output_limit,
        template_hash: None,
    };
    CompactionSourcePlan::new(source.clone(), vec![entry], vec![node], &contents)
        .unwrap()
        .with_node_templates(
            std::collections::BTreeMap::from([("root".into(), "a".repeat(64))]),
            &contents,
        )
        .unwrap()
}

pub(super) fn legacy_v5_candidate(
    state: &SessionState,
    draft: SessionSummaryDraft,
    context: &CompactionValidationContext<'_>,
    sample_id: EventId,
    narrative: NarrativeSummary,
) -> SessionCompaction {
    let candidate = state.next_compaction(draft.clone(), 0, context).unwrap();
    let mut summary = serde_json::json!({
        "schema_version": 5,
        "revision": candidate.summary().revision(),
        "fingerprint": "",
        "byte_limit": 8192,
        "task_references": draft.task_references().unwrap(),
        "evidence_spine": draft.evidence_spine(),
        "narrative": narrative,
        "model_sample_event_ref": sample_id,
    });
    summary["fingerprint"] =
        serde_json::json!(crate::domain::canonical_json::canonical_blake3_hash(&summary).unwrap());
    let mut compaction = serde_json::to_value(candidate).unwrap();
    compaction["schema_version"] = serde_json::json!(5);
    compaction["summary"] = summary;
    let candidate: SessionCompaction = serde_json::from_value(compaction).unwrap();
    assert_eq!(candidate.summary().historical_byte_limit(), Some(8192));
    candidate
}
