# Game map and T-static / T-jump (G0-G3)

`D2RUST-Game map` reads any game on D2RUST (the base game or a mod) and writes a
map of it without being told anything about that game. The data comes from the
game's own tables; a short hand-written override file adds only what the tables
cannot say.

The plan is [`docs/plans/tools/game-explorer.md`](../plans/tools/game-explorer.md).
This guide is for a developer or AI session working on the tool or on a mod.

## Build the map

```sh
# The base game (archives) with the base game's overrides from this repo:
D2RUST-Game map --game-dir "C:/D2RUST/game" --out map-out

# A converted native folder and one mod on top of it:
D2RUST-Game map --native E:/work/native --mod mymod --out map-out
```

`--game-dir` defaults to `D2_GAME_DIR`. The command writes:

- `game-map.json`: objectives, world graph, playables, mechanics and counts.
- `GAME.md`: the readable summary for developers and AI. Generated; never shipped.
- `static.json`: the T-static report.

`map` is deterministic: the same tables and overrides give the same files.

The reader takes quests, levels, level links, waypoints, monsters, NPCs and
services, objects, skills, items, cube recipes, runewords and
class data. Names come from the game's string tables. The base game's override
file is `tables/world/game-map.overrides.toml` in this repo. A mod keeps its own
`game-map.overrides.toml` in the mod's files; `map --native DIR --mod NAME`
looks for `DIR/mods/NAME/game-map.overrides.toml` unless `--overrides FILE` is
given.

## Overrides

The override file is TOML. Only the keys it needs are written; everything else
is data.

```toml
game = "Display name"
final_objective = "Defeat the last boss"
towns = [1, 40]                     # safe areas (waypoint towns)

[[act_transition]]
from = 1
to = 2
quest = "A1Q6 ..."                  # optional
area = 40                           # optional

[[gate]]
from = 12
to = 13
kind = "key"                        # door, portal, key, quest

[[quest]]
chain = 6
prerequisites = [1, 2, 3, 4, 5]

[[reward]]
chain = 2
kind = "item"                       # item, respec, unlock, point
name = "Akara's ring"
item = "rin "                       # an item code in the game's tables

[[goal]]
name = "talk-akara-a1"
kind = "talk"                       # talk, boss, area
act = 1
area = 1
npc = 148                           # a monstats row
chain = 1                           # the quest whose flag must advance
max_ticks = 200
pending = "why it blocks"           # optional: a known open defect

# A level boss. Where a boss stands is in the level maps, not the tables,
# so each one is named here: a superuniques row (the class comes from the
# row) or a monster class for a boss that is the only monster of its class.
[[boss]]
area = 8
superunique = 40                    # or: monster = 156
# optional: scenario steps (as after `at T`) run before the walk to the
# boss, for a boss the game releases on an event (a seal, an altar)
setup = ["poke goto unit 2:396", "poke operate @2:396"]
from_town = true                    # optional: enter from the act's town
fight = 2100                        # optional: fight ticks (default 1470)
pending = "why it blocks"           # optional: a known open defect

# A link the tables cannot name, and the one-line reason it is there.
[[link]]
from = 3
to = 17
kind = "path"
reason = "Cold Plains to the Burial Grounds (outdoor placer branch)"

# A chain of links, each consecutive pair.
[[path]]
areas = [1, 2, 3, 4, 5, 6, 7, 26]
kind = "path"
reason = "Act I town gate, wilderness chain and Monastery Gate"

# An area the shipped game has no entrance to.
[[unreachable]]
area = 999
reason = "unused test level: no vis slot, portal or object names an entrance"
```

`LevelDefs` `Vis*` slots are the level edges; `Warp*` is the lvlwarp row used
by that slot, not a destination level. The reader therefore builds walk links
from `Vis*` only, skips level visibility that crosses an act, and takes the
missing wilderness/town/quest edges from `[[link]]` / `[[path]]`. Each
act-transition entry also adds a link from the source act's town to the
destination town or area. A link can carry a `gate` (door, key, portal) as
before.

Towns and act transitions are the pieces the data does not encode. Without
`towns`, the T-static reachability rule has no start and reports nothing for the
act. The base game's file names its five towns and quest order.

## T-static

T-static is cheap enough for the fast gate: it reads `game-map.json` only.

