<p align="center">
  <img src="assets/readme-banner.svg" alt="Echidna — Local Vulnerability Research Agent. Question. Observe. Verify." width="100%">
</p>

<p align="center">
  <a href="README.md">简体中文</a> · <strong>English</strong>
</p>

<p align="center">
  <strong>Research locally. Keep every step traceable.</strong>
</p>

<p align="center">
  <a href="#system-design">System design</a> ·
  <a href="#capabilities">Capabilities</a> ·
  <a href="#quick-start">Quick start</a> ·
  <a href="#everyday-use">Everyday use</a> ·
  <a href="#next-steps">Next steps</a> ·
  <a href="#documentation">Documentation</a>
</p>

---

Echidna is a local-first vulnerability research agent for individual use. You define the
target, scope, and question. It works through file, command-line, and network tools,
keeping sessions, tool execution receipts, and research materials in the project so you
can continue the work, inspect the process, and export records.

Built in Rust with an interactive CLI, Echidna uses one AgentLoop for source code, web,
mobile, and binary research. Domain and phase guidance help organize the research method;
your environment provides the actual analysis tools.

## System design

Echidna couples a single research agent loop with an inspectable local record of its work.
The operator defines the task and scope; the runtime assembles context, requests model
inference, executes validated tool calls, and retains the observations needed to continue
or review the research.

[![Echidna system design: research inputs, a single agent loop with immutable sampling plans, and durable local records.](assets/architecture-overview.svg)](assets/architecture-overview.svg)

*Figure 1. Echidna's execution and persistence model. (a) Operator input, research guidance,
and project context enter through distinct sources. (b) One AgentLoop binds model inference
and tool dispatch to an immutable sampling plan. (c) Fixed persistence stages retain session
events, request audits, original artifacts, and project records. The figure abstracts
individual failure paths and optional image routing; it does not depict future orchestration.*

**Plan once per sampling step.** Context assembly accounts for source provenance and the
available context budget. A captured `SamplingStepPlan` binds that context to the advertised
tool schemas and explicit model capabilities. The same resolved tool plan validates and
dispatches the model's tool calls, keeping model-visible tools and executable tools aligned.

**Checkpoint before dependent work.** Request auditing precedes inference. Accepted model
samples are checkpointed before dependent tool execution; raw process captures are published
before execution receipts, and paired tool results are checkpointed before the next model
step. Tool execution uses resource-aware scheduling inside the single agent loop.

**Retain observations; derive context.** Original artifacts and append-only events preserve
what was observed. Memory, summaries, caches, and image observations retain their own roles
and provenance; none automatically establishes a verified finding. Explicit session recovery
validates durable history before continuing, while unknown execution effects remain explicit.

This separation lets model capabilities and research methods evolve while execution,
request budgets, and recovery remain locally controlled. When configured, a secondary vision
provider follows a scoped auxiliary path within the same agent lifecycle. The runtime keeps
one agent loop and one session authority.

## Capabilities

| | What it does |
| --- | --- |
| **Analyze and execute** | Search and read code, edit files, apply patches, run commands, send HTTP requests, and use background jobs and PTYs. |
| **Continue research** | Retain project context and sessions, explicitly resume an existing session, and compact context for longer tasks when prerequisites are met. |
| **Intervene as you work** | Stop the current turn, add instructions or change direction in the terminal, and inspect actual tool progress and output. |
| **Inspect the record** | Review tool execution receipts, original material references, and request audits; export sessions as Markdown or JSON. |
| **Build on prior methods** | Use project Markdown memory and explicit personal/project instruction files to retain constraints, decisions, and unsuccessful approaches. |
| **Work with images** | Use the primary model or an optional vision model according to explicit configuration, retaining image sources and their links to observations. |

**Local-first** means project data and tool execution stay local. When you configure a remote
model, materials included in its requests are sent to that provider. Image support depends
on capability configuration; model observations and answers still require verification
against actual evidence.

## Long-context capacity

The explicit capability mapping's `context_window` determines local capacity;
model IDs do not imply capabilities. By default, output and tool-loop reserves
each use 12.5% of the declared window W. A larger explicit output requirement
increases the output reserve. The remaining input allowance B must also fit
instructions, tool schemas, history and provider continuation.

