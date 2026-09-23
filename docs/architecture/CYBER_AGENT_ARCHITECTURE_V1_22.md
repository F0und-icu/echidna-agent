# Cyber Agent Architecture v1.22 — Operator steering and background control

- Status: LOCALLY REVIEWED / OFFLINE VERIFIED; implementation/verification governed by STATUS
- Base: immutable v1 through v1.21; decision ADR-0032
- No remote hash seal.

CLI terminal steering requests cooperative cancellation through the existing
owner before submitting a new instruction at the next Turn boundary. It never
mutates an in-flight sampling plan. Piped input remains sequential. Recovery
and unknown-side-effect boundaries remain unchanged; no automatic Resume/retry.

Base generation4 treats current operator scope as controlling over old plans,
with bounded-task stop guidance. This does not enforce semantic compliance.
Historical prompt/audit bytes remain immutable.

Pure tool discovery remains available during live process leases, so deferred
lifecycle tools remain discoverable. Actual execution conflict rules and exact
resolved-plan dispatch remain unchanged. No generalized exemption/registry,
daemon, new authority, dependency or persistent activation is introduced.
