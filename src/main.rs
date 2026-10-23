//! slack-approval-gateway: a local desktop app that is the approval gateway
//! for agent-initiated Slack messages. Agents call the MCP tool
//! `request_slack_message` over stdio; a human approves, denies, or gives
//! feedback in a desktop popup; only the gateway ever talks to Slack.

use slack_approval_gateway::core::{
    Decision, DecisionProvider, Gateway, MessageRequest, ProviderError,
};
use slack_approval_gateway::mcp;
use slack_approval_gateway::slack::UreqSlackClient;
use slack_approval_gateway::ui::{self, UiDecisionProvider};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::sync::Arc;

fn state_dir() -> PathBuf {
    if let Ok(d) = std::env::var("SLACK_GATEWAY_STATE_DIR") {
        if !d.is_empty() {
            return PathBuf::from(d);
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".slack-approval-gateway")
}

/// Fallback decision provider for headless use (`--cli`): prompts on the
/// controlling terminal. Never used when the GUI is available.
struct CliDecisionProvider;

impl DecisionProvider for CliDecisionProvider {
    fn request_decision(&self, request: &MessageRequest) -> Result<Decision, ProviderError> {
        let r = &request.incoming;
        let mut tty_w = std::fs::OpenOptions::new()
            .write(true)
            .open("/dev/tty")
            .map_err(|e| ProviderError(format!("cannot open /dev/tty: {e}")))?;
        let tty_r = std::fs::File::open("/dev/tty")
            .map_err(|e| ProviderError(format!("cannot open /dev/tty: {e}")))?;
        fn out(tty: &mut std::fs::File, line: &str) {
            let _ = tty.write_all(line.as_bytes());
            let _ = tty.write_all(b"\n");
            let _ = tty.flush();
        }
        out(&mut tty_w, &format!("\n=== Slack message approval request {} ===", request.id));
        out(&mut tty_w, &format!("Agent:         {}", r.agent));
        out(&mut tty_w, &format!(
            "Conversation:  {} ({})",
            r.conversation.title, r.conversation.id_or_link
        ));
        out(&mut tty_w, &format!("  summary: {}", r.conversation.summary));
        out(&mut tty_w, &format!("Destination:   {}{}", r.destination, match &r.thread_ts {
            Some(t) => format!(" (Slack thread {t})"),
            None => String::new(),
        }));
        out(&mut tty_w, &format!("Reason:        {}", r.reason));
        out(&mut tty_w, "Proposed message (exact text):");
        out(&mut tty_w, &r.message);
        let mut reader = BufReader::new(tty_r);
        loop {
            out(&mut tty_w, "Approve [a] / Deny [d] / Feedback [f <text>]:");
            let mut line = String::new();
            reader
                .read_line(&mut line)
                .map_err(|e| ProviderError(format!("cannot read decision: {e}")))?;
            let line = line.trim();
            if line == "a" || line.eq_ignore_ascii_case("approve") {
                return Ok(Decision::Approve);
            } else if line == "d" || line.eq_ignore_ascii_case("deny") {
                return Ok(Decision::Deny);
            } else if let Some(rest) = line.strip_prefix('f') {
                let feedback = rest.trim().to_string();
                if feedback.is_empty() {
                    out(&mut tty_w, "Feedback text must not be empty.");
                    continue;
                }
                return Ok(Decision::Feedback(feedback));
            } else if !line.is_empty() {
                out(&mut tty_w, "Please answer a, d, or f <feedback>.");
            }
        }
    }
}

fn build_gateway(provider: Arc<dyn DecisionProvider>, dir: &PathBuf) -> Gateway {
    let slack = match UreqSlackClient::from_env(dir) {
        Ok(c) => Arc::new(c),
        Err(e) => {
            eprintln!("slack-approval-gateway: cannot initialize Slack client: {}", e.0);
            eprintln!("Set SLACK_BOT_TOKEN, or create {}/slack_token (chmod 600) with a bot token.", dir.display());
            eprintln!("The token needs the chat:write and chat:write.public scopes.");
            std::process::exit(1);
        }
    };
    match Gateway::new(slack, provider, dir.clone()) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("slack-approval-gateway: cannot initialize state in {}: {e}", dir.display());
            std::process::exit(1);
        }
    }
}

fn serve_mcp(gateway: Arc<Gateway>) {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    if let Err(e) = mcp::run(gateway, stdin.lock(), stdout) {
        eprintln!("slack-approval-gateway: MCP server error: {e}");
    }
}

fn main() {
    let cli_mode = std::env::args().any(|a| a == "--cli");
    let dir = state_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("slack-approval-gateway: cannot create state dir {}: {e}", dir.display());
        std::process::exit(1);
    }

    if cli_mode {
        let gateway = Arc::new(build_gateway(Arc::new(CliDecisionProvider), &dir));
        serve_mcp(gateway);
        return;
    }

    // GUI mode: MCP server on a background thread, popup on the main thread.
    let (provider, rx) = UiDecisionProvider::channel();
    let gateway = Arc::new(build_gateway(Arc::new(provider), &dir));
    let mcp_gateway = Arc::clone(&gateway);
    std::thread::spawn(move || serve_mcp(mcp_gateway));
    if let Err(e) = ui::run_ui(rx) {
        eprintln!("slack-approval-gateway: UI error: {e}");
        std::process::exit(1);
    }
}
