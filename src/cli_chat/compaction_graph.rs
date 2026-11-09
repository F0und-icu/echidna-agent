//! Finite source partition and merge planning for tool-free handoffs.

use std::collections::BTreeMap;

use crate::app::agent_loop::{AgentLoop, AgentLoopError, minimum_compaction_handoff_bytes};
use crate::domain::events::EventId;
use crate::domain::session::{
    CompactionSourceEntry, CompactionSourceNode, CompactionSourceNodeInput, CompactionSourcePlan,
    CompactionSourceRange, SessionSourceIdentity,
};
use crate::ports::artifact::ArtifactStore;
use crate::ports::model::ModelProvider;

use super::ChatError;

impl super::DurableChatSession {
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn sample_compaction_graph<Provider: ModelProvider + ?Sized>(
        &self,
        agent: &AgentLoop<'_, '_, Provider, crate::adapters::tools::ToolRegistry>,
        request: &super::compaction::CompactionRequestContext<'_>,
        capture: &crate::domain::session::CompactionContextCapture,
        source: &SessionSourceIdentity,
        goal: &str,
        references: &[EventId],
        entries: &[CompactionSourceEntry],
        contents: &[String],
        selection_event: Option<&EventId>,
        byte_limit: usize,
        output_limit: usize,
    ) -> Result<crate::domain::session::VerifiedCompactionGraph, ChatError> {
        let max_id = super::event_id(
            self.project.project_id(),
            &self.session_id,
            u64::MAX,
            "compaction-sample",
        );
        let provider = request.composition.compaction_provider().0;
        let plan = plan(
            capture,
            source,
            entries,
            contents,
            selection_event,
            references,
            output_limit,
            &max_id,
            &request.cancellation,
        )
        .await?;
        if let Some(remaining) = request.budget.remaining()
            && remaining < plan.nodes().len()
        {
            return Err(AgentLoopError::ModelRequestLimit {
                limit: request
                    .budget
                    .limit()
                    .expect("bounded remaining has a limit"),
            }
            .into());
        }
        let bytes =
            serde_json::to_vec(&plan).map_err(|error| ChatError::CompactionSummarySource {
                message: error.to_string(),
            })?;
        let blob_id = self
            .artifacts
            .publish_bytes(&bytes)
            .map_err(|error| ChatError::CompactionSummarySource {
                message: format!("compaction plan publication failed: {error}"),
            })?
            .blob_id;
        let mut outputs = BTreeMap::<String, (EventId, String)>::new();
        let mut expected_inputs = BTreeMap::new();
        let mut accepted_audits = Vec::new();
        let mut root = None;
        for node in plan.nodes() {
            if request.cancellation.is_cancelled() {
                return Err(AgentLoopError::CompactionCancelled.into());
            }
            let children = match &node.input {
                CompactionSourceNodeInput::Source(_) => BTreeMap::new(),
                CompactionSourceNodeInput::Merge(ids) => ids
                    .iter()
                    .map(|id| (id.clone(), outputs[id].clone()))
                    .collect(),
            };
            let source_data = plan.render_node(&node.id, contents, &children)?;
            let expected = crate::app::agent_loop::compile_captured_compaction_source(
                capture,
                source,
                &source_data,
                references,
                node.output_limit,
            )?;
            if node.template_hash.as_deref()
                != Some(
                    crate::domain::session::node_template_hash(
                        &expected.model_input(),
                        expected.delivery_map(),
                        &source_data,
                    )?
                    .as_str(),
                )
            {
                return Err(ChatError::CompactionSummarySource {
                    message: "compaction node fixed request template changed after planning"
                        .to_owned(),
                });
            }
            expected_inputs.insert(
                node.id.clone(),
                (expected.model_input(), expected.delivery_map().clone()),
            );
            let mut attempts = Vec::new();
            let mut record = |attempt| {
                self.record_request_audit_manifest(attempt, None)
                    .map(|manifest| {
                        let id = manifest.audit_event_id().clone();
                        attempts.push(id.clone());
                        id
                    })
                    .map_err(ChatError::into_request_audit_agent_loop_error)
            };
            let output = agent
                .sample_compaction(
                    crate::app::agent_loop::CompactionSampleInput {
                        provider,
                        agent_model_profile: Some(request.composition.compaction_provider().1),
                        source,
                        source_data,
                        current_goal: goal.to_owned(),
                        references: references.to_vec(),
                        byte_limit,
                        request_budget: request.budget,
                        cancellation: request.cancellation.clone(),
                        audit_context: request.audit,
                        graph: crate::app::agent_loop::CompactionGraphSample {
                            capture,
                            plan: &plan,
                            plan_blob_id: blob_id.clone(),
                            node_id: &node.id,
                        },
                    },
                    &mut record,
                )
                .await?;
            if request.cancellation.is_cancelled() {
                return Err(AgentLoopError::CompactionCancelled.into());
            }
            let sample_id = self.with_projection(|projection, store, stream_id| {
                let events = crate::adapters::project_event_store::read_existing_session_events(
                    &self.project,
                    &self.session_id,
                )?;
                let project_events =
                    crate::adapters::project_event_store::read_existing_research_events(
                        &self.project,
                    )?;
                source.validate_graph_pending(
                    &events,
                    &project_events,
                    &blob_id,
                    &plan,
                    &node.id,
                    &attempts,
                )?;
                let sequence = store.next_sequence()?;
                let id = super::event_id(
                    self.project.project_id(),
                    &self.session_id,
                    sequence.get(),
                    "compaction-sample",
                );
                let event = crate::domain::session::SessionEvent::CompactionSampleRecorded {
                    sample: output.sample.clone(),
                }
                .into_envelope(
                    id.clone(),
                    stream_id,
                    sequence,
                    super::occurred_at(),
                )?;
                projection
                    .append(store, event)
                    .map_err(ChatError::from_session_append)?;
                Ok(id)
            })?;
            if let Some(usage) = output.usage {
                self.record_model_usage(
                    output.sample.audit_event_id(),
                    output.sample.provider_session(),
                    usage,
                )?;
            }
            accepted_audits.push(output.sample.audit_event_id().clone());
            let (_, _, _, raw) = output
                .sample
                .graph_node()
                .expect("v2 sample carries graph output");
            outputs.insert(node.id.clone(), (sample_id.clone(), raw.to_owned()));
            if node.is_root {
                root = Some((sample_id, output.sample));
            }
        }
        let (id, _) = root.expect("validated graph has one root");
        self.with_projection(|projection, _, stream_id| {
            let actual = self.artifacts.read_blob(&blob_id).map_err(|error| {
                ChatError::CompactionSummarySource {
                    message: format!("compaction plan read failed: {error}"),
                }
            })?;
            if actual != bytes {
                return Err(ChatError::CompactionSummarySource {
                    message: "compaction plan Blob bytes differ from its source plan".to_owned(),
                });
            }
            let events = crate::adapters::project_event_store::read_existing_session_events(
                &self.project,
                &self.session_id,
            )?;
            let project_events =
                crate::adapters::project_event_store::read_existing_research_events(&self.project)?;
            source.validate_graph_sampling_suffix(&events, &project_events, &id)?;
            let service = crate::app::request_audit::RequestAuditService::new(
                &self.artifacts,
                crate::domain::request_audit::RequestAuditBinding::new(
                    self.project.project_id().clone(),
                    stream_id,
                )?,
            );
            let mut proofs = BTreeMap::new();
            for audit_id in &accepted_audits {
                let audit = projection
                    .state()
                    .request_audits()
                    .iter()
                    .find(|audit| audit.audit_event_id() == audit_id)
                    .ok_or(ChatError::RuntimeHistoryMismatch)?;
                let proof = service
                    .reconstruct_full_logical(audit)
                    .map_err(|error| ChatError::CompactionSummarySource {
                        message: format!("compaction audit reconstruction failed: {error:?}"),
                    })?
                    .ok_or_else(|| ChatError::CompactionSummarySource {
                        message: "compaction audit has no full verified input".to_owned(),
                    })?;
                proofs.insert(audit_id.clone(), proof);
            }
            let task_fragments = plan
                .task_selection_event()
                .map(|_| capture.fragments())
                .transpose()?;
            let graph = plan.verify_model_node_audits(
                &blob_id,
                contents,
                &events,
                &project_events,
                &proofs,
                &expected_inputs,
                &capture.capabilities,
                task_fragments.as_deref(),
                capture.reference_limit,
                projection
                    .state()
                    .latest_compaction()
                    .and_then(|prior| projection.compaction_source_for(prior.summary()))
                    .and_then(|facts| facts.verified_graph()),
            )?;
            crate::app::session::verify_compaction_source_blobs(&graph, &self.artifacts)?;
            Ok(graph)
        })
    }
}

