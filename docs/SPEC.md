# SpeakEasy (Slack Approval Gateway) — Implementation Spec

A small local desktop app (Rust) that is the **approval gateway for
agent-initiated Slack messages**. Agents call one MCP tool,
`request_slack_message`; a human decides in a desktop popup; only the app
ever sends to Slack.

This document is the contract. The crate scaffold (`Cargo.toml`, `src/lib.rs`,
`src/main.rs`, and stub modules with complete public types) is already in the
repo. Your job is to replace the `unimplemented!()` bodies in the files you
own, exactly matching the public API already declared. Do not change public
signatures, do not rename items, do not touch files you do not own.

## Behavioral requirements (from the user)

1. **Message requests** require: the exact proposed message; the intended
   Slack recipient or channel; the destination Slack thread if applicable;
   the reason for sending; the requesting agent's identity; and context from
   the originating agent conversation (title, ID or link, brief summary).
2. **Approval popup**: desktop popup showing message, destination, reason,
   and originating conversation context. The agent conversation must be
   clearly distinguished from any destination Slack thread. Three actions:
   **Approve**, **Deny**, **Give feedback**.
3. **Approve**: send the exact approved message to the exact approved
   destination. Approval applies only to that request — any change to content
   or destination requires fresh approval. Return the send result and Slack
   message link to the agent. Prevent duplicate sends from repeated clicks or
   retries.
4. **Give feedback**: user enters feedback; it is returned to the requesting
   agent without sending anything. The agent may revise and submit a new
   request, which requires a new approval.
5. **Deny**: send nothing; block requests for the same underlying purpose
   for 10 minutes — including reworded requests and retries from other
   participating agents. Enforced in the app, persisted across restarts, and
   the remaining cooldown is returned without showing another popup. Do not
   rely solely on an agent-supplied reason string or request ID.
6. **Exclusive sending path**: Slack credentials stay under the app's
   control; participating agents must not be able to send through other
   tools, direct API calls, or browser automation. Enforce via permissions
   and isolation where necessary, and document environment limitations that
   prevent full enforcement (see README).

Keep it lightweight. Handle concurrent requests without mixing up approvals
or feedback. Closing or ignoring a popup must never authorize sending.

7. **Hooks for the agents' own Slack tools**: an agent whose Slack MCP
   SpeakEasy cannot take over (Claude Code's Slack plugin owns its OAuth
   session) keeps sending with its own tools, and a pre-tool hook stops
   each Slack write until the human approves it in the same popup
   (`src/hook.rs`). Same popup, cooldown and dedupe rules; SpeakEasy sends
   nothing for these and needs no credential (the `hooks-only` transport).

## Module contracts

### `src/core.rs` — gateway, cooldowns, dedupe, fingerprinting

Public API is fully declared in the stub. Implement:

- `normalize_text`: lowercase; map every non-alphanumeric char to a space;
  split on whitespace; join with single spaces.
- `similarity`: Levenshtein-based ratio `1 - dist/max(len)` on the two
  normalized strings. Cap compared length at 1000 chars per side (compare
  prefixes) so it stays fast on pathological input.
- `token_similarity`: Sørensen–Dice over whitespace-token sets
  (`2 * |A ∩ B| / (|A| + |B|)`), so a one-word rewording of a short message
  stays above the threshold.
- `same_purpose(a, b)`: true iff `a == b`, or `similarity >= 0.85`, or
  `token_similarity >= 0.7`.
- `message_hash`: stable hex hash (e.g. `std::collections::hash_map::DefaultHasher`)
  over the exact `(message, destination, thread_ts)` triple. Not cryptographic.
- `Gateway::new`: create `state_dir`; load `cooldowns.json` and `sent.json`
  if present; prune expired cooldowns (`expires_at <= now`) and sent records
  older than `SENT_RETENTION_SECS`.
- `Gateway::handle_request` — the exact flow:
  1. `now = now_secs()`. Lock state. **Cooldown check**: for each active
     cooldown entry, if `entry.destination.trim() == req.destination.trim()`
     and `same_purpose(&entry.normalized, &normalize_text(&req.message))`,
     return `CooldownBlocked { cooldown_remaining_secs: entry.expires_at - now }`.
     Do NOT call the provider (no popup). The key deliberately excludes the
     agent identity and request ID — retries from other agents and reworded
     requests match.
  2. **Dedupe check**: `hash = message_hash(&req.message, &req.destination, req.thread_ts.as_deref())`.
     If a sent record with the same hash exists, return
     `Sent { ts, permalink, duplicate: true }` from the cached record. Do NOT
     call the provider and do NOT send again.
  3. Build `MessageRequest { id, incoming: req, received_at: now, destination_display }`
     with a unique ID (e.g. `req-<now>-<atomic counter>`), where
     `destination_display = slack.resolve_destination(&req.destination)` is a
     best-effort human-readable form of the destination for the popup
     (`None` when already readable or unresolvable; never used for sending).
     Call `provider.request_decision(&msg)` **without holding the state
     lock** (it may block for a long time). `Gateway::handle_request_until(req,
     deadline, cancel)` (used by `tools/call`) is the same flow but calls
     `provider.request_decision_until(&msg, deadline, cancel)`: when the
     client cancels the call or the deadline passes before anyone decides,
     the request is withdrawn from the popup and an `Error` is returned
     (nothing sent).
  4. Match the decision:
     - `Ok(Decision::Approve)`: acquire `send_lock`. Re-check the sent records
       for `hash` under the lock (a concurrent identical request may have
       been approved and sent meanwhile); if found, return the cached
       `Sent { duplicate: true }`. Otherwise call
       `slack.post_message(&req.destination, req.thread_ts.as_deref(), &req.message)`
       — exact message, exact destination, exact thread. On success, append a
       `SentRecord`, persist `sent.json` atomically, return
       `Sent { ts, permalink, duplicate: false }`. On `SlackError`, return
       `Error(...)` and record nothing.
     - `Ok(Decision::Deny)`: append
       `CooldownEntry { destination: req.destination.trim().to_string(), normalized: normalize_text(&req.message), excerpt: first 80 chars of message, created_at: now, expires_at: now + COOLDOWN_SECS }`,
       persist `cooldowns.json` atomically, return
       `Denied { cooldown_remaining_secs: COOLDOWN_SECS }`.
     - `Ok(Decision::Feedback(f))`: return `Feedback { feedback: f }`. Send nothing.
     - `Err(ProviderError(e))`: return `Error(e)`. Never treat as approval.
  5. Persistence is JSON via serde; write atomically (write `<file>.tmp`,
     then `rename`). Persist under the state lock.
