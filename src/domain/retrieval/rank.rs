use std::collections::{BTreeMap, BTreeSet};

use super::normalize::{
    exact_identifiers, identifiers, ngram_similarity, ngrams, normalized_terms,
};
use super::{
    ALGORITHM_VERSION, AuthorityFreshnessTier, CandidateTarget, ComponentRanks, EligibilityReason,
    MAX_RESULTS, RetrievalCandidate, RetrievalExclusion, RetrievalField, RetrievalQuery,
    RetrievalResult, RetrievalShadowReport, RetrievalSource,
};

const BM25_K1: f64 = 1.2;
const BM25_B: f64 = 0.75;
const RRF_K: f64 = 60.0;

pub(super) fn rank(
    query: RetrievalQuery,
    candidates: Vec<RetrievalCandidate>,
    query_fingerprint: String,
) -> RetrievalShadowReport {
    let query_terms = normalized_terms(&query.text);
    let mut exclusions = Vec::new();
    let mut eligible = Vec::new();

    for candidate in candidates {
        match eligibility(&query, &candidate) {
            Ok((reason, tier)) => eligible.push(RankedCandidate::new(
                candidate,
                reason,
                tier,
                !query_terms.is_empty(),
            )),
            Err(reason) => exclusions.push(RetrievalExclusion {
                id: candidate.id,
                source: candidate.source,
                eligibility: reason,
            }),
        }
    }
    exclusions.sort_by(|left, right| left.id.cmp(&right.id));
    let queried_candidate_count = eligible.len() + exclusions.len();
    let eligible_candidate_count = eligible.len();

    score_and_order(&query.text, &mut eligible);

    let results = eligible
        .into_iter()
        .take(MAX_RESULTS)
        .map(|candidate| {
            let estimated_context_cost = candidate.candidate.estimated_context_cost();
            RetrievalResult {
                id: candidate.candidate.id,
                source: candidate.candidate.source,
                matched_fields: candidate.matched_fields,
                matched_identifiers: candidate.matched_identifiers,
                component_ranks: ComponentRanks {
                    exact_identifier: candidate.exact_rank,
                    fielded_bm25: candidate.bm25_rank,
                    character_ngram: candidate.ngram_rank,
                    reciprocal_rank_fusion: candidate.rrf_rank,
                },
                tier: candidate.tier,
                tier_reason: candidate.tier.reason(),
                eligibility: candidate.eligibility,
                estimated_context_cost,
            }
        })
        .collect();

    RetrievalShadowReport {
        algorithm_version: ALGORITHM_VERSION,
        query_fingerprint,
        current_target: query.current_target,
        queried_candidate_count,
        eligible_candidate_count,
        results,
        exclusions,
        deferred_metrics: super::RetrievalDeferredMetrics {
            repeated_read_reduction: "unavailable",
            estimated_context_cost_improvement: "unavailable",
            reason: "no typed baseline or complete denominator is recorded by this Shadow slice",
        },
    }
}

fn score_and_order(query: &str, eligible: &mut Vec<RankedCandidate>) {
    let query_identifiers = identifiers(query);
    let query_exact_identifiers = exact_identifiers(query);
    let query_terms = normalized_terms(query);
    let query_ngrams = ngrams(query);
    let exact = identifier_ranks(eligible, &query_exact_identifiers);
    let bm25 = bm25_ranks(eligible, &query_terms);
    let ngram = ngram_ranks(eligible, &query_ngrams);
    for candidate in eligible.iter_mut() {
        candidate.exact_rank = exact.get(&candidate.candidate.id).copied();
        candidate.bm25_rank = bm25.get(&candidate.candidate.id).copied();
        candidate.ngram_rank = ngram.get(&candidate.candidate.id).copied();
        candidate.rrf = [
            candidate.exact_rank,
            candidate.bm25_rank,
            candidate.ngram_rank,
        ]
        .into_iter()
        .flatten()
        .map(|rank| 1.0 / (RRF_K + rank as f64))
        .sum();
        candidate.matched_identifiers =
            matched_identifiers(&candidate.candidate, &query_exact_identifiers);
        candidate.matched_fields =
            matched_fields(candidate, &query_terms, &query_identifiers, &query_ngrams);
    }
    eligible.retain(|candidate| candidate.rrf > 0.0);
    eligible.sort_by(|left, right| {
        right
            .rrf
            .total_cmp(&left.rrf)
            .then_with(|| left.candidate.id.cmp(&right.candidate.id))
    });
    for (index, candidate) in eligible.iter_mut().enumerate() {
        candidate.rrf_rank = index + 1;
    }
    eligible.sort_by(|left, right| {
        left.tier
            .cmp(&right.tier)
            .then_with(|| left.rrf_rank.cmp(&right.rrf_rank))
            .then_with(|| left.candidate.id.cmp(&right.candidate.id))
    });
}

