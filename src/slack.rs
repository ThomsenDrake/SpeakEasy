//! Slack sending. This module is the ONLY place Slack credentials live.
//! Agents never see the token; they only talk to the gateway over MCP.

use crate::core::{SlackClient, SlackError, SlackSend};
use std::path::Path;

/// Slack Web API client backed by `ureq`.
pub struct UreqSlackClient {
    token: String,
    api_base: String,
}

impl UreqSlackClient {
    /// Build from the environment: `SLACK_BOT_TOKEN` (required) or a token
    /// file at `<state_dir>/slack_token` (one line, trimmed; create it with
    /// `chmod 600`). `SLACK_API_BASE` overrides the API base URL (default
    /// `https://slack.com/api`) and exists for tests.
    pub fn from_env(state_dir: &Path) -> Result<Self, SlackError> {
        unimplemented!()
    }

    /// Build explicitly (used by tests with a mock API base).
    pub fn new(token: impl Into<String>, api_base: impl Into<String>) -> Self {
        unimplemented!()
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
        unimplemented!()
    }
}
