//! Strict, read-only Trace Replay and Shadow Eval contracts.
//!
//! A Trace fixture is deliberately a set of references into existing Session,
//! Research, Artifact and RequestAudit facts. It is not another Event stream
//! or an authority for vulnerability evidence.

mod accuracy;
mod error;
mod manifest;
mod plan;
mod result;
mod source_validation;

pub use accuracy::{
    BatchOpportunity, TOOL_ACCURACY_REPORT_SCHEMA_VERSION, ToolAccuracyCaseExpectation,
    ToolAccuracyDeferredMetric, ToolAccuracyDeferredMetricKind, ToolAccuracyError,
    ToolAccuracyMetrics, ToolAccuracyReport,
};
pub(crate) use accuracy::{ToolAccuracyTraceProjection, ToolAccuracyTraceSample};
pub use error::TraceFixtureError;
pub use manifest::{
    TRACE_FIXTURE_MANIFEST_SCHEMA_VERSION, TraceEventRange, TraceExpectedInvariant,
    TraceFixtureManifest, TraceProviderEventReference,
};
pub use plan::{
    TraceReplayModelStep, TraceReplayPlan, TraceReplayPlanError, TraceReplayToolStep,
    TraceReplayToolUse,
};
pub use result::{EvalEvidence, EvalProvenance, EvalResult, EvalSource};
