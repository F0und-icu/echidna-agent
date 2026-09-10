# Cyber Agent Development Status

## Current State

- Architecture version: `1.6.0`; v1/v1.1 remain remotely sealed, v1.2 through v1.6 are locally frozen with remote seal unauthorized
- Phase: `IMPLEMENTATION`
- Implementation: `STARTED`
- Current milestone: `M5` optional interface and personal distribution evaluation
- Current slice: `M5-S2`
- Slice state: `READY_NOT_AUTHORIZED`; M5-S1a and M5-S1b are complete locally, while no M5-S2 implementation is authorized
- Lean governance: `PASS`; M5-S1 retains one AgentLoop, Session/Research/Receipt authorities and fixed CLI stages without automatic Compaction, a Hook registry, second state source or public test surface
- Local Git initialized: `yes`; dirty local `main` tracks `origin/main`, and no M5 commit or remote operation is authorized
- Remote repository: `F0und-icu/cyber-agent` private; F2a is sealed through `[skip ci]` PR `#62`, while no tag, release or Actions run was created
- Documentation baseline commit: `55ff3be`
- Current code base: M5-S1a and M5-S1b are independently audited and locally gated on one dirty local `main`, but neither is committed or remotely sealed

## M5-S1a Slice Start

- Objective: make the existing single-user local CLI operational for sustained vulnerability research through multiline input, Session lifecycle commands, exact Resume continuity, visible Tool execution and auditable exports.
- Scope Lock: add only `/paste ... /end`, `session list/show/end/export`, `chat --resume latest`, one additive durable DomainProfile selection, and fixed typed Tool start/terminal presentation over existing facts. No dependency, generic Hook/Observer registry, TUI/Web/Server, second AgentLoop, second History, Provider branch or fact store is allowed.
- Resume rule: a new Session durably selects default `source` before its first real Turn; an explicit `--domain` or `/domain` selection is durably appended before it becomes effective; Resume without an override restores the latest valid selection. A legacy Session without this event resolves to `source` and writes the legacy default only before its next real Turn.
- Export rule: Markdown/JSON are derived read-only projections over the selected Session plus existing Research/Artifact/Receipt/Vision references. Export cannot append facts or promote Summary/Vision/model prose to Evidence.
- Verification order: focused tests, affected tests, independent read-only audit, repair, full locked Cargo/Lean Gate and Lean Review. M5-S1b cannot begin until this Gate is satisfied.
- Authorization boundary: local implementation through M5-S1b is authorized. Commit, push, PR, Actions, Live Test, release, tag and Web/TUI remain unauthorized.

## M5-S1a Pre-Audit Checkpoint

- Implemented locally: `/paste ... /end`; `chat --resume latest` with `--session` compatibility; strict `session list/show/end/export`; durable `session.domain_profile_selected` default/operator/legacy continuity; fixed scheduler-start and post-Receipt Tool terminal output.
- Semantic reuse: Session JSONL/Reducer remains the lifecycle and Domain authority; exports are derived only; Tool start observes the exact planned execution at scheduler launch, while terminal output occurs only after the existing Receipt stage durably appends the real Receipt.
- Focused tests: PASS, 3/3 M5 CLI tests across `cli_chat_loopback` and `cli_smoke`.
- Affected tests: PASS, 89/89 across `cli_chat_loopback`, `cli_smoke`, `domain_events`, `event_upcasting`, `m4_research_control`, `session_resume`, `tool_loop_integration` and `vision_flow`.
- Gate state: `AWAITING_INDEPENDENT_AUDIT`. No complete Cargo/Lean Gate or Lean Review has been claimed; M5-S1b has not started.
- External effects: no credential read, Provider Live request, Codex login access, Git commit/push/PR, Actions run, tag, release or Web/TUI work occurred.
- Upstream Codex reference baseline: `868c9edb0da913a5fc699a71664e65f44f6058b0`; read-only and not a Cyber Agent dependency

## M5-S1a Completion and Lean Review

- Outcome: `COMPLETE_LOCAL`. M5-S1b remains `AUTHORIZED_NOT_STARTED`; no M5-S1b production or test code has begun.
- Independent audit: the first Terra xhigh review returned three P1 findings—`session end` could race an active Chat, presentation failure could block Tool dispatch, and resource-wave cancellation could fabricate Receipts for plans never started—and two P2 findings for latest-Session ordering and ambiguous dedupe output. Repairs made `session end` use the Chat runtime lease, made presentation best-effort, represented unstarted cancellation as a paired pre-execution result without a Receipt, selected latest Active Session by durable activity with a stable tie-break, and labeled dedupe followers with their source Receipt.
- Closure audit: bounded Terra xhigh re-review found no P0/P1 and one P2 for an unnecessarily public Tool-start stage. The stage was narrowed to `pub(crate)`; a final read-only closure review returned `APPROVE` with no P0/P1/P2. The denied-warning Gate additionally required boxing the large internal executed-result enum variant; the auditor confirmed this is storage-only and preserves scheduler semantics.
- Permanent regressions: strict malformed Domain selection; new/default/operator/legacy/Resume Domain continuity; durable-activity `latest`; active-Chat `session end` exclusion; multiline command-literal preservation; JSON/Markdown authority labels and export no-overwrite; one start/one Receipt dedupe visibility; failed/cancelled terminal labels; and resource-wave cancellation with no Receipt for an unstarted plan.
- Complete Gate: `PASS` on Rust `1.93.1`. `cargo fmt --all -- --check`, denied-warning `cargo clippy --locked --all-targets --all-features`, `cargo test --locked --all-targets --all-features`, `cargo build --locked`, the production inline-test scan, forbidden Codex-dependency scan and `git diff --check` all exited `0`. The suite discovers 394 tests: 392 default-enabled local/offline/loopback tests pass and two explicit Live Smokes remain ignored.
- Semantic owners: `domain/domain_profile` owns validated Domain selections; the existing Session Event/Reducer owns durable continuity; `cli_session` is the private CLI Session command and derived-export boundary over strict replay; `tool_scheduler` alone distinguishes executed from not-started plans; the fixed AgentLoop/CLI stage observes scheduler start and reports terminal state only after the existing durable Receipt stage.
- Reuse and duplication review: Project discovery, Chat runtime lease, Session Writer/Reducer, History pairing, RequestAudit, ExecutionReceipt, Vision and Compaction projections are reused directly. No second lifecycle state, History, export fact, cancellation rule, Receipt type, Provider path or Context owner was added. Repeated presentation calls consume the one durable disposition rather than reimplementing receipt identity.
- API/dependency/module review: no crate, Cargo feature, Port, Adapter, Config, Provider/Wire branch, Hook/Observer registry, TUI/Web/Server or Codex dependency was added. The new CLI and scheduler surfaces are `pub(crate)`/`pub(super)`; public Domain selection types have current Session Event/Reducer and CLI production consumers. The 457-line private `cli_session` module triggered the required split review and remains one cohesive Session command/projection boundary; splitting it now would add forwarding surface without a second consumer or semantic owner. Existing `cli_chat` remains the single REPL/runtime handoff rather than duplicating an input state machine.
- Removed/deferred: the repair removed fallible start-presentation control flow and fabricated pending-plan Receipts. Line editing, TUI/Web, automatic Compaction, M5-S1b behavior, Git/Actions/Live and remote sealing remain outside this completed Slice.
- External effects: no real credential resolution or Provider request, Codex login/config access, Git commit/push/PR, Actions run, Live Test, tag, release or Web/TUI work occurred.

## M5-S1b Slice Start

- Objective: connect the existing strict Evidence-Spine Session Compaction to production CLI Chat so the operator can explicitly compact, continue with one `CompactionReset` Full request and later Resume the same validated continuation.
- Scope Lock: add only one deterministic production `SessionSummaryDraft` composition path, `/compact`, one ContextDecisionManifest-derived pressure projection and permanent top-level tests. Reuse the existing Session/Event/Research/Artifact/Receipt/ContextPlanner/RequestAudit/AgentLoop Compaction owners; no Event schema, Provider/Wire/Config field, dependency, renderer, model-summary call, automatic workflow, database, Hook registry, second AgentLoop or second History is allowed.
- Failure boundary: missing current TargetSnapshot, empty resumable History, incomplete Tool pairing, stale/cross-project/missing/type-mismatched EvidenceSpine references, Research identity drift, Session/Provider identity mismatch or durable append/apply failure must stop explicitly without deleting or rewriting original facts. A candidate Summary never changes AgentLoop state; only the exact durable Compaction may be applied.
- Pressure rule: `normal`/`warning`/`critical` and `/compact` guidance are derived only from the existing ContextDecisionManifest budget/decision facts. Missing Provider usage remains unknown and must not be synthesized as token usage. Pressure never auto-compacts or mutates a Session.
- Verification order: M5-S1b focused tests, affected tests, a brand-new Terra xhigh independent read-only audit, repair, complete locked Cargo/Lean Gate and Lean Review. Completion stops at M5-S2 `READY_NOT_AUTHORIZED`.
- Authorization boundary: local implementation and one new independent read-only audit thread are authorized. Commit, push, PR, Actions, Live Test, release, tag, M5-S2 and Web/TUI remain unauthorized; Codex login/configuration remains out of scope.

## M5-S1b Pre-Audit Checkpoint

- Implemented locally: formal Chat `/compact`; one deterministic current-Target Evidence-Spine/Narrative projection; append-before-apply production composition; one `ContextDecisionManifest`-only `normal`/`warning`/`critical` pressure projection exposed by `/context` and non-normal reminders; and Compaction-reset causal preservation across a later Tool-exposure reset.
- Semantic reuse: strict Research/Session replay remains the only fact input; `EvidenceSpine`, `SessionState::next_compaction`, `SessionProjection::append`, `AgentLoop::apply_compaction`, `ProviderSession`, ContextPlanner decisions and History pairing retain their existing ownership. The CLI adds no Event, Provider, Config, renderer, model-summary or alternate History path.
- Failure boundary: current-Target absence, empty user-resumable History and an interrupted Tool pair all fail before a Compaction Event; the active Session/Primary binding and strict runtime-vs-durable History are preflighted; after a successful append any apply failure exits Chat for strict Resume rather than continuing with divergent state.
- Focused tests: `PASS`, 25/25 across the ten `m5_` CLI regressions, eight Session Compaction tests, six AgentLoop Compaction tests and the ProviderSession reset-causality regression.
- Affected tests: `PASS`, 166/166 across `cli_chat_loopback`, `cli_smoke`, `context_planner`, `m3_world_state_mode`, `m4_research_control`, `provider_baseline`, `request_audit`, `research_domain`, `research_evidence`, `research_tools`, `session_compaction`, `session_resume` and `tool_loop_integration`.
- Gate state: `AWAITING_NEW_INDEPENDENT_AUDIT`. The complete Cargo/Lean Gate and Lean Review have not yet run; M5-S2 remains unauthorized.
- External effects: no credential resolution, Provider request beyond local loopback Fixtures, Codex login/config access, Git commit/push/PR, Actions run, Live Test, tag, release or Web/TUI work occurred.

## M5-S1b Completion and Lean Review

- Outcome: `COMPLETE_LOCAL`. M5-S2 is now `READY_NOT_AUTHORIZED`; no M5-S2 production or test code has begun.
- Production result: formal Chat `/compact` deterministically composes one current-Target reference-only Summary v2 from strict Research/Session facts, appends the exact Compaction before applying it to the one AgentLoop, and preserves `CompactionReset` until the next successful Full request. `/context` reports manifest-derived `normal`/`warning`/`critical` pressure and only suggests explicit `/compact`; it never mutates or automatically compacts.
- Independent audits: the first Terra xhigh audit returned one P1 because the Adapter duplicated and weakened the existing Target applicability rule, allowing a Finding bound only to an older same-target/version Snapshot and its Experiment/Artifact/Receipt/NextAction closure into the current spine. The repair reused `ResearchState::research_reference_target_applicability` and added one narrow exact-Snapshot Evidence-reference owner with current production consumers. A new Terra xhigh closure audit then found one remaining P1: current `TargetSnapshot.target_artifact_ids` bypassed that exact owner. The second repair routed this initial path through the same owner, explicitly failed missing references and excluded incompatible old-Snapshot Artifacts. Final bounded re-review returned `APPROVE` with no P0/P1/P2.
- Permanent regressions: the production CLI regression now constructs old and current Snapshots with identical target/version, makes the current Snapshot list both old and current Artifact IDs, retains only the exact-current Artifact, excludes the old Finding/Experiment/Artifact/Receipt/NextAction closure, proves exactly one Compaction Event, and proves the next same-process request and later Resume are Full with the reference-only Summary. Existing tests retain missing Target, empty resumable History, incomplete Tool pair, pending image, durable append/apply, reset causality and no-auto-Compaction boundaries.
- Verification sequence: `PASS` in the required order—focused 25/25, affected 166/166, independent audit, repair, closure re-review, complete locked Cargo Gate and Lean Review. The final suite discovers 397 tests: 395 default-enabled local/offline/loopback tests pass and two explicit Live Smokes remain ignored.
- Complete Gate: `PASS` on Rust `1.93.1`. `cargo fmt --all -- --check`, denied-warning `cargo clippy --locked --all-targets --all-features`, `cargo test --locked --all-targets --all-features` and `cargo build --locked` all exited `0`. Production inline-test, forbidden Codex-dependency, credential-like diff, unchanged `codex/`, dependency-diff and `git diff --check` scans also passed.
- Semantic owners and reuse: `ResearchState` remains the only current-Target applicability authority for Research and Evidence references; strict Project Research/Session replay builds the ephemeral validation facts; existing `EvidenceSpine`, `SessionState::next_compaction`, `SessionProjection::append`, `AgentLoop::apply_compaction`, History pairing, ContextPlanner and ProviderSession reset owners are reused without a second fact, History, renderer or model-summary path. Candidate composition is read-only; only the exact durably replayed Compaction reaches AgentLoop state.
- API/dependency/module review: no crate, Cargo feature, Port, Event, Config, Prompt, wire/schema version, Provider branch, Hook/registry, model call or externally public test surface was added. New composition and pressure types are private or `pub(crate)`/`pub(super)` with current production consumers; `capsule_for_experiment` is widened only to `pub(crate)` for the production Adapter. The 391-line `research_projection` module triggered split review and remains one cohesive read-only Project Research plus Session Receipt projection boundary; splitting its shared strict replay/reference catalog now would add forwarding surface without a second semantic owner.
- Removed/deferred: the repair removed the Adapter-local target/version fallback and every unfiltered Evidence-closure path. Automatic Compaction, model-generated summaries, line editing, TUI/Web, M5-S2 CI/release work, Git/Actions/Live and remote sealing remain outside this completed Slice.
- External effects: no real credential resolution, Provider Live request, Codex login/config access, Git commit/push/PR, Actions run, Live Test, tag, release or Web/TUI work occurred. The read-only upstream reference remains clean at `868c9edb0da913a5fc699a71664e65f44f6058b0`.

## F2a Slice Start

- Objective: close the frozen MVP contract for formal CLI image input without changing Architecture v1 through v1.6. `chat` gains one pending `/image <path>` input that enters the existing auditable Direct or Secondary Vision path before the next Primary Turn.
- Existing foundation: `view_image` already owns PNG/JPEG validation, deterministic image budget, Artifact/Blob/Lineage and exact result selection; `AgentModelProfile` owns pure capability routing; `VisionSessionService` owns route/audit/Observation facts; `InitialImagePrimaryContext` and the fixed `view_image` follow-up stage already prevent silent omission or duplicate delivery; the one Responses Adapter already owns real `input_image` encoding.
- Missing production composition: the CLI currently calls only the text `run_turn_audited_with_stages` path, does not bind Project image resolution into `ProviderComposition`, and provides neither initial-image nor model-`view_image` follow-up stages. F2a connects those existing owners rather than introducing a new model, Adapter, workflow or image store.
- Scope Lock: only CLI Chat composition, Project-aware Provider composition, the fixed AgentLoop Vision/stage entry, the existing Research projection's latest-TargetSnapshot read semantics and one minimal crate-private typed `view_image` input may change. No dependency, schema, Provider/Wire binding, Config field, vendor branch, general attachment manager, Hook/Stage registry, Web/TUI, second History or second AgentLoop is allowed.
- Pending-input rule: `/image <path>` binds to the most recently recorded authoritative TargetSnapshot, executes and records the existing `view_image` Receipt, and queues exactly one validated selection for the next non-command Turn. Missing TargetSnapshot, a second pending image, invalid image or unavailable Vision route fails explicitly. Pending process state is never reconstructed or silently replayed on Resume.
- Acceptance: offline Direct/Secondary/disabled/error/follow-up/Resume/Project isolation and durable audit tests; independent Terra xhigh cross-boundary audit after focused and affected tests; repair; complete locked Cargo/Lean Gate; dual-Provider and single-multimodal explicit-config DeepSeek Live acceptance; Lean Review and `[skip ci]` private PR remote seal. GitHub Actions remain paused.
- Baseline: `cli_chat_loopback`, `provider_composition`, `responses_http_adapter` and `vision_flow` pass 46/46 default-enabled tests; the one explicit Primary Live Smoke remains ignored. No credential or public request was used for this baseline.

## F2a Completion

- State: `COMPLETE_REMOTE_SEALED` through private `[skip ci]` PR `#62`; this status record is part of the exact merge head. M5-S1 is only `READY_NOT_AUTHORIZED`.
- Production result: formal `chat` accepts one `/image <path>` pending input bound to the latest authoritative TargetSnapshot, executes the existing `view_image` Tool with a real Receipt, and consumes it on the next non-command Turn. Initial operator images and model-originated `view_image` results share the same fixed Direct-or-Secondary composition and one AgentLoop.
- Composition and concurrency: Config v1 is strictly decoded and validated once before Project discovery, then the one Provider Composition Root binds the Project-isolated image resolver to Primary and optional Vision Responses Adapters. Secondary execution commits Route, releases ProjectWriter, commits RequestAudit under a separate short Writer, performs the remote request without a Writer, then commits exactly one Observation or Failure under a final short Writer.
- Cancellation and Resume: SIGINT during initial Secondary Vision cancels and drains the scoped request, persists exactly one cancelled Failure, starts zero Primary requests and does not reconstruct or replay a pending image on Resume. Model `view_image` follow-up cancellation returns a normal cancelled Turn after preserving the completed Tool Receipt.
- Live-discovered repair: the first dual-Provider attempt reached `deepseek-v4-flash-vision-exp` but correctly rejected a region containing the unsupported `description` field. The provider-neutral fixed Vision instruction now states the exact five-field region contract without weakening `VisionRegion`, adding a vendor branch or changing persisted schema; focused tests and an independent Terra re-audit cover the actual ModelRequest instruction.
- Independent audit: Terra xhigh first identified the ProjectWriter-across-await and initial-Vision SIGINT P1 issues; both gained permanent regressions and were repaired. The final full review and the later prompt-contract increment review each returned `APPROVE` with no P0/P1/P2.
- Complete local Gate: PASS on Rust `1.93.1`; `cargo fmt --all -- --check`, denied-warning locked Clippy, all-target/all-feature locked tests and locked build exited `0`. The suite discovers 385 tests, passes 383 by default and keeps two explicit Provider Live Smokes ignored.
- Lean Review: PASS; no dependency, feature, public `pub` API, Event/Research/Artifact/Config schema, Provider/Wire binding, vendor/model/URL branch, second History, second AgentLoop, attachment manager, Hook registry, Web/TUI, Codex dependency or test-only production surface was added. New cross-module APIs are `pub(crate)` with current production consumers; `src/` has no inline tests and `codex/` is clean.
- Dual-Provider Live: PASS through the user-owned explicit Config v1. Formal Primary Chat first called `research_update` and persisted the harmless TargetSnapshot; `/image` recorded its Artifact/Receipt; one `deepseek_vision` RequestAudit produced one strict VisionObservation; the text Primary then called `read_file` successfully and returned `F2A_DUAL_VISION_TOOL_LOOP_PASS`. Five audited Full requests, three successful Receipts, one Route and one Observation were present; there was no fallback.
- Single-multimodal Primary Live: PASS through a temporary explicit Config v1 derived without changing the user config. `deepseek-v4-flash-vision-exp` was the sole `primary_direct` Primary and no Vision Provider was configured. Formal Chat persisted the TargetSnapshot, delivered the image directly, called `read_file` successfully and returned `F2A_PRIMARY_DIRECT_TOOL_LOOP_PASS`. Four audited Full requests all used the Primary profile; one Direct Route and zero Secondary RequestAudit/Observation/Failure facts prove route exclusivity.
- Hygiene: all three isolated harmless Live projects, including the temporary single-Primary Config copy, were deleted after verification. No secret entered Git, Project facts, RequestAudit, output or shell profiles; no GitHub Actions run occurred and no Codex credential/login/session data was read or modified.
- Git: implementation commit `bf814f3` and private PR `#62` form the F2a delivery. Both branch commits use `[skip ci]`; the branch and PR status-check lists are empty, so the operator Actions pause was preserved.

## Latest Local Design Update

- Date: `2026-08-30`.
- Scope: Architecture v1.5/v1.6 adds only completed Responses reasoning continuation and the real-protocol correction for a redundant encrypted sidecar; no new Provider binding, configuration field, dependency, fact store or vendor branch exists.
- Delivery boundary: `ModelInputIR` v3 represents plaintext completed reasoning only in provider-visible delivery. Conversation/Session History rejects it; Resume, Compaction and Primary replacement clear it.
- Protocol boundary: the one Responses Adapter replays `reasoning -> function_call -> function_call_output`. Plaintext plus opaque encrypted sidecar uses the plaintext semantic item and keeps the sidecar Adapter-local; encrypted-only and unknown content fail explicitly.
- Completion: F1e passed the complete local Cargo/Lean Gate and a configuration-driven DeepSeek Full-only `read_file` Live Tool Loop, then merged through PR `#48` as `2f6a9e5`. Remote run `33295806720` executed zero Steps because GitHub blocked it for billing/spending-limit reasons; the user explicitly authorized merging this PR without a rerun. M4-S3c subsequently completed its separately authorized local implementation, Gate and Live Test as recorded below.

## Prior Local Design Update

- Date: `2026-08-29`.
- Scope: narrow Architecture v1.4 correction over frozen v1.3; no Rust source, test, Cargo dependency, Provider config, credential, research data or `SEALED_MANIFEST.md` was changed.
- Tool failure boundary: new `ToolFailure` has only `pre_execution`/`execution`; post-execution Receipt/History/RequestAudit persistence failures are App/AgentLoop errors and cannot rewrite a successful ToolResult or fabricate a Receipt.
- Accuracy boundary: reports use only durable Trace signals; unmatched Replay termination and Observation/Replay/Vision promotion stay Deferred without typed durable signals, never fake zero, and Deferred activation requires adjacent RequestAudit ToolSpec transition proof.
- Sampling authority: one immutable SamplingStepPlan binds Context, model/capabilities, exact ToolSpecs, RequestAudit and dispatch to one ResolvedToolPlan. Deferred activation affects only later Steps in the current Turn and clears at Turn/Resume boundaries.
- Prompt/Context: `generic-v1` is the only renderer; ContextSlot controls order while Origin/Authority/Retention/Freshness remain orthogonal and later enter a validated ContextDeliveryMap sidecar. Provider DeliveryItemId remains Adapter-local.
- Evidence continuity: typed EvidenceLink relations and Session Summary v2 EvidenceSpine reference authoritative Research/Artifact/Receipt facts; bounded narrative remains lossy and non-authoritative.
- Research optimization: fixed identifier → fielded BM25 → CJK/ASCII n-gram → RRF → authority/freshness retrieval and evidence-gap/NextAction guidance remain deterministic, local, explainable, Shadow-first and non-evidence.
- Planned sequence: M4-S3a1 immutable sampling foundation; M4-S3a2 Research Control Plane; M4-S3b/c/d as already bounded; M4-S3e1/e2 evidence/provenance; M4-S3f retrieval; M4-S3g guidance; then separately authorized M4-S4 long-flow acceptance.
- Authorization: the user separately authorized remote submission of the completed M4-S3b checkpoint. Commit `892edaa` and its sole push CI `33290961921` are complete; this `[skip ci]` status correction consumes that authority. M4-S3c, further Git/PR/Actions, architecture remote seal and Live Provider requests require new authorization.

## Last Remote Delivery

- Delivery: `F1e` Responses Reasoning Continuation.
- Base commit: `c52fcbe`.
- Scope: provider-visible completed plaintext reasoning, exact Full replay ordering, Conversation/Session isolation, RequestAudit v2/v3 compatibility and redundant encrypted-sidecar handling through the existing Responses Adapter and one AgentLoop.
- Result: `COMPLETE_REMOTE_SEALED`; the complete local Cargo/Lean Gate passes 313 discovered tests with 311 default-enabled tests passing and two Live Smokes ignored. The selected configuration-driven DeepSeek Full-only Tool Loop also passed.
- Remote Gate waiver: PR-head run `33295806720` ended before executing any Step because GitHub reported an account payment or Actions spending-limit block. On 2026-08-30 the user explicitly authorized merging PR `#48` without rerunning Actions and pausing subsequent Actions until quota recovery. This is an operator waiver, not a passing CI result.
- Lean result: one Responses encoder/decoder, AgentLoop, ConversationHistory and Config Composition Root remain; no dependency, Provider branch, configuration field, second History, second AgentLoop or test-only public API was added.
- Git: implementation commit `5bf760d`, PR `#48` and `[skip ci]` Squash Merge `2f6a9e5` form the implementation seal. No tag, release or architecture-manifest seal occurred.

## Prior Remote Delivery

- Delivery: integrated `F1a` through `F1d`.
- Base commit: `08e9025`.
- Scope: strict Provider Config v1 and one Composition Root; one protocol-first Responses HTTP/SSE Adapter; first-version CLI Chat through the existing AgentLoop and durable Project/Session/RequestAudit/Tool authorities; optional local inline `api_key` or referenced `credential_ref` per Provider.
- Result: `COMPLETE_REMOTE_SEALED`; the local Gate and exact PR head both pass formatting, denied-warning Clippy, all default-enabled tests and locked build. The final suite discovers 312 tests, passes 310 by default and keeps two explicit Live Smokes ignored.
- Lean result: one package, binary, AgentLoop, Provider Composition Root and Responses wire owner remain. No new crate, persisted fact schema, Provider router/registry, vendor Adapter, Codex dependency or real credential entered the delivery.
- Live evidence: the separately authorized synthetic DeepSeek Responses smoke and first-version Chat both passed before publication; F1d added no new Live request. Codex login state remained untouched.
- Git: implementation commit `98e1094`, PR `#46`, sole PR-head CI `33182963979` and `[skip ci]` Squash Merge `6eb4959` form the implementation seal without a duplicate main CI.

## Prior Remotely Sealed Milestone Slice

