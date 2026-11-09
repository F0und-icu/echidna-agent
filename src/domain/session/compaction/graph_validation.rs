//! Validate the real audited requests and complete source graph before adoption.

use std::collections::BTreeMap;

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::domain::artifacts::BlobId;
use crate::domain::events::{EventEnvelope, EventId};
use crate::domain::model::{ContentItem, ModelInputIR};
use crate::domain::request_audit::{RequestAuditDeliveryMode, VerifiedRequestAuditInput};
use crate::domain::session::{SessionError, SessionEvent};

use super::super::operator_boundary::ProtectedOperatorInput;
use super::{CompactionSourcePlan, NarrativeSummary, SessionSourceIdentity, task::invalid_task};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VerifiedCompactionGraph {
    pub(crate) source: SessionSourceIdentity,
    pub(crate) plan_blob_id: BlobId,
    pub(crate) plan_hash: String,
    pub(crate) root_event_id: EventId,
    pub(crate) task_selection_event: Option<EventId>,
    pub(crate) task_id: Option<String>,
    pub(crate) task_scope: Vec<String>,
    pub(crate) summarized_history: Vec<EventId>,
    pub(crate) task_history_refs: Vec<EventId>,
    pub(crate) task_receipt_refs: Vec<crate::domain::research::ExecutionReceiptId>,
    pub(crate) task_artifact_refs: Vec<crate::domain::artifacts::ArtifactId>,
    pub(super) proof6: bool,
    pub(super) reference_limit: usize,
    pub(super) reference_source: Option<SessionSourceIdentity>,
    pub(super) task_definition: Option<super::task::DefinitionProvenance>,
    ancestry_history: Vec<EventId>,
    ancestry_receipts: Vec<crate::domain::research::ExecutionReceiptId>,
    ancestry_artifacts: Vec<crate::domain::artifacts::ArtifactId>,
    ancestry_summarized: Vec<EventId>,
    pub(crate) narrative: NarrativeSummary,
    pub(crate) protected_inputs: Vec<ProtectedOperatorInput>,
    pub(crate) required_source_blobs: Vec<BlobId>,
    pub(crate) required_vision_sources: Vec<(
        crate::domain::vision::VisionObservation,
        Option<crate::domain::request_audit::RequestAuditManifest>,
    )>,
}

impl VerifiedCompactionGraph {
    pub(super) fn project_task_references(
        &self,
        limit: usize,
    ) -> (
        Vec<crate::domain::artifacts::ArtifactId>,
        Vec<crate::domain::research::ExecutionReceiptId>,
        Vec<EventId>,
    ) {
        (
            self.task_artifact_refs
                .iter()
                .take(limit)
                .cloned()
                .collect(),
            self.task_receipt_refs
                .iter()
                .rev()
                .take(limit)
                .cloned()
                .collect(),
            self.task_history_refs
                .iter()
                .rev()
                .take(limit)
                .cloned()
                .collect(),
        )
    }
}

