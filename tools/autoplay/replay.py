#!/usr/bin/env python3
"""Real-input smoke routes: replays a fixed route file through
`d2-client autoplay-host` (the `play` client headless, mouse and key
events only) and checks the state at each milestone.

    python3 tools/autoplay/replay.py tools/autoplay/routes/act2-probe.route
    python3 tools/autoplay/replay.py --all
    python3 tools/autoplay/replay.py --selftest

Route format `autoplay-route 1 tick_rate=60`: `#` comments; `host ARGS` (extra
autoplay-host options, e.g. `--new amazon Name --seed 1234`, or `--front
--seed 1234` to start on the main menu); `save D2S-TOOL-NEW-ARGS` (the
character, made with `d2s-tool new`: with `--front` it is written into the
host's `--save-dir` folder, for the character select screen to list, else
it is the host's `--save`); then host commands (`step N`, `move`, `press`,
`release`, `key`, `rawkey`, `click ui:ID`, `clickunit T C`, `rclickunit T C`, `interactunit T C`, `useunit T C`, `walklevel N`, `text WORD`, `ui`) sent as they
are, and checks: `check level N`, `check sent ID
>= N` (C->S message counts, hex id), `check open|closed UI` (a ui state
of the client), `check client FIELD == JSON` (a field of the state's
`client` object). `AUTOPLAY_PROFILE=debug` runs the debug build. The route,
save and seed are fixed; unit targets and UI ids resolve against live state.

`--res 1068x600` replays a route (recorded at 800x600) in the 16:9 frame:
the world and the centred HUD both sit (W - 800) / 2 = 134 px further
right there, so every pointer x moves by that and nothing else changes.
`click ui:ID` steps are not moved: the host looks the id up in the frame
on screen (the front end is always 800x600).

Exit 0: every check held; 1: a check failed or the host died; 3: error.
Python stdlib only.
"""

import argparse
import glob
import json
import os
import shlex
import subprocess
import sys
import tempfile

FORMAT = "autoplay-route 1"
HEADER = FORMAT + " tick_rate=60"
REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
ROUTES = os.path.join(os.path.dirname(os.path.abspath(__file__)), "routes")


class RouteError(Exception):
    pass


def parse(text, name="<route>"):
    """(host args, save args or None, steps); a step is (line no, text)."""
    lines = text.splitlines()
    if not lines or lines[0].strip() != HEADER:
        raise RouteError(f"{name}: recording rate missing or not 60 Hz (legacy routes are 25 Hz); re-record with '{HEADER}'")
    host, save, steps = [], None, []
    for no, raw in enumerate(lines[1:], 2):
        t = raw.strip()
        if not t:
            continue
        if t.startswith("#"):
            steps.append((no, t))
            continue
        word, _, rest = t.partition(" ")
        if word == "host":
            host += shlex.split(rest)
        elif word == "save":
            save = shlex.split(rest)
        elif word == "check":
            c = rest.split()
            ok = ((len(c) == 2 and c[0] in ("level", "open", "closed"))
                  or (len(c) == 4 and c[0] == "sent" and c[2] == ">=")
                  or (len(c) == 4 and c[0] == "client" and c[2] == "=="))
            if not ok:
                raise RouteError(f"{name}:{no}: check level N | check open|closed UI | check sent ID >= N"
                                 " | check client FIELD == JSON")
            steps.append((no, t))
        elif word == "click":
            if not (rest.startswith("ui:") and len(rest.split()) == 1):
                raise RouteError(f"{name}:{no}: click ui:ID")
            steps.append((no, t))
        elif word in ("step", "move", "press", "release", "key", "rawkey", "text", "ui", "clickunit", "rclickunit", "interactunit", "useunit", "walklevel"):
            steps.append((no, t))
        else:
            raise RouteError(f"{name}:{no}: unknown line '{word}'")
    return host, save, steps


def check(st, text):
    """None when `check ...` holds on state `st`, else why not."""
    c = text.split()[1:]
    me = next((u for u in st["snap"]["units"] if u["ut"] == 0 and u["g"] == st.get("local")), None)
    if c[0] == "level":
        lv = me.get("lv") if me else None
        return None if lv == int(c[1]) else f"player in level {lv}, expected {c[1]}"
    if c[0] in ("open", "closed"):
        is_open = int(c[1]) in st["client"]["open"]
        return None if is_open == (c[0] == "open") else f"ui {c[1]} open = {is_open}, expected {c[0]}"
    if c[0] == "client":
        got = st["client"]
        for field in c[1].split("."):
            got = got.get(field) if isinstance(got, dict) else None
        return None if got == json.loads(c[3]) else f"client {c[1]} = {json.dumps(got)}, expected {c[3]}"
    n = st["client"]["sent"].get(c[1].upper(), 0)
    return None if n >= int(c[3]) else f"C->S 0x{c[1]} sent {n} times, expected >= {c[3]}"


def ui_left(res):
    """Pointer x shift of frame `res` ("WxH" or None) for an 800-wide route."""
    if not res:
        return 0
    w = int(res.split("x")[0])
    return max(w - 800, 0) // 2


