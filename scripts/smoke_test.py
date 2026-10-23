#!/usr/bin/env python3
"""End-to-end smoke test of the real slack-approval-gateway binary.

Runs `slack-approval-gateway --cli` with:
  - stdin/stdout/stderr = pipes (the MCP stdio transport, like a real agent)
  - a pty as the controlling terminal (so the CLI provider's /dev/tty is the
    pty, like a human's terminal)
  - SLACK_API_BASE pointed at a local mock Slack Web API

Covers: MCP handshake, tools/list, approve (exact send + permalink),
duplicate retry (no resend), deny (cooldown), reworded retry during cooldown
(blocked, no prompt), feedback (no send), and two concurrent requests whose
prompts are answered in order (CLI mutex: answers reach the right requests).
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


# ---------------------------------------------------------------- mock Slack
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
        else:
            resp = {"ok": False, "error": "unknown_method"}
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

    # 9. invalid params -> JSON-RPC error
    print()
    if failures:
        print(f"SMOKE TEST FAILED: {len(failures)} failure(s): {failures}")
        sys.exit(1)
    print("SMOKE TEST PASSED: all checks green")
finally:
    for p in (proc,):
        if p.poll() is None:
            p.kill()
    server.shutdown()
