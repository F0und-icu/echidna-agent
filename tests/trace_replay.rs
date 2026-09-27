use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::path::{Component, Path};
use std::sync::Mutex;

#[path = "support/live_provider.rs"]
mod live_provider;

use base64::Engine;
use echidna::adapters::replay::{
    ReplayAdapterError, ReplayModelProvider, ReplayModelStep, ReplayToolExecutor, ReplayToolStep,
};
use echidna::app::agent_loop::{AgentLoop, AgentLoopError};
use echidna::app::cancellation::AgentCancellation;
use echidna::app::request_audit::RequestAuditService;
use echidna::app::session::SessionProjection;
use echidna::app::trace_eval::{ContextRecompileInput, TraceEvalService};
use echidna::domain::artifacts::{ArtifactEvent, ArtifactId, ArtifactRecord, BlobId};
use echidna::domain::context::{
    BaselineInvalidationReason, ContextBudget, ContextFragmentContent, ModelIdentity, ModelProfile,
    ProviderSessionId, ProviderSessionRef, ProviderSessionRole,
};
use echidna::domain::events::{EventEnvelope, EventId, EventSequence, OccurredAt, StreamId};
use echidna::domain::model::{
    CapabilityMappingVersion, ContentItem, ModelCapabilities, ModelEvent, ModelInputIR,
    ModelRequestRequirements,
};
use echidna::domain::project::ProjectId;
use echidna::domain::request_audit::{
    RequestAdapterVersion, RequestAuditAttempt, RequestAuditBinding, RequestAuditDelivery,
    RequestAuditRunContext,
};
use echidna::domain::research::{
    ResearchEvent, ResearchReducer, ResearchState, TargetKind, TargetSnapshotDraft,
    TargetSnapshotId, TargetSnapshotRecord,
};
use echidna::domain::research_control::PromptComposition;
use echidna::domain::session::{SessionEvent, SessionReducer, SessionState};
use echidna::domain::tools::{
    DeferredActivationSet, ExecutionId, ExecutionReceipt, FileMutationAudit, RerunReason,
    ResolvedToolPlan, ToolCachePolicy, ToolCacheSource, ToolCacheValidation, ToolCall, ToolCallId,
    ToolDeterminism, ToolEffect, ToolExecutionOutcome, ToolExposure, ToolExposurePlan, ToolName,
    ToolOutputPolicy, ToolParallelism, ToolResult, ToolResultCacheKey, ToolResultStatus,
    ToolRetryPolicy, ToolSpec,
};
use echidna::domain::trace_eval::{
    EvalEvidence, EvalSource, TraceEventRange, TraceExpectedInvariant, TraceFixtureError,
    TraceFixtureManifest, TraceProviderEventReference, TraceReplayModelStep, TraceReplayPlanError,
};
use echidna::ports::artifact::{ArtifactStore, StoredBlob};
use echidna::ports::model::{
    ModelEventStream, ModelProvider, ModelProviderError, ModelRequest, ModelResponseCompletion,
    ModelStreamEvent, ProviderResponseId,
};
use echidna::ports::tool::ToolExecutor;
use echidna::trace_eval_runtime::PreparedTraceReplay;
use futures_util::stream;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const SESSION_ID: &str = "session:m4-trace";
const FIXTURE_DIRECTORY: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/m4_trace");
const FIXTURE_SUITE_SHA256: &str =
    "6c300dec946e83729175babae7d2633dc1cba5ae8eb56fcac8d6cbfd3fbf54a6";

