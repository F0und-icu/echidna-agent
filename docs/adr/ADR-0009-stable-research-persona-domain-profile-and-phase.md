# ADR-0009: Stable Research Persona, Domain Profile and Research Phase

- Status: ACCEPTED
- Date: 2026-08-28
- Deciders: project owner
- Architecture version: 1.2.0
- Related Slice: M4-S3a, M4-S3b, M4-S3c, M4-S3d, M4-S4
- Supersedes: using one ambiguous `Mode` concept for every domain, prompt and tool-selection concern
- Superseded by: none

## Context

Cyber Agent already has one AgentLoop, fixed source/web/mobile/binary vulnerability-research Profiles, layered Prompt composition, Direct/Deferred/Hidden Tool exposure and Replay/Shadow Eval. The existing fixed `ResearchProfile` values are actually target-domain selections. They are not personalities and they do not express whether the current work is mapping an attack surface, forming a hypothesis, executing an experiment, verifying impact or curating evidence.

General coding Agents often expose a small personality choice for tone while deriving operational behavior from base/developer instructions, project facts, Tool Specs and runtime invariants. Copying a large role/personality catalog into Cyber Agent would multiply Prompt combinations without improving evidence quality. Conversely, leaving all operational guidance in one domain Prompt makes it harder to select the right tools, measure tool-use accuracy or change the current research objective without changing Agent identity.

The design therefore needs three distinct concepts:

1. one stable vulnerability-research identity;
2. one target-domain Profile;
3. one current research Phase.

They must compose through the existing Context Planner and Tool exposure owner without creating another Agent, workflow engine, Prompt registry or Hook runtime.

## Constraints

- One process, one AgentLoop, one Session authority and one Research Ledger remain.
- The Agent is maintained by one person; configuration and abstractions must stay bounded.
- Existing source/web/mobile/binary Profiles and historical RequestAudit data remain readable.
- Prompt changes and Tool Schema/Exposure changes participate in Context fingerprints and IncrementalBaseline invalidation.
- Tool exposure improves model accuracy and token efficiency; it is not permission enforcement. Runtime remains FullAccess.
- Research facts remain in the existing Event/Research/Artifact authorities. A Phase is not a second vulnerability workflow state machine.
- Model prose cannot silently change runtime Phase or tool availability.
- Replay/Eval outputs remain derived and non-evidence.

## Options Considered

### Option A: Multiple personalities or runtime role Agents

Create mapper, hypothesizer, exploiter, verifier and report-writer personalities, possibly as separate Agents.

Benefits:

- Each role can carry highly specific instructions.

Costs:

- Multiplies Prompt combinations, state handoffs and test cases.
- Conflates tone, target domain and research objective.
- Encourages a multi-Agent runtime and duplicate Memory/Research authority.
- Makes a personal tool harder to audit and maintain.

### Option B: Keep only the existing domain Profile

Continue putting identity, domain guidance and current research objective in the same Prompt.

Benefits:

- No new Domain type or Prompt fragment.

Costs:

- Tool exposure remains too broad for the current objective.
- Phase changes require replacing or mutating a large Profile Prompt.
- Tool-accuracy Eval cannot attribute failures to domain selection versus research stage.

### Option C: Stable persona plus orthogonal DomainProfile and ResearchPhase

Keep one stable persona in the Base Contract, retain a fixed domain axis and add one fixed, typed research-phase axis inside the existing AgentLoop.

Benefits:

- Keeps Agent identity stable while changing objective and visible tools deterministically.
- Avoids combinatorial role Prompts: `mobile + verification` is composition, not a new persona.
- Gives Prompt inspection, Baseline invalidation, Tool exposure and Eval precise identities.
- Reuses the existing Mode/Prompt/Tool/Context owners.

Costs:

- Adds one bounded Domain enum, one Phase enum, fixed Prompt resources and composition tests.
- Requires an additive Session fact for phase continuity and a one-time terminology migration.

### Option D: General workflow or Prompt-stage engine

Make arbitrary stages, transitions, Prompt fragments and tools configurable through a DSL or registry.

Benefits:

- Maximum theoretical flexibility.

