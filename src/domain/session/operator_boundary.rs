//! Durable operator-input identity over the existing History facts.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::domain::context::{
    AuthorityLevel, ContextOrigin, ContextProvenance, ContextReferences, ContextSlot,
    FreshnessIdentity, RetentionClass, SemanticClass,
};
use crate::domain::events::{EventId, EventSequence};
use crate::domain::model::{ContentItem, MessageRole};

use super::{SessionError, SessionState};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OperatorInputRef {
    event_id: EventId,
    content_hash: String,
    provenance: ContextProvenance,
}

/// An accepted input references the original History facts rather than copying them.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum OperatorTurnBoundary {
    Accepted {
        schema_version: u16,
        anchor: EventId,
        inputs: Vec<OperatorInputRef>,
    },
    Completed {
        schema_version: u16,
        anchor: EventId,
        final_audit_event_id: EventId,
        history_cutoff_event_id: EventId,
        closed_input_anchors: Vec<EventId>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProtectedOperatorInput {
    pub(crate) anchor: EventId,
    pub(crate) event_id: EventId,
    pub(crate) item: ContentItem,
    pub(crate) provenance: ContextProvenance,
    legacy_vision_audit: Option<EventId>,
}

#[derive(Clone, Default)]
pub(crate) struct ProtectedOperatorContext {
    pub(crate) detached_items: Vec<ContentItem>,
    pub(crate) detached_provenance: BTreeMap<usize, ContextProvenance>,
    pub(crate) retained_provenance: BTreeMap<usize, ContextProvenance>,
}

#[derive(Clone)]
pub(crate) struct FollowupVisionInput {
    pub(crate) event_id: EventId,
    pub(crate) provenance: ContextProvenance,
    pub(crate) observation_blob: Option<crate::domain::artifacts::BlobId>,
}

impl OperatorTurnBoundary {
    pub(crate) fn input_provenance(
        &self,
    ) -> Result<BTreeMap<usize, ContextProvenance>, SessionError> {
        let Self::Accepted { inputs, .. } = self else {
            return Err(invalid(
                "completed boundary has no accepted input provenance",
            ));
        };
        inputs
            .iter()
            .enumerate()
            .map(|(index, input)| {
                let p = &input.provenance;
                let refs = p.references();
                let mut event_refs = refs.source_event_refs().to_vec();
                if !event_refs.contains(&input.event_id) {
                    event_refs.push(input.event_id.clone());
                }
                let references = ContextReferences::new(
                    refs.research_fact_refs().to_vec(),
                    refs.artifact_refs().to_vec(),
                    refs.execution_receipt_refs().to_vec(),
                    event_refs,
                )
                .map_err(|error| invalid(error.to_string()))?;
                let provenance = ContextProvenance::new(
                    p.slot(),
                    p.origin(),
                    p.semantic_class(),
                    p.authority_level(),
                    p.retention_class(),
                    p.freshness().clone(),
                    references,
                )
                .map_err(|error| invalid(error.to_string()))?;
                Ok((index, provenance))
            })
            .collect()
    }
}

fn invalid(message: impl Into<String>) -> SessionError {
    SessionError::CompactionContextProvenance {
        message: message.into(),
    }
}

impl SessionState {
    /// Minimum exact input retained by every future compaction candidate.
    pub(crate) fn minimum_protected_operator_context(
        &self,
    ) -> Result<ProtectedOperatorContext, SessionError> {
        let inputs = self.protected_operator_inputs()?;
        Ok(ProtectedOperatorContext {
            detached_items: inputs.iter().map(|input| input.item.clone()).collect(),
            detached_provenance: inputs
                .into_iter()
                .enumerate()
                .map(|(index, input)| (index, input.provenance))
                .collect(),
            retained_provenance: BTreeMap::new(),
        })
    }
    /// Identify Vision Tool follow-ups from their actual paired `view_image`
    /// result, typed route and, for Secondary, durable Observation and audit.
    pub(crate) fn followup_vision_inputs(&self) -> Result<Vec<FollowupVisionInput>, SessionError> {
        use crate::domain::profile::VisionRoute;
        use crate::domain::vision::ViewImageSelection;
        let accepted = self
            .operator_boundaries
            .iter()
            .filter_map(|boundary| match boundary {
                OperatorTurnBoundary::Accepted { inputs, .. } => Some(inputs.as_slice()),
                _ => None,
            })
            .flatten()
            .map(|input| input.event_id.clone())
            .collect::<BTreeSet<_>>();
        let history = self.history_items_with_event_ids().collect::<Vec<_>>();
        let mut start = 0usize;
        let mut followups = Vec::new();
        for (index, (event_id, item)) in history.iter().enumerate() {
            if !matches!(
                item,
                ContentItem::Text {
                    role: MessageRole::User,
                    ..
                } | ContentItem::ImageArtifact {
                    role: MessageRole::User,
                    ..
                }
            ) {
                continue;
            }
            if accepted.contains(*event_id) {
                start = index + 1;
                continue;
            }
            let history_sequence = self.history_event_sequences[index];
            let mut matches = Vec::new();
            let mut selection_seen = false;
            for (result_index, (_, result_item)) in
                history.iter().enumerate().take(index).skip(start)
            {
                let ContentItem::ToolResult { result } = result_item else {
                    continue;
                };
                let Some(selection) = ViewImageSelection::from_tool_result(result)
                    .map_err(|error| invalid(error.to_string()))?
                else {
                    continue;
                };
                let relevant = match item {
                    ContentItem::ImageArtifact { artifact_id, .. } => {
                        artifact_id == &selection.delivery_artifact_id
                    }
                    ContentItem::Text { text, .. } => {
                        self.vision_observations
                            .iter()
                            .try_fold(false, |found, observation| {
                                let value = observation
                                    .primary_context_text()
                                    .map_err(|error| invalid(error.to_string()))?;
                                Ok::<_, SessionError>(
                                    found
                                        || observation.source_artifact_id
                                            == selection.source_artifact_id
                                            && &value == text,
                                )
                            })?
                    }
                    _ => false,
                };
                if !relevant {
                    continue;
                }
                selection_seen = true;
                let execution = crate::domain::tools::ExecutionId::for_model_call(&result.call_id);
                if !self.execution_receipts.iter().any(|record| {
                    record.receipt.execution_id == execution
                        && record.receipt.status == result.status
                        && record.result.as_ref().is_none_or(|actual| actual == result)
                }) {
                    return Err(invalid(
                        "Vision follow-up has no matching durable Tool receipt",
                    ));
                }
                if result_index < self.recovered_history_item_count
                    && index >= self.recovered_history_item_count
                {
                    continue;
                }
                for (route, route_sequence) in
                    self.vision_routes.iter().zip(&self.vision_route_sequences)
                {
                    if !route
                        .source_artifact_ids
                        .contains(&selection.source_artifact_id)
                        || self.history_event_sequences[result_index] >= *route_sequence
                        || *route_sequence >= history_sequence
                    {
                        continue;
                    }
                    match item {
                        ContentItem::ImageArtifact { artifact_id, .. }
                            if route.route == VisionRoute::PrimaryDirect
                                && artifact_id == &selection.delivery_artifact_id =>
                        {
                            if self.primary_audit_between(*route_sequence, history_sequence) {
                                continue;
                            }
                            let provenance = crate::domain::vision::VisionRouteRecord::primary_direct_provenance(selection.source_artifact_id.clone(), selection.delivery_artifact_id.clone(), route.route_event_id.clone()).map_err(|error| invalid(error.to_string()))?;
                            matches.push((provenance, None));
                        }
                        ContentItem::Text { text, .. }
                            if route.route == VisionRoute::SecondaryTool =>
                        {
                            for ((observation_id, observation_sequence), observation) in self
                                .vision_observation_event_ids
                                .iter()
                                .zip(&self.vision_observations)
                            {
                                if observation.route_event_id != route.route_event_id
                                    || observation.source_artifact_id
                                        != selection.source_artifact_id
                                    || observation.derived_artifact_ids
                                        != selection.derived_artifact_ids
                                    || observation.image_geometry.as_ref().is_some_and(|geometry| {
                                        geometry.delivery_artifact_id
                                            != selection.delivery_artifact_id
                                    })
                                    || observation
                                        .primary_context_text()
                                        .map_err(|error| invalid(error.to_string()))?
                                        != *text
                                    || *observation_sequence >= history_sequence
                                {
                                    continue;
                                }
                                let audit_sequence = self
                                    .request_audits
                                    .iter()
                                    .zip(&self.request_audit_sequences)
                                    .find_map(|(audit, sequence)| {
                                        (audit.audit_event_id()
                                            == &observation.request_audit_event_id)
                                            .then_some(*sequence)
                                    })
                                    .ok_or_else(|| invalid("Vision follow-up audit is absent"))?;
                                if *route_sequence >= audit_sequence
                                    || audit_sequence >= *observation_sequence
                                    || self.primary_audit_between(
                                        *observation_sequence,
                                        history_sequence,
                                    )
                                {
                                    continue;
                                }
                                let provenance = observation
                                    .primary_context_provenance(observation_id.clone())
                                    .map_err(|error| invalid(error.to_string()))?;
                                matches.push((
                                    provenance,
                                    Some(observation.raw_observation_blob_id.clone()),
                                ));
                            }
                        }
                        _ => {}
                    }
                }
            }
            if matches.len() == 1 {
                let (provenance, observation_blob) = matches.pop().expect("one matched follow-up");
                followups.push(FollowupVisionInput {
                    event_id: (*event_id).clone(),
                    provenance,
                    observation_blob,
                });
            } else if selection_seen {
                return Err(invalid(
                    "Vision follow-up source is ambiguous or inconsistent",
                ));
            } else {
                start = index + 1;
            }
        }
        Ok(followups)
    }

    fn primary_audit_between(&self, start: EventSequence, end: EventSequence) -> bool {
        self.request_audits
            .iter()
            .zip(&self.request_audit_sequences)
            .any(|(audit, sequence)| {
                *sequence > start
                    && *sequence < end
                    && audit.provider_session().is_some_and(|provider| {
                        provider.role == crate::domain::context::ProviderSessionRole::Primary
                    })
            })
    }

    pub(crate) fn legacy_vision_audit_ids(&self) -> Result<Vec<EventId>, SessionError> {
        Ok(self
            .protected_operator_inputs()?
            .into_iter()
            .filter_map(|input| input.legacy_vision_audit)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect())
    }

    fn legacy_vision_provenance(
        &self,
        event_id: &EventId,
        item: &ContentItem,
    ) -> Result<(ContextProvenance, EventId), SessionError> {
        use crate::domain::context::ContextTransform;
        use crate::domain::request_audit::RequestAuditComponentIdentity;
        if self
            .history
            .items()
            .iter()
            .filter(|candidate| *candidate == item)
            .count()
            != 1
        {
            return Err(invalid("legacy Vision input identity is ambiguous"));
        }
        let history_index = self
            .history_event_ids
            .iter()
            .position(|id| id == event_id)
            .ok_or_else(|| invalid("legacy Vision History event is absent"))?;
        let history_sequence = self.history_event_sequences[history_index];
        let content_hash = super::compaction::fingerprint_value(item)?;
        for (audit, sequence) in self
            .request_audits
            .iter()
            .zip(&self.request_audit_sequences)
        {
            if *sequence <= history_sequence {
                continue;
            }
            let Some(map) = audit.context_delivery_map() else {
                continue;
            };
            for entry in map.content_items() {
                let p = entry.provenance();
                if entry.transform() != &ContextTransform::Identity
                    || entry.content_hash().as_str() != content_hash
                    || !audit.components().iter().any(|component| {
                        component.identity()
                            == &RequestAuditComponentIdentity::ContentItem(entry.final_position())
                            && component.semantic_hash().as_str() == content_hash
                    })
                {
                    continue;
                }
                let references = p.references();
                let valid_source = match item {
                    ContentItem::ImageArtifact { artifact_id, .. } => {
                        p.origin() == ContextOrigin::ArtifactProjection
                            && references.artifact_refs().contains(artifact_id)
                            && self.vision_routes.iter().any(|route| {
                                references
                                    .source_event_refs()
                                    .contains(&route.route_event_id)
                                    && route
                                        .source_artifact_ids
                                        .iter()
                                        .any(|id| references.artifact_refs().contains(id))
                            })
                    }
                    ContentItem::Text { text, .. } => {
                        let mut found = false;
                        for ((id, _), observation) in self
                            .vision_observation_event_ids
                            .iter()
                            .zip(&self.vision_observations)
                        {
                            let expected = observation
                                .primary_context_text()
                                .map_err(|error| invalid(error.to_string()))?;
                            found |= references.source_event_refs().contains(id)
                                && references
                                    .source_event_refs()
                                    .contains(&observation.route_event_id)
                                && references
                                    .source_event_refs()
                                    .contains(&observation.request_audit_event_id)
                                && &expected == text;
                        }
                        p.semantic_class() == SemanticClass::ModelObservation && found
                    }
                    _ => false,
                };
                if !valid_source {
                    continue;
                }
                let mut event_refs = references.source_event_refs().to_vec();
                for id in [event_id, audit.audit_event_id()] {
                    if !event_refs.contains(id) {
                        event_refs.push(id.clone());
                    }
                }
                let refs = ContextReferences::new(
                    references.research_fact_refs().to_vec(),
                    references.artifact_refs().to_vec(),
                    references.execution_receipt_refs().to_vec(),
                    event_refs,
                )
                .map_err(|error| invalid(error.to_string()))?;
                let provenance = ContextProvenance::new(
                    p.slot(),
                    p.origin(),
                    p.semantic_class(),
                    p.authority_level(),
                    p.retention_class(),
                    p.freshness().clone(),
                    refs,
                )
                .map_err(|error| invalid(error.to_string()))?;
                return Ok((provenance, audit.audit_event_id().clone()));
            }
        }
        Err(invalid(
            "legacy Vision input has no uniquely attributable audited delivery",
        ))
    }

    /// Recover a checkpointed initial image before its accepted boundary or
    /// first Primary audit exists. The standalone intake Receipt fixes the
    /// original/derivative pair; an Observation fixes its own route and audit.
    fn unaccepted_vision_provenance(
        &self,
        event_id: &EventId,
        item: &ContentItem,
    ) -> Result<ContextProvenance, SessionError> {
        use crate::domain::profile::VisionRoute;
        use crate::domain::tools::ToolResultStatus;

        let index = self
            .history_event_ids
            .iter()
            .position(|id| id == event_id)
            .ok_or_else(|| invalid("Vision History event is absent"))?;
        let history_sequence = self.history_event_sequences[index];
        let mut candidates = Vec::new();
        for (route, route_sequence) in self.vision_routes.iter().zip(&self.vision_route_sequences) {
            if *route_sequence >= history_sequence || route.source_artifact_ids.len() != 1 {
                continue;
            }
            let source = &route.source_artifact_ids[0];
            match item {
                ContentItem::ImageArtifact { artifact_id, .. }
                    if route.route == VisionRoute::PrimaryDirect =>
                {
                    let expected = if source == artifact_id {
                        vec![source.clone()]
                    } else {
                        vec![source.clone(), artifact_id.clone()]
                    };
                    let receipts = self
                        .execution_receipts
                        .iter()
                        .filter(|record| {
                            record.sequence < *route_sequence
                                && record.result.is_none()
                                && record.receipt.status == ToolResultStatus::Succeeded
                                && record.receipt.output_artifact_ids == expected
                        })
                        .count();
                    if receipts == 1 {
                        candidates.push(
                            crate::domain::vision::VisionRouteRecord::primary_direct_provenance(
                                source.clone(),
                                artifact_id.clone(),
                                route.route_event_id.clone(),
                            )
                            .map_err(|error| invalid(error.to_string()))?,
                        );
                    }
                }
                ContentItem::Text { text, .. } if route.route == VisionRoute::SecondaryTool => {
                    for ((observation_id, observation_sequence), observation) in self
                        .vision_observation_event_ids
                        .iter()
                        .zip(&self.vision_observations)
                    {
                        if observation.route_event_id != route.route_event_id
                            || &observation.source_artifact_id != source
                            || *observation_sequence >= history_sequence
                            || observation
                                .primary_context_text()
                                .map_err(|error| invalid(error.to_string()))?
                                != *text
                        {
                            continue;
                        }
                        let audit_sequence = self
                            .request_audits
                            .iter()
                            .zip(&self.request_audit_sequences)
                            .find_map(|(audit, sequence)| {
                                (audit.audit_event_id() == &observation.request_audit_event_id)
                                    .then_some(*sequence)
                            })
                            .ok_or_else(|| invalid("Vision observation audit is absent"))?;
                        if *route_sequence < audit_sequence
                            && audit_sequence < *observation_sequence
                        {
                            candidates.push(
                                observation
                                    .primary_context_provenance(observation_id.clone())
                                    .map_err(|error| invalid(error.to_string()))?,
                            );
                        }
                    }
                }
                _ => {}
            }
        }
        if candidates.len() != 1 {
            return Err(invalid(
                "unaccepted Vision input source is ambiguous or absent",
            ));
        }
        Ok(candidates.pop().expect("one Vision input source"))
    }

    fn recover_vision_provenance(
        &self,
        event_id: &EventId,
        item: &ContentItem,
    ) -> Result<(ContextProvenance, Option<EventId>), SessionError> {
        match self.unaccepted_vision_provenance(event_id, item) {
            Ok(provenance) => Ok((provenance, None)),
            Err(facts_error) => self.legacy_vision_provenance(event_id, item)
                .map(|(provenance, audit)| (provenance, Some(audit)))
                .map_err(|audit_error| invalid(format!("Vision source facts failed: {facts_error}; audited delivery failed: {audit_error}"))),
        }
    }

    pub(crate) fn protected_vision_blobs(
        &self,
    ) -> Result<Vec<crate::domain::artifacts::BlobId>, SessionError> {
        Ok(self
            .protected_vision_sources()?
            .into_iter()
            .map(|(observation, _)| observation.raw_observation_blob_id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect())
    }

    pub(crate) fn protected_vision_sources(
        &self,
    ) -> Result<
        Vec<(
            &crate::domain::vision::VisionObservation,
            Option<&crate::domain::request_audit::RequestAuditManifest>,
        )>,
        SessionError,
    > {
        let mut inputs: BTreeMap<_, _> = self
            .protected_operator_inputs()?
            .into_iter()
            .map(|input| (input.event_id, input.provenance))
            .collect();
        let positions = self.current_user_event_positions()?;
        for input in self.followup_vision_inputs()? {
            if positions.contains_key(&input.event_id) {
                inputs.insert(input.event_id, input.provenance);
            }
        }
        let mut observations = BTreeMap::new();
        for (event_id, provenance) in inputs {
            if provenance.semantic_class() != SemanticClass::ModelObservation {
                continue;
            }
            let references = provenance.references();
            let matches = self
                .vision_observation_event_ids
                .iter()
                .zip(&self.vision_observations)
                .filter(|((id, _), _)| references.source_event_refs().contains(id))
                .collect::<Vec<_>>();
            if matches.len() != 1 {
                return Err(invalid("protected Vision observation source is ambiguous"));
            }
            let ((observation_id, _), observation) = matches[0];
            let expected = observation
                .primary_context_text()
                .map_err(|error| invalid(error.to_string()))?;
            let item = self
                .history_items_with_event_ids()
                .find_map(|(id, item)| (id == &event_id).then_some(item))
                .ok_or_else(|| invalid("protected Vision History is absent"))?;
            if item != &ContentItem::user_text(expected)
                || !references
                    .source_event_refs()
                    .contains(&observation.route_event_id)
                || !references
                    .source_event_refs()
                    .contains(&observation.request_audit_event_id)
                || !references
                    .artifact_refs()
                    .contains(&observation.source_artifact_id)
            {
                return Err(invalid("protected Vision observation provenance mismatch"));
            }
            let audit = self
                .request_audits
                .iter()
                .find(|audit| audit.audit_event_id() == &observation.request_audit_event_id);
            observations.insert(observation_id.clone(), (observation, audit));
        }
        Ok(observations.into_values().collect())
    }

    pub(crate) fn retained_followup_artifacts(
        &self,
    ) -> Result<Vec<crate::domain::artifacts::ArtifactId>, SessionError> {
        let positions = self.current_user_event_positions()?;
        let mut artifacts = BTreeSet::new();
        for input in self.followup_vision_inputs()? {
            if positions.contains_key(&input.event_id) {
                artifacts.extend(
                    input
                        .provenance
                        .references()
                        .artifact_refs()
                        .iter()
                        .cloned(),
                );
            }
        }
        Ok(artifacts.into_iter().collect())
    }

    pub(crate) fn latest_uncompleted_batch_items(&self) -> Result<usize, SessionError> {
        let full = self.full_normalized_history()?;
        let cutoff = self
            .operator_boundaries
            .iter()
            .rev()
            .find_map(|boundary| match boundary {
                OperatorTurnBoundary::Completed {
                    history_cutoff_event_id,
                    ..
                } => Some(history_cutoff_event_id),
                _ => None,
            });
        let after_completed = if let Some(cutoff) = cutoff {
            let index = self
                .history_event_ids
                .iter()
                .position(|id| id == cutoff)
                .ok_or_else(|| invalid("completed History cutoff is absent"))?;
            self.normalize_items(&self.history.items()[..=index])?
                .len()
                .min(full.len())
        } else {
            0
        };
        Ok(crate::domain::history::latest_complete_tool_batch_items(
            &full[after_completed..],
        )?)
    }

    pub(crate) fn compaction_source_history(&self) -> Result<Vec<ContentItem>, SessionError> {
        let full = self.full_normalized_history()?;
        let active = self.normalized_history_items()?;
        let start = full
            .len()
            .checked_sub(active.len())
            .ok_or_else(|| invalid("active History is not a normalized suffix"))?;
        if full[start..] != active {
            return Err(invalid("active History is not a normalized suffix"));
        }
        let recovered = match self.recovered_history_start {
            Some(start) => full
                .len()
                .checked_sub(start)
                .ok_or_else(|| invalid("recovered History boundary is invalid"))?,
            None => 0,
        };
        let keep = active
            .len()
            .max(self.latest_uncompleted_batch_items()?)
            .max(recovered);
        Ok(full[full.len() - keep..].to_vec())
    }

    pub(super) fn capture_recovered_history_start(&mut self) -> Result<(), SessionError> {
        let full_len = self.full_normalized_history()?.len();
        let restored_len = self.compaction_source_history()?.len();
        self.recovered_history_start = Some(full_len - restored_len);
        Ok(())
    }

    pub(crate) fn verify_completed_operator_boundary(
        &self,
        boundary: &OperatorTurnBoundary,
        verified: &crate::domain::request_audit::VerifiedRequestAuditInput,
        delivered: &BTreeMap<EventId, crate::domain::model::ModelInputIR>,
    ) -> Result<(), SessionError> {
        validate_completed(self, boundary)?;
        let OperatorTurnBoundary::Completed {
            anchor,
            final_audit_event_id,
            closed_input_anchors,
            ..
        } = boundary
        else {
            return Err(invalid("expected completed boundary"));
        };
        let audit = self
            .request_audits
            .last()
            .filter(|audit| audit.audit_event_id() == final_audit_event_id)
            .ok_or_else(|| invalid("completed boundary audit is absent"))?;
        if completion_anchors(self, audit, Some((verified, delivered)))?
            != Some((anchor.clone(), closed_input_anchors.clone()))
        {
            return Err(invalid(
                "completed boundary failed verified audit reconstruction",
            ));
        }
        Ok(())
    }

    pub(crate) fn completed_operator_turn(
        &self,
        verified: &crate::domain::request_audit::VerifiedRequestAuditInput,
        delivered: &BTreeMap<EventId, crate::domain::model::ModelInputIR>,
    ) -> Result<Option<OperatorTurnBoundary>, SessionError> {
        let Some(audit) = self
            .request_audits
            .last()
            .filter(|audit| audit.audit_event_id() == verified.audit_event_id())
        else {
            return Err(invalid(
                "completed boundary does not reference the latest audited request",
            ));
        };
        let Some((anchor, closed)) = completion_anchors(self, audit, Some((verified, delivered)))?
        else {
            return Ok(None);
        };
        let cutoff = self
            .history_event_ids
            .last()
            .ok_or_else(|| invalid("completed boundary has no final History"))?
            .clone();
        Ok(Some(OperatorTurnBoundary::Completed {
            schema_version: 1,
            anchor,
            final_audit_event_id: audit.audit_event_id().clone(),
            history_cutoff_event_id: cutoff,
            closed_input_anchors: closed,
        }))
    }

    /// Reconstruct input facts from the original History; legacy inputs remain
    /// unclassified until their original delivery provenance can be verified.
    pub(crate) fn protected_operator_inputs(
        &self,
    ) -> Result<Vec<ProtectedOperatorInput>, SessionError> {
        let followup_ids = self
            .followup_vision_inputs()?
            .into_iter()
            .map(|input| input.event_id)
            .collect::<BTreeSet<_>>();
        let mut accepted = BTreeMap::new();
        let closed = self
            .operator_boundaries
            .iter()
            .filter_map(|boundary| match boundary {
                OperatorTurnBoundary::Completed {
                    closed_input_anchors,
                    ..
                } => Some(closed_input_anchors.as_slice()),
                _ => None,
            })
            .flatten()
            .cloned()
            .collect::<BTreeSet<_>>();
        let mut all_accepted = BTreeSet::new();
        for boundary in &self.operator_boundaries {
            let OperatorTurnBoundary::Accepted { anchor, inputs, .. } = boundary else {
                continue;
            };
            all_accepted.extend(inputs.iter().map(|input| input.event_id.clone()));
            if closed.contains(anchor) {
                continue;
            }
            let provenance = boundary.input_provenance()?;
            for (index, input) in inputs.iter().enumerate() {
                accepted.insert(
                    input.event_id.clone(),
                    (anchor.clone(), provenance[&index].clone()),
                );
            }
        }
        let mut protected = Vec::new();
        for (event_id, item) in self.history_items_with_event_ids() {
            if let Some((anchor, provenance)) = accepted.get(event_id) {
                protected.push(ProtectedOperatorInput {
                    anchor: anchor.clone(),
                    event_id: event_id.clone(),
                    item: item.clone(),
                    provenance: provenance.clone(),
                    legacy_vision_audit: None,
                });
            } else if all_accepted.contains(event_id) || followup_ids.contains(event_id) {
                continue;
            } else if matches!(
                item,
                ContentItem::Text {
                    role: MessageRole::User,
                    ..
                }
            ) {
                let observation =
                    self.vision_observations
                        .iter()
                        .try_fold(false, |found, observation| {
                            let text = observation
                                .primary_context_text()
                                .map_err(|error| invalid(error.to_string()))?;
                            Ok::<_, SessionError>(found || &ContentItem::user_text(text) == item)
                        })?;
                if observation {
                    let (provenance, audit) = self.recover_vision_provenance(event_id, item)?;
                    protected.push(ProtectedOperatorInput {
                        anchor: event_id.clone(),
                        event_id: event_id.clone(),
                        item: item.clone(),
                        provenance,
                        legacy_vision_audit: audit,
                    });
                    continue;
                }
                if let ContentItem::Text { text, .. } = item
                    && serde_json::from_str::<serde_json::Value>(text)
                        .ok()
                        .and_then(|value| {
                            value
                                .get("kind")
                                .and_then(serde_json::Value::as_str)
                                .map(str::to_owned)
                        })
                        .as_deref()
                        == Some("vision_observation")
                {
                    return Err(invalid(
                        "Vision observation History has no matching durable observation",
                    ));
                }
                let references = ContextReferences::new(
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    vec![event_id.clone()],
                )
                .map_err(|error| invalid(error.to_string()))?;
                let provenance = ContextProvenance::new(
                    ContextSlot::UserInput,
                    ContextOrigin::SessionProjection,
                    SemanticClass::Conversation,
                    AuthorityLevel::UnclassifiedLegacy,
                    RetentionClass::Recent,
                    FreshnessIdentity::new(super::compaction::fingerprint_value(item)?)
                        .map_err(|error| invalid(error.to_string()))?
                        .with_producer_version("legacy-operator-input:1"),
                    references,
                )
                .map_err(|error| invalid(error.to_string()))?;
                protected.push(ProtectedOperatorInput {
                    anchor: event_id.clone(),
                    event_id: event_id.clone(),
                    item: item.clone(),
                    provenance,
                    legacy_vision_audit: None,
                });
            } else if matches!(
                item,
                ContentItem::ImageArtifact {
                    role: MessageRole::User,
                    ..
                }
            ) {
                let (provenance, audit) = self.recover_vision_provenance(event_id, item)?;
                protected.push(ProtectedOperatorInput {
                    anchor: event_id.clone(),
                    event_id: event_id.clone(),
                    item: item.clone(),
                    provenance,
                    legacy_vision_audit: audit,
                });
            }
        }
        Ok(protected)
    }

    pub(crate) fn protected_operator_inputs_for(
        &self,
        candidate: &super::SessionCompaction,
    ) -> Result<Vec<ProtectedOperatorInput>, SessionError> {
        let normalized = self.full_normalized_history()?;
        let cutoff = candidate.retained_from_normalized_history_index();
        if candidate.source_history_item_count() != self.history.items().len()
            || cutoff > normalized.len()
            || candidate.retained_history() != &normalized[cutoff..]
        {
            return Err(invalid(
                "protected input candidate does not match the original History",
            ));
        }
        let durable = self
            .history_items_with_event_ids()
            .filter(|(_, item)| {
                matches!(
                    item,
                    ContentItem::Text {
                        role: MessageRole::User,
                        ..
                    } | ContentItem::ImageArtifact {
                        role: MessageRole::User,
                        ..
                    }
                )
            })
            .collect::<Vec<_>>();
        let normalized_user = normalized
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                matches!(
                    item,
                    ContentItem::Text {
                        role: MessageRole::User,
                        ..
                    } | ContentItem::ImageArtifact {
                        role: MessageRole::User,
                        ..
                    }
                )
            })
            .collect::<Vec<_>>();
        if durable.len() != normalized_user.len()
            || durable
                .iter()
                .zip(&normalized_user)
                .any(|((_, item), (_, normalized))| *item != *normalized)
        {
            return Err(invalid("normalized operator input identity mismatch"));
        }
        let retained = durable
            .iter()
            .zip(normalized_user)
            .filter_map(|((id, _), (position, _))| (position >= cutoff).then_some((*id).clone()))
            .collect::<BTreeSet<_>>();
        Ok(self
            .protected_operator_inputs()?
            .into_iter()
            .filter(|input| !retained.contains(&input.event_id))
            .collect())
    }

    pub(crate) fn protected_operator_context_for(
        &self,
        candidate: &super::SessionCompaction,
    ) -> Result<ProtectedOperatorContext, SessionError> {
        let all = self.protected_operator_inputs()?;
        let detached = self.protected_operator_inputs_for(candidate)?;
        let normalized = self.full_normalized_history()?;
        let cutoff = candidate.retained_from_normalized_history_index();
        let originals = self.history_items_with_event_ids().filter(|(_, item)| {
            matches!(
                item,
                ContentItem::Text {
                    role: MessageRole::User,
                    ..
                } | ContentItem::ImageArtifact {
                    role: MessageRole::User,
                    ..
                }
            )
        });
        let normalized_users = normalized.iter().enumerate().filter(|(_, item)| {
            matches!(
                item,
                ContentItem::Text {
                    role: MessageRole::User,
                    ..
                } | ContentItem::ImageArtifact {
                    role: MessageRole::User,
                    ..
                }
            )
        });
        let positions = originals
            .zip(normalized_users)
            .filter(|(_, (position, _))| *position >= cutoff)
            .map(|((id, _), (position, _))| (id.clone(), position - cutoff))
            .collect::<BTreeMap<_, _>>();
        let mut retained_provenance: BTreeMap<usize, ContextProvenance> = all
            .iter()
            .filter_map(|input| {
                positions
                    .get(&input.event_id)
                    .map(|position| (*position, input.provenance.clone()))
            })
            .collect();
        for followup in self.followup_vision_inputs()? {
            if let Some(position) = positions.get(&followup.event_id) {
                retained_provenance.insert(*position, followup.provenance);
            }
        }
        let detached_items = detached.iter().map(|input| input.item.clone()).collect();
        let detached_provenance = detached
            .into_iter()
            .enumerate()
            .map(|(index, input)| (index, input.provenance))
            .collect();
        Ok(ProtectedOperatorContext {
            detached_items,
            detached_provenance,
            retained_provenance,
        })
    }

    pub(crate) fn current_protected_operator_context(
        &self,
    ) -> Result<ProtectedOperatorContext, SessionError> {
        let protected = self.protected_operator_inputs()?;
        let positions = self.current_user_event_positions()?;
        let mut detached_items = Vec::new();
        let mut detached_provenance = BTreeMap::new();
        let mut retained_provenance = BTreeMap::new();
        for input in protected {
            if let Some(position) = positions.get(&input.event_id) {
                retained_provenance.insert(*position, input.provenance);
            } else {
                detached_provenance.insert(detached_items.len(), input.provenance);
                detached_items.push(input.item);
            }
        }
        for followup in self.followup_vision_inputs()? {
            if let Some(position) = positions.get(&followup.event_id) {
                retained_provenance.insert(*position, followup.provenance);
            }
        }
        Ok(ProtectedOperatorContext {
            detached_items,
            detached_provenance,
            retained_provenance,
        })
    }

    fn current_user_event_positions(&self) -> Result<BTreeMap<EventId, usize>, SessionError> {
        let history = self.compaction_source_history()?;
        let original = self
            .history_items_with_event_ids()
            .filter(|(_, item)| {
                matches!(
                    item,
                    ContentItem::Text {
                        role: MessageRole::User,
                        ..
                    } | ContentItem::ImageArtifact {
                        role: MessageRole::User,
                        ..
                    }
                )
            })
            .collect::<Vec<_>>();
        let current = history
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                matches!(
                    item,
                    ContentItem::Text {
                        role: MessageRole::User,
                        ..
                    } | ContentItem::ImageArtifact {
                        role: MessageRole::User,
                        ..
                    }
                )
            })
            .collect::<Vec<_>>();
        let skip = original
            .len()
            .checked_sub(current.len())
            .ok_or_else(|| invalid("current operator input sequence is invalid"))?;
        if original[skip..]
            .iter()
            .zip(&current)
            .any(|((_, item), (_, actual))| *item != *actual)
        {
            return Err(invalid("current operator input identity mismatch"));
        }
        Ok(original[skip..]
            .iter()
            .zip(current)
            .map(|((id, _), (position, _))| ((*id).clone(), position))
            .collect())
    }

    pub(crate) fn accepted_operator_input(
        &self,
        items: &[ContentItem],
        provenance: &BTreeMap<usize, ContextProvenance>,
    ) -> Result<OperatorTurnBoundary, SessionError> {
        let start = self
            .history
            .items()
            .len()
            .checked_sub(items.len())
            .ok_or_else(|| invalid("operator input is not the durable History tail"))?;
        if items.is_empty()
            || self.history.items()[start..] != *items
            || provenance.len() != items.len()
        {
            return Err(invalid(
                "operator input is not the exact durable History tail",
            ));
        }
        let inputs = items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let provenance = provenance
                    .get(&index)
                    .ok_or_else(|| invalid("operator input provenance is incomplete"))?
                    .clone();
                Ok(OperatorInputRef {
                    event_id: self.history_event_ids[start + index].clone(),
                    content_hash: super::compaction::fingerprint_value(item)?,
                    provenance,
                })
            })
            .collect::<Result<Vec<_>, SessionError>>()?;
        let boundary = OperatorTurnBoundary::Accepted {
            schema_version: 1,
            anchor: inputs[0].event_id.clone(),
            inputs,
        };
        validate_accepted(
            self,
            &boundary,
            self.stream_id
                .as_ref()
                .and_then(|id| self.event_state.last_sequence_for(id)),
        )?;
        Ok(boundary)
    }
}

