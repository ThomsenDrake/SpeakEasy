#!/usr/bin/env python3
"""Get a Slack user OAuth token for the slack-approval-gateway.

The gateway's default Slack transport is the official Slack MCP server
(https://mcp.slack.com/mcp, remote MCP over Streamable HTTP). It
authenticates with a Slack **user token** (xoxp-…), obtained through the
confidential OAuth 2.0 flow Slack documents for MCP clients:

  authorize  https://slack.com/oauth/v2_user/authorize
  exchange   https://slack.com/api/oauth.v2.user.access

This script runs that flow locally: it starts a callback server on
127.0.0.1 (default port 3118), opens the Slack authorize page in your
browser, exchanges the code for a user token, verifies it against the MCP
endpoint, and writes it to <state-dir>/slack_mcp_token (chmod 600). The
gateway then picks it up from there (or from SLACK_MCP_TOKEN).

Prerequisites (one-time, in your Slack app at https://api.slack.com/apps):
  - the app must be directory-published or internal (only those may use MCP);
  - register the redirect URL http://127.0.0.1:3118/callback (or the exact
    --redirect-uri you pass) under OAuth & Permissions > Redirect URLs;
  - add the user scopes (defaults below) under OAuth & Permissions >
    User Token Scopes.

Usage:
  scripts/slack_mcp_oauth.py --client-id 123.456 --client-secret s3cr3t
  scripts/slack_mcp_oauth.py            # reads SLACK_CLIENT_ID/SLACK_CLIENT_SECRET

Scopes default to what the gateway needs: chat:write (send messages),
users:read (resolve user IDs to display names in the approval popup) and
channels:read/groups:read/im:read/mpim:read (resolve channel IDs).
"""
import argparse
import http.server
import json
import os
import secrets
import sys
import threading
import urllib.error
import urllib.parse
import urllib.request
import webbrowser

AUTHORIZE_URL = "https://slack.com/oauth/v2_user/authorize"
EXCHANGE_URL = "https://slack.com/api/oauth.v2.user.access"
DEFAULT_MCP_URL = "https://mcp.slack.com/mcp"
DEFAULT_SCOPES = "chat:write,users:read,channels:read,groups:read,im:read,mpim:read"


def default_state_dir():
    env = os.environ.get("SLACK_GATEWAY_STATE_DIR")
    if env:
        return env
    return os.path.join(os.path.expanduser("~"), ".slack-approval-gateway")


def exchange_code(client_id, client_secret, code, redirect_uri):
    """Exchange the authorization code for a user token (xoxp-…)."""
    form = urllib.parse.urlencode({
        "client_id": client_id,
        "client_secret": client_secret,
        "code": code,
        "redirect_uri": redirect_uri,
    }).encode()
    request = urllib.request.Request(
        EXCHANGE_URL, data=form, method="POST",
        headers={"Content-Type": "application/x-www-form-urlencoded"},
    )
    try:
        with urllib.request.urlopen(request, timeout=20) as response:
            payload = json.loads(response.read() or b"{}")
    except urllib.error.HTTPError as e:
        detail = e.read().decode(errors="replace")
        raise SystemExit(f"slack_mcp_oauth: token exchange failed: HTTP {e.code} {detail}")
    except urllib.error.URLError as e:
        raise SystemExit(f"slack_mcp_oauth: token exchange failed: {e.reason}")
    if not payload.get("ok"):
        raise SystemExit(f"slack_mcp_oauth: token exchange failed: {payload.get('error')}")
    token = payload.get("access_token") or ""
    if not token.startswith("xoxp-"):
        raise SystemExit(
            "slack_mcp_oauth: expected a user token (xoxp-…), got something else; "
            "make sure the app requests user scopes (user_scope)"
        )
    return token


def verify_token(token, mcp_url):
    """Run the MCP initialize handshake with the token, like the gateway does."""
    body = json.dumps({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "slack-mcp-oauth-verify", "version": "0"},
        },
    }).encode()
    request = urllib.request.Request(mcp_url, data=body, method="POST", headers={
        "Authorization": f"Bearer {token}",
        "Content-Type": "application/json",
        "Accept": "application/json, text/event-stream",
    })
    try:
        with urllib.request.urlopen(request, timeout=20) as response:
            payload = json.loads(response.read() or b"{}")
    except urllib.error.HTTPError as e:
        detail = e.read().decode(errors="replace")[:300]
        raise SystemExit(
            f"slack_mcp_oauth: the MCP endpoint rejected the token: HTTP {e.code} {detail}\n"
            "Check that the app is directory-published or internal, that the redirect "
            "URL is registered, and that the scopes were granted on the consent screen."
        )
    except urllib.error.URLError as e:
        raise SystemExit(f"slack_mcp_oauth: cannot reach {mcp_url}: {e.reason}")
    server = (payload.get("result") or {}).get("serverInfo") or {}
    if not server:
        raise SystemExit(f"slack_mcp_oauth: unexpected initialize reply: {json.dumps(payload)[:300]}")
    return server


