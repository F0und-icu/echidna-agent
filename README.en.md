<p align="center">
  <img src="docs/design/readme-banner.svg" alt="Echidna — Local Vulnerability Research Agent. Question. Observe. Verify." width="100%">
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

[![Echidna system design: research inputs, a single agent loop with immutable sampling plans, and durable local records.](docs/design/architecture-overview.svg)](docs/design/architecture-overview.svg)

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

Copy the [configuration template](docs/examples/provider.toml) outside the source repository:

```sh
mkdir -p "$HOME/.config/echidna"
cp -n docs/examples/provider.toml "$HOME/.config/echidna/provider.toml"
chmod 600 "$HOME/.config/echidna/provider.toml"
```

Edit the file to set the service URL, model, and credentials. Check the context window and
each declared capability against the service's actual support. The URL, key, and 32K window
in the template are examples; `cp -n` preserves an existing file. The commands pass this
configuration path explicitly; the program does not discover it automatically.

The current integration uses **HTTP/SSE + Responses**. Chat Completions endpoints are not
supported. Choose either a local `api_key` or a `CYBER_AGENT_*_API_KEY` environment-variable
reference for credentials. See [provider configuration](docs/development/DEVELOPMENT_ENVIRONMENT.md#61-应用级配置)
for details.

### 3. Create a research project

```sh
mkdir -p ../echidna-research
cd ../echidna-research

"$ECHIDNA_BIN" init
"$ECHIDNA_BIN" --config "$HOME/.config/echidna/provider.toml" doctor
"$ECHIDNA_BIN" --config "$HOME/.config/echidna/provider.toml" chat
```

`doctor` checks configuration locally. It sends no model requests and does not verify
remote service availability. The configured model is used when you submit a task in `chat`.

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
| `/compact` | Compact context when prerequisites such as a target and research records are present. |
| `/resume` | Explicitly rebuild the model session after a recoverable stop, then enter a task. |
| `/exit` | Leave the interactive interface. |

Export a session while preserving references to its process and materials:

```sh
# Replace SESSION_ID with an actual ID from session list.
"$ECHIDNA_BIN" session export SESSION_ID --format markdown --output session.md
```

Terminal replies start with **`Echidna>`**. `NO_COLOR=1` disables theme colors.
Piped output does not add the branded welcome or theme ANSI sequences. Development builds
also support `chat --debug` to inspect request content; standard release builds omit it.

## Design direction

**Models propose methods. Tools execute them. Original records preserve the basis for review.**

Echidna keeps execution, budgets, sessions, and recovery in one core. Models, research
guidance, and methods can continue to evolve; original materials and execution records
provide a basis for later review. The project stays small and explicit so one person can
understand and maintain it.

The current entrypoint is an interactive CLI. A TUI, a machine-facing worker interface,
external task queues, and a standalone core SDK remain at the design stage.
Network interruption recovery still has [open issues](docs/development/R7_MANUAL_TEST_ISSUES.md);
automatic continuation after arbitrary stream interruptions is not guaranteed.

## Next steps

| Direction | Aim |
| --- | --- |
| Reliability and interaction | Improve interruption diagnostics, recovery guidance, and visibility into sessions and background jobs. |
| Reusable research | Make evidence and results easier to locate and hand off, and distill methods from repeated practice. |
| External parallelism | First validate the benefits of separate directories and processes, then design an explicit machine interface and coordinator. |

These are [design proposals](docs/design/FUTURE_RESEARCH_DESIGN.md); implementation order
will follow feedback from actual use. [External orchestration](docs/design/EXTERNAL_ORCHESTRATION.md)
evolves independently of the single-agent core.

## Documentation

| Resource | Contents |
| --- | --- |
| [Development environment and configuration](docs/development/DEVELOPMENT_ENVIRONMENT.md) | Providers, capability declarations, builds, and local diagnostics |
| [Current development status](docs/development/STATUS.md) | Completed work, verification records, and known limitations |
| [Architecture and documentation index](docs/README.md) | Core design, decision records, and development conventions |
| [Brand and theme](docs/design/ECHIDNA_IDENTITY.md) | Logo, palette, and terminal presentation |
| [Migration record](docs/development/ECHIDNA_MIGRATION.md) | Repository lineage and compatibility boundaries |

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

The full gate also includes the [Lean governance checks](docs/development/LEAN_CODE_GOVERNANCE.md).
Live tests are separate and must be explicitly enabled; repository Actions are currently disabled.
Engineering verification and actual model research performance are recorded separately;
consult STATUS for the scope of each result.

</details>
