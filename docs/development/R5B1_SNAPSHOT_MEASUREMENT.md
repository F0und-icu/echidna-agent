# R5b1 — Offline Snapshot baseline

## Scope and decision

This is a derived local test report, not a production metric, Session fact or
performance claim. R5b1 measures the existing `inspect_project` Tool boundary;
R5b Context/transport work and R6 long-task acceptance remain open. No cache
optimization is admitted by this baseline alone.

Permanent entry: `tests/memory_snapshot/measurement.rs`, registered in the
existing `memory_snapshot` integration suite. It reuses that suite's real
ToolRegistry planning/execution helper. No new Provider, production API,
dependency, instrumentation or monitoring owner is introduced.

## Fixed protocol

Two Project kinds: filesystem and Git (committed ordinary source fixtures).
Four independently initialized cases per repetition: unchanged, source-only
append, Core Memory-only append, and growth of a sibling external log. Each
case measures cold inspection, warm inspection, inspection after the mutation,
and the subsequent warm inspection. Case order rotates between repetitions.
All fixtures are disposable, created by the existing TemporaryDirectory owner.
No credentials, model, remote repository, target or Docker is involved.

- Default correctness lane: one repetition, four source files of 256 bytes.
- Opt-in measurement: five repetitions, 64 source files of 16384 bytes (1 MiB).
- Each inspection explicitly requests hashes for `source-000.txt` and
  `.cyber-agent/memory/core.md`; this is not the CLI refresh's empty hash list.
- Mutations append exactly4096 bytes. This lane tests size-visible changes,
  **not equal-size/restored-mtime invalidation**. The previous CSV Live proved
  current read_file/Memory delivery, not exact Snapshot content identity.
- Timings use Instant around existing Tool planning/execution, including Project
  discovery, writer lock, fingerprinting, cache validation, and any cache
  persistence. Setup, mutation, assertions and measurement JSON serialization
  are outside timing. OS page caches are not flushed; `cold` means no product
  Snapshot cache, not cold disk. Tool results are not durably submitted to a
  Session in this direct Port lane.

Every observation asserts success, expected Git/filesystem identity and
hit/rebuild transition, exact current hashes and lengths of both requested
files, and absence of the external log. A hit must retain identical cache
bytes; that assertion does not prove no identical rewrite/system call occurred.
There are no timing-based test thresholds. Partial output before a failed test
is not a successful measurement run.

## Reproduction and fields

Run from the repository, with reports outside any tested Project:

```sh
cargo test --locked --test memory_snapshot snapshot_four_case_contract
cargo test --locked --release --test memory_snapshot measurement::snapshot_four_case_measurement -- --ignored --exact --nocapture --test-threads=1
```

The second command emits160 JSON rows after the `SNAPSHOT_MEASUREMENT ` marker.
Split at that marker (libtest can prefix the first line with the test name).
Retain all rows plus the process exit/test summary. Compare like-for-like kind, case,
phase, build and fixture size; use repeated samples, not one best result.

| Field | Meaning / limitation |
|---|---|
| `tool_boundary_elapsed_ns` | Wall time of the direct Tool boundary, not hash time or chat latency |
| `tool_result_json_bytes` | Compact serde JSON length of actual Tool output, not wire/Context/RequestAudit bytes |
| `persisted_cache_bytes` | Sum of three current cache-file lengths, not bytes read/written or allocation |
| `disposition` | Actual Tool output's hit/rebuild status, not zero-scan proof |
| `hashed_paths` | Explicit second-level paths reported by the existing owner, not all BLAKE3/SHA calls |
| `unavailable` | Uninstrumented internal hash/scan counts, RequestAudit, Context estimate, wire size, usage, TTFT, model repeated-tool behavior |

Do not derive internal hash counts from debug text, hash-looking fields or
cached-file count. Source inspection shows metadata traversal and cache JSON
validation still occur on hits; this is qualitative evidence, not measured
cost attribution. Same-size/restored-mtime Snapshot reuse needs separate
correctness assessment before promoting any faster cache strategy.

## Baseline result — 2026-09-15

PASS: release measurement exited0,160 unique (Project kind, case, phase,
repetition) rows;60 full rebuilds and100 hits. Test runtime28.16s (includes
fixture setup/assertions, excludes compilation). Evidence:
`/tmp/cyber-agent-r5b1-gate.ZtKJ7t/measurement.log`.
Environment: Darwin x86_64, Rust1.93.1, Apple Git2.50.1; release build based on
`3f880f99b52df087c272e29c9dc11edca12d9a85`, production source unchanged.

Median direct Tool-boundary milliseconds, five observations per cell:

| Project | Case | Cold | Warm | After change/no-op | Warm after |
|---|---|---:|---:|---:|---:|
| Filesystem | unchanged | 184.788 | 47.377 | 39.018 | 38.284 |
| Filesystem | source edit | 182.932 | 46.078 | 158.792 | 44.060 |
| Filesystem | Memory edit | 183.258 | 46.334 | 156.863 | 43.190 |
| Filesystem | external log growth | 179.559 | 45.179 | 39.639 | 40.861 |
| Git | unchanged | 270.898 | 137.642 | 127.592 | 128.246 |
| Git | source edit | 274.777 | 135.211 | 251.535 | 131.940 |
| Git | Memory edit | 271.249 | 134.010 | 247.079 | 133.962 |
| Git | external log growth | 270.522 | 134.549 | 125.453 | 122.945 |

Actual compact Tool outputs are14761–14888 bytes; three cache files total
19273–19338 bytes. These ranges include real project IDs/metadata and are not
an exact deterministic byte budget. Warm results remain substantial JSON;
this lane does not show what ContextPlanner subsequently delivers.

Interpretation: existing cache reuse is observable, and external logs cause
no Snapshot invalidation in these fixtures. Git Tool-boundary time is higher
here, but no measured attribution to subprocesses, scanning, hashing, lock or
fsync is available. Warm-vs-cold is a lifecycle comparison, **not an A/B test
of a new optimization**. Do not infer real-model latency or Token savings.

Next: measure actual CLI refresh with its empty hash-path list and existing
RequestAudit/Context delivery, then isolate any repeatable bottleneck. Resolve
same-size/restored-mtime content-identity limits before proposing broader
reuse. No extra cache, watcher, RAG/index or weaker hash checks are admitted.

Independent audit, complete Gate, Lean and delivery status are in STATUS.
