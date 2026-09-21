mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use cyber_agent::adapters::project_event_store::{Project, ProjectEventStream};
use cyber_agent::domain::artifacts::{ArtifactEvent, ArtifactId, ArtifactRecord, BlobId};
use cyber_agent::domain::context::{ProviderSessionId, ProviderSessionRef, ProviderSessionRole};
use cyber_agent::domain::events::{EventEnvelope, EventId, EventType, OccurredAt};
use cyber_agent::domain::research::{
    FindingDraft, FindingId, FindingRecord, FindingStatus, HypothesisDraft, HypothesisId,
    HypothesisRecord, HypothesisStatus, ResearchEvent, ResearchReducer, TargetKind,
    TargetSnapshotDraft, TargetSnapshotId, TargetSnapshotRecord,
};
use cyber_agent::domain::session::{
    CompactionValidationContext, EvidenceSpine, NarrativeSummary, ResearchStreamIdentity,
    SessionEvent, SessionReducer, SessionSummaryDraft,
};
use cyber_agent::domain::tools::ToolCallId;
use cyber_agent::ports::{EventSink, EventSource};
use serde::Deserialize;
use serde_json::{Value, json};

use support::TemporaryDirectory;

#[derive(Debug, Deserialize)]
struct QueryFixture {
    queries: Vec<QueryCase>,
}

