---
id: test-automation-tools
title: Test automation tools for AI lanes
area: tools
status: in-progress
order: 1
modules: T0-T10
light: T0-T4
depends_on: []
---

# Plan: test automation tools for AI lanes

Status: **in progress**, top priority, full pass T0-T10 (owner, 2026-10-17).
T0-T2, T4-T7 and T10 are merged; T3 has a follow-up in review, alongside T8 and T9.
It's built with [`docs/FEATURE-METHOD.md`](../../FEATURE-METHOD.md).

## Goal

A bug lane (an AI session) can reproduce a game bug, watch it, fix it and prove the fix
**without a human and without taking over the PC**. The lane:
- sees what the player sees;
- drives the game the way a player does;
- checks the result as pass/fail.

All of this happens in seconds, headless where possible.

Owner rules this plan keeps:
- **Determinism stays intact.** Tools only read state, or feed input through the paths a
  player's input already takes. Nothing here changes a tick's result. State output and frame
  hashes stay bit-identical at the sim's tick rate (25 Hz today).
- **Dev tools stay on by default.** No module turns the dev panel or console off in normal play.
- **The owner uses this PC.** Default runs are headless or offscreen: no focus stealing, no
  cursor capture, no real mouse. A visible window is allowed for 1 minute or less.
- **Clean room.** Captured frames show the player's game art, so they're never committed and
  never leave BIGRIG. Only hashes, numbers and our own scenario text go into the repository.

## Baseline before this plan (2026-10-16)

All paths are in `crates/d2-client/src` unless the row says otherwise.

| Tool | What it does | What's missing for lanes |
|---|---|---|
| `play` (`main.rs::parse_options`, `app/play.rs`) | The real game. `--save F` or `--new CLASS NAME`, `--seed`, `--difficulty`, `--res`, `--frames N` (exit after N frames), `--input`, `--poke`, `--send`, `--dev-start SLOT`. | Always a visible window (`DefaultPlugins`). Any check flag makes it a "check run": dev tools off and the 800x600 frame. So a check run can't use 16:9, the native UI pass or the smooth camera, which is where BUG-012 lived. |
| `--input` script (`world_view/input_script.rs`) | Steps separated by `;`: `frame F`, `wait N`, `move/click/rclick X Y`, `wclick/wrclick`, `hold X Y N` (left only), `key/wkey/keydown/keyup K`, `clickunit/rclickunit T C [DX DY]`. Works in `play` and `state-dump`. | No independent mouse-button down and up (BUG-009 needs left held while right clicks). No "cast skill X at unit Y", no hotkey by action name, no UI click by element. Points are screen pixels, so scripts go stale when the layout changes. |
| `autoplay-host` (`app/autoplay_host.rs`) | The real client on `MinimalPlugins`: no window, a 40 ms stepping clock, UI included. stdin protocol `autoplay-1` (`step`, `move`, `press/release L\|R X Y`, `key`, `rawkey`, `state`, `map`, `quit`), with JSON replies. `state` returns player, camera, open UI, NPC menus, ground items, hover, message counts and the server `state-1` snapshot. | It's the closest thing to a repro engine, but it has no pixels and no screenshots, can't be attached to a windowed game, and can't build a test character without a save file. |
| Routes (`tools/autoplay/replay.py`, `routes/*.route`) | `autoplay-route 1`: host args, host commands, and `check level`, `check sent ID >= N`, `check open\|closed UI`, `check client FIELD == JSON`. Exit 0/1/3. | Hand-recorded pixel routes go stale (the recorder was dropped). Checks are few. Needs Python and `d2s-tool` to make the save. |
| `state-dump` (`app/state_dump.rs`, `d2-sim/src/debug/state.rs`) | Headless with no Bevy: one `state-1` JSON line per tick (43 fields per unit: type, class, mode, position, frame, life, stats and more). Byte-deterministic. Takes `--poke`, `--send`, `--input`. | No client UI. Only the keys in `HEADLESS_KEY_ACTIONS`. Output is raw: each lane writes its own reader. |
| Scenario script v1 (`scenario_script.rs`) | A strict, versioned line format: `name`, `seed`, `difficulty`, `char` (class, level, map, area, position, stats, skills, waypoints, quests, full items), `at T msg\|hex\|spawn\|poke`, `snapshot`, `end`. Parser and writer, used by `--send`. | No runner in this repo. It builds a test character from text, which is what the BUG-008 test did by hand in Rust. |
| Pokes (`d2-sim/src/poke.rs`) | Setup directives at a tick: `pos`, `warp`, `hop`, `goto`, `spawn`, `superunique`, `object`, `missile`, `item`, `stat`, `state`, `freeze`, `time`, `msg`, `operate`, `talk`, seeds. | Fine as is. They need a home in a scenario file. |
| Dev panel and console (`app/dev/*`, `d2-proto/src/dev.rs`) | 25 commands (`god`, `level`, `gold`, `warp`, `item`, `spawn`, `kill`, `speed`, `nocd`, `save/load` slot, `bugreport`, ...). Backquote opens it. | Can't be scripted: no file or command-line feed. Check runs turn it off. |
| Captures | `play --dump-draws DIR [--at-tick N] [--dump-image] [--dump-native WxH]`: the CPU-composed frame as PNG, then exit. `bugreport` takes a Bevy screenshot of the window. `verify` compares CPU vs GPU byte for byte on `render-cases/` and `capture-cases/`. | Nothing captures the **presented** GPU picture (smooth camera, native UI pass, between-tick frames) without a visible window. PrintScreen is bound to `Action::Screenshot` with no handler. |
| Frame hashes, determinism | `world_view/tests.rs::cpu_frame_hash_is_golden` (synthetic frame), `verify --perturb`, `tests/prop_worldsim.rs` (two runs, same bytes), `app_state_dump.rs` (byte-identical dumps), `STEP_MS = 40`. | No golden state or frames on real data. |
| Front-end and UI tests | `tests/front_end*.rs`, `smoke_frontend.rs`, `ui_clip_*.rs`, `app_hud_e2e.rs` on `MinimalPlugins` with synthetic data, in CI. `game_front_end_shots.rs` writes PNGs on real data (ignored). | Fine for CI. Lanes can't reuse them as quick ad hoc repros. |
| Real-data tests | `#[ignore]` tests reading `D2_GAME_DIR`. `tests/real_rig/mod.rs` `Rig` (spawn a monster, select a skill, right-click a unit, read position and mode). | Each repro is a Rust test to write and compile (minutes per change). |
| Perf (`app/perf.rs`, `d2-server/src/perf.rs`, `tools/perf/perf.py`) | `D2_PERF_OUT=file` writes `perf 1` JSON at exit: per frame (interval, pre, update, post, flags tick/drew/waited/motion/atlas), per render frame, per server tick (drain, tick, flush), peak RSS. `perf.py` gives p50/p95/max per scene. | No hitch list, no cause per hitch (room load, save, atlas), no "picture changes per second". The PERF lane wrote its own analysers and drove the real mouse with PowerShell (`SetCursorPos`, foreground window) to walk, plus a screen-region sampler for visual fps. That takes over the owner's PC. |
| Soak (`app/soak/`, `tools/soak/soak.py`) | Seeded random input with invariant checks (desync, GUID reuse, hangs), delta-debug `reduce` to the smallest input log. | Good at finding bugs. Its logs aren't scenarios a lane can extend. |
| CI (`.github/workflows/ci.yml`, `windows-build.yml`) | fmt, clippy, scoped nextest (synthetic data only), Windows build, a no-game-files smoke and `crash-test`. | Nothing runs on real data, a GPU, a window, perf or soak. |

