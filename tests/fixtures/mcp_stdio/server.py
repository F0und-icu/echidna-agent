#!/usr/bin/env python3
"""Synthetic stdio MCP peer. No SDK, model, account, or external endpoint."""
import json
import os
from pathlib import Path
import subprocess
import sys
import time

mode, record_dir, *literal_args = sys.argv[1:]
root = Path(record_dir)
root.mkdir(exist_ok=True)
descendant = subprocess.Popen(
    [sys.executable, "-c", "import time; time.sleep(60)"],
    stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
)
(root / "launch.json").write_text(json.dumps({
    "pid": os.getpid(), "descendant": descendant.pid, "cwd": os.getcwd(),
    "env_keys": sorted(os.environ), "literal": os.environ.get("MCP_LITERAL"),
    "explicit": os.environ.get("MCP_EXPLICIT"), "args": literal_args,
}))


def record(value):
    with (root / "requests.jsonl").open("a") as output:
        output.write(json.dumps(value) + "\n")


def send(value):
    body = (json.dumps(value, separators=(",", ":")) + "\n").encode()
    if mode == "fragment":
        for part in range(0, len(body), 11):
            os.write(1, body[part:part + 11])
    else:
        sys.stdout.buffer.write(body)
        sys.stdout.buffer.flush()


def reply(request, result):
    send({"jsonrpc": "2.0", "id": request["id"], "result": result})


def flood_stderr():
    for _ in range(2048):
        os.write(2, b"MCP_STDERR_PRIVATE_CANARY" + b"x" * 4071)


definition = {
    "name": "remote.echo", "description": "synthetic stdio echo",
    "inputSchema": {
        "$defs": {"Payload": {"type": "object", "properties": {"token": {"type": "string"}}}},
        "type": "object", "properties": {"execution": {"$ref": "#/$defs/Payload"}},
    },
}

for line in sys.stdin.buffer:
    request = json.loads(line)
    record(request)
    method = request.get("method")
    if method == "initialize":
        if mode == "startup_stderr":
            flood_stderr()
        if mode == "startup_timeout":
            time.sleep(60)
        reply(request, {"protocolVersion": "wrong" if mode == "bad_initialize" else "2025-11-25", "capabilities": {"tools": {}}, "serverInfo": {"name": "fixture", "version": "1"}})
    elif method == "notifications/initialized":
        pass
    elif method == "tools/list":
        if mode == "bad_catalog":
            reply(request, {"tools": "invalid"})
        elif mode == "catalog_cycle":
            reply(request, {"tools": [], "nextCursor": "same"})
        elif mode == "catalog_error":
            send({"jsonrpc": "2.0", "id": request["id"], "error": {"code": -32603, "message": "fixture"}})
        elif mode == "catalog_many":
            reply(request, {"tools": [dict(definition, name=f"remote.{i}") for i in range(1025)]})
        elif mode == "paginate" and not request["params"].get("cursor"):
            reply(request, {"tools": [definition], "nextCursor": "next"})
        elif mode == "paginate":
            reply(request, {"tools": [dict(definition, name="remote.second")]})
        else:
            reply(request, {"tools": [definition]})
    elif method == "tools/call":
        if mode == "stderr":
            flood_stderr()
        if mode in ("timeout", "cancel"):
            time.sleep(60)
        if mode == "eof":
            break
        if mode == "wrong_id":
            send({"jsonrpc": "2.0", "id": "unrelated", "result": {}})
            continue
        if mode == "malformed":
            os.write(1, b"not-json\n")
            continue
        if mode == "no_newline":
            os.write(1, b'{"jsonrpc":"2.0","id":"4","result":{}}')
            break
        if mode == "frame_limit":
            sys.stdout.buffer.write(b"x" * (2 * 1024 * 1024 + 1))
            sys.stdout.buffer.flush()
            continue
        if mode == "material_limit":
            for _ in range(9):
                send({"jsonrpc": "2.0", "method": "notifications/progress", "params": {"text": "x" * (1024 * 1024)}})
            continue
        if mode == "server_reply_limit":
            server_request = {"jsonrpc": "2.0", "id": "", "method": "x", "params": {}}
            overhead = len(json.dumps(server_request, separators=(",", ":"))) + 1
            server_request["id"] = "x" * (2 * 1024 * 1024 - overhead)
            send(server_request)
            time.sleep(60)
        if mode.startswith("bad_params_"):
            kind, shape = mode.removeprefix("bad_params_").split("_")
            bad_message = {
                "jsonrpc": "2.0", "method": "ping" if kind == "ping" else f"notifications/{kind}",
                "params": {"scalar": "invalid", "null": None, "array": []}[shape],
            }
            if kind == "ping":
                bad_message["id"] = "server-ping"
            send(bad_message)
        if mode in ("ping", "ping_absent", "unknown_request"):
            server_request = {"jsonrpc": "2.0", "id": "server-ping", "method": "sampling/createMessage" if mode == "unknown_request" else "ping", "params": {}}
            if mode == "ping_absent":
                del server_request["params"]
            send(server_request)
            record(json.loads(sys.stdin.buffer.readline()))
        if mode == "unknown_notification":
            send({"jsonrpc": "2.0", "method": "notifications/unknown", "params": {}})
            continue
        if mode in ("ping", "ping_absent"):
            send({"jsonrpc": "2.0", "method": "notifications/message", "params": {"level": "info", "data": "private server log"}})
            send({"jsonrpc": "2.0", "method": "notifications/progress", "params": {"progressToken": "p", "progress": 1}})
        if mode == "rpc_error":
            send({"jsonrpc": "2.0", "id": request["id"], "error": {"code": -32000, "message": "known error", "data": {"retained": True}}})
        else:
            reply(request, {"content": [{"type": "text", "text": "echo"}], "structuredContent": request["params"]["arguments"], "isError": mode == "tool_error", "extension": {"preserved": True}})
        if mode == "leader_exit":
            break
