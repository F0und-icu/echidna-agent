//! Orthogonal Context provenance vocabulary and validation.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::domain::artifacts::ArtifactId;
use crate::domain::events::EventId;
use crate::domain::model::{ContentItem, MessageRole};
use crate::domain::research::{ExecutionReceiptId, ResearchFactRef, TargetSnapshotId};

use super::super::error::ContextPlannerError;
use super::super::fingerprint::hash;
use super::super::fragment::{
    ContextContentHash, ContextFragment, ContextFragmentId, ContextFragmentSource, ContextOrigin,
    ContextSlot,
};

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ContextSubitemId(String);

impl ContextSubitemId {
    pub(super) fn for_ordinal(
        fragment_id: &ContextFragmentId,
        ordinal: u32,
    ) -> Result<Self, ContextPlannerError> {
        hash(&(fragment_id, ordinal)).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(super) fn validate(
        &self,
        fragment_id: &ContextFragmentId,
        ordinal: u32,
    ) -> Result<(), ContextPlannerError> {
        if *self == Self::for_ordinal(fragment_id, ordinal)? {
            Ok(())
        } else {
            Err(ContextPlannerError::InvalidContextSubitemId)
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticClass {
    Instruction,
    AuthoritativeReference,
    DerivedProjection,
    ModelObservation,
    Conversation,
    Narrative,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorityLevel {
    FactReference,
    OperatorAssertion,
    DerivedNonEvidence,
    ModelObservation,
    UnclassifiedLegacy,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RetentionClass {
    EvidenceSpine,
    Stable,
    Recent,
    Ephemeral,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FreshnessIdentity {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    target_snapshot_id: Option<TargetSnapshotId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    project_fingerprint: Option<String>,
    source_revision_or_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    producer_version: Option<String>,
}

impl FreshnessIdentity {
    pub fn new(source_revision_or_hash: impl Into<String>) -> Result<Self, ContextPlannerError> {
        let identity = Self {
            target_snapshot_id: None,
            project_fingerprint: None,
            source_revision_or_hash: source_revision_or_hash.into(),
            producer_version: None,
        };
        identity.validate()?;
        Ok(identity)
    }

    pub fn with_target_snapshot(mut self, id: TargetSnapshotId) -> Self {
        self.target_snapshot_id = Some(id);
        self
    }

    pub fn with_project_fingerprint(mut self, fingerprint: impl Into<String>) -> Self {
        self.project_fingerprint = Some(fingerprint.into());
        self
    }

    pub fn with_producer_version(mut self, version: impl Into<String>) -> Self {
        self.producer_version = Some(version.into());
        self
    }

    pub fn target_snapshot_id(&self) -> Option<&TargetSnapshotId> {
        self.target_snapshot_id.as_ref()
    }

    pub fn project_fingerprint(&self) -> Option<&str> {
        self.project_fingerprint.as_deref()
    }

    pub fn source_revision_or_hash(&self) -> &str {
        &self.source_revision_or_hash
    }

    pub fn producer_version(&self) -> Option<&str> {
        self.producer_version.as_deref()
    }

    fn validate(&self) -> Result<(), ContextPlannerError> {
        require_nonempty(
            "freshness.source_revision_or_hash",
            &self.source_revision_or_hash,
        )?;
        if let Some(id) = &self.target_snapshot_id {
            require_nonempty("freshness.target_snapshot_id", id.as_str())?;
        }
        if let Some(value) = &self.project_fingerprint {
            require_nonempty("freshness.project_fingerprint", value)?;
        }
        if let Some(value) = &self.producer_version {
            require_nonempty("freshness.producer_version", value)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContextReferences {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    research_fact_refs: Vec<ResearchFactRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    artifact_refs: Vec<ArtifactId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    execution_receipt_refs: Vec<ExecutionReceiptId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    source_event_refs: Vec<EventId>,
}

impl ContextReferences {
    pub fn new(
        research_fact_refs: Vec<ResearchFactRef>,
        artifact_refs: Vec<ArtifactId>,
        execution_receipt_refs: Vec<ExecutionReceiptId>,
        source_event_refs: Vec<EventId>,
    ) -> Result<Self, ContextPlannerError> {
        let references = Self {
            research_fact_refs,
            artifact_refs,
            execution_receipt_refs,
            source_event_refs,
        };
        references.validate()?;
        Ok(references)
    }

    pub fn research_fact_refs(&self) -> &[ResearchFactRef] {
        &self.research_fact_refs
    }

    pub fn artifact_refs(&self) -> &[ArtifactId] {
        &self.artifact_refs
    }

    pub fn execution_receipt_refs(&self) -> &[ExecutionReceiptId] {
        &self.execution_receipt_refs
    }

    pub fn source_event_refs(&self) -> &[EventId] {
        &self.source_event_refs
    }

    pub fn is_empty(&self) -> bool {
        self.research_fact_refs.is_empty()
            && self.artifact_refs.is_empty()
            && self.execution_receipt_refs.is_empty()
            && self.source_event_refs.is_empty()
    }

    fn validate(&self) -> Result<(), ContextPlannerError> {
        let mut research_refs = BTreeSet::new();
        for reference in &self.research_fact_refs {
            require_nonempty("references.research_fact_refs", reference.id())?;
            if !research_refs.insert(reference) {
                return Err(ContextPlannerError::DuplicateContextReference {
                    field: "references.research_fact_refs",
                    value: reference.id().to_owned(),
                });
            }
        }
        validate_unique(
            "references.artifact_refs",
            self.artifact_refs.iter().map(ArtifactId::as_str),
        )?;
        validate_unique(
            "references.execution_receipt_refs",
            self.execution_receipt_refs
                .iter()
                .map(ExecutionReceiptId::as_str),
        )?;
        validate_unique(
            "references.source_event_refs",
            self.source_event_refs.iter().map(EventId::as_str),
        )
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContextProvenance {
    slot: ContextSlot,
    origin: ContextOrigin,
    semantic_class: SemanticClass,
    authority_level: AuthorityLevel,
    retention_class: RetentionClass,
    freshness: FreshnessIdentity,
    references: ContextReferences,
}

impl ContextProvenance {
    pub fn new(
        slot: ContextSlot,
        origin: ContextOrigin,
        semantic_class: SemanticClass,
        authority_level: AuthorityLevel,
        retention_class: RetentionClass,
        freshness: FreshnessIdentity,
        references: ContextReferences,
    ) -> Result<Self, ContextPlannerError> {
        let provenance = Self {
            slot,
            origin,
            semantic_class,
            authority_level,
            retention_class,
            freshness,
            references,
        };
        provenance.validate()?;
        Ok(provenance)
    }

    pub const fn origin(&self) -> ContextOrigin {
        self.origin
    }

    pub const fn slot(&self) -> ContextSlot {
        self.slot
    }

    pub const fn semantic_class(&self) -> SemanticClass {
        self.semantic_class
    }

    pub const fn authority_level(&self) -> AuthorityLevel {
        self.authority_level
    }

    pub const fn retention_class(&self) -> RetentionClass {
        self.retention_class
    }

    pub fn freshness(&self) -> &FreshnessIdentity {
        &self.freshness
    }

    pub fn references(&self) -> &ContextReferences {
        &self.references
    }

    pub(crate) fn repositioned(&self, slot: ContextSlot) -> Self {
        let mut provenance = self.clone();
        provenance.slot = slot;
        provenance
    }

    pub(crate) fn operator_input(text: &str) -> Result<Self, ContextPlannerError> {
        Self::new(
            ContextSlot::UserInput,
            ContextOrigin::OperatorInput,
            SemanticClass::Conversation,
            AuthorityLevel::OperatorAssertion,
            RetentionClass::Recent,
            FreshnessIdentity::new(hash(&text)?)?.with_producer_version("agent-loop:1"),
            ContextReferences::default(),
        )
    }

    pub(in crate::domain::context::planner) fn for_fragment_parts(
        source: ContextFragmentSource,
        slot: ContextSlot,
        origin: ContextOrigin,
        content_hash: &ContextContentHash,
        artifact_reference: Option<&ArtifactId>,
    ) -> Result<Self, ContextPlannerError> {
        let references = artifact_reference.map_or_else(ContextReferences::default, |artifact| {
            ContextReferences {
                artifact_refs: vec![artifact.clone()],
                ..ContextReferences::default()
            }
        });
        let (semantic_class, authority_level, retention_class) =
            default_semantics(source, !references.is_empty());
        Self::new(
            slot,
            origin,
            semantic_class,
            authority_level,
            retention_class,
            FreshnessIdentity::new(content_hash.as_str())?
                .with_producer_version("context-planner:2"),
            references,
        )
    }

    pub(super) fn for_content_item(
        fragment: &ContextFragment,
        ordinal: u32,
        item: &ContentItem,
    ) -> Result<Self, ContextPlannerError> {
        if let Some(provenance) = fragment.content_item_provenance(ordinal) {
            return Ok(provenance.clone());
        }
        if !matches!(
            fragment.source(),
            ContextFragmentSource::UserInput
                | ContextFragmentSource::RecentHistory
                | ContextFragmentSource::TurnLocalUpdates
        ) {
            return Ok(fragment.provenance().clone());
        }
        let freshness = FreshnessIdentity::new(fragment.content_hash().as_str())?
            .with_producer_version("context-planner:2");
        match item {
            ContentItem::Text {
                role: MessageRole::User,
                ..
            } if fragment.source() == ContextFragmentSource::UserInput => Self::new(
                fragment.slot(),
                ContextOrigin::OperatorInput,
                SemanticClass::Conversation,
                AuthorityLevel::OperatorAssertion,
                RetentionClass::Recent,
                freshness,
                ContextReferences::default(),
            ),
            ContentItem::Text {
                role: MessageRole::User,
                ..
            } => Self::new(
                fragment.slot(),
                ContextOrigin::SessionProjection,
                SemanticClass::Conversation,
                AuthorityLevel::UnclassifiedLegacy,
                RetentionClass::Recent,
                freshness,
                ContextReferences::default(),
            ),
            ContentItem::ImageArtifact { artifact_id, .. } => Self::new(
                fragment.slot(),
                ContextOrigin::ArtifactProjection,
                SemanticClass::AuthoritativeReference,
                AuthorityLevel::FactReference,
                RetentionClass::Stable,
                freshness,
                ContextReferences {
                    artifact_refs: vec![artifact_id.clone()],
                    ..ContextReferences::default()
                },
            ),
            ContentItem::Text {
                role: MessageRole::Assistant,
                ..
            }
            | ContentItem::ToolCall { .. }
            | ContentItem::Reasoning { .. } => Self::new(
                fragment.slot(),
                ContextOrigin::ModelOutput,
                SemanticClass::ModelObservation,
                AuthorityLevel::ModelObservation,
                RetentionClass::Recent,
                freshness,
                ContextReferences::default(),
            ),
            ContentItem::ToolResult { .. } => Self::new(
                fragment.slot(),
                ContextOrigin::ToolResult,
                SemanticClass::DerivedProjection,
                AuthorityLevel::DerivedNonEvidence,
                RetentionClass::Recent,
                freshness,
                ContextReferences::default(),
            ),
            ContentItem::Unsupported { .. } => Self::new(
                fragment.slot(),
                ContextOrigin::SessionProjection,
                SemanticClass::Conversation,
                AuthorityLevel::UnclassifiedLegacy,
                RetentionClass::Recent,
                freshness,
                ContextReferences::default(),
            ),
        }
    }

    pub(super) fn model_output(
        source_revision_or_hash: impl Into<String>,
    ) -> Result<Self, ContextPlannerError> {
        Self::new(
            ContextSlot::History,
            ContextOrigin::ModelOutput,
            SemanticClass::ModelObservation,
            AuthorityLevel::ModelObservation,
            RetentionClass::Ephemeral,
            FreshnessIdentity::new(source_revision_or_hash)?.with_producer_version("agent-loop:1"),
            ContextReferences::default(),
        )
    }

    pub(super) fn validate(&self) -> Result<(), ContextPlannerError> {
        self.freshness.validate()?;
        self.references.validate()?;
        if self.authority_level == AuthorityLevel::FactReference && self.references.is_empty() {
            return Err(ContextPlannerError::FactReferenceMissingContextReference);
        }
        if self.authority_level == AuthorityLevel::ModelObservation
            && self.semantic_class != SemanticClass::ModelObservation
        {
            return Err(ContextPlannerError::InvalidContextAuthorityCombination);
        }
        if self.semantic_class == SemanticClass::ModelObservation
            && self.authority_level != AuthorityLevel::ModelObservation
        {
            return Err(ContextPlannerError::InvalidContextAuthorityCombination);
        }
        Ok(())
    }
}

fn default_semantics(
    source: ContextFragmentSource,
    has_fact_reference: bool,
) -> (SemanticClass, AuthorityLevel, RetentionClass) {
    if has_fact_reference {
        return (
            SemanticClass::AuthoritativeReference,
            AuthorityLevel::FactReference,
            RetentionClass::Stable,
        );
    }
    match source {
        ContextFragmentSource::BasePrompt
        | ContextFragmentSource::PersonalPreferences
        | ContextFragmentSource::ProjectPrompt
        | ContextFragmentSource::ModePrompt
        | ContextFragmentSource::DomainPrompt
        | ContextFragmentSource::PhasePrompt => (
            SemanticClass::Instruction,
            AuthorityLevel::DerivedNonEvidence,
            RetentionClass::Stable,
        ),
        ContextFragmentSource::ResearchWorldState
        | ContextFragmentSource::ProjectSnapshot
        | ContextFragmentSource::ProjectMemoryCore
        | ContextFragmentSource::RetrievedMemory => (
            SemanticClass::DerivedProjection,
            AuthorityLevel::DerivedNonEvidence,
            RetentionClass::Stable,
        ),
        ContextFragmentSource::SessionSummary => (
            SemanticClass::Narrative,
            AuthorityLevel::UnclassifiedLegacy,
            RetentionClass::Stable,
        ),
        ContextFragmentSource::RecentHistory | ContextFragmentSource::TurnLocalUpdates => (
            SemanticClass::Conversation,
            AuthorityLevel::UnclassifiedLegacy,
            RetentionClass::Recent,
        ),
        ContextFragmentSource::UserInput => (
            SemanticClass::Conversation,
            AuthorityLevel::OperatorAssertion,
            RetentionClass::Recent,
        ),
    }
}

fn require_nonempty(field: &'static str, value: &str) -> Result<(), ContextPlannerError> {
    if value.trim().is_empty() {
        Err(ContextPlannerError::ContextProvenanceFieldRequired { field })
    } else {
        Ok(())
    }
}

fn validate_unique<'a>(
    field: &'static str,
    values: impl IntoIterator<Item = &'a str>,
) -> Result<(), ContextPlannerError> {
    let mut seen = BTreeSet::new();
    for value in values {
        require_nonempty(field, value)?;
        if !seen.insert(value) {
            return Err(ContextPlannerError::DuplicateContextReference {
                field,
                value: value.to_owned(),
            });
        }
    }
    Ok(())
}