| Declared W | Usable input B | Pressure / soft target | Narrative allowance (bytes) |
| --- | --- | --- | --- |
| 272000 | 204000 | 163200 / 102000 | 16320 |
| 1000000 | 750000 | 600000 / 375000 | 60000 |
| 4000000 | 3000000 | 2400000 / 1500000 | 240000 |

These use default ratios without a larger output requirement. They are local
estimates, not exact provider tokens or verified server capacities. The old
64K/32K thresholds and eight-step content multiplier no longer apply. Tool
previews, Core context, narrative, state and readback use separate configurable
shares. Allowances do not prefill requests; a per-tool preview is not a combined
batch allowance. Core storage has a separately configurable 512KiB default cap
used by file read/write validation; context allocation is a distinct planner limit.
`core_memory_bps` limits automatic Core context. Explicit `memory_read` returns
the complete stored document and its real hash; a large read can still exceed
a small model's hard capacity. Storage capacity does not guarantee request fit.
Raw capture, MCP and transport bounds remain independent.

New main v5 handoffs budget Narrative JSON separately from protected exact inputs,
images and source references. The complete model projection still passes the
hard input budget. Historical summary units remain unchanged. Smaller windows
do not rewrite saved facts or silently remove required context.

## Quick start

The current setup primarily targets macOS. Install Git and Rust/rustup; the repository pins
Rust 1.93.1 through [`rust-toolchain.toml`](rust-toolchain.toml).
Install external analysis tools as needed for your tasks.

### 1. Build

```sh
git clone https://github.com/F0und-icu/echidna-agent.git
cd echidna-agent
cargo build --locked --release

ECHIDNA_BIN="$PWD/target/release/echidna"
"$ECHIDNA_BIN" --version
```

Run the remaining commands in the same terminal, continuing to use `ECHIDNA_BIN`.

### 2. Configure a model

Templates, ordinary development and acceptance tasks default to at least
**272,000 context tokens**. Use the service's declared **1M** window when the
model supports it, through explicit capability mapping or catalog import.

Choose a complete template for your setup and save it outside the source repository:

See the [configuration index](examples/README.md) for all six Provider templates,
current fields, units and optional settings.

| Setup | Complete template | Image handling |
| --- | --- | --- |
| Text-only model | [provider.toml](examples/provider.toml) | `disabled`: no image input |
| One multimodal model | [provider-multimodal.toml](examples/provider-multimodal.toml) | `primary_direct`: the primary model receives images directly |
| Text primary + separate vision model | [provider-text-vision.toml](examples/provider-text-vision.toml) | `secondary_tool`: the vision model analyzes images; the primary continues with its observations |
| Text model + remote MCP tools | [provider-mcp.toml](examples/provider-mcp.toml) | Deferred remote tools discovered when `chat` starts |

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

#### Environment variables and keys

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

#### Optional remote HTTP MCP tools

Use the [MCP template](examples/provider-mcp.toml) to declare one or more remote
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

#### Local stdio MCP tools

The same [MCP template](examples/provider-mcp.toml) supports Unix stdio servers.
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

#### ChatGPT subscription sign-in

Use the [subscription template](examples/provider-chatgpt-subscription.toml) and save your
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

#### Codex-compatible subscription sign-in

Use the [Codex-compatible template](examples/provider-codex-subscription.toml) with
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

#### One multimodal model

Use the [multimodal template](examples/provider-multimodal.toml) for one primary model that
handles text, tool calls, and images. Its `[capability_mappings.primary-v1]` declares
`image_input = true`, with the following profile:

```toml
[agent_profiles.default]
primary = "primary"
vision_mode = "primary_direct"
generation = 1
```

Only the primary key is needed; no additional vision provider is required.

#### Separate text and vision models

Use the [two-model template](examples/provider-text-vision.toml). Set `base_url`, `model`,
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
use W and ordinary content shares use B. Missing fields use defaults. Byte allowances follow the current
estimator, and complete serialized inputs still undergo hard-budget checks.

