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
                              timeout: line in a goose YAML extensions block.
                              A request_slack_message call blocks on a human
                              approval for minutes
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
  instruction-add FILE [gateway|hooks|confirm]
                              insert/replace the managed instruction block in
                              an agent instruction file: "use the gateway's
                              request_slack_message" (default) or, for the
                              hooks-only setup, "use your Slack tool; the
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

# The hooks-only setup, for an agent without SpeakEasy's hook (Zed's
# built-in agent): nothing but the instruction stands between it and Slack.
INSTRUCTION_CONFIRM = """## Slack: ask the user before sending

Before you send, post, schedule, or react to anything in Slack, show the user the exact message and where it would go (channel, person, thread), and wait for an explicit yes in this conversation. Reading Slack is fine. Never send Slack messages through curl, webhooks, SDKs, CLIs, scripts, or the browser."""

# STYLE -> (parent key, entry builder). A parent key starting with "@" is a
# single literal key (amp settings use the flat "amp.mcpServers" key, VS
# Code-style); otherwise the key is a dotted path into nested objects.
JSON_STYLES = {
    "claude": ("mcpServers", lambda cmd: {"type": "stdio", "command": cmd, "args": []}),
    "gemini": ("mcpServers", lambda cmd: {"command": cmd, "args": []}),
    "pi": ("mcpServers", lambda cmd: {"command": cmd, "args": []}),
    # pi 1.4+ and pdev read MCP servers from mcp.json, where `timeout` is a
    # per-request limit (default 60 s); a request waits for a human
    "pimcp": ("mcpServers", lambda cmd: {"command": cmd, "args": [], "timeout": 86400}),
    "copilot": ("mcpServers", lambda cmd: {"type": "local", "command": cmd, "args": []}),
    "opencode": ("mcp", lambda cmd: {"command": [cmd], "enabled": True}),
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


def write(path, text):
    os.makedirs(os.path.dirname(os.path.abspath(path)), exist_ok=True)
    tmp = path + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        f.write(text)
    os.replace(tmp, path)


def backup(path):
    """Back up the original once; never overwrite an existing backup."""
    if os.path.exists(path) and not os.path.exists(path + BAK):
        shutil.copy2(path, path + BAK)


def has_entry(path, name=SERVER):
    """Whether `name` is registered as an MCP server in `path`. SpeakEasy's
    approval hooks also mention its name (the binary path in a hook
    command, a managed comment), so lines that belong to a hook don't
    count."""
    text = read(path) or ""
    kept = [l for l in text.split("\n")
            if "--hook " not in l and "slack-approval-gateway hook" not in l]
    return name in "\n".join(kept)


def backup_original(path):
    """Back up a file before SpeakEasy's first change to it — only while it
    holds nothing of SpeakEasy's: a later run would otherwise back up
    SpeakEasy's own version (whose hook blocks every Slack call once the
    binary is gone)."""
    text = read(path)
    if text is not None and SERVER not in text and not os.path.exists(path + BAK):
        shutil.copy2(path, path + BAK)


def load_json(path):
    text = read(path)
    if text is None:
        return {}
    return json.loads(text)


def dump_json(path, data):
    tmp = path + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(data, f, indent=2, ensure_ascii=False)
        f.write("\n")
    os.replace(tmp, path)


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
    data = load_json(path)
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


def remove_json(style, path):
    """Uninstall path: remove only the gateway's own entries — the top-level
    one and any nested ones (e.g. claude per-project projects.<path>.mcpServers).
    The .bak-sag backup (the pre-install original) is kept for a manual
    restore; the file is never overwritten with it, so changes made after
    the install survive."""
    data = load_json(path)
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
    backup(path)
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

def zed_entry(cmd):
    return '"%s": {"command": {"path": %s, "args": []}}' % (SERVER, json.dumps(cmd))


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
    while i < len(text) and text[i] in " \t\r\n":
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
    while i < n and text[i] not in ",}]\n":
        i += 1
    return i


def add_zed(path, cmd):
    text = read(path)
    if text is None:
        text = "{}"
    if SERVER in text:
        return "already"
    backup(path)
    entry = zed_entry(cmd)
    m = re.search(r'"context_servers"\s*:\s*\{', text)
    if m:
        after = re.sub(r"//[^\n]*|/\*.*?\*/", "", text[m.end():], flags=re.S)
        if after.lstrip().startswith("}"):
            close = text.index("}", m.end())
            text = text[: m.end()] + " " + entry + " " + text[close:]
        else:
            text = text[: m.end()] + "\n    " + entry + "," + text[m.end():]
    else:
        i = text.index("{")
        text = (
            text[: i + 1]
            + '\n  "context_servers": {\n    '
            + entry
            + "\n  },"
            + text[i + 1 :]
        )
    write(path, text)
    return "added"


def remove_zed(path):
    """Text splice, the inverse of add_zed: remove only the gateway's
    context_servers entry — key, value, and one adjoining comma. Every
    other byte of the JSONC file (comments, other entries, formatting)
    is kept."""
    text = read(path)
    if text is None or SERVER not in text:
        return "absent"
    m = re.search(r'"context_servers"\s*:\s*\{', text)
    if not m:
        return "absent"
    obj_start = m.end() - 1
    obj_end = _scan_json(text, obj_start) - 1  # the matching "}"
    km = re.compile(r'"%s"\s*:' % re.escape(SERVER)).search(text, obj_start + 1, obj_end)
    if not km:
        return "absent"
    key_start = km.start()
    value_end = _scan_json(text, km.end())
    after = _skip_ws_comments(text, value_end)
    if (
        after < obj_end
        and text[after] == ","
        and _skip_ws(text, after + 1) < obj_end
    ):
        # another entry follows ours: drop our comma and the whitespace up
        # to that entry with it (comments in between are kept)
        cut = (key_start, _skip_ws(text, after + 1))
    else:
        # our entry is the last one: drop the comma in front of it, if any
        before = key_start
        while before > obj_start + 1 and text[before - 1] in " \t\r\n":
            before -= 1
        if before > obj_start + 1 and text[before - 1] == ",":
            cut = (before - 1, value_end)
        else:
            # the only entry in the object: drop the whitespace around it
            cut = (before, _skip_ws_comments(text, value_end))
    backup(path)
    write(path, text[: cut[0]] + text[cut[1]:])
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
        "    timeout: 86400" % (SERVER, json.dumps(cmd))
    )
    m = re.search(r"^extensions:\s*$", text, re.M)
    if m:
        text = text[: m.end()] + "\n" + block + text[m.end():]
    else:
        if text and not text.endswith("\n"):
            text += "\n"
        text += "extensions:\n" + block + "\n"
    write(path, text)
    return "added"


