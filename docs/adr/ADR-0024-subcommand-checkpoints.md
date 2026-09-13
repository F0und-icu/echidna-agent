# ADR-0024: Observed Subcommand Checkpoints

- Status: ACCEPTED for the operator-authorized CTF-R3b1 repair
- Architecture: additive v1.14; R3b2 progress presentation remains separate
- Preserves: ADR-0017 unknown execution, ADR-0023 bounded raw publication

## Problem and decision

The command adapter retains completed children until a whole run_commands
future returns. Dropping that future can lose already observed results even
though their side effects happened. A terminal parent Receipt cannot recover
those uncommitted observations. Keep one AgentLoop and Session Event store;
do not turn children into additional ToolCalls, Receipts or Research evidence.

One fixed optional observed-execution path on the Tool Port carries bounded
command-completion observations to the owning scheduler. Other Tools retain
their existing execution path. Reuse the existing async channel facilities,
not a new dependency, observer registry, daemon or general event bus.

The Adapter owns process observation only. After an actual child completion,
App publishes its bounded raw bytes through the existing R3a2 owner, then the
existing CLI Session stage durably appends one `tool.subcommand_checkpointed`
Event. Only after successful append does App acknowledge the observation.
Serial execution cannot launch the next child before acknowledgement;
parallel execution checkpoints each completed child without awaiting a whole
wave. Existing resource conflict and raw/preview budgets are unchanged.

The bounded acknowledgement returns published references, avoiding another
raw publication at parent completion. An observation channel closing cannot
authorize later child launches or a fabricated terminal outcome. Its producer
future remains owned by the stopping scheduler. Publication or Event append
failure is a fatal App error: preserve earlier checkpoints, stop unknown work,
never rewrite it as ToolFailure, retry execution or claim rollback.

## Facts and compatibility

Child Event payload v1 binds a parent execution ID, ToolCall ID/name and plan
fingerprint to an original zero-based child index, child count, observed
status/result and bounded raw capture references. It has strict field/schema
validation, parent/index agreement, uniqueness and one pure metadata owner.
Only supported command observations are admitted. A completed child is not a
terminal parent Receipt. Parent Call remains the existing durable History
fact; no new model-originated call or execution ID is manufactured.

The final parent ToolResult/Receipt still follow the existing one-call/one-
result rule and agree with its known child checkpoints. Known executor
timeout/cancel outcomes can carry already checkpointed children, but say
nothing about unobserved effects. Active futures dropped by the App still
have no terminal Receipt/ToolResult. Unknown children remain unknown; lack of
a checkpoint never means not_started. Docker-client termination never proves
that a container process stopped.

Resume reads the same child Events. The sole History normalization owner may
enrich its existing derived unknown-parent marker with known child facts and
Blob refs. It never rewrites durable History, closes an unknown execution as
success or retries a parent. Session/Compaction validation consumes that same
projection, including exact-prefix checks; no second History normalizer or
fact source. Mechanical Replay remains derived and cannot manufacture child
Events, output bytes or evidence from incomplete source execution.

Existing Receipt v2 and old child-free Events remain readable unchanged. New
child Events are additive schema v1; old binaries may reject them. Preserve
Session/Blob data and retain a compatible reader on rollback. No Config,
Provider wire binding, model capability inference or new ArtifactStore Port
method is introduced. Tool output changes are versioned in the catalog.

## Cancellation and presentation

App processes observations already accepted by its bounded channel before
dropping producers at a cancellation boundary; append/ack ordering is not
interruptible halfway into a fictional success. No acknowledgement is sent
on failed publication. Abrupt process death can leave an unacknowledged
checkpoint durable; Resume reads that fact without replaying execution.
There is no promise to journal bytes/results not yet observed or appended.

R3b1 establishes the durable data path and existing CLI inspection/Resume
consumers. R3b2 separately consumes successful checkpoints for progress UI,
including slow-terminal/backpressure verification. Synchronous terminal
printing is not accepted as a substitute for execution durability, and the
whole R3b work package remains open until its presentation requirements pass.

## Acceptance

Use bounded synthetic commands only. Cover serial/parallel first-child
completion while another child blocks; tool timeout, Ctrl+C and killed-CLI
Resume; durable publication failure; exact bytes/readback, capture ceilings,
duplicate/invalid child Events, parent result agreement, legacy replay and
Compaction. Assert durable facts and real execution counters, not just UI
strings or a parent Cancelled flag. No automatic rerun, fabricated Receipt,
unbounded queue, test-only public API or weakening of prior refusal rules.

Focused and affected tests precede fresh independent Terra xhigh review,
repairs, complete Gate and Lean. A failing interface is revised before code
expansion rather than hidden behind placeholders. Live stays within the
existing separate series ceilings after offline closure.
