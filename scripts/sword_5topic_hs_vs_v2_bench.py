#!/usr/bin/env python3
"""Deterministic 5-topic V2 retrieval benchmark.

This is the Batch 3 end-command companion to the heavier real-vault bench.  It
uses a small local Markdown fixture so it can run in release gates without LLM
or Hindsight credentials. Hindsight comparison is optional and disabled by
default; when disabled the JSON says so explicitly instead of inventing a PASS.
"""

from __future__ import annotations

import json
import os
import pathlib
import re
import shutil
import subprocess
import time
import urllib.error
import urllib.request
from datetime import datetime, timezone
from typing import Any

ROOT = pathlib.Path(os.getenv("ORDERK_5TOPIC_BENCH_ROOT", "/tmp/orderk-sword-5topic-hs-v2-bench"))
VAULT = ROOT / "vault"
PROJECT = (
    pathlib.Path(os.environ["ORDERK_PROJECT"])
    if os.getenv("ORDERK_PROJECT")
    else pathlib.Path(__file__).resolve().parents[1]
)
BIN = PROJECT / "target" / "debug" / "orderk"
DB = ROOT / "orderk-5topic.sqlite"
HS = os.getenv("HINDSIGHT_API_BASE", "http://127.0.0.1:8765").rstrip("/")
BANK = "orderk-5topic-bench-" + datetime.now(timezone.utc).strftime("%Y%m%d%H%M%S")
RUN_HS_REFERENCE = os.getenv("ORDERK_5TOPIC_RUN_HS", "0") not in {"0", "false", "False", ""}

DOCS: dict[str, str] = {
    "concepts/cashflow.md": """---
tags: [concept, finance, batch3]
source_type: concept
---
# 现金流

现金流 是资金流入与流出的时间序列。A good concept page answers the exact concept first, not merely any wiki/concepts page.
""",
    "concepts/noisy-investing.md": """---
tags: [concept, finance]
source_type: concept
---
# 投资噪声概念

这个概念页反复提到 现金流 现金流 现金流，但它不是现金流定义页，只是防止 blindly boosting every wiki concept page.
""",
    "scope/project-alpha.md": """---
tags: [project-alpha, strict-scope, batch3]
---
# Alpha strict scope

Scoped retrieval should return this alpha-only document when scope_tags asks for project-alpha.
""",
    "scope/project-beta.md": """---
tags: [project-beta, strict-scope]
---
# Beta strict scope decoy

Scoped retrieval should not leak this beta-only document into a project-alpha query even when the terms overlap.
""",
    "ranking/non-finite.md": """---
tags: [ranking, batch3]
---
# Non finite score guard

Retrieval sorting must sanitize NaN and Infinity score keys so corrupt numeric values never outrank finite evidence.
""",
    "sidecar/evidence-overlap.md": """---
tags: [sword, sidecar, batch3]
---
# Sword sidecar evidence overlap

Sword sidecar boosts are observational. A proposal must cite local evidence overlapping the current result before it may perturb ordering.
""",
    "trace/source-rank.md": """---
tags: [trace, source-rank, batch3]
---
# Source rank trace

Search explain traces keep keyword_rank, vector_rank, and source list evidence so ranking decisions remain inspectable.
""",
}

QUERIES: list[dict[str, Any]] = [
    {
        "id": "cashflow-concept",
        "query": "现金流",
        "expected": ["concepts/cashflow.md"],
        "scope_tags": [],
    },
    {
        "id": "strict-alpha-scope",
        "query": "strict scope overlapping project document",
        "expected": ["scope/project-alpha.md"],
        "scope_tags": ["project-alpha"],
    },
    {
        "id": "non-finite-ranking",
        "query": "NaN Infinity finite evidence retrieval sorting",
        "expected": ["ranking/non-finite.md"],
        "scope_tags": [],
    },
    {
        "id": "sidecar-evidence-overlap",
        "query": "Sword sidecar boosts local evidence overlapping current result",
        "expected": ["sidecar/evidence-overlap.md"],
        "scope_tags": [],
    },
    {
        "id": "source-rank-trace",
        "query": "keyword_rank vector_rank source list explain trace",
        "expected": ["trace/source-rank.md"],
        "scope_tags": [],
    },
]


