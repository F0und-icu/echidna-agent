# Cyber Agent Architecture v1.4 — Tool Failure and Durable Accuracy Boundary

- Version: 1.4.0
- Status: FROZEN ADDITIVE BASELINE (LOCAL; REMOTE SEAL UNAUTHORIZED)
- Date: 2026-08-29
- Base architectures: `CYBER_AGENT_ARCHITECTURE_V1.md` 1.0.0, `CYBER_AGENT_ARCHITECTURE_V1_1.md` 1.1.0, `CYBER_AGENT_ARCHITECTURE_V1_2.md` 1.2.0 and `CYBER_AGENT_ARCHITECTURE_V1_3.md` 1.3.0
- Governing decision: ADR-0014
- Implementation status: M4-S3b IN_PROGRESS; no Slice completion or Gate pass is implied

## 1. Reading and compatibility rule

Read v1.0, v1.1, v1.2, v1.3 and this file together. Earlier frozen files remain immutable. This narrow additive version supersedes only the v1.2/v1.3 `ToolFailure::post_execution` stage and the related M4-S3b accuracy-observability assumptions. All other earlier contracts remain in force.

`SEALED_MANIFEST.md` is unchanged. This local freeze does not authorize a Git commit, remote publication or remote seal.

## 2. Failure boundary

The provider-neutral ToolFailure contract has exactly two stages:

```text
ToolFailure {
    code
    stage: pre_execution | execution
    retryability: correct_arguments | explicit_rerun_only | terminal
    message
    field_errors[]?
    execution_id?
    receipt_ref?
}
```

`pre_execution` covers validation, exposure, plan and argument failures before a ToolExecutionPlan starts. It remains paired with the original ToolCall in a ToolResult, has no ExecutionReceipt and may ask the model for a newly corrected ToolCall. Runtime never silently retries it.

`execution` covers an attempt after its ToolExecutionPlan starts. The attempt must produce and persist a real ExecutionReceipt, including when the Tool itself fails or is cancelled. An execution failure is never relabeled as pre-execution and is never automatically replayed. A `receipt_ref`, when present, points to the real receipt rather than a synthetic placeholder.

There is no `post_execution` ToolFailure stage in v1.4. Failures after execution while submitting or persisting a Receipt, History, RequestAudit, Artifact or related durable result are App/AgentLoop errors. They are owned by the existing application error path and must not be turned into ToolFailure. In particular:

- a real successful ToolResult is never rewritten as a ToolFailure because a later persistence operation failed;
- a missing Receipt is never filled with a fabricated Receipt;
- the Tool is not silently run again;
- a started attempt is not claimed to be durably complete until its real Receipt is available.

Only a pre-execution failure may lack a Receipt. Every started result has a real Receipt or the App/AgentLoop reports a durability error without inventing one.

## 3. Durable accuracy observability

Tool Accuracy remains a derived, local, non-evidence report. It reads only persistent Trace facts already owned by Event, RequestAudit, ToolCall/Result, ExecutionReceipt and Artifact authorities. A metric is reported only when those facts provide a typed signal and a complete denominator. An unobservable or inapplicable metric is `Deferred`/`Unavailable` with a reason; it is never silently reported as zero.

The report may measure durable first-action, argument validation/correction, redundant reads, missed batch opportunities and schema cost when their Trace inputs are complete. It may count Deferred activation only under Section 4.

The following are explicitly Deferred until a typed durable signal is introduced and consumed:

- unmatched Replay termination;
- Observation promotion;
- Replay promotion;
- Vision promotion.

Absence of a signal, a transient in-memory termination, a prose claim or a controlled-case expectation is not a negative observation. The report never writes Research, Memory, Finding or Evidence state.

## 4. Deferred activation proof

Deferred activation is not a second durable fact. It is proven only by adjacent RequestAudit ToolSpec transition for one Turn:

```text
RequestAudit[n]:   Deferred Tool absent from sent ToolSpecs
RequestAudit[n+1]: same Turn; activated Tool present in sent ToolSpecs
```

Both entries must retain their exact sent ToolSpecs and immutable ResolvedToolPlan identities. If the adjacent transition is missing, not same-Turn, not exact or not typed, activation is unobservable and the report remains Deferred; it must not infer either success or zero.

The current SamplingStepPlan remains immutable. The next Step may capture a new plan after a successful `tool_search`, while Turn and Resume lifetime rules from v1.3 remain unchanged.

## 5. Controlled tests and public API boundary

M4-S3b controlled cases remain tests-only fixtures over the existing durable Trace/Replay behavior. A test must not introduce a controlled public projection method, endpoint, feature or other production entry point solely to construct or inspect a report. Test-only access does not justify a new `pub`; existing production consumers and persisted facts remain the verification boundary.

Mechanical Replay, Context Recompile and Shadow outputs remain derived non-evidence. Without typed durable promotion or unmatched-termination signals, tests and reports preserve the Deferred state rather than manufacturing a negative result.

## 6. Compatibility and change control

- v1.0, v1.1, v1.2 and v1.3 remain readable and unmodified.
- Historical ToolFailure data with a former `post_execution` label is not rewritten. An additive reader/upcaster may preserve that historical label as legacy data while new v1.4 production output uses only `pre_execution` or `execution`; no current execution is assigned the retired stage.
- Existing Tool Call/Result pairing, Effect/Retry/Receipt rules, SamplingStepPlan, RequestAudit and Replay authorities remain unchanged except for this narrow boundary.
- No second Error owner, Trace store, activation fact, accuracy service, public test projection or model judge is introduced.
- `SEALED_MANIFEST.md` remains untouched until separately authorized publication.

## 7. v1.4 acceptance invariants

- `ToolFailure.stage` is exactly `pre_execution | execution` for new output.
- A pre-execution failure is paired and has no Receipt; a started execution has a real Receipt, including failed/cancelled attempts.
- Receipt/History/RequestAudit/Artifact persistence failure is an App/AgentLoop error, never a ToolFailure conversion, false Receipt or automatic rerun.
- Durable accuracy reports do not infer absent signals and never fake zero.
- Unmatched Replay termination and Observation/Replay/Vision promotion remain Deferred without typed durable signals.
- Deferred activation is counted only from adjacent same-Turn RequestAudit ToolSpec transition.
- Controlled tests have no public projection entry point added for test access.
- Earlier architecture files remain immutable, and this local v1.4 freeze does not change `SEALED_MANIFEST.md`.
