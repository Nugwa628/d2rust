# Local walk/run proof

`D2RUST-Game animation-proof --out E:/work/artifacts/cx-s7s8/proof/<new-run> [--game-dir LOCAL_INSTALL] [--class 0..6]` generates one class's walk and run cycles without a window. The default class is Sorceress (1); `D2_GAME_DIR` supplies the install directory when `--game-dir` is absent.

The tool uses the shared generator with the install's act 1 palette and colour-map ramps, then composes unlit body components at fixed feet. It preserves the starting COF layer order, covers the 16 source facings, and writes side-by-side PNG sequences at 60 Hz: Original left, Smooth right. Missing and rejected samples retain their source picture. This isolated comparison omits a scene, equipment variants, lighting and gameplay movement; broader coverage follows the owner's look check.

Open `index.html` in the chosen directory locally to select walk/run and facing, play/pause at 60 Hz, or step through frames. `report.json` records accepted samples, rejects by reason, cold generation time, cache-hit flags and pixel differences between panels. The `cache/walk` and `cache/run` directories hold packs plus structured and text review sheets.

Before the ramp-family fix, the first local run (2026-10-19, Sorceress walk/run) produced 5,376 candidates: 27 accepted and 5,349 rejected (RampConflict 4,743, MotionMismatch 345, ShapeChange 249, EmptySource 12). Six of 640 comparison shots changed; every body component was rejected. The map-level partition gave 139 mostly single-index ramps, so ordinary shade changes tripped the 2% conflict gate.

After grouping by item-palette file (one colour family per file) and treating overlapping shades from different families as compatible, the same walk/run produced 3,837 accepted and 1,539 rejected (RampConflict 933, MotionMismatch 345, ShapeChange 249, EmptySource 12). Walk accepted 2,277 of 2,688 samples and run 1,560; the head was fully accepted in both cycles, the torso 82% in walk and 48% in run, the arms 32-91%, and the legs 65% in walk and 5% in run. 480 of 640 comparison shots changed, by 154,591 pixels. Cold generation took 36.53 s. The remaining rejects are motion and shape changes, plus ramp conflicts where visibly different families meet.

All source art, generated packs and proof images remain local. They must never be committed, uploaded, shipped or attached to a PR. Only the tool, synthetic tests and documentation belong in this repository. S7f-S7h and S8 wait for the owner's local look check.
