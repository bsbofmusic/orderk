#!/usr/bin/env python3
from __future__ import annotations

import json
import os
import pathlib
import shutil
import subprocess
import tempfile
from typing import Any

REPO = pathlib.Path(__file__).resolve().parents[1]
BASE = ["cargo", "run", "-q", "-p", "orderk-cli", "--bin", "orderk", "--"]
DEFAULT_FIXTURE_VAULT = REPO / "fixtures" / "eval" / "vault"
DEFAULT_QUERIES = REPO / "fixtures" / "eval" / "queries.json"
DEFAULT_BASELINE = REPO / "baselines" / "orderk-eval-baseline.json"


def run(args: list[str]) -> str:
    proc = subprocess.run(args, cwd=REPO, text=True, capture_output=True)
    if proc.returncode != 0:
        raise SystemExit(f"failed: {' '.join(args)}\nstdout={proc.stdout}\nstderr={proc.stderr}")
    return proc.stdout


def load_json(path: pathlib.Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def validate_fixture_paths(vault: pathlib.Path, queries: pathlib.Path, baseline: pathlib.Path) -> list[str]:
    failures: list[str] = []
    if not vault.is_dir():
        failures.append(f"eval fixture vault missing: {vault}")
    if not queries.is_file():
        failures.append(f"eval queries file missing: {queries}")
    if not baseline.is_file():
        failures.append(f"eval baseline file missing: {baseline}")
    return failures


def _number(report: dict[str, Any], key: str) -> float | None:
    value = report.get(key)
    if isinstance(value, (int, float)) and not isinstance(value, bool):
        return float(value)
    return None


def _check_min(report: dict[str, Any], baseline: dict[str, Any], report_key: str, baseline_key: str, failures: list[str]) -> None:
    if baseline_key not in baseline:
        return
    actual = _number(report, report_key)
    expected = float(baseline[baseline_key])
    if actual is None or actual < expected:
        failures.append(f"{report_key} {actual} < {baseline_key} {expected}")


def _check_max(report: dict[str, Any], baseline: dict[str, Any], report_key: str, baseline_key: str, failures: list[str]) -> None:
    if baseline_key not in baseline:
        return
    actual = _number(report, report_key)
    expected = float(baseline[baseline_key])
    if actual is None or actual > expected:
        failures.append(f"{report_key} {actual} > {baseline_key} {expected}")


def validate_eval_quality(report: dict[str, Any], baseline: dict[str, Any]) -> dict[str, Any]:
    failures: list[str] = []
    if report.get("schema_version") != "orderk.eval.v1":
        failures.append(f"unexpected eval schema_version: {report.get('schema_version')}")
    if baseline.get("schema_version") != "orderk.eval_baseline.v1":
        failures.append(f"unexpected eval baseline schema_version: {baseline.get('schema_version')}")
    if report.get("ok") is not True:
        failures.append("eval report ok is not true")

    _check_min(report, baseline, "queries", "min_queries", failures)
    _check_max(report, baseline, "zero_hit", "max_zero_hit", failures)
    _check_min(report, baseline, "hits_at_k", "min_hits_at_k", failures)
    _check_min(report, baseline, "top1_hits", "min_top1_hits", failures)
    _check_min(report, baseline, "recall_at_k", "min_recall_at_k", failures)
    _check_min(report, baseline, "ndcg_at_k", "min_ndcg_at_k", failures)
    _check_min(report, baseline, "mrr", "min_mrr", failures)
    _check_max(report, baseline, "mean_took_ms", "max_mean_took_ms", failures)

    outcomes = report.get("outcomes")
    if not isinstance(outcomes, list):
        failures.append("eval report outcomes is not a list")
        outcomes = []
    outcomes_by_id = {case.get("id"): case for case in outcomes if isinstance(case, dict)}
    required_case_ids = baseline.get("required_case_ids", [])
    if not isinstance(required_case_ids, list):
        failures.append("required_case_ids must be a list")
        required_case_ids = []
    for case_id in required_case_ids:
        if case_id not in outcomes_by_id:
            failures.append(f"missing required case id: {case_id}")

    case_thresholds = baseline.get("case_thresholds", {})
    if not isinstance(case_thresholds, dict):
        failures.append("case_thresholds must be an object")
        case_thresholds = {}
    cases_to_check = [outcomes_by_id[case_id] for case_id in required_case_ids if case_id in outcomes_by_id]
    if not cases_to_check:
        cases_to_check = [case for case in outcomes if isinstance(case, dict)]

    for case in cases_to_check:
        case_id = case.get("id", "<unknown>")
        if case_thresholds.get("require_hit") is True and case.get("hit") is not True:
            failures.append(f"case {case_id} hit is not true")
        if "max_rank" in case_thresholds:
            rank = case.get("rank")
            max_rank = int(case_thresholds["max_rank"])
            if not isinstance(rank, int) or isinstance(rank, bool) or rank > max_rank:
                failures.append(f"case {case_id} rank {rank} > max_rank {max_rank}")
        if "min_recall_at_k" in case_thresholds:
            recall = case.get("recall_at_k")
            expected = float(case_thresholds["min_recall_at_k"])
            if not isinstance(recall, (int, float)) or isinstance(recall, bool) or float(recall) < expected:
                failures.append(f"case {case_id} recall_at_k {recall} < {expected}")
        if "min_ndcg_at_k" in case_thresholds:
            ndcg = case.get("ndcg_at_k")
            expected = float(case_thresholds["min_ndcg_at_k"])
            if not isinstance(ndcg, (int, float)) or isinstance(ndcg, bool) or float(ndcg) < expected:
                failures.append(f"case {case_id} ndcg_at_k {ndcg} < {expected}")
        expected_phrases = case.get("expected_phrases", [])
        if expected_phrases:
            if not isinstance(expected_phrases, list):
                failures.append(f"case {case_id} expected_phrases must be a list")
                expected_phrases = []
            matched_expected_phrases = case.get("matched_expected_phrases", [])
            if not isinstance(matched_expected_phrases, list):
                matched_expected_phrases = []
            matched_set = {phrase for phrase in matched_expected_phrases if isinstance(phrase, str)}
            for phrase in expected_phrases:
                if not isinstance(phrase, str) or not phrase.strip():
                    failures.append(f"case {case_id} expected phrase must be a non-empty string")
                    continue
                if phrase not in matched_set:
                    failures.append(f"case {case_id} missing expected phrase: {phrase}")

    return {
        "schema_version": "orderk.eval_quality_gate.v1",
        "ok": not failures,
        "failures": failures,
        "thresholds": baseline,
        "metrics": {
            "queries": report.get("queries"),
            "hits_at_k": report.get("hits_at_k"),
            "top1_hits": report.get("top1_hits"),
            "zero_hit": report.get("zero_hit"),
            "recall_at_k": report.get("recall_at_k"),
            "ndcg_at_k": report.get("ndcg_at_k"),
            "mrr": report.get("mrr"),
            "mean_took_ms": report.get("mean_took_ms"),
        },
    }


def path_from_env(name: str, default: pathlib.Path) -> pathlib.Path:
    raw = os.environ.get(name)
    return pathlib.Path(raw) if raw else default


def main() -> None:
    fixture_vault = path_from_env("ORDERK_EVAL_VAULT", DEFAULT_FIXTURE_VAULT)
    queries = path_from_env("ORDERK_EVAL_QUERIES", DEFAULT_QUERIES)
    baseline_path = path_from_env("ORDERK_EVAL_BASELINE", DEFAULT_BASELINE)
    fixture_failures = validate_fixture_paths(fixture_vault, queries, baseline_path)
    if fixture_failures:
        print(
            json.dumps(
                {
                    "ok": False,
                    "schema_version": "orderk.eval_gate.v1",
                    "failures": fixture_failures,
                },
                ensure_ascii=False,
                indent=2,
            )
        )
        raise SystemExit(1)

    root = pathlib.Path(tempfile.mkdtemp(prefix="orderk-eval-"))
    vault = root / "vault"
    db = root / "orderk.sqlite"
    try:
        shutil.copytree(fixture_vault, vault)
        run(
            BASE
            + [
                "index",
                "--vault",
                str(vault),
                "--db",
                str(db),
                "--embedding-provider",
                "mock",
                "--embedding-dim",
                "16",
                "--embedding-model",
                "mock-16",
                "--json",
            ]
        )

        eval_out = json.loads(
            run(
                BASE
                + [
                    "eval",
                    "--db",
                    str(db),
                    "--queries",
                    str(queries),
                    "--limit",
                    "5",
                    "--embedding-provider",
                    "mock",
                    "--embedding-dim",
                    "16",
                    "--embedding-model",
                    "mock-16",
                    "--json",
                ]
            )
        )
        quality_gate = validate_eval_quality(eval_out, load_json(baseline_path))
        gate = {
            "ok": quality_gate["ok"],
            "schema_version": "orderk.eval_gate.v1",
            "fixture_vault": str(fixture_vault),
            "queries": str(queries),
            "baseline": str(baseline_path),
            "quality_gate": quality_gate,
            "eval": eval_out,
        }
        print(json.dumps(gate, ensure_ascii=False, indent=2))
        if not quality_gate["ok"]:
            raise SystemExit(1)
    finally:
        shutil.rmtree(root, ignore_errors=True)


if __name__ == "__main__":
    main()
