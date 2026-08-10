# Cyber Agent Development Status

## Current State

- Architecture version: `1.1.0`
- Phase: `IMPLEMENTATION`
- Implementation: `STARTED`
- Current milestone: `M1b`
- Current slice: `M1b-S1`
- Slice state: `READY_NOT_AUTHORIZED`
- Lean governance: `ACTIVE`; governance and completed M1a-S2/M1a-S3 changes are merged on `main`
- Local Git initialized: `yes`; merged M1a code baseline is `main` commit `9988b98`
- Remote repository: `F0und-icu/cyber-agent` private; PR `#4` was Squash Merged
- Documentation baseline commit: `55ff3be`
- Current code base: `main`/`origin/main` merge commit `9988b98`
- Upstream Codex baseline: `3aae5d885bac39c1262491aa3fd100dfd8b3919f`

## Last Completed Slice

- Slice: `M1a-S3`
- Delivery branch: `agent/m1a-model-tool-loop`
- Base commit: `0c413c9` plus the preceding uncommitted M1a-S2 worktree
- Scope: provider-neutral Tool contracts and stable BLAKE3 fingerprints; strict in-memory Tool Call/Result pairing; ToolExecutor Port; one bounded AgentLoop; fixed `read_file`/`list_files`/`search_files`/`run_command` Registry; argv-only process execution; bounded output, timeout and shared cancellation; OpenAI Tool Schema, Function Call and Function Call Output full-request encoding.
- Agent behavior: one primary ModelProvider and one AgentLoop perform Tool Call -> Tool Result -> full resample; a Turn is limited to 32 model requests and multiple Tool Calls in one response fail atomically rather than committing partial History.
- Test path: a Scripted Mock Provider requests `search_files` against a real temporary vulnerability fixture, receives the local Tool Result in the second complete ModelInputIR and returns a final candidate analysis. File/list/search/argv/nonzero/truncation, plan/status, pairing, cancellation and OpenAI wire paths remain permanent top-level regressions.
- Forbidden work preserved: no ProviderSession, `previous_response_id`, batching, scheduler, PTY/Job, Vision route/service, DeepSeek Adapter, Sandbox/approval/policy, Hook/Plugin Registry, real Provider request, Codex credential/login access, Codex source modification or Codex code port.
- Result: `COMPLETE`; all local Cargo and Lean Gates pass.
- Dependencies: added direct `blake3 1.8.2`; promoted the existing Tokio stack to production with only `fs`, `process`, `rt` and `time` features needed by local Tool adapters. No Tool SDK or Codex crate was added.
- Git: PR `#4` was Squash Merged as `9988b98` (`M1a: complete OpenAI streaming and single-tool agent loop`).
- GitHub Actions: final PR-head run `31388093658` and post-merge `main` run `31388314842` passed on `macos-15-arm64` with Rust `1.93.1`; formatting, production test-layout scan, Clippy, 44 tests plus one ignored Live Smoke, and locked build all passed.
- Live Test: `OFF`; the existing synthetic smoke remains ignored and no real credential or public Provider request was used.

## Current Authority

1. `AGENTS.md`
2. This file
3. `task.md`
4. `docs/development/IMPLEMENTATION_PLAN.md`
5. `docs/development/LEAN_CODE_GOVERNANCE.md`
6. `docs/architecture/CYBER_AGENT_ARCHITECTURE_V1_1.md`
7. `docs/architecture/CYBER_AGENT_ARCHITECTURE_V1.md`
8. `docs/adr/ADR-0002-capability-driven-primary-vision-models.md`
9. `docs/adr/ADR-0003-minimal-openai-http-sse-stack.md`

## Completed

- Codex local architecture and prompt-flow assessment.
- Dedicated single-user vulnerability research Agent architecture and P0/P1/P2 classification.
- Architecture v1 frozen baseline and v1.1 capability-driven Primary plus optional Vision composition.
- Development, Git/version/release, environment, credential isolation and Mock/Live/CI plans.
- Parameterized Luna single-Slice implementation prompt and private documentation baseline.
- M0-S1 single-package CLI skeleton and pinned-toolchain CI Gate.
- M0-S2 provider-neutral Event/Upcaster/Reducer/Port contracts.
- M1a-S1 provider-neutral ModelInputIR/ModelEvent/Capabilities, ModelProvider Port, Mock Provider and cancellable Streaming App Service.
- M1a-S2 minimum OpenAI Responses full-request Adapter, injected dedicated credential source, local-loopback contract tests, external permanent test layout and Lean Gate.
- M1a-S3 Tool contracts, strict in-memory History, fixed FullAccess Tool Registry, bounded AgentLoop, OpenAI Tool history encoding and real temporary-project full-resample Gate.

## Not Started

- M1b-S1 and later implementation; no ProviderSession or incremental baseline code has started.

