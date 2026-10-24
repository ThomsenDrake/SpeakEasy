//! Shared plumbing for tool-call-based Slack transports (the Mistral Slack
//! Connector and the official Slack MCP server): send-tool discovery with
//! schema-adapted argument names, and tolerant parsing of MCP
//! CallToolResult replies.

use crate::core::SlackError;
use serde_json::{Map, Value};

/// The resolved send tool: its name and the argument names to call it with.
#[derive(Debug, Clone)]
pub(crate) struct SendToolSpec {
    pub name: String,
    pub channel_arg: String,
    pub text_arg: String,
    /// `None` when the tool's schema declares no thread argument.
    pub thread_arg: Option<String>,
}

/// Candidate property names for the channel argument, most specific first.
pub(crate) const CHANNEL_ARG_CANDIDATES: &[&str] = &[
    "channel_id",
    "channel",
    "channel_name",
    "destination",
    "to",
    "recipient",
];

/// Candidate property names for the message-text argument.
pub(crate) const TEXT_ARG_CANDIDATES: &[&str] = &["message", "text", "content", "body"];

/// Candidate property names for the thread (reply) argument.
pub(crate) const THREAD_ARG_CANDIDATES: &[&str] = &["thread_ts", "thread", "parent_ts", "reply_to"];

/// A tool's declared `inputSchema.properties`, when present.
pub(crate) fn schema_properties(tool: &Value) -> Option<Map<String, Value>> {
    tool.get("inputSchema")
        .and_then(|s| s.get("properties"))
        .and_then(Value::as_object)
        .cloned()
}

/// The first candidate present in the schema's properties, else `fallback`
/// (used when the tool declares no schema at all).
pub(crate) fn pick_argument(
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
pub(crate) fn is_send_message_tool(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    (n.contains("send") || n.contains("post")) && n.contains("message")
}

/// Resolve the send-message tool from a tool list: the first whose name
/// looks like "send a message", with argument names detected from its
/// declared `inputSchema` (legacy `channel`/`text`/`thread_ts` when no schema
/// is declared). Explicit overrides win. `no_tool_hint` is appended to the
/// error when no tool matches (e.g. "set MISTRAL_SLACK_TOOL to …").
pub(crate) fn resolve_send_tool(
    tools: &[Value],
    channel_override: Option<String>,
    text_override: Option<String>,
    thread_override: Option<String>,
    no_tool_hint: &str,
) -> Result<SendToolSpec, SlackError> {
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
            Ok(SendToolSpec {
                channel_arg: channel_override.unwrap_or_else(|| {
                    pick_argument(props.as_ref(), CHANNEL_ARG_CANDIDATES, "channel")
                }),
                text_arg: text_override
                    .unwrap_or_else(|| pick_argument(props.as_ref(), TEXT_ARG_CANDIDATES, "text")),
                // Without a declared schema, assume the legacy thread_ts.
                thread_arg: match thread_override {
                    Some(arg) => Some(arg),
                    None => match props {
                        Some(props) => THREAD_ARG_CANDIDATES
                            .iter()
                            .find(|c| props.contains_key(**c))
                            .map(|c| c.to_string()),
                        None => Some("thread_ts".to_string()),
                    },
                },
                name,
            })
        }
        None => Err(SlackError(format!(
            "no send-message tool in the list (available: {}); {no_tool_hint}",
            if names.is_empty() {
                "<none listed>".to_string()
            } else {
                names.join(", ")
            }
        ))),
    }
}

/// `{"arguments": …}` — the tool-call arguments object for an exact send:
/// the exact `text` to the exact `destination`, optionally as a reply in
/// `thread_ts`. Errors when a thread reply is requested but the tool
/// declares no thread argument (nothing is sent in that case).
pub(crate) fn send_arguments(
    spec: &SendToolSpec,
    destination: &str,
    thread_ts: Option<&str>,
    text: &str,
) -> Result<Map<String, Value>, SlackError> {
    let mut arguments = Map::new();
    arguments.insert(
        spec.channel_arg.clone(),
        Value::String(destination.to_string()),
    );
    arguments.insert(spec.text_arg.clone(), Value::String(text.to_string()));
    if let Some(thread_ts) = thread_ts {
        match &spec.thread_arg {
            Some(arg) => {
                arguments.insert(arg.clone(), Value::String(thread_ts.to_string()));
            }
            None => {
                return Err(SlackError(format!(
                    "send tool {} declares no thread argument; cannot reply in thread {thread_ts}",
                    spec.name
                )))
            }
        }
    }
    Ok(arguments)
}

