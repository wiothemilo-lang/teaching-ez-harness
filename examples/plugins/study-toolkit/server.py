#!/usr/bin/env python3
"""MCP stdio demo server cho tez.

Chỉ dùng thư viện chuẩn; mục đích là test end-to-end MCP client:
- initialize
- tools/list
- tools/call: echo, add
"""

import json
import sys

TOOLS = [
    {
        "name": "echo",
        "description": "Trả lại đúng đoạn text nhận được.",
        "inputSchema": {
            "type": "object",
            "properties": {"text": {"type": "string"}},
            "required": ["text"],
        },
    },
    {
        "name": "add",
        "description": "Cộng hai số.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "a": {"type": "number"},
                "b": {"type": "number"},
            },
            "required": ["a", "b"],
        },
    },
]


def respond(request_id, result=None, error=None):
    message = {"jsonrpc": "2.0", "id": request_id}
    if error is not None:
        message["error"] = error
    else:
        message["result"] = result
    print(json.dumps(message), flush=True)


def main():
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            request = json.loads(line)
        except json.JSONDecodeError:
            continue

        method = request.get("method")
        request_id = request.get("id")

        if method == "initialize":
            respond(
                request_id,
                {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "tez-demo", "version": "0.1.0"},
                },
            )
        elif method == "notifications/initialized":
            continue
        elif method == "tools/list":
            respond(request_id, {"tools": TOOLS})
        elif method == "tools/call":
            params = request.get("params", {})
            name = params.get("name")
            args = params.get("arguments", {})
            if name == "echo":
                respond(
                    request_id,
                    {
                        "content": [{"type": "text", "text": str(args.get("text", ""))}],
                        "isError": False,
                    },
                )
            elif name == "add":
                try:
                    total = float(args.get("a", 0)) + float(args.get("b", 0))
                except (TypeError, ValueError):
                    respond(
                        request_id,
                        {
                            "content": [{"type": "text", "text": "a và b phải là số"}],
                            "isError": True,
                        },
                    )
                    continue
                text = str(int(total)) if total.is_integer() else str(total)
                respond(
                    request_id,
                    {"content": [{"type": "text", "text": text}], "isError": False},
                )
            else:
                respond(
                    request_id,
                    {
                        "content": [{"type": "text", "text": f"unknown tool: {name}"}],
                        "isError": True,
                    },
                )
        elif request_id is not None:
            respond(
                request_id,
                error={"code": -32601, "message": f"method not found: {method}"},
            )


if __name__ == "__main__":
    main()
