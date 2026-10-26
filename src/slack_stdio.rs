//! Slack sending via Korotovsky's unofficial Slack MCP server
//! (`slack-mcp-server`, <https://github.com/korotovsky/slack-mcp-server>)
//! over MCP stdio.
//!
//! The server command resolves from `SLACK_STDIO_COMMAND`, else a one-line
//! `<state_dir>/slack_stdio_command` file, else `slack-mcp-server` found on
//! PATH (run with `--transport stdio`, the server's documented stdio
//! invocation). SpeakEasy never installs the server itself; when none of
//! these resolves, startup fails with instructions. (A command taken over
//! from an agent's config runs as configured, so an `npx` line still
//! fetches the package the way it does for that agent.)
//!
//! The third [`SlackClient`] transport: the gateway spawns the server as a
//! child process and speaks MCP (newline-delimited JSON-RPC) over its
//! stdin/stdout. The gateway holds the Slack credential — a **bot** token
//! (`xoxb-…`, the default) or a **user** token (`xoxp-…`) — in
//! `SLACK_BOT_TOKEN`/`SLACK_USER_TOKEN` or `<state_dir>/slack_bot_token` /
//! `slack_user_token` (chmod 600), and passes it to the child as
//! `SLACK_MCP_XOXB_TOKEN`/`SLACK_MCP_XOXP_TOKEN`. Agents never see it.
//!
//! The server's send tool is `conversations_add_message`, disabled by
//! default for safety: the gateway sets `SLACK_MCP_ADD_MESSAGE_TOOL=true`
//! when spawning (unless the operator set it already) and fails at startup
//! when the tool is not listed. Messages go out as `text/plain`
//! (`SLACK_STDIO_CONTENT_TYPE`, the default) so the exact approved text is
//! what Slack renders, without markdown interpretation; `text/markdown` is
//! available as an override.
//!
//! Tool discovery, argument-name detection and reply parsing are shared with
//! the other transports in [`crate::slack_tools`]. The reply of
//! `conversations_add_message` is plain text (`Successfully posted message
//! to channel … (ts=…)`), so the permalink is built with
//! [`crate::slack::fallback_permalink_with_domain`] from the workspace
//! domain (`SLACK_WORKSPACE_DOMAIN`, else `<state_dir>/slack_workspace_domain`).

use crate::core::{SlackClient, SlackError, SlackSend};
use crate::slack::{
    fallback_permalink_with_domain, non_empty_env, warn_if_readable_by_others, workspace_domain,
};
use crate::slack_tools::{
    is_send_message_tool, pick_argument, schema_properties, send_arguments, send_outcome,
    text_content, tool_result_error, SendToolSpec, CHANNEL_ARG_CANDIDATES, THREAD_ARG_CANDIDATES,
};
use serde_json::{json, Map, Value};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

/// MCP protocol version offered during `initialize`.
const PROTOCOL_VERSION: &str = "2025-06-18";

/// Upper bound on each MCP call (initialize, tools/list, tools/call).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// The server program looked up on PATH when nothing else configures the
/// command. Its npm bin is a node wrapper around the native binary, so both
/// it and `node` must be reachable; the wrapper passes the arguments through.
const PATH_SERVER_PROGRAM: &str = "slack-mcp-server";

/// Candidate property names for the message-text argument. `text` is the
/// server's current parameter; `payload` is accepted by older versions.
const TEXT_ARG_CANDIDATES: &[&str] = &["text", "payload", "message", "content", "body"];

/// Candidate argument names for the user-search tool.
const USER_SEARCH_ARG_CANDIDATES: &[&str] = &["query", "q", "user_id", "user", "id"];

/// Candidate argument names for the channel-list tool.
const CHANNEL_LIST_ARG_CANDIDATES: &[&str] = &["channel_types", "types"];

/// How many stderr lines to keep for error reporting.
const STDERR_TAIL_LINES: usize = 25;

/// A [`SlackClient`] that spawns and drives the korotovsky Slack MCP server
/// over stdio.
///
/// The send tool is either pinned with `SLACK_STDIO_TOOL` or discovered once
/// from the server's `tools/list` (`conversations_add_message`, with
/// argument names detected from the tool's declared `inputSchema`).
/// `SLACK_STDIO_CHANNEL_ARG`, `SLACK_STDIO_TEXT_ARG` and
/// `SLACK_STDIO_THREAD_ARG` override the detected names.
pub struct KorotovskyClient {
    tool: Option<String>,
    /// Explicit argument-name overrides (`None` = detect from the schema).
    channel_arg: Option<String>,
    text_arg: Option<String>,
    thread_arg: Option<String>,
    /// `text/plain` (default) or `text/markdown`.
    content_type: String,
    /// The workspace domain for fallback permalinks: `SLACK_WORKSPACE_DOMAIN`
    /// wins over the `<state_dir>/slack_workspace_domain` file.
    workspace_domain: String,
    /// The child's stdin, the reply channel and the RPC id counter. One
    /// mutex: MCP requests are serialized, which also keeps id matching
    /// trivial.
    session: Mutex<StdioSession>,
    /// The server process; killed when the client is dropped.
    child: Mutex<Child>,
}

/// Per-session MCP state.
struct StdioSession {
    stdin: ChildStdin,
    /// Every parsed JSON-RPC message the server sends (responses and
    /// notifications); responses are matched by id, the rest dropped.
    rx: mpsc::Receiver<Value>,
    next_id: u64,
    /// Cached `tools/list` result (fetched during the handshake).
    tools: Vec<Value>,
    /// Cached send-tool resolution.
    send_tool: Option<SendToolSpec>,
    /// The server's last stderr lines, for error messages.
    stderr_tail: Arc<Mutex<VecDeque<String>>>,
}

