#!/usr/bin/env python3
from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import re
import subprocess
import time
from collections import Counter
from typing import Any

REPO = pathlib.Path(__file__).resolve().parents[1]
DEFAULT_GOLDEN = REPO / "fixtures" / "v2" / "golden_queries.jsonl"
DEFAULT_DIGEST = REPO / "fixtures" / "v2" / "digest_fixtures.jsonl"
DEFAULT_VAULT = REPO / "fixtures" / "eval" / "vault"
DEFAULT_SCHEMA_DIR = REPO / "schemas"

ALLOWED_RELATIONS = {"supports", "refines", "contradicts", "replaces", "depends_on", "part_of"}
ALLOWED_STATUSES = {"proposal", "active", "rejected", "superseded", "conflict"}
ALLOWED_FALLBACK_INVOCATIONS = {
    "not_called",
    "called",
    "called_unparseable_fallback",
    "called_failed_degraded",
    "called_timeout_degraded",
    "not_called_no_candidates",
    "not_used",
}
SUPPORTED_GATES = {"fixture-integrity", "schema-contract"}
SEARCH_REQUIRED = {
    "schema_version",
    "query",
    "mode",
    "reasoning_triggered",
    "trace_level",
    "fallback_invocation",
    "warnings",
    "metrics",
}
PROPOSAL_REQUIRED = {"schema_version", "id", "run_id", "relation", "from", "to", "confidence", "evidence", "status"}
DIGEST_REQUIRED = {"schema_version", "run_id", "thinking", "sidecars", "raw_unchanged"}


def load_jsonl(path: pathlib.Path) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    if not path.is_file():
        raise FileNotFoundError(str(path))
    for line_no, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not line.strip():
            continue
        try:
            row = json.loads(line)
        except json.JSONDecodeError as err:
            raise ValueError(f"{path}:{line_no}: invalid JSON: {err}") from err
        if not isinstance(row, dict):
            raise ValueError(f"{path}:{line_no}: row must be an object")
        rows.append(row)
    return rows


def load_json(path: pathlib.Path) -> dict[str, Any]:
    if not path.is_file():
        raise FileNotFoundError(str(path))
    data = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(data, dict):
        raise ValueError(f"{path}: JSON root must be an object")
    return data


