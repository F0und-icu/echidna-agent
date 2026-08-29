# ADR-0013: Versioned Prompt Renderer Boundary Without a Dialect Registry

- Status: ACCEPTED
- Date: 2026-08-29
- Deciders: project owner
- Architecture version: 1.3.0
- Related Slices: M4-S3a1, M4-S3e2, M4-S4
- Supersedes: an unversioned implicit join operation as the complete Prompt rendering identity
- Superseded by: none

## Context

Cyber Agent separates Base Instructions, Context ContentItems and ToolSpecs in provider-neutral ModelInputIR. Base, Personal, Project, Domain and Phase fragments have semantic identities and exact content Hashes, while ContextPlanner owns their order and currently joins instruction text using one fixed representation.

Different future models may benefit from different instruction formatting even when they use the same wire protocol. Codex addresses this partly through model-catalog instruction templates and personality insertion. Cyber Agent must remain replaceable across Providers without making semantic research behavior model-specific or adding unused Prompt variants.

## Constraints

- The stable Base Contract and Domain/Phase semantics are provider neutral.
- ToolSpecs remain a separate protocol channel.
- Model capabilities and rendering behavior are never inferred from model names.
- RequestAudit must reconstruct both semantic sources and exact rendered bytes.
- A single current implementation does not justify a Trait, registry or user-facing dialect catalog.
- Prompt variation must be evaluated through fixed Trace/Shadow cases.

## Options Considered

### Option A: Keep rendering unversioned

Simple now, but future formatting changes cannot be attributed independently from content changes and can invalidate Replay ambiguously.

### Option B: Add configurable PromptDialect registry now

Flexible, but creates unused configuration, combinations and tests before a second proven renderer exists.

### Option C: Version the single renderer and defer selectable dialects

Makes current semantics auditable without speculative extension machinery.

## Decision

Choose Option C.

### Current Contract

ContextPlanner retains one fixed renderer:

```text
prompt_renderer_id = generic
prompt_renderer_version = 1
```

The renderer owns only:

- protocol separators between ordered instruction fragments;
- the exact conversion from selected fragments to ModelInputIR instruction/content channels;
- ContextDeliveryMap span/position emission.

M4-S3a1 introduces the explicit renderer identity while preserving current exact bytes. M4-S3e2 completes provenance-rich ContextDeliveryMap emission and persistence; the final renderer contract includes both without introducing a second renderer.

It does not own Base/Domain/Phase text, Tool schemas, research facts, Provider wire encoding or model selection.

Renderer ID/version participate in:

- Context and Prompt fingerprints;
- RequestAudit;
- IncrementalBaseline compatibility;
- Trace fixture identity;
- Context Recompile comparison.

A renderer version change forces Full delivery. Historical exact bytes and renderer identity remain replayable.

### Future Selectable Renderer

Do not add `PromptDialect`, a renderer Trait, registry or Provider-config field in the first v1.3 implementation.

A second renderer is admitted only when:

1. a real configured model demonstrates a repeatable failure or measurable quality/efficiency gap under `generic-v1`;
2. fixed Replay/Shadow cases isolate formatting as the cause;
3. the renderer changes expression but not semantic evidence/tool contracts;
4. an additive ADR defines an explicit renderer mapping in AgentModelProfile or Capability Mapping;
5. model-name pattern inference remains forbidden.

Using one multimodal Primary or separate text/Vision Providers does not itself require another renderer. Primary and Vision rendering identities remain scoped to their separate ProviderSessions.

## Consequences

### Positive

- Prompt formatting changes become auditable and replayable.
- Future model optimization has a clean admission boundary.
- Current code stays small with one renderer and no dynamic dispatch.

### Negative

- A renderer version is another fingerprint input and fixture field.
- A future second renderer still requires a new ADR and additive configuration.

### Rejected Extensions

- No personality-specific renderer.
- No Domain × Phase × Model Prompt variants.
- No runtime Prompt registry or arbitrary user templates.
- No automatic dialect selection from Provider/model names.

## Validation

- `generic-v1` produces deterministic bytes and complete ContextDeliveryMap coverage.
- Renderer changes reset the Baseline and appear in RequestAudit.
- ToolSpecs never enter ordinary Prompt text.
- Semantic fragments and exact rendered bytes are both reconstructable.
- No second renderer implementation exists without a real consumer and accepted ADR.
