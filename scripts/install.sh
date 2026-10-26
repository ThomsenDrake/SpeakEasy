#!/usr/bin/env bash
# install.sh — install SpeakEasy (the Slack approval gateway) into every
# coding agent on this Mac, and keep the gateway itself running as a login
# service.
#
# What it does:
#   1. Builds the app (frontend + release binary).
#   2. Installs the binary to ~/.local/bin/slack-approval-gateway.
#   3. Sets up the state dir, the Slack transport selection, and the
#      credential files (chmod 600). The gateway is the only component
#      that ever sees a credential.
#   4. Installs a LaunchAgent so the single gateway instance starts at login
#      (--serve: UI + MCP socket, no agent on stdio). Agent-spawned copies
#      of the binary bridge to this instance over the state-dir socket, so
#      there is exactly one popup owner, one Slack client, and one queue.
#   5. Registers the MCP server `slack-approval-gateway` in every detected
#      coding agent (Claude Code, pdev/paradev, Codex, OpenCode, Vibe, goose,
#      pi, Gemini CLI, Copilot CLI, Zed, Amp, Hermes).
#   6. Drops a managed instruction block into each agent's instruction file:
#      "send Slack only through the gateway's request_slack_message tool".
#   7. Audits the exclusivity boundary: the official Slack MCP servers and
#      Mistral connector servers agents are expected to have (reported,
#      kept), direct (token-based) Slack MCP servers and Slack/Mistral
#      credentials in agent configs, and the legacy hook guard. With
#      --enforce it removes the direct token-based Slack MCP servers
#      (backups kept).
#
# Every modified config file is backed up once to <file>.bak-sag.
#
# Usage:
#   scripts/install.sh [--dry-run] [--enforce] [--uninstall] [--purge]
#                      [--skip-build] [--slack-mcp-token xoxp-...]
#                      [--transport slack-mcp|mistral-connector|korotovsky]
#                      [--mistral-api-key KEY] [--slack-bot-token xoxb-...]
#                      [--slack-user-token xoxp-...] [--workspace-domain DOMAIN]
#
#   --dry-run     print what would happen, change nothing
#   --enforce     also remove direct (token-based) Slack MCP servers from
#                 agent configs (agents then cannot send except through the
#                 gateway; the pre-install files stay as <file>.bak-sag).
#                 The official Slack MCP / Mistral connector servers are kept:
#                 agents are expected to have them for reading Slack
#                 context, and the instruction block + hook guard are the
#                 boundary for their send tools.
#   --uninstall   remove the MCP registrations, instruction blocks, the
#                 LaunchAgent, and the installed binary (state dir kept)
#   --purge       with --uninstall: also delete the state dir (cooldowns,
#                 sent records, token file)
#   --skip-build  reuse the existing target/release binary
#   --slack-mcp-token  Slack user OAuth token (xoxp-...) for the official
#                 Slack MCP server. Stored only in
#                 <state-dir>/slack_mcp_token (chmod 600). Without it a
#                 placeholder is written and sends fail until you get a real
#                 token:  scripts/slack_mcp_oauth.py --client-id ID
#                 --client-secret SECRET   (an existing token file is kept
#                 unless this flag is given; $SLACK_MCP_TOKEN is only a
#                 fallback)
#   --transport   how the gateway sends to Slack (default: slack-mcp):
#                 slack-mcp         the official Slack MCP server
#                                    (https://mcp.slack.com/mcp, remote MCP)
#                                    with the user token above;
#                 mistral-connector  the Mistral Slack Connector — Slack
#                                    credentials live in the Mistral console
#                                    (connect the workspace there first), and
#                                    the gateway only holds a Mistral API key;
#                 korotovsky       Korotovsky's unofficial Slack MCP server
#                                    (slack-mcp-server,
#                                    https://github.com/korotovsky/slack-mcp-server
#                                    — you install it; SpeakEasy never installs
#                                    or downloads it; spawned as a stdio child
#                                    process) with a Slack bot token
#                                    (xoxb-..., default) or user token
#                                    (xoxp-...) — see the flags below.
#   --mistral-api-key  Mistral API key, stored only in
#                 <state-dir>/mistral_api_key (chmod 600). Falls back to
#                 $MISTRAL_API_KEY. Required for the mistral-connector
#                 transport; without it a placeholder is written and sends
#                 fail until you set a real key (an existing key file is
#                 kept unless this flag is given).
#   --slack-bot-token  Slack bot token (xoxb-...) for the korotovsky
#                 transport. Stored only in <state-dir>/slack_bot_token
#                 (chmod 600). Falls back to $SLACK_BOT_TOKEN. The gateway
#                 passes it to the server it spawns; agents never see it.
#   --slack-user-token  Slack user token (xoxp-...) for the korotovsky
#                 transport, stored only in <state-dir>/slack_user_token
#                 (chmod 600). Falls back to $SLACK_USER_TOKEN. Either this
#                 or --slack-bot-token is required for the korotovsky
#                 transport (a bot token is the usual choice: the server
#                 posts as the app).
#   --workspace-domain  Slack workspace domain (e.g. acme for
#                 acme.slack.com), stored in <state-dir>/slack_workspace_domain.
#                 The korotovsky server's replies carry no permalink, so the
#                 gateway builds message links from this domain;
#                 $SLACK_WORKSPACE_DOMAIN overrides the file when the gateway
#                 runs.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN_NAME="slack-approval-gateway"
SERVER_NAME="slack-approval-gateway"
HOME_DIR="${HOME:?no HOME}"
DEFAULT_STATE_DIR="$HOME_DIR/.slack-approval-gateway"
INSTALL_BIN="$HOME_DIR/.local/bin/$BIN_NAME"
PLIST_LABEL="com.speakeasy.slack-approval-gateway"
PLIST_PATH="$HOME_DIR/Library/LaunchAgents/$PLIST_LABEL.plist"
AGENT_CONFIG="$ROOT/scripts/agent_config.py"
PY="${PYTHON:-python3}"

DRY_RUN=0
ENFORCE=0
UNINSTALL=0
PURGE=0
SKIP_BUILD=0
MCP_TOKEN_ARG=""
STATE_DIR="$DEFAULT_STATE_DIR"
TRANSPORT_ARG=""
MISTRAL_KEY_ARG=""
BOT_TOKEN_ARG=""
USER_TOKEN_ARG=""
WORKSPACE_DOMAIN_ARG=""

