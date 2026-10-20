"""No runtime animation generation: animation frames are made at install, never during play.

Owner rule (2026-10-19): animation frames are not generated on the go. They
are made when the game or a mod is installed and kept in files; the running
game only loads them. This check plays the real game offscreen in scenes with
units on screen (the local player, town NPCs, monsters) and watches, from
outside the process, for any sign of generation work:

- `generation-thread`: a game thread whose name matches `--thread-pattern`
  (default `anim`; the generator's worker is a named thread) exists while
  playing. Its CPU time is reported. Linux only (`/proc/PID/task/*/comm`).
- `generated-files`: the game wrote animation frame files into its user
  folder (`*.d2frames`, or anything under `cache/animation/`) while playing.
- `new-user-files`: any other file the game created in the user folder,
  apart from saves, logs and settings, is listed (report only), so a
  generator writing somewhere new is still seen.

The check never reads the game's memory or changes its run. It passes once
the runtime only loads baked frames (or plays source frames when none are
baked).
"""
import os
import re
import threading
import time
from pathlib import Path

try:
    from . import offscreen
except ImportError:
    import offscreen

# Scenes: (name, extra play arguments). Town has NPCs; the Blood Moor has
# monsters near the start.
SCENES = {
    "town": [],
    "blood-moor": ["--poke", "30 warp 2"],
}


def watch_threads(pid, pattern, stop, seen):
    """Polls the process's thread names until `stop` is set."""
    clk = os.sysconf("SC_CLK_TCK") if hasattr(os, "sysconf") else 100
    task = Path(f"/proc/{pid}/task")
    while not stop.is_set():
        try:
            for t in task.iterdir():
                try:
                    name = (t / "comm").read_text().strip()
                except OSError:
                    continue
                if pattern.search(name):
                    try:
                        stat = (t / "stat").read_text()
                        fields = stat[stat.rindex(")") + 2:].split()
                        cpu = (int(fields[11]) + int(fields[12])) / clk
                    except (OSError, ValueError, IndexError):
                        cpu = None
                    entry = seen.setdefault(t.name, {"name": name, "cpu_s": 0.0,
                                                     "first_s": round(time.monotonic(), 3)})
                    if cpu is not None:
                        entry["cpu_s"] = max(entry["cpu_s"], cpu)
        except OSError:
            pass
        stop.wait(0.05)


def generated(path):
    return path.endswith(".d2frames") or "/animation/" in f"/{path}" or path.startswith("cache/animation")


def run_scene(opts, name, args):
    folder = opts.out / name
    run = offscreen.Run(opts.bin, folder, ["--new", opts.cls, "Animgen", "--seed", "7",
                                           "--frames", str(opts.frames), *args, *opts.play_args],
                        video={"animation_frames": "smooth"}, size=opts.size,
                        game_dir=opts.game_dir, perf=False)
    started = time.monotonic()
    run.start()
    seen, stop = {}, threading.Event()
    watcher = None
    if Path(f"/proc/{run.proc.pid}/task").is_dir():
        watcher = threading.Thread(target=watch_threads,
                                   args=(run.proc.pid, re.compile(opts.thread_pattern), stop, seen))
        watcher.start()
    try:
        rc = run.wait(opts.timeout)
    finally:
        stop.set()
        if watcher:
            watcher.join()
    for entry in seen.values():
        entry["first_s"] = round(entry["first_s"] - started, 3)
    files = run.files_written()
    gen = [f for f in files if generated(f)]
    violations = []
    if seen:
        violations.append({"rule": "generation-thread", "threads": sorted(seen.values(), key=lambda e: e["first_s"])})
    if gen:
        violations.append({"rule": "generated-files", "count": len(gen), "files": gen[:20]})
    return {"scene": name, "exit": rc, "folder": str(folder), "threads_watched": watcher is not None,
            "new_user_files": [f for f in files if f not in gen][:50], "violations": violations}


def add_args(p):
    p.add_argument("--bin", required=True)
    p.add_argument("--game-dir")
    p.add_argument("--out", required=True, type=Path)
    p.add_argument("--scenes", default=",".join(SCENES))
    p.add_argument("--class", dest="cls", default="sorceress")
    p.add_argument("--frames", type=int, default=900)
    p.add_argument("--size", default="800x600")
    p.add_argument("--thread-pattern", default="anim")
    p.add_argument("--jobs", type=int, default=2)
    p.add_argument("--timeout", type=float, default=600)
    p.add_argument("--play-arg", dest="play_args", action="append", default=[])


def main(opts):
    opts.out.mkdir(parents=True, exist_ok=False)
    names = opts.scenes.split(",")
    unknown = [n for n in names if n not in SCENES]
    if unknown:
        return offscreen.fail(f"unknown scenes {unknown}; known: {list(SCENES)}")
    results = offscreen.jobs_map(lambda n: run_scene(opts, n, SCENES[n]), names, opts.jobs)
    violations = [dict(v, scene=r["scene"]) for r in results for v in r["violations"]]
    violations += [{"rule": "run-error", "scene": r["scene"], "exit": r["exit"]}
                   for r in results if r["exit"] != 0]
    verdict = {"format": "playcheck-animgen 1", "pass": not violations, "violations": violations,
               "runs": results}
    offscreen.write_json(opts.out / "verdict.json", verdict)
    for r in results:
        threads = next((v["threads"] for v in r["violations"] if v["rule"] == "generation-thread"), [])
        files = next((v["count"] for v in r["violations"] if v["rule"] == "generated-files"), 0)
        cpu = sum(t["cpu_s"] for t in threads)
        print(f"{r['scene']}\texit {r['exit']}\tgeneration threads {len(threads)} ({cpu:.2f} s CPU)"
              f"\tgenerated files {files}\tother new files {len(r['new_user_files'])}"
              + ("" if r["threads_watched"] else "\t(threads not watched on this OS)"))
    print(f"animgen: {'pass' if verdict['pass'] else 'FAIL'} ({len(violations)} violations); {opts.out / 'verdict.json'}")
    return 0 if verdict["pass"] else 1
