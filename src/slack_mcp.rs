//! Slack sending via the official Slack MCP server (remote, HTTP).
//!
//! The default [`SlackClient`] transport: the gateway speaks MCP (Streamable
//! HTTP: JSON-RPC POSTs, JSON or SSE responses) to a remote Slack MCP server
//! — by default the official `https://mcp.slack.com/mcp` — and calls its
//! tools. The gateway holds a Slack **user OAuth token** (not a bot token);
//! it is obtained once via the app's OAuth flow (see
//! `scripts/slack_mcp_oauth.py`) and stored in `SLACK_MCP_TOKEN` or
//! `<state_dir>/slack_mcp_token` (chmod 600). The agent-facing contract is
//! identical: send the exact text to the exact destination, return the
//! permalink.
//!
//! Tool discovery, argument-name detection and reply parsing are shared with
//! the Mistral connector transport in [`crate::slack_tools`].

use crate::core::{SlackClient, SlackError, SlackSend};
use crate::slack::{fallback_permalink, non_empty_env, read_secret_file};
use crate::slack_tools::{
    field_line, json_text, pick_argument, resolve_send_tool, schema_properties, send_arguments,
    send_outcome, string_field, text_content, tool_result_error, SendToolSpec,
};
use serde_json::{json, Map, Value};
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

/// The official Slack MCP server endpoint used when `SLACK_MCP_URL` is not set.
const DEFAULT_MCP_URL: &str = "https://mcp.slack.com/mcp";

/// MCP protocol version offered during `initialize`.
const PROTOCOL_VERSION: &str = "2025-06-18";

/// Upper bound on each MCP call (initialize, tools/list, tools/call).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Candidate argument names for user-lookup tools.
const USER_ARG_CANDIDATES: &[&str] = &["user_id", "user", "id"];

/// Candidate argument names for channel-lookup tools.
const CHANNEL_ARG_CANDIDATES: &[&str] = &["channel_id", "channel", "id"];

/// The official Slack MCP server's user-lookup tool (needs `users:read`).
const USER_LOOKUP_TOOL: &str = "slack_read_user_profile";

/// The official Slack MCP server's channel-lookup tool. It lists the
/// user's channels with their IDs and names, which is the cheapest way to
/// map a channel ID to its name: `channels:read`/`groups:read`/`im:read`/
/// `mpim:read`, versus the `*:history` scopes `slack_read_channel` needs
/// or the `search:read.*` scopes `slack_search_channels` needs (and search
/// cannot look a channel up by ID).
const CHANNEL_LOOKUP_TOOL: &str = "slack_list_user_channels";

/// A [`SlackClient`] that sends through a remote Slack MCP server.
///
/// The send tool is either pinned with `SLACK_MCP_TOOL` or discovered once
/// from the server's `tools/list`, with argument names detected from the
/// tool's declared `inputSchema` (the official server's `slack_send_message`
/// takes `channel_id`/`message`/`thread_ts`).
/// `SLACK_MCP_CHANNEL_ARG`, `SLACK_MCP_TEXT_ARG` and `SLACK_MCP_THREAD_ARG`
/// override the detected names.
pub struct SlackMcpClient {
    token: String,
    url: String,
    tool: Option<String>,
    /// Explicit argument-name overrides (`None` = detect from the schema).
    channel_arg: Option<String>,
    text_arg: Option<String>,
    thread_arg: Option<String>,
    /// Session state (MCP session id, negotiated protocol version, cached
    /// tool list and send tool) plus the RPC id counter. One mutex: MCP
    /// requests are serialized, which also keeps id matching trivial.
    session: Mutex<McpSession>,
    agent: ureq::Agent,
}

/// Per-session MCP state.
struct McpSession {
    /// `Mcp-Session-Id` returned by `initialize`, sent on later requests.
    session_id: Option<String>,
    /// Protocol version negotiated during `initialize`.
    protocol_version: String,
    /// Whether `initialize` + `notifications/initialized` completed.
    initialized: bool,
    /// Next JSON-RPC request id.
    next_id: u64,
    /// Cached `tools/list` result.
    tools: Option<Vec<Value>>,
    /// Cached send-tool resolution.
    send_tool: Option<SendToolSpec>,
}

