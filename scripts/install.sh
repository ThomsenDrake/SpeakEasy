#!/usr/bin/env bash
# install.sh — install SpeakEasy (the Slack approval gateway) into every
# coding agent on this Mac, and keep the gateway itself running as a login
# service.
#
# What it does:
#   1. Builds the app (the release binary; the UI is pure Rust).
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
#   5b. Registers SpeakEasy's approval hook in every detected agent that has
#      pre-tool hooks (Claude Code + pdev, Codex, Gemini CLI, Copilot CLI,
#      Cursor, Hermes, Amp, Vibe, goose, OpenCode, pi): the agent's OWN Slack
#      tools (Claude Code's Slack plugin, a Mistral connector, korotovsky's
#      server, …) then stop before every Slack write until you approve it in
#      the same SpeakEasy window. This works for a Slack MCP SpeakEasy cannot
#      take over (Claude Code's Slack plugin), and needs no credential.
#   6. Drops a managed instruction block into each agent's instruction file:
#      "send Slack only through the gateway's request_slack_message tool"
#      (or, hooks-only: "use your own Slack tool; the user approves each call").
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
#                      [--take-over FILE[:NAME]] [--no-take-over]
#                      [--no-hooks] [--hook-wait SECS]
#
#   SpeakEasy takes over the Slack MCP you already use: when no credential
#   is stored yet and exactly one Slack MCP server it can reuse is found in
#   your agents' configs (Claude Desktop and Cursor included), the installer
#   copies that server's command or URL and its credentials into the state
#   dir and selects its transport — a korotovsky slack-mcp-server (a
#   browser-session xoxc/xoxd login needs no Slack app), the official Slack
#   MCP server with a token in its headers, or a Mistral Slack connector
#   with a key in its headers. An OAuth session owned by an agent (Claude
#   Code's Slack plugin) cannot be taken over.
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
#   --transport   how the gateway sends to Slack (default: slack-mcp, or
#                 hooks-only when SpeakEasy has no credential and nothing to
#                 take over):
#                 hooks-only        SpeakEasy has no Slack connection of its
#                                    own: agents send with their own Slack
#                                    tools and the approval hooks (5b) ask you
#                                    first. request_slack_message is not
#                                    registered (it could not send);
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
#                                    — you install it, or SpeakEasy reuses the
#                                    command your agent runs (an npx line can
#                                    download it); spawned as a stdio child
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
#   --take-over FILE[:NAME]  take over this Slack MCP server (NAME may be
#                 omitted when FILE has only one), even if a credential is
#                 already stored; the credential files it replaces are moved
#                 to <file>.pre-takeover
#   --no-take-over  never take over an existing Slack MCP server
#   --no-hooks    do not register the approval hooks (5b); existing ones are
#                 removed
#   --hook-wait SECS  how long the approval window waits before a hooked call
#                 is blocked (default 600; 60–86400). Each agent's own hook
#                 timeout is set 60 s longer, because an agent runs the tool
#                 when its hook times out.

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
TAKE_OVER_ARG=""
NO_TAKE_OVER=0
NO_HOOKS=0
HOOK_WAIT=600

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
      --take-over) TAKE_OVER_ARG="$arg" ;;
      --hook-wait) HOOK_WAIT="$arg" ;;
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
    --no-take-over) NO_TAKE_OVER=1 ;;
    --no-hooks) NO_HOOKS=1 ;;
    --hook-wait=*) HOOK_WAIT="${arg#--hook-wait=}" ;;
    --slack-mcp-token|--transport|--mistral-api-key|--slack-bot-token|--slack-user-token|--workspace-domain|--take-over|--hook-wait)
      expect_value="$arg" ;;
    --take-over=*) TAKE_OVER_ARG="${arg#--take-over=}" ;;
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
case "$HOOK_WAIT" in
  ''|*[!0-9]*) echo "--hook-wait needs a number of seconds (60-86400)" >&2; exit 2 ;;
esac
if [ "$HOOK_WAIT" -lt 60 ] || [ "$HOOK_WAIT" -gt 86400 ]; then
  echo "--hook-wait needs a number of seconds (60-86400)" >&2
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

