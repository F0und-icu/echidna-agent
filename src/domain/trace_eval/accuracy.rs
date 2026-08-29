//! Deterministic, source-bound Tool-accuracy measurements.
//!
//! A report is one derived view over one validated Trace and one controlled
//! expectation. It never accepts caller-supplied counters or writes facts.

use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;

use crate::domain::canonical_json::canonical_json;
use crate::domain::context::tool_schema_token_cost;
use crate::domain::tools::{
    ExecutionReceipt, ToolCall, ToolCallId, ToolName, ToolResult, ToolResultStatus, ToolSpec,
};

use super::{EvalResult, EvalSource};

pub const TOOL_ACCURACY_REPORT_SCHEMA_VERSION: u16 = 1;

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ToolAccuracyError {
    #[error("a Tool-accuracy case id must not be empty")]
    EmptyCaseId,
    #[error("a Tool-accuracy case needs at least one acceptable first Tool")]
    AcceptableFirstActionRequired,
    #[error("a missed-batch expectation needs at least two individual Tool Calls")]
    BatchOpportunityRequiresMultipleCalls,
    #[error("Tool-accuracy projection has duplicate Tool Call `{call_id}")]
    DuplicateToolCall { call_id: ToolCallId },
    #[error("Tool-accuracy projection has duplicate Tool Result `{call_id}")]
    DuplicateToolResult { call_id: ToolCallId },
    #[error("Tool Result `{call_id}` is not paired to a projected Tool Call")]
    ResultWithoutCall { call_id: ToolCallId },
    #[error("Tool Result `{call_id}` has a different Tool name than its projected Tool Call")]
    ResultToolNameMismatch { call_id: ToolCallId },
    #[error("pre-execution Tool Result `{call_id}` has a projected Receipt")]
    PreExecutionReceiptPresent { call_id: ToolCallId },
    #[error("Tool Result `{call_id}` has no projected Receipt")]
    ReceiptMissing { call_id: ToolCallId },
    #[error("Tool-accuracy schema token cost could not be calculated: {message}")]
    ToolSchemaTokenCost { message: String },
    #[error("Tool-accuracy schema token cost exceeds the local counter")]
    ToolSchemaTokenCostOverflow,
}

/// A controlled expectation for a group of independent calls that should use
/// an already-available batch Tool instead.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BatchOpportunity {
    individual_call_ids: BTreeSet<ToolCallId>,
    preferred_batch_tool: ToolName,
}

impl BatchOpportunity {
    pub fn new(
        individual_call_ids: impl IntoIterator<Item = ToolCallId>,
        preferred_batch_tool: ToolName,
    ) -> Result<Self, ToolAccuracyError> {
        let individual_call_ids = individual_call_ids.into_iter().collect::<BTreeSet<_>>();
        if individual_call_ids.len() < 2 {
            return Err(ToolAccuracyError::BatchOpportunityRequiresMultipleCalls);
        }
        Ok(Self {
            individual_call_ids,
            preferred_batch_tool,
        })
    }
}

/// Controlled expectations remain separate from recorded observations. A
/// single expectation names one Trace case and cannot increment counters.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolAccuracyCaseExpectation {
    id: String,
    acceptable_first_actions: BTreeSet<ToolName>,
    batch_opportunities: Vec<BatchOpportunity>,
}

impl ToolAccuracyCaseExpectation {
    pub fn new(
        id: impl Into<String>,
        acceptable_first_actions: impl IntoIterator<Item = ToolName>,
    ) -> Result<Self, ToolAccuracyError> {
        let id = id.into();
        let acceptable_first_actions = acceptable_first_actions
            .into_iter()
            .collect::<BTreeSet<_>>();
        if id.trim().is_empty() {
            return Err(ToolAccuracyError::EmptyCaseId);
        }
        if acceptable_first_actions.is_empty() {
            return Err(ToolAccuracyError::AcceptableFirstActionRequired);
        }
        Ok(Self {
            id,
            acceptable_first_actions,
            batch_opportunities: Vec::new(),
        })
    }

    pub fn with_batch_opportunity(mut self, opportunity: BatchOpportunity) -> Self {
        self.batch_opportunities.push(opportunity);
        self
    }
}

