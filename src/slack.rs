//! Slack sending. This module is the ONLY place Slack credentials live.
//! Agents never see the token; they only talk to the gateway over MCP.

use crate::core::{SlackClient, SlackError, SlackSend};
use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;

/// Slack Web API base URL used when `SLACK_API_BASE` is not set.
const DEFAULT_API_BASE: &str = "https://slack.com/api";

/// Upper bound on each Slack API call (connect, send, and read the reply).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// Slack Web API client backed by `ureq`.
pub struct UreqSlackClient {
    token: String,
    api_base: String,
    agent: ureq::Agent,
}

impl UreqSlackClient {
    /// Build from the environment: `SLACK_BOT_TOKEN` (required) or a token
    /// file at `<state_dir>/slack_token` (one line, trimmed; create it with
    /// `chmod 600`). `SLACK_API_BASE` overrides the API base URL (default
    /// `https://slack.com/api`) and exists for tests.
    pub fn from_env(state_dir: &Path) -> Result<Self, SlackError> {
        let token = match non_empty_env("SLACK_BOT_TOKEN") {
            Some(token) => token,
            None => read_token_file(&state_dir.join("slack_token"))?,
        };
        if token.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(SlackError(
                "the Slack token must be a single token with no spaces or line breaks".to_string(),
            ));
        }
        let api_base = non_empty_env("SLACK_API_BASE").unwrap_or_else(|| DEFAULT_API_BASE.to_string());
        Ok(Self::new(token, api_base))
    }

    /// Build explicitly (used by tests with a mock API base).
    pub fn new(token: impl Into<String>, api_base: impl Into<String>) -> Self {
        let api_base = api_base.into().trim().trim_end_matches('/').to_string();
        // Statuses are inspected by hand so Slack's JSON error body can be
        // reported. The Slack API never redirects, so a redirect is treated as
        // an error instead of being followed.
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(REQUEST_TIMEOUT))
            .http_status_as_error(false)
            .max_redirects(0)
            .build()
            .new_agent();
        Self {
            token: token.into(),
            api_base,
            agent,
        }
    }

    /// `POST {api_base}/{method}` with a JSON body (plus optional query
    /// arguments) and return the parsed JSON reply. Non-2xx statuses and
    /// transport failures become `SlackError`; the token never appears in
    /// error text.
    fn call(&self, method: &str, body: &Value, query: &[(&str, &str)]) -> Result<Value, SlackError> {
        let mut request = self
            .agent
            .post(format!("{}/{}", self.api_base, method))
            .header("Authorization", format!("Bearer {}", self.token));
        for (key, value) in query {
            request = request.query(*key, *value);
        }
        let mut response = request.send_json(body).map_err(|e| request_error(method, e))?;
        let status = response.status();
        if !status.is_success() {
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            // Slack usually explains a non-2xx status (e.g. 429) in a JSON body.
            let code = response
                .body_mut()
                .read_json::<Value>()
                .ok()
                .and_then(|v| v.get("error").and_then(Value::as_str).map(str::to_string));
            let mut msg = format!("{method}: HTTP {}", status.as_u16());
            if let Some(code) = code {
                msg.push_str(&format!(" ({code})"));
            }
            if let Some(secs) = retry_after {
                msg.push_str(&format!("; retry after {secs}s"));
            }
            return Err(SlackError(msg));
        }
        // Slack already accepted the request, so a failure here (stalled or
        // truncated body, malformed JSON) must not read as "nothing happened".
        response.body_mut().read_json::<Value>().map_err(|e| {
            SlackError(format!(
                "{method}: Slack answered HTTP {} but the reply could not be read ({e}); the request may have taken effect",
                status.as_u16()
            ))
        })
    }

    /// Resolve a message permalink via `chat.getPermalink`. Returns `None`
    /// (and logs to stderr) on any failure so the caller can fall back.
    fn permalink(&self, channel: &str, ts: &str) -> Option<String> {
        let body = json!({ "channel": channel, "message_ts": ts });
        // Slack documents chat.getPermalink with form/query arguments, so the
        // same arguments also go in the query string; the JSON body follows
        // the gateway's contract.
        let result = self.call(
            "chat.getPermalink",
            &body,
            &[("channel", channel), ("message_ts", ts)],
        );
        match result {
            Ok(reply) if is_ok(&reply) => {
                let link = reply.get("permalink").and_then(Value::as_str).unwrap_or("");
                if link.is_empty() {
                    eprintln!("slack-approval-gateway: chat.getPermalink returned no permalink; using a constructed link");
                    None
                } else {
                    Some(link.to_string())
                }
            }
            Ok(reply) => {
                eprintln!(
                    "slack-approval-gateway: chat.getPermalink failed ({}); using a constructed link",
                    error_code(&reply)
                );
                None
            }
            Err(e) => {
                eprintln!("slack-approval-gateway: {}; using a constructed link", e.0);
                None
            }
        }
    }
}

