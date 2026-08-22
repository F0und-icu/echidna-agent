# ADR-0004: SHA-256 Artifact Blob Identity

- Status: ACCEPTED
- Date: 2026-08-23
- Deciders: project owner
- Architecture version: 1.1.0 implementation decision
- Related Slice: M2-S3
- Supersedes: none
- Superseded by: none

## Context

M2-S3 introduces the minimum local Blob Store for durable Artifact content. The frozen architecture already requires SHA-256 content-addressed Blob identities. The Adapter must stream regular local-file content once into durable staging while computing a cryptographic digest, then publish exact staged bytes without overwriting an existing Blob.

The standard library provides streaming I/O and filesystem primitives but no SHA-256 implementation. Blob identity must not depend on a system command, a transitive crate selected for another responsibility, or a handwritten cryptographic implementation.

## Constraints

- Keep one Rust package and one default binary.
- Preserve the frozen SHA-256 Artifact/Blob contract; do not edit frozen architecture files.
- Compute content identity locally without a network, Provider SDK, Codex dependency or credential.
- Retain the Artifact Store Adapter as the sole owner of Blob hashing and no-overwrite publication.
- Require ordinary tests to use local temporary projects and files only.
- Do not copy Codex source; implementation classification remains REIMPLEMENT.

## Options Considered

### Option A: Add `sha2`

Benefits:

- Provides maintained, auditable SHA-256 primitives with streaming `Digest` support.
- Keeps file I/O and Blob layout entirely in the local Adapter.
- Has a narrow, single responsibility: cryptographic Blob identity.

Costs:

- Adds a small locked dependency closure and initial local compilation cost.

### Option B: Handwrite SHA-256

Benefits:

- Avoids a direct dependency.

Costs:

- Reimplements security-sensitive cryptography with unnecessary maintenance and verification risk.
- Provides no feature benefit for this Slice.

### Option C: Invoke a system `shasum` command

Benefits:

- Avoids a Rust cryptographic crate.

Costs:

- Introduces platform-command availability and output-parsing dependencies.
- Separates hash computation from the Adapter's streamed staging write and makes exact-byte publication harder to audit.

### Option D: Use an accidental transitive crate

Benefits:

- Avoids an explicit Cargo dependency line.

Costs:

- Relies on an unstable implementation detail of unrelated dependencies.
- Obscures ownership and can silently break when the unrelated dependency graph changes.

## Decision

Choose Option A.

- Add direct dependency `sha2 0.10.9` with `default-features = false`.
- Use it only in the local Artifact Store Adapter to compute a streamed SHA-256 digest for a Blob ID.
- Keep the Blob ID validation, Artifact facts and lineage projection provider-neutral and independent of the crate.
- Treat this as local REIMPLEMENT work; no Codex code, dependency or upstream port is used.

## Consequences

### Positive

- Identical file bytes receive one stable, cryptographic content identity.
- Artifact provenance remains separate from Blob deduplication.
- Hashing occurs in the same single-pass local staging flow that produces the bytes to publish.
- The crate's responsibility is explicit, reviewable and replaceable at the Adapter boundary.

### Negative

- Cargo.lock gains the eight-package SHA-256 closure: `block-buffer`, `cpufeatures`, `crypto-common`, `digest`, `generic-array`, `sha2`, `typenum` and `version_check`.
- The dependency adds normal initial local build compilation work.

### Deferred

- Blob garbage collection and retention policy.
- Remote/object Blob storage, encryption and content transformations.
- Additional content identities or algorithms.
- Cache, Snapshot, Memory and Vision composition, which remain outside M2-S3.

## Compatibility and Migration

- No frozen architecture document changes; this realizes its existing SHA-256 Blob requirement.
- The existing outer Research Event schema remains version 1. Artifact payloads carry their independently versioned schema.
- Removing or replacing `sha2` later requires an Adapter implementation that computes the same lowercase SHA-256 Blob IDs and passes the existing Blob/Artifact regressions.

## Implementation Plan

- Add the constrained direct dependency and lockfile entries.
- Stream a regular source file into a durable staged file while updating `Sha256`.
- Validate existing final Blob bytes before treating a same-digest publication as deduplicated success.
- Cover deduplication, wrong existing content, non-regular files, symlink rejection and project isolation with permanent external tests.
- Run the full locked Cargo and Lean Gates without a Live Provider test.

## Validation

- Same bytes deduplicate the Blob while retaining independently recorded Artifact provenance.
- An existing final path with bytes that do not hash to its name is rejected without overwrite.
- Blob ancestry and source/destination symlinks are rejected before publication.
- The full offline Gate passes with no Codex source change, credential access or public Provider request.

## Revisit Trigger

Revisit only if `sha2` no longer supports the pinned Rust toolchain, an audited smaller maintained implementation offers the identical SHA-256 contract with lower measured cost, or a future accepted architecture changes the Blob identity algorithm.
