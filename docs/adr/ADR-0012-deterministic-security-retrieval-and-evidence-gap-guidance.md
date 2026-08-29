# ADR-0012: Deterministic Security Retrieval and Evidence-Gap Guidance

- Status: ACCEPTED
- Date: 2026-08-29
- Deciders: project owner
- Architecture version: 1.3.0
- Related Slices: M4-S3f, M4-S3g, M4-S4
- Supersedes: generic recency-only or prose-only retrieval as the planned research strategy
- Superseded by: none

## Context

Cyber Agent reserves Context slots for Project Memory and retrieved fragments but does not yet implement a production retrieval algorithm. Generic Coding Agent retrieval optimizes for likely relevance to an ordinary task or later Tool invocation. Vulnerability research instead depends on exact identifiers, target/version applicability, evidence authority, negative controls, failed experiments and the next unresolved proof boundary.

The latest inspected Codex source contains temporary Shadow experiments for weighted lexical scoring, fielded BM25, character n-grams, multi-query decomposition, RRF and recent-use fusion. The experiments are deterministic, bounded and include CJK coverage, but duplicate normalization/bounds across implementations and use later Skill invocation as a noisy label. They are reference material, not a production subsystem to port.

## Constraints

- Retrieval is local and sends no telemetry.
- Markdown, Research Events and Artifact metadata remain facts; indexes are deletable Cache.
- No default vector or graph database.
- No remote embedding, reranker or model-judge service.
- Retrieval cannot merge Research records or upgrade Findings.
- ContextPlanner remains the final token-budget and inclusion authority.
- Candidate pools retain separate eligibility rules even when ranking primitives are shared.
- New optimization must prove benefit in local Replay/Shadow Eval before becoming model-visible.

## Decision

Build one fixed, versioned vulnerability-research retrieval algorithm and a separate pure evidence-gap guidance projection. Do not create a selector registry or general retrieval framework.

### Candidate Pools

The initial candidate pools are:

- Project Memory documents and bounded sections;
- Research Hypothesis, Experiment, Finding and NextAction records;
- Artifact metadata and Observation metadata, not unbounded Blob bodies;
- explicitly selected historical RequestAudit/Trace summaries when a caller requests history.

Each pool filters Project identity, record state, Artifact availability and allowed source kinds before ranking. Cross-project candidates are impossible. Cross-target or older-version candidates are excluded by default and can only enter a query view that explicitly requests historical comparison.

### Security Identifier Extraction

One versioned normalizer preserves exact security identifiers before ordinary tokenization:

- CVE/CWE and other advisory identifiers;
- package/bundle IDs, class names, methods and symbols;
- URL scheme, host, path, query parameter and header names;
- Android permission, Intent action/category, component and URI scheme;
- binary import/export, function, string and address-like tokens;
- hashes and Artifact/Receipt/Experiment/Finding IDs;
- target, build, version and environment fingerprints.

Normalization is Unicode-aware, bounded and deterministic. It does not discard punctuation that is semantic inside identifiers.

### Query Views

The query is a bounded set of deterministic views rather than one concatenated prompt:

1. current operator input;
2. exact identifiers extracted from that input;
3. active DomainProfile and ResearchPhase terms;
4. active Hypothesis entry/sink/class fields;
5. current TargetSnapshot/version/environment;
6. derived evidence-gap terms when available.

No model call is used to generate query views.

### Ranking Pipeline

The fixed first algorithm is:

```text
eligibility and TargetSnapshot filters
  -> exact identifier matching
  -> fielded BM25
  -> CJK/ASCII character n-gram ranking
  -> reciprocal-rank fusion
  -> authority/freshness tiered rerank
  -> stable ID tie-break
```

Authority and freshness do not overwrite lexical relevance through one opaque float. They form explicit deterministic tiers. Every result records:

- algorithm version and query fingerprint;
- matched fields and exact identifiers;
- component ranks and fused rank;
- target/freshness disposition;
- eligibility, inclusion or exclusion reason;
- estimated Context cost.

ContextPlanner makes the final inclusion decision using the existing budget and manifest.

### Index and Cache Boundary

The first implementation may scan bounded local sources and use an in-memory derived index. A persistent SQLite FTS or other index requires measured latency evidence, a deletable rebuild path and a separate dependency decision. Vector retrieval remains deferred until lexical/identifier retrieval demonstrably misses relevant material that controlled fixtures can identify.

### Shadow-First Promotion

Retrieval begins in local Shadow mode:

- it computes candidates but does not alter model-visible Context;
- controlled fixtures declare acceptable relevant reference sets;
- real-session evaluation may observe later referenced Research/Artifact IDs, but later model use is treated as a noisy signal rather than ground truth;
- reports remain derived non-evidence;
- promotion requires improved recall/precision or reduced repeated reads/Token use under a fixed cost budget.

### Evidence-Gap Projection

One pure projection derives only gaps supported by typed Research fields and EvidenceLinks:

```text
entry_unproven
sink_unproven
reachability_unproven
prerequisite_unproven
positive_control_missing
negative_control_missing
environment_control_missing
impact_unproven
repeatability_missing
target_or_version_binding_missing
artifact_or_receipt_provenance_missing
contradiction_unresolved
```

Unknown is distinct from missing. A gap is derived guidance, not a Research Event.

### Experiment/Next-Action Guidance

The first guidance implementation ranks existing typed NextActions or caller-supplied Eval candidates. It does not generate a workflow or autonomously execute an experiment.

Ranking uses a versioned factor vector:

```text
boundary importance
number of competing hypotheses discriminated
expected gap closure
reproducibility
novelty versus prior action_fingerprint
TargetSnapshot applicability
execution cost
mutation risk
version-drift risk
```

Hard invalidity and missing prerequisites filter before ranking. Factors use bounded explicit values and stable reason codes; reports show every contribution and tie-break. No hidden learned weight or opaque model confidence is accepted.

The guidance result:

- does not change ResearchPhase;
- does not execute a Tool;
- does not create an Experiment;
- does not upgrade a Finding;
- does not become evidence;
- can be ignored or overridden by the operator.

Adaptive or learned ranking is deferred until enough local outcomes exist. If later considered, it runs Shadow-only against the fixed deterministic baseline before any promotion.

## Consequences

### Positive

- Retrieval understands identifiers and evidence boundaries unique to vulnerability work.
- CJK and typo tolerance do not require remote embeddings.
- Ranking remains explainable and reproducible.
- Memory retrieval, Research retrieval and Deferred Tool discovery may eventually reuse proven normalization/ranking primitives without sharing candidate eligibility.

### Negative

- Controlled relevance fixtures and local metrics are required.
- Exact identifier extraction needs domain-specific tests across Source/Web/Mobile/Binary.
- Evidence-gap quality depends on typed EvidenceLinks rather than free-form notes.

### Rejected Alternatives

- No direct copy of Codex temporary selectors.
- No single global similarity score across every candidate kind.
- No automatic semantic merge of Research records.
- No remote model judge or telemetry label.
- No vector database by default.
- No general workflow or experiment-template DSL.

## Validation

- Same query/source versions produce identical ranking and reasons.
- CJK, ASCII identifiers, URLs, package names and symbols have permanent fixtures.
- Current target results cannot be displaced by incompatible stale evidence without an explicit historical query.
- Deleted Cache rebuilds from facts.
- Shadow execution performs no Provider or Tool action and writes no Research/Memory fact.
- Promotion is tied to measured quality/efficiency and can be rolled back by deleting the derived index/configured activation.