def clean() -> None:
    if ROOT.exists():
        shutil.rmtree(ROOT)
    VAULT.mkdir(parents=True)
    for rel, content in DOCS.items():
        path = VAULT / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")


def parse_time(path: pathlib.Path) -> dict[str, str]:
    txt = path.read_text(errors="ignore") if path.exists() else ""
    out: dict[str, str] = {}
    for key in [
        "Elapsed (wall clock) time (h:mm:ss or m:ss)",
        "Maximum resident set size (kbytes)",
        "User time (seconds)",
        "System time (seconds)",
    ]:
        match = re.search(r"\t" + re.escape(key) + r": (.*)", txt)
        if match:
            out[key] = match.group(1)
    return out


def run_json(cmd: list[Any], name: str, timeout: int = 300) -> dict[str, Any]:
    out = ROOT / f"{name}.json"
    err = ROOT / f"{name}.stderr"
    tim = ROOT / f"{name}.time"
    full = ["/usr/bin/time", "-v", "-o", str(tim)] + [str(c) for c in cmd]
    started = time.time()
    proc = subprocess.run(full, cwd=PROJECT, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=timeout)
    elapsed_ms = int((time.time() - started) * 1000)
    out.write_text(proc.stdout, encoding="utf-8")
    err.write_text(proc.stderr, encoding="utf-8")
    result: dict[str, Any] = {
        "cmd": [str(c) for c in cmd],
        "exit_code": proc.returncode,
        "elapsed_ms": elapsed_ms,
        "time": parse_time(tim),
        "stderr_tail": proc.stderr[-1600:],
    }
    if proc.returncode != 0:
        result["stdout_tail"] = proc.stdout[-1600:]
        raise RuntimeError(f"{name} failed: {json.dumps(result, ensure_ascii=False)}")
    result["json"] = json.loads(proc.stdout)
    return result


def api(method: str, path: str, body: dict[str, Any] | None = None, timeout: int = 120) -> dict[str, Any]:
    data = None if body is None else json.dumps(body).encode("utf-8")
    req = urllib.request.Request(HS + path, data=data, method=method, headers={"Content-Type": "application/json"})
    started = time.time()
    try:
        with urllib.request.urlopen(req, timeout=timeout) as response:
            raw = response.read().decode("utf-8")
            return {
                "status": response.status,
                "elapsed_ms": int((time.time() - started) * 1000),
                "json": json.loads(raw) if raw else None,
            }
    except urllib.error.HTTPError as err:
        raw = err.read().decode("utf-8", errors="ignore")
        return {"status": err.code, "elapsed_ms": int((time.time() - started) * 1000), "error": raw[:2000]}
    except Exception as err:  # noqa: BLE001 - benchmark should preserve blocker evidence.
        return {"status": 0, "elapsed_ms": int((time.time() - started) * 1000), "error": repr(err)}


def rank_metrics(paths: list[str], expected: list[str]) -> dict[str, Any]:
    expected_set = set(expected)
    ranks = [idx for idx, path in enumerate(paths, start=1) if path in expected_set]
    return {
        "top1_hit": bool(ranks and ranks[0] == 1),
        "hit_at_3": bool(ranks and min(ranks) <= 3),
        "hit_at_5": bool(ranks and min(ranks) <= 5),
        "mrr": 0.0 if not ranks else round(1.0 / min(ranks), 4),
        "matched_ranks": ranks,
    }


def orderk_paths(resp: dict[str, Any]) -> list[str]:
    return [row["path"] for row in resp.get("results", [])]


def aggregate(rows: list[dict[str, Any]], key: str) -> dict[str, Any]:
    metrics = [row[key]["metrics"] for row in rows]
    return {
        "top1": sum(1 for metric in metrics if metric["top1_hit"]),
        "hit_at_3": sum(1 for metric in metrics if metric["hit_at_3"]),
        "hit_at_5": sum(1 for metric in metrics if metric["hit_at_5"]),
        "mrr_avg": round(sum(metric["mrr"] for metric in metrics) / max(1, len(metrics)), 4),
        "n": len(metrics),
    }


