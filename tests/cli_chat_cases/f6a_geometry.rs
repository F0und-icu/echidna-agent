//! Formal CLI geometry admission, without external Provider access.

use super::*;

#[test]
fn f6a_original_and_resized_images_bind_exact_decoded_geometry() {
    for (width, height) in [(32, 16), (2100, 32)] {
        let root = TemporaryDirectory::new("f6a-cli-geometry");
        initialize_project(root.path());
        let project = Project::discover(root.path()).unwrap();
        append_research_snapshot(&project, "geometry");
        fs::write(root.path().join("screen.png"), png_bytes(width, height)).unwrap();
        let vision = spawn_responses_server(vec![(
            200,
            vision_response("bounded observation", "vision-geometry"),
        )]);
        let primary = spawn_responses_server(vec![(
            200,
            text_response("GEOMETRY_OK", "primary-geometry"),
        )]);
        let config = write_dual_config(root.path(), &primary.base_url, &vision.base_url);
        let output = run_chat(
            root.path(),
            &config,
            "/image screen.png\nDescribe the image.\n/exit\n",
            &[],
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let request = f2a_request_body(&vision.requests, "geometry Vision");
        f2a_join_server(vision, "geometry Vision");
        let primary_request = f2a_request_body(&primary.requests, "geometry Primary");
        f2a_join_server(primary, "geometry Primary");
        let state = reduced_session(&project, &session_id(root.path()));
        let observation = &state.vision_observations()[0];
        let geometry = observation.image_geometry.as_ref().unwrap();
        assert_eq!(
            (geometry.source_width, geometry.source_height),
            (width, height)
        );
        let delivery_bytes = request["input"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|item| item["content"].as_array().into_iter().flatten())
            .find(|item| item["type"] == "input_image")
            .unwrap()["image_url"]
            .as_str()
            .unwrap();
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(delivery_bytes.split_once(',').unwrap().1)
            .unwrap();
        let image = image::load_from_memory(&bytes).unwrap();
        assert_eq!(
            (geometry.delivery_width, geometry.delivery_height),
            (image.width(), image.height())
        );
        if width == 32 {
            assert_eq!(
                geometry.delivery_artifact_id,
                observation.source_artifact_id
            );
        } else {
            assert_ne!(
                geometry.delivery_artifact_id,
                observation.source_artifact_id
            );
            assert!(geometry.delivery_width < geometry.source_width);
        }
        assert!(json_contains_text(
            &request["input"],
            "vision_image_geometry"
        ));
        assert!(json_contains_text(
            &primary_request["input"],
            "delivery_pixels"
        ));
        assert_audits_reconstruct(root.path(), &state);
    }
}

#[test]
fn f6a_out_of_bounds_vision_keeps_failed_attempt_and_never_samples_primary() {
    let root = TemporaryDirectory::new("f6a-cli-invalid-region");
    initialize_project(root.path());
    let project = Project::discover(root.path()).unwrap();
    append_research_snapshot(&project, "geometry-failure");
    fs::write(root.path().join("screen.png"), png_bytes(32, 16)).unwrap();
    let raw = json!({"schema_version": 2, "summary": "Left red, right blue.",
        "extracted_text": null, "regions": [{"label": "left", "x": 0, "y": 0, "width": 240, "height": 240}],
        "task_observations": [], "uncertainty": ["dimensions estimated"]}).to_string();
    let vision =
        spawn_responses_server(vec![(200, text_response(&raw, "vision-invalid-geometry"))]);
    let primary_probe = TcpListener::bind("127.0.0.1:0").unwrap();
    primary_probe.set_nonblocking(true).unwrap();
    let config = write_dual_config(
        root.path(),
        &format!("http://{}/v1", primary_probe.local_addr().unwrap()),
        &vision.base_url,
    );
    let output = run_chat(
        root.path(),
        &config,
        "/image screen.png\nDescribe the image.\n/exit\n",
        &[],
    );
    let _ = f2a_request_body(&vision.requests, "invalid geometry Vision");
    f2a_join_server(vision, "invalid geometry Vision");
    assert_eq!(
        primary_probe.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("pixel bounds"));
    let state = reduced_session(&project, &session_id(root.path()));
    assert!(state.vision_observations().is_empty());
    assert_eq!(state.vision_failures().len(), 1);
    assert_eq!(state.request_audits().len(), 1);
    assert_eq!(
        state.vision_failures()[0].error_code,
        "model_reply_geometry_invalid"
    );
    let raw_hash = format!("{:x}", Sha256::digest(raw.as_bytes()));
    assert!(state.vision_failures()[0].message.contains(&raw_hash));
    assert_eq!(
        fs::read(
            root.path()
                .join(".cyber-agent/artifacts/blobs/sha256")
                .join(raw_hash)
        )
        .unwrap(),
        raw.as_bytes()
    );
    let events = session_events(root.path(), &session_id(root.path()));
    assert_eq!(
        events
            .iter()
            .filter(|event| event["event_type"] == "tool.execution_receipt_recorded")
            .count(),
        1
    );
}
