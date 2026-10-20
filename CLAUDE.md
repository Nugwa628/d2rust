# CLAUDE.md — Project instructions for Claude Code

Read this file at the start of every session. Detailed docs live in `docs/`;
before any architectural change, read `docs/ARCHITECTURE.md`.

## What this project is

**D2RUST**: Diablo II: Lord of Destruction rebuilt in Rust, our own game
(beta 0.1 and later: single player now, multiplayer and new features
next). The player brings their own original D2 LoD files; D2RUST reads
them at run time. This repository is the game's home: features, fixes and
releases happen here.

The launcher ("D2RUST", repo `WerdeGames/D2RUST-Launcher`) is versioned
separately; the only coupling is the game's command line
(`play --game-dir <dir>`) and a small `version.json`.

## D2RUST describes itself

- Code comments and docs explain **D2RUST itself**: what the code does and
  why, in game and design terms, richly. They never cite addresses in the
  original executable, spec sections (§, rN), recordings, a ledger, or
  "matches 1.14d" / "as the original does".
- Changes, fixes and mods are judged against **D2RUST's own behaviour**,
  never against the original game. D2RUST is a very accurate copy of the
  original, so when something should "feel right", it should match how
  D2RUST plays today. Mods are design choices, made on purpose.
- Older code still carries `// Spec:` headers, `§` references and address
  literals from before this rule. Don't add new ones; when you touch such
  a comment, rewrite it in D2RUST terms.

## Hard rules (never break these)

1. **Clean room: no Blizzard code here, and no Blizzard files outside
   `game-data/mpq/`.** Executables, DLLs, saves, extracted assets and full
   extracted tables never go here, in their original form or in any
   re-encoding a process could rebuild them from. The one exception is
   `game-data/mpq/`: the owner's own 11 MPQs, split into parts
   (`game-data/manifest.json`, rebuilt by `tools/assemble.py`), so sessions
   and CI have the game. The machines the owner controls (the CI server and
   BIGRIG) may hold the assembled files on their own disks, for CI's
   real-data scenario job (`tools/ci/game_files.py`) and for lanes' headless
   runs. The forge box (37.27.64.176) may hold the game data the multiplayer
   server needs, in the server's own folder, readable only by the server's
   user, never under the download folder or any web path (owner,
   2026-10-19). They never leave this private repository and those
   machines: never into `WerdeGames/D2rs`, a release, an artifact or any
   public place. Never
   `include_bytes!` / `include_str!` any game file.
   Game files always come from the player's own install at run time. Our
   own measured numbers (offsets, counts, ids, timings), like the tables
   in `tables/`, are allowed: they hold no Blizzard text or art. The
   pre-commit hook in `tools/hooks/` blocks the forbidden file kinds
   (`sh tools/hooks/install.sh`).
2. **No decompiled code.** Rust code is written in our own words and
   structure, never pasted or line-translated from decompiler output.
   Every module starts with a comment saying what part of the game it is
   and the rules it follows.
3. **License hygiene.** No third-party code is adapted. D2MOO may be read
   for behaviour and names. OpenDiablo2 / OpenD2 (GPL-3.0): read for
   understanding only, never copy code. `THIRD_PARTY_NOTICES.md` holds
   only the Blizzard trademark statement.
4. **Bevy is for the client only.** `d2-client` is a Bevy app. Game logic
   never lives in Bevy systems, components or resources. `d2-sim`,
   `d2-data`, `d2-formats`, `d2-net` and `d2-server` must not depend on
   Bevy. The client talks to the game only through `d2-client::bridge`.
5. **Determinism in `d2-sim`.** No I/O, no wall-clock time, no global or
   thread RNG, no `HashMap` iteration affecting outcomes, no floating
   point in game logic (integer / fixed-point math as the original uses).
   All randomness flows through the D2-compatible seeded RNG, in the
   original's draw order.
6. **Server-authoritative.** Even single player runs as a local in-process
   server. The client never decides outcomes (damage, drops, item stats,
   movement validity); it sends intents and renders state.
