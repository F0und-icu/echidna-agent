# Cyber Agent Architecture v1.11 — Bounded Interactive Recovery

- Version: 1.11.0
- Status: FROZEN ADDITIVE BASELINE (LOCAL; REMOTE HASH SEAL UNAUTHORIZED)
- Base: immutable v1 through v1.10
- Governing decision: ADR-0021
- Implementation/verification: STATUS.md; a frozen contract is not a test-pass claim

One private App admission budget is shared by Primary and scoped Vision for
each operator Turn, configured through the existing Config v1 Composition Root.
Default 32 is preserved; explicit values are bounded to 1..=10000. Admission
counts are conservative attempt limits, not observed server usage or billing.

Only typed, state-validated model/context/budget stops can return to CLI input.
Explicit `/resume` rebuilds the existing ProviderSession from durable Session
facts before a new task; no automatic retry, old-input duplication, reasoning
discard at stop or Tool re-execution. Unknown in-flight execution and integrity
or persistence errors remain fatal. Current-context refresh, one AgentLoop,
strict History/Receipt authority and manual Compaction remain unchanged.
