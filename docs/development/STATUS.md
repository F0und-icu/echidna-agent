# Cyber Agent Development Status

## Current State

- Architecture version: `1.1.0`
- Phase: `IMPLEMENTATION`
- Implementation: `STARTED`
- Current milestone: `M1b`
- Current slice: `M1b-S2`
- Slice state: `IN_PROGRESS`
- Lean governance: `ACTIVE`; M1b-S2 is scope-locked to incremental Responses delivery and one bounded Full fallback
- Local Git initialized: `yes`; M1b-S2 is being developed on `agent/m1b-s2-incremental-responses`
- Remote repository: `F0und-icu/cyber-agent` private; PR `#6` was Squash Merged
- Documentation baseline commit: `55ff3be`
- Current code base: `main`/`origin/main` M1b-S1 status-seal commit `045862e` before this Slice branch
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
- Result: `COMPLETE`; all local and remote Cargo/Lean Gates pass.
- Dependencies and schemas: no dependency, Cargo, ModelInputIR/Event/Project/Research schema or frozen architecture change.
- Git: PR `#6` was Squash Merged as `a334415` (`M1b-S1: canonical ProviderSession baselines`).
- GitHub Actions: PR-head run `32517704380` and post-merge `main` run `32517919489` passed on macOS with Rust `1.93.1`; formatting, production test-layout scan, Clippy, 50 tests plus one ignored Live Smoke, and locked build all passed.
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

## In Progress

- Slice: `M1b-S2`
- Delivery branch: `agent/m1b-s2-incremental-responses`
- Base commit: `045862e`
- Scope lock: add a Provider-neutral request/response continuation boundary, keep the opaque Provider response identity outside Domain/Research/Context, map OpenAI `previous_response_id` plus strict append-only delta, and allow exactly one Full retry only after a typed continuation rejection.
- Intended production touch points: `ports/model`, `app/model_streaming`, `app/agent_loop`, `domain/context/baseline`, `adapters/mock_model` and `adapters/openai/{request,response,mod}`. A smaller final set is preferred; no new dependency is planned.
- Intended permanent tests: Port/stream contracts, OpenAI loopback request snapshots, Baseline reset behavior and the single-Agent Tool loop across same-Turn, next-Turn, rejection fallback and fresh-session/compaction reset paths.
- Atomicity constraint: a response ID and Incremental Baseline advance only after one complete model sample passes local validation and commits to History; failed/cancelled/invalid samples retain the last committed pair.
- Retry constraint: fallback retries the same logical model sample, never re-enters or replays a completed local Tool execution, and never applies to generic transport, HTTP, protocol or Provider failures.
- Live compatibility constraint: no public Provider request is part of this Slice Gate. The `store: false` plus `previous_response_id` behavior remains explicitly unverified until a separately authorized Live Test.

## Current Authorization

The current user instruction explicitly authorizes continued implementation with delegated Terra/Luna development and normal version-controlled remote delivery until the first prototype reaches the separately authorized Live Provider validation boundary.

- Run mode: `IMPLEMENT_TO_LIVE_BOUNDARY`
- Slice ID: `M1b-S2`; state `IN_PROGRESS`
- Git mode: `BRANCH_PR_REVIEW_MERGE`; commit, push, PR, CI review and merge are authorized when the Slice Gate passes
- Delegation mode: `AUTHORIZED`; delegated implementation Agents must independently read the repository authority chain and stay inside the current Slice
- Live test mode: `OFF`; no real Provider credential or public model request is authorized
- Forbidden continuation inside this Slice: M1c Batch/Scheduler/PTY/Job, Vision service, DeepSeek Adapter, CLI profile composition, Web, persistence, general Router/Hook/Plugin work, Codex source modification or Codex credentials/login access
- Stop condition: complete and remotely seal M1b-S2 before starting the next Slice; do not run a Live Test

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
- Remaining governance debt: none inside M1b-S1; remote delivery evidence is complete.

### M1b-S2 Draft Lean Review (awaiting parent review and Git delivery)

#### Semantic Owners

