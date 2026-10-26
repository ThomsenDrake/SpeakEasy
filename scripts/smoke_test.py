#!/usr/bin/env python3
"""End-to-end smoke test of the real slack-approval-gateway binary.

Runs `slack-approval-gateway --cli` with:
  - stdin/stdout/stderr = pipes (the MCP stdio transport, like a real agent)
  - a pty as the controlling terminal (so the CLI provider's /dev/tty is the
    pty, like a human's terminal)
  - SLACK_MCP_URL pointed at a local mock Slack MCP server (the default
    transport: the gateway speaks MCP Streamable HTTP to a Slack MCP server
    with a Slack user OAuth token, never a bot token)

Covers: MCP handshake, tools/list, approve (exact send + permalink),
duplicate retry (no resend), deny (cooldown), reworded retry during cooldown
(blocked, no prompt), feedback (no send), two concurrent requests whose
prompts are answered in order (CLI mutex: answers reach the right requests),
and the single-instance bridge: extra agent processes (no --cli) connect to
the running instance over the state-dir socket and their MCP sessions are
served by that one instance (same popup owner, same Slack client, shared
dedupe/cooldowns) — including an explicit error (nothing was sent) when the
instance is killed while a bridged request is pending. The mock server also
plays the Mistral platform API, and one section runs the whole approve flow
with SLACK_TRANSPORT=mistral-connector to verify the connector transport
end to end (exact message sent through the connector tool call, no Slack
MCP call). A final section checks that a Slack user ID in the destination
resolves to a display name in the prompt while the send still carries the
exact raw ID. A korotovsky section runs the approve flow with
SLACK_TRANSPORT=korotovsky against a mock of Korotovsky's unofficial Slack
MCP server (scripts/mock_korotovsky.py) spawned as a stdio child: the
gateway enables the server's conversations_add_message tool, sends the
exact approved text to the exact destination as text/plain, and builds the
permalink from the reply (no Slack MCP or connector call is made).
"""
import fcntl
import http.server
import json
import os
import pty
import socketserver
import subprocess
import sys
import tempfile
import threading
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BINARY = os.path.join(ROOT, "target/debug/slack-approval-gateway")

failures = []


def check(name, cond, detail=""):
    status = "PASS" if cond else "FAIL"
    print(f"[{status}] {name}" + (f" — {detail}" if detail and not cond else ""))
    if not cond:
        failures.append(name)


# ------------------------------------------- mock Slack MCP + Mistral API
# One HTTP server plays both the Slack MCP server (the default transport:
# MCP Streamable HTTP at /mcp) and the Mistral platform API (connector
# transport).
MCP_TOOLS = [
    {"name": "slack_search_messages", "inputSchema": {"type": "object", "properties": {
        "query": {"type": "string"}}}},
    {"name": "slack_read_user_profile", "inputSchema": {"type": "object", "properties": {
        "user_id": {"type": "string"}}}},
    {"name": "slack_read_channel", "inputSchema": {"type": "object", "properties": {
        "channel_id": {"type": "string"}}}},
    {"name": "slack_list_channels", "inputSchema": {"type": "object"}},
    {"name": "slack_send_message", "inputSchema": {"type": "object", "properties": {
        "channel_id": {"type": "string"},
        "message": {"type": "string"},
        "thread_ts": {"type": "string"},
    }, "required": ["channel_id", "message"]}},
]


def route_mcp(body):
    """Canned Slack MCP reply for one JSON-RPC message (POST /mcp).

    Returns (http_status, reply_or_None, extra_headers).
    """
    method = body.get("method")
    rid = body.get("id")
    if method == "initialize":
        params = body.get("params") or {}
        reply = {
            "jsonrpc": "2.0",
            "id": rid,
            "result": {
                "protocolVersion": params.get("protocolVersion", "2025-06-18"),
                "capabilities": {},
                "serverInfo": {"name": "mock-slack-mcp", "version": "0"},
            },
        }
        return 200, reply, {"Mcp-Session-Id": "mock-session-1"}
    if method == "notifications/initialized":
        # The handshake completes with a notification; accept it, answer nothing.
        return 202, None, {}
    if method == "tools/list":
        return 200, {"jsonrpc": "2.0", "id": rid, "result": {"tools": MCP_TOOLS}}, {}
    if method == "tools/call":
        params = body.get("params") or {}
        name = params.get("name") or ""
        args = params.get("arguments") or {}
        if name == "slack_send_message":
            # A JSON string inside the text block, like the real Slack tools.
            result = {"content": [{"type": "text", "text": json.dumps({
                "message_link": "https://mock.slack.com/archives/C123/p1700000000000100",
                "message_context": {"message_ts": "1700000000.000100",
                                    "channel_id": "C123"},
            })}]}
        elif name == "slack_read_user_profile":
            if args.get("user_id") == "U0DRAKE":
                result = {"content": [{"type": "text", "text": json.dumps({"user": {
                    "real_name": "Drake Thomsen", "name": "drake.thomsen",
                    "profile": {"display_name": "Drake"}}})}]}
            else:
                result = {"content": [{"type": "text", "text": "user_not_found"}],
                          "isError": True}
        elif name == "slack_read_channel":
            if args.get("channel_id") == "C0ENG":
                result = {"content": [{"type": "text", "text": json.dumps(
                    {"channel": {"id": "C0ENG", "name": "eng"}})}]}
            else:
                result = {"content": [{"type": "text", "text": "channel_not_found"}],
                          "isError": True}
        else:
            result = {"content": [{"type": "text", "text": f"unknown tool: {name}"}],
                      "isError": True}
        return 200, {"jsonrpc": "2.0", "id": rid, "result": result}, {}
    return 200, {"jsonrpc": "2.0", "id": rid,
                  "error": {"code": -32601, "message": f"method not found: {method}"}}, {}


