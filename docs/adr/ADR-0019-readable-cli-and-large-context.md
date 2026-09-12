# ADR-0019: Readable CLI Content and Large-Window Budget

- Status: ACCEPTED for operator-authorized F5; local verification complete with explicit Live limitations recorded in STATUS
- Architecture version: 1.9.0
- Supersedes: ADR-0018 per-event debug presentation only; uncapped proportional default reserves

Development-only debug remains an ephemeral view of the actual encoded request, never a second Prompt compiler. Display instructions, input and Tool schemas as readable content, label Full versus incremental delivery, suppress routine SSE envelopes, and reuse decoded model events for reasoning. Primary Assistant output stays on stdout without a second stderr copy; scoped Vision output may stream on debug stderr. Errors and unobserved completion remain explicit. Credential/opaque/image redaction and compile-time release exclusion remain unchanged. No logs or protocol parser are added.

Tool presentation consumes existing execution plans and outcomes. Show bounded arguments/results with explicit preview truncation, retaining failed/cancelled/deduplicated distinctions. A completed label follows successful durable Receipt recording. Display never reruns Tools, reads files, changes outcomes, or becomes evidence. One private terminal-content formatter owns readable whitespace, control escaping and preview limits for both displays.

ContextPlanner remains the single default-budget owner. Output reserve is min(window / 8, 8192); tool-loop reserve is min(window / 8, 16384). The budget-policy identity changes to invalidate derived cache/baseline compatibility. Small windows up to 65536 retain their previous allocation. At 1048576 tokens, input allowance is 1024000 rather than 786432. These are planner headroom estimates, not a Provider output-token limit or token-accuracy guarantee. Existing explicit CapabilityMapping context_window remains mandatory; model capacity is never inferred. No silent per-session working-window cap or automatic compaction is introduced.

Offline tests cover readable/escaped bounded results, mixed outcomes, real wire content versus debug projection, streamed output/omissions, release exclusion, large and small budgets, and policy invalidation. Existing reasoning continuation, History pairing, exact-target Compaction and credential isolation invariants remain unchanged. Git, Live and independent review retain separate authorization boundaries.
