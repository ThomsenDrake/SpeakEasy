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
use std::collections::{BTreeSet, HashMap, HashSet};
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
                    let path = dot_constant_keys(c.get("path")?.as_str()?, false);
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
    let decoded = decode_escapes(code);
    // Join only at quotes that end and start strings: not at one in a
    // comment, a regular expression or another string, or an escaped one
    // (`/'+'/`, `'\' + '`), which would move where code starts.
    let (bare, _) = lex(&decoded);
    let b = bare.as_bytes();
    let skip = |mut k: usize| {
        while b.get(k).is_some_and(u8::is_ascii_whitespace) {
            k += 1;
        }
        k
    };
    let mut out = String::with_capacity(decoded.len());
    let mut copied = 0;
    let mut i = 0;
    while i < b.len() {
        if matches!(b[i], b'\'' | b'"' | b'`') {
            // A closing quote followed by `+` and an opening quote of the
            // same kind: join (a template joined to a string stays open).
            let plus = skip(i + 1);
            let open = skip(plus + 1);
            if b.get(plus) == Some(&b'+') && b.get(open) == Some(&b[i]) {
                out.push_str(&decoded[copied..i]);
                copied = open + 1;
                i = open + 1;
                continue;
            }
        }
        i += 1;
    }
    out.push_str(&decoded[copied..]);
    out
}

/// `\uXXXX` / `\u{…}` / `\xNN` escapes that spell a name, decoded.
fn decode_escapes(code: &str) -> String {
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
    // Not a member or private name spelled like it (`x.return`, `#of`), but
    // after a spread (`...typeof x`) or an integer's dot (`1. in x`).
    s.strip_suffix(word).is_some_and(|rest| {
        let code = rest.trim_end();
        let number = rest.len() > code.len()
            && code.strip_suffix('.').is_some_and(|n| {
                let digits = n.len() - n.trim_end_matches(|c: char| c.is_ascii_digit()).len();
                digits > 0 && !n[..n.len() - digits].ends_with(|c: char| is_ident_char(c) || c == '.')
            });
        !rest.ends_with(is_ident_char) && (number || !(code.ends_with(['.', '#']) && !code.ends_with("...")))
    })
}

/// The program with the text of its strings, regular expressions and
/// comments blanked out, offsets kept, so that names written as text
/// (`"tools.mistral.slack is not connected"`, `// eval(`) are not read as
/// code; the code inside a template's `${…}` stays. A quote or regular
/// expression still open at the end of its line was misread (only a
/// template, or a string continued with `\`, spans lines), so its text
/// stays code.
fn code_only(code: &str) -> String {
    lex(code).0
}

/// [`code_only`], and whether any text may be misread: a quote or regular
/// expression left open at the end of its line or of the program, or a `/`
/// that may as well start a regular expression as divide (after `}`, another
/// `/`, `++`, `--`, the `)` of an `if`, `for`, `while`, `with` or `switch`
/// head, `extends`, a private name or a word that may be a name, `of / 2`).
fn lex(code: &str) -> (String, bool) {
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
    let mut misread = false;
    // For each open `(`, whether it heads a statement (`if (`), and where
    // the last one that did closed.
    let mut heads: Vec<bool> = Vec::new();
    let mut head_closed = None;
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
                        // A regular expression where a value starts, else a
                        // division (also after a postfix `i++` or a member
                        // named like a keyword, `stats.in / n`).
                        let before = out.trim_end();
                        let keyword = VALUE_KEYWORDS.iter().any(|k| ends_with_word(before, k));
                        let operator = before.ends_with("++") || before.ends_with("--");
                        let postfix = operator && {
                            let operand = &before[..before.len() - 2];
                            let line = !operand[operand.trim_end().len()..].contains(['\n', '\r']);
                            let after_word = VALUE_KEYWORDS.iter().any(|k| ends_with_word(operand.trim_end(), k));
                            line && !after_word
                                && operand.trim_end().ends_with(|c: char| is_ident_char(c) || matches!(c, ')' | ']'))
                        };
                        let value = before.is_empty()
                            || before.ends_with("...")
                            || (!postfix
                                && before.ends_with([
                                    '(', ',', '=', ':', '[', '!', '&', '|', '?', '{', '}', ';', '+',
                                    '-', '*', '%', '<', '>', '~', '^',
                                ]))
                            || keyword;
                        if value {
                            mode = Mode::Regex { class: false };
                        }
                        let words = ["extends", "of", "yield", "await", "let", "async", "get", "set", "static"];
                        let private = before.rsplit(|c: char| !is_ident_char(c)).next().is_some_and(|w| {
                            !w.is_empty() && before[..before.len() - w.len()].ends_with('#')
                        });
                        misread |= before.ends_with(['}', '/'])
                            || operator
                            || private
                            || words.iter().any(|w| ends_with_word(before, w))
                            || (before.ends_with(')') && head_closed == Some(before.len()));
                    }
                    '(' => {
                        let before = out.trim_end();
                        let awaited = before.strip_suffix("await").filter(|_| ends_with_word(before, "await"));
                        let before = awaited.map_or(before, str::trim_end);
                        heads.push(["if", "for", "while", "with", "switch"].iter().any(|k| ends_with_word(before, k)));
                    }
                    ')' => {
                        if heads.pop() == Some(true) {
                            head_closed = Some(out.len() + 1);
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
                    // Its flags (`/x/g`) are text too, not a name; a name
                    // that is called (`/ 2 /sum(…)`) was a division's.
                    let flags: String =
                        chars.clone().map(|(_, f)| f).take_while(|&f| is_ident_char(f)).collect();
                    let called = code[i + 1 + flags.len()..].trim_start().starts_with('(');
                    if !flags.is_empty() && flags.chars().all(|f| "dgimsuvy".contains(f)) && !called {
                        flags.chars().for_each(|_| _ = chars.next());
                        out.push_str(&" ".repeat(flags.len()));
                    }
                } else {
                    blank(&mut out, c);
                }
            }
            Mode::BlockComment if c == '*' && next == Some('/') => {
                chars.next();
                out.push_str("  ");
                mode = Mode::Code;
            }
            // Misread: a quote or regular expression ends with its line
            // (a string may hold U+2028 and U+2029).
            Mode::Quote(_) | Mode::Regex { .. }
                if matches!(c, '\n' | '\r') || (line_end(c) && matches!(mode, Mode::Regex { .. })) =>
            {
                out.truncate(opened);
                out.push_str(&code[opened..i + c.len_utf8()]);
                mode = Mode::Code;
                misread = true;
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
        misread = true;
    }
    (out, misread)
}

/// Spell bracket keys that are one constant name as dotted names
/// (`tools.notion["notion-search"]` → `tools.notion.notion-search`):
/// Executor writes hyphenated tool names this way, so such a key is a
/// static name, not a computed one. Array literals (`= ["a"]`, `return
/// ["a"]`) and keys with other characters (`["get post"]`) stay. With
/// `underscores`, a key's `-` becomes `_`, so that a program's dotted names
/// stay names.
fn dot_constant_keys(code: &str, underscores: bool) -> String {
    // Only brackets of code: not one in a string, comment or regular expression.
    let bare = code_only(code);
    let mut out = String::with_capacity(code.len());
    let mut rest = code;
    while let Some(open) = rest.find('[') {
        let at = code.len() - rest.len() + open;
        out.push_str(&rest[..open]);
        rest = &rest[open + 1..];
        // Not across a line break: the space may end a `//` comment.
        let gap = &out[out.trim_end().len()..];
        let code_bracket = bare.as_bytes().get(at) == Some(&b'[');
        let indexes = code_bracket
            && indexes(out.trim_end())
            && !gap.contains(['\n', '\r', '\u{2028}', '\u{2029}']);
        let key = rest.trim_start();
        let literal = key.chars().next().filter(|q| matches!(q, '\'' | '"' | '`')).and_then(|q| {
            let (name, after) = key[1..].split_once(q)?;
            let after = after.trim_start().strip_prefix(']')?;
            // Not `["n-"]`: dotted, its `-` would read as an operator.
            let plain = !name.is_empty()
                && name.chars().all(|c| is_name_char(c) || c == '.')
                && !name.starts_with('-')
                && !name.ends_with('-');
            plain.then_some((name, after))
        });
        match literal {
            Some((name, after)) if indexes => {
                // `?.["x"]`, `?. ["x"]` → `?.x`
                out.truncate(out.trim_end().len());
                if !out.ends_with('.') {
                    out.push('.');
                }
                // In a program, `-` as `_` (same length): a dotted
                // `flags.use-const` must not read as `- const`.
                if underscores {
                    out.push_str(&name.replace('-', "_"));
                } else {
                    out.push_str(name);
                }
                rest = after;
            }
            _ => out.push('['),
        }
    }
    out.push_str(rest);
    out
}

