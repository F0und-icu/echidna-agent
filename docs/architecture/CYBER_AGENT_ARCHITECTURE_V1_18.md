# Cyber Agent Architecture v1.18 — Additive Tool Continuation

- Status: FROZEN ADDITIVE BASELINE (LOCAL; REMOTE HASH SEAL UNAUTHORIZED)
- Base: immutable v1 through v1.17
- Decision: ADR-0028; implementation and verification: STATUS

The existing verified full-plus-state-diff continuation may accompany a
ToolsChanged Full request for additive current-Turn Tool activation only after
the original Baseline owner proves unchanged non-Tool semantics and exact
committed input/output prefix, without pending reset. Prior ToolSpecs remain
exact; removed/redefined tools are not admitted by this extension. Sent plans
are never rewritten for comparison. The existing exact reasoning replay path
and invalidation priority remain unchanged.

One shared Domain relationship rule admits the current request/audit contract;
provenance, complete reasoning, paired History, budget and state projection stay
with their existing owners. No new fact/schema/API/dependency, alternate
continuation, automatic retry, silent discard or vendor branch. Incompatible
combined changes remain explicit stops. Earlier files/seal manifest stay frozen.
