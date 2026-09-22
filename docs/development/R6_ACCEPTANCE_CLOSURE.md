# R6 — Bounded engineering reliability closure

Date: 2026-09-22. Execution authority: [STATUS](STATUS.md). Scope is engineering
reliability of the named ordinary-engineering fixtures, not vulnerability success,
arbitrary long sessions, model factuality or a general release qualification.

## Acceptance matrix

| Requirement | Permanent offline coverage | Existing Live / remaining work |
|---|---|---|
| Resume pairing before later user input | History normalization, `ctf_resume_pairing` CLI cases | R1 and later Full-only lanes; retain strict wire checks in final lane |
| Completed reasoning, state diffs, activation and expiry | `tool_activation_continuation`, `m3_world_state_mode`, `request_audit` | R6-01/02; incompatible replacements still stop explicitly |
| Completed raw response equals History and CLI | `responses_cases/text_finalization.rs`, CLI cases | R6-02/04/SC1; compare every completed final-lane response |
| Current values without rewriting historical facts | `current_fact_guidance` | R6-04: USD→EUR; final lane adds EUR→JPY |
| Two explicit compactions and new-process Resume cycles | Existing lifecycle case in `current_fact_guidance` now covers both cycles using the same formal CLI/Writer/Reducer/audit helpers | Final lane:14→3 and22→11 retained History items; new-process Resume after each |
| No repeated completed side effect | Lifecycle regression; `ctf_recovery`; `ctf_child_checkpoint` | One harmless marker append, never rerun; final marker exactly one across both cycles |
| Partial batch / unknown outcome / old process handles | `ctf_child_checkpoint`, process-job/PTY regressions | Offline evidence only; no Live crash injection claim. Unknown never means no side effects |
| Admission/capacity stop and explicit recovery | `ctf_recovery`, Context pressure and compaction-refusal cases | Prior engineering lanes; no automatic retry, resume, compaction or reasoning discard |
| Observed output, Receipts, audit Blobs and references survive | Existing capture/checkpoint, Session/compaction and audit suites | Final lane verifies exact stream prefixes, Blob hashes, source identity and Call/Result/Receipt pairing |
| Model wording accurately describes observed execution/history | No deterministic test can guarantee prose | Separate predeclared content rubric; historical attribution/format failures remain recorded |

Fresh Terra xhigh `r6_acceptance_gap_review` independently found no missing
production feature; the material gap is repeated lifecycle evidence. Its scope
was read-only coverage review, not approval of the final implementation/harness.
No second interruption harness or evidence store is needed.

## Final lane contract (before requests)

- One fresh ordinary reporting Project/Session, formal development CLI and the
  existing Config v1 Composition Root; explicit repo-local Provider configuration.
- At most24 actual upstream attempts,900 seconds total,8 admissions per Turn.
  No retries, redirects, hotpatches, inherited allowance or automatic extension.
- Reuse the prior external transport observer, source-Snapshot setup and fixture
  helpers. Freeze all transitive script inputs, source/tests and binary before
  independent review. Temporary credential config0600, never printed/committed.
- Execute `ledger.py --record-once` exactly once. Subsequent operations read only;
  only the controller changes settings/Core Memory at operator boundaries.
- Observe USD/en-US, update to EUR/fr-FR then JPY/ja-JP; compact and new-process
  Resume for each transition. Both compactions must reduce retained History.
- Raw final SSE text, durable Assistant History and per-phase CLI output agree;
  every actual request has a valid audit, legal pairing and no cross-session ID.
  Old event prefixes/Blobs/Research facts remain unchanged. Marker remains one.
- Stop immediately on transport failure, illegal pairing, audit/integrity loss,
  duplicate effect, scope escape or budget exhaustion. Preserve all failures.

Content rubric is separately human-reviewed: current configured values; past
observations attributed to their original time; no fresh execution claimed from
Memory/file metadata; tool-use claims agree with Receipts; lossy information
marked unavailable; requested length/end markers. No remote judge, answer
rewriter or product prompt expansion. Engineering PASS must never be reported as
content PASS; each rubric has its own result and retained failures.

## Deferred / non-claims

Historical R6-03 paired evaluation found **no measured improvement**. R6-04's
handoff was282 words versus350–450 requested; R6-SC1 falsely said no tools despite
seven real calls. These are not erased by a new sample. The old missing raw body
cannot establish the original truncation cause. Completed protocol output does
not imply a substantively complete or factually correct answer.

