# ADR-0021: Bounded Interactive Recovery and Shared Request Admission

- Status: ACCEPTED for the operator-authorized CTF-R2b repair
- Architecture version: 1.11.0
- Supersedes: CLI process exit on every model/context stop; the hard-coded Primary-only 32-request limit
- Preserves: ADR-0015/0017 exact continuation, explicit Resume, durable checkpoints and unknown-execution boundaries

## Decision

One private App request-budget owner admits Primary and scoped Vision attempts
under a shared per-operator-Turn ceiling. Config v1 gains optional
`[runtime] model_request_limit_per_turn = 32`, validated in the single
Composition Root in the range 1..=10000. Existing configs keep default 32.
The CLI creates this budget before initial Vision, and passes it through the
existing fixed stages; Direct images consume no separate Vision request.
Standalone AgentLoop consumers use the same owner's default, not another cap.

Admission is immediately before RequestAudit/stream dispatch after local
candidate validation. It is deliberately conservative: an admitted attempt can
fail audit/transport or be cancelled before bytes are sent, and is not refunded.
Counters report admitted Primary/Vision/total and remaining capacity, never
claim server usage or wire delivery from admission alone. Thus actual requests
cannot exceed the ceiling. Existing incremental-rejection fallback, when
applicable, consumes another slot; no new fallback or retry is introduced.
The final admitted response's accepted ToolCalls finish under existing rules;
no extra summary request is allowed. New operator input starts a fresh budget;
CLI commands and `/resume` do not sample. Cross-Turn Live ceilings remain the
external acceptance harness's responsibility, not a new product job runtime.

The App classifies only paired-history request-limit, continuation refusal,
specific Context capacity errors and pure model-stream/empty-response errors
as resumable stops. All persistence, audit, provenance, unknown execution,
receipt mismatch, presentation and unclassified errors remain fatal. CLI
validates the durable Session before allowing further interaction; no
catch-all retry is added. Existing cancellation semantics remain unchanged.

A resumable stop returns to `you>` with its cause and admission counts, but
blocks new task input until the operator explicitly enters `/resume`.
`/context` and `/exit` remain available. `/resume` uses the same strict Session
replay/Compaction-reference validation and fresh Primary ProviderSession as
process Resume. Only that explicit operation clears volatile completed
reasoning; stop itself does not. No old input or tool is automatically resent.
After Resume, the operator may `/compact` manually or enter a new instruction.
The next sample is Full, with current Prompt/Memory/facts and durable History.
An invalid recovery history is rejected before appending the Resumed event.

Initial secondary-Vision failures have no accepted Primary user Turn. An
admission failure is reported before its RequestAudit; an observed Provider
failure is recoverable only after its existing durable Vision failure is
committed. The selected pending image is consumed, not silently queued for
retry; the UI says to reattach it explicitly if desired. Model-originated
image follow-up failure never reruns the already-receipted view_image call.
Other Vision failures (including raw Blob/audit/terminal persistence) stay fatal.

## Ownership and compatibility

No new crate, Port, protocol field, Event, fact store, renderer, router, Hook
registry, automatic compaction or reasoning persistence. One private App budget
module owns admission; the existing App error path owns stop classification;
CLI owns interaction and strict Session handoff. Config is backward-readable
by new builds; old strict Config v1 readers reject the new optional table.
Public error variants have real CLI/analysis consumers, never test-only APIs.

## Required verification

Fixtures exercise 1/2/default32/explicit-above32 admission; shared initial and
follow-up Vision slots; incremental fallback charged twice; no audit/stream
after rejection; return to prompt, explicit recovery with no duplicated input
or tool effect, Full-only and incremental identity isolation; capacity and pure
Provider failures; malformed durable History, Receipt/Result and checkpoint
failure remain fatal. Keep manual Compaction and release behavior regressions.
Independent Terra xhigh review and complete Gate/Lean precede bounded Live.
