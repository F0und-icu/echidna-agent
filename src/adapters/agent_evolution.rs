//! Local immutable RSI revisions and a new-Session publication pointer.

use std::fs;
#[cfg(unix)]
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[cfg(unix)]
use super::project_event_store::commit_new_durable_file_with;
use super::project_event_store::{Project, commit_new_durable_file, replace_durable_file};
use crate::domain::agent_definition::{AgentDefinition, AgentDefinitionBinding};
use crate::domain::agent_evolution::{
    EvolutionCandidate, EvolutionState, PublishedScript, ScriptCandidate, ScriptFile, is_digest,
    pending_relative, validate_feedback, validate_script,
};
use crate::domain::events::StreamId;
use crate::domain::model::ContentItem;
use crate::domain::session::SessionState;
use crate::domain::tools::{ToolName, ToolResultStatus};

const MAX_REVISION_BYTES: usize = 1024 * 1024;
const MAX_CANDIDATE_BYTES: usize = 320 * 1024;

#[derive(Debug, Error)]
pub(crate) enum EvolutionError {
    #[error("RSI candidate rejected: {0}")]
    Rejected(String),
    #[error("RSI stored state is invalid: {0}")]
    Corrupt(String),
    #[error("RSI filesystem `{path}`: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Store(#[from] super::project_event_store::ProjectEventStoreError),
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ActiveRevision {
    schema_version: u16,
    base_source_digest: String,
    digest: String,
}

pub(crate) struct EvolutionStore<'a> {
    project: &'a Project,
    root: PathBuf,
}

fn rejected(error: impl std::fmt::Display) -> EvolutionError {
    EvolutionError::Rejected(error.to_string())
}

impl<'a> EvolutionStore<'a> {
    pub(crate) fn new(project: &'a Project, binding: &AgentDefinitionBinding) -> Self {
        Self {
            project,
            root: project
                .control_directory()
                .join("evolution")
                .join(binding.definition().id().as_str()),
        }
    }

    pub(crate) fn select(
        &self,
        source: AgentDefinitionBinding,
    ) -> Result<AgentDefinitionBinding, EvolutionError> {
        if source.definition().rsi().is_none() {
            return Ok(source);
        }
        let _writer = self.project.open_writer()?;
        self.prepare_directories()?;
        let active = match self.active()? {
            Some(active) => active,
            None => {
                self.save_revision(&source)?;
                self.set_active(&source)?;
                return Ok(source);
            }
        };
        if active.base_source_digest != source.definition().digest().as_str() {
            return Err(rejected(
                "source definition changed; use `agent lineage <path>` to explicitly start a new lineage while retaining old revisions",
            ));
        }
        let selected = self.revision(&active.digest)?;
        let selected_source = selected
            .rsi()
            .and_then(|rsi| rsi.state.as_ref())
            .map_or(selected.digest().as_str(), |state| {
                state.base_source_digest.as_str()
            });
        if selected_source != active.base_source_digest {
            return Err(EvolutionError::Corrupt(
                "publication points to another source lineage".to_owned(),
            ));
        }
        source
            .replacing_definition(selected.with_source_locator(source.definition()))
            .map_err(rejected)
    }

    pub(crate) fn prepare(&self, binding: &AgentDefinitionBinding) -> Result<(), EvolutionError> {
        if binding.definition().rsi().is_none() {
            return Ok(());
        }
        let _writer = self.project.open_writer()?;
        self.prepare_directories()
    }

    fn prepare_directories(&self) -> Result<(), EvolutionError> {
        for path in [
            self.root.join("revisions"),
            self.root.join("pending"),
            self.root.join("scripts"),
        ] {
            fs::create_dir_all(&path).map_err(|source| EvolutionError::Io { path, source })?;
        }
        Ok(())
    }

    fn active(&self) -> Result<Option<ActiveRevision>, EvolutionError> {
        let path = self.root.join("active.json");
        let Some(bytes) = read_optional(&path, 4096).map_err(stored_error)? else {
            return Ok(None);
        };
        let active: ActiveRevision = serde_json::from_slice(&bytes)
            .map_err(|error| EvolutionError::Corrupt(error.to_string()))?;
        if active.schema_version != 1
            || !is_digest(&active.digest)
            || !is_digest(&active.base_source_digest)
        {
            return Err(EvolutionError::Corrupt(
                "invalid publication identity".to_owned(),
            ));
        }
        Ok(Some(active))
    }

