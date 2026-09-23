# Cyber Agent Architecture v1.23 — Bounded argument regeneration

- Status: LOCAL ACCEPTED DESIGN; verification in STATUS
- Base: immutable v1 through v1.22; ADR-0033
- No remote hash seal

Only a typed malformed Tool-argument JSON diagnostic before sample commit may
trigger at most two model corrections per Turn inside the existing AgentLoop.
This narrowly supersedes earlier blanket automatic model-retry prohibitions.
Tools are never replayed, invalid JSON is not repaired by heuristic, and prior
successful History/Receipts/Provider reasoning are neither discarded nor reset.

Existing Session facts record each rejected sampling step before dependent
regeneration. Bounded SessionProjection feedback uses the existing required
Context fragment, provenance and RequestAudit owners, never operator History
or fabricated ToolResult. New samples consume original shared admission slots.
Cancellation, nonmatching protocol failures, budget/Context/integrity failures
retain existing stops. One AgentLoop, one Provider path and no new dependency.
