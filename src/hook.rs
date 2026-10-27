//! Pre-tool hook mode: `slack-approval-gateway --hook <agent> [--wait SECS]`.
//!
//! The hooks-based model: an agent keeps its own Slack MCP server (Claude
//! Code's Slack plugin, a Mistral connector, korotovsky's server, …) and
//! sends with that server's own tools. A pre-tool hook registered in the
//! agent runs this mode before every such call: it reads the agent's hook
//! payload on stdin, recognizes a Slack write, asks the human in SpeakEasy's
//! approval window (over the running instance's socket, method
//! [`crate::mcp::APPROVE_METHOD`]) and answers the agent in that agent's
//! hook format: approved calls run; denied ones are blocked with a reason
//! the model sees. SpeakEasy itself sends nothing and needs no credential.
//!
//! Fails closed: an unreadable payload that mentions Slack, an unreachable
//! SpeakEasy, an error, or no decision in time blocks the call. The hook
//! always answers before `--wait` seconds pass, because every agent runs the
//! tool when its hook times out; the agent's hook timeout must be longer
//! (the installer registers `--wait` + [`HOOK_TIMEOUT_MARGIN_SECS`]).
//! Calls that are not Slack writes get the agent's neutral answer, so the
//! agent's own permission rules still apply to them.

use crate::core::{ConversationContext, IncomingRequest};
use crate::instance;
use crate::mcp::{is_slack_ts, APPROVE_METHOD};
use serde_json::{json, Map, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// Default `--wait`: how long the human has to decide.
pub const DEFAULT_WAIT_SECS: u64 = 600;

/// How much longer than `--wait` the agent's own hook timeout must be.
pub const HOOK_TIMEOUT_MARGIN_SECS: u64 = 60;

/// The instance withdraws the request this long before `--wait` runs out,
/// so its answer (an error, never an approval) arrives first.
const INSTANCE_DEADLINE_MARGIN_SECS: u64 = 5;

/// Hook payloads larger than this are not read in full (and fail closed).
const MAX_PAYLOAD_BYTES: u64 = 16 * 1024 * 1024;

/// SpeakEasy's own MCP server (as agents spell it) and tool: never gated,
/// since the tool has its own approval popup.
const OWN_SERVERS: [&str; 2] = ["slack-approval-gateway", "slack_approval_gateway"];
const OWN_TOOL: &str = "request_slack_message";

/// Slack tools whose names do not contain "slack" (korotovsky's server),
/// recognized even when the server was registered under another name.
const SLACK_TOOLS_WITHOUT_SLACK_IN_NAME: [&str; 22] = [
    "conversations_add_message",
    "conversations_history",
    "conversations_replies",
    "conversations_search_messages",
    "conversations_unreads",
    "conversations_mark",
    "conversations_join",
    "conversations_leave",
    "channels_list",
    "channels_me",
    "reactions_add",
    "reactions_remove",
    "usergroups_list",
    "usergroups_create",
    "usergroups_update",
    "usergroups_users_update",
    "usergroups_me",
    "users_search",
    "saved_list",
    "saved_update",
    "saved_clear_completed",
    "attachment_get_data",
];

/// Slack tools that never write anything others can see, by exact name
/// (compared with the end of the tool name): drafts, the user's own read
/// marks and saved items, sign-in, step 1 of an upload (step 2,
/// `slack_complete_file_upload`, shares the file and is asked about), and
/// reads whose names carry no read verb.
const NOT_WRITES: [&str; 8] = [
    "slack_send_message_draft",
    "conversations_mark",
    "saved_update",
    "saved_clear_completed",
    "complete_authentication",
    "authenticate",
    "slack_get_file_upload_url",
    "channels_me",
];

/// Verbs (and verb-like words) that make a tool a write. They win over read
/// verbs: `get_and_send` writes.
const WRITE_WORDS: [&str; 44] = [
    "send", "post", "schedule", "reply", "respond", "add", "update", "edit", "delete",
    "remove", "upload", "share", "create", "invite", "kick", "join", "leave", "archive",
    "unarchive", "rename", "set", "pin", "unpin", "remind", "react", "publish", "forward",
    "write", "put", "patch", "broadcast", "dm", "notify", "mention", "ping", "ask", "tell",
    "announce", "alert", "say", "submit", "complete", "bookmark", "insert",
];

/// Verbs that make a tool a read when no write word is present.
const READ_WORDS: [&str; 20] = [
    "read", "search", "list", "get", "fetch", "history", "replies", "info", "lookup", "find",
    "view", "describe", "permalink", "unreads", "query", "download", "count", "check",
    "preview", "browse",
];

/// Argument names, in priority order.
const DESTINATION_KEYS: [&str; 17] = [
    "channel_id",
    "channelId",
    "channel",
    "conversation_id",
    "conversationId",
    "conversation",
    "user_id",
    "userId",
    "user",
    "users",
    "recipient",
    "to",
    "destination",
    "target",
    "channel_name",
    "user_ids",
    "canvas_id",
];
const TEXT_KEYS: [&str; 8] = [
    "message",
    "text",
    "payload",
    "content",
    "body",
    "markdown",
    "markdown_text",
    "msg",
];
const THREAD_KEYS: [&str; 4] = ["thread_ts", "threadTs", "thread_id", "thread"];

/// Rich-content arguments that Slack renders instead of (or besides) the
/// text: when present, the whole call is shown, not just the text.
const RICH_KEYS: [&str; 3] = ["blocks", "attachments", "sections"];

/// The agents this mode speaks to. Each has its own hook payload and
/// answer format (see [`render`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agent {
    /// Claude Code. Its `~/.claude/settings.json` hooks also run in pdev and,
    /// imported, in Cursor; the display name follows the environment.
    Claude,
    Codex,
    Gemini,
    Copilot,
    Cursor,
    Vibe,
    Goose,
    /// The plugins SpeakEasy installs for these agents call the hook with a
    /// Claude-style payload and read the goose-style block answer.
    OpenCode,
    Pi,
    Hermes,
    Amp,
}

impl Agent {
    pub fn parse(name: &str) -> Option<Agent> {
        Some(match name.to_ascii_lowercase().as_str() {
            "claude" | "claude-code" | "claude_code" | "pdev" | "paradev" => Agent::Claude,
            "codex" => Agent::Codex,
            "gemini" | "gemini-cli" => Agent::Gemini,
            "copilot" | "copilot-cli" => Agent::Copilot,
            "cursor" | "cursor-agent" => Agent::Cursor,
            "vibe" | "mistral-vibe" => Agent::Vibe,
            "goose" => Agent::Goose,
            "opencode" => Agent::OpenCode,
            "pi" => Agent::Pi,
            "hermes" => Agent::Hermes,
            "amp" => Agent::Amp,
            _ => return None,
        })
    }

    /// The agent's name as shown in the approval window.
    pub fn display_name(self) -> &'static str {
        let env = |k: &str| std::env::var_os(k).is_some_and(|v| !v.is_empty());
        match self {
            // Claude Code's settings hooks also run in pdev and Cursor.
            Agent::Claude if env("PDEV_VERSION") || env("PDEV_PROJECT_DIR") => "pdev",
            Agent::Claude if env("CURSOR_VERSION") => "Cursor",
            Agent::Claude => "Claude Code",
            Agent::Codex => "Codex",
            Agent::Gemini => "Gemini CLI",
            Agent::Copilot => "Copilot CLI",
            Agent::Cursor => "Cursor",
            Agent::Vibe => "Mistral Vibe",
            Agent::Goose => "goose",
            Agent::OpenCode => "OpenCode",
            Agent::Pi => "pi",
            Agent::Hermes => "Hermes",
            Agent::Amp => "Amp",
        }
    }
}

/// One tool call, as read from a hook payload.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub name: String,
    pub input: Value,
    /// The MCP server, when the payload names it separately.
    pub server: Option<String>,
    /// The bare MCP tool name, when the payload names it separately.
    pub tool: Option<String>,
    pub session: Option<String>,
    pub cwd: Option<String>,
    /// The agent's ID for this tool call: two hooks that stop the same call
    /// (Cursor runs its own and Claude Code's) share one approval.
    pub call_id: Option<String>,
}

/// What the human is asked to approve.
#[derive(Debug, Clone, PartialEq)]
pub struct Proposal {
    pub server: Option<String>,
    pub tool: String,
    pub destination: String,
    /// False when the call names no destination (the destination shown is
    /// a placeholder; the deny cooldown does not apply).
    pub destination_known: bool,
    pub message: String,
    pub thread_ts: Option<String>,
    /// Arguments other than the destination, text and thread, shown with the
    /// reason (the call is approved as a whole).
    pub other_arguments: Map<String, Value>,
    /// True when the call has a text argument (sent verbatim); false when
    /// the message shown is a rendering of all the arguments.
    pub has_text: bool,
}

/// How a tool call is handled.
#[derive(Debug, Clone, PartialEq)]
pub enum Classification {
    /// Not a Slack tool, a Slack read, or SpeakEasy's own tool.
    Pass,
    /// A Slack write: ask the human.
    Ask(Proposal),
}

