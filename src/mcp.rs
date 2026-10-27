//! MCP server over stdio (newline-delimited JSON-RPC 2.0).
//!
//! Exposes exactly one tool, `request_slack_message`, which agents call to
//! request approval for a Slack message. Protocol messages are the only
//! thing written to the output stream; diagnostics go to stderr.
//!
//! One extra method, [`APPROVE_METHOD`], is not a tool: SpeakEasy's own
//! pre-tool hook (`src/hook.rs`) calls it on the instance socket to ask the
//! human about an agent's own Slack tool call. It only returns a decision
//! and never sends anything.

use crate::core::{
    ApprovalOptions, ApprovalVerdict, ConversationContext, Gateway, IncomingRequest,
    RequestVerdict,
};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

/// The single tool agents may call.
pub const TOOL_NAME: &str = "request_slack_message";

/// Protocol version answered when the client does not request one.
const DEFAULT_PROTOCOL_VERSION: &str = "2025-06-18";

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const INTERNAL_ERROR: i64 = -32603;

/// Top-level tool arguments; anything else is rejected so a misspelled
/// field (e.g. `thread` for `thread_ts`) cannot silently change where a
/// message goes.
const ARGUMENT_FIELDS: [&str; 6] = [
    "message",
    "destination",
    "thread_ts",
    "reason",
    "agent",
    "conversation",
];

const CONVERSATION_FIELDS: [&str; 3] = ["title", "id_or_link", "summary"];

/// The approval-only method used by SpeakEasy's pre-tool hook. Its params
/// are the tool arguments plus `wait_secs` (1..=[`MAX_APPROVAL_WAIT_SECS`]):
/// how long the human has before the request is withdrawn; an optional
/// `share_key` (identifies one agent tool call; hooks stopping the same call
/// share one decision); and an optional `cooldown` (default true; false
/// when the call names no destination).
pub const APPROVE_METHOD: &str = "speakeasy/approve";

/// Clients that give up on a tool call after a fixed time without telling
/// the server (no `notifications/cancelled`), by `clientInfo.name`: their
/// `request_slack_message` calls are withdrawn from the popup just before,
/// so the human never approves a message the agent was already told failed.
/// Zed's built-in agent caps every MCP request at 600 s.
const CLIENT_CALL_LIMITS: [(&str, Duration); 1] = [("Zed", Duration::from_secs(590))];

/// One connection's `tools/call` requests that are still waiting.
#[derive(Default)]
struct Calls {
    /// Cancel flags of in-flight calls, by JSON-encoded request id.
    in_flight: Mutex<HashMap<String, Arc<AtomicBool>>>,
    /// How long a call may wait for the human (from the client's
    /// `initialize`, see [`CLIENT_CALL_LIMITS`]); `None`: no limit.
    limit: Mutex<Option<Duration>>,
}

/// Upper bound for `wait_secs`.
pub const MAX_APPROVAL_WAIT_SECS: u64 = 24 * 3600;

const COOLDOWN_NOTE: &str =
    "a similar request was denied recently; nothing was sent and no approval prompt was shown";

