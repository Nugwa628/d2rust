#!/usr/bin/env python3
"""Read-only behaviour/replay comparison and performance trends. Exit 0 same,
1 changed, 2 invalid/incomplete run. Artifacts remain in --out on local disk.
"""
from __future__ import annotations

import argparse
from fractions import Fraction
import json
import math
import os
from pathlib import Path
import re
import statistics
import subprocess
import sys
import time

try:
    from .bincache import Cache, ROOT
except ImportError:
    from bincache import Cache, ROOT


def read(path):
    value = json.loads(Path(path).read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"expected JSON object: {path}")
    return value


def write(path, value):
    Path(path).parent.mkdir(parents=True, exist_ok=True)
    Path(path).write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def binary(value, cache=None, profile="release"):
    path = Path(value)
    if path.is_file():
        return path.resolve()
    cache = cache or Cache()
    # Resolve symbolic refs here; Q1 deliberately accepts hexadecimal SHAs only.
    sha = cache.command(["git", "rev-parse", "--verify", value + "^{commit}"], capture=True)
    try:
        return cache.get(sha, profile)
    except FileNotFoundError:
        return cache.build(sha, profile)


def load_dump(path):
    records = [json.loads(line) for line in Path(path).read_text(encoding="utf-8").splitlines() if line.strip()]
    if not records or any(not isinstance(r, dict) for r in records) or records[0].get("format") != "state-1":
        raise ValueError(f"not a state-1 dump: {path}")
    rate = records[0].get("tick_rate")
    if type(rate) is not int or not 1 <= rate <= 1000:
        raise ValueError("state-1 header needs tick_rate in 1..1000")
    origin = records[0].get("start_frame", 0)
    if type(origin) is not int or origin < 0:
        raise ValueError("start_frame must be a nonnegative integer")
    snaps = {}
    for record in records[1:]:
        if record.get("k") != "snap":
            continue
        frame = record["f"]
        if type(frame) is not int or frame < origin or frame in snaps:
            raise ValueError("invalid or duplicate snapshot frame")
        units = record.get("units")
        if not isinstance(units, list):
            raise ValueError("snapshot needs units")
        if any(not isinstance(u, dict) or any(type(u.get(k)) is not int or u[k] < 0
                                             for k in ("ut", "g")) for u in units):
            raise ValueError("units need nonnegative integer ut/g identities")
        if not isinstance(record.get("seed"), list) or len(record["seed"]) != 2:
            raise ValueError("snapshot needs a two-word game seed")
        ids = [(u["ut"], u["g"]) for u in units]
        if len(ids) != len(set(ids)):
            raise ValueError("duplicate unit identity")
        snaps[frame] = record
    if not snaps or list(snaps) != sorted(snaps):
        raise ValueError("dump needs ordered snapshots")
    if min(snaps) not in (origin, origin + 1):
        raise ValueError("incomplete dump: missing initial ticks (or set start_frame)")
    return rate, {Fraction(f - origin, rate): s for f, s in snaps.items()}


MISSING = {"missing": True}


def first_field(a, b, ignore, prefix=""):
    if isinstance(a, dict) and isinstance(b, dict):
        for key in sorted(a.keys() | b.keys()):
            name = f"{prefix}.{key}" if prefix else key
            if key in ignore or name in ignore:
                continue
            diff = first_field(a.get(key, MISSING), b.get(key, MISSING), ignore, name)
            if diff:
                return diff
        return None
    if type(a) is not type(b) or a != b:
        return {"field": prefix, "a": a, "b": b}
    return None


def snapshot_diff(a, b, ignore):
    meta = first_field({k: v for k, v in a.items() if k not in ("f", "units")},
                       {k: v for k, v in b.items() if k not in ("f", "units")}, ignore)
    if meta:
        return dict(meta, unit=None)
    aa = {(u["ut"], u["g"]): u for u in a["units"]}
    bb = {(u["ut"], u["g"]): u for u in b["units"]}
    for key in sorted(aa.keys() | bb.keys()):
        diff = first_field(aa.get(key, MISSING), bb.get(key, MISSING), ignore)
        if diff:
            return dict(diff, unit=list(key))
    return None


