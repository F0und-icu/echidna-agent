# Cyber Agent Development Status

## Current State

- Architecture version: `1.1.0`
- Phase: `IMPLEMENTATION`
- Implementation: `STARTED`
- Current milestone: `M1b`
- Current slice: `M1b-S2`
- Slice state: `READY_NOT_AUTHORIZED`
- Lean governance: `ACTIVE`; M1b-S1 is complete locally and awaiting its authorized remote delivery
- Local Git initialized: `yes`; completed M1b-S1 branch is `agent/m1b-s1-provider-baseline` from `main` commit `04da25b`
- Remote repository: `F0und-icu/cyber-agent` private; PR `#4` was Squash Merged
- Documentation baseline commit: `55ff3be`
- Current code base: completed M1b-S1 worktree on `agent/m1b-s1-provider-baseline`; delivery commit/PR pending
- Upstream Codex baseline: `3aae5d885bac39c1262491aa3fd100dfd8b3919f`

## Last Completed Slice

- Slice: `M1b-S1`
- Delivery branch: `agent/m1b-s1-provider-baseline`
- Base commit: `04da25b`
- Scope: TurnContext, StepSnapshot, Primary/Vision ProviderSession identity, stable Canonical Input and request-component hashes, non-destructive IncrementalBaseline comparison, complete stable invalidation reasons and AgentLoop-local assessment audit.
- Agent behavior: one primary AgentLoop captures a Turn-stable context and per-request StepSnapshot, evaluates compatibility before each model request, records a Baseline only after a completed sample commits to History, and still sends the complete ModelInputIR for every request.
- Canonical behavior: model and profile change independently; Tool contracts sort deterministically and canonicalize JSON recursively; ContentItem order and Tool Call/Result IDs remain semantic; internal request metadata and local Tool execution policy do not alter request equivalence.
- Test path: a Scripted Mock Provider completes two Turns and proves the second request is locally append-only while the Provider still receives complete History; the domain matrix covers Model/Profile/Prompt/Tool/Output/IR/Capability changes, stable priority, reset persistence, session isolation and modified/deleted/reordered History.
- Forbidden work preserved: no `previous_response_id`, incremental HTTP encoding, Provider-reject fallback, Batch/Scheduler/PTY/Job, Vision service, DeepSeek Adapter, Live Provider request, Codex credential/login access, Codex source modification or Codex code port.
- Result: `COMPLETE` locally; all Cargo and Lean Gates pass. Remote commit/PR/CI delivery remains in progress under the current authorization.
- Dependencies and schemas: no dependency, Cargo, ModelInputIR/Event/Project/Research schema or frozen architecture change.
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
- M1b-S1 Turn/Step capture, Canonical Request hashing, non-destructive ProviderSession Baseline comparison and full-request AgentLoop audit.

## Not Started

- M1b-S2 and later implementation; no response ID, incremental Provider request or fallback code has started.

## Current Authorization

The current user request authorizes the remaining normal remote delivery of completed `M1b-S1`. It does not authorize implementation of `M1b-S2` or a Live Test.

- Run mode: `DELIVER_COMPLETED_SLICE`
- Completed Slice ID: `M1b-S1`
- Next Slice ID: `M1b-S2`; state `READY_NOT_AUTHORIZED`
- Git mode: intentional commit, push, PR, CI review and merge for the completed M1b-S1 branch; a status-only remote handoff may record final merge evidence
- Delivery branch: `agent/m1b-s1-provider-baseline`; base commit `04da25b`
- Live test mode: `OFF`; no real Provider credential or public model request is authorized
- Forbidden continuation: do not implement `previous_response_id`, incremental HTTP encoding, Provider fallback or any later-Slice behavior during delivery
- Stop condition: after remote CI/merge and accurate status handoff, stop with M1b-S2 still `READY_NOT_AUTHORIZED`

## Slice Lean Review

### Semantic Owners

- `src/domain/context/canonical.rs`: the only Turn/Step capture, recursive JSON normalization and Model/Profile/IR/Capability/Prompt/Tool/Output/Request-properties hash owner.
- `src/domain/context/baseline.rs`: the only invalidation priority, strict append-only comparison and in-process ProviderSession Baseline state owner.
- `src/domain/context.rs`: a small facade preserving one `domain::context` contract without a generic manager or utility module.
- `src/app/agent_loop.rs`: the only runtime consumer; it captures snapshots, exposes the most recent local assessment for audit, continues full resampling and advances a Baseline only after atomic model-sample commit.
- `src/domain/history.rs`: remains the only Tool Call/Result pairing owner; Canonical comparison preserves IDs/order and never repairs History.

### Reuse and Duplication

- Reused the existing provider-neutral `ModelInputIR`, explicit `ModelCapabilities`, `ToolSpec`, History, BLAKE3 and Serde stack; no second request representation, hash crate or Provider SDK was added.
- Canonical Tool hashing includes model-facing name/version/description/input schema/output schema/exposure and deliberately excludes local execution scheduling, retry, cache, timeout and output policy.
- One canonical JSON routine owns nested object-key ordering for Tool schemas, Tool Call arguments and Tool Result output; ContentItem sequence is never sorted.
- ProviderSession assessment is non-destructive. A failed/cancelled request retains the old Baseline and reset reason; only successful `record_full_response` replaces it.
- The oversized first draft was split by change reason into Canonical and Baseline modules while retaining one semantic facade; no duplicate hash or invalidation implementation remains.

