# R7-F1 — Tool and long-task repair

2026-09-23. Initial authorization: local implementation/offline tests and stop before
independent audit. Subsequent audit and full Gate/Lean authorizations/results are
recorded below. No Live, Git publication, Actions or security-task execution.
R7-L1 evidence remains immutable and incomplete. Implementation is not acceptance.

## Findings and ownership

1. Malformed inner function arguments stop before dispatch. Raw bytes were not
   captured in the manual run, so model generation versus upstream conversion
   is unresolved. Responses decoder owns metadata-only classification; CLI
   distinguishes decoding rejection from an inferred network interruption.
   Never print malformed payload fragments: structural redaction is unavailable.
2. File ToolSpecs omitted exact null/hash and patch-format semantics. Existing
   file schemas/guidance own these descriptions; no second parser, shell escape
   workaround, guessed JSON repair, strict-mode assumption or automatic retry.
   Code inspection also found two real patch defects: byte-offset slicing can
   panic on an invalid multibyte hunk prefix, and the new-line hunk offset was
   parsed then ignored. The existing parser/applicator now rejects these safely;
   staged application prevents either invalid patch from changing the file.
3. Historical README constraints disappeared after Compaction. Use the existing
   explicit `prompts.project_file` as durable current input, not an automatic
   extractor or growing copied Summary. Required delivery must refuse overflow
   rather than silently omit an explicitly configured instruction.
4. Original-versus-repaired execution attribution remains a model-content
   limitation. Tighten bounded guidance within existing Summary/Prompt owners;
   do not copy model prose into evidence or claim an offline factuality cure.
5. Output cost: request admission does not bound one response or total billing.
   Operator explicitly included its implementation in this same round. Optional
   Capability Mapping ceiling resolves into audited RequestRequirements and
   Context reserve before encoding. Preserve omitted-field legacy behavior,
   one config path and audit identity. Never silently lower reasoning,
   equate context reserve to a wire cap, or claim backend compliance from a mock.
6. R7 harness false positives (Memory bookkeeping, malformed rejected patches,
   incomplete-run exit status) are already recorded with frozen corrective
   supplements. Do not rewrite historical failures as PASS or add a production
   workflow/policy engine. Future Live requires separately frozen reviewed inputs.

## Verification and stop

Focused ordinary-engineering loopback/regressions, then affected suites including
domain_ports_contract. At the original implementation handoff, independent review
was not yet authorized, so work stopped before audit/full Gate/Lean. Permanent tests stay under
tests; no public API solely for tests, dependency, cache, second runtime or upstream
PORT. Any new configuration/retention contract needs an additive architecture
decision; old frozen files and remote seals remain untouched.

## Implementation boundaries and independent-review handoff

- No new production module, Port method, dependency, Event store, cache or
  Provider construction path. Base generation3 replaces the active resource;
  generation2 bytes move unchanged to the permanent replay fixture.
- New optional public fields in ModelCapabilities/ModelRequestRequirements have
  production consumers in composition, Context planning, request validation,
  wire encoding and existing RequestAudit hashing/reconstruction. Doctor's
  role-scoped fields are operator-facing diagnostics. Limit resolution itself
  remains crate-private; no test-only public entry point.
- Absent output limits retain legacy serialization and default reserves. An
  explicit cap must fit with the default tool reserve and leave input space;
  invalid configuration fails before credentials. A raw request omitting the
  configured cap is rejected, not silently corrected by transport.
- No arbitrary discovered-constraint retention is claimed. Required delivery
  applies to the explicitly configured Project Prompt, not every old README.
  Base generation3 is 2032 bytes versus generation2's 1086; more guidance costs
  Context. Token/latency/factuality benefits need separately authorized Live.
- At the initial handoff, no independent verdict, full Gate or Lean approval
  was claimed. The subsequently authorized fresh reviewer was asked to examine
  atomic malformed/incomplete-sample rejection and redaction; patch preimages,
  Unicode and offsets; Required Project delivery through repeated Compaction/
  Resume; optional-field legacy hashes; cap capture/reserve/encoding agreement
  for both roles; output-contract invalidation and release debug exclusion.
  Review must be bounded and explicit; silence is not approval. The completed
  independent verdict and subsequent Gate/Lean results appear below.

## Preserved verification corrections

Logs are local at `/tmp/cyber-agent-r7-f1.vUBytM/`; no credentials or real model
calls are involved. Earlier failed assertions remain in their original logs.

- The first long-write CLI fixture omitted the explicit experiment phase and
  then used a window too small for the deliberately large follow-up History.
  Corrected fixture prerequisites, not product checks: experiment phase and
  131072 window. This does not establish long-task capacity or model reliability.
