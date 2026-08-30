# Cyber Agent Architecture v1.6 — Redundant Encrypted Reasoning Sidecars

- Version: 1.6.0
- Status: FROZEN ADDITIVE BASELINE (LOCAL; REMOTE SEAL UNAUTHORIZED)
- Date: 2026-08-30
- Base architectures: v1.0 through v1.5
- Governing decision: ADR-0016
- Related Slice: F1e
- Implementation status: COMPLETE_LOCAL; REMOTE SEAL UNAUTHORIZED

## 1. Reading rule

Read v1.0 through v1.5 and this file together. This version changes only the
Responses Adapter treatment of a redundant opaque encrypted sidecar observed
next to complete plaintext reasoning. Every prior AgentLoop, History, Context,
Tool, Evidence, Vision and FullAccess invariant remains unchanged.

## 2. Supported semantic continuation

`reasoning_text` remains the only supported provider-neutral reasoning
continuation. A completed item with valid non-empty plaintext may also contain
opaque encrypted wire metadata. That sidecar is not a second semantic item and
does not cross the Adapter boundary.

The Adapter must still reject encrypted-only reasoning and every unknown or
malformed reasoning content type. It must never infer support from Provider,
model, URL, credential or sidecar shape.

## 3. Acceptance invariants

- Plaintext reasoning plus an opaque encrypted sidecar decodes to exactly one
  completed `ReasoningItem`.
- Encrypted-only continuation fails before History, Baseline or delivery state
  advances.
- Generic Full replay remains `reasoning -> function_call ->
  function_call_output`.
- No encrypted value, wire ID, new config field, vendor branch, dependency,
  second Adapter, second History or second AgentLoop is introduced.
