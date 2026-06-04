#!/usr/bin/env python3
"""Representative real-vault Sword Spirit benchmark for orderk V2.

This is intentionally separate from scripts/sword_hs_bench.py:
- sword_hs_bench.py is a tiny 4-document wiring smoke.
- this script reads the live 3713-md Obsidian vault as the source corpus, copies a
  deterministic 50-document representative sample into /tmp, indexes only that
  sample vault, runs 50 deterministic real-note queries, and records Sword Spirit
  sidecar stats.

Hindsight is optional and disabled by default. When enabled, it is used only as
an isolated temporary reference bank over the selected sample documents, then
deleted. This script must not be reported as a full-vault production benchmark.
"""

from __future__ import annotations

import hashlib
import json
import os
import pathlib
import re
import shutil
import subprocess
import time
import urllib.error
import urllib.request
from collections import Counter
from datetime import datetime, timezone
from typing import Any

ROOT = pathlib.Path(os.getenv("ORDERK_REAL_BENCH_ROOT", "/tmp/orderk-sword-real-vault-bench"))
SOURCE_VAULT = pathlib.Path(os.getenv("ORDERK_REAL_SOURCE_VAULT", os.getenv("ORDERK_REAL_VAULT", "/home/agent/obsidian-vault")))
SAMPLE_VAULT = pathlib.Path(os.getenv("ORDERK_REAL_SAMPLE_VAULT", str(ROOT / "sample-vault")))
VAULT = SAMPLE_VAULT
PROJECT = pathlib.Path(os.getenv("ORDERK_PROJECT", "/home/agent/orderk"))
BIN = PROJECT / "target" / "debug" / "orderk"
DB = ROOT / "orderk-real-qwen3.sqlite"
HS = os.getenv("HINDSIGHT_API_BASE", "http://127.0.0.1:8765").rstrip("/")
BANK = "orderk-sword-real-bench-" + datetime.now(timezone.utc).strftime("%Y%m%d%H%M%S")
QUERY_COUNT = int(os.getenv("ORDERK_REAL_BENCH_QUERY_COUNT", "50"))
SEARCH_LIMIT = int(os.getenv("ORDERK_REAL_BENCH_SEARCH_LIMIT", "10"))
ACTIVE_SAMPLE_FILES = int(os.getenv("ORDERK_REAL_BENCH_ACTIVE_SAMPLE_FILES", str(QUERY_COUNT)))
RUN_ACTIVE_PROBE = os.getenv("ORDERK_REAL_BENCH_ACTIVE_PROBE", "1") not in {"0", "false", "False"}
RUN_HS_REFERENCE = os.getenv("ORDERK_REAL_BENCH_RUN_HS", "0") not in {"0", "false", "False"}
HS_QUERY_LIMIT = int(os.getenv("ORDERK_REAL_BENCH_HS_QUERY_LIMIT", "10"))
REPRESENTATIVE_BUCKETS = [
    ("brain/", 14),
    ("wiki/concepts/", 14),
    ("wiki/sources/", 8),
    ("wiki/reports/", 6),
    ("raw/articles/", 3),
    ("raw/media/", 2),
    ("raw/repositories/", 2),
    ("raw/system-snapshots/", 1),
]

STOPWORDS = {
    "the", "and", "for", "with", "that", "this", "from", "into", "about", "where", "which",
    "should", "orderk", "vault", "note", "notes", "markdown", "content", "index", "search",
    "使用", "一个", "这个", "我们", "不是", "什么", "可以", "自己", "如果", "没有", "因为", "所以",
}
TOKEN_RE = re.compile(r"[A-Za-z][A-Za-z0-9_\-]{3,}|[\u4e00-\u9fff]{2,}")


def clean_root() -> None:
    if ROOT.exists():
        shutil.rmtree(ROOT)
    ROOT.mkdir(parents=True)


def hidden_or_sidecar(path: pathlib.Path, root: pathlib.Path) -> bool:
    rel = path.relative_to(root)
    return any(part.startswith(".") for part in rel.parts)


def markdown_files(root: pathlib.Path) -> list[pathlib.Path]:
    files = []
    for path in root.rglob("*.md"):
        if hidden_or_sidecar(path, root):
            continue
        files.append(path)
    return sorted(files, key=lambda p: p.relative_to(root).as_posix())


