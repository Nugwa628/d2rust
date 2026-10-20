# Required files

D2RUST needs the player's own, legally owned Diablo II: Lord of Destruction
install. Nothing from it is ever committed to git. For local
development, put a copy in `game/` (gitignored) or point `D2_GAME_DIR` at
it; tests that need it are `#[ignore]` and read `D2_GAME_DIR`.

## Data archives

- `d2data.mpq`: core classic data
- `d2exp.mpq`: Lord of Destruction data
- `d2char.mpq`: character graphics
- `d2sfx.mpq`, `d2speech.mpq`, `d2xtalk.mpq`: sound and speech
- `d2music.mpq`, `d2xmusic.mpq`: music
- `d2video.mpq`, `d2xvideo.mpq`: not used; D2RUST has no cinematics
- `patch_d2.mpq`: patch data

These total about 2 GB and stay on the player's machine. D2RUST reads them
at run time; `d2-verify` checks that they are present (it confirms the
player has the game, it never compares against fixed hashes).

## Saves

Characters are stored in D2RUST's own versioned format. Diablo II `.d2s`
saves can be imported (`--save`, `tools/d2s-tool`).
