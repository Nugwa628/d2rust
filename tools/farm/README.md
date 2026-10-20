# Build and soak farm (Q5)

Python 3.10+, OpenSSH, and rsync for candidate pulls. Stdlib only. No command
uploads source, game files, or pictures. Gates fetch pushed commits from
`https://git.werdegames.com/WerdeGames/D2RUST`. Full 40-character commit IDs
are required; push a WIP branch before submitting a farm gate.

```sh
export D2_FARM_HOST=farm                  # SSH alias or farm@hostname
export D2_FARM_KEY="$HOME/.ssh/lc_ci_ed25519"
export D2_FARM_ROOT=/srv/d2farm
python3 tools/farm/farm.py build COMMIT -p d2-client -p d2s-tool
python3 tools/farm/farm.py clippy COMMIT -p d2-client
python3 tools/farm/farm.py test COMMIT -p d2-client
python3 tools/farm/farm.py bin COMMIT
python3 tools/farm/farm.py soak --hours 8 --seeds 1,7,42
python3 tools/farm/farm.py candidates pull
python3 tools/farm/farm.py status
```

On BIGRIG use `py -3`. With `D2_FARM_HOST` unset **every command prints
`local (D2_FARM_HOST unset)`**. Local gates require a clean tracked checkout at
the requested HEAD; they reject a different SHA rather than test other code.
On Windows they use LC's build queue (and `ci-wait.sh` for tests). A missing
binary, game installation, or queue reports why local execution cannot proceed.
Local commands never contact SSH. `status` is read-only; `candidates pull` indexes
the existing local candidates when no host is configured.

Remote stdout and stderr stream directly to the caller. Gate exit codes,
including SSH's 255 for connection failure, propagate unchanged. Validation and
local prerequisite failures return 2. SSH is noninteractive and requires a known
host key; it never silently accepts a new host key.

## Checkout and resource contract

The server has a provisioned checkout at `$D2_FARM_ROOT/repo`. Gates fetch branch
refs from the pinned forge URL and create a detached worktree per SHA.
A short checkout lock protects fetch/worktree creation, and a per-SHA lock
protects that target directory for the duration of a gate. A failed command
stops the script; it cannot install a stale binary after a failed build.

The cache defaults to `/cache`; `D2_FARM_CACHE` can override it. Gate target dirs
are `/cache/target/<sha>`, binaries `/cache/bin/<sha>/D2RUST-Game`, and game files
`/cache/game`. Builds use release, clippy uses all targets with warnings denied,
and tests use nextest. All use 12 build jobs; tests use 12 threads. Clippy/test
enable `d2-client/dylib`, matching the lane gate contract. Local BIGRIG gates
inherit the shared queue's limits: the queue rewrites the CLI counts. As of the
owner's 2026-10-18 06:49 update these are 5 slots, 6 build jobs and 6 test threads
per slot. Use the script as-is; never set `BUILD_SLOTS`, `BUILD_JOBS` or
`BUILD_TEST_THREADS` in lane commands.

`bin` installs the downloaded Linux executable at
`E:/work/bin/<sha>/D2RUST-Game` (`D2_BIN_ROOT` overrides the root), the shared Q1
binary-cache namespace, with `farm.json` recording SHA, Linux, and release.
Downloads use a `.part` file and publish only after SCP succeeds. Linux binaries
retain their extensionless name; Windows cache consumers must require `.exe`
and must not select these as Windows builds. A local build caches the native
binary in the same SHA directory. This command does not build on a cache miss.

## Soak and candidates

`soak` runs the provisioned server checkout's already-built game and d2s-tool.
Build/provision those before starting it; it does not silently compile. Override
their paths with `D2_SOAK_CLIENT` and `D2_SOAK_D2S`. A server-wide nonblocking
soak lock prevents duplicate campaigns. The given seeds start successive time
slices (an equal share of the remaining hours each); each slice visits every
soak start, then increments its seed as the existing campaign does. Reduction
can extend past the time budget. Findings that cannot be reduced remain in the
campaign log and are reported, without pretending to be replayable scenarios.

Each reduced signature exports once to
`/cache/candidates/<sha256-of-signature>/{failure.scn,candidate.json}`. A start
save, when needed, is copied beside the scenario with a content-hash filename.
The `.scn` is a `soak-scenario 1` envelope: JSON with `signature`, `steps`, and
the complete `soak-log 1` text in `log`. It intentionally preserves the soak's
join timing, setup, inventory GUID actions, and round trips. `repro` dispatches
this versioned format to the same soak engine/checks:

```sh
python3 tools/soak/soak.py export reduced.log --sig 'desync:player-position' --out failure.scn
D2RUST-Game repro failure.scn --game-dir /cache/game --out /cache/repro
```

`repro.json` lists every finding and `signature_reproduced`; exit 1 means a
finding still exists, 0 means none remain, 2 means invalid input, and 3 means
missing game data. These envelopes do not support `--query`. They reproduce
engine findings, not the Python campaign's external wall-time/crash guards.
Treat a different signature or `signature_reproduced: false` as a changed
failure, not proof that the original signature remains.

`candidates pull` incrementally mirrors only scenarios, saves and metadata to
`E:/work/artifacts/candidates/`. It appends one **Unverified** line per new stable
signature to
`C:/Users/USER/.t3/projects/lc/scratch/bugs-unverified/farm-candidates.md`.
Summary markers make repeated pulls idempotent. Use a single pull writer at a
time. Overrides: `D2_FARM_CANDIDATES`, `D2_FARM_SUMMARY`. The tool never posts to
forge issues and never fixes findings. On Linux, local fallback defaults to
`/cache/candidates` and `/cache/candidates/farm-candidates.md` instead of the
BIGRIG paths.

## LC build-slot.sh farm-mode interface

LC owns that script outside the repository. Before acquiring a local slot:

1. If `D2_FARM_HOST` is set and the command is a supported clippy/nextest gate,
   resolve the lane's clean HEAD to a full SHA and ensure its WIP ref is pushed.
2. Translate `cargo clippy ... -p NAME` to
   `py -3 tools/farm/farm.py clippy SHA -p NAME` and `cargo nextest run ...` to
   `... test SHA -p NAME`. Repeated package selectors are supported. These are
   the standard lane gates, including the fixed job count and dylib setting.
3. Inherit stdout/stderr and exit with farm.py's exact status. Do not hold a
   BIGRIG slot during remote execution. Log capture, if desired, belongs to the
   caller (for example `> gate.log 2>&1`).
4. Commands with filters, nonstandard features/profiles, `--no-run`, or other
   flags this interface cannot represent stay local. Never silently discard
   flags. Build dispatch is available explicitly through `farm.py build`.
5. Unset `D2_FARM_HOST` for local fallback to prevent recursion. farm.py already
   clears it before calling the local queue. A remote failure is a failed gate;
   do not silently repeat a failing gate locally.

Tests (no SSH, no game assets):

```sh
py -3 -m unittest discover -s tools/farm -v
py -3 tools/farm/farm.py --selftest
```

The synthetic Rust integration test `farm_soak_scenario` also executes Python's
export command and the repro CLI, comparing the original and replayed signature
and detail. See [setup.md](setup.md) for the server layout and limits.
