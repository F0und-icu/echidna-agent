# Cyber Agent Architecture v1.10 — Vision Coordinate Frame

- Version: 1.10.0
- Status: FROZEN ADDITIVE BASELINE (LOCAL; REMOTE HASH SEAL UNAUTHORIZED)
- Base: immutable v1 through v1.9
- Governing decision: ADR-0020
- Implementation/verification: STATUS.md

New secondary Vision observations bind region coordinates to exact delivered
Artifact pixel dimensions. Original/delivery metadata comes from immutable
image bytes and enters both RequestAudit and Observation v2 through existing
owners. One pure bounds validator rejects invalid rectangles; no clamping,
implicit source-coordinate conversion or automatic retry occurs.

Legacy v1 observations remain readable with unknown coordinate semantics.
Replay does not upgrade them or rewrite facts. Bounds-valid v2 observations
remain model-derived and unverified. Primary Direct, one AgentLoop, the single
Provider-neutral protocol/composition path and prior evidence boundaries remain.
