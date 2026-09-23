# ADR-0032 — Operator steering and background control

- Status: ACCEPTED / LOCALLY OFFLINE VERIFIED for operator-authorized R7-F2; evidence and limits in STATUS.
- Date: 2026-09-23
- Architecture: additive v1.22, no remote hash seal.

The line CLI currently reads input only between Turns. Job/PTY leases with
unknown resources are globally serial and also reject pure tool discovery,
making deferred lifecycle controls unreachable. These are separate defects.

Interactive terminal input is observed alongside the current Primary or initial
Vision future. `/stop` or Ctrl+C requests existing cooperative cancellation;
a nonempty corrective line first cancels the current Turn and is queued only
for the ordinary next input boundary. Slash commands are handled there as well,
never inserted into model input during an immutable sampling step. Blank lines
do not interrupt. EOF disables further active reads without cancelling work.
Non-terminal piped input retains sequential semantics (preloaded tasks are not
mistaken for interruptions). No new thread, queue framework, Port or Event.

Cancellation must settle through existing durability/unknown-effect handling
before a queued line is consumed. Fatal/unknown execution errors remain fatal.
If explicit recovery is required, the queued line is explicitly rejected, not
saved or automatically resent after `/resume`. No hidden retry, automatic
reasoning discard, rollback or background-process termination is implied.

Base generation4 clarifies that the latest instruction can pause, replace or
narrow an earlier task. Connection/diagnosis-only requests stop at sufficient
evidence; history/summary/phase does not authorize continuing an old goal.
Ambiguous intent is clarified. This is guidance, not semantic enforcement or
a guarantee of model obedience. Original generation3 bytes remain a fixture.

`tool_search` consumes only the immutable plan's deferred catalog; it bypasses
live process resource conflicts just as existing lifecycle controls do. All
actual environment reads/writes/commands retain their previous conflict rule.
No broad read-only exemption, global-lock removal, persistent activation or
special model dispatch is added. Discovery activates stop/poll normally for a
later Step, with exact RequestAudit/dispatch binding and Hidden preserved.
Job/PTY descriptions disclose resource leases, deadlines and exact stop tools.
Long-lived service ownership/process-tree supervision remains a separate scope;
this repair does not claim to kill all descendants or reconnect expired handles.

Required regressions: TTY cancellation/correction and no extra old-task requests;
non-TTY ordering; initial Vision cancellation; explicit recovery rejection;
global Job/PTY lease permits discovery and subsequent lifecycle control while
conflicting execution stays blocked; old prompt reconstruction and current
request guidance. Mock/loopback tests prove transport/control, not model quality.
