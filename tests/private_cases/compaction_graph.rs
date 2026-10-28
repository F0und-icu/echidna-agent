//! Finite source graph remains bound to exact durable inputs.
use std::collections::BTreeMap;

use crate::domain::artifacts::BlobId;
use crate::domain::context::{ProviderSession, ProviderSessionId, ProviderSessionRole};
use crate::domain::events::{
    EventEnvelope, EventId, EventSequence, EventType, OccurredAt, StreamId,
};
use crate::domain::model::ContentItem;
use crate::domain::project::ProjectId;
use crate::domain::research::ResearchReferenceCatalog;
use crate::domain::session::{
    CompactionModelSample, CompactionSourceNode, CompactionSourceNodeInput, CompactionSourcePlan,
    CompactionSourceRange, NarrativeSummary, SessionEvent, SessionSourceIdentity,
    ValidatedResearchSource,
};

fn envelope(
    event: SessionEvent,
    sequence: u64,
) -> crate::domain::events::EventEnvelope<serde_json::Value> {
    event
        .into_envelope(
            EventId::new(format!("input-event-{sequence}")),
            StreamId::new("session:context2-input"),
            EventSequence::new(sequence),
            OccurredAt::new("2026-10-27T00:00:00Z"),
        )
        .unwrap()
}

