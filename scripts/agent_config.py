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
                              pi     (mcpServers — ~/.pi/agent/settings.json,
                                      also pdev: ~/.paradev/agent/settings.json)
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
  instruction-add FILE        insert/replace the managed "use the gateway"
                              instruction block in an agent instruction file
  instruction-remove FILE     remove the managed instruction block
  has FILE [NAME]             exit 0 when NAME (default: the gateway) is
                              registered in FILE
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

import json
import os
import re
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

# STYLE -> (parent key, entry builder). A parent key starting with "@" is a
# single literal key (amp settings use the flat "amp.mcpServers" key, VS
# Code-style); otherwise the key is a dotted path into nested objects.
JSON_STYLES = {
    "claude": ("mcpServers", lambda cmd: {"type": "stdio", "command": cmd, "args": []}),
    "gemini": ("mcpServers", lambda cmd: {"command": cmd, "args": []}),
    "pi": ("mcpServers", lambda cmd: {"command": cmd, "args": []}),
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
        with open(path, "r", encoding="utf-8") as f:
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


def load_json(path):
    text = read(path)
    if text is None:
        return {}
    return json.loads(text)


def dump_json(path, data):
    tmp = path + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(data, f, indent=2)
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

def instruction_add(path):
    text = read(path)
    created = text is None
    if text is None:
        text = ""
    backup(path)
    block = MARK_BEGIN + "\n\n" + INSTRUCTION + "\n\n" + MARK_END + "\n"
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


def enforce_remove(path, name, key):
    """Remove one direct Slack server found by the audit (install.sh
    --enforce). Prints an accurate status line — [removed], [absent],
    [failed], or [edit by hand] — instead of assuming success."""
    base = os.path.basename(path)
    if base == ".claude.json":
        style = "claude"
    elif base == "opencode.json":
        style = "opencode"
    elif base == "mcp-config.json":
        style = "copilot"
    elif base == "settings.json" and "amp" in path:
        style = "amp"
    elif base == "settings.json" and (".pi/" in path or ".paradev/" in path):
        style = "pi"
    elif base == "settings.json" and ".gemini/" in path:
        style = "gemini"
    elif base == "config.toml" and ".vibe" in path:
        style = "vibe"
    elif base == "config.toml":
        style = "codex"
    else:
        style = None
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
    elif cmd == "instruction-add":
        (path,) = args
        print("%s: %s" % (path, instruction_add(path)))
    elif cmd == "instruction-remove":
        (path,) = args
        print("%s: %s" % (path, instruction_remove(path)))
    elif cmd == "has":
        path = args[0]
        name = args[1] if len(args) > 1 else SERVER
        text = read(path) or ""
        sys.exit(0 if name in text else 1)
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
