//! Slack sending. This module is the ONLY place Slack credentials live.
//! Agents never see the token; they only talk to the gateway over MCP.
//!
//! Two transports implement [`SlackClient`]:
//! - [`UreqSlackClient`] (default) calls the Slack Web API with a bot token
//!   held by this process (`SLACK_BOT_TOKEN` or `<state_dir>/slack_token`).
//! - [`crate::mistral_slack::MistralConnectorSlackClient`] sends through the
//!   Mistral Slack Connector; the Slack credentials then live in the Mistral
//!   console and this process only holds a Mistral API key.
//!
//! [`resolve_transport`] picks one from `SLACK_TRANSPORT` (or
//! `<state_dir>/slack_transport`) and [`build_client`] constructs it; both
//! transports enforce the same exclusive-sending path: agents reach Slack
//! only through the gateway.

use crate::core::{SlackClient, SlackError, SlackSend};
use crate::mistral_slack::MistralConnectorSlackClient;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;
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
            None => read_secret_file(
                &state_dir.join("slack_token"),
                "SLACK_BOT_TOKEN",
                "Slack bot token",
            )?,
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

impl UreqSlackClient {
    /// `Drake Thomsen (@drake.thomsen)` out of a `users.info` user object.
    fn user_display(user: &Value) -> Option<String> {
        let real_name = user.get("real_name").and_then(Value::as_str);
        let display_name = user
            .pointer("/profile/display_name")
            .and_then(Value::as_str);
        let handle = user.get("name").and_then(Value::as_str);
        let name = [real_name, display_name, handle]
            .into_iter()
            .flatten()
            .find(|s| !s.is_empty())?;
        let handle = handle.filter(|h| *h != name);
        Some(match handle {
            Some(h) => format!("{name} (@{h})"),
            None => name.to_string(),
        })
    }

    /// `users.info` for a user ID; `None` (with a stderr note) on any failure.
    fn user_display_by_id(&self, user_id: &str) -> Option<String> {
        let reply = self
            .call("users.info", &json!({ "user": user_id }), &[])
            .map_err(|e| {
                eprintln!("slack-approval-gateway: users.info failed ({}); showing the raw destination", e.0);
                e
            })
            .ok()?;
        if !is_ok(&reply) {
            eprintln!(
                "slack-approval-gateway: users.info failed ({}); showing the raw destination",
                error_code(&reply)
            );
            return None;
        }
        reply.get("user").and_then(Self::user_display)
    }

    /// `conversations.info` for a channel/group/DM ID: `#name` for
    /// channels, the DM partner's `Name (@handle)` for DMs. `None` on any
    /// failure.
    fn channel_display_by_id(&self, channel_id: &str) -> Option<String> {
        let reply = self
            .call("conversations.info", &json!({ "channel": channel_id }), &[])
            .map_err(|e| {
                eprintln!("slack-approval-gateway: conversations.info failed ({}); showing the raw destination", e.0);
                e
            })
            .ok()?;
        if !is_ok(&reply) {
            eprintln!(
                "slack-approval-gateway: conversations.info failed ({}); showing the raw destination",
                error_code(&reply)
            );
            return None;
        }
        let channel = reply.get("channel")?;
        if channel.get("is_im").and_then(Value::as_bool) == Some(true) {
            let user_id = channel.get("user").and_then(Value::as_str)?;
            return self.user_display_by_id(user_id);
        }
        let name = channel.get("name").and_then(Value::as_str).filter(|s| !s.is_empty())?;
        Some(format!("#{name}"))
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

    /// Best-effort display name for the approval popup: `U…` user IDs via
    /// `users.info`, `C…`/`G…`/`D…` channel IDs via `conversations.info`
    /// (DMs resolve to the partner's name). `#name`/`@name` destinations
    /// are already readable and return `None`. Needs the `users:read` and
    /// `channels:read`/`groups:read`/`im:read` scopes; without them the
    /// lookups fail and the raw destination is shown.
    fn resolve_destination(&self, destination: &str) -> Option<String> {
        let d = destination.trim();
        if d.len() < 2 || d.starts_with('#') || d.starts_with('@') {
            return None;
        }
        let (kind, _) = d.split_at(1);
        match kind {
            "U" => self.user_display_by_id(d),
            "C" | "G" | "D" => self.channel_display_by_id(d),
            _ => None,
        }
    }
}

/// The value of env var `name`, trimmed; `None` if unset or blank.
pub(crate) fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Read a one-line secret from `path` when env var `env_var` is not set.
/// The error names both places so the operator knows where to put the value.
pub(crate) fn read_secret_file(
    path: &Path,
    env_var: &str,
    what: &str,
) -> Result<String, SlackError> {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(SlackError(format!(
                "no {what}: {env_var} is not set and {} does not exist",
                path.display()
            )))
        }
        Err(e) => {
            return Err(SlackError(format!(
                "cannot read {what} file {}: {e}",
                path.display()
            )))
        }
    };
    let secret = contents.trim();
    if secret.is_empty() {
        return Err(SlackError(format!(
            "{what} file {} is empty",
            path.display()
        )));
    }
    warn_if_readable_by_others(path);
    Ok(secret.to_string())
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
pub(crate) fn fallback_permalink(channel: &str, ts: &str) -> String {
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

/// How the gateway sends to Slack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlackTransport {
    /// Slack Web API with a bot token held by this process (default).
    Direct,
    /// Mistral Slack Connector; Slack credentials live in the Mistral console.
    MistralConnector,
}

/// The transport selected for this run: `SLACK_TRANSPORT` env var, else a
/// one-line `<state_dir>/slack_transport` file, else [`SlackTransport::Direct`].
/// Accepted values: `direct`, `mistral-connector` (also `mistral_connector` /
/// `connector`).
pub fn resolve_transport(state_dir: &Path) -> Result<SlackTransport, SlackError> {
    let selected = match non_empty_env("SLACK_TRANSPORT") {
        Some(value) => Some(value),
        None => match std::fs::read_to_string(state_dir.join("slack_transport")) {
            Ok(contents) => {
                let value = contents.trim().to_string();
                if value.is_empty() {
                    None
                } else {
                    Some(value)
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => {
                return Err(SlackError(format!(
                    "cannot read Slack transport file {}: {e}",
                    state_dir.join("slack_transport").display()
                )))
            }
        },
    };
    match selected.as_deref() {
        None => Ok(SlackTransport::Direct),
        Some("direct") => Ok(SlackTransport::Direct),
        Some("mistral-connector") | Some("mistral_connector") | Some("connector") => {
            Ok(SlackTransport::MistralConnector)
        }
        Some(other) => Err(SlackError(format!(
            "unknown SLACK_TRANSPORT value '{other}' (expected 'direct' or 'mistral-connector')"
        ))),
    }
}

/// Build the Slack client for the selected transport. The credential error
/// names what the chosen transport needs: a Slack bot token for `direct`, a
/// Mistral API key for `mistral-connector`.
pub fn build_client(state_dir: &Path) -> Result<Arc<dyn SlackClient>, SlackError> {
    match resolve_transport(state_dir)? {
        SlackTransport::Direct => Ok(Arc::new(UreqSlackClient::from_env(state_dir)?)),
        SlackTransport::MistralConnector => Ok(Arc::new(MistralConnectorSlackClient::from_env(
            state_dir,
        )?)),
    }
}
