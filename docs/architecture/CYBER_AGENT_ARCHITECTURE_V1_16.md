# Cyber Agent Architecture v1.16 — Explicit Terminal Projections

- Status: FROZEN ADDITIVE BASELINE (LOCAL; REMOTE HASH SEAL UNAUTHORIZED)
- Base: immutable v1 through v1.15
- Decision: ADR-0026; implementation and verification: STATUS

Development debug explicitly omits Tool definitions from its selected content
view while preserving actual Tool delivery and Full/incremental identity.
Assistant display separates sampling replies without changing History.
Bounded raw readback hints derive only from existing durable capture metadata;
UI truncation does not imply complete raw retention. No new I/O or fact owner.
Release retains Provider-returned reasoning and excludes debug. All earlier
contracts remain except ADR-0019's mandatory Tool-schema terminal display.
