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
SUPPORTED_GATES = {
    "fixture-integrity",
    "schema-contract",
    "raw-secret-safety",
    "profile",
    "doctor",
    "proposals",
    "graph",
    "base-non-regression",
    "quality-effect",
    "reasoning",
    "golden-retrieval",
    "resource-fallback",
    "adapters-cockpit",
}
GATE_ALIASES = {
    "fixture": "fixture-integrity",
    "fixtures": "fixture-integrity",
    "digest-fixture": "fixture-integrity",
    "digest-fixtures": "fixture-integrity",
    "schema": "schema-contract",
    "contract": "schema-contract",
    "raw-secret": "raw-secret-safety",
    "raw-secret-scan": "raw-secret-safety",
    "raw-secrets": "raw-secret-safety",
    "profile-slots": "profile",
    "profiles": "profile",
    "model-profile": "profile",
    "model-profiles": "profile",
    "doctor-status": "doctor",
    "proposal-governance": "proposals",
    "write-allowlist": "proposals",
    "base-nonregression": "base-non-regression",
    "base-regression": "base-non-regression",
    "quality": "quality-effect",
    "quality-effect-comparison": "quality-effect",
    "scoreboard": "quality-effect",
    "effect": "quality-effect",
    "active-reasoning": "reasoning",
    "reason": "reasoning",
    "golden": "golden-retrieval",
    "retrieval": "golden-retrieval",
    "fallback": "resource-fallback",
    "resource": "resource-fallback",
    "adapter": "adapters-cockpit",
    "adapters": "adapters-cockpit",
    "cockpit": "adapters-cockpit",
}
RAW_SECRET_SCAN_EXCLUDES = {
    "Cargo.lock",
    "package-lock.json",
}
RAW_SECRET_SCAN_PREFIX_EXCLUDES = (
    ".git/",
    "target/",
    "node_modules/",
    "packages/cli/vendor/",
)
RAW_SECRET_PATTERNS = [
    ("private_key", re.compile(r"BEGIN (?:RSA|OPENSSH|EC|DSA) PRIVATE KEY")),
    ("provider_token", re.compile(r"(?:sk|ghp|github_pat|xox[baprs]?)-[A-Za-z0-9_\-]{20,}")),
    (
        "secret_assignment",
        re.compile(r"(?i)\b(api[_-]?key|secret|password|passwd|token)\b\s*[:=]\s*['\"][^'\"]{12,}['\"]"),
    ),
]
RAW_HASH_FIELD_PATTERN = re.compile(
    r'(?i)"?(?:raw|source|secret|api[_-]?key|token)[_-]?(?:sha256|hash)"?\s*[:=]\s*["\'][0-9a-f]{64}["\']'
)
ARTIFACT_SECRET_PATH_PATTERN = re.compile(r"(?i)(?:sidecar|audit|report|artifact).*\.(?:json|jsonl|md|txt|log)$")
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
REASONING_REQUIRED = {
    "ok",
    "schema_version",
    "mode",
    "query",
    "reasoning_triggered",
    "trigger_reasons",
    "llm_allowed",
    "llm_calls",
    "llm_invocation",
    "evidence_used",
    "relations_activated",
    "conclusion",
    "confidence",
    "boundary",
    "suggested_patch",
    "mutation_policy",
    "raw_unchanged",
    "warnings",
}


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


def extract_rust_pub_struct_fields(source: str, struct_name: str) -> set[str]:
    start = source.find(f"pub struct {struct_name}")
    if start < 0:
        return set()
    brace_start = source.find("{", start)
    if brace_start < 0:
        return set()
    depth = 0
    end = brace_start
    for idx in range(brace_start, len(source)):
        ch = source[idx]
        if ch == "{":
            depth += 1
        elif ch == "}":
            depth -= 1
            if depth == 0:
                end = idx
                break
    body = source[brace_start + 1 : end]
    fields: set[str] = set()
    for raw_line in body.splitlines():
        line = raw_line.strip()
        if not line.startswith("pub ") or ":" not in line:
            continue
        left = line.split(":", 1)[0]
        name = left.removeprefix("pub").strip()
        if name.startswith("r#"):
            name = name[2:]
        if name and all(part.isidentifier() for part in name.split("_")):
            fields.add(name)
    return fields


