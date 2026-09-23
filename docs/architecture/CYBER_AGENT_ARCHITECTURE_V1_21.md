# Cyber Agent Architecture v1.21 — Explicit instruction retention

- Status: LOCALLY REVIEWED / GATE AND LEAN VERIFIED; not remotely hash-sealed
- Base: immutable v1 through v1.20
- Decision: ADR-0031; execution/verification: STATUS

Explicit Project Prompt bytes use the existing Required-fragment semantics:
complete delivery or pre-sampling budget refusal. Config paths, current-context
refresh, hashing, provenance and rendering retain their existing single owners.
No inferred task-contract store or automatic directory discovery is introduced.

Base generation3 adds bounded historical-result attribution and efficiency
guidance; Summary v2 explicitly warns of omitted historical tool observations.
Guidance is not a factuality guarantee or evidence. Old bytes remain Replay
authority. No new model router, automatic tool retry or JSON repair.

An optional declared supported output ceiling uses the existing Capability Mapping
and neutral RequestRequirements. ContextPlanner resolves/reserves it before the
immutable plan and RequestAudit; the Adapter only encodes the captured value.
Absent fields retain legacy serialization; changed values affect capability and
output-contract fingerprints. Primary/Vision remain separate. Incomplete output
stops atomically and needs explicit Resume; no billing guarantee, automatic retry,
reasoning-effort downgrade or vendor inference. See ADR-0031 for exact boundaries.

Malformed-argument diagnostics contain metadata only. A failed decoder never
commits a sample or invokes its calls. Release keeps reasoning presentation and
excludes full debug; no persistent raw response log is added.