impl McpSession {
    fn fresh() -> Self {
        Self {
            session_id: None,
            protocol_version: PROTOCOL_VERSION.to_string(),
            initialized: false,
            next_id: 0,
            tools: None,
            send_tool: None,
        }
    }
}

impl SlackMcpClient {
    /// Build from the environment: `SLACK_MCP_TOKEN` (required) or a token
    /// file at `<state_dir>/slack_mcp_token` (one line, trimmed; create it
    /// with `chmod 600`). `SLACK_MCP_URL` overrides the endpoint (default
    /// `https://mcp.slack.com/mcp`) and exists for tests.
    /// `SLACK_MCP_TOOL` pins the send tool. The MCP handshake runs here so a
    /// bad token or endpoint fails at startup, not after an approval.
    pub fn from_env(state_dir: &Path) -> Result<Self, SlackError> {
        let token = match non_empty_env("SLACK_MCP_TOKEN") {
            Some(token) => token,
            None => read_secret_file(
                &state_dir.join("slack_mcp_token"),
                "SLACK_MCP_TOKEN",
                "Slack MCP token",
            )?,
        };
        let url = non_empty_env("SLACK_MCP_URL").unwrap_or_else(|| DEFAULT_MCP_URL.to_string());
        let tool = non_empty_env("SLACK_MCP_TOOL");
        let channel_arg = non_empty_env("SLACK_MCP_CHANNEL_ARG");
        let text_arg = non_empty_env("SLACK_MCP_TEXT_ARG");
        let thread_arg = non_empty_env("SLACK_MCP_THREAD_ARG");
        let mut client = Self::new(token, url)?;
        client.tool = tool;
        if channel_arg.is_some() || text_arg.is_some() || thread_arg.is_some() {
            client = client.with_arguments(
                channel_arg.as_deref(),
                text_arg.as_deref(),
                thread_arg.as_deref(),
            );
        }
        Ok(client)
    }