#[test]
fn reasoning_without_assistant_output_is_not_a_replay_semantic_sample() {
    let input = ModelInputIR::current(
        "fixed replay input",
        vec![ContentItem::user_text("inspect fixture")],
        ModelRequestRequirements::default(),
    );
    assert_eq!(
        TraceReplayModelStep::new(
            input,
            vec![ModelEvent::ReasoningDelta {
                text: "private working text".to_owned(),
            }],
        ),
        Err(TraceReplayPlanError::ModelStepHasNoSemanticOutput)
    );
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum BlobFailure {
    Missing,
}

#[derive(Default)]
struct MemoryBlobStore {
    blobs: Mutex<BTreeMap<BlobId, Vec<u8>>>,
}

impl MemoryBlobStore {
    fn bytes(&self) -> Vec<Vec<u8>> {
        self.blobs.lock().unwrap().values().cloned().collect()
    }

    fn insert_fixture_blob(&self, blob_id: BlobId, bytes: Vec<u8>) {
        assert_eq!(blob_id.as_str(), format!("{:x}", Sha256::digest(&bytes)));
        assert!(self.blobs.lock().unwrap().insert(blob_id, bytes).is_none());
    }

    fn replace_blob_unchecked(&self, blob_id: BlobId, bytes: Vec<u8>) {
        self.blobs.lock().unwrap().insert(blob_id, bytes);
    }
}

impl ArtifactStore for MemoryBlobStore {
    type Error = BlobFailure;

    fn publish_file(&self, _source: &std::path::Path) -> Result<StoredBlob, Self::Error> {
        unreachable!("Trace RequestAudit fixtures publish in-memory bytes")
    }

    fn publish_bytes(&self, bytes: &[u8]) -> Result<StoredBlob, Self::Error> {
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

#[derive(Default)]
struct RecordingSink {
    events: Vec<EventEnvelope<Value>>,
}

impl echidna::ports::EventSink<Value> for RecordingSink {
    type Error = &'static str;

    fn append_event(&mut self, event: &EventEnvelope<Value>) -> Result<(), Self::Error> {
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
        incremental_input: false,
        usage_reporting: true,
        max_output_tokens: None,
        context_window: 32_768,
        capability_mapping_version: CapabilityMappingVersion::new(1),
    }
}

/// Builds the current fixed Base/Domain/Phase request shape for direct
/// AgentLoop regression cases. Historical Trace fixtures deliberately retain
/// their recorded instructions and go through `PreparedTraceReplay` instead.
fn current_prompt_input(user_text: &str, tool_specs: Vec<ToolSpec>) -> ModelInputIR {
    let instructions = PromptComposition::current()
        .fragments()
        .unwrap()
        .into_iter()
        .map(|fragment| match fragment.content() {
            ContextFragmentContent::Instructions { text } => text.clone(),
            ContextFragmentContent::ContentItems { .. } => {
                panic!("fixed prompt composition must contain only instructions")
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    ModelInputIR::current(
        instructions,
        vec![ContentItem::user_text(user_text)],
        ModelRequestRequirements::default(),
    )
    .with_tool_specs(tool_specs)
}

fn replay_source() -> EvalSource {
    EvalSource::new(
        "replay-unit-test",
        StreamId::new("session:replay-unit-test"),
        "a".repeat(64),
    )
    .unwrap()
}

fn replay_spec() -> ToolSpec {
    ToolSpec {
        name: ToolName::new("replay_read"),
        version: "fixture-v1".to_owned(),
        description: "read a fixture through Replay only".to_owned(),
        input_schema: json!({"type": "object"}),
        output_schema: json!({"type": "object"}),
        exposure: ToolExposure::Direct,
        parallelism: ToolParallelism::Serial,
        resource_key_template: None,
        effect: ToolEffect::ReadOnly,
        retry_policy: ToolRetryPolicy::ReplaySafe,
        determinism: ToolDeterminism::Deterministic,
        cache_policy: ToolCachePolicy::None,
        timeout_ms: 1_000,
        output_policy: ToolOutputPolicy { max_bytes: 4_096 },
    }
}

fn call() -> ToolCall {
    ToolCall::new(
        ToolCallId::new("call-read"),
        ToolName::new("replay_read"),
        json!({"path": "fixture.txt"}),
    )
}

fn result(status: ToolResultStatus) -> ToolResult {
    ToolResult {
        call_id: ToolCallId::new("call-read"),
        tool_name: ToolName::new("replay_read"),
        status,
        output: json!({"content": "fixed historical output"}),
    }
}

fn receipt(status: ToolResultStatus) -> ExecutionReceipt {
    ExecutionReceipt {
        execution_id: ExecutionId::new("execution:call-read"),
        plan_fingerprint: "source-plan-fingerprint".to_owned(),
        resolved_tool_plan_fingerprint: None,
        status,
        exit_reason: (status != ToolResultStatus::Succeeded).then(|| "source failure".to_owned()),
        output_artifact_ids: Vec::new(),
        output_captures: Vec::new(),
        environment_change: None,
        rerun_reason: None,
        cache_source: None,
        cache_validation: None,
        file_mutations: Vec::new(),
    }
}

fn research_with_required_artifact(
    artifact_id: ArtifactId,
    blob_id: BlobId,
    size_bytes: u64,
) -> ResearchState {
    let snapshot_id = TargetSnapshotId::new("artifact-snapshot");
    let snapshot = TargetSnapshotRecord::from_draft(TargetSnapshotDraft {
        id: snapshot_id.clone(),
        target_id: "fixture.target".to_owned(),
        kind: TargetKind::Source,
        locator: "src/fixture".to_owned(),
        version: "1.0.0".to_owned(),
        build_id: "fixture-build".to_owned(),
        target_artifact_ids: Vec::new(),
        environment_fingerprint: "fixture-environment".to_owned(),
        device_fingerprint: "fixture-device".to_owned(),
        captured_at: "2026-08-28T00:00:00Z".to_owned(),
        notes: String::new(),
    })
    .unwrap();
    let artifact = ArtifactRecord {
        id: artifact_id,
        blob_id,
        media_type: "application/octet-stream".to_owned(),
        size_bytes,
        source_path: "fixture.bin".to_owned(),
        created_by_event_id: EventId::new("artifact-event"),
        produced_by_tool_call_id: ToolCallId::new("artifact-call"),
        target_snapshot_id: snapshot_id,
        toolchain_fingerprint: "fixture-toolchain".to_owned(),
        parent_artifact_ids: Vec::new(),
        transform: None,
    };
    let reducer = ResearchReducer;
    let mut research = reducer.reduce(Vec::<EventEnvelope<Value>>::new()).unwrap();
    reducer
        .apply_event(
            &mut research,
            ResearchEvent::TargetSnapshotRecorded(snapshot),
        )
        .unwrap();
    reducer
        .apply_event(
            &mut research,
            ResearchEvent::Artifact(ArtifactEvent::Created(artifact)),
        )
        .unwrap();
    research
}

fn manifest_with_required_artifact(
    fixture: &TraceSourceFixture,
    artifact_id: &ArtifactId,
) -> TraceFixtureManifest {
    let mut manifest = serde_json::to_value(&fixture.manifest).unwrap();
    manifest["required_artifact_ids"] = json!([artifact_id]);
    serde_json::from_value(manifest).unwrap()
}

fn run_context() -> RequestAuditRunContext {
    RequestAuditRunContext::new(
        RequestAuditBinding::new(ProjectId::new("trace-project"), StreamId::new(SESSION_ID))
            .unwrap(),
        RequestAdapterVersion::new("fixture-adapter-v1").unwrap(),
    )
}

fn audit_attempt(context: &RequestAuditRunContext, input: ModelInputIR) -> RequestAuditAttempt {
    RequestAuditAttempt::new(
        context,
        ModelIdentity::new("fixture", "model"),
        ModelProfile::new("fixture-profile", 1),
        capabilities(),
        input.clone(),
        input,
        RequestAuditDelivery::full(BaselineInvalidationReason::NoBaseline),
    )
    .unwrap()
}

fn envelope(event: SessionEvent, id: &str, sequence: u64) -> EventEnvelope<Value> {
    event
        .into_envelope(
            EventId::new(id),
            StreamId::new(SESSION_ID),
            EventSequence::new(sequence),
            OccurredAt::new("2026-08-28T00:00:00Z"),
        )
        .unwrap()
}

fn append(
    projection: &mut SessionProjection,
    sink: &mut RecordingSink,
    event: SessionEvent,
    id: &str,
    sequence: u64,
) {
    projection
        .append(sink, envelope(event, id, sequence))
        .unwrap();
}

struct TraceSourceFixture {
    blobs: MemoryBlobStore,
    source_events: Vec<EventEnvelope<Value>>,
    session: SessionState,
    research: ResearchState,
    manifest: TraceFixtureManifest,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FixedTraceSuite {
    schema_version: u16,
    files: BTreeMap<String, String>,
}

fn fixed_fixture_path(relative: &str) -> std::path::PathBuf {
    let relative = Path::new(relative);
    assert!(!relative.is_absolute());
    assert!(
        relative
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    );
    Path::new(FIXTURE_DIRECTORY).join(relative)
}

fn load_fixed_source_fixture() -> TraceSourceFixture {
    let suite_bytes = fs::read(fixed_fixture_path("suite.json")).unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(&suite_bytes)),
        FIXTURE_SUITE_SHA256
    );
    let suite: FixedTraceSuite = serde_json::from_slice(&suite_bytes).unwrap();
    assert_eq!(suite.schema_version, 1);
    for (relative, expected_sha256) in &suite.files {
        let bytes = fs::read(fixed_fixture_path(relative)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), *expected_sha256);
    }

    let manifest: TraceFixtureManifest =
        serde_json::from_slice(&fs::read(fixed_fixture_path("trace_manifest.json")).unwrap())
            .unwrap();
    let source_events: Vec<EventEnvelope<Value>> =
        serde_json::from_slice(&fs::read(fixed_fixture_path("source_session.json")).unwrap())
            .unwrap();
    let session = SessionReducer
        .reduce(source_events.iter().cloned())
        .unwrap();
    let research = ResearchReducer
        .reduce(Vec::<EventEnvelope<Value>>::new())
        .unwrap();
    let blobs = MemoryBlobStore::default();
    for relative in suite
        .files
        .keys()
        .filter(|relative| relative.starts_with("blobs/"))
    {
        let path = fixed_fixture_path(relative);
        let encoded = fs::read_to_string(&path).unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded.trim())
            .unwrap();
        let blob_id = BlobId::new(path.file_stem().unwrap().to_str().unwrap()).unwrap();
        blobs.insert_fixture_blob(blob_id, bytes);
    }
    manifest
        .validate_against_sources(&source_events, &session, &research)
        .unwrap();
    TraceSourceFixture {
        blobs,
        source_events,
        session,
        research,
        manifest,
    }
}

#[test]
fn trace_renderer_identity_is_additive_and_legacy_fixture_remains_explicitly_unknown() {
    let legacy = load_fixed_source_fixture();
    assert!(legacy.manifest.prompt_renderer().is_none());
    let current = source_fixture(ToolResultStatus::Succeeded);
    assert_eq!(current.manifest.prompt_renderer().unwrap().id(), "generic");
    assert_eq!(current.manifest.prompt_renderer().unwrap().version(), 1);
}

#[derive(Clone, Copy)]
enum ReceiptFact {
    Recorded,
    CacheHit,
}

fn source_fixture(status: ToolResultStatus) -> TraceSourceFixture {
    source_fixture_with_receipt(status, ReceiptFact::Recorded)
}

fn source_fixture_with_receipt(
    status: ToolResultStatus,
    receipt_fact: ReceiptFact,
) -> TraceSourceFixture {
    source_fixture_with_captures(status, receipt_fact, Vec::new())
}

fn source_fixture_with_captures(
    status: ToolResultStatus,
    receipt_fact: ReceiptFact,
    captures: Vec<echidna::domain::tools::OutputCapture>,
) -> TraceSourceFixture {
    let mut source_receipt = receipt(status);
    let mut source_result = result(status);
    if !captures.is_empty() {
        source_result.output["output_captures"] = serde_json::to_value(&captures).unwrap();
    }
    source_receipt.output_captures = captures;
    let blobs = MemoryBlobStore::default();
    let context = run_context();
    let started = envelope(
        SessionEvent::Started {
            primary_provider_session: ProviderSessionRef {
                id: ProviderSessionId::new("primary-trace"),
                role: ProviderSessionRole::Primary,
                generation: 1,
            },
        },
        "started",
        1,
    );
    let mut projection = SessionProjection::replay([started.clone()]).unwrap();
    let mut sink = RecordingSink::default();
    let service = RequestAuditService::new(&blobs, context.binding().clone());
    let spec = replay_spec();
    let first_input = ModelInputIR::current(
        "trace instructions",
        vec![ContentItem::user_text("inspect fixture")],
        ModelRequestRequirements::default(),
    )
    .with_tool_specs(vec![spec.clone()]);
    service
        .record(
            audit_attempt(&context, first_input),
            EventId::new("audit-one"),
            OccurredAt::new("2026-08-28T00:00:01Z"),
            &mut projection,
            &mut sink,
        )
        .unwrap();
    append(
        &mut projection,
        &mut sink,
        SessionEvent::HistoryItemRecorded {
            item: ContentItem::ToolCall { call: call() },
        },
        "model-call-one",
        3,
    );
    let receipt_event = match receipt_fact {
        ReceiptFact::Recorded => SessionEvent::ExecutionReceiptRecorded {
            receipt: source_receipt,
        },
        ReceiptFact::CacheHit => SessionEvent::ToolResultCacheHit {
            receipt: source_receipt,
        },
    };
    append(&mut projection, &mut sink, receipt_event, "receipt-one", 4);
    append(
        &mut projection,
        &mut sink,
        SessionEvent::HistoryItemRecorded {
            item: ContentItem::ToolResult {
                result: source_result.clone(),
            },
        },
        "tool-result-one",
        5,
    );
    let second_input = ModelInputIR::current(
        "trace instructions",
        vec![
            ContentItem::user_text("inspect fixture"),
            ContentItem::ToolCall { call: call() },
            ContentItem::ToolResult {
                result: source_result,
            },
        ],
        ModelRequestRequirements::default(),
    )
    .with_tool_specs(vec![spec]);
    service
        .record(
            audit_attempt(&context, second_input),
            EventId::new("audit-two"),
            OccurredAt::new("2026-08-28T00:00:02Z"),
            &mut projection,
            &mut sink,
        )
        .unwrap();
    append(
        &mut projection,
        &mut sink,
        SessionEvent::HistoryItemRecorded {
            item: ContentItem::assistant_text("fixed replay conclusion"),
        },
        "model-text-two",
        7,
    );
    let mut source_events = vec![started];
    source_events.extend(sink.events);
    let stream_hash = TraceFixtureManifest::source_stream_hash_for(&source_events).unwrap();
    let manifest = TraceFixtureManifest::new(
        "m4-trace-fixture",
        StreamId::new(SESSION_ID),
        TraceEventRange::new(EventSequence::new(1), EventSequence::new(7)).unwrap(),
        stream_hash,
        Vec::new(),
        Vec::new(),
        vec![
            TraceProviderEventReference::new(
                EventId::new("audit-one"),
                vec![EventId::new("model-call-one")],
            )
            .unwrap(),
            TraceProviderEventReference::new(
                EventId::new("audit-two"),
                vec![EventId::new("model-text-two")],
            )
            .unwrap(),
        ],
        vec![EventId::new("receipt-one")],
        vec![
            TraceExpectedInvariant::FullRequestSemanticReplay,
            TraceExpectedInvariant::ToolPairing,
            TraceExpectedInvariant::ZeroExternalActions,
            TraceExpectedInvariant::NonEvidence,
        ],
    )
    .unwrap();
    TraceSourceFixture {
        blobs,
        source_events,
        session: projection.state().clone(),
        research: ResearchReducer
            .reduce(Vec::<EventEnvelope<Value>>::new())
            .unwrap(),
        manifest,
    }
}

fn prepare_fixture(fixture: &TraceSourceFixture) -> PreparedTraceReplay {
    let plan = TraceEvalService::new(&fixture.blobs)
        .prepare_mechanical_replay(
            &fixture.manifest,
            &fixture.source_events,
            &fixture.session,
            &fixture.research,
        )
        .unwrap();
    PreparedTraceReplay::new(plan, capabilities()).unwrap()
}

#[tokio::test]
async fn output_capture_replay_retains_source_refs_without_publication_or_evidence() {
    use echidna::domain::tools::{OutputCapture, OutputStream};
    let captures = vec![OutputCapture {
        stream: OutputStream::Stdout,
        child_index: None,
        blob_id: BlobId::new(format!("{:x}", Sha256::digest(b"RAW"))).unwrap(),
        observed_bytes: 3,
        retained_bytes: 3,
        eof: true,
        complete: true,
        read_error: None,
    }];
    for status in [
        ToolResultStatus::Succeeded,
        ToolResultStatus::Failed,
        ToolResultStatus::Cancelled,
    ] {
        let fixture = source_fixture_with_captures(status, ReceiptFact::Recorded, captures.clone());
        let before_session = fixture.session.clone();
        let before_blobs = fixture.blobs.bytes();
        let prepared = prepare_fixture(&fixture);
        // No output publisher is installed and the source raw Blob is absent:
        // mechanical Replay must only preserve refs, never reread/republish it.
        let mut agent = AgentLoop::new(prepared.model_provider(), prepared.tool_executor());
        let replay = prepared
            .mechanical_replay(&mut agent, ["inspect fixture".to_owned()])
            .await
            .unwrap();
        let replay_use = &replay.replay_tool_uses()[0];
        assert_eq!(replay_use.source_receipt().output_captures, captures);
        assert_eq!(replay_use.replay_receipt().output_captures, captures);
        assert!(replay_use.replay_receipt().output_artifact_ids.is_empty());
        assert!(replay_use.replay_receipt().file_mutations.is_empty());
        assert!(replay.evaluation().is_non_evidence());
        assert_eq!(fixture.session, before_session);
        assert_eq!(fixture.blobs.bytes(), before_blobs);
        assert!(agent.history().items().iter().any(|item| matches!(item,
            ContentItem::ToolResult { result } if result.output["output_captures"] == serde_json::to_value(&captures).unwrap()
        )));
        let mut source_receipt = receipt(status);
        source_receipt.output_captures = captures.clone();
        assert!(matches!(
            ReplayToolStep::new(
                EventId::new("mismatch"),
                source_receipt,
                call(),
                result(status)
            ),
            Err(TraceReplayPlanError::SourceOutputCaptureMismatch)
        ));
    }
}

#[tokio::test]
async fn mechanical_replay_is_deterministic_local_and_non_evidence() {
    let fixture = source_fixture(ToolResultStatus::Succeeded);
    let prepared = prepare_fixture(&fixture);
    let before_session = fixture.session.clone();
    let before_blobs = fixture.blobs.bytes();
    let mut agent = AgentLoop::new(prepared.model_provider(), prepared.tool_executor());

    let replay = prepared
        .mechanical_replay(&mut agent, ["inspect fixture".to_owned()])
        .await
        .unwrap();

    assert_eq!(replay.model_requests(), 2);
    assert_eq!(replay.replay_tool_uses().len(), 1);
    assert_eq!(replay.evaluation().evidence(), EvalEvidence::NonEvidence);
    assert!(replay.evaluation().is_non_evidence());
    assert_eq!(fixture.session, before_session);
    assert_eq!(fixture.blobs.bytes(), before_blobs);
    assert_eq!(prepared.model_provider().remaining_steps(), 0);
    assert_eq!(prepared.tool_executor().remaining_steps(), 0);
}

#[tokio::test]
async fn fixed_trace_fixture_is_hash_anchored_and_replays_without_external_actions() {
    let fixture = load_fixed_source_fixture();
    let prepared = prepare_fixture(&fixture);
    let mut agent = AgentLoop::new(prepared.model_provider(), prepared.tool_executor());

    let replay = prepared
        .mechanical_replay(&mut agent, ["inspect fixture".to_owned()])
        .await
        .unwrap();

    assert_eq!(replay.model_requests(), 2);
    let replay_use = &replay.replay_tool_uses()[0];
    assert_eq!(
        replay.evaluation().source().source_session_id(),
        fixture.manifest.source_session_id()
    );
    assert_eq!(
        replay.evaluation().source().source_stream_hash(),
        fixture.manifest.source_stream_hash()
    );
    assert_eq!(replay_use.source(), replay.evaluation().source());
    assert_eq!(
        replay_use.source_receipt_event_id(),
        &EventId::new("receipt-one")
    );
    assert_eq!(
        replay_use.source_execution_id(),
        &ExecutionId::new("execution:call-read")
    );
    assert_eq!(
        replay_use.replay_execution_id(),
        &ExecutionId::new("execution:call-read")
    );
    assert!(replay.evaluation().is_non_evidence());
}

#[tokio::test]
async fn cache_hit_receipt_is_a_replayable_existing_session_fact() {
    let fixture = source_fixture_with_receipt(ToolResultStatus::Succeeded, ReceiptFact::CacheHit);
    assert_eq!(fixture.session.execution_receipt_count(), 0);
    assert_eq!(fixture.session.cache_hit_count(), 1);
    let prepared = prepare_fixture(&fixture);
    let mut agent = AgentLoop::new(prepared.model_provider(), prepared.tool_executor());

    let replay = prepared
        .mechanical_replay(&mut agent, ["inspect fixture".to_owned()])
        .await
        .unwrap();

    assert_eq!(
        replay.replay_tool_uses()[0].source_receipt_event_id(),
        &EventId::new("receipt-one")
    );
}

#[test]
fn replay_tool_exposure_is_fixed_to_direct_without_a_registry_fallback() {
    for exposure in [ToolExposure::Deferred, ToolExposure::Hidden] {
        let mut spec = replay_spec();
        spec.exposure = exposure;
        assert!(matches!(
            ReplayToolExecutor::new(replay_source(), [spec], Vec::<ReplayToolStep>::new()),
            Err(ReplayAdapterError::NonDirectToolSpec { tool_name }) if tool_name == "replay_read"
        ));
    }
}

#[test]
fn replay_tool_dispatch_rejects_a_non_fixture_plan_without_consuming_a_recorded_step() {
    let tools = ReplayToolExecutor::new(
        replay_source(),
        [replay_spec()],
        [ReplayToolStep::new(
            EventId::new("source-receipt"),
            receipt(ToolResultStatus::Succeeded),
            call(),
            result(ToolResultStatus::Succeeded),
        )
        .unwrap()],
    )
    .unwrap();
    let foreign = ResolvedToolPlan::resolve(
        [replay_spec()],
        &ToolExposurePlan::new([(ToolName::new("replay_read"), ToolExposure::Hidden)]),
        &DeferredActivationSet::default(),
    )
    .unwrap();
    assert!(foreign.advertised_specs().is_empty());
    assert!(matches!(
        tools.plan_model_call(&foreign, ExecutionId::new("foreign-plan"), call()),
        Err(echidna::ports::tool::ToolExecutorError::ResolvedPlanMismatch)
    ));
    assert_eq!(tools.remaining_steps(), 1);
}

#[tokio::test]
async fn replay_narrows_an_incremental_source_mapping_to_full_delivery() {
    let mut source_capabilities = capabilities();
    source_capabilities.incremental_input = true;
    let expected_input = current_prompt_input("inspect fixture", Vec::new());
    let provider = ReplayModelProvider::new(
        source_capabilities.clone(),
        [ReplayModelStep::new(
            expected_input.clone(),
            vec![ModelEvent::TextDelta {
                text: "fixed full-only replay output".to_owned(),
            }],
        )
        .unwrap()],
    )
    .unwrap();
    let mut replay_capabilities = source_capabilities;
    replay_capabilities.incremental_input = false;
    assert_eq!(provider.capabilities(), replay_capabilities);

    let tools = ReplayToolExecutor::new(
        replay_source(),
        Vec::<ToolSpec>::new(),
        Vec::<ReplayToolStep>::new(),
    )
    .unwrap();
    let mut agent = AgentLoop::new(&provider, &tools);
    let outcome = agent
        .run_turn("inspect fixture", AgentCancellation::new())
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        echidna::app::agent_loop::AgentTurnOutcome::Completed {
            model_requests: 1,
            ..
        }
    ));
    assert_eq!(provider.remaining_steps(), 0);

    assert!(matches!(
        provider.stream(ModelRequest::incremental(
            expected_input,
            ProviderResponseId::new("source-continuation"),
        )),
        Err(ModelProviderError::ContinuationRejected { code }) if code == "replay_incremental_unsupported"
    ));
}

#[tokio::test]
async fn replay_upcasts_historical_v2_input_before_matching_a_current_request() {
    let mut historical_input = current_prompt_input("inspect fixture", Vec::new());
    historical_input.schema_version = echidna::domain::model::ModelInputIrSchemaVersion::new(2);
    let provider = ReplayModelProvider::new(
        capabilities(),
        [ReplayModelStep::new(
            historical_input,
            vec![ModelEvent::TextDelta {
                text: "historical replay output".to_owned(),
            }],
        )
        .unwrap()],
    )
    .unwrap();
    let tools = ReplayToolExecutor::new(
        replay_source(),
        Vec::<ToolSpec>::new(),
        Vec::<ReplayToolStep>::new(),
    )
    .unwrap();
    let mut agent = AgentLoop::new(&provider, &tools);

    agent
        .run_turn("inspect fixture", AgentCancellation::new())
        .await
        .expect("historical v2 input is losslessly promoted before request matching");
    assert_eq!(provider.remaining_steps(), 0);
}

#[test]
fn manifest_rejects_unknown_fields_bad_hash_and_missing_artifact_reference() {
    let fixture = source_fixture(ToolResultStatus::Succeeded);
    let mut unknown = serde_json::to_value(&fixture.manifest).unwrap();
    unknown["unexpected"] = json!(true);
    assert!(serde_json::from_value::<TraceFixtureManifest>(unknown).is_err());

    let mut forged_renderer = serde_json::to_value(&fixture.manifest).unwrap();
    forged_renderer["prompt_renderer"] = json!({"id": "forged", "version": 1});
    assert!(serde_json::from_value::<TraceFixtureManifest>(forged_renderer).is_err());

    let mut bad_hash = serde_json::to_value(&fixture.manifest).unwrap();
    bad_hash["source_stream_hash"] = json!("0".repeat(64));
    let bad_hash = serde_json::from_value::<TraceFixtureManifest>(bad_hash).unwrap();
    assert!(matches!(
        bad_hash.validate_against_sources(
            &fixture.source_events,
            &fixture.session,
            &fixture.research
        ),
        Err(TraceFixtureError::SourceStreamHashMismatch { .. })
    ));

    let mut missing_artifact = serde_json::to_value(&fixture.manifest).unwrap();
    missing_artifact["required_artifact_ids"] = json!([ArtifactId::new("missing-artifact")]);
    let missing_artifact =
        serde_json::from_value::<TraceFixtureManifest>(missing_artifact).unwrap();
    assert!(matches!(
        missing_artifact.validate_against_sources(
            &fixture.source_events,
            &fixture.session,
            &fixture.research
        ),
        Err(TraceFixtureError::ArtifactMissing { .. })
    ));

    let mut missing_invariant = serde_json::to_value(&fixture.manifest).unwrap();
    missing_invariant["expected_invariants"] = json!([
        "full_request_semantic_replay",
        "tool_pairing",
        "non_evidence"
    ]);
    let missing_invariant =
        serde_json::from_value::<TraceFixtureManifest>(missing_invariant).unwrap();
    assert!(matches!(
        missing_invariant.validate_against_sources(
            &fixture.source_events,
            &fixture.session,
            &fixture.research
        ),
        Err(TraceFixtureError::RequiredExpectedInvariantMissing {
            invariant: TraceExpectedInvariant::ZeroExternalActions
        })
    ));
}

#[test]
fn mechanical_replay_verifies_required_artifact_blob_identity_and_size() {
    let fixture = source_fixture(ToolResultStatus::Succeeded);
    let artifact_id = ArtifactId::new("required-artifact");
    let source_bytes = b"required fixture artifact".to_vec();
    let blob_id = BlobId::new(format!("{:x}", Sha256::digest(&source_bytes))).unwrap();
    let manifest = manifest_with_required_artifact(&fixture, &artifact_id);
    let research = research_with_required_artifact(
        artifact_id.clone(),
        blob_id.clone(),
        u64::try_from(source_bytes.len()).unwrap(),
    );
    let service = TraceEvalService::new(&fixture.blobs);

    assert!(matches!(
        service.prepare_mechanical_replay(
            &manifest,
            &fixture.source_events,
            &fixture.session,
            &research,
        ),
        Err(echidna::app::trace_eval::TraceEvalServiceError::RequiredArtifactBlobUnreadable {
            artifact_id: received_artifact_id,
            blob_id: received_blob_id,
        }) if received_artifact_id == artifact_id && received_blob_id == blob_id
    ));

    fixture.blobs.publish_bytes(&source_bytes).unwrap();
    assert!(
        service
            .prepare_mechanical_replay(
                &manifest,
                &fixture.source_events,
                &fixture.session,
                &research,
            )
            .is_ok()
    );

    fixture
        .blobs
        .replace_blob_unchecked(blob_id.clone(), b"alternate fixture bytes!!".to_vec());
    assert!(matches!(
        service.prepare_mechanical_replay(
            &manifest,
            &fixture.source_events,
            &fixture.session,
            &research,
        ),
        Err(echidna::app::trace_eval::TraceEvalServiceError::RequiredArtifactBlobHashMismatch {
            artifact_id: received_artifact_id,
            expected_blob_id,
            ..
        }) if received_artifact_id == artifact_id && expected_blob_id == blob_id
    ));

    fixture
        .blobs
        .replace_blob_unchecked(blob_id.clone(), b"wrong size".to_vec());
    assert!(matches!(
        service.prepare_mechanical_replay(
            &manifest,
            &fixture.source_events,
            &fixture.session,
            &research,
        ),
        Err(echidna::app::trace_eval::TraceEvalServiceError::RequiredArtifactBlobSizeMismatch {
            artifact_id: received_artifact_id,
            blob_id: received_blob_id,
            ..
        }) if received_artifact_id == artifact_id && received_blob_id == blob_id
    ));
}

#[test]
fn manifest_rejects_a_session_projection_not_derived_from_its_source_events() {
    let fixture = source_fixture(ToolResultStatus::Succeeded);
    let incomplete_projection = SessionReducer
        .reduce(fixture.source_events[..6].iter().cloned())
        .unwrap();

    assert!(matches!(
        fixture.manifest.validate_against_sources(
            &fixture.source_events,
            &incomplete_projection,
            &fixture.research
        ),
        Err(TraceFixtureError::SessionProjectionMismatch)
    ));
}

#[test]
fn manifest_rejects_provider_references_out_of_source_order() {
    let fixture = source_fixture(ToolResultStatus::Succeeded);
    let mut reversed = serde_json::to_value(&fixture.manifest).unwrap();
    reversed["provider_event_refs"]
        .as_array_mut()
        .unwrap()
        .reverse();
    let reversed = serde_json::from_value::<TraceFixtureManifest>(reversed).unwrap();

    assert!(matches!(
        reversed.validate_against_sources(
            &fixture.source_events,
            &fixture.session,
            &fixture.research
        ),
        Err(TraceFixtureError::EventReferenceOrderInvalid { .. })
    ));
}

#[test]
fn mechanical_replay_rejects_a_tool_result_outside_the_declared_event_range() {
    let fixture = source_fixture(ToolResultStatus::Succeeded);
    let manifest = TraceFixtureManifest::new(
        fixture.manifest.trace_id(),
        fixture.manifest.source_session_id().clone(),
        TraceEventRange::new(EventSequence::new(1), EventSequence::new(4)).unwrap(),
        fixture.manifest.source_stream_hash(),
        Vec::new(),
        Vec::new(),
        vec![fixture.manifest.provider_event_refs()[0].clone()],
        vec![EventId::new("receipt-one")],
        fixture.manifest.expected_invariants().to_vec(),
    )
    .unwrap();
    let service = TraceEvalService::new(&fixture.blobs);

    assert!(matches!(
        service.prepare_mechanical_replay(
            &manifest,
            &fixture.source_events,
            &fixture.session,
            &fixture.research,
        ),
        Err(echidna::app::trace_eval::TraceEvalServiceError::ToolResultOutsideRange {
            call_id,
            event_id,
        }) if call_id == "call-read" && event_id == EventId::new("tool-result-one")
    ));
}

#[test]
fn mechanical_replay_rejects_tool_facts_out_of_source_order() {
    let fixture = source_fixture(ToolResultStatus::Succeeded);
    let mut source_events = fixture.source_events.clone();
    let mut receipt = source_events[3].clone();
    let mut tool_result = source_events[4].clone();
    tool_result.sequence = EventSequence::new(4);
    receipt.sequence = EventSequence::new(5);
    source_events[3] = tool_result;
    source_events[4] = receipt;
    let session = SessionReducer
        .reduce(source_events.iter().cloned())
        .unwrap();
    let manifest = TraceFixtureManifest::new(
        fixture.manifest.trace_id(),
        fixture.manifest.source_session_id().clone(),
        fixture.manifest.event_range(),
        TraceFixtureManifest::source_stream_hash_for(&source_events).unwrap(),
        Vec::new(),
        Vec::new(),
        fixture.manifest.provider_event_refs().to_vec(),
        fixture.manifest.tool_receipt_refs().to_vec(),
        fixture.manifest.expected_invariants().to_vec(),
    )
    .unwrap();

    assert!(matches!(
        TraceEvalService::new(&fixture.blobs).prepare_mechanical_replay(
            &manifest,
            &source_events,
            &session,
            &fixture.research,
        ),
        Err(echidna::app::trace_eval::TraceEvalServiceError::ToolFactOrderInvalid { .. })
    ));
}

#[test]
fn mechanical_replay_requires_receipt_references_in_source_call_order() {
    let fixture = source_fixture(ToolResultStatus::Succeeded);
    let second_call = ToolCall::new(
        ToolCallId::new("call-read-second"),
        ToolName::new("replay_read"),
        json!({"path": "second-fixture.txt"}),
    );
    let second_result = ToolResult {
        call_id: second_call.call_id.clone(),
        tool_name: second_call.name.clone(),
        status: ToolResultStatus::Succeeded,
        output: json!({"content": "second fixed output"}),
    };
    let mut second_receipt = receipt(ToolResultStatus::Succeeded);
    second_receipt.execution_id = ExecutionId::new("execution:call-read-second");
    let mut source_events = fixture.source_events.clone();
    source_events.extend([
        envelope(
            SessionEvent::HistoryItemRecorded {
                item: ContentItem::ToolCall {
                    call: second_call.clone(),
                },
            },
            "model-call-two",
            8,
        ),
        envelope(
            SessionEvent::ExecutionReceiptRecorded {
                receipt: second_receipt,
            },
            "receipt-two",
            9,
        ),
        envelope(
            SessionEvent::HistoryItemRecorded {
                item: ContentItem::ToolResult {
                    result: second_result,
                },
            },
            "tool-result-two",
            10,
        ),
    ]);
    let session = SessionReducer
        .reduce(source_events.iter().cloned())
        .unwrap();
    let manifest = TraceFixtureManifest::new(
        fixture.manifest.trace_id(),
        fixture.manifest.source_session_id().clone(),
        TraceEventRange::new(EventSequence::new(1), EventSequence::new(10)).unwrap(),
        TraceFixtureManifest::source_stream_hash_for(&source_events).unwrap(),
        Vec::new(),
        Vec::new(),
        vec![
            fixture.manifest.provider_event_refs()[0].clone(),
            TraceProviderEventReference::new(
                EventId::new("audit-two"),
                vec![
                    EventId::new("model-text-two"),
                    EventId::new("model-call-two"),
                ],
            )
            .unwrap(),
        ],
        vec![EventId::new("receipt-two"), EventId::new("receipt-one")],
        fixture.manifest.expected_invariants().to_vec(),
    )
    .unwrap();

    assert!(matches!(
        TraceEvalService::new(&fixture.blobs).prepare_mechanical_replay(
            &manifest,
            &source_events,
            &session,
            &fixture.research,
        ),
        Err(
            echidna::app::trace_eval::TraceEvalServiceError::ToolReceiptReferenceOrderInvalid {
                call_event_id,
                expected_receipt_event_id,
                declared_receipt_event_id,
            }
        ) if call_event_id == EventId::new("model-call-one")
            && expected_receipt_event_id == EventId::new("receipt-one")
            && declared_receipt_event_id == EventId::new("receipt-two")
    ));
}

#[tokio::test]
async fn unmatched_replay_tool_call_stops_before_any_follow_up_provider_request() {
    let spec = replay_spec();
    let expected_input = current_prompt_input("inspect fixture", vec![spec.clone()]);
    let provider = ReplayModelProvider::new(
        capabilities(),
        [
            ReplayModelStep::new(
                expected_input.clone(),
                vec![ModelEvent::ToolCall {
                    call: ToolCall::new(
                        ToolCallId::new("unexpected-call"),
                        ToolName::new("unexpected_tool"),
                        json!({"input": "must not execute"}),
                    ),
                }],
            )
            .unwrap(),
            ReplayModelStep::new(
                expected_input,
                vec![ModelEvent::TextDelta {
                    text: "this second Provider request must not occur".to_owned(),
                }],
            )
            .unwrap(),
        ],
    )
    .unwrap();
    let tools = ReplayToolExecutor::new(
        replay_source(),
        [spec],
        [ReplayToolStep::new(
            EventId::new("source-receipt"),
            receipt(ToolResultStatus::Succeeded),
            call(),
            result(ToolResultStatus::Succeeded),
        )
        .unwrap()],
    )
    .unwrap();
    let mut agent = AgentLoop::new(&provider, &tools);

    assert!(matches!(
        agent.run_turn("inspect fixture", AgentCancellation::new()).await,
        Err(AgentLoopError::UnmatchedToolCall { tool_name }) if tool_name == "unexpected_tool"
    ));
    assert_eq!(provider.remaining_steps(), 1);
    assert!(tools.replay_uses().is_empty());
}

#[test]
fn context_recompile_reads_historical_bytes_without_mutating_audit_blobs() {
    let fixture = source_fixture(ToolResultStatus::Succeeded);
    let service = TraceEvalService::new(&fixture.blobs);
    let before = fixture.blobs.bytes();
    let composition = PromptComposition::current();
    let result = service
        .context_recompile(
            &fixture.manifest,
            &fixture.source_events,
            &fixture.session,
            &fixture.research,
            ContextRecompileInput {
                request_audit_event_id: &EventId::new("audit-one"),
                planner: &echidna::domain::context::ContextPlanner::current(),
                fragments: composition.fragments().unwrap(),
                budget: ContextBudget::new(32_768, 0, 0).unwrap(),
                capabilities: &capabilities(),
                tool_specs: vec![replay_spec()],
                requirements: ModelRequestRequirements::default(),
                prior_manifest: None,
            },
        )
        .unwrap();

    assert_eq!(result.evaluation().evidence(), EvalEvidence::NonEvidence);
    assert_eq!(
        result.evaluation().source().trace_id(),
        fixture.manifest.trace_id()
    );
    assert_eq!(
        result.evaluation().request_audit_event_id(),
        Some(&EventId::new("audit-one"))
    );
    assert_eq!(
        result.historical_delivered_input().instructions,
        "trace instructions"
    );
    assert_eq!(fixture.blobs.bytes(), before);
}

#[test]
fn context_recompile_rejects_an_audit_outside_the_trace_source_scope() {
    let fixture = source_fixture(ToolResultStatus::Succeeded);
    let service = TraceEvalService::new(&fixture.blobs);
    let composition = PromptComposition::current();
    let unreferenced = EventId::new("audit-outside-trace");

    assert!(matches!(
        service.context_recompile(
            &fixture.manifest,
            &fixture.source_events,
            &fixture.session,
            &fixture.research,
            ContextRecompileInput {
                request_audit_event_id: &unreferenced,
                planner: &echidna::domain::context::ContextPlanner::current(),
                fragments: composition.fragments().unwrap(),
                budget: ContextBudget::new(32_768, 0, 0).unwrap(),
                capabilities: &capabilities(),
                tool_specs: vec![replay_spec()],
                requirements: ModelRequestRequirements::default(),
                prior_manifest: None,
            },
        ),
        Err(
            echidna::app::trace_eval::TraceEvalServiceError::ContextRequestAuditNotReferenced {
                audit_event_id
            }
        ) if audit_event_id == unreferenced
    ));
}

struct ScriptedProvider {
    capabilities: ModelCapabilities,
    responses: Mutex<VecDeque<Vec<ModelEvent>>>,
}

impl ScriptedProvider {
    fn new(responses: impl IntoIterator<Item = Vec<ModelEvent>>) -> Self {
        Self {
            capabilities: capabilities(),
            responses: Mutex::new(responses.into_iter().collect()),
        }
    }
}

impl ModelProvider for ScriptedProvider {
    fn capabilities(&self) -> ModelCapabilities {
        self.capabilities.clone()
    }

    fn stream(&self, request: ModelRequest) -> Result<ModelEventStream, ModelProviderError> {
        request.validate_capabilities(&self.capabilities)?;
        let events = self.responses.lock().unwrap().pop_front().ok_or_else(|| {
            ModelProviderError::ProviderFailure {
                code: Some("fixture_exhausted".to_owned()),
                message: "no scripted Shadow sample remains".to_owned(),
            }
        })?;
        Ok(Box::pin(stream::iter(
            events
                .into_iter()
                .map(|event| Ok(ModelStreamEvent::Event(event)))
                .chain(std::iter::once(Ok(ModelStreamEvent::Completed(
                    ModelResponseCompletion::without_response_id(),
                )))),
        )))
    }
}

#[tokio::test]
async fn shadow_eval_uses_only_replay_tools_and_remains_non_evidence() {
    let fixture = source_fixture(ToolResultStatus::Succeeded);
    let prepared = prepare_fixture(&fixture);
    let before_session = fixture.session.clone();
    let before_blobs = fixture.blobs.bytes();
    let shadow = ScriptedProvider::new([
        vec![ModelEvent::ToolCall { call: call() }],
        vec![ModelEvent::TextDelta {
            text: "shadow conclusion from replayed output".to_owned(),
        }],
    ]);
    let mut agent = AgentLoop::new(&shadow, prepared.tool_executor());

    let result = prepared
        .shadow_model_eval(&mut agent, "inspect fixture", AgentCancellation::new())
        .await
        .unwrap();

    assert_eq!(result.evaluation().evidence(), EvalEvidence::NonEvidence);
    assert!(result.evaluation().is_non_evidence());
    assert_eq!(result.model_requests(), 2);
    assert_eq!(result.replay_tool_uses().len(), 1);
    assert_eq!(fixture.session, before_session);
    assert_eq!(fixture.blobs.bytes(), before_blobs);
}

#[tokio::test]
async fn shadow_eval_fails_when_recorded_replay_tool_steps_are_not_consumed() {
    let fixture = source_fixture(ToolResultStatus::Succeeded);
    let prepared = prepare_fixture(&fixture);
    let shadow = ScriptedProvider::new([vec![ModelEvent::TextDelta {
        text: "shadow skipped the recorded Tool call".to_owned(),
    }]]);
    let mut agent = AgentLoop::new(&shadow, prepared.tool_executor());

    assert!(matches!(
        prepared
            .shadow_model_eval(&mut agent, "inspect fixture", AgentCancellation::new(),)
            .await,
        Err(
            echidna::app::trace_eval::TraceReplayRunError::ReplayToolsNotFullyConsumed {
                remaining_tool_steps: 1
            }
        )
    ));
}

#[tokio::test]
async fn non_success_source_tool_result_replays_with_its_exact_output_and_cannot_repeat() {
    for status in [ToolResultStatus::Failed, ToolResultStatus::Cancelled] {
        let source_result = result(status);
        let tools = ReplayToolExecutor::new(
            replay_source(),
            [replay_spec()],
            [ReplayToolStep::new(
                EventId::new("non-success-source-receipt"),
                receipt(status),
                call(),
                source_result.clone(),
            )
            .unwrap()],
        )
        .unwrap();
        let plan = tools
            .plan(ExecutionId::new("shadow-execution"), call())
            .unwrap();
        let outcome: ToolExecutionOutcome = tools.execute(plan).await;

        assert_eq!(outcome.result.status, status);
        assert_eq!(outcome.result.output, source_result.output);
        assert_eq!(tools.replay_uses().len(), 1);
        assert!(matches!(
            tools.plan(ExecutionId::new("repeat-execution"), call()),
            Err(echidna::ports::tool::ToolExecutorError::UnmatchedToolCall { .. })
        ));
    }
}

#[tokio::test]
async fn replay_use_keeps_complete_source_receipt_separate_from_derived_receipt() {
    let source = replay_source();
    let mut source_receipt = receipt(ToolResultStatus::Succeeded);
    source_receipt.output_artifact_ids = vec![ArtifactId::new("source-output-artifact")];
    source_receipt.environment_change = Some("historical_environment_change".to_owned());
    source_receipt.rerun_reason = Some(RerunReason::new("historical-rerun"));
    source_receipt.cache_source = Some(ToolCacheSource {
        cache_key: ToolResultCacheKey::new("historical-cache-key"),
        source_stream_id: StreamId::new("session:historical-cache-source"),
        source_receipt_event_id: EventId::new("historical-cache-receipt"),
        source_execution_id: ExecutionId::new("historical-cache-execution"),
        source_plan_fingerprint: "historical-cache-plan".to_owned(),
    });
    source_receipt.cache_validation = Some(ToolCacheValidation {
        cache_resource_fingerprint: "resource-fingerprint".to_owned(),
        observed_before_fingerprint: "resource-fingerprint".to_owned(),
        observed_after_fingerprint: "resource-fingerprint".to_owned(),
    });
    source_receipt.file_mutations = vec![FileMutationAudit {
        path: "historical.txt".to_owned(),
        expected_preimage_hash: Some("expected".to_owned()),
        before_hash: Some("before".to_owned()),
        before_size: Some(1),
        after_hash: Some("after".to_owned()),
        after_size: Some(2),
        actual_diff: "+historical".to_owned(),
    }];
    let tools = ReplayToolExecutor::new(
        source.clone(),
        [replay_spec()],
        [ReplayToolStep::new(
            EventId::new("source-receipt-with-provenance"),
            source_receipt.clone(),
            call(),
            result(ToolResultStatus::Succeeded),
        )
        .unwrap()],
    )
    .unwrap();
    let plan = tools
        .plan(ExecutionId::new("eval-replay-execution"), call())
        .unwrap();
    let outcome = tools.execute(plan).await;
    let replay_use = &tools.replay_uses()[0];

    assert_eq!(replay_use.source(), &source);
    assert_eq!(replay_use.source_receipt(), &source_receipt);
    assert_eq!(replay_use.replay_receipt(), &outcome.receipt);
    assert_eq!(
        replay_use.source_execution_id(),
        &ExecutionId::new("execution:call-read")
    );
    assert_eq!(
        replay_use.replay_execution_id(),
        &ExecutionId::new("eval-replay-execution")
    );
    assert!(outcome.receipt.output_artifact_ids.is_empty());
    assert!(outcome.receipt.environment_change.is_none());
    assert!(outcome.receipt.cache_source.is_none());
    assert!(outcome.receipt.cache_validation.is_none());
    assert!(outcome.receipt.file_mutations.is_empty());
}

#[tokio::test]
#[ignore = "requires explicit CYBER_AGENT_LIVE_TESTS=1 and a configured dedicated test key"]
async fn synthetic_shadow_live_smoke_uses_composition_and_only_fixed_replay_tools() {
    if std::env::var("CYBER_AGENT_LIVE_TESTS").as_deref() != Ok("1") {
        return;
    }
    let fixture = load_fixed_source_fixture();
    let prepared = prepare_fixture(&fixture);
    let composition = live_provider::load();
    let mut agent = AgentLoop::new(
        composition.primary_provider(),
        prepared.tool_executor(),
    )
    .with_prompt_composition(
        PromptComposition::current()
            .with_project("This is an isolated protocol test. First call replay_read exactly once with the JSON arguments {\"path\":\"fixture.txt\"}. After its result, reply with LIVE_SHADOW_OK. Do not call any other Tool.")
            .unwrap(),
    );

    let result = prepared
        .shadow_model_eval(
            &mut agent,
            "Run the fixed isolated Shadow protocol test.",
            AgentCancellation::new(),
        )
        .await
        .expect("authorized synthetic Shadow request should complete");

    assert_eq!(result.replay_tool_uses().len(), 1);
    assert!(result.evaluation().is_non_evidence());
}