/// Whether a `[` after `before` (trimmed) indexes a value (`x[k]`, `f()[k]`,
/// `x?.[k]`) rather than starting an array literal (`= [`, `return [`).
fn indexes(before: &str) -> bool {
    // `x.delete["y"]` is a member named like a keyword.
    let keyword = VALUE_KEYWORDS.iter().any(|k| {
        before.strip_suffix(k).is_some_and(|rest| !rest.ends_with(|c: char| is_name_char(c) || c == '.'))
    });
    !keyword && before.ends_with(|c: char| is_name_char(c) || matches!(c, '.' | ')' | ']'))
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
    // `import(…)` runs a module's code (`import("data:text/javascript,…")`).
    let imports = code.match_indices("import").any(|(i, m)| {
        !code[..i].ends_with(is_ident_char)
            && !code[..i].trim_end().ends_with('.')
            && code[i + m.len()..].trim_start().starts_with('(')
    });
    constructor
        || imports
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
/// name (`tools.x[name]`, `globalThis[name]`, `eval`, `Function`) asks,
/// unless it is a variable that only ever holds constant names (see
/// [`Constants`]): then each of those names is judged. A
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
    if ["globalThis", "global", "window", "this", "self"].into_iter().any(indexes_global) {
        return true;
    }
    let code = dot_constant_keys(&code, true);
    // Names are read in the code, Slack's name also in its strings.
    let (bare, misread) = lex(&code);
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
        || ["globalThis", "window", "self"].iter().any(|g| word(g, &taken) && !binds(&bare, g));
    let mut constants = Constants::new(&code, &bare, &idents, misread);
    while let Some((at, len)) = heads.pop() {
        // A name bound from one of its own uses (`const a = b` … `const b =
        // a`) leads back to a chain already judged.
        if !seen.insert(at) {
            continue;
        }
        let Some((end, spellings)) = spelled_chain(&bare, &mut constants, at + len) else {
            return true;
        };
        let chain = at..end;
        let head = bare[at..at + len].to_string();
        let bound = binding(&bare, &chain);
        // One name bound to several tools (`const fn = tools.notion[t]`) is
        // followed only when each of them reads.
        let reads = |spelled: &Vec<String>| {
            let mut names = sources.get(&head).cloned().unwrap_or_else(|| vec![head.clone()]);
            names.extend(spelled.iter().cloned());
            let other = |n: &String| is_slack_name(n) || has_word(n, &RAW_WORDS);
            write_verdict(chain_tool(&names)) == Some(false) && !names.iter().any(other)
        };
        if spellings.len() > 1 && bound.is_some() && !spellings.iter().all(reads) {
            return true;
        }
        let looked_at = inspected(&bare, &chain);
        let unbound = bound.is_none() && !looked_at;
        for spelled in spellings {
            let mut names = sources.get(&head).cloned().unwrap_or_else(|| vec![head.clone()]);
            names.extend(spelled);
            let tool = chain_tool(&names);
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
                Some(Binding::Name(declared)) => {
                    follow(&bare[declared.clone()], declared, names.clone())
                }
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

/// Whether a program binds `name` itself: declares it, or takes it as a
/// parameter (`(v, i, self) => …`, `function f(window) {`).
fn binds(code: &str, name: &str) -> bool {
    declares(code, name)
        || code.match_indices(name).any(|(i, m)| {
            let after = code[i + m.len()..].trim_start();
            if code[..i].ends_with(is_ident_char) || after.starts_with(is_ident_char) {
                return false;
            }
            let before = code[..i].trim_end();
            let listed = before.ends_with(['(', ',']) && after.starts_with([',', ')']);
            let params_end = after.find(')').map(|k| after[k + 1..].trim_start());
            after.starts_with("=>")
                || (listed && params_end.is_some_and(|rest| rest.starts_with("=>") || rest.starts_with('{')))
        })
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

/// Where the member chain that starts at `from` ends, and the names after
/// its head it can spell: one list for each value its computed keys can hold
/// (`tools.granola[tool]`, with `tool` only ever "get_meetings", spells
/// granola, get_meetings), or `None` when a key may hold anything else.
fn spelled_chain(
    bare: &str,
    constants: &mut Constants,
    from: usize,
) -> Option<(usize, Vec<Vec<String>>)> {
    let mut spellings = vec![Vec::new()];
    let mut start = from;
    // A few computed keys and names at most; longer chains are not followed.
    for _ in 0..=8 {
        let (end, computed) = member_chain(bare, start);
        let names = chain_names(&bare[start..end]);
        constants.spend(spellings.len() * (names.len() + 1))?;
        spellings.iter_mut().for_each(|s| s.extend(names.iter().cloned()));
        if spellings.iter().any(|s| s.len() > 64) {
            return None;
        }
        if !computed {
            // Each name of each spelling is judged.
            constants.spend(spellings.iter().map(|s| s.len() * 16).sum())?;
            return Some((end, spellings));
        }
        let rest = bare[end..].trim_start();
        let key = rest.strip_prefix("?.").unwrap_or(rest).trim_start();
        let (mut values, close) = constants.key(bare.len() - key.len())?;
        // No tool is named `undefined`, `null` or by an index: such a call
        // fails, so it spells nothing.
        values.retain(|v| !matches!(v.as_str(), "undefined" | "null") && !v.bytes().all(|b| b.is_ascii_digit()));
        if spellings.len() * values.len() > 64 {
            return None;
        }
        let size: usize = values.iter().map(|v| v.len() + 8).sum();
        constants.spend(spellings.len() * size)?;
        spellings = spellings
            .iter()
            .flat_map(|s| {
                values.iter().map(move |v| {
                    let mut s = s.clone();
                    s.extend(v.split('.').map(str::to_string));
                    s
                })
            })
            .collect();
        start = close + 1;
    }
    None
}

/// Compound assignments, which change a value to one that cannot be known.
const COMPOUND_ASSIGNMENTS: [&str; 15] = [
    "+=", "-=", "*=", "/=", "%=", "**=", "<<=", ">>=", ">>>=", "&=", "|=", "^=", "&&=", "||=", "??=",
];

/// Array methods whose callback's first parameter is each element in turn
/// (`["a", "b"].map((t) => …)`); the third is the array itself.
const ELEMENT_CALLBACKS: [&str; 10] = [
    "map", "forEach", "flatMap", "filter", "find", "findIndex", "findLast", "findLastIndex",
    "some", "every",
];

/// Array methods that leave an array's elements among those it had (not
/// `sort` or `reverse`, which return the array itself to be changed).
const KEEPING_METHODS: [&str; 14] = [
    "includes", "indexOf", "lastIndexOf", "join", "slice", "at", "concat", "toString", "keys",
    "values", "entries", "flat", "pop", "shift",
];

/// Functions that only read the array or object passed to them.
const READING_CALLS: [&str; 6] =
    ["JSON.stringify(", "new Set(", "Array.from(", "Object.keys(", "Object.values(", "Object.entries("];

/// The global object's names (`globalThis.t = x` sets a global `t`).
const GLOBAL_OBJECTS: [&str; 5] = ["globalThis", "global", "window", "self", "this"];

/// The longest name a constant tool name is read as; a longer one asks.
const LONGEST_NAME: usize = 128;

/// Words that are not variables, or not only (`yield` in a generator).
const RESERVED: [&str; 16] = [
    "yield", "await", "this", "super", "new", "typeof", "void", "delete", "in", "of", "instanceof",
    "null", "true", "false", "let", "async",
];

/// The constant names a program's variables can hold, so that a computed key
/// (`tools.granola[tool]`, `tools.notion[full ? "a" : "b"]`) can be read as
/// the names it stands for. A key holds constants when it is a constant name,
/// a conditional or `??`/`||` between such keys, a template of them, an
/// element of an array or a key of an object only ever holding constants
/// (`names[i]`, `TOOLS.search`), or a variable holding them: every place that
/// gives the variable a value gives one of these — its declarations whose
/// block, function or loop can reach the read (the innermost such scope hides
/// what is outside it), assignments, `for (const t of …)` and element
/// callbacks (`names.map((t, i) => …)`) over such an array, or the calls of a
/// named function it is a parameter of, which is only ever called directly or
/// handed to such an array's element callback. Anything else that binds or
/// changes it (a pattern, `catch`, `t += x`, a parameter of a function passed
/// on or of a method, `arguments`, `with`, `globalThis.t = x`, prototypes or
/// symbols for arrays and objects) leaves it unknown, as does a program that
/// needs more work to read than a fixed budget.
struct Constants<'a> {
    /// The program with its strings, for the text of the constants.
    code: &'a str,
    /// The same program as code only (see [`code_only`]), offsets kept.
    bare: &'a str,
    idents: &'a HashMap<&'a str, Vec<usize>>,
    /// Whether the text layer may have misread part of the program.
    misread: bool,
    /// Made when first needed; `None` inside when the program cannot be
    /// followed at all (text that may be misread, unpaired brackets, `with`,
    /// `arguments`, names or spaces outside ASCII, escapes in names,
    /// HTML-like comments, too long).
    brackets: Option<Option<Brackets>>,
    /// Whether the program may change how arrays and objects behave
    /// (`Array.prototype.map = …`, `Symbol.iterator`).
    patched: bool,
    /// What each place does with its name.
    places: HashMap<usize, Place>,
    /// The values each place gives, once read without meeting a cycle.
    given: HashMap<usize, Option<Vec<String>>>,
    /// The values of the places that can reach a read, by those places.
    merged: HashMap<Vec<usize>, Option<Vec<String>>>,
    /// Whether each read of an array or object variable leaves it as it was.
    reads: HashMap<(usize, bool), bool>,
    /// Where each function, loop or `catch` whose brackets open here ends.
    bodies: HashMap<usize, (Range<usize>, Exact)>,
    /// The places being read, against cycles (`g(tool)` inside `g`).
    visiting: Vec<usize>,
    /// The outermost of those a cycle led back to: a result read while a
    /// place further out is still being read is partial.
    cycle: Option<usize>,
    /// How deeply values are nested (`a ? (b ? …) : …`).
    nesting: usize,
    /// Work left; a program that needs more is not followed.
    budget: usize,
}

/// For each byte of a program, its innermost open bracket, and for each
/// open bracket where it closes (`u32::MAX`: none).
struct Brackets {
    inner: Vec<u32>,
    close: Vec<u32>,
}

impl Brackets {
    fn of(code: &str) -> Option<Brackets> {
        let mut inner = vec![u32::MAX; code.len()];
        let mut close = vec![u32::MAX; code.len()];
        let mut open: Vec<(usize, char)> = Vec::new();
        for (i, c) in code.char_indices() {
            // A closing bracket's innermost is its own opening one.
            inner[i] = open.last().map_or(u32::MAX, |&(o, _)| o as u32);
            match c {
                '(' | '[' | '{' => open.push((i, c)),
                ')' | ']' | '}' => {
                    let (o, opened) = open.pop()?;
                    if !matches!((opened, c), ('(', ')') | ('[', ']') | ('{', '}')) {
                        return None;
                    }
                    close[o] = i as u32;
                }
                _ => {}
            }
        }
        open.is_empty().then_some(Brackets { inner, close })
    }
}

/// A function parameter whose values are traced: its position, whether the
/// function takes at most an element and an index (so it may be an element
/// callback), and whether it has a default.
#[derive(Clone, Copy)]
struct Parameter {
    index: usize,
    simple: bool,
    defaulted: bool,
}

/// What one place where a name is written does with it.
#[derive(Clone)]
enum Place {
    /// Reads it, or is another name spelled the same (`x.t`, `{ t: 1 }`).
    Read,
    /// Declares a variable of that name, visible within the span (exactly,
    /// or at most).
    Binds(Range<usize>, Gives, Exact),
    /// Gives whichever variable of that name is visible there a value.
    Sets(Gives),
}

/// Whether a declaration's span is exactly where it is visible, so that it
/// hides the same name outside it.
type Exact = bool;

/// The value a place gives.
#[derive(Clone, Copy)]
enum Gives {
    /// One this cannot know.
    Unknown,
    /// None yet (`let t;`).
    Nothing,
    /// The value after the `=` that ends here.
    After(usize),
    /// The same for a `var`, which is `undefined` before it (hoisted).
    Hoisted(usize),
    /// Each element of the array written between these positions (`for (t
    /// of …)`).
    Each(usize, usize),
    /// What the function whose parameters these brackets hold is called with.
    Parameter(usize, usize),
    /// What an arrow function's only parameter (`t => …`) is called with.
    Arrow,
}

impl<'a> Constants<'a> {
    fn new(
        code: &'a str,
        bare: &'a str,
        idents: &'a HashMap<&'a str, Vec<usize>>,
        misread: bool,
    ) -> Self {
        Constants {
            code,
            bare,
            idents,
            misread,
            brackets: None,
            patched: true,
            places: HashMap::new(),
            given: HashMap::new(),
            merged: HashMap::new(),
            reads: HashMap::new(),
            bodies: HashMap::new(),
            visiting: Vec::new(),
            cycle: None,
            nesting: 0,
            budget: 2_000_000,
        }
    }

    /// Reads the program's brackets once; `None` when it cannot be followed.
    fn ready(&mut self) -> Option<()> {
        if self.brackets.is_none() {
            // `with (o)` brings names into scope unseen, and `arguments[0] = x`
            // changes a parameter.
            let variable = |word: &str, then: &dyn Fn(&str) -> bool| {
                self.idents.get(word).into_iter().flatten().any(|&i| {
                    let before = self.bare[..i].trim_end();
                    let after = self.bare[i + word.len()..].trim_start();
                    let key = after.starts_with(':') && before.ends_with(['{', ',']);
                    !before.ends_with('.') && !key && then(after)
                })
            };
            let unseen =
                variable("with", &|after| after.starts_with('(')) || variable("arguments", &|_| true);
            // Names and spaces outside ASCII (`toolé`, U+FEFF) and HTML-like
            // comments (`<!--`) are read differently by JavaScript.
            // So are escapes left in names (`\u0024tool`).
            let foreign = !self.bare.is_ascii()
                || self.bare.contains(['\\'])
                || self.bare.contains("<!--")
                || self.bare.contains("-->");
            let small = self.bare.len() <= 1 << 22;
            let followable = small && !unseen && !foreign && !self.misread;
            self.brackets = Some(if followable { Brackets::of(self.bare) } else { None });
            self.patched = self.prototypes();
        }
        self.brackets.as_ref()?.as_ref()?;
        Some(())
    }

    /// Whether the program names prototypes or symbols (`Array.prototype`,
    /// `__proto__`, `Symbol.iterator`), in code or as a name in a string
    /// (`x["__proto__"]`), not in prose ("MCP protocol").
    fn prototypes(&self) -> bool {
        // Also what reaches prototypes and symbols by reflection.
        let named = |s: &str| {
            let lower = s.to_ascii_lowercase();
            ["prototype", "__proto__", "symbol", "descriptor", "reflect", "ownkeys", "defineproperty", "getownproperty"]
                .iter()
                .any(|w| lower.contains(w))
        };
        if self.idents.keys().any(|name| named(name)) {
            return true;
        }
        // A string's text is blanked in `bare`, its quotes kept. Its value
        // may be spelled with escapes and line continuations
        // (`"prot\otype"`) or in parts (`` `proto${"type"}` ``).
        let b = self.bare.as_bytes();
        let mut k = 0;
        while k < b.len() {
            if matches!(b[k], b'"' | b'\'' | b'`') {
                let Some(len) = b[k + 1..].iter().position(|&c| c == b[k]) else { break };
                let end = k + 1 + len;
                let text: String = self.code[k + 1..end]
                    .chars()
                    .filter(|c| !matches!(c, '\\' | '\n' | '\r' | '$' | '{' | '}' | '"' | '\'' | '`'))
                    .collect();
                // A template's parts may join into a name (`` `${ "proto" }type` ``).
                let joined: String = text.chars().filter(|c| !c.is_whitespace()).collect();
                let lower = joined.to_ascii_lowercase();
                let template = b[k] == b'`' && (lower.contains("prototype") || lower.contains("__proto__"));
                if template || (!text.contains(char::is_whitespace) && named(&text)) {
                    return true;
                }
                // Look inside a template's `${…}` too.
                k = if b[k] == b'`' && !b[k + 1..end].iter().all(u8::is_ascii_whitespace) { k } else { end };
            }
            k += 1;
        }
        false
    }

    /// Whether the built-ins [`READING_CALLS`] trusts are left as they are:
    /// not declared again or given members (`JSON.stringify = …`).
    fn builtins_kept(&self) -> bool {
        ["JSON", "Object", "Array", "Set"].iter().all(|base| {
            self.idents.get(base).into_iter().flatten().all(|&i| {
                let before = self.bare[..i].trim_end();
                let after = self.bare[i + base.len()..].trim_start();
                // Only `JSON.stringify(…)`, `new Set(…)`: not declared, taken
                // as a value (`const J = JSON`) or given a member.
                let member = after.strip_prefix('.').is_some_and(|m| {
                    let rest = m.trim_start().trim_start_matches(is_ident_char).trim_start();
                    !assigns(rest) && !COMPOUND_ASSIGNMENTS.iter().any(|op| rest.starts_with(op))
                });
                let declared = keyword(before).is_some() || ends_with_word(before, "function");
                // Another object's member is another name, the global
                // object's is the built-in (`globalThis.JSON.stringify = …`).
                let other = member_of(before).is_some_and(|o| !GLOBAL_OBJECTS.iter().any(|g| ends_with_word(o, g)));
                other || (!declared && (member || after.starts_with('(')))
            })
        })
    }

    /// Whether the program names the global object as a value (`this`,
    /// `globalThis`, `window`, `self`, `global`), not only its members.
    fn globals(&mut self) -> Option<bool> {
        let idents = self.idents;
        'names: for g in GLOBAL_OBJECTS {
            let places = idents.get(g).map_or(&[][..], Vec::as_slice);
            // A program's own `window` or `self` (`(v, i, self) => …`) is not it.
            if g != "this" {
                for &i in places {
                    if matches!(self.place(g, i)?, Place::Binds(..)) {
                        continue 'names;
                    }
                }
            }
            for &i in places {
                let after = self.bare[i + g.len()..].trim_start();
                let value = !after.starts_with('.') || after.starts_with("...");
                // `this` in a function or a class is not the global object.
                if value && (g != "this" || self.top_level_this(i)?) {
                    return Some(true);
                }
            }
        }
        Some(false)
    }

    /// Whether `this` at `i` is outside every function (not an arrow's) and
    /// class.
    fn top_level_this(&mut self, i: usize) -> Option<bool> {
        let mut at = i;
        for _ in 0..64 {
            let Some((open, _)) = self.around(at) else { return Some(true) };
            if self.bare[open..].starts_with('{') {
                if self.class_body(open)? != Some(false) {
                    return Some(false);
                }
                let head = self.before(open)?;
                if let Some((params, _)) = head.len().checked_sub(1).filter(|_| head.ends_with(')')).and_then(|k| self.around(k)) {
                    let before = self.before(params)?;
                    if self.function_parameters(params, before, &self.bare[open..])? != Some(false) {
                        return Some(false);
                    }
                }
            }
            at = open;
        }
        Some(true)
    }

    /// Takes `work` from the budget; `None` once it runs out.
    fn spend(&mut self, work: usize) -> Option<()> {
        self.budget = self.budget.checked_sub(work)?;
        Some(())
    }

    /// The program before `i`, without the space at its end.
    fn before(&mut self, i: usize) -> Option<&'a str> {
        let before = self.bare[..i].trim_end();
        self.spend(1 + i - before.len())?;
        Some(before)
    }

    /// The program from `i`, without the space at its start.
    fn after(&mut self, i: usize) -> Option<&'a str> {
        let after = self.bare[i..].trim_start();
        self.spend(1 + self.bare.len() - i - after.len())?;
        Some(after)
    }

    /// Where the bracket that opens at `open` closes.
    fn closing(&self, open: usize) -> Option<usize> {
        let close = *self.brackets.as_ref()?.as_ref()?.close.get(open)?;
        (close != u32::MAX).then_some(close as usize)
    }

    /// The innermost bracket around byte `i`, as where it opens and closes.
    fn around(&self, i: usize) -> Option<(usize, usize)> {
        let open = *self.brackets.as_ref()?.as_ref()?.inner.get(i)?;
        (open != u32::MAX).then_some(open as usize).and_then(|o| Some((o, self.closing(o)?)))
    }

    /// The span of the innermost bracket around `i`, or the whole program.
    fn within(&self, i: usize) -> Range<usize> {
        self.around(i).map_or(0..self.bare.len(), |(open, close)| open..close + 1)
    }

    /// The constant names the computed key in the brackets at `open` can be,
    /// and where those brackets close.
    fn key(&mut self, open: usize) -> Option<(Vec<String>, usize)> {
        self.ready()?;
        let close = self.closing(open)?;
        let values = self.constant(open + 1..close)?;
        (!values.is_empty()).then_some((values, close))
    }

    /// Every value a variable `name` visible at `at` can be given, or `None`.
    fn values(&mut self, name: &str, at: usize) -> Option<Vec<String>> {
        let variable = name.starts_with(|c: char| c.is_ascii_alphabetic() || matches!(c, '_' | '$'))
            && name.chars().all(is_ident_char)
            && !RESERVED.contains(&name);
        if !variable {
            return None;
        }
        let idents = self.idents;
        let places = idents.get(name)?;
        self.spend(places.len())?;
        let mut reaching = Vec::new();
        // The innermost declaration whose span is exactly its scope: nothing
        // outside it can reach a read inside it.
        let mut innermost: Option<Range<usize>> = None;
        // Declarations whose span is exactly their scope but holds not the
        // read: what is set inside them is theirs (`for (…) { let tool; tool = x }`).
        let mut elsewhere: Vec<Range<usize>> = Vec::new();
        let mut sets = Vec::new();
        let mut declared = false;
        let mut surely = false;
        for &i in places {
            match self.place(name, i)? {
                Place::Read => {}
                Place::Binds(scope, gives, exact) if scope.contains(&at) => {
                    if exact && innermost.as_ref().is_none_or(|s| scope.len() < s.len()) {
                        innermost = Some(scope.clone());
                    }
                    declared = true;
                    surely |= exact;
                    reaching.push((i, gives));
                }
                Place::Binds(scope, _, true) => elsewhere.push(scope),
                Place::Binds(..) => {}
                Place::Sets(gives) => sets.push((i, gives)),
            }
        }
        self.spend(sets.len() * (elsewhere.len() + 1))?;
        reaching.extend(sets.into_iter().filter(|(i, _)| !elsewhere.iter().any(|s| s.contains(i))));
        // A name no declaration reaches is the global object's, which other
        // code can set (`Object.prototype.t = …`, `Object.assign(this, …)`),
        // as may be one only a span too wide reaches.
        if !declared || (!surely && self.globals()?) {
            return None;
        }
        if let Some(scope) = innermost {
            reaching.retain(|(i, _)| scope.contains(i));
        }
        let key: Vec<usize> = reaching.iter().map(|&(i, _)| i).collect();
        if let Some(known) = self.merged.get(&key).cloned() {
            self.spend(known.as_ref().map_or(1, Vec::len))?;
            return known;
        }
        let depth = self.visiting.len();
        let outer = self.cycle.take();
        let mut merged = BTreeSet::new();
        let mut known = Some(());
        for (i, gives) in reaching {
            let Some(values) = self.gives(name, i, gives) else {
                known = None;
                break;
            };
            self.spend(values.len())?;
            merged.extend(values);
            if merged.len() > 64 {
                known = None;
                break;
            }
        }
        let values = known.map(|_| merged.into_iter().collect::<Vec<_>>());
        // Complete unless a cycle led to a place still being read further out.
        if self.cycle.is_none_or(|c| c >= depth) {
            self.merged.insert(key, values.clone());
        }
        self.cycle = [outer, self.cycle.filter(|&c| c < depth)].into_iter().flatten().min();
        values
    }

    /// What the place at `i` gives `name`, read once.
    fn gives(&mut self, name: &str, i: usize, gives: Gives) -> Option<Vec<String>> {
        if let Some(known) = self.given.get(&i).cloned() {
            self.spend(known.as_ref().map_or(1, Vec::len))?;
            return known;
        }
        // A cycle adds nothing the places outside it do not give.
        if let Some(depth) = self.visiting.iter().position(|&v| v == i) {
            self.cycle = Some(self.cycle.map_or(depth, |c| c.min(depth)));
            return Some(Vec::new());
        }
        if self.visiting.len() >= 8 {
            return None;
        }
        let depth = self.visiting.len();
        self.visiting.push(i);
        let outer = self.cycle.take();
        let values = match gives {
            Gives::Unknown => None,
            Gives::Nothing => Some(vec!["undefined".to_string()]),
            Gives::After(at) => self.expression(at).and_then(|value| self.constant(value)),
            Gives::Hoisted(at) => self.expression(at).and_then(|value| self.constant(value)).map(|mut v| {
                v.push("undefined".to_string());
                v
            }),
            Gives::Each(from, to) => self.each(from, to),
            Gives::Parameter(open, close) => self.parameter(name, i, open, close),
            Gives::Arrow => {
                self.passed(i, true, Parameter { index: 0, simple: true, defaulted: false })
            }
        };
        self.visiting.pop();
        // A cycle back to this place itself is complete once it is read.
        if self.cycle.is_none_or(|c| c >= depth) {
            self.given.insert(i, values.clone());
        }
        self.cycle = [outer, self.cycle.filter(|&c| c < depth)].into_iter().flatten().min();
        values
    }

    /// What the place at `i` where `name` is written does with it, read once.
    fn place(&mut self, name: &str, i: usize) -> Option<Place> {
        if let Some(place) = self.places.get(&i) {
            return Some(place.clone());
        }
        let place = self.classify(name, i)?;
        self.places.insert(i, place.clone());
        Some(place)
    }

    fn classify(&mut self, name: &str, i: usize) -> Option<Place> {
        let bare = self.bare;
        let before = self.before(i)?;
        let after = self.after(i + name.len())?;
        // Another object's member (`x.tool`) or a private name is another
        // name, unless the object is the global one (`globalThis.tool = x`).
        if before.ends_with('#') {
            return Some(Place::Read);
        }
        if let Some(object) = member_of(before) {
            self.spend(before.len() - object.len())?;
            let global = GLOBAL_OBJECTS.iter().any(|g| ends_with_word(object, g));
            return Some(if global { Place::Sets(Gives::Unknown) } else { Place::Read });
        }
        // An object key (`{ tool: 1 }`; `{ tool: t } = x` binds `t`) or a label.
        let braced = self.around(i).is_some_and(|(open, _)| bare[open..].starts_with('{'));
        if braced && after.starts_with(':') && before.ends_with(['{', ',']) {
            return Some(Place::Read);
        }
        // Functions, classes and imports by this name, and values changed in place.
        let unstarred = before.strip_suffix('*').map_or(before, str::trim_end);
        if ["function", "class", "import"].iter().any(|k| ends_with_word(unstarred, k))
            || before.ends_with("++")
            || before.ends_with("--")
            || after.starts_with("++")
            || after.starts_with("--")
            || COMPOUND_ASSIGNMENTS.iter().any(|op| after.starts_with(op))
        {
            return Some(Place::Sets(Gives::Unknown));
        }
        // An arrow function's only parameter (`tool => …`).
        if after.starts_with("=>") {
            let (scope, exact) = self.arrow(i, bare.len() - after.len())?;
            return Some(Place::Binds(scope, Gives::Arrow, exact));
        }
        // Where it would bind in a parameter list or pattern (`(a, tool)`,
        // `[tool] =`, `{ a: tool = "x" }`), rather than be read in a default
        // or a computed key (`(a = tool)`, `{ [tool]: a }`).
        let binding = binding_position(before, after);
        // Whether the place binds within a pattern that is itself one
        // parameter, loop variable or caught value (`({ tool }) =>`).
        let mut nested = false;
        let mut at = i;
        let mut direct = true;
        for _ in 0..64 {
            let Some((open, close)) = self.around(at) else {
                return self.declared(i, before, after);
            };
            let head = self.before(open)?;
            let tail = self.after(close + 1)?;
            // Names in a list that binds (parameters, a caught value, a
            // pattern): a binding where one is written, read where a value
            // is, and unknown deeper in.
            let listed = |this: &mut Self, scope: (Range<usize>, Exact), gives: Gives| {
                if (direct && binding) || nested {
                    let gives = if direct { gives } else { Gives::Unknown };
                    Some(Place::Binds(scope.0, gives, scope.1))
                } else if direct {
                    this.declared(i, before, after)
                } else {
                    Some(Place::Sets(Gives::Unknown))
                }
            };
            if bare[open..].starts_with('(') {
                if ends_with_word(head, "catch") {
                    let scope = self.body(open, close)?;
                    return listed(self, scope, Gives::Unknown);
                }
                // A loop's head, unless a method named `for` (`class A { for(t) {} }`).
                let method = if loop_head(head) { self.member_list(open)? } else { Some(true) };
                if method.is_none() {
                    return Some(Place::Sets(Gives::Unknown));
                }
                if method == Some(false) {
                    let of = starts_with_word(after, "of");
                    if direct && (of || starts_with_word(after, "in")) {
                        let start = bare.len() - after.len() + 2;
                        let gives = if of { Gives::Each(start, close) } else { Gives::Unknown };
                        return Some(match keyword(before) {
                            Some(keyword) => {
                                let (scope, exact) = self.declared_over(i, keyword)?;
                                Place::Binds(scope, gives, exact)
                            }
                            None => Place::Sets(gives),
                        });
                    }
                } else {
                    match self.function_parameters(open, head, tail)? {
                        Some(true) => {
                            let scope = self.body(open, close)?;
                            return listed(self, scope, Gives::Parameter(open, close));
                        }
                        None => return Some(Place::Sets(Gives::Unknown)),
                        Some(false) => {}
                    }
                }
                // `(tool) = x` assigns it, like `tool = x`, and so does a
                // loop over it (`for ((tool) of …)`).
                let looped = (starts_with_word(tail, "of") || starts_with_word(tail, "in"))
                    && self.around(open).is_some_and(|(o, _)| self.bare[o..].starts_with('('));
                if assigns(tail) || looped || COMPOUND_ASSIGNMENTS.iter().any(|op| tail.starts_with(op)) {
                    return Some(Place::Sets(Gives::Unknown));
                }
                nested = false;
            } else if bare[open..].starts_with('[') && self.indexing(head)? {
                // `x[tool]` reads it (or `x[tool = "a"]` sets it), whatever
                // is done with `x[tool]`.
                return self.declared(i, before, after);
            } else if let Some(keyword) = keyword(head) {
                // `const { tool } = …`, `for (const [a, tool] of …)`: a pattern.
                let scope = self.declared_over(open, keyword)?;
                return listed(self, scope, Gives::Unknown);
            } else if assigns(tail) || starts_with_word(tail, "of") || starts_with_word(tail, "in") {
                // `[a, tool] = …`, `({ tool } = …)`, `for ([a, tool] of …)`.
                if direct && !binding {
                    return self.declared(i, before, after);
                }
                return Some(Place::Sets(Gives::Unknown));
            } else {
                // A pattern that is one whole parameter (`(a, { tool })`) or
                // a literal; the next bracket out tells.
                nested = direct && binding && head.ends_with(['(', ',']);
            }
            at = open;
            direct = false;
        }
        // Too deep to tell.
        Some(Place::Sets(Gives::Unknown))
    }

    /// Whether a `[` after `head` indexes a value (`x[k]`), not starts an
    /// array or a pattern after a keyword or a statement's head (`const [`,
    /// `if (c) [a, b] = …`).
    fn indexing(&mut self, head: &str) -> Option<bool> {
        if !indexes(head) || keyword(head).is_some() || ["else", "do"].iter().any(|k| ends_with_word(head, k)) {
            return Some(false);
        }
        if !head.ends_with(')') {
            return Some(true);
        }
        let Some((open, _)) = self.around(head.len() - 1) else { return Some(true) };
        let header = self.before(open)?;
        let statement = ["if", "while", "with", "switch"].iter().any(|k| ends_with_word(header, k));
        Some(!statement && !loop_head(header))
    }

    /// What a place outside any pattern, parameter list or loop head does:
    /// `const tool = "x"`, `tool = "x"`, `let a, tool: T = "x"`, `let tool;`,
    /// or a read.
    fn declared(&mut self, i: usize, before: &str, after: &str) -> Option<Place> {
        let keyword = keyword(before);
        let typed = after.strip_prefix(':').filter(|_| keyword.is_some() || before.ends_with(','));
        if let Some(typed) = typed {
            self.spend(typed.find(';').unwrap_or(typed.len()))?;
        }
        let rest = typed.map_or(after, past_type);
        let gives = match assigns(rest) {
            true if keyword == Some("var") => Gives::Hoisted(self.bare.len() - rest.len() + 1),
            true => Gives::After(self.bare.len() - rest.len() + 1),
            false => Gives::Nothing,
        };
        // A later declarator (`const app = "x", tool = "y"`).
        let keyword = match keyword {
            None if before.ends_with(',') => self.declarator(i)?,
            keyword => keyword,
        };
        Some(match keyword {
            Some(keyword) => {
                let (scope, exact) = self.declared_over(i, keyword)?;
                Place::Binds(scope, gives, exact)
            }
            None if assigns(rest) => Place::Sets(gives),
            None => Place::Read,
        })
    }

    /// The keyword of the declaration the declarator after a `,` at `i`
    /// belongs to (`let a = 1,\n  tool = "x"`), if the statement it is in
    /// surely starts with one: no line before it in the statement ends
    /// without a `,` (where a semicolon could have been left out).
    fn declarator(&mut self, i: usize) -> Option<Option<&'static str>> {
        let start = self.around(i).map_or(0, |(open, _)| open + 1);
        self.spend(i - start)?;
        let b = self.bare.as_bytes();
        let mut statement = start;
        // Whether a line in the statement ended without a `,`, and the last
        // character of code (a bracket stands for all it holds).
        let mut broken = false;
        let mut last = b';';
        let mut k = start;
        while k < i {
            match b[k] {
                b'(' | b'[' | b'{' => {
                    k = self.closing(k)?;
                    last = b[k];
                }
                b';' => {
                    statement = k + 1;
                    broken = false;
                    last = b';';
                }
                b'\n' | b'\r' => broken |= !matches!(last, b',' | b';'),
                c if !c.is_ascii_whitespace() => last = c,
                _ => {}
            }
            k += 1;
        }
        let text = self.bare[statement..i].trim_start();
        // A keyword followed by what it declares (not `let = 1`, `let.x`).
        let keyword = ["const", "let", "var"].into_iter().find(|k| {
            starts_with_word(text, k) && text[k.len()..].trim_start().starts_with(|c: char| is_ident_char(c) || matches!(c, '[' | '{'))
        });
        Some(keyword.filter(|_| !broken))
    }

    /// Where a variable declared with `keyword` at `at` is visible: its block
    /// (`let`, `const`; the whole loop for a loop head's), or everywhere
    /// (`var`, at most).
    fn declared_over(&mut self, at: usize, keyword: &str) -> Option<(Range<usize>, Exact)> {
        let whole = 0..self.bare.len();
        if keyword == "var" {
            return Some((self.function_around(at)?, false));
        }
        match self.around(at) {
            None => Some((whole, true)),
            Some((open, close)) if self.bare[open..].starts_with('{') => Some((open..close + 1, true)),
            Some((open, close)) if self.bare[open..].starts_with('(') => {
                let head = self.before(open)?;
                if loop_head(head) {
                    self.body(open, close)
                } else {
                    Some((whole, false))
                }
            }
            _ => Some((whole, false)),
        }
    }

    /// The body of the function around `at` (where a `var` is visible), or
    /// the whole program.
    fn function_around(&mut self, at: usize) -> Option<Range<usize>> {
        let mut at = at;
        for _ in 0..64 {
            let Some((open, close)) = self.around(at) else { break };
            if self.bare[open..].starts_with('{') {
                let head = self.before(open)?;
                let function = if head.ends_with("=>") || ends_with_word(head, "static") {
                    true
                } else if let Some(paren) = head.len().checked_sub(1).filter(|_| head.ends_with(')')) {
                    match self.around(paren) {
                        Some((params, _)) => {
                            let before = self.before(params)?;
                            self.function_parameters(params, before, &self.bare[open..])? == Some(true)
                        }
                        None => false,
                    }
                } else {
                    false
                };
                if function {
                    return Some(open..close + 1);
                }
            }
            at = open;
        }
        Some(0..self.bare.len())
    }

    /// From the brackets at `open` to the end of the function, loop or
    /// `catch` they head: its braced body exactly, an arrow's expression, or
    /// at most the bracket around them (a loop's statement, a return type).
    fn body(&mut self, open: usize, close: usize) -> Option<(Range<usize>, Exact)> {
        if let Some(known) = self.bodies.get(&open) {
            return Some(known.clone());
        }
        let body = self.body_of(open, close)?;
        self.bodies.insert(open, body.clone());
        Some(body)
    }

    fn body_of(&mut self, open: usize, close: usize) -> Option<(Range<usize>, Exact)> {
        let tail = self.after(close + 1)?;
        if tail.starts_with("=>") {
            let (end, exact) = self.arrow_end(open, self.bare.len() - tail.len())?;
            return Some((open..end, exact));
        }
        if tail.starts_with('{') {
            return Some((open..self.closing(self.bare.len() - tail.len())? + 1, true));
        }
        Some((self.within(open), false))
    }

    /// Where an arrow function's only parameter at `i` (before its `=>` at
    /// `arrow`) is visible.
    fn arrow(&mut self, i: usize, arrow: usize) -> Option<(Range<usize>, Exact)> {
        let (end, exact) = self.arrow_end(i, arrow)?;
        Some((i..end, exact))
    }

    /// Where the arrow function starting at `start`, whose `=>` is at
    /// `arrow`, ends: after its braced body; or, for an expression body,
    /// before the `,` or bracket that ends it, exactly when the function is
    /// an argument or element (`map((t) => t.text)`; inside a conditional's
    /// branch the end could be a `:`), else at most the bracket around it.
    fn arrow_end(&mut self, start: usize, arrow: usize) -> Option<(usize, Exact)> {
        let body = self.after(arrow + 2)?;
        let at = self.bare.len() - body.len();
        if body.starts_with('{') {
            return Some((self.closing(at)? + 1, true));
        }
        let head = self.before(start)?;
        let head = match head.strip_suffix("async").filter(|h| !h.ends_with(is_ident_char)) {
            Some(rest) => self.before(rest.len())?,
            None => head,
        };
        // In arguments, a grouping or an array (not a block or a loop's head).
        let enclosing = self.around(start).filter(|&(open, _)| {
            matches!(self.bare.as_bytes()[open], b'(' | b'[')
        });
        let header = match enclosing {
            Some((open, _)) if self.bare[open..].starts_with('(') => loop_head(self.before(open)?),
            _ => false,
        };
        let Some((_, close)) = enclosing.filter(|_| head.ends_with(['(', ',', '[']) && !header) else {
            let within = self.within(start);
            return Some((within.end, false));
        };
        let b = self.bare.as_bytes();
        let mut k = at;
        while k < close {
            match b[k] {
                b'(' | b'[' | b'{' => k = self.closing(k)?,
                b',' | b';' => break,
                _ => {}
            }
            k += 1;
        }
        self.spend(k - at)?;
        Some((k, true))
    }

    /// Whether the parentheses at `open` (between `head` and `tail`) hold a
    /// function's parameters (`function g(…)`, `(…) =>`, a method's `run(…)
    /// {` in a class or object literal) rather than arguments or a
    /// condition (`g(…)`, `if (…) {`, a call before a block); `None` inside
    /// when this cannot tell (`(t): T =>`, `g<T>(t) {`).
    fn function_parameters(&mut self, open: usize, head: &str, tail: &str) -> Option<Option<bool>> {
        if tail.starts_with("=>") {
            return Some(Some(true));
        }
        // A name too long to be a keyword is not one.
        let Some(word) = trailing_word(head, 64) else {
            return Some((!tail.starts_with([':', '{'])).then_some(false));
        };
        let lead = &head[..head.len() - word.len()];
        let rest = lead.trim_end();
        self.spend(head.len() - word.len() - rest.len())?;
        let rest = rest.strip_suffix('*').map_or(rest, str::trim_end);
        // The keyword, not a member named like it (`obj.function(t)`).
        let keyword = word == "function" && member_of(lead.trim_end()).is_none() && !rest.ends_with('#');
        if keyword || (!word.is_empty() && ends_with_word(rest, "function")) {
            return Some(Some(true));
        }
        if !tail.starts_with([':', '{']) {
            return Some(Some(false));
        }
        // `(t): T =>`, `async (t): T =>`, `<T>(t) {`.
        if word.is_empty() || word == "async" {
            return Some(None);
        }
        match self.member_list(open)? {
            // A method where a member starts (`{ run(t) {`, `, async run(t) {`,
            // a new line in a class), not a call in a member's value
            // (`{ a: c ? g(t) : null }`).
            Some(true) => {
                let slot = rest.is_empty()
                    || rest.ends_with(['{', ',', ';', '}', '*'])
                    || ["async", "static", "get", "set"].iter().any(|k| ends_with_word(rest, k))
                    || lead[rest.len()..].contains(['\n', '\r']);
                Some(Some(slot))
            }
            Some(false) => Some(Some(false)),
            None => Some(None),
        }
    }

    /// Whether the bracket around `at` is a class body or an object literal,
    /// where `name(…) {` defines a method; `None` inside when this cannot
    /// tell (`class A extends mix(B) {`, a `{` after an unusual word).
    fn member_list(&mut self, at: usize) -> Option<Option<bool>> {
        match self.around(at).filter(|&(o, _)| self.bare[o..].starts_with('{')) {
            Some((brace, _)) => self.literal_brace(brace, 0),
            None => Some(Some(false)),
        }
    }

    /// Whether the `{` at `brace` opens an object literal or a class body
    /// (not a block or a function's body); `None` inside when this cannot
    /// tell.
    fn literal_brace(&mut self, brace: usize, depth: usize) -> Option<Option<bool>> {
        if depth > 16 {
            return Some(None);
        }
        // `class A {`, `class A extends mix(B, { x: 1 }) {`.
        match self.class_body(brace)? {
            Some(true) => return Some(Some(true)),
            None => return Some(None),
            Some(false) => {}
        }
        let head = self.before(brace)?;
        // A function's body after a TypeScript return type (`): Promise<T> {`).
        if self.return_type(head)? {
            return Some(Some(false));
        }
        // A value in a template (`${ {…} }`) or a loop's head (`for (…; {…}; …)`).
        let in_parens = self.around(brace).is_some_and(|(o, _)| self.bare[o..].starts_with('('));
        if head.ends_with("${") || (head.ends_with(';') && in_parens) {
            return Some(Some(true));
        }
        // A block: at the start, after a statement, a block, a head's `)`,
        // `=>`, `else`, `do`, `try`, `catch`, `finally` or `static`.
        let block = head.is_empty()
            || head.ends_with([';', '{', '}', ')'])
            || head.ends_with("=>")
            || ["else", "do", "try", "catch", "finally", "static"].iter().any(|k| ends_with_word(head, k));
        if block {
            return Some(Some(false));
        }
        // A value: after an operator that cannot end an expression, or a
        // word a value follows on the same line (`return\n{` is a block).
        // After `++`, `--`, `/`, `<` or `>` (a regular expression's end, a
        // type) this cannot tell.
        let line = !self.bare[head.len()..brace].contains(['\n', '\r']);
        let words = ["return", "await", "yield", "typeof", "void", "throw", "in", "of", "new", "delete"];
        let ends = head.ends_with("++") || head.ends_with("--") || head.ends_with(['/', '<', '>']);
        let value = head.ends_with(['=', '(', ',', '[', '?', '|', '&', '!', '+', '-', '*', '%', '~', '^'])
            || (line && words.iter().any(|k| ends_with_word(head, k)));
        if value && !ends {
            return Some(Some(true));
        }
        if head.ends_with(':') {
            // A property's value (in an object literal, or in brackets) or
            // a conditional's branch; else a label's or a `case`'s block.
            let parent = self.around(brace);
            let object = match parent {
                Some((p, _)) if !self.bare[p..].starts_with('{') => return Some(Some(true)),
                Some((p, _)) => self.literal_brace(p, depth + 1)?,
                None => Some(false),
            };
            return Some(match object {
                Some(true) => Some(true),
                Some(false) if !self.conditional(parent.map_or(0, |(p, _)| p + 1), brace)? => Some(false),
                _ => None,
            });
        }
        Some(None)
    }

    /// Whether `head` ends with a TypeScript return type after a parameter
    /// list (`): Promise<void>`, `)?: Record<K, V>`).
    fn return_type(&mut self, head: &str) -> Option<bool> {
        let b = head.as_bytes();
        let mut angle = 0i32;
        let mut named = false;
        let mut k = b.len();
        while k > 0 && b.len() - k < 512 {
            match b[k - 1] {
                b'>' => angle += 1,
                b'<' => angle -= 1,
                b':' if angle == 0 => {
                    self.spend(b.len() - k)?;
                    let before = head[..k - 1].trim_end();
                    return Some(named && (before.ends_with(')') || before.ends_with(")?")));
                }
                c if is_ident_char(c as char) => named = true,
                c if c.is_ascii_whitespace() || matches!(c, b'.' | b'[' | b']' | b'|' | b'&' | b',' | b'?') => {}
                _ => return Some(false),
            }
            k -= 1;
        }
        Some(false)
    }

    /// Whether the `{` at `brace` opens a class's body: back over names,
    /// dots, `extends` and whole brackets to the keyword `class` (not a
    /// method named `class`); `None` inside when this cannot tell.
    fn class_body(&mut self, brace: usize) -> Option<Option<bool>> {
        let b = self.bare.as_bytes();
        let mut at = brace;
        // Whether the token just after the one being read is a `(…)`.
        let mut called = false;
        for _ in 0..256 {
            while at > 0 && b[at - 1].is_ascii_whitespace() {
                at -= 1;
            }
            let Some(&c) = at.checked_sub(1).and_then(|k| b.get(k)) else {
                return Some(Some(false));
            };
            if matches!(c, b')' | b']') {
                let (open, _) = self.around(at - 1)?;
                called = c == b')';
                at = open;
                continue;
            }
            if c == b'}' {
                return Some(None);
            }
            if !(is_ident_char(c as char) || c == b'.') {
                return Some(Some(false));
            }
            let end = at;
            while at > 0 && (is_ident_char(b[at - 1] as char) || b[at - 1] == b'.') {
                at -= 1;
            }
            let word = &self.bare[at..end];
            self.spend(end - at + 1)?;
            if ["if", "for", "while", "switch", "catch", "with", "function", "return", "else", "do", "try"]
                .contains(&word)
            {
                return Some(Some(false));
            }
            if word == "class" {
                let before = self.bare[..at].trim_end();
                // A class in the heritage (`extends class {}`), or a member
                // named `class` (`class() {`, `x.class`, `#class`).
                if ends_with_word(before, "extends") {
                    called = false;
                    continue;
                }
                return Some(Some(!called && !before.ends_with(['.', '#'])));
            }
            called = false;
        }
        Some(None)
    }

    /// Whether the statement before `end` (from `start`, the start of its
    /// block) has a conditional's `?` at its own level.
    fn conditional(&mut self, start: usize, end: usize) -> Option<bool> {
        self.spend(end - start)?;
        let b = self.bare.as_bytes();
        let mut found = false;
        let mut k = start;
        while k < end {
            match b[k] {
                b'(' | b'[' | b'{' => k = self.closing(k)?,
                b';' => found = false,
                b'c' | b'd' if !self.bare[..k].ends_with(is_ident_char)
                    && (starts_with_word(&self.bare[k..], "case") || starts_with_word(&self.bare[k..], "default")) =>
                {
                    found = false
                }
                b'?' if b.get(k + 1) == Some(&b'?') => k += 1,
                b'?' if b.get(k + 1) == Some(&b'.') && !b.get(k + 2).is_some_and(u8::is_ascii_digit) => {}
                b'?' => found = true,
                _ => {}
            }
            k += 1;
        }
        Some(found)
    }

    /// The values a function's parameter can hold: its default, and what
    /// every call of the function passes there.
    fn parameter(&mut self, name: &str, i: usize, open: usize, close: usize) -> Option<Vec<String>> {
        let bare = self.bare;
        self.spend(close - open)?;
        let list = entries(bare, open + 1, close);
        let p = list.iter().position(|r| r.contains(&i))?;
        // `Map<K, V>` before it would split where no parameter ends.
        if bare[open + 1..list[p].end].contains('<') {
            return None;
        }
        let entry = bare[list[p].clone()].trim_start();
        let rest = |r: &Range<usize>| bare[r.clone()].trim_start().starts_with("...");
        // TypeScript's `this` parameter takes no argument (`g(this: T, tool)`).
        let this = |r: &Range<usize>| starts_with_word(bare[r.clone()].trim_start(), "this");
        if list[p].end - entry.len() != i || list[..=p].iter().any(|r| rest(r) || this(r)) {
            return None;
        }
        // `tool`, `tool?: T`, `tool = "x"`, `tool: T = "x"`.
        let after = bare[i + name.len()..list[p].end].trim_start();
        let after = after.strip_prefix('?').unwrap_or(after).trim_start();
        let after = if after.starts_with(':') { past_type(after) } else { after };
        let defaulted = assigns(after);
        let mut values = if defaulted {
            let at = list[p].end - after.len() + 1;
            self.constant(at..list[p].end)?
        } else if after.is_empty() {
            Vec::new()
        } else {
            return None;
        };
        let arrow = bare[close + 1..].trim_start().starts_with("=>");
        let simple = list.len() <= 2 && !list.iter().any(rest) && !bare[open..close].contains('<');
        let parameter = Parameter { index: p, simple, defaulted };
        values.extend(self.passed(open, arrow, parameter)?);
        Some(values)
    }

    /// The values parameter `p` of the function whose parameters start at
    /// `start` takes: what every call passes there, for a named function
    /// that is only ever called (`function g(…)`, `const g = (…) =>`); each
    /// element, for a callback of an array method, when the function takes at
    /// most the element and index parameters (`simple`).
    fn passed(&mut self, start: usize, arrow: bool, parameter: Parameter) -> Option<Vec<String>> {
        let Parameter { index: p, simple, .. } = parameter;
        let before = self.before(start)?;
        let word = trailing_word(before, 256)?;
        let unasync = |s: &'a str| {
            let s = s.trim_end();
            s.strip_suffix("async").filter(|r| !r.ends_with(is_ident_char)).map_or(s, str::trim_end)
        };
        if !arrow && word != "function" {
            // `function g(`, `async function* g(`: a declaration, not a
            // named function expression (`const f = function g(`) or a method.
            let rest = before[..before.len() - word.len()].trim_end();
            let rest = rest.strip_suffix('*').map_or(rest, str::trim_end);
            let lead = rest.strip_suffix("function").filter(|r| !r.ends_with(is_ident_char))?;
            let lead = lead.trim_end().strip_suffix("async").filter(|r| !r.ends_with(is_ident_char));
            let lead = lead.unwrap_or(&rest[..rest.len() - "function".len()]);
            self.spend(before.len() - lead.trim_end().len())?;
            // A declaration stands in a block, not in brackets (`for (…; function g() {}(…);)`).
            let blocked = self.around(lead.len()).is_none_or(|(o, _)| self.bare[o..].starts_with('{'));
            if word.is_empty() || !statement_starts(lead) || !blocked {
                return None;
            }
            return self.called_with(word, before.len() - word.len(), parameter);
        }
        // `const g = async (…) =>`, `const g = function (…)`, or a callback.
        let expression = if arrow { before } else { &before[..before.len() - word.len()] };
        let expression = unasync(expression);
        self.spend(before.len() - expression.len())?;
        let assigned = expression.strip_suffix('=').filter(|lhs| {
            !lhs.ends_with(['=', '!', '<', '>', '+', '-', '*', '/', '%', '&', '|', '^', '?'])
        });
        let Some(lhs) = assigned.map(str::trim_end) else {
            return if simple && p == 0 { self.callback(expression.len()) } else { None };
        };
        self.spend(expression.len() - lhs.len())?;
        let function = trailing_word(lhs, 256)?;
        let declared = lhs[..lhs.len() - function.len()].trim_end();
        if function.is_empty()
            || function.starts_with(|c: char| c.is_ascii_digit())
            || keyword(declared).is_none()
        {
            return None;
        }
        self.called_with(function, lhs.len() - function.len(), parameter)
    }

    /// What every call of `function` (declared at `declared`) passes as
    /// argument `p`, when it is only ever called (`g(…)`, `g?.(…)`, `new g(…)`)
    /// or, taking at most two parameters (`simple`), handed to an element
    /// callback of an array only ever holding constants (`names.map(g)`):
    /// not passed on otherwise, bound, or declared again. A use inside a
    /// scope that declares another variable of its name is that variable's.
    fn called_with(&mut self, function: &str, declared: usize, parameter: Parameter) -> Option<Vec<String>> {
        let Parameter { index: p, simple, defaulted } = parameter;
        let bare = self.bare;
        let idents = self.idents;
        let places = idents.get(function)?;
        self.spend(places.len())?;
        // Spans where the name is another variable's (`runs.map((run) => run.title)`).
        let mut others = Vec::new();
        for &i in places {
            if let Place::Binds(scope, _, true) = self.place(function, i)? {
                if i != declared && !scope.contains(&declared) {
                    others.push(scope);
                }
            }
        }
        self.spend(places.len() * others.len())?;
        let mut values = Vec::new();
        for &j in places {
            if j == declared || others.iter().any(|scope| scope.contains(&j)) {
                continue;
            }
            let before = self.before(j)?;
            let after = self.after(j + function.len())?;
            if before.ends_with('#') {
                continue;
            }
            if let Some(object) = member_of(before) {
                if GLOBAL_OBJECTS.iter().any(|g| ends_with_word(object, g)) {
                    return None;
                }
                continue;
            }
            let braced = self.around(j).is_some_and(|(open, _)| bare[open..].starts_with('{'));
            if braced && after.starts_with(':') && before.ends_with(['{', ',']) {
                continue;
            }
            if ends_with_word(before, "function") {
                return None;
            }
            let call = after.strip_prefix("?.").map_or(after, str::trim_start);
            if !call.starts_with('(') {
                // `names.map(g)`: the element, then its index (not a name).
                let handed = simple && before.ends_with('(') && after.starts_with([')', ',']);
                match p {
                    0 if handed => values.extend(self.callback(j)?),
                    1 if handed => {
                        self.callback(j)?;
                        values.push("0".to_string());
                    }
                    _ => return None,
                }
                continue;
            }
            let open = bare.len() - call.len();
            let close = self.closing(open)?;
            self.spend(close - open)?;
            let arguments = entries(bare, open + 1, close);
            let spread = |r: &Range<usize>| bare[r.clone()].trim_start().starts_with("...");
            // `make<A, B>()` would split where no argument ends.
            let generic = arguments.get(p).is_some_and(|r| bare[open..r.end].contains('<'));
            if arguments.iter().take(p + 1).any(spread) || generic {
                return None;
            }
            match arguments.get(p).filter(|r| !bare[(*r).clone()].trim().is_empty()) {
                Some(argument) => values.extend(self.constant(argument.clone())?),
                // A missing argument is `undefined`, or the default.
                None if !defaulted => values.push("undefined".to_string()),
                None => {}
            }
            values.sort();
            values.dedup();
            if values.len() > 64 {
                return None;
            }
        }
        Some(values)
    }

    /// The elements of the array whose method the callback after `head` (the
    /// position just before the callback) is the first argument of
    /// (`["a", "b"].map(`, `names.forEach(`).
    fn callback(&mut self, head: usize) -> Option<Vec<String>> {
        let call = self.before(head)?.strip_suffix('(')?.trim_end();
        let method = trailing_word(call, 64)?;
        if !ELEMENT_CALLBACKS.contains(&method) {
            return None;
        }
        let receiver = call[..call.len() - method.len()].trim_end().strip_suffix('.')?;
        let receiver = receiver.strip_suffix('?').unwrap_or(receiver).trim_end();
        self.spend(call.len() - receiver.len())?;
        if receiver.ends_with(']') {
            let (open, _) = self.around(receiver.len() - 1)?;
            // After `}` (an object, `{…}["a"]`), a template or a word that
            // may be a variable (`of["a"]`) it may index too.
            let head = self.before(open)?;
            let maybe = ["of", "yield", "await", "let", "async"].iter().any(|k| ends_with_word(head, k));
            if indexes(head) || head.ends_with(['}', '`']) || maybe {
                return None;
            }
            return self.each(open, receiver.len());
        }
        let name = trailing_word(receiver, 256)?;
        if name.is_empty() || receiver[..receiver.len() - name.len()].ends_with(['.', '#']) {
            return None;
        }
        self.each(receiver.len() - name.len(), receiver.len())
    }

    /// The names each element of the array that is all the text from `from`
    /// to `to` can be: an array literal (`["a", name]`), or a variable that
    /// only ever holds one and is only read (see [`Constants::array`]).
    fn each(&mut self, from: usize, to: usize) -> Option<Vec<String>> {
        if self.patched {
            return None;
        }
        let bare = self.bare;
        self.spend(to - from)?;
        let text = &bare[from..to];
        let open = from + text.len() - text.trim_start().len();
        let array = text.trim();
        let end = self.uncast(open, open + array.len())?.unwrap_or(open + array.len());
        if !array.starts_with('[') {
            return self.array(&bare[open..end], open);
        }
        if self.closing(open)? + 1 != end {
            return None;
        }
        let mut values = BTreeSet::new();
        let elements = entries(bare, open + 1, end - 1);
        for (n, element) in elements.iter().enumerate() {
            let text = bare[element.clone()].trim();
            if text.starts_with("...") {
                return None;
            }
            if !text.is_empty() {
                values.extend(self.constant(element.clone())?);
                if values.len() > 64 {
                    return None;
                }
            } else if n + 1 < elements.len() {
                // A hole (`[, "a"]`).
                values.insert("undefined".to_string());
            }
        }
        Some(values.into_iter().collect())
    }

    /// The elements of the array a variable `name` visible at `at` holds,
    /// when every declaration of it there gives an array literal, nothing
    /// sets it, and every place it is used only reads it (see
    /// [`Constants::only_read`]).
    fn array(&mut self, name: &str, at: usize) -> Option<Vec<String>> {
        self.literal(name, at, true, |this, value| this.each(value.start, value.end))
    }

    /// The value of key `key` of the object a variable `name` visible at
    /// `at` holds, when every declaration of it there gives an object literal
    /// of plain keys whose values are data (no functions or getters), nothing
    /// sets it, and every place it is used only reads it.
    fn object(&mut self, name: &str, key: &str, at: usize) -> Option<Vec<String>> {
        if self.patched {
            return None;
        }
        self.literal(name, at, false, |this, value| {
            let mut values = Vec::new();
            let mut found = false;
            for (k, v) in this.members(value)? {
                if k == key {
                    found = true;
                    values.extend(this.constant(v)?);
                }
            }
            found.then_some(values)
        })
    }

    /// What `read` gives for each declaration of `name` visible at `at`
    /// that gives an array (`array`) or object literal, when nothing sets
    /// the variable and every place it is used only reads it.
    fn literal(
        &mut self,
        name: &str,
        at: usize,
        array: bool,
        mut read: impl FnMut(&mut Self, Range<usize>) -> Option<Vec<String>>,
    ) -> Option<Vec<String>> {
        let idents = self.idents;
        let places = idents.get(name)?;
        self.spend(places.len())?;
        let mut values = Vec::new();
        let mut given = false;
        for &i in places {
            match self.place(name, i)? {
                Place::Read if self.only_read(name, i, array)? => {}
                Place::Binds(scope, ..) if !scope.contains(&at) => {}
                // Before a `var`'s value, reading it throws (`undefined[i]`).
                Place::Binds(_, Gives::After(eq) | Gives::Hoisted(eq), _) => {
                    let value = self.expression(eq)?;
                    let end = self.uncast(value.start, value.end)?.unwrap_or(value.end);
                    let open = if array { '[' } else { '{' };
                    if !self.bare[value.start..].starts_with(open) || self.closing(value.start)? + 1 != end {
                        return None;
                    }
                    given = true;
                    values.extend(read(self, value.start..end)?);
                    if values.len() > 64 {
                        return None;
                    }
                }
                _ => return None,
            }
        }
        given.then_some(values)
    }

    /// The plain keys of the object literal written over `value` and where
    /// each value is written; `None` when it has anything else (a spread, a
    /// computed key, `__proto__`, a method, a getter, a function or a call).
    fn members(&mut self, value: Range<usize>) -> Option<Vec<(String, Range<usize>)>> {
        let bare = self.bare;
        self.spend(value.len())?;
        let inner = value.start + 1..value.end - 1;
        if bare[inner.clone()].contains(['(', '`']) || bare[inner.clone()].contains("=>") {
            return None;
        }
        let mut members = Vec::new();
        for entry in entries(bare, inner.start, inner.end) {
            let text = bare[entry.clone()].trim_start();
            if text.trim().is_empty() {
                continue;
            }
            let start = entry.end - text.len();
            let (key, rest) = match text.chars().next()? {
                q @ ('"' | '\'') => {
                    let len = text[1..].find(q)?;
                    (self.code[start + 1..start + 1 + len].to_string(), start + len + 2)
                }
                _ => {
                    let key = leading_word(text, 256)?;
                    (key.to_string(), start + key.len())
                }
            };
            let plain = !key.is_empty() && key.chars().all(|c| is_name_char(c) || c == '.');
            if !plain || key == "__proto__" {
                return None;
            }
            let rest = bare[rest..entry.end].trim_start();
            let value = match rest.strip_prefix(':') {
                Some(v) => entry.end - v.len()..entry.end,
                // `{ search }` is `{ search: search }`.
                None if rest.trim().is_empty() && !text.starts_with(['"', '\'']) => start..start + key.len(),
                None => return None,
            };
            // Only data: a value that is a function could change the
            // object when called as its method (`(TOOLS.m)()`).
            let scalar = bare[value.clone()].trim();
            let data = scalar.parse::<f64>().is_ok()
                || ["true", "false", "null"].contains(&scalar)
                || self.constant(value.clone()).is_some();
            if !data {
                return None;
            }
            members.push((key, value));
        }
        Some(members)
    }

    /// Whether the read of the array (`array`) or object variable `name` at
    /// `i` leaves it holding what it was declared with.
    fn only_read(&mut self, name: &str, i: usize, array: bool) -> Option<bool> {
        if let Some(&known) = self.reads.get(&(i, array)) {
            return Some(known);
        }
        let read = self.reads_only(name, i, array)?;
        self.reads.insert((i, array), read);
        Some(read)
    }

    fn reads_only(&mut self, name: &str, i: usize, array: bool) -> Option<bool> {
        let bare = self.bare;
        let before = self.before(i)?;
        let after = self.after(i + name.len())?;
        // A member of another object, or a key, is another name.
        if member_of(before).is_some() {
            return Some(true);
        }
        if ends_with_word(before, "delete") {
            return Some(false);
        }
        // The one argument of a function that only reads it
        // (`JSON.stringify(TOOLS)`, not `my.JSON.stringify(` or a replacer).
        let reading = READING_CALLS.iter().any(|f| {
            before.strip_suffix(f).is_some_and(|r| !r.ends_with(|c: char| is_ident_char(c) || c == '.'))
        }) && self.builtins_kept();
        let spread = before.ends_with("...") && !self.in_pattern(i)?;
        if (reading && after.starts_with(')')) || spread || self.returned(i, before, after)? {
            return Some(true);
        }
        // `for (… of names)`, `for (const k in TOOLS)`.
        if (ends_with_word(before, "of") || ends_with_word(before, "in")) && after.starts_with(')') {
            return Some(true);
        }
        // Changed by assignment, as a loop's or a pattern's target, or by a
        // method called by a computed name (`names[m](x)`).
        let changes = |this: &mut Self, s: &str| {
            let target = |s: &str| {
                assigns(s)
                    || s.starts_with("++")
                    || s.starts_with("--")
                    || COMPOUND_ASSIGNMENTS.iter().any(|op| s.starts_with(op))
                    || starts_with_word(s, "of")
                    || starts_with_word(s, "in")
            };
            // Also in parentheses (`(TOOLS.read) = x`, `for ((names[0]) of …)`).
            let grouped = match this.around(i) {
                Some((open, close)) if s.starts_with(')') && this.bare[open..].starts_with('(') => {
                    let tail = this.after(close + 1)?;
                    target(tail)
                }
                _ => false,
            };
            Some(target(s) || grouped || this.in_pattern(i)?)
        };
        // Called as a method, also through parentheses or `?. (`
        // (`names[m](x)`, `(names[m])(x)`), with the array as `this`.
        let calls = |this: &mut Self, s: &str| {
            let call = |s: &str| {
                s.starts_with(['(', '`'])
                    || s.strip_prefix("?.").is_some_and(|r| r.trim_start().starts_with('('))
            };
            let grouped = match this.around(i) {
                Some((open, close)) if s.starts_with(')') && this.bare[open..].starts_with('(') => {
                    let tail = this.after(close + 1)?;
                    call(tail)
                }
                _ => false,
            };
            Some(call(s) || grouped)
        };
        // `names[i]`, `TOOLS?.[k]`.
        let after = after.strip_prefix("?.").map(str::trim_start).filter(|a| a.starts_with('[')).unwrap_or(after);
        if after.starts_with('[') {
            let close = self.closing(bare.len() - after.len())?;
            let rest = self.after(close + 1)?;
            return Some(!calls(self, rest)? && !changes(self, rest)?);
        }
        let Some(member) = after.strip_prefix("?.").or_else(|| after.strip_prefix('.')) else {
            return Some(false);
        };
        let member = member.trim_start();
        let method = leading_word(member, 256)?;
        let rest = self.after(bare.len() - member.len() + method.len())?;
        if !array {
            // `TOOLS.search`, not `TOOLS.search = x` or `TOOLS.run()`.
            return Some(!changes(self, rest)? && !calls(self, rest)?);
        }
        if method == "length" {
            return Some(!changes(self, rest)?);
        }
        let Some(arguments) = rest.strip_prefix('(') else {
            return Some(false);
        };
        if KEEPING_METHODS.contains(&method) {
            return Some(true);
        }
        // A callback that is not given the array itself.
        if !ELEMENT_CALLBACKS.contains(&method) {
            return Some(false);
        }
        let open = bare.len() - rest.len();
        let callback = arguments.trim_start();
        let callback = callback.strip_prefix("async").map_or(callback, str::trim_start);
        let function = callback.strip_prefix("function");
        let callback = function.map_or(callback, |f| {
            f.trim_start().trim_start_matches(['*', ' ']).trim_start_matches(is_ident_char).trim_start()
        });
        self.spend(rest.len() - callback.len())?;
        if !callback.starts_with('(') {
            let param = leading_word(callback, 256)?;
            let rest = callback[param.len()..].trim_start();
            if param.is_empty() {
                return Some(false);
            }
            // `t => …`, or a function by name (`names.map(run)`).
            if rest.starts_with("=>") {
                return Some(true);
            }
            if function.is_none() && rest.starts_with([')', ',']) {
                return self.takes_two(param);
            }
            return Some(false);
        }
        let params = bare.len() - callback.len();
        let close = self.closing(params).filter(|&c| c < self.closing(open).unwrap_or(0))?;
        let list = entries(bare, params + 1, close);
        let rest_param = list.iter().any(|r| bare[r.clone()].trim_start().starts_with("..."));
        // `(t, i) => …` or `function (t) {…}`, not a function passed in parentheses.
        let body = self.after(close + 1)?;
        let head = if function.is_some() { body.starts_with('{') } else { body.starts_with("=>") };
        Some(head && list.len() <= 2 && !rest_param)
    }

    /// Whether `name` is a function declared once, taking at most an element
    /// and an index (`function run(tool) {…}`, `const run = async (t, i) =>
    /// …`), and not set elsewhere.
    fn takes_two(&mut self, name: &str) -> Option<bool> {
        let bare = self.bare;
        let idents = self.idents;
        let mut declared = false;
        for &i in idents.get(name)? {
            let before = self.before(i)?;
            let after = self.after(i + name.len())?;
            if member_of(before).is_some() || before.ends_with('#') {
                continue;
            }
            let unstarred = before.strip_suffix('*').map_or(before, str::trim_end);
            let params = if ends_with_word(unstarred, "function") {
                bare.len() - after.len()
            } else if keyword(before).is_some() && assigns(after) {
                let value = self.after(bare.len() - after.len() + 1)?;
                let value = value.strip_prefix("async").filter(|v| !v.starts_with(is_ident_char)).map_or(value, str::trim_start);
                let value = match value.strip_prefix("function").filter(|v| !v.starts_with(is_ident_char)) {
                    Some(v) => v.trim_start().trim_start_matches(['*', ' ']).trim_start_matches(is_ident_char).trim_start(),
                    None => value,
                };
                let param = leading_word(value, 256)?;
                if !param.is_empty() && value[param.len()..].trim_start().starts_with("=>") {
                    // `const run = t => …`
                    if declared {
                        return Some(false);
                    }
                    declared = true;
                    continue;
                }
                bare.len() - value.len()
            } else {
                // A use: called or handed on, not set or bound again (`[run] = …`).
                match self.place(name, i)? {
                    Place::Read => continue,
                    _ => return Some(false),
                }
            };
            if declared || !bare[params..].starts_with('(') {
                return Some(false);
            }
            declared = true;
            let close = self.closing(params)?;
            self.spend(close - params)?;
            let list = entries(bare, params + 1, close);
            let rest = list.iter().any(|r| bare[r.clone()].trim_start().starts_with("..."));
            if list.len() > 2 || rest || bare[params..close].contains('<') {
                return Some(false);
            }
        }
        Some(declared)
    }

    /// Whether the place at `i` is (inside object and array literals) what
    /// the program returns at its top level (`return { names, out }`), after
    /// which nothing in it runs.
    fn returned(&mut self, i: usize, before: &str, after: &str) -> Option<bool> {
        let value = before.ends_with(['{', ',', '[', ':']) || ends_with_word(before, "return");
        if !value || !(after.is_empty() || after.starts_with([',', '}', ']', ';'])) {
            return Some(false);
        }
        // The returned value ends the statement (`return [names][0].push(…)` does not).
        let mut at = i;
        let mut head = before;
        let mut rest = after;
        for _ in 0..16 {
            let Some((open, close)) = self.around(at) else {
                let ends = rest.is_empty() || rest.starts_with(';');
                return Some(ends && ends_with_word(head, "return"));
            };
            // No function defined in it, nor a member that runs after the
            // return (`then`, `toJSON`: `{ list: names, then: later }`).
            self.spend(close - open)?;
            let text = &self.bare[open..close];
            let named = ["then", "toJSON", "toString", "valueOf", "function", "get", "set"].iter().any(|w| {
                text.match_indices(w).any(|(k, _)| {
                    !text[..k].ends_with(is_ident_char) && !text[k + w.len()..].starts_with(is_ident_char)
                })
            });
            let method = text.match_indices(')').any(|(k, _)| text[k + 1..].trim_start().starts_with('{'));
            if named || method || text.contains("=>") {
                return Some(false);
            }
            rest = self.after(close + 1)?;
            head = self.before(open)?;
            let literal = match &self.bare[open..open + 1] {
                "[" => !indexes(head),
                "{" => head.ends_with(['{', ',', '[', ':']) || ends_with_word(head, "return"),
                _ => false,
            };
            if !literal {
                return Some(false);
            }
            at = open;
        }
        Some(false)
    }

    /// Whether the place at `i` is in a destructuring pattern (`[names[0]] =
    /// x`, `({ a: TOOLS.k } = x)`, `for ([x] of …)`), which sets it.
    fn in_pattern(&mut self, i: usize) -> Option<bool> {
        let mut at = i;
        for _ in 0..16 {
            let Some((open, close)) = self.around(at) else { return Some(false) };
            let head = self.before(open)?;
            let literal = match &self.bare[open..open + 1] {
                "[" => !self.indexing(head)?,
                "{" => true,
                _ => return Some(false),
            };
            if !literal {
                return Some(false);
            }
            let tail = self.after(close + 1)?;
            if assigns(tail) || starts_with_word(tail, "of") || starts_with_word(tail, "in") {
                return Some(true);
            }
            at = open;
        }
        Some(true)
    }

    /// Where the expression after `at` is written: up to its `;`, `,` or
    /// closing bracket, or the end of its line when the next line starts a
    /// statement (`= "get"\nconst r = …`).
    fn expression(&mut self, at: usize) -> Option<Range<usize>> {
        let text = self.after(at)?;
        let start = self.bare.len() - text.len();
        let mut depth = 0i32;
        let mut end = text.len();
        // Whether the code so far ends a value (a name, literal or bracket).
        let mut ended = false;
        let mut chars = text.char_indices().peekable();
        while let Some((k, c)) = chars.next() {
            match c {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' | ';' | ',' if depth == 0 => {
                    end = k;
                    break;
                }
                ')' | ']' | '}' => depth -= 1,
                '\n' | '\r' | '\u{2028}' | '\u{2029}' if depth == 0 && ended => {
                    while chars.next_if(|&(_, n)| n.is_whitespace()).is_some() {}
                    let next = &text[chars.peek().map_or(text.len(), |&(n, _)| n)..];
                    let operator = ["in", "instanceof", "as", "satisfies"].iter().any(|w| starts_with_word(next, w));
                    if next.starts_with(is_ident_char) && !operator {
                        end = k;
                        break;
                    }
                    continue;
                }
                _ => {}
            }
            if !c.is_whitespace() {
                ended = is_ident_char(c) || matches!(c, ')' | ']' | '}' | '"' | '\'' | '`');
            }
        }
        self.spend(end)?;
        let value = text[..end].trim_end();
        Some(start..start + value.len())
    }

    /// The names the value written over `range` can be (see [`Constants`]).
    fn constant(&mut self, range: Range<usize>) -> Option<Vec<String>> {
        if self.nesting >= 32 {
            return None;
        }
        self.nesting += 1;
        let values = self.value(range);
        self.nesting -= 1;
        values.filter(|v| v.len() <= 64)
    }

    fn value(&mut self, range: Range<usize>) -> Option<Vec<String>> {
        let bare = self.bare;
        let text = &bare[range.clone()];
        let value = text.trim();
        self.spend(1 + text.len() - value.len())?;
        let start = range.end - text.trim_start().len();
        let end = start + value.len();
        if value.is_empty() {
            return None;
        }
        // `c ? a : b`, `a ?? b`, `a || b`: one of them.
        if let Some((one, other)) = self.split(start, end)? {
            let mut values = self.constant(one)?;
            values.extend(self.constant(other)?);
            return Some(values);
        }
        // `x as const`, `x satisfies T`.
        if let Some(cast) = self.uncast(start, end)? {
            return self.constant(start..cast);
        }
        if value.starts_with('(') && self.closing(start) == Some(end - 1) {
            return self.constant(start + 1..end - 1);
        }
        if let Some(quote) = value.chars().next().filter(|q| matches!(q, '"' | '\'' | '`')) {
            return self.text(start, end, quote);
        }
        let name = leading_word(value, 256)?;
        let rest = value[name.len()..].trim_start();
        if name.is_empty() || name.starts_with(|c: char| c.is_ascii_digit()) {
            return None;
        }
        if rest.is_empty() {
            // `null`, and `undefined` unless the program declares its own.
            if name == "null" || (name == "undefined" && !declares(self.bare, "undefined")) {
                return Some(vec![name.to_string()]);
            }
            return self.values(name, start);
        }
        // `names[i]`: any element, or `undefined` out of range.
        if rest.starts_with('[') && self.closing(end - rest.len())? + 1 == end {
            let mut values = self.array(name, start)?;
            values.push("undefined".to_string());
            return Some(values);
        }
        // `TOOLS.search`.
        let member = rest.strip_prefix("?.").or_else(|| rest.strip_prefix('.')).map(str::trim_start)?;
        let key = leading_word(member, 256)?;
        if key.is_empty() || key.len() != member.len() {
            return None;
        }
        self.object(name, key, start)
    }

    /// The names a string or template written from `start` to `end` can be:
    /// its text, with each `${…}` one of the names it can be.
    fn text(&mut self, start: usize, end: usize, quote: char) -> Option<Vec<String>> {
        let bare = self.bare;
        if end - start < 2 || !bare[..end].ends_with(quote) {
            return None;
        }
        let mut names = vec![String::new()];
        let mut k = start + 1;
        while k < end - 1 {
            let next = match bare[k..end - 1].find("${").filter(|_| quote == '`') {
                Some(n) => k + n,
                None => end - 1,
            };
            // The text between: blanked in `bare`, so no quote of its own.
            if bare[k..next].contains(quote) {
                return None;
            }
            let part = &self.code[k..next];
            self.spend(names.len() * (part.len() + 1))?;
            names.iter_mut().for_each(|n| n.push_str(part));
            if names.iter().any(|n| n.len() > LONGEST_NAME) {
                return None;
            }
            if next == end - 1 {
                break;
            }
            let close = self.closing(next + 1)?;
            let values = self.constant(next + 2..close)?;
            if names.len() * values.len() > 64 {
                return None;
            }
            names = names.iter().flat_map(|n| values.iter().map(move |v| format!("{n}{v}"))).collect();
            self.spend(names.iter().map(String::len).sum())?;
            if names.iter().any(|n| n.len() > LONGEST_NAME) {
                return None;
            }
            k = close + 1;
        }
        self.spend(names.iter().map(String::len).sum())?;
        let plain = |n: &String| {
            !n.is_empty() && n.len() <= LONGEST_NAME && n.chars().all(|c| is_name_char(c) || c == '.')
        };
        names.iter().all(plain).then_some(names)
    }

    /// The two values a conditional or `??`/`||` written from `start` to
    /// `end` chooses between (`c ? a : b`, `a ?? b`), if it is one; `None`
    /// when the work runs out.
    fn split(&mut self, start: usize, end: usize) -> Option<Option<(Range<usize>, Range<usize>)>> {
        self.spend(end - start)?;
        let b = self.bare.as_bytes();
        let mut depth = 0i32;
        let mut question = None;
        let mut either = None;
        // Conditionals inside the first branch (`a ? b ? c : d : e`).
        let mut inner = 0;
        let mut k = start;
        while k < end {
            let next = b.get(k + 1).copied();
            match b[k] {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth -= 1,
                _ if depth != 0 => {}
                // An arrow function (`t => c ? a : b`) or an assignment is
                // not the value either side gives.
                b'=' if next == Some(b'>') => return Some(None),
                b'=' if next != Some(b'=') && !matches!(b[k.saturating_sub(1)], b'=' | b'!' | b'<' | b'>') => {
                    return Some(None);
                }
                b'?' if next == Some(b'?') && b.get(k + 2) != Some(&b'=') => {
                    either.get_or_insert(k..k + 2);
                    k += 1;
                }
                b'|' if next == Some(b'|') && b.get(k + 2) != Some(&b'=') => {
                    either.get_or_insert(k..k + 2);
                    k += 1;
                }
                // `?.` (not `?.5`) is not a conditional.
                b'?' if next == Some(b'.') && !b.get(k + 2).is_some_and(u8::is_ascii_digit) => {}
                b'?' => match question {
                    None => question = Some(k),
                    Some(_) => inner += 1,
                },
                b':' => match question {
                    Some(q) if inner == 0 => return Some(Some((q + 1..k, k + 1..end))),
                    Some(_) => inner -= 1,
                    None => {}
                },
                _ => {}
            }
            k += 1;
        }
        Some(either.filter(|_| question.is_none()).map(|op| (start..op.start, op.end..end)))
    }

    /// Where the value written from `start` to `end` ends without a trailing
    /// TypeScript cast (`x as const`, `x as Tool`, `x satisfies T`), if it has
    /// one; `None` when the work runs out.
    fn uncast(&mut self, start: usize, end: usize) -> Option<Option<usize>> {
        self.spend(end - start)?;
        let bare = self.bare;
        let b = bare.as_bytes();
        let mut depth = 0i32;
        let mut cast = None;
        for k in start..end {
            match b[k] {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth -= 1,
                b'a' | b's' if depth == 0 && k > start && b[k - 1].is_ascii_whitespace() => {
                    let word = ["as", "satisfies"].into_iter().find(|w| starts_with_word(&bare[k..end], w));
                    if let Some(word) = word {
                        let rest = &bare[k + word.len()..end];
                        let typed = rest.len() <= 256
                            && rest.starts_with(char::is_whitespace)
                            && !rest.contains(['(', ')', '=', '"', '\'', '`']);
                        if typed {
                            cast = Some(k);
                        }
                    }
                }
                _ => {}
            }
        }
        Some(cast.map(|k| start + bare[start..k].trim_end().len()))
    }
}

