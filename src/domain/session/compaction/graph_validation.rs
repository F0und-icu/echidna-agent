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
    pub(crate) narrative: NarrativeSummary,
    pub(crate) protected_inputs: Vec<ProtectedOperatorInput>,
    pub(crate) required_source_blobs: Vec<BlobId>,
    pub(crate) required_vision_sources: Vec<(
        crate::domain::vision::VisionObservation,
        Option<crate::domain::request_audit::RequestAuditManifest>,
    )>,
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
    ) -> Result<VerifiedCompactionGraph, SessionError> {
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
        let (expected_entries, expected_contents) =
            Self::source_entries(self.source(), &events[..boundary], project_events)?;
        if self.entries() != expected_entries || contents != expected_contents {
            return Err(invalid_task(
                "Compaction source plan does not exactly cover the derived source manifest",
            ));
        }
        let mut samples = Vec::new();
        let mut audits = BTreeMap::new();
        for event in &events[boundary..] {
            match SessionEvent::decode(&event.event_type, event.payload.clone())? {
                SessionEvent::RequestAuditRecorded { manifest } => {
                    audits.insert(event.event_id.clone(), manifest);
                }
                SessionEvent::CompactionSampleRecorded { sample } => {
                    samples.push((event.event_id.clone(), sample))
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
        Ok(VerifiedCompactionGraph {
            source: self.source().clone(),
            plan_blob_id: plan_blob_id.clone(),
            plan_hash: self.hash()?,
            root_event_id: root_id.clone(),
            narrative: root.narrative().clone(),
            protected_inputs: state.protected_operator_inputs()?,
            required_source_blobs: super::source_plan::required_source_blobs(
                &state,
                project_events,
            )?,
            required_vision_sources: state
                .protected_vision_sources()?
                .into_iter()
                .map(|(observation, audit)| (observation.clone(), audit.cloned()))
                .collect(),
        })
    }
}
