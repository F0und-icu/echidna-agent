//! Coordinate regressions through the real scoped service, facts and Replay.

use super::*;
use echidna::app::vision::VisionSessionOutcome;
use echidna::domain::vision::{VISION_OBSERVATION_SCHEMA_VERSION, VisionObservation};
use serde_json::{Value, json};

struct Run {
    result: Result<VisionSessionOutcome, VisionSessionError>,
    requests: Vec<ModelRequest>,
    projection: SessionProjection,
    sink: EventLog,
    blobs: MemoryBlobs,
    raw: String,
}

async fn run(input: VisionAnalysisInput, regions: Value) -> Run {
    let raw = json!({
        "schema_version": 2, "summary": "Synthetic colors only.",
        "extracted_text": null, "regions": regions,
        "task_observations": [], "uncertainty": ["not verified evidence"]
    })
    .to_string();
    run_reply(input, raw).await
}

async fn run_reply(input: VisionAnalysisInput, raw: String) -> Run {
    let vision = capabilities(true, false);
    let provider = ScriptedProvider::new(vision.clone(), [completed(&raw, "geometry-response")]);
    let binding = binding();
    let mut projection = projection(&binding);
    let mut sink = EventLog::default();
    sink.events.push(started_event(&binding));
    let blobs = MemoryBlobs::default();
    let service = VisionSessionService::new(&blobs, binding.clone());
    let result = service
        .analyze_image(
            &provider,
            VisionSecondaryRequest {
                route: route(
                    "vision-route",
                    "artifact:original",
                    &capabilities(false, false),
                    &vision,
                    VisionMode::SecondaryTool,
                    VisionRoute::SecondaryTool,
                    VisionRouteReason::ForcedSecondaryTool,
                ),
                route_occurred_at: occurred_at(2),
                analysis: input,
                audit: VisionEventStamp::new(EventId::new("vision-audit"), occurred_at(3)),
                terminal: VisionEventStamp::new(EventId::new("vision-terminal"), occurred_at(4)),
            },
            &audit_context(binding),
            AgentCancellation::new(),
            &mut projection,
            &mut sink,
        )
        .await;
    Run {
        result,
        requests: provider.requests(),
        projection,
        sink,
        blobs,
        raw,
    }
}

#[tokio::test]
async fn neutral_observations_keep_versioned_fields_and_legacy_semantics_separate() {
    let input = vision_input(&capabilities(true, false));
    let run = run(input, json!([])).await;
    let observation = &run.result.as_ref().unwrap().observation;
    let current = serde_json::to_value(observation).unwrap();
    assert_eq!(current["schema_version"], 3);
    assert!(current.get("security_relevant_clues").is_none());
    assert_eq!(current["task_observations"], json!([]));
    let context = observation.primary_context_text().unwrap();
    assert!(context.contains("task_observations"));
    assert!(!context.contains("security_relevant_clues"));

    let mut legacy = current.clone();
    legacy["schema_version"] = json!(2);
    legacy.as_object_mut().unwrap().remove("task_observations");
    legacy["security_relevant_clues"] = json!(["old security-specific interpretation"]);
    let decoded: VisionObservation = serde_json::from_value(legacy.clone()).unwrap();
    decoded.validate().unwrap();
    assert_eq!(serde_json::to_value(&decoded).unwrap(), legacy);
    assert!(
        decoded
            .primary_context_text()
            .unwrap()
            .contains("security_relevant_clues")
    );
    assert!(
        !decoded
            .primary_context_text()
            .unwrap()
            .contains("task_observations")
    );
    for mut malformed in [current.clone(), legacy.clone()] {
        malformed["task_observations"] = json!([]);
        malformed["security_relevant_clues"] = json!([]);
        assert!(
            serde_json::from_value::<VisionObservation>(malformed)
                .unwrap()
                .validate()
                .is_err()
        );
    }
    let mut missing = current;
    missing.as_object_mut().unwrap().remove("task_observations");
    assert!(
        serde_json::from_value::<VisionObservation>(missing)
            .unwrap()
            .validate()
            .is_err()
    );
    let mut null_legacy_field = serde_json::to_value(observation).unwrap();
    null_legacy_field["security_relevant_clues"] = Value::Null;
    assert!(serde_json::from_value::<VisionObservation>(null_legacy_field).is_err());
}

