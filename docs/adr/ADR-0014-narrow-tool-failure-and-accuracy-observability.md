# ADR-0014: Narrow Tool Failure and Durable Accuracy Observability Boundary

- Status: ACCEPTED
- Date: 2026-08-29
- Deciders: project owner
- Architecture version: 1.4.0
- Related Slice: M4-S3b
- Supersedes: the `ToolFailure::post_execution` stage and any M4-S3b interpretation that treats post-execution persistence failure as a ToolFailure
- Superseded by: none

## Context

The v1.2 ToolFailure envelope included a `post_execution` stage. The current Tool and AgentLoop implementation has no independent, production-consumed semantic owner for that stage. A receipt, History, RequestAudit or related durable submission failure is an application/runtime durability failure, not a failure of the already executed Tool. Keeping it in the provider-neutral ToolFailure enum would blur the boundary between execution and persistence and could turn a real successful result into a fabricated failure or receipt.

The M4-S3b accuracy report also needs a strict observability boundary. Existing durable Event, RequestAudit, ToolCall/Result, ExecutionReceipt and Artifact facts can support some derived metrics. A transient Replay termination or an inferred promotion attempt is not a durable typed signal. Reports must not manufacture a zero for an unobservable metric. Deferred Tool activation likewise needs evidence in the adjacent request audits rather than a second activation fact or an in-memory projection.

This is a narrow additive correction. Earlier architecture files remain frozen and readable; this ADR and Architecture v1.4 supersede only the conflicting failure-stage and observability details.

## Decision

### 1. ToolFailure has two execution-bound stages

The provider-neutral envelope is:

```text
ToolFailure {
    code
    stage: pre_execution | execution
    retryability: correct_arguments | explicit_rerun_only | terminal
    message
    field_errors[]?
    execution_id?
    receipt_ref?
}
```

- `pre_execution` means validation, exposure, plan or argument failure before a ToolExecutionPlan starts. It is returned in a ToolResult paired with the original ToolCall, has no Receipt, and may invite one new corrected ToolCall from the model.
- `execution` means the ToolExecutionPlan started. The attempt, including a failed or cancelled attempt, must have a real ExecutionReceipt. The ToolFailure may reference that Receipt; it is never reclassified as a pre-execution error and is never silently retried.
- There is no `post_execution` ToolFailure stage in v1.4.

Receipt, History, RequestAudit, Artifact or other execution-after persistence submission failures belong to the App/AgentLoop error boundary. They stop or surface the durability failure according to the existing application path. They must not rewrite a real successful ToolResult as a ToolFailure, fabricate an ExecutionReceipt, or silently execute the Tool again. A started attempt is not exposed as durably complete unless its real Receipt is available.

### 2. Accuracy reports use only durable, typed evidence

`ToolAccuracyReport` is a derived non-evidence report over persistently recorded Trace facts. It may report a metric only when the Trace has the typed inputs and complete denominator needed to observe it honestly. Missing or inapplicable observation is represented as Deferred/Unavailable with a reason, never as `0`.

Until a typed durable signal exists, these remain Deferred rather than inferred:

- unmatched Replay termination;
- Observation, Replay or Vision promotion attempts.

The report never creates Research, Memory, Finding or Evidence state, and a controlled case never creates a production public projection entry point. Tests may use controlled fixtures and existing persisted behavior, but must not add a `pub` method, endpoint or other production projection solely to feed a test.

### 3. Deferred activation requires adjacent audit proof

A successful Deferred activation is observable only when adjacent RequestAudit entries for the same Turn show the exact ToolSpec transition: the prior Step records the Deferred Tool as not advertised, and the immediately following Step records that Tool as advertised in its immutable ResolvedToolPlan. The transition must retain the sent ToolSpecs and plan identities already owned by RequestAudit.

No separate durable activation fact is introduced. If the adjacent transition is absent, incomplete or not typed, the accuracy report marks activation unobservable/Deferred and never assumes a zero or a success.

### 4. Frozen-file and publication boundary

Architecture v1.0 through v1.3 are not edited. This ADR is implemented only through additive Architecture v1.4. `SEALED_MANIFEST.md` is unchanged; local freeze does not authorize Git, remote publication or a seal.

## Consequences

### Positive

- Tool execution errors and application durability errors have separate owners.
- A real successful result cannot be falsified because a later persistence operation failed.
- Accuracy claims are limited to what durable Trace data can reproduce.
- Deferred activation has an auditable request-to-request proof without a second state authority.

### Negative

- Some M4-S3b metrics remain Deferred until their typed durable Trace signal is designed and consumed.
- Application persistence failures require the existing App/AgentLoop error path to stop or surface the failure without a ToolFailure conversion.

## Validation

- A pre-execution rejection has a paired ToolResult and no Receipt.
- Every started execution has a real Receipt, including execution failure or cancellation.
- Receipt/History/RequestAudit submission failure is an App/AgentLoop error and never fabricates a Receipt or rewrites a successful ToolResult.
- No `post_execution` ToolFailure is produced or required by the v1.4 contract.
- Reports never score unmatched Replay termination or Observation/Replay/Vision promotion as zero without a typed durable signal.
- Deferred activation is counted only from adjacent RequestAudit ToolSpec transition evidence.
- Controlled tests do not require a production `pub` projection entry point.
