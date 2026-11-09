//! Production CLI composition for explicit Session Compaction and local
//! Context-pressure presentation. Durable facts and Context decisions retain
//! their existing Domain/App owners.

use crate::adapters::session_compaction::{SessionCompactionFacts, load_session_compaction_facts};
use crate::adapters::tools::ToolRegistry;
use crate::app::agent_loop::AgentLoop;
use crate::domain::context::{
    ContextDecision, ContextDecisionManifest, ContextPlan, ContextWindowPolicy,
};
use crate::domain::session::{
    SessionCompaction, SessionError, SessionEvent, SessionState, SessionSummaryDraft,
};
use crate::ports::artifact::ArtifactStore;
use crate::ports::model::ModelProvider;

use super::{ChatError, DurableChatSession, event_id, occurred_at};

pub(super) struct CompactionRequestContext<'a> {
    pub(super) composition: &'a crate::provider_composition::ProviderComposition,
    pub(super) budget: &'a crate::app::model_request_budget::ModelRequestBudget,
    pub(super) cancellation: crate::app::cancellation::AgentCancellation,
    pub(super) audit: &'a crate::domain::request_audit::RequestAuditRunContext,
}

pub(super) struct AutomaticToolBatchCompaction<'a> {
    pub(super) session: &'a DurableChatSession,
    pub(super) window_tokens: u32,
    pub(super) request: CompactionRequestContext<'a>,
    pub(super) deferred: bool,
    pub(super) end_answer: &'a dyn Fn(),
    pub(super) composition: &'a crate::provider_composition::ProviderComposition,
    pub(super) tools: &'a ToolRegistry,
    pub(super) discovered_mcp: &'a [crate::adapters::mcp::McpCatalogTool],
    pub(super) task: &'a str,
    pub(super) history_start: usize,
    pub(super) no_pending_images: bool,
    pub(super) binding: crate::domain::agent_definition::AgentDefinitionBinding,
    pub(super) batches: usize,
}