def schema_contract_gate(schema_dir: pathlib.Path = DEFAULT_SCHEMA_DIR, repo: pathlib.Path = REPO) -> dict[str, Any]:
    failures: list[str] = []
    warnings: list[str] = []
    loaded: dict[str, dict[str, Any]] = {}
    for name in ["search_result", "proposal", "digest_run", "reasoning_result"]:
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

    if "reasoning_result" in loaded:
        required = schema_required(loaded["reasoning_result"])
        runtime_fields = extract_rust_pub_struct_fields(
            read_repo_text("crates/orderk-core/src/reasoning.rs", repo),
            "ReasoningReport",
        )
        if required != REASONING_REQUIRED:
            failures.append(f"reasoning_required_mismatch expected={sorted(REASONING_REQUIRED)} actual={sorted(required)}")
        if runtime_fields != required:
            failures.append(
                "reasoning_runtime_schema_field_mismatch "
                f"runtime={sorted(runtime_fields)} schema_required={sorted(required)}"
            )
        sample = {
            "ok": True,
            "schema_version": "orderk.reasoning.result.v1",
            "mode": "evidence_only",
            "query": "judge cashflow tradeoff",
            "reasoning_triggered": True,
            "trigger_reasons": ["explicit_high_level_intent"],
            "llm_allowed": True,
            "llm_calls": 0,
            "llm_invocation": "not_called_evidence_only",
            "evidence_used": [],
            "relations_activated": [],
            "conclusion": "evidence only",
            "confidence": 0.5,
            "boundary": {
                "evidence_only": True,
                "direct_write_allowed": False,
                "raw_write_allowed": False,
                "wiki_write_allowed": False,
                "graph_write_allowed": False,
                "suggested_patch_route": "proposals",
            },
            "suggested_patch": {
                "status": "proposal_required",
                "route": "proposal_flow_only",
                "apply_allowed": False,
                "target_path": None,
                "relation": None,
                "summary": "route through proposals",
                "patch_text": None,
            },
            "mutation_policy": "no_direct_writes",
            "raw_unchanged": True,
            "warnings": [],
        }
        schema_failures = validate_schema_subset(sample, loaded["reasoning_result"])
        if schema_failures:
            failures.append(f"reasoning_sample_schema_invalid: {schema_failures[:5]}")

    metrics = {
        "schemas_checked": sorted(loaded),
        "schema_hashes": {
            name: sha256_file(schema_dir / f"orderk.v2.{name}.schema.json")
            for name in loaded
            if (schema_dir / f"orderk.v2.{name}.schema.json").is_file()
        },
        "reasoning_runtime_fields": sorted(
            extract_rust_pub_struct_fields(read_repo_text("crates/orderk-core/src/reasoning.rs", repo), "ReasoningReport")
        ),
    }
    thresholds = {
        "search_required": sorted(SEARCH_REQUIRED),
        "proposal_required": sorted(PROPOSAL_REQUIRED),
        "digest_required": sorted(DIGEST_REQUIRED),
        "reasoning_required": sorted(REASONING_REQUIRED),
        "allowed_relations": sorted(ALLOWED_RELATIONS),
        "allowed_statuses": sorted(ALLOWED_STATUSES),
        "allowed_fallback_invocations": sorted(ALLOWED_FALLBACK_INVOCATIONS),
    }
    return gate_result("schema_contract", not failures, metrics, thresholds, failures, warnings)


def list_repo_files(repo: pathlib.Path) -> list[pathlib.Path]:
    if (repo / ".git").exists():
        proc = subprocess.run(
            ["git", "ls-files", "--cached", "--others", "--exclude-standard"],
            cwd=repo,
            text=True,
            capture_output=True,
            check=False,
        )
        if proc.returncode == 0:
            return [pathlib.Path(line) for line in proc.stdout.splitlines() if line.strip()]
    return [path.relative_to(repo) for path in repo.rglob("*") if path.is_file() and ".git" not in path.parts]


def raw_secret_safety_gate(repo: pathlib.Path = REPO, files: list[pathlib.Path] | None = None) -> dict[str, Any]:
    failures: list[str] = []
    warnings: list[str] = []
    scan_files = list(files) if files is not None else list_repo_files(repo)
    scanned = 0
    artifact_files_scanned = 0
    raw_hash_markers = 0
    skipped_binary = 0
    for rel in scan_files:
        rel = pathlib.Path(rel)
        rel_text = rel.as_posix()
        if rel.name in RAW_SECRET_SCAN_EXCLUDES or rel_text.startswith(RAW_SECRET_SCAN_PREFIX_EXCLUDES):
            continue
        path = repo / rel
        if not path.is_file():
            continue
        try:
            data = path.read_bytes()
        except OSError as err:
            failures.append(f"{rel_text}:0:read_error:{err}")
            continue
        if b"\0" in data or len(data) > 2_000_000:
            skipped_binary += 1
            continue
        text = data.decode("utf-8", errors="ignore")
        scanned += 1
        artifact_like = bool(ARTIFACT_SECRET_PATH_PATTERN.search(rel_text))
        if artifact_like:
            artifact_files_scanned += 1
        for line_no, line in enumerate(text.splitlines(), start=1):
            if "ALLOW_RAW_SECRET_TEST_FIXTURE" in line:
                continue
            if RAW_HASH_FIELD_PATTERN.search(line):
                raw_hash_markers += 1
                failures.append(f"{rel_text}:{line_no}:raw_hash_marker")
            for name, pattern in RAW_SECRET_PATTERNS:
                if pattern.search(line):
                    failures.append(f"{rel_text}:{line_no}:{name}")
    metrics = {
        "files_scanned": scanned,
        "artifact_files_scanned": artifact_files_scanned,
        "raw_hash_markers": raw_hash_markers,
        "files_skipped_binary_or_large": skipped_binary,
        "findings": failures,
    }
    thresholds = {"raw_secret_findings": 0, "raw_hash_markers": 0, "max_file_bytes": 2_000_000}
    return gate_result("raw_secret_safety", not failures, metrics, thresholds, failures, warnings)


