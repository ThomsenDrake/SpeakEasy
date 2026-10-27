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
#      "use your Slack tool or the gateway's request_slack_message; the user
#      approves either" (agents with the hook), "send Slack only through
#      request_slack_message" (agents without it), or, hooks-only: "use your
#      own Slack tool; the user approves each call".
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
#                      [--transport slack-mcp|mistral-connector|korotovsky|hooks-only]
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
#                 context, and the approval hooks (5b) gate their send
#                 tools. Skipped in the hooks-only setup, where the agents'
#                 own servers are how they send.
#   --uninstall   remove the MCP registrations, approval hooks, instruction
#                 blocks, the LaunchAgent, and the installed binary (state
#                 dir kept); exits 1 and lists what it could not remove
#   --purge       with --uninstall: also delete the state dir (cooldowns,
#                 sent records, token file)
#   --skip-build  reuse the existing target/release binary (a release
#                 download installs its prebuilt bin/ binary and never builds)
#   --slack-mcp-token  Slack user OAuth token (xoxp-...) for the official
#                 Slack MCP server. Stored only in
#                 <state-dir>/slack_mcp_token (chmod 600). With --transport
#                 slack-mcp and no token, a placeholder is written and
#                 SpeakEasy cannot start until you get a real token (with no
#                 credential at all and nothing to take over, the default is
#                 hooks-only instead):  scripts/slack_mcp_oauth.py --client-id ID
#                 --client-secret SECRET   (an existing token file is kept
#                 unless this flag is given; $SLACK_MCP_TOKEN is only a
#                 fallback)
#   --transport   how the gateway sends to Slack. Default: the transport of
#                 the server taken over, or of the credential you pass
#                 (--slack-mcp-token -> slack-mcp, --mistral-api-key ->
#                 mistral-connector, --slack-bot-token / --slack-user-token ->
#                 korotovsky; a flag switches an earlier choice), else the
#                 saved choice; hooks-only when there is no credential and
#                 nothing to take over; otherwise slack-mcp. The choice is
#                 saved in <state-dir>/slack_transport:
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
#                                    command your agent runs (an npx or go run
#                                    line can fetch it); spawned as a stdio
#                                    child process) with a Slack bot token
#                                    (xoxb-..., default) or user token
#                                    (xoxp-...) — see the flags below — or the
#                                    login in <state-dir>/slack_stdio_env (taken
#                                    over, or written by hand: e.g. a browser
#                                    session's xoxc + xoxd).
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
#                 (chmod 600). Falls back to $SLACK_USER_TOKEN. The korotovsky
#                 transport needs this or --slack-bot-token (a bot token is
#                 the usual choice: the server posts as the app), unless
#                 <state-dir>/slack_stdio_env holds a login (taken over, or
#                 written by hand, e.g. a browser session).
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
#                 removed. SpeakEasy then needs a Slack login of its own: with
#                 no credential, nothing to take over and no --transport, the
#                 installer stops before changing anything
#   --hook-wait SECS  how long the approval window waits before a hooked call
#                 is blocked (default 600; 60–86400). Each agent's own hook
#                 timeout is set 60 s longer, because an agent runs the tool
#                 when its hook times out.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# The installer as the user can run it from anywhere, for the commands this
# output suggests (~ for the home folder)
SELF="$ROOT/scripts/install.sh"
OAUTH="$ROOT/scripts/slack_mcp_oauth.py"
case "$SELF" in "$HOME"/*) SELF="~${SELF#"$HOME"}"; OAUTH="~${OAUTH#"$HOME"}" ;; esac
BIN_NAME="slack-approval-gateway"
SERVER_NAME="slack-approval-gateway"
HOME_DIR="${HOME:?no HOME}"
DEFAULT_STATE_DIR="$HOME_DIR/.slack-approval-gateway"
INSTALL_BIN="$HOME_DIR/.local/bin/$BIN_NAME"
# Agents that move their config home with an environment variable: every
# step (MCP registration, hooks, instructions, audit) uses the same place.
CODEX_DIR="${CODEX_HOME:-$HOME_DIR/.codex}"
VIBE_DIR="${VIBE_HOME:-$HOME_DIR/.vibe}"
HERMES_DIR="${HERMES_HOME:-$HOME_DIR/.hermes}"
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
case "$TRANSPORT_ARG" in
  ''|slack-mcp|mistral-connector|korotovsky|hooks-only) ;;
  *) echo "unknown --transport value: $TRANSPORT_ARG (expected slack-mcp, mistral-connector, korotovsky or hooks-only)" >&2; exit 2 ;;
esac
if [ "$TRANSPORT_ARG" = "hooks-only" ] && [ "$NO_HOOKS" = 1 ]; then
  echo "--transport hooks-only needs the approval hooks; drop --no-hooks" >&2
  exit 2
fi
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
    # only the pre-install original (not a file holding SpeakEasy's content)
    "$PY" "$AGENT_CONFIG" backup "$f" || true
  fi
}

# registered agents, as "name<TAB>config-file" rows, verified at the end
REGISTERED_FILE="$(mktemp 2>/dev/null || echo /tmp/sag-install-registered.$$)"
trap 'rm -f "$REGISTERED_FILE" "${OUTDATED_FILE:-}" "${REG_FAILED_FILE:-}"' EXIT
note_registered() { # note_registered <name> <config-file>
  printf '%s\t%s\n' "$1" "$2" >> "$REGISTERED_FILE"
  say "  [ok]        $1 — $2"
}
note_registered_existing() {
  printf '%s\t%s\n' "$1" "$2" >> "$REGISTERED_FILE"
  say "  [ok]        $1 (already registered) — $2"
}
note_skipped() { say "  [skip]      $1"; }
ac_notes() { # ac_notes <output> — agent_config's messages other than its "<file>: <result>" line
  printf '%s\n' "$1" | grep -Ev ': (added|created|updated|unchanged|registered|removed|absent)$|^$' | sed 's/^/  /' || true
}
try_ac() { # try_ac <agent> <file> <agent_config args...> — report a failure, don't abort
  local agent="$1" file="$2" out; shift 2
  if out="$(ac "$@" 2>&1)"; then
    if [ "$DRY_RUN" = 1 ]; then printf '%s\n' "$out"; else ac_notes "$out"; fi
    return 0
  fi
  printf '%s\n' "$out" | sed 's/^/  /'
  printf '%s\t%s\n' "$agent" "$file" >> "$REG_FAILED_FILE"
  say "  [failed]    $agent — $file was left unchanged; add SpeakEasy's entry by hand"
  say "              (see the message above and README.md, \"Register an agent by hand\")"
  return 1
}

long_timeout() { # long_timeout <agent> <file> — a request_slack_message call waits
  # minutes for a human, longer than the agents' default tool timeouts
  local out rc=0
  if [ "$DRY_RUN" = 1 ]; then
    say "  [dry-run] agent_config set-timeout $2 $SERVER_NAME 86400"
    return 0
  fi
  out="$("$PY" "$AGENT_CONFIG" set-timeout "$2" "$SERVER_NAME" 86400 2>&1)" || rc=$?
  if [ "$rc" != 0 ]; then
    printf '%s\n' "$out" | sed 's/^/  /'
    say "  [failed]    $1 tool timeout — give $SERVER_NAME a timeout of 86400s in $2 by hand"
    return 0
  fi
  case "${out##*: }" in
    added|updated) say "  [ok]        $1 tool timeout (86400s, ${out##*: }) — $2" ;;
  esac
}

# agents whose existing entry could not be brought up to date, and agents
# whose registration failed (Verify)
OUTDATED_FILE="$(mktemp 2>/dev/null || echo /tmp/sag-install-outdated.$$)"
REG_FAILED_FILE="$(mktemp 2>/dev/null || echo /tmp/sag-install-regfailed.$$)"
reg_failed() { # reg_failed <agent> — its request_slack_message registration failed
  [ -s "$REG_FAILED_FILE" ] && awk -F '\t' -v a="$1" '$1 == a { f = 1 } END { exit !f }' "$REG_FAILED_FILE"
}
upgrade_entry() { # upgrade_entry <agent> <style, or zed> <file> — fix an entry
  # an older install wrote (OpenCode ignores one without "type", Zed one in
  # the old nested form; without the tool-call timeout approvals are cut short)
  local out rc=0
  if [ "$DRY_RUN" = 1 ]; then
    say "  [dry-run] agent_config upgrade $2 $3"
    return 0
  fi
  if [ "$2" = zed ]; then
    out="$("$PY" "$AGENT_CONFIG" upgrade-zed "$3" "$INSTALL_BIN" 2>&1)" || rc=$?
  else
    out="$("$PY" "$AGENT_CONFIG" upgrade-json "$2" "$3" "$INSTALL_BIN" 2>&1)" || rc=$?
  fi
  if [ "$rc" != 0 ]; then
    printf '%s\n' "$out" | sed 's/^/  /'
    say "  [failed]    $1 entry is out of date — fix it by hand (see above)"
    # a missing timeout only cuts long approvals short; anything else
    # (OpenCode's "type", Zed's old form) keeps the entry from loading
    if printf '%s\n' "$out" | grep -q 'by hand: {"timeout": [0-9]*}$'; then
      printf '%s\t%s\n' "$1" "its tool-call timeout is short: long approvals fail" >> "$OUTDATED_FILE"
    else
      printf '%s\t%s\n' "$1" "its entry won't load as is" >> "$OUTDATED_FILE"
    fi
    return 0
  fi
  case "${out##*: }" in
    updated*) say "  [ok]        $1 entry ${out##*: } — $3" ;;
  esac
}

# SpeakEasy's approval hooks, as "agent<TAB>file" rows (agent_config.py
# hook-add/hook-remove/hook-has). The Claude Code entry also covers pdev,
# which reads ~/.claude/settings.json hooks too.
hook_targets() {
  printf '%s\t%s\n' \
    claude "$HOME_DIR/.claude/settings.json" \
    codex "$CODEX_DIR/config.toml" \
    gemini "$HOME_DIR/.gemini/settings.json" \
    copilot "$HOME_DIR/.copilot/hooks/speakeasy.json" \
    cursor "$HOME_DIR/.cursor/hooks.json" \
    hermes "$HERMES_DIR/config.yaml" \
    amp "$HOME_DIR/.config/amp/plugins/speakeasy.ts" \
    vibe "$VIBE_DIR/hooks.toml" \
    goose "$HOME_DIR/.agents/plugins/speakeasy-slack-approval/hooks/hooks.json" \
    opencode "$HOME_DIR/.config/opencode/plugins/speakeasy-slack-approval.js" \
    pi "$HOME_DIR/.pi/agent/extensions/speakeasy-slack-gate.ts"
}
# what could not be removed (uninstall, --no-hooks): one line each
LEFT_BEHIND=""
LEFT_HOOK=0
left_behind() { say "  [failed]    $1"; LEFT_BEHIND="$LEFT_BEHIND    - $1"$'\n'; }
remove_all_hooks() {
  local agent file
  while IFS=$'\t' read -r agent file; do
    if ! quiet_ac hook-remove "$agent" "$file"; then
      left_behind "the $agent approval hook in $file (remove it by hand)"
      LEFT_HOOK=1
    fi
  done < <(hook_targets)
}
# Remove the request_slack_message registrations (uninstall, and the
# hooks-only setup, where SpeakEasy cannot send).
quiet_ac() { # agent_config, without the "absent" lines; "<file>: removed" as
  # an aligned "[removed]" line, anything else (an error) indented
  local out rc=0
  out="$(ac "$@" 2>&1)" || rc=$?
  printf '%s\n' "$out" | grep -v -e ': absent$' -e '^$' \
    | sed -E -e 's/^(.*): removed \(file was ours; deleted\)$/[removed]   \1 (file deleted: it held only SpeakEasy'"'"'s block)/' \
             -e 's/^(.*): removed$/[removed]   \1/' -e 's/^/  /' || true
  return $rc
}
remove_legacy_zed_rules() { # older installs wrote ~/.config/zed/rules/slack-gateway.md,
  # which Zed never loads; instruction-remove also drops a .bak-sag of it
  # that holds only SpeakEasy's block
  local f="$HOME_DIR/.config/zed/rules/slack-gateway.md"
  quiet_ac instruction-remove "$f" >/dev/null 2>&1 || true
  run rm -f "$f"
}
cli_remove() { # best effort through an agent's own CLI, when it is installed
  # (its "no such server" error when nothing is registered is not news)
  command -v "$1" >/dev/null 2>&1 || return 0
  if [ "$DRY_RUN" = 1 ]; then say "  [dry-run] $*"; else "$@" >/dev/null 2>&1 || true; fi
}
unregister_mcp_everywhere() {
  cli_remove claude mcp remove --scope user "$SERVER_NAME"
  cli_remove codex mcp remove "$SERVER_NAME"
  cli_remove vibe mcp remove "$SERVER_NAME"
  quiet_ac remove-json claude "$HOME_DIR/.claude.json" || left_behind "SpeakEasy's MCP entry in $HOME_DIR/.claude.json (edit it by hand)"
  quiet_ac remove-json pi "$HOME_DIR/.paradev/agent/settings.json" || left_behind "SpeakEasy's MCP entry in $HOME_DIR/.paradev/agent/settings.json (edit it by hand)"
  quiet_ac remove-json pimcp "$HOME_DIR/.paradev/agent/mcp.json" || left_behind "SpeakEasy's MCP entry in $HOME_DIR/.paradev/agent/mcp.json (edit it by hand)"
  quiet_ac remove-json pimcp "$HOME_DIR/.pi/agent/mcp.json" || left_behind "SpeakEasy's MCP entry in $HOME_DIR/.pi/agent/mcp.json (edit it by hand)"
  quiet_ac remove-json opencode "$HOME_DIR/.config/opencode/opencode.json" || left_behind "SpeakEasy's MCP entry in $HOME_DIR/.config/opencode/opencode.json (edit it by hand)"
  quiet_ac remove-json gemini "$HOME_DIR/.gemini/settings.json" || left_behind "SpeakEasy's MCP entry in $HOME_DIR/.gemini/settings.json (edit it by hand)"
  quiet_ac remove-json copilot "$HOME_DIR/.copilot/mcp-config.json" || left_behind "SpeakEasy's MCP entry in $HOME_DIR/.copilot/mcp-config.json (edit it by hand)"
  quiet_ac remove-json amp "$HOME_DIR/.config/amp/settings.json" || left_behind "SpeakEasy's MCP entry in $HOME_DIR/.config/amp/settings.json (edit it by hand)"
  quiet_ac remove-json pi "$HOME_DIR/.pi/agent/settings.json" || left_behind "SpeakEasy's MCP entry in $HOME_DIR/.pi/agent/settings.json (edit it by hand)"
  quiet_ac remove-zed "$HOME_DIR/.config/zed/settings.json" || left_behind "SpeakEasy's MCP entry in $HOME_DIR/.config/zed/settings.json (edit it by hand)"
  quiet_ac remove-goose "$HOME_DIR/.config/goose/config.yaml" || left_behind "SpeakEasy's MCP entry in $HOME_DIR/.config/goose/config.yaml (edit it by hand)"
  quiet_ac remove-hermes "$HERMES_DIR/config.yaml" || left_behind "SpeakEasy's MCP entry in $HERMES_DIR/config.yaml (edit it by hand)"
  quiet_ac remove-codex "$CODEX_DIR/config.toml" || left_behind "SpeakEasy's MCP entry in $CODEX_DIR/config.toml (edit it by hand)"
  quiet_ac remove-vibe "$VIBE_DIR/config.toml" || left_behind "SpeakEasy's MCP entry in $VIBE_DIR/config.toml (edit it by hand)"
}

# --------------------------------------------------------------------------- #
hr
if [ "$UNINSTALL" = 1 ]; then say "SpeakEasy — uninstall"; else say "SpeakEasy — install"; fi
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
  for f in "$HOME_DIR/.claude/CLAUDE.md" "$CODEX_DIR/AGENTS.md" \
           "$HOME_DIR/.config/opencode/AGENTS.md" "$VIBE_DIR/AGENTS.md" \
           "$HOME_DIR/.config/goose/.goosehints" "$HOME_DIR/.pi/agent/AGENTS.md" \
           "$HOME_DIR/.paradev/agent/AGENTS.md" "$HOME_DIR/.gemini/GEMINI.md" \
           "$HOME_DIR/.copilot/copilot-instructions.md" "$HERMES_DIR/SOUL.md" \
           "$HERMES_DIR/AGENTS.md" "$HOME_DIR/.config/zed/AGENTS.md"; do
    quiet_ac instruction-remove "$f" || left_behind "SpeakEasy's instruction block in $f (remove it by hand)"
  done
  remove_legacy_zed_rules
  run rm -f "$INSTALL_BIN"
  if [ "$PURGE" = 1 ]; then
    run rm -rf "$STATE_DIR"
  else
    say "Kept state dir $STATE_DIR (cooldowns, sent records, any credential files)."
    say "Delete it manually with: rm -rf $STATE_DIR"
  fi
  say "Restart running agents: a session started before the uninstall keeps"
  say "its hooks, which now block Slack calls because SpeakEasy is gone."
  if [ -n "$LEFT_BEHIND" ]; then
    say ""
    say "Uninstalled, except for these, which you need to remove by hand:"
    printf '%s' "$LEFT_BEHIND"
    if [ "$LEFT_HOOK" = 1 ]; then
      say "A leftover approval hook blocks that agent's Slack calls until you"
      say "remove it (it fails closed, and SpeakEasy is gone)."
    fi
    say "Everything else SpeakEasy added was removed; your own entries and"
    say "settings stay."
    exit 1
  fi
  say "Uninstalled. SpeakEasy's own entries, approval hooks and instruction"
  say "blocks were removed from your agent configs; your own entries and settings"
  say "stay (JSON files are rewritten with standard formatting). Any .bak-sag"
  say "files next to them are backups taken before SpeakEasy first changed"
  say "them, for a manual restore."
  exit 0
fi

# --------------------------------------------------------------------------- #
# credentials and take-over candidates (read-only; used from step 3 on)
# --------------------------------------------------------------------------- #
# Configs a Slack MCP server can be taken over from (step 3).
TAKEOVER_FILES=(
  "$HOME_DIR/.claude.json"
  "$CODEX_DIR/config.toml"
  "$HOME_DIR/.config/opencode/opencode.json"
  "$VIBE_DIR/config.toml"
  "$HOME_DIR/.config/goose/config.yaml"
  "$HOME_DIR/.pi/agent/settings.json"
  "$HOME_DIR/.pi/agent/mcp.json"
  "$HOME_DIR/.paradev/agent/settings.json"
  "$HOME_DIR/.paradev/agent/mcp.json"
  "$HOME_DIR/.gemini/settings.json"
  "$HOME_DIR/.copilot/mcp-config.json"
  "$HOME_DIR/.config/zed/settings.json"
  "$HOME_DIR/.config/amp/settings.json"
  "$HERMES_DIR/config.yaml"
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
flag_transport() { # the transport a credential flag given on this run implies
  if [ -n "$MCP_TOKEN_ARG" ]; then echo slack-mcp
  elif [ -n "$MISTRAL_KEY_ARG" ]; then echo mistral-connector
  elif [ -n "$BOT_TOKEN_ARG$USER_TOKEN_ARG" ]; then echo korotovsky
  fi
}
credential_transport() { # the transport of a credential given on this run (a
  # flag outranks an ambient $SLACK_MCP_TOKEN), or stored after slack_transport
  # was written (scripts/slack_mcp_oauth.py writes slack_mcp_token) — an older
  # stored credential does not override an explicit hooks-only choice
  local f first
  f="$(flag_transport)"
  if [ -n "$f" ]; then echo "$f"; return; fi
  if [ -n "${SLACK_MCP_TOKEN:-}" ]; then echo slack-mcp; return; fi
  for f in slack_mcp_token:slack-mcp mistral_api_key:mistral-connector \
           slack_bot_token:korotovsky slack_user_token:korotovsky slack_stdio_env:korotovsky; do
    [ -s "$STATE_DIR/${f%%:*}" ] || continue
    [ "$STATE_DIR/${f%%:*}" -nt "$STATE_DIR/slack_transport" ] || continue
    if [ "${f%%:*}" = slack_stdio_env ]; then
      stdio_env_credential && { echo korotovsky; return; }
      continue
    fi
    first="$(head -n 1 "$STATE_DIR/${f%%:*}")"
    case "$first" in
      xoxb-placeholder-*|xoxp-placeholder-*|mistral-placeholder-*) ;;
      *) echo "${f#*:}"; return ;;
    esac
  done
}
placeholder() { # placeholder <file>: it holds the placeholder the installer writes
  case "$(head -n 1 "$1" 2>/dev/null || true)" in
    xoxb-placeholder-*|xoxp-placeholder-*|mistral-placeholder-*) return 0 ;;
  esac
  return 1
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
# With no credential anywhere (no flag, no stored file, nothing taken over)
# the default is hooks-only: agents keep their own Slack tools and the hooks
# ask first. A credential flag or $SLACK_MCP_TOKEN keeps the old default.
CREDENTIAL_INPUT=0
if [ -n "$MCP_TOKEN_ARG$MISTRAL_KEY_ARG$BOT_TOKEN_ARG$USER_TOKEN_ARG" ] \
   || [ -n "${SLACK_MCP_TOKEN:-}" ]; then
  CREDENTIAL_INPUT=1
fi
takeover_count() { # distinct Slack MCP servers an automatic take-over could pick
  local files=() f
  for f in "${TAKEOVER_FILES[@]}"; do [ -f "$f" ] && files+=("$f"); done
  if [ "${#files[@]}" = 0 ]; then echo 0; return; fi
  "$PY" "$AGENT_CONFIG" takeover-candidates "${files[@]}" 2>/dev/null | "$PY" -c 'import json, sys
c = [x for x in json.load(sys.stdin) if x.get("importable")]
print(len({x.get("fingerprint") or str(i) for i, x in enumerate(c)}))' 2>/dev/null || echo 0
}
# --no-hooks leaves SpeakEasy only a Slack login of its own. Refuse before
# changing anything when it will have none: no credential given, none stored
# (or a stored one a hooks-only setup doesn't use), and no single server it
# would take over.
if [ "$NO_HOOKS" = 1 ] && [ -z "$TRANSPORT_ARG" ] && [ -z "$TAKE_OVER_ARG" ]; then
  SAVED_TRANSPORT="$(cat "$STATE_DIR/slack_transport" 2>/dev/null || true)"
  WILL_TAKE_OVER=0
  if [ "$NO_TAKE_OVER" = 0 ] && [ -z "$MCP_TOKEN_ARG$MISTRAL_KEY_ARG$BOT_TOKEN_ARG$USER_TOKEN_ARG" ] \
     && ! stored_credential && [ "$(takeover_count)" = 1 ]; then
    WILL_TAKE_OVER=1
  fi
  REFUSE=""
  if [ "$WILL_TAKE_OVER" = 0 ]; then
    if [ "$SAVED_TRANSPORT" = hooks-only ] && [ -z "$(credential_transport)" ]; then
      REFUSE=hooks-only
    elif [ "$SAVED_TRANSPORT" != hooks-only ] && [ "$CREDENTIAL_INPUT" = 0 ] && ! stored_credential; then
      REFUSE=none
    fi
  fi
  if [ -n "$REFUSE" ]; then
    if [ "$REFUSE" = hooks-only ]; then
      say "SpeakEasy is set up hooks-only ($STATE_DIR/slack_transport): with" >&2
      say "--no-hooks it could neither send nor ask about your agents' sends." >&2
      if stored_credential; then
        say "A credential is stored in $STATE_DIR but this setup doesn't use it:" >&2
        say "add --transport slack-mcp, mistral-connector or korotovsky (matching it)." >&2
      fi
    else
      say "With --no-hooks SpeakEasy needs a Slack login of its own (it could neither" >&2
      say "send nor ask about your agents' sends), and it has none." >&2
    fi
    if [ "$NO_TAKE_OVER" = 0 ] && [ "$(takeover_count)" -gt 1 ]; then
      say "Several Slack MCP servers could be taken over; pick one with" >&2
      say "--take-over FILE:NAME (run without --no-hooks to see them listed)." >&2
    fi
    say "Give it a credential (or --transport / --take-over), or drop --no-hooks." >&2
    say "Nothing was changed." >&2
    exit 2
  fi
fi

# --------------------------------------------------------------------------- #
# 1. build
# --------------------------------------------------------------------------- #
hr
say "1. Build"
hr
# A release download carries the prebuilt binary in bin/ and no sources:
# it is installed as is, never built.
PREBUILT="$ROOT/bin/$BIN_NAME"
RELEASE=0
if [ -x "$PREBUILT" ] && [ ! -f "$ROOT/Cargo.toml" ]; then
  RELEASE=1
fi
# cargo is only required when a build actually runs: not for --uninstall
# (which exits above), not for --skip-build or a release download, not for
# --dry-run.
if [ "$RELEASE" = 0 ] && [ "$SKIP_BUILD" = 0 ] && [ "$DRY_RUN" = 0 ]; then
  if ! command -v cargo >/dev/null 2>&1; then
    say "missing required tool: cargo (needed to build; or pass --skip-build" >&2
    say "to reuse the existing release binary)" >&2
    exit 1
  fi
fi
if [ "$RELEASE" = 1 ]; then
  say "  using this release's prebuilt binary: $PREBUILT"
elif [ "$SKIP_BUILD" = 1 ]; then
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
if [ "$RELEASE" = 1 ]; then BINARY="$PREBUILT"; fi
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
# A browser download is quarantined, and Gatekeeper would stop the login
# service (and every hook) from running this unsigned binary: you chose to
# install it, so the installed copy is not quarantined.
if [ "$DRY_RUN" = 0 ]; then
  xattr -d com.apple.quarantine "$INSTALL_BIN" 2>/dev/null || true
fi
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
  TAKEOVER_PICK="$("$PY" - "$CANDIDATES_JSON" "$TAKE_OVER_ARG" "$TRANSPORT_ARG" "$AUTO" "$SELF" <<'EOF'
import json, os, sys
cands, want, transport, auto = json.loads(sys.argv[1]), sys.argv[2], sys.argv[3], sys.argv[4] == "1"
self_cmd = sys.argv[5]
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
        print("error\t--take-over %s: %s" % (want, "; ".join(
            "server %s: %s" % (c["name"], c["reason"]) for c in match)))
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
        # nothing after the command: a trailing "# comment" breaks pasting
        # into zsh, where interactive comments are off by default
        print("    %s (server %s) in %s:" % (c["summary"], c["name"], short(c["file"])))
        print("      %s --take-over '%s:%s'" % (self_cmd, short(c["file"]), c["name"]))
else:
    print("none")
    if auto:
        for c in cands:
            if not c["importable"]:
                print("    %s (server %s) in %s: %s" % (c["summary"], c["name"], short(c["file"]), c["reason"]))
EOF
)"
  TAKEOVER_HEAD="${TAKEOVER_PICK%%$'\n'*}"
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
      ;;
    none)
      if [ "$(printf '%s\n' "$TAKEOVER_PICK" | wc -l)" -gt 1 ]; then
        say "  Found Slack MCP servers SpeakEasy cannot take over:"
        printf '%s\n' "$TAKEOVER_PICK" | tail -n +2
        say "  The agents keep using them. In agents with hooks, SpeakEasy's approval"
        say "  hooks (step 5b) make each Slack write wait for you (Claude Desktop and"
        say "  Zed have none; see step 7). To also give SpeakEasy a login of its own,"
        say "  see README.md, \"If there's nothing to take over\"."
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
        say "  transport: $TRANSPORT (was hooks-only; you gave SpeakEasy a $TRANSPORT"
        say "  credential, so request_slack_message is registered again)"
      else
        TRANSPORT="hooks-only"
        say "  transport: hooks-only (kept from the previous install, $TRANSPORT_FILE)"
      fi
      ;;
    slack-mcp|mistral-connector|korotovsky)
      FLAG_TRANSPORT="$(flag_transport)"
      if [ -n "$FLAG_TRANSPORT" ] && [ "$FLAG_TRANSPORT" != "$EXISTING_TRANSPORT" ]; then
        TRANSPORT="$FLAG_TRANSPORT"
        if [ "$DRY_RUN" = 0 ]; then
          printf '%s\n' "$TRANSPORT" > "$TRANSPORT_FILE"
        else
          say "  [dry-run] write $TRANSPORT_FILE ($TRANSPORT)"
        fi
        say "  transport: $TRANSPORT (was $EXISTING_TRANSPORT; you gave a $TRANSPORT credential)"
      else
        TRANSPORT="$EXISTING_TRANSPORT"
        say "  transport: $EXISTING_TRANSPORT (kept from the previous install, $TRANSPORT_FILE)"
      fi
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
  if [ "${TAKEOVER_HEAD:-}" = "many" ]; then
    say "  transport: hooks-only for now (SpeakEasy found several Slack MCP servers"
    say "  it could take over — pick one with a --take-over command above; until"
    say "  then your agents keep sending with their own Slack tools, and in agents"
    say "  with hooks the approval hooks ask you first — see steps 5b and 7)"
  else
    say "  transport: hooks-only (SpeakEasy has no Slack login and found none it"
    say "  could take over; your agents keep sending with their own Slack tools,"
    say "  and in agents with hooks the approval hooks ask you first — see steps"
    say "  5b and 7)"
  fi
else
  CRED_TRANSPORT="$(credential_transport)"
  TRANSPORT="${CRED_TRANSPORT:-slack-mcp}"
  if [ "$DRY_RUN" = 0 ]; then
    printf '%s\n' "$TRANSPORT" > "$TRANSPORT_FILE"
  else
    say "  [dry-run] write $TRANSPORT_FILE ($TRANSPORT)"
  fi
  if [ -n "$CRED_TRANSPORT" ]; then
    say "  transport: $TRANSPORT (from your credential; written to $TRANSPORT_FILE)"
  else
    say "  transport: slack-mcp (default; written to $TRANSPORT_FILE)"
  fi
fi
if [ "$TRANSPORT" = "hooks-only" ] && [ "$NO_HOOKS" = 1 ]; then
  say "SpeakEasy is set up hooks-only ($TRANSPORT_FILE): with --no-hooks it could" >&2
  say "neither send nor ask about your agents' sends. Give it a credential (or" >&2
  say "--transport) first, or drop --no-hooks." >&2
  exit 2
fi

WROTE=wrote
if [ "$DRY_RUN" = 1 ]; then WROTE="would write"; fi

# --- workspace domain (message permalinks; the korotovsky server's replies
# carry no permalink, so the gateway builds links from this domain)
WORKSPACE_DOMAIN_FILE="$STATE_DIR/slack_workspace_domain"
if [ -n "$WORKSPACE_DOMAIN_ARG" ]; then
  if [ "$DRY_RUN" = 0 ]; then
    printf '%s\n' "$WORKSPACE_DOMAIN_ARG" > "$WORKSPACE_DOMAIN_FILE"
  else
    say "  [dry-run] write $WORKSPACE_DOMAIN_FILE ($WORKSPACE_DOMAIN_ARG)"
  fi
  say "  $WROTE $WORKSPACE_DOMAIN_FILE — message permalinks point at $WORKSPACE_DOMAIN_ARG.slack.com"
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
  elif [ ! -f "$MISTRAL_KEY_FILE" ] || placeholder "$MISTRAL_KEY_FILE"; then
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
    say "  $WROTE $MISTRAL_KEY_FILE (chmod 600) — the only place the Mistral API key lives"
    if [ -z "$MISTRAL_KEY_ARG" ]; then say "  (the key came from \$MISTRAL_API_KEY in your environment)"; fi
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
    say "    $SELF --transport mistral-connector --mistral-api-key YOUR_KEY"
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
  elif [ ! -f "$MCP_TOKEN_FILE" ] || placeholder "$MCP_TOKEN_FILE"; then
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
    say "  $WROTE $MCP_TOKEN_FILE (chmod 600) — the only place the Slack user token lives"
    if [ -z "$MCP_TOKEN_ARG" ]; then say "  (the token came from \$SLACK_MCP_TOKEN in your environment)"; fi
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
    say "    $OAUTH --client-id YOUR_SLACK_APP_CLIENT_ID \\"
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
  elif { [ ! -f "$BOT_TOKEN_FILE" ] || placeholder "$BOT_TOKEN_FILE"; } && [ "$STDIO_LOGIN" = 0 ]; then
    BOT_TOKEN="${SLACK_BOT_TOKEN:-}"
  else
    BOT_TOKEN=""
  fi
  if [ -n "$USER_TOKEN_ARG" ]; then
    USER_TOKEN="$USER_TOKEN_ARG"
  elif { [ ! -f "$USER_TOKEN_FILE" ] || placeholder "$USER_TOKEN_FILE"; } && [ "$STDIO_LOGIN" = 0 ]; then
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
    say "  $WROTE $BOT_TOKEN_FILE (chmod 600) — the only place the Slack bot token lives"
    if [ -z "$BOT_TOKEN_ARG" ]; then say "  (the token came from \$SLACK_BOT_TOKEN in your environment)"; fi
  elif [ -f "$BOT_TOKEN_FILE" ] \
       && ! { [ "$STDIO_LOGIN" = 1 ] && grep -q '^xoxb-placeholder-' "$BOT_TOKEN_FILE"; }; then
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
    say "  $WROTE $USER_TOKEN_FILE (chmod 600) — the only place the Slack user token lives"
    if [ -z "$USER_TOKEN_ARG" ]; then say "  (the token came from \$SLACK_USER_TOKEN in your environment)"; fi
  elif [ -f "$USER_TOKEN_FILE" ] \
       && ! { [ "$STDIO_LOGIN" = 1 ] && grep -q '^xoxp-placeholder-' "$USER_TOKEN_FILE"; }; then
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
    say "        $SELF --take-over FILE[:NAME]"
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
    # (its arguments can hold a token: show the program only)
    say "  keeping existing $STDIO_COMMAND_FILE ($(awk 'NR == 1 { print $1; exit }' "$STDIO_COMMAND_FILE") …)"
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
socket_answers() { # the instance answers MCP on its socket. A bare connect
  # is not enough: the socket is bound before the Slack client is built (a
  # taken-over server's handshake can still fail and stop SpeakEasy)
  [ -S "$STATE_DIR/gateway.sock" ] && "$PY" -c 'import socket, sys
s = socket.socket(socket.AF_UNIX)
s.settimeout(3)
s.connect(sys.argv[1])
s.sendall(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-06-18\",\"capabilities\":{},\"clientInfo\":{\"name\":\"install-check\",\"version\":\"0\"}}}\n")
reply = b""
while not reply.endswith(b"\n"):
    chunk = s.recv(65536)
    if not chunk:
        break
    reply += chunk
sys.exit(0 if b"slack-approval-gateway" in reply else 1)' "$STATE_DIR/gateway.sock" 2>/dev/null
}
if [ "$DRY_RUN" = 0 ]; then
  for _ in $(seq 1 30); do
    if socket_answers; then break; fi
    sleep 1
  done
fi
SERVICE_UP=1
if [ "$DRY_RUN" = 0 ] && socket_answers; then
  # (|| true: under pipefail a service launchd doesn't know would end the run)
  PID="$(launchctl print "gui/$(id -u)/$PLIST_LABEL" 2>/dev/null | sed -n 's/.*pid = \([0-9]*\).*/\1/p' | head -1 || true)"
  say "  service running (pid ${PID:-?}); MCP socket: $STATE_DIR/gateway.sock"
