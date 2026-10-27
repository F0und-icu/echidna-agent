use super::*;

use super::compaction_support::{CompactionFacts, legacy_v1_summary_value};
use echidna::domain::session::{SessionSummary, SessionSummaryDraft};

fn compaction_facts() -> CompactionFacts {
    CompactionFacts::new("m3-agent-loop")
}

fn compaction_draft() -> SessionSummaryDraft {
    compaction_draft_with_goal("confirm the reproducible vulnerability boundary")
}

fn compaction_draft_with_goal(goal: &str) -> SessionSummaryDraft {
    compaction_facts().summary_draft(goal)
}

fn session_event(
    stream: &str,
    event: SessionEvent,
    id: &str,
    sequence: u64,
) -> echidna::domain::events::EventEnvelope<serde_json::Value> {
    event
        .into_envelope(
            EventId::new(id),
            StreamId::new(stream),
            EventSequence::new(sequence),
            OccurredAt::new("2026-08-23T00:00:00Z"),
        )
        .unwrap()
}

fn primary_ref(id: &str, generation: u64) -> ProviderSessionRef {
    ProviderSessionRef {
        id: ProviderSessionId::new(id),
        role: ProviderSessionRole::Primary,
        generation,
    }
}

fn compacted_session_state(stream: &str, primary_id: &str, generation: u64) -> SessionState {
    compacted_session_state_with_draft(stream, primary_id, generation, compaction_draft())
}

fn compacted_session_state_with_draft(
    stream: &str,
    primary_id: &str,
    generation: u64,
    draft: SessionSummaryDraft,
) -> SessionState {
    compacted_session_state_with_draft_and_retention(stream, primary_id, generation, draft, 1)
}

fn compacted_session_state_with_draft_and_retention(
    stream: &str,
    primary_id: &str,
    generation: u64,
    draft: SessionSummaryDraft,
    requested_recent_items: usize,
) -> SessionState {
    let mut state = SessionReducer
        .reduce([
            session_event(
                stream,
                SessionEvent::Started {
                    primary_provider_session: primary_ref(primary_id, generation),
                },
                "started",
                1,
            ),
            session_event(
                stream,
                SessionEvent::HistoryItemRecorded {
                    item: ContentItem::user_text("old durable history"),
                },
                "old-user",
                2,
            ),
            session_event(
                stream,
                SessionEvent::HistoryItemRecorded {
                    item: ContentItem::user_text("recent durable history"),
                },
                "recent-user",
                3,
            ),
        ])
        .unwrap();
    let facts = compaction_facts();
    let compaction = state
        .next_compaction(draft, requested_recent_items, &facts.validation())
        .unwrap();
    SessionReducer
        .apply(
            &mut state,
            session_event(
                stream,
                SessionEvent::CompactionCreated { compaction },
                "compaction",
                4,
            ),
        )
        .unwrap();
    state
}

fn resumed_compacted_session_state(
    stream: &str,
    primary_id: &str,
    generation: u64,
) -> SessionState {
    let mut state = SessionReducer
        .reduce([
            session_event(
                stream,
                SessionEvent::Started {
                    primary_provider_session: primary_ref("primary-before-resume", 1),
                },
                "started",
                1,
            ),
            session_event(
                stream,
                SessionEvent::Resumed {
                    primary_provider_session: primary_ref(primary_id, generation),
                },
                "resumed",
                2,
            ),
            session_event(
                stream,
                SessionEvent::HistoryItemRecorded {
                    item: ContentItem::user_text("old durable history"),
                },
                "old-user",
                3,
            ),
            session_event(
                stream,
                SessionEvent::HistoryItemRecorded {
                    item: ContentItem::user_text("recent durable history"),
                },
                "recent-user",
                4,
            ),
        ])
        .unwrap();
    let facts = compaction_facts();
    let compaction = state
        .next_compaction(compaction_draft(), 1, &facts.validation())
        .unwrap();
    SessionReducer
        .apply(
            &mut state,
            session_event(
                stream,
                SessionEvent::CompactionCreated { compaction },
                "compaction",
                5,
            ),
        )
        .unwrap();
    SessionReducer
        .apply(
            &mut state,
            session_event(
                stream,
                SessionEvent::HistoryItemRecorded {
                    item: ContentItem::user_text("post-compaction durable history"),
                },
                "post-compaction-user",
                6,
            ),
        )
        .unwrap();
    state
}

