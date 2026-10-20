#!/bin/sh
# One-shot setup for a cloud session of the build loop (coordinator, 2026-10-09;
# profiles and prebuilt downloads 2026-10-15). Idempotent; each step is skipped
# when already done. Also suitable as the cloud environment's setup script.
#
#   sh tools/coord/session-setup.sh [--profile code|data|game|full] [--client]
#                                   [--no-wine] [--no-game] [--dry-run]
#
# Profiles (default: $D2_SETUP_PROFILE, else code):
#   code  pinned toolchain + cargo-nextest (prebuilt): implementation, tests,
#         refactors. Bevy's system libs only with --client (d2-client work).
#   data  code + D2RUST's MPQs assembled from game-data/ into $D2_GAME_DIR
#         (default $HOME/game) + the live excel view: game-file checks and
#         #[ignore] tests.
#   game  the same D2RUST data setup as data; no Wine setup or game launch.
#   full  data + Bevy libs: everything.
# --no-wine is accepted and ignored; --no-game alone starts from full, as
# before. --dry-run prints the planned steps and changes nothing.
#
# Prebuilt nextest: release tools-cloud-v1 of the repository (the official
# tarball). The MPQs come from this repository's game-data/ (tools/assemble.py)
# and never leave it (CLAUDE.md hard rule 1).
set -u
PROFILE=${D2_SETUP_PROFILE:-}; CLIENT=; NOWINE=0; NOGAME=0; DRY=0
while [ $# -gt 0 ]; do
  case $1 in
    --profile) PROFILE=${2:-}; shift ;;
    --profile=*) PROFILE=${1#--profile=} ;;
    --client) CLIENT=1 ;;
    --no-wine) NOWINE=1 ;;
    --no-game) NOGAME=1 ;;
    --dry-run) DRY=1 ;;
    *) echo "session-setup: unknown argument $1" >&2; exit 2 ;;
  esac
  shift
done
# legacy flags without a profile meant "everything except ..."
[ -z "$PROFILE" ] && [ $((NOWINE + NOGAME)) -gt 0 ] && PROFILE=full
PROFILE=${PROFILE:-code}
case $PROFILE in
  code) GAME=0; WINE=0; LIBS=0 ;;
  data) GAME=1; WINE=0; LIBS=0 ;;
  game) GAME=1; WINE=0; LIBS=0 ;;
  full) GAME=1; WINE=0; LIBS=1 ;;
  *) echo "session-setup: unknown profile $PROFILE (code|data|game|full)" >&2; exit 2 ;;
esac
[ -n "$CLIENT" ] && LIBS=1
[ $NOWINE = 1 ] && WINE=0
[ $NOGAME = 1 ] && { GAME=0; WINE=0; }

ROOT=$(cd "$(dirname "$0")/../.." && pwd); cd "$ROOT" || exit 2
. tools/coord/forge.sh
# Prebuilt cargo-nextest: same version as CI (tools/ci/linux-image/build.sh).
NEXTEST=0.9.148
NEXTEST_SHA256=223ba936714cc861a9cdedad5a8eb664dfdd69d6fbdf6cea6aa8184f523290dc
NEXTEST_TGZ=cargo-nextest-$NEXTEST-x86_64-unknown-linux-gnu.tar.gz
TOOLS_TAG=tools-cloud-v1     # release of $FORGE_REPO holding $NEXTEST_TGZ
export D2_GAME_DIR=${D2_GAME_DIR:-$HOME/game}
EXCEL=$D2_GAME_DIR/extracted/patch_d2/data/global/excel

