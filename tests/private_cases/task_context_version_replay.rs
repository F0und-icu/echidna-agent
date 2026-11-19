//! Replay a genuine old multi-objective source without relocating its identity.
use std::fs;

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::adapters::artifact_store::LocalArtifactStore;
use crate::adapters::project_event_store::Project;
use crate::app::session::SessionProjection;
use crate::domain::events::EventEnvelope;
use crate::domain::project::ProjectId;
use crate::domain::research::ResearchReferenceCatalog;
use crate::ports::artifact::ArtifactStore;
use crate::support::TemporaryDirectory;

#[test]
fn compact_selector_preserves_candidate_material_and_old_request_bytes() {
    use crate::domain::model::ContentItem;
    use crate::domain::session::{SessionSourceIdentity, TaskContextCapture};
    let blobs = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/task_context_proof_v5_compaction/blobs");
    let capture_id = "204549b14032d96cc3fb284269ec599760e484197b2d97208c3326c07fe13f99";
    let capture: TaskContextCapture =
        serde_json::from_slice(&fs::read(blobs.join(capture_id)).unwrap()).unwrap();
    let events = fs::read_to_string(
        blobs.join("ef7d3b1e1b6b3eeb97f41ee991bc0e108da243ebcf0aada8e89fb0d811f2bde4"),
    )
    .unwrap();
    let event = events
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .find(|e| {
            e["event_type"] == "session.task_context_selected"
                && e["payload"]["capture_blob"] == capture_id
        })
        .unwrap();
    let source: SessionSourceIdentity =
        serde_json::from_value(event["payload"]["source"].clone()).unwrap();
    let render = |version, candidates: &crate::domain::task_context::SelectionCandidates| {
        let plan = crate::app::agent_loop::compile_selection(
            &crate::domain::context::ContextPlanner::current()
                .with_budget_policy(capture.common.policy),
            &capture.common.capabilities,
            &source,
            candidates,
            Some(version),
            None,
        )
        .unwrap();
        let [ContentItem::Text { text, .. }] = plan.input_items() else {
            panic!("expected selector candidate message")
        };
        let json = text
            .strip_prefix(crate::domain::task_context::SELECTION_INPUT_MARKER)
            .unwrap();
        (plan.base_instructions().to_owned(), json.to_owned())
    };
    let (old_prompt, old_json) = render(2, &capture.candidates);
    let (prompt, json) = render(3, &capture.candidates);
    // Actual generation-2 instruction blob identity before this change.
    assert_eq!(
        format!("{:x}", Sha256::digest(old_prompt.as_bytes())),
        "f788d2aa5fcccdc056a0132d7168f09f9fc6a9c4ca70dd31fe95b03a7c4282e2"
    );
    let original = serde_json::to_value(&capture.candidates).unwrap();
    assert_eq!(old_json, original.to_string());
    let example = prompt
        .lines()
        .find(|line| line.starts_with("{\"relation\""))
        .unwrap();
    serde_json::from_str::<crate::domain::task_context::TaskSelectionDecision>(example).unwrap();
    assert!(prompt.contains("For compose or unresolved use either focus."));
    assert!(prompt.len() < old_prompt.len() && json.len() < old_json.len());
    let mut restored: Value = serde_json::from_str(&json).unwrap();
    for entry in restored["context"].as_array_mut().unwrap() {
        if entry.get("source_event_ref").is_none() {
            entry["source_event_ref"] = entry["ref"].clone();
        }
    }
    assert_eq!(restored, original);

    let mut distinct = capture.candidates.clone();
    let index = distinct
        .context
        .iter()
        .position(|c| c.source_event_ref.is_some())
        .unwrap();
    distinct.context[index].reference = "constraint:fixture-range".into();
    let (_, json) = render(3, &distinct);
    let wire: Value = serde_json::from_str(&json).unwrap();
    assert_eq!(
        wire["context"][index]["source_event_ref"],
        original["context"][index]["source_event_ref"]
    );
}

