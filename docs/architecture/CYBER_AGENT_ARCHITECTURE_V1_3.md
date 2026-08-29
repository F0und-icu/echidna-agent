# Cyber Agent Architecture v1.3 — Immutable Sampling and Evidence-Guided Context

- Version: 1.3.0
- Status: FROZEN ADDITIVE BASELINE (LOCAL; REMOTE SEAL PENDING)
- Date: 2026-08-29
- Base architectures: `CYBER_AGENT_ARCHITECTURE_V1.md` 1.0.0, `CYBER_AGENT_ARCHITECTURE_V1_1.md` 1.1.0 and `CYBER_AGENT_ARCHITECTURE_V1_2.md` 1.2.0
- Governing decisions: ADR-0010, ADR-0011, ADR-0012 and ADR-0013
- Inspected Codex reference: `868c9edb0da913a5fc699a71664e65f44f6058b0`
- Implementation status: NOT_STARTED

## 1. Reading and Compatibility Rule

Read v1.0, v1.1, v1.2 and this file together. Earlier frozen files remain immutable. This version adds request-scoped immutability, orthogonal Context provenance, reference-based Compaction, deterministic security retrieval and evidence-gap guidance. It does not replace the v1.2 stable persona, DomainProfile, ResearchPhase or Tool-accuracy contracts.

This version also splits the former planned M4-S3a before implementation:

- M4-S3a1 establishes the immutable SamplingStepPlan/ResolvedToolPlan and the minimum Context slot/renderer identities they require.
- M4-S3a2 adds the stable persona, DomainProfile and ResearchPhase control plane on that foundation.

No production Slice starts from this document alone. `SEALED_MANIFEST.md` is not changed until a separately authorized Git/remote seal.

## 2. Design Objective

Cyber Agent must improve vulnerability-research quality without becoming a general Agent platform. The v1.3 additions therefore have four bounded objectives:

1. one model sampling Step observes one immutable Context, Tool, model and capability plan;
2. every delivered Context component remains attributable to its source and evidence authority;
3. Compaction preserves evidence references without copying Research claims into a second authority;
4. local retrieval and next-action guidance are deterministic, explainable and promoted only after Shadow evidence.

The runtime remains one AgentLoop, one Session authority, one Research Ledger, one primary Provider and at most one scoped Vision Provider.

## 3. Turn and Sampling-Step Authority

A Turn may contain multiple model sampling Steps separated by Tool results. Operator-controlled configuration is captured once at the Turn boundary:

```text
TurnConfigurationSnapshot {
    domain_profile_id + generation
    research_phase_id + generation
    agent_model_profile_id + generation
    primary/vision ProviderSession identities
    project and target identities required by the Turn
}
```

Each Provider request uses one immutable internal plan:

```text
SamplingStepPlan {
    step_id
    turn_configuration
    context_plan
    resolved_tool_plan
    model_profile + capability_mapping identity
    request_requirements
    prompt_renderer_id + version
}
```

The SamplingStepPlan is captured before ModelInputIR preparation and is the only request-scoped authority for:

- exact model-visible Context and ToolSpecs;
- Context, Prompt and Tool fingerprints;
- RequestAudit construction;
- validation and dispatch of model-originated ToolCalls;
- IncrementalBaseline admission;
- ToolExecutionPlan and ExecutionReceipt binding.

A queued configuration change never mutates an existing SamplingStepPlan. A Tool result or successful Deferred activation can cause the next Step in the same Turn to capture a new plan; it cannot change the plan that produced the ToolCall.

## 4. Immutable Tool Catalog and Resolved Plan

The process-local Tool catalog is immutable after composition:

```text
RegisteredTool {
    ToolName
    ToolVersion
    canonical ToolSpec
    default exposure
    Runtime binding
}
```

One pure owner resolves an ordered request view:

```text
ResolvedToolPlan {
    plan_id / semantic fingerprint
    ordered resolved entries
    ordered model-visible ToolSpecs
    final exposure and availability reason per Tool
    Tool Schema fingerprint
}
```

Resolution order remains:

```text
Registry default
  -> DomainProfile override
  -> ResearchPhase override
  -> current-Turn DeferredActivationSet
  -> runtime availability
```

Rules:

- Hidden is terminal and can never be searched, advertised or dispatched.
- Model-originated ToolCalls dispatch only through the exact ResolvedToolPlan that advertised them.
- Missing, non-activated Deferred, Hidden or unavailable Tools return a paired pre-execution ToolResult; dispatch never falls back to the raw catalog or a newer plan.
- ToolSpec normalization, argument validation, execution, pairing and failure mapping retain their existing single owners.
- Replay owns an immutable fixed Replay plan and never falls back to live Tool runtimes.
- Tool exposure remains Context optimization, not approval or permission.