class Handler(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        length = int(self.headers.get("Content-Length", 0))
        body = json.loads(self.rfile.read(length) or b"{}")
        path = self.path.split("?")[0]
        self.server.requests.append((path, dict(self.headers), body))
        if path == "/mcp":
            status, resp, extra = route_mcp(body)
            data = b"" if resp is None else json.dumps(resp).encode()
        elif path.startswith("/v1/connectors/") and path.endswith("/call"):
            # MCP CallToolResult shaped like the real slack connector: a JSON
            # string inside the text block.
            resp = {
                "content": [{"type": "text", "text":
                             "{\"message_link\":\"https://mock.slack.com/archives/C123/p1700000000000100\",\"message_context\":{\"message_ts\":\"1700000000.000100\",\"channel_id\":\"C123\"}}"}],
            }
            status, data, extra = 200, json.dumps(resp).encode(), {}
        else:
            resp = {"ok": False, "error": "unknown_method"}
            status, data, extra = 200, json.dumps(resp).encode(), {}
        self.send_response(status)
        for name, value in extra.items():
            self.send_header(name, value)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        if data:
            self.wfile.write(data)

    def do_GET(self):
        path = self.path.split("?")[0]
        self.server.requests.append((path, dict(self.headers), None))
        if path.startswith("/v1/connectors/") and path.endswith("/tools"):
            # Bare array, like the real API; the send tool declares the real
            # argument names (channel_id/message/thread_ts).
            resp = [
                {"name": "slack_search_messages", "inputSchema": {"type": "object"}},
                {"name": "slack_send_message", "inputSchema": {"type": "object",
                    "properties": {
                        "channel_id": {"type": "string"},
                        "message": {"type": "string"},
                        "thread_ts": {"type": "string"},
                    }, "required": ["channel_id", "message"]}},
            ]
        else:
            resp = {"detail": "not found"}
        data = json.dumps(resp).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *args):
        pass


server = socketserver.TCPServer(("127.0.0.1", 0), Handler)
server.requests = []
threading.Thread(target=server.serve_forever, daemon=True).start()
port = server.server_address[1]


def send_calls():
    """Recorded MCP `tools/call` requests for a send-message tool."""
    out = []
    for path, _, body in server.requests:
        if path != "/mcp" or not isinstance(body, dict):
            continue
        if body.get("method") != "tools/call":
            continue
        name = (body.get("params") or {}).get("name") or ""
        if ("send" in name or "post" in name) and "message" in name:
            out.append(body)
    return out


def send_count():
    return len(send_calls())


def send_arguments(call):
    """The `arguments` object of one recorded send-tool call."""
    return (call.get("params") or {}).get("arguments") or {}


def connector_requests(suffix):
    """Recorded Mistral API requests whose path ends with `suffix`."""
    return [(p, h, b) for p, h, b in server.requests
            if p.startswith("/v1/connectors/") and p.endswith(suffix)]


def connector_call_count():
    return len(connector_requests("/call"))


def last_connector_call_arguments():
    """The `arguments` object of the most recent connector tool call."""
    for p, _, body in reversed(server.requests):
        if p.startswith("/v1/connectors/") and p.endswith("/call"):
            return (body or {}).get("arguments", {})
    return {}


# ---------------------------------------------------------------- spawn app
state_dir = tempfile.mkdtemp(prefix="sag-smoke-")
env = dict(os.environ)
env["SLACK_MCP_TOKEN"] = "xoxp-smoke-test"
env["SLACK_MCP_URL"] = f"http://127.0.0.1:{port}/mcp"
env["SLACK_GATEWAY_STATE_DIR"] = state_dir
for k in ("HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy", "ALL_PROXY", "all_proxy"):
    env.pop(k, None)
env["NO_PROXY"] = "127.0.0.1,localhost"
env["NO_PROXY"] = "127.0.0.1,localhost"

master, slave = pty.openpty()


def make_controlling_tty():
    os.setsid()
    fcntl.ioctl(slave, 0x20007461, 0)  # TIOCSCTTY on macOS


proc = subprocess.Popen(
    [BINARY, "--cli"],
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    stderr=subprocess.PIPE,
    env=env,
    pass_fds=(slave,),
    preexec_fn=make_controlling_tty,
)
os.close(slave)
spawned = [proc]  # every instance process, for the finally cleanup
bridges = []     # every bridge process, for the finally cleanup

tty_out = bytearray()


def tty_reader():
    while True:
        try:
            chunk = os.read(master, 4096)
        except OSError:
            break
        if not chunk:
            break
        tty_out.extend(chunk)