fn validate_completed(
    state: &SessionState,
    boundary: &OperatorTurnBoundary,
) -> Result<(), SessionError> {
    let OperatorTurnBoundary::Completed {
        schema_version,
        anchor,
        final_audit_event_id,
        history_cutoff_event_id,
        closed_input_anchors,
    } = boundary
    else {
        return Err(invalid("expected completed boundary"));
    };
    let audit = state
        .request_audits
        .last()
        .filter(|audit| audit.audit_event_id() == final_audit_event_id)
        .ok_or_else(|| invalid("completed boundary references an absent or stale audit"))?;
    let expected = completion_anchors(state, audit, None)?;
    if *schema_version != 1
        || state.history_event_ids.last() != Some(history_cutoff_event_id)
        || expected
            .as_ref()
            .is_none_or(|(expected_anchor, expected_closed)| {
                expected_anchor != anchor || expected_closed != closed_input_anchors
            })
    {
        return Err(invalid(
            "completed boundary does not match its audited inputs and final History",
        ));
    }
    Ok(())
}

fn completion_anchors(
    state: &SessionState,
    audit: &crate::domain::request_audit::RequestAuditManifest,
    verified: Option<(
        &crate::domain::request_audit::VerifiedRequestAuditInput,
        &BTreeMap<EventId, crate::domain::model::ModelInputIR>,
    )>,
) -> Result<Option<(EventId, Vec<EventId>)>, SessionError> {
    use crate::domain::context::ContextTransform;
    state.ensure_completed_history()?;
    if audit.provider_session() != state.primary_provider_session.as_ref()
        || state.rejected_requests.contains(audit.audit_event_id())
        || state.history.items().len() <= state.latest_audit_history_len
        || !matches!(
            state.history.items().last(),
            Some(ContentItem::Text {
                role: MessageRole::Assistant,
                ..
            })
        )
    {
        return Ok(None);
    }
    let Some(map) = audit.full_context_delivery_map() else {
        return Ok(None);
    };
    let Some(chain) = state.model_delivery_chain(audit.audit_event_id())? else {
        return Ok(None);
    };
    if verified.is_some_and(|(proof, deliveries)| {
        proof.audit_event_id() != audit.audit_event_id()
            || proof.delivery_map() != map
            || chain
                .iter()
                .any(|delivery| !deliveries.contains_key(delivery.audit_event_id()))
    }) {
        return Err(invalid(
            "completed audit proof does not match the recorded full map",
        ));
    }
    let closed = state
        .operator_boundaries
        .iter()
        .filter_map(|boundary| match boundary {
            OperatorTurnBoundary::Completed {
                closed_input_anchors,
                ..
            } => Some(closed_input_anchors.as_slice()),
            _ => None,
        })
        .flatten()
        .collect::<BTreeSet<_>>();
    let mut open = Vec::new();
    for boundary in &state.operator_boundaries {
        let OperatorTurnBoundary::Accepted { anchor, inputs, .. } = boundary else {
            continue;
        };
        if closed.contains(anchor) {
            continue;
        }
        let mut complete = true;
        for input in inputs {
            let original = state
                .history_items_with_event_ids()
                .find(|(id, _)| *id == &input.event_id)
                .ok_or_else(|| invalid("accepted input source is absent"))?
                .1;
            let matches = map
                .content_items()
                .iter()
                .filter(|entry| {
                    entry.transform() == &ContextTransform::Identity
                        && entry.content_hash().as_str() == input.content_hash
                        && entry
                            .provenance()
                            .references()
                            .source_event_refs()
                            .contains(&input.event_id)
                        && verified.is_none_or(|(proof, _)| {
                            proof
                                .input()
                                .content_items
                                .get(entry.final_position() as usize)
                                == Some(original)
                        })
                })
                .count();
            let delivered_in_chain = chain.iter().any(|delivery| {
                let Some(actual_audit) = state.request_audits.iter().find(|candidate| candidate.audit_event_id() == delivery.audit_event_id()) else { return false; };
                let Some(actual_map) = actual_audit.context_delivery_map() else { return false; };
                actual_map.content_items().iter().any(|entry| entry.transform() == &ContextTransform::Identity
                    && entry.content_hash().as_str() == input.content_hash
                    && entry.provenance().references().source_event_refs().contains(&input.event_id)
                    && actual_audit.components().iter().any(|component| component.identity() == &crate::domain::request_audit::RequestAuditComponentIdentity::ContentItem(entry.final_position()) && component.semantic_hash().as_str() == input.content_hash)
                    && verified.is_none_or(|(_, proofs)| proofs.get(delivery.audit_event_id()).and_then(|value| value.content_items.get(entry.final_position() as usize)) == Some(original)))
            });
            if matches != 1 || !delivered_in_chain {
                complete = false;
                break;
            }
        }
        open.push((anchor.clone(), complete));
    }
    let Some((anchor, true)) = open.last() else {
        return Ok(None);
    };
    let anchor = anchor.clone();
    Ok(Some((
        anchor,
        open.into_iter()
            .filter_map(|(id, complete)| complete.then_some(id))
            .collect(),
    )))
}