/// Run the MCP server: read JSON-RPC messages from `input` until EOF, write
/// responses to `output`. `tools/call` requests are handled on spawned
/// threads (responses are serialized through a mutex), so a request waiting
/// for a human decision never blocks other requests. Returns when `input`
/// reaches EOF.
pub fn run<W: Write + Send + 'static>(
    gateway: Arc<Gateway>,
    mut input: impl BufRead,
    output: W,
) -> std::io::Result<()> {
    let output = Arc::new(Mutex::new(output));
    // Set when `input` ends: approval requests from this connection are then
    // withdrawn (the hook that asked is gone), and so are its `tools/call`
    // requests still waiting for the human (the agent can't get the result).
    let closed = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(Calls::default());
    let _mark_closed = CloseOnDrop(Arc::clone(&closed), Arc::clone(&calls));
    let mut line = Vec::new();
    loop {
        line.clear();
        match input.read_until(b'\n', &mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
        // Bytes rather than `lines()`: invalid UTF-8 is a parse error for
        // that one message, not a fatal read error.
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let message = match serde_json::from_slice::<Value>(&line) {
            Ok(message) => message,
            Err(e) => {
                log(format_args!("MCP parse error: {e}"));
                let response =
                    error_response(&Value::Null, PARSE_ERROR, format!("Parse error: {e}"));
                write_message(&output, &response)?;
                continue;
            }
        };
        if let Some(response) = dispatch(&gateway, &output, &closed, &calls, message) {
            write_message(&output, &response)?;
        }
    }
    // In-flight `tools/call` threads are withdrawn (see `_mark_closed`); a
    // call already approved still finishes sending and answers if the peer
    // is listening. They never hold up shutdown.
    Ok(())
}

/// Handle one parsed JSON-RPC message. Returns the response to write now, or
/// `None` for notifications, client responses, and `tools/call` requests
/// handed to a worker thread (which writes its own response).
fn dispatch<W: Write + Send + 'static>(
    gateway: &Arc<Gateway>,
    output: &Arc<Mutex<W>>,
    closed: &Arc<AtomicBool>,
    calls: &Arc<Calls>,
    message: Value,
) -> Option<Value> {
    let Value::Object(mut message) = message else {
        return Some(error_response(
            &Value::Null,
            INVALID_REQUEST,
            "Invalid Request: a JSON-RPC message must be an object (batches are not supported)",
        ));
    };
    let id = message.remove("id");
    // MCP ids are strings or integers; JSON-RPC also allows other numbers.
    // Any other id cannot be echoed, so errors about it carry a null id.
    let valid_id = id
        .clone()
        .filter(|id| matches!(id, Value::String(_) | Value::Number(_)));
    let error_id = valid_id.clone().unwrap_or(Value::Null);
    if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Some(error_response(
            &error_id,
            INVALID_REQUEST,
            "Invalid Request: `jsonrpc` must be \"2.0\"",
        ));
    }
    let method = match message.get("method") {
        Some(Value::String(method)) => method.clone(),
        Some(_) => {
            return Some(error_response(
                &error_id,
                INVALID_REQUEST,
                "Invalid Request: `method` must be a string",
            ))
        }
        // A response to a request we never sent: nothing to do.
        None if message.contains_key("result") || message.contains_key("error") => return None,
        None => {
            return Some(error_response(
                &error_id,
                INVALID_REQUEST,
                "Invalid Request: missing `method`",
            ))
        }
    };
    if method == "notifications/cancelled" {
        // The client gave up on a call (its own timeout, or the user
        // interrupted the agent): withdraw it from the popup.
        let request_id = message
            .get("params")
            .and_then(|p| p.get("requestId"))
            .map(Value::to_string);
        if let Some(request_id) = request_id {
            let calls = calls.in_flight.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(cancel) = calls.get(&request_id) {
                log(format_args!("tools/call {request_id} cancelled by the client"));
                cancel.store(true, Ordering::Relaxed);
            }
        }
        return None;
    }
    if method.starts_with("notifications/") {
        return None;
    }
    let Some(id) = valid_id else {
        let problem = if id.is_some() {
            "`id` must be a string or a number".to_string()
        } else {
            // Every MCP notification is under `notifications/`, so anything
            // else without an id is a request that forgot its id.
            format!("`{method}` is a request and needs an `id`")
        };
        return Some(error_response(
            &Value::Null,
            INVALID_REQUEST,
            format!("Invalid Request: {problem}"),
        ));
    };
    let params = message.remove("params").unwrap_or(Value::Null);
    if !matches!(params, Value::Null | Value::Object(_) | Value::Array(_)) {
        return Some(error_response(
            &id,
            INVALID_REQUEST,
            "Invalid Request: `params` must be an object or an array",
        ));
    }

    match method.as_str() {
        "initialize" => {
            let client = params
                .get("clientInfo")
                .and_then(|c| c.get("name"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            let limit = CLIENT_CALL_LIMITS
                .iter()
                .find(|(name, _)| client.eq_ignore_ascii_case(name))
                .map(|(_, limit)| *limit);
            *calls.limit.lock().unwrap_or_else(PoisonError::into_inner) = limit;
            Some(result_response(&id, initialize_result(&params)))
        }
        "ping" => Some(result_response(&id, json!({}))),
        "tools/list" => Some(result_response(
            &id,
            json!({ "tools": [tool_definition()] }),
        )),
        APPROVE_METHOD => match parse_approval(&params) {
            Ok((request, wait, share_key, cooldown)) => {
                spawn_approval(
                    Arc::clone(gateway),
                    Arc::clone(output),
                    Arc::clone(closed),
                    id,
                    request,
                    wait,
                    share_key,
                    cooldown,
                );
                None
            }
            Err(problem) => Some(error_response(
                &id,
                INVALID_PARAMS,
                format!("Invalid params: {problem}"),
            )),
        },
        "tools/call" => match parse_tool_call(&params) {
            Ok(request) => {
                spawn_tool_call(
                    Arc::clone(gateway),
                    Arc::clone(output),
                    Arc::clone(calls),
                    id,
                    request,
                );
                None
            }
            Err(problem) => Some(error_response(
                &id,
                INVALID_PARAMS,
                format!("Invalid params: {problem}"),
            )),
        },
        _ => Some(error_response(
            &id,
            METHOD_NOT_FOUND,
            format!("Method not found: {method}"),
        )),
    }
}

fn initialize_result(params: &Value) -> Value {
    let version = params
        .get("protocolVersion")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .unwrap_or(DEFAULT_PROTOCOL_VERSION);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": {
            "name": "slack-approval-gateway",
            "version": env!("CARGO_PKG_VERSION"),
        },
    })
}

