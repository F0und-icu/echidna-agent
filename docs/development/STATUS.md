# Cyber Agent Development Status

## Current State

- Architecture version: `1.1.0`
- Phase: `IMPLEMENTATION`
- Implementation: `STARTED`
- Current milestone: `M2`
- Current slice: `M2-S4`
- Slice state: `READY_AUTHORIZED`; implementation starts only after this M2-S3 status seal is remotely merged
- Lean governance: `ACTIVE`; M2-S3 is complete and M2-S4 remains scope-locked to Project Memory, Snapshot and rebuildable Cache
- Local Git initialized: `yes`; M2-S3 is being sealed on `agent/m2-s3-merge-status`
- Remote repository: `F0und-icu/cyber-agent` private; M2-S3 implementation PR `#20` was Squash Merged
- Documentation baseline commit: `55ff3be`
- Current code base: `main`/`origin/main` M2-S3 merge commit `15b2f88`
- Upstream Codex baseline: `3aae5d885bac39c1262491aa3fd100dfd8b3919f`

## Last Completed Slice

- Slice: `M2-S3`
- Delivery branch: `agent/m2-s3-artifact-lineage`
- Base commit: `6705acd`
- Scope: project-isolated SHA-256 Blob storage, versioned Artifact facts, provenance-preserving ArtifactRecord/Lineage replay and the fixed `artifact_store` tool.
- Content behavior: each source is streamed once into durable staging while hashing; identical bytes deduplicate only the Blob, existing destinations are rehashed, and final publication never overwrites.
- Provenance behavior: distinct Artifact IDs retain producer, TargetSnapshot, parent, transform and toolchain links; Artifact and outer Event identities are exact, and successful Receipts carry output Artifact IDs.
- Persistence behavior: one project Writer covers replay, preflight, Blob publication, Event append and reducer apply; new Blob ancestry is durably synced, and Cache deletion cannot remove Artifact or Blob facts.
- Test path: permanent external regressions cover deduplication versus provenance, lineage/identity/version failures, Target/parent/transform validation, symlink/nonregular paths, project isolation, Cache independence and one AgentLoop Receipt path.
- Forbidden work preserved: no Memory/Snapshot/Cache, file-write Tool Cache, Context/Vision/Profile composition, destructive Blob GC, Live request, Codex credential/login access, Codex source modification or Codex code port.
- Result: `COMPLETE`; all local and remote Cargo/Lean Gates pass and independent Luna xhigh review found no unresolved P0/P1 after Receipt and directory-durability fixes.
- Dependencies and schemas: adds only `sha2 0.10.9` with default features disabled, recorded in ADR-0004; the outer Research schema remains v1 and Artifact payload schema v1 is independently strict.
- Git: PR `#20` was Squash Merged as `15b2f88` (`feat(artifacts): add durable lineage store`).
- GitHub Actions: PR-head run `32586565660` and post-merge `main` run `32586668048` passed on macOS with Rust `1.93.1`; formatting, production test-layout scan, Clippy, 138 tests plus one ignored Live Smoke, and locked build all passed.
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
10. `docs/adr/ADR-0004-sha256-artifact-blob-identity.md`

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
- M1c-S2 ordinary background Jobs, lifecycle Resource leases, stable launch/completion linkage and byte-offset output polling.
- M1c-S3 current-platform PTY Sessions, shared live-process leases, stable terminal output and interactive controls.
- M2-S1 isolated local projects, strict JSONL Event streams, one project Writer lock, corrupt-tail recovery, chained Upcasters and source-preserving export/migrate.
- M2-S2 versioned Research facts, immutable TargetSnapshot, ExperimentCapsule, deterministic fingerprints, strict replay and fixed Research tools.
- M2-S3 durable SHA-256 Blobs, versioned Artifact facts, provenance-preserving Lineage and the fixed Artifact tool.

## Next Authorized Slice

- Slice: `M2-S4`
- Planned delivery branch: `agent/m2-s4-memory-snapshot-cache`
- Base commit: the M2-S3 status-seal merge commit, to be recorded at Slice start
- Scope lock: add the minimum Project Memory facts, external-edit audit, two-level ProjectFingerprint, rebuildable Project Snapshot/File Manifest/Tool Capability Cache and one fixed `inspect_project` path.
- Required behavior: unchanged projects reuse snapshots; changed files receive BLAKE3 only on demand; deleting Cache rebuilds from project/Memory/Research/Artifact facts; two projects never share Memory or derived state.
- Intended production touch points: typed Memory revision contracts, local Markdown Memory Adapter, pure ProjectFingerprint/Snapshot contracts, one rebuildable local Cache Adapter and fixed `memory_read`/`memory_write`/`inspect_project` tools.
- Intended permanent tests: internal and external Memory edits, old/new Hash plus revision Blob/diff audit, metadata-first invalidation, on-demand file hashing, cache deletion/rebuild and project isolation.
- Forbidden continuation: M2-S5 write_file/apply_patch/Tool Result Cache, Context Planner, Mode/Profile/Vision, Compaction/Request Audit, general database/index/watcher/hook/router/plugin abstractions, Live Provider requests and Codex credential/login access.

## Current Authorization

The current user instruction explicitly authorizes continued implementation with delegated Terra/Luna development and normal version-controlled remote delivery until the first prototype reaches the separately authorized Live Provider validation boundary.

