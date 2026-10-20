#!/usr/bin/env python3
"""Frame time and memory per scene: runs d2-client headless (state-dump) and
windowed under Xvfb (play) through each act's town and a busy outdoor
level, with D2_PERF_OUT timing on, and writes p50 / p95 / max per scene
against the budget (default 60 server ticks/s: a tick <= 16 ms; a client frame
<= 16 ms), the peak RSS and, with --profile, the top hot spots (perf).
Headless --tick-rate selects simulation Hz and the corresponding tick budget.
The slowest ticks include cold setup and named host timing spans when available.

    python3 tools/perf/perf.py [--bin DIR] [--scenes a1-town,...] [--ticks N] [--tick-rate HZ]
        [--frames N] [--no-window] [--profile SCENE] [--out DIR] [--md FILE]
    python3 tools/perf/perf.py --selftest

--bin: a folder with d2-client and d2s-tool built WITHOUT the coverage-map
feature (default target/perf/release). Needs D2_GAME_DIR. Standard library
only. Our own code.
"""

import argparse
import json
import os
import platform
import shutil
import subprocess
import sys
import time

VERSION = "0.2.0"
REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
TARGET_HZ = 60
BUDGET_US = 16_000  # S10e: 40% of the 25 Hz tick budget.
WARM_TICKS = 96     # steady statistics only; cold ticks remain in the gate.
SEED = 1234

# (name, act 1-5, level to warp to or None = the act's town where the save starts)
SCENES = [
    ("a1-town", 1, None, "Rogue Encampment (1)"),
    ("a1-cold-plains", 1, 3, "Cold Plains (3)"),
    ("a2-town", 2, None, "Lut Gholein (40)"),
    ("a2-far-oasis", 2, 43, "Far Oasis (43)"),
    ("a3-town", 3, None, "Kurast Docks (75)"),
    ("a3-spider-forest", 3, 76, "Spider Forest (76)"),
    ("a4-town", 4, None, "Pandemonium Fortress (103)"),
    ("a4-plains", 4, 105, "Plains of Despair (105)"),
    ("a5-town", 5, None, "Harrogath (109)"),
    ("a5-foothills", 5, 110, "Bloody Foothills (110)"),
]


class PerfError(Exception):
    pass


