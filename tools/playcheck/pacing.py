"""Frame pacing check: the frame limit holds, frames come evenly, no stalls, no skipped ticks.

Runs the real game offscreen once per setting (frame limit x animation
frames, VSync off so the game's own limiter is in force) with timing on
(`D2_PERF_OUT`, the `perf 2` report), then reads the per-frame log:

- frames: `[interval, pre, update, post, start_us, flags]` in microseconds;
- ticks: `[game_frame, drain, tick, flush, at_us]`, on the same clock;
- render: one render-world time per frame.

Stated rules (owner reports 2026-10-19: the limit doesn't hold, settings
don't apply, Smooth runs worse, frames skip at low fps):

- `cap-exceeded`: the measured rate is above the limit (more than `--rate-tol`
  percent), or a one-second window holds more frames than the limit allows.
- `cap-missed`: the machine has headroom (the p95 frame cost, the larger of
  the frame's update work and its render time, is under 80 % of one period)
  but the rate stays more than 5 % below the limit.
- `jitter`: with the limit held, frame intervals spread more than
  `--jitter-ms` (standard deviation) or the p99 interval is over 1.5 periods.
- `stall`: a frame interval over `--stall-ms` and four periods (or four
  median intervals with no limit) after the warm-up.
- `skipped-ticks`: a frame ran more game ticks than the frame rate needs
  (`ceil(tick_rate / fps)`: one at 60 fps and up, two at 30), so a tick's
  picture was never shown, in more than `--skip-pct` percent of frames.
- `smooth-costs-more`: with the same limit, Smooth frames have a p95 interval
  more than 15 % (plus 1 ms) above Original's.

When the machine is too slow for a limit (llvmpipe on the CI server at large
sizes), the limit is reported `unreachable` and only `cap-exceeded` and
`stall` apply. Timing verdicts belong to one machine at a time: compare
runs on the same machine and load, and rerun a borderline failure.
"""
import math
from pathlib import Path

try:
    from . import offscreen
except ImportError:  # run as a script
    import offscreen

TICK_RATE = 60


def pct(values, p):
    """Nearest-rank percentile; 0 for an empty list."""
    if not values:
        return 0.0
    s = sorted(values)
    return s[min(len(s) - 1, max(0, math.ceil(p / 100 * len(s)) - 1))]


def stdev(values):
    if len(values) < 2:
        return 0.0
    m = sum(values) / len(values)
    return math.sqrt(sum((v - m) ** 2 for v in values) / (len(values) - 1))


