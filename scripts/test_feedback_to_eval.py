#!/usr/bin/env python3
from __future__ import annotations

import importlib.util
import json
import pathlib
import sqlite3
import tempfile
import unittest

FEEDBACK_TO_EVAL_MODULE_PATH = pathlib.Path(__file__).resolve().parent / "feedback_to_eval.py"
spec = importlib.util.spec_from_file_location("orderk_feedback_to_eval", FEEDBACK_TO_EVAL_MODULE_PATH)
assert spec and spec.loader
feedback_to_eval = importlib.util.module_from_spec(spec)
spec.loader.exec_module(feedback_to_eval)


def make_db(path: pathlib.Path) -> None:
    conn = sqlite3.connect(path)
    conn.executescript(
        """
        CREATE TABLE chunks (
            chunk_id TEXT PRIMARY KEY,
            file_path TEXT NOT NULL
        );
        CREATE TABLE feedback_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            event TEXT NOT NULL,
            query_id TEXT,
            chunk_id TEXT,
            query TEXT,
            payload TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        """
    )
    conn.commit()
    conn.close()


def insert_feedback(
    db: pathlib.Path,
    *,
    event: str,
    query: str | None,
    payload: dict,
    chunk_id: str | None = None,
) -> None:
    conn = sqlite3.connect(db)
    conn.execute(
        "INSERT INTO feedback_events(event, query_id, chunk_id, query, payload, created_at) VALUES(?, ?, ?, ?, ?, ?)",
        (event, None, chunk_id, query, json.dumps(payload), "2026-05-13T00:00:00Z"),
    )
    conn.commit()
    conn.close()


class FeedbackToEvalTest(unittest.TestCase):
    def test_corrective_feedback_with_expected_path_becomes_eval_case(self) -> None:
        with tempfile.TemporaryDirectory(prefix="orderk-feedback-eval-test-") as tmp:
            db = pathlib.Path(tmp) / "orderk.sqlite"
            make_db(db)
            insert_feedback(
                db,
                event="wrong_top",
                query="vector search quality regression",
                payload={"expected_path": "quality/eval-gates.md"},
            )

            result = feedback_to_eval.collect_feedback_cases(db)

            self.assertEqual(result["events_seen"], 1, result)
            self.assertEqual(result["cases"][0]["query"], "vector search quality regression")
            self.assertEqual(result["cases"][0]["expected_paths"], ["quality/eval-gates.md"])
            self.assertTrue(result["cases"][0]["id"].startswith("feedback-"), result)
            self.assertEqual(result["cases"][0]["source_event_ids"], [1])

    def test_chunk_id_is_used_only_for_positive_feedback(self) -> None:
        with tempfile.TemporaryDirectory(prefix="orderk-feedback-eval-test-") as tmp:
            db = pathlib.Path(tmp) / "orderk.sqlite"
            make_db(db)
            conn = sqlite3.connect(db)
            conn.execute("INSERT INTO chunks(chunk_id, file_path) VALUES(?, ?)", ("good-chunk", "projects/right.md"))
            conn.execute("INSERT INTO chunks(chunk_id, file_path) VALUES(?, ?)", ("bad-chunk", "projects/wrong.md"))
            conn.commit()
            conn.close()
            insert_feedback(db, event="accepted", query="right project", payload={}, chunk_id="good-chunk")
            insert_feedback(db, event="bad_result", query="wrong project", payload={}, chunk_id="bad-chunk")

            result = feedback_to_eval.collect_feedback_cases(db)

            self.assertEqual(len(result["cases"]), 1, result)
            self.assertEqual(result["cases"][0]["expected_paths"], ["projects/right.md"])
            self.assertEqual(result["skipped_events"][0]["reason"], "no_expected_path")

    def test_merge_queries_dedupes_and_unions_expected_paths(self) -> None:
        existing = {
            "schema_version": "orderk.eval_queries.v1",
            "queries": [
                {"id": "manual", "query": "same query", "expected_paths": ["a.md"]},
            ],
        }
        generated = [
            {"id": "feedback-1", "query": "same query", "expected_paths": ["b.md"], "source_event_ids": [3]},
            {"id": "feedback-2", "query": "new query", "expected_paths": ["c.md"], "source_event_ids": [4]},
        ]

        merged, stats = feedback_to_eval.merge_eval_queries(existing, generated)

        self.assertEqual(stats, {"added": 1, "updated": 1, "unchanged": 0})
        self.assertEqual(len(merged["queries"]), 2)
        self.assertEqual(merged["queries"][0]["expected_paths"], ["a.md", "b.md"])
        self.assertEqual(merged["queries"][1]["id"], "feedback-2")

    def test_cli_writes_merged_queries_without_mutating_db(self) -> None:
        with tempfile.TemporaryDirectory(prefix="orderk-feedback-eval-test-") as tmp:
            root = pathlib.Path(tmp)
            db = root / "orderk.sqlite"
            queries = root / "queries.json"
            out = root / "merged.json"
            make_db(db)
            insert_feedback(
                db,
                event="miss",
                query="release resource baseline",
                payload={"expected_paths": ["ops/release-gate.md"]},
            )
            queries.write_text(
                json.dumps({"schema_version": "orderk.eval_queries.v1", "queries": []}),
                encoding="utf-8",
            )

            summary = feedback_to_eval.run_growth(db=db, queries=queries, out=out, min_generated=1)

            self.assertTrue(summary["ok"], summary)
            self.assertEqual(summary["generated_cases"], 1, summary)
            merged = json.loads(out.read_text(encoding="utf-8"))
            self.assertEqual(merged["queries"][0]["query"], "release resource baseline")
            self.assertEqual(merged["queries"][0]["expected_paths"], ["ops/release-gate.md"])


if __name__ == "__main__":
    unittest.main()
