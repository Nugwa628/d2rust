#!/usr/bin/env python3
"""Frame pacing per display setting: runs the game windowed under Xvfb (or
hidden with --offscreen on Windows) once per VSync / frame limit / animation
frames choice, with D2_PERF_OUT timing on, and checks from the client frame
starts and the server ticks of each frame:

- cap: with a frame limit in force, the frame rate is within --tol percent
  of the limit (the limiter's job; a run whose frames take longer than the
  period cannot reach it and is marked "slow", not failed);
- jitter: the standard deviation of the frame interval, and the share of
  frames shorter than 0.9 x the period (a frame let through early);
- skips: the share of frames longer than 1.5 x the period (a missed slot);
- ticks per frame: how many server ticks each frame ran (0: the frame
  repeats the last picture, 2+: a picture skipped a tick).

    python3 tools/perf/frame_pacing.py [--bin DIR] [--cases LIST] [--frames N]
        [--scene a1-cold-plains] [--out DIR] [--json FILE] [--tol 2]
    python3 tools/perf/frame_pacing.py --selftest

A case is `v` (VSync on) or `n` (off), the limit (0: Off), and `s` / `o`
for Smooth / Original animation frames: `n60s` is VSync off, 60, Smooth.
--cases defaults to every limit with VSync off plus `v0s`, all Smooth, and
`n0o`, `n60o` for the Original comparison. Needs D2_GAME_DIR. Standard
library only. Exit 1 when a case misses its cap or skips too many frames.
"""

import argparse
import json
import math
import os
import shutil
import subprocess
import sys
import time

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
SEED = 1234
LIMITS = [0, 30, 60, 120, 144, 165, 240, 360]
DEFAULT_CASES = [f"n{l}s" for l in LIMITS] + ["v0s", "v60s", "n0o", "n60o"]
WARM_FRAMES = 120
# Moves the player back and forth across the screen so units and the
# camera move in every frame: a held click to the right, then the left.
WALK = "frame 40; " + "; ".join(
    f"hold {x} 300 {n}" for x, n in [(700, 150), (100, 150)] * 6
)
SCENES = {
    "a1-town": (1, None),
    "a1-cold-plains": (1, 3),
}


def parse_case(c):
    if len(c) < 3 or c[0] not in "vn" or c[-1] not in "so" or not c[1:-1].isdigit():
        raise ValueError(f"bad case {c!r}: like n60s (v/n, limit, s/o)")
    limit = int(c[1:-1])
    if limit not in LIMITS:
        raise ValueError(f"bad case {c!r}: limit one of {LIMITS}")
    return {"vsync": c[0] == "v", "limit": limit,
            "animation": "smooth" if c[-1] == "s" else "original"}


def settings_toml(case, window="1068x600"):
    return ("version = 7\n[video]\n"
            f"window_mode = \"windowed\"\nwindow_size = \"{window}\"\n"
            f"vsync = {'true' if case['vsync'] else 'false'}\n"
            f"frame_limit = {case['limit']}\n"
            f"animation_frames = \"{case['animation']}\"\n")