impl<Provider: ModelProvider + ?Sized>
    crate::app::agent_loop::ToolBatchStage<Provider, ToolRegistry>
    for AutomaticToolBatchCompaction<'_>
{
    fn needs_current_context(&self) -> bool {
        self.composition.runtime().auto_compact
    }

    fn apply<'call, 'provider: 'call, 'tools: 'call>(
        &'call mut self,
        agent: &'call mut AgentLoop<'provider, 'tools, Provider, ToolRegistry>,
        active: &'call crate::app::agent_loop::ToolBatchContext<'_>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<(), crate::app::agent_loop::AgentLoopError>>
                + 'call,
        >,
    > {
        Box::pin(async move {
            self.batches += 1;
            // Every enabled, settled batch receives a local assessment, even
            // when a failed handoff or the shared admission limit prevents sampling.
            let needs_compaction = if self.composition.runtime().auto_compact
                && !self.request.cancellation.is_cancelled()
            {
                Some(agent.batch_needs_compaction(active)?)
            } else {
                None
            };
            let no_admissions = self.request.budget.remaining() == Some(0);
            if let Some(needs_compaction) = needs_compaction
                && (self.deferred || no_admissions || active.task_selection_pending)
                && self.composition.diagnostic_output_enabled()
            {
                console_eprintln!(
                    "[debug context pressure] settled batch evaluated; threshold_reached={needs_compaction}; automatic handoff deferred for this Turn"
                );
            }
            if no_admissions || active.task_selection_pending {
                return Ok(());
            }
            if !active.source_refresh_pending
                && self.batches.is_multiple_of(16)
                && crate::app::improvement::policy(self.binding.definition()).is_some()
            {
                let mut accepted = None;
                super::improvement::assess(
                    self.session,
                    &self.binding,
                    agent,
                    self.tools,
                    self.task,
                    self.history_start,
                    self.no_pending_images,
                    self.request.budget,
                    self.request.cancellation.clone(),
                    self.request.audit,
                    self.composition,
                    self.discovered_mcp,
                    crate::app::agent_loop::ImprovementBoundary::ToolBatch(active),
                    &mut accepted,
                )
                .await
                .map_err(|error| {
                    crate::app::agent_loop::AgentLoopError::ToolBatchCompaction {
                        message: error.to_string(),
                    }
                })?;
                if let Some(accepted) = accepted {
                    if self.deferred {
                        console_eprintln!(
                            "[RSI checkpoint deferred] Automatic handoff already failed this Turn; candidate remains pending and old instructions remain active."
                        );
                        return Ok(());
                    }
                    let updated = super::evolution::apply_checkpoint(
                        self.session,
                        &self.binding,
                        accepted,
                        agent,
                        active,
                        self.window_tokens,
                        CompactionRequestContext {
                            composition: self.request.composition,
                            budget: self.request.budget,
                            cancellation: self.request.cancellation.clone(),
                            audit: self.request.audit,
                        },
                    )
                    .await
                    .map_err(|error| {
                        crate::app::agent_loop::AgentLoopError::ToolBatchCompaction {
                            message: error.to_string(),
                        }
                    })?;
                    match updated {
                        super::evolution::CheckpointOutcome::Applied(binding) => {
                            self.session.publish_binding(&binding).map_err(|error| {
                                crate::app::agent_loop::AgentLoopError::ToolBatchCompaction {
                                    message: error.to_string(),
                                }
                            })?;
                            self.binding = *binding
                        }
                        super::evolution::CheckpointOutcome::Deferred { failed_handoff } => {
                            self.deferred |= failed_handoff;
                        }
                    }
                    return Ok(());
                }
            }
            let Some(needs_compaction) = needs_compaction else {
                return Ok(());
            };
            if self.deferred || !needs_compaction {
                return Ok(());
            }
            (self.end_answer)();
            let admitted_before = self.request.budget.counts().primary;
            match self
                .session
                .compact_tool_batch(
                    agent,
                    self.window_tokens,
                    active,
                    CompactionRequestContext {
                        composition: self.request.composition,
                        budget: self.request.budget,
                        cancellation: self.request.cancellation.clone(),
                        audit: self.request.audit,
                    },
                )
                .await
            {
                Ok(summary) => console_println!(
                    "Automatic tool-batch compaction applied. {} Current task and Turn budget retained.",
                    summary.message()
                ),
                Err(error) if automatically_deferred(&error) => {
                    console_println!(
                        "Automatic tool-batch compaction deferred: {error}; Provider delivery retained."
                    );
                    // Only a failed model handoff suppresses a new automatic
                    // sample for this Turn. A later settled batch changes the
                    // actual source and must still receive a local evaluation.
                    if failed_handoff(&error, admitted_before, self.request.budget) {
                        self.deferred = true;
                    }
                }
                Err(error) => {
                    return Err(
                        crate::app::agent_loop::AgentLoopError::ToolBatchCompaction {
                            message: error.to_string(),
                        },
                    );
                }
            }
            Ok(())
        })
    }
}

pub(super) fn failed_handoff(
    error: &ChatError,
    admitted_before: usize,
    budget: &crate::app::model_request_budget::ModelRequestBudget,
) -> bool {
    budget.counts().primary > admitted_before
        && matches!(
            error,
            ChatError::Agent(
                crate::app::agent_loop::AgentLoopError::CompactionSummaryRejected { .. }
                    | crate::app::agent_loop::AgentLoopError::Model(_)
            )
        )
}

pub(super) fn automatically_deferred(error: &ChatError) -> bool {
    matches!(error,
        ChatError::CompactionNotUseful
        | ChatError::CompactionFacts(crate::adapters::research_projection::CompactionValidationFactsError::ResearchStreamEmpty)
        | ChatError::Session(SessionError::CompactionCurrentTargetRequired)
        | ChatError::Session(SessionError::CompactionSummaryTooLarge { .. })
        | ChatError::Agent(crate::app::agent_loop::AgentLoopError::CompactionSummaryRejected { .. }
            | crate::app::agent_loop::AgentLoopError::CompactionCancelled
            | crate::app::agent_loop::AgentLoopError::Model(_)
            | crate::app::agent_loop::AgentLoopError::ModelRequestLimit { .. }
            | crate::app::agent_loop::AgentLoopError::ContextPlanner(crate::domain::context::ContextPlannerError::RequiredFragmentsExceedBudget { .. }
                | crate::domain::context::ContextPlannerError::RenderedInputExceedsBudget { .. }))
    )
}

