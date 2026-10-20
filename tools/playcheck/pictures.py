"""Picture checks on the running game: flicker, hover highlight shape, text through the upscaler.

Each check drives hidden games over the `--control stdio` channel, takes
shots of the presented picture and hands them to `img-check`'s probes
(tools/img-diff/src/probe.rs). The game runs at `--res 800x600` in a window
of an exact multiple of that frame (`--scale`, 2 by default), so a frame
pixel is a known block of shot pixels.

flicker (owner report E3: shadows flicker)
    Per render setting (`--variants`), in town and in the Blood Moor, with
    the player standing still: `held` takes frames with the game held (no
    tick runs; every frame must be the same picture, `stationary-change`),
    `ticking` alternates a tick's first frame and a held frame after it.
    `flicker` fails when pixels go A -> B -> A in consecutive frames.

hover (owner report E1: entrances highlight as a square)
    Per level (`--targets`: the player arrives at the level's warp tiles
    until one is next to an entrance), the pointer is moved over the entrance (probed
    until the game reports the tile hovered), and shots without and with the
    hover are compared: `highlight-rect` / `highlight-tile` fail (a filled
    or outlined box, a floor cell), `highlight-none` when nothing lit up,
    a level where no entrance could be reached (`no-tile`) or no pointer
    position hovered it (`not-hovered`) is a counted skip.

text (friend report F4: text doesn't upscale or run in HD)
    Per display setting (`--texts`, `--scales`), with the game held: a shot
    without and with the Esc menu; the menu's text pixels and the world
    around them must have the same upscale factor (`text-scale-mismatch`);
    `hd-text-no-effect` when the HD text setting draws the same picture as
    Upscaled.
"""
import json
import subprocess
from pathlib import Path

try:
    from . import offscreen
except ImportError:
    import offscreen

FRAME = (800, 600)

VARIANTS = {
    "default": {},
    "original": {"animation_frames": "original"},
    "solid-shadows": {"blended_shadows": 0},
}

FLICKER_SCENES = {"town": [], "blood-moor": ["--poke", "30 warp 2"]}

# Levels whose entrances are hovered: the player arrives at the level's warp
# tiles in turn (`warp LEVEL tile N`, N = 0..3) until one lands next to an
# entrance tile. Blood Moor (Den of Evil), Cold Plains (cave), Black Marsh
# (tower), Den of Evil (its way out), Lut Gholein (sewers), Rocky Waste
# (tomb), Dry Hills (Halls of the Dead), Far Oasis (Maggot Lair), Spider
# Forest (caves), Flayer Jungle (dungeon), Frigid Highlands (cave).
TARGETS = "2,3,6,8,40,41,42,43,76,78,111"
# How near (in sub-tiles) an entrance must be to the arrival point.
NEAR = 40


def img_check(opts, source, *args):
    cmd = [str(opts.img_check), str(source), *map(str, args)]
    p = subprocess.run(cmd, capture_output=True, text=True)
    if p.returncode not in (0, 1):
        raise RuntimeError(f"img-check failed ({p.returncode}): {' '.join(cmd)}\n{p.stderr}")
    return json.loads(p.stdout)


def game(opts, folder, args, video, scale=None):
    scale = scale or opts.scale
    size = f"{FRAME[0] * scale}x{FRAME[1] * scale}"
    run = offscreen.Run(opts.bin, folder, ["--new", "sorceress", "Pictures", "--seed", "7",
                                           "--res", f"{FRAME[0]}x{FRAME[1]}", *args],
                        video=video, size=size, game_dir=opts.game_dir, perf=False)
    run.start(control=True)
    return run, offscreen.Control(run, timeout=opts.timeout)


def shot(c, path):
    reply = c.send(f"shot {Path(path).as_posix()}")
    want = [int(n) for n in c.run.size.split("x")]
    if reply.get("size") != want:
        raise RuntimeError(f"shot {path} is {reply.get('size')}, not the window size {want}")
    return {"path": Path(path).name, "tick": reply.get("tick", 0), "phase": 0}


