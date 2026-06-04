#!/usr/bin/env python3
from __future__ import annotations

import json
import pathlib
import sys
import tempfile
import unittest

SCRIPT_DIR = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR))

import v2_gate_suite


def write_jsonl(path: pathlib.Path, rows: list[dict]) -> None:
    path.write_text("".join(json.dumps(row) + "\n" for row in rows), encoding="utf-8")


class V2GateSuiteTests(unittest.TestCase):
    def test_fixture_integrity_rejects_too_few_cases(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            vault = root / "vault"
            vault.mkdir()
            (vault / "a.md").write_text("# A\n", encoding="utf-8")
            golden = root / "golden.jsonl"
            digest = root / "digest.jsonl"
            write_jsonl(golden, [{"id": "q1", "query": "a", "expected_paths": ["a.md"]}])
            write_jsonl(digest, [{"id": "d1", "y_raw": [], "expected_proposals": []}])
            result = v2_gate_suite.fixture_integrity_gate(golden, digest, vault, min_golden=50, min_digest=50)
            self.assertFalse(result["ok"])
            self.assertIn("golden_queries", result["metrics"])
            self.assertTrue(any("min_golden_queries" in item for item in result["failures"]))

    def test_fixture_integrity_accepts_valid_minimal_case(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            vault = root / "vault"
            vault.mkdir()
            (vault / "a.md").write_text("# A\n", encoding="utf-8")
            golden = root / "golden.jsonl"
            digest = root / "digest.jsonl"
            write_jsonl(
                golden,
                [
                    {
                        "id": "q1",
                        "query": "a",
                        "expected_paths": ["a.md"],
                        "expected_facts": [{"fact_id": "f1", "canonical_claim": "A claim", "required_terms": ["A"]}],
                    }
                ],
            )
            write_jsonl(
                digest,
                [
                    {
                        "id": "d1",
                        "schema_version": "orderk.digest_fixture.v1",
                        "y_raw": [{"path": "a.md"}],
                        "expected_proposals": [
                            {
                                "type": "edge",
                                "relation": "supports",
                                "source_path": "a.md",
                                "target_path": "a.md",
                                "confidence_min": 0.5,
                                "auto_apply": False,
                            }
                        ],
                        "forbidden_writes": ["raw/"],
                        "secret_sentinels": ["SECRET_SENTINEL"],
                    }
                ],
            )
            result = v2_gate_suite.fixture_integrity_gate(
                golden,
                digest,
                vault,
                min_golden=1,
                min_digest=1,
                min_unique_golden_claims=1,
                min_unique_digest_sources=1,
                min_unique_digest_proposals=1,
            )
            self.assertTrue(result["ok"], result)
            self.assertEqual(result["state"], "pass")
            self.assertEqual(result["schema_version"], "orderk.v2.gate_result.v1")

    def test_fixture_integrity_rejects_invalid_expected_path_values(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            vault = root / "vault"
            vault.mkdir()
            (vault / "a.md").write_text("# A\n", encoding="utf-8")
            digest = root / "digest.jsonl"
            write_jsonl(
                digest,
                [
                    {
                        "id": "d1",
                        "schema_version": "orderk.digest_fixture.v1",
                        "y_raw": [{"path": "a.md"}],
                        "expected_proposals": [{"relation": "supports", "source_path": "a.md", "target_path": "a.md", "confidence_min": 0.5, "auto_apply": False}],
                        "forbidden_writes": ["raw/"],
                        "secret_sentinels": ["SECRET_SENTINEL"],
                    }
                ],
            )
            for bad_path in ["/etc/passwd", "../a.md", 123, ""]:
                golden = root / f"golden-{type(bad_path).__name__}-{str(bad_path).replace('/', '_')}.jsonl"
                write_jsonl(golden, [{"id": "q1", "query": "a", "expected_paths": [bad_path], "expected_facts": [{"canonical_claim": "A claim"}]}])
                result = v2_gate_suite.fixture_integrity_gate(
                    golden,
                    digest,
                    vault,
                    min_golden=1,
                    min_digest=1,
                    min_unique_golden_claims=1,
                    min_unique_digest_sources=1,
                    min_unique_digest_proposals=1,
                )
                self.assertFalse(result["ok"], (bad_path, result))
                self.assertTrue(any("invalid_expected_paths" in item or "nonexistent_expected_paths" in item for item in result["failures"]), result)

    def test_digest_integrity_rejects_missing_path_and_auto_apply(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            vault = root / "vault"
            vault.mkdir()
            (vault / "a.md").write_text("# A\n", encoding="utf-8")
            golden = root / "golden.jsonl"
            digest = root / "digest.jsonl"
            write_jsonl(golden, [{"id": "q1", "query": "a", "expected_paths": ["a.md"], "expected_facts": [{"canonical_claim": "A claim"}]}])
            write_jsonl(
                digest,
                [
                    {
                        "id": "d1",
                        "schema_version": "orderk.digest_fixture.v1",
                        "y_raw": [{"path": "raw/a.md"}],
                        "expected_proposals": [
                            {
                                "relation": "supports",
                                "source_path": "raw/a.md",
                                "target_path": "a.md",
                                "confidence_min": 1.5,
                                "auto_apply": True,
                            }
                        ],
                        "forbidden_writes": "raw/",
                        "secret_sentinels": [],
                    }
                ],
            )
            result = v2_gate_suite.fixture_integrity_gate(
                golden,
                digest,
                vault,
                min_golden=1,
                min_digest=1,
                min_unique_golden_claims=1,
                min_unique_digest_sources=1,
                min_unique_digest_proposals=1,
            )
            self.assertFalse(result["ok"], result)
            joined = "\n".join(result["failures"])
            self.assertIn("digest_invalid_paths", joined)
            self.assertIn("digest_auto_apply_true", joined)
            self.assertIn("digest_invalid_confidence", joined)
            self.assertIn("digest_invalid_forbidden_writes", joined)
            self.assertIn("digest_missing_secret_sentinels", joined)

    def test_digest_integrity_rejects_invalid_expected_neighbor_paths(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            vault = root / "vault"
            vault.mkdir()
            (vault / "a.md").write_text("# A\n", encoding="utf-8")
            golden = root / "golden.jsonl"
            digest = root / "digest.jsonl"
            write_jsonl(golden, [{"id": "q1", "query": "a", "expected_paths": ["a.md"], "expected_facts": [{"canonical_claim": "A claim"}]}])
            write_jsonl(
                digest,
                [
                    {
                        "id": "d1",
                        "schema_version": "orderk.digest_fixture.v1",
                        "y_raw": [{"path": "a.md"}],
                        "expected_neighbors": [{"source_path": "/etc/passwd", "target_path": "../secret.md", "min_rank": 0}],
                        "expected_proposals": [{"relation": "supports", "source_path": "a.md", "target_path": "a.md", "confidence_min": 0.5, "auto_apply": False}],
                        "forbidden_writes": ["raw/"],
                        "secret_sentinels": ["SECRET_SENTINEL"],
                    }
                ],
            )
            result = v2_gate_suite.fixture_integrity_gate(
                golden,
                digest,
                vault,
                min_golden=1,
                min_digest=1,
                min_unique_golden_claims=1,
                min_unique_digest_sources=1,
                min_unique_digest_proposals=1,
            )
            self.assertFalse(result["ok"], result)
            joined = "\n".join(result["failures"])
            self.assertIn("digest_invalid_neighbor_paths", joined)
            self.assertIn("digest_invalid_neighbor_rank", joined)

    def test_fixture_integrity_requires_unique_semantic_cases(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            vault = root / "vault"
            vault.mkdir()
            (vault / "a.md").write_text("# A\n", encoding="utf-8")
            golden = root / "golden.jsonl"
            digest = root / "digest.jsonl"
            write_jsonl(
                golden,
                [
                    {"id": f"q{i}", "query": f"a {i}", "expected_paths": ["a.md"], "expected_facts": [{"canonical_claim": "same claim"}]}
                    for i in range(50)
                ],
            )
            write_jsonl(
                digest,
                [
                    {
                        "id": f"d{i}",
                        "schema_version": "orderk.digest_fixture.v1",
                        "y_raw": [{"path": "a.md"}],
                        "expected_proposals": [{"relation": "supports", "source_path": "a.md", "target_path": "a.md", "confidence_min": 0.5, "auto_apply": False}],
                        "forbidden_writes": ["raw/"],
                        "secret_sentinels": ["SECRET_SENTINEL"],
                    }
                    for i in range(50)
                ],
            )
            result = v2_gate_suite.fixture_integrity_gate(golden, digest, vault)
            self.assertFalse(result["ok"], result)
            self.assertTrue(any("unique_normalized_golden_claims" in item for item in result["failures"]), result)
            self.assertTrue(any("unique_digest_proposals" in item for item in result["failures"]), result)

    def test_schema_contract_gate_catches_schema_rust_drift(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            schema_dir = root / "schemas"
            schema_dir.mkdir()
            bad_search = {
                "type": "object",
                "required": ["query", "mode", "reasoning_triggered", "fallback_invocation", "results", "profile", "latency_ms", "warnings"],
                "properties": {"fallback_invocation": {"enum": sorted(v2_gate_suite.ALLOWED_FALLBACK_INVOCATIONS)}},
            }
            bad_proposal = {
                "type": "object",
                "required": ["schema_version", "id", "run_id", "relation", "from", "to", "evidence", "status"],
                "properties": {"relation": {"enum": sorted(v2_gate_suite.ALLOWED_RELATIONS)}, "status": {"enum": sorted(v2_gate_suite.ALLOWED_STATUSES)}},
            }
            (schema_dir / "orderk.v2.search_result.schema.json").write_text(json.dumps(bad_search), encoding="utf-8")
            (schema_dir / "orderk.v2.proposal.schema.json").write_text(json.dumps(bad_proposal), encoding="utf-8")
            (schema_dir / "orderk.v2.digest_run.schema.json").write_text(json.dumps({"type": "object", "required": ["schema_version", "run_id", "thinking", "sidecars", "raw_unchanged"], "properties": {"schema_version": {"const": "orderk.v2.digest_run.v1"}}}), encoding="utf-8")
            result = v2_gate_suite.schema_contract_gate(schema_dir)
            self.assertFalse(result["ok"], result)
            joined = "\n".join(result["failures"])
            self.assertIn("search_required_mismatch", joined)
            self.assertIn("proposal_required_mismatch", joined)

    def test_unknown_gate_name_fails_even_when_mixed_with_known_gate(self) -> None:
        suite = v2_gate_suite.run_requested_gates({"fixture-integrity", "typo"}, DEFAULT_FOR_TEST=True)
        self.assertFalse(suite["ok"], suite)
        self.assertIn("unknown_gate", suite["claims_denied"])


if __name__ == "__main__":
    unittest.main()
