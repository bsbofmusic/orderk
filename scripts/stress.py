#!/usr/bin/env python3
"""orderk sqlite-vec stress verification.

This is intentionally black-box: it drives the release CLI, indexes a generated
Markdown vault, runs many sqlite-vec searches, verifies vector scores are present,
then checks incremental update/delete behavior.
"""

from __future__ import annotations

import concurrent.futures
import json
import os
import pathlib
import random
import shutil
import statistics
import subprocess
import tempfile
import time

try:
    import resource
except ImportError:  # pragma: no cover - resource is unavailable on Windows.
    resource = None

REPO = pathlib.Path(__file__).resolve().parents[1]
BIN = REPO / "target" / "release" / ("orderk.exe" if os.name == "nt" else "orderk")

NOTES = int(os.environ.get("ORDERK_STRESS_NOTES", "1000"))
QUERIES = int(os.environ.get("ORDERK_STRESS_QUERIES", "300"))
CONCURRENCY = int(os.environ.get("ORDERK_STRESS_CONCURRENCY", "12"))
DIM = int(os.environ.get("ORDERK_STRESS_DIM", "256"))
SEED = int(os.environ.get("ORDERK_STRESS_SEED", "424242"))


def run(args: list[str]) -> str:
    proc = subprocess.run(args, cwd=REPO, text=True, capture_output=True)
    if proc.returncode != 0:
        raise RuntimeError(
            f"failed: {' '.join(args)}\nstdout={proc.stdout}\nstderr={proc.stderr}"
        )
    return proc.stdout


def orderk(*args: str) -> dict:
    if not BIN.exists():
        raise RuntimeError(f"release binary missing: {BIN}; run cargo build --workspace --all-features --release")
    return json.loads(run([str(BIN), *args]))


def write_note(path: pathlib.Path, idx: int, topic: int, generation: str = "initial") -> None:
    anchor = f"anchor{idx:05d}"
    topic_token = f"topic{topic:05d}"
    repeated_anchor = " ".join([anchor] * 8)

    body = f"""---
tags: [stress, {topic_token}, {generation}]
---
# Stress Note {idx:05d}

{repeated_anchor}
This note belongs to {topic_token} and validates sqlite vec vector lookup.
orderk hybrid retrieval should expose non-zero vector scores for this document.
Generation marker: {generation}.
"""
    path.write_text(body, encoding="utf-8")


def percentile(values: list[float], pct: float) -> float:
    if not values:
        return 0.0
    data = sorted(values)
    idx = min(len(data) - 1, int(round((pct / 100.0) * (len(data) - 1))))
    return data[idx]


def max_rss_mb() -> float | None:
    """Return max RSS for child orderk processes in MiB, when the OS exposes it."""
    if resource is None:
        return None
    usage = resource.getrusage(resource.RUSAGE_CHILDREN)
    rss_kb = float(usage.ru_maxrss)
    if rss_kb <= 0:
        return None
    # Linux reports ru_maxrss in KiB. macOS reports bytes, but this release gate
    # runs on Linux CI/ops; keep a defensive conversion for unusually large values.
    if sys_platform_is_darwin():
        return round(rss_kb / 1024 / 1024, 2)
    return round(rss_kb / 1024, 2)


def sys_platform_is_darwin() -> bool:
    return os.uname().sysname == "Darwin" if hasattr(os, "uname") else False