def read_repo_text(rel: str, repo: pathlib.Path = REPO) -> str:
    path = repo / rel
    try:
        return path.read_text(encoding="utf-8")
    except OSError:
        return ""


def require_markers(text: str, markers: list[str], scope: str) -> list[str]:
    return [f"{scope}:missing:{marker}" for marker in markers if marker not in text]


def rust_runtime_text(text: str) -> str:
    """Return Rust source with #[cfg(test)] annotated items removed.

    Some modules keep #[cfg(test)] helpers before later runtime impls, so a naive
    split on the first #[cfg(test)] would miss real runtime code. This scanner
    drops only the annotated item and keeps subsequent source visible to
    namespace gates. It handles both braced items (`fn`, `mod`) and semicolon
    items (`use`, `type`, `const`).
    """
    lines = text.splitlines()
    kept: list[str] = []
    i = 0
    while i < len(lines):
        line = lines[i]
        if line.strip() != "#[cfg(test)]":
            kept.append(line)
            i += 1
            continue
        i += 1
        while i < len(lines) and lines[i].strip().startswith("#"):
            i += 1
        brace_depth = 0
        saw_open = False
        while i < len(lines):
            current = lines[i]
            open_pos = current.find("{")
            semi_pos = current.find(";")
            if not saw_open and semi_pos != -1 and (open_pos == -1 or semi_pos < open_pos):
                i += 1
                break
            brace_depth += current.count("{") - current.count("}")
            if open_pos != -1:
                saw_open = True
            i += 1
            if saw_open and brace_depth <= 0:
                break
        continue
    return "\n".join(kept)


def profile_gate(repo: pathlib.Path = REPO) -> dict[str, Any]:
    failures: list[str] = []
    warnings: list[str] = []
    profiles_rs = read_repo_text("crates/orderk-core/src/profiles.rs", repo)
    sword_spirit_rs = read_repo_text("crates/orderk-core/src/sword_spirit.rs", repo)
    api_rs = read_repo_text("crates/orderk-core/src/api.rs", repo)
    lib_rs = read_repo_text("crates/orderk-core/src/lib.rs", repo)
    cli_rs = read_repo_text("crates/orderk-cli/src/main.rs", repo)
    required_profile_markers = [
        "SwordModelKind",
        "SwordModelSlot",
        "SwordModelProfile",
        "resolve_sword_model_profile_from_env",
        "resolve_sword_model_slot_from_env",
        "ORDERK_SWORD_EMBEDDING_PROVIDER",
        "ORDERK_SWORD_RERANKER_PROVIDER",
        "ORDERK_SWORD_LLM_PROVIDER",
        "profile_fingerprint",
        "unknown embedding provider",
        "unknown reranker provider",
        "unknown llm provider",
        "slot_provider_resolves_siliconflow_embedding_with_explicit_env",
        "slot_provider_resolves_openai_embedding_when_provider_openai",
        "slot_provider_errors_on_unknown_provider",
        "slot_provider_default_falls_back_to_legacy_default_sword_paths",
        "slot_profile_ignores_non_orderk_provider_env_names",
        "slot_provider_independent_per_kind",
    ]
    forbidden_runtime_env_markers = ["HERMES_", "HINDSIGHT_API_"]
    runtime_namespace_sources = {
        "profiles.rs": rust_runtime_text(profiles_rs),
        "sword_spirit.rs": rust_runtime_text(sword_spirit_rs),
        "api.rs": rust_runtime_text(api_rs),
    }
    forbidden_runtime_env_hits = [
        f"{scope}:{marker}"
        for scope, source in runtime_namespace_sources.items()
        for marker in forbidden_runtime_env_markers
        if marker in source
    ]
    required_lib_markers = ["pub mod profiles", "resolve_sword_model_profile_from_env"]
    required_sword_markers = [
        "ORDERK_SWORD_RERANKER_SILICONFLOW_API_KEY",
        "ORDERK_SWORD_RERANKER_SILICONFLOW_BASE_URL",
        "ORDERK_SWORD_LLM_ANTHROPIC_API_KEY",
        "ORDERK_SWORD_LLM_MINIMAX_API_KEY",
        "ORDERK_SWORD_LLM_ANTHROPIC_BASE_URL",
        "ORDERK_SWORD_LLM_MINIMAX_BASE_URL",
        "sword_spirit_active_clients_accept_profile_specific_orderk_key_names",
    ]
    required_cli_markers = [
        "resolve_sword_model_profile_from_env",
        "sword_run_defaults_use_sword_model_profile_slots",
        "cli_profile_uses_sword_vendor_specific_model_dim_and_vector_backend",
    ]
    failures.extend(require_markers(profiles_rs, required_profile_markers, "profiles.rs"))
    failures.extend(require_markers(sword_spirit_rs, required_sword_markers, "sword_spirit.rs"))
    failures.extend(require_markers(lib_rs, required_lib_markers, "lib.rs"))
    failures.extend(require_markers(cli_rs, required_cli_markers, "main.rs"))
    failures.extend(
        f"runtime_forbidden_env_namespace:{hit}"
        for hit in forbidden_runtime_env_hits
    )
    metrics = {
        "profiles_rs_present": bool(profiles_rs),
        "profile_markers_checked": len(required_profile_markers),
        "sword_runtime_markers_checked": len(required_sword_markers),
        "lib_markers_checked": len(required_lib_markers),
        "cli_markers_checked": len(required_cli_markers),
        "slot_test_markers": [
            marker
            for marker in required_profile_markers
            if marker.startswith("slot_provider_") or marker.startswith("slot_profile_")
        ],
        "runtime_forbidden_env_hits": forbidden_runtime_env_hits,
    }
    thresholds = {
        "missing_required_markers": 0,
        "required_slot_tests": 6,
        "runtime_forbidden_env_hits": 0,
        "required_namespaces": [
            "ORDERK_SWORD_EMBEDDING_PROVIDER",
            "ORDERK_SWORD_RERANKER_PROVIDER",
            "ORDERK_SWORD_LLM_PROVIDER",
        ],
    }
    return gate_result("profile", not failures, metrics, thresholds, failures, warnings)