pub(super) fn explicitly_deferred(error: &ChatError) -> bool {
    automatically_deferred(error)
}

pub(super) struct AppliedCompactionSummary {
    revision: u64,
    source_history_items: usize,
    retained_history_items: usize,
    reached_soft_target: bool,
}

impl AppliedCompactionSummary {
    pub(super) fn message(&self) -> String {
        let target = if self.reached_soft_target {
            "Preferred input target reached."
        } else {
            "Preferred input target not reached; the smaller candidate fits the local hard input budget."
        };
        format!(
            "Compaction revision {} applied: retained {} of {} source History items; the next Primary request will be CompactionReset Full. {target} Historical outputs may be omitted; Summary is lossy.",
            self.revision, self.retained_history_items, self.source_history_items
        )
    }

    pub(super) fn automatic_message(&self) -> String {
        format!(
            "Automatic completed-Turn compaction applied because runtime.auto_compact is enabled. {} Completed Provider delivery state was intentionally reset at this boundary.",
            self.message()
        )
    }
}

impl DurableChatSession {
    pub(super) fn rebuild_compaction(
        &self,
        state: &SessionState,
        facts: &SessionCompactionFacts,
        composition: &crate::provider_composition::ProviderComposition,
    ) -> Result<SessionCompaction, ChatError> {
        // Refresh durable references independently of admission to a model
        // request. The caller performs the normal capacity preflight afterward.
        let (_, candidate, _) = select_compaction(
            state,
            facts,
            composition
                .runtime()
                .context_budget
                .for_capabilities(composition.agent_profile().primary().capabilities())
                .map_err(crate::app::agent_loop::AgentLoopError::from)?,
            state
                .latest_compaction()
                .and_then(|compaction| compaction.summary().narrative()),
            CompactionSelection::SourceRefresh,
            |_| Ok(None),
        )?;
        state.protected_operator_context_for(&candidate)?;
        Ok(candidate)
    }

    /// Builds one candidate from strict projections, gives that candidate to
    /// the AgentLoop, appends exactly one durable Event, then applies only the
    /// freshly replayed durable Compaction. Any failure exits Chat rather than
    /// continuing with divergent runtime and persisted state.
    pub(super) async fn compact<Provider>(
        &self,
        agent: &mut AgentLoop<'_, '_, Provider, ToolRegistry>,
        window_tokens: u32,
        request: CompactionRequestContext<'_>,
    ) -> Result<AppliedCompactionSummary, ChatError>
    where
        Provider: ModelProvider + ?Sized,
    {
        self.compact_with_boundary(agent, window_tokens, None, None, request)
            .await
    }

    pub(super) async fn compact_tool_batch<Provider>(
        &self,
        agent: &mut AgentLoop<'_, '_, Provider, ToolRegistry>,
        window_tokens: u32,
        active: &crate::app::agent_loop::ToolBatchContext<'_>,
        request: CompactionRequestContext<'_>,
    ) -> Result<AppliedCompactionSummary, ChatError>
    where
        Provider: ModelProvider + ?Sized,
    {
        self.compact_with_boundary(agent, window_tokens, Some(active), None, request)
            .await
    }

    pub(super) async fn compact_for_definition<Provider: ModelProvider + ?Sized>(
        &self,
        agent: &mut AgentLoop<'_, '_, Provider, ToolRegistry>,
        window_tokens: u32,
        active: &crate::app::agent_loop::ToolBatchContext<'_>,
        prompt: &crate::domain::research_control::PromptComposition,
        request: CompactionRequestContext<'_>,
    ) -> Result<AppliedCompactionSummary, ChatError> {
        self.compact_with_boundary(agent, window_tokens, Some(active), Some(prompt), request)
            .await
    }

