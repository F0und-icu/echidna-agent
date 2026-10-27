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
            if self.batches.is_multiple_of(16)
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
                    if let Some(binding) = updated {
                        self.binding = binding;
                    }
                    return Ok(());
                }
            }
            if self.deferred
                || self.request.cancellation.is_cancelled()
                || !agent.delivery_needs_compaction()?
            {
                return Ok(());
            }
            (self.end_answer)();
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
                    self.deferred = true;
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

pub(super) fn automatically_deferred(error: &ChatError) -> bool {
    matches!(error,
        ChatError::CompactionNotUseful
        | ChatError::CompactionFacts(crate::adapters::research_projection::CompactionValidationFactsError::ResearchStreamEmpty)
        | ChatError::Session(SessionError::CompactionCurrentTargetRequired)
        | ChatError::Agent(crate::app::agent_loop::AgentLoopError::CompactionSummaryRejected { .. }
            | crate::app::agent_loop::AgentLoopError::CompactionCancelled
            | crate::app::agent_loop::AgentLoopError::Model(_)
            | crate::app::agent_loop::AgentLoopError::ModelRequestLimit { .. }
            | crate::app::agent_loop::AgentLoopError::ContextPlanner(crate::domain::context::ContextPlannerError::RequiredFragmentsExceedBudget { .. }
                | crate::domain::context::ContextPlannerError::RenderedInputExceedsBudget { .. }))
    )
}

pub(super) fn explicitly_deferred(error: &ChatError) -> bool {
    matches!(error, ChatError::Agent(_)) && automatically_deferred(error)
}

pub(super) struct AppliedCompactionSummary {
    revision: u64,
    source_history_items: usize,
    retained_history_items: usize,
}

