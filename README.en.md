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
  <a href="#quick-start">Quick start</a> ·
  <a href="#system-design">System design</a> ·
  <a href="#automatic-task-context">Task context</a> ·
  <a href="#guides">Guides</a>
</p>

---

Echidna is a local-first agent for individual use, built in Rust with an interactive
CLI for models, files, commands and network tools. Sessions, execution receipts and
research materials stay in the project so you can continue work and inspect its basis.

The `main` branch supports external persona, work and method
files. Vulnerability research is the default; custom definitions also support tasks
such as technical writing. The previous research mainline is preserved at
`codex/backup-main-2026-11-14`; ongoing development and updates use `main`.

## Capabilities

| Capability | What it provides |
| --- | --- |
| Analysis and execution | File editing, code search, commands, HTTP, background jobs / PTYs and MCP tools. |
| Automatic task context | Ask, follow up or return to earlier work; the harness selects relevant material. |
| Continuing work | Resume Sessions and compact long context while retaining actual results and sources. |
| Editable Agents | External personas, methods, scoped memory and explicitly enabled RSI. |
| Image input | A multimodal primary model or a separately configured vision model. |
| Inspectable work | Execution receipts, request usage, debug output and Session exports. |

Local-first means project data and tools are local; requests sent to a remote model
include their selected material. Tools run with your user permissions, without a built-in
sandbox or execution confirmation layer.

## Quick start

The current setup primarily targets macOS. Install Git and Rust/rustup;
the [toolchain](rust-toolchain.toml) pins Rust 1.93.1. Use the same terminal throughout.

### 1. Build

```sh
git clone --branch main --single-branch https://github.com/F0und-icu/echidna-agent.git
cd echidna-agent
cargo build --locked --release
ECHIDNA_BIN="$PWD/target/release/echidna"
```

### 2. Configure a model

```sh
mkdir -p "$HOME/.config/echidna"
cp -n examples/provider.toml "$HOME/.config/echidna/provider.toml"
chmod 600 "$HOME/.config/echidna/provider.toml"
```

Edit the service URL, model and capabilities, then set the key referenced by the template:

```sh
export CYBER_AGENT_PRIMARY_API_KEY='replace-with-your-primary-key'
```

The integration uses **HTTP/SSE + Responses**; Chat Completions endpoints are unsupported.
The template defaults to a 272K window: match the service's actual capacity, using 1M
when explicitly supported. `.env` files are not loaded automatically. See the
[setup guide](guides/setup.en.md) for multimodal models, MCP and subscription sign-in.

### 3. Start working

```sh
mkdir -p ../echidna-research
cd ../echidna-research
"$ECHIDNA_BIN" init
"$ECHIDNA_BIN" --config "$HOME/.config/echidna/provider.toml" doctor
"$ECHIDNA_BIN" --config "$HOME/.config/echidna/provider.toml" chat
```

`doctor` checks local configuration, not key validity or service availability. Submitting
a task in Chat starts model requests. Use `chat --agent /path/to/agent` for an
[external Agent definition](guides/agents.en.md).

## System design

One AgentLoop owns context, model sampling and tool execution. Each sample fixes its
context and tool plan. Actual results are persisted before dependent work proceeds;
Resume does not rerun old tools.

[![Echidna system architecture](assets/architecture-overview.svg)](assets/architecture-overview.svg)

[Architecture and development checks →](guides/architecture.en.md)

## Automatic task context

**Retrieve candidates locally → classify task relationships → compile context in the harness.**
Complete history is retained. Excluding old task material also establishes fresh model
context. No manual task management is needed; selection adds request overhead and still
depends on the model's judgment.

[![Echidna automatic task context](assets/task-context-flow.svg)](assets/task-context-flow.svg)

[Algorithm, examples and request costs →](guides/task-context.en.md)

## Everyday use

| Action | Entry point |
| --- | --- |
| Resume the latest Session | `echidna --config /path/to/provider.toml chat --resume latest` |
| Inspect requests and diagnostics | Add `--debug` when starting Chat. |
| Stop the current Turn | `/stop` or Ctrl+C; completed effects are not rolled back. |
| Compact context / attach an image / exit | `/compact` · `/image PATH` · `/exit` |

[Full interaction, recovery, compaction and diagnostics guide →](guides/runtime.en.md)

## Guides

| Guide | Contents |
| --- | --- |
| [Setup and integrations](guides/setup.en.md) | Models, keys, subscription auth, MCP, multimodal input and configuration |
| [Agent definitions](guides/agents.en.md) | Personas, methods, memory, reload and optional RSI |
| [Task context](guides/task-context.en.md) | Algorithm, task relationships, source readback and overhead |
| [Everyday use](guides/runtime.en.md) | Resume, interruption, long context, usage and diagnostics |
| [Architecture and development](guides/architecture.en.md) | Execution, persistence, verification commands and planned directions |
| [Configuration templates](examples/README.md) | Provider / Agent templates and field reference |
| [Changelog](CHANGELOG.md) | Feature changes and compatibility notes |
