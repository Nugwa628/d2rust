#!/bin/bash
# test-server-ctl.sh: checks server-ctl's install, switch, rollback, restart-on-crash, stop and
# backup paths against a fake server binary, in a temporary folder with no scheduler
# (D2RUST_SERVER_MODE=none plus the supervisor started by `start`). Linux only.
#
#   bash tools/server/test-server-ctl.sh        exit 0 = every check passed
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
T=$(mktemp -d)
export D2RUST_SERVER_HOME=$T/home D2RUST_SERVER_MODE=cron D2RUST_SERVER_START_WAIT=8 D2RUST_SERVER_STOP_WAIT=5
# cron mode without touching the real crontab: a crontab stub on PATH.
mkdir -p "$T/stub"
printf '#!/bin/sh\n[ "$1" = -l ] || cat >/dev/null\n' >"$T/stub/crontab"
chmod 755 "$T/stub/crontab"
export PATH=$T/stub:$PATH
CTL=$D2RUST_SERVER_HOME/server-ctl
fails=0
ok() { echo "ok   $*"; }
bad() { echo "FAIL $*"; fails=$((fails + 1)); }
check() { if eval "$2"; then ok "$1"; else bad "$1"; fi; }

# A fake d2rust-server: `--version`, `--check`, `--new-identity`, `--fingerprint`, `--backup`,
# and a run mode that writes status.json, answers backup requests, and on SIGTERM records a save.
# HEALTHY=0 never writes status.json; CRASH_ONCE crashes once while the flag file exists.
fake() {
    local ver=$1 healthy=${2:-1} out=$T/fake-$1
    cat >"$out" <<EOF
#!/bin/bash
H=\$D2RUST_SERVER_HOME
case "\$*" in
  --version) echo "d2rust-server $ver"; exit 0 ;;
  *--check*) exit 0 ;;
  *--new-identity*) mkdir -p "\$H/identity"; echo cert >"\$H/identity/cert.der"; exit 0 ;;
  *--fingerprint*) echo "cert_sha256 = 00ff"; exit 0 ;;
  *--backup*) echo "offline backup"; exit 0 ;;
esac
trap 'echo "$ver saved" >>"\$H/logs/saves.log"; exit 0' TERM
echo "$ver started" >>"\$H/logs/d2rust-server.log"
if [ -f "\$H/crash-once" ]; then rm -f "\$H/crash-once"; exit 3; fi
while :; do
  [ $healthy = 1 ] && printf '{"version": "$ver"}\n' >"\$H/logs/status.json"
  if [ -f "\$H/run/backup.request" ]; then rm -f "\$H/run/backup.request"; echo "ok backups/x" >"\$H/run/backup.done"; fi
  sleep 0.2
done
EOF
    chmod 755 "$out"
    printf '%s' "$out"
}

bash "$HERE/server-ctl" setup >/dev/null
check "setup makes the folder and a 0600 config" '[ -d $D2RUST_SERVER_HOME/data ] && [ "$(stat -c %a $D2RUST_SERVER_HOME/d2rust-server.cfg)" = 600 ]'
check "setup keeps a copy of server-ctl in the folder" '[ -x $CTL ]'

# game-data.sh cleans <out dir> before it makes it, so the guard must refuse every path
# it did not make itself. GUARD_ONLY stops just before the conversion (no cargo, no game).
GD=$HERE/game-data.sh
mkdir -p "$T/victim" && echo keep >"$T/victim/keep"
for out in "" "/" "$HOME" "$T/victim" "$T/victim/.." "relative/game"; do
    if D2RUST_GAME_DATA_GUARD_ONLY=1 sh "$GD" /nonexistent "$out" >"$T/gd" 2>&1; then
        bad "game-data refuses the out dir '$out'"
    else
        ok "game-data refuses the out dir '$out'"
    fi
done
check "game-data left the folder it refused alone" '[ -f $T/victim/keep ]'
if D2RUST_GAME_DATA_GUARD_ONLY=1 sh "$GD" /nonexistent "$T/fresh/game" >"$T/gd" 2>&1; then
    ok "game-data accepts a new out dir"
else
    bad "game-data accepts a new out dir"
fi
mkdir -p "$T/own/game" && : >"$T/own/game/.d2rust-game-data"
if D2RUST_GAME_DATA_GUARD_ONLY=1 sh "$GD" /nonexistent "$T/own/game" >"$T/gd" 2>&1; then
    ok "game-data replaces its own earlier out dir"
else
    bad "game-data replaces its own earlier out dir"
fi

# A first install whose binary never reports healthy has nothing to roll back to: it must
# stop, not be left restarting.
D2RUST_SERVER_START_WAIT=2 "$CTL" install "$(fake 0.5.0.0 0)" >"$T/out" 2>&1
check "a first install that never reports healthy fails" '[ $? != 0 ]'
check "and leaves no current version to restart" '[ ! -e $D2RUST_SERVER_HOME/current ]'
sleep 3
check "and does not restart it" '[ ! -f $D2RUST_SERVER_HOME/run/server.pid ] || ! kill -0 $(cat $D2RUST_SERVER_HOME/run/server.pid) 2>/dev/null'