## Current Authorization

No implementation authorization exists for `M1b-S1`. The governance/M1a-S2/M1a-S3 delivery authorization has been consumed by merged PR `#4`.

- Run mode: `REVIEW_ONLY` for `M1b-S1`; no new implementation Slice is active
- Slice ID: `M1b-S1`
- Git mode: `NO_COMMIT` after the authorized merge-status handoff; no further branch, commit, push, PR, tag or release is authorized
- Live test mode: `OFF`; no real Provider credential or public model request is authorized
- Completed delivery: governance plus M1a-S2/M1a-S3 through PR `#4` and merge commit `9988b98`
- Stop condition: do not implement ProviderSession, Canonical Baseline or any M1b behavior; do not run a Live Test or perform further Git writes without new authorization

## Slice Lean Review

### Semantic Owners

- `src/domain/tools.rs`: Tool identity, Spec, Effect/Retry/Determinism/Cache metadata, plan/input fingerprints, ExecutionReceipt and ToolResult status invariants.
- `src/domain/history.rs`: the only Tool Call/Result pairing owner; missing, duplicate and mismatched pairs are rejected without Adapter repair.
- `src/ports/tool.rs`: provider-neutral planning/execution boundary and Tool error vocabulary.
- `src/app/cancellation.rs`: one cancellation authority shared by Model Streaming and Tool execution.
- `src/app/agent_loop.rs`: the bounded single-Agent Tool Call -> Result -> full-resample state machine and atomic model-sample commit.
- `src/adapters/tools/mod.rs`: fixed Tool Registry, canonical argument decoding, plan construction, timeout and invalid-plan mapping.
- `src/adapters/tools/filesystem.rs`, `process.rs` and `output.rs`: file/list behavior, argv-only rg/process execution and the sole output truncation implementation.
- `src/adapters/openai/request.rs` and `response.rs`: private OpenAI full-request encoding and stream-event decoding, split because they have independent change reasons.

### Reuse and Duplication

- Reused the existing `ModelInputIR`, explicit `ModelCapabilities`, `ModelEventStream`, `ModelProvider` and `ModelStreamingService`; Tool history extends the same provider-neutral IR rather than adding a Provider-specific loop.
- All built-ins call one argument decoder, one invalid-plan mapper and one bounded-output function. AgentLoop calls the Domain pairing owner and the shared cancellation owner instead of copying either rule.
- Fixed Tool specs are generated from one Registry; no Hook Registry, plugin ABI, dynamic Tool loader or second Agent runtime was introduced.
- Request, Tool request and SSE Fixtures each have one stable source under `tests/fixtures/openai/`.

### Public API and Dependencies

- New cross-boundary contracts are limited to Tool Domain types, `ToolExecutor`, `AgentLoop` outcomes/errors, shared `AgentCancellation` and the fixed `ToolRegistry`; private helpers were not widened for tests.
- `ModelInputIR` schema advanced from 1 to 2 before persisted runtime data exists, adding message roles, Tool Specs and Tool Call/Result items. No Event, Project or Research schema changed.
- `blake3` has one current responsibility: stable Tool input/plan fingerprints. Tokio production features have one current responsibility: local file/process Tool execution and timeout.
- OpenAI request/response DTOs remain private; no Provider wire type, HTTP type or Codex dependency entered Domain/App/Ports.

### Test Layout and Codex Classification

- Production `src/` contains no inline `#[cfg(test)]` module; 44 offline/local-loopback tests pass and one synthetic Live Smoke is ignored.
- Permanent Domain, pairing, Tool/AgentLoop and OpenAI wire regressions live under top-level `tests/`; the temporary vulnerability project is created under ignored `target/` and removed by its test guard.
- Credential tests use an injected fake source and never mutate the real process environment.
- Codex History normalization and Tool execution were used as `REFERENCE`; the local pairing, argv execution, cancellation and truncation code is `REIMPLEMENT`. No source was copied, so no ACTIVE Port Ledger entry exists.

### Removed and Deferred

- Removed a duplicate AgentLoop Tool Call append found by the end-to-end Gate, so one model sample now commits atomically exactly once.
- Consolidated duplicated filesystem/process argument decoding and invalid-plan mapping; split the oversized OpenAI wire file by request/response ownership without adding an abstraction layer.
- Kept `domain/tools.rs` as one approximately 300-line Tool-contract owner after review; mechanical splitting would scatter one invariant family. No production module now mixes two independent change reasons above the review threshold.
- Deferred exactly as scoped: ProviderSession, `previous_response_id`, batching, Resource scheduler, retries, Tool cache, PTY/Job, structured output, Artifact-to-image resolution, Vision routing and CLI composition.
- No unused feature flag, generic Hook/Router, Provider SDK, test-only production API, copied Codex code or unconsumed persistent Schema was retained.
- Remaining governance debt: none inside M1a-S3; remote CI evidence is pending the currently authorized PR delivery.

