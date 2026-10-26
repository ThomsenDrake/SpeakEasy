#!/usr/bin/env python3
"""Tests for scripts/agent_config.py: every config style, idempotency,
backups, removal, instruction blocks, and the audit report."""

import contextlib
import io
import json
import os
import shutil
import subprocess
import sys
import tempfile

try:
    import tomllib
except ImportError:  # macOS's /usr/bin/python3 is 3.9: pip vendors tomli
    from pip._vendor import tomli as tomllib

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "scripts"))

import agent_config as ac  # noqa: E402

CMD = "/Users/test/.local/bin/slack-approval-gateway"
failures = []


def check(name, cond, detail=""):
    status = "PASS" if cond else "FAIL"
    print(f"[{status}] {name}" + (f" — {detail}" if detail and not cond else ""))
    if not cond:
        failures.append(name)


def tmpdir():
    return tempfile.mkdtemp(prefix="sag-cfg-test-")


def write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(text)


def names(finding, field):
    """Server names in an audit finding field (entries are {name, key})."""
    return [e["name"] for e in finding[field]]


# --------------------------------------------------------------- JSON styles
d = tmpdir()

# claude: existing mcpServers with another server; file must keep it
p = os.path.join(d, "claude.json")
write(p, json.dumps({"numStartups": 3, "mcpServers": {"grafana": {"type": "stdio", "command": "docker"}}}))
check("claude: added", ac.add_json("claude", p, CMD) == "added")
data = json.load(open(p))
check("claude: entry shape", data["mcpServers"][ac.SERVER] == {"type": "stdio", "command": CMD, "args": []}, json.dumps(data))
check("claude: existing server kept", "grafana" in data["mcpServers"])
check("claude: backup made", os.path.exists(p + ac.BAK))
check("claude: idempotent update", ac.add_json("claude", p, CMD) == "updated")
check("claude: has", subprocess.run([sys.executable, os.path.join(ROOT, "scripts", "agent_config.py"), "has", p]).returncode == 0)
# uninstall semantics: remove only our entry; a change the user made after
# the install survives; the .bak-sag backup (the pre-install original) is
# kept, never restored over the file
data = json.load(open(p))
data["numStartups"] = 99
with open(p, "w") as f:
    json.dump(data, f)
check("claude: remove drops only our entry", ac.remove_json("claude", p) == "removed")
data = json.load(open(p))
check("claude: user edits survive the uninstall", data["numStartups"] == 99 and "grafana" in data["mcpServers"], json.dumps(data))
check("claude: our entry gone", ac.SERVER not in data["mcpServers"])
check("claude: backup kept (pre-install original)", "slack-approval-gateway" not in open(p + ac.BAK).read() and "grafana" in open(p + ac.BAK).read())
check("claude: remove again is absent", ac.remove_json("claude", p) == "absent")

# gemini / pi / copilot: missing file and missing parent key
for style, parent in (("gemini", "mcpServers"), ("pi", "mcpServers"), ("copilot", "mcpServers")):
    p = os.path.join(d, style + ".json")
    check(f"{style}: created", ac.add_json(style, p, CMD) == "added")
    data = json.load(open(p))
    check(f"{style}: parent created", parent in data and ac.SERVER in data[parent])

p = os.path.join(d, "copilot.json")
data = json.load(open(p))
check("copilot: entry has type local", data["mcpServers"][ac.SERVER]["type"] == "local", json.dumps(data))

# opencode: mcp parent, command as array
p = os.path.join(d, "opencode.json")
write(p, json.dumps({"$schema": "https://opencode.ai/config.json", "mcp": {"executor": {"type": "remote", "url": "https://x"}}}))
check("opencode: added", ac.add_json("opencode", p, CMD) == "added")
data = json.load(open(p))
check("opencode: entry shape", data["mcp"][ac.SERVER] == {"command": [CMD], "enabled": True}, json.dumps(data))
check("opencode: existing kept", "executor" in data["mcp"])

# amp: flat "amp.mcpServers" key (VS Code-style settings key)
p = os.path.join(d, "amp-settings.json")
write(p, json.dumps({"amp.dangerouslyAllowAll": True, "amp.mcpServers": {"RepoPromptCE": {"command": "/x"}}}))
check("amp: added", ac.add_json("amp", p, CMD) == "added")
data = json.load(open(p))
check("amp: flat entry", data["amp.mcpServers"][ac.SERVER] == {"command": CMD, "args": []}, json.dumps(data))
check("amp: existing kept", "RepoPromptCE" in data["amp.mcpServers"])
check("amp: no nested amp object created", "amp" not in data or not isinstance(data.get("amp"), dict) or "mcpServers" not in data.get("amp", {}), json.dumps(data))

# remove-entry (enforce path): drop the direct slack server, keep ours
p = os.path.join(d, "enforce.json")
write(p, json.dumps({"mcpServers": {"slack": {"type": "stdio", "command": "go", "env": {"SLACK_MCP_XOXC_TOKEN": "xoxc-1234567890-abcdef"}}, ac.SERVER: {"command": CMD}}}))
check("enforce: remove-entry drops slack", ac.remove_entry("claude", p, "slack") == "removed")
data = json.load(open(p))
check("enforce: gateway kept, slack gone", ac.SERVER in data["mcpServers"] and "slack" not in data["mcpServers"])
check("enforce: backup kept the original (with the token)", "xoxc-1234567890" in open(p + ac.BAK).read())

# remove-json without backup removes just the entry
p = os.path.join(d, "nobackup.json")
write(p, json.dumps({"mcpServers": {ac.SERVER: {"command": CMD}}}))
os.remove(p + ac.BAK) if os.path.exists(p + ac.BAK) else None
ac.add_json("gemini", p, CMD)  # makes a backup; delete it to simulate no-backup
os.remove(p + ac.BAK)
check("remove without backup", ac.remove_json("gemini", p) == "removed")
check("remove without backup: entry gone", ac.SERVER not in json.load(open(p)).get("mcpServers", {}))

# ------------------------------------------------------------------- zed
p = os.path.join(d, "zed-settings.json")
write(p, '// Zed settings\n{\n  // language models\n  "context_servers": {\n    "existing": {"command": {"path": "npx"}},\n  },\n}\n')
check("zed: added to existing context_servers", ac.add_zed(p, CMD) == "added")
text = open(p).read()
data = json.loads(ac.strip_jsonc(text))
check("zed: entry shape", data["context_servers"][ac.SERVER] == {"command": {"path": CMD, "args": []}}, json.dumps(data))
check("zed: comments preserved", "// Zed settings" in text and "// language models" in text)
check("zed: existing kept", "existing" in data["context_servers"])
check("zed: idempotent", ac.add_zed(p, CMD) == "already")

p = os.path.join(d, "zed-empty.json")
write(p, '{\n  "context_servers": {},\n}\n')
check("zed: empty object handled", ac.add_zed(p, CMD) == "added")
data = json.loads(ac.strip_jsonc(open(p).read()))
check("zed: empty object filled", ac.SERVER in data["context_servers"], open(p).read())

p = os.path.join(d, "zed-missing.json")
write(p, '// header\n{\n  "ui_font_size": 14,\n}\n')
check("zed: missing key created", ac.add_zed(p, CMD) == "added")
data = json.loads(ac.strip_jsonc(open(p).read()))
check("zed: context_servers created", ac.SERVER in data.get("context_servers", {}), open(p).read())
check("zed: other keys kept", data.get("ui_font_size") == 14)

# ------------------------------------------------------------- goose/hermes
p = os.path.join(d, "goose.yaml")
write(p, "GOOSE_TELEMETRY_ENABLED: false\nextensions:\n  chatrecall:\n    enabled: false\n    type: platform\n")
check("goose: added under extensions", ac.add_goose(p, CMD) == "added")
text = open(p).read()
check("goose: block shape", "  slack-approval-gateway:\n    enabled: true\n    type: stdio\n    cmd: \"%s\"\n    args: []\n    envs: {}\n    timeout: 86400" % CMD in text, text)
check("goose: long tool timeout (an approval can take minutes)", "timeout: 86400" in text and "timeout: 120" not in text, text)
check("goose: existing extension kept", "chatrecall" in text)
check("goose: idempotent", ac.add_goose(p, CMD) == "already")
# a change the user made after the install must survive the uninstall
write(p, text.replace("GOOSE_TELEMETRY_ENABLED: false", "GOOSE_TELEMETRY_ENABLED: true"))
check("goose: remove drops only our block", ac.remove_goose(p) == "removed")
text = open(p).read()
check("goose: entry gone, user and other content kept", "slack-approval-gateway" not in text and "chatrecall" in text and "GOOSE_TELEMETRY_ENABLED: true" in text, text)
check("goose: backup kept (pre-install original)", os.path.exists(p + ac.BAK) and "slack-approval-gateway" not in open(p + ac.BAK).read())

# textual removal when no backup exists
p = os.path.join(d, "goose-nobackup.yaml")
write(p, "extensions:\n  chatrecall:\n    enabled: false\n")
ac.add_goose(p, CMD)
os.remove(p + ac.BAK)
check("goose: textual remove", ac.remove_goose(p) == "removed")
check("goose: entry gone, other kept", "slack-approval-gateway" not in open(p).read() and "chatrecall" in open(p).read())

p = os.path.join(d, "goose-noext.yaml")
write(p, "GOOSE_TELEMETRY_ENABLED: false\n")
check("goose: missing extensions key", ac.add_goose(p, CMD) == "added")
check("goose: extensions created", "extensions:\n  slack-approval-gateway:" in open(p).read())

p = os.path.join(d, "hermes.yaml")
write(p, "telemetry:\n  shared_metrics:\n    enabled: false\nplatform_toolsets:\n  slack:\n    - hermes-slack\n")
check("hermes: added", ac.add_hermes(p, CMD) == "added")
text = open(p).read()
check("hermes: block shape", "mcp_servers:\n  slack-approval-gateway:\n    command: \"%s\"\n    args: []\n    env: {}" % CMD in text, text)
check("hermes: platform_toolsets untouched", "hermes-slack" in text)
check("hermes: idempotent", ac.add_hermes(p, CMD) == "already")

# hermes with an existing mcp_servers block
p = os.path.join(d, "hermes-existing.yaml")
write(p, "mcp_servers:\n  other:\n    command: npx\n")
check("hermes: existing mcp_servers", ac.add_hermes(p, CMD) == "added")
text = open(p).read()
check("hermes: inserted under existing key", "mcp_servers:\n  slack-approval-gateway:" in text and "other:" in text, text)