Full1M capacity, arbitrary Providers/crashes, complex Vision localization,
nonempty shared-Schema references, general cost/token reduction, model-visible
retrieval/RAG, benchmark/noninteractive execution and security outcomes remain
outside this closure. CTF/security evaluation remains operator-run. Retrieval
stays Shadow; its measured local ranker speedup does not establish Chat benefit.

## Integration and verification

#84/#85/#86 merged as `5e6f0d0`/`227f37a`/`60e83af`, preserving stacked ancestry.
Each remote main tree matched its reviewed head; final tree matched `8b43e43`.
No reported required checks, no bypass; main reported unprotected. Protection
APIs returned plan-related403, not proof of having inspected every rule. No
Actions were started; merge subjects retain `[skip ci]`.

Current work has no production/API/Port/dependency/config/schema/prompt/architecture
change. Existing lifecycle test is table-driven over two cycles, not duplicated.
Initial test compile error (`u64` receipt count versus `usize` cycle) is retained
as a fixture-development error, not a product defect or discarded failed run.

Focused2PASS, affected7suites144PASS/2existing ignored, including dependency
direction. An earlier affected invocation named a nonexistent test target and
was retained then corrected, not counted as a test failure. Fresh independent
Terra xhigh `r6_closure_independent_review` returned TEST APPROVE and HARNESS
APPROVE within ten minutes. Its single harness finding was a missing explicit
per-phase8-request verifier assertion; fixed and re-reviewed before any Live.
All319 source/test hashes and transitive harness/binary freeze matched. A prior
audit handle vanished without a verdict and is not counted as approval.

