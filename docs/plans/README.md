# Plans

Every planned feature for D2RUST (game, launcher, server, mods, tools) lives here, one file per feature,
grouped by area. A plan isn't started until the owner says so; it records the agreed scope.
Every plan follows the ranked-module method in [`docs/FEATURE-METHOD.md`](../FEATURE-METHOD.md):
- modules are ranked by importance;
- there's a light tier (the first time box) and a full tier;
- each module is one self-contained PR.

## Layout

```
docs/plans/
  README.md        this index
  game/            features of the game itself (D2RUST repo)
  launcher/        features of the launcher (code in WerdeGames/D2RUST-Launcher)
  server/          features of the server side (multiplayer)
  mods/            mod features and the mod pipeline
  tools/           tools for building and testing the game (test automation for lanes)
```

## Front matter (read by tools)

Each plan starts with this block, so a pipeline can list plans, pick the next module and
check dependencies without reading prose:

```yaml
---
id: display-update            # unique, kebab-case; module ids are prefixed (D0, S1, B2)
title: Display update
area: game                    # game | launcher | server | mods | tools
status: planned               # planned | in-progress | done | on-hold
order: 2                      # place in the queue below, or "later"
modules: D0-D9                # every module id in the plan
light: D0-D5                  # the light tier (the first time box)
depends_on: [seamless-menu-game]   # plan ids, or plan-id/module-id
---
```

## Queue

| Order | Plan | Area | Light tier | Full tier | State |
|---|---|---|---|---|---|
| 1 | Seamless menu↔game: one process, one window, one event loop | game | n/a | n/a | merged: #3 |
| 2 | [Display v2: true 16:9, sharp scaling, real motion, vsync and frame limit](game/display-v2.md) | game | V0-V4 | V5-V11 | merged through V11 (#256); V12/V13 dropped (#267) |
| 6 | [Adaptive Integer canvas](game/adaptive-integer.md) | game | AI1-AI3 | AI4-AI7 | AI1-AI7 in review; adaptive Integer enabled |
| 3 | [Smoothness update](game/smoothness-update.md): 60 Hz logic, 1 ms clock, more animation frames | game | S1-S4, ~1 h | S5-S11 | in review: S10 #297/#268/#304, S11 #273; S7/S8 in progress (owner 2026-10-18), S7e look check |
| 5 | [Bug reports from the launcher](launcher/bug-reports.md) | launcher + server | B0-B2 | B7 | B0 merged; B1/B7 follow-up in review (launcher #12); B2 shelved; triage removed (#229) |
| - | [Mod seams](mods/seams.md): what code mods plug into, for channel builds | mods | MS0-MS4, MS16 | MS5-MS15 (MS12 dropped) | core merged; providers/client hooks in review; MS15 open |
| 4 | [Multiplayer v0](server/multiplayer.md) (integration in review, excluded from releases) | server | M0-M4 | M5-M9 | in review: integration #165, M1 #282/#166/#167/#272/#224, M3a #286; M4-M9 open |
| 4 | [Multiplayer v1: the full online version](server/multiplayer-v1.md), [contracts](server/multiplayer-contracts.md) | server + launcher | P0-P12 | P13-P17 | in progress (owner, 2026-10-19): accounts, lobby, server on the forge box, Play online, MP test tool; off in release builds until the owner says; PRs target `mp/next` |
| 1 | [Test automation tools for AI lanes](tools/test-automation-tools.md) | tools | T0-T4 | T5-T10 | core merged; in review: T3 follow-up #300, T8 #241, T9 #237/#261 |
| 2 | [Automated test suite](tools/test-suite.md) | tools | Q0-Q2 | Q3-Q11 | in progress: Q0-Q11 approved; Q12 moved to the game explorer |
| 3 | [Game map and goal checks](tools/game-explorer.md), for the base game and every mod | tools | G0-G3 | G4-G5 | planned (owner, 2026-10-19) |

Suite usage: [guide for AI sessions](../tools/test-suite.md).

Status snapshot: `origin/main` at `988aada1` (CI batch 76), checked against the forge's open and
closed PR API records on 2026-10-18. The merge train is paused; open PRs are **in review**, not merged.
The train often closes individual PRs after merging their batch, so **merged** here requires code
on main (including batch/cherry-picked commits), not merely a closed PR. PR numbers refer to
`WerdeGames/D2RUST` unless marked `D2RUST-Launcher` / launcher.

Module tables use one current status per module: **merged** (PR evidence), **in review** (open PRs),
**running** (active work without a PR), or **open** (work remains without evidence of an active PR).
Owner dispositions **on hold**, **dropped** and **shelved** take precedence. A parent with reviewed
slices stays in review while its listed remaining work stays open. LC will refresh after the train lands.

Comments: a one-time comments-only pass (2026-10-17) rewrote old-style comments in the most-read code as
self-describing D2RUST comments and added module overviews; it changed no logic. What it left is rewritten
as features touch the code (see "Rewrite as you go" in [`docs/WORKFLOW.md`](../WORKFLOW.md)).

Plans marked "later" are for later; none of them starts until the owner says so.
