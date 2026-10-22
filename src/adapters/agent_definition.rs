//! Strict local Agent-definition loading and builtin template export.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;

use crate::domain::agent_definition::{
    AGENT_DEFINITION_SCHEMA_VERSION, AgentConfiguredContext, AgentContextSelection,
    AgentDefinition, AgentDefinitionBinding, AgentDefinitionError, AgentId, AgentInstructions,
    AgentMemoryScope, AgentResearchDefinition, AgentToolExposure, BuiltinAgentTemplate,
    DefinitionSourceLocator, InstructionSnapshot, InstructionSourceLocator,
    MAX_AGENT_RESOURCE_BYTES,
};
use crate::domain::agent_evolution::AgentRsi;
use crate::domain::domain_profile::DomainProfile;
use crate::domain::research_control::ResearchPhase;
use crate::domain::tools::{ToolExposure, ToolName, ToolSpec};

const MAX_MANIFEST_BYTES: usize = 64 * 1024;
const RESEARCH_MANIFEST: &str = include_str!("../../examples/agents/research/agent.toml");
const TECHNICAL_WRITING_MANIFEST: &str =
    include_str!("../../examples/agents/technical-writing/agent.toml");

pub(crate) fn load_definition(
    path: &Path,
    catalog: &[ToolSpec],
) -> Result<AgentDefinition, AgentDefinitionLoadError> {
    let manifest_path = definition_manifest_path(path);
    let manifest_path = fs::canonicalize(&manifest_path).map_err(|source| {
        AgentDefinitionLoadError::ReadResource {
            path: manifest_path,
            source,
        }
    })?;
    let root = manifest_path
        .parent()
        .expect("a canonical manifest has a parent")
        .to_path_buf();
    let raw = decode_manifest(
        &manifest_path,
        &read_utf8(&manifest_path, MAX_MANIFEST_BYTES, true)?,
    )?;
    let definition = resolve_raw_definition(
        raw,
        DefinitionSourceLocator::Local {
            manifest_path: manifest_path.clone(),
        },
        |resource| load_local_resource(&root, resource),
    )?;
    definition.validate_for_catalog(catalog)?;
    Ok(definition)
}

pub(crate) fn load_definition_binding(
    path: &Path,
    personal: Option<&Path>,
    project: Option<&Path>,
    catalog: &[ToolSpec],
) -> Result<AgentDefinitionBinding, AgentDefinitionLoadError> {
    bind_definition(load_definition(path, catalog)?, personal, project)
}

/// Reads only configured instruction files selected by the definition, then
/// captures their exact bytes in the durable binding.
pub(crate) fn bind_definition(
    definition: AgentDefinition,
    personal: Option<&Path>,
    project: Option<&Path>,
) -> Result<AgentDefinitionBinding, AgentDefinitionLoadError> {
    let personal = configured_prompt_snapshot(definition.context().personal(), personal)?;
    let project = configured_prompt_snapshot(definition.context().project(), project)?;
    definition.bind(personal, project).map_err(Into::into)
}

pub(crate) fn builtin_research_binding(
    personal: Option<&Path>,
    project: Option<&Path>,
    catalog: &[ToolSpec],
) -> Result<AgentDefinitionBinding, AgentDefinitionLoadError> {
    bind_definition(
        builtin_definition(BuiltinAgentTemplate::Research, catalog)?,
        personal,
        project,
    )
}

pub(crate) fn builtin_definition(
    template: BuiltinAgentTemplate,
    catalog: &[ToolSpec],
) -> Result<AgentDefinition, AgentDefinitionError> {
    let manifest = match template {
        BuiltinAgentTemplate::Research => RESEARCH_MANIFEST,
        BuiltinAgentTemplate::TechnicalWriting => TECHNICAL_WRITING_MANIFEST,
    };
    let raw =
        toml::from_str(manifest).map_err(|source| AgentDefinitionError::CanonicalEncoding {
            message: format!("embedded builtin definition is invalid: {source}"),
        })?;
    let definition = resolve_raw_definition(
        raw,
        DefinitionSourceLocator::Builtin { template },
        |resource| load_builtin_resource(template, resource),
    )?;
    definition.validate_for_catalog(catalog)?;
    Ok(definition)
}

