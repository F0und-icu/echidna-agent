# Cyber Agent Development Status

## Current State

- Architecture version: `1.1.0`
- Phase: `IMPLEMENTATION`
- Implementation: `STARTED`
- Current milestone: `M4`; M4-S1 is implementation-complete locally and remote sealing is pending
- Current slice: `M4-S2`
- Slice state: `READY_NOT_AUTHORIZED`; M4-S1 must be remotely sealed and work must then stop before Replay/Eval or Live Test activity
- Lean governance: `ACTIVE`; M4-S1 passed its full local Cargo/Lean Gate and final Luna xhigh code review with no unresolved P0/P1
- Local Git initialized: `yes`; M4-S1 implementation is complete on `agent/m4-s1-research-profiles`
- Remote repository: `F0und-icu/cyber-agent` private; M3-S4 implementation PR `#39` and `[skip ci]` status PR `#40` were Squash Merged without duplicate post-merge CI
- Documentation baseline commit: `55ff3be`
- Current code base: M4-S1 local implementation candidate based on sealed M3-S4 `main` at `b635645`; implementation PR evidence is pending
- Upstream Codex baseline: `3aae5d885bac39c1262491aa3fd100dfd8b3919f`

## Last Completed Slice

- Slice: `M4-S1`
- Delivery branch: `agent/m4-s1-research-profiles`
- Base commit: `b635645`
- Scope: four fixed source/web/mobile/binary vulnerability-research Prompt Profiles, one Turn-boundary selection path in the existing AgentLoop and twelve deterministic tests-only controlled offline cases.
- Profile behavior: each Profile has one fixed ID, generation, compatible Research `TargetKind`, compiled Markdown prompt and bounded Tool exposure. Firmware remains unsupported. The prompts require Candidate/Reproduced/Confirmed separation, real target/version/environment evidence, positive and negative controls, explicit `research_update` and non-fabrication of Tool or evidence facts.
- Context behavior: Mode generation now participates in `prompt:mode:<id>:<generation>` identity, and the existing Context Planner fingerprint remains the only semantic hash. Source defers low-frequency image inspection; Web/Mobile/Binary keep `view_image` Direct, while command tools remain Direct for all Profiles.
- Fixture behavior: each Profile has one positive, one negative and one tampered/environment case with distinct synthetic source, HTTP, mobile or binary content. Per-file SHA-256 plus a fixed manifest anchor detect drift; positives collectively cover Candidate, Reproduced and Confirmed, while every control is `NoFinding`.
- Result: `COMPLETE` locally; the full Cargo/Lean Gate passes 266 offline/local tests with one existing opt-in Live Smoke ignored. Main review removed an unconsumed independent Profile fingerprint, kept controlled-case types tests-only and corrected image Tool exposure; final Luna xhigh review returned code-level `APPROVE` with no unresolved P0/P1. Remote PR evidence is pending.
- Dependencies and schemas: no Cargo dependency, feature, Event, Blob, Cache, persisted configuration, Provider DTO, Port, credential source or remote transport changed. No production controlled-case or Eval schema was added without a runtime consumer.
- Git: implementation commit, final implementation PR, the single authorized PR-head CI run, Squash Merge and separate `[skip ci]` status seal are pending.
- Live Test: `OFF`; no real credential, public Provider request, Codex login state, `~/.codex` or `CODEX_HOME` was read or modified.

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
11. `docs/adr/ADR-0005-minimal-local-image-stack.md`

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

## Next Planned Slice

- Slice: `M4-S2`
- State: `READY_NOT_AUTHORIZED`; this status is planning only and does not start Replay/Eval work.
- Planned delivery branch: resolve only after the M4-S1 implementation and status seal are remotely merged and a new explicit user instruction authorizes M4-S2.
- Scope: Trace Replay and Shadow Eval as already bounded by the Implementation Plan.
- Forbidden continuation: all M4-S2 implementation, M4-S3 Observation/TargetDelta, M4-S4 long-flow/Live Provider work, general Prompt DSL/loader/registry, dataset service, database, model router, Hook/plugin runtime and Codex credential/login access.