impl KorotovskyClient {
    /// Build from the environment: `SLACK_BOT_TOKEN` and/or `SLACK_USER_TOKEN`
    /// (or the token files at `<state_dir>/slack_bot_token` /
    /// `slack_user_token`, one line, trimmed, chmod 600). At least one
    /// credential is required, unless the server's own tokens
    /// (`SLACK_MCP_XOXP_TOKEN`, `SLACK_MCP_XOXB_TOKEN`, or the browser-session
    /// pair `SLACK_MCP_XOXC_TOKEN` + `SLACK_MCP_XOXD_TOKEN`) are in the
    /// environment or in `<state_dir>/slack_stdio_env` — the settings the
    /// installer copies when it takes over a korotovsky server an agent
    /// already uses. The child gets that file's variables; the operator's
    /// environment wins over it.
    /// The server command resolves from `SLACK_STDIO_COMMAND`, else a
    /// one-line `<state_dir>/slack_stdio_command` file, else
    /// `slack-mcp-server` on PATH (run with `--transport stdio`); SpeakEasy
    /// never installs the server itself. `SLACK_STDIO_CONTENT_TYPE`
    /// picks `text/plain` (default) or `text/markdown`. The MCP handshake
    /// runs here so a bad token or a missing server fails at startup, not
    /// after an approval.
    pub fn from_env(state_dir: &Path) -> Result<Self, SlackError> {
        let bot = match non_empty_env("SLACK_BOT_TOKEN") {
            Some(token) => Some(token),
            None => read_optional_secret(&state_dir.join("slack_bot_token"), "Slack bot token")?,
        };
        let user = match non_empty_env("SLACK_USER_TOKEN") {
            Some(token) => Some(token),
            None => read_optional_secret(&state_dir.join("slack_user_token"), "Slack user token")?,
        };
        // The installer's placeholders are not credentials: passed to the
        // server, a placeholder bot token would win over a real browser
        // session (the server prefers xoxp, then xoxb, then xoxc/xoxd).
        let bot = real_token(bot);
        let user = real_token(user);
        // The server's own token variables count too: inherited from the
        // operator's environment, or taken over from an agent's MCP config
        // into <state_dir>/slack_stdio_env by the installer. A browser
        // session (xoxc + xoxd) needs no Slack app.
        let server_env = read_stdio_env(state_dir)?;
        let server_var = |name: &str| {
            non_empty_env(name).is_some()
                || server_env.iter().any(|(k, v)| k == name && !v.trim().is_empty())
        };
        let server_credential = server_var("SLACK_MCP_XOXP_TOKEN")
            || server_var("SLACK_MCP_XOXB_TOKEN")
            || (server_var("SLACK_MCP_XOXC_TOKEN") && server_var("SLACK_MCP_XOXD_TOKEN"));
        if bot.is_none() && user.is_none() && !server_credential {
            return Err(SlackError(format!(
                "no Slack token for the korotovsky transport: run the installer to take over \
                 the korotovsky server you already use (it copies its tokens into \
                 {}/slack_stdio_env), set SLACK_BOT_TOKEN (xoxb-…) or SLACK_USER_TOKEN \
                 (xoxp-…), or create {}/slack_bot_token or {}/slack_user_token (chmod 600)",
                state_dir.display(),
                state_dir.display(),
                state_dir.display()
            )));
        }
        let command = resolve_command(state_dir)?;
        let content_type = non_empty_env("SLACK_STDIO_CONTENT_TYPE")
            .unwrap_or_else(|| "text/plain".to_string());
        let mut client = Self::new(bot, user, &command, &content_type, state_dir)?;
        client.tool = non_empty_env("SLACK_STDIO_TOOL");
        client.channel_arg = non_empty_env("SLACK_STDIO_CHANNEL_ARG");
        client.text_arg = non_empty_env("SLACK_STDIO_TEXT_ARG");
        client.thread_arg = non_empty_env("SLACK_STDIO_THREAD_ARG");
        Ok(client)
    }

