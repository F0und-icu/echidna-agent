# Cyber Agent Architecture v1.14 — Observed Child Checkpoints

- Version: 1.14.0
- Status: FROZEN ADDITIVE BASELINE (LOCAL; REMOTE HASH SEAL UNAUTHORIZED)
- Base: immutable v1 through v1.13
- Decision: ADR-0024; exact implementation and verification: STATUS

One fixed Tool observation/acknowledgement path carries actually completed
command children to the existing App publication and Session Event owners.
Raw Blob publication and a strict child checkpoint Event precede acknowledgement
and later dependent execution. Child facts never create additional ToolCalls,
parent Receipts or Research evidence. Errors stop at the App boundary without
retry or fictional cancellation of unknown work.

Resume/Compaction use the existing History owner to retain known child facts
beside explicit unknown parent state. Durable History is not rewritten and
Replay does not manufacture execution facts. Progress presentation follows
durability and cannot block pipe draining; R3b1 alone does not close R3b2 UI
acceptance. No new store, dependency, general observer registry or daemon.
