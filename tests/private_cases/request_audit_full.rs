use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

use crate::adapters::mock_model::{MockModelFixture, MockModelProvider};
use crate::adapters::tools::ToolRegistry;
use crate::app::agent_loop::AgentLoop;
use crate::app::cancellation::AgentCancellation;
use crate::domain::artifacts::BlobId;
use crate::domain::events::{EventId, StreamId};
use crate::domain::model::{CapabilityMappingVersion, ContentItem, ModelCapabilities, ModelEvent};
use crate::domain::project::ProjectId;
use crate::domain::request_audit::{
    PublishedAuditBlob, RequestAdapterVersion, RequestAuditBinding, RequestAuditManifest,
    RequestAuditReconstructionError, RequestAuditRunContext,
};
use crate::ports::model::{ModelResponseCompletion, ProviderResponseId};

#[tokio::test]
async fn full_logical_reconstruction_checks_components_map_and_legacy_limits() {
    let capabilities = ModelCapabilities {
        text_input: true,
        image_input: false,
        streaming: true,
        tool_call: true,
        structured_output: true,
        incremental_input: true,
        usage_reporting: false,
        max_output_tokens: None,
        reasoning_effort: None,
        context_window: 32_768,
        capability_mapping_version: CapabilityMappingVersion::new(1),
    };
    let provider = MockModelProvider::new(MockModelFixture {
        capabilities,
        events: vec![ModelEvent::TextDelta {
            text: "answer".into(),
        }],
        completion: ModelResponseCompletion::new(ProviderResponseId::new("response")),
    });
    let tools = ToolRegistry::for_project(".");
    let mut agent = AgentLoop::new(&provider, &tools);
    let context = RequestAuditRunContext::new(
        RequestAuditBinding::new(
            ProjectId::new("audit-full"),
            StreamId::new("session:audit-full"),
        )
        .unwrap(),
        RequestAdapterVersion::new("fixture").unwrap(),
    );
    let mut blobs = BTreeMap::<BlobId, Vec<u8>>::new();
    let mut manifests = Vec::new();
    {
        let mut record = |attempt: crate::domain::request_audit::RequestAuditAttempt| {
            let rendered = attempt.render().unwrap();
            let published = rendered
                .component_bytes()
                .map(|bytes| {
                    let id = BlobId::new(format!("{:x}", Sha256::digest(bytes))).unwrap();
                    blobs.insert(id.clone(), bytes.to_vec());
                    PublishedAuditBlob {
                        blob_id: id,
                        size_bytes: bytes.len() as u64,
                    }
                })
                .collect::<Vec<_>>();
            manifests.push(
                rendered
                    .materialize(
                        EventId::new(format!("audit-{}", manifests.len())),
                        published,
                    )
                    .unwrap(),
            );
            Ok(())
        };
        agent
            .run_turn_audited(
                "first task",
                AgentCancellation::new(),
                &context,
                &mut record,
            )
            .await
            .unwrap();
        agent
            .run_turn_audited(
                "second task",
                AgentCancellation::new(),
                &context,
                &mut record,
            )
            .await
            .unwrap();
    }
    assert_eq!(manifests.len(), 2);
    assert_eq!(manifests[0].schema_version(), 4);
    let first_encoded = serde_json::to_value(&manifests[0]).unwrap();
    assert_eq!(
        first_encoded["components"],
        first_encoded["full_logical_components"]
    );
    assert_eq!(
        manifests[1].delivery().mode(),
        crate::domain::request_audit::RequestAuditDeliveryMode::Incremental
    );
    let read = |id: &BlobId| blobs.get(id).cloned().ok_or(());
    let full = manifests[1]
        .reconstruct_full_logical(read)
        .unwrap()
        .unwrap();
    assert_eq!(full.audit_event_id(), manifests[1].audit_event_id());
    full.delivery_map().validate_against(full.input()).unwrap();
    assert!(
        full.input()
            .content_items
            .contains(&ContentItem::user_text("first task"))
    );
    assert!(
        full.input()
            .content_items
            .contains(&ContentItem::user_text("second task"))
    );
    let delivered = manifests[1].reconstruct(read).unwrap();
    assert_ne!(full.input(), &delivered);
    let history_before = agent.history().items().to_vec();
    let original = serde_json::to_value(&manifests[1]).unwrap();
    assert_eq!(serde_json::to_value(&manifests[1]).unwrap(), original);

    let legacy = |index: usize| {
        let mut value = serde_json::to_value(&manifests[index]).unwrap();
        value["schema_version"] = 3.into();
        value
            .as_object_mut()
            .unwrap()
            .remove("full_logical_components");
        value
            .as_object_mut()
            .unwrap()
            .remove("full_context_delivery_map");
        serde_json::from_value::<RequestAuditManifest>(value).unwrap()
    };
    assert!(legacy(0).reconstruct_full_logical(read).unwrap().is_some());
    assert!(legacy(1).reconstruct_full_logical(read).unwrap().is_none());

    let mut missing = blobs.clone();
    let full_refs = original["full_logical_components"].as_array().unwrap();
    let first_full_id = BlobId::new(full_refs[0]["blob_id"].as_str().unwrap()).unwrap();
    missing.remove(&first_full_id);
    assert!(matches!(
        manifests[1].reconstruct_full_logical(|id| missing.get(id).cloned().ok_or(())),
        Err(RequestAuditReconstructionError::Blob(()))
    ));
    let mut corrupt = blobs.clone();
    corrupt.insert(first_full_id, b"corrupt".to_vec());
    assert!(matches!(
        manifests[1].reconstruct_full_logical(|id| corrupt.get(id).cloned().ok_or(())),
        Err(RequestAuditReconstructionError::Audit(_))
    ));

    let mut wrong_full_hash = original.clone();
    wrong_full_hash["full_logical_input"]["semantic_hash"] = "b".repeat(64).into();
    let wrong_full_hash: RequestAuditManifest = serde_json::from_value(wrong_full_hash).unwrap();
    assert!(matches!(
        wrong_full_hash.reconstruct_full_logical(read),
        Err(RequestAuditReconstructionError::Audit(
            crate::domain::request_audit::RequestAuditError::FullLogicalInputHashMismatch
        ))
    ));

    let mut tampered = original;
    tampered["full_context_delivery_map"]["content_items"][0]["content_hash"] =
        "a".repeat(64).into();
    let tampered: RequestAuditManifest = serde_json::from_value(tampered).unwrap();
    assert!(matches!(
        tampered.reconstruct_full_logical(read),
        Err(RequestAuditReconstructionError::Audit(_))
    ));
    assert_eq!(agent.history().items(), history_before);
}