# SpeakEasy's approval hooks, as "agent<TAB>file" rows (agent_config.py
# hook-add/hook-remove/hook-has). The Claude Code entry also covers pdev,
# which reads ~/.claude/settings.json hooks too.
hook_targets() {
  printf '%s\t%s\n' \
    claude "$HOME_DIR/.claude/settings.json" \
    codex "${CODEX_HOME:-$HOME_DIR/.codex}/config.toml" \
    gemini "$HOME_DIR/.gemini/settings.json" \
    copilot "$HOME_DIR/.copilot/hooks/speakeasy.json" \
    cursor "$HOME_DIR/.cursor/hooks.json" \
    hermes "${HERMES_HOME:-$HOME_DIR/.hermes}/config.yaml" \
    amp "$HOME_DIR/.config/amp/plugins/speakeasy.ts" \
    vibe "${VIBE_HOME:-$HOME_DIR/.vibe}/hooks.toml" \
    goose "$HOME_DIR/.agents/plugins/speakeasy-slack-approval/hooks/hooks.json" \
    opencode "$HOME_DIR/.config/opencode/plugins/speakeasy-slack-approval.js" \
    pi "$HOME_DIR/.pi/agent/extensions/speakeasy-slack-gate.ts"
}
remove_all_hooks() {
  local agent file
  while IFS=$'\t' read -r agent file; do
    ac hook-remove "$agent" "$file" || say "  [failed]    could not remove the $agent hook from $file (remove it by hand)"
  done < <(hook_targets)
}
# Remove the request_slack_message registrations (uninstall, and the
# hooks-only setup, where SpeakEasy cannot send).
unregister_mcp_everywhere() {
  run_ok claude mcp remove --scope user "$SERVER_NAME"
  run_ok codex mcp remove "$SERVER_NAME"
  run_ok vibe mcp remove "$SERVER_NAME"
  ac remove-json claude "$HOME_DIR/.claude.json" || say "  [failed]    remove-json claude ~/.claude.json (edit it by hand)"
  ac remove-json pi "$HOME_DIR/.paradev/agent/settings.json" || say "  [failed]    remove-json pi ~/.paradev/agent/settings.json (edit it by hand)"
  ac remove-json pimcp "$HOME_DIR/.paradev/agent/mcp.json" || say "  [failed]    remove-json pimcp ~/.paradev/agent/mcp.json (edit it by hand)"
  ac remove-json pimcp "$HOME_DIR/.pi/agent/mcp.json" || say "  [failed]    remove-json pimcp ~/.pi/agent/mcp.json (edit it by hand)"
  ac remove-json opencode "$HOME_DIR/.config/opencode/opencode.json" || say "  [failed]    remove-json opencode ~/.config/opencode/opencode.json (edit it by hand)"
  ac remove-json gemini "$HOME_DIR/.gemini/settings.json" || say "  [failed]    remove-json gemini ~/.gemini/settings.json (edit it by hand)"
  ac remove-json copilot "$HOME_DIR/.copilot/mcp-config.json" || say "  [failed]    remove-json copilot ~/.copilot/mcp-config.json (edit it by hand)"
  ac remove-json amp "$HOME_DIR/.config/amp/settings.json" || say "  [failed]    remove-json amp ~/.config/amp/settings.json (edit it by hand)"
  ac remove-json pi "$HOME_DIR/.pi/agent/settings.json" || say "  [failed]    remove-json pi ~/.pi/agent/settings.json (edit it by hand)"
  ac remove-zed "$HOME_DIR/.config/zed/settings.json" || say "  [failed]    remove-zed ~/.config/zed/settings.json (edit it by hand)"
  ac remove-goose "$HOME_DIR/.config/goose/config.yaml" || say "  [failed]    remove-goose ~/.config/goose/config.yaml (edit it by hand)"
  ac remove-hermes "$HOME_DIR/.hermes/config.yaml" || say "  [failed]    remove-hermes ~/.hermes/config.yaml (edit it by hand)"
  ac remove-codex "$HOME_DIR/.codex/config.toml" || say "  [failed]    remove-codex ~/.codex/config.toml (edit it by hand)"
  ac remove-vibe "$HOME_DIR/.vibe/config.toml" || say "  [failed]    remove-vibe ~/.vibe/config.toml (edit it by hand)"
}

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
  unregister_mcp_everywhere
  remove_all_hooks
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
  say "Restart running agents: a session started before the uninstall keeps"
  say "its hooks, which now block Slack calls because SpeakEasy is gone."
  say "Uninstalled. SpeakEasy's own entries, approval hooks and instruction"
  say "blocks were removed from your agent configs; your other changes to those files"
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
# cargo is only required when a build actually runs: not for --uninstall
# (which exits above), not for --skip-build, not for --dry-run.
if [ "$SKIP_BUILD" = 0 ] && [ "$DRY_RUN" = 0 ]; then
  if ! command -v cargo >/dev/null 2>&1; then
    say "missing required tool: cargo (needed to build; or pass --skip-build" >&2
    say "to reuse the existing release binary)" >&2
    exit 1
  fi
fi
if [ "$SKIP_BUILD" = 1 ]; then
  say "  skipping build (--skip-build)"
else
  if [ "$DRY_RUN" = 0 ]; then
    say "  cargo build --release"
    (cd "$ROOT" && cargo build --release)
  else
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

