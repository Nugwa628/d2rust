#!/bin/sh
# deploy.sh: puts a d2rust-server build on the server box and switches to it.
#
#   tools/server/deploy.sh [--host USER@HOST] [--key FILE] [--home DIR] <binary>
#
# <binary> is the Linux d2rust-server built for the box (tools/server/build.sh makes it). The
# script copies it and server-ctl to the box, runs `server-ctl setup` the first time, then
# `server-ctl install`, which checks the new version against the config, switches to it,
# restarts, waits until it reports healthy (rolling back if it doesn't) and prints the version
# and the certificate fingerprint for the launcher manifest. Characters, accounts, the identity
# and the config stay where they are.
#
# Defaults: the forge box, LC's key on BIGRIG, ~/d2rust-server on the box.
set -eu

HOST=lc@37.27.64.176
KEY=$HOME/.ssh/lc_forge_ed25519
HOME_DIR=d2rust-server
while [ $# -gt 1 ]; do
    case $1 in
        --host) HOST=$2; shift 2 ;;
        --key) KEY=$2; shift 2 ;;
        --home) HOME_DIR=$2; shift 2 ;;
        *) break ;;
    esac
done
[ $# -eq 1 ] && [ -f "$1" ] || { sed -n 2,4p "$0" | sed 's/^# //'; exit 2; }
BIN=$1
HERE=$(cd "$(dirname "$0")" && pwd)
SSH="ssh -i $KEY -o BatchMode=yes -o ConnectTimeout=15"
SCP="scp -q -i $KEY -o BatchMode=yes -o ConnectTimeout=15"

# The folder is made by setup; the upload goes to a temporary name first.
$SSH "$HOST" "mkdir -p $HOME_DIR/run"
$SCP "$HERE/server-ctl" "$HERE/d2rust-server.cfg.example" "$HOST:$HOME_DIR/"
$SCP "$BIN" "$HOST:$HOME_DIR/run/upload.bin"
$SSH "$HOST" "chmod 755 $HOME_DIR/server-ctl && export D2RUST_SERVER_HOME=\$HOME/$HOME_DIR &&
    $HOME_DIR/server-ctl setup && $HOME_DIR/server-ctl install $HOME_DIR/run/upload.bin;
    rc=\$?; rm -f $HOME_DIR/run/upload.bin; exit \$rc"
