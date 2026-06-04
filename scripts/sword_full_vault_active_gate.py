#!/usr/bin/env python3
"""Full-vault active Sword Spirit gate.

This is the non-sample race gate for orderk V2 Sword Spirit. It copies a real
Markdown vault into /tmp, runs `orderk sword run --thinking active` over the whole
vault, and fails if the run falls back, skips files, mutates raw Markdown, or
omits required sidecar artifacts.

Required providers are the same HS-aligned stack used by Sword Spirit:
- SiliconFlow Qwen3 embedding/reranker envs already supported by orderk.
- MiniMax M3 Anthropic-compatible envs already supported by orderk.

Useful envs:
- ORDERK_FULL_VAULT_SOURCE=/path/to/vault
- ORDERK_FULL_VAULT_ROOT=/tmp/custom-root
- ORDERK_FULL_VAULT_MAX_FILES=3713
- ORDERK_FULL_VAULT_MAX_PROPOSALS=50
- ORDERK_FULL_VAULT_BUDGET_PROFILE=digest_low
- ORDERK_BIN=/path/to/orderk
"""

from __future__ import annotations

import hashlib
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
import time
import tomllib
from typing import Any

PROJECT = pathlib.Path(os.getenv("ORDERK_PROJECT", pathlib.Path(__file__).resolve().parents[1]))
SOURCE_VAULT = pathlib.Path(
    os.getenv("ORDERK_FULL_VAULT_SOURCE", "/home/agent/noteriv-vaults/obsidian-migration-test")
)
ROOT = pathlib.Path(
    os.getenv(
        "ORDERK_FULL_VAULT_ROOT",
        f"/tmp/orderk-sword-full-vault-active-{time.strftime('%Y%m%dT%H%M%S')}",
    )
)
VAULT = ROOT / "vault"
BIN = pathlib.Path(os.getenv("ORDERK_BIN", str(PROJECT / "target" / "debug" / "orderk")))
MAX_FILES = int(os.getenv("ORDERK_FULL_VAULT_MAX_FILES", "0"))
MAX_PROPOSALS = int(os.getenv("ORDERK_FULL_VAULT_MAX_PROPOSALS", "50"))
BUDGET_PROFILE = os.getenv("ORDERK_FULL_VAULT_BUDGET_PROFILE", "digest_low")
TRACE_LEVEL = os.getenv("ORDERK_FULL_VAULT_TRACE", "compact")


def hidden_or_sidecar(path: pathlib.Path, root: pathlib.Path) -> bool:
    rel = path.relative_to(root)
    return any(part.startswith(".") for part in rel.parts)


def markdown_hashes(root: pathlib.Path) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for path in sorted(root.rglob("*.md")):
        if hidden_or_sidecar(path, root):
            continue
        rows.append(
            {
                "path": path.relative_to(root).as_posix(),
                "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
                "size": path.stat().st_size,
            }
        )
    return rows


def line_count(path: pathlib.Path) -> int:
    if not path.exists():
        return 0
    return sum(1 for line in path.read_text(encoding="utf-8", errors="ignore").splitlines() if line.strip())


def parse_time(path: pathlib.Path) -> dict[str, str]:
    text = path.read_text(encoding="utf-8", errors="ignore") if path.exists() else ""
    out: dict[str, str] = {}
    for key in [
        "Elapsed (wall clock) time (h:mm:ss or m:ss)",
        "Maximum resident set size (kbytes)",
        "User time (seconds)",
        "System time (seconds)",
    ]:
        match = re.search(r"\t" + re.escape(key) + r": (.*)", text)
        if match:
            out[key] = match.group(1)
    return out


def run_command(cmd: list[str], stdout_path: pathlib.Path, time_path: pathlib.Path) -> dict[str, Any]:
    full = ["/usr/bin/time", "-v", "-o", str(time_path)] + cmd
    started = time.time()
    proc = subprocess.run(full, cwd=PROJECT, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=1200)
    stdout_path.write_text(proc.stdout, encoding="utf-8")
    (stdout_path.with_suffix(stdout_path.suffix + ".stderr")).write_text(proc.stderr, encoding="utf-8")
    return {
        "cmd": cmd,
        "exit_code": proc.returncode,
        "elapsed_ms": int((time.time() - started) * 1000),
        "stderr_tail": proc.stderr[-2000:],
        "time": parse_time(time_path),
    }


def workspace_version() -> str:
    data = tomllib.loads((PROJECT / "Cargo.toml").read_text(encoding="utf-8"))
    return str(data["workspace"]["package"]["version"])


def build_binary_if_needed() -> dict[str, Any]:
    if os.getenv("ORDERK_BIN"):
        build = {"skipped": True, "reason": "ORDERK_BIN override", "cmd": None, "exit_code": 0}
    else:
        started = time.time()
        proc = subprocess.run(
            ["cargo", "build", "-p", "orderk-cli", "--all-features"],
            cwd=PROJECT,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=600,
        )
        build = {
            "skipped": False,
            "cmd": ["cargo", "build", "-p", "orderk-cli", "--all-features"],
            "exit_code": proc.returncode,
            "elapsed_ms": int((time.time() - started) * 1000),
            "stdout_tail": proc.stdout[-2000:],
            "stderr_tail": proc.stderr[-2000:],
        }
        if proc.returncode != 0:
            return {"build": build, "version_check": {"ok": False, "error": "build failed"}}
    version_proc = subprocess.run(
        [str(BIN), "--version"],
        cwd=PROJECT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=30,
    )
    expected = workspace_version()
    actual = version_proc.stdout.strip()
    return {
        "build": build,
        "version_check": {
            "ok": version_proc.returncode == 0 and expected in actual,
            "expected": expected,
            "actual": actual,
            "exit_code": version_proc.returncode,
            "stderr_tail": version_proc.stderr[-1000:],
        },
    }


