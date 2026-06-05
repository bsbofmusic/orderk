#!/usr/bin/env python3
"""Mechanical release gate for orderk.

This script intentionally does not ask an LLM to interpret quality. It runs the
same deterministic checks an agent/release operator should use before publishing.
"""

from __future__ import annotations

import json
import math
import os
import pathlib
import re
import subprocess
import sys
import time
import tomllib
from collections.abc import Iterable
from typing import Any

REPO = pathlib.Path(__file__).resolve().parents[1]
TARGET_ORDERK = REPO / "target" / "release" / ("orderk.exe" if os.name == "nt" else "orderk")
GENERATED_VENDOR_ORDERK = REPO / "packages" / "cli" / "vendor" / ("orderk.exe" if os.name == "nt" else "orderk")
RESOURCE_BASELINE = REPO / "baselines" / "orderk-resource-baseline.json"

COMMANDS = [
    [
        "python3",
        "-m",
        "unittest",
        "scripts/test_release_gate.py",
        "scripts/test_eval_gate.py",
        "scripts/test_feedback_to_eval.py",
        "scripts/test_v2_gate_suite.py",
        "scripts/test_sword_hs_bench.py",
    ],
    ["python3", "scripts/v2_gate_suite.py", "--only", "all", "--json"],
    ["cargo", "test", "-p", "orderk-core", "--all-features", "query_options_"],
    ["cargo", "test", "-p", "orderk-cli", "--all-features", "mcp_"],
    ["cargo", "fmt", "--all", "--", "--check"],
    ["cargo", "clippy", "--workspace", "--all-targets", "--all-features", "--", "-D", "warnings"],
    ["cargo", "test", "--workspace", "--all-features"],
    ["cargo", "build", "--workspace", "--all-features", "--release"],
    ["python3", "scripts/contract.py"],
    ["python3", "scripts/smoke.py"],
    ["python3", "scripts/stress.py"],
    ["python3", "scripts/eval.py"],
    ["python3", "scripts/sword_5topic_hs_vs_v2_bench.py"],
    ["npm", "install"],
    ["npm", "run", "build", "--workspaces", "--if-present"],
    ["npm", "test", "--workspaces", "--if-present"],
    ["npm", "pack", "--workspace", "orderk-cli", "--dry-run"],
]

SECRET_PATTERNS = [
    ("private_key", re.compile(r"BEGIN (?:RSA|OPENSSH|EC|DSA) PRIVATE KEY")),
    ("token_shape", re.compile(r"(?:sk|ghp|github_pat|xox[baprs]?)-[A-Za-z0-9_\-]{20,}")),
    (
        "secret_assignment",
        re.compile(r"(?i)\b(api[_-]?key|secret|password|passwd|token)\b\s*[:=]\s*['\"][^'\"]{12,}['\"]"),
    ),
]
SECRET_SCAN_EXCLUDES = {
    "Cargo.lock",
    "package-lock.json",
}
DANGEROUS_PACKAGE_SUFFIXES = (
    ".sqlite",
    ".sqlite-shm",
    ".sqlite-wal",
    ".log",
    ".pyc",
)
DANGEROUS_PACKAGE_PREFIXES = (
    "target/",
    "node_modules/",
    "packages/cli/vendor/",
)
LOCAL_TOOLING_PREFIX_EXCLUDES = (
    ".git/",
    ".codegraph/",
    "target/",
    "node_modules/",
)
PACKAGE_DIRECT_PREFIX_EXCLUDES = (
    ".git/",
    ".codegraph/",
)
DANGEROUS_PACKAGE_PARTS = {"__pycache__"}


def make_result(name: str, ok: bool, started: float, stdout: str = "", stderr: str = "") -> dict[str, object]:
    return {
        "cmd": ["internal", name],
        "ok": ok,
        "exit_code": 0 if ok else 1,
        "took_ms": int((time.time() - started) * 1000),
        "stdout_tail": stdout[-4000:],
        "stderr_tail": stderr[-4000:],
    }


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
        "stdout": proc.stdout,
        "stderr": proc.stderr,
        "stdout_tail": proc.stdout[-4000:],
        "stderr_tail": proc.stderr[-4000:],
    }


def list_repo_files(repo: pathlib.Path) -> list[pathlib.Path]:
    if (repo / ".git").exists():
        proc = subprocess.run(
            ["git", "ls-files", "--cached", "--others", "--exclude-standard"],
            cwd=repo,
            text=True,
            capture_output=True,
            check=False,
        )
        if proc.returncode == 0:
            return [pathlib.Path(line) for line in proc.stdout.splitlines() if line.strip()]
    return [path.relative_to(repo) for path in repo.rglob("*") if path.is_file() and ".git" not in path.parts]


