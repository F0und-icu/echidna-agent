//! Fixed, local, deterministic security-retrieval Shadow contracts.
//!
//! This module is deliberately a single versioned algorithm owner. It ranks
//! caller-supplied, bounded facts in memory and has no filesystem, Provider,
//! Tool, Event, Research, Memory, or Context mutation capability.

mod normalize;
mod rank;

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use thiserror::Error;

use crate::domain::canonical_json::canonical_blake3_hash;
use crate::domain::context::estimate_audit_component_tokens;

pub(crate) const ALGORITHM_VERSION: &str = "m4-s3f-deterministic-security-retrieval-v1";
pub(crate) const MAX_QUERY_BYTES: usize = 4 * 1024;
pub(crate) const MAX_CANDIDATES: usize = 512;
pub(crate) const MAX_FIELD_BYTES: usize = 16 * 1024;
pub(crate) const MAX_RESULTS: usize = 10;

/// Upper bound for the original UTF-8 bytes corresponding to a normalized
/// exact query. Each source scalar takes at most four bytes and contributes
/// at least one scalar to Unicode lowercase. Callers must use a window at
/// least this wide and retain enough overlap to include a cross-window match.
pub(crate) fn task_query_source_span_bound(query: &str) -> usize {
    normalize::normalize(query)
        .chars()
        .count()
        .saturating_mul(4)
}

/// The posting vocabulary uses the same normalization as task ranking. The
/// prefix distinguishes identifiers, lexical terms and character n-grams.
pub(crate) fn task_posting_keys(text: &str) -> Vec<String> {
    normalize::identifiers(text)
        .into_iter()
        .map(|key| format!("i:{key}"))
        .chain(
            normalize::normalized_terms(text)
                .into_iter()
                .map(|key| format!("t:{key}")),
        )
        .chain(
            normalize::ngrams(text)
                .into_iter()
                .map(|key| format!("n:{key}")),
        )
        .collect()
}

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
    TaskContext,
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
    TaskContextEligible,
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
    TaskContext,
}

impl AuthorityFreshnessTier {
    pub(crate) fn reason(self) -> &'static str {
        match self {
            Self::TaskContext => "scoped_task_context",
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

/// Incrementally ranks scoped navigation excerpts without retaining every
/// source body. Callers re-read selected original facts after ranking.
pub(crate) struct ScopedTaskRanker<'a> {
    project_id: &'a str,
    query: &'a str,
    pool: Vec<RetrievalCandidate>,
}

#[derive(Default)]
pub(crate) struct TaskRetrievalWork {
    pub(crate) source_window_bytes: usize,
    pub(crate) index_source_bytes: usize,
    pub(crate) needle_scan_bytes: usize,
    pub(crate) candidate_anchors: usize,
    pub(crate) local_gram_probes: usize,
    pub(crate) window_gram_probes: usize,
    pub(crate) rank_window_checks: usize,
    pub(crate) rank_window_bytes: usize,
}

/// One readback budget shared by anchor scoring and the returned source range.
pub(crate) struct TaskExcerptLayout<'a> {
    source: &'a str,
    character_offsets: Vec<usize>,
    reserve_characters: usize,
    maximum: usize,
}

impl<'a> TaskExcerptLayout<'a> {
    pub(crate) fn new(source: &'a str, query: &str, maximum: usize) -> Self {
        Self {
            source,
            character_offsets: source.char_indices().map(|(offset, _)| offset).collect(),
            reserve_characters: query.chars().count().saturating_mul(4),
            maximum,
        }
    }

    pub(crate) fn range(&self, offset: usize) -> std::ops::Range<usize> {
        let first = self
            .character_offsets
            .partition_point(|position| *position < offset);
        let after = first.saturating_add(self.reserve_characters);
        let reserve_end = self
            .character_offsets
            .get(after)
            .copied()
            .unwrap_or(self.source.len());
        let before = self.maximum.saturating_sub(reserve_end - offset).min(512);
        let mut start = offset.saturating_sub(before);
        while !self.source.is_char_boundary(start) {
            start += 1;
        }
        let mut end = self.source.len().min(start.saturating_add(self.maximum));
        while !self.source.is_char_boundary(end) {
            end -= 1;
        }
        start..end
    }
}

