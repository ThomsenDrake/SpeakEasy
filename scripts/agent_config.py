#!/usr/bin/env python3
"""Edit coding-agent MCP configs to register (or remove) the Slack Approval
Gateway stdio server.

Every file is backed up once to `<file>.bak-sag` before the first
modification (the backup is the pre-install original; it is kept for a
manual restore — removals never overwrite a file with it). All operations
are idempotent: adding an already-registered server updates the entry in
place, and a `--dry-run` prints what would change.

Subcommands:
  add-json STYLE FILE CMD     register in a JSON config. STYLE is one of:
                              claude (mcpServers, type stdio — ~/.claude.json)
                              gemini (mcpServers — ~/.gemini/settings.json)
                              pi     (mcpServers — the old ~/.pi/agent/settings.json
                                      and ~/.paradev/agent/settings.json place)
                              pimcp  (mcpServers + 24 h timeout — ~/.pi/agent/mcp.json,
                                      also pdev: ~/.paradev/agent/mcp.json)
                              copilot(mcpServers, type local — ~/.copilot/mcp-config.json)
                              opencode(mcp, command array — ~/.config/opencode/opencode.json)
                              amp    (amp.mcpServers — ~/.config/amp/settings.json)
  add-zed FILE CMD            ~/.config/zed/settings.json context_servers (JSONC:
                              comments and trailing commas are preserved)
  add-goose FILE CMD          ~/.config/goose/config.yaml extensions (YAML)
  add-hermes FILE CMD         ~/.hermes/config.yaml mcp_servers (YAML)
  add-codex FILE CMD          ~/.codex/config.toml [mcp_servers.NAME] (fallback
                              when the `codex mcp add` CLI is unavailable)
  add-vibe FILE CMD           ~/.vibe/config.toml [[mcp_servers]] element
                              (Mistral Vibe's array style; fallback when the
                              `vibe mcp add` CLI is unavailable)
  remove-json STYLE FILE      unregister (removes only the gateway's own
                              entry; the .bak-sag backup is kept, never
                              restored over the file)
  remove-zed|remove-goose|remove-hermes|remove-codex|remove-vibe FILE
                              unregister from a non-JSON config (removes only
                              the gateway's own entry/block; backup kept)
  remove-entry STYLE FILE NAME [KEYS]
                              remove an arbitrary server entry (used by
                              install.sh --enforce to drop direct Slack
                              servers). KEYS is a JSON list of keys locating
                              a nested servers container — e.g. claude's
                              projects.<path>.mcpServers — instead of the
                              style's top-level parent key
  remove-codex-entry FILE NAME
                              remove a [mcp_servers.NAME] table from a TOML
                              config (codex style; used by --enforce)
  remove-vibe-entry FILE NAME
                              remove one [[mcp_servers]] element (matched by
                              its name = "..."), its [mcp_servers.KEY]
                              subtables included, from a TOML config (vibe
                              style; used by --enforce)
  set-timeout FILE NAME SECONDS
                              give the entry a long tool-call timeout —
                              tool_timeout_sec in a [mcp_servers.NAME] table or
                              a [[mcp_servers]] element (codex/vibe TOML), the
                              timeout: line in a goose extensions or Hermes
                              mcp_servers YAML block. A request_slack_message
                              call blocks on a human approval for minutes
  upgrade-json STYLE FILE GATEWAY_PATH
                              bring the gateway's existing JSON entry up to
                              date (OpenCode's "type", the tool-call timeout,
                              Copilot's "tools"), keeping keys the user added
  upgrade-zed FILE GATEWAY_PATH
                              replace the gateway's Zed context_servers entry
                              when it is in the old nested form or has no
                              timeout
  enforce-remove PAIR_JSON    remove one direct Slack server found by the
                              audit. PAIR_JSON is the [file, name, key] entry
                              as reported by audit. Prints an accurate status
                              line: [removed], [absent], [failed], or
                              [edit by hand]
  hook-add AGENT FILE BIN WAIT STATE_DIR
                              register SpeakEasy's approval hook (BIN --hook
                              AGENT --wait WAIT --state-dir STATE_DIR
                              --ok-code 10) so the agent's own Slack tools
                              ask the user before they run. AGENT/FILE:
                              claude  ~/.claude/settings.json (PreToolUse +
                                      ConfigChange; pdev reads it too)
                              codex   ~/.codex/config.toml (+ trust entry)
                              gemini  ~/.gemini/settings.json (BeforeTool)
                              copilot ~/.copilot/hooks/speakeasy.json
                              cursor  ~/.cursor/hooks.json (beforeMCPExecution)
                              hermes  ~/.hermes/config.yaml (+ plugins/speakeasy)
                              amp     ~/.config/amp/plugins/speakeasy.ts
                              vibe    ~/.vibe/hooks.toml ([[hooks]] pre_tool)
                              goose   ~/.agents/plugins/speakeasy-slack-approval/
                                      hooks/hooks.json
                              opencode ~/.config/opencode/plugins/
                                      speakeasy-slack-approval.js
                              pi      ~/.pi/agent/extensions/speakeasy-slack-gate.ts
                              Replaces an earlier SpeakEasy hook in place
  hook-remove AGENT FILE      remove SpeakEasy's hook (other hooks are kept)
  hook-has AGENT FILE         exit 0 when SpeakEasy's hook is registered
  instruction-add FILE [gateway|either|hooks|confirm]
                              insert/replace the managed instruction block in
                              an agent instruction file: "use the gateway's
                              request_slack_message" (default; agents without
                              SpeakEasy's hook), "use your Slack tool or
                              request_slack_message; the user approves either"
                              (hooked agents when SpeakEasy can send), or, for
                              the hooks-only setup, "use your Slack tool; the
                              user approves each call" (hooked agents) or
                              "ask the user in chat first" (agents without
                              the hook)
  instruction-remove FILE     remove the managed instruction block
  has FILE [NAME]             exit 0 when NAME (default: the gateway) is
                              registered in FILE
  takeover-candidates FILE... JSON list of the Slack MCP servers an agent
                              already uses that SpeakEasy could take over —
                              a korotovsky slack-mcp-server (its command and
                              SLACK_MCP_* tokens, a browser session included),
                              the official Slack MCP server with a token in
                              its headers, or a Mistral Slack connector with a
                              key in its headers — and the ones it cannot
                              (OAuth sessions owned by another app, Claude
                              Code's Slack plugin). Secrets are masked. FILE
                              may also be Claude Code's
                              plugins/installed_plugins.json
  can-enforce FILE            exit 0 when --enforce can remove servers from
                              FILE itself (goose, Zed and Hermes configs are
                              edited by hand)
  take-over STATE_DIR FILE NAME
                              copy one importable candidate into SpeakEasy's
                              state dir (credential files chmod 600) and
                              select its transport. Credential files it
                              replaces are moved to <file>.pre-takeover
                              (placeholders are deleted). Prints what it
                              wrote, secrets masked
  audit FILE...               JSON report categorizing what agents have
                              installed: official Slack MCP servers and
                              Mistral connector servers (expected — agents
                              read Slack context through them), direct
                              (token-based) Slack MCP servers and other
                              agent-side Slack paths (violations), and
                              Slack/Mistral credentials found in the files.
                              Server entries are {"name": ..., "key": [...]}
                              where key is the JSON path of the servers
                              container (nested per-project entries
                              included), the YAML parent key, or [] for TOML
                              tables.
"""

import hashlib
import json
import os
import re
import shlex
import shutil
import stat
import sys

SERVER = "slack-approval-gateway"
BAK = ".bak-sag"

MARK_BEGIN = (
    "<!-- >>> slack-approval-gateway (managed by scripts/install.sh; "
    "remove with: install.sh --uninstall) >>> -->"
)
MARK_END = "<!-- <<< slack-approval-gateway <<< -->"

INSTRUCTION = """## Slack: send only through the approval gateway

You may have the Slack MCP server or the Mistral Slack connector available — use them to read Slack context (search messages, list channels, read profiles). To send a Slack message, call the MCP tool `request_slack_message` (server `slack-approval-gateway`): a desktop popup shows the exact message and destination for approval, and only the exact approved text is sent to the exact approved destination. Never send through the Slack MCP server's send tools (e.g. `slack_send_message`), the Mistral connector's Slack tools, curl, webhooks, SDKs, CLIs, scripts, or the browser."""

# The hooks-only setup (SpeakEasy has no Slack connection of its own): agents
# send with their own Slack tools, and SpeakEasy's pre-tool hook asks first.
INSTRUCTION_HOOKS = """## Slack: every message needs the user's approval

To send a Slack message, use your Slack tool (for example `slack_send_message`) as usual. Before the call runs, SpeakEasy shows the user the exact message and destination in an approval window, and the call only runs if they approve. If the call is blocked, nothing was sent: follow the reason it gives (after feedback, revise and call again; never retry a denied message). Never send Slack messages any other way: curl, webhooks, SDKs, CLIs, scripts, or the browser."""

# SpeakEasy can send AND the agent has SpeakEasy's hook: both routes ask the
# user first, so the agent may use either.
INSTRUCTION_EITHER = """## Slack: every message needs the user's approval

To send a Slack message, use your Slack tool (for example `slack_send_message`) as usual, or the MCP tool `request_slack_message` (server `slack-approval-gateway`). Either way SpeakEasy shows the user the exact message and destination in an approval window first, and only what they approve is sent. If a call is blocked, nothing was sent: follow the reason it gives (after feedback, revise and call again; never retry a denied message). Never send Slack messages any other way: curl, webhooks, SDKs, CLIs, scripts, or the browser."""

# The hooks-only setup, for an agent without SpeakEasy's hook (Zed's
# built-in agent): nothing but the instruction stands between it and Slack.
INSTRUCTION_CONFIRM = """## Slack: ask the user before sending

Before you send, post, schedule, or react to anything in Slack, show the user the exact message and where it would go (channel, person, thread), and wait for an explicit yes in this conversation. Reading Slack is fine. Never send Slack messages through curl, webhooks, SDKs, CLIs, scripts, or the browser."""

# STYLE -> (parent key, entry builder). A parent key starting with "@" is a
# single literal key (amp settings use the flat "amp.mcpServers" key, VS
# Code-style); otherwise the key is a dotted path into nested objects.
# A request_slack_message call waits for a human, often for minutes, so each
# entry gets a 24 h tool-call timeout in the agent's own units (an integer:
# 0, null or a string disables the server in several agents). Defaults
# without it: Claude Code 30 min idle, Gemini 10 min, Copilot 3 min,
# OpenCode 60 s. Amp has no setting (a fixed 5 min).
TOOL_TIMEOUT_MS = 86400 * 1000
JSON_STYLES = {
    "claude": ("mcpServers", lambda cmd: {"type": "stdio", "command": cmd, "args": [],
                                          "timeout": TOOL_TIMEOUT_MS}),
    "gemini": ("mcpServers", lambda cmd: {"command": cmd, "args": [], "timeout": TOOL_TIMEOUT_MS}),
    "pi": ("mcpServers", lambda cmd: {"command": cmd, "args": []}),
    # pi 1.4+ and pdev read MCP servers from mcp.json, where `timeout` is a
    # per-request limit in seconds (default 60)
    "pimcp": ("mcpServers", lambda cmd: {"command": cmd, "args": [], "timeout": 86400}),
    "copilot": ("mcpServers", lambda cmd: {"type": "local", "command": cmd, "args": [],
                                           "tools": ["*"], "timeout": TOOL_TIMEOUT_MS}),
    # OpenCode ignores an entry without "type"; its `command` is the argv
    "opencode": ("mcp", lambda cmd: {"type": "local", "command": [cmd], "enabled": True,
                                     "timeout": TOOL_TIMEOUT_MS}),
    "amp": ("@amp.mcpServers", lambda cmd: {"command": cmd, "args": []}),
}

TOKEN_RE = re.compile(r"xox[bpcdea]-[A-Za-z0-9-]{10,}")
MISTRAL_KEY_RE = re.compile(r"mstrl_[A-Za-z0-9_-]{10,}")
MCP_PARENT_KEYS = {"mcpServers", "mcp", "context_servers", "mcp_servers", "amp.mcpServers"}
# YAML parents whose 2-space-indented entries are MCP servers (goose uses
# `extensions`, hermes `mcp_servers`).
YAML_MCP_PARENTS = {"mcp_servers", "extensions", "context_servers", "mcp"}


def classify_server(name, entry_text):
    """Classify one MCP server entry for the exclusivity audit.

    Returns one of:
      "slack-mcp"  the official Slack MCP server (remote, OAuth; mcp.slack.com)
      "mistral"    a Mistral MCP/connector server (exposes the Slack connector)
      "direct"     a direct Slack server (a Slack token in the entry) or an
                   unclassifiable slack-named server
      "other"      unrelated
    """
    if TOKEN_RE.search(entry_text):
        return "direct"
    low = entry_text.lower()
    if "mcp.slack.com" in low or "slack.com/mcp" in low:
        return "slack-mcp"
    if "mistral" in name.lower() or "mistral.ai" in low:
        return "mistral"
    if "slack" in name.lower():
        return "direct"
    return "other"


# --------------------------------------------------------------------------- #
# small helpers
# --------------------------------------------------------------------------- #

def read(path):
    try:
        # utf-8-sig: a byte-order mark is read past (and not written back)
        with open(path, "r", encoding="utf-8-sig") as f:
            return f.read()
    except (FileNotFoundError, NotADirectoryError, IsADirectoryError):
        return None


def _replace_atomically(path, writer):
    """Write a file through a temp file and rename it over `path`. A symlink
    is followed (the dotfile it points at is edited, the link stays), and an
    existing file keeps its permissions — a config holding tokens at 600
    must not become world-readable."""
    path = os.path.realpath(path)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    try:
        mode = stat.S_IMODE(os.stat(path).st_mode)
    except FileNotFoundError:
        umask = os.umask(0)
        os.umask(umask)
        mode = 0o666 & ~umask
    tmp = path + ".tmp"
    fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w", encoding="utf-8") as f:
        writer(f)
    os.chmod(tmp, mode)
    os.replace(tmp, path)


def write(path, text):
    _replace_atomically(path, lambda f: f.write(text))


def _speakeasy_content(text):
    """Whether a config already holds something SpeakEasy wrote (its MCP
    entry, a hook, its plugin entry) — or is only the empty skeleton
    SpeakEasy leaves after removing its entries. Neither is the user's
    pre-install original."""
    if (SERVER in text or "--hook " in text or "speakeasy-slack-approval" in text
            or re.search(r"(?m)^\s+- ['\"]?speakeasy['\"]?\s*$", text)):
        return True
    stripped = text.strip()
    yaml_lines = [l.split("#")[0].rstrip() for l in stripped.split("\n") if l.strip()]
    if all(l in ("extensions:", "mcp_servers:", "plugins:", "  enabled: []") for l in yaml_lines):
        # (empty, or only the empty parents SpeakEasy leaves after removal)
        return True
    try:
        data = json.loads(stripped)
    except ValueError:
        try:
            data = json.loads(strip_jsonc(stripped))
        except ValueError:
            return False

    def empty(v):
        return isinstance(v, (dict, list)) and all(empty(x) for x in (v.values() if isinstance(v, dict) else v))

    return isinstance(data, dict) and all(empty(v) for k, v in data.items() if k != "version")


def backup(path, force=False):
    """Back up the pre-install original once; never overwrite an existing
    backup. A file that already holds SpeakEasy's content (or only the
    skeleton it leaves behind) is not the original, so it is not backed up
    unless `force` (--enforce removals, which delete the user's own
    entries)."""
    if not os.path.exists(path) or os.path.exists(path + BAK):
        return
    if not force and _speakeasy_content(read(path) or ""):
        return
    shutil.copy2(path, path + BAK)


def has_entry(path, name=SERVER):
    """Whether `name` is registered as an MCP server in `path`: an entry in
    an MCP servers container (JSON/JSONC), a [mcp_servers.NAME] table or a
    [[mcp_servers]] element named NAME (TOML), or a NAME: block under an
    MCP parent key (YAML). Other mentions — a hook command, a disabled-
    servers list, a permission rule — don't count."""
    text = read(path)
    if text is None or name not in text:
        return False
    if path.endswith(".json"):
        try:
            data = json.loads(text)
        except ValueError:
            try:
                data = json.loads(strip_jsonc(text))
            except ValueError:
                return False
        if not isinstance(data, dict):
            return False
        # Top-level containers only: a per-project entry in ~/.claude.json
        # (projects.<path>.mcpServers) is not the user-scope registration.
        containers = [data.get(k) for k in MCP_PARENT_KEYS]
        mcp = data.get("mcp")
        if isinstance(mcp, dict):
            containers.append(mcp.get("servers"))
        return any(isinstance(c, dict) and name in c for c in containers)
    if path.endswith(".toml"):
        quoted = re.escape(name)
        return any(_codex_server_header(name).match(l.strip()) for l in text.split("\n")) or any(
            re.search(r'(?m)^\s*name\s*=\s*["\']%s["\']' % quoted, "\n".join(lines[s0 + 1:e0]))
            for lines in [text.split("\n")]
            for s0, e0, tname, arr in _toml_table_spans(lines) if arr and tname == "mcp_servers")
    if path.endswith((".yaml", ".yml")):
        parents = "|".join(sorted(YAML_MCP_PARENTS))
        lines = text.split("\n")
        in_parent = False
        for line in lines:
            if re.match(r"^(%s)\s*:\s*(#.*)?$" % parents, line):
                in_parent = True
                continue
            if line.strip() and not line.startswith(" "):
                in_parent = False
            if in_parent and re.match(r"^  ['\"]?%s['\"]?\s*:" % re.escape(name), line):
                return True
        return False
    return name in text


