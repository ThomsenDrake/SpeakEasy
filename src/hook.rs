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
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::ops::Range;
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
    // No read verb: `slack_message_user`, `slack_frobnicate`, …
    write_verdict(tool).unwrap_or(true)
}

/// What a tool's name says: `Some(true)` it writes, `Some(false)` it only
/// reads (or is self-only), `None` it says neither (`slack`, `slackResults`).
fn write_verdict(tool: &str) -> Option<bool> {
    let lower = tool.to_ascii_lowercase();
    if NOT_WRITES.iter().any(|t| lower == *t || lower.ends_with(&format!("_{t}"))) {
        return Some(false);
    }
    let w = words(tool);
    let any = |list: &[&str]| w.iter().any(|x| stems(x).iter().any(|s| list.contains(s)));
    if any(&WRITE_WORDS) {
        return Some(true);
    }
    any(&READ_WORDS).then_some(false)
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
            // Each string on its own: a backtick in another field must not
            // open a template over the program.
            let strings = all_strings(&call.input);
            if !strings.iter().any(|s| slack_write_in_code(s, true)) {
                return Classification::Pass;
            }
            let program = strings.join("\n");
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
            let writes: Vec<(String, &Value)> = calls
                .iter()
                .filter_map(|c| {
                    // `tools.ws["conversations_add_message"]` names its tool too.
                    let path = dot_constant_keys(c.get("path")?.as_str()?);
                    let last = path.rsplit('.').next().unwrap_or(&path);
                    let write = is_slack_name(&path) && is_write_tool(last);
                    write.then(|| (path, c.get("input").unwrap_or(&Value::Null)))
                })
                .collect();
            // A dispatcher (`tools.executor.tools.call`) or a raw HTTP call
            // reaches Slack through its input: checked as the program
            // `path(input)`.
            let reaches_slack = |c: &&Value| {
                let path = c.get("path").and_then(Value::as_str).unwrap_or("");
                let input = c.get("input").unwrap_or(&Value::Null);
                slack_write_in_code(&format!("{path}({input})"), false)
            };
            let reaching = calls.iter().filter(|c| reaches_slack(c)).count();
            match writes.as_slice() {
                [] if reaching == 0 => Classification::Pass,
                // Shown alone only when no other call reaches Slack.
                [(path, inner)] if reaching <= 1 => {
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
            let strings = all_strings(input);
            let code = strings.join("\n");
            if strings.iter().any(|s| slack_write_in_code(s, false)) {
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
            // Only what spells a name (`\u0073lack`): a decoded quote,
            // slash or `$` would move where strings, regular expressions and
            // comments end (`"\x22"`, `/\x2f/`).
            let hex = |from: usize, len: usize| -> Option<char> {
                let s: String = chars.get(from..from + len)?.iter().collect();
                char::from_u32(u32::from_str_radix(&s, 16).ok()?)
                    .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
            };
            match chars[i + 1] {
                'u' if chars.get(i + 2) == Some(&'{') => {
                    let end = chars[i + 3..].iter().take_while(|c| c.is_ascii_hexdigit()).count();
                    if chars.get(i + 3 + end) == Some(&'}') {
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
            // A closing quote followed by `+` and an opening quote of the
            // same kind: join (a template joined to a string stays open).
            let mut j = i + 1;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            if chars.get(j) == Some(&'+') {
                let mut k = j + 1;
                while k < chars.len() && chars[k].is_whitespace() {
                    k += 1;
                }
                if chars.get(k) == Some(&c) {
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

/// Slack Web API methods that write, as a raw HTTP call names them
/// (`path: "/chat.postMessage"` through a connector's HTTP tool).
const SLACK_API_WRITES: [&str; 26] = [
    "chat.postmessage", "chat.postephemeral", "chat.schedulemessage", "chat.update",
    "chat.delete", "chat.memessage", "reactions.add", "files.upload",
    "files.completeuploadexternal", "conversations.create", "conversations.invite",
    "conversations.kick", "conversations.join", "conversations.archive", "conversations.rename",
    "conversations.settopic", "conversations.setpurpose", "pins.add", "bookmarks.add",
    "reminders.add", "canvases.create", "canvases.edit", "views.publish", "usergroups.create",
    "usergroups.update", "users.profile.set",
];

/// Words after which a value starts: `[` opens an array literal, not an
/// index (`return ["a"]`), and `/` a regular expression, not a division.
const VALUE_KEYWORDS: [&str; 14] = [
    "return", "typeof", "of", "in", "await", "yield", "case", "do", "else", "new", "delete",
    "void", "throw", "instanceof",
];

/// Words of a tool that runs another tool or an automation by name
/// (`tools.executor.tools.call`, `call_tool`, `execute_workflow`,
/// `create_workflow_schedule`, `schedules.runNow`, `workflows.start`), and
/// of an app that sends raw requests (`tools.http.request`,
/// `test_connector_http_call`).
const DISPATCH_WORDS: [&str; 9] =
    ["call", "invoke", "execute", "exec", "run", "start", "trigger", "dispatch", "workflow"];
const RAW_WORDS: [&str; 3] = ["http", "webhook", "proxy"];

/// Words that make such a tool a query instead (`run_query`, `execute_sql`).
const QUERY_WORDS: [&str; 4] = ["query", "sql", "statement", "search"];

/// Characters of a JavaScript identifier.
fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '$')
}

/// Characters of a JavaScript name, plus the `-` of a dotted hyphenated
/// tool name (`notion-search`).
fn is_name_char(c: char) -> bool {
    is_ident_char(c) || c == '-'
}

/// The name at the start of `s` (`notion-search` of `notion-search({…})`).
fn leading_name(s: &str) -> &str {
    &s[..s.find(|c: char| !is_name_char(c)).unwrap_or(s.len())]
}

/// Whether `s` ends with the word `word` (`return`, not `xreturn`).
fn ends_with_word(s: &str, word: &str) -> bool {
    s.strip_suffix(word).is_some_and(|rest| !rest.ends_with(is_ident_char))
}

/// The program with the text of its strings, regular expressions and
/// comments blanked out, offsets kept, so that names written as text
/// (`"tools.mistral.slack is not connected"`, `// eval(`) are not read as
/// code; the code inside a template's `${…}` stays. A quote or regular
/// expression still open at the end of its line was misread (only a
/// template, or a string continued with `\`, spans lines), so its text
/// stays code.
fn code_only(code: &str) -> String {
    #[derive(Clone, Copy, PartialEq)]
    enum Mode {
        Code,
        Quote(char),
        Template,
        LineComment,
        BlockComment,
        Regex { class: bool },
    }
    let blank = |out: &mut String, c: char| out.extend(std::iter::repeat_n(' ', c.len_utf8()));
    let mut out = String::with_capacity(code.len());
    let mut mode = Mode::Code;
    // Where the last code character was: the start of an open quote or
    // regular expression.
    let mut opened = 0;
    let line_end = |c: char| matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}');
    // The brace depth at each open `${`, innermost last.
    let mut interpolations: Vec<usize> = Vec::new();
    let mut depth = 0;
    let mut chars = code.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        let next = chars.peek().map(|&(_, n)| n);
        match mode {
            Mode::Code => {
                match c {
                    '"' | '\'' => mode = Mode::Quote(c),
                    '`' => mode = Mode::Template,
                    '/' if next == Some('/') => mode = Mode::LineComment,
                    '/' if next == Some('*') => {
                        // `/*/` opens a comment; its `*` does not close one.
                        chars.next();
                        out.push(' ');
                        mode = Mode::BlockComment;
                    }
                    '/' => {
                        // A regular expression where a value starts, else a division.
                        let before = out.trim_end();
                        let value = before.is_empty()
                            || before.ends_with([
                                '(', ',', '=', ':', '[', '!', '&', '|', '?', '{', '}', ';', '+', '-',
                                '*', '%', '<', '>', '~', '^',
                            ])
                            || VALUE_KEYWORDS.iter().any(|k| ends_with_word(before, k));
                        if value {
                            mode = Mode::Regex { class: false };
                        }
                    }
                    '{' => depth += 1,
                    '}' if interpolations.last() == Some(&depth) => {
                        interpolations.pop();
                        mode = Mode::Template;
                        depth -= 1;
                    }
                    '}' => depth = usize::saturating_sub(depth, 1),
                    _ => {}
                }
                opened = i;
                if matches!(mode, Mode::LineComment | Mode::BlockComment) {
                    blank(&mut out, c);
                } else {
                    out.push(c);
                }
            }
            Mode::Quote(_) | Mode::Template | Mode::Regex { .. } if c == '\\' => {
                blank(&mut out, c);
                // A string continues past `\` and a line end (also `\r\n`);
                // a regular expression does not.
                let regex = matches!(mode, Mode::Regex { .. });
                if let Some(&(_, escaped)) = chars.peek().filter(|&&(_, e)| !(regex && line_end(e))) {
                    chars.next();
                    if line_end(escaped) {
                        out.push_str(&code[i + 1..i + 1 + escaped.len_utf8()]);
                        if escaped == '\r' && chars.peek().is_some_and(|&(_, n)| n == '\n') {
                            chars.next();
                            out.push('\n');
                        }
                    } else {
                        blank(&mut out, escaped);
                    }
                }
            }
            Mode::Quote(q) if c == q => {
                mode = Mode::Code;
                out.push(c);
            }
            Mode::Template if c == '`' => {
                mode = Mode::Code;
                out.push(c);
            }
            Mode::Template if c == '$' && next == Some('{') => {
                chars.next();
                out.push_str("${");
                depth += 1;
                interpolations.push(depth);
                mode = Mode::Code;
            }
            Mode::Regex { class } if c == '[' || c == ']' || (c == '/' && !class) => {
                mode = if c == '/' { Mode::Code } else { Mode::Regex { class: c == '[' } };
                if c == '/' {
                    out.push(c);
                } else {
                    blank(&mut out, c);
                }
            }
            Mode::BlockComment if c == '*' && next == Some('/') => {
                chars.next();
                out.push_str("  ");
                mode = Mode::Code;
            }
            // Misread: a quote or regular expression ends with its line.
            Mode::Quote(_) | Mode::Regex { .. } if line_end(c) => {
                out.truncate(opened);
                out.push_str(&code[opened..i + c.len_utf8()]);
                mode = Mode::Code;
            }
            Mode::LineComment if line_end(c) => {
                mode = Mode::Code;
                out.push(c);
            }
            _ if line_end(c) => out.push(c),
            _ => blank(&mut out, c),
        }
    }
    if matches!(mode, Mode::Quote(_) | Mode::Template | Mode::Regex { .. }) {
        out.truncate(opened);
        out.push_str(&code[opened..]);
    }
    out
}

/// Spell bracket keys that are one constant name as dotted names
/// (`tools.notion["notion-search"]` → `tools.notion.notion-search`):
/// Executor writes hyphenated tool names this way, so such a key is a
/// static name, not a computed one. Array literals (`= ["a"]`, `return
/// ["a"]`) and keys with other characters (`["get post"]`) stay.
fn dot_constant_keys(code: &str) -> String {
    let mut out = String::with_capacity(code.len());
    let mut rest = code;
    while let Some(open) = rest.find('[') {
        out.push_str(&rest[..open]);
        rest = &rest[open + 1..];
        let before = out.trim_end();
        let word = before.rsplit(|c: char| !is_name_char(c)).next().unwrap_or("");
        // `x.delete["y"]` is a member named like a keyword.
        let keyword = VALUE_KEYWORDS.contains(&word)
            && !before[..before.len() - word.len()].ends_with('.');
        let indexes = !keyword
            && before.ends_with(|c: char| is_name_char(c) || matches!(c, '.' | ')' | ']'));
        let key = rest.trim_start();
        let literal = key.chars().next().filter(|q| matches!(q, '\'' | '"' | '`')).and_then(|q| {
            let (name, after) = key[1..].split_once(q)?;
            let after = after.trim_start().strip_prefix(']')?;
            let plain = !name.is_empty() && name.chars().all(|c| is_name_char(c) || c == '.');
            plain.then_some((name, after))
        });
        match literal {
            Some((name, after)) if indexes => {
                // `?.["x"]`, `?. ["x"]` → `?.x`
                out.truncate(out.trim_end().len());
                if !out.ends_with('.') {
                    out.push('.');
                }
                out.push_str(name);
                rest = after;
            }
            _ => out.push('['),
        }
    }
    out.push_str(rest);
    out
}

/// Whether a program builds a function from text, which a text search
/// cannot follow: it calls or takes `eval` or a `Function` constructor
/// (`eval(`, `window.eval?.(`, `await eval (src)`, `const e = eval;`; not
/// `rag_retrieval(` or "offline eval (large)"), or takes a constructor
/// (`(async () => {}).constructor`, not `e.constructor.name` or
/// `x.constructor === Object`).
fn builds_functions(code: &str) -> bool {
    let used = |name: &str| {
        code.match_indices(name).any(|(i, _)| {
            let (before, after) = (&code[..i], &code[i + name.len()..]);
            if before.ends_with(is_ident_char) || after.starts_with(is_ident_char) {
                return false;
            }
            let tight = after.strip_prefix("?.").unwrap_or(after).starts_with('(');
            // After code, not after a word in prose.
            let line = before.trim_end_matches([' ', '\t']);
            let code_before = line.is_empty()
                || line.ends_with(['=', '(', ',', ';', '{', '[', ':', '?', '!', '&', '|', '.', '\n'])
                || VALUE_KEYWORDS.iter().any(|k| ends_with_word(line, k));
            let next = after.trim_start();
            let taken = next.is_empty() || next.starts_with(['(', ';', ')', ',', ']', '}']);
            tight || (code_before && taken)
        })
    };
    let constructor = code.match_indices(".constructor").any(|(i, m)| {
        let rest = &code[i + m.len()..];
        let after = rest.trim_start();
        let member = after.strip_prefix('?').unwrap_or(after);
        let looks = member.strip_prefix(".name").is_some_and(|r| !r.starts_with(is_name_char))
            || ["===", "!==", "==", "!="].iter().any(|op| after.starts_with(op));
        !rest.starts_with(is_name_char) && !looks
    });
    constructor
        || ["eval", "Function", "AsyncFunction", "GeneratorFunction", "AsyncGeneratorFunction"]
            .into_iter()
            .any(used)
}

/// Where Executor's `tools` starts a chain: not `foundTools` or
/// `result.tools`, but `globalThis.tools` and `...tools`.
fn tools_roots(code: &str) -> Vec<(usize, usize)> {
    let root = |i: usize| {
        let before = &code[..i];
        let owner = Some(before.trim_end())
            .filter(|b| !b.ends_with("..."))
            .and_then(|b| b.strip_suffix('.'))
            .map(|o| o.trim_end_matches('?').trim_end());
        !before.ends_with(is_ident_char)
            && !code[i + 5..].starts_with(is_name_char)
            && owner.is_none_or(|o| {
                ["globalThis", "window", "self", "this"].iter().any(|g| ends_with_word(o, g))
            })
    };
    code.match_indices("tools").map(|(i, _)| i).filter(|&i| root(i)).map(|i| (i, 5)).collect()
}

/// Where goose's Code Mode names a Slack extension's namespace
/// (`Slack.slackSendMessage(…)`, `const s = Slack`): `slack` or a
/// capitalized name with Slack in it (not `slackResults`), used as an object
/// or a value rather than as a word in text.
fn slack_namespaces(code: &str) -> Vec<(usize, usize)> {
    let lower = code.to_ascii_lowercase();
    let own_slack = declares(code, "slack");
    let mut out = Vec::new();
    for (i, _) in lower.match_indices("slack") {
        let window = floor_boundary(&lower, i.saturating_sub(128));
        let start = i - lower[window..i].rsplit(|c: char| !is_ident_char(c)).next().unwrap_or("").len();
        let end = floor_boundary(code, (start + 128).min(code.len()));
        let len = code[start..end].find(|c: char| !is_ident_char(c)).unwrap_or(end - start);
        let before = code[..start].trim_end();
        let after = code[start + len..].trim_start();
        let member = after.strip_prefix("?.").or_else(|| after.strip_prefix('.'));
        // `Slack[op]` or `Slack.fn`, not the type `SlackPost[]` or "slack.com".
        let computed = after.strip_prefix('[').is_some_and(|k| !k.trim_start().starts_with(']'));
        let object =
            computed || member.is_some_and(|m| m.starts_with(is_ident_char) && !m.starts_with("com"));
        // `= Slack;`, `? Slack : Teams`, `Slack ?? null`, `Slack!`, `(Slack as any)`.
        let taken = before.ends_with(['=', '(', ',', ':', '?', '[']) || ends_with_word(before, "return");
        let value = taken
            && (after.is_empty()
                || after.starts_with([';', ')', ',', '}', ']', ':', '?', '!', '|', '&'])
                || after.starts_with("as "));
        // `slack`, unless the program has its own (`const slack = chats.filter(…)`).
        let lowercase = &code[start..start + len] == "slack" && !own_slack;
        let named = code[start..].starts_with(|c: char| c.is_ascii_uppercase()) || lowercase;
        let key = after.starts_with(':') && before.ends_with(['{', ',']);
        if named && !before.ends_with('.') && !key && (object || value) {
            out.push((start, len));
        }
    }
    out
}

/// Where the member chain that starts at `from` ends (`.app.tool`,
/// `?.app`), and whether a computed key follows it (`[name]`; not `[0]`,
/// which names no tool, or a type's `[]`).
fn member_chain(code: &str, from: usize) -> (usize, bool) {
    let computed = |rest: &str| {
        let key = rest.strip_prefix('[').map(str::trim_start);
        key.is_some_and(|k| !k.starts_with(|c: char| c.is_ascii_digit() || c == ']'))
    };
    let mut end = from;
    loop {
        let rest = code[end..].trim_start();
        let dot = rest.strip_prefix("?.").or_else(|| rest.strip_prefix('.'));
        let Some(name) = dot.map(str::trim_start) else {
            return (end, computed(rest));
        };
        let len = leading_name(name).len();
        if len == 0 {
            return (end, computed(name));
        }
        end = code.len() - name.len() + len;
    }
}

/// The names in a member chain (`tools.mistral?.slack` → tools, mistral, slack).
fn chain_names(chain: &str) -> Vec<String> {
    chain.split(['.', '?']).map(str::trim).filter(|n| !n.is_empty()).map(str::to_string).collect()
}

/// The tool a chain names: its last name, without a trailing `bind` or
/// `apply`, or `call` on a tool (`slack_read_channel.call(…)`, but not
/// Executor's own `tools.executor.tools.call`).
fn chain_tool(names: &[String]) -> &str {
    let names = match names {
        [rest @ .., how] if !rest.is_empty() && ["bind", "apply"].contains(&how.as_str()) => rest,
        _ => names,
    };
    match names {
        [.., tool, how] if how == "call" && write_verdict(tool).is_some() => tool,
        [.., tool] => tool,
        [] => "",
    }
}

/// Whether one of a name's words (`sends` → `send`) is in `list`.
fn has_word(name: &str, list: &[&str]) -> bool {
    words(name).iter().any(|w| stems(w).iter().any(|s| list.contains(s)))
}

/// Whether a chain runs something else by name or sends raw requests (see
/// [`DISPATCH_WORDS`]); a query (`run_query`, `execute({ sql })`), a read,
/// or a raw request in a program that only sends `"GET"`s does not.
fn dispatches(names: &[String], tool: &str, only_gets: bool, args: &str) -> bool {
    let has = has_word;
    // `execute({ sql: "…" })`, not an API's `{ query: { … }, path, body }`.
    let object = args.trim_start().strip_prefix('(').map(str::trim_start);
    let first = object.and_then(|a| a.strip_prefix('{')).map_or("", str::trim_start);
    let key = leading_name(first);
    let value = first[key.len()..].trim_start().strip_prefix(':').map(str::trim_start);
    let text = !value.is_some_and(|v| v.starts_with(['{', '[']));
    let query = has(tool, &QUERY_WORDS) || (text && has(key, &QUERY_WORDS));
    let raw = names.iter().any(|n| has(n, &RAW_WORDS));
    // `http.post` writes whatever else the program GETs.
    let gets = raw && only_gets && write_verdict(tool).is_none();
    write_verdict(tool) != Some(false) && !query && !gets && (raw || has(tool, &DISPATCH_WORDS))
}

/// What a chain's value is bound to: a name or an object pattern, as the span
/// of the name or of the text between the pattern's braces.
enum Binding {
    Name(Range<usize>),
    Pattern(Range<usize>),
}

/// What the value of the chain at `chain` is bound to, when the chain is the
/// whole right side of a declaration (`const s = tools.x`, `const { a, b: c }
/// = tools.x`, `let s: Api = tools.x as Api, t = …`, `const s = tools.x ??
/// tools.y`).
fn binding(code: &str, chain: &Range<usize>) -> Option<Binding> {
    let rest = after_cast(&code[chain.end..]);
    let alternative = rest.starts_with("??") || rest.starts_with("||");
    let ends = rest.is_empty() || rest.starts_with([';', ',', '\n', '\r']);
    if !(ends || alternative) {
        return None;
    }
    // Either side of `tools.mistral?.slack ?? tools.slack` is bound (a few
    // alternatives back at most).
    let mut before = code[..chain.start].trim_end();
    for _ in 0..4 {
        let Some(rest) = before.strip_suffix("??").or_else(|| before.strip_suffix("||")) else {
            break;
        };
        let rest = rest.trim_end();
        let in_chain = |c: char| is_name_char(c) || matches!(c, '.' | '?');
        let other = rest.rsplit(|c: char| !in_chain(c)).next().unwrap_or("");
        before = rest[..rest.len() - other.len()].trim_end();
    }
    let target = before.strip_suffix('=')?.trim_end();
    // A declaration starts within its last few lines; look no further.
    let base = floor_boundary(target, target.len().saturating_sub(4096));
    let target = &target[base..];
    let keyword_before = |before: &str| {
        let before = before.trim_end();
        before.ends_with(',') || ["const", "let", "var"].iter().any(|k| ends_with_word(before, k))
    };
    // `const s: Api = …`, `const { a }: Api = …`, within the statement.
    let statement = target.rfind([';', '\n']).map_or(0, |i| i + 1);
    let typed = match (target[statement..].split_once(':'), target[statement..].rsplit_once(':')) {
        _ if target.ends_with('}') => None,
        (_, Some((pattern, _))) if pattern.trim_end().ends_with('}') => Some(pattern),
        (Some((name, _)), _) => Some(name),
        _ => None,
    };
    let target = typed.map_or(target, |t| target[..statement + t.len()].trim_end());
    if target.ends_with('}') {
        let mut depth = 0;
        let (open, _) = target.char_indices().rev().find(|&(_, c)| {
            depth += match c {
                '}' => 1,
                '{' => -1,
                _ => 0,
            };
            depth == 0
        })?;
        let keys = base + open + 1..base + target.len() - 1;
        return keyword_before(&target[..open]).then_some(Binding::Pattern(keys));
    }
    let name = target.rsplit(|c: char| !is_name_char(c)).next().unwrap_or("");
    let start = target.len() - name.len();
    let declared = !name.is_empty() && keyword_before(&target[..start]);
    declared.then_some(Binding::Name(base + start..base + target.len()))
}

/// `s` past a TypeScript cast (`as Api`, `as Record<K, V>`) and spaces.
fn after_cast(s: &str) -> &str {
    let s = s.trim_start_matches([' ', '\t']);
    let type_char = |c: char| is_ident_char(c) || matches!(c, '<' | '>' | '.' | ',');
    s.strip_prefix("as ").map_or(s, |cast| cast.trim_start().trim_start_matches(type_char))
}

/// The keys of a flat object pattern and the names they bind (`a, b: c,
/// ...d` → (a, a), (b, c), ("", d)), or `None` when the pattern nests or
/// has an entry this cannot read.
fn pattern_entries(text: &str) -> Option<Vec<(&str, &str)>> {
    if text.contains(['{', '[']) {
        return None;
    }
    text.split(',')
        .filter(|entry| !entry.trim().is_empty())
        .map(|entry| {
            let entry = entry.trim();
            let (key, bound) = match entry.strip_prefix("...") {
                Some(rest) => ("", rest),
                None => entry.split_once(':').unwrap_or((entry, entry)),
            };
            let key = key.split('=').next().unwrap_or("").trim();
            let bound = bound.split('=').next().unwrap_or("").trim();
            (!bound.is_empty() && bound.chars().all(is_name_char)).then_some((key, bound))
        })
        .collect()
}

/// Where `name` is used as a whole name outside `declared`, the span that
/// binds it: `s` in `s.x`, `f(s)` and `...s`, not in `x.s` or an object key
/// (`{ s: 1 }`).
fn uses<'a>(
    code: &'a str,
    idents: &'a HashMap<&str, Vec<usize>>,
    name: &'a str,
    declared: &'a Range<usize>,
) -> impl Iterator<Item = usize> + 'a {
    idents.get(name).into_iter().flatten().copied().filter(move |&i| {
        let (before, after) = (&code[..i], &code[i + name.len()..]);
        let member = before.ends_with('.') && !before.ends_with("...");
        let key = after.trim_start().starts_with(':') && before.trim_end().ends_with(['{', ',']);
        // `api = …` gives it a value; it does not use one.
        let set = after.trim_start().strip_prefix('=').is_some_and(|a| !a.starts_with(['=', '>']));
        !declared.contains(&i) && !member && !key && !set
    })
}

/// Where each identifier of a program starts (`s` → every whole `s`).
fn identifiers(code: &str) -> HashMap<&str, Vec<usize>> {
    let mut out: HashMap<&str, Vec<usize>> = HashMap::new();
    let mut start = None;
    for (i, c) in code.char_indices().chain([(code.len(), ' ')]) {
        match (start, is_ident_char(c)) {
            (None, true) => start = Some(i),
            (Some(s), false) => {
                out.entry(&code[s..i]).or_default().push(s);
                start = None;
            }
            _ => {}
        }
    }
    out
}

/// The nearest char boundary at or before `i`.
fn floor_boundary(s: &str, i: usize) -> usize {
    (0..=i.min(s.len())).rev().find(|&b| s.is_char_boundary(b)).unwrap_or(0)
}

/// Whether the chain at `chain` is passed on as a value a text search cannot
/// follow (`reduce((o, k) => o[k], tools)`, `Reflect.get(tools.x, k)`,
/// `x = tools`, `{ ...tools }`, `(tools as any)`): not called, not a tool,
/// not a key, a parameter or a declared name.
fn passed_on(code: &str, chain: &Range<usize>, head: &str, tool: &str) -> bool {
    let before = code[..chain.start].trim_end();
    let after = after_cast(&code[chain.end..]);
    let value = before.ends_with("...")
        || (!before.ends_with('.')
            && (before.ends_with(['=', '(', ',', ':', '[', '?', '|', '&', '{', '>'])
                || ends_with_word(before, "await")
                || ends_with_word(before, "return")));
    let ends = after.is_empty()
        || after.starts_with([')', ',', ';', ']', '}', '|', '&', '?', ':', '/', '\n', '\r']);
    let key = after.starts_with(':') && before.ends_with(['{', ',']);
    let parameter = after.trim_start_matches(')').trim_start().starts_with("=>");
    // `const { tools } = result` declares a name of its own.
    let declared = after.strip_prefix('}').map(str::trim_start).is_some_and(|a| {
        a.starts_with('=') && !a.starts_with("==") && !a.starts_with("=>")
    });
    // `(n) => n.title` returns the arrow's own parameter.
    let arrow_parameter = before.strip_suffix("=>").is_some_and(|arrow| {
        let arrow = arrow.trim_end();
        let arrow = &arrow[floor_boundary(arrow, arrow.len().saturating_sub(256))..];
        let params = match arrow.strip_suffix(')') {
            Some(list) => &list[list.rfind('(').map_or(0, |i| i + 1)..],
            None => arrow.rsplit(|c: char| !is_name_char(c)).next().unwrap_or(""),
        };
        params.split(',').any(|p| p.trim() == head)
    });
    let named = !key && !parameter && !declared && !arrow_parameter;
    value && ends && named && write_verdict(tool).is_none()
}

/// Whether a program (Executor's `execute`, goose's Code Mode when
/// `namespaces`) may write to Slack. Every member chain from `tools` (or
/// from a goose namespace like `Slack`), and from a name bound to one,
/// which stands for the chain it was bound to (`const s =
/// tools.mistral.slack; s.post(…)` is `tools.mistral.slack.post(…)`), is
/// judged when it reaches a Slack app: by its tool's name, a write asks, a
/// read does not, and a name that says neither (`tools.mistral.slack`)
/// asks unless it is bound (its uses are judged instead). A computed tool
/// name (`tools.x[name]`, `globalThis[name]`, `eval`, `Function`) asks. A
/// program that also mentions Slack other than through a read asks when it
/// reaches tools in a way a text search cannot follow: `tools` passed on as
/// a value, a declaration it cannot read, or a dispatcher or raw HTTP tool
/// (`tools.executor.tools.call`, `execute_workflow`, `tools.http.request`).
/// Outside chains the word Slack is data (`slackResults`, "Slack
/// integration"), except goose's `server__tool` names, an undeclared
/// function named for a Slack write, korotovsky's tool names and Slack Web
/// API writes or webhooks written out.
fn slack_write_in_code(code: &str, namespaces: bool) -> bool {
    let code = normalize_program(code);
    // Before dotting: `globalThis["eval"]` must stay a computed name.
    let bare = code_only(&code);
    let indexes_global = |g: &str| {
        bare.match_indices(g).any(|(i, m)| {
            !bare[..i].ends_with(is_ident_char) && bare[i + m.len()..].trim_start().starts_with('[')
        })
    };
    if ["globalThis", "window", "this", "self"].into_iter().any(indexes_global) {
        return true;
    }
    let code = dot_constant_keys(&code);
    // Names are read in the code, Slack's name also in its strings.
    let bare = code_only(&code);
    let lower = code.to_ascii_lowercase();
    if builds_functions(&bare) {
        return true;
    }
    let quoted = |m: &str| ["\"", "'", "`"].iter().any(|q| lower.contains(&format!("{q}{m}{q}")));
    let only_gets = quoted("get") && !["post", "put", "patch", "delete"].into_iter().any(quoted);
    let idents = identifiers(&bare);
    let mut heads = if namespaces { slack_namespaces(&bare) } else { tools_roots(&bare) };
    // What a bound name stands for (`s` → tools, mistral, slack).
    let mut sources: HashMap<String, Vec<String>> = HashMap::new();
    let mut seen = HashSet::new();
    let word = |name: &str, then: &dyn Fn(&str) -> bool| {
        bare.match_indices(name).any(|(i, m)| {
            let after = bare[i + m.len()..].trim_start();
            !bare[..i].ends_with(|c: char| is_ident_char(c) || c == '.')
                && !after.starts_with(is_ident_char)
                && then(after)
        })
    };
    let called = |after: &str| after.starts_with('(');
    // Whether the program does more than read: `fetch(` sends raw requests.
    let mut acts = word("fetch", &called);
    // goose's `invoke({ name })` runs any tool by name; a global object taken
    // as a value (`const root = globalThis`) reaches `tools` unseen.
    let taken = |after: &str| {
        !after.starts_with(['.', '[']) && !(after.starts_with('=') && !after.starts_with("=="))
    };
    let mut dynamic = (namespaces && word("invoke", &called))
        || ["globalThis", "window", "self"].iter().any(|g| word(g, &taken));
    while let Some((at, len)) = heads.pop() {
        // A name bound from one of its own uses (`const a = b` … `const b =
        // a`) leads back to a chain already judged.
        if !seen.insert(at) {
            continue;
        }
        let (end, computed) = member_chain(&bare, at + len);
        if computed {
            return true;
        }
        let chain = at..end;
        let written = chain_names(&bare[chain.clone()]);
        let head = written.first().cloned().unwrap_or_default();
        let mut names = sources.get(&head).cloned().unwrap_or_else(|| vec![head.clone()]);
        names.extend(written.into_iter().skip(1));
        let tool = chain_tool(&names);
        let bound = binding(&bare, &chain);
        let looked_at = inspected(&bare, &chain);
        let unbound = bound.is_none() && !looked_at;
        if names.iter().any(|n| is_slack_name(n)) && write_verdict(tool).unwrap_or(unbound) {
            return true;
        }
        acts |= names.len() > 1 && write_verdict(tool) != Some(false);
        // An HTTP client sends a GET to a write method as well (Slack takes both).
        acts |= names.iter().any(|n| has_word(n, &RAW_WORDS));
        dynamic |= dispatches(&names, tool, only_gets, &bare[end..]);
        // A name declared again elsewhere (`const client` in two helpers)
        // stands for Slack's tools if either declaration does; its uses are
        // judged again then. A name switches once, so this ends.
        let mut follow = |name: &str, declared: &Range<usize>, source: Vec<String>| {
            let slack = |s: &Vec<String>| s.iter().any(|n| is_slack_name(n));
            let first = sources.get(name).is_none_or(|old| !slack(old) && slack(&source));
            if first && source.len() <= 32 {
                let found: Vec<usize> = uses(&bare, &idents, name, declared).collect();
                found.iter().for_each(|i| _ = seen.remove(i));
                heads.extend(found.into_iter().map(|i| (i, name.len())));
                sources.insert(name.to_string(), source);
            }
        };
        match &bound {
            Some(Binding::Name(declared)) => follow(&bare[declared.clone()], declared, names.clone()),
            Some(Binding::Pattern(keys)) => {
                let Some(entries) = pattern_entries(&bare[keys.clone()]) else {
                    dynamic = true;
                    continue;
                };
                for (key, name) in entries {
                    let mut source = names.clone();
                    source.extend((!key.is_empty()).then(|| key.to_string()));
                    follow(name, keys, source);
                }
            }
            None => dynamic |= !looked_at && passed_on(&bare, &chain, &head, tool),
        }
    }
    let mut mentions = false;
    let mut declared = HashMap::new();
    for (i, _) in lower.match_indices("slack") {
        // No tool's name is longer than this; a longer run is not one.
        let window = floor_boundary(&lower, i.saturating_sub(128));
        let start = i - lower[window..i].rsplit(|c: char| !is_name_char(c)).next().unwrap_or("").len();
        let name = leading_name(&code[start..floor_boundary(&code, (i + 128).min(code.len()))]);
        // goose's `slack__slack_send_message`, also as a string (`invoke({ name })`).
        if let Some((_, tool)) = name.rsplit_once("__").filter(|_| namespaces) {
            if write_verdict(tool) != Some(false) {
                return true;
            }
        }
        // A function named for a Slack write that the program calls but does
        // not declare (`slackSendMessage(`, not a helper `postToSlack`).
        let after = &bare[start + name.len()..];
        let called = after.strip_prefix("?.").unwrap_or(after).starts_with('(');
        let code_name = bare[start..].starts_with(name);
        let mut own = || *declared.entry(name).or_insert_with(|| declares(&bare, name));
        if code_name && called && write_verdict(name) == Some(true) && !own() {
            return true;
        }
        // What used to ask on its own: a Slack name that does not read.
        let run = &code[i..];
        let len = run
            .char_indices()
            .take(128)
            .find(|&(_, c)| !is_name_char(c) && c != '.')
            .map_or(run.len().min(128), |(k, _)| k);
        let last = run[..len].trim_end_matches('.').rsplit('.').next().unwrap_or("");
        mentions |= write_verdict(last) != Some(false);
    }
    // A Slack Web API write or webhook sent raw by a program that does more
    // than read (`path: "/chat.postMessage"`, `url:
    // "https://slack.com/api/chat.postMessage"`, `"https://hooks.slack.com/…"`).
    let quotes = ["\"", "'", "`"];
    let api = SLACK_API_WRITES.iter().any(|m| {
        let prefixes = ["", "/", "/api/", "https://slack.com/api/"];
        quotes.iter().any(|q| {
            prefixes.iter().any(|p| {
                let written = format!("{q}{p}{m}");
                lower.match_indices(&written).any(|(i, _)| {
                    let after = &lower[i + written.len()..];
                    after.starts_with(q) || after.starts_with(['?', '#', '$'])
                })
            })
        })
    });
    let webhook = quotes.iter().any(|q| lower.contains(&format!("{q}https://hooks.slack.com/")));
    let raw = (acts || dynamic) && (api || webhook);
    // Once `tools` is passed on, any `tools` property indexed by a computed
    // name may be it (`const ctx = { tools }; ctx.tools[name]`).
    let indexed = || {
        let mut walked = 0;
        bare.match_indices("tools").any(|(i, _)| {
            let fresh = i >= walked && !bare[..i].ends_with(is_ident_char);
            let (end, computed) = if fresh { member_chain(&bare, i + 5) } else { (walked, false) };
            walked = end;
            computed
        })
    };
    raw || (dynamic && (mentions || indexed()))
        || SLACK_TOOLS_WITHOUT_SLACK_IN_NAME
            .iter()
            .any(|t| lower.contains(t) && is_write_tool(t))
}

/// Whether a chain is only looked at (`Object.keys(tools.mistral.slack)`,
/// `if (!tools.mistral?.slack)`, `typeof tools.x`), not used or called.
fn inspected(code: &str, chain: &Range<usize>) -> bool {
    let before = code[..chain.start].trim_end();
    let after = code[chain.end..].trim_start();
    let called = after.strip_prefix("?.").unwrap_or(after).starts_with('(');
    !called
        && ((before.ends_with("Object.keys(") && after.starts_with(')'))
            || before.ends_with('!')
            || ends_with_word(before, "typeof"))
}

/// Whether a program declares `name` itself (`function postToSlack(`,
/// `const postToSlack =`).
fn declares(code: &str, name: &str) -> bool {
    code.match_indices(name).any(|(i, _)| {
        let before = code[..i].trim_end();
        let after = code[i + name.len()..].trim_start();
        let assigned = ["const", "let", "var"].iter().any(|k| ends_with_word(before, k));
        ends_with_word(before, "function") || (assigned && after.starts_with('='))
    })
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
        // goose's Code Mode namespaces (`Slack.fn()`) and its `invoke({ name })`.
        for (code, asks) in [
            ("async function run() { return await Slack.slackSendMessage({channel_id: 'C1'}) }", true),
            ("async function run() { return await Slack.message({channel: 'C1'}) }", true),
            ("async function run() { return await Slack[op]({channel: 'C1'}) }", true),
            ("async function run() { return await invoke({name: 'slack__slack_send_message', args}) }", true),
            ("async function run() { const s = 'slack'; return await invoke({name: `${s}__post`}) }", true),
            ("async function run() { const f = slack__frobnicate; return await f({}) }", true),
            ("async function run() { return await slack.send_message({channel: 'C1'}) }", true),
            ("async function run() { const s = slack; return await s.message({channel: 'C1'}) }", true),
            ("async function run() { const posts: SlackPost[] = []; return posts }", false),
            ("async function run() { const chat = useSlack ? Slack : Teams; return await chat.conversationsAddMessage({}) }", true),
            ("async function run() { return await Slack // the extension\n .postMessage({}) }", true),
            ("async function run() { const slack = chats.filter(c => c.network === 'Slack'); return slack.length }", false),
            ("async function run() { const hits = await Slack.slackSearchPublic({}); return { notion: n, slack: hits } }", false),
            ("async function run() { return 'see https://slack.com/help' }", false),
            ("async function run() { return await Slack.slackReadChannel({channel_id: 'C1'}) }", false),
            ("async function run() { return await Notion.notionSearch({query: 'Slack onboarding'}) }", false),
            ("async function run() { const tools = await Github.listTools(); return tools[0] }", false),
        ] {
            let c = call("execute_typescript", json!({"code": code}));
            assert_eq!(matches!(classify_for(Agent::Goose, &c), Classification::Ask(_)), asks, "{code}");
        }
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
            assert!(slack_write_in_code(program, false), "{program}");
        }
        for program in [
            "return await tools.mistral.slack.slack_read_channel({channel_id: 'C1'})",
            "return await tools.github.issues.list({repo: 'x'})",
            "return 1 + 1",
        ] {
            assert!(!slack_write_in_code(program, false), "{program}");
        }
    }

    #[test]
    fn constant_bracket_keys_are_names_not_computed() {
        // Executor spells hyphenated tools with quoted keys (a reported false positive).
        let notion = r#"const acc = "acc_a798760a-9ce6-4a8e-8c3d-504cea91ebac";
const qs = ["create GitHub App", "GitHub App installation organization owner", "Jarvis GitHub App", "Bad Company GitHub", "Hatchery GitHub App", "Pleroma GitHub", "Daedalus GitHub App", "Colony GitHub App", "Chatonome", "software factory GitHub App", "github-gitops apps", "GitHub Management via IaC", "Siit GitHub App request"];
const out = {};
for (const q of qs) {
  try {
    const r = await tools.notion["notion-search"]({accountId: acc, input: {query: q, page_size: 8, max_highlight_length: 160}});
    const raw = (r.content||[]).map(c=>c.text||"").join("\n");
    const j = JSON.parse(raw);
    out[q] = (j.results||[]).map(x => `${x.title} | ${x.url} | ${x.path||""} | ${x.timestamp} | ${(x.highlight||"").replace(/\*\*/g,"").slice(0,160)}`);
  } catch (e) { out[q] = "ERR " + e.message; }
}
return out;"#;
        assert_eq!(classify(&call("mcp__executor-v2__execute", json!({"code": notion}))), Classification::Pass);
        // More reported ones: a calendar lookup that computes free slots, and
        // a Notion fetch filtered by a regular expression naming "webhook".
        let calendar = r#"const emails = ["drake.thomsen@mistral.ai","maria.larsson@mistral.ai","helen.fotiou@mistral.ai","tom.churchill@mistral.ai","mohammed.khodr@mistral.ai","rainer.striedinger@mistral.ai","ary.sharif@mistral.ai","felipe.trigo@mistral.ai","karl.oppermann@mistral.ai","joana.langemeier@mistral.ai","jennifer.auvray@mistral.ai"];
const r = await tools.notion["notion-calendar-list-coworkers-events"]({ accountId: "acc_a798760a-9ce6-4a8e-8c3d-504cea91ebac", input: { coworkerEmails: emails, timeMin: "2026-11-19T00:00:00Z", timeMax: "2026-11-21T00:00:00Z", timeZone: "UTC" } });
const data = JSON.parse(r.content[0].text);
const busy = {}; const allDay = {}; const meta = {};
for (const c of data.coworkers) {
  const who = c.coworkerEmail.split("@")[0];
  busy[who] = []; allDay[who] = [];
  meta[who] = { eventCount: (c.events||[]).length, error: c.error || null };
  for (const e of (c.events || [])) {
    if (e.isTransparent || e.eventStatus === "cancelled") continue;
    if (e.selfResponseStatus === "declined" || e.responseStatus === "declined") continue;
    const p = e.period;
    if (p.type === "DATE") { allDay[who].push({ s: p.start.date, e: p.end.date, type: e.eventType, summary: e.eventType === "outOfOffice" ? "OOO" : (e.summary||"").slice(0,40) }); continue; }
    busy[who].push([Date.parse(p.start.dateTime), Date.parse(p.end.dateTime), e.eventType]);
  }
}
const slots = [];
for (let t = Date.parse("2026-11-19T07:00:00Z"); t <= Date.parse("2026-11-20T21:00:00Z"); t += 30*60000) {
  const end = t + 60*60000;
  const busyWho = Object.keys(busy).filter(w => busy[w].some(([s,e]) => s < end && e > t));
  slots.push({ utc: new Date(t).toISOString().slice(5,16), busyCount: busyWho.length, busy: busyWho });
}
const drakeFree = slots.filter(s => !s.busy.includes("drake.thomsen"));
return { meta, allDay, drakeFree };"#;
        let fetch = r#"const ids = ["3e26ba59-a7fe-81c9-8e02-dfbafb430bb2","3e26ba59-a7fe-8131-b4b7-ddda4e1c8ac9"];
const out = [];
for (const id of ids) {
  const r = await tools.notion["notion-fetch"]({accountId:"acc_a798760a-9ce6-4a8e-8c3d-504cea91ebac", input:{id}});
  const t = r.content[0].text;
  const re = /(plan|Enterprise|Growth|Launch|api key|API key|public api|Public API|routine|webhook|Workflows|Audiences|auto-delete|passthrough|Repeat|credits per|search-quota|quota|clay_scoped|routines create|runs start)/gi;
  const lines = t.split(/\\n|\n/).filter(l => re.test(l)).map(l => l.slice(0,400));
  out.push({id, n: lines.length, lines: lines.slice(0,40)});
}
return out;"#;
        for program in [calendar, fetch] {
            assert_eq!(classify(&call("mcp__executor-v2__execute", json!({"code": program}))), Classification::Pass);
        }
        for program in [
            "return await tools.notion['notion-fetch']({accountId: acc, input: {id}})",
            r#"return await tools[`notion`]?.[ "notion-search" ]({input: {query: "q"}})"#,
            r#"return await tools.mistral["slack"]["slack_read_channel"]({channel_id: "C1"})"#,
            r#"return deps["@slack/bolt"] ?? deps["lodash.merge"]"#,
        ] {
            assert!(!slack_write_in_code(program, false), "{program}");
        }
        for program in [
            r#"await tools.mistral["slack"]["slack_send_message"]({})"#,
            "await tools.mistral.slack?.[`slack_send_message`]({})",
            "await tools.ws['conversations_add_message']({})",
            r#"await tools.slack["chat-post-message"]({})"#,
            // Still computed: built, joined, or not a plain name.
            "await tools.mistral[`sl${v}`]({})",
            r#"await tools.mistral["notion", name]({})"#,
            r#"await tools.mistral["sl" + v]["x"]({})"#,
            r#"await tools.slack["get post"]({})"#,
            r#"await tools.slack["get/post_message"]({})"#,
            // An array literal is not a key, even after a keyword.
            r#"const m = tools.mistral; const ns = await ["slack"].find(Boolean); await m[ns]({})"#,
            r#"const e = globalThis["eval"]; await e(src)"#,
        ] {
            assert!(slack_write_in_code(program, false), "{program}");
        }
        // Executor `call` paths in the same form.
        let send = json!({"calls": {"path": "tools.ws[\"conversations_add_message\"]",
            "input": {"channel_id": "C1", "text": "x"}}});
        assert_eq!(ask("mcp__executor-v2__call", send).tool, "conversations_add_message");
        let search = json!({"calls": [{"path": "tools.notion[\"notion-search\"]", "input": {}}]});
        assert_eq!(classify(&call("mcp__executor-v2__call", search)), Classification::Pass);
    }

    #[test]
    fn slack_mentioned_as_data_is_not_a_slack_call() {
        for program in [
            r#"await tools.notion["notion-search"]({input: {query: "Slack integration"}})"#,
            r#"const page = await tools.search({query: "slack"}); return page.items"#,
            "// Read only; post the summary to Slack later, after slack_send_message is approved.\nreturn 1",
            r#"const slackResults = await tools.mistral.slack.slack_search_public({limit: 5}); return slackResults.content"#,
            r#"const m = "https://acme.slack.com/archives/C1/p1".match(/archives\/(\w+)/)"#,
            r#"await tools.notion["notion-connector-search"]({input: {sources: ["slack"]}})"#,
            r#"await linear__create_issue({description: `From Slack (#bugs): ${x}`})"#,
            // Slack-named variables and helpers, whatever their methods or nouns say.
            "const bySource = { slack: [], notion: [] }; bySource.slack.push(r)",
            "const slackLines = []; slackLines.push(x); slackIds.add(id); return slackLines.join('\\n')",
            r#"const slackPosts = rows.filter(r => r.channel === "slack-alerts"); return cfg.slack.alert_channel"#,
            "const searchSlack = q => tools.mistral.slack.slack_search_public({query: q}); return searchSlack('x')",
            // The Slack app's tools under another name, or spread over lines, used only to read.
            "const slack = tools.mistral.slack; return await slack.slack_read_channel({channel_id: 'C1'})",
            "const {\n  slack_read_thread,\n  slack_get_reactions, // emoji\n} = tools.mistral.slack;\nreturn slack_read_thread({})",
            "const { slack: s } = tools.mistral; return s.slack_read_channel({})",
            "const s = tools.mistral.slack // the connector\nreturn s.slack_read_channel({})",
            "return await tools.mistral\n  .slack?.slack_read_channel({channel_id: 'C1'})",
            "const t = tools; return t.notion.notion_fetch({id})",
            // Names that end in `eval`, `Function` or `tools`, and the words in prose.
            "return await tools.kb.hybrid_retrieval({query: 'q'})",
            "const isFunction = f => typeof f === 'function'; return isFunction(x)",
            "const toolsByApp = {}; toolsByApp[app] = 1; return foundTools[0]",
            "return (r.structuredContent?.tools ?? []).length + r.result.tools[0].length",
            r#"await tools.beeper.search_messages({query: "offline eval (mistral-large)"})"#,
            "try { x() } catch (e) { return `${e.constructor.name}: ${e.message}` }",
            "const typeName = v => v?.constructor?.name ?? typeof v; return xs.map(typeName)",
            "for (const k of ['constructor']) out.push(k)",
            // Names, keys and queries that only look like tools.
            "const n = tools.notion; const r = await n['notion-search']({query: 'Slack'}); return r.map((n) => n.title)",
            "return counts['slack_post_sent'] ?? x['slack_notify_channel']",
            "return await tools.notion['notion-search']({input: {query: 'slack-post-mortem'}})",
            r#"return await tools.bigquery.run_query({sql: "select * from slack_events"})"#,
            "const s = tools.mistral.slack; const f = s.slack_read_channel; return await f.call(null, {})",
            "const t = tools; const r = await t.linear.list_issues({}); return r.rows.map(t => t[0])",
            // Slack's tools under a name that also appears in text.
            "const s = tools.mistral.slack; console.log('slack read failed'); return await s.slack_read_channel({})",
            "const slack = tools.mistral.slack; return `${(await slack.slack_read_user_profile({})).name}'s time`",
            "const slack = tools.mistral?.slack ?? tools.slack; return await slack.slack_search_channels({})",
            "const slack: Record<string, (i: unknown) => Promise<any>> = tools.mistral.slack; return slack.slack_read_channel({})",
            // Checked, listed or named in text, not used.
            "if (!tools.mistral?.slack) throw new Error('tools.mistral.slack is not available'); return Object.keys(tools.mistral.slack)",
            r#"return await tools.search.describe({paths: ["tools.mistral.slack.slack_send_message"]})"#,
            r#"return await tools.mistral.github.search_code({q: "eval( language:javascript"})"#,
            r#"return await tools.bigquery.execute({query: "SELECT * FROM analytics.slack_messages"})"#,
            r#"const f = await tools.mistral.github.get_file_contents({path}); return f.text.replace(/x/g, "https://hooks.slack.com/services/<redacted>")"#,
            r#"const self = this; return "see this [link](x) and myself [y]""#,
            "const client = tools.github; return await client.list_issues({})",
        ] {
            assert!(!slack_write_in_code(program, false), "{program}");
        }
        for program in [
            // A Slack app's tools reached under a name that says nothing.
            "const s = tools.mistral.slack; await s.post_it({})",
            "const s = tools.mistral.slack; await s.frobnicate({})",
            "const s = tools.mistral.slack; return helper(s)",
            "const s = tools.mistral['slack']; await s.frobnicate({})",
            "const { slack: s } = tools.mistral; await s.frobnicate({})",
            "const { mistral: { slack } } = tools; await slack.frobnicate({})",
            "const { slack_read_thread, ...rest } = tools.mistral.slack; await rest.frobnicate({})",
            "const { slack: { frobnicate } } = tools.mistral; await frobnicate({})",
            "await tools.x.slack.slack_frobnicate({})",
            "const { slack_frobnicate } = tools.x.slack; await slack_frobnicate({})",
            // Names bound to `tools` are followed like `tools`.
            "const t = tools; await t[name]({})",
            "const m = tools.mistral; await m.slack.frobnicate({})",
            "const { mistral } = tools; const s = mistral.slack; await s.frobnicate({})",
            "const n = 'slack_sendMessage'; const t = tools.mistral.slack; await t[n]({})",
            // `tools` passed on, or Executor's dispatcher, in a program that mentions Slack.
            "const { items } = await tools.search({query: 'slack send'});\nconst fn = items[0].path.split('.').slice(1).reduce((o, k) => o[k], tools);\nawait fn({})",
            "const [, app, ns, tool] = path.split('.'); await Reflect.get(Reflect.get(Reflect.get(tools, app), ns), tool)({}) // slack",
            "const m = tools.mistral || {}; await m.slack[`slack_${op}`]({})",
            "let api; api = tools.mistral; await api.slack[op]({})",
            "const all = { ...tools }; await all.slack.post_frob({})",
            "const ctx = { tools }; await ctx.tools[name]({})",
            "await tools.executor.tools.call({path: {app: 'mistral'}, body: {tool: 'slack.slack_send_message', input}})",
            // Dispatchers that run a tool or an automation named after Slack.
            r#"await tools.mistral.studio.execute_workflow({workflow_identifier: "slack-todo-digest"})"#,
            r#"await tools.executor.schedules.runNow({path: {app: "slack-digest", name: "daily"}})"#,
            r#"await tools.mcp.call_tool({server: "slack", tool: "send_message", arguments})"#,
            r#"await tools.composio.execute({tool_slug: "SLACK_SEND_MESSAGE", arguments})"#,
            // A Slack Web API write or webhook sent raw.
            r#"await tools.mistral.studio.test_connector_http_call({connector: "slack_http", path: "/chat.postMessage", method: "POST"})"#,
            r#"await tools.http.request({url: "https://slack.com/api/chat.postMessage", method: "POST"})"#,
            r#"await tools.webhooks.post({url: "https://hooks.slack.com/services/T0/B0/X", body})"#,
            // Building a function from text.
            "await new Function (src)()",
            "await eval?.(src)",
            "await eval (src)",
            r#"await window["eval"](src)"#,
            "const AF = (async () => {}).constructor; await new AF(src)()",
            "const AF = (async () => {}).constructor\nawait new AF(src)()",
            "await (async () => {}).constructor.call(null, src)()",
            "if (a) b(); else eval (s)",
            // A call is not a check; text that only looks like a string is code.
            "const s = tools.mistral.slack; return !s.frobnicate({})",
            r#"const q = "\x22"; const s = tools.mistral.slack; await s.frobnicate({}); const r = "\x22";"#,
            r#"if (x) /"/.test(s) && tools.mistral.slack.frobnicate({})"#,
            // Dispatchers and HTTP clients reached under other names.
            "const api = tools.executor.tools; await api.call({body: {tool: 'slack.slack_send_message'}})",
            "const { call } = tools.executor.tools; await call({body: {tool: 'slack.slack_send_message'}})",
            "await tools.executor.tools.call.bind(null)({body: {tool: 'slack.slack_send_message'}})",
            "const { execute_workflow } = tools.mistral.studio; await execute_workflow({workflow_identifier: 'slack-digest'})",
            "await tools.mistral.studio.create_workflow_schedule({workflow_identifier: 'slack-standup-poster'})",
            r#"await tools.http.get({url: "https://slack.com/api/chat.postMessage", query: {channel: "C1", text: "x"}})"#,
            "let target; target = tools; for (const k of path) target = target[k]; await target({}) // slack",
            r#"await tools.http.get({url: "https://slack.com/api/chat.postMessage?channel=C1&text=x"})"#,
            r#"await tools.n8n.execute_workflow({query: {wait: "true"}, path: {workflowId: "slack-digest"}})"#,
            r#"await fetch("https://hooks.slack.com/services/T0/B0/X", {method: "POST"})"#,
            r#"const k = ["to", "ols"].join(""); const root = globalThis; await root[k].mistral.slack.send({})"#,
            // What hides code from a text scan: joins, continuations, escapes, line ends.
            "const text = `*PRs on ${day}*` + \"\\n\" + lines;\nawait tools.mistral.slack.slack_send_message({text})",
            "const text = \"Deploy finished. \\\nAll checks passed.\"; await tools.mistral.slack.slack_send_message({text})",
            r#"const slug = s => s.replace(/\x2f/g, "-"); await tools.mistral.slack.slack_send_message({})"#,
            "/*/ post it */ await tools.mistral.slack.send_message({text: `${a / b}`})",
            "// notify the team\rawait tools.mistral.slack.slack_send_message({})",
            r#"await tools.mistral.\u{0000000073}lack.send_message({})"#,
            // A name used for Slack's tools in one place and another app in another.
            "{ const client = tools.mistral.slack; await client.chat_post({}) }\n{ const client = tools.github; await client.add_labels({}) }",
        ] {
            assert!(slack_write_in_code(program, false), "{program}");
        }
        // A declaration far into a long program, after multi-byte text.
        let long = format!("// {}\nconst s = tools.mistral.slack; await s.send_message({{}})", "é".repeat(3000));
        assert!(slack_write_in_code(&long, false));
        // A backtick in another field does not hide the program.
        let fields = json!({"a": "`", "code": "await tools.mistral.slack.slack_send_message({})", "z": "`"});
        assert!(matches!(classify(&call("mcp__executor-v2__execute", fields)), Classification::Ask(_)));
        // The same reaches through Executor's `call`.
        for calls in [
            json!({"path": "tools.executor.tools.call", "input": {"body": {"tool": "slack.slack_send_message"}}}),
            json!({"path": "tools.mistral.studio.test_connector_http_call",
                "input": {"connector": "slack", "path": "/api/chat.postMessage", "method": "POST"}}),
        ] {
            assert!(matches!(classify(&call("mcp__executor-v2__call", json!({"calls": calls}))), Classification::Ask(_)));
        }
        let read = json!({"calls": {"path": "tools.mistral.studio.test_connector_http_call",
            "input": {"connector": "slack", "path": "/conversations.history", "method": "GET"}}});
        assert_eq!(classify(&call("mcp__executor-v2__call", read)), Classification::Pass);
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