#[tokio::test]
async fn new_secondary_requests_reject_legacy_and_incomplete_reply_schemas_once() {
    for raw in [
        json!({"schema_version": 1, "summary": "legacy", "security_relevant_clues": []}),
        json!({"schema_version": 2, "summary": "mixed", "extracted_text": null, "regions": [], "task_observations": [], "security_relevant_clues": null, "uncertainty": []}),
        json!({"schema_version": 2, "summary": "missing regions", "extracted_text": null, "task_observations": [], "uncertainty": []}),
    ] {
        let run = run_reply(vision_input(&capabilities(true, false)), raw.to_string()).await;
        assert!(run.result.is_err());
        assert_eq!(run.requests.len(), 1);
        assert!(run.projection.state().vision_observations().is_empty());
        assert_eq!(run.projection.state().vision_failures().len(), 1);
        assert_eq!(run.projection.state().request_audits().len(), 1);
    }
}

fn region(x: u32, y: u32, width: u32, height: u32) -> Value {
    json!({"label": "region", "x": x, "y": y, "width": width, "height": height})
}

#[tokio::test]
async fn delivered_pixel_edges_empty_regions_and_original_geometry_are_auditable() {
    for regions in [
        json!([]),
        json!([region(0, 0, 600, 400)]),
        json!([region(599, 399, 1, 1)]),
    ] {
        let input = vision_input(&capabilities(true, false));
        let geometry = input.image_geometry.clone();
        let run = run(input, regions).await;
        let observation = &run.result.as_ref().unwrap().observation;
        assert_eq!(
            observation.schema_version,
            VISION_OBSERVATION_SCHEMA_VERSION
        );
        assert_eq!(observation.image_geometry.as_ref(), Some(&geometry));
        assert_eq!(run.requests.len(), 1);
        assert!(
            run.requests[0]
                .input()
                .instructions
                .contains("Do not use original-image")
        );
        let metadata = run.requests[0]
            .input()
            .content_items
            .iter()
            .find_map(|item| match item {
                ContentItem::Text { text, .. } => serde_json::from_str::<Value>(text).ok(),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            metadata["image_geometry"],
            serde_json::to_value(&geometry).unwrap()
        );
        assert_eq!(metadata["region_coordinate_space"], "delivery_pixels");
        assert_eq!(
            run.blobs
                .read_blob(&observation.raw_observation_blob_id)
                .unwrap(),
            run.raw.as_bytes()
        );
        let before = run.blobs.writes();
        let replayed = VisionReplayService::new(&run.blobs)
            .replay_secondary(
                run.projection.state(),
                &observation.route_event_id,
                &observation.source_artifact_id,
            )
            .unwrap();
        assert_eq!(&replayed, observation);
        assert_eq!(run.blobs.writes(), before);
        let context: Value =
            serde_json::from_str(&observation.primary_context_text().unwrap()).unwrap();
        assert_eq!(context["region_coordinate_space"], "delivery_pixels");
        assert_eq!(context["evidence_status"], "unverified_model_observation");
    }
    let mut input = vision_input(&capabilities(true, false));
    input.delivery_artifact_id = input.source_artifact_id.clone();
    input.derived_artifact_ids.clear();
    input.image_geometry.delivery_artifact_id = input.source_artifact_id.clone();
    input.image_geometry.delivery_width = 1200;
    input.image_geometry.delivery_height = 800;
    assert!(
        run(input, json!([region(1199, 799, 1, 1)]))
            .await
            .result
            .is_ok()
    );
}

#[tokio::test]
async fn invalid_regions_fail_once_keep_raw_blob_and_never_create_an_observation() {
    for invalid in [
        region(600, 0, 1, 1),
        region(0, 400, 1, 1),
        region(599, 0, 2, 1),
        region(0, 399, 1, 2),
        region(u32::MAX, 0, 2, 1),
        region(0, u32::MAX, 1, 2),
        region(0, 0, 0, 1),
        region(0, 0, 1, 0),
        region(0, 0, 1200, 800),
    ] {
        let run = run(vision_input(&capabilities(true, false)), json!([invalid])).await;
        let Err(VisionSessionError::AnalysisFailed { failure }) = &run.result else {
            panic!("out-of-bounds result must be an explicit failed attempt");
        };
        assert_eq!(failure.error_code, "model_reply_geometry_invalid");
        assert_eq!(run.requests.len(), 1, "no implicit retry");
        assert!(run.projection.state().vision_observations().is_empty());
        assert_eq!(run.projection.state().vision_failures().len(), 1);
        assert_eq!(run.projection.state().request_audits().len(), 1);
        let raw_id = BlobId::new(format!("{:x}", Sha256::digest(run.raw.as_bytes()))).unwrap();
        assert!(failure.message.contains(raw_id.as_str()));
        assert_eq!(run.blobs.read_blob(&raw_id).unwrap(), run.raw.as_bytes());
        assert_eq!(
            VisionReplayService::new(&run.blobs).replay_secondary(
                run.projection.state(),
                &EventId::new("vision-route"),
                &ArtifactId::new("artifact:original")
            ),
            Err(VisionReplayError::RecordedFailure)
        );
    }
}

#[tokio::test]
async fn malformed_frames_stop_before_provider_or_request_audit() {
    for index in 0..5 {
        let mut input = vision_input(&capabilities(true, false));
        match index {
            0 => input.image_geometry.delivery_width = 0,
            1 => input.image_geometry.source_height = 0,
            2 => input.image_geometry.delivery_width = 1201,
            3 => input.image_geometry.delivery_artifact_id = ArtifactId::new("wrong-artifact"),
            4 => {
                input.delivery_artifact_id = input.source_artifact_id.clone();
                input.image_geometry.delivery_artifact_id = input.source_artifact_id.clone();
            }
            _ => unreachable!(),
        }
        let run = run(input, json!([])).await;
        assert!(run.result.is_err());
        assert!(run.requests.is_empty());
        assert!(run.projection.state().request_audits().is_empty());
        assert!(run.projection.state().vision_observations().is_empty());
    }
}

#[tokio::test]
async fn legacy_coordinate_uncertainty_and_new_frame_replay_tampering_are_explicit() {
    let mut input = vision_input(&capabilities(true, false));
    let mut fake = input.image_geometry.clone();
    fake.delivery_width = 601;
    input.task = json!({
        "kind": "vision_image_geometry", "schema_version": 1,
        "source_artifact_id": input.source_artifact_id,
        "region_coordinate_space": "delivery_pixels", "image_geometry": fake,
    })
    .to_string();
    let run = run(input, json!([])).await;
    let observation = &run.result.as_ref().unwrap().observation;
    let index = run
        .sink
        .events
        .iter()
        .position(|event| event.event_type.as_str() == "vision.observation_recorded")
        .unwrap();
    let mut forged = run.sink.events.clone();
    forged[index].payload["image_geometry"]["delivery_width"] = json!(601);
    let state = SessionProjection::replay(forged).unwrap();
    assert_eq!(
        VisionReplayService::new(&run.blobs).replay_secondary(
            state.state(),
            &observation.route_event_id,
            &observation.source_artifact_id,
        ),
        Err(VisionReplayError::GeometryAuditMismatch)
    );

    let mut legacy = run.sink.events.clone();
    legacy[index].payload["schema_version"] = json!(1);
    let notes = legacy[index]
        .payload
        .as_object_mut()
        .unwrap()
        .remove("task_observations")
        .unwrap();
    legacy[index].payload["security_relevant_clues"] = notes;
    let mut legacy_reply: Value = serde_json::from_str(&run.raw).unwrap();
    legacy_reply["schema_version"] = json!(1);
    let notes = legacy_reply
        .as_object_mut()
        .unwrap()
        .remove("task_observations")
        .unwrap();
    legacy_reply["security_relevant_clues"] = notes;
    let raw = serde_json::to_vec(&legacy_reply).unwrap();
    let blob = run.blobs.publish_bytes(&raw).unwrap();
    legacy[index].payload["raw_observation_blob_id"] = json!(blob.blob_id);
    legacy[index]
        .payload
        .as_object_mut()
        .unwrap()
        .remove("image_geometry");
    let state = SessionProjection::replay(legacy).unwrap();
    let legacy = VisionReplayService::new(&run.blobs)
        .replay_secondary(
            state.state(),
            &observation.route_event_id,
            &observation.source_artifact_id,
        )
        .unwrap();
    assert_eq!(legacy.schema_version, 1);
    assert!(legacy.image_geometry.is_none());
    assert!(
        legacy
            .primary_context_text()
            .unwrap()
            .contains("unknown_legacy")
    );
    assert!(
        serde_json::to_value(&legacy)
            .unwrap()
            .get("image_geometry")
            .is_none()
    );

    for (version, geometry) in [
        (2, Value::Null),
        (
            1,
            serde_json::to_value(&observation.image_geometry).unwrap(),
        ),
        (3, Value::Null),
    ] {
        let mut payload = serde_json::to_value(observation).unwrap();
        payload["schema_version"] = json!(version);
        payload["image_geometry"] = geometry;
        assert!(
            serde_json::from_value::<VisionObservation>(payload)
                .unwrap()
                .validate()
                .is_err()
        );
    }
}