Costs:

- Adds a product-sized workflow system with no second real workflow.
- Weakens deterministic replay and semantic ownership.
- Violates the small single-user boundary.

## Decision

Choose Option C.

### Stable Base Contract

Cyber Agent has one stable persona named **Pragmatic Vulnerability Researcher**. It lives in the versioned Base Contract and governs communication style, epistemic discipline and collaboration:

- concise and evidence-led;
- explicit about target, version, environment and prerequisites;
- willing to use tools before treating inference as fact;
- distinguishes Candidate, Reproduced and Confirmed impact;
- retains negative controls, uncertainty and failed paths;
- prefers the smallest decisive experiment and auditable evidence.

There is no `Personality` enum, persona registry or per-Phase Agent identity. Personal Preferences may adjust presentation and working preferences but cannot replace the evidence contract or runtime invariants.

### Orthogonal DomainProfile

The existing fixed source/web/mobile/binary catalog becomes the semantic `DomainProfile` axis. The current Rust name `ResearchProfile` is a historical naming artifact and should be replaced by `DomainProfile` in M4-S3a while the project has no external compatibility obligation. Firmware remains unsupported until a separately justified fixed Profile exists.

Each DomainProfile owns only:

- domain method Prompt;
- domain-specific Direct/Deferred Tool exposure overrides;
- target-kind compatibility;
- stable ID and generation.

It does not own Agent personality, current research Phase, Provider selection or a workflow.

### Fixed ResearchPhase

The closed initial Phase set is:

```text
mapping | hypothesis | experiment | verification | evidence_curation
```

Each Phase owns only a short objective Prompt, bounded Tool exposure overrides, stable ID and generation. It does not mutate Research records or decide that a Finding is confirmed.

The first implementation defaults a new or legacy Session to `mapping` and supports explicit operator selection. A future automatic selector is allowed only as one pure, deterministic projection over already-authoritative Research Ledger fields with stable reason codes and tests. It must not use model prose, an additional model call, hidden heuristics or a general workflow engine.

Phase changes are queued and become effective only at the next Turn boundary after the selection Event is durably appended. A new Session records its default before the first model request. Read-only replay does not backfill a legacy Session; before its next real Turn, the runtime records the stable legacy default and then compiles the request. Resume otherwise restores the latest valid selection.

### Prompt Composition

The Full Request instruction/context order becomes:

1. Base Contract with stable persona.
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

Tool Specs remain a separate protocol channel and are never pasted into ordinary Prompt text.

New fragment identities are versioned:

```text
prompt:base:pragmatic-vulnerability-researcher:<generation>
prompt:domain:<domain-id>:<generation>
prompt:phase:<phase-id>:<generation>
```

Historical `prompt:mode:*` RequestAudit components remain immutable and replayable. They are not rewritten in place.

### Tool Exposure Composition

One pure Tool exposure owner composes, in order:

1. built-in Registry exposure;
2. DomainProfile override;
3. ResearchPhase override;
4. explicit `tool_search` activation of a Deferred tool;
5. runtime capability/availability filtering.

`Hidden` is terminal and cannot be promoted by a later layer. Direct/Deferred changes are context-efficiency choices, not authorization decisions. A Phase switch that changes the sent Tool Schema resets the active IncrementalBaseline with the existing `TOOLS_CHANGED` reason.

### Tool Accuracy Contract

Before adding many domain wrappers, existing tools must become easier for the model to select correctly:

- descriptions use the stable sections `Use when`, `Do not use when`, `Prefer` and `Produces`;
- input schemas express required fields, closed enums and bounds precisely;
- output schemas describe the actual stable result instead of a generic object when the shape is known;
- model-facing failures use stable code, stage and retryability fields;
- every failure remains a ToolResult paired to the original ToolCall;
- only a pre-execution argument-validation failure may be returned for correction without an ExecutionReceipt;
- correction requires a new model ToolCall; Runtime never silently retries execution;
- completed, externally mutating and interactive executions retain the existing no-auto-retry rules.