/// The hook's answer.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// No opinion: the agent's normal permission flow continues.
    Neutral,
    /// The human approved this exact call in SpeakEasy.
    Approved,
    /// Blocked; the reason is shown to the model.
    Deny(String),
}

/// What to write and how to exit.
#[derive(Debug, Clone, PartialEq)]
pub struct Rendered {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

/// Read a hook payload. Field names differ between agents; the common
/// spellings are all accepted.
pub fn parse_payload(raw: &str) -> Result<ToolCall, String> {
    let payload: Value =
        serde_json::from_str(raw).map_err(|e| format!("the hook payload is not JSON: {e}"))?;
    let Value::Object(payload) = payload else {
        return Err("the hook payload is not a JSON object".into());
    };
    // Blank values count as absent (some plugins send "" for unknown ids).
    let str_field = |keys: &[&str]| {
        keys.iter()
            .find_map(|k| payload.get(*k).and_then(Value::as_str).filter(|v| !v.trim().is_empty()))
            .map(str::to_string)
    };
    let name = str_field(&["tool_name", "toolName", "tool", "name"])
        .ok_or("the hook payload has no tool name")?;
    let mut input = ["tool_input", "toolArgs", "tool_args", "args", "arguments", "input"]
        .iter()
        .find_map(|k| payload.get(*k).cloned())
        .unwrap_or(Value::Null);
    if let Value::String(text) = &input {
        // Some agents pass the arguments as a JSON string.
        if let Ok(parsed) = serde_json::from_str::<Value>(text) {
            input = parsed;
        }
    }
    let context = payload.get("mcp_context");
    let server = payload
        .get("mcp_server")
        .and_then(|s| s.get("name").and_then(Value::as_str).or(s.as_str()))
        .or_else(|| context.and_then(|c| c.get("server_name")).and_then(Value::as_str))
        .or_else(|| payload.get("mcp_server_name").and_then(Value::as_str))
        .or_else(|| payload.get("server_name").and_then(Value::as_str))
        .map(str::to_string);
    let tool = context
        .and_then(|c| c.get("tool_name"))
        .and_then(Value::as_str)
        .or_else(|| payload.get("mcp_tool").and_then(Value::as_str))
        .map(str::to_string);
    let cwd = str_field(&["cwd", "working_dir", "workdir"]).or_else(|| {
        payload
            .get("workspace_roots")
            .and_then(|r| r.get(0))
            .and_then(Value::as_str)
            .map(str::to_string)
    });
    Ok(ToolCall {
        name,
        input,
        server,
        tool,
        session: str_field(&["session_id", "sessionId", "conversation_id", "thread_id"]),
        cwd,
        call_id: str_field(&["tool_use_id", "toolUseId", "toolUseID", "tool_call_id", "call_id"]),
    })
}

/// Split a tool name into (server, tool) for the agents' MCP naming
/// schemes: `mcp__server__tool` (Claude Code, Codex, pdev, Hermes, Amp),
/// `mcp_server.tool` (Vibe), `server__tool` (goose), `MCP:tool` (Cursor).
/// Names that cannot be split unambiguously (Gemini's `mcp_server_tool`,
/// Copilot's `server-tool`, Vibe's `connector_alias_tool`) stay whole: the
/// classification works on their words.
pub fn split_tool_name(name: &str) -> (Option<String>, String) {
    // Cursor matches and reports MCP tools as `MCP:<tool>` (no server).
    if let Some(tool) = name.strip_prefix("MCP:") {
        return (None, tool.to_string());
    }
    if let Some(rest) = name.strip_prefix("mcp__") {
        return match rest.split_once("__") {
            Some((server, tool)) => (Some(server.to_string()), tool.to_string()),
            None => (Some(rest.to_string()), String::new()),
        };
    }
    if let Some((prefix, tool)) = name.split_once('.') {
        for kind in ["mcp_", "connector_"] {
            if let Some(server) = prefix.strip_prefix(kind) {
                return (Some(server.to_string()), tool.to_string());
            }
        }
    }
    if let Some((server, tool)) = name.split_once("__") {
        if !server.is_empty() && !tool.is_empty() {
            return (Some(server.to_string()), tool.to_string());
        }
    }
    (None, name.to_string())
}

/// Lowercase words of a tool name (`slack_send_message`,
/// `sendMessage`, `post-message` → send, message …).
fn words(name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut prev_lower = false;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            if c.is_ascii_uppercase() && prev_lower && !word.is_empty() {
                out.push(std::mem::take(&mut word));
            }
            prev_lower = c.is_ascii_lowercase() || c.is_ascii_digit();
            word.push(c.to_ascii_lowercase());
        } else {
            prev_lower = false;
            if !word.is_empty() {
                out.push(std::mem::take(&mut word));
            }
        }
    }
    if !word.is_empty() {
        out.push(word);
    }
    out
}

/// A word and its singular/verb form (`sends` → `send`, `replies` stays).
fn stems(word: &str) -> [&str; 3] {
    [
        word,
        word.strip_suffix('s').unwrap_or(word),
        word.strip_suffix("es").unwrap_or(word),
    ]
}

/// Whether a Slack tool writes (posts, schedules, reacts, edits, …) as
/// anyone else can see it. Asks by default: a tool is a read only when it
/// has a read verb and no write word, or is a known self-only tool.
pub fn is_write_tool(tool: &str) -> bool {
    let lower = tool.to_ascii_lowercase();
    if NOT_WRITES.iter().any(|t| lower == *t || lower.ends_with(&format!("_{t}"))) {
        return false;
    }
    let w = words(tool);
    let any = |list: &[&str]| w.iter().any(|x| stems(x).iter().any(|s| list.contains(s)));
    if any(&WRITE_WORDS) {
        return true;
    }
    if any(&READ_WORDS) {
        return false;
    }
    // No read verb: `slack_message_user`, `slack_frobnicate`, …
    true
}

/// [`classify`] plus what only one agent needs: goose's Code Mode runs MCP
/// tools inside `execute_typescript` without hooking the nested calls, so
/// the program itself is checked for Slack writes. (pi, pdev, Vibe and
/// OpenCode hook each nested call; checking their programs too would ask
/// twice.)
pub fn classify_for(agent: Agent, call: &ToolCall) -> Classification {
    if agent == Agent::Goose {
        let (_, tool) = split_tool_name(&call.name);
        if tool.eq_ignore_ascii_case("execute_typescript") {
            let program = all_strings(&call.input).join("\n");
            if !slack_write_in_code(&program) {
                return Classification::Pass;
            }
            let mut p = proposal(Some("goose code mode".into()), tool, &Value::Null);
            p.message = program;
            p.destination = "(see the program)".into();
            return Classification::Ask(p);
        }
    }
    classify(call)
}

/// Every string in a JSON value, depth-first.
fn all_strings(value: &Value) -> Vec<String> {
    match value {
        Value::String(s) => vec![s.clone()],
        Value::Array(items) => items.iter().flat_map(all_strings).collect(),
        Value::Object(map) => map.values().flat_map(all_strings).collect(),
        _ => Vec::new(),
    }
}

/// Decide what to do with a tool call.
pub fn classify(call: &ToolCall) -> Classification {
    let (split_server, split_tool) = split_tool_name(&call.name);
    let server = call.server.clone().or(split_server);
    let tool = call.tool.clone().unwrap_or(split_tool);
    let lower_name = call.name.to_ascii_lowercase();
    let lower_server = server.as_deref().unwrap_or("").to_ascii_lowercase();
    let lower_tool = tool.to_ascii_lowercase();

    // Executor reaches Slack connectors through its generic `call` and
    // `execute` tools; agents that do not name the server separately still
    // show "executor" in the full name (`executor-call`, `mcp_executor_call`).
    if lower_server.contains("executor") || lower_name.contains("executor") {
        for generic in ["call", "execute"] {
            if lower_tool == generic || lower_name.ends_with(&format!("_{generic}"))
                || lower_name.ends_with(&format!("-{generic}"))
                || lower_name.ends_with(&format!(".{generic}"))
            {
                return executor_classification(generic, &call.input);
            }
        }
    }
    // SpeakEasy's own tool, however the agent spells its server.
    let own_server = OWN_SERVERS.iter().any(|n| lower_server == *n || lower_name.contains(n));
    if own_server && (lower_tool == OWN_TOOL || lower_name.ends_with(OWN_TOOL)) {
        return Classification::Pass;
    }
    let slack = lower_name.contains("slack")
        || lower_server.contains("slack")
        || SLACK_TOOLS_WITHOUT_SLACK_IN_NAME
            .iter()
            .any(|t| lower_tool.ends_with(t) || lower_name.ends_with(t));
    if !slack || tool.is_empty() || !is_write_tool(&tool) {
        return Classification::Pass;
    }
    Classification::Ask(proposal(server, tool, &call.input))
}

