# Cyber Agent Repository Instructions

## Purpose

This repository builds a single-user, local-first vulnerability research Agent. Keep it small, auditable, efficient and maintainable by one person.

These instructions are durable repository guidance for Codex. A current user instruction can override them explicitly.

## Start Here

Before changing anything, read in order:

1. `docs/development/STATUS.md`
2. The current Slice in `docs/development/IMPLEMENTATION_PLAN.md`
3. `docs/development/LEAN_CODE_GOVERNANCE.md`
4. Relevant sections of `task.md`
5. `docs/architecture/CYBER_AGENT_ARCHITECTURE_V1.md` and every frozen additive architecture file through the latest version
6. Relevant accepted ADRs, including ADR-0002 for model composition, ADR-0009 for research-control composition, ADR-0010 through ADR-0014 for v1.3/v1.4 and ADR-0015/ADR-0016 for Responses reasoning continuation

Inspect the worktree and existing tests before planning changes.

If the user explicitly authorizes development sub-agents, every delegated agent must read these files itself. A parent-agent summary does not replace the repository authority chain.

## Current Phase

- Architecture v1.10 is the latest locally frozen additive extension over immutable v1 through v1.9. ADR-0020 governs F6a delivered-image geometry and legacy coordinate uncertainty; ADR-0018/0019 retain reasoning/release and F5 budget boundaries. Remote hash seal remains unauthorized; exact verification is governed by `STATUS.md`.
- F6a is REMOTE_SUBMITTED_PENDING_MERGE in Draft PR #69 (implementation `82b42e1`), stacked on the still-open F4/F5 Draft PR #68 branch. Fresh Terra xhigh independent review APPROVE, complete Gate/Lean PASS and two synthetic Vision Live lanes passed. Bounds and simple half-region agreement are not a general localization benchmark. Publication uses `[skip ci]`; merge, Actions, additional Live, Codex login access and next Slice remain excluded. Exact evidence and stop boundary are governed by STATUS.
- M5-S2 and F3 are remotely sealed. F4/F5 are locally complete and previously delivered to Draft PR #68 with `[skip ci]`; that delivery authorization is historical, not permission to publish F6a. F5 independent audit, full Gate/Lean Review and bounded development/release Live passed with the recorded coordinate limitation. F6a addresses coordinate-frame admission; complex localization and full-1M capacity remain unverified. Benchmark/non-interactive execution and Actions remain excluded; exact delivery progress is governed by STATUS.
- M0 through M5-S2, F2a and F3 are complete and remotely sealed; historical evidence and current F5 scope/state are governed by `STATUS.md` and `IMPLEMENTATION_PLAN.md`.
- F1a Config/Composition Root, F1b protocol-first Responses HTTP/SSE Adapter, F1c first-version CLI Chat and F1d local inline Provider credentials are complete and remotely sealed by PR `#46` at Squash Merge `6eb4959`; one synthetic F1c DeepSeek Live Chat passed.
- F1d preserves one configuration and Adapter path: Config v1 may select exactly one inline `api_key` or existing `credential_ref` per Provider. M5 work must preserve that same composition path.
- F1e Responses Reasoning Continuation is complete and remotely sealed by PR `#48` at Squash Merge `2f6a9e5`, including one configuration-driven DeepSeek Full-only Live Tool Loop. Completed reasoning stays in provider delivery, never Conversation History; a redundant encrypted sidecar is ignored only beside valid plaintext reasoning, while encrypted-only remains unsupported. Further Actions and Live requests remain unauthorized unless the operator explicitly enables them.
- Local Git is initialized and `main` tracks the private remote.
- `codex/` is a clean, read-only upstream reference repository.

Do not start a Slice, commit, push, create a PR or run a Live Test unless the current user request explicitly authorizes the corresponding action.

## Slice Discipline

- Implement only the Slice named in `STATUS.md` and the current user request.
- Do not begin the next Slice after passing the current Gate.
- Prefer a thin end-to-end path, then failure paths and tests.
- Preserve all prior Slice behavior.
- Apply `docs/development/LEAN_CODE_GOVERNANCE.md` and record the Slice Lean Review.
- Update STATUS at Slice start and completion.
- Stop after handing off completed verification.

Do not use multi-agent development orchestration unless the current user explicitly authorizes delegation. If authorized, every child follows the same Slice and governance boundaries. This never changes the product invariant: the Cyber Agent runtime itself remains one AgentLoop, not a multi-agent runtime.

### Bounded implementation and audit roles

