//! Production CLI composition for explicit Session Compaction and local
//! Context-pressure presentation. Durable facts and Context decisions retain
//! their existing Domain/App owners.

use crate::adapters::research_projection::{
    CompactionValidationFacts, load_compaction_validation_facts,
};
use crate::adapters::tools::ToolRegistry;
use crate::app::agent_loop::AgentLoop;
use crate::app::session::SessionProjection;
use crate::domain::context::{
    ContextDecision, ContextDecisionManifest, ContextPlan, ContextWindowPolicy,
};
use crate::domain::session::{
    SessionCompaction, SessionError, SessionEvent, SessionState, SessionSummaryDraft,
};
use crate::ports::model::ModelProvider;

use super::{ChatError, DurableChatSession, event_id, occurred_at};

pub(super) fn automatically_deferred(error: &ChatError) -> bool {
    matches!(error,
        ChatError::CompactionNotUseful
        | ChatError::CompactionFacts(crate::adapters::research_projection::CompactionValidationFactsError::ResearchStreamEmpty)
        | ChatError::Session(SessionError::CompactionCurrentTargetRequired | SessionError::CompactionSummaryTooLarge { .. })
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

pub(super) struct CompactionRequestContext<'a> {
    pub(super) composition: &'a crate::provider_composition::ProviderComposition,
    pub(super) budget: &'a crate::app::model_request_budget::ModelRequestBudget,
    pub(super) cancellation: crate::app::cancellation::AgentCancellation,
    pub(super) audit: &'a crate::domain::request_audit::RequestAuditRunContext,
}

pub(super) enum ChatCompactionValidation {
    Legacy(Box<CompactionValidationFacts>),
    Main(Box<crate::app::session::VerifiedMainV5>),
}

impl ChatCompactionValidation {
    pub(super) fn context(&self) -> crate::domain::session::CompactionValidationContext<'_> {
        match self {
            Self::Legacy(facts) => facts.context(),
            Self::Main(facts) => facts.context(),
        }
    }
}

pub(super) struct AutomaticToolBatchCompaction<'a> {
    pub(super) session: &'a DurableChatSession,
    pub(super) policy: ContextWindowPolicy,
    pub(super) request: CompactionRequestContext<'a>,
    pub(super) deferred: bool,
    pub(super) end_answer: &'a dyn Fn(),
}