def fail(summary: dict[str, Any], reason: str) -> None:
    summary["ok"] = False
    summary.setdefault("failures", []).append(reason)


def main() -> int:
    if ROOT.exists():
        shutil.rmtree(ROOT)
    ROOT.mkdir(parents=True)
    shutil.copytree(SOURCE_VAULT, VAULT)

    before = markdown_hashes(VAULT)
    source_count = len(before)
    max_files = MAX_FILES or source_count
    (ROOT / "raw-hashes-before.json").write_text(json.dumps(before, ensure_ascii=False, indent=2), encoding="utf-8")

    binary_info = build_binary_if_needed()
    if not binary_info.get("version_check", {}).get("ok"):
        summary = {
            "ok": False,
            "schema_version": "orderk.sword_spirit.full_vault_active_gate.v1",
            "scope": "full_vault_active_digest",
            "root": str(ROOT),
            "source_vault": str(SOURCE_VAULT),
            "copied_vault": str(VAULT),
            "source_md_count": source_count,
            "binary": binary_info,
            "failures": ["binary build/version check failed"],
        }
        (ROOT / "summary.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2), encoding="utf-8")
        print(json.dumps(summary, ensure_ascii=False, indent=2))
        return 1

    command = [
        str(BIN),
        "sword",
        "run",
        "--vault",
        str(VAULT),
        "--thinking",
        "active",
        "--max-files",
        str(max_files),
        "--max-proposals",
        str(MAX_PROPOSALS),
        "--budget-profile",
        BUDGET_PROFILE,
        "--trace",
        TRACE_LEVEL,
    ]
    run_meta = run_command(command, ROOT / "sword-active.json", ROOT / "sword-active.time")

    after = markdown_hashes(VAULT)
    (ROOT / "raw-hashes-after.json").write_text(json.dumps(after, ensure_ascii=False, indent=2), encoding="utf-8")

    response: dict[str, Any] = {}
    if run_meta["exit_code"] == 0:
        response = json.loads((ROOT / "sword-active.json").read_text(encoding="utf-8"))
    run_dir = pathlib.Path(response.get("run_dir", ""))
    thinking = response.get("thinking") or {}
    sidecar_counts = {
        "neighbors": line_count(run_dir / "neighbors.jsonl"),
        "proposals": line_count(run_dir / "proposals.jsonl"),
        "rejected": line_count(run_dir / "rejected.jsonl"),
        "audit": line_count(run_dir / "audit.jsonl"),
        "report_exists": bool((run_dir / "report.md").exists()),
    }
    summary: dict[str, Any] = {
        "ok": True,
        "schema_version": "orderk.sword_spirit.full_vault_active_gate.v1",
        "scope": "full_vault_active_digest",
        "root": str(ROOT),
        "source_vault": str(SOURCE_VAULT),
        "copied_vault": str(VAULT),
        "source_md_count": source_count,
        "binary": binary_info,
        "run": run_meta,
        "response_path": str(ROOT / "sword-active.json"),
        "raw_unchanged": before == after,
        "files_scanned": response.get("files_scanned"),
        "files_considered": response.get("files_considered"),
        "proposal_count": response.get("proposal_count"),
        "rejected_count": response.get("rejected_count"),
        "run_dir": response.get("run_dir"),
        "sidecar_counts": sidecar_counts,
        "thinking": thinking,
        "warnings": response.get("warnings") or [],
    }

    if run_meta["exit_code"] != 0:
        fail(summary, f"sword active exited {run_meta['exit_code']}")
    if response.get("ok") is not True:
        fail(summary, "sword response ok was not true")
    if response.get("files_scanned") != source_count:
        fail(summary, "files_scanned did not equal source md count")
    if response.get("files_considered") != source_count:
        fail(summary, "files_considered did not equal source md count")
    if thinking.get("embedding_invocation") != "called" or thinking.get("embedded_count") != source_count:
        fail(summary, "embedding was not called for every source markdown file")
    if thinking.get("reranker_invocation") != "called" or int(thinking.get("reranked_count") or 0) <= 0:
        fail(summary, "reranker did not run")
    if thinking.get("llm_invocation") != "called" or int(thinking.get("llm_calls") or 0) <= 0:
        fail(summary, "LLM typed decision path did not complete")
    if thinking.get("fallback_invocation") != "not_used":
        fail(summary, "active digest used fallback instead of typed decisions")
    if not summary["raw_unchanged"]:
        fail(summary, "raw markdown changed during Sword Spirit run")
    if sidecar_counts["neighbors"] <= 0 or sidecar_counts["audit"] <= 0 or not sidecar_counts["report_exists"]:
        fail(summary, "required sidecar evidence is missing")

    (ROOT / "summary.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps(summary, ensure_ascii=False, indent=2))
    return 0 if summary["ok"] else 1


if __name__ == "__main__":
    sys.exit(main())
