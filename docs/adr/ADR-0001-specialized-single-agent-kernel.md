# ADR-0001: Build a Specialized Single-Agent Kernel

- Status: ACCEPTED
- Date: 2026-08-08
- Deciders: project owner
- Architecture version: 1.0.0
- Related Slice: M0-S1 onward
- Supersedes: direct Codex CLI trimming proposal
- Superseded by: none

## Context

The local Codex source provides high-quality examples for model streaming, tool loops, history normalization, context fragments, multimodal input and incremental requests. Its full product architecture also includes many team, cloud, policy, sandbox, connector, UI and cross-platform responsibilities that are not required by a personal vulnerability research Agent.

Physically deleting those product modules would create a long-lived fork with a large dependency closure and high upstream synchronization cost.

## Constraints

- One person uses and maintains the Agent.
- Runtime defaults to FullAccess.
- No telemetry, cloud server, team collaboration or approval product.
- Project isolation, memory, tools, context and audit remain mandatory.
- Multimodal input remains mandatory.
- CLI is the primary interface.
- The result must be implementable and testable as one Rust package.

## Options Considered

### Option A: Trim the complete Codex workspace

Benefits:

- More existing behavior is immediately present.

Costs:

- Large workspace and dependency closure.
- Product features remain coupled even when disabled.
- Difficult compilation and upstream synchronization.
- Specialized semantics remain an overlay on a general agent.

### Option B: Build a dedicated kernel and port small algorithms

Benefits:

- Product boundary matches actual use.
- Single package and binary remain understandable.
- Vulnerability research state becomes a first-class domain.
- Upstream code is reused only where tests and dependency cost justify it.

Costs:

- AgentLoop, research state and adapters must be implemented.
- Existing Codex behavior cannot be assumed; it needs local tests.

## Decision

Choose Option B.

Build Cyber Agent as a new single-package Rust project. Reimplement the small Agent kernel around Domain/Ports/App/Adapters. Port only independent algorithms and tests that pass the Upstream Port Ledger admission gate.

Keep the local `codex/` repository read-only and outside Cyber Agent Git history.

## Consequences

### Positive

- No inherited cloud/team/policy product layer.
- Stable local data ownership and audit model.
- Research-specific TargetSnapshot, ExperimentCapsule and Evidence references.
- Easier compilation, review and personal maintenance.

### Negative

- Initial M0-M3 engineering work is larger than a prompt-only Codex customization.
- Compatibility with Codex product behavior is not a goal.
- Provider and tool semantics must be explicitly specified.

### Deferred

- Additional model providers.
- Local Web UI.
- SQLite FTS retrieval index.
- MCP Adapter.

## Compatibility and Migration

- No Cyber Agent runtime data exists yet.
- Architecture v1 establishes Event Upcasters before the first released data schema.
- Future incompatible architecture changes require a new ADR and frozen architecture version.

## Implementation Plan

Follow `docs/development/IMPLEMENTATION_PLAN.md` from M0-S1. Do not begin the next Slice automatically.

## Validation

- One package and default binary.
- Builds without the local `codex/` directory.
- No forbidden Codex crate dependencies.
- M1a completes a real single-tool loop.
- M4 completes a long vulnerability research workflow with replayable local evidence.

## Revisit Trigger

Revisit only if measured implementation cost shows the dedicated kernel is materially harder to maintain than a pinned, narrow upstream library boundary that does not import Codex product layers.
