# Cyber Agent Architecture v1.8 — Opt-in Provider Debug

- Version: 1.8.0
- Status: FROZEN ADDITIVE BASELINE (LOCAL; REMOTE SEAL UNAUTHORIZED)
- Base: immutable v1 through v1.7
- Governing decision: ADR-0018
- Implementation: governed by `docs/development/STATUS.md`

Default CLI reasoning presentation exists in all builds, including release, through the existing Provider Composition Root and protocol Adapter. One transient development-only CLI debug flag adds sanitized wire request/response diagnostics on local stderr. Debug option/methods are compiled out of the standard release build using `cfg(debug_assertions)`. No second Prompt renderer, wire encoder, History, event stream, output persistence or logging service is added.

Debug observes the encoded request actually used for Full/incremental delivery, and distinguishes Primary from scoped Vision requests. Credentials, encrypted sidecars and binary image data are excluded with explicit markers. Ordinary text can contain research-sensitive information; enabling debug makes that content visible locally.

Labeled reasoning stderr and explicit diagnostic output may contain Provider-returned plaintext reasoning; unavailable/opaque reasoning is not exposed. Assistant stdout and Conversation History retain the prior suppression contract. Partial/error/closed streams never become invented completed facts. Earlier architecture and its seal manifest are unchanged.
