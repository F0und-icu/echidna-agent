use std::io::Cursor;

use echidna::adapters::image::prepare_local_image;
use echidna::domain::artifacts::ArtifactId;
use echidna::domain::context::{
    ModelProfile, ProviderSessionId, ProviderSessionRef, ProviderSessionRole,
};
use echidna::domain::events::EventId;
use echidna::domain::image::{ImageDeliveryBudget, ImageInputLimits};
use echidna::domain::model::{CapabilityMappingVersion, ModelCapabilities};
use echidna::domain::profile::{VisionMode, VisionRoute, VisionRouteReason, resolve_vision_route};
use echidna::domain::vision::{VISION_FACT_SCHEMA_VERSION, VisionObservation, VisionObservationId};
use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};

mod support;

fn limits() -> ImageInputLimits {
    ImageInputLimits::new(4 * 1024 * 1024, 2_000, 2_000, 4_000_000)
}

fn png_bytes(width: u32, height: u32) -> Vec<u8> {
    let image = RgbaImage::from_fn(width, height, |x, y| {
        Rgba([(x % 251) as u8, (y % 251) as u8, ((x + y) % 251) as u8, 255])
    });
    let mut bytes = Vec::new();
    DynamicImage::ImageRgba8(image)
        .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
        .unwrap();
    bytes
}

fn noisy_png_bytes(width: u32, height: u32) -> Vec<u8> {
    let image = RgbaImage::from_fn(width, height, |x, y| {
        let seed = x
            .wrapping_mul(1_664_525)
            .wrapping_add(y.wrapping_mul(1_013_904_223));
        Rgba([
            seed as u8,
            seed.rotate_left(9) as u8,
            seed.rotate_left(17) as u8,
            255,
        ])
    });
    let mut bytes = Vec::new();
    DynamicImage::ImageRgba8(image)
        .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
        .unwrap();
    bytes
}

fn jpeg_bytes() -> Vec<u8> {
    let image = RgbaImage::from_pixel(8, 8, Rgba([12, 34, 56, 255]));
    let mut bytes = Vec::new();
    DynamicImage::ImageRgba8(image)
        .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Jpeg)
        .unwrap();
    bytes
}

#[test]
fn png_and_jpeg_are_accepted_and_budget_derivation_is_stable() {
    let root = support::TemporaryDirectory::new("m3-image-prepare");
    let png_path = root.path().join("screen.png");
    let jpeg_path = root.path().join("screen.jpg");
    let original_png = png_bytes(320, 240);
    std::fs::write(&png_path, &original_png).unwrap();
    std::fs::write(&jpeg_path, jpeg_bytes()).unwrap();

    let budget = ImageDeliveryBudget::new(32 * 1024, 64, 64, 4_096);
    let first = prepare_local_image(&png_path, limits(), budget).unwrap();
    let second = prepare_local_image(&png_path, limits(), budget).unwrap();
    let derivative = first
        .derivative
        .as_ref()
        .expect("large PNG requires derivative");
    assert_eq!(first.original.bytes, original_png);
    assert_eq!(first.transform(), Some("image_budget_v1"));
    assert_eq!(first.derivative, second.derivative);
    assert!(derivative.metadata.width <= 64);
    assert!(derivative.metadata.height <= 64);
    assert!(derivative.metadata.size_bytes <= 32 * 1024);
    assert!(prepare_local_image(&jpeg_path, limits(), budget).is_ok());
}

#[test]
fn unsupported_corrupt_and_oversized_images_fail_explicitly() {
    let root = support::TemporaryDirectory::new("m3-image-reject");
    let text = root.path().join("input.gif");
    let corrupt = root.path().join("corrupt.png");
    let large = root.path().join("large.png");
    std::fs::write(&text, b"not an image").unwrap();
    std::fs::write(&corrupt, b"\x89PNG\r\n\x1a\nnot-a-real-png").unwrap();
    std::fs::write(&large, png_bytes(100, 100)).unwrap();
    let budget = ImageDeliveryBudget::new(64 * 1024, 128, 128, 16_384);
    assert!(
        prepare_local_image(&text, limits(), budget)
            .unwrap_err()
            .to_string()
            .contains("corrupt")
    );
    assert!(prepare_local_image(&corrupt, limits(), budget).is_err());
    assert!(
        prepare_local_image(
            &large,
            ImageInputLimits::new(100, 2_000, 2_000, 4_000_000),
            budget,
        )
        .unwrap_err()
        .to_string()
        .contains("exceeding")
    );
}