#[derive(Debug, Deserialize)]
struct QueryCase {
    case_id: String,
    domain: String,
    current_target: String,
    query: String,
    required: Vec<RequiredResult>,
    acceptable_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RequiredResult {
    id: String,
    source: String,
    eligibility: String,
    tier: String,
    tier_reason: String,
    exact_identifier: bool,
    matched_fields: Vec<String>,
}

fn run(directory: &TemporaryDirectory, args: &[String]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_cyber-agent"))
        .current_dir(directory.path())
        .args(args)
        .output()
        .expect("retrieval inspection command should run");
    assert!(output.status.success(), "stderr: {:?}", output.stderr);
    serde_json::from_slice(&output.stdout).expect("retrieval report should be JSON")
}

fn command(query: &str, target_snapshot: &str) -> Vec<String> {
    vec![
        "inspect".to_owned(),
        "retrieval".to_owned(),
        "--query".to_owned(),
        query.to_owned(),
        "--target-snapshot".to_owned(),
        target_snapshot.to_owned(),
    ]
}

fn historical_command(
    query: &str,
    target_snapshot: &str,
    sessions: &[&str],
    historical_comparison: bool,
) -> Vec<String> {
    let mut args = command(query, target_snapshot);
    for session in sessions {
        args.extend(["--historical-session".to_owned(), (*session).to_owned()]);
    }
    if historical_comparison {
        args.push("--historical-comparison".to_owned());
    }
    args
}

fn append_research(project: &Project, event_id: &str, event: ResearchEvent) {
    let mut writer = project.open_writer().unwrap();
    let mut store = writer.open_research(None).unwrap();
    let envelope = EventEnvelope::current(
        EventId::new(event_id),
        ProjectEventStream::Research.stream_id(),
        store.next_sequence().unwrap(),
        OccurredAt::new("2026-08-31T00:00:00Z"),
        EventType::new(event.event_type().as_str()),
        event.encode_payload().unwrap(),
    );
    EventSink::append_event(&mut store, &envelope).unwrap();
}

fn target(project: &Project, id: &str, target_id: &str, version: &str, kind: TargetKind) {
    append_research(
        project,
        &format!("event-{id}"),
        ResearchEvent::TargetSnapshotRecorded(
            TargetSnapshotRecord::from_draft(TargetSnapshotDraft {
                id: TargetSnapshotId::new(id),
                target_id: target_id.to_owned(),
                kind,
                locator: format!("package:{target_id}"),
                version: version.to_owned(),
                build_id: format!("build-{version}"),
                target_artifact_ids: Vec::new(),
                environment_fingerprint: "fixture-environment".to_owned(),
                device_fingerprint: "fixture-device".to_owned(),
                captured_at: "2026-08-31T00:00:00Z".to_owned(),
                notes: "retrieval fixture target".to_owned(),
            })
            .unwrap(),
        ),
    );
}

fn finding(project: &Project, id: &str, snapshot: &str, target_id: &str, version: &str, cve: &str) {
    finding_with_impact(
        project,
        id,
        snapshot,
        target_id,
        version,
        cve,
        "cross-app intent launch",
    );
}

fn finding_with_impact(
    project: &Project,
    id: &str,
    snapshot: &str,
    target_id: &str,
    version: &str,
    identifier: &str,
    impact: &str,
) {
    append_research(
        project,
        &format!("event-{id}"),
        ResearchEvent::FindingRecorded {
            record: FindingRecord::from_draft(FindingDraft {
                id: FindingId::new(id),
                title: format!("{identifier} target finding"),
                status: FindingStatus::Candidate,
                target_id: target_id.to_owned(),
                target_version: version.to_owned(),
                vulnerability_class: "intent redirection".to_owned(),
                entry_point: format!("com.example.target.{id}.ExportActivity"),
                sensitive_sink: "startActivity".to_owned(),
                target_snapshot_ids: vec![TargetSnapshotId::new(snapshot)],
                prerequisites: vec!["authenticated test account".to_owned()],
                impact: impact.to_owned(),
                evidence_links: Vec::new(),
            })
            .unwrap(),
            keep_independent: false,
        },
    );
}

fn hypothesis(project: &Project, id: &str, target_id: &str, title: &str) {
    append_research(
        project,
        &format!("event-{id}"),
        ResearchEvent::HypothesisRecorded {
            record: HypothesisRecord::from_draft(HypothesisDraft {
                id: HypothesisId::new(id),
                title: title.to_owned(),
                target_id: target_id.to_owned(),
                attack_surface: "exported activity intent input".to_owned(),
                vulnerability_class: "intent redirection".to_owned(),
                entry_point: "com.example.target.ExplainActivity".to_owned(),
                sensitive_sink: "startActivity".to_owned(),
                rationale: "intent redirection explanation for the current target".to_owned(),
                status: HypothesisStatus::Active,
                priority: 1,
                evidence_links: Vec::new(),
                last_action: None,
                next_action: None,
            })
            .unwrap(),
            keep_independent: false,
        },
    );
}

fn start_session(project: &Project, session: &str) {
    let session_id = cyber_agent::domain::events::StreamId::new(session);
    let stream_id = ProjectEventStream::Session(session_id.clone()).stream_id();
    let mut writer = project.open_writer().unwrap();
    let mut store = writer.open_session(session_id, None).unwrap();
    let event = SessionEvent::Started {
        primary_provider_session: ProviderSessionRef {
            id: ProviderSessionId::new(format!("primary-{session}")),
            role: ProviderSessionRole::Primary,
            generation: 1,
        },
    }
    .into_envelope(
        EventId::new(format!("event-started-{session}")),
        stream_id,
        store.next_sequence().unwrap(),
        OccurredAt::new("2026-08-31T00:00:00Z"),
    )
    .unwrap();
    EventSink::append_event(&mut store, &event).unwrap();
}

fn artifact(project: &Project, id: &str, snapshot: &str, source_path: &str) {
    let event_id = EventId::new(format!("event-{id}"));
    append_research(
        project,
        event_id.as_str(),
        ResearchEvent::Artifact(ArtifactEvent::Created(ArtifactRecord {
            id: ArtifactId::new(id),
            blob_id: BlobId::new("a".repeat(64)).unwrap(),
            media_type: "application/octet-stream".to_owned(),
            size_bytes: 4096,
            source_path: source_path.to_owned(),
            created_by_event_id: event_id.clone(),
            produced_by_tool_call_id: ToolCallId::new(format!("call-{id}")),
            target_snapshot_id: TargetSnapshotId::new(snapshot),
            toolchain_fingerprint: "fixture-toolchain".to_owned(),
            parent_artifact_ids: Vec::new(),
            transform: None,
        })),
    );
}

fn append_historical_summary(project: &Project, session: &str, target: &TargetSnapshotId) {
    let research_events = {
        let mut writer = project.open_writer().unwrap();
        let store = writer.open_research(None).unwrap();
        EventSource::read_events(&store, &ProjectEventStream::Research.stream_id()).unwrap()
    };
    let research_stream =
        ResearchStreamIdentity::from_events(project.project_id().clone(), &research_events)
            .unwrap();
    let research_state = ResearchReducer.reduce(research_events).unwrap();
    let receipt_ids = BTreeSet::new();
    let validation = CompactionValidationContext::new(
        project.project_id(),
        &research_stream,
        &research_state,
        &receipt_ids,
    );
    let draft = SessionSummaryDraft::new(
        EvidenceSpine::new(
            research_stream.clone(),
            vec![target.clone()],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap(),
        NarrativeSummary::new(
            "historical-sentinel",
            "explicitly selected derived summary",
            Vec::new(),
            Vec::new(),
        )
        .unwrap(),
    )
    .unwrap();
    let session_id = cyber_agent::domain::events::StreamId::new(session);
    let stream_id = ProjectEventStream::Session(session_id.clone()).stream_id();
    {
        let mut writer = project.open_writer().unwrap();
        let mut store = writer.open_session(session_id.clone(), None).unwrap();
        let started = SessionEvent::Started {
            primary_provider_session: ProviderSessionRef {
                id: ProviderSessionId::new(format!("primary-{session}")),
                role: ProviderSessionRole::Primary,
                generation: 1,
            },
        }
        .into_envelope(
            EventId::new(format!("event-started-{session}")),
            stream_id.clone(),
            store.next_sequence().unwrap(),
            OccurredAt::new("2026-08-31T00:00:00Z"),
        )
        .unwrap();
        EventSink::append_event(&mut store, &started).unwrap();
    }
    let state = {
        let mut writer = project.open_writer().unwrap();
        let store = writer.open_session(session_id, None).unwrap();
        SessionReducer
            .reduce(EventSource::read_events(&store, &stream_id).unwrap())
            .unwrap()
    };
    let compaction = state.next_compaction(draft, 0, &validation).unwrap();
    let mut writer = project.open_writer().unwrap();
    let mut store = writer
        .open_session(cyber_agent::domain::events::StreamId::new(session), None)
        .unwrap();
    let envelope = SessionEvent::CompactionCreated { compaction }
        .into_envelope(
            EventId::new(format!("event-compaction-{session}")),
            stream_id,
            store.next_sequence().unwrap(),
            OccurredAt::new("2026-08-31T00:00:01Z"),
        )
        .unwrap();
    EventSink::append_event(&mut store, &envelope).unwrap();
}

fn fixture_project(label: &str) -> (TemporaryDirectory, Project) {
    let directory = TemporaryDirectory::new(label);
    let project = Project::initialize(directory.path()).unwrap();
    target(
        &project,
        "target-current",
        "com.example.target",
        "2.0.0",
        TargetKind::Mobile,
    );
    target(
        &project,
        "target-old",
        "com.example.target",
        "1.0.0",
        TargetKind::Mobile,
    );
    target(
        &project,
        "target-other",
        "org.example.other",
        "2.0.0",
        TargetKind::Mobile,
    );
    target(
        &project,
        "target-source",
        "src.example.target",
        "2.0.0",
        TargetKind::Source,
    );
    target(
        &project,
        "target-web",
        "web.example.target",
        "2.0.0",
        TargetKind::Web,
    );
    target(
        &project,
        "target-binary",
        "bin.example.target",
        "2.0.0",
        TargetKind::Binary,
    );
    finding(
        &project,
        "finding-current",
        "target-current",
        "com.example.target",
        "2.0.0",
        "CVE-2026-7777",
    );
    finding(
        &project,
        "finding-old",
        "target-old",
        "com.example.target",
        "1.0.0",
        "CVE-2025-0001",
    );
    artifact(
        &project,
        "artifact-current",
        "target-current",
        "native/JNI_OnLoad.bin",
    );
    artifact(&project, "artifact-old", "target-old", "native/legacy.bin");
    artifact(
        &project,
        "artifact-binary",
        "target-binary",
        "native/JNI_OnLoad.bin",
    );
    let memory = project.root().join(".cyber-agent/memory/notes");
    fs::create_dir_all(&memory).unwrap();
    fs::write(
        memory.join("security.md"),
        "# Security identifiers\n\
CVE-2026-1234 CWE-79 https://api.example.test/v1/redirect?next=\n\
com.example.target.ExportActivity onNewIntent android.permission.SEND_SMS\n\
android.intent.action.VIEW JNI_OnLoad\n\
aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n\
导出的组件可被 WebView 调用；WebView bridge requires a control.\n",
    )
    .unwrap();
    (directory, project)
}

fn fixture_domain(target_snapshot: &str) -> &str {
    match target_snapshot {
        "target-source" => "source",
        "target-web" => "web",
        "target-current" => "mobile",
        "target-binary" => "binary",
        _ => panic!("unexpected fixture target `{target_snapshot}`"),
    }
}

fn tree_bytes(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn collect(root: &Path, current: &Path, entries: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(current).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                collect(root, &path, entries);
            } else {
                entries.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut entries = BTreeMap::new();
    collect(root, root, &mut entries);
    entries
}

#[test]
fn deterministic_shadow_meets_controlled_identifier_cjk_and_typo_contract_without_writes() {
    let (directory, project) = fixture_project("m4-s3f-controlled");
    let fixture: QueryFixture =
        serde_json::from_str(include_str!("fixtures/m4_retrieval/queries.json")).unwrap();
    let before = tree_bytes(project.root().join(".cyber-agent").as_path());
    let mut recalled = 0_usize;
    let mut returned = 0_usize;
    let mut relevant_returned = 0_usize;

    for case in fixture.queries {
        assert_eq!(case.domain, fixture_domain(&case.current_target));
        let args = command(&case.query, &case.current_target);
        let report = run(&directory, &args);
        assert_eq!(
            report["report"]["current_target"]["snapshot_id"],
            case.current_target
        );
        assert_eq!(report["status"], "ok");
        assert_eq!(report["shadow"], true);
        assert_eq!(report["model_context_changed"], false);
        assert_eq!(
            report["effects"],
            json!({
                "provider_requests": 0,
                "tool_actions": 0,
                "research_writes": 0,
                "memory_writes": 0,
                "event_writes": 0,
                "cache_writes": 0
            })
        );
        assert_eq!(
            report["report"]["algorithm_version"],
            "m4-s3f-deterministic-security-retrieval-v1"
        );
        assert_eq!(
            report["report"]["deferred_metrics"]["repeated_read_reduction"],
            "unavailable"
        );
        assert_eq!(
            report["report"]["deferred_metrics"]["estimated_context_cost_improvement"],
            "unavailable"
        );
        let results = report["report"]["results"].as_array().unwrap();
        assert!(results.len() <= 10);
        for required in &case.required {
            let result = results
                .iter()
                .find(|result| result["id"].as_str() == Some(&required.id))
                .unwrap_or_else(|| panic!("{} must recall {}", case.case_id, required.id));
            assert_eq!(result["source"], required.source, "{} source", case.case_id);
            assert_eq!(
                result["eligibility"], required.eligibility,
                "{} eligibility",
                case.case_id
            );
            assert_eq!(result["tier"], required.tier, "{} tier", case.case_id);
            assert_eq!(
                result["tier_reason"], required.tier_reason,
                "{} tier reason",
                case.case_id
            );
            assert_eq!(
                result["component_ranks"]["exact_identifier"].is_u64(),
                required.exact_identifier,
                "{} exact identifier",
                case.case_id
            );
            let matched_fields = result["matched_fields"].as_array().unwrap();
            for field in &required.matched_fields {
                assert!(
                    matched_fields.iter().any(|matched| matched == field),
                    "{} must explain matched field {field}",
                    case.case_id
                );
            }
        }
        assert_eq!(report["report"], run(&directory, &args)["report"]);
        recalled += case.required.len();
        returned += results.len();
        relevant_returned += results
            .iter()
            .filter(|result| {
                result["id"].as_str().is_some_and(|id| {
                    case.acceptable_ids
                        .iter()
                        .any(|acceptable| acceptable == id)
                })
            })
            .count();
    }

    assert_eq!(recalled, 13, "controlled Recall@10 must be 1.00");
    assert!(
        relevant_returned as f64 / returned as f64 >= 0.60,
        "controlled precision@10 must remain at least 0.60"
    );
    assert_eq!(
        tree_bytes(project.root().join(".cyber-agent").as_path()),
        before,
        "Shadow must not mutate facts, cache, session, Research, or Memory"
    );
}

#[test]
fn current_target_filters_old_and_cross_project_facts_and_artifact_metadata_without_blob_reads() {
    let (directory, project) = fixture_project("m4-s3f-target-filter");
    let current = run(&directory, &command("CVE-2026-7777", "target-current"));
    let current_ids = current["report"]["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|result| result["id"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(current_ids.contains(&"research:finding:finding-current"));
    assert!(!current_ids.contains(&"research:finding:finding-old"));
    assert!(
        current["report"]["exclusions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["id"] == "research:finding:finding-old"
                && entry["eligibility"] == "current_target_mismatch_excluded")
    );

    let artifact = run(&directory, &command("JNI_OnLoad", "target-current"));
    assert!(
        artifact["report"]["results"]
            .as_array()
            .unwrap()
            .iter()
            .any(|result| result["id"] == "artifact:artifact-current")
    );
    assert!(
        !artifact["report"]["results"]
            .as_array()
            .unwrap()
            .iter()
            .any(|result| result["id"] == "artifact:artifact-old")
    );

    let other = TemporaryDirectory::new("m4-s3f-cross-project");
    let other_project = Project::initialize(other.path()).unwrap();
    target(
        &other_project,
        "target-other-project",
        "org.other.project",
        "9.9.9",
        TargetKind::Mobile,
    );
    finding(
        &other_project,
        "finding-other-project",
        "target-other-project",
        "org.other.project",
        "9.9.9",
        "CVE-2026-7777",
    );
    let isolated = run(&directory, &command("CVE-2026-7777", "target-current"));
    assert!(
        !isolated["report"]["results"]
            .as_array()
            .unwrap()
            .iter()
            .any(|result| result["id"] == "research:finding:finding-other-project")
    );
    assert!(!project.root().join(".cyber-agent/artifacts/blobs").exists());
}

#[test]
fn mixed_exact_queries_keep_explanations_and_report_true_rrf_rank_before_tier_rerank() {
    let (directory, project) = fixture_project("m4-s3f-rank-regressions");
    hypothesis(
        &project,
        "hypothesis-explanation",
        "com.example.target",
        "intent redirection reachability explanation",
    );

    let mixed = run(
        &directory,
        &command("CVE-2026-7777 intent redirection", "target-current"),
    );
    let mixed_ids = mixed["report"]["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|result| result["id"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(mixed_ids.contains(&"research:finding:finding-current"));
    assert!(mixed_ids.contains(&"research:hypothesis:hypothesis-explanation"));

    fs::write(
        project
            .root()
            .join(".cyber-agent/memory/notes/mixed-tier.md"),
        "# Mixed tier\nCVE-2026-5151\n",
    )
    .unwrap();
    let cross_tier = run(
        &directory,
        &command("CVE-2026-5151 intent redirection", "target-current"),
    );
    let cross_tier_ids = cross_tier["report"]["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|result| result["id"].as_str().unwrap())
        .collect::<Vec<_>>();
    let explanation_position = cross_tier_ids
        .iter()
        .position(|id| *id == "research:hypothesis:hypothesis-explanation")
        .unwrap();
    let exact_memory_position = cross_tier_ids
        .iter()
        .position(|id| *id == "memory:notes/mixed-tier.md:0")
        .unwrap();
    assert!(
        explanation_position < exact_memory_position,
        "authority/freshness tier must rerank the RRF result without an extra exact-match key"
    );

    finding(
        &project,
        "finding-tier",
        "target-current",
        "com.example.target",
        "2.0.0",
        "CVE-2026-4242",
    );
    fs::write(
        project
            .root()
            .join(".cyber-agent/memory/notes/tier-rank.md"),
        "# Tier rank\nCVE-2026-4242\n",
    )
    .unwrap();
    let tiered = run(&directory, &command("CVE-2026-4242", "target-current"));
    let results = tiered["report"]["results"].as_array().unwrap();
    let finding_result = results
        .iter()
        .find(|result| result["id"] == "research:finding:finding-tier")
        .unwrap();
    let memory_result = results
        .iter()
        .find(|result| result["id"] == "memory:notes/tier-rank.md:0")
        .unwrap();
    assert_eq!(results[0]["id"], "research:finding:finding-tier");
    assert!(
        memory_result["component_ranks"]["reciprocal_rank_fusion"]
            .as_u64()
            .unwrap()
            < finding_result["component_ranks"]["reciprocal_rank_fusion"]
                .as_u64()
                .unwrap(),
        "reported RRF rank must be captured before authority/freshness reranking"
    );
}

#[test]
fn production_results_are_top_ten_and_core_memory_is_not_a_retrieval_candidate() {
    let (directory, project) = fixture_project("m4-s3f-top-ten");
    let sections = (0..12)
        .map(|index| format!("# Candidate {index}\nCVE-2026-9999\n"))
        .collect::<String>();
    fs::write(
        project.root().join(".cyber-agent/memory/notes/top-ten.md"),
        sections,
    )
    .unwrap();
    fs::write(
        project.root().join(".cyber-agent/memory/core.md"),
        "CVE-2099-0001 must remain in always-injected core Memory only\n",
    )
    .unwrap();

    let report = run(&directory, &command("CVE-2026-9999", "target-current"));
    assert_eq!(report["report"]["results"].as_array().unwrap().len(), 10);

    let core = run(&directory, &command("CVE-2099-0001", "target-current"));
    assert!(
        core["report"]["results"]
            .as_array()
            .unwrap()
            .iter()
            .all(|result| !result["id"].as_str().unwrap().contains("memory:core.md"))
    );
}

#[test]
fn oversized_facts_and_many_observation_sessions_remain_bounded_and_read_only() {
    let (directory, project) = fixture_project("m4-s3f-bounded-sources");
    finding_with_impact(
        &project,
        "finding-oversized",
        "target-current",
        "com.example.target",
        "2.0.0",
        "CVE-2026-8888",
        &format!("bounded-impact {}", "x".repeat(20 * 1024)),
    );
    let oversized = run(&directory, &command("CVE-2026-8888", "target-current"));
    assert!(
        oversized["report"]["results"]
            .as_array()
            .unwrap()
            .iter()
            .any(|result| result["id"] == "research:finding:finding-oversized")
    );

    for index in 0..64 {
        start_session(&project, &format!("observation-cap-{index:02}"));
    }
    start_session(&project, "zzzz-observation-cap-excluded");
    let bounded = run(&directory, &command("CVE-2026-1234", "target-current"));
    assert_eq!(bounded["status"], "ok");
    assert_eq!(bounded["effects"]["event_writes"], 0);
}

fn memory_quota_project(label: &str) -> (TemporaryDirectory, Project) {
    let directory = TemporaryDirectory::new(label);
    let project = Project::initialize(directory.path()).unwrap();
    target(
        &project,
        "quota-target",
        "ordinary-project",
        "1",
        TargetKind::Source,
    );
    let notes = project.root().join(".cyber-agent/memory/notes");
    fs::create_dir_all(&notes).unwrap();
    // Reverse creation order proves selection uses sorted names, not directory order.
    for index in (0..65).rev() {
        fs::write(
            notes.join(format!("note-{index:04}.md")),
            format!("# Note\nsymbol_{index:04}\n"),
        )
        .unwrap();
    }
    (directory, project)
}

#[test]
fn memory_document_quota_counts_present_files_including_empty_but_not_missing() {
    for fixed in [
        [None, None],
        [Some("# Decision\nfixed_decision\n"), None],
        [None, Some("# Failed\nfixed_failed\n")],
        [
            Some("# Decision\nfixed_decision\n"),
            Some("# Failed\nfixed_failed\n"),
        ],
        [Some(""), Some("")],
    ] {
        let (directory, project) = memory_quota_project("r5c2-present-documents");
        let memory = project.root().join(".cyber-agent/memory");
        for (name, content) in ["decisions.md", "failed-paths.md"].into_iter().zip(fixed) {
            if let Some(content) = content {
                fs::write(memory.join(name), content).unwrap();
            }
        }
        let present = fixed.iter().flatten().count();
        let empty = fixed
            .iter()
            .flatten()
            .filter(|text| text.is_empty())
            .count();
        let last_note = 63 - present;
        // The first excluded body must not even be read/validated after quota is full.
        fs::write(
            memory.join(format!("notes/note-{:04}.md", last_note + 1)),
            vec![0xff; 65 * 1024],
        )
        .unwrap();
        let before = tree_bytes(&project.root().join(".cyber-agent"));
        let args = command(&format!("symbol_{last_note:04}"), "quota-target");
        let report = run(&directory, &args);
        assert_eq!(report["report"]["queried_candidate_count"], 65 - empty);
        assert_eq!(
            report["report"]["results"][0]["id"],
            format!("memory:notes/note-{last_note:04}.md:0")
        );
        assert_eq!(
            report["report"]["results"][0]["component_ranks"]["exact_identifier"],
            1
        );
        assert_eq!(report["report"], run(&directory, &args)["report"]);
        assert_eq!(report["shadow"], true);
        assert_eq!(report["model_context_changed"], false);
        assert!(
            report["effects"]
                .as_object()
                .unwrap()
                .values()
                .all(|value| value == 0)
        );
        assert_eq!(tree_bytes(&project.root().join(".cyber-agent")), before);
    }
}

#[test]
fn memory_document_quota_preserves_selected_content_and_fixed_path_errors() {
    for (relative, content, expected) in [
        (
            "notes/note-0063.md",
            vec![b'x'; 65 * 1024],
            "above the 65536-byte limit",
        ),
        ("notes/note-0063.md", vec![0xff], "not UTF-8 Markdown"),
        ("decisions.md", vec![0xff], "not UTF-8 Markdown"),
        (
            "failed-paths.md",
            vec![b'x'; 65 * 1024],
            "above the 65536-byte limit",
        ),
    ] {
        let (directory, project) = memory_quota_project("r5c2-invalid-selected-document");
        fs::write(
            project.root().join(".cyber-agent/memory").join(relative),
            content,
        )
        .unwrap();
        let before = tree_bytes(&project.root().join(".cyber-agent"));
        let output = Command::new(env!("CARGO_BIN_EXE_cyber-agent"))
            .current_dir(directory.path())
            .args(command("symbol_0063", "quota-target"))
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "invalid selected file {relative} must fail"
        );
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(expected),
            "{:?}",
            output.stderr
        );
        assert_eq!(tree_bytes(&project.root().join(".cyber-agent")), before);
    }
}

#[test]
fn memory_document_quota_preserves_path_rejections() {
    for (name, kind, expected) in [
        (
            "notes/zzzz.txt",
            "file",
            "outside the supported project Memory vocabulary",
        ),
        ("notes/zzzz.md", "directory", "must be a regular file"),
        ("decisions.md", "directory", "must be a regular file"),
        #[cfg(unix)]
        ("notes/zzzz.md", "symlink", "must not be a symlink"),
        #[cfg(unix)]
        ("failed-paths.md", "symlink", "must not be a symlink"),
    ] {
        let (directory, project) = memory_quota_project("r5c2-invalid-note-entry");
        let memory = project.root().join(".cyber-agent/memory");
        let path = memory.join(name);
        match kind {
            "directory" => fs::create_dir(path).unwrap(),
            "file" => fs::write(path, "invalid note name").unwrap(),
            #[cfg(unix)]
            "symlink" => {
                std::os::unix::fs::symlink(memory.join("notes/note-0000.md"), path).unwrap()
            }
            _ => unreachable!("fixed fixture entry kind"),
        }
        let before = tree_bytes(&project.root().join(".cyber-agent"));
        let output = Command::new(env!("CARGO_BIN_EXE_cyber-agent"))
            .current_dir(directory.path())
            .args(command("symbol_0000", "quota-target"))
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(expected),
            "{:?}",
            output.stderr
        );
        assert_eq!(tree_bytes(&project.root().join(".cyber-agent")), before);
    }
}

#[test]
fn lexical_ranking_preserves_per_field_deduplication_weights_and_stable_ties() {
    let directory = TemporaryDirectory::new("r6-query-field-terms");
    let project = Project::initialize(directory.path()).unwrap();
    target(&project, "terms-target", "fixture", "1", TargetKind::Source);
    let notes = project.root().join(".cyber-agent/memory/notes");
    fs::create_dir_all(&notes).unwrap();
    for (name, body) in [
        ("a.md", "# amber\nquartz quartz quartz\n"),
        ("b.md", "# amber\nquartz\n"),
        ("c.md", "# quartz\nquartz amber\n"),
    ] {
        fs::write(notes.join(name), body).unwrap();
    }
    let before = tree_bytes(&project.root().join(".cyber-agent"));
    let args = command("quartz", "terms-target");
    let report = run(&directory, &args);
    let results = report["report"]["results"].as_array().unwrap();
    for (name, bm25, fields) in [
        ("c.md", 1, json!(["title", "body"])),
        ("a.md", 2, json!(["body"])),
        ("b.md", 3, json!(["body"])),
    ] {
        let id = format!("memory:notes/{name}:0");
        let result = results.iter().find(|result| result["id"] == id).unwrap();
        assert_eq!(result["component_ranks"]["fielded_bm25"], bm25);
        assert_eq!(result["matched_fields"], fields);
        assert_eq!(result["component_ranks"]["exact_identifier"], Value::Null);
    }
    assert_eq!(report, run(&directory, &args));
    assert_eq!(tree_bytes(&project.root().join(".cyber-agent")), before);
}

#[test]
fn lexical_field_term_limit_and_later_query_refresh_remain_independent() {
    let directory = TemporaryDirectory::new("r6-query-term-boundary");
    let project = Project::initialize(directory.path()).unwrap();
    target(&project, "terms-target", "fixture", "1", TargetKind::Source);
    let notes = project.root().join(".cyber-agent/memory/notes");
    fs::create_dir_all(&notes).unwrap();
    let terms = (0..256).map(|i| format!("word{i:04} ")).collect::<String>();
    let path = notes.join("boundary.md");
    // Each field has its own limit: the title remains eligible even when the
    // same term is past the body limit and beyond the n-gram character window.
    fs::write(&path, format!("# quartz\n{terms}quartz\n")).unwrap();
    let args = command("quartz", "terms-target");
    let first = run(&directory, &args);
    let hit = &first["report"]["results"][0];
    assert_eq!(hit["id"], "memory:notes/boundary.md:0");
    assert_eq!(hit["matched_fields"], json!(["title"]));
    assert_eq!(hit["component_ranks"]["fielded_bm25"], 1);
    let last = run(&directory, &command("word0255", "terms-target"));
    assert_eq!(
        last["report"]["results"][0]["component_ranks"]["fielded_bm25"],
        1
    );

    // A later invocation observes current bytes, not persisted query data.
    fs::write(&path, "# amber\nquartz\n").unwrap();
    let before = tree_bytes(&project.root().join(".cyber-agent"));
    let second = run(&directory, &args);
    assert_eq!(
        second["report"]["results"][0]["matched_fields"],
        json!(["body"])
    );
    assert_eq!(
        first["report"]["query_fingerprint"],
        second["report"]["query_fingerprint"]
    );
    assert!(
        run(&directory, &command("word0255", "terms-target"))["report"]["results"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(second, run(&directory, &args));
    assert_eq!(tree_bytes(&project.root().join(".cyber-agent")), before);
}

#[test]
fn explicitly_selected_historical_summaries_are_order_invariant_and_default_to_current_target() {
    let (directory, project) = fixture_project("m4-s3f-historical");
    append_historical_summary(
        &project,
        "history-current",
        &TargetSnapshotId::new("target-current"),
    );
    append_historical_summary(
        &project,
        "history-old",
        &TargetSnapshotId::new("target-old"),
    );

    let forward = run(
        &directory,
        &historical_command(
            "historical-sentinel",
            "target-current",
            &["history-current", "history-old"],
            false,
        ),
    );
    let reversed = run(
        &directory,
        &historical_command(
            "historical-sentinel",
            "target-current",
            &["history-old", "history-current"],
            false,
        ),
    );
    assert_eq!(forward["report"], reversed["report"]);

    let current = forward["report"]["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|result| result["id"] == "historical_summary:history-current")
        .expect("selected Summary bound to the current target must be eligible");
    assert_eq!(current["source"], "historical_summary");
    assert_eq!(current["eligibility"], "historical_summary_current_target");
    assert_eq!(current["tier"], "explicit_historical_summary");
    assert_eq!(
        current["tier_reason"],
        "explicitly_selected_historical_summary"
    );
    assert!(
        current["matched_fields"]
            .as_array()
            .unwrap()
            .contains(&json!("body"))
    );
    assert!(
        current["matched_identifiers"]
            .as_array()
            .unwrap()
            .contains(&json!("historical-sentinel"))
    );
    assert!(current["estimated_context_cost"].as_u64().unwrap() > 0);
    assert!(
        forward["report"]["exclusions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["id"] == "historical_summary:history-old"
                && entry["eligibility"] == "current_target_mismatch_excluded")
    );

    let comparison = run(
        &directory,
        &historical_command(
            "historical-sentinel",
            "target-current",
            &["history-current", "history-old"],
            true,
        ),
    );
    assert!(
        comparison["report"]["results"]
            .as_array()
            .unwrap()
            .iter()
            .any(|result| result["id"] == "historical_summary:history-old"
                && result["eligibility"] == "historical_comparison_selected")
    );
}