def shift_step(t, dx):
    """Step text `t` with its pointer x moved by `dx` (move/press/release)."""
    w = t.split()
    if dx and w and w[0] == "move" and len(w) == 3:
        w[1] = str(int(w[1]) + dx)
    elif dx and w and w[0] in ("press", "release") and len(w) == 4:
        w[2] = str(int(w[2]) + dx)
    else:
        return t
    return " ".join(w)


def save_name(save_args):
    """The character name of `d2s-tool new` arguments (its `--name`)."""
    if "--name" not in save_args[:-1]:
        raise RouteError("save: --name NAME is needed with host --front")
    return save_args[save_args.index("--name") + 1]


def replay(path, game_dir, work, res=None):
    name = os.path.basename(path)
    host_args, save_args, steps = parse(open(path).read(), name)
    dx = ui_left(res)
    steps = [(no, shift_step(t, dx)) for no, t in steps]
    if res:
        host_args = list(host_args) + ["--res", res]
    rel = os.path.join(REPO, "target", os.environ.get("AUTOPLAY_PROFILE", "release"))
    env = dict(os.environ, **({"D2_GAME_DIR": game_dir} if game_dir else {}))
    # Keep logs and startup user-folder migration inside this run by default.
    env["D2RUST_LOG_DIR"] = os.path.abspath(
        env.get("D2RUST_LOG_DIR") or os.path.join(work, os.path.splitext(name)[0], "logs"))
    env["D2RUST_HOME"] = os.path.abspath(
        env.get("D2RUST_HOME") or env.get("D2RS_HOME") or os.path.join(work, "user"))
    args = list(host_args)
    front = "--front" in host_args
    if front:
        # The front end lists this folder: the route's character only.
        save_dir = os.path.join(work, os.path.splitext(name)[0] + "-saves")
        os.makedirs(save_dir, exist_ok=True)
        args += ["--save-dir", save_dir]
    if save_args is not None:
        if front:
            save = os.path.join(save_dir, save_name(save_args) + ".d2s")
        else:
            save = os.path.join(work, os.path.splitext(name)[0] + ".d2s")
        r = subprocess.run([os.path.join(rel, "d2s-tool"), "new"] + save_args + ["-o", save],
                           capture_output=True, text=True, env=env)
        if r.returncode != 0:
            raise RouteError(f"d2s-tool new: {(r.stderr or r.stdout).strip()}")
        if not front:
            args = ["--save", save] + args
    err = open(os.path.join(work, os.path.splitext(name)[0] + ".stderr"), "w")
    p = subprocess.Popen([os.path.join(rel, "D2RUST-Game"), "autoplay-host", "--mute"] + args, stdin=subprocess.PIPE,
                         stdout=subprocess.PIPE, stderr=err, text=True, env=env, bufsize=1)

    def read():
        while True:
            line = p.stdout.readline()
            if not line:
                return None
            if line.startswith("{"):
                return json.loads(line)

    def send(line):
        p.stdin.write(line + "\n")
        p.stdin.flush()
        return read()

    milestone, passed, failed = None, [], None
    try:
        hello = read() or {}
        if hello.get("k") != "hello":
            raise RouteError("the host did not start")
        if hello.get("tick_rate") != 60:
            raise RouteError("host is not 60 Hz; rebuild the client before replay")
        for no, t in steps:
            if t.startswith("# milestone "):
                milestone = t[len("# milestone "):].split(":")[0]
                continue
            if t.startswith("#"):
                continue
            if t.startswith("check "):
                st = send("state")
                if st is None or st.get("k") != "state":
                    failed = (no, milestone, f"host gone or no state: {st}")
                    break
                why = check(st, t)
                if why:
                    failed = (no, milestone, why)
                    break
                if milestone and milestone not in passed:
                    passed.append(milestone)
                continue
            r = send(t)
            if r is None:
                failed = (no, milestone, "the host exited (see the .stderr file)")
                break
            if r.get("k") == "error":
                failed = (no, milestone, f"host error: {r['error']}")
                break
    finally:
        if failed:
            try:
                failure = {"failure": failed, "state": send("state"), "ui": send("ui"), "map": send("map")}
                with open(os.path.join(work, os.path.splitext(name)[0] + ".failure.json"), "w") as out:
                    json.dump(failure, out, indent=2)
            except (OSError, ValueError):
                pass  # A dead host may no longer answer diagnostic reads.
        try:
            p.stdin.write("quit\n")
            p.stdin.flush()
            p.wait(timeout=30)
        except Exception:  # noqa: BLE001 - the host may be gone
            p.kill()
        err.close()
    return passed, failed


