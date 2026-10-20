---
id: test-suite
title: Automated test suite
area: tools
status: in-progress
order: 2
modules: Q0-Q12
light: Q0-Q2
depends_on: [test-automation-tools]
---

# Plan: automated test suite

Status: **in progress** (owner, 2026-10-18), all modules Q0-Q12 approved; the AI guide is [`docs/tools/test-suite.md`](../../tools/test-suite.md). It builds on
[`test-automation-tools.md`](test-automation-tools.md) (T0-T10) and follows
[`docs/FEATURE-METHOD.md`](../../FEATURE-METHOD.md).

## Goal

D2RUST finds its own obvious bugs. The suite does three things without a human playing:
- **plays the game's content** (every skill, NPC, area, item action and panel);
- **checks generic rules on every run** (no crash, no desync, nothing duplicated or lost, nothing stuck);
- **compares builds** (behaviour, pictures, timings).

It runs fast: seconds per check on a ready-built binary, minutes for the full suite.

Owner rules:
- **Determinism stays intact.** Tools read state or feed input through the player's input paths; no tick result changes.
- **Dev tools stay on by default.**
- **The owner uses BIGRIG.** Runs are headless or offscreen, never touch the real mouse or focus, and never test over the network (in-process and loopback only).
- **Game files and game art** stay on BIGRIG and the CI server, never in the repository and never uploaded elsewhere. Owner 2026-10-18: reference shots may be kept on local disk (BIGRIG `E:/work/artifacts/golden/`, CI server `/cache/golden/`).
- **Bug reports from tools are "Unverified"** until the owner asks for a fix (`scratch/bugs-unverified`). Tools file candidates; they never auto-fix.
- **BIGRIG resource guard:** 85% CPU, 60/64 GB RAM, 80% disk. Heavy runs go through the build queue, or to the CI server.

## Where things are today (2026-10-18)