impl std::fmt::Debug for UreqSlackClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UreqSlackClient")
            .field("token", &"<redacted>")
            .field("api_base", &self.api_base)
            .finish()
    }
}

impl SlackClient for UreqSlackClient {
    /// `chat.postMessage` with the exact `text` to the exact `destination`,
    /// optionally as a reply in thread `thread_ts`. Resolves a permalink via
    /// `chat.getPermalink`, falling back to a constructed archives URL.
    fn post_message(
        &self,
        destination: &str,
        thread_ts: Option<&str>,
        text: &str,
    ) -> Result<SlackSend, SlackError> {
        let mut body = json!({ "channel": destination, "text": text });
        if let Some(thread_ts) = thread_ts {
            body["thread_ts"] = Value::String(thread_ts.to_string());
        }
        let reply = self.call("chat.postMessage", &body, &[])?;
        if !is_ok(&reply) {
            return Err(SlackError(error_code(&reply)));
        }
        let ts = match reply.get("ts").and_then(Value::as_str) {
            Some(ts) if !ts.is_empty() => ts.to_string(),
            _ => {
                return Err(SlackError(
                    "chat.postMessage: Slack replied ok but without a message ts; the message may have been posted"
                        .to_string(),
                ))
            }
        };
        // Slack replies with the resolved channel ID (e.g. `#general` -> `C…`).
        let channel = match reply.get("channel").and_then(Value::as_str) {
            Some(channel) if !channel.is_empty() => channel.to_string(),
            _ => destination.to_string(),
        };
        let permalink = self
            .permalink(&channel, &ts)
            .unwrap_or_else(|| fallback_permalink(&channel, &ts));
        Ok(SlackSend {
            channel,
            ts,
            permalink,
        })
    }
}

/// The value of env var `name`, trimmed; `None` if unset or blank.
fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn read_token_file(path: &Path) -> Result<String, SlackError> {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(SlackError(format!(
                "no Slack token: SLACK_BOT_TOKEN is not set and {} does not exist",
                path.display()
            )))
        }
        Err(e) => {
            return Err(SlackError(format!(
                "cannot read Slack token file {}: {e}",
                path.display()
            )))
        }
    };
    let token = contents.trim();
    if token.is_empty() {
        return Err(SlackError(format!("Slack token file {} is empty", path.display())));
    }
    warn_if_readable_by_others(path);
    Ok(token.to_string())
}

#[cfg(unix)]
fn warn_if_readable_by_others(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::metadata(path) {
        if meta.permissions().mode() & 0o077 != 0 {
            eprintln!(
                "slack-approval-gateway: warning: {} is accessible by other users; run `chmod 600` on it",
                path.display()
            );
        }
    }
}

#[cfg(not(unix))]
fn warn_if_readable_by_others(_path: &Path) {}

fn is_ok(reply: &Value) -> bool {
    reply.get("ok").and_then(Value::as_bool) == Some(true)
}

/// Slack's `error` code from a `{ok: false}` reply, or `unknown_error`.
fn error_code(reply: &Value) -> String {
    match reply.get("error").and_then(Value::as_str) {
        Some(code) if !code.is_empty() => code.to_string(),
        _ => "unknown_error".to_string(),
    }
}

fn request_error(method: &str, err: ureq::Error) -> SlackError {
    match err {
        ureq::Error::Timeout(_) => SlackError(format!(
            "{method}: no reply from Slack within {}s; the request may still have reached Slack",
            REQUEST_TIMEOUT.as_secs()
        )),
        other => SlackError(format!("{method}: request to Slack failed: {other}")),
    }
}

/// `https://<SLACK_WORKSPACE_DOMAIN or "workspace">.slack.com/archives/{channel}/p{ts without '.'}`.
fn fallback_permalink(channel: &str, ts: &str) -> String {
    let domain = non_empty_env("SLACK_WORKSPACE_DOMAIN")
        .map(|d| {
            d.trim_start_matches("https://")
                .trim_start_matches("http://")
                .trim_end_matches('/')
                .trim_end_matches(".slack.com")
                .to_string()
        })
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| "workspace".to_string());
    format!(
        "https://{domain}.slack.com/archives/{channel}/p{}",
        ts.replace('.', "")
    )
}