- Slice: `M4-S2`
- Delivery branch: `agent/m4-s2-trace-replay-eval`
- Base commit: `fea1433`
- Scope: one strict reference-only Trace fixture, fixed Replay Model/Tool Adapters, Mechanical Replay, Context Recompile and explicitly invoked Shadow Model Eval over the existing one AgentLoop and fact authorities.
- Source behavior: `TraceFixtureManifest` binds `trace_id`, exact source Session, inclusive Event range, full-stream BLAKE3 hash, Target/Artifact references, ordered RequestAudit/History samples, source-call-ordered Receipt references and the fixed M4-S2 invariant set. The supplied Session projection must equal a fresh reduction of the complete source Event stream; required Artifact bytes are reread and verified by exact SHA-256 Blob ID and size.
- Replay behavior: source requests are Full semantic samples. Replay narrows an incremental-capable source mapping to a truthful Full-only effective capability, consumes only exact recorded Model/Tool steps, honors current fixed Tool exposure and terminates `UNMATCHED_TOOL_CALL` before any later Provider or Tool action. Complete source Receipts remain distinct from derived replay Receipts, so historical Artifact/cache/file-mutation claims are retained as provenance but never forged as current execution effects.
- Eval behavior: `EvalSource` binds every result to Trace ID, source Session and source-stream hash; Context Recompile additionally binds one exact RequestAudit Event. Mechanical/Context/Shadow outputs have the closed `NonEvidence` state and no Event, Research, Memory, Artifact or Project writer.
- Fixture behavior: `tests/fixtures/m4_trace/` contains one controlled full-request Session plus immutable RequestAudit Blobs. Per-file SHA-256 and a fixed `suite.json` anchor detect drift; Replay performs zero real Provider/Tool actions.
- Result: `COMPLETE`; the full Cargo/Lean Gate passes 288 discovered tests, including two ignored opt-in OpenAI Live smokes, with all 286 default-enabled tests passing. Main review closed exact source identity, complete Receipt provenance, Blob validation, exposure, Full-only capability, frozen field naming and multi-Tool ordering; final Luna xhigh review returned `APPROVE` with no unresolved P0/P1. The exact implementation head passed its sole remote macOS Gate and is merged.
- Dependencies and schemas: no Cargo dependency, feature, persisted Event/Blob/Cache/configuration, Provider DTO, credential source or remote transport was added. New production surface is the non-persisted Trace/Eval schema, fixed Replay Adapters/composition and one terminal Tool Port error; existing `sha2`, Session/Research reducers, RequestAudit, ArtifactStore, ContextPlanner and AgentLoop are reused.
- Git: implementation commit `d99f395`, PR `#44`, sole PR-head CI `33165972124`, `[skip ci]` Squash Merge `2afe900` and separate `[skip ci]` status PR `#45` complete the remote seal.
- Live Test: `NOT_RUN_PROVIDER_BINDING_NOT_IMPLEMENTED`; the supplied dedicated credential has no configuration-driven Provider binding in the current production runtime. It was not read, persisted, logged or aliased to the OpenAI credential reference, and no Codex login state was accessed.

## Current Authority

1. `AGENTS.md`
2. This file
3. `task.md`
4. `docs/development/IMPLEMENTATION_PLAN.md`
5. `docs/development/LEAN_CODE_GOVERNANCE.md`
6. `docs/architecture/CYBER_AGENT_ARCHITECTURE_V1_6.md`
7. `docs/architecture/CYBER_AGENT_ARCHITECTURE_V1_5.md`
8. `docs/architecture/CYBER_AGENT_ARCHITECTURE_V1_4.md`
9. `docs/architecture/CYBER_AGENT_ARCHITECTURE_V1_3.md`
10. `docs/architecture/CYBER_AGENT_ARCHITECTURE_V1_2.md`
11. `docs/architecture/CYBER_AGENT_ARCHITECTURE_V1_1.md`
12. `docs/architecture/CYBER_AGENT_ARCHITECTURE_V1.md`
13. `docs/adr/ADR-0016-redundant-encrypted-reasoning-sidecar.md`
14. `docs/adr/ADR-0015-responses-reasoning-continuation.md`
15. `docs/adr/ADR-0014-narrow-tool-failure-and-accuracy-observability.md`
16. `docs/adr/ADR-0010-immutable-sampling-step-and-tool-plan.md`
17. `docs/adr/ADR-0011-context-provenance-evidence-relations-and-compaction.md`
18. `docs/adr/ADR-0012-deterministic-security-retrieval-and-evidence-gap-guidance.md`
19. `docs/adr/ADR-0013-versioned-prompt-renderer-boundary.md`
20. `docs/adr/ADR-0009-stable-research-persona-domain-profile-and-phase.md`
21. `docs/adr/ADR-0002-capability-driven-primary-vision-models.md`
22. `docs/adr/ADR-0003-minimal-openai-http-sse-stack.md`
23. `docs/adr/ADR-0004-sha256-artifact-blob-identity.md`
24. `docs/adr/ADR-0005-minimal-local-image-stack.md`
25. `docs/adr/ADR-0006-single-provider-composition-root.md`
26. `docs/adr/ADR-0007-protocol-first-responses-adapter.md`
27. `docs/adr/ADR-0008-local-inline-provider-credentials.md`

## Completed

- Codex local architecture and prompt-flow assessment.
- Dedicated single-user vulnerability research Agent architecture and P0/P1/P2 classification.
- Architecture v1 frozen baseline and v1.1 capability-driven Primary plus optional Vision composition.
- Architecture v1.2 locally frozen Research Control Plane: one stable persona, orthogonal DomainProfile/ResearchPhase, phase-aware Tool exposure and measurable Tool-accuracy contracts; production and remote seal remain unstarted.
- Architecture v1.3 locally frozen immutable Sampling/Tool plan, versioned Prompt renderer, Context provenance, typed evidence/Compaction and deterministic Shadow-first retrieval/guidance contracts; production and remote seal remain unstarted.
- Architecture v1.4 locally frozen narrow ToolFailure and durable accuracy-observability correction; M4-S3b implements and gates that boundary locally without a remote seal.
- Architecture v1.5/v1.6 locally frozen provider-visible completed reasoning, Full replay and redundant encrypted-sidecar boundaries.
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
- M2-S4 audited Project Memory, two-level Project Fingerprints and rebuildable Snapshot/File Manifest/Tool Capability caches.
- M2-S5 auditable atomic file writes, a constrained owned Patch Adapter and a project/session-bound deterministic read-only Tool Result Cache.
- M3-S1 budgeted Context planning, explainable decisions, logical manifests, Fragment Cache and AgentLoop Context compilation.
- M3-S2 bounded World/Project State Diff delivery, fixed Tool Exposure/search, Mode/Profile composition and pure Vision routing.
- M3-S3a strict History normalization, versioned Session facts/replay, append-before-apply persistence and Resume-first Full delivery.
- M3-S3b bounded typed SessionSummary, strict Compaction facts, pair-safe retained History and Compaction-first Full delivery.
- M3-S3c strict RequestAudit facts, deduplicated exact semantic component Blobs, pre-stream durable commit and historical reconstruction.
- M3-S3d read-only prompt, Snapshot and local metrics CLI inspection over existing auditable facts.
- M3-S4 auditable local images, fixed `view_image`, Direct/Secondary Vision composition, strict Vision Session facts and zero-request Mechanical Replay.
- M4-S1 fixed vulnerability-research Prompt Profiles, Turn-boundary AgentLoop selection and deterministic tests-only controlled offline cases.
- M4-S2 strict Trace fixtures, full-only Replay Model/Tool Adapters, Mechanical Replay, Context Recompile and isolated non-evidence Shadow Eval.
- M4-S3a1 immutable SamplingStepPlan/ResolvedToolPlan binding, current-Turn Deferred activation, fixed `generic-v1` renderer identity and behavior-preserving ContextSlot/ContextOrigin migration.
- M4-S3a2 fixed Base Contract, orthogonal DomainProfile/ResearchPhase composition, Turn-boundary durable phase selection and bounded Tool exposure.
- M4-S3b canonical Tool guidance, precise schemas, narrow structured ToolFailure and durable-Trace-only non-evidence Tool accuracy reports.
- M4-S3c provider-neutral bounded `http_request` and deterministic `inspect_environment` through the existing Tool/Artifact/Receipt authorities (remotely sealed through PR `#50` and Squash Merge `1b7b032`).
- M4-S3d strict AndroidManifest Observation ingestion and deterministic two-Snapshot TargetDelta with complete source provenance, exposed only through read-only local inspection commands (`COMPLETE_LOCAL`; remote publication unauthorized).
- M4-S3e1 additive typed EvidenceLink, conservative v1-to-v2 Research Upcast, real Session Receipt/target applicability validation and pure evidence-gap projection (`COMPLETE_LOCAL`; remote publication unauthorized).
- F1a strict Provider Config v1, one Provider Composition Root, shared credential source and configuration-driven generic Live Smokes (remotely sealed in integrated PR `#46`).
- F1b one protocol-first Responses HTTP/SSE Adapter, provider-neutral reasoning events and configuration-only conforming-service selection (remotely sealed in integrated PR `#46`).
- F1c first-version CLI Chat through the existing AgentLoop, durable Session/RequestAudit/Tool facts and one synthetic DeepSeek Live Chat (remotely sealed in integrated PR `#46`).
- F1d compatible Config v1 inline `api_key` or referenced `credential_ref` selection through the existing Composition Root and Adapter boundary (remotely sealed in integrated PR `#46`).
- F1e provider-visible Responses reasoning continuation, ModelInputIR v2/v3 compatibility and one DeepSeek Full-only Live Tool Loop (remotely sealed through PR `#48` and Squash Merge `2f6a9e5`).

## Prior Fix Slice

- Slice: `F1a`
- State: `COMPLETE_REMOTE_SEALED` through integrated PR `#46`.
- Scope: add the one strict Provider Config v1 and one Composition Root; make `chat`, `doctor` and both ignored Live Smokes consume that path; keep credentials lazy and explicit; remove generic runtime endpoint/model hard-coding.
- Protocol boundary: this Slice keeps the existing OpenAI Responses binding but makes selection configuration-driven. A later, separately authorized binding Slice may extract shared Responses wire code when the second real Provider proves the exact common contract; it must not duplicate request encoding, HTTP/SSE transport, response decoding or the ModelProvider Contract Suite.
- Scope budget: no DeepSeek binding, no real Live Test, no first-version Chat loop, no Provider router/registry/plugin, no fallback, no new dependency, no Event or fact store, and no M4-S3 Observation/TargetDelta work.
- Git/remote: the Slice later shipped with F1b/F1c/F1d through integrated PR `#46` and Squash Merge `6eb4959`.

## Prior Completed Fix Slice

- Slice: `F1c`
- State: `COMPLETE_REMOTE_SEALED` through integrated PR `#46`.
- Delivered scope: `chat --config <path>` now reaches the one Composition Root and one existing AgentLoop for an initialized local Project. It supports fixed Prompt Profiles, durable new/resumed Sessions, RequestAudit, FullAccess Tools, ordered text deltas, Ctrl-C cancellation and explicit exit without a second loop, Provider construction or fact authority.
- Durable safety: one project-scoped advisory Chat runtime lease prevents concurrent Chat processes while ordinary Writer locks remain short-lived. AgentLoop appends every real Tool Result and verifies complete Tool pairing before invoking receipt persistence; a receipt-stage failure therefore cannot replace successful results with synthetic cancellations.
- Verification: loopback end-to-end tests cover new/resumed/profiled Chat, audit/history/receipt persistence, lease isolation, cancellation, continuation rejection/Full fallback and PID-bearing Session identities. The final locked Cargo/Lean Gate passes with 308 discovered tests, 306 default-enabled tests passing and two opt-in Live Smokes ignored.
- Boundary: no TUI/Web, general command framework, model Router/Registry/fallback, new fact store, M4-S3/M4-S4 behavior, Git operation or Codex credential/login access was added or run. One separately authorized synthetic DeepSeek Live Chat used the completed generic path.

## Prior Completed Fix Slice

- Slice: `F1d`
- State: `COMPLETE_REMOTE_SEALED` through integrated PR `#46`.
- Delivered scope: each Config v1 Provider selects exactly one local inline `api_key` or existing `credential_ref`; both route through the one Provider Composition Root and existing Adapter credential interface.
- Compatibility: existing `credential_ref` configurations remain valid. Inline credentials are explicit local configuration data, do not create a Provider binding and are not written to Session, RequestAudit, Artifact, logs or diagnostic output.
- Scope budget: no Keychain, encrypted vault, config discovery, credential manager, schema database, Provider router, Codex credential source, Live request, Git operation or M4-S3/M4-S4 work.
- Verification: inline/reference composition, exact-one validation, lazy Provider use, CLI `doctor` redaction, loopback Chat Authorization and fact/output non-disclosure pass with the complete locked Cargo/Lean Gate.

## Last Completed Fix Slice

- Slice: `F1e`
- State: `COMPLETE_REMOTE_SEALED` through PR `#48` and Squash Merge `2f6a9e5`; the required remote Gate has an explicit one-time operator waiver rather than a PASS result.
- Delivered scope: completed Responses `reasoning_text` is retained only in provider-visible delivery and replayed with the matching Function Call and Tool Result in exact order. Conversation/Session/Research/Memory and user output never receive reasoning.
- Compatibility: new requests use ModelInputIR v3; historical RequestAudit v2 remains readable and real Replay request matching promotes v2 to v3 without changing the stored audit. Full RequestAudit differences retain the established state-diff reasons; Prompt/Tool-plan changes accept only an exact reasoning replay of the logical input.
- Protocol: one generic Responses Adapter accepts plaintext reasoning with an optional redundant encrypted sidecar, rejects encrypted-only/unknown content, and contains no DeepSeek, Provider, model, URL or credential branch.
- Verification: complete Gate passes on Rust `1.93.1` with 313 discovered tests, 311 default-enabled tests passing and two ignored opt-in Live Smokes; Lean scans pass. A selected DeepSeek `deepseek-v4-flash` Full-only Live Chat called `read_file`, replayed reasoning/call/result and returned the fixed synthetic file content.
- Audit: the bounded independent Luna audit first found the Context-candidate reasoning-loss P1; the permanent over-budget regression and explicit failure fixed it. Its repair review returned `APPROVE` with no P0/P1. Main cross-boundary review additionally fixed Deferred `tool_search`/ToolsChanged replay and added its regression. Later optional addendum audit threads timed out and were terminated; none was counted as approval.
- Git: implementation commit `5bf760d` was Squash Merged through PR `#48` as `2f6a9e5` with `[skip ci]`. Run `33295806720` executed no Steps because of the GitHub billing/spending-limit block; the user explicitly waived its rerun for this merge. No tag, release or architecture-manifest seal occurred.

### F1e Lean Review

- Semantic owners: `domain/model` owns provider-neutral completed reasoning and the exact reasoning-replay relation; `adapters/responses` owns all wire encoding/decoding and encrypted-sidecar handling; the one AgentLoop owns atomic completion, provider delivery and fallback; ConversationHistory remains the sole durable pairing/history owner.
- Reuse: existing Config v1 Composition Root, Responses transport, ModelStreaming completion, ProviderSession Baseline, ContextPlanner/state-diff, RequestAudit, Replay, Tool Registry and Session owners are reused. No second path or copied vendor Adapter exists.
- API/dependencies: ModelInputIR advances additively from v2 to v3. New surface defaults to crate-private where cross-module production consumers permit; no Cargo dependency, feature, Event, Config, Provider binding, service, Registry, Hook or public test-only projection was added.
- Removed/deferred: partial reasoning deltas, opaque encrypted values and Provider item IDs are not retained. Encrypted-only continuation, general model routing, automatic capability probing and reconstruction of ephemeral provider delivery after Resume remain explicitly unsupported/deferred.
- Tests: permanent top-level regressions cover wire order, malformed/unknown/encrypted-only content, redundant sidecar, Full/incremental/fallback, budget failure, `tool_search` ToolSpecs change, History isolation, RequestAudit v2/v3 and real Replay request matching. Production `src/` has no inline tests.
- Governance debt: none blocking. Empty/missing reasoning `content` remains a documented no-op when no encrypted continuation exists. Timed-out addendum audits are recorded as no conclusion, never `APPROVE`.

## Last Completed Slice

- Slice: `M4-S3e2`
- State: `COMPLETE_REMOTE_SEALED`; independent audit, complete local Gate, one configuration-driven synthetic Live Tool Loop and Lean Review pass, then PR `#53` Squash Merge `1f382ee` completed the remote seal without an Actions run.
- Scope lock: complete Context provenance through deterministic Fragment/Subitem identities and one validated ContextDeliveryMap; persist it additively through RequestAudit; replace new Compaction summaries with a typed EvidenceSpine plus bounded non-authoritative Narrative while retaining strict legacy v1 readability.
- Intended production touch points: the existing ContextPlanner/generic-v1 renderer, SamplingStepPlan and RequestAudit owner; the existing Session Compaction/Resume path; the existing Research/Artifact/Receipt reducers only through typed reference validation.
- Boundary: no second renderer, Provider DeliveryItemId in Domain, copied Research claim strings, second fact store, autonomous Compaction workflow, retrieval/guidance work, new dependency, Actions run, tag, release or architecture seal.
- Acceptance: exact instruction/content provenance coverage including truncation and ArtifactRef transforms; RequestAudit v1/v2 compatibility plus v3 map reconstruction; Summary v1 readability and Summary v2 missing/cross-Project/stale/type-mismatch rejection; complete local Cargo/Lean Gate after independent audit.

### M4-S3e2 Pre-Audit Checkpoint

- State remains `IN_PROGRESS`; this checkpoint is not an audit approval, complete Gate, Lean Review or Slice completion.
- Basic checks pass at the current worktree: `cargo fmt --all -- --check`, `cargo check --locked --all-targets --all-features` and `git diff --check`.
- The five bounded Slice suites pass 38/38 tests: `context_planner`, `m4_sampling_plan`, `request_audit`, `session_compaction` and `vision_flow`.
- The thirteen affected suites pass 159 default-enabled tests with one explicit Live Smoke ignored: `cli_chat_loopback`, `m3_world_state_mode`, `m4_research_control`, `tool_loop_integration`, `trace_replay`, `session_resume`, `inspection_cli`, `inspection_audited_cli`, `project_event_store`, `research_evidence`, `provider_baseline`, `m3_multimodal` and `event_upcasting`.
- Main-thread pre-review found no blocking ownership drift: one `generic-v1` renderer and delivery-map owner remain; Full, Incremental, Provider fallback, state-diff and scoped Vision paths carry validated maps; Summary v2 reuses authoritative Research/Artifact/Receipt projections and CLI Resume validates before its durable Resume Event.
- No dependency, Provider/configuration path, Codex port, inline `src/` test, Actions run, Live request, commit, push or remote publication was added. Independent read-only audit remains separately unauthorized, so the complete Gate and Lean Review have not started.

### M4-S3e2 Completion and Lean Review

- Semantic owners: `domain/context/planner` remains the sole `generic-v1` ordering/render/budget/transform and `ContextDeliveryMap` owner. It now assigns deterministic Fragment/Subitem identities, validates orthogonal provenance, covers final instruction spans and ContentItem positions, and binds canonical Artifact-reference transforms. `domain/request_audit` alone persists and reconstructs additive v3 delivery maps while v1/v2 remain readable. `domain/session/compaction` alone owns reference-only Summary v2 `EvidenceSpine` plus bounded non-authoritative `NarrativeSummary`; authoritative Research, Artifact and Receipt projections remain its validation inputs rather than copied facts.
- Reuse: the existing one AgentLoop, SamplingStepPlan, ContextPlanner, Prompt renderer, RequestAudit Blob layout, Session reducer/Resume path, Research/Artifact/Receipt projections, Provider composition and scoped Vision flow are reused. Full, Incremental, provider-rejection fallback, state-diff and Vision requests carry the same validated delivery-map semantics. `ContentFragmentInput` is one private current-consumer parameter object shared by full and incremental History provenance construction.
- API and dependencies: no crate, Cargo feature, Provider/config field, credential source, Event/fact store, Port, renderer Trait/Registry, Hook or Codex port was added. New production surface is limited to the typed Context provenance/map, RequestAudit v3 and Summary v2 contracts with current App/Domain consumers; visibility remains crate-private where the production boundary permits. No test-only public projection or future placeholder type remains.
- Removed/deferred: Summary v2 removes duplicated authoritative `verified_facts`/hypothesis/path strings in favor of validated references; legacy Summary v1 remains strictly readable without semantic promotion. Retrieval and evidence-gap guidance remain M4-S3f/g Shadow work; Provider DeliveryItemId, autonomous Compaction, second renderer, new database and model-visible Shadow output remain deferred. No experimental inline test or duplicate transform validator remains.
- Independent audit: Terra xhigh first returned `CHANGES_REQUIRED` for incomplete scoped-Vision source/route provenance and an `ArtifactReference` transform that could be relabeled without checking the actual delivered item. Both were fixed with permanent ContextPlanner, Vision and RequestAudit-reconstruction regressions. Its repair review confirmed those P1 closed but found two denied-warning Clippy failures; redundant RequestAudit capabilities now come from immutable `SamplingStepPlan`, full/incremental fragment inputs share one private parameter object, and test closure borrows end by lexical scope. The final bounded re-audit returned `APPROVE` with no P0/P1.
- Tests and Gate: the five Slice suites pass 38/38; the thirteen affected suites pass 159 default-enabled tests with one Live Smoke ignored. The complete pinned-toolchain Gate passes `cargo fmt --all -- --check`, denied-warning Clippy, `cargo test --locked --all-targets --all-features`, `cargo build --locked`, inline-test scan and `git diff --check`. The suite discovers 368 tests: 366 default-enabled pass and two opt-in Live Smokes remain ignored.
- Live: the explicit user-owned `/Users/f0und/.config/cyber-agent/deepseek-live.toml` selects `deepseek-v4-flash` through the one `http_sse + responses` Composition Root with Full-only text capabilities and Vision disabled. `doctor` returned ready; the fixed synthetic Shadow Tool Loop passed 1/1, invoked only its replay Tool and remained derived non-evidence. No Codex credential/login state or real target was used, no secret entered Git or durable Project facts, and no Actions run occurred.
- Governance debt: none blocking. The prerequisite remote seal is satisfied by PR `#53`/`1f382ee`; every subsequent Slice retains its own independent audit, Gate, Live decision, Lean Review and Git/PR seal.

## Last Completed Slice

- Slice: `M4-S3f` deterministic security retrieval Shadow.
- State: `COMPLETE_REMOTE_SEALED`; PR `#55` was Squash Merged as `02bdbaa` without GitHub Actions under the active operator pause.
- Scope lock: one fixed versioned pipeline owns eligibility/Target filtering, exact vulnerability identifiers, fielded BM25, bounded CJK/ASCII character n-grams, RRF, explicit authority/freshness tiers and stable-ID tie-break. It reads bounded Project Memory, Research, Artifact/Observation metadata and explicitly selected historical summaries; it remains local, Shadow-only, explainable, derived non-evidence and invisible to model Context.
- Data and dependency budget: begin with bounded scans and a deletable in-memory derived index. No SQLite/FTS dependency, vector/graph database, remote embedding/reranker/judge, selector Registry, Provider/Tool call, persistent fact, autonomous action or Context promotion is allowed in this Slice.
- Controlled quality contract: exact security identifiers must achieve `Recall@10 = 1.00`; the aggregate controlled suite targets `Recall@10 >= 0.90` and `Precision@10 >= 0.60`, with zero cross-Project or incompatible-current-Target candidates. Promotion remains forbidden in M4-S3f; a candidate promotion later additionally requires at least 20% lower repeated-read or estimated Context-token cost without recall regression. Missing these benefit thresholds keeps the implementation Shadow or triggers removal, but never permits metric inflation.
- Latency rule: SQLite FTS remains deferred unless a representative bounded corpus first proves in-memory p95 query latency above 100 ms and a separately reviewed rebuildable prototype demonstrates at least a 2x p95 improvement without changing rank/reason output. No such evidence currently exists.
- Required fixtures: Source/Web/Mobile/Binary cases cover CVE/CWE, URL parts, package/bundle/class/method/symbol, Android permission/intent/component, binary symbol/hash, CJK and typo tolerance; every case declares acceptable reference sets, Target applicability and stable expected reasons.
- Gate:专项 tests, affected tests, independent bounded audit, fixes, complete locked Cargo/Lean Gate and Lean Review. A real Provider Live request is not required for a pure Shadow algorithm; any optional Live Eval remains separately explicit and cannot replace controlled metrics.

### M4-S3f Completion and Lean Review

- State: `COMPLETE_REMOTE_SEALED`; commit `e11d592` was published through private PR `#55` and Squash Merged as `02bdbaa`. M4-S3g may now start from this sealed base on its own branch.
- Semantic owners: `domain/retrieval` owns the one versioned fixed eligibility, exact-identifier, fielded-BM25, bounded CJK/ASCII n-gram, RRF, authority/freshness-tier and stable-ID pipeline. It canonicalizes bounded fields, reports true pre-tier component ranks and returns at most ten results. `app/retrieval` only projects bounded existing Project Memory, Research, Artifact/Vision Observation metadata and explicitly selected historical Summary facts. `project_memory` owns the confined read-only Markdown scan, while `cli_inspection` owns the zero-write local Shadow command and its pre-replay 48-Session/48-Observation bounds.
- Reuse and API: existing Project isolation, strict Research/Session reducers, Artifact metadata projection, Summary v1/v2 readers, BLAKE3 canonical fingerprinting and the existing Context token estimator are reused. New retrieval contracts, source iterators and the current-consumer `RetrievalSources` composition parameter remain crate-private. No Port, Event, Config, Prompt, Cache format, dependency, Cargo feature, Codex port, Provider/Tool path, test-only public projection or future placeholder was added.
- Controlled quality: eleven fixed Source/Web/Mobile/Binary cases declare thirteen required Top-10 references plus exact source, eligibility, tier, tier reason and matched-field expectations. Exact-identifier and aggregate `Recall@10` are `1.00`; aggregate `Precision@10` satisfies the fixed `>= 0.60` threshold without expanding acceptable sets after output. Cross-Project facts are compositionally isolated, incompatible current Targets are excluded, CJK/typo matches expose their actual fields, repeated identical requests are byte-stable and explicitly selected historical Session order does not change fingerprints, ranks or reasons.
- Audit and fixes: the independent Terra xhigh audit first returned `CHANGES_REQUIRED` for exact-hit deletion of explanatory candidates, missing production Top-10, mislabeled post-tier RRF ranks, an unbounded Observation replay loop, a valid-long-fact panic and incomplete controlled fixture contracts. Repairs removed the hard deletion, captured RRF rank before tier rerank, bounded source projection and valid UTF-8 fields, and added permanent regressions. Repair review found and removed one extra exact-first sorting key that bypassed the frozen RRF-to-tier order; the final finite re-audit returned `APPROVE` with no P0/P1.
- Tests and Gate: the six M4-S3f tests and 28 affected inspection, Research, Compaction and multimodal tests pass. The complete Rust `1.93.1` Gate passes formatting, denied-warning Clippy, `cargo test --locked --all-targets --all-features`, `cargo build --locked`, production inline-test scan and `git diff --check`. The suite discovers 371 tests: 369 default-enabled tests pass and two explicit Live Smokes remain ignored.
- Lean size review: `domain/retrieval/mod.rs`, `normalize.rs` and `rank.rs` separate contracts, normalization and the single ranking owner; `app/retrieval.rs` keeps the independently eligible source projections together. Their 300-line review signals were inspected rather than mechanically split: further splitting would expose private ranking state or duplicate source eligibility without a second consumer. The only parameter object groups the three currently consumed derived sources and replaced an eight-argument composition call without an unused abstraction.
- Removed and deferred: the implementation removed the temporary exact-only result filter, the later exact-first hidden sort key, duplicated n-gram thresholds, `core.md` retrieval despite its always-injected Context role, a projection `expect` panic and an unused explicit-selection flag. Repeated-read reduction and Context-cost improvement remain honestly `unavailable` because no typed baseline/complete denominator exists; retrieval therefore stays Shadow and invisible to ModelInputIR. Persistent SQLite/FTS, vector/graph storage, remote embedding/reranker/judge, model-visible promotion and autonomous action remain deferred. No representative p95 evidence currently justifies a new index dependency.
- Live and remote: no Provider Live request was required or run for this pure deterministic Shadow Slice. PR `#55` was merged with `[skip ci]`; no credential, Codex login/session state, `~/.codex` or `CODEX_HOME` was read or modified, and no GitHub Actions run was triggered.

## Last Completed Slice