# ------------------------------------------------------------------- codex
p = os.path.join(d, "codex.toml")
write(p, 'model = "gpt-6.1-sol"\n')
check("codex: added", ac.add_codex(p, CMD) == "added")
text = open(p).read()
check("codex: block shape", '[mcp_servers.%s]\ncommand = "%s"\nargs = []\ntool_timeout_sec = 86400' % (ac.SERVER, CMD) in text, text)
check("codex: long tool timeout (an approval can take minutes)", "tool_timeout_sec = 86400" in text, text)
check("codex: existing config kept", 'model = "gpt-6.1-sol"' in text)
check("codex: idempotent", ac.add_codex(p, CMD) == "already")
# a change the user made after the install must survive the uninstall
write(p, text.replace('model = "gpt-6.1-sol"', 'model = "gpt-6.2"'))
check("codex: remove drops only our table", ac.remove_codex(p) == "removed")
text = open(p).read()
check("codex: entry gone, user edit kept", ac.SERVER not in text and 'model = "gpt-6.2"' in text, text)
check("codex: backup kept (pre-install original)", os.path.exists(p + ac.BAK) and ac.SERVER not in open(p + ac.BAK).read())

# textual removal when no backup exists
p = os.path.join(d, "codex-nobackup.toml")
write(p, 'model = "gpt-6.1-sol"\n')
ac.add_codex(p, CMD)
os.remove(p + ac.BAK)
check("codex: textual remove", ac.remove_codex(p) == "removed")
check("codex: entry gone, other kept", ac.SERVER not in open(p).read() and "gpt-6.1-sol" in open(p).read())

# ------------------------------------------------------------- instructions
p = os.path.join(d, "CLAUDE.md")
write(p, "# My instructions\n\nBe nice.\n")
check("instruction: added", ac.instruction_add(p) == "added")
text = open(p).read()
check("instruction: markers present", ac.MARK_BEGIN in text and ac.MARK_END in text)
check("instruction: original kept", "Be nice." in text)
check("instruction: idempotent update", ac.instruction_add(p) == "updated")
check("instruction: single block", text.count(ac.MARK_BEGIN) == open(p).read().count(ac.MARK_BEGIN) == 1)
check("instruction: remove", ac.instruction_remove(p) == "removed")
check("instruction: block gone, original kept", ac.MARK_BEGIN not in open(p).read() and "Be nice." in open(p).read())

p = os.path.join(d, "fresh.md")
check("instruction: created", ac.instruction_add(p) == "created")
check("instruction: remove deletes our-only file", ac.instruction_remove(p) == "removed (file was ours; deleted)" and not os.path.exists(p))

# ------------------------------------------------------- codex TOML entries
p = os.path.join(d, "codex-slack.toml")
write(p, 'model = "gpt"\n\n[mcp_servers.unofficial_slack]\ncommand = "go"\nargs = ["run", "github.com/korotovsky/slack-mcp-server/cmd/slack-mcp-server@latest"]\nenv = { SLACK_MCP_XOXC_TOKEN = "xoxc-1234567890-abcdef" }\n\n[mcp_servers.unofficial_slack.env]\nSLACK_MCP_XOXD_TOKEN = "xoxd-abcdef"\n\n[mcp_servers.other]\ncommand = "npx"\n')
check("codex-entry: removed with subtable", ac.remove_codex_entry(p, "unofficial_slack") == "removed")
text = open(p).read()
check("codex-entry: table and env subtable gone", "unofficial_slack" not in text and "xoxd-abcdef" not in text, text)
check("codex-entry: other servers kept", "[mcp_servers.other]" in text and 'model = "gpt"' in text)

# a comment the user appended after our table (our writers append at EOF)
# is the user's, not ours: it must survive the removal
p = os.path.join(d, "codex-trailing-comment.toml")
write(p, 'model = "gpt"\n')
ac.add_codex(p, CMD)
with open(p, "a") as f:
    f.write("# my note about the next table\n\n[other]\nkey = 1\n")
check("codex-entry: trailing user comment kept", ac.remove_codex_entry(p, ac.SERVER) == "removed")
text = open(p).read()
check("codex-entry: comment and next table survive",
      "# my note about the next table" in text and "[other]" in text and ac.SERVER not in text,
      text)
tomllib.loads(text)

p = os.path.join(d, "vibe-trailing-comment.toml")
write(p, '[[mcp_servers]]\nname = "other"\ncommand = "npx"\n')
ac.add_vibe(p, CMD)
with open(p, "a") as f:
    f.write("# my vibe note\n")
check("vibe-entry: trailing user comment kept", ac.remove_vibe(p) == "removed")
text = open(p).read()
check("vibe-entry: comment and other element survive",
      "# my vibe note" in text and 'name = "other"' in text and ac.SERVER not in text,
      text)
tomllib.loads(text)

# ------------------------------------------------------------------- audit
p = os.path.join(d, "audit-claude.json")
write(p, json.dumps({"mcpServers": {
    "slack": {"type": "stdio", "command": "go", "env": {"SLACK_MCP_XOXC_TOKEN": "xoxc-5330704053091-9437214043493-abcdef0123456789"}},
    ac.SERVER: {"type": "stdio", "command": CMD},
}}))
f = ac.audit_file(p)
check("audit: finds direct slack server", names(f, "direct_slack_servers") == ["slack"], json.dumps(f))
check("audit: finds token masked", len(f["slack_tokens"]) == 1 and f["slack_tokens"][0].startswith("xoxc-5330704") and "…" in f["slack_tokens"][0] and "9437214043493" not in f["slack_tokens"][0], json.dumps(f))
check("audit: gateway not flagged", ac.SERVER not in names(f, "direct_slack_servers"))

p = os.path.join(d, "audit-hermes.yaml")
write(p, "platform_toolsets:\n  slack:\n    - hermes-slack\nmcp_servers:\n  executor:\n    url: https://x\n")
f = ac.audit_file(p)
check("audit: yaml slack key found", "slack" in names(f, "direct_slack_servers"), json.dumps(f))

p = os.path.join(d, "audit-clean.json")
write(p, json.dumps({"mcpServers": {ac.SERVER: {"command": CMD}}}))
f = ac.audit_file(p)
check("audit: clean file", names(f, "direct_slack_servers") == [] and f["slack_tokens"] == [], json.dumps(f))

p = os.path.join(d, "audit-codex.toml")
write(p, '[mcp_servers.unofficial_slack]\ncommand = "go"\n\n[mcp_servers.%s]\ncommand = "%s"\n' % (ac.SERVER, CMD))
f = ac.audit_file(p)
check("audit: toml slack server found", names(f, "direct_slack_servers") == ["unofficial_slack"], json.dumps(f))
check("audit: toml gateway not flagged", ac.SERVER not in names(f, "direct_slack_servers"))

check("audit: missing file skipped", ac.audit_file(os.path.join(d, "nope.json")) is None)

# ------------------------------------- audit: expected vs violation categories
check("classify: official slack mcp", ac.classify_server("slack", json.dumps({"url": "https://mcp.slack.com/mcp", "oauth": {}})) == "slack-mcp")
check("classify: token in entry wins", ac.classify_server("slack", json.dumps({"env": {"SLACK_TOKEN": "xoxb-1234567890-abcdef"}})) == "direct")
check("classify: mistral by url", ac.classify_server("mistral", json.dumps({"url": "https://api.mistral.ai/v1/mcp"})) == "mistral")
check("classify: mistral by name", ac.classify_server("mistral-connectors", "{}") == "mistral")
check("classify: unclassifiable slack server", ac.classify_server("my-slack", json.dumps({"command": "npx", "args": ["slack-mcp"]})) == "direct")
check("classify: unrelated server", ac.classify_server("grafana", json.dumps({"command": "docker"})) == "other")

p = os.path.join(d, "audit-expected.json")
write(p, json.dumps({"mcpServers": {
    "slack": {"type": "http", "url": "https://mcp.slack.com/mcp",
              "oauth": {"clientId": "123.456", "callbackPort": 3118}},
    "mistral": {"type": "http", "url": "https://api.mistral.ai/v1/mcp",
                "env": {"MISTRAL_API_KEY": "mstrl_abcdefghij0123456789KLMNOP"}},
    "grafana": {"type": "stdio", "command": "docker"},
    ac.SERVER: {"type": "stdio", "command": CMD},
}}))
f = ac.audit_file(p)
check("audit: official slack mcp is expected", names(f, "slack_mcp_servers") == ["slack"], json.dumps(f))
check("audit: mistral connector is expected", names(f, "mistral_connector_servers") == ["mistral"], json.dumps(f))
check("audit: no direct servers", names(f, "direct_slack_servers") == [], json.dumps(f))
check("audit: mistral key masked",
      len(f["mistral_keys"]) == 1 and f["mistral_keys"][0].startswith("mstrl_abcdef")
      and "…" in f["mistral_keys"][0] and "0123456789" not in f["mistral_keys"][0],
      json.dumps(f))
check("audit: unrelated server ignored",
      "grafana" not in names(f, "slack_mcp_servers") + names(f, "mistral_connector_servers") + names(f, "direct_slack_servers"),
      json.dumps(f))

p = os.path.join(d, "audit-codex-official.toml")
write(p, '[mcp_servers.slack]\ntype = "http"\nurl = "https://mcp.slack.com/mcp"\n\n[mcp_servers.%s]\ncommand = "%s"\n' % (ac.SERVER, CMD))
f = ac.audit_file(p)
check("audit: toml official slack mcp is expected", names(f, "slack_mcp_servers") == ["slack"], json.dumps(f))
check("audit: toml direct servers empty", names(f, "direct_slack_servers") == [], json.dumps(f))
check("audit: toml gateway not flagged", ac.SERVER not in names(f, "slack_mcp_servers") + names(f, "direct_slack_servers"), json.dumps(f))

p = os.path.join(d, "audit-goose.yaml")
write(p, "extensions:\n  slack:\n    enabled: true\n    type: http\n    url: https://mcp.slack.com/mcp\n  chatrecall:\n    enabled: false\n")
f = ac.audit_file(p)
check("audit: yaml official slack mcp is expected", names(f, "slack_mcp_servers") == ["slack"], json.dumps(f))
check("audit: yaml unrelated extension ignored",
      "chatrecall" not in names(f, "direct_slack_servers") + names(f, "slack_mcp_servers"), json.dumps(f))

p = os.path.join(d, "audit-amp.json")
write(p, json.dumps({"amp.mcpServers": {
    "slack": {"url": "https://mcp.slack.com/mcp"},
    ac.SERVER: {"command": CMD},
}}))
f = ac.audit_file(p)
check("audit: amp flat key scanned", names(f, "slack_mcp_servers") == ["slack"], json.dumps(f))

p = os.path.join(d, "audit-goose-providers.yaml")
write(p, "active_provider: mistral\nproviders:\n  mistral:\n    enabled: true\n    model: zai-glm-5-2\n    context_limit: 1000000\n")
f = ac.audit_file(p)
check("audit: yaml llm provider is not a connector server",
      names(f, "mistral_connector_servers") == [] and names(f, "direct_slack_servers") == [],
      json.dumps(f))