elif [ "$DRY_RUN" = 1 ]; then
  say "  [dry-run] (the service would start now)"
else
  SERVICE_UP=0
  say "  WARNING: SpeakEasy is not running (it may have stopped at startup); check:"
  say "    launchctl print gui/$(id -u)/$PLIST_LABEL   and   $STATE_DIR/gateway.log"
  LOG_EXCERPT=""
  if [ -f "$STATE_DIR/gateway.log" ]; then
    # SpeakEasy's last error and what follows it (e.g. the Slack server's
    # stderr, whose own lines may say "error" too); without one, the last
    # lines that mention an error, or the last lines
    LOG_EXCERPT="$(awk '{ l[NR] = $0 } /^slack-approval-gateway: / { n = NR } /[Ee]rror/ { e = NR }
         END { s = n ? n : e ? e : (NR > 3 ? NR - 2 : 1); for (i = s; i <= NR && i < s + 8; i++) print l[i] }' \
      "$STATE_DIR/gateway.log" 2>/dev/null || true)"
  fi
  if [ -n "$LOG_EXCERPT" ]; then
    printf '%s\n' "$LOG_EXCERPT" | sed 's/^/      | /'
    LOG_WHY="the log lines above say why"
  else
    say "      (gateway.log is empty: launchd may not have started it at all)"
    LOG_WHY="launchctl print (above) may say why"
  fi
  if [ "$NO_HOOKS" = 0 ]; then
    say "  Until it runs, the approval hooks block your agents' own Slack writes"
    say "  (nothing is sent without your approval)."
  fi
  case "$TRANSPORT" in
    hooks-only)
      say "  SpeakEasy needs no credential in this setup; once the log is clear,"
      say "  start it: launchctl kickstart -k gui/$(id -u)/$PLIST_LABEL" ;;
    korotovsky)
      say "  SpeakEasy starts the Slack MCP server (the command in"
      say "  $STATE_DIR/slack_stdio_command, or slack-mcp-server"
      say "  on your PATH) and stops when it fails; $LOG_WHY."
      say "  Check that the server is installed and runs, and that its login"
      say "  works: a token (--slack-bot-token / --slack-user-token), or the one in"
      say "  $STATE_DIR/slack_stdio_env (a browser-session login expires;"
      say "  if you took it over, sign in again there and re-run --take-over)." ;;
    mistral-connector)
      say "  The Mistral connector transport needs a valid Mistral API key"
      say "  ($SELF --mistral-api-key …) and your Slack workspace"
      say "  connected to the Slack connector in the Mistral console." ;;
    *)
      if [ -s "$STATE_DIR/slack_mcp_token" ] \
         && ! grep -q '^xoxp-placeholder-' "$STATE_DIR/slack_mcp_token"; then
        say "  SpeakEasy has a Slack user token ($STATE_DIR/slack_mcp_token);"
        say "  $LOG_WHY it failed (a revoked or expired token,"
        say "  or no network). If you took it over from an agent, sign in again"
        say "  there and re-run with --take-over FILE:NAME; or get a new token with"
        say "  $OAUTH or --slack-mcp-token xoxp-…"
      else
        say "  The Slack MCP transport needs a Slack user token: run"
        say "  $OAUTH, or $SELF --slack-mcp-token xoxp-…"
      fi ;;
  esac
  if [ "$TRANSPORT" != "hooks-only" ] && [ "$NO_HOOKS" = 0 ]; then
    say "  Or run the installer with --transport hooks-only (no Slack login needed)."
  fi
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
  CLAUDE_ENTRY="$("$PY" -c 'import json, sys; sys.path.insert(0, sys.argv[2]); import agent_config as a
