# D2RUST test suite: guide for AI sessions

This is how D2RUST tests itself without anyone playing. Read it before you reproduce a bug, fix one, or
add a test. The build plan is [`docs/plans/tools/test-suite.md`](../plans/tools/test-suite.md) (Q0-Q12). The
tools it builds on are in [`docs/plans/tools/test-automation-tools.md`](../plans/tools/test-automation-tools.md)
(T0-T10). Where a tool below is still being built, its module id is given; check the plan's status line.

## 1. What counts as a bug

A bug is a **broken stated rule**:
- a crash, panic or hang;
- a broken invariant (section 4);
- a written expectation that fails: a scenario `check`, a property test or a table rule;
- a behaviour or picture change that nobody intended (section 3, compare layer).

How the game looks, feels or is balanced is **not** a bug until the owner states the rule. Once the owner
states one, write it down as a check (a `check` line, a property, a table rule or an invariant), and the
suite enforces it from then on. That is how the suite grows: every stated rule and every fixed bug becomes
a permanent check.

## 2. Owner rules every tool keeps

- **Determinism:** tools read state or feed input through the player's input paths. A tool never changes a tick's result, and `state-1` stays byte-identical with tools on or off.
- **Dev tools stay on** by default.
- **Silent:** automated runs never make sound. Every game run gets `--mute` or `D2RUST_MUTE=1` (any subcommand; the variable wins over `--sound` and reaches child processes); `--offscreen` runs are muted unless `--sound` is given, and `--no-sound` mutes too. Muted means no audio output device is opened; the game's sound logic, `--sound-log` and the player's options run unchanged.
- **No real mouse, no focus stealing, no visible windows** (one minute at most when unavoidable), and **no network tests** (in-process and loopback only).
- **Clean room:** game files and game art stay on BIGRIG and the CI server. Shots and reference shots live on local disk (`E:/work/artifacts/`, `/cache/golden/`), never in the repo and never uploaded. Scenarios, hashes and numbers may be committed.
- **Unverified:** a bug the suite finds goes on the Unverified list (`scratch/bugs-unverified`). Nobody fixes it until the owner asks.
- **Resources:** heavy builds go through the build queue (`scratch/build-slot.sh`) or the farm. Tests wait for `ci-wait.sh` while a merge-train batch runs.

## 3. Layers, fastest first

Use the fastest layer that can see the bug.

| Layer | Tool | Sees | Speed |
|---|---|---|---|
| Static | `data-check` (Q9), clippy | broken table references, ids, files | < 1 s |
| Unit and property | `cargo nextest`, proptest (Q9) | rule code: damage, stats, item generation, pathing | seconds after the build |
| Fuzz | `cargo fuzz` (Q9; long runs Q10) | decoders: messages, saves, mod packs, file formats, scenarios | 60 s per target, hours on the farm |
| Scenario | `D2RUST-Game repro FILE` (T0-T2) | one situation, headless, full client and server | 6-10 s; < 1 s per case in a batch |
| Batch | `repro --batch DIR --jobs N` (Q1) | many scenarios on one loaded game | minutes for hundreds |
| Sweep | `D2RUST-Game sweep FAMILY` (Q3) | all content: skills, NPCs, areas, items, panels, generated from the tables | minutes per family |
| UI walk | `explore` (Q7) | every reachable screen and element | minutes |
| Pictures | `play --offscreen` (T5/T6), `img-check`, `shots.py` (Q6) | what the player sees, between ticks too | seconds per shot |
| Compare | `bdiff.py`, `bisect_runs.py` (Q4), `img-diff` (T7) | what changed between two builds, and which commit changed it | seconds to minutes |
| Perf | `perf-summary` (T3), trend (Q10) | hitches with causes, tick and frame budgets | one run |
| Running game | `playcheck` ([guide](playcheck.md)) | frame limit, even frames, stalls, skipped ticks; animation frames made while playing; flicker, hover highlight shape, text through the upscaler | a minute per setting or case |
| Farm | `farm.py` (Q5, Q10) | overnight soak, leaks, mutation testing, Miri, long fuzzing, remote builds | hours, off BIGRIG |
| Multiplayer | `mp-test` (Q11, [guide](mp-test.md)): up to 64 bots against a real server process, latency/loss/jitter relay | accounts, lobby, join/leave, play together, reconnect, restarts, version refusal, desync | seconds per scenario; the gate of every multiplayer PR |
| Goals | game explorer G0-G5 ([plan](../plans/tools/game-explorer.md)) | progress blockers in quests and areas, for any game on D2RUST; the G0-G3 light tier is in PR #416 and #419 | planned |

