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
/// or "post" and "message"). The argument names are taken from the tool's
/// declared `inputSchema` (e.g. the `slack` connector's `slack_send_message`
/// takes `channel_id`/`message`/`thread_ts`); the legacy `channel`/`text`/
/// `thread_ts` names are the fallback when the schema is not declared.
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
    discovered: Mutex<Option<SendTool>>,
    agent: ureq::Agent,
}

/// The resolved send tool: its name and the argument names to call it with.
#[derive(Debug, Clone)]
struct SendTool {
    name: String,
    channel_arg: String,
    text_arg: String,
    /// `None` when the tool's schema declares no thread argument.
    thread_arg: Option<String>,
}

/// Candidate property names for the channel argument, most specific first.
const CHANNEL_ARG_CANDIDATES: &[&str] = &[
    "channel_id",
    "channel",
    "channel_name",
    "destination",
    "to",
    "recipient",
];

/// Candidate property names for the message-text argument.
const TEXT_ARG_CANDIDATES: &[&str] = &["message", "text", "content", "body"];

/// Candidate property names for the thread (reply) argument.
const THREAD_ARG_CANDIDATES: &[&str] = &["thread_ts", "thread", "parent_ts", "reply_to"];

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
    /// tool list (the first tool whose name contains "send" or "post" and
    /// "message"), with argument names resolved from its `inputSchema`.
    fn send_tool(&self) -> Result<SendTool, SlackError> {
        // A pinned tool skips discovery; without a schema the legacy
        // channel/text/thread_ts names are used (overridable).
        if let Some(tool) = &self.tool {
            return Ok(SendTool {
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
        let names: Vec<&str> = tools
            .iter()
            .filter_map(|t| t.get("name").and_then(Value::as_str))
            .collect();
        let found = tools
            .iter()
            .find(|t| t.get("name").and_then(Value::as_str).is_some_and(is_send_message_tool));
        match found {
            Some(tool) => {
                let name = tool
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let props = schema_properties(tool);
                let resolved = SendTool {
                    channel_arg: self.channel_arg.clone().unwrap_or_else(|| {
                        pick_argument(props.as_ref(), CHANNEL_ARG_CANDIDATES, "channel")
                    }),
                    text_arg: self
                        .text_arg
                        .clone()
                        .unwrap_or_else(|| pick_argument(props.as_ref(), TEXT_ARG_CANDIDATES, "text")),
                    // Without a declared schema, assume the legacy thread_ts.
                    thread_arg: match &self.thread_arg {
                        Some(arg) => Some(arg.clone()),
                        None => match props {
                            Some(props) => THREAD_ARG_CANDIDATES
                                .iter()
                                .find(|c| props.contains_key(**c))
                                .map(|c| c.to_string()),
                            None => Some("thread_ts".to_string()),
                        },
                    },
                    name,
                };
                *lock() = Some(resolved.clone());
                Ok(resolved)
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
            tool.channel_arg.clone(),
            Value::String(destination.to_string()),
        );
        arguments.insert(tool.text_arg.clone(), Value::String(text.to_string()));
        if let Some(thread_ts) = thread_ts {
            match &tool.thread_arg {
                Some(arg) => {
                    arguments.insert(arg.clone(), Value::String(thread_ts.to_string()));
                }
                None => {
                    return Err(SlackError(format!(
                        "connector tool {} declares no thread argument; cannot reply in thread {thread_ts}",
                        tool.name
                    )))
                }
            }
        }
        let reply = self.post(
            &format!("/v1/connectors/{}/tools/{}/call", self.connector, tool.name),
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
                "connector tool {} failed: {}",
                tool.name,
                text_content(&reply).unwrap_or_else(|| "no detail returned".to_string())
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
}

/// `{"arguments": …}` — the tool-call request body.
fn json_arguments(arguments: Map<String, Value>) -> Map<String, Value> {
    let mut body = Map::new();
    body.insert("arguments".to_string(), Value::Object(arguments));
    body
}

/// A tool's declared `inputSchema.properties`, when present.
fn schema_properties(tool: &Value) -> Option<Map<String, Value>> {
    tool.get("inputSchema")
        .and_then(|s| s.get("properties"))
        .and_then(Value::as_object)
        .cloned()
}

/// The first candidate present in the schema's properties, else `fallback`
/// (used when the tool declares no schema at all).
fn pick_argument(
    props: Option<&Map<String, Value>>,
    candidates: &[&str],
    fallback: &str,
) -> String {
    if let Some(props) = props {
        for candidate in candidates {
            if props.contains_key(*candidate) {
                return candidate.to_string();
            }
        }
    }
    fallback.to_string()
}

/// True when a tool name looks like "send a message" (`slack_send_message`,
/// `post_message`, …).
fn is_send_message_tool(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    (n.contains("send") || n.contains("post")) && n.contains("message")
}

/// The text of a tool-call reply's text content blocks.
fn text_blocks(reply: &Value) -> Vec<&str> {
    reply
        .get("content")
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect()
        })
        .unwrap_or_default()
}

/// The joined text of a tool-call reply's text content blocks.
fn text_content(reply: &Value) -> Option<String> {
    let texts = text_blocks(reply);
    if texts.is_empty() {
        None
    } else {
        Some(texts.join("\n"))
    }
}

/// What a successful tool call reported (every field optional; the caller
/// falls back).
#[derive(Debug, Default)]
struct SendOutcome {
    ts: Option<String>,
    permalink: Option<String>,
    channel: Option<String>,
}

/// Best-effort outcome extraction from a tool-call reply. The slack
/// connector reports `{"message_link": …, "message_context": {…}}` as a JSON
/// string inside a text block; other connectors use MCP structured content;
/// a plain Slack archives URL in the text is the last resort (the `ts` is
/// parsed out of the permalink when only a link is returned).
fn send_outcome(reply: &Value) -> SendOutcome {
    let mut out = SendOutcome::default();
    // 1. MCP structured content.
    let structured = reply
        .get("metadata")
        .and_then(|m| m.get("mcp_meta"))
        .and_then(|m| m.get("structuredContent").or_else(|| m.get("structured_content")))
        .and_then(Value::as_object);
    if let Some(s) = structured {
        out.ts = string_field(s, &["ts", "message_ts"]);
        out.permalink = string_field(s, &["permalink", "url", "link"])
            .filter(|url| url.starts_with("http"));
        out.channel = string_field(s, &["channel", "channel_id"]);
    }
    // 2. A text block that parses as a JSON object.
    if out.permalink.is_none() || out.ts.is_none() || out.channel.is_none() {
        for text in text_blocks(reply) {
            let Ok(value) = serde_json::from_str::<Value>(text) else {
                continue;
            };
            let Some(obj) = value.as_object() else { continue };
            if out.permalink.is_none() {
                out.permalink = string_field(obj, &["message_link", "permalink", "url", "link"])
                    .filter(|url| url.starts_with("http"));
            }
            let context = obj.get("message_context").and_then(Value::as_object);
            if out.ts.is_none() {
                out.ts = context
                    .and_then(|c| string_field(c, &["message_ts", "ts"]))
                    .or_else(|| string_field(obj, &["message_ts", "ts"]));
            }
            if out.channel.is_none() {
                out.channel = context
                    .and_then(|c| string_field(c, &["channel_id", "channel"]))
                    .or_else(|| string_field(obj, &["channel_id", "channel"]));
            }
        }
    }
    // 3. Any Slack archives URL in the text (also after unescaping JSON `\/`).
    if out.permalink.is_none() {
        if let Some(text) = text_content(reply) {
            out.permalink =
                find_slack_url(&text).or_else(|| find_slack_url(&text.replace("\\/", "/")));
        }
    }
    if out.ts.is_none() {
        if let Some(link) = &out.permalink {
            out.ts = ts_from_permalink(link);
        }
    }
    out
}

/// The first of `keys` present as a non-empty string in `obj`.
fn string_field(obj: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|k| obj.get(*k))
        .and_then(Value::as_str)
        .map(str::to_string)
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
        let out = send_outcome(&reply);
        assert_eq!(out.ts.as_deref(), Some("1700000000.000100"));
        assert_eq!(
            out.permalink.as_deref(),
            Some("https://acme.slack.com/archives/C1/p1700000000000100")
        );
    }

    #[test]
    fn json_text_block_yields_link_ts_and_channel() {
        // The real slack connector's reply: a JSON string inside the text
        // block, with JSON-escaped slashes.
        let reply = serde_json::json!({
            "content": [{ "type": "text", "text":
                "{\"message_link\":\"https:\\/\\/mistralai.slack.com\\/archives\\/D09CV6A8SF7\\/p1792850219522549\",\"message_context\":{\"message_ts\":\"1792850219.522549\",\"channel_id\":\"D09CV6A8SF7\"}}"
            }]
        });
        let out = send_outcome(&reply);
        assert_eq!(out.ts.as_deref(), Some("1792850219.522549"));
        assert_eq!(
            out.permalink.as_deref(),
            Some("https://mistralai.slack.com/archives/D09CV6A8SF7/p1792850219522549")
        );
        assert_eq!(out.channel.as_deref(), Some("D09CV6A8SF7"));
    }

    #[test]
    fn permalink_comes_from_text_and_ts_from_the_link() {
        let reply = serde_json::json!({
            "content": [{ "type": "text",
                "text": "Posted to #general: https://acme.slack.com/archives/C1/p1700000000000100 — done" }]
        });
        let out = send_outcome(&reply);
        assert_eq!(out.ts.as_deref(), Some("1700000000.000100"));
        assert_eq!(
            out.permalink.as_deref(),
            Some("https://acme.slack.com/archives/C1/p1700000000000100")
        );
    }

    #[test]
    fn non_slack_urls_are_ignored() {
        let reply = serde_json::json!({
            "content": [{ "type": "text", "text": "see https://example.com/docs for details" }]
        });
        let out = send_outcome(&reply);
        assert_eq!(out.ts, None);
        assert_eq!(out.permalink, None);
        assert_eq!(out.channel, None);
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