def backup_original(path):
    """Back up a file before SpeakEasy's first change to it — only while it
    holds nothing of SpeakEasy's: a later run would otherwise back up
    SpeakEasy's own version (whose hook blocks every Slack call once the
    binary is gone)."""
    backup(path)


class JsoncFile(SystemExit):
    """A JSON config with comments or trailing commas: SpeakEasy does not
    rewrite it (that would drop them)."""


def load_json(path):
    text = read(path)
    if text is None or not text.strip():
        return {}
    try:
        return json.loads(text)
    except ValueError:
        try:
            json.loads(strip_jsonc(text))
        except ValueError:
            raise SystemExit("%s is not valid JSON; fix it, then run the installer again" % path)
        raise JsoncFile("%s has comments or trailing commas; SpeakEasy will not rewrite it" % path)


def dump_json(path, data):
    def writer(f):
        json.dump(data, f, indent=2, ensure_ascii=False)
        f.write("\n")
    _replace_atomically(path, writer)


def parent_of(data, keypath):
    """Return (container, last_key) for a parent key. "@flat.key" is one
    literal key; otherwise the key is a dotted path, creating objects."""
    if keypath.startswith("@"):
        return data, keypath[1:]
    parts = keypath.split(".")
    d = data
    for p in parts[:-1]:
        nxt = d.get(p)
        if not isinstance(nxt, dict):
            nxt = {}
            d[p] = nxt
        d = nxt
    return d, parts[-1]


def strip_jsonc(text):
    """Remove // and /* */ comments (outside strings) and trailing commas."""
    out = []
    i, n = 0, len(text)
    in_str = esc = False
    while i < n:
        c = text[i]
        if in_str:
            out.append(c)
            if esc:
                esc = False
            elif c == "\\":
                esc = True
            elif c == '"':
                in_str = False
            i += 1
            continue
        if c == '"':
            in_str = True
            out.append(c)
            i += 1
            continue
        if c == "/" and i + 1 < n and text[i + 1] == "/":
            while i < n and text[i] != "\n":
                i += 1
            continue
        if c == "/" and i + 1 < n and text[i + 1] == "*":
            i += 2
            while i + 1 < n and not (text[i] == "*" and text[i + 1] == "/"):
                i += 1
            i += 2
            continue
        out.append(c)
        i += 1
    s = "".join(out)
    return re.sub(r",(\s*[}\]])", r"\1", s)


# --------------------------------------------------------------------------- #
# JSON configs
# --------------------------------------------------------------------------- #

def add_json(style, path, cmd):
    keypath, make = JSON_STYLES[style]
    try:
        data = load_json(path)
    except JsoncFile as e:
        raise SystemExit("%s. Add this to its %s by hand:\n  %s" % (
            e, keypath.lstrip("@"), json.dumps({SERVER: make(cmd)})))
    if not isinstance(data, dict):
        raise SystemExit(f"{path}: top level is not a JSON object")
    container, key = parent_of(data, keypath)
    servers = container.get(key)
    if not isinstance(servers, dict):
        servers = {}
        container[key] = servers
    action = "updated" if SERVER in servers else "added"
    servers[SERVER] = make(cmd)
    backup(path)
    dump_json(path, data)
    return action


def _upgrade_fields(style, entry, cmd):
    """The fields of SpeakEasy's existing entry that keep it from loading or
    cut approvals short, as {key: value}: the command, OpenCode's required
    "type", and the tool-call timeout. Optional fields ("args", "enabled",
    Copilot's "tools", Claude's "type") and keys the user added are left
    alone; a longer timeout stays."""
    want = JSON_STYLES[style][1](cmd)
    fix = {}
    if entry.get("command") != want["command"]:
        fix["command"] = want["command"]
    if style == "opencode" and entry.get("type") != "local":
        fix["type"] = "local"
    if "timeout" in want:
        have = entry.get("timeout")
        # past a JS timer's limit (2^31 - 1 ms) Gemini and OpenCode time
        # out at once
        if (not isinstance(have, int) or isinstance(have, bool) or have < want["timeout"]
                or have > 2**31 - 1):
            fix["timeout"] = want["timeout"]
    return fix


def upgrade_json(style, path, cmd):
    """Bring SpeakEasy's existing entry up to date (a missing "type" makes
    OpenCode ignore it; a missing timeout cuts approvals short)."""
    keypath = JSON_STYLES[style][0]
    text = read(path)
    if text is None or SERVER not in text:
        return "absent"
    try:
        data = load_json(path)
        jsonc = False
    except JsoncFile:
        data = json.loads(strip_jsonc(text))
        jsonc = True
    if not isinstance(data, dict):
        return "absent"
    container, key = parent_of(data, keypath)
    servers = container.get(key)
    entry = servers.get(SERVER) if isinstance(servers, dict) else None
    if not isinstance(entry, dict):
        return "absent"
    fix = _upgrade_fields(style, entry, cmd)
    if not fix:
        return "unchanged"
    if jsonc:
        raise SystemExit("%s has comments or trailing commas; SpeakEasy will not rewrite it. "
                         "Set these in its %s entry by hand: %s"
                         % (path, SERVER, json.dumps(fix)))
    entry.update(fix)
    dump_json(path, data)
    return "updated (%s)" % ", ".join(sorted(fix))


def remove_json(style, path):
    """Uninstall path: remove only the gateway's own entries — the top-level
    one and any nested ones (e.g. claude per-project projects.<path>.mcpServers).
    The .bak-sag backup (the pre-install original) is kept for a manual
    restore; the file is never overwritten with it, so changes made after
    the install survive."""
    # (any depth: uninstall also removes per-project entries)
    if SERVER not in (read(path) or ""):
        return "absent"
    try:
        data = load_json(path)
    except JsoncFile as e:
        # with comments: complain only when an MCP entry really is there
        # (a hook command also mentions SpeakEasy's binary)
        found = []

        def deep(obj):
            if isinstance(obj, dict):
                for k, v in obj.items():
                    if k in MCP_PARENT_KEYS and isinstance(v, dict) and SERVER in v:
                        found.append(k)
                    deep(v)
            elif isinstance(obj, list):
                for v in obj:
                    deep(v)

        deep(json.loads(strip_jsonc(read(path))))
        if not found:
            return "absent"
        raise SystemExit("%s; remove the %s entry by hand" % (e, SERVER))
    if not isinstance(data, dict):
        return "absent"
    removed = False

    def sweep(obj):
        nonlocal removed
        if isinstance(obj, dict):
            for k, v in obj.items():
                if k in MCP_PARENT_KEYS and isinstance(v, dict) and SERVER in v:
                    del v[SERVER]
                    removed = True
                sweep(v)
        elif isinstance(obj, list):
            for v in obj:
                sweep(v)

    sweep(data)
    if not removed:
        return "absent"
    backup(path)
    dump_json(path, data)
    return "removed"


def remove_entry(style, path, name, keys=None):
    """Remove an arbitrary server entry (install.sh --enforce uses this to
    drop direct Slack servers from agent configs). With KEYS (a list of
    JSON keys), the servers container is that nested path — e.g. claude's
    per-project projects.<path>.mcpServers — instead of the style's
    top-level parent key."""
    backup(path, force=True)
    keypath, _ = JSON_STYLES[style]
    data = load_json(path)
    if not isinstance(data, dict):
        return "absent"
    if keys:
        container = data
        for k in keys[:-1]:
            nxt = container.get(k) if isinstance(container, dict) else None
            if not isinstance(nxt, dict):
                return "absent"
            container = nxt
        key = keys[-1]
    else:
        container, key = parent_of(data, keypath)
    servers = container.get(key) if isinstance(container, dict) else None
    if not isinstance(servers, dict) or name not in servers:
        return "absent"
    del servers[name]
    dump_json(path, data)
    return "removed"


# --------------------------------------------------------------------------- #
# Zed (JSONC)
# --------------------------------------------------------------------------- #

# Zed caps every MCP request at 600 s (larger values are clamped); the
# gateway withdraws a Zed call just before that. The nested
# {"command": {"path": …}} form older installs wrote is not loaded by
# current Zed.
ZED_TIMEOUT_SEC = 600


def zed_entry(cmd):
    return '"%s": {"command": %s, "args": [], "timeout": %d}' % (SERVER, json.dumps(cmd), ZED_TIMEOUT_SEC)


def upgrade_zed(path, cmd):
    """Replace an entry in the old nested form, or one without the timeout,
    with the current one (one write: the file is never left half-done)."""
    text = _read_exact(path)
    if text is None or SERVER not in text:
        return "absent"
    if not _jsonc_ok(text):
        raise SystemExit("%s is not valid JSON (with comments); fix it, then SpeakEasy's %s "
                         "entry, by hand: %s" % (path, SERVER, zed_entry(cmd)))
    servers = json.loads(strip_jsonc(text.lstrip("\ufeff"))).get("context_servers")
    entry = servers.get(SERVER) if isinstance(servers, dict) else None
    if not isinstance(entry, dict):
        return "absent"
    timeout = entry.get("timeout")
    fixed = []
    if not isinstance(entry.get("command"), str):
        fixed.append("flat form")
    elif entry["command"] != cmd:
        fixed.append("command")
    if not (isinstance(timeout, int) and not isinstance(timeout, bool) and timeout >= ZED_TIMEOUT_SEC):
        fixed.append("timeout")
    if not fixed:
        return "unchanged"
    new = _zed_add_text(_zed_remove_text(text), cmd)
    if not _jsonc_ok(new):
        raise SystemExit("%s: SpeakEasy could not update its entry safely; replace it by hand "
                         "with: %s" % (path, zed_entry(cmd)))
    write(path, new)
    return "updated (%s)" % ", ".join(fixed)


def _scan_string(text, i):
    """End (exclusive) of the JSON string whose opening quote is at i."""
    i += 1
    esc = False
    while i < len(text):
        c = text[i]
        if esc:
            esc = False
        elif c == "\\":
            esc = True
        elif c == '"':
            return i + 1
        i += 1
    return len(text)


def _skip_comment(text, i):
    """End (exclusive) of the // or /* */ comment at i, or i if there is none."""
    if text[i : i + 2] == "//":
        j = text.find("\n", i)
        return len(text) if j < 0 else j
    if text[i : i + 2] == "/*":
        j = text.find("*/", i + 2)
        return len(text) if j < 0 else j + 2
    return i


def _skip_ws(text, i):
    while i < len(text) and text[i] in " \t\r\n\ufeff":
        i += 1
    return i


def _skip_ws_comments(text, i):
    while True:
        j = _skip_ws(text, i)
        k = _skip_comment(text, j)
        if k == j:
            return j
        i = k


def _scan_json(text, i):
    """End (exclusive) of the JSON value starting at i: balanced brackets,
    with strings and comments respected (a bracket inside either does not
    count)."""
    n = len(text)
    i = _skip_ws_comments(text, i)
    if i < n and text[i] in "{[":
        depth = 0
        while i < n:
            c = text[i]
            if c == '"':
                i = _scan_string(text, i)
                continue
            if c == "/":
                j = _skip_comment(text, i)
                if j != i:
                    i = j
                    continue
            if c in "{[":
                depth += 1
            elif c in "}]":
                depth -= 1
                if depth == 0:
                    return i + 1
            i += 1
        return n
    if i < n and text[i] == '"':
        return _scan_string(text, i)
    # a number, true, false or null: its own characters only (a comment
    # after it may hold , } or ])
    while i < n and (text[i].isalnum() or text[i] in "+-._"):
        i += 1
    return i


def _top_object(text):
    """Index of the "{" opening a JSONC document's top-level object, or None."""
    i = _skip_ws_comments(text, 0)
    return i if i < len(text) and text[i] == "{" else None


def _members(text, obj_start):
    """(key, key_start, value_start, value_end) for each member of the JSONC
    object whose "{" is at obj_start. Comments (and a "}" or a quoted key
    inside one) are skipped; scanning stops at anything unexpected."""
    out = []
    n = len(text)
    i = obj_start + 1
    while True:
        i = _skip_ws_comments(text, i)
        if i >= n or text[i] == "}":
            return out
        if text[i] == ",":
            i += 1
            continue
        if text[i] != '"':
            return out
        key_end = _scan_string(text, i)
        try:
            key = json.loads(text[i:key_end])
        except ValueError:
            return out
        colon = _skip_ws_comments(text, key_end)
        if colon >= n or text[colon] != ":":
            return out
        value = _skip_ws_comments(text, colon + 1)
        value_end = _scan_json(text, value)
        out.append((key, i, value, value_end))
        i = value_end


def _zed_servers(text):
    """(top, context_servers member or None) of a Zed settings file; of
    duplicate keys the last counts, as in JSON."""
    top = _top_object(text)
    if top is None:
        return None, None
    found = [m for m in _members(text, top) if m[0] == "context_servers"]
    return top, (found[-1] if found else None)


def _read_exact(path):
    """A file's text with its line endings and byte-order mark kept, so a
    splice changes nothing but the spliced bytes."""
    try:
        with open(path, "r", encoding="utf-8", newline="") as f:
            return f.read()
    except (FileNotFoundError, NotADirectoryError, IsADirectoryError):
        return None


def _unterminated(text):
    """Whether a string or /* comment runs to the end of the text."""
    i, n = 0, len(text)
    while i < n:
        if text[i] == '"':
            j = _scan_string(text, i)
            if j >= n and not text.endswith('"'):
                return True
            i = j
        elif text.startswith("/*", i):
            j = text.find("*/", i + 2)
            if j < 0:
                return True
            i = j + 2
        elif text.startswith("//", i):
            i = _line_end(text, i)
        else:
            i += 1
    return False


def _jsonc_ok(text):
    """Whether a JSONC file parses (Zed's settings allow comments and
    trailing commas)."""
    if _unterminated(text):
        return False
    try:
        json.loads(strip_jsonc(text.lstrip("\ufeff")) or "{}")
        return True
    except ValueError:
        return False


def _line_end(text, i):
    j = text.find("\n", i)
    return len(text) if j < 0 else j


class _ZedEdit(Exception):
    """A Zed settings file SpeakEasy will not edit; the message says why."""


def _zed_add_text(text, cmd):
    """`text` with SpeakEasy's entry spliced into context_servers (created if
    missing); every other byte is kept."""
    if not text.strip():
        text = "{}"
    entry = zed_entry(cmd)
    top, servers = _zed_servers(text)
    if top is None:
        raise _ZedEdit("the top level is not a JSON object")
    if servers and text[servers[2]] != "{":
        raise _ZedEdit("context_servers is not an object")
    if servers:
        o, close = servers[2], servers[3] - 1  # its "{" and "}"
        if not text[o + 1:close].strip() and "\n" not in text[o + 1:close]:
            # {} / { }: inline, after the spaces (removal cuts just the entry)
            at = o + 1
            while text[at] in " \t":
                at += 1
            return text[:at] + entry + text[at:]
        comma = "," if _members(text, o) else ""
        line_end = _line_end(text, o + 1)
        rest = text[o + 1:line_end].strip()
        if not rest or rest.startswith("//"):
            # on a line of its own after the "{" line (and a comment on it),
            # with that line's own line ending
            at = line_end - 1 if text[line_end - 1:line_end] == "\r" else line_end
            nl = text[at:line_end + 1] or "\n"
            return text[:at] + nl + "    " + entry + comma + text[at:]
        # inline style: right after the "{" (and the spaces after it)
        at = o + 1
        while text[at] in " \t":
            at += 1
        return text[:at] + entry + (comma + " " if comma else "") + text[at:]
    nl = "\r\n" if "\r\n" in text else "\n"
    comma = "," if _members(text, top) else ""
    return (text[: top + 1] + nl + '  "context_servers": {' + nl + "    " + entry + nl
            + "  }" + comma + text[top + 1:])


def _zed_remove_text(text):
    """`text` without SpeakEasy's context_servers entry — its line when it
    is alone on one, otherwise the key and value — and the comma that
    separated it from a neighbour; comments and every other byte are kept.
    None when there is no entry."""
    _, servers = _zed_servers(text)
    if not servers or text[servers[2]] != "{":
        return None
    obj_start, obj_end = servers[2], servers[3] - 1  # its "{" and "}"
    members = _members(text, obj_start)
    k = next((i for i, m in enumerate(members) if m[0] == SERVER), None)
    if k is None:
        return None
    key_start, value_end = members[k][1], members[k][3]
    after = _skip_ws_comments(text, value_end)
    own_comma = after < obj_end and text[after] == ","
    line_start = text.rfind("\n", 0, key_start) + 1
    line_end = _line_end(text, value_end)
    tail = text[value_end:line_end]
    if own_comma and after < line_end:
        tail = tail.replace(",", "", 1)
    cuts = []
    if not text[line_start:key_start].strip() and not tail.strip():
        # alone on its line: the whole line goes (with its comma)
        cuts.append((line_start, min(line_end + 1, len(text))))
        if own_comma and after > line_end:
            # a comma-first line: its comma and the spaces after it
            end = after + 1
            while end < len(text) and text[end] in " \t":
                end += 1
            cuts.append((after, end))
    else:
        cuts.append((key_start, value_end))
        if own_comma:
            # its comma and the spaces after it; comments before it stay
            end = after + 1
            while end < len(text) and text[end] in " \t":
                end += 1
            cuts.append((after, end))
    if not own_comma and k > 0:
        # the last entry: the comma after the one before goes
        c = _skip_ws_comments(text, members[k - 1][3])
        if text[c] == ",":
            cuts.append((c, c + 1))
    for a, b in sorted(cuts, reverse=True):
        text = text[:a] + text[b:]
    return text