| Rule | Checks |
|---|---|
| `area-reachable` | every area has a link path from its act's town (or from any area of the act when no town is named); `[[unreachable]]` areas are exempt with their reason |
| `link-target` | every link names an existing area |
| `quest-step` | every quest that names a message table has an NPC message step (a scripted quest with no table needs none) |
| `reward-exists` | every item reward names an item in the map (codes compare without their space/NUL padding) |
| `item-references` | skill requirements, cube inputs and outputs, runeword runes and types, and item codes resolve; an item carries its direct type and its `Equiv1`/`Equiv2` parent types, so a sword also answers `mele` and `weap` |
| `class-skills` | every class has a skill with requirement 30 or lower |
| `world-references` | waypoints and boss areas exist |

`static.json` has one result per rule with the checked count and findings. The
command exits 1 when a rule has a finding. Findings from real data are
**Unverified** until a person confirms them.

The synthetic tiny game test in `crates/d2-client/src/app/game_map` runs every
rule on normal tests; the base game test is `#[ignore]` and needs `D2_GAME_DIR`.

## T-jump

`--t-jump` generates goals from the map and the override file:
- one `area` goal per area (load it and check the player is there);
- one `boss` goal per `[[boss]]` entry (place the player next to it and kill
  it): the goal warps to the level (from the act's town first for a boss
  marked `from_town`: an act change raises no level-change quest event,
  and Nihlathak only reaches his temple after one), runs the
  boss's `setup` steps if it has any (a setup `poke goto` gets 240 ticks,
  other steps 20, then 90 ticks for the boss to appear; the fight keeps its
  length after them), walks to the boss with `poke goto
  superunique ROW` (or `goto unit 1:CLASS`), holds its attacks with
  `wait-until` until the client sees the boss and the walk has landed within
  8 sub-tiles of it (the walk takes several ticks and input steps do not wait
  for pokes), attacks it with the character's spell and checks
  `superunique(ROW).dead` (or the class unit's mode is `dead`). The goal
  character is a Necromancer casting Bone Spear (magic damage) with a raised
  mana pool: the generated character carries no items and the runner cannot
  build `char item`, so a skill that needs a weapon or ammunition would never
  land a hit, and the cold spells left the cold-immune bosses alive. The
  goals also turn on the god switch so the boss cannot kill the character the
  goal is about. A level's `umon` columns are random-unique candidates, not
  bosses;
- the `talk` goals in the overrides (spawn the NPC next to the player, send
  the client's entity-interaction message and the NPC's own `C→S 0x31` quest
  line, then check the quest flag word changes). The NPC is spawned because a
  town's NPCs arrive with their rooms and some town rooms are not active from
  the entrance the warp lands on (Akara, Atma); the goal checks the
  conversation, not the placement. The quest line comes from the map's state-0
  message for that NPC; the Space keys only page the dialog.
  `talk` goals start where the previous goal left the character, so a chained
  goal sees the earlier conversations' quest state.

Boss and area goals are self-contained scenarios. The talk goals chain from
the previous talk goal's checkpoint, so a later conversation sees the earlier
quest state; a failed talk goal reruns from the last good one. Each worker
loads the game data once and forks a fresh game per goal (Q1's warm runner).
A blocked goal writes a standalone scenario under `<out>/t-jump/blocks/`, with
its repro under `<out>/t-jump/runs/`.

```sh
D2RUST-Game map --game-dir DIR --out map-out --t-jump --jobs 2 \
  --goals talk --limit 8 --difficulty normal
```

`--goals` filters on the `kind/name` text (a substring: `area` selects every
area goal, `area/area-40` only that one), and `--limit` caps the list. The
report is `<out>/t-jump/t-jump.json`; it gives each goal's wall time and the
total. A failed goal writes its repro under `<out>/t-jump/blocks/`.

`map --t-jump` exits 1 when a goal blocked. A goal marked `pending = "reason"`
in the overrides (a known open defect) is reported as `pending` and does not
fail the run; every other blocked goal does. The totals give `passed`,
`blocked`, `pending` and `failed` (blocked and not pending).

## Scope

The light tier (G0-G3) does not walk or play in real time: T-static and T-jump
only. The interaction matrix runner (G4) and mod onboarding (G5) come later.