    pub(crate) fn revision(&self, digest: &str) -> Result<AgentDefinition, EvolutionError> {
        if !is_digest(digest) {
            return Err(rejected("invalid revision digest"));
        }
        let path = self.root.join("revisions").join(format!("{digest}.json"));
        let bytes = read_optional(&path, MAX_REVISION_BYTES)
            .map_err(stored_error)?
            .ok_or_else(|| rejected("revision does not exist"))?;
        let definition: AgentDefinition = serde_json::from_slice(&bytes)
            .map_err(|error| EvolutionError::Corrupt(error.to_string()))?;
        definition
            .validate()
            .map_err(|error| EvolutionError::Corrupt(error.to_string()))?;
        if definition.digest().as_str() != digest
            || self.root.file_name().and_then(|id| id.to_str()) != Some(definition.id().as_str())
        {
            return Err(EvolutionError::Corrupt(
                "revision content identity or Agent ID mismatch".to_owned(),
            ));
        }
        Ok(definition)
    }

    pub(crate) fn pending(
        &self,
        binding: &AgentDefinitionBinding,
        session_id: &StreamId,
        state: &SessionState,
    ) -> Result<Option<AgentDefinitionBinding>, EvolutionError> {
        let _writer = self.project.open_writer()?;
        let path = self.project.control_directory().join(pending_relative(
            binding.definition().id().as_str(),
            session_id.as_str(),
        ));
        let Some(bytes) = read_optional(&path, MAX_CANDIDATE_BYTES)? else {
            return Ok(None);
        };
        let candidate: EvolutionCandidate = serde_json::from_slice(&bytes).map_err(|error| {
            rejected(format!(
                "invalid candidate JSON or unsupported fields at line {}, column {}",
                error.line(),
                error.column()
            ))
        })?;
        self.candidate(binding, session_id, state, candidate)
            .map(Some)
    }

    pub(crate) fn candidate(
        &self,
        binding: &AgentDefinitionBinding,
        session_id: &StreamId,
        state: &SessionState,
        candidate: EvolutionCandidate,
    ) -> Result<AgentDefinitionBinding, EvolutionError> {
        if candidate.schema_version != 1
            || candidate.agent_id != binding.definition().id().as_str()
            || candidate.session_id != session_id.as_str()
            || candidate.parent_digest != binding.definition().digest().as_str()
        {
            return Err(rejected(
                "candidate schema, Agent, Session or parent does not match the active binding",
            ));
        }
        self.require_head(binding)?;
        validate_feedback(&candidate.reason, &candidate.basis).map_err(rejected)?;
        let policy = binding
            .definition()
            .rsi()
            .expect("pending is read only for enabled RSI");
        let base_source_digest = source_digest(binding).to_owned();
        let (persona, methods, mut tools, mut experiences) =
            if let Some(rollback) = &candidate.rollback_to {
                if candidate.persona.is_some()
                    || candidate.methods.is_some()
                    || !candidate.tools.is_empty()
                    || !candidate.remove_tools.is_empty()
                    || candidate.experience.is_some()
                {
                    return Err(rejected(
                        "rollback cannot be combined with instruction or script edits",
                    ));
                }
                let target = self.revision(rollback)?;
                let target_source = target
                    .rsi()
                    .and_then(|rsi| rsi.state.as_ref())
                    .map_or(target.digest().as_str(), |state| {
                        state.base_source_digest.as_str()
                    });
                if target_source != base_source_digest {
                    return Err(rejected(
                        "rollback revision belongs to another source lineage",
                    ));
                }
                let target_rsi = target
                    .rsi()
                    .ok_or_else(|| rejected("rollback target has no RSI policy"))?;
                if target_rsi.persona != policy.persona
                    || target_rsi.methods != policy.methods
                    || target_rsi.improvement != policy.improvement
                    || target_rsi.fixed_rules != policy.fixed_rules
                {
                    return Err(rejected("rollback cannot change RSI policy or fixed rules"));
                }
                (
                    policy
                        .persona
                        .then(|| target.instructions().persona().content().to_owned()),
                    policy.methods.then(|| {
                        target
                            .instructions()
                            .methods()
                            .iter()
                            .map(|method| method.content().to_owned())
                            .collect()
                    }),
                    target_rsi
                        .state
                        .as_ref()
                        .map_or_else(Vec::new, |state| state.tools.clone()),
                    target_rsi
                        .state
                        .as_ref()
                        .map_or_else(Vec::new, |state| state.experiences.clone()),
                )
            } else {
                (
                    candidate.persona.clone(),
                    candidate.methods.clone(),
                    policy
                        .state
                        .as_ref()
                        .map_or_else(Vec::new, |state| state.tools.clone()),
                    policy
                        .state
                        .as_ref()
                        .map_or_else(Vec::new, |state| state.experiences.clone()),
                )
            };
        if candidate.persona.is_none()
            && candidate.methods.is_none()
            && candidate.tools.is_empty()
            && candidate.remove_tools.is_empty()
            && candidate.rollback_to.is_none()
        {
            return Err(rejected(
                "candidate contains no instruction, script or rollback change",
            ));
        }
        for name in &candidate.remove_tools {
            let before = tools.len();
            tools.retain(|tool| &tool.name != name);
            if before == tools.len() {
                return Err(rejected("cannot remove an unknown script"));
            }
        }
        if candidate.tools.len() > 16 {
            return Err(rejected("at most 16 script updates per batch"));
        }
        for script in &candidate.tools {
            let published = self.validate_script(script, session_id, state)?;
            tools.retain(|tool| tool.name != published.name);
            tools.push(published);
        }
        tools.sort_by(|left, right| left.name.cmp(&right.name));
        if let Some(experience) = candidate.experience {
            let expected = state
                .improvement_evidence(&experience.attempt_id, &candidate.parent_digest)
                .map_err(rejected)?;
            if !policy.improvement || experience.references != expected {
                return Err(rejected(
                    "experience must reference this Session's accepted real contrast",
                ));
            }
            experiences.push(experience);
            if experiences.len() > 8 {
                experiences.remove(0);
            }
        }
        let definition = binding
            .definition()
            .with_evolution(
                persona,
                methods,
                EvolutionState {
                    base_source_digest,
                    parent_digest: candidate.parent_digest,
                    reason: candidate.reason,
                    basis: candidate.basis,
                    rollback_to: candidate.rollback_to,
                    tools,
                    experiences,
                },
            )
            .map_err(rejected)?;
        encode_revision(&definition)?;
        binding.replacing_definition(definition).map_err(rejected)
    }