impl CompactionSourcePlan {
    /// Each proof was reconstructed from actual audit Blobs by the App. The
    /// expected IR is rebuilt from the fixed node renderer and sampling owner.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn verify_model_node_audits(
        &self,
        plan_blob_id: &BlobId,
        contents: &[String],
        events: &[EventEnvelope<Value>],
        project_events: &[EventEnvelope<Value>],
        proofs: &BTreeMap<EventId, VerifiedRequestAuditInput>,
        expected_inputs: &BTreeMap<
            String,
            (ModelInputIR, crate::domain::context::ContextDeliveryMap),
        >,
        capabilities: &crate::domain::model::ModelCapabilities,
        task_fragments: Option<&[crate::domain::context::ContextFragment]>,
        reference_limit: usize,
        prior_graph: Option<&VerifiedCompactionGraph>,
    ) -> Result<VerifiedCompactionGraph, SessionError> {
        if self.task_selection_event().is_some() != task_fragments.is_some() {
            return Err(invalid_task(
                "task Compaction fragment proof is absent or unexpected",
            ));
        }
        let expected_capabilities =
            crate::domain::request_audit::RequestAuditCapabilityMapping::from_capabilities(
                capabilities,
            )
            .map_err(|error| invalid_task(error.to_string()))?;
        let plan_bytes =
            serde_json::to_vec(self).map_err(|error| invalid_task(error.to_string()))?;
        if format!("{:x}", Sha256::digest(&plan_bytes)) != plan_blob_id.as_str() {
            return Err(invalid_task("Compaction plan Blob identity mismatch"));
        }
        let boundary = events.partition_point(|event| event.sequence <= self.source().watermark());
        let state = self.validate_source_facts(contents, &events[..boundary], project_events)?;
        let proof6 = self.task_selection_event().is_some()
            && state
                .latest_task_context_selection()
                .is_some_and(|(_, selection)| selection.request_proof_version >= 6);
        let mut samples = Vec::new();
        let mut audits = BTreeMap::new();
        for event in &events[boundary..] {
            match SessionEvent::decode(&event.event_type, event.payload.clone())? {
                SessionEvent::RequestAuditRecorded { manifest } => {
                    audits.insert(event.event_id.clone(), manifest);
                }
                SessionEvent::CompactionSampleRecorded { sample } => {
                    samples.push((event.event_id.clone(), sample));
                }
                SessionEvent::ModelUsageRecorded { .. } => {}
                _ => {
                    return Err(invalid_task(
                        "unrelated Session change during Compaction graph sampling",
                    ));
                }
            }
        }
        let Some((root_id, root)) = samples.last() else {
            return Err(invalid_task("Compaction graph has no root sample"));
        };
        self.source()
            .validate_graph_sampling_suffix(events, project_events, root_id)?;
        if samples.len() != self.nodes().len() || expected_inputs.len() != self.nodes().len() {
            return Err(invalid_task("Compaction graph sample count mismatch"));
        }
        let mut children = BTreeMap::<String, (EventId, String)>::new();
        for ((sample_id, sample), node) in samples.iter().zip(self.nodes()) {
            let (blob, hash, recorded_node, output) = sample
                .graph_node()
                .ok_or_else(|| invalid_task("Compaction graph contains a legacy sample"))?;
            if blob != plan_blob_id
                || hash != self.hash()?
                || recorded_node != node
                || sample.source() != self.source()
                || sample.provider_session() != &self.node_provider_session(&node.id)?
            {
                return Err(invalid_task(
                    "Compaction sample does not match the source plan",
                ));
            }
            let audit = audits
                .get(sample.audit_event_id())
                .ok_or_else(|| invalid_task("Compaction sample audit is absent"))?;
            let proof = proofs
                .get(sample.audit_event_id())
                .ok_or_else(|| invalid_task("Compaction sample audit Blob proof is absent"))?;
            if audit.delivery().mode() != RequestAuditDeliveryMode::Full
                || audit.capability_mapping() != &expected_capabilities
                || audit.full_logical_input() != audit.delivered_input()
                || audit.provider_session() != Some(sample.provider_session())
                || proof.audit_event_id() != sample.audit_event_id()
                || !proof.input().tool_specs.is_empty()
                || expected_inputs.get(&node.id)
                    != Some(&(proof.input().clone(), proof.delivery_map().clone()))
                || audit.full_context_delivery_map() != Some(proof.delivery_map())
            {
                return Err(invalid_task(
                    "Compaction node audit scope or full input mismatch",
                ));
            }
            let inputs = match &node.input {
                super::CompactionSourceNodeInput::Source(_) => BTreeMap::new(),
                super::CompactionSourceNodeInput::Merge(ids) => ids
                    .iter()
                    .map(|id| {
                        children
                            .get(id)
                            .cloned()
                            .map(|value| (id.clone(), value))
                            .ok_or_else(|| invalid_task("Compaction child sample is absent"))
                    })
                    .collect::<Result<BTreeMap<_, _>, _>>()?,
            };
            let rendered = ContentItem::user_text(self.render_node(&node.id, contents, &inputs)?);
            if proof
                .input()
                .content_items
                .iter()
                .filter(|item| **item == rendered)
                .count()
                != 1
            {
                return Err(invalid_task(
                    "Compaction node source is not exactly delivered",
                ));
            }
            let ContentItem::Text {
                text: source_text, ..
            } = &rendered
            else {
                unreachable!()
            };
            let actual_template =
                super::node_template_hash(proof.input(), proof.delivery_map(), source_text)?;
            if proof.input().instructions != super::compaction_instructions(node.output_limit)
                || node.template_hash.as_deref() != Some(actual_template.as_str())
            {
                return Err(invalid_task(
                    "Compaction node fixed request template mismatch",
                ));
            }
            use crate::domain::context::{
                ContextContentHash, ContextFragmentContent, ContextOrigin, ContextSlot,
                ContextTransform,
            };
            for entry in proof.delivery_map().content_items() {
                let item = &proof.input().content_items[entry.final_position() as usize];
                let provenance = entry.provenance();
                if item == &rendered {
                    if provenance.slot() != ContextSlot::UserInput
                        || provenance.origin() != ContextOrigin::SessionProjection
                        || provenance.freshness().source_revision_or_hash()
                            != self.source().stream_hash()
                    {
                        return Err(invalid_task("Compaction source provenance mismatch"));
                    }
                    continue;
                }
                if let Some(fragments) = task_fragments {
                    let exact = fragments.iter().any(|fragment| {
                        let ContextFragmentContent::ContentItems { items } = fragment.content()
                        else {
                            return false;
                        };
                        items.iter().enumerate().any(|(ordinal, candidate)| {
                            let Ok(ordinal) = u32::try_from(ordinal) else {
                                return false;
                            };
                            candidate == item
                                && fragment
                                    .content_item_provenance(ordinal)
                                    .unwrap_or_else(|| fragment.provenance())
                                    == provenance
                        })
                    });
                    if !exact || entry.transform() != &ContextTransform::Identity {
                        return Err(invalid_task(
                            "task Compaction fragment differs from its verified capture",
                        ));
                    }
                    continue;
                }
                if !matches!(
                    (provenance.slot(), provenance.origin()),
                    (
                        ContextSlot::ResearchState,
                        ContextOrigin::ResearchProjection
                    ) | (ContextSlot::ProjectState, ContextOrigin::ProjectFile)
                        | (ContextSlot::MemoryCore, ContextOrigin::ProjectMemory)
                ) || entry.transform() != &ContextTransform::Identity
                {
                    return Err(invalid_task(
                        "Compaction common snapshot provenance is not exact",
                    ));
                }
                let expected = ContextContentHash::for_fragment(
                    &ContextFragmentContent::ContentItems {
                        items: vec![item.clone()],
                    },
                    None,
                )
                .map_err(|error| invalid_task(error.to_string()))?;
                if provenance.freshness().source_revision_or_hash() != expected.as_str() {
                    return Err(invalid_task(
                        "Compaction common snapshot freshness mismatch",
                    ));
                }
            }
            children.insert(node.id.clone(), (sample_id.clone(), output.to_owned()));
        }
        let (_, _, root_node, _) = root
            .graph_node()
            .ok_or_else(|| invalid_task("Compaction root sample is invalid"))?;
        if !root_node.is_root {
            return Err(invalid_task(
                "Compaction final sample is not the planned root",
            ));
        }
        let (mut task_history_refs, mut task_receipt_refs, mut task_artifact_refs) =
            if self.task_selection_event().is_some() {
                self.task_fact_references()
            } else {
                (Vec::new(), Vec::new(), Vec::new())
            };
        let mut summarized_history = if self.task_selection_event().is_some() {
            self.entries()
                .iter()
                .filter_map(|entry| match &entry.source {
                    super::source_plan::CompactionSourceKind::History(id)
                        if !entry.protected_exact =>
                    {
                        Some(id.clone())
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        // Keep complete verified graph ancestry separately from the bounded
        // display references persisted by older Summaries.
        let mut ancestry_history = task_history_refs.clone();
        let mut ancestry_receipts = task_receipt_refs.clone();
        let mut ancestry_artifacts = task_artifact_refs.clone();
        let mut ancestry_summarized = summarized_history.clone();
        let prior_summary = state
            .latest_compaction()
            .map(|compaction| compaction.summary());
        if let Some(prior) = prior_summary
            && self.entries().iter().any(|entry| matches!(&entry.source, super::source_plan::CompactionSourceKind::Summary(id) if id == prior.fingerprint()))
            && let Some((prior_root, _, _, _)) = prior.graph_reference()
        {
            if let Some(previous) = prior_graph.filter(|graph| &graph.root_event_id == prior_root) {
                ancestry_history.extend_from_slice(&previous.ancestry_history);
                ancestry_receipts.extend_from_slice(&previous.ancestry_receipts);
                ancestry_artifacts.extend_from_slice(&previous.ancestry_artifacts);
                ancestry_summarized.extend_from_slice(&previous.ancestry_summarized);
            } else if proof6 {
                return Err(invalid_task("Compaction prior Summary graph is absent"));
            }
        }
        let history_set = ancestry_history
            .iter()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        let receipt_set = ancestry_receipts
            .iter()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        let summarized_set = ancestry_summarized
            .iter()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        ancestry_history = state
            .history_items_with_event_ids()
            .filter_map(|(id, _)| history_set.contains(id).then_some(id.clone()))
            .collect();
        ancestry_receipts = state
            .execution_receipts()
            .iter()
            .filter_map(|record| {
                let id = crate::domain::research::ExecutionReceiptId::new(record.event_id.as_str());
                receipt_set.contains(&id).then_some(id)
            })
            .collect();
        ancestry_summarized = state
            .history_items_with_event_ids()
            .filter_map(|(id, _)| summarized_set.contains(id).then_some(id.clone()))
            .collect();
        ancestry_artifacts.sort();
        ancestry_artifacts.dedup();
        if proof6
            && (ancestry_history.len() != history_set.len()
                || ancestry_receipts.len() != receipt_set.len()
                || ancestry_summarized.len() != summarized_set.len())
        {
            return Err(invalid_task(
                "Compaction graph references are absent from original event order",
            ));
        }
        let proof5 = self.task_selection_event().is_some()
            && state
                .latest_task_context_selection()
                .is_some_and(|(_, selection)| selection.request_proof_version >= 5);
        if proof5 && let Some(prior) = state.latest_compaction().map(|compaction| compaction.summary())
            && self.entries().iter().any(|entry| matches!(&entry.source, super::source_plan::CompactionSourceKind::Summary(id) if id == prior.fingerprint()))
        {
            summarized_history.extend_from_slice(prior.summarized_history());
            if let Some(refs) = prior.task_references() {
                task_history_refs.extend_from_slice(refs.history_refs());
                task_receipt_refs.extend_from_slice(refs.receipt_refs());
                task_artifact_refs.extend_from_slice(refs.artifact_refs());
            }
            summarized_history.sort(); summarized_history.dedup();
            task_history_refs.sort(); task_history_refs.dedup();
            task_receipt_refs.sort(); task_receipt_refs.dedup();
            task_artifact_refs.sort(); task_artifact_refs.dedup();
        }
        if proof6 {
            let eligible = super::source_plan::session_artifact_refs(
                project_events,
                self.source().stream_id(),
            )?;
            task_history_refs = ancestry_history.clone();
            task_receipt_refs = ancestry_receipts.clone();
            task_artifact_refs = ancestry_artifacts
                .iter()
                .filter(|id| eligible.contains(*id))
                .cloned()
                .collect();
            summarized_history = ancestry_summarized.clone();
        }
        Ok(VerifiedCompactionGraph {
            source: self.source().clone(),
            plan_blob_id: plan_blob_id.clone(),
            plan_hash: self.hash()?,
            root_event_id: root_id.clone(),
            task_selection_event: self.task_selection_event().cloned(),
            task_id: self.task_selection_event().and_then(|_| {
                state
                    .latest_task_context_selection()
                    .and_then(|(_, selection)| {
                        selection
                            .task_revisions
                            .first()
                            .map(|revision| revision.task_id.clone())
                    })
            }),
            task_scope: self
                .task_selection_event()
                .and_then(|_| state.latest_task_context_selection())
                .map(|(_, selection)| {
                    selection
                        .task_revisions
                        .iter()
                        .map(|revision| revision.task_id.clone())
                        .chain(selection.decision.related_task_refs.iter().cloned())
                        .collect::<std::collections::BTreeSet<_>>()
                        .into_iter()
                        .collect()
                })
                .unwrap_or_default(),
            summarized_history,
            task_history_refs,
            task_receipt_refs,
            task_artifact_refs,
            proof6,
            reference_limit,
            reference_source: if proof6 {
                Some(
                    SessionSourceIdentity::from_events(
                        self.source().project_id().clone(),
                        events,
                        project_events,
                    )?
                    .0,
                )
            } else {
                None
            },
            task_definition: if proof6 {
                Some(super::task::DefinitionProvenance::from_binding(
                    state
                        .agent_definition_binding()
                        .ok_or_else(|| invalid_task("task graph has no bound definition"))?,
                )?)
            } else {
                None
            },
            ancestry_history,
            ancestry_receipts,
            ancestry_artifacts,
            ancestry_summarized,
            narrative: root.narrative().clone(),
            protected_inputs: state.protected_operator_inputs()?,
            required_source_blobs: super::source_plan::required_source_blobs(
                &state,
                project_events,
            )?
            .into_iter()
            .chain(
                self.task_selection_event()
                    .into_iter()
                    .flat_map(|_| self.source_blob_references().cloned()),
            )
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect(),
            required_vision_sources: state
                .protected_vision_sources()?
                .into_iter()
                .map(|(observation, audit)| (observation.clone(), audit.cloned()))
                .collect(),
        })
    }
}