Binaries come from the cache (`tools/ci/bincache.py get SHA`, Q1), so running a test never needs a compile
unless you changed the code.

## 4. Oracles: how a run knows it failed

Every scenario, batch, sweep and explore run checks all of these. A scenario turns one off only with
`allow RULE`, which bug scenarios need to show the broken state.

- **Soak rules** (shared `app/checks/`, Q2): GUIDs unique and below their counter, no dangling list entries, no GUID reused as another class, no client/server desync that holds.
- **Conservation:** gold and items only appear or vanish through an event that creates or destroys them.
- **HUD equals server:** life, mana, gold and stats as drawn equal the server's numbers.
- **No stuck mode:** no unit stays in a mode longer than its limit (death and idle are exempt).
- **Animation bounds:** the frame index stays within the animation's frame count.
- **Walkable:** units stand only on walkable positions.
- **Clean log:** no panic, no `error!` line and no missing-asset fallback.
- **Save round trip** (Q3): save, load and compare the state at sweep checkpoints.
- **Picture rules** (Q6): no all-black or one-colour frame, no missing-texture colour, no change while nothing moves, no between-tick jump, no UI element off-screen or overlapping.

Violations have a stable **signature** (rule plus units, no tick), so the same bug found a hundred times is
reported once.

## 5. Fixing a bug: the loop

1. **Reproduce:** write `tools/scenarios/bugs/BUG-NNN.scn` (format: `docs/tools/scenarios.md`): `char` or `save`, `at T poke/dev/input`, `check` lines and `end`. Use names, not pixels: `cast NAME at unit`, `action NAME` and `click ui:ID`. Run `repro`; it must fail on main.
2. **Localise:** if it used to work, run `bisect_runs.py BUG-NNN.scn --good SHA --bad main` (Q4). For a visual bug, take shots at the exact tick and phase (`--display-clock phases:..`, `--shot-phase`).
3. **Fix** the source of truth (data tables or rule code, never generated output).
4. **Prove:** the scenario passes on the branch and fails on main; paste both `repro` lines in the PR. Then run the families the change touches (`sweep skills --filter ...`). For a refactor, run `bdiff.py main HEAD` to show nothing else changed.
5. **Generalise** (the adaptive step). Ask whether the bug can happen elsewhere:
   - **a whole class of content:** add a case or rule to the sweep generator, so every skill, NPC or item gets it;
   - **a state that should never exist:** add an invariant to `app/checks/`;
   - **a data mistake:** add a `data-check` rule;
   - **bad input:** add a fuzz seed;
   - **a picture:** add an `img-check` rule or a mask.

   Then one bug becomes a check that guards everything like it.

## 6. Extending the suite

Each extension point is one small, local change:

| To add | Where | Size |
|---|---|---|
| A query name for `check`/`query` | the name table in `app/repro/query.rs` | one line plus a reader |
| An invariant | a file in `app/checks/`, registered in the rule list, with a planted-failure test | one function |
| A sweep case or family | a generator in `app/sweep/<family>.rs` reading the tables, with skip rules and a skip count | one generator |
| A table rule | `data-check` rule list | one function |
| A fuzz target | `fuzz/fuzz_targets/<name>.rs` plus seeds | one file |
| A picture rule | `tools/img-diff` `img-check` rules | one function |
| A running-game check | a module in `tools/playcheck/` over `offscreen.Run` | one module plus a synthetic test |
| A farm job | `tools/farm/` job list | one entry |
| A stated owner rule | the cheapest layer that can check it: table rule, property, invariant or scenario `check` | per rule |