def main() -> None:
    random.seed(SEED)
    root = pathlib.Path(tempfile.mkdtemp(prefix="orderk-stress-"))
    vault = root / "vault"
    db = root / "orderk.sqlite"
    try:
        (vault / "notes").mkdir(parents=True)
        for i in range(NOTES):
            write_note(vault / "notes" / f"note-{i:05d}.md", i, i)

        t0 = time.perf_counter()
        index = orderk(
            "index",
            "--vault", str(vault),
            "--db", str(db),
            "--embedding-provider", "mock",
            "--embedding-dim", str(DIM),
            "--embedding-model", f"mock-{DIM}",
            "--vector-backend", "sqlite_vec",
            "--json",
        )
        index_ms = round((time.perf_counter() - t0) * 1000)
        assert index["ok"] is True, index
        assert index["files"] == NOTES, index
        assert index["embedded"] >= NOTES, index
        assert index["vector_backend"] == "sqlite_vec", index

        status = orderk("status", "--db", str(db), "--json")
        assert status["ok"] is True, status
        assert status["notes"] == NOTES, status
        assert status["embeddings"] >= NOTES, status
        assert status["vector_enabled"] is True, status
        assert status["vector_backend"] == "sqlite_vec", status
        assert status["vec_version"], status

        sample_idx = NOTES // 2
        sample = orderk(
            "search", "--db", str(db),
            "--query", f"anchor{sample_idx:05d} topic{sample_idx:05d}",
            "--limit", "8",
            "--embedding-provider", "mock",
            "--embedding-dim", str(DIM),
            "--embedding-model", f"mock-{DIM}",
            "--vector-backend", "sqlite_vec",
            "--reranker", "none",
            "--json",
        )
        assert sample["vector_backend"] == "sqlite_vec", sample
        assert sample["results"], sample
        assert sample["results"][0]["score_breakdown"]["vector"] > 0, sample
        assert sample["results"][0]["path"].endswith(f"note-{sample_idx:05d}.md"), sample

        query_ids = [random.randrange(NOTES) for _ in range(QUERIES)]

        def one_query(idx: int) -> float:
            start = time.perf_counter()
            payload = orderk(
                "search", "--db", str(db),
                "--query", f"anchor{idx:05d} topic{idx:05d}",
                "--limit", "8",
                "--embedding-provider", "mock",
                "--embedding-dim", str(DIM),
                "--embedding-model", f"mock-{DIM}",
                "--vector-backend", "sqlite_vec",
                "--reranker", "none",
                "--json",
            )
            elapsed_ms = (time.perf_counter() - start) * 1000
            if payload["vector_backend"] != "sqlite_vec":
                raise AssertionError(payload)
            if not payload["results"]:
                raise AssertionError(payload)
            top = payload["results"][0]
            if top["score_breakdown"]["vector"] <= 0:
                raise AssertionError(payload)
            if not top["path"].endswith(f"note-{idx:05d}.md"):
                raise AssertionError(payload)
            return elapsed_ms

        with concurrent.futures.ThreadPoolExecutor(max_workers=CONCURRENCY) as pool:
            durations = list(pool.map(one_query, query_ids))

        update_count = max(1, NOTES // 20)
        delete_count = max(1, NOTES // 20)
        update_ids = list(range(update_count))
        delete_ids = list(range(NOTES - delete_count, NOTES))
        for i in update_ids:
            write_note(vault / "notes" / f"note-{i:05d}.md", i, i, generation="updated")
        for i in delete_ids:
            (vault / "notes" / f"note-{i:05d}.md").unlink()

        second_index = orderk(
            "index",
            "--vault", str(vault),
            "--db", str(db),
            "--embedding-provider", "mock",
            "--embedding-dim", str(DIM),
            "--embedding-model", f"mock-{DIM}",
            "--vector-backend", "sqlite_vec",
            "--json",
        )
        assert second_index["updated"] >= update_count, second_index
        assert second_index["deleted"] >= delete_count, second_index

        status_after = orderk("status", "--db", str(db), "--json")
        assert status_after["notes"] == NOTES - delete_count, status_after
        assert status_after["vector_enabled"] is True, status_after

        deleted_probe = orderk(
            "search", "--db", str(db),
            "--query", f"anchor{delete_ids[0]:05d}",
            "--limit", "20",
            "--embedding-provider", "mock",
            "--embedding-dim", str(DIM),
            "--embedding-model", f"mock-{DIM}",
            "--vector-backend", "sqlite_vec",
            "--reranker", "none",
            "--json",
        )
        assert all(not r["path"].endswith(f"note-{delete_ids[0]:05d}.md") for r in deleted_probe["results"]), deleted_probe

        summary = {
            "ok": True,
            "notes_indexed": NOTES,
            "queries": QUERIES,
            "concurrency": CONCURRENCY,
            "embedding_dim": DIM,
            "vector_backend": status["vector_backend"],
            "vec_version": status["vec_version"],
            "initial_index_ms": index_ms,
            "query_ms_min": round(min(durations), 2),
            "query_ms_p50": round(statistics.median(durations), 2),
            "query_ms_p95": round(percentile(durations, 95), 2),
            "query_ms_max": round(max(durations), 2),
            "max_rss_mb": max_rss_mb(),
            "updated": second_index["updated"],
            "deleted": second_index["deleted"],
            "final_notes": status_after["notes"],
        }
        print(json.dumps(summary, ensure_ascii=False, indent=2))
    finally:
        shutil.rmtree(root, ignore_errors=True)


if __name__ == "__main__":
    main()
