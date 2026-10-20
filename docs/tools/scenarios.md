# Scenarios and `repro`

A scenario is a small text file that says who plays where, what they do and (with checks) what
must be true. `repro` runs it headless in seconds and answers pass or fail:

```sh
D2RUST-Game.exe repro tools/scenarios/bugs/BUG-008.scn --out E:/work/artifacts/<lane>/bug-008
```

It prints one line (`repro: PASS name (40 ticks, 1800 ms); artifacts in ...`) and exits with:

| Exit | Meaning |
|---|---|
| 0 | every step ran (and every check held) |
| 1 | a step or a check failed: the line names the first one |
| 2 | the scenario is invalid, or asks for something `repro` can't build yet |
| 3 | the game files are missing (`--game-dir DIR` or `D2_GAME_DIR`) |

Options: `--out DIR` (default `repro-out/<name>`), `--game-dir DIR`, `--keep-going` (run every step
and check even after a failure), `--query "NAME, ..." [--at T,...]` (see [Queries](#queries)).

For converted data use `--native DIR`, optionally with `--mod NAME` for exactly
that mod over the base. This selection ignores the mod order file and also works
with `repro --batch`. Use the same selection to replay a saved sweep failure.
`--native` and `--game-dir` are mutually exclusive; soak scenarios retain their
existing archive-based source.

Artifacts stay on the machine that ran them (never committed, never uploaded). On BIGRIG use
`E:/work/artifacts/<lane>/`.

| File | What it holds |
|---|---|
| `repro.json` | format `repro 1`: verdict, exit, the first failure, every step's and check's result, query answers, ticks, build, wall time. Two runs of the same scenario give the same file apart from `wall_ms`. |
| `state-tail.jsonl` | the server's `state-1` snapshot of the last 10 ticks |
| `client-state.json` | the client's view at the end (the `autoplay-host` `state` reply: player, camera, open panels, ground items with their label text, HUD numbers, hover, messages, the cursor item `client.cursor`) |

## How a run goes

1. The game starts on the headless client (`autoplay-host`'s app: the full client and UI, no
   window, rational 60 Hz stepping deadlines, so one pass is one tick).
2. The character joins, then the start setup runs: the level (`char level`, through the dev `level`
   command, so life, mana and points are a real character's), `char stat` values, the warp to
   `char area` and the place `char at`. The client then gets 8 ticks to settle.
3. **Tick 0** is the first tick after that. `at T ...` runs once T ticks have passed, before the
   next tick. Steps at the same tick run in file order within their kind.
   A check at T reads the game once T ticks have passed, before the steps at T run: to check a
   step's effect, check a tick or more later.
4. The run stops after `end` ticks.

## Format (`scenario 2`)

Lines are `keyword values...`; `#` starts a comment. Header lines come first, steps (`at`, `by`)
last, in tick order.

### Header

| Line | Default | Meaning |
|---|---|---|
| `scenario 2` | required, first | the format version |
| `tick_rate HZ` | 25 for legacy files | units for scenario times, 1–1000 Hz |
| `name NAME` | required | `[a-z0-9-]+` |
| `end T` | required | ticks to run after tick 0 |
| `seed N` | 1 | the game seed |
| `difficulty normal\|nightmare\|hell` | normal | |
| `expansion yes` | yes | `repro` runs expansion games only |
| `init N` | 0 | kept for the format; not used by `repro` |
| `allow RULE` | no opt-outs | disable one always-on rule; repeat for multiple rules |

Every run also checks server GUIDs, lists, vitals, rooms and inventories; client/server
agreement; HUD totals; item and gold conservation; stuck modes; animation bounds; actor
terrain walkability; panics; errors; and missing-asset fallbacks. Checks read snapshots between
ticks. HUD mismatches allow two client passes after publication. The vitals/gold/experience
sync can wait 20 server updates before forcing a flush, so those fields get 22 observations;
saved-stat fields get three. Persistent mismatches then fail.
The HUD comparison follows the install's replication contract: vitals, experience and gold
use their dedicated sync; other fields require the stat row's `saved` flag.
Stationary movement has a 250-tick limit; action modes have a 125-tick limit. Idle and death
are exempt, and movement or a new mode request resets the timer.

The rule names accepted by `allow` are `guid`, `list`, `vitals`, `room`, `item`, `desync`,
`hud`, `conservation`, `stuck`, `animation`, `walkable`, `panic`, `error`, and `missing-asset`.
Use opt-outs for intentional broken states. Unknown names are parse errors. Opt-outs are
included in the scenario's canonical text and hash. `repro.json` lists each violation's
`rule`, stable `signature`, `detail`, and observation tick (`at`, null for diagnostics).
Repeated signatures are reported once; any unallowed violation makes the run exit 1.

The conservation ledger compares total quantities by item row and total player/stash/ground
gold. Transfers grant no exemption. Explicit source/sink events (vendor requests, cube,
consumption, deaths, expiry, replenishment and dev setup) permit the corresponding direction
of change during that event window. This checks unexplained creation/loss; it does not
validate vendor prices or recipe quantities.

`at T`, `by T`, `end T`, snapshot times, query `--at` values and input
`wait-until ... max N` and `hold X Y N` use the scenario's rate. The runner converts each time
to `ceil(T * game_hz / scenario_hz)` game ticks, preserving its real-time
meaning without running it early. For example, `max 25` at 25 Hz becomes
60 game ticks at 60 Hz. Missing `tick_rate` retains the historical 25 Hz meaning;
new files should state the rate explicitly. Standalone and `repro --batch`
use the same conversion. Reports identify both `scenario_tick_rate` and the
game's `tick_rate`; reported step/query times and `end` are in game ticks,
while `scenario_end` and the scenario hash describe the source file.

Soak scenarios retain an exact action log rather than this scenario grammar.
Their `steps` and action indices use the embedded `soak-log`'s explicit
`tick_rate`; missing or mismatched rates are refused with a re-recording
message. They are never implicitly retimed.

Picture manifests under `tools/scenarios/pictures/` also state their rate.
Their raw input/frame schedules and capture ticks are authored at that rate;
phase fractions are converted to milliseconds using it. The checked-in
town-walk manifest is retimed to 60 Hz.

### Character: a save file, or inline lines

- `save FILE`: a `.d2s` file, relative to the scenario's folder or absolute.
- Inline (any `char` line makes one):

| Line | Default | Meaning |
|---|---|---|
| `char class N` | required | 0 Amazon, 1 Sorceress, 2 Necromancer, 3 Paladin, 4 Barbarian, 5 Druid, 6 Assassin |
| `char level N` | 1 | 1-99 |
| `char area ACT LEVEL` | `0 1` | the act (0-4) and level id to start in (a warp when it isn't the act's town) |
| `char at X Y` | the level's start | the player's position in sub-tiles |
| `char skill ID LEVEL` | none | a class skill (skills row) at a base level |
| `char hand left\|right ID` | the class default | the skill on that mouse button |
| `char stat ID VALUE` | none | a stat (itemstatcost row) set after the level-up |

`char item`, `char waypoint`, `char quest` and `char map` are part of the format but `repro` can't
build them yet (exit 2). Give items with `at T dev item ...` or `at T poke item ...`, waypoints with
`at 0 dev waypoints`.

### Steps

| Step | Runs | Example |
|---|---|---|
| `at T poke DIRECTIVE` | on the server, as `play --poke` (pos, warp, hop, goto, object, missile, item, stat, state, freeze, time, msg, ...) | `at 0 poke warp 2` |
| `at T spawn CLASS X Y KIND [umod ...]` | a monster (monstats row), as the poke `spawn` | `at 1 spawn 19 @x+12 @y normal` |
| `at T msg NAME field=value ...` / `at T hex ..` | a C→S message injected as `play --send` | `at 2 msg Walk x=@x+5 y=@y` |
| `at T dev CMD` | a dev console line, through the dev tools' server end | `at 0 dev god on` |
| `at T input STEP` | one step of the `--input` grammar, played by the client's input system as `play --input` | `at 8 input rclickunit 1 19` |
| `at T check EXPR` | a check: EXPR must hold after tick T ([Checks](#checks)) | `at 12 check player.mode == cast` |
| `by T check EXPR` | a check: EXPR must hold after some tick from 0 to T; it passes at the first such tick | `by 40 check ground_item(gold).label == "1234 Gold"` |

References in pokes, spawns and messages: `@player`, `@x±N`, `@y±N` (the player's position plus an
offset), `@TYPE[:CLASS][#N]` (a unit), `@wp`. Input steps are listed in [Input steps](#input-steps).
Their timing comes from `at`, so `frame` and `wait` aren't steps here.

### Input steps

The `--input` grammar, shared by `repro`, `play --input` and `state-dump --input`. Points are pixels
of the play frame. Units are picked by type T (0 player, 1 monster, 2 object, 3 missile, 4 item,
5 tile) and class C (`*`: any living one).

| Step | Does | Example |
|---|---|---|
| `move X Y` | the cursor to (X, Y) | `move 400 300` |
| `click X Y` / `rclick X Y` | a left / right click there | `rclick 500 260` |
| `click ui:ID` | a left click on the UI element ID, wherever the frame lays it out; an id not on screen fails with the ids that are. The front end (`autoplay-host --front`) and the open game panels have ids (`npc.menu(N)`, `npc.topic(N)`, `dialog.skip`, `waypoint.tab(N)`, `waypoint.row(N)`, `waypoint.close`, `options.<page>.<row>` in the Esc menu); `ui` lists them | `click ui:menu.single_player` |
| `hold X Y N` | a left press, released N ticks later | `hold 560 300 10` |
| `down L\|R X Y` | that button pressed and kept down until its `up`; the other button is free meanwhile | `down L 560 300` |
| `up L\|R X Y` | that button released | `up L 560 300` |
| `move-held X Y` | the cursor to (X, Y) while a `down` button is held (a drag) | `move-held 600 320` |
| `key K` / `wkey K` | a key press: a letter, digit, `ESC`, `TAB`, `ENTER`, `SPACE`, `F1`-`F12` or a virtual-key number | `key I` |
| `keydown K` / `keyup K` | a key held until its up (not `state-dump`) | `keydown SHIFT` |
| `action NAME` | a key action by its controls name, as a key bound to it sends it | `action toggle_inventory` |
| `clickunit T C [DX DY]` / `rclickunit ...` | a click on the nearest unit of type T and class C; `superunique ROW` names the monster spawned as that `superuniques` row instead | `rclickunit 1 19`, `rclickunit superunique 7` |
| `skill L\|R NAME` | selects skill NAME on that hand, as the skill menu does | `skill R fireball` |
| `cast NAME at unit T C [DX DY]` | `skill R NAME`, then a tick later `rclickunit T C` (or `rclickunit superunique ROW`) | `cast fireball at unit 1 *`, `cast 36 at unit superunique 6` |
| `cast NAME at X Y` | `skill R NAME`, then a tick later a right click at (X, Y) | `cast fireball at 500 260` |
| `wait-until EXPR max N` | the input steps after it wait until the check EXPR holds, read after each tick, for at most N ticks | `wait-until player.mode == cast max 20` |

- **Skill names** are the player's `skills.txt` names in any case, with spaces, `_` and `-` ignored
  (`fireball`, `fire_ball`, `Fire Ball`), or a skill id (`47`). The player must have the skill.
- **Action names** are the names in the controls file (`toggle_inventory`, `toggle_run`,
  `skill_slot_1`, ...). `state-dump` refuses an action it can't run headless.
- **`wait-until`** uses the [check](#checks) grammar. Once EXPR holds, the input steps after it run,
  and every later input step moves later by the ticks the wait took, so their spacing stays as
  written. Checks, pokes, messages and dev lines keep their own ticks, so use a `wait-until` (not a
  `by T check`) when an input step must follow an event. A wait that never holds fails the run and
  stops the script. `wait-until` reads the client state that `repro` builds, so `play` and
  `state-dump` refuse it.
- **Failures:** a step that can't run (no such unit, a skill the player lacks, a `wait-until` that
  never held) fails the run, with the reason in the step's result.

A result other than `ok` (an unresolved reference, a refused poke, a dropped message, a dev
error) fails the run.

`poke goto preset LEVEL 5:WARP` can approach a loaded interaction tile for a
level-link case, using the same bounded goto setup as monsters and objects.
`poke goto superunique ROW` (or `goto preset LEVEL superunique ROW`) walks to
the monster spawned as that `superuniques` row, not to the first monster of
its class (its minions often share the class). A trailing `mode M` on the
`unit` and `preset` forms keeps only a unit in that mode (`poke goto preset 28
2:15 mode 0`: a closed door). A last `opening` lands the player a few
sub-tiles in front of a door's opening (the middle of the footprint cells with
no wall under them) on the positive side of its narrow axis, instead of
next to its position. A goto walk takes several ticks
and later input steps do not wait for it: put a `wait-until` before the input
that needs the target (`wait-until superunique(40).guid max 900`).

### Conditional player keys

`at T key-if VK EXPR` reads a check expression and sends the virtual key through
normal player input only when it holds. `VK` is a supported numeric virtual key
(for example `32` for Space). The report records the observed value and whether
the key was sent or skipped. Sweep greeting setup uses
`at 12 key-if 32 client.dialog_lines != null`, avoiding extra keys once the dialog
has ended. Each scheduled key is evaluated once, including while paused.

### Save checkpoints

`at T checkpoint` (scenario 2) serializes the live saved state, loads it into
an independent headless game, and compares the restored state and a second save.
It uses the soak round-trip comparisons, including expected load normalization.
The active game is unchanged. Failures are `checkpoint` outcomes with stable
`roundtrip:*` signatures. Generated [sweeps](sweep.md) include this step so exported
failures replay it. Checkpoints use the scenario tick rate.

### Checks

A check is `NAME`, `NAME OP VALUE` or `NAME changes`:

- `NAME` alone holds when the value is true, a non-zero number or a non-empty text or list.
- `OP` is `==`, `!=`, `<`, `<=`, `>`, `>=`, or `~ VALUE N`: every number within N
  (`player.pos ~ (5120, 3400) 2`).
- `VALUE` is a number (`12`, `0x0C`), `"quoted text"`, a bare word (`cast`), a list `(a, b)`,
  `true`, `false` or `null`.
- `NAME changes` holds when the value differs from the tick before (`at 30 check
  unit(monster, 19).anim_frame changes`).
- A name that reads nothing (no such unit) is `null`: it equals only `null` and fails every other
  comparison.

A failing check is a failed step: its result is the value it read (`actual: 45`, or for `by`:
`never held by tick 40; last: "Gold"`), and the verdict line shows it.

### Names

Every name reads the headless client's `state` reply after a tick: the server's `state-1` snapshot
(the authority), the client's view and the HUD numbers.

| Name | Reads |
|---|---|
| `player.FIELD` | the local player's server unit (`dist` is 0) |
| `unit(TYPE, CLASS).FIELD` | the first server unit of that type and class; TYPE `player`, `monster`, `object`, `missile`, `item`, `tile` or 0-5; CLASS a number or `*`; `dist` is the Chebyshev distance to the local player in sub-tiles |
| `count(TYPE, CLASS)` | how many server units match |
| `superunique(ROW).FIELD` | the client's monster spawned as that `superuniques` row: `guid`, `class`, `mode`, `pos`, `dead` (dying or dead), `dist` (Chebyshev distance to the local player in sub-tiles), `life` / `life_max` (the server unit with that GUID, not the first of its class) |
| `near(TYPE, CLASS).FIELD` | the server unit of that type and class nearest the local player: any unit field, or `ax` / `ay` (the player's x / y minus the unit's, in sub-tiles) and `dist` (the larger of their sizes) |
| `ground_item(CODE).FIELD` | the first ground item with that code (`gold` for `gld`, `*` for any): `label` (the Show Items label text, shown or not), `gold`, `code`, `guid`, `x`, `y` |
| `ui.open(NAME)` | whether a UI state is open: `inventory`, `character`, `skill_select`, `skill_tree`, `chat`, `new_stats`, `new_skills`, `npc_menu`, `esc_menu`, `automap`, `npc_shop`, `show_items`, `socket`, `quest_screen`, `quest_log`, `hireling_icons`, `waypoint`, `mini_panel`, `party`, `message_log`, `stash`, `cube`, `hireling_inventory`, or a number |
| `sent(ID)`, `received(ID)` | how many C→S / S→C messages with that id the client sent / received |
| `hud.NAME` | the numbers the HUD draws from: `life`, `life_max`, `mana`, `mana_max`, `stamina`, `stamina_max`, `gold`, `level`, `exp` |
| `quest(SLOT, BIT)`, `quest_word(SLOT)` | whether a quest flag bit is set, or a flag slot's whole word, for the local player |
| `server.PATH`, `client.PATH` | any field of the snapshot or the client view, keys and list indices by dots (`server.f`, `client.player.pos.0`, `client.weapon_set`) |
| `anim_rate(TYPE, CLASS, MODE).FIELD` | how fast the first unit of that type and class was drawn in MODE (a mode name, a number or `*`: its mode now): `fps` (the drawn frames per second, null before 20 ticks of the mode), `expected_fps` (the server's speed for the unit in that mode, `speed / 256` frames per 25 Hz base tick; null while the server unit is in another mode), `ratio` (`fps / expected_fps`), `ticks`, `advances`, `frames` |

Unit fields: `pos` (`[x, y]` in sub-tiles), `x`, `y`, `target`, `mode` (a name: `neutral`, `walk`,
`run`, `cast`, `attack1`, `gethit`, `town_walk`, ...), `mode_id`, `dir`, `anim_frame`,
`anim_frames`, `anim_speed`, `life`, `life_max`, `mana`, `mana_max`, `stamina`, `stamina_max`
(whole points), `level`, `area` (level id), `act`, `class`, `guid`, `type`, `owner`.

`client.view` is the world view's last built frame: `tick` (the server tick it shows), `items`
(its draw items), `tiles` (its map tiles) and `skipped` (frames the view could not build and
skipped instead of stopping the game; 0 in a healthy run).

The names live in one table each in `crates/d2-client/src/app/repro/query.rs`; a new name is one
line there.

### Queries

`--query` reads names while exploring, with no checks to write:

```sh
D2RUST-Game.exe repro bug.scn --query "player.pos, unit(monster, *).mode, hud.life" --at 0,40
```

It prints one JSON line per tick (`{"at":40,"values":{"player.pos":[5120,3400],...}}`) before the
verdict line, and the answers go into `repro.json` (`queries`). Without `--at` it reads the end
tick. `autoplay-host` takes the same names as a command: `query player.pos, hud.life`.

## Live control (`play --control`)

`play --new CLASS NAME|--save F [--offscreen] --control stdio|pipe` drives the real game, windowed
or offscreen, with the same `autoplay-1` lines as `autoplay-host`. Control is off without the flag.

- `stdio`: commands on stdin, one JSON reply per command on stdout (the reply lines start with
  `{`; the game's log shares stdout). The game ends when stdin closes.
- `pipe`: a local named pipe `\\.\pipe\d2rust-<pid>` (Windows; the path is printed on stderr).
  One driver at a time; the game keeps running when it leaves. No network port.

The game starts held and sends `{"k":"hello",...,"tick":N,"paused":true}` once the player is in
the game. Added to the `autoplay-host` commands:

| Command | Does |
|---|---|
| `pause` / `resume` | hold the game (frames still draw, no server tick runs) / let it run on its clock |
| `step-ticks N` | run exactly N server ticks, then hold; the reply has the new `tick` |
| `step N` | run N frames (held: no tick runs) |
| `shot PATH` | write a PNG of the presented picture; the reply has its `size` |

Pointer and keys come only from the channel: the real mouse is never read or moved. An input
script (`--input`) runs alongside, on the same ticks as without control.

## `scenario 1`

The first version (a `game` line, every header line required, no `dev`/`input`/`check` steps, `char
save NAME`) still parses and writes back unchanged. `repro` runs only `scenario 2`.

## Examples

`tools/scenarios/bugs/` holds one scenario per bug ticket (`BUG-<number>.scn`). How they're
written, how a fix PR proves itself with one, and how a bug report's state lines become scenario
lines: [`docs/bugs/README.md`](../bugs/README.md).

`tools/scenarios/view/` checks that the world view never stops the game: per act, warps (and in
act I a waypoint) from the town into outdoor levels, each followed by a drawn frame with map
tiles and no skipped frame.

## Route runs and logs

`tools/autoplay/replay.py` replays the fixed input routes in `tools/autoplay/routes/`
through `autoplay-host`. It prints its work directory under `target/autoplay-replay-*`;
each route's `game.log`, rotated `game.prev.log` and panic `crash.log` go under
`<work>/<route-name>/logs/`. Replay sets `D2RUST_HOME` to `<work>/user/` as well,
so startup migration, saves and settings stay out of the player's Documents folder.
Explicit `D2RUST_LOG_DIR` and `D2RUST_HOME` environment values take precedence.

For a direct `autoplay-host` invocation, set `D2RUST_LOG_DIR` to a directory under
your run's output folder **before starting the executable**. Set `D2RUST_HOME` to
a scratch user folder too when isolating the whole run. For example, in PowerShell:

```powershell
$env:D2RUST_LOG_DIR = 'E:/work/artifacts/<lane>/route/logs'
$env:D2RUST_HOME = 'E:/work/artifacts/<lane>/route/user'
# Invoke autoplay-host or tools/autoplay/replay.py with the route's usual arguments.
```

The client reads `D2RUST_LOG_DIR` before opening or rotating logs. It creates the
chosen directory when needed; if it cannot, game logging stays on stdout and the
crash log falls back beside the executable, without retrying in Documents.
An unset or empty value keeps the player default (`Documents\D2RUST\logs`, or
`$D2RUST_HOME/logs` when that is set). This option changes only logs,
not saves or settings.


## Warm batches and ready-built binaries

Run many scenarios without loading the game tables for every case:

```text
D2RUST-Game repro --batch tools/scenarios/bugs --jobs 4 --out E:/work/artifacts/batch
D2RUST-Game repro --batch cases.txt --jobs 4 --out E:/work/artifacts/batch
```

A directory is searched recursively for `.scn` files in sorted path order. A list
is UTF-8, one path per line, relative to the list's directory; blank lines and
lines starting with `#` are ignored. List order and duplicate entries are preserved.
The default is one worker. `--jobs` must be positive and requires `--batch`.
The usual `--game-dir`, `--keep-going`, `--query` and `--at` options apply to each case.

Each worker owns a `repro::Session`: data is loaded once, then each scenario gets
a new server game, clock, client and join. Mutable UI, held input, hooks, character
state and message counters are discarded between scenarios. This keeps standalone
`repro 1` output unchanged apart from wall time. A session's data directory stays
fixed for its lifetime.

Artifacts go into numbered directories (`0001/repro.json`, `0002/repro.json`, ...)
in input order, so identical scenario names and duplicate list entries are safe.
Each successful run also writes the usual state tail and client state. Invalid
cases and load errors leave a `repro.json` error record and do not stop the batch.
`batch.json` (`batch 1`) lists each input, output directory, verdict, exit code and
elapsed milliseconds, totals (including wall time and summed worker time), and the
ten slowest cases. Its exit code is the greatest per-case exit code (0 pass, 1
failure, 2 invalid, 3 missing game files). Wall timings include worker data loading
in the first case; per-scenario reports time the warm run itself.

Use the binary cache to avoid compiling a scenario run:

```text
py -3 tools/ci/bincache.py build COMMIT_SHA
py -3 tools/ci/bincache.py get COMMIT_SHA
py -3 tools/ci/bincache.py build COMMIT_SHA --profile dist
py -3 tools/ci/bincache.py prune --keep 10
```

`get` prints only an existing binary path and never builds; a miss exits 1 with a
build command. SHAs can be abbreviated to at least seven hexadecimal digits and
are resolved in the local clone. `build` makes a detached worktree under
`E:/work/bin/src/<full-sha>`, then runs `cargo build -j 12 --profile release`
through BIGRIG's shared build queue. The queue sets the effective concurrency
(currently six compile jobs and six test threads per slot). `--profile dist` uses the dist profile.
Release binaries live directly under `E:/work/bin/<full-sha>/`; dist binaries
live under its `dist/` subdirectory. `D2_BIN_ROOT` overrides the cache root (the
Linux default is `/cache/bin`). Build logs go to stderr; stdout is the binary path.
A per-commit lock prevents overlapping publication or pruning; retry a busy entry.
Modified source worktrees are refused. Cache misses and failed builds never
publish a partial executable.

When `D2_FARM_HOST` is set, release builds instead invoke
`tools/farm/farm.py bin SHA` as an external process and check that it populated the
cache. Q5 supplies a Linux `D2RUST-Game` binary, which stays separate from a
local Windows `.exe`; with `D2_FARM_HOST` set, release `get` selects that Linux
artifact for execution on Linux/the farm. Unset it to select a native Windows
binary for local repro runs. Missing Q5 tooling produces an actionable error. Farm dist builds are not
yet supported. `prune` keeps the newest N commit directories by publication time,
skips busy entries, removes their clean source worktrees with Git, and ignores
unrelated directories. Game data and run artifacts do not belong in the cache.

## Behaviour comparison, replay and bisect (Q4)

Compare executable paths or locally resolvable commit refs with the same cases:

```text
py -3 tools/ci/bdiff.py GOOD_SHA HEAD tools/scenarios/bugs --out E:/work/artifacts/cx-q4-bdiff/compare --game-dir INSTALL
py -3 tools/ci/bdiff.py GOOD_SHA HEAD cases.txt --replay 2 --ignore fr,sp --out E:/work/artifacts/cx-q4-bdiff/replay
py -3 tools/ci/bisect_runs.py bug.scn --good GOOD_SHA --bad HEAD --out E:/work/artifacts/cx-q4-bdiff/bisect
```

`bdiff` accepts one `.scn`, a recursively searched directory, or a UTF-8 path list
(paths relative to the list, comments and blank lines allowed). Defaults to the
bug scenarios. `--sweep FAMILY` instead invokes Q3's `sweep FAMILY --out DIR`
once on B, then replays its saved `DIR/scenarios/*.scn` on both builds. B must
support Q3; an unsupported sweep command or an empty family is an error. Cases
are generated once so both builds receive identical inputs. To reuse an earlier
sweep, pass its `scenarios` directory as `SCENARIOS` without `--sweep`.

Refs resolve locally to full SHAs and use Q1's `Cache.get`; a miss calls only
`Cache.build`, which owns the queued build. Executable paths bypass the cache.
`--profile release|dist` defaults to release. No game build is launched directly.
No farm/runner folder is accessed by these tools except when explicitly enabled
through Q1's `D2_FARM_HOST` configuration.

The existing read-only `server.f`, `server.seed`, and `server.units` queries
export the entire `state-1` snapshot, including tick zero. The short state tail
is never used as a substitute for a full replay. Scenario deadlines and input
`hold`/`wait-until` durations are converted with the runner's ceiling rule to an
explicit `tick_rate 60` copy, and relative save paths stay relative to the source.
Source files stay untouched. Binaries must support 60 Hz scenarios and report
their rate, completed tick count and query answers; older incompatible binaries
produce an error. Queries are split into groups of 500 to fit Windows command
limits; each group replays the complete scenario from a fresh isolated home.
Long scenarios therefore cost more than one run. Use `--replay N` to check N
additional complete replays of **each** build against that build's first run.

Comparison matches units by `(ut,g)`, reports the first differing game time,
both snapshot frames, unit and field, and keeps a `--context N` tick window
(default 2). Unit order and header build/date metadata do not affect comparison.
Game seeds, unit seeds, fields newly added or removed, and missing/extra units
do affect it. `--ignore FIELD[,FIELD...]` is repeatable and skips exactly named
fields (or dotted object paths). Use it only for an intended change; ignored
fields also apply to replay checks. Arrays are compared as complete field values.
`bdiff.json` holds the full result and replay details. Each run has `run.json`
with wall seconds, the normalized script, chunk reports and a process log.
Outputs must be fresh; existing per-run directories are refused.

Exit 0 means same (or within the perf threshold), 1 means changed or a completed
scenario failed a check, and 2 means invalid/incomplete input, crash, timeout, missing files or a
failed cache build. Such execution errors never count as a successful comparison
or as a bad bisect verdict. A completed failed scenario is a bad behaviour
verdict; the good endpoint must pass. Perf sampling requires passing scenarios.
Differences from real-file runs are **Unverified
candidates**: retain the signature and repro command for owner review, never
automatically fix game behaviour.

### Tick-rate equivalence at equal game time

Record two headless runs with `state-dump --tick-rate HZ --every 1`, using the
same seed/save and inputs placed at the same game times. For example, idle runs:

```text
D2RUST-Game state-dump --seed 1234 --ticks 120 --tick-rate 60 --every 1 --out E:/work/artifacts/cx-q4-bdiff/60.jsonl --game-dir INSTALL
D2RUST-Game state-dump --seed 1234 --ticks 50 --tick-rate 25 --every 1 --out E:/work/artifacts/cx-q4-bdiff/25.jsonl --game-dir INSTALL
py -3 tools/ci/bdiff.py E:/work/artifacts/cx-q4-bdiff/60.jsonl E:/work/artifacts/cx-q4-bdiff/25.jsonl --dumps --equal-time --out E:/work/artifacts/cx-q4-bdiff/rates
```

`--dumps` reads existing `state-1` JSONL; it never runs a binary. Header
`tick_rate` is required. `--equal-time` matches exact rational times, never a
rounded frame index or nearest neighbour. At 25/60 Hz the common samples are
every 0.2 seconds; different-rate frames between those times cannot be compared.
The number of compared samples is in the report. Both recordings must cover
every tick and start/end at the same game times; sparse, duplicate or unordered
snapshots are errors, and unequal time ranges differ. For recordings that begin
at scenario tick zero after setup, the header can state `start_frame` (default 0).
Frame numbers align time; `f` itself is not a behaviour field.

### Performance trend and performance bisect

```text
py -3 tools/ci/bdiff.py GOOD_SHA HEAD fixed.scn --perf --metric server.p95 --repeats 5 --threshold 10 --out E:/work/artifacts/cx-q4-bdiff/perf
py -3 tools/ci/bisect_runs.py fixed.scn --good GOOD_SHA --bad HEAD --perf --metric frame.p95 --repeats 5 --threshold 10 --out E:/work/artifacts/cx-q4-bdiff/perf-bisect
```

The trend report identifies both input refs and resolved executable paths, and
retains each sample, median in microseconds, threshold and percentage change for
every case (percentage is null for a zero baseline). Each sample runs the unmodified scenario in a fresh process with
`D2_PERF_OUT`; `--metric PHASE.STAT` reads the embedded `perf 2` summary (default
`server.p95`). The head is over threshold only when its median exceeds the base
median by the given percentage (default 10). Missing/nonfinite metrics are
errors. No absolute perf gate or game-time mutation is introduced. Shared-machine
timings are noisy: keep scenario, files, binary profile, machine and load fixed,
inspect samples, and rerun a suspected regression before filing it. Repeated
main comparisons give Q10 a per-main timing trend in local JSON artifacts.

Bisect resolves the good and bad endpoints and walks only the bad endpoint's
first-parent history; good must be an earlier commit on that chain. It probes
both endpoints first, then cached midpoints, and records every verdict in
`bisect.json`, naming `first_bad` and `previous_good`. Behaviour is compared to
the good endpoint's complete recording; perf medians are compared to its fixed
baseline, not to the previous midpoint. Exit 0 means a boundary was found;
exit 2 means invalid endpoints or an execution error. Both searches assume a
monotone regression: if a change was reverted and reintroduced, or timing noise
changes the predicate, a binary search does not establish the earliest event
across all history. Inspect the reported adjacent boundary and retained runs.