    /// Build explicitly (used by tests with a mock server) and run the MCP
    /// handshake: `initialize`, then `notifications/initialized`, then
    /// `tools/list` to fail fast when the send tool is not registered.
    pub fn new(
        bot_token: Option<String>,
        user_token: Option<String>,
        command: &str,
        content_type: &str,
        state_dir: &Path,
    ) -> Result<Self, SlackError> {
        for (kind, token) in [("bot", &bot_token), ("user", &user_token)] {
            if let Some(token) = token {
                if token.chars().any(|c| c.is_whitespace() || c.is_control()) {
                    return Err(SlackError(format!(
                        "the Slack {kind} token must be a single token with no spaces or line breaks"
                    )));
                }
            }
        }
        let content_type = validate_content_type(content_type)?;
        let (program, args) = split_command(command)?;
        let workspace_domain = workspace_domain(state_dir);
        let server_env = read_stdio_env(state_dir)?;

        let mut cmd = Command::new(&program);
        cmd.args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // The server's own settings taken over from an agent's MCP config
        // (tokens included); the operator's environment wins over the file.
        for (key, value) in &server_env {
            if std::env::var_os(key).is_none() {
                cmd.env(key, value);
            }
        }
        let is_set = |key: &str| {
            std::env::var_os(key).is_some() || server_env.iter().any(|(k, _)| k == key)
        };
        // The credential(s) the gateway holds. They are passed explicitly so
        // the gateway's files are the only place they live.
        if let Some(token) = &bot_token {
            cmd.env("SLACK_MCP_XOXB_TOKEN", token);
        }
        if let Some(token) = &user_token {
            cmd.env("SLACK_MCP_XOXP_TOKEN", token);
        }
        // Defaults the gateway controls; the operator's own values (and the
        // taken-over server settings) win.
        // `conversations_add_message` is disabled unless this is set, and a
        // quiet log level keeps the stdio stream clean.
        if !is_set("SLACK_MCP_ADD_MESSAGE_TOOL") {
            cmd.env("SLACK_MCP_ADD_MESSAGE_TOOL", "true");
        }
        if !is_set("SLACK_MCP_LOG_LEVEL") {
            cmd.env("SLACK_MCP_LOG_LEVEL", "error");
        }
        // Keep the server's caches inside the gateway's state dir: they hold
        // workspace data, and they must not collide with agent-side
        // instances of the same server.
        if !is_set("SLACK_MCP_USERS_CACHE") {
            cmd.env(
                "SLACK_MCP_USERS_CACHE",
                state_dir.join("korotovsky_users_cache.json"),
            );
        }
        if !is_set("SLACK_MCP_CHANNELS_CACHE") {
            cmd.env(
                "SLACK_MCP_CHANNELS_CACHE",
                state_dir.join("korotovsky_channels_cache.json"),
            );
        }
        let mut child = cmd.spawn().map_err(|e| {
            SlackError(format!(
                "cannot start the Slack MCP server '{command}': {e} (check the server command: \
                 SLACK_STDIO_COMMAND, the slack_stdio_command file in {}, or `slack-mcp-server` \
                 on PATH — https://github.com/korotovsky/slack-mcp-server)",
                state_dir.display()
            ))
        })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| SlackError("cannot open the Slack MCP server's stdin".to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| SlackError("cannot open the Slack MCP server's stdout".to_string()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| SlackError("cannot open the Slack MCP server's stderr".to_string()))?;

        // One thread per stream: stdout carries JSON-RPC (anything else is
        // skipped — a bad SLACK_MCP_LOG_LEVEL prints to stdout), stderr is
        // kept as a short tail for error messages.
        let (tx, rx) = mpsc::channel::<Value>();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if let Ok(message) = serde_json::from_str::<Value>(line) {
                    if tx.send(message).is_err() {
                        break;
                    }
                }
            }
        });
        let stderr_tail: Arc<Mutex<VecDeque<String>>> = Arc::new(Mutex::new(VecDeque::new()));
        {
            let tail = Arc::clone(&stderr_tail);
            std::thread::spawn(move || {
                for line in BufReader::new(stderr).lines() {
                    let Ok(line) = line else { break };
                    let mut guard = tail.lock().unwrap_or_else(|e| e.into_inner());
                    if guard.len() >= STDERR_TAIL_LINES {
                        guard.pop_front();
                    }
                    guard.push_back(line);
                }
            });
        }

        let mut session = StdioSession {
            stdin,
            rx,
            next_id: 0,
            tools: Vec::new(),
            send_tool: None,
            stderr_tail,
        };
        if let Err(e) = handshake(&mut session) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(SlackError(format!(
                "Slack MCP server '{command}' handshake failed: {e}{}",
                stderr_suffix(&session.stderr_tail)
            )));
        }
        Ok(Self {
            tool: None,
            channel_arg: None,
            text_arg: None,
            thread_arg: None,
            content_type,
            workspace_domain,
            session: Mutex::new(session),
            child: Mutex::new(child),
        })
    }

    /// Run `f` with the session locked (serializes all MCP traffic).
    fn with_session<T>(
        &self,
        f: impl FnOnce(&mut StdioSession) -> Result<T, SlackError>,
    ) -> Result<T, SlackError> {
        let mut session = self.session.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut session)
    }

    /// The server's tool list (fetched during the handshake, cached).
    fn tools(&self) -> Result<Vec<Value>, SlackError> {
        self.with_session(|s| Ok(s.tools.clone()))
    }

    /// The send-message tool: pinned, or resolved once from the tool list
    /// (`conversations_add_message` first, then any send-message tool), with
    /// argument names resolved from its `inputSchema`.
    fn send_tool(&self) -> Result<SendToolSpec, SlackError> {
        if let Some(tool) = &self.tool {
            return Ok(SendToolSpec {
                name: tool.clone(),
                channel_arg: self
                    .channel_arg
                    .clone()
                    .unwrap_or_else(|| "channel_id".to_string()),
                text_arg: self.text_arg.clone().unwrap_or_else(|| "text".to_string()),
                thread_arg: Some(
                    self.thread_arg
                        .clone()
                        .unwrap_or_else(|| "thread_ts".to_string()),
                ),
            });
        }
        self.with_session(|s| {
            if let Some(cached) = &s.send_tool {
                return Ok(cached.clone());
            }
            let spec = resolve_korotovsky_send_tool(
                &s.tools,
                self.channel_arg.clone(),
                self.text_arg.clone(),
                self.thread_arg.clone(),
            )?;
            s.send_tool = Some(spec.clone());
            Ok(spec)
        })
    }

    /// Call a tool (`tools/call`) and return the CallToolResult.
    fn call_tool(&self, tool: &str, arguments: Map<String, Value>) -> Result<Value, SlackError> {
        self.with_session(|s| {
            rpc(
                s,
                "tools/call",
                json!({ "name": tool, "arguments": Value::Object(arguments) }),
            )
        })
    }

    /// The first tool whose name satisfies `predicate`.
    fn find_tool(&self, predicate: impl Fn(&str) -> bool) -> Option<Value> {
        self.tools()
            .ok()?
            .into_iter()
            .find(|t| t.get("name").and_then(Value::as_str).is_some_and(|n| predicate(n)))
    }

    /// `Drake Thomsen (@drake.thomsen)` for a user ID, via the server's
    /// `users_search` tool (a user ID is looked up directly): the reply is
    /// CSV (`UserID,UserName,RealName,DisplayName,…`).
    fn resolve_user(&self, user_id: &str) -> Option<String> {
        let tool = self.find_tool(|n| {
            let n = n.to_ascii_lowercase();
            n == "users_search" || (n.contains("user") && n.contains("search"))
        })?;
        let name = tool.get("name").and_then(Value::as_str)?;
        let arg = pick_argument(
            schema_properties(&tool).as_ref(),
            USER_SEARCH_ARG_CANDIDATES,
            "query",
        );
        let mut arguments = Map::new();
        arguments.insert(arg, Value::String(user_id.to_string()));
        let reply = self.call_tool(name, arguments).ok()?;
        let rows = parse_csv(&text_content(&reply)?);
        let header = rows.first()?;
        let col = |row: &[String], name: &str| -> Option<String> {
            let i = header
                .iter()
                .position(|h| h.trim().eq_ignore_ascii_case(name))?;
            row.get(i).cloned().filter(|v| !v.is_empty())
        };
        for row in rows.iter().skip(1) {
            if col(row, "UserID").as_deref() == Some(user_id) {
                let real = col(row, "RealName");
                let display = col(row, "DisplayName");
                let username = col(row, "UserName");
                let name = real.or(display).or_else(|| username.clone())?;
                return Some(format_name(name, username));
            }
        }
        None
    }

    /// `#name` for a channel ID, via the server's `channels_list` tool: the
    /// reply is CSV (`ID,Name,Topic,Purpose,MemberCount,Cursor`).
    fn resolve_channel(&self, channel_id: &str) -> Option<String> {
        let tool = self.find_tool(|n| {
            let n = n.to_ascii_lowercase();
            n == "channels_list" || (n.contains("channel") && n.contains("list"))
        })?;
        let name = tool.get("name").and_then(Value::as_str)?;
        let arg = pick_argument(
            schema_properties(&tool).as_ref(),
            CHANNEL_LIST_ARG_CANDIDATES,
            "channel_types",
        );
        let mut arguments = Map::new();
        arguments.insert(arg, Value::String("public_channel,private_channel,im,mpim".to_string()));
        let reply = self.call_tool(name, arguments).ok()?;
        let rows = parse_csv(&text_content(&reply)?);
        let header = rows.first()?;
        let col = |row: &[String], name: &str| -> Option<String> {
            let i = header
                .iter()
                .position(|h| h.trim().eq_ignore_ascii_case(name))?;
            row.get(i).cloned().filter(|v| !v.is_empty())
        };
        for row in rows.iter().skip(1) {
            if col(row, "ID").as_deref() == Some(channel_id) {
                let channel_name = col(row, "Name")?;
                return Some(format!("#{channel_name}"));
            }
        }
        None
    }
}

