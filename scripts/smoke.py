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
    smoke = pathlib.Path(tempfile.mkdtemp(prefix="orderk-smoke-"))
    vault = smoke / "vault"
    db = smoke / "orderk.sqlite"
    (vault / "projects").mkdir(parents=True)
    (vault / "archive").mkdir()
    (vault / "projects" / "alpha.md").write_text("""---\ntags: [project, alpha]\n---\n# Alpha Project\n\nThe alpha project uses sqlite-vec for local semantic search.\n""")
    (vault / "projects" / "bravo.md").write_text("""# Bravo Project\n\nBravo documents Obsidian plugin packaging and npm workspace builds.\n""")
    (vault / "archive" / "old.md").write_text("""# Old Archived Note\n\nThis note should be indexed initially and then deleted.\n""")

    index = json.loads(run(BASE + ["index", "--vault", str(vault), "--db", str(db), "--embedding-provider", "mock", "--embedding-dim", "16", "--embedding-model", "mock-16", "--json"]))
    assert index["ok"] and index["files"] == 3, index

    status = json.loads(run(BASE + ["status", "--db", str(db), "--json"]))
    assert status["ok"] and status["notes"] == 3 and status["vector_enabled"], status

    alpha = json.loads(run(BASE + ["search", "--db", str(db), "--query", "sqlite-vec semantic search", "--limit", "3", "--embedding-provider", "mock", "--embedding-dim", "16", "--embedding-model", "mock-16", "--json"]))
    assert alpha["results"] and alpha["results"][0]["path"] == "projects/alpha.md", alpha

    (vault / "archive" / "old.md").unlink()
    deleted = json.loads(run(BASE + ["index", "--vault", str(vault), "--db", str(db), "--embedding-provider", "mock", "--embedding-dim", "16", "--embedding-model", "mock-16", "--json"]))
    assert deleted["deleted"] >= 1, deleted

    status2 = json.loads(run(BASE + ["status", "--db", str(db), "--json"]))
    assert status2["notes"] == 2, status2

    shutil.rmtree(smoke)
    print("orderk smoke verification passed")


if __name__ == "__main__":
    main()
