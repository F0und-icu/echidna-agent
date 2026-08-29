# ADR-0011: Context Provenance, Evidence Relations and Evidence-Spine Compaction

- Status: ACCEPTED
- Date: 2026-08-29
- Deciders: project owner
- Architecture version: 1.3.0
- Related Slices: M4-S3e1, M4-S3e2, M4-S4
- Supersedes: treating Context placement as complete source semantics and copying Research claims into Summary strings
- Superseded by: none

## Context

Cyber Agent can reconstruct exact ModelInputIR and already distinguishes Event/Markdown/Artifact facts from derived Context, Cache, Summary and Replay outputs. Two limitations remain:

1. `ContextFragmentSource` combines fixed request ordering with semantic origin. Adding more source combinations would grow one enum without expressing authority, freshness or evidence references.
2. SessionSummary retains `verified_facts`, `active_hypotheses` and `failed_paths` as strings. Although the Summary is explicitly derived, these strings duplicate Research meaning and can drift from the Research Ledger after Compaction.

The inspected Codex source now keeps content classification attached while merging and truncating messages, and assigns stable Provider item identities for selected generated requests. Those mechanisms solve generic harness provenance and retry identity, but they do not express vulnerability-evidence authority, TargetSnapshot applicability or Research references.

## Constraints

- ContextPlanner remains the only ordering, budget, truncation and render owner.
- Research JSONL, Session JSONL, Markdown and Artifact Blobs remain the only facts.
- Provider-specific item IDs cannot enter Domain evidence identity.
- Historical RequestAudit, Session Compaction and Research Events are immutable.
- VisionObservation, Replay and Eval remain derived non-evidence.
- A Summary cannot create or upgrade a Research claim.
- New persisted semantics require additive payload versions and chained Upcasters.

## Decision

Introduce orthogonal Context provenance, typed Research evidence relations and a reference-based Evidence Spine. They share references but retain separate owners.

### Orthogonal Context Metadata

`ContextFragmentSource` is replaced for new plans by independent fields:

```text
ContextSlot {
    base | personal | project | domain | phase |
    research_state | project_snapshot | memory_core |
    retrieved_memory | session_summary | recent_history |
    user_input | turn_local_update
}

ContextOrigin {
    prompt_resource | operator_input | project_file | project_memory |
    research_projection | session_projection | artifact_projection |
    tool_result | model_output | replay_fixture
}

SemanticClass {
    instruction | authoritative_reference | derived_projection |
    model_observation | conversation | narrative
}

AuthorityLevel {
    fact_reference | operator_assertion | derived_non_evidence |
    model_observation | unclassified_legacy
}

RetentionClass {
    evidence_spine | stable | recent | ephemeral
}
```

`ContextSlot` alone determines request order and instruction/content channel. The other fields never change placement.

Freshness and source links remain typed values rather than enum combinations:

```text
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

Validation rejects impossible combinations, such as `fact_reference` with no authoritative reference, or `model_observation` presented as verified evidence.

### Fragment and Subitem Identity

Three identities remain distinct:

1. `ContextFragmentId`: stable semantic fragment identity.
2. `ContextSubitemId`: deterministic fragment-local identity derived from Fragment ID and subitem ordinal; exact bytes retain a separate content Hash.
3. Adapter-local `DeliveryItemId`: optional Provider request identity generated once for a prepared request and reused for transport retries.

`DeliveryItemId` is scoped to one ProviderSession/request, never crosses Primary and Vision, and is not a Research, Context or Artifact identity. It is sent only when the explicit Wire binding supports such IDs; no model-name inference is allowed.

### ContextDeliveryMap

Provenance is not inserted into model-visible text and does not widen every `ContentItem` variant. ContextPlanner produces a validated sidecar:

```text
ContextDeliveryMap {
    instruction_spans[] {
        byte range
        Fragment/Subitem identity
        content Hash / transform
        provenance
    }
    content_items[] {
        final position
        Fragment/Subitem identity
        content Hash / transform
        provenance
    }
}
```

Every rendered instruction byte and ContentItem must be attributable exactly once, except protocol separators owned by the versioned Prompt renderer. RequestAudit persists the map or its deduplicated Blob references alongside the exact request components. Position remains useful for reconstruction but is no longer the only source description.

### Typed Research Evidence Relations

The next additive Research payload version may replace bare positive/negative vectors with:

```text
EvidenceLink {
    relation: supports | contradicts | positive_control |
              negative_control | environment_control |
              reachability_proof | prerequisite_proof | impact_proof
    evidence: EvidenceRef
}
```

The relation is a Research Ledger assertion created only through explicit Research updates. It is not inferred from Prompt prose, Summary text or model confidence.

Legacy fields upcast conservatively:

- historical `evidence_refs` become `supports`;
- historical `negative_refs` become `contradicts`;
- neither is silently relabeled as a control.

Control relations require new explicit updates. Original Events remain unchanged.

### Evidence-Spine Compaction

Session Summary v2 separates:

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

EvidenceSpine is assembled and validated from existing facts before the Compaction Event is appended. NarrativeSummary is bounded and lossy. It can help the model continue but cannot supply verified facts.

On Resume or replay:

- every EvidenceSpine reference is validated against the bound Project and source stream;
- a missing, cross-project, stale-watermark or type-mismatched reference fails the Compaction projection;
- current Context is reprojected from facts and references;
- old Summary v1 remains readable through the existing schema chain and retains its original weaker semantics.

## Consequences

### Positive

- Every final request component can be traced to its semantic source.
- Compaction cannot silently redefine a Finding or lose its target boundary.
- Freshness, evidence authority and model observations remain distinguishable.
- Research evidence gaps can be derived without parsing prose.

### Negative

- RequestAudit and Compaction gain additive schemas.
- Context rendering must emit a sidecar map and validate span/position coverage.
- Existing Research evidence fields require a conservative Upcaster before typed controls are available.

### Rejected Alternatives

- Do not add Provider IDs directly to `ContentItem`.
- Do not store a second evidence database or graph database.
- Do not persist model-generated evidence-gap conclusions as facts.
- Do not rewrite old Session or Research streams.
- Do not make Summary authoritative.

## Validation

- ContextSlot order is independent of Origin/Authority/Retention.
- Each rendered component maps to exact Fragment/Subitem provenance.
- Truncation and ArtifactRef substitution retain source identity and record the transform.
- Legacy unknown provenance is explicit, never guessed.
- EvidenceLink control roles require explicit facts.
- Summary v2 contains no copied verified-fact, hypothesis or failed-path prose as authority.
- Deleting derived Summary/Context maps still leaves all Research and Artifact facts recoverable.
- Mechanical Replay and Context Recompile never upgrade derived content to evidence.
