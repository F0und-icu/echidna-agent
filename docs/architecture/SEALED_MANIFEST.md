# Cyber Agent Sealed Documentation Manifest

## Seal 1

- Seal ID: `architecture-v1-governance-20260808`
- Date: `2026-08-08`
- Architecture status: `FROZEN`
- Implementation status: `NOT_STARTED`
- Root Git status: `NOT_INITIALIZED`
- GitHub repository: `F0und-icu/cyber-agent` private, intentionally empty at seal time
- Upstream Codex commit: `3aae5d885bac39c1262491aa3fd100dfd8b3919f`
- Upstream Codex worktree: clean at seal time

## Provenance

Architecture v1 was derived from `task.md` at SHA-256:

```text
31b1cca7205ccce86cc30be263648966c008515b3cb26aee8645cb06bb28a2a5  task.md@architecture-content-seal
```

After adding document references and the M0-S1 execution entry, the current governance-linked task file became:

```text
fb3ecae063676e3f2ba965d85d80b524ec2299e9c79c56f459644a41563575a5  task.md
```

The architecture semantics did not change between these two task hashes; the latter adds document routing and execution governance references.

## Sealed and Governance Files

```text
0b712f03e7b53c320af7505d292030e78298cdd7ad7274e2c0e8528dee2c95f1  AGENTS.md
7473a2ba74152ca393d19089ceaafee3960137c60c0e18996523c07de07b0e0c  Codex架构与漏洞挖掘Agent复用评估.md
6f187ede0c534ec0e650e0e30e8d3e6304f4d0de8ef679978729c244ddbf762e  docs/README.md
f1ca72c71171a548389e464ee2cf0f0201cfab2b3a57c034fe1119a1799925a7  docs/architecture/CYBER_AGENT_ARCHITECTURE_V1.md
773707bbf01bb291c299b920e4c401ded64cdbb309c6864399b0e76a6ec318f5  docs/development/IMPLEMENTATION_PLAN.md
09874eb233cb285af0e15b76ba44b673b732171ea55ff904773f2f3c15a2afab  docs/development/STATUS.md
78049b1e7ca2c3f00b1ab32ed824d8d55ddc8a09221b7bd67267e85dc3876852  docs/development/GIT_VERSION_RELEASE.md
d18d15acf5b4a051bd03ed985a7776be61233e2fd80547f2f5b0f559fe36ce2e  docs/development/DEVELOPMENT_ENVIRONMENT.md
102df4b6694affd302cd0a3962e8ceb1f34beefc9b40d4f1d029e2d90f888921  docs/development/UPSTREAM_PORT_LEDGER.md
37ce4cc345b9eb8351fdcbd14429aa44471aeccdeb008b4ce45cfd7b09799f4c  docs/adr/ADR-0000-template.md
f3e68726a09106af5216f0f27c1e68ae04b30830b7f6ee2a4287d2f73817d584  docs/adr/ADR-0001-specialized-single-agent-kernel.md
```

This manifest intentionally does not include its own Hash.

## Change Policy

- `CYBER_AGENT_ARCHITECTURE_V1.md` is immutable after the first authorized Git commit.
- Corrective or extended architecture is published as a new versioned file.
- `task.md`, AGENTS, development plans and STATUS may evolve through normal reviewed changes.
- Major governance or architecture changes require an ADR.
- A future seal appends a new section to this manifest; it does not overwrite Seal 1.

## Verification Boundary

At Seal 1:

- No Cargo.toml existed.
- No Rust source files existed outside the read-only `codex/` repository.
- No GitHub Workflow existed.
- No commit, push, PR, tag or release was created.
- The next implementation action was M0-S1 and still required explicit user authorization.

## Seal 2

- Seal ID: `architecture-v1.1-model-composition-20260809`
- Date: `2026-08-09`
- Architecture status: `FROZEN ADDITIVE BASELINE`
- Architecture version: `1.1.0`
- Base architecture: `1.0.0`, unchanged
- Governing ADR: `ADR-0002-capability-driven-primary-vision-models.md`
- Implementation status: `NOT_STARTED`
- Root Git status: `NOT_INITIALIZED`
- GitHub repository: `F0und-icu/cyber-agent` private and still intentionally empty
- Upstream Codex commit: `3aae5d885bac39c1262491aa3fd100dfd8b3919f`
- Upstream Codex worktree: clean at Seal 2 verification

