# Cyber Agent Architecture v1.5 — Provider Reasoning Continuation

- Version: 1.5.0
- Status: FROZEN ADDITIVE BASELINE (LOCAL; REMOTE SEAL UNAUTHORIZED)
- Date: 2026-08-30
- Base architectures: v1.0 through v1.4
- Governing decision: ADR-0015
- Related Slice: F1e
- Implementation status: COMPLETE_LOCAL; REMOTE SEAL UNAUTHORIZED

## 1. Reading and compatibility rule

Read v1.0, v1.1, v1.2, v1.3, v1.4 and this file together. Earlier frozen
files remain immutable. This version narrowly completes the provider output
continuation boundary required by a real Responses Tool loop. All persona,
Context, Tool, Evidence, Vision and FullAccess invariants remain unchanged.

This local freeze does not authorize Git, remote publication or a manifest
seal.

## 2. Two histories with different authority

Conversation History contains durable user/assistant messages and paired Tool
Calls/Results. It remains the Session projection and Context source.

Provider-visible delivery state additionally retains completed model output
items required to continue the same Primary ProviderSession:

```text
ProviderDeliveryState {
    complete semantic input
    completed server output items in exact order
    paired later Tool Results and semantic input
}
```

The first added output item is provider-neutral reasoning text:

```text
ContentItem::Reasoning { text }
```

It never becomes an Assistant message, Research/Memory fact, evidence or
Provider-specific identity. RequestAudit may record it only as part of the
actual local delivered ModelInputIR.

## 3. Atomic model-sample rule

Streaming reasoning deltas are transient. A completed reasoning input item is
accepted only from a completed Responses output item and only after the entire
model response completes successfully.

The one AgentLoop then atomically:

1. commits Assistant text and Tool Calls to Conversation History;
2. retains completed reasoning plus the same Tool Calls in provider delivery
   order;
3. executes and persists paired Tool Results;
4. builds the next Step from a new immutable SamplingStepPlan;
5. sends either a strict Delta or a complete provider-visible input according
   to explicit capability mapping.

Cancellation, stream failure, malformed reasoning and empty responses do not
advance History, continuation identity, Baseline or provider delivery state.

## 4. Full and incremental delivery

For `incremental_input = true`, `previous_response_id + strict delta` must be
sufficient for the configured protocol service. Prior server output remains
in the local Baseline but is not duplicated into the Delta.

For `incremental_input = false`, the complete request retains completed
provider output items in their original order. A Tool continuation therefore
contains:

```text
reasoning -> function_call -> function_call_output
```

A returned response ID alone is not proof of incremental capability. The
mapping is explicit configuration and is never inferred from names or URLs.

## 5. Protocol boundary

Responses DTOs, item IDs, status, summaries and encrypted wire fields remain
inside `adapters/responses`. Domain expresses only supported semantic
reasoning text. Unsupported non-empty reasoning continuation formats fail
explicitly at the Adapter boundary.

No vendor Adapter, Provider router, capability probe, second AgentLoop or
History owner is introduced.

## 6. Lifetime and isolation

- Provider delivery state is process-local and bound to one Primary
  ProviderSession.
- Resume, Compaction reset and Primary semantic replacement clear it.
- It never crosses into a scoped Vision ProviderSession.
- It is not reconstructed from ordinary Conversation History.
- Historical ModelInputIR v2 RequestAudit remains readable; new requests use
  additive ModelInputIR v3.

## 7. Acceptance invariants

- Completed reasoning required for a Tool continuation is never dropped.
- Reasoning is never presented or persisted as ordinary Assistant History.
- Full-only Responses Tool loops preserve reasoning/Call/Result order.
- Incremental behavior remains capability-driven and unchanged for services
  where server continuation plus Delta is sufficient.
- Partial model output never advances continuation state.
- Provider wire types and IDs do not enter Domain.
- One Responses encoder/transport/decoder, one AgentLoop and one History
  normalizer remain.
- No new dependency, config field, vendor branch or remote service exists.