def extract_title(text: str, path: pathlib.Path) -> str:
    for line in text.splitlines()[:80]:
        stripped = line.strip()
        if stripped.startswith("# "):
            title = stripped[2:].strip()
            if title:
                return title[:120]
    return path.stem.replace("-", " ").replace("_", " ")[:120]


def extract_tags(text: str) -> list[str]:
    tags: list[str] = []
    frontmatter = re.match(r"\A---\s*\n(.*?)\n---\s*\n", text, re.S)
    if frontmatter:
        for line in frontmatter.group(1).splitlines():
            if line.strip().startswith("tags:"):
                tags.extend(re.findall(r"[A-Za-z0-9_\-/\u4e00-\u9fff]+", line.split(":", 1)[1]))
    tags.extend(tag.strip("#") for tag in re.findall(r"#[A-Za-z0-9_\-/\u4e00-\u9fff]+", text[:3000]))
    seen = set()
    unique = []
    for tag in tags:
        if tag and tag not in seen and tag.lower() not in {"tags"}:
            seen.add(tag)
            unique.append(tag)
    return unique[:8]


def top_terms(text: str, title: str, tags: list[str]) -> list[str]:
    words = []
    for token in TOKEN_RE.findall((title + "\n" + " ".join(tags) + "\n" + text[:5000]).lower()):
        if token in STOPWORDS or len(token) > 32:
            continue
        words.append(token)
    counts = Counter(words)
    return [term for term, _count in counts.most_common(8)]


