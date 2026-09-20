# ADR-0028: Additive Tool Activation with Retained State Diffs

- Status: ACCEPTED for the operator-authorized R6-01 repair
- Date: 2026-09-18
- Architecture: additive v1.18; implementation/Gate status remains in STATUS.

## Evidence

R6 ordinary-engineering Live preserves completed reasoning and a Project
full-plus-diff history. Successful tool_search adds read_output to the next
SamplingStepPlan, but ToolsChanged currently permits only exact logical-input
reasoning replay. The verified diff representation is consequently refused
before another Provider request. No malformed wire request was sent.

## Decision

Keep the same Context state-diff owner and exact old completed reasoning,
Call/Result order and provenance. Admit the additional ToolsChanged diff case
only when all previously sent ToolSpecs remain byte-for-byte present, new
specs are added, no explicit baseline reset is pending, and the existing
Baseline comparison proves every other semantic field and the committed
input/server-output prefix unchanged. For that proof alone, pass the prior Tool
hash by reference to the same comparator; do not clone long History or change the sent snapshot,
audit reason, ToolSpecs or global invalidation priority. This is an extra pure
check in the existing ProviderSession, not a second comparator or retry.

Only current-Turn activation can satisfy the active runtime path; Turn/Resume
activation clearing, queued operator changes and identity replacement retain
their existing behavior. Removal/redefinition is not admitted by the new path.
The earlier exact logical-plus-reasoning path stays valid independently.

The existing Domain delivery-reason owner shares the Full input relationship
predicate between AgentLoop and RequestAudit. The additional ToolsChanged
relationship requires retained reasoning and exact current schema,
instructions, ToolSpecs and request requirements; only content can use the
already-verified state-diff representation. The live AgentLoop additionally
proves the prior-prefix/additive contract above. RequestAudit validates the
current logical/delivered contract and bound immutable plan/map, not an invented
second history. It does not reconstruct a prior ProviderSession from one audit.

Prompt/Model/Profile/Capability/renderer/output/request-property changes,
Core Memory replacement, missing provenance, history replacement and budget
overflow never gain permission from simultaneous Tool activation. No automatic
reasoning discard, Resume, compaction or execution retry is added.

## Compatibility and validation

No new reason enum, wire field, persisted schema, public API, dependency or
Provider binding. Old facts remain readable; newly accepted input pairs use
existing ToolsChanged and complete audit components. Older binaries can reject
their reconstruction under their narrower relationship; do not promise backward
execution compatibility or rewrite old facts. Architecture v1 through v1.17
stay frozen; the remote seal manifest is untouched.

Add a failing formal CLI fixture combining Full-only completed reasoning,
Project diff, Deferred discovery, actual raw-output readback and exact audit
reconstruction. Prove no duplicate execution, no activation across Turn/Resume,
and negative Prompt/Memory/budget/identity/contract cases. Preserve existing
invalidation priority and use existing public production behavior for tests.
Focused/affected tests, finite independent review, full Gate and Lean precede
any completion claim. New Live/remote delivery is separate.
