---
id: smoothness-update
title: Smoothness update
area: game
status: in-progress
order: 3
modules: S1-S11
light: S1-S4
depends_on: [display-v2/V0]
---

# Plan: Smoothness update

Status: **in progress**. S1-S6 and S9 are merged (S2/S3/S5/S6 through Display v2);
S10d parts 1 and 2 are merged (#304, #317). S10e is implemented on
`codex/cx-s10e-60hz` (#335); the headless tick-cost and automated gates pass. Owner
play/sign-off is pending. S11 has an open PR. S7/S8 are in progress (owner, 2026-10-18: hold lifted). S7a–S7e proceed to
a local look check before S7f–S7h and S8. It's built with `docs/FEATURE-METHOD.md` and is order 3 in `docs/plans/README.md`.

## Goal

The game feels smooth and responsive on every monitor, from 60 Hz to 360 Hz:
- **game logic at 60 Hz** for every system, replacing the 25 Hz tick;
- **a 1000 Hz base clock** for backend tasks and later use;
- **more animation frames and facing directions**, so movement and actions look smooth, not only the camera;
- **display-rate smoothing** between ticks on monitors faster than the logic.

Owner rules:
- **VSync follows Display v2 V2:** On/Off with a frame limit (#85); this replaces the earlier locked-on scope.
- **360 fps is the maximum.**
- The player may pick a frame cap.
- **The 25 Hz tick goes.** All game logic runs at 60 Hz (S10), and a 1000 Hz base clock serves backend tasks and later use (S11).
- **New animation frames come from two sources (S7):**
  - **base-game frames** come from the player's own game files, so they're baked on the player's machine when the game is installed or updated (kept in the install's bake folder) and never shipped;
  - **mod frames** are baked the same way when the mod is installed, or made ahead of time by the mod's build. Nothing is generated while the game runs (owner, 2026-10-19).
- The light tier (S1–S4) fits in about 1 hour.

## Where things are on main

- **Tick:** S10e selects 60 Hz for the base game (automated gates pass; owner sign-off pending).
  - `TICKS_PER_SECOND` is in `crates/d2-sim/src/lib.rs`, and the host's tick schedule (`TickDriver`, at the game's `TickRate` since S10c) is in `crates/d2-server/src/host.rs`.
  - In single player it runs inside the client frame (`bridge/mirror.rs` → `Bridge::frame`).
  - S9 routes simulation timers, speeds, animation rates and table values through `TickRate`;
    the base game selects 60 Hz in S10e; table units remain at 25 Hz.
- **Drawing:** Display v2 V5/V6 (#81, #86) smooth the camera, units and missiles between ticks.
- **Cursor, vsync, frame cap, input:** done in S1–S4. The pointer is a hardware cursor at display rate. VSync is selectable, the frame cap goes up to 360, and input between ticks is queued.
- **Animation art:** units are composites. A COF (`d2-formats/src/cof.rs`) lists up to 16 component layers and the per-frame layer draw order. Each layer's pictures are DCC or DC6 frames (8-bit palette indices, 8, 16 or 32 directions). `animdata` gives frame counts, speeds and frame events (attack, missile, sound).
- **Positions:** moving units retain previous/current positions for Display v2 V6 interpolation (#86).

## Modules

| Rank | Id | Module | Tier | Size | Depends on | Status |
|---|---|---|---|---|---|---|
| 1 | S1 | Pointer at display rate (hardware cursor) | light | — | none | merged: #252 |
| 2 | S2 | VSync (scope replaced by Display v2 V2) | light | — | none | merged: #85 (Display v2 V2 replaces locked-on scope) |
| 3 | S3 | Frame cap, max 360 | light | — | V0 | merged: #85 (Display v2 V2) |
| 4 | S4 | No input lost between ticks | light | — | none | merged: #214 |
| 5 | S5 | Smooth camera between ticks | full | 3–4 h | S2 | merged: #81 (Display v2 V5) |
| 6 | S9 | Tick rate as a time unit | full | 2–4 days | none | merged: #122, #124, #126, #131, #135 |
| 7 | S10 | Game logic at 60 Hz | full | 3–5 days | S9 | S10d part 1 merged (#304); part 2 merged (#317); S10e automated and tick-cost gates pass; owner sign-off pending |
| 8 | S7 | More animation frames | full | 1–2 weeks | S10 | in progress (owner, 2026-10-18) |
| 9 | S8 | More facing directions | full | 2–3 days | S7 | in progress (owner, 2026-10-18); implementation follows S7e look check |
| 10 | S6 | Smooth unit and missile movement between ticks | full | 1 day | S5 | merged: #86 (Display v2 V6) |
| 11 | S11 | 1000 Hz base clock | full | 2–3 days | S9 | in review: #273 |

Ids are kept from the first version of this plan, so PRs and branches that already use them
(S9a, S5) stay valid. The rank column is the build order.

### S1. Pointer at display rate (done)

- **What:** the cursor is a hardware cursor that moves at full monitor refresh.
- **State:** merged (#252, restored after the #77 revert). `world_view/hw_cursor.rs` takes the cursor cel the UI draws last out of the frame and shows it as the window's cursor, scaled by the presented `sx`/`sy`, on play windows driven by the real mouse. A held item is that cursor too.
- **Note:** an item drawn translucent or lit by a gradient stays in the frame (a window cursor cannot blend), so it moves at the tick rate. Scripted and scheduled runs keep the cursor in the frame.

### S2. VSync (merged through Display v2 V2)

- **What:** #85 replaced the locked-on scope with VSync On/Off; On uses `Fifo`, Off chooses a supported low-latency mode.

### S3. Frame cap, max 360 (done)

- **What:** #85 restored the limiter with Off, 30, 60, 120, 144, 165, 240 or 360, set in the Video page.

### S4. No input lost between ticks (done)

- **What:** input is read every display frame, and presses between ticks queue for the next tick.
- **State:** merged. Restored in #214 after the #77 revert: `world_view/pointer_input.rs` reads every left and right press and release of a display frame from the window's button messages, in order, and the UI queue holds them until the next pass.

### S5. Smooth camera between ticks

- **What:** the view scrolls smoothly at display rate while the player walks.
- **Seam:**
  - the compositor writes two layers of each tick frame: the world, and the screen (UI and the local player);
  - between ticks the world layer is shown at a sub-pixel offset, from client-side timing;
  - tick frames are unchanged.
- **State:** merged through Display v2 V5 (#81), replacing the reverted #55 implementation.
- **Done when:** walking at 144 Hz scrolls with no tick stepping, and tick frames are identical (frame hash test).
- **After S10:** the logic runs at 60 Hz, so S5 matters most on monitors above 60 Hz.

### S9. Tick rate as a time unit

- **What:** every duration, speed, timer and rate in `d2-sim`, `tables/` and the protocol becomes a real-time value, converted to ticks for the current tick rate in one place.
- **Seam:**
  - a `TickRate` value carried by the simulation instead of the `TICKS_PER_SECOND` constant;
  - conversion helpers in one module;
  - tables keep their numbers, read as "at 25 Hz" and converted on load.
- **Done when:** at 25 Hz every existing test passes, and the simulation is bit-identical to today (determinism tests).
- **Split by system:**
  - S9a: movement (PR #52, reverted in #77, restored in #122);
  - S9b: timers and states (#124);
  - S9c: skills and missiles (#126);
  - S9d: animation rates and frame events (#131);
  - S9e: the protocol (#135).

  Each part keeps 25 Hz bit-identical, so S9 is safe to land before the rate changes.

### S10. Game logic at 60 Hz

- **Status:** in review (#297 S10b, #268 S10c, #304 S10d part 1). S10a merged (#157);
  S10d part 1 is merged (#304), part 2 is merged (#317), and S10e
  (the 60 Hz switch) is implemented (#335); the headless tick-cost gate passes on repeat.
  Automated validation passes; owner play/sign-off remains pending.

- **What:** the game runs at 60 ticks per second for every system: movement, combat, skills, missiles, AI, animations and their frame events.
- **Seam:**
  - S9's `TickRate` set to 60 for the base game;
  - the host loop and the client accumulator use the new tick length;
  - durations and speeds come from S9's real-time values, so the game plays at the same real-time speed.
- **Equivalence:**
  - walk and run speeds, attack and cast durations, cooldowns and regeneration match their 25 Hz real-time values: a 25 Hz time is only known to one 25 Hz tick, so the 60 Hz time must fall between one 25 Hz tick before and one 60 Hz tick after it (40 ms before, 16.7 ms after);
  - the report is `crates/test-fixtures/tests/rate_equivalence.rs`, on the synthetic Act I install so CI runs it; each system adds its rows as it is measured;
  - frame events (hit, cast, sound) land within one tick of their 25 Hz time;
  - new determinism baselines are recorded at 60 Hz, because bit-identity with 25 Hz no longer applies.
- **Which systems stay coarser:** none (owner, 2026-10-16). Every system runs at the base 60 Hz; no system keeps a coarser interval.
- **Split (one PR each, in order; 25 Hz stays bit-identical until S10e):**
  - **S10a, no silent base rate (sim):** the seam traits' `tick_rate()` loses its "base rate" default, so every world states its rate and none falls back to 25 Hz by accident. Real worlds return the game's rate; offline tools and test fakes say `TickRate::BASE` on purpose.
  - **S10b, equivalence report (sim):** whole-game runs at 25 and 60 Hz side by side, measuring in real time: walk and run speed, stamina, attack, cast and hit-recovery durations, frame events, cooldowns, timers and states, regeneration, missile range and lifetime, AI reaction and spawn timings. Each is listed with its tolerance (one 60 Hz tick) and fails CI when out of it. Mismatches found here are fixed in the sim before the switch.
    - *Status:* S10a merged (#157). S10b rows on main or in review: town walk and run (#158); stamina and mana regeneration, and a monster attack's hit event, first AI think and end for both synthetic monsters (second slice). All are inside the window, and none needed a sim fix. Still open: cast and hit-recovery durations, cooldowns, states with a duration, missiles, life regeneration and spawn timings. The synthetic set needs rows for these first (skill delays, state durations, a life regeneration stat, player animations).
  - **S10c, host schedule (`d2-server/src/host.rs`):** the `TickDriver` and the flush throttle take the game's `TickRate`. Tick *n* is due at `⌊n · 1000 / hz⌋` ms after the start (16, 17, 17 ms at 60 Hz), so there's no drift, and catch-up stays at most one tick. Games are made with the host's rate. Single player and the headless server use the same driver.
    - *Status:* done. The `Tick` seam states the game's rate (`Tick::tick_rate`, no default; `SimGame` returns `Game::tick_rate`), and `Host::new` builds its `TickDriver` at that rate, so the host and the game cannot disagree. The flush throttle is the rate's shortest tick (`TickDriver::tick_ms`, 40 ms at 25 Hz, 16 ms at 60 Hz). At 25 Hz the driver matches the fixed 40 ms schedule for any clock sequence (tested against the old algorithm); at 60 Hz 600 ticks land exactly on `⌊n · 1000 / 60⌋` over 10 s. Every host, single player's and the test rigs', goes through `Host::new`, so all use this driver.
    - *Status:* S10a merged (#157). S10b rows on main or in review: town walk and run (#158); stamina and mana regeneration, and a monster attack's hit event, first AI think and end for both synthetic monsters (second slice). All are inside the window, and none needed a sim fix. Third slice (S9-B): hit recovery for both monsters and the player, the player's cast, an arrow's lifetime and flight, poison, chill and stun durations, and life regeneration; the report's own synthetic set adds the get-hit and cast animations, the arrow's flight function, the life regeneration stat and the cold and stun states. Still open: skill cooldowns (the fixture session has no skill-use pipeline yet), spawn timings, and the life a poison takes (the player in the report's session loses none at either rate; to be looked at).
    - *Status:* in review (#297). #158 and #212 are on main (walk/run, stamina/mana regeneration, monster attack and AI). #297 adds hit recovery, player cast, arrow lifetime/flight, timed states and life regeneration. Skill cooldowns and spawn timings remain open; #297 also flags poison damage for investigation. No 60 Hz switch has landed.
    - *Status:* S10a merged (#157). S10b rows on main or in review: town walk and run (#158); stamina and mana regeneration, and a monster attack's hit event, first AI think and end for both synthetic monsters (second slice). All are inside the window, and none needed a sim fix. Third slice (S9-B): hit recovery for both monsters and the player, the player's cast, an arrow's lifetime and flight, poison, chill and stun durations, and life regeneration; the report's own synthetic set adds the get-hit and cast animations, the arrow's flight function, the life regeneration stat and the cold and stun states. Fourth slice: a successful Shoot use through the skill do pipeline starts the shared cooldown, observed until it stops blocking and can start again; a spawner's interval is observed between two child monsters actually appearing. Both use the same report table and window at 25 and 60 Hz, with no sim fix needed. The report adds only a fixture skill-use adapter and synthetic skill delay, state and spawner data. Still open: the life a poison takes (the player in the report's session loses none at either rate; to be looked at).
  - **S10c, host schedule (`d2-server/src/host.rs`):** the `TickDriver` and the flush throttle take the game's `TickRate`. Tick *n* is due at `⌊n · 1000 / hz⌋` ms after the start (17, 17, 16 ms at 60 Hz), so there's no drift, and catch-up stays at most one tick. Games are made with the host's rate. Single player and the headless server use the same driver.
  - **S10d, client at the server's rate:** the client learns the rate from `ServerLink` (no wire change) and steps by it: walk prediction, client paths, player and monster animation, client missiles and the day clock use `TickRate::share`/`ticks` like the sim; the 40 ms constants (`rules/camera.rs`, `world_view/smooth.rs`, sound, ping, preload and chicken periods) come from the rate. The front end's own 40 ms clock is a UI clock and stays a fixed real-time step.
    - *Status:* part 1 merged (#304), from `codex/cx-s10d1-client-rate`: required `ServerLink::tick_rate`, propagated through local, threaded and wrapper links into `ClientWorld`; walk prediction, player and critter paths, and monster motion use the game's rate; pings keep their 5,000 ms period (125 ticks at 25 Hz, 300 at 60 Hz). Part 2 merged (#317), from `codex/cx-s10d2-client-rate`: player and monster animation (including sequences), client missile lifetimes, animation and motion, and the day clock use the game rate; camera shake, smoothing spans, sound delays and lifetimes, preload and chicken periods preserve real time. Local and threaded links expose the authoritative server game frame, forwarded through the production wrappers, so movement and animation shares use the same phase as the simulation; links without a game frame fall back to the client tick counter. Synthetic 25/60 Hz tests cover per-second progress and timing. Part 2 passes formatting, default and dylib Clippy, and the client nextest suite (3,382 passed, 306 skipped); part 1 passes its required dylib Clippy and client suite (3,370 passed, 306 skipped). The front end's 40 ms UI clock stays fixed. The base-rate switch remains S10e.
  - **S10e, the switch:** the base game's rate becomes 60. New determinism and frame-hash baselines at 60 Hz. Recordings and frame schedules state their rate, and an old 25 Hz one is refused with a clear message (not replayed at the wrong speed). Saves store no tick counts that outlive a game (checked here). `docs/ARCHITECTURE.md`'s "Fixed 25 Hz tick" is rewritten.
    - *Status:* implemented on `codex/cx-s10e-60hz`, based on main after #317 merged. Base play and
      headless defaults select `TickRate::GAME` (60 Hz), while `TickRate::BASE` stays
      the 25 Hz table unit. Autoplay and soak clocks use rational game-rate deadlines.
      Tick-indexed replay inputs require rate metadata and refuse legacy or mismatched
      rates with a re-recording message. Save fields were audited: no game tick counts
      persist; creation/save timestamps are Unix seconds. Synthetic 60 Hz state and CPU
      frame baselines and save-clock independence are covered by `rate_baselines`.
      Scripted scenarios state `tick_rate` (legacy default 25 Hz); standalone and
      batch repro convert deadlines, hold durations and wait limits to game ticks with ceiling
      division. All bug scenarios retain explicit 25 Hz units, and the picture
      manifest is retimed to 60 Hz. Soak scenarios retain their exact log rate.
      The combined three-crate nextest run including `world::rate_baselines` passes
      (8,876 passed, 447 skipped, dylib), after merging Q0 (#347) and registering
      the baseline test in its world group. Formatting and strict all-target
      Clippy pass. Both real-data query/failure timestamp tests pass separately.
      All three CI bug scenarios and all three warm-batch replays pass.
      Soak fixtures state 60 Hz and test legacy refusal.
      Fifteen rate-tagged recordings stay local under `E:/work/artifacts/cx-s10e/`;
      none are committed.
      Six short autoplay probes, a fresh 360-tick soak and seven of eight retimed
      soak inputs pass. The remaining `roundtrip:live.waypoints` finding is
      **Unverified**: main at 25 Hz (`c0fe5464`) reproduces the identical
      None-to-Some waypoint records at step 100/frame 122 (60 Hz: step 240/frame 262).
      It is left unchanged. Two unavailable saved starts use fresh saves;
      historical long-route milestones and bug reproduction are not claimed.
      Headless `tools/perf` after merging main through `8ba67abc`
      (10 scenes × 1,800 ticks, seed 1234) passes:
      471.0–1,977.5 ticks/s, every tick+flush at most 4.026 ms (16 ms budget).
      Initial cold maxima of 21.664 ms (Spider Forest), 20.075 ms (Cold Plains)
      and 18.211 ms (Far Oasis) do not reproduce with the preserved unchanged
      release executable (repeat all-tick maxima 3.426, 2.347 and 4.699 ms).
      With host timing spans, the merged-head cold maxima are respectively 2.303,
      2.278 and 2.304 ms. The room pass costs 1.900 ms in Spider Forest and
      1.879 ms in Cold Plains; the Far Oasis client pass reaches 2.036 ms.
      Cold Plains frame 66 instead spends 0.167 ms in timer dispatch. The initial
      stalls' cause remains unconfirmed; no setup or rule work is moved between
      ticks. All ten runs' state snapshots remain identical (output-path headers
      are excluded) and the 60 Hz state/RGBA baseline is unchanged.
      A three-area repeat during batch CI also passes (280.7–383.8 ticks/s,
      7.618 ms all-tick maximum); its state traces still match.
      Reports, raw traces and binary/source provenance stay local under
      `E:/work/artifacts/cx-s10e/` (`perf60*`, `profile-*-spans`, `perf-final*`,
      `perf-merged/`, `profile-merged-repeat/`).
      Shared builds ran during these measurements. Owner play/sign-off is pending.

- **Tick cost:** 60 Hz runs 2.4 times as many ticks per second, so a tick at 60 Hz must cost at most 40 % of today's 25 Hz budget on the target PCs. PERF's BUG-010 and BUG-011 numbers (tick cost, present scheduling) come first: S10e lands only once a 60 Hz game holds 60 ticks per second in `tools/perf`.
- **Done when:**
  - the autoplay routes and the soak test pass at 60 Hz;
  - the equivalence report shows every listed timing within tolerance;
  - the CPU stays within budget (`tools/perf`);
  - the owner plays it and signs off.
- **Risks:**
  - rounding: rates that were whole numbers of 25 Hz ticks aren't whole numbers of 60 Hz ticks;
  - 60 Hz doesn't divide 1000 ms evenly (16.67 ms), so tick lengths alternate 17, 17 and 16 ms on S11's clock;
  - recordings require explicit rates and fresh 60 Hz captures; S10e audited
    character saves and found no persistent game tick counts;
  - `docs/ARCHITECTURE.md` ("Fixed 25 Hz tick") is rewritten as part of this module.

### S7. More animation frames (in progress; owner 2026-10-18)

- **What:** walk, run, attack, cast, get-hit, block and death cycles get more frames, so at 60 Hz logic and high-refresh displays they animate smoothly instead of repeating 25 fps pictures.
- **How:**
  - **base game:** the frames are derived from the player's own game files, so the generator runs on the player's machine, once, when the launcher installs or updates the game (`bake-anims`, [the bake](../../mods/animation-generation.md)). Results are kept in files beside the install and never shipped or uploaded. The game only loads them; it never generates frames while it runs (owner, 2026-10-19);
  - **mods:** mods are baked the same way when installed (reusing the base game's packs), or ship ready-made frame packs built by the mod's own pipeline. Nothing is generated for them at run time.
- **The palette problem:** the art is 8-bit palette indices, which can't be averaged as numbers. The generator interpolates in colour (motion-aware, per component and direction), then maps back to the palette by nearest match, keeping colour-mapped areas on their ramps. A quality gate rejects bad frames, which keep the source frame.
- **Sub-modules:**
  1. **S7a, frame source seam (landed: #354, batch 105):** a `FrameSource` that picks the picture for (unit, component, direction, frame, sub-frame), with the default returning today's frame. Frame hashes are unchanged.
  2. **S7b, frame pack format v1 (landed: #355, batch 105):** versioned, keyed by animation token, mode, component, weapon class, direction and sub-frame, with load-time validation. It's the same format for the local cache and for mod packs.
  3. **S7c, playback (landed: #356, batch 105):** the compositor draws pack frames on the 60 Hz ticks between the source frames, with the COF layer order of the starting frame.
  4. **S7d, generator (landed: #360, batch 106):** runs on the player's machine for the base game (at install since 2026-10-19: `bake-anims`, with launcher progress and a quality gate; it ran on first use in play before), and as a tool in a mod's build for mod packs. A review sheet lists the rejects.
  5. **S7e, proof (landed: #384, batch 114; ramp-family fix in #389):** one class's walk and run cycle, generated locally, for the owner's look check before scaling up.
  6. **S7f, coverage (implemented locally; in review):** all classes, modes and weapon classes.
  7. **S7g, coverage (implemented locally; in review):** monsters and unit overlays; the separate missile/effect layer stays original for now, including fast translucent missiles. Fast translucent missiles may stay original on purpose.
  8. **S7h, controls:** a Video row "Animation frames: Original / Smooth", a per-animation deny list, and a dev command to compare live.
  5. **S7e, proof (implemented locally; owner look check pending):** one class's walk and run cycle, generated locally, for the owner's look check before scaling up.
  6. **S7f, coverage:** all classes, modes and weapon classes.
  7. **S7g, coverage:** monsters, missiles and overlays. Fast translucent missiles may stay original on purpose.
  8. **S7h, controls (implemented on `codex/cx-s7h-controls`; automated gates pass, owner look check pending):** a Video row "Animation frames: Original / Smooth", a per-animation deny list, and a dev command to compare live.
- **Look:** owner 2026-10-18 lifted the hold. Build S7a–S7e, then stop for the owner’s local look check before S7f–S7h and S8.
- **Done when:**
  - the proof (S7e) is done;
  - the coverage report lists every animation as done, rejected (with reason) or original;
  - frame events still land on their tick;
  - Original mode reproduces today's frames exactly.

### S8. More facing directions (implemented locally; in review)

- **What:** units turn through more directions than the art has.
- **How:** in-between directions come from the same generator (S7d): locally for the base game, in the packs for mods. The game logic's direction count is unchanged unless S10's study changes it.
- **Done when:** turning in place shows no jumps between directions, and the game state is unchanged.

### S6. Smooth unit and missile movement between ticks

- **Status:** merged through Display v2 V6 (#86).

- **What:** on monitors faster than the 60 Hz logic, players, monsters and missiles glide between tick positions.
- **Seam:**
  - `ClientUnit` keeps the previous and current fixed-point positions;
  - non-tick display frames are recomposed at blended positions on a **non-committing** compose path;
  - teleports and spawns snap.
- **Done when:**
  - movement at 144 Hz or more is smooth;
  - tick frames are unchanged (hash test);
  - teleports don't smear;
  - the GPU cost at 360 Hz is measured (`tools/perf`).

### S11. 1000 Hz base clock

- **Status:** in review (#273), stacked on #268.

- **What:** a 1 ms base clock under the game. Ticks are scheduled on it, and backend tasks use it directly:
  - input timestamps;
  - sub-tick timers;
  - network timing (multiplayer, later);
  - profiling;
  - later uses such as a 1000 Hz logic rate for mods.
- **Seam:** one clock in the host (the injected `Clock` in `d2-server/src/host.rs` already exists), with tick scheduling as "next tick at ms N" and the rate from S9 and S10.
- **Done when:** a 60 Hz game scheduled on the 1 ms clock keeps real-time speed over a 1-hour soak (no drift), and the tick spacing (17, 17, 16 ms) is tested.
- *Status:* done. `BaseClock` in `d2-server/src/host.rs` extends the injected 32-bit `Clock` reading to a 64-bit millisecond count that never wraps (a step back holds it). The `TickDriver` schedules on it (`TickDriver::next_tick_ms` is "next tick at ms N"), and backend tasks read it from `Host::base_ms`. The schedule's old 31-bit limit is gone: it used to stall after 2^31 ms (24.8 days) of uptime. Below that, 25 Hz ticks exactly as before. The test `sixty_hz_hour_on_the_base_clock` runs a 60 Hz host for one hour of uneven 1–16 ms frames across the 32-bit clock wrap. All 216,000 ticks land on `⌊n · 1000 / 60⌋` (16, 17, 17 ms). The listed uses (input timestamps, sub-tick timers, network timing, profiling, a mod's logic rate) read `Host::base_ms` as each is built.

## Owner decisions

- **2026-10-18:** S7 and S8 are in progress; the 2026-10-17 hold is lifted. Stop after the local S7e proof for the owner’s look check before S7f–S7h and S8.
- **2026-10-19:** animation frames must not be generated on the go. They are baked when the game or a mod is installed and kept in files; the top-left generation indicator goes. S8 in-between facings are not baked yet, so they draw the nearest file facing's frames until they are.
1. **S10's per-system list (2026-10-16):** no exceptions; every system runs at 60 Hz.
2. **S7 look (2026-10-16):** approved by the owner; the 2026-10-18 decision adds a local look check at S7e.

## Not planned

- **VSync policy:** the earlier locked-on rule was replaced by Display v2 V2 (#85).
- **Above 360 fps:** owner rule.
- **Shipping frames made from the player's game files:** base-game frames are baked on the player's machine only.
- **Generating frames at run time:** the base game and mods are baked at install (owner, 2026-10-19).
