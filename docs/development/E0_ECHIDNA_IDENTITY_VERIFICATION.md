# E0 — Echidna identity verification

Date: 2026-09-27. Outcome: **OFFLINE_GATE_PASS / LEAN_PASS**.
Implementation owner: main agent. Independent reviewer: fresh GPT-5.6 Terra
xhigh, `/root/echidna_e0_audit`, **APPROVE at 10:25 UTC**, before 10:29 deadline.
No repair requested. This is a presentation/repository transition, not a new
whole-core acceptance or real-model compliance claim.

## Candidate and ordering

Base/core checkpoint: `eee836d056bf71a008c5d29d0bf94c158809e687`.
Verified E0 implementation commit: `4f7ecb7` (code, tests, build and packaging).
Evidence: `/tmp/echidna-migration-7yvma8u2/`; `e0-candidate.json` freezes455 files.
The parent rechecked the complete candidate before independent review; source
and tests remain unchanged through final Gate. Later edits only record results.

1. Focused:2 tests PASS (version and real-PTY theme matrix). The matrix covers
   normal color, empty/nonempty NO_COLOR, TERM=dumb, and brand text exclusion from
   actual request JSON and durable History. Existing piped reply assertions also
   verify no added ANSI/banner and preserved streaming reply boundaries.
2. Build identity:3 external checks PASS — actual Git HEAD, no-Git source archive,
   archive nested under an unrelated Git root. Initial standalone rustc probe
   omitted `--edition 2024`; corrected command passed without source changes.
3. Affected final:8 suites,141 PASS/1 ignored, including domain_ports_contract,
   CLI, Session Resume, durable checkpoints, RequestAudit, Provider composition
   and event upcasting. Initial CLI run was93 PASS/1 FAIL/1 ignored: an existing
   activation-continuation child exceeded its unchanged15-second deadline while
   release compilation also ran. Exact isolated rerun passed in8.37seconds and
   complete affected rerun passed. No timeout/assertion/code was changed; resource
   contention is a possible contributor, not a proven causal diagnosis.
4. Local macOS arm64 release package and negative archive checks PASS: correct
   Mach-O architecture, binary-only allowlist and SHA256; corrupted checksum,
   extra log and symlink archives rejected. Host is x86_64, so arm64 executable
   execution/version checks were skipped by the existing packaging contract.
   Host CLI short/long version checks passed separately. No release was published.
5. Fresh independent read-only review APPROVE after affected verification;
   reviewer read the authority chain and candidate/evidence, ran no extra tests.
   Review covered compatibility, presentation/fact separation, source-base
   identity, packaging, test assertions and design-only scope. No repairs.
6. Full Gate PASS: fmt, Clippy all-targets/all-features with warnings denied,
   all-target/all-feature tests, and locked ordinary build.45 nonempty suites:
   **544 unique passed,4 ignored**. Count uses each suite's final result, excluding
   the existing child-process duplicate summary. Full CLI again passed the
   previously timed-out activation case. No unrelated release build ran alongside
   this full Gate. Inline-test, generic Provider and single Responses-owner scans
   and `git diff --check` also passed.
7. Final Lean Review PASS after complete Gate, below.

## Lean Review

- New owner: private `cli_chat/theme.rs` (36 lines) only chooses terminal welcome
  and answer label styling. Existing sampling callbacks still own reply boundaries.
  `build.rs` (35 lines, std only) owns compile-time source-base reporting; Clap and
  package verification consume it. No second Context, Session or execution owner.
- Existing entrypoints reused: one package/default binary, Provider composition,
  CLI run, terminal text/fact separation, packaging and permanent CLI fixtures.
- No new dependency, public API, Port, configuration, schema, Prompt generation,
  cache or fact store. Core runtime logic, frozen architecture/ADR and108 protected
  historical/prompt/fixture files stay byte-identical to the core checkpoint.
- Tests remain under `tests/`; no test-only production API or inline test module.
  Crate/bin import changes are mechanical; new coverage stays in existing suites.
- No Codex implementation was copied or linked. No upstream PORT ledger entry
  required. The build/test checkout has no `codex/` directory.
- No speculative runtime types, registries, unused abstractions or dependencies.
  TUI, separate core SDK, headless/worker protocol, external queue and capability
  loader remain Deferred in the design documents. No experimental runtime code
  required removal.
- Existing timeout-harness diagnostics and NET-01 remain open/deferred; E0 does
  not resolve model quality limitations or infer zero unknown execution effects.

## Delivery limits

No Live Provider request, Actions execution, tag/release, old PR merge or frozen
manual-test binary replacement. New remote is private and Actions disabled.
The validation archive was built from the frozen, then-uncommitted E0 candidate;
its source-base field is the core checkpoint, not a claim of clean release source.
`source base` denotes build-time HEAD only. This archive is local verification,
not a published release; rebuild committed source before producing a release.