def read_json(path: pathlib.Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8"))


def read_cargo_workspace_version(repo: pathlib.Path) -> str:
    data = tomllib.loads((repo / "Cargo.toml").read_text(encoding="utf-8"))
    return str(data["workspace"]["package"]["version"])


def check_version_consistency(repo: pathlib.Path = REPO) -> dict[str, object]:
    started = time.time()
    try:
        versions = {
            "Cargo.toml workspace.package.version": read_cargo_workspace_version(repo),
            "package.json": str(read_json(repo / "package.json")["version"]),
            "packages/cli/package.json": str(read_json(repo / "packages" / "cli" / "package.json")["version"]),
            "packages/obsidian/package.json": str(read_json(repo / "packages" / "obsidian" / "package.json")["version"]),
            "packages/obsidian/manifest.json": str(read_json(repo / "packages" / "obsidian" / "manifest.json")["version"]),
        }
        expected = versions["Cargo.toml workspace.package.version"]
        mismatches = {path: version for path, version in versions.items() if version != expected}
        versions_json = read_json(repo / "packages" / "obsidian" / "versions.json")
        if expected not in versions_json:
            mismatches["packages/obsidian/versions.json"] = f"missing {expected}"
        ok = not mismatches
        stdout = json.dumps({"expected": expected, "versions": versions}, indent=2, sort_keys=True)
        stderr = "" if ok else "version mismatch: " + json.dumps(mismatches, indent=2, sort_keys=True)
        return make_result("version_consistency", ok, started, stdout, stderr)
    except Exception as err:  # noqa: BLE001 - release gate should return structured failure evidence.
        return make_result("version_consistency", False, started, stderr=f"version consistency check failed: {err}")


def check_changelog(repo: pathlib.Path = REPO) -> dict[str, object]:
    """Check that CHANGELOG.md exists and contains a section for the current version."""
    started = time.time()
    try:
        version = read_cargo_workspace_version(repo)
        changelog_path = repo / "CHANGELOG.md"
        if not changelog_path.exists():
            return make_result(
                "changelog",
                False,
                started,
                stderr=f"CHANGELOG.md not found at {changelog_path}",
            )
        text = changelog_path.read_text(encoding="utf-8")
        pattern = re.compile(r"^## \[" + re.escape(version) + r"\]", re.MULTILINE)
        if not pattern.search(text):
            return make_result(
                "changelog",
                False,
                started,
                stderr=f"CHANGELOG.md missing section header '## [{version}]'",
            )
        return make_result(
            "changelog",
            True,
            started,
            stdout=f"CHANGELOG.md contains section for {version}",
        )
    except Exception as err:  # noqa: BLE001 - release gate should return structured failure evidence.
        return make_result("changelog", False, started, stderr=f"changelog check failed: {err}")


def check_secret_scan(repo: pathlib.Path = REPO, files: Iterable[pathlib.Path] | None = None) -> dict[str, object]:
    started = time.time()
    findings: list[str] = []
    scan_files = list(files) if files is not None else list_repo_files(repo)
    for rel in scan_files:
        rel = pathlib.Path(rel)
        rel_text = rel.as_posix()
        if rel.name in SECRET_SCAN_EXCLUDES or rel_text.startswith(LOCAL_TOOLING_PREFIX_EXCLUDES):
            continue
        path = repo / rel
        if not path.is_file():
            continue
        try:
            data = path.read_bytes()
        except OSError as err:
            findings.append(f"{rel_text}:0:read_error:{err}")
            continue
        if b"\0" in data or len(data) > 2_000_000:
            continue
        text = data.decode("utf-8", errors="ignore")
        for line_no, line in enumerate(text.splitlines(), start=1):
            for name, pattern in SECRET_PATTERNS:
                if pattern.search(line):
                    findings.append(f"{rel_text}:{line_no}:{name}")
    return make_result("secret_scan", not findings, started, "\n".join(findings), "" if not findings else "possible secrets found")


def check_package_cleanliness(repo: pathlib.Path = REPO, files: Iterable[pathlib.Path] | None = None) -> dict[str, object]:
    started = time.time()
    scan_files = list(files) if files is not None else list_repo_files(repo)
    dangerous: list[str] = []
    for rel in scan_files:
        rel_path = pathlib.Path(rel)
        rel_text = rel_path.as_posix()
        if rel_text.startswith(PACKAGE_DIRECT_PREFIX_EXCLUDES):
            continue
        if (
            rel_text.startswith(DANGEROUS_PACKAGE_PREFIXES)
            or rel_text.endswith(DANGEROUS_PACKAGE_SUFFIXES)
            or any(part in DANGEROUS_PACKAGE_PARTS for part in rel_path.parts)
        ):
            dangerous.append(rel_text)
        if rel_text == ".env" or rel_text.endswith("/.env"):
            dangerous.append(rel_text)
    if files is None:
        for rel_text in (".env", f"packages/cli/vendor/{TARGET_ORDERK.name}"):
            if (repo / rel_text).exists():
                dangerous.append(rel_text)
        for pattern in ("*.sqlite", "*.sqlite-shm", "*.sqlite-wal", "*.log"):
            for path in repo.rglob(pattern):
                rel_text = path.relative_to(repo).as_posix()
                if not rel_text.startswith(LOCAL_TOOLING_PREFIX_EXCLUDES):
                    dangerous.append(rel_text)
    return make_result(
        "package_cleanliness",
        not dangerous,
        started,
        "\n".join(sorted(set(dangerous))),
        "" if not dangerous else "runtime/build/private artifacts are present in package scope",
    )


def load_resource_baseline(repo: pathlib.Path = REPO) -> dict[str, Any]:
    return read_json(repo / RESOURCE_BASELINE.relative_to(REPO))


def count_orderk_processes() -> int:
    proc_root = pathlib.Path("/proc")
    if not proc_root.exists():
        return 0
    count = 0
    current_pid = os.getpid()
    for entry in proc_root.iterdir():
        if not entry.name.isdigit() or int(entry.name) == current_pid:
            continue
        try:
            comm = (entry / "comm").read_text(encoding="utf-8", errors="ignore").strip()
        except OSError:
            continue
        if comm == "orderk":
            count += 1
    return count


def check_stress_resource_baseline(
    stress_report: dict[str, Any],
    baseline: dict[str, Any] | None = None,
) -> dict[str, object]:
    started = time.time()
    baseline = baseline or load_resource_baseline(REPO)
    details: dict[str, Any] = {
        "schema_version": "orderk.stress_resource_gate.v1",
        "notes_indexed": stress_report.get("notes_indexed"),
        "queries": stress_report.get("queries"),
        "query_ms_p50": stress_report.get("query_ms_p50"),
        "query_ms_p95": stress_report.get("query_ms_p95"),
        "initial_index_ms": stress_report.get("initial_index_ms"),
        "max_rss_mb": stress_report.get("max_rss_mb"),
    }
    failures: list[str] = []

    def required_int(field: str) -> int | None:
        raw = stress_report.get(field)
        if raw is None:
            failures.append(f"{field} missing from stress report")
            return None
        try:
            return int(raw)
        except (TypeError, ValueError):
            failures.append(f"{field} invalid in stress report: {raw!r}")
            return None

    def required_float(field: str) -> float | None:
        raw = stress_report.get(field)
        if raw is None:
            failures.append(f"{field} missing from stress report")
            return None
        try:
            value = float(raw)
        except (TypeError, ValueError):
            failures.append(f"{field} invalid in stress report: {raw!r}")
            return None
        if not math.isfinite(value):
            failures.append(f"{field} invalid in stress report: {raw!r}")
            return None
        return value

    if stress_report.get("ok") is not True:
        failures.append("stress_report ok is not true")

    min_notes = int(baseline.get("stress_min_notes_indexed", 0))
    if min_notes:
        details["stress_min_notes_indexed"] = min_notes
        notes_indexed = required_int("notes_indexed")
        if notes_indexed is not None and notes_indexed < min_notes:
            failures.append(f"notes_indexed {notes_indexed} < stress_min_notes_indexed {min_notes}")

    min_queries = int(baseline.get("stress_min_queries", 0))
    if min_queries:
        details["stress_min_queries"] = min_queries
        queries = required_int("queries")
        if queries is not None and queries < min_queries:
            failures.append(f"queries {queries} < stress_min_queries {min_queries}")

    p50_max = float(baseline.get("mock_query_ms_p50_max", 0.0) or 0.0)
    if p50_max:
        details["mock_query_ms_p50_max"] = p50_max
        p50 = required_float("query_ms_p50")
        if p50 is not None and p50 > p50_max:
            failures.append(f"query_ms_p50 {p50} > mock_query_ms_p50_max {p50_max}")

    p95_max = float(baseline.get("mock_query_ms_p95_max", 0.0) or 0.0)
    if p95_max:
        details["mock_query_ms_p95_max"] = p95_max
        p95 = required_float("query_ms_p95")
        if p95 is not None and p95 > p95_max:
            failures.append(f"query_ms_p95 {p95} > mock_query_ms_p95_max {p95_max}")

    index_max = float(baseline.get("mock_index_ms_max", 0.0) or 0.0)
    if index_max:
        details["mock_index_ms_max"] = index_max
        index_ms = required_float("initial_index_ms")
        if index_ms is not None and index_ms > index_max:
            failures.append(f"initial_index_ms {index_ms} > mock_index_ms_max {index_max}")

    rss_max = float(baseline.get("mock_stress_rss_max_mb", 0.0) or 0.0)
    if rss_max:
        details["mock_stress_rss_max_mb"] = rss_max
        rss_mb = required_float("max_rss_mb")
        if rss_mb is not None and rss_mb > rss_max:
            failures.append(f"max_rss_mb {rss_mb} > mock_stress_rss_max_mb {rss_max}")

    ok = not failures
    stdout = json.dumps(details, indent=2, sort_keys=True)
    stderr = "" if ok else "\n".join(failures)
    return make_result("stress_resource_baseline", ok, started, stdout, stderr)


def check_quality_effect_comparison(bench_report: dict[str, Any]) -> dict[str, object]:
    """Require quantified base-vs-new effect metrics before release closure.

    Passing tests prove the system runs. Release closure also needs scorekeeping:
    explicit top1 / hit@k / MRR deltas against a baseline. Without this object a
    green benchmark is treated as incomplete evidence.
    """
    started = time.time()
    failures: list[str] = []
    details: dict[str, Any] = {
        "schema_version": "orderk.quality_effect_gate.v1",
        "bench_schema_version": bench_report.get("schema_version"),
        "bench_ok": bench_report.get("ok"),
    }
    if bench_report.get("ok") is not True:
        failures.append("bench ok is not true")
    effect = bench_report.get("quality_effect")
    if not isinstance(effect, dict):
        failures.append("quality_effect missing")
    else:
        metrics = effect.get("metrics")
        thresholds = effect.get("thresholds")
        details["comparison_type"] = effect.get("comparison_type")
        details["metrics"] = metrics
        details["thresholds"] = thresholds
        if effect.get("comparison_type") != "base_vs_sword":
            failures.append(f"quality_effect comparison_type invalid: {effect.get('comparison_type')!r}")
        if not isinstance(metrics, dict):
            failures.append("quality_effect metrics missing")
            metrics = {}
        if not isinstance(thresholds, dict):
            failures.append("quality_effect thresholds missing")
            thresholds = {}

        def metric_float(name: str) -> float | None:
            raw = metrics.get(name)
            try:
                value = float(raw)
            except (TypeError, ValueError):
                failures.append(f"quality_effect metric {name} invalid: {raw!r}")
                return None
            if not math.isfinite(value):
                failures.append(f"quality_effect metric {name} non-finite: {raw!r}")
                return None
            return value

        def threshold_float(name: str, default: float) -> float:
            raw = thresholds.get(name)
            if raw is None:
                raw = default
            try:
                value = float(raw)
            except (TypeError, ValueError):
                failures.append(f"quality_effect threshold {name} invalid: {raw!r}")
                return default
            return value

        query_count = metric_float("query_count")
        top1_delta = metric_float("top1_delta")
        hit_at_3_delta = metric_float("hit_at_3_delta")
        hit_at_5_delta = metric_float("hit_at_5_delta")
        mrr_avg_delta = metric_float("mrr_avg_delta")
        if query_count is not None and query_count < threshold_float("min_query_count", 1.0):
            failures.append(f"quality_effect query_count {query_count:g} below threshold")
        if top1_delta is not None and top1_delta < threshold_float("min_top1_delta", 0.0):
            failures.append(f"quality_effect top1_delta {top1_delta:g} below threshold")
        if hit_at_3_delta is not None and hit_at_3_delta < threshold_float("min_hit_at_3_delta", 0.0):
            failures.append(f"quality_effect hit_at_3_delta {hit_at_3_delta:g} below threshold")
        if hit_at_5_delta is not None and hit_at_5_delta < threshold_float("min_hit_at_5_delta", 0.0):
            failures.append(f"quality_effect hit_at_5_delta {hit_at_5_delta:g} below threshold")
        if mrr_avg_delta is not None and mrr_avg_delta < threshold_float("min_mrr_avg_delta", 0.0):
            failures.append(f"quality_effect mrr_avg_delta {mrr_avg_delta:g} below threshold")
    ok = not failures
    stdout = json.dumps(details, indent=2, sort_keys=True)
    stderr = "" if ok else "\n".join(failures)
    return make_result("quality_effect_comparison", ok, started, stdout, stderr)


def check_resource_baseline(
    repo: pathlib.Path = REPO,
    baseline: dict[str, Any] | None = None,
    binary: pathlib.Path = TARGET_ORDERK,
    run_runtime_checks: bool = True,
) -> dict[str, object]:
    started = time.time()
    try:
        baseline = baseline or load_resource_baseline(repo)
        details: dict[str, Any] = {
            "schema_version": "orderk.resource_gate.v1",
            "binary": str(binary),
        }
        failures: list[str] = []
        if not binary.exists():
            failures.append(f"release binary missing: {binary}")
        else:
            size = binary.stat().st_size
            details["binary_size_bytes"] = size
            max_size = int(baseline.get("binary_max_bytes", 0))
            details["binary_max_bytes"] = max_size
            if max_size and size > max_size:
                failures.append(f"binary_size_bytes {size} > binary_max_bytes {max_size}")
        if run_runtime_checks:
            daemon_count = count_orderk_processes()
            details["daemon_count"] = daemon_count
            max_daemon_count = int(baseline.get("daemon_count_max", 0))
            details["daemon_count_max"] = max_daemon_count
            if daemon_count > max_daemon_count:
                failures.append(f"daemon_count {daemon_count} > daemon_count_max {max_daemon_count}")
        ok = not failures
        stdout = json.dumps(details, indent=2, sort_keys=True)
        stderr = "" if ok else "\n".join(failures)
        return make_result("resource_baseline", ok, started, stdout, stderr)
    except Exception as err:  # noqa: BLE001 - release gate should return structured failure evidence.
        return make_result("resource_baseline", False, started, stderr=f"resource baseline check failed: {err}")


def emit_failure(failed: dict[str, object], results: list[dict[str, object]]) -> int:
    print(json.dumps({"ok": False, "schema_version": "orderk.release_gate.v1", "failed": failed, "results": results}, indent=2))
    return 1


def main() -> int:
    if GENERATED_VENDOR_ORDERK.exists():
        GENERATED_VENDOR_ORDERK.unlink()
    results: list[dict[str, object]] = []
    for check in (check_version_consistency, check_changelog, check_secret_scan, check_package_cleanliness):
        result = check(REPO)
        results.append(result)
        if not result["ok"]:
            return emit_failure(result, results)
    for cmd in COMMANDS:
        result = run(cmd)
        results.append(result)
        if not result["ok"]:
            return emit_failure(result, results)
        if cmd[:2] == ["cargo", "build"]:
            resource_result = check_resource_baseline(REPO)
            results.append(resource_result)
            if not resource_result["ok"]:
                return emit_failure(resource_result, results)
        if cmd == ["python3", "scripts/stress.py"]:
            try:
                stress_report = json.loads(str(result.get("stdout", result.get("stdout_tail", "{}"))))
            except json.JSONDecodeError as err:
                stress_resource_result = make_result(
                    "stress_resource_baseline",
                    False,
                    time.time(),
                    stderr=f"stress JSON parse failed: {err}",
                )
            else:
                stress_resource_result = check_stress_resource_baseline(stress_report)
            results.append(stress_resource_result)
            if not stress_resource_result["ok"]:
                return emit_failure(stress_resource_result, results)
        if cmd == ["python3", "scripts/sword_5topic_hs_vs_v2_bench.py"]:
            try:
                bench_report = json.loads(str(result.get("stdout", result.get("stdout_tail", "{}"))))
            except json.JSONDecodeError as err:
                quality_result = make_result(
                    "quality_effect_comparison",
                    False,
                    time.time(),
                    stderr=f"5-topic bench JSON parse failed: {err}",
                )
            else:
                quality_result = check_quality_effect_comparison(bench_report)
            results.append(quality_result)
            if not quality_result["ok"]:
                return emit_failure(quality_result, results)
    print(json.dumps({"ok": True, "schema_version": "orderk.release_gate.v1", "results": results}, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
