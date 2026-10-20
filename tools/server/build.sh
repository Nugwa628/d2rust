#!/bin/sh
# build.sh: the Linux d2rust-server for the server box, built on the CI server.
#
#   sh scratch/remote.sh sh tools/server/build.sh        (from a worktree on BIGRIG)
#   sh scratch/remote.sh --fetch dist/d2rust-server.xz   (then tools/server/deploy.sh on it)
#
# The box runs an older glibc (Debian 12: 2.36) than the CI server, so the binary is linked
# against that glibc with cargo-zigbuild (zig as the linker; no root, no container). It runs at
# a low CPU priority: game bug lanes and test tools come first on the CI server.
# Output: dist/d2rust-server (and .xz for the upload), and the version it reports.
set -eu
GLIBC=${GLIBC:-2.36}
TARGET=x86_64-unknown-linux-gnu
PATH=$HOME/.local/bin:$HOME/.local/opt/zig:$HOME/.cargo/bin:$PATH
export PATH
command -v cargo-zigbuild >/dev/null || { echo "build.sh: needs cargo-zigbuild and zig (see tools/server/README.md)" >&2; exit 2; }

nice -n 10 cargo zigbuild --release --locked -p d2rust-server --target "$TARGET.$GLIBC"
out=${CARGO_TARGET_DIR:-target}/$TARGET/release/d2rust-server
mkdir -p dist
cp "$out" dist/d2rust-server
strip dist/d2rust-server 2>/dev/null || :
xz -T2 -6 -kf dist/d2rust-server
dist/d2rust-server --version
# The newest glibc symbol the binary needs must not be newer than the box's.
need=$(objdump -T dist/d2rust-server | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail -n 1)
echo "needs $need (box: GLIBC_$GLIBC)"
