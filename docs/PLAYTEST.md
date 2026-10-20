# Playtest on Windows (no Rust build)

The Windows build workflow (`.github/workflows/windows-build.yml`, run
for version tags or on demand) builds `D2RUST-Game.exe`. The download holds
only our compiled code; the game reads **your own Diablo II 1.14d install**
(`CLAUDE.md` hard rule 1). Players normally get it through the D2RUST
launcher's installer; this page is for testing a build directly.

## 1. Download

1. Open the repository's **Actions** tab on the forge, workflow
   **Windows build** (to build a branch on demand: **Run workflow**).
2. Click the newest green run.
3. Under **Artifacts**, download `D2RUST-Game-<version>-<commit>` (a zip,
   kept 30 days).

The zip holds `D2RUST-Game.exe`, `version.json` (`{"version":"0.1.1.<build>",
"exe":"D2RUST-Game.exe"}`, read by the launcher) and `README-PLAY.txt`.
The version is 0.1.1 plus the build number; the exe's Properties →
Details show the same.

## 2. Where to put it

The client looks for the game, in this order:

1. `--game-dir <folder>` on the command line;
2. the `D2_GAME_DIR` environment variable;
3. the folder of `D2RUST-Game.exe`, then the current folder, if it holds
   `d2data.mpq`.

Simplest: unzip `D2RUST-Game.exe` into your Diablo II
folder (the one with `Game.exe` and `d2data.mpq`) and double-click
`D2RUST-Game.exe`. It never writes into that folder: saves, settings and
logs go in `Documents\D2RUST`. The exe opens no console window; a fatal
error shows a message box naming the reason and the crash log.

Elsewhere: `D2RUST-Game.exe --game-dir "C:\Program Files (x86)\Diablo II"`.

A console window opens next to the game window; it shows the log
(`game folder: ...`, `play: ...`). On an error it stays open until Enter.

## 3. Start a character or load a save

Double-click starts at the main menu: **Single Player** opens character
select (saves in the save folder) and create.

From a command prompt (skips the menu):

| Command | What |
|---|---|
| `D2RUST-Game.exe --new sorceress MyName` | new character (amazon, sorceress, necromancer, paladin, barbarian, druid, assassin; name 1–15 of A–Z a–z 0–9 `-` `_`) |
| `D2RUST-Game.exe --save "C:\...\MyName.d2s"` | load a save |
| `--difficulty normal\|nightmare\|hell` | difficulty |
| `--res WxH` | the frame: `640x480`, `800x600` or `1068x600` (default: the Video page's View row, 16:9 Widescreen) |
| `--seed N` | a fixed game seed (to replay a bug) |

Everything the game writes goes in your D2RUST folder,
`Documents\D2RUST`, never into the game folder: characters in `saves\`
(`--save-dir` to change), `settings.toml` and `controls.toml`. On the first
start, an existing `Documents\d2rs` folder from an earlier build is moved
there, so your characters carry over. Original 1.14d `.d2s` files can be
loaded with `--save`; copy them first, since the game writes back to the
file it loaded.

## 4. Keys

The default key configuration: left click walk / attack / interact, right click the right skill,
`Ctrl` held run, `R` run / walk toggle, `Shift` stand still, `Alt` show
items, `A` / `C` character, `I` / `B` inventory, `T` skill tree, `S`
skill choice, `Q` quests, `M` message log, `H` help, `O` hireling,
`W` swap weapons, `Tab` automap (`F9`–`F12` its options), `V` minimap,
`F1`–`F8` skill hotkeys, `1`–`4` belt, `` ` `` show belt, `Space` clear
screen, `Esc` game menu (Options, Save and Exit). Rebind in the menu's Configure Controls,
or edit `Documents\D2RUST\controls.toml`. `Alt+Enter` (in game and on the main menu) switches
the window between its mode (Windowed or Fullscreen) and Windowed Borderless.

Anything that plays differently from what you expect is worth reporting.

## 5. Dev tools (D2RUST's own)

Built-in tools to get anywhere fast while testing; they are not part of
Diablo II. They are **on by default** in
this alpha. Turn them off with `--no-dev` or the environment variable
`D2RUST_DEV=0`. Check runs (`--frames`, `--dump-draws`, `--input`,
`--poke`, `--send`, `--no-sound`, ...) never have them.

Press `` ` `` (backquote, left of `1`) to open or close the dev panel;
`Esc` also closes it. Click its buttons: god / heal / no cooldown / free
mana, speed presets, level +1 / +10 / set, gold, stat and skill points,
unlock waypoints and acts, identify, kill, town 1–5, a warp list per act,
and a monster / item picker (search, quality, item level, count, to
inventory or ground). While it is open the game gets no keys and no
clicks (the game's `` ` `` show-belt key is taken by the panel). The
console at the bottom left takes typed commands: `Enter` runs one,
`Up` / `Down` walk the history, `Tab` completes a command name.
Every command runs on the local server between two ticks.