def pct(values, p):
    if not values:
        return 0
    v = sorted(values)
    k = max(0, min(len(v) - 1, -(-p * len(v) // 100) - 1))
    return v[k]


def analyse(report, case, warm=WARM_FRAMES, tol=2.0):
    """The pacing numbers of one `perf 2` report (frames after `warm`)."""
    frames = report["frames"][warm:]
    if len(frames) < 10:
        return {"error": f"only {len(frames)} frames after warm-up"}
    iv = [f[0] for f in frames]
    starts = [f[4] for f in frames]
    span_us = starts[-1] + iv[-1] - starts[0]
    fps = len(frames) / (span_us / 1e6)
    mean = sum(iv) / len(iv)
    sd = math.sqrt(sum((x - mean) ** 2 for x in iv) / len(iv))
    # Ticks per frame: each tick's drain start falls in one frame.
    tick_at = sorted(t[4] for t in report["ticks"] if len(t) > 4 and t[4])
    per_frame = []
    j = 0
    for f in frames:
        lo, hi = f[4], f[4] + f[0]
        while j < len(tick_at) and tick_at[j] < lo:
            j += 1
        n = 0
        while j < len(tick_at) and tick_at[j] < hi:
            n += 1
            j += 1
        per_frame.append(n)
    hist = {}
    for n in per_frame:
        k = str(min(n, 3)) if n < 3 else "3+"
        hist[k] = hist.get(k, 0) + 1
    work = [f[1] + f[2] + f[3] for f in frames]
    out = {
        "frames": len(frames), "fps": round(fps, 2),
        "interval_ms": {"p50": pct(iv, 50) / 1e3, "p95": pct(iv, 95) / 1e3,
                        "p99": pct(iv, 99) / 1e3, "max": max(iv) / 1e3,
                        "sd": round(sd / 1e3, 3)},
        "work_ms_p50": pct(work, 50) / 1e3,
        "render_ms_p50": pct(report.get("render", [])[warm:], 50) / 1e3,
        "ticks_per_frame": {k: round(v / len(per_frame), 4) for k, v in sorted(hist.items())},
        # Frames with a new tick picture (DREW) / a between-tick motion
        # picture (MOVED), per second.
        "drawn_per_s": round(sum(1 for f in frames if f[5] & 1) / (span_us / 1e6), 2),
        "moved_per_s": round(sum(1 for f in frames if f[5] & 2) / (span_us / 1e6), 2),
        "tick_rate": round(len([t for t in tick_at if starts[0] <= t < starts[-1] + iv[-1]]) / (span_us / 1e6), 2),
    }
    limit = case["limit"] if not case["vsync"] else 0
    if limit:
        period = 1e6 / limit
        early = sum(1 for x in iv if x < 0.9 * period) / len(iv)
        late = sum(1 for x in iv if x > 1.5 * period) / len(iv)
        out["early"] = round(early, 4)
        out["late"] = round(late, 4)
        # The machine cannot reach the limit when frames take longer than
        # the period by themselves.
        render = report.get("render", [])[warm:]
        slow = max(pct(work, 50), pct(render, 50)) > 0.9 * period
        err = abs(fps - limit) / limit * 100
        out["cap_err_pct"] = round(err, 2)
        out["verdict"] = "slow" if slow and fps < limit else (
            "ok" if err <= tol and late <= 0.05 and early <= 0.05 else "FAIL")
    else:
        out["verdict"] = "info"
    return out


def run_case(a, case_name, save, out_dir):
    case = parse_case(case_name)
    home = os.path.join(out_dir, case_name)
    saves = os.path.join(home, "saves")
    os.makedirs(saves, exist_ok=True)
    dst = os.path.join(saves, os.path.basename(save))
    shutil.copyfile(save, dst)
    with open(os.path.join(home, "settings.toml"), "w") as f:
        f.write(settings_toml(case, a.window))
    perf = os.path.join(home, "perf.json")
    env = dict(os.environ, D2_PERF_OUT=perf)
    env.pop("D2_COVERAGE_DIR", None)
    act, level = SCENES[a.scene]
    pokes = ["--poke", f"{a.warp_frame} warp {level}"] if level is not None else []
    game = os.path.join(a.bin, a.exe)
    cmd = [game, "play", "--save", dst, "--seed", str(SEED),
           "--frames", str(a.frames), "--input", WALK] + pokes
    if a.offscreen:
        cmd += ["--offscreen"]
    elif a.display:
        env["DISPLAY"] = a.display
    else:
        return {"error": "no X display (Xvfb not found)"}
    t = time.time()
    with open(os.path.join(home, "run.log"), "w") as log:
        rc = subprocess.run(cmd, env=env, stdout=log, stderr=subprocess.STDOUT,
                            timeout=a.timeout).returncode
    if not os.path.exists(perf):
        return {"error": f"exit {rc}, no report; see {home}/run.log"}
    with open(perf) as f:
        r = analyse(json.load(f), case, tol=a.tol)
    r["wall_s"] = round(time.time() - t, 1)
    if rc != 0:
        r["exit"] = rc
    return r


def start_xvfb():
    """An Xvfb server of our own on a free display (no xauth needed), or
    (None, None) when Xvfb is not installed."""
    exe = shutil.which("Xvfb")
    if not exe:
        return None, None
    for n in range(90 + os.getpid() % 100, 400):
        if os.path.exists(f"/tmp/.X{n}-lock"):
            continue
        p = subprocess.Popen([exe, f":{n}", "-screen", "0", "1280x800x24", "-nolisten", "tcp"],
                             stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        for _ in range(50):
            if os.path.exists(f"/tmp/.X11-unix/X{n}"):
                return p, f":{n}"
            if p.poll() is not None:
                break
            time.sleep(0.1)
        p.kill()
    return None, None


def make_save(bin_dir, out_dir):
    path = os.path.join(out_dir, "Pacing.d2s")
    if os.path.exists(path):
        return path
    cmd = [os.path.join(bin_dir, "d2s-tool"), "new", "--name", "Pacing", "--class", "sor",
           "--expansion", "--level", "30", "--waypoints", "all",
           "--stat", "7=128000", "--stat", "6=128000", "-o", path]
    subprocess.run(cmd, check=True, capture_output=True)
    return path


def selftest():
    period = 1e6 / 60
    frames = [[int(period), 1000, 1000, 1000, 1000 + int(i * period), 1] for i in range(300)]
    ticks = [[0, 1, 1, 1, 1000 + int(i * period) + 5] for i in range(300)]
    r = analyse({"frames": frames, "ticks": ticks, "render": []},
                parse_case("n60s"), warm=10)
    assert r["verdict"] == "ok", r
    assert r["ticks_per_frame"] == {"1": 1.0}, r
    # Every other frame twice as long: 40 fps, half the frames late.
    frames = []
    at = 0
    for i in range(300):
        d = int(period * (2 if i % 2 else 1))
        frames.append([d, 100, 100, 100, at, 1])
        at += d
    r = analyse({"frames": frames, "ticks": [], "render": []}, parse_case("n60s"), warm=10)
    assert r["verdict"] == "FAIL" and r["late"] > 0.4, r
    for bad in ["x60s", "n61s", "n60", "60s"]:
        try:
            parse_case(bad)
        except ValueError:
            continue
        raise AssertionError(bad)
    print("selftest ok")
    return 0


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--bin", default=os.path.join(REPO, "target", "release"))
    ap.add_argument("--cases", default=",".join(DEFAULT_CASES))
    ap.add_argument("--frames", type=int, default=900)
    ap.add_argument("--scene", default="a1-cold-plains", choices=sorted(SCENES))
    ap.add_argument("--out", default="frame-pacing-out")
    ap.add_argument("--window", default="1068x600", help="window size (1x: least GPU work)")
    ap.add_argument("--warp-frame", type=int, default=4)
    ap.add_argument("--save", help="a .d2s to play (default: made with d2s-tool from --bin)")
    ap.add_argument("--exe", default="D2RUST-Game", help="the game's file name in --bin")
    ap.add_argument("--json")
    ap.add_argument("--tol", type=float, default=2.0, help="cap tolerance, percent")
    ap.add_argument("--timeout", type=int, default=600)
    ap.add_argument("--offscreen", action="store_true", help="hidden window, no Xvfb")
    ap.add_argument("--selftest", action="store_true")
    a = ap.parse_args(argv)
    if a.selftest:
        return selftest()
    cases = [c for c in a.cases.split(",") if c]
    for c in cases:
        parse_case(c)
    if not os.environ.get("D2_GAME_DIR"):
        print("frame_pacing: D2_GAME_DIR is not set", file=sys.stderr)
        return 3
    os.makedirs(a.out, exist_ok=True)
    save = a.save or make_save(a.bin, a.out)
    xvfb = None
    a.display = None
    if not a.offscreen:
        xvfb, a.display = start_xvfb()
    results = {}
    bad = 0
    for c in cases:
        r = run_case(a, c, save, a.out)
        results[c] = r
        bad += r.get("verdict") == "FAIL" or "error" in r
        print(f"{c}: {json.dumps(r)}", flush=True)
    if xvfb:
        xvfb.terminate()
        xvfb.wait()
    if a.json:
        with open(a.json, "w") as f:
            json.dump({"format": "frame-pacing 1", "scene": a.scene, "results": results}, f, indent=1)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
