//! Slack sending via the Mistral Slack Connector.
//!
//! Alternative [`SlackClient`] transport: instead of calling the Slack Web
//! API with a bot token, the gateway calls the Slack tool of a Mistral
//! platform connector directly —
//! `POST /v1/connectors/{connector}/tools/{tool}/call` — so the Slack
//! credentials live in the Mistral console (the connected Slack workspace)
//! and this process only holds a Mistral API key. The agent-facing contract
//! is identical: send the exact text to the exact destination, return the
//! permalink.

use crate::core::{SlackClient, SlackError, SlackSend};
use crate::slack::{fallback_permalink, non_empty_env, read_secret_file};
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
/// from the connector's tool list (the first tool whose name contains "send"
/// or "post" and "message"). The tool is called with `channel`/`text`
/// arguments (`thread_ts` for replies) by default;
/// `MISTRAL_SLACK_CHANNEL_ARG`, `MISTRAL_SLACK_TEXT_ARG` and
/// `MISTRAL_SLACK_THREAD_ARG` rename them.
pub struct MistralConnectorSlackClient {
    api_key: String,
    api_base: String,
    connector: String,
    tool: Option<String>,
    channel_arg: String,
    text_arg: String,
    thread_arg: String,
    /// Discovered send-tool name, cached for the life of the client.
    discovered: Mutex<Option<String>>,
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
        let channel_arg =
            non_empty_env("MISTRAL_SLACK_CHANNEL_ARG").unwrap_or_else(|| "channel".to_string());
        let text_arg = non_empty_env("MISTRAL_SLACK_TEXT_ARG").unwrap_or_else(|| "text".to_string());
        let thread_arg =
            non_empty_env("MISTRAL_SLACK_THREAD_ARG").unwrap_or_else(|| "thread_ts".to_string());
        Ok(Self::new(api_key, api_base)
            .with_connector(&connector)
            .with_tool(tool)
            .with_arguments(&channel_arg, &text_arg, &thread_arg))
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
            channel_arg: "channel".to_string(),
            text_arg: "text".to_string(),
            thread_arg: "thread_ts".to_string(),
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

    /// Rename the tool arguments the connector expects.
    pub fn with_arguments(mut self, channel: &str, text: &str, thread: &str) -> Self {
        self.channel_arg = channel.to_string();
        self.text_arg = text.to_string();
        self.thread_arg = thread.to_string();
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

    /// The name of the connector's send-message tool: pinned, or discovered
    /// once from the tool list (the first tool whose name contains "send" or
    /// "post" and "message").
    fn send_tool(&self) -> Result<String, SlackError> {
        if let Some(tool) = &self.tool {
            return Ok(tool.clone());
        }
        let lock = || self.discovered.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(cached) = lock().clone() {
            return Ok(cached);
        }
        let tools = self.list_tools()?;
        let names: Vec<&str> = tools
            .iter()
            .filter_map(|t| t.get("name").and_then(Value::as_str))
            .collect();
        let found = names
            .iter()
            .find(|name| is_send_message_tool(name))
            .map(|name| name.to_string());
        match found {
            Some(name) => {
                *lock() = Some(name.clone());
                Ok(name)
            }
            None => Err(SlackError(format!(
                "connector '{}' has no send-message tool (available: {}); set MISTRAL_SLACK_TOOL to the exact tool name",
                self.connector,
                if names.is_empty() {
                    "<none listed>".to_string()
                } else {
                    names.join(", ")
                }
            ))),
        }
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
        let mut arguments = Map::new();
        arguments.insert(
            self.channel_arg.clone(),
            Value::String(destination.to_string()),
        );
        arguments.insert(self.text_arg.clone(), Value::String(text.to_string()));
        if let Some(thread_ts) = thread_ts {
            arguments.insert(
                self.thread_arg.clone(),
                Value::String(thread_ts.to_string()),
            );
        }
        let reply = self.post(
            &format!("/v1/connectors/{}/tools/{}/call", self.connector, tool),
            &Value::Object(json_arguments(arguments)),
        )?;
        // The reply is an MCP CallToolResult: `content` blocks plus metadata
        // (`mcp_meta.isError` / `structuredContent`).
        let meta = reply.get("metadata").and_then(|m| m.get("mcp_meta"));
        let is_error = meta
            .and_then(|m| m.get("isError").or_else(|| m.get("is_error")))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if is_error {
            return Err(SlackError(format!(
                "connector tool {tool} failed: {}",
                text_content(&reply).unwrap_or_else(|| "no detail returned".to_string())
            )));
        }
        let (ts, permalink) = send_outcome(&reply);
        let permalink = permalink
            .or_else(|| ts.as_deref().map(|ts| fallback_permalink(destination, ts)))
            .unwrap_or_default();
        Ok(SlackSend {
            channel: destination.to_string(),
            ts: ts.unwrap_or_default(),
            permalink,
        })
    }
}

/// `{"arguments": …}` — the tool-call request body.
fn json_arguments(arguments: Map<String, Value>) -> Map<String, Value> {
    let mut body = Map::new();
    body.insert("arguments".to_string(), Value::Object(arguments));
    body
}

/// True when a tool name looks like "send a message" (`slack_send_message`,
/// `post_message`, …).
fn is_send_message_tool(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    (n.contains("send") || n.contains("post")) && n.contains("message")
}

/// The joined text of a tool-call reply's text content blocks.
fn text_content(reply: &Value) -> Option<String> {
    let blocks = reply.get("content")?.as_array()?;
    let texts: Vec<&str> = blocks
        .iter()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|b| b.get("text").and_then(Value::as_str))
        .collect();
    if texts.is_empty() {
        None
    } else {
        Some(texts.join("\n"))
    }
}