/// Executor (an MCP host whose `call` and `execute` tools run other apps'
/// tools by path, e.g. `tools.mistral.slack.slack_send_message`).
fn executor_classification(tool: &str, input: &Value) -> Classification {
    match tool {
        "call" => {
            let calls: Vec<&Value> = match input.get("calls") {
                Some(Value::Array(list)) => list.iter().collect(),
                Some(one @ Value::Object(_)) => vec![one],
                _ => Vec::new(),
            };
            let writes: Vec<(&str, &Value)> = calls
                .iter()
                .filter_map(|c| {
                    let path = c.get("path")?.as_str()?;
                    let last = path.rsplit('.').next().unwrap_or(path);
                    (is_slack_name(path) && is_write_tool(last))
                        .then(|| (path, c.get("input").unwrap_or(&Value::Null)))
                })
                .collect();
            match writes.as_slice() {
                [] => Classification::Pass,
                [(path, inner)] => {
                    let (prefix, last) = path.rsplit_once('.').unwrap_or(("", path));
                    Classification::Ask(proposal(
                        Some(format!("executor {prefix}")),
                        last.to_string(),
                        inner,
                    ))
                }
                _ => Classification::Ask(proposal(
                    Some("executor".into()),
                    "call".into(),
                    &json!({ "calls": input.get("calls") }),
                )),
            }
        }
        "execute" => {
            let code = all_strings(input).join("\n");
            if slack_write_in_code(&code) {
                let mut p = proposal(Some("executor".into()), "execute".into(), &Value::Null);
                p.message = code;
                p.destination = "(see the program)".into();
                Classification::Ask(p)
            } else {
                Classification::Pass
            }
        }
        _ => Classification::Pass,
    }
}

/// Whether a tool name or path names a Slack tool: "slack" anywhere, or
/// one of korotovsky's tool names at the end.
fn is_slack_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("slack")
        || SLACK_TOOLS_WITHOUT_SLACK_IN_NAME
            .iter()
            .any(|t| lower.ends_with(t))
}

