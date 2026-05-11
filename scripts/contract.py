#!/usr/bin/env python3
import json
import pathlib
import shutil
import subprocess
import tempfile

REPO = pathlib.Path(__file__).resolve().parents[1]
BASE = ["cargo", "run", "-q", "-p", "orderk-cli", "--bin", "orderk", "--"]


def run(args):
    proc = subprocess.run(args, cwd=REPO, text=True, capture_output=True)
    if proc.returncode != 0:
        raise SystemExit(f"failed: {' '.join(args)}\nstdout={proc.stdout}\nstderr={proc.stderr}")
    return json.loads(proc.stdout)


def assert_check_shape(check):
    assert isinstance(check.get("component"), str), check
    assert isinstance(check.get("ok"), bool), check
    assert isinstance(check.get("message"), str), check
    assert "details" in check, check


def main():
    root = pathlib.Path(tempfile.mkdtemp(prefix="orderk-contract-"))
    vault = root / "vault"
    db = root / "orderk.sqlite"
    queries = root / "queries.json"
    report_dir = root / "reports"
    try:
        (vault / "projects").mkdir(parents=True)
        (vault / "people").mkdir()
        (vault / "projects" / "alpha.md").write_text(
            """---\ntags: [project, alpha]\n---\n# Alpha Project\n\nThe alpha project uses sqlite-vec for local semantic search and agent retrieval.\n""",
            encoding="utf-8",
        )
        (vault / "people" / "ada.md").write_text(
            """# Ada\n\nAda studies retrieval quality, eval gates, and deterministic maintenance reports.\n""",
            encoding="utf-8",
        )
        queries.write_text(
            json.dumps(
                {
                    "schema_version": "orderk.eval_queries.v1",
                    "queries": [
                        {
                            "id": "alpha",
                            "query": "sqlite vec semantic search agent retrieval",
                            "expected_paths": ["projects/alpha.md"],
                        },
                        {
                            "id": "ada",
                            "query": "retrieval quality eval gates maintenance reports",
                            "expected_paths": ["people/ada.md"],
                        },
                    ],
                },
                indent=2,
            ),
            encoding="utf-8",
        )

        index = run(BASE + ["index", "--vault", str(vault), "--db", str(db), "--embedding-provider", "mock", "--embedding-dim", "16", "--embedding-model", "mock-16", "--json"])
        assert index["ok"] is True and index["files"] == 2, index

        status = run(BASE + ["status", "--db", str(db), "--json"])
        assert status["schema_version"] == "orderk.status.v1", status
        assert status["ok"] is True and status["health_state"] == "ready", status
        for check in status["checks"]:
            assert_check_shape(check)

        health = run(BASE + ["health", "--db", str(db), "--vault", str(vault), "--embedding-provider", "mock", "--embedding-dim", "16", "--embedding-model", "mock-16", "--json"])
        assert health["schema_version"] == "orderk.health.v1", health
        assert health["ok"] is True and health["state"] == "ready", health
        for check in health["checks"]:
            assert_check_shape(check)

        doctor = run(BASE + ["doctor", "--db", str(db), "--vault", str(vault), "--smoke-query", "sqlite vec semantic search", "--embedding-provider", "mock", "--embedding-dim", "16", "--embedding-model", "mock-16", "--json"])
        assert doctor["ok"] is True, doctor
        assert any(c["component"] == "smoke_query" and c["ok"] for c in doctor["checks"]), doctor

        search = run(BASE + ["search", "--db", str(db), "--query", "sqlite vec semantic search agent retrieval", "--limit", "3", "--embedding-provider", "mock", "--embedding-dim", "16", "--embedding-model", "mock-16", "--json"])
        assert search["results"] and search["routing"]["returned"] >= 1, search
        assert "score_breakdown" in search["results"][0], search
        assert "evidence" in search["results"][0], search

        eval_report = run(BASE + ["eval", "--db", str(db), "--queries", str(queries), "--limit", "5", "--embedding-provider", "mock", "--embedding-dim", "16", "--embedding-model", "mock-16", "--json"])
        assert eval_report["schema_version"] == "orderk.eval.v1", eval_report
        assert eval_report["ok"] is True and eval_report["hits_at_k"] == 2, eval_report

        maintain = run(BASE + ["maintain", "--db", str(db), "--vault", str(vault), "--queries", str(queries), "--smoke-query", "sqlite vec semantic search", "--report-dir", str(report_dir), "--embedding-provider", "mock", "--embedding-dim", "16", "--embedding-model", "mock-16", "--json"])
        assert maintain["schema_version"] == "orderk.maintain.v1", maintain
        assert maintain["ok"] is True and maintain["state"] == "ready", maintain
        assert maintain["health"]["schema_version"] == "orderk.health.v1", maintain
        assert maintain["eval"]["schema_version"] == "orderk.eval.v1", maintain
        assert maintain["report_path"], maintain
        report_path = pathlib.Path(maintain["report_path"])
        assert report_path.exists(), maintain
        persisted = json.loads(report_path.read_text(encoding="utf-8"))
        assert persisted["schema_version"] == "orderk.maintain.v1", persisted
        print("orderk contract verification passed")
    finally:
        shutil.rmtree(root, ignore_errors=True)


if __name__ == "__main__":
    main()
