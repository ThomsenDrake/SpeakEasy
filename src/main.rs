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
//!
//! The popup is a Rust-native window (egui/eframe with the wgpu renderer —
//! no browser engine; see `src/ui.rs` and `src/ui/`). The window is created
//! hidden; a request arriving at any time reveals and focuses it, and the
//! menu-bar tray icon (Show / Quit) is always available.

use slack_approval_gateway::core::{
    Decision, DecisionProvider, Gateway, MessageRequest, ProviderError,
};
use slack_approval_gateway::slack;
use slack_approval_gateway::ui;
use slack_approval_gateway::{hook, instance, mcp};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

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
    /// A terminal prompt cannot be withdrawn, so the request is asked as
    /// usual and its answer is discarded when the deadline passed or the
    /// agent stopped waiting meanwhile (a hook that went away, or a
    /// cancelled `tools/call`).
    fn request_decision_until(
        &self,
        request: &MessageRequest,
        deadline: std::time::Instant,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<Decision, ProviderError> {
        use std::sync::atomic::Ordering;
        let gone = || cancel.load(Ordering::Relaxed) || std::time::Instant::now() >= deadline;
        // Wait for the terminal first: a request withdrawn while an earlier
        // prompt held it is never shown.
        let _guard = CLI_PROMPT_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if gone() {
            return Err(ProviderError("the agent that asked stopped waiting".into()));
        }
        let decision = self.prompt(request)?;
        if gone() {
            if let Ok(mut tty) = std::fs::OpenOptions::new().write(true).open("/dev/tty") {
                let _ = writeln!(
                    tty,
                    "That request was withdrawn before you answered (the agent stopped \
                     waiting, or no one answered in time); your answer was not used and \
                     nothing was sent."
                );
            }
            return Err(ProviderError("no one answered the approval request in time".into()));
        }
        Ok(decision)
    }

    fn request_decision(&self, request: &MessageRequest) -> Result<Decision, ProviderError> {
        let _guard = CLI_PROMPT_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.prompt(request)
    }
}