- Run mode: `IMPLEMENT_TO_LIVE_BOUNDARY`
- Slice ID: `M2-S4`; state `READY_AUTHORIZED` pending the separate M2-S3 status-seal merge
- Git mode: `BRANCH_PR_REVIEW_MERGE`; commit, push, PR, CI review and merge are authorized when the Slice Gate passes
- Delegation mode: `AUTHORIZED`; delegated implementation Agents must independently read the repository authority chain and stay inside the current Slice
- Live test mode: `OFF`; no real Provider credential or public model request is authorized
- Forbidden continuation inside M2-S4: file writes/Tool Cache, Context Planner, Mode/Profile/Vision, Compaction/Request Audit, general database/index/watcher/hook/router/plugin work, Codex source modification or Codex credentials/login access
- Stop condition: remotely merge this M2-S3 status seal before starting M2-S4; do not run a Live Test

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

### M1c-S2 Lean Review

#### Semantic Owners

- `src/domain/tools.rs` owns the provider-neutral `JobId`, lifecycle `JobStatus` and terminal `JobCompletion` contract. Completion is a terminal observation explicitly linked to the original start execution and plan fingerprint; it is deliberately not a second, conflicting `ExecutionReceipt`.
- `src/domain/tools/resource.rs` extends the existing sole Resource conflict predicate with exclusive `Job` keys. The App scheduler serializes same-Job calls, while the process-local Job table reuses that same predicate to retain a running Job's resolved resource lease until terminal publication.
- `src/adapters/tools/job/` is the only in-process ordinary-Job owner: `launch.rs` owns argv spawn, `control.rs` owns poll/input/stop contracts, `output.rs` owns bounded caller-offset views and UTF-8-safe resume boundaries, and `supervisor.rs` owns timeout/stop race resolution, direct-child reaping and terminal-state publication. `JobEntry` owns the named Running-to-Stopping and terminal transitions; its table is intentionally process-local and has no persistence or recovery claim.
- `src/adapters/tools/command.rs` remains the sole argv validation and command Resource resolution owner. The Job adapter reuses its typed arguments and validation instead of creating a second shell/Resource parser.
- `src/adapters/tools/mod.rs` only exposes four fixed Direct Tools: `start_job`, `poll_job`, `input_job` and `stop_job`. It does not add a Hook, Job Manager Port, polling cursor or general background-runtime abstraction.

#### Reuse, API and Dependencies

- `start_job` is explicitly `dedupe_by_key`; its immediate Receipt records only successful launch and prevents duplicate launch in the current AgentLoop. `poll_job` and `stop_job` return the stable terminal completion linked to that start identity. `input_job` and `stop_job` remain `never_auto_retry`.
- Reused existing `ToolExecutionPlan`, Receipt, output policy, Resource scheduler, FullAccess argv execution and Tokio process stack. The existing Tokio dependency's selected features extend only with `io-util`, `sync` and `macros` for piped I/O, one-shot stop signalling, watch-based terminal notification and the small supervisor; no crate, project-level Cargo feature, Provider dependency, Port or Codex code was added.
- The output buffer retains only the first bounded bytes of each stream and reports `truncated`; poll offsets are explicit byte offsets supplied by the caller, never a hidden global cursor. For whole valid UTF-8 or a valid prefix with only an incomplete tail, a trailing code point is withheld and continuation-byte offsets are rejected; arbitrary invalid bytes permit every byte offset and remain explicitly lossy only at the display boundary.
- The small new public Domain types have concrete consumers in the Tool Adapter and external regressions. Existing `RunCommandArgs`/Resource validation visibility is `pub(super)` only, not public API expansion.

#### Test Layout, Removal and Deferred Work

- Permanent external regressions cover argv-only planning, empty Job IDs, input close, stdout/stderr incremental offset reads, valid UTF-8 partial/continuation boundaries, arbitrary-binary offsets, repeated poll stability, out-of-range offsets, timeout, stop, natural-completion/stop race, descendant-held pipes, stable start-completion linkage, active resource leases and their terminal release, same-Job scheduling and cross-Turn start dedupe.
- No production inline tests, Job cache, persistent Job/Event/Artifact state, Project recovery, PTY session, terminal resize, Web/CLI composition, Provider change or Live request is introduced. The direct child is reaped before terminal Job state is published; descendant process-group management remains outside this ordinary non-PTY Slice. Terminal Job entries currently remain in the process-local table without a retention cap; retention/eviction semantics are explicit P2 governance debt for M2-S1, where persisted lifecycle facts can make eviction auditable, rather than an unsafe cache added before persistence exists.
- Codex classification is `REIMPLEMENT`: no source was copied and no Port Ledger entry is required. Final Cargo/Lean Gate evidence is recorded only after the full local verification pass.

### M1c-S3 Lean Review

#### Semantic Owners

