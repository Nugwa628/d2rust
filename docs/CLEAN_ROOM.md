# Clean room

D2RUST is our own Rust code. It runs on the player's own copy of the game's
data files and contains none of Blizzard's code, assets or text. This is a
process note, not legal advice.

## Code

- All code is written by us, in our own words and structure.
- Decompiler output, assembly and near-verbatim C are never pasted or
  line-translated into this repository, in code or in comments.
- No third-party code is adapted (see `THIRD_PARTY_NOTICES.md`).
- How the original behaviour was worked out (the reverse-engineering record)
  is kept outside this repository. Code and docs here describe what D2RUST
  does and why, in game terms.

## Data

- `tables/` holds small tables of numbers the game logic uses (ids, counts,
  frame and layout values, function indices). They are our own measured
  facts, not Blizzard files: no strings, no art, nothing a process could
  rebuild a game file from.
- Everything else (MPQs, executables, saves, extracted assets, full `.txt`
  or `.bin` tables) comes from the player's own install at run time and is
  never committed or shipped, in any form.
- Mods are expressed as patches applied to the player's own tables at load
  time.

## Release

- A release contains our code, our own assets, and nothing from the game.
