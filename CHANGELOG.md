# Changelog

User-visible changes are recorded here from the F4/F5 delivery onward; earlier
changes remain available in Git history.

## Unreleased

### Selectable task sources — 2026-11-11

- Include bounded original History excerpts for sources referenced by supplied
  task cards and active constraints, even when ranked retrieval omits them.
  Selecting these references can now proceed through the existing expansion
  flow without an unknown-candidate rejection.
- Preserve earlier candidate policies for historical replay and retain rejection
  of references that do not identify available sources.

### Task preparation efficiency — 2026-11-10

- Batch disposable task postings and keep their counts in memory, removing
  per-entry authentication trees and redundant disk writes.
- Reuse synchronized Session state and incremental source digests across turns;
  scan only new events after a live append-only cursor. Preserve source identity,
  complete records, ordering, Tool pairing and unknown execution outcomes.
- Keep retrieval vocabulary, candidate policy and context capacity unchanged.
  Debug output identifies source indexing, postings, candidates and source preparation.

### Recovery and local read efficiency — 2026-11-09

- Restore current task sources and typed execution state without reconstructing
  retired Provider request chains on every Resume. Explicit historical audit
  retains full verification; Resume does not retry inputs or tools.
- Reuse unchanged local file revisions for Session/Research projections and
  verified Blob reads. Repeated Blob pages read only their requested ranges.
- Reuse prepared task sources and validate newly appended selection events;
  avoid repeated decoding of unrelated event variants during replay.
- `chat --debug` reports local startup/recovery stage timings.

### Task-context selection repairs — 2026-11-08

- Accept repeated known references, equivalent task scopes and valid overlapping
  excerpts without rejecting the turn. Preserve requested superseded constraints
  as historical context while applying the current correction; keep exact source
  ranges through subsequent refresh and Resume. Conflicting scopes and invalid
  sources or ranges still fail visibly.
- Retain Provider-reported usage for completed selectors before decision
  validation, including rejected decisions, without double counting accepted ones.
  New completed-selector usage events require a compatible binary for Resume.
- Add optional `runtime.task_selection_reasoning_effort`; omission inherits
  primary, while an explicit value changes only the same model's isolated selector.
  Main-task effort remains unchanged. No general latency improvement is claimed.

### Automatic task context — 2026-11-05

- Select relevant task history, constraints, progress and scoped Memory within
  one complete Session record. Natural follow-ups, returns and cross-task
  references use source-bound views; exclusions start a new Provider context.
- Add read-only `context_read` for scoped search, exact source expansion and
  task progress. In Chat, an omitted declaration defaults to Direct when its
  Session reader is available; explicit Hidden remains terminal. Saved binding
  bytes and historical requests retain their original meaning.
- Isolate tool-free selection samples and account for every actual request in
  the existing Turn budget. Retain actual usage, input receive time, source
  revisions, durable selection and independent replay checks. Task changes do
  not replay completed actions or bypass explicit failure recovery.
- Preserve original instruction files and historical events. New task-context
  Session events require the new binary. Routing accuracy and Live latency are
  separate from offline mechanism tests; no general speedup is claimed.
- Keep harness-generated diagnostics, Tool progress and capture/preview notices
  in English while preserving original model and Tool content.

### General technical domain guidance — 2026-11-04

- Make the persona branch's built-in Source/Web/Mobile/Binary guidance useful
  for implementation, diagnosis and analysis. Security-specific guidance applies
  when relevant to the requested task. The default research persona and phases
  keep their existing roles.
- Advance Source/Web/Mobile prompts to generation 2 and Binary to generation 3.
  Preserve old selection values, saved instruction bindings and audit bytes.
  Fresh builtin exports and new Sessions selecting the builtin definition use
  revised prompts. External definitions load their selected local files; update
  those files before a new Session or compatible reload can load the new text.
  Saved bindings remain pinned until explicit compatible reload. Older binaries
  cannot Resume Sessions with the new domain generations.