# --- take over the Slack MCP the user already has
# SpeakEasy reuses a Slack MCP server an agent is already set up with — its
# command or URL and its credentials — so the user needs no new Slack app or
# token. Automatic when nothing is stored yet, no credential flag is given,
# and exactly one server can be taken over; --take-over picks one
# explicitly; --no-take-over skips this.
TAKEOVER_FILES=(
  "$HOME_DIR/.claude.json"
  "$HOME_DIR/.codex/config.toml"
  "$HOME_DIR/.config/opencode/opencode.json"
  "$HOME_DIR/.vibe/config.toml"
  "$HOME_DIR/.config/goose/config.yaml"
  "$HOME_DIR/.pi/agent/settings.json"
  "$HOME_DIR/.pi/agent/mcp.json"
  "$HOME_DIR/.paradev/agent/settings.json"
  "$HOME_DIR/.paradev/agent/mcp.json"
  "$HOME_DIR/.gemini/settings.json"
  "$HOME_DIR/.copilot/mcp-config.json"
  "$HOME_DIR/.config/zed/settings.json"
  "$HOME_DIR/.config/amp/settings.json"
  "$HOME_DIR/.hermes/config.yaml"
  "$HOME_DIR/Library/Application Support/Claude/claude_desktop_config.json"
  "$HOME_DIR/.cursor/mcp.json"
  "$HOME_DIR/.claude/plugins/installed_plugins.json"
)
stdio_env_credential() { # slack_stdio_env holds a login the server accepts
  local f="$STATE_DIR/slack_stdio_env"
  [ -s "$f" ] || return 1
  grep -Eq '^[[:space:]]*SLACK_MCP_XOX[PB]_TOKEN[[:space:]]*=[[:space:]]*[^[:space:]]' "$f" && return 0
  grep -Eq '^[[:space:]]*SLACK_MCP_XOXC_TOKEN[[:space:]]*=[[:space:]]*[^[:space:]]' "$f" \
    && grep -Eq '^[[:space:]]*SLACK_MCP_XOXD_TOKEN[[:space:]]*=[[:space:]]*[^[:space:]]' "$f"
}
credential_transport() { # the transport a given or stored credential is for
  if [ -n "$MCP_TOKEN_ARG" ] || [ -n "${SLACK_MCP_TOKEN:-}" ]; then echo slack-mcp; return; fi
  if [ -n "$MISTRAL_KEY_ARG" ]; then echo mistral-connector; return; fi
  if [ -n "$BOT_TOKEN_ARG$USER_TOKEN_ARG" ]; then echo korotovsky; return; fi
  local f first
  for f in slack_mcp_token:slack-mcp mistral_api_key:mistral-connector \
           slack_bot_token:korotovsky slack_user_token:korotovsky; do
    [ -s "$STATE_DIR/${f%%:*}" ] || continue
    first="$(head -n 1 "$STATE_DIR/${f%%:*}")"
    case "$first" in
      xoxb-placeholder-*|xoxp-placeholder-*|mistral-placeholder-*) ;;
      *) echo "${f#*:}"; return ;;
    esac
  done
  if stdio_env_credential; then echo korotovsky; fi
}
stored_credential() { # a real (non-placeholder) credential is already stored
  local f first
  stdio_env_credential && return 0
  for f in slack_mcp_token mistral_api_key slack_bot_token slack_user_token; do
    [ -s "$STATE_DIR/$f" ] || continue
    first="$(head -n 1 "$STATE_DIR/$f")"
    case "$first" in
      xoxb-placeholder-*|xoxp-placeholder-*|mistral-placeholder-*) ;;
      *) return 0 ;;
    esac
  done
  return 1
}
TAKEOVER_TRANSPORT=""
TAKEOVER_HEAD=""
TAKEOVER_PICK=""
TAKEOVER_SOURCE=""
if [ "$NO_TAKE_OVER" = 0 ]; then
  TAKEOVER_EXISTING=()
  if [ -n "$TAKE_OVER_ARG" ]; then
    # --take-over reads exactly the file it names (FILE or FILE:NAME)
    TAKEOVER_ARG_FILE="$("$PY" -c 'import os, sys
w = sys.argv[1]
p, sep, n = w.rpartition(":")
print(os.path.expanduser(w if (not sep or os.path.exists(os.path.expanduser(w))) else p))' "$TAKE_OVER_ARG")"
    if [ ! -f "$TAKEOVER_ARG_FILE" ]; then
      say "  --take-over $TAKE_OVER_ARG: no such file: $TAKEOVER_ARG_FILE" >&2
      exit 2
    fi
    TAKEOVER_EXISTING=("$TAKEOVER_ARG_FILE")
  else
    for f in "${TAKEOVER_FILES[@]}"; do
      [ -f "$f" ] && TAKEOVER_EXISTING+=("$f")
    done
  fi
  CANDIDATES_JSON="[]"
  if [ "${#TAKEOVER_EXISTING[@]}" -gt 0 ]; then
    CANDIDATES_JSON="$("$PY" "$AGENT_CONFIG" takeover-candidates "${TAKEOVER_EXISTING[@]}" 2>/dev/null || echo '[]')"
  fi
  AUTO=0
  if [ -z "$TAKE_OVER_ARG" ] && [ -z "$MCP_TOKEN_ARG$MISTRAL_KEY_ARG$BOT_TOKEN_ARG$USER_TOKEN_ARG" ] \
     && ! stored_credential; then
    AUTO=1
  fi
  # pick: "take<TAB>file<TAB>name<TAB>transport<TAB>summary", "many", "none",
  # or "error<TAB>message"; the report lines follow on stdout
  TAKEOVER_PICK="$("$PY" - "$CANDIDATES_JSON" "$TAKE_OVER_ARG" "$TRANSPORT_ARG" "$AUTO" <<'EOF'
import json, os, sys
cands, want, transport, auto = json.loads(sys.argv[1]), sys.argv[2], sys.argv[3], sys.argv[4] == "1"
home = os.path.expanduser("~")
short = lambda p: p.replace(home, "~", 1)

def distinct(cs):
    """One candidate per server: copies of the same config (several agents,
    several project scopes) share a fingerprint."""
    seen, out = set(), []
    for c in cs:
        if c.get("fingerprint") in seen:
            continue
        if c.get("fingerprint"):
            seen.add(c["fingerprint"])
        out.append(c)
    return out

def take(c):
    print("take\t%s\t%s\t%s\t%s" % (c["file"], c["name"], c["transport"], c["summary"]))

if want:
    p, sep, name = want.rpartition(":")
    if not sep or os.path.exists(os.path.expanduser(want)):
        name = ""
    match = [c for c in cands if not name or c["name"] == name]
    usable = distinct([c for c in match if c["importable"]])
    if name and not match:
        print("error\t--take-over %s: no Slack MCP server named %s there (found: %s)"
              % (want, name, ", ".join(c["name"] for c in cands) or "none"))
    elif len(usable) == 1:
        c = usable[0]
        if transport and c["transport"] != transport:
            print("error\t--take-over %s is a %s server, but --transport is %s" % (want, c["transport"], transport))
        else:
            take(c)
    elif not usable and match:
        print("error\t--take-over %s: %s" % (want, match[0]["reason"]))
    elif not usable:
        print("error\t--take-over %s: no Slack MCP server there" % want)
    else:
        print("error\t--take-over %s: several Slack MCP servers there; name one: %s"
              % (want, ", ".join(c["name"] for c in usable)))
    sys.exit(0)
usable = distinct([c for c in cands if c["importable"] and (not transport or c["transport"] == transport)])
if auto and len(usable) == 1:
    take(usable[0])
