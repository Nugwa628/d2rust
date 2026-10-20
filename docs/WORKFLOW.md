# D2RUST workflow (preliminary)

How work moves from an idea to merged code in D2RUST. It's preliminary (owner, 2026-10-16) and will be
tightened as features land. How a single feature is planned and built is in
[`FEATURE-METHOD.md`](FEATURE-METHOD.md); this file covers the people, the
pipeline and the rules around it.

## Current priority: features fast, as a proof of concept

- Features come first. There is **no separate rewrite pass** of the source code before them.
- Code and comments are rewritten **as needed, inside the feature work** (see "Rewrite as you go" below).
- Plans and their order are in `docs/plans/README.md`.

## Roles

| Who | Does |
|---|---|
| Owner | Decides features, order and owner rules. Says go. Tests and releases by hand. |
| LC (local coordinator) | Writes and ranks plans, cuts modules into jobs, starts sessions, reviews PRs against the plan, keeps plans current. |
| MC (BIGRIG) | CI runners, the merge bot, the main-CI watcher, Windows builds. |
| Code sessions | One job each: a module, or a run of modules from one plan. They write code, tests and the PR. |

## Sessions

- **A new job gets a new session.** A finished session isn't re-prompted with new work; it's closed.
- A job post names the plan, the module ids, the branch prefix (`claude/<job>-…`) and the done-when checks.
- **A job post needs an ack.** If there's no ack within 10 minutes, the session is idle or dead: check it and start a fresh one.
- **One session per job.** If two pick up the same job, the first to ack keeps it and the other stops.
- Sessions coordinate in coord threads (`sh tools/coord/forge.sh`, repo `WerdeGames/D2RUST`). Plans and decisions go in threads, not in private notes.

## Time boxes and lanes

- Work runs in **time boxes** (for example 2 hours) with **3–4 sessions at once, 5 at most**.
  - Feature modules share files, and every PR costs a CI run, so more sessions mostly add conflicts and queue time.
- A time box has **one coord thread** holding the brief: the lanes, their modules in rank order, the start conditions and the rules.
- Each lane is one new session.
- Split lanes by **files touched**, not by feature: two lanes never edit the same file in the same box.
  - Different features can run side by side when their files are separate (display and smoothness did).
  - Where lanes do overlap, stack the later one on the earlier one's branch.
- A lane posts:
  - `ACK <lane>` when it reads the brief;
  - `START <lane>` when its start conditions are met (the box counts from here);
  - one line per PR opened;
  - one line when blocked;
  - a final status when the box ends.

## Subagents

Sessions use subagents (the Task tool) to go faster, matched to the work:

| Model | Use for |
|---|---|
| Haiku | Finding files and symbols, reading and summarising code, greps (e.g. leftover spec references), checking CI or thread status |
| Sonnet | Clearly specified sub-steps: one function and its tests, a migration, clippy/fmt fixes, per-file comment rewrites with the style rules in the prompt |
| Opus (the session itself) | Design and seams, decisions that span files, **reviewing every subagent diff before commit**, PR text |

Rules:
- Subagents in one session edit **separate files**. The session commits only after reviewing their diffs.
- Run **one cargo build or test at a time** per session; disk space is the limit, not CPU.
- Give a subagent everything it needs in its prompt: the files, the plan section, the rules and the done-when checks. It doesn't see the thread.

## Stacked PRs

- A lane doesn't wait for a merge before starting the next module.
- Every PR targets `main`. A later module's branch contains the earlier commits, so its diff shrinks as they merge. The body says `Stacked on #N`.
- The merge bot merges in order as each one goes green.

## Check-ins

- LC checks every lane, PR and CI run **every 3 minutes** during a time box.
- LC reacts at once to:
  - a missing ACK (after 10 min);
  - a red check;
  - a stalled lane;
  - two lanes about to touch the same file.

## Pipeline

1. **Plan:** `docs/plans/<area>/<feature>.md`, written by LC with ranked modules (`FEATURE-METHOD.md`).
2. **Job:** LC posts the module ids to a new session.
3. **Build:** the session reads the plan section and the code, then implements the module. It rewrites what it touches (below), adds tests and runs `sh tools/coord/pre_pr.sh <crates>`.
4. **PR:** one PR per module, with a title like `D1: scale modes`. The body has what changed, how it was tested, the done-when list ticked, and feel-check steps for the owner.
5. **CI:** fmt, clippy, tests and a Windows release build.
6. **Merge:** the merge bot merges green `claude/*` PRs. LC merges its own `lc/*` docs PRs.
7. **Plan update:** the module is marked done in its plan, with the PR number and the actual time.

## Rewrite as you go

The code came over with comments that explain it by reference to the original game (spec sections, addresses, "matches the original"). A one-time comments-only pass (2026-10-17) cleaned up the most-read code; the rest is cleaned up wherever feature work goes:

- **Touch it, fix it.**
  - When a module changes a function, type or file, rewrite the comments of what it changes in D2RUST terms: what the code does, why, and which game rule it follows.
  - Remove spec references and address literals from those lines.
  - Untouched code is left alone, so PRs stay small and on topic.
- **Restructure only for the feature.**
  - Change code shape when the module needs it (e.g. an integer scale becoming `f32`, a new `ScaleMode` seam).
  - A larger restructure the feature needs becomes its own ranked module in the plan, with id `0` or a lettered sub-module.
- **Module headers.**
  - Each file a module edits ends up with a header comment saying what part of the game it is and how it fits.
  - A new file gets one from the start.
- **Reasons come from evidence:** the code, its tests or its behaviour. If the reason for a rule isn't clear, say so honestly ("this order is deliberate; test X pins it") rather than inventing one. The archived 1.14d source (`WerdeGames/1.14d-MAIN`, read-only) is consulted only in rare cases.
- **Old tooling goes when it's in the way.**
  - Leftover tick-exact verification tooling from the earlier project is removed when a module conflicts with it (smoothness will), in that module's PR or a sub-module before it.
- **Behaviour doesn't change by accident.**
  - A rewrite inside a feature PR changes comments and structure, not how the game plays.
  - Behaviour changes are the feature itself and are listed in the PR body.

Over time, the most-worked areas become the best-described ones, which is where the description matters most.

## CI and machines

- CI runs on BIGRIG's runners (Linux for check, Windows for the release build).
- Tests link Bevy as a shared library with no debug info (`--features d2-client/dylib`). That keeps the client's ~120 test binaries within the runner's disk.
- Builds and tests are native. There are no Wine containers and no recordings; nothing here runs the original game.
- Docs-only changes (`docs/**`, `*.md`) skip CI.
- **Cloning:** `game-data/` holds about 2 GB of MPQ parts. Clone with `git clone --filter=blob:limit=2m --depth 50 …` so they aren't downloaded; building and testing never need them (`tools/assemble.py` rebuilds the MPQs only for runs against game files).
- Job logs are only on the forge web UI. Sessions that can't read them ask MC in a thread for the error lines.

## Not tracked here

Playtesting and releases are done and tracked by the owner by hand. Sessions report work, not test or release status.

## Retired from the earlier project

Wine recorder lanes, recordings, the run queue, the ledger and promotions, and reverse engineering as a daily input.
