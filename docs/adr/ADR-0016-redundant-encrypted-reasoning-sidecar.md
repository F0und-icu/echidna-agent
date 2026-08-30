# ADR-0016: Redundant Encrypted Reasoning Sidecar Stays Adapter-Local

- Status: ACCEPTED
- Date: 2026-08-30
- Deciders: project owner
- Architecture version: 1.6.0
- Related Slice: F1e
- Supersedes: ADR-0015 only where it rejected every non-empty `encrypted_content`
- Superseded by: none

## Context

The first configuration-driven DeepSeek Full-only Live Tool Loop reached a
completed Responses reasoning output item containing both:

- a complete `content[]` entry of type `reasoning_text`; and
- a non-empty opaque `encrypted_content` sidecar.

Earlier real protocol probes had already proved that replaying the plaintext
`reasoning_text`, its `function_call` and the paired `function_call_output` in
wire order succeeds. The opaque sidecar was therefore redundant for the
implemented generic replay contract. Rejecting it before reading the supported
plaintext made the Adapter stricter than the reusable protocol evidence.

## Decision

The one Responses decoder first validates every `content[]` entry. When at
least one non-empty supported `reasoning_text` is present, it emits the existing
provider-neutral `ModelEvent::ReasoningItem` and leaves any accompanying opaque
encrypted sidecar inside the Adapter.

The Adapter still fails explicitly when:

- non-empty encrypted content is the only available continuation;
- a reasoning content type is unknown;
- a reasoning content entry omits its string type or text; or
- the plaintext reasoning cannot be represented by the existing semantic
  `ContentItem::Reasoning`.

No encrypted bytes, Provider item IDs or vendor fields enter Domain, History,
Session, Research, Memory, RequestAudit semantics or user output. No Provider,
model, URL or credential branch is added.

## Compatibility

- `ModelInputIR` remains v3; no new Core variant or persisted fact schema is
  required.
- The request encoder continues to replay only the portable plaintext
  reasoning item.
- Encrypted-only services remain explicitly unsupported until a second real,
  reusable wire requirement proves a safe semantic contract.
- ADR-0015 otherwise remains unchanged.

## Validation

- The permanent Responses fixture contains plaintext plus a non-empty opaque
  sidecar and decodes through the existing reasoning continuation path.
- Encrypted-only and unknown-content fixtures remain protocol failures.
- The selected Full-only Live Tool Loop must complete through the unchanged
  Config v1 Composition Root and one AgentLoop.
