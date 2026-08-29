# Cyber Agent Architecture v1.2 — Research Control Plane

- Version: 1.2.0
- Status: FROZEN ADDITIVE BASELINE (LOCAL; REMOTE SEAL PENDING)
- Date: 2026-08-28
- Base architectures: `CYBER_AGENT_ARCHITECTURE_V1.md` 1.0.0 and `CYBER_AGENT_ARCHITECTURE_V1_1.md` 1.1.0
- Governing decision: `ADR-0009-stable-research-persona-domain-profile-and-phase.md`
- Implementation status: NOT_STARTED

## 1. Reading and Compatibility Rule

Read v1.0, v1.1 and this file together. The earlier files remain immutable. This version explicitly refines the v1 term `Mode Prompt` into two orthogonal axes—DomainProfile and ResearchPhase—and inserts the Phase Prompt after the Domain Prompt in Full Request composition.

This version does not add another Agent, a personality catalog, a workflow engine, a Prompt/Hook registry, a new fact store or a permission system. It defines a small research control plane inside the existing one AgentLoop.

The file is frozen locally by the owner's design authorization. `SEALED_MANIFEST.md` must receive a new append-only seal only after a separately authorized Git/remote publication; no seal is implied by this local documentation change.

## 2. Runtime Topology

```text
Interactive CLI
      │
      ▼
Single Agent Runtime / AgentLoop
      │
      ├── Stable Base Contract
      │     └── Pragmatic Vulnerability Researcher
      │
      ├── Research Control
      │     ├── DomainProfile: source | web | mobile | binary
      │     ├── ResearchPhase: mapping | hypothesis | experiment
      │     │                    | verification | evidence_curation
      │     └── fixed Tool Exposure composition
      │
      ├── ContextPlanner / PromptComposition
      ├── Primary ProviderSession + optional scoped Vision Provider
      ├── Tool Runtime / Research Ledger / Memory
      └── Local Event / Artifact / RequestAudit / Replay-Eval
```

There is still one Agent identity, one AgentLoop, one current Session, one Research Ledger and one authoritative Event/Artifact chain.

## 3. Stable Agent Identity

The Base Contract has one fixed identity:

```text
persona_id = pragmatic-vulnerability-researcher
```

The Base Contract controls:

- pragmatic, concise collaboration;
- evidence before claims;
- explicit target/version/environment/prerequisites;
- Candidate/Reproduced/Confirmed separation;
- positive, negative and environmental controls;
- uncertainty and failed-path retention;
- auditable Artifact and Research references.

Operational guarantees do not rely on persona text. Tool schemas, Context selection, Event/Artifact facts, runtime validation and Eval enforce them. Personal Preferences can change presentation but cannot replace the Base Contract.

Because only one persona exists, production code must not add a `Personality` enum, persona registry, role router or persona-to-tool policy table. The versioned Base Prompt identity is sufficient.

## 4. Orthogonal Configuration Axes

The names are deliberately distinct:

| Axis | Question answered | Owns | Does not own |
|---|---|---|---|
| Stable Base Contract | Who is the Agent? | identity, collaboration and evidence discipline | current objective, Provider, permissions |
| DomainProfile | What target domain is being studied? | domain method Prompt, target compatibility, domain Tool exposure | persona, research progress, model routing |
| ResearchPhase | What objective is active now? | phase objective Prompt and phase Tool exposure | Finding truth, Research facts, workflow transitions |
| AgentModelProfile | Which configured model capabilities are used? | Primary, optional Vision and VisionMode | persona, domain or research Phase |

Do not create combined variants such as `MobileVerifier` or `WebMapper`. Compose `mobile + verification` at the Turn boundary.

### 4.1 DomainProfile

The closed initial catalog is:

```text
source | web | mobile | binary
```

Each entry contains a stable ID, generation, compile-time Markdown Prompt, TargetKind compatibility and a bounded ToolExposurePlan. Firmware remains a TargetKind without a DomainProfile.