#[test]
fn proof_v1_multi_objective_sources_replay_with_original_attribution() {
    replay_fixture(
        "task_context_proof_v1",
        "C33a",
        "1020df128eabe4af691168b8fdddb08f2e93d32ce7506026ab2cd7941e27d89d",
        1,
    );
}

#[test]
fn proof_v0_multi_objective_sources_replay_with_original_attribution() {
    replay_fixture(
        "task_context_proof_v0",
        "C19",
        "25e1dc8d315b0802fc3df83e4e5e9a6ca5f777d3a2b8d051c98be9d7f2be3567",
        0,
    );
}

#[test]
fn proof_v2_multi_objective_sources_replay_with_original_attribution() {
    replay_fixture(
        "task_context_proof_v2",
        "C39",
        "c75bfda11b6148ca77a484b67a97f69967258e5027ee19d53d090598f1c59e8d",
        2,
    );
}

#[test]
fn proof_v2_shared_excerpt_replays_original_missing_progress() {
    replay_fixture(
        "task_context_proof_v2_shared_defect",
        "C39",
        "c75bfda11b6148ca77a484b67a97f69967258e5027ee19d53d090598f1c59e8d",
        2,
    );
}

#[test]
fn genuine_c40_v3_upgrade_replays_with_original_v2_prefix() {
    replay_fixture(
        "task_context_proof_v3",
        "C40-v2-to-v3",
        "ddc567d83c2db871d52bee52fc336a0242a3a1b90795f67c7f4e62f4c1ff205c",
        3,
    );
}

#[test]
fn genuine_c44_v3_source_identity_defect_replays_without_rewriting_history() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/task_context_proof_v3_source_identity");
    let manifest: Value =
        serde_json::from_slice(&fs::read(fixture.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["source_candidate"], "C44");
    assert_eq!(
        manifest["binary_sha256"],
        "80643c0b34754eb4a1ed58d3b2e6740d6f963f388ae34ed2099aa1882028a851"
    );
    let root = TemporaryDirectory::new("task-proof-v3-source-identity");
    let project = Project::initialize(root.path()).unwrap();
    let store = LocalArtifactStore::for_project(&project);
    let mut old_events = None;
    for (relative, digest) in manifest["files"].as_object().unwrap() {
        let digest = digest.as_str().unwrap();
        let bytes = fs::read(fixture.join("blobs").join(digest)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(&bytes)), digest);
        if relative.starts_with(".echidna/artifacts/blobs/sha256/") {
            assert_eq!(
                store.publish_bytes(&bytes).unwrap().blob_id.as_str(),
                digest
            );
        } else if relative.ends_with("/events.jsonl") {
            old_events = Some(bytes);
        } else if relative == "effect-count.txt" {
            assert_eq!(bytes, b"one\n");
        }
    }
    let events: Vec<EventEnvelope<Value>> = std::str::from_utf8(&old_events.unwrap())
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let selections = events
        .iter()
        .filter(|event| event.event_type.as_str() == "session.task_context_selected")
        .collect::<Vec<_>>();
    assert!(!selections.is_empty());
    assert!(
        selections
            .iter()
            .all(|event| event.payload["request_proof_version"] == 3)
    );
    assert_eq!(
        events
            .iter()
            .filter(
                |event| event.event_type.as_str() == "tool.execution_outcome_recorded"
                    && event.payload["result"]["call_id"] == "report-call"
            )
            .count(),
        1
    );
    let projection = SessionProjection::replay_project_verified(
        ProjectId::new(manifest["project_id"].as_str().unwrap()),
        &events,
        &[],
        || Ok(ResearchReferenceCatalog::default()),
        &store,
    )
    .unwrap();
    let input = projection
        .state()
        .request_audits()
        .last()
        .unwrap()
        .reconstruct_full_logical(|blob| store.read_blob(blob))
        .unwrap()
        .unwrap();
    let input = serde_json::to_string(input.input()).unwrap();
    assert!(!input.contains("red team report found"));
    assert!(!input.contains("red team report still needs independent verification"));
    assert!(input.contains("Use blue instead."));
}

