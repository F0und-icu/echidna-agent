# Setup and integrations

[Home](../README.en.md) · [简体中文](setup.md)

Start commands from the repository root in one terminal; keep using `ECHIDNA_BIN` after changing to the research directory.

[1. Build](#1-build) · [2. Configure a model](#2-configure-a-model) · [Environment variables and keys](#environment-variables-and-keys) · [Optional remote HTTP MCP tools](#optional-remote-http-mcp-tools) · [Local stdio MCP tools](#local-stdio-mcp-tools) · [ChatGPT subscription sign-in](#chatgpt-subscription-sign-in) · [Codex-compatible subscription sign-in](#codex-compatible-subscription-sign-in) · [One multimodal model](#one-multimodal-model) · [Separate text and vision models](#separate-text-and-vision-models) · [3. Create a research project](#3-create-a-research-project)

---

The current setup primarily targets macOS. Install Git and Rust/rustup; the repository pins
Rust 1.93.1 through [`rust-toolchain.toml`](../rust-toolchain.toml).
Install external analysis tools as needed for your tasks.

## 1. Build

```sh
git clone --branch main --single-branch https://github.com/F0und-icu/echidna-agent.git
cd echidna-agent
cargo build --locked --release

ECHIDNA_BIN="$PWD/target/release/echidna"
"$ECHIDNA_BIN" --version
```

Run the remaining commands in the same terminal, continuing to use `ECHIDNA_BIN`.

## 2. Configure a model

Templates, ordinary development and acceptance tasks default to at least
**272,000 context tokens**. Use the service's declared **1M** window when the
model supports it, through explicit capability mapping or catalog import.

Choose a complete template for your setup and save it outside the source repository:

See the [configuration index](../examples/README.md) for all six Provider templates,
current fields, units, optional settings and Agent examples.

| Setup | Complete template | Image handling |
| --- | --- | --- |
| Text-only model | [provider.toml](../examples/provider.toml) | `disabled`: no image input |
| One multimodal model | [provider-multimodal.toml](../examples/provider-multimodal.toml) | `primary_direct`: the primary model receives images directly |
| Text primary + separate vision model | [provider-text-vision.toml](../examples/provider-text-vision.toml) | `secondary_tool`: the vision model analyzes images; the primary continues with its observations |
| Text model + remote MCP tools | [provider-mcp.toml](../examples/provider-mcp.toml) | Deferred remote tools discovered when `chat` starts |

The following copies the text-only template. For image support, replace the source file
in the copy command with the appropriate template:

```sh
mkdir -p "$HOME/.config/echidna"
cp -n examples/provider.toml "$HOME/.config/echidna/provider.toml"
chmod 600 "$HOME/.config/echidna/provider.toml"
```

Edit the file to set the service URL and model. Check the context window and
each declared capability against the service's actual support. The URL and model name
are examples; the window defaults to 272K and must match the service's capacity.
`cp -n` preserves an existing file. The commands pass this
configuration path explicitly; the program does not discover it automatically.

The current integration uses **HTTP/SSE + Responses**. Chat Completions endpoints are not
supported. With separate models, both the primary and vision services must support this protocol.

### Environment variables and keys

The templates read keys through `credential_ref`; service URLs, model names, and capability
declarations remain in TOML. Set the variables in the same terminal that will start Echidna:

```sh
export CYBER_AGENT_PRIMARY_API_KEY='replace-with-your-primary-key'
# Set this only when using a separate vision model.
export CYBER_AGENT_VISION_API_KEY='replace-with-your-vision-key'
```

The primary provider uses `credential_ref = "CYBER_AGENT_PRIMARY_API_KEY"`, and the vision
provider uses `credential_ref = "CYBER_AGENT_VISION_API_KEY"`. Each reads its own named variable;
there is no fallback to the other role's key or to `OPENAI_API_KEY`.
Custom names must follow `CYBER_AGENT_*_API_KEY` and contain only uppercase letters, digits,
and underscores.

The program **does not load `.env` files automatically**. If you maintain a trusted env file,
load it in your shell and export its variables before starting. Alternatively, replace
`credential_ref` with `api_key = "…"` in your local TOML; each provider must use exactly one.
Keep real keys and configuration outside the repository and out of Git.

### Optional remote HTTP MCP tools

Use the [MCP template](../examples/provider-mcp.toml) to declare one or more remote
Streamable HTTP tool servers under `mcp_servers`. Each server needs an absolute
HTTP(S) `url` and either `protocol_version = "2025-11-25"` or
`protocol_version = "2026-07-28"`. An optional `timeout_ms` defaults to 60000
and bounds each tool call. Both HTTP and stdio accept `startup_timeout_ms` for
the complete server startup, initialization and catalog discovery; when omitted,
it inherits `timeout_ms`. For a slow startup with short calls, use for example
`timeout_ms = 1000` and `startup_timeout_ms = 10000`. Startup failures identify
the server and phase without skipping or retrying it.

If a server requires Bearer authentication, use only a dedicated reference such
as `CYBER_AGENT_MCP_EXAMPLE_API_KEY`; provider keys and subscription accounts are
never reused. Export it in the terminal that starts Chat:

```sh
export CYBER_AGENT_MCP_EXAMPLE_API_KEY='replace-with-your-MCP-key'
```

`doctor` does not read MCP credentials or contact MCP servers. A Chat startup
resolves each configured MCP credential once, discovers its finite tool catalog
in configured-name order, and keeps that catalog unchanged for the process,
including compaction and `/resume`. Remote tools begin as deferred entries and
become available through the ordinary tool search flow. Their schemas and
arguments are passed by value to the server; MCP output remains transport output
until you verify it. A disconnected or incomplete remote call stops the turn
without inventing a receipt or replaying the call.

### Local stdio MCP tools

The same [MCP template](../examples/provider-mcp.toml) supports Unix stdio servers.
Choose exactly one `url` or `command` per server. Stdio uses protocol `2025-11-25`:

```toml
[mcp_servers.local]
command = "python3"
args = ["/absolute/path/to/server.py", "--transport", "stdio"]
protocol_version = "2025-11-25"
timeout_ms = 60000
startup_timeout_ms = 60000
```

Bare commands use PATH; relative command paths and `cwd` resolve from the config
directory, which is also the default cwd. Arguments and explicit `env` values
are literal, without shell expansion. Only the documented base environment plus
explicit `env` reaches the child; provider keys are not implicitly inherited.
`credential_ref` is HTTP-only. Discovery runs when chat starts, not during doctor.

### ChatGPT subscription sign-in

Use the [subscription template](../examples/provider-chatgpt-subscription.toml) and save your
configuration outside the repository. Set `auth_store` to an absolute private directory without symlinks and with an existing parent
and retain the official API URL. `chatgpt_subscription` is mutually exclusive with
`api_key` and `credential_ref`. Echidna performs its own OAuth authorization; it does not
read existing Codex or browser credentials.

```sh
echidna --config /absolute/path/subscription.toml auth login --provider chatgpt
echidna --config /absolute/path/subscription.toml auth status --provider chatgpt
echidna --config /absolute/path/subscription.toml auth models --provider chatgpt
```

Login opens a browser. Use `--no-browser` to open the printed authorization URL manually.
The browser must reach the loopback callback on the machine running Echidna; using a
different machine requires forwarding the callback port.
`status` checks local state only; `models` queries the server with the selected account.
Put a returned model slug in `model`, verify its capabilities and context window, then use
the same configuration for `doctor` and `chat`. The template defaults to 272K;
configure or import the actual 1M window when supported.
Subscription HTTP requires `incremental_input = false` and does not accept
`max_output_tokens`. Successful login and offline doctor do not prove model eligibility.

```sh
echidna --config /absolute/path/subscription.toml auth logout --provider chatgpt
```

Access tokens refresh on demand before use. Logout attempts revocation and removes local
tokens, reporting whether remote revocation was confirmed. Keep the authentication
directory outside repositories and research projects with owner-only directory/file access.
Account eligibility and usage follow the [official preview limitations](https://developers.openai.com/siwc/token-sharing-open-source/preview-limitations).
Failures do not switch to paid API access or replay tools. Other vendors' non-key
subscriptions are not yet supported.

### Codex-compatible subscription sign-in

Use the [Codex-compatible template](../examples/provider-codex-subscription.toml) with
explicit `codex_subscription` and `https://chatgpt.com/backend-api/codex`.
This choice is exclusive with SIWC and both API-key forms. Use a separate private
`auth_store` for this profile. Existing API-key, Qwen and DeepSeek configurations
need no changes.

To declare a catalog compatibility version, set
`catalog_client_version = "major.minor.patch"` inside `codex_subscription`. Its
default remains `0.144.0`; an explicit value changes only the catalog request's
`client_version` query and `version` header. It does not claim an installed Codex
release or guarantee new model IDs or account access.
The current template explicitly uses `0.162.0`, previously verified in the
catalog comparison, and `supports_reasoning_summary_parameter = true`. This
does not auto-detect a current upstream version or guarantee a complete catalog.

Use the same `auth login/status/models/logout` commands above with the new config
path and its provider ID. Echidna performs its own browser authorization and owns
its token store; it does not read an existing Codex login. The loopback callback
uses port 1455, or 1457 when that port is occupied. If the catalog returns HTTP401
while local status still reports future expiry, explicitly refresh the own grant:

```sh
echidna --config /absolute/path/codex.toml auth refresh --provider codex
```

This Codex-only command refreshes credentials and returns local status. It opens
no browser and repeats no catalog/model request. After success, explicitly run
selection again; a permanently rejected refresh requires a new login. Local
`status` does not prove the backend still accepts the grant. SIWC and API-key
flows remain unchanged.

`auth models` / `auth select-model` also support standard Responses services
configured with an API key or `credential_ref`, including DeepSeek. They query
`/models` under the selected Provider's `base_url`; substitute your config path
and provider ID below. Login, status, refresh and logout remain subscription-only.
Discover models, choose one and its reasoning effort, and save the choice with
the terminal picker:

```sh
echidna --config /absolute/path/codex.toml auth select-model --provider codex
```

When the catalog advertises capabilities, import its actual default window and
input metadata while selecting:

```sh
echidna --config /absolute/path/codex.toml auth select-model --provider codex --apply-capabilities
```

Changing only the model ID does not update `context_window`. Leaving the
template at 272000 keeps local planning at 272K. If the service does not provide an
importable window, configure its documented capacity explicitly.

For noninteractive selection, use a model returned by the catalog and an effort
advertised for that model:

```sh
echidna --config /absolute/path/codex.toml auth select-model --provider codex \
  --model gpt-6-astra --reasoning-effort xhigh
```

Efforts accept valid catalog-advertised strings, including future values; the
picker keeps Codex and advertised API-key levels strict. If an API-key catalog
does not advertise effort support, enter an explicit `--reasoning-effort` or picker
value; Echidna reports unknown support and the server may reject it. Direct configuration
accepts 1–64 ASCII letters, digits, underscores or hyphens. Omitting effort keeps
the server default; explicit `none` is a different request value. Saving clones an isolated
capability mapping for the provider and preserves other providers, comments and
credentials. The configuration lock coordinates Echidna selection commands, so
avoid editing the file externally while a selection is being saved.
Add `--apply-capabilities` to import present default context-window, text/image
input and summary-parameter support metadata for the selected model. Missing
fields or unknown windows (including `null`) preserve existing settings; `max_context_window` remains metadata and
never replaces the default window. Existing configuration/output-budget checks
must pass before publication; failures leave the original file intact. Without
the flag, selection behavior is unchanged. Other declarations such as tool
support still need explicit review; none are inferred from model names.
ID-only API-key catalogs preserve the explicit mapping and reject
`--apply-capabilities` without saving. API-key imports use actual `context_window`
and `input_modalities`; server `max_output_tokens` and other protocol capabilities
do not become output budgets or Responses capabilities. Offline `doctor` shows the
configured effort; a new `chat` uses the saved settings. The protocol
is based on a pinned Codex source version; actual account/server compatibility
requires live verification. Failures do not switch models or paid APIs or replay
tools automatically.

The Codex path requests available reasoning summaries and labels them
`reasoning summary` on stderr in debug and release builds. These are provider summaries,
not full internal reasoning. Empty summaries stay empty. They do not enter
conversation text or replace encrypted continuation. Reasoning effort is
independent of presentation; selecting `high` does not guarantee a summary.
If a server rejects this parameter, explicitly set
`supports_reasoning_summary_parameter = false` inside `codex_subscription`.
Missing or `true` preserves the default request.

### One multimodal model

Use the [multimodal template](../examples/provider-multimodal.toml) for one primary model that
handles text, tool calls, and images. Its `[capability_mappings.primary-v1]` declares
`image_input = true`, with the following profile:

```toml
[agent_profiles.default]
primary = "primary"
vision_mode = "primary_direct"
generation = 1
```

Only the primary key is needed; no additional vision provider is required.

### Separate text and vision models

Use the [two-model template](../examples/provider-text-vision.toml). Set `base_url`, `model`,
`credential_ref`, and `capability_mapping` independently in `[providers.primary]` and
`[providers.vision]`. The models can use different services, each with its own context window
and capabilities. The profile connects them:

```toml
[agent_profiles.default]
primary = "primary"
vision = "vision"
vision_mode = "secondary_tool"
generation = 1
```

The primary model handles reasoning and tool calls; the vision model analyzes images and
returns observations to the primary. They work within one AgentLoop. The primary must
support text input, streaming, and tool calls. The vision model must support text and image
input, streaming, and structured output; it does not need tool-call support.

You can also select `vision_mode = "auto"`: **declared capabilities** determine routing,
preferring the primary model's image support, then the configured vision model. If neither
supports images, image input is rejected. This does not probe model capabilities or provide
automatic failover after a request fails.

Declarations must match the actual service; setting `image_input = true` cannot give a
text-only model image support. Set each model's `context_window` to its actual capacity.
`[runtime].model_request_limit_per_turn` defaults to `32`, a per-turn request limit shared
by the primary and vision models. Set it explicitly to `0` for unlimited admission;
`1..=10000` retains a finite limit. Attempts are still counted, and context capacity,
cancellation, deadlines and the per-request retry limit remain unchanged.

Configure proportional allowances under `[runtime.context_budget]`. Integer
`_bps` fields use basis points: **100 = 1%, 10000 = 100%**. Output/tool reserves
use W; ordinary content shares use B; diagnostic task/handoff excerpts use the
diagnostic pool. Missing fields use defaults. Byte allowances follow the current
estimator, and complete serialized inputs still undergo hard-budget checks.

```toml
[runtime.context_budget]
output_reserve_bps = 1250
tool_loop_reserve_bps = 1250
pressure_bps = 8000
target_bps = 5000
summary_bps = 200
diagnostic_bps = 200
diagnostic_task_bps = 313
diagnostic_handoff_bps = 313
tool_output_bps = 3200
core_memory_bps = 1600
state_section_bps = 10
continuation_field_bps = 40
continuation_goal_part_bps = 16
readback_default_bps = 100
readback_max_bps = 400
compaction_reference_items = 64  # Per reference kind; unit: items.
max_core_file_bytes = 524288     # Core storage cap; unit: bytes.
# max_content_bytes = 1048576   # Optional fixed content cap, absent by default.
```

These are purpose-specific limits, not disjoint shares to sum to 100%. Tool-free
compaction releases only the tool-loop reserve; output/reasoning room remains.

In Chat, use `/image PATH` to select a local image. You can repeat the command: images queue in input order and the next ordinary task submits the whole queue as that Turn's initial context. With `primary_direct`, each image reaches the primary; with `secondary_tool`, each image gets its own retained observation before the primary request. If a later image fails, is cancelled, or reaches the shared request limit, earlier durable image facts remain, but the task and queued images are not submitted to the primary. The queue exists only in the current Chat process; restart and `/resume` do not replay image processing.

## 3. Create a research project

```sh
mkdir -p ../echidna-research
cd ../echidna-research

"$ECHIDNA_BIN" init
"$ECHIDNA_BIN" --config "$HOME/.config/echidna/provider.toml" doctor
"$ECHIDNA_BIN" --config "$HOME/.config/echidna/provider.toml" chat
```

`doctor` checks configuration locally. It does not read environment-variable keys, send model
requests, or verify key validity or remote service availability. The configured model is used
when you submit a task in `chat`.

Start in a separate research directory. Tools execute with your current user permissions;
there is currently no built-in sandbox or execution confirmation layer.
