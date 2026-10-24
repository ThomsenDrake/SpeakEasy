//! Slack sending. This module is the ONLY place Slack credentials live.
//! Agents never see the token; they only talk to the gateway over MCP.
//!
//! Two transports implement [`SlackClient`]:
//! - [`crate::slack_mcp::SlackMcpClient`] (default) sends through the
//!   official Slack MCP server (remote, HTTP) using a Slack **user** OAuth
//!   token held by this process (`SLACK_MCP_TOKEN` or
//!   `<state_dir>/slack_mcp_token`). The gateway never holds a Slack bot
//!   token: it is not a Slack app and cannot post as one.
//! - [`crate::mistral_slack::MistralConnectorSlackClient`] sends through the
//!   Mistral Slack Connector; the Slack credentials then live in the Mistral
//!   console and this process only holds a Mistral API key.
//!
//! [`resolve_transport`] picks one from `SLACK_TRANSPORT` (or
//! `<state_dir>/slack_transport`) and [`build_client`] constructs it; both
//! transports enforce the same exclusive-sending path: agents reach Slack
//! only through the gateway.

use crate::core::{SlackClient, SlackError};
use crate::mistral_slack::MistralConnectorSlackClient;
use crate::slack_mcp::SlackMcpClient;
use std::path::Path;
use std::sync::Arc;

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
    /// The official Slack MCP server (remote, HTTP) with a Slack user OAuth
    /// token held by this process (default).
    SlackMcp,
    /// Mistral Slack Connector; Slack credentials live in the Mistral console.
    MistralConnector,
}

/// The transport selected for this run: `SLACK_TRANSPORT` env var, else a
/// one-line `<state_dir>/slack_transport` file, else
/// [`SlackTransport::SlackMcp`]. Accepted values: `slack-mcp` (also `mcp` /
/// `slack_mcp`) and `mistral-connector` (also `mistral_connector` /
/// `connector`). The old `direct` transport (a Slack bot token held by the
/// gateway) was removed on purpose: the gateway is not a Slack app.
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
        None => Ok(SlackTransport::SlackMcp),
        Some("slack-mcp") | Some("mcp") | Some("slack_mcp") => Ok(SlackTransport::SlackMcp),
        Some("mistral-connector") | Some("mistral_connector") | Some("connector") => {
            Ok(SlackTransport::MistralConnector)
        }
        Some(other @ ("direct" | "bot-token" | "token")) => Err(SlackError(format!(
            "the '{other}' transport (a Slack bot token held by the gateway) was removed: \
             the gateway now sends through the official Slack MCP server or the Mistral \
             Slack Connector; use 'slack-mcp' (default) or 'mistral-connector'"
        ))),
        Some(other) => Err(SlackError(format!(
            "unknown SLACK_TRANSPORT value '{other}' (expected 'slack-mcp' or 'mistral-connector')"
        ))),
    }
}

/// Build the Slack client for the selected transport. The credential error
/// names what the chosen transport needs: a Slack MCP user token for
/// `slack-mcp`, a Mistral API key for `mistral-connector`.
pub fn build_client(state_dir: &Path) -> Result<Arc<dyn SlackClient>, SlackError> {
    match resolve_transport(state_dir)? {
        SlackTransport::SlackMcp => Ok(Arc::new(SlackMcpClient::from_env(state_dir)?)),
        SlackTransport::MistralConnector => Ok(Arc::new(MistralConnectorSlackClient::from_env(
            state_dir,
        )?)),
    }
}
