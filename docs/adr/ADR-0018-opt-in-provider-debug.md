# ADR-0018: Opt-in Local Provider Wire Diagnostics

- Status: ACCEPTED for operator-requested F4
- Architecture version: 1.8.0
- Supersedes: normal terminal reasoning suppression, narrowly for labeled CLI stderr in all builds

The CLI displays Provider-returned plaintext reasoning on labeled stderr by default, including release builds. It observes decoded reasoning deltas/items, labels partial output as uncommitted and distinguishes observed completion from an interrupted/error stream. It does not change Assistant stdout or Conversation History.

`chat --debug` is a transient development-build-only option, off by default. Its CLI field, wire diagnostic methods and wire redaction path are compiled only under `cfg(debug_assertions)`; the standard release build cannot accept the flag and contains no full-wire debug output path. The existing Provider Composition Root selects CLI presentation for Primary and scoped Vision; generic load/doctor/tests do not gain console output. Debug observes the actual request after encoding and incoming response events before decoding; it never rebuilds Prompt content or changes model delivery.

Output is escaped, labeled local stderr, not Assistant stdout or a second durable fact/log. Each request has a process-local diagnostic identity and role. Credentials/known secret fields, opaque encrypted reasoning and image Base64 are omitted or redacted; omissions are explicit, so this is not a byte-exact wire capture. Prompt and tool text otherwise remain visible and may contain sensitive research data. This is not a general secret classifier.

Non-JSON payloads are omitted with an explicit marker because structural redaction cannot be guaranteed. The added diagnostic-only HTTP error-body read and all HTTP error-body presentation are bounded to 64 KiB. Existing continuation-rejection classification keeps its complete-body read and fallback semantics regardless of debug; an oversized body is omitted from diagnostics, not from that existing classifier. Default reasoning suppresses a completed item only when its text exactly equals the preceding accumulated deltas; different completed text is displayed rather than silently dropped.

Default reasoning presentation and explicit diagnostics show only plaintext actually returned by the Provider. They do not expose unavailable internal reasoning, decrypt opaque data, promote partial output to completed state, or persist reasoning into Conversation History. Full debug events replace, rather than duplicate, the default reasoning delta view for that request.

No debug option enters ModelInputIR, fingerprints, RequestAudit facts or Config schema. No registry, dependency, automatic input rewriting, remote logging, extra model request or automatic retry is introduced. Debug write errors are best-effort and cannot fail a successful Tool/model operation. Cancellation/closure is labeled observationally and never invents successful completion.

Offline loopback tests must compare sent request payloads against diagnostic views, cover both Provider roles, default reasoning/debug-off behavior, redaction and incomplete/error responses. Release tests prove reasoning remains visible and `--debug` is rejected. Live and remote publication require separate authorization.
