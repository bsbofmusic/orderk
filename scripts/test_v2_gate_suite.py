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

    def test_raw_secret_safety_gate_rejects_literal_secret_values(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            leaked = root / "leaked.py"
            token = "sk-" + "test-secret-value-1234567890"
            leaked.write_text(f'API_KEY = "{token}"\n', encoding="utf-8")
            result = v2_gate_suite.raw_secret_safety_gate(root, [pathlib.Path("leaked.py")])
            self.assertFalse(result["ok"], result)
            self.assertTrue(any("secret_assignment" in item or "provider_token" in item for item in result["failures"]), result)

    def test_raw_secret_safety_gate_allows_explicit_test_fixture_lines(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            fixture = root / "fixture.py"
            token = "sk-" + "test-secret-value-1234567890"
            fixture.write_text(
                f'API_KEY = "{token}"  # ALLOW_RAW_SECRET_TEST_FIXTURE\n',
                encoding="utf-8",
            )
            result = v2_gate_suite.raw_secret_safety_gate(root, [pathlib.Path("fixture.py")])
            self.assertTrue(result["ok"], result)
            self.assertEqual(result["metrics"]["findings"], [])


    def test_raw_secret_safety_gate_rejects_raw_hash_markers_in_artifacts(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            artifact = root / "audit-report.json"
            artifact.write_text(
                '{"raw_sha256":"' + ("a" * 64) + '"}\n',
                encoding="utf-8",
            )
            result = v2_gate_suite.raw_secret_safety_gate(root, [pathlib.Path("audit-report.json")])
            self.assertFalse(result["ok"], result)
            self.assertGreaterEqual(result["metrics"]["artifact_files_scanned"], 1)
            self.assertEqual(result["metrics"]["raw_hash_markers"], 1)
            self.assertTrue(any("raw_hash_marker" in item for item in result["failures"]), result)

    def test_unknown_gate_name_fails_even_when_mixed_with_known_gate(self) -> None:
        suite = v2_gate_suite.run_requested_gates({"fixture-integrity", "typo"}, DEFAULT_FOR_TEST=True)
        self.assertFalse(suite["ok"], suite)
        self.assertIn("unknown_gate", suite["claims_denied"])

    def test_batch2_plan_gate_names_are_supported_without_unknown_gate(self) -> None:
        suite = v2_gate_suite.run_requested_gates({"profile", "raw-secret", "doctor"}, DEFAULT_FOR_TEST=True)
        self.assertNotIn("unknown_gate", suite["claims_denied"], suite)
        self.assertEqual(
            [gate["gate_id"] for gate in suite["gates"]],
            ["profile", "raw_secret_safety", "doctor"],
        )

    def test_batch4_plan_gate_names_are_supported_without_unknown_gate(self) -> None:
        suite = v2_gate_suite.run_requested_gates({"proposals", "raw-secret", "doctor"}, DEFAULT_FOR_TEST=True)
        self.assertNotIn("unknown_gate", suite["claims_denied"], suite)
        self.assertEqual(
            [gate["gate_id"] for gate in suite["gates"]],
            ["proposals", "raw_secret_safety", "doctor"],
        )

    def test_batch5_plan_gate_names_are_supported_without_unknown_gate(self) -> None:
        suite = v2_gate_suite.run_requested_gates(
            {"digest-fixture", "graph", "base-non-regression", "raw-secret"},
            DEFAULT_FOR_TEST=True,
        )
        self.assertNotIn("unknown_gate", suite["claims_denied"], suite)
        self.assertEqual(
            [gate["gate_id"] for gate in suite["gates"]],
            ["fixture_integrity", "graph", "base_non_regression", "raw_secret_safety"],
        )

    def test_batch6_plan_gate_names_are_supported_without_unknown_gate(self) -> None:
        suite = v2_gate_suite.run_requested_gates(
            {"reasoning", "golden-retrieval", "resource-fallback"},
            DEFAULT_FOR_TEST=True,
        )
        self.assertNotIn("unknown_gate", suite["claims_denied"], suite)
        self.assertEqual(
            [gate["gate_id"] for gate in suite["gates"]],
            ["reasoning", "golden_retrieval", "resource_fallback"],
        )

    def test_batch7_plan_gate_names_are_supported_without_unknown_gate(self) -> None:
        suite = v2_gate_suite.run_requested_gates({"adapters-cockpit", "raw-secret"}, DEFAULT_FOR_TEST=True)
        self.assertNotIn("unknown_gate", suite["claims_denied"], suite)
        self.assertEqual(
            [gate["gate_id"] for gate in suite["gates"]],
            ["adapters_cockpit", "raw_secret_safety"],
        )

    def test_gate_aliases_normalize_plan_and_cli_spellings(self) -> None:
        self.assertEqual(v2_gate_suite.normalize_gate_name("raw_secret"), "raw-secret-safety")
        self.assertEqual(v2_gate_suite.normalize_gate_name("model-profile"), "profile")
        self.assertEqual(v2_gate_suite.normalize_gate_name("doctor-status"), "doctor")
        self.assertEqual(v2_gate_suite.normalize_gate_name("proposal-governance"), "proposals")
        self.assertEqual(v2_gate_suite.normalize_gate_name("digest-fixture"), "fixture-integrity")
        self.assertEqual(v2_gate_suite.normalize_gate_name("base-nonregression"), "base-non-regression")
        self.assertEqual(v2_gate_suite.normalize_gate_name("active-reasoning"), "reasoning")
        self.assertEqual(v2_gate_suite.normalize_gate_name("golden"), "golden-retrieval")
        self.assertEqual(v2_gate_suite.normalize_gate_name("fallback"), "resource-fallback")
        self.assertEqual(v2_gate_suite.normalize_gate_name("adapters"), "adapters-cockpit")
        self.assertEqual(v2_gate_suite.normalize_gate_name("cockpit"), "adapters-cockpit")

    def test_extract_rust_pub_struct_fields_detects_reasoning_schema_drift(self) -> None:
        source = """
#[derive(Serialize)]
pub struct ReasoningReport {
    pub ok: bool,
    pub llm_allowed: bool,
    pub evidence_used: Vec<String>,
}
"""
        self.assertEqual(
            v2_gate_suite.extract_rust_pub_struct_fields(source, "ReasoningReport"),
            {"ok", "llm_allowed", "evidence_used"},
        )

    def test_rust_runtime_text_strips_cfg_test_items_only(self) -> None:
        source = """
fn runtime_before() { let _ = \"ORDERK_SWORD_LLM_API_KEY\"; }
#[cfg(test)]
fn test_helper() { let _ = \"HERMES_MINIMAX_API_KEY\"; }
fn runtime_after() { let _ = \"ORDERK_SWORD_RERANKER_API_KEY\"; }
#[cfg(test)]
const TEST_ONLY_OLD_ENV: &str = \"HINDSIGHT_API_RERANKER_PROVIDER\";
fn runtime_after_const() { let _ = \"ORDERK_SWORD_EMBEDDING_MODEL\"; }
#[cfg(test)]
mod tests { fn keeps_forbidden_only_in_tests() { let _ = \"HINDSIGHT_API_LLM_API_KEY\"; } }
"""
        runtime = v2_gate_suite.rust_runtime_text(source)
        self.assertIn("runtime_before", runtime)
        self.assertIn("runtime_after", runtime)
        self.assertIn("runtime_after_const", runtime)
        self.assertNotIn("HERMES_", runtime)
        self.assertNotIn("HINDSIGHT_API_", runtime)

    def test_profile_gate_rejects_forbidden_runtime_env_names_but_allows_tests(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            (root / "crates/orderk-core/src").mkdir(parents=True)
            (root / "crates/orderk-cli/src").mkdir(parents=True)
            (root / "crates/orderk-core/src/profiles.rs").write_text(
                "\n".join(
                    [
                        "pub enum SwordModelKind { Embedding, Reranker, Llm }",
                        "pub struct SwordModelSlot; pub struct SwordModelProfile;",
                        "fn resolve_sword_model_profile_from_env() {}",
                        "fn resolve_sword_model_slot_from_env() {}",
                        "const _: &str = \"ORDERK_SWORD_EMBEDDING_PROVIDER\";",
                        "const _: &str = \"ORDERK_SWORD_RERANKER_PROVIDER\";",
                        "const _: &str = \"ORDERK_SWORD_LLM_PROVIDER\";",
                        "fn profile_fingerprint() {}",
                        "const _: &str = \"unknown embedding provider\";",
                        "const _: &str = \"unknown reranker provider\";",
                        "const _: &str = \"unknown llm provider\";",
                        "fn slot_provider_resolves_siliconflow_embedding_with_explicit_env() {}",
                        "fn slot_provider_resolves_openai_embedding_when_provider_openai() {}",
                        "fn slot_provider_errors_on_unknown_provider() {}",
                        "fn slot_provider_default_falls_back_to_legacy_default_sword_paths() {}",
                        "fn slot_profile_ignores_non_orderk_provider_env_names() {}",
                        "fn slot_provider_independent_per_kind() {}",
                        "#[cfg(test)] fn negative_test_can_name_old_env() { let _ = \"HERMES_MINIMAX_API_KEY\"; }",
                    ]
                ),
                encoding="utf-8",
            )
            (root / "crates/orderk-core/src/sword_spirit.rs").write_text(
                "\n".join(
                    [
                        "const _: &str = \"ORDERK_SWORD_RERANKER_SILICONFLOW_API_KEY\";",
                        "const _: &str = \"ORDERK_SWORD_RERANKER_SILICONFLOW_BASE_URL\";",
                        "const _: &str = \"ORDERK_SWORD_LLM_ANTHROPIC_API_KEY\";",
                        "const _: &str = \"ORDERK_SWORD_LLM_MINIMAX_API_KEY\";",
                        "const _: &str = \"ORDERK_SWORD_LLM_ANTHROPIC_BASE_URL\";",
                        "const _: &str = \"ORDERK_SWORD_LLM_MINIMAX_BASE_URL\";",
                        "fn sword_spirit_active_clients_accept_profile_specific_orderk_key_names() {}",
                        "#[cfg(test)] const TEST_ONLY_OLD_ENV: &str = \"HERMES_SILICONFLOW_API_KEY\";",
                        'fn runtime() { let _ = "HINDSIGHT_API_LLM_PROVIDER"; }',
                    ]
                ),
                encoding="utf-8",
            )
            (root / "crates/orderk-core/src/api.rs").write_text(
                '#[cfg(test)] fn api_negative_test() { let _ = "HERMES_SILICONFLOW_API_KEY"; }\n',
                encoding="utf-8",
            )
            (root / "crates/orderk-core/src/lib.rs").write_text(
                "pub mod profiles; use profiles::resolve_sword_model_profile_from_env;\n",
                encoding="utf-8",
            )
            (root / "crates/orderk-cli/src/main.rs").write_text(
                "resolve_sword_model_profile_from_env sword_run_defaults_use_sword_model_profile_slots cli_profile_uses_sword_vendor_specific_model_dim_and_vector_backend\n",
                encoding="utf-8",
            )
            result = v2_gate_suite.profile_gate(root)
            self.assertFalse(result["ok"], result)
            self.assertIn(
                "runtime_forbidden_env_namespace:sword_spirit.rs:HINDSIGHT_API_",
                "\n".join(result["failures"]),
            )


if __name__ == "__main__":
    unittest.main()