- Slice: `M4-S3g` deterministic evidence-gap and experiment guidance Shadow.
- State: `COMPLETE_REMOTE_SEALED`; commit `e2410d2` was published through private PR `#57` and Squash Merged as `5f63157` without GitHub Actions under the active operator pause.
- Scope lock: one separate pure projection reads only existing typed evidence gaps and existing typed `NextAction` or caller-supplied controlled Eval candidates. It applies hard invalidity and missing-prerequisite filters before one versioned deterministic factor vector: boundary importance, competing-hypothesis discrimination, expected gap closure, reproducibility, novelty, Target applicability, execution cost, mutation risk and version-drift risk.
- Explainability contract: every candidate reports bounded factor values, stable Reason Codes, exclusion reasons, action fingerprint and deterministic tie-break. Unknown remains distinct from missing; duplicate action fingerprints, cross-Snapshot candidates, incompatible Targets and unmet prerequisites cannot silently enter ranking.
- Controlled quality contract: fixed fixtures must cover unknown/missing gaps, prerequisites, duplicate fingerprints, cross-Snapshot applicability, mutation/version-drift risk and stable ties. The preferred candidate must close more declared critical gaps than a merely cheaper alternative. Gap-closure and cost metrics require explicit denominators; unavailable values remain unavailable rather than becoming zero.
- Side-effect boundary: the report remains local, Shadow-only, operator-optional, derived non-evidence and invisible to `ModelInputIR`. It cannot execute a Tool, create an Experiment, change `ResearchPhase`, update Research/Memory, promote a Finding/Observation or modify retrieval state.
- Dependency and abstraction budget: reuse the existing typed gap projection, NextAction records, Target applicability and canonical fingerprint primitives. Do not add an Event, Config, Port, Provider path, persistent index, learned ranker, model judge, autonomous experiment generator, Workflow DSL, candidate registry or future placeholder abstraction.
- Gate and Live decision: execute focused tests, affected tests, independent bounded audit, repairs, the complete locked Cargo/Lean Gate and Lean Review. A Provider Live request is not required for this pure deterministic Shadow Slice; controlled Mock/Fixture/Replay evidence is authoritative for S3g.

### M4-S3g Completion and Lean Review

- Semantic owners: `domain/guidance/mod.rs` is the one Shadow orchestration owner. `contracts.rs` owns the fixed bounded candidate/factor/report contracts, `validation.rs` owns candidate bounds plus deterministic candidate/input identity, `projection.rs` maps the two current typed sources into explainable factors and exclusions, and `rank.rs` owns the fixed nine-factor order and stable tie-break. `app/guidance` is the production App boundary; `cli_inspection` owns the read-only command and its private strict wire DTO/file bound.
- Reuse and compatibility: Guidance reads the existing typed EvidenceGap projection, NextAction lifecycle/reference facts, Research target applicability and ephemeral Receipt-aware projection. Existing Experiment and controlled action witnesses use the same `fingerprint.rs` normalization, canonical JSON, sorting and `action-` namespace owner. The attempted `ResearchRecordRef` decode tightening was removed; no durable Event, NextAction, Research schema or Upcaster changed.
- Input and ranking contract: `inspect guidance --target-snapshot <id> [--eval-candidates <path>]` accepts at most 64 KiB and 128 strictly decoded controlled candidates. A caller supplies a bounded typed action-identity preimage rather than a claimed fingerprint; the canonical lowercase fingerprint is computed locally. Invalid lifecycle, explicit invalidity, missing prerequisites, duplicate action identity, prior execution, missing/incompatible reference and cross-Snapshot candidates are excluded. Unknown applicability, prerequisite and gap state remain distinct from Missing and Unavailable; source priority is reported but never ranked.
- Controlled quality: four permanent tests prove empty input, 128/129 count behavior within the file bound, nested unknown-field rejection, candidate/reference/action-witness bounds, computed/prior/duplicate fingerprints, duplicate gap declarations, missing/unknown prerequisites and Research applicability, exact Snapshot filtering, deterministic tie order, lower mutation/version-drift risk, priority neutrality and critical-gap closure outranking lower cost. Reordered identical input is byte-stable, and the complete `.cyber-agent` tree is unchanged with zero Provider, Tool, Experiment, Research, Memory, Phase, Finding, Observation, retrieval, Event or Cache effects.
- Audit and repairs: independent Terra xhigh first returned `CHANGES_REQUIRED` for a Guidance-motivated tightening of the persistent `ResearchRecordRef` schema and caller-declared hex fingerprints that could falsely appear novel; it also required stronger bounds/risk/side-effect regressions and a review of the 933-line module. Repairs restored persistent compatibility, introduced a CLI-private strict DTO, moved action identity to a typed preimage computed by the one existing owner, expanded permanent tests and split current responsibilities into five cohesive modules without a Trait or Registry. The finite repair re-audit returned `APPROVE` with no P0/P1/P2.
- Tests and Gate: the four M4-S3g tests and 28 affected inspection, Research, retrieval and CLI tests pass. The complete Rust `1.93.1` Gate passes formatting, denied-warning Clippy, `cargo test --locked --all-targets --all-features`, `cargo build --locked`, production inline-test scan and `git diff --check`. The suite discovers 375 tests: 373 default-enabled tests pass and two explicit Live Smokes remain ignored.
- Lean size review: the original single guidance module was removed. The remaining `contracts.rs` and `projection.rs` cross the 300-line review signal but each contains one cohesive current responsibility: the complete serialized report/input contract, and the two-source typed projection sharing one gap/applicability path. Splitting either further would scatter one schema or duplicate projection state without a second consumer. `validation.rs` and `rank.rs` remain independent pure owners; no `common`, `utils`, manager, registry, future placeholder or test-only surface was added.
- API, dependencies and deferred work: all new internal surface is `pub(crate)` or narrower and has a current CLI/App/Domain production consumer. No crate, Cargo feature, Port, Event, Config, Provider/Tool path, persistent index, learned ranker, model judge, autonomous experiment generator, Workflow DSL or Codex port was added. Actual critical-gap closure and cost-effectiveness remain explicitly Unavailable until M4-S4 has typed outcome denominators; Guidance stays Shadow, model-invisible, operator-optional and non-evidence.
- Live and remote: no Provider Live request was required or run for this pure deterministic Shadow Slice. PR `#57` was merged with `[skip ci]`; no credential, Codex login/session state, `~/.codex` or `CODEX_HOME` was read or modified, and no GitHub Actions run was triggered.

## M4-S4 Completion and Lean Review

- Slice: `M4-S4` controlled long-flow vulnerability-research acceptance.
- State: `COMPLETE_REMOTE_SEALED`; acceptance began from remotely sealed `main` at status seal `ecc3af2`, was implemented on `agent/m4-s4-long-flow-acceptance`, and merged through private PR `#59` as Squash Merge `a7143ba`. The sealed S3g behavior base is `5f63157`.
- Outcome lock: complete one authorized, bounded, synthetic local vulnerability-research task through the existing CLI, one AgentLoop, one Session and one Research authority. The task must exercise durable Resume, Research Ledger, TargetSnapshot, ExperimentCapsule/Receipt/Artifact evidence, explicit DomainProfile/ResearchPhase boundaries and auditable RequestAudit/ContextDeliveryMap/Evidence Spine reconstruction without using a real third-party target.
- Offline acceptance first: all Mock/Fixture/Replay paths must prove text-primary plus scoped Vision routing, single multimodal primary without a Secondary request, `disabled`/unavailable image capability errors, Profile/Turn-boundary Session isolation, immutable SamplingStepPlan/ResolvedToolPlan binding, Deferred activation lifetime and reproducible long-flow Resume. Mechanical/Fixture paths execute no uncontrolled external action and cannot promote Observation/Replay/Vision output into evidence.
- Live matrix: each real request is opt-in and uses only an explicit Config v1 through the one Provider Composition Root. Run every currently available Cyber Agent Provider configuration independently; the existing text-only DeepSeek configuration may satisfy only the compatible text long-flow lane. A text-primary plus Vision lane and a single multimodal-primary lane require an explicit compatible Vision/multimodal Provider configuration. Missing credentials/capabilities are recorded as `UNAVAILABLE`, never inferred from model names and never fabricated as PASS.
- Measurement contract: record Provider requests, available token/usage data, Full/Incremental attempts and fallback, tool rounds, batch opportunities, repeated reads, Snapshot/Memory retrieval, duplicate Hypothesis/Experiment signals, acceptable first action, invalid-argument correction, unmatched tools, Tool Schema cost, retrieval recall/precision/cost and Guidance critical-gap closure/cost. Any missing typed denominator remains `UNAVAILABLE`; Shadow retrieval/guidance cannot be promoted in this Slice without their frozen benefit evidence.
- Change budget: M4-S4 is acceptance-first. Add only controlled fixtures, Replay manifests, tests or a minimal bug fix proven necessary by the acceptance path. Do not add a new AgentLoop, workflow runner, benchmark framework, Provider binding/router, model judge, telemetry, persistent Eval store, target-specific Tool, Event/Config field or speculative abstraction. A source change restarts focused/affected audit and the complete Gate.
- Gate and seal: run focused long-flow tests, affected tests, independent bounded audit, any required repairs, the complete locked Cargo/Lean Gate and Lean Review; then run separately authorized compatible Live lanes. GitHub Actions remain paused and every local/remote delivery uses `[skip ci]`. Stop at a truthful M4-S4 acceptance record, including explicit unavailable lanes and removal/defer decisions for optimizations without benefit.

### Acceptance record

- Controlled source: the harmless local `fixture_router.rs` has SHA-256 `e21e873e836aee939dd4379ec2fbec6c323f0c3c8c365d940aaa0679afeb64fb`; its complete manifest is independently anchored by SHA-256 `0196cd31b0d366dd4f1755708ed1874db5d6faf0e006482bf502722242bc7730`. It performs no I/O, network, process or third-party-target action.
- Authoritative chain: one CLI, AgentLoop, Session and Research stream records `TargetSnapshot -> artifact_store Artifact/Receipt -> Artifact-linked Hypothesis -> real read_file Receipt -> Experiment/ExperimentCapsule -> Summary v2 EvidenceSpine -> Compaction -> explicit Resume`. The Capsule binds the read execution and treats the previously produced Artifact as input. The Artifact-producing and read Receipt Event IDs both survive Compaction and appear in the reconstructed Resume request.
- Evidence boundary: five started Tool executions have five real Receipts; every Tool Call/Result remains paired. The Research state contains one candidate Hypothesis and one inconclusive Experiment, no Finding, and no VisionObservation. Model prose never promotes the candidate, Summary remains reference-only derived state, and Resume creates a fresh Primary ProviderSession with a Full request and no old `previous_response_id`.
- Request audit: six pre-Resume and one post-Resume RequestAudit manifests are schema v3 with `generic-v1`, the exact ResolvedToolPlan fingerprint and a validated ContextDeliveryMap. All seven reconstruct exactly from their referenced Blobs after the controlled source flow and Compaction.
- Controlled measurements: seven Model requests comprise two Full and five Incremental requests, zero fallbacks and one Compaction. One accepted local run observed a RequestAudit estimate of 131183 aggregate tokens, Snapshot state `hit` and zero Tool Cache hits; the permanent regression intentionally requires only a positive token estimate while fixing the 7/2/5/0/1 request/Compaction counts. Provider-reported usage is `UNAVAILABLE`. Five causally ordered Tool rounds execute, one source read occurs, and all five advertised calls match a dispatched Tool. General batch-opportunity, acceptable-first-action, invalid-argument-correction, duplicate-Hypothesis/Experiment, Memory-retrieval-cost and Tool-Schema-token denominators remain `UNAVAILABLE`; the acceptance does not report invented zeros.
- Offline model-composition matrix: the existing permanent scoped-Vision, direct multimodal-primary, disabled/unavailable, Profile/Turn-boundary, immutable SamplingStepPlan, Deferred activation and Replay regressions pass in the affected and complete Gates. The new CLI long-flow lane itself is intentionally text-primary and contains no image input; it does not duplicate those established owners.
- Retrieval and Guidance: the sealed deterministic retrieval fixtures retain exact and aggregate `Recall@10 = 1.00` and aggregate `Precision@10 >= 0.60`. Repeated-read/Context-token improvement and Guidance critical-gap closure/cost still lack typed outcome denominators, so both remain Shadow, model-invisible, non-evidence and unpromoted. Observation/Replay/Vision promotion metrics likewise remain Deferred/`UNAVAILABLE`; the accepted path separately proves no Finding or Observation promotion occurred.
- Regression and audit: the frozen M4-S3e2 five-suite provenance/Compaction checkpoint still passes 38/38 and an independent Terra xhigh regression audit returned `APPROVE` with no P0/P1, matching its sealed completion record, so no sealed S3e2 code was reopened. M4-S4 focused acceptance passes 1/1; its twelve affected suites pass 142 default-enabled tests with one Live Smoke ignored. Independent Terra xhigh review first approved the bounded test, but main review then found and repaired a Capsule input/output causality ambiguity and an under-realized fixture path. The independent repair review returned `APPROVE` with no P0/P1/P2.
- Complete local Gate: `cargo fmt --all -- --check`, denied-warning locked Clippy, `cargo test --locked --all-targets --all-features`, `cargo build --locked`, production inline-test, generic Provider/protocol-owner, forbidden Codex dependency, `git diff --check` and clean fixed `codex/` scans pass on Rust `1.93.1`. The complete suite discovers 381 tests: 379 default-enabled tests pass and two explicit Live Smokes remain ignored by default.
- Compatible Live lane: the user-owned explicit `/Users/f0und/.config/cyber-agent/deepseek-live.toml` selects `deepseek-v4-flash` as a text-only, Full-only Responses Primary through the one Config v1 Composition Root. The primary protocol smoke passes 1/1 and the fixed Shadow AgentLoop passes 1/1 after actually invoking its single permitted `replay_read` Tool. This proves only compatible text Responses and one controlled Tool round; it is not reported as a Vision or full real-target long-flow PASS.
- Unavailable Live lanes: no explicit compatible text-primary-plus-Vision configuration and no explicit compatible multimodal-primary configuration exist, so both Live lanes are `UNAVAILABLE`. Offline coverage passes, but model names are not used to infer image capabilities and no substitute credential or Codex login state is consulted.
- Remote seal: implementation commit `32973fd`, private PR `#59` and Squash Merge `a7143ba` form the M4-S4 delivery seal. Both commit subjects contain `[skip ci]`; PR status reported no checks, no GitHub Actions run was created, and no tag, release or architecture-manifest seal occurred.

### Lean Review

- Semantic owners and reuse: no production semantic owner changed. The permanent vertical test reuses the existing CLI Chat, Provider Composition, one AgentLoop, Tool Registry, Research/Event/Artifact/Receipt, RequestAudit, ContextDeliveryMap, Compaction/Resume and metrics paths. Keeping the vertical assertions in the existing CLI loopback suite avoids a second integration harness or copied Provider server.
- API, dependencies and configuration: no `src/`, Cargo dependency/feature, public API, Port, Event, Config field, Provider/Tool binding, Prompt, renderer, registry, cache, store, workflow or Codex port was added. The new fixture and test have no production consumer and create no production surface; no test-only public projection was introduced.
- Removed/deferred: the temporary metric print used only to capture the accepted local estimate was removed before the final diff. No experimental in-place test, target artifact or Live credential enters the repository. A general benchmark/Eval framework, outcome telemetry, autonomous experiment generation, Web UI, additional Provider bindings and unproven retrieval/guidance promotion remain absent or Deferred.
- Remaining risk at the original seal: the then-available Live configuration could not validate either image route, and Provider usage/token detail was not exposed by the durable local metrics path. The post-seal addendum below narrows only the dual-Provider protocol/service uncertainty; formal CLI Vision integration, the single-multimodal lane and Provider usage detail remain explicitly unavailable. There is no blocking code or governance debt for the bounded M4-S4 acceptance.

### Post-Seal Dual-Provider Vision Live Addendum

- Authorization and scope: on `2026-08-31` the operator separately authorized one explicit-config DeepSeek dual-Provider compatibility run after the M4-S4 remote seal. This addendum does not reopen M4-S4, authorize M5, change production behavior or replace the sealed offline/controlled acceptance record.
- Explicit composition: the user-owned local Config v1 selects `deepseek-v4-flash` as the Full-only text Primary and `deepseek-v4-flash-vision-exp` as the Secondary Vision Provider with `vision_mode = "secondary_tool"`. Its explicit Vision Capability Mapping declares image input, structured output and Tool calls with a `1048576` context window; no capability is inferred from either model name.
- Vision protocol/service result: one temporary ignored probe compiled and passed 1/1. A real `view_image` produced a Project Artifact for a synthetic 512-by-256 PNG; the existing Responses encoder sent that Artifact as `input_image`; the real Secondary returned a strict structured `VisionObservation` that correctly identified the left red region, right blue region and centered green square; the text Primary then consumed that Observation and returned the fixed success marker.
- Compatible Primary regressions: under the same dual-Provider Config v1, the generic Primary Responses smoke passed 1/1 and the fixed Shadow Live AgentLoop passed 1/1 after invoking only its permitted replay Tool. These independently preserve the existing Primary text and controlled Tool-loop paths.
- Truthful boundary: this proves the generic Responses Vision wire contract, strict `VisionObservation` service and Observation-to-text-Primary compatibility. The temporary probe had to bind the Project image resolver directly because the production CLI has no image argument and does not route Chat through `VisionSessionService`; therefore formal CLI image ingress and an integrated Observation-to-same-AgentLoop Tool/Receipt long flow remain `UNAVAILABLE_NOT_WIRED`, not PASS. A single multimodal-Primary Live lane remains `UNAVAILABLE_NOT_RUN`.
- Hygiene and isolation: the temporary probe was deleted after the authorized runs. No `src/`, permanent `tests/`, Cargo, architecture, ADR, Provider binding or public API changed; no secret entered Git or durable Project facts; no GitHub Actions run occurred; Codex credential/login/session data was not read or modified.
- Seal effect: M4-S4 remains `COMPLETE_REMOTE_SEALED`. This status-only addendum replaces only the stale claim that no compatible dual-Provider configuration exists; it does not convert the unwired CLI lane or unrun single-multimodal lane into a PASS.

## Next Target

- Current target: `STOPPED_PENDING_NEW_AUTHORIZATION`; F2a is complete and no further action is authorized.
- M5-S1 CLI sufficiency evaluation is `READY_NOT_AUTHORIZED`; F2a does not authorize Web work, CI restoration, release packaging or any M5 implementation.

## Current Authorization

The F1e and M4-S3c authorities are consumed. PR `#51`/`32bc0f2` and PR `#52`/`7d5b6d0` consumed the M4-S3d and M4-S3e1 submission authorities. The user authorized completing, testing and independently sealing M4-S3e2, then proceeding in order through M4-S3f, M4-S3g and M4-S4; PR `#59`/`a7143ba` completes that sequence. The later dual-Provider compatibility Live and status addendum are sealed through PR `#61`/`385a318`. Private PR `#62` consumes the F2a design, implementation, audit, Gate, dual/single-multimodal Live and remote-seal authorities. The earlier Actions pause remains in force.

- Run mode: `STOPPED_PENDING_NEW_AUTHORIZATION`; M5-S1 is not authorized
- Git mode: `NONE`; the one-time `F2A_PRIVATE_PR_SKIP_CI` authority is consumed by PR `#62`
- GitHub Actions mode: `PAUSED_BY_OPERATOR`; do not trigger or rerun a Workflow until the user explicitly resumes it after quota recovery
- Delegation mode: `AUTHORIZED_BOUNDED`; Terra xhigh owns cross-boundary implementation/audit and Luna xhigh only bounded modules/tests/docs as defined by repository governance; one-Agent runtime is unchanged
- Live test mode: `OFF`; the fixed F2a dual-Provider and single-multimodal authorities are consumed
- Configuration rule: Provider binding, model, Base URL and Capability Mapping remain Config v1 data. A Provider selects exactly one inline `api_key` or dedicated `credential_ref`; neither may infer Provider/model identity or use Codex login state.
- Stop condition: satisfied; remain stopped before M5

## M4-S3e1 Slice Start

- Base commit: M4-S3d Squash Merge `32bc0f2`; `main` matches `origin/main`, PR `#51` is merged and no Actions run was created.
- Intended production touch points: the existing Research record/event payload, chained Research reader/upcaster, Research reducer/reference validator, `research_update` schema/Adapter and one pure derived evidence-gap projection.
- Scope lock: add only the fixed supports/contradicts/positive_control/negative_control/environment_control/reachability_proof/prerequisite_proof/impact_proof relations; conservatively upcast historical positive/negative vectors; distinguish unknown from missing gaps. Do not add a second evidence store, graph database, model judge, autonomous transition, Context/Compaction work or Observation/Vision/Replay promotion.
- Compatibility: original Research Events remain immutable; historical `evidence_refs` become supports and `negative_refs` become contradicts only in the current projection. Control/proof roles require new explicit `research_update` input.
- Intended tests: every relation and reference kind, duplicate/cross-project/type/missing-reference failure, complete v1-to-current fixture upcast, original-stream immutability, explicit-only controls and proof that the gap projection cannot write Research/Memory, change Phase or promote derived observations.

## M4-S3e1 Lean Review

### Semantic Owners and Production Path

- `src/domain/research/records.rs` owns Research payload v2, the eight fixed `EvidenceRelation` values and strict `EvidenceLink`/nested `EvidenceRef` decoding. `src/domain/research/upcast.rs` is the sole chained v1-to-v2 owner: legacy positive references become `supports`, negative references become `contradicts`, controls are never guessed and original JSONL bytes remain unchanged.
- `src/domain/research/reducer.rs` owns reference existence, exact duplicate rejection and one target/version applicability predicate shared by record, update and Gap paths. Finding updates are staged before mutation; cross-target/version evidence fails atomically. `ResearchEvent::ExperimentRecorded` boxes its Capsule only as an internal enum-size optimization; encoded payload bytes and schema remain unchanged.
- `src/adapters/research_projection.rs` composes the current Project's authoritative Session Receipt facts with Research replay. Receipt identity is the persistent Session Event ID; every Capsule Receipt resolves to a real Receipt whose Execution ID occurs in the same Capsule, and a duplicate Receipt Event ID across Session streams is rejected as ambiguous. This catalog is derived per read and is never serialized as a second Receipt store.
- `src/domain/research/evidence_gap.rs` is one pure, deterministic, non-evidence projection over typed Research facts. It preserves the twelve fixed ordered Gap kinds, distinguishes `unknown` from `missing`, filters repeatability and Artifact/Receipt provenance through the same target/version applicability owner, and has no writer or Phase transition.
- The current production consumers are the fixed `research_update` writer and `research_get` reader plus existing Artifact, Memory, Responses image resolution, CLI Chat and CLI inspection Research loads. They all reuse the same reference-aware Research reduction path.

### Cache Correction, Reuse, API and Dependencies

- Independent audit proved that the old `research_get` result-cache key covered only Research JSONL while the result now also depends on every Project Session Receipt stream. `research_get` therefore uses `ToolCachePolicy::None` and always performs the current strict projection. Folding all Session bytes into the key was rejected because the Tool's own later Receipt would make the key unstable.
- Because `research_get` was the only production consumer, the unconsumed `LocalToolResultCache`, its Session audit adapter, Registry binding, lookup/store path and cache-only tests were removed instead of retained as dormant infrastructure. Historical `ToolCacheSource`, `ToolCacheValidation`, `ToolResultCacheHit` and Receipt payload decoding remain for old Session/Trace compatibility; Fragment/Context caches are unaffected.
- Existing Research, Session, Artifact, Memory, AgentLoop, Tool Registry and Project Event Store owners are reused. No Cargo dependency, feature, Provider/configuration field, Port, Event store, graph/vector database, model judge, workflow, Hook/Registry or public test-only projection was added. Cargo, architecture and ADR files are unchanged; Codex reuse is `REFERENCE` only and the Upstream Port Ledger is unchanged.

### Audit, Tests and Complete Gate

- The required sequence completed as专项测试 → affected tests → independent Terra xhigh audit → fixes → audit repair review → complete Gate → Lean Review. The first audit returned `CHANGES_REQUIRED` with three P1 findings: nested unknown EvidenceRef fields, unverified Capsule Receipt strings and missing target/version applicability. Their repairs and permanent regressions passed.
- Repair review found one further P1 stale-cache path. Removing the unsafe cache path and adding the cross-Session duplicate-Receipt regression received `APPROVE` with no P0/P1/P2. Main cross-boundary review then made Experiment Artifact/Capsule provenance target-aware; the final bounded Terra xhigh review again returned `APPROVE` with no P0/P1/P2.
- Permanent top-level tests cover all eight relations, exact/request duplicates, wrong/missing/cross-Project references, strict nested fields, v1 immutable Upcast, real/mismatched/ambiguous Receipt facts, target/version record and update atomicity, cross-target Artifact provenance, repeatability filtering, stable Gap ordering and read-only behavior. The increased fixed Research Tool Schema required only raising one old constrained Context fixture from 16K to 18K so it still reaches and verifies its original World-State truncation boundary.
- Complete local Gate passes on Rust `1.93.1`: formatting, denied-warning Clippy, 354 discovered tests with 352 default-enabled tests passing and two existing opt-in Live Smokes ignored, plus locked build. Production inline-test scan, forbidden Codex dependency scan, changed-file credential scan, `git diff --check`, unchanged Cargo/architecture/ADR files and clean fixed `codex/` commit `868c9edb0da913a5fc699a71664e65f44f6058b0` all pass.

### Removed and Deferred

- Removed the unsafe `research_get` persistent-result cache runtime, its sole-consumer adapter/session audit and cache-only execution tests. No temporary inline test, second Evidence/Receipt authority, public fixture surface, model-inferred relation or future placeholder abstraction remains.
- Deferred exactly as planned: ContextDeliveryMap/Evidence-Spine Compaction is M4-S3e2; retrieval and guidance Shadow are M4-S3f/g; M4-S4 remains the long-flow acceptance Slice. M4-S3e1 commit, push, PR, Actions, Live Test, merge, tag, release and architecture seal remain unauthorized.
- Governance debt: none blocking. Tool Result Cache benefit/eligibility remains an explicit future measurement question rather than an active runtime abstraction.

## M4-S3d Slice Start

- Base commit: M4-S3c Squash Merge `1b7b032`; `main` matches `origin/main` and PR `#50` is merged with no new Actions run.
- Intended production touch points: one provider-neutral Observation/TargetDelta Domain owner, the existing Research/Artifact/Receipt reference validators, and one bounded AndroidManifest external Extractor composed through existing `run_commands` rather than a second process executor.
- Scope lock: each Observation binds TargetSnapshot, ToolCall, real ExecutionReceipt, raw Artifact and exact Extractor ID/version; deterministic TargetDelta compares two Snapshot-bound Observation sets with stable ordering and full provenance. Do not add a second fact database, scanner framework, in-Rust manifest parser, generic Extractor registry, workflow, Provider path or new dependency.
- Intended tests: strict schema/unknown-field/reference rejection; AndroidManifest success/failure/version fixtures; stable component/exported/permission/intent-filter deltas; exact provenance back to Snapshot/Call/Receipt/Artifact/Extractor; and proof that Observation/Delta never promotes a Finding or writes Research/Memory.

## M4-S3d Lean Review

### Semantic Owners and Production Path

- `src/domain/observation.rs` owns the strict provider-neutral Observation batch/envelope contract and the first closed `android_manifest_component` kind. It validates exact schema versions, required and unknown fields, unique Observation/component/filter identities, bounded Extractor identity syntax and raw-Artifact evidence inclusion; it does not parse Android binary XML or write facts.
- `src/domain/observation/delta.rs` owns one pure deterministic TargetDelta over two distinct Snapshot-bound Observation sets for the same target and exact Extractor identity/version. Stable component/kind/filter ordering and complete before/after provenance are produced once; added/removed components retain their full manifest details.
- `src/app/observation.rs` owns the read-only cross-fact validation path. It verifies actual Artifact Blob size, the 512-KiB ingestion bound, TargetSnapshot and evidence-Artifact applicability, exact `extractor_id@version` toolchain provenance, the durable `run_commands` ToolCall/paired successful ToolResult and the matching real successful ExecutionReceipt.
- The only current production consumer is the local CLI inspection path: `inspect android-manifest` validates one external result and `inspect target-delta` compares two validated results. Neither command appends Session/Research/Memory Events, creates a Finding or introduces an Observation store.