#[test]
fn source_tree_requires_exact_utf8_coverage_and_a_single_reachable_root() {
    let events = vec![
        envelope(
            SessionEvent::Started {
                primary_provider_session: ProviderSession::new(
                    ProviderSessionId::new("source-test"),
                    ProviderSessionRole::Primary,
                    0,
                )
                .reference()
                .clone(),
            },
            1,
        ),
        envelope(
            SessionEvent::HistoryItemRecorded {
                item: ContentItem::user_text("protected"),
            },
            2,
        ),
        envelope(
            SessionEvent::HistoryItemRecorded {
                item: ContentItem::assistant_text("中a"),
            },
            3,
        ),
    ];
    let (source, _) =
        SessionSourceIdentity::from_events(ProjectId::new("context2"), &events).unwrap();
    let research = ValidatedResearchSource::from_events(
        ProjectId::new("context2"),
        &[],
        &ResearchReferenceCatalog::default(),
    )
    .unwrap();
    let (entries, contents) =
        CompactionSourcePlan::source_entries(&source, &research, &events, &[]).unwrap();
    let chinese = contents[1].find('中').unwrap();
    let split = chinese + '中'.len_utf8();
    let nodes = vec![
        CompactionSourceNode {
            id: "left".into(),
            input: CompactionSourceNodeInput::Source(vec![CompactionSourceRange {
                entry: 1,
                start: 0,
                end: split,
            }]),
            is_root: false,
            output_limit: 100,
            template_hash: None,
        },
        CompactionSourceNode {
            id: "right".into(),
            input: CompactionSourceNodeInput::Source(vec![CompactionSourceRange {
                entry: 1,
                start: split,
                end: contents[1].len(),
            }]),
            is_root: false,
            output_limit: 100,
            template_hash: None,
        },
        CompactionSourceNode {
            id: "root".into(),
            input: CompactionSourceNodeInput::Merge(vec!["left".into(), "right".into()]),
            is_root: true,
            output_limit: 100,
            template_hash: None,
        },
    ];
    let plan = CompactionSourcePlan::new(
        source.clone(),
        &research,
        entries.clone(),
        nodes.clone(),
        &contents,
    )
    .unwrap();
    plan.validate(&contents).unwrap();
    let templates = ["left", "right", "root"]
        .into_iter()
        .map(|id| (id.to_owned(), "a".repeat(64)))
        .collect::<BTreeMap<_, _>>();
    let templated = plan
        .clone()
        .with_node_templates(templates, &contents)
        .unwrap();
    let narrative = NarrativeSummary::new("goal", "done", Vec::new(), Vec::new()).unwrap();
    let output = r#"{"concise_progress":"done","unresolved_questions":[],"next_step_notes":[]}"#;
    let sample = CompactionModelSample::new_v2(
        source.clone(),
        EventId::new("audit"),
        narrative,
        100,
        BlobId::new("b".repeat(64)).unwrap(),
        &templated,
        "left",
        output.to_owned(),
    )
    .unwrap();
    let mut legacy = serde_json::to_value(&sample).unwrap();
    legacy["schema_version"] = 1.into();
    legacy.as_object_mut().unwrap().remove("graph");
    assert!(
        serde_json::from_value::<CompactionModelSample>(legacy)
            .unwrap()
            .validate()
            .is_err()
    );
    let mut altered = serde_json::to_value(&sample).unwrap();
    altered["graph"]["output"] = "{}".into();
    assert!(
        serde_json::from_value::<CompactionModelSample>(altered)
            .unwrap()
            .validate()
            .is_err()
    );
    let make_audit = |event_id: EventId, text: &str| {
        use crate::domain::context::{
            BaselineInvalidationReason, ContextBudget, ContextFragment, ContextFragmentContent,
            ContextFragmentId, ContextFragmentPriority, ContextFragmentSource, ContextPlanner,
            FragmentCachePolicy, ModelIdentity, ModelProfile,
        };
        use crate::domain::model::{
            CapabilityMappingVersion, ModelCapabilities, ModelRequestRequirements,
        };
        use crate::domain::request_audit::{
            PublishedAuditBlob, RequestAdapterVersion, RequestAuditAttempt, RequestAuditBinding,
            RequestAuditDelivery, RequestAuditRunContext,
        };
        use crate::domain::tools::{DeferredActivationSet, ResolvedToolPlan, ToolExposurePlan};
        let capabilities = ModelCapabilities {
            text_input: true,
            image_input: false,
            streaming: true,
            tool_call: true,
            structured_output: false,
            incremental_input: false,
            usage_reporting: false,
            reasoning_effort: None,
            max_output_tokens: None,
            context_window: 32768,
            capability_mapping_version: CapabilityMappingVersion::new(1),
        };
        let fragment = ContextFragment::new(
            ContextFragmentId::new("input").unwrap(),
            ContextFragmentSource::UserInput,
            ContextFragmentPriority::Required,
            FragmentCachePolicy::DoNotCache,
            ContextFragmentContent::ContentItems {
                items: vec![ContentItem::user_text(text)],
            },
        )
        .unwrap();
        let compiled = ContextPlanner::current()
            .compile(
                vec![fragment],
                ContextBudget::new(32768, 0, 0).unwrap(),
                &capabilities,
                Vec::new(),
                ModelRequestRequirements::default(),
                None,
            )
            .unwrap();
        let no_tools = ResolvedToolPlan::resolve(
            Vec::new(),
            &ToolExposurePlan::default(),
            &DeferredActivationSet::default(),
        )
        .unwrap();
        let context = RequestAuditRunContext::new(
            RequestAuditBinding::new(
                ProjectId::new("context2"),
                StreamId::new("session:context2-input"),
            )
            .unwrap(),
            RequestAdapterVersion::new("proof:1").unwrap(),
        );
        let rendered = RequestAuditAttempt::new(
            &context,
            ModelIdentity::new("provider", "model"),
            ModelProfile::new("default", 0),
            capabilities,
            compiled.model_input(),
            compiled.model_input(),
            RequestAuditDelivery::full(BaselineInvalidationReason::NoBaseline),
        )
        .unwrap()
        .bind_provider_session(templated.node_provider_session("left").unwrap())
        .bind_scoped_no_tool_context(&compiled, &no_tools)
        .unwrap()
        .render()
        .unwrap();
        let published: Vec<_> = rendered
            .component_bytes()
            .enumerate()
            .map(|(i, bytes)| PublishedAuditBlob {
                blob_id: BlobId::new(format!("{:064x}", i + 1)).unwrap(),
                size_bytes: bytes.len() as u64,
            })
            .collect();
        rendered.materialize(event_id, published).unwrap()
    };
    let audit = envelope(
        SessionEvent::RequestAuditRecorded {
            manifest: make_audit(EventId::new("input-event-4"), "request"),
        },
        4,
    );
    let mut pending = events.clone();
    pending.push(audit);
    let plan_blob = BlobId::new("b".repeat(64)).unwrap();
    assert!(
        source
            .validate_graph_pending(
                &pending,
                &[],
                &research,
                &plan_blob,
                &templated,
                "left",
                &[EventId::new("input-event-4")]
            )
            .is_ok()
    );
    let unrelated = envelope(
        SessionEvent::HistoryItemRecorded {
            item: ContentItem::user_text("late change"),
        },
        4,
    );
    let audit = envelope(
        SessionEvent::RequestAuditRecorded {
            manifest: make_audit(EventId::new("input-event-5"), "request"),
        },
        5,
    );
    let mut changed_prefix = events.clone();
    changed_prefix.extend([unrelated, audit]);
    assert!(
        source
            .validate_graph_pending(
                &changed_prefix,
                &[],
                &research,
                &plan_blob,
                &templated,
                "left",
                &[EventId::new("input-event-5")]
            )
            .is_err()
    );
    let retry = envelope(
        SessionEvent::RequestAuditRecorded {
            manifest: make_audit(EventId::new("input-event-5"), "different request"),
        },
        5,
    );
    pending.push(retry);
    assert!(
        source
            .validate_graph_pending(
                &pending,
                &[],
                &research,
                &plan_blob,
                &templated,
                "left",
                &[EventId::new("input-event-4"), EventId::new("input-event-5")]
            )
            .is_err()
    );
    let mut tampered = serde_json::to_value(&plan).unwrap();
    tampered["research_source"]["stream_hash"] = serde_json::json!("0".repeat(64));
    let tampered: CompactionSourcePlan = serde_json::from_value(tampered).unwrap();
    assert!(tampered.validate(&contents).is_err());
    assert!(
        plan.validate_source_facts(&contents, &events, &[], &research)
            .is_ok()
    );
    let foreign = ValidatedResearchSource::from_events(
        ProjectId::new("other"),
        &[],
        &ResearchReferenceCatalog::default(),
    )
    .unwrap();
    assert!(
        plan.validate_source_facts(&contents, &events, &[], &foreign)
            .is_err()
    );
    let corrupt = EventEnvelope::current(
        EventId::new("corrupt-research"),
        StreamId::new("research"),
        EventSequence::new(1),
        OccurredAt::new("2026-10-27T00:00:00Z"),
        EventType::new("research.unknown"),
        serde_json::json!({}),
    );
    assert!(
        ValidatedResearchSource::from_events(
            ProjectId::new("context2"),
            &[corrupt],
            &ResearchReferenceCatalog::default()
        )
        .is_err()
    );
    let mut changed = contents.clone();
    changed[1].push('x');
    assert!(plan.validate(&changed).is_err());
    let mut missing = nodes.clone();
    if let CompactionSourceNodeInput::Source(ranges) = &mut missing[1].input {
        ranges[0].start = split + 1;
    }
    assert!(
        CompactionSourcePlan::new(
            source.clone(),
            &research,
            entries.clone(),
            missing,
            &contents
        )
        .is_err()
    );
    let mut split_utf8 = nodes.clone();
    if let CompactionSourceNodeInput::Source(ranges) = &mut split_utf8[0].input {
        ranges[0].end = chinese + 1;
    }
    assert!(
        CompactionSourcePlan::new(
            source.clone(),
            &research,
            entries.clone(),
            split_utf8,
            &contents
        )
        .is_err()
    );
    let mut duplicate = nodes;
    if let CompactionSourceNodeInput::Merge(children) = &mut duplicate[2].input {
        children[1] = "left".into();
    }
    assert!(
        CompactionSourcePlan::new(
            source.clone(),
            &research,
            entries.clone(),
            duplicate.clone(),
            &contents
        )
        .is_err()
    );
    if let CompactionSourceNodeInput::Merge(children) = &mut duplicate[2].input {
        *children = vec!["right".into(), "left".into()];
    }
    assert!(
        CompactionSourcePlan::new(
            source.clone(),
            &research,
            entries.clone(),
            duplicate.clone(),
            &contents
        )
        .is_err()
    );
    duplicate[2].is_root = false;
    assert!(CompactionSourcePlan::new(source, &research, entries, duplicate, &contents).is_err());
}