def doctor_gate(repo: pathlib.Path = REPO) -> dict[str, Any]:
    failures: list[str] = []
    warnings: list[str] = []
    cli_rs = read_repo_text("crates/orderk-cli/src/main.rs", repo)
    health_rs = read_repo_text("crates/orderk-core/src/health.rs", repo)
    models_rs = read_repo_text("crates/orderk-core/src/models.rs", repo)
    required_cli_markers = [
        '"doctor"',
        "doctor_schema_version",
        "model_profile",
        "model_profile_redaction",
        "secret_values",
        "env_name_only",
        "doctor_surfaces_missing_sword_provider_key_without_secret_values",
        "doctor_reports_redacted_model_profile_without_secret_values",
        "doctor_surfaces_embedding_profile_mismatch",
        "doctor_surfaces_embedding_dim_mismatch",
    ]
    required_health_markers = [
        "ErrorCode::EProfileMismatch",
        "ErrorCode::EEmbeddingDimensionMismatch",
        "embedding_model",
        "embedding dimension mismatch",
        "profile_check",
    ]
    required_model_markers = [
        "EProfileMismatch",
        "EEmbeddingDimensionMismatch",
    ]
    failures.extend(require_markers(cli_rs, required_cli_markers, "main.rs"))
    failures.extend(require_markers(health_rs, required_health_markers, "health.rs"))
    failures.extend(require_markers(models_rs, required_model_markers, "models.rs"))
    metrics = {
        "doctor_cli_markers_checked": len(required_cli_markers),
        "doctor_health_markers_checked": len(required_health_markers),
        "doctor_model_markers_checked": len(required_model_markers),
        "redaction_contract": "env_name_only/no_secret_values",
    }
    thresholds = {
        "missing_required_markers": 0,
        "required_doctor_schema": "orderk.doctor.v1",
        "secret_values_serialized": "never",
    }
    return gate_result("doctor", not failures, metrics, thresholds, failures, warnings)


def proposals_gate(repo: pathlib.Path = REPO) -> dict[str, Any]:
    failures: list[str] = []
    warnings: list[str] = []
    cli_rs = read_repo_text("crates/orderk-cli/src/main.rs", repo)
    core_rs = read_repo_text("crates/orderk-core/src/proposals.rs", repo)
    lib_rs = read_repo_text("crates/orderk-core/src/lib.rs", repo)
    required_cli_markers = [
        '"proposals"',
        "proposals_command",
        '"list"',
        '"show"',
        '"approve"',
        '"reject"',
        "--dry-run",
        "--apply",
        "--reason",
        "approve_proposal",
        "reject_proposal",
    ]
    required_core_markers = [
        "MAX_BACKLOG",
        "duplicates_deduped",
        "audit.jsonl",
        "allowlist.json",
        "append_audit",
        "ensure_allowlisted",
        "ensure_evidence_gate",
        "target_path_in_evidence_set",
        "outside candidate evidence set",
        "proposal apply is fail-closed",
        "normalize_vault_relative_path",
        "unsafe proposal target_path",
        "refusing to read audit through symlink",
        "refusing to append audit through symlink",
        "refusing to read allowlist through symlink",
    ]
    required_lib_markers = ["pub mod proposals", "approve_proposal", "reject_proposal"]
    failures.extend(require_markers(cli_rs, required_cli_markers, "main.rs"))
    failures.extend(require_markers(core_rs, required_core_markers, "proposals.rs"))
    failures.extend(require_markers(lib_rs, required_lib_markers, "lib.rs"))

    mcp_match = re.search(r"fn mcp_tool_definitions\(\).*?(?=\nfn )", cli_rs, flags=re.S)
    if not mcp_match:
        failures.append("mcp_tool_definitions_missing")
        mcp_tool_names: set[str] = set()
    else:
        mcp_tool_names = set(re.findall(r'"name"\s*:\s*"([^"]+)"', mcp_match.group(0)))
        forbidden_mcp_write_tools = sorted(mcp_tool_names & {"proposals", "approve", "reject", "apply", "write", "patch"})
        if forbidden_mcp_write_tools:
            failures.append(f"mcp_write_tools_enabled_by_default:{forbidden_mcp_write_tools}")
    metrics = {
        "proposal_cli_markers_checked": len(required_cli_markers),
        "proposal_core_markers_checked": len(required_core_markers),
        "mcp_tool_names": sorted(mcp_tool_names),
        "mcp_write_tools_default": "disabled",
        "apply_policy": "local_allowlist_fail_closed",
        "audit_policy": "append_only_jsonl",
    }
    thresholds = {
        "missing_required_markers": 0,
        "mcp_write_tools_enabled_by_default": 0,
        "required_apply_policy": "local_allowlist_fail_closed",
    }
    return gate_result("proposals", not failures, metrics, thresholds, failures, warnings)