def compare(a, b, ignore=(), context=2, equal_time=False):
    ar, aa = load_dump(a)
    br, bb = load_dump(b)
    if ar != br and not equal_time:
        raise ValueError("different tick rates: use --equal-time")
    # Never silently compare sparse/incomplete recordings as a complete replay.
    for rate, snaps in ((ar, aa), (br, bb)):
        times = sorted(snaps)
        if any(y - x != Fraction(1, rate) for x, y in zip(times, times[1:])):
            raise ValueError("incomplete dump: snapshots must cover every tick")
    # state-dump starts after tick 1; repro queries include tick 0. Both
    # conventions cover game time starting at zero, without inventing a state.
    astart = max(Fraction(0), min(aa) - Fraction(1, ar))
    bstart = max(Fraction(0), min(bb) - Fraction(1, br))
    if astart != bstart or max(aa) != max(bb) or (ar == br and min(aa) != min(bb)):
        return {"same": False, "field": "time_range", "a": [str(min(aa)), str(max(aa))],
                "b": [str(min(bb)), str(max(bb))]}
    times = sorted(aa.keys() & bb.keys())
    if not times:
        raise ValueError("no common game times")
    for i, t in enumerate(times):
        diff = snapshot_diff(aa[t], bb[t], set(ignore))
        if diff:
            return dict(diff, same=False, time=str(t), tick_a=aa[t]["f"], tick_b=bb[t]["f"],
                        context=[{"time": str(s), "a": aa[s], "b": bb[s]}
                                 for s in times[max(0, i-context):i+context+1]])
    return {"same": True, "compared": len(times), "tick_rates": [ar, br]}


def normalized_scenario(source):
    """Use the runner's ceil timing rule, preserving durations and save base."""
    source = Path(source).resolve()
    text = source.read_text(encoding="utf-8")
    source_lines = [raw.split("#", 1)[0].strip() for raw in text.splitlines()]
    rate_lines = [line for line in source_lines if line and line.split()[0] == "tick_rate"]
    if len(rate_lines) > 1 or (rate_lines and not re.fullmatch(r"tick_rate\s+\d+", rate_lines[0])):
        raise ValueError("invalid or duplicate scenario tick_rate")
    rate = int(rate_lines[0].split()[1]) if rate_lines else 25
    if not 1 <= rate <= 1000:
        raise ValueError("invalid scenario tick_rate")
    def ticks(value):
        return (int(value) * 60 + rate - 1) // rate
    lines = []
    end = None
    for line in source_lines:
        if not line or line.split()[0] == "tick_rate":
            continue
        if line.startswith("snapshot "):
            continue  # Query export controls its own sampling.
        match = re.fullmatch(r"end\s+(\d+)", line)
        if match:
            end = ticks(match[1])
            line = f"end {end}"
        match = re.match(r"(at|by)\s+(\d+)\s+(.*)", line)
        if match:
            body = match[3]
            if re.match(r"input\s+(hold|wait-until)\s", body):
                body = re.sub(r"\d+$", lambda m: str(ticks(m[0])), body)
            line = f"{match[1]} {ticks(match[2])} {body}"
        if line.startswith("save "):
            line = "save " + str((source.parent / line[5:]).resolve())
        lines.append(line)
    if end is None or not 0 <= end <= 1_000_000:
        raise ValueError("scenario needs end in 0..1000000")
    lines.insert(1, "tick_rate 60")
    return "\n".join(lines) + "\n", end


def run_command(command, out, timeout, perf=False, acceptable=(0,)):
    out = Path(out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, D2RUST_HOME=str(out / "user"), D2RUST_LOG_DIR=str(out / "logs"))
    env.pop("D2_PERF_OUT", None)
    if perf:
        env["D2_PERF_OUT"] = str(out / "perf.json")
    result = subprocess.run([str(x) for x in command], env=env, capture_output=True,
                            text=True, timeout=timeout)
    (out / "process.log").write_text(result.stdout + result.stderr, encoding="utf-8")
    if result.returncode not in acceptable:
        raise RuntimeError(f"run exited {result.returncode}; see {out / 'process.log'}")
    return result.returncode


