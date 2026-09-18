# ADR-0027: Explicit Snapshot Hash Freshness

- Status: ACCEPTED
- Date: 2026-09-17
- Related Slice: R5b3; architecture: additive v1.17
- Authority: operator authorizes repair of the reproduced explicit-hash defect.

## Evidence and decision

At base513c8ca, six same-size/restored-mtime fixtures return old digests via
inspect_project even with explicit hash_paths. Both cache-hit filtering and
metadata-rebuild digest reuse skip existing hashes. Unchanged/size-change
controls work, as does rebuilding without the old cache. This is not a BLAKE3
failure or evidence that read_file/Memory prompt delivery is stale.

Keep the cheap two-level Snapshot design. An empty hash_paths request is a
metadata observation and may reuse old manifest hashes; it is not exact current
content verification. Explicit normalized/deduplicated paths require reading
their current bytes on every invocation, including hit/rebuild and old caches.
The existing fingerprint second-level owner validates and hashes those paths;
the cache owner persists only changed entries or an existing rebuild reason.
Unrequested hashes retain their metadata-cache semantics and are not promoted
to newly verified content. No whole-project content hashing is added.

`hashed_paths` reports paths actually hashed in this invocation, including an
unchanged verification. `status=hit` describes reuse of persisted cache bytes,
not zero content reads. Unchanged verification must not rewrite cache bytes or
created_at. Missing/nonregular/unreadable requested files fail through the
existing Tool error path; no success with a stale digest or partial cache save.
Results describe observed reads, not an atomic snapshot against arbitrary
concurrent external file mutation. No retry, watcher, ctime identity or new
fact/cache owner is introduced.

## Compatibility and cost

Tool input/output shape and cache schema remain unchanged. Existing cached
digests are revalidated when explicitly requested; historical Session/audit
facts are not rewritten. Schema descriptions clarify work/disposition semantics
through existing ToolSpec hashes. Old offline measurements remain dated
baselines, not post-fix performance claims. Explicit requests now cost the
requested bytes each time; ordinary Chat refresh passes no requested paths.
No dependency, Port, Config, Event or public API is added.

## Verification

Cover both hit and rebuild branches with same-metadata edits, Git clean/dirty/
untracked and filesystem Projects, source/Memory, cold/warm/reloaded caches,
deduplicated paths, unchanged cache bytes and invalid/nonregular requests.
Preserve current formal Chat/Memory delivery and existing no-request behavior.
Full Gate, independent audit and Lean are governed by STATUS; no Live required.