fn tool_definition() -> Value {
    json!({
        "name": TOOL_NAME,
        "description": "Request human approval to send a Slack message. Nothing is sent until a \
            human explicitly approves this exact request in the SpeakEasy desktop \
            popup; only then is the exact `message` sent to the exact `destination` (and \
            `thread_ts`, if given). Any change to the content or destination needs a new request \
            and a fresh approval. The call blocks until the human decides. The result is a JSON \
            object whose `status` is one of: `sent` (with the Slack `ts` and `permalink`; \
            `duplicate: true` means an identical request was already approved and sent, so \
            nothing was sent again); `feedback` (nothing was sent; revise using `feedback` and \
            submit a new request); `denied` (nothing was sent; requests with the same purpose to \
            the same destination, including reworded ones and ones from other agents, are \
            blocked for 10 minutes); `cooldown_blocked` (a similar request was denied recently; \
            nothing was sent, no popup was shown, and `cooldown_remaining_secs` says how long \
            the block lasts); or `error` (the request failed; usually nothing was sent, but if \
            the `error` text says the message may have been sent, check the destination before \
            resubmitting). This gateway is the only way to send Slack messages.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "description": "The exact proposed message text. It is sent verbatim if approved.",
                },
                "destination": {
                    "type": "string",
                    "description": "Slack recipient: a channel (`#name` or channel ID) or a user (`@name` or user ID).",
                },
                "thread_ts": {
                    "type": "string",
                    "description": "Optional. Timestamp (`ts`, e.g. \"1700000000.000100\") of the parent message of the destination Slack thread, to send the message as a thread reply. Omit to post at the top level.",
                },
                "reason": {
                    "type": "string",
                    "description": "Why the agent wants to send this message.",
                },
                "agent": {
                    "type": "string",
                    "description": "Identity of the requesting agent.",
                },
                "conversation": {
                    "type": "object",
                    "description": "Context from the originating agent conversation (not a Slack thread).",
                    "properties": {
                        "title": {
                            "type": "string",
                            "description": "Title of the originating agent conversation.",
                        },
                        "id_or_link": {
                            "type": "string",
                            "description": "ID or link of the originating agent conversation.",
                        },
                        "summary": {
                            "type": "string",
                            "description": "Brief summary of the originating agent conversation relevant to this message.",
                        },
                    },
                    "required": CONVERSATION_FIELDS,
                    "additionalProperties": false,
                },
            },
            "required": ["message", "destination", "reason", "agent", "conversation"],
            "additionalProperties": false,
        },
    })
}

/// Validate `tools/call` params and build the gateway request. The error
/// lists every problem found, for the `-32602` response.
fn parse_tool_call(params: &Value) -> Result<IncomingRequest, String> {
    let Value::Object(params) = params else {
        return Err("`params` must be an object with `name` and `arguments`".into());
    };
    match params.get("name") {
        Some(Value::String(name)) if name == TOOL_NAME => {}
        Some(Value::String(name)) => {
            return Err(format!(
                "unknown tool `{name}`; the only tool is `{TOOL_NAME}`"
            ))
        }
        Some(_) => return Err("`name` must be a string".into()),
        None => return Err("missing tool `name`".into()),
    }
    let arguments = match params.get("arguments") {
        Some(Value::Object(arguments)) => arguments,
        Some(_) => return Err("`arguments` must be an object".into()),
        None => return Err(format!("missing `arguments` for `{TOOL_NAME}`")),
    };

    parse_arguments(arguments, &ARGUMENT_FIELDS)
}