#[test]
fn genuine_c45f_proof4_replays_original_equal_gap_projection() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/task_context_proof_v4_distinct_gaps");
    let manifest_bytes = fs::read(fixture.join("manifest.json")).unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(&manifest_bytes)),
        "4e4ce01367aab48fa05d946f82477a912f65bdf705199f32dbdab64879b0d84e"
    );
    let manifest: Value = serde_json::from_slice(&manifest_bytes).unwrap();
    assert_eq!(manifest["source_candidate"], "C45f");
    assert_eq!(
        manifest["binary_sha256"],
        "9b38f19bcd3543a84d3e15700f72b74e752383c197abe2d954057df6d7ab368c"
    );
    let root = TemporaryDirectory::new("task-proof-v4-equal-gaps");
    let project = Project::initialize(root.path()).unwrap();
    let store = LocalArtifactStore::for_project(&project);
    let mut old_events = None;
    for (relative, digest) in manifest["files"].as_object().unwrap() {
        let digest = digest.as_str().unwrap();
        let bytes = fs::read(fixture.join("blobs").join(digest)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(&bytes)), digest);
        if relative.starts_with(".echidna/artifacts/blobs/sha256/") {
            assert_eq!(
                store.publish_bytes(&bytes).unwrap().blob_id.as_str(),
                digest
            );
        } else if relative.ends_with("/events.jsonl") {
            assert_eq!(bytes.len(), 151714);
            old_events = Some(bytes);
        }
    }
    let events: Vec<EventEnvelope<Value>> = std::str::from_utf8(&old_events.unwrap())
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let refs = manifest["expected_distinct_selection_refs"]
        .as_array()
        .unwrap();
    assert_eq!(refs.len(), 2);
    for reference in refs {
        assert!(events.iter().any(|event| {
            event.event_id.as_str() == reference.as_str().unwrap()
                && event.payload["request_proof_version"] == 4
                && event.payload["unresolved_gaps"]
                    .to_string()
                    .contains("SAME_WORDS_DISTINCT_UNRESOLVED_SOURCE")
        }));
    }
    let projection = SessionProjection::replay_project_verified(
        ProjectId::new(manifest["project_id"].as_str().unwrap()),
        &events,
        &[],
        || Ok(ResearchReferenceCatalog::default()),
        &store,
    )
    .unwrap();
    let input = projection
        .state()
        .request_audits()
        .last()
        .unwrap()
        .reconstruct_full_logical(|blob| store.read_blob(blob))
        .unwrap()
        .unwrap();
    let input = serde_json::to_string(input.input()).unwrap();
    assert!(!input.contains(refs[0].as_str().unwrap()));
    assert!(input.contains(refs[1].as_str().unwrap()));
    assert_eq!(
        manifest["original_gap_records"].as_array().unwrap().len(),
        1
    );
    assert_eq!(
        manifest["original_gap_records"][0]["selection_event_ref"],
        refs[1]
    );
    assert!(input.contains(manifest["original_gap_records"][0]["gap"].as_str().unwrap()));
}