/// `Name (@handle)` when a handle differs from the name, else just `Name`.
fn format_name(name: String, handle: Option<String>) -> String {
    match handle {
        Some(h) if h != name => format!("{name} (@{h})"),
        _ => name,
    }
}

impl std::fmt::Debug for KorotovskyClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KorotovskyClient")
            .field("tokens", &"<redacted>")
            .field("tool", &self.tool)
            .field("content_type", &self.content_type)
            .finish()
    }
}

impl Drop for KorotovskyClient {
    fn drop(&mut self) {
        // The server is our child: it must not outlive the gateway.
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl SlackClient for KorotovskyClient {
    /// Call the server's send tool with the exact `text` to the exact
    /// `destination` (optionally as a reply in thread `thread_ts`) and report
    /// the permalink. The server replies with plain text
    /// (`Successfully posted message to channel … (ts=…)`); the permalink is
    /// built from the channel and ts.
    fn post_message(
        &self,
        destination: &str,
        thread_ts: Option<&str>,
        text: &str,
    ) -> Result<SlackSend, SlackError> {
        let tool = self.send_tool()?;
        let mut arguments = send_arguments(&tool, destination, thread_ts, text)?;
        // Exact-text fidelity: text/plain disables Slack's markdown
        // interpretation of the approved text.
        arguments.insert(
            "content_type".to_string(),
            Value::String(self.content_type.clone()),
        );
        let reply = self.call_tool(&tool.name, arguments)?;
        if let Some(error) = tool_result_error(&reply) {
            return Err(SlackError(format!(
                "Slack MCP tool {} failed: {error}",
                tool.name
            )));
        }
        let mut outcome = send_outcome(&reply);
        if outcome.ts.is_none() || outcome.channel.is_none() {
            if let Some(text) = text_content(&reply) {
                if let Some((channel, ts)) = parse_add_message_reply(&text) {
                    if outcome.channel.is_none() {
                        outcome.channel = Some(channel);
                    }
                    if outcome.ts.is_none() {
                        outcome.ts = Some(ts);
                    }
                }
            }
        }
        let channel = outcome.channel.unwrap_or_else(|| destination.to_string());
        let ts = outcome.ts.unwrap_or_default();
        let permalink = outcome
            .permalink
            .or_else(|| {
                if ts.is_empty() {
                    None
                } else {
                    Some(fallback_permalink_with_domain(
                        &channel,
                        &ts,
                        &self.workspace_domain,
                    ))
                }
            })
            .unwrap_or_default();
        Ok(SlackSend {
            channel,
            ts,
            permalink,
        })
    }

    /// Best-effort display name for the approval popup: `U…` user IDs via
    /// `users_search`, `C…`/`G…` channel IDs via `channels_list`.
    /// `#name`/`@name` destinations and `D…` DM IDs return `None`. Any
    /// failure returns `None`.
    fn resolve_destination(&self, destination: &str) -> Option<String> {
        let d = destination.trim();
        if d.len() < 2 || d.starts_with('#') || d.starts_with('@') {
            return None;
        }
        match &d[..1] {
            "U" => self.resolve_user(d),
            "C" | "G" => self.resolve_channel(d),
            _ => None,
        }
    }
}

/// Read a one-line secret from `path`, when it exists. Unlike the required
/// secret files of the other transports, a missing file is `Ok(None)`: both
/// the bot and the user token are optional individually (one of them is
/// required).
fn read_optional_secret(path: &Path, what: &str) -> Result<Option<String>, SlackError> {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(SlackError(format!(
                "cannot read {what} file {}: {e}",
                path.display()
            )))
        }
    };
    let secret = contents.trim();
    if secret.is_empty() {
        return Err(SlackError(format!(
            "{what} file {} is empty",
            path.display()
        )));
    }
    Ok(Some(secret.to_string()))
}