async fn build_plan(
    source: &SessionSourceIdentity,
    entries: &[CompactionSourceEntry],
    contents: &[String],
    selection_event: Option<&EventId>,
    chunk_bytes: usize,
    output_limit: usize,
    cancellation: &crate::app::cancellation::AgentCancellation,
) -> Result<CompactionSourcePlan, ChatError> {
    let mut nodes = Vec::new();
    let mut level = Vec::new();
    let mut ranges = Vec::new();
    let mut used = 0usize;
    for (index, _entry) in entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| !entry.protected_exact)
    {
        let text = &contents[index];
        let mut start = 0;
        while start < text.len() {
            tokio::task::yield_now().await;
            if cancellation.is_cancelled() {
                return Err(AgentLoopError::CompactionCancelled.into());
            }
            let mut end = start
                .saturating_add(chunk_bytes.saturating_sub(used))
                .min(text.len());
            while end < text.len() && !text.is_char_boundary(end) {
                end += 1;
            }
            ranges.push(CompactionSourceRange {
                entry: index,
                start,
                end,
            });
            used += end - start;
            start = end;
            if used >= chunk_bytes {
                let id = format!("source-{}", nodes.len());
                nodes.push(CompactionSourceNode {
                    id: id.clone(),
                    input: CompactionSourceNodeInput::Source(std::mem::take(&mut ranges)),
                    is_root: false,
                    output_limit,
                    template_hash: None,
                });
                level.push(id);
                used = 0;
            }
        }
    }
    if !ranges.is_empty() {
        let id = format!("source-{}", nodes.len());
        nodes.push(CompactionSourceNode {
            id: id.clone(),
            input: CompactionSourceNodeInput::Source(ranges),
            is_root: false,
            output_limit,
            template_hash: None,
        });
        level.push(id);
    }
    if level.is_empty() {
        return Err(ChatError::CompactionNotUseful);
    }
    while level.len() > 1 {
        tokio::task::yield_now().await;
        if cancellation.is_cancelled() {
            return Err(AgentLoopError::CompactionCancelled.into());
        }
        let mut next = Vec::new();
        for group in level.chunks(2) {
            if group.len() == 1 {
                next.push(group[0].clone());
                continue;
            }
            let id = format!("merge-{}", nodes.len());
            nodes.push(CompactionSourceNode {
                id: id.clone(),
                input: CompactionSourceNodeInput::Merge(group.to_vec()),
                is_root: false,
                output_limit,
                template_hash: None,
            });
            next.push(id);
        }
        level = next;
    }
    nodes.last_mut().expect("non-empty source tree").is_root = true;
    let plan = CompactionSourcePlan::new(source.clone(), entries.to_vec(), nodes, contents)?;
    Ok(match selection_event {
        Some(event) => plan.with_task_selection(event.clone(), contents)?,
        None => plan,
    })
}

