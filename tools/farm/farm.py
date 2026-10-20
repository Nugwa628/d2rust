#!/usr/bin/env python3
"""SSH build/test/soak farm. Python stdlib only; no game assets are transferred.

Commands stream stdout/stderr and preserve child exit codes. With no
D2_FARM_HOST, commands explicitly select local execution (or explain why they
cannot run). See README.md for the build-slot interface and cache contract.
"""
import argparse
import hashlib
import importlib.util
import json
import math
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import sys
import time
from dataclasses import dataclass

REPO = Path(__file__).resolve().parents[2]
FORGE = "https://git.werdegames.com/WerdeGames/D2RUST"
SHA = re.compile(r"[0-9a-fA-F]{40}\Z")


def module(path, name):
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


@dataclass
class Config:
    host: str = ""
    key: str = ""
    root: str = "/srv/d2farm"
    cache: str = "/cache"
    bins: Path = Path("E:/work/bin")
    candidates: Path = Path("E:/work/artifacts/candidates")
    summary: Path = Path("C:/Users/USER/.t3/projects/lc/scratch/bugs-unverified/farm-candidates.md")

    @classmethod
    def env(cls):
        return cls(host=os.environ.get("D2_FARM_HOST", ""),
                   key=os.environ.get("D2_FARM_KEY", str(Path.home() / ".ssh/lc_ci_ed25519")),
                   root=os.environ.get("D2_FARM_ROOT", "/srv/d2farm"),
                   cache=os.environ.get("D2_FARM_CACHE", "/cache"),
                   bins=Path(os.environ.get("D2_BIN_ROOT", "E:/work/bin" if os.name == "nt" else "/cache/bin")),
                   candidates=Path(os.environ.get("D2_FARM_CANDIDATES", "E:/work/artifacts/candidates" if os.name == "nt" else "/cache/candidates")),
                   summary=Path(os.environ.get("D2_FARM_SUMMARY", str(cls.summary) if os.name == "nt" else "/cache/candidates/farm-candidates.md")))

    def validate(self):
        if self.host and (self.host.startswith("-") or not re.fullmatch(r"[\w.@:-]+", self.host)):
            raise ValueError("D2_FARM_HOST must be an SSH host alias or user@host")
        for path in (self.root, self.cache):
            if not path.startswith("/") or "\n" in path:
                raise ValueError("farm root/cache must be absolute POSIX paths")


class Runner:
    """Injectable boundary. Inherit stdio for immediate logs; never buffer a gate."""
    def run(self, argv, *, script=None, cwd=None, env=None):
        if script is None:
            return subprocess.run(argv, cwd=cwd, env=env).returncode
        # communicate sends the script while stdout/stderr stay live.
        with subprocess.Popen(argv, stdin=subprocess.PIPE, cwd=cwd, env=env,
                              text=True, encoding="utf-8") as proc:
            proc.communicate(script)
            return proc.returncode


def ssh(config):
    config.validate()
    args = ["ssh", "-T", "-o", "BatchMode=yes", "-o", "StrictHostKeyChecking=yes"]
    if config.key:
        args += ["-i", config.key]
    return args + ["--", config.host]


def remote(config, runner, script):
    return runner.run(ssh(config) + ["bash -se"], script="set -euo pipefail\n" + script)


def commit(value):
    if not SHA.fullmatch(value):
        raise ValueError("SHA must be a full 40-character hexadecimal commit id")
    return value.lower()


def gate_command(kind, packages):
    for package in packages:
        if not re.fullmatch(r"[A-Za-z0-9_-]+", package):
            raise ValueError(f"invalid package: {package}")
    flags = [x for p in packages for x in ("-p", p)]
    if kind == "build":
        return ["cargo", "build", "--locked", "--release", "-j", "12"] + flags
    if kind == "clippy":
        return (["cargo", "clippy", "--locked", "-j", "12", "--all-targets"] + flags
                + ["--features", "d2-client/dylib", "--", "-D", "warnings"])
    return (["cargo", "nextest", "run", "--locked", "--build-jobs", "12", "--test-threads", "12"]
            + flags + ["--features", "d2-client/dylib"])


