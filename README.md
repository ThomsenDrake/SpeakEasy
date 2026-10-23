# Slack Approval Gateway

A small, local desktop app (Rust) that is the **approval gateway for
agent-initiated Slack messages**. Agents call one MCP tool,
`request_slack_message`; a human approves, denies, or gives feedback in a
desktop popup; **only the app ever sends to Slack**. Closing or ignoring the
popup never authorizes sending.

```
agent ──MCP/stdio──> slack-approval-gateway ──human popup──> approve / deny / feedback
                            │ (only credential holder)
                            └──────────> Slack Web API (chat.postMessage)
```

## What agents can ask for

The MCP tool `request_slack_message` requires:

| Field | Meaning |
|---|---|
| `message` | The exact proposed message text. Sent verbatim on approval. |
| `destination` | Intended Slack recipient: channel (`#name` or ID) or user (`@name` or ID). |
| `thread_ts` | Optional. Destination Slack thread timestamp for a reply. |
| `reason` | Why the agent wants to send this. |
| `agent` | Identity of the requesting agent. |
| `conversation` | Context from the originating agent conversation: `title`, `id_or_link`, `summary`. |

The popup shows all of the above, clearly separating the **Slack
destination** (where the message would go) from the **originating agent
conversation** (context only, not a Slack thread), and offers three actions:
**Approve**, **Deny**, **Give feedback**.

Return values to the agent:

| `status` | Meaning |
|---|---|
| `sent` | The exact approved message was sent to the exact approved destination. Includes `ts`, `permalink`, and `duplicate` (true when this was a retry of an already-sent identical request — nothing was resent). |
| `denied` | Nothing was sent. The same purpose is blocked for `cooldown_remaining_secs` (600). |
| `feedback` | Nothing was sent. The human's `feedback` text is returned; the agent may revise and submit a new request, which requires a new approval. |
| `cooldown_blocked` | A matching deny cooldown is active. No popup was shown, nothing was sent; `cooldown_remaining_secs` tells the agent how long to wait. |
| `error` | Nothing was sent. See `error` for details. |

Semantics:

- **Approval is per-request.** Any change to the message content or the
  destination requires fresh approval.
- **No duplicate sends.** An identical retry of an approved request returns
  the cached result (`duplicate: true`) without resending — repeated clicks
  or retries cannot double-send.
- **Deny blocks the purpose for 10 minutes**, including reworded requests
  and retries from *other* participating agents. The block is keyed by
  destination + message-content similarity — never by agent identity, reason
  string, or request ID — and is persisted across restarts. While a block is
  active, matching requests get `cooldown_blocked` with the remaining time
  and no popup.
- **Feedback sends nothing** and returns the text to the requesting agent.
- **Closing or ignoring the popup never approves.** If the app quits with
  requests pending, the agents receive an error; nothing is sent.

## Requirements

- macOS (developed and tested on macOS arm64; Linux should work)
- Rust 1.98+ (`rustup`), Node 24+ (`npm`)
- A Slack bot token
- Agents that can run an MCP stdio server on the same machine

## Setup

### 1. Slack authentication

1. Create a Slack app at <https://api.slack.com/apps> (or use an existing
   workspace bot).
2. Under **OAuth & Permissions**, add bot token scopes:
   `chat:write` and `chat:write.public` (`chat.getPermalink` needs no extra
   scope).
3. Install the app to your workspace and invite the bot to every channel it
   may post to (`/invite @your-bot`).
4. Give the token to the gateway — **never to the agents** — in one of two
   ways:
   - set `SLACK_BOT_TOKEN` in the gateway's environment when launching it, or
   - write it to `~/.slack-approval-gateway/slack_token` (one line,
     `chmod 600`).

Optional environment variables:

- `SLACK_WORKSPACE_DOMAIN` — used only for fallback permalink construction
  when `chat.getPermalink` is unavailable.
- `SLACK_API_BASE` — override the Slack API endpoint (used by tests).
- `SLACK_GATEWAY_STATE_DIR` — state directory (default
  `~/.slack-approval-gateway`). Holds `cooldowns.json`, `sent.json`, and lock
  files; everything persists across restarts.

### 2. Launching the app

```sh
# Build the frontend once (Tauri embeds ui/dist into the binary)
cd ui && npm install && npm run build && cd ..

cargo build --release
./target/release/slack-approval-gateway          # GUI mode (default)
./target/release/slack-approval-gateway --cli    # terminal approval (headless/SSH)
```

- **GUI mode**: a Tauri webview popup appears for each request, built with
  the **Mistral design system (`@mistral/ui`)** — React 19 + Tailwind 4,
  with the DS vendored under `ui/shared/` (from Mistral's
  `product-prototypes` template). The MCP server runs on a background
  thread speaking newline-delimited JSON-RPC on stdio. stdout carries
  protocol only; diagnostics go to stderr. The binary is self-contained:
  the UI is embedded (Tauri `custom-protocol` feature), so no dev server is
  needed at runtime.
