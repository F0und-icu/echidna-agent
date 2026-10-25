//! Durable Session handoff for the fixed improvement stage.
use super::*;

impl DurableChatSession {
    pub(super) fn improvement_state(&self) -> Result<SessionState, ChatError> {
        self.durable_state()
    }

    pub(super) fn record_improvement(
        &self,
        attempt_id: &str,
        fact: ImprovementFact,
    ) -> Result<EventId, ChatError> {
        self.with_projection(|projection, store, stream_id| {
            let sequence = store.next_sequence()?;
            let id = event_id(
                self.project.project_id(),
                &self.session_id,
                sequence.get(),
                "rsi-activity",
            );
            let event = SessionEvent::ImprovementActivityRecorded {
                activity: ImprovementActivity {
                    schema_version: 1,
                    attempt_id: attempt_id.to_owned(),
                    fact,
                },
            }
            .into_envelope(id.clone(), stream_id, sequence, occurred_at())?;
            projection
                .append(store, event)
                .map_err(ChatError::from_session_append)?;
            Ok(id)
        })
    }

    pub(super) fn finish_improvement(
        &self,
        attempt: &str,
        disposition: &str,
        reason: impl Into<String>,
    ) -> Result<EventId, ChatError> {
        let reason = reason.into();
        console_eprintln!("[RSI2 {disposition}] {reason}");
        self.record_improvement(
            attempt,
            ImprovementFact::Finished {
                disposition: disposition.to_owned(),
                reason,
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn scoped_sample<Provider: ModelProvider + ?Sized>(
        &self,
        agent: &AgentLoop<'_, '_, Provider, ToolRegistry>,
        tools: &ToolRegistry,
        attempt: &str,
        stage: ImprovementStage,
        prompt: &crate::domain::research_control::PromptComposition,
        task: String,
        references: Vec<EventId>,
        budget: &ModelRequestBudget,
        cancellation: AgentCancellation,
        audit_context: &RequestAuditRunContext,
    ) -> Result<ImprovementSampleOutput, ChatError> {
        let mut record_audit = |attempt| {
            self.record_request_audit_manifest(attempt, None)
                .map(|manifest| manifest.audit_event_id().clone())
                .map_err(ChatError::into_request_audit_agent_loop_error)
        };
        let mut record_sample = |audit_event_id, provider_session, items, original_call_ids| {
            self.record_improvement(
                attempt,
                ImprovementFact::Sample {
                    stage,
                    audit_event_id,
                    provider_session,
                    items,
                    original_call_ids,
                },
            )
            .map(|_| ())
            .map_err(ChatError::into_history_checkpoint_agent_loop_error)
        };
        let mut durability =
            |checkpoint: ChatDurabilityCheckpoint<'_>| -> Result<(), AgentLoopError> {
                match checkpoint {
                    ChatDurabilityCheckpoint::ExecutionReceipt(outcome) => self
                        .record_execution(outcome)
                        .map(|_| ())
                        .map_err(ChatError::into_execution_receipt_agent_loop_error),
                    ChatDurabilityCheckpoint::History(items) => {
                        if stage == ImprovementStage::Diagnosis {
                            return Ok(());
                        }
                        let state = self
                            .improvement_state()
                            .map_err(ChatError::into_history_checkpoint_agent_loop_error)?;
                        let ids = items
                            .iter()
                            .filter_map(|item| {
                                if let ContentItem::ToolResult { result } = item {
                                    state
                                        .execution_receipts()
                                        .iter()
                                        .find(|record| {
                                            record.receipt.execution_id
                                                == ExecutionId::for_model_call(&result.call_id)
                                        })
                                        .map(|record| record.event_id.clone())
                                } else {
                                    None
                                }
                            })
                            .collect();
                        self.record_improvement(
                            attempt,
                            ImprovementFact::Results {
                                stage,
                                items: items.to_vec(),
                                receipt_event_ids: ids,
                            },
                        )
                        .map(|_| ())
                        .map_err(ChatError::into_history_checkpoint_agent_loop_error)
                    }
                    _ => Err(AgentLoopError::ImprovementResponse {
                        message: "unsupported checkpoint in one-command RSI scope".to_owned(),
                    }),
                }
            };
        agent
            .sample_improvement(
                ImprovementSampleInput {
                    attempt_id: attempt,
                    stage,
                    prompt,
                    task,
                    references,
                    tools,
                    request_budget: budget,
                    cancellation,
                    audit_context,
                },
                &mut record_audit,
                &mut record_sample,
                &mut durability,
            )
            .await
            .map_err(ChatError::from)
    }
}
