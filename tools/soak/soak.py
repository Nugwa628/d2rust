#!/usr/bin/env python3
"""Soak campaign and input-log reducer for `d2-client soak` (D2RUST only).

    python3 tools/soak/soak.py campaign --minutes 30 --out OUT
    python3 tools/soak/soak.py run --start town2 --seed 7 --steps 3000 --out OUT
    python3 tools/soak/soak.py reduce OUT/logs/town1-7.log --sig SIG [--out OUT]
    python3 tools/soak/soak.py saves [--out target/soak]
    python3 tools/soak/soak.py roundtrip [--out target/soak/roundtrip]
    python3 tools/soak/soak.py selftest

`campaign` runs seeded soaks from every start (each act's town, a few
outdoor levels) until the time is up, one child process per run with a
wall-time limit (a run past it is a `timeout` finding: a hang the server
tick guard cannot see). Every new finding signature keeps the input log
that found it, then `reduce` shrinks the log (cut after the finding,
then delta debugging over the action lines) to the least input that
still gives the same signature. Results: OUT/findings.jsonl (one line
per signature: kind, sig, detail, start, seed, the reduced log and the
repro command).

Saves for the act starts are made with `d2s-tool new` into OUT/saves
(never committed). Python stdlib only.
"""

import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
import time
import hashlib
from pathlib import Path

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
REL = os.path.join(REPO, "target", "release")
EXE = ".exe" if os.name == "nt" else ""
CLIENT = os.environ.get("D2_SOAK_CLIENT", os.path.join(REL, "D2RUST-Game" + EXE))
D2S = os.environ.get("D2_SOAK_D2S", os.path.join(REL, "d2s-tool" + EXE))
FORMAT = "soak-log 1"

# Starts: name -> (save args for d2s-tool new, or None for a
# fresh `--new` character; soak options).
SAVE_BASE = ["--class", "sor", "--expansion", "--level", "30", "--waypoints", "all",
             "--stat", "8=128000", "--stat", "9=128000", "--difficulty", "normal",
             # fixed seeds and times: the same start gives the same bytes
             # (the committed repro logs name these saves)
             "--seed", "1", "--map-seed", "1", "--time", "0x60000000"]
STARTS = {
    "new-sor": (None, ["--new", "sorceress"]),
    "new-bar": (None, ["--new", "barbarian"]),
    "town1": (SAVE_BASE + ["--quests", "acts=0", "--act", "0"], []),
    "town2": (SAVE_BASE + ["--quests", "acts=1", "--act", "1"], []),
    "town3": (SAVE_BASE + ["--quests", "acts=2", "--act", "2"], []),
    "town4": (SAVE_BASE + ["--quests", "acts=3", "--act", "3"], []),
    "town5": (SAVE_BASE + ["--quests", "acts=4", "--act", "4"], []),
    # Outdoor levels: the act's save, then the start poke `warp`.
    "blood-moor": (SAVE_BASE + ["--quests", "acts=0", "--act", "0"], ["--warp", "2"]),
    "cold-plains": (SAVE_BASE + ["--quests", "acts=0", "--act", "0"], ["--warp", "3"]),
    "rocky-waste": (SAVE_BASE + ["--quests", "acts=1", "--act", "1"], ["--warp", "41"]),
    "spider-forest": (SAVE_BASE + ["--quests", "acts=2", "--act", "2"], ["--warp", "76"]),
    "outer-steppes": (SAVE_BASE + ["--quests", "acts=3", "--act", "3"], ["--warp", "104"]),
    "bloody-foothills": (SAVE_BASE + ["--quests", "acts=4", "--act", "4"], ["--warp", "110"]),
}


CHECKPOINTS = os.path.join(REPO, "target", "checkpoints")


def checkpoint_starts():
    """`ck-<name>` for every built checkpoint save
    (`python3 tools/checkpoints/make.py`, specs/tools/checkpoints.md)."""
    if not os.path.isdir(CHECKPOINTS):
        return {}
    return {"ck-" + f[:-4]: os.path.join(CHECKPOINTS, f)
            for f in sorted(os.listdir(CHECKPOINTS)) if f.endswith(".d2s")}


def all_starts():
    return list(STARTS) + list(checkpoint_starts())


def env():
    e = dict(os.environ)
    if "D2_GAME_DIR" not in e:
        home = os.path.join(os.path.expanduser("~"), "game")
        if os.path.isdir(home):
            e["D2_GAME_DIR"] = home
    return e


def check_tools():
    for p in (CLIENT, D2S):
        if not os.path.exists(p):
            sys.exit(f"soak: {p} missing: cargo build --release -p d2-client -p d2s-tool")


