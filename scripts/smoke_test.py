#!/usr/bin/env python3
"""End-to-end smoke test of the real slack-approval-gateway binary.

Runs `slack-approval-gateway --cli` with:
  - stdin/stdout/stderr = pipes (the MCP stdio transport, like a real agent)
  - a pty as the controlling terminal (so the CLI provider's /dev/tty is the
    pty, like a human's terminal)
  - SLACK_API_BASE pointed at a local mock Slack Web API

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
end to end (exact message sent through the connector tool call, no direct
Slack API call).
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

ROOT = "/Users/drake.thomsen/.superset/worktrees/SpeakEasy/use-superset-orchestrate-skill-d12645b9"
BINARY = os.path.join(ROOT, "target/debug/slack-approval-gateway")

failures = []


def check(name, cond, detail=""):
    status = "PASS" if cond else "FAIL"
    print(f"[{status}] {name}" + (f" — {detail}" if detail and not cond else ""))
    if not cond:
        failures.append(name)


# ------------------------------------------------- mock Slack + Mistral API
# One HTTP server plays both the Slack Web API (direct transport) and the
# Mistral platform API (connector transport).
class Handler(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        length = int(self.headers.get("Content-Length", 0))
        body = json.loads(self.rfile.read(length) or b"{}")
        path = self.path.split("?")[0]
        self.server.requests.append((path, dict(self.headers), body))
        if path == "/chat.postMessage":
            resp = {"ok": True, "channel": "C123", "ts": "1700000000.000100"}
        elif path == "/chat.getPermalink":
            resp = {"ok": True, "permalink": "https://mock.slack.com/archives/C123/p1700000000000100"}
        elif path.startswith("/v1/connectors/") and path.endswith("/call"):
            # MCP CallToolResult shaped like the real slack connector: a JSON
            # string inside the text block.
            resp = {
                "content": [{"type": "text", "text":
                             "{\"message_link\":\"https://mock.slack.com/archives/C123/p1700000000000100\",\"message_context\":{\"message_ts\":\"1700000000.000100\",\"channel_id\":\"C123\"}}"}],
            }
        else:
            resp = {"ok": False, "error": "unknown_method"}
        data = json.dumps(resp).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
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


def post_count():
    return sum(1 for p, _, _ in server.requests if p == "/chat.postMessage")


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
env["SLACK_BOT_TOKEN"] = "xoxb-smoke-test"
env["SLACK_API_BASE"] = f"http://127.0.0.1:{port}"
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
    posts = [b for p, _, b in server.requests if p == "/chat.postMessage"]
    check("mock Slack received exactly one postMessage", post_count() == 1, f"count={post_count()}")
    check("postMessage carried the exact message, destination and thread",
          posts and posts[0].get("text") == "Hello from the smoke test"
          and posts[0].get("channel") == "#general"
          and posts[0].get("thread_ts") == "1699999999.000001",
          json.dumps(posts[:1]))

    # 3. identical retry: cached duplicate, no resend
    tool_call(4, "Hello from the smoke test", thread_ts="1699999999.000001")
    r = wait_response(4)
    v = verdict(r)
    check("identical retry returns duplicate without resending",
          v.get("status") == "sent" and v.get("duplicate") is True and post_count() == 1,
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
          and post_count() == 1,
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
          and post_count() == 1,
          json.dumps(v))

    # 7. concurrency: two requests, prompts serialized, answers matched
    tool_call(8, "Concurrent request alpha")
    tool_call(9, "Concurrent request beta")
    wait_tty("Concurrent request alpha")
    answer("a")  # approves alpha
    r = wait_response(8)
    v8 = verdict(r)
    wait_tty("Concurrent request beta")
    answer("f not now")  # feedback for beta
    r = wait_response(9)
    v9 = verdict(r)
    check("concurrent requests do not mix up decisions",
          v8.get("status") == "sent" and not v8.get("duplicate")
          and v9.get("status") == "feedback" and v9.get("feedback") == "not now"
          and post_count() == 2,
          f"v8={json.dumps(v8)} v9={json.dumps(v9)} posts={post_count()}")
    alpha_posts = [b for p, _, b in server.requests
                   if p == "/chat.postMessage" and b.get("text") == "Concurrent request alpha"]
    check("alpha was the message actually sent", len(alpha_posts) == 1)

    # 7b. bridge: a second agent process (default mode, no --cli) must not
    # start its own instance — it bridges to the running one over the
    # state-dir socket and its MCP session is served by that instance.
    bridges = []

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

    posts_before = post_count()
    bridge_tool_call(brpc1, 102, "Bridged request one")
    wait_tty("Bridged request one")
    answer("a")
    r = wait_b1(102)
    v = verdict(r)
    check("bridge: approve through the bridge sends via the instance",
          v.get("status") == "sent" and post_count() == posts_before + 1,
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
          and post_count() == posts_before + 1,
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

    posts_before_conn = post_count()
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
    check("connector transport: no direct Slack API call was made",
          post_count() == posts_before_conn,
          f"posts={post_count()} before={posts_before_conn}")

    proc3.stdin.close()
    proc3.wait(timeout=15)

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
