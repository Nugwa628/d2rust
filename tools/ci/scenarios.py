#!/usr/bin/env python3
"""The real-data scenario suite: every `tools/scenarios/**/*.scn` through
`D2RUST-Game repro` on the game files, as one table.

    python3 tools/ci/scenarios.py --game-dir DIR --out DIR [--game CMD]
                                  [--budget SPEC] [--timeout S] [FILE ...]
    python3 tools/ci/scenarios.py --selftest

  --game CMD     how to start the game (split like a shell line). Default:
                 build D2RUST-Game once with the test job's settings
                 (`cargo build --locked -p d2-client --features
                 d2-client/dylib`) and run the binary directly, with the
                 Bevy and Rust shared libraries on the library path, so no
                 scenario pays for cargo's own start-up.
  --game-dir     the assembled game files (tools/ci/game_files.py).
  --out          one artifacts folder per scenario (repro.json, perf.json).
                 Artifacts stay on the runner; they are never uploaded.
  --budget       the perf budget each run is measured against
                 (`perf-summary --budget`, default tick=40,frame=17,hitch=0).
                 Report only: a run over budget never fails the suite.
  --timeout      seconds per scenario before it counts as failed (60).
  FILE ...       run these instead of the whole folder.

Each scenario runs as `repro FILE --keep-going --game-dir DIR --out DIR`
with D2_PERF_OUT set, then `perf-summary --json` reads its timing. The
table has one row per scenario: the verdict, ticks, wall time, the server
tick p95 and the budget verdict, and the first failing step. The table's
rows start with `scenario |` so the merge train can pick it out of the
log (progress lines while it runs start with `ran |`); a failed row also
says FAILED.

Exit 0 when every scenario passes, 1 when one fails (a failing check, an
invalid scenario, missing game files, a crash or a timeout), 2 on bad
arguments. Every scenario in the folder is expected to pass on main: a bug
scenario (tools/scenarios/bugs/) asserts the fixed behaviour, so one that
fails means the bug is back.
"""
import argparse
import json
import os
import shlex
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
FOLDER = ROOT / "tools" / "scenarios"
BUILD = ["cargo", "build", "--locked", "-p", "d2-client", "--features", "d2-client/dylib",
         "--bin", "D2RUST-Game", "--message-format=json-render-diagnostics"]
DEFAULT_BUDGET = "tick=40,frame=17,hitch=0"
# The suite's time limit (plans/tools/test-automation-tools.md, T10). Going
# over it is reported, not failed: a slow batch is still a tested batch.
SUITE_LIMIT_S = 120
# repro's exit codes (app/repro `exit`).
VERDICTS = {0: "pass", 1: "FAILED", 2: "FAILED (invalid)", 3: "FAILED (no game files)"}


def scenario_files(folder: Path) -> list:
    """Every .scn under the folder, sorted by path."""
    return sorted(p for p in folder.rglob("*.scn") if p.is_file())


def first_failure(report: dict) -> str:
    """repro.json's first failing step as `TEXT -> result`; a step whose
    text has no `at`/`by` gets `at T: KIND` in front."""
    f = report.get("first_failure")
    if not isinstance(f, dict):
        return ""
    text = str(f.get("text"))
    if not text.startswith(("at ", "by ")):
        text = f"at {f.get('at')}: {f.get('kind')} {text}"
    return f"{text} -> {f.get('result')}"


def last_json(out: str):
    """The last stdout line that is a JSON object (the game prints its
    version and game folder lines first), or None."""
    for ln in reversed(out.splitlines()):
        ln = ln.strip()
        if ln.startswith("{"):
            try:
                return json.loads(ln)
            except ValueError:
                return None
    return None


def built_game() -> tuple:
    """Builds D2RUST-Game once; returns (command, env) to run it directly.
    The binary links Bevy and the Rust standard library as shared
    libraries (d2-client/dylib), so their folders go on the library path
    the way `cargo run` puts them there."""
    p = subprocess.run(BUILD, cwd=ROOT, stdout=subprocess.PIPE, text=True)
    if p.returncode != 0:
        raise SystemExit(f"scenarios: building the game failed (exit {p.returncode})")
    exe = None
    for ln in p.stdout.splitlines():
        try:
            msg = json.loads(ln)
        except ValueError:
            continue
        if msg.get("reason") == "compiler-artifact" and msg.get("executable"):
            exe = msg["executable"]
    if not exe:
        raise SystemExit("scenarios: the build named no D2RUST-Game binary")
    def rustc(*args: str) -> str:
        return subprocess.run(["rustc", *args], cwd=ROOT, capture_output=True, text=True,
                              check=True).stdout
    sysroot = rustc("--print", "sysroot").strip()
    host = next(ln.split(":", 1)[1].strip() for ln in rustc("-vV").splitlines()
                if ln.startswith("host:"))
    # The standard library's shared library sits in the host's rustlib folder.
    libs = [str(Path(exe).parent / "deps"), str(Path(sysroot) / "lib" / "rustlib" / host / "lib")]
    var = "PATH" if os.name == "nt" else "LD_LIBRARY_PATH"
    env = dict(os.environ)
    env[var] = os.pathsep.join(libs + ([env[var]] if env.get(var) else []))
    return [exe], env


def perf_cell(summary) -> tuple:
    """(tick p95 in ms, budget verdict) from `perf-summary --json` output."""
    if not isinstance(summary, dict):
        return "-", "-"
    server = (summary.get("phases_us") or {}).get("server") or {}
    p95 = server.get("p95")
    tick = f"{p95 / 1000:.1f}" if isinstance(p95, (int, float)) else "-"
    verdict = summary.get("verdict") or {}
    if verdict.get("pass") is True:
        budget = "ok"
    elif verdict.get("over"):
        budget = "over: " + "; ".join(verdict["over"])
    else:
        budget = "-"
    return tick, budget