threading.Thread(target=tty_reader, daemon=True).start()

responses = {}
resp_lock = threading.Lock()


def stdout_reader():
    for raw in proc.stdout:
        line = raw.decode(errors="replace").strip()
        if not line:
            continue
        try:
            msg = json.loads(line)
        except json.JSONDecodeError:
            continue
        if "id" in msg:
            with resp_lock:
                responses[msg["id"]] = msg


threading.Thread(target=stdout_reader, daemon=True).start()


def rpc(id_, method, params=None):
    msg = {"jsonrpc": "2.0", "id": id_, "method": method}
    if params is not None:
        msg["params"] = params
    proc.stdin.write((json.dumps(msg) + "\n").encode())
    proc.stdin.flush()


def wait_response(id_, timeout=30):
    deadline = time.time() + timeout
    while time.time() < deadline:
        with resp_lock:
            if id_ in responses:
                return responses[id_]
        if proc.poll() is not None:
            raise AssertionError(f"gateway exited early (code {proc.returncode})")
        time.sleep(0.05)
    raise AssertionError(f"timeout waiting for response id={id_}")


def wait_tty(pattern, timeout=30):
    deadline = time.time() + timeout
    while time.time() < deadline:
        if pattern.encode() in tty_out:
            return
        time.sleep(0.05)
    raise AssertionError(
        f"timeout waiting for tty pattern {pattern!r}; tty tail: {bytes(tty_out[-1500:])!r}"
    )


def answer(text):
    os.write(master, (text + "\n").encode())


def tool_call(id_, message, destination="#general", thread_ts=None, reason="smoke test",
              agent="smoke-agent"):
    args = {
        "message": message,
        "destination": destination,
        "reason": reason,
        "agent": agent,
        "conversation": {
            "title": "Smoke test conversation",
            "id_or_link": "smoke-1",
            "summary": "Verifying the approval gateway end to end.",
        },
    }
    if thread_ts is not None:
        args["thread_ts"] = thread_ts
    rpc(id_, "tools/call", {"name": "request_slack_message", "arguments": args})


def verdict(resp):
    """Extract the structured verdict object from a tools/call response."""
    result = resp.get("result", {})
    text = result.get("content", [{}])[0].get("text", "")
    return json.loads(text)


