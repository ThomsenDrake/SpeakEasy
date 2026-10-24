#!/usr/bin/env python3
"""Tests for scripts/agent_config.py: every config style, idempotency,
backups, removal, instruction blocks, and the audit report."""

import json
import os
import shutil
import subprocess
import sys
import tempfile

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
check("claude: remove restores backup", ac.remove_json("claude", p) == "restored backup")
check("claude: backup content is the original", "slack-approval-gateway" not in open(p + ac.BAK).read() and "grafana" in open(p).read())

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
check("goose: block shape", "  slack-approval-gateway:\n    enabled: true\n    type: stdio\n    cmd: \"%s\"\n    args: []\n    envs: {}\n    timeout: 120" % CMD in text, text)
check("goose: existing extension kept", "chatrecall" in text)
check("goose: idempotent", ac.add_goose(p, CMD) == "already")
check("goose: remove restores backup", ac.remove_goose(p) == "restored backup")
check("goose: original restored", "slack-approval-gateway" not in open(p).read() and "chatrecall" in open(p).read())

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
check("codex: block shape", '[mcp_servers.%s]\ncommand = "%s"\nargs = []' % (ac.SERVER, CMD) in text, text)
check("codex: existing config kept", 'model = "gpt-6.1-sol"' in text)
check("codex: idempotent", ac.add_codex(p, CMD) == "already")
check("codex: remove restores backup", ac.remove_codex(p) == "restored backup")
check("codex: original restored", ac.SERVER not in open(p).read() and "gpt-6.1-sol" in open(p).read())

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

# ------------------------------------------------------------------- audit
p = os.path.join(d, "audit-claude.json")
write(p, json.dumps({"mcpServers": {
    "slack": {"type": "stdio", "command": "go", "env": {"SLACK_MCP_XOXC_TOKEN": "xoxc-5330704053091-9437214043493-abcdef0123456789"}},
    ac.SERVER: {"type": "stdio", "command": CMD},
}}))
f = ac.audit_file(p)
check("audit: finds direct slack server", f["direct_slack_servers"] == ["slack"], json.dumps(f))
check("audit: finds token masked", len(f["slack_tokens"]) == 1 and f["slack_tokens"][0].startswith("xoxc-5330704") and "…" in f["slack_tokens"][0] and "9437214043493" not in f["slack_tokens"][0], json.dumps(f))
check("audit: gateway not flagged", ac.SERVER not in f["direct_slack_servers"])

p = os.path.join(d, "audit-hermes.yaml")
write(p, "platform_toolsets:\n  slack:\n    - hermes-slack\nmcp_servers:\n  executor:\n    url: https://x\n")
f = ac.audit_file(p)
check("audit: yaml slack key found", "slack" in f["direct_slack_servers"], json.dumps(f))

p = os.path.join(d, "audit-clean.json")
write(p, json.dumps({"mcpServers": {ac.SERVER: {"command": CMD}}}))
f = ac.audit_file(p)
check("audit: clean file", f["direct_slack_servers"] == [] and f["slack_tokens"] == [], json.dumps(f))

p = os.path.join(d, "audit-codex.toml")
write(p, '[mcp_servers.unofficial_slack]\ncommand = "go"\n\n[mcp_servers.%s]\ncommand = "%s"\n' % (ac.SERVER, CMD))
f = ac.audit_file(p)
check("audit: toml slack server found", f["direct_slack_servers"] == ["unofficial_slack"], json.dumps(f))
check("audit: toml gateway not flagged", ac.SERVER not in f["direct_slack_servers"])

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
check("audit: official slack mcp is expected", f["slack_mcp_servers"] == ["slack"], json.dumps(f))
check("audit: mistral connector is expected", f["mistral_connector_servers"] == ["mistral"], json.dumps(f))
check("audit: no direct servers", f["direct_slack_servers"] == [], json.dumps(f))
check("audit: mistral key masked",
      len(f["mistral_keys"]) == 1 and f["mistral_keys"][0].startswith("mstrl_abcdef")
      and "…" in f["mistral_keys"][0] and "0123456789" not in f["mistral_keys"][0],
      json.dumps(f))
check("audit: unrelated server ignored",
      "grafana" not in f["slack_mcp_servers"] + f["mistral_connector_servers"] + f["direct_slack_servers"],
      json.dumps(f))

p = os.path.join(d, "audit-codex-official.toml")
write(p, '[mcp_servers.slack]\ntype = "http"\nurl = "https://mcp.slack.com/mcp"\n\n[mcp_servers.%s]\ncommand = "%s"\n' % (ac.SERVER, CMD))
f = ac.audit_file(p)
check("audit: toml official slack mcp is expected", f["slack_mcp_servers"] == ["slack"], json.dumps(f))
check("audit: toml direct servers empty", f["direct_slack_servers"] == [], json.dumps(f))
check("audit: toml gateway not flagged", ac.SERVER not in f["slack_mcp_servers"] + f["direct_slack_servers"], json.dumps(f))

p = os.path.join(d, "audit-goose.yaml")
write(p, "extensions:\n  slack:\n    enabled: true\n    type: http\n    url: https://mcp.slack.com/mcp\n  chatrecall:\n    enabled: false\n")
f = ac.audit_file(p)
check("audit: yaml official slack mcp is expected", f["slack_mcp_servers"] == ["slack"], json.dumps(f))
check("audit: yaml unrelated extension ignored",
      "chatrecall" not in f["direct_slack_servers"] + f["slack_mcp_servers"], json.dumps(f))

p = os.path.join(d, "audit-amp.json")
write(p, json.dumps({"amp.mcpServers": {
    "slack": {"url": "https://mcp.slack.com/mcp"},
    ac.SERVER: {"command": CMD},
}}))
f = ac.audit_file(p)
check("audit: amp flat key scanned", f["slack_mcp_servers"] == ["slack"], json.dumps(f))

p = os.path.join(d, "audit-goose-providers.yaml")
write(p, "active_provider: mistral\nproviders:\n  mistral:\n    enabled: true\n    model: zai-glm-5-2\n    context_limit: 1000000\n")
f = ac.audit_file(p)
check("audit: yaml llm provider is not a connector server",
      f["mistral_connector_servers"] == [] and f["direct_slack_servers"] == [],
      json.dumps(f))

p = os.path.join(d, "audit-hermes-providers.yaml")
write(p, "providers:\n  mistral-glm-5-2:\n    name: Mistral GLM 5.2\n    base_url: https://api.mistral.ai/v1\n    key_env: MISTRAL_API_KEY\n")
f = ac.audit_file(p)
check("audit: yaml provider with base_url is not a connector server",
      f["mistral_connector_servers"] == [] and f["direct_slack_servers"] == [],
      json.dumps(f))

p = os.path.join(d, "audit-hermes-platform.yaml")
write(p, "platform_toolsets:\n  slack:\n    - hermes-slack\nmcp_servers:\n  executor:\n    url: https://x\n")
f = ac.audit_file(p)
check("audit: hermes platform slack still reported as direct",
      "slack" in f["direct_slack_servers"], json.dumps(f))
check("audit: hermes executor not flagged", "executor" not in f["direct_slack_servers"], json.dumps(f))

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

print()
if failures:
    print(f"AGENT CONFIG TESTS FAILED: {len(failures)} failure(s): {failures}")
    sys.exit(1)
print("AGENT CONFIG TESTS PASSED: all checks green")