print(json.dumps(a.JSON_STYLES["claude"][1](sys.argv[1])))' "$INSTALL_BIN" "$ROOT/scripts")"
  if has "$CFG"; then
    note_registered_existing claude "$CFG"
  else
    backup_once "$CFG"
    if [ "$DRY_RUN" = 0 ]; then
      # add-json: `claude mcp add` cannot set the tool-call timeout
      if ! claude mcp add-json --scope user "$SERVER_NAME" "$CLAUDE_ENTRY" >/dev/null 2>&1; then
        say "  (claude mcp add-json failed; editing .claude.json directly)"
        try_ac claude "$CFG" add-json claude "$CFG" "$INSTALL_BIN" || REG_FAILED=1
      fi
    else
      say "  [dry-run] claude mcp add-json --scope user $SERVER_NAME '$CLAUDE_ENTRY'"
    fi
    [ "${REG_FAILED:-0}" = 1 ] || note_registered claude "$CFG"
    REG_FAILED=0
  fi
  if has "$CFG"; then upgrade_entry claude claude "$CFG"; fi
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
    if try_ac pdev "$CFG" add-json pimcp "$CFG" "$INSTALL_BIN"; then
      note_registered pdev "$CFG"
    fi
  fi
  if has "$CFG"; then upgrade_entry pdev pimcp "$CFG"; fi