- **Development**: after changing `ui/`, run `npm run build` in `ui/` and
  rebuild the binary; `npm run dev` in `ui/` serves the frontend with hot
  reload for iterating on components.
- **CLI mode**: approvals happen on the controlling terminal
  (`a` approve, `d` deny, `f <feedback>`). Concurrent prompts are
  serialized so an answer always reaches the right request.

### 3. Agent integration

Configure each participating agent's MCP client with a stdio server, e.g.:

```json
{
  "mcpServers": {
    "slack-approval-gateway": {
      "command": "/absolute/path/to/slack-approval-gateway",
      "args": []
    }
  }
}
```

The gateway must be running before the agent starts (agents spawn it as a
child process over stdio). Agents call `request_slack_message` with the
fields above and receive one of the verdicts.

## Enforcement boundaries (exclusive sending path)

The gateway is the **only** component that holds Slack credentials and the
only path that sends:

- Agents receive **no** Slack token, no Slack MCP server, and no
  `chat:write`-capable API access. Their only way to reach Slack is this
  gateway's single approval-gated tool.
- Configure the agent harness's tool permissions to exclude direct HTTP /
  web / browser-automation tools (or run agents without network egress
  except the gateway process).
- Prefer running agents as a **separate OS user** that cannot read the
  gateway's state directory (which may contain `slack_token`). An agent
  running as the same user could read the token file directly.
- MCP is stdio, so agents must run on the same machine as the gateway (or
  reach it over a transport you trust).
- Keep the token file `chmod 600`; prefer the env var over the file when
  the gateway is launched by a process manager.

### Environment limitations (honest list)

- The gateway gates **agent-initiated sends that go through its MCP tool**.
  It cannot stop an agent that already possesses the bot token, has
  unrestricted network access, or can drive a browser — the mitigations
  above reduce, but on a single-user machine cannot fully eliminate, this.
- Purpose blocking is a **heuristic**: destination + normalized
  message-content similarity (character-level Levenshtein ratio ≥ 0.85, or
  token-set Sørensen–Dice ≥ 0.7). It is not a semantic guarantee. Known gaps:
  messages whose accents are encoded as separate combining marks (short,
  heavily accented messages), look-alike letters from other alphabets
  (e.g. Cyrillic "а"), a channel's ID vs its `#name`, and requests already
  queued when a similar request was denied (they still get their own popup).
- A human can of course still send anything manually; the gateway gates
  agents, not people. On macOS, any app with **Accessibility** permission
  can press the popup's buttons, as it can in any app.
- Cooldowns and dedupe rely on the system clock; large clock jumps can
  shorten a cooldown.
- Approval popups require the desktop app to be running with access to the
  window server; use `--cli` for headless hosts.

## Development

```sh
cd ui && npm install && npm run build && cd ..   # frontend (required once)
cargo build --release    # first build compiles Tauri/wry; takes minutes
cargo test               # 55 lib tests + 13 MCP e2e tests + 12 Slack client tests
cargo test -- --ignored  # also the 15s HTTP timeout test
python3 scripts/smoke_test.py   # drives the real binary end to end (mock Slack + pty)
```

`docs/SPEC.md` is the module contract (`core` = gateway/cooldowns/dedupe,
`slack` = the only credential holder, `mcp` = stdio JSON-RPC server,
`ui` = the Tauri webview bridge). GUI behavior is verified by build + spec
review plus the IPC bridge unit tests; the decision flows are covered by
the test suite and the smoke script.

## Layout

```
├── src/core.rs    gateway: validation, purpose fingerprinting, persisted
│                  deny cooldowns, persisted sent-record dedupe, decision flow
├── src/slack.rs   Slack Web API client (only place credentials exist)
├── src/mcp.rs     MCP stdio server (one tool: request_slack_message)
├── src/ui.rs      Tauri webview bridge (decision provider + IPC commands)
├── src/main.rs    wiring, state dir, --cli fallback provider
├── ui/            the approval popup frontend (React + Tailwind 4 +
│   │              the vendored Mistral @mistral/ui design system)
│   ├── src/App.tsx        approval popup (DS components)
│   └── shared/            vendored @mistral/ui components + design tokens
├── tests/         unit/integration/e2e tests (mock Slack server, scripted humans)
├── scripts/smoke_test.py  end-to-end test of the real binary
├── docs/SPEC.md   behavioral + module contract
├── tauri.conf.json, capabilities/, icons/, build.rs   Tauri shell
└── README.md
```
