#!/bin/sh
# game-data.sh: the game data folder the server needs, and nothing else.
#
#   sh scratch/remote.sh sh tools/server/game-data.sh <game install> <out dir>
#
# Converts the install to a native folder (d2-convert), marks every file unread, runs the
# server's --probe on it (one game, a new character of every class joins and is saved), and
# copies only the files the probe read into <out dir>, keeping the folder layout. The result
# is probed again on its own, then packed as <out dir>.tar.xz for the server box, where it
# goes to ~/d2rust-server/game/ (tools/server/README.md). Needs a file system that records
# reads (relatime or strictatime, the default on Linux). Low CPU priority.
#
# <out dir> is cleaned before it is made: an earlier run's folder is replaced. To make that
# clean safe, the script refuses an empty or relative path, `/`, `$HOME`, and any existing
# folder it did not make itself (it marks its folder with .d2rust-game-data).
set -eu
[ $# -eq 2 ] || { sed -n 2,4p "$0" | sed 's/^# //'; exit 2; }
INSTALL=$1
OUT=$2
MARKER=.d2rust-game-data
T=${CARGO_TARGET_DIR:-target}

die() { printf 'game-data: %s\n' "$*" >&2; exit 1; }

# Never removes anything the script cannot prove it made.
guard_out() {
    [ -n "$OUT" ] || die "refusing an empty out dir"
    case $OUT in
        /*) ;;
        *) die "the out dir must be an absolute path: $OUT" ;;
    esac
    OUT=$(readlink -m -- "$OUT") || die "cannot resolve the out dir: $OUT"
    [ "$OUT" != / ] || die "refusing / as the out dir"
    [ "$OUT" != "$HOME" ] || die "refusing the home folder as the out dir"
    if [ -e "$OUT" ] && [ ! -d "$OUT" ]; then
        die "refusing the out dir $OUT: not a folder"
    fi
    if [ -d "$OUT" ] && [ ! -f "$OUT/$MARKER" ]; then
        die "refusing the out dir $OUT: it is not a game-data folder this script made"
    fi
}
guard_out
# The test (test-server-ctl.sh) checks the guard without running the conversion.
if [ "${D2RUST_GAME_DATA_GUARD_ONLY:-}" = 1 ]; then
    printf 'out ok: %s\n' "$OUT"
    exit 0
fi
[ -d "$INSTALL" ] || die "no game install at $INSTALL"

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

nice -n 10 cargo build -q --release -p d2-convert -p d2rust-server
"$T/release/d2-convert" convert --install "$INSTALL" --out "$WORK/native" --quiet

# Every file looks unread: an access time older than its change time.
find "$WORK/native" -type f -exec touch -a -d '2000-01-01 00:00:00' {} +
touch "$WORK/marker"

probe() {
    printf 'game_data = %s\nlogs = %s/logs\ndata = %s/data\n' "$1" "$WORK" "$WORK" >"$WORK/probe.cfg"
    "$T/release/d2rust-server" --config "$WORK/probe.cfg" --probe
}
probe "$WORK/native"

rm -rf "$OUT"
mkdir -p "$OUT"
: >"$OUT/$MARKER"
(cd "$WORK/native" && find . -type f -anewer "$WORK/marker" -print) >"$WORK/read.txt"
# The layer list is read before any file of a layer; keep it with them.
(cd "$WORK/native" && find . -maxdepth 2 -type f -name '*.toml') >>"$WORK/read.txt"
sort -u "$WORK/read.txt" | (cd "$WORK/native" && xargs -d '\n' cp --parents -t "$OUT")
echo "kept $(sort -u "$WORK/read.txt" | wc -l) of $(find "$WORK/native" -type f | wc -l) files: $(du -sh "$OUT" | cut -f1) (all: $(du -sh "$WORK/native" | cut -f1))"

# The kept folder alone must still play.
probe "$OUT"
tar -C "$(dirname "$OUT")" -cf - "$(basename "$OUT")" | xz -T2 -6 >"$OUT.tar.xz"
echo "packed $OUT.tar.xz ($(du -h "$OUT.tar.xz" | cut -f1))"
