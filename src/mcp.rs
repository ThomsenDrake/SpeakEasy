//! MCP server over stdio (newline-delimited JSON-RPC 2.0).
//!
//! Exposes exactly one tool, `request_slack_message`, which agents call to
//! request approval for a Slack message. Protocol messages are the only
//! thing written to the output stream; diagnostics go to stderr.

use crate::core::Gateway;
use std::io::{BufRead, Write};
use std::sync::Arc;

/// The single tool agents may call.
pub const TOOL_NAME: &str = "request_slack_message";

/// Run the MCP server: read JSON-RPC messages from `input` until EOF, write
/// responses to `output`. `tools/call` requests are handled on spawned
/// threads (responses are serialized through a mutex), so a request waiting
/// for a human decision never blocks other requests. Returns when `input`
/// reaches EOF.
pub fn run<W: Write + Send + 'static>(
    gateway: Arc<Gateway>,
    input: impl BufRead,
    output: W,
) -> std::io::Result<()> {
    unimplemented!()
}
