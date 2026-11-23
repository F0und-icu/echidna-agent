# Chat display protocol v1

A TUI or local Web backend can start Echidna as a child process in an initialized project and read its stdout as UTF-8 JSON Lines.

```sh
echidna --config provider.toml chat --output-format jsonl
```

Every line is one flushed JSON object with `v: 1` and a `type`. Newlines inside strings are JSON escapes. Consumers can ignore unfamiliar event types and fields within v1. Supply ordinary Chat lines and commands on stdin. JSONL uses sequential line input even on a TTY. After `input.mode=task` the child can accept the next line. During processing, a line waits for the next input read. Send SIGINT to request interruption of the running turn. On `input.mode=paste`, finish a multiline input with `/end` on its own line. Process exit ends the stream. Keep stderr separate for command-line errors and process diagnostics, and check the exit code.

| Type | Fields and meaning |
| --- | --- |
| `session` | `session_id`, `disposition` (`started` or `resumed`). A session switch emits a new record. |
| `input` | `mode` is `task`, `processing` or `paste`. |
| `assistant_delta` | Append `text` to the assistant's draft. Preserve embedded whitespace. |
| `sample` | `role`, process-local `request_id` and `state` (`started`, `completed`, `rejected` or `interrupted`). Use the primary sample boundary to mark abandoned draft output. Completion here refers to the Provider stream. |
| `turn` | `state` is `completed`, `cancelled` or `stopped`. Wait for the next input event before soliciting a new task. |
| `tool` | `call_id`, `name`, `stage`, `status`, readable bounded `text`. Stages are `started`, `checkpointed` (persisted child), `recorded` (receipt available) and `unknown`. Status is `succeeded`, `failed` or `cancelled` for observed outcomes, otherwise null. A reused receipt can appear without a fresh start. |
| `previews_skipped` | `count` of tool display frames dropped under backpressure. A missing preview leaves the outcome unknown to the viewer. |
| `usage` | `role`, durable request `audit_id` and `usage` with `input_tokens`, `output_tokens` and optional `cached_input_tokens`, `reasoning_output_tokens`. Each optional count is a subset of its total. |
| `timing` | With `--debug`, `phase`, `stage_ms`, `turn_ms` from a CLI timing checkpoint. |
| `model_log` | `role`, nullable `request_id` and display `text` fragments for Provider diagnostics. `--debug` enables the same request content views as terminal mode. |
| `mcp`, `diagnostic`, `message` | Display `text` for MCP lifecycle, other diagnostics and ordinary CLI messages respectively. |
| `error` | A Chat failure, including startup errors after CLI argument parsing. `text` explains the failure. A fatal exit returns a nonzero code. |

The stream is a live display. Read it continuously. Regular output applies pipe backpressure. Tool previews share a bounded queue and can be skipped while execution proceeds. Use the existing Session export for a durable transcript.

### Minimal consumer

This Python client sends one task, renders assistant text as it arrives and exits when the next task prompt becomes available. Run it from the project directory with `echidna` on PATH and a local `provider.toml`.

```python
import json
import subprocess

with subprocess.Popen(
    ["echidna", "--config", "provider.toml", "chat", "--output-format", "jsonl"],
    stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, encoding="utf-8",
) as chat:
    commands = iter(["Summarize note.txt.", "/exit"])
    for line in chat.stdout:
        event = json.loads(line)
        if event["type"] == "input" and event["mode"] == "task":
            print(next(commands), file=chat.stdin, flush=True)
        elif event["type"] == "assistant_delta":
            print(event["text"], end="", flush=True)
    raise SystemExit(chat.wait())
```