expect_value=""
for arg in "$@"; do
  if [ -n "$expect_value" ]; then
    case "$expect_value" in
      --slack-mcp-token) MCP_TOKEN_ARG="$arg" ;;
      --transport) TRANSPORT_ARG="$arg" ;;
      --mistral-api-key) MISTRAL_KEY_ARG="$arg" ;;
      --slack-bot-token) BOT_TOKEN_ARG="$arg" ;;
      --slack-user-token) USER_TOKEN_ARG="$arg" ;;
      --workspace-domain) WORKSPACE_DOMAIN_ARG="$arg" ;;
    esac
    expect_value=""
    continue
  fi
  case "$arg" in
    --dry-run) DRY_RUN=1 ;;
    --enforce) ENFORCE=1 ;;
    --uninstall) UNINSTALL=1 ;;
    --purge) PURGE=1 ;;
    --skip-build) SKIP_BUILD=1 ;;
    --slack-mcp-token|--transport|--mistral-api-key|--slack-bot-token|--slack-user-token|--workspace-domain)
      expect_value="$arg" ;;
    --slack-mcp-token=*) MCP_TOKEN_ARG="${arg#--slack-mcp-token=}" ;;
    --transport=*) TRANSPORT_ARG="${arg#--transport=}" ;;
    --mistral-api-key=*) MISTRAL_KEY_ARG="${arg#--mistral-api-key=}" ;;
    --slack-bot-token=*) BOT_TOKEN_ARG="${arg#--slack-bot-token=}" ;;
    --slack-user-token=*) USER_TOKEN_ARG="${arg#--slack-user-token=}" ;;
    --workspace-domain=*) WORKSPACE_DOMAIN_ARG="${arg#--workspace-domain=}" ;;
    --state-dir|--state-dir=*)
      echo "--state-dir is not supported: the service and the agents always use" >&2
      echo "~/.slack-approval-gateway — there is no per-install state dir to move" >&2
      exit 2 ;;
    -h|--help)
      # Print the whole header comment block (every leading '#' line after
      # the shebang), so the help survives edits that change its length.
      awk 'NR > 1 { if ($0 !~ /^#/) exit; sub(/^# ?/, ""); print }' "$0"
      exit 0
      ;;
    *) echo "unknown argument: $arg (see --help)" >&2; exit 2 ;;
  esac
done
if [ -n "$expect_value" ]; then
  echo "missing value for $expect_value (see --help)" >&2
  exit 2
fi

say() { printf '%s\n' "$*"; }
hr() { say ""; say "────────────────────────────────────────────────────────────"; }
run() {
  if [ "$DRY_RUN" = 1 ]; then say "  [dry-run] $*"; else "$@"; fi
}
run_ok() { # command that may legitimately fail
  if [ "$DRY_RUN" = 1 ]; then say "  [dry-run] $*"; else "$@" || true; fi
}
ac() { # agent_config.py mutation (respects --dry-run)
  if [ "$DRY_RUN" = 1 ]; then say "  [dry-run] agent_config $*"; else "$PY" "$AGENT_CONFIG" "$@"; fi
}
has() { # has <file> [name] — read-only check
  "$PY" "$AGENT_CONFIG" has "$1" "${2:-$SERVER_NAME}" 2>/dev/null
}
backup_once() { # back up a config file before the first modification
  local f="$1"
  if [ "$DRY_RUN" = 1 ]; then
    say "  [dry-run] backup $f -> $f.bak-sag"
  elif [ -f "$f" ] && [ ! -f "$f.bak-sag" ]; then
    cp -p "$f" "$f.bak-sag"
  fi
}

# registered agents, as "name<TAB>config-file" rows, verified at the end
REGISTERED_FILE="$(mktemp 2>/dev/null || echo /tmp/sag-install-registered.$$)"
trap 'rm -f "$REGISTERED_FILE"' EXIT
note_registered() { # note_registered <name> <config-file>
  printf '%s\t%s\n' "$1" "$2" >> "$REGISTERED_FILE"
  say "  [ok]        $1 — $2"
}
note_registered_existing() {
  printf '%s\t%s\n' "$1" "$2" >> "$REGISTERED_FILE"
  say "  [ok]        $1 (already registered) — $2"
}
note_skipped() { say "  [skip]      $1"; }

# --------------------------------------------------------------------------- #
hr
say "SpeakEasy — install"
hr

if [ "$(uname -s)" != "Darwin" ]; then
  say "This installer targets macOS (LaunchAgent autostart). The gateway"
  say "binary itself builds and runs on Linux; register the MCP server in"
  say "your agents manually (see README.md)."
  exit 1
fi
# python runs scripts/agent_config.py in every mode, --uninstall included.
if ! command -v "$PY" >/dev/null 2>&1; then
  say "missing required tool: $PY (runs scripts/agent_config.py)" >&2
  exit 1
fi

# --------------------------------------------------------------------------- #
# uninstall
# --------------------------------------------------------------------------- #
if [ "$UNINSTALL" = 1 ]; then
  hr
  say "Uninstalling"
  hr
  run_ok pkill -x "$BIN_NAME"
  run_ok launchctl bootout "gui/$(id -u)/$PLIST_LABEL"
  run rm -f "$PLIST_PATH"
  run_ok claude mcp remove --scope user "$SERVER_NAME"
  run_ok codex mcp remove "$SERVER_NAME"
  run_ok vibe mcp remove "$SERVER_NAME"
  ac remove-json claude "$HOME_DIR/.claude.json"
  ac remove-json pi "$HOME_DIR/.paradev/agent/settings.json"
  ac remove-json opencode "$HOME_DIR/.config/opencode/opencode.json"
  ac remove-json gemini "$HOME_DIR/.gemini/settings.json"
  ac remove-json copilot "$HOME_DIR/.copilot/mcp-config.json"
  ac remove-json amp "$HOME_DIR/.config/amp/settings.json"
  ac remove-json pi "$HOME_DIR/.pi/agent/settings.json"
  ac remove-zed "$HOME_DIR/.config/zed/settings.json"
  ac remove-goose "$HOME_DIR/.config/goose/config.yaml"
  ac remove-hermes "$HOME_DIR/.hermes/config.yaml"
  ac remove-codex "$HOME_DIR/.codex/config.toml"
  ac remove-vibe "$HOME_DIR/.vibe/config.toml"
  for f in "$HOME_DIR/.claude/CLAUDE.md" "$HOME_DIR/.codex/AGENTS.md" \
           "$HOME_DIR/.config/opencode/AGENTS.md" "$HOME_DIR/.vibe/AGENTS.md" \
           "$HOME_DIR/.config/goose/.goosehints" "$HOME_DIR/.pi/agent/AGENTS.md" \
           "$HOME_DIR/.paradev/agent/AGENTS.md" "$HOME_DIR/.gemini/GEMINI.md" \
           "$HOME_DIR/.copilot/copilot-instructions.md" "$HOME_DIR/.hermes/SOUL.md" \
           "$HOME_DIR/.hermes/AGENTS.md" "$HOME_DIR/.config/zed/AGENTS.md"; do
    ac instruction-remove "$f"
  done
  run rm -f "$HOME_DIR/.config/zed/rules/slack-gateway.md"
  run rm -f "$INSTALL_BIN"
  if [ "$PURGE" = 1 ]; then
    run rm -rf "$STATE_DIR"
  else
    say "Kept state dir $STATE_DIR (cooldowns, sent records, token file)."
    say "Delete it manually with: rm -rf $STATE_DIR"
  fi
  say "Uninstalled. SpeakEasy's own entries and instruction blocks were"
  say "removed from your agent configs; your other changes to those files"
  say "are untouched. The pre-install .bak-sag backups remain next to each"
  say "config file for a manual full restore."
  exit 0
fi

# --------------------------------------------------------------------------- #
# 1. build
# --------------------------------------------------------------------------- #
hr
say "1. Build"
hr
# cargo and npm are only required when a build actually runs: not for
# --uninstall (which exits above), not for --skip-build, not for --dry-run.
if [ "$SKIP_BUILD" = 0 ] && [ "$DRY_RUN" = 0 ]; then
  for tool in cargo npm; do
    if ! command -v "$tool" >/dev/null 2>&1; then
      say "missing required tool: $tool (needed to build; or pass --skip-build" >&2
      say "to reuse the existing release binary)" >&2
      exit 1
    fi
  done