def add_zed(path, cmd):
    text = _read_exact(path)
    if text is None or not text.strip():
        text = "{}"
    by_hand = "add this to its context_servers by hand: %s" % zed_entry(cmd)
    if not _jsonc_ok(text):
        raise SystemExit("%s is not valid JSON (with comments); SpeakEasy will not edit it. "
                         "Fix it, or %s" % (path, by_hand))
    _, servers = _zed_servers(text)
    if servers and text[servers[2]] == "{" and any(m[0] == SERVER for m in _members(text, servers[2])):
        return "already"
    try:
        new = _zed_add_text(text, cmd)
    except _ZedEdit as e:
        raise SystemExit("%s: %s; %s" % (path, e, by_hand))
    if not _jsonc_ok(new):
        raise SystemExit("%s: SpeakEasy could not add its entry safely; %s" % (path, by_hand))
    # (a mention in a comment is not SpeakEasy's content: back up the original)
    backup(path, force=True)
    write(path, new)
    return "added"


def remove_zed(path):
    """Text splice, the inverse of add_zed (see _zed_remove_text). A file
    that does not parse is not edited."""
    text = _read_exact(path)
    if text is None or SERVER not in text:
        return "absent"
    if not _jsonc_ok(text):
        raise SystemExit("%s is not valid JSON (with comments); SpeakEasy will not edit it. "
                         "Remove its %s entry by hand" % (path, SERVER))
    new = _zed_remove_text(text)
    if new is None:
        return "absent"
    if not _jsonc_ok(new):
        raise SystemExit("%s: SpeakEasy could not remove its %s entry safely; remove it by hand"
                         % (path, SERVER))
    backup(path)
    write(path, new)
    return "removed"


# --------------------------------------------------------------------------- #
# YAML configs (goose, hermes)
# --------------------------------------------------------------------------- #

def add_goose(path, cmd):
    text = read(path)
    if text is None:
        text = ""
    if re.search(r"^  %s:" % re.escape(SERVER), text, re.M):
        return "already"
    backup(path)
    # timeout is seconds and goose cancels the call when it expires; a
    # request_slack_message call blocks on a human approval for minutes
    block = (
        "  %s:\n"
        "    enabled: true\n"
        "    type: stdio\n"
        "    cmd: %s\n"
        "    args: []\n"
        "    envs: {}\n"
        "    timeout: 86400\n" % (SERVER, json.dumps(cmd))
    )
    m = re.search(r"^extensions:[ \t]*$", text, re.M)
    if m:
        # (`[ \t]*`, not `\s*`: the match must stop before the newline)
        rest = text[m.end():]
        text = text[: m.end()] + "\n" + block + (rest[1:] if rest.startswith("\n") else rest)
    else:
        if text and not text.endswith("\n"):
            text += "\n"
        text += "extensions:\n" + block
    write(path, text)
    return "added"


def set_goose_timeout(path, name, seconds):
    """Set the timeout: line in the gateway's YAML block (the `  name:`
    entry under goose's extensions: or Hermes's mcp_servers:), adding it
    after the block's last key if it is missing. Only that line changes;
    the rest of the file — comments, other entries — is kept byte-for-byte."""
    text = read(path)
    if text is None:
        return "absent"
    m = re.search(r"^  %s:[^\n]*\n((?:    [^\n]*(?:\n|\Z)|\n)*)" % re.escape(name), text, re.M)
    if not m:
        return "absent"
    # the block must live under extensions: (goose's MCP servers key)
    parent = None
    for line in text[: m.start()].splitlines():
        pm = re.match(r"^([A-Za-z0-9_.-]+)\s*:", line)
        if pm:
            parent = pm.group(1)
    if parent not in ("extensions", "mcp_servers"):
        return "absent"
    block = m.group(1)
    tm = re.search(r"^([ \t]+)timeout:[^\n]*\n?", block, re.M)
    if tm:
        had_nl = block[tm.end() - 1] == "\n"
        repl = "%stimeout: %d" % (tm.group(1), seconds) + ("\n" if had_nl else "")
        if block[tm.start():tm.end()] == repl:
            return "unchanged"
        block = block[: tm.start()] + repl + block[tm.end():]
        result = "updated"
    else:
        block_lines = block.splitlines(keepends=True)
        insert_at = len(block_lines)
        for i in range(len(block_lines) - 1, -1, -1):
            if block_lines[i].strip():
                insert_at = i + 1
                break
        if insert_at and not block_lines[insert_at - 1].endswith("\n"):
            block_lines[insert_at - 1] += "\n"
        block_lines.insert(insert_at, "    timeout: %d\n" % seconds)
        block = "".join(block_lines)
        result = "added"
    backup(path)
    write(path, text[: m.start(1)] + block + text[m.end(1):])
    return result


def add_hermes(path, cmd):
    text = read(path)
    if text is None:
        text = ""
    if SERVER in text:
        return "already"
    backup(path)
    # timeout: seconds, for tool calls (default 300); an integer
    entry = (
        "  %s:\n"
        "    command: %s\n"
        "    args: []\n"
        "    env: {}\n"
        "    timeout: %d\n" % (SERVER, json.dumps(cmd), TOOL_TIMEOUT_SEC)
    )
    m = re.search(r"^mcp_servers:[ \t]*$", text, re.M)
    if m:
        rest = text[m.end():]
        text = text[: m.end()] + "\n" + entry + (rest[1:] if rest.startswith("\n") else rest)
    else:
        if text and not text.endswith("\n"):
            text += "\n"
        text += "mcp_servers:\n" + entry
    write(path, text)
    return "added"


def _remove_yaml_block(path, header_re, name):
    text = read(path)
    if text is None:
        return "absent"
    # Remove the `  name:` entry (2-space indent) and its deeper-indented
    # children, plus a now-empty parent header if it was the only entry.
    m = re.search(r"^  %s:[^\n]*(?:\n|\Z)(?:    [^\n]*(?:\n|\Z)|\n)*" % re.escape(name), text, re.M)
    if not m:
        return "absent"
    text = text[: m.start()] + text[m.end():]
    backup(path)
    write(path, text)
    return "removed"


def remove_goose(path):
    return _remove_yaml_block(path, None, SERVER)


def remove_hermes(path):
    return _remove_yaml_block(path, None, SERVER)


# --------------------------------------------------------------------------- #
# Codex (TOML)
# --------------------------------------------------------------------------- #

def add_codex(path, cmd):
    text = read(path)
    if text is None:
        text = ""
    if any(_codex_server_header(SERVER).match(l) for l in text.splitlines()):
        return "already"
    backup(path)
    if text and not text.endswith("\n"):
        text += "\n"
    # tool_timeout_sec: a request_slack_message call blocks on a human
    # approval for minutes; codex's default per-tool timeout is 60s
    text += (
        '\n[mcp_servers.%s]\ncommand = %s\nargs = []\ntool_timeout_sec = 86400\n'
        % (SERVER, json.dumps(cmd))
    )
    write(path, text)
    return "added"


def remove_codex(path):
    return remove_codex_entry(path, SERVER)


def _codex_server_header(name):
    """A [mcp_servers.NAME] table header, NAME bare or quoted."""
    n = re.escape(name)
    return re.compile(r"""^\[\s*mcp_servers\s*\.\s*(?:%s|"%s"|'%s')\s*\][ \t]*(#.*)?$""" % (n, n, n))


def _cut_lines(lines, start, end):
    """Remove lines[start:end], keeping comment/blank lines that follow the
    region's last key line: our writers append at EOF, so a comment after
    the last key of a removed table/element is the user's, not ours."""
    region = lines[start:end]
    last = len(region)
    while last > 0 and (
        not region[last - 1].strip() or region[last - 1].lstrip().startswith("#")
    ):
        last -= 1
    head = "".join(lines[:start])
    kept = "".join(region[last:])
    tail = "".join(lines[end:])
    # the removal can leave a run of blank lines where the entry was:
    # collapse it at the seam only (the rest of the file stays as it is)
    seam = head[len(head.rstrip("\n")):] + kept + tail[:len(tail) - len(tail.lstrip("\n"))]
    middle = re.sub(r"\n{3,}", "\n\n", seam)
    new = head.rstrip("\n") + middle + tail.lstrip("\n")
    if not (kept + tail).strip():
        new = new.rstrip("\n") + "\n" if new.strip() else ""
    return new


def remove_codex_entry(path, name):
    """Remove a `[mcp_servers.<name>]` table (and its subtables, e.g.
    `[mcp_servers.<name>.env]`) from a TOML config. install.sh --enforce uses
    this to drop direct Slack servers from codex configs."""
    text = read(path)
    if text is None:
        return "absent"
    lines = text.splitlines(keepends=True)
    tables = _toml_tables(text)
    # the [mcp_servers.NAME] table plus its [mcp_servers.NAME.*] subtables,
    # which directly follow it
    header_re = _codex_server_header(name)
    sub_re = re.compile(r"^\[mcp_servers\.%s\.[^\]\n]+\][ \t]*$" % re.escape(name))
    start = None
    end = None
    for i, t in enumerate(tables):
        if header_re.match(t["header"].strip()):
            start = i
            end = i + 1
            while end < len(tables) and sub_re.match(tables[end]["header"].strip()):
                end += 1
            break
    if start is None:
        return "absent"
    backup(path, force=(name != SERVER))
    write(path, _cut_lines(lines, tables[start]["start"], tables[end - 1]["end"]))
    return "removed"


# --------------------------------------------------------------------------- #
# Mistral Vibe (TOML array tables)
#
# Vibe stores MCP servers as [[mcp_servers]] array elements carrying
# name = "..."; a [mcp_servers.KEY] table that follows an element (e.g.
# [mcp_servers.env]) is a sub-table of that element, not a server.
# --------------------------------------------------------------------------- #

# a tool call can block on a human approval for minutes; vibe's default
# tool_timeout_sec is 60s
TOOL_TIMEOUT_SEC = 86400

_VIBE_ARRAY_RE = re.compile(r"^\[\[mcp_servers\]\][ \t]*$")
_VIBE_SUB_RE = re.compile(r"^\[mcp_servers\.[A-Za-z0-9_.-]+\][ \t]*$")
_VIBE_NAME_RE = re.compile(r'^\s*name\s*=\s*"([^"]+)"', re.M)


def _toml_tables(text):
    """Split a TOML file into top-level table chunks. A chunk runs from its
    column-0 header line ([table] or [[array]]) to the next column-0 header
    (or EOF): {"header", "start", "end", "body"} — start/end are line
    indexes into text.splitlines(keepends=True); body excludes the header
    line and the text of the chunk is header + body."""
    lines = text.splitlines(keepends=True)
    tables = []
    start = None
    for i, line in enumerate(lines):
        if line.startswith("["):
            if start is not None:
                tables.append((start, i))
            start = i
    if start is not None:
        tables.append((start, len(lines)))
    return [
        {
            "header": lines[s],
            "start": s,
            "end": e,
            "body": "".join(lines[s + 1 : e]),
        }
        for s, e in tables
    ]


def _vibe_element_name(table):
    """The name = "..." of a [[mcp_servers]] element, or None."""
    m = _VIBE_NAME_RE.search(table["body"])
    return m.group(1) if m else None


def add_vibe(path, cmd):
    """Append a [[mcp_servers]] element (Mistral Vibe's array style)."""
    text = read(path)
    if text is None:
        text = ""
    for t in _toml_tables(text):
        if _VIBE_ARRAY_RE.match(t["header"]) and _vibe_element_name(t) == SERVER:
            return "already"
    backup(path)
    if text and not text.endswith("\n"):
        text += "\n"
    if text.strip():
        text += "\n"
    text += (
        "[[mcp_servers]]\n"
        'name = "%s"\n'
        'transport = "stdio"\n'
        'command = %s\n'
        "args = []\n"
        "tool_timeout_sec = %d\n" % (SERVER, json.dumps(cmd), TOOL_TIMEOUT_SEC)
    )
    write(path, text)
    return "added"


def remove_vibe_entry(path, name):
    """Remove one [[mcp_servers]] element (matched by its name = "<name>")
    and the [mcp_servers.KEY] subtables that belong to it. install.sh
    --enforce uses this to drop direct Slack servers from vibe configs;
    --uninstall uses it for the gateway's own element."""
    text = read(path)
    if text is None:
        return "absent"
    lines = text.splitlines(keepends=True)
    tables = _toml_tables(text)
    target = None
    for i, t in enumerate(tables):
        if _VIBE_ARRAY_RE.match(t["header"]) and _vibe_element_name(t) == name:
            target = i
            break
    if target is None:
        return "absent"
    # The element's region runs to the next header that is not one of its
    # [mcp_servers.KEY] subtables (the next [[mcp_servers]] element or any
    # other table).
    end = len(tables)
    for j in range(target + 1, len(tables)):
        if not _VIBE_SUB_RE.match(tables[j]["header"]):
            end = j
            break
    region_end = tables[end]["start"] if end < len(tables) else len(lines)
    backup(path, force=(name != SERVER))
    write(path, _cut_lines(lines, tables[target]["start"], region_end))
    return "removed"


def remove_vibe(path):
    return remove_vibe_entry(path, SERVER)


def _set_toml_key(body_lines, key, value):
    """Set key = value in a table body (a list of lines). Returns
    (new_lines, existed)."""
    pat = re.compile(r"^%s\s*=" % re.escape(key))
    for i, line in enumerate(body_lines):
        if pat.match(line):
            body_lines[i] = "%s = %s\n" % (key, value)
            return body_lines, True
    insert_at = len(body_lines)
    for i in range(len(body_lines) - 1, -1, -1):
        if body_lines[i].strip():
            insert_at = i + 1
            break
    body_lines.insert(insert_at, "%s = %s\n" % (key, value))
    return body_lines, False


def set_toml_timeout(path, name, seconds):
    """Set tool_timeout_sec = seconds in the [mcp_servers.NAME] table
    (codex style) or the [[mcp_servers]] element named NAME (vibe style)."""
    text = read(path)
    if text is None:
        return "absent"
    lines = text.splitlines(keepends=True)
    value = str(seconds)
    # codex style: a [mcp_servers.NAME] table (its body runs to the next
    # column-0 header, so subtables are not touched)
    pat = _codex_server_header(name)
    for i, line in enumerate(lines):
        if pat.match(line):
            j = i + 1
            while j < len(lines) and not lines[j].startswith("["):
                j += 1
            body, existed = _set_toml_key(lines[i + 1 : j], "tool_timeout_sec", value)
            new = "".join(lines[: i + 1] + body + lines[j:])
            if new == text:
                return "unchanged"
            backup(path)
            write(path, new)
            return "updated" if existed else "added"
    # vibe style: a [[mcp_servers]] element with name = "NAME"
    for t in _toml_tables(text):
        if _VIBE_ARRAY_RE.match(t["header"]) and _vibe_element_name(t) == name:
            body_lines = t["body"].splitlines(keepends=True)
            body, existed = _set_toml_key(body_lines, "tool_timeout_sec", value)
            if name == SERVER and not any(re.match(r"\s*transport\s*=", l) for l in body):
                # an element written by an older installer: Vibe rejects the
                # whole config without a transport
                body = [body[0]] + ['transport = "stdio"\n'] + body[1:] if body else ['transport = "stdio"\n']
            new = "".join(lines[: t["start"] + 1] + body + lines[t["end"] :])
            if new == text:
                return "unchanged"
            backup(path)
            write(path, new)
            return "updated" if existed else "added"
    return "absent"


def set_timeout(path, name, seconds):
    """set-timeout: give the entry a long tool-call timeout — tool_timeout_sec
    in a TOML entry (codex [mcp_servers.NAME] table, vibe [[mcp_servers]]
    element) or the timeout: line in a goose YAML extensions block. A
    request_slack_message call blocks on a human approval for minutes."""
    result = set_toml_timeout(path, name, seconds)
    if result != "absent":
        return result
    return set_goose_timeout(path, name, seconds)


# --------------------------------------------------------------------------- #
# instruction blocks
# --------------------------------------------------------------------------- #

def _block_span(text):
    """(start, end) of SpeakEasy's instruction block, or None when there is
    none; an unmatched begin marker (end marker missing, or only before it)
    is an error, never a guess at where the block ends."""
    if MARK_BEGIN not in text:
        return None
    start = text.index(MARK_BEGIN)
    end = text.find(MARK_END, start)
    if end < 0:
        raise SystemExit("SpeakEasy's instruction block has a begin marker but no end marker "
                         "after it (%s); fix or delete the block by hand" % MARK_END)
    return start, end + len(MARK_END)


