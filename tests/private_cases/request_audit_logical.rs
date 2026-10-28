use std::collections::BTreeMap;

use crate::domain::artifacts::BlobId;
use crate::domain::context::{
    BaselineInvalidationReason, ContextBudget, ContextFragment, ContextFragmentContent,
    ContextFragmentId, ContextFragmentPriority, ContextFragmentSource, ContextPlanner,
    FragmentCachePolicy, ModelIdentity, ModelProfile,
};
use crate::domain::events::{EventId, StreamId};
use crate::domain::model::{
    CapabilityMappingVersion, ContentItem, ModelCapabilities, ModelRequestRequirements,
};
use crate::domain::project::ProjectId;
use crate::domain::request_audit::{
    PublishedAuditBlob, RequestAdapterVersion, RequestAuditAttempt, RequestAuditBinding,
    RequestAuditDelivery, RequestAuditManifest, RequestAuditRunContext,
};
use crate::domain::tools::{DeferredActivationSet, ResolvedToolPlan, ToolExposurePlan};

#[test]
fn logical_reconstruction_uses_exact_audited_blobs_and_keeps_legacy_full_proof() {
    let capabilities = ModelCapabilities {
        text_input: true,
        image_input: false,
        streaming: true,
        tool_call: true,
        structured_output: false,
        incremental_input: true,
        usage_reporting: false,
        reasoning_effort: None,
        max_output_tokens: None,
        context_window: 32768,
        capability_mapping_version: CapabilityMappingVersion::new(1),
    };
    let fragment = ContextFragment::new(
        ContextFragmentId::new("input").unwrap(),
        ContextFragmentSource::UserInput,
        ContextFragmentPriority::Required,
        FragmentCachePolicy::DoNotCache,
        ContextFragmentContent::ContentItems {
            items: vec![ContentItem::user_text("exact input")],
        },
    )
    .unwrap();
    let plan = ContextPlanner::current()
        .compile(
            vec![fragment],
            ContextBudget::new(32768, 0, 0).unwrap(),
            &capabilities,
            Vec::new(),
            ModelRequestRequirements::default(),
            None,
        )
        .unwrap();
    let no_tools = ResolvedToolPlan::resolve(
        Vec::new(),
        &ToolExposurePlan::default(),
        &DeferredActivationSet::default(),
    )
    .unwrap();
    let context = RequestAuditRunContext::new(
        RequestAuditBinding::new(ProjectId::new("project"), StreamId::new("session:proof"))
            .unwrap(),
        RequestAdapterVersion::new("proof:1").unwrap(),
    );
    let attempt = RequestAuditAttempt::new(
        &context,
        ModelIdentity::new("provider", "model"),
        ModelProfile::new("default", 0),
        capabilities,
        plan.model_input(),
        plan.model_input(),
        RequestAuditDelivery::full(BaselineInvalidationReason::NoBaseline),
    )
    .unwrap()
    .bind_scoped_no_tool_context(&plan, &no_tools)
    .unwrap();
    let rendered = attempt.render().unwrap();
    let mut bytes = BTreeMap::<BlobId, Vec<u8>>::new();
    let mut published = Vec::new();
    for (index, content) in rendered.component_bytes().enumerate() {
        let blob_id = BlobId::new(format!("{:064x}", index + 1)).unwrap();
        bytes.insert(blob_id.clone(), content.to_vec());
        published.push(PublishedAuditBlob {
            blob_id,
            size_bytes: content.len() as u64,
        });
    }
    let manifest = rendered
        .materialize(EventId::new("audit"), published)
        .unwrap();
    assert_eq!(manifest.schema_version(), 4);
    let proof = manifest
        .reconstruct_full_logical(|id| bytes.get(id).cloned().ok_or(()))
        .unwrap()
        .unwrap();
    assert_eq!(proof.audit_event_id(), manifest.audit_event_id());
    assert_eq!(proof.input(), &plan.model_input());
    assert_eq!(proof.delivery_map(), plan.delivery_map());

    // Every separately retained full component is checked against its original
    // size/hash; the delivered half cannot fill a missing logical component.
    let encoded = serde_json::to_value(&manifest).unwrap();
    for component in encoded["full_logical_components"].as_array().unwrap() {
        let id: BlobId = serde_json::from_value(component["blob_id"].clone()).unwrap();
        let mut corrupted = bytes.clone();
        corrupted.insert(id.clone(), vec![b'x'; bytes[&id].len().max(1)]);
        assert!(
            manifest
                .reconstruct_full_logical(|id| corrupted.get(id).cloned().ok_or(()))
                .is_err()
        );
        corrupted.remove(&id);
        assert!(
            manifest
                .reconstruct_full_logical(|id| corrupted.get(id).cloned().ok_or(()))
                .is_err()
        );
    }
    let mut old = encoded;
    let object = old.as_object_mut().unwrap();
    object.remove("full_logical_components");
    object.remove("full_context_delivery_map");
    object.insert("schema_version".to_owned(), serde_json::json!(3));
    let old: RequestAuditManifest = serde_json::from_value(old).unwrap();
    assert_eq!(
        old.reconstruct_full_logical(|id| bytes.get(id).cloned().ok_or(()))
            .unwrap()
            .unwrap()
            .input(),
        &plan.model_input()
    );
}