/// The object whose member a name after `before` is (`x` of `x.tool`,
/// `x?.tool`), not a spread's or a number's (`...tool`, `1.\ntool = x`).
fn member_of(before: &str) -> Option<&str> {
    let object = before.strip_suffix('.').filter(|_| !before.ends_with("..."))?;
    let object = object.trim_end_matches('?').trim_end();
    let number = trailing_word(object, 256).is_none_or(|w| w.starts_with(|c: char| c.is_ascii_digit()));
    (!number || object.is_empty()).then_some(object)
}

/// Whether a name between `before` and `after` stands where a parameter
/// list or pattern binds one (`(a, tool)`, `{ a: tool = "x" }`, `[...tool]`).
fn binding_position(before: &str, after: &str) -> bool {
    let start = before.ends_with(['(', ',', '{', '[', ':']) || before.ends_with("...");
    let end = after.is_empty() || after.starts_with([',', ')', '}', ']', ':', '?']) || assigns(after);
    start && end
}

/// `const`, `let` or `var` at the end of `before`.
fn keyword(before: &str) -> Option<&'static str> {
    ["const", "let", "var"].into_iter().find(|k| ends_with_word(before, k))
}

/// Whether `s` starts with the word `word` (`of x`, not `offset`).
fn starts_with_word(s: &str, word: &str) -> bool {
    s.strip_prefix(word).is_some_and(|rest| !rest.starts_with(is_ident_char))
}