def sha256_file(path: pathlib.Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def gate_result(
    gate_id: str,
    ok: bool,
    metrics: dict[str, Any],
    thresholds: dict[str, Any],
    failures: list[str],
    warnings: list[str] | None = None,
    state: str | None = None,
    manual_review_required: bool = False,
) -> dict[str, Any]:
    if state is None:
        state = "pass" if ok else "fail"
    return {
        "schema_version": "orderk.v2.gate_result.v1",
        "gate_id": gate_id,
        "ok": ok,
        "state": state,
        "severity": "hard",
        "unattended_safe": not manual_review_required,
        "manual_review_required": manual_review_required,
        "thresholds": thresholds,
        "metrics": metrics,
        "failures": failures,
        "warnings": warnings or [],
        "artifacts": {},
        "next_action_on_failure": "fix fixture/schema failures and rerun scripts/v2_gate_suite.py" if not ok else "none",
    }


def duplicate_values(values: list[Any]) -> list[Any]:
    return sorted([value for value, count in Counter(values).items() if value and count > 1])


def normalize_semantic_text(value: str) -> str:
    lowered = value.lower()
    lowered = re.sub(r"\b(case|scenario)\s*[-#:]?\s*\d+\b", " ", lowered)
    lowered = re.sub(r"\b\d+\b", " ", lowered)
    lowered = re.sub(r"\s+", " ", lowered).strip()
    return lowered


def validate_vault_rel_path(value: Any, vault_root: pathlib.Path, *, require_exists: bool = True) -> tuple[str | None, str | None]:
    if not isinstance(value, str):
        return None, f"not_string:{value!r}"
    rel = value.strip().replace("\\", "/")
    if not rel:
        return None, "empty"
    pure = pathlib.PurePosixPath(rel)
    if pure.is_absolute():
        return None, f"absolute:{value}"
    if any(part in {"", ".", ".."} for part in pure.parts):
        return None, f"unsafe_parts:{value}"
    root = vault_root.resolve(strict=False)
    candidate = (root / pathlib.Path(*pure.parts)).resolve(strict=False)
    try:
        candidate.relative_to(root)
    except ValueError:
        return None, f"outside_vault:{value}"
    if require_exists and not candidate.is_file():
        return rel, f"missing:{value}"
    return rel, None


def fixture_integrity_gate(
    golden_path: pathlib.Path = DEFAULT_GOLDEN,
    digest_path: pathlib.Path = DEFAULT_DIGEST,
    vault_root: pathlib.Path = DEFAULT_VAULT,
    *,
    min_golden: int = 50,
    min_digest: int = 50,
    min_unique_golden_claims: int = 50,
    min_unique_normalized_golden_claims: int | None = None,
    min_unique_digest_sources: int = 10,
    min_unique_digest_proposals: int = 30,
) -> dict[str, Any]:
    if min_unique_normalized_golden_claims is None:
        min_unique_normalized_golden_claims = min_unique_golden_claims
    failures: list[str] = []
    warnings: list[str] = []
    try:
        golden = load_jsonl(golden_path)
    except Exception as err:  # noqa: BLE001 - gate must preserve structured failure.
        golden = []
        failures.append(f"golden_load_failed: {err}")
    try:
        digest = load_jsonl(digest_path)
    except Exception as err:  # noqa: BLE001
        digest = []
        failures.append(f"digest_load_failed: {err}")

    golden_ids = [row.get("id") for row in golden]
    digest_ids = [row.get("id") for row in digest]
    duplicate_golden_ids = duplicate_values(golden_ids)
    duplicate_digest_ids = duplicate_values(digest_ids)
    invalid_expected_paths: list[str] = []
    nonexistent_expected_paths: list[str] = []
    missing_expected_paths: list[str] = []
    missing_expected_facts: list[str] = []
    queries_with_llm_allowed_without_reason: list[str] = []
    unique_claims: set[str] = set()
    unique_normalized_claims: set[str] = set()

    digest_invalid_paths: list[str] = []
    digest_missing_y_raw: list[str] = []
    digest_invalid_neighbor_paths: list[str] = []
    digest_invalid_neighbor_rank: list[str] = []
    digest_invalid_relations: list[str] = []
    digest_missing_expected: list[str] = []
    digest_auto_apply_true: list[str] = []
    digest_invalid_confidence: list[str] = []
    digest_invalid_forbidden_writes: list[str] = []
    digest_missing_secret_sentinels: list[str] = []
    digest_sources: set[str] = set()
    digest_proposal_signatures: set[tuple[str, str, str]] = set()

    for row in golden:
        case_id = str(row.get("id", "<missing>"))
        expected_paths = row.get("expected_paths") or row.get("expected") or []
        if not isinstance(expected_paths, list) or not expected_paths:
            missing_expected_paths.append(case_id)
        else:
            for rel in expected_paths:
                normalized, error = validate_vault_rel_path(rel, vault_root)
                if error:
                    if error.startswith("missing:"):
                        nonexistent_expected_paths.append(f"{case_id}:{rel}")
                    else:
                        invalid_expected_paths.append(f"{case_id}:{error}")
                elif normalized:
                    pass
        expected_facts = row.get("expected_facts") or []
        if not isinstance(expected_facts, list) or not expected_facts:
            missing_expected_facts.append(case_id)
        else:
            for fact in expected_facts:
                if isinstance(fact, dict) and isinstance(fact.get("canonical_claim"), str) and fact["canonical_claim"].strip():
                    claim = fact["canonical_claim"].strip()
                    unique_claims.add(claim)
                    normalized_claim = normalize_semantic_text(claim)
                    if normalized_claim:
                        unique_normalized_claims.add(normalized_claim)
        if row.get("llm_allowed") is True and row.get("reasoning_expected") is not True:
            queries_with_llm_allowed_without_reason.append(case_id)

    for row in digest:
        case_id = str(row.get("id", "<missing>"))
        if row.get("schema_version") != "orderk.digest_fixture.v1":
            digest_invalid_paths.append(f"{case_id}:bad_schema_version")
        y_raw = row.get("y_raw") or []
        if not isinstance(y_raw, list) or not y_raw:
            digest_missing_y_raw.append(case_id)
        else:
            for source in y_raw:
                if not isinstance(source, dict):
                    digest_invalid_paths.append(f"{case_id}:y_raw_not_object")
                    continue
                normalized, error = validate_vault_rel_path(source.get("path"), vault_root)
                if error:
                    digest_invalid_paths.append(f"{case_id}:y_raw:{error}")
                elif normalized:
                    digest_sources.add(normalized)

        expected_neighbors = row.get("expected_neighbors") or []
        if not isinstance(expected_neighbors, list):
            digest_invalid_neighbor_paths.append(f"{case_id}:expected_neighbors_not_list")
        else:
            for neighbor in expected_neighbors:
                if not isinstance(neighbor, dict):
                    digest_invalid_neighbor_paths.append(f"{case_id}:neighbor_not_object")
                    continue
                normalized_source, source_error = validate_vault_rel_path(neighbor.get("source_path"), vault_root)
                normalized_target, target_error = validate_vault_rel_path(neighbor.get("target_path"), vault_root)
                if source_error:
                    digest_invalid_neighbor_paths.append(f"{case_id}:neighbor_source:{source_error}")
                if target_error:
                    digest_invalid_neighbor_paths.append(f"{case_id}:neighbor_target:{target_error}")
                min_rank = neighbor.get("min_rank")
                if not isinstance(min_rank, int) or isinstance(min_rank, bool) or min_rank < 1 or min_rank > 1000:
                    digest_invalid_neighbor_rank.append(f"{case_id}:{min_rank!r}")
                # Keep normalized_source/target names live for readability and future metrics.
                _ = (normalized_source, normalized_target)

        expected_proposals = row.get("expected_proposals") or []
        if not isinstance(expected_proposals, list) or not expected_proposals:
            digest_missing_expected.append(case_id)
            continue
        for proposal in expected_proposals:
            if not isinstance(proposal, dict):
                digest_invalid_relations.append(f"{case_id}:proposal_not_object")
                continue
            relation = proposal.get("relation")
            if relation not in ALLOWED_RELATIONS:
                digest_invalid_relations.append(f"{case_id}:{relation}")
            normalized_source, source_error = validate_vault_rel_path(proposal.get("source_path"), vault_root)
            normalized_target, target_error = validate_vault_rel_path(proposal.get("target_path"), vault_root)
            if source_error:
                digest_invalid_paths.append(f"{case_id}:proposal_source:{source_error}")
            if target_error:
                digest_invalid_paths.append(f"{case_id}:proposal_target:{target_error}")
            if relation in ALLOWED_RELATIONS and normalized_source and normalized_target:
                digest_proposal_signatures.add((relation, normalized_source, normalized_target))
            if proposal.get("auto_apply") is not False:
                digest_auto_apply_true.append(case_id)
            confidence = proposal.get("confidence_min")
            if not isinstance(confidence, (int, float)) or isinstance(confidence, bool) or not (0.0 <= float(confidence) <= 1.0):
                digest_invalid_confidence.append(case_id)
        forbidden_writes = row.get("forbidden_writes")
        if not isinstance(forbidden_writes, list) or not forbidden_writes or not all(isinstance(item, str) and item for item in forbidden_writes):
            digest_invalid_forbidden_writes.append(case_id)
        secret_sentinels = row.get("secret_sentinels")
        if not isinstance(secret_sentinels, list) or not secret_sentinels or not all(isinstance(item, str) and item for item in secret_sentinels):
            digest_missing_secret_sentinels.append(case_id)

    if len(golden) < min_golden:
        failures.append(f"golden_queries {len(golden)} < min_golden_queries {min_golden}")
    if len(digest) < min_digest:
        failures.append(f"digest_fixtures {len(digest)} < min_digest_fixtures {min_digest}")
    if len(unique_claims) < min_unique_golden_claims:
        failures.append(f"unique_golden_claims {len(unique_claims)} < min_unique_golden_claims {min_unique_golden_claims}")
    if len(unique_normalized_claims) < min_unique_normalized_golden_claims:
        failures.append(
            f"unique_normalized_golden_claims {len(unique_normalized_claims)} < "
            f"min_unique_normalized_golden_claims {min_unique_normalized_golden_claims}"
        )
    if len(digest_sources) < min_unique_digest_sources:
        failures.append(f"unique_digest_sources {len(digest_sources)} < min_unique_digest_sources {min_unique_digest_sources}")
    if len(digest_proposal_signatures) < min_unique_digest_proposals:
        failures.append(
            f"unique_digest_proposals {len(digest_proposal_signatures)} < min_unique_digest_proposals {min_unique_digest_proposals}"
        )
    if duplicate_golden_ids:
        failures.append(f"duplicate_golden_ids: {duplicate_golden_ids}")
    if duplicate_digest_ids:
        failures.append(f"duplicate_digest_ids: {duplicate_digest_ids}")
    if invalid_expected_paths:
        failures.append(f"invalid_expected_paths: {invalid_expected_paths[:20]}")
    if nonexistent_expected_paths:
        failures.append(f"nonexistent_expected_paths: {nonexistent_expected_paths[:20]}")
    if missing_expected_paths:
        failures.append(f"missing_expected_paths: {missing_expected_paths[:20]}")
    if len(missing_expected_facts) > max(0, len(golden) // 5):
        failures.append(f"queries_missing_expected_facts {len(missing_expected_facts)} > allowed {len(golden)//5}")
    elif missing_expected_facts:
        warnings.append(f"queries_missing_expected_facts: {missing_expected_facts[:20]}")
    if queries_with_llm_allowed_without_reason:
        failures.append(f"llm_allowed_without_reasoning_expected: {queries_with_llm_allowed_without_reason[:20]}")
    if digest_missing_y_raw:
        failures.append(f"digest_missing_y_raw: {digest_missing_y_raw[:20]}")
    if digest_invalid_paths:
        failures.append(f"digest_invalid_paths: {digest_invalid_paths[:20]}")
    if digest_invalid_neighbor_paths:
        failures.append(f"digest_invalid_neighbor_paths: {digest_invalid_neighbor_paths[:20]}")
    if digest_invalid_neighbor_rank:
        failures.append(f"digest_invalid_neighbor_rank: {digest_invalid_neighbor_rank[:20]}")
    if digest_invalid_relations:
        failures.append(f"digest_invalid_relations: {digest_invalid_relations[:20]}")
    if digest_missing_expected:
        failures.append(f"digest_missing_expected_proposals: {digest_missing_expected[:20]}")
    if digest_auto_apply_true:
        failures.append(f"digest_auto_apply_true: {digest_auto_apply_true[:20]}")
    if digest_invalid_confidence:
        failures.append(f"digest_invalid_confidence: {digest_invalid_confidence[:20]}")
    if digest_invalid_forbidden_writes:
        failures.append(f"digest_invalid_forbidden_writes: {digest_invalid_forbidden_writes[:20]}")
    if digest_missing_secret_sentinels:
        failures.append(f"digest_missing_secret_sentinels: {digest_missing_secret_sentinels[:20]}")

    metrics = {
        "golden_queries": len(golden),
        "digest_fixtures": len(digest),
        "unique_golden_claims": len(unique_claims),
        "unique_normalized_golden_claims": len(unique_normalized_claims),
        "unique_digest_sources": len(digest_sources),
        "unique_digest_proposals": len(digest_proposal_signatures),
        "duplicate_golden_ids": duplicate_golden_ids,
        "duplicate_digest_ids": duplicate_digest_ids,
        "invalid_expected_paths": invalid_expected_paths,
        "missing_expected_paths": missing_expected_paths,
        "missing_expected_facts": missing_expected_facts,
        "nonexistent_expected_paths": nonexistent_expected_paths,
        "queries_with_llm_allowed_without_reason": queries_with_llm_allowed_without_reason,
        "digest_invalid_paths": digest_invalid_paths,
        "digest_invalid_neighbor_paths": digest_invalid_neighbor_paths,
        "digest_invalid_neighbor_rank": digest_invalid_neighbor_rank,
        "digest_invalid_relations": digest_invalid_relations,
        "fixture_hashes": {
            "golden_queries": sha256_file(golden_path) if golden_path.is_file() else None,
            "digest_fixtures": sha256_file(digest_path) if digest_path.is_file() else None,
        },
    }
    thresholds = {
        "min_golden_queries": min_golden,
        "min_digest_fixtures": min_digest,
        "min_unique_golden_claims": min_unique_golden_claims,
        "min_unique_normalized_golden_claims": min_unique_normalized_golden_claims,
        "min_unique_digest_sources": min_unique_digest_sources,
        "min_unique_digest_proposals": min_unique_digest_proposals,
        "duplicate_ids": 0,
        "invalid_expected_paths": 0,
        "nonexistent_expected_paths": 0,
        "allowed_missing_expected_facts_ratio": 0.2,
        "allowed_relations": sorted(ALLOWED_RELATIONS),
    }
    return gate_result("fixture_integrity", not failures, metrics, thresholds, failures, warnings)


def schema_required(schema: dict[str, Any]) -> set[str]:
    required = schema.get("required")
    return set(required) if isinstance(required, list) and all(isinstance(item, str) for item in required) else set()


def schema_enum(schema: dict[str, Any], prop: str) -> set[str]:
    enum = schema.get("properties", {}).get(prop, {}).get("enum", [])
    return set(enum) if isinstance(enum, list) and all(isinstance(item, str) for item in enum) else set()


def validate_schema_subset(instance: Any, schema: dict[str, Any], path: str = "$", *, failures: list[str] | None = None) -> list[str]:
    if failures is None:
        failures = []
    if "const" in schema and instance != schema["const"]:
        failures.append(f"{path}: expected const {schema['const']!r}, got {instance!r}")
    if "enum" in schema and instance not in schema["enum"]:
        failures.append(f"{path}: expected one of {schema['enum']!r}, got {instance!r}")
    schema_type = schema.get("type")
    if schema_type is not None:
        allowed = schema_type if isinstance(schema_type, list) else [schema_type]
        type_ok = False
        for allowed_type in allowed:
            if allowed_type == "object" and isinstance(instance, dict):
                type_ok = True
            elif allowed_type == "array" and isinstance(instance, list):
                type_ok = True
            elif allowed_type == "string" and isinstance(instance, str):
                type_ok = True
            elif allowed_type == "boolean" and isinstance(instance, bool):
                type_ok = True
            elif allowed_type == "number" and isinstance(instance, (int, float)) and not isinstance(instance, bool):
                type_ok = True
            elif allowed_type == "integer" and isinstance(instance, int) and not isinstance(instance, bool):
                type_ok = True
            elif allowed_type == "null" and instance is None:
                type_ok = True
        if not type_ok:
            failures.append(f"{path}: expected type {allowed!r}, got {type(instance).__name__}")
            return failures
    if isinstance(instance, dict):
        required = schema.get("required", [])
        if isinstance(required, list):
            for key in required:
                if key not in instance:
                    failures.append(f"{path}: missing required {key}")
        props = schema.get("properties", {})
        if isinstance(props, dict):
            for key, value in instance.items():
                prop_schema = props.get(key)
                if isinstance(prop_schema, dict):
                    validate_schema_subset(value, prop_schema, f"{path}.{key}", failures=failures)
                elif schema.get("additionalProperties") is False:
                    failures.append(f"{path}: unexpected property {key}")
    elif isinstance(instance, list):
        item_schema = schema.get("items")
        if isinstance(item_schema, dict):
            for idx, value in enumerate(instance):
                validate_schema_subset(value, item_schema, f"{path}[{idx}]", failures=failures)
    return failures


def schema_contract_gate(schema_dir: pathlib.Path = DEFAULT_SCHEMA_DIR) -> dict[str, Any]:
    failures: list[str] = []
    warnings: list[str] = []
    loaded: dict[str, dict[str, Any]] = {}
    for name in ["search_result", "proposal", "digest_run"]:
        path = schema_dir / f"orderk.v2.{name}.schema.json"
        try:
            loaded[name] = load_json(path)
        except Exception as err:  # noqa: BLE001
            failures.append(f"schema_load_failed:{name}:{err}")

    if "search_result" in loaded:
        required = schema_required(loaded["search_result"])
        if required != SEARCH_REQUIRED:
            failures.append(f"search_required_mismatch expected={sorted(SEARCH_REQUIRED)} actual={sorted(required)}")
        if schema_enum(loaded["search_result"], "fallback_invocation") != ALLOWED_FALLBACK_INVOCATIONS:
            failures.append("search_fallback_enum_mismatch")
        sample = {
            "schema_version": "orderk.v2.search_contract.v1",
            "query": "sqlite vec semantic search",
            "mode": "standard",
            "reasoning_triggered": False,
            "trace_level": "compact",
            "fallback_invocation": "not_called",
            "warnings": [],
            "metrics": {},
        }
        schema_failures = validate_schema_subset(sample, loaded["search_result"])
        if schema_failures:
            failures.append(f"search_sample_schema_invalid: {schema_failures[:5]}")

    if "proposal" in loaded:
        required = schema_required(loaded["proposal"])
        if required != PROPOSAL_REQUIRED:
            failures.append(f"proposal_required_mismatch expected={sorted(PROPOSAL_REQUIRED)} actual={sorted(required)}")
        if schema_enum(loaded["proposal"], "relation") != ALLOWED_RELATIONS:
            failures.append("proposal_relation_enum_mismatch")
        if schema_enum(loaded["proposal"], "status") != ALLOWED_STATUSES:
            failures.append("proposal_status_enum_mismatch")
        sample = {
            "schema_version": "orderk.v2.proposal.v1",
            "id": "proposal-1",
            "run_id": "run-1",
            "relation": "supports",
            "from": {"id": "raw/a.md"},
            "to": {"id": "wiki/concepts/a.md"},
            "confidence": 0.75,
            "evidence": [{"source_path": "raw/a.md", "chunk_id": "chunk-1"}],
            "status": "proposal",
        }
        schema_failures = validate_schema_subset(sample, loaded["proposal"])
        if schema_failures:
            failures.append(f"proposal_sample_schema_invalid: {schema_failures[:5]}")

    if "digest_run" in loaded:
        required = schema_required(loaded["digest_run"])
        if required != DIGEST_REQUIRED:
            failures.append(f"digest_required_mismatch expected={sorted(DIGEST_REQUIRED)} actual={sorted(required)}")
        sample = {
            "schema_version": "orderk.v2.digest_run.v1",
            "run_id": "run-1",
            "thinking": {},
            "sidecars": {},
            "raw_unchanged": True,
        }
        schema_failures = validate_schema_subset(sample, loaded["digest_run"])
        if schema_failures:
            failures.append(f"digest_sample_schema_invalid: {schema_failures[:5]}")

    metrics = {
        "schemas_checked": sorted(loaded),
        "schema_hashes": {
            name: sha256_file(schema_dir / f"orderk.v2.{name}.schema.json")
            for name in loaded
            if (schema_dir / f"orderk.v2.{name}.schema.json").is_file()
        },
    }
    thresholds = {
        "search_required": sorted(SEARCH_REQUIRED),
        "proposal_required": sorted(PROPOSAL_REQUIRED),
        "digest_required": sorted(DIGEST_REQUIRED),
        "allowed_relations": sorted(ALLOWED_RELATIONS),
        "allowed_statuses": sorted(ALLOWED_STATUSES),
        "allowed_fallback_invocations": sorted(ALLOWED_FALLBACK_INVOCATIONS),
    }
    return gate_result("schema_contract", not failures, metrics, thresholds, failures, warnings)


def repo_context() -> dict[str, Any]:
    def run(cmd: list[str]) -> str:
        proc = subprocess.run(cmd, cwd=REPO, text=True, capture_output=True, check=False)
        return proc.stdout.strip() if proc.returncode == 0 else ""

    return {
        "path": str(REPO),
        "git_sha": run(["git", "rev-parse", "HEAD"]),
        "branch": run(["git", "branch", "--show-current"]),
    }


def gate_suite(gates: list[dict[str, Any]]) -> dict[str, Any]:
    hard_failed = [g for g in gates if g.get("severity") == "hard" and not g.get("ok")]
    blocked = [g for g in gates if g.get("state") == "blocked"]
    needs_manual = [g for g in gates if g.get("state") == "needs_manual"]
    decision = "pass" if not hard_failed and not blocked and not needs_manual else "no-release"
    return {
        "schema_version": "orderk.v2.gate_suite.v1",
        "ok": decision == "pass",
        "decision": decision,
        "repo": repo_context(),
        "gates": gates,
        "hard_gates_passed": sum(1 for g in gates if g.get("severity") == "hard" and g.get("ok")),
        "hard_gates_failed": len(hard_failed),
        "blocked": len(blocked),
        "needs_manual": len(needs_manual),
        "claims_granted": [g["gate_id"] for g in gates if g.get("ok")],
        "claims_denied": [g["gate_id"] for g in gates if not g.get("ok")],
    }


def run_requested_gates(
    requested: set[str],
    *,
    golden: pathlib.Path = DEFAULT_GOLDEN,
    digest: pathlib.Path = DEFAULT_DIGEST,
    vault: pathlib.Path = DEFAULT_VAULT,
    schema_dir: pathlib.Path = DEFAULT_SCHEMA_DIR,
    DEFAULT_FOR_TEST: bool = False,
) -> dict[str, Any]:
    del DEFAULT_FOR_TEST  # Test hook: this function intentionally remains side-effect-free either way.
    normalized = {part.strip().replace("_", "-") for part in requested if part.strip()}
    gates_to_run = set(SUPPORTED_GATES) if "all" in normalized else {gate for gate in normalized if gate in SUPPORTED_GATES}
    unknown = sorted(normalized - SUPPORTED_GATES - {"all"})
    gates: list[dict[str, Any]] = []
    if unknown:
        gates.append(gate_result("unknown_gate", False, {"requested": unknown}, {"supported": sorted(SUPPORTED_GATES)}, [f"unsupported gates: {unknown}"]))
    if "fixture-integrity" in gates_to_run:
        gates.append(fixture_integrity_gate(golden, digest, vault))
    if "schema-contract" in gates_to_run:
        gates.append(schema_contract_gate(schema_dir))
    if not gates:
        gates.append(gate_result("unknown_gate", False, {}, {"supported": sorted(SUPPORTED_GATES)}, ["no supported gates requested"]))
    return gate_suite(gates)


def main() -> None:
    parser = argparse.ArgumentParser(description="orderk V2 mechanical gate suite")
    parser.add_argument("--only", default="all", help="comma-separated gate ids: fixture-integrity,schema-contract,all")
    parser.add_argument("--golden", type=pathlib.Path, default=DEFAULT_GOLDEN)
    parser.add_argument("--digest", type=pathlib.Path, default=DEFAULT_DIGEST)
    parser.add_argument("--vault", type=pathlib.Path, default=DEFAULT_VAULT)
    parser.add_argument("--schema-dir", type=pathlib.Path, default=DEFAULT_SCHEMA_DIR)
    parser.add_argument("--json", action="store_true", help="print JSON (default true; kept for CLI symmetry)")
    args = parser.parse_args()

    requested = {part for part in args.only.split(",") if part.strip()}
    started = time.time()
    suite = run_requested_gates(requested, golden=args.golden, digest=args.digest, vault=args.vault, schema_dir=args.schema_dir)
    suite["duration_ms"] = int((time.time() - started) * 1000)
    print(json.dumps(suite, ensure_ascii=False, indent=2))
    if not suite["ok"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
