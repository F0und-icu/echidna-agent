# R6 follow-up — Release retrieval cost and local profile

2026-09-20. **OFFLINE_MEASUREMENT_RECORDED**, main-agent measurement, not a new
independent audit, algorithm implementation or whole-R6 acceptance claim.
Historical R5c1/R5c2 identifiers remain traceable; the mainline stays R6.
Retrieval optimization is not a mandatory R6 prerequisite without measured
blocking impact on an acceptance workflow.

## Candidate and method

HEAD `247f13ccb16acec3a05f877278801da84fdcf377` plus unchanged audited R5c2 patch;
all319 source/test entries match its review freeze. No new runtime/test changes.
`cargo build --release --locked --offline` PASS; release retrieval_shadow9/9
PASS. Normal release build reran before freezing the binary. Existing external
TargetSnapshot helper was recompiled against the current release library.

Binary SHA256: `8d6f0be57c41ebd8c0090ffcadc167f66aae34f89064ebde67f086a682b01fe0`.
Mach-O x86_64; macOS15.7.5 (24G624), confirmed separately from Python's platform
compatibility string. No profile overrides, dependencies or public benchmark API.

Evidence `/tmp/cyber-agent-r6-retrieval-release.W11cJL/`: build.log,
release-tests.log, helper-build.log, started.json, measure.py,24 raw case JSONs,
run.log, result.json, compare.py/comparison.json, profile.py/profile.txt,
profile-result.json, sampler output and source-freeze-check.log.

Same final R5c1 fixture shape, four fresh ordinary-engineering Projects:
8/64 one-section Memory documents,512/4096 ASCII body bytes. Both fixed Memory
files are present within the document count. Queries are calculate_sum alone
or plus16 ordinary terms. Three repetitions per cell with rotated ordering;
all samples retained.30seconds/process and360seconds lane limit. Completed24
observations in3.690seconds including setup/checks. Zero Provider/Tool calls.

Timing includes CLI startup, loading, projection, ranking and JSON; tree hashing
is outside each interval. Host load/filesystem cache uncontrolled; descriptive
data, not statistical significance or isolated ranker timing.

| Sections | Body bytes | Extra terms | Median ms | Min–max ms |
|---:|---:|---:|---:|---:|
|8|512|0|14.344|12.848–15.843|
|8|512|16|24.167|22.708–28.034|
|8|4096|0|20.484|20.177–21.148|
|8|4096|16|83.835|82.040–84.510|
|64|512|0|39.473|38.980–41.034|
|64|512|16|118.557|108.187–121.097|
|64|4096|0|94.767|89.752–96.079|
|64|4096|16|593.582|568.539–639.030|

All24 reports deterministic within their cell; known identifier at rank1;
9/65 candidates, Shadow/no model-context change/zero effects; fixture/source/
binary bytes unchanged. Eight comparisons against R5c1 reports match after
excluding only query_fingerprint, which includes the fresh Project identity.
Algorithm version, target, ranks, matches, eligibility, cost estimates and
exclusions remain compared unchanged. Not real-corpus recall/precision.

Prior5.453second debug and current0.594second release measurements are different
builds/runs, not a paired code-optimization experiment. No algorithm speedup,
billed-token benefit or causal Agent latency improvement is claimed.

## One bounded stress-query profile

One extra formal inspect retrieval process against64×4096 fixture, with
calculate_sum plus256 terms (1951bytes, within existing query limits). Process
bound30seconds. Own child observed by `/usr/bin/sample` for1second at requested
1ms intervals; CLI/sampler both exit0. No elevated permissions or other-process
inspection. Known-hit/count/Shadow/zero-effect/unchanged-source checks PASS.

Observer-inclusive duration7.330seconds includes sampling overhead, not an
uninstrumented latency. Captured642 main-thread stack samples;638 contain
normalized_terms in the call graph. This describes the sampled stress-query
window only, not99% of every complete query's wall time. Lowercasing and string
allocation appear under normalization/rank. Together with repeated source calls
from candidate_term_frequency, candidate_length and field_weight_sum, this
supports per-query normalization reuse as the next candidate, not a proven gain.

## Next finite implementation candidate — not started

Reuse field terms/statistics once per eligible candidate per query inside the
existing private ranker and normalizer. Preserve identifier extraction, per-field
deduplication, term limits, field weights, floating-point operation order,
eligibility, n-gram/RRF/tier sorting and stable tie-breaks. No persistent cache,
invalidation service, registry, dependency, Provider call or public test API.
Query-local data must stay bounded and be dropped after the call; do not
precompute excluded candidates or expand Memory/Context authority.

Require identical-fixture baseline/candidate release measurements in alternating
order, exact complete report equality for fixed Project/query identities,
existing quality/boundary tests and no short-query regression. Retain only
measured benefit without disproportionate code/space cost. Sequence: focused →
affected → independently authorized finite audit → fixes → full Gate → Lean.

No Live/credentials/Actions/Git commit/push/merge/security targets/Codex login
access. No new full Gate or independent measurement audit claimed; prior R5c2
Gate remains valid for unchanged source. This is a docs/external-harness-only
investigation; incoming uncommitted R5c2 changes are preserved.
