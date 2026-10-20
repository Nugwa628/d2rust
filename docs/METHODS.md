# Methods

The working methods of this project, written to carry over to other
projects. Each entry states the method in general terms; its **Here** line
binds it to this project. `CLAUDE.md` holds the hard rules and project
facts; it points here for how work is done. Every session follows every
method below.

Format (checked by `py tools/methods.py check`, run in CI): `## Mnn Title`,
then exactly the lines Rule, Why, Check, Here, Status, in that order.
Status is `proven — <evidence>` (shown to work in this project) or
`trial — <what would prove it>` (adopted, not yet shown). Add a method
with `py tools/methods.py new "Title"`; export the general form (no Here
lines) with `py tools/methods.py export <file>`. Change an entry only with
evidence; a trial that fails is removed or rewritten, not kept.

## M01 Ground truth decides
- Rule: Every behavior taken from a reference is held to exact match under a comparison the spec defines for its area (bytes, pixels, decoded samples, ticks). There is no "close enough" tier.
- Why: no reliable boundary between "must be exact" and "may be approximate" exists, and one slipped area costs more debugging than loosening saves.
- Check: each feature names its comparison and the reference it runs against; a passing comparison is the only proof of done.
- Here: the reference is the original 1.14d (`CLAUDE.md` hard rule 10): live `.bin` data, traces from `tools/trace-recorder`, CPU-reference renders.
- Status: proven — 73 tables byte-identical from text (`data-tool tables`), 32,543 recorded RNG draws matched, GPU render byte-identical to the CPU reference.

## M02 Unverified is not done
- Rule: A feature whose check cannot run yet is marked unverified and its check is queued with the exact command and the expected result; nobody judges it done by eye.
- Why: an unchecked "done" is where errors enter silently and spread.
- Check: every unverified item has a queue entry; queue entries are removed only with a recorded result.
- Here: the local run queue, one file per entry in `docs/handoff/run-queue/` (older entries in `docs/HANDOFF.md` §5); spec statuses in `specs/`. A check PARTIAL with no differing row may count EQUAL only as PARTIAL-EQUAL (owner decision 2026-10-12): set by `tools/coord/promote.py --partial-equal` alone, marked in the ledger note, listed by `tools/coord/ledger.py --partial-equal` so bug-fixing revisits it (`tools/coord/README.md`); an input check with no packets channel and every other channel MATCH is EQUAL (single-player rule, owner decision 2026-10-13).
- Status: proven — table callbacks and fix-ups written in the cloud, confirmed through the queue on game files (2026-10-05).

## M03 Specs are the memory
- Rule: Behavior lives in specs: confirmed facts with provenance, a status, open questions, one owner per fact. Code implements a spec and names it; transcripts are never the record.
- Why: sessions forget; a fact stated in two places drifts.
- Check: every module names its spec; every rule has one owner spec; unknowns are open questions, not guesses.
- Here: `specs/` (bar and template in `specs/README.md`, `specs/_TEMPLATE.md`); module header `// Spec: specs/...`.
- Status: proven — every Phase 0–2 module built from specs; callbacks reproduced 17,355 calls from their spec.

## M04 Clean-room separation
- Rule: Sessions that read reference implementations or decompiled code write prose, tables and test vectors only; implementation sessions read specs only.
- Why: keeps licenses clean and forces every fact through a checkable statement instead of copied code.
- Check: implementation inputs are limited to specs, docs and the codebase; specs contain no pasted code.
- Here: `CLAUDE.md` hard rules 2–4 (`re/`, `../refs/` for spec sessions only).
- Status: proven — all crates written from specs.

## M05 Turn facts into checks
- Rule: A list or table that code consumes is kept as machine-readable data with a mechanical check, not as prose.
- Why: a failing check is cheaper and surer than rereading prose.
- Check: each such list has a data file and a command or test that fails when the data and the code disagree.
- Here: `specs/data/fields.tsv` + `tables.tsv` with `data-tool tables`; generated code with its staleness test.
- Status: proven — 3,591 field entries drive the compiler, the cross-check and the generated structs.