/// Validate `speakeasy/approve` params: the tool arguments plus
/// `wait_secs`.
type ApprovalParams = (IncomingRequest, u64, Option<String>, bool);

fn parse_approval(params: &Value) -> Result<ApprovalParams, String> {
    let Value::Object(params) = params else {
        return Err("`params` must be an object".into());
    };
    let mut allowed = ARGUMENT_FIELDS.to_vec();
    allowed.extend(["wait_secs", "share_key", "cooldown"]);
    let share_key = match params.get("share_key") {
        None | Some(Value::Null) => None,
        Some(Value::String(c)) => Some(c.clone()),
        Some(_) => return Err("`share_key` must be a string".into()),
    };
    let cooldown = match params.get("cooldown") {
        None | Some(Value::Null) => true,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err("`cooldown` must be a boolean".into()),
    };
    let wait = match params.get("wait_secs") {
        Some(w) => match w.as_u64() {
            Some(w) if (1..=MAX_APPROVAL_WAIT_SECS).contains(&w) => Ok(w),
            _ => Err(format!(
                "`wait_secs` must be an integer from 1 to {MAX_APPROVAL_WAIT_SECS}"
            )),
        },
        None => Err("missing required argument `wait_secs`".to_string()),
    };
    match (parse_arguments(params, &allowed), wait) {
        (Ok(request), Ok(wait)) => Ok((request, wait, share_key, cooldown)),
        (Err(a), Err(b)) => Err(format!("{a}; {b}")),
        (Err(problem), _) | (_, Err(problem)) => Err(problem),
    }
}

/// Validate the request fields shared by the tool and the approval method.
fn parse_arguments(
    arguments: &Map<String, Value>,
    allowed: &[&str],
) -> Result<IncomingRequest, String> {
    let mut problems = Vec::new();
    reject_unknown_fields(arguments, allowed, "", &mut problems);
    let message = required_string(arguments, "message", "message", &mut problems);
    let destination = required_string(arguments, "destination", "destination", &mut problems);
    let thread_ts = optional_thread_ts(arguments, &mut problems);
    let reason = required_string(arguments, "reason", "reason", &mut problems);
    let agent = required_string(arguments, "agent", "agent", &mut problems);
    let conversation = match arguments.get("conversation") {
        Some(Value::Object(conversation)) => {
            reject_unknown_fields(
                conversation,
                &CONVERSATION_FIELDS,
                "conversation.",
                &mut problems,
            );
            let title = required_string(conversation, "title", "conversation.title", &mut problems);
            let id_or_link = required_string(
                conversation,
                "id_or_link",
                "conversation.id_or_link",
                &mut problems,
            );
            let summary = required_string(
                conversation,
                "summary",
                "conversation.summary",
                &mut problems,
            );
            Some(ConversationContext {
                title,
                id_or_link,
                summary,
            })
        }
        Some(_) => {
            problems.push("`conversation` must be an object".to_string());
            None
        }
        None => {
            problems.push("missing required argument `conversation`".to_string());
            None
        }
    };

    match conversation {
        Some(conversation) if problems.is_empty() => Ok(IncomingRequest {
            message,
            destination,
            thread_ts,
            reason,
            agent,
            conversation,
        }),
        _ => Err(problems.join("; ")),
    }
}

fn reject_unknown_fields(
    object: &Map<String, Value>,
    allowed: &[&str],
    prefix: &str,
    problems: &mut Vec<String>,
) {
    for key in object.keys().filter(|k| !allowed.contains(&k.as_str())) {
        problems.push(format!("unknown argument `{prefix}{key}`"));
    }
}

/// A required, non-blank string field, returned verbatim (never trimmed:
/// the message must be sent exactly as approved). Records a problem and
/// returns an empty string otherwise.
fn required_string(
    object: &Map<String, Value>,
    key: &str,
    path: &str,
    problems: &mut Vec<String>,
) -> String {
    match object.get(key) {
        Some(Value::String(s)) if !s.trim().is_empty() => s.clone(),
        Some(Value::String(_)) => {
            problems.push(format!("`{path}` must not be empty"));
            String::new()
        }
        Some(_) => {
            problems.push(format!("`{path}` must be a string"));
            String::new()
        }
        None => {
            problems.push(format!("missing required argument `{path}`"));
            String::new()
        }
    }
}