fi
if [ "$SKIP_BUILD" = 1 ]; then
  say "  skipping build (--skip-build)"
else
  if [ "$DRY_RUN" = 0 ]; then
    if [ ! -d "$ROOT/ui/node_modules" ]; then
      say "  npm install (ui/)"
      (cd "$ROOT/ui" && npm install --no-audit --no-fund)
    fi
    say "  npm run build (ui/)"
    (cd "$ROOT/ui" && npm run build)
    say "  cargo build --release"
    (cd "$ROOT" && cargo build --release)
  else
    say "  [dry-run] cd ui && npm install && npm run build"
    say "  [dry-run] cargo build --release"
  fi
fi
BINARY="$ROOT/target/release/$BIN_NAME"
if [ ! -x "$BINARY" ]; then
  say "release binary missing: $BINARY (run without --skip-build)" >&2
  exit 1
fi

# --------------------------------------------------------------------------- #
# 2. install binary
# --------------------------------------------------------------------------- #
hr
say "2. Install binary"
hr
run mkdir -p "$HOME_DIR/.local/bin"
run install -m 0755 "$BINARY" "$INSTALL_BIN"
say "  installed: $INSTALL_BIN"

# --------------------------------------------------------------------------- #
# 3. state dir, transport, and credentials
# --------------------------------------------------------------------------- #
hr
say "3. State dir, transport, and credentials"
hr
run mkdir -p "$STATE_DIR"
if [ "$DRY_RUN" = 0 ]; then chmod 700 "$STATE_DIR"; fi

# --- transport selection (--transport slack-mcp|mistral-connector|korotovsky)
TRANSPORT="$TRANSPORT_ARG"
if [ -z "$TRANSPORT" ]; then TRANSPORT="slack-mcp"; fi
case "$TRANSPORT" in
  slack-mcp|mistral-connector|korotovsky) ;;
  *) say "unknown --transport value: $TRANSPORT (expected slack-mcp, mistral-connector or korotovsky)" >&2; exit 2 ;;
esac
TRANSPORT_FILE="$STATE_DIR/slack_transport"
if [ -n "$TRANSPORT_ARG" ]; then
  TRANSPORT="$TRANSPORT_ARG"
  if [ "$DRY_RUN" = 0 ]; then
    printf '%s\n' "$TRANSPORT" > "$TRANSPORT_FILE"
  else
    say "  [dry-run] write $TRANSPORT_FILE ($TRANSPORT)"
  fi
  say "  transport: $TRANSPORT (written to $TRANSPORT_FILE)"
elif [ -f "$TRANSPORT_FILE" ]; then
  # The effective transport is what the file selects; the credential blocks
  # and the summary below key off $TRANSPORT, so resolve it here.
  EXISTING_TRANSPORT="$(cat "$TRANSPORT_FILE")"
  case "$EXISTING_TRANSPORT" in
    slack-mcp|mistral-connector|korotovsky)
      TRANSPORT="$EXISTING_TRANSPORT"
      say "  keeping existing transport selection in $TRANSPORT_FILE ($EXISTING_TRANSPORT)"
      ;;
    *)
      # The old bot-token transport ("direct") was removed: the gateway no
      # longer holds a Slack bot token.
      TRANSPORT="slack-mcp"
      say "  $TRANSPORT_FILE selects '$EXISTING_TRANSPORT', a removed transport"
      say "  (the gateway no longer holds a Slack bot token); switching to slack-mcp"
      if [ "$DRY_RUN" = 0 ]; then
        printf '%s\n' "slack-mcp" > "$TRANSPORT_FILE"
      else
        say "  [dry-run] write $TRANSPORT_FILE (slack-mcp)"
      fi
      ;;
  esac
else
  say "  transport: slack-mcp (default; SLACK_TRANSPORT or $TRANSPORT_FILE can change it)"
fi

# --- workspace domain (message permalinks; the korotovsky server's replies
# carry no permalink, so the gateway builds links from this domain)
WORKSPACE_DOMAIN_FILE="$STATE_DIR/slack_workspace_domain"
if [ -n "$WORKSPACE_DOMAIN_ARG" ]; then
  if [ "$DRY_RUN" = 0 ]; then
    printf '%s\n' "$WORKSPACE_DOMAIN_ARG" > "$WORKSPACE_DOMAIN_FILE"
  else
    say "  [dry-run] write $WORKSPACE_DOMAIN_FILE ($WORKSPACE_DOMAIN_ARG)"
  fi
  say "  wrote $WORKSPACE_DOMAIN_FILE — message permalinks point at $WORKSPACE_DOMAIN_ARG.slack.com"
  say "  (SLACK_WORKSPACE_DOMAIN overrides the file when the gateway runs)"
fi

# --- Mistral API key (the mistral-connector transport)
# Precedence: the --mistral-api-key flag, then an existing key file, then
# $MISTRAL_API_KEY. An ambient env var must never clobber a working file.
MISTRAL_KEY_FILE="$STATE_DIR/mistral_api_key"
MISTRAL_CREDENTIAL_WRITTEN=0
if [ "$TRANSPORT" = "mistral-connector" ] || [ -n "$MISTRAL_KEY_ARG" ]; then
  if [ -n "$MISTRAL_KEY_ARG" ]; then
    MISTRAL_KEY="$MISTRAL_KEY_ARG"
  elif [ ! -f "$MISTRAL_KEY_FILE" ]; then
    MISTRAL_KEY="${MISTRAL_API_KEY:-}"
  else
    MISTRAL_KEY=""
  fi
  if [ -n "$MISTRAL_KEY" ]; then
    MISTRAL_CREDENTIAL_WRITTEN=1
    if [ "$DRY_RUN" = 0 ]; then
      umask 077
      printf '%s\n' "$MISTRAL_KEY" > "$MISTRAL_KEY_FILE"
      chmod 600 "$MISTRAL_KEY_FILE"
    else
      say "  [dry-run] write $MISTRAL_KEY_FILE (chmod 600)"
    fi
    say "  wrote $MISTRAL_KEY_FILE (chmod 600) — the only place the Mistral API key lives"
    say "  (Slack credentials stay in the Mistral console; connect the Slack"
    say "   workspace to the Slack connector there before sending)"
  elif [ -f "$MISTRAL_KEY_FILE" ]; then
    say "  keeping existing $MISTRAL_KEY_FILE (pass --mistral-api-key to replace it)"
  else
    if [ "$DRY_RUN" = 0 ]; then
      umask 077
      printf '%s\n' "mistral-placeholder-set-a-real-key" > "$MISTRAL_KEY_FILE"
      chmod 600 "$MISTRAL_KEY_FILE"
    else
      say "  [dry-run] write placeholder $MISTRAL_KEY_FILE"
    fi
    say "  WARNING: no Mistral API key given (--mistral-api-key KEY or MISTRAL_API_KEY)."
    say "  Wrote a placeholder to $MISTRAL_KEY_FILE — the gateway starts and"
    say "  approval popups work, but connector sends fail until you install a"
    say "  real key:"
    say "    scripts/install.sh --transport mistral-connector --mistral-api-key YOUR_KEY"
    say "  (re-running the installer restarts the service, which reads the file"
    say "   at startup)"
  fi
fi

