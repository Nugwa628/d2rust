# Local picture checks

Q6 uses the existing T5 hidden-window renderer and T6 fixed display clock. No
visible windows, OS input, or uploads are used. Reference PNGs and reports stay
outside the repository: `E:/work/artifacts/golden/` on BIGRIG, `/cache/golden/`
on the CI server. Every heavy build goes through BIGRIG's shared build queue.

Build the two small tools with `cargo build -p img-diff` (through that queue).
They do not link the client. Both return 0 for pass, 1 for a picture violation,
and 2 for invalid input or an operational failure.

## Capture

```powershell
py -3 tools/ci/shots.py capture tools/scenarios/pictures --sha HEAD --bin PATH/TO/D2RUST-Game.exe
py -3 tools/ci/shots.py compare BASE_SHA HEAD_SHA --out E:/work/artifacts/my-lane/report --img-diff PATH/TO/img-diff.exe --img-check PATH/TO/img-check.exe
```

`--bin` is optional once Q1's `tools/ci/bincache.py` is available: `get SHA`
must print the cached executable path, without building. A supplied binary is
used as given; the capture records its path and SHA-256, so a local test build
can be distinguished from its source SHA. For dylib builds, put the binary's
`deps` and rustup host `lib` directories on PATH (LD_LIBRARY_PATH on Linux), as
with the existing scenario runner. `--game-dir` defaults to `D2_GAME_DIR`.

`SCENARIOS` is a JSON shot manifest, an array of manifests, or a directory of
`*.json` manifests. This is the screenshot adapter for `play`, separate from
`repro`'s `.scn` checks. It makes the complete play command explicit, rather
than silently dropping unsupported repro setup or checks. An example lives at
`tools/scenarios/pictures/town-walk.json`:

```json
{
  "name": "town-walk",
  "tick_rate": 60,
  "args": ["--new", "sorceress", "Picture", "--seed", "7", "--input", "frame 72; click 700 300"],
  "ticks": [144, 147],
  "phases": [0, 0.25, 0.5, 0.75],
  "size": "1280x720"
}
```

The `warp-*` manifests warp a new character from the act 1 start straight
into Pandemonium Runs 1-3 and Uber Tristram (levels 133-136): each of these
levels is drawn with another act's palette than the act it belongs to, so
they cover a level whose palette is not its act's, reached across acts.

Q3 generators can emit this same manifest, with chosen ticks and their setup
in `args`, to feed picture checks. Allowed play options are `--new CLASS NAME`
or `--save FILE`, `--seed`, `--difficulty`, `--input`, `--poke`, `--send`, and
`--res`. Save paths are relative to the manifest. Capture owns the offscreen
flags, clock, shot destination, and end condition. A phase list always starts
at zero and increases by at least 1 ms of the tick at `tick_rate` (1–1000 Hz;
legacy manifests default to 25 Hz). Phase milliseconds use `1000 / tick_rate`.
Raw input schedules and capture ticks must be authored for the selected binary's
game rate; the checked-in manifest is retimed to 60 Hz. Ticks are positive,
unique and sorted. Defaults: ticks 60/120, phases 0/.25/.5/.75, size 1280x720.

T5 selects one shot phase per game process, so capture runs each phase
separately, twice by default (`--repeats N`). Each process has isolated user,
save and log folders; input only comes from scripts. `--timeout` limits each
process (180 seconds by default). All requested PNGs must exist before a run
is marked complete. A failed capture retains its logs and incomplete marker
and cannot be compared. Destinations are never overwritten.

The layout is `GOLDEN/SHA/SCENARIO/rREPEAT-pPHASE/tick-NNNNNN[-fK].png`, with
`capture.json` at the SHA root and `frames.json` in each scenario. The manifest
records the source SHA, executable hash, complete command settings, tick,
phase in milliseconds, and repeat index. `--root` changes GOLDEN; `--label`
creates a named experiment directory (for example `same-a`, `same-b`, `offset`).

## Compare and read the report

`compare [BASE] HEAD` pairs every scenario and shot by name, fails missing
scenarios/shots or changed capture settings, runs `img-diff` on each pair, and
runs `img-check` on HEAD's repeated phase sequence. Defaults are exact pixels;
`--tol N`, `--max-pct P` and repeatable `--mask hud|belt|edges:N|rect:X,Y,W,H`
pass through to `img-diff`. The tolerance also applies to temporal checks.
Masks and allowed percentages apply to reference diffs, not black-frame,
missing-color or layout rules. Keep settings the same between captures.

