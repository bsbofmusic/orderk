#!/usr/bin/env python3
from __future__ import annotations

import importlib.util
import json
import pathlib
import tempfile
import unittest

V3_EVAL_MODULE_PATH = pathlib.Path(__file__).resolve().parent / "v3_search_eval.py"
spec = importlib.util.spec_from_file_location("orderk_v3_search_eval", V3_EVAL_MODULE_PATH)
assert spec and spec.loader
v3_eval = importlib.util.module_from_spec(spec)
spec.loader.exec_module(v3_eval)


class V3SearchEvalMetricsTest(unittest.TestCase):
    def make_results(self, paths: list[str], sources: list[list[str]] | None = None, took_ms: int = 10) -> dict:
        sources = sources or [["keyword", "vector"] for _ in paths]
        return {
            "took_ms": took_ms,
            "routing": {"timings": {"total_ms": took_ms}},
            "results": [
                {
                    "path": path,
                    "chunk_id": f"{path}#chunk-{idx}",
                    "evidence": {"sources": sources[idx]},
                    "score": 1.0 - idx * 0.01,
                }
                for idx, path in enumerate(paths)
            ],
        }

    def test_metric_math_scores_hit_rank_mrr_ndcg_and_diversity(self) -> None:
        case = {
            "id": "alpha",
            "query": "alpha",
            "expected_paths": ["a.md", "b.md"],
            "expected_phrases": [],
        }
        response = self.make_results(
            ["noise.md", "a.md", "a.md", "b.md", "b.md"],
            sources=[
                ["vector"],
                ["keyword", "vector"],
                ["vector"],
                ["keyword", "route"],
                ["route"],
            ],
        )

        outcome = v3_eval.score_case(case, response, limit=5)

        self.assertTrue(outcome["hit_at_3"])
        self.assertTrue(outcome["hit_at_10"])
        self.assertFalse(outcome["hit_at_1"])
        self.assertEqual(outcome["rank"], 2)
        self.assertAlmostEqual(outcome["mrr_at_10"], 0.5)
        self.assertGreater(outcome["ndcg_at_10"], 0.0)
        self.assertEqual(outcome["unique_files_at_10"], 3)
        self.assertEqual(outcome["max_chunks_per_file_at_10"], 2)
        self.assertAlmostEqual(outcome["duplicate_file_rate_at_10"], 2 / 5)
        self.assertEqual(outcome["vector_only_top3"], 2)
        self.assertEqual(outcome["hybrid_confirmed_top3"], 1)

    def test_aggregate_reports_latency_percentiles_and_regressions(self) -> None:
        baseline = [
            v3_eval.score_case({"id": "a", "query": "a", "expected_paths": ["a.md"]}, self.make_results(["a.md"], took_ms=10), 10),
            v3_eval.score_case({"id": "b", "query": "b", "expected_paths": ["b.md"]}, self.make_results(["b.md"], took_ms=30), 10),
        ]
        candidate = [
            v3_eval.score_case({"id": "a", "query": "a", "expected_paths": ["a.md"]}, self.make_results(["x.md", "a.md"], took_ms=20), 10),
            v3_eval.score_case({"id": "b", "query": "b", "expected_paths": ["b.md"]}, self.make_results(["b.md"], took_ms=40), 10),
        ]

        report = v3_eval.aggregate_comparison(baseline, candidate, limit=10)

        self.assertEqual(report["schema_version"], "orderk.v3_search_eval.v1")
        self.assertEqual(report["baseline"]["hit_at_10"], 2)
        self.assertEqual(report["candidate"]["hit_at_10"], 2)
        self.assertLess(report["deltas"]["mrr_at_10"], 0.0)
        self.assertEqual(report["candidate"]["latency_p50_ms"], 40)
        self.assertEqual(report["candidate"]["latency_p95_ms"], 40)
        self.assertIn("a", report["regressions"]["mrr_at_10"])

    def test_quality_gate_rejects_missing_frozen_qrels_metric_regression_and_case_regression(self) -> None:
        report = {
            "schema_version": "orderk.v3_search_eval.v1",
            "qrels_frozen": False,
            "baseline": {"hit_at_10": 2, "mrr_at_10": 1.0, "ndcg_at_10": 1.0, "duplicate_file_rate_at_10": 0.3},
            "candidate": {"hit_at_10": 1, "mrr_at_10": 0.5, "ndcg_at_10": 0.6, "duplicate_file_rate_at_10": 0.8},
            "regressions": {"mrr_at_10": ["case-a"], "ndcg_at_10": [], "hit_at_10": ["case-b"]},
            "thresholds": {"max_duplicate_file_rate_at_10": 0.5},
        }

        gate = v3_eval.validate_quality_gate(report)

        self.assertFalse(gate["ok"])
        joined = "\n".join(gate["failures"])
        self.assertIn("qrels_frozen", joined)
        self.assertIn("hit_at_10", joined)
        self.assertIn("duplicate_file_rate_at_10", joined)
        self.assertIn("per_case", joined)

    def test_quality_gate_rejects_empty_improvement_when_candidate_equals_baseline(self) -> None:
        report = {
            "schema_version": "orderk.v3_search_eval.v1",
            "qrels_frozen": True,
            "baseline": {"hit_at_10": 2, "mrr_at_10": 0.75, "ndcg_at_10": 0.8, "duplicate_file_rate_at_10": 0.2},
            "candidate": {"hit_at_10": 2, "mrr_at_10": 0.75, "ndcg_at_10": 0.8, "duplicate_file_rate_at_10": 0.2},
            "deltas": {"hit_at_10": 0, "mrr_at_10": 0.0, "ndcg_at_10": 0.0, "duplicate_file_rate_at_10": 0.0},
            "regressions": {"mrr_at_10": [], "ndcg_at_10": [], "hit_at_10": []},
            "thresholds": {"max_duplicate_file_rate_at_10": 0.5, "require_positive_effect": True},
        }

        gate = v3_eval.validate_quality_gate(report)

        self.assertFalse(gate["ok"])
        self.assertIn("positive", "\n".join(gate["failures"]))

    def test_quality_gate_accepts_full_score_candidate_when_mandatory_reranker_evidence_is_added(self) -> None:
        report = {
            "schema_version": "orderk.v3_search_eval.v1",
            "qrels_frozen": True,
            "baseline": {
                "hit_at_10": 13,
                "mrr_at_10": 1.0,
                "ndcg_at_10": 1.0,
                "recall_at_10": 1.0,
                "duplicate_file_rate_at_10": 0.0,
                "reranker_evidence_rate_at_10": 0.0,
            },
            "candidate": {
                "hit_at_10": 13,
                "mrr_at_10": 1.0,
                "ndcg_at_10": 1.0,
                "recall_at_10": 1.0,
                "duplicate_file_rate_at_10": 0.0,
                "reranker_evidence_rate_at_10": 1.0,
            },
            "deltas": {
                "hit_at_10": 0,
                "mrr_at_10": 0.0,
                "ndcg_at_10": 0.0,
                "recall_at_10": 0.0,
                "duplicate_file_rate_at_10": 0.0,
                "reranker_evidence_rate_at_10": 1.0,
            },
            "regressions": {"mrr_at_10": [], "ndcg_at_10": [], "hit_at_10": []},
            "thresholds": {
                "max_duplicate_file_rate_at_10": 0.5,
                "require_positive_effect": True,
                "require_reranker_evidence": True,
            },
        }

        gate = v3_eval.validate_quality_gate(report)

        self.assertTrue(gate["ok"], gate)

    def test_quality_gate_rejects_missing_mandatory_reranker_evidence(self) -> None:
        report = {
            "schema_version": "orderk.v3_search_eval.v1",
            "qrels_frozen": True,
            "baseline": {"hit_at_10": 1, "mrr_at_10": 0.5, "ndcg_at_10": 0.5, "duplicate_file_rate_at_10": 0.0},
            "candidate": {
                "hit_at_10": 1,
                "mrr_at_10": 0.6,
                "ndcg_at_10": 0.6,
                "duplicate_file_rate_at_10": 0.0,
                "reranker_evidence_rate_at_10": 0.0,
            },
            "deltas": {"hit_at_10": 0, "mrr_at_10": 0.1, "ndcg_at_10": 0.1, "duplicate_file_rate_at_10": 0.0},
            "regressions": {"mrr_at_10": [], "ndcg_at_10": [], "hit_at_10": []},
            "thresholds": {"max_duplicate_file_rate_at_10": 0.5, "require_reranker_evidence": True},
        }

        gate = v3_eval.validate_quality_gate(report)

        self.assertFalse(gate["ok"])
        self.assertIn("reranker", "\n".join(gate["failures"]))

    def test_load_baseline_requires_schema_and_qrels_hash(self) -> None:
        with tempfile.TemporaryDirectory(prefix="orderk-v3-baseline-test-") as tmp:
            path = pathlib.Path(tmp) / "baseline.json"
            path.write_text(
                json.dumps(
                    {
                        "schema_version": "orderk.v3_search_baseline.v1",
                        "qrels_sha256": "abc123",
                        "fixture_sha256": "def456",
                        "baseline_outcomes": [],
                        "baseline": {"hit_at_10": 0, "mrr_at_10": 0, "ndcg_at_10": 0},
                    }
                ),
                encoding="utf-8",
            )
            loaded = v3_eval.load_baseline(path)
            self.assertEqual(loaded["qrels_sha256"], "abc123")

            path.write_text(json.dumps({"schema_version": "wrong"}), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "baseline"):
                v3_eval.load_baseline(path)

    def test_report_compares_candidate_to_frozen_baseline_not_live_baseline(self) -> None:
        baseline_case = v3_eval.score_case(
            {"id": "alpha", "query": "alpha", "expected_paths": ["alpha.md"]},
            self.make_results(["alpha.md"], took_ms=10),
            10,
        )
        candidate_case = v3_eval.score_case(
            {"id": "alpha", "query": "alpha", "expected_paths": ["alpha.md"]},
            self.make_results(["noise.md", "alpha.md"], took_ms=20),
            10,
        )
        baseline_doc = {
            "schema_version": "orderk.v3_search_baseline.v1",
            "qrels_sha256": "qrels-hash",
            "fixture_sha256": "fixture-hash",
            "baseline_outcomes": [baseline_case],
            "baseline": v3_eval.aggregate([baseline_case], 10),
        }

        report = v3_eval.aggregate_against_frozen_baseline(
            baseline_doc,
            [candidate_case],
            limit=10,
            qrels_sha256="qrels-hash",
            fixture_sha256="fixture-hash",
            candidate_args=["--v3-hybrid"],
        )

        self.assertEqual(report["baseline_outcomes"], [baseline_case])
        self.assertEqual(report["candidate_outcomes"], [candidate_case])
        self.assertLess(report["deltas"]["mrr_at_10"], 0.0)
        self.assertTrue(report["thresholds"]["require_positive_effect"])
        self.assertEqual(report["candidate_args"], ["--v3-hybrid"])
        self.assertFalse(report["quality_gate"]["ok"])

        with self.assertRaisesRegex(ValueError, "qrels_sha256"):
            v3_eval.aggregate_against_frozen_baseline(
                baseline_doc,
                [candidate_case],
                limit=10,
                qrels_sha256="changed",
                fixture_sha256="fixture-hash",
                candidate_args=[],
            )

    def test_baseline_artifact_shape_records_hashes_args_and_raw_outputs(self) -> None:
        case = {"id": "alpha", "query": "alpha", "expected_paths": ["alpha.md"]}
        response = self.make_results(["alpha.md"], took_ms=10)
        outcome = v3_eval.score_case(case, response, 10)

        artifact = v3_eval.build_baseline_artifact(
            qrels_path=pathlib.Path("fixtures/eval/v3_qrels.json"),
            qrels_sha256="qrels-hash",
            fixture_sha256="fixture-hash",
            limit=10,
            baseline_args=[],
            queries=[case],
            raw_outputs={"alpha": response},
            baseline_outcomes=[outcome],
        )

        self.assertEqual(artifact["schema_version"], "orderk.v3_search_baseline.v1")
        self.assertEqual(artifact["qrels_sha256"], "qrels-hash")
        self.assertEqual(artifact["fixture_sha256"], "fixture-hash")
        self.assertEqual(artifact["baseline_args"], [])
        self.assertEqual(artifact["raw_outputs"]["alpha"], response)
        self.assertEqual(artifact["baseline"], v3_eval.aggregate([outcome], 10))
        self.assertEqual(artifact["thresholds"]["require_positive_effect"], True)

    def test_run_fixture_eval_uses_frozen_baseline_file_and_only_searches_candidate(self) -> None:
        case = {"id": "alpha", "query": "alpha", "expected_paths": ["alpha.md"]}
        baseline_case = v3_eval.score_case(case, self.make_results(["alpha.md"], took_ms=10), 10)
        baseline_doc = {
            "schema_version": "orderk.v3_search_baseline.v1",
            "qrels_sha256": "qrels-hash",
            "fixture_sha256": "fixture-hash",
            "baseline_outcomes": [baseline_case],
            "baseline": v3_eval.aggregate([baseline_case], 10),
        }
        calls: list[list[str]] = []
        originals = {
            "load_qrels": v3_eval.load_qrels,
            "load_baseline": v3_eval.load_baseline,
            "file_sha256": v3_eval.file_sha256,
            "tree_sha256": v3_eval.tree_sha256,
            "index_fixture": v3_eval.index_fixture,
            "search": v3_eval.search,
        }
        try:
            v3_eval.load_qrels = lambda _path: {"queries": [case]}
            v3_eval.load_baseline = lambda _path: baseline_doc
            v3_eval.file_sha256 = lambda _path: "qrels-hash"
            v3_eval.tree_sha256 = lambda _path: "fixture-hash"
            v3_eval.index_fixture = lambda _vault, _db: None

            def fake_search(_db: pathlib.Path, _query: str, _limit: int, extra_args: list[str]) -> dict:
                calls.append(extra_args)
                return self.make_results(["noise.md", "alpha.md"], took_ms=20)

            v3_eval.search = fake_search
            report = v3_eval.run_fixture_eval(pathlib.Path("qrels.json"), pathlib.Path("baseline.json"), 10, ["--candidate"])
        finally:
            for name, value in originals.items():
                setattr(v3_eval, name, value)

        self.assertEqual(calls, [["--candidate"]])
        self.assertEqual(report["baseline_outcomes"], [baseline_case])
        self.assertEqual(report["candidate_args"], ["--candidate"])

    def test_qrels_file_requires_frozen_true_and_expected_paths(self) -> None:
        with tempfile.TemporaryDirectory(prefix="orderk-v3-qrels-test-") as tmp:
            path = pathlib.Path(tmp) / "qrels.json"
            path.write_text(
                json.dumps(
                    {
                        "schema_version": "orderk.v3_qrels.v1",
                        "frozen": True,
                        "queries": [
                            {"id": "alpha", "query": "alpha", "expected_paths": ["alpha.md"], "expected_phrases": []}
                        ],
                    }
                ),
                encoding="utf-8",
            )
            loaded = v3_eval.load_qrels(path)
            self.assertEqual(loaded["queries"][0]["id"], "alpha")

            path.write_text(
                json.dumps({"schema_version": "orderk.v3_qrels.v1", "frozen": False, "queries": []}),
                encoding="utf-8",
            )
            with self.assertRaisesRegex(ValueError, "frozen"):
                v3_eval.load_qrels(path)


if __name__ == "__main__":
    unittest.main()