# --- Slack MCP user token (the slack-mcp transport)
# Precedence: the --slack-mcp-token flag, then an existing token file, then
# $SLACK_MCP_TOKEN. An ambient env var must never clobber a working file.
MCP_TOKEN_FILE="$STATE_DIR/slack_mcp_token"
MCP_CREDENTIAL_WRITTEN=0
if [ "$TRANSPORT" = "slack-mcp" ]; then
  if [ -n "$MCP_TOKEN_ARG" ]; then
    MCP_TOKEN="$MCP_TOKEN_ARG"
  elif [ ! -f "$MCP_TOKEN_FILE" ]; then
    MCP_TOKEN="${SLACK_MCP_TOKEN:-}"
  else
    MCP_TOKEN=""
  fi
  if [ -n "$MCP_TOKEN" ]; then
    MCP_CREDENTIAL_WRITTEN=1
    if [ "$DRY_RUN" = 0 ]; then
      umask 077
      printf '%s\n' "$MCP_TOKEN" > "$MCP_TOKEN_FILE"
      chmod 600 "$MCP_TOKEN_FILE"
    else
      say "  [dry-run] write $MCP_TOKEN_FILE (chmod 600)"
    fi
    say "  wrote $MCP_TOKEN_FILE (chmod 600) — the only place the Slack user token lives"
    say "  (the gateway presents it to the Slack MCP server; it is a user token,"
    say "   not a bot token, and agents never see it)"
  elif [ -f "$MCP_TOKEN_FILE" ]; then
    say "  keeping existing $MCP_TOKEN_FILE (pass --slack-mcp-token to replace it)"
  else
    if [ "$DRY_RUN" = 0 ]; then
      umask 077
      printf '%s\n' "xoxp-placeholder-get-a-real-token" > "$MCP_TOKEN_FILE"
      chmod 600 "$MCP_TOKEN_FILE"
    else
      say "  [dry-run] write placeholder $MCP_TOKEN_FILE"
    fi
    say "  WARNING: no Slack MCP token given (--slack-mcp-token xoxp-... or SLACK_MCP_TOKEN)."
    say "  Wrote a placeholder to $MCP_TOKEN_FILE — the gateway cannot start with"
    say "  it: it runs the Slack MCP handshake at startup and exits on failure, so"
    say "  the service keeps restarting (launchd retries every 10 s) and agents"
    say "  report that the MCP server failed to start. Neither popups nor sends"
    say "  work until you install a real Slack user token:"
    say "    scripts/slack_mcp_oauth.py --client-id YOUR_SLACK_APP_CLIENT_ID \\"
    say "        --client-secret YOUR_SLACK_APP_CLIENT_SECRET"
    say "  (opens the Slack authorize page for the official Slack MCP server and"
    say "   writes the token to $MCP_TOKEN_FILE; needs a Slack app with the"
    say "   redirect URL http://127.0.0.1:3118/callback — see README.md)"
    say "  Then restart the service so it picks the token up:"
    say "    launchctl kickstart -k gui/$(id -u)/com.speakeasy.slack-approval-gateway"
    say "  (re-running this installer restarts it too)"
  fi
else
  say "  $TRANSPORT transport selected: no Slack MCP token needed"
fi

# --- Slack bot/user token (the korotovsky transport)
# Precedence: the --slack-bot-token/--slack-user-token flags, then existing
# token files, then $SLACK_BOT_TOKEN/$SLACK_USER_TOKEN. An ambient env var
# must never clobber a working file. Either token is enough; the gateway
# passes it to the server it spawns (agents never see it).
BOT_TOKEN_FILE="$STATE_DIR/slack_bot_token"
USER_TOKEN_FILE="$STATE_DIR/slack_user_token"
if [ "$TRANSPORT" = "korotovsky" ]; then
  if [ -n "$BOT_TOKEN_ARG" ]; then
    BOT_TOKEN="$BOT_TOKEN_ARG"
  elif [ ! -f "$BOT_TOKEN_FILE" ]; then
    BOT_TOKEN="${SLACK_BOT_TOKEN:-}"
  else
    BOT_TOKEN=""
  fi
  if [ -n "$USER_TOKEN_ARG" ]; then
    USER_TOKEN="$USER_TOKEN_ARG"
  elif [ ! -f "$USER_TOKEN_FILE" ]; then
    USER_TOKEN="${SLACK_USER_TOKEN:-}"
  else
    USER_TOKEN=""
  fi
  if [ -n "$BOT_TOKEN" ]; then
    if [ "$DRY_RUN" = 0 ]; then
      umask 077
      printf '%s\n' "$BOT_TOKEN" > "$BOT_TOKEN_FILE"
      chmod 600 "$BOT_TOKEN_FILE"
    else
      say "  [dry-run] write $BOT_TOKEN_FILE (chmod 600)"
    fi
    say "  wrote $BOT_TOKEN_FILE (chmod 600) — the only place the Slack bot token lives"
  elif [ -f "$BOT_TOKEN_FILE" ]; then
    say "  keeping existing $BOT_TOKEN_FILE (pass --slack-bot-token to replace it)"
  fi
  if [ -n "$USER_TOKEN" ]; then
    if [ "$DRY_RUN" = 0 ]; then
      umask 077
      printf '%s\n' "$USER_TOKEN" > "$USER_TOKEN_FILE"
      chmod 600 "$USER_TOKEN_FILE"
    else
      say "  [dry-run] write $USER_TOKEN_FILE (chmod 600)"
    fi
    say "  wrote $USER_TOKEN_FILE (chmod 600) — the only place the Slack user token lives"
  elif [ -f "$USER_TOKEN_FILE" ]; then
    say "  keeping existing $USER_TOKEN_FILE (pass --slack-user-token to replace it)"
  fi
  if [ -z "$BOT_TOKEN" ] && [ -z "$USER_TOKEN" ] \
     && [ ! -f "$BOT_TOKEN_FILE" ] && [ ! -f "$USER_TOKEN_FILE" ]; then
    if [ "$DRY_RUN" = 0 ]; then
      umask 077
      printf '%s\n' "xoxb-placeholder-get-a-real-token" > "$BOT_TOKEN_FILE"
      chmod 600 "$BOT_TOKEN_FILE"
    else
      say "  [dry-run] write placeholder $BOT_TOKEN_FILE"
    fi
    say "  WARNING: no Slack token given (--slack-bot-token xoxb-... or --slack-user-token"
    say "  xoxp-..., or SLACK_BOT_TOKEN/SLACK_USER_TOKEN). Wrote a placeholder to"
    say "  $BOT_TOKEN_FILE — approval popups work, but sends fail until you install"
    say "  a real token. For the korotovsky transport you need either:"
    say "    - a Slack app bot token (xoxb-...): create an app at"
    say "      https://api.slack.com/apps, add the bot scopes chat:write,"
    say "      channels:read, users:read (plus groups:read for private"
    say "      channels), install the app to the workspace, and invite the bot"
    say "      to the channels it may post to (bot tokens see invited channels"
    say "      only); or"
    say "    - a Slack user token (xoxp-...) with the same scopes as a user."
    say "  The gateway spawns the slack-mcp-server you installed"
    say "  (https://github.com/korotovsky/slack-mcp-server) as a stdio child"
    say "  process and passes it the token — see README.md."
    say "  (the service restarts automatically; it reads the file at startup)"
  fi

  # --- the server command (SpeakEasy never installs the server)
  # Unless the command file already exists, detect `slack-mcp-server` on
  # this PATH and record its absolute path — with --transport stdio, the
  # server's documented stdio invocation — so the login service can spawn
  # it. When it is missing, the gateway fails at startup with instructions;
  # install it yourself, never via this script.
  STDIO_COMMAND_FILE="$STATE_DIR/slack_stdio_command"
  if [ -f "$STDIO_COMMAND_FILE" ]; then
    say "  keeping existing $STDIO_COMMAND_FILE ($(cat "$STDIO_COMMAND_FILE"))"
  elif SERVER_BIN="$(command -v slack-mcp-server 2>/dev/null)"; then
    if [ "$DRY_RUN" = 0 ]; then
      printf '%s\n' "$SERVER_BIN --transport stdio" > "$STDIO_COMMAND_FILE"
    else
      say "  [dry-run] write $STDIO_COMMAND_FILE ($SERVER_BIN --transport stdio)"
    fi
    say "  korotovsky server: $SERVER_BIN (recorded in $STDIO_COMMAND_FILE)"
  else
    say "  WARNING: slack-mcp-server is not on PATH. SpeakEasy does not install"
    say "  or download it — install it yourself:"
    say "    https://github.com/korotovsky/slack-mcp-server"
    say "  (e.g. npm install -g slack-mcp-server), then either keep it on PATH,"
    say "  write its command to $STDIO_COMMAND_FILE, or set SLACK_STDIO_COMMAND:"
    say "    printf '%s\\n' \"slack-mcp-server --transport stdio\" > $STDIO_COMMAND_FILE"
    say "  The gateway refuses to start the korotovsky transport until the"
    say "  server is installed and reachable."
  fi