/// `thread_ts` is optional (`null` counts as absent). When present it must
/// be a Slack message timestamp, so a link or other value pasted by mistake
/// is rejected before a human approves a reply to the wrong place.
fn optional_thread_ts(
    arguments: &Map<String, Value>,
    problems: &mut Vec<String>,
) -> Option<String> {
    match arguments.get("thread_ts") {
        None | Some(Value::Null) => None,
        Some(Value::String(ts)) if is_slack_ts(ts) => Some(ts.clone()),
        Some(Value::String(_)) => {
            problems.push(
                "`thread_ts` must be the Slack timestamp of the thread's parent message, \
                 such as \"1700000000.000100\" (not a link)"
                    .to_string(),
            );
            None
        }
        Some(_) => {
            problems.push("`thread_ts` must be a string".to_string());
            None
        }
    }
}

/// Slack message timestamps look like `1700000000.000100`.
pub(crate) fn is_slack_ts(ts: &str) -> bool {
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    ts.split_once('.')
        .is_some_and(|(secs, micros)| digits(secs) && digits(micros))
}

/// Handle a validated `tools/call` on its own thread so a request waiting
/// for a human decision never blocks the read loop or other requests. A
/// `notifications/cancelled` for it, or the client's call limit, withdraws
/// it from the popup if no one has decided yet.
fn spawn_tool_call<W: Write + Send + 'static>(
    gateway: Arc<Gateway>,
    output: Arc<Mutex<W>>,
    calls: Arc<Calls>,
    id: Value,
    request: IncomingRequest,
) {
    let key = id.to_string();
    let cancel = Arc::new(AtomicBool::new(false));
    calls
        .in_flight
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(key.clone(), Arc::clone(&cancel));
    let deadline = calls
        .limit
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .map(|limit| Instant::now() + limit);
    let worker_output = Arc::clone(&output);
    let worker_id = id.clone();
    let worker_calls = Arc::clone(&calls);
    let worker_key = key.clone();
    let spawned = thread::Builder::new()
        .name("mcp-tools-call".into())
        .spawn(move || {
            let verdict = panic::catch_unwind(AssertUnwindSafe(|| {
                gateway.handle_request_until(request, deadline, &cancel)
            }))
            .unwrap_or_else(|_| {
                RequestVerdict::Error(
                    "internal gateway error while handling the request; check the \
                     destination before retrying, as the message may have been sent"
                        .into(),
                )
            });
            worker_calls
                .in_flight
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&worker_key);
            // A cancelled request gets no response (MCP), unless it was sent.
            if cancel.load(Ordering::Relaxed) && !matches!(verdict, RequestVerdict::Sent { .. }) {
                return;
            }
            let response = result_response(&worker_id, tool_result(verdict));
            if let Err(e) = write_message(&worker_output, &response) {
                log(format_args!("cannot write tools/call response: {e}"));
            }
        });
    if let Err(e) = spawned {
        calls
            .in_flight
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&key);
        // The closure (and the request) was dropped unrun: nothing was shown
        // to the human and nothing was sent.
        log(format_args!("cannot start a tools/call worker: {e}"));
        let response = error_response(
            &id,
            INTERNAL_ERROR,
            "Internal error: the gateway could not start a worker; nothing was sent",
        );
        if let Err(e) = write_message(&output, &response) {
            log(format_args!("cannot write tools/call response: {e}"));
        }
    }
}

/// Handle a validated `speakeasy/approve` on its own thread (it blocks until
/// the human decides or `wait_secs` pass).
#[allow(clippy::too_many_arguments)]
fn spawn_approval<W: Write + Send + 'static>(
    gateway: Arc<Gateway>,
    output: Arc<Mutex<W>>,
    closed: Arc<AtomicBool>,
    id: Value,
    request: IncomingRequest,
    wait_secs: u64,
    share_key: Option<String>,
    cooldown: bool,
) {
    let worker_output = Arc::clone(&output);
    let worker_id = id.clone();
    let spawned = thread::Builder::new()
        .name("speakeasy-approve".into())
        .spawn(move || {
            let opts = ApprovalOptions {
                deadline: Instant::now() + Duration::from_secs(wait_secs),
                share_key,
                cooldown,
            };
            let verdict = panic::catch_unwind(AssertUnwindSafe(|| {
                gateway.handle_approval(request, &opts, &closed)
            }))
                    .unwrap_or_else(|_| {
                        ApprovalVerdict::Error("internal gateway error while asking the human".into())
                    });
            let response = result_response(&worker_id, approval_result(verdict));
            if let Err(e) = write_message(&worker_output, &response) {
                log(format_args!("cannot write {APPROVE_METHOD} response: {e}"));
            }
        });
    if let Err(e) = spawned {
        log(format_args!("cannot start an approval worker: {e}"));
        let response = error_response(
            &id,
            INTERNAL_ERROR,
            "Internal error: the gateway could not start a worker; no decision was made",
        );
        if let Err(e) = write_message(&output, &response) {
            log(format_args!("cannot write {APPROVE_METHOD} response: {e}"));
        }
    }
}