else
  note_skipped "pdev (not installed)"
fi

# --- Codex
if command -v codex >/dev/null 2>&1; then
  CFG="$CODEX_DIR/config.toml"
  if has "$CFG"; then
    note_registered_existing codex "$CFG"
  else
    backup_once "$CFG"
    if [ "$DRY_RUN" = 0 ]; then
      if ! codex mcp add "$SERVER_NAME" -- "$INSTALL_BIN" >/dev/null 2>&1; then
        say "  (codex mcp add failed; appending to config.toml)"
        try_ac codex "$CFG" add-codex "$CFG" "$INSTALL_BIN" || REG_FAILED=1
      fi
    else
      say "  [dry-run] codex mcp add $SERVER_NAME -- $INSTALL_BIN"
    fi
    [ "${REG_FAILED:-0}" = 1 ] || note_registered codex "$CFG"
    REG_FAILED=0
  fi
  # `codex mcp add` has no timeout flag; a request_slack_message call
  # blocks on a human approval for minutes (codex's default tool timeout
  # is 60s), so give the entry a long tool_timeout_sec
  long_timeout codex "$CFG"
else
  note_skipped "codex (not installed)"
fi

# --- OpenCode
if command -v opencode >/dev/null 2>&1 || [ -f "$HOME_DIR/.config/opencode/opencode.json" ]; then
  CFG="$HOME_DIR/.config/opencode/opencode.json"
  if has "$CFG"; then
    note_registered_existing opencode "$CFG"
  else
    if try_ac opencode "$CFG" add-json opencode "$CFG" "$INSTALL_BIN"; then
      note_registered opencode "$CFG"
    fi
  fi
  if has "$CFG"; then upgrade_entry opencode opencode "$CFG"; fi