def checkout_script(config, sha, command):
    """Fetch only from the forge, isolate the commit's tree and target, lock both."""
    root, cache = shlex.quote(config.root), shlex.quote(config.cache)
    return f"""root={root}
cache={cache}
sha={shlex.quote(commit(sha))}
mkdir -p "$root/worktrees" "$cache/target" "$cache/bin/$sha"
exec 8>"$root/checkout.lock"
flock 8
git -C "$root/repo" fetch --no-tags {shlex.quote(FORGE)} '+refs/heads/*:refs/remotes/origin/*'
test "$(git -C "$root/repo" cat-file -t "$sha")" = commit
if ! test -d "$root/worktrees/$sha"; then
  git -C "$root/repo" worktree add --detach "$root/worktrees/$sha" "$sha"
fi
flock -u 8
exec 9>"$cache/target/$sha.lock"
flock 9
cd "$root/worktrees/$sha"
test "$(git rev-parse HEAD)" = "$sha"
test -z "$(git status --porcelain)"
export PATH="$HOME/.cargo/bin:$PATH"
export CARGO_TARGET_DIR="$cache/target/$sha"
export D2_GAME_DIR="$cache/game"
export D2RUST_HOME="$cache/target/$sha/user"
export D2RUST_LOG_DIR="$cache/target/$sha/logs"
{command}
"""


def gate(config, runner, kind, sha, packages):
    sha = commit(sha)
    cmd = gate_command(kind, packages)
    if config.host:
        command = shlex.join(cmd)
        if kind == "build" and (not packages or "d2-client" in packages):
            command += '\ninstall -m755 "$CARGO_TARGET_DIR/release/D2RUST-Game" "$cache/bin/$sha/D2RUST-Game.tmp"'
            command += '\nmv "$cache/bin/$sha/D2RUST-Game.tmp" "$cache/bin/$sha/D2RUST-Game"'
            command += '\nprintf "%s\\n" "$sha" > "$cache/bin/$sha/commit"'
        return remote(config, runner, checkout_script(config, sha, command))
    # Never quietly build the dirty/current checkout for a requested other SHA.
    actual = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=REPO, text=True).strip()
    dirty = subprocess.check_output(["git", "status", "--porcelain"], cwd=REPO, text=True).strip()
    if actual != sha or dirty:
        print("farm: local gate unavailable: requested SHA must be this clean checkout's HEAD", file=sys.stderr)
        return 2
    if os.name == "nt":
        queue = Path("C:/Users/USER/.t3/projects/lc/scratch/build-slot.sh")
        shell = shutil.which("sh") or "C:/Program Files/Git/bin/sh.exe"
        if not queue.is_file():
            print("farm: local gate unavailable: build queue script missing", file=sys.stderr)
            return 2
        cmd = [shell, str(queue)] + cmd
        if kind == "test":
            rc = runner.run([shell, str(queue.with_name("ci-wait.sh"))], cwd=REPO)
            if rc:
                return rc
    # Avoid the queue's farm hook dispatching back into this command.
    env = dict(os.environ, D2_FARM_HOST="")
    rc = runner.run(cmd, cwd=REPO, env=env)
    if not rc and kind == "build" and (not packages or "d2-client" in packages):
        name = "D2RUST-Game" + (".exe" if os.name == "nt" else "")
        built = REPO / Path(env.get("CARGO_TARGET_DIR", "target")) / "release" / name
        target = config.bins / sha / name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(built, target)
    return rc


def download(config, runner, source, destination):
    """SCP only tool-owned artifacts; SSH uses the same key/host-key policy."""
    destination = Path(destination)
    destination.parent.mkdir(parents=True, exist_ok=True)
    argv = ["scp", "-B", "-o", "BatchMode=yes", "-o", "StrictHostKeyChecking=yes"]
    config.validate()
    if config.key:
        argv += ["-i", config.key]
    return runner.run(argv + ["--", config.host + ":" + source, str(destination)])