/// One RequestAudit-reconstructed model sample. Tool schemas and model Tool
/// Calls remain together so deferred activation is measured from the actual
/// adjacent advertised schemas rather than output names alone.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ToolAccuracyTraceSample {
    tool_specs: Vec<ToolSpec>,
    tool_calls: Vec<ToolCall>,
}

impl ToolAccuracyTraceSample {
    pub(crate) fn new(tool_specs: Vec<ToolSpec>, tool_calls: Vec<ToolCall>) -> Self {
        Self {
            tool_specs,
            tool_calls,
        }
    }
}

/// A non-persisted projection of concrete Tool facts under one validated
/// Trace. It is constructed only by the App Trace reconstruction owner.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ToolAccuracyTraceProjection {
    samples: Vec<ToolAccuracyTraceSample>,
    tool_results: Vec<ToolResult>,
}

impl ToolAccuracyTraceProjection {
    pub(crate) fn from_validated_trace(
        samples: Vec<ToolAccuracyTraceSample>,
        tool_results: Vec<ToolResult>,
        receipts: Vec<ExecutionReceipt>,
    ) -> Result<Self, ToolAccuracyError> {
        let mut calls = BTreeMap::new();
        for call in samples.iter().flat_map(|sample| &sample.tool_calls) {
            if calls.insert(call.call_id.clone(), call).is_some() {
                return Err(ToolAccuracyError::DuplicateToolCall {
                    call_id: call.call_id.clone(),
                });
            }
        }
        let receipt_ids = receipts
            .iter()
            .map(|receipt| receipt.execution_id.to_string())
            .collect::<BTreeSet<_>>();
        let mut result_ids = BTreeSet::new();
        for result in &tool_results {
            if !result_ids.insert(result.call_id.clone()) {
                return Err(ToolAccuracyError::DuplicateToolResult {
                    call_id: result.call_id.clone(),
                });
            }
            let Some(call) = calls.get(&result.call_id) else {
                return Err(ToolAccuracyError::ResultWithoutCall {
                    call_id: result.call_id.clone(),
                });
            };
            if result.tool_name != call.name {
                return Err(ToolAccuracyError::ResultToolNameMismatch {
                    call_id: result.call_id.clone(),
                });
            }
            let expected_execution_id = format!("execution:{}", result.call_id);
            if result.is_pre_execution_failure() {
                if receipt_ids.contains(&expected_execution_id) {
                    return Err(ToolAccuracyError::PreExecutionReceiptPresent {
                        call_id: result.call_id.clone(),
                    });
                }
            } else if !receipt_ids.contains(&expected_execution_id) {
                return Err(ToolAccuracyError::ReceiptMissing {
                    call_id: result.call_id.clone(),
                });
            }
        }
        Ok(Self {
            samples,
            tool_results,
        })
    }

