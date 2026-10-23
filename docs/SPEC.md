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
  3. Build `MessageRequest { id, incoming: req, received_at: now }` with a
     unique ID (e.g. `req-<now>-<atomic counter>`). Call
     `provider.request_decision(&msg)` **without holding the state lock**
     (it may block for a long time).
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

### `src/slack.rs` — the only place credentials live

- `from_env(state_dir)`: token from `SLACK_BOT_TOKEN`, else read
  `<state_dir>/slack_token` (trim whitespace). Error if neither. API base
  from `SLACK_API_BASE` (default `https://slack.com/api`, trailing slash
  stripped).
- `new(token, api_base)`.
- `post_message`: `POST {api_base}/chat.postMessage` with
  `Authorization: Bearer <token>` and JSON body
  `{channel, text, thread_ts?}` (`thread_ts` only when `Some`). Slack's API
  accepts `application/json` bodies. Parse `{ok, error?, channel?, ts?}`;
  `!ok` → `SlackError(error or "unknown_error")`. Then resolve the permalink
  with `POST {api_base}/chat.getPermalink` `{channel, message_ts: ts}` →
  `{ok, permalink?}`. If that fails, fall back to a constructed URL
  `https://<SLACK_WORKSPACE_DOMAIN or "workspace">.slack.com/archives/{channel}/p{ts without '.'}`.
  Return `SlackSend { channel, ts, permalink }`.
- Use `ureq` 3.x (`features = ["json", "tls"]` already in Cargo.toml):
  `ureq::post(url).header(...).send_json(serde_json::json!(...))`, then
  `resp.body_mut().read_json::<serde_json::Value>()`. Handle `ureq::Error`
  (transport vs status) and timeouts (set a reasonable timeout, e.g. 15s).
- Tests (`tests/slack_client_test.rs`): a mock Slack API on a local
  `TcpListener` (127.0.0.1:0) that records method, path, `Authorization`
  header, and JSON body, and replies with canned JSON. Assert: exact
  `channel`/`text`/`thread_ts` in the body; `thread_ts` key absent when
  `None`; `SlackSend` fields from `chat.postMessage` + `chat.getPermalink`;
  `{ok:false,error}` → `SlackError`; `from_env` reads token file.

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
  `DecisionProvider`, and the mock Slack server from `tests/common/mod.rs`.
  Cover: initialize/tools/list shape; approve → mock server received the exact
  message/destination/thread and the agent got `sent` + permalink; deny →
  `denied` with 600 and a reworded same-destination retry is
  `cooldown_blocked` with remaining time and the provider was NOT called
  again; feedback → agent gets the text and the mock server saw no call;
  identical retry after approval → `sent` with `duplicate: true` and the
  mock server's post count unchanged; two concurrent requests with different
  content get correctly matched decisions (no mixing); a NEW `Gateway` on
  the same state dir (simulated restart) still returns `cooldown_blocked`.

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
Tailwind 4 + the **Mistral design system (`@mistral/ui`)**, vendored under
`ui/shared/` (copied from `mistralai/product-prototypes`' self-contained
template; components are imported via the `#components/...` path aliases).
`src/ui.rs` is the Rust↔webview bridge:

- `UiState`: pending-request map (`request_id -> Sender<Result<Decision,
  ProviderError>>`), an outbox of requests for a not-yet-ready webview,
  the webview-ready flag, the `AppHandle`, and the gateway (for the
  cooldown status line).
- `TauriDecisionProvider::request_decision`: register the request, deliver
  it to the webview (emit `approval-request`, or queue it in the outbox if
  the webview is not ready), then block on the single-use reply channel.
  If the app exits, the sender is dropped and the request fails with
  `ProviderError` — it is never approved. A single-use channel guarantees
  exactly one decision per request; repeated UI clicks cannot produce a
  second decision.
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
  `--uninstall` flags; see README §0). `scripts/agent_config.py` is the
  per-agent config editor it uses (idempotent, `.bak-sag` backups);
  `scripts/test_agent_config.py` tests it.
- First builds compile Tauri/wry and take several minutes; that's expected.
- `cargo build` / `cargo build --release` must succeed with zero errors.
- `cargo test` must pass (core unit tests, slack client tests, MCP e2e
  tests, and the `UiState` bridge tests).
- Do not add dependencies beyond those in Cargo.toml unless strictly
  necessary; if you must, keep them minimal and say so in your summary.

## Out of scope

Do not write the README (the coordinator does that at the end). Do not
change `Cargo.toml`, `src/lib.rs`, `src/main.rs`, or files owned by other
tasks. Do not broaden the MCP surface beyond the one tool.