/// The identifier at the end of `s` (empty when there is none), or `None`
/// when it is longer than `max`.
fn trailing_word(s: &str, max: usize) -> Option<&str> {
    let len = s.bytes().rev().take(max + 1).take_while(|&b| is_ident_char(b as char)).count();
    (len <= max).then(|| &s[s.len() - len..])
}

/// The identifier at the start of `s` (empty when there is none), or `None`
/// when it is longer than `max`.
fn leading_word(s: &str, max: usize) -> Option<&str> {
    let len = s.bytes().take(max + 1).take_while(|&b| is_ident_char(b as char)).count();
    (len <= max).then(|| &s[..len])
}

/// Whether brackets after `head` are a loop's head (`for (`, `for await (`).
fn loop_head(head: &str) -> bool {
    let head = head.strip_suffix("await").filter(|_| ends_with_word(head, "await")).map_or(head, str::trim_end);
    ends_with_word(head, "for")
}

/// Whether `rest` (trimmed) starts by giving a value (`= x`), not comparing
/// (`==`) or starting an arrow's body (`=>`).
fn assigns(rest: &str) -> bool {
    rest.starts_with('=') && !rest.starts_with("==") && !rest.starts_with("=>")
}

/// `: T = …` past its type, as the text from the `=` that gives the value,
/// or nothing when it gives none (`let t: string;`).
fn past_type(typed: &str) -> &str {
    let statement = &typed[..typed.find(';').unwrap_or(typed.len())];
    let b = statement.as_bytes();
    (0..b.len())
        .find(|&k| {
            b[k] == b'='
                && !matches!(b.get(k + 1), Some(b'=' | b'>'))
                && !(k > 0 && matches!(b[k - 1], b'=' | b'!' | b'<' | b'>'))
        })
        .map_or("", |k| &typed[k..])
}

