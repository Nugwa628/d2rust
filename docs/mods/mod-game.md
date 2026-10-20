# Making a mod game

A mod for D2RUST is **its own game**: its own binary, built on the D2RUST crates, shipped as its own files and
started directly by the player. The D2RUST exe never loads or selects a mod; it always plays D2RUST's own rules on
the `base/` layer of the player's files. The working example is `examples/mod-game/` (synthetic, never shipped).

## Workspace layout

A mod game lives in its own repository with a Cargo workspace:

```
my-mod/
  Cargo.toml          workspace; depends on the D2RUST crates (git or path)
  game/               the mod game's binary (like examples/mod-game)
  crates/             the mod's own rule crates (its ModRules, providers)
  layers/             the mod's layer folders: mod.toml + files/ (patches, art)
```

Depend on:
- `d2-sim`: `rules::{Ruleset, ModRules}` and the provider traits of the seams the mod overrides;
- `d2-client`: `app::single_player::{GameBuilder, GameSource, Layers, Saves, CharacterCodec}` and the app;
- `d2-server`: the server and session layer, shared by D2RUST and every mod game.

## Starting a game

1. **Data and layers.** Open the player's files with the mod's layers:
   `GameData::open(&GameSource::Native(dir, Layers::List(vec!["my-mod".into()])))`. Each layer is a folder under
   the native folder's `mods/` with a `mod.toml` and a `files/` tree; table changes are patches applied to the
   player's own tables, never copies of them.
2. **Rules.** Implement `ModRules` with the seams the mod changes (every method defaults to D2RUST's rules), and
   pass it as `Ruleset::Mod(Arc::new(MyRules))`. The ruleset is set when the game is built and never changes.
   Overrides stay deterministic: no I/O, no clock, no floats in game state, randomness only from the game's RNG.
3. **Saves.** Give the game its own folder, `Saves::new(folder, codec)`. `D2sCodec` writes D2RUST's `.d2s`; a mod
   whose characters hold more implements `CharacterCodec` with its own format. D2RUST's saves are never extended.
4. **Build.** `GameBuilder::new(data).rules(rules).saves(saves)`, then `build()` or `start(clock)`.

The example's binary does all four and runs a few headless frames:

```sh
cargo run -p mod-game-example -- --native <native folder> --layer <layer> --saves <folder>
```

## Where overrides go

The seams (override points) are listed in `docs/plans/mods/seams.md`. When a mod needs to change something that has
no seam yet, the seam is added to D2RUST there; mod games don't fork D2RUST's code.

## Multiplayer

D2RUST and the mod games share one server and session layer and one protocol. A mod game's own messages are to
travel in the protocol's mod message envelope, and its server is D2RUST's server built with the mod game's
`Ruleset` (seams MS13 and MS15, not built yet).

## Animation frame packs

Mods generate animation packs in their own build pipeline with the shared [animation generator](animation-generation.md), then install validated [frame packs](animation-frame-pack.md) through [pack playback](animation-playback.md). The base-game runtime generator is disabled in shared mod wiring. Mod source art and generated packs stay in the mod repository.