- Terra xhigh owns Context, Session, Replay, Evidence and AgentLoop work, plus any change crossing a Port or Adapter boundary.
- Luna xhigh owns only a bounded single-module Tool/Schema/Fixture/tests/docs change or a small fix. Luna's audit is read-only and limited to the finite files and explicit checklist named in the handoff.
- The implementation owner must not audit its own changes. Terra xhigh or the main agent performs the final cross-boundary audit.
- 每个 Slice 固定顺序必须精确为：专项测试 → 受影响测试 → 独立审计 → 修复 → 完整 Gate → Lean Review。审计必须限时；超时或无结论不记为 `APPROVE`。

## Architecture Invariants

- One Rust package and one default binary.
- No Cargo workspace unless an accepted ADR changes this.
- Internal direction: `ui/main -> app -> ports -> domain`; adapters implement ports and do not depend on app.
- Domain contains no filesystem, network, process, clock or Provider-specific I/O.
- Responses wire types stay inside the protocol Adapter; Core uses ModelInputIR.
- Runtime has one primary ModelProvider and at most one optional vision ModelProvider; both remain inside one AgentLoop and one Session authority.
- The Base Contract has one stable `Pragmatic Vulnerability Researcher` identity. Do not add multiple personalities, role Agents or a persona registry.
- Keep `DomainProfile` (`source`, `web`, `mobile`, `binary`), `ResearchPhase` (`mapping`, `hypothesis`, `experiment`, `verification`, `evidence_curation`) and `AgentModelProfile` as orthogonal axes.
- Prompt composition is fixed: Base Contract, Personal, Project, DomainProfile, ResearchPhase, Research/Project/Memory facts, Summary/History, current User input and Turn-local updates. Tool Specs remain a separate protocol channel.
- DomainProfile and ResearchPhase changes take effect only at a Turn boundary. A model's unstructured prose never changes runtime Phase or Tool exposure.
- Every model sampling Step captures one immutable SamplingStepPlan. The exact ResolvedToolPlan that advertises ToolSpecs also validates and dispatches model-originated ToolCalls.
- Deferred Tool activation lasts only for later Steps of the current Turn and clears at the next Turn boundary and after Resume.
- `generic-v1` is the only Prompt renderer. Do not add a renderer Trait, registry or Config field without a second measured implementation and an accepted ADR.
- Vision routing is capability-driven and limited to `auto`, `primary_direct`, `secondary_tool` or `disabled`; do not build a general model router.
- JSONL/Markdown/Artifact Blob are facts; Cache/Summary/Snapshot/Replay are derived.
- Context placement, origin, authority, retention and freshness are orthogonal. Keep provenance in a validated ContextDeliveryMap sidecar rather than widening every ContentItem.
- Event Streams are append-only and use chained Upcasters.
- TargetSnapshot/ExperimentCapsule/ExecutionReceipt preserve reproducibility.
- Tool exposure is a deterministic context-efficiency mechanism, not authorization. Hidden is terminal; Domain/Phase overrides and `tool_search` must use one pure exposure owner.
- Tool Effect metadata controls retry/cache/scheduling, not approval.
- Session Summary v2 must retain typed fact references plus bounded lossy narrative; it never copies Research claims into a second authority.
- Security retrieval and evidence-gap guidance are local, deterministic, explainable, Shadow-first and non-evidence. Do not add a vector database, remote judge or autonomous workflow.
- Runtime defaults to FullAccess; do not add Sandbox, Policy or confirmation systems.
- No telemetry, analytics, feedback or remote audit.
- Provider credentials are explicit Cyber Agent inputs; Codex/ChatGPT login state is never a credential source.
- `chat`, `doctor` and generic Live Smokes use the one configuration-driven Provider Composition Root; generic runtime paths do not own Provider URLs, models or Adapter construction.
- One Wire API has one request encoding, transport and response-decoding owner. The fixed `http_sse + responses` binding is protocol-first: URL, model and credential reference never create a vendor binding. Add a wire-level extension only when fixtures prove a real reusable protocol difference.
- Completed Responses reasoning is provider delivery state, not Conversation History. Full-only replay preserves reasoning/call/result order; loss under Context budget or replacement fails explicitly. Opaque encrypted reasoning stays Adapter-local and may be ignored only when complete supported plaintext reasoning is present.
- CLI first; Web remains optional.
- F4 displays only Provider-returned plaintext reasoning on labeled CLI stderr in all builds; Assistant stdout and Conversation History remain reasoning-free. Full request/response `chat --debug` is compiled only under `cfg(debug_assertions)` and must be absent from standard release. Presentation never creates facts, rewrites requests or discloses transport credentials; raw debug is local/non-persistent, not another audit owner.
- F5 debug displays actual sent content without routine SSE wrappers or duplicated Primary answers. Tool previews are bounded and follow real start/durable receipt observations. ContextPlanner alone owns capped default reserves; explicit CapabilityMapping remains the window authority, never model-name inference.
- F3 checkpoints History before dependent execution/resampling. Resume after an append-only Research suffix validates the old exact prefix then appends a new current-target Compaction; never apply stale references directly. Explicit Prompt files/Core Memory and current facts use the fixed pre-sampling Context owner, not a registry. Compaction budget/reduction checks precede append; Provider deadlines never authorize retries.
- F3 interruption exception: a dropped unobserved executor future has no established terminal outcome; never manufacture its cancelled Receipt/Result. Persist known completed outcomes, stop the Turn and retain explicit unknown recovery state. Executor-returned cancelled outcomes remain real observations, not proof of zero side effects.

