use std::collections::BTreeMap;
use std::sync::Mutex;

use cyber_agent::app::request_audit::{RequestAuditService, RequestAuditServiceError};
use cyber_agent::app::session::{AppendAfterDurableError, SessionProjection};
use cyber_agent::domain::artifacts::BlobId;
use cyber_agent::domain::context::{ProviderSessionId, ProviderSessionRef, ProviderSessionRole};
use cyber_agent::domain::events::{EventEnvelope, EventId, EventSequence, OccurredAt, StreamId};
use cyber_agent::domain::model::{ModelCapabilities, ModelInputIR, ModelRequestRequirements};
use cyber_agent::domain::project::ProjectId;
use cyber_agent::domain::request_audit::{
    RequestAdapterVersion, RequestAuditAttempt, RequestAuditBinding, RequestAuditDelivery,
    RequestAuditError, RequestAuditManifest, RequestAuditReconstructionError,
    RequestAuditRunContext,
};
use cyber_agent::domain::session::SessionEvent;
use cyber_agent::ports::EventSink;
use cyber_agent::ports::artifact::{ArtifactStore, StoredBlob};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const SESSION_STREAM: &str = "session:request-audit-fixture";

#[derive(Clone, Debug, Eq, PartialEq)]
enum BlobFailure {
    Missing,
    Publish,
}

#[derive(Default)]
struct MemoryBlobStore {
    blobs: Mutex<BTreeMap<BlobId, Vec<u8>>>,
    publish_calls: Mutex<usize>,
    fail_publish_call: Mutex<Option<usize>>,
}

impl MemoryBlobStore {
    fn fail_on_publish_call(&self, call: usize) {
        *self.fail_publish_call.lock().unwrap() = Some(call);
    }

    fn blob_count(&self) -> usize {
        self.blobs.lock().unwrap().len()
    }

    fn tamper(&self, blob_id: &BlobId, bytes: &[u8]) {
        self.blobs
            .lock()
            .unwrap()
            .insert(blob_id.clone(), bytes.to_vec());
    }

    fn stored_bytes(&self) -> Vec<Vec<u8>> {
        self.blobs.lock().unwrap().values().cloned().collect()
    }
}

impl ArtifactStore for MemoryBlobStore {
    type Error = BlobFailure;

    fn publish_file(&self, _source: &std::path::Path) -> Result<StoredBlob, Self::Error> {
        unreachable!("RequestAudit publishes owned rendered bytes")
    }

    fn publish_bytes(&self, bytes: &[u8]) -> Result<StoredBlob, Self::Error> {
        let mut calls = self.publish_calls.lock().unwrap();
        *calls += 1;
        if *self.fail_publish_call.lock().unwrap() == Some(*calls) {
            return Err(BlobFailure::Publish);
        }
        let blob_id = BlobId::new(format!("{:x}", Sha256::digest(bytes))).unwrap();
        let mut blobs = self.blobs.lock().unwrap();
        let already_present = blobs.contains_key(&blob_id);
        blobs
            .entry(blob_id.clone())
            .or_insert_with(|| bytes.to_vec());
        Ok(StoredBlob {
            blob_id,
            size_bytes: u64::try_from(bytes.len()).unwrap(),
            already_present,
        })
    }

