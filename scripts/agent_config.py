#!/usr/bin/env python3
"""Edit coding-agent MCP configs to register (or remove) the Slack Approval
Gateway stdio server.

Every file is backed up once to `<file>.bak-sag` before the first
modification (the backup is the pre-install original; `--uninstall` restores
it). All operations are idempotent: adding an already-registered server
updates the entry in place, and a `--dry-run` prints what would change.

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
  remove-json STYLE FILE      unregister (restores the .bak-sag backup when
                              present, else removes the entry)
  remove-zed|remove-goose|remove-hermes|remove-codex FILE
                              unregister from a non-JSON config
  remove-entry STYLE FILE NAME
                              remove an arbitrary server entry (used by
                              install.sh --enforce to drop direct Slack servers)
  remove-codex-entry FILE NAME
                              remove a [mcp_servers.NAME] table from a TOML
                              config (used by install.sh --enforce)
  instruction-add FILE        insert/replace the managed "use the gateway"
                              instruction block in an agent instruction file
  instruction-remove FILE     remove the managed instruction block
  has FILE [NAME]             exit 0 when NAME (default: the gateway) is
                              registered in FILE
  audit FILE...               JSON report of direct Slack MCP servers and
                              Slack tokens found in the given config files
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

To send a Slack message, call the MCP tool `request_slack_message` (server `slack-approval-gateway`). A desktop popup shows the exact message and destination for approval; only the exact approved text is sent to the exact approved destination. Do not send Slack messages by any other route — no Slack MCP servers, curl, webhooks, SDKs, CLIs, scripts, or the browser. Reading Slack is fine."""

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
MCP_PARENT_KEYS = {"mcpServers", "mcp", "context_servers", "mcp_servers"}


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
    if os.path.exists(path + BAK):
        shutil.copy2(path + BAK, path)
        return "restored backup"
    keypath, _ = JSON_STYLES[style]
    data = load_json(path)
    if not isinstance(data, dict):
        return "absent"
    container, key = parent_of(data, keypath)
    servers = container.get(key)
    if not isinstance(servers, dict) or SERVER not in servers:
        return "absent"
    del servers[SERVER]
    dump_json(path, data)
    return "removed"


def remove_entry(style, path, name):
    """Remove an arbitrary server entry (install.sh --enforce uses this to
    drop direct Slack servers from agent configs)."""
    backup(path)
    keypath, _ = JSON_STYLES[style]
    data = load_json(path)
    if not isinstance(data, dict):
        return "absent"
    container, key = parent_of(data, keypath)
    servers = container.get(key)
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
    if os.path.exists(path + BAK):
        shutil.copy2(path + BAK, path)
        return "restored backup"
    text = read(path)
    if text is None or SERVER not in text:
        return "absent"
    data = json.loads(strip_jsonc(text))
    servers = data.get("context_servers")
    if isinstance(servers, dict) and SERVER in servers:
        del servers[SERVER]
        dump_json(path, data)
        return "removed"
    return "absent"


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
    block = (
        "  %s:\n"
        "    enabled: true\n"
        "    type: stdio\n"
        "    cmd: %s\n"
        "    args: []\n"
        "    envs: {}\n"
        "    timeout: 120" % (SERVER, json.dumps(cmd))
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
    if os.path.exists(path + BAK):
        shutil.copy2(path + BAK, path)
        return "restored backup"
    text = read(path)
    if text is None:
        return "absent"
    # Remove the `  name:` entry (2-space indent) and its deeper-indented
    # children, plus a now-empty parent header if it was the only entry.
    m = re.search(r"^  %s:\n(?:    .*\n|\n)*" % re.escape(name), text, re.M)
    if not m:
        return "absent"
    text = text[: m.start()] + text[m.end():]
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
    text += '\n[mcp_servers.%s]\ncommand = %s\nargs = []\n' % (SERVER, json.dumps(cmd))
    write(path, text)
    return "added"


def remove_codex(path):
    if os.path.exists(path + BAK):
        shutil.copy2(path + BAK, path)
        return "restored backup"
    return remove_codex_entry(path, SERVER)


def remove_codex_entry(path, name):
    """Remove a `[mcp_servers.<name>]` table (and its subtables, e.g.
    `[mcp_servers.<name>.env]`) from a TOML config. install.sh --enforce uses
    this to drop direct Slack servers from codex configs."""
    text = read(path)
    if text is None:
        return "absent"
    # The table runs from its header line to the next line that starts a new
    # table header (`[` at column 0); value lines like `args = [...]` do not
    # start with `[`. Subtables ([mcp_servers.NAME.env]) are consumed too.
    pattern = (
        r"\n?(?:\[mcp_servers\.%s(?:\.[^\]\n]*)?\]\n(?:(?!\[)[^\n]*\n?)*)+" % re.escape(name)
    )
    m = re.search(pattern, text)
    if not m:
        return "absent"
    backup(path)
    write(path, text[: m.start()] + "\n" + text[m.end():])
    return "removed"


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


def audit_file(path):
    text = read(path)
    if text is None:
        return None
    finding = {"file": path, "direct_slack_servers": [], "slack_tokens": []}
    for m in TOKEN_RE.finditer(text):
        finding["slack_tokens"].append(_mask(m.group(0)))
    try:
        data = json.loads(strip_jsonc(text))
    except ValueError:
        data = None
    if isinstance(data, dict):
        def scan(obj, parent_key):
            if isinstance(obj, dict):
                for k, v in obj.items():
                    if (
                        parent_key in MCP_PARENT_KEYS
                        and isinstance(v, dict)
                        and "slack" in k.lower()
                        and k != SERVER
                    ):
                        finding["direct_slack_servers"].append(k)
                    scan(v, k)
            elif isinstance(obj, list):
                for v in obj:
                    scan(v, parent_key)

        scan(data, None)
    else:
        # YAML / plain text: report slack-named keys under any 2+-space indent.
        for m in re.finditer(
            r"^\s{2,}([A-Za-z0-9_.-]*slack[A-Za-z0-9_.-]*)\s*:", text, re.M
        ):
            if m.group(1) != SERVER:
                finding["direct_slack_servers"].append(m.group(1))
    # TOML: [mcp_servers.<name>] tables with slack in the name (a .env
    # subtable is part of its server, not a server of its own).
    for m in re.finditer(
        r"^\[mcp_servers\.([A-Za-z0-9_.-]*slack[A-Za-z0-9_.-]*)\]", text, re.M
    ):
        name = m.group(1).split(".")[0]
        if name != SERVER and name not in finding["direct_slack_servers"]:
            finding["direct_slack_servers"].append(name)
    return finding


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
    elif cmd == "remove-json":
        style, path = args
        print("%s: %s" % (path, remove_json(style, path)))
    elif cmd in ("remove-zed", "remove-goose", "remove-hermes", "remove-codex"):
        (path,) = args
        fn = {
            "remove-zed": remove_zed,
            "remove-goose": remove_goose,
            "remove-hermes": remove_hermes,
            "remove-codex": remove_codex,
        }[cmd]
        print("%s: %s" % (path, fn(path)))
    elif cmd == "remove-entry":
        style, path, name = args
        print("%s: %s" % (path, remove_entry(style, path, name)))
    elif cmd == "remove-codex-entry":
        path, name = args
        print("%s: %s" % (path, remove_codex_entry(path, name)))
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
