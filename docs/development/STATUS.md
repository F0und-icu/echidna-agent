# Cyber Agent Development Status

## Current State

- Architecture version: `1.1.0`
- Phase: `IMPLEMENTATION`
- Implementation: `STARTED`
- Current milestone: `M1c`
- Current slice: `M1c-S2`
- Slice state: `READY_AUTHORIZED`; implementation starts only after this M1c-S1 status seal is remotely merged
- Lean governance: `ACTIVE`; M1c-S1 is complete and M1c-S2 remains scope-locked to ordinary background Jobs
- Local Git initialized: `yes`; M1c-S1 is being sealed on `agent/m1c-s1-merge-status`
- Remote repository: `F0und-icu/cyber-agent` private; PR `#10` was Squash Merged
- Documentation baseline commit: `55ff3be`
- Current code base: `main`/`origin/main` M1c-S1 merge commit `59fc30f`
- Upstream Codex baseline: `3aae5d885bac39c1262491aa3fd100dfd8b3919f`

## Last Completed Slice

- Slice: `M1c-S1`
- Delivery branch: `agent/m1c-s1-batch-scheduler`
- Base commit: `4d08b42`
- Scope: `read_files`, multi-pattern `search_files`, argv-only serial/parallel `run_commands`, typed Resource Keys, conflict-aware scheduling, completed Receipt dedupe and explicit rerun audit.
- Agent behavior: the one AgentLoop stages multiple Tool Calls from one completed sample, executes non-conflicting plans concurrently, preserves model order and appends one paired Result for every planned, failed or cancelled call.
- Resource behavior: Device, PTY and Output keys are exclusive even for a mislabelled Parallel plan; independent files/devices/outputs can overlap, unresolved interactive work remains globally serial, and path identities canonicalize existing symlink parents.
- Retry behavior: semantic idempotency excludes execution identity; a non-cancelled completed Receipt prevents duplicate execution within the AgentLoop process, while an explicit rerun replaces the current Receipt and records its reason.
- Test path: permanent Domain, Adapter and AgentLoop regressions cover partial Batch failures, output bounds, resource waves, cancellation, cross-Turn dedupe, rerun replacement, symlinked outputs and multiple Tool Call pairing.
- Forbidden work preserved: no background Job state, PTY runtime, persistence, Vision service, DeepSeek Adapter, CLI profile composition, Live Provider request, Codex credential/login access, Codex source modification or Codex code port.
- Result: `COMPLETE`; all local and remote Cargo/Lean Gates pass.
- Dependencies and schemas: no dependency, Cargo, ModelInputIR/Event/Project/Research schema or frozen architecture change.
- Git: PR `#10` was Squash Merged as `59fc30f` (`feat(tools): add batch resource scheduler`).
- GitHub Actions: PR-head run `32564693085` and post-merge `main` run `32564797656` passed on macOS with Rust `1.93.1`; formatting, production test-layout scan, Clippy, 73 tests plus one ignored Live Smoke, and locked build all passed.
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
- M1b-S2 Provider-neutral continuation, strict OpenAI Responses delta delivery, atomic Baseline/response identity advancement and bounded typed-rejection Full fallback.
- M1c-S1 Batch tools, typed Resource Key scheduling, ordered multi-Tool pairing and in-process completed Receipt dedupe with explicit rerun audit.

## Next Authorized Slice

- Slice: `M1c-S2`
- Planned delivery branch: `agent/m1c-s2-background-jobs`
- Base commit: the M1c-S1 status-seal merge commit, to be recorded at Slice start
- Scope lock: add ordinary non-PTY background process Jobs with typed Job ID/status, bounded stdout/stderr offset polling, input/stop operations, timeout/cancellation and completed Receipt integration.
- Required behavior: polling returns only bytes after the caller's offset, process completion is stable, stop/cancel leaves no orphaned process or corrupt Job state, and completed dedupe does not relaunch the same execution.
- Forbidden continuation: M1c-S3 PTY sessions, project persistence, Artifact/Event stores, Vision, DeepSeek Adapter, CLI profile composition, general Workflow/Hook/Router/Plugin abstractions, Live Provider requests and Codex credential/login access.

