#!/usr/bin/env python3
"""Find the first changed first-parent commit, using Q1's queued binary cache.
Assumes a monotone predicate. Endpoint errors stop the search, never count bad.
"""
import argparse
import json
import math
from pathlib import Path
import subprocess
import sys

def history(repo, good, bad):
    def git(*args):
        return subprocess.check_output(["git", *args], cwd=repo, text=True).strip()
    good = git("rev-parse", "--verify", good + "^{commit}")
    bad = git("rev-parse", "--verify", bad + "^{commit}")
    commits = git("rev-list", "--first-parent", bad).splitlines()
    if good not in commits or good == bad:
        raise ValueError("good must precede bad on its first-parent history")
    return list(reversed(commits[:commits.index(good) + 1]))


def search(commits, is_bad):
    results = {}
    def probe(i):
        if i not in results:
            results[i] = bool(is_bad(commits[i]))
        return results[i]
    if probe(0):
        raise ValueError("good endpoint is bad")
    if not probe(len(commits)-1):
        raise ValueError("bad endpoint is good")
    lo, hi = 0, len(commits)-1
    while hi - lo > 1:
        mid = (lo + hi) // 2
        if probe(mid):
            hi = mid
        else:
            lo = mid
    return {"first_bad": commits[hi], "previous_good": commits[lo],
            "probes": [{"sha": commits[i], "bad": bad} for i, bad in sorted(results.items())],
            "assumption": "monotone predicate on first-parent history"}


def main(argv=None):
    try:
        from . import bdiff
    except ImportError:
        import bdiff
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("scenario", type=Path)
    p.add_argument("--good", required=True)
    p.add_argument("--bad", required=True)
    p.add_argument("--out", required=True, type=Path)
    p.add_argument("--repo", type=Path, default=bdiff.ROOT)
    p.add_argument("--game-dir")
    p.add_argument("--ignore", action="append", default=[])
    p.add_argument("--timeout", type=float, default=120)
    p.add_argument("--perf", action="store_true")
    p.add_argument("--metric", default="server.p95")
    p.add_argument("--threshold", type=float, default=10)
    p.add_argument("--repeats", type=int, default=3)
    args = p.parse_args(argv)
    try:
        if (not math.isfinite(args.timeout) or args.timeout <= 0 or args.repeats < 1
                or not math.isfinite(args.threshold) or args.threshold < 0):
            raise ValueError("invalid timeout/repeats/threshold")
        commits = history(args.repo, args.good, args.bad)
        cache = bdiff.Cache(repo=args.repo)
        baseline = None
        def probe(sha):
            nonlocal baseline
            exe = bdiff.binary(sha, cache)
            folder = args.out / sha
            if args.perf:
                sample = bdiff.perf_sample(exe, args.scenario, folder, args.game_dir,
                                          args.timeout, args.repeats, args.metric)
                bdiff.write(folder / "measurement.json", sample)
                if baseline is None:
                    baseline = sample["median_us"]
                return sample["median_us"] > baseline * (1 + args.threshold / 100)
            dump = bdiff.run_scenario(exe, args.scenario, folder, args.game_dir, args.timeout)
            if baseline is None:
                baseline = dump
            result = bdiff.compare(baseline, dump, [f for g in args.ignore for f in g.split(",")])
            result["scenario"] = bdiff.read(dump.parent / "run.json")
            bdiff.write(folder / "comparison.json", result)
            return not result["same"] or result["scenario"]["exit"] != 0
        result = search(commits, probe)
        result["predicate"] = ({"mode": "perf", "metric": args.metric,
                                "threshold_percent": args.threshold, "repeats": args.repeats}
                               if args.perf else {"mode": "behaviour", "ignore": args.ignore})
        bdiff.write(args.out / "bisect.json", result)
        print(json.dumps(result, indent=2))
        return 0
    except (OSError, ValueError, KeyError, TypeError, RuntimeError, subprocess.SubprocessError) as error:
        print(f"bisect: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