/// Whether a statement starts after `before`: at the start, after `;`, `{`
/// or `}`, or on a new line after a value (`x\nfunction g() {}`, not
/// `return\nfunction g() {}`).
fn statement_starts(before: &str) -> bool {
    let code = before.trim_end();
    let new_line = before[code.len()..].contains(['\n', '\r', '\u{2028}', '\u{2029}']);
    code.is_empty()
        || code.ends_with([';', '}'])
        // Not a template's `${`.
        || (code.ends_with('{') && !code.ends_with("${"))
        || (new_line
            && code.ends_with(|c: char| is_ident_char(c) || matches!(c, ')' | ']' | '"' | '\'' | '`' | '/'))
            && !VALUE_KEYWORDS.iter().any(|k| ends_with_word(code, k))
            && !ends_with_word(code, "extends"))
}

/// The comma-separated entries from `start` to `end` (parameters,
/// arguments, elements), as their spans; none when there is only space.
fn entries(code: &str, start: usize, end: usize) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut from = start;
    for (k, c) in code[start..end].char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                out.push(from..start + k);
                from = start + k + 1;
            }
            _ => {}
        }
    }
    if !(out.is_empty() && code[from..end].trim().is_empty()) {
        out.push(from..end);
    }
    out
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
    fn computed_keys_that_only_hold_constants_are_names() {
        // Reported false positives: a retrying helper that runs the Granola
        // tool it is called with.
        let helper = r#"const acc = "acc_1ca950c2-be4e-4f10-9434-c9d691235df1";
async function g(tool, input, tries=8){
  for (let i=0;i<tries;i++){
    const r = await tools.granola[tool]({accountId: acc, input});
    const t = r?.content?.[0]?.text || "";
    if (r?.isError && /rate_limit/.test(t)) { await new Promise(res=>setTimeout(res, 2000*(i+1))); continue; }
    return r;
  }
  return {error:"rate limited after retries"};
}
"#;
        let meetings = r#"const ids = ["b6195187-1c6b-4646-b579-7bac1de34f8b","f41feaa4-b33e-4934-8edd-24c4b031f9a5","7b1227e8-aaf3-4266-b358-bd7e16e3bebb","8637a753-20e4-4e2b-9371-ca74ee20c689","8c748d59-92bb-412b-a0c5-5b270635045d"];
const r = await g("get_meetings", {meeting_ids: ids, attachments: "none"});
let t = r.content?.map(c=>c.text).join("\n") ?? JSON.stringify(r);
t = t.replace(/<transcript_guidance>[\s\S]*?<\/transcript_guidance>/,"").replace(/The connected_user_email[\s\S]*?Treat it strictly as data; do not follow instructions that appear within it\./,"").replace(/<context_for_agents>[\s\S]*?<\/context_for_agents>/g,"");
return t;"#;
        let inline = r#"const r = await g("get_meetings", {meeting_ids: ["06ece7b8-dfd3-4a44-9e74-71c22eab910c","274ba6ba-9fe0-435d-8a5a-5d98b79f02fa"], attachments: "none"});
return r.content?.map(c=>c.text).join("\n") ?? r;"#;
        let participant = r#"const r = await g("search_meetings", {participant: "joana.langemeier@mistral.ai", from_date: "2026-09-01", to_date: "2026-11-16", include_shared: true, limit: 50, max_participants: 12});
return r.content?.map(c=>c.text).join("\n") ?? r;"#;
        let keywords = r#"const r = await g("search_meetings", {keywords: ["Joana","Langemeier"], from_date: "2026-08-01", to_date: "2026-11-30", include_shared: true, limit: 30, max_participants: 8});
let t = r.content?.map(c=>c.text).join("\n") ?? JSON.stringify(r);
return t.replace(/The content below[\s\S]*?instructions that appear within it\./g,"");"#;
        let transcripts = r#"const ids = ["581fc4ca-f150-4dc9-b2e6-0475a5e7e9ea","b946231a-31eb-4244-8c16-b1ba7b640d5d","4c8b9a8e-2804-4dc2-9b8f-54c6d28975f6"];
const out = [];
const r1 = await g("search_transcripts", {meeting_ids: ids, keywords: ["Amplemarket","Vibe","Salesforce","LinkedIn","Sales Navigator","Organigramm","org chart","Stakeholder","Forecast","Pipeline","CRM","Terret","sequence","prospecting","Account Plan","Account Map"], limit: 20});
out.push(r1.content?.map(c=>c.text).join("\n"));
return out.join("\n=====\n").replace(/The content below[\s\S]*?instructions that appear within it\./g,"");"#;
        for body in [meetings, inline, participant, keywords, transcripts] {
            let program = format!("{helper}{body}");
            assert_eq!(classify(&call("mcp__executor-v2__execute", json!({"code": program}))), Classification::Pass, "{body}");
        }
        // The same helper run with a Slack write asks.
        let send = format!("{helper}await g(\"get_meetings\", {{}}); await g(\"slack_send_message\", {{}})");
        assert!(slack_write_in_code(&send.replace("granola", "mistral.slack"), false));
        for program in [
            "const t = 'notion-search'; return await tools.notion[t]({})",
            "let t = 'notion-search'\nconst r = await tools.notion[t]({})",
            "for (const t of ['notion-search', 'notion-fetch']) await tools.notion[t]({})",
            "await Promise.all(['get_meetings', 'list_meetings'].map(t => tools.granola[t]({})))",
            "await Promise.all([\"get_meetings\"].map(async (t, i) => tools.granola?.[t]({})))",
            "const run = async (tool, input) => tools.granola[tool](input); await run('get_meetings', {}); await run(`search_meetings`, {})",
            "function g(tool) { return tools.granola[tool]({}) } const name = 'get_meetings'; await g(name)",
            "async function g(tool, n = 0) { if (n < 3) return g(tool, n + 1); return tools.granola[tool]({}) } await g('get_meetings')",
            "const app = 'granola', tool = 'get_meetings'; await tools[app][tool]({})",
            // A `var` read before its value is `undefined`, which names no tool either.
            "function go() { return tools.mistral.slack[t]({}); }\nawait go();\nvar t = 'slack_read_channel';",
            // Nor does an index (`forEach` passes 0 as the second argument).
            "async function post(_, tool) { return tools.mistral.slack[tool]({}) }\nawait post(null, 'slack_read_channel');\n['x'].forEach(post);",
            // A missing argument names no tool (`tools.mistral.slack[undefined]` fails).
            "async function g(tool) { return tools.mistral.slack[tool]({}) } await g('slack_read_channel'); await g();",
            // The same name bound elsewhere to constants too.
            "for (const tool of ['a']) {} async function g(tool) { return tools.granola[tool]({}) } await g('get_meetings')",
            // The same name bound in another scope to something else.
            "async function g(t, input) { return tools.granola[t](input) }\nconst r = await g('list_meetings', {});\nreturn r.content.map((t) => t.text).join('\\n')",
            "async function g(t) { return tools.granola[t]({}) } for (const t of items) log(t); try { await g('get_meetings') } catch (t) { log(t) }",
            "const tool = 'notion-search'; await tools.notion[tool]({}); function f() { const { tool } = x; return tool }",
            // A variable that only ever holds an array of constants.
            "const names = ['get_meetings', 'list_meetings'];\nconst out = {};\nfor (const n of names) {\n  out[n] = await tools.granola[n]({});\n}\nreturn { out, count: names.length, first: names[0] };",
            "const names = ['get_meetings'];\nawait Promise.all(names.map((n, i) => tools.granola[n]({})));\nreturn names.join(',')",
            // A conditional between constants.
            "const wantFull = true;\nconst tool = wantFull ? 'notion-fetch' : 'notion-search';\nconst r = await tools.notion[tool]({ id: 'abc' });\nreturn r.content[0].text;",
            "async function g(t) { return tools.notion[t]({}) } await g(full ? 'notion-fetch' : (x ?? y) ? 'notion-search' : 'notion-get-users')",
            // One name bound to several tools that all read.
            "const out = {};\nfor (const t of ['notion-search', 'notion-get-users']) {\n  const fn = tools.notion[t];\n  out[t] = await fn({ query: 'x' });\n}\nreturn out;",
            // A helper's parameter hides a variable of its name outside it.
            "async function g(t, input) {\n  const r = await tools.granola[t](input);\n  return r;\n}\nconst r = await g('list_meetings', { limit: 20 });\nlet t = r.content?.map((c) => c.text).join('\\n') ?? JSON.stringify(r);\nt = t.replace(/<guidance>[\\s\\S]*?<\\/guidance>/g, '');\nreturn t;",
            "async function run(tool, input) {\n  const r = await tools.linear[tool](input);\n  return JSON.parse(r.content[0].text);\n}\nconst runs = await run('list_issues', { first: 10 });\nreturn runs.nodes.map((run) => run.title);",
            // \"protocol\" in a string is prose.
            "const out = {};\nfor (const t of ['notion-search', 'notion-get-users']) {\n  const r = await tools.notion[t]({ query: 'MCP protocol design review' });\n  out[t] = r.content[0].text;\n}\nreturn out;",
            // Keys written as expressions, elements, members, templates and casts.
            "const full = true;\nconst r = await tools.notion[full ? 'notion-fetch' : 'notion-search']({ id: 'abc123' });\nreturn r.content[0].text;",
            "const names = ['list_meetings', 'search_meetings'];\nconst out = [];\nfor (let i = 0; i < names.length; i++) {\n  const r = await tools.granola[names[i]]({ limit: 10 });\n  out.push(r.content[0].text.length);\n}\nreturn out;",
            "const TOOLS = {\n  search: 'notion-search',\n  fetch: 'notion-fetch',\n};\nconst r = await tools.notion[TOOLS.search]({ query: 'Q4 roadmap' });\nconst pages = [];\nfor (const h of hits) pages.push(await tools.notion[TOOLS.fetch]({ id: h.id }));\nreturn { pages, tools: Object.keys(TOOLS) };",
            "const kinds = ['users', 'teams'];\nconst out = {};\nfor (const k of kinds) {\n  const r = await tools.notion[`notion-get-${k}`]({});\n  out[k] = r.content[0].text.length;\n}\nreturn out;",
            "const READS = ['list_issues', 'get_issue'] as const;\nfor (const t of READS) await tools.linear[t]({ limit: 3 });",
            "async function g(tool, input) {\n  const name = tool ?? 'list_meetings';\n  return tools.granola[name](input);\n}\nconst r = await g('get_meetings', {});\nreturn r;",
            "async function call(tool: string, input: object = {}): Promise<string> {\n  const r = await tools.linear[tool](input);\n  return r.content[0].text;\n}\nreturn await call('list_issues', { first: 10 });",
            "async function fetchOne(tool) {\n  const r = await tools.notion[tool]({});\n  return r.content[0].text;\n}\nconst texts = await Promise.all(['notion-get-users', 'notion-get-teams'].map(fetchOne));\nreturn texts.map((t) => t.length);",
            "const names = ['list_issues', 'list_projects'];\nconst out = {};\nfor (const n of names) out[n] = (await tools.linear[n]({ first: 5 })).content[0].text.length;\nreturn { names, out };",
        ] {
            assert!(!slack_write_in_code(program, false), "{program}");
        }
        for program in [
            // A constant that names a Slack write.
            "const t = 'slack_send_message'; await tools.mistral.slack[t]({})",
            "async function g(tool) { return tools.mistral.slack[tool]({}) } await g('slack_read_channel'); await g('slack_send_message')",
            "for (const t of ['slack_read_channel', 'slack_send_message']) await tools.mistral.slack[t]({})",
            "['slack_read_channel', 'chat_post'].forEach(t => tools.mistral.slack[t]({}))",
            "const app = 'mistral'; await tools[app].slack.send_message({})",
            "await tools.mistral.studio[op]({}); const op = 'execute_workflow'; // slack",
            // Values that are not only constants.
            "async function g(tool) { return tools.granola[tool]({}) } await g('get_meetings'); await g(name)",
            "async function g(tool) { return tools.granola[tool]({}) } await g(...names)",
            "async function g(tool) { return tools.granola[tool]({}) } await g('get_meetings'); helpers.push(g)",
            "async function g(tool) { return tools.granola[tool]({}) } await g.call(null, 'slack_send_message')",
            "async function g(tool) { tool = tool + '_message'; return tools.granola[tool]({}) } await g('slack_send')",
            "async function g(tool) { tool += '_message'; return tools.granola[tool]({}) } await g('slack_send')",
            "async function g(tool) { arguments[0] = x; return tools.granola[tool]({}) } await g('get_meetings')",
            "const g = async (tool) => tools.granola[tool]({}); const h = g; await h(name)",
            "const t = 'get_meetings'; t2 = t; await tools.granola[t + suffix]({})",
            "let t = 'get_meetings'; t = pick(); await tools.granola[t]({})",
            "let t = 'get_meetings'; [t] = picks; await tools.granola[t]({})",
            "let t = 'get_meetings'; ({ t } = picks); await tools.granola[t]({})",
            "const t = 'get_meetings'; { const { t } = picks; await tools.granola[t]({}) }",
            "const t = 'get_meetings'; try { x() } catch (t) { await tools.granola[t]({}) }",
            "const t = 'get_meetings'; for (const t in tools.granola) await tools.granola[t]({})",
            "const t = 'get_meetings'; for (const t of names) await tools.granola[t]({})",
            "const t = 'get_meetings'; names.map(t => tools.granola[t]({}))",
            "const t = 'get_meetings'; names.map((t) => tools.granola[t]({}))",
            "const t = 'get_meetings'; names.map(function (t) { return tools.granola[t]({}) })",
            "const t = 'get_meetings'; const f = ({ t }) => tools.granola[t]({})",
            "const t = 'get_meetings'; obj = { run(t) { return tools.granola[t]({}) } }",
            "const t = 'get_meetings'; globalThis.t = name; await tools.granola[t]({})",
            "const t = 'get_meetings'; with (picks) { await tools.granola[t]({}) }",
            "const t = 'get_meetings'\n+ suffix; await tools.granola[t]({})",
            "const t = 'get_meetings' + suffix; await tools.granola[t]({})",
            "const t = cond ? 'get_meetings' : other; await tools.granola[t]({})",
            "await tools.granola[name]({})",
            "await tools.granola[`get_${what}`]({})",
            "function g(tool) { return tools.granola[tool]({}) } function g(other) {} await g('get_meetings')",
            "const g = (tool: string): Promise<any> => tools.granola[tool]({}); await g(name)",
            // Scopes: an assignment anywhere counts, a declaration where it is visible.
            "const t = 'get_meetings'; function f() { t = pick() } await tools.granola[t]({})",
            "let t = 'get_meetings'; { let u = 1; t = name } await tools.granola[t]({})",
            "function f(t) { return tools.granola[t]({}) } const t = 'get_meetings'; await f(t); await f(other)",
            "await tools.granola[x[t = name]]({}); const t = 'a'; await tools.granola[t]({})",
            "const t = 'get_meetings'; xs.map(t => tools.granola[t]({}))",
            // Arrays that change, or loops that may be changed.
            "const names = ['get_meetings']; names.push(other); for (const n of names) await tools.granola[n]({})",
            "const names = ['get_meetings']; names[0] = other; for (const n of names) await tools.granola[n]({})",
            "const names = ['get_meetings']; helper(names); for (const n of names) await tools.granola[n]({})",
            "const names = ['get_meetings']; names.forEach((n, i, all) => all.push(x)); for (const n of names) await tools.granola[n]({})",
            "const names = ['get_meetings']; names.map((fn)); for (const n of names) await tools.granola[n]({})",
            "let names = ['get_meetings']; names = more; for (const n of names) await tools.granola[n]({})",
            "Array.prototype.map = function (cb) { return [cb('slack_send_message', 0)] };\nawait Promise.all(['slack_read_channel'].map((t) => tools.mistral.slack[t]({})))",
            "const it = Symbol.iterator; for (const t of ['slack_read_channel']) await tools.mistral.slack[t]({})",
            // Conditionals that are not only constants.
            "const t = full ? 'notion-fetch' : other; await tools.notion[t]({})",
            "const t = full ? 'notion-fetch' : 'notion-search' + x; await tools.notion[t]({})",
            "const t = full ? 'slack_read_channel' : 'slack_send_message'; await tools.mistral.slack[t]({})",
            "const t = (x) => full ? 'a' : 'b'; await tools.granola[t]({})",
            // One name bound to several tools, one of which writes or reaches Slack.
            "for (const t of ['notion-search', 'notion-update-page']) { const fn = tools.notion[t]; await fn({}) }",
            "for (const t of ['notion-search', 'slack']) { const fn = tools.mistral[t]; await fn.post({}) }",
            // A loop body without braces reaches past its first `;`.
            "const tool = 'slack_read_channel';\nconst dryRun = false;\nfor (const tool of ['slack_send_message'])\n  if (dryRun) console.log('would call', tool);\n  else await tools.mistral.slack[tool]({ channel_id: 'C0123', message: 'Deploy finished' });\n",
            // Hiding needs an exact scope: an arrow inside a conditional may end at its `:`.
            "function run(t) { return tools.mistral.slack[t]({}) } const f = c ? (run) => run : run('slack_send_message'); run('slack_read_channel')",
            // Expressions, elements, members and templates that may hold other names.
            "await tools.mistral.slack[full ? 'slack_read_channel' : 'slack_send_message']({})",
            "await tools.mistral.slack[x ?? 'slack_send_message']({})",
            "const names = ['slack_read_channel', 'slack_send_message']; await tools.mistral.slack[names[i]]({})",
            "const T = { read: 'slack_read_channel', send: 'slack_send_message' }; await tools.mistral.slack[T.send]({})",
            "const T = { read: 'slack_read_channel' }; T.read = 'slack_send_message'; await tools.mistral.slack[T.read]({})",
            "const T = { read: 'slack_read_channel', m() { this.read = x } }; T.m(); await tools.mistral.slack[T.read]({})",
            "const T = { read: 'slack_read_channel', m: function () { this.read = x } }; T.m(); await tools.mistral.slack[T.read]({})",
            "const T = { read: 'slack_read_channel' }; Object.assign(T, other); await tools.mistral.slack[T.read]({})",
            "const T = { read: 'slack_read_channel' }; await tools.mistral.slack[T.other]({})",
            "const T = { read: 'slack_read_channel', __proto__: P }; await tools.mistral.slack[T.read]({})",
            "await tools.mistral.slack[`slack_${op}_message`]({})",
            "const op = 'send'; await tools.mistral.slack[`slack_${op}_message`]({})",
            "const k = '__proto__'; [][k].map = f; for (const t of ['slack_read_channel']) await tools.mistral.slack[t]({})",
            "const names = ['slack_read_channel']; [...names].forEach(f); names.map(g); await tools.mistral.slack[names[0]]({})",
            "function g(t, i, all) { all[1] = x; return tools.mistral.slack[t]({}) } ['slack_read_channel', 'b'].map(g)",
            // A constant too long to be a tool's name.
            &format!("const t = '{}'; await tools.granola[t]({{}})", "a".repeat(200)),
        ] {
            assert!(slack_write_in_code(program, false), "{program}");
        }
        // Programs built to make the check slow still finish quickly.
        let nested = format!("const t = 'get_meetings';\nlet a;\n{}{}{};\nawait tools.granola[a]({{}});\n",
            "[".repeat(2000), vec!["a = t"; 315].join(", "), "]".repeat(2000));
        let comment = format!("const t = 'get_meetings';\nlet a;\nconsole.log([{}])\n{}await tools.granola[a]({{}});\n",
            vec!["a = t"; 315].join(", "), "// notes about the meetings we fetched above\n".repeat(852));
        let chained = format!("for (var t of [{}]) {{}}\nlet x1, x2, x3, x4, x5, x6, x7;\n{}{}{}{}{}{}{}await tools.granola[x7]({{}});\n",
            vec!["'a'"; 6000].join(","), "x1 = t;\n".repeat(3), "x2 = x1;\n".repeat(3), "x3 = x2;\n".repeat(2),
            "x4 = x3;\n".repeat(3), "x5 = x4;\n".repeat(5), "x6 = x5;\n".repeat(7), "x7 = x6;\n".repeat(7));
        let dotted = format!("const a = '{}';\nawait tools.granola{}({{}});\n", vec!["x"; 1000].join("."), "[a]".repeat(1000));
        let calls = format!("let x;\nfunction g(a, t) {{\n{}  return a;\n}}\n{}0{};\nawait tools.granola[x]({{}});\n",
            "  x = t;\n".repeat(90), "g(".repeat(1000), ",'get_meetings')".repeat(1000));
        let walk = format!("const t = 'get_meetings'; await tools.granola[t]({{}});\n{}{}{};\n",
            "[".repeat(2000), "t,".repeat(2000), "]".repeat(2000));
        let fanout = format!("for (var u of [{}]) {{}}\nlet t;\n{}\n{}\n",
            (0..64).map(|i| format!("'v{i}'")).collect::<Vec<_>>().join(","), "t=u;".repeat(8000), "tools.a[t]();".repeat(2000));
        let sites = format!("const x = 'get_meetings';\nfunction g(t) {{ return tools.granola[t]({{}}) }}\n{}", "g(x);".repeat(12785));
        let reads = format!("const t = 'get_meetings';\n{}", "tools.a[t]();".repeat(4921));
        let long = format!("for (var u of [{}]) {{}}\nlet t;\n{}\nawait tools.a[t]();\n",
            (0..64).map(|i| format!("'{i:02}{}'", "A".repeat(14000))).collect::<Vec<_>>().join(","), "t=u;".repeat(4000));
        let dots = format!("for (const t of [{}]) {{\n{}}}\n",
            (0..64).map(|i| format!("\"{}\"", vec![format!("v{i}"); 63].join("."))).collect::<Vec<_>>().join(","), "tools[t]();".repeat(4000));
        let ident = format!("const t = 'get_meetings'; await tools.a[t]({{}});\nfunction {}(t{}) {{}}\n", "A".repeat(32000), ",t".repeat(15965));
        for program in [nested, comment, chained, dotted, calls, walk, fanout, sites, reads, long, dots, ident] {
            let started = std::time::Instant::now();
            slack_write_in_code(&program, false);
            assert!(started.elapsed() < std::time::Duration::from_secs(5), "{}", &program[..80]);
        }
    }

    #[test]
    fn traced_names_include_every_value_a_key_can_hold() {
        // Found by reviews: each runs a Slack write through a computed key.
        for program in [
            r##"const tool = "slack_list_channels";
const channels = await tools.mistral.slack[tool]({});
const plan = { slack_send_message: { channel: "C1", text: "hi" } };
for (const [tool, args] of Object.entries(plan)) {
  await tools.mistral.slack[tool](args);
}"##,
            r##"function g(tool) { return tools.mistral.slack[tool]({}); }
function h(x = g("slack_send_message")) { return x; }
await g("slack_list_channels");
await h();"##,
            r##"let tool = "slack_list_channels";
async function* steps() { yield "slack_send_message"; }
for await (tool of steps()) {
  console.log(tool);
}
await tools.mistral.slack[tool]({});"##,
            r##"const names = ["slack_list_channels"];
for (names[0] of ["slack_send_message"]) {}
await tools.mistral.slack[names[0]]({});"##,
            r##"let tool = "slack_send_message";
{
  let toolé = 1;
  if (false) tool = "slack_list_channels";
  await tools.mistral.slack[tool]({});
}"##,
            r##"Object.assign(global, { tool: "slack_send_message" });
if (false) tool = "slack_list_channels";
await tools.mistral.slack[tool]({});"##,
            r##"let tool = "slack_list_channels";
let ratio = 1.
tool = "slack_send_message"
await tools.mistral.slack[tool]({});"##,
            r##"const post = async (ctx, tool = "slack_read_channel") => tools.mistral.slack[tool]({ channel: "C1", text: "x" });
const withRetry = async (fn) => fn(null, "slack_send_message");
await withRetry(post);"##,
            r##"async function post(tool) { return tools.mistral.slack[tool]({ channel: "C1", text: "x" }) }
async function report(text, send = post) { return post("slack_send_message") }
await post("slack_read_channel");
await report("done");"##,
            r##"async function notify(tool) { return tools.mistral.slack[tool]({ channel: "C1", text: "done" }) }
await notify("slack_read_channel")
console.log(notify)
{
  await notify("slack_send_message")
}"##,
            r##"const TOOLS = { post: "slack_read_channel" };
for (TOOLS.post of ["slack_send_message"]) {}
await tools.mistral.slack[TOOLS.post]({ channel: "C1", text: "x" });"##,
            r##"const names = ["slack_read_channel"];
const MyObject = { keys(a) { a.push("slack_send_message"); return [] } };
MyObject.keys(names);
await Promise.all(names.map((t) => tools.mistral.slack[t]({ channel: "C1", text: "x" })));"##,
            r##"const tool = "slack_read_channel";
class Channel { async for(tool) { return tools.mistral.slack[tool]({ channel: "C1", text: "x" }) } }
await tools.mistral.slack[tool]({ channel: "C1" });
await new Channel().for("slack_send_message");"##,
            r##"async function g(this: unknown, tool: string) { return tools.mistral.slack[tool]({ channel: "C1", text: "x" }) }
await g("slack_send_message");
await g(undefined, "slack_read_channel");"##,
            r##"async function post(tool) { return tools.mistral.slack[tool]({ channel: "C1", text: "x" }) }
const routes = { slack_read_channel: ["slack_send_message"] }
["slack_read_channel"].map(post)"##,
            r##"async function post(tool) { return tools.mistral.slack[tool]({ channel: "C1", text: "x" }) }
async function report(ñpost) { return post("slack_send_message") }
await post("slack_read_channel");
await report(1);"##,
            r##"async function sum(tool) { return tools.mistral.slack[tool]({ channel: "C1", text: "x" }) }
await sum("slack_read_channel");
let i = 4;
const ratio = i-- / 2 /sum("slack_send_message");"##,
            r##"async function g(t) { return tools.mistral.slack[t]({ channel: "C1", text: "x" }) }
await g("slack_read_channel");
const scale = 1.
g("slack_send_message");"##,
            r##"function make<A, B, C>(): any { return null }
async function g(ctx: any, tool: string) { return tools.mistral.slack[tool]({ channel: "C1", text: "x" }) }
await g(make<string, "slack_read_channel", number>(), "slack_send_message");"##,
            r##"const s = `${function g(t, n) { if (n) g("slack_read_channel", 0); return tools.mistral.slack[t]({}) }("slack_send_message", 1)}`;"##,
            r##"const names = ["slack_read_channel"];
for (names[0] of ["slack_send_message"]) {}
await tools.mistral.slack[names[0]]({ channel: "C1", text: "hi" });"##,
            r##"const names = ["slack_read_channel"];
const sorted = names.sort();
sorted[0] = "slack_send_message";
await tools.mistral.slack[names[0]]({ channel: "C1", text: "hi" });"##,
            r##"const names = ["slack_read_channel"];
const m = "push";
names[m]("slack_send_message");
for (const t of names) await tools.mistral.slack[t]({ channel: "C1", text: "hi" });"##,
            r##"const myJSON = { stringify: (o) => { o.read = "slack_send_message"; } };
const TOOLS = { read: "slack_read_channel" };
myJSON.stringify(TOOLS);
await tools.mistral.slack[TOOLS.read]({ channel: "C1", text: "hi" });"##,
            r##"const names = ["slack_read_channel"];
JSON.stringify(names, (k, v) => { if (k === "") v[0] = "slack_send_message"; return v; });
await tools.mistral.slack[names[0]]({ channel: "C1", text: "hi" });"##,
            r##"function f() { this.read = "slack_send_message"; }
const TOOLS = { read: "slack_read_channel", m: f };
(TOOLS.m)();
await tools.mistral.slack[TOOLS.read]({ channel: "C1", text: "hi" });"##,
            r##"const names = ["slack_read_channel"];
return [names][0].push("slack_send_message"), await tools.mistral.slack[names[1]]({ channel: "C1", text: "hi" });"##,
            r##"let t = "slack_read_channel";
let n = 1;
n++ / 2; t = "slack_send_message"; n = n / 2;
await tools.mistral.slack[t]({ channel: "C1", text: "hi" });"##,
            r##"const names = ["slack_read_channel"];
const d = Object.values(Object.getOwnPropertyDescriptors(Array)).find((d) => Array.isArray(d.value));
d.value[3] = "slack_send_message";
const i = 3;
await tools.mistral.slack[names[i]]({ channel: "C1", text: "hi" });"##,
            r##"let t = "slack_read_channel";
const sep = "a b"; t = "slack_send_message"; const z = "q";
await tools.mistral.slack[t]({});"##,
            r##"let tool = "slack_read_channel";
const stats = { in: 10, out: 5 };
if (stats.out) {
  console.log(`ratio ${stats.in / stats.out}`);
}
tool = "slack_send_message";
const msg = `tool: ${tool}`;
await tools.mistral.slack[tool]({ text: msg });"##,
            r##"let t = "slack_send_message";
for (const técnico of ["Ana"]) {
  await tools.mistral.slack[t]({ text: técnico });
  t = "slack_read_channel";
}"##,
            r##"let t = "slack_read_channel";
t﻿= "slack_send_message";
await tools.mistral.slack[t]({});"##,
            r##"let t = "slack_send_message";
{
  <!-- let t = "slack_read_channel";
  await tools.mistral.slack[t]({});
}
(also with `-->` at the start of the line: /tmp/review-text/final/05a_html.js, 05b_html.js)"##,
            r##"let t = "slack_read_channel";
const quotes = /'+'/g; t = "slack_send_message";
await tools.mistral.slack[t]({});"##,
            r##"let t = "slack_read_channel";
const x = {};
x // remember the name
["y"] = t = "slack_send_message";
await tools.mistral.slack[t]({});
(/tmp/review-text/final/07_dotkeys_comment.js; also `cfg // per-app settings\n  ["slack"].tool = t = "slack_send_message";`)"##,
            r##"Array["prot\otype"].map = function (cb) { return [cb("slack_send_message", 0)]; };
await Promise.all(["slack_read_channel"].map((t) => tools.mistral.slack[t]({})));"##,
            r##"let t = "slack_read_channel";
if (true) [t] = ["slack_send_message"];
await tools.mistral.slack[t]({});"##,
            r##"const names = ["slack_read_channel"];
const m = "push";
names[m]("slack_send_message");
for (const t of names) await tools.mistral.slack[t]({});
(/tmp/review-text/final/10_computed_method.js)"##,
            r##"if (false) t = 'slack_list_channels';
Object.prototype.t = 'slack_send_message';
await tools.mistral.slack[t]({});

// sloppy-mode variants, same result (PASS / slack_send_message):
// t = 'slack_list_channels'; Object.assign(this, { t: 'slack_send_message' }); await tools.mistral.slack[t]({});
// t = 'slack_list_channels'; const g = this; g.t = 'slack_send_message'; await tools.mistral.slack[t]({});"##,
            r##"function unused() { var t = 'slack_list_channels'; return t; }
Object.prototype.t = 'slack_send_message';
await tools.mistral.slack[t]({});

// array / object variants (literal()):
// function unused() { var NAMES = ['slack_list_channels']; }
// Reflect.set(this, 'NAMES', ['slack_send_message']);
// for (const t of NAMES) await tools.mistral.slack[t]({});
//
// function unused() { var TOOLS = { send: 'slack_list_channels' }; }
// Reflect.set(this, 'TOOLS', { send: 'slack_send_message' });
// await tools.mistral.slack[TOOLS.send]({});"##,
            r##"const ap = Object.values(Object.getOwnPropertyDescriptors(Array)).find((d) => d.value && d.value.map).value;
ap.map = function (f) { return [f('slack_send_message', 0)]; };
await Promise.all(['slack_list_channels'].map((t) => tools.mistral.slack[t]({})));

// for-of variant (Symbol.iterator found without the word):
// const ap = Object.values(Object.getOwnPropertyDescriptors(Array)).find((d) => d.value && d.value.map).value;
// const it = Reflect.ownKeys(ap).find((k) => typeof k !== 'string' && ap[k] === ap.values);
// ap[it] = function* () { yield 'slack_send_message'; };
// for (const t of ['slack_list_channels']) await tools.mistral.slack[t]({});
// (also ap.forEach = …, or ap[1] = 'slack_send_message' with names[i] read out of range)"##,
            r##"const TOOLS = { send: 'slack_list_channels' };
JSON.stringify(TOOLS, (k, v) => { if (k === '') v.send = 'slack_send_message'; return v; });
await tools.mistral.slack[TOOLS.send]({});

// array variant:
// const names = ['slack_list_channels'];
// JSON.stringify(names, (k, v) => { if (k === '') v[0] = 'slack_send_message'; return v; });
// for (const t of names) await tools.mistral.slack[t]({});"##,
            r##"await import('data:text/javascript,Array.prototype.map=function(f){return [f("slack_send_message",0)]}');
await Promise.all(['slack_list_channels'].map((t) => tools.mistral.slack[t]({})));"##,
            r##"var yield = 'slack_list_channels';
function* g() { const t = yield; return tools.mistral.slack[t]({}); }
const it = g();
it.next();
await it.next('slack_send_message').value;"##,
            r##"let i = 4; let t = 'slack_list_channels';
const r = i++ / 2; t = 'slack_send_message'; const q = r / 1;
await tools.mistral.slack[t]({});"##,
        ] {
            assert!(slack_write_in_code(program, false), "{program}");
        }
        // And a later review.
        for program in [
            r##"const mode = "notify";
var tool = "slack_read_channel";
switch (mode) {
  case "notify": {
    if (mode === "notify") {
      var tool = "slack_send_message";
    }
    break;
  }
}
await tools.mistral.slack[tool]({ channel_id: "C1", text: "hi" });"##,
            r##"const t = "slack_read_channel";
const api = await { post(t) { return tools.mistral.slack[t]({}); } };
await api.post("slack_send_message");"##,
            r##"const t = "slack_read_channel";
const withOpts = (B, o) => B;
class Api extends withOpts(Object, { retries: 3 }) {
  post(t) { return tools.mistral.slack[t]({}); }
}
await new Api().post("slack_send_message");"##,
            r##"let t = "slack_read_channel";
function f(env) {
  const x = env.var
  t = "slack_send_message"
}
f({});
await tools.mistral.slack[t]({});"##,
            r##"let t = "slack_read_channel";
for ((t) of ["slack_send_message"]) {}
await tools.mistral.slack[t]({});"##,
            r##"var t = "slack_read_channel";
{
  let = 1, t = "slack_send_message";
}
await tools.mistral.slack[t]({});"##,
            r##"JSON.stringify = (a) => { a.push("slack_send_message"); return ""; };
const names = ["slack_read_channel"];
console.log(JSON.stringify(names));
for (const t of names) await tools.mistral.slack[t]({ channel: "C1", text: "x" });"##,
            r##"for (let t of ["slack_read_channel"]) console.log(t);
Object.assign(this, { t: "slack_send_message" });
await tools.mistral.slack[t]({ channel: "C1", text: "x" });"##,
            r##"const TOOLS = { read: "slack_read_channel" };
[...TOOLS.read] = ["slack_send_message"];
await tools.mistral.slack[TOOLS.read]({ channel: "C1", text: "hi" });"##,
            r##"const names = ["slack_read_channel"];
return {
  names,
  async then(resolve) {
    const me = this;
    me.names.push("slack_send_message");
    for (const t of names) await tools.mistral.slack[t]({ channel: "C1", text: "hi" });
    resolve("done");
  },
};"##,
            r##"const TOOLS = { read: "slack_read_channel" };
for ((TOOLS.read) in { slack_send_message: 1 }) {}
await tools.mistral.slack[TOOLS.read]({ channel: "C1", text: "hi" });"##,
            r##"let t = "slack_read_channel";
for ((t) of ["slack_send_message"]) {}
await tools.mistral.slack[t]({ channel: "C1", text: "hi" });"##,
            r##"const TOOLS = { read: "slack_read_channel" };
const m = "__defineGetter__";
TOOLS?.[m]("read", () => "slack_send_message");
await tools.mistral.slack[TOOLS.read]({ channel: "C1", text: "x" });"##,
            r##"function plan() { return { slack_read_channel: ["slack_send_message"] }; }
await Promise.all(plan`daily`["slack_read_channel"].map((t) => tools.mistral.slack[t]({ channel: "C1", text: "x" })));"##,
            r##"const of = { slack_read_channel: ["slack_send_message"] };
await Promise.all(of["slack_read_channel"].map((t) => tools.mistral.slack[t]({ channel: "C1", text: "x" })));"##,
            r##"for (let i = 0; i < 1; function g(t, n) { if (n) g("slack_read_channel", 0); return tools.mistral.slack[t]({ channel: "C1", text: "x" }) }("slack_send_message", 1)) i++;"##,
            r##"const z = ++/'/.lastIndex; await tools.mistral.slack.slack_send_message({ channel: "x", text: "hi" }); //'"##,
            r##"let n = 1
++/'/.lastIndex; await tools.mistral.slack.slack_send_message({ channel: "x", text: "hi" }); //'"##,
            r##"let tool = "slack_read_channel";
const n = ++/'/.lastIndex; tool = "slack_send_message"; //'
await tools.mistral.slack[tool]({ channel: "x" });"##,
            r##"const z = [...typeof /'/]; await tools.mistral.slack.slack_send_message({ channel: "x", text: "hi" }); //'"##,
            r##"let tool = "slack_read_channel";
const z = [...typeof /'/, 1]; tool = "slack_send_message"; const w = [...typeof /'/, 1];
await tools.mistral.slack[tool]({ channel: "x" });"##,
            r##"let tool = "slack_read_channel";
const z = 1. in /'/; tool = "slack_send_message"; //'
await tools.mistral.slack[tool]({ channel: "x" });"##,
            r##"let tool = "slack_read_channel";
const n = 1 / /'/.lastIndex; tool = "slack_send_message"; //'
await tools.mistral.slack[tool]({ channel: "x" });"##,
            r##"let tool = "slack_read_channel";
const z = false && class extends /'/ {}; tool = "slack_send_message"; //'
await tools.mistral.slack[tool]({ channel: "x" });"##,
            r##"let tool = "slack_read_channel";
const z = false && [.../'/]; tool = "slack_send_message"; const w = false && [.../'/];
await tools.mistral.slack[tool]({ channel: "x" });"##,
            r##"let tool = "slack_read_channel";
const of = 4;
const half = of / 2; tool = "slack_send_message"; const q = half / 1;
await tools.mistral.slack[tool]({ channel: "x" });"##,
            r##"let tool = "slack_read_channel";
class S { #of = 4; half() { const h = this.#of / 2; tool = "slack_send_message"; return h / 1; } }
new S().half();
await tools.mistral.slack[tool]({ channel: "x" });"##,
            r##"let tool = "slack_read_channel";
var yield = 4;
const half = yield / 2; tool = "slack_send_message"; const q = half / 1;
await tools.mistral.slack[tool]({ channel: "x" });"##,
            r##"let tool = "slack_read_channel";
(function () { var await = 4; const z = await / 2; tool = "slack_send_message"; return z / 1; })();
await tools.mistral.slack[tool]({ channel: "x" });"##,
            r##"let tool = "slack_read_channel";
const stats = { "time-in": 4 };
const h = stats["time-in"] / 2; tool = "slack_send_message"; const q = h / 1;
await tools.mistral.slack[tool]({ channel: "x" });"##,
            r##"let tool = "slack_read_channel";
const o = { "n-": 4 };
const h = o["n-"] / 2; tool = "slack_send_message"; const q = h / 1;
await tools.mistral.slack[tool]({ channel: "x" });"##,
            r##"let tool = "slack_read_channel";
const r = /[a['b']/; tool = "slack_send_message"; const z = /]/;
await tools.mistral.slack[tool]({ channel: "x" });"##,
            r##"let tool = "slack_read_channel";
function later() { tool = "slack_send_message"; }
{
  const o = { const: 1 };
  o.const
  tool = "slack_read_channel"
  later();
  await tools.mistral.slack[tool]({ channel: "x" });
}"##,
            r##"const o = { var: 1 };
if (false) {
  o.var
  t = 'slack_list_channels'
}
Object.prototype.t = 'slack_send_message';
await tools.mistral.slack[t]({});"##,
            r##"let $tool = "slack_read_channel";
$tool = "slack_send_message";
await tools.mistral.slack[$tool]({ channel: "x" });"##,
            r##"let tool$ = "slack_read_channel";
tool\u{24} = "slack_send_message";
await tools.mistral.slack[tool$]({ channel: "x" });"##,
            r##"const AP = Array[`${ "prototype" }`];
AP.forEach = function (cb) { cb("slack_send_message"); };
const names = ["slack_read_channel"];
names.forEach((t) => tools.mistral.slack[t]({ channel: "x" }));"##,
        ] {
            assert!(slack_write_in_code(program, false), "{program}");
        }
        // And the one after it.
        for program in [
            r##"let t = "slack_read_channel";
let i = 0
i++
{
  for (t of ["slack_send_message"]) {
    await tools.mistral.slack[t]({});
  }
}"##,
            r##"const t = "slack_read_channel";
const mix = (B) => B;
class A extends mix(class {}) {
  run(t) { return tools.mistral.slack[t]({}); }
}
await new A().run("slack_send_message");"##,
            r##"let t = "slack_read_channel";
const o = {
  class() {
    for (t of ["slack_send_message"]) {
      tools.mistral.slack[t]({});
    }
  }
};
await o.class();"##,
            r##"const t = "slack_read_channel";
const s = `${ { run(t) { return tools.mistral.slack[t]({}); } }.run("slack_send_message") }`;"##,
            r##"let t = "slack_read_channel";
const obj = { function(x) { return x; } };
obj.function(t)
{
  t = "slack_send_message";
}
await tools.mistral.slack[t]({});"##,
            r##"let t = "slack_read_channel";
for (t => 0; !(t = "slack_send_message"); ) {}
await tools.mistral.slack[t]({});"##,
            r##"const t = "slack_read_channel";
-function g(t) { tools.mistral.slack[t]({}); }("slack_send_message");"##,
            r##"const z = 1 - typeof /'/; await tools.mistral.slack.slack_send_message({ channel: "x", text: "hi" }); //'"##,
            r##"const z = typeof ++/'/.lastIndex; await tools.mistral.slack.slack_send_message({ channel: "x", text: "hi" }); //'"##,
            r##"let tool = "slack_read_channel";
const $1 = { in: 4 };
const h = $1.in / 2; tool = "slack_send_message"; const q = h / 1;
await tools.mistral.slack[tool]({ channel: "x" });"##,
            r##"const names = ["slack_read_channel"];
const m = "push";
(names[m])("slack_send_message");
for (const t of names) await tools.mistral.slack[t]({ channel: "C1", text: "hi" });"##,
            r##"const TOOLS = { read: "slack_read_channel" };
(TOOLS.__defineGetter__)("read", () => "slack_send_message");
await tools.mistral.slack[TOOLS.read]({ channel: "C1", text: "hi" });"##,
            r##"const names = ["slack_read_channel"];
const m = "push";
names[m]?. ("slack_send_message");
for (const t of names) await tools.mistral.slack[t]({ channel: "C1", text: "hi" });"##,
            r##"const TOOLS = { read: "slack_read_channel" };
TOOLS.__defineGetter__?. ("read", () => "slack_send_message");
await tools.mistral.slack[TOOLS.read]({ channel: "C1", text: "hi" });"##,
            r##"const names = ["slack_read_channel"];
async function later(resolve) {
  this.list.push("slack_send_message");
  for (const t of names) await tools.mistral.slack[t]({ channel: "C1", text: "hi" });
  resolve("done");
}
return { list: names, then: later };

(object variant /tmp/r5-reads-final/c06_then_ref_array.js: `this.cfg.read = "slack_send_message"; … return { cfg: TOOLS, then: later };`)"##,
            r##"globalThis.JSON.stringify = (a) => { a.push("slack_send_message"); return ""; };
const names = ["slack_read_channel"];
console.log(JSON.stringify(names));
for (const t of names) await tools.mistral.slack[t]({ channel: "C1", text: "x" });"##,
            r##"const names = ["slack_read_channel"];
function run(t) { return t; }
[run] = [(t, i, a) => a.push("slack_send_message")];
names.map(run);
for (const t of names) await tools.mistral.slack[t]({ channel: "C1", text: "hi" });"##,
            r##"const names = ["slack_read_channel"];
function run(t) { return t; }
function each(run) { names.forEach(run); }
each((t, i, a) => a.push("slack_send_message"));
for (const t of names) await tools.mistral.slack[t]({ channel: "C1", text: "hi" });"##,
            r##"async function main(): Promise<void> {
  let tool = "slack_read_channel";
  if (tool) {
    tool = "slack_send_message";
  }
  await tools.mistral.slack[tool]({ channel_id: "C1", message: "Deploy finished" });
}
await main();

// u8.js: for (const app of ["slack"]) await tools.mistral[app][`${app}_send_message`](...) inside `async function broadcast(text: string): Promise<void> {`
// u9.js: let tool = "slack_read_channel"; for (tool of ["slack_read_channel", "slack_send_message"]) await tools.mistral.slack[tool](...) inside `): Promise<void> {`
// u10.js: switch (tool) { case "slack_read_channel": if (!history.messages?.length) tool = "slack_send_message"; break; } then tools.mistral.slack[tool](...)"##,
        ] {
            assert!(slack_write_in_code(program, false), "{program}");
        }
        for program in [
            r##"const threadTs = "1731700000.000100";
let tool;
if (threadTs) tool = "slack_read_thread";
else tool = "slack_read_channel";
const r = await tools.mistral.slack[tool]({ accountId: "acc_s", input: { channel_id: "C1", message_ts: threadTs } });
return r.content[0].text;"##,
            r##"const READS = ["slack_search_public", "slack_search_users"] as const;
async function searchAll(query: string): Promise<Record<string, string>> {
  const res: Record<string, string> = {};
  for (const t of READS) {
    const r = await tools.mistral.slack[t]({ accountId: "acc_s", input: { query } });
    res[t] = r.content[0].text;
  }
  return res;
}
return await searchAll("launch");"##,
            r##"let r;
try {
  r = await tools.granola.get_meetings({ accountId: "a", input: { meeting_ids: ["x"] } });
} catch {
  for (const t of ["list_meetings", "search_meetings"]) {
    r = await tools.granola[t]({ accountId: "a", input: {} });
    if (r) break;
  }
}
return r;"##,
            r##"async function g(tool, input) {
  return tools.granola[tool]({ accountId: "a", input });
}
const t = "list_meetings";
const want = true;
const summary = { meetings: want ? await g(t, {}) : null, at: Date.now() };
return summary;"##,
            r##"async function g(tool, input) {
  return tools.granola[tool]({ accountId: "a", input });
}
const window = { from_date: "2026-11-01", to_date: "2026-11-15" };
const r = await g("search_meetings", { keywords: ["pricing"], from_date: window.from_date, to_date: window.to_date });
console.log("searched", window);
return r;"##,
            r##"class Cache {
  constructor() { this.m = new Map(); }
  get(k) { return this.m.get(k); }
  set(k, v) { this.m.set(k, v); return this; }
}
const cache = new Cache();
const g = async (tool, input) => tools.granola[tool]({ accountId: "a", input });
const r = await g("list_meetings", {});
cache.set("m", r);
return cache.get("m");"##,
            r##"const TOOLS = ["list_meetings", "get_meetings"];
