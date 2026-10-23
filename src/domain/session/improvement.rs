//! Scoped RSI observations in the original Session, outside operator History.

use super::*;
use crate::domain::agent_evolution::ScriptFile;
use crate::domain::tools::{ExecutionId, ToolCallId};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ImprovementStage {
    Diagnosis,
    Old,
    New,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ImprovementFact {
    Started {
        parent_digest: String,
        history_start: usize,
        friction: Vec<EventId>,
    },
    Sample {
        stage: ImprovementStage,
        audit_event_id: EventId,
        provider_session: ProviderSessionRef,
        items: Vec<ContentItem>,
        original_call_ids: Vec<ToolCallId>,
    },
    Results {
        stage: ImprovementStage,
        items: Vec<ContentItem>,
        receipt_event_ids: Vec<EventId>,
    },
    ReferenceMatch {
        source_call_id: ToolCallId,
        inputs: Vec<ScriptFile>,
        old_matches: bool,
        new_matches: bool,
    },
    Finished {
        disposition: String,
        reason: String,
    },
}

/// A versioned payload consumed by the Session reducer and completed-boundary RSI service.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ImprovementActivity {
    pub(crate) schema_version: u16,
    pub(crate) attempt_id: String,
    pub(crate) fact: ImprovementFact,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ImprovementRecord {
    pub(crate) event_id: EventId,
    pub(crate) activity: ImprovementActivity,
}

impl SessionState {
    pub(super) fn ensure_no_scope_call_identity_collision(
        &self,
        call_id: &ToolCallId,
    ) -> Result<(), SessionError> {
        if self.improvement_records.iter().any(|record|matches!(&record.activity.fact,ImprovementFact::Sample {items,..} if items.iter().any(|item|matches!(item,ContentItem::ToolCall {call} if &call.call_id==call_id)))) {
            return Err(SessionError::InvalidImprovementActivity);
        }
        Ok(())
    }
    pub(crate) fn improvement_evidence(
        &self,
        attempt: &str,
        parent_digest: &str,
    ) -> Result<Vec<EventId>, SessionError> {
        let invalid = || SessionError::InvalidImprovementActivity;
        let records = self
            .improvement_records
            .iter()
            .filter(|record| record.activity.attempt_id == attempt)
            .collect::<Vec<_>>();
        let started=records.iter().find(|record|matches!(&record.activity.fact,ImprovementFact::Started {parent_digest:seen,..} if seen==parent_digest)).ok_or_else(invalid)?;
        let finished=records.iter().find(|record|matches!(&record.activity.fact,ImprovementFact::Finished {disposition,..} if disposition=="accepted")).ok_or_else(invalid)?;
        let mut references = vec![started.event_id.clone(), finished.event_id.clone()];
        for stage in [
            ImprovementStage::Diagnosis,
            ImprovementStage::Old,
            ImprovementStage::New,
        ] {
            let audit = records
                .iter()
                .find_map(|record| match &record.activity.fact {
                    ImprovementFact::Sample {
                        stage: seen,
                        audit_event_id,
                        ..
                    } if stage == *seen => Some(audit_event_id.clone()),
                    _ => None,
                })
                .ok_or_else(invalid)?;
            references.push(audit);
        }
        for stage in [ImprovementStage::Old, ImprovementStage::New] {
            let receipts = records
                .iter()
                .find_map(|record| match &record.activity.fact {
                    ImprovementFact::Results {
                        stage: seen,
                        receipt_event_ids,
                        ..
                    } if stage == *seen => Some(receipt_event_ids),
                    _ => None,
                })
                .ok_or_else(invalid)?;
            if receipts.len() != 1 {
                return Err(invalid());
            }
            references.push(receipts[0].clone());
        }
        Ok(references)
    }
    pub(crate) fn ensure_no_unknown_improvement(&self) -> Result<(), SessionError> {
        for record in &self.improvement_records {
            if let ImprovementFact::Sample {stage,items,..} = &record.activity.fact
                && items.iter().any(|item| matches!(item,ContentItem::ToolCall {..}))
                && !self.improvement_records.iter().any(|later| later.activity.attempt_id == record.activity.attempt_id && matches!(&later.activity.fact,ImprovementFact::Results {stage:seen,..} if seen == stage)) {
                return Err(SessionError::InvalidImprovementActivity);
            }
        }
        Ok(())
    }
}

pub(super) fn apply(
    state: &mut SessionState,
    activity: ImprovementActivity,
    event_id: &EventId,
) -> Result<(), SessionError> {
    let invalid = || SessionError::InvalidImprovementActivity;
    if activity.schema_version != 1
        || activity.attempt_id.is_empty()
        || activity.attempt_id.len() > 128
        || state.lifecycle != SessionLifecycle::Active
        || serde_json::to_vec(&activity).map_err(|_| invalid())?.len() > 1024 * 1024
    {
        return Err(invalid());
    }
    let prior = state
        .improvement_records
        .iter()
        .filter(|record| record.activity.attempt_id == activity.attempt_id)
        .collect::<Vec<_>>();
    match &activity.fact {
        ImprovementFact::Started {
            parent_digest,
            history_start,
            friction,
        } => {
            state.ensure_no_unknown_improvement()?;
            let binding = state
                .agent_definition_binding
                .as_ref()
                .ok_or_else(invalid)?;
            if !prior.is_empty() || binding.definition().digest().as_str() != parent_digest
                || !binding.definition().rsi().is_some_and(|rsi| rsi.improvement)
                || friction.is_empty() || friction.len() > 8
                || !matches!(state.history.items().get(*history_start),Some(ContentItem::Text {role:super::super::model::MessageRole::User,..}))
                || state.improvement_records.iter().any(|record|matches!(&record.activity.fact,ImprovementFact::Started {history_start:seen,..} if seen==history_start))
                || friction.iter().any(|id| !state.history_event_ids.iter().skip(*history_start).any(|seen| seen==id)
                    && !state.execution_receipts.iter().any(|record| &record.event_id == id && state.history.items().iter().skip(*history_start).any(|item|matches!(item,ContentItem::ToolResult {result} if ExecutionId::for_model_call(&result.call_id)==record.receipt.execution_id)))) {
                return Err(invalid());
            }
            state.history.ensure_all_tool_calls_paired()?;
        }
        fact => {
            if prior.is_empty()
                || prior
                    .iter()
                    .any(|record| matches!(record.activity.fact, ImprovementFact::Finished { .. }))
            {
                return Err(invalid());
            }
            match fact {
                ImprovementFact::Sample {
                    stage,
                    audit_event_id,
                    provider_session,
                    items,
                    original_call_ids,
                } => {
                    if state.request_audits.iter().rev().find(|audit| audit.improvement_scope() == Some(provider_session)).is_none_or(|audit| audit.audit_event_id() != audit_event_id)
                        || provider_session.id != ProviderSessionId::new(format!("rsi:{}:{stage:?}",activity.attempt_id))
                        || provider_session.role != ProviderSessionRole::Primary
                        || state.primary_provider_session.as_ref() == Some(provider_session)
                        || prior.iter().any(|record| matches!(&record.activity.fact, ImprovementFact::Sample {stage: seen, ..} if stage == seen))
                        || items.iter().any(|item| !matches!(item, ContentItem::ToolCall {..} | ContentItem::Text {role: super::super::model::MessageRole::Assistant, ..})) {
                        return Err(invalid());
                    }
                    let calls = items
                        .iter()
                        .filter_map(|item| match item {
                            ContentItem::ToolCall { call } => Some(call),
                            _ => None,
                        })
                        .collect::<Vec<_>>();
                    if calls.len() != original_call_ids.len()
                        || calls.len() > usize::from(*stage != ImprovementStage::Diagnosis)
                        || calls.iter().zip(original_call_ids).any(|(call, original)| {
                            call.call_id != scoped_call_id(&activity.attempt_id, *stage, original)
                        })
                    {
                        return Err(invalid());
                    }
                    for call in &calls {
                        state.ensure_no_scope_call_identity_collision(&call.call_id)?;
                        if state.history.contains_tool_call_id(&call.call_id) {
                            return Err(invalid());
                        }
                    }
                    if *stage != ImprovementStage::Diagnosis
                        && !prior.iter().any(|record| {
                            matches!(
                                record.activity.fact,
                                ImprovementFact::Sample {
                                    stage: ImprovementStage::Diagnosis,
                                    ..
                                }
                            )
                        })
                    {
                        return Err(invalid());
                    }
                    if *stage == ImprovementStage::New
                        && !prior.iter().any(|record| {
                            matches!(
                                record.activity.fact,
                                ImprovementFact::Results {
                                    stage: ImprovementStage::Old,
                                    ..
                                }
                            )
                        })
                    {
                        return Err(invalid());
                    }
                    let mut history = ConversationHistory::default();
                    for item in items {
                        history.append_content_item(item.clone())?;
                    }
                }
                ImprovementFact::Results {
                    stage,
                    items,
                    receipt_event_ids,
                } => {
                    if *stage == ImprovementStage::Diagnosis {
                        return Err(invalid());
                    }
                    let sample = prior
                        .iter()
                        .find_map(|record| match &record.activity.fact {
                            ImprovementFact::Sample {
                                stage: seen, items, ..
                            } if seen == stage => Some(items),
                            _ => None,
                        })
                        .ok_or_else(invalid)?;
                    if prior.iter().any(|record| matches!(&record.activity.fact, ImprovementFact::Results {stage: seen, ..} if seen == stage)) { return Err(invalid()); }
                    let mut history = ConversationHistory::default();
                    let mut expected_receipts = Vec::new();
                    for item in sample {
                        history.append_content_item(item.clone())?;
                    }
                    for item in items {
                        let ContentItem::ToolResult { result } = item else {
                            return Err(invalid());
                        };
                        history.append_tool_result(result.clone())?;
                        let receipt = state.execution_receipts.iter().find(|record| {
                            record.receipt.execution_id
                                == ExecutionId::for_model_call(&result.call_id)
                        });
                        if let Some(record) = receipt {
                            expected_receipts.push(record.event_id.clone());
                            if record.receipt.status != result.status {
                                return Err(invalid());
                            }
                            record
                                .receipt
                                .validate_output_captures(result)
                                .map_err(|_| invalid())?;
                        } else if result.output.get("output_captures").is_some() {
                            return Err(invalid());
                        }
                    }
                    history.ensure_all_tool_calls_paired()?;
                    if receipt_event_ids != &expected_receipts {
                        return Err(invalid());
                    }
                }
                ImprovementFact::ReferenceMatch {
                    source_call_id,
                    inputs,
                    ..
                } => {
                    crate::domain::agent_evolution::validate_input_files(inputs)
                        .map_err(|_| invalid())?;
                    if inputs.is_empty() || [ImprovementStage::Old,ImprovementStage::New].iter().any(|stage| !prior.iter().any(|record|matches!(&record.activity.fact,ImprovementFact::Results {stage:seen,receipt_event_ids,items} if seen==stage && receipt_event_ids.len()==1 && items.len()==1))) {return Err(invalid());}
                    if prior.iter().any(|record|matches!(record.activity.fact,ImprovementFact::ReferenceMatch {..}))
                        || !state.history.items().iter().any(|item| matches!(item,ContentItem::ToolResult {result} if &result.call_id == source_call_id))
                        || [ImprovementStage::Old, ImprovementStage::New].iter().any(|stage| !prior.iter().any(|record| matches!(&record.activity.fact, ImprovementFact::Results {stage: seen,..} if stage == seen))) { return Err(invalid()); }
                }
                ImprovementFact::Finished {
                    disposition,
                    reason,
                } => {
                    state.ensure_no_unknown_improvement()?;
                    if reason.trim().is_empty()
                        || reason.len() > 4096
                        || !["no_change", "rejected", "inconclusive", "accepted"]
                            .contains(&disposition.as_str())
                    {
                        return Err(invalid());
                    }
                    if disposition == "accepted"
                        && !prior.iter().any(|record| {
                            matches!(
                                record.activity.fact,
                                ImprovementFact::ReferenceMatch {
                                    old_matches: false,
                                    new_matches: true,
                                    ..
                                }
                            )
                        })
                    {
                        return Err(invalid());
                    }
                }
                ImprovementFact::Started { .. } => unreachable!(),
            }
        }
    }
    state.improvement_records.push(ImprovementRecord {
        event_id: event_id.clone(),
        activity,
    });
    Ok(())
}

pub(crate) fn scoped_call_id(
    attempt: &str,
    stage: ImprovementStage,
    original: &ToolCallId,
) -> ToolCallId {
    ToolCallId::new(format!(
        "rsi:{}",
        blake3::hash(format!("{attempt}:{stage:?}:{}", original.as_str()).as_bytes()).to_hex()
    ))
}

pub(super) fn validate_audit_scope(
    state: &SessionState,
    scope: &ProviderSessionRef,
    manifest: &RequestAuditManifest,
) -> Result<(), SessionError> {
    let started = state
        .improvement_records
        .iter()
        .rev()
        .find(|record| matches!(record.activity.fact, ImprovementFact::Started { .. }))
        .ok_or(SessionError::InvalidImprovementActivity)?;
    let attempt = &started.activity.attempt_id;
    let records = state
        .improvement_records
        .iter()
        .filter(|record| &record.activity.attempt_id == attempt)
        .collect::<Vec<_>>();
    if records
        .iter()
        .any(|record| matches!(record.activity.fact, ImprovementFact::Finished { .. }))
    {
        return Err(SessionError::InvalidImprovementActivity);
    }
    let stage = if records.iter().any(|record| {
        matches!(
            record.activity.fact,
            ImprovementFact::Results {
                stage: ImprovementStage::Old,
                ..
            }
        )
    }) {
        ImprovementStage::New
    } else if records.iter().any(|record| {
        matches!(
            record.activity.fact,
            ImprovementFact::Sample {
                stage: ImprovementStage::Diagnosis,
                ..
            }
        )
    }) {
        ImprovementStage::Old
    } else {
        ImprovementStage::Diagnosis
    };
    if scope.id != ProviderSessionId::new(format!("rsi:{attempt}:{stage:?}"))
        || scope.role != ProviderSessionRole::Primary
        || scope.generation != 0
        || records.iter().any(|record| {
            matches!(record.activity.fact,
                ImprovementFact::Sample {stage:seen,..} | ImprovementFact::Results {stage:seen,..}
                    if seen == stage)
        })
    {
        return Err(SessionError::InvalidImprovementActivity);
    }
    let audits = state
        .request_audits
        .iter()
        .filter(|audit| audit.improvement_scope() == Some(scope))
        .collect::<Vec<_>>();
    if audits.len() >= 3 || audits.iter().any(|audit| !manifest.repeats_request(audit)) {
        return Err(SessionError::InvalidImprovementActivity);
    }
    Ok(())
}