def quality_effect(base: dict[str, Any], sword: dict[str, Any]) -> dict[str, Any]:
    """Quantified quality comparison required for release closure.

    A green command only proves the harness ran. This object proves the new Sword
    path was compared against base retrieval with explicit effect metrics.
    """
    return {
        "comparison_type": "base_vs_sword",
        "metrics": {
            "query_count": sword["n"],
            "top1_delta": sword["top1"] - base["top1"],
            "hit_at_3_delta": sword["hit_at_3"] - base["hit_at_3"],
            "hit_at_5_delta": sword["hit_at_5"] - base["hit_at_5"],
            "mrr_avg_delta": round(float(sword["mrr_avg"]) - float(base["mrr_avg"]), 4),
            "base_top1": base["top1"],
            "sword_top1": sword["top1"],
            "base_hit_at_3": base["hit_at_3"],
            "sword_hit_at_3": sword["hit_at_3"],
            "base_hit_at_5": base["hit_at_5"],
            "sword_hit_at_5": sword["hit_at_5"],
            "base_mrr_avg": base["mrr_avg"],
            "sword_mrr_avg": sword["mrr_avg"],
        },
        "thresholds": {
            "min_query_count": 5,
            "min_top1_delta": 0,
            "min_hit_at_3_delta": 0,
            "min_hit_at_5_delta": 0,
            "min_mrr_avg_delta": 0.0,
        },
        "note": "Release closure requires quantified effect, not only pass/fail. Non-negative deltas are the deterministic no-regression floor; positive deltas are reported when present.",
    }


def scope_filter(tags: list[str]) -> list[str]:
    if not tags:
        return []
    clauses = ["tag == '" + tag.replace("\\", "\\\\").replace("'", "\\'") + "'" for tag in tags]
    return ["--filter", " && ".join(clauses)]


def maybe_hindsight_reference() -> dict[str, Any]:
    if not RUN_HS_REFERENCE:
        return {"enabled": False, "reason": "set ORDERK_5TOPIC_RUN_HS=1 to run isolated Hindsight reference"}
    hs: dict[str, Any] = {"enabled": True, "bank": BANK, "base_url": HS}
    try:
        hs["create"] = api(
            "PUT",
            f"/v1/default/banks/{BANK}",
            {
                "name": "orderk 5-topic benchmark",
                "retain_mission": "Extract exact retrieval facts from small benchmark Markdown documents.",
                "reflect_mission": "Answer only from retained benchmark facts.",
                "enable_observations": False,
                "retain_extraction_mode": "concise",
            },
        )
        if hs["create"].get("status") not in {200, 201}:
            return hs
        items = [
            {
                "content": f"Document {path}\n\n{content}",
                "context": "orderk 5-topic benchmark markdown source",
                "document_id": path,
                "tags": ["orderk-5topic-bench"],
                "metadata": {"source": "orderk-5topic-bench", "path": path},
            }
            for path, content in DOCS.items()
        ]
        hs["retain"] = api("POST", f"/v1/default/banks/{BANK}/memories", {"items": items, "async": False}, timeout=240)
    finally:
        hs["delete"] = api("DELETE", f"/v1/default/banks/{BANK}", None, timeout=120)
    return hs