else
  note_skipped "opencode (not installed)"
fi

# --- Mistral Vibe
if command -v vibe >/dev/null 2>&1; then
  CFG="$VIBE_DIR/config.toml"
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
        try_ac vibe "$CFG" add-vibe "$CFG" "$INSTALL_BIN" || REG_FAILED=1
      fi
    else
      say "  [dry-run] vibe mcp add $SERVER_NAME --transport stdio --command $INSTALL_BIN --tool-timeout-sec 86400"
    fi
    [ "${REG_FAILED:-0}" = 1 ] || note_registered vibe "$CFG"
    REG_FAILED=0
  fi
  # make sure the entry has a long tool_timeout_sec whether the CLI wrote
  # it or an older vibe without --tool-timeout-sec did
  long_timeout vibe "$CFG"
else
  note_skipped "vibe (not installed)"
fi

# --- goose
if command -v goose >/dev/null 2>&1 || [ -f "$HOME_DIR/.config/goose/config.yaml" ]; then
  CFG="$HOME_DIR/.config/goose/config.yaml"
  if has "$CFG"; then
    note_registered_existing goose "$CFG"
  else
    if try_ac goose "$CFG" add-goose "$CFG" "$INSTALL_BIN"; then
      note_registered goose "$CFG"
    fi
  fi
  # a request_slack_message call blocks on a human approval for minutes
  # (goose cancels the call when its timeout expires; older installs wrote
  # timeout: 120), so set a long timeout on every run
  long_timeout goose "$CFG"
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
    if try_ac pi "$CFG" add-json pimcp "$CFG" "$INSTALL_BIN"; then
      note_registered pi "$CFG"
    fi
  fi
  if has "$CFG"; then upgrade_entry pi pimcp "$CFG"; fi
else
  note_skipped "pi (not installed)"
fi

# --- Gemini CLI (config is honored even when the CLI is not on PATH yet)
if [ -d "$HOME_DIR/.gemini" ]; then
  CFG="$HOME_DIR/.gemini/settings.json"
  if has "$CFG"; then
    note_registered_existing gemini "$CFG"
  else
    if try_ac gemini "$CFG" add-json gemini "$CFG" "$INSTALL_BIN"; then
      if command -v gemini >/dev/null 2>&1 && gemini --version >/dev/null 2>&1; then
        note_registered gemini "$CFG"
      else
        say "  [ok]        gemini (config written; CLI not currently on PATH) — $CFG"
        printf '%s\t%s\n' gemini "$CFG" >> "$REGISTERED_FILE"
      fi
    fi
  fi
  if has "$CFG"; then upgrade_entry gemini gemini "$CFG"; fi
else
  note_skipped "gemini (not installed)"
fi

# --- GitHub Copilot CLI
if [ -d "$HOME_DIR/.copilot" ]; then
  CFG="$HOME_DIR/.copilot/mcp-config.json"
  if has "$CFG"; then
    note_registered_existing copilot "$CFG"
  else
    if try_ac copilot "$CFG" add-json copilot "$CFG" "$INSTALL_BIN"; then
      note_registered copilot "$CFG"
    fi
  fi
  if has "$CFG"; then upgrade_entry copilot copilot "$CFG"; fi
else
  note_skipped "copilot (not installed)"
fi

# --- Zed
if command -v zed >/dev/null 2>&1 || [ -d "$HOME_DIR/.config/zed" ]; then
  CFG="$HOME_DIR/.config/zed/settings.json"
  if has "$CFG"; then
    note_registered_existing zed "$CFG"
  else
    if try_ac zed "$CFG" add-zed "$CFG" "$INSTALL_BIN"; then
      note_registered zed "$CFG"
    fi
  fi
  if has "$CFG"; then upgrade_entry zed zed "$CFG"; fi
  say "              (Zed ends an MCP tool call after 10 minutes, whatever the"
  say "               settings; SpeakEasy withdraws a Zed request just before)"
else
  note_skipped "zed (not installed)"
fi

# --- Amp
if command -v amp >/dev/null 2>&1 || [ -f "$HOME_DIR/.config/amp/settings.json" ]; then
  CFG="$HOME_DIR/.config/amp/settings.json"
  if has "$CFG"; then
    note_registered_existing amp "$CFG"
  else
    if try_ac amp "$CFG" add-json amp "$CFG" "$INSTALL_BIN"; then
      note_registered amp "$CFG"
    fi
  fi
  say "              (Amp ends an MCP tool call after 5 minutes, whatever the"
  say "               settings; SpeakEasy then withdraws the request)"
else
  note_skipped "amp (not installed)"
fi

