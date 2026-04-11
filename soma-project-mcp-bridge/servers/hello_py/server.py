#!/usr/bin/env python3
"""Minimal MCP stdio server exposing a 'greet' tool.

Pure Python stdlib — no `mcp` SDK dependency so this proof project works on
any machine with Python 3. This is the shortest possible demonstration that
soma-next can load an MCP server as a port via `McpTransport::Stdio`.

The same wire protocol is what `@modelcontextprotocol/sdk` (Node) and the
`mcp` pip package speak; any language with an MCP SDK produces a server
that slots into the same place.
"""

import json
import sys

PROTOCOL_VERSION = "2024-11-05"

TOOLS = [
    {
        "name": "greet",
        "description": "Return a personalized greeting for the given name.",
        "inputSchema": {
            "type": "object",
            "required": ["name"],
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Who to greet.",
                },
            },
        },
    },
    {
        "name": "reverse",
        "description": "Reverse the given text.",
        "inputSchema": {
            "type": "object",
            "required": ["text"],
            "properties": {
                "text": {"type": "string"},
            },
        },
    },
]


def respond(req_id, result=None, error=None):
    """Write a single JSON-RPC response line to stdout and flush."""
    msg = {"jsonrpc": "2.0", "id": req_id}
    if error is not None:
        msg["error"] = error
    else:
        msg["result"] = result
    sys.stdout.write(json.dumps(msg) + "\n")
    sys.stdout.flush()


def handle(req):
    method = req.get("method", "")
    req_id = req.get("id")
    params = req.get("params") or {}

    if method == "initialize":
        respond(req_id, {
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "hello-mcp", "version": "0.1.0"},
        })
        return

    if method == "notifications/initialized":
        # Notification has no id and needs no response.
        return

    if method == "tools/list":
        respond(req_id, {"tools": TOOLS})
        return

    if method == "tools/call":
        name = params.get("name")
        args = params.get("arguments") or {}

        if name == "greet":
            who = args.get("name", "stranger")
            payload = {"message": f"hello {who}! (from python)"}
            respond(req_id, {
                "content": [
                    {"type": "text", "text": json.dumps(payload)},
                ],
                "isError": False,
            })
            return

        if name == "reverse":
            text = args.get("text", "")
            payload = {"reversed": text[::-1]}
            respond(req_id, {
                "content": [
                    {"type": "text", "text": json.dumps(payload)},
                ],
                "isError": False,
            })
            return

        respond(req_id, error={
            "code": -32601,
            "message": f"unknown tool: {name}",
        })
        return

    if req_id is not None:
        respond(req_id, error={
            "code": -32601,
            "message": f"unknown method: {method}",
        })


def main():
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
        except json.JSONDecodeError as e:
            sys.stderr.write(f"[hello-mcp] ignoring invalid JSON: {e}\n")
            continue
        handle(req)


if __name__ == "__main__":
    main()
