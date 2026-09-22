# R5c1 — Local retrieval Shadow cost baseline

2026-09-20. **OFFLINE_MEASUREMENT_RECORDED**, main-agent investigation only.
Not an implementation Slice, independent audit, promotion decision or full Gate.
Base: `943cb9414e166460040d8b5febc66fb3969b2782`, R6 Draft PR #84 (unmerged).
No production/test/architecture changes, Provider calls or credentials.

## Method and evidence

Formal `cyber-agent inspect retrieval`, unchanged debug binary SHA256
`61f80225269ca2941d40ebbdfe3de373db7e225c51649ad94bc6cef3bc3073d3`.
The existing external TargetSnapshot helper was recompiled against the current
library and writes fixture setup through the production Writer/Reducer. No new
public API, benchmark hook or copied ranker. Source and binary hashes remained
unchanged throughout the final run.

Four fresh ordinary-engineering projects:8/64 one-section Memory documents,
512/4096 ASCII body bytes each. The final fixture counts include decisions.md
and failed-paths.md; other documents are notes. Queries contain calculate_sum,
alone or with16 distinct ordinary terms present in the relevant note. Three
repetitions per cell in rotated order;24 timed processes,30seconds/process and
360seconds total bound. Completed in25.719seconds including setup/checks.

Evidence: `/tmp/cyber-agent-r5c1-shadow-v3.G3ZcZE/` contains frozen baseline,
external measure.py, run.log,24 raw JSON reports, result.json and retrieval-tests.log.
The host string reported by Python is macOS-10.16-x86_64-i386-64bit; do not infer
hardware/native-process architecture or exact current macOS release from it.

Each timed interval includes process startup, Project loading, candidate
projection, ranking and JSON output. File-tree hash checks occur outside the
timing interval. Filesystem cache and host load are uncontrolled; no timing
threshold or statistically significant causal improvement is claimed.

## Results

Milliseconds, three retained samples per cell:

| Memory sections | Body bytes | Extra query terms | Median | Min–max |
|---:|---:|---:|---:|---:|
|8|512|0|44.617|41.457–45.733|
|8|512|16|134.665|134.259–143.656|
|8|4096|0|101.746|100.732–105.330|
|8|4096|16|741.360|733.142–742.182|
|64|512|0|190.409|189.421–204.582|
|64|512|16|838.963|833.061–942.414|
|64|4096|0|649.816|644.683–660.129|
|64|4096|16|5453.358|5442.336–5765.513|

All24 reports: status ok, Shadow true, model_context_changed false and every
reported effect zero. Complete report content deterministic across repetitions
within each cell; relevant note exact-identifier hit at rank1;9/65 candidates
including the TargetSnapshot. Project file contents unchanged. These fixtures
establish a simple known-hit property, not real-corpus recall/precision or a
comparison against another retriever. Existing retrieval_shadow suite6/6 PASS
separately covers its controlled identifier/CJK/typo/filter/ordering/bounds cases.

## Preserved preparation failures and boundary observation

1. Initial directory `/tmp/cyber-agent-r5c1-shadow.U0WnTs/` stopped before any
   timed observation: fresh init does not create optional Memory directories.
   External fixture setup used mkdir without parents=True. No product defect.
2. `/tmp/cyber-agent-r5c1-shadow-v2.trKYSA/` retained4 initial samples, then failed
   its count assertion on64 notes. Production read_retrieval_documents appends
   decisions.md/failed-paths.md, sorts and truncates paths at64, then filters
   missing documents. Thus two absent fixed paths consume slots:62 notes plus
   one snapshot produce63 candidates, not65. Separate read-only reproduction is
   document-cap-observation.json. This quota behavior is a repair candidate;
   it was not fixed or concealed by loosening the assertion.

The final fresh fixture includes both fixed files inside the8/64 document count,
keeps the original sections+1 assertion, and does not pool earlier samples.
Original failed scripts/fixtures/logs remain intact. All data is synthetic;
no security task, model, live budget or credential was used.

## Decision and next bounded work

No new algorithm or cache is justified yet. The measured query-length sensitivity
and source inspection identify a plausible cost owner: bm25_ranks repeatedly
calls normalized_terms for each field/candidate/query term, including document
frequency, score and field weights. Whole-process measurements do not isolate
that function. Retain the current fixed ranking semantics and Shadow boundary.

Priority1: reproduce the missing-file document-quota boundary as a permanent
formal-CLI regression and decide whether the bound counts paths or present
documents. Any correction stays in the existing Memory loader, preserves
symlink/invalid-path/size errors and deterministic selection, and must not create
a second eligibility owner or unbounded content reads.

Priority2: measure the same controlled workload in release and profile before
optimizing. If justified, evaluate private per-call reuse of normalized fields
inside the existing ranker; no persistent cache, invalidation framework or changed
BM25/identifier/ngram/RRF/tier semantics. Require exact report equality over
quality/boundary fixtures, paired cost measurements, narrow tests, affected tests,
independent finite audit, fixes, full Gate and Lean before retaining any change.

This investigation does not authorize model-visible retrieval, RAG services,
vector indexes, remote judges, new Provider tests, automatic merge or Actions.
No release-performance, billed-token, latency-improvement or model-quality claim.