def graph_gate(repo: pathlib.Path = REPO) -> dict[str, Any]:
    failures: list[str] = []
    warnings: list[str] = []
    graph_rs = read_repo_text("crates/orderk-core/src/graph.rs", repo)
    digest_rs = read_repo_text("crates/orderk-core/src/digest.rs", repo)
    cli_rs = read_repo_text("crates/orderk-cli/src/main.rs", repo)
    lib_rs = read_repo_text("crates/orderk-core/src/lib.rs", repo)
    test_rs = read_repo_text("crates/orderk-core/tests/batch5_graph_digest_contract.rs", repo)
    required_graph_markers = [
        "GraphEdgeRelation",
        "Supports",
        "Refines",
        "Contradicts",
        "Replaces",
        "DependsOn",
        "PartOf",
        "GraphEdgeState",
        "Proposal",
        "Active",
        "Rejected",
        "Superseded",
        "Conflict",
        "rebuild_graph",
        "explain_graph",
        "edges.jsonl",
        "markdown_wikilink",
        "sword_sidecar",
        "relation_not_in_prd_allowlist",
        "normalize_vault_relative_path",
        "existing_vault_rel_path",
        "target_path_not_in_evidence_set",
        "ensure_plain_sidecar_dir",
        "ensure_plain_output_file",
    ]
    required_digest_markers = [
        "digest_vault",
        "DigestOptions",
        "state.json",
        "digest.lock",
        "--resume",
        "changed_paths",
        "create_digest_lock",
        "create_new(true)",
        "prepare_digest_root",
        "ensure_plain_output_file",
    ]
    required_cli_markers = [
        '"graph"',
        '"digest"',
        '"rebuild"',
        '"explain"',
        '"run"',
        "only one of --dry-run or --apply",
        "unknown graph rebuild flag",
        "unknown graph explain flag",
        "unknown digest run flag",
    ]
    required_lib_markers = ["pub mod graph", "pub mod digest", "GraphEdgeRelation", "DigestOptions"]
    required_test_markers = [
        "graph_rebuild_accepts_only_prd_relations",
        "graph_rebuild_applies_audit_states",
        "graph_rejects_non_prd_sidecar_relation_unsafe_paths_and_missing_evidence_overlap",
        "graph_and_digest_apply_reject_symlinked_sidecar_paths",
        "digest_dry_run_detects_changes_apply_records_state",
    ]
    failures.extend(require_markers(graph_rs, required_graph_markers, "graph.rs"))
    failures.extend(require_markers(digest_rs, required_digest_markers, "digest.rs"))
    failures.extend(require_markers(cli_rs, required_cli_markers, "main.rs"))
    failures.extend(require_markers(lib_rs, required_lib_markers, "lib.rs"))
    failures.extend(require_markers(test_rs, required_test_markers, "batch5_graph_digest_contract.rs"))
    if "made_up_relation" not in test_rs:
        failures.append("graph_contract_missing_invalid_relation_fixture")
    if '"supports" | "wikilink"' in graph_rs or '"wikilink" => Some' in graph_rs:
        failures.append("graph_sidecar_relation_parser_allows_wikilink_alias")
    metrics = {
        "graph_markers_checked": len(required_graph_markers),
        "digest_markers_checked": len(required_digest_markers),
        "cli_markers_checked": len(required_cli_markers),
        "allowed_relations": sorted(ALLOWED_RELATIONS),
        "allowed_states": sorted(ALLOWED_STATUSES),
        "store_policy": ".orderk/graph only; raw markdown untouched",
        "digest_policy": "dry-run no state write; apply writes .orderk/digest state with lock/resume",
    }
    thresholds = {
        "missing_required_markers": 0,
        "allowed_relation_count": 6,
        "allowed_state_count": 5,
    }
    return gate_result("graph", not failures, metrics, thresholds, failures, warnings)


def resource_fallback_gate(repo: pathlib.Path = REPO) -> dict[str, Any]:
    failures: list[str] = []
    warnings: list[str] = []
    reasoning_rs = read_repo_text("crates/orderk-core/src/reasoning.rs", repo)
    bench_py = read_repo_text("scripts/sword_hs_bench.py", repo)
    test_py = read_repo_text("scripts/test_sword_hs_bench.py", repo)
    required = [
        "not_called_no_trigger",
        "not_called_evidence_only",
        "llm_calls: 0",
        "missing_active_llm_key",
        "live_active_sword_llm_probe",
        "claims_denied",
    ]
    combined = "\n".join([reasoning_rs, bench_py, test_py])
    failures.extend(require_markers(combined, required, "reasoning/resource fallback"))
    return gate_result(
        "resource_fallback",
        not failures,
        {
            "reasoning_has_zero_llm_path": "llm_calls: 0" in reasoning_rs,
            "active_probe_missing_key_blocked": "missing_active_llm_key" in bench_py,
            "active_probe_live_guard": "claims_denied" in bench_py,
        },
        {"expected_no_trigger_llm_calls": 0, "active_missing_key_state": "blocked"},
        failures,
        warnings,
    )