    pub(crate) fn stage_candidate(
        &self,
        binding: &AgentDefinitionBinding,
        session_id: &StreamId,
        candidate: &EvolutionCandidate,
    ) -> Result<(), EvolutionError> {
        let _writer = self.project.open_writer()?;
        self.require_head(binding)?;
        let bytes = serde_json::to_vec(candidate).map_err(rejected)?;
        if bytes.len() > MAX_CANDIDATE_BYTES {
            return Err(rejected("host candidate exceeds bounded pending schema"));
        }
        let path = self.project.control_directory().join(pending_relative(
            binding.definition().id().as_str(),
            session_id.as_str(),
        ));
        commit_new_durable_file(&path, &bytes)?;
        Ok(())
    }

    fn validate_script(
        &self,
        script: &ScriptCandidate,
        session_id: &StreamId,
        state: &SessionState,
    ) -> Result<PublishedScript, EvolutionError> {
        validate_script(
            &script.name,
            &script.description,
            &script.argv,
            &script.dependencies,
            &script.files,
        )
        .map_err(rejected)?;
        let call = state
            .history()
            .items()
            .iter()
            .find_map(|item| match item {
                ContentItem::ToolCall { call } if call.call_id == script.test_call_id => Some(call),
                _ => None,
            })
            .ok_or_else(|| rejected("script test call is not in this Session"))?;
        let result = state
            .history()
            .items()
            .iter()
            .find_map(|item| match item {
                ContentItem::ToolResult { result } if result.call_id == script.test_call_id => {
                    Some(result)
                }
                _ => None,
            })
            .ok_or_else(|| rejected("script test result is not paired"))?;
        let receipt = state
            .execution_receipts()
            .iter()
            .find(|record| {
                record.receipt.execution_id
                    == crate::domain::tools::ExecutionId::for_model_call(&call.call_id)
            })
            .ok_or_else(|| rejected("script test has no real Receipt"))?;
        if call.name != ToolName::new("run_command")
            || result.status != ToolResultStatus::Succeeded
            || receipt.receipt.status != ToolResultStatus::Succeeded
            || result.output["exit_code"] != 0
        {
            return Err(rejected(
                "script test requires an actual successful run_command and Receipt",
            ));
        }
        let argv: Vec<String> =
            serde_json::from_value(call.arguments["argv"].clone()).map_err(rejected)?;
        if !argv.starts_with(&script.argv) {
            return Err(rejected("test argv does not invoke the indexed entrypoint"));
        }
        if call
            .arguments
            .get("cwd")
            .is_some_and(|cwd| !cwd.is_null() && cwd != ".")
        {
            return Err(rejected(
                "indexed script tests must use the project-root cwd",
            ));
        }
        let attested: Vec<ScriptFile> =
            serde_json::from_value(result.output["verified_inputs"].clone())
                .map_err(|_| rejected("test needs runtime verified_inputs attestation"))?;
        let mut bundle_files = Vec::new();
        for file in &script.files {
            if !attested.contains(file) {
                return Err(rejected(
                    "script code identity was not verified by the test invocation",
                ));
            }
            let source_path = self.project.root().join(&file.path);
            let mode = private_script_mode(&source_path)?;
            if script.argv[0] == file.path && mode != 0o700 {
                return Err(rejected("direct script entrypoint is no longer executable"));
            }
            let bytes = fs::read(&source_path).map_err(|source| EvolutionError::Io {
                path: self.project.root().join(&file.path),
                source,
            })?;
            if bytes.len() > 1024 * 1024 || blake3::hash(&bytes).to_hex().as_str() != file.blake3 {
                return Err(rejected("script bytes changed after the recorded test"));
            }
            bundle_files.push((file, bytes, mode));
        }
        let bundle_identity: Vec<_> = bundle_files
            .iter()
            .map(|(file, _, mode)| (*file, *mode))
            .collect();
        let bundle = crate::domain::canonical_json::canonical_blake3_hash(&bundle_identity)
            .map_err(rejected)?;
        let mut files = Vec::new();
        let mut published_argv = script.argv.clone();
        for (file, bytes, mode) in bundle_files {
            let relative = format!(
                "{}/evolution/{}/scripts/{}/{}",
                self.project.control_directory_name(),
                self.root.file_name().expect("Agent root").to_string_lossy(),
                bundle,
                file.path
            );
            let destination = self.project.root().join(&relative);
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent).map_err(|source| EvolutionError::Io {
                    path: parent.to_path_buf(),
                    source,
                })?;
            }
            store_immutable_script(&destination, &bytes, mode)?;
            for argument in &mut published_argv {
                if argument == &file.path {
                    *argument = relative.clone();
                }
            }
            files.push(ScriptFile {
                path: relative,
                blake3: file.blake3.clone(),
            });
        }
        Ok(PublishedScript {
            name: script.name.clone(),
            description: script.description.clone(),
            argv: published_argv,
            dependencies: script.dependencies.clone(),
            files,
            test_session_id: session_id.clone(),
            test_call_id: script.test_call_id.clone(),
            test_receipt_event_id: receipt.event_id.clone(),
        })
    }

    pub(crate) fn require_head(
        &self,
        binding: &AgentDefinitionBinding,
    ) -> Result<(), EvolutionError> {
        let active = self
            .active()?
            .ok_or_else(|| rejected("no published RSI head"))?;
        if active.digest != binding.definition().digest().as_str()
            || active.base_source_digest != source_digest(binding)
        {
            return Err(rejected(
                "saved Session is behind the published head; use explicit /agent reload to select it, or /rsi reconcile after an interrupted publication",
            ));
        }
        Ok(())
    }

    pub(crate) fn save_revision(
        &self,
        binding: &AgentDefinitionBinding,
    ) -> Result<(), EvolutionError> {
        let path = self
            .root
            .join("revisions")
            .join(format!("{}.json", binding.definition().digest().as_str()));
        if path.exists() {
            self.revision(binding.definition().digest().as_str())?;
            return Ok(());
        }
        let bytes = encode_revision(binding.definition())?;
        store_immutable(&path, &bytes)
    }

    pub(crate) fn set_active(
        &self,
        binding: &AgentDefinitionBinding,
    ) -> Result<(), EvolutionError> {
        let active = ActiveRevision {
            schema_version: 1,
            base_source_digest: source_digest(binding).to_owned(),
            digest: binding.definition().digest().as_str().to_owned(),
        };
        let bytes = serde_json::to_vec_pretty(&active)
            .map_err(|error| EvolutionError::Corrupt(error.to_string()))?;
        replace_durable_file(&self.root.join("active.json"), &bytes)?;
        Ok(())
    }

    pub(crate) fn finish(
        &self,
        binding: &AgentDefinitionBinding,
        session_id: &StreamId,
    ) -> Result<(), EvolutionError> {
        let _writer = self.project.open_writer()?;
        self.set_active(binding)?;
        let path = self.project.control_directory().join(pending_relative(
            binding.definition().id().as_str(),
            session_id.as_str(),
        ));
        fs::remove_file(&path).map_err(|source| EvolutionError::Io {
            path: path.clone(),
            source,
        })?;
        super::project_event_store::sync_directory(path.parent().expect("pending directory"))?;
        Ok(())
    }

    pub(crate) fn reconcile(&self, binding: &AgentDefinitionBinding) -> Result<(), EvolutionError> {
        let _writer = self.project.open_writer()?;
        let active = self
            .active()?
            .ok_or_else(|| rejected("no RSI publication to reconcile"))?;
        if active.base_source_digest != source_digest(binding) {
            return Err(rejected("publication has another source lineage"));
        }
        if active.digest == binding.definition().digest().as_str() {
            return Ok(());
        }
        let saved = binding
            .definition()
            .rsi()
            .and_then(|rsi| rsi.state.as_ref())
            .ok_or_else(|| rejected("Session has no committed RSI transition"))?;
        if active.digest != saved.parent_digest
            || active.base_source_digest != saved.base_source_digest
        {
            return Err(rejected("publication has another head or source lineage"));
        }
        self.revision(binding.definition().digest().as_str())?;
        self.set_active(binding)
    }

    pub(crate) fn new_lineage(
        &self,
        binding: &AgentDefinitionBinding,
    ) -> Result<(), EvolutionError> {
        if binding.definition().rsi().is_none() {
            return Err(rejected("definition has no [rsi] opt-in"));
        }
        let _writer = self.project.open_writer()?;
        self.prepare_directories()?;
        self.save_revision(binding)?;
        self.set_active(binding)
    }
}

