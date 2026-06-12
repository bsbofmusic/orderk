#!/usr/bin/env python3
from __future__ import annotations

import ast
import importlib.util
import json
import pathlib
import tempfile
import unittest

RELEASE_GATE_MODULE_PATH = pathlib.Path(__file__).resolve().parent / "release_gate.py"
spec = importlib.util.spec_from_file_location("orderk_release_gate", RELEASE_GATE_MODULE_PATH)
assert spec and spec.loader
release_gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release_gate)


class ReleaseGateStaticChecksTest(unittest.TestCase):
    def test_mock_stress_searches_explicitly_disable_model_reranker(self) -> None:
        """Mock stress is a local resource baseline; it must not call live model rerankers."""
        stress_path = pathlib.Path(__file__).resolve().parent / "stress.py"
        tree = ast.parse(stress_path.read_text(encoding="utf-8"))
        offenders: list[str] = []
        for node in ast.walk(tree):
            if not isinstance(node, ast.Call):
                continue
            if not isinstance(node.func, ast.Name) or node.func.id != "orderk":
                continue
            literal_args = [arg.value for arg in node.args if isinstance(arg, ast.Constant) and isinstance(arg.value, str)]
            if "search" not in literal_args:
                continue
            if "--embedding-provider" in literal_args and "mock" in literal_args:
                if "--reranker" not in literal_args or "none" not in literal_args:
                    offenders.append(f"line {node.lineno}: mock search lacks --reranker none")
        self.assertEqual(offenders, [], offenders)

    def test_clippy_gate_covers_all_targets(self) -> None:
        clippy_commands = [cmd for cmd in release_gate.COMMANDS if len(cmd) > 1 and cmd[0] == "cargo" and cmd[1] == "clippy"]
        self.assertEqual(len(clippy_commands), 1)
        self.assertIn("--all-targets", clippy_commands[0])
        self.assertIn("--all-features", clippy_commands[0])

    def test_release_gate_sanitizes_production_jianling_llm_env(self) -> None:
        env = {
            "ORDERK_JIANLING_LLM_ENABLED": "1",
            "ORDERK_JIANLING_LLM_ENABLED_DEFAULT": "1",
            "ORDERK_SWORD_LLM_BASE_URL": "https://api.minimaxi.com/anthropic",
            "ORDERK_SWORD_LLM_API_KEY_ENV": "HERMES_MINIMAX_API_KEY",
            "HERMES_MINIMAX_API_KEY": "secret",
            "ORDERK_SWORD_EMBEDDING_PROVIDER": "siliconflow",
        }
        sanitized = release_gate.sanitize_release_gate_env(env)
        self.assertNotIn("ORDERK_JIANLING_LLM_ENABLED", sanitized)
        self.assertNotIn("ORDERK_JIANLING_LLM_ENABLED_DEFAULT", sanitized)
        self.assertNotIn("ORDERK_SWORD_LLM_BASE_URL", sanitized)
        self.assertNotIn("ORDERK_SWORD_LLM_API_KEY_ENV", sanitized)
        self.assertNotIn("HERMES_MINIMAX_API_KEY", sanitized)
        self.assertEqual(sanitized["ORDERK_SWORD_EMBEDDING_PROVIDER"], "siliconflow")

    def test_release_gate_runs_5topic_retrieval_non_regression_bench(self) -> None:
        self.assertIn(["python3", "scripts/sword_5topic_hs_vs_v2_bench.py"], release_gate.COMMANDS)

    def test_release_gate_runs_v3_frozen_search_eval_and_tests(self) -> None:
        unit_commands = [cmd for cmd in release_gate.COMMANDS if cmd[:3] == ["python3", "-m", "unittest"]]
        self.assertEqual(len(unit_commands), 1)
        self.assertIn("scripts/test_v3_search_eval.py", unit_commands[0])
        self.assertIn(
            [
                "python3",
                "scripts/v3_search_eval.py",
                "--fixture",
                "--qrels",
                "fixtures/eval/v3_qrels.json",
                "--baseline",
                "baselines/orderk-v3-search-baseline.json",
            ],
            release_gate.COMMANDS,
        )

    def test_quality_effect_gate_rejects_ok_bench_without_quantified_deltas(self) -> None:
        bench = {
            "ok": True,
            "schema_version": "orderk.sword_5topic_hs_vs_v2_bench.v1",
            "aggregate": {
                "orderk_base": {"top1": 5, "hit_at_3": 5, "hit_at_5": 5, "mrr_avg": 1.0, "n": 5},
                "orderk_sword": {"top1": 5, "hit_at_3": 5, "hit_at_5": 5, "mrr_avg": 1.0, "n": 5},
            },
            "failures": [],
        }
        result = release_gate.check_quality_effect_comparison(bench)
        self.assertFalse(result["ok"], result)
        self.assertIn("quality_effect missing", result["stderr_tail"])

    def test_quality_effect_gate_accepts_quantified_base_vs_sword_deltas(self) -> None:
        bench = {
            "ok": True,
            "schema_version": "orderk.sword_5topic_hs_vs_v2_bench.v1",
            "aggregate": {
                "orderk_base": {"top1": 5, "hit_at_3": 5, "hit_at_5": 5, "mrr_avg": 0.8, "n": 5},
                "orderk_sword": {"top1": 5, "hit_at_3": 5, "hit_at_5": 5, "mrr_avg": 0.9, "n": 5},
            },
            "quality_effect": {
                "comparison_type": "base_vs_sword",
                "metrics": {
                    "top1_delta": 0,
                    "hit_at_3_delta": 0,
                    "hit_at_5_delta": 0,
                    "mrr_avg_delta": 0.1,
                    "query_count": 5,
                },
                "thresholds": {
                    "min_query_count": 5,
                    "min_top1_delta": 0,
                    "min_hit_at_3_delta": 0,
                    "min_hit_at_5_delta": 0,
                    "min_mrr_avg_delta": 0.0,
                },
            },
            "failures": [],
        }
        result = release_gate.check_quality_effect_comparison(bench)
        self.assertTrue(result["ok"], result)
        self.assertIn("mrr_avg_delta", result["stdout_tail"])

    def test_npm_publish_workflow_checks_out_trigger_sha_for_workflow_run(self) -> None:
        workflow = RELEASE_GATE_MODULE_PATH.parents[1] / ".github" / "workflows" / "npm-publish.yml"
        text = workflow.read_text(encoding="utf-8")
        self.assertIn("github.event.workflow_run.head_sha", text)
        self.assertIn("github.ref", text)

    def test_npm_publish_clean_install_smoke_allows_orderk_postinstall_and_checks_exact_version(self) -> None:
        workflow = RELEASE_GATE_MODULE_PATH.parents[1] / ".github" / "workflows" / "npm-publish.yml"
        text = workflow.read_text(encoding="utf-8")
        self.assertIn("npm pkg set allowScripts.orderk-cli=true --json", text)
        self.assertIn('npm install "orderk-cli@${{ steps.version.outputs.version }}"', text)
        self.assertIn('test "$(npx orderk --version)" = "${{ steps.version.outputs.version }}"', text)

    def make_repo(self) -> pathlib.Path:
        root = pathlib.Path(tempfile.mkdtemp(prefix="orderk-release-gate-test-"))
        (root / "crates" / "orderk-cli").mkdir(parents=True)
        (root / "crates" / "orderk-core").mkdir(parents=True)
        (root / "packages" / "cli").mkdir(parents=True)
        (root / "packages" / "obsidian").mkdir(parents=True)
        (root / "Cargo.toml").write_text(
            """[workspace]\nmembers = [\"crates/orderk-core\", \"crates/orderk-cli\"]\n\n[workspace.package]\nversion = \"0.1.5\"\nedition = \"2021\"\nlicense = \"MIT\"\n""",
            encoding="utf-8",
        )
        (root / "package.json").write_text(json.dumps({"version": "0.1.5"}), encoding="utf-8")
        (root / "packages" / "cli" / "package.json").write_text(json.dumps({"version": "0.1.5"}), encoding="utf-8")
        (root / "packages" / "obsidian" / "package.json").write_text(json.dumps({"version": "0.1.5"}), encoding="utf-8")
        (root / "packages" / "obsidian" / "manifest.json").write_text(json.dumps({"version": "0.1.5"}), encoding="utf-8")
        (root / "packages" / "obsidian" / "versions.json").write_text(json.dumps({"0.1.5": "1.5.0"}), encoding="utf-8")
        return root

    def test_version_consistency_accepts_matching_versions(self) -> None:
        repo = self.make_repo()
        result = release_gate.check_version_consistency(repo)
        self.assertTrue(result["ok"], result)

    def test_version_consistency_rejects_manifest_mismatch(self) -> None:
        repo = self.make_repo()
        (repo / "packages" / "obsidian" / "manifest.json").write_text(json.dumps({"version": "9.9.9"}), encoding="utf-8")
        result = release_gate.check_version_consistency(repo)
        self.assertFalse(result["ok"], result)
        self.assertIn("packages/obsidian/manifest.json", result["stderr_tail"])

    def test_secret_scan_detects_private_token_shape(self) -> None:
        repo = self.make_repo()
        secret_file = repo / "docs" / "example.md"
        secret_file.parent.mkdir()
        fake_token = "sk-" + "1234567890abcdef1234567890abcdef"
        secret_file.write_text(f"token = \"{fake_token}\"\n", encoding="utf-8")
        result = release_gate.check_secret_scan(repo, files=[secret_file.relative_to(repo)])
        self.assertFalse(result["ok"], result)
        self.assertIn("docs/example.md", result["stdout_tail"])

    def test_package_cleanliness_rejects_runtime_artifacts(self) -> None:
        repo = self.make_repo()
        result = release_gate.check_package_cleanliness(
            repo,
            files=[
                pathlib.Path("target/release/orderk"),
                pathlib.Path("vault/orderk.sqlite"),
                pathlib.Path("packages/cli/vendor/orderk"),
                pathlib.Path("scripts/__pycache__/release_gate.cpython-312.pyc"),
            ],
        )
        self.assertFalse(result["ok"], result)
        self.assertIn("target/release/orderk", result["stdout_tail"])
        self.assertIn("vault/orderk.sqlite", result["stdout_tail"])
        self.assertIn("packages/cli/vendor/orderk", result["stdout_tail"])
        self.assertIn("scripts/__pycache__/release_gate.cpython-312.pyc", result["stdout_tail"])

    def test_package_cleanliness_allows_codegraph_local_index_but_not_other_logs(self) -> None:
        repo = self.make_repo()
        codegraph_log = repo / ".codegraph" / "daemon.log"
        codegraph_log.parent.mkdir(parents=True)
        codegraph_log.write_text("local codegraph daemon log\n", encoding="utf-8")
        app_log = repo / "orderk.log"
        app_log.write_text("runtime log\n", encoding="utf-8")
        result = release_gate.check_package_cleanliness(repo)
        self.assertFalse(result["ok"], result)
        self.assertNotIn(".codegraph/daemon.log", result["stdout_tail"])
        self.assertIn("orderk.log", result["stdout_tail"])

    def test_resource_baseline_rejects_oversized_binary(self) -> None:
        repo = self.make_repo()
        binary = repo / "target" / "release" / "orderk"
        binary.parent.mkdir(parents=True)
        binary.write_bytes(b"x" * 32)
        baseline = {"binary_max_bytes": 16, "daemon_count_max": 0}
        result = release_gate.check_resource_baseline(repo, baseline=baseline, binary=binary, run_runtime_checks=False)
        self.assertFalse(result["ok"], result)
        self.assertIn("binary_size_bytes", result["stdout_tail"])

    def test_resource_baseline_ignores_hermes_managed_orderk_mcp_but_counts_other_daemons(self) -> None:
        hermes_parent = "/opt/hermes/.venv/bin/python3 /opt/hermes/.venv/bin/hermes gateway run --replace --accept-hooks"
        orderk_mcp = "/home/agent/.local/bin/orderk mcp --db /vault/.obsidian/orderk/orderk-clean.sqlite"
        self.assertFalse(
            release_gate.is_countable_orderk_process("orderk", orderk_mcp, hermes_parent),
            "Hermes-managed MCP tool server is test harness state, not an orderk runtime daemon leak",
        )
        self.assertTrue(
            release_gate.is_countable_orderk_process("orderk", orderk_mcp, "python3 unrelated-supervisor.py"),
            "Non-Hermes orderk mcp process must still fail the daemon-count gate",
        )
        self.assertTrue(
            release_gate.is_countable_orderk_process("orderk", "/usr/local/bin/orderk serve --port 9999", hermes_parent),
            "Only Hermes-managed MCP stdio servers are ignored; other orderk daemons still count",
        )

    def test_stress_resource_baseline_rejects_latency_regression(self) -> None:
        stress_report = {
            "ok": True,
            "notes_indexed": 1000,
            "queries": 300,
            "query_ms_p50": 151.0,
            "query_ms_p95": 201.0,
            "initial_index_ms": 1900.0,
            "max_rss_mb": 9.0,
        }
        baseline = {
            "stress_min_notes_indexed": 1000,
            "stress_min_queries": 300,
            "mock_query_ms_p50_max": 150.0,
            "mock_query_ms_p95_max": 200.0,
            "mock_index_ms_max": 5000.0,
            "mock_stress_rss_max_mb": 15.0,
        }
        result = release_gate.check_stress_resource_baseline(stress_report, baseline=baseline)
        self.assertFalse(result["ok"], result)
        self.assertIn("query_ms_p50", result["stderr_tail"])
        self.assertIn("query_ms_p95", result["stderr_tail"])

    def test_stress_resource_baseline_rejects_missing_required_metrics(self) -> None:
        stress_report = {
            "ok": True,
            "notes_indexed": 1000,
            "queries": 300,
            "max_rss_mb": 9.0,
        }
        baseline = {
            "stress_min_notes_indexed": 1000,
            "stress_min_queries": 300,
            "mock_query_ms_p50_max": 150.0,
            "mock_query_ms_p95_max": 200.0,
            "mock_index_ms_max": 5000.0,
            "mock_stress_rss_max_mb": 15.0,
        }
        result = release_gate.check_stress_resource_baseline(stress_report, baseline=baseline)
        self.assertFalse(result["ok"], result)
        self.assertIn("query_ms_p50 missing", result["stderr_tail"])
        self.assertIn("query_ms_p95 missing", result["stderr_tail"])
        self.assertIn("initial_index_ms missing", result["stderr_tail"])

    def test_stress_resource_baseline_rejects_index_regression(self) -> None:
        stress_report = {
            "ok": True,
            "notes_indexed": 1000,
            "queries": 300,
            "query_ms_p50": 75.0,
            "query_ms_p95": 90.0,
            "initial_index_ms": 5001.0,
            "max_rss_mb": 9.0,
        }
        baseline = {
            "stress_min_notes_indexed": 1000,
            "stress_min_queries": 300,
            "mock_query_ms_p50_max": 150.0,
            "mock_query_ms_p95_max": 200.0,
            "mock_index_ms_max": 5000.0,
            "mock_stress_rss_max_mb": 15.0,
        }
        result = release_gate.check_stress_resource_baseline(stress_report, baseline=baseline)
        self.assertFalse(result["ok"], result)
        self.assertIn("initial_index_ms", result["stderr_tail"])

    def test_stress_resource_baseline_rejects_missing_rss_when_required(self) -> None:
        stress_report = {
            "ok": True,
            "notes_indexed": 1000,
            "queries": 300,
            "query_ms_p50": 75.0,
            "query_ms_p95": 90.0,
            "initial_index_ms": 1900.0,
        }
        baseline = {
            "stress_min_notes_indexed": 1000,
            "stress_min_queries": 300,
            "mock_query_ms_p50_max": 150.0,
            "mock_query_ms_p95_max": 200.0,
            "mock_index_ms_max": 5000.0,
            "mock_stress_rss_max_mb": 15.0,
        }
        result = release_gate.check_stress_resource_baseline(stress_report, baseline=baseline)
        self.assertFalse(result["ok"], result)
        self.assertIn("max_rss_mb missing", result["stderr_tail"])

    def test_stress_resource_baseline_rejects_rss_regression(self) -> None:
        stress_report = {
            "ok": True,
            "notes_indexed": 1000,
            "queries": 300,
            "query_ms_p50": 75.0,
            "query_ms_p95": 90.0,
            "initial_index_ms": 1900.0,
            "max_rss_mb": 16.0,
        }
        baseline = {
            "stress_min_notes_indexed": 1000,
            "stress_min_queries": 300,
            "mock_query_ms_p50_max": 150.0,
            "mock_query_ms_p95_max": 200.0,
            "mock_index_ms_max": 5000.0,
            "mock_stress_rss_max_mb": 15.0,
        }
        result = release_gate.check_stress_resource_baseline(stress_report, baseline=baseline)
        self.assertFalse(result["ok"], result)
        self.assertIn("max_rss_mb", result["stderr_tail"])

    def test_stress_resource_baseline_accepts_complete_report_within_limits(self) -> None:
        stress_report = {
            "ok": True,
            "notes_indexed": 1000,
            "queries": 300,
            "query_ms_p50": 75.0,
            "query_ms_p95": 90.0,
            "initial_index_ms": 1900.0,
            "max_rss_mb": 9.0,
        }
        baseline = {
            "stress_min_notes_indexed": 1000,
            "stress_min_queries": 300,
            "mock_query_ms_p50_max": 150.0,
            "mock_query_ms_p95_max": 200.0,
            "mock_index_ms_max": 5000.0,
            "mock_stress_rss_max_mb": 15.0,
        }
        result = release_gate.check_stress_resource_baseline(stress_report, baseline=baseline)
        self.assertTrue(result["ok"], result)


if __name__ == "__main__":
    unittest.main()
