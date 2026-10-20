---
id: game-explorer
title: Game map and goal bot (any game on D2RUST)
area: tools
status: planned
order: 3
modules: G0-G5
light: G0-G3
depends_on: [test-suite]
---

# Plan: game map and goal bot

Status: **planned** (owner, 2026-10-19). Replaces [test-suite](test-suite.md) Q12 with a general tool. The light tier runs on BIGRIG; large matrix runs go to the CI server.

## Goal

For any game that runs on D2RUST (the base game or a mod), find out **without being told**:
- what the player is meant to do (main objectives, progression order, endgame);
- what is playable (classes, areas, NPCs, items, skills, objects, services);
- which mechanics and interactions exist, and whether each one works.

Then prove that every objective can be reached, and save a ready repro scenario for anything that blocks.

Mods are their own games with their own data. So nothing about the base game is hard-coded: the tool reads the game's own tables and writes a **game map** for that game. A short hand-written override file fixes what the data can't say.

## Outputs

1. **`game-map.json`** is generated from the game's data and contains:
   - **Objectives:** quests with their steps, flags, rewards and prerequisites; boss kills; act/chapter transitions; endgame.
   - **World graph:** areas, links between areas, waypoints, town/safe areas, and the doors, portals and keys that gate each link.
   - **Playables:** classes, skills, items (by type, slot and socket), NPCs with their services, and objects (chests, shrines, wells, portals).
   - **Mechanics:** an interaction matrix of actors × actions × targets, for example skill → monster, item → NPC service, item → cube, object → player, and player → player (trade, party).
2. **`game-map.overrides.toml`** is hand-written and short. It names what the data can't say: the real final objective, story order the data doesn't encode, and mechanics the game intends to be unreachable.
3. **`GAME.md`** is a readable summary built from the map for humans and AI: "this game has 5 acts, 27 quests, 7 classes, the endgame is X…". An AI working on a mod reads this first.
4. **Reports:** for each goal, mechanic and tier, a pass, a block (with the reason and a repro `.scn`), or not-covered.

## Speed tiers

Owner 2026-10-19: only the cheap tiers. Walking and route bugs (stuck on a door, blocked corridor) are too rare in D2RUST to check on every run, so nothing here walks or plays in real time.

| Tier | What it checks | Cost |
|---|---|---|
| **T-static** | The graph from the data alone: every area is reachable, every quest step has a way to complete it, every reward exists, no item or recipe points to something missing, and every class has usable skills. | milliseconds, on every PR |
| **T-jump** | The sim is loaded at each goal's starting checkpoint. The player is placed next to each step: the door opens, the NPC dialog advances the flag, the boss can be damaged and killed with the step's gear, and the portal leads where the map says. | about 1-5 s per goal; minutes for everything |

## How it stays fast

- **Checkpoint chaining:** each passed goal saves a checkpoint, and the next goal starts from it. A failure reruns only from the last checkpoint.
- **One warm process per worker:** data is loaded once and a fresh sim is forked per case (Q1's warm runner). This removes the 3.5-4.4 s startup from each case.
- **No client and no render.** Pictures are Q6's job.
- **Seeds and difficulties as a matrix:** on the CI server's cores. BIGRIG runs T-static and small samples.

## Oracles (what counts as a bug)

A bug is only a broken **stated** rule (suite rule). Look, feel and balance are not bugs until the owner states a rule.
- **Blocked:** a goal can't be reached in any tier that should reach it.
- **Softlock:** in the map, no available action leads closer to any remaining objective.
- **Wrong result:** a flag, reward, area or portal doesn't match the map.
- **Generic rules from Q2** run everywhere: no panic, no desync, nothing duplicated or lost, and nothing stuck.
- **Balance numbers are reported as information only:** time to kill a boss at the expected level, damage spread per class, and so on. They become checks only when the owner states a target.

Findings are **Unverified** and go to `bugs-unverified` with their repro. They are never auto-fixed.

## Modules

- **G0, data reader:** builds `game-map.json` from the tables (quests, levels, level links, waypoints, monsters with boss flags, NPCs and services, objects, skills, items, cube recipes, runewords, class data). Adds the override file and the `GAME.md` writer. Tested on the base game and one synthetic tiny game.
- **G1, T-static checks** over the map. Runs in the fast gate.
- **G2, checkpoints:** saves and restores sim state at goal boundaries, plus the warm fork per case.
- **G3, T-jump goal runner:** generated goals become scenarios, with the repro `.scn` written on a block.
- **G4, interaction matrix runner:** actor × action × target cases from the map, each placed and checked like T-jump. Reuses Q3's content families, which turns them into the general form.
- **G5, mod onboarding:** one command (`d2rust-game map --game-dir <mod>`) produces the map, `GAME.md` and the T-static report for a new mod, plus a guide for the AI that adapts the overrides.

- **Status (cx-game-map, 2026-10-19):** G0+G1 are implemented in PR #416 (map reader, overrides, `GAME.md`, T-static; synthetic tiny game in normal tests, base-game test ignored). G2 is implemented in PR #412 (`repro::checkpoint` on the warm Session). G3 is implemented in PR #419, stacked on #416 and #412 (generated T-jump goals, checkpoint chaining, a standalone repro `.scn` on a block; synthetic tests only). Real-game runs and measurements: owner later.

Light tier: **G0-G3**, which runs on BIGRIG in minutes. Full tier adds G4-G5, with the large matrix runs on the CI server.

Not planned (owner 2026-10-19): real-time play, route walking and an exploration bot.

PRs: G0+G1, G2+G3, G4+G5.

## Owner decisions (2026-10-19)

- `game-map.overrides.toml` for a mod lives in **the mod's own files** (mods ship as their own files). The base game's file lives in this repo.
- `GAME.md` is a **developer file**: it is generated for AI and developers and does not ship to players.
