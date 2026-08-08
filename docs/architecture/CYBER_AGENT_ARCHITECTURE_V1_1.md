# Cyber Agent Architecture v1.1 — Capability-Driven Model Composition

- Version: 1.1.0
- Status: FROZEN ADDITIVE BASELINE
- Date: 2026-08-09
- Base architecture: `CYBER_AGENT_ARCHITECTURE_V1.md` version 1.0.0
- Governing decision: `ADR-0002-capability-driven-primary-vision-models.md`

## 1. Reading and Compatibility Rule

Architecture v1.1 is an additive frozen extension to Architecture v1.0. Read both files. All v1.0 invariants remain authoritative unless this file explicitly replaces the v1 assumption that every selected image must be handled by the primary Provider.

This version does not introduce multi-Agent orchestration or a general model router. It introduces one optional, capability-scoped secondary vision inference path inside the same process and Session authority.

## 2. Updated Runtime Topology

```text
Interactive CLI
      │
      ▼
Single Agent Runtime / AgentLoop
      │
      ├── Primary ProviderSession
      │     ├── Primary ModelProvider
      │     ├── Streaming / Tool Calls
      │     ├── IncrementalBaseline
      │     └── previous_response_id when supported
      │
      ├── Vision Route Resolver
      │     ├── primary_direct
      │     ├── secondary_tool
      │     └── disabled/rejected
      │
      ├── Optional VisionAnalysisService
      │     ├── Vision ModelProvider
      │     ├── Scoped Image Request
      │     └── VisionObservation
      │
      ├── Tool Runtime / Context / Memory / Research
      └── Local Event / Artifact / Request Audit
```

There is still exactly one AgentLoop, one Project state, one Research Ledger and one authoritative Event Stream. The vision model has no independent memory, autonomous loop, general tools or Finding authority.

## 3. Provider-Neutral Contracts

The existing `ModelProvider` Port remains unchanged:

```text
trait ModelProvider {
    async fn stream(&self, input: ModelInputIR) -> ModelEventStream;
    fn capabilities(&self) -> ModelCapabilities;
}
```

Provider protocols remain Adapter concerns. A DeepSeek-style Chat Completions Adapter, an OpenAI Responses Adapter and a future compatible/local Adapter may all implement the same Port without exposing wire types to Domain or App.

Capabilities are explicit data, not model-name inference:

```text
ModelCapabilities {
    text_input
    image_input
    streaming
    tool_call
    structured_output
    incremental_input
    usage_reporting
    context_window
    capability_mapping_version
}
```

Unknown capability is not treated as supported. Capability Mapping Hash remains part of request fingerprints and Baseline invalidation.

## 4. Composite Agent Model Profile

```text
AgentModelProfile {
    profile_id
    primary_profile_id
    vision_profile_id?
    vision_mode
    generation
}

VisionMode = auto | primary_direct | secondary_tool | disabled
```

The primary profile must satisfy the active AgentLoop requirements, initially text input, Streaming and Tool Call. Image input is optional.

The vision profile is optional. When configured for the secondary path it must support text plus image input and the required output contract; it is not required to support general Agent tools or incremental requests.

## 5. Deterministic Vision Resolution

```text
resolve(mode, primary_capabilities, vision_capabilities, has_image):
    if !has_image:
        return no_vision_route

    switch mode:
        auto:
            if primary.image_input:
                return primary_direct(PRIMARY_IMAGE_CAPABLE)
            if vision?.image_input:
                return secondary_tool(PRIMARY_IMAGE_UNSUPPORTED_SECONDARY_SELECTED)
            return reject(NO_IMAGE_CAPABLE_PROVIDER)

        primary_direct:
            require primary.image_input
            return primary_direct(FORCED_PRIMARY_DIRECT)

        secondary_tool:
            require vision?.image_input
            return secondary_tool(FORCED_SECONDARY_TOOL)

        disabled:
            return reject(VISION_DISABLED)
```

The resolver is pure and exhaustively table-tested. It does not inspect price, latency, model output, transient availability or vendor branding. It never silently changes Provider after a failure.

## 6. Direct Multimodal Path

In `primary_direct`:

- selected image ContentItems enter the primary `ModelInputIR` in canonical order;
- image budget, format validation and derived Artifact generation occur before encoding;
- primary RequestAuditManifest records each source/derived Artifact reference;
- image items participate in the primary Baseline and Context fingerprint;
- only one model request path is used;
- no secondary vision credential or request is required.

This is the preferred simple deployment when one primary model provides sufficient text, image, Streaming and Tool Call capabilities.

## 7. Secondary Vision Path

In `secondary_tool`, initial user image input is processed as follows:

```text
validate/store original image Artifact
    -> resolve image budget and derived crops
    -> create scoped VisionAnalysisPlan
    -> call VisionAnalysisService
    -> persist VisionRouteDecision and VisionObservation
    -> compile primary context with observation and Artifact references
    -> run the normal primary AgentLoop
```

The scoped request contains only:

- selected image/crop Artifacts;
- the user task or a stable default analysis task;
- minimal relevant context explicitly selected by Context Planner;
- a structured output contract;
- no general Tool Schema unless a future ADR requires it.

