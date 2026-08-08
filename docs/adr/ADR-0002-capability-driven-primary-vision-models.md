# ADR-0002: Capability-Driven Primary and Vision Model Composition

- Status: ACCEPTED
- Date: 2026-08-09
- Deciders: project owner
- Architecture version: 1.1.0
- Related Slice: M1a-S1, M3-S2, M3-S4, M4-S4
- Supersedes: Architecture v1 assumption that the selected Provider must handle every selected image directly
- Superseded by: none

## Context

The strongest text/reasoning model and the strongest multimodal model may be different products, protocols and cost tiers. A personal vulnerability-research Agent should be able to use a fast text model for source, protocol and tool-heavy work while invoking a vision-capable model only for screenshots and image artifacts. It must also remain possible to use one multimodal model for the entire AgentLoop when that is simpler or higher quality.

Hard-coding either “one model must support everything” or “two models are always required” would create avoidable migration cost. A general model router would solve a much larger problem than this single-user product needs.

## Constraints

- Keep one process, one AgentLoop and one authoritative Session/Research state.
- A secondary vision model is a scoped inference capability, not another autonomous Agent.
- Provider-specific request and response types remain inside Adapters.
- Capability decisions must be explicit and testable; model slugs are not capability evidence.
- Images and derived crops retain Artifact Lineage and local auditability.
- Unsupported images must never be silently dropped.
- No load balancer, model marketplace, remote routing server or automatic quality/cost optimizer.
- Primary and vision credentials remain independently configurable and never use Codex login state.

## Options Considered

### Option A: Require one multimodal primary model

Benefits:

- One Provider Session and the least orchestration.
- Joint reasoning over image, text and tools.

Costs:

- Excludes excellent text-only models.
- Forces all text work onto a possibly slower or more expensive model.
- Couples the whole Agent to one Provider capability set.

### Option B: Always require separate text and vision models

Benefits:

- Each role can use a specialized model.

Costs:

- Unnecessary second-Provider complexity when the primary is already multimodal.
- Vision-to-text handoff can lose information.
- More credentials, requests and failure paths for every installation.

### Option C: One primary model plus an optional capability-scoped vision model

Benefits:

- Supports text-only primary, multimodal-only primary and mixed deployments.
- Preserves one AgentLoop and avoids a general router.
- Allows explicit cost/quality choices without hard-coding model names.

Costs:

- Requires a small deterministic VisionMode resolver.
- Secondary analysis needs separate audit, replay and failure semantics.

### Option D: General dynamic model router

Benefits:

- Could optimize model selection across many tasks.

Costs:

- Adds policy, scoring, fallback, billing and observability complexity.
- Makes behavior less reproducible.
- Conflicts with the small personal-Agent boundary.

## Decision

Choose Option C.

Cyber Agent has exactly one active `primary_profile` for the AgentLoop and zero or one `vision_profile` for scoped image analysis. Both profiles resolve to the existing Provider-neutral `ModelProvider` Port; protocol differences are handled by separate Adapters.

The active Agent Model Profile contains:

```text
AgentModelProfile {
    primary_profile_id
    vision_profile_id?
    vision_mode: auto | primary_direct | secondary_tool | disabled
}
```

`VisionMode` semantics are deterministic:

- `auto`: use `primary_direct` when the primary capability mapping declares image input; otherwise use `secondary_tool` when the configured vision profile declares image input; otherwise reject the image with a capability error.
- `primary_direct`: send selected image ContentItems to the primary Provider; configuration is invalid when primary image input is not declared.
- `secondary_tool`: keep the primary AgentLoop text/tool oriented and run scoped image analysis through `VisionAnalysisService`; configuration is invalid without an image-capable vision profile.
- `disabled`: do not expose a vision route; image input returns an explicit capability error.

The resolver evaluates declared `ModelCapabilities`, never model-name patterns. `doctor` validates the selected mode and reports capability completeness without reading or displaying credentials.

`VisionAnalysisService` is an App service using a configured `ModelProvider`; it is not a second AgentLoop. It receives an Artifact reference, a scoped task, optional crop/region references and an output contract. It has no independent project memory, Research Ledger, general Tool Runtime or autonomous continuation.

Initial user images in `secondary_tool` mode receive one explicit scoped analysis before the primary request so the image cannot be silently ignored. The primary model may later request targeted re-analysis through the `analyze_image` tool using the same service. The original image is never replaced by the derived observation.

The provider-neutral result is:

