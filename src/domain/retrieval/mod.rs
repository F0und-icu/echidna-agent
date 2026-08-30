//! Fixed, local, deterministic security-retrieval Shadow contracts.
//!
//! This module is deliberately a single versioned algorithm owner. It ranks
//! caller-supplied, bounded facts in memory and has no filesystem, Provider,
//! Tool, Event, Research, Memory, or Context mutation capability.

mod normalize;
mod rank;

use std::collections::BTreeSet;

use serde::Serialize;
use thiserror::Error;

use crate::domain::canonical_json::canonical_blake3_hash;
use crate::domain::context::estimate_audit_component_tokens;

pub(crate) const ALGORITHM_VERSION: &str = "m4-s3f-deterministic-security-retrieval-v1";
pub(crate) const MAX_QUERY_BYTES: usize = 4 * 1024;
pub(crate) const MAX_CANDIDATES: usize = 512;
pub(crate) const MAX_FIELD_BYTES: usize = 16 * 1024;
pub(crate) const MAX_RESULTS: usize = 10;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RetrievalQuery {
    pub(crate) project_id: String,
    pub(crate) current_target: CurrentTarget,
    pub(crate) text: String,
    pub(crate) historical_comparison: bool,
}

impl RetrievalQuery {
    pub(crate) fn new(
        project_id: impl Into<String>,
        current_target: CurrentTarget,
        text: impl Into<String>,
        historical_comparison: bool,
    ) -> Result<Self, RetrievalError> {
        let query = Self {
            project_id: project_id.into(),
            current_target,
            text: text.into(),
            historical_comparison,
        };
        require_text("project_id", &query.project_id)?;
        query.current_target.validate()?;
        require_text("query", &query.text)?;
        if query.text.len() > MAX_QUERY_BYTES {
            return Err(RetrievalError::QueryTooLarge {
                actual: query.text.len(),
                maximum: MAX_QUERY_BYTES,
            });
        }
        Ok(query)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct CurrentTarget {
    pub(crate) snapshot_id: String,
    pub(crate) target_id: String,
    pub(crate) version: String,
}

impl CurrentTarget {
    pub(crate) fn new(
        snapshot_id: impl Into<String>,
        target_id: impl Into<String>,
        version: impl Into<String>,
    ) -> Result<Self, RetrievalError> {
        let target = Self {
            snapshot_id: snapshot_id.into(),
            target_id: target_id.into(),
            version: version.into(),
        };
        target.validate()?;
        Ok(target)
    }

    fn validate(&self) -> Result<(), RetrievalError> {
        require_text("current_target.snapshot_id", &self.snapshot_id)?;
        require_text("current_target.target_id", &self.target_id)?;
        require_text("current_target.version", &self.version)
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RetrievalSource {
    ProjectMemory,
    Research,
    ArtifactMetadata,
    ObservationMetadata,
    HistoricalSummary,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RetrievalField {
    Title,
    Identifiers,
    Target,
    Body,
    Metadata,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CandidateTarget {
    ProjectScoped,
    Snapshot(String),
    SnapshotSet(Vec<String>),
    TargetIdentity {
        target_id: String,
        version: Option<String>,
    },
    Unknown,
}

impl CandidateTarget {
    pub(crate) fn snapshot(value: impl Into<String>) -> Self {
        Self::Snapshot(value.into())
    }

    pub(crate) fn snapshot_set(values: Vec<String>) -> Self {
        Self::SnapshotSet(values)
    }

    pub(crate) fn target_identity(target_id: impl Into<String>, version: Option<String>) -> Self {
        Self::TargetIdentity {
            target_id: target_id.into(),
            version,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RetrievalCandidate {
    pub(crate) id: String,
    pub(crate) project_id: String,
    pub(crate) source: RetrievalSource,
    pub(crate) target: CandidateTarget,
    pub(crate) fields: Vec<(RetrievalField, String)>,
}

impl RetrievalCandidate {
    pub(crate) fn new(
        id: impl Into<String>,
        project_id: impl Into<String>,
        source: RetrievalSource,
        target: CandidateTarget,
        fields: Vec<(RetrievalField, String)>,
    ) -> Result<Self, RetrievalError> {
        let mut fields = fields
            .into_iter()
            .map(|(field, value)| (field, truncate_utf8(value, MAX_FIELD_BYTES)))
            .collect::<Vec<_>>();
        fields.sort_by_key(|(field, _)| *field);
        let candidate = Self {
            id: id.into(),
            project_id: project_id.into(),
            source,
            target,
            fields,
        };
        require_text("candidate.id", &candidate.id)?;
        require_text("candidate.project_id", &candidate.project_id)?;
        if candidate.fields.is_empty() {
            return Err(RetrievalError::CandidateMissingFields {
                id: candidate.id.clone(),
            });
        }
        let mut seen = BTreeSet::new();
        for (field, _) in &candidate.fields {
            if !seen.insert(*field) {
                return Err(RetrievalError::CandidateDuplicateField {
                    id: candidate.id.clone(),
                    field: *field,
                });
            }
        }
        Ok(candidate)
    }

    pub(crate) fn estimated_context_cost(&self) -> u64 {
        self.fields
            .iter()
            .map(|(_, value)| estimate_audit_component_tokens(value.len()) as u64)
            .sum()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EligibilityReason {
    CurrentTargetSnapshot,
    CurrentTargetIdentity,
    ProjectScopedMemory,
    HistoricalSummaryCurrentTarget,
    HistoricalComparisonSelected,
    CrossProjectExcluded,
    CurrentTargetMismatchExcluded,
    TargetVersionMismatchExcluded,
    HistoricalSummaryTargetUnknown,
    TargetUnknownExcluded,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AuthorityFreshnessTier {
    CurrentTargetResearch,
    CurrentTargetArtifactMetadata,
    CurrentTargetObservationMetadata,
    ProjectMemory,
    ExplicitHistoricalSummary,
}

impl AuthorityFreshnessTier {
    pub(crate) fn reason(self) -> &'static str {
        match self {
            Self::CurrentTargetResearch => "current_target_research_fact",
            Self::CurrentTargetArtifactMetadata => "current_target_artifact_metadata",
            Self::CurrentTargetObservationMetadata => "current_target_derived_observation_metadata",
            Self::ProjectMemory => "current_project_memory",
            Self::ExplicitHistoricalSummary => "explicitly_selected_historical_summary",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct ComponentRanks {
    pub(crate) exact_identifier: Option<usize>,
    pub(crate) fielded_bm25: Option<usize>,
    pub(crate) character_ngram: Option<usize>,
    pub(crate) reciprocal_rank_fusion: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct RetrievalResult {
    pub(crate) id: String,
    pub(crate) source: RetrievalSource,
    pub(crate) matched_fields: Vec<RetrievalField>,
    pub(crate) matched_identifiers: Vec<String>,
    pub(crate) component_ranks: ComponentRanks,
    pub(crate) tier: AuthorityFreshnessTier,
    pub(crate) tier_reason: &'static str,
    pub(crate) eligibility: EligibilityReason,
    pub(crate) estimated_context_cost: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct RetrievalExclusion {
    pub(crate) id: String,
    pub(crate) source: RetrievalSource,
    pub(crate) eligibility: EligibilityReason,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct RetrievalShadowReport {
    pub(crate) algorithm_version: &'static str,
    pub(crate) query_fingerprint: String,
    pub(crate) current_target: CurrentTarget,
    pub(crate) queried_candidate_count: usize,
    pub(crate) eligible_candidate_count: usize,
    pub(crate) results: Vec<RetrievalResult>,
    pub(crate) exclusions: Vec<RetrievalExclusion>,
    pub(crate) deferred_metrics: RetrievalDeferredMetrics,
}

/// Metrics whose required typed baseline or denominator does not exist in the
/// current fact model. Shadow reports name that absence rather than scoring it
/// as zero or claiming an unmeasured improvement.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct RetrievalDeferredMetrics {
    pub(crate) repeated_read_reduction: &'static str,
    pub(crate) estimated_context_cost_improvement: &'static str,
    pub(crate) reason: &'static str,
}

#[derive(Debug, Error)]
pub(crate) enum RetrievalError {
    #[error("retrieval {field} must not be empty")]
    RequiredText { field: &'static str },
    #[error("retrieval query is {actual} bytes, above the {maximum}-byte limit")]
    QueryTooLarge { actual: usize, maximum: usize },
    #[error("retrieval received {actual} candidates, above the {maximum}-candidate limit")]
    TooManyCandidates { actual: usize, maximum: usize },
    #[error("retrieval candidate `{id}` has no fields")]
    CandidateMissingFields { id: String },
    #[error("retrieval candidate `{id}` duplicates field `{field:?}")]
    CandidateDuplicateField { id: String, field: RetrievalField },
    #[error("retrieval candidate ID `{id}` is duplicated")]
    DuplicateCandidateId { id: String },
    #[error("failed to fingerprint retrieval query: {message}")]
    QueryFingerprint { message: String },
}

pub(crate) fn shadow_report(
    query: RetrievalQuery,
    candidates: Vec<RetrievalCandidate>,
) -> Result<RetrievalShadowReport, RetrievalError> {
    if candidates.len() > MAX_CANDIDATES {
        return Err(RetrievalError::TooManyCandidates {
            actual: candidates.len(),
            maximum: MAX_CANDIDATES,
        });
    }
    let mut ids = BTreeSet::new();
    for candidate in &candidates {
        if !ids.insert(candidate.id.as_str()) {
            return Err(RetrievalError::DuplicateCandidateId {
                id: candidate.id.clone(),
            });
        }
    }
    let query_fingerprint = canonical_blake3_hash(&QueryFingerprintInput {
        algorithm_version: ALGORITHM_VERSION,
        project_id: &query.project_id,
        current_target: &query.current_target,
        text: &query.text,
        historical_comparison: query.historical_comparison,
    })
    .map_err(|error| RetrievalError::QueryFingerprint {
        message: error.to_string(),
    })?;
    Ok(rank::rank(query, candidates, query_fingerprint))
}

#[derive(Serialize)]
struct QueryFingerprintInput<'a> {
    algorithm_version: &'static str,
    project_id: &'a str,
    current_target: &'a CurrentTarget,
    text: &'a str,
    historical_comparison: bool,
}

fn require_text(field: &'static str, value: &str) -> Result<(), RetrievalError> {
    if value.trim().is_empty() {
        Err(RetrievalError::RequiredText { field })
    } else {
        Ok(())
    }
}

fn truncate_utf8(mut value: String, maximum: usize) -> String {
    if value.len() <= maximum {
        return value;
    }
    let mut end = maximum;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
    value
}