- `src/domain/tools.rs` now owns the provider-neutral `PtySessionId`, `PtySessionStatus` and one terminal `PtySessionCompletion`, linked to the original `start_pty` execution rather than represented as a second Receipt.
- `src/adapters/tools/pty/` is the sole live current-platform PTY owner: `launch.rs` validates argv and creates a terminal, `control.rs` owns poll/input/resize/stop contracts, and `supervisor.rs` owns the dedicated blocking direct-child wait, bounded reader drain and single terminal publication. It does not introduce a Port, a PTY runtime or a cross-platform compatibility layer.
- `pty/control.rs` exceeds the review signal because all four fixed operations share the same typed-ID validation, table snapshot and terminal-wait contract; splitting a one-consumer control surface now would either duplicate those rules or create an empty abstraction. Its next split trigger is an independently changing control path with distinct state or I/O semantics.
- `src/adapters/tools/output.rs` is the one byte-capture and caller-offset owner for both ordinary Jobs and PTY output, including incomplete UTF-8 withholding, continuation-boundary rejection, binary display semantics and the terminal seal that rejects late reader mutations. The former private Job-only output module was removed instead of copied.
- `src/adapters/tools/active.rs` is the small concrete live-process lease owner now justified by its two real consumers: Jobs and PTY sessions. It delegates every Resource/Effect/Parallelism decision to `domain/tools/resource.rs`; one RAII guard performs atomic pre-spawn admission, survives through terminal publication and closes spawn-failure/cancellation release paths without a second Job/PTy conflict table.

#### Reuse, API and Dependencies

- Fixed Direct Tools are `start_pty`, `poll_pty`, `input_pty`, `resize_pty` and `stop_pty`. Their typed session arguments, `pty:<session>` control keys and exact schemas are consumed by the existing Registry, Tool scheduler and AgentLoop; no test-only public API was added.
- PTY open/setup/spawn and synchronous writer, resize and kill calls run through `tokio::task::spawn_blocking`; blocking read and wait use dedicated standard threads. Reader/writer handles are established before the direct child is spawned, and an abandoned setup result owns kill-plus-wait cleanup. Terminal publication occurs only after the direct child has been reaped, writer/master closure, bounded drain and output sealing.
- Added `portable-pty 0.9.0` with `default-features = false`. Its only responsibility is native current-platform PTY creation, child handles and resize; the standard library cannot create a PTY and the existing Tokio process stack is pipe-only. The crate has no enabled default features, adds no Provider/Web/Server/Codex dependency, uses MIT licensing, and remains replaceable at this Adapter boundary.
- `start_pty` is `dedupe_by_key`; input/resize/stop are `never_auto_retry`, and poll is replay-safe. No tool output cache, Event/Artifact schema, persistence contract, configuration surface or frozen architecture file changed.
- Codex classification is `REIMPLEMENT`: no Codex PTY source was inspected for implementation or copied, so no Port Ledger entry or notice change is required.

#### Test Layout, Removal and Deferred Work

- `tests/pty_sessions.rs` permanently covers interactive echo, real terminal resize, caller byte offsets, partial UTF-8/continuation offsets, arbitrary binary bytes, stop/natural-completion race, direct-child terminal completion, terminal output sealing against attempted descendant late writes, spawn-before-side-effect Job-to-PTY and PTY-to-Job conflict/release, same-session `pty:<session>` scheduling keys, empty IDs and exact model-facing schemas. `tests/tool_loop_integration.rs` now asserts the expanded fixed Tool set and schema count. No production inline tests exist.
- Final local Gate: PASS on Rust `1.93.1`; `cargo fmt --all -- --check`, `cargo clippy --locked --all-targets --all-features -- -D warnings`, `cargo test --locked --all-targets --all-features` (88 passed, 0 failed, 1 ignored synthetic Live Smoke) and `cargo build --locked` all exited `0`. The seven PTY regressions also passed 100 consecutive runs after final lifecycle review.
- Removed the Job-only lease conflict table and Job-only output owner; all retained Job regressions pass through the shared implementations. No generic manager, Hook, Trait, router, plugin or future configuration was retained.
- Deferred: PTY persistence/recovery, terminal retention/eviction policy, process-group/descendant management, broad Windows/Linux compatibility claims, terminal emulation/history/search, CLI profiles, Artifacts/Events, Vision, Provider changes and Live validation. Current Unix PTY EOF treats the native master `EIO` close condition as terminal EOF; any broader portability work needs a measured future Slice.
- Remaining governance debt: terminal Job and PTY entries remain process-local without retention/eviction because no persistent lifecycle fact exists yet; a PTY reader that survives the bounded post-reap drain is sealed, explicitly marked incomplete and detached rather than blocking terminal publication. Rare standard-thread creation failure and already-running `spawn_blocking` control cancellation remain P2 lifecycle debt; neither changes the stable completion or lease rules on the verified path.

### M2-S1 Lean Review

#### Semantic Owners