#[test]
fn proof_v4_rejects_source_coherent_forged_or_missing_feedback_even_with_matching_plan() {
    use crate::domain::context::{ContextContentHash, ContextFragmentContent};
    use crate::domain::session::{
        SessionReducer, SessionSourceIdentity, TaskContextCapture, TaskContextSelected,
    };

    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/task_context_proof_v4_rejection_refresh");
    let manifest: Value =
        serde_json::from_slice(&fs::read(fixture.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["source_candidate"], "C45c");
    assert_eq!(
        manifest["binary_sha256"],
        "3258ceccf96aa07fb4c0c74eedaa27c4ed1eb26e80e57f834a09475add21cedd"
    );
    let root = TemporaryDirectory::new("task-proof-v4-feedback");
    let project = Project::initialize(root.path()).unwrap();
    let store = LocalArtifactStore::for_project(&project);
    let mut events_bytes = None;
    for (relative, digest) in manifest["files"].as_object().unwrap() {
        let digest = digest.as_str().unwrap();
        let bytes = fs::read(fixture.join("blobs").join(digest)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(&bytes)), digest);
        if relative.starts_with(".echidna/artifacts/blobs/sha256/") {
            assert_eq!(
                store.publish_bytes(&bytes).unwrap().blob_id.as_str(),
                digest
            );
        } else if relative.ends_with("/events.jsonl") {
            events_bytes = Some(bytes);
        } else if relative == "effect-count.txt" {
            assert_eq!(bytes, b"one\n");
        }
    }
    let events: Vec<EventEnvelope<Value>> = std::str::from_utf8(&events_bytes.unwrap())
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let selected = events
        .iter()
        .position(|event| {
            event.event_type.as_str() == "session.task_context_selected"
                && event.payload["refresh_of"].is_string()
        })
        .unwrap();
    let selection: TaskContextSelected =
        serde_json::from_value(events[selected].payload.clone()).unwrap();
    assert_eq!(selection.request_proof_version, 4);
    let end = events.partition_point(|event| event.sequence <= selection.source.watermark());
    assert!(end <= selected);
    let project_id = ProjectId::new(manifest["project_id"].as_str().unwrap());
    let (_, source) =
        SessionSourceIdentity::from_events(project_id.clone(), &events[..end], &[]).unwrap();
    let current = SessionReducer
        .reduce(events[..=selected].iter().cloned())
        .unwrap();
    let capture: TaskContextCapture =
        serde_json::from_slice(&store.read_blob(&selection.capture_blob).unwrap()).unwrap();
    assert_eq!(capture.compile.correction_feedback.len(), 1);
    let captured = crate::app::agent_loop::compile_captured_task_context(
        &source,
        &capture,
        &selection,
        events[..end].iter(),
        &[],
    )
    .unwrap();
    let following = crate::app::agent_loop::compile_following_task_context(
        &source,
        &current,
        &capture,
        &selection,
        &events[..=selected],
        &[],
    )
    .unwrap()
    .plan;
    assert_eq!(
        following.context_fingerprint(),
        selection.planned_context_hash
    );
    assert_eq!(
        following.context_fingerprint(),
        captured.context_fingerprint()
    );
    assert_eq!(following.model_input(), captured.model_input());
    assert_eq!(following.delivery_map(), captured.delivery_map());
    let rejection = events
        .iter()
        .find(|event| event.event_type.as_str() == "model.sample_rejected")
        .unwrap();
    let actual_step = rejection.payload["sampling_step_id"].as_str().unwrap();
    assert!(
        events
            .iter()
            .any(|event| event.event_type.as_str() == "model.argument_correction_reset")
    );

    for forged_id in [false, true] {
        let mut value = serde_json::to_value(&capture).unwrap();
        if forged_id {
            let fragment = &mut value["compile"]["correction_feedback"][0];
            fragment["id"] = serde_json::json!("correction:forged-step");
            let text = fragment["content"]["items"][0]["text"]
                .as_str()
                .unwrap()
                .replace(actual_step, "forged-step");
            fragment["content"]["items"][0]["text"] = serde_json::json!(text);
            let content: ContextFragmentContent =
                serde_json::from_value(fragment["content"].clone()).unwrap();
            let hash = ContextContentHash::for_fragment(&content, None).unwrap();
            fragment["provenance"]["freshness"]["source_revision_or_hash"] =
                serde_json::json!(hash.as_str());
            fragment["item_provenance"]["0"]["freshness"]["source_revision_or_hash"] =
                serde_json::json!(hash.as_str());
        } else {
            value["compile"]["correction_feedback"] = serde_json::json!([]);
        }
        let forged: TaskContextCapture = serde_json::from_value(value).unwrap();
        let forged_blob = store
            .publish_bytes(&serde_json::to_vec(&forged).unwrap())
            .unwrap()
            .blob_id;
        let mut prefix = events[..end].to_vec();
        assert_eq!(
            prefix.last().unwrap().event_type.as_str(),
            "session.task_context_captured"
        );
        prefix.last_mut().unwrap().payload["blob_id"] = serde_json::json!(forged_blob.as_str());
        let (forged_source, forged_state) =
            SessionSourceIdentity::from_events(project_id.clone(), &prefix, &[]).unwrap();
        let mut forged_selection = selection.clone();
        forged_selection.capture_blob = forged_blob;
        forged_selection.source = forged_source;
        let mut selected_event = events[selected].clone();
        selected_event.payload = serde_json::to_value(&forged_selection).unwrap();
        let mut forged_events = prefix.clone();
        forged_events.push(selected_event);
        let forged_current = SessionReducer
            .reduce(forged_events.iter().cloned())
            .unwrap();
        let forged_plan = crate::app::agent_loop::compile_following_task_context(
            &forged_state,
            &forged_current,
            &forged,
            &forged_selection,
            &forged_events,
            &[],
        )
        .unwrap()
        .plan;
        forged_selection.planned_context_hash = forged_plan.context_fingerprint().to_owned();
        assert_ne!(
            forged_selection.planned_context_hash,
            selection.planned_context_hash
        );
        assert!(
            crate::app::agent_loop::compile_captured_task_context(
                &forged_state,
                &forged,
                &forged_selection,
                prefix.iter(),
                &[]
            )
            .is_err(),
            "source verification accepted a complete forged plan, forged_id={forged_id}"
        );
    }
}

