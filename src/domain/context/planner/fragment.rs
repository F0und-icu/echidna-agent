//! Validated provider-neutral Context fragment inputs.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};

use crate::domain::artifacts::ArtifactId;
use crate::domain::model::ContentItem;

use super::error::ContextPlannerError;
use super::estimator;
use super::fingerprint::hash;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ContextFragmentId(String);

impl ContextFragmentId {
    pub fn new(value: impl Into<String>) -> Result<Self, ContextPlannerError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(ContextPlannerError::EmptyFragmentId);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for ContextFragmentId {
    fn deserialize<DeserializerType>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error>
    where
        DeserializerType: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

impl fmt::Display for ContextFragmentId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ContextContentHash(String);

impl ContextContentHash {
    fn new(value: impl Into<String>) -> Result<Self, ContextPlannerError> {
        let value = value.into();
        if value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            Ok(Self(value))
        } else {
            Err(ContextPlannerError::InvalidContentHash)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Computes the expected final Fragment content identity before any
    /// cache lookup. This is the same owner used by `ContextFragment` itself.
    pub fn for_fragment(
        content: &ContextFragmentContent,
        artifact_reference: Option<&ArtifactId>,
    ) -> Result<Self, ContextPlannerError> {
        Self::new(hash(&(content, artifact_reference))?)
    }
}

impl<'de> Deserialize<'de> for ContextContentHash {
    fn deserialize<DeserializerType>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error>
    where
        DeserializerType: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// The frozen twelve-slot logical request order.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextFragmentSource {
    BasePrompt,
    PersonalPreferences,
    ProjectPrompt,
    ModePrompt,
    ResearchWorldState,
    ProjectSnapshot,
    ProjectMemoryCore,
    RetrievedMemory,
    SessionSummary,
    RecentHistory,
    UserInput,
    TurnLocalUpdates,
}

impl ContextFragmentSource {
    pub(super) fn slot(self) -> u8 {
        match self {
            Self::BasePrompt => 1,
            Self::PersonalPreferences => 2,
            Self::ProjectPrompt => 3,
            Self::ModePrompt => 4,
            Self::ResearchWorldState => 5,
            Self::ProjectSnapshot => 6,
            Self::ProjectMemoryCore => 7,
            Self::RetrievedMemory => 8,
            Self::SessionSummary => 9,
            Self::RecentHistory => 10,
            Self::UserInput => 11,
            Self::TurnLocalUpdates => 12,
        }
    }

    fn is_instruction_slot(self) -> bool {
        self.slot() <= 4
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextFragmentPriority {
    Low,
    Normal,
    High,
    Required,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FragmentCachePolicy {
    ContentAddressed,
    DoNotCache,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContextFragmentContent {
    Instructions { text: String },
    ContentItems { items: Vec<ContentItem> },
}

impl ContextFragmentContent {
    fn is_instructions(&self) -> bool {
        matches!(self, Self::Instructions { .. })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ContextFragment {
    pub(super) id: ContextFragmentId,
    pub(super) source: ContextFragmentSource,
    pub(super) priority: ContextFragmentPriority,
    pub(super) cache_policy: FragmentCachePolicy,
    pub(super) content_hash: ContextContentHash,
    pub(super) token_estimate: u32,
    pub(super) content: ContextFragmentContent,
    pub(super) artifact_reference: Option<ArtifactId>,
}

impl ContextFragment {
    pub fn new(
        id: ContextFragmentId,
        source: ContextFragmentSource,
        priority: ContextFragmentPriority,
        cache_policy: FragmentCachePolicy,
        content: ContextFragmentContent,
    ) -> Result<Self, ContextPlannerError> {
        Self::with_artifact_reference(id, source, priority, cache_policy, content, None)
    }

    pub fn with_artifact_reference(
        id: ContextFragmentId,
        source: ContextFragmentSource,
        priority: ContextFragmentPriority,
        cache_policy: FragmentCachePolicy,
        content: ContextFragmentContent,
        artifact_reference: Option<ArtifactId>,
    ) -> Result<Self, ContextPlannerError> {
        if source.is_instruction_slot() != content.is_instructions() {
            return Err(ContextPlannerError::InvalidFragmentChannel {
                fragment_source: source,
            });
        }
        if source.is_instruction_slot() && artifact_reference.is_some() {
            return Err(ContextPlannerError::InstructionArtifactReference {
                fragment_source: source,
            });
        }
        let content_hash = ContextContentHash::for_fragment(&content, artifact_reference.as_ref())?;
        let token_estimate = estimator::estimate_fragment_tokens(&content)?;
        Ok(Self {
            id,
            source,
            priority,
            cache_policy,
            content_hash,
            token_estimate,
            content,
            artifact_reference,
        })
    }

    pub fn id(&self) -> &ContextFragmentId {
        &self.id
    }

    pub const fn source(&self) -> ContextFragmentSource {
        self.source
    }

    pub const fn priority(&self) -> ContextFragmentPriority {
        self.priority
    }

    pub const fn cache_policy(&self) -> FragmentCachePolicy {
        self.cache_policy
    }

    pub fn content_hash(&self) -> &ContextContentHash {
        &self.content_hash
    }

    pub const fn token_estimate(&self) -> u32 {
        self.token_estimate
    }

    pub fn content(&self) -> &ContextFragmentContent {
        &self.content
    }

    pub fn artifact_reference(&self) -> Option<&ArtifactId> {
        self.artifact_reference.as_ref()
    }
}