## Forbidden Dependencies and Scope

Do not directly depend on or copy whole implementations from:

- `codex-core`
- `codex-tools`
- `codex-apply-patch`
- `codex-utils-pty`
- Codex app-server, connectors, code-mode or cloud products

Do not modify any file under `codex/`.

Do not add:

- Multi-agent runtime
- Cloud server
- Team/tenant/auth product layer
- Marketplace or dynamic Rust plugin ABI
- Default vector/graph database
- Workflow DSL
- Distributed job queue
- Personality/role catalogs, combined domain-phase role variants or a general research workflow engine
- Prompt renderer/tool selector registries, remote model judges or autonomous experiment generation

## Upstream Port Rules

Before copying any Codex code:

1. Prove a small algorithm cannot be more cheaply reimplemented.
2. Identify exact upstream commit, files and tests.
3. Replace Codex-specific types and workspace dependencies.
4. Add an entry to `docs/development/UPSTREAM_PORT_LEDGER.md`.
5. Preserve license/notice requirements.
6. Record local tests, dependency cost and sync policy.

If these conditions are not met, use REFERENCE or REIMPLEMENT instead of PORT.

## Code Rules

- Use stable Rust pinned by `rust-toolchain.toml`.
- Commit Cargo.lock for the binary project.
- Keep `main.rs` thin.
- Prefer explicit enums and typed IDs over stringly-typed state.
- Keep Reducers, Upcasters, Fingerprints and normalization pure.
- Give each semantic rule one named owner. Repeated calls to that owner are allowed; copied validation, pairing, truncation, ordering, fingerprint or cancellation logic is not.
- Default to `pub(crate)`; test access alone does not justify a public API.
- Use the rule of three for general abstractions, while uniqueness and normalization invariants have one owner from their first implementation.
- Do not add semantic junk drawers such as `common.rs`, `utils.rs`, `helpers.rs`, `base.rs` or a generic `manager.rs`.
- Do not add a general Hook Registry. Keep fixed, named and typed stages inside the one AgentLoop until a second real implementation proves a Trait is needed.
- Do not add a general Prompt/Phase/Profile registry. Fixed compile-time DomainProfile and ResearchPhase resources compose through the existing Prompt/Context owners.
- Do not add mutable request-global Tool exposure. The immutable catalog, pure exposure resolver and SamplingStepPlan are the only model-visible Tool-plan path.
- Keep ContextFragment/Subitem identity in Domain. Optional Provider DeliveryItemId stays inside the Adapter and cannot cross Primary/Vision sessions.
- Model-visible Tool descriptions use the stable `Use when`, `Do not use when`, `Prefer` and `Produces` contract. Known result shapes require precise output schemas rather than a generic object.
- Tool failures have one provider-neutral structured owner with only `pre_execution` and `execution` stages. Only pre-execution argument failure may invite a corrected new ToolCall without a Receipt; every started execution has a real Receipt. Receipt/History/RequestAudit submission and other post-execution persistence failures are App/AgentLoop errors, never a rewritten successful ToolResult or a fabricated Receipt; Runtime never silently retries execution.
- Pass argv arrays; do not assemble untrusted shell strings inside tool adapters.
- Preserve Tool Call/Result pairing.
- Do not auto-retry completed, external-mutation or interactive tool execution.
- Do not cache nondeterministic or side-effecting results.
- Do not silently drop unsupported ContentItems or unknown Events.
- Never infer model capabilities from model-name patterns; use explicit versioned Capability Mapping.
- Primary and secondary ProviderSession IDs, response IDs and Incremental Baselines must never cross.
- Treat VisionObservation as a derived observation linked to source Artifacts, never as verified Finding evidence by itself.
- Treat Replay/Eval and Tool-accuracy reports as derived non-evidence over durable Trace facts only; unmatched Replay termination and Observation/Replay/Vision promotion remain Deferred without typed durable signals and must never be reported as zero. Deferred activation requires an adjacent RequestAudit ToolSpec transition.
- Avoid speculative abstractions and unused configuration.
- Remove code with no current runtime/test consumer before completing a Slice; record removed and deferred work in the Lean Review.
- A `pub` item is justified only by a current production consumer; a test-only consumer never justifies `pub`. Types with no production consumer and future placeholder/TODO surface must be removed or explicitly Deferred before a Slice is sealed. Controlled tests must not add a public projection entry point solely for test access.

