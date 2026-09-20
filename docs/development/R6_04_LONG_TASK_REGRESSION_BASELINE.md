# R6-04 — Ordinary-engineering long-task regression baseline

Date: 2026-09-20. Current execution status: [STATUS](STATUS.md).
Scope: tests and development documentation only; no production, architecture,
dependency, public API, fixture framework or prompt change. No Live/Git/Actions.

## Coverage and its limits

All entries below use existing formal CLI/Domain paths. Scripted Responses
exercise delivery and durable runtime behavior, not model decision quality.

| Boundary | Permanent test in `tests/` | Observable acceptance |
|---|---|---|
| Combined lifecycle (new) | `cli_chat_cases/current_fact_guidance.rs::explicit_compaction_then_process_resume_refreshes_facts_without_reexecuting_completed_tools` | Real once-only file append and read; formal `/compact` with a real typed source Snapshot; exit; same-size file/Memory change; new-process Resume and fresh read. Positive History reduction, current EUR delivery, immutable old USD facts/audits, exact3 calls/receipts and unchanged once marker. Full-only5 requests across2 sampling processes; compaction-only process makes0 requests. |
| Current facts alongside uncompressed old prose | same module, `resume_delivers_current_memory_alongside_old_prose_without_rewriting_history` | Current Memory and original Assistant prose coexist; audits and old Event prefix unchanged. |
| Activation/reasoning/current Project diff | `cli_chat_cases/tool_activation_continuation.rs` | Added Tool continuity, next-Turn/Resume expiry; unsupported prompt/Memory replacement still rejects. |
| Exact typed references and compaction refusal | `cli_chat_loopback.rs`, `m5_production_compaction_applies_one_durable_reference_summary_then_resumes_exactly`, `m5_compaction_preconditions_never_append_a_partial_fact`, `f3_compaction_rejects_negative_reduction_without_appending` | Real production compaction and exact current references; missing prerequisite/non-useful compaction cannot append. |
| Budget stop and explicit recovery | `cli_chat_cases/ctf_recovery.rs::ctf_r2b_budget_stop_requires_explicit_resume_without_repeating_input_or_tool` | Paired durable stop, explicit recovery, no automatic retry/replay. Historical filename is not a CTF runtime mode. |
| Known children and unknown parent execution | `cli_chat_cases/ctf_child_checkpoint.rs::ctf_r3b1_completed_child_survives_killed_batch_and_resume_without_rerun` | Serial/parallel and kill/SIGINT cases preserve acknowledged children and unknown parent outcome; no extra side effect. Reuse this existing slow test rather than duplicate it. |
| Context pressure | `cli_chat_loopback.rs::m5_context_pressure_uses_only_planner_manifest_and_never_auto_compacts` | Unchanged20,000-token critical fixture; no budget inflation or automatic compaction. |

The new case uses one finite scripted sequence and existing bounded process,
loopback, Project/Session and audit helpers. Its source Snapshot records the old
target; changing a file does not silently mint a new Research Snapshot or prove
the old one current. A real post-Resume read observes current bytes separately.
No invented Research Finding or promotion of Memory/metadata to tool evidence.

This is a repeatable **offline lifecycle baseline**, not a prolonged autonomous
Live, arbitrary crash guarantee, whole-R6 acceptance or model factuality score.
Existing tests prove unknown-outcome handling; the new case alone does not.

## Pending delivery inventory (not a staging instruction)

