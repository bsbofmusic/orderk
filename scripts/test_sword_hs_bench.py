#!/usr/bin/env python3
from __future__ import annotations

import importlib.util
import json
import pathlib
import unittest

REPO = pathlib.Path(__file__).resolve().parents[1]
SCRIPT = REPO / "scripts" / "sword_hs_bench.py"

spec = importlib.util.spec_from_file_location("sword_hs_bench", SCRIPT)
assert spec is not None
assert spec.loader is not None
sword_hs_bench = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sword_hs_bench)


class SwordHsBenchTests(unittest.TestCase):
    def test_missing_active_llm_key_is_blocked_claim_not_pass_or_traceback(self) -> None:
        result = {
            "exit_code": 1,
            "json": {
                "error_code": "E_PROVIDER_DOWN",
                "message": "Anthropic-compatible MiniMax M3 API key is missing; set one of ORDERK_SWORD_LLM_ANTHROPIC_API_KEY or ORDERK_SWORD_LLM_MINIMAX_API_KEY or ORDERK_SWORD_LLM_API_KEY",
                "ok": False,
                "schema_version": "orderk.error.v1",
            },
            "stderr_tail": "",
        }
        key_presence = {name: False for name in sword_hs_bench.ACTIVE_LLM_KEY_NAMES}

        probe = sword_hs_bench.classify_active_probe(result, key_presence)

        self.assertEqual(probe["state"], "blocked", probe)
        self.assertFalse(probe["ok"], probe)
        self.assertEqual(probe["blocked_reason"], "missing_active_llm_key")
        self.assertEqual(probe["error_code"], "E_PROVIDER_DOWN")
        self.assertEqual(probe["required_any_key"], list(sword_hs_bench.ACTIVE_LLM_KEY_NAMES))
        serialized = json.dumps(probe, ensure_ascii=False)
        self.assertIn("ORDERK_SWORD_LLM_MINIMAX_API_KEY", serialized)
        self.assertNotIn("sk-", serialized)

    def test_active_probe_failure_with_present_key_is_fail_not_blocked(self) -> None:
        result = {
            "exit_code": 1,
            "json": {"error_code": "E_PROVIDER_DOWN", "message": "upstream timeout"},
            "stderr_tail": "",
        }
        key_presence = {name: False for name in sword_hs_bench.ACTIVE_LLM_KEY_NAMES}
        key_presence[sword_hs_bench.ACTIVE_LLM_KEY_NAMES[0]] = True

        probe = sword_hs_bench.classify_active_probe(result, key_presence)

        self.assertEqual(probe["state"], "fail", probe)
        self.assertFalse(probe["ok"], probe)
        self.assertNotIn("blocked_reason", probe)
    def test_active_exit_zero_without_key_or_llm_call_is_blocked_not_live_pass(self) -> None:
        result = {
            "exit_code": 0,
            "json": {
                "thinking": {
                    "mode": "active",
                    "llm_invocation": "not_called_no_candidates",
                    "llm_calls": 0,
                }
            },
            "stderr_tail": "",
        }
        key_presence = {name: False for name in sword_hs_bench.ACTIVE_LLM_KEY_NAMES}

        probe = sword_hs_bench.classify_active_probe(result, key_presence)

        self.assertEqual(probe["state"], "blocked", probe)
        self.assertEqual(probe["blocked_reason"], "missing_active_llm_key")
        self.assertFalse(probe["ok"], probe)

    def test_clean_rejects_bench_root_outside_orderk_tmp_prefix(self) -> None:
        with self.assertRaises(ValueError):
            sword_hs_bench.ensure_safe_bench_root(REPO)


if __name__ == "__main__":
    unittest.main()
