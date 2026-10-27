#!/usr/bin/env python3
"""Print the licenses of the crates linked into the release binary, as
Markdown: what a binary release must carry along (the crates are compiled
into it). Reads `cargo metadata`, follows normal dependencies only (build and
dev dependencies are not linked) for macOS, and reproduces every license,
notice or copyright file each crate package ships (fonts/OFL.txt and the
like included). Identical texts are printed once, with every crate that
ships them.

Usage: scripts/third_party_licenses.py > THIRD_PARTY_LICENSES.md
"""
import collections
import json
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TARGETS = ("aarch64-apple-darwin", "x86_64-apple-darwin")
LICENSE_FILE = re.compile(r"(licen[cs]e|copying|notice|copyright|^ofl\b|^ufl\b)", re.I)


def metadata(target):
    out = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked", "--filter-platform", target],
        cwd=ROOT, check=True, capture_output=True, text=True,
    ).stdout
    return json.loads(out)


def linked_packages(meta):
    """Package ids reachable from the root through normal dependencies."""
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    root = meta["resolve"]["root"]
    seen, todo = set(), [root]
    while todo:
        pid = todo.pop()
        if pid in seen:
            continue
        seen.add(pid)
        for dep in nodes[pid]["deps"]:
            if any(k["kind"] is None for k in dep["dep_kinds"]):
                todo.append(dep["pkg"])
    seen.discard(root)
    return seen


def license_files(manifest_path):
    """License-like files at a package's root and one directory down."""
    base = os.path.dirname(manifest_path)
    found = []
    for dirpath, dirnames, filenames in os.walk(base):
        depth = os.path.relpath(dirpath, base).count(os.sep) + (dirpath != base)
        dirnames[:] = [d for d in dirnames if depth < 1 and not d.startswith(".")
                       and d not in ("src", "tests", "benches", "examples", "target")]
        for name in sorted(filenames):
            if LICENSE_FILE.search(name) and not name.endswith((".rs", ".toml", ".json")):
                found.append(os.path.join(dirpath, name))
    return sorted(found)


def main():
    packages = {}
    for target in TARGETS:
        meta = metadata(target)
        by_id = {p["id"]: p for p in meta["packages"]}
        for pid in linked_packages(meta):
            packages[pid] = by_id[pid]

    texts = collections.OrderedDict()  # text -> [(crate, file)]
    no_file = []
    for p in sorted(packages.values(), key=lambda p: (p["name"], p["version"])):
        crate = "%s %s" % (p["name"], p["version"])
        files = license_files(p["manifest_path"])
        if not files:
            no_file.append((crate, p.get("license") or p.get("license_file") or "unspecified"))
        for path in files:
            with open(path, encoding="utf-8", errors="replace") as f:
                text = f.read().strip()
            rel = os.path.relpath(path, os.path.dirname(p["manifest_path"]))
            texts.setdefault(text, []).append((crate, rel))

    w = sys.stdout.write
    w("# Third-party licenses\n\n")
    w("The SpeakEasy binary (`bin/slack-approval-gateway`) is built from these\n"
      "Rust crates (%d, for macOS), compiled in. Below is every license, notice\n"
      "and copyright file their packages ship, each text once with the crates\n"
      "that ship it. SpeakEasy's own license is in LICENSE, and the notices of\n"
      "the projects its window adapts are in NOTICE.md.\n\n" % len(packages))
    if no_file:
        w("## Crates that ship no license file\n\n")
        w("Their license, as their package declares it (the full MIT, Apache-2.0\n"
          "and other texts appear below, from crates that ship them):\n\n")
        for crate, lic in no_file:
            w("- %s — %s\n" % (crate, lic))
        w("\n")
    for text, owners in texts.items():
        w("## %s\n\n" % ", ".join(sorted({c for c, _ in owners})))
        files = sorted({f for _, f in owners})
        w("(%s)\n\n" % ", ".join(files))
        w("```text\n%s\n```\n\n" % text.replace("```", "'''"))
    return 0


if __name__ == "__main__":
    sys.exit(main())