def write_token_file(path, token):
    """Write the token with owner-only permissions (no world-readable window)."""
    os.makedirs(os.path.dirname(path), exist_ok=True)
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w") as f:
        f.write(token + "\n")
    os.chmod(path, 0o600)


def main():
    parser = argparse.ArgumentParser(
        description="Get a Slack user OAuth token for the slack-approval-gateway "
                    "(Slack MCP server transport)."
    )
    parser.add_argument("--client-id", default=os.environ.get("SLACK_CLIENT_ID"),
                        help="Slack app client ID (or SLACK_CLIENT_ID)")
    parser.add_argument("--client-secret", default=os.environ.get("SLACK_CLIENT_SECRET"),
                        help="Slack app client secret (or SLACK_CLIENT_SECRET)")
    parser.add_argument("--state-dir", default=default_state_dir(),
                        help="gateway state dir (default: %(default)s)")
    parser.add_argument("--port", type=int, default=3118,
                        help="local callback port (default: %(default)s)")
    parser.add_argument("--scopes", default=DEFAULT_SCOPES,
                        help="comma-separated user scopes (default: %(default)s)")
    parser.add_argument("--mcp-url", default=DEFAULT_MCP_URL,
                        help="MCP endpoint used for verification (default: %(default)s)")
    parser.add_argument("--no-browser", action="store_true",
                        help="print the authorize URL instead of opening a browser")
    args = parser.parse_args()

    if not args.client_id or not args.client_secret:
        raise SystemExit(
            "slack_mcp_oauth: need the Slack app's client ID and secret\n"
            "  pass --client-id/--client-secret, or set SLACK_CLIENT_ID/SLACK_CLIENT_SECRET\n"
            "  create the app at https://api.slack.com/apps (it must be directory-published\n"
            "  or internal to use the Slack MCP server)"
        )

    redirect_uri = f"http://127.0.0.1:{args.port}/callback"
    state = secrets.token_urlsafe(24)
    query = urllib.parse.urlencode({
        "client_id": args.client_id,
        "user_scope": args.scopes,
        "redirect_uri": redirect_uri,
        "state": state,
    })
    authorize_url = f"{AUTHORIZE_URL}?{query}"

    result = {}

    class CallbackHandler(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            parsed = urllib.parse.urlparse(self.path)
            params = urllib.parse.parse_qs(parsed.query)
            if parsed.path != "/callback":
                self.send_response(404)
                self.end_headers()
                return
            if params.get("state", [None])[0] != state:
                self.send_response(400)
                self.end_headers()
                self.wfile.write(b"state mismatch; close this window and retry")
                return
            error = params.get("error", [None])[0]
            if error:
                result["error"] = error
            else:
                result["code"] = params.get("code", [None])[0]
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.end_headers()
            self.wfile.write(
                b"<html><body><h2>Slack approval gateway: authorized.</h2>"
                b"<p>You can close this window and return to the terminal.</p></body></html>"
            )

        def log_message(self, *args):
            pass

    server = http.server.HTTPServer(("127.0.0.1", args.port), CallbackHandler)
    threading.Thread(target=server.serve_forever, daemon=True).start()

    print("slack_mcp_oauth: Slack MCP server user-token OAuth")
    print(f"  redirect URI : {redirect_uri}")
    print("  (register it under OAuth & Permissions > Redirect URLs in your Slack app)")
    print(f"  scopes       : {args.scopes}")
    print()
    print(f"Authorize URL:\n  {authorize_url}\n")
    if not args.no_browser:
        print("Opening the authorize page in your browser…")
        webbrowser.open(authorize_url)
    print("Waiting for Slack to redirect back… (authorize the app in the browser)")

    try:
        while "code" not in result and "error" not in result:
            threading.Event().wait(0.2)
    except KeyboardInterrupt:
        raise SystemExit("slack_mcp_oauth: cancelled")
    finally:
        server.shutdown()

    if "error" in result:
        raise SystemExit(f"slack_mcp_oauth: Slack returned an error: {result['error']}")
    if not result.get("code"):
        raise SystemExit("slack_mcp_oauth: no authorization code in the callback")

    print("Exchanging the authorization code for a user token…")
    token = exchange_code(args.client_id, args.client_secret, result["code"], redirect_uri)

    print(f"Verifying the token against {args.mcp_url} (MCP initialize)…")
    server_info = verify_token(token, args.mcp_url)
    print(f"  MCP server   : {server_info.get('name')} {server_info.get('version', '')}".rstrip())

    token_path = os.path.join(args.state_dir, "slack_mcp_token")
    write_token_file(token_path, token)
    print()
    print(f"Wrote the Slack user token to {token_path} (chmod 600)")
    print("The gateway uses it from there (or from SLACK_MCP_TOKEN); restart the")
    print("gateway (launchctl kickstart -k gui/$(id -u)/com.speakeasy.slack-approval-gateway)")
    print("to pick it up.")


if __name__ == "__main__":
    main()