def analyse(perf, cap, warm_s=3.0, rate_tol=2.0, jitter_ms=1.5, stall_ms=50.0, skip_pct=2.0,
            tick_rate=TICK_RATE):
    """Metrics and rule violations for one run's `perf 2` report.

    `cap` is the frame limit in force (None or 0: no limit)."""
    frames = perf.get("frames") or []
    ticks = perf.get("ticks") or []
    render = perf.get("render") or []
    if len(frames) < 10:
        return {"error": f"only {len(frames)} frames recorded"}
    # Warm-up: from the first tick on, skip `warm_s` seconds (loading, the
    # first atlas pages). Drop the last frames too (leaving, the save).
    first_tick = min((t[4] for t in ticks if t[4]), default=frames[0][4])
    begin = first_tick + int(warm_s * 1e6)
    keep = [i for i, f in enumerate(frames[:-3]) if f[4] >= begin]
    if len(keep) < 10:
        return {"error": f"only {len(keep)} frames after the {warm_s} s warm-up"}
    rows = [frames[i] for i in keep]
    intervals = [f[0] / 1000 for f in rows]
    seconds = sum(intervals) / 1000
    fps = len(rows) / seconds
    # Render times have no frame index; they are one per frame in order.
    rend = [r / 1000 for r in render[keep[0]:keep[-1] + 1]] if len(render) >= len(frames) - 1 else []
    work = [(f[2] + f[3]) / 1000 for f in rows]
    cost = [max(w, rend[i] if i < len(rend) else 0.0) for i, w in enumerate(work)]
    # Ticks per frame, by time: a tick at `at_us` belongs to the frame whose
    # interval holds it.
    starts = [f[4] for f in rows]
    ends = [f[4] + f[0] for f in rows]
    per_frame = [0] * len(rows)
    j = 0
    for at in sorted(t[4] for t in ticks if t[4]):
        while j < len(rows) and at >= ends[j]:
            j += 1
        if j == len(rows):
            break
        if at >= starts[j]:
            per_frame[j] += 1
    period = 1000 / cap if cap else None
    m = {
        "frames": len(rows), "seconds": round(seconds, 3), "fps": round(fps, 2),
        "interval_ms": {"p50": round(pct(intervals, 50), 3), "p95": round(pct(intervals, 95), 3),
                        "p99": round(pct(intervals, 99), 3), "max": round(max(intervals), 3),
                        "stdev": round(stdev(intervals), 3)},
        "cost_ms": {"p50": round(pct(cost, 50), 3), "p95": round(pct(cost, 95), 3)},
        "render_ms": {"p50": round(pct(rend, 50), 3), "p95": round(pct(rend, 95), 3)} if rend else None,
        "ticks_per_s": round(sum(per_frame) / seconds, 2),
        "new_picture_frames": sum(1 for f in rows if (f[5] or 0) & 3),
        "still_frames": sum(1 for f in rows if not (f[5] or 0) & 3),
    }
    v = []
    reachable = None
    if cap:
        reachable = pct(cost, 95) < 0.8 * period
        m["cap"] = cap
        m["cap_reachable"] = reachable
        if fps > cap * (1 + rate_tol / 100):
            v.append({"rule": "cap-exceeded", "fps": round(fps, 2), "cap": cap})
        # One-second windows: a burst above the limit counts even when the
        # average holds.
        burst, w0, n = 0, 0, 0
        for i, s in enumerate(starts):
            while s - starts[w0] >= 1_000_000:
                w0 += 1
            n = i - w0 + 1
            burst = max(burst, n)
        if burst > cap * 1.05 + 1:
            v.append({"rule": "cap-exceeded", "window_frames": burst, "cap": cap,
                      "detail": "a one-second window holds more frames than the limit"})
        if reachable:
            if fps < cap * (1 - 5 / 100):
                v.append({"rule": "cap-missed", "fps": round(fps, 2), "cap": cap,
                          "cost_p95_ms": m["cost_ms"]["p95"]})
            # Stalls are their own rule; the spread is of the other frames.
            even = [x for x in intervals if x <= max(stall_ms, 4 * period)]
            m["interval_ms"]["stdev_even"] = round(stdev(even), 3)
            if stdev(even) > jitter_ms or pct(even, 99) > 1.5 * period:
                v.append({"rule": "jitter", "stdev_ms": m["interval_ms"]["stdev_even"],
                          "p99_ms": m["interval_ms"]["p99"], "period_ms": round(period, 3)})
    limit = max(stall_ms, 4 * (period or pct(intervals, 50)))
    stalls = [{"at_s": round((rows[i][4] - first_tick) / 1e6, 3), "ms": round(x, 1)}
              for i, x in enumerate(intervals) if x > limit]
    m["stalls"] = len(stalls)
    if stalls:
        v.append({"rule": "stall", "count": len(stalls), "limit_ms": round(limit, 1),
                  "worst": sorted(stalls, key=lambda s: -s["ms"])[:5]})
    target = min(fps, cap) if cap else fps
    # Ticks the rate needs per frame (2 at 30 fps); a small margin keeps a
    # rate a hair under the tick rate from allowing two.
    allowed = max(1, math.ceil(tick_rate / max(target, 1) - 0.05))
    skipped = sum(1 for k in per_frame if k > allowed)
    m["ticks_per_frame"] = {str(k): per_frame.count(k) for k in sorted(set(per_frame))}
    if skipped * 100 > skip_pct * len(rows):
        v.append({"rule": "skipped-ticks", "frames": skipped, "of": len(rows),
                  "allowed_ticks_per_frame": allowed})
    return {"metrics": m, "violations": v}


def compare_modes(results):
    """`smooth-costs-more` across runs with the same limit."""
    by = {}
    for r in results:
        if "metrics" in r.get("analysis", {}):
            by.setdefault(r["limit"], {})[r["mode"]] = r["analysis"]["metrics"]
    v = []
    for limit, modes in sorted(by.items()):
        if "smooth" in modes and "original" in modes:
            s, o = modes["smooth"]["interval_ms"]["p95"], modes["original"]["interval_ms"]["p95"]
            if s > o * 1.15 + 1:
                v.append({"rule": "smooth-costs-more", "limit": limit, "smooth_p95_ms": s,
                          "original_p95_ms": o, "smooth_fps": modes["smooth"]["fps"],
                          "original_fps": modes["original"]["fps"]})
    return v


