#!/usr/bin/env python3
"""V3 recall-quality eval harness for orderk.

This script is intentionally mechanical: it scores frozen qrels against baseline
and candidate search outputs. It does not tune labels or ranking weights.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import pathlib
import shutil
import statistics
import subprocess
import tempfile
from collections import Counter
from typing import Any

REPO = pathlib.Path(__file__).resolve().parents[1]
BASE = ["cargo", "run", "-q", "-p", "orderk-cli", "--bin", "orderk", "--"]
DEFAULT_FIXTURE_VAULT = REPO / "fixtures" / "eval" / "vault"
DEFAULT_QRELS = REPO / "fixtures" / "eval" / "v3_qrels.json"
DEFAULT_BASELINE = REPO / "baselines" / "orderk-v3-search-baseline.json"


def load_json(path: pathlib.Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def file_sha256(path: pathlib.Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def tree_sha256(root: pathlib.Path) -> str:
    digest = hashlib.sha256()
    for path in sorted(item for item in root.rglob("*") if item.is_file()):
        rel = path.relative_to(root).as_posix()
        digest.update(rel.encode("utf-8"))
        digest.update(b"\0")
        digest.update(path.read_bytes())
        digest.update(b"\0")
    return digest.hexdigest()


def load_qrels(path: pathlib.Path) -> dict[str, Any]:
    data = load_json(path)
    if data.get("schema_version") != "orderk.v3_qrels.v1":
        raise ValueError(f"unsupported qrels schema_version: {data.get('schema_version')}")
    if data.get("frozen") is not True:
        raise ValueError("qrels_frozen must be true before V3 eval can run")
    queries = data.get("queries")
    if not isinstance(queries, list) or not queries:
        raise ValueError("qrels queries must be a non-empty list")
    seen: set[str] = set()
    for case in queries:
        if not isinstance(case, dict):
            raise ValueError("each qrels case must be an object")
        case_id = str(case.get("id", "")).strip()
        if not case_id:
            raise ValueError("each qrels case requires id")
        if case_id in seen:
            raise ValueError(f"duplicate qrels case id: {case_id}")
        seen.add(case_id)
        if not str(case.get("query", "")).strip():
            raise ValueError(f"qrels case {case_id} requires query")
        expected_paths = case.get("expected_paths")
        if not isinstance(expected_paths, list) or not expected_paths:
            raise ValueError(f"qrels case {case_id} requires expected_paths")
    return data


def load_baseline(path: pathlib.Path) -> dict[str, Any]:
    data = load_json(path)
    if data.get("schema_version") != "orderk.v3_search_baseline.v1":
        raise ValueError(f"unsupported baseline schema_version: {data.get('schema_version')}")
    for key in ("qrels_sha256", "fixture_sha256"):
        if not str(data.get(key, "")).strip():
            raise ValueError(f"baseline requires {key}")
    if not isinstance(data.get("baseline_outcomes"), list):
        raise ValueError("baseline requires baseline_outcomes")
    if not isinstance(data.get("baseline"), dict):
        raise ValueError("baseline requires aggregate baseline metrics")
    return data


def safe_div(num: float, den: float) -> float:
    return num / den if den else 0.0


def dcg_for_ranks(ranks: list[int]) -> float:
    return sum(1.0 / math.log2(rank + 1) for rank in ranks)


def percentile(values: list[float], pct: float) -> float:
    if not values:
        return 0.0
    ordered = sorted(values)
    if len(ordered) == 1:
        return float(ordered[0])
    # Conservative nearest-rank variant: prefer the upper bucket so release gates
    # do not under-report latency on tiny eval sets.
    idx = math.floor((pct / 100.0) * len(ordered))
    idx = min(max(idx, 0), len(ordered) - 1)
    return float(ordered[idx])


def result_sources(result: dict[str, Any]) -> list[str]:
    raw_evidence = result.get("evidence")
    evidence: dict[str, Any] = raw_evidence if isinstance(raw_evidence, dict) else {}
    sources = evidence.get("sources", [])
    return [str(source) for source in sources] if isinstance(sources, list) else []


def score_case(case: dict[str, Any], response: dict[str, Any], limit: int) -> dict[str, Any]:
    expected_paths = [str(path) for path in case.get("expected_paths", [])]
    expected_set = set(expected_paths)
    results = response.get("results", [])
    if not isinstance(results, list):
        results = []
    top_results = [result for result in results[:limit] if isinstance(result, dict)]
    paths = [str(result.get("path", "")) for result in top_results]
    matched_ranks: list[dict[str, Any]] = []
    matched_seen: set[str] = set()
    found_rank: int | None = None
    for idx, path in enumerate(paths, start=1):
        if path in expected_set and path not in matched_seen:
            matched_seen.add(path)
            matched_ranks.append({"path": path, "rank": idx})
            if found_rank is None:
                found_rank = idx
    relevant_ranks = [item["rank"] for item in matched_ranks]
    idcg = dcg_for_ranks(list(range(1, min(len(expected_set), limit) + 1)))
    ndcg = safe_div(dcg_for_ranks(relevant_ranks), idcg)
    counts = Counter(paths)
    duplicate_count = sum(count - 1 for count in counts.values() if count > 1)
    vector_only_top3 = 0
    hybrid_confirmed_top3 = 0
    reranker_evidence_at_10 = 0
    for result in top_results:
        sources = set(result_sources(result))
        if "qwen_reranker" in sources or "reranker" in sources:
            reranker_evidence_at_10 += 1
    for result in top_results[:3]:
        sources = set(result_sources(result))
        has_vector = "vector" in sources
        independent = {source for source in sources if source in {"keyword", "vector", "route", "link"}}
        if has_vector and len(independent) == 1:
            vector_only_top3 += 1
        if len(independent) >= 2:
            hybrid_confirmed_top3 += 1
    expected_phrases = [str(p) for p in case.get("expected_phrases", []) if str(p).strip()]
    matched_expected_phrases: list[str] = []
    for phrase in expected_phrases:
        needle = phrase.lower()
        if any(str(result.get("snippet", "")).lower().find(needle) >= 0 for result in top_results):
            matched_expected_phrases.append(phrase)
    took_ms = response.get("took_ms")
    if not isinstance(took_ms, (int, float)):
        raw_routing = response.get("routing")
        routing: dict[str, Any] = raw_routing if isinstance(raw_routing, dict) else {}
        raw_timings = routing.get("timings")
        timings: dict[str, Any] = raw_timings if isinstance(raw_timings, dict) else {}
        took_ms = timings.get("total_ms", 0)
    took_ms = float(took_ms or 0)
    return {
        "id": case.get("id"),
        "query": case.get("query"),
        "expected_paths": expected_paths,
        "expected_phrases": expected_phrases,
        "matched_expected_phrases": matched_expected_phrases,
        "hit_at_1": found_rank == 1,
        "hit_at_3": found_rank is not None and found_rank <= 3,
        "hit_at_10": found_rank is not None and found_rank <= 10,
        "rank": found_rank,
        "mrr_at_10": 1.0 / found_rank if found_rank is not None and found_rank <= 10 else 0.0,
        "ndcg_at_10": ndcg,
        "recall_at_10": safe_div(len(matched_seen), len(expected_set)),
        "top_path": paths[0] if paths else None,
        "result_count": len(top_results),
        "unique_files_at_10": len(counts),
        "max_chunks_per_file_at_10": max(counts.values()) if counts else 0,
        "duplicate_file_rate_at_10": safe_div(duplicate_count, len(paths)),
        "vector_only_top3": vector_only_top3,
        "hybrid_confirmed_top3": hybrid_confirmed_top3,
        "reranker_evidence_at_10": reranker_evidence_at_10,
        "took_ms": took_ms,
        "matched_ranks": matched_ranks,
    }


def aggregate(outcomes: list[dict[str, Any]], limit: int) -> dict[str, Any]:
    total = len(outcomes)
    latencies = [float(item.get("took_ms", 0) or 0) for item in outcomes]
    return {
        "queries": total,
        "limit": limit,
        "hit_at_1": sum(1 for item in outcomes if item.get("hit_at_1")),
        "hit_at_3": sum(1 for item in outcomes if item.get("hit_at_3")),
        "hit_at_10": sum(1 for item in outcomes if item.get("hit_at_10")),
        "mrr_at_10": safe_div(sum(float(item.get("mrr_at_10", 0) or 0) for item in outcomes), total),
        "ndcg_at_10": safe_div(sum(float(item.get("ndcg_at_10", 0) or 0) for item in outcomes), total),
        "recall_at_10": safe_div(sum(float(item.get("recall_at_10", 0) or 0) for item in outcomes), total),
        "duplicate_file_rate_at_10": safe_div(
            sum(float(item.get("duplicate_file_rate_at_10", 0) or 0) for item in outcomes), total
        ),
        "unique_files_at_10_avg": safe_div(sum(float(item.get("unique_files_at_10", 0) or 0) for item in outcomes), total),
        "max_chunks_per_file_at_10_max": max([int(item.get("max_chunks_per_file_at_10", 0) or 0) for item in outcomes] or [0]),
        "vector_only_top3_rate": safe_div(sum(float(item.get("vector_only_top3", 0) or 0) for item in outcomes), total * 3),
        "hybrid_confirmed_top3_rate": safe_div(
            sum(float(item.get("hybrid_confirmed_top3", 0) or 0) for item in outcomes), total * 3
        ),
        "reranker_evidence_rate_at_10": safe_div(
            sum(float(item.get("reranker_evidence_at_10", 0) or 0) for item in outcomes), total * limit
        ),
        "latency_p50_ms": percentile(latencies, 50),
        "latency_p95_ms": percentile(latencies, 95),
        "latency_mean_ms": statistics.fmean(latencies) if latencies else 0.0,
    }


def build_baseline_artifact(
    *,
    qrels_path: pathlib.Path,
    qrels_sha256: str,
    fixture_sha256: str,
    limit: int,
    baseline_args: list[str],
    queries: list[dict[str, Any]],
    raw_outputs: dict[str, Any],
    baseline_outcomes: list[dict[str, Any]],
) -> dict[str, Any]:
    baseline = aggregate(baseline_outcomes, limit)
    return {
        "schema_version": "orderk.v3_search_baseline.v1",
        "qrels_path": str(qrels_path),
        "qrels_sha256": qrels_sha256,
        "fixture_sha256": fixture_sha256,
        "limit": limit,
        "baseline_args": baseline_args,
        "queries": queries,
        "raw_outputs": raw_outputs,
        "baseline_outcomes": baseline_outcomes,
        "baseline": baseline,
        "thresholds": {
            "max_duplicate_file_rate_at_10": min(0.5, float(baseline.get("duplicate_file_rate_at_10", 0) or 0) + 1e-9),
            "min_hit_at_10_delta": 0.0,
            "min_mrr_at_10_delta": 0.0,
            "min_ndcg_at_10_delta": 0.0,
            "require_positive_effect": True,
            "require_reranker_evidence": True,
        },
    }


def aggregate_comparison(
    baseline_outcomes: list[dict[str, Any]], candidate_outcomes: list[dict[str, Any]], limit: int
) -> dict[str, Any]:
    baseline_doc = {
        "schema_version": "orderk.v3_search_baseline.v1",
        "qrels_sha256": "inline-baseline",
        "fixture_sha256": "inline-fixture",
        "baseline_outcomes": baseline_outcomes,
        "baseline": aggregate(baseline_outcomes, limit),
    }
    return aggregate_against_frozen_baseline(
        baseline_doc,
        candidate_outcomes,
        limit=limit,
        qrels_sha256="inline-baseline",
        fixture_sha256="inline-fixture",
        candidate_args=[],
    )


def aggregate_against_frozen_baseline(
    baseline_doc: dict[str, Any],
    candidate_outcomes: list[dict[str, Any]],
    *,
    limit: int,
    qrels_sha256: str,
    fixture_sha256: str,
    candidate_args: list[str],
) -> dict[str, Any]:
    if str(baseline_doc.get("qrels_sha256", "")) != qrels_sha256:
        raise ValueError("qrels_sha256 mismatch between qrels and baseline artifact")
    if str(baseline_doc.get("fixture_sha256", "")) != fixture_sha256:
        raise ValueError("fixture_sha256 mismatch between fixture vault and baseline artifact")
    baseline_outcomes = baseline_doc.get("baseline_outcomes")
    if not isinstance(baseline_outcomes, list):
        raise ValueError("baseline artifact requires baseline_outcomes")
    baseline = baseline_doc.get("baseline")
    if not isinstance(baseline, dict):
        baseline = aggregate(baseline_outcomes, limit)
    candidate = aggregate(candidate_outcomes, limit)
    numeric_keys = [key for key, value in baseline.items() if isinstance(value, (int, float)) and not isinstance(value, bool)]
    deltas = {key: float(candidate.get(key, 0) or 0) - float(baseline.get(key, 0) or 0) for key in numeric_keys}
    regressions: dict[str, list[str]] = {"mrr_at_10": [], "ndcg_at_10": [], "hit_at_10": []}
    by_candidate = {item.get("id"): item for item in candidate_outcomes}
    for base in baseline_outcomes:
        if not isinstance(base, dict):
            continue
        case_id = base.get("id")
        cand = by_candidate.get(case_id)
        if not cand:
            for key in regressions:
                regressions[key].append(str(case_id))
            continue
        for key in ("mrr_at_10", "ndcg_at_10"):
            if float(cand.get(key, 0) or 0) < float(base.get(key, 0) or 0):
                regressions[key].append(str(case_id))
        if bool(base.get("hit_at_10")) and not bool(cand.get("hit_at_10")):
            regressions["hit_at_10"].append(str(case_id))
    report = {
        "schema_version": "orderk.v3_search_eval.v1",
        "qrels_frozen": True,
        "qrels_sha256": qrels_sha256,
        "fixture_sha256": fixture_sha256,
        "baseline": baseline,
        "candidate": candidate,
        "deltas": deltas,
        "regressions": regressions,
        "baseline_outcomes": baseline_outcomes,
        "candidate_outcomes": candidate_outcomes,
        "candidate_args": candidate_args,
        "thresholds": {
            "max_duplicate_file_rate_at_10": min(0.5, float(baseline.get("duplicate_file_rate_at_10", 0) or 0) + 1e-9),
            "min_hit_at_10_delta": 0.0,
            "min_mrr_at_10_delta": 0.0,
            "min_ndcg_at_10_delta": 0.0,
            "require_positive_effect": True,
            "require_reranker_evidence": True,
        },
    }
    report["quality_gate"] = validate_quality_gate(report)
    return report


def validate_quality_gate(report: dict[str, Any]) -> dict[str, Any]:
    failures: list[str] = []
    if report.get("schema_version") != "orderk.v3_search_eval.v1":
        failures.append(f"unexpected schema_version: {report.get('schema_version')}")
    if report.get("qrels_frozen") is not True:
        failures.append("qrels_frozen is not true")
    raw_baseline = report.get("baseline")
    raw_candidate = report.get("candidate")
    raw_thresholds = report.get("thresholds")
    raw_deltas = report.get("deltas")
    baseline: dict[str, Any] = raw_baseline if isinstance(raw_baseline, dict) else {}
    candidate: dict[str, Any] = raw_candidate if isinstance(raw_candidate, dict) else {}
    thresholds: dict[str, Any] = raw_thresholds if isinstance(raw_thresholds, dict) else {}
    deltas: dict[str, Any] = raw_deltas if isinstance(raw_deltas, dict) else {}
    for key in ("hit_at_10", "mrr_at_10", "ndcg_at_10"):
        if float(candidate.get(key, 0) or 0) < float(baseline.get(key, 0) or 0):
            failures.append(f"{key} regressed: candidate {candidate.get(key)} < baseline {baseline.get(key)}")
    regressions = report.get("regressions") if isinstance(report.get("regressions"), dict) else {}
    for key in ("hit_at_10", "mrr_at_10", "ndcg_at_10"):
        cases = regressions.get(key, [])
        if isinstance(cases, list) and cases:
            failures.append(f"per_case_{key}_regressions: {cases}")
    max_dup = float(thresholds.get("max_duplicate_file_rate_at_10", 1.0))
    cand_dup = float(candidate.get("duplicate_file_rate_at_10", 0) or 0)
    if cand_dup > max_dup:
        failures.append(f"duplicate_file_rate_at_10 {cand_dup} > max {max_dup}")
    if thresholds.get("require_positive_effect") is True:
        positive_keys = ("hit_at_1", "hit_at_3", "hit_at_10", "mrr_at_10", "ndcg_at_10", "recall_at_10")
        has_positive = any(float(deltas.get(key, 0) or 0) > 0 for key in positive_keys)
        has_diversity_gain = float(deltas.get("duplicate_file_rate_at_10", 0) or 0) < 0
        has_reranker_evidence_gain = (
            thresholds.get("require_reranker_evidence") is True
            and float(deltas.get("reranker_evidence_rate_at_10", 0) or 0) > 0
            and float(candidate.get("reranker_evidence_rate_at_10", 0) or 0) > 0
        )
        if not (has_positive or has_diversity_gain or has_reranker_evidence_gain):
            failures.append("positive effect required but candidate did not improve any gated metric")
    if thresholds.get("require_reranker_evidence") is True:
        reranker_rate = float(candidate.get("reranker_evidence_rate_at_10", 0) or 0)
        if reranker_rate <= 0.0:
            failures.append("reranker evidence required but candidate results had none")
    return {
        "schema_version": "orderk.v3_search_eval_gate.v1",
        "ok": not failures,
        "failures": failures,
        "metrics": {"baseline": baseline, "candidate": candidate, "thresholds": thresholds},
    }


def run(args: list[str]) -> str:
    proc = subprocess.run(args, cwd=REPO, text=True, capture_output=True)
    if proc.returncode != 0:
        raise SystemExit(f"failed: {' '.join(args)}\nstdout={proc.stdout}\nstderr={proc.stderr}")
    return proc.stdout


def index_fixture(vault: pathlib.Path, db: pathlib.Path) -> None:
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


def search(db: pathlib.Path, query: str, limit: int, extra_args: list[str]) -> dict[str, Any]:
    return json.loads(
        run(
            BASE
            + [
                "search",
                "--db",
                str(db),
                "--query",
                query,
                "--limit",
                str(limit),
                "--embedding-provider",
                "mock",
                "--embedding-dim",
                "16",
                "--embedding-model",
                "mock-16",
                "--json",
            ]
            + extra_args
        )
    )


def run_fixture_baseline(qrels_path: pathlib.Path, limit: int, baseline_args: list[str]) -> dict[str, Any]:
    qrels = load_qrels(qrels_path)
    qrels_hash = file_sha256(qrels_path)
    fixture_hash = tree_sha256(DEFAULT_FIXTURE_VAULT)
    root = pathlib.Path(tempfile.mkdtemp(prefix="orderk-v3-eval-"))
    try:
        vault = root / "vault"
        db = root / "orderk.sqlite"
        shutil.copytree(DEFAULT_FIXTURE_VAULT, vault)
        index_fixture(vault, db)
        raw_outputs: dict[str, Any] = {}
        baseline_outcomes: list[dict[str, Any]] = []
        for case in qrels["queries"]:
            response = search(db, case["query"], limit, baseline_args)
            raw_outputs[str(case["id"])] = response
            baseline_outcomes.append(score_case(case, response, limit))
        return build_baseline_artifact(
            qrels_path=qrels_path,
            qrels_sha256=qrels_hash,
            fixture_sha256=fixture_hash,
            limit=limit,
            baseline_args=baseline_args,
            queries=qrels["queries"],
            raw_outputs=raw_outputs,
            baseline_outcomes=baseline_outcomes,
        )
    finally:
        shutil.rmtree(root, ignore_errors=True)


def run_fixture_eval(
    qrels_path: pathlib.Path, baseline_path: pathlib.Path, limit: int, candidate_args: list[str]
) -> dict[str, Any]:
    qrels = load_qrels(qrels_path)
    baseline_doc = load_baseline(baseline_path)
    qrels_hash = file_sha256(qrels_path)
    fixture_hash = tree_sha256(DEFAULT_FIXTURE_VAULT)
    root = pathlib.Path(tempfile.mkdtemp(prefix="orderk-v3-eval-"))
    try:
        vault = root / "vault"
        db = root / "orderk.sqlite"
        shutil.copytree(DEFAULT_FIXTURE_VAULT, vault)
        index_fixture(vault, db)
        candidate_outcomes: list[dict[str, Any]] = []
        for case in qrels["queries"]:
            candidate_outcomes.append(score_case(case, search(db, case["query"], limit, candidate_args), limit))
        report = aggregate_against_frozen_baseline(
            baseline_doc,
            candidate_outcomes,
            limit=limit,
            qrels_sha256=qrels_hash,
            fixture_sha256=fixture_hash,
            candidate_args=candidate_args,
        )
        report["qrels_path"] = str(qrels_path)
        report["baseline_path"] = str(baseline_path)
        return report
    finally:
        shutil.rmtree(root, ignore_errors=True)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Run orderk V3 recall eval")
    parser.add_argument("--fixture", action="store_true", help="run against the fixture eval vault")
    parser.add_argument("--qrels", type=pathlib.Path, default=DEFAULT_QRELS)
    parser.add_argument("--baseline", type=pathlib.Path, default=DEFAULT_BASELINE)
    parser.add_argument("--limit", type=int, default=10)
    parser.add_argument("--candidate-arg", action="append", default=[], help="extra orderk search arg for candidate; repeat")
    parser.add_argument("--write-baseline", type=pathlib.Path, default=None)
    parser.add_argument("--json", action="store_true")
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    if not args.fixture:
        raise SystemExit("only --fixture mode is implemented for the V3 mechanical gate")
    if args.write_baseline:
        report = run_fixture_baseline(args.qrels, args.limit, ["--reranker", "none"])
        args.write_baseline.parent.mkdir(parents=True, exist_ok=True)
        args.write_baseline.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        print(json.dumps(report, ensure_ascii=False, indent=2))
        return
    report = run_fixture_eval(args.qrels, args.baseline, args.limit, list(args.candidate_arg))
    print(json.dumps(report, ensure_ascii=False, indent=2))
    if not report["quality_gate"]["ok"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