With only HEAD, the base is the newest complete captured ancestor of HEAD in
Git's date order, excluding HEAD itself. With two arguments, capture labels
are used directly (or Git refs resolved to full SHAs); this supports same-SHA
repeats and planted-change experiments.

Open the printed `index.html` path from disk. Rows show base, head and red-pixel
diff images with clickable full-resolution links. The report also contains
reference-free violations and writes `verdict.json`; per-scenario `check.json`
and each `img-diff` verdict are retained. A report directory must be new. Nothing
is embedded in the repository or uploaded to a service.

## Reference-free rules

```powershell
img-check.exe frame.png
img-check.exe E:/work/artifacts/golden/SHA/town-walk/frames.json --tol 2 --jump 8 --missing 255,0,255 --out verdict.json
img-check.exe frame.png --ui ui-ids.json
```

Single PNGs or a folder of PNGs get spatial checks. A sequence manifest adds
temporal checks: `{"frames":[{"path":"a.png","tick":60,"phase":10}, ...]}`.
Paths are relative to the manifest, phase is milliseconds within a tick.

- `black` / `one-colour`: all RGB pixels are identical, regardless of alpha.
- `missing-texture`: an exact configured RGB appears (default magenta
  255,0,255). This is a color heuristic; choose the fallback color for the
  renderer being tested. Intentional use of that color can also trip it.
- `stationary-change`: repeated shots of the same tick and phase differ beyond
  the channel tolerance. This detects repeatability problems without guessing
  whether animated units should move between ticks.
- `phase-jump`: a between-tick image differs from both neighboring phases by
  more than `--jump` mean RGB levels (default 8), and by more than four times
  the difference between those neighbors (floor .25). Smooth changes stay
  quiet; a flash and return trips. Four consecutive phase samples also detect
  a sustained step whose middle edge exceeds both neighboring edges by that
  same factor. Only one tick's adjacent captured phases,
  including the next tick's phase-zero boundary, are considered. Sparse phases
  can miss a short flash; capture more phases to narrow it down.
- `ui-offscreen` / `ui-overlap`: optional T9 JSON, either its real `ui` reply
  (`ids` with `id` and `rect:[x,y,w,h]`), `elements`, or a bare array. Named
  x/y/w/h rectangles are accepted too. Coordinates must be in the image's
  pixel space. Nonpositive sizes fail; touching edges do not overlap, but
  containment does. Layout mistakes in the JSON return exit 2. An intentional
  overlap still needs review. Pass `compare --ui FILE` to check HEAD layouts.

Probes for bugs that need two or more pictures (`img_diff::probe`; driven on
the running game by `playcheck flicker|hover|text`, [playcheck.md](playcheck.md)):

```powershell
img-check.exe frames.json --flicker 24 [--flicker-delta 16] [--tol 2]
img-check.exe hover.png --highlight before.png --highlight after.png [--min-pixels 50] [--ignore X,Y,W,H]...
img-check.exe text.png --text-base base.png --world X,Y,W,H
```

- `flicker`: in the frames' order (the manifest's, or sorted names), a
  pixel that goes A -> B -> A (first and third within `--tol`, the middle
  more than `--flicker-delta` from both) flickers; a triple with at least
  `--flicker` such pixels fails. One violation reports the count of
  flickering triples, their union box and the worst triple.
- `highlight-rect` / `highlight-tile` / `highlight-none`: the pixels that
  got brighter (20 % and 12 levels) than every `--highlight` base (pointer
  away; give one from before and one from after, so light that changed
  between the shots is not taken for the highlight) in the picture (pointer
  on the target), `--ignore` boxes (the pointer) left out. Their biggest
  group fails as a rectangle when it fills 90 % of its box or only its
  outline; its solid part (holes closed, strokes under 7 px such as label
  text opened away) fails as a tile when half of its rows' side edges run at
  the 2:1 slope of floor cells: an isometric cell or block of cells instead
  of the hovered art's own shape (a tip hidden under the label does not
  matter). Nothing brightened is `highlight-none`.
- `text-scale-mismatch` / `text-none`: the pixels that differ between
  `--text-base` and the picture are the text; the unchanged pixels in
  `--world` are the world. Each gets an upscale factor (the block size k
  whose inside pixel pairs differ least against its block edges); the two
  must be equal: text goes through the same display upscaler as the world.
  The verdict's `metrics` holds both factors.

The checks read images and metadata only. They never modify game state or
auto-fix violations. Found game bugs remain **Unverified** until owner review.

Run Python tests with `py -3 -m unittest discover -s tools/ci -p test_shots.py`;
run the synthetic image rules with `cargo nextest run -p img-diff` through the
CI wait and shared build queue.