    async fn compact_with_boundary<Provider>(
        &self,
        agent: &mut AgentLoop<'_, '_, Provider, ToolRegistry>,
        _window_tokens: u32,
        active: Option<&crate::app::agent_loop::ToolBatchContext<'_>>,
        replacement: Option<&crate::domain::research_control::PromptComposition>,
        request: CompactionRequestContext<'_>,
    ) -> Result<AppliedCompactionSummary, ChatError>
    where
        Provider: ModelProvider + ?Sized,
    {
        let policy = request
            .composition
            .runtime()
            .context_budget
            .for_capabilities(request.composition.agent_profile().primary().capabilities())
            .map_err(crate::app::agent_loop::AgentLoopError::from)?;
        let protected = self.durable_state()?.current_protected_operator_context()?;
        let capture = agent.capture_compaction_context_for_task(
            request.composition.compaction_provider().0,
            policy.compaction_reference_items(),
            agent.has_task_context(),
            Some(&protected),
        )?;
        let capture_bytes =
            serde_json::to_vec(&capture).map_err(|error| ChatError::CompactionSummarySource {
                message: error.to_string(),
            })?;
        let (source, goal, references, entries, contents, output_limit, capture_blob, selection_event) =
            self.with_projection(|projection, store, stream_id| {
                let state = projection.state();
                self.validate_protected_sources(state)?;
                agent.validate_compaction_session(state)?;
                match state.latest_task_context_selection() {
                    Some((_, selection)) => {
                        let bytes = self.artifacts.read_blob(&selection.capture_blob).map_err(|error| ChatError::CompactionSummarySource { message: format!("task capture read failed: {error}") })?;
                        let task_capture: crate::domain::session::TaskContextCapture = serde_json::from_slice(&bytes)
                            .map_err(|error| ChatError::CompactionSummarySource { message: format!("task capture is invalid: {error}") })?;
                        let research_units = if task_capture.compile.research_enabled {
                            let read = self.project_read(state)?;
                            let end = read.events.partition_point(|event| event.sequence.get() <= selection.source.project_stream_watermark());
                            let hash = crate::domain::session::SessionSourceIdentity::project_hash_for_events(&read.events[..end])?;
                            if hash != selection.source.project_stream_hash() { return Err(ChatError::CompactionSummarySource { message: "selected Research source changed".to_owned() }); }
                            state.task_research_units(&hash, &task_capture.compile, &read.events[..end], &read.references, &selection.selected_research)?
                        } else { Vec::new() };
                        let events = crate::adapters::project_event_store::read_existing_session_events(&self.project, &self.session_id)?;
                        crate::app::agent_loop::verify_task_compaction_capture(state, selection, &events, &task_capture, &capture, &research_units)?;
                    }
                    None if capture.is_task_scoped() => return Err(ChatError::CompactionSummarySource { message: "task Compaction has no selected source".to_owned() }),
                    None => {}
                }
                state
                    .ensure_completed_history()
                    .map_err(SessionError::from)?;
                super::recovery::validate_runtime_history(state, agent)?;
                let facts = load_session_compaction_facts(&self.project, state, state.latest_task_context_selection().is_some())
                    .map_err(ChatError::CompactionFacts)?;
                // Do not spend a model request when even the original local
                // capacity/retention preflight proves no useful replacement fits.
                select_compaction(state, &facts, policy, None, CompactionSelection::Preview, |candidate| {
                    let protected = state.protected_operator_context_for(candidate)?;
                    match (active, replacement) {
                        (Some(active), Some(prompt)) => agent
                            .preflight_checkpoint_definition(candidate, active, prompt, &protected),
                        (Some(active), None) => {
                            agent.preflight_tool_batch_compaction(candidate, active, &protected)
                        }
                        (None, _) => agent.preflight_compaction(candidate, true, &protected),
                    }
                    .map_err(ChatError::from)
                })?;
                let mut goal = if let Some((_, selected)) = state.latest_task_context_selection() {
                    selected.task_revisions.first().ok_or(SessionError::CompactionResumableHistoryRequired)?.goal.clone()
                } else {
                    let previous = state.latest_compaction().map(|value| value.summary());
                    crate::adapters::research_projection::continuation_text(
                        &state.full_normalized_history().map_err(SessionError::from)?, previous, policy,
                    )?.0
                };
                if !state.protected_operator_inputs()?.is_empty() {
                    goal = "The earlier operator request and constraints are retained verbatim in this context.".to_owned();
                }
                let references = state
                    .history_items_with_event_ids()
                    .map(|(id, _)| id.clone())
                    .rev()
                    .take(policy.compaction_reference_items())
                    .collect::<Vec<_>>();
                let output_limit = crate::domain::session::SessionSummaryDraft::v6_handoff_byte_limit(
                    &goal,
                    policy.summary_bytes(),
                    crate::app::agent_loop::minimum_compaction_handoff_bytes(),
                )?;
                let blob_id = self.artifacts.publish_bytes(&capture_bytes).map_err(|error| ChatError::CompactionSummarySource { message: format!("Compaction context publication failed: {error}") })?.blob_id;
                let sequence = store.next_sequence()?;
                let event = SessionEvent::CompactionContextCaptured { blob_id: blob_id.clone() }.into_envelope(
                    event_id(self.project.project_id(), &self.session_id, sequence.get(), "compaction-context"), stream_id, sequence, occurred_at(),
                )?;
                projection.append(store, event).map_err(ChatError::from_session_append)?;
                let facts = load_session_compaction_facts(&self.project, projection.state(), projection.state().latest_task_context_selection().is_some()).map_err(ChatError::CompactionFacts)?;
                let source = facts.task_source()?.clone();
                let events = crate::adapters::project_event_store::read_existing_session_events(
                    &self.project,
                    &self.session_id,
                )?;
                let project_events =
                    crate::adapters::project_event_store::read_existing_research_events(
                        &self.project,
                    )?;
                let selection_event = projection.state().latest_task_context_selection().map(|(id, _)| id.clone());
                let (entries, contents) =
                    crate::domain::session::CompactionSourcePlan::source_entries_for_selection(
                        &source,
                        &events,
                        &project_events,
                        selection_event.as_ref(),
                    )?;
                Ok((source, goal, references, entries, contents, output_limit, blob_id, selection_event))
            })?;
        let verify_capture = || -> Result<(), ChatError> {
            self.artifacts.read_blob(&capture_blob).map_err(|error| {
                ChatError::CompactionSummarySource {
                    message: format!("Compaction context Blob read failed: {error}"),
                }
            })?;
            Ok(())
        };
        verify_capture()?;
        console_eprintln!(
            "[compaction] Requesting a tool-free model handoff; original Context remains active until validation and durable append."
        );
        let graph = self
            .sample_compaction_graph(
                agent,
                &request,
                &capture,
                &source,
                &goal,
                &references,
                &entries,
                &contents,
                selection_event.as_ref(),
                policy.summary_bytes(),
                output_limit,
            )
            .await?;
        if request.cancellation.is_cancelled() {
            return Err(crate::app::agent_loop::AgentLoopError::CompactionCancelled.into());
        }
        self.with_projection(|projection, store, stream_id| {
            verify_capture()?;
            let events = crate::adapters::project_event_store::read_existing_session_events(
                &self.project,
                &self.session_id,
            )?;
            let project_events =
                crate::adapters::project_event_store::read_existing_research_events(&self.project)?;
            source.validate_graph_sampling_suffix(
                &events,
                &project_events,
                &graph.root_event_id,
            )?;
            crate::app::session::verify_compaction_source_blobs(&graph, &self.artifacts)?;
            agent.validate_compaction_session(projection.state())?;
            projection
                .state()
                .ensure_completed_history()
                .map_err(crate::domain::session::SessionError::from)?;
            super::recovery::validate_runtime_history(projection.state(), agent)?;

            // The Writer lock freezes both Research and Session appenders while
            // these read-only facts and the new Compaction Event are composed.
            let facts = load_session_compaction_facts(
                &self.project,
                projection.state(),
                projection.state().latest_task_context_selection().is_some(),
            )
            .map_err(ChatError::CompactionFacts)?
            .with_verified_graph(graph)?;
            let (draft, compaction, reached_soft_target) = select_compaction(
                projection.state(),
                &facts,
                policy,
                None,
                CompactionSelection::ModelAdoption,
                |candidate| {
                    let protected = projection
                        .state()
                        .protected_operator_context_for(candidate)?;
                    match (active, replacement) {
                        (Some(active), Some(prompt)) => agent
                            .preflight_checkpoint_definition(candidate, active, prompt, &protected),
                        (Some(active), None) => {
                            agent.preflight_tool_batch_compaction(candidate, active, &protected)
                        }
                        (None, _) => agent.preflight_compaction(candidate, true, &protected),
                    }
                    .map_err(ChatError::from)
                },
            )?;
            let validation = facts.context();
            let result = AppliedCompactionSummary {
                revision: compaction.summary().revision(),
                source_history_items: compaction.source_history_item_count(),
                retained_history_items: compaction.retained_history().len(),
                reached_soft_target,
            };

            // Candidate state cannot affect active Context. It exists only to
            // prove that the Event applied below contains this exact draft.
            agent.update_session_summary_candidate(draft);
            let sequence = store.next_sequence()?;
            let event = SessionEvent::CompactionCreated { compaction }.into_envelope(
                event_id(
                    self.project.project_id(),
                    &self.session_id,
                    sequence.get(),
                    "compaction",
                ),
                stream_id,
                sequence,
                occurred_at(),
            )?;
            projection
                .append(store, event)
                .map_err(ChatError::from_session_append)?;
            if let Some(source) = facts.task_projection() {
                projection.retain_compaction_source(source.clone())?;
            }
            agent.apply_compaction(projection.state(), Some(&validation))?;
            Ok(result)
        })
    }
}

