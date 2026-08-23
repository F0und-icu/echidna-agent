use super::*;

#[test]
fn research_world_state_and_project_snapshot_render_bounded_full_and_diff_context() {
    let previous = ResearchWorldState::default();
    let current = ResearchWorldState::from_sections([
        (
            ResearchWorldStateSection::Target,
            json!({"package": "com.fixture.target", "notes": "x".repeat(4_096)}),
        ),
        (
            ResearchWorldStateSection::ActiveHypotheses,
            json!([{"id": "hypothesis-1", "status": "active"}]),
        ),
    ])
    .unwrap();
    let full = current.full_fragment().unwrap();
    assert!(full.token_estimate() < 2_500);
    let diff = current.diff_fragment(&previous).unwrap().unwrap();
    assert!(diff.token_estimate() < 1_000);
    assert!(current.diff_fragment(&current).unwrap().is_none());
    assert_eq!(
        current.section(ResearchWorldStateSection::Target).id.id(),
        "target"
    );

    let snapshot = ProjectSnapshot {
        cache_schema_version: 1,
        project_id: ProjectId::new("project-m3"),
        source_fingerprint: ProjectFingerprint {
            kind: ProjectFingerprintKind::Filesystem,
            digest: "fixture-fingerprint".to_owned(),
            cacheable: true,
        },
        created_at: "2026-08-23T00:00:00Z".to_owned(),
        rebuild_reason: "fixture".to_owned(),
        file_manifest_hash: "files-a".to_owned(),
        tool_capability_hash: "tools-a".to_owned(),
        file_count: 4,
    };
    let first = ProjectSnapshotContext::new(snapshot.clone()).unwrap();
    let mut volatile_change = snapshot.clone();
    volatile_change.created_at = "2026-08-23T12:00:00Z".to_owned();
    volatile_change.rebuild_reason = "different cache refresh".to_owned();
    assert_eq!(
        first.snapshot_hash(),
        ProjectSnapshotContext::new(volatile_change.clone())
            .unwrap()
            .snapshot_hash()
    );
    let volatile_context = ProjectSnapshotContext::new(volatile_change).unwrap();
    assert_eq!(
        first.full_fragment().unwrap(),
        volatile_context.full_fragment().unwrap()
    );
    assert!(volatile_context.diff_fragment(&first).unwrap().is_none());
    let mut changed = snapshot;
    changed.file_count = 5;
    let second = ProjectSnapshotContext::new(changed).unwrap();
    assert_ne!(first.snapshot_hash(), second.snapshot_hash());
    assert!(second.diff_fragment(&first).unwrap().is_some());
    assert!(first.diff_fragment(&first).unwrap().is_none());

    let multibyte = ResearchWorldState::from_sections([(
        ResearchWorldStateSection::Target,
        json!({"text": format!("{}é", "a".repeat(374))}),
    )])
    .unwrap();
    let fragment = multibyte.full_fragment().unwrap();
    let ContextFragmentContent::ContentItems { items } = fragment.content() else {
        panic!("WorldState is a content-item fragment");
    };
    let ContentItem::Text { text, .. } = &items[0] else {
        panic!("WorldState fragment is text");
    };
    let rendered: serde_json::Value = serde_json::from_str(text).unwrap();
    let prefix = rendered["sections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|section| section["section_id"] == "target")
        .unwrap()["snapshot"]["rendered_prefix"]
        .as_str()
        .unwrap();
    assert!(
        prefix.len() <= 384,
        "the UTF-8 prefix must not cross the byte cap"
    );
    assert!(matches!(
        ResearchWorldState::from_sections([
            (ResearchWorldStateSection::Target, json!({"first": true})),
            (ResearchWorldStateSection::Target, json!({"second": true})),
        ]),
        Err(
            cyber_agent::domain::world_state::WorldStateError::DuplicateSection {
                id: ResearchWorldStateSection::Target
            }
        )
    ));
}
