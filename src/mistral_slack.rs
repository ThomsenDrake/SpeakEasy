//! Slack sending via the Mistral Slack Connector.
//!
//! Alternative [`SlackClient`] transport: instead of calling a Slack MCP
//! server, the gateway calls the Slack tool of a Mistral platform connector
//! directly — `POST /v1/connectors/{connector}/tools/{tool}/call` — so the
//! Slack credentials live in the Mistral console (the connected Slack
//! workspace) and this process only holds a Mistral API key. The
//! agent-facing contract is identical: send the exact text to the exact
//! destination, return the permalink.
//!
//! Tool discovery, argument-name detection and reply parsing are shared with
//! the Slack MCP transport in [`crate::slack_tools`].

use crate::core::{SlackClient, SlackError, SlackSend};
use crate::slack::{fallback_permalink, non_empty_env, read_secret_file};
use crate::slack_tools::{
    field_line, json_text, resolve_send_tool, send_arguments, send_outcome, tool_result_error,
    SendToolSpec,
};
use serde_json::{Map, Value};
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

/// Mistral platform API base URL used when `MISTRAL_API_BASE` is not set.
const DEFAULT_API_BASE: &str = "https://api.mistral.ai";

/// Upper bound on each Mistral API call. Connector tool calls go through an
/// OAuth-backed MCP server, so they are slower than a direct Slack call.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Default connector name (the Slack connector in the Mistral catalog).
const DEFAULT_CONNECTOR: &str = "slack";

/// A [`SlackClient`] that sends through the Mistral Slack Connector.
///
/// The send tool is either pinned with `MISTRAL_SLACK_TOOL` or discovered once
/// from the connector's tool list, with argument names detected from the
/// tool's declared `inputSchema` (e.g. the `slack` connector's
/// `slack_send_message` takes `channel_id`/`message`/`thread_ts`).
/// `MISTRAL_SLACK_CHANNEL_ARG`, `MISTRAL_SLACK_TEXT_ARG` and
/// `MISTRAL_SLACK_THREAD_ARG` override the detected names.
pub struct MistralConnectorSlackClient {
    api_key: String,
    api_base: String,
    connector: String,
    tool: Option<String>,
    /// Explicit argument-name overrides (`None` = detect from the schema).
    channel_arg: Option<String>,
    text_arg: Option<String>,
    thread_arg: Option<String>,
    /// Discovered send tool, cached for the life of the client.
    discovered: Mutex<Option<SendToolSpec>>,
    agent: ureq::Agent,
}

