# Lane brief (template for a cloud lane prompt)

Copy, fill every `<...>`, and post it as the lane thread's opening post
(FINISH-LINE §4); the session prompt is then only "you are lane <id>, your
thread is #<n>: read it and start". Code lanes (cloud) do code, spec and
check work; recorder lanes (cloud VMs under Wine, one game each) and BIGRIG
(native) do recordings and game runs; BIGRIG also does RE (`CLAUDE.md`
"Where work runs", M16). A recorder lane's brief is the run-queue entries
it owns instead of rows. Keep the filled brief self-contained: the lane reads nothing
else in full.

```
Lane <id>: <scope in one line>. Thread #<n>, run thread #<r>.
Sign every post as the lane: export FORGE_FROM=<id>.

First (once):
  sh tools/coord/session-setup.sh --profile <code|data>   # code is the default; data only if a row needs game files
  . tools/coord/forge.sh
  sh tools/coord/forge.sh read <n>; then keep `sh tools/coord/forge.sh watch <n> <r>`
  running as a background monitor for the whole session; answer posts as they land.
  Claim every row below in docs/handoff/causes/<id>.tsv ("claimed, not measured").

Rows, in order (refill: when one is done or parked, take the next; when the
list is empty post DONE and wait for more):
  <row id> | spec <specs/...md §n> | may touch <crates/.../x.rs, specs/...> | done: <check name> EQUAL
  <row id> | spec <...>            | may touch <...>                      | done: <ledger part docs/handoff/ledger/<part>.tsv row> / <PROVISIONAL settled>
Do not touch: <frozen files, other lanes' rows and their threads>.

Per row:
  1. Re-measure on current origin/main (a recorded verdict is a hint).
  2. Spec first: the behaviour in our own words (+ 1.14d address) in the spec;
     no decompiled code in crates/ (hard rule 2).
  3. Code + tests: cargo test -q -p <crate>; cargo clippy -q -p <crate>
     --all-targets -- -D warnings (pre_pr.sh below runs both for its crates).
  4. Done only when the named check passes (M01/M02): promotion by
     tools/coord/promote.py, never by hand; otherwise mark it PROVISIONAL (M22).

Needs a recording or a game run (1.14d side not in traces/orig-cache/, a
fresh trace, a render or ignored test on files your profile lacks):
  game runs go where there is capacity: BIGRIG (native 1.14d) first for the
  important ones while it has room, otherwise the cloud (Wine, one Game.exe
  per VM, setup profile `game`); nothing forbids running it in your own VM.
  Either run it yourself (profile `game`), or, when the run is long or
  BIGRIG should do it, add docs/handoff/run-queue/<id>-<row>.md
  (format: docs/handoff/run-queue/README.md: exact command, what to look for),
  ship the code as PROVISIONAL in the row's PR, post
  `BLOCKED <row>: queued run-queue/<id>-<row>.md` in #<n>, take the next row;
  never sit idle waiting. Recorder lanes (cloud VMs under Wine, in parallel)
  and BIGRIG work the queue and post the result in #<n>.

PRs: one small PR per row, branch off origin/main.
  PYTHONUTF8=1 sh tools/coord/pre_pr.sh <crate...>     # before every push
  git push -u origin HEAD; check `git ls-remote origin <branch>` == HEAD
  open: forge_api POST repos/$FORGE_REPO/pulls '{"title":..,"head":..,"base":"main","body":..}'
  body: what changed, rows, and `Measured on: main <sha> + this branch` (or `n/a`).
  The PR waits only for `Gate / gate` (merge base in, record checks, fmt,
  cargo check of affected packages). When it is green and
  `git merge-tree --write-tree origin/main <head>` is clean, merge it yourself:
  forge_api POST repos/$FORGE_REPO/pulls/<pr>/merge '{"Do":"merge","head_commit_id":"<40-char sha>"}'
  Post `PR #<pr> merged: <rows>` in #<n>. Conflicted or red: merge origin/main,
  fix, push, without asking (M27). The full suite runs after merge on main.

Tokens: read docs by section (python3 tools/spec_index.py <file>, M11); code via
  python3 tools/code_index.py --find '<regex>' then read ranges (M30); cargo -q
  and `| tail -20` on logs; build/test only touched crates (-p), never d2-client
  unless the row is client work; hand mechanical steps to the `runner` (Haiku)
  and scoped edits/tests to the `worker` (Sonnet) subagent (M14); results go in
  files and PRs, posts only link them (FINISH-LINE §5.2).

Blocked on anything else: post `BLOCKED <row>: <on what>` in #<n> and take the
next row; never wait idle.
```

## Who owns a file

Before asking LC whether you may edit a file outside your brief, run
`python3 tools/coord/owners.py who <file>` (needs `. tools/coord/forge.sh` for
the open-PR list). It prints the owning lane from
`docs/handoff/lanes/owners.tsv` (longest path prefix wins; `file#section` rows
hold one section of a shared spec) and the open PRs that change the file.
**free**: edit it, in your own PR. **owned** by another lane: do not edit it;
send that lane a `FINDING` in its thread with the exact change. A file changes
hands only by a row edit in `owners.tsv` (LC, or the releasing lane with a
`RELEASED` post); `python3 tools/coord/owners.py --check` (CI, `pre_pr.sh`)
fails on a prefix listed twice.