## Current Authorization

The current user instruction authorizes only completion and remote sealing of M4-S1, then requires work to stop.

- Run mode: `SEAL_M4_S1_AND_STOP`
- Slice ID: `M4-S1`; state `COMPLETE_LOCAL_PENDING_REMOTE_SEAL`
- Git mode: `BRANCH_PR_REVIEW_MERGE`; implementation commit/push/PR, one final PR-head CI run, Squash Merge and a separate `[skip ci]` status-seal PR are authorized
- Delegation mode: `INACTIVE`; Terra xhigh implementation and Luna xhigh final review are complete
- Live test mode: `OFF`; no real Provider credential or public model request is authorized
- Forbidden continuation: all M4-S2/M4-S3/M4-S4 work, Live Tests, general database/index/watcher/hook/router/plugin/model-router work, Codex source modification or Codex credentials/login access
- Stop condition: after the M4-S1 status-seal PR is merged, stop without starting M4-S2 or a Live Test

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
- Final local Gate passes on Rust `1.93.1`: formatting, denied-warning Clippy, 266 offline/local tests plus one ignored existing synthetic Live Smoke and locked build. Production inline-test, forbidden Codex dependency, credential, architecture/general-registry, `git diff --check` and clean fixed `codex/` scans pass. Terra xhigh implemented the bounded path; main review corrected Profile composition, controlled-case quality, Tool exposure and unused hash/API growth; Luna xhigh returned final code-level `APPROVE` with no unresolved P0/P1.

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

- None for M2-S5: implementation, stabilization and status PRs plus all post-merge `main` CI runs are complete.
- None for M3-S1: implementation PR `#27`, status PR `#28` and all corresponding post-merge `main` Gates are complete.
- None for M3-S2: implementation PR `#29`, status PR `#30` and all corresponding post-merge `main` Gates are complete.
- None for M3-S3a; implementation PR `#31`, status PR `#32` and all corresponding post-merge `main` Gates are complete.
- None for M3-S3b; implementation PR `#33`, status PR `#34` and all corresponding post-merge `main` Gates are complete.
- None for M3-S3c; implementation PR `#35`, status PR `#36` and all corresponding post-merge `main` Gates are complete.
- None for M3-S3d; implementation PR `#37`, status PR `#38` and the single required PR-head Gate are complete.
- None for M3-S4; implementation PR `#39`, status PR `#40` and the single required PR-head Gate are complete.
- None for M4-S1 implementation; the full local Gate and final review are complete, while implementation PR evidence and the separate status seal remain pending.
- Live Provider compatibility remains unverified by design; missing live credentials do not block offline development and no Codex credential is an allowed substitute.

## Last Verification

- M4-S1 local Gate: PASS on Rust `1.93.1`; formatting, denied-warning Clippy, 266 offline/local tests plus one ignored existing synthetic Live Smoke, locked build, inline-test scan, forbidden Codex dependency scan, credential/general-registry review, `git diff --check` and the clean fixed `codex/` reference check all passed.
- M4-S1 Profile Gate: PASS; the exact four compile-time Markdown Profiles map one-to-one to Source/Web/Mobile/Binary, exclude Firmware, preserve direct command access, keep multimodal access direct where routinely relevant, compose through one Turn-boundary AgentLoop path and invalidate the existing Context identity when Mode generation changes.
- M4-S1 controlled-case Gate: PASS; twelve synthetic offline fixtures form positive, negative and tampered/environment controls, bind unique same-Profile relationships, verify fixed manifest and per-file SHA-256 identities and cover Candidate/Reproduced/Confirmed without treating model output as fact.
- M4-S1 review: Terra xhigh implemented the bounded path. Main review rejected a production Eval contract without a consumer, removed an independent fingerprint/API, made controlled samples substantive, corrected image Tool exposure and fixed the manifest anchor. Luna xhigh returned final code-level `APPROVE` with no unresolved P0/P1 after these changes and the explicit tests-only contract decision.
- M4-S1 Live Test: `OFF`; no credential, public Provider request, real target, Codex login state, `~/.codex` or `CODEX_HOME` was read or modified.
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
