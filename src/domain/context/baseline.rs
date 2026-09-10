//! Non-destructive Baseline assessment and in-process Provider session state.

use serde::{Deserialize, Serialize};

use crate::domain::model::ContentItem;

use super::canonical::{
    ContextError, ProviderSessionId, ProviderSessionRef, ProviderSessionRole, StepSnapshot,
};

/// Stable reason codes for requiring a complete Provider request.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BaselineInvalidationReason {
    NoBaseline,
    ProviderSessionChanged,
    ModelChanged,
    ProfileChanged,
    ModelInputIrSchemaChanged,
    CapabilityMappingChanged,
    PromptChanged,
    PromptRendererChanged,
    ToolsChanged,
    OutputContractChanged,
    RequestPropertiesChanged,
    InputNotAppendOnly,
    IncrementalInputUnsupported,
    CompactionReset,
    SessionResumed,
    ProviderRejected,
}

impl BaselineInvalidationReason {
    /// Full delivery may differ from the current logical input only for the
    /// two existing state-diff fallbacks or for an exact Provider reasoning
    /// replay across a Prompt/Tool-plan change.
    pub(crate) const fn permits_distinct_full_delivery(self) -> bool {
        matches!(
            self,
            Self::IncrementalInputUnsupported
                | Self::ProviderRejected
                | Self::PromptChanged
                | Self::ToolsChanged
        )
    }

    /// These reasons may carry the established state-diff representation,
    /// which need not equal the logical input after removing reasoning.
    pub(crate) const fn permits_state_diff_full_delivery(self) -> bool {
        matches!(
            self,
            Self::IncrementalInputUnsupported | Self::ProviderRejected
        )
    }
}

/// The local delivery decision for one semantic request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BaselineAssessment {
    FullRequest { reason: BaselineInvalidationReason },
    AppendOnlyCandidate { delta_items: Vec<ContentItem> },
}

/// A complete semantic reference point for a future incremental request.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(super) struct IncrementalBaseline {
    provider_session: ProviderSessionRef,
    request: super::canonical::CanonicalRequest,
    server_output_items: Vec<ContentItem>,
}

impl IncrementalBaseline {
    fn from_full_response(snapshot: &StepSnapshot, server_output_items: Vec<ContentItem>) -> Self {
        Self {
            provider_session: snapshot.provider_session.clone(),
            request: snapshot.request.clone(),
            server_output_items: super::canonical::CanonicalInput::from_items(server_output_items)
                .items,
        }
    }

    fn assess(&self, snapshot: &StepSnapshot) -> BaselineAssessment {
        let candidate = &snapshot.request;
        let baseline = &self.request;

        let reason = if self.provider_session != snapshot.provider_session {
            Some(BaselineInvalidationReason::ProviderSessionChanged)
        } else if baseline.model_hash != candidate.model_hash {
            Some(BaselineInvalidationReason::ModelChanged)
        } else if baseline.profile_hash != candidate.profile_hash {
            Some(BaselineInvalidationReason::ProfileChanged)
        } else if baseline.model_input_ir_schema_hash != candidate.model_input_ir_schema_hash {
            Some(BaselineInvalidationReason::ModelInputIrSchemaChanged)
        } else if baseline.provider_capability_hash != candidate.provider_capability_hash {
            Some(BaselineInvalidationReason::CapabilityMappingChanged)
        } else if baseline.prompt_hash != candidate.prompt_hash {
            Some(BaselineInvalidationReason::PromptChanged)
        } else if baseline.prompt_renderer_hash != candidate.prompt_renderer_hash {
            Some(BaselineInvalidationReason::PromptRendererChanged)
        } else if baseline.tool_schema_hash != candidate.tool_schema_hash {
            Some(BaselineInvalidationReason::ToolsChanged)
        } else if baseline.output_contract_hash != candidate.output_contract_hash {
            Some(BaselineInvalidationReason::OutputContractChanged)
        } else if baseline.request_properties_hash != candidate.request_properties_hash {
            Some(BaselineInvalidationReason::RequestPropertiesChanged)
        } else {
            None
        };

        if let Some(reason) = reason {
            return BaselineAssessment::FullRequest { reason };
        }

        let baseline_items = baseline.canonical_input.items();
        let expected_prefix_len = baseline_items.len() + self.server_output_items.len();
        let actual_items = candidate.canonical_input.items();
        if actual_items.len() < expected_prefix_len
            || actual_items[..baseline_items.len()] != baseline_items[..]
            || actual_items[baseline_items.len()..expected_prefix_len]
                != self.server_output_items[..]
        {
            return BaselineAssessment::FullRequest {
                reason: BaselineInvalidationReason::InputNotAppendOnly,
            };
        }

        BaselineAssessment::AppendOnlyCandidate {
            delta_items: actual_items[expected_prefix_len..].to_vec(),
        }
    }
}

