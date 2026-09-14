# R5b2 — Formal Chat offline delivery baseline

## Scope

This is a derived local test report, not production telemetry or real-model
quality/performance evidence. The actual CLI uses the existing Config/Provider
Composition Root and Responses Adapter against a bounded127.0.0.1 fixture.
No real key, remote model, Docker or security task is used. No production
code/API/dependency/contract/cache strategy changes are admitted by this lane.

Entry: `tests/cli_chat_cases/delivery_measurement.rs` in `cli_chat_loopback`.
It reuses existing Config, HTTP/SSE reader/writer, child deadline/pipe draining,
Session reduction and audit reconstruction helpers. A private fixed four-request
server is not an alternate protocol implementation or generic benchmark runtime.

## Fixed experiment

- Four independent filesystem Projects: unchanged, source-only edit, Core
  Memory-only edit, and1MiB growth of a log outside the Project.
- Source is `alpha=10\nbeta=20\n`, changed to `alpha=11\nbeta=20\n`.
  Memory is `currency=USD\n`, changed to `currency=EUR\n`. Mutations assert
  equal length, restore the original modified time, and verify bytes/mtime.
- Each Project has one Session, two normal process runs and explicit Resume.
  Each operator Turn has one scripted `read_file` call and a fixed final reply:
  two actual HTTP requests per Turn. Mutation happens only after process exit.
- Both configured Full-only and incremental-capable modes are tested. Resume
  must start Full with no prior response ID; later sampling uses incremental
  only when configured. Context window1048576 is configuration, not a capacity
  test. Vision is configured by the existing shared fixture but never invoked.
- Normal regression: one repetition (8 Sessions,16 CLI runs,32 requests).
  Explicit release measurement: three repetitions (24 Sessions,48 CLI runs,
  96 requests). Case order rotates; protocol lane order is fixed.
- The fake Provider supplies tool calls and `DONE_n` replies; it does not reason
  about content. Acceptance tests runtime delivery, not model comprehension.

Assertions cover success/deadlines, one Session across Resume, preserved old
event-file prefix, exact cumulative audit/Receipt counts, paired History,
current nontruncated ToolResult and its actual wire output, exact current
Memory in the new Full request, expected delivery mode/response ID and valid
audit reconstruction. Wire instructions and tool count match reconstructed IR;
this is not an exhaustive independent re-encoding of every wire field.

Snapshot fingerprint is observed separately. Only unchanged/external-log cases
require fingerprint equality. A cheap metadata fingerprint is not proof of
unchanged content. This lane does not exercise explicit `inspect_project`
hash requests and cannot establish their cached-hash freshness.

## Measurements and reproduction

```sh
cargo test --locked --test cli_chat_loopback delivery_measurement::chat_delivery_preserves_current_content_across_same_metadata_resume -- --exact
cargo test --locked --release --test cli_chat_loopback delivery_measurement::chat_delivery_four_case_measurement -- --exact --ignored --nocapture --test-threads=1
```

Retain stdout/test summary and exit status outside the tested Project. Extract
the JSON after each `CHAT_MEASUREMENT ` marker (libtest can prefix its first
line); a successful measurement has48 unique mode/case/repetition/phase rows,
each containing two received requests. Partial rows after a failure are not a
successful run. Ordinary tests do not print measurements.

- `http_body_bytes`: actual received UTF-8 HTTP body length, excluding headers,
  TLS/framing and response bytes. Headers/fake credentials are not printed.
- Input/Tools JSON lengths: compact reserialization of parsed wire sections;
  Instructions: decoded UTF-8 byte length. They need not sum to body length.
- Audit manifest bytes: compact serialization of the actual durable manifest;
  component bytes: sum of referenced sizes, not globally unique Blob storage.
- `cumulative_audit_component_token_estimate`: existing `metrics` CLI estimate
  over all current Session audits, not tokenizer usage or Context budget.
  The test does not copy the private estimator or enlarge its API.
- CLI wall time includes startup, Resume/reduction, tool/audit work, local
  transport and process exit. It excludes fixture setup/mutation and subsequent
  verification/metrics commands; it is not real-model latency or TTFT.
- Session byte length includes all existing event metadata. Internal hash
  calls/bytes/time, scan count, Provider usage and TTFT remain unavailable.

There are no timing thresholds, cache flushes, new optimization or speedup
claim. Compare only matched fixtures/build/configuration, and retain all
repetitions rather than selecting the fastest run. Actual ordinary long-task
Live requires a new bounded authorization; security long-task tests stay with
the operator. R5/R6 and retrieval promotion remain open.

## Result

Release measurement PASS (exit0),95.12s including fixture setup/verification,
excluding compilation.48 unique rows contain96 received requests:72 Full and
24 incremental. Evidence `/tmp/cyber-agent-r5b2-gate.LBGzA5/measurement.log`.
Darwin x86_64, Rust1.93.1; production base `5d7817155e772a6993e8c7f50109a5e7bf97688f`
unchanged. The default correctness lane passed40.87s.
Measured release CLI SHA256:
`3cbba504a15171c8b7d23bd99e525d2aceb142502bd43468f582d01229a14b48`.

All three repetitions and four cases had these exact HTTP body lengths:

| Configuration | First Turn request1 / request2 | Resumed Turn request1 / request2 |
|---|---:|---:|
| Full-only | 24650 / 25019 | 25205 / 25574 |
| Incremental-capable | 24650 / 21509 | 25205 / 21509 |

Every request still carries19082 bytes of compact Tool JSON and1963 decoded
instruction bytes. In the incremental continuation, Input JSON is267 bytes
versus3818/4373 bytes for Full continuations. Existing incremental delivery
therefore reduces these received request bodies by3510/4065 bytes; this is a
configured-mode observation on a tiny fixture, not a new optimization, Token
usage proof or an assertion that real backends permit Tool/instruction omission.

Median CLI seconds over three repetitions (first process / resumed process):

| Case | Full-only | Incremental-capable |
|---|---:|---:|
| unchanged | 2.229 / 1.521 | 2.075 / 1.356 |
| source edit | 2.171 / 1.550 | 2.075 / 1.329 |
| Memory edit | 2.216 / 1.599 | 2.027 / 1.406 |
| external log growth | 2.183 / 1.509 | 2.056 / 1.333 |

Both protocol lanes run in fixed order on one host with warm OS caches; do
not attribute the timing difference causally to incremental delivery. Final
Session sizes: Full-only60911–61053 bytes, incremental48710–48762 bytes.
These include per-run identifiers/times and are not wire/token counts.

All24 second phases retained equal cheap Snapshot fingerprints, including the
equal-size/restored-mtime source/Memory edits. Nevertheless the actual resumed
request/ToolResult included the new content, and the old durable event prefix
was retained. This confirms why cheap Snapshot equality cannot substitute for
actual source reads or exact Memory hashes. Explicit cached `hash_paths`
freshness remains untested by this lane and must not be declared solved.

Next admission priorities: assess explicit file-hash cache freshness before
broader reuse; then isolate large Tool/context/audit components through their
existing owners and fixed correctness fixtures. Do not remove ToolSpecs from
incremental wire requests on this evidence alone, weaken integrity checks,
add another estimator, or promote retrieval without separate quality evidence.
Full Gate, independent audit, Lean and delivery are governed by STATUS.
