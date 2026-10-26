# SpeakEasy

**Let your coding agents draft Slack messages. Nothing is sent until you approve it.**

SpeakEasy is a small macOS app that sits between your coding agents and
Slack. Agents get one Slack tool, `request_slack_message`. Every call opens a
window with the exact message, where it would go, and why the agent wants to
send it. Nothing reaches Slack until you click **Approve**. Then SpeakEasy
posts exactly that text to exactly that place, once.

<p align="center">
  <img src="docs/screenshots/approval-popup.png" width="600" alt="The SpeakEasy window: claude-code asks to post a deploy announcement to #eng-deploys. It shows the exact message, the destination, the reason, the agent's conversation, and Give feedback, Deny, and Approve buttons.">
</p>

- **One install covers your agents.** The installer registers SpeakEasy with
  Claude Code, Codex, Mistral Vibe, OpenCode, goose, Gemini CLI, Copilot CLI,
  Zed, Amp, and [more](#supported-agents).
- **Agents aren't given the credential.** SpeakEasy keeps it in its own
  folder and sends only what you approve. (An agent running as your user
  could still read that folder; see [Security](#security).)
- **Deny means stop.** A denied message is blocked for 10 minutes, including
  close rewordings and retries from other agents.
- **Feedback instead of a dead end.** Tell the agent what to change. It gets
  your note and can ask again.

## How it works

1. An agent calls `request_slack_message` with the message, the destination
   (a channel or a person), its reason, and a summary of the conversation
   it's working in.
2. The SpeakEasy window comes to the front with the request.
3. You decide. The agent's tool call waits until you do.

| You click | In Slack | The agent gets back |
|---|---|---|
| **Approve** | The exact text is posted to the exact destination, once | `sent`, with a link to the message |
| **Deny** | Nothing | `denied`. Similar requests to that destination are blocked for 10 minutes. |
| **Give feedback**, then **Submit feedback** | Nothing | `feedback`, with your note, so it can revise and ask again |

<p align="center">
  <img src="docs/screenshots/approval-popup-feedback.png" width="480" alt="The same request with Give feedback open: a note for the agent is typed in the feedback field above the Submit feedback button.">
</p>

Closing the window doesn't approve or deny anything. It only hides the
window, and the request keeps waiting. To bring it back, choose **Show
approval window** from SpeakEasy's menu-bar icon or click its Dock icon.

All your agents share one SpeakEasy. Requests from every agent queue in the
same window, and the header shows how many are waiting.

## Install

SpeakEasy builds from source and installs as a login service. Its binary,
MCP server, and state folder keep an older name, `slack-approval-gateway`.

**Requirements:** macOS with the Xcode Command Line Tools
(`xcode-select --install`), Rust (stable, via [rustup](https://rustup.rs)),
and Python 3.

### 1. Get the code

```sh
git clone https://github.com/ThomsenDrake/SpeakEasy.git
cd SpeakEasy
```

### 2. Choose a transport

The transport is how SpeakEasy posts to Slack. Agents can't tell the
transports apart. The choice decides which credential SpeakEasy stores and
whose name the messages appear under.

| | **Slack MCP** (default) | **Mistral connector** |
|---|---|---|
| Sends through | Slack's official MCP server, `mcp.slack.com` | Mistral's Slack connector |
| Messages appear from | you | the Slack user who connected Slack in Mistral (usually you) |
| SpeakEasy stores | a Slack user token | a Mistral API key |
| You need | a Slack app of your own | a Mistral account with Slack connected |

Use **Slack MCP** unless you already use Mistral's Slack connector. The
**Mistral connector** keeps the Slack token off your Mac, but the Mistral API
key that SpeakEasy stores can post to Slack too, so protect it the same way.

> **Korotovsky server (not working yet).** The code also has a third
> transport for [korotovsky/slack-mcp-server](https://github.com/korotovsky/slack-mcp-server),
> an open-source Slack MCP server that SpeakEasy would run on your Mac. As
> shipped, it can't start. Its default command names an npm package that
> doesn't exist (`@korotovsky/slack-mcp-server`; the real package is
> `slack-mcp-server`), and the login service can't find `npx`. Use one of
> the two transports above.

### 3. Set up the credential

<details open>
<summary><b>Slack MCP (default)</b></summary>

1. Create a Slack app at <https://api.slack.com/apps> (**From scratch**, in
   your workspace). Don't turn on public distribution: Slack only lets
   internal apps and apps published in the Slack Marketplace use its MCP
   server.
2. Turn on MCP for the app. In the app's settings sidebar, open **Agents**
   and turn on **Slack Model Context Protocol (MCP) Server**. (Older Slack
   pages call these **Agents & AI Apps** and **Model Context Protocol**.)
   Your workspace admin may also need to approve MCP access. Do this before
   step 4.
3. In the app's **OAuth & Permissions** page:
   - add the redirect URL `http://127.0.0.1:3118/callback`, exactly as
     written (to Slack, `localhost` is a different host);
   - under **User Token Scopes**, add `chat:write` (to send) and
     `users:read`, `channels:read`, `groups:read`, `im:read`, `mpim:read`
     (to show names instead of IDs in the window).
4. Run the OAuth helper with the client ID and secret from the app's
   **Basic Information** page:

   ```sh
   scripts/slack_mcp_oauth.py --client-id YOUR_CLIENT_ID --client-secret YOUR_CLIENT_SECRET
   ```

   It opens Slack in your browser, trades the authorization for a user token
   (`xoxp-…`), checks the token against `mcp.slack.com`, and saves it to
   `~/.slack-approval-gateway/slack_mcp_token` (mode 600).

The helper asks for all six scopes. To ask for fewer, pass
`--scopes chat:write`. The window then shows raw IDs instead of names.

If Slack won't save the redirect URL, skip the helper. Click **Install to
Workspace** on the **OAuth & Permissions** page, copy the **User OAuth
Token** (`xoxp-…`), and pass it to the installer with
`--slack-mcp-token`.

If SpeakEasy is already installed, restart it after you get a new token:
`launchctl kickstart -k gui/$(id -u)/com.speakeasy.slack-approval-gateway`.

</details>

<details>
<summary><b>Mistral connector</b></summary>

1. Connect Slack in Mistral. In the console, open **Context → Connectors**
   (<https://console.mistral.ai/build/connectors>), choose **Slack**, open
   its **Credentials** tab, and click **+ Add credentials**. Connecting at
   <https://chat.mistral.ai/connections> also works. The admin panel's
   **Connect to Slack** button installs a different integration and doesn't
   count.
2. Create a Mistral API key and set its **Connector access scope** to
   **Private and shared connectors**. The default, **Shared connectors
   only**, can't use a Slack connection that you made yourself. You can't
   change a key's scope later.

Messages post as the Slack user who connected Slack in step 1.

</details>

### 4. Run the installer

Run the command for your transport.

Slack MCP (it uses the token the helper saved):

```sh
scripts/install.sh
```

Mistral connector:

```sh
scripts/install.sh --transport mistral-connector --mistral-api-key YOUR_API_KEY
```

Your shell keeps a copy of any key or token you type in a command, so
delete that line from its history afterwards.

The installer:

1. Builds the app (the window UI is pure Rust — no Node/npm step), and
   installs the binary to `~/.local/bin/slack-approval-gateway`.
2. Creates `~/.slack-approval-gateway/` (mode 700) and stores the credential
   there (mode 600). With `--transport`, it also saves your choice in
   `slack_transport`. SpeakEasy doesn't copy the credential anywhere else.
3. Installs a LaunchAgent, `com.speakeasy.slack-approval-gateway`, that
   starts SpeakEasy at login, and starts it now.
4. Registers the MCP server `slack-approval-gateway` with every agent it
   finds ([table below](#supported-agents)). It also adds a short block to
   each agent's instructions file: read Slack through your Slack MCP server
   or Mistral connector, and send only through `request_slack_message`.
5. Audits those agents' config files for other ways to post to Slack, and
   prints what it finds. See [Security](#security).
6. Checks that the service answers and that each registration worked.

At the end it lists next steps. The first one always repeats the credential
step. If you already stored a credential, skip it.

The installer backs up every file before its first change, to
`<file>.bak-sag`. You can run it again at any time: it keeps existing
registrations and credential files unless you pass a new credential. Each
run restarts SpeakEasy, which disconnects agents that are already running,
so restart them afterwards.

If no credential is stored and you don't pass one, the installer writes a
placeholder, and SpeakEasy can't send until you replace it. With Slack MCP,
SpeakEasy doesn't start at all until then, and agents report that the
`slack-approval-gateway` server failed to start. A token the helper saved is
kept.

| Option | Effect |
|---|---|
| `--transport slack-mcp` or `--transport mistral-connector` | Choose the transport. The choice is saved, so later runs keep it. |
| `--slack-mcp-token xoxp-…` | Store a Slack user token (Slack MCP). |
| `--mistral-api-key KEY` | Store a Mistral API key (Mistral connector). |
| `--enforce` | Also remove token-based Slack MCP servers from agent configs. See [Security](#what-the-installers-audit-reports). |
| `--dry-run` | Show what would change, and change nothing. It doesn't build, so it needs an existing `target/release` binary: run `cargo build --release` first. |
| `--skip-build` | Use the existing `target/release` binary. |
| `--uninstall [--purge]` | See [Uninstall](#uninstall). |

#### Supported agents

The installer finds most agents by their command or their config folder.
Claude Code, Codex, and Vibe must be on your `PATH`, because the installer
registers them through their own CLIs. Gemini CLI and Copilot CLI are found
only by their config folder, so run each one once before you install.

| Agent | MCP server added to | Instruction block added to |
|---|---|---|
| Claude Code | `~/.claude.json`, user scope | `~/.claude/CLAUDE.md` |
| Codex | `~/.codex/config.toml` | `~/.codex/AGENTS.md` |
| Mistral Vibe | `~/.vibe/config.toml` | `~/.vibe/AGENTS.md` |
| OpenCode | `~/.config/opencode/opencode.json` | `~/.config/opencode/AGENTS.md` |
| goose | `~/.config/goose/config.yaml` | `~/.config/goose/.goosehints` |
| pi | `~/.pi/agent/settings.json` | `~/.pi/agent/AGENTS.md` |
| pdev (paradev) | `~/.paradev/agent/settings.json` | `~/.paradev/agent/AGENTS.md` |
| Gemini CLI | `~/.gemini/settings.json` | `~/.gemini/GEMINI.md` |
| Copilot CLI | `~/.copilot/mcp-config.json` | `~/.copilot/copilot-instructions.md` |
| Zed | `~/.config/zed/settings.json` | `~/.config/zed/rules/slack-gateway.md`, which Zed doesn't read. Copy the block into `~/.config/zed/AGENTS.md`. |
| Amp | `~/.config/amp/settings.json` | none |
| Hermes Agent | `~/.hermes/config.yaml` | `~/.hermes/AGENTS.md`, which Hermes doesn't read. Copy the block into `~/.hermes/SOUL.md`. |

A block you copy by hand isn't removed when you uninstall.

Tools that launch these agent CLIs as your user, such as
[Superset](https://superset.sh) workspaces, pick up the registration with no
extra setup. The installer doesn't register Devin's CLI. To add it, or any
other MCP client, see [Register an agent by hand](#register-an-agent-by-hand).

### 5. Try it

1. Restart your agents, or start a new session.
2. Find your Slack member ID: in Slack, open your profile, click **⋮**, and
   choose **Copy member ID**. It looks like `U0123ABCDE`.
3. Ask an agent, with your own ID:

   > Send a Slack DM to U0123ABCDE that says "SpeakEasy test".

   The SpeakEasy window appears with you as the destination. Click
   **Approve**. The DM arrives, and the agent gets back a link to it.
4. Ask for a different DM, such as "Checking that Deny works", and click
   **Deny**. Then ask the agent to send it again with one word added. It
   gets `cooldown_blocked`, and no window appears.

Asking for "SpeakEasy test" again doesn't open a window either: SpeakEasy
returns the first result with `duplicate: true`.

## Day to day

**The menu-bar icon** has two items: **Show approval window** and **Quit
SpeakEasy**. The window hides itself when no requests are left. Its footer
lists the deny blocks that are active.

**Quitting** cancels the requests that are still waiting for you; those are
never sent. A message you just approved may still be posting, so wait until
it shows up in Slack before you quit, stop the service, or re-run the
installer. If you cut a send off, the message can still reach Slack even
though the agent gets an error. SpeakEasy comes back at your next login,
when you start it, or when an agent starts a new session.

**The login service:**

```sh
launchctl kickstart -k gui/$(id -u)/com.speakeasy.slack-approval-gateway   # restart, or start after Quit
launchctl bootout gui/$(id -u)/com.speakeasy.slack-approval-gateway        # stop the service
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/com.speakeasy.slack-approval-gateway.plist  # start after bootout
tail -f ~/.slack-approval-gateway/gateway.log                              # log
```

**To change the transport or a credential,** run the installer again with
the new options, for example
`scripts/install.sh --transport mistral-connector --mistral-api-key NEW_KEY`.
You can also replace the file in `~/.slack-approval-gateway/` and restart the
service. SpeakEasy reads credentials only when it starts.

**Some agents stop waiting after a while.** Mistral Vibe gives up on a tool
call after 60 seconds by default, and goose after 2 minutes with the
installer's settings. The request stays in the window, and approving it
still sends, but the agent doesn't learn the result. If the agent retries
the same message after you approved it, it gets `duplicate: true`, not a
second post.

**Over SSH, or without the window,** stop the login service (`bootout`,
above) and run SpeakEasy in a terminal:

```sh
tail -f /dev/null | ~/.local/bin/slack-approval-gateway --cli
```

Requests appear in that terminal. Answer `a` to approve, `d` to deny, or
`f <your note>` to give feedback. Agents connect to it as usual. Keep the
`tail -f /dev/null |` part, because `--cli` exits when its input closes.

## Security

### What SpeakEasy guarantees

- **Only Approve sends.** Deny, feedback, and closing the window never send
  anything, and nothing approves a request for you.
- **You get what you approved.** SpeakEasy sends the exact text, unchanged,
  to the exact destination and thread in the window. The window shows the
  raw text, and Slack still acts on its markup: `<!channel>` or `<!here>`
  notifies a whole channel, `<@U…>` mentions someone, and
  `<https://…|label>` shows `label` for a different link. Look for these
  before you approve.
- **One approval, one message.** A changed text or destination is a new
  request and needs a new approval. A repeat of a message that was already
  sent (same text, destination, and thread, within 7 days) returns the first
  result with `duplicate: true` and posts nothing, even across restarts.
- **Deny blocks the purpose for 10 minutes.** The block matches the
  destination and similar wording, no matter which agent asks or what
  reason it gives. Blocked requests get `cooldown_blocked` and no window.
  Blocks survive restarts.

### Where the boundary is

SpeakEasy controls messages sent **through its tool**. It can't stop an
agent that has another way to post: a Slack MCP server or Mistral connector
with send access, its own Slack token, a signed-in browser, or open network
access. The instruction blocks and the installer's audit make that less
likely, but they can't prevent it.

Two more gaps come from agents running as your macOS user:

- An agent can read SpeakEasy's credential in `~/.slack-approval-gateway`
  and post with it. Full isolation would need agents that run as a separate
  macOS user, and SpeakEasy doesn't support that yet.
- An agent that can click on your screen can click **Approve** on its own
  request. That includes an agent with a computer-use tool, such as
  Peekaboo or Cua, and one that runs inside an app you gave Accessibility
  access, such as your terminal.

### What the installer's audit reports

Each installer run checks the config files in the [Supported
agents](#supported-agents) table and reports:

1. **Official Slack MCP servers and Mistral connector servers.** These are
   expected: agents use them to read Slack. Both also have a send tool.
   For a Slack MCP server, keep the agent's Slack grant read-only (no
   `chat:write`). A Mistral connector can't be limited that way, so an
   agent with Mistral connector access can send through it, and only the
   instruction block stands in the way.
2. **Token-based Slack MCP servers.** Agents could post through these
   without asking. `scripts/install.sh --enforce` removes them from the
   configs of Claude Code, Codex, OpenCode, pi, pdev, Gemini CLI, Copilot
   CLI, and Amp. Remove the rest by hand: entries in goose, Zed, and Hermes
   configs, and per-project entries in `~/.claude.json`. Removing an entry
   doesn't revoke its token, and the file's `.bak-sag` backup may still
   hold it, so rotate the token in Slack.
3. **Slack tokens and Mistral API keys in agent configs**, masked. Rotate
   them.

A clean report doesn't prove that agents have no other way to send. The
audit can't read Mistral Vibe's server list (`[[mcp_servers]]` in
`~/.vibe/config.toml`), so check that file yourself. It also doesn't see
servers that agent plugins add (such as Claude Code's Slack plugin),
per-project files such as a repo's `.mcp.json`, or credentials in
environment variables.

The audit also prints the state folder's permissions. If you use the
separate agent-guards hook guard (`~/.config/agent-guards`, not part of
SpeakEasy), it checks that the guard is healthy. Hermes Agent's built-in
Slack integration (`platform_toolsets.slack`) is reported but left alone,
and SpeakEasy doesn't gate it.

### Known limits

- "Similar wording" is a heuristic (edit distance and word overlap), not an
  understanding of meaning. A rephrased message can come back as a new
  request. Different names for one destination count as different
  destinations: a channel's `#name` and its ID, or a person's `@name`, user
  ID, and DM channel ID. Requests that were already waiting when you denied
  one still appear.
- The agent name, reason, and conversation in the window are whatever the
  agent sent. SpeakEasy can't check them, so judge each request by its
  message, destination, and thread.
- SpeakEasy and the agents must run on the same Mac.
- The Mistral connector transport relies on a Mistral API that is in public
  preview, and Mistral's SDK marks its tool-call endpoint as deprecated. It
  may change.

## Troubleshooting

**An agent says the `slack-approval-gateway` server failed to start, or no
window appears.** Read `~/.slack-approval-gateway/gateway.log`. The usual
cause is a missing or placeholder credential. With Slack MCP, SpeakEasy
connects to `mcp.slack.com` when it starts and exits if that fails, and
launchd tries again every 10 seconds. Install a real credential and restart
the service.

**`App is not enabled for Slack MCP server access`.** Turn on MCP for your
Slack app (step 3, item 2), then run the OAuth helper again. If MCP calls
still fail after you turn it on, authorize again: some reports say a token
issued before the switch was on keeps failing.

**`No credentials found for connector 'slack'` (Mistral connector).** Either
Slack isn't connected to the Slack connector in Mistral, or the API key's
connector access scope is **Shared connectors only**. Create a new key with
**Private and shared connectors**. SpeakEasy first contacts Mistral when a
request arrives, so this shows up as an `error` result, not at startup.

**The window shows an ID such as `U05…` instead of a name.** SpeakEasy looks
names up when it can, and that needs the read scopes from step 3. The ID is
what gets sent either way.

**A send to a channel fails** (for example with `not_in_channel`). With a
user token, you can post only in channels you're a member of. Join the
channel, or ask the agent to use another one.

**The installer prints `WARN: self-test did not get a handshake reply`.** The
self-test needs the `timeout` command, which macOS doesn't include (install
it with `brew install coreutils`). If `gateway.log` shows no errors, the
service is probably fine.

**An agent lost the tool after you ran the installer again.** Restart the
agent. The installer restarts SpeakEasy, which drops existing connections.

## Uninstall

**Read this first.** For every agent except Claude Code and Vibe,
uninstalling restores the config file from the `.bak-sag` backup taken
before SpeakEasy first edited it. Changes you made to that file since then
are lost, so compare each file with its backup first.

```sh
scripts/install.sh --uninstall           # keeps ~/.slack-approval-gateway
scripts/install.sh --uninstall --purge   # also deletes it: credentials, blocks, and sent records
```

This stops SpeakEasy, removes the LaunchAgent and the binary, unregisters
SpeakEasy from your agents (Claude Code and Vibe through their own CLIs),
and removes the instruction blocks. The `.bak-sag` files are kept. Like the
installer, it stops unless `cargo` and `python3` are on your `PATH`.

Uninstalling doesn't revoke the credential. Without `--purge`, it stays in
`~/.slack-approval-gateway`. With `--purge`, the file is deleted, but the
credential still works. If you're done with SpeakEasy, revoke it: remove
your Slack app from the workspace (Slack MCP), or delete the API key in the
Mistral console (Mistral connector).

## For agent and tool authors

### Register an agent by hand

Any MCP client that can start a stdio server can use SpeakEasy:

```json
{
  "mcpServers": {
    "slack-approval-gateway": {
      "command": "/Users/you/.local/bin/slack-approval-gateway",
      "args": []
    }
  }
}
```

Use the absolute path. If SpeakEasy isn't running, the process your agent
starts becomes SpeakEasy: its menu-bar icon appears, and the window opens
when the first request arrives. If SpeakEasy is running, the process relays
to it through `~/.slack-approval-gateway/gateway.sock`.

### The tool

`request_slack_message` takes:

| Field | Required | Meaning |
|---|---|---|
| `message` | yes | The exact text to send. It is sent unchanged. Up to 40,000 characters. |
| `destination` | yes | A channel (`#name` or channel ID) or a person (`@name` or user ID). |
| `thread_ts` | no | The timestamp of the thread's parent message, such as `1700000000.000100` (not a link), to reply in that thread. |
| `reason` | yes | Why the agent wants to send it. |
| `agent` | yes | Who is asking, such as `claude-code`. |
| `conversation` | yes | The agent's own conversation, for context: `title`, `id_or_link`, `summary`. It's never used as a destination. |

It returns one JSON object, both as `structuredContent` and as text:

| `status` | Other fields | Meaning |
|---|---|---|
| `sent` | `ts`, `permalink`, `duplicate` | Posted. `duplicate: true` means the same `message` was already sent to the same `destination` and `thread_ts` in the last 7 days, whatever the `reason`, `agent`, or `conversation`. Nothing was posted again, and `ts` and `permalink` are from the first send. Usually no window was shown; the exception is a second identical request that was already waiting when you approved the first. |
| `denied` | `cooldown_remaining_secs` | Nothing was sent. Similar requests to this destination are blocked for 600 seconds. |
| `feedback` | `feedback` | Nothing was sent. Revise the request using the human's note and send a new one. |
| `cooldown_blocked` | `cooldown_remaining_secs`, `note` | A similar request was denied recently. Nothing was sent and no window was shown. |
| `error` | `error` | The request failed. Usually nothing was sent; the `error` text says when a message might have been posted. |

Example call and result:

```json
{
  "name": "request_slack_message",
  "arguments": {
    "message": "api v2.14 is live in production. All 312 checks passed.",
    "destination": "C07ENGDPLY",
    "reason": "The release runbook says to announce production deploys in #eng-deploys.",
    "agent": "claude-code",
    "conversation": {
      "title": "Ship api v2.14",
      "id_or_link": "https://github.com/acme/api/pull/1182",
      "summary": "Ran the release checklist for PR #1182: CI green, migration applied, canary healthy."
    }
  }
}
```

```json
{ "status": "sent", "ts": "1700000000.000100", "permalink": "https://acme.slack.com/archives/C07ENGDPLY/p1700000000000100", "duplicate": false }
```

Things to know:

- **The call blocks until the human decides.** SpeakEasy has no timeout. If
  your MCP client gives up first, the request stays in the window, and
  approving it still sends. A retry with the same `message`, `destination`,
  and `thread_ts` never posts twice. If the first request was already sent,
  the retry returns that result with `duplicate: true`. If the first is
  still waiting, the retry appears as a second request, and approving both
  still posts once. If the first was denied, a retry within 10 minutes gets
  `cooldown_blocked`.
- Cancelling the call doesn't withdraw the request from the window.
- If SpeakEasy quits while a request is waiting, you get no `status`
  object. An agent connected through the relay gets a JSON-RPC `-32000`
  error ("…exited before a decision was made; nothing was sent"). If your
  agent's own process was SpeakEasy, the connection just closes.
- Arguments that don't match the schema (missing, unknown, wrong type,
  blank, or a `thread_ts` that isn't a Slack timestamp) return a JSON-RPC
  `-32602` error that lists every problem. A message over 40,000
  characters, or one with no visible characters, returns `status: "error"`
  instead. Nothing is sent in either case.

## Configuration reference

SpeakEasy keeps its state in `~/.slack-approval-gateway/`:

| File | What it holds |
|---|---|
| `slack_transport` | `slack-mcp` or `mistral-connector`. Written only when you pass `--transport`. Without it, SpeakEasy uses `slack-mcp`. |
| `slack_mcp_token` | Slack user token (Slack MCP) |
| `mistral_api_key` | Mistral API key (Mistral connector) |
| `cooldowns.json` | Active deny blocks |
| `sent.json` | Records of sent messages, used to catch duplicates (kept 7 days) |
| `gateway.log` | The login service's log |
| `gateway.lock`, `gateway.sock` | Marks the running SpeakEasy, and the socket that agents' processes relay through |
| `state.lock`, `send.lock` | Keep two SpeakEasy processes from writing state or sending at the same time |
| `*.corrupt-<time>` | A copy of a state file SpeakEasy couldn't read, kept before it rewrote the file |

<details>
<summary><b>Environment variables</b></summary>

Environment variables take priority over the files. The login service
doesn't get your shell's environment, so they don't affect it. They do
affect a SpeakEasy you start yourself, such as with `--cli` or in tests.
They also affect one that an agent starts while the service is off: that
process uses the environment the agent gives its MCP server, so a Slack
token or `SLACK_TRANSPORT` set there wins over the files.

| Variable | Default | Effect |
|---|---|---|
| `SLACK_TRANSPORT` | `slack_transport` file, then `slack-mcp` | The transport |
| `SLACK_GATEWAY_STATE_DIR` | `~/.slack-approval-gateway` | The state folder. A process with a different value doesn't find the running SpeakEasy and starts its own. |
| `SLACK_WORKSPACE_DOMAIN` | `workspace` | Your workspace's subdomain (`acme` for `acme.slack.com`). Used only to build a message link when the transport doesn't return one. |
| **Slack MCP** | | |
| `SLACK_MCP_TOKEN` | `slack_mcp_token` file | The user token |
| `SLACK_MCP_URL` | `https://mcp.slack.com/mcp` | The MCP endpoint |
| `SLACK_MCP_TOOL` | found automatically | The send tool's name. Pinning it also resets the argument names to `channel` / `text` / `thread_ts`, so for `slack_send_message` also set `SLACK_MCP_CHANNEL_ARG=channel_id` and `SLACK_MCP_TEXT_ARG=message`. |
| `SLACK_MCP_CHANNEL_ARG`, `SLACK_MCP_TEXT_ARG`, `SLACK_MCP_THREAD_ARG` | read from the tool's schema | Argument names for the send tool |
| **Mistral connector** | | |
| `MISTRAL_API_KEY` | `mistral_api_key` file | The API key |
| `MISTRAL_API_BASE` | `https://api.mistral.ai` | The API endpoint |
| `MISTRAL_SLACK_CONNECTOR` | `slack` | The connector's name or ID |
| `MISTRAL_SLACK_TOOL`, `MISTRAL_SLACK_CHANNEL_ARG`, `MISTRAL_SLACK_TEXT_ARG`, `MISTRAL_SLACK_THREAD_ARG` | as for Slack MCP | Send tool and argument names |

The Korotovsky transport's variables are described in `src/slack_stdio.rs`.

</details>

## Development

```sh
cargo build                                      # add --release for the installer's build
cargo test                                       # all tests except one slow one
cargo test --test slack_mcp_test -- --ignored    # the slow one: a 30-second timeout
python3 scripts/test_agent_config.py             # tests for the installer's config editing
python3 scripts/smoke_test.py                    # end to end against target/debug (macOS); run cargo build first
scripts/install.sh --dry-run                     # preview an install; needs a release build (cargo build --release)
```

To look at the window without the app, run it in demo mode — synthetic
fixtures, isolated from your credentials and state, never sending:

```sh
SLACK_GATEWAY_DEMO_SEED=single ./target/debug/slack-approval-gateway --demo
# seeds: empty | single | single_cooldown | cooldown | multi | long | unresolved
# env: SLACK_GATEWAY_THEME=light|dark, SLACK_GATEWAY_DEMO_SIZE=WxH,
#      SLACK_GATEWAY_REDUCE_MOTION=1, SLACK_GATEWAY_DEMO_STICKY_DECIDING=1,
#      SLACK_GATEWAY_DEMO_FAIL_SUBMIT=1
```

| Path | What's there |
|---|---|
| `src/core.rs` | The decision flow: validation, deny blocks, duplicate detection |
| `src/mcp.rs` | The MCP server and the `request_slack_message` tool |
| `src/instance.rs` | One running SpeakEasy per user: lock, socket, relays |
| `src/main.rs`, `src/ui.rs` | Startup, run modes, the window, the menu-bar icon |
| `src/ui/` | The approval window: a native egui/eframe + wgpu UI (no browser engine, no webview) |
| `src/slack.rs` | Transport selection and shared credential helpers |
| `src/slack_mcp.rs`, `src/mistral_slack.rs` | The Slack MCP and Mistral connector transports |
| `src/slack_stdio.rs` | The Korotovsky transport (not working yet) |
| `src/slack_tools.rs` | Send-tool discovery and reply parsing, shared by the transports |
| `NOTICE.md` | Upstream license notices (Once UI, Lucide, Heroicons, …) |
| `scripts/install.sh`, `scripts/agent_config.py` | The installer and its agent-config editor |
| `scripts/slack_mcp_oauth.py` | The OAuth helper for the Slack MCP token |
| `tests/`, `scripts/smoke_test.py`, `scripts/mock_korotovsky.py` | Tests and mocks |

[`docs/SPEC.md`](docs/SPEC.md) is the original design spec and module
contract.