elif auto and len(usable) > 1:
    print("many")
    for c in usable:
        print("    scripts/install.sh --take-over '%s:%s'   # %s" % (short(c["file"]), c["name"], c["summary"]))
else:
    print("none")
    if auto:
        for c in cands:
            if not c["importable"]:
                print("    %s in %s: %s" % (c["summary"], short(c["file"]), c["reason"]))
EOF
)"
  TAKEOVER_HEAD="$(printf '%s\n' "$TAKEOVER_PICK" | head -n 1)"
  case "$TAKEOVER_HEAD" in
    take*)
      IFS=$'\t' read -r _ TAKEOVER_FILE TAKEOVER_NAME TAKEOVER_TRANSPORT TAKEOVER_SUMMARY <<< "$TAKEOVER_HEAD"
      TAKEOVER_SOURCE="$TAKEOVER_NAME in $TAKEOVER_FILE"
      say "  taking over your Slack MCP: $TAKEOVER_SUMMARY"
      say "    ($TAKEOVER_SOURCE)"
      if [ "$DRY_RUN" = 0 ]; then
        "$PY" "$AGENT_CONFIG" take-over "$STATE_DIR" "$TAKEOVER_FILE" "$TAKEOVER_NAME" | sed 's/^/  /'
      else
        say "  [dry-run] agent_config take-over $STATE_DIR <$TAKEOVER_SOURCE>"
      fi
      if [ "$TAKEOVER_TRANSPORT" = "mistral-connector" ]; then
        say "  Agents that have this connector can still send through it, and"
        say "  --enforce keeps connectors; see README.md (Security)."
      elif ! "$PY" "$AGENT_CONFIG" can-enforce "$TAKEOVER_FILE"; then
        say "  The agent can still post through it directly, and --enforce can't"
        say "  edit this file: remove server $TAKEOVER_NAME from $TAKEOVER_FILE"
        say "  by hand so only SpeakEasy can send."
      elif [ "$ENFORCE" = 1 ]; then
        say "  --enforce below removes it from the agent's config, so only"
        say "  SpeakEasy can send through it."
      else
        say "  The agent can still post through it directly. Run the installer"
        say "  with --enforce to remove it there, so only SpeakEasy can send."
      fi
      ;;
    error*)
      say "  ${TAKEOVER_HEAD#error?}" >&2
      exit 2
      ;;
    many)
      say "  Found several Slack MCP servers SpeakEasy could take over; pick one:"
      printf '%s\n' "$TAKEOVER_PICK" | tail -n +2
      if [ "$ENFORCE" = 1 ]; then
        # --enforce would remove these from the agents' configs, and the
        # commands above would then have nothing left to take over.
        say "  Pick one with --take-over before using --enforce." >&2
        exit 2
      fi
      ;;
    none)
      if [ "$(printf '%s\n' "$TAKEOVER_PICK" | wc -l)" -gt 1 ]; then
        say "  Found Slack MCP servers SpeakEasy cannot take over:"
        printf '%s\n' "$TAKEOVER_PICK" | tail -n +2
        say "  The agents keep using them: SpeakEasy's approval hooks (step 5b) make"
        say "  each Slack write wait for you. To also give SpeakEasy a login of its"
        say "  own, see README.md, \"If there's nothing to take over\"."
      fi
      ;;
  esac
fi

# --- transport selection (--transport slack-mcp|mistral-connector|korotovsky|hooks-only)
TRANSPORT="$TRANSPORT_ARG"
if [ -z "$TRANSPORT" ]; then TRANSPORT="slack-mcp"; fi
case "$TRANSPORT" in
  slack-mcp|mistral-connector|korotovsky|hooks-only) ;;
  *) say "unknown --transport value: $TRANSPORT (expected slack-mcp, mistral-connector, korotovsky or hooks-only)" >&2; exit 2 ;;
esac
if [ "$TRANSPORT_ARG" = "hooks-only" ] && [ "$NO_HOOKS" = 1 ]; then
  say "--transport hooks-only needs the approval hooks; drop --no-hooks" >&2
  exit 2
fi
# With no credential anywhere (no flag, no stored file, nothing taken over)
# the default is hooks-only: agents keep their own Slack tools and the hooks
# ask first. A credential flag or $SLACK_MCP_TOKEN keeps the old default.
CREDENTIAL_INPUT=0
if [ -n "$MCP_TOKEN_ARG$MISTRAL_KEY_ARG$BOT_TOKEN_ARG$USER_TOKEN_ARG" ] \
   || [ -n "${SLACK_MCP_TOKEN:-}" ]; then
  CREDENTIAL_INPUT=1
fi
TRANSPORT_FILE="$STATE_DIR/slack_transport"
if [ -n "$TAKEOVER_TRANSPORT" ]; then
  # the take-over above wrote $TRANSPORT_FILE
  TRANSPORT="$TAKEOVER_TRANSPORT"
  say "  transport: $TRANSPORT (taken over)"