def flicker_case(opts, variant, scene):
    folder = opts.out / "flicker" / f"{scene}-{variant}"
    run, c = game(opts, folder, FLICKER_SCENES[scene], VARIANTS[variant])
    result = {"case": f"{scene}-{variant}"}
    try:
        c.send(f"step-ticks {opts.settle}")
        c.send("step 8")
        held = []
        for i in range(opts.frames):
            c.send("step 1")
            held.append(shot(c, folder / f"held-{i:02}.png"))
        ticking = []
        for i in range(opts.frames):
            c.send("step-ticks 1")
            ticking.append(shot(c, folder / f"tick-{i:02}a.png"))
            c.send("step 1")
            ticking.append(shot(c, folder / f"tick-{i:02}b.png"))
    finally:
        c.quit()
        run.wait(60)
    for name, frames in (("held", held), ("ticking", ticking)):
        manifest = folder / f"{name}.json"
        offscreen.write_json(manifest, {"frames": frames})
        v = img_check(opts, manifest, "--flicker", opts.min_pixels, "--tol", 2)
        result[name] = v["violations"]
    result["violations"] = [dict(x, sequence=n) for n in ("held", "ticking") for x in result[n]]
    return result


def screen_point(state, pos):
    """Frame pixel of a static unit's feet, from the `state` camera."""
    cam = state["client"]["camera"]
    a, b = cam["x16"] >> 11, cam["y16"] >> 11
    px, py = (a - b) >> 1, (a + b) >> 2
    dx, dy = cam.get("shake", [0, 0])
    ux, uy = (pos[0] - pos[1]) * 16, (pos[0] + pos[1]) * 8
    return ux - px + FRAME[0] // 2 - dx, uy - py + FRAME[1] // 2 - 8 - dy


def arrive(opts, folder, level):
    """A game standing next to an entrance tile of the level: first at its
    warp tiles 0..3 (`warp LEVEL tile N`), then brought by `goto preset LEVEL
    5:ROW` to the level's warp links the `map` reply lists. (run, control, state, tile, how, tried)."""
    tried, seen = [], {}
    attempts = [(f"tile-{n}", ["--poke", f"30 warp {level} tile {n}"]) for n in range(4)]
    i = 0
    while i < len(attempts):
        how, pokes = attempts[i]
        i += 1
        run, c = game(opts, folder / how, pokes, {})
        try:
            c.send(f"step-ticks {opts.settle if how.startswith('tile') else opts.settle + 480}")
            state = c.send("state")
            me = state["client"]["player"]["pos"]
            tiles = state["client"].get("tiles") or []
            near = [t for t in tiles
                    if abs(t["pos"][0] - me[0]) <= NEAR and abs(t["pos"][1] - me[1]) <= NEAR]
            tried.append({"how": how, "area_tiles": len(tiles), "near": len(near)})
            if near:
                unit = min(near, key=lambda t: (t["pos"][0] - me[0]) ** 2 + (t["pos"][1] - me[1]) ** 2)
                return run, c, state, unit, how, tried
            # Entrances the client has not loaded yet: the level's warp
            # links from the `map` reply, reached by `goto preset`.
            if how.startswith("tile") and len(seen) < 2:
                for lv in c.send("map").get("levels") or []:
                    if lv.get("id") != int(level):
                        continue
                    for room in lv.get("rooms") or []:
                        for wl in room.get("warps") or []:
                            row = wl.get("row")
                            if row is not None and row not in seen and len(seen) < 2:
                                seen[row] = True
                                attempts.append((f"goto-{row}", [
                                    "--poke", f"30 goto preset {level} 5:{row}"]))
        except Exception:
            c.quit()
            run.wait(60)
            raise
        c.quit()
        run.wait(60)
    return None, None, None, None, None, tried


