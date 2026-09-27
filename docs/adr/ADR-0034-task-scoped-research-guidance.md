# ADR-0034 — Task-scoped research guidance

- Status: ACCEPTED DESIGN for operator-authorized R7-C1a; verification in STATUS.
- Date: 2026-09-25
- Additive architecture: v1.24; no remote seal.

A new Session in an existing Project received only a greeting but continued old
research. This is observed model behavior, not proof of a runtime task dispatcher
defect. Existing Phase prompts unconditionally say "Current objective" while
Base v4 says current operator scope prevails. Remove that semantic conflict.

Base v5 states once that phases/history do not create tasks, greetings do not
start research, and "continue" needs a clear unfinished task. Keep current-scope,
checks-only stopping, explicit Resume and evidence guidance. Five fixed Phase
resources describe methods conditional on the requested task; Mapping advances
2→3 and the other phases 1→2. No keyword interpreter, extra model request, idle
phase, task ledger, tool filter or workflow engine is introduced.

ResearchPhaseSelection retains exact recorded generations. Accept only the
explicit known old values (Mapping 1/2; other phases 1) plus current generation,
preserving source/reason checks and rejecting zero/unknown/future values.
Current live/resumed requests use current resources via PromptComposition;
mechanical historical audit reconstruction still uses recorded bytes. Resources
superseded in this Slice move unchanged into test fixtures, with no runtime consumer.
Domain generations, selection event schemas and Turn-boundary behavior do not change.

Existing ContextPlanner, hashes, RequestAudit and Baseline invalidation observe
the revised bytes. Never preserve continuation by silently replacing old sent
instructions, dropping completed reasoning or relaxing a budget pressure test.
Older binaries may reject newly recorded Phase generations; downgrade must not
resume such Sessions. No migration/rewrite of historical streams is authorized.

Offline tests cover all accepted/rejected generation values, unchanged exposure,
current fragment identities/fingerprints and historical audit reconstruction;
formal CLI fixtures bind old-project background, fresh Session input and resumed
scope to actual requests. Scripted answers establish delivery, not model obedience.
Model quality requires separately authorized bounded ordinary-engineering Live.
No new public API, Port, dependency, Event, configuration or Provider path.