impl MistralConnectorSlackClient {
    /// Build from the environment: `MISTRAL_API_KEY` (required) or a key file
    /// at `<state_dir>/mistral_api_key` (one line, trimmed; create it with
    /// `chmod 600`). `MISTRAL_API_BASE` overrides the API base URL (default
    /// `https://api.mistral.ai`) and exists for tests.
    /// `MISTRAL_SLACK_CONNECTOR` selects the connector (default `slack`).
    pub fn from_env(state_dir: &Path) -> Result<Self, SlackError> {
        let api_key = match non_empty_env("MISTRAL_API_KEY") {
            Some(key) => key,
            None => read_secret_file(
                &state_dir.join("mistral_api_key"),
                "MISTRAL_API_KEY",
                "Mistral API key",
            )?,
        };
        if api_key.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(SlackError(
                "the Mistral API key must be a single token with no spaces or line breaks"
                    .to_string(),
            ));
        }
        let api_base =
            non_empty_env("MISTRAL_API_BASE").unwrap_or_else(|| DEFAULT_API_BASE.to_string());
        let connector =
            non_empty_env("MISTRAL_SLACK_CONNECTOR").unwrap_or_else(|| DEFAULT_CONNECTOR.to_string());
        let tool = non_empty_env("MISTRAL_SLACK_TOOL");
        let channel_arg = non_empty_env("MISTRAL_SLACK_CHANNEL_ARG");
        let text_arg = non_empty_env("MISTRAL_SLACK_TEXT_ARG");
        let thread_arg = non_empty_env("MISTRAL_SLACK_THREAD_ARG");
        let mut client = Self::new(api_key, api_base)
            .with_connector(&connector)
            .with_tool(tool);
        if channel_arg.is_some() || text_arg.is_some() || thread_arg.is_some() {
            client = client.with_arguments(
                channel_arg.as_deref(),
                text_arg.as_deref(),
                thread_arg.as_deref(),
            );
        }
        Ok(client)
    }

    /// Build explicitly (used by tests with a mock API base).
    pub fn new(api_key: impl Into<String>, api_base: impl Into<String>) -> Self {
        let api_base = api_base.into().trim().trim_end_matches('/').to_string();
        // Statuses are inspected by hand so the API's error body can be
        // reported, and redirects are never followed: the Authorization
        // header must not leak to another host.
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(REQUEST_TIMEOUT))
            .http_status_as_error(false)
            .max_redirects(0)
            .build()
            .new_agent();
        Self {
            api_key: api_key.into(),
            api_base,
            connector: DEFAULT_CONNECTOR.to_string(),
            tool: None,
            channel_arg: None,
            text_arg: None,
            thread_arg: None,
            discovered: Mutex::new(None),
            agent,
        }
    }

    /// Select the connector by id or name (default `slack`).
    pub fn with_connector(mut self, connector: &str) -> Self {
        let connector = connector.trim();
        if !connector.is_empty() {
            self.connector = connector.to_string();
        }
        self
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

    /// Override the tool argument names the connector expects (`None` leaves
    /// a name to be detected from the tool's input schema).
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

    /// `GET {api_base}{path}` with the API key as a bearer token; return the
    /// parsed JSON reply.
    fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<Value, SlackError> {
        let url = format!("{}{}", self.api_base, path);
        let mut request = self
            .agent
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key));
        for (key, value) in query {
            request = request.query(*key, *value);
        }
        let response = request.call().map_err(|e| request_error("GET", path, e))?;
        read_reply(response, "GET", path)
    }

    /// `POST {api_base}{path}` with a JSON body and the API key as a bearer
    /// token; return the parsed JSON reply.
    fn post(&self, path: &str, body: &Value) -> Result<Value, SlackError> {
        let url = format!("{}{}", self.api_base, path);
        let response = self
            .agent
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send_json(body)
            .map_err(|e| request_error("POST", path, e))?;
        read_reply(response, "POST", path)
    }

    /// Call any connector tool with the given arguments and return the
    /// parsed reply (the caller decides how to treat `isError` metadata).
    fn call_tool(&self, tool: &str, arguments: Map<String, Value>) -> Result<Value, SlackError> {
        self.post(
            &format!("/v1/connectors/{}/tools/{}/call", self.connector, tool),
            &Value::Object(crate::slack_tools::json_arguments(arguments)),
        )
    }

    /// The connector's tool list (`GET /v1/connectors/{connector}/tools`).
    fn list_tools(&self) -> Result<Vec<Value>, SlackError> {
        let reply = self.get(
            &format!("/v1/connectors/{}/tools", self.connector),
            &[("page_size", "100")],
        )?;
        // The API returns a bare array of tools; accept a wrapped list too.
        if let Some(items) = reply.as_array() {
            return Ok(items.clone());
        }
        for key in ["data", "items"] {
            if let Some(items) = reply.get(key).and_then(Value::as_array) {
                return Ok(items.clone());
            }
        }
        Err(SlackError(format!(
            "Mistral API: unexpected tool list reply for connector '{}'",
            self.connector
        )))
    }

    /// The connector's send-message tool: pinned, or discovered once from the
    /// tool list, with argument names resolved from its `inputSchema`.
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
        let lock = || self.discovered.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(cached) = lock().clone() {
            return Ok(cached);
        }
        let tools = self.list_tools()?;
        let resolved = resolve_send_tool(
            &tools,
            self.channel_arg.clone(),
            self.text_arg.clone(),
            self.thread_arg.clone(),
            &format!(
                "set MISTRAL_SLACK_TOOL to the exact tool name (connector '{}')",
                self.connector
            ),
        )?;
        *lock() = Some(resolved.clone());
        Ok(resolved)
    }

    /// `Drake Thomsen (@drake.thomsen)` out of a `slack_read_user_profile`
    /// reply (`{"result": "…\nReal Name: …\nUsername: …"}`).
    fn resolve_user(&self, user_id: &str) -> Option<String> {
        let mut arguments = Map::new();
        arguments.insert("user_id".to_string(), Value::String(user_id.to_string()));
        let reply = self.call_tool("slack_read_user_profile", arguments).ok()?;
        let result = json_text(&reply)
            .and_then(|v| v.get("result").and_then(Value::as_str).map(str::to_string))?;
        let real_name = field_line(&result, "Real Name");
        let display_name = field_line(&result, "Display Name");
        let username = field_line(&result, "Username");
        let name = real_name.or(display_name).or_else(|| username.clone())?;
        let handle = username.filter(|u| *u != name);
        Some(match handle {
            Some(u) => format!("{name} (@{u})"),
            None => name,
        })
    }

    /// `#name` out of a `slack_read_channel` reply, whose first line is
    /// `Channel: #name (C…)` (or `Channel: DM (D…)`, which is not resolved).
    fn resolve_channel(&self, channel_id: &str) -> Option<String> {
        let mut arguments = Map::new();
        arguments.insert("channel_id".to_string(), Value::String(channel_id.to_string()));
        arguments.insert("limit".to_string(), Value::from(1));
        let reply = self.call_tool("slack_read_channel", arguments).ok()?;
        let messages = json_text(&reply)
            .and_then(|v| v.get("messages").and_then(Value::as_str).map(str::to_string))?;
        let first = messages.lines().next()?;
        let rest = first.strip_prefix("Channel:")?.trim();
        if rest.starts_with('#') {
            return Some(rest.split_whitespace().next()?.to_string());
        }
        None
    }
}