/// Undo the simple ways a program can hide a name from a text search:
/// `\uXXXX` / `\u{…}` / `\xNN` escapes and adjacent string literals joined
/// with `+` (`'sl' + 'ack'`).
fn normalize_program(code: &str) -> String {
    let mut out = String::with_capacity(code.len());
    let chars: Vec<char> = code.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && i + 1 < chars.len() {
            let hex = |from: usize, len: usize| -> Option<char> {
                let s: String = chars.get(from..from + len)?.iter().collect();
                char::from_u32(u32::from_str_radix(&s, 16).ok()?)
            };
            match chars[i + 1] {
                'u' if chars.get(i + 2) == Some(&'{') => {
                    if let Some(end) = chars[i + 3..].iter().position(|c| *c == '}') {
                        if let Some(ch) = hex(i + 3, end) {
                            out.push(ch);
                            i += 4 + end;
                            continue;
                        }
                    }
                }
                'u' => {
                    if let Some(ch) = hex(i + 2, 4) {
                        out.push(ch);
                        i += 6;
                        continue;
                    }
                }
                'x' => {
                    if let Some(ch) = hex(i + 2, 2) {
                        out.push(ch);
                        i += 4;
                        continue;
                    }
                }
                _ => {}
            }
        }
        if matches!(c, '\'' | '"' | '`') {
            // A closing quote followed by `+` and an opening quote: join.
            let mut j = i + 1;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            if chars.get(j) == Some(&'+') {
                let mut k = j + 1;
                while k < chars.len() && chars[k].is_whitespace() {
                    k += 1;
                }
                if matches!(chars.get(k), Some('\'' | '"' | '`')) {
                    i = k + 1;
                    continue;
                }
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

/// Whether a program (Executor's `execute`, goose's Code Mode) may write to
/// Slack: it names a Slack tool that writes, or reaches tools by a computed
/// name (`tools.x[name]`, `globalThis[name]`, `eval`, `Function`), which a
/// text search cannot follow.
fn slack_write_in_code(code: &str) -> bool {
    let code = normalize_program(code);
    if code.contains("eval(") || code.contains("Function(") {
        return true;
    }
    let lower = code.to_ascii_lowercase();
    let compact: String = lower.chars().filter(|c| !c.is_whitespace()).collect();
    for dynamic in ["globalthis[", "this[", "self["] {
        if compact.contains(dynamic) {
            return true;
        }
    }
    // tools[...] or tools.app[...] / tools.app.slack[...]
    let mut rest = compact.as_str();
    while let Some(i) = rest.find("tools") {
        let after: String = rest[i + 5..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '$' | '-' | '?'))
            .collect();
        if rest[i + 5 + after.len()..].starts_with('[') {
            return true;
        }
        rest = &rest[i + 5..];
    }
    let token_at = |i: usize| -> String {
        lower[i..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
            .collect()
    };
    let mut from = 0;
    while let Some(i) = lower[from..].find("slack").map(|i| i + from) {
        let token = token_at(i);
        let last = token.trim_end_matches('.').rsplit('.').next().unwrap_or("");
        if is_write_tool(last) {
            return true;
        }
        from = i + 5;
    }
    SLACK_TOOLS_WITHOUT_SLACK_IN_NAME
        .iter()
        .any(|t| lower.contains(t) && is_write_tool(t))
}

/// Build what the human sees from the call's arguments.
fn proposal(server: Option<String>, tool: String, input: &Value) -> Proposal {
    let args = match input {
        Value::Object(map) => map.clone(),
        Value::Null => Map::new(),
        other => {
            let mut m = Map::new();
            m.insert("arguments".into(), other.clone());
            m
        }
    };
    let present = |keys: &[&'static str]| -> Vec<&'static str> {
        keys.iter()
            .copied()
            .filter(|k| match args.get(*k) {
                Some(Value::String(s)) => !s.trim().is_empty(),
                Some(Value::Null) | None => false,
                Some(_) => true,
            })
            .collect()
    };
    let destinations = present(&DESTINATION_KEYS);
    let texts = present(&TEXT_KEYS);
    let threads = present(&THREAD_KEYS);
    let rich = RICH_KEYS
        .iter()
        .any(|k| args.get(*k).is_some_and(|v| !v.is_null() && v != "" && v != &json!([])));
    let shown = |v: &Value| match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let destination_known = !destinations.is_empty();
    // Several destination-like arguments: which one the server uses is
    // unknown, so all of them are named.
    let destination = if destination_known {
        destinations
            .iter()
            .map(|k| shown(&args[*k]))
            .collect::<Vec<_>>()
            .join(", ")
    } else {
        "(not given in the call)".to_string()
    };
    // The text is shown as the exact message only when it is unambiguous:
    // one text argument, one destination, at most one thread, and no rich
    // content Slack would render instead. Otherwise every argument is
    // shown, so a decoy `message` cannot hide the `text` that is sent.
    let plain = texts.len() == 1
        && destinations.len() <= 1
        && threads.len() <= 1
        && !rich
        && matches!(args.get(texts[0]), Some(Value::String(_)));
    let mut rest = args.clone();
    if !plain {
        let all = Value::Object(args);
        return Proposal {
            server,
            tool,
            destination,
            destination_known,
            message: serde_json::to_string_pretty(&all).unwrap_or_else(|_| all.to_string()),
            thread_ts: None,
            other_arguments: Map::new(),
            has_text: false,
        };
    }
    for k in destinations.iter().chain(texts.iter()) {
        rest.remove(*k);
    }
    let thread_ts = threads.first().and_then(|k| match args.get(*k) {
        Some(Value::String(ts)) if is_slack_ts(ts) => {
            rest.remove(*k);
            Some(ts.clone())
        }
        _ => None,
    });
    Proposal {
        server,
        tool,
        destination,
        destination_known,
        message: shown(&args[texts[0]]),
        thread_ts,
        other_arguments: rest,
        has_text: true,
    }
}

/// The approval request for a proposal.
pub fn incoming_request(agent: Agent, call: &ToolCall, p: &Proposal) -> IncomingRequest {
    let server = p
        .server
        .as_deref()
        .map(|s| format!(" on the Slack MCP server `{s}`"))
        .unwrap_or_default();
    let mut reason = format!(
        "{} wants to run its own Slack tool `{}`{server}. Approving lets this exact call run; \
         SpeakEasy itself sends nothing.",
        agent.display_name(),
        p.tool
    );
    if !p.has_text {
        reason.push_str(
            " The call has no plain message text (or carries rich content such as blocks), so \
             all of its arguments are shown as the message.",
        );
    }
    if !p.other_arguments.is_empty() {
        reason.push_str(" Other arguments: ");
        reason.push_str(&Value::Object(p.other_arguments.clone()).to_string());
    }
    let place = call
        .cwd
        .as_deref()
        .map(|c| format!(" in {c}"))
        .unwrap_or_default();
    IncomingRequest {
        message: p.message.clone(),
        destination: p.destination.clone(),
        thread_ts: p.thread_ts.clone(),
        reason,
        agent: agent.display_name().to_string(),
        conversation: ConversationContext {
            title: format!("{} session{place}", agent.display_name()),
            id_or_link: call
                .session
                .clone()
                .unwrap_or_else(|| "session not given".to_string()),
            summary: format!(
                "The agent called `{}` directly; SpeakEasy's hook stopped the call until you \
                 decide.",
                call.name
            ),
        },
    }
}

/// Text appended to every block: the model must not route around it.
const DO_NOT_ROUTE_AROUND: &str = "Do not try another route to Slack (other tools, the shell, \
    scripts, the browser or the desktop app).";

/// The model-facing reason for a decision from the instance.
pub fn outcome_for(result: &Value) -> Outcome {
    let secs = |key: &str| result.get(key).and_then(Value::as_u64).unwrap_or(0);
    let minutes = |s: u64| s.div_ceil(60).max(1);
    match result.get("decision").and_then(Value::as_str) {
        Some("approved") => Outcome::Approved,
        Some("denied") if secs("cooldown_remaining_secs") == 0 => Outcome::Deny(format!(
            "The user denied this Slack call in SpeakEasy; nothing was sent. Do not retry it. \
             {DO_NOT_ROUTE_AROUND}"
        )),
        Some("denied") => Outcome::Deny(format!(
            "The user denied this Slack message in SpeakEasy; nothing was sent. Do not retry it: \
             messages with the same purpose to this destination are blocked for {} minutes. {}",
            minutes(secs("cooldown_remaining_secs")),
            DO_NOT_ROUTE_AROUND
        )),
        Some("feedback") => Outcome::Deny(format!(
            "The user did not approve this Slack message and gave feedback instead: \"{}\". \
             Nothing was sent. Revise the call using the feedback and call the tool again; the \
             user will be asked again.",
            result.get("feedback").and_then(Value::as_str).unwrap_or("")
        )),
        Some("cooldown_blocked") => Outcome::Deny(format!(
            "The user recently denied a similar Slack message to this destination, so this call \
             was blocked without asking; nothing was sent. Do not retry for {} minutes. {}",
            minutes(secs("cooldown_remaining_secs")),
            DO_NOT_ROUTE_AROUND
        )),
        Some("already_sent") => Outcome::Deny(format!(
            "SpeakEasy already sent this exact message to this destination (Slack ts {}, {}), \
             so it was not sent again.",
            result.get("ts").and_then(Value::as_str).unwrap_or("?"),
            result.get("permalink").and_then(Value::as_str).unwrap_or("no permalink")
        )),
        Some("error") => blocked(&format!(
            "SpeakEasy could not get the user's approval: {}",
            result.get("error").and_then(Value::as_str).unwrap_or("unknown error")
        )),
        _ => blocked("SpeakEasy returned an answer this hook does not understand"),
    }
}

/// A block caused by the hook itself (not by the human).
pub fn blocked(why: &str) -> Outcome {
    Outcome::Deny(format!(
        "{why}. This Slack call was blocked; nothing was sent. {DO_NOT_ROUTE_AROUND} Tell the \
         user."
    ))
}

/// The answer in the agent's hook format. Every format was checked against
/// the agent's own hook runner; see docs/SPEC.md for the table.
pub fn render(agent: Agent, outcome: &Outcome) -> Rendered {
    let out = |stdout: String, exit_code: i32| Rendered {
        stdout,
        stderr: String::new(),
        exit_code,
    };
    let approved_note = "The user approved this exact Slack call in SpeakEasy.";
    match (agent, outcome) {
        // Cursor treats empty output as a hook failure (with failClosed).
        (Agent::Cursor, Outcome::Neutral) => out(line(json!({})), 0),
        (Agent::Cursor, Outcome::Approved) => out(line(json!({ "permission": "allow" })), 0),
        (Agent::Cursor, Outcome::Deny(reason)) => out(
            line(json!({ "permission": "deny", "user_message": reason, "agent_message": reason })),
            0,
        ),
        (_, Outcome::Neutral) => out(String::new(), 0),

        // Claude Code (and pdev): `allow` skips the second, built-in prompt.
        (Agent::Claude, Outcome::Approved) => out(
            line(json!({ "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "allow",
                "permissionDecisionReason": approved_note,
            }})),
            0,
        ),
        // Codex treats a bare `allow` as a hook failure: stay neutral.
        (Agent::Codex, Outcome::Approved) => out(String::new(), 0),
        (Agent::Claude | Agent::Codex, Outcome::Deny(reason)) => Rendered {
            stdout: line(json!({ "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": reason,
            }})),
            stderr: format!("{reason}\n"),
            exit_code: 2,
        },

        // Gemini ignores Claude-style output; its deny is top-level.
        (Agent::Gemini, Outcome::Approved) => out(String::new(), 0),
        (Agent::Gemini, Outcome::Deny(reason)) => {
            out(line(json!({ "decision": "deny", "reason": reason })), 0)
        }

        // Copilot: flat permissionDecision; `allow` skips its own prompt.
        (Agent::Copilot, Outcome::Approved) => out(
            line(json!({ "permissionDecision": "allow", "permissionDecisionReason": approved_note })),
            0,
        ),
        (Agent::Copilot, Outcome::Deny(reason)) => out(
            line(json!({ "permissionDecision": "deny", "permissionDecisionReason": reason })),
            0,
        ),

        // Vibe: deny is JSON on exit 0 (`strict = true` makes any failure a
        // deny).
        (Agent::Vibe, Outcome::Approved) => out(String::new(), 0),
        (Agent::Vibe, Outcome::Deny(reason)) => {
            out(line(json!({ "decision": "deny", "reason": reason })), 0)
        }

        // goose hooks and SpeakEasy's own plugins (OpenCode, pi, Hermes, Amp).
        (
            Agent::Goose | Agent::OpenCode | Agent::Pi | Agent::Hermes | Agent::Amp,
            Outcome::Approved,
        ) => out(String::new(), 0),
        (
            Agent::Goose | Agent::OpenCode | Agent::Pi | Agent::Hermes | Agent::Amp,
            Outcome::Deny(reason),
        ) => Rendered {
            stdout: line(json!({ "decision": "block", "reason": reason })),
            stderr: format!("{reason}\n"),
            exit_code: 2,
        },
    }
}

fn line(value: Value) -> String {
    format!("{value}\n")
}

/// Serializes the single answer: whichever of the main path and the
/// watchdog answers first wins, and the process exits while holding it.
static ANSWER: Mutex<bool> = Mutex::new(false);

/// The exit code that means "proceed" (`--ok-code`, default 0).
static OK_CODE: OnceLock<i32> = OnceLock::new();

fn exit_with(code: i32) -> ! {
    std::process::exit(if code == 0 { *OK_CODE.get().unwrap_or(&0) } else { code })
}

fn answer(agent: Agent, outcome: &Outcome) -> ! {
    let mut answered = ANSWER.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if *answered {
        // Never reached: the first answer exits while holding the lock.
        std::process::exit(2);
    }
    *answered = true;
    let r = render(agent, outcome);
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(r.stdout.as_bytes());
    let _ = out.flush();
    let _ = std::io::stderr().write_all(r.stderr.as_bytes());
    exit_with(r.exit_code)
}

/// Ask the running instance; `Err` is a reason to block.
fn ask_instance(
    state_dir: &Path,
    request: &IncomingRequest,
    share_key: Option<&str>,
    cooldown: bool,
    wait_secs: u64,
) -> Result<Value, String> {
    let Some(stream) = instance::connect_existing(state_dir) else {
        return Err("SpeakEasy, the app that asks the user to approve Slack messages, is not \
                    running (the user can start it with: launchctl kickstart -k \
                    gui/$(id -u)/com.speakeasy.slack-approval-gateway)"
            .into());
    };
    let mut params = serde_json::to_value(request).map_err(|e| e.to_string())?;
    if request.thread_ts.is_none() {
        if let Some(m) = params.as_object_mut() {
            m.remove("thread_ts");
        }
    }
    params["wait_secs"] = json!(wait_secs
        .saturating_sub(INSTANCE_DEADLINE_MARGIN_SECS)
        .max(1));
    if let Some(share_key) = share_key {
        params["share_key"] = json!(share_key);
    }
    params["cooldown"] = json!(cooldown);
    let call = json!({ "jsonrpc": "2.0", "id": 1, "method": APPROVE_METHOD, "params": params });
    let mut writer = stream
        .try_clone()
        .map_err(|e| format!("cannot talk to SpeakEasy: {e}"))?;
    writer
        .write_all(format!("{call}\n").as_bytes())
        .map_err(|e| format!("cannot talk to SpeakEasy: {e}"))?;
    let mut reader = BufReader::new(stream);
    let mut buf = String::new();
    loop {
        buf.clear();
        match reader.read_line(&mut buf) {
            Ok(0) => return Err("SpeakEasy closed the connection without a decision".into()),
            Ok(_) => {}
            Err(e) => return Err(format!("lost the connection to SpeakEasy: {e}")),
        }
        let Ok(msg) = serde_json::from_str::<Value>(&buf) else {
            continue;
        };
        if msg.get("id") != Some(&json!(1)) {
            continue;
        }
        if let Some(result) = msg.get("result") {
            return Ok(result.clone());
        }
        let detail = msg
            .pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or("unknown error");
        if msg.pointer("/error/code").and_then(Value::as_i64) == Some(-32601) {
            return Err(
                "the running SpeakEasy is too old for hooks; ask the user to restart it".into(),
            );
        }
        return Err(format!("SpeakEasy rejected the request: {detail}"));
    }
}

/// The matcher the installer registers for Claude Code's PreToolUse hook
/// (`CLAUDE_MATCHER` in scripts/agent_config.py; a test keeps them equal).
pub const CLAUDE_MATCHER: &str =
    "mcp__.*([Ss]lack|SLACK|[Ee]xecutor|conversations_|reactions_|usergroups_|saved_).*";

/// Whether Claude Code settings still run SpeakEasy's PreToolUse hook as
/// the installer registered it: the exact matcher, the fail-closed wrapper
/// around `--hook claude --wait <wait>`, and a timeout that outlasts it.
fn runs_speakeasy_hook(settings: &Value, wait_secs: u64) -> bool {
    let needle = format!("--hook claude --wait {wait_secs} ");
    let Some(groups) = settings.pointer("/hooks/PreToolUse").and_then(Value::as_array) else {
        return false;
    };
    groups.iter().any(|g| {
        g.get("matcher").and_then(Value::as_str) == Some(CLAUDE_MATCHER)
            && g.get("hooks").and_then(Value::as_array).is_some_and(|hs| {
                hs.iter().any(|h| {
                    let command = h.get("command").and_then(Value::as_str).unwrap_or("");
                    let timeout = h.get("timeout").and_then(Value::as_f64).unwrap_or(0.0);
                    command.starts_with("[ -x ")
                        && command.contains(&needle)
                        && timeout >= (wait_secs + HOOK_TIMEOUT_MARGIN_SECS) as f64
                })
            })
    })
}

/// Claude Code `ConfigChange` (`--event config-change`): block a live
/// settings change that would switch SpeakEasy's hook off for the running
/// session — `disableAllHooks`, `CLAUDE_CODE_SIMPLE`, or user settings that
/// no longer run it as installed (matcher, wrapper, timeout). A project's
/// `.claude/settings.json` can set `disableAllHooks` (the model can write
/// one in bypass mode), and Claude Code applies such a change without a
/// restart unless a hook blocks it.
pub fn config_change_outcome(raw: &str, wait_secs: u64) -> Outcome {
    let Ok(payload) = serde_json::from_str::<Value>(raw) else {
        return Outcome::Neutral;
    };
    let path = payload.get("file_path").and_then(Value::as_str).unwrap_or("");
    let source = payload.get("source").and_then(Value::as_str).unwrap_or("");
    if path.is_empty() {
        return Outcome::Neutral;
    }
    let mut problems = Vec::new();
    match std::fs::read_to_string(path) {
        Err(_) if source == "user_settings" => {
            problems.push("deletes the user settings that run it".to_string())
        }
        Err(_) => {}
        Ok(text) => match serde_json::from_str::<Value>(&text) {
            Err(_) if source == "user_settings" => problems
                .push("makes the user settings invalid JSON, which drops every hook".to_string()),
            Err(_) => {}
            Ok(settings) => {
                if settings.get("disableAllHooks") == Some(&Value::Bool(true)) {
                    problems.push("sets disableAllHooks".to_string());
                }
                if settings.pointer("/env/CLAUDE_CODE_SIMPLE").is_some() {
                    problems.push("sets CLAUDE_CODE_SIMPLE".to_string());
                }
                if source == "user_settings" && !runs_speakeasy_hook(&settings, wait_secs) {
                    problems.push("removes or changes SpeakEasy's PreToolUse hook".to_string());
                }
            }
        },
    }
    if problems.is_empty() {
        return Outcome::Neutral;
    }
    Outcome::Deny(format!(
        "SpeakEasy blocked this settings change because it {} in {path}, which would let \
         Slack messages go out without the user's approval. It was not applied to this \
         session. Do not try to switch hooks off; if the user made this change themselves, \
         they can restart Claude Code.",
        problems.join(" and ")
    ))
}

fn run_config_change(wait_secs: u64) -> ! {
    let mut raw = String::new();
    let _ = std::io::stdin()
        .take(MAX_PAYLOAD_BYTES)
        .read_to_string(&mut raw);
    match config_change_outcome(&raw, wait_secs) {
        Outcome::Deny(reason) => {
            let _ = std::io::stdout()
                .write_all(line(json!({ "decision": "block", "reason": reason })).as_bytes());
            let _ = std::io::stderr().write_all(format!("{reason}\n").as_bytes());
            exit_with(2)
        }
        _ => exit_with(0),
    }
}

/// Identifies one agent tool call, so hooks that stop the same call (Cursor
/// runs its own and the Claude Code hooks it imports) share one decision:
/// the agent, session, tool, every argument and the agent's call ID. `None`
/// without a call ID.
fn share_key(agent: Agent, call: &ToolCall) -> Option<String> {
    let call_id = call.call_id.as_deref()?;
    let (_, tool) = split_tool_name(&call.name);
    let tool = call.tool.clone().unwrap_or(tool);
    let identity = format!(
        "{}\n{}\n{}\n{}",
        agent.display_name(),
        call.session.as_deref().unwrap_or(""),
        tool,
        call.input
    );
    Some(crate::core::message_hash(&identity, call_id, None))
}

/// Hook mode entry point: never returns.
pub fn run(agent: Agent, wait_secs: u64, state_dir: &Path) -> ! {
    // Answer before the agent's hook timeout, whatever happens below.
    let watchdog_after = Duration::from_secs(wait_secs);
    std::thread::spawn(move || {
        std::thread::sleep(watchdog_after);
        answer(
            agent,
            &blocked("No one answered SpeakEasy's approval window in time"),
        );
    });
    let outcome = std::panic::catch_unwind(|| decide(agent, wait_secs, state_dir))
        .unwrap_or_else(|_| blocked("SpeakEasy's hook failed"));
    answer(agent, &outcome)
}

fn decide(agent: Agent, wait_secs: u64, state_dir: &Path) -> Outcome {
    let mut raw = Vec::new();
    let read = std::io::stdin()
        .take(MAX_PAYLOAD_BYTES + 1)
        .read_to_end(&mut raw);
    let text = String::from_utf8_lossy(&raw);
    // A payload this hook cannot read is blocked only when it may be a
    // Slack call; anything else keeps the agent's normal flow.
    let unreadable = |why: String| {
        if text.to_ascii_lowercase().contains("slack") {
            blocked(&format!("SpeakEasy's hook could not read the tool call: {why}"))
        } else {
            Outcome::Neutral
        }
    };
    if let Err(e) = read {
        return unreadable(e.to_string());
    }
    if raw.len() as u64 > MAX_PAYLOAD_BYTES {
        return unreadable("it is too large".into());
    }
    let call = match parse_payload(&text) {
        Ok(call) => call,
        Err(problem) => return unreadable(problem),
    };
    let proposal = match classify_for(agent, &call) {
        Classification::Pass => return Outcome::Neutral,
        Classification::Ask(p) => p,
    };
    let request = incoming_request(agent, &call, &proposal);
    let share_key = share_key(agent, &call);
    match ask_instance(
        state_dir,
        &request,
        share_key.as_deref(),
        proposal.destination_known,
        wait_secs,
    ) {
        Ok(result) => outcome_for(&result),
        Err(why) => blocked(&why),
    }
}

/// Run hook mode from the command line (`--hook <agent> [--wait SECS]
/// [--event config-change]`).
/// Hook mode's command line: `--hook <agent> [--wait SECS]
/// [--state-dir DIR] [--ok-code N] [--event config-change]`.
#[derive(Debug, Clone, PartialEq)]
pub struct HookArgs {
    pub agent: Agent,
    pub wait_secs: u64,
    /// The instance's state dir, pinned by the installer: a project's
    /// settings can set `SLACK_GATEWAY_STATE_DIR` or `HOME` for the agent,
    /// which must not point the hook at another socket.
    pub state_dir: Option<PathBuf>,
    /// Exit with this instead of 0 when the call may proceed. The
    /// installer's wrapper turns it back into 0 and treats any other exit —
    /// 0 from an older binary that has no hook mode, for one — as a block.
    pub ok_code: i32,
    /// `--event config-change`: Claude Code's ConfigChange guard.
    pub config_change: bool,
}

/// Run hook mode from the command line: never returns.
pub fn main(args: HookArgs, default_state_dir: &Path) -> ! {
    let _ = OK_CODE.set(args.ok_code);
    if args.config_change && args.agent == Agent::Claude {
        run_config_change(args.wait_secs);
    }
    let state_dir = args.state_dir.unwrap_or_else(|| default_state_dir.to_path_buf());
    run(args.agent, args.wait_secs, &state_dir)
}

/// Parse hook mode's command line. `None` when `--hook` is absent.
pub fn parse_args(args: &[String]) -> Option<Result<HookArgs, String>> {
    let pos = args.iter().position(|a| a == "--hook")?;
    let value = |flag: &str| args.iter().position(|a| a == flag).map(|i| args.get(i + 1));
    let Some(name) = args.get(pos + 1) else {
        return Some(Err("--hook needs an agent name".into()));
    };
    let Some(agent) = Agent::parse(name) else {
        return Some(Err(format!("unknown agent '{name}' for --hook")));
    };
    let wait_secs = match value("--wait") {
        None => DEFAULT_WAIT_SECS,
        Some(w) => match w.and_then(|w| w.parse::<u64>().ok()) {
            Some(w) if (10..=crate::mcp::MAX_APPROVAL_WAIT_SECS).contains(&w) => w,
            _ => return Some(Err("--wait needs a number of seconds from 10 to 86400".into())),
        },
    };
    let state_dir = match value("--state-dir") {
        None => None,
        Some(Some(d)) if Path::new(d).is_absolute() => Some(PathBuf::from(d)),
        Some(_) => return Some(Err("--state-dir needs an absolute path".into())),
    };
    let ok_code = match value("--ok-code") {
        None => 0,
        Some(c) => match c.and_then(|c| c.parse::<i32>().ok()) {
            Some(c) if (3..=125).contains(&c) => c,
            _ => return Some(Err("--ok-code needs a number from 3 to 125".into())),
        },
    };
    let config_change = match value("--event") {
        None => false,
        Some(Some(e)) if e == "config-change" => true,
        Some(_) => return Some(Err("--event only accepts config-change".into())),
    };
    Some(Ok(HookArgs {
        agent,
        wait_secs,
        state_dir,
        ok_code,
        config_change,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(name: &str, input: Value) -> ToolCall {
        ToolCall {
            name: name.into(),
            input,
            server: None,
            tool: None,
            session: Some("s-1".into()),
            cwd: Some("/work/repo".into()),
            call_id: None,
        }
    }

    fn ask(name: &str, input: Value) -> Proposal {
        match classify(&call(name, input)) {
            Classification::Ask(p) => p,
            Classification::Pass => panic!("{name} should be gated"),
        }
    }

    fn passes(name: &str) -> bool {
        classify(&call(name, json!({"channel_id": "C1", "message": "hi"}))) == Classification::Pass
    }

    #[test]
    fn splits_every_agents_mcp_tool_names() {
        let cases = [
            ("mcp__plugin_slack_slack__slack_send_message", Some("plugin_slack_slack"), "slack_send_message"),
            ("mcp__slack__conversations_add_message", Some("slack"), "conversations_add_message"),
            ("mcp_slack.slack_send_message", Some("slack"), "slack_send_message"),
            ("connector_slack.slack_send_message", Some("slack"), "slack_send_message"),
            ("slack__slack_send_message", Some("slack"), "slack_send_message"),
            ("MCP:slack_send_message", None, "slack_send_message"),
            ("mcp_slack_slack_send_message", None, "mcp_slack_slack_send_message"),
            ("Bash", None, "Bash"),
        ];
        for (name, server, tool) in cases {
            let (s, t) = split_tool_name(name);
            assert_eq!((s.as_deref(), t.as_str()), (server, tool), "{name}");
        }
    }

    #[test]
    fn gates_slack_writes_and_passes_reads_and_other_tools() {
        for name in [
            "mcp__plugin_slack_slack__slack_send_message",
            "mcp__plugin_slack_slack__slack_schedule_message",
            "mcp__plugin_slack_slack__slack_create_canvas",
            "mcp__slack__conversations_add_message",
            "mcp__workspace__conversations_add_message",
            "mcp__workspace__reactions_add",
            "mcp_slack.slack_send_message",
            "connector_slack.slack_send_message",
            "slack__slack_post_message",
            "mcp__slack__slack_reply_to_thread",
            "mcp__slack__slack_frobnicate",
            "MCP:slack_send_message",
            "MCP:conversations_add_message",
            "mcp_slack_slack_send_message",
            "mcp_ws_conversations_add_message",
            "slack-slack_send_message",
            "ws-conversations_add_message",
        ] {
            assert!(!passes(name), "{name} should be gated");
        }
        for name in [
            "Bash",
            "Edit",
            "mcp__github__create_issue",
            "mcp__plugin_slack_slack__slack_read_channel",
            "mcp__plugin_slack_slack__slack_search_public",
            "mcp__plugin_slack_slack__slack_read_user_profile",
            "mcp__plugin_slack_slack__slack_send_message_draft",
            "mcp__plugin_slack_slack__authenticate",
            "mcp__slack__conversations_history",
            "mcp__slack__channels_list",
            "mcp__slack__conversations_mark",
            "mcp__slack__slack_list_messages",
            "mcp__slack-approval-gateway__request_slack_message",
            "slack-approval-gateway__request_slack_message",
            "mcp_slack-approval-gateway.request_slack_message",
            "mcp_slack-approval-gateway_request_slack_message",
            "slack-approval-gateway-request_slack_message",
            "MCP:slack_read_channel",
            "mcp_slack_slack_search_public",
            "slack-conversations_history",
        ] {
            assert!(passes(name), "{name} should pass");
        }
    }

    #[test]
    fn server_named_in_the_payload_counts() {
        let mut c = call("send_message", json!({"channel": "C1", "text": "hi"}));
        assert_eq!(classify(&c), Classification::Pass);
        c.server = Some("slack".into());
        assert!(matches!(classify(&c), Classification::Ask(_)));
    }

    #[test]
    fn extracts_destination_text_and_thread_verbatim() {
        let p = ask(
            "mcp__plugin_slack_slack__slack_send_message",
            json!({"channel_id": "C0123", "message": "  Deploy *done*\n", "thread_ts": "1700000000.000100"}),
        );
        assert_eq!(p.destination, "C0123");
        assert_eq!(p.message, "  Deploy *done*\n");
        assert_eq!(p.thread_ts.as_deref(), Some("1700000000.000100"));
        assert!(p.has_text);
        assert!(p.other_arguments.is_empty());

        let p = ask(
            "mcp__slack__conversations_add_message",
            json!({"channel_id": "#eng", "payload": "hi", "content_type": "text/markdown"}),
        );
        assert_eq!((p.destination.as_str(), p.message.as_str()), ("#eng", "hi"));
        assert_eq!(p.other_arguments.get("content_type"), Some(&json!("text/markdown")));
    }

    #[test]
    fn a_malformed_thread_stays_with_the_other_arguments() {
        let p = ask(
            "mcp__slack__slack_send_message",
            json!({"channel_id": "C1", "message": "x", "thread_ts": "https://x.slack.com/p1"}),
        );
        assert_eq!(p.thread_ts, None);
        assert_eq!(p.other_arguments.get("thread_ts"), Some(&json!("https://x.slack.com/p1")));
    }

    #[test]
    fn a_call_without_text_shows_every_argument() {
        let p = ask(
            "mcp__slack__reactions_add",
            json!({"channel_id": "C1", "timestamp": "1.2", "emoji": "tada"}),
        );
        assert!(!p.has_text);
        assert!(p.message.contains("\"emoji\": \"tada\""), "{}", p.message);
        assert!(p.message.contains("\"channel_id\": \"C1\""));
        assert!(p.other_arguments.is_empty());
        assert_eq!(p.destination, "C1");

        let p = ask("mcp__slack__slack_create_canvas", json!({"title": "Plan"}));
        assert_eq!(p.destination, "(not given in the call)");
    }

    #[test]
    fn other_arguments_reach_the_reason() {
        let c = call(
            "mcp__plugin_slack_slack__slack_send_message",
            json!({"channel_id": "C1", "message": "hi", "reply_broadcast": true}),
        );
        let Classification::Ask(p) = classify(&c) else { panic!() };
        let req = incoming_request(Agent::Claude, &c, &p);
        assert!(req.reason.contains("\"reply_broadcast\":true"), "{}", req.reason);
        assert!(req.reason.contains("`slack_send_message`"));
        assert_eq!(req.agent, "Claude Code");
        assert_eq!(req.conversation.id_or_link, "s-1");
        assert!(req.conversation.title.contains("/work/repo"));
    }

    #[test]
    fn reads_the_payload_spellings_of_every_agent() {
        let c = parse_payload(r#"{"tool_name":"x","tool_input":{"a":1},"session_id":"s","cwd":"/w"}"#).unwrap();
        assert_eq!((c.name.as_str(), c.input.clone()), ("x", json!({"a": 1})));
        assert_eq!((c.session.as_deref(), c.cwd.as_deref()), (Some("s"), Some("/w")));
        let c = parse_payload(r#"{"toolName":"x","toolArgs":"{\"a\":2}"}"#).unwrap();
        assert_eq!(c.input, json!({"a": 2}));
        let c = parse_payload(r#"{"tool_name":"x","tool_input":{},"mcp_server":{"name":"slack"},"tool_use_id":"tu1"}"#).unwrap();
        assert_eq!((c.server.as_deref(), c.call_id.as_deref()), (Some("slack"), Some("tu1")));
        // Gemini names the server and tool separately.
        let c = parse_payload(r#"{"tool_name":"mcp_ws_post","tool_input":{},"mcp_context":{"server_name":"slack","tool_name":"post"}}"#).unwrap();
        assert_eq!((c.server.as_deref(), c.tool.as_deref()), (Some("slack"), Some("post")));
        // Cursor: the input is a JSON string, the server a separate field.
        let c = parse_payload(r#"{"tool_name":"slack_send_message","tool_input":"{\"channel_id\":\"C1\"}","mcp_server_name":"slack","workspace_roots":["/w"]}"#).unwrap();
        assert_eq!(c.input, json!({"channel_id": "C1"}));
        assert_eq!((c.server.as_deref(), c.cwd.as_deref()), (Some("slack"), Some("/w")));
        assert!(parse_payload("[]").is_err());
        assert!(parse_payload(r#"{"tool_input":{}}"#).is_err());
    }

    #[test]
    fn decisions_map_to_outcomes() {
        assert_eq!(outcome_for(&json!({"decision": "approved"})), Outcome::Approved);
        for result in [
            json!({"decision": "denied", "cooldown_remaining_secs": 600}),
            json!({"decision": "feedback", "feedback": "say please"}),
            json!({"decision": "cooldown_blocked", "cooldown_remaining_secs": 61}),
            json!({"decision": "already_sent", "ts": "1.2", "permalink": "p"}),
            json!({"decision": "error", "error": "boom"}),
            json!({"decision": "maybe"}),
            json!({}),
        ] {
            assert!(matches!(outcome_for(&result), Outcome::Deny(_)), "{result}");
        }
        let Outcome::Deny(r) = outcome_for(&json!({"decision": "feedback", "feedback": "say please"})) else {
            panic!()
        };
        assert!(r.contains("say please"));
        let Outcome::Deny(r) = outcome_for(&json!({"decision": "cooldown_blocked", "cooldown_remaining_secs": 61})) else {
            panic!()
        };
        assert!(r.contains("2 minutes"), "{r}");
    }

    #[test]
    fn executor_calls_to_slack_connectors_are_gated() {
        let one = json!({"calls": {"path": "tools.mistral.slack.slack_send_message",
            "input": {"channel_id": "C1", "message": "via executor"}}});
        let p = ask("mcp__executor__call", one);
        assert_eq!((p.destination.as_str(), p.message.as_str()), ("C1", "via executor"));
        assert_eq!(p.tool, "slack_send_message");

        let read = json!({"calls": [{"path": "tools.mistral.slack.slack_read_channel", "input": {}}]});
        assert_eq!(classify(&call("mcp__executor-v2__call", read)), Classification::Pass);
        let other = json!({"calls": [{"path": "tools.github.issues.create", "input": {}}]});
        assert_eq!(classify(&call("mcp__executor__call", other)), Classification::Pass);

        let code = "const r = await tools.mistral.slack.slack_send_message({channel_id:'C1', message:'x'}); return r";
        let p = ask("mcp__executor__execute", json!({"code": code}));
        assert_eq!(p.message, code);
        for dynamic in ["return tools.mistral.slack[name](args)", "tools.mistral.slack.slack_add_reaction({})"] {
            assert!(matches!(classify(&call("mcp__executor__execute", json!({"code": dynamic}))), Classification::Ask(_)), "{dynamic}");
        }
        let read = "return await tools.mistral.slack.slack_read_channel({channel_id:'C1'})";
        assert_eq!(classify(&call("mcp__executor__execute", json!({"code": read}))), Classification::Pass);
        assert_eq!(classify(&call("mcp__executor__search", json!({"query": "slack send"}))), Classification::Pass);
    }

    #[test]
    fn goose_code_mode_programs_are_checked() {
        let send = call("execute_typescript", json!({"code": "await slack__slack_send_message({channel_id:'C1', message:'x'})"}));
        assert!(matches!(classify_for(Agent::Goose, &send), Classification::Ask(_)));
        // Other agents hook the nested calls themselves.
        assert_eq!(classify_for(Agent::Claude, &send), Classification::Pass);
        let read = call("execute_typescript", json!({"code": "return await slack__slack_read_channel({channel_id:'C1'})"}));
        assert_eq!(classify_for(Agent::Goose, &read), Classification::Pass);
        let plain = call("execute_typescript", json!({"code": "return 1 + 1"}));
        assert_eq!(classify_for(Agent::Goose, &plain), Classification::Pass);
    }

    #[test]
    fn opencode_names_the_server_and_tool_separately() {
        let c = parse_payload(r#"{"tool_name":"ws_post_message","tool_input":{"channel":"C1","text":"x"},"mcp_server":{"name":"slack"},"mcp_tool":"post_message"}"#).unwrap();
        let Classification::Ask(p) = classify(&c) else { panic!("should be gated") };
        assert_eq!(p.tool, "post_message");
    }

    #[test]
    fn rich_content_shows_the_whole_call() {
        let p = ask(
            "mcp__slack__conversations_add_message",
            json!({"channel_id": "C1", "text": "fallback", "blocks": "[{\"type\":\"section\"}]"}),
        );
        assert!(!p.has_text);
        assert!(p.message.contains("blocks") && p.message.contains("fallback"), "{}", p.message);
    }

    #[test]
    fn catalog_tools_are_classified() {
        for name in [
            "mcp__slack__slack_add_reaction",
            "mcp__slack__slack_create_conversation",
            "mcp__slack__slack_update_canvas",
            "mcp__slack__slack_complete_file_upload",
            "mcp__slack__slack_add_list_record",
            "mcp__ws__conversations_join",
            "mcp__ws__usergroups_users_update",
            "mcp__ws__usergroups_me",
            "connector_slack_slack_send_message",
            "connector_slack_slack_schedule_message",
        ] {
            assert!(!passes(name), "{name} should be gated");
        }
        for name in [
            "mcp__slack__slack_get_file_upload_url",
            "mcp__slack__slack_read_list",
            "mcp__slack__slack_get_reactions",
            "mcp__slack__slack_list_channel_members",
            "mcp__ws__saved_update",
            "connector_slack_slack_read_channel",
            "connector_slack_slack_search_public",
        ] {
            assert!(passes(name), "{name} should pass");
        }
    }

    #[test]
    fn every_agent_gets_its_own_answer_format() {
        let deny = Outcome::Deny("no".into());
        for agent in [
            Agent::Claude, Agent::Codex, Agent::Gemini, Agent::Copilot, Agent::Cursor,
            Agent::Vibe, Agent::Goose, Agent::OpenCode, Agent::Pi, Agent::Hermes, Agent::Amp,
        ] {
            let n = render(agent, &Outcome::Neutral);
            assert_eq!(n.exit_code, 0, "{agent:?}");
            assert!(n.stdout.is_empty() || n.stdout == "{}\n", "{agent:?} neutral: {:?}", n.stdout);
            let d = render(agent, &deny);
            assert!(d.stdout.contains("\"no\""), "{agent:?} deny carries the reason: {:?}", d.stdout);
            let a = render(agent, &Outcome::Approved);
            assert_eq!(a.exit_code, 0, "{agent:?}");
            assert!(!a.stdout.contains("deny"), "{agent:?}");
        }
        let j = |r: Rendered| serde_json::from_str::<Value>(r.stdout.trim()).unwrap();
        assert_eq!(j(render(Agent::Claude, &deny))["hookSpecificOutput"]["permissionDecision"], "deny");
        assert_eq!(render(Agent::Claude, &deny).exit_code, 2);
        assert_eq!(j(render(Agent::Claude, &Outcome::Approved))["hookSpecificOutput"]["permissionDecision"], "allow");
        assert_eq!(render(Agent::Codex, &Outcome::Approved).stdout, "");
        assert_eq!(j(render(Agent::Gemini, &deny))["decision"], "deny");
        assert_eq!(render(Agent::Gemini, &deny).exit_code, 0);
        assert_eq!(j(render(Agent::Copilot, &deny))["permissionDecision"], "deny");
        assert_eq!(j(render(Agent::Cursor, &deny))["permission"], "deny");
        assert_eq!(j(render(Agent::Cursor, &Outcome::Approved))["permission"], "allow");
        assert_eq!(j(render(Agent::Vibe, &deny))["decision"], "deny");
        assert_eq!(render(Agent::Vibe, &deny).exit_code, 0);
        for agent in [Agent::Goose, Agent::OpenCode, Agent::Pi, Agent::Hermes, Agent::Amp] {
            assert_eq!(j(render(agent, &deny))["decision"], "block");
            assert_eq!(render(agent, &deny).exit_code, 2);
        }
    }

    #[test]
    fn config_changes_that_switch_the_hook_off_are_blocked() {
        let dir = std::env::temp_dir().join(format!("sag-cc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("settings.json");
        let check = |source: &str, content: Option<&str>| {
            match content {
                Some(c) => std::fs::write(&file, c).unwrap(),
                None => {
                    let _ = std::fs::remove_file(&file);
                }
            }
            let payload = json!({"source": source, "file_path": file.to_str().unwrap()});
            config_change_outcome(&payload.to_string(), 600)
        };
        let settings = |matcher: &str, command: &str, timeout: u64| {
            json!({"hooks": {"PreToolUse": [{"matcher": matcher, "hooks": [
                {"type": "command", "command": command, "timeout": timeout}]}]}})
            .to_string()
        };
        let cmd = "[ -x /b ] || exit 2; /b --hook claude --wait 600 --state-dir /s --ok-code 10; rc=$?";
        let ours = settings(CLAUDE_MATCHER, cmd, 660);
        assert_eq!(check("user_settings", Some(&ours)), Outcome::Neutral);
        // Changes that keep the text but switch the hook off.
        for changed in [
            settings(CLAUDE_MATCHER, cmd, 1),
            settings("^NOTHING$", cmd, 660),
            settings(CLAUDE_MATCHER, "exit 0; /b --hook claude --wait 600 ", 660),
            settings(CLAUDE_MATCHER, "[ -x /b ] || exit 2; /b --hook claude --wait 60 ", 660),
        ] {
            assert!(matches!(check("user_settings", Some(&changed)), Outcome::Deny(_)), "{changed}");
        }
        assert_eq!(check("project_settings", Some(r#"{"model":"x"}"#)), Outcome::Neutral);
        assert_eq!(check("project_settings", None), Outcome::Neutral);
        for (source, content) in [
            ("project_settings", Some(r#"{"disableAllHooks": true}"#)),
            ("local_settings", Some(r#"{"env": {"CLAUDE_CODE_SIMPLE": "1"}}"#)),
            ("user_settings", Some(r#"{"hooks": {}}"#)),
            ("user_settings", Some("{not json")),
            ("user_settings", None),
        ] {
            assert!(matches!(check(source, content), Outcome::Deny(_)), "{source} {content:?}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_decoy_argument_never_hides_what_is_sent() {
        // korotovsky sends `payload`/`text`; a `message` decoy is not shown alone.
        let p = ask(
            "mcp__slack__conversations_add_message",
            json!({"channel_id": "#eng", "message": "standup notes", "payload": "the real text"}),
        );
        assert!(!p.has_text);
        assert!(p.message.contains("the real text") && p.message.contains("standup notes"));
        // Two destinations: both are named, and the whole call is shown.
        let p = ask(
            "mcp__slack__slack_post_message",
            json!({"channel_id": "C_TEAM", "channel": "C_CEO", "text": "Build passed"}),
        );
        assert_eq!(p.destination, "C_TEAM, C_CEO");
        assert!(!p.has_text && p.message.contains("C_CEO"));
        // No destination: a placeholder, and no deny cooldown.
        let p = ask("mcp__slack__slack_create_canvas", json!({"title": "Plan", "content": "x"}));
        assert!(!p.destination_known);
        let p = ask("mcp__slack__slack_send_message", json!({"channel_id": "C1", "message": "hi"}));
        assert!(p.destination_known && p.has_text);
    }

    #[test]
    fn executor_is_recognized_in_every_naming_scheme() {
        let send = json!({"calls": {"path": "tools.mistral.slack.slack_send_message",
            "input": {"channel_id": "C1", "message": "x"}}});
        for name in ["executor-call", "mcp_executor_call", "executor_call", "executor-v2-call", "mcp__executor-v2__call"] {
            assert!(matches!(classify(&call(name, send.clone())), Classification::Ask(_)), "{name}");
        }
        let koro = json!({"calls": [{"path": "tools.ws.conversations_add_message",
            "input": {"channel_id": "C1", "text": "x"}}]});
        assert!(matches!(classify(&call("mcp__executor__call", koro)), Classification::Ask(_)));
        let code = json!({"code": "await tools.ws.conversations_add_message({channel_id:'C1', text:'x'})"});
        assert!(matches!(classify(&call("executor-execute", code)), Classification::Ask(_)));
    }

    #[test]
    fn hidden_slack_calls_in_programs_are_asked_about() {
        for program in [
            "await tools.mistral['sl'+'ack']['sl' + 'ack_send_message']({})",
            "await tools.mistral.\\u0073lack.\\u0073lack_send_message({})",
            "await tools.mistral.\\x73lack.slack_send_message({})",
            "const [p] = await tools.search({query: 'post'}); await tools[p.app][p.tool]({})",
            "await globalThis['sl'+'ack__sl'+'ack_send_message']({})",
            "await eval('tools.x.y')()",
            "await ws__conversations_add_message({channel_id: 'C1', text: 'x'})",
        ] {
            assert!(slack_write_in_code(program), "{program}");
        }
        for program in [
            "return await tools.mistral.slack.slack_read_channel({channel_id: 'C1'})",
            "return await tools.github.issues.list({repo: 'x'})",
            "return 1 + 1",
        ] {
            assert!(!slack_write_in_code(program), "{program}");
        }
    }

    #[test]
    fn write_names_without_a_read_verb_are_asked_about() {
        for tool in [
            "slack_message_user", "slack_ping_user", "slack_ask_user", "slack_mention_users",
            "slack_message_members", "slack_sends_message_to_user", "slack_invites_users",
            "slack_send_auth_link", "slack_publish_saved_view", "slack_addBookMark", "slack_frobnicate",
        ] {
            assert!(is_write_tool(tool), "{tool}");
        }
        for tool in [
            "slack_list_scheduled_messages", "slack_read_channel", "slack_get_reactions",
            "slack_search_public_and_private", "slack_list_channel_members", "slack_read_user_profile",
            "conversations_history", "conversations_unreads", "channels_list", "channels_me",
            "users_search", "attachment_get_data", "saved_list", "slack_send_message_draft",
            "conversations_mark", "slack_get_file_upload_url",
        ] {
            assert!(!is_write_tool(tool), "{tool}");
        }
    }

    #[test]
    fn only_speakeasys_own_server_is_exempt() {
        assert!(!passes("mcp__speakeasy_slack__slack_send_message"));
        assert!(!passes("mcp__slack__request_slack_message_now"));
        assert!(passes("mcp__slack-approval-gateway__request_slack_message"));
    }

    #[test]
    fn blank_ids_are_ignored_and_the_share_key_identifies_one_call() {
        let c = parse_payload(r#"{"tool_name":"x","tool_input":{},"session_id":"","conversation_id":"conv-1"}"#).unwrap();
        assert_eq!(c.session.as_deref(), Some("conv-1"));

        let claude = parse_payload(r#"{"tool_name":"MCP:slack_send_message","tool_input":{"channel_id":"C1","message":"x"},"session_id":"s","tool_use_id":"t1"}"#).unwrap();
        let cursor = parse_payload(r#"{"tool_name":"slack_send_message","tool_input":"{\"channel_id\":\"C1\",\"message\":\"x\"}","mcp_server_name":"slack","session_id":"s","tool_use_id":"t1"}"#).unwrap();
        // Under Cursor, the imported Claude Code hook calls itself Cursor.
        assert_eq!(share_key(Agent::Cursor, &claude), share_key(Agent::Cursor, &cursor));
        assert_ne!(share_key(Agent::Codex, &claude), share_key(Agent::Cursor, &claude));
        let mut other_args = claude.clone();
        other_args.input = json!({"channel_id": "C1", "message": "x", "reply_broadcast": true});
        assert_ne!(share_key(Agent::Cursor, &other_args), share_key(Agent::Cursor, &claude));
        let mut other_session = claude.clone();
        other_session.session = Some("s2".into());
        assert_ne!(share_key(Agent::Cursor, &other_session), share_key(Agent::Cursor, &claude));
        let mut no_id = claude.clone();
        no_id.call_id = None;
        assert_eq!(share_key(Agent::Cursor, &no_id), None);
    }

    #[test]
    fn parses_the_command_line() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(parse_args(&a(&["bin", "--serve"])).is_none());
        assert_eq!(
            parse_args(&a(&["bin", "--hook", "claude"])).unwrap().unwrap(),
            HookArgs {
                agent: Agent::Claude,
                wait_secs: DEFAULT_WAIT_SECS,
                state_dir: None,
                ok_code: 0,
                config_change: false,
            }
        );
        assert_eq!(
            parse_args(&a(&[
                "bin", "--hook", "vibe", "--wait", "120", "--state-dir", "/s", "--ok-code", "10",
                "--event", "config-change",
            ]))
            .unwrap()
            .unwrap(),
            HookArgs {
                agent: Agent::Vibe,
                wait_secs: 120,
                state_dir: Some(PathBuf::from("/s")),
                ok_code: 10,
                config_change: true,
            }
        );
        assert_eq!(
            parse_args(&a(&["bin", "--hook", "pdev"])).unwrap().unwrap().agent,
            Agent::Claude
        );
        for bad in [
            &["bin", "--hook", "pi", "--state-dir", "relative"][..],
            &["bin", "--hook", "pi", "--ok-code", "0"],
            &["bin", "--hook", "pi", "--ok-code", "2"],
            &["bin", "--hook", "pi", "--event", "other"],
        ] {
            assert!(parse_args(&a(bad)).unwrap().is_err(), "{bad:?}");
        }
        assert!(parse_args(&a(&["bin", "--hook"])).unwrap().is_err());
        assert!(parse_args(&a(&["bin", "--hook", "nope"])).unwrap().is_err());
        assert!(parse_args(&a(&["bin", "--hook", "pi", "--wait", "5"])).unwrap().is_err());
    }
}
