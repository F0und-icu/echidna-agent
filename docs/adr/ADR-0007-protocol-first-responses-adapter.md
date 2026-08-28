# ADR-0007: Protocol-First Responses HTTP/SSE Adapter

- Status: ACCEPTED
- Date: 2026-08-28
- Deciders: project owner
- Architecture version: 1.1.0 implementation decision
- Related Slice: F1b and later Provider configuration
- Supersedes: the Provider-specific binding and thin-vendor-binding portions of ADR-0006
- Superseded by: none

## Context

F1a established one strict Provider Config and one Composition Root, but its
only concrete binding was named for OpenAI. F1b originally planned separate
thin OpenAI and DeepSeek bindings over one shared Responses wire.

The project owner clarified that vendor identity is not a runtime behavior:
any configured service may be used when its URL implements the supported
Responses HTTP/SSE contract. The model, endpoint, credential reference and
capabilities are already explicit configuration. Repeating vendor bindings
would add names and branches without changing request encoding or transport.

## Decision

Use one fixed protocol binding:

```toml
binding = "http_sse"
wire_api = "responses"
base_url = "<configured service base URL>"
model = "<configured model>"
credential_ref = "CYBER_AGENT_<CONFIGURED>_API_KEY"
capability_mapping = "<explicit mapping>"
```

`ResponsesHttpAdapter` is the only owner of Responses request encoding,
`POST <base_url>/responses`, Authorization and SSE transport, response
decoding and protocol error mapping. The Composition Root constructs this one
Adapter for every configured `http_sse + responses` profile. It never branches
on Provider profile name, URL hostname, model text or credential-reference
text.

The configured credential reference selects only the secret value to read
lazily. It may name any dedicated `CYBER_AGENT_*_API_KEY` variable and does not
identify or infer a Provider or model. No model environment variable exists.

Capabilities remain explicit versioned data. A stateless service is configured
with `incremental_input = false`; the existing AgentLoop then sends Full
History and never emits `previous_response_id`. Image, Tool, structured output
and Usage support follow the same rule. No capability is inferred from a model
slug or vendor brand.

The shared decoder supports the implemented Responses event set plus the
fixture-proven `response.reasoning_text.delta` and
`response.reasoning_text.done` extension. Reasoning deltas become the
provider-neutral `ModelEvent::ReasoningDelta`; they are kept distinct from
assistant output and are not committed into Conversation History. Unknown
events remain typed protocol failures.

## Consequences

### Positive

- OpenAI, DeepSeek or another conforming endpoint uses the same audited code.
- Adding a configured model or changing a credential reference adds no Rust
  binding, URL branch or model environment variable.
- Request, transport, decoder, cancellation and contract regressions have one
  permanent test owner.
- The Core and AgentLoop remain Provider-neutral and unchanged.

### Negative

- Configuration truthfully claiming protocol and capabilities is the user's
  responsibility; the runtime cannot prove a remote service contract before a
  request.
- A service that only resembles Responses but changes request or event
  semantics is unsupported until fixture evidence defines a reusable protocol
  extension.

### Deferred

- Chat Completions and other Wire APIs.
- Automatic protocol discovery, URL probing or capability negotiation.
- Provider Registry, Router, fallback and vendor-specific tuning.
- First-version interactive Chat composition in F1c.

## Compatibility and Migration

F1a was still an uncommitted local fix, so Config v1 replaces
`binding = "openai"` with `binding = "http_sse"` before its first remote seal.
The explicit `wire_api = "responses"` field remains. Configured URL, model,
credential reference, capability mapping and Agent profile fields are
otherwise unchanged.

Historical ADR-0003 still records why the HTTP/SSE dependency stack was chosen;
its Provider-specific module naming is superseded by this decision.

## Validation

- One configuration test composes differently named models, URLs and
  credential references through the same fixed Adapter.
- Fake credentials prove that only the exact configured reference is read and
  only when `stream` starts.
- A Full-only capability mapping rejects an incremental request before
  credential or network access.
- One Responses fixture covers reasoning text, Tool Call, output text, Usage
  and Completed; malformed and unknown events fail explicitly.
- Static scans prove there is one Responses request/transport/decoder owner and
  no Provider-specific runtime URL or model environment variable.

## Revisit Trigger

Revisit only when a real service cannot satisfy the implemented Responses
contract and offline/live evidence proves a reusable wire-level difference.
Do not add a vendor binding solely for a name, URL, model or credential change.
