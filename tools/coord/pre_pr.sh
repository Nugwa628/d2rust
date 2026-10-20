#!/bin/sh
# Pre-PR gate for sprint sessions: the fast CI checks, run before every push
# that opens or updates a PR. Usage: sh tools/coord/pre_pr.sh [crate ...]
# With crate names, clippy/tests run only for those (d2-client is slow; pass it
# only when you touched it). Exit non-zero on the first failure.
set -e
# Windows: python3 and python may be Microsoft Store stubs; fall back as
# forge.sh does (python, then the py launcher). Linux: python3 as it is.
if ! python3 -c '' 2>/dev/null; then
  if python -c '' 2>/dev/null; then python3() { python "$@"; }; else python3() { py -3 "$@"; }; fi
fi
python3 tools/methods.py check
python3 tools/assemble.py --selftest
python3 tools/coord/forge_watch.py --selftest
python3 tools/coord/seq_check.py --selftest
python3 tools/coord/new_tools.py --selftest
python3 tools/farm/farm.py --selftest
cargo fmt --all -- --check
for c in "$@"; do
  cargo clippy -p "$c" --all-targets -- -D warnings
  if cargo nextest --version >/dev/null 2>&1; then
    cargo nextest run --no-tests=pass -p "$c"
  else
    cargo test -p "$c"
  fi
done
echo "pre_pr: OK"