def binary(config, runner, sha):
    sha = commit(sha)
    # Linux farm binaries have no .exe suffix. Do not masquerade as Windows cache entries.
    dest = config.bins / sha / "D2RUST-Game"
    if not config.host:
        paths = [dest.with_suffix(".exe")] if os.name == "nt" else [dest]
        for path in paths:
            if path.is_file():
                print(path)
                return 0
        print("farm: local binary unavailable: no cached binary for this SHA; run build first", file=sys.stderr)
        return 2
    # Publish only a complete download, leaving a previous cache entry intact on failure.
    part = dest.with_name(dest.name + ".part")
    rc = download(config, runner, f"{config.cache}/bin/{sha}/D2RUST-Game", part)
    if not rc:
        part.replace(dest)
        dest.with_name("farm.json").write_text(json.dumps({"sha": sha, "platform": "linux", "profile": "release"}) + "\n")
        print(dest)
    else:
        part.unlink(missing_ok=True)
    return rc


def signature_id(signature):
    return hashlib.sha256(signature.encode()).hexdigest()


def collect(soak, out, candidates):
    """Reduce/export once per stable signature. Metadata marks every finding Unverified."""
    out, candidates = Path(out), Path(candidates)
    count = 0
    db = out / "findings.jsonl"
    if not db.exists():
        return count
    for line in db.read_text(encoding="utf-8").splitlines():
        entry = json.loads(line)
        signature = entry["sig"]
        folder = candidates / signature_id(signature)
        if (folder / "candidate.json").is_file():
            continue
        reduced = entry.get("reduced")
        if not reduced:
            if "reduced" in entry:
                print(f"farm: cannot export {signature}: {entry.get('reduce_note', 'reduction failed')}", file=sys.stderr)
                continue
            reduced, note = soak.reduce(entry["log"], signature, str(out))
            if not reduced:
                print(f"farm: cannot reduce {signature}: {note}", file=sys.stderr)
                continue
        folder.mkdir(parents=True, exist_ok=True)
        soak.export_scenario(reduced, signature, folder / "failure.scn",
                             max(entry.get("step", 0), soak.last_step(reduced), 0) + 400)
        metadata = {**entry, "status": "Unverified", "scenario": "failure.scn"}
        (folder / "candidate.json").write_text(json.dumps(metadata, indent=2) + "\n", encoding="utf-8")
        count += 1
    return count


def soak_local(config, hours, seeds, out):
    soak = module(REPO / "tools/soak/soak.py", "farm_soak")
    for tool in (soak.CLIENT, soak.D2S):
        if not Path(tool).is_file():
            raise ValueError(f"local soak unavailable: {tool} missing; build d2-client and d2s-tool first")
    deadline = time.monotonic() + hours * 3600
    out = Path(out)
    out.mkdir(parents=True, exist_ok=True)
    os.environ.setdefault("D2RUST_HOME", str(out.resolve() / "user"))
    os.environ.setdefault("D2RUST_LOG_DIR", str(out.resolve() / "logs"))
    # Each seed receives an equal time slice; campaign handles all starts and reduction.
    for index, seed in enumerate(seeds):
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            break
        args = argparse.Namespace(out=str(out), minutes=remaining / (len(seeds) - index) / 60,
                                  first_seed=seed, starts=None, steps=3000, timeout=600,
                                  no_reduce=False, roundtrip_every=0)
        soak.campaign(args, on_finding=lambda entry: collect(soak, out, config.candidates))
        collect(soak, out, config.candidates)
    return 0


def pull_candidates(config, runner):
    dest = config.candidates
    dest.mkdir(parents=True, exist_ok=True)
    if config.host:
        # Download only .scn, referenced start saves and JSON metadata. Never game art.
        # rsync incremental mirroring retains signature directories across pulls.
        transport = shlex.join(ssh(config)[:-2])
        rc = runner.run(["rsync", "-rlt", "--protect-args", "--safe-links", "-e", transport,
                         "--include=*/", "--include=*.scn", "--include=*.d2s",
                         "--include=candidate.json", "--exclude=*", "--",
                         config.host + ":" + config.cache + "/candidates/", str(dest) + "/"])
        if rc:
            return rc
    existing = config.summary.read_text(encoding="utf-8") if config.summary.exists() else ""
    lines = []
    for metadata in sorted(dest.glob("*/candidate.json")):
        entry = json.loads(metadata.read_text(encoding="utf-8"))
        sig = entry["sig"]
        ident = signature_id(sig)
        if metadata.parent.name != ident or not (metadata.parent / "failure.scn").is_file():
            raise ValueError(f"incomplete or mismatched candidate: {metadata}")
        marker = f"<!-- farm:{ident} -->"
        if marker not in existing:
            safe = sig.replace("\n", " ").replace("\r", " ").replace("`", "'")
            lines.append(f"- **Unverified** `{safe}` — {metadata.parent / 'failure.scn'} {marker}\n")
    if lines:
        config.summary.parent.mkdir(parents=True, exist_ok=True)
        with config.summary.open("a", encoding="utf-8") as stream:
            stream.writelines(lines)
    print(f"farm: {len(lines)} new Unverified signature(s); candidates in {dest}")
    return 0


