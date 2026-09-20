# ADR-0030: Bounded current-fact and answer guidance

- Status: ACCEPTED for operator-authorized R6-03 continuation
- Date: 2026-09-19
- Architecture: additive v1.20; verification governed by STATUS

R6-02 raw request03 contained the current EUR Memory, yet the model repeated an
old uncertainty statement. Later answers corrected it. This does not establish
a delivery/cache/Compaction bug. Improve only the existing stable Base Contract
to generation2, not Context ordering or runtime interpretation of facts.

Guidance reconciles observations/operator facts with historical assistant prose
and summaries only for the same target, field and scope. Supported current
values should replace stale recollection; genuine conflicts remain explicit.
Memory is not fresh tool evidence. Metadata counts/manifests alone do not prove
unchanged bytes or absence of effects. Follow current requested format/length
without unrelated history. Existing evidence and control requirements remain.

Identity remains pragmatic-vulnerability-researcher; no second persona, renderer,
registry, automatic validator, reread, retry or output rewriter. Generation and
exact bytes use existing PromptComposition/fingerprint/audit/Baseline owners.
Old Events and RequestAudit Blobs remain immutable and reconstruct from recorded
bytes; no new Base selection Event or compatible-generation whitelist is needed.
The old base-v1 source resource has no runtime selector and moves byte-for-byte
to a historical test fixture. New live/resumed Turns use current generation2;
mechanical historical audit/replay never substitutes current prompt text.

Tests prove wire/identity/history preservation, not model reasoning or compliant
prose. Report byte/token-estimate cost; do not relax old pressure gates to hide
growth. Real adherence improvement needs separately authorized bounded evaluation.
No new API, Rust module, dependency, configuration or persisted schema.