/// One candidate/reduction owner shared by explicit compact and stale-source
/// reconstruction. Complete suffix selection remains Session/History-owned.
#[derive(Clone, Copy, Eq, PartialEq)]
enum CompactionSelection {
    Preview,
    ModelAdoption,
    SourceRefresh,
}

fn select_compaction(
    state: &SessionState,
    facts: &SessionCompactionFacts,
    policy: ContextWindowPolicy,
    historical_narrative: Option<&crate::domain::session::NarrativeSummary>,
    selection: CompactionSelection,
    mut fits: impl FnMut(
        &SessionCompaction,
    ) -> Result<Option<crate::app::agent_loop::CompactionFit>, ChatError>,
) -> Result<(SessionSummaryDraft, SessionCompaction, bool), ChatError> {
    let history = state
        .full_normalized_history()
        .map_err(SessionError::from)?;
    let minimum_recent = state.latest_uncompleted_batch_items()?;
    let mut references = policy.compaction_reference_items();
    let mut smallest: Option<(u64, SessionSummaryDraft, SessionCompaction)> = None;
    let mut summary_too_large = None;
    let mut valid_candidate_seen = false;
    loop {
        let draft = if let Some((_, selected)) = state.latest_task_context_selection()
            && selection != CompactionSelection::SourceRefresh
        {
            let goal = &selected
                .task_revisions
                .first()
                .ok_or(SessionError::CompactionResumableHistoryRequired)?
                .goal;
            facts.compose_selected_task_draft(goal, references)?
        } else {
            facts.compose_session_summary_draft(
                &history,
                state
                    .latest_compaction()
                    .map(|compaction| compaction.summary()),
                references,
                policy,
            )?
        };
        let mut draft = if selection == CompactionSelection::Preview {
            draft.with_preview_byte_limit(policy.summary_bytes())?
        } else {
            let limit = if selection == CompactionSelection::ModelAdoption
                && facts.verified_graph().is_some()
            {
                policy.summary_bytes()
            } else {
                state
                    .latest_compaction()
                    .and_then(|compaction| compaction.summary().historical_byte_limit())
                    .ok_or_else(|| ChatError::CompactionSummarySource {
                        message: "historical refresh requires an existing Summary".to_owned(),
                    })?
            };
            draft.with_byte_limit(limit)?
        };
        if let Some(narrative) = historical_narrative {
            draft = draft.with_historical_narrative(narrative.clone());
        }
        let mut retained = state.compaction_source_history()?.len();
        loop {
            match state.next_compaction(draft.clone(), retained, &facts.context()) {
                Ok(candidate) => {
                    valid_candidate_seen = true;
                    if selection == CompactionSelection::SourceRefresh {
                        return Ok((draft, candidate, false));
                    }
                    if let Some(fit) = fits(&candidate)? {
                        if fit.demand <= u64::from(fit.target) {
                            return Ok((draft, candidate, true));
                        }
                        if smallest
                            .as_ref()
                            .is_none_or(|(demand, _, _)| fit.demand < *demand)
                        {
                            smallest = Some((fit.demand, draft.clone(), candidate));
                        }
                    }
                }
                Err(SessionError::CompactionSummaryTooLarge { actual, maximum }) => {
                    summary_too_large =
                        Some(SessionError::CompactionSummaryTooLarge { actual, maximum });
                    break;
                }
                Err(error) => return Err(error.into()),
            }
            if retained <= minimum_recent {
                break;
            }
            retained = (retained / 2).max(minimum_recent);
        }
        if references == 0 {
            return smallest
                .map(|(_, draft, candidate)| (draft, candidate, false))
                .ok_or_else(|| {
                    if valid_candidate_seen {
                        ChatError::CompactionNotUseful
                    } else {
                        summary_too_large
                            .map(ChatError::Session)
                            .unwrap_or(ChatError::CompactionNotUseful)
                    }
                });
        }
        references /= 2;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ContextPressureLevel {
    Normal,
    Warning,
    Critical,
}

impl ContextPressureLevel {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Warning => "warning",
            Self::Critical => "critical",
        }
    }
}