### Reuse, API, Dependencies and Split Review

- Reused the existing Research reducer, TargetSnapshot and ArtifactRecord facts, SHA-256-verified LocalArtifactStore, Session reducer and ConversationHistory pairing, real ExecutionReceipt records, generic `run_commands` execution and project discovery. No second process executor, artifact publisher, pairing validator, database, index or AgentLoop exists.
- All new Observation/App/Delta production surface is `pub(crate)` and has a current CLI consumer. No external `pub`, Port method, Event, Config, Prompt, Cache, Provider binding, Cargo dependency, feature, Extractor Trait/Registry or test-only projection entry point was added.
- The Observation schema and Delta algorithm are split by change reason: ingestion validity and comparison evolution are independent. No generic scanner/extractor framework, in-Rust aapt/JADX parser, Hook, workflow or future placeholder abstraction was retained.
- Codex classification is `REIMPLEMENT/DOMAIN-SPECIFIC`; no Codex source was copied, no dependency was added and the Upstream Port Ledger is unchanged.

### Audit, Tests and Complete Gate

- The required sequence completed as专项测试 → affected tests → independent bounded Luna xhigh audit → fixes → complete Gate → Lean Review. The independent audit returned `CHANGES_REQUIRED`, not `APPROVE`: it found three P1 issues (actual Artifact size, Extractor-to-Artifact toolchain binding and incomplete added/removed component details) plus one P2 test-coverage gap.
- All findings were fixed with permanent top-level regressions. Main cross-boundary review additionally required same-batch Call/Receipt identity, same-Snapshot evidence Artifacts, full intent-filter before/after provenance and stable input-order independence. The original audit result remains recorded honestly; resolved findings plus main review form the final acceptance, not a fabricated approval.
- `tests/observation_target_delta.rs` retains five integration tests covering real existing `run_commands` execution, strict schema/unknown-kind/missing/duplicate rejection, forged/mismatched Snapshot/Artifact/Call/Result/Receipt paths, Blob hash/size and toolchain tampering, Extractor-version refusal, deterministic ordering, complete Delta provenance and absence of Research/Memory/Finding writes.
- Complete local Gate passes on Rust `1.93.1`: formatting, denied-warning Clippy, 354 discovered tests with 352 default-enabled tests passing and two existing opt-in Live Smokes ignored, locked build, production inline-test scan, forbidden Codex dependency scan, credential scan, `git diff --check`, unchanged Cargo/architecture/ADR files and clean fixed `codex/` verification. The existing CLI Chat Ctrl-C loopback wait was raised from 5 to 15 seconds only to tolerate process-heavy full-Gate scheduling; its cancellation and resumability assertions are unchanged.

### Removed and Deferred

- No temporary inline tests, duplicate fixtures, public test access, unconsumed Observation type, Observation persistence, scanner wrapper, Extractor registry or speculative generalized Delta abstraction remains.
- Deferred exactly as planned: explicit Observation-to-Research promotion telemetry remains unavailable/Deferred rather than fake zero; typed EvidenceLink and evidence-gap projection begin only in separately authorized M4-S3e1; Context provenance/Compaction, retrieval/guidance Shadow work and M4-S4 long-flow acceptance remain later Slices.
- Governance debt: none blocking. M4-S3d is remotely sealed through PR `#51` and Squash Merge `32bc0f2`; no Actions run, tag, release or architecture seal was created.

## M4-S3c Lean Review

### Semantic Owners and Reuse

- `src/adapters/tools/http_request/` owns one provider-neutral bounded HTTP request: closed method/URL/header/query/body inputs, pre-follow URL and redirect validation, bounded timeout/request body/preview/capture, completed HTTP status/final URL/metadata/timing and one response-body Artifact. HTTP `4xx/5xx` remain successful completed protocol results; transport, timeout, capture and post-request Artifact publication failures remain real execution failures with one Receipt.
- `src/adapters/tools/environment.rs` and its private `environment/{probe,schema}.rs` modules own one fixed environment projection over OS/runtime, Git, proxy-presence booleans, five fixed toolchains and ADB/xctrace device observations. Probe outputs are concurrently drained through the existing command-process owner under fixed memory bounds, normalized to strict version tokens, stably sorted and hashed once with BLAKE3; no raw stdout/stderr, proxy value, arbitrary Probe DSL or Environment store is exposed.
- `src/adapters/tools/artifact.rs` remains the sole Artifact provenance and publication owner. It now provides network-before-action provenance preflight, repeats validation under the real writer before publication, and resolves ArtifactRef request bodies through authoritative Research state plus Blob identity with a consumer size check before Blob read. The HTTP Adapter does not construct a second Artifact/Event path.
- The existing Tool Registry remains the single Schema/guidance/normalize/dispatch owner. `http_request` is `ExternalMutation + NeverAutoRetry + Nondeterministic + cache None`, Deferred by default and Direct only for Web/Mobile DomainProfiles. `inspect_environment` is `ReadOnly + ReplaySafe + EnvironmentDependent + cache None` and Direct in all Domains. Both consume the existing immutable ResolvedToolPlan, ResourceKey scheduling, ToolFailure, ExecutionReceipt, RequestAudit and Tool Call/Result pairing paths.

### API, Dependencies and Scope

- No Cargo dependency, feature, Config field, Event schema, fact store, Provider binding, model/router branch, service, Hook, workflow, scanner or external vulnerability-tool wrapper was added. Existing `reqwest`, BLAKE3, SHA-256 Artifact storage, Research replay and process execution are reused; Codex remains REFERENCE/REIMPLEMENT with no copied source or Port Ledger entry.
- New production visibility is parent/crate-private and has current production consumers: the Artifact preflight/bounded-read helpers are consumed by `http_request`; the fixed process probe is consumed by `inspect_environment`; `research::load_state` is widened only to its sibling Artifact owner. No public or controlled test-only projection exists.
- Main review removed an unused `Clone`, rejected a post-follow-only redirect check, made fixed process capture streaming and memory-bounded, tightened version parsing against secret-bearing stderr tokens, accepted real Apple physical/simulator identifiers and corrected the dependency-direction scan from the overbroad string `http` to the Rust path `http::` so a provider-neutral Domain Tool name is not mistaken for wire I/O.

### Audit, Tests and Gate

- Independent bounded Luna xhigh audit returned `CHANGES_REQUIRED`: oversized ArtifactRef bodies were read before the authoritative size check; successful probe stderr could be misclassified as a version; and `Command::output` collected fixed-probe output before truncation. All three were fixed with permanent regressions or the existing command integration suite. The optional repair-only follow-up exceeded its time limit and was terminated with no conclusion; it is not recorded as `APPROVE`. Main cross-boundary review closed the findings from source and regression evidence.
- Permanent top-level tests cover exact Effect/Retry/Cache/Exposure contracts; success, `4xx/5xx`, redirect, unsafe pre-follow redirect rejection, timeout, binary/captured-prefix Artifact, JSON and ArtifactRef request bodies, pre-network provenance rejection, pre-read oversized Artifact rejection and credential non-echo; environment available/missing/version-failure, successful stderr secret rejection, proxy-value non-echo, stable order/fingerprint and ADB/xctrace device counts. Registry, Domain/Phase exposure, Tool accuracy, SamplingStepPlan, RequestAudit, History pairing, Replay and Responses affected suites all pass.
- Complete local Gate: PASS on Rust `1.93.1`. `cargo fmt --all -- --check`, denied-warning Clippy, `cargo test --locked --all-targets --all-features` and `cargo build --locked` all exited `0`; 349 tests are discovered, 347 default-enabled tests pass and the two existing opt-in Live Smokes remain ignored. One existing CLI Ctrl-C race timed out once, then passed its exact reproduction and the complete rerun without code changes.
- Lean scans: PASS. Production inline tests, forbidden Codex dependencies, real-secret patterns and vendor branches are absent; exactly one AgentLoop and no Environment store remain; Cargo dependency files and `codex/` are unchanged; `git diff --check` passes. No unresolved P0/P1 or governance debt remains.

### Live Test and Local Seal

- Result: `PASS`; one configuration-driven DeepSeek `deepseek-v4-flash` CLI Chat used the existing Config v1 Composition Root, Responses HTTP/SSE Adapter, one AgentLoop and a temporary isolated Web Project. `doctor` first reported the configured Primary ready with Vision disabled.
- Turn 1 produced one real `research_update` ToolCall, a paired successful ToolResult and a real ExecutionReceipt, durably recording fixed TargetSnapshot `snapshot-m4-s3c-live-web`.
- Turn 2 produced exactly two real ToolCalls, `inspect_environment {}` and one bounded `http_request` GET to loopback `http://127.0.0.1:18765/Cargo.toml`; both paired ToolResults and both real ExecutionReceipts succeeded under the same immutable ResolvedToolPlan. The HTTP Receipt references `artifact-m4-s3c-live-cargo-toml`.
- HTTP evidence: status `200`, captured body `961` bytes, complete Artifact `artifact-m4-s3c-live-cargo-toml`, Blob `5c3d86e3f265b2fe6de83e64daaf39b6df6cfa01752cafcbcf16c25b72df0b5c`. The Blob SHA-256 exactly equals the served repository `Cargo.toml`; the Research stream durably links it to the fixed TargetSnapshot.
- Environment evidence: the stable fixed projection returned fingerprint `b9a0196d6c5544e088d816446c680993207464895fc5b6c2eb59954df24ef48c`; the final model answer quoted the exact HTTP status, environment fingerprint, Artifact ID and Blob ID.
- Audit/continuation evidence: four RequestAudit entries bind Full ModelInputIR v3 delivery and the same advertised Tool plan. The post-Tool audits preserve completed plaintext `reasoning -> ToolCall(s) -> ToolResult(s)` order, while durable Session History contains zero reasoning items and preserves all three call/result pairs.
- Isolation: the public Provider request used only the explicit Cyber Agent inline credential selected by the local Config. The credential was not printed or copied into the repository, temporary Project facts, RequestAudit Blobs or shell profiles. Codex login state, `~/.codex` and `CODEX_HOME` were not read or changed. The loopback server was stopped; the temporary Project was removed from `/tmp` into the user Trash and remains recoverable, while the user-owned Provider config is retained.
- Git/remote: `COMPLETE_REMOTE_SEALED`; implementation commit `05ae0e3`, PR `#50` and `[skip ci]` Squash Merge `1b7b032` complete the M4-S3c publication with no Actions run while the operator pause remains active. No tag, release or architecture seal occurred.

### Removed and Deferred

- No vendor client, crawler/scanner framework, arbitrary environment probe, second HTTP stack, duplicate Artifact publisher, dedicated external-tool Rust wrapper, test-only public API or speculative abstraction remains.
- Deferred exactly as planned: ObservationEnvelope/AndroidManifest Extractor/TargetDelta (M4-S3d), typed Evidence/provenance, retrieval/guidance Shadow work and M4-S4 long-flow acceptance. Generic remote-target scanning and additional toolchain/device protocols require separately measured consumers and a separately authorized Slice.

## M4-S3a1 Slice Start

- Base commit: `ee04b7e`; `main` matches `origin/main`. The existing uncommitted Architecture v1.2/v1.3 and governance documentation changes are preserved in place.
- Intended production touch points: the existing Context Planner and canonical Tool catalog, the one AgentLoop, ToolExecutor Port, Replay Tool Adapter, RequestAudit schema and the existing ToolExecutionPlan/ExecutionReceipt contracts.
- Scope lock: add one immutable `SamplingStepPlan`, one pure `ResolvedToolPlan` owner, current-Turn `DeferredActivationSet`, fixed `generic-v1` renderer identity and the minimum behavior-preserving ContextSlot/ContextOrigin migration. Do not add persona/Phase, Tool-accuracy, Evidence, Retrieval, Guidance, Provider or Live behavior.
- Reuse: preserve the current ToolSpec normalization, History pairing, Context ordering/budget/rendering, Baseline, RequestAudit, scheduler, execution and Replay owners. Codex remains `REFERENCE`; no source Port or dependency is admitted.
- Dependency/public budget: no new crate, Cargo feature, Provider binding, fact store, general Registry/Router/Hook/Policy or second AgentLoop. New API must have a current AgentLoop, RequestAudit, Tool dispatch or Replay consumer and default to crate-private where possible.
- Baseline verification: the unmodified tree passed formatting, denied-warning Clippy, 310 default-enabled tests with two explicit Live Smokes ignored, locked build, production inline-test scan, `git diff --check` and clean fixed `codex/` verification.
- Acceptance: full locked Cargo/Lean Gate plus permanent tests for immutable Step/Tool binding, current-Turn Deferred lifetime, paired pre-execution rejection, Replay fixed-plan behavior, renderer identity and byte-preserving Context migration.

## M4-S3a1 Lean Review

### Semantic Owners

- `src/domain/sampling.rs` owns the immutable `SamplingStepPlan` and `TurnConfigurationSnapshot`; the one AgentLoop captures it once per model sampling Step and reuses it for Context, RequestAudit, Tool schemas and dispatch.
- `src/domain/tools/resolution.rs` owns pure Tool exposure resolution, ordered `ResolvedToolPlan`, stable plan fingerprint and current-Turn `DeferredActivationSet`; `ToolRegistry` remains the single immutable canonical Tool catalog and runtime binding owner.
- `ContextPlanner` remains the only Context ordering, budgeting, truncation, rendering and fingerprint owner. `PromptRendererIdentity::generic_v1()` only makes the existing renderer version explicit and preserves exact Prompt bytes.
- Existing History normalization, argument validation, scheduling, Tool execution, Receipt persistence and Replay owners remain unchanged. Plan rejection uses the existing paired ToolResult path and never executes a Runtime or invents a Receipt.

### Reuse, API and Dependencies

- RequestAudit schema v2 additively binds `PromptRendererIdentity` and `ResolvedToolPlanFingerprint`; schema v1 remains readable and explicitly lacks those verified identities. Trace manifest schema v2 records the renderer while the fixed v1 fixture remains readable as unknown.
- `ToolExecutionPlan` and `ExecutionReceipt` add an optional resolved-plan fingerprint for historical compatibility. Replay consumes one fixed immutable Replay plan and cannot fall back to the live catalog.
- The minimum non-persisted Context migration separates `ContextSlot` from `ContextOrigin` while retaining the historical `ContextFragmentSource` compatibility mapping and all existing rendered order/bytes.
- No crate, Cargo feature, Provider/configuration/credential path, second AgentLoop/catalog/renderer, Event fact source or Codex dependency was added. Codex use remains `REFERENCE`/local `REIMPLEMENT`; no PORT ledger entry is required.

### Tests and Verification

- Permanent `tests/m4_sampling_plan.rs` coverage proves one plan binds Context/ToolSpecs/RequestAudit/dispatch; captured plans are not changed by later candidate configuration; Deferred activation affects only the next Step of the current Turn and clears on Turn/Resume; Hidden, unavailable, unadvertised and fingerprint-mismatched calls fail before Runtime with paired results and no Receipt.
- Existing Context, RequestAudit, Trace, Replay, Vision, Session, CLI Chat and Tool-loop suites prove byte compatibility, historical schema readability, renderer-triggered Full delivery, Primary/Vision isolation and fixed Replay dispatch.
- Final Gate passed `cargo fmt --all -- --check`, denied-warning locked Clippy, all-target/all-feature locked tests, locked build, production inline-test scan, one-package/one-default-binary/one-AgentLoop checks, mutable exposure and forbidden Codex dependency scans, `git diff --check` and fixed read-only `codex/` verification.
- The suite discovers 317 tests: 315 default-enabled tests pass and two explicit Live Smokes remain ignored. No Live Provider request or credential resolution occurred.

### Removed and Deferred

- Removed the Registry-owned mutable exposure state, discovered-Deferred set and exposure mutation/planning entry points, so no second active exposure authority remains.
- Complete Authority/Retention/Freshness/ContextDeliveryMap, stable persona, DomainProfile/ResearchPhase, Tool Accuracy, Evidence, Retrieval and Guidance behavior remain deferred to their named Slices; no placeholder Trait, Registry, configuration, Event or unused schema was retained.
- No Git branch, commit, push, PR, tag, release or GitHub Actions run occurred.

## M4-S3a2 Slice Start

- Base: locally completed M4-S3a1 working tree over remote status-seal commit `ee04b7e`; all prior uncommitted architecture/governance and a1 implementation changes remain preserved.
- Intended production touch points: the existing fixed Profile/Prompt resources and composition owner, Session Event/upcaster/reducer path, Turn-boundary AgentLoop configuration, Context fragment construction, immutable Tool resolver and read-only CLI/Replay reconstruction paths.
- Scope lock: add exactly one versioned `Pragmatic Vulnerability Researcher` Base Contract; rename the current source type from `ResearchProfile` to `DomainProfile` without a duplicate compatibility type; add the five fixed compile-time `ResearchPhase` entries; durably select/default/restore Phase only at Turn boundaries; compose Base/Domain/Phase Prompt and exposure through the existing single owners.
- Reuse: preserve the one AgentLoop, ContextPlanner, Session/Event Store, RequestAudit, immutable Tool catalog/resolver, Baseline and historical `prompt:mode:*` reconstruction. Phase is a Session projection, not a Research workflow or Finding state machine.
- Dependency/public budget: no new crate, Cargo feature, Provider/credential path, Personality/Profile/Phase/Prompt registry, automatic Phase inference, workflow engine, Hook, second AgentLoop or new fact store. New public surface is limited to real Domain/App/CLI consumers and defaults to crate-private otherwise.
- Codex classification: `REFERENCE` only; no upstream source copy, dependency or PORT is admitted.
- Acceptance: four Domain by five Phase composition, exact fixed Prompt order/identity, one persona, Turn-boundary queue/durable-append/apply atomicity, new/legacy/Resume continuity, immutable plan exposure, Baseline invalidation and historical audit/replay compatibility, followed by the complete locked Cargo/Lean Gate.

## M4-S3a2 Lean Review

### Semantic Owners

- `src/domain/research_control/phase.rs` owns the one compiled Base Contract and five fixed `ResearchPhase` resources; `selection.rs` owns source/reason/generation validation; `prompt.rs` owns the sole Base → Personal → Project → Domain → Phase composition and fragment identity path. The root module only re-exports that fixed surface.
- `src/domain/domain_profile.rs` owns the four fixed Domain resources, TargetKind compatibility and bounded Domain exposure. It neither selects a Phase nor creates a combined role.
- `src/domain/session.rs` owns the strict, versioned `ResearchPhaseSelected` payload wrapper and its reducer projection. `cli_chat.rs` durably appends the choice before queueing it into the existing AgentLoop; a failed append therefore has no applied Phase or Provider path.
- The existing AgentLoop, ContextPlanner, immutable Tool catalog/resolver, SamplingStepPlan, Baseline and RequestAudit owners remain unchanged. Domain/Phase only supply compiled fragments and one bounded exposure layer to those owners.

### Reuse, API and Dependencies

- Reused the existing Session envelope/reducer, append-after-durable projection, Context fragment cache/fingerprint, Tool exposure resolver, SamplingStepPlan, Baseline invalidation and CLI Chat composition. No Cargo crate, feature, Provider binding, credential source, Fact store, Registry, Workflow, Router, Hook or second AgentLoop was added.
- `ResearchProfile` and `ModePack` were removed without a compatibility alias. Historical `ContextFragmentSource::ModePrompt` and `prompt:mode:*` reconstruction remain read-only compatibility paths; new requests use only Base/Domain/Phase identities.
- The only persisted addition is the strict Session Event payload for phase selection. It has an explicit `schema_version`, rejects unknown outer and nested fields, validates the phase generation plus source/reason pairing, and reuses the existing Session schema failure behavior.

### Tests and Verification

- Permanent M4 regressions cover all four Domains × five Phases for deterministic Prompt fingerprints and bounded exposure, current-Step immutability across a queued Domain/Phase change, latest durable Phase restoration on Resume, strict payload schema/unknown-field/source-reason/generation rejection, and failed append atomicity.
- CLI loopback regressions prove a new Session records one `NEW_SESSION_DEFAULT_MAPPING` before its first request, a legacy read-only replay writes `LEGACY_DEFAULT_MAPPING` only on its next real Turn, and `/phase` records the operator source/reason before the Turn. Existing Context, RequestAudit and Replay coverage keeps historical `ModePrompt` readable.
- Passed: `cargo fmt --all -- --check`; `cargo clippy --locked --all-targets --all-features -- -D warnings`; `cargo test --locked --all-targets --all-features`; `cargo build --locked`; the production inline-test scan; `git diff --check`; and focused M4/CLI loopback suites. The two explicit Live Smokes remain ignored; no credential resolution or real Provider request occurred.
- Independent review initially found three P1 defects: Mapping left `apply_patch` Direct, two old tests asserted pre-Control-Plane values, and arbitrary Prompt renderer identities were accepted. All three were corrected before completion: Mapping mutation is Deferred, tests assert current semantic owners, and persisted current schemas accept only `generic-v1` while v1 legacy `None` remains readable.
- The strict renderer correction exposed five historical Replay mismatches. The final implementation reconstructs exact historical RequestAudit instruction bytes only through a crate-private `PreparedTraceReplay` path; ordinary Prompt composition cannot select it, historical facts are not rewritten, and the fixed M4-S2 Trace again replays exactly.
- Parent review reran the M4 Research Control, immutable Sampling, CLI Chat, Tool-loop and Trace suites together: 70 tests passed, one explicit Live-only Shadow smoke remained ignored, and no Provider credential or network path was entered.

### Removed and Deferred

- Removed the monolithic Research Control implementation file in favor of three ownership-based modules, and corrected historical tests that still assumed the pre-Base/Domain/Phase fragment-cache count or Experiment exposure.
- Deferred exactly as planned: M4-S3b Tool accuracy, all later evidence/provenance/retrieval/guidance Slices, any second Prompt renderer, any dynamic research registry or workflow engine, and all Live Provider evidence.
- No Git branch, commit, push, PR, tag, release or GitHub Actions run occurred. No file under `codex/` and no Codex login or credential state was accessed.

## M4-S3b Slice Start

- Base: locally completed M4-S3a1 and M4-S3a2 working tree over remote status-seal commit `ee04b7e`; all existing uncommitted architecture, governance and implementation changes remain preserved.
- Intended production touch points: the canonical `ToolSpec` definitions and validators, the existing Tool Port failure boundary, AgentLoop `pre_execution`/`execution` result mapping plus the App/AgentLoop post-execution persistence-error boundary, and the existing Trace/Eval derived-result types.
- Scope lock: normalize every model-visible description to `Use when` / `Do not use when` / `Prefer` / `Produces`; replace unjustified generic output objects with precise schemas; add one provider-neutral structured ToolFailure envelope; derive ToolAccuracyReport only from existing controlled Trace/Event/Tool/Receipt/Artifact inputs.
- Failure contract: all ToolFailures remain paired ToolResults and use only `pre_execution`/`execution`; only pre-execution argument correction can lack a Receipt; every started execution has a real Receipt. Receipt/History/RequestAudit persistence failures are App/AgentLoop errors, never a rewritten successful ToolResult or fabricated Receipt; started or completed Runtime work is never auto-retried, silently replayed or reclassified as pre-execution.
- Dependency/public budget: no new crate, Cargo feature, Tool runtime, fact store, Eval platform, remote judge, Registry, Provider/credential path or second AgentLoop. New public surface must have current Tool/AgentLoop/Trace consumers and otherwise remain crate-private.
- Codex classification: `REFERENCE`/local `REIMPLEMENT` only; no upstream source copy, dependency or PORT is admitted.
- Acceptance: shared Tool schema/validation across Registry, AgentLoop, Responses and Replay; durable-Trace-only metrics with adjacent RequestAudit proof for Deferred activation; unmatched Replay termination and Observation/Replay/Vision promotion remain Deferred without typed durable signals; complete locked Cargo/Lean Gate; no Live Provider or Codex credential/login access. Git was initially OFF and later narrowed by the user to one local completion checkpoint only.

## M4-S3b Lean Review

### Semantic Owners and Reuse

- `src/adapters/tools/mod.rs` and the existing Tool-specific schema modules remain the one canonical ToolSpec/guidance/input-output Schema owner. All 25 model-visible Tools use the stable `Use when` / `Do not use when` / `Prefer` / `Produces` contract; Registry, immutable ResolvedToolPlan, AgentLoop, Responses request encoding, Replay and `tool_search` consume that same surface.
- `src/domain/tools/failure.rs` owns the provider-neutral structured ToolFailure with only `pre_execution` and `execution`. The Tool Port maps planning/validation failures once, while AgentLoop execution failures retain real Receipt semantics. Receipt, History or RequestAudit persistence failures remain App/AgentLoop errors and never fabricate a Receipt or rewrite a successful ToolResult.
- `src/domain/trace_eval/accuracy.rs` owns pure report calculation and reuses the Context Planner's Tool Schema token estimator. `src/app/trace_eval/service.rs` reconstructs its input only from validated Trace manifest, RequestAudit, History, Receipt and Artifact authorities; the report remains derived non-evidence and has no writer.
- The existing immutable Tool catalog/resolver, SamplingStepPlan, RequestAudit reconstruction, History pairing, ExecutionReceipt, Replay composition and controlled Trace fixture path are reused. No second exposure, pairing, retry, token-cost, Eval fact or Prompt owner was added.

### API, Dependency and Failure Budget

- No crate, Cargo feature, Tool runtime, Event/fact schema, Cache, Provider binding, credential source, external service, model judge, Eval platform, Hook, registry, workflow or second AgentLoop was added.
- `ToolAccuracyTraceProjection::controlled` was removed; controlled tests now enter through real AgentLoop, RequestAudit, History and Receipt boundaries. Projection construction remains crate-private.
- `ToolFailure::post_execution`, `ToolResult::failed`, the unused ToolAccuracy percentage getters, `ToolAccuracyReport::is_non_evidence`, the single-field ToolSearchExecution wrapper and unused construction strings were removed. No `ToolFieldError` or placeholder `field_errors`/`receipt_ref` API remains without a production consumer.
- `ToolFailure::post_execution` has no production replacement because post-execution persistence errors are not Tool execution outcomes. They remain explicit App/AgentLoop failures and do not trigger automatic re-execution.

### Accuracy and Schema Corrections

- Argument correction observes only the immediately following model sampling Step and requires changed arguments; parallel calls in the same sample or unrelated later calls cannot inflate correction success.
- Missed batch opportunities count only when the declared batch Tool was actually advertised in that sample; unrelated batch use cannot hide an observed sequence of singular calls.
- Deferred activation remains `None` with a typed Deferred reason. Current RequestAudit has no persistent Turn identity, so adjacent entries cannot yet prove same-Turn activation and the report does not invent success or zero.
- Precise schemas now cover file batches/search, Job/PTY completion, Snapshot/Fingerprint/FileManifest/ToolCapability and Research outputs. Only `tool_search` keeps open `input_schema`/`output_schema` objects because those values are JSON Schema documents themselves.
- Main review restored `write_file`'s truthful atomic discovery term, retained strict nested execution control with `additionalProperties:false`, updated the fixed Responses wire Fixture and resized only the deliberately constrained Context regression so exact Tool schemas fit while oversized WorldState still truncates.

### Tests, Audit and Gate