pub(crate) fn write_builtin_definition(
    template: BuiltinAgentTemplate,
    destination: &Path,
    id: Option<&str>,
) -> Result<(), AgentDefinitionLoadError> {
    // Validate the optional custom ID before any filesystem change.
    let manifest = export_manifest(template, id)?;
    if destination.exists() {
        return Err(AgentDefinitionLoadError::ExportDestinationExists {
            path: destination.to_path_buf(),
        });
    }
    fs::create_dir_all(destination).map_err(|source| AgentDefinitionLoadError::WriteResource {
        path: destination.to_path_buf(),
        source,
    })?;
    for (relative_path, content) in builtin_files(template) {
        let path = destination.join(relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| {
                AgentDefinitionLoadError::WriteResource {
                    path: parent.to_path_buf(),
                    source,
                }
            })?;
        }
        let content = if relative_path == "agent.toml" {
            manifest.as_str()
        } else {
            content
        };
        fs::write(&path, content)
            .map_err(|source| AgentDefinitionLoadError::WriteResource { path, source })?;
    }
    Ok(())
}

fn definition_manifest_path(path: &Path) -> PathBuf {
    if path.is_dir() {
        path.join("agent.toml")
    } else {
        path.to_path_buf()
    }
}

fn configured_prompt_snapshot(
    selection: AgentConfiguredContext,
    path: Option<&Path>,
) -> Result<Option<InstructionSnapshot>, AgentDefinitionLoadError> {
    match selection {
        AgentConfiguredContext::None => Ok(None),
        AgentConfiguredContext::Configured => {
            let Some(path) = path else {
                return Ok(None);
            };
            let content = read_utf8(path, MAX_AGENT_RESOURCE_BYTES, false)?;
            InstructionSnapshot::loaded(
                content,
                InstructionSourceLocator::Local {
                    path: path.to_path_buf(),
                },
            )
            .map(Some)
            .map_err(Into::into)
        }
    }
}

fn load_local_resource(
    root: &Path,
    resource: &Path,
) -> Result<InstructionSnapshot, AgentDefinitionLoadError> {
    let path = resolve_definition_resource(root, resource)?;
    let content = read_utf8(&path, MAX_AGENT_RESOURCE_BYTES, false)?;
    InstructionSnapshot::loaded(content, InstructionSourceLocator::Local { path })
        .map_err(Into::into)
}

fn resolve_definition_resource(
    root: &Path,
    resource: &Path,
) -> Result<PathBuf, AgentDefinitionLoadError> {
    if resource.as_os_str().is_empty()
        || resource.is_absolute()
        || resource
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(AgentDefinitionLoadError::InvalidResourcePath {
            path: resource.to_path_buf(),
        });
    }
    let path = fs::canonicalize(root.join(resource)).map_err(|source| {
        AgentDefinitionLoadError::ReadResource {
            path: root.join(resource),
            source,
        }
    })?;
    if !path.starts_with(root) {
        return Err(AgentDefinitionLoadError::ResourceOutsideDefinition { path });
    }
    Ok(path)
}

fn read_utf8(
    path: &Path,
    maximum: usize,
    manifest: bool,
) -> Result<String, AgentDefinitionLoadError> {
    let bytes = fs::read(path).map_err(|source| AgentDefinitionLoadError::ReadResource {
        path: path.to_path_buf(),
        source,
    })?;
    if bytes.len() > maximum {
        if manifest {
            return Err(AgentDefinitionLoadError::ManifestTooLarge {
                path: path.to_path_buf(),
                maximum,
            });
        }
        return Err(AgentDefinitionError::InstructionResourceTooLarge {
            instruction_source: "instruction resource",
            actual: bytes.len(),
            maximum,
        }
        .into());
    }
    String::from_utf8(bytes).map_err(|_| AgentDefinitionLoadError::InvalidUtf8Resource {
        path: path.to_path_buf(),
    })
}

fn decode_manifest(path: &Path, content: &str) -> Result<RawDefinition, AgentDefinitionLoadError> {
    let raw: RawDefinition =
        toml::from_str(content).map_err(|source| AgentDefinitionLoadError::DecodeManifest {
            path: path.to_path_buf(),
            message: source.to_string(),
        })?;
    if raw.schema_version != AGENT_DEFINITION_SCHEMA_VERSION {
        return Err(AgentDefinitionError::UnsupportedSchemaVersion {
            actual: raw.schema_version,
            expected: AGENT_DEFINITION_SCHEMA_VERSION,
        }
        .into());
    }
    Ok(raw)
}