The primary may later invoke `analyze_image` for a new crop, question or comparison. Each invocation produces a new observation linked to the same original Artifact; it never overwrites the earlier observation.

## 8. Vision Observation and Evidence Boundary

```text
VisionObservation {
    observation_id
    source_artifact_id
    derived_artifact_ids[]
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

`VisionObservation` is a locally persisted derived model observation. It may guide a Hypothesis or Experiment, but it is not a verified fact, reproduced sink, final impact or Finding Evidence by itself. The original image and any crop remain the auditable source Artifacts.

## 9. Session and Switching Semantics

- The primary Provider owns the AgentLoop ProviderSession and IncrementalBaseline.
- A secondary vision request uses an isolated scoped request identity.
- Primary and secondary Provider Request IDs, output items, response IDs and baselines never cross.
- `/profile` and VisionMode changes take effect only after the current Turn.
- Changing primary Provider, model, capability mapping or direct/secondary semantics resets the primary Baseline when canonical primary input changes.
- Changing only the secondary profile does not rewrite old observations; caches derived from that profile are invalidated by profile/capability hash.
- `auto` may resolve differently after an explicit Profile/Capability change, but the historical decision remains immutable.

No server-side session is transferred across Providers.

## 10. Audit Events

Before any image model request, record a versioned route decision:

```text
VisionRouteDecision {
    decision_id
    vision_mode
    resolved_route
    reason_code
    primary_profile_ref
    vision_profile_ref?
    primary_capability_hash
    vision_capability_hash?
    source_artifact_ids[]
}
```

The result links to the corresponding ModelRequest, RequestAuditManifest and VisionObservation. API Keys and Authorization Headers never enter these records.

Required stable reason codes:

- `PRIMARY_IMAGE_CAPABLE`
- `PRIMARY_IMAGE_UNSUPPORTED_SECONDARY_SELECTED`
- `FORCED_PRIMARY_DIRECT`
- `FORCED_SECONDARY_TOOL`
- `VISION_DISABLED`
- `NO_IMAGE_CAPABLE_PROVIDER`

## 11. Replay and Context Rules

- Mechanical Replay consumes recorded route decisions and observations and performs zero Provider requests.
- Context Recompile may calculate a candidate decision for comparison but cannot replace the historical decision.
- A secondary observation enters primary context as a derived observation plus Artifact references, not as an Assistant message from the primary model.
- Compaction must retain the observation ID, source Artifact ID, uncertainty and current research relevance.
- Deleting Cache cannot delete the Event, observation content Blob or source Artifact.

## 12. Configuration

Mixed deployment:

```toml
[agent_profiles.default]
primary = "deepseek-fast"
vision = "openai-vision"
vision_mode = "secondary_tool"
```

Single multimodal deployment:

```toml
[agent_profiles.multimodal]
primary = "multimodal-primary"
vision_mode = "primary_direct"
```

Portable capability-driven deployment:

```toml
[agent_profiles.portable]
primary = "selected-primary"
vision = "optional-vision"
vision_mode = "auto"
```

`doctor` reports profile references, modes and declared capabilities without displaying or resolving secret values. Invalid forced modes fail configuration validation before network activity.

## 13. Testing Matrix

| Primary image | Secondary image | Mode | Expected result |
|---|---|---|---|
| yes | absent/any | auto | primary_direct |
| no | yes | auto | secondary_tool |
| no | absent/no | auto | explicit capability error |
| yes | any | primary_direct | primary_direct |
| no | any | primary_direct | configuration error |
| any | yes | secondary_tool | secondary_tool |
| any | absent/no | secondary_tool | configuration error |
| any | any | disabled | explicit capability error for image input |

Mandatory integration tests:

- text-only Mock primary plus Mock vision;
- multimodal Mock primary with proof no secondary call occurred;
- initial secondary observation before primary sampling;
- targeted `analyze_image` re-analysis;
- route and ProviderSession identity separation;
- image never omitted from both routes;
- Replay with zero real model calls;
- Profile/VisionMode changes at Turn boundary;
- provider-specific Adapter fixtures for each live protocol.

Live tests remain small and separately opt-in for the primary and vision paths. A DeepSeek live test cannot prove another Provider's image protocol, and a vision live test cannot prove the primary AgentLoop protocol.

## 14. Non-Goals

- No autonomous vision Agent.
- No general task-to-model routing.
- No automatic fallback on Provider failure.
- No price/latency/quality optimizer.
- No more than one configured secondary vision profile per Agent Profile.
- No video, audio or image generation in v1.1.
- No new remote service, queue or model gateway.

## 15. v1.1 Acceptance Invariants

- One process and one AgentLoop remain authoritative.
- A single multimodal primary works with no vision Provider configured.
- A text-only primary works with one explicitly configured vision Provider.
- Unsupported image input produces a typed error and is never silently dropped.
- Route selection is pure, deterministic and auditable.
- Model slugs never substitute for declared capability mapping.
- Primary and secondary server-side session state never cross.
- Vision output remains a derived observation linked to original Artifact Lineage.
- Changing Provider protocols does not change AgentLoop, Research or Context contracts.
- The implementation contains no general model router or multi-Agent runtime.