def set_goose_timeout(path, name, seconds):
    """Set the timeout: line in the gateway's goose YAML block (the
    `  name:` entry under extensions:), adding it after the block's last
    key if it is missing. Only that line changes; the rest of the file —
    comments, other extensions — is kept byte-for-byte."""
    text = read(path)
    if text is None:
        return "absent"
    m = re.search(r"^  %s:[^\n]*\n((?:    .*\n|\n)*)" % re.escape(name), text, re.M)
    if not m:
        return "absent"
    # the block must live under extensions: (goose's MCP servers key)
    parent = None
    for line in text[: m.start()].splitlines():
        pm = re.match(r"^([A-Za-z0-9_.-]+)\s*:", line)
        if pm:
            parent = pm.group(1)
    if parent != "extensions":
        return "absent"
    block = m.group(1)
    tm = re.search(r"^([ \t]+)timeout:[^\n]*\n?", block, re.M)
    if tm:
        had_nl = block[tm.end() - 1] == "\n"
        repl = "%stimeout: %d" % (tm.group(1), seconds) + ("\n" if had_nl else "")
        block = block[: tm.start()] + repl + block[tm.end():]
        result = "updated"
    else:
        block_lines = block.splitlines(keepends=True)
        insert_at = len(block_lines)
        for i in range(len(block_lines) - 1, -1, -1):
            if block_lines[i].strip():
                insert_at = i + 1
                break
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
    entry = (
        "  %s:\n"
        "    command: %s\n"
        "    args: []\n"
        "    env: {}" % (SERVER, json.dumps(cmd))
    )
    m = re.search(r"^mcp_servers:\s*$", text, re.M)
    if m:
        text = text[: m.end()] + "\n" + entry + text[m.end():]
    else:
        if text and not text.endswith("\n"):
            text += "\n"
        text += "mcp_servers:\n" + entry + "\n"
    write(path, text)
    return "added"