def instruction_add(path, variant="gateway"):
    text = read(path)
    created = text is None
    if text is None:
        text = ""
    body = {"hooks": INSTRUCTION_HOOKS, "either": INSTRUCTION_EITHER,
            "confirm": INSTRUCTION_CONFIRM}.get(variant, INSTRUCTION)
    block = MARK_BEGIN + "\n\n" + body + "\n\n" + MARK_END + "\n"
    try:
        span = _block_span(text)
    except SystemExit as e:
        raise SystemExit("%s: %s" % (path, e))
    if span:
        start, end = span
        new = text[:start] + block.rstrip("\n") + text[end:]
        if new == text:
            return "unchanged"
        write(path, new)
        return "updated"
    backup(path)
    if text and not text.endswith("\n"):
        text += "\n"
    if text.strip():
        text += "\n"
    write(path, text + block)
    return "added" if not created else "created"


def _only_our_block(text):
    """Whether an instruction file holds nothing but SpeakEasy's block."""
    try:
        span = _block_span(text)
    except SystemExit:
        return False
    return bool(span) and not (text[:span[0]] + text[span[1]:]).strip()


def instruction_remove(path):
    # An older installer could back up a file that held only SpeakEasy's
    # block; that "backup" is not the user's original (dropped even when the
    # file itself is gone or no longer has the block).
    if _only_our_block(read(path + BAK) or ""):
        os.remove(path + BAK)
    text = read(path)
    if text is None or MARK_BEGIN not in text:
        return "absent"
    try:
        start, end = _block_span(text)
    except SystemExit as e:
        raise SystemExit("%s: %s" % (path, e))
    text = text[:start] + text[end:]
    text = re.sub(r"\n{3,}", "\n\n", text).strip("\n")
    if text:
        write(path, text + "\n")
        return "removed"
    # The file only contained our block: remove it entirely.
    os.remove(path)
    return "removed (file was ours; deleted)"


# --------------------------------------------------------------------------- #
# audit
# --------------------------------------------------------------------------- #

def _mask(token):
    return token[:12] + "…" + token[-4:]


def _yaml_server_blocks(text):
    """Yield (parent, name, block_text) for 2-space-indented `name:` entries
    in a YAML-ish config (the block runs to the next entry at the same
    indent; parent is the nearest indent-0 key)."""
    lines = text.splitlines()
    parent = None
    i = 0
    while i < len(lines):
        line = lines[i]
        m0 = re.match(r"^([A-Za-z0-9_.-]+)\s*:", line)
        if m0:
            parent = m0.group(1)
            i += 1
            continue
        m = re.match(r"^  ([A-Za-z0-9_.-]+)\s*:", line)
        if not m:
            i += 1
            continue
        name = m.group(1)
        j = i + 1
        while j < len(lines):
            nxt = lines[j]
            if nxt.strip() == "":
                j += 1
                continue
            indent = len(nxt) - len(nxt.lstrip())
            if indent <= 2:
                break
            j += 1
        yield parent, name, "\n".join(lines[i:j])
        i = j


_TOML_TABLE_RE = re.compile(r"^\[mcp_servers\.([A-Za-z0-9_-]+)(?:\.[^\]\n]*)?\][ \t]*$")


def _toml_server_blocks(text):
    """Yield (name, block_text) for the MCP servers in a TOML config, in
    both styles agents use:

      codex: [mcp_servers.NAME] tables, with subtables
             ([mcp_servers.NAME.env]) folded into NAME's block.
      vibe:  [[mcp_servers]] array elements with name = "..."; the
             [mcp_servers.KEY] subtables that follow an element belong to
             it and are folded into its block (they are not servers).

    A file containing [[mcp_servers]] elements is treated as vibe-style:
    its [mcp_servers.X] headers are then subtables of the current element,
    not servers (so [mcp_servers.env] is never reported as a server "env").
    """
    tables = _toml_tables(text)
    vibe_style = any(_VIBE_ARRAY_RE.match(t["header"]) for t in tables)
    blocks = {}
    order = []
    current = None
    for t in tables:
        header = t["header"].strip()
        if _VIBE_ARRAY_RE.match(header):
            current = _vibe_element_name(t)
            if current is not None:
                if current not in blocks:
                    blocks[current] = []
                    order.append(current)
                blocks[current].append(t["header"] + t["body"])
            continue
        m = _TOML_TABLE_RE.match(header)
        if m:
            if vibe_style:
                # a [mcp_servers.KEY] subtable of the current element
                if current is not None:
                    blocks[current].append(t["header"] + t["body"])
                continue
            name = m.group(1)
            if name not in blocks:
                blocks[name] = []
                order.append(name)
            blocks[name].append(t["header"] + t["body"])
            continue
        # any other table header ends the current vibe element
        current = None
    for name in order:
        yield name, "".join(blocks[name])


def audit_file(path):
    """Audit one agent config for the exclusivity boundary.

    Reports, per file:
      slack_mcp_servers          the official Slack MCP server (OAuth, remote)
                                 — expected in agents (reading Slack context)
      mistral_connector_servers  Mistral MCP/connector servers — expected in
                                 agents (they expose the Slack connector)
      direct_slack_servers       direct/token-based Slack servers and other
                                 agent-side Slack paths the gateway does not
                                 control (hermes platform_toolsets, …)
      slack_tokens               masked Slack tokens (xox…) in the file
      mistral_keys               masked Mistral API keys (mstrl_…) in the
                                 file — an agent with one can call the Slack
                                 connector's send tool directly

    Server entries are {"name": ..., "key": [...]} objects: key is the JSON
    path of the servers container (so nested per-project entries, e.g.
    claude projects.<path>.mcpServers, are located and removable), the YAML
    parent key, or [] for TOML tables/elements.
    """
    text = read(path)
    if text is None:
        return None
    finding = {
        "file": path,
        "slack_mcp_servers": [],
        "mistral_connector_servers": [],
        "direct_slack_servers": [],
        # Slack integrations that are not MCP servers (Hermes's
        # platform_toolsets.slack): reported, never removed by --enforce
        "other_slack_paths": [],
        "slack_tokens": [],
        "mistral_keys": [],
    }
    for m in TOKEN_RE.finditer(text):
        finding["slack_tokens"].append(_mask(m.group(0)))
    for m in MISTRAL_KEY_RE.finditer(text):
        finding["mistral_keys"].append(_mask(m.group(0)))

    def classify(name, entry_text, key):
        bucket = {
            "slack-mcp": "slack_mcp_servers",
            "mistral": "mistral_connector_servers",
            "direct": "direct_slack_servers",
        }.get(classify_server(name, entry_text))
        if bucket and not any(
            e["name"] == name and e["key"] == key for e in finding[bucket]
        ):
            finding[bucket].append({"name": name, "key": key})

    try:
        data = json.loads(strip_jsonc(text))
    except ValueError:
        data = None
    if isinstance(data, dict):
        def scan(obj, keys):
            if isinstance(obj, dict):
                for k, v in obj.items():
                    if (
                        keys
                        and keys[-1] in MCP_PARENT_KEYS
                        and isinstance(v, dict)
                        and k != SERVER
                    ):
                        classify(k, json.dumps(v), keys)
                    scan(v, keys + [k])
            elif isinstance(obj, list):
                for v in obj:
                    scan(v, keys)

        scan(data, [])
    else:
        # YAML / plain text. Only MCP-server-shaped entries are classified
        # (under an MCP parent key, or carrying url/command/cmd) — LLM
        # provider entries like `providers.mistral` are not Slack paths.
        for parent, name, block in _yaml_server_blocks(text):
            if name == SERVER:
                continue
            if parent in YAML_MCP_PARENTS or re.search(
                r"^\s+(url|command|cmd)\s*:", block, re.M
            ):
                classify(name, block, [parent] if parent else [])
            elif "slack" in name.lower():
                # A slack-named key that is not an MCP server entry (e.g.
                # hermes platform_toolsets.slack): still an agent-side Slack
                # path SpeakEasy does not control, but not a server --enforce
                # could remove.
                key = [parent] if parent else []
                if not any(
                    e["name"] == name and e["key"] == key
                    for e in finding["other_slack_paths"]
                ):
                    finding["other_slack_paths"].append({"name": name, "key": key})
    # TOML, both styles: codex [mcp_servers.<name>] tables and vibe
    # [[mcp_servers]] elements (a .env subtable is part of its server, not
    # a server of its own).
    for name, block in _toml_server_blocks(text):
        if name != SERVER:
            classify(name, block, [])
    return finding


# --------------------------------------------------------------------------- #
# take over an existing Slack MCP (install.sh): SpeakEasy reuses the Slack
# MCP server the user already set up for an agent — its command or URL and
# its credentials — instead of asking for a new Slack app or token
# --------------------------------------------------------------------------- #

# Server settings SpeakEasy must control itself: its own caches (in its
# state dir), and the tool list (it needs conversations_add_message).
TAKEOVER_ENV_DROP = {
    "SLACK_MCP_USERS_CACHE",
    "SLACK_MCP_CHANNELS_CACHE",
    "SLACK_MCP_ENABLED_TOOLS",
}
# What install.sh writes when no credential is given.
PLACEHOLDERS = {
    "xoxb-placeholder-get-a-real-token",
    "xoxp-placeholder-get-a-real-token",
    "mistral-placeholder-set-a-real-key",
}
# A Mistral connector's MCP/API URL; group 1 is the connector name or id.
_MISTRAL_CONNECTOR_URL_RE = re.compile(
    r"api\.mistral\.ai/v1/connectors(?:-gateway)?/([^/?#\s]+)", re.I
)
_ENV_NAME_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
# korotovsky lists conversations_add_message for any non-empty
# SLACK_MCP_ADD_MESSAGE_TOOL, but only "true" and "1" allow every channel;
# any other value is read as a channel-ID list, so "false" or "yes" deny
# every send. Only a real ID list (C…/D…/G…, optionally "!"-negated) is
# kept from a taken-over config; anything else falls back to SpeakEasy's
# own "true".
_SLACK_ID_RE = re.compile(r"[CDG][A-Z0-9]{6,}")
_PACKAGE_RUNNERS = ("npx", "bunx", "pnpx", "uvx")
_CONTAINER_RUNNERS = ("docker", "podman", "nerdctl")


def _literal(value):
    """The value when it is a literal secret, None when it is empty or an
    environment reference the agent resolves at runtime: ${VAR}, $VAR, or
    OpenCode's {env:VAR} / {file:path}."""
    v = ("" if value is None else str(value)).strip()
    if not v or v.startswith("$") or "${" in v or re.search(r"\{(env|file):", v):
        return None
    return v


def _authorization(headers):
    for k, v in (headers or {}).items():
        if str(k).lower() == "authorization":
            return str(v)
    return None


def _bearer(headers):
    m = re.match(r"^\s*Bearer\s+(\S+)\s*$", _authorization(headers) or "")
    return _literal(m.group(1)) if m else None


def _send_policy_to_keep(value):
    items = [i.strip().lstrip("!") for i in value.split(",") if i.strip()]
    return bool(items) and all(_SLACK_ID_RE.fullmatch(i) for i in items)


def _normalize_entry(name, v, key=None):
    """One MCP server entry as {"name", "key", "argv", "env", "url",
    "headers"}, whatever the agent's config shape: command + args (most
    agents), a command array (opencode), a whole command line in one string
    (Cursor deeplinks), {"command": {"path", "args", "env"}} (zed), env /
    environment / envs, and url / serverUrl / httpUrl / uri. key is the JSON
    path of the servers container ([] for TOML and YAML)."""
    if not isinstance(v, dict):
        return None
    cmd = v.get("command", v.get("cmd"))
    args = v.get("args") or []
    env = v.get("env") or v.get("environment") or v.get("envs") or {}
    if isinstance(cmd, dict):
        args = cmd.get("args") or args
        env = cmd.get("env") or env
        cmd = cmd.get("path")
    if isinstance(cmd, list):
        argv = [str(c) for c in cmd] + [str(a) for a in args]
    elif isinstance(cmd, str) and cmd.strip():
        # A whole command line in one string (Cursor deeplinks) is split on
        # spaces — unless its first word is an absolute path that does not
        # exist, i.e. the string is a program path with spaces in it.
        first = cmd.split()[0]
        one_line = not args and " " in cmd.strip() and (
            not os.path.isabs(first) or os.path.exists(first)
        )
        argv = cmd.split() if one_line else [cmd]
        argv += [str(a) for a in args]
    else:
        argv = []
    url = v.get("url") or v.get("serverUrl") or v.get("httpUrl") or v.get("uri") or ""
    headers = v.get("headers") or v.get("http_headers") or {}
    # codex: the bearer token read from an environment variable
    env_headers = v.get("env_http_headers") or {}
    token_env = bool(v.get("bearer_token_env_var")) or (
        isinstance(env_headers, dict) and any(str(h).lower() == "authorization" for h in env_headers)
    )
    env = {str(k): str(x) for k, x in env.items()} if isinstance(env, dict) else {}
    headers = headers if isinstance(headers, dict) else {}
    if not url:
        # A remote server reached through a local bridge (mcp-remote,
        # mcp-proxy, …): `npx mcp-remote https://mcp.slack.com/mcp --header
        # "Authorization: Bearer ${SLACK_TOKEN}"`, with the variable in env.
        url, bridged = _bridged_remote(argv, env)
        if url:
            headers = dict(bridged, **headers)
            token_env = token_env or any(
                "$" in str(v) and _literal(str(v)) is None for v in bridged.values())
    return {
        "name": str(name),
        "key": list(key or []),
        "token_env": token_env,
        "argv": argv,
        "env": env,
        "url": str(url),
        "headers": headers,
    }


def _bridged_remote(argv, env):
    """(url, headers) of a remote MCP server reached through a stdio bridge
    command, or ("", {}). `${VAR}` / `$VAR` in a header value are filled in
    from the entry's own literal env; a value that still has a variable is
    left as is (it comes from the environment)."""
    url = next((a for a in argv if re.match(r"^https?://", a)), "")
    if not url:
        return "", {}
    headers = {}
    for i, a in enumerate(argv):
        value = None
        if a == "--header" and i + 1 < len(argv):
            value = argv[i + 1]
        elif a.startswith("--header="):
            value = a[len("--header="):]
        if not value or ":" not in value:
            continue
        hname, hval = value.split(":", 1)

        def fill(m):
            lit = _literal(env.get(m.group(1) or m.group(2), "")) if (m.group(1) or m.group(2)) in env else None
            return lit if lit is not None else m.group(0)

        headers[hname.strip()] = re.sub(r"\$\{([A-Za-z_][A-Za-z0-9_]*)\}|\$([A-Za-z_][A-Za-z0-9_]*)",
                                        fill, hval.strip())
    return url, headers


def _yaml_scalar(text):
    """A YAML scalar: a quoted value up to its closing quote (anything after
    it, such as a # comment, is dropped), or a plain value without a
    trailing # comment."""
    t = text.strip()
    if t[:1] == '"':
        m = re.match(r'"((?:[^"\\]|\\.)*)"', t)
        if m:
            return m.group(1).replace('\\"', '"')
    elif t[:1] == "'":
        m = re.match(r"'((?:[^']|'')*)'", t)
        if m:
            return m.group(1).replace("''", "'")
    return re.sub(r"\s+#.*$", "", t)


def _yaml_entry(name, block, key=None):
    """A goose/hermes 2-space-indented server block as a dict: 4-space keys
    with scalar, inline-list or block (list / mapping) values. Block list
    items may sit at the key's own indent (serde_yaml and PyYAML write them
    that way) or deeper."""
    d = {}
    k = None
    for line in block.splitlines()[1:]:
        if not line.strip() or line.strip().startswith("#"):
            continue
        indent = len(line) - len(line.lstrip())
        stripped = line.strip()
        is_item = stripped == "-" or stripped.startswith("- ")
        if is_item and k and (d.get(k) is None or isinstance(d.get(k), list)):
            if not isinstance(d.get(k), list):
                d[k] = []
            item = stripped[1:].strip()
            if item:
                d[k].append(_yaml_scalar(item))
            continue
        if indent <= 4:
            m = re.match(r"^([A-Za-z0-9_.-]+)\s*:\s*(.*)$", stripped)
            k = m.group(1) if m else None
            if not m:
                continue
            val = m.group(2).strip()
            if val[:1] not in ("'", '"'):
                val = re.sub(r"\s+#.*$", "", val)
            if val == "":
                d[k] = None
            elif val.startswith("["):
                d[k] = [_yaml_scalar(x) for x in re.findall(r"\"[^\"]*\"|'[^']*'|[^,\[\]\s][^,\[\]]*", val)]
            elif val == "{}":
                d[k] = {}
            else:
                d[k] = _yaml_scalar(val)
        elif k:
            m = re.match(r"^([A-Za-z0-9_.-]+)\s*:\s*(.*)$", stripped)
            if m:
                if not isinstance(d.get(k), dict):
                    d[k] = {}
                d[k][m.group(1)] = _yaml_scalar(m.group(2))
    return _normalize_entry(name, d, key)


def _toml_loader():
    """tomllib (Python 3.11+), else tomli, else the copy pip vendors (it is
    there on macOS's /usr/bin/python3 3.9); None when none is available."""
    try:
        import tomllib
        return tomllib
    except ImportError:
        pass
    try:
        import tomli
        return tomli
    except ImportError:
        pass
    try:
        from pip._vendor import tomli
        return tomli
    except ImportError:
        return None


