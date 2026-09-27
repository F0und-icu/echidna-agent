# Changelog

User-visible changes are recorded here from the F4/F5 delivery onward; earlier
milestone history remains in `docs/development/STATUS.md` and Git history.

## Unreleased

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

- R7-F1: reject invalid Unicode patch prefixes without panic and validate new
  hunk offsets before writing. File schemas explain preimage hashes and supported
  patch syntax. Malformed argument diagnostics omit payloads and distinguish
  decoding rejection from network interruption; no automatic repair or retry.

- R6-02: reconcile finalized Responses text with streamed output and reject
  contradictory terminal status; discover exact Tool names despite explanatory
  words; clarify current Tool visibility, provenance, observed time and rerun
  controls. Preserve delivery across proven temporary activation expiry without
  keeping expired Tools callable. Verification and Live limits are in STATUS.

- R6-01: preserve verified reasoning/state-diff continuation across proven
  additive Tool activation, using the existing Baseline and audit owners.
  Incompatible changes still stop explicitly; this does not establish general
  cross-Turn continuity or completed long-task acceptance.

### Added

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
- The sole ContextPlanner caps default output/tool-loop reserves at 8192/16384
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
