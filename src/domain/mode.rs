//! Fixed Mode Pack prompt and Tool-exposure composition.

use serde::Serialize;
use thiserror::Error;

use crate::domain::context::{
    ContextFragment, ContextFragmentContent, ContextFragmentId, ContextFragmentPriority,
    ContextFragmentSource, FragmentCachePolicy,
};
use crate::domain::tools::ToolExposurePlan;

/// A bounded, local Mode selection. Filesystem discovery and TOML parsing are
/// intentionally deferred to a future configuration slice.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ModePack {
    id: String,
    generation: u64,
    prompt: String,
    tool_exposure: ToolExposurePlan,
}

impl ModePack {
    pub fn new(
        id: impl Into<String>,
        generation: u64,
        prompt: impl Into<String>,
        tool_exposure: ToolExposurePlan,
    ) -> Result<Self, ModeError> {
        let id = id.into();
        let prompt = prompt.into();
        if id.trim().is_empty() {
            return Err(ModeError::EmptyModeId);
        }
        if prompt.trim().is_empty() {
            return Err(ModeError::EmptyModePrompt);
        }
        Ok(Self {
            id,
            generation,
            prompt,
            tool_exposure,
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub const fn generation(&self) -> u64 {
        self.generation
    }

    pub fn prompt(&self) -> &str {
        &self.prompt
    }

    pub fn tool_exposure(&self) -> &ToolExposurePlan {
        &self.tool_exposure
    }
}

/// The four fixed instruction sources assembled before Research and History.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PromptComposition {
    base: String,
    personal: Option<String>,
    project: Option<String>,
    mode: Option<ModePack>,
}

impl PromptComposition {
    pub fn new(base: impl Into<String>) -> Result<Self, ModeError> {
        let base = base.into();
        if base.trim().is_empty() {
            return Err(ModeError::EmptyBasePrompt);
        }
        Ok(Self {
            base,
            personal: None,
            project: None,
            mode: None,
        })
    }

    pub fn with_personal(mut self, prompt: impl Into<String>) -> Result<Self, ModeError> {
        self.personal = Some(non_empty_prompt(prompt.into(), "personal")?);
        Ok(self)
    }

    pub fn with_project(mut self, prompt: impl Into<String>) -> Result<Self, ModeError> {
        self.project = Some(non_empty_prompt(prompt.into(), "project")?);
        Ok(self)
    }

    pub fn with_mode(mut self, mode: ModePack) -> Self {
        self.mode = Some(mode);
        self
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn mode(&self) -> Option<&ModePack> {
        self.mode.as_ref()
    }

    pub fn fragments(&self) -> Result<Vec<ContextFragment>, ModeError> {
        let mut fragments = vec![instruction_fragment(
            "prompt:base",
            ContextFragmentSource::BasePrompt,
            ContextFragmentPriority::Required,
            &self.base,
        )?];
        if let Some(personal) = &self.personal {
            fragments.push(instruction_fragment(
                "prompt:personal",
                ContextFragmentSource::PersonalPreferences,
                ContextFragmentPriority::High,
                personal,
            )?);
        }
        if let Some(project) = &self.project {
            fragments.push(instruction_fragment(
                "prompt:project",
                ContextFragmentSource::ProjectPrompt,
                ContextFragmentPriority::High,
                project,
            )?);
        }
        if let Some(mode) = &self.mode {
            fragments.push(instruction_fragment(
                &format!("prompt:mode:{}", mode.id()),
                ContextFragmentSource::ModePrompt,
                ContextFragmentPriority::High,
                mode.prompt(),
            )?);
        }
        Ok(fragments)
    }
}

fn non_empty_prompt(prompt: String, source: &str) -> Result<String, ModeError> {
    if prompt.trim().is_empty() {
        return Err(ModeError::EmptyPrompt {
            prompt_source: source.into(),
        });
    }
    Ok(prompt)
}

fn instruction_fragment(
    id: &str,
    source: ContextFragmentSource,
    priority: ContextFragmentPriority,
    text: &str,
) -> Result<ContextFragment, ModeError> {
    ContextFragment::new(
        ContextFragmentId::new(id)?,
        source,
        priority,
        FragmentCachePolicy::ContentAddressed,
        ContextFragmentContent::Instructions {
            text: text.to_owned(),
        },
    )
    .map_err(ModeError::Context)
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ModeError {
    #[error("Mode Pack ID must not be empty")]
    EmptyModeId,
    #[error("Mode Pack prompt must not be empty")]
    EmptyModePrompt,
    #[error("base prompt must not be empty")]
    EmptyBasePrompt,
    #[error("{prompt_source} prompt must not be empty")]
    EmptyPrompt { prompt_source: String },
    #[error(transparent)]
    Context(#[from] crate::domain::context::ContextPlannerError),
}