impl std::fmt::Debug for MistralConnectorSlackClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MistralConnectorSlackClient")
            .field("api_key", &"<redacted>")
            .field("api_base", &self.api_base)
            .field("connector", &self.connector)
            .field("tool", &self.tool)
            .finish()
    }
}

impl SlackClient for MistralConnectorSlackClient {
    /// Call the connector's send tool with the exact `text` to the exact
    /// `destination` (optionally as a reply in thread `thread_ts`) and report
    /// the permalink the connector returns.
    fn post_message(
        &self,
        destination: &str,
        thread_ts: Option<&str>,
        text: &str,
    ) -> Result<SlackSend, SlackError> {
        let tool = self.send_tool()?;
        let arguments = send_arguments(&tool, destination, thread_ts, text)?;
        let reply = self.call_tool(&tool.name, arguments)?;
        // The reply is an MCP CallToolResult: `content` blocks plus
        // `isError` (wrapped in `metadata.mcp_meta` by the platform).
        if let Some(error) = tool_result_error(&reply) {
            return Err(SlackError(format!(
                "connector tool {} failed: {error}",
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

    /// Best-effort display name for the approval popup: `U…` user IDs via
    /// `slack_read_user_profile`, `C…`/`G…` channel IDs via
    /// `slack_read_channel` (its reply opens with `Channel: #name (ID)`).
    /// `#name`/`@name` destinations and `D…` DM IDs return `None` (the
    /// connector has no DM-partner lookup). Any failure returns `None`.
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

/// Check the status of an accepted request and parse its JSON reply.
/// Non-2xx statuses become `SlackError` with the API's `detail` text; the
/// API key never appears in error text.
fn read_reply(
    mut response: ureq::http::Response<ureq::Body>,
    method: &str,
    path: &str,
) -> Result<Value, SlackError> {
    let status = response.status();
    if !status.is_success() {
        // The platform explains failures in a JSON body (`detail`).
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
            "Mistral API {method} {path}: HTTP {}{suffix}",
            status.as_u16()
        )));
    }
    // The API already accepted the request, so a failure here must not
    // read as "nothing happened".
    response.body_mut().read_json::<Value>().map_err(|e| {
        SlackError(format!(
            "Mistral API {method} {path}: HTTP {} but the reply could not be read ({e}); the request may have taken effect",
            status.as_u16()
        ))
    })
}

fn request_error(method: &str, path: &str, err: ureq::Error) -> SlackError {
    match err {
        ureq::Error::Timeout(_) => SlackError(format!(
            "Mistral API {method} {path}: no reply within {}s; the request may still have reached the API",
            REQUEST_TIMEOUT.as_secs()
        )),
        other => SlackError(format!("Mistral API {method} {path}: request failed: {other}")),
    }
}
