# Configuration examples

Templates and ordinary development tasks start at **272000 tokens**. For a
model explicitly supporting **1M**, use its declared window or import it with
`auth select-model --apply-capabilities`. A model ID alone does not prove its
capacity. Small-window boundary fixtures do not define the product defaults.

[简体中文说明](../README.md#2-配置模型) · [English guide](../README.en.md#2-configure-a-model)

Copy one complete Provider template outside the repository. Replace placeholder
URLs, models and private paths; export only the dedicated keys it references.
Each file is self-contained. Do not concatenate templates: TOML tables are unique.

| Setup | Template | Credential / image path |
| --- | --- | --- |
| Text and tools | [provider.toml](provider.toml) | API key reference; no images |
| One multimodal model | [provider-multimodal.toml](provider-multimodal.toml) | Primary handles images directly |
| Text plus separate vision | [provider-text-vision.toml](provider-text-vision.toml) | Independent primary/vision keys and capabilities |
| HTTP or Unix stdio MCP | [provider-mcp.toml](provider-mcp.toml) | Dedicated HTTP Bearer reference or explicit stdio process |
| Codex-compatible subscription | [provider-codex-subscription.toml](provider-codex-subscription.toml) | Echidna-owned OAuth store; Codex route |
| ChatGPT subscription preview | [provider-chatgpt-subscription.toml](provider-chatgpt-subscription.toml) | Separate preview route and private store |

```sh
mkdir -p "$HOME/.config/echidna"
cp -n examples/provider.toml "$HOME/.config/echidna/provider.toml"
# Edit this private copy before starting chat.
echidna --config "$HOME/.config/echidna/provider.toml" doctor
```

`doctor` parses configuration offline without reading environment keys, sending
a model request, or connecting to MCP. A ready report does not verify the service
or credentials. Only run `auth` network operations and `chat` when intended.

## Provider and capability fields

`schema_version = 1` remains current. `active_agent_profile` selects a model
profile; optional top-level `agent_definition` selects an exported Agent directory,
independently of that model profile. Relative Agent and prompt file paths resolve
from the configuration file. Export a template with `echidna agent init` before
uncommenting `agent_definition = "./my-agent"`.

Each Provider uses `binding = "http_sse"`, `wire_api = "responses"`, an explicit
`base_url`, `model`, `capability_mapping` and `generation`. Configure exactly one
of `credential_ref`, inline `api_key`, `codex_subscription` or `chatgpt_subscription`.
Real keys belong only in the private copy. Primary/vision key references are
separate `CYBER_AGENT_*_API_KEY` names.

`connect_timeout_ms = 30000` and `read_timeout_ms = 300000` are the current
defaults. The latter bounds stalled reads, rather than the whole task duration.
Successful streaming reads reset it. These are independent of MCP timeouts.

Capability mappings declare text/image input, streaming, tool calls, structured
output, incremental input, usage reporting and `context_window`. The supplied
272000 window is a placeholder, not a discovery result or a long-window ceiling.
Optional `reasoning_effort` is a wire setting, not added prompt text. Optional
`max_output_tokens` must be supported by the service and fit its declared window;
neither subscription example supports that field on its route.

Model selection and optional capability import use the existing commands:

```sh
echidna --config /absolute/path/codex.toml auth models --provider codex
echidna --config /absolute/path/codex.toml auth select-model --provider codex --apply-capabilities
```

Use the Provider ID from your copy. API-key Providers support the same discovery
and selection commands; login/refresh/status/logout are subscription-only.
Only metadata actually advertised by the service can be imported. A model ID
alone does not change local capacity. Unknown or absent window metadata requires
an explicit value; no model-name inference or automatic capability fallback occurs.

The Codex template explicitly declares catalog compatibility `0.162.0`, used in
the recorded new-ID comparison. Omitting it retains runtime default `0.144.0`;
neither value guarantees all account models or tracks the newest Codex release.
`supports_reasoning_summary_parameter` defaults true; disable it only if the
route rejects that parameter. Returned summaries are not complete internal reasoning.

## Runtime and context budgets

All six templates show the same current budget defaults. `auto_compact` defaults
false in the runtime; the Codex template explicitly opts in. Other templates show
false so enabling it is deliberate. `model_request_limit_per_turn = 32` is shared
by Primary, Vision, RSI, compaction and model retries; use explicit `0` for unlimited
admission. It is not a context window or Provider usage count.

Optional `compaction_reasoning_effort = "high"` selects only the same primary
model's isolated compaction scope. Omission inherits primary. The configured
service must support that value; there is no silent downgrade.

Optional `task_selection_reasoning_effort = "low"` under `[runtime]` selects only
the same primary model's isolated task-context selection scope. Omission inherits
primary. Main-task and compaction settings are unchanged; the service must support
the selected value, with no automatic downgrade.

Under `runtime.context_budget`, `_bps` uses **100 = 1%, 10000 = 100%**:

| Fields | Denominator / purpose |
| --- | --- |
| `output_reserve_bps`, `tool_loop_reserve_bps` | Declared model window; both default to 1250 (12.5%). Explicit output requirements can increase the first. |
| `pressure_bps`, `target_bps` | Usable input B after reserves; 8000 trigger / 5000 soft compaction target. |
| `summary_bps`, `diagnostic_bps` | Independent content-byte allowances derived from B; both default to 200. |
| `diagnostic_task_bps`, `diagnostic_handoff_bps` | Shares of the diagnostic pool, both 313; leave room for observations. |
| `tool_output_bps`, `core_memory_bps` | Per-tool preview and Core-write allowances derived from B, 3200 / 1600. |
| `state_section_bps`, `continuation_field_bps`, `continuation_goal_part_bps` | B-derived state/navigation allowances, 10 / 40 / 16. |
| `readback_default_bps`, `readback_max_bps` | B-derived raw readback allowances, 100 / 400; encoded results still consume request space. |
| `compaction_reference_items` | Fixed count per reference kind; default 64 items. |
| `max_core_file_bytes` | Core storage limit shared by loader/write/inspection; default 524288 bytes. |
| `max_content_bytes` | Optional absolute ceiling on derived content allowances, absent by default. |

These limits have different purposes and do not sum to 100%. Byte conversion uses
the current local estimator, not exact server tokens. Tool schemas/instructions
also consume B. Reference/protection metadata and complete requests still undergo
their own validation; a small configured limit cannot authorize dropping required
inputs. A percentage is an allowance, not data automatically inserted into requests.

Automatic compaction evaluates fully durable tool batches and normal completion.
It protects exact unfinished inputs/images and the latest complete batch. A source
may require finite segment and merge requests, each consuming the original budget.
Tool-free sampling releases only the tool-loop reserve. Failed sampling retains
the parent continuation and suppresses further automatic sampling that Turn.
Valid oversized recovery permits `/context`, `/compact` and `/exit` after explicit
resume; it does not relax corrupt-history or unknown-execution failures.

## MCP and prompt files

Each MCP server selects exactly one HTTP(S) `url` or stdio `command`. HTTP supports
protocol `2025-11-25` or `2026-07-28`; Unix stdio uses `2025-11-25`. `timeout_ms`
bounds a tool call; optional `startup_timeout_ms` independently bounds startup,
initialize and discovery and otherwise inherits `timeout_ms`. The template shows
60000 for each. HTTP `credential_ref` is dedicated to that server; stdio uses
literal `args`, optional `cwd` and explicit `env`, without shell expansion.

Optional stable guidance paths can be added as a separate table in the private copy:

```toml
[prompts]
personal_file = "personal.md"
project_file = "project.md"
```

Create these files relative to the config first. Existing Sessions retain their
instruction snapshots; edit/reload and Resume are explicit operations. Durable
rules belong in these files rather than relying on a lossy session summary.

## Agent definitions

| Agent template | Purpose |
| --- | --- |
| [Research](agents/research/agent.toml) | Research identity/phases with task-scoped technical domain guidance |
| [Technical writing](agents/technical-writing/agent.toml) | Generic task definition and Agent-scoped memory |
| [Self-improving](agents/self-improving/agent.toml) | Explicit persona/method RSI and bounded improvement, with operator-owned fixed rules |

Use `echidna agent init ./my-agent --id my-agent --from technical-writing` or
`--from builtin-research` to export supported templates. For the self-improving
example, copy its complete directory with its Markdown resources. Offline
`agent check` validates resources and known tool names. Add `--discover-mcp` to
the supported inspect/lineage commands only when actual directory discovery is
intended. RSI is not enabled by changing model or context budgets.