- `src/domain/project.rs` owns the pure typed `ProjectId`; it is deliberately separate from Event schema semantics. `src/domain/events.rs` owns the pure `EventUpcasterRegistry`, requiring exact chained `vN -> vN+1` links and producing explicit missing-link, invalid-step and invalid-output failures without filesystem, clock or project-state access.
- `src/adapters/project_event_store/durable_file.rs` is the private, single owner of durable no-overwrite file publication and Unix directory syncing. It writes and syncs a same-directory `create_new` temporary file, publishes only through a hard link that cannot replace a final name, then cleans the temporary name best-effort and syncs the parent. `project.rs` consumes it for the final `project.toml` sentinel and then syncs the project root; `migration.rs` consumes it for both target and last-written manifest; JSONL recovery consumes it for the content-addressed raw-tail Artifact before replacing the source stream.
- `src/adapters/project_event_store/project.rs` owns canonical-root project initialization/discovery, safe Session physical-path segments, the stable `research` versus `session:<safe-id>` persisted namespace mapping, and the Rust `1.93` standard-library advisory `File::try_lock` boundary. An existing `.cyber-agent/` without `project.toml` is reported as incomplete without modification; initialization prepares required directories and a new empty Research stream before the durable final sentinel. Discovery recomputes `ProjectId` from the canonical root and rejects a configured identity mismatch. Only `WouldBlock` maps to `WriterLocked`; every other lock failure remains a typed Adapter I/O error.
- `src/adapters/project_event_store/jsonl/scan.rs` is the sole JSONL byte-boundary, strict stream-ID/next-sequence and read-time Upcast owner. Only a lexical/parse failure of the final terminated record, or an unterminated final record, can reach recovery; parsed schema, Upcaster, stream and sequence failures—including in the final record—are exact-offset read-only corruption. Unknown `event_type` is intentionally not classified here and remains deferred to the M2-S2 reducer. `jsonl/recovery.rs` is the sole raw-tail Artifact and atomic replacement owner; it reuses the durable owner’s directory sync after persisting the Artifact and after same-directory stream replacement.
- `src/adapters/project_event_store/migration.rs` owns the explicit non-destructive export/migrate skeleton under the `ProjectWriter` lock: it scans one fixed source-byte snapshot, pre-encodes a new target plus BLAKE3 source hash/target schema/tool-version manifest, and writes the manifest last as the completion sentinel. An existing pair is complete only when the manifest parses with a current schema, a formatted BLAKE3 digest and non-empty tool version and the target JSONL scans completely for the requested stream; otherwise it is `IncompleteMigration`. It never compares a retained historical hash against the live source or migrates source bytes in place.

#### Reuse, API and Dependencies

- The filesystem Adapter implements the existing generic `EventSource`/`EventSink`/`EventStore` Port for `serde_json::Value`; no second Event Port, reducer, Research record type, Blob Store, cache or generic manager was added. `Project`, `ProjectWriter`, `JsonlEventStore`, `ProjectEventStream`, explicit `RecoveryMetadata` and the narrow empty-tool-version Adapter error are the minimum runtime-facing contracts consumed by the `init` composition root and future Event callers. The durable-file owner is adapter-private rather than a generic transaction or utility surface.
- Added only direct dependency `toml 0.8.23` (with its locked parser closure). The standard library has no TOML parser; using this narrow Serde-compatible parser avoids a fragile hand-written parser for escaped canonical paths and project configuration validation. Existing Serde JSON, BLAKE3 and `thiserror` own their prior responsibilities.
- Codex classification is `REIMPLEMENT`: no Codex source, crate or Port was used. `codex/`, frozen architecture files and accepted ADRs remain unchanged.

#### Test Layout, Removal and Deferred Work

- Permanent external regressions are in `tests/`: real CLI initialization and parseable final TOML without durable temporary files, canonical-root Project-ID verification, upward discovery/two-project isolation, repeat-init and incomplete-init non-mutation, same- and cross-process second Writer rejection, Session(`research`) namespace isolation, Port reuse and strict sequence from one, durable lexical-tail Artifact/atomic-recovery ordering plus collision no-mutation, semantic-final-tail read-only exact-offset diagnostics, invalid-first-sequence no-mutation, source-preserving manifest-sentinel export, complete-pair no-overwrite, malformed-manifest and truncated/semantic-target `IncompleteMigration`, and the fixed historical v1 fixture traversing a pure v1-to-v2-to-v3 test registry with malformed-registry controls. `src/` has no inline test modules.
- The original mixed Adapter draft was split by filesystem/layout, durable no-overwrite publication, JSONL writing, byte scanning, recovery and migration change reasons; no `manager`, Hook, router, plugin or future configuration surface remains. The project/layout module is now 297 lines, below the 300-line review signal after the precisely shared durability rules moved to the 110-line private durable-file owner; the shared Adapter error boundary and export-path work remain with their real owners.
- Deferred exactly as scoped: M2-S2 Research/Target/Capsule records and reducers, including unknown `event_type` validation, Blob/Lineage beyond the required recovery raw-byte Artifact, Memory/Snapshot/Cache, Context, Vision, Provider/Profile/Web composition and all Live Provider work. A failed pre-sentinel init can leave only uncommitted preparation directories or an empty new Research file, never a readable `project.toml`; a failed recovery can retain its raw-tail Artifact while leaving the source unchanged; export target and manifest deliberately remain a non-transactional two-file pair, with a target-only or invalid pair diagnosed as `IncompleteMigration` and no automatic repair. Neither condition is promoted into a new recovery or filesystem abstraction in this Slice.
- Remaining governance debt (P2): the JSONL scanner requires the first persisted sequence to be `1`, while the generic Domain reducer permits any first strictly-increasing sequence; an `append_event` `sync_data` failure can leave an uncertain external append while its in-memory Writer is not poisoned; non-Unix directory syncing remains a documented no-op; and a committed durable file’s temporary-name cleanup is best effort and can leave a harmless temporary file. Each requires a dedicated persistence/portability Slice rather than broadening M2-S1.