/// Build the entire finite tree and check the actual encoded IR for every
/// source node and worst-case-sized child output before any model admission.
#[allow(clippy::too_many_arguments)]
pub(super) async fn plan(
    capture: &crate::domain::session::CompactionContextCapture,
    source: &SessionSourceIdentity,
    entries: &[CompactionSourceEntry],
    contents: &[String],
    selection_event: Option<&EventId>,
    references: &[EventId],
    byte_limit: usize,
    maximum_event_id: &EventId,
    cancellation: &crate::app::cancellation::AgentCancellation,
) -> Result<CompactionSourcePlan, ChatError> {
    let minimum_output = minimum_compaction_handoff_bytes();
    let mut output_limit = byte_limit;
    if output_limit < minimum_output {
        return Err(AgentLoopError::CompactionSummaryRejected {
            reason: "summary envelope cannot hold the minimum handoff".to_owned(),
        }
        .into());
    }
    let mut chunk = contents.iter().map(String::len).sum::<usize>().max(1);
    loop {
        let plan = build_plan(
            source,
            entries,
            contents,
            selection_event,
            chunk,
            output_limit,
            cancellation,
        )
        .await?;
        let mut source_failed = false;
        let mut merge_failed = false;
        let mut templates = BTreeMap::new();
        for node in plan.nodes() {
            tokio::task::yield_now().await;
            if cancellation.is_cancelled() {
                return Err(AgentLoopError::CompactionCancelled.into());
            }
            let mut children = BTreeMap::new();
            if let CompactionSourceNodeInput::Merge(ids) = &node.input {
                for id in ids {
                    let child = plan
                        .nodes()
                        .iter()
                        .find(|candidate| &candidate.id == id)
                        .expect("validated tree has child");
                    // Quotes are the maximum expansion per byte under the JSON
                    // encoder; the real strict JSON output cannot encode more.
                    children.insert(
                        id.clone(),
                        (maximum_event_id.clone(), "\"".repeat(child.output_limit)),
                    );
                }
            }
            let input = plan.render_node(&node.id, contents, &children)?;
            match crate::app::agent_loop::compile_captured_compaction_source(
                capture,
                source,
                &input,
                references,
                node.output_limit,
            ) {
                Ok(compiled) => {
                    templates.insert(
                        node.id.clone(),
                        crate::domain::session::node_template_hash(
                            &compiled.model_input(),
                            compiled.delivery_map(),
                            &input,
                        )?,
                    );
                }
                Err(AgentLoopError::ContextPlanner(
                    crate::domain::context::ContextPlannerError::FixedInputExceedsBudget { .. }
                    | crate::domain::context::ContextPlannerError::RequiredFragmentsExceedBudget {
                        ..
                    }
                    | crate::domain::context::ContextPlannerError::RenderedInputExceedsBudget {
                        ..
                    },
                )) => match node.input {
                    CompactionSourceNodeInput::Source(_) => source_failed = true,
                    CompactionSourceNodeInput::Merge(_) => merge_failed = true,
                },
                Err(error) => return Err(error.into()),
            }
        }
        if !source_failed && !merge_failed {
            return Ok(plan.with_node_templates(templates, contents)?);
        }
        if merge_failed {
            if output_limit == minimum_output {
                return Err(AgentLoopError::CompactionSummaryRejected {
                    reason: "two minimum child handoffs cannot fit a merge request".to_owned(),
                }
                .into());
            }
            output_limit = (output_limit / 2).max(minimum_output);
        }
        if source_failed {
            if chunk == 1 {
                return Err(AgentLoopError::CompactionSummaryRejected {
                    reason: "a minimum source interval cannot fit a handoff request".to_owned(),
                }
                .into());
            }
            chunk = (chunk / 2).max(1);
        }
    }
}
