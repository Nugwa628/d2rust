# Content sweep

`D2RUST-Game sweep FAMILY --jobs 4 --out DIR` generates standalone scenarios
from the selected game's loaded tables and runs fresh games on Q1's warm workers.
Each case runs Q2's always-on rules. No window or physical input is used.

Families are `skills`, `npcs`, `areas`, `items`, `panels`, `anims`, `doors`, and `all`.
`--filter TEXT` selects case identities by substring. `--game-dir DIR` selects
archives (otherwise `D2_GAME_DIR`). `--native DIR --mod NAME` selects the converted
base plus exactly that mod, ignoring the order file; omit `--mod` for base alone.
Generate and execute separately for each mod so both use that mod's own tables.

The output directory contains generated `scenarios/*.scn`, numbered repro artifacts
under `runs/`, `runs/batch.json`, and `sweep.json` (`sweep 1`). The console prints a
family/case/verdict/first-violation table. Reports include source skips by explicit
reason, filtered case counts, jobs, elapsed milliseconds, per-case details, and
failures grouped by stable signatures. An empty selection succeeds with zero cases;
check case and skip counts when assessing coverage. A sweep is bounded by its
implemented setup rules, not a claim that every possible game state was reached.

`--save-failures DIR` writes each failed case as a standalone `.scn`. Run it with
`repro FILE`; for mod data, use the same selected data when replaying. Findings
are **Unverified** candidates until the owner approves a fix. Scenarios and reports
may describe cases; game data and images remain on the local machine.

Every generated scenario states `tick_rate 60`. At its final checkpoint,
`at T checkpoint` reads the live save state, serializes and parses a save, joins
an independent game from those bytes, and compares its live saved state and its
second save using the soak round-trip rules. Expected load normalization and
regeneration follow those existing rules. The running scenario is never replaced
or advanced by this comparison. Checkpoint failures have `roundtrip:*` signatures
and remain in exported scenarios. Multiple checkpoints may be authored in a repro.

Coverage follows explicit setup rules:

- Skills use loaded class, level, animation, target and equipment requirements.
  Passive skills, auras, special corpse/item/pet/ally targets and unavailable
  equipment are counted skips. Cast shapes include monster, ground and self,
  except unit-only skills.
- Town NPCs use loaded enabled interaction rows and the client menu definitions.
  Cases cover talk/close and available trade, gamble, identify, repair and hire
  options. Quest-gated travel/imbue and resurrection without a dead hireling are
  counted skips. Hire offers come from the selected game's hireling rows.
- Areas cover playable generators, short walking routes, explicit interaction
  tiles for level links, waypoint destinations and death/respawn in each act.
  Seam links without an interaction tile and missing generators are counted skips.
- Items cover cursor/inventory/drop, legal equipment, stash, belt consumption,
  repair, socket insertion, vendor purchase/sale and simple cube recipes. Recipes
  requiring special conditions, difficulty, ladder state, non-item outputs or
  inputs that cannot fit the cube are counted skips. Socket setup searches a
  bounded seed range through the loaded item creation rules. Equipment cases
  identify the item first (an unidentified item cannot be worn), quest-gated
  pickups set their quest bit with `dev quest`, vendor purchases click
  `shop.item(0)` (the first stocked item of the visible page), and rows whose
  ground art is in no archive are a counted skip.
- Panels traverse the reachable combinations of headless toggle actions using
  the client's gate/conflict policy, then close them and verify the flags.
  Context panels are exercised by the NPC/item/waypoint cases; states without a
  headless toggle are counted separately. The pause menu uses paced Escape input
  so it can resume while the server clock is paused.
  bounded seed range through the loaded item creation rules.
- Panels traverse the reachable combinations of single-player headless toggle
  actions using the client's gate/conflict policy, then close them and verify the
  flags. The Party screen is multiplayer only, so its state is a counted skip
  here. Context panels are exercised by the NPC/item/waypoint cases; states
  without a headless toggle are counted separately. The pause menu uses paced
  Escape input so it can resume while the server clock is paused.
- Anims check the animation playback rate: a unit's animation is drawn at
  `speed / 256` frames per base tick of 25 Hz (the speed the server holds for
  the unit's mode), whatever the logic tick rate. Every class gets town idle,
  town walk, field idle, walk, run (the left button held, as a player moves),
  attack (a click on a standing target) and its first level-1 cast skill;
  every spawnable monster is spawned near the player and checked in whatever
  mode it is drawn in; each act's town checks its first NPC in view. Each
  case is `by T check anim_rate(TYPE, CLASS, MODE).ratio ~ 1 0.15`: the
  frames the world view drew per second (`world_view::anim_clock`) against
  the table rate, over at least a third of a second of the mode. A failure
  shows the measured ratio (2.4 is a per-tick advance meant for 25 Hz run at
  60 Hz). A one-frame animation reads ratio 1 (it draws the same picture at
  any rate), and the drawn rate counts frames the nearer way round, so a
  sequence played backwards (a tentacle head's submerge) reads its own rate.
  Monsters that are not enabled, are NPCs or cannot be killed are a counted
  skip, as is a class without a level-1 cast skill. So are monsters the
  Act I spawn cannot show: a class without neutral art
  (`anims:monster-no-neutral-art`: bird2, fish), water-only placement
  (`anims:monster-water-spawn`: `spawnCol` 1, the frog demons) and a Baal
  clone, which dies without its owner (`anims:monster-owner-only`).
- Doors open on a click and let the player through, both ways. Every door
  class of every level in the door table the map builder places doors from
  gets a case: the player is brought in front of the opening of the lowest
  closed door of the class, on the positive side of its narrow axis (the
  smaller side of its footprint) (`poke goto preset LEVEL 2:CLASS mode 0
  opening`: a class with `PreOperate` spawns open on a 1-in-14 draw, which
  is not a failure). The door must be closed with the player on that side,
  open on a click (`near(object, CLASS).mode_id == 2`), and a ground click
  11 sub-tiles from the player across the door, then 11 back, must bring
  the player through each way (`near(object, CLASS).ay` or `.ax` at most
  -2, then at least 2). Each result is an `input wait-until`, so a late
  step moves the later ones with it. The clicks are measured from the
  player because a long door's position need not lie on the way through
  (slime door 230), and they reach past the open door's sprite, which
  would take the click. Every `IsDoor` object belongs to Acts I and II;
  the Act V rows of the table are barricades (monsters, left out), the
  town gate and a portal (counted skips), as are levels outside the
  playable acts.

The synthetic integration tests register in the existing `app` target. They run
small slices of every family, prove a mod changes generated level requirements
and is used during execution, and compare checkpoint/no-checkpoint state bytes.
A deliberately unrepresentable save stat verifies checkpoint failures are reported.