def make_save(out, name):
    args, _ = STARTS[name]
    if args is None:
        return None
    d = os.path.join(out, "saves")
    os.makedirs(d, exist_ok=True)
    path = os.path.join(d, f"Soak{re.sub('[^A-Za-z0-9]', '', name)}.d2s")
    if not os.path.exists(path):
        nm = "Soak" + re.sub("[^A-Za-z]", "", name)[:10]
        r = subprocess.run([D2S, "new", "--name", nm] + args + ["-o", path],
                           capture_output=True, text=True, env=env())
        if r.returncode != 0:
            raise RuntimeError(f"d2s-tool new for {name}: {r.stderr.strip() or r.stdout.strip()}")
    return path


def start_args(out, name):
    if name.startswith("ck-"):
        return ["--save", checkpoint_starts()[name]]
    save = make_save(out, name)
    _, extra = STARTS[name]
    return (["--save", save] if save else []) + extra


def read_report(path):
    found, summary = [], {}
    if os.path.exists(path):
        for line in open(path):
            line = line.strip()
            if not line:
                continue
            j = json.loads(line)
            if j.get("summary"):
                summary = j
            else:
                found.append(j)
    return found, summary


def last_step(log):
    s = 0
    for line in Path(log).read_text(encoding="utf-8").splitlines():
        w = line.split()
        if w and w[0].isdigit():
            s = int(w[0])
    return s


def soak_once(cmd_args, report, timeout):
    """Runs one `d2-client soak`; returns (findings, summary). A run past
    `timeout` seconds or one that dies is a finding of its own."""
    cmd = [CLIENT, "soak", "--mute"] + cmd_args + ["--report", report]
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, env=env(), timeout=timeout)
    except subprocess.TimeoutExpired:
        return [{"kind": "timeout", "sig": "timeout", "step": -1, "frame": -1,
                 "detail": f"no exit in {timeout} s"}], {}
    found, summary = read_report(report)
    if r.returncode not in (0, 1) or not summary:
        tail = (r.stderr or "").strip().splitlines()[-3:]
        found.append({"kind": "crash", "sig": f"crash:exit{r.returncode}", "step": -1,
                      "frame": -1, "detail": " | ".join(tail)})
    return found, summary


def run(out, name, seed, steps, timeout, roundtrip_every=0):
    os.makedirs(os.path.join(out, "logs"), exist_ok=True)
    log = os.path.join(out, "logs", f"{name}-{seed}.log")
    report = os.path.join(out, "logs", f"{name}-{seed}.json")
    args = start_args(out, name) + ["--seed", str(seed), "--steps", str(steps),
                                     "--keep-going", "--log-out", log]
    if roundtrip_every > 0 and steps > roundtrip_every:
        args += ["--roundtrip-at", ",".join(str(s) for s in range(roundtrip_every, steps, roundtrip_every))]
    found, summary = soak_once(args, report, timeout)
    return log, found, summary


# ---------------------------------------------------------------- reduce


def parse_log(path):
    lines = [l.split("#", 1)[0].strip() for l in Path(path).read_text(encoding="utf-8").splitlines()]
    lines = [l for l in lines if l]
    if not lines or not lines[0].startswith(FORMAT):
        raise RuntimeError(f"{path}: not a {FORMAT} file")
    rates = [w for w in lines[0].split() if w.startswith("tick_rate=")]
    if rates != ["tick_rate=60"]:
        raise RuntimeError(f"{path}: recording rate missing or not 60 Hz (legacy logs are 25 Hz); re-record at 60 Hz")
    return lines[0], lines[1:]


def replay_has(head, acts, sig, steps, timeout, tmp):
    path = os.path.join(tmp, "probe.log")
    with open(path, "w") as f:
        f.write(head + "\n" + "".join(a + "\n" for a in acts))
    found, _ = soak_once(["--replay", path, "--steps", str(steps), "--keep-going"],
                         os.path.join(tmp, "probe.json"), timeout)
    return any(x["sig"] == sig for x in found), found


