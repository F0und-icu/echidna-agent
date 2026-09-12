# Changelog

User-visible changes are recorded here from the F4/F5 delivery onward; earlier
milestone history remains in `docs/development/STATUS.md` and Git history.

## Unreleased

### Added

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