The existing implementation name `ResearchProfile` becomes `DomainProfile` during M4-S3a. Historical serialized RequestAudit content and fragment IDs remain untouched.

### 4.2 ResearchPhase

```text
enum ResearchPhase {
    Mapping,
    Hypothesis,
    Experiment,
    Verification,
    EvidenceCuration,
}
```

Each entry contains only a stable ID, generation, short compile-time Markdown Prompt and bounded ToolExposurePlan.

| Phase | Objective | Completion signal | Default Tool posture |
|---|---|---|---|
| Mapping | Establish target, environment and attack surface | bounded map and next high-value surfaces exist | inspect/read/search/batch/environment/network/image direct as domain permits; mutation deferred |
| Hypothesis | Turn observations into falsifiable vulnerability hypotheses | hypothesis states entry, sink, prerequisites and decisive test | read/search/research direct; broad mutation and long interactive work deferred |
| Experiment | Execute the smallest decisive test | positive, negative or inconclusive result is attached to exact Snapshot/Receipt/Artifact | command/batch/job/PTY/http/mutation available as required; research/artifact direct |
| Verification | Reproduce, run controls and prove impact boundaries | repeatability, prerequisites, negative controls and final effect are explicit | execution/environment/snapshot/artifact/research direct; prefer reproducible non-destructive controls |
| Evidence Curation | Normalize evidence and prepare an auditable finding record | references, lineage, affected versions, controls and uncertainty are complete | artifact/research/memory/read direct; mutation/background execution deferred |

These are work objectives, not Finding statuses. A Phase never upgrades a Candidate or Observation automatically.

## 5. Phase Selection and Continuity

The phase selector is a fixed AgentLoop stage evaluated only at a Turn boundary.

Initial rules:

1. an explicit operator selection has priority;
2. otherwise a new Session selects `mapping` and appends that default selection before its first model request;
3. read-only replay of a legacy Session with no Phase Event resolves to `mapping` with reason `LEGACY_DEFAULT_MAPPING` without rewriting history;
4. before the next real Turn for that legacy Session, the runtime appends the resolved default selection and only then compiles the request;
5. Resume restores the latest valid Phase selection before compiling the first Full request.

The first implementation does not infer phase changes from model prose. A later pure Research Ledger projection may be added only when exact predicates, stable reason codes and a real consumer are defined and tested. It cannot call a model or mutate Research facts.

Selection is recorded in the existing Session Event stream:

```text
ResearchPhaseSelected {
    schema_version
    phase
    phase_generation
    source: default | operator
    reason_code
}
```

The Event is append-only. It is not a new database or workflow state source. The queued selection becomes effective only after its Event is durably appended at the Turn boundary; append failure stops the Turn before a model request. The Session reducer restores only the latest valid selection while retaining full history. If a real Ledger projection is later implemented, its source value and predicates require an additive payload version and tests; M4-S3a does not reserve an unused enum value.

## 6. Prompt Composition

The model request continues to use three independent protocol channels:

1. Base Instructions;
2. Conversation/Context Input Items;
3. Tool Specs.

Tool schemas are not pasted into Prompt text.

The v1.2 Full Logical Request order is:

1. Base Contract: Pragmatic Vulnerability Researcher.
2. Personal Preferences.
3. Project Prompt.
4. DomainProfile Prompt.
5. ResearchPhase Prompt.
6. ResearchWorldState Full/Diff.
7. Project Snapshot Full/Diff.
8. Project Memory Core.
9. Retrieved Memory Fragments.
10. Session Summary.
11. Recent History.
12. Current User Input.
13. Turn-local Tool/Job updates.

Stable fragments precede dynamic fragments. Existing Context budgeting, Full Logical Manifest, Append-only Candidate and ContextDecisionManifest rules remain.

Fragment identities are:

```text
prompt:base:pragmatic-vulnerability-researcher:<generation>
prompt:personal:<content-hash>
prompt:project:<content-hash>
prompt:domain:<domain-id>:<generation>
prompt:phase:<phase-id>:<generation>
```