pub(super) fn rank_task(query: &str, candidates: Vec<RetrievalCandidate>) -> Vec<String> {
    let has_terms = !normalized_terms(query).is_empty();
    let mut eligible = candidates
        .into_iter()
        .map(|candidate| {
            RankedCandidate::new(
                candidate,
                EligibilityReason::TaskContextEligible,
                AuthorityFreshnessTier::TaskContext,
                has_terms,
            )
        })
        .collect::<Vec<_>>();
    score_and_order(query, &mut eligible);
    eligible
        .into_iter()
        .map(|candidate| candidate.candidate.id)
        .collect()
}

/// Reuse the ranker's n-gram threshold when no lexical or identifier hit is
/// possible in a bounded source window.
pub(super) fn task_ngram_can_match(query_ngrams: &BTreeSet<String>, text: &str) -> bool {
    ngram_similarity(query_ngrams, &ngrams(text)).is_some()
}

struct RankedCandidate {
    candidate: RetrievalCandidate,
    field_terms: Vec<Vec<String>>,
    eligibility: EligibilityReason,
    tier: AuthorityFreshnessTier,
    exact_rank: Option<usize>,
    bm25_rank: Option<usize>,
    ngram_rank: Option<usize>,
    rrf: f64,
    rrf_rank: usize,
    matched_identifiers: Vec<String>,
    matched_fields: Vec<RetrievalField>,
}

impl RankedCandidate {
    fn new(
        candidate: RetrievalCandidate,
        eligibility: EligibilityReason,
        tier: AuthorityFreshnessTier,
        has_lexical_query: bool,
    ) -> Self {
        // Query-local and built only after eligibility. Preserve each field's
        // independent normalization limit and deduplication semantics.
        let field_terms = candidate
            .fields
            .iter()
            .map(|(_, value)| {
                if has_lexical_query {
                    normalized_terms(value)
                } else {
                    Vec::new()
                }
            })
            .collect();
        Self {
            candidate,
            field_terms,
            eligibility,
            tier,
            exact_rank: None,
            bm25_rank: None,
            ngram_rank: None,
            rrf: 0.0,
            rrf_rank: 0,
            matched_identifiers: Vec::new(),
            matched_fields: Vec::new(),
        }
    }
}