elif [ -n "$TRANSPORT_ARG" ]; then
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
    hooks-only)
      # A credential given now, or stored since (scripts/slack_mcp_oauth.py
      # writes slack_mcp_token), gives SpeakEasy a login of its own.
      CRED_TRANSPORT="$(credential_transport)"
      if [ -n "$CRED_TRANSPORT" ]; then
        TRANSPORT="$CRED_TRANSPORT"
        if [ "$DRY_RUN" = 0 ]; then
          printf '%s\n' "$TRANSPORT" > "$TRANSPORT_FILE"
        else
          say "  [dry-run] write $TRANSPORT_FILE ($TRANSPORT)"
        fi
        say "  transport: $TRANSPORT (was hooks-only; SpeakEasy now has a $TRANSPORT"
        say "  credential, so request_slack_message is registered again)"
      else
        TRANSPORT="hooks-only"
        say "  keeping existing transport selection in $TRANSPORT_FILE (hooks-only)"
      fi
      ;;
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
elif [ "$CREDENTIAL_INPUT" = 0 ] && [ "$NO_HOOKS" = 0 ] && ! stored_credential; then
  TRANSPORT="hooks-only"
  if [ "$DRY_RUN" = 0 ]; then
    printf '%s\n' "$TRANSPORT" > "$TRANSPORT_FILE"
  else
    say "  [dry-run] write $TRANSPORT_FILE ($TRANSPORT)"
  fi
  say "  transport: hooks-only (SpeakEasy has no Slack login and found none it"
  say "  could take over; your agents keep sending with their own Slack tools"
  say "  and the approval hooks ask you first — see step 5b)"
else
  say "  transport: slack-mcp (default; SLACK_TRANSPORT or $TRANSPORT_FILE can change it)"
fi
if [ "$TRANSPORT" = "hooks-only" ] && [ "$NO_HOOKS" = 1 ]; then
  say "SpeakEasy is set up hooks-only ($TRANSPORT_FILE): with --no-hooks it could" >&2
  say "neither send nor ask about your agents' sends. Give it a credential (or" >&2
  say "--transport) first, or drop --no-hooks." >&2
  exit 2
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
if [ "$TAKEOVER_TRANSPORT" = "mistral-connector" ]; then
  say "  using the Mistral API key taken over in $MISTRAL_KEY_FILE"
elif [ "$TRANSPORT" = "mistral-connector" ] || [ -n "$MISTRAL_KEY_ARG" ]; then
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
if [ "$TAKEOVER_TRANSPORT" = "slack-mcp" ]; then
  say "  using the Slack MCP token taken over in $MCP_TOKEN_FILE"
elif [ "$TRANSPORT" = "slack-mcp" ]; then
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
    if [ "$TAKEOVER_HEAD" = "many" ]; then
      say "  Wrote a placeholder to $MCP_TOKEN_FILE: SpeakEasy can't start until you"
      say "  pick one of the Slack MCP servers listed above with --take-over."
    else
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
  # With a login in slack_stdio_env (taken over or written by hand), an
  # ambient $SLACK_BOT_TOKEN/$SLACK_USER_TOKEN must not become a token file:
  # the server would prefer it over that login.
  STDIO_LOGIN=0
  if [ -n "$TAKEOVER_TRANSPORT" ] || stdio_env_credential; then STDIO_LOGIN=1; fi
  if [ -n "$BOT_TOKEN_ARG" ]; then
    BOT_TOKEN="$BOT_TOKEN_ARG"
  elif [ ! -f "$BOT_TOKEN_FILE" ] && [ "$STDIO_LOGIN" = 0 ]; then
    BOT_TOKEN="${SLACK_BOT_TOKEN:-}"
  else
    BOT_TOKEN=""
  fi
  if [ -n "$USER_TOKEN_ARG" ]; then
    USER_TOKEN="$USER_TOKEN_ARG"
  elif [ ! -f "$USER_TOKEN_FILE" ] && [ "$STDIO_LOGIN" = 0 ]; then
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
  if [ -n "$TAKEOVER_TRANSPORT" ] || stdio_env_credential; then
    say "  using the server settings in $STATE_DIR/slack_stdio_env"
    # A placeholder token file would reach the server and win over that
    # login (it prefers xoxp, then xoxb, then xoxc/xoxd).
    for f in "$BOT_TOKEN_FILE" "$USER_TOKEN_FILE"; do
      case "$(head -n 1 "$f" 2>/dev/null)" in
        xoxb-placeholder-*|xoxp-placeholder-*)
          if [ "$DRY_RUN" = 0 ]; then rm -f "$f"; fi
          say "  removed the placeholder $f" ;;
        xox*)
          say "  WARNING: $f also holds a token, and the server uses it instead"
          say "  of the login in slack_stdio_env (it prefers xoxp, then xoxb, then"
          say "  xoxc/xoxd). To use slack_stdio_env, move it aside:"
          say "    mv $f $f.pre-takeover" ;;
      esac
    done
  elif [ -s "$STATE_DIR/slack_stdio_env" ]; then
    say "  WARNING: $STATE_DIR/slack_stdio_env has no usable login: it needs"
    say "  SLACK_MCP_XOXP_TOKEN, SLACK_MCP_XOXB_TOKEN, or both SLACK_MCP_XOXC_TOKEN"
    say "  and SLACK_MCP_XOXD_TOKEN."
  fi
  if [ -z "$BOT_TOKEN" ] && [ -z "$USER_TOKEN" ] \
     && [ ! -f "$BOT_TOKEN_FILE" ] && [ ! -f "$USER_TOKEN_FILE" ] \
     && [ ! -s "$STATE_DIR/slack_stdio_env" ] && [ -z "$TAKEOVER_TRANSPORT" ]; then
    if [ "$DRY_RUN" = 0 ]; then
      umask 077
      printf '%s\n' "xoxb-placeholder-get-a-real-token" > "$BOT_TOKEN_FILE"
      chmod 600 "$BOT_TOKEN_FILE"
    else
      say "  [dry-run] write placeholder $BOT_TOKEN_FILE"
    fi
    say "  WARNING: no Slack credential for the korotovsky transport. Wrote a"
    say "  placeholder to $BOT_TOKEN_FILE; SpeakEasy cannot start until it has a"
    say "  real one (the server checks the token at startup). Either:"
    say "    - let SpeakEasy take over the korotovsky server you already use:"
    say "        scripts/install.sh --take-over FILE[:NAME]"
    say "      (FILE is the agent config where it is registered, e.g."
    say "      ~/.cursor/mcp.json; a browser-session login, xoxc + xoxd, needs no"
    say "      Slack app); or"
    say "    - pass a token: --slack-bot-token xoxb-... or --slack-user-token xoxp-..."
    say "      (these come from a Slack app with the scopes listed in README.md)."
    say "  Re-running the installer restarts the service, which reads the files"
    say "  at startup."
  fi

  # --- the server command (SpeakEasy never installs the server)
  # Unless the command file already exists, detect `slack-mcp-server` on
  # this PATH and record its absolute path — with --transport stdio, the
  # server's documented stdio invocation — so the login service can spawn
  # it. When it is missing, the gateway fails at startup with instructions;
  # install it yourself, never via this script.
  STDIO_COMMAND_FILE="$STATE_DIR/slack_stdio_command"
  if [ -n "$TAKEOVER_TRANSPORT" ]; then
    say "  korotovsky server: the command taken over in $STDIO_COMMAND_FILE"
  elif [ -f "$STDIO_COMMAND_FILE" ]; then
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
if [ "$TRANSPORT" = "hooks-only" ]; then
  say "  hooks-only: SpeakEasy has no Slack connection of its own, so"
  say "  request_slack_message could not send — it is not registered (earlier"
  say "  registrations are removed). Agents send with their own Slack tools;"
  say "  the approval hooks (5b) ask you first."
  unregister_mcp_everywhere
