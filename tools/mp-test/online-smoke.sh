#!/bin/sh
# The online smoke of the shipped client on a real d2rust-server over loopback (CI server):
#   sh C:/Users/USER/.t3/projects/lc/scratch/remote.sh sh tools/mp-test/online-smoke.sh [--release]
# Builds d2rust-server, starts it on 127.0.0.1 (a fresh folder: new identity, open registration,
# game data from D2_GAME_DIR), reads address and certificate from its ready line, then runs
# d2-client's online_smoke test (feature `online`; --release: the release profile, as shipped):
# register, create a character, create + join a game, enter the world. The server is stopped at
# the end whatever happens.
# D2RUST_SMOKE_ADDR=host:port D2RUST_SMOKE_CERT=<64 hex> skip the local server (a remote one).
# Exit code: the test's (0 = pass), 2 when the server did not start.
set -u
profile=
[ "${1:-}" = --release ] && profile=--release
: "${D2_GAME_DIR:?D2_GAME_DIR is not set}"
pid=
dir=
cleanup() {
    [ -n "$pid" ] && kill "$pid" 2>/dev/null && wait "$pid" 2>/dev/null
    [ -n "$dir" ] && rm -rf "$dir"
}
trap cleanup EXIT INT TERM
if [ -z "${D2RUST_SMOKE_ADDR:-}" ]; then
    cargo build $profile -p d2rust-server || exit 2
    bin=${CARGO_TARGET_DIR:-target}/$([ -n "$profile" ] && echo release || echo debug)/d2rust-server
    bin=$(cd "$(dirname "$bin")" && pwd)/d2rust-server
    dir=$(mktemp -d)
    printf 'listen = 127.0.0.1:0\ngame_data = %s\ncharacters = characters\nlogs = logs\nidentity = identity\nregistration = open\n' \
        "$D2_GAME_DIR" > "$dir/d2rust-server.cfg"
    (cd "$dir" && "$bin" --config d2rust-server.cfg --new-identity > new-identity.log 2>&1)
    (cd "$dir" && exec "$bin" --config d2rust-server.cfg > console.log 2>&1 < /dev/null) &
    pid=$!
    i=0
    until grep -q '^ready ' "$dir/console.log" 2>/dev/null; do
        i=$((i + 1))
        if [ $i -gt 600 ] || ! kill -0 "$pid" 2>/dev/null; then
            echo "online-smoke: d2rust-server did not start:"; tail -20 "$dir/console.log"; exit 2
        fi
        sleep 0.2
    done
    ready=$(grep -m1 '^ready ' "$dir/console.log")
    echo "online-smoke: $ready"
    D2RUST_SMOKE_ADDR=$(printf %s "$ready" | sed -n 's/.* listen=\([^ ]*\).*/\1/p')
    D2RUST_SMOKE_CERT=$(printf %s "$ready" | sed -n 's/.* cert_sha256=\([0-9a-f]*\).*/\1/p')
fi
export D2RUST_SMOKE_ADDR D2RUST_SMOKE_CERT
cargo nextest run $profile -p d2-client --features online --test front_end --run-ignored only \
    --no-capture -E 'test(online_smoke)'
rc=$?
[ -n "$dir" ] && { echo "online-smoke: server console (tail):"; tail -15 "$dir/console.log"; }
echo "online-smoke: exit $rc"
exit $rc
