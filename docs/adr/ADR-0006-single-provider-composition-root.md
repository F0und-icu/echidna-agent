# ADR-0006: Single Configuration-Driven Provider Composition Root

- Status: ACCEPTED
- Date: 2026-08-28
- Deciders: project owner
- Architecture version: 1.1.0 implementation decision
- Related Slice: F1a, future Provider binding slices, M4-S4
- Supersedes: direct Provider construction in generic Live Smoke candidates
- Superseded by: ADR-0007 for Provider-specific binding and protocol-reuse policy; the single Config and Composition Root decision remains active

## Context

Cyber Agent already has a Provider-neutral `ModelProvider` Port, explicit capability mappings and one OpenAI Responses Adapter. The CLI `chat` shell, CLI `doctor` and two ignored Live Smoke tests were created in separate Slices. The Live Smokes directly selected the OpenAI endpoint, model environment variable and Adapter constructor, while `doctor` did not load a real Provider profile.

That split makes a second Provider look like a second runtime path. Copying the same setup for DeepSeek or another Responses implementation would duplicate credentials, capabilities, request construction and test behavior. A dynamic Provider registry or model router would solve a larger problem than this single-user Agent needs.

## Constraints

- Keep one Rust package, one binary, one AgentLoop and at most one optional scoped Vision Provider.
- Preserve `ui/main -> app -> ports -> domain`; Adapter wire types do not enter App or Domain.
- Capabilities are explicit, versioned configuration and are never inferred from Provider or model names.
- Credentials remain dedicated Cyber Agent references and are resolved only when an explicitly selected live Provider stream starts.
- `doctor`, Mock, Fixture and Replay paths must remain credential-free and offline.
- Do not add a registry, plugin ABI, routing server, automatic fallback or Provider SDK.
- Do not add a Cargo dependency when existing Serde, TOML and HTTP/SSE dependencies are sufficient.

## Decision

Add one strict Provider Config v1 and one top-level Provider Composition Root.

The configuration contains:

```text
ProviderConfigV1
  schema_version
  active_agent_profile
  providers[name]
    binding
    wire_api
    base_url
    model
    credential_ref
    capability_mapping
    generation
  capability_mappings[name]
    explicit ModelCapabilities fields and mapping version
  agent_profiles[name]
    primary
    vision?
    vision_mode
    generation
```

The schema rejects unknown fields, unsupported schema versions, empty identifiers, dangling references, invalid URLs, non-dedicated credential references and invalid Primary/Vision capability combinations. Provider and Wire API are fixed enums containing only implemented paths. They are not extension registries.

`chat`, `doctor` and every generic Live Smoke load the same configuration and call the same Composition Root. The Composition Root creates the concrete Adapter behind `dyn ModelProvider` and compiles the provider-neutral `AgentModelProfile`. `doctor` consumes only the compiled profile and never resolves a credential. An Adapter receives the configured credential reference but calls the injected `CredentialSource` lazily from `ModelProvider::stream`.

Provider-specific offline wire/contract tests may construct their Adapter directly because the Adapter is their subject. CLI, App services and Live Smokes may not.

## Protocol Reuse Rule

Wire protocol is a separate explicit configuration field; it is not inferred from vendor branding or model text.

The first binding may own a minimal protocol implementation. When a second real Provider uses the same Wire API, its Slice must first extract the common request encoding, HTTP/SSE transport and response decoding into one shared wire owner. The new Provider then adds only a thin binding for proven differences such as endpoint conventions, credential reference, capability mapping or extra events. It must reuse the same `ModelProvider` Port Contract Suite.

A different Wire API follows the same rule: one reusable protocol implementation plus thin Provider bindings. Generic CLI/Live code never contains a Provider-specific branch beyond invoking the Composition Root.

## Consequences

### Positive

- Endpoint, model and capability selection become auditable configuration rather than test code.
- Chat, diagnostics and Live tests cannot drift into separate runtime constructions.
- A future Provider can reuse an existing wire protocol without copying the AgentLoop or App services.
- Credential lookup remains explicit, lazy and independent from Codex login state.

### Negative

- Config v1 becomes a maintained input contract and requires strict compatibility tests.
- Each truly distinct Provider binding still needs fixtures for its proven wire differences.
- The current OpenAI Responses implementation is not renamed or generalized until a second real binding proves the common extraction boundary.

### Deferred

- DeepSeek Responses binding and its `response.reasoning_text.*` event differences.
- First-version interactive Chat composition.
- Automatic configuration discovery in an operating-system config directory.
- Dynamic Provider registration, cost/latency routing and fallback.

## Validation

- `doctor` and shell-only `chat` accept or reject the same config without credential access.
- Both ignored Live Smokes obtain the Primary `ModelProvider` through the same Composition Root.
- Generic runtime and Live paths contain no Provider official URL or vendor-specific model environment variable.
- Fake credentials prove that only the exact configured reference is requested and only on `stream`.
- Invalid schema, unknown fields, invalid references, capability mismatches and unsafe credential references fail before network activity.
- Existing OpenAI wire/contract fixtures remain direct Adapter tests and pass unchanged semantically.

## Revisit Trigger

Revisit only when a second real Provider binding, a second Wire API, or operating-system config discovery has an immediate runtime consumer and its exact contract can be tested.
