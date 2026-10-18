#[path = "view_image_tool/blob_publication.rs"]
mod blob_publication;
mod support;

use std::fs;
use std::io::Cursor;

use echidna::adapters::project_event_store::Project;
use echidna::adapters::tools::ToolRegistry;
use echidna::domain::tools::{ExecutionId, ToolCall, ToolCallId, ToolName, ToolResultStatus};
use echidna::domain::vision::ViewImageSelection;
use echidna::ports::tool::ToolExecutor;
use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use support::TemporaryDirectory;

fn initialized_project(label: &str) -> (TemporaryDirectory, Project) {
    let directory = TemporaryDirectory::new(label);
    let project = Project::initialize(directory.path()).unwrap();
    (directory, project)
}

async fn execute(
    tools: &ToolRegistry,
    call_id: &str,
    name: &str,
    arguments: Value,
) -> echidna::domain::tools::ToolExecutionOutcome {
    let plan = tools
        .plan(
            ExecutionId::new(format!("execution-{call_id}")),
            ToolCall::new(ToolCallId::new(call_id), ToolName::new(name), arguments),
        )
        .unwrap();
    tools.execute(plan).await.expect("Tool execution completes")
}

async fn record_target_snapshot(tools: &ToolRegistry) {
    let outcome = execute(
        tools,
        "call-snapshot",
        "research_update",
        json!({
            "event_id": "event-snapshot",
            "occurred_at": "2026-08-26T00:00:00Z",
            "operation": {"kind": "record_target_snapshot", "record": {
                "id": "snapshot-a", "target_id": "fixture", "kind": "source",
                "locator": "fixture/source", "version": "1", "build_id": "1",
                "target_artifact_ids": ["artifact-root"], "environment_fingerprint": "env",
                "device_fingerprint": "device", "captured_at": "2026-08-26T00:00:00Z"
            }}
        }),
    )
    .await;
    assert_eq!(outcome.result.status, ToolResultStatus::Succeeded);
}

fn png_bytes(width: u32, height: u32) -> Vec<u8> {
    let image = RgbaImage::from_pixel(width, height, Rgba([5, 9, 13, 255]));
    let mut bytes = Vec::new();
    DynamicImage::ImageRgba8(image)
        .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
        .unwrap();
    bytes
}

fn view_args(event_id: &str, artifact_id: &str, source_path: &str) -> Value {
    json!({
        "event_id": event_id,
        "occurred_at": "2026-08-26T00:01:00Z",
        "artifact_id": artifact_id,
        "source_path": source_path,
        "target_snapshot_id": "snapshot-a",
        "toolchain_fingerprint": "fixture-toolchain"
    })
}