### Public API and Dependencies

- New formal Domain contracts are TurnContext, StepSnapshot, model/profile/session identities, RequestHash, BaselineAssessment/Reason and ProviderSession. IncrementalBaseline remains internal to ProviderSession.
- New App API is limited to explicit `AgentLoop::with_primary_context`, `last_baseline_assessment` and the typed primary-session configuration error; no test-only canonical helper is public.
- No dependency or Cargo feature changed. Existing BLAKE3 now also owns stable request-component fingerprints; existing Serde owns canonical serialization.
- ModelInputIR schema remains version 2. Output/Request hashes cover only semantics the current IR already expresses; actual output schema, reasoning/include properties and wire support remain deferred rather than silently ignored.
- No OpenAI DTO, response ID, HTTP/SSE type or Codex dependency entered Domain/App/Ports.

### Test Layout and Codex Classification

- Production `src/` contains no inline `#[cfg(test)]` module; 50 offline/local-loopback tests pass and one synthetic Live Smoke is ignored.
- `tests/provider_baseline.rs` permanently covers every requested invalidation component, complete priority, stable reason serialization, Tool order/metadata boundaries, internal metadata, session isolation and strict History append-only behavior.
- `tests/tool_loop_integration.rs` proves the one AgentLoop observes NoBaseline then append-only compatibility while the Mock Provider receives complete History on both requests.
- Existing credential, OpenAI loopback, Tool, pairing, cancellation and real temporary-project tests remain unchanged or extended; no test reads a real environment credential.
- Codex incremental-context behavior was `REFERENCE`; Canonical comparison and ProviderSession state are local `REIMPLEMENT`. No source was copied and no Port Ledger entry or license change is required.

### Removed and Deferred

- Removed the destructive assessment draft that cleared Baseline/reset state before request success, the discarded AgentLoop assessment, an unused last-full-request hash and test-only public Canonical accessors.
- Removed local Tool execution metadata from Provider request equivalence and removed an infallible `expect` from Tool sorting; sort keys are computed once with typed error propagation.
- Deferred exactly as scoped: response IDs, `previous_response_id`, incremental wire requests, Provider rejection fallback, Resume/Compaction runtime wiring, actual output schema/reasoning/include contracts, Batch/Scheduler/PTY/Job, Vision and CLI composition.
- Stable reset reason codes exist for audit vocabulary, but no Provider rejection/fallback behavior was implemented.
- No unused feature flag, generic Hook/Router, Provider SDK, persistent Schema, copied Codex code or unconsumed adapter path was retained.
- Remaining governance debt: none inside M1b-S1; remote CI/merge evidence is pending the authorized delivery.

## Blockers

- None for the completed M1b-S1 local implementation and Gate.
- Remote delivery evidence is pending the authorized commit/PR/CI/merge workflow.
- M1b-S2 is intentionally stopped at `READY_NOT_AUTHORIZED`.
- Live Provider compatibility remains unverified by design; missing live credentials do not block offline development and no Codex credential is an allowed substitute.

## Last Verification

- Pinned toolchain: Rust `1.93.1`; all final commands exited `0`.
- `cargo fmt --all -- --check`: PASS.
- `cargo clippy --locked --all-targets --all-features -- -D warnings`: PASS.
- `cargo test --locked --all-targets --all-features`: PASS; 50 passed, 0 failed, 1 ignored.
- `cargo build --locked`: PASS.
- M1b-S1 vertical Gate: PASS; two completed Turns produce NoBaseline then append-only local assessment while the Mock Provider still receives complete History.
- Invalidation coverage: PASS; Model/Profile/IR/Capability/Prompt/Tool/Output/Request-properties changes, stable priority/reason codes, reset persistence, Primary/Vision identity separation and modified/deleted/reordered History are permanent regressions.
- Canonical coverage: PASS; stable Tool order, recursive JSON key normalization, Tool pair IDs and ContentItem order remain semantic; internal metadata and local Tool execution policy are excluded.
- Prior Tool/OpenAI coverage remains PASS, including real temporary-project search, full Tool history encoding, HTTP/SSE errors and cancellation.
- Inline-test scan: PASS; no `#[cfg(test)]` or test module remains under `src/`.
- Dependency-boundary contract: PASS; Domain/App/Ports contain no OpenAI wire/HTTP client dependency and Adapters do not depend on App.
- Credential-pattern and production fallback scans: PASS; no real key pattern, generic `OPENAI_API_KEY` production fallback, `CODEX_HOME`, Codex CLI login or `.codex` path exists in production code.
- Forbidden-scope and Codex dependency scans: PASS; no `previous_response_id`, incremental HTTP, Vision service, DeepSeek, Hook/Plugin Registry or forbidden Codex crate exists in production.
- `codex/` worktree: clean at `3aae5d885bac39c1262491aa3fd100dfd8b3919f`; root diff contains no `codex/` path.
- Frozen Architecture v1/v1.1: unchanged.
- Root ignore rules for `/codex/`, `/target/` and `**/.cyber-agent/`: present.
- `git diff --check`: PASS.
- Git state: completed M1b-S1 changes are uncommitted on `agent/m1b-s1-provider-baseline`; base `main == origin/main == 04da25b` before delivery.
- Remote CI: pending M1b-S1 PR delivery.
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