/// The token, unless it is one of the installer's placeholders
/// (`xoxb-placeholder-…`, `xoxp-placeholder-…`).
fn real_token(token: Option<String>) -> Option<String> {
    token.filter(|t| !t.starts_with("xoxb-placeholder-") && !t.starts_with("xoxp-placeholder-"))
}

/// `KEY=VALUE` lines from `<state_dir>/slack_stdio_env`: the server's own
/// environment (`SLACK_MCP_*` tokens and settings), written by the
/// installer when it takes over a korotovsky server an agent already uses.
/// A missing file is no settings; blank lines and `#` comments are skipped.
fn read_stdio_env(state_dir: &Path) -> Result<Vec<(String, String)>, SlackError> {
    let path = state_dir.join("slack_stdio_env");
    let contents = match std::fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => {
            return Err(SlackError(format!(
                "cannot read the Slack MCP server settings file {}: {e}",
                path.display()
            )))
        }
    };
    warn_if_readable_by_others(&path);
    let mut vars = Vec::new();
    for (i, line) in contents.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err(SlackError(format!(
                "{} line {}: expected KEY=VALUE",
                path.display(),
                i + 1
            )));
        };
        let key = key.trim();
        if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(SlackError(format!(
                "{} line {}: '{key}' is not a variable name",
                path.display(),
                i + 1
            )));
        }
        vars.push((key.to_string(), value.trim().to_string()));
    }
    Ok(vars)
}

/// `text/plain` (the default) or `text/markdown`.
fn validate_content_type(value: &str) -> Result<String, SlackError> {
    let value = value.trim();
    if value == "text/plain" || value == "text/markdown" {
        Ok(value.to_string())
    } else {
        Err(SlackError(format!(
            "SLACK_STDIO_CONTENT_TYPE must be 'text/plain' or 'text/markdown' (got '{value}')"
        )))
    }
}

/// Resolve the Slack MCP server command. SpeakEasy never installs the
/// server itself — it runs a command it is given or one already on PATH.
/// Precedence:
///   1. `SLACK_STDIO_COMMAND` (a full command line, e.g.
///      `slack-mcp-server --transport stdio`);
///   2. a one-line `<state_dir>/slack_stdio_command` file (same format);
///   3. `slack-mcp-server` found on PATH, run with `--transport stdio`.
/// When none resolves, the error says the server is not installed and how
/// to point SpeakEasy at it.
fn resolve_command(state_dir: &Path) -> Result<String, SlackError> {
    resolve_command_from(
        state_dir,
        non_empty_env("SLACK_STDIO_COMMAND").as_deref(),
        std::env::var_os("PATH").as_deref(),
    )
}

/// The resolution behind [`resolve_command`], with the environment inputs
/// passed explicitly so tests need not touch the process environment.
fn resolve_command_from(
    state_dir: &Path,
    env_command: Option<&str>,
    path_var: Option<&std::ffi::OsStr>,
) -> Result<String, SlackError> {
    if let Some(command) = env_command.map(str::trim).filter(|c| !c.is_empty()) {
        return Ok(command.to_string());
    }
    let file = state_dir.join("slack_stdio_command");
    match std::fs::read_to_string(&file) {
        Ok(contents) => {
            let command = contents.trim();
            if command.is_empty() {
                return Err(SlackError(format!(
                    "the Slack MCP server command file {} is empty",
                    file.display()
                )));
            }
            return Ok(command.to_string());
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            return Err(SlackError(format!(
                "cannot read the Slack MCP server command file {}: {e}",
                file.display()
            )))
        }
    }
    if let Some(path) = find_on_path(PATH_SERVER_PROGRAM, path_var) {
        return Ok(format!("{} --transport stdio", path.display()));
    }
    Err(SlackError(format!(
        "the korotovsky Slack MCP server is not installed: `{PATH_SERVER_PROGRAM}` was not \
         found on PATH. SpeakEasy does not install it — install it yourself from \
         https://github.com/korotovsky/slack-mcp-server, then either leave it on PATH, write \
         its command to {} (e.g. 'slack-mcp-server --transport stdio'), or set \
         SLACK_STDIO_COMMAND when starting the gateway",
        file.display()
    )))
}

