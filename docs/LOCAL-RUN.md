# Local run (Windows PC with the game installed)

Everything that needs the player's game files, a real GPU or a display.
CI runs none of this: it has no game files.

Shell: PowerShell on Windows 10/11, repo root as the working directory.
Disk is tight: build only what a step names; only the client steps build
Bevy (`d2-client`).

## Setup

```powershell
$env:D2_GAME_DIR = "C:\path\to\Diablo II"   # the install (MPQs), not a copy of the repo
cargo install cargo-nextest --locked        # optional, faster test runs
```

`D2_GAME_DIR` is read by every `#[ignore]` game-file test, `data-tool`,
`mpq-tool` and `d2-client play`. Set it in each new shell.

## 1. Tables and formats (fast, no Bevy)

| Command | Expect |
|---|---|
| `cargo run --release -p data-tool -- tables` | every runtime table loads; 0 mismatched |
| `cargo run --release -p data-tool -- links` | no broken link between tables |
| `cargo run --release -p mpq-tool -- check` | every archive block decodes |
| `cargo run --release -p mpq-tool -- formats` | 0 errors in every file kind |

## 2. Game-file tests (`--ignored`)

```powershell
cargo test --release -p d2-formats -p d2-data -- --ignored
cargo test --release -p d2-sim -- --ignored
cargo test --release -p d2-server -- --ignored --test-threads 1
cargo test --release -p d2-client -- --ignored --test-threads 1   # builds Bevy
```

A failing test names its assertion. `game_wired_host` prints one digest
line per class: run it twice, the digests must be equal (determinism).

Saves: `$env:D2_SAVE_DIR = "<folder with .d2s files>"`, then
`cargo test --release -p d2s-tool --test real_saves -- --ignored` (every
save parses and rewrites byte for byte).

## 3. The client window (real GPU, builds Bevy)

| Command | Expect |
|---|---|
| `cargo run -p d2-client --release -- play --synthetic --frames 1500` | black 800×600 view, about 25 server ticks per second, `gpu: true`, exit 0 |
| `cargo run -p d2-client --release -- play --frames 1500` | the game data line (levels, objects, level files), then a running game; exit 0 |
| `cargo run -p d2-client --release -- play` | the full game: menu, character, play (see `docs/PLAYTEST.md`) |
| `cargo run --release -p d2-client -- verify` | every render case passes (GPU matches the CPU reference) |

Record the adapter line (`AdapterInfo { … }`) with any render result.

## What to send back

The command, its exit code, the last 30 lines of output, the adapter
line for GPU steps, and for a crash the crash log (see `docs/PLAYTEST.md`).