def main(argv=None, *, config=None, runner=None):
    if (sys.argv[1:] if argv is None else argv) == ["--selftest"]:
        import unittest
        suite = unittest.defaultTestLoader.discover(str(Path(__file__).parent / "tests"))
        return 0 if unittest.TextTestRunner(verbosity=2).run(suite).wasSuccessful() else 1
    ap = argparse.ArgumentParser(description=__doc__)
    sub = ap.add_subparsers(dest="command", required=True)
    for kind in ("build", "clippy", "test"):
        p = sub.add_parser(kind)
        p.add_argument("sha")
        p.add_argument("-p", "--package", action="append", default=[])
    p = sub.add_parser("bin")
    p.add_argument("sha")
    p = sub.add_parser("soak")
    p.add_argument("--hours", type=float, required=True)
    p.add_argument("--seeds", required=True, help="comma-separated integer seeds")
    p.add_argument("--out", type=Path)
    p.add_argument("--local", action="store_true", help=argparse.SUPPRESS)
    p = sub.add_parser("candidates")
    p.add_argument("action", choices=["pull"])
    sub.add_parser("status")
    a = ap.parse_args(argv)
    config, runner = config or Config.env(), runner or Runner()
    if getattr(a, "local", False):
        config.host = ""
    print(f"farm: {'remote ' + config.host if config.host else 'local (D2_FARM_HOST unset)'}", flush=True)
    try:
        config.validate()
        if a.command in ("build", "clippy", "test"):
            return gate(config, runner, a.command, a.sha, a.package)
        if a.command == "bin":
            return binary(config, runner, a.sha)
        if a.command == "candidates":
            return pull_candidates(config, runner)
        if a.command == "status":
            if config.host:
                return remote(config, runner, shlex.join(["df", "-h", config.root, config.cache]) + "\nuptime\n")
            print(f"checkout: {REPO}\nbinary cache: {config.bins}\ncandidates: {config.candidates}")
            return 0
        if not math.isfinite(a.hours) or a.hours <= 0:
            raise ValueError("--hours must be positive and finite")
        seeds = [int(s) for s in a.seeds.split(",")]
        if not seeds or any(s < 0 or s >= 2**64 for s in seeds):
            raise ValueError("seeds must be unsigned 64-bit integers")
        if config.host:
            # The server uses its provisioned main checkout; no local source is uploaded.
            out = str(a.out or Path(config.cache) / "soak")
            command = ["python3", config.root + "/repo/tools/farm/farm.py", "soak", "--local",
                       "--hours", str(a.hours), "--seeds", a.seeds, "--out", out]
            script = (f"export D2_GAME_DIR={shlex.quote(config.cache + '/game')}\n"
                      f"export D2_FARM_CANDIDATES={shlex.quote(config.cache + '/candidates')}\n"
                      f"export D2RUST_HOME={shlex.quote(out + '/user')}\n"
                      f"export D2RUST_LOG_DIR={shlex.quote(out + '/logs')}\n"
                      f"exec 9>{shlex.quote(config.root + '/soak.lock')}\nflock -n 9\n"
                      + shlex.join(command) + "\n")
            return remote(config, runner, script)
        return soak_local(config, a.hours, seeds, a.out or config.candidates.parent / "soak")
    except (ValueError, OSError, subprocess.CalledProcessError) as exc:
        print(f"farm: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