/// In-process state for one Provider role. The Baseline remains unchanged by
/// assessment; only a completed and locally committed sample replaces it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderSession {
    reference: ProviderSessionRef,
    baseline: Option<IncrementalBaseline>,
    pending_reset: Option<BaselineInvalidationReason>,
}

impl ProviderSession {
    pub fn new(id: ProviderSessionId, role: ProviderSessionRole, generation: u64) -> Self {
        Self {
            reference: ProviderSessionRef {
                id,
                role,
                generation,
            },
            baseline: None,
            pending_reset: None,
        }
    }

    pub(crate) fn reference(&self) -> &ProviderSessionRef {
        &self.reference
    }

    /// Marks a reset reason without discarding a Baseline before the forced
    /// full request completes successfully.
    pub fn reset(&mut self, reason: BaselineInvalidationReason) {
        // Compaction replaces History and Summary semantics. Once queued, no
        // later Turn-boundary Prompt/Tool change may obscure that causal reset
        // before the first successful complete request commits it.
        if self.pending_reset == Some(BaselineInvalidationReason::CompactionReset) {
            return;
        }
        self.pending_reset = Some(reason);
    }

    /// Evaluates the next complete semantic request without consuming reset
    /// state or replacing the prior Baseline. A Provider that cannot deliver
    /// incremental input keeps the semantic Baseline for audit but receives a
    /// complete request.
    pub fn assess_delivery(
        &self,
        snapshot: &StepSnapshot,
        incremental_input_supported: bool,
    ) -> BaselineAssessment {
        if self.reference != snapshot.provider_session {
            return BaselineAssessment::FullRequest {
                reason: BaselineInvalidationReason::ProviderSessionChanged,
            };
        }
        if let Some(reason) = self.pending_reset {
            return BaselineAssessment::FullRequest { reason };
        }
        let assessment = self.baseline.as_ref().map_or(
            BaselineAssessment::FullRequest {
                reason: BaselineInvalidationReason::NoBaseline,
            },
            |baseline| baseline.assess(snapshot),
        );
        match assessment {
            BaselineAssessment::AppendOnlyCandidate { .. } if !incremental_input_supported => {
                BaselineAssessment::FullRequest {
                    reason: BaselineInvalidationReason::IncrementalInputUnsupported,
                }
            }
            assessment => assessment,
        }
    }

    /// Replaces the Baseline after any completed Provider sample has committed
    /// its local History output, then clears any reset reason. The captured
    /// snapshot is always the full logical request, independent of delivery.
    pub fn record_completed_sample(
        &mut self,
        snapshot: &StepSnapshot,
        server_output_items: Vec<ContentItem>,
    ) -> Result<(), ContextError> {
        if self.reference != snapshot.provider_session {
            return Err(ContextError::ProviderSessionMismatch {
                expected: self.reference.clone(),
                received: snapshot.provider_session.clone(),
            });
        }
        self.baseline = Some(IncrementalBaseline::from_full_response(
            snapshot,
            server_output_items,
        ));
        self.pending_reset = None;
        Ok(())
    }
}