| Command | What |
|---|---|
| `help [command]` | list the commands / one command's usage |
| `god [on\|off]` | no death; life, mana, stamina refilled every tick |
| `heal` | life, mana, stamina to maximum |
| `level <1..99>` / `level +<n>` | raise the character to that level / n levels (stat and skill points as on level-up) |
| `gold <n>` | add gold to the inventory (capped at level × 10000) |
| `statpts <n>` / `skillpts <n>` | add unspent stat / skill points |
| `town <1..5>` | warp to an act's town |
| `warp <level id or name>` | warp to any level, e.g. `warp 35`, `warp catacombs level 2` |
| `waypoints` | know every waypoint in every difficulty |
| `acts` | open acts 2–5 (quest flags, every difficulty); then travel by waypoint |
| `item <code or name> [quality] [ilvl <n>] [ground]` | create an item, e.g. `item hax magic ilvl 30`, `item ring unique`; quality: low normal superior magic set rare unique crafted |
| `identify` | identify every item you carry |
| `kill [screen\|level]` | kill the monsters near you (40 sub-tiles) or in the level |
| `spawn <monster id or name> [count]` | spawn monsters next to you, e.g. `spawn fallen 5` |
| `reveal` | not built yet |
| `speed <percent>` | faster walk / run (`speed 0`: off) |
| `nocd [on\|off]` | no skill cooldown |
| `freemana [on\|off]` | mana refilled every tick |
| `difficulty <normal\|nightmare\|hell>` | tells you how to restart on that difficulty |
| `info` | info overlay (area, position, tick, FPS, hovered unit) |
| `bugreport [note]` | writes `Documents\D2RUST\saves\bug-reports\<time>\` as a report bundle (`report.json` with the note, plus screenshot, state and log; format in `crates/d2-report`) and prints the path: send that folder with a bug |
| `save <name>` / `load [name]` | save the character and its place to a slot / load one (the game restarts there; the world is re-generated from the seed) / list the slots |
| `clear` | clear the console |

A game where dev commands were used is not a reference for a bug about
the original's behaviour: say so in the report.

## 6. Reporting a bug

Send:

1. **The logs** in `Documents\D2RUST\logs`: `crash.log` if it crashed
   (the panic or error, a backtrace, the command line and the last steps:
   game folder, menu, game start with seed and save), and `game.log`
   (everything the game printed in that run; `game.prev.log` is the run
   before). Both are overwritten by later runs: copy them first. When
   that folder cannot be written, the crash log is `D2RUST-crash.log`
   next to `D2RUST-Game.exe`.
2. **A screenshot** of the window (and of the message box if it showed
   one).
3. **The save**: `Documents\D2RUST\saves\<name>.d2s` (or the file given to
   `--save`).
4. The version (`D2RUST-Game-<version>-<commit>`), what you did, and what
   you expected.

## 7. Version and exit codes (for the launcher)

- `D2RUST-Game.exe --version` prints one line, `D2RUST 0.1.1.<build>`, and
  exits 0 without starting anything.
- `--error-file <file>`: a run that fails writes its one-line reason
  there (the launcher shows it); the file is removed at start.
- Exit codes: `0` ended normally, `1` a fatal error, `2` a wrong command
  line, `3` the Diablo II files were not found, `101` a crash (panic;
  the crash log holds the backtrace).

## 8. What the build is

`cargo xwin build --target x86_64-pc-windows-msvc -p d2-client --profile dist`,
cross-built on the CI server (runner label `windows-cross`: cargo-xwin with the
MSVC CRT and Windows SDK, the same MSVC target as a native Windows build) with the
pinned toolchain (`rust-toolchain.toml`): release with LTO, one codegen
unit and line tables in a .pdb, which is kept by whoever builds the release and
not shipped (crash logs show addresses; that .pdb maps them to file and line). Before
upload, the job checks that the exe's version resource carries the build's version.
The Windows checks of the exe itself (`D2RUST-Game.exe crash-test`, a deliberate
panic, writes `crash.log` with a backtrace; an offscreen start-up into a new game on
the game files) run on a Windows PC on the fetched exe, since the CI server cannot
run it.
