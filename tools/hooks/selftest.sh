#!/bin/sh
# Selftest of tools/hooks/pre-commit in a scratch repo: a staged Blizzard
# file or RE output must block the commit, an ordinary file must not, and
# neither must an MPQ or its part in game-data/mpq/ (the one exception).
set -e
root="$(cd "$(dirname "$0")/../.." && pwd)"
hook="$(cd "$(dirname "$0")" && pwd)/pre-commit"
d=$(mktemp -d)
trap 'rm -rf "$d"' EXIT
cd "$d"
git init -q .
git config user.email t@t; git config user.name t
mkdir -p game re game-data/mpq game-data/mpq/sub
for f in a.mpq Game.exe x.dll s.d2s a.dc6 a.dcc a.dt1 a.ds1 a.cof a.tbl a.bin game/x.txt re/x.txt \
    game-data/mpq/Game.exe game-data/mpq/sub/a.mpq game-data/a.mpq; do
  : > "$f"
  git add -f "$f"
  if sh "$hook" >/dev/null; then echo "FAIL: $f was not blocked"; exit 1; fi
  git reset -q "$f"
done
echo ok > readme.md
git add readme.md
sh "$hook" >/dev/null || { echo "FAIL: ordinary file blocked"; exit 1; }
for f in game-data/mpq/d2data.mpq.part-000 game-data/mpq/d2sfx.mpq game-data/mpq/D2XVIDEO.MPQ.part-001; do
  : > "$f"
  git add -f "$f"
  sh "$hook" >/dev/null || { echo "FAIL: $f (game-data exception) blocked"; exit 1; }
done
echo "pre-commit hook selftest: ok"

# The same patterns over everything tracked in the real repo (CI has no
# staged change to check, so it checks the tree).
bad=$(git -C "$root" ls-files | grep -Eiv '^game-data/mpq/[^/]+\.mpq(\.part-[0-9]+)?$' \
  | grep -Ei '\.(mpq|exe|dll|d2s|dc6|dcc|dt1|ds1|cof|tbl|bin)$|^game/|^re/' || true)
if [ -n "$bad" ]; then echo "FAIL: tracked Blizzard/RE files:"; echo "$bad"; exit 1; fi
echo "tracked tree: no blocked files"