/// The absolute path of `program` on `path_var` (an executable file), if any.
fn find_on_path(program: &str, path_var: Option<&std::ffi::OsStr>) -> Option<std::path::PathBuf> {
    let path_var = path_var?;
    std::env::split_paths(path_var)
        .map(|dir| dir.join(program))
        .find(|candidate| is_executable(candidate))
}

/// Whether `path` is an executable file (a directory named like the program
/// is not enough).
#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    match std::fs::metadata(path) {
        Ok(meta) => meta.is_file() && meta.permissions().mode() & 0o111 != 0,
        Err(_) => false,
    }
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path).map(|m| m.is_file()).unwrap_or(false)
}

/// Split a command line into program + arguments (whitespace-separated; no
/// quoting — paths with spaces need a wrapper script).
fn split_command(command: &str) -> Result<(String, Vec<String>), SlackError> {
    let mut parts = command.split_whitespace();
    let program = parts.next().unwrap_or_default().to_string();
    if program.is_empty() {
        return Err(SlackError(
            "SLACK_STDIO_COMMAND is empty (expected e.g. 'slack-mcp-server --transport stdio')"
                .to_string(),
        ));
    }
    Ok((program, parts.map(str::to_string).collect()))
}

/// The MCP handshake: `initialize`, `notifications/initialized`, then
/// `tools/list` — failing fast when the send tool is not registered (the
/// server registers `conversations_add_message` only when
/// `SLACK_MCP_ADD_MESSAGE_TOOL` is set; the gateway sets it by default).
fn handshake(session: &mut StdioSession) -> Result<(), SlackError> {
    let init = rpc(
        session,
        "initialize",
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": { "name": "slack-approval-gateway", "version": env!("CARGO_PKG_VERSION") }
        }),
    )?;
    if init.get("protocolVersion").and_then(Value::as_str).is_none() {
        return Err(SlackError(
            "Slack MCP initialize: unexpected reply (no protocolVersion)".to_string(),
        ));
    }
    notify(session, "notifications/initialized", json!({}))?;
    let result = rpc(session, "tools/list", json!({}))?;
    let tools = result
        .get("tools")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| SlackError("Slack MCP: unexpected tools/list reply".to_string()))?;
    // Verify the send tool exists before serving any request.
    resolve_korotovsky_send_tool(&tools, None, None, None)?;
    session.tools = tools;
    Ok(())
}

/// One JSON-RPC request/response round trip over stdio (one message per
/// line). Notifications and unrelated messages are dropped while waiting
/// for our id; a dead server surfaces its stderr tail.
fn rpc(session: &mut StdioSession, method: &str, params: Value) -> Result<Value, SlackError> {
    session.next_id += 1;
    let id = session.next_id;
    let body = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
    let line = serde_json::to_string(&body)
        .map_err(|e| SlackError(format!("Slack MCP {method}: cannot encode request: {e}")))?;
    session
        .stdin
        .write_all(line.as_bytes())
        .and_then(|()| session.stdin.write_all(b"\n"))
        .and_then(|()| session.stdin.flush())
        .map_err(|e| SlackError(format!("Slack MCP {method}: cannot write to the server: {e}")))?;
    let deadline = Instant::now() + REQUEST_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(SlackError(format!(
                "Slack MCP {method}: no reply within {}s; the request may still have reached the server",
                REQUEST_TIMEOUT.as_secs()
            )));
        }
        match session.rx.recv_timeout(remaining) {
            Ok(message) => {
                if message.get("id").and_then(Value::as_u64) != Some(id) {
                    continue;
                }
                if let Some(error) = message.get("error") {
                    let detail = error
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown error");
                    return Err(SlackError(format!("Slack MCP {method}: {detail}")));
                }
                return Ok(message.get("result").cloned().unwrap_or(Value::Null));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                return Err(SlackError(format!(
                    "Slack MCP {method}: no reply within {}s; the request may still have reached the server",
                    REQUEST_TIMEOUT.as_secs()
                )))
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(SlackError(format!(
                    "Slack MCP {method}: the server exited before replying{}",
                    stderr_suffix(&session.stderr_tail)
                )));
            }
        }
    }
}

/// A JSON-RPC notification (no response expected).
fn notify(session: &mut StdioSession, method: &str, params: Value) -> Result<(), SlackError> {
    let body = json!({ "jsonrpc": "2.0", "method": method, "params": params });
    let line = serde_json::to_string(&body)
        .map_err(|e| SlackError(format!("Slack MCP {method}: cannot encode notification: {e}")))?;
    session
        .stdin
        .write_all(line.as_bytes())
        .and_then(|()| session.stdin.write_all(b"\n"))
        .and_then(|()| session.stdin.flush())
        .map_err(|e| SlackError(format!("Slack MCP {method}: cannot write to the server: {e}")))
}

/// ` (…)…server stderr tail…` for error messages (empty when there is none).
fn stderr_suffix(stderr_tail: &Arc<Mutex<VecDeque<String>>>) -> String {
    let guard = stderr_tail.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_empty() {
        return String::new();
    }
    let tail: Vec<String> = guard
        .iter()
        .skip(guard.len().saturating_sub(5))
        .cloned()
        .collect();
    format!("\n  server stderr: {}", tail.join("\n  "))
}

