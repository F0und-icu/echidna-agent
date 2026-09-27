# Cyber Agent Architecture v1.24 — Task-scoped research guidance

- Date: 2026-09-25
- Additive implementation contract: ADR-0034; acceptance state in STATUS.
- Prior v1–v1.23 remain immutable. No remote hash seal or stable-core claim.

ResearchPhase is a method/exposure axis for an operator-requested task, not an
implicit request to act. Base v5 owns the shared current-task guidance; Mapping
v3 and Hypothesis/Experiment/Verification/EvidenceCuration v2 express conditional
methods. Explicit/legacy Phase selection remains the existing Session authority,
with unchanged Turn-boundary application and immutable SamplingStepPlan.

The existing selection validator accepts exactly current and known historical
Phase generations. It preserves recorded values, source/reason validation and
unknown-generation errors. PromptComposition renders current resources for new
sampling; recorded RequestAudit reconstructs old bytes without substituting new
prompts. Tool exposure and domain generations remain unchanged.

No new task runtime, classifier, registry, path scanner or automatic environment
probe. Research/Memory/Summary keep existing provenance and are not execution
commands. Cancellation, unknown effects, explicit Resume, reasoning continuity,
request admission and R7-F3 bounded argument correction remain unchanged.

Versioned bytes use the original fingerprint/audit/Baseline path and budget
checks. Offline delivery is distinct from model compliance. R7-C1 research
context and capability-package follow-ups remain design-only; this increment
does not implement a plugin loader or claim a full small-contract acceptance.
