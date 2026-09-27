# R7-C1a separate compaction/Resume Live

Outcome: **ENGINEERING_PASS / CONTENT_LIMITED**, main-agent verified, not
independently audited. This completes the separately authorized bounded
compaction/new-process Resume lane, not whole-core or long-task acceptance.

## Scope and prerequisites

Operator explicitly authorized real target setup, preflight, then a separate
compaction/Resume test after the [stopped first lane](R7_C1A_LIVE_2026_09_26.md).
That lane remains unchanged and failed to reach compaction; this result does not
retroactively turn it into PASS or reuse its closed allowance.

Evidence: `/tmp/cyber-agent-r7-c1a-compact.Fp1Qij/`.
Product candidate: the same334-source/test freeze and debug binary as the prior
audit/Gate; no production, permanent-test, Cargo or architecture changes.

- New ordinary CSV Project with60 data rows,7206 bytes. Actual file SHA256:
  `ffb6a5d80474bd55796e74ccf6d4e1f094877b51b60fdd703c8107522c468e5b`.
- External `record_target.rs` links the current library, hashes actual file bytes
  and calls existing TargetSnapshot validation/Project writer/EventSink. The
  single Research record describes observed file bytes, OS/architecture/time;
  it does not invent execution, findings, Artifact provenance or hardware identity.
  No direct hand-authored JSONL or new public API was used.
- Separate offline Project ran the complete formal CLI path against a dummy
  loopback Provider: actual read_file, positive compaction6→3, clean exit,
  fresh-process Resume, greeting/continuation and new read. Seven dummy requests,
  zero remote calls. The Live Project starts with zero synthetic Session history.
- `compact_test.py` reuses the previous frozen external observer byte-for-byte;
  it does not copy or change product protocol/budget logic. Helper/controller,
  scope/plan/prepared files, successful preflight and observer were frozen before Live.

## Bound and actual use

One fresh allowance:12remote attempts/600active seconds,4admissions per Turn,
8192output per response, observed aggregate500000input/20000output with one
in-flight usage crossing permitted. No retry or additional target.

The explicit local configuration selected `deepseek-v4-flash`, Responses Full
delivery, context window1048576, Vision disabled. Original credentials/config
were unchanged; real key remained in the local observer memory, not the dummy
temporary Provider config. Retained-file credential scan passed.

Actual **7 HTTP200/completed requests,17.26seconds,70041input/883output tokens**
(70924 total). This short lane does not establish sustained endurance or full1M
capacity. All allowance is closed at completion.

## Results

| Step | Observed outcome | Assessment |
| --- | --- | --- |
| Read explicit CSV, report and stop | Lists Project root to depth2, then reads CSV. Reports correct header,60rows,R0001/R0060. No mutation or command execution. | Content correct; extra list_files violates the requested minimal scope |
| Explicit task closure | One reply confirms no pending task; zero Tools | PASS in this sample |
| `/compact` | One durable revision1,8source History items→2retained; latest task-closure user/assistant pair retained | Engineering PASS |
| Clean exit/new-process `--resume latest` | Same Session restored; both processes exit0 without forced termination | Engineering PASS |
| Post-Resume `hello` | Brief greeting asking what is needed, no unsolicited status dump and zero Tools | PASS in this sample |
| Ambiguous `继续` | States prior task is finished and asks for a concrete goal; zero Tools | PASS in this sample |
| New settings-only request | Reads current-settings.json once, reports correct version/currency, stops; CSV not reread | PASS in this sample |

The extra initial `list_files(path=".", max_depth=2)` is a real model-generated
ToolCall, not automatic Project Snapshot maintenance. Do not report all model
scope behavior as PASS. The result suggests a bounded follow-up on unnecessary
discovery when a valid path is explicitly supplied; it does not justify a new
classifier, permissions layer or workflow engine.

## Integrity evidence

Read-only `verify.py` / `verification.json` establish:

- 7RequestAudits;5 assistant replies agree with raw completed output, durable
  History and CLI.3Calls/3Results/3real successful Receipts agree; no mutations.
- 169audit-component references/38unique audit Blobs; all38Blob files match SHA256
  identities, component sizes and retained facts. Audited instruction bytes and
  8192response cap match actual requests. Reasoning is not persisted in History.
- Exactly one Compaction event, source count8/retained2/index6. Its EvidenceSpine
  points to the actual target hash and Research watermark1. All event prefixes
  remain unchanged; Research/Memory/fixture files remain byte-identical.
- After new-process Resume, actual requests remain Full and do not use an old
  response ID. The first new request contains task-closure guidance and excludes
  the original CSV row body. Observed input11137(last before)→9472(first after)
  is descriptive only: different prompts/steps, not a controlled savings measure.
- Both process exits are clean. Reviewed source334-entry freeze, binary, original
  Provider config, new frozen harness and prior-lane freeze remain unchanged.

No independent subagent audit, complete canonical IR reconstruction, causal
old/new model comparison, mid-execution steering, background Job test, security
target, Git/Actions or Codex login access occurred. No production fix is implied.

## Remaining work

This closes the missing **bounded compaction/Resume test**, not the earlier
missing-prerequisite CLI usability issue: an empty Research stream still causes
`/compact` to exit, by current fail-closed behavior. Test-harness timeout diagnostics
also remain deferred. Keep the observed extra directory listing as a separate
quality issue and evaluate minimal guidance/tool-contract improvements only in
a separately authorized implementation/evaluation Slice.