pub(super) struct ContextPressure {
    level: ContextPressureLevel,
    estimated_demand: u64,
    window: u32,
    input_limit: u32,
    fixed_input: u32,
    reserved_output: u32,
    reserved_tool_loop: u32,
    artifact_references: usize,
    truncations: usize,
    exclusions: usize,
}

impl ContextPressure {
    pub(super) fn from_plan(plan: &ContextPlan) -> Self {
        Self::from_manifest(plan.context_decision_manifest())
    }

    fn from_manifest(manifest: &ContextDecisionManifest) -> Self {
        let mut artifact_references = 0;
        let mut truncations = 0;
        let mut exclusions = 0;
        let candidate_tokens = manifest
            .candidates()
            .iter()
            .map(|candidate| {
                match candidate.decision() {
                    ContextDecision::Include => {}
                    ContextDecision::ArtifactRef { .. } => artifact_references += 1,
                    ContextDecision::Truncate { .. } => truncations += 1,
                    ContextDecision::Exclude => exclusions += 1,
                }
                u64::from(candidate.token_estimate())
            })
            .sum::<u64>();
        let estimated_demand =
            u64::from(manifest.fixed_input_tokens()).saturating_add(candidate_tokens);
        let input_limit = manifest.budget().input_token_limit();
        let working_set = manifest.budget().working_set_input_tokens();
        let threshold_reached = estimated_demand >= u64::from(working_set);
        let level = if truncations > 0 || exclusions > 0 {
            ContextPressureLevel::Critical
        } else if artifact_references > 0 || threshold_reached {
            ContextPressureLevel::Warning
        } else {
            ContextPressureLevel::Normal
        };
        Self {
            level,
            estimated_demand,
            window: manifest.budget().context_window_tokens(),
            input_limit,
            fixed_input: manifest.fixed_input_tokens(),
            reserved_output: manifest.budget().reserved_output_tokens(),
            reserved_tool_loop: manifest.budget().reserved_tool_loop_tokens(),
            artifact_references,
            truncations,
            exclusions,
        }
    }

    pub(super) fn should_remind(&self) -> bool {
        self.level != ContextPressureLevel::Normal
    }

    pub(super) fn message(&self) -> String {
        let guidance = if self.should_remind() {
            " Consider /compact before a long next Turn."
        } else {
            " No Compaction is suggested."
        };
        format!(
            "[context pressure: {}] planner demand estimate={}/{} input tokens; window={}, output reserve={}, tool-loop reserve={}, fixed input={}; artifact_refs={}, truncations={}, exclusions={}. Provider usage is not inferred. Image token cost is unknown.{guidance}",
            self.level.as_str(),
            self.estimated_demand,
            self.input_limit,
            self.window,
            self.reserved_output,
            self.reserved_tool_loop,
            self.fixed_input,
            self.artifact_references,
            self.truncations,
            self.exclusions,
        )
    }
}