def ddmin(items, test):
    """Zeller's ddmin: a 1-minimal subsequence of `items` for which
    `test` holds (`test(items)` must hold)."""
    n = 2
    while len(items) >= 2:
        size = max(1, len(items) // n)
        chunks = [items[i:i + size] for i in range(0, len(items), size)]
        reduced = False
        for i in range(len(chunks)):
            rest = [x for j, c in enumerate(chunks) if j != i for x in c]
            if test(rest):
                items, n, reduced = rest, max(n - 1, 2), True
                break
        if not reduced:
            if n >= len(items):
                break
            n = min(len(items), n * 2)
    return items


def reduce(log, sig, out, timeout=600, max_tests=400):
    """The least input of `log` that still finds `sig`."""
    head, acts = parse_log(log)
    tmp = tempfile.mkdtemp(prefix="soak-reduce-")
    steps = last_step(log) + 400
    ok, found = replay_has(head, acts, sig, steps, timeout, tmp)
    if not ok:
        return None, f"the log does not reproduce {sig} ({[x['sig'] for x in found]})"
    hit = [x for x in found if x["sig"] == sig][0]
    # Cut after the finding's step (steps past it cannot have caused it).
    if hit.get("step", -1) >= 0:
        cut = [a for a in acts if int(a.split()[0]) <= hit["step"]]
        if replay_has(head, cut, sig, hit["step"] + 400, timeout, tmp)[0]:
            acts, steps = cut, hit["step"] + 400
    tests = [0]

    def test(sub):
        tests[0] += 1
        if tests[0] > max_tests:
            return False
        return replay_has(head, sub, sig, steps, timeout, tmp)[0]

    acts = ddmin(acts, test) if acts else acts
    # Steps run: the last kept action's step plus the finding's distance.
    os.makedirs(os.path.join(out, "repro"), exist_ok=True)
    slug = re.sub(r"[^A-Za-z0-9]+", "-", sig).strip("-")[:60] + "-" + hashlib.sha256(sig.encode()).hexdigest()[:16]
    dst = os.path.join(out, "repro", f"{slug}.log")
    with open(dst, "w") as f:
        f.write(head + "\n" + "".join(a + "\n" for a in acts))
    return dst, f"{len(acts)} action(s), {tests[0]} replays, --steps {steps}"


def export_scenario(log, sig, out, steps=None):
    """Lossless, standalone repro envelope; copy a start save beside it if needed."""
    head, acts = parse_log(log)
    fields = dict(word.split("=", 1) for word in head[len(FORMAT):].split())
    out = Path(out)
    out.parent.mkdir(parents=True, exist_ok=True)
    if "save" in fields:
        save = Path(fields["save"])
        if not save.is_file():
            raise ValueError(f"start save is missing: {save}")
        content = save.read_bytes()
        name = hashlib.sha256(content).hexdigest() + ".d2s"
        (out.parent / name).write_bytes(content)
        fields["save"] = name
    head = FORMAT + " " + " ".join(f"{k}={v}" for k, v in fields.items())
    duration = steps if steps is not None else last_step(log) + 400
    if not sig or not 0 < duration <= 1_000_000:
        raise ValueError("signature required and steps must be in 1..1000000")
    if any(int(a.split()[0]) >= duration for a in acts):
        raise ValueError("action outside scenario duration")
    payload = {"signature": sig, "steps": duration,
               "log": head + "\n" + "".join(a + "\n" for a in acts)}
    out.write_text("soak-scenario 1\n" + json.dumps(payload, indent=2) + "\n", encoding="utf-8")
    return out


# ---------------------------------------------------------------- campaign


def campaign(a, on_finding=None):
    check_tools()
    out = a.out
    os.makedirs(out, exist_ok=True)
    db = os.path.join(out, "findings.jsonl")
    known = {}
    if os.path.exists(db):
        for line in open(db):
            j = json.loads(line)
            known[j["sig"]] = j
    names = a.starts.split(",") if a.starts else all_starts()
    end = time.time() + a.minutes * 60
    seed = a.first_seed
    runs = frames = 0
    while time.time() < end:
        for name in names:
            if time.time() >= end:
                break
            log, found, summary = run(out, name, seed, a.steps, a.timeout, a.roundtrip_every)
            runs += 1
            frames += summary.get("steps", 0)
            new = [f for f in found if f["sig"] not in known]
            for f in new:
                steps = (f["step"] if f["step"] >= 0 else last_step(log)) + 400
                entry = {"sig": f["sig"], "kind": f["kind"], "detail": f["detail"],
                         "start": name, "seed": seed, "log": log, "step": f["step"]}
                if not a.no_reduce:
                    red, note = reduce(log, f["sig"], out, a.timeout)
                    entry["reduced"] = red
                    entry["reduce_note"] = note
                    if red:
                        entry["repro"] = (f"D2_GAME_DIR=... target/release/D2RUST-Game soak --replay {red}"
                                          f" --steps {steps} --keep-going")
                known[f["sig"]] = entry
                with open(db, "a") as fh:
                    fh.write(json.dumps(entry) + "\n")
                if on_finding is not None:
                    on_finding(entry)
                print(f"NEW {f['kind']} {f['sig']} [{name} seed {seed}]: {f['detail']}", flush=True)
            print(f"run {runs}: {name} seed {seed}: {summary.get('steps', 0)} frames, "
                  f"{len(found)} finding(s), {len(new)} new", flush=True)
        seed += 1
    print(f"campaign: {runs} runs, {frames} frames, {len(known)} signatures -> {db}")
    return 0


def roundtrips(a):
    """At every start and checkpoint: no input, round trips at
    frames 30 and 80 (two reloads), then the same with random input and
    round trips every 300 frames. Prints one line per start; exit 1 when
    any round trip differs."""
    check_tools()
    bad = 0
    for name in all_starts():
        for label, extra in (("still", ["--rate", "0", "--no-kit", "--steps", "120",
                                        "--roundtrip-at", "30,80"]),
                             ("played", ["--seed", str(a.seed), "--steps", "1000",
                                         "--roundtrip-at", "300,600,900"])):
            report = os.path.join(a.out, f"rt-{name}-{label}.json")
            log = os.path.join(a.out, f"rt-{name}-{label}.log")
            os.makedirs(a.out, exist_ok=True)
            found, summary = soak_once(start_args(a.out, name) + extra +
                                       ["--keep-going", "--log-out", log,
                                        "--save-dir", os.path.join(a.out, f"rt-{name}-{label}")],
                                       report, a.timeout)
            rt = [f for f in found if f["kind"] in ("roundtrip", "panic", "crash", "timeout", "setup")]
            bad += bool(rt)
            print(f"{name} {label}: {summary.get('roundtrips', 0)} round trip(s), "
                  f"{len(rt)} difference(s)" + "".join(f"\n  {f['sig']}: {f['detail'][:200]}" for f in rt),
                  flush=True)
    return 1 if bad else 0


def selftest():
    # ddmin finds the two needed items.
    got = ddmin(list(range(40)), lambda s: 7 in s and 31 in s)
    assert got == [7, 31], got
    got = ddmin(list(range(10)), lambda s: 3 in s)
    assert got == [3], got
    with tempfile.TemporaryDirectory() as tmp:
        path = os.path.join(tmp, "rate.log")
        for rate in ("", " tick_rate=25", " tick_rate=60"):
            with open(path, "w") as f:
                f.write(FORMAT + rate + " seed=1 new=amazon\n1 wait 1\n")
            if rate == " tick_rate=60":
                assert len(parse_log(path)[1]) == 1
            else:
                try:
                    parse_log(path)
                except RuntimeError as e:
                    assert "re-record" in str(e)
                else:
                    raise AssertionError("legacy log accepted")
    print("soak.py selftest: ok")
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("campaign")
    c.add_argument("--minutes", type=float, default=30)
    c.add_argument("--out", required=True)
    c.add_argument("--starts", help="comma list (default all, with ck-<name> for each built checkpoint): "
                   + ",".join(STARTS))
    c.add_argument("--steps", type=int, default=3000)
    c.add_argument("--first-seed", type=int, default=1)
    c.add_argument("--timeout", type=int, default=600)
    c.add_argument("--no-reduce", action="store_true")
    c.add_argument("--roundtrip-every", type=int, default=0,
                   help="a save/load round trip every N frames of each run")
    r = sub.add_parser("run")
    r.add_argument("--start", required=True, help="a start: " + ",".join(STARTS) + ", ck-<checkpoint>")
    r.add_argument("--seed", type=int, default=1)
    r.add_argument("--steps", type=int, default=3000)
    r.add_argument("--timeout", type=int, default=600)
    r.add_argument("--out", required=True)
    v = sub.add_parser("saves", help="make every start's save (the committed repro logs use target/soak/saves)")
    v.add_argument("--out", default=os.path.join(REPO, "target", "soak"))
    d = sub.add_parser("reduce")
    d.add_argument("log")
    d.add_argument("--sig", required=True)
    d.add_argument("--out", default=".")
    d.add_argument("--timeout", type=int, default=600)
    t = sub.add_parser("roundtrip", help="save/load round trips at every start and checkpoint")
    t.add_argument("--out", default=os.path.join(REPO, "target", "soak", "roundtrip"))
    t.add_argument("--seed", type=int, default=1)
    t.add_argument("--timeout", type=int, default=600)
    sub.add_parser("selftest")
    x = sub.add_parser("export", help="export a reduced log as a lossless repro .scn")
    x.add_argument("log")
    x.add_argument("--sig", required=True)
    x.add_argument("--out", required=True)
    x.add_argument("--steps", type=int)
    a = ap.parse_args()
    if a.cmd == "selftest":
        return selftest()
    if a.cmd == "export":
        print(export_scenario(a.log, a.sig, a.out, a.steps))
        return 0
    if a.cmd == "campaign":
        return campaign(a)
    if a.cmd == "roundtrip":
        return roundtrips(a)
    check_tools()
    if a.cmd == "saves":
        for name in STARTS:
            p = make_save(a.out, name)
            if p:
                print(p)
        return 0
    if a.cmd == "run":
        log, found, summary = run(a.out, a.start, a.seed, a.steps, a.timeout)
        for f in found:
            print(json.dumps(f))
        print(json.dumps(summary), f"log: {log}")
        return 1 if found else 0
    red, note = reduce(a.log, a.sig, a.out, a.timeout)
    print(red or "not reduced", note)
    return 0 if red else 1


if __name__ == "__main__":
    sys.exit(main())
