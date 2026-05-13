#!/usr/bin/env python3
"""Turn orderk feedback events into reusable eval query fixtures.

This is a one-shot growth tool, not a daemon: it reads feedback evidence from an
existing orderk SQLite DB, derives explicit eval cases, and writes a merged
`orderk.eval_queries.v1` JSON file. It never mutates the DB or Markdown vault.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import sqlite3
from typing import Any

POSITIVE_EVENTS = {
    "accepted",
    "correct",
    "clicked",
    "selected",
    "relevant",
    "useful",
    "thumbs_up",
    "good_result",
}
CORRECTIVE_EVENTS = {
    "miss",
    "missing",
    "not_found",
    "zero_hit",
    "wrong_top",
    "wrong_result",
    "bad_result",
    "irrelevant",
}
EXPECTED_PATH_KEYS = (
    "expected_paths",
    "expected_path",
    "correct_paths",
    "correct_path",
    "target_paths",
    "target_path",
)


def normalize_event(event: str) -> str:
    return event.strip().lower().replace("-", "_").replace(" ", "_")


def normalize_query(query: str | None) -> str | None:
    if not query:
        return None
    cleaned = " ".join(query.split())
    return cleaned or None


def valid_eval_path(path: str) -> bool:
    if not path or path.startswith("/") or "\\" in path:
        return False
    rel = pathlib.PurePosixPath(path)
    if any(part in {"", ".", ".."} for part in rel.parts):
        return False
    return rel.suffix == ".md"


def append_path(paths: list[str], path: str) -> None:
    if valid_eval_path(path) and path not in paths:
        paths.append(path)


def extract_payload_paths(payload: dict[str, Any]) -> list[str]:
    paths: list[str] = []
    for key in EXPECTED_PATH_KEYS:
        raw = payload.get(key)
        if isinstance(raw, str):
            append_path(paths, raw)
        elif isinstance(raw, list):
            for item in raw:
                if isinstance(item, str):
                    append_path(paths, item)
    expected = payload.get("expected")
    if isinstance(expected, dict):
        for key in EXPECTED_PATH_KEYS + ("path", "file_path"):
            raw = expected.get(key)
            if isinstance(raw, str):
                append_path(paths, raw)
            elif isinstance(raw, list):
                for item in raw:
                    if isinstance(item, str):
                        append_path(paths, item)
    return paths


def feedback_case_id(query: str, expected_paths: list[str]) -> str:
    digest = hashlib.sha1((query + "\0" + "\0".join(expected_paths)).encode("utf-8")).hexdigest()[:12]
    return f"feedback-{digest}"


def open_readonly(db: pathlib.Path) -> sqlite3.Connection:
    uri = db.resolve().as_uri() + "?mode=ro"
    return sqlite3.connect(uri, uri=True)


def table_exists(conn: sqlite3.Connection, table: str) -> bool:
    row = conn.execute("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?", (table,)).fetchone()
    return row is not None


def chunk_path(conn: sqlite3.Connection, chunk_id: str | None) -> str | None:
    if not chunk_id or not table_exists(conn, "chunks"):
        return None
    row = conn.execute("SELECT file_path FROM chunks WHERE chunk_id = ? LIMIT 1", (chunk_id,)).fetchone()
    if not row:
        return None
    path = str(row[0])
    return path if valid_eval_path(path) else None


def collect_feedback_cases(db: pathlib.Path) -> dict[str, Any]:
    conn = open_readonly(db)
    try:
        if not table_exists(conn, "feedback_events"):
            return {"events_seen": 0, "cases": [], "skipped_events": [{"id": None, "reason": "feedback_events_missing"}]}
        rows = conn.execute(
            "SELECT id, event, query_id, chunk_id, query, payload FROM feedback_events ORDER BY id"
        ).fetchall()
        cases_by_query: dict[str, dict[str, Any]] = {}
        skipped: list[dict[str, Any]] = []
        for event_id, event, _query_id, row_chunk_id, row_query, payload_text in rows:
            event_name = normalize_event(str(event))
            try:
                payload = json.loads(payload_text or "{}")
            except json.JSONDecodeError:
                skipped.append({"id": event_id, "reason": "invalid_payload_json"})
                continue
            if not isinstance(payload, dict):
                skipped.append({"id": event_id, "reason": "payload_not_object"})
                continue
            query = normalize_query(row_query if row_query else payload.get("query"))
            if query is None:
                skipped.append({"id": event_id, "reason": "no_query"})
                continue

            expected_paths = extract_payload_paths(payload)
            if not expected_paths and event_name in POSITIVE_EVENTS:
                path = chunk_path(conn, row_chunk_id)
                if path:
                    expected_paths = [path]
            if not expected_paths:
                skipped.append({"id": event_id, "reason": "no_expected_path"})
                continue
            if event_name not in POSITIVE_EVENTS and event_name not in CORRECTIVE_EVENTS:
                skipped.append({"id": event_id, "reason": "unsupported_event"})
                continue

            key = query.lower()
            existing = cases_by_query.get(key)
            if existing is None:
                existing = {
                    "id": feedback_case_id(query, expected_paths),
                    "query": query,
                    "expected_paths": [],
                    "source": "feedback_events",
                    "source_event_ids": [],
                }
                cases_by_query[key] = existing
            for path in expected_paths:
                append_path(existing["expected_paths"], path)
            existing["id"] = feedback_case_id(existing["query"], existing["expected_paths"])
            existing["source_event_ids"].append(int(event_id))

        cases = [case for case in cases_by_query.values() if case["expected_paths"]]
        return {"events_seen": len(rows), "cases": cases, "skipped_events": skipped}
    finally:
        conn.close()


def load_eval_queries(path: pathlib.Path) -> dict[str, Any]:
    data = json.loads(path.read_text(encoding="utf-8"))
    if data.get("schema_version") != "orderk.eval_queries.v1":
        raise ValueError(f"unsupported eval query schema_version: {data.get('schema_version')}")
    if not isinstance(data.get("queries"), list):
        raise ValueError("eval queries file must contain a queries list")
    return data


def merge_eval_queries(existing: dict[str, Any], generated: list[dict[str, Any]]) -> tuple[dict[str, Any], dict[str, int]]:
    merged = {
        "schema_version": "orderk.eval_queries.v1",
        "queries": [dict(case) for case in existing.get("queries", [])],
    }
    by_query = {str(case.get("query", "")).strip().lower(): case for case in merged["queries"] if case.get("query")}
    stats = {"added": 0, "updated": 0, "unchanged": 0}
    for case in generated:
        query = normalize_query(case.get("query"))
        if query is None:
            stats["unchanged"] += 1
            continue
        expected_paths = [path for path in case.get("expected_paths", []) if isinstance(path, str) and valid_eval_path(path)]
        if not expected_paths:
            stats["unchanged"] += 1
            continue
        key = query.lower()
        target = by_query.get(key)
        if target is None:
            target = {
                "id": case.get("id") or feedback_case_id(query, expected_paths),
                "query": query,
                "expected_paths": [],
            }
            for optional_key in ("source", "source_event_ids"):
                if optional_key in case:
                    target[optional_key] = case[optional_key]
            for path in expected_paths:
                append_path(target["expected_paths"], path)
            merged["queries"].append(target)
            by_query[key] = target
            stats["added"] += 1
            continue
        before = list(target.get("expected_paths", []))
        target_paths = [path for path in before if isinstance(path, str) and valid_eval_path(path)]
        target["expected_paths"] = target_paths
        for path in expected_paths:
            append_path(target["expected_paths"], path)
        if target["expected_paths"] != before:
            stats["updated"] += 1
        else:
            stats["unchanged"] += 1
    return merged, stats


def run_growth(db: pathlib.Path, queries: pathlib.Path, out: pathlib.Path, min_generated: int = 0) -> dict[str, Any]:
    existing = load_eval_queries(queries)
    collected = collect_feedback_cases(db)
    merged, merge_stats = merge_eval_queries(existing, collected["cases"])
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(merged, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    generated_count = len(collected["cases"])
    ok = generated_count >= min_generated
    return {
        "ok": ok,
        "schema_version": "orderk.feedback_to_eval.v1",
        "db": str(db),
        "queries": str(queries),
        "out": str(out),
        "events_seen": collected["events_seen"],
        "generated_cases": generated_count,
        "skipped_events": collected["skipped_events"],
        "merge": merge_stats,
        "min_generated": min_generated,
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Generate eval query cases from orderk feedback_events")
    parser.add_argument("--db", required=True, type=pathlib.Path, help="Existing orderk SQLite DB to read")
    parser.add_argument("--queries", required=True, type=pathlib.Path, help="Existing orderk.eval_queries.v1 JSON")
    parser.add_argument("--out", required=True, type=pathlib.Path, help="Merged output JSON path")
    parser.add_argument("--min-generated", default=0, type=int, help="Fail unless at least this many feedback cases are generated")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    summary = run_growth(args.db, args.queries, args.out, args.min_generated)
    print(json.dumps(summary, ensure_ascii=False, indent=2))
    return 0 if summary["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
