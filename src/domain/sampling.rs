//! Immutable per-Turn configuration and per-request sampling authority.

use thiserror::Error;

use crate::domain::context::{ContextPlan, PromptRendererIdentity, TurnContext};
use crate::domain::domain_profile::DomainProfile;
use crate::domain::model::{ModelCapabilities, ModelRequestRequirements};
use crate::domain::profile::AgentModelProfile;
use crate::domain::research_control::ResearchPhase;
use crate::domain::tools::ResolvedToolPlan;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TurnConfigurationSnapshot {
    turn: TurnContext,
    domain_profile: DomainProfile,
    research_phase: ResearchPhase,
    agent_model_profile: Option<AgentModelProfile>,
}

impl TurnConfigurationSnapshot {
    pub fn new(
        turn: TurnContext,
        domain_profile: DomainProfile,
        research_phase: ResearchPhase,
        agent_model_profile: Option<AgentModelProfile>,
    ) -> Self {
        Self {
            turn,
            domain_profile,
            research_phase,
            agent_model_profile,
        }
    }

    pub fn turn(&self) -> &TurnContext {
        &self.turn
    }

    pub fn agent_model_profile(&self) -> Option<&AgentModelProfile> {
        self.agent_model_profile.as_ref()
    }

    pub const fn domain_profile(&self) -> DomainProfile {
        self.domain_profile
    }

    pub const fn research_phase(&self) -> ResearchPhase {
        self.research_phase
    }
}

/// The single immutable semantic authority captured before one Provider
/// request. Context, advertised Tool schemas, audit and later dispatch all
/// consume this exact value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SamplingStepPlan {
    step_id: String,
    turn_configuration: TurnConfigurationSnapshot,
    context_plan: ContextPlan,
    resolved_tool_plan: ResolvedToolPlan,
    capabilities: ModelCapabilities,
    requirements: ModelRequestRequirements,
    prompt_renderer: PromptRendererIdentity,
}

impl SamplingStepPlan {
    pub fn new(
        step_id: impl Into<String>,
        turn_configuration: TurnConfigurationSnapshot,
        context_plan: ContextPlan,
        resolved_tool_plan: ResolvedToolPlan,
        capabilities: ModelCapabilities,
    ) -> Result<Self, SamplingPlanError> {
        let step_id = step_id.into();
        if step_id.trim().is_empty() {
            return Err(SamplingPlanError::StepIdRequired);
        }
        if context_plan.tool_specs() != resolved_tool_plan.advertised_specs() {
            return Err(SamplingPlanError::ContextToolPlanMismatch);
        }
        let requirements = context_plan.requirements();
        let prompt_renderer = context_plan.prompt_renderer().clone();
        Ok(Self {
            step_id,
            turn_configuration,
            context_plan,
            resolved_tool_plan,
            capabilities,
            requirements,
            prompt_renderer,
        })
    }

    pub fn step_id(&self) -> &str {
        &self.step_id
    }

    pub fn turn_configuration(&self) -> &TurnConfigurationSnapshot {
        &self.turn_configuration
    }

    pub fn context_plan(&self) -> &ContextPlan {
        &self.context_plan
    }

    pub fn resolved_tool_plan(&self) -> &ResolvedToolPlan {
        &self.resolved_tool_plan
    }

    pub fn capabilities(&self) -> &ModelCapabilities {
        &self.capabilities
    }

    pub const fn requirements(&self) -> ModelRequestRequirements {
        self.requirements
    }

    pub fn prompt_renderer(&self) -> &PromptRendererIdentity {
        &self.prompt_renderer
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum SamplingPlanError {
    #[error("Sampling Step ID must not be empty")]
    StepIdRequired,
    #[error("Context ToolSpecs do not match the ResolvedToolPlan")]
    ContextToolPlanMismatch,
}