p = os.path.join(d, "audit-hermes-providers.yaml")
write(p, "providers:\n  mistral-glm-5-2:\n    name: Mistral GLM 5.2\n    base_url: https://api.mistral.ai/v1\n    key_env: MISTRAL_API_KEY\n")
f = ac.audit_file(p)
check("audit: yaml provider with base_url is not a connector server",
      names(f, "mistral_connector_servers") == [] and names(f, "direct_slack_servers") == [],
      json.dumps(f))

p = os.path.join(d, "audit-hermes-platform.yaml")
write(p, "platform_toolsets:\n  slack:\n    - hermes-slack\nmcp_servers:\n  executor:\n    url: https://x\n")
f = ac.audit_file(p)
check("audit: hermes platform slack still reported as direct",
      "slack" in names(f, "direct_slack_servers"), json.dumps(f))
check("audit: hermes executor not flagged", "executor" not in names(f, "direct_slack_servers"), json.dumps(f))

# ------------------------------------------- audit + edits: vibe TOML style
# Vibe stores servers as [[mcp_servers]] array elements; [mcp_servers.env]
# after an element is that element's sub-table, not a server.
p = os.path.join(d, "audit-vibe.toml")
write(p, """# vibe config
[[mcp_servers]]
name = "slack"
transport = "http"
url = "https://mcp.slack.com/mcp"

[[mcp_servers]]
name = "direct-slack"
transport = "stdio"
command = "npx"
args = ["-y", "@korotovsky/slack-mcp-server@latest"]

[mcp_servers.env]
SLACK_MCP_XOXC_TOKEN = "xoxc-5330704053091-9437214043493-abcdef0123456789"

[[mcp_servers]]
name = "mistral"
transport = "http"
url = "https://api.mistral.ai/v1/mcp"

[[mcp_servers]]
name = "slack-approval-gateway"
transport = "stdio"
command = "%s"
args = []
tool_timeout_sec = 86400
""" % CMD)
f = ac.audit_file(p)
check("audit: vibe official slack mcp is expected", names(f, "slack_mcp_servers") == ["slack"], json.dumps(f))
check("audit: vibe token-based slack server is direct", names(f, "direct_slack_servers") == ["direct-slack"], json.dumps(f))
check("audit: vibe [mcp_servers.env] is not a server",
      "env" not in names(f, "slack_mcp_servers") + names(f, "mistral_connector_servers") + names(f, "direct_slack_servers"),
      json.dumps(f))
check("audit: vibe mistral connector is expected", names(f, "mistral_connector_servers") == ["mistral"], json.dumps(f))
check("audit: vibe gateway element not flagged",
      ac.SERVER not in names(f, "direct_slack_servers") + names(f, "slack_mcp_servers"),
      json.dumps(f))
check("audit: vibe token masked", len(f["slack_tokens"]) == 1 and f["slack_tokens"][0].startswith("xoxc-5330704"), json.dumps(f))

# a codex-style file keeps its meaning: [mcp_servers.X] tables are servers
p = os.path.join(d, "audit-codex-env.toml")
write(p, '[mcp_servers.slack]\ncommand = "npx"\n\n[mcp_servers.slack.env]\nSLACK_MCP_XOXC_TOKEN = "xoxc-5330704053091-9437214043493-abcdef0123456789"\n')
f = ac.audit_file(p)
check("audit: codex env subtable folds into its server (direct)",
      names(f, "direct_slack_servers") == ["slack"] and "env" not in names(f, "direct_slack_servers"),
      json.dumps(f))

# ------------------------------------------------- vibe add/remove (TOML)
p = os.path.join(d, "vibe-add.toml")
write(p, 'model = "magistral-medium-latest"\n')
check("vibe: added", ac.add_vibe(p, CMD) == "added")
text = open(p).read()
check("vibe: element shape", '[[mcp_servers]]\nname = "%s"\ncommand = "%s"\nargs = []\ntool_timeout_sec = 86400' % (ac.SERVER, CMD) in text, text)
check("vibe: long tool timeout (an approval can take minutes)", "tool_timeout_sec = 86400" in text, text)
check("vibe: existing config kept", 'model = "magistral-medium-latest"' in text)
check("vibe: idempotent", ac.add_vibe(p, CMD) == "already")
check("vibe: backup made", os.path.exists(p + ac.BAK))
tomllib.loads(open(p).read())  # must stay valid TOML

p = os.path.join(d, "vibe-remove.toml")
write(p, """[[mcp_servers]]
name = "other"
command = "npx"

[[mcp_servers]]
name = "direct-slack"
command = "npx"

[mcp_servers.env]
SLACK_MCP_XOXC_TOKEN = "xoxc-5330704053091-9437214043493-abcdef0123456789"

[[mcp_servers]]
name = "slack-approval-gateway"
command = "%s"
""" % CMD)
check("vibe-entry: removed with subtable", ac.remove_vibe_entry(p, "direct-slack") == "removed")
text = open(p).read()
check("vibe-entry: element and env subtable gone", "direct-slack" not in text and "xoxc-5330704" not in text, text)
check("vibe-entry: other elements kept", 'name = "other"' in text and 'name = "%s"' % ac.SERVER in text, text)
tomllib.loads(text)  # must stay valid TOML
check("vibe-entry: absent when missing", ac.remove_vibe_entry(p, "direct-slack") == "absent")
check("vibe: remove gateway element", ac.remove_vibe(p) == "removed")
text = open(p).read()
check("vibe: gateway gone, other kept", ac.SERVER not in text and 'name = "other"' in text, text)
check("vibe: backup kept", os.path.exists(p + ac.BAK))
tomllib.loads(text)

# a vibe file whose only element is ours: removal leaves a valid (empty) file
p = os.path.join(d, "vibe-only-ours.toml")
write(p, '[[mcp_servers]]\nname = "%s"\ncommand = "%s"\n' % (ac.SERVER, CMD))
check("vibe: remove the only element", ac.remove_vibe(p) == "removed")
tomllib.loads(open(p).read())

# ------------------------------------------------------------- set-timeout
p = os.path.join(d, "timeout-codex.toml")
write(p, '[mcp_servers.%s]\ncommand = "%s"\n' % (ac.SERVER, CMD))
check("set-timeout: added to codex table", ac.set_toml_timeout(p, ac.SERVER, 86400) == "added")
check("set-timeout: codex table has it", "tool_timeout_sec = 86400" in open(p).read())
check("set-timeout: updated in place", ac.set_toml_timeout(p, ac.SERVER, 3600) == "updated")
text = open(p).read()
check("set-timeout: codex value replaced, single key", "tool_timeout_sec = 3600" in text and text.count("tool_timeout_sec") == 1, text)
tomllib.loads(text)

p = os.path.join(d, "timeout-vibe.toml")
write(p, '[[mcp_servers]]\nname = "%s"\ncommand = "%s"\n\n[[mcp_servers]]\nname = "other"\ncommand = "npx"\n' % (ac.SERVER, CMD))
check("set-timeout: added to vibe element", ac.set_toml_timeout(p, ac.SERVER, 86400) == "added")
text = open(p).read()
check("set-timeout: vibe element has it, other element untouched",
      "tool_timeout_sec = 86400" in text and text.count("tool_timeout_sec") == 1 and 'name = "other"' in text,
      text)
tomllib.loads(text)

p = os.path.join(d, "timeout-vibe-sub.toml")
write(p, '[[mcp_servers]]\nname = "%s"\ncommand = "%s"\n\n[mcp_servers.env]\nFOO = "bar"\n' % (ac.SERVER, CMD))
check("set-timeout: vibe element with subtable", ac.set_toml_timeout(p, ac.SERVER, 86400) == "added")
text = open(p).read()
check("set-timeout: key lands in the element, not the subtable",
      "tool_timeout_sec = 86400" in text and text.index("tool_timeout_sec") < text.index("[mcp_servers.env]"),
      text)
tomllib.loads(text)
check("set-timeout: absent for unknown server", ac.set_toml_timeout(p, "nope", 60) == "absent")
check("set-timeout: absent for missing file", ac.set_toml_timeout(os.path.join(d, "nope.toml"), ac.SERVER, 60) == "absent")

# ------------------------------------------------- set-timeout: goose YAML
# an old install wrote 'timeout: 120'; every run must raise it to 86400
p = os.path.join(d, "goose-timeout-old.yaml")
write(p, "GOOSE_TELEMETRY_ENABLED: false\n# my goose config\nextensions:\n  chatrecall:\n    enabled: false\n    type: platform\n  slack-approval-gateway:\n    enabled: true\n    type: stdio\n    cmd: \"%s\"\n    args: []\n    envs: {}\n    timeout: 120\n" % CMD)
check("set-timeout: goose old timeout updated", ac.set_timeout(p, ac.SERVER, 86400) == "updated")
text = open(p).read()
check("set-timeout: goose 120 becomes 86400", "    timeout: 86400\n" in text and "timeout: 120" not in text, text)
check("set-timeout: goose comments and other extensions kept",
      "# my goose config" in text and "chatrecall" in text and "GOOSE_TELEMETRY_ENABLED: false" in text,
      text)
check("set-timeout: goose single timeout line", text.count("timeout:") == 1, text)
check("set-timeout: goose backup made", os.path.exists(p + ac.BAK))

# a missing timeout line gets added after the block's last key
p = os.path.join(d, "goose-timeout-missing.yaml")
write(p, "extensions:\n  slack-approval-gateway:\n    enabled: true\n    type: stdio\n    cmd: \"%s\"\n    args: []\n    envs: {}\n  other:\n    command: npx\n" % CMD)
check("set-timeout: goose missing timeout added", ac.set_timeout(p, ac.SERVER, 86400) == "added")
text = open(p).read()
check("set-timeout: goose timeout added at the end of the block",
      "    envs: {}\n    timeout: 86400\n  other:" in text, text)
check("set-timeout: goose other extension untouched", "  other:\n    command: npx\n" in text, text)

# an absent entry is reported absent (and the file is not touched)
p = os.path.join(d, "goose-timeout-absent.yaml")
fixture = "extensions:\n  chatrecall:\n    enabled: false\n"
write(p, fixture)
check("set-timeout: goose absent entry", ac.set_timeout(p, ac.SERVER, 86400) == "absent")
check("set-timeout: goose absent leaves the file untouched", open(p).read() == fixture)

# a YAML block that is not under extensions: is not ours to touch
p = os.path.join(d, "goose-timeout-wrong-parent.yaml")
write(p, "mcp_servers:\n  slack-approval-gateway:\n    command: \"%s\"\n" % CMD)
check("set-timeout: yaml block not under extensions is absent", ac.set_timeout(p, ac.SERVER, 86400) == "absent")

# the dispatcher still handles TOML
p = os.path.join(d, "timeout-dispatch.toml")
write(p, '[mcp_servers.%s]\ncommand = "%s"\n' % (ac.SERVER, CMD))
check("set-timeout: dispatches to TOML", ac.set_timeout(p, ac.SERVER, 86400) == "added")
check("set-timeout: TOML entry has it", "tool_timeout_sec = 86400" in open(p).read())

