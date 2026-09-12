# Changelog

User-visible changes are recorded here from the F4/F5 delivery onward; earlier
milestone history remains in `docs/development/STATUS.md` and Git history.

## Unreleased

### Added

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

- No Config/Event schema, dependency or credential-source changes. Existing
  configured windows are unchanged; no model-name-based capacity inference.
- Vision Live color recognition passed, but estimated region coordinates were
  inaccurate. Vision observations remain derived, non-verified evidence.