- `final-affected-extra.log` initially failed the frozen R6 Schema-sharing
  assertion because file contracts now intentionally add descriptions and a
  hash pattern. The old fixture stays unchanged. A private test projection
  validates only those enumerated R7 additions, removes them for the historical
  comparison, and retains all other expanded-hash/size checks. The historical
  1021-byte research-schema saving is NOT today's net catalog saving.
  `schema-baseline-recheck.log` passes both tests.
- `targeted-clippy.log` caught a nonminimal boolean in limit validation. Replaced
  negated `is_some_and` with equivalent `is_none_or`; no allowance or validation
  branch was removed. Recheck results are recorded separately.

## Offline verification results (2026-09-23)

- `final-focused.log`: seven suites, 102 PASS / 1 ignored (real Provider Smoke).
  Domain model/Context/baseline/composition, file writes, research control and
  Responses HTTP/SSE decoding all pass.
- `final-cli-focused.log`: four targeted PASS (two argument-integrity cases,
  incomplete-output stop/explicit Resume, dual-role cap/audit agreement). These
  are included in the full CLI count below, not counted again.
- `final-affected.log`: twelve suites, 229 PASS / 2 ignored. Includes complete
  CLI (83 PASS), domain_ports_contract, streaming, sampling, world state,
  RequestAudit, Compaction, Session Resume, tool loops, Replay and Vision.
- Additional batch/cache/audited-inspection/accuracy suites: 27 PASS in
  `final-affected-extra.log`; both Schema tests PASS in
  `schema-baseline-recheck.log` after the preserved baseline failure above.
  Total unique debug-profile coverage: 360 PASS / 3 ignored across 24 suites.
- `budget-lint-recheck.log`: four output-contract suites, 45 PASS after the
  equivalent lint cleanup; repeats existing coverage, not extra unique tests.
- `targeted-clippy-recheck.log`: production lib/binary plus nine affected test
  targets pass with warnings denied. Formatting, whitespace and the forbidden
  src-inline-test scan pass. No dependency or upstream source changes.

- `release-focused.log`: optimized binary compiled; nine PASS across six F4
  presentation cases, two argument-integrity cases and one incomplete-output
  budget case. Release rejects `--debug`, retains streaming reasoning separately
  from answer/History, and does not execute rejected/uncommitted ToolCalls.

These were pre-audit focused/affected checks, not the final all-target/all-feature
Gate or independent approval. The handoff state was
IMPLEMENTED_OFFLINE_PENDING_INDEPENDENT_AUDIT; no audit, Live or Git action was
started under that initial authorization. Later authorized closure follows.

## Independent audit and complete Gate

Current state: COMPLETE_LOCAL_OFFLINE. A fresh GPT-5.6 Terra xhigh reviewer gave
APPROVE within the12-minute limit, with no P1/P2 findings and96 independent
offline tests PASS/1 opt-in Live Smoke ignored. Report and exact candidate manifest:
`/tmp/cyber-agent-r7-f1.vUBytM/independent-review-20260923T091244Z.md` and
`independent-review-candidate-20260923T091244Z.txt` in the same directory.
No production/test repair was required after review. At Gate entry, HEAD/status/
all candidate hashes matched the review snapshot exactly. Documentation status
updates are the only subsequent repository edits;47 source/test manifest entries
still match after Gate.