### 4.1 Deferred Activation Lifetime

A successful `tool_search` updates a typed DeferredActivationSet only for later Steps of the current Turn:

- the current SamplingStepPlan remains unchanged;
- the next Step resolves a new plan with matching Deferred Tools activated;
- activation persists for the remainder of that Turn;
- it clears at the next Turn boundary and after Resume;
- RequestAudit records the exact ToolSpecs sent by every Step.

No additional durable activation fact or permanent Session-level Tool growth is introduced.

## 5. Versioned Prompt Renderer

ContextPlanner remains the only Context ordering, budget, truncation, rendering and fingerprint owner. Its first explicit renderer identity is:

```text
prompt_renderer_id = generic
prompt_renderer_version = 1
logical identity = generic-v1
```

The renderer owns only protocol separators, conversion of selected fragments into ModelInputIR instruction/content channels, and ContextDeliveryMap span/position emission. It does not own semantic Prompt text, ToolSpecs, Research facts, Provider wire encoding or model selection.

Renderer identity participates in Context/Prompt fingerprints, RequestAudit, IncrementalBaseline compatibility, Trace fixtures and Context Recompile. A renderer change forces Full delivery.

There is no PromptDialect type, Trait, registry or Config field. A second renderer requires a real configured-model failure or measured quality/efficiency gap, fixed Replay/Shadow evidence and a new ADR. Model-name inference remains forbidden.

## 6. Orthogonal Context Provenance

Context placement and semantic origin are separate axes. New Context plans use:

```text
ContextSlot =
    base | personal | project | domain | phase |
    research_state | project_snapshot | memory_core |
    retrieved_memory | session_summary | recent_history |
    user_input | turn_local_update

ContextOrigin =
    prompt_resource | operator_input | project_file | project_memory |
    research_projection | session_projection | artifact_projection |
    tool_result | model_output | replay_fixture
```

ContextSlot alone controls fixed request placement and instruction/content channel. ContextOrigin never changes ordering.

M4-S3a1 performs only the minimum non-persisted migration from the current combined ContextFragmentSource to ContextSlot plus ContextOrigin and preserves all existing rendered behavior. Full provenance is deferred to M4-S3e2 and adds:

```text
SemanticClass = instruction | authoritative_reference |
                derived_projection | model_observation |
                conversation | narrative

AuthorityLevel = fact_reference | operator_assertion |
                 derived_non_evidence | model_observation |
                 unclassified_legacy

RetentionClass = evidence_spine | stable | recent | ephemeral

FreshnessIdentity {
    target_snapshot_id?
    project_fingerprint?
    source_revision_or_hash
    producer_version?
}

ContextReferences {
    research_record_refs[]
    artifact_refs[]
    execution_receipt_refs[]
    source_event_refs[]
}
```

Validation rejects impossible combinations. In particular, a fact_reference needs an authoritative reference and a model_observation cannot be presented as verified evidence.

## 7. Fragment, Subitem and Delivery Identity

Three identities remain distinct:

1. ContextFragmentId identifies one semantic fragment.
2. ContextSubitemId is deterministically derived from the Fragment ID and fragment-local ordinal; exact bytes retain a separate content Hash.
3. Adapter-local DeliveryItemId is an optional Provider request identity generated once for a prepared request and reused only for transport retries when the explicit Wire binding supports it.

DeliveryItemId never becomes a Context, Research, Artifact or evidence identity. It never crosses Primary and Vision ProviderSessions. No Provider item ID is added to Domain ContentItem variants.

## 8. ContextDeliveryMap Sidecar

Provenance is not pasted into model-visible text and does not widen every ContentItem. ContextPlanner emits a validated sidecar:

```text
ContextDeliveryMap {
    instruction_spans[] {
        byte_range
        fragment_id + subitem_id
        content_hash + transform
        provenance
    }
    content_items[] {
        final_position
        fragment_id + subitem_id
        content_hash + transform
        provenance
    }
}
```

Every rendered instruction byte and ContentItem is attributable exactly once, except separators owned by `generic-v1`. Truncation, merging and ArtifactRef substitution retain source identities and record the transform. RequestAudit stores the map or deduplicated Blob references next to exact request components.

Historical RequestAudit remains readable. Unknown legacy provenance is explicit and is never guessed.

## 9. Typed Evidence Relations

The next additive Research payload version may replace bare evidence vectors with:

