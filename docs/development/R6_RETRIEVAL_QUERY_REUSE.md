# R6 follow-up — Query-local field term reuse

2026-09-21. COMPLETE_LOCAL_OFFLINE_VERIFIED; independent audit, subsequent full
Gate and Lean PASS. Not a whole-R6 acceptance claim. Main implements after
operator-authorized R5c2 delivery.
Baseline commit3e024b8, dependent Draft PR #85; new implementation remains
separate on codex/r6-retrieval-query-reuse.

## Scope and original owner

The preceding release probe identified repeated normalized_terms calls inside
BM25's candidate length, document frequency, term frequency and field weights.
The existing private RankedCandidate now retains each field's normalized terms
once, only after candidate eligibility succeeds. Those same terms also supply
matched_fields. Identifier-only queries have no ordinary lexical terms, so
neither BM25 nor matched_fields consumes field terms; keep aligned empty vectors
in that case and leave identifier/n-gram matching unchanged. Terms are dropped
with the query; nothing is persisted or reused across source revisions,
Projects or queries.

The original normalizer is unchanged. Per-field deduplication and limit behavior
stay independent: concatenating fields or using raw word occurrence counts would
change BM25. Candidate field order and weights, integer frequency counts,
floating-point expression/order, exact identifiers, n-grams, RRF, tiering,
stable ties, report/query identity and the Shadow boundary remain unchanged.
Only field normalization is reused, not an alternate scoring implementation.

Space is bounded by the existing admitted candidate/field/byte limits. Retained
terms trade some per-query memory for fewer transient allocations; no RSS or
full-capacity memory improvement is claimed. The original normalizer's limit
is applied at raw-token boundaries; this patch does not redefine it as a hard
256-result guarantee for compound tokens.

No production module/public API/Port/dependency/config/schema/algorithm version/
persistent cache or Codex PORT is added. Existing private functions retain their
roles. Scope excludes model-visible retrieval, RAG promotion, automatic actions,
Live, Actions and credentials.

## Verification record

Evidence: `/tmp/cyber-agent-r6-query-reuse.344tMr/`.

- Baseline release binary rebuilt before production changes, SHA256
  8d6f0be57c41ebd8c0090ffcadc167f66aae34f89064ebde67f086a682b01fe0.
- Two new permanent CLI tests passed on the original implementation, before
  optimization (baseline-regression.log). These characterize existing semantics,
  not a red bug reproduction: field-level deduplication/weight/stable ties,
  independent normalization limits, final admitted term, current-file refresh,
  repeated deterministic reports and unchanged Project facts.
- First candidate compile failed at a borrowed iterator string comparison
  (focused.log); corrected locally to as_str without changing behavior. The
  failure log remains; it was not a passing test or an inherited product defect.
- Corrected focused retrieval suite11 PASS (focused-fixed.log). Affected suites:
  domain_ports_contract2, guidance_shadow4, inspection_cli4, memory_snapshot14
  PASS/1existing ignored (affected.log). fmt and whitespace checks PASS.
- First paired release comparison passed96 observations and complete CLI JSON
  equality across8 cases, six alternating-order baseline/candidate repetitions.
  Large64×4096/+16term query median581.645→90.756ms; identifier-only medians
  ranged from effectively unchanged to5.49% slower. Retain paired.log/result
  and both binaries rather than declaring the short-query criterion met.
- A second candidate avoids unused ordinary-term normalization/storage when
  the query has no ordinary terms. Corrected focused11 PASS and affected24
  PASS/1ignored repeated in focused-final.log and affected-final.log. Second
  release build/comparison PASS in separate evidence directory
  `/tmp/cyber-agent-r6-query-reuse-final.ELFFQ8/`; same harness and original
  baseline, same fixture/query identities and96-observation alternating design.
  Both runs are retained; the later result is not a selected repeat of identical
  code. Bounds30seconds/process and180seconds/lane, no Provider requests.

## Final paired release result