fn resumed_legacy_compacted_session_state(
    stream: &str,
    primary_id: &str,
    generation: u64,
) -> SessionState {
    let events = vec![
        session_event(
            stream,
            SessionEvent::Started {
                primary_provider_session: primary_ref("primary-before-resume", 1),
            },
            "legacy-started",
            1,
        ),
        session_event(
            stream,
            SessionEvent::Resumed {
                primary_provider_session: primary_ref(primary_id, generation),
            },
            "legacy-resumed",
            2,
        ),
        session_event(
            stream,
            SessionEvent::HistoryItemRecorded {
                item: ContentItem::user_text("legacy retained history"),
            },
            "legacy-history",
            3,
        ),
    ];
    let state = SessionReducer.reduce(events.clone()).unwrap();
    let raw_compaction = echidna::domain::events::EventEnvelope::current(
        EventId::new("legacy-compaction"),
        StreamId::new(stream),
        EventSequence::new(4),
        OccurredAt::new("2026-08-23T00:00:00Z"),
        echidna::domain::events::EventType::new("session.compaction_created"),
        serde_json::json!({
            "schema_version": 1,
            "summary": legacy_v1_summary_value(),
            "source_history_item_count": state.history().items().len(),
            "source_event_sequence": 3,
            "retained_from_normalized_history_index": 0,
            "retained_history": state.history().items(),
        }),
    );
    SessionReducer
        .reduce(events.into_iter().chain(std::iter::once(raw_compaction)))
        .unwrap()
}

fn make_agent<'provider, 'tools>(
    provider: &'provider ScriptedProvider,
    tools: &'tools ToolRegistry,
    primary_id: &str,
    generation: u64,
) -> AgentLoop<'provider, 'tools, ScriptedProvider, ToolRegistry> {
    AgentLoop::with_primary_context(
        provider,
        tools,
        ModelIdentity::new("fixture", "model"),
        ModelProfile::new("profile", 1),
        ProviderSession::new(
            ProviderSessionId::new(primary_id),
            ProviderSessionRole::Primary,
            generation,
        ),
    )
    .unwrap()
}

fn contains_summary(items: &[ContentItem], summary: &SessionSummary) -> bool {
    let fingerprint = summary.fingerprint();
    items.iter().any(|item| {
        matches!(
            item,
            ContentItem::Text {
                role: echidna::domain::model::MessageRole::User,
                text,
            } if text.starts_with("derived_session_summary_v2:") && text.contains(fingerprint)
        )
    })
}

#[tokio::test]
async fn candidate_summary_does_not_touch_an_active_baseline_but_durable_compaction_forces_one_full()
 {
    let facts = compaction_facts();
    let root = TemporaryDirectory::new("m3-session-compaction");
    let tools = ToolRegistry::for_project(root.path());
    let provider = ScriptedProvider::new(
        incremental_capabilities(),
        [
            completed_response("initial", "response-initial"),
            completed_response("candidate", "response-candidate"),
            completed_response("compacted", "response-compacted"),
            completed_response("continued", "response-continued"),
        ],
    );
    let mut agent = make_agent(&provider, &tools, "primary-compaction", 1);
    agent
        .run_turn("volatile first input", AgentCancellation::new())
        .await
        .unwrap();

    let session = compacted_session_state("session:compaction-runtime", "primary-compaction", 1);
    let candidate_draft = compaction_draft();
    let candidate = session
        .next_compaction(candidate_draft.clone(), 1, &facts.validation())
        .unwrap();
    assert_eq!(candidate.summary().revision(), 2);
    agent.update_session_summary_candidate(candidate_draft);
    agent
        .run_turn("baseline remains incremental", AgentCancellation::new())
        .await
        .unwrap();
    assert!(matches!(
        agent.last_baseline_assessment(),
        Some(BaselineAssessment::AppendOnlyCandidate { .. })
    ));

    agent
        .apply_compaction(&session, Some(&facts.validation()))
        .unwrap();
    agent
        .run_turn(
            "first real request after compaction",
            AgentCancellation::new(),
        )
        .await
        .unwrap();
    assert_eq!(
        agent.last_baseline_assessment(),
        Some(&BaselineAssessment::FullRequest {
            reason: BaselineInvalidationReason::CompactionReset,
        })
    );
    agent
        .apply_compaction(&session, Some(&facts.validation()))
        .unwrap();
    agent
        .run_turn("second request after compaction", AgentCancellation::new())
        .await
        .unwrap();

    let requests = provider.requests();
    assert_eq!(requests.len(), 4);
    assert!(requests[1].is_incremental());
    assert!(!requests[2].is_incremental());
    assert!(contains_summary(
        &requests[2].input().content_items,
        session.latest_compaction().unwrap().summary()
    ));
    let summary = session.latest_compaction().unwrap().summary();
    let durable = serde_json::to_value(summary).unwrap();
    let projected = requests[2]
        .input()
        .content_items
        .iter()
        .find_map(|item| match item {
            ContentItem::Text { text, .. } => text.strip_prefix("derived_session_summary_v2:"),
            _ => None,
        })
        .unwrap();
    let projected: serde_json::Value = serde_json::from_str(projected).unwrap();
    assert_eq!(projected["narrative"], durable["narrative"]);
    assert_eq!(projected["evidence_spine"], durable["evidence_spine"]);
    assert!(requests[3].is_incremental());
}