#### Delivery Gate

- Local Gate: PASS on Rust `1.93.1`; `cargo fmt --all -- --check`, `cargo clippy --locked --all-targets --all-features -- -D warnings`, `cargo test --locked --all-targets --all-features` (114 passed, 0 failed, 1 ignored existing synthetic Live Smoke) and `cargo build --locked` all exited `0`.
- Lean scans: PASS; `src/` inline-test scan, forbidden Codex dependency scan, credential-pattern scan, `git diff --check`, and `codex/` unchanged check all exited `0`.
- Independent Luna xhigh review returned `APPROVE` with no P0/P1. PR `#16` and post-merge `main` passed the same remote Rust Gate before this status seal; no credential or Live Provider request was used.

### M2-S2 Draft Lean Review

#### Semantic Owners

- `src/domain/canonical_json.rs` is the sole recursive JSON object-key normalizer for Domain hashes. Context request hashing and Research action fingerprints share it; arrays and scalar values retain their original semantic order and representation.
- `src/domain/research/fingerprint.rs` owns Research identity normalization plus `dedup_key`, `snapshot_fingerprint` and `action_fingerprint`. Target fingerprints exclude capture time, notes and analysis-tool version. Action identity contains normalized tool/action and arguments, target Snapshot, tool version, input Artifacts, environment delta, toolchain fingerprint and normalized account-prerequisite set; it deliberately excludes execution, receipt and output-result IDs.
- `src/domain/research/records.rs` owns typed Research IDs, records, typed evidence references, constructor-derived fingerprints and field validation. Blob/Artifact Lineage is not implemented; Artifact and Receipt references remain typed values only.
- `src/domain/research/reducer.rs` owns versioned Research Event encoding/decoding, event-local intrinsic validation, strict unknown/payload-version rejection and replay. An Event ID is idempotent only after full structural/intrinsic validation and only when its BLAKE3 digest of exact occurrence time, exact Event type and canonical typed payload matches; a different validated semantic Event with that ID is rejected. It also owns state-dependent record/reference validation, append-only state updates, explicit rerun admission and `RecordsMerged` provenance. Mutable-record updates stage and validate only the affected cloned record before replacing it, so failed public reductions do not partially mutate ResearchState. `HypothesisRecorded` and `FindingRecorded` cannot carry `merged_into`; the existing `StreamRecovered` control Event is strictly accepted without becoming a Research fact so M2-S1 recovery remains readable.
- `src/adapters/project_event_store/jsonl/mod.rs` owns the narrow crate-private existing-Research-stream reader, reusing the one scanner/upcaster without a Writer lock or append-mode file handle. `src/adapters/tools/research/mod.rs` owns fixed Research argument normalization, read/locked-update execution and state helpers; its private `research/schema.rs` owns all fixed Research JSON Schema construction. `research_get` uses the reader; `research_update` takes the Writer lock once, opens the already initialized stream, reads/reduces/validates, appends one durable Event, then applies that same Event in memory. `src/adapters/tools/mod.rs` only wires the two fixed specs into the existing Registry and one AgentLoop.

#### Public API, Schemas and Reuse

- `domain::research` exposes the minimum typed Research records, IDs, Events, reducer/state and deterministic fingerprints used by the Tool adapter and permanent external regressions. Reducer-only record-reference lookups remain private rather than widening the public API. The persisted `research_schema_version: 1` payload envelope and fixed `research.*` Event names reject unknown or malformed Research data before any new append.
- `research_get` is a read-only, replay-safe fixed Tool and cannot create `.cyber-agent/writer.lock`, files or directories; it explicitly fails when the initialized `research.jsonl` is missing. `research_update` is a `LocalMutation`, `NeverAutoRetry`, resource-locked fixed Tool and requires that same initialized stream rather than recreating it. Its tagged operations create or append-only update TargetSnapshot, Hypothesis, Experiment/Capsule, Finding and NextAction facts, or write an explicit merge; no second Registry, Port, transaction system or general workflow abstraction was introduced.
- No dependency, Cargo feature, Provider path, frozen architecture file or ADR changed. Existing Serde, BLAKE3, generic JSONL EventStore, Project Writer lock and Tool scheduler are reused. Canonical JSON ownership was extracted from Context instead of duplicated.
- Codex classification is `REIMPLEMENT`: no Codex source or crate was copied or added, so no Upstream Port Ledger entry or license change is needed.

#### Test Layout and Verification

- Permanent coverage is external: `tests/research_domain.rs` covers metadata-excluded snapshot identity, nested canonical action arguments, case-sensitive action boundaries, toolchain/account-prerequisite versus execution-result fingerprint boundaries, atomic reducer failures, first-Hypothesis missing-Experiment evidence rejection without State mutation, strict Event-ID idempotence (including malformed, wrong-stream, unknown, future-version and bad-recovery revalidation before duplicate success) and recovered-stream compatibility. `tests/research_tools.rs` covers fixed schemas, lock-free Reader behavior, missing-stream no-recreation, first-Hypothesis missing-Experiment no-append behavior, explicit dedup/rerun choices, updates/replay, confirmed Finding evidence/Snapshot validation, persisted forged-merge-provenance rejection, unknown/malformed/unsupported/invalid persisted events and project isolation. `tests/tool_loop_integration.rs` proves the existing Mock Provider invokes `research_update` through one AgentLoop.
- Local Gate: PASS on Rust `1.93.1`; `cargo fmt --all -- --check`, denied-warning `cargo clippy --locked --all-targets --all-features`, `cargo test --locked --all-targets --all-features` (130 passed, 0 failed, 1 ignored existing synthetic Live Smoke) and `cargo build --locked` all exited `0`. Inline-test, forbidden Codex dependency, credential-pattern, no-whole-ResearchState-clone, `git diff --check` and `codex/`-unchanged scans passed.
- Live Test: `OFF`; no Provider credential, public request, Codex login state, `~/.codex` or `CODEX_HOME` was read or modified.