Rules for extensions:
- Every check prints the actual value when it fails.
- Every new rule ships with a test that shows it fires.
- Mods: anything that reads game tables also runs per mod (`--mod NAME`), because each mod is its own game on D2RUST.

## 7. Bug reports from players and other sources

A report that doesn't come from the owner is **Unverified**: file it, don't fix it. To make it actionable,
turn its `bugreport` state line into a scenario (`char`/`poke` lines; the table is in `docs/bugs/README.md`).
Run it, attach the `repro` result to the Unverified entry, and wait for the owner. Bugs the suite finds by
itself (sweep, soak, explore, farm) go into the same Unverified list, grouped by signature.

## 8. Speed budget

- Single scenario: under 10 s cold (release), under 1 s per case in a batch.
- d2-client test gate: a few minutes after Q0 (was 29 min).
- Each sweep family: a few minutes on BIGRIG with `--jobs 4`.
- Anything longer goes to the farm.
- Gates test only the crates a change touches (`tools/ci/affected.py`); the merge train's batch runs everything.

## 9. UI exploration and event coverage

Run each resolution in a separate process (frame size is process-wide):

```powershell
D2RUST-Game explore --front --out E:/work/artifacts/explore/front
D2RUST-Game explore --game --res 800x600 --out E:/work/artifacts/explore/800
D2RUST-Game explore --game --res 1068x600 --out E:/work/artifacts/explore/wide
D2RUST-Game explore --game --res 1068x600 --offscreen --out E:/work/artifacts/explore/shots
py -3 tools/ci/coverage.py E:/work/artifacts/explore/800/explore.json --out E:/work/artifacts/explore/coverage.json
```

`D2_GAME_DIR` or `--game-dir` selects the local installation. Game starts default
to a seeded new Amazon; `--save`, `--new`, `--seed` and `--difficulty` use the
same arguments as autoplay-host. Front walks stop at the game handoff; `--game`
walks the in-game panels separately. Front exploration uses an output-local empty
save folder unless `--save-dir` selects a fixture folder. Each route uses a
fresh output-local copy of its `.d2s` files, isolating normal creation/deletion
actions from the seed folder and other routes.
`--max-states 128 --repeats 2` bounds the breadth-first walk; sliders probe only
the two legal endpoints. A limited walk reports `complete: false`. Every edge
restarts a session and replays input through T9 IDs; the report records routes,
observable UI, text bounds, Q2 violations and Unverified signatures. A finding
or an incomplete walk returns nonzero. These candidates require owner review.
Response checks observe element IDs, control values and text. Changes confined
to animation, sound or pixels can therefore produce Unverified `no_response`
candidates; review their local route captures before filing a stated-rule bug.

`--offscreen` replays visited in-game routes through T5 in hidden child processes
at 1920x1080 and stores local PNGs. The headless checks measure original font
layout against control boxes or text clip rectangles; screenshots remain inputs
for separate picture checks. Front-end offscreen capture is not supported.
The hidden control bridge accepts the same in-game `ui` and `click ui:ID` commands.

`repro.json` and `explore.json` include a shared `coverage 1` envelope. Table
universes and counters contain IDs and counts only. Counts include successful
skill starts, allocated missiles, initialized monsters, completed monster-death
starts, area transitions, NPC menu sends, UI open transitions and transport packet
IDs; item-action counts are sent packet intents, including refused requests.
Exploration totals include its replay sessions, not just distinct states.
Counters default off in normal play and autoplay hosts. Only repro and explore
opt in; `D2_COVERAGE=0` disables them there for same-binary performance comparisons.
Disabled hooks check one inline guard before evaluating an ID. Enabled hooks use
static categories and numeric keys, formatted only when writing reports. Existing
client transport diagnostics remain available.

Merge explicit report files, or directories containing `repro.json` (falling
back to explorer or sweep reports). Never supply both a sweep aggregate and its
members: that duplicates counts. `never_reached` lists numeric table rows with
zero events; it does not claim that every row is spawnable or a valid test case.
Q3's sweep writer is absent on this base: its integration TODO and shared merge
struct live in `app/coverage.rs`.