    /// Build explicitly (used by tests with a mock MCP endpoint) and run the
    /// MCP handshake: `initialize`, then `notifications/initialized`.
    pub fn new(token: impl Into<String>, url: impl Into<String>) -> Result<Self, SlackError> {
        let token = token.into();
        if token.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(SlackError(
                "the Slack MCP token must be a single token with no spaces or line breaks"
                    .to_string(),
            ));
        }
        let url = url.into().trim().trim_end_matches('/').to_string();
        // Statuses are inspected by hand so error bodies can be reported, and
        // redirects are never followed: the bearer token must not leak to
        // another host.
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(REQUEST_TIMEOUT))
            .http_status_as_error(false)
            .max_redirects(0)
            .build()
            .new_agent();
        let mut session = McpSession::fresh();
        let init = rpc_once(
            &agent,
            &token,
            &url,
            &mut session,
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "slack-approval-gateway", "version": env!("CARGO_PKG_VERSION") }
            }),
        )?;
        if let Some(version) = init.get("protocolVersion").and_then(Value::as_str) {
            session.protocol_version = version.to_string();
        }
        session.initialized = true;
        // `notifications/initialized` completes the handshake; the server
        // answers 202 Accepted with no body.
        notify(
            &agent,
            &token,
            &url,
            &mut session,
            "notifications/initialized",
            json!({}),
        )?;
        Ok(Self {
            token,
            url,
            tool: None,
            channel_arg: None,
            text_arg: None,
            thread_arg: None,
            session: Mutex::new(session),
            agent,
        })
    }

    /// Pin the send tool name (skips discovery).
    pub fn with_tool(mut self, tool: Option<String>) -> Self {
        self.tool = tool.and_then(|t| {
            let t = t.trim().to_string();
            if t.is_empty() {
                None
            } else {
                Some(t)
            }
        });
        self
    }

    /// Override the tool argument names the server expects (`None` leaves a
    /// name to be detected from the tool's input schema).
    pub fn with_arguments(
        mut self,
        channel: Option<&str>,
        text: Option<&str>,
        thread: Option<&str>,
    ) -> Self {
        self.channel_arg = channel.map(str::to_string);
        self.text_arg = text.map(str::to_string);
        self.thread_arg = thread.map(str::to_string);
        self
    }

    /// Run `f` with the session locked (serializes all MCP traffic).
    fn with_session<T>(
        &self,
        f: impl FnOnce(&mut McpSession) -> Result<T, SlackError>,
    ) -> Result<T, SlackError> {
        let mut session = self.session.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut session)
    }

    /// The server's tool list (cached for the life of the client).
    fn tools(&self) -> Result<Vec<Value>, SlackError> {
        self.with_session(|s| {
            if s.tools.is_none() {
                let result = rpc_once(&self.agent, &self.token, &self.url, s, "tools/list", json!({}))?;
                let tools = result
                    .get("tools")
                    .and_then(Value::as_array)
                    .cloned()
                    .ok_or_else(|| {
                        SlackError("Slack MCP: unexpected tools/list reply".to_string())
                    })?;
                s.tools = Some(tools);
            }
            Ok(s.tools.clone().unwrap_or_default())
        })
    }

    /// The send-message tool: pinned, or discovered once from the tool list,
    /// with argument names resolved from its `inputSchema`.
    fn send_tool(&self) -> Result<SendToolSpec, SlackError> {
        // A pinned tool skips discovery; without a schema the legacy
        // channel/text/thread_ts names are used (overridable).
        if let Some(tool) = &self.tool {
            return Ok(SendToolSpec {
                name: tool.clone(),
                channel_arg: self
                    .channel_arg
                    .clone()
                    .unwrap_or_else(|| "channel".to_string()),
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
            if s.tools.is_none() {
                let result =
                    rpc_once(&self.agent, &self.token, &self.url, s, "tools/list", json!({}))?;
                s.tools = Some(
                    result
                        .get("tools")
                        .and_then(Value::as_array)
                        .cloned()
                        .ok_or_else(|| {
                            SlackError("Slack MCP: unexpected tools/list reply".to_string())
                        })?,
                );
            }
            let tools = s.tools.clone().unwrap_or_default();
            let spec = resolve_send_tool(
                &tools,
                self.channel_arg.clone(),
                self.text_arg.clone(),
                self.thread_arg.clone(),
                "set SLACK_MCP_TOOL to the exact tool name",
            )?;
            s.send_tool = Some(spec.clone());
            Ok(spec)
        })
    }

    /// Call a tool (`tools/call`) and return the CallToolResult.
    fn call_tool(&self, tool: &str, arguments: Map<String, Value>) -> Result<Value, SlackError> {
        self.with_session(|s| {
            rpc_once(
                &self.agent,
                &self.token,
                &self.url,
                s,
                "tools/call",
                json!({ "name": tool, "arguments": Value::Object(arguments) }),
            )
        })
    }

    /// The lookup tool to call: the exact official tool name when the
    /// server lists it, else the first tool whose name loosely matches
    /// `predicate` (a fallback for servers that name their tools
    /// differently). The loose match alone can pick a wrong tool — e.g.
    /// `slack_search_users` (which needs `search:read.users` and takes a
    /// `query`, not a `user_id`) or `slack_read_channel` (which needs the
    /// `*:history` scopes) — so the exact name always wins.
    fn find_lookup_tool(&self, preferred: &str, predicate: impl Fn(&str) -> bool) -> Option<Value> {
        let tools = self.tools().ok()?;
        tools
            .iter()
            .find(|t| t.get("name").and_then(Value::as_str) == Some(preferred))
            .or_else(|| {
                tools.iter().find(|t| {
                    t.get("name")
                        .and_then(Value::as_str)
                        .is_some_and(|n| predicate(n))
                })
            })
            .cloned()
    }

    /// `Drake Thomsen (@drake.thomsen)` for a user ID, via the user-lookup
    /// tool (`slack_read_user_profile`): the reply's JSON object carries
    /// the name fields, or the text carries `Real Name:`/`Username:` lines.
    fn resolve_user(&self, user_id: &str) -> Option<String> {
        let tool = self.find_lookup_tool(USER_LOOKUP_TOOL, |n| {
            let n = n.to_ascii_lowercase();
            n.contains("user")
                && (n.contains("info")
                    || n.contains("profile")
                    || n.contains("list")
                    || n.contains("search"))
        })?;
        let name = tool.get("name").and_then(Value::as_str)?;
        let arg = pick_argument(
            schema_properties(&tool).as_ref(),
            USER_ARG_CANDIDATES,
            "user_id",
        );
        let mut arguments = Map::new();
        arguments.insert(arg, Value::String(user_id.to_string()));
        let reply = self.call_tool(name, arguments).ok()?;
        let (json, text) = unwrap_reply(&reply);
        if let Some(obj) = json.as_ref().and_then(Value::as_object) {
            let name = string_field(obj, &["real_name", "display_name", "name"])
                .or_else(|| {
                    obj.get("user")
                        .and_then(Value::as_object)
                        .and_then(|u| string_field(u, &["real_name", "display_name", "name"]))
                })
                .or_else(|| {
                    obj.get("profile")
                        .and_then(Value::as_object)
                        .and_then(|p| string_field(p, &["real_name", "display_name", "name"]))
                });
            let handle = string_field(obj, &["name"])
                .or_else(|| {
                    obj.get("user")
                        .and_then(Value::as_object)
                        .and_then(|u| string_field(u, &["name"]))
                })
                .or_else(|| {
                    obj.get("profile")
                        .and_then(Value::as_object)
                        .and_then(|p| string_field(p, &["display_name"]))
                });
            if let Some(name) = name {
                return Some(format_name(name, handle));
            }
        }
        let text = text.or_else(|| text_content(&reply))?;
        let real = field_line(&text, "Real Name");
        let display = field_line(&text, "Display Name");
        let username = field_line(&text, "Username");
        let name = real.or(display).or_else(|| username.clone())?;
        Some(format_name(name, username))
    }

    /// `#name` for a channel ID, via the channel-lookup tool
    /// (`slack_list_user_channels`): the reply's JSON has a channels array
    /// (or a channel object) with the name, or the text opens with
    /// `Channel: #name (ID)`.
    fn resolve_channel(&self, channel_id: &str) -> Option<String> {
        let tool = self.find_lookup_tool(CHANNEL_LOOKUP_TOOL, |n| {
            let n = n.to_ascii_lowercase();
            (n.contains("channel") || n.contains("conversation"))
                && (n.contains("info")
                    || n.contains("list")
                    || n.contains("history")
                    || n.contains("read"))
        })?;
        let name = tool.get("name").and_then(Value::as_str)?;
        let arguments = if name == CHANNEL_LOOKUP_TOOL {
            // The official list tool takes no channel ID: ask for a full
            // page of the user's channels and match the ID in the reply.
            let mut arguments = Map::new();
            arguments.insert("limit".to_string(), json!(200));
            arguments
        } else {
            let arg = pick_argument(
                schema_properties(&tool).as_ref(),
                CHANNEL_ARG_CANDIDATES,
                "channel_id",
            );
            let mut arguments = Map::new();
            arguments.insert(arg, Value::String(channel_id.to_string()));
            arguments
        };
        let reply = self.call_tool(name, arguments).ok()?;
        let (json, text) = unwrap_reply(&reply);
        if let Some(value) = &json {
            // A channels array — under "channels", or the whole payload —
            // with the ID matched against the requested one.
            let empty: &[Value] = &[];
            let channels = value
                .as_object()
                .and_then(|o| o.get("channels"))
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .or_else(|| value.as_array().map(Vec::as_slice))
                .unwrap_or(empty);
            for channel in channels {
                if channel.get("id").and_then(Value::as_str) == Some(channel_id) {
                    if let Some(name) = channel
                        .get("name")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                    {
                        return Some(format!("#{name}"));
                    }
                }
            }
            if let Some(channel) = value
                .as_object()
                .and_then(|o| o.get("channel"))
                .and_then(Value::as_object)
            {
                if let Some(name) = channel
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                {
                    return Some(format!("#{name}"));
                }
            }
        }
        let text = text.or_else(|| text_content(&reply))?;
        let first = text.lines().next()?;
        let rest = first.strip_prefix("Channel:")?.trim();
        if rest.starts_with('#') {
            return Some(rest.split_whitespace().next()?.to_string());
        }
        None
    }
}

