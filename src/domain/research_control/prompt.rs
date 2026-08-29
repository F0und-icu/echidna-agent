use thiserror::Error;

use crate::domain::context::{
    ContextContentHash, ContextFragment, ContextFragmentContent, ContextFragmentId,
    ContextFragmentPriority, ContextFragmentSource, FragmentCachePolicy,
};
use crate::domain::domain_profile::DomainProfile;
use crate::domain::tools::ToolExposurePlan;

use super::{BaseContract, ResearchPhase};

/// The single fixed Prompt composition path. Domain and Phase remain
/// orthogonal values rather than a generated role combination.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct PromptComposition {
    base: BaseContract,
    personal: Option<String>,
    project: Option<String>,
    domain: DomainProfile,
    phase: ResearchPhase,
    /// Exact historical bytes used only by the fixed mechanical Replay path.
    /// It is not serializable configuration and cannot enter ordinary model
    /// sampling or introduce a second renderer.
    #[serde(skip)]
    legacy_replay_instructions: Option<String>,
}

impl Default for PromptComposition {
    fn default() -> Self {
        Self::current()
    }
}

impl PromptComposition {
    pub const fn current() -> Self {
        Self {
            base: BaseContract,
            personal: None,
            project: None,
            domain: DomainProfile::Source,
            phase: ResearchPhase::Mapping,
            legacy_replay_instructions: None,
        }
    }

    /// Recreates the one historical instruction component recorded in a
    /// legacy RequestAudit for mechanical Replay. New turns always use
    /// [`Self::current`]; this constructor is crate-private so an operator or
    /// runtime caller cannot select an arbitrary Prompt identity.
    pub(crate) fn legacy_replay(
        rendered_instructions: impl Into<String>,
    ) -> Result<Self, ResearchControlError> {
        Ok(Self {
            legacy_replay_instructions: Some(non_empty_prompt(
                rendered_instructions.into(),
                "legacy replay",
            )?),
            ..Self::current()
        })
    }

    pub fn with_personal(
        mut self,
        prompt: impl Into<String>,
    ) -> Result<Self, ResearchControlError> {
        self.personal = Some(non_empty_prompt(prompt.into(), "personal")?);
        Ok(self)
    }

    pub fn with_project(mut self, prompt: impl Into<String>) -> Result<Self, ResearchControlError> {
        self.project = Some(non_empty_prompt(prompt.into(), "project")?);
        Ok(self)
    }

    pub const fn with_domain_profile(mut self, domain: DomainProfile) -> Self {
        self.domain = domain;
        self
    }

    pub const fn with_research_phase(mut self, phase: ResearchPhase) -> Self {
        self.phase = phase;
        self
    }

    pub const fn base_contract(&self) -> BaseContract {
        self.base
    }

    pub const fn domain_profile(&self) -> DomainProfile {
        self.domain
    }

    pub const fn research_phase(&self) -> ResearchPhase {
        self.phase
    }

    pub fn tool_exposure(&self) -> ToolExposurePlan {
        ToolExposurePlan::layered([self.domain.tool_exposure(), self.phase.tool_exposure()])
    }

    pub fn fragments(&self) -> Result<Vec<ContextFragment>, ResearchControlError> {
        if let Some(rendered_instructions) = &self.legacy_replay_instructions {
            return Ok(vec![content_identified_instruction_fragment(
                "prompt:mode:legacy-replay",
                ContextFragmentSource::ModePrompt,
                ContextFragmentPriority::Required,
                rendered_instructions,
            )?]);
        }
        let mut fragments = vec![instruction_fragment(
            &format!(
                "prompt:base:{}:{}",
                BaseContract::ID,
                BaseContract::GENERATION
            ),
            ContextFragmentSource::BasePrompt,
            ContextFragmentPriority::Required,
            self.base.prompt(),
        )?];
        if let Some(personal) = &self.personal {
            fragments.push(content_identified_instruction_fragment(
                "prompt:personal",
                ContextFragmentSource::PersonalPreferences,
                ContextFragmentPriority::High,
                personal,
            )?);
        }
        if let Some(project) = &self.project {
            fragments.push(content_identified_instruction_fragment(
                "prompt:project",
                ContextFragmentSource::ProjectPrompt,
                ContextFragmentPriority::High,
                project,
            )?);
        }
        fragments.push(instruction_fragment(
            &format!(
                "prompt:domain:{}:{}",
                self.domain.id(),
                self.domain.generation()
            ),
            ContextFragmentSource::DomainPrompt,
            ContextFragmentPriority::High,
            self.domain.prompt(),
        )?);
        fragments.push(instruction_fragment(
            &format!(
                "prompt:phase:{}:{}",
                self.phase.id(),
                self.phase.generation()
            ),
            ContextFragmentSource::PhasePrompt,
            ContextFragmentPriority::High,
            self.phase.prompt(),
        )?);
        Ok(fragments)
    }
}

fn non_empty_prompt(prompt: String, source: &str) -> Result<String, ResearchControlError> {
    if prompt.trim().is_empty() {
        return Err(ResearchControlError::EmptyPrompt {
            prompt_source: source.into(),
        });
    }
    Ok(prompt)
}

fn content_identified_instruction_fragment(
    id_prefix: &str,
    source: ContextFragmentSource,
    priority: ContextFragmentPriority,
    text: &str,
) -> Result<ContextFragment, ResearchControlError> {
    let content = ContextFragmentContent::Instructions {
        text: text.to_owned(),
    };
    let content_hash = ContextContentHash::for_fragment(&content, None)?;
    instruction_fragment(
        &format!("{id_prefix}:{}", content_hash.as_str()),
        source,
        priority,
        text,
    )
}

fn instruction_fragment(
    id: &str,
    source: ContextFragmentSource,
    priority: ContextFragmentPriority,
    text: &str,
) -> Result<ContextFragment, ResearchControlError> {
    ContextFragment::new(
        ContextFragmentId::new(id)?,
        source,
        priority,
        FragmentCachePolicy::ContentAddressed,
        ContextFragmentContent::Instructions {
            text: text.to_owned(),
        },
    )
    .map_err(ResearchControlError::Context)
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ResearchControlError {
    #[error("{prompt_source} prompt must not be empty")]
    EmptyPrompt { prompt_source: String },
    #[error("ResearchPhase selection source/reason/phase combination is invalid")]
    InvalidPhaseSelection,
    #[error("ResearchPhase {phase:?} generation must be {expected}, received {received}")]
    PhaseGenerationMismatch {
        phase: ResearchPhase,
        expected: u64,
        received: u64,
    },
    #[error(transparent)]
    Context(#[from] crate::domain::context::ContextPlannerError),
}