pub(super) fn apply(
    state: &mut SessionState,
    boundary: OperatorTurnBoundary,
    previous_sequence: Option<EventSequence>,
) -> Result<(), SessionError> {
    match &boundary {
        OperatorTurnBoundary::Accepted { .. } => {
            validate_accepted(state, &boundary, previous_sequence)?
        }
        OperatorTurnBoundary::Completed { .. } => validate_completed(state, &boundary)?,
    }
    state.operator_boundaries.push(boundary);
    Ok(())
}

fn validate_accepted(
    state: &SessionState,
    boundary: &OperatorTurnBoundary,
    previous_sequence: Option<EventSequence>,
) -> Result<(), SessionError> {
    let OperatorTurnBoundary::Accepted {
        schema_version,
        anchor,
        inputs,
    } = boundary
    else {
        return Err(invalid("expected accepted operator boundary"));
    };
    let start = state
        .history
        .items()
        .len()
        .checked_sub(inputs.len())
        .ok_or_else(|| invalid("operator input references missing History"))?;
    if *schema_version != 1
        || inputs.is_empty()
        || anchor != &inputs[0].event_id
        || state.operator_boundaries.iter().any(|old| match old {
            OperatorTurnBoundary::Accepted {
                inputs: previous, ..
            } => previous
                .iter()
                .any(|item| inputs.iter().any(|input| input.event_id == item.event_id)),
            OperatorTurnBoundary::Completed { .. } => false,
        })
    {
        return Err(invalid("invalid or duplicate accepted operator input"));
    }
    let mut ids = BTreeSet::new();
    for (offset, input) in inputs.iter().enumerate() {
        let index = start + offset;
        let item = &state.history.items()[index];
        let sequence = state.history_event_sequences[index].get();
        if !ids.insert(&input.event_id)
            || input.event_id != state.history_event_ids[index]
            || input.content_hash != super::compaction::fingerprint_value(item)?
            || !matches!(
                item,
                ContentItem::Text {
                    role: MessageRole::User,
                    ..
                } | ContentItem::ImageArtifact {
                    role: MessageRole::User,
                    ..
                }
            )
            || previous_sequence.is_none_or(|last| {
                sequence.checked_add((inputs.len() - 1 - offset) as u64) != Some(last.get())
            })
        {
            return Err(invalid(
                "accepted operator input does not match its ordered History facts",
            ));
        }
        let p = &input.provenance;
        ContextProvenance::new(
            p.slot(),
            p.origin(),
            p.semantic_class(),
            p.authority_level(),
            p.retention_class(),
            p.freshness().clone(),
            p.references().clone(),
        )
        .map_err(|error| invalid(error.to_string()))?;
        match item {
            ContentItem::Text { text, .. } if offset == 0 => {
                if *p
                    != ContextProvenance::operator_input(text)
                        .map_err(|error| invalid(error.to_string()))?
                {
                    return Err(invalid(
                        "operator text provenance does not match its original input",
                    ));
                }
            }
            ContentItem::ImageArtifact { artifact_id, .. } => {
                let refs = p.references();
                if p.origin() != ContextOrigin::ArtifactProjection
                    || !refs.artifact_refs().contains(artifact_id)
                    || !state.vision_routes.iter().any(|route| {
                        refs.source_event_refs().contains(&route.route_event_id)
                            && route
                                .source_artifact_ids
                                .iter()
                                .any(|id| refs.artifact_refs().contains(id))
                    })
                {
                    return Err(invalid(
                        "operator image provenance does not match a durable Vision route",
                    ));
                }
            }
            ContentItem::Text { text, .. } => {
                let refs = p.references();
                let mut matches = 0;
                for ((observation_id, _), observation) in state
                    .vision_observation_event_ids
                    .iter()
                    .zip(&state.vision_observations)
                {
                    let expected = observation
                        .primary_context_text()
                        .map_err(|error| invalid(error.to_string()))?;
                    if &expected == text
                        && refs.source_event_refs().contains(observation_id)
                        && refs
                            .source_event_refs()
                            .contains(&observation.route_event_id)
                        && refs
                            .source_event_refs()
                            .contains(&observation.request_audit_event_id)
                        && refs
                            .artifact_refs()
                            .contains(&observation.source_artifact_id)
                    {
                        matches += 1;
                    }
                }
                if p.origin() != ContextOrigin::ModelOutput
                    || p.semantic_class() != SemanticClass::ModelObservation
                    || matches != 1
                {
                    return Err(invalid(
                        "operator Vision observation provenance does not match its durable source",
                    ));
                }
            }
            _ => unreachable!("input kinds validated above"),
        }
    }
    Ok(())
}
