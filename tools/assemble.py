#!/usr/bin/env python3
"""Rebuild the game's MPQs from the pieces in game-data/ (cloud sessions, CI
jobs that need game files).

    python3 tools/assemble.py DEST_DIR        # then D2_GAME_DIR=DEST_DIR
    python3 tools/assemble.py --selftest

game-data/manifest.json (format version 1) lists every MPQ with its size,
sha256 and the .part-NNN pieces it was split into (files over 95 MiB; none
for smaller ones). Each file is written into DEST_DIR by joining its pieces
from game-data/mpq/, and its size and sha256 are checked. A file already in
DEST_DIR with the right size and sha256 is left as it is. Exit 1 on any
mismatch.

The MPQs are the owner's own Diablo II files. They stay in this private
repository and the player's machine: never in a release, a public place or
an include_bytes! (CLAUDE.md hard rule 1).
"""
import argparse
import hashlib
import json
import sys
import tempfile
from pathlib import Path

FORMAT_VERSION = 1
ROOT = Path(__file__).resolve().parent.parent


def digest(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def assemble(manifest_path: Path, src: Path, dest: Path) -> int:
    """Writes every file of the manifest into dest; returns the mismatch count."""
    manifest = json.loads(manifest_path.read_text())
    if manifest["version"] != FORMAT_VERSION:
        print(f"manifest version {manifest['version']}, expected {FORMAT_VERSION}")
        return 1
    dest.mkdir(parents=True, exist_ok=True)
    bad = kept = 0
    for f in manifest["files"]:
        dst = dest / f["path"]
        if dst.is_file() and dst.stat().st_size == f["size"] and digest(dst) == f["sha256"]:
            kept += 1
            continue
        h = hashlib.sha256()
        with dst.open("wb") as out:
            for piece in f["parts"] or [f["path"]]:
                with (src / piece).open("rb") as p:
                    for block in iter(lambda: p.read(1 << 20), b""):
                        h.update(block)
                        out.write(block)
        if dst.stat().st_size != f["size"] or h.hexdigest() != f["sha256"]:
            print(f"MISMATCH {f['path']}")
            bad += 1
    print(f"{len(manifest['files'])} files -> {dest} ({kept} already there), {bad} mismatches")
    return bad


def selftest() -> int:
    with tempfile.TemporaryDirectory() as t:
        t = Path(t)
        src = t / "mpq"
        src.mkdir()
        big = bytes(range(256)) * 5
        (src / "a.mpq.part-000").write_bytes(big[:700])
        (src / "a.mpq.part-001").write_bytes(big[700:])
        (src / "b.mpq").write_bytes(b"small")
        files = [
            {"path": "a.mpq", "size": len(big), "sha256": hashlib.sha256(big).hexdigest(),
             "parts": ["a.mpq.part-000", "a.mpq.part-001"]},
            {"path": "b.mpq", "size": 5, "sha256": hashlib.sha256(b"small").hexdigest(), "parts": []},
        ]
        man = t / "manifest.json"
        man.write_text(json.dumps({"version": 1, "files": files}))
        out = t / "game"
        assert assemble(man, src, out) == 0
        assert (out / "a.mpq").read_bytes() == big and (out / "b.mpq").read_bytes() == b"small"
        # A second run keeps the files; a damaged piece is reported.
        assert assemble(man, src, out) == 0
        (out / "b.mpq").unlink()
        (src / "b.mpq").write_bytes(b"SMALL")
        assert assemble(man, src, out) == 1
    print("assemble selftest: ok")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("dest", type=Path, nargs="?")
    ap.add_argument("--manifest", type=Path, default=ROOT / "game-data" / "manifest.json")
    ap.add_argument("--src", type=Path, default=ROOT / "game-data" / "mpq")
    ap.add_argument("--selftest", action="store_true")
    a = ap.parse_args()
    if a.selftest:
        return selftest()
    if a.dest is None:
        ap.error("DEST_DIR is required")
    return 1 if assemble(a.manifest, a.src, a.dest) else 0


if __name__ == "__main__":
    sys.exit(main())