7. **Original behaviour is the default.** Changes to how the game plays go
   in data layers or behind explicit rule switches (`Ruleset::Original`
   vs `Ruleset::Mod`), never as edits to the original-behaviour code
   paths. Mod data is expressed as patches applied to the player's own
   tables at load time; full modified Blizzard tables never go in the repo
   or a release.
8. **The file check confirms the player has the game.** It never compares
   an install against fixed hashes and never scans beyond the game folder.

## Repository layout

```
CLAUDE.md
docs/                 architecture, clean room, required files, playtest, local run
tables/               small number tables the game logic embeds (include_str!)
crates/
  d2-formats/         MPQ, DC6, DCC, DT1, DS1, COF, palettes, string tables
  d2-data/            typed .txt/.bin tables + mod patch layers
  d2-sim/             deterministic game logic (no I/O, no rendering, no Bevy)
  d2-proto/           client<->server message types (shared, versioned)
  d2-net/             transport
  d2-server/          authoritative server, character storage
  d2-client/          Bevy app: rendering, input, UI, audio, bridge (the game exe)
  d2-native/          native-asset conversion support
  d2-verify/          "has the game" check on the player's install
  test-fixtures/      synthetic test data
tools/                d2-convert, mpq-tool, d2s-tool, data-tool, depcheck,
                      autoplay, perf, soak, git hooks, coord (forge threads, pre_pr),
                      Windows play notes
fuzz/                 cargo-fuzz targets
game-data/            the game's MPQs in parts + manifest (private; tools/assemble.py)
game/                 a local original D2 install (gitignored, never committed)
```

## Pinned versions

- Rust: stable, edition 2021; exact toolchain in `rust-toolchain.toml`.
- Bevy: **pinned to one version** in the workspace `Cargo.toml`. Do not
  upgrade Bevy as part of a feature task; upgrades are their own task and
  touch only `d2-client`.
- Game files: the player's retail LoD **1.14d** install.

## Conventions

- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets
  -- -D warnings` and `cargo test --workspace` must pass (CI runs them,
  plus a Windows release build of `d2-client`).
- `thiserror` in libraries, `anyhow` in binaries and tools.
- Binary parsing: explicit little-endian reads; no `unsafe` transmutes of
  file data.
- Tests that need game files are `#[ignore]` and read `D2_GAME_DIR`; CI
  runs without game files.
- Follow D2MOO / community names for structs and fields.
- Every persisted format (saves, protocol, caches, settings) has a version
  number from its first commit.
- Comments state what the code does and the game rule it follows, in plain
  words; no history or session notes in code.
- Disk is limited on developer PCs: build only the crates you need; build
  `d2-client` (Bevy) only for client work.

## Workflow (sessions and lanes)

Sessions coordinate through `coord` threads on the forge
(`sh tools/coord/forge.sh`, repo `WerdeGames/D2RUST`); the SessionStart hook
(`.claude/settings.json`) prints the open threads. Lane prompts follow
`docs/LANE-BRIEF.md`; working methods are in `docs/METHODS.md` and
`docs/T3CODE.md` (checked by `python3 tools/methods.py check`). Run
`sh tools/coord/pre_pr.sh [crate ...]` before every push. These were
carried over from the earlier project and are being adapted here: where
they mention the ledger, recordings or 1.14d matching, that part does not
apply in this repository.

## How to work a task

1. Read the code you're changing and how D2RUST behaves today.
2. Implement the change in the right crate (layering rules above;
   `tools/depcheck` enforces them).
3. Add unit tests for the rules the change adds or relies on.
4. Keep the workspace building and every check above green; one small PR
   per change, with what changed and how it was tested.

How work flows (roles, sessions, CI, rewriting code as features touch it)
is in `docs/WORKFLOW.md`. Features follow `docs/FEATURE-METHOD.md`: modules ranked by importance, one
self-contained PR each, built in rank order; plans live in `docs/plans/`.