else

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

# --- pdev / paradev (pi-based: MCP servers in ~/.paradev/agent/mcp.json)
if command -v pdev >/dev/null 2>&1 || [ -d "$HOME_DIR/.paradev" ]; then
  CFG="$HOME_DIR/.paradev/agent/mcp.json"
  # older installs wrote settings.json, which pdev never reads for MCP
  ac remove-json pi "$HOME_DIR/.paradev/agent/settings.json" >/dev/null || true
  if has "$CFG"; then
    note_registered_existing pdev "$CFG"
  else
    ac add-json pimcp "$CFG" "$INSTALL_BIN"
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

# --- pi (1.4+: MCP servers in ~/.pi/agent/mcp.json)
if command -v pi >/dev/null 2>&1 || [ -d "$HOME_DIR/.pi/agent" ]; then
  CFG="$HOME_DIR/.pi/agent/mcp.json"
  # older installs wrote settings.json, which pi 1.4 ignores for MCP
  ac remove-json pi "$HOME_DIR/.pi/agent/settings.json" >/dev/null || true
  if has "$CFG"; then
    note_registered_existing pi "$CFG"
  else
    ac add-json pimcp "$CFG" "$INSTALL_BIN"
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
fi

say ""
say "  Superset workspaces run these same agent CLIs as this user, so they"
say "  pick up the registrations above automatically — no per-workspace setup."

# --------------------------------------------------------------------------- #
# 5b. approval hooks: the agents' own Slack tools ask SpeakEasy first
# --------------------------------------------------------------------------- #
hr
say "5b. Approval hooks for the agents' own Slack tools"
hr
# "agent<TAB>file" rows of the hooks registered here, verified at the end
HOOKED_FILE="$(mktemp 2>/dev/null || echo /tmp/sag-install-hooked.$$)"
trap 'rm -f "$REGISTERED_FILE" "$HOOKED_FILE"' EXIT
hook_detected() { # hook_detected <agent> — is the agent installed here?
  case "$1" in
    claude) [ -d "$HOME_DIR/.claude" ] || command -v claude >/dev/null 2>&1 \
              || [ -d "$HOME_DIR/.paradev" ] ;;
    codex) [ -d "${CODEX_HOME:-$HOME_DIR/.codex}" ] || command -v codex >/dev/null 2>&1 ;;
    gemini) [ -d "$HOME_DIR/.gemini" ] ;;
    copilot) [ -d "$HOME_DIR/.copilot" ] ;;
    cursor) [ -d "$HOME_DIR/.cursor" ] ;;
    hermes) [ -f "${HERMES_HOME:-$HOME_DIR/.hermes}/config.yaml" ] ;;
    amp) [ -d "$HOME_DIR/.config/amp" ] ;;
    vibe) [ -d "${VIBE_HOME:-$HOME_DIR/.vibe}" ] || command -v vibe >/dev/null 2>&1 ;;
    goose) [ -d "$HOME_DIR/.config/goose" ] || command -v goose >/dev/null 2>&1 ;;
    opencode) [ -d "$HOME_DIR/.config/opencode" ] || command -v opencode >/dev/null 2>&1 ;;
    pi) [ -d "$HOME_DIR/.pi/agent" ] || command -v pi >/dev/null 2>&1 ;;
    *) return 1 ;;
  esac
}
if [ "$NO_HOOKS" = 1 ]; then
  say "  --no-hooks: removing SpeakEasy's approval hooks (if any)"
  remove_all_hooks