def adapters_cockpit_gate(repo: pathlib.Path = REPO) -> dict[str, Any]:
    failures: list[str] = []
    warnings: list[str] = []
    mod_rs = read_repo_text("crates/orderk-core/src/adapters/mod.rs", repo)
    obsidian_rs = read_repo_text("crates/orderk-core/src/adapters/obsidian.rs", repo)
    cli_rs = read_repo_text("crates/orderk-cli/src/main.rs", repo)
    batch7_test = read_repo_text("crates/orderk-core/tests/batch7_adapter_contract.rs", repo)
    required_adapter_markers = [
        "scan_obsidian_adapter",
        "AdapterScanReport",
        "write_capability",
        "raw_write_performed",
        "extract_obsidian_attachments",
        "record_markdown_symlink_warnings",
    ]
    failures.extend(require_markers(mod_rs + "\n" + obsidian_rs, required_adapter_markers, "adapters"))
    required_mcp_markers = [
        '"get_source"',
        '"explain_result"',
        '"graph_neighbors"',
        '"list_concepts"',
        '"list_tags"',
        '"doctor"',
        '"ingest_raw"',
        '"run_digest"',
        '"approve_proposal"',
        "disabled_mcp_write_tool_definition",
        "mcp_disabled_write_tool",
        "remote self-authorization is not supported",
        "normalize_mcp_vault_path",
    ]
    failures.extend(require_markers(cli_rs, required_mcp_markers, "main.rs MCP adapters"))
    required_tests = [
        "obsidian_adapter_reads_markdown_frontmatter_wikilinks_and_attachment_metadata_only",
        "obsidian_adapter_rejects_symlinked_markdown_and_stays_inside_vault",
        "mcp_adapter_read_tools_return_source_concepts_tags_without_writes",
        "mcp_write_tool_stubs_reject_remote_self_authorization",
    ]
    failures.extend(require_markers(cli_rs + "\n" + batch7_test, required_tests, "Batch 7 tests"))
    direct_adapter_writes = re.findall(r"\bfs::write\b|\bFile::create\b|\bOpenOptions\b", rust_runtime_text(obsidian_rs))
    if direct_adapter_writes:
        failures.append(f"adapter_runtime_write_markers_present: {sorted(set(direct_adapter_writes))}")
    if "Tauri" in mod_rs + obsidian_rs:
        failures.append("adapter core must not introduce cockpit/editor implementation")
    return gate_result(
        "adapters_cockpit",
        not failures,
        {
            "adapter_markers_checked": len(required_adapter_markers),
            "mcp_markers_checked": len(required_mcp_markers),
            "batch7_test_markers_checked": len(required_tests),
            "adapter_runtime_write_markers": sorted(set(direct_adapter_writes)),
            "cockpit_scope": "search/source preview/graph/proposals/status only; no editor clone",
        },
        {"adapter_runtime_write_markers": 0, "write_tools_default": "disabled"},
        failures,
        warnings,
    )


def base_non_regression_gate(repo: pathlib.Path = REPO) -> dict[str, Any]:
    failures: list[str] = []
    warnings: list[str] = []
    graph_rs = read_repo_text("crates/orderk-core/src/graph.rs", repo)
    cli_rs = read_repo_text("crates/orderk-cli/src/main.rs", repo)
    release_gate_py = read_repo_text("scripts/release_gate.py", repo)
    contract_rs = read_repo_text("crates/orderk-core/tests/batch5_graph_digest_contract.rs", repo)
    required_markers = [
        "bounded_graph_boost",
        "0.03",
        "base_score >= 0.95",
        "graph_boost_is_bounded_and_cannot_demote_base_top",
    ]
    failures.extend(require_markers(graph_rs + contract_rs, required_markers, "graph non-regression contract"))
    if "sword_5topic_hs_vs_v2_bench.py" not in release_gate_py:
        failures.append("release_gate_missing_5topic_retrieval_non_regression")
    if "original_top_chunk_id" not in cli_rs:
        failures.append("sword_sidecar_base_top_guard_missing")
    metrics = {
        "graph_boost_cap": 0.03,
        "high_confidence_base_score_boost": 0.0,
        "release_gate_has_5topic_bench": "sword_5topic_hs_vs_v2_bench.py" in release_gate_py,
        "sword_sidecar_base_top_guard": "original_top_chunk_id" in cli_rs,
    }
    thresholds = {"max_graph_boost": 0.03, "base_top_regression_allowed": 0}
    return gate_result("base_non_regression", not failures, metrics, thresholds, failures, warnings)


