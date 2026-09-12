# Cyber Agent Architecture v1.12 — Runtime Process Handle Isolation

- Version:1.12.0
- Status:FROZEN ADDITIVE BASELINE (LOCAL; REMOTE HASH SEAL UNAUTHORIZED)
- Base: immutable v1 through v1.11
- Decision: ADR-0022
- Verification is governed by STATUS, not by this contract freeze.

Job and PTY handles bind to a runtime-local table namespace and checked
sequence through one private Adapter owner. New runtimes do not reissue bare
legacy counters or recover control of old processes. Exact table membership
is the sole operation authority; absent handles fail explicitly without
reattachment. Domain IDs/Tool schemas/Session facts remain unchanged.

Output retention and partial-execution checkpoints are separate planned
contracts, not implemented or validated by this identity extension.
