#!/usr/bin/env python3
"""A mock of Korotovsky's Slack MCP server (stdio MCP) for tests.

Speaks newline-delimited JSON-RPC over stdin/stdout, exactly like the real
server's stdio transport: `initialize`, `notifications/initialized` (no
reply), `tools/list` (the korotovsky tool set, including
`conversations_add_message` with its schema) and `tools/call`.

Every tool call is appended as one JSON line to the file named by
MOCK_KOROTOVSKY_LOG (the tests read it back). The first line written is the
child environment the gateway set (the keys the server cares about), so
tests can assert the gateway enabled the send tool and passed the token.

Environment knobs:
  MOCK_KOROTOVSKY_LOG       JSON-lines file to record into (required)
  MOCK_KOROTOVSKY_PIDFILE   write the pid here (drop test)
  MOCK_KOROTOVSKY_NOISE     print a non-JSON line to stdout before replies
  MOCK_KOROTOVSKY_NO_ADD_MESSAGE  do not list conversations_add_message
  MOCK_KOROTOVSKY_EXIT      print to stderr and exit 1 immediately
  MOCK_KOROTOVSKY_TOOL_ERROR  reply to conversations_add_message with isError
"""

import json
import os
import sys

LOG = os.environ.get("MOCK_KOROTOVSKY_LOG")


def record(entry):
    if not LOG:
        return
    with open(LOG, "a", encoding="utf-8") as f:
        f.write(json.dumps(entry) + "\n")


def send(message):
    sys.stdout.write(json.dumps(message) + "\n")
    sys.stdout.flush()


ADD_MESSAGE_TOOL = {
    "name": "conversations_add_message",
    "description": "Add a message to a conversation by channel_id and thread_ts.",
    "inputSchema": {
        "type": "object",
        "properties": {
            "channel_id": {"type": "string"},
            "thread_ts": {"type": "string"},
            "text": {"type": "string"},
            "content_type": {"type": "string", "default": "text/markdown"},
            "blocks": {"type": "string"},
        },
        "required": ["channel_id"],
    },
}

CHANNELS_LIST_TOOL = {
    "name": "channels_list",
    "description": "Get list of channels",
    "inputSchema": {
        "type": "object",
        "properties": {
            "channel_types": {"type": "string"},
            "sort": {"type": "string"},
            "limit": {"type": "number"},
            "cursor": {"type": "string"},
        },
        "required": ["channel_types"],
    },
}

USERS_SEARCH_TOOL = {
    "name": "users_search",
    "description": "Search for users by name, email, display name, or Slack user ID.",
    "inputSchema": {
        "type": "object",
        "properties": {
            "query": {"type": "string"},
            "limit": {"type": "number"},
        },
        "required": ["query"],
    },
}

HISTORY_TOOL = {
    "name": "conversations_history",
    "description": "Get messages from the channel by channel_id.",
    "inputSchema": {
        "type": "object",
        "properties": {"channel_id": {"type": "string"}},
        "required": ["channel_id"],
    },
}

# CSV replies in the real server's shape (gocsv: struct field names as the
# header row).
CHANNELS_CSV = (
    "ID,Name,Topic,Purpose,MemberCount,Cursor\n"
    "C0123SMOKE,eng,topic,purpose,42,\n"
    'C0456SMOKE,"team, chat",t,p,7,'
)
USERS_CSV = (
    "UserID,UserName,RealName,DisplayName,Email,Title,DMChannelID\n"
    "U0123SMOKE,drake.thomsen,Drake Thomsen,Drake,,CTO,D0123SMOKE\n"
    "U0456SMOKE,jane.doe,Jane Doe,Jane,jane@example.com,,D0456SMOKE"
)


def text_result(text, is_error=False):
    return {
        "content": [{"type": "text", "text": text}],
        "isError": is_error,
    }


def handle_call(params):
    name = params.get("name")
    arguments = params.get("arguments") or {}
    record({"tool": name, "arguments": arguments})
    if name == "conversations_add_message":
        if os.environ.get("MOCK_KOROTOVSKY_TOOL_ERROR"):
            return text_result("channel_not_found (mock)", is_error=True)
        channel = arguments.get("channel_id", "")
        ts = "1700000000.000100"
        thread = arguments.get("thread_ts")
        if thread:
            return text_result(
                f"Successfully posted message to channel {channel} "
                f"in thread {thread} (ts={ts})"
            )
        return text_result(f"Successfully posted message to channel {channel} (ts={ts})")
    if name == "channels_list":
        return text_result(CHANNELS_CSV)
    if name == "users_search":
        return text_result(USERS_CSV)
    if name == "conversations_history":
        return text_result("[]")
    return text_result(f"unknown tool {name}", is_error=True)


def main():
    if os.environ.get("MOCK_KOROTOVSKY_EXIT"):
        sys.stderr.write("mock korotovsky: invalid token, refusing to start\n")
        sys.exit(1)

    pidfile = os.environ.get("MOCK_KOROTOVSKY_PIDFILE")
    if pidfile:
        with open(pidfile, "w", encoding="utf-8") as f:
            f.write(str(os.getpid()))

    # Record the environment the gateway gave us (the server's own keys).
    record(
        {
            "env": {
                key: os.environ.get(key)
                for key in (
                    "SLACK_MCP_XOXB_TOKEN",
                    "SLACK_MCP_XOXP_TOKEN",
                    "SLACK_MCP_ADD_MESSAGE_TOOL",
                    "SLACK_MCP_LOG_LEVEL",
                    "SLACK_MCP_USERS_CACHE",
                    "SLACK_MCP_CHANNELS_CACHE",
                )
            }
        }
    )

    tools = [CHANNELS_LIST_TOOL, USERS_SEARCH_TOOL, HISTORY_TOOL]
    if not os.environ.get("MOCK_KOROTOVSKY_NO_ADD_MESSAGE"):
        tools.insert(0, ADD_MESSAGE_TOOL)

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        if os.environ.get("MOCK_KOROTOVSKY_NOISE"):
            # The real server prints to stdout on a bad log level; the client
            # must skip anything that is not one JSON-RPC message.
            sys.stdout.write("2026-10-23T00:00:00Z INFO mock starting up\n")
            sys.stdout.flush()
        try:
            request = json.loads(line)
        except json.JSONDecodeError:
            continue
        method = request.get("method")
        if method == "initialize":
            send(
                {
                    "jsonrpc": "2.0",
                    "id": request.get("id"),
                    "result": {
                        "protocolVersion": "2025-06-18",
                        "capabilities": {"tools": {}},
                        "serverInfo": {"name": "mock-korotovsky", "version": "0.0.0"},
                    },
                }
            )
        elif method == "notifications/initialized":
            pass  # a notification: no reply
        elif method == "tools/list":
            send(
                {
                    "jsonrpc": "2.0",
                    "id": request.get("id"),
                    "result": {"tools": tools},
                }
            )
        elif method == "tools/call":
            send(
                {
                    "jsonrpc": "2.0",
                    "id": request.get("id"),
                    "result": handle_call(request.get("params") or {}),
                }
            )
        elif method is not None and "id" in request:
            send(
                {
                    "jsonrpc": "2.0",
                    "id": request.get("id"),
                    "error": {"code": -32601, "message": f"method not found: {method}"},
                }
            )


if __name__ == "__main__":
    main()