/// Resolve the korotovsky server's send tool from a tool list:
/// `conversations_add_message` first (the server's own send tool, not
/// matched by the generic send-message pattern), then any tool whose name
/// looks like "send a message". Argument names are detected from the
/// declared `inputSchema` (`channel_id`/`text`/`thread_ts`).
fn resolve_korotovsky_send_tool(
    tools: &[Value],
    channel_override: Option<String>,
    text_override: Option<String>,
    thread_override: Option<String>,
) -> Result<SendToolSpec, SlackError> {
    let found = tools
        .iter()
        .find(|t| t.get("name").and_then(Value::as_str) == Some("conversations_add_message"))
        .or_else(|| {
            tools
                .iter()
                .find(|t| t.get("name").and_then(Value::as_str).is_some_and(is_send_message_tool))
        });
    match found {
        Some(tool) => {
            let name = tool
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let props = schema_properties(tool);
            Ok(SendToolSpec {
                channel_arg: channel_override
                    .unwrap_or_else(|| pick_argument(props.as_ref(), CHANNEL_ARG_CANDIDATES, "channel_id")),
                text_arg: text_override
                    .unwrap_or_else(|| pick_argument(props.as_ref(), TEXT_ARG_CANDIDATES, "text")),
                thread_arg: Some(
                    thread_override.unwrap_or_else(|| {
                        pick_argument(props.as_ref(), THREAD_ARG_CANDIDATES, "thread_ts")
                    }),
                ),
                name,
            })
        }
        None => {
            let names: Vec<&str> = tools
                .iter()
                .filter_map(|t| t.get("name").and_then(Value::as_str))
                .collect();
            Err(SlackError(format!(
                "no send-message tool in the list (available: {}); the korotovsky server \
                 registers conversations_add_message only when SLACK_MCP_ADD_MESSAGE_TOOL is \
                 set (the gateway sets it to 'true' by default)",
                if names.is_empty() {
                    "<none listed>".to_string()
                } else {
                    names.join(", ")
                }
            )))
        }
    }
}

/// `("C0123", "1700000000.000100")` out of the server's reply
/// `Successfully posted message to channel C0123 (ts=1700000000.000100)`
/// (or `… to channel C0123 in thread 1699999999.000200 (ts=…)`).
fn parse_add_message_reply(text: &str) -> Option<(String, String)> {
    let channel = text
        .split("channel ")
        .nth(1)?
        .split(|c: char| c.is_whitespace() || c == '(')
        .next()?
        .to_string();
    let ts = text
        .split("ts=")
        .nth(1)?
        .split(|c: char| !(c.is_ascii_digit() || c == '.'))
        .next()?
        .to_string();
    if channel.is_empty() || ts.is_empty() {
        return None;
    }
    Some((channel, ts))
}