fn eligibility(
    query: &RetrievalQuery,
    candidate: &RetrievalCandidate,
) -> Result<(EligibilityReason, AuthorityFreshnessTier), EligibilityReason> {
    if candidate.project_id != query.project_id {
        return Err(EligibilityReason::CrossProjectExcluded);
    }
    let current = &query.current_target;
    match &candidate.target {
        CandidateTarget::ProjectScoped if candidate.source == RetrievalSource::ProjectMemory => {
            Ok((
                EligibilityReason::ProjectScopedMemory,
                AuthorityFreshnessTier::ProjectMemory,
            ))
        }
        CandidateTarget::ProjectScoped => Err(EligibilityReason::TargetUnknownExcluded),
        CandidateTarget::Snapshot(snapshot) if snapshot == &current.snapshot_id => Ok(
            current_target_tier(candidate, EligibilityReason::CurrentTargetSnapshot),
        ),
        CandidateTarget::SnapshotSet(snapshots)
            if snapshots
                .iter()
                .any(|snapshot| snapshot == &current.snapshot_id) =>
        {
            if candidate.source == RetrievalSource::HistoricalSummary {
                Ok((
                    EligibilityReason::HistoricalSummaryCurrentTarget,
                    AuthorityFreshnessTier::ExplicitHistoricalSummary,
                ))
            } else {
                Ok(current_target_tier(
                    candidate,
                    EligibilityReason::CurrentTargetSnapshot,
                ))
            }
        }
        CandidateTarget::TargetIdentity { target_id, version }
            if target_id == &current.target_id
                && version
                    .as_deref()
                    .is_none_or(|version| version == current.version) =>
        {
            Ok(current_target_tier(
                candidate,
                EligibilityReason::CurrentTargetIdentity,
            ))
        }
        CandidateTarget::Snapshot(_) | CandidateTarget::SnapshotSet(_)
            if candidate.source == RetrievalSource::HistoricalSummary
                && query.historical_comparison =>
        {
            Ok((
                EligibilityReason::HistoricalComparisonSelected,
                AuthorityFreshnessTier::ExplicitHistoricalSummary,
            ))
        }
        CandidateTarget::Snapshot(_) | CandidateTarget::SnapshotSet(_)
            if candidate.source == RetrievalSource::HistoricalSummary =>
        {
            Err(EligibilityReason::CurrentTargetMismatchExcluded)
        }
        CandidateTarget::Snapshot(_) | CandidateTarget::SnapshotSet(_) => {
            Err(EligibilityReason::CurrentTargetMismatchExcluded)
        }
        CandidateTarget::TargetIdentity { target_id, version }
            if target_id == &current.target_id
                && version
                    .as_deref()
                    .is_some_and(|version| version != current.version) =>
        {
            Err(EligibilityReason::TargetVersionMismatchExcluded)
        }
        CandidateTarget::TargetIdentity { .. } => {
            Err(EligibilityReason::CurrentTargetMismatchExcluded)
        }
        CandidateTarget::Unknown if candidate.source == RetrievalSource::HistoricalSummary => {
            Err(EligibilityReason::HistoricalSummaryTargetUnknown)
        }
        CandidateTarget::Unknown => Err(EligibilityReason::TargetUnknownExcluded),
    }
}

fn current_target_tier(
    candidate: &RetrievalCandidate,
    reason: EligibilityReason,
) -> (EligibilityReason, AuthorityFreshnessTier) {
    let tier = match candidate.source {
        RetrievalSource::Research => AuthorityFreshnessTier::CurrentTargetResearch,
        RetrievalSource::ArtifactMetadata => AuthorityFreshnessTier::CurrentTargetArtifactMetadata,
        RetrievalSource::ObservationMetadata => {
            AuthorityFreshnessTier::CurrentTargetObservationMetadata
        }
        RetrievalSource::ProjectMemory => AuthorityFreshnessTier::ProjectMemory,
        RetrievalSource::HistoricalSummary => AuthorityFreshnessTier::ExplicitHistoricalSummary,
        RetrievalSource::TaskContext => AuthorityFreshnessTier::TaskContext,
    };
    (reason, tier)
}

fn identifier_ranks(
    candidates: &[RankedCandidate],
    query_identifiers: &[String],
) -> BTreeMap<String, usize> {
    ranked_ids(candidates, |candidate| {
        let candidate_identifiers = candidate_identifiers(&candidate.candidate);
        query_identifiers
            .iter()
            .filter(|identifier| candidate_identifiers.contains(*identifier))
            .count() as f64
    })
}

fn bm25_ranks(candidates: &[RankedCandidate], query_terms: &[String]) -> BTreeMap<String, usize> {
    if query_terms.is_empty() || candidates.is_empty() {
        return BTreeMap::new();
    }
    let average_length = candidates
        .iter()
        .map(|candidate| candidate_length(candidate) as f64)
        .sum::<f64>()
        / candidates.len() as f64;
    let document_frequency = query_terms
        .iter()
        .map(|term| {
            let count = candidates
                .iter()
                .filter(|candidate| candidate_term_frequency(candidate, term) > 0.0)
                .count();
            (term, count)
        })
        .collect::<BTreeMap<_, _>>();
    ranked_ids(candidates, |candidate| {
        let length = candidate_length(candidate) as f64;
        query_terms
            .iter()
            .map(|term| {
                let frequency = candidate_term_frequency(candidate, term);
                if frequency == 0.0 {
                    return 0.0;
                }
                let documents = *document_frequency.get(term).unwrap_or(&0) as f64;
                let inverse_frequency =
                    ((candidates.len() as f64 - documents + 0.5) / (documents + 0.5) + 1.0).ln();
                let denominator = frequency
                    + BM25_K1 * (1.0 - BM25_B + BM25_B * length / average_length.max(1.0));
                field_weight_sum(candidate, term) * inverse_frequency * frequency * (BM25_K1 + 1.0)
                    / denominator
            })
            .sum()
    })
}