def run_one(opts, limit, mode):
    folder = opts.out / f"limit-{limit}-{mode}"
    video = {"vsync": False, "frame_limit": limit, "animation_frames": mode}
    run = offscreen.Run(opts.bin, folder, ["--new", "sorceress", "Pacing", "--seed", "7",
                                           "--frames", str(opts.frames), *opts.play_args],
                        video=video, size=opts.size, game_dir=opts.game_dir)
    run.start()
    rc = run.wait(opts.timeout)
    perf = run.perf_report()
    result = {"limit": limit, "mode": mode, "exit": rc, "folder": str(folder)}
    if perf is None:
        result["analysis"] = {"error": f"no perf report (exit {rc}); see {folder}/stdout.log"}
    else:
        result["analysis"] = analyse(perf, limit or None, warm_s=opts.warm, rate_tol=opts.rate_tol,
                                     jitter_ms=opts.jitter_ms, stall_ms=opts.stall_ms,
                                     skip_pct=opts.skip_pct)
    return result


def add_args(p):
    p.add_argument("--bin", required=True, help="the D2RUST-Game executable")
    p.add_argument("--game-dir", help="game files (default: D2_GAME_DIR)")
    p.add_argument("--out", required=True, type=Path, help="a new folder for runs and verdict.json")
    p.add_argument("--limits", default="30,60,120,0", help="frame limits to run (0: Off)")
    p.add_argument("--modes", default="original,smooth", help="animation frames settings")
    p.add_argument("--frames", type=int, default=900, help="frames per run (incl. loading)")
    p.add_argument("--size", default="800x600", help="hidden window size")
    p.add_argument("--warm", type=float, default=3.0, help="seconds skipped after the first tick")
    p.add_argument("--rate-tol", type=float, default=2.0)
    p.add_argument("--jitter-ms", type=float, default=1.5)
    p.add_argument("--stall-ms", type=float, default=50.0)
    p.add_argument("--skip-pct", type=float, default=2.0)
    p.add_argument("--jobs", type=int, default=1,
                   help="runs at once (timing: keep 1 unless the machine is idle)")
    p.add_argument("--timeout", type=float, default=600)
    p.add_argument("--play-arg", dest="play_args", action="append", default=[],
                   help="extra play argument (repeatable), e.g. --play-arg=--poke --play-arg='30 warp 2'")


def main(opts):
    opts.out.mkdir(parents=True, exist_ok=False)
    configs = [(int(l), m) for l in opts.limits.split(",") for m in opts.modes.split(",")]
    results = offscreen.jobs_map(lambda c: run_one(opts, *c), configs, opts.jobs)
    violations = []
    for r in results:
        a = r["analysis"]
        if "error" in a:
            violations.append({"rule": "run-error", "limit": r["limit"], "mode": r["mode"],
                               "detail": a["error"]})
        for x in a.get("violations", []):
            violations.append(dict(x, limit=r["limit"], mode=r["mode"]))
    violations += compare_modes(results)
    verdict = {"format": "playcheck-pacing 1", "pass": not violations, "violations": violations,
               "runs": results, "size": opts.size, "frames": opts.frames}
    offscreen.write_json(opts.out / "verdict.json", verdict)
    print("limit\tmode\tfps\tp50\tp99\tstdev\tstalls\treachable\tviolations")
    for r in results:
        m = r["analysis"].get("metrics", {})
        iv = m.get("interval_ms", {})
        print(f"{r['limit'] or 'off'}\t{r['mode']}\t{m.get('fps')}\t{iv.get('p50')}\t{iv.get('p99')}\t"
              f"{iv.get('stdev')}\t{m.get('stalls')}\t{m.get('cap_reachable', '-')}\t"
              + ",".join(x["rule"] for x in r["analysis"].get("violations", []))
              + r["analysis"].get("error", ""))
    for x in compare_modes(results):
        print(f"{x['rule']}: limit {x['limit']}: smooth p95 {x['smooth_p95_ms']} ms vs original {x['original_p95_ms']} ms")
    print(f"pacing: {'pass' if verdict['pass'] else 'FAIL'} ({len(violations)} violations); {opts.out / 'verdict.json'}")
    return 0 if verdict["pass"] else 1
