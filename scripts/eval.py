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
    return proc.stdout


def main():
    root = pathlib.Path(tempfile.mkdtemp(prefix="orderk-eval-"))
    vault = root / "vault"
    db = root / "orderk.sqlite"
    queries = root / "queries.json"
    try:
        (vault / "projects").mkdir(parents=True)
        (vault / "archive").mkdir()
        (vault / "projects" / "alpha.md").write_text(
            """---\ntags: [project, alpha]\n---\n# Alpha Project\n\nThe alpha project uses sqlite-vec for local semantic search.\n""",
            encoding="utf-8",
        )
        (vault / "projects" / "bravo.md").write_text(
            """# Bravo Project\n\nBravo documents Obsidian plugin packaging and npm workspace builds.\n""",
            encoding="utf-8",
        )
        (vault / "archive" / "old.md").write_text(
            """# Old Archived Note\n\nThis note should be indexed initially and then deleted.\n""",
            encoding="utf-8",
        )

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

        queries.write_text(
            json.dumps(
                {
                    "schema_version": "orderk.eval_queries.v1",
                    "queries": [
                        {
                            "id": "alpha",
                            "query": "sqlite vec semantic search",
                            "expected_paths": ["projects/alpha.md"],
                        },
                        {
                            "id": "bravo",
                            "query": "obsidian plugin packaging npm workspace",
                            "expected_paths": ["projects/bravo.md"],
                        },
                    ],
                },
                ensure_ascii=False,
                indent=2,
            ),
            encoding="utf-8",
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
        assert eval_out["ok"] is True, eval_out
        assert eval_out["queries"] == 2, eval_out
        assert eval_out["hits_at_k"] == 2, eval_out
        assert eval_out["top1_hits"] == 2, eval_out
        assert eval_out["zero_hit"] == 0, eval_out
        assert eval_out["recall_at_k"] >= 1.0, eval_out
        assert eval_out["ndcg_at_k"] >= 1.0, eval_out
        assert all("recall_at_k" in outcome and "ndcg_at_k" in outcome for outcome in eval_out["outcomes"]), eval_out
        print(json.dumps(eval_out, ensure_ascii=False, indent=2))
    finally:
        shutil.rmtree(root, ignore_errors=True)


if __name__ == "__main__":
    main()