fn source_digest(binding: &AgentDefinitionBinding) -> &str {
    binding
        .definition()
        .rsi()
        .and_then(|rsi| rsi.state.as_ref())
        .map_or(binding.definition().digest().as_str(), |state| {
            state.base_source_digest.as_str()
        })
}

fn read_optional(path: &Path, maximum: usize) -> Result<Option<Vec<u8>>, EvolutionError> {
    match fs::read(path) {
        Ok(bytes) if bytes.len() > maximum => Err(rejected(format!(
            "{} exceeds {maximum} bytes",
            path.display()
        ))),
        Ok(bytes) => Ok(Some(bytes)),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(EvolutionError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn stored_error(error: EvolutionError) -> EvolutionError {
    match error {
        EvolutionError::Rejected(message) => EvolutionError::Corrupt(message),
        error => error,
    }
}

fn encode_revision(definition: &AgentDefinition) -> Result<Vec<u8>, EvolutionError> {
    let bytes = serde_json::to_vec_pretty(definition)
        .map_err(|error| EvolutionError::Corrupt(error.to_string()))?;
    if bytes.len() > MAX_REVISION_BYTES {
        return Err(rejected(format!(
            "encoded RSI revision has {} bytes; maximum is {MAX_REVISION_BYTES}",
            bytes.len()
        )));
    }
    Ok(bytes)
}

#[cfg(unix)]
fn private_script_mode(path: &Path) -> Result<u32, EvolutionError> {
    let permissions = fs::metadata(path)
        .map_err(|source| EvolutionError::Io {
            path: path.to_path_buf(),
            source,
        })?
        .permissions();
    Ok(if permissions.mode() & 0o111 == 0 {
        0o600
    } else {
        0o700
    })
}

#[cfg(not(unix))]
fn private_script_mode(_path: &Path) -> Result<u32, EvolutionError> {
    Err(rejected(
        "RSI script publication requires Unix execution modes",
    ))
}

#[cfg(unix)]
fn store_immutable_script(path: &Path, bytes: &[u8], mode: u32) -> Result<(), EvolutionError> {
    if path.exists() {
        store_immutable(path, bytes)?;
        let permissions = fs::metadata(path)
            .map_err(|source| EvolutionError::Io {
                path: path.to_path_buf(),
                source,
            })?
            .permissions();
        if permissions.mode() & 0o7777 != mode {
            return Err(EvolutionError::Corrupt(format!(
                "immutable script permissions changed at {}",
                path.display()
            )));
        }
        return Ok(());
    }
    commit_new_durable_file_with(path, |file| {
        file.set_permissions(fs::Permissions::from_mode(mode))?;
        file.write_all(bytes)
    })?;
    Ok(())
}

#[cfg(not(unix))]
fn store_immutable_script(_path: &Path, _bytes: &[u8], _mode: u32) -> Result<(), EvolutionError> {
    Err(rejected(
        "RSI script publication requires Unix execution modes",
    ))
}

fn store_immutable(path: &Path, bytes: &[u8]) -> Result<(), EvolutionError> {
    match fs::read(path) {
        Ok(existing) if existing == bytes => Ok(()),
        Ok(_) => Err(EvolutionError::Corrupt(format!(
            "immutable revision collision at {}",
            path.display()
        ))),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            commit_new_durable_file(path, bytes)?;
            Ok(())
        }
        Err(source) => Err(EvolutionError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}
