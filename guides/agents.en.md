# Agent definitions

[Home](../README.en.md) · [简体中文](agents.md)

[Memory and scope](#memory-and-scope) · [Switching, reload and Resume](#switching-reload-and-resume) · [Optional self-improvement](#optional-self-improvement)

---

`codex/agent-definition-v1` externalizes persona, work instructions, and methods into local
files while retaining one execution loop. This branch is maintained separately and will
not merge into main. Research remains the default; the
[`technical-writing` example](../examples/agents/technical-writing) provides a generic agent.

```sh
echidna agent init ./my-agent --id my-writer --from technical-writing
# Edit my-agent/persona.md, work.md, and the Markdown files under methods/.
echidna --config /path/to/provider.toml agent check ./my-agent
echidna --config /path/to/provider.toml agent inspect ./my-agent
# Opt in to one configured MCP discovery when authoring exact remote Tool names.
echidna --config /path/to/provider.toml agent inspect ./my-agent --discover-mcp
echidna --config /path/to/provider.toml chat --agent ./my-agent
```

Use `--from builtin-research` to export the research template. `check` validates resources,
known tools, and static context budgets offline; runtime admission also checks task history
and current facts. `inspect` shows resolved instructions and tools, including your selected
personal/project instruction content.

`check` and default `inspect` stay offline: they do not contact an MCP server or resolve
an MCP credential. To author an external Tool declaration, run `agent inspect --discover-mcp`
against a definition that omits the remote Tool. It discovers the configured MCP catalog once,
prints `discovered_mcp_tools` even though they are currently hidden, and then performs the
usual static binding and budget validation without a model request or persisted catalog. Copy
the exact generated local name into `[tools]`, for example
`mcp__fixture__0123456789abcdef0123 = "deferred"`; names cannot be wildcarded or defaulted.

The version-1 `agent.toml` contains:

| Setting | Meaning |
| --- | --- |
| `id` / `name` | Stable identity and display name. Content edits do not change memory identity. |
| `instructions` | Relative `persona_file`, `work_file`, and ordered `method_files`. |
| `context.personal` / `project` | `configured` snapshots the files selected in Provider config; `none` excludes them. |
| `context.memory` | `none`, `project`, or `agent`; Agent memory is scoped by project and stable ID. |
| `tools` | Known built-in or exact discovered MCP names mapped to `direct`, `deferred`, or `hidden`; omitted names are Hidden, except Session-bound `context_read` defaults to Direct in Chat. Explicit declarations take precedence. Deferred tools require direct `tool_search`. |
| Optional `research` | Existing domain/phase selections and complete external prompt maps. Omission means no Research state or default Research events. |

For research definitions, Direct/Deferred are baseline exposure and existing Domain/Phase
rules may adjust them; Hidden stays terminal. Tool visibility controls context, not permission.
Generic image, HTTP, and artifact tools bind materials to the current Session without a fake Target.

## Memory and scope

The runtime supplies the current Agent ID and Memory scope to the model.
Logical Memory paths resolve within that scope. Missing documents return null
content/hash; create with expected_hash:null and update with the current hash
returned by memory_read. Version 2 of memory_read, memory_write and inspect_project
uses runtime operation timestamps and rejects caller occurred_at/observed_at
arguments. Historical sessions and source timestamps remain unchanged.

Use `/memory` in chat to inspect the actual directory, scope, `core.md` presence
and configured memory tools. Project memory lives in `.echidna/memory/`; Agent
memory lives in `.echidna/memory/agents/<agent-id>/`. Legacy projects keep their
existing `.cyber-agent/` layout. The first successful `memory_write` creates a
document; an ordinary reply or compaction does not save memory. Enabled Agents
receive built-in guidance to preserve reusable lessons and explicit remember
requests through the memory tools. `core.md` is selected by task, source scope
and revision, retaining established global constraints; detailed notes are read
on demand. Context selection does not rewrite Memory files or trim the bound
persona, work and method instructions. There is no background model extraction
task or guarantee of a memory write on every turn.
Current fixed guidance requests English narrative in new memory writes, preserving
literal quotations, code, paths and identifiers. Existing documents are not translated.

## Switching, reload and Resume

Inside chat, `/agent` displays identity and digest. `/agent reload [directory or agent.toml]`
reloads instructions for the same identity, keeps History, and rebuilds the Provider session.
`/agent use <path>` opens a new Session and leaves the old one resumable. Changes require
paired calls, no running/stopping Job or PTY, and no queued image. `/image cancel` detaches
a queued image while preserving its durable artifact.

Sessions pin complete instruction content. Editing or deleting source files does not change
an existing Session; Resume uses its saved content and reload explicitly reads new content.
Reload cannot change the ID, memory scope, or research contract; use a new Session for those
changes. Neither command automatically samples a model, continues a task, or replays tools.
New Sessions select `chat --agent`, then top-level config `agent_definition`, then builtin
research. Config paths resolve relative to that config file. Model and credential selection
remain independent. Older binaries cannot read the new definition events.

## Optional self-improvement

The [self-improving Agent](../examples/agents/self-improving) explicitly enables
`[rsi] persona = true`, `methods = true`, and `improvement = true`, with an
operator-owned `fixed_rules_file`. Omitted RSI stays disabled. Persona/method
changes do not rewrite the work scope, fixed rules, memory scope, tools, or model
configuration. Passive RSI applies at a completed Turn boundary.

With `improvement = true`, bounded diagnosis can also run after 16 complete durable
tool batches, using recent actual failures, verified repeated commands or explicit
`[rsi-feedback]`. Diagnosis, old/new contrast and any same-Turn handoff share the
original request budget. A locally improved candidate applies in the same Turn
only after a validated durable handoff; the task, activations and executed results
remain. No signal, insufficient budget, running jobs/PTYs, queued images or
incomplete outcomes prevent that work. Local contrast does not prove general
learning gains. Recovery never replays the probes.