def hover_case(opts, level):
    folder = opts.out / "hover" / f"level-{level}"
    result = {"case": f"level-{level}", "violations": []}
    run, c, state, unit, how, tried = arrive(opts, folder, level)
    result["arrivals"] = tried
    if run is None:
        result["skip"] = f"no-tile: no entrance within {NEAR} sub-tiles of warp tiles 0-3 or a goto"
        return result
    folder = folder / how
    try:
        # A new act loads its art for a while after the arrival: wait until
        # the picture is no longer black (one colour) before hovering.
        for attempt in range(5):
            shot(c, folder / "ready.png")
            ready = img_check(opts, folder / "ready.png")
            if not any(v["rule"] in ("black", "one-colour") for v in ready["violations"]):
                break
            c.send("step-ticks 120")
        else:
            result["skip"] = "black: the picture stayed one colour after the arrival"
            return result
        state = c.send("state")
        fx, fy = screen_point(state, unit["pos"])
        result.update(tile=unit, feet=[fx, fy], arrival=how)
        # Probe points around the feet, nearest to a point a little above
        # them first (entrance art stands up from its feet).
        points = sorted(((fx + dx, fy + dy) for dy in range(-160, 41, 8) for dx in range(-96, 97, 16)
                         if 4 <= fx + dx < FRAME[0] - 4 and 4 <= fy + dy < FRAME[1] - 60),
                        key=lambda p: (p[0] - fx) ** 2 + (p[1] - fy + 40) ** 2)
        hit = None
        for x, y in points[:opts.probes]:
            c.send(f"move {x} {y}")
            c.send("step-ticks 1")
            if c.send("state")["client"].get("hover") == [5, unit["g"]]:
                hit = (x, y)
                break
        if hit is None:
            result["skip"] = f"not-hovered: no pointer position of {min(len(points), opts.probes)} hovered it"
            return result
        result["pointer"] = list(hit)
        # Pointer away (a corner of the play area that hovers nothing), on
        # the entrance, away again: light that changes between the ticks
        # is in both outer shots, the highlight only in the middle one. Each
        # shot waits two ticks after its move: the hover pick runs on a tick
        # and the picture showing it is built on a later one.
        away = (8, 8)
        for away in ((8, 8), (FRAME[0] - 8, 8), (8, FRAME[1] - 70)):
            c.send(f"move {away[0]} {away[1]}")
            c.send("step-ticks 2")
            if c.send("state")["client"].get("hover") is None:
                break
        shot(c, folder / "base-before.png")
        c.send(f"move {hit[0]} {hit[1]}")
        c.send("step-ticks 2")
        if c.send("state")["client"].get("hover") != [5, unit["g"]]:
            result["skip"] = "not-hovered: hover lost on the second move"
            return result
        shot(c, folder / "hover.png")
        c.send(f"move {away[0]} {away[1]}")
        c.send("step-ticks 2")
        shot(c, folder / "base-after.png")
    finally:
        c.quit()
        run.wait(60)
    s = opts.scale
    ignore = []
    for x, y in (hit, away):
        ignore += ["--ignore", f"{max(0, (x - 8) * s)},{max(0, (y - 8) * s)},{48 * s},{48 * s}"]
    v = img_check(opts, folder / "hover.png", "--highlight", folder / "base-before.png",
                  "--highlight", folder / "base-after.png", *ignore)
    result["metrics"] = v.get("metrics", {}).get("highlight")
    result["violations"] += v["violations"]
    return result


def text_case(opts, text_mode, scale):
    folder = opts.out / "text" / f"{text_mode}-x{scale}"
    run, c = game(opts, folder, [], {"text_mode": text_mode}, scale=scale)
    try:
        c.send(f"step-ticks {opts.settle}")
        c.send("step 8")
        shot(c, folder / "base.png")
        c.send("key ESC")
        c.send("step 12")
        shot(c, folder / "text.png")
    finally:
        c.quit()
        run.wait(60)
    # World: below the menu, above the control panel, clear of the corners.
    world = f"{120 * scale},{360 * scale},{560 * scale},{180 * scale}"
    v = img_check(opts, folder / "text.png", "--text-base", folder / "base.png", "--world", world)
    return {"case": f"{text_mode}-x{scale}", "metrics": v.get("metrics", {}).get("text_scale"),
            "violations": v["violations"], "mode": text_mode, "scale": scale, "folder": folder}


def changed_pct(opts, a, b, out):
    """Share of pixels that differ between two shots (img-diff)."""
    tool = opts.img_check.with_name(opts.img_check.name.replace("img-check", "img-diff"))
    p = subprocess.run([str(tool), str(a), str(b), "--out", str(out)], capture_output=True, text=True)
    if p.returncode not in (0, 1):
        raise RuntimeError(f"img-diff failed ({p.returncode}): {p.stderr}")
    v = json.loads(p.stdout)
    return v["changed_pct"]


