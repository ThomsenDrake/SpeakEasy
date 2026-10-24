# Slack Approval Gateway — Implementation Spec

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
     lock** (it may block for a long time).
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
plumbing. The only Slack credential the gateway can hold is a Slack **user
OAuth token** for the Slack MCP server (the default transport, implemented
in `src/slack_mcp.rs`); the alternative (`mistral-connector`) holds a
Mistral API key and lives in `src/mistral_slack.rs`. The old bot-token
(`direct`) transport was removed on purpose: the gateway is not a Slack app
and never posts as one.

- `resolve_transport(state_dir) -> Result<SlackTransport, SlackError>`:
  `SLACK_TRANSPORT` env (`slack-mcp` | `mistral-connector`; aliases `mcp` /
  `slack_mcp` / `mistral_connector` / `connector`), else a one-line
  `<state_dir>/slack_transport` file, else `SlackMcp`. Legacy values
  (`direct` / `bot-token` / `token`) are an error explaining the removal and
  pointing at the new transports; unknown values are an error naming the
  accepted ones.
- `build_client(state_dir) -> Result<Arc<dyn SlackClient>, SlackError>`:
  dispatch on the selected transport; `src/main.rs` calls this once at
  startup and exits with usage hints (naming what the selected transport
  needs: `SLACK_MCP_TOKEN`/`slack_mcp_token` + the OAuth helper for
  `slack-mcp`, `MISTRAL_API_KEY`/`mistral_api_key` for `mistral-connector`)
  on error.
- Shared `pub(crate)` helpers used by both transports: `non_empty_env`,
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

### `src/slack_tools.rs` — shared tool-call plumbing (both transports)