impl AppliedCompactionSummary {
    pub(super) fn message(&self) -> String {
        format!(
            "Compaction revision {} applied: retained {} of {} source History items; the next Primary request will be CompactionReset Full. Historical outputs/constraints may be omitted; use prompts.project_file for stable rules, not a lossy Summary.",
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
        provider_session: crate::domain::context::ProviderSession,
        discovered_mcp: &[crate::adapters::mcp::McpCatalogTool],
        read: &crate::adapters::research_projection::ProjectReadContext,
    ) -> Result<SessionCompaction, ChatError> {
        // A context-only preflight uses the same configured Provider and Tool
        // catalog, never `stream` or a second executing AgentLoop.
        let window_tokens = composition
            .agent_profile()
            .primary()
            .capabilities()
            .context_window;
        let tools = self.bound_tools(discovered_mcp, window_tokens)?;
        let mut preview = AgentLoop::with_primary_context(
            composition.primary_provider(),
            &tools,
            composition.agent_profile().primary().model().clone(),
            composition.agent_profile().primary().profile().clone(),
            provider_session,
        )?
        .with_prompt_composition(super::definition::prompt_for_state(
            &self.project,
            self.agent_binding(),
            state,
            self.session_id.as_str(),
        )?);
        preview.queue_agent_model_profile(composition.agent_profile().clone())?;
        preview.refresh_current_context(
            super::context::ChatContextCache::default().refresh_from_read_context(
                &self.project,
                &tools,
                self.agent_binding(),
                read,
            )?,
        )?;
        let (_, candidate) = select_compaction(
            state,
            facts,
            ContextWindowPolicy::for_window(window_tokens),
            state
                .latest_compaction()
                .and_then(|compaction| compaction.summary().narrative()),
            |candidate| {
                preview
                    .preflight_compaction(candidate, false)
                    .map_err(ChatError::from)
            },
        )?;
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
        window_tokens: u32,
        active: Option<&crate::app::agent_loop::ToolBatchContext<'_>>,
        replacement: Option<&crate::domain::research_control::PromptComposition>,
        request: CompactionRequestContext<'_>,
    ) -> Result<AppliedCompactionSummary, ChatError>
    where
        Provider: ModelProvider + ?Sized,
    {
        let policy = ContextWindowPolicy::for_window(window_tokens);
        let (source, goal, source_data, references) =
            self.with_projection(|projection, _, _| {
                let state = projection.state();
                agent.validate_compaction_session(state)?;
                state
                    .ensure_completed_history()
                    .map_err(SessionError::from)?;
                super::recovery::validate_runtime_history(state, agent)?;
                let facts = load_session_compaction_facts(&self.project, state)
                    .map_err(ChatError::CompactionFacts)?;
                // Do not spend a model request when even the original local
                // capacity/retention preflight proves no useful replacement fits.
                select_compaction(state, &facts, policy, None, |candidate| {
                    match (active, replacement) {
                        (Some(active), Some(prompt)) => {
                            agent.preflight_checkpoint_definition(candidate, active, prompt)
                        }
                        (Some(active), None) => {
                            agent.preflight_tool_batch_compaction(candidate, active)
                        }
                        (None, _) => agent.preflight_compaction(candidate, true),
                    }
                    .map_err(ChatError::from)
                })?;
                let history = state
                    .normalized_history_items()
                    .map_err(SessionError::from)?;
                let previous = state.latest_compaction().map(|value| value.summary());
                let (goal, _) = crate::adapters::research_projection::continuation_text(
                    &state
                        .full_normalized_history()
                        .map_err(SessionError::from)?,
                    previous,
                    policy,
                )?;
                let references = state
                    .history_items_with_event_ids()
                    .map(|(id, _)| id.clone())
                    .rev()
                    .take(policy.compaction_reference_items())
                    .collect::<Vec<_>>();
                let previous = previous
                    .map(|summary| summary.model_projection())
                    .transpose()?;
                let data = serde_json::to_string(&serde_json::json!({
                    "source": facts.task_source()?,
                    "operator_goal": goal,
                    "active_operator_input": active.map(|active| active.original_input()),
                    "previous_derived_summary": previous,
                    "active_normalized_history": history,
                    "recent_history_event_refs": references,
                }))
                .map_err(|error| ChatError::CompactionSummarySource {
                    message: error.to_string(),
                })?;
                Ok((facts.task_source()?.clone(), goal, data, references))
            })?;
        console_eprintln!(
            "[compaction] Requesting a tool-free model handoff; original Context remains active until validation and durable append."
        );
        let mut attempts = Vec::new();
        let mut record_attempt = |attempt| {
            self.record_request_audit_manifest(attempt, None)
                .map(|manifest| {
                    let id = manifest.audit_event_id().clone();
                    attempts.push(id.clone());
                    id
                })
                .map_err(ChatError::into_request_audit_agent_loop_error)
        };
        let output = agent
            .sample_compaction(
                crate::app::agent_loop::CompactionSampleInput {
                    provider: request.composition.compaction_provider().0,
                    agent_model_profile: Some(request.composition.compaction_provider().1),
                    source: &source,
                    source_data,
                    current_goal: goal,
                    references,
                    byte_limit: policy.summary_bytes(),
                    request_budget: request.budget,
                    cancellation: request.cancellation.clone(),
                    audit_context: request.audit,
                },
                &mut record_attempt,
            )
            .await?;
        if request.cancellation.is_cancelled() {
            return Err(crate::app::agent_loop::AgentLoopError::CompactionCancelled.into());
        }
        let sample_id = self.with_projection(|projection, store, stream_id| {
            crate::adapters::session_compaction::validate_compaction_source_suffix(
                &self.project,
                &source,
                &attempts,
                None,
            )
            .map_err(|message| ChatError::CompactionSummarySource { message })?;
            let sequence = store.next_sequence()?;
            let id = event_id(
                self.project.project_id(),
                &self.session_id,
                sequence.get(),
                "compaction-sample",
            );
            let event = SessionEvent::CompactionSampleRecorded {
                sample: output.sample.clone(),
            }
            .into_envelope(id.clone(), stream_id, sequence, occurred_at())?;
            projection
                .append(store, event)
                .map_err(ChatError::from_session_append)?;
            Ok(id)
        })?;
        if let Some(usage) = output.usage {
            self.record_model_usage(
                output.sample.audit_event_id(),
                output.sample.provider_session(),
                usage,
            )?;
        }
        self.with_projection(|projection, store, stream_id| {
            crate::adapters::session_compaction::validate_compaction_source_suffix(
                &self.project,
                &source,
                &attempts,
                Some(&sample_id),
            )
            .map_err(|message| ChatError::CompactionSummarySource { message })?;
            agent.validate_compaction_session(projection.state())?;
            projection
                .state()
                .ensure_completed_history()
                .map_err(crate::domain::session::SessionError::from)?;
            super::recovery::validate_runtime_history(projection.state(), agent)?;

            // The Writer lock freezes both Research and Session appenders while
            // these read-only facts and the new Compaction Event are composed.
            let facts = load_session_compaction_facts(&self.project, projection.state())
                .map_err(ChatError::CompactionFacts)?
                .with_model_sample(sample_id, output.sample.narrative().clone());
            let (draft, compaction) = select_compaction(
                projection.state(),
                &facts,
                ContextWindowPolicy::for_window(window_tokens),
                None,
                |candidate| {
                    match (active, replacement) {
                        (Some(active), Some(prompt)) => {
                            agent.preflight_checkpoint_definition(candidate, active, prompt)
                        }
                        (Some(active), None) => {
                            agent.preflight_tool_batch_compaction(candidate, active)
                        }
                        (None, _) => agent.preflight_compaction(candidate, true),
                    }
                    .map_err(ChatError::from)
                },
            )?;
            let validation = facts.context();
            let result = AppliedCompactionSummary {
                revision: compaction.summary().revision(),
                source_history_items: compaction.source_history_item_count(),
                retained_history_items: compaction.retained_history().len(),
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
pub(super) fn select_compaction(
    state: &SessionState,
    facts: &SessionCompactionFacts,
    policy: ContextWindowPolicy,
    historical_narrative: Option<&crate::domain::session::NarrativeSummary>,
    mut fits: impl FnMut(&SessionCompaction) -> Result<bool, ChatError>,
) -> Result<(SessionSummaryDraft, SessionCompaction), ChatError> {
    let history = state
        .full_normalized_history()
        .map_err(SessionError::from)?;
    let minimum_recent = crate::domain::history::latest_complete_tool_batch_items(&history)
        .map_err(SessionError::from)?;
    let mut references = policy.compaction_reference_items();
    loop {
        let mut draft = facts
            .compose_session_summary_draft(
                &history,
                state
                    .latest_compaction()
                    .map(|compaction| compaction.summary()),
                references,
                policy,
            )?
            .with_byte_limit(policy.summary_bytes())?;
        if let Some(narrative) = historical_narrative {
            draft = draft.with_historical_narrative(narrative.clone());
        }
        let mut retained = state
            .normalized_history_items()
            .map_err(SessionError::from)?
            .len()
            .max(minimum_recent);
        loop {
            match state.next_compaction(draft.clone(), retained, &facts.context()) {
                Ok(candidate) if fits(&candidate)? => return Ok((draft, candidate)),
                Ok(_) => {}
                Err(SessionError::CompactionSummaryTooLarge { .. }) => break,
                Err(error) => return Err(error.into()),
            }
            if retained <= minimum_recent {
                break;
            }
            retained = (retained / 2).max(minimum_recent);
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