#[test]
fn oversized_header_is_rejected_before_corrupt_pixel_data_is_decoded() {
    let root = support::TemporaryDirectory::new("m3-image-header-limit");
    let path = root.path().join("oversized-corrupt.png");
    let mut bytes = png_bytes(2_001, 1);
    let idat = bytes
        .windows(4)
        .position(|window| window == b"IDAT")
        .expect("fixture PNG must contain IDAT");
    bytes[idat + 4] ^= 0xff; // Keep the header parseable but corrupt pixel data.
    std::fs::write(&path, bytes).unwrap();

    let error = prepare_local_image(
        &path,
        limits(),
        ImageDeliveryBudget::new(64 * 1024, 128, 128, 16_384),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("dimensions 2001x1 exceed"), "{error}");
}

#[test]
fn byte_budget_retries_with_smaller_dimensions_until_the_png_fits() {
    let root = support::TemporaryDirectory::new("m3-image-byte-budget");
    let path = root.path().join("noisy.png");
    std::fs::write(&path, noisy_png_bytes(256, 256)).unwrap();
    let prepared = prepare_local_image(
        &path,
        limits(),
        ImageDeliveryBudget::new(4_096, 256, 256, 65_536),
    )
    .unwrap();
    let derivative = prepared.derivative.unwrap();
    assert!(derivative.metadata.size_bytes <= 4_096);
    assert!(derivative.metadata.width < 256);
    assert!(derivative.metadata.height < 256);
}

#[cfg(unix)]
#[test]
fn symlinked_image_is_rejected_without_following_it() {
    use std::os::unix::fs::symlink;

    let root = support::TemporaryDirectory::new("m3-image-symlink");
    let original = root.path().join("original.png");
    let link = root.path().join("link.png");
    std::fs::write(&original, png_bytes(4, 4)).unwrap();
    symlink(&original, &link).unwrap();
    assert!(
        prepare_local_image(
            &link,
            limits(),
            ImageDeliveryBudget::new(64 * 1024, 64, 64, 4_096)
        )
        .unwrap_err()
        .to_string()
        .contains("symbolic link")
    );
}

fn capabilities(image_input: bool, structured_output: bool) -> ModelCapabilities {
    ModelCapabilities {
        text_input: true,
        image_input,
        streaming: true,
        tool_call: true,
        structured_output,
        incremental_input: false,
        usage_reporting: false,
        max_output_tokens: None,
        context_window: 128_000,
        capability_mapping_version: CapabilityMappingVersion::new(1),
    }
}

#[test]
fn route_matrix_and_secondary_context_are_deterministic() {
    let primary = capabilities(true, true);
    let text_primary = capabilities(false, true);
    let vision = capabilities(true, true);
    assert_eq!(
        resolve_vision_route(VisionMode::Auto, &primary, Some(&vision), true),
        echidna::domain::profile::VisionRouteDecision {
            route: VisionRoute::PrimaryDirect,
            reason: Some(VisionRouteReason::PrimaryImageCapable),
        }
    );
    assert_eq!(
        resolve_vision_route(VisionMode::Auto, &text_primary, Some(&vision), true).route,
        VisionRoute::SecondaryTool
    );
    assert_eq!(
        resolve_vision_route(VisionMode::Disabled, &primary, Some(&vision), true).reason,
        Some(VisionRouteReason::VisionDisabled)
    );

    let observation = VisionObservation {
        schema_version: VISION_FACT_SCHEMA_VERSION,
        id: VisionObservationId::new("observation:1"),
        route_event_id: EventId::new("event:route"),
        source_artifact_id: ArtifactId::new("artifact:source"),
        derived_artifact_ids: vec![ArtifactId::new("artifact:derived")],
        image_geometry: None,
        task_hash: "task-hash".to_owned(),
        summary: "login view has a debug control".to_owned(),
        extracted_text: Some("debug".to_owned()),
        regions: Vec::new(),
        security_relevant_clues: Some(vec!["debug affordance".to_owned()]),
        task_observations: None,
        uncertainty: vec!["requires runtime validation".to_owned()],
        provider_profile: ModelProfile::new("vision", 1),
        capability_hash: "capability-hash".to_owned(),
        provider_session: ProviderSessionRef {
            id: ProviderSessionId::new("vision:1"),
            role: ProviderSessionRole::Vision,
            generation: 1,
        },
        request_audit_event_id: EventId::new("event:audit"),
        request_input_hash: "request-input-hash".to_owned(),
        raw_observation_blob_id: echidna::domain::artifacts::BlobId::new("a".repeat(64)).unwrap(),
    };
    let first = observation.primary_context_text().unwrap();
    assert_eq!(first, observation.primary_context_text().unwrap());
    assert!(first.contains("unverified_model_observation"));
    assert!(first.contains("artifact:derived"));
}