Candidate SHA256:
1e1a68b7dc832b8e9312ec463ab62f3b9f6b7568401b50eac081bb6989b9a711.
Both locked offline release builds completed successfully. Final lane has96
observations, complete CLI JSON equality (including fingerprints, ranks,
matches, exclusions, costs and zero effects), unchanged fixture/source/binary
checks and matching harness/source freeze manifests. No normalization of report
differences. Existing ordinary-engineering fixtures are read-only; no new
Writer/Reducer helper or stale helper binary was invoked.

| Sections | Body bytes | Additional query terms | Baseline median ms | Candidate median ms |
|---:|---:|---:|---:|---:|
|8|512|0|14.548|14.481|
|8|512|16|24.018|15.137|
|8|4096|0|20.979|20.149|
|8|4096|16|84.429|20.807|
|64|512|0|38.962|39.031|
|64|512|16|106.758|39.952|
|64|4096|0|92.297|79.454|
|64|4096|16|577.310|92.985|

The largest long-query case is about84% lower wall time (6.2x ratio).
Identifier-only queries range from13.9% lower to0.18% higher; no material
short-query regression observed in this fixture, not a statistical guarantee.
Host load/filesystem caches are uncontrolled. These are whole CLI timings,
not ranker-only profiling, live Chat latency, billed tokens, real-corpus
relevance or vulnerability-research success. The first run and its limitations
remain retained separately. Production diff is46 additions/29 removals in one
existing private module; tests add79 lines in the existing integration suite.

Final implementation/tests are frozen in the final directory's
preaudit-source.sha256 (319 entries); audit must use this final manifest, not
the first candidate's manifest. Baseline/failure/focused/affected/build logs
remain in the original directory. Subsequent Gate/Lean are recorded below.

## Independent audit

Operator explicitly authorized fresh GPT-5.6 Terra xhigh review, then full Gate
and delivery. Independent r6_query_reuse_audit returned APPROVE within ten
minutes on2026-09-21, with no blocking findings or requested changes. Reviewer
verified the final319 source/test entries and independently compared all48
baseline/candidate JSON pairs. The finite review checked per-field semantics,
identifier-only behavior, tests, ownership and measurement limits. Verdict is
recorded in final evidence audit-result.md; no edits or full Gate by reviewer.

No source/test changes followed approval. No subsequent Slice starts here,
and no Live/Actions/merge is inferred.

## Complete Gate and Lean Review

Post-review gate.sh/gate.log exited0: cargo fmt --all -- --check; locked
all-target/all-feature Clippy with warnings denied; locked all-target/all-feature
tests; locked build; source inline-test scan and whitespace checks.45 integration
suites,515 unique PASS/4existing ignored. Raw516 includes the existing writer-lock
subprocess test reported twice. All319 reviewed source/test entries match both
before and after Gate. No review fixes or production edits occurred after
approval; completion documentation only. These are local results, not Actions.

Lean Review PASS:

- One original ranker and normalizer retain semantic ownership. Repeated calls
  to normalized_terms inside length/frequency/weight/matched-field paths are
  replaced by query-local field vectors; no copied normalizer or scorer.
- Production+46/-29 in one existing private file. The file exceeds the300-line
  review signal but retains one fixed ranking responsibility; splitting a second
  preparation module would not add a separate semantic owner. No new production
  module/type/public API/Port/dependency/config/Event/schema/algorithm version.
- The new private field is consumed by existing production scoring/matching;
  there is no test-only public entrypoint, unused abstract type, registry or
  placeholder. No persisted cache or invalidation layer; derived data is bounded
  by current input bounds and dropped with the query. No RSS benefit claimed.
- Two permanent tests (+79 lines) reuse existing CLI/Target/tree helpers in the
  top-level retrieval suite; original tests retained, no src inline tests.
- No Codex PORT, upstream dependency or license change; existing independent
  dependency-direction contract and full Gate pass. Provider, Tool, Context,
  History, evidence and one-AgentLoop contracts remain unchanged.
- All final paired outputs match, synthetic cost benefit supports retention;
  first candidate's regressions/failure evidence remain outside Git. Retrieval
  promotion, general model quality and broader capacity remain Deferred.
- Required focused → affected → distinct authorized finite audit → no requested
  fixes → full Gate → Lean sequence complete. No unresolved Slice governance
  debt. Remote delivery is separate and must preserve these exact source/tests.