fn replay_fixture(name: &str, candidate: &str, binary_hash: &str, version: u8) {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    let manifest: Value =
        serde_json::from_slice(&fs::read(fixture.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["source_candidate"], candidate);
    assert_eq!(manifest["binary_sha256"], binary_hash);
    let root = TemporaryDirectory::new("task-proof-version-replay");
    let project = Project::initialize(root.path()).unwrap();
    let store = LocalArtifactStore::for_project(&project);
    let files = manifest["files"].as_object().unwrap();
    let mut old_events = None;
    for (relative, expected) in files {
        let expected = expected.as_str().unwrap();
        let bytes = fs::read(fixture.join("blobs").join(expected)).unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(&bytes)),
            expected,
            "{relative}"
        );
        if relative.starts_with(".echidna/artifacts/blobs/sha256/") {
            assert_eq!(
                store.publish_bytes(&bytes).unwrap().blob_id.as_str(),
                expected
            );
        } else if relative.ends_with("/events.jsonl") {
            old_events = Some(bytes);
        } else if relative == "attribution-count.txt" {
            assert_eq!(
                bytes,
                if name.ends_with("shared_defect") {
                    b"b\n".as_slice()
                } else {
                    b"a\nb\n".as_slice()
                }
            );
        }
    }
    let old_events = old_events.unwrap();
    if version == 3 {
        let prefix_len = manifest["old_event_bytes"].as_u64().unwrap() as usize;
        let prefix = &old_events[..prefix_len];
        assert_eq!(
            format!("{:x}", Sha256::digest(prefix)),
            manifest["old_session_sha256"].as_str().unwrap()
        );
        let v2 = fixture.parent().unwrap().join("task_context_proof_v2");
        let v2_manifest: Value =
            serde_json::from_slice(&fs::read(v2.join("manifest.json")).unwrap()).unwrap();
        let v2_hash = v2_manifest["files"]
            .as_object()
            .unwrap()
            .iter()
            .find(|(relative, _)| relative.ends_with("/events.jsonl"))
            .unwrap()
            .1
            .as_str()
            .unwrap();
        assert_eq!(prefix, fs::read(v2.join("blobs").join(v2_hash)).unwrap());
        let appended = std::str::from_utf8(&old_events[prefix_len..])
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<EventEnvelope<Value>>(line).unwrap())
            .collect::<Vec<_>>();
        assert!(
            appended
                .iter()
                .any(|event| event.event_type.as_str() == "session.resumed")
        );
        assert!(appended.iter().any(|event| event.event_type.as_str()
            == "session.task_context_selected"
            && event.payload["request_proof_version"] == 3));
        assert!(
            !appended
                .iter()
                .any(|event| event.event_type.as_str() == "tool.execution_outcome_recorded")
        );
    }
    let events: Vec<EventEnvelope<Value>> = std::str::from_utf8(&old_events)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let selections = events
        .iter()
        .filter(|event| event.event_type.as_str() == "session.task_context_selected")
        .collect::<Vec<_>>();
    assert!(selections.iter().all(|event| match version {
        0 => event.payload.get("request_proof_version").is_none(),
        1 => event.payload["request_proof_version"] == 1,
        2 => event.payload["request_proof_version"] == 2,
        3 => matches!(event.payload["request_proof_version"].as_u64(), Some(2 | 3)),
        _ => panic!("unsupported fixture version"),
    }));
    assert!(
        selections
            .iter()
            .any(|event| event.payload["task_revisions"].as_array().unwrap().len() >= 2)
    );
    let audits = events
        .iter()
        .filter(|event| event.event_type.as_str() == "session.request_audit_recorded")
        .collect::<Vec<_>>();
    assert!(
        audits
            .iter()
            .any(|event| event.payload["delivery"]["mode"] == "full")
    );
    assert!(
        audits
            .iter()
            .any(|event| event.payload["delivery"]["mode"] == "incremental")
    );
    let old_project = ProjectId::new(manifest["project_id"].as_str().unwrap());
    let resumed = SessionProjection::replay_for_resume(
        old_project.clone(),
        &events,
        &[],
        || Ok(ResearchReferenceCatalog::default()),
        &store,
    )
    .unwrap();
    let projection = SessionProjection::replay_project_verified(
        old_project,
        &events,
        &[],
        || Ok(ResearchReferenceCatalog::default()),
        &store,
    )
    .unwrap();
    assert_eq!(
        resumed.state(),
        projection.state(),
        "bounded Resume preserves complete typed state"
    );
    let mut saw_old_shared_progress = false;
    let mut saw_unresolved = false;
    for audit in projection.state().request_audits() {
        let request = audit.reconstruct(|blob| store.read_blob(blob)).unwrap();
        let input = serde_json::to_string(&request).unwrap();
        saw_old_shared_progress |= input.contains("A_ONLY_VALUE") && input.contains("B_ONLY_VALUE");
        saw_unresolved |= input.contains("unresolved task attribution")
            && !input.contains("A_ONLY_VALUE")
            && !input.contains("B_ONLY_VALUE");
    }
    if version < 2 {
        assert!(
            saw_old_shared_progress,
            "old shared attribution must be preserved"
        );
    } else if name.ends_with("shared_defect") {
        let final_request = projection
            .state()
            .request_audits()
            .last()
            .unwrap()
            .reconstruct(|blob| store.read_blob(blob))
            .unwrap();
        let input = serde_json::to_string(&final_request).unwrap();
        assert!(input.contains("ALPHA-METHOD-ONLY"));
        assert!(
            !input.contains("BETA_LATER_VALUE"),
            "v2 must preserve the original omission"
        );
        assert!(events.iter().any(|event| event.event_type.as_str()
            == "tool.execution_outcome_recorded"
            && event.payload["result"]["call_id"] == "shared-beta-result"));
    } else {
        assert!(
            saw_unresolved,
            "v2 and v3 unresolved attribution must be preserved"
        );
    }
    let event_hash = files
        .iter()
        .find(|(relative, _)| relative.ends_with("/events.jsonl"))
        .unwrap()
        .1
        .as_str()
        .unwrap();
    assert_eq!(
        fs::read(fixture.join("blobs").join(event_hash)).unwrap(),
        old_events
    );
}