# --- Hermes Agent
if command -v hermes >/dev/null 2>&1 || [ -d "$HERMES_DIR" ]; then
  CFG="$HERMES_DIR/config.yaml"
  if has "$CFG"; then
    note_registered_existing hermes "$CFG"
  else
    if try_ac hermes "$CFG" add-hermes "$CFG" "$INSTALL_BIN"; then
      note_registered hermes "$CFG"
    fi
  fi
  long_timeout hermes "$CFG"
  # Hermes before 0.21.6 fails every stdio MCP tool call (an inverted
  # liveness check); before 0.22.3 its tool executor cuts a call off after
  # 7 minutes whatever the server's timeout. A script install records its
  # version in hermes-agent/install-stamp.json (baseVersion; pyproject.toml
  # says 0.0.0 since 0.21.6); Hermes installed some other way gets no note.
  HERMES_AGE="$("$PY" - "$HERMES_DIR/hermes-agent" <<'EOF' 2>/dev/null || true
import json, os, re, sys
d = sys.argv[1]
v = ""
try:
    v = json.load(open(os.path.join(d, "install-stamp.json"))).get("baseVersion") or ""
except (OSError, ValueError, AttributeError):
    pass
if not v:
    try:
        m = re.search(r'^version\s*=\s*"([0-9.]+)"', open(os.path.join(d, "pyproject.toml")).read(), re.M)
        v = m.group(1) if m else ""
    except OSError:
        pass
t = tuple(int(x) for x in str(v).split(".")[:3] if x.isdigit())
if t and t != (0, 0, 0):
    print("broken" if t < (0, 21, 6) else "short" if t < (0, 22, 3) else "ok", v)
EOF
)"
  HERMES_VERSION="${HERMES_AGE#* }"
  case "$HERMES_AGE" in
    broken\ *)
      say "  note: Hermes $HERMES_VERSION fails every stdio MCP tool call, so it can't"
      say "        use request_slack_message. Update it: hermes update" ;;
    short\ *)
      say "  note: Hermes $HERMES_VERSION cuts a tool call off after 7 minutes; for"
      say "        longer approvals, update it: hermes update" ;;
  esac
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
if [ "$TRANSPORT" = "hooks-only" ]; then
say "  Superset workspaces run these same agent CLIs as this user, so they"
say "  pick up the approval hooks (5b) automatically — no per-workspace setup."
else
say "  Superset workspaces run these same agent CLIs as this user, so they"
say "  pick up the registrations above automatically — no per-workspace setup."
fi

# --------------------------------------------------------------------------- #
# 5b. approval hooks: the agents' own Slack tools ask SpeakEasy first
# --------------------------------------------------------------------------- #
hr
say "5b. Approval hooks for the agents' own Slack tools"
hr
# "agent<TAB>file" rows of the hooks registered here, verified at the end
HOOKED_FILE="$(mktemp 2>/dev/null || echo /tmp/sag-install-hooked.$$)"
HOOK_FAILED_FILE="$(mktemp 2>/dev/null || echo /tmp/sag-install-hookfail.$$)"
trap 'rm -f "$REGISTERED_FILE" "$HOOKED_FILE" "$HOOK_FAILED_FILE" "$OUTDATED_FILE" "$REG_FAILED_FILE"' EXIT
hook_detected() { # hook_detected <agent> — is the agent installed here?
  case "$1" in
    claude) [ -d "$HOME_DIR/.claude" ] || command -v claude >/dev/null 2>&1 \
              || [ -d "$HOME_DIR/.paradev" ] ;;
    codex) [ -d "$CODEX_DIR" ] || command -v codex >/dev/null 2>&1 ;;
    gemini) [ -d "$HOME_DIR/.gemini" ] ;;
    copilot) [ -d "$HOME_DIR/.copilot" ] ;;
    cursor) [ -d "$HOME_DIR/.cursor" ] || command -v cursor-agent >/dev/null 2>&1 ;;
    hermes) [ -f "$HERMES_DIR/config.yaml" ] || command -v hermes >/dev/null 2>&1 ;;
    amp) [ -d "$HOME_DIR/.config/amp" ] || command -v amp >/dev/null 2>&1 ;;
    vibe) [ -d "$VIBE_DIR" ] || command -v vibe >/dev/null 2>&1 ;;
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
    if HOOK_OUT="$(ac hook-add "$agent" "$file" "$INSTALL_BIN" "$HOOK_WAIT" "$STATE_DIR" 2>&1)"; then
      if [ "$DRY_RUN" = 1 ]; then printf '%s\n' "$HOOK_OUT"; fi
      if printf '%s\n' "$HOOK_OUT" | grep -q 'the hook will not run'; then
        # a setting in the agent switches all its hooks off
        say "  WARNING:   $agent hook added, but it will not run — $file:"
        printf '%s\n' "$HOOK_OUT" | grep 'the hook will not run' | sed 's/^warning: /              /'
        say "              Turn hooks back on there, or $agent's Slack writes are not gated."
        printf '%s\t%s\t%s\n' "$agent" "$file" "its hooks are switched off" >> "$HOOK_FAILED_FILE"
        continue
      fi
      printf '%s\t%s\n' "$agent" "$file" >> "$HOOKED_FILE"
      say "  [ok]        $agent hook — $file"
    else
      printf '%s\n' "$HOOK_OUT" | sed 's/^/  /'
      say "  [failed]    $agent hook — $file (see above)"
      printf '%s\t%s\t%s\n' "$agent" "$file" "its hook was not added" >> "$HOOK_FAILED_FILE"
    fi
  done < <(hook_targets)
  if [ -d "$HOME_DIR/.paradev" ]; then
    say "  (pdev runs the Claude Code hooks in ~/.claude/settings.json too)"
  fi
  if [ -d "$HOME_DIR/.cursor" ] || hooked cursor; then
    say "  (Cursor also imports Claude Code's hooks; SpeakEasy shows one window"
    say "   per tool call either way)"
  fi
  if [ -d "$HOME_DIR/.config/zed" ]; then
    if [ "$TRANSPORT" = "hooks-only" ]; then
      note_skipped "zed (its built-in agent has no hooks; Zed asks before each MCP"
      say "              tool call unless you chose Always allow for it)"
    elif reg_failed zed; then
      note_skipped "zed (its built-in agent has no hooks, and its registration failed:"
      say "              see step 5)"
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
# 6. instruction blocks (how agents send to Slack)
# --------------------------------------------------------------------------- #
hr
say "6. Agent instruction blocks"
hr
# Which block each agent gets. When SpeakEasy can send: "use your Slack tool
# or request_slack_message, the user approves either" where the hook was
# registered, "use request_slack_message" where it wasn't. In the hooks-only
# setup: "use your own Slack tool, the user approves each call" where the
# hook was registered, and "ask the user in chat first" where it wasn't
# (Zed's built-in agent has no hooks).
hooked() { # hooked <agent> — the approval hook was registered for it
  [ -s "$HOOKED_FILE" ] && awk -F '\t' -v a="$1" '$1 == a { f = 1 } END { exit !f }' "$HOOKED_FILE"
}
add_instruction() { # add_instruction <file> <hook agent, or "none"> [<registered agent>]
  # (an agent whose registration failed has no request_slack_message either)
  local variant="gateway"
  if [ "$TRANSPORT" = "hooks-only" ] || reg_failed "${3:-$2}"; then
    if [ "$2" != none ] && hooked "$2"; then variant="hooks"; else variant="confirm"; fi
  elif [ "$2" != none ] && hooked "$2"; then
    # its own Slack tools ask first too: either route is fine
    variant="either"
  fi
  local result
  if result="$(ac instruction-add "$1" "$variant" 2>&1)"; then
    if [ "$DRY_RUN" = 1 ]; then say "$result"; return; fi
    say "  [ok]        instruction block (${result##*: }) — $1"
  else
    printf '%s\n' "$result" | sed 's/^/  /'
    say "  [failed]    instruction block — $1"
  fi
}
if [ -f "$HOME_DIR/.claude.json" ] || [ -d "$HOME_DIR/.claude" ]; then
  add_instruction "$HOME_DIR/.claude/CLAUDE.md" claude
fi
if [ -f "$CODEX_DIR/config.toml" ] || hooked codex; then
  add_instruction "$CODEX_DIR/AGENTS.md" codex
fi
if [ -f "$HOME_DIR/.config/opencode/opencode.json" ] || hooked opencode; then
  add_instruction "$HOME_DIR/.config/opencode/AGENTS.md" opencode
fi
if [ -f "$VIBE_DIR/config.toml" ] || hooked vibe; then
  add_instruction "$VIBE_DIR/AGENTS.md" vibe
fi
if [ -f "$HOME_DIR/.config/goose/config.yaml" ] || hooked goose; then
  add_instruction "$HOME_DIR/.config/goose/.goosehints" goose
fi
if [ -d "$HOME_DIR/.pi/agent" ] || hooked pi; then
  add_instruction "$HOME_DIR/.pi/agent/AGENTS.md" pi
fi
if [ -d "$HOME_DIR/.paradev/agent" ]; then
  # pdev runs the Claude Code hooks
  add_instruction "$HOME_DIR/.paradev/agent/AGENTS.md" claude pdev
fi
if [ -f "$HOME_DIR/.gemini/settings.json" ] || hooked gemini; then
  add_instruction "$HOME_DIR/.gemini/GEMINI.md" gemini
fi
if [ -f "$HOME_DIR/.copilot/mcp-config.json" ] || hooked copilot; then
  add_instruction "$HOME_DIR/.copilot/copilot-instructions.md" copilot
