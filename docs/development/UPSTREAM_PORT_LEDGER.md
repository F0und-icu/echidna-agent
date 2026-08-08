# Upstream Codex Port Ledger

## 0. Purpose

This ledger is the authoritative record for code copied or mechanically ported from the local `codex/` reference repository.

Architecture inspiration, interface similarity and independent reimplementation do not count as PORT, but should still be named in design documentation when material.

## 1. Locked Upstream Baseline

- Repository path: `codex/`
- Commit: `3aae5d885bac39c1262491aa3fd100dfd8b3919f`
- Repository role: read-only reference
- Cyber Agent dependency: none

## 2. Current Entries

No Codex source code has been ported into Cyber Agent.

## 3. Entry Template

Copy this section for each PORT:

```markdown
### PORT-XXXX: <capability>

- Status: PROPOSED | ACTIVE | REPLACED | REMOVED
- Cyber Agent Slice:
- Local owner module:
- Upstream repository:
- Upstream commit:
- Upstream file paths:
- Upstream test paths:
- License/SPDX:
- Copyright/NOTICE action:
- Ported symbols/algorithms:
- Approximate copied lines:
- Codex-specific types removed:
- New local dependencies:
- Local behavior changes:
- Local tests:
- Benchmark/benefit:
- Upstream sync policy: pinned | manual-review | no-sync
- Removal/reimplementation trigger:
- Notes:
```

## 4. Admission Gate

A PORT entry can become ACTIVE only when:

- Exact upstream files and tests are identified.
- The needed subset is materially smaller than the dependency closure.
- `codex-core`, `codex-tools`, `codex-protocol` or workspace closure is not imported directly.
- Local types replace Codex product-layer types.
- License and Notice actions are complete.
- Local unit/contract tests pass without the upstream repository present.
- Compile-time and dependency impact are recorded.
- A maintainer can explain how to update or remove the port.

## 5. Expected Candidate Areas

Candidates that may qualify after inspection:

- Tool Call/Result pairing normalization.
- Output truncation algorithm and tests.
- Context Fragment hashing/order tests.
- Image validation/resize subset.
- Streaming Responses decoding subset.
- previous_response_id comparison fixtures.

These are candidates, not approved PORT entries.

## 6. Reimplementation Areas

The following remain REIMPLEMENT unless a new ADR proves otherwise:

- AgentLoop.
- ModelInputIR.
- ProviderSession ownership.
- Research Ledger.
- TargetSnapshot/ExperimentCapsule.
- Artifact Lineage.
- Tool Runtime and Apply Patch Adapter.
- PTY Runtime.
- Replay/Eval.
- Observation/TargetDelta.

## 7. Verification

CI should eventually verify Cyber Agent builds and tests when the local `codex/` directory is absent. Any accidental compile-time dependency on it is a release blocker.