- `src/ports/model.rs` owns the provider-neutral `ModelRequest` delivery contract, opaque completion identity and the one capability-validation rule shared by Adapters.
- `src/domain/context/baseline.rs` still owns only semantic Baseline comparison, reset priority and the `incremental_input=false` Full-delivery reason. It contains no Provider response identity.
- `src/app/agent_loop.rs` is the sole owner of pairing a locally committed History/Baseline with an opaque continuation identity, selecting delta versus Full delivery and performing at most one typed-rejection Full fallback. Fallback counts against the existing 32-request Turn bound.
- `src/app/model_sample.rs` owns atomic completed-sample decoding, validation and staged History commit; it was split from the control loop by change reason without adding a Trait or public API.
- `src/adapters/openai/request.rs` and `response.rs` privately own Responses wire encoding/decoding. Only an incremental request with exact code `previous_response_not_found` is mapped to `ContinuationRejected`; all other Provider, protocol and HTTP failures remain non-fallback errors.

#### Reuse, API and Dependencies

- The existing `ModelInputIR`, History pairing, canonical Baseline and constrained HTTP/SSE stack are reused. The Port replaces a Domain `Completed` event with Port completion metadata, so OpenAI DTOs and response IDs remain outside Domain/Context/Research.
- The only new public Port surface is the real Adapter contract: `ModelRequest`, `ModelRequestDelivery`, `ProviderResponseId`, `ModelResponseCompletion` and `ModelStreamEvent`. No test-only API, Cargo dependency, feature, SDK, Router, Hook or persistence type was added.
- Main review split the 413-line mixed control/sample module into a 330-line AgentLoop and an 87-line private sample owner; further splitting was rejected because continuation delivery still has only one runtime consumer.
- The Codex WebSocket implementation and tests at fixed upstream `3aae5d885bac39c1262491aa3fd100dfd8b3919f` remain `REFERENCE`: the local HTTP/SSE implementation was reimplemented without copied source, so no Port Ledger entry or license change is required.

#### Test Layout, Removal and Deferred Work

- Top-level permanent tests cover same-Turn ToolResult delta, next-Turn User delta, capability-disabled Full delivery, Compaction reset assessment, fresh ProviderSession isolation, SSE and non-2xx exact rejection mapping, one Full fallback, partial-event discard, no Tool replay, generic-error no fallback, and failed/cancelled/empty/multiple-Tool samples retaining the committed continuation.
- Removed the full-request-only Port stream completion event and the misleading `record_full_response`/`assess_full_request` names. `record_completed_sample` now records either Full or incremental delivery from the same full logical snapshot.
- `store:false + previous_response_id` over real OpenAI HTTP/SSE remains explicitly unverified. No Live Test was run; the next proof requires separately authorized dedicated Cyber Agent credentials and a public Provider request.

## Blockers

- None for the completed M1b-S1 local implementation and Gate.
- M1b-S2 implementation passed parent architecture/Lean review and the local Gate; authorized commit/PR/CI delivery remains pending and is not a Live-Provider compatibility proof.
- Live Provider compatibility remains unverified by design; missing live credentials do not block offline development and no Codex credential is an allowed substitute.

## Last Verification

- M1b-S2 draft local Gate: PASS on Rust `1.93.1`; `cargo fmt --all -- --check`, `cargo clippy --locked --all-targets --all-features -- -D warnings`, `cargo test --locked --all-targets --all-features` (64 passed, 0 failed, 1 ignored), `cargo build --locked`, inline-test scan, Domain/App/Ports Provider-wire scan and `git diff --check` all exited `0`.
- M1b-S2 loopback verification: PASS; Full requests retain `store:false`, incremental requests contain only delta input plus the opaque previous response identity, exact HTTP/SSE rejection makes one Full fallback, and fallback does not persist partial stream output or re-execute a completed Tool.
- M1b-S2 Live Test: OFF; no Provider credential, public network request, Codex login state, `~/.codex` or `CODEX_HOME` was read or modified.

### Retained M1b-S1 verification evidence

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
- Historical M1b-S1 forbidden-scope and Codex dependency scans: PASS at that Slice; `previous_response_id` and incremental HTTP were then absent and are now intentionally implemented only inside the M1b-S2 OpenAI Adapter boundary.
- `codex/` worktree: clean at `3aae5d885bac39c1262491aa3fd100dfd8b3919f`; root diff contains no `codex/` path.
- Frozen Architecture v1/v1.1: unchanged.
- Root ignore rules for `/codex/`, `/target/` and `**/.cyber-agent/`: present.
- `git diff --check`: PASS.
- Git state: PR `#6` is `MERGED`; `main` and `origin/main` contain M1b-S1 Squash Merge commit `a334415` before this status-only handoff.
- Remote CI: PASS for PR head run `32517704380` and post-merge `main` run `32517919489`.
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