impl CliDecisionProvider {
    /// Show one request on the terminal and read the answer (the caller
    /// holds `CLI_PROMPT_LOCK`).
    fn prompt(&self, request: &MessageRequest) -> Result<Decision, ProviderError> {
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

/// The first line of a credential file in the state dir, if it has one.
fn fs_token(dir: &std::path::Path, name: &str) -> Option<String> {
    let text = std::fs::read_to_string(dir.join(name)).ok()?;
    let line = text.lines().next()?.trim().to_string();
    (!line.is_empty()).then_some(line)
}

/// Build the gateway, or print why it cannot start and return `None`. The
/// caller exits after dropping its `InstanceGuard`, so a failed start
/// leaves no stale socket or lock behind (a stale socket would look like a
/// running SpeakEasy to the installer and to the agents' hooks).
fn build_gateway(provider: Arc<dyn DecisionProvider>, dir: &PathBuf) -> Option<Gateway> {
    let slack = match slack::build_client(dir) {
        Ok(c) => c,
        Err(e) => {
            // Help for the transport in use, then the error itself last, so
            // the end of gateway.log (which the installer shows) names it.
            use slack::SlackTransport as T;
            let d = dir.display();
            match slack::resolve_transport(dir) {
                Ok(T::MistralConnector) => {
                    eprintln!("Mistral Slack Connector transport: set MISTRAL_API_KEY, or create {d}/mistral_api_key");
                    eprintln!("(chmod 600), and connect the Slack workspace to the Slack connector in the");
                    eprintln!("Mistral console.");
                }
                // A login is there (the "no Slack token" error comes before
                // the server is even looked up): the server itself failed.
                Ok(T::Korotovsky) if !e.0.starts_with("no Slack token") => {
                    eprintln!("Korotovsky Slack MCP transport: the Slack MCP server did not start (see below).");
                    eprintln!("Check that it is installed ({d}/slack_stdio_command, or slack-mcp-server on");
                    eprintln!("PATH) and that its login works: the token file, or {d}/slack_stdio_env");
                    eprintln!("(a browser-session login expires; if you took it over, sign in again there");
                    eprintln!("and re-run scripts/install.sh --take-over FILE:NAME).");
                }
                Ok(T::Korotovsky) => {
                    eprintln!("Korotovsky Slack MCP transport: set SLACK_BOT_TOKEN (xoxb-…) or SLACK_USER_TOKEN");
                    eprintln!("(xoxp-…), or create {d}/slack_bot_token / slack_user_token (chmod 600),");
                    eprintln!("or take over an agent's server with scripts/install.sh --take-over — see README.md.");
                    if dir.join("slack_stdio_env").exists() {
                        eprintln!("({d}/slack_stdio_env holds no usable login: it needs SLACK_MCP_XOXP_TOKEN or");
                        eprintln!("SLACK_MCP_XOXB_TOKEN, or both SLACK_MCP_XOXC_TOKEN and SLACK_MCP_XOXD_TOKEN.)");
                    }
                }
                Ok(T::HooksOnly) => {}
                Ok(T::SlackMcp)
                    if fs_token(dir, "slack_mcp_token")
                        .is_some_and(|t| !t.starts_with("xoxp-placeholder-")) =>
                {
                    eprintln!("Slack MCP transport: SpeakEasy has a token ({d}/slack_mcp_token), but the");
                    eprintln!("connection failed (see below): a revoked or expired token, or no network.");
                    eprintln!("If you took it over from an agent, sign in again there and re-run");
                    eprintln!("scripts/install.sh --take-over FILE:NAME; or run scripts/slack_mcp_oauth.py.");
                }
                Ok(T::SlackMcp) | Err(_) => {
                    eprintln!("Slack MCP transport: set SLACK_MCP_TOKEN, or create {d}/slack_mcp_token (chmod 600)");
                    eprintln!("with a Slack user OAuth token (run scripts/slack_mcp_oauth.py to get one; it needs");
                    eprintln!("the chat:write, users:read and channels:read scopes). With no Slack login at all,");
                    eprintln!("run scripts/install.sh --transport hooks-only.");
                }
            }
            eprintln!("slack-approval-gateway: cannot initialize Slack client: {}", e.0);
            return None;
        }
    };
    match Gateway::new(slack, provider, dir.clone()) {
        Ok(g) => Some(g),
        Err(e) => {
            eprintln!("slack-approval-gateway: cannot initialize state in {}: {e}", dir.display());
            None
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

/// Run the native approval window (egui/eframe, wgpu). Blocks until the
/// human quits (tray Quit / Cmd+Q); returns false if the UI could not run.
///
/// The window starts hidden (`visible: false`) and is revealed by the first
/// request (the reveal hook) or by the tray's Show item. Closing the window
/// only hides it; the gateway keeps serving MCP until Quit.
fn run_gui(
    ui_state: Arc<ui::UiState>,
    is_demo: bool,
    demo: ui::app::DemoFlags,
) -> bool {
    use slack_approval_gateway::ui::app::ApprovalApp;
    use slack_approval_gateway::ui::text;
    use slack_approval_gateway::ui::theme::Theme;

    // The UI event-loop hooks: gateway worker threads call these while the
    // bridge lock is held, so they only send viewport commands (cheap, and
    // they never call back into the bridge).
    let ctx_slot: Arc<Mutex<Option<egui::Context>>> = Arc::new(Mutex::new(None));
    let reveal_slot = Arc::clone(&ctx_slot);
    let notify_slot = Arc::clone(&ctx_slot);
    let hide_slot = Arc::clone(&ctx_slot);
    let quit_slot = Arc::clone(&ctx_slot);
    ui_state.install_hooks(ui::UiHooks {
        reveal: Some(Box::new(move || {
            if let Some(ctx) = reveal_slot.lock().unwrap().as_ref() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                ctx.request_repaint();
            }
        })),
        notify: Some(Box::new(move || {
            if let Some(ctx) = notify_slot.lock().unwrap().as_ref() {
                ctx.request_repaint();
            }
        })),
        hide: Some(Box::new(move || {
            if let Some(ctx) = hide_slot.lock().unwrap().as_ref() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                ctx.request_repaint();
            }
        })),
        quit: Some(Box::new(move || {
            if let Some(ctx) = quit_slot.lock().unwrap().as_ref() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                ctx.request_repaint();
            }
        })),
    });

    let icon = match eframe::icon_data::from_png_bytes(include_bytes!("../icons/icon.png")) {
        Ok(icon) => Some(icon),
        Err(e) => {
            eprintln!("slack-approval-gateway: cannot load window icon: {e}");
            None
        }
    };
    // The reference window: 720×780 logical px, min 520×560, resizable,
    // decorated, centered, and initially hidden.
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("SpeakEasy")
        .with_inner_size([720.0, 780.0])
        .with_min_inner_size([520.0, 560.0])
        .with_resizable(true)
        .with_visible(false);
    // Demo only: `SLACK_GATEWAY_DEMO_SIZE=WxH` overrides the initial size.
    if is_demo {
        if let Some(size) = ui::demo::demo_window_size() {
            viewport = viewport.with_inner_size(size);
        }
    }
    if let Some(icon) = icon {
        viewport = viewport.with_icon(icon);
    }
    let opts = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    let reduce_motion = ui::sys::reduce_motion();
    let ctx_slot_for_app = Arc::clone(&ctx_slot);
    let result = eframe::run_native(
        "SpeakEasy",
        opts,
        Box::new(move |cc| {
            // Install SF Pro / SF Mono before the first frame (no-op families
            // on non-macOS systems, where the bundled/system fallback fonts
            // are used instead).
            text::install_fonts(&cc.egui_ctx);
            // The reference (Chrome) places text baselines at fractional
            // positions; egui's default rounds every galley origin to whole
            // device pixels, which would quantize the baseline correction in
            // `ui::text::baseline_correction` away. The glyph bitmaps already
            // carry their own subpixel ink offsets, so unrounded origins keep
            // the text crisp.
            cc.egui_ctx
                .tessellation_options_mut(|opts| opts.round_text_to_pixels = false);
            Ok(Box::new(ApprovalApp::new(
                ui_state,
                ctx_slot_for_app,
                // The app re-detects the theme (env override, then the system
                // appearance) on its first logic pass.
                Theme::Light,
                reduce_motion,
                is_demo,
                demo,
            )))
        }),
    );
    match result {
        Ok(()) => true,
        Err(e) => {
            eprintln!("slack-approval-gateway: UI error: {e}");
            false
        }
    }
}

/// Demo mode (`--demo`): the native popup with synthetic fixtures, fully
/// isolated from production (throwaway state dir, in-memory Slack client,
/// no instance lock, no socket, no MCP stdio, no credentials). Never sends
/// real messages.
fn run_demo() {
    use slack_approval_gateway::core::DecisionProvider;
    use slack_approval_gateway::ui::demo;

    let dir = demo::demo_state_dir();
    let seed = demo::DemoSeed::from_env();
    demo::seed_state(&dir, &seed.cooldowns());
    let ui_state = Arc::new(ui::UiState::new());
    let provider: Arc<dyn DecisionProvider> =
        Arc::new(ui::GuiDecisionProvider::new(Arc::clone(&ui_state)));
    let gateway = match Gateway::new(Arc::new(demo::DemoSlack::default()), provider, dir) {
        Ok(g) => Arc::new(g),
        Err(e) => {
            eprintln!("slack-approval-gateway: demo: cannot initialize state: {e}");
            std::process::exit(1);
        }
    };
    ui_state.set_gateway(Arc::clone(&gateway));
    // The demo requests go through the real gateway (cooldown/dedupe/
    // decision plumbing) on worker threads, exactly like MCP workers.
    demo::spawn_demo_requests(&gateway, &seed.requests());
    let ui_ok = run_gui(
        ui_state,
        true,
        ui::app::DemoFlags {
            sticky_deciding: demo::demo_sticky_deciding(),
            fail_submit: demo::demo_fail_submit(),
        },
    );
    // Let any approved (recorded, in-memory) send finish before exiting.
    gateway.wait_until_no_send_in_progress();
    if !ui_ok {
        std::process::exit(1);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // Pre-tool hook mode: answer one agent hook call and exit. Checked
    // first: it never becomes the instance or a bridge.
    if let Some(parsed) = hook::parse_args(&args) {
        match parsed {
            Ok(hook_args) => hook::main(hook_args, &state_dir()),
            Err(problem) => {
                // A misconfigured hook must not let a Slack call through:
                // exit 2 blocks in every Claude-style agent.
                eprintln!("slack-approval-gateway: {problem}");
                std::process::exit(2);
            }
        }
    }
    // Answer these without starting (or relaying to) SpeakEasy.
    if args.iter().skip(1).any(|a| a == "--version" || a == "-V") {
        println!("slack-approval-gateway {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if args.iter().skip(1).any(|a| a == "--help" || a == "-h") {
        println!(
            "SpeakEasy (slack-approval-gateway) {}\n\n\
             Run by your coding agents as an MCP server over stdio, and as the\n\
             login service (--serve). Install it with scripts/install.sh.\n\n\
             Usage:\n  \
             slack-approval-gateway            MCP server over stdio (agents start it)\n  \
             slack-approval-gateway --serve    the background service (login item)\n  \
             slack-approval-gateway --cli      ask on this terminal instead of a window\n  \
             slack-approval-gateway --demo     show the window with sample requests\n  \
             slack-approval-gateway --hook AGENT [--wait SECS]\n                                    \
             approval hook for an agent's own Slack tools\n  \
             slack-approval-gateway --version\n\n\
             README: https://github.com/ThomsenDrake/SpeakEasy",
            env!("CARGO_PKG_VERSION")
        );
        return;
    }
    let cli_mode = args.iter().any(|a| a == "--cli");
    let serve_mode = args.iter().any(|a| a == "--serve");
    let demo_mode = args.iter().any(|a| a == "--demo");
    let dir = state_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("slack-approval-gateway: cannot create state dir {}: {e}", dir.display());
        std::process::exit(1);
    }

    if cli_mode {
        // Headless: be the instance, answer decisions on the terminal, and
        // still serve the socket so bridged agents reach this instance.
        let (guard, listener) = become_instance(&dir);
        let Some(gateway) = build_gateway(Arc::new(CliDecisionProvider), &dir) else {
            drop(guard);
            std::process::exit(1);
        };
        let gateway = Arc::new(gateway);
        instance::serve_connections(Arc::clone(&gateway), listener);
        serve_mcp(Arc::clone(&gateway));
        // stdin reached EOF; let any approved send finish and be recorded
        // before the process exits.
        gateway.wait_until_no_send_in_progress();
        return;
    }

    if demo_mode {
        run_demo();
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
    let (guard, listener) = match instance::acquire(&dir) {
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
    let provider = Arc::new(ui::GuiDecisionProvider::new(Arc::clone(&ui_state)));
    let Some(gateway) = build_gateway(provider, &dir) else {
        drop(guard);
        std::process::exit(1);
    };
    let gateway = Arc::new(gateway);
    ui_state.set_gateway(Arc::clone(&gateway));
    instance::serve_connections(Arc::clone(&gateway), listener);
    if !serve_mode {
        let mcp_gateway = Arc::clone(&gateway);
        std::thread::spawn(move || serve_mcp(mcp_gateway));
    }
    let ui_state_for_quit = Arc::clone(&ui_state);
    let ui_ok = run_gui(ui_state, false, ui::app::DemoFlags::default());
    if !ui_ok {
        // The UI never ran: no human can decide anything, so fail every
        // pending request (never approved) instead of hanging.
        eprintln!("slack-approval-gateway: UI failed; quitting the gateway");
        ui_state_for_quit.quit();
    }
    // Quit via the tray: pending requests fail in the provider (never
    // approved). Let any approved send finish and be recorded before the
    // process exits.
    gateway.wait_until_no_send_in_progress();
    if !ui_ok {
        std::process::exit(1);
    }
}
