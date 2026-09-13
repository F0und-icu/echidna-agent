# ADR-0023: Bounded Process Output Before Receipt Publication

- Status: ACCEPTED for operator-authorized CTF-R3a2
- Architecture: additive v1.13; CTF-R3b child checkpoints remain separate

## Decision

Separate process byte retention from model/CLI previews. Reuse the existing
captured-byte owner for foreground commands and Job/PTY streams. Retain at
most 1 MiB per stream and 8 MiB per foreground batch (divided deterministically
among its streams). Continue draining after the retention ceiling. These are
per-execution bounds, not a project-wide disk quota or unlimited retention.

Capture metadata identifies stream and optional zero-based batch child, exact
observed/retained byte counts, EOF/completeness and read error. Observed bytes
are not the unknown total produced by an interrupted process. Raw retained
bytes are the prefix; a bounded head/tail preview is derived from that prefix,
never presented as the unobserved tail of an over-limit stream. Invalid UTF-8
is preserved as bytes; text preview replacement is explicitly lossy.

The Adapter returns bounded transient captures on the existing Outcome. They
are not serialized as History or cached. One fixed App publication stage uses
the existing ArtifactStore, before scheduler Receipt callbacks, dedupe/cache,
History and resampling. Missing store, publication or metadata mismatch is a
fatal App error, never a rewritten ToolFailure or automatic retry. A partially
published Blob can remain orphaned when later publication fails; no fictitious
Receipt or rollback of the already executed command is claimed.

Receipt payload v2 adds typed `output_captures`; the exact same list is placed
on the ToolResult. One pure owner validates metadata and equality. v1 upcasts
to an empty list with legacy absence, without changing original Events. v1
must not smuggle v2 references. Existing ArtifactId fields remain unchanged:
a raw Blob capture is not a Research ArtifactRecord or Finding evidence.
Replay preserves recorded references and never republishes raw bytes.

One bounded `read_output` Tool reads immutable retained bytes through the
existing local Blob owner, revalidates SHA-256, returns offset/next offset,
stored size, text and exact base64. No Blob filesystem path, second hashing
implementation, guessed TargetSnapshot or new knowledge store is exposed.

Foreground captures are delivered only after the execution future returns.
Dropped unobserved futures remain unknown; terminal-only retention does not
solve R3b. Job/PTY running polls remain volatile and bounded. Only observed
terminal poll/stop snapshots attach immutable captures; repeated terminal
snapshots deduplicate by content identity, not by suppressing real Receipts.

## Alternatives and bounds

Changing every ToolExecutionFuture to Result is rejected: execution success
and later persistence failure have different owners. Publishing from a UI
renderer or after a Receipt is rejected. No observer registry, daemon, new
crate, automatic retry, growing Blob per running poll or global GC policy.

## Acceptance

Cover large output beyond preview, 32 children, non-UTF8, retention ceiling,
EOF/read failure and known terminal timeout/cancel states. Verify readback and
tampered Blob rejection; legacy v1/strict v2 and Receipt/Result equality;
no-sink/publication failure stops before Receipt/cache/History/resampling;
Resume and Replay preserve references without executing old tools. Focused
and affected tests precede a fresh independent review and full Gate/Lean.