def run_scenario(exe, scenario, out, game_dir=None, timeout=120, perf=False):
    out = Path(out).resolve()
    # Refuse reuse: stale reports can never turn a failed run into a pass.
    out.mkdir(parents=True, exist_ok=False)
    text, end = normalized_scenario(scenario)
    script = out / "case.scn"
    script.write_text(text, encoding="utf-8")
    snapshots = {}
    origin = None
    wall = time.monotonic()
    outcome = None
    # Bound --at below Windows' command length limit. Repeated runs are also
    # checked for a matching starting frame; no mutation or extra input is used.
    for chunk, begin in enumerate(range(0, end + 1, 500)):
        folder = out / f"chunk-{chunk}"
        expected = sorted({0, max(0, begin - 1), *range(begin, min(begin + 500, end + 1))})
        cmd = [exe, "repro", script, "--out", folder, "--keep-going", "--query",
               "server.f,server.seed,server.units", "--at",
               ",".join(str(t) for t in expected)]
        if game_dir:
            cmd += ["--game-dir", game_dir]
        code = run_command(cmd, folder, timeout, perf, acceptable=(0, 1))
        report = read(folder / "repro.json")
        if (report.get("tick_rate") != 60 or report.get("ticks") != end or
                report.get("exit") != code or report.get("verdict") != ("pass" if code == 0 else "fail")):
            raise ValueError(f"incomplete/unsupported repro report in {folder}")
        this_outcome = {"exit": code, "first_failure": report.get("first_failure")}
        if outcome is not None and outcome != this_outcome:
            raise RuntimeError("replay verdict changed between query chunks")
        outcome = this_outcome
        if origin is not None and origin != report["start_frame"]:
            raise ValueError("replay start frame changed between query chunks")
        origin = report["start_frame"]
        answers = report["queries"]
        if [q["at"] for q in answers] != expected:
            raise ValueError("missing query ticks")
        for q in answers:
            v = q["values"]
            if v["server.f"] != origin + q["at"] or not isinstance(v["server.seed"], list):
                raise ValueError("invalid server query")
            snap = {"k": "snap", "f": v["server.f"], "seed": v["server.seed"],
                    "units": v["server.units"]}
            if q["at"] in snapshots and snapshots[q["at"]] != snap:
                raise RuntimeError("replay changed at an overlapping query tick")
            snapshots[q["at"]] = snap
    dump = out / "state.jsonl"
    header = {"k": "header", "format": "state-1", "tick_rate": 60, "start_frame": origin}
    dump.write_text("\n".join(json.dumps(x) for x in [header, *[snapshots[t] for t in sorted(snapshots)]])
                    + "\n", encoding="utf-8")
    write(out / "run.json", {"binary": str(exe), "scenario": str(scenario),
                             "ticks": end, "wall_s": time.monotonic() - wall, **outcome})
    return dump


def scenario_files(source):
    path = Path(source).resolve()
    if path.is_dir():
        files = sorted(path.rglob("*.scn"))
    elif path.suffix == ".scn":
        files = [path]
    else:
        files = [(path.parent / line.strip()).resolve() for line in path.read_text(encoding="utf-8").splitlines()
                 if line.strip() and not line.lstrip().startswith("#")]
    if not files or any(not p.is_file() for p in files):
        raise ValueError("no scenarios or missing scenario file")
    return files


def perf_value(path, metric="server.p95"):
    phase, stat = metric.split(".")
    report = read(path)
    value = report.get("summary", report)["phases_us"][phase][stat]
    if type(value) not in (int, float) or not math.isfinite(value) or value < 0:
        raise ValueError("perf metric must be finite and nonnegative")
    return value


def perf_sample(exe, scenario, out, game_dir, timeout, repeats, metric):
    wall = time.monotonic()
    values = []
    for i in range(repeats):
        folder = Path(out) / str(i)
        cmd = [exe, "repro", Path(scenario).resolve(), "--out", folder]
        if game_dir:
            cmd += ["--game-dir", game_dir]
        if folder.exists():
            raise ValueError(f"output already exists: {folder}")
        run_command(cmd, folder, timeout, perf=True)
        values.append(perf_value(folder / "perf.json", metric))
    return {"median_us": statistics.median(values), "samples_us": values,
            "wall_s": time.monotonic() - wall}


def parser():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("a")
    p.add_argument("b")
    p.add_argument("scenarios", nargs="?", help="case, directory or list; defaults to bug scenarios")
    p.add_argument("--dumps", action="store_true", help="A and B are state-1 JSONL files")
    p.add_argument("--equal-time", action="store_true", help="compare different rates at shared exact times")
    p.add_argument("--ignore", action="append", default=[], help="comma-separated field names/paths")
    p.add_argument("--context", type=int, default=2)
    p.add_argument("--out", type=Path, required=True)
    p.add_argument("--game-dir")
    p.add_argument("--timeout", type=float, default=120)
    p.add_argument("--profile", choices=("release", "dist"), default="release")
    p.add_argument("--replay", type=int, default=0, help="additional replays of each build")
    p.add_argument("--sweep", help="generate FAMILY once with B, then replay its saved cases on A and B")
    p.add_argument("--perf", action="store_true", help="performance trend instead of state comparison")
    p.add_argument("--repeats", type=int, default=3)
    p.add_argument("--metric", default="server.p95")
    p.add_argument("--threshold", type=float, default=10, help="allowed percentage increase")
    return p