def _config_entries(path):
    """Every MCP server entry in an agent config (JSON/JSONC, TOML, or the
    goose/hermes YAML), normalized; the gateway's own entry excluded.
    Returns None for a TOML file no TOML parser is available for."""
    text = read(path)
    if text is None:
        return []
    entries = []
    try:
        data = json.loads(strip_jsonc(text))
    except ValueError:
        data = None
    if isinstance(data, dict):
        def scan(obj, keys):
            if isinstance(obj, dict):
                for k, v in obj.items():
                    # VS Code keeps servers under a top-level "servers" key
                    # (mcp.json) or "mcp": {"servers": …} (settings.json)
                    container = keys and (
                        keys[-1] in MCP_PARENT_KEYS or keys in (["servers"], ["mcp", "servers"])
                    )
                    if container and isinstance(v, dict) and k != SERVER:
                        e = _normalize_entry(k, v, keys)
                        if e:
                            entries.append(e)
                    scan(v, keys + [k])
            elif isinstance(obj, list):
                for v in obj:
                    scan(v, keys)

        scan(data, [])
        return entries
    if path.endswith(".toml"):
        toml = _toml_loader()
        if toml is None:
            return None
        try:
            data = toml.loads(text)
        except ValueError:
            return []
        servers = data.get("mcp_servers")
        if isinstance(servers, dict):
            items = list(servers.items())
        elif isinstance(servers, list):
            items = [(s.get("name"), s) for s in servers if isinstance(s, dict)]
        else:
            items = []
        for name, v in items:
            if name and name != SERVER:
                e = _normalize_entry(name, v)
                if e:
                    entries.append(e)
        return entries
    for parent, name, block in _yaml_server_blocks(text):
        if name != SERVER and parent in YAML_MCP_PARENTS:
            e = _yaml_entry(name, block, [parent])
            if e:
                entries.append(e)
    return entries


def _korotovsky_tokens(env):
    """The usable logins in a korotovsky server's env: literal values with
    the right token prefix (xoxp-, xoxb-, or the xoxc- + xoxd- pair)."""
    lit = {k: _literal(v) for k, v in env.items()}
    shaped = lambda k, prefix: (lit.get(k) or "").startswith(prefix)
    kinds = []
    if shaped("SLACK_MCP_XOXP_TOKEN", "xoxp-"):
        kinds.append("xoxp")
    if shaped("SLACK_MCP_XOXB_TOKEN", "xoxb-"):
        kinds.append("xoxb")
    if shaped("SLACK_MCP_XOXC_TOKEN", "xoxc-") and shaped("SLACK_MCP_XOXD_TOKEN", "xoxd-"):
        kinds.append("xoxc+xoxd")
    return kinds


def _korotovsky_no_token_reason(env):
    """Why a korotovsky server's env holds no usable login."""
    keys = ("SLACK_MCP_XOXP_TOKEN", "SLACK_MCP_XOXB_TOKEN", "SLACK_MCP_XOXC_TOKEN", "SLACK_MCP_XOXD_TOKEN")
    present = [k for k in keys if str(env.get(k) or "").strip()]
    refs = [k for k in present if _literal(env[k]) is None]
    if refs:
        return ("its token comes from an environment variable (%s), not a literal "
                "value in the config, so SpeakEasy can't copy it" % ", ".join(refs))
    lit = {k: _literal(env[k]) for k in present}
    if "SLACK_MCP_XOXC_TOKEN" in lit and "SLACK_MCP_XOXD_TOKEN" not in lit:
        return "it has the browser-session token SLACK_MCP_XOXC_TOKEN but not its SLACK_MCP_XOXD_TOKEN cookie"
    if "SLACK_MCP_XOXD_TOKEN" in lit and "SLACK_MCP_XOXC_TOKEN" not in lit:
        return "it has the SLACK_MCP_XOXD_TOKEN cookie but not the browser-session token SLACK_MCP_XOXC_TOKEN"
    wrong = [k for k in lit if not lit[k].startswith(k[len("SLACK_MCP_"):].split("_")[0].lower() + "-")]
    if wrong:
        return "%s does not hold a token of that kind (wrong prefix)" % ", ".join(wrong)
    return ("no literal Slack token in its env (SLACK_MCP_XOXP_TOKEN, "
            "SLACK_MCP_XOXB_TOKEN, or SLACK_MCP_XOXC_TOKEN + SLACK_MCP_XOXD_TOKEN)")


def _fingerprint(e, transport):
    """A hash of what take-over would copy: two entries with the same
    fingerprint are the same server (the same config pasted into several
    agents or project scopes). Hashed, because candidates are printed."""
    if transport == "korotovsky":
        env = sorted((k, _literal(v)) for k, v in e["env"].items()
                     if k not in TAKEOVER_ENV_DROP and _literal(v) is not None)
        parts = ["korotovsky", e["argv"], env]
    elif transport == "slack-mcp":
        parts = ["slack-mcp", _bearer(e["headers"])]
    else:
        m = _MISTRAL_CONNECTOR_URL_RE.search(e["url"])
        parts = ["mistral-connector", m.group(1) if m else "", _bearer(e["headers"])]
    return hashlib.sha256(json.dumps(parts).encode()).hexdigest()[:16]


def _candidate(path, e):
    """The take-over candidate for one entry, or None when it is not a
    Slack MCP. Each has: file, name, transport, importable, reason,
    summary (secrets masked), fingerprint."""
    base = {"file": path, "name": e["name"]}
    url = e["url"]
    argv_text = " ".join(e["argv"]).lower()
    if "mcp.slack.com" in url.lower():
        token = _bearer(e["headers"])
        auth = _authorization(e["headers"])
        if token and token.startswith("xoxp-"):
            c = dict(base, transport="slack-mcp", importable=True, reason="",
                     summary="official Slack MCP server, user token %s" % _mask(token))
        elif token:
            c = dict(base, transport="slack-mcp", importable=False,
                     reason="its token is not a Slack user token (xoxp-)",
                     summary="official Slack MCP server")
        elif auth or e.get("token_env"):
            c = dict(base, transport="slack-mcp", importable=False,
                     reason=("its token comes from an environment variable, not the config; "
                             "put the xoxp- token in the config or pass --slack-mcp-token"),
                     summary="official Slack MCP server")
        else:
            c = dict(base, transport="slack-mcp", importable=False,
                     reason="it signs in with OAuth inside its agent; SpeakEasy cannot reuse that session",
                     summary="official Slack MCP server")
    else:
        m = _MISTRAL_CONNECTOR_URL_RE.search(url)
        if m:
            ref = m.group(1)
            if "slack" not in ref.lower():
                return None
            key = _bearer(e["headers"])
            if key:
                c = dict(base, transport="mistral-connector", importable=True, reason="",
                         summary="Mistral connector %s, API key %s" % (ref, _mask(key)))
            else:
                c = dict(base, transport="mistral-connector", importable=False,
                         reason="no literal Mistral API key in its headers",
                         summary="Mistral connector %s" % ref)
        elif ("slack-mcp-server" in argv_text or "korotovsky" in argv_text
                or any(k.startswith("SLACK_MCP_XOX") for k in e["env"])):
            kinds = _korotovsky_tokens(e["env"])
            summary = "korotovsky slack-mcp-server (%s)" % (", ".join(kinds) or "no usable login")
            if not e["argv"]:
                reason = "it runs as a remote (HTTP/SSE) server; SpeakEasy needs its stdio command"
            elif any(re.search(r"\s", a) for a in e["argv"]):
                reason = "an argument of its command contains a space, which SpeakEasy cannot pass"
            elif os.path.basename(e["argv"][0]) in _PACKAGE_RUNNERS and len(e["argv"]) == 1:
                reason = "its command is only %s, with no package to run" % e["argv"][0]
            elif not kinds:
                reason = _korotovsky_no_token_reason(e["env"])
            else:
                reason = ""
            c = dict(base, transport="korotovsky", importable=not reason, reason=reason,
                     summary=summary)
        else:
            return None
    c["fingerprint"] = _fingerprint(e, c["transport"]) if c["importable"] else None
    return c


def _plugin_candidates(path):
    """Claude Code's installed_plugins.json: its Slack plugin reaches the
    official server through Anthropic's Claude Slack app, signed in with
    OAuth and stored in Claude Code's credential store — not a session
    SpeakEasy can take over."""
    try:
        data = json.loads(read(path) or "")
    except ValueError:
        return []
    plugins = data.get("plugins") if isinstance(data, dict) else None
    if not isinstance(plugins, dict):
        return []
    return [
        {
            "file": path,
            "name": key,
            "transport": "slack-mcp",
            "importable": False,
            "reason": ("Claude Code's Slack plugin signs in through Anthropic's Claude "
                       "Slack app, and its session lives in Claude Code's credential store"),
            "summary": "official Slack MCP server via Claude Code's Slack plugin",
            "fingerprint": None,
        }
        for key in plugins
        if key.split("@", 1)[0] == "slack"
    ]


def _toml_unreadable_candidates(path):
    """Slack servers in a TOML config this Python cannot parse (no tomllib,
    tomli, or pip's copy): reported, not offered."""
    out = []
    for name, block in _toml_server_blocks(read(path) or ""):
        if name != SERVER and classify_server(name, block) != "other":
            out.append({
                "file": path, "name": name, "transport": "korotovsky", "importable": False,
                "reason": "reading TOML needs Python 3.11+ (or tomli); rerun with PYTHON=python3.11",
                "summary": "Slack MCP server", "fingerprint": None,
            })
    return out


def takeover_candidates(paths):
    """Candidates from every file, in order. Entries with the same name in
    one file are numbered (selector NAME#N) unless they are identical
    copies; identical copies of one server across files and scopes share a
    fingerprint, which install.sh uses to treat them as one server."""
    out = []
    for path in paths:
        if os.path.basename(path) == "installed_plugins.json":
            out.extend(_plugin_candidates(path))
            continue
        entries = _config_entries(path)
        if entries is None:
            out.extend(_toml_unreadable_candidates(path))
            continue
        seen = {}
        for e in entries:
            c = _candidate(path, e)
            if not c:
                continue
            same = seen.setdefault(c["name"], [])
            fp = c["fingerprint"] or json.dumps([e["key"], e["argv"], e["url"]])
            if fp in same:
                continue  # an identical copy in another scope of this file
            same.append(fp)
            if len(same) > 1:
                c["name"] = "%s#%d" % (c["name"], len(same))
            out.append(c)
    return out


def _write_private(path, text):
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w") as f:
        f.write(text)
    os.chmod(path, 0o600)


def _set_aside(path):
    """Clear a credential file the take-over replaces: a placeholder is
    deleted, a real credential is moved to <file>.pre-takeover (or
    .pre-takeover.1, .2, … — an earlier set-aside credential is never
    overwritten)."""
    if not os.path.exists(path):
        return None
    content = (read(path) or "").strip()
    if content in PLACEHOLDERS:
        os.remove(path)
        return "removed placeholder %s" % path
    dest = path + ".pre-takeover"
    n = 0
    while os.path.exists(dest):
        n += 1
        dest = "%s.pre-takeover.%d" % (path, n)
    os.replace(path, dest)
    os.chmod(dest, 0o600)
    return "moved %s to %s" % (path, dest)


def _installed_server_argv(argv):
    """The command to run, with its program as an absolute path. A package
    runner line (npx/bunx/pnpx … slack-mcp-server[@version] ARGS, or
    go run …/cmd/slack-mcp-server[@version] ARGS) becomes the installed
    `slack-mcp-server ARGS` when one is on PATH, so SpeakEasy runs the server
    already installed instead of fetching and building it on every start."""
    runner = os.path.basename(argv[0])
    if runner in _PACKAGE_RUNNERS:
        package = r"^slack-mcp-server(@\S*)?$"
    elif runner == "go" and argv[1:2] == ["run"]:
        package = r"^github\.com/korotovsky/slack-mcp-server/cmd/slack-mcp-server(@\S*)?$"
    else:
        package = None
    if package:
        skip = False
        for i, a in enumerate(argv[1:], 1):
            # the value of the runner's own package option is not the command
            # (npx --package/-p X CMD, uvx --from X CMD)
            if skip:
                skip = False
                continue
            if a in ("--package", "-p", "--from"):
                skip = True
                continue
            if re.match(package, a):
                installed = shutil.which("slack-mcp-server")
                if installed:
                    return [installed] + argv[i + 1:]
                break
    if not os.path.isabs(argv[0]):
        found = shutil.which(argv[0])
        if found:
            argv[0] = found
    return argv


def _container_forwarded(argv, names):
    """A docker/podman `run` line passes only the variables it names with
    -e/--env into the container: add `-e NAME` right after `run` for each
    of NAMES it does not pass yet (the CLI forwards the value SpeakEasy sets
    in its environment)."""
    if os.path.basename(argv[0]) not in _CONTAINER_RUNNERS or "run" not in argv[1:]:
        return argv
    passed = set()
    for i, a in enumerate(argv):
        if a in ("-e", "--env") and i + 1 < len(argv):
            passed.add(argv[i + 1].split("=", 1)[0])
        elif a.startswith("--env="):
            passed.add(a[len("--env="):].split("=", 1)[0])
    at = argv.index("run", 1) + 1
    extra = []
    for n in names:
        if n not in passed:
            extra += ["-e", n]
    return argv[:at] + extra + argv[at:]


def _pick_entry(path, name):
    """The entry NAME (or NAME#N, the Nth distinct same-name entry) in PATH."""
    base, _, nth = name.partition("#")
    matches = []
    for e in _config_entries(path) or []:
        if e["name"] != base:
            continue
        c = _candidate(path, e)
        if not c:
            continue  # numbered like takeover_candidates: Slack MCPs only
        fp = c["fingerprint"] or json.dumps([e["key"], e["argv"], e["url"]])
        if fp not in [m[0] for m in matches]:
            matches.append((fp, e))
    index = int(nth) if nth.isdigit() else 1
    return matches[index - 1][1] if 0 < index <= len(matches) else None


def take_over(state_dir, path, name):
    """Copy the named importable candidate from PATH into STATE_DIR and
    select its transport. Returns the lines to print (secrets masked)."""
    entry = _pick_entry(path, name)
    cand = _candidate(path, entry) if entry else None
    if not cand:
        raise SystemExit("take-over: no Slack MCP server %r in %s" % (name, path))
    if not cand["importable"]:
        raise SystemExit("take-over: cannot take over %s in %s: %s" % (name, path, cand["reason"]))
    os.makedirs(state_dir, exist_ok=True)
    sd = lambda f: os.path.join(state_dir, f)
    lines = []
    transport = cand["transport"]
    if transport == "korotovsky":
        for f in ("slack_bot_token", "slack_user_token", "slack_stdio_env", "slack_stdio_command"):
            note = _set_aside(sd(f))
            if note:
                lines.append(note)
        env_lines = ["# taken over from %s (server: %s)" % (path, name)]
        names = []
        for k, v in entry["env"].items():
            lit = _literal(v)
            if (k in TAKEOVER_ENV_DROP or lit is None or "\n" in lit
                    or not _ENV_NAME_RE.match(k)):
                continue
            if k == "SLACK_MCP_ADD_MESSAGE_TOOL" and lit not in ("true", "1"):
                if not _send_policy_to_keep(lit):
                    lines.append("ignored SLACK_MCP_ADD_MESSAGE_TOOL=%s (it would block every "
                                 "send); SpeakEasy allows every channel you approve" % lit)
                    continue
                lines.append("kept SLACK_MCP_ADD_MESSAGE_TOOL=%s: SpeakEasy can only send to "
                             "these channels" % lit)
            env_lines.append("%s=%s" % (k, lit))
            names.append(k)
        argv = _installed_server_argv(list(entry["argv"]))
        argv = _container_forwarded(
            argv, names + [n for n in ("SLACK_MCP_ADD_MESSAGE_TOOL", "SLACK_MCP_LOG_LEVEL") if n not in names]
        )
        _write_private(sd("slack_stdio_command"), " ".join(argv) + "\n")
        _write_private(sd("slack_stdio_env"), "\n".join(env_lines) + "\n")
        lines.append("wrote %s (%s)" % (sd("slack_stdio_command"), " ".join(argv[:1] + ["…"] if len(argv) > 1 else argv)))
        lines.append("wrote %s (chmod 600; %s)" % (sd("slack_stdio_env"), ", ".join(_korotovsky_tokens(entry["env"]))))
    elif transport == "slack-mcp":
        token = _bearer(entry["headers"])
        note = _set_aside(sd("slack_mcp_token"))
        if note:
            lines.append(note)
        _write_private(sd("slack_mcp_token"), token + "\n")
        lines.append("wrote %s (chmod 600; %s)" % (sd("slack_mcp_token"), _mask(token)))
    else:  # mistral-connector
        key = _bearer(entry["headers"])
        ref = _MISTRAL_CONNECTOR_URL_RE.search(entry["url"]).group(1)
        note = _set_aside(sd("mistral_api_key"))
        if note:
            lines.append(note)
        _write_private(sd("mistral_api_key"), key + "\n")
        lines.append("wrote %s (chmod 600; %s)" % (sd("mistral_api_key"), _mask(key)))
        if ref != "slack":
            write(sd("mistral_slack_connector"), ref + "\n")
            lines.append("wrote %s (%s)" % (sd("mistral_slack_connector"), ref))
        elif os.path.exists(sd("mistral_slack_connector")):
            os.remove(sd("mistral_slack_connector"))
    write(sd("slack_transport"), transport + "\n")
    lines.append("wrote %s (%s)" % (sd("slack_transport"), transport))
    return lines


