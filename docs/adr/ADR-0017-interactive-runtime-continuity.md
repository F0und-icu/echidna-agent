# ADR-0017: Interactive Runtime Continuity

- Status: ACCEPTED for the operator-authorized F3 repair
- Architecture version: 1.7.0
- Supersedes: rejection solely due to an intact Research append-only suffix; the deferred runtime Personal/Project Prompt file loading prohibition, narrowly for two explicitly configured files (no discovery or registry); scheduler-generated cancelled Receipts for dropped, unobserved active executions

## Problem

Production Chat persists History only at Turn end, repeatedly replays the whole Session for individual appends, incompletely composes research and memory context, and cannot resume a compacted Session after ordinary Research evolution. Provider streams have no configured deadlines. Existing short successful flows do not establish long-session reliability.

## Decision

Keep one Session/Event authority, AgentLoop, ContextPlanner and Responses transport. Introduce fixed, typed durability/context stages rather than a Hook registry. Persist user/model History before dependent actions and Tool results before the next request. Durability errors remain App errors. Interrupted execution is unknown unless an existing real Receipt proves the result; neither recovery nor transport retries may rerun a Tool.

Cancellation while an executor future remains unobserved cannot create a terminal Receipt or ToolResult. Persist already observed outcomes and known not-started pre-execution results, then stop with an explicit App interruption error. The unmatched durable Call remains unknown on derived Resume normalization. A cancelled outcome actually returned by an executor remains an observed outcome and retains its real Receipt; it does not prove absence of partial side effects.

An old Compaction is not accepted against a changed current stream. First reconstruct its exact source prefix, verifying project, stream, watermark/hash and original references. If the prefix is valid and the current stream only advanced, build and validate a fresh current-target Compaction from current facts before Resume. Missing or changed prefixes, invalid references and incompatible identities fail. Existing exact-Snapshot applicability has one owner and is not weakened. New Compactions remain schema v2 Events; no historical Event is rewritten.

Current-target research context and Core Memory are derived inputs. Explicit Personal/Project Prompt sources use the existing Prompt composition. Context changes are observed before a new SamplingStepPlan; an already captured plan remains immutable. Operator Domain/Phase remains fixed for the Turn. Retrieval/guidance stays Shadow.

Compaction selects bounded references and complete recent History under the existing planner budget; explicit refusal must precede Event append if useful reduction cannot be achieved. Narrative remains lossy and non-evidence. Provider timeout values are transport configuration, never model capabilities, wire semantics or a retry policy.

## Validation and compatibility

Cover interruption after model ToolCalls and real execution, ordinary Research appends after Compaction, tampered prefix rejection, exact-target filtering, current-context freshness, prompt/memory source updates, large Compaction candidates and stalled HTTP/SSE. Preserve legacy Event replay, Full-only reasoning continuity, Primary/Vision isolation and all previous offline regressions. No dependency or benchmark runtime is introduced.
