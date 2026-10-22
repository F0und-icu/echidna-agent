//! Bounded self-evolution data; publication and execution remain I/O owners.

use serde::{Deserialize, Serialize};

use super::agent_definition::InstructionSnapshot;
pub(crate) use super::agent_definition::is_blake3_digest as is_digest;
use super::events::{EventId, StreamId};
use super::tools::ToolCallId;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AgentRsi {
    pub(crate) persona: bool,
    pub(crate) methods: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) improvement: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) fixed_rules: Option<InstructionSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) state: Option<EvolutionState>,
}

pub(crate) fn is_false(value: &bool) -> bool {
    !value
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EvolutionState {
    pub(crate) base_source_digest: String,
    pub(crate) parent_digest: String,
    pub(crate) reason: String,
    pub(crate) basis: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) rollback_to: Option<String>,
    pub(crate) tools: Vec<PublishedScript>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) experiences: Vec<EvolutionExperience>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EvolutionExperience {
    pub(crate) cue: String,
    pub(crate) change: String,
    pub(crate) attempt_id: String,
    pub(crate) references: Vec<EventId>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScriptFile {
    pub(crate) path: String,
    pub(crate) blake3: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PublishedScript {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) argv: Vec<String>,
    pub(crate) dependencies: Vec<String>,
    pub(crate) files: Vec<ScriptFile>,
    pub(crate) test_session_id: StreamId,
    pub(crate) test_call_id: ToolCallId,
    pub(crate) test_receipt_event_id: EventId,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EvolutionCandidate {
    pub(crate) schema_version: u16,
    pub(crate) agent_id: String,
    pub(crate) session_id: String,
    pub(crate) parent_digest: String,
    pub(crate) reason: String,
    pub(crate) basis: Vec<String>,
    pub(crate) persona: Option<String>,
    pub(crate) methods: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) tools: Vec<ScriptCandidate>,
    #[serde(default)]
    pub(crate) remove_tools: Vec<String>,
    pub(crate) rollback_to: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) experience: Option<EvolutionExperience>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScriptCandidate {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) argv: Vec<String>,
    pub(crate) dependencies: Vec<String>,
    pub(crate) files: Vec<ScriptFile>,
    pub(crate) test_call_id: ToolCallId,
}

pub(crate) fn pending_relative(agent_id: &str, session_id: &str) -> String {
    format!(
        "evolution/{agent_id}/pending/{}.json",
        blake3::hash(session_id.as_bytes()).to_hex()
    )
}

impl AgentRsi {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if !self.persona && !self.methods {
            return Err("RSI requires at least one editable instruction scope".to_owned());
        }
        if let Some(state) = &self.state {
            if !is_digest(&state.base_source_digest) || !is_digest(&state.parent_digest) {
                return Err("RSI source/parent identity must be a BLAKE3 digest".to_owned());
            }
            validate_feedback(&state.reason, &state.basis)?;
            if (!self.improvement && !state.experiences.is_empty())
                || state.experiences.len() > 8
                || state.experiences.iter().any(|experience| {
                    experience.cue.trim().is_empty()
                        || experience.cue.len() > 256
                        || experience.change.trim().is_empty()
                        || experience.change.len() > 2048
                        || experience.attempt_id.is_empty()
                        || experience.references.is_empty()
                        || experience.references.len() > 8
                })
            {
                return Err(
                    "RSI experience requires bounded cue, change and actual contrast references"
                        .to_owned(),
                );
            }
            if state.tools.len() > 16 {
                return Err("RSI supports at most 16 published scripts".to_owned());
            }
            let mut names = std::collections::BTreeSet::new();
            for script in &state.tools {
                validate_script(
                    &script.name,
                    &script.description,
                    &script.argv,
                    &script.dependencies,
                    &script.files,
                )?;
                if !names.insert(&script.name) {
                    return Err("RSI script names must be unique".to_owned());
                }
            }
            if state
                .rollback_to
                .as_ref()
                .is_some_and(|value| !is_digest(value))
            {
                return Err("RSI rollback identity must be a BLAKE3 digest".to_owned());
            }
        }
        Ok(())
    }
}

pub(crate) fn validate_feedback(reason: &str, basis: &[String]) -> Result<(), String> {
    if reason.trim().is_empty()
        || reason.len() > 2048
        || basis.is_empty()
        || basis.len() > 8
        || basis
            .iter()
            .any(|item| item.trim().is_empty() || item.len() > 2048)
    {
        return Err(
            "RSI reason and 1..=8 basis entries must be nonempty and at most 2048 bytes each"
                .to_owned(),
        );
    }
    Ok(())
}

pub(crate) fn validate_script(
    name: &str,
    description: &str,
    argv: &[String],
    dependencies: &[String],
    files: &[ScriptFile],
) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        || description.trim().is_empty()
        || description.len() > 2048
    {
        return Err("RSI script needs a bounded name and description".to_owned());
    }
    if argv.is_empty()
        || argv.len() > 32
        || argv
            .iter()
            .any(|value| value.is_empty() || value.len() > 4096)
        || dependencies.len() > 16
        || dependencies
            .iter()
            .any(|value| value.trim().is_empty() || value.len() > 256)
        || files.is_empty()
        || files.len() > 8
    {
        return Err("RSI script argv/dependencies/files exceed their bounds".to_owned());
    }
    validate_input_files(files)?;
    if !files.iter().any(|file| argv.contains(&file.path)) {
        return Err("RSI argv must explicitly name at least one indexed script file".to_owned());
    }
    Ok(())
}

pub(crate) fn validate_input_files(files: &[ScriptFile]) -> Result<(), String> {
    if files.len() > 8 {
        return Err("at most 8 verified input files are supported".to_owned());
    }
    let mut paths = std::collections::BTreeSet::new();
    for file in files {
        let path = std::path::Path::new(&file.path);
        if path.is_absolute()
            || file.path.is_empty()
            || file.path.len() > 4096
            || !path
                .components()
                .all(|part| matches!(part, std::path::Component::Normal(_)))
            || !is_digest(&file.blake3)
            || !paths.insert(&file.path)
        {
            return Err(
                "verified inputs need unique bounded project-relative paths and BLAKE3 digests"
                    .to_owned(),
            );
        }
    }
    Ok(())
}