    fn read_blob(&self, blob_id: &BlobId) -> Result<Vec<u8>, Self::Error> {
        self.blobs
            .lock()
            .unwrap()
            .get(blob_id)
            .cloned()
            .ok_or(BlobFailure::Missing)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum SinkFailure {
    Append,
}

#[derive(Default)]
struct RecordingSink {
    events: Vec<EventEnvelope<Value>>,
    fail_append: bool,
}

impl EventSink<Value> for RecordingSink {
    type Error = SinkFailure;

    fn append_event(&mut self, event: &EventEnvelope<Value>) -> Result<(), Self::Error> {
        if self.fail_append {
            return Err(SinkFailure::Append);
        }
        self.events.push(event.clone());
        Ok(())
    }
}

fn capabilities() -> ModelCapabilities {
    ModelCapabilities {
        text_input: true,
        image_input: false,
        streaming: true,
        tool_call: true,
        structured_output: true,
        incremental_input: true,
        usage_reporting: true,
        max_output_tokens: None,
        context_window: 32_768,
        capability_mapping_version: cyber_agent::domain::model::CapabilityMappingVersion::new(7),
    }
}

fn run_context(project: &str) -> RequestAuditRunContext {
    RequestAuditRunContext::new(
        RequestAuditBinding::new(ProjectId::new(project), StreamId::new(SESSION_STREAM)).unwrap(),
        RequestAdapterVersion::new("fixture-adapter-v1").unwrap(),
    )
}

fn input(instructions: &str, user_text: &str) -> ModelInputIR {
    ModelInputIR::current(
        instructions,
        vec![cyber_agent::domain::model::ContentItem::user_text(
            user_text,
        )],
        ModelRequestRequirements::default(),
    )
}

fn attempt(context: &RequestAuditRunContext, input: ModelInputIR) -> RequestAuditAttempt {
    RequestAuditAttempt::new(
        context,
        cyber_agent::domain::context::ModelIdentity::new("fixture", "model"),
        cyber_agent::domain::context::ModelProfile::new("fixture-profile", 1),
        capabilities(),
        input.clone(),
        input,
        RequestAuditDelivery::full(
            cyber_agent::domain::context::BaselineInvalidationReason::NoBaseline,
        ),
    )
    .unwrap()
}

#[test]
fn full_request_audit_allows_distinct_provider_delivery_only_for_continuation_reasons() {
    let logical_input = input("logical instructions", "logical user input");
    let delivered_input = input("delivered instructions", "logical user input");
    let mut reasoning_replay = logical_input.clone();
    reasoning_replay.content_items.insert(
        0,
        cyber_agent::domain::model::ContentItem::Reasoning {
            text: "completed reasoning".to_owned(),
        },
    );

    assert!(matches!(
        RequestAuditAttempt::new(
            &run_context("project-left"),
            cyber_agent::domain::context::ModelIdentity::new("fixture", "model"),
            cyber_agent::domain::context::ModelProfile::new("fixture-profile", 1),
            capabilities(),
            logical_input.clone(),
            delivered_input.clone(),
            RequestAuditDelivery::full(
                cyber_agent::domain::context::BaselineInvalidationReason::NoBaseline,
            ),
        ),
        Err(RequestAuditError::FullDeliveryInputMismatch)
    ));

    for reason in [
        cyber_agent::domain::context::BaselineInvalidationReason::IncrementalInputUnsupported,
        cyber_agent::domain::context::BaselineInvalidationReason::ProviderRejected,
    ] {
        RequestAuditAttempt::new(
            &run_context("project-left"),
            cyber_agent::domain::context::ModelIdentity::new("fixture", "model"),
            cyber_agent::domain::context::ModelProfile::new("fixture-profile", 1),
            capabilities(),
            logical_input.clone(),
            delivered_input.clone(),
            RequestAuditDelivery::full(reason),
        )
        .expect("continuation-aware Full delivery retains distinct audited input");
    }

    RequestAuditAttempt::new(
        &run_context("project-left"),
        cyber_agent::domain::context::ModelIdentity::new("fixture", "model"),
        cyber_agent::domain::context::ModelProfile::new("fixture-profile", 1),
        capabilities(),
        logical_input,
        reasoning_replay,
        RequestAuditDelivery::full(
            cyber_agent::domain::context::BaselineInvalidationReason::ToolsChanged,
        ),
    )
    .expect("an exact reasoning replay may accompany an otherwise identical Full request");
}

#[test]
fn tool_change_diff_audit_requires_exact_current_request_contract() {
    use cyber_agent::domain::context::BaselineInvalidationReason;
    use cyber_agent::domain::model::ContentItem;
    use cyber_agent::ports::tool::ToolExecutor;
    let tool = cyber_agent::adapters::tools::ToolRegistry::for_project(env!("CARGO_MANIFEST_DIR"))
        .catalog()
        .into_iter()
        .find(|spec| spec.name.as_str() == "read_file")
        .unwrap();
    let logical =
        input("current instructions", "current full projection").with_tool_specs(vec![tool]);
    let mut delivered = logical.clone();
    delivered.content_items = vec![
        ContentItem::user_text("retained full projection"),
        ContentItem::Reasoning {
            text: "completed reasoning".into(),
        },
        ContentItem::user_text("verified state diff"),
    ];
    let check = |candidate: ModelInputIR, reason| {
        RequestAuditAttempt::new(
            &run_context("project-left"),
            cyber_agent::domain::context::ModelIdentity::new("fixture", "model"),
            cyber_agent::domain::context::ModelProfile::new("fixture-profile", 1),
            capabilities(),
            logical.clone(),
            candidate,
            RequestAuditDelivery::full(reason),
        )
    };
    check(delivered.clone(), BaselineInvalidationReason::ToolsChanged).unwrap();
    for change in ["instructions", "tools", "requirements", "reasoning"] {
        let mut candidate = delivered.clone();
        match change {
            "instructions" => candidate.instructions.push_str(" changed"),
            "tools" => candidate.tool_specs[0].version.push_str("changed"),
            "requirements" => {
                candidate.requirements.usage_reporting = !candidate.requirements.usage_reporting
            }
            "reasoning" => candidate
                .content_items
                .retain(|i| !matches!(i, ContentItem::Reasoning { .. })),
            _ => unreachable!(),
        }
        assert!(
            matches!(
                check(candidate, BaselineInvalidationReason::ToolsChanged),
                Err(RequestAuditError::FullDeliveryInputMismatch)
            ),
            "{change}"
        );
    }
    for reason in [
        BaselineInvalidationReason::PromptChanged,
        BaselineInvalidationReason::ModelChanged,
        BaselineInvalidationReason::CapabilityMappingChanged,
        BaselineInvalidationReason::CompactionReset,
    ] {
        assert!(
            matches!(
                check(delivered.clone(), reason),
                Err(RequestAuditError::FullDeliveryInputMismatch)
            ),
            "{reason:?}"
        );
    }
}

fn projection() -> SessionProjection {
    SessionProjection::replay([SessionEvent::Started {
        primary_provider_session: ProviderSessionRef {
            id: ProviderSessionId::new("primary-audit"),
            role: ProviderSessionRole::Primary,
            generation: 1,
        },
    }
    .into_envelope(
        EventId::new("session-started"),
        StreamId::new(SESSION_STREAM),
        EventSequence::new(1),
        OccurredAt::new("2026-08-23T00:00:00Z"),
    )
    .unwrap()])
    .unwrap()
}

fn record(
    service: &RequestAuditService<'_, MemoryBlobStore>,
    input: ModelInputIR,
    projection: &mut SessionProjection,
    sink: &mut RecordingSink,
    event_id: &str,
) -> cyber_agent::domain::request_audit::RequestAuditManifest {
    service
        .record(
            attempt(&run_context("project-left"), input),
            EventId::new(event_id),
            OccurredAt::new("2026-08-23T00:00:01Z"),
            projection,
            sink,
        )
        .unwrap()
}

fn service(blobs: &MemoryBlobStore) -> RequestAuditService<'_, MemoryBlobStore> {
    RequestAuditService::new(blobs, run_context("project-left").binding().clone())
}

#[test]
fn base_revision_does_not_replace_recorded_historical_bytes() {
    let blobs = MemoryBlobStore::default();
    let service = service(&blobs);
    let mut session = projection();
    let mut sink = RecordingSink::default();
    let old = input(
        include_str!("fixtures/prompts/base-v1.md"),
        "historical question",
    );
    let old_manifest = record(&service, old.clone(), &mut session, &mut sink, "base-v1");
    let old2 = input(
        include_str!("fixtures/prompts/base-v2.md"),
        "generation two question",
    );
    let old2_manifest = record(&service, old2.clone(), &mut session, &mut sink, "base-v2");
    let old3 = input(
        include_str!("fixtures/prompts/base-v3.md"),
        "generation three question",
    );
    let old3_manifest = record(&service, old3.clone(), &mut session, &mut sink, "base-v3");
    let old4 = input(
        &format!(
            "{}\n{}",
            include_str!("fixtures/prompts/base-v4.md"),
            include_str!("fixtures/prompts/mapping-v2.md")
        ),
        "generation four question",
    );
    let old4_manifest = record(&service, old4.clone(), &mut session, &mut sink, "base-v4");
    let old_event = sink.events[0].clone();
    let old_blobs = blobs.blobs.lock().unwrap().clone();
    let current = input(
        cyber_agent::domain::research_control::BaseContract.prompt(),
        "current question",
    );
    let current_manifest = record(
        &service,
        current.clone(),
        &mut session,
        &mut sink,
        "base-v5",
    );
    assert_ne!(old_manifest.components(), current_manifest.components());
    assert_eq!(service.reconstruct(&old_manifest).unwrap(), old);
    assert_eq!(service.reconstruct(&old2_manifest).unwrap(), old2);
    assert_eq!(service.reconstruct(&old3_manifest).unwrap(), old3);
    assert_eq!(service.reconstruct(&old4_manifest).unwrap(), old4);
    assert_eq!(service.reconstruct(&current_manifest).unwrap(), current);
    assert_eq!(sink.events[0], old_event);
    for (id, bytes) in old_blobs {
        assert_eq!(blobs.read_blob(&id).unwrap(), bytes);
    }
}

#[test]
fn request_audit_reconstructs_exact_rendered_input_after_source_mutation_and_deduplicates() {
    let blobs = MemoryBlobStore::default();
    let service = service(&blobs);
    let mut session = projection();
    let mut sink = RecordingSink::default();
    let mut source_instructions = "original instructions".to_owned();
    let original = input(&source_instructions, "original user input");
    let manifest = record(
        &service,
        original.clone(),
        &mut session,
        &mut sink,
        "request-audit-one",
    );
    source_instructions.replace_range(.., "changed instructions");

    assert_eq!(service.reconstruct(&manifest).unwrap(), original);
    assert_eq!(
        session.state().request_audits(),
        std::slice::from_ref(&manifest)
    );
    assert_eq!(sink.events.len(), 1);

    let second = record(
        &service,
        original,
        &mut session,
        &mut sink,
        "request-audit-two",
    );
    assert_eq!(manifest.components(), second.components());
    assert_eq!(blobs.blob_count(), manifest.components().len());
    assert!(manifest.components().iter().all(|component| {
        second
            .components()
            .iter()
            .any(|other| other.blob_id() == component.blob_id())
    }));

    let repeated = ModelInputIR::current(
        "repeated instructions",
        vec![
            cyber_agent::domain::model::ContentItem::user_text("same exact item"),
            cyber_agent::domain::model::ContentItem::user_text("same exact item"),
        ],
        ModelRequestRequirements::default(),
    );
    let repeated_manifest = record(
        &service,
        repeated.clone(),
        &mut session,
        &mut sink,
        "request-audit-repeated-content",
    );
    assert_eq!(
        repeated_manifest.components()[1].blob_id(),
        repeated_manifest.components()[2].blob_id(),
        "one Blob may appear at multiple typed positions"
    );
    assert_eq!(service.reconstruct(&repeated_manifest).unwrap(), repeated);
}

#[test]
fn request_audit_rejects_unknown_schema_reordered_duplicate_and_mismatched_content() {
    let blobs = MemoryBlobStore::default();
    let service = service(&blobs);
    let mut session = projection();
    let mut sink = RecordingSink::default();
    let manifest = record(
        &service,
        input("instructions", "user input"),
        &mut session,
        &mut sink,
        "request-audit-strict",
    );
    let encoded = serde_json::to_value(&manifest).unwrap();

    let mut historical_ir_v2 = encoded.clone();
    historical_ir_v2["full_logical_input"]["schema_version"] = json!(2);
    historical_ir_v2["delivered_input"]["schema_version"] = json!(2);
    let historical_ir_v2: RequestAuditManifest = serde_json::from_value(historical_ir_v2).unwrap();
    historical_ir_v2
        .validate()
        .expect("historical ModelInputIR v2 audit identities remain readable");

    let mut historical_manifest_v2 = encoded.clone();
    historical_manifest_v2["schema_version"] = json!(2);
    historical_manifest_v2["prompt_renderer"] = json!({"id": "generic", "version": 1});
    historical_manifest_v2["resolved_tool_plan_fingerprint"] = json!("a".repeat(64));
    let historical_manifest_v2: RequestAuditManifest =
        serde_json::from_value(historical_manifest_v2).unwrap();
    historical_manifest_v2
        .validate()
        .expect("RequestAudit v2 remains readable without a delivery map");
    assert!(historical_manifest_v2.context_delivery_map().is_none());
    assert_eq!(
        service.reconstruct(&historical_manifest_v2).unwrap(),
        input("instructions", "user input")
    );

    let mut incomplete_manifest_v3 = encoded.clone();
    incomplete_manifest_v3["schema_version"] = json!(3);
    incomplete_manifest_v3["prompt_renderer"] = json!({"id": "generic", "version": 1});
    incomplete_manifest_v3["resolved_tool_plan_fingerprint"] = json!("a".repeat(64));
    let incomplete_manifest_v3: RequestAuditManifest =
        serde_json::from_value(incomplete_manifest_v3).unwrap();
    assert!(matches!(
        incomplete_manifest_v3.validate(),
        Err(RequestAuditError::SamplingIdentityMismatch)
    ));

    let mut conflicting_duplicate = encoded.clone();
    conflicting_duplicate["adapter_version"] = json!("conflicting-adapter-v2");
    let conflicting_duplicate: RequestAuditManifest =
        serde_json::from_value(conflicting_duplicate).unwrap();
    let duplicate_event = SessionEvent::RequestAuditRecorded {
        manifest: conflicting_duplicate,
    }
    .into_envelope(
        EventId::new("request-audit-strict"),
        StreamId::new(SESSION_STREAM),
        EventSequence::new(3),
        OccurredAt::new("2026-08-23T00:00:02Z"),
    )
    .unwrap();
    assert!(matches!(
        session.append(&mut sink, duplicate_event),
        Err(AppendAfterDurableError::Session(
            cyber_agent::domain::session::SessionError::RequestAuditDuplicateEventConflict { .. }
        ))
    ));
    assert_eq!(sink.events.len(), 1);

    let mut unknown_schema = encoded.clone();
    unknown_schema["schema_version"] = json!(4);
    let unknown_schema: RequestAuditManifest = serde_json::from_value(unknown_schema).unwrap();
    assert!(matches!(
        unknown_schema.validate(),
        Err(RequestAuditError::UnsupportedManifestSchema { schema_version: 4 })
    ));

    let mut forged_renderer = encoded.clone();
    forged_renderer["schema_version"] = json!(2);
    forged_renderer["prompt_renderer"] = json!({"id": "forged", "version": 1});
    forged_renderer["resolved_tool_plan_fingerprint"] = json!("a".repeat(64));
    assert!(serde_json::from_value::<RequestAuditManifest>(forged_renderer).is_err());

    let mut unknown_full_ir_schema = encoded.clone();
    unknown_full_ir_schema["full_logical_input"]["schema_version"] = json!(999);
    let unknown_full_ir_schema: RequestAuditManifest =
        serde_json::from_value(unknown_full_ir_schema).unwrap();
    assert!(matches!(
        unknown_full_ir_schema.validate(),
        Err(RequestAuditError::UnsupportedModelInputIrSchema {
            input_field: "full_logical_input",
            schema_version: 999,
        })
    ));

    let mut unknown_delivered_ir_schema = encoded.clone();
    unknown_delivered_ir_schema["delivered_input"]["schema_version"] = json!(999);
    let unknown_delivered_ir_schema: RequestAuditManifest =
        serde_json::from_value(unknown_delivered_ir_schema).unwrap();
    assert!(matches!(
        unknown_delivered_ir_schema.validate(),
        Err(RequestAuditError::UnsupportedModelInputIrSchema {
            input_field: "delivered_input",
            schema_version: 999,
        })
    ));

    let mut mismatched_full_delivery = encoded.clone();
    mismatched_full_delivery["delivered_input"]["semantic_hash"] = json!("0".repeat(64));
    let mismatched_full_delivery: RequestAuditManifest =
        serde_json::from_value(mismatched_full_delivery).unwrap();
    assert!(matches!(
        mismatched_full_delivery.validate(),
        Err(RequestAuditError::FullDeliveryInputMismatch)
    ));

    let mut reordered = encoded.clone();
    let components = reordered["components"].as_array_mut().unwrap();
    components.swap(0, 1);
    let reordered: RequestAuditManifest = serde_json::from_value(reordered).unwrap();
    assert!(matches!(
        reordered.validate(),
        Err(RequestAuditError::ComponentPositionMismatch { .. })
    ));

    let mut duplicate = encoded.clone();
    duplicate["components"][2]["identity"] = duplicate["components"][1]["identity"].clone();
    let duplicate: RequestAuditManifest = serde_json::from_value(duplicate).unwrap();
    assert!(matches!(
        duplicate.validate(),
        Err(RequestAuditError::DuplicateComponentIdentity { .. })
    ));

    let first_blob = manifest.components()[0].blob_id().clone();
    blobs.tamper(&first_blob, b"tampered instructions");
    assert!(matches!(
        service.reconstruct(&manifest),
        Err(RequestAuditReconstructionError::Audit(
            RequestAuditError::ComponentSizeMismatch { .. }
                | RequestAuditError::ComponentHashMismatch { .. }
        ))
    ));
}

#[test]
fn request_audit_blob_and_event_failures_never_apply_a_partial_audit_fact() {
    let failing_blobs = MemoryBlobStore::default();
    failing_blobs.fail_on_publish_call(2);
    let failing_service = service(&failing_blobs);
    let mut session = projection();
    let mut sink = RecordingSink::default();
    assert!(matches!(
        failing_service.record(
            attempt(
                &run_context("project-left"),
                input("instructions", "user input")
            ),
            EventId::new("blob-failure"),
            OccurredAt::new("2026-08-23T00:00:01Z"),
            &mut session,
            &mut sink,
        ),
        Err(RequestAuditServiceError::Blob(BlobFailure::Publish))
    ));
    assert!(session.state().request_audits().is_empty());
    assert!(sink.events.is_empty());

    let blobs = MemoryBlobStore::default();
    let service = service(&blobs);
    let mut session = projection();
    let mut sink = RecordingSink {
        events: Vec::new(),
        fail_append: true,
    };
    assert!(matches!(
        service.record(
            attempt(
                &run_context("project-left"),
                input("instructions", "user input")
            ),
            EventId::new("event-failure"),
            OccurredAt::new("2026-08-23T00:00:01Z"),
            &mut session,
            &mut sink,
        ),
        Err(RequestAuditServiceError::Append(
            AppendAfterDurableError::Sink(SinkFailure::Append)
        ))
    ));
    assert!(session.state().request_audits().is_empty());
    assert!(sink.events.is_empty());
    assert!(
        blobs.blob_count() > 0,
        "orphan Blobs are allowed after Event failure"
    );
}

#[test]
fn request_audit_is_project_and_session_isolated_and_never_serializes_transport_or_credential_metadata()
 {
    let left_blobs = MemoryBlobStore::default();
    let left_service = service(&left_blobs);
    let mut session = projection();
    let mut sink = RecordingSink::default();
    assert!(matches!(
        left_service.record(
            attempt(
                &run_context("project-right"),
                input("wrong binding", "wrong user input")
            ),
            EventId::new("project-right-record"),
            OccurredAt::new("2026-08-23T00:00:01Z"),
            &mut session,
            &mut sink,
        ),
        Err(RequestAuditServiceError::Audit(
            RequestAuditError::ServiceBindingMismatch { .. }
        ))
    ));
    assert_eq!(left_blobs.blob_count(), 0);
    assert!(sink.events.is_empty());

    let mismatched_projection = &mut projection();
    let mismatched_binding = RequestAuditBinding::new(
        ProjectId::new("project-left"),
        StreamId::new("session:request-audit-other"),
    )
    .unwrap();
    let mismatched_service = RequestAuditService::new(&left_blobs, mismatched_binding.clone());
    let mismatched_context = RequestAuditRunContext::new(
        mismatched_binding,
        RequestAdapterVersion::new("fixture-adapter-v1").unwrap(),
    );
    let blob_count_before_mismatch = left_blobs.blob_count();
    assert!(matches!(
        mismatched_service.record(
            attempt(
                &mismatched_context,
                input("wrong stream", "wrong stream input")
            ),
            EventId::new("wrong-stream-record"),
            OccurredAt::new("2026-08-23T00:00:01Z"),
            mismatched_projection,
            &mut sink,
        ),
        Err(RequestAuditServiceError::Session(
            cyber_agent::domain::session::SessionError::RequestAuditSessionStreamMismatch { .. }
        ))
    ));
    assert_eq!(left_blobs.blob_count(), blob_count_before_mismatch);

    let manifest = record(
        &left_service,
        input("safe instructions", "safe user input"),
        &mut session,
        &mut sink,
        "project-left-audit",
    );
    let blob_count_before_duplicate = left_blobs.blob_count();
    assert!(matches!(
        left_service.record(
            attempt(
                &run_context("project-left"),
                input("different request", "reused event identity")
            ),
            EventId::new("project-left-audit"),
            OccurredAt::new("2026-08-23T00:00:02Z"),
            &mut session,
            &mut sink,
        ),
        Err(RequestAuditServiceError::Audit(
            RequestAuditError::EventIdAlreadyApplied { .. }
        ))
    ));
    assert_eq!(left_blobs.blob_count(), blob_count_before_duplicate);
    let right_blobs = MemoryBlobStore::default();
    for bytes in left_blobs.stored_bytes() {
        right_blobs.publish_bytes(&bytes).unwrap();
    }
    let right_service = RequestAuditService::new(
        &right_blobs,
        RequestAuditBinding::new(
            ProjectId::new("project-right"),
            StreamId::new("session:request-audit-right"),
        )
        .unwrap(),
    );
    assert!(matches!(
        right_service.reconstruct(&manifest),
        Err(RequestAuditReconstructionError::Audit(
            RequestAuditError::ServiceBindingMismatch { .. }
        ))
    ));

    let encoded = serde_json::to_string(&manifest).unwrap();
    for sentinel in [
        "Authorization",
        "CYBER_AGENT_OPENAI_API_KEY",
        "previous_response_id",
        "transport",
    ] {
        assert!(!encoded.contains(sentinel));
        assert!(
            left_blobs
                .stored_bytes()
                .iter()
                .all(|bytes| !String::from_utf8_lossy(bytes).contains(sentinel))
        );
    }
}