- Focused M4-S3b suite: `cargo test --locked --test m4_tool_accuracy` passed 5 tests; focused denied-warning Clippy passed.
- Initial affected suites passed 87 tests with one explicit Live-only test ignored. The later full-Gate review also ran and passed all 29 `m3_world_state_mode` tests and the Responses Adapter contract after catching four cross-suite regressions that the narrower set had not covered.
- Final full Gate on pinned Rust `1.93.1`: `cargo fmt --all -- --check`; `cargo clippy --locked --all-targets --all-features -- -D warnings`; `cargo test --locked --all-targets --all-features`; and `cargo build --locked` all pass. The suite discovers 329 tests, passes 327 default-enabled tests and keeps two explicit Live Smokes ignored.
- Luna's bounded audits timed out or ended without a final verdict and therefore are not recorded as `APPROVE`. The Terra implementation owner did not audit its own work; the main agent performed the final cross-boundary audit, corrections and complete Gate.
- Production inline-test, forbidden dependency/credential fallback, one-package/one-binary/one-AgentLoop, `git diff --check`, secret-pattern, fixed `codex/` and schema-openness scans pass before the local checkpoint.

### Removed and Deferred

- Removed test-only public projection access, unconsumed convenience APIs, a fake post-execution failure stage, a redundant wrapper and dead Tool description inputs instead of preserving future placeholders.
- Deferred exactly because their durable signals do not exist: unmatched Replay termination, Observation promotion, Replay promotion, Vision promotion and same-Turn Deferred activation counts. Architecture v1.4 optional `field_errors`/`receipt_ref` vocabulary remains design-only until a real production consumer exists.
- No Live Test, public Provider request, credential resolution, PR, tag or architecture remote seal occurred. The user-authorized implementation checkpoint `892edaa` was pushed directly to private `main`; its sole CI `33290961921` passed the macOS Rust Gate in 2m14s. This status-only correction uses `[skip ci]`. No file under `codex/`, Codex login state, `~/.codex` or `CODEX_HOME` was read or modified.

## F1d Lean Review

### Semantic Owners

- `src/provider_composition/config.rs` remains the sole Config v1 decode owner. Its private `InlineApiKey` masks `Debug`, and decode diagnostics redact an `api_key` source line before the error can reach CLI output.
- `src/provider_composition/validation.rs` remains the sole Provider configuration validator. It requires exactly one non-empty inline `api_key` or valid `CYBER_AGENT_*_API_KEY` `credential_ref` for every Provider before composition or network activity.
- `src/provider_composition/mod.rs` remains the only production Provider Composition Root. It converts an inline value into one private Provider-scoped in-memory `CredentialSource`; referenced credentials retain the injected external source. Both then enter the same Responses Adapter constructor and existing `ModelProvider` path.
- `src/adapters/responses/` and the one AgentLoop remain credential-form agnostic. Authorization construction, Model delivery, Tool execution, Session facts and RequestAudit were not given another credential branch or owner.

### Reuse, API and Dependencies

- Reused Config v1, `ProviderComposition`, `CredentialSource`, the protocol-first Responses Adapter, CLI `doctor`, CLI Chat and the existing Session/RequestAudit/Artifact boundaries. Existing referenced configurations remain valid.
- F1d adds no crate, Cargo feature, public Rust API, Provider binding, Port, Event, Artifact, Cache, persisted schema, fact source or AgentLoop. All new credential representation and composition types are private to `provider_composition`.
- The three touched production composition modules remain below the 300-line split-review signal. No Config Manager, Secret Service, Provider Registry/Router/Factory or vendor wrapper was introduced.
- Codex classification remains `REIMPLEMENT`; no Codex source, crate, credential, OAuth/session data or login state was read or copied. The fixed `codex/` reference tree remains clean.

### Tests and Verification

- Permanent regressions under top-level `tests/` cover inline/reference composition, both/neither/empty rejection, malformed-TOML redaction, `doctor` non-disclosure and loopback Chat proof that the inline Key alone reaches Authorization while stdout, stderr and `.cyber-agent` facts do not contain it.
- Full local Gate passes on Rust `1.93.1`: formatting, denied-warning Clippy, 312 discovered tests with 310 default-enabled tests passing and two opt-in Live Smokes ignored, plus locked build.
- One-package/one-binary, production inline-test, credential-like literal, generic Provider literal, single Responses owner, forbidden Codex dependency, one-AgentLoop/one-Composition, `git diff --check` and clean fixed `codex/` scans pass. No real API Key exists in the worktree.
- Live Test: `NOT_RUN`; F1d used only synthetic and loopback credentials. Git: F1a-F1d implementation commit `98e1094`, PR `#46`, CI `33182963979` and `[skip ci]` Squash Merge `6eb4959` are complete.

### Removed and Deferred

- No temporary Provider, Adapter, CLI or environment-variable-specific inline-key path remains; the implementation is an additive field and private source inside the existing composition path.
- File-permission enforcement, Keychain, encrypted Vault, credential manager, automatic config discovery and secret rotation remain intentionally absent for this single-user local tool. CI may continue using `credential_ref`; a personal local config may use `api_key` and remains user-owned uncommitted state.
- Provider Router/Registry/fallback, vendor-specific Responses wrappers, new Wire APIs and M4-S3/M4-S4 behavior remain deferred and unauthorized.

## F1c Lean Review

### Semantic Owners

- `src/provider_composition/` remains the only generic runtime Provider construction path. `chat` loads its explicit Config v1 and consumes the resulting `ProviderComposition`; it owns no URL, model, credential lookup or protocol Adapter construction.
- `src/cli_chat.rs` owns only line-oriented CLI interaction and the durable Chat Session handoff. It replays the authoritative Session stream before each append, uses the existing Project Writer and RequestAudit service, and assigns a new Session identity as `chat-<seconds>-<nanoseconds>-<pid>-<sequence>`.
- `src/app/agent_loop.rs` remains the one History, Context, Provider delivery, Tool pairing and stage-order owner. `src/app/tool_scheduler.rs` is execution-only: it returns the complete ordered real-outcome batch. AgentLoop appends every Tool Result, proves complete pairing, then invokes the receipt stage.
- The Project Event Store owns the narrowly scoped advisory Chat runtime lease. It is separate from Writer ownership and has no Event or Session-fact semantics.

### Reuse, API and Dependencies

- Reused the existing Config v1/Composition Root, AgentLoop, ProviderSession, Prompt Profiles, Context Planner, ToolRegistry, Project/Event Store, RequestAudit, History normalizer, Local Fragment Cache and Project Snapshot owners. No second AgentLoop, Provider Adapter, History, Tool, audit or fact-store path exists.
- No new crate, persisted schema, Port, public Rust API, Cache, Provider capability or credential convention was added. Existing `tokio` gained only the already-required `io-std` and `signal` features for interactive stdin and Ctrl-C; `ProjectChatRuntimeLease` and its acquisition method are `pub(crate)` internal composition infrastructure.
- Codex classification remains `REIMPLEMENT`; no Codex source, crate, credential or login state was read or copied. The fixed `codex/` reference tree remains clean.

### Removed and Deferred

- Removed the scheduler's former AgentLoop-specific receipt/error handling. Receipt-stage failure is now a permanent multi-Tool regression path with completed real outcomes retained in History.
- No speculative TUI, line editor/history UI, Web surface, command framework, Provider Router/Registry/fallback, Profile loader/DSL, database, M4-S3/M4-S4 behavior or Live Chat route was retained.
- `src/cli_chat.rs` exceeds the 300-line review signal because its cohesive durable Session composition requires the start/resume, audit, receipt and History handoffs to share one authoritative replay-before-append rule. Splitting it now would add an otherwise single-consumer coordinator without moving a separate semantic owner.

### Tests and Verification

- Permanent loopback regressions cover configuration agreement with `doctor`, initialized-Project enforcement, Session start/resume/Profile selection, History/Receipt/RequestAudit durability, provider failure after a receipt, concurrent same/different Project Chat processes, Ctrl-C, incremental rejection output isolation and cross-process Session IDs. The receipt-stage regression uses two real `read_file` calls and verifies their successful paired History results after the forced persistence failure.
- Final local Gate: `cargo fmt --all -- --check`, `cargo clippy --locked --all-targets --all-features -- -D warnings`, `cargo test --locked --all-targets --all-features`, and `cargo build --locked` all pass on the pinned toolchain. The suite discovers 308 tests: 306 default-enabled pass and two explicit Live Smokes remain ignored.
- Lean/static checks pass: no `src/` inline tests; Config-only generic Provider literals; one Responses request/transport/decoder owner; no forbidden Codex dependency; clean fixed `codex/`; no root runtime directory; and `git diff --check`.
- Git: no Git write or remote action was run. Live Test: one separately authorized synthetic DeepSeek Chat passed through Config v1, the protocol-first Responses Adapter and the one AgentLoop; no Codex credential, `~/.codex` or `CODEX_HOME` was read or changed.

## F1c DeepSeek Live Chat Result

- Authorization: one synthetic text Turn through the completed first-version CLI Chat, with no real target and an instruction not to invoke Tools.
- Configuration: Config v1 selected `deepseek-v4-flash`, the explicit DeepSeek Responses Base URL, `binding = "http_sse"`, `wire_api = "responses"`, the exact `CYBER_AGENT_PRIMARY_API_KEY` reference and a conservative explicit Full-only text Capability Mapping. `doctor` reported the selected profile ready without reading the credential.
- Runtime result: `PASS`; the real service returned `LIVE_CHAT_OK` with process exit code 0 through `ProviderComposition -> ResponsesHttpAdapter -> AgentLoop -> CLI Chat`.
- Durable result: the isolated Session contains exactly one `session.started`, one `session.request_audit_recorded` and two ordered `session.history_item_recorded` facts for the user and assistant text. `prompt inspect` reconstructed the Full request from 16 components with Adapter version `responses-http-sse-v1`; metrics report one Full request, zero Incremental requests, zero fallback and zero Tool cache hits.
- Credential handling: the user-supplied value was placed only in a mode-0600 credential file inside one mode-0700 `/tmp/cyber-agent-deepseek-chat.*` directory, injected into the Chat child process as the exact configured environment reference, never stored in Provider TOML, repository files, Session facts, RequestAudit Blobs or command output, and removed with the complete temporary directory after verification.
- Isolation: no Codex login state, `~/.codex`, `CODEX_HOME`, real target, Vision request, Git operation or GitHub Actions run was accessed or performed.

## F1b DeepSeek Live Smoke Result

- Authorization: one fixed synthetic Primary request using the completed generic Composition Root and Responses HTTP/SSE Adapter.
- Configuration: `deepseek-v4-flash`, explicit DeepSeek Responses Base URL, exact `CYBER_AGENT_PRIMARY_API_KEY` reference, text/stream/tool/usage enabled, image/incremental/structured output disabled and a conservative test-only context window.
- Credential handling: the user supplied a mode-0600 secret file inside one mode-0700 `/tmp/cyber-agent-deepseek-live.*` directory. The test command read it only into the child process environment; it was never printed, copied into Config/repository/logs or sourced from Codex state. The complete temporary directory was removed after the attempt.
- Scope: no real target, Tool execution, F1c Chat, Vision request, Git operation or GitHub Actions run.
- `doctor`: PASS; Config v1 composed the configured Primary profile as ready without resolving or displaying the credential.
- Primary Live Smoke: PASS; Cargo reported 1 passed, 0 failed in approximately 0.99 seconds. Authentication, configured model and Base URL, Responses streaming text, Usage, Completed and the shared decoder path were exercised.
- Isolation: no Codex login state, `~/.codex` or `CODEX_HOME` was read or modified; no Git operation or GitHub Actions run occurred.

## F1b Slice Start

- Base: remote-sealed `08e9025` plus the locally complete, uncommitted F1a worktree.
- Authorized scope: extract one shared Responses request/HTTP-SSE/response owner; remove vendor-named production bindings; decode fixture-proven reasoning-text events; require explicit Capability Mapping for Full-only services; reuse the one Composition Root, generic Live Smokes and ModelProvider contract behavior.
- Configuration contract: Config v1 is the only Provider/model selection authority. No `*_MODEL` or `*_LIVE_MODEL` environment variable is introduced; `credential_ref` selects only the lazily read secret value.
- Intended tests: shared request/transport/decoder regression, reasoning-text/Tool/Usage/Completed/unknown-event fixtures, exact credential isolation, Full-only capability validation, differently configured services through the same binding and unchanged existing Responses behavior.
- Acceptance: full locked Cargo/Lean Gate plus static proof of one Responses transport/decoder, no direct generic Adapter construction, no Provider URL/model environment literal in generic runtime paths and a clean fixed `codex/` tree.

## F1b Lean Review

### Semantic Owners

- `src/adapters/responses/mod.rs` is the sole Responses HTTP/SSE transport and lazy credential-resolution owner. `request.rs` owns the only Responses request DTO/encoding path and `response.rs` owns the only event decoder/error mapping; no vendor wrapper or dialect registry exists.
- `src/provider_composition/` remains the only runtime construction path. The fixed implemented pair is `binding = "http_sse"` plus `wire_api = "responses"`; URL, Model, exact `CYBER_AGENT_*_API_KEY` reference and Capability Mapping are data and never trigger a Provider-name branch.
- `domain/model::ModelEvent::ReasoningDelta` is the provider-neutral reasoning event. `app/model_sample` deliberately excludes it from assistant output and Conversation History, while the Trace semantic-output rule requires actual assistant text or a Tool Call rather than reasoning alone.
- ADR-0007 supersedes vendor-specific binding policy without modifying frozen Architecture v1/v1.1. It requires fixture evidence for any later wire extension and forbids a vendor binding based only on name, URL, Model or credential.

### Reuse, API and Dependencies

- The existing `reqwest`, `eventsource-stream`, Serde, ModelProvider Port, ModelInputIR, explicit Capability Mapping, AgentLoop, Project Artifact resolver, Composition Root and generic Live Smokes are reused. No Cargo dependency, Feature, Provider SDK, Event/Artifact/Cache/fact schema, second AgentLoop or service was added.
- Public `OpenAiResponsesAdapter`/`OpenAiResponsesConfig` are replaced by the protocol-named `ResponsesHttpAdapter`/`ResponsesHttpConfig`; their production consumer is the Composition Root and their direct test consumer is the permanent protocol Contract Suite. The only new provider-neutral public enum variant is `ModelEvent::ReasoningDelta`.
- Config v1 replaces the unsealed F1a value `binding = "openai"` with `binding = "http_sse"`; `wire_api = "responses"` and all other fields remain strict. The schema version does not change because Config v1 had not been remotely sealed or released.
- The configured credential reference may name any dedicated `CYBER_AGENT_*_API_KEY` variable and selects only the secret value. There is no hard-coded OpenAI/DeepSeek credential constant, provider URL, model default, `*_MODEL` environment variable or cross-reference fallback.
- Codex classification remains `REIMPLEMENT`; no Codex source, crate, credential or login state was read or copied, so the Upstream Port Ledger and dependency set are unchanged.

### Removed and Deferred

- Removed the vendor-named `adapters/openai` production module and its default credential constant. The 760-line F1b-start request/transport/decoder implementation became a 736-line `adapters/responses` owner before the small provider-neutral Reasoning integration; no parallel DeepSeek Adapter was created.
- Renamed the permanent wire suite/fixtures from `openai` to `responses`, retained every existing request, Tool, image, incremental, failure and cancellation regression, and added rather than replacing reasoning/Full-only coverage.
- Removed the planned thin OpenAI/DeepSeek binding split and vendor-specific Live-Test key instruction. Deferred Chat Completions, automatic protocol/capability discovery, Provider Router/Registry/fallback, vendor tuning, F1c Chat and M4-S3/M4-S4.

### Tests and Verification

- The protocol fixture covers reasoning text, Tool Call, assistant output, Usage and Full-only Completed without a response ID; malformed reasoning and unknown events are typed failures. Existing request JSON, image, tool pairing, continuation fallback, cancellation and Provider failure tests remain green.
- Composition tests prove differently named URL/Model/credential profiles use the same Adapter, a Full-only map rejects incremental delivery before credential/network access, and only the exact configured credential reference is read lazily. AgentLoop and Trace tests prove reasoning never becomes assistant History/output or a semantic replay sample by itself.
- Full local Gate passes on Rust `1.93.1`: formatting, denied-warning Clippy, 298 discovered tests with 296 default-enabled tests passing and two generic Live Smokes ignored, plus locked build.
- Static scans pass for production inline tests, Provider URL/model environment literals, vendor Adapter symbols, exactly one Responses request/transport/decoder owner, direct construction limited to the Composition Root and protocol Contract Suite, credential/Codex isolation, forbidden Codex dependencies, dependency direction, one AgentLoop/no Provider Router, secret-like literals, `git diff --check` and clean fixed `codex/` state.
- Live Test: `PASS`; after F1b implementation completed, a separately authorized fixed DeepSeek Primary smoke passed through the generic Config v1 Composition Root and Responses HTTP/SSE Adapter. The temporary credential/configuration directory was removed afterward; no Codex login state, `~/.codex` or `CODEX_HOME` was read or changed.
- Git: no branch, commit, push, PR, merge, tag, release or GitHub Actions run was created; F1a/F1b remain uncommitted over remote base `08e9025`.

## F1a Slice Start

- Base commit: `08e9025`; worktree was clean and `main` matched `origin/main` before the Slice start.
- Intended production touch points: one strict configuration schema and one top-level Provider Composition Root; the existing OpenAI Responses binding gains an explicit configured credential reference; `chat` and `doctor` load the same configuration path without resolving credentials; Live Smokes receive the composed Provider from that path.
- Configuration contract: explicit schema version; named Provider and Agent profiles; required base URL, model, dedicated `CYBER_AGENT_*_API_KEY` reference and versioned capability mapping; exactly one Primary and at most one optional Vision profile; no model-name capability inference.
- Scope budget: no new Cargo dependency, dynamic Registry, general Router, fallback, second AgentLoop, persisted Event/Artifact/Cache, DeepSeek binding, shared-wire extraction without a second implementation, live request or M4-S3/M4-S4 behavior.
- Intended permanent tests: strict parse/unknown-field/reference/capability/credential validation; primary/vision composition; credential laziness and exact-reference resolution; `doctor` and shell-only `chat` use the same config; both ignored Live Smokes have no Provider URL/model environment construction.
- Acceptance commands: locked fmt/clippy/test/build Gate; production inline-test, generic-runtime Provider literal, direct Live Smoke Adapter construction, forbidden Codex dependency, credential isolation, dependency-direction, one-AgentLoop/no-general-router, `git diff --check` and clean fixed `codex/` scans.

## F1a Lean Review

### Semantic Owners

- `src/provider_composition/config.rs` is the sole Config v1 decode and Domain-profile compilation owner. It defines fixed implemented `binding`/`wire_api` enums, named Provider/Capability/Agent profile references and conversion to the existing `AgentModelProfile`/`ModelCapabilities` contracts.
- `src/provider_composition/validation.rs` is the sole Config v1 validation/error owner. It rejects unknown fields/schema/bindings, malformed identifiers/URLs/credential references, zero generations/windows, dangling references and invalid active or inactive Primary/Vision capability compositions before network activity.
- `src/provider_composition/mod.rs` is the only concrete Provider Composition Root. CLI `chat`, CLI `doctor` and both generic Live Smokes load this same path; only this module creates a production OpenAI Responses Adapter.
- `src/adapters/credentials.rs` is the one Provider-neutral credential-source owner. The Adapter receives the configured reference and resolves it lazily at `ModelProvider::stream`; static composition and diagnostics do not read a key.

### Reuse, API and Dependencies

- Reused existing Serde/TOML, `reqwest::Url`, `AgentModelProfile`, `ProviderProfile`, explicit `ModelCapabilities`, `ModelProvider`, OpenAI Responses Adapter and App doctor. No Cargo dependency, Feature, Event, Artifact, Cache, database, remote service, AgentLoop or fact source was added.
- New public contracts are limited to `CredentialSource`/`CredentialSourceError`/`ProcessEnvironmentCredentialSource` and `ProviderComposition`/`ProviderCompositionError`. They have current OpenAI Adapter, CLI, Primary/Vision composition and generic Live Smoke consumers. Raw Config structs, fixed binding/wire enums, validation and schema version remain internal.
- `OpenAiResponsesConfig` now receives the exact credential reference selected by composition instead of owning an unconditional lookup. The official OpenAI default constant remains only a binding/contract-test convenience and is absent from generic runtime selection.
- Codex classification is `REIMPLEMENT`; no Codex source, crate, login state or implementation was read or copied, so the Upstream Port Ledger is unchanged.

### Removed and Deferred

- Removed the OpenAI-owned credential trait/error implementation, two direct Live Smoke Adapter constructions, two hard-coded official endpoints and the vendor-specific live-model environment variable. Retained one direct Adapter constructor only in the OpenAI offline contract suite, where the Adapter itself is the subject.
- The initial 458-line combined module was split after line-count/ownership review into a 96-line Composition Root, 162-line decode/compile owner and 226-line validation/error owner. No `common`, generic Factory, Registry, Router, Manager or Plugin abstraction exists.
- Deferred exactly as scoped: DeepSeek binding, shared Responses wire extraction before the second binding, reasoning-text event support, OS config-directory discovery, real Live Test, first-version Chat AgentLoop composition and M4-S3/M4-S4 behavior.

### Tests and Verification

- `tests/provider_composition.rs` covers Primary/Vision composition, exact lazy credential references, credential failure isolation, explicit capability-over-model-name precedence, unknown fields/schema/binding, unsafe URL/reference, zero versions/windows, dangling references and invalid inactive profiles.
- `tests/cli_smoke.rs` proves `doctor` and shell-only `chat` accept/reject the same explicit config without the configured key and without using generic `OPENAI_API_KEY`. The two ignored Live Smokes use only `CYBER_AGENT_LIVE_CONFIG`, the common Composition Root and fixed synthetic inputs.
- Full local Gate passes on Rust `1.93.1`: formatting, denied-warning Clippy, 294 discovered tests with 292 default-enabled tests passing and two generic Live Smokes ignored, plus locked build. Production inline-test, credential-like literal, generic Provider URL/model-env, forbidden Codex dependency, Provider Registry/Router, one-AgentLoop, `git diff --check` and clean fixed `codex/` scans pass.
- Live Test: `NOT_RUN`; F1a had no live authorization and did not read the supplied DeepSeek key, any Cyber Agent Provider key, Codex login state, `~/.codex` or `CODEX_HOME`.
- Git: no branch, commit, push, PR, merge, tag, release or GitHub Actions run was created; the completed Slice remains uncommitted over clean remote base `08e9025`.

## M4-S2 Slice Start

- Delivery branch: `agent/m4-s2-trace-replay-eval`
- Base commit: `fea1433`
- Intended production touch points: one pure Trace/Eval contract beside the existing request-audit and Session facts; fixed Replay Model and Tool adapters consuming validated trace steps; one narrowly named App composition for Mechanical Replay, Context Recompile and explicitly triggered Shadow Model Eval. Reuse the existing ModelProvider/ToolExecutor Ports, AgentLoop, Context Planner, RequestAudit reconstruction, Session History/Receipts and Artifact Blob authority.
- Scope budget: no second Trace/Event/Blob/Research/Memory store, no Eval database or dataset service, no Provider router/fallback, no new Provider wire protocol, no general Hook/Manager/Registry, no second AgentLoop and no M4-S3 Observation/TargetDelta or M4-S4 CLI Chat composition. Replay/Shadow results remain isolated derived data and cannot satisfy Finding evidence.
- Intended permanent tests: strict TraceFixtureManifest validation against exact Session/Event/Artifact/Receipt references; deterministic Mechanical Replay with zero network and zero real Tool side effects; immediate `UNMATCHED_TOOL_CALL`; Context Recompile comparison without historical Manifest mutation; Shadow Eval with a real/Mock Provider and Replay-only Tools; proof that every Eval mode leaves formal Project Memory/Research/Event facts unchanged and labels outputs as non-evidence.
- Acceptance commands: locked Cargo fmt/clippy/test/build Gate; production inline-test, forbidden Codex dependency, credential isolation, Provider-wire/dependency-direction, one-AgentLoop/no-second-fact-store scans; `git diff --check`; clean fixed `codex/` check.
- Delegation: Luna xhigh performs the read-only contract/reuse audit; Terra xhigh implements only the approved vertical path; Luna xhigh performs final read-only review. The main agent owns contracts, corrections, Git and merge authority.
- Live Test: explicitly authorized for one bounded synthetic Shadow request only after offline gates. A dedicated Provider environment reference is required; no secret may be committed, printed, passed through Codex credentials or persisted in shell profiles. Provider incompatibility is recorded as a test result rather than bypassed through credential aliasing.

## M2-S5 Slice Start

- Delivery branch: `agent/m2-s5-file-writes-tool-cache`
- Base commit: `01f6782`
- Intended production touch points: `domain/tools` for typed Receipt/cache-reference and file-mutation audit contracts; the existing durable-file owner; fixed `write_file`/`apply_patch` Tool adapters; the existing scheduler/AgentLoop Receipt path; one project-isolated, content-addressed derived Tool Result Cache.
- Intended permanent tests: atomic write and patch success/failure evidence, whole-operation preimage validation, cache eligibility denial for every non-deterministic or mutating Effect, deterministic read-only hit/miss/corruption/deletion/project-isolation behavior, and one AgentLoop cache-hit path with a fresh Receipt linked to its source.
- Acceptance commands: the locked Cargo fmt/clippy/test/build Gate; production inline-test, forbidden Codex dependency, dedicated-credential and `codex/`-unchanged scans; `git diff --check`.
- Live Test: `OFF`; this Slice uses only temporary local projects, fixtures and Mock Providers.

## M3-S1 Slice Start

- Delivery branch: `agent/m3-s1-context-planner`
- Base commit: `206f142`
- Intended production touch points: a pure Context Fragment/Planner/Manifest owner in Domain, a project-isolated rebuildable Fragment Cache Adapter, the existing AgentLoop/Baseline integration and one minimal CLI `/context` inspection path.
- Intended permanent tests: fixed full-request order; stable include/truncate/artifact-ref/exclude decisions and Reason Codes; unchanged Fragment cache reuse; corruption/deletion rebuild; append-only Candidate correctness; and Context Recompile producing derived comparison data without mutating historical audit.
- Acceptance commands: the locked Cargo fmt/clippy/test/build Gate; production inline-test, forbidden Codex dependency, dedicated-credential, architecture-direction and `codex/`-unchanged scans; `git diff --check`.
- Live Test: `OFF`; this Slice uses only deterministic Domain tests, temporary local projects, Fixtures, Mock and Replay paths.

## M3-S2 Slice Start

- Delivery branch: `agent/m3-s2-world-state-mode`
- Base commit: `d4217d2`
- Intended production touch points: pure Section Full/Diff and capability-routing contracts in Domain; fixed Tool exposure/search and Mode/Profile composition in App/Domain; the existing Context Planner and ProviderSession integration; minimal `doctor` capability diagnostics.
- Scope budget: no new fact store, Provider protocol, model router, dynamic plugin registry, Hook registry or runtime Agent. Reuse existing Research/Project Snapshot, ToolSpec, ModelCapabilities, ModelProfile, Context Fragment and Baseline owners.
- Intended permanent tests: stable Section IDs/hashes/full-diff renderers and bounds; Direct/Deferred/Hidden visibility and `tool_search`; Deferred activation Baseline reset; Mode Prompt/Tool fingerprints; exhaustive four-mode Vision capability matrix; invalid forced modes before any Provider call; Turn-boundary profile switching and Primary/Vision session separation.
- Acceptance commands: the locked Cargo fmt/clippy/test/build Gate; production inline-test, forbidden Codex dependency, credential isolation, Provider-wire boundary, architecture direction, one-AgentLoop/no-general-router and `codex/`-unchanged scans; `git diff --check`.
- Delegation: one Terra xhigh implementation owner for the bounded vertical path, followed by Luna xhigh read-only review; the main agent owns scope decisions, fixes, Git and merge authority.
- Live Test: `OFF`; only pure Domain, Mock, Fixture and local integration paths are authorized.