# ------------------------------- audit locations + nested per-project removal
p = os.path.join(d, "claude-projects.json")
proj = "/Users/test/my.project"
write(p, json.dumps({
    "mcpServers": {"grafana": {"command": "docker"}},
    "projects": {
        proj: {"mcpServers": {
            "slack": {"type": "stdio", "command": "npx", "env": {"SLACK_MCP_XOXC_TOKEN": "xoxc-5330704053091-9437214043493-abcdef0123456789"}},
            ac.SERVER: {"type": "stdio", "command": CMD},
        }},
        "/Users/test/other": {"mcpServers": {"keep": {"command": "x"}}},
    },
}))
f = ac.audit_file(p)
check("audit: per-project server found", names(f, "direct_slack_servers") == ["slack"], json.dumps(f))
check("audit: per-project location reported",
      f["direct_slack_servers"][0]["key"] == ["projects", proj, "mcpServers"], json.dumps(f))
check("enforce: remove-entry removes the nested entry",
      ac.remove_entry("claude", p, "slack", ["projects", proj, "mcpServers"]) == "removed")
data = json.load(open(p))
check("enforce: nested entry gone", "slack" not in data["projects"][proj]["mcpServers"], json.dumps(data))
check("enforce: gateway and other entries kept",
      ac.SERVER in data["projects"][proj]["mcpServers"]
      and "grafana" in data["mcpServers"]
      and "keep" in data["projects"]["/Users/test/other"]["mcpServers"],
      json.dumps(data))
check("enforce: remove-entry absent for a wrong path",
      ac.remove_entry("claude", p, "slack", ["projects", "/nope", "mcpServers"]) == "absent")

# remove-json (uninstall) also sweeps nested per-project gateway entries
p = os.path.join(d, "claude-projects-uninstall.json")
write(p, json.dumps({
    "projects": {proj: {"mcpServers": {ac.SERVER: {"command": CMD}}}},
}))
check("uninstall: nested per-project gateway entry removed", ac.remove_json("claude", p) == "removed")
data = json.load(open(p))
check("uninstall: nested gateway gone", ac.SERVER not in data["projects"][proj].get("mcpServers", {}), json.dumps(data))

# ------------------------------------------------------------- enforce-remove
# enforce-remove is what install.sh --enforce runs per finding; it must
# report what actually happened and route by config file.
vd = os.path.join(d, ".vibe")
p = os.path.join(vd, "config.toml")
write(p, '[[mcp_servers]]\nname = "slack"\ncommand = "npx"\n\n[mcp_servers.env]\nSLACK_MCP_XOXC_TOKEN = "xoxc-5330704053091-9437214043493-abcdef0123456789"\n\n[[mcp_servers]]\nname = "other"\ncommand = "x"\n')
buf = io.StringIO()
with contextlib.redirect_stdout(buf):
    res = ac.enforce_remove(p, "slack", [])
check("enforce-remove: vibe server removed", res == "removed" and "[removed]" in buf.getvalue(), buf.getvalue())
check("enforce-remove: vibe element gone, other kept",
      'name = "slack"' not in open(p).read() and 'name = "other"' in open(p).read())
buf = io.StringIO()
with contextlib.redirect_stdout(buf):
    res = ac.enforce_remove(p, "slack", [])
check("enforce-remove: absent reported accurately", res == "absent" and "[absent]" in buf.getvalue(), buf.getvalue())

nd = os.path.join(d, "claude-nested")
p = os.path.join(nd, ".claude.json")
write(p, json.dumps({"projects": {proj: {"mcpServers": {"slack": {"command": "npx", "env": {"SLACK_MCP_XOXC_TOKEN": "xoxc-5330704053091-9437214043493-abcdef0123456789"}}}}}}))
buf = io.StringIO()
with contextlib.redirect_stdout(buf):
    res = ac.enforce_remove(p, "slack", ["projects", proj, "mcpServers"])
data = json.load(open(p))
check("enforce-remove: nested per-project entry removed",
      res == "removed" and "[removed]" in buf.getvalue() and "slack" not in data["projects"][proj]["mcpServers"],
      buf.getvalue() + json.dumps(data))

# a YAML config has no automatic removal: reported as edit by hand
yp = os.path.join(d, "goose-direct.yaml")
write(yp, "extensions:\n  slack:\n    enabled: true\n    type: stdio\n    cmd: npx\n")
buf = io.StringIO()
with contextlib.redirect_stdout(buf):
    res = ac.enforce_remove(yp, "slack", ["extensions"])
check("enforce-remove: yaml reported as edit by hand", res == "edit-by-hand" and "[edit by hand]" in buf.getvalue(), buf.getvalue())

# a failing removal is reported as failed, never as removed
bad = os.path.join(d, "claude-broken")
bp = os.path.join(bad, ".claude.json")
write(bp, "{not json")
buf = io.StringIO()
with contextlib.redirect_stdout(buf):
    res = ac.enforce_remove(bp, "slack", [])
check("enforce-remove: failure reported accurately", res == "failed" and "[failed]" in buf.getvalue() and "[removed]" not in buf.getvalue(), buf.getvalue())

# ------------------------------------------- uninstall semantics: zed/hermes
p = os.path.join(d, "zed-uninstall.json")
write(p, '{\n  "context_servers": {\n    "existing": {"command": {"path": "npx"}},\n    "%s": {"command": {"path": "%s", "args": []}}\n  },\n  "ui_font_size": 14\n}\n' % (ac.SERVER, CMD))
check("zed: remove drops only our entry", ac.remove_zed(p) == "removed")
data = json.load(open(p))
check("zed: other settings kept", "existing" in data["context_servers"] and data["ui_font_size"] == 14, json.dumps(data))
check("zed: remove again is absent", ac.remove_zed(p) == "absent")

# --------------------------- remove-zed is a text splice (JSONC preserved)
# a commented JSONC fixture: every comment and every other entry must survive
# byte-for-byte, with only the gateway entry spliced out
p = os.path.join(d, "zed-splice-comments.json")
fixture = (
    "// Zed settings\n"
    "{\n"
    "  // language models\n"
    '  "context_servers": {\n'
    "    // the official slack (read-only)\n"
    '    "slack": {"command": {"path": "npx", "args": ["slack-mcp"]}}, // official\n'
    "    /* gateway entry below */\n"
    '    "slack-approval-gateway": {"command": {"path": "%s", "args": []}},\n'
    '    "existing": {"command": {"path": "npx"}},\n'
    "  },\n"
    '  "ui_font_size": 14, // favourite\n'
    "}\n" % CMD
)
write(p, fixture)
check("zed-splice: removed", ac.remove_zed(p) == "removed")
text = open(p).read()
expected = (
    "// Zed settings\n"
    "{\n"
    "  // language models\n"
    '  "context_servers": {\n'
    "    // the official slack (read-only)\n"
    '    "slack": {"command": {"path": "npx", "args": ["slack-mcp"]}}, // official\n'
    "    /* gateway entry below */\n"
    '    "existing": {"command": {"path": "npx"}},\n'
    "  },\n"
    '  "ui_font_size": 14, // favourite\n'
    "}\n"
)
check("zed-splice: comments and other entries byte-identical", text == expected, text)
data = json.loads(ac.strip_jsonc(text))
check("zed-splice: result parses as JSONC",
      ac.SERVER not in data["context_servers"] and data["context_servers"]["existing"]["command"]["path"] == "npx",
      json.dumps(data))

# first in context_servers
p = os.path.join(d, "zed-splice-first.json")
write(p, '{\n  "context_servers": {\n    "slack-approval-gateway": {"command": {"path": "%s", "args": []}},\n    "existing": {"command": {"path": "npx"}}\n  }\n}\n' % CMD)
check("zed-splice: first entry removed", ac.remove_zed(p) == "removed")
check("zed-splice: first entry spliced out cleanly",
      open(p).read() == '{\n  "context_servers": {\n    "existing": {"command": {"path": "npx"}}\n  }\n}\n',
      open(p).read())

# last in context_servers (the comma in front of it goes too)
p = os.path.join(d, "zed-splice-last.json")
write(p, '{\n  "context_servers": {\n    "existing": {"command": {"path": "npx"}},\n    "slack-approval-gateway": {"command": {"path": "%s", "args": []}}\n  }\n}\n' % CMD)
check("zed-splice: last entry removed", ac.remove_zed(p) == "removed")
check("zed-splice: last entry spliced out, no dangling comma",
      open(p).read() == '{\n  "context_servers": {\n    "existing": {"command": {"path": "npx"}}\n  }\n}\n',
      open(p).read())

# the only entry: add_zed's `{ "entry" }` splice is undone exactly
p = os.path.join(d, "zed-splice-only.json")
original = '{\n  "context_servers": {},\n  "ui_font_size": 14\n}\n'
write(p, original)
ac.add_zed(p, CMD)
check("zed-splice: added into the empty object", '"slack-approval-gateway"' in open(p).read())
check("zed-splice: only entry removed", ac.remove_zed(p) == "removed")
check("zed-splice: empty object restored byte-for-byte", open(p).read() == original, open(p).read())

# a server name that is not an entry (in a comment, or no context_servers)
p = os.path.join(d, "zed-splice-absent.json")
fixture = '{\n  // the slack-approval-gateway server\n  "context_servers": {"existing": {"command": {"path": "npx"}}}\n}\n'
write(p, fixture)
check("zed-splice: name in a comment is not an entry", ac.remove_zed(p) == "absent")
check("zed-splice: absent leaves the file untouched", open(p).read() == fixture)
p = os.path.join(d, "zed-splice-nocs.json")
write(p, '{\n  "ui_font_size": 14\n}\n')
check("zed-splice: no context_servers is absent", ac.remove_zed(p) == "absent")

p = os.path.join(d, "hermes-uninstall.yaml")
write(p, "telemetry:\n  enabled: false\nmcp_servers:\n  %s:\n    command: \"%s\"\n    args: []\n    env: {}\n  other:\n    command: npx\n" % (ac.SERVER, CMD))
check("hermes: remove drops only our block", ac.remove_hermes(p) == "removed")
text = open(p).read()
check("hermes: other entries kept", "other:" in text and "telemetry:" in text and ac.SERVER not in text, text)

# ------------------------------------------------- install.sh wiring (grep)
# install.sh manages the real login service, so it cannot be run end to end
# in tests; pin the wiring it must have instead.
sh = open(os.path.join(ROOT, "scripts", "install.sh")).read()
check("install.sh: zed instruction block goes to ~/.config/zed/AGENTS.md",
      'add_instruction "$HOME_DIR/.config/zed/AGENTS.md"' in sh)
check("install.sh: zed legacy rules file deleted on install",
      'run rm -f "$HOME_DIR/.config/zed/rules/slack-gateway.md"' in sh)
check("install.sh: hermes instruction block goes to ~/.hermes/SOUL.md",
      'add_instruction "$HOME_DIR/.hermes/SOUL.md"' in sh)