Content Hash remains authoritative for exact bytes. Identity and generation explain semantic source/version. Historical `prompt:mode:<id>:<generation>` components remain valid in historical RequestAudit manifests.

Changing Base Contract, Personal, Project, DomainProfile or ResearchPhase invalidates the prompt fingerprint. Changing Domain/Phase Tool exposure also invalidates the Tool Schema fingerprint. A queued change never mutates a request already in flight.

## 7. Fixed Tool Exposure Composition

One pure owner resolves each Tool:

```text
resolve_tool_exposure(
    registry_default,
    domain_override?,
    phase_override?,
    deferred_activation?,
    runtime_available
) -> direct | deferred | hidden | unavailable
```

Resolution order:

1. start with Registry default;
2. apply DomainProfile override;
3. apply ResearchPhase override;
4. allow explicit `tool_search` activation only for a Deferred tool;
5. remove a schema when the runtime capability is unavailable.

Rules:

- Hidden is terminal and never searchable or promotable.
- Phase/Domain exposure may promote or defer a non-Hidden tool.
- `tool_search` is always Direct when Deferred tools exist.
- `research_get` and `research_update` remain Direct in every Phase so work is not detached from the Ledger.
- FullAccess is unchanged; exposure controls model context, not OS authorization.
- Any change to actually sent Tool Specs uses the existing `TOOLS_CHANGED` Baseline reset.

Do not introduce a policy engine, permission prompt, dynamic tool plugin or general exposure-rule DSL.

## 8. Tool Selection Accuracy Contract

Prompt instructions alone are insufficient for reliable tool use. Every model-visible ToolSpec must provide four stable guidance sections:

```text
Use when: ...
Do not use when: ...
Prefer: ...
Produces: ...
```

The ToolSpec owner must also ensure:

- exact required/optional fields and closed enums;
- explicit item, byte, time and batch bounds;
- typed output fields where the output shape is stable;
- no generic `{ "type": "object" }` output for a known result;
- stable model-facing failure codes and stages;
- the same schema is used for request hashing, Adapter encoding, validation and inspection.

The model-facing failure envelope is provider-neutral:

```text
ToolFailure {
    code
    stage: pre_execution | execution | post_execution
    retryability: correct_arguments | explicit_rerun_only | terminal
    message
    field_errors[]?
    execution_id?
    receipt_ref?
}
```

Every ToolFailure is carried in a ToolResult paired to the original ToolCall. Absence of an ExecutionReceipt never permits dropping that result. Only `pre_execution + correct_arguments` may return without an ExecutionReceipt because no ToolExecutionPlan was executed. The model must issue a new ToolCall after correcting arguments. Runtime does not silently retry it. Once execution starts, existing Effect/Retry/Receipt rules are authoritative; completed, external-mutation and interactive executions never auto-retry.

## 9. Tool Accuracy Eval

Existing Event, RequestAudit, ToolCall/Result, ExecutionReceipt and Trace fixtures remain the only sources. Eval adds derived reports, not facts.

Controlled cases may declare a set of acceptable first actions rather than one exact Tool. Reports measure:

- acceptable first-action match;
- invalid-argument count and correction outcome;
- unmatched Tool calls;
- redundant reads of unchanged content;
- missed batch opportunities;
- `tool_search` use and successful Deferred activation;
- Tool Schema token cost;
- attempts to promote Observation/Replay/Vision output beyond its evidence boundary.

Mechanical Replay performs zero real action. Shadow Eval uses Replay Tools and stops at unmatched calls. A score never updates Research, Memory or Finding state.

## 10. Fixed AgentLoop Stages

The existing AgentLoop gains bounded stages, not hooks:

```text
Turn boundary
  -> apply queued DomainProfile / ResearchPhase / AgentModelProfile
  -> compose Tool exposure
  -> capture TurnContext identities

before model request
  -> compile fixed Prompt order and ContextPlan
  -> validate sent Tool Specs
  -> choose Full or Incremental request

before tool execution
  -> schema validation
  -> structured pre-execution failure or ToolExecutionPlan

after tool result
  -> persist Receipt/Artifact/Observation references
  -> update derived State and append paired ToolResult

turn end
  -> persist Session/Research references and local metrics
```

