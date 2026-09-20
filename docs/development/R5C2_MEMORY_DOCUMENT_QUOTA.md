# R5c2 — Present Memory document quota

2026-09-20. State: COMPLETE_LOCAL_OFFLINE_VERIFIED. Main implemented; operator
subsequently authorized fresh Terra xhigh independent read-only audit. Separate
reviewer r5c2_quota_independent_audit began14:19UTC and returned APPROVE within
the ten-minute bound, no blocking findings or repairs. Full Gate and Lean then
passed. Local handoff only; no new Slice, Live or Git delivery.

## Cause and bounded correction

R5c1 measured64 notes with absent decisions.md/failed-paths.md. The existing
LocalProjectMemory::read_retrieval_documents inserted those optional paths,
sorted and truncated the path list at64, then removed missing documents. Two
missing paths consumed quota, omitting the final two real notes.

The limit now applies lazily after the original read-and-presence filter:

1. Discover/validate note paths and sort the existing fixed vocabulary.
2. Reuse original size/path/UTF-8 checks and document reader.
3. Missing documents contribute no item; present empty documents still count.
4. Take at most64 documents, without reading later document bodies.
5. Existing errors propagate immediately; they are not treated as missing.

One removed truncate and one added take, with explanatory comments. No new
loader, metadata existence probe, cache, index, normalizer or ranking owner.
Note discovery still validates directory entries before body selection, as
before; this change does not claim bounded directory enumeration or an atomic
filesystem snapshot under concurrent external mutation. The64KiB document
limit, App's64-section bound and deterministic ranking stay unchanged.

This is a bug correction inside ADR-0012's existing bounded Memory source,
not a new schema, persisted contract, fact source or architecture version.
Shadow remains local/non-evidence, does not change model Context, and reads
through the existing formal inspect retrieval CLI.

## Verification evidence

Directory: `/tmp/cyber-agent-r5c2-quota.DnAXqo/`.

- red.log: original loader,3 focused tests,1 PASS/2 FAIL. Count observed63
  instead of65; malformed note0063 was not admitted and failed to produce its
  expected error. This is the reproduced boundary, not a compilation failure.
- Three permanent tests in existing tests/retrieval_shadow.rs cover absent,
  either/both present, empty fixed documents; reverse file creation with stable
  sorted selection; exact last admitted note; oversized invalid-UTF8 first
  excluded body not read; selected size/UTF8 errors; fixed and beyond-quota
  path errors including Unix symlinks; deterministic reports and unchanged
  facts/zero Shadow effects. One private tests-only fixture helper, no production
  API widened. Earlier correctness tests remain unchanged.
- focused.log:3/3 PASS after correction. affected.log:4 suites29 PASS/1ignored
  (domain_ports_contract2, inspection_cli4, memory_snapshot14/1ignored,
  retrieval_shadow9). The ignored test is the existing opt-in offline Snapshot
  measurement, not a new skipped regression. fmt and whitespace checks PASS.
  preaudit-source.sha256 freezes all tracked source/test files for review.

Independent review scope was the loader diff, three regressions, report/STATUS/
plan changes and necessary existing context. Reviewer independently verified
all319 source/test hashes, lazy quota/presence/error behavior, separate App
section cap, tests and red/green/affected evidence. Verdict APPROVE, retained
in audit-result.md; no fixes requested. This did not itself certify full Gate.

Subsequent gate.sh/gate.log exited0: fmt, locked all-target/all-feature Clippy
with warnings denied,45 integration suites513 unique PASS/4ignored, locked
build, inline-test scan and whitespace checks. Raw514 passing occurrences
include the existing writer-lock subprocess duplicate. Final319 source/test
hash checks all match the reviewed freeze in postgate-freeze-check.log. No
production/test edits occurred after audit. Completion documentation only.

## Lean Review — PASS after Gate

- Existing Memory loader remains sole document-presence/quota owner; removed
  pre-presence path truncation and reused the same lazy reader/error pipeline.
- Production3 added/1 removed lines, including2 comments; no new production
  module, public API, Port, dependency, cache, Event, Config, Prompt or schema.
- Three permanent table-driven regressions stay in the existing top-level test
  file. One private fixture helper has three test consumers; existing CLI,
  Project, TargetSnapshot and tree comparison helpers are reused. No production
  API expansion for tests; all original tests retained and no inline src tests.
- No speculative normalizer/ranker, persistent index, duplicate authority or
  Codex PORT. Architecture/ADR remain unchanged. FullAccess/credential isolation,
  fact immutability and non-evidence Shadow boundary remain unchanged.
- Directory enumeration and concurrent filesystem mutation guarantees are not
  expanded. Release performance, per-query normalization reuse, retrieval
  promotion and model quality are unproven/deferred, not covered by this fix.

## Delivery and following work

No Git commit/push/merge, Live, Actions, model calls, credentials or Codex
authentication access in this Slice. Incoming R6 Draft PR #84 remains separate.
Release cost/profile measurement is a later offline investigation. No ranker
optimization, persistent cache, algorithm replacement or model-visible retrieval
is included or inferred from this quota fix.