def selftest():
    host, save, steps = parse(f"{HEADER}\n# c\nhost --seed 7\nsave --name A --class sor\n"
                              "step 3\npress L 1 2\n# milestone m: x\ncheck level 2\ncheck sent 2F >= 1\n")
    assert host == ["--seed", "7"] and save == ["--name", "A", "--class", "sor"]
    assert [t for _, t in steps][1:] == ["step 3", "press L 1 2", "# milestone m: x", "check level 2",
                                         "check sent 2F >= 1"]
    st = {"local": 1, "snap": {"units": [{"ut": 0, "g": 1, "lv": 2}]}, "client": {"sent": {"2F": 1}}}
    assert check(st, "check level 2") is None and check(st, "check level 3")
    assert check(st, "check sent 2F >= 1") is None and check(st, "check sent 30 >= 1")
    for bad in ["autoplay-route 1\n", "autoplay-route 1 tick_rate=25\n", "route 1\n", f"{HEADER}\nfly 1\n", f"{HEADER}\ncheck level\n"]:
        try:
            parse(bad)
        except RouteError:
            continue
        raise AssertionError(bad)
    assert ui_left(None) == 0 and ui_left("800x600") == 0 and ui_left("1068x600") == 134
    assert shift_step("press L 560 364", 134) == "press L 694 364"
    assert shift_step("move 10 20", 134) == "move 144 20"
    assert shift_step("key esc", 134) == "key esc" and shift_step("press L 5 6", 0) == "press L 5 6"
    assert shift_step("click ui:menu.single_player", 134) == "click ui:menu.single_player"
    _, _, ui_steps = parse(f"{HEADER}\nhost --front\nclick ui:menu.single_player\ntext Ab\nui\n")
    assert [t for _, t in ui_steps] == ["click ui:menu.single_player", "text Ab", "ui"]
    assert save_name(["--name", "A", "--class", "sor"]) == "A"
    for bad in [f"{HEADER}\nclick 1 2\n", f"{HEADER}\nclick ui:a ui:b\n"]:
        try:
            parse(bad)
        except RouteError:
            continue
        raise AssertionError(bad)
    for line in ("clickunit 1 154", "rclickunit 2 119 0 -8", "interactunit 1 154", "useunit 2 119", "walklevel 2"):
        assert parse(FORMAT + "\n" + line)[2] == [(2, line)]
        assert shift_step(line, 134) == line
    st["client"]["npc_menu"] = {"class": 154, "talking": True}
    assert check(st, "check client npc_menu.class == 154") is None
    assert check(st, "check client npc_menu.class == 148")
    assert check(st, "check client npc_menu.talking == true") is None
    st["client"]["npc_menu"] = None
    assert check(st, "check client npc_menu.class == 154")
    for f in glob.glob(os.path.join(ROUTES, "*.route")):
        try:
            parse(open(f).read(), f)
        except RouteError as e:
            assert "re-record" in str(e)  # Committed legacy recordings remain refused.
    # Check the environment passed to the host without launching the game.
    import io
    from unittest.mock import Mock, patch

    with tempfile.TemporaryDirectory(prefix="autoplay-env-") as work:
        route = os.path.join(work, "isolated.route")
        with open(route, "w") as f:
            f.write(HEADER + "\n")
        for overrides in ({}, {"D2RUST_LOG_DIR": os.path.join(work, "custom-logs"),
                               "D2RUST_HOME": os.path.join(work, "custom-user")}):
            host = Mock(stdout=io.StringIO('{"k":"hello","tick_rate":60}\n'), stdin=io.StringIO())
            with patch.dict(os.environ, overrides, clear=True), patch.object(
                    subprocess, "Popen", return_value=host) as popen:
                assert replay(route, None, work) == ([], None)
            env = popen.call_args.kwargs["env"]
            assert env["D2RUST_LOG_DIR"] == overrides.get(
                "D2RUST_LOG_DIR", os.path.join(work, "isolated", "logs"))
            assert env["D2RUST_HOME"] == overrides.get(
                "D2RUST_HOME", os.path.join(work, "user"))
    print("selftest ok")
    return 0


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("routes", nargs="*")
    ap.add_argument("--all", action="store_true", help="every route in tools/autoplay/routes/")
    ap.add_argument("--game-dir", default=os.environ.get("D2_GAME_DIR"))
    ap.add_argument("--selftest", action="store_true")
    ap.add_argument("--res", help="play frame WxH (800x600, 640x480, 1068x600); routes are 800x600")
    a = ap.parse_args(argv)
    if a.selftest:
        return selftest()
    routes = sorted(glob.glob(os.path.join(ROUTES, "*.route"))) if a.all else a.routes
    if not routes:
        ap.error("name a route or --all")
    work = tempfile.mkdtemp(prefix="autoplay-replay-", dir=os.path.join(REPO, "target"))
    rc = 0
    for r in routes:
        try:
            passed, failed = replay(r, a.game_dir, work, a.res)
        except (RouteError, OSError) as e:
            print(f"{os.path.basename(r)}: ERROR {e}")
            return 3
        if failed:
            no, m, why = failed
            rc = 1
            print(f"{os.path.basename(r)}: FAIL at line {no} (milestone {m}): {why}; passed {len(passed)}: {' '.join(passed)}")
        else:
            print(f"{os.path.basename(r)}: ok, {len(passed)} milestones: {' '.join(passed)}")
    print(f"work dir {work}")
    return rc


if __name__ == "__main__":
    sys.exit(main())
