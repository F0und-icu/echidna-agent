# Changelog

User-visible changes are recorded here from the F4/F5 delivery onward; earlier
milestone history remains in `docs/development/STATUS.md` and Git history.

## Unreleased

### Fixed

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