check("install.sh: hermes legacy AGENTS.md block migrated away",
      'ac instruction-remove "$HOME_DIR/.hermes/AGENTS.md"' in sh)
check("install.sh: uninstall removes the zed AGENTS.md block",
      '"$HOME_DIR/.config/zed/AGENTS.md"' in sh)
check("install.sh: uninstall removes the claude entry",
      'ac remove-json claude "$HOME_DIR/.claude.json"' in sh)
check("install.sh: uninstall removes the vibe entry",
      'ac remove-vibe "$HOME_DIR/.vibe/config.toml"' in sh)
check("install.sh: uninstall keeps .bak-sag backups for manual restore",
      ".bak-sag backups remain" in sh and "restored from their .bak-sag backups" not in sh)
check("install.sh: vibe registration sets a long tool timeout",
      "--tool-timeout-sec 86400" in sh and 'set-timeout "$CFG" "$SERVER_NAME" 86400' in sh)
check("install.sh: codex registration sets a long tool timeout",
      'set-timeout "$CFG" "$SERVER_NAME" 86400' in sh)
check("install.sh: goose registration sets a long tool timeout too",
      sh.count('set-timeout "$CFG" "$SERVER_NAME" 86400') == 3)
check("install.sh: enforce reports removed/absent/failed via enforce-remove",
      'ac enforce-remove "$line"' in sh and "[removed] {path}" not in sh)

# ------------------------------------------------------------ instruction text
check("instruction: read via slack mcp/connector, send via the gateway",
      "read Slack context" in ac.INSTRUCTION
      and "request_slack_message" in ac.INSTRUCTION
      and "slack_send_message" in ac.INSTRUCTION
      and "Mistral Slack connector" in ac.INSTRUCTION,
      ac.INSTRUCTION)

# ------------------------------------------------------------ strip_jsonc
check("strip_jsonc: comments and trailing commas",
      json.loads(ac.strip_jsonc('{\n // c\n "a": [1, 2,], /* b */ "b": "x,y",\n}')) == {"a": [1, 2], "b": "x,y"})
check("strip_jsonc: comment-like strings kept",
      json.loads(ac.strip_jsonc('{"a": "http://x // not a comment"}')) == {"a": "http://x // not a comment"})

shutil.rmtree(d, ignore_errors=True)

# ------------------------------------------------------- take over a Slack MCP
d = tmpdir()
XOXC, XOXD = "xoxc-1111111111-2222222222-session", "xoxd-cookie%2Bvalue="
XOXP = "xoxp-1111111111-2222222222-usertoken"
XOXB = "xoxb-1111111111-2222222222-bottoken"
MKEY = "mstrl_abcdefghijklmnopqrs"
cursor = os.path.join(d, ".cursor", "mcp.json")
write(cursor, json.dumps({"mcpServers": {
    "slack": {"command": "npx -y slack-mcp-server@latest --transport stdio",
              "env": {"SLACK_MCP_XOXC_TOKEN": XOXC, "SLACK_MCP_XOXD_TOKEN": XOXD,
                      "SLACK_MCP_ENABLED_TOOLS": "channels_list",
                      "SLACK_MCP_USERS_CACHE": "/elsewhere/users.json",
                      "SLACK_MCP_ADD_MESSAGE_TOOL": "C0ALLOWED"}},
    "official": {"url": "https://mcp.slack.com/mcp",
                 "headers": {"Authorization": "Bearer " + XOXP}},
    "official-oauth": {"type": "http", "url": "https://mcp.slack.com/mcp",
                       "oauth": {"clientId": "1.2", "callbackPort": 3118}},
    "mistral-slack": {"url": "https://api.mistral.ai/v1/connectors-gateway/my_slack/mcp",
                      "headers": {"Authorization": "Bearer " + MKEY}},
    "mistral-notion": {"url": "https://api.mistral.ai/v1/connectors-gateway/notion/mcp",
                       "headers": {"Authorization": "Bearer " + MKEY}},
    "env-ref": {"command": "slack-mcp-server", "args": ["--transport", "stdio"],
                "env": {"SLACK_MCP_XOXP_TOKEN": "${SLACK_TOKEN}"}},
    "half-session": {"command": "slack-mcp-server", "env": {"SLACK_MCP_XOXC_TOKEN": XOXC}},
    "spaced": {"command": "/Applications/My Tools/slack-mcp-server",
               "env": {"SLACK_MCP_XOXP_TOKEN": XOXP}},
    "remote": {"url": "http://localhost:13080/sse", "env": {"SLACK_MCP_XOXP_TOKEN": XOXP}},
    "grafana": {"command": "mcp-grafana"},
}}))
codex = os.path.join(d, ".codex", "config.toml")
write(codex, '[mcp_servers.kslack]\ncommand = "slack-mcp-server"\nargs = ["--transport", "stdio"]\n'
             '[mcp_servers.kslack.env]\nSLACK_MCP_XOXB_TOKEN = "%s"\n' % XOXB)
vibe = os.path.join(d, ".vibe", "config.toml")
write(vibe, '[[mcp_servers]]\nname = "vslack"\ntransport = "stdio"\ncommand = "slack-mcp-server"\n'
            'args = ["--transport", "stdio"]\n[mcp_servers.env]\nSLACK_MCP_XOXP_TOKEN = "%s"\n' % XOXP)
goose = os.path.join(d, ".config", "goose", "config.yaml")
write(goose, "extensions:\n  gslack:\n    enabled: true\n    type: stdio\n    cmd: slack-mcp-server\n"
             "    args:\n    - --transport\n    - stdio\n    envs:\n      SLACK_MCP_XOXC_TOKEN: \"%s\"\n"
             "      SLACK_MCP_XOXD_TOKEN: %s\n    timeout: 300\n" % (XOXC, XOXD))
plugins = os.path.join(d, ".claude", "plugins", "installed_plugins.json")
write(plugins, json.dumps({"version": 2, "plugins": {"slack@claude-plugins-official": [{}], "other@x": [{}]}}))

out = io.StringIO()
with contextlib.redirect_stdout(out):
    ac.main(["takeover-candidates", cursor, codex, vibe, goose, plugins])
raw = out.getvalue()
cands = {(os.path.basename(os.path.dirname(c["file"])) + ":" + c["name"]): c for c in json.loads(raw)}
def cand(key):
    return cands.get(key) or {}
check("takeover: candidate output never contains a raw secret",
      not any(x in raw for x in (XOXC, XOXD, XOXP, XOXB, MKEY)), raw)
check("takeover: korotovsky browser session (Cursor one-string command) is importable",
      cand(".cursor:slack").get("importable") and cand(".cursor:slack").get("transport") == "korotovsky"
      and "xoxc+xoxd" in cand(".cursor:slack").get("summary", ""), json.dumps(cand(".cursor:slack")))
check("takeover: official server with a bearer user token is importable",
      cand(".cursor:official").get("importable") and cand(".cursor:official").get("transport") == "slack-mcp")
check("takeover: official server signed in with OAuth is not",
      cand(".cursor:official-oauth").get("importable") is False and "OAuth" in cand(".cursor:official-oauth").get("reason", ""))
check("takeover: Mistral Slack connector with a key is importable",
      cand(".cursor:mistral-slack").get("importable") and cand(".cursor:mistral-slack").get("transport") == "mistral-connector")
check("takeover: a non-Slack Mistral connector and other servers are ignored",
      ".cursor:mistral-notion" not in cands and ".cursor:grafana" not in cands, sorted(cands))
check("takeover: a token from an env reference is not importable",
      cand(".cursor:env-ref").get("importable") is False and "literal" in cand(".cursor:env-ref").get("reason", ""))
check("takeover: xoxc without xoxd is not importable", cand(".cursor:half-session").get("importable") is False)
check("takeover: a command with a space is not importable",
      cand(".cursor:spaced").get("importable") is False and "space" in cand(".cursor:spaced").get("reason", ""))
check("takeover: a remote (HTTP/SSE) korotovsky server is not importable",
      cand(".cursor:remote").get("importable") is False, json.dumps(cand(".cursor:remote")))
check("takeover: codex TOML [mcp_servers.NAME] with env subtable",
      cand(".codex:kslack").get("importable") and "xoxb" in cand(".codex:kslack").get("summary", ""))
check("takeover: vibe TOML [[mcp_servers]] element with env subtable",
      cand(".vibe:vslack").get("importable") and "xoxp" in cand(".vibe:vslack").get("summary", ""))
check("takeover: goose YAML block list args and envs map",
      cand("goose:gslack").get("importable") and "xoxc+xoxd" in cand("goose:gslack").get("summary", ""),
      json.dumps(cand("goose:gslack")))
check("takeover: Claude Code's Slack plugin is reported, not importable",
      cand("plugins:slack@claude-plugins-official").get("importable") is False
      and "plugins:other@x" not in cands)

state = os.path.join(d, "state")
os.makedirs(state)
write(os.path.join(state, "slack_bot_token"), "xoxb-placeholder-get-a-real-token\n")
write(os.path.join(state, "slack_user_token"), "xoxp-old-real-token-123\n")
bindir = os.path.join(d, "bin")
write(os.path.join(bindir, "slack-mcp-server"), "#!/bin/sh\nexit 0\n")
os.chmod(os.path.join(bindir, "slack-mcp-server"), 0o755)
old_path = os.environ.get("PATH", "")
os.environ["PATH"] = bindir + os.pathsep + old_path
try:
    lines = ac.take_over(state, cursor, "slack")
finally:
    os.environ["PATH"] = old_path
printed = "\n".join(lines)
check("take-over: printed lines never contain a raw secret",
      not any(x in printed for x in (XOXC, XOXD)), printed)
check("take-over: korotovsky selects its transport",
      open(os.path.join(state, "slack_transport")).read().strip() == "korotovsky")
cmdline = open(os.path.join(state, "slack_stdio_command")).read().strip()
check("take-over: npx line becomes the installed server (no download)",
      cmdline == os.path.join(bindir, "slack-mcp-server") + " --transport stdio", cmdline)
envtext = open(os.path.join(state, "slack_stdio_env")).read()
check("take-over: session tokens copied verbatim",
      ("SLACK_MCP_XOXC_TOKEN=" + XOXC) in envtext and ("SLACK_MCP_XOXD_TOKEN=" + XOXD) in envtext, envtext)
check("take-over: the agent's own settings are kept (send allowlist)",
      "SLACK_MCP_ADD_MESSAGE_TOOL=C0ALLOWED" in envtext)
check("take-over: SpeakEasy-controlled settings are dropped (tool list, caches)",
      "SLACK_MCP_ENABLED_TOOLS" not in envtext and "SLACK_MCP_USERS_CACHE" not in envtext, envtext)
check("take-over: credential files are private (0600)",
      all(os.stat(os.path.join(state, f)).st_mode & 0o077 == 0 for f in ("slack_stdio_env", "slack_stdio_command")))
check("take-over: a placeholder token is deleted",
      not os.path.exists(os.path.join(state, "slack_bot_token")))