fn ngram_ranks(
    candidates: &[RankedCandidate],
    query_ngrams: &BTreeSet<String>,
) -> BTreeMap<String, usize> {
    if query_ngrams.is_empty() {
        return BTreeMap::new();
    }
    ranked_ids(candidates, |candidate| {
        candidate
            .candidate
            .fields
            .iter()
            .filter_map(|(_, value)| ngram_similarity(query_ngrams, &ngrams(value)))
            .max_by(f64::total_cmp)
            .unwrap_or(0.0)
    })
}

fn ranked_ids(
    candidates: &[RankedCandidate],
    score: impl Fn(&RankedCandidate) -> f64,
) -> BTreeMap<String, usize> {
    let mut scored = candidates
        .iter()
        .map(|candidate| (candidate.candidate.id.as_str(), score(candidate)))
        .filter(|(_, score)| *score > 0.0)
        .collect::<Vec<_>>();
    scored.sort_by(|left, right| right.1.total_cmp(&left.1).then_with(|| left.0.cmp(right.0)));
    scored
        .into_iter()
        .enumerate()
        .map(|(index, (id, _))| (id.to_owned(), index + 1))
        .collect()
}

fn candidate_identifiers(candidate: &RetrievalCandidate) -> BTreeSet<String> {
    candidate
        .fields
        .iter()
        .flat_map(|(_, value)| identifiers(value))
        .collect()
}

fn candidate_length(candidate: &RankedCandidate) -> usize {
    candidate
        .field_terms
        .iter()
        .map(Vec::len)
        .sum::<usize>()
        .max(1)
}

fn candidate_term_frequency(candidate: &RankedCandidate, term: &str) -> f64 {
    candidate
        .field_terms
        .iter()
        .flatten()
        .filter(|candidate_term| candidate_term.as_str() == term)
        .count() as f64
}

fn field_weight_sum(candidate: &RankedCandidate, term: &str) -> f64 {
    candidate
        .candidate
        .fields
        .iter()
        .zip(&candidate.field_terms)
        .filter(|(_, terms)| terms.iter().any(|value| value == term))
        .map(|((field, _), _)| match field {
            RetrievalField::Title => 3.0,
            RetrievalField::Identifiers => 5.0,
            RetrievalField::Target => 2.0,
            RetrievalField::Body => 1.0,
            RetrievalField::Metadata => 1.5,
        })
        .sum()
}

fn matched_identifiers(
    candidate: &RetrievalCandidate,
    query_identifiers: &[String],
) -> Vec<String> {
    let candidate_identifiers = candidate_identifiers(candidate);
    query_identifiers
        .iter()
        .filter(|identifier| candidate_identifiers.contains(*identifier))
        .cloned()
        .collect()
}

fn matched_fields(
    candidate: &RankedCandidate,
    query_terms: &[String],
    query_identifiers: &[String],
    query_ngrams: &BTreeSet<String>,
) -> Vec<RetrievalField> {
    candidate
        .candidate
        .fields
        .iter()
        .zip(&candidate.field_terms)
        .filter_map(|((field, value), terms)| {
            let field_identifiers = identifiers(value);
            (query_terms.iter().any(|term| terms.contains(term))
                || query_identifiers
                    .iter()
                    .any(|identifier| field_identifiers.contains(identifier))
                || ngram_similarity(query_ngrams, &ngrams(value)).is_some())
            .then_some(*field)
        })
        .collect()
}
