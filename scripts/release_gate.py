#!/usr/bin/env python3
"""Mechanical release gate for orderk.

This script intentionally does not ask an LLM to interpret quality. It runs the
same deterministic checks an agent/release operator should use before publishing.
"""

from __future__ import annotations

import json
import os
import pathlib
import subprocess
import sys
import time

REPO = pathlib.Path(__file__).resolve().parents[1]
TARGET_ORDERK = REPO / "target" / "release" / ("orderk.exe" if os.name == "nt" else "orderk")
GENERATED_VENDOR_ORDERK = REPO / "packages" / "cli" / "vendor" / ("orderk.exe" if os.name == "nt" else "orderk")

COMMANDS = [
    ["cargo", "test", "--workspace", "--all-features"],
    ["cargo", "build", "--workspace", "--all-features", "--release"],
    ["python3", "scripts/contract.py"],
    ["python3", "scripts/smoke.py"],
    ["python3", "scripts/stress.py"],
    ["python3", "scripts/eval.py"],
    ["npm", "install"],
    ["npm", "run", "build", "--workspaces", "--if-present"],
    ["npm", "test", "--workspaces", "--if-present"],
    ["npm", "pack", "--workspaces", "--dry-run"],
]


def run(cmd: list[str]) -> dict[str, object]:
    started = time.time()
    env = os.environ.copy()
    if cmd and cmd[0] == "npm":
        env["ORDERK_SKIP_BINARY_DOWNLOAD"] = "1"
        env["ORDERK_BIN"] = str(TARGET_ORDERK)
    proc = subprocess.run(cmd, cwd=REPO, text=True, capture_output=True, env=env)
    took_ms = int((time.time() - started) * 1000)
    return {
        "cmd": cmd,
        "ok": proc.returncode == 0,
        "exit_code": proc.returncode,
        "took_ms": took_ms,
        "stdout_tail": proc.stdout[-4000:],
        "stderr_tail": proc.stderr[-4000:],
    }


def main() -> int:
    if GENERATED_VENDOR_ORDERK.exists():
        GENERATED_VENDOR_ORDERK.unlink()
    results = []
    for cmd in COMMANDS:
        result = run(cmd)
        results.append(result)
        if not result["ok"]:
            print(json.dumps({"ok": False, "schema_version": "orderk.release_gate.v1", "failed": result, "results": results}, indent=2))
            return 1
    print(json.dumps({"ok": True, "schema_version": "orderk.release_gate.v1", "results": results}, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