#[test]
fn compaction_application_is_idempotent_only_for_the_exact_durable_fact() {
    let facts = compaction_facts();
    let root = TemporaryDirectory::new("m3-session-compaction-conflict");
    let tools = ToolRegistry::for_project(root.path());
    let provider = ScriptedProvider::new(incremental_capabilities(), []);
    let mut agent = make_agent(&provider, &tools, "primary-compaction", 1);
    let state = compacted_session_state("session:compaction-conflict", "primary-compaction", 1);
    let mut mismatched_candidate_agent = make_agent(&provider, &tools, "primary-compaction", 1);
    mismatched_candidate_agent.update_session_summary_candidate(compaction_draft_with_goal(
        "a local candidate must match the durable Compaction",
    ));
    assert!(matches!(
        mismatched_candidate_agent.apply_compaction(&state, Some(&facts.validation())),
        Err(
            echidna::app::agent_loop::AgentLoopConfigurationError::CompactionSummaryCandidateMismatch
        )
    ));
    assert!(mismatched_candidate_agent.history().items().is_empty());
    let conflicting = compacted_session_state_with_draft(
        "session:compaction-conflict",
        "primary-compaction",
        1,
        compaction_draft_with_goal("a distinct valid Summary cannot replace revision one"),
    );
    let conflicting_retention = compacted_session_state_with_draft_and_retention(
        "session:compaction-conflict",
        "primary-compaction",
        1,
        compaction_draft(),
        2,
    );
    let mut later = state.clone();
    let next = later
        .next_compaction(compaction_draft(), 1, &facts.validation())
        .unwrap();
    SessionReducer
        .apply(
            &mut later,
            session_event(
                "session:compaction-conflict",
                SessionEvent::CompactionCreated { compaction: next },
                "compaction-revision-two",
                5,
            ),
        )
        .unwrap();

    agent
        .apply_compaction(&state, Some(&facts.validation()))
        .unwrap();
    agent
        .apply_compaction(&state, Some(&facts.validation()))
        .unwrap();
    assert!(matches!(
        agent.apply_compaction(&conflicting, Some(&facts.validation())),
        Err(echidna::app::agent_loop::AgentLoopConfigurationError::ConflictingCompaction { .. })
    ));
    assert!(matches!(
        agent.apply_compaction(&conflicting_retention, Some(&facts.validation())),
        Err(echidna::app::agent_loop::AgentLoopConfigurationError::ConflictingCompaction { .. })
    ));
    agent
        .apply_compaction(&later, Some(&facts.validation()))
        .unwrap();
    assert!(matches!(
        agent.apply_compaction(&state, Some(&facts.validation())),
        Err(echidna::app::agent_loop::AgentLoopConfigurationError::StaleCompaction { .. })
    ));
}