def enforce_style(path):
    """The removal style --enforce uses for PATH, or None when it cannot
    edit that config (goose, Zed, Hermes: their entries are edited by hand)."""
    base = os.path.basename(path)
    if base == ".claude.json":
        return "claude"
    if base == "opencode.json":
        return "opencode"
    if base == "mcp-config.json":
        return "copilot"
    if base == "settings.json" and "/.config/amp/" in path:
        return "amp"
    if base == "settings.json" and (".pi/" in path or ".paradev/" in path):
        return "pi"
    if base == "mcp.json" and (".pi/" in path or ".paradev/" in path):
        return "pimcp"
    if base == "settings.json" and ".gemini/" in path:
        return "gemini"
    if base == "claude_desktop_config.json" or (base == "mcp.json" and ".cursor/" in path):
        # Claude Desktop / Cursor: a plain mcpServers object, like gemini
        return "gemini"
    if base == "config.toml" and ".vibe" in path:
        return "vibe"
    if base == "config.toml":
        return "codex"
    return None

def enforce_remove(path, name, key):
    """Remove one direct Slack server found by the audit (install.sh
    --enforce). Prints an accurate status line — [removed], [absent],
    [failed], or [edit by hand] — instead of assuming success."""
    style = enforce_style(path)
    if style is None:
        print(
            "    [edit by hand] %s (server: %s) — no automatic removal" % (path, name)
        )
        return "edit-by-hand"
    try:
        if style == "vibe":
            result = remove_vibe_entry(path, name)
        elif style == "codex":
            result = remove_codex_entry(path, name)
        else:
            result = remove_entry(style, path, name, key or None)
    except (Exception, SystemExit) as e:
        print("    [failed]  %s (server: %s): %s" % (path, name, e))
        return "failed"
    if result == "removed":
        print("    [removed] %s (server: %s)" % (path, name))
    elif result == "absent":
        print("    [absent]  %s (server: %s) — already gone" % (path, name))
    else:
        print("    [failed]  %s (server: %s): %s" % (path, name, result))
    return result


# --------------------------------------------------------------------------- #
# approval hooks: the agents' own Slack tools ask SpeakEasy before they run
# --------------------------------------------------------------------------- #
#
# Every agent runs the tool when its hook times out, crashes or exits oddly,
# so each registration (a) gives the agent a timeout longer than the hook's
# own --wait (the binary always answers first), and (b) runs the binary
# through a shell wrapper that turns a missing binary or a crash into a
# block (exit 2). Matchers only narrow which calls start the hook; the
# binary decides precisely (src/hook.rs).

HOOK_MARGIN_SECS = 60  # = src/hook.rs HOOK_TIMEOUT_MARGIN_SECS
# The binary exits with this (not 0) when a call may proceed; the wrapper
# and the plugins turn it into "proceed" and anything else into a block, so
# an older binary without hook mode (which exits 0) fails closed.
HOOK_OK_CODE = 10

# Names that may be Slack tools: anything with "slack" in it, Executor's
# generic tools, and korotovsky's tools on a server named something else.
_SLACK_ISH = "[Ss]lack|SLACK|[Ee]xecutor|conversations_|reactions_|usergroups_|saved_"
# Claude Code tests the matcher unanchored; pdev (which reads the same file)
# anchors it (^(?:m)$), so it must match whole names. src/hook.rs keeps a
# copy for its ConfigChange guard (the tests check they are equal).
CLAUDE_MATCHER = "mcp__.*(%s).*" % _SLACK_ISH
CLAUDE_CONFIG_MATCHER = "user_settings|project_settings|local_settings"
CODEX_MATCHER = "mcp__.*(%s)" % _SLACK_ISH
GEMINI_MATCHER = "mcp_.*(%s)" % _SLACK_ISH          # unanchored JS regex
COPILOT_MATCHER = ".*(%s).*" % _SLACK_ISH           # anchored ^(?:m)$
HOOK_NAME = "speakeasy"

CODEX_HOOK_BEGIN = "# >>> slack-approval-gateway hook (managed by scripts/install.sh) >>>"
CODEX_HOOK_END = "# <<< slack-approval-gateway hook <<<"


def is_hook_command(cmd):
    return isinstance(cmd, str) and SERVER in cmd and "--hook " in cmd


def hook_args(bin_path, agent, wait, state_dir, ok_code=True):
    """The binary's argv for hook mode. --state-dir pins the instance's
    socket (a project's env settings must not move it)."""
    argv = [bin_path, "--hook", agent, "--wait", str(int(wait)), "--state-dir", state_dir]
    if ok_code:
        argv += ["--ok-code", str(HOOK_OK_CODE)]
    return argv


def hook_command(bin_path, agent, wait, state_dir, extra=""):
    """Shell form: the binary's "proceed" code becomes exit 0 and its block
    (2) stays 2; a missing binary or any other exit — 0 from an older binary
    without hook mode included — becomes a block, because every agent lets
    the call through when its hook fails."""
    argv = " ".join(shlex.quote(a) for a in hook_args(bin_path, agent, wait, state_dir))
    b = shlex.quote(bin_path)
    return (
        "[ -x %s ] || { echo 'SpeakEasy is not installed, so this Slack call was blocked; "
        "nothing was sent. Tell the user.' >&2; exit 2; }; "
        "%s%s; rc=$?; [ $rc -eq %d ] && exit 0; [ $rc -eq 2 ] || "
        "echo \"SpeakEasy's hook failed (exit $rc), so this Slack call was blocked; "
        "nothing was sent. Tell the user.\" >&2; exit 2"
        % (b, argv, extra, HOOK_OK_CODE)
    )


def config_change_command(bin_path, wait, state_dir):
    """Claude Code's ConfigChange guard. Unlike the tool hook it lets a
    change through when the binary is missing or fails: after an uninstall
    (or a crash) it must not block every settings change in sessions that
    are still running."""
    argv = " ".join(shlex.quote(a) for a in hook_args(bin_path, "claude", wait, state_dir)
                    + ["--event", "config-change"])
    return ("[ -x %s ] || exit 0; %s; rc=$?; [ $rc -eq 2 ] && exit 2; exit 0"
            % (shlex.quote(bin_path), argv))


def _write_json(path, data):
    os.makedirs(os.path.dirname(os.path.abspath(path)), exist_ok=True)
    dump_json(path, data)


def _strip_json_hooks(hooks):
    """Remove SpeakEasy's handlers from a {event: [group, ...]} hooks map
    (Claude Code / Gemini group style and Cursor's flat style). Returns the
    number removed; empty groups and events are dropped."""
    removed = 0
    for event in list(hooks):
        groups = hooks.get(event)
        if not isinstance(groups, list):
            continue
        kept = []
        for g in groups:
            if not isinstance(g, dict):
                kept.append(g)
                continue
            if is_hook_command(g.get("command")) or is_hook_command(g.get("bash")):
                removed += 1
                continue
            inner = g.get("hooks")
            if isinstance(inner, list):
                mine = [h for h in inner if isinstance(h, dict) and is_hook_command(h.get("command"))]
                if mine:
                    removed += len(mine)
                    g = dict(g, hooks=[h for h in inner if h not in mine])
                    if not g["hooks"]:
                        continue
            kept.append(g)
        if kept:
            hooks[event] = kept
        else:
            del hooks[event]
    return removed


def _json_hooks_edit(path, add):
    """Load a JSON settings file, strip SpeakEasy's hooks, let `add` append
    the new ones (None: remove only), and write it back when it changed."""
    text = read(path)
    try:
        data = json.loads(text) if text and text.strip() else {}
    except ValueError:
        # JSONC (comments, trailing commas): rewriting it would drop them.
        try:
            existing = json.loads(strip_jsonc(text)).get("hooks") or {}
        except (ValueError, AttributeError):
            existing = {}
        has_ours = "--hook " in json.dumps(existing) and SERVER in json.dumps(existing)
        if add is None:
            if not has_ours:
                return "absent"
            raise SystemExit("%s has comments or is not plain JSON; SpeakEasy will not rewrite "
                             "it. Remove SpeakEasy's hook (the entries running --hook) by hand" % path)
        entry = {}
        add(entry)
        wanted = [h for groups in entry.values() for g in groups for h in (g.get("hooks") or [g])]
        present = json.dumps(existing)
        if wanted and all(json.dumps(h.get("command"))[1:-1] in present for h in wanted):
            return "unchanged"  # added by hand, as asked
        raise SystemExit("%s has comments or is not plain JSON; SpeakEasy will not rewrite it. "
                         "Add this to its \"hooks\" object by hand (merge into existing lists):\n%s"
                         % (path, json.dumps(entry, indent=2)))
    if not isinstance(data, dict):
        raise SystemExit("%s is not a JSON object" % path)
    hooks = data.get("hooks")
    if hooks is not None and not isinstance(hooks, dict):
        raise SystemExit("%s: \"hooks\" is not an object" % path)
    hooks = dict(hooks or {})
    removed = _strip_json_hooks(hooks)
    if add is not None:
        add(hooks)
    if hooks:
        data["hooks"] = hooks
    else:
        data.pop("hooks", None)
    if add is None and not removed:
        return "absent"
    if add is not None:
        _warn_hooks_off(path, data)
        backup_original(path)
    _write_json(path, data)
    return "registered" if add is not None else "removed"


def _warn_hooks_off(path, data):
    """Warn about settings that switch every hook off in this agent."""
    off = []
    if data.get("disableAllHooks") is True:
        off.append("disableAllHooks")
    hc = data.get("hooksConfig") if isinstance(data.get("hooksConfig"), dict) else {}
    if hc.get("enabled") is False:
        off.append("hooksConfig.enabled = false")
    if HOOK_NAME in (hc.get("disabled") or []):
        off.append("hooksConfig.disabled lists %s" % HOOK_NAME)
    if off:
        print("warning: %s switches hooks off (%s); the hook will not run" % (path, ", ".join(off)),
              file=sys.stderr)


def hook_add_claude(path, bin_path, wait, state_dir):
    """~/.claude/settings.json (also read by pdev; imported by Cursor)."""
    def add(hooks):
        hooks.setdefault("PreToolUse", []).append({
            "matcher": CLAUDE_MATCHER,
            "hooks": [{"type": "command", "command": hook_command(bin_path, "claude", wait, state_dir),
                       "timeout": wait + HOOK_MARGIN_SECS}],
        })
        hooks.setdefault("ConfigChange", []).append({
            "matcher": CLAUDE_CONFIG_MATCHER,
            "hooks": [{"type": "command",
                       "command": config_change_command(bin_path, wait, state_dir),
                       "timeout": 30}],
        })
    return _json_hooks_edit(path, add)


def hook_add_gemini(path, bin_path, wait, state_dir):
    """~/.gemini/settings.json BeforeTool (timeout in milliseconds)."""
    def add(hooks):
        hooks.setdefault("BeforeTool", []).append({
            "matcher": GEMINI_MATCHER,
            "hooks": [{"name": HOOK_NAME, "type": "command",
                       "command": hook_command(bin_path, "gemini", wait, state_dir),
                       "timeout": (wait + HOOK_MARGIN_SECS) * 1000}],
        })
    return _json_hooks_edit(path, add)


def hook_add_cursor(path, bin_path, wait, state_dir):
    """~/.cursor/hooks.json beforeMCPExecution. Cursor matches MCP tools as
    MCP:<tool> without the server, so there is no matcher: the binary sees
    mcp_server_name and passes every non-Slack call. failClosed makes a
    missing binary, a crash or a timeout block the call."""
    def add(hooks):
        hooks.setdefault("beforeMCPExecution", []).append({
            # run directly (no shell wrapper): Cursor's failClosed blocks a
            # missing binary or a crash, and an older binary prints no {}
            "command": " ".join(shlex.quote(a) for a in
                                hook_args(bin_path, "cursor", wait, state_dir, ok_code=False)),
            "timeout": wait + HOOK_MARGIN_SECS,
            "failClosed": True,
        })
    result = _json_hooks_edit(path, add)
    data = load_json(path)
    if "version" not in data:
        data = dict({"version": 1}, **data)
        _write_json(path, data)
    return result


def hook_add_copilot(path, bin_path, wait, state_dir):
    """A file of its own in ~/.copilot/hooks/ (timeoutSec must be a number)."""
    _write_json(path, {"version": 1, "hooks": {"preToolUse": [{
        "type": "command",
        "bash": hook_command(bin_path, "copilot", wait, state_dir),
        "matcher": COPILOT_MATCHER,
        "timeoutSec": wait + HOOK_MARGIN_SECS,
    }]}})
    return "registered"


def _remove_own_file(path):
    text = read(path)
    if text is None:
        return "absent"
    if SERVER not in text:
        return "not ours (left alone)"
    os.remove(path)
    return "removed"