Incoming local HEAD is `5c07308` (R5b4/PR#83). All following work is uncommitted.
Separate scopes share some files, so future delivery must review hunks as well
as file names; never blindly stage all dirty files. No remote refresh performed.

| Scope | Principal owners and tests | Incoming verification |
|---|---|---|
| R6-01 | `domain/model`, `context/baseline`, `request_audit/manifest`, `app/agent_loop`; activation/App/audit tests; ADR0028/v1.18 | Independent APPROVE, Gate491, Lean; later R6-02 validates integration. |
| R6-02 | Same continuation owners plus private `adapters/responses/response/text`, response decode and Tool descriptions/search; Responses/CLI fixtures; ADR0029/v1.19 | Final independent re-review, Gate502, Lean;6-request Live engineering pass with content limits. |
| R6-03 | Existing Base owner and generation2 resource, exact v1 fixture; research-control/audit/CLI tests and generation expectations; ADR0030/v1.20 | Independent APPROVE, Gate506, Lean; paired6-request Live found **no measured improvement**, historical attribution errors retained. |
| R6-04 | One existing test module, current status/archive, plan/index/AGENTS and this baseline | This Slice's final verification recorded below. |

Full incoming tracked diff and source/test hashes are preserved outside the
repository at `/tmp/cyber-agent-r6-04.6OIiMf/`. These are local evidence, not
commits. R6 reports retain precise per-Slice audit/Gate and failed-run details.
Before any later publication: inspect full tracked/untracked manifest, confirm
reviewed heads and tests, exclude local configs/credentials/target artifacts,
obtain current delivery confirmation, and keep Actions paused. Shared-file
changes must not be split into unbuildable commits just to mirror Slice labels.

## Future Live rubric — proposed only

A fresh authorization must specify attempt/time ceilings, explicit Cyber Agent
config, frozen code/binary/harness and an ordinary-engineering target before
requests. Prior allowances are exhausted. Run the same broad lifecycle through
formal CLI, preserving raw requests/responses and durable facts outside Git.

Score separately: current answer correctness; historical attribution accuracy;
disclosure of unavailable/lossy information; unnecessary rereads/scans and tool
calls; actual Provider usage; raw/CLI/History consistency; compaction/Resume and
known/unknown effects. A correct current answer does not excuse fabricated
retrospective explanations. Missing data is unmeasured, not zero. Predeclare
checks before responses, retain failures and stop on integrity/duplicate-effect
failures; no hotpatch/retry/automatic extension. Security tests remain operator-run.

Only after a measured bottleneck is identified may a new Slice optimize one
existing owner. No speculative RAG/cache, new judge, prompt expansion or forced
answer rewrite follows from this small sample.

## Verification and Lean Review

Focused2 PASS (`focused-final.log`); affected7 suites147 PASS/2ignored
(`affected.log`, exit0), including CLI80 PASS/1ignored183.27s and dependency
direction. Two earlier test compile errors (missing enum import, incorrect
ToolCall field name) are preserved in `focused-first.log`/`focused.log`; no
runtime defect was found and no assertion/product rule was weakened.
Pre-Slice source/test manifest confirms only this test module changed.
Independent Terra xhigh `/root/r6_04_independent_audit` returned APPROVE within
ten minutes, no P0/P1/P2 findings or requested fixes. All seven review hashes,
archive equality, intended-only source/test delta and logs checked read-only;
reviewer ran no tests or Live. See external `audit-result.md`.

Subsequent full `gate.log` exit0: fmt, locked all-target/all-feature Clippy with
warnings denied,44 integration suites507 unique PASS/4ignored, locked build,
production inline-test and whitespace scans. Raw508 includes the existing
writer-lock subprocess duplicate. CLI80 PASS/1ignored180.09s. Test/source and
archive remained unchanged after audit/Gate; only final documentation records
are updated. No new Live, repository commit/push/merge or Actions.

Lean Review PASS after Gate: zero production/API/Port/dependency/schema/config/
prompt changes. One237-line permanent regression/import addition in the existing
CLI module; original test retained. Reuses bounded CLI/server helpers and actual
Project/Session/audit/compaction paths, no new framework or copied Core logic.
Existing slow interruption/unknown-outcome tests remain the only owners of that
coverage. No upstream PORT, temporary production test or unused runtime surface.
Model-quality/long-run improvement and cost optimization remain unimplemented
pending evidence, not represented by placeholders. Current STATUS becomes a
short active entrypoint; the entire prior chronology remains available.

Historical STATUS archive is byte-identical (SHA256 recorded in current STATUS),
including failed checkpoints and superseded permissions. Current STATUS is the
only active entrypoint; the archive cannot renew expired Live/Git permissions.
