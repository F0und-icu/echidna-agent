# ADR-0022: Runtime-local Job and PTY Handle Namespaces

- Status: ACCEPTED for operator-authorized CTF-R3a1
- Architecture: additive1.12.0; output retention is a separate R3a2 decision

Existing Job and PTY tables each restart their counters at1. A historical
`job:1`/`pty:1` can therefore operate on an unrelated new process. Handles
must bind to their issuing table instance rather than reuse a bare sequence.

One private Adapter owner generates an opaque v2 namespace from two fresh
standard-library randomized hash states, the OS process identity and a
process-wide monotonically allocated instance number. Each table retains a
checked monotonically increasing local handle sequence. No clock, PID alone,
model label or guessed Session identity is sufficient. Exhaustion must fail
instead of wrapping. Randomized namespace collisions across separate process
lifetimes are probabilistic, not a claim of mathematically global uniqueness;
same-process instances are separated by the checked instance counter.

Job/PTY prefixes remain distinct. No authentication/security property is
claimed for opaque handles. Existing exact table lookup remains the only
control authority; no parsed sequence fallback, table scan or implicit
reattachment is allowed. Unknown diagnostics explicitly state that historical
or other-runtime handles may be expired, without asserting every missing ID
once existed. Legacy strings remain valid historical data and return unknown
when operated against a new table; stored Events and Tool schemas do not change.

No daemon, process resurrection, persistent registry, new crate, public API,
Event or Config field. One private identity owner has two present consumers.
Resource leases and terminal cleanup remain unchanged. Tests must prove old
handles cannot poll/input/stop/resize new processes, and current handles still
work, including independently launched registries in distinct processes.