else
  say "  Each hook stops the agent's own Slack writes (send, schedule, react,"
  say "  canvas, …) until you decide in the SpeakEasy window; it blocks the"
  say "  call when SpeakEasy is not running or no one answers in ${HOOK_WAIT}s."
  while IFS=$'\t' read -r agent file; do
    if ! hook_detected "$agent"; then
      note_skipped "$agent hook (not installed)"
      continue
    fi
    if ac hook-add "$agent" "$file" "$INSTALL_BIN" "$HOOK_WAIT" "$STATE_DIR"; then
      printf '%s\t%s\n' "$agent" "$file" >> "$HOOKED_FILE"
      say "  [ok]        $agent hook — $file"
    else
      say "  [failed]    $agent hook — $file (see the error above)"
    fi
  done < <(hook_targets)
  if [ -d "$HOME_DIR/.paradev" ]; then
    say "  (pdev runs the Claude Code hooks in ~/.claude/settings.json too)"
  fi
  if [ -d "$HOME_DIR/.cursor" ]; then
    say "  (Cursor also imports Claude Code's hooks; SpeakEasy shows one window"
    say "   per tool call either way)"
  fi
  if [ -d "$HOME_DIR/.config/zed" ]; then
    if [ "$TRANSPORT" = "hooks-only" ]; then
      note_skipped "zed (its built-in agent has no hooks; Zed asks before each MCP"
      say "              tool call unless you chose Always allow for it)"
    else
      note_skipped "zed (its built-in agent has no hooks; it uses request_slack_message)"
    fi
  fi
  if [ -d "$HOME_DIR/.config/amp" ] && "$PY" - "$HOME_DIR/.config/amp/settings.json" <<'EOF' 2>/dev/null
import json, sys
sys.exit(0 if json.load(open(sys.argv[1])).get("amp.dangerouslyAllowAll") is True else 1)
EOF
  then
    say "  note: amp.dangerouslyAllowAll is on in ~/.config/amp/settings.json;"
    say "  SpeakEasy's Amp plugin still runs (plugins are not permission rules)."
  fi
fi

# --------------------------------------------------------------------------- #
# 6. instruction blocks ("send only through the gateway")
# --------------------------------------------------------------------------- #
hr
say "6. Agent instruction blocks"
hr
# Which block each agent gets: "use request_slack_message" when SpeakEasy
# can send; in the hooks-only setup, "use your own Slack tool, the user
# approves each call" where the hook was registered, and "ask the user in
# chat first" where it wasn't (Zed's built-in agent has no hooks).
hooked() { # hooked <agent> — the approval hook was registered for it
  [ -s "$HOOKED_FILE" ] && cut -f1 "$HOOKED_FILE" | grep -qx "$1"
}
add_instruction() { # add_instruction <file> <hook agent, or "none">
  local variant="gateway"
  if [ "$TRANSPORT" = "hooks-only" ]; then
    if [ "$2" != none ] && hooked "$2"; then variant="hooks"; else variant="confirm"; fi
  fi
  ac instruction-add "$1" "$variant"
}
if [ -f "$HOME_DIR/.claude.json" ] || [ -d "$HOME_DIR/.claude" ]; then
  add_instruction "$HOME_DIR/.claude/CLAUDE.md" claude
fi
if [ -f "$HOME_DIR/.codex/config.toml" ]; then
  add_instruction "$HOME_DIR/.codex/AGENTS.md" codex
fi
if [ -f "$HOME_DIR/.config/opencode/opencode.json" ]; then
  add_instruction "$HOME_DIR/.config/opencode/AGENTS.md" opencode
fi
if [ -f "$HOME_DIR/.vibe/config.toml" ]; then
  add_instruction "$HOME_DIR/.vibe/AGENTS.md" vibe
fi
if [ -f "$HOME_DIR/.config/goose/config.yaml" ]; then
  add_instruction "$HOME_DIR/.config/goose/.goosehints" goose
fi
if [ -d "$HOME_DIR/.pi/agent" ]; then
  add_instruction "$HOME_DIR/.pi/agent/AGENTS.md" pi
fi
if [ -d "$HOME_DIR/.paradev/agent" ]; then
  # pdev runs the Claude Code hooks
  add_instruction "$HOME_DIR/.paradev/agent/AGENTS.md" claude
fi
if [ -f "$HOME_DIR/.gemini/settings.json" ]; then
  add_instruction "$HOME_DIR/.gemini/GEMINI.md" gemini
fi
if [ -f "$HOME_DIR/.copilot/mcp-config.json" ]; then
  add_instruction "$HOME_DIR/.copilot/copilot-instructions.md" copilot
fi
if [ -f "$HOME_DIR/.hermes/config.yaml" ]; then
  # Hermes loads global instructions from SOUL.md (HERMES_HOME), never from
  # ~/.hermes/AGENTS.md (that file is only read from a working directory)
  add_instruction "$HOME_DIR/.hermes/SOUL.md" hermes
  if has "$HOME_DIR/.hermes/AGENTS.md"; then
    say "  migrating: removing the old block from $HOME_DIR/.hermes/AGENTS.md"
    ac instruction-remove "$HOME_DIR/.hermes/AGENTS.md"
  fi
fi
if [ -f "$HOME_DIR/.config/zed/settings.json" ]; then
  # Zed reads user-level instructions from ~/.config/zed/AGENTS.md; the old
  # rules/slack-gateway.md location is never loaded, so drop it
  add_instruction "$HOME_DIR/.config/zed/AGENTS.md" none
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
  "$HOME_DIR/.pi/agent/mcp.json"
  "$HOME_DIR/.paradev/agent/settings.json"
  "$HOME_DIR/.paradev/agent/mcp.json"
  "$HOME_DIR/.gemini/settings.json"
  "$HOME_DIR/.copilot/mcp-config.json"
  "$HOME_DIR/.config/zed/settings.json"
  "$HOME_DIR/.config/amp/settings.json"
  "$HOME_DIR/.hermes/config.yaml"
  "$HOME_DIR/Library/Application Support/Claude/claude_desktop_config.json"
  "$HOME_DIR/.cursor/mcp.json"
)
# A file taken over with --take-over (a project's config, say) is audited
# too, so --enforce can remove the agent's copy there.
if [ -n "${TAKEOVER_FILE:-}" ] && ! "$PY" - "$TAKEOVER_FILE" "${AUDIT_FILES[@]}" <<'EOF'
import os, sys
f = os.path.realpath(sys.argv[1])
sys.exit(0 if f in {os.path.realpath(p) for p in sys.argv[2:]} else 1)
EOF
then
  AUDIT_FILES+=("$TAKEOVER_FILE")
