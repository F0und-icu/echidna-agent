//! Accepted, isolated model handoff. It is derived text, never task evidence.

use serde::{Deserialize, Serialize};

use crate::domain::artifacts::BlobId;
use crate::domain::context::{ProviderSessionId, ProviderSessionRef, ProviderSessionRole};
use crate::domain::events::{EventEnvelope, EventId};
use crate::domain::session::SessionEvent;
use serde_json::Value;

use super::{
    CompactionSourceNode, CompactionSourcePlan, NarrativeSummary, SessionError,
    SessionSourceIdentity,
};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct CompactionSampleGraph {
    plan_blob_id: BlobId,
    plan_hash: String,
    node: CompactionSourceNode,
    output: String,
    retention: CompactionRetention,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum CompactionRetention {
    SingleSample,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Handoff {
    concise_progress: String,
    unresolved_questions: Vec<String>,
    next_step_notes: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompactionModelSample {
    schema_version: u16,
    source: SessionSourceIdentity,
    request_audit_event_id: EventId,
    provider_session: ProviderSessionRef,
    narrative: NarrativeSummary,
    byte_limit: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    graph: Option<CompactionSampleGraph>,
}

impl CompactionModelSample {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_v2(
        source: SessionSourceIdentity,
        request_audit_event_id: EventId,
        narrative: NarrativeSummary,
        byte_limit: usize,
        plan_blob_id: BlobId,
        plan: &CompactionSourcePlan,
        node_id: &str,
        output: String,
    ) -> Result<Self, SessionError> {
        if plan.source() != &source {
            return Err(super::source_plan::invalid_task(
                "sample plan source mismatch",
            ));
        }
        let node = plan
            .nodes()
            .iter()
            .find(|node| node.id == node_id)
            .ok_or_else(|| super::source_plan::invalid_task("sample node is absent from plan"))?
            .clone();
        let sample = Self {
            schema_version: 2,
            provider_session: plan.node_provider_session(node_id)?,
            source,
            request_audit_event_id,
            narrative,
            byte_limit,
            graph: Some(CompactionSampleGraph {
                plan_blob_id,
                plan_hash: plan.hash()?,
                node,
                output,
                retention: CompactionRetention::SingleSample,
            }),
        };
        sample.validate()?;
        Ok(sample)
    }

    pub(crate) fn validate(&self) -> Result<(), SessionError> {
        self.source.validate()?;
        super::require_text(
            "compaction_sample.audit",
            self.request_audit_event_id.as_str(),
        )?;
        self.narrative.validate()?;
        let graph = self
            .graph
            .as_ref()
            .filter(|_| self.schema_version == 2)
            .ok_or_else(|| super::source_plan::invalid_task("invalid Compaction sample version"))?;
        let handoff: Handoff = serde_json::from_str(&graph.output)
            .map_err(|_| super::source_plan::invalid_task("invalid Compaction node output"))?;
        let valid_hash = graph.plan_hash.len() == 64
            && graph
                .plan_hash
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'));
        let scope = ProviderSessionRef {
            id: ProviderSessionId::new(format!(
                "compaction:{}:{}:{}:{}",
                self.source.stream_id(),
                self.source.watermark().get(),
                graph.plan_hash,
                super::fingerprint_value(&graph.node.id.as_str())?
            )),
            role: ProviderSessionRole::Primary,
            generation: 0,
        };
        if !valid_hash
            || BlobId::new(graph.plan_blob_id.as_str()).is_err()
            || self.provider_session != scope
            || graph.node.id.is_empty()
            || graph.node.output_limit == 0
            || graph.node.template_hash.as_ref().is_none_or(|hash| {
                hash.len() != 64
                    || !hash
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
            })
            || graph.output.len() > graph.node.output_limit
            || handoff.concise_progress != self.narrative.concise_progress()
            || handoff.unresolved_questions != self.narrative.unresolved_questions()
            || handoff.next_step_notes != self.narrative.next_step_notes()
            || self.byte_limit == 0
        {
            return Err(super::source_plan::invalid_task(
                "invalid bounded Compaction graph sample",
            ));
        }
        Ok(())
    }

    pub(crate) fn source(&self) -> &SessionSourceIdentity {
        &self.source
    }
    pub(crate) fn audit_event_id(&self) -> &EventId {
        &self.request_audit_event_id
    }
    pub(crate) fn provider_session(&self) -> &ProviderSessionRef {
        &self.provider_session
    }
    pub(crate) fn narrative(&self) -> &NarrativeSummary {
        &self.narrative
    }

    pub(crate) fn graph_node(&self) -> Option<(&BlobId, &str, &CompactionSourceNode, &str)> {
        self.graph.as_ref().map(|graph| {
            (
                &graph.plan_blob_id,
                graph.plan_hash.as_str(),
                &graph.node,
                graph.output.as_str(),
            )
        })
    }
}

impl SessionSourceIdentity {
    #[allow(clippy::too_many_arguments)] // Exact source and audit inputs share this validator.
    pub(crate) fn validate_graph_pending(
        &self,
        events: &[EventEnvelope<Value>],
        project_events: &[EventEnvelope<Value>],
        research_source: &super::ValidatedResearchSource,
        plan_blob_id: &BlobId,
        plan: &CompactionSourcePlan,
        node_id: &str,
        attempts: &[EventId],
    ) -> Result<(), SessionError> {
        if plan.source() != self || attempts.is_empty() {
            return Err(super::source_plan::invalid_task(
                "invalid pending graph attempt",
            ));
        }
        let start = events
            .len()
            .checked_sub(attempts.len())
            .ok_or_else(|| super::source_plan::invalid_task("missing graph attempt"))?;
        let scope = plan.node_provider_session(node_id)?;
        let mut first_attempt = None;
        for (event, expected) in events[start..].iter().zip(attempts) {
            match SessionEvent::decode(&event.event_type, event.payload.clone())? {
                SessionEvent::RequestAuditRecorded { manifest }
                    if &event.event_id == expected
                        && manifest.provider_session() == Some(&scope)
                        && first_attempt
                            .as_ref()
                            .is_none_or(|first| manifest.repeats_request(first)) =>
                {
                    first_attempt.get_or_insert(manifest);
                }
                _ => {
                    return Err(super::source_plan::invalid_task(
                        "pending graph attempt does not match its audit",
                    ));
                }
            }
        }
        let prefix = &events[..start];
        let boundary = prefix.partition_point(|event| event.sequence <= self.watermark());
        let mut prior_nodes = Vec::new();
        for event in &prefix[boundary..] {
            if let SessionEvent::CompactionSampleRecorded { sample } =
                SessionEvent::decode(&event.event_type, event.payload.clone())?
            {
                let (_, _, node, _) = sample.graph_node().ok_or_else(|| {
                    super::source_plan::invalid_task("mixed compaction sample versions")
                })?;
                prior_nodes.push(node.id.clone());
            }
        }
        if plan
            .nodes()
            .get(prior_nodes.len())
            .is_none_or(|node| node.id != node_id)
            || prior_nodes
                .iter()
                .zip(plan.nodes())
                .any(|(seen, planned)| seen != &planned.id)
        {
            return Err(super::source_plan::invalid_task(
                "graph node sampling order does not match the plan",
            ));
        }
        let previous = prefix[boundary..]
            .iter()
            .rev()
            .find(|event| event.event_type.as_str() == "session.compaction_sample_recorded");
        if let Some(previous) = previous {
            let SessionEvent::CompactionSampleRecorded { sample } =
                SessionEvent::decode(&previous.event_type, previous.payload.clone())?
            else {
                unreachable!()
            };
            let (blob, hash, node, _) = sample.graph_node().ok_or_else(|| {
                super::source_plan::invalid_task("mixed compaction sample versions")
            })?;
            if blob != plan_blob_id || hash != plan.hash()? || node.is_root {
                return Err(super::source_plan::invalid_task(
                    "pending graph follows a different or finished operation",
                ));
            }
            self.validate_graph_sampling_suffix(
                research_source,
                prefix,
                project_events,
                &previous.event_id,
            )?;
        } else {
            let boundary = self.validate_graph_source(research_source, prefix, project_events)?;
            if boundary != prefix.len() {
                return Err(super::source_plan::invalid_task(
                    "unrelated Session change before Compaction graph sampling",
                ));
            }
        }
        if plan.research_source() != research_source.identity() {
            return Err(super::source_plan::invalid_task(
                "pending graph Research source mismatch",
            ));
        }
        Ok(())
    }

    pub(crate) fn validate_graph_sampling_suffix(
        &self,
        research_source: &super::ValidatedResearchSource,
        events: &[EventEnvelope<Value>],
        project_events: &[EventEnvelope<Value>],
        last_sample: &EventId,
    ) -> Result<(), SessionError> {
        use std::collections::BTreeSet;
        let boundary = self.validate_graph_source(research_source, events, project_events)?;
        let mut operation: Option<(BlobId, String)> = None;
        let mut pending = Vec::new();
        let mut nodes = BTreeSet::new();
        let mut last_accepted: Option<(EventId, ProviderSessionRef)> = None;
        let mut root_seen = false;
        let mut last_seen = false;
        for event in &events[boundary..] {
            match SessionEvent::decode(&event.event_type, event.payload.clone())? {
                SessionEvent::RequestAuditRecorded { manifest } if !root_seen => {
                    let scope = manifest.provider_session().ok_or_else(|| {
                        super::source_plan::invalid_task("graph audit has no scope")
                    })?;
                    if pending
                        .first()
                        .is_some_and(|(_, _, first)| !manifest.repeats_request(first))
                    {
                        return Err(super::source_plan::invalid_task(
                            "Compaction retry audit does not match its prepared request",
                        ));
                    }
                    pending.push((event.event_id.clone(), scope.clone(), manifest));
                    last_accepted = None;
                }
                SessionEvent::CompactionSampleRecorded { sample } if !root_seen => {
                    let (blob, hash, node, _) = sample.graph_node().ok_or_else(|| {
                        super::source_plan::invalid_task("mixed compaction sample versions")
                    })?;
                    if sample.source() != self
                        || operation
                            .as_ref()
                            .is_some_and(|old| old != &(blob.clone(), hash.to_owned()))
                        || pending.is_empty()
                        || pending
                            .iter()
                            .any(|(_, scope, _)| scope != sample.provider_session())
                        || pending
                            .last()
                            .is_none_or(|(audit, _, _)| audit != sample.audit_event_id())
                        || !nodes.insert(node.id.clone())
                    {
                        return Err(super::source_plan::invalid_task(
                            "Compaction graph sample or audit mismatch",
                        ));
                    }
                    if let super::CompactionSourceNodeInput::Merge(children) = &node.input
                        && children
                            .iter()
                            .any(|child| !nodes.contains(child) || child == &node.id)
                    {
                        return Err(super::source_plan::invalid_task(
                            "Compaction graph child is not a prior sample",
                        ));
                    }
                    operation = Some((blob.clone(), hash.to_owned()));
                    pending.clear();
                    root_seen = node.is_root;
                    last_accepted = Some((
                        sample.audit_event_id().clone(),
                        sample.provider_session().clone(),
                    ));
                    last_seen = &event.event_id == last_sample;
                }
                SessionEvent::ModelUsageRecorded {
                    request_audit_event_id,
                    provider_session,
                    ..
                } if last_accepted.as_ref().is_some_and(|(audit, scope)| {
                    audit == &request_audit_event_id && scope == &provider_session
                }) =>
                {
                    last_accepted = None;
                }
                _ => {
                    return Err(super::source_plan::invalid_task(
                        "unrelated Session change during Compaction graph sampling",
                    ));
                }
            }
        }
        if !last_seen || !pending.is_empty() {
            return Err(super::source_plan::invalid_task(
                "incomplete Compaction graph suffix",
            ));
        }
        Ok(())
    }

    fn validate_graph_source(
        &self,
        research_source: &super::ValidatedResearchSource,
        events: &[EventEnvelope<Value>],
        research_events: &[EventEnvelope<Value>],
    ) -> Result<usize, SessionError> {
        self.validate()?;
        let actual =
            super::ResearchSourceIdentity::from_events(self.project_id().clone(), research_events)?;
        let boundary = events.partition_point(|event| event.sequence <= self.watermark());
        if &actual != research_source.identity()
            || events[..boundary].last().is_none_or(|event| {
                event.sequence != self.watermark() || &event.stream_id != self.stream_id()
            })
            || super::fingerprint_value(&&events[..boundary])? != self.stream_hash()
        {
            return Err(super::source_plan::invalid_task(
                "Compaction graph source prefix changed",
            ));
        }
        Ok(boundary)
    }
}