## M3-S3a Slice Start

- Delivery branch: `agent/m3-s3a-session-resume`
- Base commit: `48c8cb1`
- Split rationale: the former broad M3-S3 crossed History, Session facts, derived Summary, audit Blob and CLI composition boundaries. It was split before production implementation into M3-S3a through M3-S3d; frozen architecture and fact ownership are unchanged.
- Intended production touch points: the existing `domain/history` pairing owner, one strict Session payload/Reducer beside the existing Event envelope, the existing Project Session stream commit boundary, and the ProviderSession reset path.
- Scope budget: no Summary, Compaction payload, RequestAuditManifest, audit Blob, CLI inspection composition, database, Provider protocol, general replay framework, image path, background service, Hook/Router or second runtime Agent.
- Intended permanent tests: Tool pairs normalize without split ownership; unknown/invalid Session Events and transitions fail; failed append never applies; replay is deterministic; Resume uses a fresh Primary ProviderSession and forces exactly the next request Full.
- Acceptance commands: the locked Cargo fmt/clippy/test/build Gate; production inline-test, forbidden Codex dependency, credential isolation, Provider-wire boundary, architecture direction, one-AgentLoop/no-second-History-owner and `codex/`-unchanged scans; `git diff --check`.
- Delegation: Luna xhigh performs a read-only contract/reuse audit before one Terra xhigh implementation owner; Luna xhigh then performs the final read-only review. The main agent owns scope, schema, fixes, Git and merge authority.
- Live Test: `OFF`; only pure Domain, Mock, Fixture and local Session Event integration paths are authorized.

## M3-S3b Slice Start

- Delivery branch: `agent/m3-s3b-session-compaction`
- Base commit: `3ebf112`
- Intended production touch points: extend the existing strict Session payload/Reducer boundary with one versioned Compaction fact and deterministic derived SessionSummary; reuse `domain/history` for normalized retention and the existing AgentLoop/ProviderSession reset path for `CompactionReset`.
- Scope budget: no RequestAuditManifest or audit Blob, CLI inspection composition, image/Vision path, Provider protocol, credential source, database, general Manager/Router/Hook, second Event Store, second History owner or second runtime Agent.
- Intended permanent tests: deterministic Summary rebuild and required vulnerability-research fields; retained complete Tool pairs and original Event preservation; ordinary Summary/Baseline separation; failed forced-Full retry retaining `CompactionReset`; successful forced Full clearing it exactly once before later incremental delivery.
- Acceptance commands: the locked Cargo fmt/clippy/test/build Gate; production inline-test, forbidden Codex dependency, credential isolation, Provider-wire and dependency-direction scans, one-AgentLoop/no-second-History-owner review, `git diff --check` and `codex/`-unchanged check.
- Delegation: Luna xhigh performs a read-only contract/reuse audit before one Terra xhigh implementation owner; Luna xhigh performs the final read-only review. The main agent owns scope, schema, corrections, Git and merge authority.
- Live Test: `OFF`; only pure Domain, Mock, Fixture and local Session Event integration paths are authorized.

## M3-S3c Slice Start

- Delivery branch: `agent/m3-s3c-request-audit`
- Base commit: `d6639c5`
- Intended production touch points: one pure versioned RequestAuditManifest owner in Domain; reuse the existing Context plan/ModelInputIR semantic owners, `ArtifactStore::publish_bytes`, strict Session Event stream and one App service that publishes content before durably appending the audit fact; one read-only reconstruction path over exact Blob bytes.
- Scope budget: no second Blob/Event Store, Artifact provenance record, raw HTTP snapshot, credential/transport capture, Provider wire change, inspection CLI, image/Vision path, database, general Manager/Router/Hook or second runtime Agent.
- Intended permanent tests: ordered exact reconstruction after source mutation; same-content SHA-256 dedupe; project isolation; missing/tampered Blob, unknown schema and reordered/duplicate reference rejection; Blob/Event failure atomicity; credential, Authorization and transport-metadata exclusion.
- Acceptance commands: the locked Cargo fmt/clippy/test/build Gate; production inline-test, forbidden Codex dependency, credential isolation, Provider-wire and dependency-direction scans, one-AgentLoop/one-Blob-Store review, `git diff --check` and `codex/`-unchanged check.
- Delegation: Luna xhigh performs a read-only contract/reuse audit before one Terra xhigh implementation owner; Luna xhigh performs the final read-only review. The main agent owns scope, schema, corrections, Git and merge authority.
- Live Test: `OFF`; only pure Domain, temporary local projects, Fixture and Mock paths are authorized.

## M3-S3d Slice Start

- Delivery branch: `agent/m3-s3d-inspection-cli`
- Base commit: `96385f1`
- Intended production touch points: one read-only App inspection projection over the existing ContextDecisionManifest, Project Snapshot/Cache outcome, Session facts and RequestAuditManifest; thin CLI command dispatch and stable human-readable output only.
- Scope budget: no new fact source, metrics store, Provider/Tool execution path, credential resolution, raw request snapshot, database, general Manager/Router/Hook, second Event/Blob Store or second runtime Agent.
- Intended permanent tests: empty/uninitialized project, no Session/request, normal and corrupt Session/audit reference paths; stable prompt/snapshot/metrics output; counts derived from existing facts; no writes, Provider calls, Tool calls or secret/transport fields.
- Acceptance commands: the locked Cargo fmt/clippy/test/build Gate; production inline-test, forbidden Codex dependency, credential/transport, dependency-direction and `codex/`-unchanged scans; `git diff --check`.
- Delegation: Luna xhigh owns the bounded implementation; the main agent owns contract decisions, corrections, Git and merge authority, with Terra review only if cross-layer risk remains after the local Gate.
- Live Test: `OFF`; this Slice uses only local Event/Blob fixtures and subprocess CLI tests.

## M3-S4 Slice Start

- Delivery branch: `agent/m3-s4-multimodal`
- Base commit: `1bdb19a`
- Intended production touch points: one pure image validation/budget contract; reuse the existing Artifact Blob and Lineage owners; one fixed `VisionAnalysisService`; one strict versioned Session route/observation fact; direct Primary image delivery and one scoped Secondary Provider path inside the existing AgentLoop authority.
- Scope budget: no second AgentLoop, autonomous vision Agent, general model router, Provider fallback, image-generation stack, video/audio, database, image cache service, dynamic plugin/hook registry or new remote service. Reuse existing ModelInputIR image items, AgentModelProfile/VisionMode resolver, ModelProvider Port, Artifact Store, Session Projection and RequestAudit service.
- Intended permanent tests: supported/corrupt/oversized inputs, deterministic budgeted derived image and Lineage, route matrix integration, direct path with zero Secondary calls, Secondary observation before Primary sampling, Primary/Vision identity isolation, uncertainty/evidence boundary, target re-analysis and zero-request replay.
- Acceptance commands: the locked Cargo fmt/clippy/test/build Gate; production inline-test, forbidden Codex dependency, credential isolation, Provider-wire/dependency-direction, one-AgentLoop/no-general-router and `codex/`-unchanged scans; `git diff --check`.
- Delegation: Luna xhigh performs the contract/reuse audit; Terra xhigh owns one bounded implementation after the audit; Luna xhigh performs final read-only review. The main agent owns scope, schema, corrections, Git and merge authority.
- Actions budget: local Gate is authoritative during implementation. No GitHub Workflow is triggered until the Slice reaches its final implementation PR, and all duplicate post-merge/status runs use `[skip ci]`.
- Live Test: `OFF`; only local image fixtures, Mock/Fixture/Replay Providers and loopback-free offline tests are authorized.

## M4-S1 Slice Start

- Delivery branch: `agent/m4-s1-research-profiles`
- Base commit: `b635645`
- Intended production touch points: add four fixed vulnerability-research Profiles beside the existing pure `domain/mode` owner and consume them through the existing AgentLoop Turn boundary; reuse `PromptComposition`, `ToolExposurePlan`, Research `TargetKind`/Finding statuses and Context fingerprints. Controlled-case contracts and fixtures remain tests-only until an authorized Eval runtime becomes their production consumer.
- Scope budget: no Prompt filesystem/TOML loader, dynamic registry, workflow DSL, Provider path, new Event/Blob/Cache store, Replay/Eval runtime, Observation/TargetDelta, external scanner, report generator or second AgentLoop.
- Intended permanent tests: exact four-Profile catalog and target compatibility; stable rendered fragments/Context fingerprints; evidence-stage language; positive/negative/tampered-or-environment control matrix; fixture integrity/tamper detection; and proof that evaluation setup performs no network, Tool, Provider, credential or project write.
- Acceptance commands: the locked Cargo fmt/clippy/test/build Gate; production inline-test, forbidden Codex dependency, credential isolation, architecture direction, one-AgentLoop/no-general-registry scans; `git diff --check`; clean fixed `codex/` check.
- Delegation: Luna xhigh performs a read-only contract/reuse audit; Terra xhigh implements only the approved vertical path; the main agent owns schema/API decisions, corrections, Git and merge authority.
- Live Test: `OFF`; only deterministic local fixtures and pure Domain/App composition tests are authorized.

## Slice Lean Review

### M4-S2 Lean Review

#### Semantic Owners

- `src/domain/trace_eval/manifest.rs` and `source_validation.rs` are the sole Trace fixture contract and source-reference validators. `TraceFixtureManifest` stores only exact Session/Event/Target/Artifact/Receipt references plus the full source-stream hash and closed invariant set; the existing Session and Research reducers remain authoritative for every referenced fact.
- `src/domain/trace_eval/plan.rs` owns the provider-neutral derived Replay plan and the distinction between complete historical `source_receipt` provenance and the side-effect-free `replay_receipt`. `result.rs` owns the closed `EvalSource`, Mechanical/Context/Shadow provenance and `NonEvidence` label; it has no conversion into Research Finding evidence.
- `src/adapters/replay/model.rs` and `tool.rs` are the only fixed Replay Port implementations. The Model adapter consumes exact Full semantic requests; the Tool adapter consumes exact call-ordered steps under current fixed exposure and has no real Provider/Tool fallback. The existing AgentLoop owns Tool planning, pairing, scheduling and the terminal `UNMATCHED_TOOL_CALL` boundary.
- `src/app/trace_eval/service.rs` is the sole read-only Trace preparation and Context Recompile service. It composes existing RequestAudit reconstruction, Session/Research projections and immutable Artifact Blob reads. `src/trace_eval_runtime.rs` is the only concrete Replay/Shadow wiring root and contains no Event, Research, Memory, Artifact or Project writer.

#### Reuse, API, Dependencies and Split Review

- Reused the existing one AgentLoop, ModelProvider/ToolExecutor Ports, Session Reducer and strict History pairing, RequestAudit reconstruction, Research reducer, ArtifactStore, ContextPlanner, Tool exposure and canonical BLAKE3/SHA-256 owners. Session gained crate-private Event-ID/Receipt projections only so Trace validation can reference existing facts without parsing payloads or creating another reducer.
- Public production growth is limited to the versioned Trace fixture, derived Replay/Eval result contracts, fixed Replay Adapters, App service and one concrete composition root. Each has a current Mechanical Replay, Context Recompile, Shadow Eval or integration-test consumer; internal source projections, plan parts and replay-only cancelled-output construction remain `pub(crate)`.
- No Cargo dependency, feature, persisted Event/Blob/Cache/configuration, Provider DTO, credential reference, database, remote service or second store was added. The implementation reuses the existing `sha2` dependency for required Artifact Blob verification and the existing canonical BLAKE3 owner for source identity.
- `service.rs` is 337 lines and was split-reviewed. Its single change reason is read-only preparation from validated facts; Artifact verification and Tool-step pairing remain private stages because extracting them would expose internal plans or create a service without another consumer. Split when a second Trace source or preparation consumer proves a distinct boundary. `manifest.rs` is 309 lines after source validation was already separated; it remains one cohesive versioned schema, accessors and structural validation owner. Neither file mixes filesystem/network/process I/O with Domain rules.
- Codex classification is `REIMPLEMENT`: no Codex source, test framework or workspace dependency was copied, so `UPSTREAM_PORT_LEDGER.md` and license notices are unchanged.

#### Test Layout and Verification

- Permanent regressions live in `tests/trace_replay.rs`; fixed source Events, audit Blobs, required Artifact bytes and manifest anchors live once under `tests/fixtures/m4_trace/`. The suite covers schema/reference/hash/range/order failures, fresh Session reduction equality, required Blob read/size/SHA-256 validation, Full-only capabilities, exact requests, fixed exposure, multi-Tool ordering, complete source versus derived Replay Receipts, cache-hit Receipts and complete step consumption.
- Mechanical Replay proves zero real Provider/Tool action; Context Recompile proves historical RequestAudit immutability; Shadow Eval uses a caller-supplied Provider with Replay-only Tools, terminates unmatched calls before later action and keeps every result `NonEvidence`. One synthetic OpenAI Shadow smoke remains ignored and requires both the existing live-test switch and dedicated Cyber Agent OpenAI credential path.
- Final local Gate passes on Rust `1.93.1`: formatting, denied-warning Clippy, 288 discovered tests with 286 default-enabled tests passing and two OpenAI Live smokes ignored, locked build, production inline-test, forbidden Codex dependency, credential/key, Provider-wire/dependency-direction, one-AgentLoop/no-second-store, `git diff --check` and clean fixed `codex/` scans. Terra xhigh implemented the bounded path; main review closed source identity, Receipt provenance, Blob validation, exposure, capability and ordering gaps; Luna xhigh returned final `APPROVE` with no unresolved P0/P1.

#### Removed and Deferred

- Rejected a second Trace/Event/Eval fact store, dataset database, general model router, Hook/Registry, real Tool fallback and independent pairing/hash/capability owners. Replay Plans and results remain in-memory derived values, and historical Receipts are never rewritten as current execution effects.
- Removed or corrected the earlier source-session-only Eval identity, derived Receipts that could imply historical Artifact/cache/file mutations, trust in Artifact metadata without reading the Blob, Replay exposure that could consume hidden steps, incremental capability claims without continuation identity, mutable manifest field naming and unordered Receipt matching.
- Deferred exactly as scope-locked: persisted Eval datasets/results, incremental semantic Replay, alternate Provider Adapters, M4-S3 Observation/TargetDelta, M4-S4 CLI Chat/long-flow composition, real target research and live compatibility for the supplied non-OpenAI credential. A future Full/Incremental Trace format requires a new proven contract rather than silently widening v1.
- No M4-S3/M4-S4 production code, Prompt loader/DSL, general workflow engine, database, telemetry, cloud service, multi-Agent runtime, Codex source change or Codex credential/login access exists in this Slice.

### M4-S1 Lean Review

#### Semantic Owners

- `src/domain/research_profile.rs` is the sole fixed source/web/mobile/binary Profile catalog. It owns Profile ID, generation, Research `TargetKind` compatibility, versioned compile-time Markdown selection and the minimum Tool-exposure override. Firmware remains a Research target kind but has no M4-S1 Profile.
- `src/domain/mode.rs` remains the sole Mode/Prompt composition owner. Mode generation now participates in the fragment ID, so an explicit generation change invalidates the existing Context identity even when prompt text is unchanged. Context ordering, rendering, budgeting and fingerprinting remain exclusively in the existing Context Planner.
- The existing `AgentLoop` Turn-boundary configuration path consumes `ResearchProfile`; it applies one `ModePack`, preserves pending Base/Personal/Project prompts and reuses the existing Tool exposure/Baseline transition. No second loop, Profile registry, router or configuration manager exists.
- Controlled-case manifest/types/loaders live only in `tests/m4_research_profiles.rs`; the twelve synthetic files live only under `tests/fixtures/m4_profiles/`. They are test inputs, not Project/Research facts, runtime configuration or an Eval schema.

#### Reuse, API and Dependencies

- Reused `ModePack`, `PromptComposition`, `ToolExposurePlan`, `ToolRegistry`, Research `TargetKind`, the existing Candidate/Reproduced/Confirmed semantics, `ContextPlanner` and one AgentLoop. Tool schema validation remains owned by the fixed Registry; no Tool schema or Finding validator was copied into Profile code.
- Public production growth is one fixed `ResearchProfile` enum plus the two AgentLoop selection entries. ID/generation/target compatibility and `ModePack` conversion are the bounded Profile contract; the AgentLoop is the current runtime consumer. Long prompt text stays in four versioned Markdown resources loaded with `include_str!`, with no runtime filesystem or TOML loader.
- Main review deleted the independent Profile BLAKE3 fingerprint because it had no production consumer. Stable golden tests now use the existing Context Planner fingerprint with the Profile's actual direct Tool schemas; canonical hashing still has one owner.
- No dependency, Cargo feature, Event, persisted schema, Cache, Port, Provider DTO, credential source, Adapter, database or remote service was added. Codex classification is `REIMPLEMENT`; no Codex source or implementation was copied, so the Upstream Port Ledger is unchanged.

#### Test Layout and Verification

- `tests/m4_research_profiles.rs` permanently covers the exact four-item catalog, Firmware exclusion, domain-specific prompt language, explicit `research_update` and evidence boundaries, stable generation-bound fragments, actual AgentLoop consumption, efficient command/image Tool exposure and stable unique Context fingerprints.
- Twelve tests-only fixtures provide one positive, one negative and one tampered/environment control per Profile. Their content contains comparable synthetic source, HTTP, mobile or binary inputs rather than labels alone. Positives collectively cover Candidate, Reproduced and Confirmed; all controls are `NoFinding`; the Confirmed fixture binds target version, environment, reproducible impact and positive/negative case references.
- Fixture paths and case IDs are unique, relationships stay inside the same Profile/Target, per-file SHA-256 is verified and the manifest has a fixed test-code SHA-256 anchor. In-memory byte tampering fails before decode. No fixture test calls a Provider, resolves a credential, executes a Tool, initializes a project, accesses a real target or uses public network.
- Final local Gate passes on Rust `1.93.1`: formatting, denied-warning Clippy, 266 offline/local tests plus one ignored existing synthetic Live Smoke and locked build. Production inline-test, forbidden Codex dependency, credential, architecture/general-registry, `git diff --check` and clean fixed `codex/` scans pass. Exact implementation head `b9cb3f9` passed the macOS PR-head Gate in run `32885992788`; `[skip ci]` Squash Merge `7631709` intentionally produced no duplicate main run. Terra xhigh implemented the bounded path; main review corrected Profile composition, controlled-case quality, Tool exposure and unused hash/API growth; Luna xhigh returned final code-level `APPROVE` with no unresolved P0/P1.

#### Removed and Deferred

- Did not add the proposed production ControlledCase contract or Loader because M4-S1 has no Eval runtime consumer. The controlled-case contract is intentionally tests-only; M4-S2 may promote only the minimum proven fields if an authorized Replay/Eval runtime genuinely consumes them. Main review removed only the unconsumed Profile metadata fingerprint API.
- Removed redundant default Tool overrides, duplicate target compatibility mapping, duplicated AgentLoop builder/queue composition and the mutable manifest-digest sidecar. Source alone defers the low-frequency image Tool; Web/Mobile/Binary preserve direct multimodal access, and all Profiles preserve direct command access.
- Deferred exactly as scope-locked: runtime Profile CLI/configuration, Prompt filesystem/TOML loading, TraceFixtureManifest, Replay/Shadow Eval, ObservationEnvelope, TargetDelta, external scanners, report generation, real targets, Live Provider compatibility and long-flow acceptance.
- No M4-S2 implementation, Provider/network request, credential resolution, Codex login access, new fact store, general Hook/plugin/Prompt DSL, workflow engine or multi-Agent runtime exists in this Slice.

### M3-S4 Lean Review

#### Semantic Owners

- `src/domain/image.rs` is the sole pure owner of accepted image formats, original-input limits, delivery budgets, validated metadata and aspect-ratio-preserving target dimensions. `src/adapters/image.rs` alone owns filesystem inspection, signature/decoder validation, pre-decode dimension enforcement, resize and deterministic PNG bytes; it never publishes facts itself.
- `src/adapters/tools/view_image.rs` is the only fixed local-image Tool. It reuses the existing Artifact Tool publication boundary for original/derived SHA-256 Blobs and Research Events, emits the fixed typed selection shape and never gives the ordinary generic Artifact Tool image semantics. `domain::vision::ViewImageSelection` is the single strict bridge from that successful Tool Result into model follow-up.
- `src/domain/vision/facts.rs` owns Vision Route, Observation, Failure, identity and evidence-boundary validation. `src/domain/session.rs` remains the only Session Event/reducer owner and enforces Route -> RequestAudit -> exactly one Observation/Failure, source/Profile/Capability/request binding, terminal uniqueness and Primary/Vision ProviderSession isolation.
- `src/app/vision/analysis.rs` owns one Full, no-tool, structured Secondary request and raw-reply publication. `session.rs` owns append-before-apply route/audit/terminal composition; `replay.rs` owns read-only Observation/Blob replay. `InitialImagePrimaryContext` and the fixed post-`view_image` stage are opaque typed stages inside the existing AgentLoop, not a Hook registry or another Agent.
- The OpenAI Responses Adapter remains the sole OpenAI wire owner. Its project resolver replays strict Artifact facts and reads their verified immutable Blobs; request encoding alone creates PNG/JPEG Data URLs and the minimal `json_object` output configuration.

#### Reuse, API and Dependencies

- Reused `ModelInputIR`, explicit `ModelCapabilities`, the M3-S2 Vision resolver/Profile contracts, one `ModelProvider` Port, `ConversationHistory`, RequestAuditService, SessionProjection, Research/Artifact reducers, LocalArtifactStore and the fixed Tool Registry. No second Capability map, Artifact/Blob/Event Store, History owner, AgentLoop, model router, retry/fallback policy or evidence type was introduced.
- Public contract growth is limited to the image Domain contract, strict Vision facts, scoped Vision App composition/Replay services, the opaque initial-image/fixed-follow-up AgentLoop entries and the OpenAI project image-resolver constructor. They have current Direct, Secondary, Tool-follow-up, Replay and Adapter consumers; test access did not create a generic extension interface.
- ADR-0005 accepts manifest dependency `image` 0.25 with default features disabled and only `png`/`jpeg` enabled (locked to 0.25.10), plus `base64` 0.22 for Adapter-only Data URLs. No SDK, multimedia stack, system image command, feature flag, Provider credential, configuration service or remote transport was added.
- Persisted growth is exactly Vision fact schema v1 and `vision.route_decided`, `vision.observation_recorded`, `vision.analysis_failed` in the existing Session stream. Raw model JSON is an immutable Blob; Observation remains a derived, unverified model statement and cannot satisfy a Research Finding evidence contract.

#### Test Layout, Split Review and Codex Classification

- Permanent regressions live in `tests/m3_multimodal.rs`, `tests/view_image_tool.rs`, `tests/vision_flow.rs` and the existing OpenAI/AgentLoop integration suites. They cover PNG/JPEG, unsupported/corrupt/symlink/byte/dimension/pixel boundaries, pre-decode oversized-header rejection, deterministic byte-budget retries and Lineage; real project-isolated Data URLs; Direct/Secondary non-omission and non-duplication; route/audit/terminal ordering; identity isolation; Provider failure; targeted re-analysis; malformed/reused facts; and zero-write/zero-request replay with missing, corrupt and mismatched raw Blobs.
- Production `src/` contains no inline test module. The final full Gate on Rust 1.93.1 passes formatting, denied-warning Clippy, 261 offline/local tests with one existing opt-in Live Smoke ignored, locked build, forbidden-dependency/credential/general-router/second-AgentLoop scans, `git diff --check` and the clean fixed `codex/` reference check. The exact implementation head `c8158c3` passed the macOS PR-head Gate in run `32881009122`; the `[skip ci]` Squash Merge `35753a3` intentionally produced no duplicate main run.
- Image/Vision patterns in Codex remain `REFERENCE`; the bounded local contracts, adapters and tests are independent `REIMPLEMENT`. No upstream line, crate, workspace dependency or test framework was copied, so no Upstream Port Ledger or license update is required.
- `analysis.rs`, `session.rs` and `facts.rs` remain slightly above the 300-line review signal after being split by request execution, durable composition/replay and persisted-fact ownership. A further mechanical split would separate validation from the fact/request it protects; the next split trigger is a genuinely different reply contract, event version/upcaster or replay consumer, not line count alone.

#### Removed and Deferred

- Main review removed the first-encode byte-budget early return, manually assembled event-only tests, constructible Primary image context, unchecked Primary/Vision Profile/Capability combinations, reusable scoped Vision Sessions, replay that trusted only projected Observation fields and test-only image resolvers. It corrected the `view_image` output schema, made project Artifact lookup real and added header-first limit rejection before full pixel decode.
- Deferred exactly as scope-locked: Live Provider compatibility, a user-facing configured Profile/credential composition for the CLI, vulnerability Mode Profiles and controlled samples, Trace Replay/Shadow Eval, TargetDelta and long-flow acceptance. The current OpenAI structured contract is intentionally the smallest `json_object`; richer JSON Schema negotiation requires measured Provider behavior and a later decision.
- The project resolver currently replays the small local Research stream per encoded image, and image preparation/publication remains synchronous. These are explicit single-user prototype costs, not justification for a database, image cache service, detached worker or general resolver abstraction before profiling.
- No M4 implementation, Live Test, real key resolution, public Provider request, Codex login read, image generation, audio/video, Provider fallback, database, dynamic plugin/hook or remote service exists in this Slice.

### M3-S3d Lean Review

#### Semantic Owners

- `src/app/inspection_{prompt,snapshot,metrics}.rs` own only deterministic report projections for their individual CLI views. They consume validated existing facts and do not own a second Session reducer, request model, Snapshot lifecycle, metrics store or generic inspection registry.
- `src/cli_inspection.rs` is the filesystem composition boundary: it discovers one existing Project, selects one exact Session, replays the existing Session projection and supplies existing Snapshot/Blob facts to the App projections. It has no Provider, Tool execution, credential or write path.
- `RequestAuditService::reconstruct` remains the sole App-level binding and exact Blob reconstruction path. The new CLI calls it rather than copying binding or content validation. The Context Planner's existing estimator remains the one conservative Token-estimation owner.
- `LocalProjectSnapshot::inspect_read_only` owns current cache validation without persistence. It reuses the existing fingerprint, Tool capability and complete-cache validators; historical `rebuild_reason` cannot turn a currently valid cache Hit into a false current Incremental Refresh.

#### Reuse, API and Dependencies

- Reused `SessionProjection`, RequestAuditManifest, ArtifactStore, RequestAuditService, Project Snapshot/cache validators, fixed Tool specs, Baseline reason codes and Session counters. No request/metrics/Event/Blob/Cache duplicate, Provider-specific DTO or second Token formula was added.
- Public API growth is limited to two CLI error variants and the command surface. Inspection report types and projection modules remain crate-private; test access did not widen Domain, Port or Adapter contracts.
- No Cargo dependency, feature, Provider wire type, credential reference, persisted schema, Event type, configuration source or remote transport changed. The command output is a derived JSON view and is never treated as an authoritative fact.

#### Test Layout, Split Review and Codex Classification

- Production `src/` contains no inline test module. Eight permanent subprocess regressions live in `tests/inspection_cli.rs` and `tests/inspection_audited_cli.rs`; the full repository Gate passes 244 offline/local tests with one existing synthetic Live Smoke ignored.
- Prompt, Snapshot and Metrics projections are separate narrow modules because their output contracts and fact inputs change independently. The shared module contains only immutable report inputs and the calls into the existing estimator; no `utils`, Manager, Registry or speculative Trait was introduced.
- Codex prompt inspection and local metrics behavior remain `REFERENCE`; all local composition is `REIMPLEMENT`. No upstream line, crate or test framework was copied, so no Upstream Port Ledger or license change is required.

#### Removed and Deferred

