# Cyber Agent Architecture v1.7 — Interactive Runtime Continuity

- Version: 1.7.0
- Status: FROZEN ADDITIVE BASELINE (LOCAL; REMOTE SEAL PENDING)
- Governing decision: ADR-0017
- Base: immutable v1.0 through v1.6
- Implementation: governed exclusively by `docs/development/STATUS.md`; architecture freeze alone does not establish implementation or verification

Read all prior versions together. Rejection of an intact historical Compaction solely due to a later Research suffix is superseded: verify the historical prefix and rebuild a separately validated current Compaction before Resume. The earlier deferred runtime Prompt file-loader prohibition is superseded only for two explicitly configured Personal/Project files, without discovery or a registry. Never apply stale EvidenceSpine as current evidence, rewrite historical facts, or weaken exact-TargetSnapshot applicability.

Production interactive Chat checkpoints History at user/model/result boundaries using the existing Session stream. Model output precedes dependent Tool execution; real Receipts and paired results precede resampling. Persistence failures remain App errors. Recovery exposes uncertainty and never automatically reruns execution.

Unobserved active futures interrupted by cancellation are not terminal execution facts: no synthetic cancelled Receipt/ToolResult is emitted. Known completed outcomes remain durable; the Turn stops and unmatched Calls remain explicitly unknown in derived recovery. This narrowly supersedes the old scheduler cancellation assumption, without adding an Event schema or fact store.

One fixed pre-sampling composition refreshes current Research/Project/Memory facts while retaining immutable SamplingStepPlan and Turn-boundary operator selections. Personal and Project Prompt sources remain explicit and use the existing renderer/fingerprint/provenance path. Missing optional context is distinct from malformed context. Retrieval/guidance stays Shadow.

Compaction remains explicit, local, deterministic, reference-based and bounded. Candidate construction and budget/reduction validation precede durable application. Provider connection/read deadlines are explicit non-semantic transport settings with compatible defaults and no automatic retry.

All prior one-package, one-AgentLoop, FullAccess, credential isolation, no-telemetry, no-cloud, no-registry, no-auto-retry and fact-authority invariants remain. Benchmark and non-interactive execution are outside F3. The architecture seal manifest is not updated by local implementation.