- Merged tools are listed in `test-automation-tools.md`: `repro` and scenarios (T0-T2), the perf verdict (T3), bug scenarios (T4), offscreen shots (T5), the fixed display clock (T6), `img-diff` (T7) and CI scenarios (T10). Open: the live control channel T8 (#241) and the element ids T9 (#237, #261 and the screens stacked on it).
- **Few cases:** 5 scenarios (`tools/scenarios/bugs/BUG-008/009/013.scn`, `crates/d2-client/tests/smoke/*.scn`).
- **Slow gate:** d2-client has 123 integration test files, each its own binary that links the whole client. A full d2-client test build took 15m24s and the run 13m47s (3,372 tests in 126 binaries).
- **Startup cost:** a `repro` run on real files takes 6-10 s in release, of which 3.5-4.4 s is startup (data load, server start, join).
- **Soak** (`app/soak/`, `tools/soak/soak.py`) has invariant rules 1-5 (`soak/checks.rs::server_check`: GUIDs, lists, desync and more) and a delta-debug `reduce`. Only soak runs use them.
- **Fuzz targets** (`fuzz/`) cover file formats only.
- **CI server:** a Mevspace Ryzen 9 7950X (Ubuntu 24.04, 96 GB) joins BIGRIG on 2026-10-19 as CI runners plus a build and test farm. Game files are allowed there. Tools reach it over SSH (`D2_FARM_HOST`), and every tool falls back to local when it isn't set.

## Modules

| Rank | Id | Module | Tier | Size | Depends on |
|---|---|---|---|---|---|
| 0 | Q0 | Fast gate: fewer test binaries, shared fixture cache, changed-crate selection | light | 4-6 h | none |
| 1 | Q1 | Warm batch runner and binary cache | light | 4-6 h | none |
| 2 | Q2 | Always-on checks in every `repro` run | light | 3-4 h | none |
| 3 | Q3 | Content sweep (Q3a framework + skills, Q3b NPCs and menus, Q3c areas and waypoints, Q3d items and trade, Q3e panels) | full | 1 day + 2-4 h per family | Q1, Q2 |
| 4 | Q4 | Behaviour diff between builds, and bisect | full | 4-6 h | Q1 |
| 5 | Q5 | Farm: CI-server soak, Unverified candidates, remote builds and tests | full | 1 day | Q1 |
| 6 | Q6 | Picture checks: rules with no reference, plus local reference shots | full | 1 day | Q1 |
| 7 | Q7 | UI explorer over T9 element ids | full | 4-6 h | Q2, T9 (#261) |
| 8 | Q8 | Coverage map | full | 4-6 h | Q1, Q3a |
| 9 | Q9 | Static and robustness: table checks, fuzzing of messages/saves/mod packs/scenarios, rule property tests | full | 1 day | none |
| 10 | Q10 | Farm deep checks: long runs and leaks, perf trend, system-order ambiguity, software-GPU shots, mutation testing, Miri | full | 1 day | Q5 |
| 11 | Q11 | Multiplayer in one process: server + N headless clients, simulated lag/loss, join/leave, desync | full | 1 day | mp/* on main |
| 12 | Q12 | Moved to [game-explorer.md](game-explorer.md) (G0-G5) | - | - | - |

**Owner rule (2026-10-18): what counts as a bug.** A bug is a broken stated rule: a crash, a broken invariant, a
written expectation (`check`, property, table rule) that fails, or a change nobody intended (Q4). How the game
looks, feels or is balanced is not a bug until the owner states the rule; once stated, it becomes a check and
the suite enforces it from then on.

**PRs (owner 2026-10-18: as few as possible).** Q0, Q1, Q2, Q5 and Q6 are one PR each (already running in
parallel). Then:
- **Q3** in one PR: the sweep framework and all five families, the save round trip at sweep checkpoints, and `--mod NAME` (each mod runs the sweep on its own data).
- **Q4** in one PR: behaviour diff, bisect, replay determinism, tick-rate equivalence, perf trend and perf bisect.
- **Q7 + Q8** in one PR.
- **Q9** in one PR.
- **Q10** in one PR (follows Q5).
- **Q11** later, one PR. Q12 moved to [game-explorer.md](game-explorer.md).

### Q0. Fast gate

- **Merge validation (Q0, 2026-10-18):** merged main `c155fa6e` with a merge commit, preserving every module status line and main’s repro fixture additions. Registered the new `app_control.rs`; all 124 root integration files are included, and the nested `app_repro::farm_soak_scenario` test remains. Client nextest: **3,456 passed / 308 skipped** versus **3,381 / 306** before the merge (+75 passed, +2 ignored); fixtures: **110 passed / 2 skipped**, unchanged. The combined run passed all 3,566 runnable tests in 419.674 s (310 skipped); resource-guard pauses were observed. Every prior listed test and ignored state is preserved after accounting for main’s two checks-test namespace moves. Queued format check, strict all-target clippy, prebuild, inventory and CI-waited nextest all exited 0.
- **Status (Q0, 2026-10-18): implemented and locally validated; awaiting review.** The 124 top-level integration files stay in place under five grouped targets plus the existing mutation target. Shared helpers are imported from each target root; nextest still starts a fresh process per test.
- **Inventory:** 3,684 existing tests and all 306 ignored states match after normalizing the added source-file module prefix (saved inventory diff is empty); three registration tests are added. Total binaries drop from 126 to 8.
- **Matched warm rebuild:** before **16m11s**, after **2m08s** (Cargo-reported time). Dependencies and object caches were warm; only the listed test executables were removed (126 before, 8 after). Both measurements used the unchanged build-slot wrapper with 5 build jobs and 6 test threads. Wrapper wall times, including queue waits, were 1,147.891 s and 391.265 s.
- **Full nextest run:** before **403.431 s**, 3,378 passed / 306 skipped; after **279.208s**, 3,381 passed / 306 skipped. Queue wait is excluded from nextest summaries. Earlier grouped runs overlapped merge-train batches #342/#343/#344 and were excluded or interrupted at the owner's request; the accepted rerun followed `ci-wait.sh`. Shared-machine load and resource-guard pauses affect these wall measurements, so they are not a controlled speedup estimate.
- **Additional build context:** the initial cold baseline took 56m57s at an older 3-job setting and is not comparable to the matched warm rebuilds. The first grouped build took 14m05s, including rebuilding fixture/client inputs.
- **Validation:** formatting and all-target clippy (`-D warnings`) pass for client (dylib) and fixtures; client and fixture nextest gates pass. The real unregistered-file probe fails with its filename and suggested target; removing it restores a passing guard. The affected-crate tool's six unit tests and the existing scope selftest pass. Cache checks cover input invalidation, incomplete recovery, private copies and concurrent processes. Both latest owner suite documents are included, with index links.

- **What:** the d2-client test gate drops from about 29 minutes (build plus run) to a few minutes, without losing a test.
- **How:**
  - **Fewer binaries:** set `autotests = false` in d2-client and group the integration tests into a few `[[test]]` targets (for example `ui`, `app`, `world`, `front_end`, `real_data`). Each target is one `main.rs` that pulls the existing files in as modules with `#[path]`, so the files don't move. That avoids conflicts with open PRs.
  - **Registration check:** a test lists `tests/*.rs` and fails when a file isn't registered in a target, so a new test file can't silently stop running.
  - **One shared synthetic install:** `test-fixtures` builds it once into a cache under the target dir, keyed by a hash of its inputs. A file lock guards it, so parallel test processes don't build it twice.
  - **Changed-crate selection:** `tools/ci/affected.py BASE` maps changed files to crates and prints the nextest `-p` flags. The train's batch still runs everything.
  - Measure before and after, and write both into this plan.
- **Files:** `crates/d2-client/Cargo.toml`, new `crates/d2-client/tests/<target>/main.rs`, `crates/test-fixtures/src/` (cache), new `tools/ci/affected.py`, `.config/nextest.toml` if needed.
- **Done when:**
  - nextest runs the same test names (diff the `--list` output before and after, nothing lost);
  - the build and run times are recorded here;
  - the registration check fails on an unregistered file.
- **Risks:**
  - Test files that used the same helper modules (`mod common;`) or statics now share a binary. Fix those collisions with paths, not renames.
  - Tests that relied on their own process (globals, env vars) run under nextest's process-per-test, which is unchanged.

### Q1. Warm batch runner and binary cache

Status: **complete** (cx-q1-batch, 2026-10-18). Real-file release validation
and synthetic first/last parity passed, including UI, input and character state
changes between cases. The full d2-client nextest gate passed: **3409 passed,
308 skipped**, in **126.840 s** (execution only). On BIGRIG, with other lanes
building, 50 cases (10 copies each of both smoke scenarios and BUG-008/009/013)
took **366.627 s** as separate release processes versus **53.932 s** with
`repro --batch LIST --jobs 4`: **6.80x faster**, 14.7% of the separate-run time.
Both measurements used the same ready-built release executable, built from
`dcdc1a0a` plus the Q1 implementation, and exclude compilation. The measured
executable and SHA-256 remain with the local artifacts; main was merged afterward
to preserve the later suite-plan additions.
All 50 passed in each mode; all 50 `repro.json` files matched byte for byte after
normalizing only `wall_ms`. The batch's internal wall time was 53.725 s.
Raw reports, list and timing summary stay under
`E:/work/artifacts/cx-q1-batch/measure/`. The binary cache's 10 fake-runner unit
tests pass, including read-only cache hits and the Q5 Linux artifact contract.

- **What:** hundreds of scenarios run in minutes from one ready-built binary, and nobody compiles just to run a test.
- **How:**
  - **`repro --batch DIR|LIST --jobs N --out DIR`:** loads the game data once per worker and resets the game between scenarios (a new server game and a client rejoin, with no data reload). It writes one `repro.json` per scenario plus `batch.json` (a table, totals, the slowest runs). It is byte-identical to separate runs apart from wall time, and one parity test checks that.
  - **`tools/ci/bincache.py`:**
    - `get SHA [--profile release|dist]` returns a path to a `D2RUST-Game` binary for that main commit;
    - `build SHA` builds it (from a worktree checkout, through the build queue on BIGRIG, or on the farm when `D2_FARM_HOST` is set);
    - `prune --keep N`.
    - The cache lives in `E:/work/bin/<sha>/` on BIGRIG and `/cache/bin/<sha>/` on the farm. The merge train calls `build` for each new main.
- **Files:** `crates/d2-client/src/app/repro/` (batch mode, game reset seam), `main.rs`, new `tools/ci/bincache.py`, `docs/tools/scenarios.md`.
- **Seam:** `repro::Session` (a loaded game that runs one scenario after another).
- **Done when:**
  - [x] 50 copies of the smoke scenarios run with `--jobs 4` in a fraction of 50 separate runs' time (record the numbers);
  - [x] batch and single runs give identical `repro.json` apart from wall time;
  - [x] `bincache.py get` returns a cached binary without a build.
- **Risks:** state leaking between scenarios (globals, resources not reset). The parity test runs a scenario first and last in a batch and compares.

### Q2. Always-on checks

- **Status:** implemented and validated in [PR #337](https://git.werdegames.com/WerdeGames/D2RUST/pulls/337), branch `codex/cx-q2-checks-always-on`: 14 opt-out rules, 25 new tests. Integrated main `8ba67abc`, preserving tick-rate forwarding and Q1 standalone/warm Session/batch parity. Final fmt and all-target clippy with warnings denied passed (1m25s); nextest: 3443 passed, 308 skipped, zero failures in 196.667s across 127 binaries. Soak, smoke, determinism and byte-for-byte opt-out parity tests passed. Real-data BUG-008/009/013 each exited 0 with zero violations: 40/70/20 ticks in 31188/36933/19339ms; gate total 89.260s. Initial full test build: 31m41s; final build: 17m29s. Logs and reports: `E:/work/artifacts/cx-q2-checks/`. Conservation permits recognized creation/destruction directions without checking exact prices or recipe quantities. Final commit `7c8297f0`; PR is mergeable and linked; worktree is clean; cargo clean exited 0 (68413 files / 51.5GiB removed).

- **What:** every `repro` run checks generic rules, not only the scenario's own `check` lines.
- **Rules:**
  - the soak rules from `soak/checks.rs` (GUIDs, lists, desync);
  - HUD numbers equal the server's stats;
  - gold and items are conserved across pickup, drop, buy, sell, trade, stash and cube (nothing appears or vanishes outside an event that creates or destroys it);
  - no unit stays in one mode longer than its limit unless the mode is meant to last (death, idle);
  - animation frames stay inside the frame count;
  - positions are walkable;
  - no panic, no `error!` log line and no missing-asset fallback.
- **Opt-outs:** a scenario can turn a rule off with `allow RULE` (needed for bug scenarios that show the broken state).
- **Output:** violations in `repro.json` with a stable signature (rule plus units, no tick), so runs can be grouped. Exit code 1 on any violation.
- **Files:** move the soak rules into a shared `app/checks/` used by soak and repro, new rule files there, `app/repro/` (wiring, `allow`), a log-capture layer for `error!` lines.
- **Done when:** one test per rule shows it trips on a broken synthetic state and stays quiet on the smoke scenarios; soak still passes its own tests; the three bug scenarios pass on main.
- **Risks:** false alarms on legit edge cases. Each rule gets a clear message and an `allow`, and rules start strict.

### Q3. Content sweep

- **Status (cx-q3-sweep, 2026-10-19): implemented and locally validated; awaiting review.** All five loaded-table generators, warm parallel execution, signature grouping, standalone failure exports, replayable save checkpoints and explicit per-mod data selection are implemented. Framework/skills synthetic tests passed before the remaining families were added. Complete real-file runs exist for all five families; every remaining failure is an **Unverified** candidate, not a confirmed game defect. The owner stopped game scenario runs on 2026-10-19, so the items rerun on the final generator code (level, stat and stash-object setup fixes) is marked **owner later**.

- **Checkpoint integration (2026-10-19):** G2's goal-boundary `repro/checkpoint.rs` module is kept; Q3's `at N checkpoint` save round trip is an extension of that module, and the scenario step and its timing test are unchanged. No duplicate checkpoint module remains.

- **Skill findings (merged 2026-10-19):** the sequence-animation bound (findings 2 and 3), the temporary-state checkpoint comparison (4) and the ground/self shape of unit-target skills (5) were false positives of the checks/generator; the merged sweep-tool branch skips them and adds tests. Finding 1 (melee approach) is fixed by PR #406 and finding 6 (stuck sequence mode 18) by PR #407. No post-fix skills rerun was made after the owner stopped game runs; the table below is the pre-fix run.

- **Real-file runs (BIGRIG, 60 Hz, headless, shared load; timings are observations, not controlled comparisons):**

  | Family | Cases | Passed | Failed | CLI exit | Jobs | Wall s | Signatures | Tested source |
  | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
  | skills | 382 | 333 | 49 | 1 | 4 | 1321.076 | 9 | lane `7d3bab18`, merged main `c3b26196` |
  | NPCs | 120 | 75 | 45 | 1 | 4 | 293.755 | 308 | Q3 merged main `e88e6dd1` |
  | areas | 408 | 234 | 174 | 1 | 4 | 2001.251 | 287 | Q3 merged main `e88e6dd1` |
  | items | 2751 | 2690 | 61 | 1 | 2 | 7929.221 | 57 | merged head `4b19c5ae` (main `4e737eb9`) |
  | panels | 72 | 58 | 14 | 1 | 4 | 142.769 | 1 | Q3 merged main `e88e6dd1` |

  The items run used the generator after the equip-stat, stash-class and greeting fixes, but before the final character-level fix; the representative equip case was rechecked at level 99 and passes, and the other 37 are expected to share that cause. The full owner-authorized rerun is pending. Failures are grouped by signature and left for owner triage; no issues were filed.

- **Synthetic CI slices (debug, BIGRIG, 2026-10-18):** skills **11.456 s**, NPCs **9.570 s**, areas **7.278 s**, items **3.845 s**, panels **3.183 s** (one selected case per family; wall time includes generation). NPC/area/item/panel slices request `--jobs 4`; the skill slice uses one worker. Explicit mod execution, both conditional-greeting branches and byte-identical checkpoint/no-checkpoint comparisons pass. On main merged at `c3b26196`, the 12-package affected gate passes **9,797 tests**, **520 skipped**, **261.875 s** execution / **661.297 s** including build and wait. Strict all-target client/simulation clippy exits **0** in **91.992 s**; the lightweight pre-PR gate, including formatting, exits **0** in **25.217 s**.

- **What:** `sweep FAMILY [--filter ..] [--jobs N]` builds scenarios from the game tables and runs them through Q1 with Q2's rules, so every skill, NPC, area, item action and panel gets exercised automatically. New content is covered without writing a scenario.
- **Families (one combined PR, implemented in this order):**
  - **Q3a, the framework plus skills:** every class and skill at a fitting level, cast at a monster, on the ground and on self. It checks the cast completes and the player returns to neutral, the costs are paid, and no rule from Q2 trips.
  - **Q3b, NPCs and menus:** every town NPC: talk, trade, repair, hire, identify, gamble; open and close every menu.
  - **Q3c, areas and waypoints:** load every area, walk a short route, take every waypoint and level link, die and respawn in town.
  - **Q3d, items and trade:** buy, sell, pick up, drop, equip, socket, stash, cube recipes and belt use, with Q2's conservation rule.
  - **Q3e, panels:** open and close every panel by action name, and every panel combination that can be open together.
- **Output:** `sweep.json` and a table (family, case, pass/fail, first violation), with failures grouped by signature. A failure can be saved as a standalone `.scn` (`--save-failures DIR`), filed as **Unverified**.
- **Files:** new `crates/d2-client/src/app/sweep/` (generators per family, reading the loaded data tables), `main.rs`, `docs/tools/sweep.md`.
- **Done when:** each family runs on real files on BIGRIG and on the synthetic install in a CI test (a small slice); the run time per family is recorded here; known-good main shows zero failures, or the failures are filed as Unverified candidates.
- **Risks:** generated cases that are invalid for the game, not bugs (a skill with no target type). The generator skips those by rule, with a count of skips per reason.

### Q4. Behaviour diff and bisect

- **Status (cx-q4-bdiff, 2026-10-18): implemented and locally validated; awaiting review**
  in [PR #353](https://git.werdegames.com/WerdeGames/D2RUST/pulls/353).
  QA follow-up renamed the CLI to `bisect_runs.py` to preserve standard-library
  imports from `tools/ci`; Python tests passed again: **39 passed / 16 subtests**,
  **36.03 s** (38.515 s wall), exit **0**.
  Python-only read-only orchestration exports full `state-1` through existing
  server queries, compares builds/replays and generated sweep cases, aligns
  different tick rates at exact common game times, and records repeated perf
  medians and first-parent behaviour/perf bisects. Offline tests plant state and
  replay changes, use distinct cached SHAs with identical behaviour, and find a
  planted bad commit in an actual synthetic Git history. Final CI Python gate:
  **39 passed / 16 subtests**, **6.14 s** (6.672 s wall). Formatting passed in
  **18.156 s**; queued strict all-target client/mod-game-example clippy passed
  in **15m19s**. Queued nextest prebuild passed in **34m31s**, including its
  wait for clippy's target lock. CI-waited queued nextest passed: **3,474 passed,
  309 skipped**, **96.758 s** execution / **181.719 s** including queue wait.
  Determinism, smoke and invariant opt-out parity checks stay green.
- **Real-file acceptance:** the freshly built debug executable ran both smoke
  scenarios on both comparison sides and replayed each side once: all eight
  runs passed and full tick-zero-through-end state matched (**83.796 s**).
  The perf workflow's three samples per side took **47.813 s**; same-binary
  server p95 medians were **11,594 / 11,753 us** (+1.37%; workflow threshold
  100%, not a controlled performance claim). An archived release's two 120-tick
  60 Hz recordings also matched exactly (repeat **4.454 s**).
- **Unverified candidate, not fixed:** `state-diff:unit(1,6):xf` at game time
  **1 s**: tick 60 at 60 Hz reports **53258**, tick 25 at 25 Hz reports **42605**.
  Fresh seed-1234 two-second captures took **5.703 / 3.656 s** and reproduced
  the archived release's candidate. Repro: `py -3 tools/ci/bdiff.py E:/work/artifacts/cx-q4-bdiff/fresh-rates/60/state.jsonl E:/work/artifacts/cx-q4-bdiff/fresh-rates/25/state.jsonl --dumps --equal-time --out E:/work/artifacts/cx-q4-bdiff/fresh-rates/repro`.
  Logs, reports, executable hashes and the retained fresh binary stay local
  under `E:/work/artifacts/cx-q4-bdiff/`. No game logic or tick result changed.

- **What:** prove that a change didn't change behaviour (refactors, comment rewrites, conflict fixes), or find the commit that did.
- **How:**
  - `tools/ci/bdiff.py A B [SCENARIOS|--sweep FAMILY]` runs the same scenarios on two binaries (paths or shas through Q1's cache), compares `state-1` tick by tick, and reports the first tick, unit and field that differ, plus a short context window. Exit 0 same, 1 differs. It can compare two tick rates at equal game time (for S10e).
  - `tools/ci/bisect_runs.py SCENARIO --good SHA --bad SHA` walks main's first-parent history with cached binaries (building the missing ones through Q1) and names the first bad merge.
- **Files:** new `tools/ci/bdiff.py`, `tools/ci/bisect_runs.py`, docs in `docs/tools/scenarios.md`.
- **Done when:**
  - a test with two synthetic dumps finds the planted difference;
  - two different shas with no behaviour change report "same";
  - bisect finds a planted bad commit in a synthetic history.
- **Risks:** dumps that legitimately change (a new state field) need a field ignore list (`--ignore FIELD`).

### Q5. Farm: soak, candidates, remote builds and tests

- **Status (2026-10-18): implemented; local gates passed.** `tools/farm/`
  supplies forge-fetched SSH gates, binary downloads, soak/reduce/export,
  signature-grouped Unverified candidates, and explicit local fallback.
  Python: **16 tests passed in 3.321 s**. Formatting and the lightweight pre-PR
  gate passed. Clippy (all targets, dylib, warnings denied) passed in **28m54s**
  including Cargo-lock wait. Nextest: **3,380 passed, 306 skipped, 1 slow** in
  **140.091 s**; its test build took 13m33s including Cargo-lock wait.
  The synthetic export/repro test passed in **1.923 s**, comparing the first
  setup failure's signature and detail and then checking a healthy replay.
  The `soak-scenario 1` envelope preserves the soak engine, clock, GUID actions
  and round trips. Live server acceptance is deferred to 2026-10-19; no server
  connection was attempted. See [`tools/farm/README.md`](../../../tools/farm/README.md).

- **What:** the CI server works while lanes sleep. It soaks the game overnight, files reduced failures as Unverified candidates, and runs lanes' heavy builds and tests remotely so BIGRIG stays free.
- **How:**
  - **`tools/farm/farm.py`, with config from env** (`D2_FARM_HOST`, `D2_FARM_KEY`, `D2_FARM_ROOT`):
    - `farm build|clippy|test SHA [-p ..]`: the farm fetches the commit from the forge (lanes push a WIP branch first; nothing is uploaded from BIGRIG), runs the command in its own target dir, and streams the log back;
    - `farm bin SHA` downloads a built binary into Q1's cache;
    - `farm soak --hours N --seeds ..` runs soak, reduces each failure, saves it as a `.scn`, and groups by signature;
    - `farm status`.
  - **Build-queue hook:** `scratch/build-slot.sh` gets a farm mode. When `D2_FARM_HOST` is set and the command is a gate (clippy/nextest), it runs on the farm. LC makes that change outside the repo; this PR documents the interface.
  - **Candidates** go to `/cache/candidates/` on the farm and are mirrored to `E:/work/artifacts/candidates/` on BIGRIG, never into the repo. A summary line per signature goes to `scratch/bugs-unverified`, marked **Unverified**.
- **Files:** new `tools/farm/` (farm.py, `setup.md` for the server: Ubuntu 24.04, rustup, nextest, the game files via `tools/ci/game_files.py`, disk layout), `tools/soak/soak.py` (export a reduced log as `.scn`).
- **Done when:**
  - with `D2_FARM_HOST` unset, every command runs locally and says so;
  - unit tests cover the command building and log parsing with a fake SSH runner;
  - a soak failure exported as `.scn` fails the same way under `repro`.
  - The live check comes once the server is up (2026-10-19).
- **Risks:** the server isn't online until 2026-10-19. Everything is testable with the fake runner until then.

### Q6. Picture checks

- **Status (2026-10-18): complete** on `codex/cx-q6-picture-checks` (PR #333).
  Implemented `img-check` and the local `shots.py` capture/compare workflow.
  Python orchestration tests: 7 pass. Rust tests: 18 pass, 0 skipped (0.612 s);
  fmt and all-target Clippy with warnings denied pass. Fresh debug-build offscreen
  captures: 8/8 PNGs byte-identical, 152.109 s and 177.344 s; exact reference
  comparison and reference-free checks pass. A temporary 16-pixel player-sprite
  offset build (46.99 s incremental build, 180.281 s capture) fails all 8 reference
  pairs as expected: 3,701-3,704 pixels changed per frame (0.4016-0.4019%), with
  diff PNGs. The source was byte-restored and the offset is not committed.
  Local reports: `E:/work/artifacts/cx-q6-shots/report-clean/index.html` and
  `E:/work/artifacts/cx-q6-shots/report-offset/index.html`; the latter loads all
  24 side-by-side/diff images in the hidden preview via a loopback disk server.
  References and reports stay on BIGRIG; no images are committed or uploaded.

- **What:** visual bugs (black frames, missing textures, flicker, smears between ticks, clipped UI) are found automatically.
- **How:**
  - **Rules with no reference** (a new `img-check` binary in `tools/img-diff`):
    - a frame is all black or one colour;
    - missing-texture colour is present;
    - a frame changes while nothing moved (two shots at the same tick and phase);
    - a between-tick frame (T6 phases) jumps more than its neighbours;
    - a UI element's rectangle (from T9 ids, when present) lies outside the screen or overlaps another.
  - **Local reference shots:** `tools/ci/shots.py capture SCENARIOS --sha SHA` takes offscreen shots (T5/T6 fixed clock) into `E:/work/artifacts/golden/<sha>/`. `shots.py compare BASE_SHA HEAD_SHA` diffs them with `img-diff` (masks, tolerance) and writes an HTML report with side-by-side images and diffs, on local disk only. The last green main is the default base.
  - **Sweep hook:** Q3 families can take shots at chosen ticks, so the same cases feed both checks.
- **Files:** `tools/img-diff` (new `img-check` binary and rules), new `tools/ci/shots.py`, docs in `docs/tools/`.
- **Done when:**
  - synthetic images trip each rule;
  - two runs of the same sha compare clean;
  - a planted visual change (a test build with one sprite offset) is found with a diff image;
  - the report opens from disk.
- **Risks:** GPU noise between runs. T6's fixed clock makes shots repeatable; tolerance and masks cover the rest. References never enter the repo.

### Q7. UI explorer

- **Status (Q7, 2026-10-18): implemented; bounded local walks validated, full exploration remains limited.** T9 breadth-first routes share Q2 checks, measure text/layout, probe slider endpoints and replay through hidden T5 control. The planted unreachable close test passes. The 800x600 front walk visited 27 states / 311 edges in 44.379 s (31 Unverified candidates); the bounded 800x600 game walk visited 3 states / 69 edges in 493.412 s (6 Unverified candidates). The 1068x600 walk visited 3 states / 69 edges in 487.666 s, with 6 Unverified candidates and 3 hidden 1920x1080 PNGs. All report `complete: false`. Hidden T5 UI queries/clicks and PNG writing work; a visually black capture after both 8 and 128 held passes remains an Unverified candidate, with no renderer changes.

- **What:** `explore [--front|--game] [--res ..]` clicks through every UI element reachable from the main menu and the in-game panels, using T9 ids. It is a breadth-first walk over UI states. On every state it checks:
  - no panic and no Q2 violation;
  - Esc (or close) leaves every screen;
  - no element lies off-screen or overlaps another;
  - no text runs outside its box;
  - every listed element responds to its click.
- **Runs** at 800x600 and 1068x600, and at 16:9 through T5 when offscreen.
- **Files:** new `crates/d2-client/src/app/explore.rs` on autoplay-host, `main.rs`, docs.
- **Done when:** the front end and every in-game panel with ids are walked; a planted unreachable close button is reported; the run time is recorded here.
- **Risks:** state explosion (shop grids, option values). Limit per-screen repeats and skip value sweeps over number sliders (test the ends only).

### Q8. Coverage map

- **Status (Q8, 2026-10-19): implemented; counters are opt-in; determinism validated; the 1%/3% default-path target is not demonstrated.** Normal games, clients and autoplay hosts start with coverage disabled; only repro/explore construction opts in, and `D2_COVERAGE=0` still disables it for parity/timing probes. Disabled hooks check one inline guard before evaluating an ID; enabled hooks use static categories and numeric keys. Session-owned integer counters, real event hooks and the shared report envelope are implemented. State-1 bytes match for every tick of the 30-tick enabled/disabled test; wired sim/server tests count successful starts and reject refused starts. Python merge tests: 8 passed. Q3 is absent on this main revision; `app/coverage.rs` records its sweep-writer integration TODO.

- **T3 validation (Q8, 2026-10-19):** seed 7 at 60 Hz, 400 ticks, 410 samples per run, affinity mask 1. Earlier always-on runs measured mean server p50 1002.5/1057 us (+5.4% enabled) and p95 1758/2092 us (+19.0%). After the opt-in change, old opt-out build (`bin-t5-final`) vs new default-disabled build, off/new/new/off: walls 78.9/117.0/109.9/208.0 s; server p50 1057/1166/1240/1274 us and p95 1763/1928/1897/1990 us. That quartet ran under external load (one 208 s run), so it was repeated: walls 81.6/81.0/85.5/93.8 s, old p50 1063/1118 us vs new 1112/1204 us. Pooled over both quartets, old 1128/1816 us vs new 1180.5/1878.75 us (+4.7% p50 / +3.5% p95). The old build predates the main merge, so upstream changes are mixed into this comparison and it cannot isolate this change. Within the new build, enabling counters costs +10.3% p50 / +10.4% p95 (walls 71.5/77.9/81.2/75.9 s), which is the explicit opt-in path rather than the default. CPU was below 60% for a minute before each quartet, but shared-machine load makes the numbers noisy; the 1%/3% default-path target is not demonstrated.

- **Never-reached local aggregate (Q8):**

  | Table counter | Total | Reached | Never reached |
  |---|---:|---:|---:|
  | skills_cast | 357 | 1 | 356 |
  | missiles_spawned | 684 | 1 | 683 |
  | monsters_spawned | 734 | 8 | 726 |
  | monsters_killed | 734 | 1 | 733 |
  | areas_loaded | 137 | 1 | 136 |
  | npc_menus_opened | 59 | 0 | 59 |

- **What:** shows what the suite exercised and what it never touched, so new cases go to the gaps and "full" can be measured.
- **How:**
  - Read-only counters (no effect on ticks): skills cast, missiles spawned, monsters spawned and killed, areas loaded, NPC menus opened, packets sent and received by id, panels opened, item actions.
  - They are written into `repro.json` and `sweep.json`. `tools/ci/coverage.py` merges runs and lists what's never reached, by table (skills.txt rows never cast, and so on).
  - Optional: `cargo llvm-cov` on the farm for line coverage of the sim and server.
- **Files:** a counter resource in the client and server (read-only hooks next to existing events), `app/repro/` output, new `tools/ci/coverage.py`.
- **Done when:** a sweep run's coverage lists exact counts; the counters don't change `state-1` (determinism test); the never-reached report is checked into this plan as numbers only.
- **Risks:** hooks in hot paths. Counters are plain integers in a resource, and perf T3 checks for a slowdown.

### Q9. Static and robustness checks

- **What:** bugs found without running the game.
  - **Table checks:** `data-check [--mod NAME]`: every cross-table reference resolves (skill -> missile -> animation -> files, monster -> modes -> animations, item -> graphics/sounds, level -> presets), with no orphan or duplicate ids. Under a second; each mod runs it on its own tables.
  - **Fuzzing:** new cargo-fuzz targets for client and server message decoding (the server must never panic on any client message), the save loader, the mod-pack loader and the scenario parser, each with a seed corpus from synthetic data. Short runs in CI; long runs on the farm (Q10).
  - **Rule property tests** (proptest, in d2-sim and d2-server): damage and stat totals stay in range and are monotonic where the rules say so; item generation makes only legal affixes and level requirements; drop-table weights sum; pathfinding finds an existing path and never crosses blocked tiles.
- **Files:** a new `data-check` subcommand (tools/data-tool or d2-data), `fuzz/fuzz_targets/`, `crates/d2-sim/tests/prop_*.rs`, `crates/d2-server/tests/prop_*.rs`.
- **Done when:** each check has a planted-failure test; `data-check` passes on real files on BIGRIG and on the synthetic set in CI (or its failures are filed Unverified); each fuzz target runs 60 s clean.
- **Status:** cx-q9-static round 2 validation in progress (2026-10-18), PR https://git.werdegames.com/WerdeGames/D2RUST/pulls/338 (mergeable=true on opening). Release data-check: 85 tables, 474.331 ms total (387.346 ms checks), 71 Unverified candidates / 70 distinct messages. Formatting and strict all-target main/fuzz clippy exit 0; nextest exit 0 (5,412 passed, 139 skipped, 357.238 s). Client/server messages, save loader and mod-pack each fuzzed 61 s clean. Scenario retry queued without sccache after Windows command-length error 206 in its first instrumented build.

### Q10. Farm deep checks

- **What:** slow, deep checks run on the CI server every night, so they never slow a lane.
  - **Long runs:** a multi-hour soak and sweep loop that records RSS, unit counts and asset-handle counts over time and fails on steady growth (a leak).
  - **Perf trend:** tick and frame timings per main commit on fixed scenarios. A regression past a threshold runs Q4's perf bisect.
  - **System-order ambiguity:** Bevy's schedule ambiguity report for the client app. A new ambiguous pair (systems with conflicting data access and no set order) fails.
  - **Software-GPU shots:** Q6 shots rendered on a software Vulkan driver, so picture checks run without a GPU.
  - **Test quality:** `cargo mutants` on d2-sim and d2-server rule code, listing mutants no test catches (weak spots); Miri and sanitizers on the unsafe code in d2-native and d2-formats.
  - **Long fuzzing:** Q9's fuzz targets.
- **Files:** `tools/farm/` jobs, an ambiguity test in d2-client, docs.
- **Done when:** each job runs with the fake runner in tests and documents its server command; the ambiguity test passes on main with an allow list of the pairs that exist today.

### Q11. Multiplayer in one process

- **Status (Q11, 2026-10-19): built as `mp-test` (lane cl-mp-testtool), guide [`docs/tools/mp-test.md`](../../tools/mp-test.md).** It goes past the plan below: the server is a real process (the stand-in `mp-test-synth` or `d2rust-server`), bots connect over QUIC on loopback through a relay that injects latency, jitter and loss, and scenarios cover accounts, characters, the difficulty rule, lobby passwords and list, play together, reconnect, restarts and version refusal. Scenarios for parts not built yet are pending until their PR makes them required (contracts section 10). Gate: `tools/mp-test/gate.sh`.

- **What:** one server and several headless clients in one process, through the real message path with a simulated link (latency, jitter, loss, reorder, all in-process). Players join and leave mid-action, trade, party and fight the same monster. Q2's desync and conservation rules run on every client.
- **When:** after the multiplayer branches (mp/*) are on main. Planned now so the seams match.

### Q12. Goal checks (moved)

- **Moved (owner, 2026-10-19)** to [`game-explorer.md`](game-explorer.md) (G0-G5): a game map read from each game's own data, plus fast static and jump checks for every objective. It has no real-time play bot and works for every mod.

## Not planned

- **Golden images in the repository:** clean room. Reference shots stay on local disk (Q6).
- **Auto-fixing bugs found by tools:** found bugs are Unverified until the owner asks.
- **Network-socket tests:** in-process and loopback only, by owner rule.