#### Deferred Scope and Debt

- Deferred exactly as locked: Blob Store/Artifact Lineage beyond typed references, Memory/Snapshot/Cache, Context Planner, Vision, Provider/Profile/Web composition, a generic workflow/transaction/hook/router/plugin system, and Live Provider validation.
- P2 debt: the synchronous JSONL scan and `sync_data` append run inside the asynchronous fixed-Tool future. A future offload must preserve the current atomic visibility boundary: a timeout or cancellation may not return while a write can later complete. M2-S4 Snapshot work is the scoped way to reduce the full-stream replay cost. M2-S1 JSONL durability/portability debt remains unchanged and is not broadened here.
- Split review: `research/reducer.rs` and `research/records.rs` exceed the 300-line review signal, but a mechanical family split was rejected because it would expose or duplicate the single reference, provenance, fingerprint and staged-update invariants. Their next split trigger is an independently changing schema/upcaster or Snapshot-hydration path with a real consumer; that work must move cohesive ownership rather than add a Manager or Registry.

### M2-S3 Lean Review

#### Semantic Owners

- `src/domain/artifacts/{records,event,reducer}.rs` owns the provider-neutral Artifact contract: strict lowercase SHA-256 `BlobId` syntax, provenance-preserving `ArtifactRecord`, root-versus-derived parent/transform rules, the independent `artifact_schema_version: 1` payload and the pure ordered Lineage projection. One Blob can support several Artifact records; duplicate Artifact IDs, duplicate/self/missing parents and malformed/unknown Artifact Event types reject.
- `src/domain/research/reducer.rs` remains the single Research stream/replay owner. It decodes `artifact.created` and `artifact.derived` inside the existing `research_schema_version: 1` envelope, requires `created_by_event_id` to equal the outer Event ID, preserves validated Event-ID idempotence/conflict behavior, and checks Artifact TargetSnapshot references before applying lineage state. Existing typed future Artifact references remain backward-compatible and gain no retroactive validation.
- `src/adapters/artifact_store.rs` is the sole local-byte owner. It streams a regular source once into same-directory durable staging while hashing SHA-256, creates every Blob ancestry component only after rejecting symlinks/non-directories and syncs each new parent entry before Event append, hard-link-publishes without overwrite, and rehashes an existing regular destination before reporting deduplication. `artifact_store/error.rs` is split only for its separate typed error mapping; the Adapter remains below the 300-line review signal.
- `src/adapters/tools/artifact.rs` owns the fixed Tool's ordering: take the project Writer, replay and preflight every Event/Artifact/TargetSnapshot/parent reference, publish the Blob, append the Event, then apply the same envelope in memory. An append failure cannot create an Event that references a missing Blob; an unreachable Blob may remain after an append or late publication failure, with no GC in this Slice.

#### Public API, Dependencies and Cancellation

- `ports/artifact.rs` adds only `ArtifactStore` and `StoredBlob`; the local Adapter is intentionally direct rather than a second Event, cache, transaction, graph or workflow Port. The fixed `artifact_store` Tool is `ResourceLocked`, `LocalMutation`, `NeverAutoRetry`, `EnvironmentDependent` and cache `None`, with source and Research-stream resource keys.
- Added direct `sha2 0.10.9` with `default-features = false`, recorded in ADR-0004. Its sole responsibility is streamed cryptographic SHA-256 Blob identity; its locked closure is `block-buffer`, `cpufeatures`, `crypto-common`, `digest`, `generic-array`, `sha2`, `typenum` and `version_check`. Handwritten crypto, system `shasum` and accidental transitive use were rejected. No Codex source, crate or port was used; classification is REIMPLEMENT.
- `source_path` is a caller-supplied local provenance label, not content identity or storage location; it can be absolute and is never telemetry. `created_by_event_id`, the Research Artifact Event, producer Tool Call and TargetSnapshot form the audited provenance chain without an invented duplicate research-event field.
- The existing `ExecutionReceipt.output_artifact_ids` remains the sole receipt-to-Artifact link. A narrow crate-private success constructor populates it for `artifact_store`; no second Receipt shape or Adapter-side Receipt construction was introduced. Unused public Artifact projection lookups/counts were removed and `ArtifactState` was returned to crate-private visibility.
- Artifact publication and Event append run synchronously inside the fixed Tool future: no `spawn`, `spawn_blocking` or detached task is introduced. A cancellation/timeout cannot return while a later Artifact write is still running; as with M2-S2, this direct blocking path remains explicit P2 responsiveness debt rather than a new unsafe offload abstraction.

#### Test Layout and Verification