def hd_effect(opts, results):
    """`hd-text-no-effect`: at a scale, the HD text setting draws the very same
    picture as Upscaled (the option does nothing there)."""
    by = {(r["mode"], r["scale"]): r for r in results}
    for (mode, scale), r in by.items():
        other = by.get(("upscaled", scale))
        if mode != "hd" or other is None:
            continue
        text = changed_pct(opts, other["folder"] / "text.png", r["folder"] / "text.png",
                           r["folder"] / "vs-upscaled")
        base = changed_pct(opts, other["folder"] / "base.png", r["folder"] / "base.png",
                           r["folder"] / "vs-upscaled-base")
        r["hd_vs_upscaled"] = {"text_changed_pct": text, "base_changed_pct": base}
        if text == 0:
            r["violations"].append({"rule": "hd-text-no-effect", "scale": scale})


def common_args(p):
    p.add_argument("--bin", required=True)
    p.add_argument("--img-check", required=True, type=Path, help="the img-check executable")
    p.add_argument("--game-dir")
    p.add_argument("--out", required=True, type=Path)
    p.add_argument("--scale", type=int, default=2, help="window = this many times the 800x600 frame")
    p.add_argument("--settle", type=int, default=180, help="ticks to run before the first shot")
    p.add_argument("--jobs", type=int, default=3)
    p.add_argument("--timeout", type=float, default=180)


def finish(opts, name, results):
    violations = [dict(v, case=r["case"]) for r in results for v in r["violations"]]
    skips = {r["case"]: r["skip"] for r in results if r.get("skip")}
    verdict = {"format": f"playcheck-{name} 1", "pass": not violations, "violations": violations,
               "skips": skips, "cases": results}
    offscreen.write_json(opts.out / name / "verdict.json", verdict)
    for r in results:
        print(f"{r['case']}\t" + (", ".join(sorted({v['rule'] for v in r['violations']}))
                                   or (f"skip ({r['skip']})" if r.get("skip") else "pass"))
              + (f"\t{json.dumps(r['metrics'])}" if r.get("metrics") else ""))
    print(f"{name}: {'pass' if verdict['pass'] else 'FAIL'} ({len(violations)} violations, "
          f"{len(skips)} skipped); "
          f"{opts.out / name / 'verdict.json'}")
    return 0 if verdict["pass"] else 1


class Flicker:
    __doc__ = "Flicker: still scenes stay stable frame to frame."

    @staticmethod
    def add_args(p):
        common_args(p)
        p.add_argument("--variants", default=",".join(VARIANTS))
        p.add_argument("--scenes", default=",".join(FLICKER_SCENES))
        p.add_argument("--frames", type=int, default=12, help="frames per sequence")
        p.add_argument("--min-pixels", type=int, default=24)

    @staticmethod
    def main(opts):
        opts.out.mkdir(parents=True, exist_ok=False)
        cases = [(v, s) for v in opts.variants.split(",") for s in opts.scenes.split(",")]
        return finish(opts, "flicker", offscreen.jobs_map(lambda c: flicker_case(opts, *c), cases, opts.jobs))


class Hover:
    __doc__ = "Hover: an entrance's highlight follows its graphics, not a box or a floor tile."

    @staticmethod
    def add_args(p):
        common_args(p)
        p.add_argument("--targets", default=TARGETS, help="level ids whose entrances are hovered")
        p.add_argument("--probes", type=int, default=120, help="pointer positions tried per entrance")

    @staticmethod
    def main(opts):
        opts.out.mkdir(parents=True, exist_ok=False)
        return finish(opts, "hover", offscreen.jobs_map(lambda t: hover_case(opts, t),
                                                        opts.targets.split(","), opts.jobs))


class Text:
    __doc__ = "Text: menu text is enlarged like the world around it, at every text and display setting."

    @staticmethod
    def add_args(p):
        common_args(p)
        p.add_argument("--texts", default="upscaled,hd", help="text_mode settings")
        p.add_argument("--scales", default="2,3", help="window sizes as multiples of the frame")

    @staticmethod
    def main(opts):
        opts.out.mkdir(parents=True, exist_ok=False)
        cases = [(t, int(s)) for t in opts.texts.split(",") for s in opts.scales.split(",")]
        results = offscreen.jobs_map(lambda c: text_case(opts, *c), cases, opts.jobs)
        hd_effect(opts, results)
        for r in results:
            r["folder"] = str(r["folder"])
        return finish(opts, "text", results)