- Existing external definitions and installed binaries are not modified.

### Bounded compaction size regeneration — 2026-10-30

- Regenerate an oversized isolated compaction summary once through the existing
  prepared-request retry owner. Transport and size attempts together remain
  capped at three per node and use the original admission/audit budget.
- Report actual/maximum UTF-8 bytes on failure. Repeated oversize still defers
  without clipping JSON, discarding parent reasoning or replaying tools.
- Label and display explicit compaction debug text as compaction rather than Vision.

### Actual diagnostic content — 2026-10-28

- Prioritize personal use, functionality and efficiency. Remove message and
  debug redaction, safety-only omission of failure payloads, and masking of
  explicitly requested failure snapshots. Preserve actual server errors and
  configuration diagnostics through the existing presentation owners.
- Make explicit `chat --debug` available in release builds through the same
  diagnostic path as development builds.
- Keep protocol decoding, real Tool outcomes, Session/source integrity and
  concrete resource limits as functional contracts; no new safety workflow or
  masked/unmasked configuration switch is added.

### Long-context defaults — 2026-10-28

- Set Provider templates and the ordinary development/acceptance baseline to
  at least272000 context tokens. Use the actual declared1M window when the model
  supports it. Capability metadata remains authoritative; runtime source and
  personality/RSI behavior are unchanged by this configuration/documentation update.

### Current configuration and context behavior — 2026-10-28

- All six Provider templates document runtime admission, optional compaction
  effort and the complete proportional `runtime.context_budget` settings, with
  configurable byte/count caps. Output/tool-loop defaults are each 12.5% of the
  declared window; pressure/soft target use 80%/50% of usable input. These replace
  the historical fixed ceilings and eight-step content multiplier below.
- Compaction uses a tool-free sample or a finite preplanned segment/merge graph,
  preserving exact unfinished inputs, images and the latest complete tool batch.
  Every attempt uses the existing shared budget; replacement requires a complete
  source-verified root. Valid oversized Sessions remain available for explicit
  `/context`, `/compact` and `/exit` after resume, without tool replay.
- New handoffs use Summary v6, source-graph samples v2 and RequestAudit v4 with
  accepted-delivery lineage and frozen common-context proof. Historical summary
  units remain unchanged; older binaries cannot read the new Session events.
- Codex examples explicitly show catalog compatibility, summary support and
  optional advertised-capability import. MCP examples show independent startup
  and tool-call deadlines. Chinese/English guides and the examples index match
  current behavior. No setting guarantees all account models or complete
  internal reasoning, and model semantic quality remains separately assessed.

Entries tagged F/R/CTF below describe their original deliveries. Where later
entries supersede defaults or behavior, use the current configuration reference.

### Changed

- E0: continue development as Echidna in the new `echidna-agent` repository.
  Package/crate/binary becomes `echidna`; replies display `Echidna> `. Interactive
  terminals receive a restrained welcome and optional color (NO_COLOR/TERM=dumb
  respected). `-V` retains the short version; `--version` adds source-base HEAD.
  Existing `.cyber-agent` data, `CYBER_AGENT_*` settings, events and prompts stay
  unchanged. The old repository and new pre-rename branch retain the core checkpoint.

- R7-F2: interactive terminal input can stop or redirect the active Turn through
  existing cancellation; background Tool discovery remains available while
  Job/PTY resource leases are held. Base generation4 clarifies current operator
  scope without introducing a second task controller.
- R7-F3: malformed Tool argument JSON may trigger at most two model corrections
  per Turn within the existing shared request budget. The rejected sample never
  dispatches Tools; earlier successful results and reasoning remain retained.
  Ordinary transport/integrity failures and Tool execution are not retried.
  New version1 `model.sample_rejected` Session events are readable by the new
  binary, but old binaries cannot Resume Sessions containing these events.

- R7-F1: explicitly configured Project instructions are delivered completely or
  planning fails before sampling. Base generation3 and lossy-summary guidance
  distinguish original, repaired and current results without inventing evidence.