def row(name: str, verdict: str, ticks, wall_s: float, tick: str, budget: str, why: str) -> str:
    return (f"scenario | {name:<34} | {verdict:<22} | {str(ticks):>5} | {wall_s:6.1f} | "
            f"{tick:>6} | {budget:<12} | {why}")


HEADER = row("name", "result", "ticks", 0.0, "tick95", "budget", "first failure").replace(
    "   0.0", "wall s")


def run_one(game: list, base_env: dict, path: Path, game_dir: Path, out: Path, budget: str,
            timeout: int) -> tuple:
    """Runs one scenario; returns (passed, table row)."""
    rel = path.relative_to(FOLDER).as_posix() if path.is_relative_to(FOLDER) else path.name
    name = rel[:-4] if rel.endswith(".scn") else rel
    art = out / name.replace("/", "-")
    art.mkdir(parents=True, exist_ok=True)
    perf = art / "perf.json"
    perf.unlink(missing_ok=True)
    env = dict(base_env, D2_PERF_OUT=str(perf))
    cmd = game + ["repro", str(path), "--mute", "--keep-going", "--game-dir", str(game_dir), "--out", str(art)]
    t0 = time.monotonic()
    try:
        p = subprocess.run(cmd, env=env, capture_output=True, text=True, timeout=timeout)
        code, log = p.returncode, (p.stdout + p.stderr)
    except subprocess.TimeoutExpired:
        code, log = None, ""
    wall = time.monotonic() - t0
    report = {}
    try:
        report = json.loads((art / "repro.json").read_text())
    except (OSError, ValueError):
        pass
    if code is None:
        verdict, why = "FAILED (timeout)", f"no result after {timeout} s"
    else:
        verdict = VERDICTS.get(code, f"FAILED (exit {code})")
        why = first_failure(report)
        if code != 0 and not why:
            # No report: the run's own last line says why (INVALID, NO GAME, a panic).
            lines = [ln for ln in log.splitlines() if ln.strip()]
            why = lines[-1][:160] if lines else ""
    summary = None
    if perf.is_file():
        try:
            s = subprocess.run(game + ["perf-summary", str(perf), "--budget", budget, "--json"],
                               env=base_env, capture_output=True, text=True, timeout=60)
            summary = last_json(s.stdout)
        except subprocess.TimeoutExpired:
            summary = None
    tick, budget_cell = perf_cell(summary)
    return code == 0, row(name, verdict, report.get("ticks", "-"), wall, tick, budget_cell, why)


def selftest() -> int:
    assert first_failure({"first_failure": None}) == ""
    assert first_failure({"first_failure": {"at": 8, "kind": "check", "text": "at 8 check player.mode == cast",
                                            "result": "actual: walk"}}) \
        == "at 8 check player.mode == cast -> actual: walk"
    assert first_failure({"first_failure": {"at": 2, "kind": "dev", "text": "gold x", "result": "bad"}}) \
        == "at 2: dev gold x -> bad"
    assert last_json('D2RUST 0.1\ngame folder: x\n{"a": 1}\n') == {"a": 1}
    assert last_json("no json\n") is None
    assert perf_cell(None) == ("-", "-")
    assert perf_cell({"phases_us": {"server": {"p95": 1500}}, "verdict": {"pass": True, "over": []}}) \
        == ("1.5", "ok")
    assert perf_cell({"phases_us": {}, "verdict": {"pass": False, "over": ["tick p95 41.00 ms > 40 ms"]}}) \
        == ("-", "over: tick p95 41.00 ms > 40 ms")
    assert row("bugs/BUG-008", "pass", 40, 2.0, "1.0", "ok", "").startswith("scenario | bugs/BUG-008")
    assert "wall s" in HEADER and HEADER.startswith("scenario | name")
    assert all(p.suffix == ".scn" for p in scenario_files(FOLDER))
    print("scenarios selftest ok")
    return 0


def main(argv: list) -> int:
    if argv == ["--selftest"]:
        return selftest()
    ap = argparse.ArgumentParser(prog="scenarios.py", add_help=True)
    ap.add_argument("--game")
    ap.add_argument("--game-dir", required=True, type=Path)
    ap.add_argument("--out", required=True, type=Path)
    ap.add_argument("--budget", default=DEFAULT_BUDGET)
    ap.add_argument("--timeout", default=60, type=int)
    ap.add_argument("files", nargs="*", type=Path)
    a = ap.parse_args(argv)
    game, env = (shlex.split(a.game), dict(os.environ)) if a.game else built_game()
    files = [f.resolve() for f in a.files] or scenario_files(FOLDER)
    if not files:
        print(f"scenarios: none in {FOLDER}")
        return 0
    t0 = time.monotonic()
    rows, failed = [], 0
    for f in files:
        ok, r = run_one(game, env, f, a.game_dir, a.out, a.budget, a.timeout)
        failed += not ok
        rows.append(r)
        print(r.replace("scenario |", "ran      |", 1), flush=True)
    total = time.monotonic() - t0
    print()
    print(f"Scenario results ({len(files)} run, {failed} failed, {total:.0f} s; "
          f"perf budget {a.budget}, report only):")
    print(HEADER)
    for r in rows:
        print(r)
    if total > SUITE_LIMIT_S:
        print(f"scenario suite took {total:.0f} s, over its {SUITE_LIMIT_S} s limit (report only)")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