def doc_record(path: pathlib.Path, root: pathlib.Path) -> dict[str, Any] | None:
    rel = path.relative_to(root).as_posix()
    try:
        text = path.read_text(encoding="utf-8", errors="ignore")
    except OSError:
        return None
    title = extract_title(text, path)
    tags = extract_tags(text)
    terms = top_terms(text, title, tags)
    body = re.sub(r"\s+", " ", text).strip()
    if len(title) < 2 or len(body) < 80 or not terms:
        return None
    query = " ".join([title] + terms[:5])[:220]
    score = len(terms) + (4 if tags else 0) + min(len(body) // 500, 8)
    return {"path": rel, "title": title, "tags": tags, "terms": terms, "query": query, "score": score, "content": text}


def deterministic_pick(records: list[dict[str, Any]], cap: int, salt: str) -> list[dict[str, Any]]:
    records = sorted(records, key=lambda r: (-r["score"], hashlib.sha256(r["path"].encode()).hexdigest()))
    pool = records[: max(cap * 8, cap)]
    return sorted(pool, key=lambda r: hashlib.sha256((salt + r["path"]).encode()).hexdigest())[:cap]


def representative_records(files: list[pathlib.Path], root: pathlib.Path) -> list[dict[str, Any]]:
    all_records = [r for p in files if (r := doc_record(p, root)) is not None]
    by_path = {r["path"]: r for r in all_records}
    selected: list[dict[str, Any]] = []
    used: set[str] = set()
    for prefix, cap in REPRESENTATIVE_BUCKETS:
        bucket = [r for r in all_records if r["path"].startswith(prefix)]
        for record in deterministic_pick(bucket, cap, f"bucket:{prefix}:"):
            if record["path"] not in used:
                selected.append(record)
                used.add(record["path"])
    if len(selected) < QUERY_COUNT:
        remainder = [r for r in all_records if r["path"] not in used]
        for record in deterministic_pick(remainder, QUERY_COUNT - len(selected), "fill:"):
            selected.append(record)
            used.add(record["path"])
    selected = selected[:QUERY_COUNT]
    if len(selected) < QUERY_COUNT:
        raise RuntimeError(f"only selected {len(selected)} benchmark queries from {len(files)} markdown files")
    # Re-read from by_path order to keep selected records canonical and deterministic.
    return [by_path[r["path"]] for r in selected]


def select_queries(records: list[dict[str, Any]]) -> list[dict[str, Any]]:
    selected: list[dict[str, Any]] = []
    seen_paths: set[str] = set()
    for rec in records:
        if rec["path"] in seen_paths:
            continue
        seen_paths.add(rec["path"])
        selected.append(
            {
                "id": f"q{len(selected)+1:02d}",
                "query": rec["query"],
                "expected": [rec["path"]],
                "title": rec["title"],
                "tags": rec["tags"],
                "terms": rec["terms"],
                "path": rec["path"],
                "content": rec["content"],
            }
        )
        if len(selected) >= QUERY_COUNT:
            break
    if len(selected) < QUERY_COUNT:
        raise RuntimeError(f"only selected {len(selected)} benchmark queries from {len(records)} representative records")
    return selected


def prepare_sample_vault(selected: list[dict[str, Any]]) -> None:
    if SAMPLE_VAULT.exists():
        shutil.rmtree(SAMPLE_VAULT)
    SAMPLE_VAULT.mkdir(parents=True)
    for row in selected:
        src = SOURCE_VAULT / row["path"]
        dst = SAMPLE_VAULT / row["path"]
        dst.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(src, dst)


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


def run_json(cmd: list[Any], name: str, timeout: int = 600) -> dict[str, Any]:
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


def api(method: str, path: str, body: dict[str, Any] | None = None, timeout: int = 300) -> dict[str, Any]:
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
    except Exception as err:  # noqa: BLE001 - bench should preserve failure evidence.
        return {"status": None, "elapsed_ms": int((time.time() - started) * 1000), "error": repr(err)}


def orderk_paths(resp: dict[str, Any]) -> list[str]:
    return [row.get("path", "") for row in resp.get("results", [])]


def hs_paths(resp: dict[str, Any], selected: list[dict[str, Any]]) -> list[str]:
    by_doc = {row["path"]: row for row in selected}
    paths: list[str] = []
    for item in resp.get("results", []):
        doc = item.get("document_id") or item.get("document", {}).get("id") or ""
        if doc in by_doc:
            paths.append(doc)
            continue
        text = json.dumps(item, ensure_ascii=False).lower()
        matched = ""
        for path in by_doc:
            stem = pathlib.Path(path).stem.lower()
            if path.lower() in text or stem in text:
                matched = path
                break
        paths.append(matched or doc or "<unknown>")
    return paths


def rank_metrics(paths: list[str], expected: list[str]) -> dict[str, Any]:
    expected_set = set(expected)
    ranks = [idx for idx, path in enumerate(paths, 1) if path in expected_set]
    return {
        "top1_hit": bool(ranks and ranks[0] == 1),
        "hit_at_3": bool(ranks and min(ranks) <= 3),
        "hit_at_10": bool(ranks and min(ranks) <= 10),
        "mrr": 0.0 if not ranks else round(1.0 / min(ranks), 4),
        "matched_ranks": ranks,
    }


def aggregate(rows: list[dict[str, Any]], key: str) -> dict[str, Any]:
    vals = [row[key]["metrics"] for row in rows]
    return {
        "top1": sum(1 for row in vals if row["top1_hit"]),
        "hit_at_3": sum(1 for row in vals if row["hit_at_3"]),
        "hit_at_10": sum(1 for row in vals if row["hit_at_10"]),
        "mrr_avg": round(sum(row["mrr"] for row in vals) / max(1, len(vals)), 4),
        "n": len(vals),
    }


def line_count(path: pathlib.Path) -> int:
    if not path.exists():
        return 0
    with path.open("r", encoding="utf-8", errors="ignore") as handle:
        return sum(1 for line in handle if line.strip())


def run_hindsight_reference(selected: list[dict[str, Any]]) -> dict[str, Any]:
    hs: dict[str, Any] = {
        "scope": "temporary_bank_selected_50_docs_not_full_3713_vault",
        "bank": BANK,
    }
    try:
        hs["create"] = api(
            "PUT",
            f"/v1/default/banks/{BANK}",
            {
                "name": "orderk sword real-vault selected-doc benchmark",
                "retain_mission": "Extract factual claims from selected Markdown benchmark documents. Preserve exact document_id/path and title.",
                "reflect_mission": "Answer benchmark queries from selected Markdown facts only.",
                "enable_observations": False,
                "retain_extraction_mode": "concise",
            },
            timeout=120,
        )
        items = []
        for row in selected:
            items.append(
                {
                    "content": f"Document {row['path']}\nTitle: {row['title']}\nTags: {', '.join(row['tags'])}\n\n{row['content'][:8000]}",
                    "context": "orderk real-vault selected markdown benchmark source",
                    "document_id": row["path"],
                    "tags": ["orderk-sword-real-bench"],
                    "timestamp": "unset",
                    "metadata": {"source": "orderk-sword-real-bench", "path": row["path"]},
                }
            )
        hs["retain"] = api("POST", f"/v1/default/banks/{BANK}/memories", {"items": items, "async": False}, timeout=600)
        rows = []
        if hs.get("retain", {}).get("status") in {200, 201}:
            for query in selected:
                resp = api(
                    "POST",
                    f"/v1/default/banks/{BANK}/memories/recall",
                    {
                        "query": query["query"],
                        "budget": "low",
                        "max_tokens": 2048,
                        "tags": ["orderk-sword-real-bench"],
                        "tags_match": "all_strict",
                    },
                    timeout=180,
                )
                paths = hs_paths(resp.get("json") or {}, selected)
                rows.append(
                    {
                        "id": query["id"],
                        "query": query["query"],
                        "expected": query["expected"],
                        "status": resp.get("status"),
                        "elapsed_ms": resp.get("elapsed_ms"),
                        "paths": paths[:SEARCH_LIMIT],
                        "metrics": rank_metrics(paths, query["expected"]),
                    }
                )
        hs["recall_eval"] = rows
        hs["aggregate"] = {
            "top1": sum(1 for row in rows if row["metrics"]["top1_hit"]),
            "hit_at_3": sum(1 for row in rows if row["metrics"]["hit_at_3"]),
            "hit_at_10": sum(1 for row in rows if row["metrics"]["hit_at_10"]),
            "mrr_avg": round(sum(row["metrics"]["mrr"] for row in rows) / max(1, len(rows)), 4),
            "n": len(rows),
        }
    finally:
        hs["delete"] = api("DELETE", f"/v1/default/banks/{BANK}", None, timeout=120)
    return hs


def main() -> None:
    clean_root()
    source_files = markdown_files(SOURCE_VAULT)
    records = representative_records(source_files, SOURCE_VAULT)
    selected = select_queries(records)
    prepare_sample_vault(selected)
    files = markdown_files(VAULT)
    (ROOT / "queries.jsonl").write_text(
        "\n".join(json.dumps({k: v for k, v in row.items() if k != "content"}, ensure_ascii=False) for row in selected) + "\n",
        encoding="utf-8",
    )

    summary: dict[str, Any] = {
        "schema_version": "orderk.sword_spirit.real_vault_bench.v1",
        "bench_scope": "representative_50_doc_sample_from_real_3713_md_vault",
        "root": str(ROOT),
        "source_vault": str(SOURCE_VAULT),
        "sample_vault": str(VAULT),
        "source_vault_md_count": len(source_files),
        "sample_vault_md_count": len(files),
        "source_vault_bytes": sum(path.stat().st_size for path in source_files),
        "sample_vault_bytes": sum(path.stat().st_size for path in files),
        "representative_buckets": REPRESENTATIVE_BUCKETS,
        "db": str(DB),
        "query_count": len(selected),
        "search_limit": SEARCH_LIMIT,
        "queries_path": str(ROOT / "queries.jsonl"),
        "selected_docs": [{k: v for k, v in row.items() if k != "content"} for row in selected],
    }

    subprocess.run(["cargo", "build", "-p", "orderk-cli", "--all-features"], cwd=PROJECT, check=True)

    summary["sword_heuristic_full"] = run_json(
        [
            BIN,
            "sword",
            "run",
            "--vault",
            VAULT,
            "--thinking",
            "heuristic",
            "--max-files",
            len(files),
            "--max-proposals",
            "200",
            "--budget-profile",
            "digest_low",
            "--trace",
            "compact",
        ],
        "sword-heuristic-full",
        timeout=600,
    )

    if RUN_ACTIVE_PROBE:
        try:
            summary["sword_active_probe"] = run_json(
                [
                    BIN,
                    "sword",
                    "run",
                    "--vault",
                    VAULT,
                    "--thinking",
                    "active",
                    "--max-files",
                    ACTIVE_SAMPLE_FILES,
                    "--max-proposals",
                    "12",
                    "--budget-profile",
                    "digest_low",
                    "--trace",
                    "compact",
                ],
                "sword-active-probe",
                timeout=900,
            )
        except Exception as err:  # noqa: BLE001 - preserve blocker evidence but continue sample search bench.
            summary["sword_active_probe_error"] = repr(err)

    summary["index"] = run_json(
        [
            BIN,
            "index",
            "--vault",
            VAULT,
            "--db",
            DB,
            "--embedding-provider",
            "siliconflow",
            "--embedding-model",
            "Qwen/Qwen3-Embedding-4B",
            "--embedding-dim",
            "1024",
        ],
        "orderk-index-real-vault",
        timeout=1800,
    )

    orderk_rows = []
    for query in selected:
        base = run_json(
            [
                BIN,
                "search",
                "--db",
                DB,
                "--query",
                query["query"],
                "--limit",
                SEARCH_LIMIT,
                "--embedding-provider",
                "siliconflow",
                "--embedding-model",
                "Qwen/Qwen3-Embedding-4B",
                "--embedding-dim",
                "1024",
            ],
            f"orderk-base-{query['id']}",
            timeout=240,
        )
        sword = run_json(
            [
                BIN,
                "sword",
                "search",
                "--vault",
                VAULT,
                "--db",
                DB,
                "--query",
                query["query"],
                "--limit",
                SEARCH_LIMIT,
                "--embedding-provider",
                "siliconflow",
                "--embedding-model",
                "Qwen/Qwen3-Embedding-4B",
                "--embedding-dim",
                "1024",
            ],
            f"orderk-sword-{query['id']}",
            timeout=240,
        )
        base_paths = orderk_paths(base["json"])
        sword_paths = orderk_paths(sword["json"])
        orderk_rows.append(
            {
                "id": query["id"],
                "query": query["query"],
                "expected": query["expected"],
                "base": {
                    "paths": base_paths[:SEARCH_LIMIT],
                    "metrics": rank_metrics(base_paths, query["expected"]),
                    "elapsed_ms": base["elapsed_ms"],
                    "rss_kb": base["time"].get("Maximum resident set size (kbytes)"),
                    "took_ms": base["json"].get("took_ms"),
                },
                "sword": {
                    "paths": sword_paths[:SEARCH_LIMIT],
                    "metrics": rank_metrics(sword_paths, query["expected"]),
                    "elapsed_ms": sword["elapsed_ms"],
                    "rss_kb": sword["time"].get("Maximum resident set size (kbytes)"),
                    "took_ms": sword["json"].get("took_ms"),
                    "sidecar": sword["json"].get("sidecar"),
                },
            }
        )
    summary["orderk_eval"] = orderk_rows
    summary["aggregate"] = {
        "orderk_base": aggregate(orderk_rows, "base"),
        "orderk_sword": aggregate(orderk_rows, "sword"),
    }

    summary["hindsight_reference_enabled"] = RUN_HS_REFERENCE
    summary["hindsight_reference_query_limit"] = HS_QUERY_LIMIT if RUN_HS_REFERENCE else 0
    if RUN_HS_REFERENCE:
        try:
            summary["hindsight_reference"] = run_hindsight_reference(selected[:HS_QUERY_LIMIT])
        except Exception as err:  # noqa: BLE001 - bench remains useful without HS.
            summary["hindsight_reference_error"] = repr(err)

    heuristic_run_dir = pathlib.Path(summary["sword_heuristic_full"]["json"].get("run_dir", ""))
    active_run_dir = pathlib.Path(summary.get("sword_active_probe", {}).get("json", {}).get("run_dir", ""))
    summary["sidecar_counts"] = {
        "heuristic_run_dir": str(heuristic_run_dir) if heuristic_run_dir else "",
        "heuristic_neighbors": line_count(heuristic_run_dir / "neighbors.jsonl"),
        "heuristic_proposals": line_count(heuristic_run_dir / "proposals.jsonl"),
        "active_run_dir": str(active_run_dir) if active_run_dir else "",
        "active_neighbors": line_count(active_run_dir / "neighbors.jsonl") if active_run_dir else 0,
        "active_proposals": line_count(active_run_dir / "proposals.jsonl") if active_run_dir else 0,
    }

    (ROOT / "summary.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2), encoding="utf-8")
    compact = {
        "root": str(ROOT),
        "summary": str(ROOT / "summary.json"),
        "bench_scope": summary["bench_scope"],
        "source_vault_md_count": summary["source_vault_md_count"],
        "sample_vault_md_count": summary["sample_vault_md_count"],
        "source_vault_mb": round(summary["source_vault_bytes"] / 1024 / 1024, 2),
        "sample_vault_mb": round(summary["sample_vault_bytes"] / 1024 / 1024, 2),
        "query_count": summary["query_count"],
        "aggregate": summary["aggregate"],
        "hindsight_reference_aggregate": summary.get("hindsight_reference", {}).get("aggregate"),
        "sidecar_counts": summary["sidecar_counts"],
        "index_resource": summary["index"].get("time"),
        "heuristic_thinking": summary["sword_heuristic_full"]["json"].get("thinking"),
        "active_thinking": summary.get("sword_active_probe", {}).get("json", {}).get("thinking"),
        "active_probe_error": summary.get("sword_active_probe_error"),
    }
    print(json.dumps(compact, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
