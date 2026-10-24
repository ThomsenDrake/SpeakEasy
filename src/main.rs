//! slack-approval-gateway: a local desktop app that is the approval gateway
//! for agent-initiated Slack messages. Agents call the MCP tool
//! `request_slack_message` over stdio; a human approves, denies, or gives
//! feedback in a desktop popup; only the gateway ever talks to Slack.
//!
//! Process model (see `src/instance.rs`): exactly one *instance* per state
//! dir owns the popup, the Slack client, and the pending-request queue, and
//! serves MCP both on its own stdio and on a Unix socket. Processes spawned
//! by other agents become *bridges* to that instance. The instance is
//! normally kept running by a LaunchAgent (`--serve`); closing the popup
//! window only hides it — the gateway keeps running, and closing never
//! approves anything.

use slack_approval_gateway::core::{
    Decision, DecisionProvider, Gateway, MessageRequest, ProviderError,
};
use slack_approval_gateway::slack;
use slack_approval_gateway::{instance, mcp, ui};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::Manager;

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

/// Serializes CLI prompts: concurrent `tools/call` workers share /dev/tty,
/// and a typed answer must never be attributed to the wrong request.
static CLI_PROMPT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

impl DecisionProvider for CliDecisionProvider {
    fn request_decision(&self, request: &MessageRequest) -> Result<Decision, ProviderError> {
        let _guard = CLI_PROMPT_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let destination = match &request.destination_display {
            Some(display) => format!("{display} ({})", r.destination),
            None => r.destination.clone(),
        };
        out(&mut tty_w, &format!("Destination:   {destination}{}", match &r.thread_ts {
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
    let slack = match slack::build_client(dir) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("slack-approval-gateway: cannot initialize Slack client: {}", e.0);
            eprintln!("Slack MCP transport (default): set SLACK_MCP_TOKEN, or create {}/slack_mcp_token (chmod 600)", dir.display());
            eprintln!("with a Slack user OAuth token (run scripts/slack_mcp_oauth.py to get one;");
            eprintln!("it needs the chat:write, users:read and channels:read scopes).");
            eprintln!("Mistral Slack Connector transport (SLACK_TRANSPORT=mistral-connector): set MISTRAL_API_KEY,");
            eprintln!("or create {}/mistral_api_key (chmod 600), and connect the Slack workspace to the", dir.display());
            eprintln!("Slack connector in the Mistral console.");
            eprintln!("Korotovsky Slack MCP transport (SLACK_TRANSPORT=korotovsky): set SLACK_BOT_TOKEN (xoxb-…)");
            eprintln!("or SLACK_USER_TOKEN (xoxp-…), or create {}/slack_bot_token / slack_user_token", dir.display());
            eprintln!("(chmod 600) — see README.md.");
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

/// Show and focus the approval window. The window exists from app start
/// (hidden); a request arriving at any time brings it to the front.
fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Menu-bar tray icon: the gateway's always-available affordance. A left
/// click (or the Show item) reopens the window; Quit stops the gateway.
fn setup_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let menu = Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, "sag-show", "Show approval window", true, None::<&str>)?,
            &MenuItem::with_id(app, "sag-quit", "Quit SpeakEasy", true, None::<&str>)?,
        ],
    )?;
    let mut builder = TrayIconBuilder::with_id("slack-approval-gateway")
        .menu(&menu)
        .tooltip("SpeakEasy")
        .icon_as_template(true)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

/// Become the single gateway instance for this state dir, or exit.
fn become_instance(dir: &PathBuf) -> (instance::InstanceGuard, std::os::unix::net::UnixListener) {
    match instance::acquire(dir) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("slack-approval-gateway: cannot become the gateway instance: {e}");
            std::process::exit(1);
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cli_mode = args.iter().any(|a| a == "--cli");
    let serve_mode = args.iter().any(|a| a == "--serve");
    let dir = state_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("slack-approval-gateway: cannot create state dir {}: {e}", dir.display());
        std::process::exit(1);
    }

    if cli_mode {
        // Headless: be the instance, answer decisions on the terminal, and
        // still serve the socket so bridged agents reach this instance.
        let (_guard, listener) = become_instance(&dir);
        let gateway = Arc::new(build_gateway(Arc::new(CliDecisionProvider), &dir));
        instance::serve_connections(Arc::clone(&gateway), listener);
        serve_mcp(Arc::clone(&gateway));
        // stdin reached EOF; let any approved send finish and be recorded
        // before the process exits.
        gateway.wait_until_no_send_in_progress();
        return;
    }

    // Default mode: spawned by an agent over MCP stdio. If an instance is
    // already running, become a bridge to it instead of starting a second
    // popup owner.
    if !serve_mode {
        if let Some(stream) = instance::connect_existing(&dir) {
            instance::run_bridge(stream);
        }
    }

    // Become the single instance (a service manager passes --serve and
    // starts us with no agent on stdio).
    let (_guard, listener) = match instance::acquire(&dir) {
        Ok(x) => x,
        Err(instance::AcquireError::AlreadyRunning) => {
            // Lost the startup race: another process just became the
            // instance. Bridge to it like any other spawned process.
            match instance::connect_existing(&dir) {
                Some(stream) => instance::run_bridge(stream),
                None => {
                    eprintln!(
                        "slack-approval-gateway: another instance holds the lock \
                         but does not answer on the socket"
                    );
                    std::process::exit(1);
                }
            }
        }
        Err(e) => {
            eprintln!("slack-approval-gateway: cannot become the gateway instance: {e}");
            std::process::exit(1);
        }
    };

    // We are the instance: popup UI, Slack client, MCP on our own stdio
    // (unless started by the service manager), and MCP on the socket for
    // every other agent's bridge.
    let ui_state = Arc::new(ui::UiState::new());
    let provider = Arc::new(ui::TauriDecisionProvider::new(Arc::clone(&ui_state)));
    let gateway = Arc::new(build_gateway(provider, &dir));
    ui_state.set_gateway(Arc::clone(&gateway));
    instance::serve_connections(Arc::clone(&gateway), listener);
    if !serve_mode {
        let mcp_gateway = Arc::clone(&gateway);
        std::thread::spawn(move || serve_mcp(mcp_gateway));
    }
    let ui_state_for_setup = Arc::clone(&ui_state);
    let app = tauri::Builder::default()
        .manage(ui_state)
        .setup(move |app| {
            ui_state_for_setup.set_app(app.handle().clone());
            if let Some(window) = app.get_webview_window("main") {
                eprintln!(
                    "slack-approval-gateway: main window created (visible={})",
                    window.is_visible().unwrap_or(false)
                );
            } else {
                eprintln!("slack-approval-gateway: main window missing at setup");
            }
            if let Err(e) = setup_tray(app.handle()) {
                eprintln!("slack-approval-gateway: cannot set up tray icon: {e}");
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ui::submit_decision,
            ui::ui_ready,
            ui::get_active_cooldowns,
            ui::hide_window,
            ui::quit_app
        ])
        .on_window_event(|window, event| {
            // Closing the popup only hides it. The gateway keeps serving
            // MCP for every agent; closing never approves anything.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .on_menu_event(|app, event| match event.id().as_ref() {
            "sag-show" => show_main_window(app),
            "sag-quit" => app.exit(0),
            _ => {}
        })
        .build(tauri::generate_context!());
    let app = match app {
        Ok(app) => app,
        Err(e) => {
            eprintln!("slack-approval-gateway: UI error: {e}");
            gateway.wait_until_no_send_in_progress();
            std::process::exit(1);
        }
    };
    app.run(|app, event| {
        // Dock icon / `open -a` with no visible window: reopen the popup.
        #[cfg(target_os = "macos")]
        if let tauri::RunEvent::Reopen {
            has_visible_windows: false,
            ..
        } = event
        {
            show_main_window(app);
        }
        #[cfg(not(target_os = "macos"))]
        let _ = (app, event);
    });
    // Quit via the tray: pending requests fail in the provider (never
    // approved). Let any approved send finish and be recorded before the
    // process exits.
    gateway.wait_until_no_send_in_progress();
}
