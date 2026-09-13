# ADR-0025: Environment Guidance and Compatible Prompt Revision

- Status: ACCEPTED for the operator-authorized R4b repair
- Architecture: additive v1.15; verification is governed by STATUS

R4b reuses the fixed environment probes and command owner. A failed process
launch is `probe_failed`, not proof that a program is missing: ENOENT also
covers missing cwd/interpreter/loader. The v1 output shape and its legacy
`missing` enum remain readable; new probes no longer infer missing from errno.
`available` means the fixed probe succeeded and its version was recognized,
not arbitrary tool usability. Device count is meaningful only when status is
available. No raw diagnostic output is added to this privacy-bounded report.

Tool descriptions direct task-specific identity/path/version checks through
existing commands at the actual host/container location. No fixed probe list
growth, new path resolver, capability cache, automatic preflight or CTF mode.
Kernel/runtime architecture does not establish target or container identity;
OS alone does not establish GNU/BSD/LLVM parameter support. Canonical path
aliases require observation, not hardcoded rewriting of `/tmp`.

Binary Domain and Mapping Phase prompts advance to generation 2. Their
generation-1 resources remain for historical reference. Existing typed
selection validators accept only their explicit generation-1 predecessor
and current generation; all other unknown generations still fail. This
narrowly supersedes exact-current-generation validation for those two axes.
Selection source/reason checks do not change. Historical Events and audits
are never rewritten: mechanical Replay uses recorded requests, while a real
resumed Turn uses current generation-2 resources through PromptComposition.
Existing Full-on-Resume, content hashes, ToolSpec fingerprints and RequestAudit
record the new bytes. No mid-Turn refresh, new selection mode or registry.

Tests must prove old selection readback, rejected unknown generations, current
Prompt/audit identities, schema/fingerprint changes, ambiguous probe failures,
and bounded task-specific GNU/LLVM/BSD observations through existing commands.
Offline evidence proves delivery and deterministic behavior, not model compliance.
No new public API, dependency, fact source, automatic retry or external search
service. Reference fetching remains explicit gap-driven HTTP; tool_search is
internal tool discovery, never Internet search.