fi

# --------------------------------------------------------------------------- #
# 4. LaunchAgent (single always-on instance)
# --------------------------------------------------------------------------- #
hr
say "4. Login service (single gateway instance)"
hr
if [ "$DRY_RUN" = 0 ]; then
  mkdir -p "$HOME_DIR/Library/LaunchAgents"
  umask 022
  # launchd starts the service with PATH=/usr/bin:/bin:/usr/sbin:/sbin,
  # which can find neither the korotovsky server nor the node its npm
  # wrapper needs — pass the installer's own PATH through (XML-escaped).
  SERVICE_PATH_XML="$(printf '%s' "$PATH" | sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g')"
  cat > "$PLIST_PATH" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$PLIST_LABEL</string>
  <key>ProgramArguments</key>
  <array><string>$INSTALL_BIN</string><string>--serve</string></array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
  <key>ThrottleInterval</key><integer>10</integer>
  <key>EnvironmentVariables</key>
  <dict><key>PATH</key><string>$SERVICE_PATH_XML</string></dict>
  <key>StandardOutPath</key><string>$STATE_DIR/gateway.log</string>
  <key>StandardErrorPath</key><string>$STATE_DIR/gateway.log</string>
</dict>
</plist>
EOF
else
  say "  [dry-run] write $PLIST_PATH (ProgramArguments: $INSTALL_BIN --serve)"
fi
say "  restarting any running gateway so the new binary takes over"
run_ok pkill -x "$BIN_NAME"
sleep 1
run_ok launchctl bootout "gui/$(id -u)/$PLIST_LABEL"
run launchctl bootstrap "gui/$(id -u)" "$PLIST_PATH" || run_ok true
run_ok launchctl kickstart -k "gui/$(id -u)/$PLIST_LABEL"
if [ "$DRY_RUN" = 0 ]; then
  for _ in $(seq 1 50); do
    if [ -S "$STATE_DIR/gateway.sock" ]; then break; fi
    sleep 0.2
  done
fi
if [ -S "$STATE_DIR/gateway.sock" ]; then
  PID="$(launchctl print "gui/$(id -u)/$PLIST_LABEL" 2>/dev/null | sed -n 's/.*pid = \([0-9]*\).*/\1/p' | head -1)"
  say "  service running (pid ${PID:-?}); MCP socket: $STATE_DIR/gateway.sock"
else
  say "  WARNING: service socket not up yet; check:"
  say "    launchctl print gui/$(id -u)/$PLIST_LABEL   and   $STATE_DIR/gateway.log"
fi

# --------------------------------------------------------------------------- #
# 5. register the MCP server in every detected agent
# --------------------------------------------------------------------------- #
hr
say "5. Register the MCP server in coding agents"
hr

# --- Claude Code (user scope so every project gets it)
if command -v claude >/dev/null 2>&1; then
  CFG="$HOME_DIR/.claude.json"
  if has "$CFG"; then
    note_registered_existing claude "$CFG"
  else
    backup_once "$CFG"
    if [ "$DRY_RUN" = 0 ]; then
      if ! claude mcp add --scope user --transport stdio "$SERVER_NAME" -- "$INSTALL_BIN" >/dev/null 2>&1; then
        say "  (claude mcp add failed; editing .claude.json directly)"
        "$PY" "$AGENT_CONFIG" add-json claude "$CFG" "$INSTALL_BIN"
      fi
    else
      say "  [dry-run] claude mcp add --scope user --transport stdio $SERVER_NAME -- $INSTALL_BIN"
    fi
    note_registered claude "$CFG"
  fi
else
  note_skipped "claude (not installed)"
fi

# --- pdev / paradev (pi-style settings)
if command -v pdev >/dev/null 2>&1 || [ -d "$HOME_DIR/.paradev" ]; then
  CFG="$HOME_DIR/.paradev/agent/settings.json"
  if has "$CFG"; then
    note_registered_existing pdev "$CFG"
  else
    ac add-json pi "$CFG" "$INSTALL_BIN"
    note_registered pdev "$CFG"
  fi
else
  note_skipped "pdev (not installed)"
fi

# --- Codex
if command -v codex >/dev/null 2>&1; then
  CFG="$HOME_DIR/.codex/config.toml"
  if has "$CFG"; then
    note_registered_existing codex "$CFG"
  else
    backup_once "$CFG"
    if [ "$DRY_RUN" = 0 ]; then
      if ! codex mcp add "$SERVER_NAME" -- "$INSTALL_BIN" >/dev/null 2>&1; then
        say "  (codex mcp add failed; appending to config.toml)"
        "$PY" "$AGENT_CONFIG" add-codex "$CFG" "$INSTALL_BIN"
      fi
    else
      say "  [dry-run] codex mcp add $SERVER_NAME -- $INSTALL_BIN"
    fi
    note_registered codex "$CFG"
  fi
  # `codex mcp add` has no timeout flag; a request_slack_message call
  # blocks on a human approval for minutes (codex's default tool timeout
  # is 60s), so give the entry a long tool_timeout_sec
  run_ok "$PY" "$AGENT_CONFIG" set-timeout "$CFG" "$SERVER_NAME" 86400
else
  note_skipped "codex (not installed)"
fi

# --- OpenCode
if command -v opencode >/dev/null 2>&1 || [ -f "$HOME_DIR/.config/opencode/opencode.json" ]; then
  CFG="$HOME_DIR/.config/opencode/opencode.json"
  if has "$CFG"; then
    note_registered_existing opencode "$CFG"
  else
    ac add-json opencode "$CFG" "$INSTALL_BIN"
    note_registered opencode "$CFG"
  fi
else
  note_skipped "opencode (not installed)"
fi

