#!/usr/bin/env python3
from __future__ import annotations

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

    def test_resource_baseline_rejects_oversized_binary(self) -> None:
        repo = self.make_repo()
        binary = repo / "target" / "release" / "orderk"
        binary.parent.mkdir(parents=True)
        binary.write_bytes(b"x" * 32)
        baseline = {"binary_max_bytes": 16, "daemon_count_max": 0}
        result = release_gate.check_resource_baseline(repo, baseline=baseline, binary=binary, run_runtime_checks=False)
        self.assertFalse(result["ok"], result)
        self.assertIn("binary_size_bytes", result["stdout_tail"])


if __name__ == "__main__":
    unittest.main()