- Removed redundant `inspect` aliases, eager loading of unrelated Session/Blob/Snapshot facts, duplicated RequestAudit binding validation, lexicographic multi-Session guessing, historical incremental-build/current-Hit conflation and acceptance of symlinked Session Event files.
- Deferred exactly as scope-locked: image validation/direct delivery, VisionAnalysisService, Vision route/observation facts and replay to M3-S4; vulnerability Profiles, Trace Replay/Shadow Eval, Observation/TargetDelta and long-flow validation to M4. No implementation for those Slices exists in this branch.
- Retained prototype-exit gates: the standalone `chat` command still has no formal Project/Session/Provider composition, and `doctor` still has no configured Profile source. Before any Live Test, one CLI runtime must compose `run_turn_audited`, real History Session facts and dedicated Cyber Agent credentials without adding a second AgentLoop or reading Codex login state.

### M3-S3c Lean Review

#### Semantic Owners

- `src/domain/request_audit` is the only owner of Manifest schema v1, typed ordered component identity/source, BLAKE3 semantic identity, full-logical versus delivered-input identity, delivery mode/reason validation, exact canonical rendering and pure Blob-backed reconstruction. It does not own filesystem or Event I/O.
- The existing `ArtifactStore` remains the sole SHA-256 Blob authority. Its new `read_blob` Port operation returns bytes only after the local Adapter opens a regular file once and hashes the bytes read from that same handle; RequestAudit does not derive Blob paths or reimplement SHA-256 identity.
- `src/domain/session.rs` remains the sole Session Event/reducer owner. `src/app/request_audit.rs` only composes binding/Event-ID preflight, Blob publication and the existing `SessionProjection` append-before-apply boundary; Event append is the successful audit commit point.
- `src/app/agent_loop/request_audit.rs` is one fixed typed stage inside the existing AgentLoop. It maps the existing Baseline assessment and actual `ModelRequest` into one audit attempt before each Provider stream, including the bounded ProviderRejected Full fallback; it is not a Hook Registry or second loop.

#### Reuse, API and Dependencies

- Reused `ModelInputIR`, `ModelCapabilities`, `ModelIdentity`, `ModelProfile`, Baseline reason codes, canonical JSON/BLAKE3, existing Event envelope/SessionProjection, Project/Session typed IDs and the existing content-addressed Artifact Store. No second request model, capability map, canonicalizer, Blob/Event Store, History owner or Provider session was added.
- Public contracts are limited to validated RequestAudit construction/inspection/reconstruction types, the App service, the explicit audited AgentLoop entry and `ArtifactStore::read_blob`. Internal rendering/materialization, stage shape, Event-sequence lookup and Event-ID preflight remain crate-private; test access did not widen them.
- No Cargo dependency, feature, Provider DTO, credential source, raw HTTP snapshot, database, configuration service or remote transport changed. Persisted additions are only strict RequestAuditManifest v1 and `session.request_audit_recorded` payload v1.

#### Test Layout, Split Review and Codex Classification

- Production `src/` contains no inline test module. Permanent Domain/Port/App/AgentLoop regressions live in `tests/request_audit.rs` and the existing M3 integration suite; the full repository Gate passes 236 offline/local tests with one existing synthetic Live Smoke ignored.
- The 382-line pure Manifest module was split-reviewed and retained because persisted shape, construction and validation must evolve together; rendering, reconstruction, component-layout validation and errors are separate narrow modules. Existing large Session and AgentLoop parents only gained exhaustive event/control wiring, with the fixed audit stage isolated in a named child module.
- Codex request audit behavior remains `REFERENCE`; strict local Manifest/Blob reconstruction and fixed AgentLoop composition are `REIMPLEMENT`. No upstream line, crate or test framework was copied, so no Upstream Port Ledger or license change is required.

#### Removed and Deferred

- Removed cross-stream and already-applied Event-ID side effects before Blob publication, conflicting duplicate RequestAudit acceptance, separate hash/read file handles, generic callback placement in the main loop file and any claim that rendered components carry nonexistent Session Event provenance.
- Deferred exactly as scope-locked: read-only inspection composition to M3-S3d, image/Vision execution to M3-S4 and vulnerability Mode/Replay/Observation/long-flow work to M4. No implementation for those Slices exists in this branch.
- Retained P2 prototype-exit gates: production Provider composition must call only `run_turn_audited` with a real project-bound Session service; the compatible plain `run_turn` remains for non-persisted Core/Mock/Fixture use. Current components honestly identify rendered semantic sources because `ConversationHistory` has no per-item durable Event IDs; before prototype exit, the one formal Session composition must emit real `HistoryItemRecorded` facts and propagate their real provenance without adding a second History owner or fabricating IDs.

### M3-S3b Lean Review

#### Semantic Owners

- `src/domain/session/compaction.rs` owns structured Summary construction, validation, the bounded semantic-order-preserving representation, Summary and complete Compaction payload fingerprints, source/retention metadata and payload-shape validation. `src/domain/session.rs` remains the sole Session Event mapping/reducer owner and projects the latest Compaction without rewriting original History.
- `src/domain/history.rs` remains the only Tool Call/Result pairing and normalization owner. Its narrow recent-retention operation normalizes once, expands a requested suffix to include required Calls and revalidates the complete retained view; Session and AgentLoop do not copy pair discovery or repair.
- `src/app/agent_loop/compaction.rs` owns only binding an already-durable Compaction to the one Session/Primary runtime, full-payload idempotence and `CompactionReset`. `ProviderSession` remains the only Baseline/reset owner; Context Planner remains the only fragment ordering/budget owner.

#### Reuse, API and Dependencies

- Reused the shared Event envelope, strict Session reducer, append-before-apply `SessionProjection`, `ConversationHistory`, existing BLAKE3 semantic stack, typed TargetSnapshot/Artifact/Execution/Job/PTY IDs, `ContextFragmentSource::SessionSummary`, ProviderSession reset semantics and committed-delivery clearing path.
- New public contracts are limited to validated `SessionSummaryDraft`, immutable `SessionSummary`, typed `UnfinishedExecutionRef`, `SessionCompaction`, SessionState construction/inspection and the AgentLoop candidate/apply methods used by the current vertical path. Pair-retention and Context rendering helpers remain crate-private; test access did not widen them.
- No Cargo dependency, feature, Provider DTO, credential source, Artifact/Research schema, database, Event Store, History representation, general manager/router/hook or second AgentLoop was added. The new persisted surface is only strict `session.compaction_created` payload v1.

#### Test Layout, Split Review and Codex Classification

- Production `src/` contains no inline test module. Permanent Domain and runtime regressions live in `tests/session_compaction.rs` and the existing M3 integration suite; the full repository Gate passes 228 offline/local tests with one existing synthetic Live Smoke ignored.
- Summary/Compaction data and pure validation were split from the already-large strict Session event/reducer file, and durable runtime application was split from the AgentLoop control file. The parent files remain above the line-count review signal because their exhaustive existing event/control contracts change together; no second owner or speculative Trait was introduced.
- Codex Compaction and response-history behavior remains `REFERENCE`; pair-safe retention, strict payload replay and reset composition are local `REIMPLEMENT`. No upstream line, crate or test framework was copied, so no Upstream Port Ledger or license change is required.

#### Removed and Deferred

- Removed the first no-op retention draft that always kept index zero, Assistant-text-derived pseudo-facts, Summary-only idempotence, stringly TargetSnapshot/Artifact/execution references, unbounded/duplicate Summary entries, public test-only rendering/retention helpers and the mixed 1,000-line Session draft.
- Deferred exactly as scope-locked: RequestAuditManifest/Blob reconstruction to M3-S3c, inspection CLI to M3-S3d, image/Vision execution to M3-S4 and Replay/Observation/Eval to M4. No implementation for those Slices exists in this branch.
- Retained P2: the 8 KiB Summary limit is a conservative fixed local bound rather than a model-profile-derived budget, and active Job/PTY lifecycle facts are represented only by typed IDs in Summary. Measured prototype work may tune the bound without changing fact ownership.

### M3-S3a Lean Review

#### Semantic Owners

- `src/domain/history.rs` remains the only Tool Call/Result pairing and normalization owner. Runtime append, Session replay, Context validation and recovery all delegate to `ConversationHistory`; orphan removal, stable aborted repair, duplicate rejection and tool-name matching are not reimplemented elsewhere.
- `src/domain/session.rs` owns the exhaustive Session payload v1 vocabulary, lifecycle transitions, durable History projection, used-Primary identity set and typed Session errors. `domain/events` still solely owns the `cyber-agent.event` envelope, schema/upcasting, Event identity and sequence metadata; `domain/tools` still owns the published ExecutionReceipt payload.
- `src/app/session.rs` owns only validate-before-append and promote-after-success composition through the existing `EventSink`. The existing JSONL Adapter remains the sole durable Event Store; an already-applied duplicate is a no-write idempotent result and cannot desynchronize stream sequence.
- `ProviderSession` remains the only Incremental Baseline owner. `AgentLoop::resume_from_session` binds an already-durable Active/Resumed Session projection to one exactly matching fresh Primary identity, replaces volatile History and clears only runtime delivery state before the forced Full request.

#### Reuse, API and Dependencies

- Reused `ContentItem`, `ConversationHistory`, `ExecutionReceiptEventPayload`, the generic Event Reducer/Port, Project Session JSONL streams, `ProviderSession` reset semantics and existing AgentLoop Context compilation. No second transcript, pairing map, Event Store, Provider session, continuation or recovery format was added.
- New public contracts are limited to `SessionEvent`, `SessionState`, `SessionReducer`, `SessionProjection`, their typed errors, the History normalizer and the durable-state-bound AgentLoop resume entry. Replay-only mutation helpers and resumed-state predicates remain crate-private; direct unaudited runtime resume was removed.
- No Cargo dependency or feature changed. No Provider DTO, credential source, envelope schema, Artifact/Research fact, Cache, database or configuration service was added. Existing ExecutionReceipt shapes only reject unknown fields more strictly; their required/defaulted fields and version remain unchanged.

#### Test Layout, Split Review and Codex Classification

- Production `src/` contains no inline test module. Permanent regressions live in `tests/history_pairing.rs`, `tests/session_resume.rs` and the existing shared AgentLoop fixture; the full repository Gate passes 220 offline/local tests with one existing synthetic Live Smoke ignored.
- The 617-line pure Session module was split-reviewed and retained as one exhaustive schema/reducer boundary: event-type mapping, payload encode/decode, lifecycle transition and error attribution must change together. Filesystem append composition remains separately isolated in the 75-line App module, and no generic Session Manager or Journal was extracted.
- Codex History normalization remains `REFERENCE`; the small local pairing/repair algorithm is `REIMPLEMENT`. No upstream line, crate or test framework was copied, no forbidden dependency was added and no Upstream Port Ledger or license change is required.

#### Removed and Deferred

- Removed normalizer-only runtime call-order storage, direct resume that could bypass a durable `session.resumed` fact, public helpers used only by tests, duplicate Event re-appends that could strand sequence state, nested unknown-field acceptance and any reuse of an old Primary ID with a changed generation or later A-to-B-to-A cycle.
- Deferred exactly as scope-locked: derived SessionSummary/Compaction to M3-S3b, RequestAudit/Blob reconstruction to M3-S3c, prompt/snapshot/metrics CLI reads to M3-S3d, and all image/Vision execution to M3-S4. No implementation for those Slices exists in this branch.
- Retained P2 for M3-S3d: Resume preserves `last_context_plan` and `last_successful_context` as historical derived inspection values; the future CLI must label them as belonging to the previous ProviderSession or clear its active display. `StreamRecovered` also retains its pre-Session-v1 recovery payload without a nested payload version for M2-S1 compatibility.

### M3-S2 Lean Review

#### Semantic Owners

- `src/domain/world_state.rs` is the only owner of the fixed twelve vulnerability-research Sections, stable Section/hash semantics, bounded Full/Diff rendering and the Project Snapshot semantic projection. It derives from existing Research/Snapshot facts and cannot overwrite them.
- `src/domain/mode.rs` owns the four fixed Prompt sources and Mode Pack composition; the existing Context Planner remains the only owner of their ordering, rendered budget and fingerprints.
- `src/domain/tools.rs` owns `ToolExposure` and the bounded `ToolExposurePlan`; `src/adapters/tools/mod.rs` remains the one fixed catalog/execution owner, and `tool_search.rs` only searches and activates the currently Deferred subset.
- `src/domain/profile.rs` owns named Primary/optional Vision composition and the pure four-mode route decision. `ProviderSession` remains the only Baseline owner; `AgentLoop` applies queued Profile/Mode changes only at Turn boundaries.
- `src/app/agent_loop/context_state.rs` owns only the App-level assembly and safe State-Diff substitution. It calls the Context Planner's crate-private final-input budget check and never duplicates token estimation, Tool pairing or Provider compatibility rules.

#### Reuse, API and Dependencies

- Reused the existing Research reducer projection, Project Snapshot, Context Fragment/Planner/Cache, canonical Tool hashing, `ModelCapabilities`, `ModelProfile`, `ModelInputIR`, `ProviderSession`, History pairing and fixed Tool Registry. No second state store, request model, token estimator, Capability map, Registry or session implementation was added.
- New public Domain/App contracts are limited to World/Project State projections, Prompt/Mode composition, Tool Exposure data, Agent/Provider Profile and diagnostic report types, plus the AgentLoop setters/queue operations consumed by the current vertical path and permanent contract tests. The duplicate Result-only doctor wrapper was removed before Gate.
- No Cargo dependency, feature, Provider DTO, Event schema, authoritative Research/Artifact schema, fact source or configuration service changed. No credential reference or secret-resolution path was added.

#### Test Layout, Split Review and Codex Classification

- Production `src/` contains no inline test module. Nineteen M3-S2 regressions share one root fixture and three focused external modules; the full repository Gate passes 211 offline/local tests with one existing synthetic Live Smoke ignored.
- The AgentLoop's Context/State logic moved to its narrowly named child module. The 440-line pure WorldState module was split-reviewed and retained because its fixed Section projection and matching Full/Diff contract change together. The existing Tool Registry remains the single catalog owner; adding a second manager or registry would duplicate execution authority.
- Codex World State, Tool Exposure and deferred discovery remain `REFERENCE`; bounded research Sections, diff delivery, exposure search and profile composition are local `REIMPLEMENT`. No upstream source was copied, no forbidden dependency was added and no Upstream Port Ledger or license change is required.

#### Removed and Deferred

- Removed the unchecked cumulative Full-plus-Diff path, Exposure changes invisible to Baseline invalidation, model execution of schemas absent from the sent request, duplicate doctor validation wrapper and duplicated M3 test fixtures.
- Deferred exactly as scope-locked: History normalization, Compaction/Resume and RequestAudit persistence to M3-S3; local image validation/direct delivery, VisionAnalysisService, route events and VisionObservation to M3-S4; vulnerability Profiles, Replay/Shadow and Observation/TargetDelta to M4.
- Retained P2: the standalone CLI `doctor` still reports no configured Provider and does not yet compose a selected `AgentModelProfile`; Mode Pack filesystem/TOML/bootstrap loading is also absent. Both require the later CLI/Profile composition path and must be closed before the offline prototype is declared usable. No Live Provider request is needed for either.

### M3-S1 Lean Review

#### Semantic Owners

- `src/domain/context/planner.rs` and its narrowly named private modules are the only owners of fixed Context ordering, rendered-channel budgeting, selection, truncation, canonical Tool ordering, fingerprints, Full Logical Manifest and logical append-only comparison. `ContextPlan` cannot be deserialized into an invalid compiled state.
- `src/domain/context/canonical.rs` remains the one canonical Tool-schema and request-hash owner; the Planner reuses its ordered Tool representation instead of introducing a second normalization rule.
- `src/domain/history.rs` remains the only Tool Call/Result pairing owner. Context compilation validates externally assembled items through that owner and never repairs or drops an unsupported item silently.
- `src/app/context_compilation.rs` owns cache-aware Fragment resolution at the App/Port boundary; it distrusts arbitrary Port hits and revalidates each Fragment against its requested key.
- `src/adapters/fragment_cache.rs` owns the single local derived Fragment Cache representation and project-isolated durable path. `src/app/agent_loop.rs` remains the sole runtime consumer and `ProviderSession` remains the sole Full/incremental delivery authority.

#### Reuse, API and Dependencies

- Reused the existing `ModelInputIR`, `ModelCapabilities`, `ToolSpec`, `ConversationHistory`, Project writer/durable replacement, Serde, BLAKE3 and Canonical JSON stack. No second model input, pairing, hash, filesystem durability or Provider-session implementation was added.
- Fragment construction derives its own content hash and token estimate. Tool-schema order used for budgeting is the same canonical order sent to the Provider, and the final rendered request is rechecked against the budget.
- New public contracts are limited to the Context Fragment/Plan/Manifest inspection model, the narrow `ContextFragmentCache` Port, `LocalFragmentCache` Adapter and AgentLoop Context inspection/configuration path. The broader Context compilation visibility and standalone lowercase CLI status remain P2 cleanup candidates, not competing runtime semantics.
- No Cargo dependency, Cargo feature, Provider DTO, Event type, authoritative schema or configuration source changed. Fragment Cache schema v2 is explicitly derived and deletable.

#### Test Layout, Split Review and Codex Classification

- Production `src/` contains no inline test module. Permanent Domain, Cache, CLI and AgentLoop regressions remain under top-level `tests/`; 192 tests pass and the one existing synthetic Live Smoke remains ignored.
- The 383-line Fragment Cache Adapter was split-reviewed and retained as one cohesive persistence boundary: project path checks, private rebuildable DTO/checksum, durable store and typed Adapter errors change together and have no second consumer or reusable abstraction to extract. Large test files are permanent table/fixture coverage, not production modules.
- Codex Context behavior remains `REFERENCE`; ordering, budgeting, truncation, cache identity and manifests are local `REIMPLEMENT`. No Codex source was copied, no forbidden dependency was added and no Upstream Port Ledger entry or license change is required.

#### Removed and Deferred

- Removed caller-supplied token estimates, byte-count-only budgeting, UTF-8-unsafe truncation, unchecked deserialization of compiled plans/cache fragments, weak cache identity, mismatched Port-hit trust and failure-path promotion of successful Context inspection.
- Deferred exactly as scope-locked: ResearchWorldState/Project Diff production composition, Tool Exposure/Mode/Profile/Vision routing, RequestAudit persistence and offline historical recompile storage, Compaction/Resume, dynamic History Fragment caching, general Router/Hook/Plugin/database abstractions and Live Provider validation.
- Retained P2: project-level rather than Session-level Fragment Cache isolation, Base Prompt as the only current runtime cached Fragment, public Context compilation surface and lowercase standalone CLI status. Each has a real current consumer or harmless inspection impact; none blocks the M3-S1 contract or duplicates a semantic owner.

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

### M2-S4 Lean Review

#### Semantic Owners

- `src/domain/memory.rs` owns the fixed Memory path grammar, versioned Changed/ExternalChangeDetected revision contracts and the isolated pure Memory projection; Markdown remains content authority and Research state does not absorb Memory content.
- `src/adapters/project_memory.rs` owns confined UTF-8 Markdown reads, symlink rejection, the single 64-KiB `core.md` bound, BLAKE3 content identity and deterministic replacement Diff; Notes/Decisions/Failed Paths are not given a speculative storage cap. `project_event_store/durable_file.rs` remains the sole same-directory temporary-write/rename/fsync owner.
- `src/domain/research/reducer.rs` remains the strict `research.jsonl` envelope, Event-ID and replay owner. Memory is one typed, independently versioned ResearchEvent branch; unknown type/version/kind mismatch fails replay, while `MemoryState` is reduced separately.
- `src/domain/snapshot.rs` owns the serialized ProjectFingerprint, File Manifest, Project Snapshot and fixed Tool Capability Cache contracts. `src/adapters/project_snapshot/fingerprint.rs` owns both levels of Git/filesystem invalidation and `src/adapters/project_snapshot/mod.rs` owns cache generation validation and manifest/tool/snapshot publication order.
- The fixed `memory_read`, `memory_write` and `inspect_project` adapters own one synchronous Tool path each. They reuse the existing Tool Registry, Resource Keys, Receipt pairing and one project Writer lock; no background writer or second AgentLoop exists.

#### Reuse, API and Dependencies

- Reused `Project::open_writer`, the existing strict JSONL scanner/writer, `ResearchReducer`, `LocalArtifactStore`, `ArtifactStore`, fixed Tool Registry, canonical JSON, BLAKE3 and SHA-256 Blob identity. No crate, feature, fact stream, database, watcher, generic Manager or probe framework was added.
- `ArtifactStore::publish_bytes` is the only new Port method; it avoids a temporary caller-owned file while preserving the existing single staging/publication owner. Memory revision bytes and Diffs are durable Blobs before any Event can reference them.
- Git fingerprints use HEAD, stable porcelain-v2 status with `.cyber-agent`/`target` excluded, stable index size, dirty/untracked path type-size-mtime and control-fact metadata. A clean Git cache hit avoids a full source-tree metadata walk; a second edit while a path is already dirty invalidates correctly; non-Git projects use the documented sorted metadata fingerprint.
- Explicit `hash_paths` performs an incremental BLAKE3 refresh only for requested files without a valid cached Hash. Already cached unchanged Hashes are reused; ordinary metadata changes make the old Hash stale without reading every changed file; missing, non-regular and symlinked requested paths fail explicitly.
- Cache files carry schema, Project ID, source fingerprint, timestamp and reason; Manifest and Tool Capability files are durably replaced before the Project Snapshot completion marker, whose hashes bind both files. Mismatched generation/schema/project/content rebuilds instead of becoming fact.
- Exact duplicate Memory Event IDs retain Research idempotence without reapplying the revision chain, and a CAS-valid no-op `memory_write` emits neither Blob nor Event. The one `core.md` size check executes before Blob publication and is also enforced on external reads.

#### Test Layout and Scope

- Permanent top-level regressions cover first Memory creation, no-op writes, old/new Hash and Blob/Diff bytes, exact duplicate Event replay, CAS rejection, external edit audit, the bounded `core.md`/unbounded Notes distinction, path confinement, symlink rejection, unknown Event/payload version/revision-chain failure and project isolation.
- Snapshot regressions cover clean and cached-Hash hits, explicit incremental hash, stale-hash clearing, repeated dirty Git edits, deleted files, required-fact loss, corrupt/schema-incompatible/deleted Cache rebuild, Git cache self-exclusion, symlink non-traversal, missing requested paths, project isolation and a real AgentLoop Tool Call/Result/Receipt path.
- Production `src/` retains no inline tests. Final local Gate passes 148 offline/local tests with one existing synthetic Live Smoke ignored. Terra xhigh found and drove fixes for duplicate Memory replay and cached-Hash reuse; Luna xhigh returned final `APPROVE` with no unresolved P0/P1 after the `core.md` bound was narrowed to the design-required document only.

#### Removed and Deferred

- Main review removed automatic content hashing for every new/metadata-changed file, repeated hashing of an unchanged explicitly requested file, unstable Git-index mtime invalidation, full Git tree traversal on a cache hit, unconditional ToolSpec cloning for unrelated tools, unverified partial-Cache hash reuse, a blanket Memory-document size limit and unused public Memory projection API.
- The original mixed Snapshot Adapter exceeded 1,000 lines and was split by change reason into private cache lifecycle and fingerprint modules without a Trait or public API. Each retained module still exceeds the 300-line review signal but has one semantic owner. The 519-line Memory Tool module remains cohesive around one writer/replay/revision transaction; splitting it now would expose or duplicate the failure-atomic append/apply boundary.
- Deferred exactly as scoped: M2-S5 file writes/apply_patch/Tool Result Cache, Context Planner, Mode/Profile/Vision, Compaction/Request Audit, watcher, FTS/database/vector index, external tool version probing and Live Provider requests.
- Retained P2: deterministic injection of the narrow Markdown-replaced/Event-append-failed crash window, large Notes Diff compression, dirty submodule refinement and extreme same-size/same-mtime false negatives remain deferred. The next read repairs the crash window as an external revision, and no detached task can write after Tool completion.

### M2-S5 Lean Review

#### Semantic Owners

- `src/adapters/tools/file_write/{schema,patch,filesystem}.rs` own, respectively, the fixed single-file schemas and normalization, the deliberately small UTF-8/LF Unified Diff subset, and project-confined preimage/afterimage filesystem evidence. `src/adapters/tools/file_write.rs` is only the execution facade and never claims a multi-file transaction.
- `src/adapters/project_event_store/durable_file.rs` remains the sole same-directory staged-write, sync and atomic-rename owner; the file Tools reuse it and retain existing Unix permissions. `src/adapters/file_diff.rs` is the one complete replacement-Diff renderer shared by Memory and file-mutation audit, including explicit no-final-newline markers.
- `src/adapters/project_event_store/project.rs` remains the sole project Writer-lock owner and now explicitly unlocks before its descriptor closes. The status-seal Gate exposed that implicit close-based release was not deterministic enough on macOS; one permanent regression proves immediate reacquisition after `ProjectWriter` drop without adding a retry or second lock manager.
- `src/domain/tools.rs` owns cache eligibility, Tool Cache identity links, file-mutation Receipt facts and the single strict versioned `ExecutionReceiptEventPayload`. A Cache Hit has a fresh execution/plan Receipt whose `cache_source` is the only persisted source-link copy.
- `src/adapters/tool_result_cache.rs` owns the project-isolated content-addressed derived Cache. `tool_result_cache/session.rs` owns only its fixed Session Event append/validation path and short-lived Writer acquisition; it is not a general audit manager. `research_get` remains the only consumer because avoiding its JSONL scan, Upcast and Reducer work has a concrete benefit.

#### Reuse, API and Dependencies

- Cache eligibility requires `read_only + deterministic + content_addressed` plus an Adapter-verified resource fingerprint. The key binds Cache schema, Project ID, Tool name/version, normalized input, exact Research-stream BLAKE3 and the explicitly absent TargetSnapshot. Only successful, Artifact-free, plan/outcome-consistent source results are stored; explicit reruns always execute freshly.
- A source `tool.execution_receipt_recorded` Event is durably appended before a Cache file is published. Lookup verifies the referenced stream/Event/type/schema/exact Receipt, and a hit appends `tool.result_cache_hit` before returning the fresh cached outcome. Missing, corrupt, forged, cross-project or cross-Session facts safely miss; ordinary `ToolRegistry::for_project` is cache-free and only the explicit Session constructor enables persistence.
- The Session cache path serializes its own appends but never retains the project-wide `ProjectWriter`; Research, Memory and Artifact writers remain usable by the same Agent. The existing Event Store scanner, Project identity, Tool Registry, Receipt, BLAKE3, Serde and durable-file owner are reused without a new Port, fact stream, database, dependency, Cargo feature or background worker.
- The only new public composition surface is `ToolRegistry::for_project_with_session`; it has a current AgentLoop consumer and returns the existing typed Project Event Store error. All cache implementation and Event-payload helpers remain crate-private. Codex classification is `REIMPLEMENT`; no source was copied and no Port Ledger entry is required.

#### Test Layout and Verification

- Permanent top-level regressions cover stale/missing preimages, invalid hunks and CRLF, actual before/after Hash/size/Diff, non-terminated final lines, Unix permission preservation, absolute/parent/symlink rejection, and explicit refusal of multi-file Patch transactions.
- Cache regressions cover Effect/Determinism eligibility, explicit rerun bypass, Research invalidation, plan/outcome binding, source checksum/JSON/deletion/project isolation, missing/corrupt source Receipt Events, source and hit Event linkage, append failure refusing an unaudited hit, short Writer lifetime, ordered Session sequences and one AgentLoop vertical path. The Session sequence regression passed 30 consecutive additional runs.
- Final local Gate passes on Rust `1.93.1`: formatting, denied-warning Clippy, 165 offline/local tests plus one ignored synthetic Live Smoke, locked build, `git diff --check`, production inline-test scan, forbidden Codex dependency scan, credential-isolation scan and `codex/`-unchanged check. Terra xhigh implemented the bounded slice; main review fixed the long-held Writer, plan/outcome binding and duplicate Event source payload; Luna xhigh returned final `APPROVE` and independently constrained the later lock-release correction to the existing Writer owner.