`pub(crate)` helpers shared by `slack_mcp` and `mistral_slack`:

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
  `MISTRAL_SLACK_CONNECTOR` (default `slack`), `MISTRAL_SLACK_TOOL` (pin
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
    Otherwise build `IncomingRequest`, call `gateway.handle_request(...)`,
    and respond with `result.content = [{type: "text", text: <JSON string>}]`
    where the JSON string is one of:
    - `{"status":"sent","ts":...,"permalink":...,"duplicate":bool}`
    - `{"status":"denied","cooldown_remaining_secs":600}`
    - `{"status":"feedback","feedback":"..."}`
    - `{"status":"cooldown_blocked","cooldown_remaining_secs":N,"note":"a similar request was denied recently; nothing was sent and no approval prompt was shown"}`
    - `{"status":"error","error":"..."}` (with `result.isError = true`)
    Also include `result.structuredContent` mirroring the same object.
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
  saw while the send still carries the raw ID.

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

### `src/ui.rs` — Tauri webview bridge (the approval popup)

The human-facing popup is the webview frontend in `ui/` — React 19 +
Tailwind 4 + the OSS **Once UI** design system (`@once-ui-system/core`;
its tokens and utility layer are imported in `ui/src/styles/globals.css`)
and animated primitives vendored from **Motion Primitives** under
`ui/src/components/motion-primitives/` (added with
`npx motion-primitives add <name>`; they import `cn` from `@/lib/utils`).
The theme (the `data-theme`/`data-brand`/… attributes on `<html>`) is
applied in `ui/src/main.tsx`, because the Tauri CSP forbids inline
scripts.
`src/ui.rs` is the Rust↔webview bridge:

- `UiState`: pending-request map (`request_id -> Sender<Result<Decision,
  ProviderError>>`), an outbox of requests for a not-yet-ready webview,
  the webview-ready flag, the `AppHandle`, and the gateway (for the
  cooldown status line).
- `TauriDecisionProvider::request_decision`: register the request, deliver
  it to the webview (emit `approval-request` with the whole `MessageRequest`,
  including `destination_display`, or queue it in the outbox if the webview
  is not ready), then block on the single-use reply channel.
  If the app exits, the sender is dropped and the request fails with
  `ProviderError` — it is never approved. A single-use channel guarantees
  exactly one decision per request; repeated UI clicks cannot produce a
  second decision.
- The frontend renders the destination as
  `destination_display ?? incoming.destination`, with the raw destination
  as a secondary tag when a display name was resolved (display only — the
  exact `destination` is what gets sent).
- Tauri commands invoked by the webview: `submit_decision(request_id,
  decision)` (first decision wins; repeats and unknown IDs are ignored),
  `ui_ready` (flushes the outbox — the frontend awaits its `listen()`
  registration before invoking this so no event is lost),
  `get_active_cooldowns`, `hide_window` (the webview calls this once its
  queue has drained after a decision), and `quit_app` (tray Quit).
- `UiState::reveal_window`: when a request is delivered (emitted or
  queued), the hidden main window is shown and focused, so a request
  arriving at any time surfaces the popup. `mark_webview_ready` reveals the
  window when it flushed queued requests.
- The frontend (`ui/src/App.tsx`) renders one request at a time with the
  design-system components (`Button`, `Typography`, `Badge`, `Card`,
  `Textarea`): the exact proposed message, the Slack destination (with a
  caption distinguishing it from the agent conversation), the reason, the
  originating agent conversation (context only — not a Slack thread), the
  three actions **Approve** / **Deny** / **Give feedback** (feedback text
  entry enabled only when non-empty), a "N waiting" badge, active deny
  cooldowns, and the caption "Closing this window never approves a message.
  If the app quits, pending requests fail and the agent is told nothing was
  sent."
- Closing the window only **hides** it (`CloseRequested` is intercepted:
  `window.hide()` + `api.prevent_close()`); the gateway process keeps
  serving MCP for every agent. Quitting happens only via the tray menu's
  **Quit gateway** (`quit_app` → `app.exit(0)`) or a signal; pending reply
  senders are then dropped, so blocked `request_decision` calls return
  `Err` — never an approval. A Dock click / `open` with no visible window
  (`RunEvent::Reopen`) shows the window again.
- A menu-bar tray icon (Tauri `tray-icon` feature) offers **Show approval
  window** / **Quit gateway**; a left click on the icon shows the window.
- Tauri 2 gates `listen`/`invoke` behind **capabilities**:
  `capabilities/default.json` grants `core:default` to the `main` window.
  Without it the webview renders but no IPC reaches the backend (a blank
  queue that never becomes ready).
- The `tauri` dependency enables the **`custom-protocol`** feature, so
  every build (debug and release) embeds `ui/dist` and serves it from
  `tauri://localhost` — no dev server is needed to run the binary. (Without
  it, Tauri expects a dev server at `devUrl` and the webview stays blank.)

### `src/main.rs` — run modes, tray, window lifecycle

Wires state dir (`SLACK_GATEWAY_STATE_DIR` or `~/.slack-approval-gateway`),
Slack client, provider, MCP server, and UI. Three run modes:

- `--cli`: headless. Becomes the instance (lock + socket), builds the
  gateway with the terminal fallback provider, serves the socket, and runs
  MCP on stdio. Used by the smoke test and for SSH hosts.
- `--serve`: service mode (LaunchAgent). Becomes the instance, builds the
  Tauri app, serves the socket, but runs no MCP on stdio.
- default (spawned by an agent over stdio): if an instance answers on the
  socket, bridge to it (`instance::run_bridge`, never returns); otherwise
  become the instance (lost startup races bridge instead). The instance
  serves MCP on its own stdio (unless `--serve`) plus the socket.

GUI mode builds the Tauri app (managed `UiState`, commands
`submit_decision`/`ui_ready`/`get_active_cooldowns`/`hide_window`/
`quit_app`, MCP server on a background thread), installs the tray icon in
`setup`, hides the window on close, and shows it on `RunEvent::Reopen`
(macOS). After the event loop ends, the app waits for any in-flight
approved send to finish before exiting. The window is created hidden
(`"visible": false` in `tauri.conf.json`) and revealed per request.

## Build, install, and test

- Rust toolchain (cargo 1.98+) and Node 24+ are installed. The frontend
  must be built before `cargo build` (Tauri embeds `ui/dist`):
  `cd ui && npm install && npm run build`.
- `scripts/install.sh` installs the app into every detected coding agent on
  this Mac and starts the LaunchAgent service (`--dry-run`, `--enforce`,
  `--uninstall`, `--slack-mcp-token`, `--state-dir`, `--transport`,
  `--mistral-api-key` flags; see README §0). `scripts/agent_config.py` is
  the per-agent config editor it uses (idempotent, `.bak-sag` backups);
  `scripts/test_agent_config.py` tests it. `scripts/slack_mcp_oauth.py`
  runs the Slack MCP user-token OAuth flow (local callback on port 3118,
  verifies the token against the MCP endpoint, writes
  `<state-dir>/slack_mcp_token` chmod 600).
- First builds compile Tauri/wry and take several minutes; that's expected.
- `cargo build` / `cargo build --release` must succeed with zero errors.
- `cargo test` must pass (core + slack_tools unit tests, Slack MCP client
  tests, mistral connector client tests + transport-selection tests, MCP
  e2e tests, and the `UiState` bridge tests).
- `scripts/smoke_test.py` drives the real binary end to end: a mock Slack
  MCP server (the default transport) + mock Mistral platform API on one
  local server, a pty as the CLI terminal, the single-instance bridge path,
  one full approve flow with `SLACK_TRANSPORT=mistral-connector` (asserts
  the exact message went through the connector tool call and no Slack MCP
  call was made), and a destination display-name check (a Slack user ID
  resolves to a display name in the prompt while the send still carries the
  exact raw ID).
- Do not add dependencies beyond those in Cargo.toml unless strictly
  necessary; if you must, keep them minimal and say so in your summary.

## Out of scope

Do not write the README (the coordinator does that at the end). Do not
change `Cargo.toml`, `src/lib.rs`, `src/main.rs`, or files owned by other
tasks. Do not broaden the MCP surface beyond the one tool.