/// Best-effort `(ts, permalink)` from a tool-call reply: the structured
/// content first, then any Slack archives URL in the text blocks (the `ts`
/// is parsed out of the permalink when only a link is returned).
fn send_outcome(reply: &Value) -> (Option<String>, Option<String>) {
    let structured = reply
        .get("metadata")
        .and_then(|m| m.get("mcp_meta"))
        .and_then(|m| m.get("structuredContent").or_else(|| m.get("structured_content")))
        .and_then(Value::as_object);
    let mut ts = structured
        .and_then(|s| s.get("ts").or_else(|| s.get("message_ts")))
        .and_then(Value::as_str)
        .map(str::to_string);
    let mut permalink = structured
        .and_then(|s| ["permalink", "url", "link"].iter().find_map(|key| s.get(*key)))
        .and_then(Value::as_str)
        .filter(|url| url.starts_with("http"))
        .map(str::to_string);
    if permalink.is_none() {
        if let Some(text) = text_content(reply) {
            permalink = find_slack_url(&text);
        }
    }
    if ts.is_none() {
        if let Some(link) = &permalink {
            ts = ts_from_permalink(link);
        }
    }
    (ts, permalink)
}

/// The first `https://…` URL in `text` that points at a Slack archive.
fn find_slack_url(text: &str) -> Option<String> {
    let mut rest = text;
    while let Some(start) = rest.find("https://") {
        let end = rest[start..]
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | ')' | ']'))
            .map(|i| start + i)
            .unwrap_or(rest.len());
        let url = &rest[start..end];
        if url.contains("slack.com") {
            return Some(url.to_string());
        }
        rest = &rest[end..];
    }
    None
}

/// `1700000000.000100` out of `…/p1700000000000100`, when the digits line up.
fn ts_from_permalink(permalink: &str) -> Option<String> {
    let start = permalink.rfind("/p")? + 2;
    let digits: String = permalink[start..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if digits.len() == 16 {
        Some(format!("{}.{}", &digits[..10], &digits[10..]))
    } else {
        None
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_message_tool_pattern_matches_common_names() {
        for name in [
            "slack_send_message",
            "send_message",
            "post_message",
            "slack_postMessage",
            "mcp__slack__sendMessage",
        ] {
            assert!(is_send_message_tool(name), "{name} should match");
        }
        for name in [
            "slack_search_messages",
            "slack_list_channels",
            "slack_read_canvas",
            "send_canvas",
            "message_search",
        ] {
            assert!(!is_send_message_tool(name), "{name} should not match");
        }
    }

    #[test]
    fn permalink_and_ts_are_extracted_from_structured_content() {
        let reply = serde_json::json!({
            "content": [{ "type": "text", "text": "sent" }],
            "metadata": { "mcp_meta": { "structuredContent": {
                "ok": true, "ts": "1700000000.000100",
                "permalink": "https://acme.slack.com/archives/C1/p1700000000000100"
            } } }
        });
        let (ts, permalink) = send_outcome(&reply);
        assert_eq!(ts.as_deref(), Some("1700000000.000100"));
        assert_eq!(
            permalink.as_deref(),
            Some("https://acme.slack.com/archives/C1/p1700000000000100")
        );
    }

    #[test]
    fn permalink_comes_from_text_and_ts_from_the_link() {
        let reply = serde_json::json!({
            "content": [{ "type": "text",
                "text": "Posted to #general: https://acme.slack.com/archives/C1/p1700000000000100 — done" }]
        });
        let (ts, permalink) = send_outcome(&reply);
        assert_eq!(ts.as_deref(), Some("1700000000.000100"));
        assert_eq!(
            permalink.as_deref(),
            Some("https://acme.slack.com/archives/C1/p1700000000000100")
        );
    }

    #[test]
    fn non_slack_urls_are_ignored() {
        let reply = serde_json::json!({
            "content": [{ "type": "text", "text": "see https://example.com/docs for details" }]
        });
        let (ts, permalink) = send_outcome(&reply);
        assert_eq!(ts, None);
        assert_eq!(permalink, None);
    }

    #[test]
    fn ts_from_permalink_only_accepts_slack_ts_length() {
        assert_eq!(
            ts_from_permalink("https://acme.slack.com/archives/C1/p1700000000000100").as_deref(),
            Some("1700000000.000100")
        );
        assert_eq!(ts_from_permalink("https://acme.slack.com/archives/C1/p123"), None);
        assert_eq!(ts_from_permalink("https://example.com/"), None);
    }
}
