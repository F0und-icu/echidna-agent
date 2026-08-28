//! Closed derived-only Eval result labels and source scope.

use crate::domain::events::{EventId, StreamId};

use super::{TraceFixtureError, TraceFixtureManifest};

/// Every M4-S2 result carries one of these derived-only origins.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EvalProvenance {
    MechanicalReplay,
    ContextRecompile,
    ShadowModelEval,
}

/// Eval output has one intentionally closed evidence state. It cannot be
/// constructed as a Finding or promoted by this contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EvalEvidence {
    NonEvidence,
}

/// Exact source scope for a derived Eval result.
///
/// Historical identifiers, including an `ExecutionId`, are interpreted under
/// this source Session. Replay executions remain derived Eval activity even
/// when the AgentLoop deterministically produces the same bare identifier.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvalSource {
    trace_id: String,
    source_session_id: StreamId,
    source_stream_hash: String,
}

impl EvalSource {
    pub fn new(
        trace_id: impl Into<String>,
        source_session_id: StreamId,
        source_stream_hash: impl Into<String>,
    ) -> Result<Self, TraceFixtureError> {
        let source = Self {
            trace_id: trace_id.into(),
            source_session_id,
            source_stream_hash: source_stream_hash.into(),
        };
        if source.trace_id.trim().is_empty() {
            return Err(TraceFixtureError::EmptyField { field: "trace_id" });
        }
        if source.source_session_id.as_str().trim().is_empty() {
            return Err(TraceFixtureError::EmptyField {
                field: "source_session_id",
            });
        }
        if !is_lowercase_blake3_hash(&source.source_stream_hash) {
            return Err(TraceFixtureError::InvalidSourceStreamHash);
        }
        Ok(source)
    }

    pub(crate) fn from_validated_manifest(manifest: &TraceFixtureManifest) -> Self {
        Self {
            trace_id: manifest.trace_id().to_owned(),
            source_session_id: manifest.source_session_id().clone(),
            source_stream_hash: manifest.source_stream_hash().to_owned(),
        }
    }

    pub fn trace_id(&self) -> &str {
        &self.trace_id
    }

    pub fn source_session_id(&self) -> &StreamId {
        &self.source_session_id
    }

    pub fn source_stream_hash(&self) -> &str {
        &self.source_stream_hash
    }
}

fn is_lowercase_blake3_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

/// Shared non-evidence identity embedded in every Trace/Eval result.
///
/// `provenance + source` is the isolated Eval namespace: it prevents derived
/// replay identities from being interpreted as facts in the source Session.
/// Context Recompile further scopes the comparison to one RequestAudit Event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvalResult {
    provenance: EvalProvenance,
    evidence: EvalEvidence,
    source: EvalSource,
    request_audit_event_id: Option<EventId>,
}

impl EvalResult {
    pub(crate) fn mechanical_replay(source: EvalSource) -> Self {
        Self {
            provenance: EvalProvenance::MechanicalReplay,
            evidence: EvalEvidence::NonEvidence,
            source,
            request_audit_event_id: None,
        }
    }

    pub(crate) fn context_recompile(source: EvalSource, request_audit_event_id: EventId) -> Self {
        Self {
            provenance: EvalProvenance::ContextRecompile,
            evidence: EvalEvidence::NonEvidence,
            source,
            request_audit_event_id: Some(request_audit_event_id),
        }
    }

    pub(crate) fn shadow_model_eval(source: EvalSource) -> Self {
        Self {
            provenance: EvalProvenance::ShadowModelEval,
            evidence: EvalEvidence::NonEvidence,
            source,
            request_audit_event_id: None,
        }
    }

    pub const fn provenance(&self) -> EvalProvenance {
        self.provenance
    }

    pub const fn evidence(&self) -> EvalEvidence {
        self.evidence
    }

    pub const fn is_non_evidence(&self) -> bool {
        matches!(self.evidence, EvalEvidence::NonEvidence)
    }

    pub const fn source(&self) -> &EvalSource {
        &self.source
    }

    /// Present for Context Recompile, which compares one exact historical
    /// RequestAudit rather than a whole Trace model sequence.
    pub const fn request_audit_event_id(&self) -> Option<&EventId> {
        self.request_audit_event_id.as_ref()
    }
}
