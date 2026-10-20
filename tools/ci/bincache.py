#!/usr/bin/env python3
"""Ready-built game binaries by commit. Builds use isolated local worktrees.

D2_BIN_ROOT overrides E:/work/bin (or /cache/bin on Linux). get never builds;
a cache miss exits 1. Build output goes to stderr so stdout is always a path.
"""
from __future__ import annotations

import argparse
from contextlib import contextmanager
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
QUEUE = "C:/Users/USER/.t3/projects/lc/scratch/build-slot.sh"


class Cache:
    def __init__(self, root=None, repo=ROOT, runner=subprocess.run, env=None):
        self.env = dict(os.environ if env is None else env)
        if os.name == "nt" and not shutil.which("sh", path=self.env.get("PATH", "")):
            self.env["PATH"] = "C:/Program Files/Git/bin;" + self.env.get("PATH", "")
        self.root = Path(root or self.env.get("D2_BIN_ROOT", "E:/work/bin" if os.name == "nt" else "/cache/bin")).resolve()
        self.env["D2_BIN_ROOT"] = str(self.root)
        self.repo = Path(repo)
        self.runner = runner

    def command(self, args, cwd=None, capture=False):
        result = self.runner([str(a) for a in args], cwd=cwd or self.repo,
                             env=self.env, check=True, text=True,
                             stdout=subprocess.PIPE if capture else sys.stderr,
                             stderr=sys.stderr)
        return result.stdout.strip() if capture else None

    def sha(self, value):
        if not re.fullmatch(r"[0-9a-fA-F]{7,40}", value):
            raise ValueError("SHA must be 7-40 hexadecimal characters")
        sha = self.command(["git", "rev-parse", "--verify", value + "^{commit}"], capture=True)
        if not re.fullmatch(r"[0-9a-f]{40}", sha):
            raise ValueError("git did not return a full commit SHA")
        return sha

    def binary(self, sha, profile):
        if profile not in ("release", "dist"):
            raise ValueError("profile must be release or dist")
        base = self.root / sha
        if profile != "release":
            base /= profile
        # Q5 downloads a Linux artifact without an .exe suffix even when
        # invoked on BIGRIG. Keep it distinct from a locally built executable.
        farm_release = profile == "release" and self.env.get("D2_FARM_HOST")
        return base / ("D2RUST-Game.exe" if os.name == "nt" and not farm_release else "D2RUST-Game")

    def get(self, sha, profile="release"):
        binary = self.binary(self.sha(sha), profile)
        if not binary.is_file():
            raise FileNotFoundError(f"binary not cached: {binary}; run bincache.py build {sha} --profile {profile}")
        return binary

    @contextmanager
    def lock(self, sha):
        self.root.mkdir(parents=True, exist_ok=True)
        lock = self.root / (sha + ".lock")
        try:
            lock.mkdir()
        except FileExistsError:
            raise RuntimeError(f"cache entry busy: {sha}; retry after its build/prune finishes") from None
        try:
            yield
        finally:
            lock.rmdir()

    def build(self, value, profile="release"):
        sha = self.sha(value)
        binary = self.binary(sha, profile)
        with self.lock(sha):
            if binary.is_file():
                return binary
            if self.env.get("D2_FARM_HOST"):
                farm = self.repo / "tools/farm/farm.py"
                if not farm.is_file():
                    raise RuntimeError("D2_FARM_HOST is set but tools/farm/farm.py is unavailable; merge Q5 or unset D2_FARM_HOST")
                if profile != "release":
                    raise ValueError("farm bin SHA currently supplies the release profile only")
                self.command([sys.executable, farm, "bin", sha])
                if not binary.is_file():
                    raise FileNotFoundError(f"farm did not populate {binary}")
                return binary
            print("bincache: building locally", file=sys.stderr)
            source = self.root / "src" / sha
            source.parent.mkdir(parents=True, exist_ok=True)
            if not source.exists():
                self.command(["git", "worktree", "add", "--detach", source, sha])
            head = self.command(["git", "rev-parse", "HEAD"], cwd=source, capture=True)
            dirty = self.command(["git", "status", "--porcelain"], cwd=source, capture=True)
            if head != sha or dirty:
                raise RuntimeError(f"cache checkout is modified: {source}")
            # Explicit target directory prevents inherited shared-target settings
            # from picking up another lane's binary.
            target = source / "target"
            self.command(["sh", QUEUE, "cargo", "build", "-j", "12", "-p", "d2-client",
                          "--bin", "D2RUST-Game", "--locked", "--profile", profile,
                          "--target-dir", target], cwd=source)
            built = target / profile / binary.name
            if not built.is_file():
                raise FileNotFoundError(f"build did not produce {built}")
            binary.parent.mkdir(parents=True, exist_ok=True)
            temp = binary.with_suffix(binary.suffix + ".tmp")
            shutil.copy2(built, temp)
            temp.replace(binary)
            return binary

    def prune(self, keep):
        if keep < 0:
            raise ValueError("--keep must be nonnegative")
        if not self.root.exists():
            return []
        entries = sorted((p for p in self.root.iterdir() if p.is_dir() and re.fullmatch(r"[0-9a-f]{40}", p.name)),
                         key=lambda p: (p.stat().st_mtime_ns, p.name), reverse=True)
        removed = []
        for entry in entries[keep:]:
            try:
                with self.lock(entry.name):
                    source = self.root / "src" / entry.name
                    if source.exists():
                        # Without --force: never discard edits in a cache checkout.
                        self.command(["git", "worktree", "remove", source])
                    if entry.resolve().parent != self.root:
                        raise RuntimeError(f"cache entry escapes root: {entry}")
                    shutil.rmtree(entry)
                    removed.append(entry.name)
            except RuntimeError as error:
                if "cache entry busy:" not in str(error):
                    raise
        return removed


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    for name in ("get", "build"):
        command = sub.add_parser(name)
        command.add_argument("sha")
        command.add_argument("--profile", choices=("release", "dist"), default="release")
    sub.add_parser("prune").add_argument("--keep", type=int, required=True)
    args = parser.parse_args(argv)
    try:
        cache = Cache()
        if args.command == "prune":
            for sha in cache.prune(args.keep):
                print(sha)
        else:
            print(getattr(cache, args.command)(args.sha, args.profile))
        return 0
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"bincache: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