/// `{"arguments": …}` — the tool-call request body (MCP `tools/call` params
/// shape is built by the caller; this wraps a bare arguments object).
pub(crate) fn json_arguments(arguments: Map<String, Value>) -> Map<String, Value> {
    let mut body = Map::new();
    body.insert("arguments".to_string(), Value::Object(arguments));
    body
}

/// The error text of a failed tool call (`isError: true` in the MCP
/// CallToolResult — top-level per the MCP spec, or wrapped in
/// `metadata.mcp_meta` by some gateways), or `None` on success.
pub(crate) fn tool_result_error(reply: &Value) -> Option<String> {
    let is_error = |m: &Value| {
        m.get("isError")
            .or_else(|| m.get("is_error"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    };
    let errored = is_error(reply)
        || reply
            .get("metadata")
            .and_then(|m| m.get("mcp_meta"))
            .is_some_and(is_error);
    if errored {
        Some(
            text_content(reply)
                .unwrap_or_else(|| "no detail returned".to_string()),
        )
    } else {
        None
    }
}

/// The text of a tool-call reply's text content blocks.
pub(crate) fn text_blocks(reply: &Value) -> Vec<&str> {
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
pub(crate) fn text_content(reply: &Value) -> Option<String> {
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
pub(crate) struct SendOutcome {
    pub ts: Option<String>,
    pub permalink: Option<String>,
    pub channel: Option<String>,
}

/// Best-effort outcome extraction from a tool-call reply. Standard MCP
/// servers return `structuredContent` (top-level); some gateways wrap it in
/// `metadata.mcp_meta`; the Slack tools report
/// `{"message_link": …, "message_context": {…}}` as a JSON string inside a
/// text block; a plain Slack archives URL in the text is the last resort
/// (the `ts` is parsed out of the permalink when only a link is returned).
pub(crate) fn send_outcome(reply: &Value) -> SendOutcome {
    let mut out = SendOutcome::default();
    // 1. Structured content: MCP top-level, or wrapped in mcp_meta; then the
    //    result object's own fields.
    let structured = reply
        .get("structuredContent")
        .or_else(|| reply.get("structured_content"))
        .or_else(|| {
            reply
                .get("metadata")
                .and_then(|m| m.get("mcp_meta"))
                .and_then(|m| m.get("structuredContent").or_else(|| m.get("structured_content")))
        })
        .and_then(Value::as_object);
    let empty = Map::new();
    let sources: Vec<&Map<String, Value>> = match structured {
        Some(s) => vec![s, reply.as_object().unwrap_or(&empty)],
        None => vec![reply.as_object().unwrap_or(&empty)],
    };
    for s in sources {
        if out.ts.is_none() {
            out.ts = string_field(s, &["ts", "message_ts"]);
        }
        if out.permalink.is_none() {
            out.permalink = string_field(s, &["permalink", "message_link", "url", "link"])
                .filter(|url| url.starts_with("http"));
        }
        if out.channel.is_none() {
            out.channel = string_field(s, &["channel", "channel_id"]);
        }
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
pub(crate) fn string_field(obj: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|k| obj.get(*k))
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// A tool reply's first text block parsed as JSON (the Slack tools wrap
/// their results in a JSON string: `{"result": …}` / `{"messages": …}` /
/// `{"message_link": …, "message_context": …}`).
pub(crate) fn json_text(reply: &Value) -> Option<Value> {
    let text = text_blocks(reply).into_iter().next()?;
    serde_json::from_str(text).ok()
}

/// The trimmed value of a `Key: value` line in a formatted tool result.
pub(crate) fn field_line(text: &str, key: &str) -> Option<String> {
    text.lines()
        .find_map(|line| {
            line.strip_prefix(key)?
                .strip_prefix(':')
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_string)
        })
}

/// The first `https://…` URL in `text` that points at a Slack archive.
pub(crate) fn find_slack_url(text: &str) -> Option<String> {
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
pub(crate) fn ts_from_permalink(permalink: &str) -> Option<String> {
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
    fn resolve_send_tool_detects_argument_names_from_the_schema() {
        let tools = serde_json::json!([
            { "name": "slack_search_messages" },
            { "name": "slack_send_message", "inputSchema": { "type": "object", "properties": {
                "channel_id": { "type": "string" }, "message": { "type": "string" },
                "thread_ts": { "type": "string" }, "reply_broadcast": { "type": "boolean" }
            } } }
        ])
        .as_array()
        .unwrap()
        .clone();
        let spec = resolve_send_tool(&tools, None, None, None, "pin it").unwrap();
        assert_eq!(spec.name, "slack_send_message");
        assert_eq!(spec.channel_arg, "channel_id");
        assert_eq!(spec.text_arg, "message");
        assert_eq!(spec.thread_arg.as_deref(), Some("thread_ts"));

        // Overrides win over the schema.
        let spec = resolve_send_tool(
            &tools,
            Some("channel".to_string()),
            Some("text".to_string()),
            Some("parent_ts".to_string()),
            "pin it",
        )
        .unwrap();
        assert_eq!(spec.channel_arg, "channel");
        assert_eq!(spec.text_arg, "text");
        assert_eq!(spec.thread_arg.as_deref(), Some("parent_ts"));
    }

    #[test]
    fn resolve_send_tool_without_schema_uses_legacy_names() {
        let tools = serde_json::json!([{ "name": "post_message" }])
            .as_array()
            .unwrap()
            .clone();
        let spec = resolve_send_tool(&tools, None, None, None, "pin it").unwrap();
        assert_eq!(spec.channel_arg, "channel");
        assert_eq!(spec.text_arg, "text");
        assert_eq!(spec.thread_arg.as_deref(), Some("thread_ts"));
    }

    #[test]
    fn resolve_send_tool_without_a_match_lists_names_and_the_hint() {
        let tools = serde_json::json!([{ "name": "slack_list_channels" }])
            .as_array()
            .unwrap()
            .clone();
        let err = resolve_send_tool(&tools, None, None, None, "set MISTRAL_SLACK_TOOL").unwrap_err();
        assert!(err.0.contains("no send-message tool"), "{}", err.0);
        assert!(err.0.contains("slack_list_channels"), "{}", err.0);
        assert!(err.0.contains("set MISTRAL_SLACK_TOOL"), "{}", err.0);
    }

    #[test]
    fn send_arguments_carry_the_exact_values_and_no_thread_key_when_none() {
        let tools = serde_json::json!([{ "name": "slack_send_message", "inputSchema": {
            "type": "object", "properties": {
                "channel_id": {}, "message": {}, "thread_ts": {}
            } } }])
        .as_array()
        .unwrap()
        .clone();
        let spec = resolve_send_tool(&tools, None, None, None, "pin it").unwrap();

        let args = send_arguments(&spec, "#eng", None, "hi").unwrap();
        assert_eq!(
            Value::Object(args.clone()),
            serde_json::json!({ "channel_id": "#eng", "message": "hi" })
        );
        assert!(!args.contains_key("thread_ts"));

        let args = send_arguments(&spec, "#eng", Some("1699999999.000200"), "hi").unwrap();
        assert_eq!(
            Value::Object(args),
            serde_json::json!({ "channel_id": "#eng", "message": "hi", "thread_ts": "1699999999.000200" })
        );
    }

    #[test]
    fn send_arguments_refuse_a_thread_reply_the_tool_cannot_express() {
        let tools = serde_json::json!([{ "name": "slack_send_message", "inputSchema": {
            "type": "object", "properties": { "channel_id": {}, "message": {} } } }])
        .as_array()
        .unwrap()
        .clone();
        let spec = resolve_send_tool(&tools, None, None, None, "pin it").unwrap();
        let err = send_arguments(&spec, "#eng", Some("1699999999.000200"), "hi").unwrap_err();
        assert!(err.0.contains("no thread argument"), "{}", err.0);
        assert!(err.0.contains("slack_send_message"), "{}", err.0);
    }

    #[test]
    fn tool_result_error_reads_top_level_and_wrapped_is_error() {
        let top_level = serde_json::json!({
            "content": [{ "type": "text", "text": "channel_not_found" }], "isError": true
        });
        assert_eq!(tool_result_error(&top_level).as_deref(), Some("channel_not_found"));
        let wrapped = serde_json::json!({
            "content": [{ "type": "text", "text": "no_text" }],
            "metadata": { "mcp_meta": { "isError": true } }
        });
        assert_eq!(tool_result_error(&wrapped).as_deref(), Some("no_text"));
        let ok = serde_json::json!({ "content": [{ "type": "text", "text": "sent" }] });
        assert_eq!(tool_result_error(&ok), None);
    }

    #[test]
    fn permalink_and_ts_are_extracted_from_structured_content() {
        let reply = serde_json::json!({
            "content": [{ "type": "text", "text": "sent" }],
            "structuredContent": {
                "ok": true, "ts": "1700000000.000100",
                "permalink": "https://acme.slack.com/archives/C1/p1700000000000100"
            }
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
        // The Slack tools' reply: a JSON string inside the text block, with
        // JSON-escaped slashes.
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
