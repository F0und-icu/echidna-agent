# R6-02 reliability repairs — 2026-09-19

Verification state is maintained in STATUS. Historical reports are not rewritten.
Evidence root: `/tmp/cyber-agent-r6-repairs/`. No Git/Actions in this repair.

Current outcome: all confirmed in-scope code defects are repaired, independent
review and final Gate/Lean PASS. A subsequent explicitly authorized post-fix
Live completed6 HTTP200 requests in60.07seconds; engineering checks PASS with
prose-quality limits and independent result-report APPROVE. See
[R6_02_LIVE_REVALIDATION_2026_09_19.md](R6_02_LIVE_REVALIDATION_2026_09_19.md).
The earlier failed lane below remains unchanged historical evidence.

## Issue disposition

| Observed problem | Change and owner | Acceptance boundary |
|---|---|---|
| Answer ends at `## Prev` | Responses response-local final-text reconciliation and terminal-status validation | Offline missing-final-text/status defects reproduced. Historical cause remains unknown without its raw body; never auto-continue or invent prose. |
| `read_output durable blob stdout` misses known Tool | Existing tool_search: exact eligible name tokens precede unchanged all-term fallback; stable names/limits/Hidden preserved | Protocol-free permanent Tool test, then bounded formal CLI Live. |
| Current advertised list mistaken for full catalog | Canonical ToolSpec plus precise Snapshot output-schema description | Existing immutable plan remains the only visibility/dispatch authority. |
| Unnecessary Memory/Research reads, invalid Artifacts | Existing ToolSpec guidance: reuse supplied current facts; real TargetSnapshot/root provenance; raw Blob readback first | No fabricated evidence or forced research workflow; model compliance is not guaranteed. |
| Invented dates | ToolSpec instructs observed time through existing POSIX date argv; preserve historical source times | No implicit timestamp rewrite, clock service or new schema field. |
| First calls unnecessarily carry rerun_reason | Existing optional execution object explains deliberate reruns only | Dedupe/retry/effect semantics unchanged. |
| Activation expiry stops reasoning + state diff | Store immutable committed Turn pre-activation plan; exact expiry proof and original Baseline comparison | Expired Tools remain unadvertised; redefinition/removal/config/budget failures remain explicit. |
| Compaction fixture lacks target / conflicting currency | Back up the established canonical-root fixture, verify real target/source identity, correct stale Memory to actual EUR | Full pre-run backup and original external logs retained; live Session is append-only, Research/source/old Blobs unchanged. No relaxed product prerequisite. |

## Reproduction and corrections

- Initial four response-finalization regressions all failed against the original
  decoder (`response-red.log`); final suite adds multiple parts, Tool ordering,
  missing/contradictory snapshots, identity and terminal-status cases. Real CLI
  verifies output reaches History and a conflicting response never executes its
  streamed ToolCall or commits Assistant output.
- Next-Turn formal CLI regression reproduced `BASELINE_REJECTED: ToolsChanged`
  (`expiry-red.log`). Positive case now checks seamless next Turn and explicit
  process Resume; App cases retain prior definitions and reject changed catalog.
- First search test accidentally used an unbound operator plan (empty discovery
  candidates), not the model-bound path. Corrected to immutable model-call
  planning, then disabled only the new exact-name branch to rerun the old AND
  matcher: failure reproduced in `search-valid-old-matcher.log`; restored matcher
  passes. Earlier invalid reproduction/compile attempts remain in the logs.
- Added execution-schema descriptions required updating the strict request
  golden fixture. No equality assertion was removed. Verbose repeated guidance
  was shortened after the small-window fixture stopped at 15048/15000 tokens.
  The unchanged 20000-window pressure test now passes.
- Long-flow fixture's final pre-Compaction request changed to
  `InputNotAppendOnly` under its original 32768 window. This evidence/continuation
  fixture now uses65536, with exact two Full/five incremental assertions retained;
  dedicated pressure tests still cover truncation/no automatic Compaction.

## Scope and pending verification

