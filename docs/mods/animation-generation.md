# Animation frame bake

Generated in-between animation frames are made once per build on our side and published as files with that build; the launcher downloads them. The game never generates frames while it runs. A local bake remains for mods and as the fallback when a download is missing. The frame pack format is shared by published bakes and mod build outputs; see [the format](animation-frame-pack.md). The repository contains generator code and synthetic tests only. Source art and any pictures derived from it remain outside this repository.

## The bake

The launcher downloads the build's bake folder (the channel's `anims` list, launcher `docs/updates.md`). When a build has no list, or the download fails, it runs the installed game's bake instead, before PLAY is offered:

```text
D2RUST-Game bake-anims --game-dir DIR [--out DIR] [--reuse DIR] [--jobs N]
D2RUST-Game bake-anims --native DIR [--mod NAME]... [--out DIR] [--reuse DIR] [--jobs N]
```

One work item is one player or monster component file: every file under `data\global\chars` and `data\global\monsters` whose name parses as unit token, component, armor class, mode and weapon class. That covers every class, mode, weapon class and equipment variant, and every monster class and mode, across all source facings of the file. Animations named in `tables/render/animation-deny.tsv` are skipped. Objects, missiles and effect overlays keep their source frames.

Each file's pack is `packs/<identity>.d2fz`: one frame pack v2 as a deflate stream. A v2 pack stores one motion vector and one accepted-fraction mask per source frame pair, not pixels; the folder's `blend.lut` turns the pair of source palette indices into the output index at load. The identity is the SHA-256 of the source pictures, palette, colour-map ramps and generator settings (not the archive name, so identical files share one pack). Packs are quantized to act 1's presented palette; unit pictures are palette indices drawn with the act palette in use, so a generated picture follows the act palette the same way its source frames do. `bake.json` lists every pack with generated frames, its component file and key, and the `blend.lut` hash. It is written last, through a temporary file, so a bake that stops early has no index. A full base-game bake with real data measured 5.0 MB of packs plus a 0.20 MB `blend.lut` (the manifest adds 7.4 MB), against 908 MB for the stored-pixels format.

A rerun is cheap. With a finished `bake.json` of the same game version, generator and data stamp (archive names, sizes and times; for a native folder its manifest and the mod layers' files), the bake returns at once. Without one, existing packs whose identity still matches are kept, not generated again, so a stopped bake resumes. `--reuse DIR` copies a pack of the same identity from another bake folder, so a mod bake does not regenerate what the base game's bake already made. Without `--out` the bake writes to the play default (below).

The bake prints one line per step on stdout for the launcher: `bake-anims: total N`, `bake-anims: progress DONE TOTAL` (DONE only grows), `bake-anims: done PACKS BYTES` on success (exit code 0) or `bake-anims: error MESSAGE` (exit code 1).

## Generation

The CPU generator estimates bounded translation, interpolates in linear sRGB, and quantizes to the palette. The translation search is two-stage: a 5x5 grid with step 4 over the +/-8 range, then a 5x5 refinement around the winner. It evaluates about 50 offsets instead of the exhaustive 289, which brought the full bake from 656 s to 25 s on four CI cores; against the exhaustive search it changes 2.7% of accepted in-between frames on a 577-pack sample (0.005% of pixels differ by more than 64 RGB levels). One item-palette file is one colour family, so its map signatures partition the mapped indices into families rather than single shades. Interpolation stays within the starting family, and two families whose colours overlap within the shade tolerance may blend; visibly different families keep their source pixels. Mask changes, motion disagreement, incompatible ramps and excessive palette error reject a candidate, which keeps its source picture.

Generated samples cover fractions 64, 128 and 192 of each source interval in 1/256 units. Playback holds each sample until the next quartile. Simulation frames, events and COF ordering retain the starting source frame.

In-between facings (between a file's own facings) are not baked yet; a unit facing between two file facings draws the samples of the file facing its source picture uses.

## Play

`play --anim-dir DIR` is the dev/test override for the bake folder. The launcher sets `$D2_ANIM_DIR` to `<install>\anims\<channel>\<version>` when that folder has a finished `bake.json`, and clears it otherwise; without either, play uses `<user folder>/cache/animation/baked`. A loader thread reads the pack of each component file a drawn player or monster uses, checks its identity against that source file, and rebuilds the accepted in-between pictures from the source cel and the folder's `blend.lut`. The view then makes the pictures resident; installing a pack costs the pack's size, never a copy of the resident pictures. A missing index, a missing `blend.lut` or pack, or a pack made from other source pictures plays the source frames, and the game logs it once. Nothing shows on screen while packs load.

The Video options page has an **Animation frames** row: **Original** draws source frames only and loads no packs; **Smooth** draws baked in-between frames where a pack is resident and holds the source frame everywhere else. The choice is saved with the other video settings (default **Smooth**) and applies at once. Animations named in `tables/render/animation-deny.tsv` always use source frames: each row names a unit kind (`player`, `monster`, `object`, `missile` or `overlay`), a unit token, a mode (animation) token and the reason; tokens match in any case. In play, the dev console command `animation [original|smooth]` toggles or picks a mode on the running client without changing the saved setting.

## Mods

Mod games are downloaded and baked the same way: the mod channel carries its own `anims` list, and a missing list or failed download falls back to a local bake in the mod folder, reusing the base game's packs. A mod's own pipeline can also build packs from a source pack with the same generator:

```text
D2RUST-Game animation-build --source-pack source.d2frames --palette palette.dat --ramps ramps.dat --out build/animation [--loop]
```

The input uses frame pack v1 with source frames at sub-frame zero, contiguous frame numbers starting at zero per component and facing. Palette input is 768 BGR bytes; ramps are 256 labels (equal labels share a ramp, zero is unmapped). `--loop` blends the final frame into the first; without it, the last frame holds. The output contains the generated pack and reviews. Mod data and generated mod packs stay in the mod's own pipeline.

The [local proof command](animation-proof.md) generates one class's walk/run comparison and a local-only 60 Hz viewer for the owner's look check.
