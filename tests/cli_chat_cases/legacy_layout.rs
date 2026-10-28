use super::*;

#[test]
fn legacy_control_directory_routes_cli_memory_blob_cache_and_tool_resources_in_place() {
    let root = TemporaryDirectory::new("legacy-cli-layout");
    initialize_project(root.path());
    fs::rename(
        root.path().join(".echidna"),
        root.path().join(".cyber-agent"),
    )
    .unwrap();
    fs::write(
        root.path().join("legacy-source.txt"),
        "legacy artifact bytes\n",
    )
    .unwrap();

    let target_snapshot = json!({
        "event_id": "legacy-target-snapshot",
        "occurred_at": "2026-10-22T00:00:00Z",
        "operation": {
            "kind": "record_target_snapshot",
            "record": {
                "id": "legacy-snapshot",
                "target_id": "legacy.fixture",
                "kind": "source",
                "locator": "legacy-source.txt",
                "version": "1",
                "build_id": "legacy-layout-fixture",
                "target_artifact_ids": ["legacy-artifact"],
                "environment_fingerprint": "legacy-layout-offline",
                "device_fingerprint": "legacy-layout-offline",
                "captured_at": "2026-10-22T00:00:00Z",
                "notes": "offline legacy control-directory fixture"
            }
        }
    });
    let artifact = json!({
        "event_id": "legacy-artifact-event",
        "occurred_at": "2026-10-22T00:00:01Z",
        "artifact_id": "legacy-artifact",
        "source_path": "legacy-source.txt",
        "media_type": "text/plain",
        "target_snapshot_id": "legacy-snapshot",
        "toolchain_fingerprint": "legacy-layout-fixture"
    });
    let memory_write = json!({
        "path": "core.md",
        "content": "legacy Core Memory\n",
        "expected_hash": null,
        "event_id": "legacy-memory-event"
    });
    let server = spawn_responses_server(vec![
        (
            200,
            m4_long_flow_tool_response(
                "legacy-target-call",
                "research_update",
                target_snapshot,
                "legacy-target-response",
            ),
        ),
        (
            200,
            m4_long_flow_tool_response(
                "legacy-artifact-call",
                "artifact_store",
                artifact,
                "legacy-artifact-response",
            ),
        ),
        (
            200,
            m4_long_flow_tool_response(
                "legacy-memory-call",
                "memory_write",
                memory_write.clone(),
                "legacy-memory-response",
            ),
        ),
        (
            200,
            text_response("legacy layout complete", "legacy-final-response"),
        ),
    ]);
    let config = write_config(root.path(), &server.base_url);
    let output = run_chat(
        root.path(),
        &config,
        "retain the offline legacy fixture\n/exit\n",
        &["--domain", "source", "--phase", "evidence-curation"],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("legacy layout complete"));
    for _ in 0..4 {
        server.requests.recv().unwrap();
    }
    assert!(server.requests.try_recv().is_err());
    server.join();

    let legacy = root.path().join(".cyber-agent");
    assert!(!root.path().join(".echidna").exists());
    assert!(legacy.join("project.toml").is_file());
    assert!(legacy.join("research.jsonl").is_file());
    let research = fs::read_to_string(legacy.join("research.jsonl")).unwrap();
    assert!(research.contains("legacy-memory-event"), "{research}");
    assert_eq!(
        fs::read_to_string(legacy.join("memory/core.md")).unwrap(),
        "legacy Core Memory\n"
    );
    assert_eq!(
        fs::read_dir(legacy.join("sessions")).unwrap().count(),
        1,
        "the CLI Session must remain in the selected legacy control directory"
    );
    let blobs = legacy.join("artifacts/blobs/sha256");
    let blob = blobs.join(format!("{:x}", Sha256::digest(b"legacy artifact bytes\n")));
    assert_eq!(fs::read(blob).unwrap(), b"legacy artifact bytes\n");
    assert!(
        fs::read_dir(legacy.join("cache/context-fragments"))
            .unwrap()
            .next()
            .is_some(),
        "the CLI Context fragment cache must remain under the selected control directory"
    );

    let session = fs::read_dir(legacy.join("sessions"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .file_name()
        .into_string()
        .unwrap();
    let project = Project::discover(root.path()).unwrap();
    let state = reduced_session(&project, &session);
    let delivered_inputs = state
        .request_audits()
        .iter()
        .map(|manifest| {
            manifest.reconstruct(|blob| {
                fs::read(legacy.join("artifacts/blobs/sha256").join(blob.as_str()))
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    for (name, expected_template) in [
        (
            "memory_write",
            "file:.cyber-agent/memory/{path},file:.cyber-agent/research.jsonl",
        ),
        (
            "artifact_store",
            "file:{source_path},file:.cyber-agent/research.jsonl",
        ),
    ] {
        let templates = delivered_inputs
            .iter()
            .flat_map(|input| input.tool_specs.iter())
            .filter(|spec| spec.name.as_str() == name)
            .filter_map(|spec| spec.resource_key_template.as_deref())
            .collect::<Vec<_>>();
        assert!(
            templates.contains(&expected_template),
            "the persisted CLI RequestAudit must record the selected legacy resource template for {name}: {templates:?}"
        );
    }
}