try:
    # 1. handshake
    rpc(1, "initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                          "clientInfo": {"name": "smoke", "version": "0"}})
    r = wait_response(1)
    check("initialize returns serverInfo", r.get("result", {}).get("serverInfo", {}).get("name")
          == "slack-approval-gateway", json.dumps(r)[:200])

    rpc(2, "tools/list")
    r = wait_response(2)
    tools = r.get("result", {}).get("tools", [])
    check("tools/list exposes request_slack_message",
          len(tools) == 1 and tools[0]["name"] == "request_slack_message",
          json.dumps(r)[:300])
    schema = tools[0].get("inputSchema", {})
    check("tool schema requires the six required fields",
          set(schema.get("required", [])) == {"message", "destination", "reason", "agent",
                                              "conversation"},
          json.dumps(schema)[:300])

    # 2. approve: exact message sent, permalink returned
    tool_call(3, "Hello from the smoke test", thread_ts="1699999999.000001")
    wait_tty("Hello from the smoke test")
    wait_tty("Approve [a]")
    answer("a")
    r = wait_response(3)
    v = verdict(r)
    check("approve returns sent with permalink",
          v.get("status") == "sent" and v.get("permalink", "").startswith("https://mock.slack.com/"),
          json.dumps(v))
    sends = send_calls()
    check("mock Slack MCP received exactly one send", send_count() == 1, f"count={send_count()}")
    check("send tool call carried the exact message, destination and thread",
          sends and send_arguments(sends[0]).get("message") == "Hello from the smoke test"
          and send_arguments(sends[0]).get("channel_id") == "#general"
          and send_arguments(sends[0]).get("thread_ts") == "1699999999.000001",
          json.dumps(sends[:1]))

    # 3. identical retry: cached duplicate, no resend
    tool_call(4, "Hello from the smoke test", thread_ts="1699999999.000001")
    r = wait_response(4)
    v = verdict(r)
    check("identical retry returns duplicate without resending",
          v.get("status") == "sent" and v.get("duplicate") is True and send_count() == 1,
          json.dumps(v))

    # 4. deny: cooldown set
    tool_call(5, "Deploy finished, please review")
    wait_tty("Deploy finished, please review")
    answer("d")
    r = wait_response(5)
    v = verdict(r)
    check("deny returns denied with 600s cooldown",
          v.get("status") == "denied" and v.get("cooldown_remaining_secs") == 600,
          json.dumps(v))

    # 5. reworded retry during cooldown: blocked, no prompt, no send
    tool_call(6, "Deploy finished, kindly review")  # same purpose, reworded
    r = wait_response(6)
    v = verdict(r)
    check("reworded retry is cooldown_blocked with remaining time, no send",
          v.get("status") == "cooldown_blocked"
          and 0 < v.get("cooldown_remaining_secs", 0) <= 600
          and send_count() == 1,
          json.dumps(v))
    time.sleep(0.5)
    check("no CLI prompt appeared for the blocked request",
          tty_out.count(b"Approve [a]") == 2, f"prompts={tty_out.count(b'Approve [a]')}")

    # 6. feedback: returned to the agent, nothing sent
    tool_call(7, "Lunch plans for Friday?")
    wait_tty("Lunch plans for Friday?")
    answer("f how about pizza instead")
    r = wait_response(7)
    v = verdict(r)
    check("feedback is returned verbatim, nothing sent",
          v.get("status") == "feedback" and v.get("feedback") == "how about pizza instead"
          and send_count() == 1,
          json.dumps(v))

    # 7. concurrency: two requests, prompts serialized, answers matched.
    # The two prompts race for the CLI lock, so whichever request prompts
    # first is answered first; the answers are matched to the right
    # requests by the prompt text (verified through the per-request
    # verdicts and the send log).
    tool_call(8, "Concurrent request alpha")
    tool_call(9, "Concurrent request beta")

    def wait_tty_either(patterns, timeout=30):
        deadline = time.time() + timeout
        while time.time() < deadline:
            for pattern in patterns:
                if pattern.encode() in tty_out:
                    return pattern
            time.sleep(0.05)
        raise AssertionError(
            f"timeout waiting for tty patterns {patterns!r}; tty tail: {bytes(tty_out[-1500:])!r}"
        )

    first = wait_tty_either(["Concurrent request alpha", "Concurrent request beta"])
    other = ("Concurrent request beta" if first == "Concurrent request alpha"
             else "Concurrent request alpha")
    answer("a")  # approves whichever request prompted first
    wait_tty(other)  # the second prompt: the other request
    answer("f not now")  # feedback for it
    v8 = verdict(wait_response(8))
    v9 = verdict(wait_response(9))
    approved, feedback = (v8, v9) if first == "Concurrent request alpha" else (v9, v8)
    check("concurrent requests do not mix up decisions",
          approved.get("status") == "sent" and not approved.get("duplicate")
          and feedback.get("status") == "feedback" and feedback.get("feedback") == "not now"
          and send_count() == 2,
          f"v8={json.dumps(v8)} v9={json.dumps(v9)} sends={send_count()}")
    approved_sends = [b for b in send_calls()
                      if send_arguments(b).get("message") == first]
    check("the approved request was the message actually sent", len(approved_sends) == 1)

    # 7b. bridge: a second agent process (default mode, no --cli) must not
    # start its own instance — it bridges to the running one over the
    # state-dir socket and its MCP session is served by that instance.
    def spawn_bridge():
        bp = subprocess.Popen(
            [BINARY],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            env=env,
        )
        bridges.append(bp)
        responses_b = {}
        lock_b = threading.Lock()
        # Set only after the reader has drained stdout to EOF, so a bridge
        # that answers and then exits is not mistaken for one that died
        # without answering.
        reader_done = threading.Event()

        def reader_b():
            for raw in bp.stdout:
                line = raw.decode(errors="replace").strip()
                if not line:
                    continue
                try:
                    msg = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if "id" in msg:
                    with lock_b:
                        responses_b[msg["id"]] = msg
            reader_done.set()

        threading.Thread(target=reader_b, daemon=True).start()
        return bp, responses_b, lock_b, reader_done

    def make_bridge_rpc(bp, responses_b, lock_b, reader_done):
        def brpc(id_, method, params=None):
            msg = {"jsonrpc": "2.0", "id": id_, "method": method}
            if params is not None:
                msg["params"] = params
            bp.stdin.write((json.dumps(msg) + "\n").encode())
            bp.stdin.flush()

        def wait_b(id_, timeout=30):
            deadline = time.time() + timeout
            while time.time() < deadline:
                with lock_b:
                    if id_ in responses_b:
                        return responses_b[id_]
                if reader_done.is_set():
                    raise AssertionError(
                        f"bridge exited without answering id={id_} (code {bp.returncode})")
                time.sleep(0.05)
            raise AssertionError(f"timeout waiting for bridge response id={id_}")

        return brpc, wait_b

    def bridge_tool_call(brpc, id_, message):
        brpc(id_, "tools/call", {"name": "request_slack_message", "arguments": {
            "message": message,
            "destination": "#general",
            "reason": "smoke test (bridged)",
            "agent": "smoke-agent-2",
            "conversation": {
                "title": "Smoke test conversation",
                "id_or_link": "smoke-bridge",
                "summary": "A second agent process bridged to the instance.",
            },
        }})

    bridge1, bresp1, block1, done1 = spawn_bridge()
    brpc1, wait_b1 = make_bridge_rpc(bridge1, bresp1, block1, done1)
    brpc1(101, "initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                              "clientInfo": {"name": "smoke-bridge", "version": "0"}})
    r = wait_b1(101)
    check("bridge: initialize answered by the running instance",
          r.get("result", {}).get("serverInfo", {}).get("name") == "slack-approval-gateway",
          json.dumps(r)[:200])

    posts_before = send_count()
    bridge_tool_call(brpc1, 102, "Bridged request one")
    wait_tty("Bridged request one")
    answer("a")
    r = wait_b1(102)
    v = verdict(r)
    check("bridge: approve through the bridge sends via the instance",
          v.get("status") == "sent" and send_count() == posts_before + 1,
          json.dumps(v))

    # a second bridge at the same time: concurrent bridged agents
    bridge2, bresp2, block2, done2 = spawn_bridge()
    brpc2, wait_b2 = make_bridge_rpc(bridge2, bresp2, block2, done2)
    brpc2(201, "initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                              "clientInfo": {"name": "smoke-bridge-2", "version": "0"}})
    wait_b2(201)
    bridge_tool_call(brpc2, 202, "Bridged request two")
    wait_tty("Bridged request two")
    answer("f bridged feedback")
    r = wait_b2(202)
    v = verdict(r)
    check("bridge: second concurrent bridge gets its own verdict",
          v.get("status") == "feedback" and v.get("feedback") == "bridged feedback"
          and send_count() == posts_before + 1,
          json.dumps(v))

    # closing an agent's stdio ends its bridge cleanly (the instance lives on)
    bridge1.stdin.close()
    bridge1.wait(timeout=15)
    check("bridge exits cleanly when the agent closes stdio", bridge1.returncode == 0,
          f"code={bridge1.returncode}")
    bridge2.stdin.close()
    bridge2.wait(timeout=15)
    check("second bridge exits cleanly too", bridge2.returncode == 0,
          f"code={bridge2.returncode}")
    bridge_err = bridge1.stderr.read().decode(errors="replace")
    check("bridge logged that it bridged (did not start a second instance)",
          "bridging MCP stdio" in bridge_err, bridge_err[-300:])

    # 7c. bridge: if the instance dies (SIGKILL) while a bridged request is
    # pending, the agent gets an explicit JSON-RPC error (nothing was sent),
    # not a hang on a dead session
    bridge3, bresp3, block3, done3 = spawn_bridge()
    brpc3, wait_b3 = make_bridge_rpc(bridge3, bresp3, block3, done3)
    brpc3(401, "initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                              "clientInfo": {"name": "smoke-bridge-3", "version": "0"}})
    wait_b3(401)
    bridge_tool_call(brpc3, 402, "Bridged request pending at kill")
    wait_tty("Bridged request pending at kill")
    proc.kill()  # the instance dies without answering
    r = wait_b3(402)
    check("bridge answers a pending request with an error when the instance dies",
          "nothing was sent" in r.get("error", {}).get("message", ""),
          json.dumps(r)[:300])
    bridge3.wait(timeout=15)
    check("bridge exits after the instance dies", bridge3.returncode == 0,
          f"code={bridge3.returncode}")

    # 8. persistence across restart: cooldown survives a new process
    proc.stdin.close()
    proc.wait(timeout=15)
    master2, slave2 = pty.openpty()

    def make_ctty2():
        os.setsid()
        fcntl.ioctl(slave2, 0x20007461, 0)

    proc2 = subprocess.Popen(
        [BINARY, "--cli"],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        env=env, pass_fds=(slave2,), preexec_fn=make_ctty2,
    )
    spawned.append(proc2)
    os.close(slave2)
    tty_out2 = bytearray()

    def tty_reader2():
        while True:
            try:
                chunk = os.read(master2, 4096)
            except OSError:
                break
            if not chunk:
                break
            tty_out2.extend(chunk)

    threading.Thread(target=tty_reader2, daemon=True).start()
    responses2 = {}
    resp2_lock = threading.Lock()

    def stdout_reader2():
        for raw in proc2.stdout:
            line = raw.decode(errors="replace").strip()
            if not line:
                continue
            try:
                msg = json.loads(line)
            except json.JSONDecodeError:
                continue
            if "id" in msg:
                with resp2_lock:
                    responses2[msg["id"]] = msg

    threading.Thread(target=stdout_reader2, daemon=True).start()

    def rpc2(id_, method, params=None):
        msg = {"jsonrpc": "2.0", "id": id_, "method": method}
        if params is not None:
            msg["params"] = params
        proc2.stdin.write((json.dumps(msg) + "\n").encode())
        proc2.stdin.flush()

    def wait_response2(id_, timeout=30):
        deadline = time.time() + timeout
        while time.time() < deadline:
            with resp2_lock:
                if id_ in responses2:
                    return responses2[id_]
            time.sleep(0.05)
        raise AssertionError(f"timeout waiting for response id={id_} (restart)")

    rpc2(1, "initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                           "clientInfo": {"name": "smoke", "version": "0"}})
    wait_response2(1)
    args = {
        "message": "Deploy finished, kindly review",  # same purpose, reworded
        "destination": "#general",
        "reason": "smoke test",
        "agent": "other-agent",  # a different participating agent
        "conversation": {"title": "Smoke test conversation", "id_or_link": "smoke-2",
                         "summary": "Retry from another agent during the cooldown."},
    }
    rpc2(2, "tools/call", {"name": "request_slack_message", "arguments": args})
    r = wait_response2(2)
    v = verdict(r)
    check("cooldown persists across restart and blocks other agents",
          v.get("status") == "cooldown_blocked"
          and 0 < v.get("cooldown_remaining_secs", 0) <= 600,
          json.dumps(v))
    time.sleep(0.5)
    check("no prompt appeared after restart for the blocked request",
          b"Approve [a]" not in tty_out2, f"tty={bytes(tty_out2[-500:])!r}")

    proc2.stdin.close()
    proc2.wait(timeout=15)

    # 8b. Mistral Slack Connector transport: a fresh process selected with
    # SLACK_TRANSPORT=mistral-connector must send the approved message
    # through the connector tool call (not the Slack Web API) and return
    # the permalink the connector reports.
    master3, slave3 = pty.openpty()

    def make_ctty3():
        os.setsid()
        fcntl.ioctl(slave3, 0x20007461, 0)

    env3 = dict(env)
    env3["SLACK_TRANSPORT"] = "mistral-connector"
    env3["MISTRAL_API_KEY"] = "mistral-smoke-test"
    env3["MISTRAL_API_BASE"] = f"http://127.0.0.1:{port}"
    proc3 = subprocess.Popen(
        [BINARY, "--cli"],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        env=env3, pass_fds=(slave3,), preexec_fn=make_ctty3,
    )
    spawned.append(proc3)
    os.close(slave3)
    tty_out3 = bytearray()

    def tty_reader3():
        while True:
            try:
                chunk = os.read(master3, 4096)
            except OSError:
                break
            if not chunk:
                break
            tty_out3.extend(chunk)

    threading.Thread(target=tty_reader3, daemon=True).start()
    responses3 = {}
    resp3_lock = threading.Lock()

    def stdout_reader3():
        for raw in proc3.stdout:
            line = raw.decode(errors="replace").strip()
            if not line:
                continue
            try:
                msg = json.loads(line)
            except json.JSONDecodeError:
                continue
            if "id" in msg:
                with resp3_lock:
                    responses3[msg["id"]] = msg

    threading.Thread(target=stdout_reader3, daemon=True).start()

    def rpc3(id_, method, params=None):
        msg = {"jsonrpc": "2.0", "id": id_, "method": method}
        if params is not None:
            msg["params"] = params
        proc3.stdin.write((json.dumps(msg) + "\n").encode())
        proc3.stdin.flush()

    def wait_response3(id_, timeout=30):
        deadline = time.time() + timeout
        while time.time() < deadline:
            with resp3_lock:
                if id_ in responses3:
                    return responses3[id_]
            time.sleep(0.05)
        raise AssertionError(f"timeout waiting for response id={id_} (connector)")

    def wait_tty3(pattern, timeout=30):
        deadline = time.time() + timeout
        while time.time() < deadline:
            if pattern.encode() in tty_out3:
                return
            time.sleep(0.05)
        raise AssertionError(
            f"timeout waiting for tty pattern {pattern!r}; tty tail: {bytes(tty_out3[-1500:])!r}"
        )

    def answer3(text):
        os.write(master3, (text + "\n").encode())

    posts_before_conn = send_count()
    calls_before_conn = connector_call_count()
    rpc3(1, "initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                           "clientInfo": {"name": "smoke-connector", "version": "0"}})
    wait_response3(1)
    args3 = {
        "message": "Sent via the Mistral connector",
        "destination": "#general",
        "reason": "smoke test (connector transport)",
        "agent": "smoke-agent",
        "conversation": {"title": "Smoke test conversation", "id_or_link": "smoke-connector",
                         "summary": "Verifying the Mistral Slack Connector transport."},
    }
    rpc3(2, "tools/call", {"name": "request_slack_message", "arguments": args3})
    wait_tty3("Sent via the Mistral connector")
    wait_tty3("Approve [a]")
    answer3("a")
    r = wait_response3(2)
    v = verdict(r)
    check("connector transport: approve returns sent with the connector permalink",
          v.get("status") == "sent"
          and v.get("permalink") == "https://mock.slack.com/archives/C123/p1700000000000100",
          json.dumps(v))
    check("connector transport: the send tool was discovered and called once",
          len(connector_requests("/tools")) >= 1
          and connector_call_count() == calls_before_conn + 1,
          f"calls={connector_call_count()}")
    conn_args = last_connector_call_arguments()
    check("connector tool call carried the exact message and destination",
          conn_args.get("message") == "Sent via the Mistral connector"
          and conn_args.get("channel_id") == "#general",
          json.dumps(conn_args))
    check("connector transport: no Slack MCP call was made",
          send_count() == posts_before_conn,
          f"sends={send_count()} before={posts_before_conn}")

    proc3.stdin.close()
    proc3.wait(timeout=15)

    # 8c. destination display names: a request to a Slack user ID shows the
    # resolved display name in the prompt, while the send still carries the
    # exact raw ID.
    master4, slave4 = pty.openpty()

    def make_ctty4():
        os.setsid()
        fcntl.ioctl(slave4, 0x20007461, 0)

    proc4 = subprocess.Popen(
        [BINARY, "--cli"],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        env=env, pass_fds=(slave4,), preexec_fn=make_ctty4,
    )
    spawned.append(proc4)
    os.close(slave4)
    tty_out4 = bytearray()

    def tty_reader4():
        while True:
            try:
                chunk = os.read(master4, 4096)
            except OSError:
                break
            if not chunk:
                break
            tty_out4.extend(chunk)

    threading.Thread(target=tty_reader4, daemon=True).start()
    responses4 = {}
    resp4_lock = threading.Lock()

    def stdout_reader4():
        for raw in proc4.stdout:
            line = raw.decode(errors="replace").strip()
            if not line:
                continue
            try:
                msg = json.loads(line)
            except json.JSONDecodeError:
                continue
            if "id" in msg:
                with resp4_lock:
                    responses4[msg["id"]] = msg

    threading.Thread(target=stdout_reader4, daemon=True).start()

    def rpc4(id_, method, params=None):
        msg = {"jsonrpc": "2.0", "id": id_, "method": method}
        if params is not None:
            msg["params"] = params
        proc4.stdin.write((json.dumps(msg) + "\n").encode())
        proc4.stdin.flush()

    def wait_response4(id_, timeout=30):
        deadline = time.time() + timeout
        while time.time() < deadline:
            with resp4_lock:
                if id_ in responses4:
                    return responses4[id_]
            time.sleep(0.05)
        raise AssertionError(f"timeout waiting for response id={id_} (display)")

    def wait_tty4(pattern, timeout=30):
        deadline = time.time() + timeout
        while time.time() < deadline:
            if pattern.encode() in tty_out4:
                return
            time.sleep(0.05)
        raise AssertionError(
            f"timeout waiting for tty pattern {pattern!r}; tty tail: {bytes(tty_out4[-1500:])!r}"
        )

    def answer4(text):
        os.write(master4, (text + "\n").encode())

    rpc4(1, "initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                           "clientInfo": {"name": "smoke-display", "version": "0"}})
    wait_response4(1)
    posts_before_disp = send_count()
    rpc4(2, "tools/call", {"name": "request_slack_message", "arguments": {
        "message": "Display name resolution test",
        "destination": "U0DRAKE",
        "reason": "smoke test (destination display)",
        "agent": "smoke-agent",
        "conversation": {"title": "Smoke test conversation", "id_or_link": "smoke-disp",
                         "summary": "Verifying Slack ID resolution in the prompt."},
    }})
    wait_tty4("Destination:   Drake Thomsen (@drake.thomsen) (U0DRAKE)")
    answer4("a")
    r = wait_response4(2)
    v = verdict(r)
    check("display names: prompt shows the resolved name next to the raw ID",
          v.get("status") == "sent",
          json.dumps(v))
    disp_sends = [b for b in send_calls()
                  if send_arguments(b).get("message") == "Display name resolution test"]
    check("display names: the send still carries the exact raw Slack ID",
          len(disp_sends) == 1 and send_arguments(disp_sends[0]).get("channel_id") == "U0DRAKE"
          and send_count() == posts_before_disp + 1,
          f"sends={send_count()}")

    proc4.stdin.close()
    proc4.wait(timeout=15)

    # 8d. Korotovsky stdio transport: SLACK_TRANSPORT=korotovsky must spawn
    # the (mock) korotovsky Slack MCP server as a stdio child process, enable
    # its conversations_add_message tool, and send the approved message
    # through it — the exact text to the exact destination, as text/plain.
    state_dir5 = tempfile.mkdtemp(prefix="sag-smoke-koro-")
    koro_log = os.path.join(state_dir5, "mock-korotovsky.log")
    env5 = dict(os.environ)
    env5["SLACK_GATEWAY_STATE_DIR"] = state_dir5
    env5["SLACK_TRANSPORT"] = "korotovsky"
    env5["SLACK_BOT_TOKEN"] = "xoxb-smoke-test"
    env5["SLACK_WORKSPACE_DOMAIN"] = "smokeworkspace"
    env5["SLACK_STDIO_COMMAND"] = (
        f"env MOCK_KOROTOVSKY_LOG={koro_log} python3 {ROOT}/scripts/mock_korotovsky.py"
    )
    for k in ("HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy", "ALL_PROXY", "all_proxy"):
        env5.pop(k, None)
    env5["NO_PROXY"] = "127.0.0.1,localhost"

    def koro_entries():
        entries = []
        if not os.path.exists(koro_log):
            return entries
        with open(koro_log, encoding="utf-8") as f:
            for line in f:
                line = line.strip()
                if line:
                    entries.append(json.loads(line))
        return entries

    def koro_calls():
        return [e for e in koro_entries() if e.get("tool")]

    def koro_env_record():
        for e in reversed(koro_entries()):
            if e.get("env") is not None:
                return e["env"]
        return {}

    master5, slave5 = pty.openpty()

    def make_ctty5():
        os.setsid()
        fcntl.ioctl(slave5, 0x20007461, 0)

    proc5 = subprocess.Popen(
        [BINARY, "--cli"],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        env=env5, pass_fds=(slave5,), preexec_fn=make_ctty5,
    )
    spawned.append(proc5)
    os.close(slave5)
    tty_out5 = bytearray()

    def tty_reader5():
        while True:
            try:
                chunk = os.read(master5, 4096)
            except OSError:
                break
            if not chunk:
                break
            tty_out5.extend(chunk)

    threading.Thread(target=tty_reader5, daemon=True).start()
    responses5 = {}
    resp5_lock = threading.Lock()

    def stdout_reader5():
        for raw in proc5.stdout:
            line = raw.decode(errors="replace").strip()
            if not line:
                continue
            try:
                msg = json.loads(line)
            except json.JSONDecodeError:
                continue
            if "id" in msg:
                with resp5_lock:
                    responses5[msg["id"]] = msg

    threading.Thread(target=stdout_reader5, daemon=True).start()

    def rpc5(id_, method, params=None):
        msg = {"jsonrpc": "2.0", "id": id_, "method": method}
        if params is not None:
            msg["params"] = params
        proc5.stdin.write((json.dumps(msg) + "\n").encode())
        proc5.stdin.flush()

    def wait_response5(id_, timeout=30):
        deadline = time.time() + timeout
        while time.time() < deadline:
            with resp5_lock:
                if id_ in responses5:
                    return responses5[id_]
            if proc5.poll() is not None:
                raise AssertionError(f"gateway exited early (code {proc5.returncode})")
            time.sleep(0.05)
        raise AssertionError(f"timeout waiting for response id={id_} (korotovsky)")

    def wait_tty5(pattern, timeout=30):
        deadline = time.time() + timeout
        while time.time() < deadline:
            if pattern.encode() in tty_out5:
                return
            time.sleep(0.05)
        raise AssertionError(
            f"timeout waiting for tty pattern {pattern!r}; tty tail: {bytes(tty_out5[-1500:])!r}"
        )

    def answer5(text):
        os.write(master5, (text + "\n").encode())

    posts_before_koro = send_count()
    conn_before_koro = connector_call_count()
    rpc5(1, "initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                            "clientInfo": {"name": "smoke-korotovsky", "version": "0"}})
    wait_response5(1)
    koro_env = koro_env_record()
    check("korotovsky transport: the gateway enabled the server's send tool",
          koro_env.get("SLACK_MCP_ADD_MESSAGE_TOOL") == "true"
          and koro_env.get("SLACK_MCP_XOXB_TOKEN") == "xoxb-smoke-test",
          json.dumps(koro_env))
    rpc5(2, "tools/call", {"name": "request_slack_message", "arguments": {
        "message": "Sent via the korotovsky stdio server",
        "destination": "C0123SMOKE",
        "reason": "smoke test (korotovsky transport)",
        "agent": "smoke-agent",
        "conversation": {"title": "Smoke test conversation", "id_or_link": "smoke-koro",
                         "summary": "Verifying the korotovsky stdio transport."},
    }})
    wait_tty5("Sent via the korotovsky stdio server")
    wait_tty5("Approve [a]")
    answer5("a")
    r = wait_response5(2)
    v = verdict(r)
    check("korotovsky transport: approve returns sent with the built permalink",
          v.get("status") == "sent"
          and v.get("permalink") == "https://smokeworkspace.slack.com/archives/C0123SMOKE/p1700000000000100",
          json.dumps(v))
    calls = [c for c in koro_calls()
             if c.get("arguments", {}).get("text") == "Sent via the korotovsky stdio server"]
    check("korotovsky transport: conversations_add_message got the exact message and destination",
          len(calls) == 1
          and calls[0]["tool"] == "conversations_add_message"
          and calls[0]["arguments"].get("channel_id") == "C0123SMOKE"
          and calls[0]["arguments"].get("content_type") == "text/plain",
          json.dumps(koro_calls()))
    check("korotovsky transport: no Slack MCP or connector call was made",
          send_count() == posts_before_koro and connector_call_count() == conn_before_koro,
          f"sends={send_count()} conn={connector_call_count()}")

    # A Slack user ID destination resolves to a display name in the prompt
    # (via the server's users_search CSV), while the send carries the raw ID.
    rpc5(3, "tools/call", {"name": "request_slack_message", "arguments": {
        "message": "Korotovsky display name resolution",
        "destination": "U0123SMOKE",
        "reason": "smoke test (korotovsky display)",
        "agent": "smoke-agent",
        "conversation": {"title": "Smoke test conversation", "id_or_link": "smoke-koro-disp",
                         "summary": "Verifying korotovsky ID resolution in the prompt."},
    }})
    wait_tty5("Destination:   Drake Thomsen (@drake.thomsen) (U0123SMOKE)")
    answer5("a")
    r = wait_response5(3)
    v = verdict(r)
    disp_calls = [c for c in koro_calls()
                  if c.get("arguments", {}).get("text") == "Korotovsky display name resolution"]
    check("korotovsky transport: prompt resolves the name, the send carries the raw ID",
          v.get("status") == "sent"
          and len(disp_calls) == 1
          and disp_calls[0]["arguments"].get("channel_id") == "U0123SMOKE",
          json.dumps(v))

    proc5.stdin.close()
    proc5.wait(timeout=15)

    # 9. invalid params -> JSON-RPC error
    print()
    if failures:
        print(f"SMOKE TEST FAILED: {len(failures)} failure(s): {failures}")
        sys.exit(1)
    print("SMOKE TEST PASSED: all checks green")
finally:
    for p in spawned + bridges:
        if p.poll() is None:
            p.kill()
    server.shutdown()