### Decision

Architecture v1.1 adds capability-driven model composition while preserving one process, one AgentLoop and one authoritative Session/Research state:

- one required Primary ModelProvider;
- zero or one optional Vision ModelProvider;
- deterministic `auto`, `primary_direct`, `secondary_tool` and `disabled` modes;
- explicit versioned ModelCapabilities rather than model-name inference;
- no general model router, automatic Provider failure fallback or second autonomous Agent;
- isolated Primary/Vision ProviderSession, credential, audit and replay boundaries.

Architecture v1.0 remains immutable and must be read together with the v1.1 additive file.

### Provenance

Current task authority at Seal 2:

```text
b34a213928ed978732cf064f397a0c89b42e3a9c59ef3dbe2c644dea27001f8d  task.md
```

### Sealed and Governance Files

```text
fb9ff7c20026825d9bd092535898c86e9136cedf155b69632c1cd7dba4190918  AGENTS.md
b34a213928ed978732cf064f397a0c89b42e3a9c59ef3dbe2c644dea27001f8d  task.md
a808548e521c552537f443a1541171d7a38d40b86f108cd065eba1279098135f  docs/README.md
f1ca72c71171a548389e464ee2cf0f0201cfab2b3a57c034fe1119a1799925a7  docs/architecture/CYBER_AGENT_ARCHITECTURE_V1.md
1accf2fe1ff2b093b4b91b0f72231a8643fe150c41897ba5a97a2dd93f1ecd8f  docs/architecture/CYBER_AGENT_ARCHITECTURE_V1_1.md
f3e68726a09106af5216f0f27c1e68ae04b30830b7f6ee2a4287d2f73817d584  docs/adr/ADR-0001-specialized-single-agent-kernel.md
3dd70f459d53a6b31a5608d42ebd89dfee0839830c9c36d0c05ce8e8b6e7263a  docs/adr/ADR-0002-capability-driven-primary-vision-models.md
e16559609b9d890034a00e992037812c2ed573c36194ca2a5636f27d78c8cfd5  docs/development/IMPLEMENTATION_PLAN.md
52e7a686a355993b921c512ace09c610238197e44ef8645de8571209179825a9  docs/development/STATUS.md
6ffd27e4fced1a18885981e29c00895d102e3a2c3d6dce94e50035d9d3df6382  docs/development/GIT_VERSION_RELEASE.md
4d0d4b7831cb9b089ebef1070f3ca7ee3a802dbd9edea318cbaad52675c10187  docs/development/DEVELOPMENT_ENVIRONMENT.md
102df4b6694affd302cd0a3962e8ceb1f34beefc9b40d4f1d029e2d90f888921  docs/development/UPSTREAM_PORT_LEDGER.md
```

This manifest intentionally does not include its own Hash.

### Verification Boundary

At Seal 2:

- No Cargo.toml or Rust source existed in the Cyber Agent root.
- No local root Git repository or GitHub Workflow existed.
- No model credential was configured, read or modified.
- No live model API request was performed.
- The current Codex login state and `~/.codex` were not inspected or changed.
- M0-S1 remained `READY_NOT_AUTHORIZED`.

## Seal 3

- Seal ID: `documentation-baseline-published-20260809`
- Date: `2026-08-09`
- Architecture status: `FROZEN ADDITIVE BASELINE` (`1.1.0`)
- Implementation status: `NOT_STARTED`
- Root Git status: `INITIALIZED`
- Baseline commit: `55ff3be` (`docs(governance): publish design baseline`)
- GitHub repository: `F0und-icu/cyber-agent` private; baseline pushed to `main`
- Upstream Codex commit: `3aae5d885bac39c1262491aa3fd100dfd8b3919f`
- No Cargo package, Rust source, Workflow, live Provider request or research data was added.

### Verification Boundary

At Seal 3:

- Staged and published files were limited to governance, architecture, ADR, development-plan and reference-design documents plus the root `.gitignore`.
- `codex/`, `deepseek`, `.cyber-agent/`, build output, logs, credentials and target data were excluded from Git.
- M0-S1 remained `READY_NOT_AUTHORIZED`.