def quality_effect_gate(repo: pathlib.Path = REPO) -> dict[str, Any]:
    """Static guard that release closure includes a quantified scoreboard.

    This does not replace the live release-gate benchmark; it prevents the V2 gate
    suite from accepting a PRD/release loop that only proves commands are green.
    """
    failures: list[str] = []
    warnings: list[str] = []
    release_gate_py = read_repo_text("scripts/release_gate.py", repo)
    bench_py = read_repo_text("scripts/sword_5topic_hs_vs_v2_bench.py", repo)
    required_release_markers = [
        "check_quality_effect_comparison",
        "quality_effect_comparison",
        "top1_delta",
        "hit_at_3_delta",
        "hit_at_5_delta",
        "mrr_avg_delta",
        "min_mrr_avg_delta",
    ]
    required_bench_markers = [
        "quality_effect",
        "base_vs_sword",
        "top1_delta",
        "hit_at_3_delta",
        "hit_at_5_delta",
        "mrr_avg_delta",
        "base_top1",
        "sword_top1",
    ]
    failures.extend(require_markers(release_gate_py, required_release_markers, "release_gate quality effect"))
    failures.extend(require_markers(bench_py, required_bench_markers, "5-topic bench quality effect"))
    if "check_quality_effect_comparison(bench_report)" not in release_gate_py:
        failures.append("release_gate_does_not_validate_5topic_quality_effect")
    metrics = {
        "release_gate_validates_quality_effect": "check_quality_effect_comparison(bench_report)" in release_gate_py,
        "bench_reports_quality_effect": "summary[\"quality_effect\"]" in bench_py,
        "required_metric_names": ["top1_delta", "hit_at_3_delta", "hit_at_5_delta", "mrr_avg_delta"],
        "completion_rule": "green gates without quantified baseline-vs-new deltas are not closed",
    }
    thresholds = {
        "min_query_count": 5,
        "min_top1_delta": 0,
        "min_hit_at_3_delta": 0,
        "min_hit_at_5_delta": 0,
        "min_mrr_avg_delta": 0.0,
    }
    return gate_result("quality_effect", not failures, metrics, thresholds, failures, warnings)


def reasoning_fixture_rows(repo: pathlib.Path = REPO) -> list[dict[str, Any]]:
    root = repo / "fixtures" / "golden_queries" / "reasoning"
    rows: list[dict[str, Any]] = []
    if not root.is_dir():
        return rows
    for path in sorted(root.glob("*.jsonl")):
        rows.extend(load_jsonl(path))
    return rows


def reasoning_gate(repo: pathlib.Path = REPO) -> dict[str, Any]:
    failures: list[str] = []
    warnings: list[str] = []
    reasoning_rs = read_repo_text("crates/orderk-core/src/reasoning.rs", repo)
    lib_rs = read_repo_text("crates/orderk-core/src/lib.rs", repo)
    cli_rs = read_repo_text("crates/orderk-cli/src/main.rs", repo)
    core_test = read_repo_text("crates/orderk-core/tests/batch6_reasoning_contract.rs", repo)
    fixture_rows = reasoning_fixture_rows(repo)
    required_markers = [
        "reason_about_vault",
        "ReasoningReport",
        "evidence_used",
        "relations_activated",
        "suggested_patch",
        "direct_write_allowed: false",
        "raw_unchanged: true",
        "llm_calls: 0",
        "not_called_no_trigger",
        "not_called_evidence_only",
        "no_direct_writes",
        "proposal_flow_only",
        "sanitize_excerpt",
    ]
    failures.extend(require_markers(reasoning_rs, required_markers, "reasoning.rs"))
    failures.extend(require_markers(lib_rs, ["pub mod reasoning", "reason_about_vault"], "lib.rs"))
    failures.extend(require_markers(cli_rs, ["reason_command", '"reason"', "unknown reason flag", "--confidence must be between 0 and 1", "take_optional_f32"], "main.rs"))
    failures.extend(
        require_markers(
            core_test,
            [
                "reasoning_no_trigger_keeps_llm_zero_and_does_not_write",
                "reasoning_trigger_outputs_evidence_only_proposal_patch_without_mutating_vault",
                "reasoning_rejects_unsafe_context_paths",
                "reasoning_evidence_excerpts_redact_common_secret_shapes",
            ],
            "batch6_reasoning_contract.rs",
        )
    )
    runtime_text = rust_runtime_text(reasoning_rs)
    forbidden_write_markers = ["fs::write", "OpenOptions", "create_dir_all", "File::create"]
    for marker in forbidden_write_markers:
        if marker in runtime_text:
            failures.append(f"reasoning_runtime_has_direct_write_marker:{marker}")
    if not fixture_rows:
        failures.append("reasoning_fixtures_missing")
    invalid_fixture_rows: list[str] = []
    llm_nonzero_rows: list[str] = []
    llm_allowed_without_reason: list[str] = []
    missing_fields: list[str] = []
    for row in fixture_rows:
        row_id = str(row.get("id", "<missing>"))
        if row.get("schema_version") != "orderk.reasoning_fixture.v1":
            invalid_fixture_rows.append(f"{row_id}:bad_schema")
        if row.get("llm_allowed") is True and row.get("reasoning_expected") is not True:
            llm_allowed_without_reason.append(row_id)
        if row.get("expected_llm_calls") != 0:
            llm_nonzero_rows.append(row_id)
        expected_fields = row.get("expected_fields") or []
        for field in ["evidence_used", "relations_activated", "conclusion", "confidence", "boundary", "suggested_patch"]:
            if field not in expected_fields:
                missing_fields.append(f"{row_id}:{field}")
        forbidden_writes = row.get("forbidden_writes") or []
        if not forbidden_writes:
            invalid_fixture_rows.append(f"{row_id}:missing_forbidden_writes")
        for path in row.get("context_paths") or []:
            _normalized, error = validate_vault_rel_path(path, repo / "fixtures" / "eval" / "vault")
            if error:
                invalid_fixture_rows.append(f"{row_id}:context_path:{error}")
    if invalid_fixture_rows:
        failures.append(f"invalid_reasoning_fixtures: {invalid_fixture_rows[:20]}")
    if llm_nonzero_rows:
        failures.append(f"reasoning_fixture_expected_llm_calls_nonzero: {llm_nonzero_rows[:20]}")
    if llm_allowed_without_reason:
        failures.append(f"reasoning_llm_allowed_without_trigger: {llm_allowed_without_reason[:20]}")
    if missing_fields:
        failures.append(f"reasoning_fixture_missing_expected_fields: {missing_fields[:20]}")
    metrics = {
        "reasoning_fixture_rows": len(fixture_rows),
        "runtime_direct_write_markers": [marker for marker in forbidden_write_markers if marker in runtime_text],
        "triggered_fixture_rows": sum(1 for row in fixture_rows if row.get("reasoning_expected") is True),
        "no_trigger_fixture_rows": sum(1 for row in fixture_rows if row.get("reasoning_expected") is False),
    }
    thresholds = {"min_reasoning_fixture_rows": 2, "max_runtime_direct_write_markers": 0, "expected_llm_calls": 0}
    if len(fixture_rows) < 2:
        failures.append(f"reasoning_fixture_rows {len(fixture_rows)} < 2")
    return gate_result("reasoning", not failures, metrics, thresholds, failures, warnings)