- R7-F1: optional role-scoped `max_output_tokens` flows through the existing
  capability mapping, Context reserve, RequestAudit and Responses encoder.
  Omission preserves legacy behavior; real-backend enforcement is not yet verified.

- R6-SC1: reuse four existing Research input types through local Schema
  definitions, preserving expanded argument contracts and tool availability.
  The measured24-tool wire array is1021bytes smaller (3.02%); this is not a
  measured model-token/latency improvement. No runtime optimizer or dependency.

### Fixed

- Fix Qwen Responses compatibility when completed reasoning items contain
  `status: null` or `content: null`. Nullable status is accepted only for
  reasoning items; missing, null and empty content use the existing empty
  reasoning path. Malformed Tool argument JSON, invalid content and conflicting
  completion snapshots remain rejected.

- R7-F1: reject invalid Unicode patch prefixes without panic and validate new
  hunk offsets before writing. File schemas explain preimage hashes and supported
  patch syntax. Malformed argument diagnostics omit payloads and distinguish
  decoding rejection from network interruption; no automatic repair or retry.

- R6-02: reconcile finalized Responses text with streamed output and reject
  contradictory terminal status; discover exact Tool names despite explanatory
  words; clarify current Tool visibility, provenance, observed time and rerun
  controls. Preserve delivery across proven temporary activation expiry without
  keeping expired Tools callable.

- R6-01: preserve verified reasoning/state-diff continuation across proven
  additive Tool activation, using the existing Baseline and audit owners.
  Incompatible changes still stop explicitly; this does not establish general
  cross-Turn continuity or completed long-task acceptance.

### Added

- Retry uncompleted model connection/response transport failures at most twice,
  with cancellable 500ms/1000ms backoff. Each attempt uses the original shared
  budget and its own request audit while retaining prepared request credentials,
  content and continuation. Abandoned partial output is marked as discarded;
  only a successful complete sample commits. HTTP/protocol/credential errors and
  Tool execution are not retried; exhaustion preserves explicit recovery. RSI
  keeps SingleSample isolation and original arm/command ownership.
  RSI samples bind the last identical pending-scope audit (at most3 attempts).
  Older persona binaries cannot resume these repeated-audit RSI sessions.

- R6-03: bounded Base Contract v2 guidance for same-scope current facts and
  requested answer format. Historical audit bytes remain unchanged; offline
  delivery coverage does not establish improved real-model adherence.

- R5b2: offline formal-CLI Full/incremental delivery measurements and a
  same-size/restored-mtime Resume regression for current source/Memory content;
  captured HTTP body sizes remain distinct from audit estimates and usage.

- R5b1: repeatable offline Snapshot measurements and a four-case regression
  through the existing Tool boundary; reports distinguish cache observations
  from unavailable internal hash, Context and Provider metrics. No runtime
  cache strategy or model behavior changes.

### Fixed

- R5b4: publishing already-owned bytes reuses an existing Blob only after the
  original full SHA-256/size/type verification, without staging and syncing a
  redundant temporary copy. Missing Blobs retain durable no-overwrite
  publication; provenance and reconstruction checks remain unchanged.

- R5b3: explicit project hash requests verify current bytes on both cache hits
  and rebuilds, including same-size/restored-mtime edits. Unchanged verification
  preserves cache bytes; hashed_paths reports actual work even on a hit.

- R5a: separate Assistant sampling replies, explicitly identify omitted debug
  Tool definitions, and distinguish UI previews from retained raw output with
  bounded read_output references. Actual prompts, ToolSpecs and Session facts
  remain unchanged; release still excludes debug and retains reasoning.

- R4b: fixed environment probes no longer infer program absence from ambiguous
  ENOENT failures. Versioned Binary/Mapping guidance distinguishes host, target,
  container, tool dialect and observed path aliases. Generation-1 selections
  remain readable while new requests use generation-2 prompts; no automatic
  preflight, new probe list or CTF-specific workflow is introduced.