- `tests/artifact_store.rs` adds seven permanent external regressions for exact Tool schema/effect/retry/resource policy; same Blob with distinct provenance plus derived lineage and receipt linkage; conflicting Event/Artifact IDs with no new Blob; exact outer/inner Event identity and version rejection; root/derived transform, TargetSnapshot and parent failures before publication; wrong/nonregular existing Blobs and sources; Unix source/destination/ancestor symlink plus non-directory rejection; project isolation; and Cache-independent fact replay. `tests/tool_loop_integration.rs` adds the fixed one-AgentLoop vertical path and verifies the paired Receipt carries the Artifact ID. Production `src/` retains no inline test modules.
- Final local Gate: PASS on Rust `1.93.1`; `cargo fmt --all -- --check`, `cargo clippy --locked --all-targets --all-features -- -D warnings`, `cargo test --locked --all-targets --all-features` (138 passed, 0 failed, 1 ignored existing synthetic Live Smoke) and `cargo build --locked` all exited `0`.
- Lean scans: PASS; inline-test, forbidden Codex-dependency, credential-isolation, `git diff --check` and `codex/`-unchanged scans passed. The credential scan matched only the established `CYBER_AGENT_OPENAI_API_KEY` constant, not a generic fallback. Live Test remains `OFF`; no credential, public Provider request, Codex login state, `~/.codex` or `CODEX_HOME` was read or modified.

#### Removed and Deferred

- The oversized Artifact Store draft was split by typed error mapping. No generic Artifact manager, graph API, Hook, router, plugin, Blob cache, detached worker or unused configuration remains.
- Deferred exactly as scope-locked: destructive Blob GC/retention, remote/object Blob storage, Memory/Project Snapshot/Cache, file-write Tool cache, Context Planner, Vision, Provider/Profile/Web composition and Live Provider validation. Orphan-Blob reclamation and non-Unix directory-sync behavior remain persistence work for a separately authorized Slice.
- Deterministic fault injection for a post-Blob Event append failure and a scheduler cancellation racing the synchronous commit remain P2 test-harness debt. The production ordering already publishes and syncs the Blob before append, never detaches commit work, and therefore cannot leave an Event that references a missing Blob or perform a late write after Tool return. Adversarial local filesystem TOCTOU hardening is also deferred under the trusted single-user threat model.
- `research/reducer.rs` exceeds the 300-line review signal but retains the cohesive Research Event decode/idempotence/reference/apply invariant. A mechanical Artifact split would duplicate or expose that invariant; the split trigger is a separately consumed Research replay/upcaster family, not a generic coordinator.

## Blockers

- None for the completed M2-S3 local and remote delivery Gates or the authorized M2-S4 start after this status seal.
- M2-S4 remains scope-locked to Project Memory, Snapshot and rebuildable Cache; file writes and deterministic Tool Result Cache start only in M2-S5.
- Live Provider compatibility remains unverified by design; missing live credentials do not block offline development and no Codex credential is an allowed substitute.

## Last Verification

- M2-S3 local Gate: PASS on Rust `1.93.1`; formatting, denied-warning Clippy, 138 offline/local tests plus one ignored existing synthetic Live Smoke, locked build, inline-test scan, forbidden Codex dependency scan, credential-pattern scan, `git diff --check` and `codex/`-unchanged check all passed. No Live Test was run.
- M2-S3 review: independent Luna xhigh returned `APPROVE` with no unresolved P0/P1 after the successful Receipt gained its Artifact ID, new Blob ancestry became crash-durable and unused public Artifact projection APIs were removed.
- M2-S3 Live Test: `OFF`; no credential, external model request, Codex login state, `~/.codex` or `CODEX_HOME` was read or modified.
- M2-S3 remote Gate: PASS; PR `#20` was Squash Merged as `15b2f88`, PR-head run `32586565660` passed, and post-merge `main` run `32586668048` passed the same Rust Gate.
- M2-S2 local Gate: PASS on Rust `1.93.1`; formatting, denied-warning Clippy, 130 offline/local tests plus one ignored existing synthetic Live Smoke, locked build, inline-test scan, forbidden Codex dependency scan, credential-pattern scan, no-whole-ResearchState-clone scan, `git diff --check` and `codex/`-unchanged check all passed.
- M2-S2 review: independent Luna xhigh returned `APPROVE` with no unresolved P0/P1 after exact validated Event-ID idempotence, merge provenance, strict missing-stream behavior, failure-atomic updates and initial Hypothesis evidence integrity were verified.
- M2-S2 Live Test: `OFF`; no credential, external model request, Codex login state, `~/.codex` or `CODEX_HOME` was read or modified.
- M2-S2 remote Gate: PASS; PR `#18` was Squash Merged as `ba8d880`, PR-head run `32583858745` passed, and post-merge `main` run `32583981820` passed the same Rust Gate.
- M2-S2 status seal: PASS; PR `#19` was Squash Merged as `6705acd`, PR-head run `32584183819` passed, and post-merge `main` run `32584270821` passed the same Rust Gate before M2-S3 started.