fi
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
  say "  Both also expose send tools (slack_send_message). In agents with"
  say "  SpeakEasy's approval hook (5b) every such send waits for your approval;"
  say "  elsewhere the boundary is the instruction block. --enforce keeps these"
  say "  (agents are expected to have them)."
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
  if [ -n "$TAKEOVER_TRANSPORT" ] || stored_credential; then
    say "  If one of these is the login SpeakEasy took over, SpeakEasy sends"
    say "  with it too: rotating it (or signing out of that browser session)"
    say "  stops SpeakEasy as well. To take it from the agent, use --enforce"
    say "  and delete that config's .bak-sag backup; after rotating, run"
    say "  --take-over again or update the file in $STATE_DIR."
  fi
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
  if [ "$TRANSPORT" = "mistral-connector" ]; then
    say "  If one of these is the key SpeakEasy uses, rotating it stops"
    say "  SpeakEasy too; give it the new key with --mistral-api-key."
  fi
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

if [ "$ENFORCE" = 1 ] && [ "$TRANSPORT" = "hooks-only" ]; then
  say "  --enforce: skipped. In the hooks-only setup the agents' own Slack"
  say "  servers are how they send (SpeakEasy can't), and the approval hooks"
  say "  make each send wait for you; removing them would cut agents off."
elif [ "$ENFORCE" = 1 ]; then
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
        if "$PY" "$AGENT_CONFIG" can-enforce "$("$PY" -c 'import json,sys; print(json.loads(sys.argv[1])[0])' "$line")"; then
          "$PY" -c 'import json,sys; p,n,k=json.loads(sys.argv[1]); print(f"  [dry-run] would remove server {n} from {p}")' "$line"
        else
          "$PY" -c 'import json,sys; p,n,k=json.loads(sys.argv[1]); print(f"  [dry-run] [edit by hand] {p} (server: {n}) — no automatic removal")' "$line"
        fi
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
  while IFS=$'\t' read -r agent cfg; do
    if "$PY" "$AGENT_CONFIG" hook-has "$agent" "$cfg" 2>/dev/null; then
      say "  VERIFIED    $agent approval hook ($cfg)"
    else
      say "  MISSING     $agent approval hook ($cfg)"
      MISSING=1
    fi
  done < "$HOOKED_FILE"
  # The hook binary itself: a non-Slack call gets no opinion (exit 0).
  if [ -s "$HOOKED_FILE" ]; then
    HOOK_RC=0
    printf '%s' '{"tool_name":"Bash","tool_input":{"command":"true"}}' \
      | perl -e 'alarm shift; exec @ARGV' 15 "$INSTALL_BIN" --hook claude --wait 60 \
          --state-dir "$STATE_DIR" --ok-code 10 >/dev/null 2>&1 || HOOK_RC=$?
    if [ "$HOOK_RC" = 10 ]; then
      say "  VERIFIED    $INSTALL_BIN --hook answers"
    else
      say "  MISSING     $INSTALL_BIN --hook did not answer (hooked Slack calls will be blocked)"
      MISSING=1
    fi
  fi
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
# A taken-over server brings its own credential (for korotovsky, the
# settings in slack_stdio_env, e.g. a browser-session xoxc/xoxd login).
if [ -n "$TAKEOVER_TRANSPORT" ] \
   || { [ "$TRANSPORT" = "korotovsky" ] && stdio_env_credential; }; then
  CREDENTIAL_OK=1
fi
if [ "$TRANSPORT" = "hooks-only" ]; then CREDENTIAL_OK=1; fi
STEP=1
if [ "$CREDENTIAL_OK" = 0 ] && [ "${TAKEOVER_HEAD:-}" = "many" ]; then
  # several Slack MCP servers could be taken over: picking one is the step
  say "  $STEP. Pick the Slack MCP server SpeakEasy should take over:"
  printf '%s\n' "$TAKEOVER_PICK" | tail -n +2 | sed 's/^    /       /'
  STEP=$((STEP + 1))
  CREDENTIAL_OK=1
fi
if [ "$TRANSPORT" = "hooks-only" ]; then
  CREDENTIAL_OK=1
  say "  $STEP. Nothing to set up for Slack: SpeakEasy needs no login in this"
  say "      setup. Your agents keep their own Slack MCP (Claude Code's Slack"
  say "      plugin, …); each Slack write they make waits for you in the"
  say "      SpeakEasy window. To also give agents without hooks (Zed's built-in"
  say "      agent) the request_slack_message tool, give SpeakEasy a Slack login"
  say "      of its own (see README.md) and re-run the installer."
  STEP=$((STEP + 1))
elif [ "$TRANSPORT" = "mistral-connector" ]; then
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
if [ "$TRANSPORT" = "hooks-only" ]; then
  say "  $STEP. Restart your agents (or start a new session) so they load the"
  say "      approval hooks; then ask any agent to send a Slack message."
  STEP=$((STEP + 1))
  say "  $STEP. The SpeakEasy window appears before the agent's own Slack tool"
  say "      runs; only what you approve is sent, and closing the window never"
  say "      approves anything."
else
  say "  $STEP. Restart your agents (or start a new session) so they pick up the"
  say "      new MCP server; then ask any agent to send a Slack message."
  STEP=$((STEP + 1))
  say "  $STEP. The popup appears, the gateway sends only what you approve, and"
  say "      closing the popup never sends anything."
fi
say ""
say "  Uninstall: scripts/install.sh --uninstall"
say "  Enforce exclusivity (remove direct Slack MCP servers from agents):"
say "     scripts/install.sh --enforce"
