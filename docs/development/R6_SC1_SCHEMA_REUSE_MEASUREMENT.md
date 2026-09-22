# R6-SC1 — Measured input Schema reuse

2026-09-20. Local scope only; status and verification: [STATUS](STATUS.md).
Evidence: `/tmp/cyber-agent-r6-sc1.hzepR3/`. No new Live, Git or Actions.

## Decision and rejected experiment

R6-04's eight successful requests each carried 24 Tool definitions, 33767
canonical JSON bytes. This is a measured cost candidate, not proof of a transport
bug, excessive latency or model incapability. Only input Schemas are sent in the
Responses Tool channel; output Schemas must not be counted as wire savings.

An offline greedy common-subexpression prototype suggested1710bytes/5.06%
savings. A conservative runtime implementation preserving existing definitions
and rejecting scope/reference movement saved only850bytes across all28 inputs.
Debug catalog construction increased from roughly8ms to24ms. It was rejected:
the new runtime module and unreviewed ADR-0031/v1.21 drafts were removed before
audit; experimental source and measurements remain outside the repo. They were
not accepted/frozen delivery artifacts. The frozen chain remains throughv1.20.

The retained candidate uses four explicit, already-repeated input definitions
(`text`, `texts`, `links`, `merge`) in the existing Research Schema builder.
Each definition reuses its original type builder; input references are declared
at construction, with no runtime tree search/rewriter, cache or new dependency.
Existing evidence definitions, outputs and execution normalization stay intact.

## Size result and limits

| Measured representation | Before | After | Saved |
|---|---:|---:|---:|
| `research_update` input |9711|8690|1021 (10.51%)|
| Complete28-tool input catalog |29276|28255|1021 (3.49%)|
| Same24-tool array from R6-04, substituting new input definition offline |33767|32746|1021 (3.02%)|

Units are UTF-8 bytes of sorted compact JSON, not actual billed model tokens.
The last row is a replayed encoding measurement, **not a new remote request**.
No tool is removed/deferred, and no extra discovery round trip is introduced.
One small optimization does not establish long-task throughput improvement.
True tokenizer/cache/latency/usability effects remain unmeasured pending a new
bounded ordinary-engineering Live; do not infer them from these bytes.

## Local construction-cost comparison

Same external Rust driver and debug build mode, using the saved before and
static-definition candidate binaries. ABBA execution order; each invocation
has10 batches of100 complete catalog constructions; each side has20 batches /
2000 catalog constructions. No compilation was running during these batches.

| Per-catalog time, derived from batch durations | Before | After |
|---|---:|---:|
| Median |8.314810ms|8.323745ms|
| Minimum–maximum |7.960220–9.438440ms|7.812510–9.911180ms|

Source: `catalog.rs`, `catalog-before`, `catalog-static`, and
`bench-paired-{before,after}-{1,2}.txt` in the evidence directory. Descriptive
debug microbenchmark only, not a release benchmark, statistical equivalence
test or request-latency measurement. The medians differ by0.008935ms (~0.11%);
this small sample shows no obvious added construction cost. Do not claim faster
runtime from it. The rejected greedy experiment's preliminary timing is separate.

## Contract and verification

Before editing production, an external program exported all28 inputs from the
current library through `ToolRegistry::catalog`. Permanent fixtures retain
expanded-schema hashes and original byte lengths. A Schema-position-aware test
oracle resolves local definitions; complete expanded contracts must match,
including required fields, constraints, branches and enums. Literal data stays
literal, with an oracle control that detects a changed constraint. Existing
Research tests retain evidence relation/required/execution assertions and real
accepted/rejected updates; only the assertion path follows the new shared type.

A real loopback Responses test compares every sent Tool name/parameter to the
immutable resolved plan. Existing affected CLI/App/RequestAudit/Context suites
exercise execution, history, budget, compaction, Resume and audit reconstruction.
Representation hashes change through existing owners; no version/compatibility
exception is added. Tool execution versions stay unchanged: arguments and
execution behavior are identical. Historical audit bytes are never rewritten.

No public API/Port/config/Event/Prompt/module or algorithm registry is added.
No upstream code is copied. The default Tool exposure and security/runtime
boundaries are unchanged. Detailed Gate, independent review, CPU measurements
and final Lean result follow below.

## Verified completion and Lean Review

Sequence completed: focused11 PASS → affected7 suites207 PASS/2ignored → fresh
Terra xhigh independent read-only APPROVE within ten minutes → no repairs
requested → full Gate PASS → Lean PASS. Evidence: `focused-final.log`,
`affected-final.log`, `audit-result.md`, `gate.log` (exit0). The initial missing
incremental response ID in the new wire fixture and nonexistent affected-target
typo remain in the original logs; only fixture/command were corrected.

Full Gate: fmt, locked all-target/all-feature Clippy with warnings denied,
45 integration suites510 unique PASS/4ignored, locked build, inline-test scan
and whitespace check. Raw511 includes the existing writer-lock subprocess
duplicate. CLI80 PASS/1ignored182.78s; no deadline/window/assertion relaxation.
The complete reviewed source/test hash manifest still matches after Gate.

Lean: one existing production file,59 added/52 removed lines (net7); one private
reference constructor with actual input consumers. Four definitions reuse
original builders; no semantic validation, cache, selector, transport or audit
logic is duplicated. Three permanent tests added (two catalog/oracle, one wire);
one existing schema assertion follows the new reference without losing checks.
No inline tests/public API/Port/dependency/config/Event/Prompt/schema dialect,
new production module, upstream PORT or new ownership boundary. The existing
large Schema file remains one concern; this change does not justify a framework
or mechanical file split. Runtime optimizer/proposed architecture removed before
review; no dead code remains. Incoming R6 work and historical archive preserved.

Outcome: COMPLETE_LOCAL_OFFLINE_VERIFIED. This only closes the bounded Schema
reuse optimization. Live model usability, billed tokens, release performance,
general long-task quality and the prior handoff-format limitation remain open.
Stop at local handoff: no new Live, Git publication, Actions or login-state access.

## Subsequent separately authorized Live

The operator later authorized the bounded [R6-SC1 Live](R6_SC1_LIVE_2026_09_20.md).
It confirms actual32746byte/24-tool delivery and successful Research argument
generation/mutation/readback/Resume in9 HTTP200 requests. `texts` was advertised
but its optional argument omitted; nonempty-reference behavior and comparative
token/latency benefits remain unverified. One initial “no tools” wording error
is retained as a model limitation. Subsequently authorized fresh Terra xhigh
post-run result review returned RESULT APPROVE; this is not pre-run harness
approval. Reviewed code was delivered to private Draft PR #84 (`3ec73ad`),
without merge or Actions; the earlier offline completion record is not rewritten.