    fn tool_calls(&self) -> impl Iterator<Item = &ToolCall> {
        self.samples
            .iter()
            .flat_map(|sample| sample.tool_calls.iter())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolAccuracyDeferredMetricKind {
    DeferredActivation,
    UnmatchedReplayTermination,
    ObservationPromotion,
    ReplayPromotion,
    VisionPromotion,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct ToolAccuracyDeferredMetric {
    pub metric: ToolAccuracyDeferredMetricKind,
    pub reason: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct ToolAccuracyMetrics {
    pub cases: usize,
    pub first_action_observations: usize,
    pub acceptable_first_action_matches: usize,
    pub invalid_argument_count: usize,
    pub correction_attempt_count: usize,
    pub correction_success_count: usize,
    pub redundant_reads: usize,
    pub missed_batch_opportunities: usize,
    pub tool_search_calls: usize,
    pub tool_search_hits: usize,
    /// `None` means a successful Deferred search has no adjacent audited
    /// Step, so the activation denominator is not durably observable.
    pub deferred_activations: Option<usize>,
    pub schema_token_cost: usize,
    pub deferred_metrics: Vec<ToolAccuracyDeferredMetric>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct ToolAccuracyReport {
    pub schema_version: u16,
    evaluation: EvalResult,
    case_id: String,
    metrics: ToolAccuracyMetrics,
}

impl ToolAccuracyReport {
    pub(crate) fn from_validated_trace(
        source: EvalSource,
        expectation: ToolAccuracyCaseExpectation,
        projection: ToolAccuracyTraceProjection,
    ) -> Result<Self, ToolAccuracyError> {
        let calls = projection.tool_calls().collect::<Vec<_>>();
        let (tool_search_hits, deferred_activations) = tool_search_stats(&projection);
        let metrics = ToolAccuracyMetrics {
            cases: 1,
            first_action_observations: usize::from(!calls.is_empty()),
            acceptable_first_action_matches: usize::from(
                calls
                    .first()
                    .is_some_and(|call| expectation.acceptable_first_actions.contains(&call.name)),
            ),
            invalid_argument_count: invalid_argument_results(&projection).len(),
            correction_attempt_count: correction_outcomes(&projection).0,
            correction_success_count: correction_outcomes(&projection).1,
            redundant_reads: redundant_reads(&projection),
            missed_batch_opportunities: missed_batch_opportunities(&projection, &expectation),
            tool_search_calls: calls
                .iter()
                .filter(|call| call.name.as_str() == "tool_search")
                .count(),
            tool_search_hits,
            deferred_activations,
            schema_token_cost: schema_token_cost(&projection)?,
            deferred_metrics: deferred_metrics(deferred_activations.is_none()),
        };
        Ok(Self {
            schema_version: TOOL_ACCURACY_REPORT_SCHEMA_VERSION,
            evaluation: EvalResult::tool_accuracy(source),
            case_id: expectation.id,
            metrics,
        })
    }

    pub const fn evaluation(&self) -> &EvalResult {
        &self.evaluation
    }

    pub fn case_id(&self) -> &str {
        &self.case_id
    }

    pub const fn metrics(&self) -> &ToolAccuracyMetrics {
        &self.metrics
    }
}

fn is_correctable_invalid_argument(result: &ToolResult) -> bool {
    result.status == ToolResultStatus::Failed
        && result.output.get("failure").is_some_and(|failure| {
            failure.get("code").and_then(serde_json::Value::as_str) == Some("invalid_arguments")
                && failure.get("stage").and_then(serde_json::Value::as_str) == Some("pre_execution")
                && failure
                    .get("retryability")
                    .and_then(serde_json::Value::as_str)
                    == Some("correct_arguments")
        })
}

fn invalid_argument_results(projection: &ToolAccuracyTraceProjection) -> Vec<&ToolResult> {
    projection
        .tool_results
        .iter()
        .filter(|result| is_correctable_invalid_argument(result))
        .collect()
}

fn correction_outcomes(projection: &ToolAccuracyTraceProjection) -> (usize, usize) {
    let results = projection
        .tool_results
        .iter()
        .map(|result| (&result.call_id, result))
        .collect::<BTreeMap<_, _>>();
    let mut attempts = 0;
    let mut successes = 0;
    for invalid in invalid_argument_results(projection) {
        let Some((sample_index, invalid_call)) =
            projection
                .samples
                .iter()
                .enumerate()
                .find_map(|(sample_index, sample)| {
                    sample
                        .tool_calls
                        .iter()
                        .find(|call| call.call_id == invalid.call_id)
                        .map(|call| (sample_index, call))
                })
        else {
            continue;
        };
        let Some(next_sample) = projection.samples.get(sample_index + 1) else {
            continue;
        };
        if let Some(corrected) = next_sample
            .tool_calls
            .iter()
            .find(|call| call.name == invalid.tool_name && call.arguments != invalid_call.arguments)
        {
            attempts += 1;
            if results
                .get(&corrected.call_id)
                .is_some_and(|result| result.status == ToolResultStatus::Succeeded)
            {
                successes += 1;
            }
        }
    }
    (attempts, successes)
}

fn redundant_reads(projection: &ToolAccuracyTraceProjection) -> usize {
    let results = projection
        .tool_results
        .iter()
        .map(|result| (&result.call_id, result))
        .collect::<BTreeMap<_, _>>();
    let mut seen = BTreeSet::new();
    projection
        .tool_calls()
        .filter(|call| call.name.as_str() == "read_file")
        .filter_map(|call| {
            let result = results.get(&call.call_id)?;
            (result.status == ToolResultStatus::Succeeded).then_some((call, *result))
        })
        .filter(|(call, result)| {
            let arguments = serde_json::to_string(&canonical_json(&call.arguments))
                .expect("ToolCall arguments are always JSON serializable");
            let output = serde_json::to_string(&canonical_json(&result.output))
                .expect("ToolResult output is always JSON serializable");
            !seen.insert((arguments, output))
        })
        .count()
}

fn missed_batch_opportunities(
    projection: &ToolAccuracyTraceProjection,
    expectation: &ToolAccuracyCaseExpectation,
) -> usize {
    expectation
        .batch_opportunities
        .iter()
        .filter(|opportunity| {
            let observed = projection
                .tool_calls()
                .filter(|call| opportunity.individual_call_ids.contains(&call.call_id))
                .count();
            observed == opportunity.individual_call_ids.len()
                && projection.samples.iter().any(|sample| {
                    sample
                        .tool_specs
                        .iter()
                        .any(|spec| spec.name == opportunity.preferred_batch_tool)
                })
        })
        .count()
}

fn tool_search_stats(projection: &ToolAccuracyTraceProjection) -> (usize, Option<usize>) {
    let result_by_call = projection
        .tool_results
        .iter()
        .map(|result| (&result.call_id, result))
        .collect::<BTreeMap<_, _>>();
    let mut hits = 0;
    for sample in &projection.samples {
        for call in sample
            .tool_calls
            .iter()
            .filter(|call| call.name.as_str() == "tool_search")
        {
            let Some(matches) = result_by_call
                .get(&call.call_id)
                .filter(|result| result.status == ToolResultStatus::Succeeded)
                .and_then(|result| result.output.get("matches"))
                .and_then(serde_json::Value::as_array)
            else {
                continue;
            };
            if !matches.is_empty() {
                hits += 1;
            }
        }
    }
    // RequestAudit currently binds the Session stream, exact ToolSpecs and
    // immutable plan identity, but it has no durable Turn identity. Adjacent
    // samples therefore cannot prove the same-Turn requirement from v1.4.
    // Keep activation Deferred instead of inferring a success or a zero.
    (hits, None)
}

fn deferred_metrics(deferred_activation_unobservable: bool) -> Vec<ToolAccuracyDeferredMetric> {
    let mut metrics = vec![
        ToolAccuracyDeferredMetric {
            metric: ToolAccuracyDeferredMetricKind::UnmatchedReplayTermination,
            reason: "no durable typed Replay-termination signal exists",
        },
        ToolAccuracyDeferredMetric {
            metric: ToolAccuracyDeferredMetricKind::ObservationPromotion,
            reason: "no durable typed Observation-promotion signal exists",
        },
        ToolAccuracyDeferredMetric {
            metric: ToolAccuracyDeferredMetricKind::ReplayPromotion,
            reason: "no durable typed Replay-promotion signal exists",
        },
        ToolAccuracyDeferredMetric {
            metric: ToolAccuracyDeferredMetricKind::VisionPromotion,
            reason: "no durable typed Vision-promotion signal exists",
        },
    ];
    if deferred_activation_unobservable {
        metrics.push(ToolAccuracyDeferredMetric {
            metric: ToolAccuracyDeferredMetricKind::DeferredActivation,
            reason: "no adjacent RequestAudit proves the Deferred ToolSpec transition",
        });
    }
    metrics
}

fn schema_token_cost(projection: &ToolAccuracyTraceProjection) -> Result<usize, ToolAccuracyError> {
    projection
        .samples
        .iter()
        .try_fold(0_usize, |total, sample| {
            let cost = tool_schema_token_cost(&sample.tool_specs).map_err(|error| {
                ToolAccuracyError::ToolSchemaTokenCost {
                    message: error.to_string(),
                }
            })?;
            let cost = usize::try_from(cost)
                .map_err(|_| ToolAccuracyError::ToolSchemaTokenCostOverflow)?;
            total
                .checked_add(cost)
                .ok_or(ToolAccuracyError::ToolSchemaTokenCostOverflow)
        })
}