#[tokio::test]
async fn failed_empty_and_cancelled_compaction_full_do_not_consume_the_reset() {
    let facts = compaction_facts();
    let session = compacted_session_state("session:compaction-retry", "primary-compaction", 1);

    let root = TemporaryDirectory::new("m3-session-compaction-failed");
    let tools = ToolRegistry::for_project(root.path());
    let provider = ScriptedProvider::new(
        incremental_capabilities(),
        [
            failed_response("temporary failure"),
            completed_response("retry", "response-retry"),
        ],
    );
    let mut agent = make_agent(&provider, &tools, "primary-compaction", 1);
    agent
        .apply_compaction(&session, Some(&facts.validation()))
        .unwrap();
    assert!(matches!(
        agent
            .run_turn("failed full", AgentCancellation::new())
            .await,
        Err(echidna::app::agent_loop::AgentLoopError::Model(_))
    ));
    assert_eq!(
        agent.last_baseline_assessment(),
        Some(&BaselineAssessment::FullRequest {
            reason: BaselineInvalidationReason::CompactionReset,
        })
    );
    agent
        .run_turn("retry full", AgentCancellation::new())
        .await
        .unwrap();
    assert!(
        provider
            .requests()
            .iter()
            .all(|request| !request.is_incremental())
    );

    let root = TemporaryDirectory::new("m3-session-compaction-empty");
    let tools = ToolRegistry::for_project(root.path());
    let provider = ScriptedProvider::new(
        incremental_capabilities(),
        [
            vec![Ok(ModelStreamEvent::Completed(
                ModelResponseCompletion::new(ProviderResponseId::new("empty")),
            ))],
            completed_response("retry", "response-after-empty"),
        ],
    );
    let mut agent = make_agent(&provider, &tools, "primary-compaction", 1);
    agent
        .apply_compaction(&session, Some(&facts.validation()))
        .unwrap();
    assert!(matches!(
        agent.run_turn("empty full", AgentCancellation::new()).await,
        Err(echidna::app::agent_loop::AgentLoopError::EmptyModelResponse)
    ));
    assert_eq!(
        agent.last_baseline_assessment(),
        Some(&BaselineAssessment::FullRequest {
            reason: BaselineInvalidationReason::CompactionReset,
        })
    );
    agent
        .run_turn("retry after empty", AgentCancellation::new())
        .await
        .unwrap();
    assert!(
        provider
            .requests()
            .iter()
            .all(|request| !request.is_incremental())
    );

    let root = TemporaryDirectory::new("m3-session-compaction-cancelled");
    let tools = ToolRegistry::for_project(root.path());
    let provider = ScriptedProvider::new(
        incremental_capabilities(),
        [completed_response("after cancel", "response-after-cancel")],
    );
    let mut agent = make_agent(&provider, &tools, "primary-compaction", 1);
    agent
        .apply_compaction(&session, Some(&facts.validation()))
        .unwrap();
    let cancellation = AgentCancellation::new();
    cancellation.cancel();
    assert!(matches!(
        agent
            .run_turn("cancelled full", cancellation)
            .await
            .unwrap(),
        AgentTurnOutcome::Cancelled {
            model_requests: 0,
            ..
        }
    ));
    agent
        .run_turn("first uncancelled request", AgentCancellation::new())
        .await
        .unwrap();
    assert_eq!(
        agent.last_baseline_assessment(),
        Some(&BaselineAssessment::FullRequest {
            reason: BaselineInvalidationReason::CompactionReset,
        })
    );
}

