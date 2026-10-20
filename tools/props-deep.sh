#!/bin/sh
# Deep property-test hunt: long proptest runs, more cases than CI.
#   sh tools/props-deep.sh [sim|wire|worldsim|wiredpath|all] [cases]
#     sim       d2-sim state-machine / model / path / timer properties  (default cases 20000)
#     wire      d2-proto, d2-server, d2-data, d2-formats properties and parsers (20000)
#     worldsim  d2-client prop_worldsim, the wired game over time (500; needs
#               the Bevy system libraries)
#     wiredpath d2-sim prop_wired_path, the path wiring on the wired sim (300;
#               ~2 s per case in a debug build, so not in the sim group)
# Proptest keeps no regression files here (failure_persistence is off), so the
# counterexample exists only in the output: it is written to
# target/props-deep/<group>.log and its "minimal failing input" block is
# printed at the end (and appended to $GITHUB_STEP_SUMMARY when set).
# A counterexample is a real bug, never a flake (HANDOFF §8): paste the
# minimal input into a fixed regression test, then fix the wrong side.
cd "$(dirname "$0")/.." || exit 2

group=${1:-all}
CASES=${2:-$CASES}   # optional override of every group default
out=target/props-deep
mkdir -p "$out"
export PROPTEST_MAX_SHRINK_TIME=${PROPTEST_MAX_SHRINK_TIME:-60000}

if cargo nextest --version >/dev/null 2>&1; then
  RUN="cargo nextest run --no-tests=pass --no-fail-fast --failure-output final"
else
  echo "cargo-nextest missing: cargo install cargo-nextest --locked" >&2
  exit 2
fi

FAILED=0

# run <group> <cases> <nextest args...>
run() {
  name=$1; cases=$2; shift 2
  echo "=== $name (PROPTEST_CASES=$cases)"
  start=$(date +%s)
  PROPTEST_CASES=${CASES:-$cases} $RUN "$@" >"$out/$name.log" 2>&1
  rc=$?
  secs=$(( $(date +%s) - start ))
  # Per-test wall times, slowest first (nextest prints "PASS [ 12.3s] ...").
  sed 's/\x1b\[[0-9;]*m//g' "$out/$name.log" | grep -E '^ +(PASS|FAIL)|Summary' |
    sort -t'[' -k2 -rn | head -12
  echo "$name: exit $rc, ${secs}s"
  if [ $rc -ne 0 ]; then
    FAILED=1
    report "$name" "$secs"
  fi
}

# Print (and add to the job summary) every counterexample block of a log.
report() {
  name=$1
  clean="$out/$name.clean.log"
  sed 's/\x1b\[[0-9;]*m//g' "$out/$name.log" >"$clean"
  {
    echo "### Property failure: $name"
    echo
    echo "Failing tests:"
    echo '```'
    grep -E '^ +FAIL' "$clean" | sort -u
    echo '```'
    echo "Counterexamples (paste the minimal input into a regression test):"
    echo '```'
    grep -E -A40 'minimal failing input|Test failed|Test aborted' "$clean" | head -200
    echo '```'
  } >"$out/$name.summary.md"
  cat "$out/$name.summary.md"
  [ -n "$GITHUB_STEP_SUMMARY" ] && cat "$out/$name.summary.md" >>"$GITHUB_STEP_SUMMARY"
  return 0
}

case $group in
  sim|all)
    run sim 20000 -p d2-sim -E '(binary(/^prop_/) & !binary(=prop_wired_path)) | test(/prop_tests::/)' ;;
esac
case $group in
  wiredpath|all)
    run wiredpath 300 -p d2-sim --test prop_wired_path ;;
esac
case $group in
  wire|all)
    run wire 20000 -p d2-proto -p d2-server -p d2-data -p d2-formats \
      -E 'binary(/^prop_/) | test(/robust/) | test(/_property$/)' ;;
esac
case $group in
  worldsim|all)
    run worldsim 500 -p d2-client --test properties -E 'test(/^prop_worldsim::/)' ;;
esac

[ $FAILED = 0 ] && echo "props-deep: PASS" || echo "props-deep: FAIL (logs in $out/)"
exit $FAILED