## Current Authorization

The current user instruction explicitly authorizes continued implementation with delegated Terra/Luna development and normal version-controlled remote delivery until the first prototype reaches the separately authorized Live Provider validation boundary.

- Run mode: `IMPLEMENT_TO_LIVE_BOUNDARY`
- Slice ID: `M1c-S2`; state `READY_AUTHORIZED` pending the separate status-seal merge
- Git mode: `BRANCH_PR_REVIEW_MERGE`; commit, push, PR, CI review and merge are authorized when the Slice Gate passes
- Delegation mode: `AUTHORIZED`; delegated implementation Agents must independently read the repository authority chain and stay inside the current Slice
- Live test mode: `OFF`; no real Provider credential or public model request is authorized
- Forbidden continuation inside M1c-S2: PTY sessions, Vision service, DeepSeek Adapter, CLI profile composition, Web, persistence, Artifact/Event stores, general Router/Hook/Plugin work, Codex source modification or Codex credentials/login access
- Stop condition: remotely merge this M1c-S1 status seal before starting M1c-S2; do not run a Live Test

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
- ProviderSession assessment is non-destructive. A failed/cancelled request retains the old Baseline and reset reason; only successful `record_completed_sample` advances it.
- The oversized first draft was split by change reason into Canonical and Baseline modules while retaining one semantic facade; no duplicate hash or invalidation implementation remains.

### Public API and Dependencies

- New formal Domain contracts are TurnContext, StepSnapshot, model/profile/session identities, RequestHash, BaselineAssessment/Reason and ProviderSession. IncrementalBaseline remains internal to ProviderSession.
- New App API is limited to explicit `AgentLoop::with_primary_context`, `last_baseline_assessment` and the typed primary-session configuration error; no test-only canonical helper is public.
- No dependency or Cargo feature changed. Existing BLAKE3 now also owns stable request-component fingerprints; existing Serde owns canonical serialization.
- ModelInputIR schema remains version 2. Output/Request hashes cover only semantics the current IR already expresses; actual output schema, reasoning/include properties and wire support remain deferred rather than silently ignored.
- No OpenAI DTO or HTTP/SSE type entered Domain/App/Ports. Provider response identity is opaque at the Port/App boundary and remains absent from Domain/Context/Research; no Codex dependency was added.

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

### M1b-S2 Lean Review

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

### M1c-S1 Lean Review

#### Semantic Owners

- `src/domain/tools/resource.rs` owns typed `ResourceKey` values and the one pure Resource/Effect/Parallelism conflict predicate. Device, PTY and Output keys remain exclusive even if a future Adapter mislabels a plan as Parallel, while parallel read-only File plans may share a key. `src/app/tool_scheduler.rs` is the sole Turn-local consumer that starts non-conflicting plans, preserves model order and fills every staged Tool Call with a Result on cancellation.
- `src/domain/tools.rs` owns normalized Plan fingerprints, the stable `dedupe_by_key` identity, explicit rerun control and the receipt-carried rerun reason. The scheduler indexes only non-cancelled completed Receipts in the current AgentLoop process; an explicit rerun bypasses a prior Receipt and replaces it after completion.
- `src/adapters/tools/resources.rs` owns concrete project-relative path resolution to File/Output keys, including a symlink-aware existing-parent canonicalization. `src/adapters/tools/command.rs` owns the one command Resource JSON Schema, argv execution and batch command waves; it reuses the Domain conflict predicate rather than defining a second lock rule.
- `src/adapters/tools/filesystem.rs` and `search.rs` own their respective batch argument/result handling while continuing to use the existing bounded-output owner. `src/app/model_sample.rs` remains the atomic model-sample staging owner; AgentLoop only invokes the Scheduler after staging valid Tool Calls.

#### Reuse, API and Dependencies

