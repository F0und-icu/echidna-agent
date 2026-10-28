//! Validated provider-neutral Context fragment inputs.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};

use crate::domain::artifacts::ArtifactId;
use crate::domain::model::ContentItem;

use super::delivery_map::ContextProvenance;
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

    pub(super) fn for_value(value: &impl Serialize) -> Result<Self, ContextPlannerError> {
        Self::new(hash(value)?)
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
    /// Historical M3/M4-S1 audit identity. New requests use DomainPrompt.
    ModePrompt,
    DomainPrompt,
    PhasePrompt,
    ResearchWorldState,
    ProjectSnapshot,
    ProjectMemoryCore,
    RetrievedMemory,
    SessionSummary,
    RecentHistory,
    UserInput,
    TurnLocalUpdates,
}

/// Placement alone controls the stable request order. Semantic provenance is
/// represented independently by [`ContextOrigin`].
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextSlot {
    BaseInstructions,
    PersonalInstructions,
    ProjectInstructions,
    DomainInstructions,
    PhaseInstructions,
    ResearchState,
    ProjectState,
    MemoryCore,
    RetrievedContext,
    Summary,
    History,
    UserInput,
    TurnUpdates,
}

impl ContextSlot {
    pub(super) const fn order(self) -> u8 {
        match self {
            Self::BaseInstructions => 1,
            Self::PersonalInstructions => 2,
            Self::ProjectInstructions => 3,
            Self::DomainInstructions => 4,
            Self::PhaseInstructions => 5,
            Self::ResearchState => 6,
            Self::ProjectState => 7,
            Self::MemoryCore => 8,
            Self::RetrievedContext => 9,
            Self::Summary => 10,
            Self::History => 11,
            Self::UserInput => 12,
            Self::TurnUpdates => 13,
        }
    }

    const fn is_instruction(self) -> bool {
        self.order() <= 5
    }
}

/// Semantic producer identity, deliberately independent from placement.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextOrigin {
    PromptResource,
    OperatorInput,
    ProjectFile,
    ProjectMemory,
    ResearchProjection,
    SessionProjection,
    ArtifactProjection,
    ToolResult,
    ModelOutput,
    ReplayFixture,
}

impl ContextFragmentSource {
    pub const fn placement(self) -> ContextSlot {
        match self {
            Self::BasePrompt => ContextSlot::BaseInstructions,
            Self::PersonalPreferences => ContextSlot::PersonalInstructions,
            Self::ProjectPrompt => ContextSlot::ProjectInstructions,
            Self::ModePrompt | Self::DomainPrompt => ContextSlot::DomainInstructions,
            Self::PhasePrompt => ContextSlot::PhaseInstructions,
            Self::ResearchWorldState => ContextSlot::ResearchState,
            Self::ProjectSnapshot => ContextSlot::ProjectState,
            Self::ProjectMemoryCore => ContextSlot::MemoryCore,
            Self::RetrievedMemory => ContextSlot::RetrievedContext,
            Self::SessionSummary => ContextSlot::Summary,
            Self::RecentHistory => ContextSlot::History,
            Self::UserInput => ContextSlot::UserInput,
            Self::TurnLocalUpdates => ContextSlot::TurnUpdates,
        }
    }

    pub const fn origin(self) -> ContextOrigin {
        match self {
            Self::BasePrompt | Self::ModePrompt | Self::DomainPrompt | Self::PhasePrompt => {
                ContextOrigin::PromptResource
            }
            Self::PersonalPreferences | Self::UserInput => ContextOrigin::OperatorInput,
            Self::ProjectPrompt | Self::ProjectSnapshot => ContextOrigin::ProjectFile,
            Self::ProjectMemoryCore | Self::RetrievedMemory => ContextOrigin::ProjectMemory,
            Self::ResearchWorldState => ContextOrigin::ResearchProjection,
            Self::SessionSummary | Self::RecentHistory | Self::TurnLocalUpdates => {
                ContextOrigin::SessionProjection
            }
        }
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
    #[serde(skip)]
    pub(super) slot: ContextSlot,
    #[serde(skip)]
    pub(super) origin: ContextOrigin,
    #[serde(skip)]
    pub(super) provenance: ContextProvenance,
    #[serde(skip)]
    pub(super) content_item_provenance: BTreeMap<u32, ContextProvenance>,
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
        let slot = source.placement();
        let origin = source.origin();
        if slot.is_instruction() != content.is_instructions() {
            return Err(ContextPlannerError::InvalidFragmentChannel {
                fragment_source: source,
            });
        }
        if slot.is_instruction() && artifact_reference.is_some() {
            return Err(ContextPlannerError::InstructionArtifactReference {
                fragment_source: source,
            });
        }
        let content_hash = ContextContentHash::for_fragment(&content, artifact_reference.as_ref())?;
        let token_estimate = estimator::estimate_fragment_tokens(&content)?;
        let provenance = ContextProvenance::for_fragment_parts(
            source,
            slot,
            origin,
            &content_hash,
            artifact_reference.as_ref(),
        )?;
        Ok(Self {
            id,
            source,
            slot,
            origin,
            provenance,
            content_item_provenance: BTreeMap::new(),
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

    pub const fn slot(&self) -> ContextSlot {
        self.slot
    }

    pub const fn origin(&self) -> ContextOrigin {
        self.origin
    }

    pub fn provenance(&self) -> &ContextProvenance {
        &self.provenance
    }

    pub fn with_provenance(
        mut self,
        provenance: ContextProvenance,
    ) -> Result<Self, ContextPlannerError> {
        if provenance.slot() != self.slot {
            return Err(ContextPlannerError::ContextProvenanceSlotMismatch);
        }
        self.provenance = provenance;
        Ok(self)
    }

    pub(crate) fn with_content_item_provenance(
        mut self,
        ordinal: u32,
        provenance: ContextProvenance,
    ) -> Result<Self, ContextPlannerError> {
        if provenance.slot() != self.slot {
            return Err(ContextPlannerError::ContextProvenanceSlotMismatch);
        }
        let ContextFragmentContent::ContentItems { items } = &self.content else {
            return Err(ContextPlannerError::InvalidFragmentChannel {
                fragment_source: self.source,
            });
        };
        if usize::try_from(ordinal)
            .ok()
            .is_none_or(|index| index >= items.len())
        {
            return Err(ContextPlannerError::ContextSubitemOrdinalOutOfBounds);
        }
        if self
            .content_item_provenance
            .insert(ordinal, provenance)
            .is_some()
        {
            return Err(ContextPlannerError::DuplicateContextSubitemProvenance);
        }
        Ok(self)
    }

    pub(crate) fn content_item_provenance(&self, ordinal: u32) -> Option<&ContextProvenance> {
        self.content_item_provenance.get(&ordinal)
    }

    pub(crate) fn reidentify(mut self, id: ContextFragmentId) -> Self {
        self.id = id;
        self
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
