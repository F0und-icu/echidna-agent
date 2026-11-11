# Everyday use and long context

[Home](../README.en.md) · [简体中文](runtime.md)

[Interaction and Sessions](#interaction-and-sessions) · [Compaction and request budgets](#compaction-and-request-budgets) · [Model retries](#model-retries) · [Exports and diagnostics](#exports-and-diagnostics) · [Long-context capacity](#long-context-capacity)

---

## Interaction and Sessions

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
| `/compact` | Ask the configured primary model for a tool-free handoff, then validate and compact context using a command-scoped request budget; unavailable while images are queued. |
| `/resume` | Explicitly rebuild the model session after a recoverable stop, then enter a task. |
| `/exit` | Leave the interactive interface. |

Interrupting `run_command` / `run_commands` terminates and reaps the directly owned
local child, preserves captured output and child outcomes, and stops later launches.
Chat then returns to input or handles queued instructions. This does not undo effects
or imply that remote, background or descendant processes have stopped. Unknown outcomes
and persistence failures remain explicit errors; commands are not automatically replayed.

## Compaction and request budgets

Automatic compaction remains opt-in with `[runtime] auto_compact = true`. After
a complete durable tool batch or completed Turn, fresh current context and
accepted continuation determine pressure. Defaults are 80% of B for evaluation
and a 50% soft target; required inputs, images and the latest complete tool batch
take priority over that target. A useful reduction that fits the hard budget can
still be adopted. A fitting source uses one handoff; larger sources use a finite
preplanned segment/merge graph. Every physical attempt, including segments,
merges, transport retries and size regeneration, shares the task's original request budget.
An isolated summary exceeding its UTF-8 byte limit may regenerate once using the
same frozen request. Transport and size attempts together stay capped at three
per node. Repeated oversize reports actual and maximum bytes; exhausted admissions
report the request-limit error. Neither clips JSON nor replays tools.
Only a complete validated root permits durable replacement. Invalid, persistently oversized,
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

Optionally set `[runtime].task_selection_reasoning_effort = "low"` for isolated
task-context selection using the same primary model. Omission inherits primary;
main-task and compaction effort remain unchanged. The service must support the
selected value. Completed selector responses retain reported usage even when
their decisions fail validation; absent usage stays unknown.

## Model retries

Connection failures and transport interruptions before a complete model response receive
at most two retries, after 500ms and 1000ms. `/stop` and Ctrl+C can cancel the wait.
Each attempt consumes the existing request budget and records a separate request audit.
The request content, credentials, and continuation state stay fixed; each retry generates
the current response again from the start. Displayed fragments from an abandoned response
are explicitly marked as discarded. Only a complete successful sample enters history or
dispatches tools, and completed tools are never replayed. HTTP refusals, protocol errors,
and credential errors are not retried. After exhaustion, the checkpoint is retained;
follow the stop message to use `/resume` and enter a new task. RSI diagnosis and contrast
samples share that same budget; retry does not reopen an arm, replay a completed probe
command, or adopt a failed candidate.
Opted-in improvement also checks fresh friction at most once per16 settled durable
tool batches, with no active jobs, pending images, cancellation or unknown execution.
Its bounded source package prioritizes the latest eight signals. Accepted local
gains can apply in the same Turn after a successful tool-free handoff, complete-batch
preflight and durable definition checkpoint; task, budget and tool activations remain.
One improvement attempt has diagnosis, old, and new samples; same-Turn application
also needs a handoff sample. Each may make up to three physical attempts, for at most12 actual requests;
the original shared Turn budget can stop the attempt earlier.
New RSI sessions with multiple request audits for one scope must be resumed with
this version; older persona binaries reject those sessions.
Malformed Tool JSON is rejected before the whole sample dispatches. At most two
regenerations are allowed per consecutive rejection chain; only a fully accepted,
durably committed sample resets it. Cancellation/transport failure does not reset
the chain and completed tools are never replayed.

## Exports and diagnostics

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
previews, Core writes, narrative, state and readback use separate configurable
shares. Allowances do not prefill requests; a per-tool preview is not a combined
batch allowance. Core storage has a separately configurable 512KiB default cap.
Raw capture, MCP and transport bounds remain independent.

New v6 handoffs budget Narrative JSON separately from protected exact inputs,
images and source references. The complete model projection still passes the
hard input budget. Historical summary units remain unchanged. Smaller windows
do not rewrite saved facts or silently remove required context.
