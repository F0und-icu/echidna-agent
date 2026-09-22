# ADR-0029: Response text finalization and proven Tool activation expiry

- Status: ACCEPTED for operator-authorized R6 reliability repairs
- Date: 2026-09-19
- Architecture: additive v1.19; verification remains governed by STATUS.

## Response integrity

The protocol Adapter previously ignored output_text.done, message content and
completed output snapshots. A private response-local text owner reconciles
these with emitted deltas, keyed by output/content positions and checked item
identity. Equal final forms are deduplicated; an exact missing suffix may be
emitted only before a later semantic output makes insertion out of order.
Conflicts, ambiguous identities, unsupported content and non-completed terminal
status fail explicitly. Never invent text, silently rewrite streamed content,
automatically continue or retry. Legacy single unindexed text streams remain
supported where their identity is unambiguous. Reasoning and Tool semantics
stay separate, and failure cannot commit a successful response.

This fixes a fixture-provable decoder gap; it does not establish the cause of
the historical `## Prev` answer without its missing raw upstream body. Provider
completion is not a guarantee that generated prose is substantively complete.
Reference: https://developers.openai.com/api/reference/resources/responses/streaming-events#response.output_text.done

## Activation expiry

Extend ADR-0028 only for exact expiry of the previous Turn's temporary activated
ToolSpecs. Retain the Turn's immutable pre-activation resolved plan alongside
the successfully committed delivery (not a dispatch authority). At the first
Step of a new Turn this must equal the complete current resolved plan; prior
advertised specs must equal the current specs plus only exact Direct promotions
of that plan's Deferred specs. Catalog/exposure changes therefore do not qualify.
Reuse the original Baseline comparator with its Tool-hash override to verify
all non-Tool semantics and committed input/output prefix, with no pending reset.
No removals caused by changed catalog/exposure or redefinition are admitted.
Activation still clears; the expired tools are not advertised or dispatchable.
No auto-Resume, reasoning discard, retry or second mutable Tool authority.

## Bounded guidance

Tool discovery preserves eligibility/Hidden rules and strict all-term fallback;
an exact Tool-name token takes precedence over descriptive terms. Shared ToolSpec
guidance distinguishes the current sent plan from discoverable tools, real
Artifact provenance from raw captures, observed timestamps from guesses, and
optional deliberate reruns from first execution. Existing command execution can
observe UTC time; no clock service, implicit timestamp substitution or schema
field is introduced. Prompt compliance is not a correctness guarantee.

No new public API, Port, persisted schema, dependency, vendor binding or remote
fact authority. Existing audit/fingerprint owners observe changed ToolSpecs.
Prior architecture files and remote seals remain untouched. Regressions cover
suffix/dedup/conflict/order/status, exact-name discovery/Hidden/limits and
expiry with incompatible changes. Focused/affected tests, independent review,
Gate and Lean precede bounded raw-SSE Live acceptance.
