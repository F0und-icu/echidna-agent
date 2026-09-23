# ADR-0031 — Explicit instruction retention, attribution and output budgets

- Status: ACCEPTED DESIGN for the operator-authorized R7-F1 repair; implementation
  is independently reviewed and locally Gate/Lean verified. No remote seal or Live acceptance.
- Date: 2026-09-23
- Additive architecture: v1.21 (locally reviewed/verified, not remotely sealed)

R7 lost a historical README's exact command definitions after Compaction. A
lossy narrative cannot guarantee retention of arbitrary discovered constraints.
Reuse the explicit Config v1 `prompts.project_file` path: its Project fragment
becomes Required, delivered completely or rejected by the existing Context
budget owner before sampling. No automatic README discovery/extractor, authority
promotion, Policy, new persistence/schema or duplicate instructions are added.
Operators must explicitly select a concise rules document. This does not promise
retention of arbitrary historical reads or force model obedience. A file edit
continues through the original hash/audit/Baseline invalidation and Resume path.

The Base Contract advances to generation3 with bounded guidance: distinguish
original, repaired and current results; require original observation for baseline
claims; do not invent missing historical values or treat rereads as executions.
Prefer smallest decisive actions and bounded edits; do not override current
operator-requested verification. Summary v2 adds a fixed loss warning in its
existing non-authoritative next-step notes. No copied result becomes evidence.
Historical prompt bytes and Events remain immutable and replayable.

Malformed function arguments remain strict failures. The one decoder reports
category/coordinates/byte length without payload or token-bearing parser errors.
The CLI labels a decoder rejection separately from an unobserved completion;
no network-disconnect inference, persisted response logger, auto repair or retry.
Existing schemas explain write preimages and the actual minimal patch syntax.

## Operator-authorized same-Slice output budget extension

An optional positive `max_output_tokens` in the existing versioned Capability
Mapping explicitly declares a supported conservative per-response output ceiling.
Each Provider already selects its mapping; use different mappings when Primary
and Vision require different ceilings. There is no model-name inference, vendor
branch, second Provider construction path or mutable request-global override.
Omission preserves legacy wire bytes and is NOT a limit inferred from context
size. This declaration is operator supplied, not proof of backend compliance.

The one Domain capability owner resolves the declared ceiling into the existing
ModelRequestRequirements before Context compilation/audit. An explicit smaller
request is allowed; zero, undeclared or above-ceiling requests fail. An Adapter
must reject a missing configured limit before credential access, not inject an
unaudited wire field. Generic direct Smokes explicitly use the same configured
capability value; Chat, Compaction and Vision use the existing ContextPlanner.

ContextPlanner reserves at least the resolved output ceiling alongside its
existing capped tool-loop reserve. Invalid/overcommitted default budgets,
including an explicit cap leaving no input space, fail; no input
truncation/reasoning discard is introduced. Explicit custom budgets cannot reserve
less than the request limit. RequestRequirements and capability serialization
omit None, preserving historical absent-field bytes/hashes. New values participate
in RequestAudit reconstruction and the existing OutputContract/Capability hashes;
changing the request limit invalidates continuation through the original owner.
No new Event or copied usage ledger is needed. Doctor reports role-scoped limits.

Responses encodes the already-resolved neutral limit as `max_output_tokens`.
The [official protocol guide](https://developers.openai.com/api/docs/guides/reasoning#controlling-costs)
defines this as total generated output including reasoning, visible output and
non-visible formatting. It may end before any visible answer. An incomplete
response retains the existing typed stop, atomic sample rejection and explicit
Resume semantics; never auto-retry, auto-increase the cap or lower reasoning.
Other backends' enforcement remains unverified until separately authorized Live.
This is a per-response requested ceiling, NOT a cumulative usage/billing guarantee
or a token-efficiency/factuality improvement claim.