- Added `read_files`, multi-pattern `search_files` and explicit serial/parallel `run_commands`; each keeps individual partial failures in its ordered output and all commands remain argv arrays.
- Added typed Resource/Idempotency/Rerun contracts to the existing Tool Domain, plus one private App scheduler. The explicit-rerun constructor remains `pub(crate)` because only the built-in Adapter consumes it; the existing public Plan constructor remains the Port-facing contract. No new Port, Cargo dependency, Cargo feature, Provider SDK, Hook/Router, Job, PTY or persistence abstraction was added.
- The existing History pairing, cancellation authority, BLAKE3/Serde fingerprints, Tokio process timeout and bounded output implementation are reused. OpenAI wire types remain Adapter-private; no Codex source or dependency was copied (local implementation is `REIMPLEMENT`).

#### Test Layout, Removal and Deferred Work

- Permanent regressions are external: resource concurrency/serialization, batch partial failure and ordered pairing, cancellation, cross-Turn dedupe, explicit rerun replacement, typed key normalization, exact command-resource schemas, missing output behind a symlinked parent, built-in batch tools and multi-Tool model samples. `src/` has no inline test module.
- Split the former mixed `process.rs` by distinct change reason into `command.rs` and `search.rs`; removed its duplicate responsibility. The command batch uses the Domain predicate rather than a second conflict implementation. Final review also preserved the pre-existing exact bounded-text behavior of un-ranged `read_file`, including trailing newlines, while range selection stays limited to `read_files`. No unused generic scheduler, schema-builder, cache or future feature was retained.
- Deferred exactly as scoped: persistent ExecutionReceipts/Events, Tool cache, background Jobs, PTY sessions, Artifact storage, Project/Research updates, Vision/DeepSeek Adapter, CLI profile composition and real Provider validation. `parallel_tool_calls` remains unchanged in the OpenAI wire Adapter; AgentLoop support for multiple calls is local and protocol-neutral.

## Blockers

- None for the completed M1c-S1 local and remote delivery Gates.
- M1c-S2 can start after this status-only seal is remotely merged under the current user authorization.
- Live Provider compatibility remains unverified by design; missing live credentials do not block offline development and no Codex credential is an allowed substitute.

## Last Verification

- M1b-S2 local Gate: PASS on Rust `1.93.1`; `cargo fmt --all -- --check`, `cargo clippy --locked --all-targets --all-features -- -D warnings`, `cargo test --locked --all-targets --all-features` (64 passed, 0 failed, 1 ignored), `cargo build --locked`, inline-test scan, Domain/App/Ports Provider-wire scan and `git diff --check` all exited `0`.
- M1b-S2 loopback verification: PASS; Full requests retain `store:false`, incremental requests contain only delta input plus the opaque previous response identity, exact HTTP/SSE rejection makes one Full fallback, and fallback does not persist partial stream output or re-execute a completed Tool.
- M1b-S2 Live Test: OFF; no Provider credential, public network request, Codex login state, `~/.codex` or `CODEX_HOME` was read or modified.
- M1b-S2 remote Gate: PASS; PR `#8` head run `32561668586` and post-merge `main` run `32561848176` passed, and independent Terra xhigh review returned APPROVE with no medium-or-higher issue.
- M1c-S1 local Gate: PASS on Rust `1.93.1`; `cargo fmt --all -- --check`, `cargo clippy --locked --all-targets --all-features -- -D warnings`, `cargo test --locked --all-targets --all-features` (73 passed, 0 failed, 1 ignored synthetic Live Smoke) and `cargo build --locked` all exited `0`. Inline-test, forbidden Codex dependency, dedicated-credential, Provider-wire boundary, `git diff --check` and `codex/`-unchanged scans also passed.
- M1c-S1 Live Test: OFF; no credential, external model request, Codex login state, `~/.codex` or `CODEX_HOME` was read or modified.
- M1c-S1 remote Gate: PASS; PR `#10` head run `32564693085` and post-merge `main` run `32564797656` passed, and independent Luna xhigh review returned APPROVE with no P0/P1 issue before the final Domain exclusivity fix and full Gate rerun.

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
