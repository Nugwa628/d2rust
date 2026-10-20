# Feature method: ranked modules

How D2RUST builds a feature, from plan to merged code. The owner set it on 2026-10-16. It applies
to every plan in `docs/plans/` and to every session that implements one.

## 1. The idea

- A feature is a list of **modules ranked by importance**.
- Each module is a small, self-contained change that leaves the game working.
- Modules are built in rank order, so whenever work stops, what landed is a complete, smaller feature, never a half-built big one.
- A feature grows in **tiers**:
  - **Light** is the first time box (usually about 1 hour) and gives the biggest felt improvement for the least work.
  - **Full** is the complete feature, often days, built later in further time boxes.
- Both tiers live in one plan, so the light tier is designed as the first part of the full feature, not a dead end.

## 2. Writing a plan

One file per feature, `docs/plans/<area>/<feature>.md` (areas: `game`, `launcher`, `server`, `mods`, `tools`), in this shape. It starts with the front matter described in `docs/plans/README.md`, so tools can read plans:

1. **Status line:** planned / in progress / done, the date, and its place in `docs/plans/README.md`.
2. **Goal:** what the player gets, in one or two sentences, plus any owner rules (fixed decisions that modules must respect).
3. **Where things are today:** the current behaviour with file paths and type or function names. Read the code to write this; don't guess.
4. **Module table:** rank, id, module, tier, size, depends on.
   - Ids are short and stable (`D1`, `S3`), so later plans and PRs can refer to them.
   - Ranks are a single list across both tiers.
5. **One section per module:**
   - **What:** the change in player terms, then in code terms.
   - **Files:** where it goes, including any new module or type.
   - **Seam:** the type, system or setting that carries the new behaviour.
   - **Done when:** checks a reviewer can run; tests by name where possible.
   - **Risks:** what could go wrong, if anything.
6. **Not planned:** ideas considered and set aside, with one line of why.

## 3. Ranking modules

Rank by these, in order:

1. **Felt by the player:** how much better the game looks or plays with this module alone.
2. **Unblocks others:** groundwork several modules need, such as a settings key or a shared type. It ranks just above its first user, or as rank 0 when everything needs it.
3. **Cost:** cheaper first when two modules feel about the same.
4. **Risk:** riskier modules (anything touching game logic, saves or the protocol) go later, after the safe wins have landed.

A module may depend only on modules ranked above it.

## 4. Sizing a module

- A module fits in one PR that one session can finish and review in one sitting: under about 30 minutes for light-tier modules, at most a day for full-tier ones.
- Anything bigger is split into ranked sub-modules (`D7a`, `D7b`), each of which still leaves the game working.
- Sizes are estimates. Once a module lands, write the time it actually took into the plan.

## 5. Building a module

1. **Read first:** the plan section, the files it names, and how D2RUST behaves there today.
2. **One PR per module:**
   - The title starts with the id, for example `D1: scale modes`.
   - The PR body has what changed, how it was tested, and the plan's "done when" list ticked.
3. **Clean seams:**
   - New behaviour lives behind one small type, system or setting (a `ScaleMode` enum, a limiter system), not scattered `if`s.
   - Put new code in its own file when it's more than a few functions.
   - The module header comment says what it does and why, in D2RUST terms.
4. **Defaults keep today's behaviour** unless the owner picked a new default in the plan. A module adds a choice; it never silently changes how the game looks or plays.
5. **Layering rules hold:**
   - Client-only features stay in `d2-client`.
   - Anything touching game rules goes through `d2-sim` with deterministic integer math.
   - Changes to how the game plays go behind `Ruleset` switches (see `CLAUDE.md`).
6. **Rewrite as you go:** comments in the code a module changes are rewritten in D2RUST terms in the same PR, and code is restructured only as far as the module needs (see "Rewrite as you go" in `WORKFLOW.md`). There is no separate rewrite pass before features.
7. **Persisted formats:** a new settings key, save field or protocol message bumps that format's version and migrates old data. Old files must keep loading.
8. **Tests:**
   - Unit tests for every rule the module adds (mapping math, limits, migrations).
   - Visual or feel checks go in the PR body as steps for the owner.
9. **Green before merge:** `sh tools/coord/pre_pr.sh <crates>`, then CI.

## 6. Time boxes

- A time box names a tier, or a rank range, and a duration.
- Work goes down the ranks.
- When the box ends, stop after the last **merged** module. Unfinished work goes back into the plan as notes; it never lands half-done.
- At the end of a box, update the plan:
  - mark modules done, with their PR numbers and actual time;
  - re-rank what's left if something new was learned.

## 7. Updating plans

- Plans are living documents: a module may be split, merged, re-ranked or dropped when the code shows a better way.
- Change the plan in the same PR as the code that taught you, or in a small docs PR before it.
- Owner rules in a plan change only when the owner says so.

## 8. Parallel work

- Modules with no dependency between them can go to separate sessions at the same time, one module each, on separate branches.
- Modules that touch the same files run one after another, in rank order.