Focused/affected tests → fresh finite independent review → repairs → complete
Gate → Lean precede Live. A separate independent result review precedes final
R6 closure. Evidence: `/tmp/cyber-agent-r6-closure.UtC0iL/`. Independent result
review and REPORT APPROVE completed; published in [PR #87](https://github.com/F0und-icu/cyber-agent/pull/87).
Reviewed test `de2e723` and separate documentation `3849815` precede this
delivery-reference-only update. Exact merge commit/state is verified from PR/Git,
not inferred from this document or local Gate. No next Slice is selected.

Post-audit complete Gate exit0: fmt, locked all-target/all-feature Clippy with
warnings denied,45 integration suites515 unique PASS/4existing ignored (raw516
includes the writer-lock subprocess duplicate), locked build, inline scan and
whitespace. All319 source/test hashes remained unchanged. Existing process
launch checks were slow but passed without weakening assertions or OS changes.
Lean PASS: zero production/API/dependency growth; existing permanent test net26
lines with a shared two-cycle body; no copied normalizer, budget, audit or
compaction owner, test-only API, new framework, speculative cache or Codex PORT.

The final default build changed executable bytes after the pre-Gate freeze.
Pre-Live checksum verification caught it before dispatch; original manifests
and plan/prepared records are preserved. Only their binary identity is rebound
to the successful final Gate build, followed by independent pre-Live recheck.
This is not a source change, a failed model request or a consumed Live retry.

## Final Live result

One fresh lane completed in46.346seconds with12 actual requests, all HTTP200 and
completed, versus the frozen24/900s ceiling. Per-phase request counts were
4,2,1,1,2,1,1, all within8. Configuration selected `deepseek-v4-flash`, Full-only
Responses, declared1048576 context and Vision disabled. This is not a full-window
capacity test. Final Gate-built binary SHA256:
`c2765c7ee9aebc4b9e2846cabfbcea36126f79550dd8238c8bab0cbbeacbbacf`.

Engineering result: PASS for this fixture. Five actual Calls, five successful
Results and five successful Receipts, exact execution-ID pairing. The one
command was argv `[/opt/anaconda3/bin/python3, ledger.py, --record-once]`, cwd
omitted; subsequent tools only read files. Fresh per-phase results read
EUR/fr-FR and JPY/ja-JP from settings.json and one `executed` line from once.txt.
Both explicit production compactions reduced retained History (14→3,22→11),
each followed by successful new-process Resume; no completed side effect reran.
All12 completed raw final responses equal durable Assistant History and occur
in phase CLI order; no missing response suffix was found.463 audit component
Blob references match hash/size; every stored Blob and old event prefix remains
intact, Research/source unchanged. Temporary credential config removed, source
credential file and final binary unchanged, no credential in retained evidence.

Original frozen `verify.py` failed on request09: its global user-text exclusion
mistook `report.currency=EUR` inside the explicitly derived historical summary
for a stale current Memory fragment. Current direct Memory was exactly JPY/ja-JP.
The original script, freeze and failure log are retained unchanged. Separate
`verify-current-scope.py` checks the exact direct Memory fragment while allowing
historical quotations, additionally binds both changed-fact phases to actual
read_files Calls/Results and checks the full command argv; it passes against the
same evidence. This is a post-run verifier correction requiring independent
review, not a pre-approved all-pass run, altered product or second Live attempt.
An initial supplemental string-escape assertion also failed during local
verifier editing; corrected before its final PASS, without changing run evidence.

### Content rubric (separate from engineering integrity)

| Dimension | Observed result |
|---|---|
| Current configuration | PASS: USD initially, EUR after first update/compaction/Resume, JPY after second; no old currency presented as current |
| Execution versus configuration | PASS for no new ledger execution: later responses do not claim a rerun; actual marker stays one |
| Temporal tool attribution | FAIL: request07 says settings.json read “this turn”; requests08/12 say read_files served “this turn”. These no-tool phases only had earlier-turn reads |
| Lossy-history attribution | FAIL: request07 calls pre-edit totals overwritten/unavailable although total30 remains in its delivered summary; settings changes did not overwrite an observed total |
| Requested handoff length | Initial350words, first current357 PASS; second current317 FAIL against350–450. Whitespace-delimited count of final answer, not all intermediate text |
| Short answers and end markers | PASS: compact/Resume69,71,55,70words, all≤120; required six markers present |

Therefore content is FAIL/LIMITED, not an all-pass model-quality result. Historical
R6-03/04/SC1 limitations still apply. No new runtime-integrity failure was found;
do not add a prompt patch, second judge or response-rewriting owner to hide these
model-generated errors. Future attribution-quality work needs a separately
scoped paired evaluation, not promotion of this single sample into a guarantee.

Provider-reported usage across12 requests:160267input,5329output,165596total;
146048cached input is a subset, not an additional charge/token count. These are
absolute observed totals, not comparative savings or reduced long-task cost.
This46second bounded multi-lifecycle lane is not an hours-long endurance test.

### Independent result and report disposition

Terra xhigh `r6_closure_independent_review` independently verified raw evidence
and the supplemental verifier within the ten-minute result-review bound:
engineering PASS, content FAIL/LIMITED, no new product-integrity defect.
The following five-minute read-only report review returned REPORT APPROVE after
one wording fix: only the ledger/marker ran once, not all tools. No source/test
change followed the previously successful Gate. Approval covers honest report
and evidence delivery, not model-content success. Final source319-entry and
13-entry transitive freeze checks still pass. No extra Live/retry or feature
work is authorized by this result.

## Original issue dispositions (not blanket closure of model behavior)

The historical [CTF issue log](CTF_LONG_TASK_ISSUES.md) keeps its original evidence
and superseded status text. This index distinguishes implemented engineering
repairs from external-environment work and unproven model behavior.

| Original issues | Disposition for this R6 acceptance |
|---|---|
| 01 | R1 unique History normalizer repair; permanent ordering/late-result and formal CLI regressions |
| 02,13,18 | R2a/b typed diagnostics, configured admission and explicit recovery; compatible continuation broadened only by R6-01/02 proofs; incompatible changes remain explicit stops |
| 03,04 | R4 observed environment/error/path guidance and compatibility tested; no guarantee a model always selects the right platform arguments |
| 05,15 | R3 durable child checkpoints and non-blocking progress; unknown parent remains unknown, never fabricated cancelled or automatically rerun |
| 06 | Docker resource collision/shell masking is external harness practice; final ordinary fixture uses no Docker. Actual CTF/container retest remains operator-owned |
| 07,08,10 | Fresh allowlisted Project/Session, complete single-line prompts and external logs in final lane; no claim old deleted CTF evidence was recovered |
| 09 | R5 measurements distinguish actual work from repeated debug history; explicit hash freshness and Blob publication repaired. Unmeasured internal counters stay unavailable |
| 11 | R5a explicit debug omissions/reply boundaries/raw-output hints, prior offline/release/Live verified |
| 12 | R4 bounded source/version guidance; search service/knowledge base and general web-selection quality Deferred |
| 14 | R3a2 retained bounded raw output and readback, R5a honest preview/retention distinction; no unlimited output retention claim |
| 16 | Logical demand, actual delivery estimate and Provider usage remain distinct; delivery budget is enforced, automatic compaction/in-Turn command processing not added |
| 17 | R3a1 runtime-scoped Job/PTY handles reject stale identities; restart is not reattachment to old OS processes |

R6 closes the agreed ordinary-engineering reliability scope only when final
Live, independent result review, complete Gate/Lean and remote delivery pass.
It does not convert these deferred model/research/environment dimensions into
success, nor introduce a CTF-specific runtime workflow.