def main() -> None:
    clean()
    summary: dict[str, Any] = {
        "schema_version": "orderk.sword_5topic_hs_vs_v2_bench.v1",
        "root": str(ROOT),
        "vault": str(VAULT),
        "db": str(DB),
        "query_count": len(QUERIES),
        "docs": sorted(DOCS),
        "queries": QUERIES,
        "hindsight_reference_enabled": RUN_HS_REFERENCE,
    }
    subprocess.run(["cargo", "build", "-p", "orderk-cli", "--all-features"], cwd=PROJECT, check=True)
    summary["sword_heuristic_run"] = run_json(
        [BIN, "sword", "run", "--vault", VAULT, "--thinking", "heuristic", "--max-files", len(DOCS), "--max-proposals", "30", "--budget-profile", "digest_low", "--trace", "compact"],
        "sword-heuristic",
        timeout=300,
    )
    summary["index"] = run_json(
        [BIN, "index", "--vault", VAULT, "--db", DB, "--embedding-provider", "mock", "--embedding-model", "mock-16", "--embedding-dim", "16"],
        "orderk-index",
        timeout=180,
    )
    rows = []
    for query in QUERIES:
        filter_args = scope_filter(query.get("scope_tags", []))
        base = run_json(
            [BIN, "search", "--db", DB, "--query", query["query"], "--limit", "5", *filter_args, "--embedding-provider", "mock", "--embedding-model", "mock-16", "--embedding-dim", "16", "--explain"],
            f"orderk-base-{query['id']}",
            timeout=120,
        )
        sword = run_json(
            [BIN, "sword", "search", "--vault", VAULT, "--db", DB, "--query", query["query"], "--limit", "5", *filter_args, "--embedding-provider", "mock", "--embedding-model", "mock-16", "--embedding-dim", "16"],
            f"orderk-sword-{query['id']}",
            timeout=120,
        )
        base_paths = orderk_paths(base["json"])
        sword_paths = orderk_paths(sword["json"])
        rows.append(
            {
                "id": query["id"],
                "query": query["query"],
                "scope_tags": query.get("scope_tags", []),
                "expected": query["expected"],
                "base": {
                    "paths": base_paths,
                    "metrics": rank_metrics(base_paths, query["expected"]),
                    "elapsed_ms": base["elapsed_ms"],
                    "took_ms": base["json"].get("took_ms"),
                    "explain_has_source_rank_trace": bool(
                        base["json"].get("explain", {}).get("result_ranks")
                        and all(
                            "keyword_rank" in item and "vector_rank" in item and "sources" in item
                            for item in base["json"].get("explain", {}).get("result_ranks", [])
                        )
                    ),
                },
                "sword": {
                    "paths": sword_paths,
                    "metrics": rank_metrics(sword_paths, query["expected"]),
                    "elapsed_ms": sword["elapsed_ms"],
                    "took_ms": sword["json"].get("took_ms"),
                    "sidecar": sword["json"].get("sidecar"),
                },
            }
        )
    summary["orderk_eval"] = rows
    summary["aggregate"] = {"orderk_base": aggregate(rows, "base"), "orderk_sword": aggregate(rows, "sword")}
    summary["quality_effect"] = quality_effect(
        summary["aggregate"]["orderk_base"],
        summary["aggregate"]["orderk_sword"],
    )
    summary["hindsight_reference"] = maybe_hindsight_reference()
    failures: list[str] = []
    for row in rows:
        if not row["base"]["metrics"]["top1_hit"]:
            failures.append(f"base top1 miss: {row['id']}")
        if not row["sword"]["metrics"]["top1_hit"]:
            failures.append(f"sword top1 miss: {row['id']}")
        if row["base"]["metrics"]["top1_hit"] and not row["sword"]["metrics"]["top1_hit"]:
            failures.append(f"sword top1 regression vs base: {row['id']}")
        if row["sword"].get("sidecar", {}).get("llm_calls") != 0:
            failures.append(f"query-time llm_calls != 0: {row['id']}")
    if not all(row["base"].get("explain_has_source_rank_trace") for row in rows):
        failures.append("base explain trace missing keyword_rank/vector_rank/sources")
    if summary["aggregate"]["orderk_sword"]["top1"] < summary["aggregate"]["orderk_base"]["top1"]:
        failures.append("aggregate sword top1 below base")
    if summary["aggregate"]["orderk_sword"]["mrr_avg"] < summary["aggregate"]["orderk_base"]["mrr_avg"]:
        failures.append("aggregate sword mrr below base")
    summary["ok"] = not failures
    summary["failures"] = failures
    (ROOT / "summary.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    compact = {
        "schema_version": summary["schema_version"],
        "ok": summary["ok"],
        "root": str(ROOT),
        "summary": str(ROOT / "summary.json"),
        "aggregate": summary["aggregate"],
        "quality_effect": summary["quality_effect"],
        "failures": failures,
        "hindsight_reference": summary["hindsight_reference"],
    }
    print(json.dumps(compact, ensure_ascii=False, indent=2))
    if failures:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
