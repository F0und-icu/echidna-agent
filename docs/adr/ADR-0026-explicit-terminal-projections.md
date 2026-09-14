# ADR-0026: Explicit Terminal Projections

- Status: ACCEPTED for operator-authorized R5a
- Architecture: additive v1.16; implementation/verification: STATUS

Narrowly supersede ADR-0019's requirement to print Tool schemas in debug.
The operator wants sent prompt/input and response content, not repeated Tool
definitions. The existing Responses display omits those definitions with an
explicit notice; the actual encoded request, Tool plans and audit are unchanged.
Full/incremental labels describe the actual request. This remains a selected,
redacted content view, not a byte-exact wire capture or reconstruction.

CLI reply boundaries follow existing sampling/execution callbacks, without
inserting separators into History or ModelInputIR. UI preview truncation is
not proof of complete raw retention. Existing typed Receipt/child captures
provide bounded read_output hints, retained/observed counts and completeness;
unretained bytes are unavailable. No display-triggered reads, hashes or tools.

All release reasoning/debug-exclusion, credential redaction, durability,
bounded non-blocking Tool presentation and fact ownership contracts remain.
No new API, schema, dependency, telemetry or performance cache is introduced.
Regression tests compare actual wire and facts, streaming reply boundaries,
raw readback hints and incomplete capture labels, including release behavior.