#[tokio::test]
async fn resume_restores_the_compacted_view_with_session_resumed_and_rejects_another_stream() {
    let facts = compaction_facts();
    let root = TemporaryDirectory::new("m3-session-compaction-resume");
    let tools = ToolRegistry::for_project(root.path());
    let provider = ScriptedProvider::new(
        incremental_capabilities(),
        [completed_response("resumed", "response-resumed")],
    );
    let mut agent = make_agent(&provider, &tools, "primary-before-runtime", 1);
    let session =
        resumed_compacted_session_state("session:resume-compaction", "primary-resumed-runtime", 2);
    agent
        .resume_from_session(
            &session,
            ProviderSession::new(
                ProviderSessionId::new("primary-resumed-runtime"),
                ProviderSessionRole::Primary,
                2,
            ),
            Some(&facts.validation()),
        )
        .unwrap();
    assert!(agent.history().items().iter().any(|item| matches!(
        item,
        ContentItem::Text { text, .. } if text == "recent durable history"
    )));
    assert!(agent.history().items().iter().any(|item| matches!(
        item,
        ContentItem::Text { text, .. } if text == "post-compaction durable history"
    )));
    assert!(!agent.history().items().iter().any(|item| matches!(
        item,
        ContentItem::Text { text, .. } if text == "old durable history"
    )));
    agent
        .run_turn("continue restored session", AgentCancellation::new())
        .await
        .unwrap();
    assert_eq!(
        agent.last_baseline_assessment(),
        Some(&BaselineAssessment::FullRequest {
            reason: BaselineInvalidationReason::SessionResumed,
        })
    );
    assert!(contains_summary(
        &provider.requests()[0].input().content_items,
        session.latest_compaction().unwrap().summary()
    ));

    let another =
        compacted_session_state("session:another-compaction", "primary-resumed-runtime", 2);
    assert!(matches!(
        agent.apply_compaction(&another, Some(&facts.validation())),
        Err(echidna::app::agent_loop::AgentLoopConfigurationError::CompactionSessionStreamMismatch { .. })
    ));
}

#[test]
fn summary_v2_resume_requires_current_references_before_mutating_the_agent_loop() {
    let facts = compaction_facts();
    let wrong_facts = CompactionFacts::new("wrong-project");
    let root = TemporaryDirectory::new("m4-summary-v2-resume-validation");
    let tools = ToolRegistry::for_project(root.path());
    let provider = ScriptedProvider::new(incremental_capabilities(), []);
    let mut agent = make_agent(&provider, &tools, "primary-before-runtime", 1);
    let session =
        resumed_compacted_session_state("session:v2-resume-validation", "primary-v2-runtime", 2);
    let resumed_provider = || {
        ProviderSession::new(
            ProviderSessionId::new("primary-v2-runtime"),
            ProviderSessionRole::Primary,
            2,
        )
    };

    assert!(matches!(
        agent.resume_from_session(&session, resumed_provider(), None),
        Err(
            echidna::app::agent_loop::AgentLoopConfigurationError::CompactionValidationContextRequired
        )
    ));
    assert!(agent.history().items().is_empty());
    assert!(agent.last_sampling_step_plan().is_none());

    assert!(matches!(
        agent.resume_from_session(
            &session,
            resumed_provider(),
            Some(&wrong_facts.validation()),
        ),
        Err(
            echidna::app::agent_loop::AgentLoopConfigurationError::Session(
                echidna::domain::session::SessionError::CompactionEvidenceCrossProject { .. }
            )
        )
    ));
    assert!(agent.history().items().is_empty());
    assert!(agent.last_sampling_step_plan().is_none());

    agent
        .resume_from_session(&session, resumed_provider(), Some(&facts.validation()))
        .unwrap();
    assert!(agent.history().items().iter().any(|item| matches!(
        item,
        ContentItem::Text { text, .. } if text == "recent durable history"
    )));
}

#[test]
fn legacy_summary_v1_resume_remains_readable_without_reference_validation() {
    let root = TemporaryDirectory::new("m4-summary-v1-resume");
    let tools = ToolRegistry::for_project(root.path());
    let provider = ScriptedProvider::new(incremental_capabilities(), []);
    let mut agent = make_agent(&provider, &tools, "primary-before-runtime", 1);
    let session =
        resumed_legacy_compacted_session_state("session:v1-resume", "primary-v1-runtime", 2);

    agent
        .resume_from_session(
            &session,
            ProviderSession::new(
                ProviderSessionId::new("primary-v1-runtime"),
                ProviderSessionRole::Primary,
                2,
            ),
            None,
        )
        .unwrap();

    assert!(agent.history().items().iter().any(|item| matches!(
        item,
        ContentItem::Text { text, .. } if text == "legacy retained history"
    )));
    assert!(agent.last_sampling_step_plan().is_none());
}
