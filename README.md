# D2RUST

D2RUST is a rewrite of Diablo II: Lord of Destruction in Rust. It plays like the original game and modernizes the
engine underneath: a 60 Hz game loop, 16:9 and high resolutions, smooth animation, and one server design for both
single player and online play.

**You need your own copy of Diablo II: Lord of Destruction.** This repository has no game files. D2RUST reads the
data archives (`d2data.mpq`, `d2exp.mpq`, ...) from your own install when it runs. See
[docs/REQUIRED_FILES.md](docs/REQUIRED_FILES.md).

> Status: early beta (0.1.1). Expect bugs.

## Features

- **The whole game:** five acts, every class, skills, items, quests, waypoints, towns, vendors, gambling and the
  Horadric Cube, all built from the original game data tables.
- **60 Hz game loop:** game logic runs 60 ticks per second instead of 25. Speeds and durations are converted from
  the game's own tables, so the game plays the same but responds faster and has finer attack-speed steps.
- **Smooth animation:** extra in-between animation frames are made from your game files once, at install, and
  then stored on disk. You can switch back to the original frames in Video options.
- **Modern display:** a 16:9 view (1068x600) by default, plus 4:3. Sharp scaling to 1080p, 1440p and 4K. Windowed,
  borderless and fullscreen modes, VSync and a frame limit.
- **One server for everything:** single player runs the same game server inside the client, so offline and online
  use the same code. The server simulation is deterministic: the same inputs always give the same game.
- **Your characters:** saved in D2RUST's own format under `Documents\D2RUST\saves`. Diablo II `.d2s` saves can be
  imported.
- **Mods as their own games:** a mod runs on D2RUST as its own game, with its own files and saves. See
  [docs/mods](docs/mods) and [examples/mod-game](examples/mod-game).
- **Dev tools:** an in-game dev panel and console (press `` ` ``): god mode, levels, items, warps and more. They are
  on by default during the beta. Turn them off with `--no-dev`.

## Playing

1. Build the game (below) or get a release build.
2. Put `D2RUST-Game.exe` in your Diablo II folder (the folder that has `d2data.mpq`), or point it at the folder:
   ```powershell
   D2RUST-Game.exe --game-dir "C:\Program Files (x86)\Diablo II"
   ```
   You can also set the `D2_GAME_DIR` environment variable instead.
3. Start it. The main menu opens.

Shortcuts:

```powershell
D2RUST-Game.exe --new sorceress MyName          # straight into a new character
D2RUST-Game.exe --save "path\to\MyName.d2s"     # load (or import) a save
D2RUST-Game.exe --difficulty nightmare ...      # normal / nightmare / hell
```

Key bindings are in `Documents\D2RUST\controls.toml`. They start as the original game's default keys.

## Building from source

You need [Rust](https://rustup.rs). `rust-toolchain.toml` picks the right version automatically.

```powershell
git clone https://github.com/Nugwa628/d2rust
cd d2rust
$env:D2_GAME_DIR = "C:\path\to\Diablo II"
cargo run --release -p d2-client -- play
```

Windows is the main platform. The game client uses [Bevy](https://bevyengine.org) and needs a GPU with Vulkan,
DirectX 12 or Metal. The server and the tools also build on Linux.

Tests:

```powershell
cargo test --workspace                       # everything that needs no game files
cargo test --release -p d2-sim -- --ignored  # tests that read your game files (D2_GAME_DIR)
```

[docs/LOCAL-RUN.md](docs/LOCAL-RUN.md) lists every local check that needs the game files or a GPU.

## How it fits together

```
your Diablo II files ──► d2-formats ──► d2-data ──┬──► d2-sim (60 Hz, deterministic) ◄── d2-server
                         (MPQ, DC6,      (tables)  │                                       ▲
                          DCC, COF ...)            └──► d2-client (Bevy: draw, UI, audio) ─┘ d2-proto / d2-net
```

| Crate | What it does |
|---|---|
| `d2-formats` | Reads the game's file formats: MPQ archives, tables, sprites, palettes, maps, sounds |
| `d2-data` | Turns the tables into typed game data, with mod patch layers on top |
| `d2-sim` | The game rules: units, combat, skills, items, AI, quests, levels. Deterministic, headless |
| `d2-server` | Runs the simulation. Owns accounts, characters and games |
| `d2-proto`, `d2-net`, `d2-quic` | The messages between client and server, and how they travel |
| `d2-online` | The online client session: connect, account, lobby and the game stream |
| `d2rust-server` | The dedicated online server: runs games with no window, with characters kept on the server |
| `d2-client` | The game window: drawing, UI, sound and input (Bevy) |
| `d2-verify` | Checks that the game files are present |
| `tools/` | Developer tools: `mpq-tool`, `data-tool`, `d2s-tool`, `img-diff` and others |

More detail: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Documentation

- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md): the crates and the rules between them
- [docs/REQUIRED_FILES.md](docs/REQUIRED_FILES.md): which game files D2RUST reads
- [docs/LOCAL-RUN.md](docs/LOCAL-RUN.md): building and checking on a PC with the game installed
- [docs/PLAYTEST.md](docs/PLAYTEST.md): how to play-test and report a bug
- [docs/WORKFLOW.md](docs/WORKFLOW.md): how the project is developed
- [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md): third-party licenses

## Legal

D2RUST is a fan project. It is not affiliated with, endorsed by or connected to Blizzard Entertainment. Diablo is a
trademark of Blizzard Entertainment, Inc. This repository contains no Blizzard game files, art, sound or other
content. To play, you need your own legally bought copy of Diablo II: Lord of Destruction.

The D2RUST source code is released under the [MIT License](LICENSE).
