# Cyber Agent Architecture v1.13 — Retained Process Output

- Version: 1.13.0
- Status: FROZEN ADDITIVE BASELINE (LOCAL; REMOTE HASH SEAL UNAUTHORIZED)
- Base: immutable v1 through v1.12
- Decision: ADR-0023; implementation and verification: STATUS

Bounded raw process captures are separate from lossy model/CLI previews. One
fixed App completion stage publishes retained bytes through ArtifactStore
before Receipt, dedupe, History and resampling. Typed Receipt v2 references
and ToolResult references agree; v1 reads with explicit legacy absence. Blob
references do not fabricate Research Artifact provenance. Publication errors
are fatal App errors without retry or rewritten execution outcomes.

Bounded readback revalidates bytes through the existing Blob owner. Running
process output remains volatile; terminal snapshots are durable only after
publication. Unobserved dropped futures remain unknown. Child checkpoints are
not established by this extension. No new fact store, registry or dependency.
