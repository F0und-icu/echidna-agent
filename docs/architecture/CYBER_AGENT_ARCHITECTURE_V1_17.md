# Cyber Agent Architecture v1.17 — Explicit Snapshot Hash Verification

- Status: FROZEN ADDITIVE BASELINE (LOCAL; REMOTE HASH SEAL UNAUTHORIZED)
- Base: immutable v1 through v1.16
- Decision: ADR-0027; implementation/verification: STATUS

Cheap Project fingerprints and unrequested cached hashes remain metadata
observations, not proof of exact current content. Explicit inspect_project
hash_paths require current-byte verification through the one existing
second-level fingerprint owner, on both cache hits and rebuilds. Only requested
paths are hashed, deduplicated per invocation. Unchanged verified entries reuse
persisted cache bytes; hashed_paths records actual work even on a cache hit.

Old caches remain readable and explicit requests revalidate them. Snapshot is
still derived non-evidence, not a TargetSnapshot or Artifact authority. No new
schema, dependency, public API, cache service, watcher, retry or atomic external
filesystem snapshot is introduced. All earlier runtime invariants remain.