ADR-0029/additive v1.19 covers the narrow protocol/expiry semantics. One private
text module; no public API, Port, dependency, persisted schema, model/vendor
branch, new service, cache or automatic action. Tool schema shapes are unchanged;
descriptions still flow through canonical fingerprints/audit. Old architecture
and remote seals remain unchanged.

Focused/affected tests, independent review, post-review full Gate, Lean and the
new six-request/600-second ordinary-engineering raw-SSE lane must finish before
closure. No universal model compliance, full-1M, arbitrary interruption,
multimodal or security research-quality acceptance is claimed.

## Independent audit repair

Fresh Terra xhigh read-only audit verified all20 frozen review inputs and returned
REQUEST_CHANGES for product / APPROVE for the external Live harness. Final
snapshot validation checked the presence of a content slot but not its message
type, known item identity and exact output_text value. A streamed message could
therefore be matched to a same-position reasoning item. The added regression
reproduced failure before the patch (`audit-red.log`). The same private owner now
requires those properties; no new API or alternate decoder was added.

After repair: Responses27 PASS/1ignored, dependency-direction2 PASS and CLI
integrity1 PASS (`audit-fix-affected.log`, `audit-fix-cli.log`). The test also
covers missing/changed known identity, changed text and wrong content type.
Two changed files are frozen in `review-fix.sha256`. Five-minute bounded
independent re-review returned PRODUCT APPROVE and retained harness APPROVE;
the reviewer verified both hashes. The original20-input check differs only at
these two expected repair files (`post-review-original-check.log`). Full Gate
now runs after approval; Lean/Live remain pending.

## Complete Gate and Lean Review

Post-review sequential Gate PASS, exit0: fmt, locked all-target/all-feature
Clippy with warnings denied,44 integration suites501 unique tests passed /
4 opt-in ignored, locked development build, source inline-test scan and
whitespace check. Raw502 includes the existing writer-lock subprocess duplicate.
CLI78 PASS/1ignored179.60s. Evidence: `gate.log`. Production/tests stayed frozen
after independent re-review.

Lean Review PASS: one private response-text module (205 lines), called by the
existing protocol decoder. Existing Baseline comparator and immutable Tool plan
own continuation proof; existing search and ToolSpecs own discovery/guidance.
No public API, new dependency, configuration, Port or persisted-schema change;
no vendor logic, retry, registry, cache or second History/dispatch authority.
Removed duplicated search matching/counting and replaced ignored final-text
events with one reconciliation path. Permanent regressions remain under tests/;
no inline production tests or test-only public entrypoints. Current combined
R6-01/R6-02 production diff:542 additions/118 removals, net424 across10 files
(including the new private module); R6-01 previously accounted for net57.
External fixture/controller and raw captures stay outside Git, not in the
runtime. Old unresolved truncation attribution and unmeasured capacity/domain
quality remain explicit limits, not placeholder code. Bounded Live is next.

## Zero-request preflight correction

The first external harness stopped before any outbound or received request:
copying a Project to a new root preserves project.toml's canonical-root identity,
so formal CLI Resume correctly rejected it. `preflight.stderr` and `summary.json`
retain this setup failure; doctor succeeded, original project/source and binary
were unchanged, temporary credential config was removed with no credential leak.
This is not a product defect and consumes no request allowance.

Revised harness under `live.HHjW57/` continues the original synthetic Session at
its established canonical root, after a complete independent backup to
`live.HHjW57/before/project`. Old event/Research prefixes and Blobs must remain;
only the backed-up stale Core Memory currency is corrected to actual EUR and
the explicitly allowed fixture note is written. This replaces the invalid copy-
at-new-root assumption; no project identity rewrite or product guard weakening.
Historical log directories remain unchanged. Verification compares the backup,
not a mutable live input; credentials are scanned across both evidence and
working Project. Same total6-request/600-second lane (zero spent), not a retry
or expanded allowance. Revised harness receives a separate finite read-only
review before execution. Product/tests/binary remain frozen and Gate-passed.