fn resolve_raw_definition<E>(
    raw: RawDefinition,
    source_locator: DefinitionSourceLocator,
    mut load_resource: impl FnMut(&Path) -> Result<InstructionSnapshot, E>,
) -> Result<AgentDefinition, E>
where
    E: From<AgentDefinitionError>,
{
    let persona = load_resource(&raw.instructions.persona_file)?;
    let work = load_resource(&raw.instructions.work_file)?;
    let methods = raw
        .instructions
        .method_files
        .iter()
        .map(|path| load_resource(path))
        .collect::<Result<Vec<_>, _>>()?;
    let instructions = AgentInstructions::new(persona, work, methods).map_err(E::from)?;
    let rsi = raw
        .rsi
        .map(|rsi| {
            let fixed_rules = rsi
                .fixed_rules_file
                .as_ref()
                .map(|path| load_resource(path))
                .transpose()?;
            Ok::<_, E>(AgentRsi {
                persona: rsi.persona,
                methods: rsi.methods,
                improvement: rsi.improvement,
                fixed_rules,
                state: None,
            })
        })
        .transpose()?;
    let tools = AgentToolExposure::new(
        raw.tools
            .into_iter()
            .map(|(name, exposure)| (ToolName::new(name), exposure))
            .collect(),
    );
    let research = raw
        .research
        .map(|research| {
            let domain_prompts = research
                .domain_prompt_files
                .into_iter()
                .map(|(profile, path)| load_resource(&path).map(|snapshot| (profile, snapshot)))
                .collect::<Result<BTreeMap<_, _>, _>>()?;
            let phase_prompts = research
                .phase_prompt_files
                .into_iter()
                .map(|(phase, path)| load_resource(&path).map(|snapshot| (phase, snapshot)))
                .collect::<Result<BTreeMap<_, _>, _>>()?;
            AgentResearchDefinition::new(
                research.domain,
                research.phase,
                domain_prompts,
                phase_prompts,
            )
            .map_err(E::from)
        })
        .transpose()?;
    AgentDefinition::loaded(
        source_locator,
        AgentId::new(raw.id).map_err(E::from)?,
        raw.name,
        instructions,
        AgentContextSelection::new(
            raw.context.personal,
            raw.context.project,
            raw.context.memory,
        ),
        tools,
        research,
        rsi,
    )
    .map_err(E::from)
}

fn load_builtin_resource(
    template: BuiltinAgentTemplate,
    resource: &Path,
) -> Result<InstructionSnapshot, AgentDefinitionError> {
    let name = resource
        .to_str()
        .ok_or_else(|| AgentDefinitionError::CanonicalEncoding {
            message: "builtin resource path is not UTF-8".to_owned(),
        })?;
    let content = builtin_files(template)
        .into_iter()
        .find_map(|(path, content)| (path == name).then_some(content))
        .ok_or_else(|| AgentDefinitionError::CanonicalEncoding {
            message: format!("builtin resource `{name}` is missing"),
        })?;
    InstructionSnapshot::loaded(
        content.to_owned(),
        InstructionSourceLocator::Builtin {
            template,
            resource: name.to_owned(),
        },
    )
}

fn export_manifest(
    template: BuiltinAgentTemplate,
    id: Option<&str>,
) -> Result<String, AgentDefinitionLoadError> {
    let manifest = match template {
        BuiltinAgentTemplate::Research => RESEARCH_MANIFEST,
        BuiltinAgentTemplate::TechnicalWriting => TECHNICAL_WRITING_MANIFEST,
    };
    let Some(id) = id else {
        return Ok(manifest.to_owned());
    };
    let id = AgentId::new(id.to_owned())?;
    let expected = match template {
        BuiltinAgentTemplate::Research => "id = \"research\"",
        BuiltinAgentTemplate::TechnicalWriting => "id = \"technical-writing\"",
    };
    Ok(manifest.replacen(expected, &format!("id = \"{}\"", id.as_str()), 1))
}