def _remove_yaml_block(path, header_re, name):
    text = read(path)
    if text is None:
        return "absent"
    # Remove the `  name:` entry (2-space indent) and its deeper-indented
    # children, plus a now-empty parent header if it was the only entry.
    m = re.search(r"^  %s:\n(?:    .*\n|\n)*" % re.escape(name), text, re.M)
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
    if "[mcp_servers.%s]" % SERVER in text:
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
    new = head + kept + tail
    # the removal can leave runs of blank lines where the entry was
    new = re.sub(r"\n{3,}", "\n\n", new)
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
    header_re = re.compile(r"^\[mcp_servers\.%s\][ \t]*$" % re.escape(name))
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
    backup(path)
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
    backup(path)
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
    pat = re.compile(r"^\[mcp_servers\.%s\][ \t]*$" % re.escape(name))
    for i, line in enumerate(lines):
        if pat.match(line):
            j = i + 1
            while j < len(lines) and not lines[j].startswith("["):
                j += 1
            body, existed = _set_toml_key(lines[i + 1 : j], "tool_timeout_sec", value)
            backup(path)
            write(path, "".join(lines[: i + 1] + body + lines[j:]))
            return "updated" if existed else "added"
    # vibe style: a [[mcp_servers]] element with name = "NAME"
    for t in _toml_tables(text):
        if _VIBE_ARRAY_RE.match(t["header"]) and _vibe_element_name(t) == name:
            body_lines = t["body"].splitlines(keepends=True)
            body, existed = _set_toml_key(body_lines, "tool_timeout_sec", value)
            backup(path)
            write(path, "".join(lines[: t["start"] + 1] + body + lines[t["end"] :]))
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

def instruction_add(path, variant="gateway"):
    text = read(path)
    created = text is None
    if text is None:
        text = ""
    backup(path)
    body = {"hooks": INSTRUCTION_HOOKS, "confirm": INSTRUCTION_CONFIRM}.get(variant, INSTRUCTION)
    block = MARK_BEGIN + "\n\n" + body + "\n\n" + MARK_END + "\n"
    if MARK_BEGIN in text:
        start = text.index(MARK_BEGIN)
        end = text.index(MARK_END) + len(MARK_END)
        text = text[:start] + block.rstrip("\n") + text[end:]
        write(path, text)
        return "updated"
    if text and not text.endswith("\n"):
        text += "\n"
    if text.strip():
        text += "\n"
    write(path, text + block)
    return "added" if not created else "created"


def instruction_remove(path):
    text = read(path)
    if text is None or MARK_BEGIN not in text:
        return "absent"
    start = text.index(MARK_BEGIN)
    end = text.index(MARK_END) + len(MARK_END)
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
                # path the gateway does not control.
                key = [parent] if parent else []
                if not any(
                    e["name"] == name and e["key"] == key
                    for e in finding["direct_slack_servers"]
                ):
                    finding["direct_slack_servers"].append({"name": name, "key": key})
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
    return {
        "name": str(name),
        "key": list(key or []),
        "token_env": token_env,
        "argv": argv,
        "env": {str(k): str(x) for k, x in env.items()} if isinstance(env, dict) else {},
        "url": str(url),
        "headers": headers if isinstance(headers, dict) else {},
    }


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
            summary = "korotovsky slack-mcp-server (%s)" % (", ".join(kinds) or "no token")
            if not e["argv"]:
                reason = "it runs as a remote (HTTP/SSE) server; SpeakEasy needs its stdio command"
            elif any(re.search(r"\s", a) for a in e["argv"]):
                reason = "an argument of its command contains a space, which SpeakEasy cannot pass"
            elif os.path.basename(e["argv"][0]) in _PACKAGE_RUNNERS and len(e["argv"]) == 1:
                reason = "its command is only %s, with no package to run" % e["argv"][0]
            elif not kinds:
                reason = ("no literal Slack token in its env (SLACK_MCP_XOXP_TOKEN, "
                          "SLACK_MCP_XOXB_TOKEN, or SLACK_MCP_XOXC_TOKEN + SLACK_MCP_XOXD_TOKEN)")
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
    deleted, a real credential is moved to <file>.pre-takeover."""
    if not os.path.exists(path):
        return None
    content = (read(path) or "").strip()
    if content in PLACEHOLDERS:
        os.remove(path)
        return "removed placeholder %s" % path
    os.replace(path, path + ".pre-takeover")
    os.chmod(path + ".pre-takeover", 0o600)
    return "moved %s to %s.pre-takeover" % (path, path)


def _installed_server_argv(argv):
    """The command to run, with its program as an absolute path. A package
    runner line (npx/bunx/pnpx … slack-mcp-server[@version] ARGS) becomes the
    installed `slack-mcp-server ARGS` when one is on PATH, so SpeakEasy runs
    the server already installed instead of downloading it."""
    runner = os.path.basename(argv[0])
    if runner in _PACKAGE_RUNNERS:
        for i, a in enumerate(argv[1:], 1):
            if re.match(r"^slack-mcp-server(@[^\s]*)?$", a):
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
        raise SystemExit("%s has comments or is not plain JSON; SpeakEasy will not rewrite it. "
                         "Add or remove its hook by hand (see README.md, \"Approval hooks\")" % path)
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
    elif cmd == "add-zed":
        path, gateway = args
        print("%s: %s" % (path, add_zed(path, gateway)))
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
        # instruction-add FILE [gateway|hooks]
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
