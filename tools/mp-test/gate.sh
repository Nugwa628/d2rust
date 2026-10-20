#!/bin/sh
# The multiplayer gate: every multiplayer PR runs it on the CI server and pastes the summary.
#   sh C:/Users/USER/.t3/projects/lc/scratch/remote.sh sh tools/mp-test/gate.sh [mp-test args]
# Builds the tool, its stand-in server and d2rust-server, then runs every scenario in
# tools/mp-test/scenarios against both servers: the stand-in (synthetic town, no game files) and
# d2rust-server (game data from D2_GAME_DIR; skipped without it). Extra arguments go to mp-test
# (for example --filter join).
# Exit code: 0 when no rule broke (pending scenarios don't count), 1 when one did, 2 when the
# build failed or a scenario doesn't parse.
# Reports: target/mp-test/<server>/mp-test.md and mp-test.json.
# Multiplayer work runs below the game-bug lanes and test tools on the server (owner
# 2026-10-19), so everything here is niced; MP_NICE overrides the level.
set -u
n=${MP_NICE:-15}
nice -n "$n" cargo build -p mp-test -p d2rust-server || exit 2
bin=${CARGO_TARGET_DIR:-target}/debug
rc=0
rm -rf target/mp-test
nice -n "$n" "$bin/mp-test" --server synth --out target/mp-test/synth "$@"
r=$?
[ $r -gt 1 ] && r=2
[ $r -gt $rc ] && rc=$r
if [ -n "${D2_GAME_DIR:-}" ]; then
    nice -n "$n" "$bin/mp-test" --server d2rust-server --out target/mp-test/d2rust-server "$@"
    r=$?
    [ $r -gt 1 ] && r=2
    [ $r -gt $rc ] && rc=$r
else
    echo "mp-test gate: D2_GAME_DIR is not set; d2rust-server not run"
fi
for f in target/mp-test/*/mp-test.md; do
    [ -f "$f" ] && { echo; echo "==== $f"; cat "$f"; }
done
echo "mp-test gate: exit $rc"
exit $rc
