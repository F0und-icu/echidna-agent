# ADR-0033 — Bounded pre-dispatch argument regeneration

- Status: ACCEPTED DESIGN by operator, implementation/verification in STATUS
- Date: 2026-09-23
- Architecture: additive v1.23, no remote seal

Explicitly supersedes ADR-0021/0031/0032's blanket no-automatic-model-retry rule
only for typed malformed JSON in a completed Tool argument field. It does not
authorize execution retry or guess repaired JSON. Operator accepted up to two
model corrections, reusing the existing Turn budget and successful continuation.

The Adapter remains strict and emits a provider-neutral diagnostic containing
only enum category and numeric coordinates/length. No raw argument, call/tool
identity or parser token enters the diagnostic. Other protocol/transport errors,
unknown execution, persistence failures and cancellation do not qualify.

AgentLoop rejects the whole uncommitted sample, including valid sibling calls.
No History, Tool result, Receipt, successful Baseline or response identity is
created from it. Earlier successful effects and completed reasoning remain.
The same AgentLoop may regenerate at most twice total per operator Turn, never
resetting this count after a success. All actual attempts, including existing
incremental fallback, consume the one Primary/Vision budget and original audit
path. Cancellation precedes another attempt; exhaustion returns normal input
recovery rather than guessing success. No automatic Resume or model switching.

A new version1 `model.sample_rejected` Session event binds diagnostic and used
correction count to the exact latest RequestAudit event before any correction.
The existing manifest binds the immutable plan; no new audit schema is added.
Reducer requires an active paired Session, latest audit identity, an unrejected
request and a correction count inside the shared bound. CLI chooses that exact
latest audit after the single Primary request returns, never a Vision identity.
This is an additive event in the existing stream, not another log or authority.
Its failure is fatal and prevents regeneration. Older readers reject the unknown
event; older events remain unchanged. It does not create a History item or Tool
Receipt. Runtime feedback is a Required, non-evidence SessionProjection fragment
in the existing turn-local Context channel, with exact provenance/audit/budget.
Protocol user-message encoding of context is not a user-authored History event.
The at-most-two feedback fragments are scoped to the current Turn's logical
Context; accepted provider delivery retains their exact bytes as needed for
existing continuation until reset. Resume never requeues correction feedback.

Each correction captures a fresh immutable plan through the same current-context
and Tool exposure owners. Feedback is appended once to retained delivery; prior
reasoning/call/result order and all incompatible-context stops remain. The UI
distinguishes rejected streamed prose from accepted output; no claimed rollback.

No new dependency, model router, configurable retry framework, raw logger,
automatic compaction, replacement Tool dispatcher or public test-only API.
Required fixtures: full/incremental correction; sibling-call atomicity; prior
effects once-only; exact feedback/audit/history separation; two-correction and
shared-budget exhaustion; cancellation; persistence failure; ordinary errors
not retried; prior reasoning/activation continuity and explicit Resume.