def main(argv=None):
    p = parser()
    args = p.parse_args(argv)
    try:
        if (args.context < 0 or args.replay < 0 or args.repeats < 1 or
                not math.isfinite(args.timeout) or args.timeout <= 0 or
                not math.isfinite(args.threshold) or args.threshold < 0):
            raise ValueError("invalid context/replay/repeats/timeout/threshold")
        ignore = [f for group in args.ignore for f in group.split(",") if f]
        if args.dumps and (args.perf or args.replay or args.sweep or args.scenarios):
            raise ValueError("--dumps cannot run scenarios, sweeps, replays or perf sampling")
        if args.perf and args.replay:
            raise ValueError("--replay checks state determinism; use it separately from --perf")
        if args.dumps:
            result = compare(args.a, args.b, ignore, args.context, args.equal_time)
        else:
            if args.equal_time:
                raise ValueError("--equal-time requires --dumps from state-dump --tick-rate HZ")
            exes = [binary(args.a, profile=args.profile), binary(args.b, profile=args.profile)]
            source = Path(args.scenarios or ROOT / "tools/scenarios/bugs")
            if args.sweep:
                if args.scenarios:
                    raise ValueError("use SCENARIOS or --sweep FAMILY, not both")
                if not re.fullmatch(r"[a-zA-Z0-9_-]+", args.sweep):
                    raise ValueError("invalid sweep family")
                generated = (args.out / "generated").resolve()
                if generated.exists():
                    raise ValueError(f"output already exists: {generated}")
                command = [exes[1], "sweep", args.sweep, "--out", generated]
                if args.game_dir:
                    command += ["--game-dir", args.game_dir]
                code = run_command(command, generated, args.timeout, acceptable=(0, 1))
                report = read(generated / "sweep.json")
                if report.get("format") != "sweep 1" or report.get("exit") != code:
                    raise ValueError("incomplete/unsupported sweep report")
                source = generated / "scenarios"
            files = scenario_files(source)
            rows = []
            for i, scenario in enumerate(files):
                folder = args.out / f"case-{i:04}"
                if args.perf:
                    samples = [perf_sample(e, scenario, folder / side, args.game_dir,
                                           args.timeout, args.repeats, args.metric)
                               for side, e in zip(("a", "b"), exes)]
                    base, head = [s["median_us"] for s in samples]
                    row = {"same": head <= base * (1 + args.threshold / 100),
                           "metric": args.metric, "threshold_percent": args.threshold,
                           "change_percent": (head - base) / base * 100 if base else None,
                           "a": samples[0], "b": samples[1]}
                else:
                    dumps = [run_scenario(e, scenario, folder / side, args.game_dir, args.timeout)
                             for side, e in zip(("a", "b"), exes)]
                    row = compare(*dumps, ignore, args.context)
                    row["failures"] = [dict(read(d.parent / "run.json"), side=side)
                                       for side, d in zip(("a", "b"), dumps)
                                       if read(d.parent / "run.json")["exit"] != 0]
                    row["same"] &= not row["failures"]
                    row["replays"] = []
                    for side, exe, baseline in zip(("a", "b"), exes, dumps):
                        for n in range(args.replay):
                            replay = run_scenario(exe, scenario, folder / f"{side}-replay-{n}",
                                                  args.game_dir, args.timeout)
                            check = compare(baseline, replay, ignore, args.context)
                            replay_outcome = read(replay.parent / "run.json")
                            if replay_outcome["exit"] != 0:
                                if check["same"]:
                                    check["field"] = "repro.first_failure"
                                check.update(same=False, first_failure=replay_outcome["first_failure"])
                            row["replays"].append(dict(check, side=side, replay=n))
                            row["same"] &= check["same"]
                rows.append(dict(row, scenario=str(scenario)))
            result = {"same": all(r["same"] for r in rows), "cases": rows,
                      "builds": [{"input": value, "binary": str(exe)}
                                 for value, exe in zip((args.a, args.b), exes)]}
        write(args.out / "bdiff.json", result)
        print("same" if result["same"] else "differs")
        if not result["same"]:
            for row in result.get("cases", [result]):
                if not row["same"]:
                    detail = {k: v for k, v in row.items() if k not in ("context", "replays")}
                    print(json.dumps(detail))
        print(f"report: {args.out / 'bdiff.json'}")
        return 0 if result["same"] else 1
    except (OSError, ValueError, KeyError, TypeError, RuntimeError, subprocess.SubprocessError) as error:
        print(f"bdiff: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
