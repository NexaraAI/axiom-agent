#!/usr/bin/env python3
"""Report unused-import and unresolved-name errors from a cargo check run.

Run after moving code between modules to get a precise, short worklist
instead of scrolling full compiler output. Encoding-safe: reads the build
log as UTF-8.
"""
import io
import re
import subprocess
import sys

PKGS = {
    "axiom-cli": "crates/axiom-cli",
    "axiom-core": "crates/axiom-core",
    "axiom-engine": "crates/axiom-engine",
    "axiom-agent": "crates/axiom-agent",
    "axiom-llm": "crates/axiom-llm",
    "axiom-mcp": "crates/axiom-mcp",
    "axiom-proof": "crates/axiom-proof",
    "axiom-update": "crates/axiom-update",
    "axiom-lens": "crates/axiom-lens",
    "axiom-coder": "crates/axiom-coder",
}


def main():
    pkg = sys.argv[1] if len(sys.argv) > 1 else "axiom-cli"
    extra = sys.argv[2:]
    cmd = ["cargo", "check", "-p", pkg] + extra
    proc = subprocess.run(cmd, capture_output=True)
    log = proc.stdout.decode("utf-8", "replace") + proc.stderr.decode("utf-8", "replace")

    unused = set()
    missing = {}
    for m in re.finditer(r"unused import[s]?: (.+)", log):
        for name in re.findall(r"`([^`]+)`", m.group(1)):
            unused.add(name)
    for m in re.finditer(
        r"cannot find (?:type|value|function|struct|variant or union type|module or crate) "
        r"`?([A-Za-z_][A-Za-z0-9_]*)`? in this scope",
        log,
    ):
        missing.setdefault(m.group(1), 0)
        missing[m.group(1)] += 1

    if not unused and not missing:
        print("clean")
        return 0
    if unused:
        print("UNUSED IMPORTS (%d): %s" % (len(unused), ", ".join(sorted(unused))))
    if missing:
        print("UNRESOLVED NAMES (%d):" % len(missing))
        for name, count in sorted(missing.items()):
            print("   %-40s x%d" % (name, count))
    return 1


if __name__ == "__main__":
    sys.exit(main())