/// A lookup-tool reply, unwrapped: the JSON value the tool reported — the
/// first text block parsed as JSON, or the inner JSON of Slack's
/// `{"result": "…"}` / `{"messages": "…"}` string wrapping (the Mistral
/// connector proxies this same server) — plus the plain text when the
/// wrapped string is not JSON (formatted `Key: value` lines).
fn unwrap_reply(reply: &Value) -> (Option<Value>, Option<String>) {
    let Some(parsed) = json_text(reply) else {
        return (None, text_content(reply));
    };
    if let Some(inner) = parsed
        .get("result")
        .or_else(|| parsed.get("messages"))
        .and_then(Value::as_str)
    {
        return match serde_json::from_str::<Value>(inner) {
            Ok(value) if value.is_object() || value.is_array() => (Some(value), None),
            _ => (None, Some(inner.to_string())),
        };
    }
    if parsed.is_object() || parsed.is_array() {
        (Some(parsed), None)
    } else if let Some(text) = parsed.as_str() {
        (None, Some(text.to_string()))
    } else {
        (None, text_content(reply))
    }
}

/// `Name (@handle)` when a handle differs from the name, else just `Name`.
fn format_name(name: String, handle: Option<String>) -> String {
    match handle {
        Some(h) if h != name => format!("{name} (@{h})"),
        _ => name,
    }
}

