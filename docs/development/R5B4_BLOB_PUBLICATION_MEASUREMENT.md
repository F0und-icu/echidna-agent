# R5b4 — Verified Blob publication without redundant staging

## Decision and boundaries

R5b2 measured repeated request/audit components. At R5b3 merge `268fcc0`,
LocalArtifactStore::publish_bytes writes and syncs a temporary file before
discovering an identical existing Blob. This is redundant I/O, not necessary
content validation. The bounded change computes the input SHA-256 and size,
then calls the existing destination verifier before staging. A verified hit
returns the same StoredBlob reference; a miss follows original durable
no-overwrite publication, including its later destination/race check.

The existing destination is still reread and SHA-256 checked on every call.
There is no metadata shortcut, remembered verification, new cache, public API,
Port, schema, dependency or production module. publish_file and readback are
unchanged. An external concurrent writer is not made atomic by this change.
Independent provenance remains owned by each Artifact/Session caller.

## Correctness and attributable I/O reduction

`tests/view_image_tool/blob_publication.rs` exercises the real private byte
store through its existing public Tool consumer; no test access was added.
Synthetic image bytes do not invoke a model. A fresh Registry is composed for
each warm call. On Unix, a deliberately old Blob-directory modified time
detects even a create/unlink pair when no temporary file survives. Repeated
publication must retain that timestamp, the Blob bytes and modified time,
while still appending distinct Artifact facts. This is a filesystem mutation
observation, not a measured syscall/byte/fsync count.

Negative cases cover same-size/restored-mtime corrupt bytes, directory and
symlink destinations; no new Artifact fact is accepted. Removing only the
fixture Blob then exercises successful durable recreation. Existing Artifact,
Memory, HTTP, audit, captured output and formal Chat regressions remain required.

Pre-change new tests: 1 expected no-write failure / 2 passing controls.
Post-change focused tests: 3 PASS. Logs live outside the Project at
`/tmp/cyber-agent-r5b4-gate.410R87/{red,focused}.log`.

## Matched formal CLI measurement

Reuse the unchanged R5b2 entry and four cases (stable, source edit, Core Memory
edit, external log growth), Full-only and incremental-capable configurations,
three repetitions each. Each case uses one Session and two normal CLI processes
with explicit Resume. Every lane validates current bytes, paired results,
durable RequestAudit reconstruction and the old event-file prefix.

```sh
cargo test --locked --release --test cli_chat_loopback delivery_measurement::chat_delivery_four_case_measurement -- --exact --ignored --nocapture --test-threads=1
```

Before production edits: PASS, 48 rows / 96 actual loopback requests, 103.15s
test time excluding compilation. Darwin x86_64, Rust1.93.1; release CLI SHA256
`5a97c49a2c3b0c9b1bdb088df9c78e8adfe8a26dbf94ffd39923797cb1cb723c`.
Evidence: `before.log` in the same external evidence directory.

Post-change: PASS, 48 rows / 96 loopback requests, 77.30s test time excluding
compilation. Release CLI SHA256:
`4760af3792b2559b9e6d6864fedf99b1d4a5540ba62e90553bffd6f7fe2bb705`.
Evidence: `affected-followup-and-after.log` (the final measurement follows
the corrected affected-test commands). All 48 matched rows have equal HTTP
body, input JSON, Tool JSON, instruction UTF-8 and audit component byte sizes,
delivery modes/reasons and existing cumulative component-token estimates.
Requests still contain 19310 compact Tool JSON bytes; no request content was
omitted to obtain this result. Manifest/Session sizes contain run-specific IDs
and were not required to be byte-equal. Each run independently verifies current
source/Memory delivery and reconstructs its audit, not just byte lengths.

CLI wall-time medians in seconds (three repetitions per cell):

| Mode / case | First process before → after | Resumed process before → after |
|---|---:|---:|
| Full / unchanged | 2.243 → 2.124 | 1.687 → 0.919 |
| Full / source edit | 2.254 → 1.923 | 1.633 → 0.754 |
| Full / Memory edit | 2.266 → 1.997 | 1.666 → 0.872 |
| Full / external log | 2.232 → 1.920 | 1.566 → 0.792 |
| Incremental / unchanged | 2.202 → 1.927 | 1.338 → 0.816 |
| Incremental / source edit | 2.103 → 1.803 | 1.397 → 0.713 |
| Incremental / Memory edit | 2.098 → 1.816 | 1.465 → 0.766 |
| Incremental / external log | 2.213 → 1.834 | 1.367 → 0.708 |

Admission rests on eliminated hit-path staging with preserved verification,
proven by the red/green filesystem regression and unchanged publication owner,
not on a required speedup percentage. Timings are supporting observations only.
Final independent audit, full Gate and Lean acceptance remain governed by STATUS.

Closure: finite independent Terra xhigh product audit and report re-review
APPROVE; complete local Gate PASS (487 unique tests / 4 opt-in ignored,
fmt/Clippy/build), Lean PASS. Source/test hashes did not change after review.
The initial affected command used a nonexistent `tool_loop` test target after
seven passing suites; corrected `tool_loop_integration` and all remaining
affected tests passed. Both logs are retained; no assertion was weakened.

These are local fake-Provider runs, not remote-model latency or research quality.
CLI wall time includes startup, context/audit/tool work and exit; excludes
fixture setup/mutation and subsequent verification. Before runs precede after
runs on one machine, with warm OS caches and fixed protocol-lane ordering;
timing differences are descriptive, not isolated causal model-speedup claims.
Internal hash/scan/syscall counts, Provider usage and TTFT remain unavailable.
Wire/schema omission, weaker integrity, retrieval promotion, full-1M capacity
and R6 acceptance are outside this Slice.