check("take-over: a real token it replaces is moved aside, not lost",
      not os.path.exists(os.path.join(state, "slack_user_token"))
      and open(os.path.join(state, "slack_user_token.pre-takeover")).read().strip() == "xoxp-old-real-token-123")

os.environ["PATH"] = "/usr/bin:/bin"
try:
    ac.take_over(state, cursor, "slack")
finally:
    os.environ["PATH"] = old_path
check("take-over: without an installed server the agent's own command is kept",
      open(os.path.join(state, "slack_stdio_command")).read().split()[1:3] == ["-y", "slack-mcp-server@latest"],
      open(os.path.join(state, "slack_stdio_command")).read())

ac.take_over(state, cursor, "official")
check("take-over: official token -> slack_mcp_token, transport slack-mcp",
      open(os.path.join(state, "slack_mcp_token")).read().strip() == XOXP
      and open(os.path.join(state, "slack_transport")).read().strip() == "slack-mcp"
      and os.stat(os.path.join(state, "slack_mcp_token")).st_mode & 0o077 == 0)
ac.take_over(state, cursor, "mistral-slack")
check("take-over: Mistral key and a non-default connector name",
      open(os.path.join(state, "mistral_api_key")).read().strip() == MKEY
      and open(os.path.join(state, "mistral_slack_connector")).read().strip() == "my_slack"
      and open(os.path.join(state, "slack_transport")).read().strip() == "mistral-connector")
for name, needle in (("official-oauth", "OAuth"), ("env-ref", "literal"), ("nope", "no Slack MCP server")):
    try:
        ac.take_over(state, cursor, name)
        refused = ""
    except SystemExit as e:
        refused = str(e)
    check("take-over: refuses %s" % name, needle in refused, refused)
ac.take_over(state, goose, "gslack")
check("take-over: goose YAML session copied",
      ("SLACK_MCP_XOXC_TOKEN=" + XOXC) in open(os.path.join(state, "slack_stdio_env")).read())
# goose/hermes write block-list items at the key's own indent
goose2 = os.path.join(d, "g2", ".config", "goose", "config.yaml")
write(goose2, "extensions:\n  gslack:\n    cmd: npx\n    args:\n    - -y\n    - slack-mcp-server@latest\n"
              "    - --transport\n    - stdio\n    envs:\n      SLACK_MCP_XOXP_TOKEN: %s\n" % XOXP)
st2 = os.path.join(d, "state-g2")
os.environ["PATH"] = "/usr/bin:/bin"
try:
    ac.take_over(st2, goose2, "gslack")
finally:
    os.environ["PATH"] = old_path
check("take-over: goose same-indent list args are kept",
      open(os.path.join(st2, "slack_stdio_command")).read().split()[1:] ==
      ["-y", "slack-mcp-server@latest", "--transport", "stdio"],
      open(os.path.join(st2, "slack_stdio_command")).read())

# the same server in several Claude Code project scopes
claude = os.path.join(d, "cc", ".claude.json")
same = {"command": "slack-mcp-server", "args": ["--transport", "stdio"], "env": {"SLACK_MCP_XOXP_TOKEN": XOXP}}
other = {"command": "slack-mcp-server", "args": ["--transport", "stdio"], "env": {"SLACK_MCP_XOXB_TOKEN": XOXB}}
write(claude, json.dumps({"mcpServers": {"slack": same},
                          "projects": {"/a": {"mcpServers": {"slack": same}},
                                       "/b": {"mcpServers": {"slack": other}}}}))
cc = [c for c in ac.takeover_candidates([claude])]
check("takeover: identical same-name copies collapse; a different one is NAME#2",
      [c["name"] for c in cc] == ["slack", "slack#2"], [c["name"] for c in cc])
st3 = os.path.join(d, "state-cc")
ac.take_over(st3, claude, "slack#2")
check("take-over: NAME#2 picks the second distinct entry",
      "SLACK_MCP_XOXB_TOKEN=" + XOXB in open(os.path.join(st3, "slack_stdio_env")).read())
desktop = os.path.join(d, "cd", "Library", "Application Support", "Claude", "claude_desktop_config.json")
write(desktop, json.dumps({"mcpServers": {"slack": same}}))
fps = {c["fingerprint"] for c in ac.takeover_candidates([claude, desktop]) if c["name"] == "slack"}
check("takeover: the same server in two configs shares a fingerprint", len(fps) == 1, fps)

# send policy from the agent's config
pol = os.path.join(d, "pol", ".cursor", "mcp.json")
write(pol, json.dumps({"mcpServers": {
    "deny": {"command": "slack-mcp-server", "env": {"SLACK_MCP_XOXP_TOKEN": XOXP, "SLACK_MCP_ADD_MESSAGE_TOOL": "false"}},
    "allow": {"command": "slack-mcp-server", "env": {"SLACK_MCP_XOXP_TOKEN": XOXP, "SLACK_MCP_ADD_MESSAGE_TOOL": "C0123ABCD,!C0999XYZW"}},
    "bad env": {"command": "slack-mcp-server", "env": {"SLACK_MCP_XOXP_TOKEN": XOXP, "NOT-A-NAME": "x"}},
    "docker": {"command": "docker", "args": ["run", "-i", "--rm", "-e", "SLACK_MCP_XOXC_TOKEN", "-e", "SLACK_MCP_XOXD_TOKEN",
                                              "ghcr.io/korotovsky/slack-mcp-server", "--transport", "stdio"],
               "env": {"SLACK_MCP_XOXC_TOKEN": XOXC, "SLACK_MCP_XOXD_TOKEN": XOXD}},
    "runner-only": {"command": "npx", "env": {"SLACK_MCP_XOXP_TOKEN": XOXP}},
    "official-envref": {"url": "https://mcp.slack.com/mcp", "headers": {"Authorization": "Bearer ${SLACK_TOKEN}"}},
}}))
stp = os.path.join(d, "state-pol")
lines = ac.take_over(stp, pol, "deny")
check("take-over: a deny-all SLACK_MCP_ADD_MESSAGE_TOOL is dropped (and said so)",
      "SLACK_MCP_ADD_MESSAGE_TOOL" not in open(os.path.join(stp, "slack_stdio_env")).read()
      and any("ignored SLACK_MCP_ADD_MESSAGE_TOOL" in l for l in lines), lines)
lines = ac.take_over(stp, pol, "allow")
check("take-over: a channel allowlist is kept (and said so)",
      "SLACK_MCP_ADD_MESSAGE_TOOL=C0123ABCD,!C0999XYZW" in open(os.path.join(stp, "slack_stdio_env")).read()
      and any("kept SLACK_MCP_ADD_MESSAGE_TOOL" in l for l in lines))
ac.take_over(stp, pol, "bad env")
check("take-over: env names that are not variable names are skipped",
      "NOT-A-NAME" not in open(os.path.join(stp, "slack_stdio_env")).read())
ac.take_over(stp, pol, "docker")
dcmd = open(os.path.join(stp, "slack_stdio_command")).read().split()
check("take-over: a docker run line forwards SpeakEasy's settings into the container",
      dcmd[2:6] == ["-e", "SLACK_MCP_ADD_MESSAGE_TOOL", "-e", "SLACK_MCP_LOG_LEVEL"]
      and dcmd.count("SLACK_MCP_XOXC_TOKEN") == 1, dcmd)
pc = {c["name"]: c for c in ac.takeover_candidates([pol])}
check("takeover: a bare package runner is not importable",
      pc["runner-only"]["importable"] is False and "no package" in pc["runner-only"]["reason"])
check("takeover: an official token from an env reference says so",
      pc["official-envref"]["importable"] is False and "environment variable" in pc["official-envref"]["reason"])

# TOML without any TOML parser: reported, not silently skipped
real_loader = ac._toml_loader
ac._toml_loader = lambda: None
try:
    tc = ac.takeover_candidates([codex])
finally:
    ac._toml_loader = real_loader
check("takeover: TOML without a parser is reported with the reason",
      len(tc) == 1 and tc[0]["importable"] is False and "Python 3.11" in tc[0]["reason"], tc)

check("can-enforce: goose, Zed and Hermes configs are edited by hand",
      ac.enforce_style(goose) is None and ac.enforce_style(os.path.join(d, ".config", "zed", "settings.json")) is None
      and ac.enforce_style(os.path.join(d, ".hermes", "config.yaml")) is None)
check("can-enforce: Cursor, Claude Desktop, Codex, Vibe",
      ac.enforce_style(cursor) == "gemini" and ac.enforce_style(desktop) == "gemini"
      and ac.enforce_style(codex) == "codex" and ac.enforce_style(vibe) == "vibe")
check("enforce style: a home dir containing 'amp' is not mistaken for Amp",
      ac.enforce_style("/Users/sampson/.config/zed/settings.json") is None
      and ac.enforce_style("/Users/x/.config/amp/settings.json") == "amp")
# references an agent resolves at runtime are not tokens
oc = os.path.join(d, "oc", ".config", "opencode", "opencode.json")
write(oc, json.dumps({"mcp": {
    "kslack": {"command": ["slack-mcp-server", "--transport", "stdio"],
               "environment": {"SLACK_MCP_XOXP_TOKEN": "{env:SLACK_USER_TOKEN}"}},
    "mslack": {"url": "https://api.mistral.ai/v1/connectors-gateway/slack/mcp",
               "headers": {"Authorization": "Bearer {env:MISTRAL_API_KEY}"}},
    "badshape": {"command": ["slack-mcp-server"], "environment": {"SLACK_MCP_XOXP_TOKEN": "not-a-token"}}}}))
occ = {c["name"]: c for c in ac.takeover_candidates([oc])}
check("takeover: OpenCode {env:VAR} references are not taken as tokens",
      occ["kslack"]["importable"] is False and occ["mslack"]["importable"] is False, occ)
check("takeover: a value without a token prefix is not a token", occ["badshape"]["importable"] is False)

# VS Code keeps servers under a top-level "servers" key
vsc = os.path.join(d, "proj", ".vscode", "mcp.json")
write(vsc, json.dumps({"servers": {"slack": {"type": "stdio", "command": "slack-mcp-server",
                                             "args": ["--transport", "stdio"],
                                             "env": {"SLACK_MCP_XOXP_TOKEN": XOXP}}}, "inputs": []}))
check("takeover: VS Code mcp.json (servers key) is read",
      [c["name"] for c in ac.takeover_candidates([vsc]) if c["importable"]] == ["slack"])

# a non-Slack server with the same name does not shift NAME#N
cc2 = os.path.join(d, "cc2", ".claude.json")
write(cc2, json.dumps({"projects": {
    "/a": {"mcpServers": {"slack": {"command": "npx", "args": ["-y", "@modelcontextprotocol/server-slack"]}}},
    "/b": {"mcpServers": {"slack": {"command": "slack-mcp-server", "env": {"SLACK_MCP_XOXP_TOKEN": XOXP}}}},
    "/c": {"mcpServers": {"slack": {"command": "slack-mcp-server", "env": {"SLACK_MCP_XOXB_TOKEN": XOXB}}}}}}))
