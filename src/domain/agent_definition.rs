//! Immutable, content-addressed Agent definitions and durable Session bindings.
//!
//! The filesystem loader owns TOML and Markdown I/O. This module owns the
//! typed resolved snapshot, its semantic identity, and the compatibility
//! checks used by Session transitions.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::domain::canonical_json::canonical_blake3_hash;
use crate::domain::domain_profile::DomainProfile;
use crate::domain::research_control::ResearchPhase;
use crate::domain::tools::{ToolExposure, ToolExposurePlan, ToolName, ToolSpec};

pub const AGENT_DEFINITION_SCHEMA_VERSION: u16 = 1;
pub(crate) const MAX_AGENT_RESOURCE_BYTES: usize = 64 * 1024;
pub(crate) const MAX_AGENT_INSTRUCTION_BYTES: usize = 256 * 1024;
pub(crate) const MAX_AGENT_METHOD_FILES: usize = 16;

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct AgentId(String);

impl AgentId {
    pub fn new(value: impl Into<String>) -> Result<Self, AgentDefinitionError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 64
            || !value.bytes().enumerate().all(|(index, byte)| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'_' | b'-')
                    || (index > 0 && byte == b'.')
            })
        {
            return Err(AgentDefinitionError::InvalidAgentId { value });
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn validate(&self) -> Result<(), AgentDefinitionError> {
        Self::new(self.0.clone()).map(|_| ())
    }
}

impl fmt::Display for AgentId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct AgentDefinitionDigest(String);

impl AgentDefinitionDigest {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn from_hash(value: String) -> Self {
        Self(value)
    }