# --- Mistral Vibe
if command -v vibe >/dev/null 2>&1; then
  CFG="$HOME_DIR/.vibe/config.toml"
  if has "$CFG"; then
    note_registered_existing vibe "$CFG"
  else
    backup_once "$CFG"
    if [ "$DRY_RUN" = 0 ]; then
      # --tool-timeout-sec: a request_slack_message call blocks on a human
      # approval for minutes (vibe's default tool timeout is 60s). An older
      # vibe without the flag still registers (second attempt); the
      # set-timeout below then fixes the entry either way.
      if ! vibe mcp add "$SERVER_NAME" --transport stdio --command "$INSTALL_BIN" --tool-timeout-sec 86400 >/dev/null 2>&1 \
         && ! vibe mcp add "$SERVER_NAME" --transport stdio --command "$INSTALL_BIN" >/dev/null 2>&1; then
        say "  (vibe mcp add failed; appending to config.toml)"
        "$PY" "$AGENT_CONFIG" add-vibe "$CFG" "$INSTALL_BIN"
      fi
    else
      say "  [dry-run] vibe mcp add $SERVER_NAME --transport stdio --command $INSTALL_BIN --tool-timeout-sec 86400"
    fi
    note_registered vibe "$CFG"
  fi
  # make sure the entry has a long tool_timeout_sec whether the CLI wrote
  # it or an older vibe without --tool-timeout-sec did
  run_ok "$PY" "$AGENT_CONFIG" set-timeout "$CFG" "$SERVER_NAME" 86400
else
  note_skipped "vibe (not installed)"
fi

# --- goose
if command -v goose >/dev/null 2>&1 || [ -f "$HOME_DIR/.config/goose/config.yaml" ]; then
  CFG="$HOME_DIR/.config/goose/config.yaml"
  if has "$CFG"; then
    note_registered_existing goose "$CFG"
  else
    ac add-goose "$CFG" "$INSTALL_BIN"
    note_registered goose "$CFG"
  fi
  # a request_slack_message call blocks on a human approval for minutes
  # (goose cancels the call when its timeout expires; older installs wrote
  # timeout: 120), so set a long timeout on every run
  run_ok "$PY" "$AGENT_CONFIG" set-timeout "$CFG" "$SERVER_NAME" 86400
else
  note_skipped "goose (not installed)"
fi

# --- pi
if command -v pi >/dev/null 2>&1 || [ -f "$HOME_DIR/.pi/agent/settings.json" ]; then
  CFG="$HOME_DIR/.pi/agent/settings.json"
  if has "$CFG"; then
    note_registered_existing pi "$CFG"
  else
    ac add-json pi "$CFG" "$INSTALL_BIN"
    note_registered pi "$CFG"
  fi
else
  note_skipped "pi (not installed)"
fi

# --- Gemini CLI (config is honored even when the CLI is not on PATH yet)
if [ -d "$HOME_DIR/.gemini" ]; then
  CFG="$HOME_DIR/.gemini/settings.json"
  if has "$CFG"; then
    note_registered_existing gemini "$CFG"
  else
    ac add-json gemini "$CFG" "$INSTALL_BIN"
    if command -v gemini >/dev/null 2>&1 && gemini --version >/dev/null 2>&1; then
      note_registered gemini "$CFG"
    else
      say "  [ok]        gemini (config written; CLI not currently on PATH) — $CFG"
      printf '%s\t%s\n' gemini "$CFG" >> "$REGISTERED_FILE"
    fi
  fi
else
  note_skipped "gemini (not installed)"
fi

# --- GitHub Copilot CLI
if [ -d "$HOME_DIR/.copilot" ]; then
  CFG="$HOME_DIR/.copilot/mcp-config.json"
  if has "$CFG"; then
    note_registered_existing copilot "$CFG"
  else
    ac add-json copilot "$CFG" "$INSTALL_BIN"
    note_registered copilot "$CFG"
  fi
else
  note_skipped "copilot (not installed)"
fi

# --- Zed
if command -v zed >/dev/null 2>&1 || [ -d "$HOME_DIR/.config/zed" ]; then
  CFG="$HOME_DIR/.config/zed/settings.json"
  if has "$CFG"; then
    note_registered_existing zed "$CFG"
  else
    ac add-zed "$CFG" "$INSTALL_BIN"
    note_registered zed "$CFG"
  fi
else
  note_skipped "zed (not installed)"
fi

# --- Amp
if command -v amp >/dev/null 2>&1 || [ -f "$HOME_DIR/.config/amp/settings.json" ]; then
  CFG="$HOME_DIR/.config/amp/settings.json"
  if has "$CFG"; then
    note_registered_existing amp "$CFG"
  else
    ac add-json amp "$CFG" "$INSTALL_BIN"
    note_registered amp "$CFG"
  fi
else
  note_skipped "amp (not installed)"
fi

# --- Hermes Agent
if command -v hermes >/dev/null 2>&1 || [ -d "$HOME_DIR/.hermes" ]; then
  CFG="$HOME_DIR/.hermes/config.yaml"
  if has "$CFG"; then
    note_registered_existing hermes "$CFG"
  else
    ac add-hermes "$CFG" "$INSTALL_BIN"
    note_registered hermes "$CFG"
  fi
else
  note_skipped "hermes (not installed)"
fi

# --- Devin CLI (no known user-level MCP config location; noted only)
if command -v devin >/dev/null 2>&1 && devin --version >/dev/null 2>&1; then
  note_skipped "devin (detected; no known MCP config — register manually, see README)"
else
  note_skipped "devin (not installed)"
fi

say ""
say "  Superset workspaces run these same agent CLIs as this user, so they"
say "  pick up the registrations above automatically — no per-workspace setup."

# --------------------------------------------------------------------------- #
# 6. instruction blocks ("send only through the gateway")
# --------------------------------------------------------------------------- #
hr
say "6. Agent instruction blocks"
hr
add_instruction() {
  ac instruction-add "$1"
}
if [ -f "$HOME_DIR/.claude.json" ] || [ -d "$HOME_DIR/.claude" ]; then
  add_instruction "$HOME_DIR/.claude/CLAUDE.md"
fi
if [ -f "$HOME_DIR/.codex/config.toml" ]; then
  add_instruction "$HOME_DIR/.codex/AGENTS.md"
fi
if [ -f "$HOME_DIR/.config/opencode/opencode.json" ]; then
  add_instruction "$HOME_DIR/.config/opencode/AGENTS.md"
fi
if [ -f "$HOME_DIR/.vibe/config.toml" ]; then
  add_instruction "$HOME_DIR/.vibe/AGENTS.md"
fi
if [ -f "$HOME_DIR/.config/goose/config.yaml" ]; then
  add_instruction "$HOME_DIR/.config/goose/.goosehints"
fi
if [ -f "$HOME_DIR/.pi/agent/settings.json" ]; then
  add_instruction "$HOME_DIR/.pi/agent/AGENTS.md"
fi
if [ -f "$HOME_DIR/.paradev/agent/settings.json" ]; then
  add_instruction "$HOME_DIR/.paradev/agent/AGENTS.md"
fi
if [ -f "$HOME_DIR/.gemini/settings.json" ]; then
  add_instruction "$HOME_DIR/.gemini/GEMINI.md"
fi
if [ -f "$HOME_DIR/.copilot/mcp-config.json" ]; then
  add_instruction "$HOME_DIR/.copilot/copilot-instructions.md"