Replay/Shadow Eval adds derived, local tool-accuracy measurements for controlled cases: acceptable first-action match, invalid arguments, unmatched tools, redundant reads, missed batch opportunities, Deferred search usefulness and evidence-promotion boundary violations. These metrics do not become a new fact source or a remote telemetry system.

### Generic Tool and Extractor Sequence

After Prompt/Phase and Tool accuracy are measurable:

1. add provider-neutral `http_request` and `inspect_environment` tools;
2. add ObservationEnvelope ingestion and TargetDelta;
3. use AndroidManifest as the first bounded external Extractor example by invoking existing local tools through `run_commands`;
4. keep Frida, JADX, Semgrep, Ghidra, radare2, nmap and similar integrations behind `run_command(s)` until repeated use proves a dedicated Rust contract is valuable.

## Consequences

### Positive

- The Agent keeps one recognizable research identity across all targets and stages.
- Domain and current objective can evolve independently without a role explosion.
- Prompt and Tool Schema changes are inspectable, hashable and replayable.
- Tool accuracy can be improved with measured fixtures rather than longer prompts alone.
- The design continues to reuse one AgentLoop, Context Planner, Tool Registry and Research authority.

### Negative

- Historical and new fragment IDs use different terminology and both must remain reconstructible.
- Phase continuity adds one Session Event and corresponding reducer/upcaster tests.
- Exact Tool output schemas and structured errors require a bounded migration of existing Tool fixtures.

### Deferred

- Automatic Phase selection from Research Ledger state beyond the stable `mapping` default.
- User-defined personas, dynamic Domain Profiles and arbitrary Phase plugins.
- A general workflow language or transition graph.
- Dedicated wrappers for external vulnerability tools without repeated measured demand.

## Compatibility and Migration

- Architecture v1.0 and v1.1 remain immutable and authoritative except where Architecture v1.2 explicitly refines Prompt/Mode terminology and ordering.
- Existing Session, Research, Artifact and RequestAudit facts are not rewritten.
- A new Phase-selection Session Event uses the existing Event Store and continuous schema discipline; legacy Sessions without it resolve to `mapping` with a stable reason code.
- Renaming `ResearchProfile` to `DomainProfile` is a bounded source/API migration. Do not keep a duplicate compatibility type unless a real external consumer is found before implementation.
- Prompt/Domain/Phase generation, Tool Schema and Tool Exposure changes invalidate IncrementalBaseline and Fragment Cache identities through existing owners.
- No new crate, service, database, Provider, credential path or fact store is authorized by this ADR.

## Implementation Plan

- M4-S3a: stable Base Contract, DomainProfile terminology, ResearchPhase, Turn-boundary selection, prompt ordering, exposure composition and audit continuity.
- M4-S3b: precise Tool guidance/schema/error contracts plus Replay/Shadow tool-accuracy fixtures and reports.
- M4-S3c: provider-neutral `http_request` and `inspect_environment` tools.
- M4-S3d: ObservationEnvelope ingestion, AndroidManifest external Extractor and TargetDelta.
- M4-S4: authorized long-flow text/multimodal acceptance with the new accuracy metrics.

Each Slice remains separately unauthorized until named by the user and `STATUS.md`.

## Validation

- Exactly one stable persona and one AgentLoop exist.
- All five ResearchPhase values have stable IDs, generations, Prompt fragments and Tool exposure tests.
- DomainProfile and ResearchPhase compose without creating combined role variants.
- Phase changes take effect only at a Turn boundary and are restored on Resume.
- Historical `prompt:mode:*` requests reconstruct unchanged; new requests use domain and phase identities.
- Prompt or Phase changes invalidate the correct Context/Baseline state.
- Hidden tools remain undiscoverable; phase exposure and `tool_search` are deterministic.
- Structured pre-execution failures never claim an ExecutionReceipt; completed executions never auto-retry.
- Controlled Eval reports tool-selection errors without performing unrecorded real actions or promoting evidence.

## Revisit Trigger

Revisit only if measured long-flow research shows that the five fixed phases cannot express a recurring objective, phase-specific Tool exposure lowers success, or a second real persona/workflow implementation is required. Evidence must come from local Replay/Shadow/long-flow results, not speculative flexibility.
