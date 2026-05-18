#!/usr/bin/env python3
from __future__ import annotations

import importlib.util
import pathlib
import tempfile
import unittest

EVAL_MODULE_PATH = pathlib.Path(__file__).resolve().parent / "eval.py"
spec = importlib.util.spec_from_file_location("orderk_eval_script", EVAL_MODULE_PATH)
assert spec and spec.loader
orderk_eval = importlib.util.module_from_spec(spec)
spec.loader.exec_module(orderk_eval)


class EvalQualityGateTest(unittest.TestCase):
    def perfect_report(self) -> dict:
        return {
            "schema_version": "orderk.eval.v1",
            "ok": True,
            "queries": 2,
            "hits_at_k": 2,
            "top1_hits": 2,
            "zero_hit": 0,
            "recall_at_k": 1.0,
            "ndcg_at_k": 1.0,
            "mrr": 1.0,
            "mean_took_ms": 12.5,
            "outcomes": [
                {"id": "alpha", "hit": True, "rank": 1, "recall_at_k": 1.0, "ndcg_at_k": 1.0},
                {"id": "bravo", "hit": True, "rank": 1, "recall_at_k": 1.0, "ndcg_at_k": 1.0},
            ],
        }

    def baseline(self) -> dict:
        return {
            "schema_version": "orderk.eval_baseline.v1",
            "min_queries": 2,
            "max_zero_hit": 0,
            "min_hits_at_k": 2,
            "min_top1_hits": 2,
            "min_recall_at_k": 1.0,
            "min_ndcg_at_k": 1.0,
            "min_mrr": 1.0,
            "max_mean_took_ms": 1000.0,
            "required_case_ids": ["alpha", "bravo"],
            "case_thresholds": {
                "require_hit": True,
                "max_rank": 1,
                "min_recall_at_k": 1.0,
                "min_ndcg_at_k": 1.0,
            },
        }

    def test_eval_quality_gate_accepts_matching_report(self) -> None:
        result = orderk_eval.validate_eval_quality(self.perfect_report(), self.baseline())
        self.assertTrue(result["ok"], result)
        self.assertEqual(result["failures"], [])

    def test_eval_quality_gate_rejects_metric_regression(self) -> None:
        report = self.perfect_report()
        report.update({"zero_hit": 1, "recall_at_k": 0.5, "ndcg_at_k": 0.4, "mrr": 0.25})
        report["outcomes"][1].update({"hit": False, "rank": None, "recall_at_k": 0.0, "ndcg_at_k": 0.0})
        result = orderk_eval.validate_eval_quality(report, self.baseline())
        self.assertFalse(result["ok"], result)
        joined = "\n".join(result["failures"])
        self.assertIn("zero_hit", joined)
        self.assertIn("recall_at_k", joined)
        self.assertIn("case bravo", joined)

    def test_eval_quality_gate_rejects_missing_required_case(self) -> None:
        baseline = self.baseline()
        baseline["required_case_ids"] = ["alpha", "missing"]
        result = orderk_eval.validate_eval_quality(self.perfect_report(), baseline)
        self.assertFalse(result["ok"], result)
        self.assertIn("missing required case id: missing", "\n".join(result["failures"]))

    def test_eval_quality_gate_rejects_missing_expected_phrase_evidence(self) -> None:
        report = self.perfect_report()
        report["outcomes"][0].update(
            {
                "expected_phrases": ["sqlite vec semantic search"],
                "matched_expected_phrases": [],
            }
        )
        result = orderk_eval.validate_eval_quality(report, self.baseline())
        self.assertFalse(result["ok"], result)
        joined = "\n".join(result["failures"])
        self.assertIn("case alpha missing expected phrase", joined)

    def test_fixture_files_must_exist(self) -> None:
        with tempfile.TemporaryDirectory(prefix="orderk-eval-fixture-test-") as tmp:
            root = pathlib.Path(tmp)
            vault = root / "vault"
            queries = root / "queries.json"
            baseline = root / "baseline.json"
            failures = orderk_eval.validate_fixture_paths(vault, queries, baseline)
            self.assertEqual(
                failures,
                [
                    f"eval fixture vault missing: {vault}",
                    f"eval queries file missing: {queries}",
                    f"eval baseline file missing: {baseline}",
                ],
            )


if __name__ == "__main__":
    unittest.main()