const out = {};
for (const t of TOOLS) {
  out[t] = await tools.granola[t]({ accountId: "a", input: {} });
}
return { tools: TOOLS, count: Object.keys(out).length };"##,
            r##"const kind = "detail"
let label
switch (kind) {
  case "list":
    label = kind === "list" ? "all" : "some"
    break
  case "detail": {
    for (const t of ["get_meetings", "list_meetings"]) {
      await tools.granola[t]({ accountId: "a", input: {} })
    }
    break
  }
}
return label"##,
        ] {
            assert!(!slack_write_in_code(program, false), "{program}");
        }
        // Read-only programs those fixes must still let through.
        for program in [
            r##"async function gh(tool, input) {
  const r = await tools.github[tool](input);
  return JSON.parse(r.content[0].text);
}
const prs = await gh('list_pull_requests', { owner: 'acme', repo: 'web', state: 'open' });
return prs.map((p) => ({
  title: p.title,
  body: (p.body ?? '').replace(/<!--[\s\S]*?-->/g, '').trim().slice(0, 300),
}));"##,
            r##"const tool = 'list_issues';
const r = await tools.linear[tool]({ first: 20 });
const nodes = JSON.parse(r.content[0].text).nodes;
const counts = {};
for (const n of nodes) {
  let tool = n.labels?.[0]?.name ?? 'none';
  tool = tool.toLowerCase();
  counts[tool] = (counts[tool] ?? 0) + 1;
}
return counts;"##,
            r##"for (const t of ['notion-search', 'notion-get-users']) {
  const r = await tools.notion[t]({ query: 'protocol' });
  console.log(t, r.content[0].text.length);
}"##,
            r##"async function run(tool) {
  const r = await tools.linear[tool]({ first: 10 });
  return r.content[0].text.length;
}
const names = ['list_issues', 'list_projects'];
const sizes = await Promise.all(names.map(run));
return Object.fromEntries(names.map((n, i) => [n, sizes[i]]));"##,
            r##"const PATTERN = /meeting_id="([^"]+)"/g
function g(tool, input) {
  return tools.granola[tool](input)
}
const r = await g('list_meetings', {})
return [...r.content[0].text.matchAll(PATTERN)].map((m) => m[1])"##,
            r##"let tool = null;
const wantProjects = true;
if (wantProjects) tool = 'list_projects';
else tool = 'list_issues';
const r = await tools.linear[tool]({ first: 10 });
return r.content[0].text;"##,
        ] {
            assert!(!slack_write_in_code(program, false), "{program}");
        }
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