```text
EvidenceLink {
    relation: supports | contradicts | positive_control |
              negative_control | environment_control |
              reachability_proof | prerequisite_proof | impact_proof
    evidence: EvidenceRef
}
```

An EvidenceLink is a Research Ledger assertion created only by an explicit Research update. Prompt prose, Summary text, model confidence, Replay, Eval and VisionObservation cannot create or upgrade it.

Legacy upcasting is conservative:

- historical `evidence_refs` become `supports`;
- historical `negative_refs` become `contradicts`;
- neither is silently reclassified as a control;
- original Events are never rewritten.

## 10. Evidence-Spine Compaction

Session Summary v2 separates references from narrative:

```text
EvidenceSpine {
    target_snapshot_refs[]
    active_hypothesis_refs[]
    experiment_refs[]
    finding_refs[]
    next_action_refs[]
    artifact_refs[]
    execution_receipt_refs[]
    source Research stream identity + watermark/hash
}

NarrativeSummary {
    current_goal
    concise_progress
    unresolved_questions[]
    next_step_notes[]
}
```

The Compaction Event is a durable fact that Compaction occurred. EvidenceSpine and NarrativeSummary remain derived continuation data: the spine references authoritative facts and the narrative is bounded and lossy. Neither can create a verified claim.

Before append, every spine reference is validated against the bound Project, source stream and type. Resume/replay rejects missing, cross-project, stale-watermark or type-mismatched references and reprojects current Context from facts. Summary v1 remains readable through chained Upcasters with its original weaker semantics.

## 11. Deterministic Security Retrieval

The first production candidate is one fixed versioned algorithm, not a selector framework:

```text
Project/target eligibility filters
  -> exact security identifier matching
  -> fielded BM25
  -> CJK/ASCII character n-gram ranking
  -> reciprocal-rank fusion
  -> authority/freshness tiered rerank
  -> stable ID tie-break
```

The normalizer preserves CVE/CWE IDs, package/bundle IDs, classes, methods, symbols, URL components, Android permissions/intents/components, binary imports/exports/addresses, Hashes and local Research/Artifact/Receipt IDs.

Candidate pools are independently eligible Project Memory sections, Research records, Artifact/Observation metadata and explicitly requested historical audit summaries. They may reuse normalization/ranking primitives, but never share one global eligibility rule. Cross-project candidates are impossible; cross-target or stale-version candidates require an explicit historical-comparison view.

Each result records algorithm version, query fingerprint, exact matches, component ranks, fused rank, authority/freshness disposition, inclusion/exclusion reason and estimated Context cost. ContextPlanner remains the final inclusion authority.

Retrieval starts in local Shadow mode and does not alter model-visible Context. Promotion requires controlled relevance fixtures and measured recall/precision, repeated-read or Token benefit under a fixed cost budget. Indexes are deletable Cache. Persistent FTS, vectors, remote embeddings, remote rerankers and model judges are not part of v1.3.

## 12. Evidence-Gap and Experiment Guidance

One pure projection derives only gaps supported by typed Research fields and EvidenceLinks, including entry, sink, reachability, prerequisite, control, impact, repeatability, target/version, provenance and unresolved-contradiction gaps. Unknown remains distinct from missing.

Guidance ranks existing typed NextActions or caller-supplied Eval candidates using a versioned factor vector:

- boundary importance and expected gap closure;
- number of competing hypotheses discriminated;
- reproducibility and novelty versus prior action_fingerprint;
- TargetSnapshot applicability;
- execution cost, mutation risk and version-drift risk.

Hard invalidity and missing prerequisites filter before ranking. Every factor and tie-break is reported. Guidance does not change ResearchPhase, execute a Tool, create an Experiment, update Memory or promote a Finding. It begins in Shadow mode and can be ignored by the operator.

No autonomous experiment generator, workflow DSL, learned ranker or opaque model confidence is introduced.

## 13. Request Lifecycle and Failure Atomicity

```text
Turn boundary
  -> durably apply queued Domain/Phase/Model selections
  -> capture TurnConfigurationSnapshot

Each sampling Step
  -> resolve immutable Context and Tool plans
  -> render generic-v1 + ContextDeliveryMap
  -> persist mandatory RequestAudit
  -> invoke Provider
  -> validate ToolCalls against the same ResolvedToolPlan
  -> persist paired results/receipts/artifacts
  -> capture a new Step only when another sample is required
```

Rules:

