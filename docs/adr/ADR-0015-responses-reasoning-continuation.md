# ADR-0015: Responses Reasoning Continuation Without Vendor Branches

- Status: ACCEPTED
- Date: 2026-08-30
- Deciders: project owner
- Architecture version: 1.5.0
- Related Slice: F1e
- Supersedes: ADR-0007's incomplete assumption that reasoning deltas can be discarded after display
- Superseded by: none

## Context

The shared Responses Adapter already decodes `response.reasoning_text.delta`
as provider-neutral `ModelEvent::ReasoningDelta` and keeps it out of ordinary
Conversation History. A separately authorized live Tool loop proved that this
is not sufficient for every conforming stateless Responses service.

The observed sequence was:

1. the first request completed with one `reasoning` output item followed by a
   `function_call`;
2. the local Tool executed successfully;
3. the next request returned HTTP 400 because the prior `reasoning_text` was
   absent;
4. replaying only the reasoning item and Tool output still failed because the
   corresponding Function Call was absent;
5. replaying `reasoning -> function_call -> function_call_output` in original
   order succeeded both with and without `previous_response_id`.

The credential source, model name, URL and Tool Schema were not the cause.
This is a reusable Responses output-item continuation requirement, not a
vendor identity.

## Decision

### Completed reasoning is provider input state

Add one provider-neutral semantic input item:

```text
ContentItem::Reasoning { text }
```

It represents completed model reasoning text that a Provider requires the
client to replay. It is not an Assistant message, operator input, Research
fact, Memory fact, Finding evidence or Provider item identity.

`ModelEvent::ReasoningDelta` remains transient streaming output. A new
completed reasoning event is produced only from a successfully decoded
`response.output_item.done` reasoning item. The AgentLoop admits that completed
item only after the whole response completes; cancellation, malformed streams
and failed responses do not advance continuation state.

### History and delivery remain separate

The completed reasoning item is retained in the AgentLoop's provider-visible
delivery input in exact output order next to the Function Call. It is not
committed into Conversation History or Session History events.

For a later Tool Step, the existing committed-delivery path appends the paired
Tool Result while retaining prior completed provider output items. A Full
request therefore contains:

```text
prior semantic input
-> completed reasoning item
-> function_call
-> function_call_output
-> later semantic input
```

ContextPlanner remains the owner of logical Prompt/History composition. The
RequestAudit continues to distinguish full logical input from actually
delivered input; delivered reasoning is local auditable request data, not a
new fact authority.

### Capability meaning remains explicit

`incremental_input = true` means that the configured service can continue from
`previous_response_id` plus the strict semantic Delta without replaying prior
server output items. A service that requires client-side output-item replay
must declare `incremental_input = false`; the AgentLoop sends a complete
request containing the retained output items.

No capability is inferred from Provider name, model slug, URL, credential or
the presence of reasoning text. No vendor Adapter, automatic probing or
fallback router is added.

### Wire ownership

`ResponsesHttpAdapter` remains the only Responses wire owner. It:

- decodes completed `reasoning` output items containing `reasoning_text`;
- explicitly rejects unsupported non-empty encrypted or unknown reasoning
  continuation content rather than silently dropping it;
- encodes `ContentItem::Reasoning` as one Responses `reasoning` input item;
- retains existing Tool Call/Result and `previous_response_id` encoding.

Provider item IDs, status fields and wire DTOs do not enter Domain.

## Compatibility and evolution

- `ModelInputIR` advances additively from schema v2 to v3 for the new semantic
  ContentItem; RequestAudit continues to validate historical v2 identities.
- No Session, Research, Artifact, configuration or credential schema changes.
- Resume, Compaction or Primary ProviderSession replacement clears ephemeral
  committed delivery state as before.
- Primary and scoped Vision requests never share reasoning continuation.
- No Cargo dependency, Feature, service, Registry or Hook is introduced.

## Validation

- A protocol fixture decodes completed reasoning and re-encodes it in the
  exact reasoning/FunctionCall/ToolResult order.
- A Full-only loopback AgentLoop completes two model Steps and one real local
  Tool execution.
- Reasoning deltas and completed reasoning never enter Conversation History or
  user-visible assistant text.
- Partial, failed and cancelled streams never commit reasoning continuation.
- Incremental Provider behavior and rejection fallback remain unchanged.
- Historical RequestAudit ModelInputIR v2 remains readable.
- A separately authorized live Tool loop completes with an explicit Full-only
  capability mapping.

## Rejected alternatives

- Do not paste reasoning into Assistant text.
- Do not persist reasoning as Session History, Research or Memory.
- Do not add a DeepSeek binding or model-name conditional.
- Do not mark a client-replay service incremental merely because it returns a
  response ID.
- Do not preserve partial reasoning deltas after cancellation or failure.