impl std::fmt::Debug for SlackMcpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SlackMcpClient")
            .field("token", &"<redacted>")
            .field("url", &self.url)
            .field("tool", &self.tool)
            .finish()
    }
}

impl SlackClient for SlackMcpClient {
    /// Call the MCP server's send tool with the exact `text` to the exact
    /// `destination` (optionally as a reply in thread `thread_ts`) and report
    /// the permalink the server returns.
    fn post_message(
        &self,
        destination: &str,
        thread_ts: Option<&str>,
        text: &str,
    ) -> Result<SlackSend, SlackError> {
        let tool = self.send_tool()?;
        let arguments = send_arguments(&tool, destination, thread_ts, text)?;
        let reply = self.call_tool(&tool.name, arguments)?;
        // The reply is an MCP CallToolResult: `content` blocks plus `isError`.
        if let Some(error) = tool_result_error(&reply) {
            return Err(SlackError(format!(
                "Slack MCP tool {} failed: {error}",
                tool.name
            )));
        }
        let outcome = send_outcome(&reply);
        let permalink = outcome
            .permalink
            .or_else(|| outcome.ts.as_deref().map(|ts| fallback_permalink(destination, ts)))
            .unwrap_or_default();
        Ok(SlackSend {
            channel: outcome.channel.unwrap_or_else(|| destination.to_string()),
            ts: outcome.ts.unwrap_or_default(),
            permalink,
        })
    }

    /// Best-effort display name for the approval popup: `U…` user IDs via a
    /// user-lookup tool, `C…`/`G…` channel IDs via a channel-lookup tool.
    /// `#name`/`@name` destinations and `D…` DM IDs return `None` (no
    /// DM-partner lookup). Any failure returns `None`.
    fn resolve_destination(&self, destination: &str) -> Option<String> {
        let d = destination.trim();
        if d.len() < 2 || d.starts_with('#') || d.starts_with('@') {
            return None;
        }
        let (kind, _) = d.split_at(1);
        match kind {
            "U" => self.resolve_user(d),
            "C" | "G" => self.resolve_channel(d),
            _ => None,
        }
    }
}

/// One JSON-RPC request/response round trip (Streamable HTTP: a single
/// message per POST; the reply is JSON or an SSE stream of `data:` lines).
fn rpc_once(
    agent: &ureq::Agent,
    token: &str,
    url: &str,
    session: &mut McpSession,
    method: &str,
    params: Value,
) -> Result<Value, SlackError> {
    session.next_id += 1;
    let id = session.next_id;
    let body = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
    let response = post(agent, token, url, session, &body)
        .map_err(|e| request_error(method, e))?;
    let message = read_rpc_response(response, method, id, session)?;
    if let Some(error) = message.get("error") {
        let detail = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown error");
        return Err(SlackError(format!("Slack MCP {method}: {detail}")));
    }
    Ok(message.get("result").cloned().unwrap_or(Value::Null))
}

