# ADR-0010: Immutable Sampling Step and Resolved Tool Plan

- Status: ACCEPTED
- Date: 2026-08-29
- Deciders: project owner
- Architecture version: 1.3.0
- Related Slices: M4-S3a1, M4-S3a2, M4-S3b
- Supersedes: mutating shared Tool exposure as the request-level authority
- Superseded by: none

## Context

Cyber Agent already has one AgentLoop, a pure ContextPlanner, a fixed Tool Registry and Turn-boundary Prompt/Profile changes. The current Tool Registry nevertheless retains mutable exposure and deferred-discovery state behind a Mutex, while AgentLoop separately retains the active ToolExposurePlan. That is sufficient for the existing fixed flow, but it leaves two representations of the active plan and makes it possible for request preparation, advertised schemas and later Tool dispatch to observe different state.

The latest inspected Codex source independently demonstrates the value of capturing one immutable request-scoped state object and storing the final model-visible Tool specs together with the matching runtime router. It also retains a known TODO where part of Tool planning still uses the admitted Turn model rather than the captured Step model. Cyber Agent should adopt the invariant without copying the product-sized implementation or its unfinished boundary.

## Constraints

- There remains one AgentLoop and one Tool execution path.
- DomainProfile, ResearchPhase and AgentModelProfile changes remain Turn-boundary operations.
- A Turn can contain multiple model sampling Steps after Tool results.
- `tool_search` may activate Deferred Tools for later Steps of the same Turn.
- Hidden Tools cannot be activated, advertised or called by the model.
- ToolSpec and Runtime must not become independent registries.
- Tool exposure remains context optimization, not approval or permission.
- Replay must use its fixed Replay Tool runtimes and never fall back to live Tools.

## Options Considered

### Option A: Keep mutable Registry exposure

Benefits:

- Minimal immediate code change.

Costs:

- Active exposure exists in AgentLoop and Registry.
- In-flight request semantics depend on later shared mutation.
- RequestAudit can prove sent schemas but not that dispatch used the same resolved plan.

### Option B: Copy Codex ToolRouter

Benefits:

- Mature handling of many product surfaces.

Costs:

- Brings Code Mode, MCP, namespaces, collaboration and product-specific types.
- Violates the forbidden-dependency and minimal-runtime boundaries.
- Copies an upstream design that still has a Step-model TODO.

### Option C: Reimplement one immutable SamplingStepPlan and ResolvedToolPlan

Benefits:

- One request view binds Context, schemas, capabilities and dispatch.
- Keeps Cyber Agent's three exposure states and fixed catalog.
- Allows precise RequestAudit, Replay and Baseline invalidation.

Costs:

- Requires a bounded internal refactor before ResearchPhase behavior is added.
- Deferred activation lifetime must be explicit.

## Decision

Choose Option C.

### Turn Configuration and Sampling Step

A Turn first captures the queued operator-controlled selections:

```text
TurnConfigurationSnapshot {
    domain_profile_id + generation
    research_phase_id + generation
    agent_model_profile_id + generation
    primary/vision ProviderSession identities
    target/project identities needed by this Turn
}
```

Each model request within that Turn then receives one immutable:

```text
SamplingStepPlan {
    step_id
    turn_configuration
    context_plan
    resolved_tool_plan
    model + capability_mapping identity
    request requirements
}
```

The names describe one internal plan, not a new runtime layer. ContextPlanner remains the only Context ordering, budgeting, rendering and fingerprint owner. Tool resolution remains the only exposure owner.

The plan is captured once before preparing the request. The same plan is used to:

1. choose the exact model-visible ToolSpecs;
2. compile ModelInputIR;
3. build RequestAudit;
4. validate Tool Calls produced by that sampling Step;
5. bind ToolExecutionPlan and ExecutionReceipt to the advertised plan identity.

Later configuration changes cannot mutate an already captured plan.

### Static Catalog and ResolvedToolPlan

The Tool Registry becomes an immutable process-local catalog of canonical entries:

```text
RegisteredTool {
    ToolName
    ToolVersion
    canonical ToolSpec
    default exposure
    Runtime binding
}
```

One pure resolver produces:

```text
ResolvedToolPlan {
    plan_id / semantic fingerprint
    ordered resolved entries
    ordered model-visible ToolSpecs
    final exposure per Tool
    runtime availability and stable reason codes
    Tool Schema fingerprint
}
```

Resolution order remains:

```text
Registry default
  -> DomainProfile override
  -> ResearchPhase override
  -> current-Turn DeferredActivationSet
  -> runtime availability
```

`ResolvedToolPlan` does not duplicate Tool normalization, schema validation or execution. Its entries refer to the one registered Runtime and exact canonical ToolSpec.

### Deferred Activation Lifetime

`tool_search` activation is scoped to the current Turn:

- a successful matched search updates a typed `DeferredActivationSet`;
- the current SamplingStepPlan remains unchanged;
- the next Step resolves a new plan containing the activated Tool;
- activation remains for later Steps in that Turn;
- activation is cleared at the next Turn boundary and after Resume;
- Hidden or unavailable Tools cannot enter the set;
- RequestAudit records every Step's exact sent schemas, so Replay needs no separate activation fact store.

This scope avoids permanent Tool-schema growth while preserving Tool-result follow-up inside one Turn.

### Dispatch Binding

A model-originated ToolCall is dispatched only through the ResolvedToolPlan that advertised it. A call to a missing, Deferred-not-activated, Hidden or unavailable Tool fails as a paired pre-execution ToolResult. It never falls back to a newer plan or a raw Registry lookup.

Internal typed callers may use explicitly separate non-model entry points, but those paths cannot be used to dispatch a model-originated call.

### Failure Atomicity

- Failure to resolve or audit a plan stops before the Provider request.
- Failure to persist mandatory RequestAudit stops before the Provider request.
- A plan fingerprint mismatch stops before Tool execution.
- No failed request promotes its Context/Tool plan to the successful incremental Baseline.

## Consequences

### Positive

- Tool schemas and Runtime cannot drift within one request.
- Domain/Phase composition has one pure consumer path.
- Replay can prove exact exposure without mutable global state.
- Future Primary/Vision or model-profile changes remain isolated.

### Negative

- ToolExecutor and AgentLoop composition require a focused internal migration.
- Tests need explicit Step-plan fixtures rather than mutating a Registry in place.

### Rejected Extensions

- No general Router/Registry plugin system.
- No Hook pipeline.
- No Tool policy or approval engine.
- No automatic Provider fallback.
- No Code Mode or MCP exposure surface.

## Migration

- M4-S3a1 replaces mutable exposure authority while preserving current model-visible behavior.
- Existing Tool names, schemas, ToolExecutionPlan, Effect and Receipt contracts remain.
- Historical RequestAudit remains readable; a new additive audit schema may bind `resolved_tool_plan_fingerprint`.
- ReplayToolExecutor produces its own immutable fixed plan and cannot refer to the live catalog.

## Validation

- Sent ToolSpecs and dispatchable Tool runtimes have a one-to-one plan binding.
- A queued Domain/Phase/Profile change cannot affect an in-flight Step.
- Deferred activation affects only later Steps of the current Turn.
- Resume clears activation and sends a Full request.
- Hidden and unavailable Tools never become searchable or callable.
- Tool-plan changes use the existing `TOOLS_CHANGED` Baseline invalidation.
- No second ToolSpec, normalization, pairing or error-mapping owner is introduced.