def golden_retrieval_gate(golden: pathlib.Path = DEFAULT_GOLDEN, digest: pathlib.Path = DEFAULT_DIGEST, vault: pathlib.Path = DEFAULT_VAULT) -> dict[str, Any]:
    base = fixture_integrity_gate(golden, digest, vault)
    failures = list(base.get("failures", []))
    metrics = dict(base.get("metrics", {}))
    thresholds = dict(base.get("thresholds", {}))
    metrics["source_gate"] = "fixture_integrity"
    return gate_result("golden_retrieval", not failures, metrics, thresholds, failures, base.get("warnings", []))


def normalize_gate_name(name: str) -> str:
    normalized = name.strip().replace("_", "-")
    return GATE_ALIASES.get(normalized, normalized)


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
    normalized = {normalize_gate_name(part) for part in requested if part.strip()}
    gates_to_run = set(SUPPORTED_GATES) if "all" in normalized else {gate for gate in normalized if gate in SUPPORTED_GATES}
    unknown = sorted(normalized - SUPPORTED_GATES - {"all"})
    gates: list[dict[str, Any]] = []
    if unknown:
        gates.append(gate_result("unknown_gate", False, {"requested": unknown}, {"supported": sorted(SUPPORTED_GATES)}, [f"unsupported gates: {unknown}"]))
    gate_order = [
        "fixture-integrity",
        "schema-contract",
        "profile",
        "proposals",
        "graph",
        "reasoning",
        "golden-retrieval",
        "resource-fallback",
        "adapters-cockpit",
        "base-non-regression",
        "quality-effect",
        "raw-secret-safety",
        "doctor",
    ]
    for gate_id in gate_order:
        if gate_id not in gates_to_run:
            continue
        if gate_id == "fixture-integrity":
            gates.append(fixture_integrity_gate(golden, digest, vault))
        elif gate_id == "schema-contract":
            gates.append(schema_contract_gate(schema_dir, REPO))
        elif gate_id == "profile":
            gates.append(profile_gate(REPO))
        elif gate_id == "proposals":
            gates.append(proposals_gate(REPO))
        elif gate_id == "graph":
            gates.append(graph_gate(REPO))
        elif gate_id == "reasoning":
            gates.append(reasoning_gate(REPO))
        elif gate_id == "golden-retrieval":
            gates.append(golden_retrieval_gate(golden, digest, vault))
        elif gate_id == "resource-fallback":
            gates.append(resource_fallback_gate(REPO))
        elif gate_id == "adapters-cockpit":
            gates.append(adapters_cockpit_gate(REPO))
        elif gate_id == "base-non-regression":
            gates.append(base_non_regression_gate(REPO))
        elif gate_id == "quality-effect":
            gates.append(quality_effect_gate(REPO))
        elif gate_id == "raw-secret-safety":
            gates.append(raw_secret_safety_gate(REPO))
        elif gate_id == "doctor":
            gates.append(doctor_gate(REPO))
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