fi
if [ -f "$HERMES_DIR/config.yaml" ] || hooked hermes; then
  # Hermes loads global instructions from SOUL.md (HERMES_HOME), never from
  # ~/.hermes/AGENTS.md (that file is only read from a working directory)
  add_instruction "$HERMES_DIR/SOUL.md" hermes
  if grep -qF '<<< slack-approval-gateway <<<' "$HERMES_DIR/AGENTS.md" 2>/dev/null; then
    say "  migrating: removing the old block from $HERMES_DIR/AGENTS.md"
    quiet_ac instruction-remove "$HERMES_DIR/AGENTS.md" >/dev/null \
      || say "  [failed]    could not remove it; delete SpeakEasy's block from $HERMES_DIR/AGENTS.md by hand"
  fi
fi
if [ -f "$HOME_DIR/.config/zed/settings.json" ]; then
  # Zed reads user-level instructions from ~/.config/zed/AGENTS.md; the old
  # rules/slack-gateway.md location is never loaded, so drop it
  add_instruction "$HOME_DIR/.config/zed/AGENTS.md" none zed
  remove_legacy_zed_rules
fi

# --------------------------------------------------------------------------- #
# 7. exclusivity audit (+ optional enforcement)
# --------------------------------------------------------------------------- #
hr
say "7. Enforcement audit (exclusive sending path)"
hr
AUDIT_FILES=(
  "$HOME_DIR/.claude.json"
  "$CODEX_DIR/config.toml"
  "$HOME_DIR/.config/opencode/opencode.json"
  "$VIBE_DIR/config.toml"
  "$HOME_DIR/.config/goose/config.yaml"
  "$HOME_DIR/.pi/agent/settings.json"
  "$HOME_DIR/.pi/agent/mcp.json"
  "$HOME_DIR/.paradev/agent/settings.json"
  "$HOME_DIR/.paradev/agent/mcp.json"
  "$HOME_DIR/.gemini/settings.json"
  "$HOME_DIR/.copilot/mcp-config.json"
  "$HOME_DIR/.config/zed/settings.json"
  "$HOME_DIR/.config/amp/settings.json"
  "$HERMES_DIR/config.yaml"
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
OTHER_SLACK="$(audit_pairs other_slack_paths)"
TOKENS="$(audit_pairs slack_tokens)"
MISTRAL_KEYS="$(audit_pairs mistral_keys)"

if [ -n "$EXPECTED_SLACK_MCP" ]; then
  if [ "$TRANSPORT" = "hooks-only" ]; then
    say "  Official Slack MCP servers in agent configs (agents read and send"
    say "  through them):"
  else
    say "  Official Slack MCP servers in agent configs (expected: agents read"
    say "  Slack context through them; agents without SpeakEasy's hook are told"
    say "  to send through request_slack_message instead):"
  fi
  report_pairs "$EXPECTED_SLACK_MCP"
fi
if [ -n "$EXPECTED_MISTRAL" ]; then
  say "  Mistral connector servers in agent configs (agents read Slack"
  say "  through them):"
  report_pairs "$EXPECTED_MISTRAL"
fi
if [ -n "$EXPECTED_SLACK_MCP" ] || [ -n "$EXPECTED_MISTRAL" ]; then
  if [ "$NO_HOOKS" = 1 ]; then
    say "  These also have send tools (slack_send_message). With --no-hooks"
    say "  only the instruction block (where the agent has one) asks agents"
    say "  not to use them."
  else
    say "  These also have send tools (slack_send_message). In agents with"
    say "  SpeakEasy's approval hook (5b) every such send waits for your approval;"
    say "  elsewhere only the instruction block (where the agent has one) asks"
    say "  the agent to behave."
  fi
  if [ "$TRANSPORT" != "hooks-only" ]; then
    say "  --enforce keeps these (agents are expected to have them)."
  fi
fi
# Clients SpeakEasy can neither hook nor instruct
GAPS_KEPT="$GAPS"
if [ "$ENFORCE" = 1 ] && [ "$TRANSPORT" != "hooks-only" ]; then
  # --enforce below removes the ones it can edit
  GAPS_KEPT=""
  while IFS= read -r line; do
    [ -n "$line" ] || continue
    if ! "$PY" "$AGENT_CONFIG" can-enforce "$("$PY" -c 'import json,sys; print(json.loads(sys.argv[1])[0])' "$line")"; then
      GAPS_KEPT="$GAPS_KEPT$line"$'\n'
    fi
  done <<< "$GAPS"
fi
NOT_GATED="$(printf '%s\n%s\n%s\n' "$EXPECTED_SLACK_MCP" "$EXPECTED_MISTRAL" "$GAPS_KEPT" | grep -E 'claude_desktop_config\.json|/\.config/zed/' || true)"
if [ -n "$NOT_GATED" ]; then
  say "  NOT gated by SpeakEasy (these clients have no hooks; Zed gets only the"
  say "  instruction block). In Claude Desktop, set the Slack send tools to"
  say "  Needs approval; in Zed, don't choose Always allow for them:"
  report_pairs "$NOT_GATED"
fi
GAPS_HOOKED="$(printf '%s\n' "$GAPS" | grep -vE 'claude_desktop_config\.json|/\.config/zed/' || true)"
gap_agent() { # gap_agent <config file> — the agent whose approval hook covers it
  case "$1" in
    "$HOME_DIR/.claude.json"|"$HOME_DIR/.paradev/agent/"*) echo claude ;;
    "$CODEX_DIR/config.toml") echo codex ;;
    "$HOME_DIR/.config/opencode/opencode.json") echo opencode ;;
    "$VIBE_DIR/config.toml") echo vibe ;;
    "$HOME_DIR/.config/goose/config.yaml") echo goose ;;
    "$HOME_DIR/.pi/agent/"*) echo pi ;;
    "$HOME_DIR/.gemini/settings.json") echo gemini ;;
    "$HOME_DIR/.copilot/mcp-config.json") echo copilot ;;
    "$HOME_DIR/.cursor/mcp.json") echo cursor ;;
    "$HOME_DIR/.config/amp/settings.json") echo amp ;;
    "$HERMES_DIR/config.yaml") echo hermes ;;
  esac
}
# Token-based servers in agents whose approval hook gates them (their writes
# wait for the user), and the rest (agents could send without asking).
GAPS_GATED=""
GAPS_UNGATED=""
while IFS= read -r line; do
  [ -n "$line" ] || continue
  gap_file="$("$PY" -c 'import json,sys; print(json.loads(sys.argv[1])[0])' "$line")"
  gap_hook="$(gap_agent "$gap_file")"
  if [ -n "$gap_hook" ] && hooked "$gap_hook"; then
    GAPS_GATED="$GAPS_GATED$line"$'\n'
  else
    GAPS_UNGATED="$GAPS_UNGATED$line"$'\n'
  fi
done <<< "$GAPS"
if [ -n "$GAPS" ] && [ "$TRANSPORT" = "hooks-only" ]; then
  if [ -n "$GAPS_HOOKED" ]; then
    say "  Token-based Slack MCP servers in agent configs (in the hooks-only setup"
    say "  they are how agents send; in agents with the approval hook (5b) each"
    say "  write waits for your approval):"
    report_pairs "$GAPS_HOOKED"
  fi
elif [ -n "$GAPS" ]; then
  if [ -n "$GAPS_GATED" ]; then
    say "  Token-based Slack MCP servers in agents with SpeakEasy's approval hook"
    say "  (5b): each write through them waits for your approval:"
    report_pairs "$GAPS_GATED"
  fi
  if [ -n "$GAPS_UNGATED" ]; then
    say "  Direct Slack MCP servers in agent configs (agents could send"
    say "  without asking):"
    report_pairs "$GAPS_UNGATED"
  fi
else
  say "  No direct (token-based) Slack MCP servers in agent configs."
fi
if [ -n "$OTHER_SLACK" ]; then
  say "  Other Slack integrations in agent configs (not MCP servers; SpeakEasy"
  say "  doesn't gate them, and --enforce leaves them alone — e.g. Hermes's"
  say "  platform_toolsets.slack, which matters only when Hermes runs as a"
  say "  Slack bot):"
  report_pairs "$OTHER_SLACK"
fi
if [ -n "$TOKENS" ]; then
  say "  Slack tokens found in agent configs (masked):"
  while IFS= read -r line; do
    [ -n "$line" ] || continue
    "$PY" -c 'import json,sys; p,t=json.loads(sys.argv[1]); print(f"    - {p}  ({t})")' "$line"
  done <<< "$TOKENS"
  if [ "$TRANSPORT" != "hooks-only" ]; then
    say "  (these belong to direct Slack MCP servers; --enforce removes the"
    say "   server entries but cannot scrub every copy — rotate the tokens in"
    say "   Slack if you want them gone for good)"
  fi
  if [ "$TRANSPORT" = "slack-mcp" ] || [ "$TRANSPORT" = "korotovsky" ]; then
    say "  If one of these is the login SpeakEasy took over, SpeakEasy sends"
    say "  with it too: rotating it (or signing out of that browser session)"
    if [ "$ENFORCE" = 1 ]; then
      say "  stops SpeakEasy as well. --enforce removes the agent's copy below;"
      say "  delete that config's .bak-sag backup too (it still holds the token)."
    else
      say "  stops SpeakEasy as well. To take it from the agent, use --enforce"
      say "  and delete that config's .bak-sag backup; after rotating, run"
      say "  --take-over again or update the file in $STATE_DIR."
    fi
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
    if [ -n "$EXPECTED_SLACK_MCP" ] || [ -n "$EXPECTED_MISTRAL" ]; then
      say "  Kept (expected in agents): the official Slack MCP / Mistral"
      say "  connector servers listed above (a server holding a Slack token"
      say "  counts as direct, even an official one, and is handled above)."
    fi
  fi