/// Sets the flag, and cancels the connection's in-flight `tools/call`
/// requests, when dropped (the read loop ended, by EOF or error).
struct CloseOnDrop(Arc<AtomicBool>, Arc<Calls>);

impl Drop for CloseOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
        let calls = self.1.in_flight.lock().unwrap_or_else(PoisonError::into_inner);
        for cancel in calls.values() {
            cancel.store(true, Ordering::Relaxed);
        }
    }
}

/// The `speakeasy/approve` result: `{"decision": …}` plus details.
fn approval_result(verdict: ApprovalVerdict) -> Value {
    match verdict {
        ApprovalVerdict::Approved => json!({ "decision": "approved" }),
        ApprovalVerdict::Denied {
            cooldown_remaining_secs,
        } => json!({ "decision": "denied", "cooldown_remaining_secs": cooldown_remaining_secs }),
        ApprovalVerdict::Feedback { feedback } => {
            json!({ "decision": "feedback", "feedback": feedback })
        }
        ApprovalVerdict::CooldownBlocked {
            cooldown_remaining_secs,
        } => json!({
            "decision": "cooldown_blocked",
            "cooldown_remaining_secs": cooldown_remaining_secs,
        }),
        ApprovalVerdict::AlreadySent { ts, permalink } => {
            json!({ "decision": "already_sent", "ts": ts, "permalink": permalink })
        }
        ApprovalVerdict::Error(error) => json!({ "decision": "error", "error": error }),
    }
}

/// The `tools/call` result for a gateway verdict: the status object as JSON
/// text content, mirrored in `structuredContent`.
fn tool_result(verdict: RequestVerdict) -> Value {
    let (payload, is_error) = match verdict {
        RequestVerdict::Sent {
            ts,
            permalink,
            duplicate,
        } => (
            json!({ "status": "sent", "ts": ts, "permalink": permalink, "duplicate": duplicate }),
            false,
        ),
        RequestVerdict::Denied {
            cooldown_remaining_secs,
        } => (
            json!({ "status": "denied", "cooldown_remaining_secs": cooldown_remaining_secs }),
            false,
        ),
        RequestVerdict::Feedback { feedback } => {
            (json!({ "status": "feedback", "feedback": feedback }), false)
        }
        RequestVerdict::CooldownBlocked {
            cooldown_remaining_secs,
        } => (
            json!({
                "status": "cooldown_blocked",
                "cooldown_remaining_secs": cooldown_remaining_secs,
                "note": COOLDOWN_NOTE,
            }),
            false,
        ),
        RequestVerdict::Error(error) => (json!({ "status": "error", "error": error }), true),
    };
    json!({
        "content": [{ "type": "text", "text": payload.to_string() }],
        "structuredContent": payload,
        "isError": is_error,
    })
}

fn result_response(id: &Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error_response(id: &Value, code: i64, message: impl Into<String>) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message.into() },
    })
}

/// Write one message as a single line. The whole line is written under the
/// lock, so responses from concurrent workers never interleave.
fn write_message<W: Write>(output: &Mutex<W>, message: &Value) -> std::io::Result<()> {
    // Compact JSON escapes control characters, so the line has no raw newline.
    let mut line = serde_json::to_vec(message).map_err(std::io::Error::other)?;
    line.push(b'\n');
    let mut output = output.lock().unwrap_or_else(PoisonError::into_inner);
    output.write_all(&line)?;
    output.flush()
}

/// Write a diagnostic line to stderr. Unlike `eprintln!`, this never panics
/// (e.g. when the client has closed the stderr pipe), so logging cannot take
/// the server down.
fn log(message: std::fmt::Arguments<'_>) {
    let _ = writeln!(std::io::stderr(), "slack-approval-gateway: {message}");
}