- failure to resolve, validate or audit a plan stops before Provider activity;
- a plan fingerprint mismatch stops before Tool execution;
- no failed request advances the successful IncrementalBaseline;
- transport retries reuse one prepared Adapter request and optional DeliveryItemIds, but never execute local Tools;
- Tool results, Deferred activation and Context updates affect only a later SamplingStepPlan.

## 14. Replay, Audit and Evolution

- RequestAudit reconstructs semantic sources, exact rendered bytes, renderer identity, ContextDeliveryMap, sent ToolSpecs and ResolvedToolPlan fingerprint.
- Historical `prompt:mode:*`, Summary v1, Research evidence vectors and RequestAudit schemas remain readable through additive versions and chained Upcasters.
- Mechanical Replay uses recorded plans and Replay runtimes with zero real Provider/Tool action.
- Context Recompile and Shadow reports remain derived non-evidence and never overwrite historical audit.
- Primary/Vision Context maps, DeliveryItemIds, response IDs and IncrementalBaselines remain isolated.
- New persisted fields are additive and have strict unknown-event/schema behavior.

## 15. Implementation Sequence

### M4-S3a1 — Immutable Sampling Foundation

Introduce SamplingStepPlan, immutable ResolvedToolPlan, current-Turn DeferredActivationSet, `generic-v1` renderer identity and the minimum ContextSlot/ContextOrigin migration. Preserve current Prompt bytes and Tool behavior.

### M4-S3a2 — Research Control Plane

Implement the v1.2 stable Base Contract, DomainProfile terminology, five ResearchPhase values, Turn-boundary continuity and Domain/Phase Tool exposure through the a1 plan owner.

### M4-S3b — Tool Accuracy Contracts and Eval

Canonical Tool guidance, precise schemas, structured failures and derived accuracy reports.

### M4-S3c — Generic Research Tools

Bounded `http_request` and `inspect_environment` through existing Tool/Artifact/Receipt contracts.

### M4-S3d — Observation and TargetDelta

Strict ObservationEnvelope ingestion, AndroidManifest external Extractor and deterministic TargetDelta.

### M4-S3e1 — Typed Evidence Relations

Add EvidenceLink payload/upcasters and evidence-gap projection over explicit facts.

### M4-S3e2 — Provenance and Evidence-Spine Compaction

Complete Context provenance, ContextDeliveryMap persistence and Session Summary v2 without copied Research authority.

### M4-S3f — Deterministic Security Retrieval

Run the fixed retrieval pipeline in Shadow mode, add controlled fixtures and promote only after measured benefit.

### M4-S3g — Evidence-Gap Experiment Guidance

Run pure evidence-gap/NextAction ranking in Shadow mode with stable reasons and no autonomous execution.

### M4-S4 — Long-Flow Acceptance

An explicitly authorized controlled vulnerability-research flow validates correctness, evidence quality, tool accuracy, retrieval/guidance benefit, text/multimodal composition and efficiency.

Every Slice requires its own authorization. Completion of one Slice does not start the next.

## 16. Codex Reuse Decision

The inspected upstream commit demonstrates useful boundaries in its captured StepContext, ToolRouter/spec planning, response item identity handling and temporary deterministic skill-selection experiments. Cyber Agent uses those as REFERENCE only.

No Codex source was copied or ported for v1.3. Product-specific Code Mode, MCP, collaboration, app/plugin, telemetry, policy, server and model-catalog machinery remain outside scope. Any future code copy must pass the Upstream Port Ledger admission gate before implementation.

## 17. v1.3 Acceptance Invariants

- One AgentLoop, Session authority and Research Ledger remain.
- One sampling Step has one immutable Context/Tool/model/capability plan.
- Sent ToolSpecs and dispatchable runtimes are bound by the same ResolvedToolPlan.
- Deferred activation affects only later Steps of the current Turn and clears at Turn/Resume boundaries.
- `generic-v1` is the only renderer until a measured second implementation is accepted.
- ContextSlot determines placement; provenance and authority never reorder Prompt content.
- Every delivered Context component has exact sidecar provenance; Provider item IDs do not become Domain identity.
- Summary v2 references Research facts and never duplicates verified claims as authority.
- Evidence controls and proof roles require explicit Research updates.
- Retrieval and guidance are local, deterministic, explainable, Shadow-first and non-evidence.
- No vector/graph database, remote judge, workflow engine, Hook/Prompt/Tool registry, general model router, cloud service, team layer or multi-Agent runtime is introduced.

## 18. Change Control

Changing this architecture requires a new ADR and additive architecture version. Do not edit v1.0, v1.1, v1.2 or this file after local freeze. A future Git/remote seal appends exact hashes to `SEALED_MANIFEST.md`; it does not alter earlier seals.