names2 = [c["name"] for c in ac.takeover_candidates([cc2]) if c["importable"]]
st4 = os.path.join(d, "state-cc2")
ac.take_over(st4, cc2, "slack#2")
env4 = open(os.path.join(st4, "slack_stdio_env")).read()
ac.take_over(st4, cc2, names2[0])
env5 = open(os.path.join(st4, "slack_stdio_env")).read()
check("take-over: NAME#N picks the entry the candidate list named",
      names2 == ["slack", "slack#2"] and ("SLACK_MCP_XOXB_TOKEN=" + XOXB) in env4
      and ("SLACK_MCP_XOXP_TOKEN=" + XOXP) in env5, (names2, env4, env5))

# YAML inline comments are not part of a value
hermes = os.path.join(d, "hm", ".hermes", "config.yaml")
write(hermes, "mcp_servers:\n  hslack:\n    command: slack-mcp-server  # installed with npm\n"
              "    args: [\"--transport\", \"stdio\"]  # stdio mode\n    env:\n"
              "      SLACK_MCP_XOXP_TOKEN: \"%s\"  # my user token\n" % XOXP)
st6 = os.path.join(d, "state-hm")
ac.take_over(st6, hermes, "hslack")
check("take-over: YAML inline comments are dropped from values",
      ("SLACK_MCP_XOXP_TOKEN=" + XOXP + "\n") in open(os.path.join(st6, "slack_stdio_env")).read()
      and open(os.path.join(st6, "slack_stdio_command")).read().strip().endswith("slack-mcp-server --transport stdio"),
      open(os.path.join(st6, "slack_stdio_env")).read() + open(os.path.join(st6, "slack_stdio_command")).read())

# codex reads an official server's bearer token from the environment
cx2 = os.path.join(d, "cx2", ".codex", "config.toml")
write(cx2, '[mcp_servers.slack]\nurl = "https://mcp.slack.com/mcp"\nbearer_token_env_var = "SLACK_TOKEN"\n')
cxc = ac.takeover_candidates([cx2])
check("takeover: codex bearer_token_env_var is reported as an environment variable",
      cxc and cxc[0]["importable"] is False and "environment variable" in cxc[0]["reason"], cxc)
check("enforce: Cursor and Claude Desktop configs use the mcpServers style",
      ac.enforce_remove(cursor, "official", None) == "removed"
      and "official" not in json.load(open(cursor))["mcpServers"])
shutil.rmtree(d, ignore_errors=True)

# --------------------------------------------------------------- approval hooks
d = tmpdir()
HB = "/Users/test/.local/bin/slack-approval-gateway"
SD = "/Users/test/.slack-approval-gateway"
q = contextlib.redirect_stderr(io.StringIO())

# claude: other hooks survive; ours is replaced in place, then removed
cl = os.path.join(d, ".claude", "settings.json")
write(cl, json.dumps({"model": "opus", "hooks": {"PreToolUse": [
    {"matcher": "Artifact", "hooks": [{"type": "command", "command": "/x/other.sh"}]}]}}))
ac.hook_add("claude", cl, HB, "600", SD)
ac.hook_add("claude", cl, HB, "300", SD)
data = json.load(open(cl))
pre = data["hooks"]["PreToolUse"]
ours = [g for g in pre if any(ac.is_hook_command(h.get("command")) for h in g["hooks"])]
check("hooks/claude: other PreToolUse hooks kept", pre[0]["matcher"] == "Artifact", pre)
check("hooks/claude: re-adding replaces ours (one entry)", len(ours) == 1, pre)
check("hooks/claude: timeout = wait + margin", ours[0]["hooks"][0]["timeout"] == 360, ours)
check("hooks/claude: --wait, --state-dir and --ok-code in the command",
      "--hook claude --wait 300 --state-dir %s --ok-code 10" % SD in ours[0]["hooks"][0]["command"], ours)
check("hooks/claude: the matcher equals src/hook.rs's copy (ConfigChange guard)",
      ('"%s";' % ac.CLAUDE_MATCHER) in open(os.path.join(ROOT, "src", "hook.rs")).read())
check("hooks/claude: ConfigChange guard registered",
      any("--event config-change" in h["command"] for g in data["hooks"]["ConfigChange"] for h in g["hooks"]))
check("hooks/claude: has", ac.hook_has("claude", cl))
check("hooks/claude: remove", ac.hook_remove("claude", cl) == "removed")
data = json.load(open(cl))
check("hooks/claude: only ours removed", data == {"model": "opus", "hooks": {"PreToolUse": [
    {"matcher": "Artifact", "hooks": [{"type": "command", "command": "/x/other.sh"}]}]}}, data)
check("hooks/claude: remove again is absent", ac.hook_remove("claude", cl) == "absent")
check("hooks/claude: missing file is absent", ac.hook_remove("claude", os.path.join(d, "nope.json")) == "absent")

# the matchers select Slack tools in every agent's naming scheme
import re as _re
for matcher, anchored, yes, no in (
    (ac.CLAUDE_MATCHER, True, ["mcp__plugin_slack_slack__slack_send_message", "mcp__claude_ai_Slack__x",
                               "mcp__ws__conversations_add_message", "mcp__executor__call"],
     ["mcp__github__create_issue", "Bash"]),
    (ac.CODEX_MATCHER, False, ["mcp__slack__slack_send_message"], ["mcp__github__x"]),
    (ac.GEMINI_MATCHER, False, ["mcp_slack_slack_send_message", "mcp_ws_conversations_add_message"], ["mcp_github_x", "run_shell_command"]),
    (ac.COPILOT_MATCHER, True, ["slack-slack_send_message", "ws-reactions_add"], ["github-create_issue", "bash"]),
):
    test = (lambda m, n: _re.fullmatch(m, n)) if anchored else (lambda m, n: _re.search(m, n))
    check("hooks: matcher %s selects Slack tools" % matcher, all(test(matcher, n) for n in yes) and not any(test(matcher, n) for n in no))

# codex: trust entry with the verified hash formula, block replaced in place
cx = os.path.join(d, ".codex", "config.toml")
write(cx, 'model = "gpt"\n\n[[hooks.PreToolUse]]\nmatcher = "Bash"\n\n[[hooks.PreToolUse.hooks]]\ntype = "command"\ncommand = "/x/b.sh"\n\n[hooks.state]\n')
ac.hook_add("codex", cx, HB, "600", SD)
ac.hook_add("codex", cx, HB, "600", SD)
parsed = tomllib.loads(open(cx).read())
groups = parsed["hooks"]["PreToolUse"]
check("hooks/codex: other group kept, ours appended once", len(groups) == 2 and groups[0]["matcher"] == "Bash", groups)
key = "%s:pre_tool_use:1:0" % os.path.realpath(cx)
cmd = groups[1]["hooks"][0]["command"]
check("hooks/codex: trust entry for our group (index 1)",
      parsed["hooks"]["state"].get(key, {}).get("trusted_hash") == ac._codex_hash(cmd, ac.CODEX_MATCHER, 660),
      list(parsed["hooks"]["state"]))
check("hooks/codex: remove restores the file", ac.hook_remove("codex", cx) == "removed"
      and "slack-approval-gateway" not in open(cx).read() and 'matcher = "Bash"' in open(cx).read())

# codex: tables Codex itself writes after SpeakEasy's survive re-adding and
# removing (they used to land inside a marked span and get deleted)
cx3 = os.path.join(d, "cx3", ".codex", "config.toml")
write(cx3, 'model = "gpt"\n')
ac.hook_add("codex", cx3, HB, "600", SD)
with open(cx3, "a") as f:
    f.write('\n[mcp_servers.other]\ncommand = "x"\n\n[features]\nfoo = true\n')
ac.hook_add("codex", cx3, HB, "300", SD)
ac.hook_remove("codex", cx3)
parsed = tomllib.loads(open(cx3).read())
check("hooks/codex: the user's later tables survive re-add and removal",
      parsed.get("mcp_servers", {}).get("other") == {"command": "x"} and parsed.get("features") == {"foo": True}
      and "hooks" not in parsed, open(cx3).read())
# old marker-wrapped installs: the markers go, the user's tables inside stay
cx4 = os.path.join(d, "cx4", ".codex", "config.toml")
write(cx4, ac.CODEX_HOOK_BEGIN + '\n[mcp_servers.inside]\ncommand = "y"\n' + ac.CODEX_HOOK_END + "\n")
ac.hook_remove("codex", cx4)
check("hooks/codex: legacy markers are dropped, enclosed user tables kept",
      tomllib.loads(open(cx4).read()) == {"mcp_servers": {"inside": {"command": "y"}}}, open(cx4).read())
# a commented user group header still counts for the trust index
cx5 = os.path.join(d, "cx5", ".codex", "config.toml")
write(cx5, '[[hooks.PreToolUse]]  # mine\nmatcher = "Bash"\n[[hooks.PreToolUse.hooks]]\ntype = "command"\ncommand = "/x"\n')
ac.hook_add("codex", cx5, HB, "600", SD)
keys = list(tomllib.loads(open(cx5).read())["hooks"]["state"])
check("hooks/codex: a commented group header counts (our group is index 1)",
      all(k.endswith(":pre_tool_use:1:0") for k in keys) and keys, keys)
# removing SpeakEasy's group renumbers the trust of groups after it
cx6 = os.path.join(d, "cx6", ".codex", "config.toml")
write(cx6, "")
ac.hook_add("codex", cx6, HB, "600", SD)
real6 = os.path.realpath(cx6)
with open(cx6, "a") as f:
    f.write('\n[[hooks.PreToolUse]]\nmatcher = "Edit"\n[[hooks.PreToolUse.hooks]]\ntype = "command"\ncommand = "/y"\n'
            '\n[hooks.state."%s:pre_tool_use:1:0"]\ntrusted_hash = "sha256:user"\n' % real6)
ac.hook_add("codex", cx6, HB, "300", SD)
st = tomllib.loads(open(cx6).read())["hooks"]["state"]
check("hooks/codex: re-adding keeps our group in place (the user's trust stays at 1)",
      st.get("%s:pre_tool_use:1:0" % real6) == {"trusted_hash": "sha256:user"}, st)
ac.hook_remove("codex", cx6)
st = tomllib.loads(open(cx6).read())["hooks"]["state"]
check("hooks/codex: after removal the user's group trust moves to index 0",
      st == {"%s:pre_tool_use:0:0" % real6: {"trusted_hash": "sha256:user"}}, st)

# vibe: updated in place by name; a table Vibe/Superset added after it stays
vb2 = os.path.join(d, "vb2", ".vibe", "hooks.toml")
write(vb2, "")
ac.hook_add("vibe", vb2, HB, "600", SD)
with open(vb2, "a") as f:
    f.write('\n[[hooks]]\nname = "later"\ntype = "post_agent"\ncommand = "/z"\n')
