#!/usr/bin/env python3
"""Game files for the real-data scenario job: the owner's MPQs, assembled
once on the CI server's runner and kept there.

    python3 tools/ci/game_files.py DEST     # then D2_GAME_DIR=DEST
    python3 tools/ci/game_files.py --selftest

The job's checkout leaves out game-data/mpq/ (the parts)
(ci.yml). DEST lives on the runner's own disk and outlives the job, so the
2 GB of MPQ parts come down from the forge only when the manifest changes:

  - DEST/.manifest-sha256 holds the sha256 of the manifest DEST was built
    from. When it matches the checked-out manifest and every file has its
    size, DEST is used as it is.
  - Otherwise the checkout widens to game-data/mpq/ (git fetches the parts),
    tools/assemble.py rebuilds DEST and checks every sha256, the stamp is
    written, and the checkout narrows again so the parts leave the job's
    workspace.

The files never leave the runner (the CI server, or BIGRIG for an overflow
job): no step uploads DEST, the parts or anything read from them (CLAUDE.md hard rule 1). Exit 1 when DEST can't be
built.
"""
import hashlib
import json
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
STAMP = ".manifest-sha256"


def up_to_date(manifest: Path, dest: Path) -> bool:
    """DEST was built from this manifest and every file has its size."""
    stamp = dest / STAMP
    if not stamp.is_file():
        return False
    if stamp.read_text().strip() != hashlib.sha256(manifest.read_bytes()).hexdigest():
        return False
    files = json.loads(manifest.read_text())["files"]
    return all((dest / f["path"]).is_file() and (dest / f["path"]).stat().st_size == f["size"]
               for f in files)


def git(*args: str) -> None:
    subprocess.run(["git", "-C", str(ROOT), *args], check=True)


def build(manifest: Path, dest: Path) -> int:
    """Fetches the parts, assembles DEST, writes the stamp; returns the exit code."""
    (dest / STAMP).unlink(missing_ok=True)
    patterns = subprocess.run(["git", "-C", str(ROOT), "sparse-checkout", "list"],
                              capture_output=True, text=True).stdout.splitlines()
    print("game files: fetching game-data/mpq/ (the manifest changed or DEST is new)", flush=True)
    try:
        if patterns:
            # Later patterns win: this one undoes the job's `!/game-data/mpq/`.
            git("sparse-checkout", "add", "/game-data/mpq/")
        rc = subprocess.run([sys.executable, str(ROOT / "tools/assemble.py"), str(dest)]).returncode
    finally:
        if patterns:
            git("sparse-checkout", "set", "--no-cone", *patterns)
    if rc != 0:
        return 1
    (dest / STAMP).write_text(hashlib.sha256(manifest.read_bytes()).hexdigest() + "\n")
    return 0


def selftest() -> int:
    with tempfile.TemporaryDirectory() as t:
        t = Path(t)
        man = t / "manifest.json"
        man.write_text(json.dumps({"version": 1, "files": [
            {"path": "a.mpq", "size": 3, "sha256": "x", "parts": []}]}))
        dest = t / "dest"
        dest.mkdir()
        assert not up_to_date(man, dest), "no stamp"
        (dest / "a.mpq").write_bytes(b"abc")
        (dest / STAMP).write_text(hashlib.sha256(man.read_bytes()).hexdigest() + "\n")
        assert up_to_date(man, dest), "stamp and sizes match"
        (dest / "a.mpq").write_bytes(b"ab")
        assert not up_to_date(man, dest), "a file has the wrong size"
        (dest / "a.mpq").write_bytes(b"abc")
        (dest / STAMP).write_text("old\n")
        assert not up_to_date(man, dest), "another manifest"
    print("game_files selftest ok")
    return 0


def main(argv: list) -> int:
    if argv == ["--selftest"]:
        return selftest()
    if len(argv) != 1:
        print(__doc__.strip().splitlines()[3])
        return 2
    dest = Path(argv[0])
    manifest = ROOT / "game-data/manifest.json"
    if not manifest.is_file():
        print("game files: game-data/manifest.json is not checked out")
        return 1
    dest.mkdir(parents=True, exist_ok=True)
    if up_to_date(manifest, dest):
        print(f"game files: {dest} is up to date")
        return 0
    return build(manifest, dest)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