impl<Provider: ModelProvider + ?Sized>
    crate::app::agent_loop::ToolBatchStage<Provider, ToolRegistry>
    for AutomaticToolBatchCompaction<'_>
{
    fn needs_current_context(&self) -> bool {
        self.request.composition.runtime().auto_compact
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
            if !self.request.composition.runtime().auto_compact
                || self.request.cancellation.is_cancelled()
            {
                return Ok(());
            }
            let needs_compaction = agent.batch_needs_compaction(active)?;
            if self.deferred || !needs_compaction || self.request.budget.remaining() == Some(0) {
                return Ok(());
            }
            (self.end_answer)();
            let admitted_before = self.request.budget.counts().primary;
            match self
                .session
                .compact_tool_batch(
                    agent,
                    self.policy,
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
                    "Automatic tool-batch compaction applied. {} Current Turn budget retained.",
                    summary.message()
                ),
                Err(error) if automatically_deferred(&error) => {
                    if self.request.budget.counts().primary > admitted_before {
                        self.deferred = true;
                    }
                    console_println!(
                        "Automatic tool-batch compaction deferred: {error}; Provider delivery retained."
                    );
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

pub(super) struct AppliedCompactionSummary {
    revision: u64,
    source_history_items: usize,
    retained_history_items: usize,
    reached_soft_target: bool,
}

impl AppliedCompactionSummary {
    pub(super) fn message(&self) -> String {
        format!(
            "Compaction revision {} applied: retained {} of {} source History items; the next Primary request will be CompactionReset Full. {} Historical outputs/constraints may be omitted; use prompts.project_file for stable rules, not a lossy Summary.",
            self.revision,
            self.retained_history_items,
            self.source_history_items,
            if self.reached_soft_target {
                "Preferred input target reached."
            } else {
                "Preferred input target not reached; the candidate fits the hard input budget."
            }
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
    pub(super) fn refresh_main_compaction(
        &self,
        projection: &mut SessionProjection,
        store: &mut crate::adapters::project_event_store::JsonlEventStore<'_>,
        stream_id: &crate::domain::events::StreamId,
        tools: &ToolRegistry,
    ) -> Result<crate::app::session::VerifiedMainV5, ChatError> {
        use crate::domain::session::{
            CompactionValidationContext, MainV5ValidationFacts, ResearchSourceIdentity,
            SessionSummary, ValidatedResearchSource,
        };
        let old = projection.take_verified_main_v5().ok_or_else(|| {
            SessionError::CompactionContextProvenance {
                message: "main v5 replay proof is absent".to_owned(),
            }
        })?;
        let research_events =
            crate::adapters::project_event_store::read_existing_research_events(&self.project)?;
        let (catalog, receipts) = crate::adapters::research_projection::load_reference_catalog_and_receipt_ids(&self.project).map_err(|error| ChatError::CompactionFacts(crate::adapters::research_projection::CompactionValidationFactsError::Projection(error)))?;
        let research = ValidatedResearchSource::from_events(
            self.project.project_id().clone(),
            &research_events,
            &catalog,
        )?;
        if old.research_source() == research.identity() {
            return Ok(old);
        }
        let events = crate::adapters::project_event_store::read_existing_session_events(
            &self.project,
            &self.session_id,
        )?;
        let facts = MainV5ValidationFacts::from_verified_sources(
            self.project.project_id().clone(),
            &events,
            &research,
            old.graph().clone(),
        )?;
        let research_stream = match research.identity() {
            ResearchSourceIdentity::Empty { .. } => None,
            ResearchSourceIdentity::Nonempty { source } => Some(source),
        };
        let validation = CompactionValidationContext::for_main_v5(
            self.project.project_id(),
            research_stream,
            research.state(),
            &receipts,
            &facts,
        );
        let state = projection.state();
        let policy = tools.window_policy();
        let spine = if research.state().latest_target_snapshot_id().is_some() {
            let current = load_compaction_validation_facts(&self.project)
                .map_err(ChatError::CompactionFacts)?;
            Some(
                current
                    .compose_session_summary_draft(
                        &state
                            .full_normalized_history()
                            .map_err(SessionError::from)?,
                        state.latest_compaction().map(|value| value.summary()),
                        policy.compaction_reference_items(),
                    )?
                    .evidence_spine()
                    .clone(),
            )
        } else {
            None
        };
        let revision = state
            .latest_compaction()
            .expect("v5 proof belongs to a Compaction")
            .summary()
            .revision()
            .checked_add(1)
            .ok_or(SessionError::StateOverflow {
                field: "Compaction revision",
            })?;
        let summary = SessionSummary::from_verified_graph(
            revision,
            &facts,
            spine,
            state
                .latest_compaction()
                .expect("v5 proof belongs to a Compaction")
                .summary()
                .byte_limit(),
            policy.compaction_reference_items(),
            &validation,
        )?;
        // The verified graph predates any subsequently completed Turns. A
        // Research-only refresh cannot summarize or discard that active suffix.
        let retained = state
            .normalized_history_items()
            .map_err(SessionError::from)?
            .len();
        let compaction = state.next_main_compaction(summary, retained)?;
        let sequence = store.next_sequence()?;
        let event = SessionEvent::CompactionCreated { compaction }.into_envelope(
            event_id(
                self.project.project_id(),
                &self.session_id,
                sequence.get(),
                "compaction-rebuild",
            ),
            stream_id.clone(),
            sequence,
            occurred_at(),
        )?;
        projection
            .append_project_verified(
                store,
                event,
                self.project.project_id().clone(),
                events,
                &research_events,
                &catalog,
                &receipts,
                &self.artifacts,
            )
            .map_err(ChatError::from_session_append)?;
        projection.take_verified_main_v5().ok_or_else(|| {
            SessionError::CompactionContextProvenance {
                message: "refreshed main v5 replay proof is absent".to_owned(),
            }
            .into()
        })
    }

    pub(super) fn rebuild_compaction(
        &self,
        state: &SessionState,
        facts: &CompactionValidationFacts,
        composition: &crate::provider_composition::ProviderComposition,
        provider_session: crate::domain::context::ProviderSession,
        tools: &ToolRegistry,
    ) -> Result<SessionCompaction, ChatError> {
        // A context-only preflight uses the same configured Provider and Tool
        // catalog, never `stream` or a second executing AgentLoop.
        let mut preview = AgentLoop::with_primary_context(
            composition.primary_provider(),
            tools,
            composition.agent_profile().primary().model().clone(),
            composition.agent_profile().primary().profile().clone(),
            provider_session,
        )?
        .with_context_budget_policy(composition.runtime().context_budget)
        .with_domain_profile(state.effective_domain_profile())
        .with_research_phase(state.effective_research_phase());
        preview.queue_agent_model_profile(composition.agent_profile().clone())?;
        preview.refresh_current_context(super::context::ChatContextCache::default().refresh(
            &self.project,
            tools,
            composition,
        )?)?;
        let (_, candidate) = select_compaction(state, facts, tools.window_policy(), |candidate| {
            preview
                .preflight_compaction(
                    candidate,
                    false,
                    &state.protected_operator_context_for(candidate)?,
                )
                .map_err(ChatError::from)
        })?;
        Ok(candidate)
    }

    /// Builds one candidate from strict projections, gives that candidate to
    /// the AgentLoop, appends exactly one durable Event, then applies only the
    /// freshly replayed durable Compaction. Any failure exits Chat rather than
    /// continuing with divergent runtime and persisted state.
    pub(super) async fn compact<Provider>(
        &self,
        agent: &mut AgentLoop<'_, '_, Provider, ToolRegistry>,
        policy: ContextWindowPolicy,
        request: CompactionRequestContext<'_>,
    ) -> Result<AppliedCompactionSummary, ChatError>
    where
        Provider: ModelProvider + ?Sized,
    {
        self.compact_with_boundary(agent, policy, None, request)
            .await
    }

    pub(super) async fn compact_tool_batch<Provider>(
        &self,
        agent: &mut AgentLoop<'_, '_, Provider, ToolRegistry>,
        policy: ContextWindowPolicy,
        active: &crate::app::agent_loop::ToolBatchContext<'_>,
        request: CompactionRequestContext<'_>,
    ) -> Result<AppliedCompactionSummary, ChatError>
    where
        Provider: ModelProvider + ?Sized,
    {
        self.compact_with_boundary(agent, policy, Some(active), request)
            .await
    }

    async fn compact_with_boundary<Provider>(
        &self,
        agent: &mut AgentLoop<'_, '_, Provider, ToolRegistry>,
        policy: ContextWindowPolicy,
        active: Option<&crate::app::agent_loop::ToolBatchContext<'_>>,
        request: CompactionRequestContext<'_>,
    ) -> Result<AppliedCompactionSummary, ChatError>
    where
        Provider: ModelProvider + ?Sized,
    {
        use crate::domain::session::{
            CompactionSourcePlan, SessionSourceIdentity, ValidatedResearchSource,
        };
        use crate::ports::artifact::ArtifactStore;
        let capture = agent.capture_compaction_context(
            request.composition.compaction_provider().0,
            policy.compaction_reference_items(),
        )?;
        let capture_bytes = serde_json::to_vec(&capture).map_err(|error| {
            SessionError::CompactionSourceInvalid {
                message: error.to_string(),
            }
        })?;
        let (source, research, goal, references, entries, contents, output_limit) = self.with_projection(|projection, store, stream_id| {
            let state = projection.state();
            agent.validate_compaction_session(state)?;
            state.ensure_completed_history().map_err(SessionError::from)?;
            super::recovery::validate_runtime_history(state, agent)?;
            if active.is_none() {
                let protected = state.minimum_protected_operator_context()?;
                if !protected.detached_items.is_empty() && !agent.minimum_compaction_context_fits(&protected)? {
                    return Err(ChatError::CompactionNotUseful);
                }
            }
            let history = state.full_normalized_history().map_err(SessionError::from)?;
            let goal = if !state.protected_operator_inputs()?.is_empty() {
                "The earlier operator request and constraints are retained verbatim in this context.".to_owned()
            } else {
                crate::adapters::research_projection::continuation_goal(&history, state.latest_compaction().map(|value| value.summary()), policy)?
            };
            let output_limit = crate::domain::session::SessionSummary::handoff_byte_limit(&goal, policy.summary_bytes(), crate::app::agent_loop::minimum_compaction_handoff_bytes())?;
            let blob = self.artifacts.publish_bytes(&capture_bytes).map_err(|error| SessionError::CompactionContextProvenance { message: format!("Compaction context publication failed: {error}") })?.blob_id;
            let sequence = store.next_sequence()?;
            let event = SessionEvent::CompactionContextCaptured { blob_id: blob }.into_envelope(event_id(self.project.project_id(), &self.session_id, sequence.get(), "compaction-context"), stream_id, sequence, occurred_at())?;
            projection.append(store, event).map_err(ChatError::from_session_append)?;
            let events = crate::adapters::project_event_store::read_existing_session_events(&self.project, &self.session_id)?;
            let research_events = crate::adapters::project_event_store::read_existing_research_events(&self.project)?;
            let catalog = crate::adapters::research_projection::load_reference_catalog(&self.project).map_err(|error| ChatError::CompactionFacts(crate::adapters::research_projection::CompactionValidationFactsError::Projection(error)))?;
            let research = ValidatedResearchSource::from_events(self.project.project_id().clone(), &research_events, &catalog)?;
            let (source, state) = SessionSourceIdentity::from_events(self.project.project_id().clone(), &events)?;
            let references = state.history_items_with_event_ids().map(|(id, _)| id.clone()).rev().take(policy.compaction_reference_items()).collect::<Vec<_>>();
            let (entries, contents) = CompactionSourcePlan::source_entries(&source, &research, &events, &research_events)?;
            Ok((source, research, goal, references, entries, contents, output_limit))
        })?;
        let graph = self
            .sample_compaction_graph(
                agent,
                &request,
                &capture,
                &source,
                &research,
                &goal,
                &references,
                &entries,
                &contents,
                policy.summary_bytes(),
                output_limit,
            )
            .await?;
        if request.cancellation.is_cancelled() {
            return Err(crate::app::agent_loop::AgentLoopError::CompactionCancelled.into());
        }
        self.with_projection(|projection, store, stream_id| {
            let events = crate::adapters::project_event_store::read_existing_session_events(&self.project, &self.session_id)?;
            let research_events = crate::adapters::project_event_store::read_existing_research_events(&self.project)?;
            let (catalog, receipts) = crate::adapters::research_projection::load_reference_catalog_and_receipt_ids(&self.project).map_err(|error| ChatError::CompactionFacts(crate::adapters::research_projection::CompactionValidationFactsError::Projection(error)))?;
            let current = ValidatedResearchSource::from_events(self.project.project_id().clone(), &research_events, &catalog)?;
            source.validate_graph_sampling_suffix(&current, &events, &research_events, &graph.root_event_id)?;
            crate::app::session::verify_compaction_source_blobs(&graph, &self.artifacts)?;
            super::recovery::validate_runtime_history(projection.state(), agent)?;
            let facts = crate::domain::session::MainV5ValidationFacts::from_verified_sources(self.project.project_id().clone(), &events, &current, graph)?;
            let research_stream = match current.identity() { crate::domain::session::ResearchSourceIdentity::Empty { .. } => None, crate::domain::session::ResearchSourceIdentity::Nonempty { source } => Some(source) };
            let validation = crate::domain::session::CompactionValidationContext::for_main_v5(self.project.project_id(), research_stream, current.state(), &receipts, &facts);
            let spine = if current.state().latest_target_snapshot_id().is_some() {
                let research_facts = load_compaction_validation_facts(&self.project).map_err(ChatError::CompactionFacts)?;
                Some(research_facts.compose_session_summary_draft(&projection.state().full_normalized_history().map_err(SessionError::from)?, projection.state().latest_compaction().map(|item| item.summary()), policy.compaction_reference_items())?.evidence_spine().clone())
            } else { None };
            let revision = projection.state().latest_compaction().map_or(Ok(1), |value| value.summary().revision().checked_add(1).ok_or(SessionError::StateOverflow { field: "Compaction revision" }))?;
            let summary = crate::domain::session::SessionSummary::from_verified_graph(revision, &facts, spine, policy.summary_bytes(), policy.compaction_reference_items(), &validation)?;
            let mut retained = projection.state().normalized_history_items().map_err(SessionError::from)?.len();
            let minimum = if active.is_some() { projection.state().latest_uncompleted_batch_items()? } else { 0 };
            let mut best = None;
            let (compaction, reached_soft_target) = loop {
                let candidate = projection.state().next_main_compaction(summary.clone(), retained)?;
                let protected = projection.state().protected_operator_context_for(&candidate)?;
                let fit = match active {
                    Some(active) => agent.preflight_tool_batch_compaction(&candidate, active, &protected)?,
                    None => agent.preflight_compaction_fit(&candidate, true, &protected)?,
                };
                if let Some(fit) = fit && candidate.retained_history().len() >= minimum {
                    if fit.demand <= u64::from(fit.target) { break (candidate, true); }
                    if best.as_ref().is_none_or(|(_, demand)| fit.demand < *demand) { best = Some((candidate, fit.demand)); }
                }
                if retained <= minimum { break (best.ok_or(ChatError::CompactionNotUseful)?.0, false); }
                retained = (retained / 2).max(minimum);
            };
            let result = AppliedCompactionSummary { revision, source_history_items: compaction.source_history_item_count(), retained_history_items: compaction.retained_history().len(), reached_soft_target };
            let sequence = store.next_sequence()?;
            let event = SessionEvent::CompactionCreated { compaction }.into_envelope(event_id(self.project.project_id(), &self.session_id, sequence.get(), "compaction"), stream_id, sequence, occurred_at())?;
            projection.append_project_verified(store, event, self.project.project_id().clone(), events, &research_events, &catalog, &receipts, &self.artifacts).map_err(ChatError::from_session_append)?;
            agent.apply_compaction(projection.state(), Some(&validation))?;
            Ok(result)
        })
    }
}

/// One candidate/reduction owner shared by explicit compact and stale-source
/// reconstruction. Complete suffix selection remains Session/History-owned.
pub(super) fn select_compaction(
    state: &SessionState,
    facts: &CompactionValidationFacts,
    policy: ContextWindowPolicy,
    mut fits: impl FnMut(&SessionCompaction) -> Result<bool, ChatError>,
) -> Result<(SessionSummaryDraft, SessionCompaction), ChatError> {
    let history = state
        .full_normalized_history()
        .map_err(SessionError::from)?;
    let mut references = policy.compaction_reference_items();
    loop {
        let draft = facts
            .compose_session_summary_draft(
                &history,
                state
                    .latest_compaction()
                    .map(|compaction| compaction.summary()),
                references,
            )?
            .with_byte_limit(
                state
                    .latest_compaction()
                    .map_or(policy.summary_bytes(), |value| value.summary().byte_limit()),
            )?;
        let mut retained = state
            .normalized_history_items()
            .map_err(SessionError::from)?
            .len();
        loop {
            match state.next_compaction(draft.clone(), retained, &facts.context()) {
                Ok(candidate) if fits(&candidate)? => return Ok((draft, candidate)),
                Ok(_) => {}
                Err(SessionError::CompactionSummaryTooLarge { .. }) => break,
                Err(error) => return Err(error.into()),
            }
            if retained == 0 {
                break;
            }
            retained /= 2;
        }
        if references == 0 {
            return Err(ChatError::CompactionNotUseful);
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
    input_limit: u32,
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
        let threshold_reached =
            estimated_demand >= u64::from(manifest.budget().working_set_input_tokens());
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
            input_limit,
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
            "[context pressure: {}] planner demand estimate={}/{} input tokens; artifact_refs={}, truncations={}, exclusions={}. Provider usage is not inferred.{guidance}",
            self.level.as_str(),
            self.estimated_demand,
            self.input_limit,
            self.artifact_references,
            self.truncations,
            self.exclusions,
        )
    }
}
