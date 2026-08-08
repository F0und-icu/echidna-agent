# Cyber Agent Repository Instructions

## Purpose

This repository builds a single-user, local-first vulnerability research Agent. Keep it small, auditable, efficient and maintainable by one person.

These instructions are durable repository guidance for Codex. A current user instruction can override them explicitly.

## Start Here

Before changing anything, read in order:

1. `docs/development/STATUS.md`
2. The current Slice in `docs/development/IMPLEMENTATION_PLAN.md`
3. Relevant sections of `task.md`
4. `docs/architecture/CYBER_AGENT_ARCHITECTURE_V1.md` and the latest frozen additive architecture file
5. Relevant accepted ADRs, including ADR-0002 for model composition

Inspect the worktree and existing tests before planning changes.

## Current Phase

At creation of this file:

- Architecture v1.1 is sealed as an additive extension over the immutable v1 baseline.
- Rust implementation has not started.
- Local Git has not been initialized.
- `codex/` is a clean, read-only upstream reference repository.

Do not start M0 or initialize Git unless the current user request explicitly authorizes it.

## Slice Discipline

- Implement only the Slice named in `STATUS.md` and the current user request.
- Do not begin the next Slice after passing the current Gate.
- Prefer a thin end-to-end path, then failure paths and tests.
- Preserve all prior Slice behavior.
- Update STATUS at Slice start and completion.
- Stop after handing off completed verification.

Do not use multi-agent orchestration for this project unless the user explicitly changes the architecture decision.

## Architecture Invariants

- One Rust package and one default binary.
- No Cargo workspace unless an accepted ADR changes this.
- Internal direction: `ui/main -> app -> ports -> domain`; adapters implement ports and do not depend on app.
- Domain contains no filesystem, network, process, clock or Provider-specific I/O.
- OpenAI Responses types stay inside the OpenAI Adapter; Core uses ModelInputIR.
- Runtime has one primary ModelProvider and at most one optional vision ModelProvider; both remain inside one AgentLoop and one Session authority.
- Vision routing is capability-driven and limited to `auto`, `primary_direct`, `secondary_tool` or `disabled`; do not build a general model router.
- JSONL/Markdown/Artifact Blob are facts; Cache/Summary/Snapshot/Replay are derived.
- Event Streams are append-only and use chained Upcasters.
- TargetSnapshot/ExperimentCapsule/ExecutionReceipt preserve reproducibility.
- Tool Effect metadata controls retry/cache/scheduling, not approval.
- Runtime defaults to FullAccess; do not add Sandbox, Policy or confirmation systems.
- No telemetry, analytics, feedback or remote audit.
- Provider credentials are explicit Cyber Agent inputs; Codex/ChatGPT login state is never a credential source.
- CLI first; Web remains optional.

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
- Pass argv arrays; do not assemble untrusted shell strings inside tool adapters.
- Preserve Tool Call/Result pairing.
- Do not auto-retry completed, external-mutation or interactive tool execution.
- Do not cache nondeterministic or side-effecting results.
- Do not silently drop unsupported ContentItems or unknown Events.
- Never infer model capabilities from model-name patterns; use explicit versioned Capability Mapping.
- Primary and secondary ProviderSession IDs, response IDs and Incremental Baselines must never cross.
- Treat VisionObservation as a derived observation linked to source Artifacts, never as verified Finding evidence by itself.
- Avoid speculative abstractions and unused configuration.

## Credential Isolation

- Never read or modify `~/.codex`, `CODEX_HOME`, Codex OAuth/session data or Codex CLI login state.
- Never invoke `codex login`, `codex logout` or Codex CLI as a credential helper.
- The default OpenAI credential reference is `CYBER_AGENT_OPENAI_API_KEY`; do not fall back to `OPENAI_API_KEY`.
- Every additional Provider credential reference must use a dedicated `CYBER_AGENT_*_API_KEY` name and must not fall back across Primary/Vision roles.
- Resolve a real credential only when a user explicitly starts a live Provider path. Mock, Fixture and Replay paths require no credential.
- Keep local-development and GitHub Actions live-test keys separately revocable. Do not commit a real `.env` or place credentials in shell profiles.
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

New behavior requires tests at the narrowest layer plus an integration test when it crosses Ports.

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

Update architecture/task/ADR only when behavior or contracts actually change. Do not edit frozen Architecture v1 or v1.1 files; create a new ADR and architecture version.

New dependencies and Codex ports must be documented.

## Stop Conditions

Stop and report instead of guessing when:

- The required change contradicts a frozen architecture invariant.
- A new fact source, crate, service or incompatible schema is required without ADR.
- Relevant user changes overlap and cannot be preserved safely.
- A live credential or external authorization is essential to continue.
- Tests prove the planned interface cannot meet the Slice Gate.

Otherwise make reasonable local assumptions, implement the current Slice, verify it and stop.