def pct(values, p):
    """Nearest-rank percentile (p in 0..100) of a list; 0 for an empty one."""
    if not values:
        return 0
    v = sorted(values)
    k = max(0, min(len(v) - 1, -(-p * len(v) // 100) - 1))
    return v[k]


def stats(values):
    return {"n": len(values), "mean": sum(values) / len(values) if values else 0,
            "p99": pct(values, 99), "p50": pct(values, 50), "p95": pct(values, 95),
            "max": max(values) if values else 0}


def make_save(bin_dir, act, out_dir):
    path = os.path.join(out_dir, f"PerfA{act}.d2s")
    if os.path.exists(path):
        return path
    cmd = [os.path.join(bin_dir, "d2s-tool"), "new", "--name", f"PerfA{act}", "--class", "sor",
           "--expansion", "--level", "30", "--waypoints", "all",
           "--stat", "7=128000", "--stat", "6=128000"]
    if act > 1:
        cmd += ["--act", str(act - 1), "--quests", f"acts={act - 1}"]
    cmd += ["-o", path]
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode != 0:
        raise PerfError(f"d2s-tool: {r.stderr.strip()}")
    return path


def run(cmd, env, timeout, log):
    t = time.time()
    with open(log, "w") as f:
        f.write("$ " + " ".join(cmd) + "\n")
        f.flush()
        r = subprocess.run(cmd, env=env, stdout=f, stderr=subprocess.STDOUT, timeout=timeout)
    return r.returncode, time.time() - t


def load(path):
    with open(path) as f:
        d = json.load(f)
    if d.get("format") not in ("perf 1", "perf 2"):
        raise PerfError(f"{path}: not a 'perf 1' or 'perf 2' report")
    return d


def summarize(d, warm):
    """Steady-state stats after `warm` ticks / the matching frames."""
    ticks = d["ticks"]
    first = ticks[0][0] if ticks else 0
    steady = [t for t in ticks if t[0] - first >= warm]
    out = {
        "peak_rss_mib": d["peak_rss_kib"] / 1024,
        "ticks_total": len(ticks),
        "tick": stats([t[2] + t[3] for t in steady]),
        "tick_only": stats([t[2] for t in steady]),
        "flush": stats([t[3] for t in steady]),
        "tick_load_max": max((t[2] + t[3] for t in ticks), default=0),
    }
    # Attribute the slowest ticks, including cold setup, to host timing spans.
    # Nested spans are inclusive; their costs must not be added together.
    out["slow_ticks"] = []
    for t in sorted(ticks, key=lambda t: t[2] + t[3], reverse=True)[:3]:
        phases = {}
        if len(t) > 4:
            start, end = t[4] + t[1], t[4] + sum(t[1:4])
            for kind, at, duration in d.get("spans", []):
                if start <= at < end:
                    phases[kind] = phases.get(kind, 0) + min(duration, end - at)
        out["slow_ticks"].append({"frame": t[0], "us": t[2] + t[3],
                                  "spans": sorted(phases.items(), key=lambda p: p[1], reverse=True)[:8]})
    if d.get("bridge"):
        b = d["bridge"][warm:]
        out["bridge"] = stats(b)
    if d.get("frames"):
        # Frames: drop those before the steady state by time share (the
        # same fraction of the run as the warm ticks).
        fr = d["frames"]
        skip = len(fr) * min(warm, len(ticks)) // max(1, len(ticks))
        f = fr[skip:]
        out["frame"] = stats([x[0] for x in f])
        out["pre"] = stats([x[1] for x in f])
        out["update"] = stats([x[2] for x in f])
        out["post"] = stats([x[3] for x in f])
        out["frame_load_max"] = max((x[0] for x in fr), default=0)
        out["fps"] = round(1e6 * len(f) / max(1, sum(x[0] for x in f)), 1)
        out["over_budget"] = sum(1 for x in f if x[0] > BUDGET_US)
    if d.get("render"):
        r = d["render"]
        out["render"] = stats(r[len(r) * min(warm, len(ticks)) // max(1, len(ticks)):])
    s = d.get("summary")
    if s:
        # perf 2: the game's own summary of the whole run (rates, hitches
        # with their causes), read as it is rather than recomputed.
        for k in ("ticks_per_s", "drawn_per_s", "pictures_per_s", "hitch_ms", "hitch_count"):
            out[k] = s.get(k)
        out["hitches"] = s.get("hitches", [])[:10]
    return out


def ms(us):
    return f"{us / 1000:.2f}"


def triple(s, fields=("p50", "p95", "max")):
    return " / ".join(ms(s[f]) for f in fields)


def misses(r):
    """Budget misses of a scene result (spec rule 2)."""
    m = []
    for mode in ("headless", "window"):
        x = r.get(mode)
        if not x:
            continue
        if mode == "headless":
            if x.get("ticks_per_s") is None:
                m.append("headless throughput missing; need a perf 2 report")
            elif x["ticks_per_s"] < TARGET_HZ:
                m.append(f"headless throughput {x['ticks_per_s']} ticks/s < {TARGET_HZ}")
        if TARGET_HZ == 60 and x.get("tick_load_max", 0) > BUDGET_US:
            m.append(f"{mode} tick including load max {ms(x['tick_load_max'])} ms")
        if x["tick"]["max"] > BUDGET_US:
            m.append(f"{mode} tick max {ms(x['tick']['max'])} ms")
        if "frame" in x and x["frame"]["max"] > BUDGET_US:
            m.append(f"{mode} frame max {ms(x['frame']['max'])} ms ({x['over_budget']} frames over)")
    return m


def profile(bin_dir, save, level, ticks, out_dir, env, tick_rate=60):
    perf = shutil.which("perf") or next(
        (os.path.join(d, "perf") for d in sorted(os.listdir("/usr/lib"), reverse=True)
         if d.startswith("linux-tools-") and os.path.exists(os.path.join("/usr/lib", d, "perf"))), None)
    if perf and not os.path.isabs(perf):
        perf = os.path.join("/usr/lib", perf)
    if not perf:
        return None, "perf not found"
    data = os.path.join(out_dir, "perf.data")
    cmd = [perf, "record", "-F", "999", "-g", "-o", data, "--",
           os.path.join(bin_dir, "D2RUST-Game"), "state-dump", "--save", save, "--seed", str(SEED),
           "--ticks", str(ticks), "--tick-rate", str(tick_rate),
           "--out", os.path.join(out_dir, "profile.state.jsonl")]
    if level is not None:
        cmd += ["--poke", f"4 warp {level}"]
    rc, _ = run(cmd, env, 3600, os.path.join(out_dir, "profile.log"))
    if rc != 0:
        return None, f"perf record failed ({rc})"
    rep = subprocess.run([perf, "report", "-i", data, "--no-children", "--sort", "symbol",
                          "--stdio", "--percent-limit", "0.7", "-g", "none"],
                         capture_output=True, text=True)
    rows = []
    for line in rep.stdout.splitlines():
        line = line.strip()
        if not line or line.startswith("#") or "%" not in line:
            continue
        p, _, rest = line.partition("%")
        try:
            share = float(p)
        except ValueError:
            continue
        sym = rest.split("]", 1)[-1].strip()
        rows.append((share, sym))
    return rows[:25], None


def write_md(results, prof, args, path):
    o = []
    w = o.append
    w("# Frame time and memory")
    w("")
    w(f"Generated by `tools/perf/perf.py` {VERSION}: `{' '.join(args)}`.")
    w(f"Machine: {platform.node()}, {os.cpu_count()} logical CPUs, {platform.system()}.")
    w(f"Budget: server tick (tick + flush) <= {ms(BUDGET_US)} ms; client frame")
    w(f"interval <= {ms(BUDGET_US)} ms. Stats are steady state, after the first {WARM_TICKS} ticks")
    w("(level load); \"load max\" is the worst tick / frame including the load.")
    w("Headless server tick times are mean / p99 / max ms; other times are p50 / p95 / max ms.")
    w("")
    w("## Headless (state-dump: server + bridge, no Bevy)")
    w("")
    w("| Scene | Ticks | Server tick | of which flush | Bridge frame | Tick load max | Peak RSS MiB | Budget |")
    w("|---|---|---|---|---|---|---|---|")
    for name, r in results:
        h = r.get("headless")
        if not h:
            w(f"| {name} | - | {r.get('headless_error', 'not run')} | | | | | |")
            continue
        miss = [m for m in misses(r) if m.startswith("headless")]
        w(f"| {name} ({r['label']}) | {h['tick']['n']} | {triple(h['tick'], ('mean', 'p99', 'max'))} | {triple(h['flush'])} | "
          f"{triple(h['bridge']) if 'bridge' in h else '-'} | {ms(h['tick_load_max'])} | "
          f"{h['peak_rss_mib']:.0f} | {'MISS' if miss else 'ok'} |")
    w("")
    if any(r.get("headless", {}).get("slow_ticks") for _, r in results):
        w("## Slowest headless ticks (including load)")
        w("")
        w("Timing spans are inclusive wall time; nested spans overlap.")
        w("")
        w("| Scene | Frame | Tick + flush ms | Top spans (ms) |")
        w("|---|---|---|---|")
        for name, r in results:
            for tick in r.get("headless", {}).get("slow_ticks", []):
                phases = ", ".join(f"{k}: {ms(v)}" for k, v in tick["spans"])
                w(f"| {name} | {tick['frame']} | {ms(tick['us'])} | {phases or '-'} |")
        w("")
    if any(r.get("window") or r.get("window_error") for _, r in results):
        w("## Windowed (play under Xvfb, 800x600, lavapipe)")
        w("")
        w("| Scene | Frames | FPS | Frame interval | Pump (First+PreUpdate) | Update | Post+Last | Render world | Server tick | Frames over budget | Frame load max | Peak RSS MiB |")
        w("|---|---|---|---|---|---|---|---|---|---|---|---|")
        for name, r in results:
            x = r.get("window")
            if not x:
                w(f"| {name} | - | {r.get('window_error', 'not run')} | | | | | | | | | |")
                continue
            w(f"| {name} | {x['frame']['n']} | {x['fps']} | {triple(x['frame'])} | {triple(x['pre'])} | "
              f"{triple(x['update'])} | {triple(x['post'])} | {triple(x['render']) if 'render' in x else '-'} | "
              f"{triple(x['tick'])} | {x['over_budget']} | {ms(x['frame_load_max'])} | {x['peak_rss_mib']:.0f} |")
        w("")
    w("## Budget misses")
    w("")
    any_miss = False
    for name, r in results:
        m = misses(r)
        if m:
            any_miss = True
            w(f"- **{name}**: " + "; ".join(m))
    if not any_miss:
        w("None.")
    w("")
    if prof:
        scene, rows, err = prof
        w(f"## Hot spots (perf record -F 999 -g, headless {scene}, self time)")
        w("")
        if err:
            w(f"Not run: {err}.")
        else:
            w("| Self % | Symbol |")
            w("|---|---|")
            for share, sym in rows:
                w(f"| {share:.2f} | `{sym[:140]}` |")
        w("")
    with open(path, "w") as f:
        f.write("\n".join(o) + "\n")


def selftest():
    assert pct([], 50) == 0
    assert pct([5], 95) == 5
    v = list(range(1, 101))
    assert pct(v, 50) == 50 and pct(v, 95) == 95 and pct(v, 100) == 100
    assert stats(v)["mean"] == 50.5 and stats(v)["p99"] == 99
    s = summarize({"format": "perf 1", "peak_rss_kib": 2048,
                   "ticks": [[i, 1, 100 * i, 10] for i in range(1, 61)],
                   "frames": [], "render": [], "bridge": [1] * 60}, 40)
    assert s["tick"]["n"] == 20 and s["tick"]["max"] == 6010 and s["peak_rss_mib"] == 2
    s2 = summarize({"format": "perf 2", "peak_rss_kib": 0, "ticks": [[1, 1, 100, 10, 0]],
                    "frames": [[16000, 1, 2, 3, 0, 1]], "render": [], "bridge": [], "spans": [],
                    "summary": {"pictures_per_s": 60.0, "hitch_ms": 50, "hitch_count": 2,
                                "hitches": [{"frame": 0}]}}, 0)
    assert s2["pictures_per_s"] == 60.0 and s2["hitch_count"] == 2 and s2["frame"]["max"] == 16000
    attributed = summarize({"format": "perf 2", "peak_rss_kib": 0,
                            "ticks": [[7, 2, 20000, 10, 100]],
                            "spans": [["setup", 102, 19000], ["drain", 100, 2], ["outside", 0, 10]],
                            "frames": [], "render": [], "bridge": []}, 0)
    assert attributed["slow_ticks"] == [{"frame": 7, "us": 20010, "spans": [("setup", 19000)]}]
    r = {"headless": {"tick": {"max": BUDGET_US + 1}}}
    assert misses(r) and not misses({"headless": {"tick": {"max": BUDGET_US}, "ticks_per_s": TARGET_HZ}})
    assert any("throughput missing" in m for m in misses(r))
    assert misses({"headless": {"tick": {"max": 1}, "tick_load_max": BUDGET_US + 1}})
    assert misses({"headless": {"tick": {"max": 1}, "ticks_per_s": TARGET_HZ - 1}})
    assert not misses({"headless": {"tick": {"max": 1}, "ticks_per_s": TARGET_HZ}})
    print("selftest ok")
    return 0


def main(argv=None):
    global BUDGET_US, WARM_TICKS, TARGET_HZ
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--bin", default=os.path.join(REPO, "target", "perf", "release"))
    ap.add_argument("--scenes", default=",".join(s[0] for s in SCENES))
    ap.add_argument("--ticks", type=int, default=1800, help="headless server ticks (30 s at 60 Hz)")
    ap.add_argument("--tick-rate", type=int, default=60, help="headless simulation Hz (25..1000)")
    ap.add_argument("--frames", type=int, default=900, help="windowed frames")
    ap.add_argument("--no-window", action="store_true")
    ap.add_argument("--no-headless", action="store_true")
    ap.add_argument("--profile", help="scene to profile with perf (headless)")
    ap.add_argument("--out", default=os.path.join(REPO, "target", "perf-runs"))
    ap.add_argument("--md")
    ap.add_argument("--json")
    ap.add_argument("--selftest", action="store_true")
    a = ap.parse_args(argv)
    if a.selftest:
        return selftest()
    if not 25 <= a.tick_rate <= 1000:
        ap.error("--tick-rate must be between 25 and 1000 Hz")
    if a.tick_rate != 60 and not a.no_window:
        ap.error("--tick-rate other than 60 requires --no-window (state-dump)")
    TARGET_HZ = a.tick_rate
    BUDGET_US = min(1_000_000 / a.tick_rate, 16_000) if a.tick_rate == 60 else 1_000_000 / a.tick_rate
    WARM_TICKS = (40 * a.tick_rate + 24) // 25  # the same 1.6 s at every rate
    if not os.environ.get("D2_GAME_DIR"):
        print("perf: D2_GAME_DIR is not set", file=sys.stderr)
        return 3
    os.makedirs(a.out, exist_ok=True)
    wanted = a.scenes.split(",")
    unknown = [s for s in wanted if s not in {x[0] for x in SCENES}]
    if unknown:
        ap.error(f"unknown scene(s) {unknown}")
    xvfb = shutil.which("xvfb-run")
    results = []
    try:
        for name, act, level, label in SCENES:
            if name not in wanted:
                continue
            save = make_save(a.bin, act, a.out)
            pokes = ["--poke", f"4 warp {level}"] if level is not None else []
            r = {"label": label}
            if not a.no_headless:
                out = os.path.join(a.out, f"{name}.headless.json")
                env = dict(os.environ, D2_PERF_OUT=out)
                env.pop("D2_COVERAGE_DIR", None)
                cmd = [os.path.join(a.bin, "D2RUST-Game"), "state-dump", "--save", save,
                       "--seed", str(SEED), "--ticks", str(a.ticks), "--tick-rate", str(a.tick_rate),
                       "--out", os.path.join(a.out, f"{name}.state.jsonl")] + pokes
                rc, secs = run(cmd, env, 3600, os.path.join(a.out, f"{name}.headless.log"))
                if rc == 0 and os.path.exists(out):
                    r["headless"] = summarize(load(out), WARM_TICKS)
                    r["headless"]["wall_s"] = round(secs, 1)
                else:
                    r["headless_error"] = f"exit {rc}, see {name}.headless.log"
            if not a.no_window:
                if not xvfb:
                    r["window_error"] = "xvfb-run not found"
                else:
                    out = os.path.join(a.out, f"{name}.window.json")
                    env = dict(os.environ, D2_PERF_OUT=out)
                    env.pop("D2_COVERAGE_DIR", None)
                    cmd = [xvfb, "-a", "-s", "-screen 0 1024x768x24",
                           os.path.join(a.bin, "D2RUST-Game"), "play", "--mute", "--save", save,
                           "--seed", str(SEED), "--frames", str(a.frames)] + pokes
                    rc, secs = run(cmd, env, 3600, os.path.join(a.out, f"{name}.window.log"))
                    if os.path.exists(out):
                        r["window"] = summarize(load(out), WARM_TICKS)
                        r["window"]["wall_s"] = round(secs, 1)
                        if rc != 0:
                            r["window"]["exit"] = rc
                    else:
                        r["window_error"] = f"exit {rc}, see {name}.window.log"
            results.append((name, r))
            print(f"{name}: " + json.dumps({k: v for k, v in r.items() if k != 'label'})[:400], flush=True)
        prof = None
        if a.profile:
            s = next(x for x in SCENES if x[0] == a.profile)
            save = make_save(a.bin, s[1], a.out)
            env = dict(os.environ)
            env.pop("D2_PERF_OUT", None)
            rows, err = profile(a.bin, save, s[2], a.ticks, a.out, env, a.tick_rate)
            prof = (a.profile, rows, err)
    except PerfError as e:
        print(f"perf: {e}", file=sys.stderr)
        return 3
    if a.json:
        with open(a.json, "w") as f:
            json.dump({"format": "perf-report 1", "tick_rate": a.tick_rate, "budget_us": BUDGET_US, "results": results}, f, indent=1)
    if a.md:
        write_md(results, prof, sys.argv[1:] if argv is None else argv, a.md)
    return int(any(misses(r) or "headless_error" in r or "window_error" in r
                   for _, r in results))


if __name__ == "__main__":
    sys.exit(main())