## Blockers

- None for the completed M1a-S3 local implementation and Gate.
- M1b-S1 is intentionally stopped at `READY_NOT_AUTHORIZED`.
- Live Provider compatibility remains unverified by design; missing live credentials do not block offline development and no Codex credential is an allowed substitute.

## Last Verification

- Pinned toolchain: Rust `1.93.1`; all final commands exited `0`.
- `cargo fmt --all -- --check`: PASS.
- `cargo clippy --locked --all-targets --all-features -- -D warnings`: PASS.
- `cargo test --locked --all-targets --all-features`: PASS; 44 passed, 0 failed, 1 ignored.
- `cargo build --locked`: PASS.
- `cargo build --locked --release`: PASS on local `x86_64-apple-darwin` with Rust `1.93.1`.
- `cargo build --locked --release --target aarch64-apple-darwin`: PASS; the resulting arm64 Mach-O SHA-256 is `d7e5ce5152d1f404aebb7cd6786d9d46059f82dcc2736a3081b7b284def0fdd0`.
- GitHub Actions first PR run `31387701048`: FAILED because the clean macOS runner did not contain the external `rg` binary required by the real `search_files` integration tests; the earlier shell guard also treated a missing command as a negative scan.
- CI portability fix `528c443`: PASS locally for the complete Tool-loop integration test; the Workflow now installs and verifies ripgrep before both the source-layout scan and Rust tests.
- GitHub Actions corrected run `31387944243`: PASS; all Gate steps completed on the clean macOS runner.
- GitHub Actions final PR-head run `31388093658`: PASS with merge state `CLEAN` and no unresolved reviews or comments.
- GitHub Actions post-merge `main` run `31388314842`: PASS for merge commit `9988b98`.
- M1a-S3 vertical Gate: PASS; a Scripted Mock Provider called real `search_files` against a temporary source fixture, received a paired Tool Result in a complete second ModelInputIR and returned the final vulnerability-candidate analysis.
- Tool coverage: PASS; fixed four-Spec Registry, read/list/search/no-match, argv-only command, nonzero exit, UTF-8-safe bounded output, stable fingerprints, Receipt/Result status, plan failure, cancellation and atomic multi-call rejection.
- OpenAI local-loopback coverage: stable text and Tool full-request JSON; User/Assistant Message, Function Call and Function Call Output order; split SSE Text -> Function Call -> Usage -> Completed; HTTP 401; `response.failed`; malformed/unknown events; missing completion; pending HTTP cancellation.
- Inline-test scan: PASS; no `#[cfg(test)]` or test module remains under `src/`.
- Dependency-boundary contract: PASS; Domain/App/Ports contain no OpenAI wire/HTTP client dependency and Adapters do not depend on App.
- Credential-pattern and production fallback scans: PASS; no real key pattern, generic `OPENAI_API_KEY` production fallback, `CODEX_HOME`, Codex CLI login or `.codex` path exists in production code.
- Forbidden-scope and Codex dependency scans: PASS; no ProviderSession, `previous_response_id`, Vision service, DeepSeek, Hook/Plugin Registry or forbidden Codex crate exists in production.
- `codex/` worktree: clean at `3aae5d885bac39c1262491aa3fd100dfd8b3919f`; root diff contains no `codex/` path.
- Frozen Architecture v1/v1.1: unchanged.
- Root ignore rules for `/codex/`, `/target/` and `**/.cyber-agent/`: present.
- `git diff --check`: PASS.
- `task.md` architecture-content seal SHA-256: `31b1cca7205ccce86cc30be263648966c008515b3cb26aee8645cb06bb28a2a5`.
- Current `task.md` SHA-256: `2718be4c8172b49cab00788f37ec210848ed255a09a9d8a840f0533c4bdbdf9f`.
- Git state: PR `#4` is `MERGED`; `main` and `origin/main` contain Squash Merge commit `9988b98`.
- Remote CI: PASS for final PR head (`31388093658`) and post-merge `main` (`31388314842`).
- Live Test: not run; no real Provider credential, public model request or Codex login state was read or used.

## Update Protocol

At Slice start:

- Set Slice state to `IN_PROGRESS`.
- Record branch and base commit after Git exists.
- Record intended files and acceptance commands.

At Slice completion:

- Set Slice state to `COMPLETE` for the completed Slice, then point Current Slice to the next `READY_NOT_AUTHORIZED` Slice.
- Record exact commands and results.
- Record commit/PR only if created with authorization.
- Stop execution.

If blocked:

- Keep Current Slice unchanged.
- Record exact blocker and completed safe work.
- Do not mark the Slice complete.