## M06 Evidence over reviewers
- Rule: One focused writer per spec or module, then an executable check. No stacked review, revise and critic layers. Agent prompts are tight, with exact file and section pointers.
- Why: more reviewers add cost and correlated opinions, not proof.
- Check: quality claims point to a check result, not to a review.
- Here: `data-tool tables`, `mpq-tool formats`, `d2-client verify`, traces.
- Status: proven — a 24-agent review workflow cost about 6M tokens for little gain; single writers plus checks did better.

## M07 Stop errors where they are made
- Rule: Check every artifact at the boundary where it is produced or extended, before anything consumes it. Inputs are strict; errors are loud; nothing falls back silently.
- Why: if each error causes fewer than one new error before it is caught, errors die out at a cost that grows with the work, not with its square; global checking after the fact does not scale.
- Check: each producer (spec edit, generator, compiler, loader) has its own check; unknown input is an error, not a default.
- Here: strict parsers (`txt-format.md` §9), spec index check and generated-file check in CI, load checks in `d2-data::bin`.
- Status: proven — strict format and table parsers surfaced every 1.14d quirk as a spec rule instead of a hidden fallback.

## M08 Prove the check can fail
- Rule: For each check, change an input on purpose and confirm the check reports exactly that change.
- Why: a check that cannot fail proves nothing.
- Check: each acceptance check has a perturbation test.
- Here: `crosscheck_catches_perturbations`, `d2-client verify --perturb N`.
- Status: proven — both report exactly the perturbed bytes or pixels.