## Credential Isolation

- Never read or modify `~/.codex`, `CODEX_HOME`, Codex OAuth/session data or Codex CLI login state.
- Never invoke `codex login`, `codex logout` or Codex CLI as a credential helper.
- Every live Provider credential is explicit Config v1 data: either a local inline `api_key` or a `credential_ref` in the `CYBER_AGENT_*_API_KEY` namespace. Never infer Provider/model identity from either form or fall back to generic vendor variables such as `OPENAI_API_KEY`.
- Every additional referenced Provider credential must use a dedicated `CYBER_AGENT_*_API_KEY` name and must not fall back across Primary/Vision roles. Inline credentials remain bound to their exact Provider entry.
- Use a real credential in a Provider request only when the user explicitly starts a live path. Loading an inline-key config parses the value, but `doctor`, Mock, Fixture and Replay do not send or display it.
- Keep local-development and GitHub Actions live-test keys separately revocable. A personal local Provider config may contain `api_key`; never commit that real config, a real `.env`, or credentials in shell profiles.
- Tests for credential resolution use an injected fake environment or credential source; they must not mutate the developer's real process environment.

## Tests and Commands

After Cargo exists, the default Gate is:

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
cargo build --locked
```

Use Mock/Fixture/Replay Providers by default. Live Provider tests must be explicit, ignored by default and gated by `CYBER_AGENT_LIVE_TESTS=1` plus a Cyber Agent-specific credential.

Generic Live Smokes select one explicit Config v1 through `CYBER_AGENT_LIVE_CONFIG`; they must not select a Provider endpoint or model through test code or model environment variables.

New behavior requires tests at the narrowest layer plus an integration test when it crosses Ports.

Permanent regression tests belong under top-level `tests/` and remain on `main`. Production modules under `src/` must not retain inline `#[cfg(test)] mod tests` after the Slice Gate. Temporary inline tests may be used only during uncommitted exploration and must be migrated to `tests/` or removed before completion; discovered regressions must be preserved before removal. Do not widen production APIs only to make tests reachable.

Every Gate also runs the inline-test scan defined in `docs/development/LEAN_CODE_GOVERNANCE.md`.

## File and Git Safety

- Preserve user changes and unrelated files.
- Use `rg`/`rg --files` for search.
- Use `apply_patch` for manual file edits.
- Never run destructive reset/checkout/clean commands without explicit authorization.
- Root `.gitignore` must ignore `/codex/`, `/target/` and `**/.cyber-agent/`.
- Do not commit credentials, target artifacts, research sessions or request snapshots.
- Do not commit, push, create PRs, tags or releases unless the current user request authorizes that action.

## Documentation Updates

Every completed Slice updates `STATUS.md`.

Update architecture/task/ADR only when behavior or contracts actually change. Do not edit frozen Architecture v1, v1.1, v1.2, v1.3 or v1.4 files; create a new ADR and architecture version.

New dependencies and Codex ports must be documented.

Changes to ownership, public API, dependencies, test layout or Codex reuse must also update or comply with `docs/development/LEAN_CODE_GOVERNANCE.md`.

## Stop Conditions

Stop and report instead of guessing when:

- The required change contradicts a frozen architecture invariant.
- A new fact source, crate, service or incompatible schema is required without ADR.
- Relevant user changes overlap and cannot be preserved safely.
- A live credential or external authorization is essential to continue.
- Tests prove the planned interface cannot meet the Slice Gate.

Otherwise make reasonable local assumptions, implement the current Slice, verify it and stop.