The operator then authorized the full Gate followed by Lean Review. All commands
below completed successfully; logs are `full-gate-{fmt,clippy,tests,build,inline,
whitespace}.log` in the evidence directory.

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
cargo build --locked
```

The governance inline-test scan and `git diff --check` also pass. Tests cover45
integration suites:527 unique PASS/4 default-ignored. Raw result totals show528
passes because the existing writer-lock test launches its own subprocess case;
it is not counted twice. Ignored cases are Primary/Shadow Live Smokes and explicit
delivery/Snapshot offline measurements. This is local verification, not remote CI.

## Final Lean Review — PASS

1. **Semantic owners (checklist1/2/10/11):** limit resolution/validation stays in
   ModelCapabilities; ContextPlanner resolves the cap before fingerprint/audit
   and owns reserves. Existing Composition Root, canonical OutputContract,
   RequestAudit and Responses encoder consume the same captured value. Decoder
   rejection/CLI labels, file patch parser/hash schema and Prompt/Summary changes
   remain in their original owners. No second pairing, truncation, fingerprint,
   error-repair or cancellation implementation.
2. **Public contracts/consumers (3/4/17):** optional capability/request fields are
   consumed by composition, planning, validation, auditing and wire encoding;
   role-scoped doctor fields reach the production CLI JSON report. Typed error
   variants are raised at real validation boundaries. Resolution stays
   crate-private; schema helper stays private. No test-only public entry point,
   future placeholder, registry or unused configuration.
3. **Code budget and boundaries (1/4/8):**19existing Rust files add190/remove27
   lines (net163); no new Rust production module. The active Base resource moves
   to generation3; v2 is removed from the runtime resource path but its exact
   bytes are retained in a permanent fixture. Existing larger modules were
   reviewed by change reason: these additions belong to their current capability,
   planning, transport or Tool responsibilities, so no line-count-driven module
   split is introduced. No experimental production code remains.
4. **Dependencies/upstream (3/6/7):** Cargo.toml/lock/toolchain are unchanged.
   Offline Cargo metadata confirms one package, one binary and no path dependency;
   source resource includes do not require codex/. Forbidden dependency-direction
   tests pass. Existing local owners were extended, not upstream code copied:
   no new PORT, license obligation or port-ledger entry. Frozen architecture
   through v1.20 and remote seal manifest stay unchanged.
5. **Tests/evidence (5/8/10/13):** all permanent regressions remain under tests/;
   src inline scan passes. The old Base v2 SHA256 is preserved, and the frozen
   R6 Schema fixture is unchanged. Only explicitly validated R7 additions are
   projected away for the historical Schema comparison. Failed exploratory
   assertions and original long-task evidence are preserved; Summary/Prompt/
   Vision/Replay stay derived, not a new evidence authority.
6. **Runtime composition (9/11/12/13):** one stable identity, one AgentLoop,
   orthogonal domain/phase/model axes, immutable sampling plans and separate
   Primary/Vision sessions remain. No model-name inference, mutable global cap,
   automatic execution retry, fabricated Receipt or hidden transport injection.
7. **Measurement/deferred scope (8/14):** no retrieval promotion, automatic
   discovered-rule extractor, guessed JSON repair, remote judge, second usage
   ledger or generalized workflow. Base bytes rise1086→2032 and Tool guidance
   also grows; no token/latency saving is claimed without measurement. True
   backend cap enforcement, model factuality/adherence and full-window capacity
   remain unverified. R7-L1 remains incomplete, not retroactively passed.
8. **Sequence and independence (15/16/17):** focused → affected → fresh Terra
   read-only audit → no required repairs → full Gate → this Lean Review. Audit
   finished within its limit with an explicit APPROVE, not inferred silence.
   Main's Lean check is governance closure, not a replacement for independent
   code review. No blocking Lean debt identified for this local Slice.

Stop after this local closure. No Live request, Git commit/push/PR/merge, Actions,
remote architecture seal, actual Provider-config edit or Codex login access
occurred. Further Live/delivery/next-Slice work requires explicit authorization.

## Subsequent delivery authorization and proposed next steps

Operator subsequently authorized remote submission and asked for the next plan.
Deliver R7-L1's stopped-result documentation unchanged alongside the reviewed
R7-F1 repair as a Draft PR; commit subjects use `[skip ci]`. No raw logs, Provider
configuration, credentials or target artifacts enter Git. This authorization
does not include merge, Actions, Live or an architecture hash seal.

1. **Bounded real-Provider verification (pending authorization):** use ordinary
   document/file-editing fixtures, explicit project instructions and separate
   Primary/Vision mappings. Freeze request/time/output bounds before dispatch.
   Check actual wire/audit cap agreement, backend acceptance/reported usage,
   valid escaped content/preimage/patch behavior and explicit recovery if a
   response is incomplete. Do not manufacture an incomplete response or infer
   hard enforcement merely because a normal response was below the cap. Keep
   deterministic forced-incomplete coverage as offline evidence where necessary.
2. **New ordinary-engineering long-task lane:** only after the bounded checks,
   freeze a fresh Project/Session and reviewed harness with all12 meaningful
   phases, two explicit compactions and process Resume. Preserve original and
   revised results independently; distinguish current facts, historical prose,
   repeated reads, completed effects, usage and task outcome. Do not restart
   R7-L1's exhausted allowance or erase its10/12/content-failure result. Security
   tasks remain operator-tested; a quick run is not30–60minute endurance proof.
3. **Evidence-led follow-up:** scope a small R7-F2 only if observed residual
   defects justify it. Reuse the existing owners and focused → affected →
   independent audit → repair → full Gate → Lean sequence. No speculative RAG,
   extra Tool wrappers, automatic JSON repair/retry, new model router or TUI
   expansion while the core long-task acceptance remains open.

These are proposed verification/development stages, not new Live allowances or
instructions to continue automatically after remote handoff.

Remote handoff: Draft PR [#88](https://github.com/F0und-icu/cyber-agent/pull/88)
on `codex/r7-f1-long-task-repair` against `main`. The stopped-result report is
commit `007a3ec`; reviewed repair is `a664c71`. Remote branch identity matched
the local repair before PR creation. Subsequent delivery-record changes are
documentation only. No merge, Actions or Live result is implied by publication.
