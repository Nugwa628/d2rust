#!/usr/bin/env python3
# Spec: specs/ui/text.md (§9), specs/ui/messages.md (§7 r4); playtest bug B013
"""B013 check: time the client passes while Akara's first dialog scrolls.

Runs `d2-client autoplay-host` (the play client headless, with the
preview's frame build) on a fresh Amazon, seed 1234, walks the Act I
probe route to Akara until her menu and greeting dialog are up, then
times N single-tick passes. A pass whose frame build fails returns early
(the preview does not present it), so a broken dialog shows as passes far
shorter than normal play (B013: p50 ~5 ms before the fix, ~13 ms after
on a 4-core cloud container, the frame built every pass).

    python3 tools/perf/b013_dialog.py [--bin target/release/D2RUST-Game] [--steps 400]

Needs D2_GAME_DIR. Standard library only. Our own code.
"""

import argparse
import json
import os
import subprocess
import sys
import time

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
ROUTE = os.path.join(REPO, "tools", "autoplay", "routes", "act1-probe.route")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", default=os.path.join(REPO, "target", "release", "D2RUST-Game"))
    ap.add_argument("--steps", type=int, default=400)
    a = ap.parse_args()
    if not os.environ.get("D2_GAME_DIR"):
        sys.exit("b013_dialog: set D2_GAME_DIR")
    lines = open(ROUTE).read().splitlines()
    cmds = [l for l in lines[2:] if l.strip() and not l.startswith(("#", "check"))]
    cmds = cmds[: cmds.index("press L 400 160")]  # up to Akara's menu
    p = subprocess.Popen([a.bin, "autoplay-host", "--mute", "--new", "amazon", "AutoProbe", "--seed", "1234"],
                         stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                         text=True, bufsize=1)

    def rd():
        while True:
            line = p.stdout.readline()
            if not line or line.startswith("{"):
                return line

    def cmd(c):
        p.stdin.write(c + "\n")
        p.stdin.flush()
        return rd()

    rd()
    for c in cmds:
        cmd(c)
    for _ in range(60):
        client = json.loads(cmd("state"))["client"]
        if client.get("dialog_lines"):
            break
        cmd("step 5")
    else:
        sys.exit("b013_dialog: no dialog opened (route stale?)")
    times = []
    for _ in range(a.steps):
        t = time.perf_counter()
        cmd("step 1")
        times.append(time.perf_counter() - t)
    still = json.loads(cmd("state"))["client"].get("dialog_lines")
    p.stdin.close()
    p.wait(timeout=60)
    times.sort()
    n = len(times)
    print(f"dialog lines {client.get('dialog_lines')} (still up: {bool(still)}); {n} passes: "
          f"p50 {1000 * times[n // 2]:.1f} ms, p95 {1000 * times[n * 95 // 100]:.1f} ms, "
          f"max {1000 * times[-1]:.1f} ms")


if __name__ == "__main__":
    main()