fi
if [ -f "$HOME_DIR/.hermes/config.yaml" ]; then
  # Hermes loads global instructions from SOUL.md (HERMES_HOME), never from
  # ~/.hermes/AGENTS.md (that file is only read from a working directory)
  add_instruction "$HOME_DIR/.hermes/SOUL.md"
  if has "$HOME_DIR/.hermes/AGENTS.md"; then
    say "  migrating: removing the old block from $HOME_DIR/.hermes/AGENTS.md"
    ac instruction-remove "$HOME_DIR/.hermes/AGENTS.md"
  fi
fi
if [ -f "$HOME_DIR/.config/zed/settings.json" ]; then
  # Zed reads user-level instructions from ~/.config/zed/AGENTS.md; the old
  # rules/slack-gateway.md location is never loaded, so drop it
  add_instruction "$HOME_DIR/.config/zed/AGENTS.md"
  run rm -f "$HOME_DIR/.config/zed/rules/slack-gateway.md"
fi

# --------------------------------------------------------------------------- #
# 7. exclusivity audit (+ optional enforcement)
# --------------------------------------------------------------------------- #
hr
say "7. Enforcement audit (exclusive sending path)"
hr
AUDIT_FILES=(
  "$HOME_DIR/.claude.json"
  "$HOME_DIR/.codex/config.toml"
  "$HOME_DIR/.config/opencode/opencode.json"
  "$HOME_DIR/.vibe/config.toml"
  "$HOME_DIR/.config/goose/config.yaml"
  "$HOME_DIR/.pi/agent/settings.json"
  "$HOME_DIR/.paradev/agent/settings.json"
  "$HOME_DIR/.gemini/settings.json"
  "$HOME_DIR/.copilot/mcp-config.json"
  "$HOME_DIR/.config/zed/settings.json"
  "$HOME_DIR/.config/amp/settings.json"
  "$HOME_DIR/.hermes/config.yaml"
)
AUDIT_JSON="$("$PY" "$AGENT_CONFIG" audit "${AUDIT_FILES[@]}" 2>/dev/null || echo '[]')"
# The audit categorizes what agents have installed. The official Slack MCP
# server and Mistral connector servers are expected (agents read Slack
# context through them); direct token-based Slack servers are violations.
audit_pairs() { # audit_pairs <finding-field> — "[file, name, key]" lines for
  # server entries (key = the JSON path of the servers container, [] for
  # TOML), "[file, value]" lines for token/key findings
  "$PY" - "$AUDIT_JSON" "$1" <<'EOF'
import json, sys
for f in json.loads(sys.argv[1]):
    for e in f.get(sys.argv[2], []):
        if isinstance(e, dict):
            print(json.dumps([f["file"], e["name"], e.get("key", [])]))
        else:
            print(json.dumps([f["file"], e]))
EOF
}
report_pairs() { # report_pairs <pairs> — one "    - file (server: name)" per line
  while IFS= read -r line; do
    [ -n "$line" ] || continue
    "$PY" -c 'import json,sys; p,n,k=json.loads(sys.argv[1]); print(f"    - {p}  (server: {n})")' "$line"
  done <<< "$1"
}
EXPECTED_SLACK_MCP="$(audit_pairs slack_mcp_servers)"
EXPECTED_MISTRAL="$(audit_pairs mistral_connector_servers)"
GAPS="$(audit_pairs direct_slack_servers)"
TOKENS="$(audit_pairs slack_tokens)"
MISTRAL_KEYS="$(audit_pairs mistral_keys)"

if [ -n "$EXPECTED_SLACK_MCP" ]; then
  say "  Official Slack MCP servers in agent configs (expected: agents read"
  say "  Slack context through them; the instruction block routes all sending"
  say "  through the gateway):"
  report_pairs "$EXPECTED_SLACK_MCP"
fi
if [ -n "$EXPECTED_MISTRAL" ]; then
  say "  Mistral connector servers in agent configs (expected: same — they"
  say "  expose the Slack connector for reading):"
  report_pairs "$EXPECTED_MISTRAL"
fi
if [ -n "$EXPECTED_SLACK_MCP" ] || [ -n "$EXPECTED_MISTRAL" ]; then
  say "  Both also expose send tools (slack_send_message), so the boundary for"
  say "  them is the instruction block + the hook guard; for the Slack MCP"
  say "  server, keep the agents' grant read-only (no chat:write scope) so"
  say "  their token cannot send — the gateway's token is the only one with"
  say "  chat:write. --enforce keeps these (agents are expected to have them)."
fi
if [ -n "$GAPS" ]; then
  say "  Direct Slack MCP servers in agent configs (agents could send"
  say "  without the gateway):"
  report_pairs "$GAPS"
else
  say "  No direct (token-based) Slack MCP servers in agent configs."
fi
if [ -n "$TOKENS" ]; then
  say "  Slack tokens found in agent configs (masked):"
  while IFS= read -r line; do
    [ -n "$line" ] || continue
    "$PY" -c 'import json,sys; p,t=json.loads(sys.argv[1]); print(f"    - {p}  ({t})")' "$line"
  done <<< "$TOKENS"
  say "  (these belong to direct Slack MCP servers; --enforce removes the"
  say "   server entries but cannot scrub every copy — rotate the tokens in"
  say "   Slack if you want them gone for good)"
fi
if [ -n "$MISTRAL_KEYS" ]; then
  say "  Mistral API keys found in agent configs (masked) — an agent with"
  say "  one can call the Slack connector's send tool directly:"
  while IFS= read -r line; do
    [ -n "$line" ] || continue
    "$PY" -c 'import json,sys; p,t=json.loads(sys.argv[1]); print(f"    - {p}  ({t})")' "$line"
  done <<< "$MISTRAL_KEYS"
  say "  (remove them from agent configs, or rotate the keys, if you want"
  say "   agents unable to reach the connector at all)"
fi

if [ -f "$HOME_DIR/.config/agent-guards/install.py" ]; then
  if "$PY" "$HOME_DIR/.config/agent-guards/install.py" doctor >/dev/null 2>&1; then
    say "  Legacy hook guard (~/.config/agent-guards): installed and healthy."
    say "  It stays as the backstop that blocks direct Slack writes (curl,"
    say "  browser, SDKs) unless you approve them in its dialog."
  else
    say "  Legacy hook guard found but 'install.py doctor' failed — run:"
    say "    python3 ~/.config/agent-guards/install.py doctor"
  fi
else
  say "  No legacy hook guard found. Direct Slack writes by agents (curl,"
  say "  browser automation) are then only covered by agent instructions."
fi

if [ "$ENFORCE" = 1 ]; then
  if [ -z "$GAPS" ]; then
    say "  --enforce: nothing to remove (no direct Slack MCP servers found)."
    say "  (the official Slack MCP / Mistral connector servers are kept —"
    say "   agents are expected to have them for reading Slack context)"
  else
    say ""
    say "  --enforce: removing direct Slack MCP servers from agent configs"
    while IFS= read -r line; do
      [ -n "$line" ] || continue
      if [ "$DRY_RUN" = 1 ]; then
        "$PY" -c 'import json,sys; p,n,k=json.loads(sys.argv[1]); print(f"  [dry-run] would remove server {n} from {p}")' "$line"
        continue
      fi
      # enforce-remove reports what actually happened: [removed], [absent],
      # [failed], or [edit by hand] (configs it cannot edit, e.g. YAML)
      ac enforce-remove "$line"
    done <<< "$GAPS"
    if [ "$DRY_RUN" = 0 ]; then
      say "  Backups of the originals: <file>.bak-sag (kept — restore one by"
      say "  hand if you ever want the pre-install file back)"
    fi
    say "  Kept (expected in agents): the official Slack MCP / Mistral"
    say "  connector servers — see the report above."
  fi
