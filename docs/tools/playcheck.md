# playcheck: checks on the running game

Some bugs only show while the real game draws frames: the frame limit not
holding, frames stalling, animation frames being made while playing.
`repro` and the sweeps run headless and never see them. `playcheck` plays
hidden, isolated games and checks stated rules on what happened, from the
game's own timing log and from outside the process.

```sh
python3 tools/playcheck/playcheck.py pacing  --bin EXE --out DIR [--game-dir D]
python3 tools/playcheck/playcheck.py animgen --bin EXE --out DIR [--game-dir D]
python3 tools/playcheck/playcheck.py flicker|hover|text --bin EXE --img-check EXE --out DIR
python3 tools/playcheck/playcheck.py selftest
```

`--bin` is a `D2RUST-Game` executable (`tools/ci/bincache.py get SHA`, or the
lane's own build). `--game-dir` defaults to `D2_GAME_DIR`. `--out` must be a
new folder; it gets one subfolder per run (command, stdout, logs, user
folder, perf report) and `verdict.json` (`pass`, `violations` with the
measured values, per-run metrics). Exit 0 pass, 1 a rule broke, 2 bad
arguments or the game could not run. `selftest` runs the rules on synthetic
data (planted faults) and needs no game.

## How a run is isolated

Each run is `play --new CLASS NAME --offscreen --no-sound --size WxH
--frames N` with its own `D2RUST_HOME` (saves, settings, caches) and
`D2RUST_LOG_DIR` in its run folder, so nothing touches Documents or another
run, and several runs can go at once (`--jobs`). Settings are a
`settings.toml` written into that folder before the start (`[video]` keys:
`vsync`, `frame_limit`, `animation_frames`, `text_mode`, ...). On Linux
without a display each run starts its own `Xvfb :N` (no TCP, no xauth); the
picture comes from the machine's Vulkan adapter (llvmpipe on the CI
server). On Windows `--offscreen` hides the window itself. No real mouse,
no focus, no network.

On the CI server run it through `scratch/remote.sh` with the lane's release
build: `sh remote.sh python3 tools/playcheck/playcheck.py pacing --bin
$CARGO_TARGET_DIR/release/D2RUST-Game --out out/pacing`, then `remote.sh
--fetch out/pacing/verdict.json`.

## pacing: the frame limit, even frames, no stalls, no skipped ticks

One run per frame limit (`--limits 30,60,120,0`, 0 is Off) and animation
frames setting (`--modes original,smooth`), VSync off so the game's own
limiter is in force, with `D2_PERF_OUT` timing. The rules read the `perf 2`
frame log (`frames`: interval, update phases, start time, new-picture
flags; `ticks`: when each game tick ran; `render`: render time per frame)
after a warm-up (`--warm 3` seconds after the first tick):

| Rule | Fails when |
|---|---|
| `cap-exceeded` | the rate is over the limit by more than `--rate-tol` % (2), or a one-second window holds more than the limit allows |
| `cap-missed` | the machine has headroom (p95 frame cost under 80 % of a period) but the rate is more than 5 % under the limit |
| `jitter` | the limit is reachable and the intervals (stalls aside) spread more than `--jitter-ms` (1.5) or p99 is over 1.5 periods |
| `stall` | an interval is over `--stall-ms` (50) and four periods (four median intervals with no limit) |
| `skipped-ticks` | more than `--skip-pct` % (2) of frames ran more ticks than the rate needs (one at 60 fps and up, two at 30), so a tick's picture was never shown |
| `smooth-costs-more` | Smooth's p95 interval is over Original's by more than 15 % plus 1 ms at the same limit |

The frame cost is the larger of the frame's update work and its render
time. When it is too high for a limit, the run reports `cap_reachable:
false` and only `cap-exceeded`, `stall` and `skipped-ticks` apply: a slow
software renderer can't hold 120 fps, but it must not get worse for trying.
Timing is noisy on a shared machine: keep the machine, size and load the
same when comparing, and rerun a borderline failure. A small `--size`
(`400x300`) gives the software renderer room to reach the limits.

## animgen: no animation frames made while playing

Owner rule: animation frames are made when the game or a mod is installed
and kept in files; the running game only loads them. The check plays
scenes with units on screen (`--scenes town,blood-moor`) with animation
frames on Smooth and fails on:

| Rule | Fails when |
|---|---|
| `generation-thread` | a game thread named like `--thread-pattern` (`anim`) runs while playing (Linux, `/proc/PID/task/*/comm`); its CPU time is reported |
| `generated-files` | the game wrote animation frame files (`*.d2frames`, `cache/animation/`) into its user folder |

Other files the game made in the user folder (apart from saves, logs and
settings) are listed per run as `new_user_files`, so generation that moves
somewhere else is still seen. On Windows only the file rule applies.

## Picture checks: flicker, hover highlight, text

These drive the game over `--control stdio` (held, stepped tick by tick,
the pointer moved by the channel), take shots and hand them to
`img-check`'s probes ([shots.md](shots.md)); `--img-check` names that
executable (`img-diff` must sit next to it). The game runs at
`--res 800x600` in a window of an exact multiple (`--scale`, 2), so one
frame pixel is a known block of shot pixels. `--jobs` (3) runs cases at once,
each with its own Xvfb display. Shots are game art: they stay in `--out`.

| Check | Cases | Fails on |
|---|---|---|
| `flicker` | town and Blood Moor, standing still, per render setting (`--variants default,original,solid-shadows`) | `held`: frames with the game held must be one picture (`stationary-change`); `ticking`: a tick's first frame and a held frame after it, alternating; `flicker` in either |
| `hover` | per level (`--targets`, the player arriving at its warp tiles, or brought by `goto preset` to the warp links the `map` reply lists, until one is next to an entrance): Den of Evil, caves, the tower, sewers, tombs, ... | `highlight-tile` (the highlight is floor cells, owner report E1), `highlight-rect`, `highlight-none`. Counted skips: `no-tile` (no arrival or `goto` reaches an entrance), `not-hovered` (no pointer position hovered it) |
| `text` | `text_mode` upscaled and HD (`--texts`) at 2x and 3x (`--scales`), the Esc menu over the world | `text-scale-mismatch` (text not enlarged like the world, friend report F4); `hd-text-no-effect` (HD draws the same picture as Upscaled) |

`hover` finds the entrance from the `state` reply (`client.tiles`, the
camera), probes pointer positions around its feet until `client.hover` names
it, then shoots with the pointer away, on it and away again (light that
changes between the ticks is in both outer shots, not taken for the
highlight).

## Adding a check

A check is a module in `tools/playcheck/` with `add_args(parser)` and
`main(opts)`, registered in `playcheck.py`'s `CHECKS`, using
`offscreen.Run` (and `offscreen.Control` for the `--control stdio`
channel). Its rules are pure functions over recorded data with planted
failure tests in `test_*.py`, and every violation carries the measured
value.