ac.hook_add("vibe", vb2, HB, "300", SD)
names = [h["name"] for h in tomllib.loads(open(vb2).read())["hooks"]]
check("hooks/vibe: in place, the later entry kept", names == ["speakeasy-slack-approval", "later"], names)
ac.hook_remove("vibe", vb2)
check("hooks/vibe: removal keeps the later entry",
      [h["name"] for h in tomllib.loads(open(vb2).read())["hooks"]] == ["later"])

# hermes: YAML it can't rewrite safely is refused, untouched
for name, text in (("flow", "plugins:\n  enabled: [a, b]\n"), ("flowmap", "plugins: {enabled: [a]}\n"),
                   ("indent", "plugins:\n    enabled:\n      - a\n")):
    hy = os.path.join(d, "hy-" + name, ".hermes", "config.yaml")
    write(hy, text)
    try:
        ac.hook_add("hermes", hy, HB, "600", SD)
        refused = False
    except SystemExit:
        refused = True
    check("hooks/hermes: %s YAML is refused and left as it was" % name,
          refused and open(hy).read() == text and not ac.hook_has("hermes", hy))
hy2 = os.path.join(d, "hy2", ".hermes", "config.yaml")
write(hy2, "plugins:\n  enabled:\n  - a\n  disabled: []\n")
ac.hook_add("hermes", hy2, HB, "600", SD)
check("hooks/hermes: items at the key's own indent are kept in that style",
      "  enabled:\n  - a\n  - speakeasy\n  disabled: []" in open(hy2).read(), open(hy2).read())

# JSON with comments is not rewritten
jc = os.path.join(d, "jc", ".gemini", "settings.json")
write(jc, '{\n  // mine\n  "a": 1\n}\n')
try:
    ac.hook_add("gemini", jc, HB, "600", SD)
    refused = False
except SystemExit:
    refused = True
check("hooks: a JSON file with comments is refused and left as it was", refused and "// mine" in open(jc).read())
# non-ASCII survives; no backup of SpeakEasy's own version
na = os.path.join(d, "na", ".gemini", "settings.json")
write(na, '{"title": "été"}')
ac.hook_add("gemini", na, HB, "600", SD)
ac.hook_add("gemini", na, HB, "300", SD)
check("hooks: non-ASCII text is written as is", "été" in open(na).read())
check("hooks: the backup is the user's original", "slack-approval-gateway" not in open(na + ac.BAK).read())
fresh = os.path.join(d, "fresh", ".gemini", "settings.json")
ac.hook_add("gemini", fresh, HB, "600", SD)
ac.hook_add("gemini", fresh, HB, "300", SD)
check("hooks: a file SpeakEasy created gets no backup of its own version", not os.path.exists(fresh + ac.BAK))
# a hook mentioning SpeakEasy is not an MCP registration
check("has: a hook alone is not a registration", not ac.has_entry(fresh))
ac.add_json("gemini", fresh, HB)
check("has: the MCP entry is", ac.has_entry(fresh))
# the ConfigChange guard lets changes through when the binary is gone
r = subprocess.run(["sh", "-c", ac.config_change_command("/nonexistent/slack-approval-gateway", 600, SD)],
                   input="{}", capture_output=True, text=True)
check("hooks/claude: the ConfigChange guard fails open without the binary", r.returncode == 0, r)

# gemini: milliseconds; cursor: no matcher, failClosed, other hooks kept
ge = os.path.join(d, ".gemini", "settings.json")
ac.hook_add("gemini", ge, HB, "600", SD)
bt = json.load(open(ge))["hooks"]["BeforeTool"][0]
check("hooks/gemini: timeout in milliseconds", bt["hooks"][0]["timeout"] == 660000, bt)
cu = os.path.join(d, ".cursor", "hooks.json")
write(cu, json.dumps({"version": 1, "hooks": {"beforeMCPExecution": [{"command": "/x/superset.sh Permission"}]}}))
ac.hook_add("cursor", cu, HB, "600", SD)
bm = json.load(open(cu))["hooks"]["beforeMCPExecution"]
check("hooks/cursor: failClosed, no matcher, other hook kept",
      len(bm) == 2 and bm[1]["failClosed"] is True and "matcher" not in bm[1] and bm[1]["timeout"] == 660, bm)
ac.hook_remove("cursor", cu)
check("hooks/cursor: remove keeps the other hook", json.load(open(cu)) == {"version": 1, "hooks": {"beforeMCPExecution": [{"command": "/x/superset.sh Permission"}]}})

# goose: integer timeout (a float makes goose skip the file)
go = os.path.join(d, ".agents", "plugins", "speakeasy-slack-approval", "hooks", "hooks.json")
ac.hook_add("goose", go, HB, "600", SD)
gh = json.load(open(go))["hooks"]["PreToolUse"][0]
check("hooks/goose: integer timeout", isinstance(gh["hooks"][0]["timeout"], int), gh)
check("hooks/goose: Code Mode in the matcher", "execute_typescript" in gh["matcher"])
check("hooks/goose: remove deletes the plugin", ac.hook_remove("goose", go) == "removed"
      and not os.path.exists(os.path.dirname(os.path.dirname(go))))

# vibe: a managed [[hooks]] block, strict, parsed; Superset's hook kept
vb = os.path.join(d, ".vibe", "hooks.toml")
write(vb, '[[hooks]]\nname = "superset-notify"\ntype = "pre_tool"\ncommand = "/x/n.sh"\n')
ac.hook_add("vibe", vb, HB, "600", SD)
ac.hook_add("vibe", vb, HB, "600", SD)
hooks = tomllib.loads(open(vb).read())["hooks"]
check("hooks/vibe: ours once, strict, float timeout, other kept",
      len(hooks) == 2 and hooks[1]["strict"] is True and hooks[1]["timeout"] == 660.0 and hooks[0]["name"] == "superset-notify", hooks)
check("hooks/vibe: remove", ac.hook_remove("vibe", vb) == "removed" and len(tomllib.loads(open(vb).read())["hooks"]) == 1)

# hermes: plugin + plugins.enabled entry, other plugins kept
hc = os.path.join(d, ".hermes", "config.yaml")
write(hc, "model: x\nplugins:\n  enabled:\n    - other-plugin\n  disabled: []\nmcp_servers: {}\n")
ac.hook_add("hermes", hc, HB, "600", SD)
ac.hook_add("hermes", hc, HB, "600", SD)
text = open(hc).read()
check("hooks/hermes: enabled once, other plugin kept",
      text.count("- speakeasy") == 1 and "- other-plugin" in text and "mcp_servers: {}" in text, text)
init = os.path.join(d, ".hermes", "plugins", "speakeasy", "__init__.py")
check("hooks/hermes: plugin written with the binary", HB in open(init).read())
check("hooks/hermes: has", ac.hook_has("hermes", hc))
ac.hook_remove("hermes", hc)
check("hooks/hermes: remove", "speakeasy" not in open(hc).read() and not os.path.exists(init), open(hc).read())
hc2 = os.path.join(d, "h2", ".hermes", "config.yaml")
write(hc2, "model: x\n")
ac.hook_add("hermes", hc2, HB, "600", SD)
check("hooks/hermes: plugins section created", "plugins:\n  enabled:\n    - speakeasy" in open(hc2).read(), open(hc2).read())
hc3 = os.path.join(d, "h3", ".hermes", "config.yaml")
write(hc3, "plugins:\n  enabled: []\n")
ac.hook_add("hermes", hc3, HB, "600", SD)
check("hooks/hermes: inline empty list expanded", "  enabled:\n    - speakeasy" in open(hc3).read(), open(hc3).read())

# single-file plugins: copilot, amp, opencode, pi
for agent, rel in (("copilot", ".copilot/hooks/speakeasy.json"), ("amp", ".config/amp/plugins/speakeasy.ts"),
                   ("opencode", ".config/opencode/plugins/speakeasy-slack-approval.js"),
                   ("pi", ".pi/agent/extensions/speakeasy-slack-gate.ts")):
    f = os.path.join(d, rel)
    ac.hook_add(agent, f, HB, "600", SD)
    check("hooks/%s: written with the binary and wait" % agent, HB in open(f).read() and "600" in open(f).read())
    check("hooks/%s: has" % agent, ac.hook_has(agent, f))
    check("hooks/%s: remove" % agent, ac.hook_remove(agent, f) == "removed" and not os.path.exists(f))
mine = os.path.join(d, ".copilot", "hooks", "theirs.json")
write(mine, "{}")
check("hooks: a file that is not ours is left alone", ac.hook_remove("copilot", mine) == "not ours (left alone)" and os.path.exists(mine))

# the shell wrapper fails closed: a missing binary, a crash, and exit 0 from
# an older binary without hook mode all block; the "proceed" code passes
for bin_path, needle, rc in (("/nonexistent/slack-approval-gateway", "not installed", 2),
                             ("/usr/bin/false", "hook failed (exit 1)", 2),
                             ("/usr/bin/true", "hook failed (exit 0)", 2)):
    cmd = ac.hook_command(bin_path, "claude", 60, SD)
    r = subprocess.run(["sh", "-c", cmd], input="{}", capture_output=True, text=True)
    check("hooks: wrapper blocks (exit 2) when the binary is %s" % bin_path, r.returncode == rc and needle in r.stderr, r)
fake = os.path.join(d, "fake-gateway")
write(fake, "#!/bin/sh\ncat >/dev/null; echo '{\"ok\":1}'; exit 10\n")
os.chmod(fake, 0o755)
r = subprocess.run(["sh", "-c", ac.hook_command(fake, "claude", 60, SD)], input="{}", capture_output=True, text=True)
check("hooks: wrapper turns the proceed code into exit 0 and keeps stdout", r.returncode == 0 and '"ok"' in r.stdout, r)
cu2 = os.path.join(d, "c2", ".cursor", "hooks.json")
ac.hook_add("cursor", cu2, "/Apps/My Tools/slack-approval-gateway", "600", SD)
cmd = json.load(open(cu2))["hooks"]["beforeMCPExecution"][0]["command"]
check("hooks/cursor: the binary path is quoted and there is no --ok-code",
      cmd.startswith("'/Apps/My Tools/slack-approval-gateway' --hook cursor") and "--ok-code" not in cmd, cmd)

# instruction variants
ins = os.path.join(d, "AGENTS.md")
ac.instruction_add(ins, "hooks")
check("instructions: hooks variant", "every message needs the user's approval" in open(ins).read())
ac.instruction_add(ins)
check("instructions: switching back replaces the block", "request_slack_message" in open(ins).read()
      and "every message needs" not in open(ins).read())
ac.instruction_add(ins, "confirm")
check("instructions: confirm variant", "ask the user before sending" in open(ins).read())
shutil.rmtree(d, ignore_errors=True)

print()
if failures:
    print(f"AGENT CONFIG TESTS FAILED: {len(failures)} failure(s): {failures}")
    sys.exit(1)
print("AGENT CONFIG TESTS PASSED: all checks green")