**How bug lanes worked before this plan.**
- **BUG-008** (cast dashes at a monster): reproduced headless with a Rust test that builds a level 30 Sorceress in code and right-clicks a monster through `state_dump`. It worked, but took a compile per try.
- **BUG-009** (held walk stops after a cast): a unit test in `controls`, because no script can hold the left button while the right one clicks.
- **BUG-013** (gold label without its amount): needed the client's label text, which `state-dump` can't see.
- **BUG-010, 011, 014** (low fps, stutter, stepped monsters): windowed 25-60 s runs with `D2_PERF_OUT`, the real mouse driven by PowerShell, then ad hoc Python. The fix still ended in "NEEDS OWNER RUN" because nothing headless sees presentation.
- **BUG-012** (screen edges smear while moving): GPU shader behaviour between ticks at 16:9. No tool can capture it, because check runs force 800x600 and the CPU dumps don't run the smooth camera. It was fixed by reasoning plus a unit test.

## Modules

| Rank | Id | Module | Tier | Size | Depends on |
|---|---|---|---|---|---|
| 0 | T0 | Scenario file v2 and `repro` runner (headless, on autoplay-host) | light | 4 h | none |
| 1 | T1 | Checks and queries: `check` lines plus a `query` command | light | 3 h | T0 |
| 2 | T2 | Input steps: mouse down/up per button, `cast SKILL at unit`, `action NAME`, `wait-until` | light | 2-3 h | T0 |
| 3 | T3 | Perf summary: hitches with causes (including audio spans), picture-change rate, JSON verdict (**in review**, PR #208) | light | 2-3 h | none |
| 4 | T4 | Bug ticket to scenario workflow (template, folder, "fails before, passes after") | light | 1 h | T0, T1 |
| 5 | T5 | Offscreen GPU run: hidden window, real swapchain path, PNG shots at ticks (**done**, PR #228) | full | 1 day | T0 |
| 6 | T6 | Fixed display clock: between-tick frames at chosen phases, repeatable shots | full | 3-4 h | T5 |
| 7 | T7 | Visual A/B diff: tolerant image compare between two builds, diff image (**in review**, PR #236) | full | 3-4 h | T5 |
| 6 | T6 | Fixed display clock: between-tick frames at chosen phases, repeatable shots (**done**, PR #242) | full | 3-4 h | T5 |
| 7 | T7 | Visual A/B diff: tolerant image compare between two builds, diff image | full | 3-4 h | T5 |
| 8 | T8 | Live control channel for `play`: the `autoplay-1` protocol over a pipe, plus pause, step and shot (**in review**, PR #241) | full | 1 day | T1, T5 |
| 9 | T9 | UI clicks by element id instead of pixels (T9a **in review**, PR #237; T9b **in review**, PR #261; item grids **in review**, PR #313; controls/HUD **in review**, PR #315; character/skills **in review**, PR #305) | full | 1 day | T2 |
| 10 | T10 | CI and train hooks: scenario suite per PR, perf budgets on BIGRIG (**in review**, PR #247) | full | 4-6 h | T1, T3, T4 |
| Rank | Id | Module | Tier | Size | Depends on | Status |
|---|---|---|---|---|---|---|
| 0 | T0 | Scenario file v2 and `repro` runner (headless, on autoplay-host) | light | 4 h | none | merged: #186, #239 |
| 1 | T1 | Checks and queries: `check` lines plus a `query` command | light | 3 h | T0 | merged: #215 |
| 2 | T2 | Input steps: mouse down/up per button, `cast SKILL at unit`, `action NAME`, `wait-until` | light | 2-3 h | T0 | merged: #217 |
| 3 | T3 | Perf summary: hitches with causes, picture-change rate, JSON verdict | light | 2-3 h | none | in review: #300 (core merged: #208) |
| 4 | T4 | Bug ticket to scenario workflow (template, folder, "fails before, passes after") | light | 1 h | T0, T1 | merged: #220 |
| 5 | T5 | Offscreen GPU run: hidden window, real swapchain path, PNG shots at ticks | full | 1 day | T0 | merged: #228 |
| 6 | T6 | Fixed display clock: between-tick frames at chosen phases, repeatable shots | full | 3-4 h | T5 | merged: #242 |
| 7 | T7 | Visual A/B diff: tolerant image compare between two builds, diff image | full | 3-4 h | T5 | merged: #236 |
| 8 | T8 | Live control channel for `play`: the `autoplay-1` protocol over a pipe, plus pause, step and shot | full | 1 day | T1, T5 | in review: #241 |
| 9 | T9 | UI clicks by element id instead of pixels | full | 1 day | T2 | in review: #237, #261 (remaining screens open) |
| 10 | T10 | CI and train hooks: scenario suite per PR, perf budgets on BIGRIG | full | 4-6 h | T1, T3, T4 | merged: #247 |

The light tier (T0-T4) moves logic and input bugs (BUG-008, 009 and 013 style) to a headless
repro in seconds, and turns perf runs into a one-line verdict. The full tier adds pixels (T5-T7),
live driving (T8), stable UI scripts (T9) and CI (T10). The light tier is larger than the usual
1-hour box. Split it over two boxes: T0-T2 first, then T3 and T4.

### T0. Scenario file v2 and `repro` runner

- **What:** a lane writes one small text file that says who plays where, what they do, and what
  must be true, then runs `D2RUST-Game.exe repro FILE`. It returns pass/fail in seconds, with no
  window, and leaves its artifacts in a folder.
- **Format:** extend the existing scenario script (`scenario_script.rs`, version 1 → 2) instead of
  inventing one:
  - **Start:** `char` lines (class, level, skills, items, area, position) or `save FILE`;
    `seed`, `difficulty`.
  - **Setup:** `at T poke ...` (existing directives) and `at T dev CMD` (dev console lines,
    through the same `DevChannel` the panel uses).
  - **Input:** `at T input STEP` using the `--input` step grammar.
  - **Checks:** from T1.
  - **End:** `end T`.
  - A v1 file still parses.
- **Engine:** run it on **autoplay-host** (full client including UI, headless, stepping clock),
  not on `state-dump`, so client-side bugs (labels, held buttons, open panels) are reachable.
  The runner feeds the host in-process, not over stdin.
- **Output:** `OUT/repro.json` (format `repro 1`) with pass/fail, the first failing line,
  ticks run, wall time, the build version and a `state-1` dump of the last ticks. Exit codes:
  0 pass, 1 a check failed, 2 the scenario is invalid, 3 game files are missing.
  `--keep-going` runs all checks.
- **Files:** `scenario_script.rs` (v2 lines), new `app/repro.rs` (the runner), `main.rs`
  (subcommand), `app/autoplay_host.rs` (expose the in-process step API), new
  `docs/tools/scenarios.md` (format reference).
- **Seam:** `repro::run(&Scenario, &RunOpts) -> ReproReport`.
- **Done when:**
  - a scenario rebuilding BUG-008's setup (a Sorceress with Fire Ball right-clicks a Fallen)
    runs in under 10 s on BIGRIG;
  - two runs give byte-identical `repro.json` apart from the wall time;
  - parser tests for every new line kind, and v1 round trips;
  - a synthetic-data scenario test (`test-fixtures` town) runs in CI.
- **Risks:** autoplay-host drifts from `play` (its own plugin set). Mitigation: share the plugin
  list with `play` where possible, and keep one parity scenario that runs both ways.
- **Status: merged** (#186, #239). The runner landed in #186; #239 added the follow-ups:
  - The end-to-end test (`crates/d2-client/tests/app_repro.rs`) runs in CI on
    `test_fixtures::play::playable`. That is the five-act synthetic set plus the act palettes,
    the walk-back field, 14 stand-in fonts and two HUD images (control panel, cursor). Each
    repro run takes about 2.5 s in a debug build. The same check on real game files stays
    `#[ignore]`.
  - Building that install first took about 80 s in a debug build. Two small fixes cut it to
    about 18 s. The five-act merge (`acts.rs`) cloned a whole table for every lookup (33 s,
    now 8 s). The new large files were also being compressed (28 s); they now go in a stored
    list.
  - Release timing on BIGRIG (`D2RUST-Game repro tools/scenarios/bugs/BUG-008.scn`, real game
    files, other lanes building, CPU at 85-100 %): 6.8, 9.6 and 6.2 s, so under the 10 s
    target. Startup alone (game data load, server start and join, `end 1`) took 3.5-4.4 s. The
    40 BUG-008 ticks took the rest, 2-5 s depending on load. A debug build takes about 35 s,
    26 s of it in the data load.

### T1. Checks and queries

- **What:** state questions answered by name, both as asserts in a scenario and as a one-off
  query a lane runs while exploring.
- **Check lines:** `at T check EXPR` and `by T check EXPR` (true at some tick up to T). Examples:
  - `player.pos ~ (5120,3400) 2` (within 2 sub-tiles);
  - `player.mode == cast`;
  - `unit(monster, fallen).anim_frame changes`;
  - `ground_item(gold).label == "1234 Gold"`;
  - `hud.life == 145`;
  - `ui.open(inventory)`;
  - `sent(0x0C) >= 1`.
- **Names:**
  - `server.*` from the `state-1` snapshot;
  - `client.*` and `ui.*` from the autoplay `state` reply;
  - `hud.*` from the HUD model the UI draws from (numbers, not pixels).
  - One table maps each name to its source, so new names are one line each.
- **Query:** `repro FILE --query "player.pos, unit(monster,*).mode" --at 40,80` prints JSON.
  The same `query` works as an autoplay-host command.
- **Files:** new `app/repro/query.rs` (names and the expression parser), `app/autoplay_host.rs`
  (`query` command), the `state` reply (add ground item label text and HUD values where missing).
- **Done when:** unit tests for the parser and each source; BUG-013's label check fails on a
  build without #139 and passes with it; every check failure prints the actual value.
- **Risks:** label text is built in UI systems. If it isn't in a model the host can read, T1
  adds a small read-only accessor and doesn't move UI code.
- **Status:** merged, PR #215. The `hud` reply reads the server's stat totals for
  the local player; label text comes from `Original::ground_label_texts`.

### T2. Input steps

- **What:** scripts say what a player does, not only where they click.
- **New steps** (in the shared `--input` grammar, so `play`, `state-dump` and scenarios all get
  them):
  - `down L|R X Y` and `up L|R X Y` (independent buttons; BUG-009);
  - `move-held X Y` (drag while held);
  - `skill L|R NAME` (select by skill name);
  - `cast NAME at unit T C` (select the skill, then `rclickunit`);
  - `cast NAME at X Y`;
  - `action NAME` (any keymap action by its name in `controls/names.rs`, e.g. `action
    toggle_inventory`, `action toggle_run`);
  - `wait-until EXPR max N` (needs T1).
- **Files:** `world_view/input_script.rs`, `controls/names.rs` (lookup only), the T0 runner.
- **Done when:** parser tests per step; a scenario holds left, casts with right and checks the
  walk resumes (BUG-009); `cast fireball at unit 1 *` works headless.
- **Risks:** `state-dump` refuses keys outside `HEADLESS_KEY_ACTIONS`. `action` reports "not
  available headless" there instead of skipping silently.
- **Status:** merged, PR #217. Skill names come from the player's `skills.txt` (new
  `world_view/script_skills.rs`). `wait-until` reads the host's `state` reply, so only `repro`
  runs it; `play` and `state-dump` refuse it. A step that can't run fails the `repro` run. The
  BUG-009 scenario is `tools/scenarios/bugs/BUG-009.scn`.

### T3. Perf summary

- **What:** one windowed run of 1 minute or less, or a headless one, gives a verdict a lane can
  read without writing Python.
- **Changes:**
  - `perf 1` → `perf 2` (the old reader still accepts `perf 1`);
  - a `summary` block: fps, ticks/s, drawn ticks/s, waited frames, p50/p95/p99/max per phase
    (pre, update, post, render, server drain/tick/flush);
  - **hitches:** every frame over `--hitch-ms` (default 50), with its phase and the events in it
    (room loads, save write, atlas upload, settings write, audio stream). The events are marks
    the perf log already has room for;
  - **picture changes per second:** counted in the render world when the presented image
    differs from the last one. This replaces the screen-sampling PowerShell probe;
  - **budgets:** `--budget tick=40,frame=17,hitch=0` sets the exit code.
- **Input:** perf runs take `--input` or a scenario (T0/T2) for walking, so nobody drives the
  real mouse.
- **Files:** `app/perf.rs`, `d2-server/src/perf.rs` (marks), new `perf-summary FILE` subcommand
  in `main.rs`, `tools/perf/perf.py` (reads the summary instead of recomputing).
- **Done when:** a unit test feeds synthetic frames and gets the expected hitches; a 30 s
  windowed walk on BIGRIG prints the verdict. On BUG-010's build it shows the drawn-ticks gap
  the PERF lane found by hand.
- **Risks:** timing on a shared PC is noisy. Budgets are verdicts for a lane, not CI gates (T10).
- **Status:** in review (#300, audio spans); core merged in #208. Built on main as:
  - `perf 2` adds each frame's start on the report clock and its picture flags (bit 1: a new tick
    picture, bit 2: a new between-tick picture), each tick's start, `spans` (`save`, `room`,
    `atlas`, `settings`, `audio`) and the `summary` block. Picture changes are read from the compose and
    motion jobs in `Last`, read-only. Audio spans cover front-end stream loading and WAV decoding;
    `atlas`, `settings`) and the `summary` block. Picture changes are read from the compose and
    motion jobs in `Last`, read-only. Audio stream events are not marked on main yet (#300 is open);
  - `perf-summary FILE [--hitch-ms N] [--budget tick=40,frame=17,hitch=0] [--json]`: `tick` and
    `frame` are p95 limits in ms, `hitch` the most hitches; exit 0 pass, 1 over budget, 2 bad
    input. It reads `perf 1` too (no clock, flags or spans there);
  - `tools/perf/perf.py` reads both formats and takes rates and hitches from the summary.
  - Still to do: the 30 s windowed walk on BIGRIG, using the merged T2 `--input` walking.

### T4. Bug ticket to scenario workflow

- **What:** every bug ticket gets a scenario, and a fix PR proves itself with it.
- **Changes:**
  - `docs/bugs/README.md` gets a "Repro" section in the ticket template;
  - new `tools/scenarios/bugs/BUG-NNN.scn` holds one scenario per ticket;
  - the rule: the fix PR adds the scenario, it fails on `main` and passes on the branch, and the
    PR body pastes both `repro` lines;
  - reproducing a player's report uses `repro` instead of hand-editing `slot.txt` (steps in `docs/bugs/README.md`);
  - a `bugreport` state line maps to scenario `char`/`poke` lines (documented, maybe a
    `repro --from-report` later).
- **Files:** docs only, plus the folder.
- **Done when:** BUG-008, 009 and 013 have scenarios in the folder that pass on main.
- **Risks:** none. Scenarios with real data run only on BIGRIG until T10.
- **Status:** merged, PR #220. The rule and the ticket template are in `docs/bugs/README.md`, with
  the bug report to scenario table. `docs/TRIAGE.md` is left out because it is being removed with
  the triage module. `tests/bug_scenarios.rs` checks every file in the folder in CI and runs
  them all on real data (ignored). BUG-008 now checks the cast plays in place; BUG-013 came with
  T1; BUG-009 comes with T2 (#217), since it needs per-button down and up.

### T5. Offscreen GPU run

- **Status: merged** (PR #228). `play --new CLASS NAME|--save F --offscreen [--size WxH]
  [--shot-at T,..] [--shot-every N] --shot-dir DIR` (code: `world_view/shots.rs`). The hidden
  window works on BIGRIG (RX 9070 XT, Vulkan): the screenshot reads the window's final image, so
  the offscreen-target fallback wasn't needed. The window is borderless so a screen-sized
  `--size` isn't shrunk by the system, and a fatal error in an offscreen run never opens a
  message box. Dev tools stay on unless the run has pokes or sends (they share one server hook).
- **What:** `play --offscreen` runs the real GPU path (16:9, native UI pass, smooth camera,
  between-tick frames) with no visible window and no focus, and writes PNGs of the **presented**
  picture: `--shot-at T[,T..]`, `--shot-every N`, `--shot-dir DIR`, `--size 1920x1080`.
- **How:**
  - A hidden window (`Window { visible: false, focused: false }`, no cursor grab or confine),
    with the presented image read back after the native UI pass, the same readback `verify` and
    `bugreport` use.
  - If Windows throttles presentation for a hidden window, fall back to an offscreen render
    target of the same size and run the final pass into it. The PNG is then the final pass
    output, not the swapchain (vsync and present-mode bugs still need a window).
  - **Check runs stop forcing 800x600 and dev-off when `--offscreen` is given.** `--res` and the
    user's video settings apply, so lanes test what players see. The default check run keeps
    today's behaviour.
- **Files:** `app/play.rs` (window setup, `is_check_run` split), new `world_view/shots.rs` (the
  readback-to-PNG system, reusing `d2-report` encoding), `main.rs`.
- **Done when:**
  - a 300-tick offscreen walk at 1920x1080 writes 10 PNGs while the owner's foreground app
    keeps focus;
  - tick output (`state-1`) is identical with and without `--offscreen`;
  - a shot of BUG-012's case shows clean edges on main.
- **Risks:**
  - Hidden-window behaviour differs between drivers (Intel HD 630, BUG-001).
  - GPU memory on a busy PC.
  - Shots are game art: they go to the lane's work folder only (owner rule above).

### T6. Fixed display clock

- **What:** between-tick frames depend on wall-clock time, so a smear or jump that happens at
  "40% between ticks" is hard to catch twice. `--display-clock fixed:FPS` (or
  `phases:0,0.25,0.5,0.75`) drives the presentation clock from the frame count instead of the
  wall clock. Frames render as fast as the GPU allows, and the same frames are drawn every run.
- **Files:** `world_view/present.rs` (there's already a `FrameSchedule` seam), `app/play.rs`.
- **Seam:** the display clock source. The sim's tick clock is not touched.
- **Done when:** two offscreen runs give identical PNG hashes for the same `--shot-at`; BUG-012
  and BUG-014 cases can be pinned to a phase.
- **Risks:** low. It only affects interpolation, never ticks.
- **Status: merged** (PR #242). `play --offscreen --display-clock fixed:FPS` or
  `phases:0,0.25,0.5,0.75` (`world_view/display_clock.rs`). Frame `n` gets a fixed time, and
  that one clock feeds Bevy's `Time` (smooth camera, unit motion), the bridge's host clock and
  the server's host clock. The server needs it too: with the wall clock, the frame a tick lands
  on would still vary from run to run. Tick rules are unchanged; ticks just come every 40 ms of
  display clock. With phases, the first phase must be 0 (the frame the tick runs on), and the
  other phases are the between-tick frames. Only for `--offscreen` runs. `--shot-phase P` takes
  each planned shot on the frame at phase `P` of its tick (`tick-NNNNNN-fK.png`). Shots key on
  the tick on screen (the world view's last drawn tick), not the newest tick received. The other
  wall clocks that change pixels follow the display clock too (the Esc menu's pentagrams, the
  light events' wall seconds), and the frame limiter is off. Checked on the GPU: two runs of
  `--new sorceress --seed 7 --size 1280x720` walking in town gave identical PNG hashes at ticks
  60 and 120, with `fixed:60` and with `phases:0,0.25,0.5,0.75 --shot-phase 0.5`; the phase 0.5
  shots differ from the phase 0 ones, so a between-tick frame (BUG-012's camera smear, BUG-014's
  unit motion) can be pinned and shot again.

### T7. Visual A/B diff

- **Status: merged** (PR #236). A small crate `tools/img-diff` (binary `img-diff`, `image` crate,
  no Bevy): `img-diff A B [--tol N] [--max-pct P] [--mask hud,belt,edges:N,rect:X,Y,W,H]
  [--out DIR]`. Writes `diff.png` (or `<name>.diff.png` per folder pair) and `verdict.json` to
  `--out` (default `img-diff-out`), prints the verdict; exit 0 pass, 1 fail, 2 bad input. The
  verdict has the changed share, the largest channel difference and the box around the changes.
  Defaults are strict (`--tol 0 --max-pct 0`). `hud` and `belt` are rectangles in the 800 x 600
  UI box, scaled to the shot's height and centred (`hud` widened to the full shot width, as the
  control panel is in a 16:9 frame). Not wired as a `D2RUST-Game.exe` subcommand
  (that would make every use build `d2-client`).
- **What:** `D2RUST-Game.exe img-diff A B --tol 3 --max-pct 0.5 --mask hud`. It compares two
  PNGs per pixel with a small colour tolerance and an allowed share of changed pixels, writes
  `diff.png` (changes in red) and a JSON verdict. It also takes a folder pair, so a lane runs the
  same scenario shots on `main` and its branch and diffs them.
- **No stored references:** references would be game art in the repo. Comparison is always
  build against build on BIGRIG. Optional: only per-shot hashes go in the repo, as a "nothing
  changed" check.
- **Files:** new `tools/img-diff` crate or a `d2-client` subcommand (no Bevy needed; `image`
  crate). Masks are named regions (`hud`, `belt`, `edges:N`).
- **Done when:** unit tests on synthetic images (identical, 1 px off, shifted, within tolerance);
  a before/after pair for BUG-012 shows the edge strip only.
- **Risks:** tolerance tuning. Start strict and loosen per mask.

### T8. Live control channel for `play`

- **Status:** in review (#241).

- **What:** drive a real (offscreen or windowed) game step by step: pause, step N ticks, query,
  send input, take a shot, resume. It reuses the `autoplay-1` line protocol so routes, scenarios
  and lanes share one vocabulary, and adds `pause`, `resume`, `step-ticks N`, `shot PATH`,
  `query EXPR`.
- **Transport:**
  - stdin/stdout when the lane starts the game (`play --control stdio`);
  - a named pipe `\\.\pipe\d2rust-<pid>` (`--control pipe`) to attach to a running game;
  - no network port.
- **Status:** in review, PR #241. The pause is the bridge's Esc-menu pause path (a `held` flag
  beside it), not the dev `speed` command, which is the player's velocity, not a game clock.
- **Pause and step:** go through the game's existing speed and pause path (the dev `speed`
  command), so a paused game is just a game at speed 0 and stepping runs whole ticks.
- **Files:** new `app/control.rs`, `app/autoplay_host.rs` (move the protocol to a shared module),
  `app/play.rs`.
- **Done when:** a test drives `play --offscreen --control stdio` through pause, step 10, query
  and shot; a run driven step by step gives the same `state-1` as the same input scripted.
- **Risks:**
  - A pipe a local process can open: off unless `--control` is given (D-T2).
  - Bevy's frame loop while paused must keep rendering without ticking.

### T9. UI clicks by element id

- **Status:** in review (#237 front end, #261 in-game panels); remaining screen coverage is open.

- **What:** `click ui:inventory.slot(3,2)`, `click ui:options.video.vsync`,
  `click ui:menu.single_player`. The UI's layout code registers each clickable element's id and
  rectangle per frame, and the input step looks the id up. Scripts survive layout, resolution
  and 16:9 changes.
- **Files:** `ui/*` (registration at the points that already compute hit rectangles),
  `world_view/input_script.rs`, `app/autoplay_host.rs` (`ui` command lists ids on screen).
- **Done when:** the act-probe routes are rewritten with ids and pass at 800x600 and 1068x600;
  an unknown id fails with the list of ids on screen.
- **Risks:** touches many UI files. Do it screen by screen (T9a front end, T9b in-game panels),
  each its own PR.
- **Status:** T9a (front end) in review, PR #237 (stacked on #217). Ids are named per screen in
  `ui/front_end/screens/element_ids.rs` from each control's action, with the control's hit box on
  the current frame; screens that hit-test their own rows add them through `Screen::ui_elements`
  (options rows). `autoplay-host --front --save-dir DIR` starts on the main menu and takes
  `click ui:ID`, `ui` and `text WORD`; the act-probe routes walk the front end by id.
  T9b (in-game panels) in review, PR #261 (stacked on #237): `Panel::ui_elements` lists a panel's
  ids from its hit-test layout, the root lists the open panels top-most first (an id a higher
  panel would take the click from is left out) and the host moves them from the 800-wide UI box
  into the frame. Ids: `npc.menu(N)`, `npc.topic(N)`, `dialog.skip`, `waypoint.tab(N)` /
  `row(N)` / `close`, the Esc menu's `options.<page>.<row>`.
  Controls/HUD follow-up in review, PR #315 (lane cx-t9a, stacked on #261): `controls.binding(CMD,SLOT)` uses the
  command number and binding slot (1 primary, 0 secondary), only for non-separator rows in the
  current scroll window; `controls.default` / `accept` / `cancel` / `scroll_up` / `scroll_down`
  work in the front end and Esc overlay. `hud.run` / `minipanel` / `skill_left` / `skill_right` /
  `hp_text` / `mp_text` / `new_stats` / `new_skills` / `help`, `minipanel.character` / `inventory` /
  `skills` / `party` / `automap` / `message_log` / `quests` / `game_menu`, `belt.slot(N)` (zero
  based), `quests.slot(N)` / `close`, and `message_log.scroll_up` / `scroll_down` / `close` use
  their current pointer hit rectangles and handlers. Closed, hidden, covered and unreachable
  targets are omitted. Synthetic pointer-handler tests cover 800x600 and 1068x600 layouts.
  Remaining scope: party and chat have state/hotkey handling but no installed mouse-panel
  adapters to register internal controls. Automap's toggle is `minipanel.automap`; its settings
  already use `options.automap.*`; pan/zoom and other keyboard actions have no clickable
  rectangles. These gaps need UI adapters before ids can expose them without new click paths.
  Item grids (cx-t9b) in review, PR #313 (stacked on #261), now list
  `inventory.slot(col,row)` / `equip(body_loc)`, `stash.slot(col,row)` / `close`,
  `cube.slot(col,row)` / `transmute` / `close`, and `shop.slot(col,row)` / `tab(N)` /
  `confirm.yes` / `confirm.no`. Cells include empty slots and use current hit boxes;
  confirmation dialogs cover underlying ids, and cube animation hides covered cells.
  Synthetic root-dispatch tests exercise clicks at 800x600 and 1068x600.
  The in-game act-probe migration remains.

  Character and skill panels are merged, PR #305 (lane cx-t9c,
  based on #261): `character.stat(N)` (stat id 0/2/3/1) / `character.close`,
  `skilltree.tab(N)` (1–3) / `skilltree.skill(N)` (skill id) / `skilltree.close`.
  Current hit rows and strict icon/tab hit regions supply the rectangles; unavailable stat
  buttons, unlearnable skills while spending points and skills on hidden pages are omitted.
  With no free skill points, non-passive visible icons use the existing skill-selection intent.
  Where the close button overlaps a bottom-row skill, each id clicks an uncovered part of its
  existing hit region, so the skill remains available and closing emits no skill intent.
  Synthetic clicks cover spending, selection, tabs, close and unknown ids at 800x600 and 1068x600.

### T10. CI and train hooks

- **Start small (owner, 2026-10-17):** begin with a short smoke set of scenarios per PR and only a few perf budgets. Keep the load inside the BIGRIG resource guard (85% CPU, 60 GB RAM, 80% disk), and grow the suite only when it fits.

- **What:**
  - **Every PR:** synthetic-data scenarios (`test-fixtures`) run in the existing `test` job.
  - **Real-data scenarios** (`tools/scenarios/**`, the bug repros): a job on BIGRIG's runner
    with the game files assembled locally (`tools/assemble.py`), run by the merge train once per
    batch, not per push.
  - **Perf budgets:** report-only at first. They become a gate only when BIGRIG's numbers are
    steady (D-T3).
- **Files:** `.github/workflows/ci.yml` (a new job with a runner label), `tools/ci/`, the
  train's batch script.
- **Done when:** a batch run prints a table of scenario results; a failing bug scenario turns
  the batch red.
- **Risks:**
  - Game files on a CI runner must stay on BIGRIG (clean room rule 1).
  - Runner time: keep the suite under 2 minutes.
- **Status:** merged, PR #247. Built as:
  - **Smoke set, every PR:** `crates/d2-client/tests/scenario_smoke.rs` runs the scenarios of
    `crates/d2-client/tests/smoke/` (a town walk, dev lines and a level-up) through `repro` on a
    synthetic install (`test_fixtures::acts::all_acts` plus the few made-up files the headless
    client reads at start), plus a failing check and a twice-the-same-run test; about 10 s. The
    `test` job's nextest step runs it when d2-client is in scope; a `scenario smoke` step runs it
    otherwise. Panels that need UI art (the inventory) and gold piles are out of the synthetic set.
  - **Real data, once per batch:** ci.yml job `scenarios`, only on the train's batch PRs
    (`lc/batch-*`), on `gate-heavy`. `tools/ci/game_files.py` assembles the MPQs with
    `tools/assemble.py` on the runner's own volume (`/cache/game` if mounted, else the target
    volume) and rebuilds only when `game-data/manifest.json` changes; nothing is uploaded.
    `tools/ci/scenarios.py` builds the game once and runs every `tools/scenarios/**/*.scn`
    through `repro --keep-going` on that binary, and prints one table (`scenario | ...` rows). A failing scenario fails the job and so the
    batch; the perf budget (`perf-summary --budget`, default `tick=40,frame=17,hitch=0`) and the
    2-minute limit are only reported. `repro` now writes its `D2_PERF_OUT` report before exiting.
  - **Train:** a patch for LC's batch script prints the table in the batch's result post.

## What each module unlocks for bug lanes (baseline comparison)

| Ticket | Today | With the plan |
|---|---|---|
| BUG-008 cast dashes at a monster | Rust test written by hand, a compile per try | T0 + T1 + T2: a 10-line scenario (`cast fireball at unit`, `check player.pos ~ start 0`), seconds per try |
| BUG-009 held walk stops after a cast | unit test only; no script could hold left while right clicks | T2 `down L` / `down R` / `up R`, T1 `by T check player.mode == walk` |
| BUG-010 low fps | windowed runs, real mouse via PowerShell, own Python | T3 verdict from a scripted 30 s walk; T5 offscreen when the window isn't the question |
| BUG-011 stutter every few seconds | frame logs read by hand | T3 hitch list with the cause per hitch (room load, save, atlas) |
| BUG-012 edges smear while moving | not reproducible: check runs force 800x600, the CPU dumps have no smooth camera | T5 offscreen 16:9 shots, T6 pinned phase, T7 diff before/after |
| BUG-013 gold label without amount | needed client label text | T1 `ground_item(gold).label` check |
| BUG-014 stepped monster animation | owner run | T1 `anim_frame` checks per tick; T5 + T6 shots between ticks; T3 drawn-ticks rate |

## Decisions

The owner decided all four on 2026-10-16 (the recommended defaults).

- **D-T1 (decided): where captured frames live.**
  - Shots are game art, so they never go in the repo and are never uploaded.
  - Captured frames and other artifacts stay in a lane's work folder on E: (for example
    `E:/work/artifacts/<lane>/`), deleted at DONE. A diff image may be shown to the owner in a
    thread only as a local path.
- **D-T2 (decided): the live control channel (T8).**
  - Off unless `--control` is given on the command line. stdin or a named pipe only, never a
    network port.
  - Dev tools themselves stay on by default as now.
- **D-T3 (decided): real-data scenarios in CI (T10).**
  - They run only on BIGRIG's runner, once per merge-train batch, with MPQs assembled on the
    runner's disk (D:).
  - Perf budgets are report-only until the owner says gate.
- **D-T4 (decided): check runs at the player's resolution.**
  - An `--offscreen` check run (T5) uses the player's resolution and video settings, with dev
    tools on.
  - Plain check runs don't change: they keep 800x600 and dev-off, so existing tests and hashes
    don't move.

## Not planned

- **Driving the real OS mouse or keyboard** (the PERF lane's PowerShell): it takes the owner's
  PC. T2/T3 scripted input covers it.
- **Screen-region sampling for visual fps:** replaced by T3's in-engine picture-change count.
- **Golden reference images in the repository:** clean room. T7 compares builds instead.
- **A new script language:** v2 of the existing scenario script plus the shared `--input`
  grammar is enough.
- **Network-socket control or remote driving from the cloud:** the lanes are local; a pipe
  covers it without opening a port.
- **Reviving the old recorder bot for routes:** T9 element ids make hand-written scripts stable
  instead.