```toml
[runtime.context_budget]
output_reserve_bps = 1250
tool_loop_reserve_bps = 1250
pressure_bps = 8000
target_bps = 5000
summary_bps = 200
tool_output_bps = 3200
core_memory_bps = 1600
state_section_bps = 10          # Research state sections: 0.1% of usable input.
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

In Chat, use `/image PATH` to select a local image. The research mainline requires a real current TargetSnapshot and rejects image intake without one. You can repeat the command: images queue in input order and the next ordinary task submits the whole queue as that Turn's initial context. With `primary_direct`, each image reaches the primary; with `secondary_tool`, each image gets its own retained observation before the primary request. If a later image fails, is cancelled, or reaches the shared request limit, earlier durable image facts remain, but the task and queued images are not submitted to the primary. The queue exists only in the current Chat process; restart and `/resume` do not replay image processing.

### 3. Create a research project

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

## Everyday use

List prior sessions or continue the latest session in a new process:

```sh
"$ECHIDNA_BIN" session list
"$ECHIDNA_BIN" --config "$HOME/.config/echidna/provider.toml" chat --resume latest
```

To select a research domain, add `--domain` after `chat` with one of
`source|web|mobile|binary`, for example `chat --domain binary`.
Domain guidance does not install analysis tools automatically.

| Interaction | Behavior |
| --- | --- |
| `/stop` or Ctrl+C | Request that the current turn stop. Completed operations are not undone; background jobs need separate handling. |
| `/paste` | Follow the prompt to enter a multiline task or additional material. |
| `/image PATH` | Append one local image to the next Turn's queue; repeated commands preserve order. |
| `/memory` | Inspect the actual project memory directory, Core status and configured memory tools without a model request. |
| `/compact` | Ask the configured primary model for a tool-free handoff, then validate and compact context using a command-scoped request budget; unavailable while images are queued. |
| `/resume` | Explicitly rebuild the model session after a recoverable stop, then enter a task. |
| `/exit` | Leave the interactive interface. |

Project memory lives in `.echidna/memory/`; legacy projects keep their existing
`.cyber-agent/memory/`. An absent `core.md` means it has not been written.
Ordinary replies and compaction do not save memory: the model must call
`memory_write`, using the current hash returned by `memory_read` for updates.
Detail notes require explicit reads.

Automatic compaction remains opt-in with `[runtime] auto_compact = true`. After
a complete durable tool batch or completed Turn, fresh current context and
accepted continuation determine pressure. Defaults are 80% of B for evaluation
and a 50% soft target; required inputs, images and the latest complete tool batch
take priority over that target. A useful reduction that fits the hard budget can
still be adopted. A fitting source uses one handoff; larger sources use a finite
preplanned segment/merge graph. Every physical attempt, including segments,
merges and transport retries, shares the task's original request budget.
Only a complete validated root permits durable replacement. Invalid, oversized,
cancelled or failed handoffs visibly defer replacement and retain the original
context and provider continuation. Summaries remain lossy, derived material;
offline fixtures do not establish real-task quality gains.
Machine protection references remain in durable facts and the provenance sidecar;
exact unfinished inputs reach the model as separate context. Provenance, narrative
and the Research spine remain validated without inventing completion.
On Resume after append-only Research updates, references refresh offline while
the previous derived handoff is retained; this refresh sends no model request.

A valid oversized Session can explicitly resume into a command-capable recovery
state for `/context`, `/compact` or `/exit`. Required content that still cannot fit
fails explicitly. Corrupt facts, missing outcomes and unknown execution effects
remain integrity failures. A failed automatic compaction sample suppresses further
automatic sampling that Turn, but local capacity evaluation continues.

Optionally set `[runtime].compaction_reasoning_effort = "high"` for the isolated
compaction request. Omit it to inherit the primary setting. Compaction uses the
same model, endpoint and credentials; primary and vision requests are unchanged.
The server must support the selected effort; a refusal is shown without downgrade.

Connection failures and transport interruptions before a complete model response receive
at most two retries, after 500ms and 1000ms. `/stop` and Ctrl+C can cancel the wait.
Each attempt consumes the existing request budget and records a separate request audit.
The request content, credentials, and continuation state stay fixed; each retry generates
the current response again from the start. Displayed fragments from an abandoned response
are explicitly marked as discarded. Only a complete successful sample enters history or
dispatches tools, and completed tools are never replayed. HTTP refusals, protocol errors,
and credential errors are not retried. After exhaustion, the checkpoint is retained;
follow the stop message to use `/resume` and enter a new task. Malformed Tool JSON is rejected before the whole sample dispatches. At most two
regenerations are allowed per consecutive rejection chain; only a fully accepted,
durably committed sample resets it. Cancellation/transport failure does not reset
the chain and completed tools are never replayed.

Export a session while preserving references to its process and materials:

```sh
# Replace SESSION_ID with an actual ID from session list.
"$ECHIDNA_BIN" session export SESSION_ID --format markdown --output session.md
```

Terminal replies start with **`Echidna>`**. `NO_COLOR=1` disables theme colors.
Piped output does not add the branded welcome or theme ANSI sequences. All builds
support explicit `chat --debug` to inspect actual request content and failure details.
Ordinary runs do not emit additional debug content.

All builds support explicit snapshots of failed requests without content redaction:

```sh
"$ECHIDNA_BIN" --config "$HOME/.config/echidna/provider.toml" chat --debug-failures ../debug-failures
```

The directory is relative to chat's actual working directory.
The feature is off by default. Successful and cancelled attempts save
nothing; each failed physical attempt gets a separate file, including failures before
a successful retry. Each file is created separately without overwriting.
The terminal reports save success or failure while preserving the original request
error and retry behavior. Snapshots retain the complete encoded request body,
endpoint and failure details without replacing fields, account values, opaque
content, image data or URL parameters. These are local diagnostics, not TLS
packet captures, RequestAudit or automatic replay inputs. Authentication headers
are not additionally stored. Returned text, reasoning summaries, Tool arguments
and server failure details are not masked or hidden as sensitive content.

## Design direction

**Models propose methods. Tools execute them. Original records preserve the basis for review.**

Echidna keeps execution, budgets, sessions, and recovery in one core. Models, research
guidance, and methods can continue to evolve; original materials and execution records
provide a basis for later review. The project stays small and explicit so one person can
understand and maintain it.

The current entrypoint is an interactive CLI. A TUI, a machine-facing worker interface,
external task queues, and a standalone core SDK remain at the design stage.
Network recovery is limited to the bounded retries of incomplete model requests above;
tool execution retains its existing outcome and recovery rules.

## Next steps

| Direction | Aim |
| --- | --- |
| Reliability and interaction | Improve interruption diagnostics, recovery guidance, and visibility into sessions and background jobs. |
| Reusable research | Make evidence and results easier to locate and hand off, and distill methods from repeated practice. |
| External parallelism | First validate the benefits of separate directories and processes, then design an explicit machine interface and coordinator. |

These directions remain at the planning stage; implementation order will follow feedback
from actual use. External orchestration evolves independently of the single-agent core.

## Documentation

| Resource | Contents |
| --- | --- |
| [Configuration index](examples/README.md) | All Provider templates, proportional and fixed limits, and compaction effort |
| [Text-only configuration](examples/provider.toml) | Primary model, capability declarations, and environment key reference |
| [Single multimodal model](examples/provider-multimodal.toml) | One primary model for text and images |
| [Text + vision configuration](examples/provider-text-vision.toml) | Independent services, capabilities, and keys for the two models |
| [MCP configuration](examples/provider-mcp.toml) | HTTP and Unix stdio tools, dedicated credentials and independent startup deadlines |
| [Codex-compatible subscription](examples/provider-codex-subscription.toml) | Own OAuth store, catalog version, capability import and summary support |
| [ChatGPT subscription preview](examples/provider-chatgpt-subscription.toml) | Separate HTTP subscription preview and private authorization store |
| [Original mechanism figure](assets/architecture-overview.svg) | Execution and persistence flow in scalable SVG form |
| [Changelog](CHANGELOG.md) | Feature changes and compatibility notes |

<details>
<summary><strong>Development and verification</strong></summary>

Default verification uses offline Mock / Fixture / Replay providers and local loopback:

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
cargo build --locked
```

Local distribution for macOS arm64:

```sh
./scripts/package-macos-arm64.sh
./tests/distribution_package.sh dist
```

Live tests are separate and must be explicitly enabled; repository Actions are currently disabled.
Passing engineering checks does not establish that the model can correctly complete every
research task.

</details>