    fn validate(&self, field: &'static str) -> Result<(), AgentDefinitionError> {
        if is_blake3_digest(&self.0) {
            Ok(())
        } else {
            Err(AgentDefinitionError::InvalidDigest {
                field,
                value: self.0.clone(),
            })
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct AgentContentDigest(String);

impl AgentContentDigest {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn for_content(content: &str) -> Self {
        Self(blake3::hash(content.as_bytes()).to_hex().to_string())
    }

    fn validate(&self) -> Result<(), AgentDefinitionError> {
        if is_blake3_digest(&self.0) {
            Ok(())
        } else {
            Err(AgentDefinitionError::InvalidDigest {
                field: "content_digest",
                value: self.0.clone(),
            })
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BuiltinAgentTemplate {
    Research,
    TechnicalWriting,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DefinitionSourceLocator {
    Local { manifest_path: PathBuf },
    Builtin { template: BuiltinAgentTemplate },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum InstructionSourceLocator {
    Local {
        path: PathBuf,
    },
    Builtin {
        template: BuiltinAgentTemplate,
        resource: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InstructionSnapshot {
    content: String,
    content_digest: AgentContentDigest,
    source_locator: InstructionSourceLocator,
}

impl InstructionSnapshot {
    pub(crate) fn loaded(
        content: String,
        source_locator: InstructionSourceLocator,
    ) -> Result<Self, AgentDefinitionError> {
        validate_instruction_content(&content, "instruction resource")?;
        Ok(Self {
            content_digest: AgentContentDigest::for_content(&content),
            content,
            source_locator,
        })
    }

    pub fn content(&self) -> &str {
        &self.content
    }

    pub fn digest(&self) -> &AgentContentDigest {
        &self.content_digest
    }

    fn validate(&self) -> Result<(), AgentDefinitionError> {
        validate_instruction_content(&self.content, "instruction resource")?;
        self.content_digest.validate()?;
        let actual = AgentContentDigest::for_content(&self.content);
        if actual != self.content_digest {
            return Err(AgentDefinitionError::ContentDigestMismatch {
                expected: self.content_digest.0.clone(),
                actual: actual.0,
            });
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentInstructions {
    persona: InstructionSnapshot,
    work: InstructionSnapshot,
    methods: Vec<InstructionSnapshot>,
}

impl AgentInstructions {
    pub(crate) fn new(
        persona: InstructionSnapshot,
        work: InstructionSnapshot,
        methods: Vec<InstructionSnapshot>,
    ) -> Result<Self, AgentDefinitionError> {
        let value = Self {
            persona,
            work,
            methods,
        };
        value.validate()?;
        Ok(value)
    }

    pub fn persona(&self) -> &InstructionSnapshot {
        &self.persona
    }

    pub fn work(&self) -> &InstructionSnapshot {
        &self.work
    }

    pub fn methods(&self) -> &[InstructionSnapshot] {
        &self.methods
    }

    fn validate(&self) -> Result<(), AgentDefinitionError> {
        self.persona.validate()?;
        self.work.validate()?;
        if self.methods.len() > MAX_AGENT_METHOD_FILES {
            return Err(AgentDefinitionError::TooManyMethodFiles {
                actual: self.methods.len(),
                maximum: MAX_AGENT_METHOD_FILES,
            });
        }
        for method in &self.methods {
            method.validate()?;
        }
        let total = self.persona.content.len()
            + self.work.content.len()
            + self
                .methods
                .iter()
                .map(|method| method.content.len())
                .sum::<usize>();
        if total > MAX_AGENT_INSTRUCTION_BYTES {
            return Err(AgentDefinitionError::InstructionBudgetExceeded {
                actual: total,
                maximum: MAX_AGENT_INSTRUCTION_BYTES,
            });
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentConfiguredContext {
    None,
    Configured,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentMemoryScope {
    None,
    Project,
    Agent,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentContextSelection {
    personal: AgentConfiguredContext,
    project: AgentConfiguredContext,
    memory: AgentMemoryScope,
}

impl AgentContextSelection {
    pub(crate) const fn new(
        personal: AgentConfiguredContext,
        project: AgentConfiguredContext,
        memory: AgentMemoryScope,
    ) -> Self {
        Self {
            personal,
            project,
            memory,
        }
    }

    pub const fn personal(&self) -> AgentConfiguredContext {
        self.personal
    }

    pub const fn project(&self) -> AgentConfiguredContext {
        self.project
    }

    pub const fn memory(&self) -> AgentMemoryScope {
        self.memory
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentToolExposure {
    exposures: BTreeMap<ToolName, ToolExposure>,
}

impl AgentToolExposure {
    pub(crate) fn new(exposures: BTreeMap<ToolName, ToolExposure>) -> Self {
        Self { exposures }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentResearchDefinition {
    domain: DomainProfile,
    phase: ResearchPhase,
    domain_prompts: BTreeMap<DomainProfile, InstructionSnapshot>,
    phase_prompts: BTreeMap<ResearchPhase, InstructionSnapshot>,
}

impl AgentResearchDefinition {
    pub(crate) fn new(
        domain: DomainProfile,
        phase: ResearchPhase,
        domain_prompts: BTreeMap<DomainProfile, InstructionSnapshot>,
        phase_prompts: BTreeMap<ResearchPhase, InstructionSnapshot>,
    ) -> Result<Self, AgentDefinitionError> {
        let value = Self {
            domain,
            phase,
            domain_prompts,
            phase_prompts,
        };
        value.validate()?;
        Ok(value)
    }

    pub const fn domain(&self) -> DomainProfile {
        self.domain
    }

    pub const fn phase(&self) -> ResearchPhase {
        self.phase
    }

    pub fn domain_prompt(&self, profile: DomainProfile) -> &InstructionSnapshot {
        self.domain_prompts
            .get(&profile)
            .expect("validated AgentDefinition contains every DomainProfile prompt")
    }

    pub fn phase_prompt(&self, phase: ResearchPhase) -> &InstructionSnapshot {
        self.phase_prompts
            .get(&phase)
            .expect("validated AgentDefinition contains every ResearchPhase prompt")
    }

    fn validate(&self) -> Result<(), AgentDefinitionError> {
        for profile in DomainProfile::ALL {
            self.domain_prompts
                .get(&profile)
                .ok_or(AgentDefinitionError::MissingResearchPrompt {
                    kind: "domain",
                    id: profile.id(),
                })?
                .validate()?;
        }
        for phase in ResearchPhase::ALL {
            self.phase_prompts
                .get(&phase)
                .ok_or(AgentDefinitionError::MissingResearchPrompt {
                    kind: "phase",
                    id: phase.id(),
                })?
                .validate()?;
        }
        Ok(())
    }

    fn compatibility_key(&self) -> (DomainProfile, ResearchPhase) {
        (self.domain, self.phase)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentDefinition {
    schema_version: u16,
    id: AgentId,
    name: String,
    digest: AgentDefinitionDigest,
    instructions: AgentInstructions,
    context: AgentContextSelection,
    tools: AgentToolExposure,
    research: Option<AgentResearchDefinition>,
    source_locator: DefinitionSourceLocator,
}

impl AgentDefinition {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn loaded(
        source_locator: DefinitionSourceLocator,
        id: AgentId,
        name: String,
        instructions: AgentInstructions,
        context: AgentContextSelection,
        tools: AgentToolExposure,
        research: Option<AgentResearchDefinition>,
    ) -> Result<Self, AgentDefinitionError> {
        let mut value = Self {
            schema_version: AGENT_DEFINITION_SCHEMA_VERSION,
            id,
            name,
            digest: AgentDefinitionDigest::from_hash(String::new()),
            instructions,
            context,
            tools,
            research,
            source_locator,
        };
        value.digest = value.expected_digest()?;
        value.validate()?;
        Ok(value)
    }

    pub fn id(&self) -> &AgentId {
        &self.id
    }

    pub fn digest(&self) -> &AgentDefinitionDigest {
        &self.digest
    }

    pub fn instructions(&self) -> &AgentInstructions {
        &self.instructions
    }

    pub fn context(&self) -> &AgentContextSelection {
        &self.context
    }

    pub fn research(&self) -> Option<&AgentResearchDefinition> {
        self.research.as_ref()
    }

    pub fn source_locator(&self) -> &DefinitionSourceLocator {
        &self.source_locator
    }

    pub fn tool_exposure_for(
        &self,
        catalog: &[ToolSpec],
    ) -> Result<ToolExposurePlan, AgentDefinitionError> {
        self.validate()?;
        let names = catalog_names(catalog)?;
        for (name, exposure) in &self.tools.exposures {
            if !names.contains(name) && *exposure != ToolExposure::Hidden {
                return Err(AgentDefinitionError::UnknownTool {
                    name: name.to_string(),
                });
            }
        }
        Ok(ToolExposurePlan::new(
            catalog
                .iter()
                .map(|spec| {
                    (
                        spec.name.clone(),
                        self.tools
                            .exposures
                            .get(&spec.name)
                            .copied()
                            .unwrap_or(ToolExposure::Hidden),
                    )
                })
                .collect::<BTreeMap<_, _>>(),
        ))
    }

    pub(crate) fn bind(
        &self,
        personal: Option<InstructionSnapshot>,
        project: Option<InstructionSnapshot>,
    ) -> Result<AgentDefinitionBinding, AgentDefinitionError> {
        AgentDefinitionBinding::new(self.clone(), personal, project)
    }

    pub(crate) fn validate_for_catalog(
        &self,
        catalog: &[ToolSpec],
    ) -> Result<(), AgentDefinitionError> {
        self.validate()?;
        let names = catalog_names(catalog)?;
        for name in self.tools.exposures.keys() {
            if !names.contains(name) {
                return Err(AgentDefinitionError::UnknownTool {
                    name: name.to_string(),
                });
            }
        }
        let exposure = |name: &str| {
            self.tools
                .exposures
                .get(&ToolName::new(name))
                .copied()
                .unwrap_or(ToolExposure::Hidden)
        };
        if self.research.is_none()
            && ["research_get", "research_update"]
                .into_iter()
                .any(|name| exposure(name) != ToolExposure::Hidden)
        {
            return Err(AgentDefinitionError::ResearchToolWithoutResearch);
        }
        if self.context.memory == AgentMemoryScope::None
            && ["memory_read", "memory_write"]
                .into_iter()
                .any(|name| exposure(name) != ToolExposure::Hidden)
        {
            return Err(AgentDefinitionError::MemoryToolWithoutMemory);
        }
        if self
            .tools
            .exposures
            .values()
            .any(|exposure| *exposure == ToolExposure::Deferred)
            && exposure("tool_search") != ToolExposure::Direct
        {
            return Err(AgentDefinitionError::DeferredToolsRequireSearch);
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<(), AgentDefinitionError> {
        if self.schema_version != AGENT_DEFINITION_SCHEMA_VERSION {
            return Err(AgentDefinitionError::UnsupportedSchemaVersion {
                actual: self.schema_version,
                expected: AGENT_DEFINITION_SCHEMA_VERSION,
            });
        }
        self.id.validate()?;
        if self.name.trim().is_empty() || self.name.len() > 256 {
            return Err(AgentDefinitionError::InvalidName);
        }
        self.instructions.validate()?;
        if let Some(research) = &self.research {
            research.validate()?;
        }
        self.digest.validate("definition_digest")?;
        let actual = self.expected_digest()?;
        if self.digest != actual {
            return Err(AgentDefinitionError::DefinitionDigestMismatch {
                expected: self.digest.0.clone(),
                actual: actual.0,
            });
        }
        Ok(())
    }

    fn expected_digest(&self) -> Result<AgentDefinitionDigest, AgentDefinitionError> {
        canonical_blake3_hash(&DefinitionSemanticIdentity {
            schema_version: self.schema_version,
            id: &self.id,
            name: &self.name,
            instructions: DefinitionSemanticInstructions {
                persona: self.instructions.persona.content(),
                work: self.instructions.work.content(),
                methods: self
                    .instructions
                    .methods
                    .iter()
                    .map(InstructionSnapshot::content)
                    .collect(),
            },
            context: &self.context,
            tools: &self.tools,
            research: self
                .research
                .as_ref()
                .map(|research| DefinitionSemanticResearch {
                    domain: research.domain,
                    phase: research.phase,
                    domain_prompts: research
                        .domain_prompts
                        .iter()
                        .map(|(profile, prompt)| (*profile, prompt.content()))
                        .collect(),
                    phase_prompts: research
                        .phase_prompts
                        .iter()
                        .map(|(phase, prompt)| (*phase, prompt.content()))
                        .collect(),
                }),
        })
        .map(AgentDefinitionDigest::from_hash)
        .map_err(|source| AgentDefinitionError::CanonicalEncoding {
            message: source.to_string(),
        })
    }
}

fn catalog_names(catalog: &[ToolSpec]) -> Result<BTreeSet<ToolName>, AgentDefinitionError> {
    let names = catalog
        .iter()
        .map(|spec| spec.name.clone())
        .collect::<BTreeSet<_>>();
    if names.len() != catalog.len() {
        return Err(AgentDefinitionError::DuplicateCatalogTool);
    }
    Ok(names)
}

#[derive(Serialize)]
struct DefinitionSemanticIdentity<'a> {
    schema_version: u16,
    id: &'a AgentId,
    name: &'a str,
    instructions: DefinitionSemanticInstructions<'a>,
    context: &'a AgentContextSelection,
    tools: &'a AgentToolExposure,
    research: Option<DefinitionSemanticResearch<'a>>,
}

#[derive(Serialize)]
struct DefinitionSemanticInstructions<'a> {
    persona: &'a str,
    work: &'a str,
    methods: Vec<&'a str>,
}

#[derive(Serialize)]
struct DefinitionSemanticResearch<'a> {
    domain: DomainProfile,
    phase: ResearchPhase,
    domain_prompts: BTreeMap<DomainProfile, &'a str>,
    phase_prompts: BTreeMap<ResearchPhase, &'a str>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentDefinitionBinding {
    schema_version: u16,
    definition: AgentDefinition,
    personal: Option<InstructionSnapshot>,
    project: Option<InstructionSnapshot>,
}

impl AgentDefinitionBinding {
    pub(crate) fn new(
        definition: AgentDefinition,
        personal: Option<InstructionSnapshot>,
        project: Option<InstructionSnapshot>,
    ) -> Result<Self, AgentDefinitionError> {
        let value = Self {
            schema_version: AGENT_DEFINITION_SCHEMA_VERSION,
            definition,
            personal,
            project,
        };
        value.validate()?;
        Ok(value)
    }

    pub fn definition(&self) -> &AgentDefinition {
        &self.definition
    }

    pub fn personal(&self) -> Option<&InstructionSnapshot> {
        self.personal.as_ref()
    }

    pub fn project(&self) -> Option<&InstructionSnapshot> {
        self.project.as_ref()
    }

    pub fn source_locator(&self) -> &DefinitionSourceLocator {
        self.definition.source_locator()
    }

    pub fn validate(&self) -> Result<(), AgentDefinitionError> {
        if self.schema_version != AGENT_DEFINITION_SCHEMA_VERSION {
            return Err(AgentDefinitionError::UnsupportedBindingSchemaVersion {
                actual: self.schema_version,
                expected: AGENT_DEFINITION_SCHEMA_VERSION,
            });
        }
        self.definition.validate()?;
        validate_optional_snapshot(
            self.definition.context.personal,
            self.personal.as_ref(),
            "personal",
        )?;
        validate_optional_snapshot(
            self.definition.context.project,
            self.project.as_ref(),
            "project",
        )?;
        Ok(())
    }

    pub fn reload_compatible_with(
        &self,
        candidate: &AgentDefinition,
    ) -> Result<(), AgentDefinitionError> {
        self.validate()?;
        candidate.validate()?;
        if self.definition.id != candidate.id {
            return Err(AgentDefinitionError::ReloadAgentIdChanged {
                current: self.definition.id.as_str().to_owned(),
                candidate: candidate.id.as_str().to_owned(),
            });
        }
        if self.definition.context.memory != candidate.context.memory {
            return Err(AgentDefinitionError::ReloadMemoryScopeChanged);
        }
        let current_research = self
            .definition
            .research
            .as_ref()
            .map(AgentResearchDefinition::compatibility_key);
        let candidate_research = candidate
            .research
            .as_ref()
            .map(AgentResearchDefinition::compatibility_key);
        if current_research != candidate_research {
            return Err(AgentDefinitionError::ReloadResearchContractChanged);
        }
        Ok(())
    }
}

fn validate_optional_snapshot(
    selection: AgentConfiguredContext,
    snapshot: Option<&InstructionSnapshot>,
    name: &'static str,
) -> Result<(), AgentDefinitionError> {
    match (selection, snapshot) {
        (AgentConfiguredContext::None, None) => Ok(()),
        (AgentConfiguredContext::Configured, Some(snapshot)) => snapshot.validate(),
        (AgentConfiguredContext::Configured, None) => Ok(()),
        (AgentConfiguredContext::None, Some(_)) => {
            Err(AgentDefinitionError::UnexpectedConfiguredPrompt { name })
        }
    }
}

fn validate_instruction_content(
    content: &str,
    source: &'static str,
) -> Result<(), AgentDefinitionError> {
    if content.trim().is_empty() {
        return Err(AgentDefinitionError::EmptyInstruction {
            instruction_source: source,
        });
    }
    if content.len() > MAX_AGENT_RESOURCE_BYTES {
        return Err(AgentDefinitionError::InstructionResourceTooLarge {
            instruction_source: source,
            actual: content.len(),
            maximum: MAX_AGENT_RESOURCE_BYTES,
        });
    }
    Ok(())
}

fn is_blake3_digest(value: &str) -> bool {
    value.len() == blake3::OUT_LEN * 2
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum AgentDefinitionError {
    #[error("Agent definition schema {actual} is unsupported; expected {expected}")]
    UnsupportedSchemaVersion { actual: u16, expected: u16 },
    #[error("Agent definition binding schema {actual} is unsupported; expected {expected}")]
    UnsupportedBindingSchemaVersion { actual: u16, expected: u16 },
    #[error(
        "Agent id `{value}` must contain 1..=64 lowercase letters, digits, `_`, `-`, or internal `.`"
    )]
    InvalidAgentId { value: String },
    #[error("Agent definition name must be non-empty and at most 256 bytes")]
    InvalidName,
    #[error("{instruction_source} must not be empty")]
    EmptyInstruction { instruction_source: &'static str },
    #[error("{instruction_source} has {actual} bytes; maximum is {maximum}")]
    InstructionResourceTooLarge {
        instruction_source: &'static str,
        actual: usize,
        maximum: usize,
    },
    #[error("Agent definition has {actual} method files; maximum is {maximum}")]
    TooManyMethodFiles { actual: usize, maximum: usize },
    #[error("Agent definition instruction resources have {actual} bytes; maximum is {maximum}")]
    InstructionBudgetExceeded { actual: usize, maximum: usize },
    #[error("Agent definition is missing the {kind} prompt for `{id}`")]
    MissingResearchPrompt {
        kind: &'static str,
        id: &'static str,
    },
    #[error("{field} must be a lowercase 64-character BLAKE3 digest, received `{value}`")]
    InvalidDigest { field: &'static str, value: String },
    #[error("instruction content digest mismatch: expected {expected}, actual {actual}")]
    ContentDigestMismatch { expected: String, actual: String },
    #[error("Agent definition digest mismatch: expected {expected}, actual {actual}")]
    DefinitionDigestMismatch { expected: String, actual: String },
    #[error("failed to encode Agent definition identity: {message}")]
    CanonicalEncoding { message: String },
    #[error("Agent definition names unknown Tool `{name}`")]
    UnknownTool { name: String },
    #[error("Tool catalog contains duplicate Tool names")]
    DuplicateCatalogTool,
    #[error("a generic Agent definition cannot expose Research Tools")]
    ResearchToolWithoutResearch,
    #[error("an Agent definition with memory = none cannot expose Memory Tools")]
    MemoryToolWithoutMemory,
    #[error("a definition with Deferred Tools must expose tool_search directly")]
    DeferredToolsRequireSearch,
    #[error(
        "Agent definition context.personal is none but the binding contains a Personal snapshot"
    )]
    UnexpectedConfiguredPrompt { name: &'static str },
    #[error("Agent reload changes id from `{current}` to `{candidate}`")]
    ReloadAgentIdChanged { current: String, candidate: String },
    #[error("Agent reload changes the durable Memory scope")]
    ReloadMemoryScopeChanged,
    #[error("Agent reload changes the fixed Research domain or phase contract")]
    ReloadResearchContractChanged,
}
