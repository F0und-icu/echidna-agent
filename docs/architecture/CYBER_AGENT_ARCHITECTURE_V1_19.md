# Cyber Agent Architecture v1.19 — Finalization and activation expiry

- Status: FROZEN ADDITIVE BASELINE (LOCAL; REMOTE HASH SEAL UNAUTHORIZED)
- Base: immutable v1 through v1.18
- Decision: ADR-0029; implementation and verification: STATUS

One response-local Adapter owner reconciles final text with streamed text.
Only an exact missing suffix in valid semantic order may be emitted; conflicting
or ambiguous finalization fails, never rewrites or automatically continues.
Wire positions/identities remain Adapter-local. A completed event cannot mask
an explicitly non-completed status. No successful History commit on failure.

ADR-0028 additionally permits precisely proven previous-Turn activation expiry:
the prior Turn's immutable pre-activation resolved plan must equal the complete
current plan, with only exact Deferred promotions removed from delivery and the original
non-Tool/prefix proof. No blanket removal/redefinition exception; next-Turn
dispatch still excludes expired Tools. All provenance/budget checks remain.

ToolSpec owns exact-name discovery and concise provenance/time/rerun/visibility
guidance. No new evidence, schema, dependency, API, registry or clock framework.