Harness re-review requested exact working-tree comparison instead of checking
only the backup plus selected paths. Repaired externally: all old entries must
remain; no new directories/symlinks/nonregular entries; every old file is exact
except the expected Core USD→EUR replacement, designated append-only Session,
and fixed runtime-derived JSON caches. Research stays byte-identical. Only the
exact note and new content-verified SHA256 Blobs/hex-named fragment caches may
be added. Unexpected source/Memory/other-session changes fail the lane, including
on error cleanup. Five offline scope-check cases pass without importing the
credential-loading controller. Repaired harness frozen in
`live.HHjW57/review-harness-fixed.sha256`; finite re-review pending, still zero
real requests.

## First raw-SSE Live result and repair

Revised harness received independent APPROVE and passed canonical-root preflight
with a complete backup. One actual HTTP200/completed upstream request then
exposed a regression introduced by this Slice's new final-text handler:
`response.content_part.done` can contain `reasoning_text`, not only output_text.
The Adapter wrongly routed that known lifecycle marker to Assistant text and
stopped with `unsupported completed message content`. No Tool ran, no Assistant
answer was committed and no retry occurred. Subsequent queued task lines were
rejected by the explicit stopped-Turn guard. Raw SSE is retained in
`live.HHjW57/response-01.sse`;4.83s, one request. Entire Project scope check PASS,
old Research unchanged, Session append-only, counter1, backup/binary/config
source unchanged; temporary config removed and no credential found.

Permanent minimal regression reproduces the exact failure
(`reasoning-part-red.log`). The existing protocol dispatcher now recognizes the
known reasoning marker like reasoning_text.done; completed reasoning validation
and once-only emission remain owned by decode_reasoning_item. Unknown message
content still fails. CLI continuation fixtures now include the same marker.
No vendor branch, new module/API/dependency or duplicate reasoning authority.
Focused/affected tests, finite independent re-review and a fresh full Gate are
required for this repair. Previous Gate is not claimed for the new candidate.
The stopped Live lane is not silently retried; a new six-request/600-second
lane has been requested explicitly, not yet assumed authorized.

Post-Live repair re-review returned PRODUCT APPROVE after verifying the three
hashes in `reasoning-part-review.sha256`. Responses28 PASS/1ignored, dependency
contract2 PASS and formal CLI activation2/integrity1 PASS. Fresh full Gate is
recorded separately as `gate-after-live-repair.log`, not overwriting the earlier
candidate's Gate. The successful offline preflight had already persisted
Compaction revision2 (84 source History items →23 retained), with a real target;
the failed request does not establish remote post-Compaction success.

## Final post-Live-repair Gate / Lean / handoff

`gate-after-live-repair.log`: exit0, fmt, locked all-target/all-feature Clippy
with denied warnings,44 suites502 unique PASS/4 opt-in ignored, locked build,
production inline-test scan and whitespace check. Raw503 includes one existing
writer-lock subprocess duplicate. CLI78 PASS/1ignored178.93s. All three final
review hashes remain unchanged. This replaces the earlier501-test candidate
as the final offline verification, without deleting its evidence.

Final Lean Review PASS. The compatibility repair adds only6 lines to the existing
dispatcher; no new owner, API, crate, configuration or schema. Reasoning lifecycle
markers emit nothing, completed reasoning still validates/emits once, and
unknown message content still fails. Combined R6 production548 additions/
118 removals, net430 across10 files (one205-line private text module). Permanent
regressions remain in top-level tests; no unused public surface or inline tests.
The OpenAI Docs/Responses event-contract reference informed text finalization;
the actual observed reasoning marker is covered by a protocol-generic synthetic
regression, not a provider-name exception or raw private-history fixture.

No additional real request, Git mutation/publication, Actions or Codex login
access. Current Live total1 actual HTTP200, zero executed Tools; the failed
lane is closed, not silently retried. A fresh bounded six-request/600-second
ordinary-engineering lane was explicitly requested and is pending approval.
It must verify raw final text versus History/CLI, activation expiry with retained
reasoning/state diff, manual Compaction and explicit new-process Resume on the
new frozen binary; correct the harness's ambiguous "same response" phrasing to
search first, then use the returned schema in a later model Step. Preserve all
old logs and use a new complete pre-run backup at the canonical project root.
Historical truncation attribution, general prompt compliance, full-1M capacity,
arbitrary interruption, multimodal and security quality remain unclaimed.