/// A JSON-RPC notification (no response expected; 202 Accepted is success).
fn notify(
    agent: &ureq::Agent,
    token: &str,
    url: &str,
    session: &mut McpSession,
    method: &str,
    params: Value,
) -> Result<(), SlackError> {
    let body = json!({ "jsonrpc": "2.0", "method": method, "params": params });
    let response = post(agent, token, url, session, &body)
        .map_err(|e| request_error(method, e))?;
    if !response.status().is_success() {
        return Err(SlackError(format!(
            "Slack MCP {method}: HTTP {}",
            response.status().as_u16()
        )));
    }
    Ok(())
}

/// POST one JSON-RPC message with the MCP headers (bearer token, both
/// response content types accepted, session id and negotiated protocol
/// version when the handshake completed).
fn post(
    agent: &ureq::Agent,
    token: &str,
    url: &str,
    session: &McpSession,
    body: &Value,
) -> Result<ureq::http::Response<ureq::Body>, ureq::Error> {
    let mut request = agent
        .post(url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream");
    if session.initialized {
        request = request
            .header("MCP-Protocol-Version", &session.protocol_version);
        if let Some(session_id) = &session.session_id {
            request = request.header("Mcp-Session-Id", session_id);
        }
    }
    request.send_json(body)
}

/// Read a JSON-RPC response: JSON, or an SSE stream (find the `data:` line
/// carrying our request id). Non-2xx statuses become `SlackError` with the
/// body's `detail`/`message` text; the token never appears in error text.
fn read_rpc_response(
    mut response: ureq::http::Response<ureq::Body>,
    method: &str,
    id: u64,
    session: &mut McpSession,
) -> Result<Value, SlackError> {
    // The server hands out a session id during the handshake; keep it and
    // send it on every later request (Streamable HTTP sessions).
    if let Some(session_id) = response
        .headers()
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
    {
        session.session_id = Some(session_id);
    }
    let status = response.status();
    if !status.is_success() {
        let detail = response
            .body_mut()
            .read_json::<Value>()
            .ok()
            .and_then(|v| {
                v.get("detail")
                    .or_else(|| v.get("message"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            });
        let suffix = detail.map(|d| format!(" ({d})")).unwrap_or_default();
        return Err(SlackError(format!(
            "Slack MCP {method}: HTTP {}{suffix}",
            status.as_u16()
        )));
    }
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    if content_type.contains("text/event-stream") {
        let body = response
            .body_mut()
            .read_to_string()
            .map_err(|e| SlackError(format!("Slack MCP {method}: cannot read SSE reply: {e}")))?;
        for line in body.lines() {
            let Some(data) = line.strip_prefix("data:") else { continue };
            let data = data.trim();
            if data.is_empty() || data == "[DONE]" {
                continue;
            }
            if let Ok(message) = serde_json::from_str::<Value>(data) {
                if message.get("id").and_then(Value::as_u64) == Some(id) {
                    return Ok(message);
                }
            }
        }
        return Err(SlackError(format!(
            "Slack MCP {method}: no response with id {id} in the SSE stream"
        )));
    }
    // The request was accepted, so a failure here must not read as "nothing
    // happened".
    response.body_mut().read_json::<Value>().map_err(|e| {
        SlackError(format!(
            "Slack MCP {method}: HTTP {} but the reply could not be read ({e}); the request may have taken effect",
            status.as_u16()
        ))
    })
}

fn request_error(method: &str, err: ureq::Error) -> SlackError {
    match err {
        ureq::Error::Timeout(_) => SlackError(format!(
            "Slack MCP {method}: no reply within {}s; the request may still have reached the server",
            REQUEST_TIMEOUT.as_secs()
        )),
        other => SlackError(format!("Slack MCP {method}: request failed: {other}")),
    }
}