fi

if [ "$DRY_RUN" = 0 ]; then
  DIR_MODE="$(stat -f '%Lp' "$STATE_DIR" 2>/dev/null || echo '?')"
  TOK_MODE="$(stat -f '%Lp' "$MCP_TOKEN_FILE" 2>/dev/null || echo '?')"
  say "  state dir $STATE_DIR (mode $DIR_MODE), Slack MCP token file mode $TOK_MODE (want 600)"
fi

# --------------------------------------------------------------------------- #
# 8. self-test: MCP handshake through a real spawned bridge
# --------------------------------------------------------------------------- #
hr
say "8. Self-test (MCP handshake through a spawned process)"
hr
if [ "$DRY_RUN" = 1 ]; then
  say "  [dry-run] skipped"
elif [ -S "$STATE_DIR/gateway.sock" ]; then
  # perl ships with macOS; GNU timeout does not. `alarm shift; exec @ARGV`
  # runs the command under a 15 s alarm.
  SELF_TEST="$(printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"install-self-test","version":"0"}}}' \
    | perl -e 'alarm shift; exec @ARGV' 15 "$INSTALL_BIN" 2>/dev/null | head -1 || true)"
  if printf '%s' "$SELF_TEST" | grep -q '"slack-approval-gateway"'; then
    say "  PASS: a spawned process bridged to the running instance and completed"
    say "        the MCP initialize handshake."
  else
    say "  WARN: self-test did not get a handshake reply. Service log:"
    say "        $STATE_DIR/gateway.log"
  fi
else
  say "  skipped (service socket not up)"
fi

# --------------------------------------------------------------------------- #
# verify + summary
# --------------------------------------------------------------------------- #
hr
say "Verify registrations"
hr
if [ "$DRY_RUN" = 1 ]; then
  say "  [dry-run] skipped"
else
  MISSING=0
  while IFS=$'\t' read -r name cfg; do
    if has "$cfg"; then
      say "  VERIFIED    $name ($cfg)"
    else
      say "  MISSING     $name ($cfg)"
      MISSING=1
    fi
  done < "$REGISTERED_FILE"
  if [ "$MISSING" = 1 ]; then
    say "  (some registrations failed; see the output above)"
  fi
fi

hr
say "Summary"
hr
say "  Binary:     $INSTALL_BIN"
say "  Service:    $PLIST_LABEL (launchctl; logs in $STATE_DIR/gateway.log)"
say "  State dir:  $STATE_DIR (cooldowns, sent records, token, socket)"
say ""
say "Next steps:"
# A stored credential counts only if it is not the placeholder the installer
# writes when no credential is given; step 1 (set up a credential) is shown
# only when a placeholder was written or no credential exists.
real_secret() { # real_secret <file> <placeholder-value>
  [ -s "$1" ] && [ "$(cat "$1" 2>/dev/null)" != "$2" ]
}
if [ "$TRANSPORT" = "mistral-connector" ]; then
  CREDENTIAL_OK="$MISTRAL_CREDENTIAL_WRITTEN"
  if real_secret "$MISTRAL_KEY_FILE" "mistral-placeholder-set-a-real-key"; then
    CREDENTIAL_OK=1
  fi
elif [ "$TRANSPORT" = "korotovsky" ]; then
  CREDENTIAL_OK=0
  if real_secret "$BOT_TOKEN_FILE" "xoxb-placeholder-get-a-real-token" \
     || real_secret "$USER_TOKEN_FILE" "xoxp-placeholder-get-a-real-token"; then
    CREDENTIAL_OK=1
  fi
else
  CREDENTIAL_OK="$MCP_CREDENTIAL_WRITTEN"
  if real_secret "$MCP_TOKEN_FILE" "xoxp-placeholder-get-a-real-token"; then
    CREDENTIAL_OK=1
  fi
fi
STEP=1
if [ "$TRANSPORT" = "mistral-connector" ]; then
  if [ "$CREDENTIAL_OK" = 0 ]; then
    say "  $STEP. In the Mistral console, connect your Slack workspace to the Slack"
    say "      connector (the gateway calls it; it holds the Slack credentials),"
    say "      and set a real Mistral API key:"
    say "        scripts/install.sh --transport mistral-connector --mistral-api-key YOUR_KEY"
    say "      (re-running the installer restarts the service, which reads the"
    say "       file at startup)"
    STEP=$((STEP + 1))
  fi
elif [ "$TRANSPORT" = "korotovsky" ]; then
  if [ "$CREDENTIAL_OK" = 0 ]; then
    say "  $STEP. Install a Slack token for the korotovsky transport:"
    say "        scripts/install.sh --transport korotovsky --slack-bot-token xoxb-your-bot-token"
    say "      (or --slack-user-token xoxp-...; see the scopes in README.md — the"
    say "       gateway spawns the slack-mcp-server you installed"
    say "       (https://github.com/korotovsky/slack-mcp-server) as a stdio child)"
    say "      (re-running the installer restarts the service, which reads the"
    say "       file at startup)"
    STEP=$((STEP + 1))
  fi
  if [ ! -s "$STATE_DIR/slack_workspace_domain" ]; then
    say "  $STEP. Set the workspace domain used in message links (the server's"
    say "      replies carry no permalink):"
    say "        scripts/install.sh --transport korotovsky --workspace-domain YOUR_WORKSPACE"
    STEP=$((STEP + 1))
  fi
else
  if [ "$CREDENTIAL_OK" = 0 ]; then
    say "  $STEP. Get a Slack user token for the Slack MCP server (one-time):"
    say "        scripts/slack_mcp_oauth.py --client-id YOUR_SLACK_APP_CLIENT_ID \\"
    say "            --client-secret YOUR_SLACK_APP_CLIENT_SECRET"
    say "      It opens the Slack authorize page, verifies the token against"
    say "      https://mcp.slack.com/mcp, and writes $MCP_TOKEN_FILE (chmod 600)."
    say "      You need a Slack app (directory-published or internal) with the"
    say "      redirect URL http://127.0.0.1:3118/callback and the user scopes"
    say "      chat:write, users:read, channels:read — see README.md."
    say "      Or set the token directly:"
    say "        scripts/install.sh --slack-mcp-token xoxp-your-user-token"
    say "      Then restart the service so it picks the token up:"
    say "        launchctl kickstart -k gui/$(id -u)/com.speakeasy.slack-approval-gateway"
    say "      (re-running the installer restarts it too)"
    STEP=$((STEP + 1))
  fi
fi
say "  $STEP. Restart your agents (or start a new session) so they pick up the"
say "      new MCP server; then ask any agent to send a Slack message."
STEP=$((STEP + 1))
say "  $STEP. The popup appears, the gateway sends only what you approve, and"
say "      closing the popup never sends anything."
say ""
say "  Uninstall: scripts/install.sh --uninstall"
say "  Enforce exclusivity (remove direct Slack MCP servers from agents):"
say "     scripts/install.sh --enforce"