fi

if [ "$DRY_RUN" = 0 ]; then
  DIR_MODE="$(stat -f '%Lp' "$STATE_DIR" 2>/dev/null || echo '?')"
  CRED_FILE=""
  case "$TRANSPORT" in
    slack-mcp) CRED_FILE="$STATE_DIR/slack_mcp_token" ;;
    mistral-connector) CRED_FILE="$STATE_DIR/mistral_api_key" ;;
    korotovsky)
      for f in slack_stdio_env slack_user_token slack_bot_token; do
        if [ -f "$STATE_DIR/$f" ]; then CRED_FILE="$STATE_DIR/$f"; break; fi
      done ;;
  esac
  if [ -n "$CRED_FILE" ] && [ -f "$CRED_FILE" ]; then
    say "  state dir $STATE_DIR (mode $DIR_MODE), $(basename "$CRED_FILE") mode $(stat -f '%Lp' "$CRED_FILE") (want 600)"
  elif [ "$TRANSPORT" = "hooks-only" ]; then
    if stored_credential; then
      say "  state dir $STATE_DIR (mode $DIR_MODE); hooks-only: SpeakEasy doesn't use the credential stored there"
    else
      say "  state dir $STATE_DIR (mode $DIR_MODE); hooks-only: no Slack credential stored"
    fi
  else
    say "  state dir $STATE_DIR (mode $DIR_MODE)"
  fi
fi

# --------------------------------------------------------------------------- #
# 8. self-test: MCP handshake through a real spawned bridge
# --------------------------------------------------------------------------- #
hr
say "8. Self-test (MCP handshake through a spawned process)"
hr
if [ "$DRY_RUN" = 1 ]; then
  say "  [dry-run] skipped"
elif socket_answers; then
  # perl ships with macOS; GNU timeout does not. `alarm shift; exec @ARGV`
  # runs the command under a 15 s alarm.
  SELF_TEST="$(printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"install-self-test","version":"0"}}}' \
    | perl -e 'alarm shift; exec @ARGV' 15 "$INSTALL_BIN" 2>/dev/null | head -1 || true)"
  if printf '%s' "$SELF_TEST" | grep -q '"slack-approval-gateway"'; then
    say "  PASS: a spawned process bridged to the running instance and completed"
    say "        the MCP initialize handshake."
  else
    say "  WARNING: self-test did not get a handshake reply. Service log:"
    say "        $STATE_DIR/gateway.log"
  fi
else
  say "  WARNING: skipped — SpeakEasy is not answering (see step 4 and"
  say "        $STATE_DIR/gateway.log)"
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
    WHY="$( [ -s "$OUTDATED_FILE" ] && awk -F '\t' -v n="$name" '$1 == n { print $2; exit }' "$OUTDATED_FILE" || true)"
    if [ -n "$WHY" ]; then
      say "  OUTDATED    $name ($cfg; $WHY — see above)"
      MISSING=1
    elif has "$cfg"; then
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
  while IFS=$'\t' read -r agent cfg why; do
    say "  NOT GATED   $agent — ${why:-its hook was not added} ($cfg; see above)"
  done < "$HOOK_FAILED_FILE"
  while IFS=$'\t' read -r agent cfg; do
    say "  MISSING     $agent ($cfg; it was not registered, see above)"
    MISSING=1
  done < "$REG_FAILED_FILE"
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
if [ "$TRANSPORT" = "hooks-only" ]; then
  say "  State dir:  $STATE_DIR (cooldowns, sent records, socket)"
else
  say "  State dir:  $STATE_DIR (cooldowns, sent records, credential, socket)"
fi
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
STEP=1
if [ "$SERVICE_UP" = 0 ]; then
  say "  $STEP. Get SpeakEasy running first: see the WARNING in step 4 and"
  if [ "$NO_HOOKS" = 1 ]; then
    say "      $STATE_DIR/gateway.log. Until it runs, no SpeakEasy window"
    say "      appears and request_slack_message fails."
  else
    say "      $STATE_DIR/gateway.log. Until it runs, no SpeakEasy window"
    say "      appears and the approval hooks block your agents' Slack writes."
  fi
  STEP=$((STEP + 1))
fi
if [ "${TAKEOVER_HEAD:-}" = "many" ] && { [ "$CREDENTIAL_OK" = 0 ] || [ "$TRANSPORT" = "hooks-only" ]; }; then
  # several Slack MCP servers could be taken over: picking one is the step
  if [ "$TRANSPORT" = "hooks-only" ]; then
    say "  $STEP. Optional: give SpeakEasy a Slack login of its own by taking over"
    say "      one of these (the approval hooks work without it; with it, Zed's"
    say "      built-in agent, which has no hooks, gets request_slack_message too):"
  else
    say "  $STEP. Pick the Slack MCP server SpeakEasy should take over:"
  fi
  printf '%s\n' "$TAKEOVER_PICK" | tail -n +2 | sed 's/^    /       /'
  STEP=$((STEP + 1))
  CREDENTIAL_OK=1
fi
if [ "$TRANSPORT" = "hooks-only" ]; then CREDENTIAL_OK=1; fi
if [ "$TRANSPORT" = "hooks-only" ]; then
  CREDENTIAL_OK=1
  say "  $STEP. Nothing to set up for Slack: SpeakEasy needs no login in this"
  say "      setup. Your agents keep their own Slack tools; in every agent with"
  say "      SpeakEasy's approval hook (5b), each Slack write waits for you in"
  say "      the SpeakEasy window."
  if [ -d "$HOME_DIR/.config/zed" ] || command -v zed >/dev/null 2>&1; then
    say "      To also give Zed's built-in agent (no hooks) the"
    say "      request_slack_message tool, give SpeakEasy a Slack login of its own"
    say "      (see README.md) and re-run the installer."
  fi
  if stored_credential; then
    say "      (A Slack/Mistral credential is stored in $STATE_DIR, but this"
    say "      setup doesn't use it. To use it: $SELF --transport"
    say "      slack-mcp, mistral-connector or korotovsky, matching the credential.)"
  fi
  if [ -s "$HOOK_FAILED_FILE" ]; then
    say "      Except: $(cut -f1 "$HOOK_FAILED_FILE" | tr '\n' ' ')— their hook is not"
    say "      active (see step 5b), so their Slack writes are not gated until it is."
  fi
  if printf '%s\n' "$NOT_GATED" | grep -q 'claude_desktop_config\.json'; then
    say "      Not gated: Claude Desktop (no hooks; set its Slack send tools to"
    say "      Needs approval — see step 7)."
  fi
  STEP=$((STEP + 1))
elif [ "$TRANSPORT" = "mistral-connector" ]; then
  if [ "$CREDENTIAL_OK" = 0 ]; then
    say "  $STEP. In the Mistral console, connect your Slack workspace to the Slack"
    say "      connector (the gateway calls it; it holds the Slack credentials),"
    say "      and set a real Mistral API key:"
    say "        $SELF --transport mistral-connector --mistral-api-key YOUR_KEY"
    say "      (re-running the installer restarts the service, which reads the"
    say "       file at startup)"
    STEP=$((STEP + 1))
  fi
elif [ "$TRANSPORT" = "korotovsky" ]; then
  if [ "$CREDENTIAL_OK" = 0 ]; then
    say "  $STEP. Give the korotovsky transport a Slack login: your browser"
    say "      session's xoxc/xoxd in $STATE_DIR/slack_stdio_env, or a token:"
    say "        $SELF --transport korotovsky --slack-bot-token xoxb-your-bot-token"
    say "      (or --slack-user-token xoxp-…). See README.md, \"If there's nothing"
    say "      to take over\" → Korotovsky server. SpeakEasy spawns the"
    say "      slack-mcp-server you installed as a stdio child; re-running the"
    say "      installer restarts the service, which reads the login at startup."
    STEP=$((STEP + 1))
  fi
  if [ ! -s "$STATE_DIR/slack_workspace_domain" ]; then
    say "  $STEP. Set the workspace domain used in message links (the server's"
    say "      replies carry no permalink):"
    say "        $SELF --transport korotovsky --workspace-domain YOUR_WORKSPACE"
    STEP=$((STEP + 1))
  fi
else
  if [ "$CREDENTIAL_OK" = 0 ]; then
    say "  $STEP. Get a Slack user token for the Slack MCP server (one-time). Set"
    say "      up a Slack app as README.md, \"If there's nothing to take over\" →"
    say "      Slack MCP describes (its MCP server turned on, the redirect URL"
    say "      http://127.0.0.1:3118/callback, the user scopes listed there), then:"
    say "        $OAUTH --client-id YOUR_SLACK_APP_CLIENT_ID \\"
    say "            --client-secret YOUR_SLACK_APP_CLIENT_SECRET"
    say "      It verifies the token against https://mcp.slack.com/mcp and writes"
    say "      $MCP_TOKEN_FILE (chmod 600). Or set a token directly:"
    say "        $SELF --slack-mcp-token xoxp-your-user-token"
    say "      Then run the installer again: it restarts SpeakEasy, which reads the"
    say "      token at startup."
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
say "  Uninstall: $SELF --uninstall"
if [ -n "$GAPS_UNGATED" ] && [ "$TRANSPORT" != "hooks-only" ] && [ "${TAKEOVER_HEAD:-}" != "many" ] && [ "$ENFORCE" = 0 ]; then
  say "  Enforce exclusivity (remove direct Slack MCP servers from agents):"
  say "     $SELF --enforce"
fi