#### Removed and Deferred

- Removed the original Cache path that could return a fresh Receipt without a persistent Event, the long-lived Project Writer that blocked mutating Tools, duplicate Memory/file Diff rendering, duplicate Cache-Hit source fields, unconditional plan cloning for non-cache Tools and caching for explicit reruns.
- Deferred exactly as scoped: multi-file Patch transactions, binary/non-LF Patch input, Artifact-bearing result reuse, additional Tool cache consumers, cross-Session cache reuse, Context Planner, Mode/Profile/Vision, Compaction/Request Audit and Live Provider requests. A new cache consumer must prove measured value and validate referenced Artifacts before eligibility is widened.
- Retained P2 under the trusted single-user model: external filesystem TOCTOU between preimage and rename, the post-rename/pre-Receipt crash window, synchronous JSONL/cache filesystem work on an async worker, and adversarial cache-directory replacement. None permits a validation failure to partially publish the staged single-file write or an unaudited Cache Hit to reach the AgentLoop.

## Blockers

- None for M4-S3d: implementation, independent bounded review, fixes and complete local Cargo/Lean Gate pass. Commit and publication remain intentionally unperformed because they were not authorized.
- M4-S3e1 is not blocked; it is `READY_NOT_AUTHORIZED` and awaits an explicit implementation instruction.
- None for M4-S3c: implementation, independent bounded review, fixes, complete local Cargo/Lean Gate, one authorized DeepSeek Live Chat, PR `#50` and Squash Merge `1b7b032` are complete. No Actions run was requested or triggered for that merge.
- None for local F1a implementation; its full offline Gate is complete. Git publication remains intentionally unperformed because it was not authorized.
- None for local F1b implementation; its full offline Gate is complete. F1c, Git publication and any real Live Test require separate current instructions.
- None for M2-S5: implementation, stabilization and status PRs plus all post-merge `main` CI runs are complete.
- None for M3-S1: implementation PR `#27`, status PR `#28` and all corresponding post-merge `main` Gates are complete.
- None for M3-S2: implementation PR `#29`, status PR `#30` and all corresponding post-merge `main` Gates are complete.
- None for M3-S3a; implementation PR `#31`, status PR `#32` and all corresponding post-merge `main` Gates are complete.
- None for M3-S3b; implementation PR `#33`, status PR `#34` and all corresponding post-merge `main` Gates are complete.
- None for M3-S3c; implementation PR `#35`, status PR `#36` and all corresponding post-merge `main` Gates are complete.
- None for M3-S3d; implementation PR `#37`, status PR `#38` and the single required PR-head Gate are complete.
- None for M3-S4; implementation PR `#39`, status PR `#40` and the single required PR-head Gate are complete.
- None for M4-S1; implementation PR `#42`, exact-head CI, implementation merge and separate status PR `#43` complete the remote seal.
- None for M4-S2; implementation commit `d99f395`, PR `#44`, sole PR-head CI `33165972124`, Squash Merge `2afe900` and separate status PR `#45` complete the remote seal.
- None for F1e implementation: the local Gate, selected DeepSeek Full-only Tool Loop and operator-authorized remote merge are complete. Run `33295806720` is explicitly recorded as zero-Step billing/spending-limit failure, not PASS. M4-S3c was later separately authorized and merged through PR `#50`; M4-S3d is complete locally, while additional Live requests, Git publication and further Actions remain unauthorized, and no Codex credential is an allowed substitute.

## Last Verification

- M4-S3d local Gate: PASS on Rust `1.93.1`; formatting, denied-warning Clippy, 354 discovered tests with 352 default-enabled tests passing and two opt-in Live Smokes ignored, locked build and all Lean scans pass.
- M4-S3d Observation/Delta Gate: PASS; strict AndroidManifest batches retain exact Snapshot/Artifact/Call/Receipt/Extractor provenance, reject malformed or forged cross-fact inputs, compare full component/exported/permission/intent-filter state deterministically and never write Research, Memory or Finding state.
- M4-S3d review: the independent bounded Luna audit returned `CHANGES_REQUIRED` with three P1 and one P2; actual Blob-size validation, toolchain binding, complete added/removed detail and the requested strict/permutation regressions are fixed. Main cross-boundary review closed the remaining provenance/ordering checks; no audit timeout or absence of feedback was counted as approval.
- M4-S3d isolation/Git: PASS; no Provider request, Live credential, Actions run or Codex login access occurred. The complete changes remain uncommitted on `agent/m4-s3d-observation-target-delta`; M4-S3e1 and all remote publication remain unauthorized.
- M4-S3c local Gate: PASS on Rust `1.93.1`; formatting, denied-warning Clippy, 349 discovered tests with 347 default-enabled tests passing and two opt-in Live Smokes ignored, locked build and all Lean scans pass.
- M4-S3c Tool Gate: PASS; `http_request` preserves bounded URL/body/redirect/timeout/capture and Artifact semantics without a second client/publisher, while `inspect_environment` preserves fixed bounded concurrent probes, secret-free normalized versions, stable ordering/fingerprint and no Environment store.
- M4-S3c review: the independent bounded Luna audit found three concrete memory/secret-boundary issues; all were fixed with permanent regressions. Its optional repair follow-up timed out and is recorded as no conclusion, while the main cross-boundary review closed all findings from source and regression evidence.
- M4-S3c Live Test: PASS; one Config-v1-selected DeepSeek Chat produced real `research_update`, `inspect_environment` and `http_request` calls with exact pairing and three successful Receipts. The bounded loopback HTTP result was `200`, its 961-byte Artifact/Blob exactly matched `Cargo.toml`, completed reasoning was replayed only in provider delivery, and the final answer accurately quoted all required values.
- M4-S3c isolation/Git: PASS; the loopback service was stopped and the temporary Project moved out of `/tmp` into the recoverable user Trash after validation, while the user-owned local Provider config remains. No Codex credential state was accessed. Implementation commit `05ae0e3`, PR `#50` and Squash Merge `1b7b032` are complete with no Workflow run; no tag or release occurred.
- F1e local Gate: PASS on Rust `1.93.1`; formatting, denied-warning Clippy, 313 discovered tests with 311 default-enabled tests passing and two configuration-driven Live Smokes ignored, locked build and all Lean scans pass.
- F1e protocol/AgentLoop Gate: PASS; one Responses decoder accepts completed plaintext reasoning with an optional redundant Adapter-local encrypted sidecar, rejects encrypted-only/unknown content, and one encoder preserves reasoning/call/result order. Atomic completion, cancellation/failure isolation, Full/incremental/rejection, Context budget and Deferred ToolSpecs-change paths are covered.
- F1e History/Audit/Replay Gate: PASS; reasoning never enters Conversation/Session/Research/Memory or user output. RequestAudit reads ModelInputIR v2/v3, narrows distinct Full delivery, and a real Replay provider regression proves v2-to-v3 request matching.
- F1e review: the bounded independent Luna audit found one Context-candidate P1, which was fixed with explicit `ProviderContinuationUnavailable` and a permanent regression; its repair review returned `APPROVE`. Main cross-boundary review found and fixed the deferred `tool_search`/ToolsChanged path. Optional post-fix addendum audit threads timed out and were terminated without being counted as approval.
- F1e Live Test: PASS; explicit Config v1 selected `deepseek-v4-flash` with `incremental_input=false`. The first model step produced reasoning plus `read_file`, the existing Tool Registry executed the synthetic file read, and the second Full step replayed reasoning/call/result and returned the exact fixture content. The first attempt exposed a redundant encrypted sidecar and failed under v1.5; ADR-0016/v1.6 plus permanent fixtures narrowed that wire rule, after which the same generic path passed.
- F1e isolation/Git: PASS with an explicit CI waiver; the inline credential was not displayed or copied into repository facts, and Codex login state, `~/.codex` and `CODEX_HOME` were untouched. Five temporary reasoning probe SSE files were permanently deleted; the user-owned local Provider config and synthetic project remain. Commit `5bf760d`, PR `#48` and Squash Merge `2f6a9e5` are complete. Run `33295806720` executed zero Steps and is not counted as PASS; no tag or release occurred.

- F1b local Gate: PASS on Rust `1.93.1`; formatting, denied-warning Clippy, 298 discovered tests with 296 default-enabled tests passing and two generic configuration-driven Live Smokes ignored, plus locked build and all Lean scans.
- F1b protocol Gate: PASS; one `ResponsesHttpAdapter` owns request encoding, HTTP/SSE transport and decoding for every explicitly configured conforming service. Reasoning, Tool, text, Usage, Completed, unknown-event, cancellation, incremental and Full-only paths are covered without a vendor binding.
- F1b composition Gate: PASS; Config v1 `http_sse + responses` selects URL, Model, exact dedicated credential reference and explicit Capability Mapping without inference. `chat`, `doctor` and both Live Smokes retain the same Composition Root; Full-only rejection occurs before credential/network access.
- F1b reuse Gate: PASS; no vendor Adapter symbols, official Provider URL, model environment variable, second wire owner, Provider Router/Registry, second AgentLoop, new dependency or forbidden Codex dependency exists. Reasoning never enters assistant History/output or qualifies alone as semantic Replay output.
- F1b Live Test: PASS; `doctor` reported the configured Primary ready, then the separately authorized fixed DeepSeek Primary smoke completed with 1 passed, 0 failed in approximately 0.99 seconds. The temporary credential/configuration directory was removed; Codex login state, `~/.codex` and `CODEX_HOME` were untouched.
- F1b Git/remote: `NOT_RUN`; local F1a/F1b worktree changes remain over `08e9025` because no branch/commit/push/PR/Workflow authorization was provided.
- F1a local Gate: PASS on Rust `1.93.1`; formatting, denied-warning Clippy, 294 discovered tests with 292 default-enabled tests passing and two generic configuration-driven Live Smokes ignored, plus locked build and all Lean scans.
- F1a composition Gate: PASS; Config v1 is strict and capability-explicit, Primary/Vision compile through one Root, inactive invalid profiles are rejected, CLI `chat`/`doctor` and Live Smokes share the same entry, and credential resolution remains exact-reference and stream-lazy.
- F1a reuse Gate: PASS; generic runtime/test paths contain no Provider official URL or vendor-specific live-model environment variable. Only the Root and the provider-specific offline OpenAI contract test directly construct the existing Adapter; there is one AgentLoop and no Provider Registry/Router/Plugin.
- F1a Live Test: `NOT_RUN`; no real model request or Provider credential read was authorized. Codex login state, `~/.codex` and `CODEX_HOME` were untouched.
- F1a Git/remote: `NOT_RUN`; local worktree changes remain over `08e9025` because no branch/commit/push/PR/Workflow authorization was provided.
- M4-S2 local Gate: PASS on Rust `1.93.1`; formatting, denied-warning Clippy, 288 discovered tests with 286 default-enabled tests passing and two ignored OpenAI Live smokes, locked build, inline-test, forbidden Codex dependency, credential/key, Provider-wire/dependency-direction, one-AgentLoop/no-second-store, `git diff --check` and clean fixed `codex/` scans all passed.
- M4-S2 Trace Gate: PASS; one reference-only fixture is bound to the complete source Session hash, fresh reduced projection, exact Event range, Target/Artifact facts, ordered full RequestAudit/History samples and source-call-ordered Receipts. Required Artifact bytes are reread from the existing store and checked by size and SHA-256.
- M4-S2 Replay Gate: PASS; Mechanical Replay consumes exact Full model requests and fixed Replay Tool steps with zero external action. Tool exposure is honored without consuming hidden/delayed steps, unmatched calls terminate immediately, complete historical Receipts remain source provenance and derived replay Receipts assert only current replay effects.
- M4-S2 Context/Shadow Gate: PASS; Context Recompile compares a current candidate to one exact historical RequestAudit without mutation. Shadow Eval accepts only a caller-supplied model plus ReplayToolExecutor, requires complete Tool-step consumption and returns an EvalSource-bound `NonEvidence` result with no formal fact writer.
- M4-S2 review: Terra xhigh implemented the bounded path. Main review fixed exact source/Eval identity, Receipt provenance, required Blob verification, exposure, Full-only capability truthfulness, frozen field naming and multi-Tool order. Luna xhigh returned final `APPROVE` with no unresolved P0/P1.
- M4-S2 Live Test: `NOT_RUN_PROVIDER_BINDING_NOT_IMPLEMENTED`; DeepSeek publicly supports Responses, but Cyber Agent did not yet have a configuration-driven DeepSeek binding or its explicit event/capability mapping. No credential was read, logged, persisted, aliased across Provider roles or sourced from Codex login state.
- M4-S2 remote Gate: PASS; exact implementation head `d99f395` was Squash Merged through PR `#44` as `2afe900`, and sole PR-head run `33165972124` passed the macOS Rust Gate. The merge used `[skip ci]`, so no duplicate `main` Workflow was created.
- M4-S2 status seal: PASS; status-only PR `#45` records the implementation identities and final stopped state. Its commits and merge use `[skip ci]` and start no M4-S3, M4-S4 or Live Test work.
- M4-S1 local Gate: PASS on Rust `1.93.1`; formatting, denied-warning Clippy, 266 offline/local tests plus one ignored existing synthetic Live Smoke, locked build, inline-test scan, forbidden Codex dependency scan, credential/general-registry review, `git diff --check` and the clean fixed `codex/` reference check all passed.
- M4-S1 Profile Gate: PASS; the exact four compile-time Markdown Profiles map one-to-one to Source/Web/Mobile/Binary, exclude Firmware, preserve direct command access, keep multimodal access direct where routinely relevant, compose through one Turn-boundary AgentLoop path and invalidate the existing Context identity when Mode generation changes.
- M4-S1 controlled-case Gate: PASS; twelve synthetic offline fixtures form positive, negative and tampered/environment controls, bind unique same-Profile relationships, verify fixed manifest and per-file SHA-256 identities and cover Candidate/Reproduced/Confirmed without treating model output as fact.
- M4-S1 review: Terra xhigh implemented the bounded path. Main review rejected a production Eval contract without a consumer, removed an independent fingerprint/API, made controlled samples substantive, corrected image Tool exposure and fixed the manifest anchor. Luna xhigh returned final code-level `APPROVE` with no unresolved P0/P1 after these changes and the explicit tests-only contract decision.
- M4-S1 Live Test: `OFF`; no credential, public Provider request, real target, Codex login state, `~/.codex` or `CODEX_HOME` was read or modified.
- M4-S1 remote Gate: PASS; exact implementation head `b9cb3f9` was Squash Merged through PR `#42` as `7631709`, and sole PR-head run `32885992788` passed the macOS Rust Gate. The merge used `[skip ci]`, so no duplicate main Workflow was created.
- M4-S1 status seal: PASS; status-only PR `#43` records the implementation identities and final stopped state. Its commits and merge use `[skip ci]` and start no M4-S2 or Live Test work.
- M3-S3d local Gate: PASS on Rust `1.93.1`; formatting, denied-warning Clippy, 244 offline/local tests plus one ignored existing synthetic Live Smoke, locked build, inline-test scan, forbidden Codex dependency scan, credential/transport and `codex/`-unchanged scans, plus `git diff --check`, all passed.
- M3-S3d inspection Gate: PASS; uninitialized/empty/normal/corrupt facts are stable, one Session auto-selects while ambiguity requires `--session`, all RequestAudit Blob references are revalidated through the existing service, current Snapshot disposition does not misreport historical refresh provenance, and inspections perform no local write, Provider call, Tool execution or credential resolution.
- M3-S3d review: Luna xhigh implemented the bounded path; main review separated per-command reads, removed redundant aliases/abstractions, added explicit Session selection and strict filesystem handling, reused the RequestAudit/Token owners and fixed current-Hit semantics. Terra xhigh returned final `APPROVE` with no P0/P1/P2 blocker.
- M3-S3d Live Test: `OFF`; no credential, external model request, Codex login state, `~/.codex` or `CODEX_HOME` was read or modified.
- M3-S3d remote Gate: PASS; implementation commit `41546ba` was Squash Merged through PR `#37` as `a29187b`, and PR-head run `32649939090` passed the macOS Rust Gate. The merge used `[skip ci]` to avoid repeating the exact tree Gate on `main` after the user reported limited Actions quota.
- M3-S3d status seal: PASS; documentation commit `e4be415` was Squash Merged through PR `#38` as `1bdb19a`. The status-only PR and merge used `[skip ci]` and triggered no Workflow.
- M3-S3c remote Gate: PASS; implementation commit `c4a2cf9` was Squash Merged through PR `#35` as `cb6781e`, with PR-head run `32643924616` and post-merge `main` run `32644062157` passing the macOS Rust Gate.
- M3-S3c status seal: PASS; PR `#36` was Squash Merged as `96385f1`, with PR-head run `32644218115` and post-merge `main` run `32644328650` passing before M3-S3d started.
- M3-S3b local Gate: PASS on Rust `1.93.1`; formatting, denied-warning Clippy, 228 offline/local tests plus one ignored existing synthetic Live Smoke, locked build, inline-test scan, forbidden Codex dependency scan, credential isolation, Provider-wire and dependency-direction scans, one-AgentLoop/no-second-History-owner review, `git diff --check` and `codex/`-unchanged check all passed.
- M3-S3b Summary/Compaction Gate: PASS; explicit bounded typed Summary content replays from strict Events, actual recent History retention preserves complete Tool pairs and all original facts, source/revision/fingerprint mismatches fail atomically, later History remains recoverable, and exact full-payload Compaction application is Session/Primary-bound and idempotent.
- M3-S3b runtime Gate: PASS; pending Summary changes remain outside active Context/Baseline, durable Compaction forces `CompactionReset` Full, failure/empty/cancellation preserve the reset, one completed committed sample clears it, a repeated exact fact is a no-op and compacted Resume restores Summary plus retained/post-boundary History under `SessionResumed`.
- M3-S3b review: Terra xhigh implemented the bounded path; main review replaced no-op retention and stringly/unbounded data, added full-payload identity and pending-candidate separation, and split Compaction Domain/App ownership. Luna xhigh initially returned `REQUEST_CHANGES`, then returned final `APPROVE` with no P0/P1 after the fixes and explicit architecture interpretation.
- M3-S3b Live Test: `OFF`; no credential, external model request, Codex login state, `~/.codex` or `CODEX_HOME` was read or modified.
- M3-S3b remote Gate: PASS for implementation commit `86240e3`; PR `#33` was Squash Merged as `5e2b2e3`, PR-head run `32639174240` and post-merge `main` run `32639345787` passed the macOS Rust Gate before this status seal.
- M3-S3b status seal: PASS; PR `#34` was Squash Merged as `d6639c5`, PR-head run `32639529288` and post-merge `main` run `32639656923` passed before M3-S3c started.
- M3-S3a local Gate: PASS on Rust `1.93.1`; formatting, denied-warning Clippy, 220 offline/local tests plus one ignored existing synthetic Live Smoke, locked build, inline-test scan, forbidden Codex dependency scan, credential isolation, Provider-wire and dependency-direction scans, one-AgentLoop/no-second-History-owner review, `git diff --check` and `codex/`-unchanged check all passed.
- M3-S3a Session/Resume Gate: PASS; Session payload v1 is strict including nested fields, failed validation/append leaves State unchanged, duplicate append is a no-write result, existing Receipt/CacheHit/Recovery facts replay, used Primary IDs never recur, and only a durable Active/Resumed projection can restore normalized History and force `SessionResumed` Full delivery.
- M3-S3a review: Terra xhigh implemented the bounded path; main review added existing-Session-event compatibility, permanent identity uniqueness, durable History restoration, duplicate-append sequence safety and strict nested payload checks. Luna xhigh returned final `APPROVE` with no P0/P1. Historical Context inspection labeling and the legacy `StreamRecovered` payload are recorded P2 for the later inspection/migration boundary.
- M3-S3a Live Test: `OFF`; no credential, external model request, Codex login state, `~/.codex` or `CODEX_HOME` was read or modified.
- M3-S3a remote Gate: PASS for implementation commit `cc0c4ed`; PR `#31` was Squash Merged as `3154787`, PR-head run `32635754926` and post-merge `main` run `32635865554` passed the macOS Rust Gate before this status seal.
- M3-S3a status seal: PASS; PR `#32` was Squash Merged as `3ebf112`, PR-head run `32636139730` and post-merge `main` run `32636237874` passed before M3-S3b started.
- M3-S2 local Gate: PASS on Rust `1.93.1`; formatting, denied-warning Clippy, 211 offline/local tests plus one ignored existing synthetic Live Smoke, locked build, inline-test scan, forbidden Codex dependency scan, credential-isolation scan, Provider-wire and dependency-direction scans, one-AgentLoop/no-general-router review, `git diff --check` and `codex/`-unchanged check all passed.
- M3-S2 runtime Gate: PASS; State Diff delivery is causal, fully selected and revalidated by the Context Planner against the actual cumulative rendered input. Overflow falls back to Full and a successful Full establishes a fresh incremental Baseline; failed requests do not promote derived state markers.
- M3-S2 Tool/Profile Gate: PASS; only the sent Direct schemas can execute, `tool_search` sees only Deferred tools, Hidden tools remain undiscoverable, Exposure-directory changes force `TOOLS_CHANGED`, and Primary/secondary Profile identities never cross. Vision routing is capability-only and exhaustively tested.
- M3-S2 review: Terra xhigh implemented and repaired the bounded vertical path. Luna xhigh first returned `REQUEST_CHANGES` for cumulative Diff budgeting, Deferred/Hidden-only invalidation and an unused doctor wrapper; after permanent regressions and removal, its second review returned `APPROVE` with no P0/P1. Static CLI doctor integration remains recorded P2 for the later CLI/Profile composition path.
- M3-S2 Live Test: `OFF`; no credential, external model request, Codex login state, `~/.codex` or `CODEX_HOME` was read or modified.
- M3-S2 remote Gate: PASS; implementation PR `#29` was Squash Merged as `64209bc`, PR-head run `32632629044` and post-merge `main` run `32632750182` passed the macOS Rust Gate before this status seal.
- M3-S2 status seal: PASS; PR `#30` was Squash Merged as `48c8cb1`, PR-head run `32632953088` and post-merge `main` run `32633057788` passed before M3-S3 started.
- M3-S1 local Gate: PASS on Rust `1.93.1`; formatting, denied-warning Clippy, 192 offline/local tests plus one ignored existing synthetic Live Smoke, locked build, inline-test scan, forbidden Codex dependency scan, dedicated-credential review, dependency-direction scan, secret-pattern scan, `git diff --check` and `codex/`-unchanged check all passed.
- M3-S1 review: Terra xhigh implemented the bounded Context path and four P1 hardening rounds; Luna xhigh initially returned `REQUEST_CHANGES`, then returned final `APPROVE` after UTF-8/rendered-overhead budgeting, constructor-owned estimates, complete cache content identity/checksum and mismatched-hit revalidation gained permanent regressions. No unresolved P0/P1 remains.
- M3-S1 runtime Gate: PASS; AgentLoop sends only the Planner's final `ModelInputIR`, successful Context state advances only after Provider/History/Baseline commit, and a failed request exposes its attempted plan without replacing the preceding successful inspection or committed logical manifest.
- M3-S1 Live Test: `OFF`; no credential, external model request, Codex login state, `~/.codex` or `CODEX_HOME` was read or modified.
- M3-S1 remote Gate: PASS; implementation PR `#27` was Squash Merged as `96fea6b`, PR-head run `32625847959` and post-merge `main` run `32625961489` passed the macOS Rust Gate before this status seal.
- M3-S1 status seal: PASS; PR `#28` was Squash Merged as `d4217d2`, PR-head run `32626117840` and post-merge `main` run `32626210003` passed before M3-S2 started.
- M2-S5 local Gate: PASS on Rust `1.93.1`; formatting, denied-warning Clippy, 164 offline/local tests plus one ignored existing synthetic Live Smoke, locked build, inline-test scan, forbidden Codex dependency scan, credential-isolation scan, `git diff --check` and `codex/`-unchanged check all passed. The Session sequence regression passed 30 additional consecutive runs.
- M2-S5 review: Terra xhigh completed the bounded implementation; main review fixed the persistent Receipt/Event contract, long-held Project Writer, plan/outcome binding and duplicate Event source payload; final Luna xhigh review returned `APPROVE` with no unresolved P0/P1.
- M2-S5 Live Test: `OFF`; no credential, external model request, Codex login state, `~/.codex` or `CODEX_HOME` was read or modified.
- M2-S5 remote Gate: PASS; PR `#24` was Squash Merged as `a46ad33`, PR-head run `32598264243` passed, and post-merge `main` run `32598378267` passed the same Rust Gate.
- M2-S5 status-seal correction: PR `#25` run `32601709085` exposed nondeterministic implicit macOS Writer-lock release on two attempts. The sole `ProjectWriter` owner now explicitly unlocks and a permanent immediate-reacquisition regression was added; the broader Tool-dispatch refactor was rejected as unnecessary.
- M2-S5 stabilization Gate: PASS; local formatting, denied-warning Clippy, 165 offline/local tests plus one ignored Live Smoke and locked build passed. PR `#26` was Squash Merged as `018d47a`; PR-head run `32614460762` and post-merge `main` run `32614845713` passed.
- M2-S5 status seal: PASS; PR `#25` was Squash Merged as `206f142`, final PR-head run `32615810257` and post-merge `main` run `32617170396` passed before M3-S1 started.
- M2-S4 local Gate: PASS on Rust `1.93.1`; formatting, denied-warning Clippy, 148 offline/local tests plus one ignored existing synthetic Live Smoke, locked build, inline-test scan, forbidden Codex dependency scan, credential-pattern scan, dependency/architecture diff scans, `git diff --check` and `codex/`-unchanged check all passed. No Live Test was run.
- M2-S4 review: Terra xhigh `REQUEST_CHANGES` findings for exact duplicate Memory replay and cached BLAKE3 reuse were fixed with permanent regressions; main review fixed repeated-dirty Git invalidation and split the oversized Snapshot Adapter; Luna xhigh returned final `APPROVE` after the explicit `core.md` bound gained pre-Blob, external-read and Schema coverage. No unresolved P0/P1 remains.
- M2-S4 Live Test: `OFF`; no credential, external model request, Codex login state, `~/.codex` or `CODEX_HOME` was read or modified.
- M2-S4 remote Gate: PASS; PR `#22` was Squash Merged as `19253a2`, PR-head run `32591476669` passed, and post-merge `main` run `32591626316` passed the same Rust Gate.
- M2-S3 local Gate: PASS on Rust `1.93.1`; formatting, denied-warning Clippy, 138 offline/local tests plus one ignored existing synthetic Live Smoke, locked build, inline-test scan, forbidden Codex dependency scan, credential-pattern scan, `git diff --check` and `codex/`-unchanged check all passed. No Live Test was run.
- M2-S3 review: independent Luna xhigh returned `APPROVE` with no unresolved P0/P1 after the successful Receipt gained its Artifact ID, new Blob ancestry became crash-durable and unused public Artifact projection APIs were removed.
- M2-S3 Live Test: `OFF`; no credential, external model request, Codex login state, `~/.codex` or `CODEX_HOME` was read or modified.
- M2-S3 remote Gate: PASS; PR `#20` was Squash Merged as `15b2f88`, PR-head run `32586565660` passed, and post-merge `main` run `32586668048` passed the same Rust Gate.
- M2-S3 status seal: PASS; PR `#21` was Squash Merged as `feb5eb9`, PR-head run `32586862900` passed, and post-merge `main` run `32586968200` passed the same Rust Gate before M2-S4 started.
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