```text
VisionObservation {
    observation_id
    source_artifact_id
    derived_artifact_ids
    task_hash
    summary
    extracted_text?
    regions[]
    security_relevant_clues[]
    uncertainty[]
    provider_profile_ref
    model_capability_hash
    request_audit_manifest_ref
}
```

A `VisionObservation` is a derived model observation, not a verified vulnerability fact or Finding Evidence. Promotion still requires an explicit research update backed by reproducible target evidence.

## Routing and Session Rules

- The primary Provider owns the only AgentLoop `ProviderSession` and Incremental Baseline.
- Direct images participate in the primary canonical input and its Baseline fingerprint.
- A secondary vision call has its own scoped request identity and never shares `previous_response_id`, server output items or Incremental Baseline with the primary.
- Changing primary profile, primary capability mapping or VisionMode takes effect only after the current Turn and resets the primary ProviderSession when primary request semantics change.
- Changing only the secondary vision profile invalidates vision-derived cache entries but does not reuse or rewrite old observations.
- `auto` is capability routing only. It never changes models due to latency, price, transient failure or output quality.
- Provider failure does not silently switch to another model. The route and failure remain visible and auditable.

## Audit and Replay

Each image route records a provider-neutral `VisionRouteDecided` event or equivalent versioned Session Event containing:

- mode and resolved route;
- stable reason code;
- primary/vision profile references;
- capability mapping hashes;
- source Artifact IDs;
- resulting request/observation references.

Reason codes are stable, including:

- `PRIMARY_IMAGE_CAPABLE`
- `PRIMARY_IMAGE_UNSUPPORTED_SECONDARY_SELECTED`
- `FORCED_PRIMARY_DIRECT`
- `FORCED_SECONDARY_TOOL`
- `VISION_DISABLED`
- `NO_IMAGE_CAPABLE_PROVIDER`

Replay uses the recorded route and `VisionObservation`; it performs no real vision request. Context Recompile may recompute a candidate route but cannot rewrite the historical route or observation. Shadow Eval requires explicit user action and an isolated Eval Session.

## Consequences

### Positive

- Text-only primary models remain first-class.
- A single multimodal model remains the simplest valid deployment.
- Model replacement usually changes only Profile, Capability Mapping and Adapter.
- Vision cost is incurred only when the selected mode requires it.
- Image routing is deterministic, inspectable and replayable.

### Negative

- Secondary vision adds one model round trip before the primary can reason over an initial image.
- Structured vision output may omit details present in the original image.
- Two live Provider paths require separate compatibility tests and credentials.

### Deferred

- Automatic price/latency/quality routing.
- More than one fallback vision profile.
- Video, audio and image generation.
- Cross-provider server-side session transfer.

## Compatibility and Migration

- No runtime data exists, so no existing Event Upcaster or data migration is required now.
- Add `VisionMode`, `VisionRouteDecision` and `VisionObservation` before their first published schema.
- Existing `ModelInputIR`, `ModelProvider`, Artifact and Tool contracts remain provider neutral.
- `VisionObservation` is stored through existing Event/Artifact facts, not a new database.
- Model/Profile/Capability fingerprints include the resolved route when it changes primary request semantics.
- Rolling back to Architecture v1 means only `primary_direct` with an image-capable primary remains supported; v1.1 events must not be silently discarded.

## Implementation Plan

- M1a-S1 defines image/tool/streaming capability fields without model-name inference.
- M3-S2 adds composite Agent Model Profile, `VisionMode`, deterministic resolution and `doctor` validation.
- M3-S4 adds `VisionAnalysisService`, `analyze_image`, route events, structured observation, Artifact Lineage and direct/secondary paths.
- M4-S4 validates text-only primary plus vision, multimodal-only primary, disabled mode and profile switching.
- Mock, Fixture and Replay tests are mandatory; each live Provider path remains separately opt-in.

## Validation

- A text-only primary plus image-capable secondary completes image-to-observation-to-primary reasoning.
- A multimodal primary in `primary_direct` completes the same sample without a secondary request.
- `auto` produces the expected route for every capability matrix entry.
- Invalid forced modes fail before a model request.
- Image input is never omitted from both the primary request and secondary analysis.
- Primary and secondary ProviderSession identifiers never cross.
- Replay performs zero external model requests and reproduces the recorded observation.
- Changing Provider-specific protocols requires only Adapter/Profile changes, not AgentLoop changes.

## Revisit Trigger

Revisit only if measured workflows require more than one secondary capability model, deterministic capability routing becomes a bottleneck, or a single multimodal primary consistently makes the secondary path unnecessary enough to remove its maintenance cost.