fn research_events(project: &Project) -> Vec<Value> {
    fs::read_to_string(project.root().join(".cyber-agent/research.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[tokio::test]
async fn small_image_needs_no_caller_guessed_media_type_or_derivative_identity() {
    let (_directory, project) = initialized_project("view-image-small");
    let tools = ToolRegistry::for_project(project.root());
    record_target_snapshot(&tools).await;
    let source = project.root().join("screen.png");
    let source_bytes = png_bytes(32, 32);
    fs::write(&source, &source_bytes).unwrap();

    let outcome = execute(
        &tools,
        "call-view-small",
        "view_image",
        view_args(
            "event-image-small",
            "artifact-image-small",
            &source.display().to_string(),
        ),
    )
    .await;
    assert_eq!(outcome.result.status, ToolResultStatus::Succeeded);
    assert_eq!(outcome.receipt.output_artifact_ids.len(), 1);
    assert_eq!(
        outcome.result.output["image_artifact_ref"],
        "artifact-image-small"
    );
    assert_eq!(
        outcome.result.output["original"]["artifact"]["media_type"],
        "image/png"
    );
    assert_eq!(
        outcome.result.output["original"]["blob"]["blob_id"],
        format!("{:x}", Sha256::digest(&source_bytes))
    );
    assert!(outcome.result.output["derived"].is_null());
    let selection = ViewImageSelection::from_tool_result(&outcome.result)
        .unwrap()
        .expect("a successful view_image result must select one Artifact");
    assert_eq!(
        selection.source_artifact_id.as_str(),
        "artifact-image-small"
    );
    assert_eq!(
        selection.delivery_artifact_id.as_str(),
        "artifact-image-small"
    );
    assert!(selection.derived_artifact_ids.is_empty());
    assert_eq!(research_events(&project).len(), 2);
}

#[tokio::test]
async fn large_image_automatically_publishes_a_lineage_linked_derivative() {
    let (_directory, project) = initialized_project("view-image-derived");
    let tools = ToolRegistry::for_project(project.root());
    record_target_snapshot(&tools).await;
    let source = project.root().join("wide.png");
    let source_bytes = png_bytes(3_000, 10);
    fs::write(&source, &source_bytes).unwrap();

    let outcome = execute(
        &tools,
        "call-view-derived",
        "view_image",
        view_args(
            "event-image-wide",
            "artifact-image-wide",
            &source.display().to_string(),
        ),
    )
    .await;
    assert_eq!(outcome.result.status, ToolResultStatus::Succeeded);
    assert_eq!(outcome.receipt.output_artifact_ids.len(), 2);
    assert_eq!(
        outcome.result.output["image_artifact_ref"],
        "artifact-image-wide:image_budget_v1"
    );
    assert_eq!(
        outcome.result.output["derived"]["artifact"]["parent_artifact_ids"],
        json!(["artifact-image-wide"])
    );
    assert_eq!(
        outcome.result.output["derived"]["artifact"]["transform"],
        "image_budget_v1"
    );
    assert_eq!(
        outcome.result.output["original"]["blob"]["blob_id"],
        format!("{:x}", Sha256::digest(&source_bytes))
    );
    let derived_blob_id = outcome.result.output["derived"]["blob"]["blob_id"]
        .as_str()
        .unwrap();
    let derived_bytes = fs::read(
        project
            .root()
            .join(".cyber-agent/artifacts/blobs/sha256")
            .join(derived_blob_id),
    )
    .unwrap();
    assert_eq!(
        derived_blob_id,
        format!("{:x}", Sha256::digest(&derived_bytes))
    );
    let selection = ViewImageSelection::from_tool_result(&outcome.result)
        .unwrap()
        .expect("a successful view_image result must select one Artifact");
    assert_eq!(selection.source_artifact_id.as_str(), "artifact-image-wide");
    assert_eq!(
        selection.delivery_artifact_id.as_str(),
        "artifact-image-wide:image_budget_v1"
    );
    assert_eq!(
        selection
            .derived_artifact_ids
            .iter()
            .map(|artifact| artifact.as_str())
            .collect::<Vec<_>>(),
        vec!["artifact-image-wide:image_budget_v1"]
    );
    assert_eq!(research_events(&project).len(), 3);
}

#[tokio::test]
async fn non_image_fails_without_blob_or_artifact_facts_and_ordinary_artifact_store_stays_generic()
{
    let (_directory, project) = initialized_project("view-image-invalid");
    let tools = ToolRegistry::for_project(project.root());
    record_target_snapshot(&tools).await;
    let invalid = project.root().join("not-image.bin");
    fs::write(&invalid, b"not an image").unwrap();
    let failed = execute(
        &tools,
        "call-view-invalid",
        "view_image",
        view_args(
            "event-image-invalid",
            "artifact-image-invalid",
            &invalid.display().to_string(),
        ),
    )
    .await;
    assert_eq!(failed.result.status, ToolResultStatus::Failed);
    assert_eq!(research_events(&project).len(), 1);
    assert!(
        !project
            .root()
            .join(".cyber-agent/artifacts/blobs/sha256")
            .exists()
    );

    let png = project.root().join("generic.png");
    fs::write(&png, png_bytes(8, 8)).unwrap();
    let generic = execute(
        &tools,
        "call-generic-artifact",
        "artifact_store",
        json!({
            "event_id": "event-generic", "occurred_at": "2026-08-26T00:02:00Z",
            "artifact_id": "artifact-generic", "source_path": png.display().to_string(),
            "media_type": "application/octet-stream", "target_snapshot_id": "snapshot-a",
            "toolchain_fingerprint": "fixture-toolchain"
        }),
    )
    .await;
    assert_eq!(generic.result.status, ToolResultStatus::Succeeded);
    assert!(generic.result.output["artifact"]["transform"].is_null());
    assert_eq!(
        ViewImageSelection::from_tool_result(&generic.result).unwrap(),
        None
    );
    assert_eq!(research_events(&project).len(), 2);
}
