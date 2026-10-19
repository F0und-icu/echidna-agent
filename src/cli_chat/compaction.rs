//! Production CLI composition for explicit Session Compaction and local
//! Context-pressure presentation. Durable facts and Context decisions retain
//! their existing Domain/App owners.

use crate::adapters::project_event_store::ProjectEventStream;
use crate::adapters::research_projection::{
    CompactionValidationFacts, load_compaction_validation_facts,
};
use crate::adapters::tools::ToolRegistry;
use crate::app::agent_loop::AgentLoop;
use crate::app::session::SessionProjection;
use crate::domain::context::{ContextDecision, ContextDecisionManifest, ContextPlan};
use crate::domain::session::{
    SessionCompaction, SessionError, SessionEvent, SessionState, SessionSummaryDraft,
};
use crate::ports::EventSource;
use crate::ports::model::ModelProvider;

use super::{ChatError, DurableChatSession, event_id, occurred_at};

const MAX_REFERENCE_ITEMS_PER_KIND: usize = 32;
const WARNING_PERCENT: u64 = 80;

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
        .with_domain_profile(state.effective_domain_profile())
        .with_research_phase(state.effective_research_phase());
        preview.queue_agent_model_profile(composition.agent_profile().clone())?;
        preview.refresh_current_context(super::context::ChatContextCache::default().refresh(
            &self.project,
            tools,
            composition,
        )?)?;
        let (_, candidate) = select_compaction(state, facts, |candidate| {
            preview
                .preflight_compaction(candidate, false)
                .map_err(ChatError::from)
        })?;
        Ok(candidate)
    }

    /// Builds one candidate from strict projections, gives that candidate to
    /// the AgentLoop, appends exactly one durable Event, then applies only the
    /// freshly replayed durable Compaction. Any failure exits Chat rather than
    /// continuing with divergent runtime and persisted state.
    pub(super) fn compact<Provider>(
        &self,
        agent: &mut AgentLoop<'_, '_, Provider, ToolRegistry>,
    ) -> Result<AppliedCompactionSummary, ChatError>
    where
        Provider: ModelProvider + ?Sized,
    {
        let mut writer = self.project.open_writer()?;
        let mut store = writer.open_session(self.session_id.clone(), None)?;
        let stream_id = ProjectEventStream::Session(self.session_id.clone()).stream_id();
        let mut projection =
            SessionProjection::replay(EventSource::read_events(&store, &stream_id)?)?;

        agent.validate_compaction_session(projection.state())?;
        projection
            .state()
            .history()
            .ensure_all_tool_calls_paired()
            .map_err(crate::domain::session::SessionError::from)?;
        super::recovery::validate_runtime_history(projection.state(), agent)?;

        // The Writer lock freezes both Research and Session appenders while
        // these read-only facts and the new Compaction Event are composed.
        let facts =
            load_compaction_validation_facts(&self.project).map_err(ChatError::CompactionFacts)?;
        let (draft, compaction) = select_compaction(projection.state(), &facts, |candidate| {
            agent
                .preflight_compaction(candidate, true)
                .map_err(ChatError::from)
        })?;
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
            .append(&mut store, event)
            .map_err(ChatError::from_session_append)?;
        agent.apply_compaction(projection.state(), Some(&validation))?;
        Ok(result)
    }
}

/// One candidate/reduction owner shared by explicit compact and stale-source
/// reconstruction. Complete suffix selection remains Session/History-owned.
pub(super) fn select_compaction(
    state: &SessionState,
    facts: &CompactionValidationFacts,
    mut fits: impl FnMut(&SessionCompaction) -> Result<bool, ChatError>,
) -> Result<(SessionSummaryDraft, SessionCompaction), ChatError> {
    let history = state
        .full_normalized_history()
        .map_err(SessionError::from)?;
    let mut references = MAX_REFERENCE_ITEMS_PER_KIND;
    loop {
        let draft = facts.compose_session_summary_draft(
            &history,
            state
                .latest_compaction()
                .map(|compaction| compaction.summary()),
            references,
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
        let threshold_reached = estimated_demand.saturating_mul(100)
            >= u64::from(input_limit).saturating_mul(WARNING_PERCENT);
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
            "[context pressure: {}] last model-request planner demand estimate={}/{} input tokens; artifact_refs={}, truncations={}, exclusions={}. Provider usage is not inferred.{guidance}",
            self.level.as_str(),
            self.estimated_demand,
            self.input_limit,
            self.artifact_references,
            self.truncations,
            self.exclusions,
        )
    }
}