## M09 Say where a claim holds
- Rule: A fact passed between sessions, agents or documents names its scope: branch or commit, version, the data it was checked on. "Exists" means "exists on the main branch" unless stated.
- Why: a fact true in one place and assumed everywhere is an error crossing a boundary unnoticed.
- Check: handoffs and prompts state scope; a receiver treats an unscoped claim as unverified. Every PR description has a `Measured on: <commit>` line (`n/a` when it reports nothing measured), and the commit is in the PR's history.
- Here: branch names in prompts and HANDOFF; 1.14d and archive (P/X/D) in specs; `tools/coord/measured_on.py` in `.github/workflows/pr-body.yml` checks the PR line, re-run by editing the description (owner, 2026-10-15, #771).
- Status: trial — adopted 2026-10-05 after a prompt claimed typed structs existed that were only on an unmerged branch; proven when handoffs stop producing such leaks.

## M10 Proven methods before new ones
- Rule: Solve a problem with a project convention, an existing tool or an established method first. Invent only when none fits; then say so and give the new piece its own check before anything builds on it.
- Why: an improvised solution is an untested component added where checks are thinnest.
- Check: commits and specs that introduce a new method say so and name its check; a PR that adds a `tools/**/*.py` gives it a `--selftest` that CI or `pre_pr.sh` calls.
- Here: tools in `tools/`, the methods in this file, debugger traces, Ghidra exports; `tools/coord/new_tools.py` in `pre_pr.sh` and the CI tools job checks the added tools (owner, 2026-10-15, #771).
- Status: trial — adopted 2026-10-05.

## M11 Read by section
- Rule: Large documents have a section index with line ranges; readers read the index, then only the sections they need, and prompts name sections.
- Why: context is a finite channel; whole-file reads waste it.
- Check: a tool generates the index on demand; nothing generated is committed into the documents.
- Here: specs over 12 KB; `py tools/spec_index.py` writes `docs/spec-index.md` (gitignored), `py tools/spec_index.py specs/x.md` prints one spec's index; CI (`--check`) only fails if a spec holds a committed index block (revision 2026-10-13: the in-file blocks were the main merge-conflict source between parallel PRs).
- Status: proven — CI caught a stale index on `main` (2026-10-05).

## M12 Start from the map
- Rule: One handoff file holds state, next steps in order, a code map and the command that proves each thing; sessions start there instead of exploring.
- Why: exploration costs tokens and rediscovers what is known.
- Check: every session ends by updating the handoff file.
- Here: `docs/HANDOFF.md`, the current state only (under 200 lines; 2026-10-14: the 1 MB history moved, byte-exact, to `docs/handoff/sections/`, read by section index and parsed through `tools/handoff_text.py`), plus the checklist and decisions log in `docs/PLAN.md`; messages between running sessions go through `coord` issues on the forge (`tools/coord/forge.sh inbox|read|say|new`, `CLAUDE.md` "Where work runs"), never into the handoff.
- Status: proven — fresh cloud sessions completed tasks from it alone.

## M13 One step per session
- Rule: A session does one step: start from the handoff and the latest commit, end by updating the handoff, committing and pushing. Start fresh rather than continue a long thread.
- Why: long threads cost more every turn and carry stale context.
- Check: each step ends in a commit that updates the handoff.
- Here: `CLAUDE.md` "How to work a task".
- Status: proven — RNG and cross-reference steps each finished in one short session.

## M14 Match the agent to the task
- Rule: Pick model and effort by task class; run parallel agents only on independent work with separate files.
- Why: parallel agents buy speed, not savings; overpowered agents waste budget, underpowered ones make errors.
- Check: launches state the task class and why they are independent.
- Here: RE, spec writing, exactness debugging, architecture: Opus, high. Implementation from a clear spec, extraction scripts, tools: Opus or Sonnet, medium. Docs cleanup, formatting, boilerplate, renames, simple fixes: Sonnet or Haiku, low. Every session has two project subagents (`.claude/agents/`, 2026-10-14): `runner` (Haiku: exact, fully specified steps; stops and reports on anything unexpected) and `worker` (Sonnet: scoped implementation, tests, docs, lookups); the lead session briefs them precisely, runs independent ones in parallel, and checks every result before using it.
- Status: proven — medium-effort implementation sessions finished specced work first time.

## M15 Spend on the critical path
- Rule: Spend on the critical path first, reach throughput with more independent useful work in parallel rather than with padding (review layers, repeated runs, higher effort than the task class needs), and raise effort only where the task class needs it.
- Why: padding adds cost without proof, and work off the critical path delays nothing when it waits; independent work in parallel is what buys speed.
- Check: each batch launch states its critical-path items and why the parallel work is independent.
- Here: the token budget is set per session by the user, not here. A focused run costs about 250–500k; a long main thread costs more every turn. High effort only for RE and spec writing, an exactness mismatch the first fix did not solve, architecture, and a spec a whole phase builds on.
- Status: trial — revised 2026-10-14 by the user's decision; proven when batches run without padding.

## M16 Split work by access to ground truth
- Rule: Work that needs the reference runs where the reference is; elsewhere, work proceeds on a branch and queues its checks (M02).
- Why: environments without the reference cannot prove fidelity, but can do everything else.
- Check: work that needs the reference is never marked done where the reference is absent.
- Here: cloud has the main repo plus the game data repo, both on the Forgejo (the 1.14d install and extracted data, 2026-10-08); local also has `re/`, `../refs/` and Windows for the recorder (`CLAUDE.md` "Where work runs"). Game runs go where there is capacity (owner decision 2026-10-15; `CLAUDE.md` "Where work runs", brief `docs/LANE-BRIEF.md`): BIGRIG native first for important runs while it has room, else the cloud (1.14d under Wine, one Game.exe per VM). Code lanes never sit idle waiting: a row needing a recording or game run gets a file in `docs/handoff/run-queue/` and a `BLOCKED` post in the lane's thread, the lane takes its next row, and recorder lanes work the run queue and post the results there.
- Status: proven — cloud implementation plus local confirmation (2026-10-05).

## M17 Generated code is committed and checked
- Rule: Code derived from data is generated by a tool, committed, never edited by hand, and guarded by a test that regenerating gives the same output.
- Why: the data stays the single source; drift between data and code fails a test.
- Check: the staleness test runs in CI.
- Here: `data-tool gen-tables` → `crates/d2-data/src/tables/generated.rs`; test `generated_file_is_current`. Exception: a rollup merged from per-session parts is not committed, since every PR touching it conflicted (2026-10-12): sessions commit only their `docs/handoff/ledger/*.tsv` parts; `docs/handoff/fidelity-ledger.tsv` + `.md` are gitignored, built by `tools/coord/ledger.py` in CI `check` (with `--check`) and on demand by the tools that read them; `ledger.py --print-summary` measures a branch without them.
- Status: trial — adopted 2026-10-05; proven once schema changes flow through it.

## M18 Frameworks at the edge
- Rule: Core logic is pure and deterministic and never lives inside a framework; frameworks are confined to the outer layer; dependency rules are checked mechanically.
- Why: the core stays testable and portable; framework upgrades touch one layer.
- Check: a dependency checker runs in CI.
- Here: Bevy only in `d2-client` (`CLAUDE.md` hard rules 5–7); `cargo run -p depcheck`.
- Status: proven — depcheck green; data and formats crates are framework-free.

## M19 Variants beside the reference
- Rule: Changes to reference behavior are data layers or explicit switches beside the reference code path, never edits of it.
- Why: the reference stays checkable while variants grow; each change is visible as a layer.
- Check: reference conformance tests run with no variant enabled.
- Here: `Ruleset::Original` vs `Ruleset::Mod`, patch layers (`CLAUDE.md` hard rules 8–9, `specs/data/patch-layers.md`).
- Status: trial — proven in the mod repository, where patch layers are built (out of this project's scope, `CLAUDE.md` "Scope"); here it is enforced as hard rules 8–9 (owner, 2026-10-15, #771).

## M20 Version every persisted format
- Rule: Every format written to disk or the wire carries a version number from its first commit.
- Why: unversioned data cannot evolve without breaking old files silently.
- Check: format specs state the version field.
- Here: saves, protocol, traces (`traces/FORMAT.md`), caches.
- Status: proven — traces carry their format version.

## M21 Log escapes
- Rule: When an error escapes a check, record where it was made, where it was caught, and the check that would have caught it at its source; then add that check.
- Why: the distance from creation to catch shows which boundaries lack a local check.
- Check: each escape has a log line and a new or changed check.
- Here: one file per escape in `docs/handoff/escapes/` with the fields made / caught / check / added (`python3 tools/coord/escapes.py --new <slug>`; `--check` in `pre_pr.sh`; `--pending` lists escapes whose check is not added yet); the 46 earlier lessons (Problem | Fix only) stay in `docs/handoff/sections/8-lessons-problems-met-fixes.md` (2026-10-14).
- Status: trial — adopted 2026-10-05.

## M22 Provisional over blocked
- Rule: When a behavior can only be settled by a capture or recording, do not leave the code blocked: implement the most plausible reading of the evidence already in the spec, mark it provisional in spec and code, and name the capture that settles it. Captures of choices that spread silently (RNG draw order, persisted or wire byte layouts) go to the top of the recording list.
- Why: a blocked seam leaves holes that stop the build from being played and tested; the capture comparison runs the same whether the code guessed or not, and in this project the misses found by testing have been small, local edits.
- Check: every provisional point is greppable and names its settling capture; it counts as unverified (M02), never done.
- Here: spec line `PROVISIONAL: <chosen behavior> (because …); settled by <capture / recording id>`; code comment `// PROVISIONAL (<spec ref>)` on a plain implementation (no blocking seam); the cloud implementation session makes the choice while coding, no PC research; `grep -rn PROVISIONAL specs crates` is the testing-phase checklist; the settling captures are in the HANDOFF §7 recording list.
- Status: trial — adopted 2026-10-07 by the user's decision; proven when the testing phase settles the provisional points with only local edits.

## M23 Real data before green
- Rule: From a feature's first commit, test it on the real data and against measured facts of the original, and give every boundary between systems a contract check (units, coordinate spaces, owner of each state) on both sides; invented fixtures only fill what the real data cannot reach, and are built from the real data's measured shape.
- Why: code checked only against invented data and its own spec passes thousands of tests and still fails as a whole; the errors sit at the seams and in values the invented data never had.
- Check: every merged feature has a test on real data or facts; a green gate reports how much of it ran on real data.
- Here: the game data repo and `facts/` (CLAUDE.md rule 1); the play smoke path (`d2-client` `play_smoke`) on the real install; the 2026-10-08 lesson in `docs/HANDOFF.md` §8.
- Status: trial — adopted 2026-10-08 by the user's decision; proven when real-data runs find seam bugs before a release instead of after.

## M24 Progress as remaining work
- Rule: When asked how close the build is to playable or exact, answer with the remaining work and its size (known divergences, systems never run on real data, open seams), with a stated uncertainty, never a single written-or-tested percentage.
- Why: a percentage of specs implemented or tests passing said "99 % done" for a build that did not play; the remaining-work list is what decides the next step.
- Check: progress answers name what is left and how sure the estimate is.
- Here: the coordinator's reports (`docs/handoff/coordinator-resume.md`) and answers to the user.
- Status: trial — adopted 2026-10-08 by the user's decision; proven when estimates track what the next releases actually needed.

## M25 Falsifiable guesses with an oracle early
- Rule: Write every behavior as a precise, testable claim (exact values, order, units), mark every guess as a guess with what would settle it, and put a check against the reference into the loop from the first week; then a wrong claim is a hypothesis one comparison can settle, and its correction is an edit, not a rewrite.
- Why: a wrong but labeled, precise claim costs one comparison and one small edit and teaches where the analysis was thin; the same wrong claim unlabeled, vague or never compared spreads silently into everything built on it (thousands of green tests on invented data, a build that did not play).
- Check: every guess is greppable with its settling check; every area has a comparison against the reference that can fail; a mismatch is traced to one labeled claim and fixed there, in the spec or the code, never by changing the expected value to pass.
- Here: `PROVISIONAL` / REC ids (M22) with their settling captures; the oracle is 1.14d: the game data repo, traces, `scenario-run compare`, `d2-client facts-compare`, 1.14d under Wine in the cloud (`tools/cloud-game/`); lesson of 2026-10-08 in `docs/HANDOFF.md` §8 (the oracle came after the code instead of with it).
- Status: trial — adopted 2026-10-08 by the user's decision; proven when the real-data comparisons settle provisional points with local edits to the spec or code only.

## M26 Own files per change, shared views generated
- Rule: Work that runs in parallel writes its records (status notes, verdict rows, causes, measurements, decisions) into files of its own, one per change or per lane, in a folder for that kind of record; nobody edits a shared append-table or a generated block by hand, and any combined view (an index, a summary table, a dashboard) is produced by a tool from those files, in CI or on demand, not committed as a hand-maintained copy.
- Why: when many changes land at once, every shared table or generated block they all touch conflicts on every merge even when the changes are unrelated; per-change files merge without conflict and are easy to find, review and retire later, and a generated view is never stale.
- Check: two open changes never edit the same line of a shared record file; a merge conflict in a record or generated file is treated as a defect of the layout, fixed by splitting the file or generating the view, not by hand-merging forever.
- Here: ledger parts `docs/handoff/ledger/<part>.tsv` and `docs/handoff/ledger-sprint/STATUS-<lane>.md` already follow it; check verdicts are one file per check in `docs/handoff/checks-status/` (written by `status_patch.py` / `status_md.py` / `audio_diff.py` through `tools/coord/status_store.py`; the table `py tools/coord/status_store.py --build`, gitignored `docs/handoff/checks-status.md`) and causes one file per lane in `docs/handoff/causes/` (`tools/coord/causes.py`; the list `--build`, gitignored `docs/handoff/ledger-sprint/causes-in-flight.tsv`), each file with a format line; CI and `pre_pr.sh` fail if a generated view is committed; old branches merge once with `status_store.py --import-changed` and `causes.py --import --base` (`coordinator/mergeone.sh` runs both); spec index blocks leave the specs (#548); the client-do / use-state code is one module per function (`crates/d2-client/src/bridge/client_do/fNNN_*.rs`, `use_state/sNNN_*.rs`; dispatch and shared helpers stay in each `mod.rs`): a change to one cltdofunc / cltstfunc edits its own module; the specs split the same way: one file per cltdofunc in `specs/client/client-do/fNNN-*.md` (its module's `// Spec:` line names it; `msg-skills.md` §11 keeps the dispatch, the tail and the shared helpers; the directory index is generated by `tools/spec_index.py`), so a change to one function edits its own spec and module; the local run queue is one file per entry in `docs/handoff/run-queue/` (`tools/coord/run_queue.py`; the queue `--build`, gitignored `docs/handoff/run-queue.md`; the older HANDOFF §5 entries stay in place); the provisional index (`docs/handoff/provisional-index*.tsv`) is generated by `tools/provisional_index.py` and gitignored, built by its readers when missing. Owner decision 2026-10-13.
- Status: trial — adopted 2026-10-13 by the user's decision; proven when a batch of 10+ parallel PRs merges with no conflict in record or generated files.

## M27 Short PRs that land themselves
- Rule: A change goes up as a small pull request (one finding or one fix) as soon as it passes its checks, with auto-merge on, so it lands the moment a fast pre-merge gate is green (it merges cleanly onto the base, its records are valid, it compiles); the full suite runs after merge on the main line, and a red main line is fixed forward or reverted at once by the coordinator. Its author keeps it mergeable without asking anyone: merging the base in, resolving conflicts and fixing CI on one's own PR are pre-approved and never wait for input. A PR open more than a day is merged, split or closed.
- Why: every merge moves the base under the PRs still open, so a PR that waits collects conflicts and costs a re-merge per landing; in RUN13 (2026-10-13/14) twelve open PRs needed a coordinator, a merge-batch session and fresh conflict-fix sessions, and lanes sat idle waiting for a human "yes" to fix their own conflicts.
- Check: no open PR older than a day; no lane idle on a question about its own PR's conflicts or CI; open PRs stay under 10; every red full-suite run on main is followed at once by a fix-forward or revert PR.
- Here: the only required status is `Gate / gate (pull_request)` (owner decision 2026-10-15, #782: `.github/workflows/gate.yml` merges the base in, then runs the record checks, `cargo fmt --check` and `cargo check --all-targets` of the packages `tools/ci/gate_plan.py` finds affected, and, when those include `d2-sim`, `d2-server`, `test-fixtures` or `conformance`, `cargo nextest run` of the affected ones of these, fail-fast (`gate_plan.py --tests`; 2026-10-15, after #1052 and #1074 turned `main` red past a type-check-only gate), plus the `d2-client` e2e tests (`e2e_single_player`, `e2e_full_loop`; `gate_plan.py --e2e`) when `d2-sim` or `d2-server` is affected); the full suite (`ci.yml`) runs on `main` after merge, and MC fixes a red `main` forward or reverts the breaking merge (`git revert -m 1`) at once (`docs/FINISH-LINE.md` §6.1); Forgejo "merge when checks succeed" (git.werdegames.com since 2026-10-14) is turned on for each lane PR when it is opened (`sh tools/coord/forge.sh automerge <n>`; Forgejo merges it once the gate passes); merge commits only, never rebase or force-push; a lane that sees its PR conflicted or red merges `origin/main`, re-runs `sh tools/coord/pre_pr.sh` and the checks of the crates it touched, and pushes, without asking; PRs that would edit the same lines are serialized by the coordinator, not raced. Auto-merge may not fire (#570, #465, #573 and #575 sat green and unmerged): when a PR's checks are all green (the required gate included; a head with no check runs yet is not green) and `git merge-tree --write-tree origin/main <head>` is clean, its author, or the coordinator at its tick, merges it at once with a merge commit pinned to the full 40-character head SHA (the sweep, or `POST repos/WerdeGames/D2RUST-MAIN/pulls/<n>/merge` via `sh tools/coord/forge.sh api` with `{"Do":"merge","head_commit_id":"<sha>"}`), without waiting or asking (owner decision 2026-10-14). No merge-batch sessions.
- Status: trial — adopted 2026-10-14 by the user's decision; proven when a run of 20 lanes keeps open PRs under 10 with no coordinator conflict sessions.

## M28 Numbers assigned at merge, not at branch
- Rule: Parallel work never picks the "next" number of a shared sequence (format versions, rule or revision numbers, record ids) on its branch; it names its change with a key that cannot collide (a name or a lane-scoped token), and any sequential number is given once, on the main line, by a tool, after the change has landed.
- Why: two branches that each take the next free number collide on merge, and the loser renumbers its code, spec and data; in RUN13 the recorder versions needed a hand-kept reservation registry and were still renumbered, and every menu session picked the same record id.
- Check: no branch adds a number that main has not issued; a collision on merge is treated as a defect of the numbering, fixed in the tool, not by hand.
- Here: names, no issuing tool (`specs/tools/scenario-diff.md` §4 rule 3e). Recorder output changes add a named tag file `tools/scenario-diff/formats/<script>/<tag>` (`format-tag 1`, optional `op:` / `adds: yes`; lane-prefixed name); the cache key is the frozen format plus the sorted tags, so old entries keep hitting; the tables in `orig_cache.py` are frozen at their 2026-10-14 values. Each recorder's `TOOL` is `tool_tags.tool(__file__, "<frozen literal>")` (`tools/trace-recorder/tool_tags.py`), the literal plus the same tags. New spec revisions are `(r:<name>, <date>, <lane>)`, cited `§n r:<name>`; new run ids are `REC-<lane>[-<name>]`. `tools/coord/seq_check.py` (in `pre_pr.sh` and the CI `tools` job, against `origin/main`) fails a branch that edits a frozen table or a `TOOL` literal, adds a numbered `(rN` revision or a `REC-<digits>` id main lacks.
- Status: trial — adopted 2026-10-14 by the user's decision; proven when parallel PRs land with no renumbering.

## M29 Finish-line ramp
- Rule: To finish the last 10 % of a project, run as many parallel sessions as there are non-overlapping scopes under one coordinator that does not implement: first make the remaining work countable (one generated ledger, every row with an owner and an oracle check), split hot files, and partition rows by cause area; each lane re-measures on current main, claims its rows before working them and reports findings before fixing; every finished change lands alone as a short self-merging PR; sessions talk through shared threads that each one watches for itself, never relayed: every lane posts where the reader is, and the coordinator reads every thread, answers on events (route, rule on overlaps with one final post per side, notify dependents of merges, refill any idle slot) and on a slow tick sweeps and merges green clean PRs and replaces stalled lanes. Full playbook: `docs/FINISH-LINE.md`.
- Why: the last 10 % is a long tail of small unrelated gaps that no single context holds; one session stalls on it, and many sessions without this structure duplicate work, conflict on shared files, sit on unmerged PRs and act on stale measurements.
- Check: every non-done ledger row has exactly one lane; no lane idle across two ticks; open PRs under 10 with none green and unmerged at a tick; no message relayed by the coordinator; the ledger's non-done count falls every hour.
- Here: RUN13 (2026-10-13/14, 24 lanes): ledger `tools/coord/ledger.py`, per-lane causes `docs/handoff/causes/`, sweep `tools/coord/sweep.sh` (Forgejo API; the `hold` label holds a PR), check-in then a 5-minute `send_later` self-message, replaced 2026-10-14 by `coord` threads watched every second by every session (`tools/coord/forge.sh watch`; FINISH-LINE §5, §11) with a 30-minute tick for MC (the coordinator), hot-file split #600; owner decision 2026-10-14. LC checks itself, MC and the other sessions every 3 minutes (owner rule 2026-10-15; FINISH-LINE §5.5).
- Status: trial — adopted 2026-10-14 by the user's decision after RUN13 moved EQUAL 4166 → 4250, DIVERGED 108 → 102, NO-CHECK 120 → 44 in under three hours; proven when a second project or run reaches all rows done or owned with this playbook.

## M30 Code index before reading
- Rule: Code has a generated index of its items with line ranges (M11 applied to code); when an answer may span several files, look the symbols up in the index and read only their ranges; when one search already finds the spot, read it directly.
- Why: exploration is where reads pile up, and every read is re-sent on every later turn; the index only pays when it replaces reads, and is overhead when a single search would have found the spot.
- Check: the index is generated on demand, never committed; its ranges are exact (spot-checked against the file); the method keeps its place only while A/B runs on real questions show fewer tokens with correct answers.
- Here: `python tools/code_index.py <file|dir>` (items with ranges and the file's `// Spec:` line) and `--find '<regex>' [path]` (one call across all crates, about 13 s); then read the range (Read with offset/limit).
- Status: trial — 2026-10-14 A/B, 4 Sonnet workers, same 2 questions, all answers correct: multi-file seed chain 28,620 → 21,189 tokens (−26 %), single-function lookup 18,038 → 20,296 (+12 %), total −11 %; proven when a run of 10+ real multi-file tasks keeps a saving with no accuracy loss.
