# ADR-0008: Local Inline Provider Credentials

- Status: ACCEPTED
- Date: 2026-08-28
- Deciders: project owner
- Architecture version: 1.1.0 implementation decision
- Related Slice: F1d
- Supersedes: ADR-0006 credential-reference-only constraint
- Superseded by: none

## Context

Cyber Agent is a single-user local tool. Config v1 currently requires every Provider credential to be named indirectly through `credential_ref`, even when the owner wants one self-contained personal configuration file. That indirection adds setup friction but does not improve the Agent runtime, Provider protocol reuse or vulnerability-research behavior.

The user explicitly accepts storing a real API Key in their local configuration. The runtime must support that choice without adding another Provider construction path or allowing the value to enter durable research facts and diagnostics.

## Decision

Config v1 supports two mutually exclusive credential forms on each Provider:

```toml
api_key = "local-personal-key"
# or
credential_ref = "CYBER_AGENT_PRIMARY_API_KEY"
```

Exactly one form is required. Existing `credential_ref` files remain valid, so this is a compatible additive Config v1 extension rather than a new schema version.

The Provider Composition Root converts either form into the existing Adapter credential boundary:

- `credential_ref` continues to use the injected external `CredentialSource` lazily;
- `api_key` receives a Provider-scoped opaque internal reference and a private in-memory source owned by composition;
- the Adapter, request encoder, transport, response decoder, AgentLoop, Chat and Live Smoke remain unaware of which form was selected.

No file-permission enforcement, Keychain, encryption, secret manager or automatic config discovery is added. A personal inline-key configuration is user-owned local state and must not be committed as a repository fixture.

## Non-Disclosure Boundary

Accepting plaintext local storage does not make the Key model or audit data:

- configuration errors and `doctor` output do not include the value;
- Adapter errors identify only an opaque inline credential reference;
- Authorization headers and API Keys do not enter Session Events, RequestAudit, Artifact Blobs, Snapshots, metrics or logs;
- tests use synthetic fixture values and never mutate the developer process environment.

This boundary reuses the frozen Architecture v1/v1.1 rule that Authorization Headers and API Keys never enter durable request or vision records.

## Consequences

### Positive

- One personal TOML file can be sufficient to start Chat.
- Environment-variable references remain available for CI and users who prefer them.
- Provider and Wire API reuse remain configuration-driven and vendor-neutral.

### Negative

- Anyone who can read the personal config can read its inline Key.
- `doctor` must parse the inline value into process memory even though it never sends a request.
- Users must avoid committing or broadly backing up a real inline-key config.

## Validation

- Inline and referenced Providers use the same Composition Root and Adapter type.
- Both or neither credential fields fail before Provider construction.
- A loopback Chat proves the inline value reaches only the Authorization header.
- CLI output, configuration errors and durable Project facts do not contain the fixture value.
- Existing referenced-credential and credential-isolation tests remain green.