fn builtin_files(template: BuiltinAgentTemplate) -> Vec<(&'static str, &'static str)> {
    match template {
        BuiltinAgentTemplate::Research => vec![
            ("agent.toml", RESEARCH_MANIFEST),
            (
                "persona.md",
                include_str!("../../examples/agents/research/persona.md"),
            ),
            (
                "work.md",
                include_str!("../../examples/agents/research/work.md"),
            ),
            (
                "methods/evidence.md",
                include_str!("../../examples/agents/research/methods/evidence.md"),
            ),
            (
                "research/domain/source.md",
                include_str!("../../examples/agents/research/research/domain/source.md"),
            ),
            (
                "research/domain/web.md",
                include_str!("../../examples/agents/research/research/domain/web.md"),
            ),
            (
                "research/domain/mobile.md",
                include_str!("../../examples/agents/research/research/domain/mobile.md"),
            ),
            (
                "research/domain/binary.md",
                include_str!("../../examples/agents/research/research/domain/binary.md"),
            ),
            (
                "research/phase/mapping.md",
                include_str!("../../examples/agents/research/research/phase/mapping.md"),
            ),
            (
                "research/phase/hypothesis.md",
                include_str!("../../examples/agents/research/research/phase/hypothesis.md"),
            ),
            (
                "research/phase/experiment.md",
                include_str!("../../examples/agents/research/research/phase/experiment.md"),
            ),
            (
                "research/phase/verification.md",
                include_str!("../../examples/agents/research/research/phase/verification.md"),
            ),
            (
                "research/phase/evidence_curation.md",
                include_str!("../../examples/agents/research/research/phase/evidence_curation.md"),
            ),
        ],
        BuiltinAgentTemplate::TechnicalWriting => vec![
            ("agent.toml", TECHNICAL_WRITING_MANIFEST),
            (
                "persona.md",
                include_str!("../../examples/agents/technical-writing/persona.md"),
            ),
            (
                "work.md",
                include_str!("../../examples/agents/technical-writing/work.md"),
            ),
            (
                "methods/clarity.md",
                include_str!("../../examples/agents/technical-writing/methods/clarity.md"),
            ),
        ],
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDefinition {
    schema_version: u16,
    id: String,
    name: String,
    instructions: RawInstructions,
    context: RawContext,
    tools: BTreeMap<String, ToolExposure>,
    research: Option<RawResearch>,
    rsi: Option<RawRsi>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRsi {
    persona: bool,
    methods: bool,
    #[serde(default)]
    improvement: bool,
    fixed_rules_file: Option<PathBuf>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawInstructions {
    persona_file: PathBuf,
    work_file: PathBuf,
    method_files: Vec<PathBuf>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawContext {
    personal: AgentConfiguredContext,
    project: AgentConfiguredContext,
    memory: AgentMemoryScope,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawResearch {
    domain: DomainProfile,
    phase: ResearchPhase,
    domain_prompt_files: BTreeMap<DomainProfile, PathBuf>,
    phase_prompt_files: BTreeMap<ResearchPhase, PathBuf>,
}

#[derive(Debug, Error)]
pub(crate) enum AgentDefinitionLoadError {
    #[error(transparent)]
    Definition(#[from] AgentDefinitionError),
    #[error("failed to read Agent definition resource `{path}`: {source}")]
    ReadResource {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to decode Agent definition `{path}`: {message}")]
    DecodeManifest { path: PathBuf, message: String },
    #[error("Agent definition resource path `{path}` must be a non-empty relative file path")]
    InvalidResourcePath { path: PathBuf },
    #[error("Agent definition resource `{path}` resolves outside its definition directory")]
    ResourceOutsideDefinition { path: PathBuf },
    #[error("Agent definition resource `{path}` is not valid UTF-8")]
    InvalidUtf8Resource { path: PathBuf },
    #[error("Agent definition manifest `{path}` exceeds the {maximum}-byte limit")]
    ManifestTooLarge { path: PathBuf, maximum: usize },
    #[error("refusing to export Agent definition because `{path}` already exists")]
    ExportDestinationExists { path: PathBuf },
    #[error("failed to export Agent definition resource `{path}`: {source}")]
    WriteResource {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}
