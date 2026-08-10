# ADR-0003: Minimal OpenAI HTTP and SSE Stack

- Status: ACCEPTED
- Date: 2026-08-10
- Deciders: project owner
- Architecture version: 1.1.0 implementation decision
- Related Slice: M1a-S2
- Supersedes: none
- Superseded by: none

## Context

M1a-S2 needs one OpenAI Responses full-request HTTP Streaming Adapter behind the existing provider-neutral `ModelProvider` Port. The Adapter must support cancellable asynchronous response streaming, SSE framing, stable request encoding, explicit errors and local-loopback tests without importing Codex product crates or a Provider SDK.

The current Port returns a `ModelEventStream`; a blocking HTTP client would consume a thread and complicate cancellation. Hand-writing TLS, HTTP parsing or complete SSE framing would add security and protocol maintenance unrelated to vulnerability research.

## Constraints

- Keep one Rust package and one default binary.
- Provider wire types stay inside `adapters/openai`.
- Do not depend on `codex-api`, `codex-client`, `codex-protocol`, `codex-core` or a Provider SDK.
- Use one HTTP stack and one SSE framing implementation.
- Dropping a pending Model stream must cancel its request without a second cancellation authority.
- Ordinary tests use local loopback or Fixtures and never require a credential or public network.
- Live tests remain ignored and separately authorized.
- M1a-S2 sends only full requests; `previous_response_id` remains M1b-S2 work.

## Options Considered

### Option A: Import Codex API or an OpenAI SDK

Benefits:

- Existing request and streaming behavior.

Costs:

- Imports product authentication, session, telemetry or broad protocol types.
- Expands the dependency and synchronization boundary.
- Conflicts with the provider-neutral Core and forbidden-dependency rules.

### Option B: Use `reqwest` plus `eventsource-stream`

Benefits:

- Reuses maintained TLS/HTTP and SSE framing.
- Keeps request DTOs, event decoding and error mapping local and auditable.
- A lazily polled async stream is cancelled by normal Rust drop semantics.
- Local-loopback tests can exercise the complete Adapter without a Provider SDK.

Costs:

- Adds an asynchronous HTTP dependency closure.
- The eventual CLI composition root must run an async runtime.

### Option C: Implement blocking HTTP, TLS and SSE locally

Benefits:

- Maximum control over source code.

Costs:

- Reimplements complex transport behavior.
- Blocking cancellation requires additional threads and coordination.
- Higher security, compatibility and maintenance cost than the Agent feature warrants.

## Decision

Choose Option B.

- Use `reqwest 0.12` with default features disabled and only `rustls-tls`, `json` and `stream` enabled.
- Use `eventsource-stream 0.2` only for SSE frame reconstruction across byte chunks.
- Use `futures-util 0.3` for stream polling and `async-stream 0.3` for the small lazy Adapter stream.
- Use `tokio 1` only as an explicit dev dependency in M1a-S2 local-loopback tests. Move it to normal dependencies only when a later authorized Composition Root directly owns the runtime.
- Implement OpenAI request/response DTOs, capability validation, event decoding and provider-neutral error mapping locally under `adapters/openai`.
- Encode `store: false` explicitly instead of depending on a Provider retention default.
- Treat the inspected Codex Responses client and decoder as REFERENCE. No upstream code is copied and no Port Ledger ACTIVE entry is created.
- Terminate immediately after `response.completed`; ending the stream first is a protocol error.
- Application cancellation remains owned by `ModelStreamCancellation`; dropping the Adapter stream cancels the pending HTTP future/response. No second cancellation token is added.

## Consequences

### Positive

- The Core remains independent of OpenAI and Codex types.
- HTTP/TLS/SSE edge cases are delegated to narrowly selected maintained crates.
- The local Adapter remains small enough to audit and replace.
- Offline CI can validate request headers, JSON shape, chunked SSE, errors, Usage and cancellation.

### Negative

- `reqwest` and Rustls increase compile time and Cargo.lock size.
- The Adapter requires a Tokio-compatible executor when actually polled.
- SSE event semantics still require local fixtures when OpenAI adds event kinds.

### Deferred

- HTTP connection tuning and WebSocket transport.
- Retry policy.
- `previous_response_id` and ProviderSession state.
- Image Artifact resolution and preprocessing.
- Tool Schema and structured-output request encoding.

## Compatibility and Migration

- No Event, Research or persisted Config schema changes.
- `ModelProviderError` gains provider-neutral credential, request, transport, HTTP, protocol and provider-failure variants.
- Existing Mock behavior remains source-compatible.
- Removing this stack later requires only another Adapter implementation of `ModelProvider` and equivalent Contract tests.

## Implementation Plan

- Add the minimum dependencies with constrained features.
- Implement `adapters/openai/{credentials,wire,mod}.rs`.
- Add local-loopback request/stream tests and stable JSON/SSE fixtures.
- Add one ignored, environment-gated synthetic live smoke test without running it.
- Preserve and externalize all existing inline regression tests.
- Run the full M1a-S2 and Lean Gates.

## Validation

- A local HTTP server observes the dedicated Authorization header, SSE Accept header and stable full request JSON.
- Text delta, complete Function Call, Usage and Completed decode in order.
- Missing completion, malformed/unknown events, Provider failure and HTTP rejection remain explicit typed errors.
- Cancelling a pending request completes the App service without waiting for the server.
- Fake credential tests prove `OPENAI_API_KEY` alone is never read or used.
- The project builds and tests without the local `codex/` directory.

## Revisit Trigger

Revisit only if measured build/runtime cost is excessive, `reqwest` no longer supports the pinned MSRV, or a smaller maintained transport provides the same cancellation, TLS and streaming behavior without importing a Provider product layer.