/// Locate and rank source excerpts using original UTF-8 byte offsets.
pub(crate) fn task_ranked_match_offset(
    project: &str,
    text: &str,
    query: &str,
    excerpt: &TaskExcerptLayout<'_>,
    base: usize,
    work: &mut TaskRetrievalWork,
) -> Result<Option<usize>, RetrievalError> {
    work.source_window_bytes += text.len();
    let mut normalized = String::new();
    let mut original_offsets = Vec::new();
    for (offset, character) in text.char_indices() {
        for lowered in character.to_lowercase() {
            normalized.push(lowered);
            original_offsets.extend(std::iter::repeat_n(offset, lowered.len_utf8()));
        }
    }
    let single_query = normalize::task_single_character_query(query);
    if let Some(single) = &single_query
        && !query.trim().chars().next().unwrap().is_ascii_alphanumeric()
    {
        return Ok(normalized
            .find(single)
            .map(|offset| original_offsets[offset]));
    }
    // An exact lexical hit must point to the token that actually contributed
    // to ranking, not an earlier weak gram whose window contains that token.
    let query_identifiers = normalize::exact_identifiers(query);
    let query_terms = normalize::normalized_terms(query)
        .into_iter()
        .chain(single_query.iter().cloned())
        .collect::<BTreeSet<_>>();
    let strong_needles = query_identifiers
        .iter()
        .cloned()
        .chain(query_terms.iter().cloned())
        .collect::<BTreeSet<_>>();
    // Every exact identifier component and term is a substring of the
    // normalized source. Only inspect source tokens when such a hit is
    // possible; the token checks below still decide whether it is exact.
    if strong_needles
        .iter()
        .any(|needle| normalized.contains(needle))
    {
        let mut scanned = 0;
        for raw in text.split_whitespace() {
            let start = scanned + text[scanned..].find(raw).expect("split token in source");
            scanned = start + raw.len();
            let identifiers = normalize::identifiers(raw);
            if let Some(identifier) = query_identifiers
                .iter()
                .find(|identifier| identifiers.contains(identifier))
            {
                let normalized_start = original_offsets.partition_point(|offset| *offset < start);
                let normalized_end = original_offsets.partition_point(|offset| *offset < scanned);
                if let Some(relative) =
                    normalized[normalized_start..normalized_end].find(identifier)
                {
                    return Ok(Some(original_offsets[normalized_start + relative]));
                }
            }
            let valid_terms = normalize::normalized_terms(raw);
            let mut token_offset = start;
            for token in raw.split(|character: char| !character.is_alphanumeric()) {
                let normalized_token = normalize::normalize(token);
                if query_terms.contains(&normalized_token)
                    && (valid_terms.contains(&normalized_token)
                        || (single_query.is_some() && token.chars().count() == 1))
                {
                    return Ok(Some(token_offset));
                }
                token_offset += token.len();
                if token_offset < scanned {
                    token_offset += text[token_offset..].chars().next().unwrap().len_utf8();
                }
            }
        }
    }
    // A single ASCII character has no n-grams. Its complete token and exact
    // identifier cases have already been checked above, so no remaining rank
    // component can accept it.
    if single_query.is_some() {
        return Ok(None);
    }
    let query_ngrams = normalize::ngrams(query);
    let query_characters = query_ngrams
        .iter()
        .flat_map(|gram| gram.chars())
        .collect::<BTreeSet<_>>();
    let needles = std::iter::once(normalize::normalize(query))
        .chain(normalize::identifiers(query))
        .chain(strong_needles.iter().cloned())
        .chain(query_ngrams.iter().cloned());
    let mut offsets = BTreeMap::new();
    for needle in needles.filter(|needle| !needle.is_empty()) {
        let is_gram = query_ngrams.contains(&needle);
        let mut from = 0;
        while let Some(relative) = normalized[from..].find(&needle) {
            work.needle_scan_bytes += relative + needle.len();
            let offset = from + relative;
            let original = original_offsets[offset];
            let end = if is_gram {
                let last = original_offsets[offset + needle.len() - 1];
                last + text[last..].chars().next().unwrap().len_utf8()
            } else {
                original
            };
            offsets
                .entry(original)
                .and_modify(|current: &mut usize| *current = (*current).max(end))
                .or_insert(end);
            from = offset + normalized[offset..].chars().next().unwrap().len_utf8();
        }
        work.needle_scan_bytes += normalized.len() - from;
    }
    if offsets.is_empty() {
        return Ok(None);
    }
    let mut context_start = base.saturating_sub(512);
    while !excerpt.source.is_char_boundary(context_start) {
        context_start += 1;
    }
    let mut context_end = excerpt
        .source
        .len()
        .min(base + text.len() + task_query_source_span_bound(query));
    while !excerpt.source.is_char_boundary(context_end) {
        context_end -= 1;
    }
    let mut ngram_index =
        normalize::TaskNgramIndex::new(&excerpt.source[context_start..context_end], &query_ngrams);
    work.index_source_bytes += context_end - context_start;
    // Score only grams visible in the actual bounded readback. The original
    // whole-window ranker remains a necessary source eligibility check.
    let local_characters = normalize::normalize(query)
        .chars()
        .count()
        .min(normalize::MAX_NGRAM_CHARS);
    let mut best = None;
    for (&offset, &end) in &offsets {
        work.candidate_anchors += 1;
        if end == offset {
            continue;
        }
        let source_offset = base + offset;
        let range = excerpt.range(source_offset);
        let first = excerpt
            .character_offsets
            .partition_point(|position| *position < source_offset);
        let query_end = excerpt
            .character_offsets
            .get(first.saturating_add(local_characters))
            .copied()
            .unwrap_or(excerpt.source.len());
        let end = range.end.min(query_end).min(context_end);
        if range.start >= end {
            continue;
        }
        let local_overlap = ngram_index.local_ngrams(
            range.start - context_start,
            end - context_start,
            &mut work.local_gram_probes,
        );
        // Equal visible evidence keeps the earlier source range. Unlike the
        // former query-sized score, neither range borrows unseen later bytes.
        if local_overlap == 0 || best.is_some_and(|(_, score)| score >= local_overlap) {
            continue;
        }
        let mut rank_start = offset.saturating_sub(128);
        while !text.is_char_boundary(rank_start) {
            rank_start -= 1;
        }
        let part = &text[rank_start..];
        // Exact source terms and identifiers were handled before this loop;
        // a substring elsewhere in a later window cannot bypass gram gating.
        if !ngram_index.may_match(
            &query_ngrams,
            base + rank_start - context_start,
            &mut work.window_gram_probes,
        ) {
            continue;
        }
        let local = &excerpt.source[range.start..end];
        // Whitespace consumes the actual readback range even though the
        // ngram vocabulary skips it. Excluding it here would let a tiny
        // cluster claim complete coverage of an otherwise empty excerpt.
        let visible_characters = local
            .chars()
            .flat_map(char::to_lowercase)
            .take(normalize::MAX_NGRAM_CHARS)
            .count();
        work.local_gram_probes +=
            visible_characters.saturating_sub(1) + visible_characters.saturating_sub(2);
        let local_ngrams = normalize::ngrams(local);
        let available_grams = query_ngrams.len().min(local_ngrams.len());
        if !normalize::ngram_overlap_matches(available_grams, local_overlap) {
            continue;
        }
        // Distinct grams alone can overstate repeated fragments: abcabcab
        // contains three query grams out of only six local distinct grams.
        // Use the same coverage rule on query characters against the visible
        // character budget, so repetition does not manufacture coverage.
        let local_characters = local_ngrams
            .iter()
            .flat_map(|gram| gram.chars())
            .filter(|character| query_characters.contains(character))
            .collect::<BTreeSet<_>>();
        let available_characters = query_characters.len().min(visible_characters);
        if !normalize::ngram_overlap_matches(available_characters, local_characters.len()) {
            continue;
        }
        work.rank_window_checks += 1;
        work.rank_window_bytes += part.len();
        if !rank::task_ngram_can_match(&query_ngrams, part) {
            continue;
        }
        let mut ranker = ScopedTaskRanker::new(project, query)?;
        work.rank_window_bytes += part.len();
        ranker.push("source", part)?;
        if !ranker.finish().is_empty() {
            best = Some((offset, local_overlap));
        }
    }
    Ok(best.map(|(offset, _)| offset))
}

impl<'a> ScopedTaskRanker<'a> {
    pub(crate) fn new(project_id: &'a str, query: &'a str) -> Result<Self, RetrievalError> {
        require_text("project_id", project_id)?;
        require_text("query", query)?;
        Ok(Self {
            project_id,
            query,
            pool: Vec::new(),
        })
    }

    pub(crate) fn push(&mut self, id: &str, text: &str) -> Result<(), RetrievalError> {
        if self.pool.len() == MAX_CANDIDATES {
            let ranked = rank::rank_task(self.query, self.pool.clone())
                .into_iter()
                .take(MAX_CANDIDATES / 2)
                .collect::<BTreeSet<_>>();
            self.pool.retain(|candidate| ranked.contains(&candidate.id));
        }
        self.pool.push(RetrievalCandidate::new(
            id,
            self.project_id,
            RetrievalSource::TaskContext,
            CandidateTarget::ProjectScoped,
            vec![(RetrievalField::Body, text.to_owned())],
        )?);
        Ok(())
    }

    pub(crate) fn finish(self) -> Vec<String> {
        rank::rank_task(self.query, self.pool)
    }
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
