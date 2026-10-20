# Goal checkpoints (`repro::checkpoint`)

A checkpoint is a goal-boundary state that the game explorer's T-jump goal
runner (G3, [`plans/tools/game-explorer.md`](../plans/tools/game-explorer.md))
saves after a passed goal and starts the next goal from. It lives in
`crates/d2-client/src/app/repro/checkpoint.rs`.

D2RUST's sim is deterministic from the game data, the seed, the character
and the input script. A checkpoint is therefore the run's canonical
`scenario 2` prefix plus the server `state-1` line at the boundary; restore
replays the prefix on a warm `repro::Session` (game data loaded once) and
the state bytes must match exactly. A mismatch is an error, so a silent
divergence cannot become a wrong goal result.

## API

```rust
use d2_client::app::repro::{checkpoint::Checkpoint, RunOpts, Session};

// Save after a run that ended at tick 30 with opts.capture_state = true.
let cp = Checkpoint::capture(&opts.out, &passed_goal, 30)?;

// Or run the prefix and save in one call.
let cp = Checkpoint::save(&mut session, &passed_goal, 30, &opts)?;
cp.write(path)?;

// Prove a boundary on the warm session (replay + exact state bytes).
let restored = cp.restore(&mut session, &opts)?;

// Build the next goal: the tail is a normal `scenario 2` script; only its
// steps are taken.
let next = cp.chain(&goal_tail)?;
let report = session.run(&next, &opts)?;

// Or fork one case from the checkpoint in a single call.
let report = cp.fork_case(&mut session, &goal_tail, &opts)?;
```

`Checkpoint::chain` shifts the tail's ticks past the boundary by `at` and
extends `end`; the tail's character, save, seed, difficulty, tick rate and
variant must match the checkpoint's. [`Checkpoint::fork_case`] chains and
runs the case on the warm session, so each case gets a fresh server game
and client join without a data reload. [`RunOpts::capture_state`] makes
`repro` write
`state-end.jsonl` (the server `state-1` line after the run), which
`capture` reads.

## File (`checkpoint 1`)

JSON with `format` (`checkpoint 1`), `at` (script ticks after tick 0),
`script` (the canonical prefix), and `state` (the `state-1` line). The
file is readable and diffable; the state line is the largest part.

## Warm fork

The warm runner is Q1's `repro::Session` / `repro::batch`: one worker
process loads the game data once, then each case builds a fresh server
game and client join (`Session::run`) with no data reload. Checkpoints use
the same session, so a restore pays only the prefix replay, never the
3.5-4.4 s install load.