T0=$(date +%s); TS=$T0; CUR=
endstep() { [ -n "$CUR" ] && echo "== session-setup: $CUR: $(($(date +%s) - TS)) s"; CUR=; }
# want NAME: start (and time) a step; under --dry-run only print it and skip.
want() {
  endstep
  if [ $DRY = 1 ]; then echo "plan: $1"; return 1; fi
  CUR=$1; TS=$(date +%s); echo "== session-setup: $1"
}
note() { echo "   $*"; }
fetch() { # URL: GET from the forge with the token, to stdout
  curl -sSfL ${FORGEJO_TOKEN:+-H "Authorization: token $FORGEJO_TOKEN"} "$1"
}
check_install() { # every MPQ of game-data/manifest.json present with its size and sha256
  $PY - game-data/manifest.json "$D2_GAME_DIR" <<'EOF'
import hashlib, json, os, sys
from concurrent.futures import ThreadPoolExecutor
files = json.load(open(sys.argv[1]))["files"]
def ok(f):
    p = os.path.join(sys.argv[2], f["path"])
    h = hashlib.sha256()
    try:
        with open(p, "rb") as fh:
            for b in iter(lambda: fh.read(1 << 22), b""):
                h.update(b)
    except OSError:
        return False
    return os.path.getsize(p) == f["size"] and h.hexdigest() == f["sha256"]
bad = [f["path"] for f, good in zip(files, ThreadPoolExecutor(8).map(ok, files)) if not good]
print("   install check: %d files, %d bad %s" % (len(files), len(bad), " ".join(bad[:5])))
sys.exit(1 if bad else 0)
EOF
}

echo "== session-setup: profile $PROFILE (client libs $LIBS, game $GAME, wine $WINE)"
# A session created from a GitHub copy works on the forge from here on.
if want "forge origin"; then
  case "$(git remote get-url origin 2>/dev/null)" in *"$FORGE_HOST"*) note "already";; *) sh tools/coord/forge.sh origin;; esac
fi

if [ $LIBS = 1 ] && want "apt: Bevy libs"; then
  if pkg-config --exists wayland-client alsa libudev 2>/dev/null; then note "already"; else
    apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq pkg-config \
      libwayland-dev libasound2-dev libudev-dev libxkbcommon-dev libx11-dev libxcursor-dev \
      libxrandr-dev libxi-dev >/dev/null
  fi
fi

# Once, not concurrently: parallel rustup installs corrupt it.
if want "toolchain"; then
  if rustup show active-toolchain >/dev/null 2>&1; then note "already"; else
    rustup toolchain install "$(sed -n 's/^channel *= *"\(.*\)"/\1/p' rust-toolchain.toml)" \
      --profile minimal -c rustfmt -c clippy
  fi
fi

if want "cargo-nextest $NEXTEST (prebuilt)"; then
  if command -v cargo-nextest >/dev/null 2>&1; then note "already"; else
    BIN=${CARGO_HOME:-$HOME/.cargo}/bin; T=$(mktemp -d)
    { fetch "$FORGE_URL/$FORGE_REPO/releases/download/$TOOLS_TAG/$NEXTEST_TGZ" > "$T/n.tgz" \
        || curl -sSfL "https://get.nexte.st/$NEXTEST/linux" > "$T/n.tgz"; } \
      && echo "$NEXTEST_SHA256  $T/n.tgz" | sha256sum -c --quiet - \
      && mkdir -p "$BIN" && tar -xzf "$T/n.tgz" -C "$BIN" && cargo nextest --version \
      || { note "prebuilt nextest failed; compiling"; cargo install cargo-nextest --version "$NEXTEST" --locked -q; }
    rm -rf "$T"
  fi
fi

if [ $GAME = 1 ] && want "game files -> $D2_GAME_DIR"; then
  # The MPQs live in this repository (game-data/, split into parts);
  # assemble.py joins and checks them, keeping files already in place.
  if check_install >/dev/null 2>&1; then note "already"; else
    $PY tools/assemble.py "$D2_GAME_DIR" \
      || echo "session-setup: assembling the game files failed (disk full? df -h) — rerun after fixing"
  fi
fi

if [ $GAME = 1 ] && want "live excel view"; then
  if [ -d "$EXCEL" ]; then note "already"; elif ! check_install >/dev/null 2>&1; then note "no game files: skipped"; else
    mkdir -p "$EXCEL"
    CARGO_INCREMENTAL=0 cargo run -q --release -p data-tool -- excel-dir "$EXCEL" "$D2_GAME_DIR" || rm -rf "$EXCEL"
  fi
fi

endstep
[ $DRY = 1 ] && exit 0
[ $GAME = 1 ] || note "no game files in profile $PROFILE: --profile data adds the MPQs + excel view"
echo "== session-setup: done in $(($(date +%s) - T0)) s: export D2_GAME_DIR=$D2_GAME_DIR CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0"