"$CTL" install "$(fake 1.0.0.0)" >"$T/out" 2>&1
check "install of a first version succeeds" '[ $? = 0 ]'
check "install prints the version and the fingerprint" 'grep -q "running: d2rust-server 1.0.0.0" $T/out && grep -q cert_sha256 $T/out'
check "install makes the identity once" '[ -f $D2RUST_SERVER_HOME/identity/cert.der ]'
pid1=$(cat "$D2RUST_SERVER_HOME/run/server.pid")

"$CTL" install "$(fake 1.1.0.0)" >"$T/out" 2>&1
check "install of a second version succeeds" '[ $? = 0 ]'
check "current is the new version, previous the old" '[ "$(basename $(readlink $D2RUST_SERVER_HOME/current))" = d2rust-server-1.1.0.0 ] && [ "$(basename $(readlink $D2RUST_SERVER_HOME/previous))" = d2rust-server-1.0.0.0 ]'
check "the old version was stopped with SIGTERM (it saved)" 'grep -q "1.0.0.0 saved" $D2RUST_SERVER_HOME/logs/saves.log'
check "the old process is gone" '! kill -0 $pid1 2>/dev/null'
check "server-ctl logs reads the server log" '"$CTL" logs 5 | grep -q "1.1.0.0 started"'

"$CTL" install "$(fake 2.0.0.0 0)" >"$T/out" 2>&1
check "install of a version that never reports healthy fails" '[ $? != 0 ]'
check "and rolls back to the version before" '[ "$(basename $(readlink $D2RUST_SERVER_HOME/current))" = d2rust-server-1.1.0.0 ]'
sleep 2
check "the rolled-back version runs" 'grep -q "\"1.1.0.0\"" $D2RUST_SERVER_HOME/logs/status.json'

pid=$(cat "$D2RUST_SERVER_HOME/run/server.pid")
touch "$D2RUST_SERVER_HOME/crash-once"
kill -KILL "$pid"
sleep 4
pid2=$(cat "$D2RUST_SERVER_HOME/run/server.pid" 2>/dev/null)
check "a killed server is restarted (after a crash on the way)" '[ -n "$pid2" ] && [ "$pid2" != "$pid" ] && kill -0 $pid2'

"$CTL" backup >"$T/out" 2>&1
check "backup of a running server goes through the server" '[ $? = 0 ] && grep -q "^ok" $T/out'

"$CTL" stop
check "stop ends the server and sets the stop flag" '[ -z "$(cat $D2RUST_SERVER_HOME/run/server.pid 2>/dev/null)" ] || ! kill -0 $(cat $D2RUST_SERVER_HOME/run/server.pid) 2>/dev/null'
"$CTL" ensure
sleep 1
check "ensure does not start a stopped server" '[ ! -f $D2RUST_SERVER_HOME/run/supervisor.pid ] || ! kill -0 $(cat $D2RUST_SERVER_HOME/run/supervisor.pid) 2>/dev/null'
"$CTL" backup >"$T/out" 2>&1
check "backup of a stopped server runs offline" 'grep -q "offline backup" $T/out'

rm -f "$D2RUST_SERVER_HOME/run/stopped"
"$CTL" ensure
sleep 2
check "ensure starts the server again once the flag is gone" 'kill -0 $(cat $D2RUST_SERVER_HOME/run/server.pid) 2>/dev/null'

"$CTL" rollback >/dev/null 2>&1
sleep 2
check "rollback switches to the previous version" '[ "$(basename $(readlink $D2RUST_SERVER_HOME/current))" = d2rust-server-1.0.0.0 ] && grep -q "\"1.0.0.0\"" $D2RUST_SERVER_HOME/logs/status.json'

# A rollback target that also never reports healthy is stopped, not left restarting.
bad=$(fake 1.0.0.0 0)
mv "$bad" "$D2RUST_SERVER_HOME/bin/d2rust-server-1.0.0.0"
D2RUST_SERVER_START_WAIT=2 "$CTL" install "$(fake 2.1.0.0 0)" >"$T/out" 2>&1
check "an install whose rollback target is unhealthy fails" '[ $? != 0 ]'
check "and leaves no current version to restart" '[ ! -e $D2RUST_SERVER_HOME/current ]'
sleep 3
check "and does not restart it" '[ ! -f $D2RUST_SERVER_HOME/run/server.pid ] || ! kill -0 $(cat $D2RUST_SERVER_HOME/run/server.pid) 2>/dev/null'

"$CTL" stop
rm -rf "$T"
echo "$fails failed"
[ $fails = 0 ]