- M2-S1 local Gate: PASS on Rust `1.93.1`; formatting, denied-warning Clippy, 114 offline/local tests plus one ignored existing synthetic Live Smoke, locked build, inline-test scan, forbidden Codex dependency scan, credential-pattern scan, `git diff --check` and `codex/`-unchanged check all passed.
- M2-S1 review: independent Luna xhigh returned `APPROVE` with no P0/P1 after the final durability, canonical Project identity, Session namespace and migration-integrity fixes.
- M2-S1 Live Test: `OFF`; no credential, external model request, Codex login state, `~/.codex` or `CODEX_HOME` was read or modified.
- M2-S1 remote Gate: PASS; PR `#16` was Squash Merged as `b2c69a1`, PR-head run `32578862449` passed, and post-merge `main` run `32578967609` passed the same Rust Gate.
- M2-S1 status seal: PASS; PR `#17` was Squash Merged as `a73bcce`, PR-head run `32579106280` passed, and post-merge `main` run `32579208575` passed the same Rust Gate before M2-S2 started.

- M1c-S3 local Gate: PASS on Rust `1.93.1`; formatting, Clippy with denied warnings, 88 offline/local-loopback tests plus one ignored synthetic Live Smoke, locked build, inline-test scan, forbidden Codex dependency scan, dependency-tree review, `git diff --check` and `codex/` unchanged check passed. The seven PTY regressions passed 100 consecutive runs.
- M1c-S3 review: independent Luna xhigh first returned `REQUEST_CHANGES` for post-terminal mutable output, post-spawn lease admission, synchronous PTY setup on Tokio and partial-setup child cleanup. The final diff adds a shared terminal seal, pre-spawn RAII lease, blocking setup isolation and abandoned-child kill-plus-wait; Luna's second review returned `APPROVE` with no unresolved P0/P1.
- M1c-S3 Live Test: `OFF`; no credential, external model request, Codex login state, `~/.codex` or `CODEX_HOME` was read or modified.
- M1c-S3 remote Gate: PASS; PR `#14` was Squash Merged as `4fb42da`, PR-head run `32574343739` passed, and post-merge `main` run `32574436821` passed the same Rust Gate.

- M1b-S2 local Gate: PASS on Rust `1.93.1`; `cargo fmt --all -- --check`, `cargo clippy --locked --all-targets --all-features -- -D warnings`, `cargo test --locked --all-targets --all-features` (64 passed, 0 failed, 1 ignored), `cargo build --locked`, inline-test scan, Domain/App/Ports Provider-wire scan and `git diff --check` all exited `0`.
- M1b-S2 loopback verification: PASS; Full requests retain `store:false`, incremental requests contain only delta input plus the opaque previous response identity, exact HTTP/SSE rejection makes one Full fallback, and fallback does not persist partial stream output or re-execute a completed Tool.
- M1b-S2 Live Test: OFF; no Provider credential, public network request, Codex login state, `~/.codex` or `CODEX_HOME` was read or modified.
- M1b-S2 remote Gate: PASS; PR `#8` head run `32561668586` and post-merge `main` run `32561848176` passed, and independent Terra xhigh review returned APPROVE with no medium-or-higher issue.
- M1c-S1 local Gate: PASS on Rust `1.93.1`; `cargo fmt --all -- --check`, `cargo clippy --locked --all-targets --all-features -- -D warnings`, `cargo test --locked --all-targets --all-features` (73 passed, 0 failed, 1 ignored synthetic Live Smoke) and `cargo build --locked` all exited `0`. Inline-test, forbidden Codex dependency, dedicated-credential, Provider-wire boundary, `git diff --check` and `codex/`-unchanged scans also passed.
- M1c-S1 Live Test: OFF; no credential, external model request, Codex login state, `~/.codex` or `CODEX_HOME` was read or modified.
- M1c-S1 remote Gate: PASS; PR `#10` head run `32564693085` and post-merge `main` run `32564797656` passed, and independent Luna xhigh review returned APPROVE with no P0/P1 issue before the final Domain exclusivity fix and full Gate rerun.
- M1c-S2 local Gate: PASS on Rust `1.93.1`; `cargo fmt --all -- --check`, `cargo clippy --locked --all-targets --all-features -- -D warnings`, `cargo test --locked --all-targets --all-features` (81 passed, 0 failed, 1 ignored synthetic Live Smoke) and `cargo build --locked` all exited `0`. The seven Job regressions passed 20 consecutive runs after the binary-offset fix; after remote feedback replaced a fixed sleep with an observed descendant-start marker, that pipe-drain race regression passed another 100 consecutive local runs. Inline-test, forbidden Codex dependency, dedicated-credential, Provider-wire boundary, dependency-tree, `git diff --check` and `codex/`-unchanged scans passed.
- M1c-S2 review: independent Luna xhigh returned `APPROVE_WITH_FIX`; its binary-offset P1 was fixed with a permanent regression, the documentation wording was corrected and the full local Gate was rerun successfully. No unresolved P0/P1 issue remains; unbounded process-local terminal retention and transient resource-busy dedupe remain explicit P2 debt.
- M1c-S2 Live Test: OFF; no credential, external model request, Codex login state, `~/.codex` or `CODEX_HOME` was read or modified.
- M1c-S2 remote Gate: PASS; PR `#12` final head run `32572323825` and post-merge `main` run `32572471229` passed. The first two PR runs failed only because the descendant-pipe Fixture assumed wall-clock scheduling; it now observes a start marker and passed 100 consecutive local runs before the final remote Gate.

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