- `active_cooldowns`: active entries as `(destination, excerpt, remaining_secs)`.
- `Gateway::handle_approval(req, &ApprovalOptions { deadline, share_key,
  cooldown }, cancel) -> ApprovalVerdict` — the
  approval-only flow for a hook-stopped tool call: the same validation,
  cooldown check (→ `CooldownBlocked`, no popup) and dedupe check (an
  identical message SpeakEasy already sent → `AlreadySent { ts, permalink }`,
  no popup), then `provider.request_decision_until(&msg, deadline, cancel)`.
  `Approve` → `Approved` (nothing is sent: the agent's own tool call runs);
  `Deny` → the same cooldown entry as above and `Denied`; `Feedback` →
  `Feedback`; a provider error or a passed deadline → `Error` (never an
  approval). Nothing is recorded as sent.
- `DecisionProvider::request_decision_until(request, deadline, cancel)`: default
  waits without a deadline (`request_decision`); the GUI provider withdraws
  the request from the popup at the deadline (see `src/ui.rs`).
- `SlackClient::can_send()` (default true): false for the `hooks-only`
  client; `handle_request` then returns `Error` before any popup, telling
  the agent to use its own Slack tool.

  Concurrency rule: the state lock is never held across provider calls or
  Slack network calls; `send_lock` covers only the dedupe-recheck + send +
  record critical section, so approvals never mix and identical concurrent
  approvals cannot double-send.

  Unit tests (in `src/core.rs` `#[cfg(test)]`, using a mock `SlackClient`
  that counts calls and a scripted `DecisionProvider`): normalize/similarity/
  same_purpose behavior; approve → sends exact text/destination/thread;
  identical retry → duplicate, mock call count stays 1; changed message →
  fresh approval; deny → cooldown set; reworded same-destination request
  blocked without provider call; same message to a different destination NOT
  blocked; expired cooldown passes; persistence roundtrip (new `Gateway` on
  same dir still blocks); provider error → `Error`, nothing sent.

- `SlackClient` also has `resolve_destination(destination) -> Option<String>`
  (default `None`): a best-effort display name for Slack IDs
  (`U…` → `Name (@handle)`, `C…`/`G…` → `#name`, `D…` → partner's name where
  the transport can). It is called once per popup, only on the path that
  actually reaches the provider (not for cooldown-blocked or duplicate
  requests), and its result rides on `MessageRequest.destination_display`
  to the UI. It must never fail the request.
- The CLI provider prints `Destination:   <display> (<raw id>)` when a
  display name was resolved, else the raw destination.

### `src/slack.rs` — transport selection + credential plumbing

This module owns **transport selection** and the shared credential
plumbing. The gateway holds exactly one credential, per transport: a Slack
**user OAuth token** for the Slack MCP server (the default transport,
implemented in `src/slack_mcp.rs`), a Mistral API key for the
`mistral-connector` transport (`src/mistral_slack.rs`), or a Slack **bot**
token (`xoxb-…`, default) or **user** token (`xoxp-…`) for the
`korotovsky` transport (`src/slack_stdio.rs`). The old direct Slack API
(`direct`) transport was removed on purpose.

- `resolve_transport(state_dir) -> Result<SlackTransport, SlackError>`:
  `SLACK_TRANSPORT` env (`slack-mcp` | `mistral-connector` | `korotovsky`;
  aliases `mcp` / `slack_mcp` / `mistral_connector` / `connector` /
  `slack-mcp-stdio` / `stdio`), else a one-line
  `<state_dir>/slack_transport` file, else `SlackMcp`. Legacy values
  (`direct` / `bot-token` / `token`) are an error explaining the removal and
  pointing at the new transports; unknown values are an error naming the
  accepted ones.
- `hooks-only` (aliases `hooks`, `none`) selects `NoSlackClient`: no
  credential, `post_message` always fails and `can_send()` is false. The
  instance still runs (popup, socket) to answer hook approvals.
- `build_client(state_dir) -> Result<Arc<dyn SlackClient>, SlackError>`:
  dispatch on the selected transport; `src/main.rs` calls this once at
  startup and exits with usage hints (naming what the selected transport
  needs: `SLACK_MCP_TOKEN`/`slack_mcp_token` + the OAuth helper for
  `slack-mcp`, `MISTRAL_API_KEY`/`mistral_api_key` for `mistral-connector`,
  `SLACK_BOT_TOKEN`/`SLACK_USER_TOKEN` or `slack_bot_token`/
  `slack_user_token` for `korotovsky`) on error.
- Shared `pub(crate)` helpers used by the transports: `non_empty_env`,
  `read_secret_file` (trims whitespace; the error names both the env var
  and the file; warns when the file is readable by other users), and
  `fallback_permalink` (a constructed
  `https://<SLACK_WORKSPACE_DOMAIN or "workspace">.slack.com/archives/{channel}/p{ts without '.'}`
  URL, used when a transport returns a `ts` but no link).

### `src/slack_mcp.rs` — the Slack MCP server transport (default)

`SlackMcpClient` implements the `SlackClient` contract by speaking MCP
(Streamable HTTP: one JSON-RPC 2.0 message per POST; replies are plain JSON
or an SSE stream of `data:` lines) to a remote Slack MCP server — by default
the official `https://mcp.slack.com/mcp`. The credential is a Slack **user
OAuth token** (`SLACK_MCP_TOKEN` or `<state_dir>/slack_mcp_token`), obtained
with `scripts/slack_mcp_oauth.py` (authorize
`https://slack.com/oauth/v2_user/authorize`, exchange
`https://slack.com/api/oauth.v2.user.access`); it is never a bot token.

- `from_env(state_dir)`: token from `SLACK_MCP_TOKEN`, else
  `<state_dir>/slack_mcp_token` (via the shared `read_secret_file`);
  `SLACK_MCP_URL` (default `https://mcp.slack.com/mcp`, trailing slash
  stripped), `SLACK_MCP_TOOL` (pin the send tool), and argument renames
  `SLACK_MCP_CHANNEL_ARG` / `SLACK_MCP_TEXT_ARG` / `SLACK_MCP_THREAD_ARG`.
  The MCP handshake runs at construction, so a bad token or endpoint fails
  at startup, not after an approval.
- `new(token, url)`: reject a token with whitespace/control characters;
  build a `ureq` agent with a 30s timeout, `http_status_as_error(false)`
  (statuses are inspected by hand so the server's error body can be
  reported) and `max_redirects(0)` (the bearer token must never leak via a
  redirect); run `initialize` + `notifications/initialized`. The
  `Mcp-Session-Id` response header from `initialize` is stored and sent on
  later requests, along with the negotiated `MCP-Protocol-Version`. All RPC
  goes through one `Mutex<McpSession>` (session id, protocol version, id
  counter, cached tool list and send tool), so concurrent requests are
  serialized and id matching is trivial.
- Send-tool discovery, argument-name detection and reply parsing are shared
  with the connector transport in `src/slack_tools.rs`.
- `post_message`: `tools/call` the send tool with the exact text to the
  exact destination (optionally `thread_ts`). A CallToolResult with
  `isError` (top-level or in `metadata.mcp_meta`) → `SlackError` with the
  joined text blocks. Otherwise the outcome is extracted by
  `slack_tools::send_outcome` (structured content, a JSON text block like
  `{"message_link": …, "message_context": {…}}`, or a plain `slack.com`
  URL in the text; the `ts` is parsed out of a `/p<16 digits>` permalink
  when only a link is available). A missing permalink falls back to
  `fallback_permalink(destination, ts)`.
- `resolve_destination`: `#`/`@` names → `None`. `U…` → the first tool
  whose name contains "user" + ("info"|"profile"|"list"|"search"), called
  with `user_id`; the display name is `Name (@handle)` from the reply's JSON
  name fields (flat, nested `user`/`profile`, or `Real Name:`/`Username:`
  text lines). `C…`/`G…` → the first tool whose name contains
  "channel"/"conversation" + ("info"|"list"|"history"|"read"), called with
  `channel_id`; `#name` from the reply's channels array, channel object, or
  a `Channel: #name (ID)` first text line. `D…` DM IDs → `None`. Any failure
  → `None` (the popup shows the raw ID).
- HTTP rules: the token never appears in error text or `Debug` output.
- Tests (`tests/slack_mcp_test.rs`): `MockSlackMcpServer` in
  `tests/common/mod.rs` (a local MCP server on a `TcpListener`:
  handshake with an `Mcp-Session-Id` header, `tools/list`, `tools/call`
  with canned CallToolResults, a per-server `ts` counter, per-tool reply
  overrides, SSE replies). Assert: the handshake runs at construction and
  the session id is sent back; discovery + exact `params.arguments` +
  bearer token; outcome from structured content, from text, and the
  constructed fallback; `thread_ts` absent when `None`; pinned tool skips
  discovery and uses the legacy names; argument renames; schema-adapted
  names; `isError` → `SlackError`; HTTP error → `SlackError` with `detail`
  and never the token; a rejected handshake fails construction; no send
  tool → error listing names; discovery cached across calls; SSE replies
  parse; concurrent posts each get their own result; `from_env` token file
  + env precedence + arg overrides + `SLACK_WORKSPACE_DOMAIN` fallback +
  `build_client` dispatch; `resolve_destination` user/channel resolution
  (nested, flat, and text shapes) and its `None` cases.

### `src/slack_stdio.rs` — the korotovsky Slack MCP server transport (stdio)

`KorotovskyClient` implements the same `SlackClient` contract by **spawning
Korotovsky's unofficial Slack MCP server**
(`slack-mcp-server`, <https://github.com/korotovsky/slack-mcp-server>)
as a child process and speaking MCP (newline-delimited JSON-RPC 2.0 over
its stdin/stdout). The user installs the server; SpeakEasy never installs
it. A command taken over from an agent's config runs as configured, so an
`npx … slack-mcp-server@latest` line can download the package at each
start; the installer substitutes an installed `slack-mcp-server` when one is
on PATH. The command resolves from `SLACK_STDIO_COMMAND`, else a
one-line `<state_dir>/slack_stdio_command` file, else `slack-mcp-server`
found on PATH (run with `--transport stdio`); when none resolves, startup
fails with a "not installed" error. Message links use the workspace domain
from `SLACK_WORKSPACE_DOMAIN`, else `<state_dir>/slack_workspace_domain`. The
credential is a Slack **bot token** (`xoxb-…`, default) or **user token**
(`xoxp-…`), held by the gateway and passed to the child as
`SLACK_MCP_XOXB_TOKEN` / `SLACK_MCP_XOXP_TOKEN`; agents never see it.

- `from_env(state_dir)`: bot token from `SLACK_BOT_TOKEN` else
  `<state_dir>/slack_bot_token`, user token from `SLACK_USER_TOKEN` else
  `<state_dir>/slack_user_token` (either one suffices). The server's own
  token variables also count — `SLACK_MCP_XOXP_TOKEN`,
  `SLACK_MCP_XOXB_TOKEN`, or the browser-session pair
  `SLACK_MCP_XOXC_TOKEN` + `SLACK_MCP_XOXD_TOKEN` (no Slack app) — whether
  inherited from the environment or listed in `<state_dir>/slack_stdio_env`
  (`KEY=VALUE` lines, chmod 600, written when the installer takes over a
  korotovsky server an agent already uses). `new` passes that file's
  variables to the child, except where the operator's environment sets
  them. The installer's placeholder tokens (`xoxb-placeholder-…`,
  `xoxp-placeholder-…`) are ignored, so they never outrank a session. At least one credential is required, else an error naming the
  files. The server command (see
  above; whitespace-split, no quoting), `SLACK_STDIO_CONTENT_TYPE`
  (`text/plain` default / `text/markdown`), `SLACK_STDIO_TOOL` (pin the
  send tool) and argument renames `SLACK_STDIO_CHANNEL_ARG` /
  `SLACK_STDIO_TEXT_ARG` / `SLACK_STDIO_THREAD_ARG`.
- `new(bot_token, user_token, command, content_type, state_dir)`: reject a
  token with whitespace/control characters; spawn the child with piped
  stdio and set-if-absent environment defaults the gateway controls:
  `SLACK_MCP_ADD_MESSAGE_TOOL=true` (the server's `conversations_add_message`
  tool is disabled by default for safety — without this the send tool is
  not even listed), `SLACK_MCP_LOG_LEVEL=error`, and the server's user/channel
  caches (`SLACK_MCP_USERS_CACHE` / `SLACK_MCP_CHANNELS_CACHE`) pointed into
  the state dir. The operator's own values win. A spawn failure names the
  command and the fix.
- Process plumbing: a reader thread parses stdout line by line and skips
  anything that is not one JSON-RPC message (a bad `SLACK_MCP_LOG_LEVEL`
  prints to stdout); a second thread keeps the last 25 stderr lines for
  error messages. All RPC goes through one `Mutex<StdioSession>` (child
  stdin, reply channel, id counter, cached tool list and send tool), so
  concurrent requests are serialized and id matching is trivial; a dead
  server surfaces its stderr tail. `Drop` kills the child — the server never
  outlives the gateway.
- Handshake at construction: `initialize` + `notifications/initialized` +
  `tools/list`, and the send tool must resolve — the handshake fails at
  startup (with the server's stderr) when `conversations_add_message` is
  not registered.
- Send-tool resolution: `conversations_add_message` first (the server's own
  send tool — it does **not** match the generic send-message name pattern),
  then any tool whose name matches `slack_tools::is_send_message_tool`;
  argument names detected from the declared `inputSchema`
  (`channel_id` / `text` / `thread_ts`; the text candidates try `payload`
  for older server versions). Missing tool → error listing the available
  names and the `SLACK_MCP_ADD_MESSAGE_TOOL` hint.
- `post_message`: `tools/call` the send tool with the exact text to the
  exact destination (optionally `thread_ts`) **plus `content_type:
  text/plain`** (Slack then renders the exact approved text without markdown
  interpretation; `text/markdown` is the opt-in override). A CallToolResult
  with `isError` → `SlackError` with the joined text blocks. The server's
  reply is plain text (`Successfully posted message to channel … (ts=…)`,
  or `… in thread … (ts=…)`); the channel and `ts` are parsed out of it
  (after trying `slack_tools::send_outcome` first, for forward
  compatibility), and the permalink falls back to
  `fallback_permalink(channel, ts)`.
- `resolve_destination`: `#`/`@` names → `None`. `U…` → `users_search`
  (query = the user ID; the server looks user IDs up directly), parsing the
  CSV reply (`UserID,UserName,RealName,DisplayName,…`) → `Name (@handle)`.
  `C…`/`G…` → `channels_list` (`channel_types:
  public_channel,private_channel,im,mpim`), parsing the CSV reply
  (`ID,Name,…`) → `#name`. `D…` DM IDs → `None`. Any failure → `None`. The
  CSV parser handles gocsv-quoted fields (commas/quotes inside values).
- Tests (`tests/slack_stdio_test.rs`, mock in `scripts/mock_korotovsky.py`:
  a stdio MCP server recording tool calls + its environment to a JSON-lines
  file): exact send arguments + `text/plain`; thread replies; tool errors;
  stdout noise skipped; startup failures (no send tool, server exit with
  stderr tail, missing command); user/channel resolution incl. quoted CSV;
  dropping the client kills the child; `from_env` credential precedence
  (flag/env/file, inherited `SLACK_MCP_XOX*`, whitespace rejection, content
  type validation), the set-if-absent server env defaults, and
  `build_client` dispatch. Unit tests cover the reply/CSV/command parsers
  and send-tool resolution.

### `src/slack_tools.rs` — shared tool-call plumbing (all transports)

`pub(crate)` helpers shared by `slack_mcp`, `slack_stdio` and
`mistral_slack`:

- `SendToolSpec { name, channel_arg, text_arg, thread_arg }` and
  `resolve_send_tool(tools, channel_override, text_override,
  thread_override, no_tool_hint)`: the first tool whose lowercase name
  contains ("send"|"post") and "message", with argument names picked from
  the tool's declared `inputSchema.properties` (candidates, most specific
  first: channel — `channel_id`/`channel`/`channel_name`/`destination`/
  `to`/`recipient`; text — `message`/`text`/`content`/`body`; thread —
  `thread_ts`/`thread`/`parent_ts`/`reply_to`). No declared schema → legacy
  `channel`/`text`/`thread_ts` (the thread argument assumed present). No
  match → error listing the available names and the pin hint. Explicit
  overrides win. A thread reply to a tool whose schema declares no thread
  argument is an error (nothing is sent).
- `send_arguments(spec, destination, thread_ts, text)`: the exact arguments
  object (`thread` key only when `Some`).
- `tool_result_error(reply)`: the error text when the CallToolResult has
  `isError`/`is_error` (top-level or wrapped in `metadata.mcp_meta`).
- `send_outcome(reply)`: best-effort `SendOutcome { ts, permalink, channel }`
  from (1) top-level or `mcp_meta`-wrapped `structuredContent` plus the
  result object's own fields, (2) a text block that parses as a JSON object
  (`message_link`/`permalink`/`url`/`link`, and `message_context`'s
  `message_ts`/`channel_id`), (3) the first `https://…slack.com` URL in the
  text (also after unescaping `\/`); the `ts` is parsed out of a
  `/p<16 digits>` permalink when only a link is available.
- `text_content`/`json_text`/`field_line`/`find_slack_url`/`ts_from_permalink`
  plus unit tests for all of the above.

### `src/mistral_slack.rs` — the Mistral Slack Connector transport

`MistralConnectorSlackClient` implements the same `SlackClient` contract by
calling the Slack tool of a Mistral platform connector **directly** (no LLM
in the send path), so the exact approved message still goes to the exact
approved destination. The Slack credentials live in the Mistral console
(the connected Slack workspace); this process holds only a Mistral API key.
Send-tool discovery, argument-name detection and reply parsing are shared
with the Slack MCP transport in `src/slack_tools.rs`.

- `from_env(state_dir)`: key from `MISTRAL_API_KEY`, else
  `<state_dir>/mistral_api_key` (via the shared `read_secret_file`).
  `MISTRAL_API_BASE` (default `https://api.mistral.ai`),
  `MISTRAL_SLACK_CONNECTOR` (else a one-line
  `<state_dir>/mistral_slack_connector` file, written when the installer
  takes over a connector that isn't named `slack`; default `slack`),
  `MISTRAL_SLACK_TOOL` (pin
  the send tool), and argument renames `MISTRAL_SLACK_CHANNEL_ARG` /
  `MISTRAL_SLACK_TEXT_ARG` / `MISTRAL_SLACK_THREAD_ARG` (defaults
  `channel` / `text` / `thread_ts`).
- Send-tool discovery: `GET {api_base}/v1/connectors/{connector}/tools?page_size=100`
  (bare JSON array; `data`/`items` wrappers also accepted), first tool whose
  lowercase name contains ("send"|"post") and "message", cached for the
  life of the client (`Mutex<Option<SendTool>>`). No match → error listing
  the available names and the `MISTRAL_SLACK_TOOL` hint. A pinned tool
  skips discovery.
- Argument names are resolved from the discovered tool's declared
  `inputSchema.properties` (candidates, most specific first: channel —
  `channel_id`/`channel`/`channel_name`/`destination`/`to`/`recipient`;
  text — `message`/`text`/`content`/`body`; thread —
  `thread_ts`/`thread`/`parent_ts`/`reply_to`), so the real `slack`
  connector's `slack_send_message` (`channel_id`/`message`/`thread_ts`) works
  with no configuration. No declared schema → legacy `channel`/`text`/
  `thread_ts`. `MISTRAL_SLACK_CHANNEL_ARG`/`MISTRAL_SLACK_TEXT_ARG`/
  `MISTRAL_SLACK_THREAD_ARG` (or `with_arguments`) override the detected
  names. A thread reply to a tool whose schema declares no thread argument
  is an error (nothing is sent).
- `resolve_destination`: `#`/`@` names → `None`. `U…` →
  `slack_read_user_profile {user_id}` — the reply's text block is a JSON
  string `{"result": "…Real Name: …\nUsername: …"}`; the display name is
  `Real Name` (fallback `Display Name`, `Username`), formatted
  `Name (@username)`. `C…`/`G…` → `slack_read_channel {channel_id, limit: 1}`
  — the reply's `{"messages": "Channel: #name (ID)\n…"}` first line yields
  `#name`. `D…` DM IDs → `None` (the connector has no DM-partner lookup).
  Any failure → `None`.
- `post_message`: `POST {api_base}/v1/connectors/{connector}/tools/{tool}/call`
  with `Authorization: Bearer <key>` and body
  `{"arguments": {channel_arg: destination, text_arg: text, thread_arg?}}`
  (`thread_arg` only when `Some`). The reply is an MCP CallToolResult:
  `metadata.mcp_meta.isError`/`is_error` → `SlackError` with the joined
  text blocks; otherwise the outcome is extracted from (1)
  `metadata.mcp_meta.structuredContent`, (2) a text block that parses as a
  JSON object — the real slack connector returns
  `{"message_link": …, "message_context": {"message_ts": …, "channel_id": …}}`
  as a JSON string with escaped slashes, (3) the first `https://…slack.com`
  URL in the text blocks (also after unescaping `\/`). The `ts` is parsed
  out of a `/p<16 digits>` permalink when only a link is available; the
  channel is the resolved one when reported. Missing permalink falls back
  to `fallback_permalink(destination, ts)`.
- HTTP rules mirror the direct client: statuses inspected by hand (the
  platform's `{"detail": …}` error body is reported), 30s timeout,
  redirects never followed (the bearer key must not leak), and the API key
  never appears in error text or `Debug` output.
- Tests (`tests/mistral_slack_test.rs`): a mock Mistral platform API
  (`MockMistralServer` in `tests/common/mod.rs`, same recording contract as
  the mock Slack server) serving the tool catalog and canned CallToolResult
  replies. Assert: discovery + exact `arguments` + bearer key; outcome from
  structured content, from text, and the constructed fallback;
  `thread_ts` absent when `None`; pinned tool skips discovery; argument
  renames; `isError` → `SlackError`; HTTP error → `SlackError` with
  `detail` and never the key; no send tool → error listing names;
  discovery cached across calls; `from_env` key file + arg overrides;
  `resolve_transport` precedence (env > file > default).

### `src/mcp.rs` — MCP JSON-RPC server (stdio and socket)

- Newline-delimited JSON-RPC 2.0. `run(gateway, input: impl BufRead, output:
  impl Write)` serves any byte stream; it is used for the instance's own
  stdio and for every socket connection from a bridge
  (`src/instance.rs`). Read lines from `input`; dispatch:
  - `initialize` → result `{protocolVersion: <client's requested version, or "2025-06-18">, capabilities: {tools: {listChanged: false}}, serverInfo: {name: "slack-approval-gateway", version: env!("CARGO_PKG_VERSION")}}`
  - `notifications/cancelled` → set the cancel flag of the in-flight
    `tools/call` with that `requestId` (if any): a request still waiting
    for the human is withdrawn from the popup, nothing is sent, and the
    call gets no response (MCP); a decision made before stands. When the
    connection's input ends (the agent quit), every in-flight call of that
    connection is cancelled the same way; the bridge forgets a cancelled
    id, and all ids once the agent's input ends, so it never answers them
    with its "instance exited" error
  - `notifications/initialized` and any other `notifications/*` → no response
  - `ping` → `{}`
  - `tools/list` → one tool:
    - name: `request_slack_message`
    - description: explain that this requests human approval to send a Slack message; the exact message and destination are sent only after explicit human approval; denials block the same purpose for 10 minutes; feedback is returned without sending.
    - inputSchema: `type: "object"`, `required: ["message","destination","reason","agent","conversation"]`, properties:
      - `message` (string): the exact proposed message text
      - `destination` (string): Slack channel (`#name` or ID) or user (`@name` or ID)
      - `thread_ts` (string, optional): destination Slack thread timestamp for a reply
      - `reason` (string): why the agent wants to send this
      - `agent` (string): identity of the requesting agent
      - `conversation` (object, required `["title","id_or_link","summary"]`): context from the originating agent conversation (title; ID or link; brief summary)
  - `tools/call` with `params.name == "request_slack_message"`: validate
    `arguments` (required fields present and correctly typed;
    `thread_ts` optional string; `conversation` object with its three
    required string fields). Invalid → JSON-RPC error `-32602`.
    Otherwise build `IncomingRequest`, call
    `gateway.handle_request_until(req, deadline, cancel)` (cancel: the
    call's flag above; deadline: none, except for clients that give up
    without cancelling — `clientInfo.name` "Zed" from `initialize` gets
    590 s, under Zed's fixed 600 s limit), and respond with `result.content = [{type: "text", text: <JSON string>}]`
    where the JSON string is one of:
    - `{"status":"sent","ts":...,"permalink":...,"duplicate":bool}`
    - `{"status":"denied","cooldown_remaining_secs":600}`
    - `{"status":"feedback","feedback":"..."}`
    - `{"status":"cooldown_blocked","cooldown_remaining_secs":N,"note":"a similar request was denied recently; nothing was sent and no approval prompt was shown"}`
    - `{"status":"error","error":"..."}` (with `result.isError = true`)
    Also include `result.structuredContent` mirroring the same object.
  - `speakeasy/approve` (not a tool; called by `--hook` processes on the
    instance socket): params are the `request_slack_message` arguments
    plus `wait_secs` (1..=86400, required); unknown fields → `-32602`.
    Optional `share_key` (string) and `cooldown` (bool, default true).
    Runs `gateway.handle_approval(req, opts, closed)` on a worker
    thread and answers `{"decision": "approved" | "denied" | "feedback" |
    "cooldown_blocked" | "already_sent" | "error", …}` with
    `cooldown_remaining_secs`, `feedback`, `ts`/`permalink` or `error` as
    applicable. It never sends anything.
  - unknown method → `-32601`; malformed JSON → `-32700` (id null);
    JSON-RPC message not an object / missing id on a request → `-32600`.
- Concurrency: handle each `tools/call` on a spawned thread; serialize all
  writes to `output` through `Arc<Mutex<W>>`. The read loop stays on the
  calling thread. Return `Ok(())` on EOF of `input`.
- Only protocol bytes go to `output`; log diagnostics to stderr.
- Tests (`tests/e2e.rs`): drive `mcp::run` in-process with
  `std::os::unix::net::UnixStream::pair()` (macOS/Linux), a scripted
  `DecisionProvider`, and the mock Slack MCP server from
  `tests/common/mod.rs` (the gateway's Slack client is a real
  `SlackMcpClient` pointed at it). Cover: initialize/tools/list shape;
  approve → the mock's send tool received the exact message/destination/
  thread as `params.arguments` and the agent got `sent` + permalink; deny →
  `denied` with 600 and a reworded same-destination retry is
  `cooldown_blocked` with remaining time and the provider was NOT called
  again; feedback → agent gets the text and the mock saw no send;
  identical retry after approval → `sent` with `duplicate: true` and the
  mock's send count unchanged; two concurrent requests with different
  content get correctly matched decisions (no mixing); a NEW `Gateway` on
  the same state dir (simulated restart) still returns `cooldown_blocked`;
  a `U…` destination resolves to a display name in the request the provider
  saw while the send still carries the raw ID; a `notifications/cancelled`
  withdraws a waiting call (no response, nothing sent); a Zed client's
  calls carry the 590 s deadline and other clients' none.

### `src/instance.rs` — single-instance coordination and stdio bridges

Every coding agent spawns the binary over MCP stdio, but the popup, the
Slack client, and the pending-request queue must live in exactly one
process per state dir:

- `connect_existing(state_dir) -> Option<UnixStream>`: connect to
  `<state_dir>/gateway.sock`; `None` means no live instance.
- `acquire(state_dir) -> Result<(InstanceGuard, UnixListener), AcquireError>`:
  atomically take `<state_dir>/gateway.lock` (`create_new`; a lock whose
  socket refuses connections for ~2s is stale and is stolen, so a crashed
  instance never blocks the next start), remove a stale socket file, and
  bind the listener. `AcquireError::AlreadyRunning` means a live instance
  exists — the caller bridges instead. The state-dir path is rejected when
  it cannot fit in `sun_path` (~104 bytes on macOS).
- `serve_connections(gateway, listener)`: accept loop; each connection is
  served by `mcp::run` on its own thread (one agent waiting for a human
  never blocks another).
- `run_bridge(stream) -> !`: byte-transparent pump between the process's
  stdio and the instance socket; exits when the instance closes the
  connection. stdout carries protocol bytes only.
- `InstanceGuard` drops by removing the lock and socket files (best effort;
  a crash leaves them and the next starter steals the lock).

### `src/ui.rs` — native-UI bridge (the approval popup)

The human-facing popup is a **Rust-native desktop UI** — egui/eframe with
the wgpu renderer, no browser engine, no embedded webview. It is a 1:1
visual and behavioral recreation of the former React 19 + Tailwind 4 +
**Once UI** (`@once-ui-system/core`) + **Motion Primitives** frontend:
same layout, spacing, typography, colors, radii, states, and animations
(title word reveal, spring-animated pending count), recreated from the
reference's computed styles and the installed Motion defaults. (The
reference's card `Spotlight` paints *nothing* — its Tailwind radial-gradient
background is invalid at computed-value time — so no spotlight is rendered;
see the `shaders.rs` module doc.) SF Pro / SF Mono are installed explicitly
so text metrics match the reference on macOS.

`src/ui.rs` is the Rust bridge between the gateway's MCP worker threads
and the native UI event loop (it replaces the old Tauri events/commands
transport with typed Rust calls):

- `UiState`: pending-request map (`request_id -> Sender<Result<Decision,
  ProviderError>>`), an outbox of requests for a not-yet-ready UI, the
  installed `UiHooks`, and the gateway (for the cooldown status line).
- `UiHooks`: callbacks into the event loop — `reveal` (a request arrived:
  show + focus the hidden window), `notify` (wake the event loop so the
  outbox drain is seen promptly), `hide` (the queue drained: hide the
  window), `quit` (tray Quit). They run on worker threads while the bridge
  lock is held, so they only capture the `egui::Context` and request
  wakeups — never blocking work.
- `GuiDecisionProvider::request_decision`: register the request, queue it
  in the outbox, reveal the window, then block on the single-use reply
  channel. Requests arriving before the UI is running stay queued and are
  delivered on the UI's first drain. If the app exits, the sender is
  dropped and the request fails with `ProviderError` — it is never
  approved. A single-use channel guarantees exactly one decision per
  request; repeated UI clicks cannot produce a second decision.
- The UI renders the destination as
  `destination_display ?? incoming.destination`, with the raw destination
  as a secondary tag when a display name was resolved (display only — the
  exact `destination` is what gets sent).
- `UiState::submit(request_id, decision)`: resolves exactly one blocked
  worker. The first decision wins; repeats and unknown/stale IDs are
  rejected (`SubmitError`), so a repeated click cannot double-send and a
  stale ID cannot decide another request. A rejected submission is
  reported to the UI as an actionable error and the request is *retained*
  so the human can try again — the queue only advances after an accepted
  decision.
- `GuiDecisionProvider::request_decision_until(request, deadline, cancel)`: like
  `request_decision`, but at the deadline, or when `cancel` is set (polled
  every 200 ms), the request is withdrawn (`UiState::withdraw`: removed
  from the pending map and the outbox, and listed for
  `UiState::drain_withdrawn`, which the UI drains every pass to drop it
  from its queue and hide an emptied window). A request whose hook left
  before it was shown is never queued. A late click is rejected as stale.
  A decision submitted before the withdrawal wins and is returned.
- `UiState::hide_if_drained`: hides the window once the queue has drained
  (checked under the bridge lock, so it never races a newly arriving
  request into a hidden window with something pending).
- The UI (`src/ui/app.rs` + `src/ui/app/paint.rs`) renders one request at
  a time: the exact proposed message (monospace, wrapped, selectable), the
  Slack destination (with a hint distinguishing it from the agent
  conversation), the reason, the originating agent conversation (context
  only — not a Slack thread), the three actions **Approve** / **Deny** /
  **Give feedback** (feedback text entry enabled only when non-empty), a
  "N waiting" badge, active deny cooldowns, and the caption "Closing this
  window hides it and never approves a message; SpeakEasy keeps running
  (menu-bar icon to reopen or quit). If SpeakEasy quits, pending requests
  fail and the agent is told nothing was sent."
- Closing the window only **hides** it (egui `CloseRequested` → hide); the
  gateway process keeps serving MCP for every agent. Quitting happens only
  via **Quit SpeakEasy** in the tray menu or the app menu (⌘Q)
  (`UiState::quit` → the event loop exits) or a signal; pending reply
  senders are then dropped, so blocked `request_decision` calls return
  `Err` — never an approval. Any activation of the app while it is
  inactive (a Dock click, Cmd+Tab, `open`) shows the window again (an
  `NSApplicationDidBecomeActive` observer, `sys::observe_dock_reopen`); a
  Dock click while SpeakEasy is still the active app does not.
- A menu-bar tray icon (`tray-icon` crate, macOS) offers **Show approval
  window** / **Quit SpeakEasy** on right-click; a left click on the icon
  shows the window.

`src/ui/` module map:

| Module | Role |
|---|---|
| `app.rs` | eframe app: window lifecycle (720×780 default, min 520×560, centered, initially hidden), tray menu, keyboard/focus model, decision dispatch, reduced-motion handling |
| `app/paint.rs` | frame layout + painting (fixed header / scrolling main / fixed footer), scrollbar |
| `components.rs` | Once UI-style controls: `Tag`, `Button`, `Textarea`, section labels, selectable text |
| `theme.rs` | light/dark palettes + responsive metrics (rem root 15px ≤768px viewport, else 16px) |
| `text.rs` | SF Pro / SF Mono installation + CSS-exact `TextSpec`s (size, weight, line-height, letter-spacing) |
| `anim.rs` | Motion-matched animation: spring physics (stiffness 100 / damping 10 / mass 1), cubic-bezier tweens, `TextEffect` fade-in-blur, `AnimatedNumber` |
| `icons.rs` | Lucide SVG path parser + flattener/renderer (Slack, inbox, hash, person, chat, …) |
| `shaders.rs` | the title reveal's gaussian blur + premultiplied text draw as small wgpu paint callbacks |
| `sys.rs` | macOS integration: NSPasteboard copy, dock/reveal, reduced-motion + on-screen queries |
| `demo.rs` | `--demo` synthetic fixtures (isolated state dir, mock Slack — never sends) |

### `src/hook.rs` — pre-tool hook mode (`--hook <agent> [--wait SECS]`)

An agent hook runs the binary before a tool call with the agent's hook
payload on stdin. The mode:

1. Reads the payload (field spellings of every supported agent:
   `tool_name`/`toolName`/`tool`, `tool_input`/`toolArgs`/`args`/
   `arguments`/`input` — a JSON string is parsed —, `session_id`, `cwd`,
   `mcp_server.name`). An unreadable payload is blocked only when it
   mentions Slack; otherwise the answer is neutral.
2. Classifies the call. Splits MCP names (`mcp__server__tool`,
   `mcp_server.tool`, `connector_x.tool`, `server__tool`, `MCP:tool`); a
   call is a Slack call when its name or server contains `slack` or ends
   with one of korotovsky's tool names (`conversations_add_message`,
   `reactions_add`, …). SpeakEasy's own tool on its own server
   (`slack-approval-gateway` / `slack_approval_gateway`) passes. A Slack
   call is gated unless it is a read: a write word (send, post, schedule,
   add, create, update, react, upload, join, mention, ping, …; `sends` →
   `send`) always gates; otherwise a read verb (read, search, list, get,
   history, …) passes, and no recognizable verb gates. A fixed list of
   self-only tools passes (`slack_send_message_draft`, `conversations_mark`,
   saved items, sign-in, `slack_get_file_upload_url`, `channels_me`).
   Executor, recognized by its server or by "executor" in the full name
   (`executor-call`, `mcp_executor_call`), is gated through its `call`
   (by `path`) and `execute` (the program); goose's `execute_typescript`
   likewise. Programs are checked after undoing `\u`/`\x` escapes and
   `'a' + 'b'` joins: a Slack tool that writes, a computed tool name
   (`tools…[…]`, `globalThis[…]`), `eval(` or `Function(` gates.
3. Builds the popup request: destination from `channel_id`/`channel`/
   `user_id`/… , the exact text from `message`/`text`/`payload`/…, a Slack
   `thread_ts`; other arguments are listed in the reason. The text is shown
   as the exact message only when it is unambiguous — one text argument, at
   most one destination and thread argument, no rich content (`blocks`,
   `attachments`, `sections`); otherwise the message shown is every
   argument as pretty JSON, and every destination-like value is named.
4. Sends `speakeasy/approve` with `wait_secs = --wait − 5`, a `share_key`
   (a hash of the agent's display name, session, tool, arguments and call
   ID; only with a call ID) and `cooldown: false` when the call names no
   destination, to the instance at `--state-dir` (else the usual state
   dir), and maps the answer: `approved` → allow; everything else →
   deny with a model-facing reason (denied + cooldown, feedback text,
   cooldown-blocked, already sent, error). No instance → deny.
5. Answers in the agent's format (`render`; each was checked against the
   agent's hook runner):

   | Agent | Neutral | Approved | Deny |
   |---|---|---|---|
   | Claude Code (also pdev; imported by Cursor) | exit 0, no output | `hookSpecificOutput.permissionDecision: "allow"` | same with `"deny"` + reason, reason on stderr, exit 2 |
   | Codex | exit 0, no output | neutral (a bare `allow` is a hook failure) | as Claude Code |
   | Gemini CLI | exit 0, no output | neutral | `{"decision":"deny","reason":…}`, exit 0 |
   | Copilot CLI | exit 0, no output | `{"permissionDecision":"allow",…}` | `{"permissionDecision":"deny","permissionDecisionReason":…}`, exit 0 |
   | Cursor | `{}` (empty output is a failure with `failClosed`) | `{"permission":"allow"}` | `{"permission":"deny","user_message":…,"agent_message":…}`, exit 0 |
   | Mistral Vibe | exit 0, no output | neutral | `{"decision":"deny","reason":…}`, exit 0 (`strict = true`) |
   | goose, and SpeakEasy's OpenCode / pi / Hermes / Amp plugins | exit 0, no output | neutral | `{"decision":"block","reason":…}`, reason on stderr, exit 2 |

   Approved answers skip the agent's own second prompt where the format
   allows it; elsewhere the agent's normal permission flow follows.

The `--hook claude` display name follows the environment (`PDEV_VERSION` →
pdev, `CURSOR_VERSION` → Cursor). goose's Code Mode (`execute_typescript`)
does not hook the MCP calls a program makes, so for goose the program text
is checked for Slack writes (`classify_for`); pi, pdev, Vibe and OpenCode
hook each nested call themselves.

A watchdog thread answers "blocked" when `--wait` seconds pass (default
600), and one mutex-guarded `answer` makes the first answer final. With
`--ok-code N` (3..=125) every exit 0 becomes N; the installer's wrapper
maps N to 0 and anything but N and 2 to a block, so an older binary
without hook mode (exit 0) fails closed. Every
agent runs the tool when its hook times out, so the installer registers
the hook with the agent's timeout set to `--wait` + 60 seconds, through a
shell wrapper that turns a missing binary or any exit other than 0/2 into
exit 2 (Cursor: `failClosed`; Vibe: `strict`; the plugins block on any
failure themselves).

Instance side (`speakeasy/approve`): requests that carry the same
`share_key` and exact message share one popup and one decision
(`Gateway::handle_approval` with `ApprovalOptions`; a decision is shared
while pending and for 30 s after, measured with `Instant`; a panic while
asking fills the shared slot with an error; a waiting duplicate leaves
when its own connection closes). `cooldown: false` skips the cooldown
check and records no deny cooldown. Slack's 40,000-character limit does
not apply to approvals. A request is withdrawn from the popup when its connection closes
(the hook was killed or interrupted): `mcp::run` sets a per-connection
flag at EOF, which `request_decision_until` polls.

`--hook claude --event config-change` is Claude Code's `ConfigChange`
hook: it blocks (exit 2) a live settings change that sets
`disableAllHooks` or `env.CLAUDE_CODE_SIMPLE`, or leaves the user settings
missing, invalid, or without SpeakEasy's `PreToolUse` hook as installed
(the exact `CLAUDE_MATCHER`, a command that starts with the wrapper and
runs `--hook claude --wait <its own --wait>`, and a timeout of at least
that + 60).

In the GUI, a click whose press began before the current card appeared is
ignored (`ApprovalApp::decide`): a withdrawal can change the front card
under the pointer. `--cli` mode cannot withdraw a terminal prompt; an
answer given after the deadline or after the hook left is discarded.

### `src/main.rs` — run modes, tray, window lifecycle

Wires state dir (`SLACK_GATEWAY_STATE_DIR` or `~/.slack-approval-gateway`),
Slack client, provider, MCP server, and UI. `--hook <agent> [--wait
SECS]` is checked first (see `src/hook.rs`; it answers one agent hook call
and exits, never becoming the instance or a bridge). Three run modes:

- `--cli`: headless. Becomes the instance (lock + socket), builds the
  gateway with the terminal fallback provider, serves the socket, and runs
  MCP on stdio. Used by the smoke test and for SSH hosts.
- `--serve`: service mode (LaunchAgent). Becomes the instance, runs the
  native eframe app, serves the socket, but runs no MCP on stdio.
- default (spawned by an agent over stdio): if an instance answers on the
  socket, bridge to it (`instance::run_bridge`, never returns); otherwise
  become the instance (lost startup races bridge instead). The instance
  serves MCP on its own stdio (unless `--serve`) plus the socket.

GUI mode runs the eframe app (managed `UiState` + `UiHooks`, MCP server
on a background thread), installs the tray icon, hides the window on
close, and shows it again when the app is activated
(`NSApplicationDidBecomeActive`). After the event loop ends, the app
waits for any in-flight approved send to finish and be recorded before
exiting. The window is created hidden (720×780, min 520×560, centered)
and revealed per request. `--demo` runs the same UI against synthetic
fixtures with an isolated state dir and a mock Slack client — it never
sends real messages.

## Build, install, and test

- Rust toolchain (cargo 1.98+) is installed. The UI is pure Rust — there
  is no Node/npm/Vite build step; `cargo build` produces the complete
  binary.
- `scripts/install.sh` installs the app into every detected coding agent on
  this Mac and starts the LaunchAgent service (`--dry-run`, `--enforce`,
  `--uninstall`, `--purge`, `--skip-build`, `--slack-mcp-token`,
  `--transport` (`slack-mcp` | `mistral-connector` | `korotovsky` |
  `hooks-only`), `--mistral-api-key`, `--slack-bot-token`,
  `--slack-user-token`, `--workspace-domain`, `--take-over FILE[:NAME]`,
  `--no-take-over`, `--no-hooks`, `--hook-wait SECS` flags; see README
  "Install"). Transport choice, in order: a taken-over server's transport;
  `--transport`; an existing `slack_transport` file (kept, except that a
  credential flag given now switches it to the flag's transport, and a
  hooks-only file also switches for a `$SLACK_MCP_TOKEN` or a credential
  stored after it, `credential_transport` in the installer; a flag
  outranks `$SLACK_MCP_TOKEN`); the transport a credential flag implies (`--mistral-api-key` → `mistral-connector`,
  `--slack-bot-token`/`--slack-user-token` → `korotovsky`); `slack-mcp`
  when a Slack MCP token is given or stored. The choice is always written
  to `slack_transport`. With no credential flag, no
  `$SLACK_MCP_TOKEN`, no stored credential, no `slack_transport` file and
  nothing taken over, it selects `hooks-only` (and `--no-hooks` is then
  refused with exit 2 before anything changes, unless exactly one server
  can be taken over): `request_slack_message` is not registered (earlier
  registrations are removed) and the instruction block becomes "use your
  own Slack tool; the user approves each call". Step 5b registers the
  approval hook in every detected agent with hooks
  (`agent_config.py hook-add AGENT FILE BIN WAIT`; `hook-remove`,
  `hook-has`): Claude Code `~/.claude/settings.json` (PreToolUse +
  ConfigChange; pdev reads it too), Codex `config.toml` (+ the
  `hooks.state` trust entry, `sha256` of the hook identity), Gemini CLI
  `settings.json` (timeout in ms), Copilot CLI `~/.copilot/hooks/`,
  Cursor `hooks.json` (`beforeMCPExecution`, `failClosed`), Vibe
  `hooks.toml` (`strict = true`), goose `~/.agents/plugins/…/hooks.json`
  (integer timeout), and plugin files for OpenCode, pi, Hermes (enabled in
  `config.yaml`) and Amp. pi and pdev get `request_slack_message` in their
  `mcp.json` (`timeout: 86400`; their `settings.json` is not read for MCP).
  Every registration carries a 24 h tool-call timeout in the agent's units,
  as an integer (a call waits for a human; defaults are Claude Code 30 min
  idle, Gemini 10 min, Hermes 5 min, Copilot 3 min, OpenCode 60 s): Claude
  Code, Gemini, Copilot and OpenCode `timeout: 86400000` (ms; above
  2^31 − 1 a JS timer fires at once), Hermes `timeout: 86400`, Codex and
  Vibe `tool_timeout_sec`, goose `timeout`; Zed `timeout: 600` (its cap;
  the gateway ends Zed calls at 590 s); Amp has no setting (a fixed
  5 min, after which it cancels the call). OpenCode's entry needs
  `"type": "local"` (it ignores one without), Copilot's gets
  `"tools": ["*"]`, Zed's is flat (`{"command": PATH, "args": [],
  "timeout": 600}`; the nested `{"command": {"path": …}}` form is not
  loaded). Claude Code is registered with `claude mcp add-json` (`mcp add`
  cannot set a timeout). On every run `agent_config.py upgrade-json STYLE
  FILE BIN` / `upgrade-zed FILE BIN` bring an existing entry up to date
  (the command, OpenCode's `type`, and a missing, shorter or out-of-range
  timeout; optional fields and keys the user added are left alone); a
  JSONC file is not rewritten (the fix is printed) and Verify reports that
  agent OUTDATED, saying whether the entry won't load or only its timeout
  is short. Zed's JSONC is edited by splicing, never re-serialized: members
  are found by a comment- and string-aware scan, SpeakEasy's entry goes on
  its own line (inline after `{` in inline-style objects) and removal takes
  that line and the comma that separated it, so install then uninstall
  restores the file byte for byte (line endings and BOM included) when it
  already had `context_servers`; a file that does not parse is refused. The installer warns about Hermes before 0.21.6 (every stdio MCP
  call fails) or 0.22.3 (calls cut off after 7 minutes), reading the
  version from `hermes-agent/install-stamp.json` (`baseVersion`), else a
  non-zero `pyproject.toml` version.
  The tool-hook command is a shell wrapper: `[ -x BIN ] || exit 2; BIN
  --hook A --wait W --state-dir S --ok-code 10; rc=$?; [ $rc -eq 10 ] &&
  exit 0; …; exit 2` (Cursor runs the binary directly, without
  `--ok-code`, and relies on `failClosed`; the plugins check the code
  themselves). The ConfigChange guard's wrapper fails open. TOML hook
  entries are found by content, never by a comment span (Codex inserts its
  own tables before a file's trailing comment): Codex's group is the
  `[[hooks.PreToolUse]]` followed by a `[[hooks.PreToolUse.hooks]]` running
  `--hook codex`, updated in place (the trust index comes from the file's
  groups; trust tables are matched by key or by the old group's hash, and
  removal renumbers the trust keys of later groups); Vibe's entry is the
  `[[hooks]]` named `speakeasy-slack-approval`. The Hermes
  `plugins.enabled` edit handles only block style (items at the key's
  indent or two deeper) and refuses anything else. JSON with comments is
  never rewritten: an entry or hook already present (found structurally)
  counts as `unchanged`, a missing one is printed for the user to add by
  hand, and a removal that finds nothing is `absent`. JSON is written with
  non-ASCII text as is. Every write replaces the file atomically through
  symlinks and keeps its mode. Backups (`.bak-sag`) are taken only while a
  file holds nothing of SpeakEasy's and is not the empty skeleton an
  uninstall leaves (`mcp_servers:` / `plugins:` / `enabled: []` YAML, JSON
  of empty containers); uninstall deletes a `.bak-sag` that holds only
  SpeakEasy's own instruction block. `has` is structural: a server entry
  under a top-level MCP key (JSON), a `[mcp_servers.NAME]` table or
  `[[mcp_servers]]` element (TOML), or a block under an MCP key (YAML),
  never a mention elsewhere (a hook command names the binary too). Step 5
  registrations are non-fatal: a failing agent prints `[failed]` and the
  install continues; "Verify registrations" then marks an agent whose hook
  failed as NOT GATED. Step 4 counts the service as up only when its socket
  answers an MCP `initialize` (the socket is bound before the Slack client
  is built, and a taken-over server can still fail); otherwise the warning
  quotes the error lines of `gateway.log`. A credential given (or stored)
  after a hooks-only install switches `slack_transport` to its transport;
  hooks-only with `--no-hooks` is refused; `--enforce` is skipped in
  hooks-only mode. An agent whose own settings switch its hooks off
  (Claude Code `disableAllHooks`, Codex `[features] hooks = false`, Gemini
  `hooksConfig`) gets the hook but is reported NOT GATED, not VERIFIED, and
  its instruction block is not a hooked variant. Commands the installer
  suggests are printed with its own path (`~/…/scripts/install.sh`), every
  run prints a `transport:` line, and `--dry-run` says "would write".
  `--uninstall` lists whatever it could not remove and
  exits 1 (a leftover hook fails closed). Instruction blocks, when SpeakEasy
  can send: `either` ("your Slack tool or `request_slack_message`; the user
  approves either") where the hook was registered, `gateway` (only
  `request_slack_message`) elsewhere; hooks-only: `hooks` where the hook was
  registered, `confirm` ("ask the user in chat first") elsewhere. `CODEX_HOME`, `HERMES_HOME`
  and `VIBE_HOME` select those agents' hook files. Before choosing a transport it takes over the
  Slack MCP the user already has: `agent_config.py takeover-candidates`
  lists the Slack MCP servers in agent configs (Claude Desktop and Cursor
  included) — a korotovsky stdio server with literal `SLACK_MCP_*` tokens,
  the official server with a bearer user token, a Mistral Slack connector
  with a bearer key — and the ones that cannot be taken over (OAuth
  sessions owned by an agent, Claude Code's Slack plugin); with no stored
  credential and exactly one importable server (copies of one server in
  several agents or project scopes share a hashed fingerprint and count
  once), `agent_config.py take-over` copies its command/env, token, or key
  and connector name into the state dir (chmod 600; replaced credential
  files are moved to `*.pre-takeover`, then `.pre-takeover.1`, `.2`, … so
  none is overwritten) and writes `slack_transport`. A remote server
  reached through a stdio bridge (`mcp-remote URL --header "Authorization:
  Bearer …"`) counts as that remote server; `${VAR}` in a header is filled
  in from the entry's own env.
  `--take-over FILE[:NAME]` reads exactly FILE (any MCP config); same-name
  entries that differ are `NAME#2`, `NAME#3`. A taken-over
  `SLACK_MCP_ADD_MESSAGE_TOOL` is kept only when it is a channel-ID
  allowlist; a docker/podman `run` line gets `-e NAME` for the variables
  SpeakEasy sets. TOML is read with tomllib, tomli, or pip's vendored copy
  (macOS's Python 3.9); without one, TOML servers are reported, not offered.
  `agent_config.py can-enforce FILE` tells the installer whether `--enforce`
  can edit a takeover source (goose, Zed and Hermes are edited by hand).
  `scripts/agent_config.py` is
  the per-agent config editor it uses (idempotent, `.bak-sag` backups);
  `scripts/test_agent_config.py` tests it. `scripts/slack_mcp_oauth.py`
  runs the Slack MCP user-token OAuth flow (local callback on port 3118,
  verifies the token against the MCP endpoint, writes
  `<state-dir>/slack_mcp_token` chmod 600).
- The installer's exclusivity audit (`agent_config.py audit`, run by
  `install.sh` step 7) categorizes what agents have installed: the official
  Slack MCP server (`mcp.slack.com`) and Mistral connector servers are
  **expected** (agents read Slack context through them; their sends are
  gated by the approval hook where the agent has one, and otherwise only
  asked about by the instruction block); direct token-based Slack servers
  (a `xox*` token in the entry, or an unclassifiable slack-named server)
  are listed as gated when the agent whose config holds them has
  SpeakEasy's approval hook (its writes through them wait for the user),
  and as **violations** otherwise (in hooks-only they are how agents send), as are Slack tokens and Mistral
  API keys found in agent configs. Slack servers in Claude Desktop and Zed
  are reported as NOT gated (no hooks there; Zed gets only the
  instruction block), with what to set by hand. Non-MCP Slack settings
  (e.g. Hermes's `platform_toolsets.slack`) are listed separately and
  never touched. `--enforce` removes only the direct token-based servers
  from the JSON/TOML configs it can edit and keeps the expected ones, and
  the installer suggests it only while such servers remain;
  `--dry-run` changes nothing, including under `--enforce`.
- Releases: pushing a tag `vX.Y.Z` runs `.github/workflows/release.yml`
  (macOS runner): the tag must equal `Cargo.toml`'s version; `cargo test`,
  `scripts/test_agent_config.py`, `scripts/smoke_test.py` and `bash -n
  scripts/install.sh` must pass; the aarch64 and x86_64 release builds
  (`MACOSX_DEPLOYMENT_TARGET=11.0`) are combined with `lipo`, signed with
  the Developer ID from the `MACOS_SIGNING_CERTIFICATE_P12` secret
  (hardened runtime, secure timestamp, in a temporary keychain that also
  holds Apple's Developer ID intermediates), notarized with `notarytool`
  (an App Store Connect API key, or an Apple ID and app-specific password;
  a bare binary can't be stapled), and checked (a non-Slack `--hook claude`
  call answers the OK code). A tag is never published unsigned; a manual
  run without the secrets is ad-hoc signed, with a warning. `speakeasy-macos-universal.tar.gz` holds
  `speakeasy-macos-universal/{bin/slack-approval-gateway, scripts/
  {install.sh, agent_config.py, slack_mcp_oauth.py}, README.md, LICENSE, NOTICE.md,
  THIRD_PARTY_LICENSES.md, VERSION}`; it and its `.sha256` are published
  with `gh release create --generate-notes` (`--prerelease` for a version
  with a `-`; a re-run replaces the assets). A manual run (workflow_dispatch)
  builds the same archive as a workflow artifact and publishes nothing.
  `install.sh` run from such an archive (a `bin/` binary and no
  `Cargo.toml` next to `scripts/`) installs that binary and never builds,
  and clears `com.apple.quarantine` on the installed copy.
- `slack-approval-gateway --version` / `--help` print and exit without
  starting or relaying to SpeakEasy.
- First builds compile eframe/wgpu and take several minutes; that's
  expected.
- `cargo build` / `cargo build --release` must succeed with zero errors.
- `cargo test` must pass (core + slack_tools + slack_stdio unit tests, Slack
  MCP client tests, korotovsky stdio client tests + transport-selection
  tests, mistral connector client tests, MCP e2e tests, the `UiState`
  bridge tests, and the native-UI tests: bridge lifecycle, Motion-matched
  spring/bezier math, Lucide path parsing, and the capture-derived layout
  geometry contract in `src/ui/app/paint/tests.rs`).
- `scripts/smoke_test.py` drives the real binary end to end: a mock Slack
  MCP server (the default transport) + mock Mistral platform API on one
  local server, a pty as the CLI terminal, the single-instance bridge path,
  one full approve flow with `SLACK_TRANSPORT=mistral-connector` (asserts
  the exact message went through the connector tool call and no Slack MCP
  call was made), one full approve flow with `SLACK_TRANSPORT=korotovsky`
  against `scripts/mock_korotovsky.py` spawned as a stdio child (asserts
  the gateway enabled `SLACK_MCP_ADD_MESSAGE_TOOL`, the send went through
  `conversations_add_message` with the exact message/destination as
  `text/plain`, the permalink was built from the reply, and no Slack MCP or
  connector call was made), and a destination display-name check (a Slack
  user ID resolves to a display name in the prompt while the send still
  carries the exact raw ID).
- Do not add dependencies beyond those in Cargo.toml unless strictly
  necessary; if you must, keep them minimal and say so in your summary.

## Out of scope

Do not write the README (the coordinator does that at the end). Do not
change `Cargo.toml`, `src/lib.rs`, `src/main.rs`, or files owned by other
tasks. Do not broaden the MCP surface beyond the one tool.