/// Minimal CSV parser (the server marshals its results with gocsv, which
/// quotes fields containing commas or quotes).
fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let mut row = Vec::new();
        let mut field = String::new();
        let mut chars = line.chars().peekable();
        let mut in_quotes = false;
        while let Some(c) = chars.next() {
            if in_quotes {
                if c == '"' {
                    if chars.peek() == Some(&'"') {
                        chars.next();
                        field.push('"');
                    } else {
                        in_quotes = false;
                    }
                } else {
                    field.push(c);
                }
            } else if c == '"' {
                in_quotes = true;
            } else if c == ',' {
                row.push(std::mem::take(&mut field));
            } else {
                field.push(c);
            }
        }
        row.push(field);
        rows.push(row);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_add_message_reply_reads_channel_and_ts() {
        assert_eq!(
            parse_add_message_reply("Successfully posted message to channel C0123 (ts=1700000000.000100)"),
            Some(("C0123".to_string(), "1700000000.000100".to_string()))
        );
        assert_eq!(
            parse_add_message_reply(
                "Successfully posted message to channel D0456 in thread 1699999999.000200 (ts=1700000000.000100)"
            ),
            Some(("D0456".to_string(), "1700000000.000100".to_string()))
        );
        assert_eq!(parse_add_message_reply("nope"), None);
        assert_eq!(parse_add_message_reply("channel  (ts=)"), None);
    }

    #[test]
    fn parse_csv_handles_plain_and_quoted_fields() {
        let rows = parse_csv("ID,Name,Topic\nC1,eng,\"team, chat\"\nC2,\"say \"\"hi\"\"\",x");
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0], vec!["ID", "Name", "Topic"]);
        assert_eq!(rows[1], vec!["C1", "eng", "team, chat"]);
        assert_eq!(rows[2], vec!["C2", "say \"hi\"", "x"]);
        assert!(parse_csv("\n  \n").is_empty());
    }

    #[test]
    fn split_command_splits_program_and_args() {
        assert_eq!(
            split_command("slack-mcp-server --transport stdio").unwrap(),
            (
                "slack-mcp-server".to_string(),
                vec!["--transport".to_string(), "stdio".to_string()]
            )
        );
        assert_eq!(
            split_command("/usr/local/bin/slack-mcp-server").unwrap(),
            ("/usr/local/bin/slack-mcp-server".to_string(), vec![])
        );
        assert!(split_command("   ").is_err());
    }

    /// A fresh scratch directory for the command-resolution tests.
    fn scratch_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "slack-approval-gateway-test-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn resolve_command_prefers_env_then_file_then_path() {
        let dir = scratch_dir("resolve-command");
        let bin_dir = dir.join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let server = bin_dir.join(PATH_SERVER_PROGRAM);
        std::fs::write(&server, "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&server, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let server_path = std::ffi::OsString::from(bin_dir.as_os_str());
        let empty_dir = dir.join("empty");
        std::fs::create_dir_all(&empty_dir).unwrap();
        let empty_path = std::ffi::OsString::from(empty_dir.as_os_str());
        let file = dir.join("slack_stdio_command");

        // Nothing resolves: the not-installed error names the file, the env
        // var and the repo.
        let err = resolve_command_from(&dir, None, Some(&empty_path)).unwrap_err();
        assert!(err.0.contains("not installed"), "{}", err.0);
        assert!(err.0.contains("slack_stdio_command"), "{}", err.0);
        assert!(err.0.contains("SLACK_STDIO_COMMAND"), "{}", err.0);
        assert!(
            err.0.contains("https://github.com/korotovsky/slack-mcp-server"),
            "{}",
            err.0
        );

        // A non-executable file on PATH does not count either.
        let noexec_dir = dir.join("noexec");
        std::fs::create_dir_all(&noexec_dir).unwrap();
        std::fs::write(noexec_dir.join(PATH_SERVER_PROGRAM), "not executable\n").unwrap();
        let noexec_path = std::ffi::OsString::from(noexec_dir.as_os_str());
        let err = resolve_command_from(&dir, None, Some(&noexec_path)).unwrap_err();
        assert!(err.0.contains("not installed"), "{}", err.0);

        // PATH: the server is run with --transport stdio.
        assert_eq!(
            resolve_command_from(&dir, None, Some(&server_path)).unwrap(),
            format!("{} --transport stdio", server.display())
        );

        // The file beats PATH.
        std::fs::write(&file, "  /opt/slack/server --transport stdio \n").unwrap();
        assert_eq!(
            resolve_command_from(&dir, None, Some(&server_path)).unwrap(),
            "/opt/slack/server --transport stdio"
        );

        // The env var beats the file (a blank one falls through to the file).
        assert_eq!(
            resolve_command_from(&dir, Some("/env/server --flag"), Some(&server_path)).unwrap(),
            "/env/server --flag"
        );
        assert_eq!(
            resolve_command_from(&dir, Some("   "), Some(&server_path)).unwrap(),
            "/opt/slack/server --transport stdio"
        );

        // An empty command file is an error, not a silent fallback.
        std::fs::write(&file, "\n").unwrap();
        let err = resolve_command_from(&dir, None, Some(&server_path)).unwrap_err();
        assert!(err.0.contains("is empty"), "{}", err.0);
    }

    #[test]
    fn validate_content_type_accepts_only_the_two_values() {
        assert_eq!(validate_content_type("text/plain").unwrap(), "text/plain");
        assert_eq!(validate_content_type(" text/markdown ").unwrap(), "text/markdown");
        assert!(validate_content_type("text/html").is_err());
    }

    #[test]
    fn resolve_korotovsky_send_tool_prefers_the_exact_name_and_detects_args() {
        let tools = serde_json::json!([
            { "name": "channels_list" },
            { "name": "conversations_add_message", "inputSchema": { "type": "object", "properties": {
                "channel_id": { "type": "string" },
                "thread_ts": { "type": "string" },
                "text": { "type": "string" },
                "content_type": { "type": "string" },
                "blocks": { "type": "string" }
            } } },
            { "name": "slack_send_message", "inputSchema": { "type": "object", "properties": {
                "channel_id": {}, "message": {}
            } } }
        ])
        .as_array()
        .unwrap()
        .clone();
        let spec = resolve_korotovsky_send_tool(&tools, None, None, None).unwrap();
        assert_eq!(spec.name, "conversations_add_message");
        assert_eq!(spec.channel_arg, "channel_id");
        assert_eq!(spec.text_arg, "text");
        assert_eq!(spec.thread_arg.as_deref(), Some("thread_ts"));

        // Overrides win over the schema.
        let spec = resolve_korotovsky_send_tool(
            &tools,
            Some("channel".to_string()),
            Some("payload".to_string()),
            Some("parent_ts".to_string()),
        )
        .unwrap();
        assert_eq!(spec.channel_arg, "channel");
        assert_eq!(spec.text_arg, "payload");
        assert_eq!(spec.thread_arg.as_deref(), Some("parent_ts"));
    }

    #[test]
    fn resolve_korotovsky_send_tool_falls_back_to_a_generic_send_tool() {
        let tools = serde_json::json!([
            { "name": "channels_list" },
            { "name": "slack_send_message", "inputSchema": { "type": "object", "properties": {
                "channel_id": {}, "message": {}
            } } }
        ])
        .as_array()
        .unwrap()
        .clone();
        let spec = resolve_korotovsky_send_tool(&tools, None, None, None).unwrap();
        assert_eq!(spec.name, "slack_send_message");
        assert_eq!(spec.text_arg, "message");
    }

    #[test]
    fn resolve_korotovsky_send_tool_errors_with_a_hint_when_missing() {
        let tools = serde_json::json!([{ "name": "channels_list" }])
            .as_array()
            .unwrap()
            .clone();
        let err = resolve_korotovsky_send_tool(&tools, None, None, None).unwrap_err();
        assert!(err.0.contains("no send-message tool"), "{}", err.0);
        assert!(err.0.contains("SLACK_MCP_ADD_MESSAGE_TOOL"), "{}", err.0);
        assert!(err.0.contains("channels_list"), "{}", err.0);
    }
}