No stage is dynamically registered. A second real implementation must exist before introducing a Trait for any new stage.

## 11. Generic Tools Before Domain Wrappers

The next generic Tools are:

### 11.1 http_request

- structured method, URL, headers, query and optional body/Artifact input;
- bounded redirects, timeout and response preview;
- status, final URL, selected response metadata, timing and body Artifact output;
- positive and negative cases remain distinct and auditable;
- no Web scanner framework or vendor-specific API client.

### 11.2 inspect_environment

- captures relevant OS/runtime, Git, proxy, compiler/toolchain and connected-device fingerprints;
- returns structured availability/version observations and one stable environment fingerprint;
- reuses existing process execution and Project Snapshot owners;
- does not become a second environment database or arbitrary probe framework.

External vulnerability tools remain available through `run_command(s)`. The first bounded Observation Extractor is AndroidManifest: invoke existing installed tooling, validate an ObservationEnvelope and retain exact ToolCall/Receipt/Artifact/Extractor provenance. Do not reimplement aapt/JADX parsing engines in Rust.

## 12. Audit, Replay and Baseline Rules

- RequestAudit records the exact Base/Domain/Phase fragments and sent Tool Specs in order.
- Session Events record Phase selection; Research Ledger remains the owner of hypotheses, experiments and findings.
- Phase/Domain Prompt or Tool exposure changes never rewrite prior requests.
- Resume creates a new Primary ProviderSession and sends Full, as in v1/v1.1.
- Fragment/Tool Schema generations participate in fingerprints and cache invalidation.
- Tool-accuracy reports are local derived outputs and cannot become Finding evidence.
- No credential, Authorization header or non-semantic transport field enters these records.

## 13. Implementation Slices

### M4-S3a — Research Control Plane

Stable Base Contract, DomainProfile terminology, five ResearchPhase values, compile-time Prompt resources, Turn-boundary selection, Session continuity, Prompt order and exposure composition.

### M4-S3b — Tool Accuracy Contracts and Eval

Canonical guidance, precise input/output schemas, structured failure semantics and controlled Replay/Shadow accuracy reports over the existing Tool catalog.

### M4-S3c — Generic Research Tools

Provider-neutral `http_request` and `inspect_environment`, with bounded outputs, Artifact links and no new fact store.

### M4-S3d — Observation and TargetDelta

ObservationEnvelope ingestion, one AndroidManifest external Extractor and deterministic TargetDelta with complete provenance.

### M4-S4 — Long-Flow Acceptance

An explicitly authorized controlled vulnerability-research flow measures research quality, tool accuracy, text/multimodal composition and efficiency.

No Slice starts from this architecture file alone.

## 14. Architecture Invariants

- Exactly one stable persona and one AgentLoop exist.
- DomainProfile, ResearchPhase and AgentModelProfile remain distinct axes.
- The fixed Phase set is not a workflow DSL or Finding state machine.
- Phase changes occur only at Turn boundaries and never from unstructured model prose.
- Prompt and Tool Specs use separate protocol channels.
- Historical RequestAudit/Event/Artifact facts are not rewritten.
- Hidden Tools cannot be discovered or promoted.
- Tool exposure never becomes a permission system.
- Invalid arguments cannot create a false execution Receipt; started/completed execution cannot be silently retried.
- Eval remains derived, local and non-evidence.
- External security tools stay behind generic process/tool contracts until repeated use proves a dedicated wrapper.
- No new Provider router, cloud service, team layer, database, Hook registry, plugin ABI or multi-Agent runtime is introduced.

## 15. Change Control

Changing this architecture requires a new ADR and additive architecture version. Do not edit v1.0 or v1.1. A future Git/remote seal appends a new `SEALED_MANIFEST.md` section with exact hashes; it does not alter earlier seals.