- R4a: command process errors retain the requested program, effective cwd,
  observed failure stage and original OS error, without guessing an ENOENT
  cause or changing execution/retry semantics. Project-root paths now display
  as `.` instead of an empty string through the existing shared path owner.

- CTF-R3b2: Tool progress distinguishes unsaved execution, durably saved
  child results and terminal/unknown parent state, with command previews and
  elapsed/deadline labels. A single bounded best-effort presentation worker
  per Chat keeps slow tool-preview writes outside the execution loop; skipped
  previews are labeled and never change Session facts.

- CTF-R3b1: interactive command batches persist each observed child after raw
  Blob publication and before acknowledgement. Resume and Compaction retain
  known children beside explicit unknown parent state; no batch is rerun.
  `session show` reports the child checkpoint count. Receipt v2 is unchanged;
  run_commands uses catalog version3 and new child Events require a compatible
  reader. Progress presentation/backpressure remains the separate R3b2 work.

- CTF-R3a2: process previews are separate from bounded raw
  byte retention. Receipt v2 and ToolResult carry matching Blob references;
  `read_output` supports exact bounded binary readback. Publication failures
  stop before Receipt/history/resampling, without execution retry. Running or
  unobserved interrupted executions are not claimed durable by terminal capture.

- CTF-R3a1: Job/PTY handles use runtime-local opaque namespaces and checked
  sequences, preventing stale bare-counter handles from controlling new processes.
  History and Tool argument schemas are unchanged; output/partial-batch durability
  repairs remain separate.

- CTF-R2a: continuation refusal distinguishes Core Memory replacement, incompatible
  baseline/history/state projections and an over-budget delivery candidate.
  Budget failures report the existing planner estimate/limit, not inferred Provider
  usage. Compatible Prompt/state updates still continue without discarding reasoning.

- CTF-R1: resumed unknown Tool results now precede later user messages, including
  images. Real results across a user boundary fail explicitly rather than being
  reordered. Recovery does not rewrite durable History, invent Receipts or rerun Tools.

### Added

- CTF-R2b: optional Config v1 runtime request ceiling shared by Primary and
  Vision (default 32). Typed, durable model/context/budget stops return to the
  CLI and require explicit `/resume` before new work; no old input or Tool is
  automatically retried. Admission counters are not Provider usage.

- F6a: secondary Vision observations bind regions to exact delivered-image
  dimensions. Out-of-bounds regions fail explicitly without retry or Primary
  injection; in-bounds regions remain unverified model observations.
- Labeled, continuously streamed Provider-returned plaintext reasoning on CLI
  stderr in development and release builds, without adding reasoning to History.
- Development-only `chat --debug` for readable actual request content and
  Provider output; transport credentials, opaque reasoning and image bytes are
  omitted. Standard release builds exclude the flag and debug path.

### Changed

- Tool activity now shows bounded arguments, file/directory results and explicit
  partial failures, cache/deduplication and uncertain outcomes instead of long IDs.
- F5 historical default: the sole ContextPlanner capped output/tool-loop reserves at 8192/16384
  tokens respectively while preserving proportional reserves for small windows.
  An explicitly configured 1048576-token window permits 1024000 estimated input
  tokens; this is not a claim of verified server capacity.

### Migration

- F4/F5: no Config/Event schema, dependency or credential-source changes. Existing
  configured windows are unchanged; no model-name-based capacity inference.
- F6a: new Observation payloads use v2 with image geometry; legacy v1 remains
  readable with unknown coordinate space. Config, model reply, route/failure
  and Session envelope versions are unchanged. Older binaries that only read
  v1 cannot resume v2 sessions; never strip geometry or rewrite historical facts
  to downgrade them.
- Historical F5 Live recognized colors but estimated invalid region dimensions.
  F6a Live passed two original/resized synthetic half-region cases with exact
  delivered-frame coordinates; general localization accuracy remains unverified.
  Vision observations remain derived, unverified model observations.