def _codex_hash(command, matcher, timeout):
    """Codex's trusted_hash for a hook: sha256 of the sorted, compact JSON
    of its identity (verified against Codex 0.163's own currentHash)."""
    ident = {"event_name": "pre_tool_use", "matcher": matcher,
             "hooks": [{"type": "command", "command": command, "timeout": timeout, "async": False}]}
    text = json.dumps(ident, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
    return "sha256:" + hashlib.sha256(text.encode("utf-8")).hexdigest()


def _toml_basic(s):
    return json.dumps(s, ensure_ascii=False)


# TOML hook tables are found by content, never by a comment span: Codex
# (and other TOML writers) insert new tables before a file's trailing
# comment, which would put the user's tables inside a marked span.
_TOML_HEADER = re.compile(r'^\s*(\[\[?)(.+?)(\]\]?)\s*(#.*)?$')
_CODEX_STATE_KEY = re.compile(r'^\s*\[\s*hooks\s*\.\s*state\s*\.\s*"((?:[^"\\]|\\.)*)"\s*\]\s*(#.*)?$')
_HOOK_MARKERS = (CODEX_HOOK_BEGIN, CODEX_HOOK_END)


def _toml_table_spans(lines):
    """[(start, end, name, is_array)] for every table header, where `end` is
    the next header (or EOF) minus trailing blank and comment lines, which
    belong to whatever follows. `name` is normalized (no spaces/quotes)."""
    heads = []
    for i, line in enumerate(lines):
        m = _TOML_HEADER.match(line)
        if m and (m.group(1) == "[[") == (m.group(3) == "]]"):
            name = re.sub(r"\s+", "", m.group(2)).replace('"', "").replace("'", "")
            heads.append((i, name, m.group(1) == "[["))
    tables = []
    for k, (i, name, arr) in enumerate(heads):
        end = heads[k + 1][0] if k + 1 < len(heads) else len(lines)
        while end > i + 1 and (not lines[end - 1].strip() or lines[end - 1].lstrip().startswith("#")):
            end -= 1
        tables.append((i, end, name, arr))
    return tables


def _drop_spans(lines, spans):
    """Remove [start, end) spans (and one blank line left behind each)."""
    drop = set()
    for start, end in spans:
        drop.update(range(start, end))
        if end < len(lines) and not lines[end].strip():
            drop.add(end)
    return [l for i, l in enumerate(lines) if i not in drop]


def _codex_ours(lines, tables):
    """SpeakEasy's PreToolUse group in a Codex config: (group table index,
    hooks table index, group index among PreToolUse groups), or None. The
    group is the [[hooks.PreToolUse]] table followed by a
    [[hooks.PreToolUse.hooks]] table whose command runs `--hook codex`."""
    group_index = -1
    for ti, (start, end, name, arr) in enumerate(tables):
        if not (arr and name == "hooks.PreToolUse"):
            continue
        group_index += 1
        nxt = ti + 1
        if nxt < len(tables) and tables[nxt][2] == "hooks.PreToolUse.hooks" and tables[nxt][3]:
            body = "\n".join(lines[tables[nxt][0]:tables[nxt][1]])
            if SERVER in body and "--hook codex" in body:
                return ti, nxt, group_index
    return None


def _codex_old_hash(lines, tables, ours):
    """The trusted_hash of SpeakEasy's existing group (None without a TOML
    reader or when it cannot be read)."""
    loader = _toml_loader()
    if loader is None or ours is None:
        return None
    g, h, _ = ours
    snippet = "\n".join(lines[tables[g][0]:tables[g][1]] + lines[tables[h][0]:tables[h][1]])
    try:
        group = loader.loads(snippet)["hooks"]["PreToolUse"][0]
        hook = group["hooks"][0]
        return _codex_hash(hook["command"], group.get("matcher", ""), hook.get("timeout"))
    except Exception:  # noqa: BLE001
        return None


def _codex_state_tables(lines, tables):
    """[(table index, key, trusted_hash line text)] for [hooks.state."KEY"]."""
    out = []
    for ti, (start, end, name, arr) in enumerate(tables):
        m = _CODEX_STATE_KEY.match(lines[start])
        if m and not arr:
            key = json.loads('"%s"' % m.group(1))
            out.append((ti, key, "\n".join(lines[start + 1:end])))
    return out


def _codex_key_paths(path):
    out = []
    for p in (os.path.abspath(path), os.path.realpath(path)):
        if p not in out:
            out.append(p)
    return out


def _codex_parse_check(path, text):
    loader = _toml_loader()
    if loader is None:
        return None
    try:
        return loader.loads(text)
    except Exception as e:  # noqa: BLE001
        raise SystemExit("refusing to write %s: the result would not parse (%s)" % (path, e))


def hook_add_codex(path, bin_path, wait, state_dir):
    """~/.codex/config.toml [[hooks.PreToolUse]] plus the trust entry Codex
    requires before it runs a user hook (without it `codex exec` skips the
    hook silently). The trust key names the group by its index among the
    file's PreToolUse groups: an existing SpeakEasy group is updated where
    it is (other groups keep their index, and their trust); a new one is
    appended. Only SpeakEasy's own tables are touched."""
    original = read(path) or ""
    lines = [l for l in original.split("\n") if l.strip() not in _HOOK_MARKERS]
    tables = _toml_table_spans(lines)
    ours = _codex_ours(lines, tables)
    old_hash = _codex_old_hash(lines, tables, ours)
    cmd = hook_command(bin_path, "codex", wait, state_dir)
    timeout = wait + HOOK_MARGIN_SECS
    trusted = _codex_hash(cmd, CODEX_MATCHER, timeout)
    group = ["[[hooks.PreToolUse]]", "matcher = %s" % _toml_basic(CODEX_MATCHER), "",
             "[[hooks.PreToolUse.hooks]]", 'type = "command"', "command = %s" % _toml_basic(cmd),
             "timeout = %d" % timeout]
    if ours is not None:
        g, h, index = ours
        start, end = tables[g][0], tables[h][1]
        lines[start:end] = group
    else:
        index = sum(1 for t in tables if t[3] and t[2] == "hooks.PreToolUse")
        lines = (lines[:-1] if lines and not lines[-1].strip() else lines)
        lines += ([""] if any(l.strip() for l in lines) else []) + group
    keys = ["%s:pre_tool_use:%d:0" % (p, index) for p in _codex_key_paths(path)]
    # Replace SpeakEasy's trust tables: by key, and any other carrying the
    # old group's hash (a stale index).
    tables = _toml_table_spans(lines)
    stale = []
    present = set()
    for ti, key, body in _codex_state_tables(lines, tables):
        if key in keys:
            present.add(key)
            stale.append(ti)
        elif old_hash and old_hash in body:
            stale.append(ti)
    lines = _drop_spans(lines, [(tables[ti][0], tables[ti][1]) for ti in stale])
    while lines and not lines[-1].strip():
        lines.pop()
    for key in keys:
        lines += ["", "[hooks.state.%s]" % _toml_basic(key), "trusted_hash = %s" % _toml_basic(trusted)]
    new = "\n".join(lines) + "\n"
    parsed = _codex_parse_check(path, new)
    if parsed is not None:
        feats = parsed.get("features") if isinstance(parsed.get("features"), dict) else {}
        if feats.get("hooks") is False or feats.get("codex_hooks") is False:
            print("warning: %s turns hooks off ([features] hooks = false); the hook will not run" % path,
                  file=sys.stderr)
        groups = (parsed.get("hooks") or {}).get("PreToolUse") or []
        if len(groups) <= index or "--hook codex" not in json.dumps(groups[index]):
            raise SystemExit("refusing to write %s: SpeakEasy's hook group would not be at index %d"
                             % (path, index))
    backup_original(path)
    write(path, new)
    return "registered"


def hook_remove_codex(path):
    """Remove SpeakEasy's group and trust tables. Groups after it move up one
    index, so their trust keys are renumbered to keep them trusted."""
    text = read(path)
    if text is None:
        return "absent"
    lines = [l for l in text.split("\n") if l.strip() not in _HOOK_MARKERS]
    had_markers = len(lines) != len(text.split("\n"))
    tables = _toml_table_spans(lines)
    ours = _codex_ours(lines, tables)
    if ours is None:
        if had_markers:
            write(path, "\n".join(lines))
        return "absent"
    old_hash = _codex_old_hash(lines, tables, ours)
    g, h, index = ours
    spans = [(tables[g][0], tables[h][1])]
    paths = _codex_key_paths(path)
    renames = {}
    for ti, key, body in _codex_state_tables(lines, tables):
        m = re.match(r"^(.*):pre_tool_use:(\d+):(\d+)$", key)
        mine_by_key = m and m.group(1) in paths and int(m.group(2)) == index
        if mine_by_key or (old_hash and old_hash in body):
            spans.append((tables[ti][0], tables[ti][1]))
        elif m and m.group(1) in paths and int(m.group(2)) > index:
            renames[tables[ti][0]] = "[hooks.state.%s]" % _toml_basic(
                "%s:pre_tool_use:%d:%s" % (m.group(1), int(m.group(2)) - 1, m.group(3)))
    for i, header in renames.items():
        lines[i] = header
    lines = _drop_spans(lines, spans)
    new = "\n".join(lines)
    _codex_parse_check(path, new)
    write(path, new)
    return "removed"


HERMES_PLUGIN_PY = r"""# SpeakEasy: Hermes's own Slack tools ask the user before they run.
# Managed by SpeakEasy's scripts/install.sh (removed by --uninstall).
import json
import subprocess

ARGV = %(argv)s
WAIT = %(wait)d
OK = %(ok)d  # the binary's "proceed" exit code; 0 (an older binary) blocks
_MAYBE_SLACK = ("slack", "executor", "conversations_", "reactions_", "usergroups_", "saved_")


def _block(message):
    return {"action": "block", "message": message}


def _pre_tool_call(tool_name="", args=None, **kw):
    try:
        if not any(k in (tool_name or "").lower() for k in _MAYBE_SLACK):
            return None
        payload = json.dumps({
            "tool_name": tool_name or "",
            "tool_input": args if args is not None else {},
            "session_id": kw.get("session_id") or "",
            "tool_call_id": kw.get("tool_call_id") or "",
        })
        p = subprocess.run(ARGV, input=payload, capture_output=True, text=True, timeout=WAIT + 60)
        if p.returncode == OK:
            return None
        reason = ""
        try:
            reason = json.loads(p.stdout or "{}").get("reason") or ""
        except Exception:
            pass
        return _block(reason or p.stderr.strip()
                      or "SpeakEasy blocked this Slack call; nothing was sent. Tell the user.")
    except Exception as e:  # Hermes lets a call through when a hook raises
        return _block("SpeakEasy could not check this Slack call (%%s), so it was blocked; "
                      "nothing was sent. Tell the user." %% e)


def register(ctx):
    ctx.register_hook("pre_tool_call", _pre_tool_call)
"""

HERMES_PLUGIN_YAML = """name: speakeasy
version: "1"
description: "SpeakEasy: ask the user before Slack tools send (managed by SpeakEasy's install.sh)"
hooks:
  - pre_tool_call
"""


class HermesConfigUnsupported(Exception):
    """config.yaml's `plugins:` section is in a form this editor does not
    rewrite safely (flow style, unusual indentation)."""


def _hermes_plugins_state(text):
    """Find config.yaml's `plugins.enabled` list in the block style Hermes
    writes. Returns (lines, plugins_at, enabled_at, items, item_indent):
    line indices (None when absent), the item line indices and their
    indentation. Raises HermesConfigUnsupported for anything else."""
    lines = text.split("\n")
    plugins = [i for i, l in enumerate(lines) if re.match(r"^plugins\s*:", l)]
    if len(plugins) > 1:
        raise HermesConfigUnsupported("more than one top-level plugins: key")
    if not plugins:
        return lines, None, None, [], 4
    p = plugins[0]
    if not re.match(r"^plugins:\s*(#.*)?$", lines[p]):
        raise HermesConfigUnsupported("plugins: is written in flow style")
    end = len(lines)
    for j in range(p + 1, len(lines)):
        if lines[j].strip() and not lines[j].lstrip().startswith("#") and not lines[j].startswith(" "):
            end = j
            break
    enabled = [j for j in range(p + 1, end) if re.match(r"^\s+enabled\s*:", lines[j])]
    if not enabled:
        children = [lines[j] for j in range(p + 1, end) if lines[j].strip() and not lines[j].lstrip().startswith("#")]
        if any(not c.startswith("  ") or c.startswith("   ") for c in children if not c.startswith("    ")):
            raise HermesConfigUnsupported("plugins: children are not indented by two spaces")
        return lines, p, None, [], 4
    if len(enabled) > 1:
        raise HermesConfigUnsupported("more than one plugins.enabled key")
    e = enabled[0]
    if not re.match(r"^  enabled:\s*(\[\s*\])?\s*(#.*)?$", lines[e]):
        raise HermesConfigUnsupported("plugins.enabled is written in flow style or indented unusually")
    items = []
    indent = None
    for j in range(e + 1, end):
        line = lines[j]
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        m = re.match(r"^( +)- ", line)
        if not m or len(m.group(1)) not in (2, 4):
            break
        if indent is None:
            indent = len(m.group(1))
        if len(m.group(1)) != indent:
            raise HermesConfigUnsupported("plugins.enabled items are indented unevenly")
        items.append(j)
    if items and "[" in lines[e].split("#")[0]:
        raise HermesConfigUnsupported("plugins.enabled mixes [] and items")
    return lines, p, e, items, indent or 4


def _is_speakeasy_item(line):
    return line.split("#")[0].strip() in ("- speakeasy", "- 'speakeasy'", '- "speakeasy"')


def _hermes_enabled(text):
    """Whether config.yaml enables the speakeasy plugin (False when the
    section cannot be read)."""
    try:
        lines, _, _, items, _ = _hermes_plugins_state(text)
    except HermesConfigUnsupported:
        return False
    return any(_is_speakeasy_item(lines[i]) for i in items)


def _hermes_enabled_edit(config_path, enable):
    """Add/remove `speakeasy` in config.yaml's plugins.enabled list. Returns
    True when the file changed. Raises SystemExit (nothing written) when the
    section is in a form it does not rewrite safely."""
    text = read(config_path) or ""
    try:
        lines, p, e, items, indent = _hermes_plugins_state(text)
    except HermesConfigUnsupported as why:
        if not enable:
            return False
        raise SystemExit("%s: %s; add `speakeasy` to plugins.enabled by hand" % (config_path, why))
    mine = [i for i in items if _is_speakeasy_item(lines[i])]
    if enable == bool(mine):
        return False
    if enable:
        item = " " * indent + "- speakeasy"
        if e is not None:
            lines[e] = "  enabled:"
            lines.insert((items[-1] if items else e) + 1, item)
        elif p is not None:
            lines[p + 1:p + 1] = ["  enabled:", item]
        else:
            while lines and not lines[-1].strip():
                lines.pop()
            lines += ["plugins:", "  enabled:", item, ""]
    else:
        lines = [l for i, l in enumerate(lines) if i not in mine]
        if len(mine) == len(items):
            lines[e] = "  enabled: []"
    new = "\n".join(lines)
    if not _hermes_enabled(new) == enable:
        raise SystemExit("%s: could not %s speakeasy in plugins.enabled safely; edit it by hand"
                         % (config_path, "enable" if enable else "disable"))
    if enable:
        backup_original(config_path)
    write(config_path, new)
    return True


def hook_add_hermes(config_path, bin_path, wait, state_dir):
    """A Hermes plugin (~/.hermes/plugins/speakeasy) enabled in config.yaml.
    Plugins have no hook time limit (shell hooks are capped at 300 s) and
    load in every Hermes front end."""
    plugin_dir = os.path.join(os.path.dirname(config_path), "plugins", HOOK_NAME)
    os.makedirs(plugin_dir, exist_ok=True)
    write(os.path.join(plugin_dir, "plugin.yaml"), HERMES_PLUGIN_YAML)
    write(os.path.join(plugin_dir, "__init__.py"),
          HERMES_PLUGIN_PY % {"argv": json.dumps(hook_args(bin_path, "hermes", wait, state_dir)),
                              "wait": wait, "ok": HOOK_OK_CODE})
    _hermes_enabled_edit(config_path, True)
    return "registered"


def hook_remove_hermes(config_path):
    plugin_dir = os.path.join(os.path.dirname(config_path), "plugins", HOOK_NAME)
    removed = False
    init = os.path.join(plugin_dir, "__init__.py")
    if SERVER in (read(init) or "") or "SpeakEasy" in (read(init) or ""):
        shutil.rmtree(plugin_dir, ignore_errors=True)
        removed = True
    if read(config_path) is not None and _hermes_enabled_edit(config_path, False):
        removed = True
    return "removed" if removed else "absent"


AMP_PLUGIN_TS = r"""// SpeakEasy: Amp's own Slack tools ask the user before they run.
// Managed by SpeakEasy's scripts/install.sh (removed by --uninstall).
import { spawn } from 'node:child_process'

const BIN = %(bin)s
const ARGS = %(args)s
const WAIT = %(wait)d
const OK = %(ok)d // the binary's "proceed" exit code; 0 (an older binary) blocks
const MAYBE_SLACK = /slack|executor|conversations_|reactions_|usergroups_|saved_/i

export default function (amp: any) {
  amp.on('tool.call', (e: any) => new Promise((resolve) => {
    if (!MAYBE_SLACK.test(String(e.tool ?? ''))) return resolve({ action: 'allow' })
    const reject = (message: string) => resolve({ action: 'reject-and-continue', message })
    let out = ''
    let err = ''
    let p
    try {
      p = spawn(BIN, ARGS)
    } catch (x: any) {
      return reject(`SpeakEasy could not run (${x?.message ?? x}), so this Slack call was blocked; nothing was sent. Tell the user.`)
    }
    p.stdout.on('data', (d: any) => (out += d))
    p.stderr.on('data', (d: any) => (err += d))
    p.on('error', (x: any) => reject(`SpeakEasy could not run (${x?.message ?? x}), so this Slack call was blocked; nothing was sent. Tell the user.`))
    p.on('close', (code: number) => {
      if (code === OK) return resolve({ action: 'allow' })
      let reason = ''
      try { reason = JSON.parse(out || '{}').reason ?? '' } catch {}
      reject(reason || err.trim() || 'SpeakEasy blocked this Slack call; nothing was sent. Tell the user.')
    })
    p.stdin.end(JSON.stringify({
      tool_name: e.tool, tool_input: e.input ?? {}, tool_use_id: e.toolUseID ?? '',
      session_id: e.thread?.id ?? '',
    }))
  }))
}
"""


def hook_add_amp(path, bin_path, wait, state_dir):
    """~/.config/amp/plugins/speakeasy.ts: a `tool.call` plugin (Amp's
    documented way to gate tools; its permission rules are legacy and are
    skipped by amp.dangerouslyAllowAll)."""
    argv = hook_args(bin_path, "amp", wait, state_dir)
    write(path, AMP_PLUGIN_TS % {"bin": json.dumps(argv[0]), "args": json.dumps(argv[1:]),
                                 "wait": wait, "ok": HOOK_OK_CODE})
    return "registered"


VIBE_HOOK_BEGIN = "# >>> slack-approval-gateway hook (managed by scripts/install.sh) >>>"
VIBE_HOOK_END = "# <<< slack-approval-gateway hook <<<"
# Vibe tests `re:` matchers as a full match against mcp_<server>.<tool>,
# <tool> and mcp_<server>_<tool>; connectors are connector_<alias>.<tool>.
VIBE_MATCHER = "re:(mcp|connector)_.*(%s).*" % _SLACK_ISH


VIBE_HOOK_NAME = "speakeasy-slack-approval"


def _vibe_ours(lines, tables):
    """Indices of SpeakEasy's [[hooks]] tables (by their unique name)."""
    out = []
    for ti, (start, end, name, arr) in enumerate(tables):
        body = lines[start + 1:end]
        if arr and name == "hooks" and any(
                re.match(r'^\s*name\s*=\s*["\']%s["\']\s*(#.*)?$' % VIBE_HOOK_NAME, l) for l in body):
            out.append(ti)
    return out


def hook_add_vibe(path, bin_path, wait, state_dir):
    """~/.vibe/hooks.toml [[hooks]] pre_tool entry. strict = true makes a
    crash, a timeout or bad output a deny (Vibe runs the tool otherwise).
    The entry is found by its unique name and updated in place. A TOML
    error would make Vibe skip every hook in the file, so the result is
    parsed before it is written."""
    original = read(path) or ""
    lines = [l for l in original.split("\n") if l.strip() not in (VIBE_HOOK_BEGIN, VIBE_HOOK_END)]
    tables = _toml_table_spans(lines)
    entry = ["[[hooks]]",
             'name = "%s"' % VIBE_HOOK_NAME,
             'type = "pre_tool"',
             "match = %s" % _toml_basic(VIBE_MATCHER),
             "command = %s" % _toml_basic(hook_command(bin_path, "vibe", wait, state_dir)),
             "timeout = %.1f" % (wait + HOOK_MARGIN_SECS),
             "strict = true",
             'description = "SpeakEasy: ask the user before a Slack write"']
    ours = _vibe_ours(lines, tables)
    if ours:
        first = tables[ours[0]]
        lines = _drop_spans(lines, [(tables[ti][0], tables[ti][1]) for ti in ours[1:]])
        lines[first[0]:first[1]] = entry
    else:
        while lines and not lines[-1].strip():
            lines.pop()
        lines += ([""] if lines else []) + entry
    new = "\n".join(lines).rstrip("\n") + "\n"
    loader = _toml_loader()
    if loader is not None:
        try:
            loader.loads(new)
        except Exception as e:  # noqa: BLE001
            raise SystemExit("refusing to write %s: the result would not parse (%s)" % (path, e))
    backup_original(path)
    write(path, new)
    return "registered"


def hook_remove_vibe(path):
    text = read(path)
    if text is None:
        return "absent"
    lines = [l for l in text.split("\n") if l.strip() not in (VIBE_HOOK_BEGIN, VIBE_HOOK_END)]
    tables = _toml_table_spans(lines)
    ours = _vibe_ours(lines, tables)
    if not ours:
        return "absent"
    lines = _drop_spans(lines, [(tables[ti][0], tables[ti][1]) for ti in ours])
    write(path, "\n".join(lines))
    return "removed"


# goose searches the raw tool name (it may arrive as functions.slack__x,
# Slack__x or slack.x); execute_typescript is Code Mode, whose nested MCP
# calls are not hooked, so the binary reads the program.
GOOSE_MATCHER = "(?i)slack|executor|conversations_|reactions_|usergroups_|saved_|execute_typescript"


def hook_add_goose(path, bin_path, wait, state_dir):
    """~/.agents/plugins/speakeasy-slack-approval/hooks/hooks.json, a plugin
    of its own. The timeout must be an integer (a float makes goose skip
    the file); goose runs the tool when a hook fails or times out."""
    _write_json(path, {"hooks": {"PreToolUse": [{
        "matcher": GOOSE_MATCHER,
        "hooks": [{"type": "command", "command": hook_command(bin_path, "goose", wait, state_dir),
                   "timeout": int(wait + HOOK_MARGIN_SECS)}],
    }]}})
    return "registered"


def hook_remove_goose(path):
    """Remove the plugin directory (<plugin>/hooks/hooks.json)."""
    text = read(path)
    if text is None:
        return "absent"
    if SERVER not in text:
        return "not ours (left alone)"
    plugin_dir = os.path.dirname(os.path.dirname(path))
    shutil.rmtree(plugin_dir, ignore_errors=True)
    return "removed"


OPENCODE_PLUGIN_JS = r"""// SpeakEasy: ask the user before OpenCode's own Slack tools run.
// Managed by SpeakEasy's scripts/install.sh (removed by --uninstall).
// Runs `slack-approval-gateway --hook opencode` (payload on stdin) before Slack-looking tool calls.
// Exit OK = run the tool; anything else (deny, crash, kill, spawn failure, backstop) blocks it.
// Keep ONLY function exports here: OpenCode skips the whole file if any export is not a
// function. Install ONE copy only: a second copy would ask the user twice.

const GATEWAY = %(bin)s
const ARGS = %(args)s
const WAIT_SECS = %(wait)d
const OK = %(ok)d // the gateway's "proceed" exit code; 0 (an older binary) blocks
const BACKSTOP_MS = (WAIT_SECS + 60) * 1000 // the gateway answers within WAIT_SECS; OpenCode never times out a hook

// Cheap pre-filter; the gateway makes the real decision.
const SLACKISH = /slack|executor|conversations_|reactions_|usergroups_|saved_/i
const OWN = /slack[-_]approval[-_]gateway|speakeasy|request_slack_message/i

const sanitize = (s) => String(s).replace(/[^a-zA-Z0-9_-]/g, "_") // as OpenCode's McpCatalog.sanitize

export const SpeakEasySlackApproval = async ({ directory, worktree }) => {
  let servers = [] // sanitized MCP server names, longest first
  const decisions = new Map() // session, call and arguments -> Promise, so a repeated trigger never asks twice

  const serverOf = (tool) => servers.find((s) => tool.startsWith(s + "_"))

  const runGateway = async (payload) => {
    let proc
    try {
      proc = Bun.spawn([GATEWAY, ...ARGS], {
        stdin: new Blob([payload]),
        stdout: "pipe",
        stderr: "pipe",
        env: { ...process.env },
      })
    } catch (e) {
      return { code: null, error: String(e?.message ?? e), out: "", err: "" }
    }
    let timedOut = false
    const timer = setTimeout(() => {
      timedOut = true
      try { proc.kill(9) } catch {}
    }, BACKSTOP_MS)
    try {
      const [code, out, err] = await Promise.all([proc.exited, new Response(proc.stdout).text(), new Response(proc.stderr).text()])
      return { code, signal: proc.signalCode, timedOut, out: out.trim(), err: err.trim() }
    } finally {
      clearTimeout(timer)
    }
  }

  return {
    config: async (cfg) => {
      servers = Object.keys(cfg?.mcp ?? {}).map(sanitize).sort((a, b) => b.length - a.length)
    },
    "tool.execute.before": async (input, output) => {
      const tool = String(input?.tool ?? "")
      const server = serverOf(tool)
      const hay = `${tool} ${server ?? ""}`
      if (!SLACKISH.test(hay) || OWN.test(hay)) return
      const args = output?.args ?? {}
      const key = JSON.stringify([input.sessionID, input.callID, tool, args])
      let pending = decisions.get(key)
      if (!pending) {
        pending = runGateway(
          JSON.stringify({
            hook_event_name: "PreToolUse",
            tool_name: tool,
            tool_input: args,
            ...(server ? { mcp_server: { name: server }, mcp_tool: tool.slice(server.length + 1) } : {}),
            session_id: input.sessionID,
            tool_use_id: input.callID,
            cwd: directory || worktree || process.cwd(),
          }),
        )
        decisions.set(key, pending)
        setTimeout(() => decisions.delete(key), 60_000).unref?.()
      }
      const r = await pending
      if (r.code === OK && !r.signal && !r.timedOut && !r.error) return // approved, or not a Slack write
      let reason
      try {
        reason = JSON.parse(r.out)?.reason
      } catch {}
      if (!reason && r.code === 2 && r.err) reason = r.err
      throw new Error(
        reason ||
          `SpeakEasy could not check this Slack call, so it was blocked; nothing was sent. Do not retry or use another route to Slack; tell the user. (${r.error || (r.timedOut ? "timed out" : r.err || `exit ${r.code}${r.signal ? ", " + r.signal : ""}`)})`,
      )
    },
  }
}
"""

PI_EXTENSION_TS = r"""// SpeakEasy: ask the user before pi's own Slack tools run (including calls nested in
// codemode scripts). Managed by SpeakEasy's scripts/install.sh (removed by --uninstall).
// Runs `slack-approval-gateway --hook pi` with the payload on stdin. Exit OK = run the call.
// Anything else (deny, crash, signal, spawn failure, backstop, user abort) blocks it.
// Keep this file dependency-free: a load error makes pi refuse to start.
import { spawn, type ChildProcess } from "node:child_process"

const GATEWAY = %(bin)s
const ARGS = %(args)s
const WAIT_SECS = %(wait)d
const OK = %(ok)d // the gateway's "proceed" exit code; 0 (an older binary) blocks
const BACKSTOP_MS = (WAIT_SECS + 60) * 1000 // pi has no hook timeout of its own
const SLACKISH = /slack|executor|conversations_|reactions_|usergroups_|saved_/i
const OWN = /slack[-_]approval[-_]gateway|speakeasy|request_slack_message/i
const NO_ROUTE = "Nothing was sent. Do not retry and do not try another route to Slack; tell the user."

type GatewayResult = { code: number | null; signal: string | null; stdout: string; stderr: string; error?: string; why?: string }
const children = new Set<ChildProcess>()

function deepFreeze(v: unknown): void {
	if (v && typeof v === "object" && !Object.isFrozen(v)) {
		Object.freeze(v)
		for (const k of Object.keys(v)) deepFreeze((v as Record<string, unknown>)[k])
	}
}

function runGateway(payload: string, signal?: AbortSignal): Promise<GatewayResult> {
	return new Promise((resolve) => {
		let stdout = ""
		let stderr = ""
		let done = false
		let child: ChildProcess
		const finish = (r: Partial<GatewayResult>) => {
			if (done) return
			done = true
			clearTimeout(timer)
			signal?.removeEventListener("abort", onAbort)
			children.delete(child)
			resolve({ code: null, signal: null, stdout, stderr, ...r })
		}
		const stop = (why: string) => {
			try { child.kill("SIGTERM") } catch {}
			setTimeout(() => { try { child.kill("SIGKILL") } catch {} }, 2000).unref?.()
			finish({ why })
		}
		const onAbort = () => stop("the user interrupted the agent")
		const timer = setTimeout(() => stop("SpeakEasy did not answer in time"), BACKSTOP_MS)
		try {
			child = spawn(GATEWAY, ARGS, { stdio: ["pipe", "pipe", "pipe"] })
		} catch (e) {
			finish({ error: String(e) })
			return
		}
		children.add(child)
		if (signal?.aborted) return onAbort()
		signal?.addEventListener("abort", onAbort, { once: true })
		child.stdout?.on("data", (d) => (stdout += d))
		child.stderr?.on("data", (d) => (stderr += d))
		child.on("error", (e) => finish({ error: String(e) }))
		child.on("close", (code, sig) => finish({ code, signal: sig }))
		child.stdin?.on("error", () => {})
		child.stdin?.end(payload)
	})
}

export default function (pi: any) {
	pi.on("session_shutdown", () => {
		for (const c of children) try { c.kill("SIGTERM") } catch {}
	})

	pi.on("tool_call", async (event: any, ctx: any) => {
		const ns = pi.getAllTools?.().find((t: any) => t.name === event.toolName)?.namespace
		const server: string | undefined = ns?.aliases?.[0] ?? (typeof ns?.name === "string" ? ns.name.replace(/^mcp__/, "") : undefined)
		const hay = `${event.toolName} ${server ?? ""}`
		if (!SLACKISH.test(hay) || OWN.test(hay)) return undefined // not Slack: no opinion

		// Later handlers see the same input object; freeze it so what the user approves is what runs.
		deepFreeze(event.input)
		let sessionId: string | undefined
		let cwd: string | undefined
		try { sessionId = ctx.sessionManager?.getSessionId?.(); cwd = ctx.cwd } catch {}
		const r = await runGateway(
			JSON.stringify({
				hook_event_name: "PreToolUse",
				tool_name: event.toolName,
				tool_input: event.input,
				tool_use_id: event.toolCallId,
				...(server ? { mcp_server: { name: server } } : {}),
				session_id: sessionId,
				cwd,
			}),
			ctx.signal,
		)
		if (r.code === OK && !r.signal && !r.error && !r.why) return undefined // approved or no opinion

		let reason: string | undefined
		if (r.code === 2) {
			try { reason = JSON.parse(r.stdout)?.reason } catch {}
			reason ||= r.stderr.trim() || undefined
		}
		if (!reason) {
			const detail = r.why ?? (r.error ? `it could not start: ${r.error}` : r.stderr.trim() || `exit ${r.code}${r.signal ? `, ${r.signal}` : ""}`)
			reason = `SpeakEasy could not get the user's approval (${detail}). ${NO_ROUTE}`
		}
		return { block: true, reason }
	})
}
"""


def hook_add_opencode(path, bin_path, wait, state_dir):
    """~/.config/opencode/plugins/speakeasy-slack-approval.js (one copy)."""
    argv = hook_args(bin_path, "opencode", wait, state_dir)
    write(path, OPENCODE_PLUGIN_JS % {"bin": json.dumps(argv[0]), "args": json.dumps(argv[1:]),
                                      "wait": wait, "ok": HOOK_OK_CODE})
    return "registered"


def hook_add_pi(path, bin_path, wait, state_dir):
    """~/.pi/agent/extensions/speakeasy-slack-gate.ts."""
    argv = hook_args(bin_path, "pi", wait, state_dir)
    write(path, PI_EXTENSION_TS % {"bin": json.dumps(argv[0]), "args": json.dumps(argv[1:]),
                                   "wait": wait, "ok": HOOK_OK_CODE})
    return "registered"


HOOK_ADDERS = {
    "claude": hook_add_claude,
    "codex": hook_add_codex,
    "gemini": hook_add_gemini,
    "copilot": hook_add_copilot,
    "cursor": hook_add_cursor,
    "hermes": hook_add_hermes,
    "amp": hook_add_amp,
    "vibe": hook_add_vibe,
    "goose": hook_add_goose,
    "opencode": hook_add_opencode,
    "pi": hook_add_pi,
}


def hook_add(agent, path, bin_path, wait, state_dir):
    if not os.path.isabs(state_dir):
        raise SystemExit("hook-add: STATE_DIR must be an absolute path")
    return HOOK_ADDERS[agent](path, bin_path, int(wait), state_dir)


def hook_remove(agent, path):
    if agent in ("claude", "gemini", "cursor"):
        if read(path) is None:
            return "absent"
        return _json_hooks_edit(path, None)
    if agent == "codex":
        return hook_remove_codex(path)
    if agent == "hermes":
        return hook_remove_hermes(path)
    if agent == "vibe":
        return hook_remove_vibe(path)
    if agent == "goose":
        return hook_remove_goose(path)
    if agent in ("copilot", "amp", "opencode", "pi"):
        return _remove_own_file(path)
    raise SystemExit("unknown agent %r" % agent)


def hook_has(agent, path):
    """True when SpeakEasy's hook is registered in `path`."""
    text = read(path)
    if text is None:
        return False
    if agent == "codex":
        lines = text.split("\n")
        return _codex_ours(lines, _toml_table_spans(lines)) is not None
    if agent == "vibe":
        lines = text.split("\n")
        return bool(_vibe_ours(lines, _toml_table_spans(lines)))
    if agent == "hermes":
        init = os.path.join(os.path.dirname(path), "plugins", HOOK_NAME, "__init__.py")
        return read(init) is not None and _hermes_enabled(text)
    if agent in ("copilot", "amp", "opencode", "pi", "goose"):
        return SERVER in text
    return "--hook " in text and SERVER in text


# --------------------------------------------------------------------------- #
# CLI
# --------------------------------------------------------------------------- #

USAGE = __doc__


def main(argv):
    if not argv or argv[0] in ("-h", "--help"):
        print(USAGE)
        return 0 if argv else 2
    cmd, args = argv[0], argv[1:]
    if cmd == "add-json":
        style, path, gateway = args
        print("%s: %s" % (path, add_json(style, path, gateway)))
    elif cmd == "upgrade-json":
        style, path, gateway = args
        print("%s: %s" % (path, upgrade_json(style, path, gateway)))
    elif cmd == "add-zed":
        path, gateway = args
        print("%s: %s" % (path, add_zed(path, gateway)))
    elif cmd == "upgrade-zed":
        path, gateway = args
        print("%s: %s" % (path, upgrade_zed(path, gateway)))
    elif cmd == "add-goose":
        path, gateway = args
        print("%s: %s" % (path, add_goose(path, gateway)))
    elif cmd == "add-hermes":
        path, gateway = args
        print("%s: %s" % (path, add_hermes(path, gateway)))
    elif cmd == "add-codex":
        path, gateway = args
        print("%s: %s" % (path, add_codex(path, gateway)))
    elif cmd == "add-vibe":
        path, gateway = args
        print("%s: %s" % (path, add_vibe(path, gateway)))
    elif cmd == "remove-json":
        style, path = args
        print("%s: %s" % (path, remove_json(style, path)))
    elif cmd in ("remove-zed", "remove-goose", "remove-hermes", "remove-codex", "remove-vibe"):
        (path,) = args
        fn = {
            "remove-zed": remove_zed,
            "remove-goose": remove_goose,
            "remove-hermes": remove_hermes,
            "remove-codex": remove_codex,
            "remove-vibe": remove_vibe,
        }[cmd]
        print("%s: %s" % (path, fn(path)))
    elif cmd == "remove-entry":
        style, path, name = args[0], args[1], args[2]
        keys = json.loads(args[3]) if len(args) > 3 else None
        print("%s: %s" % (path, remove_entry(style, path, name, keys)))
    elif cmd == "remove-codex-entry":
        path, name = args
        print("%s: %s" % (path, remove_codex_entry(path, name)))
    elif cmd == "remove-vibe-entry":
        path, name = args
        print("%s: %s" % (path, remove_vibe_entry(path, name)))
    elif cmd == "set-timeout":
        path, name, seconds = args
        print("%s: %s" % (path, set_timeout(path, name, int(seconds))))
    elif cmd == "enforce-remove":
        path, name, key = json.loads(args[0])
        enforce_remove(path, name, key)
    elif cmd == "backup":
        (path,) = args
        backup(path)
    elif cmd == "hook-add":
        agent, path, bin_path, wait, state_dir = args
        print("%s: %s" % (path, hook_add(agent, path, bin_path, wait, state_dir)))
    elif cmd == "hook-remove":
        agent, path = args
        print("%s: %s" % (path, hook_remove(agent, path)))
    elif cmd == "hook-has":
        agent, path = args
        return 0 if hook_has(agent, path) else 1
    elif cmd == "instruction-add":
        # instruction-add FILE [gateway|either|hooks|confirm]
        path, variant = args[0], (args[1] if len(args) > 1 else "gateway")
        print("%s: %s" % (path, instruction_add(path, variant)))
    elif cmd == "instruction-remove":
        (path,) = args
        print("%s: %s" % (path, instruction_remove(path)))
    elif cmd == "has":
        path = args[0]
        name = args[1] if len(args) > 1 else SERVER
        sys.exit(0 if has_entry(path, name) else 1)
    elif cmd == "can-enforce":
        (path,) = args
        sys.exit(0 if enforce_style(path) else 1)
    elif cmd == "takeover-candidates":
        print(json.dumps(takeover_candidates(args), indent=2))
    elif cmd == "take-over":
        state_dir, path, name = args
        for line in take_over(state_dir, path, name):
            print(line)
    elif cmd == "audit":
        findings = [f for f in (audit_file(p) for p in args) if f]
        print(json.dumps(findings, indent=2))
    else:
        print("unknown subcommand: %s" % cmd, file=sys.stderr)
        print(USAGE, file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
