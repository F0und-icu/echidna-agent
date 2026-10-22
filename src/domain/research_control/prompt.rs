use thiserror::Error;

use crate::domain::agent_definition::{AgentDefinitionBinding, InstructionSnapshot};
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
    binding: Option<AgentDefinitionBinding>,
    /// Exact historical bytes used only by the fixed mechanical Replay path.
    /// It is not serializable configuration and cannot enter ordinary model
    /// sampling or introduce a second renderer.
    #[serde(skip)]
    legacy_replay_instructions: Option<String>,
    #[serde(skip)]
    evolution_session: Option<(String, String)>,
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
            binding: None,
            legacy_replay_instructions: None,
            evolution_session: None,
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

    /// Builds the current Prompt from a durable resolved binding. The binding
    /// has already captured instruction bytes, so this path never rereads the
    /// definition or configured Personal/Project files.
    pub(crate) fn from_binding(
        binding: AgentDefinitionBinding,
    ) -> Result<Self, ResearchControlError> {
        binding.validate()?;
        let (domain, phase) = binding.definition().research().map_or(
            (DomainProfile::Source, ResearchPhase::Mapping),
            |research| (research.domain(), research.phase()),
        );
        Ok(Self {
            domain,
            phase,
            binding: Some(binding),
            ..Self::current()
        })
    }

    pub fn with_personal(
        mut self,
        prompt: impl Into<String>,
    ) -> Result<Self, ResearchControlError> {
        if self.binding.is_some() {
            return Err(ResearchControlError::BoundPromptOverride {
                prompt_source: "personal",
            });
        }
        self.personal = Some(non_empty_prompt(prompt.into(), "personal")?);
        Ok(self)
    }

    pub fn with_project(mut self, prompt: impl Into<String>) -> Result<Self, ResearchControlError> {
        if self.binding.is_some() {
            return Err(ResearchControlError::BoundPromptOverride {
                prompt_source: "project",
            });
        }
        self.project = Some(non_empty_prompt(prompt.into(), "project")?);
        Ok(self)
    }

    pub const fn with_domain_profile(mut self, domain: DomainProfile) -> Self {
        self.domain = domain;
        self
    }

    pub(crate) fn with_evolution_session(mut self, session_id: &str, pending_path: String) -> Self {
        self.evolution_session = Some((session_id.to_owned(), pending_path));
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

    /// Legacy compositions are always Research. Bound generic definitions
    /// intentionally have no synthetic DomainProfile or ResearchPhase.
    pub fn research_enabled(&self) -> bool {
        self.binding
            .as_ref()
            .is_none_or(|binding| binding.definition().research().is_some())
    }

    pub fn tool_exposure(&self) -> ToolExposurePlan {
        ToolExposurePlan::layered([self.domain.tool_exposure(), self.phase.tool_exposure()])
    }

    /// Bound generic definitions are their complete Tool layer. Bound Research
    /// definitions supply the base layer, then retain the established
    /// Domain/Phase overlays. The immutable catalog is an input so omitted
    /// definition entries become explicit Hidden overrides for every actual
    /// Tool; Hidden remains terminal in `ToolExposurePlan::layered`.
    pub(crate) fn tool_exposure_for_catalog(
        &self,
        catalog: &[crate::domain::tools::ToolSpec],
    ) -> Result<ToolExposurePlan, ResearchControlError> {
        match &self.binding {
            Some(binding) => {
                let definition = binding
                    .definition()
                    .tool_exposure_for(catalog)
                    .map_err(ResearchControlError::AgentDefinition)?;
                if binding.definition().research().is_some() {
                    Ok(ToolExposurePlan::layered([
                        definition,
                        self.domain.tool_exposure(),
                        self.phase.tool_exposure(),
                    ]))
                } else {
                    Ok(definition)
                }
            }
            None => Ok(self.tool_exposure().restricted_to_catalog(catalog)),
        }
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
        if let Some(binding) = &self.binding {
            return self.bound_fragments(binding);
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
                ContextFragmentPriority::Required,
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

    pub(crate) fn applicable_improvement(
        &self,
        task: &str,
    ) -> Result<Option<ContextFragment>, ResearchControlError> {
        use crate::domain::context::{
            AuthorityLevel, ContextOrigin, ContextProvenance, ContextReferences, ContextSlot,
            FreshnessIdentity, RetentionClass, SemanticClass,
        };
        let Some(binding) = &self.binding else {
            return Ok(None);
        };
        let Some(rsi) = binding.definition().rsi().filter(|rsi| rsi.improvement) else {
            return Ok(None);
        };
        let Some(state) = &rsi.state else {
            return Ok(None);
        };
        let task = task.to_lowercase();
        let matching = state
            .experiences
            .iter()
            .filter(|experience| task.contains(&experience.cue.to_lowercase()))
            .collect::<Vec<_>>();
        if matching.is_empty() {
            return Ok(None);
        }
        let references = matching
            .iter()
            .flat_map(|experience| experience.references.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        let text = serde_json::to_string(&serde_json::json!({"kind":"rsi_applicable_experience_v2","source_lineage":state.base_source_digest,"entries":matching,"limitation":"Derived guidance from one local reference_match pair; not current facts or general correctness. Revalidate when requested."})).expect("validated RSI experience serializes");
        let provenance = ContextProvenance::new(
            ContextSlot::RetrievedContext,
            ContextOrigin::SessionProjection,
            SemanticClass::DerivedProjection,
            AuthorityLevel::DerivedNonEvidence,
            RetentionClass::Stable,
            FreshnessIdentity::new(binding.definition().digest().as_str())?
                .with_producer_version("rsi-experience:2"),
            ContextReferences::new(Vec::new(), Vec::new(), Vec::new(), references)?,
        )?;
        Ok(Some(
            ContextFragment::new(
                ContextFragmentId::new("rsi:applicable-experience:2")?,
                ContextFragmentSource::RetrievedMemory,
                ContextFragmentPriority::High,
                FragmentCachePolicy::DoNotCache,
                ContextFragmentContent::ContentItems {
                    items: vec![crate::domain::model::ContentItem::user_text(text)],
                },
            )?
            .with_provenance(provenance)?,
        ))
    }

    fn bound_fragments(
        &self,
        binding: &AgentDefinitionBinding,
    ) -> Result<Vec<ContextFragment>, ResearchControlError> {
        const GENERIC_CORE: &str = "Follow the current operator task, preserve durable facts and runtime boundaries, and state uncertainty rather than inventing results.";

        let definition = binding.definition();
        let id = definition.id().as_str();
        let mut fragments = vec![instruction_fragment(
            "prompt:agent:00:generic-core:1",
            ContextFragmentSource::BasePrompt,
            ContextFragmentPriority::Required,
            GENERIC_CORE,
        )?];
        let (memory_scope, binding_facts) = runtime_binding_facts(binding);
        fragments.push(instruction_fragment(
            &format!("prompt:agent:00:runtime-binding:1:{id}:{memory_scope}"),
            ContextFragmentSource::BasePrompt,
            ContextFragmentPriority::Required,
            &binding_facts,
        )?);
        if let Some(rsi) = definition.rsi() {
            let (session, pending) = self
                .evolution_session
                .as_ref()
                .map(|(session, pending)| (session.as_str(), pending.as_str()))
                .unwrap_or(("definition-preflight", "<runtime-provided-pending-path>"));
            let tools = rsi
                .state
                .as_ref()
                .map(|state| state.tools.as_slice())
                .unwrap_or(&[]);
            let protocol = format!(
                "{}\nRuntime RSI binding: agent_id={id}; session_id={session}; parent_digest={}; pending_path={pending}; editable persona={}; methods={}.\nPublished ordinary script index: {}",
                include_str!("../../../assets/prompts/rsi.md"),
                definition.digest().as_str(),
                rsi.persona,
                rsi.methods,
                serde_json::to_string(tools).expect("validated script index serializes")
            );
            fragments.push(content_identified_instruction_fragment(
                "prompt:agent:00:rsi-protocol:1",
                ContextFragmentSource::BasePrompt,
                ContextFragmentPriority::Required,
                &protocol,
            )?);
            if rsi.improvement {
                fragments.push(content_identified_instruction_fragment(
                    "prompt:agent:00:rsi-improvement:2",
                    ContextFragmentSource::BasePrompt,
                    ContextFragmentPriority::Required,
                    include_str!("../../../assets/prompts/rsi-improvement-v2.md"),
                )?);
            }
            if let Some(fixed) = &rsi.fixed_rules {
                fragments.push(bound_instruction_fragment(
                    "prompt:agent:00:rsi-fixed-rules:1",
                    ContextFragmentSource::BasePrompt,
                    fixed,
                )?);
            }
        }
        fragments.push(bound_instruction_fragment(
            &format!(
                "prompt:agent:01:{id}:persona:{}",
                definition.instructions().persona().digest().as_str()
            ),
            ContextFragmentSource::BasePrompt,
            definition.instructions().persona(),
        )?);
        fragments.push(bound_instruction_fragment(
            &format!(
                "prompt:agent:02:{id}:work:{}",
                definition.instructions().work().digest().as_str()
            ),
            ContextFragmentSource::BasePrompt,
            definition.instructions().work(),
        )?);
        for (index, method) in definition.instructions().methods().iter().enumerate() {
            fragments.push(bound_instruction_fragment(
                &format!(
                    "prompt:agent:03:{index:04}:{id}:method:{}",
                    method.digest().as_str()
                ),
                ContextFragmentSource::BasePrompt,
                method,
            )?);
        }
        if let Some(personal) = binding.personal() {
            fragments.push(bound_instruction_fragment(
                &format!("prompt:agent:{id}:personal:{}", personal.digest().as_str()),
                ContextFragmentSource::PersonalPreferences,
                personal,
            )?);
        }
        if let Some(project) = binding.project() {
            fragments.push(bound_instruction_fragment(
                &format!("prompt:agent:{id}:project:{}", project.digest().as_str()),
                ContextFragmentSource::ProjectPrompt,
                project,
            )?);
        }
        if let Some(research) = definition.research() {
            let domain = research.domain_prompt(self.domain);
            fragments.push(bound_instruction_fragment(
                &format!(
                    "prompt:agent:{id}:domain:{}:{}",
                    self.domain.id(),
                    domain.digest().as_str()
                ),
                ContextFragmentSource::DomainPrompt,
                domain,
            )?);
            let phase = research.phase_prompt(self.phase);
            fragments.push(bound_instruction_fragment(
                &format!(
                    "prompt:agent:{id}:phase:{}:{}",
                    self.phase.id(),
                    phase.digest().as_str()
                ),
                ContextFragmentSource::PhasePrompt,
                phase,
            )?);
        }
        Ok(fragments)
    }
}

fn runtime_binding_facts(binding: &AgentDefinitionBinding) -> (&'static str, String) {
    let definition = binding.definition();
    let id = definition.id().as_str();
    let (scope, path_meaning) = match definition.context().memory() {
        crate::domain::agent_definition::AgentMemoryScope::None => (
            "none",
            "Logical Memory path `core.md` is unavailable because Memory is disabled.".to_owned(),
        ),
        crate::domain::agent_definition::AgentMemoryScope::Project => (
            "project",
            "Logical Memory path `core.md` is in the shared project Memory namespace.".to_owned(),
        ),
        crate::domain::agent_definition::AgentMemoryScope::Agent => (
            "agent",
            format!(
                "Logical Memory path `core.md` is in the Agent Memory namespace for Agent ID `{id}`."
            ),
        ),
    };
    (
        scope,
        format!(
            "Runtime binding facts: stable Agent ID `{id}`. Configured Memory scope: `{scope}`. {path_meaning} This scope is a logical Memory namespace, not a filesystem sandbox."
        ),
    )
}

fn bound_instruction_fragment(
    id: &str,
    source: ContextFragmentSource,
    snapshot: &InstructionSnapshot,
) -> Result<ContextFragment, ResearchControlError> {
    instruction_fragment(
        id,
        source,
        ContextFragmentPriority::Required,
        snapshot.content(),
    )
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
    #[error("a bound Agent definition does not permit a live {prompt_source} prompt override")]
    BoundPromptOverride { prompt_source: &'static str },
    #[error("ResearchPhase {phase:?} generation must be {expected}, received {received}")]
    PhaseGenerationMismatch {
        phase: ResearchPhase,
        expected: u64,
        received: u64,
    },
    #[error(transparent)]
    Context(#[from] crate::domain::context::ContextPlannerError),
    #[error(transparent)]
    AgentDefinition(#[from] crate::domain::agent_definition::AgentDefinitionError),
}
