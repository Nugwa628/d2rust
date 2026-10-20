# Running the multiplayer server

`d2rust-server` is one process: accounts, the lobby and every running game, on one UDP port with one QUIC
endpoint (`docs/plans/server/multiplayer-contracts.md`, sections 1 and 7). It runs on the forge box
(37.27.64.176) as the `lc` user, in `~/d2rust-server/`, without root. Only the steps in "Admin steps" need root.

| File | What it does |
|---|---|
| `build.sh` | builds the Linux binary for the box on the CI server (glibc 2.36, low CPU priority) |
| `deploy.sh <binary>` | the one deploy command: uploads, installs, switches, restarts, prints version and fingerprint |
| `server-ctl` | runs on the box: `setup`, `install`, `start`, `stop`, `restart`, `status`, `rollback`, `backup`, `fingerprint`, `logs` |
| `d2rust-server.cfg.example` | the config `setup` starts from |
| `game-data.sh` | builds the game data folder the server needs and copies it to the box |

## The server folder

```
~/d2rust-server/
  current -> bin/d2rust-server-<version>    previous -> the version before
  server-ctl   d2rust-server.cfg (0600)
  identity/cert.der  key.der (0600)          made once; its fingerprint is in the launcher manifest
  data/accounts.sqlite  data/characters/  data/deleted/
  game/                                      the game data, read-only
  logs/d2rust-server.log  console.log  status.json  backup.log
  backups/<date>/                            14 kept
  run/                                       pid files, stop flag, backup requests
```

## Deploy a version

From a worktree on BIGRIG:

```sh
sh C:/Users/USER/.t3/projects/lc/scratch/remote.sh sh tools/server/build.sh
sh C:/Users/USER/.t3/projects/lc/scratch/remote.sh --fetch dist/d2rust-server.xz
xz -d dist/d2rust-server.xz
sh tools/server/deploy.sh dist/d2rust-server
```

`deploy.sh` runs `server-ctl setup` (harmless when done before) and `server-ctl install`. The install checks the
new version against the config (`--check`), switches `current`, restarts (the old version saves every character
on SIGTERM first), waits up to 45 s for `logs/status.json` to report the new version, and rolls back if it doesn't.
The rolled-back version is checked the same way. When neither is healthy (or there was no earlier version), the
install stops the server and removes `current`, so the broken binary cannot be left restarting; the binary stays in
`bin/` for inspection, and the next good install takes over. It prints the fingerprint for the manifest's `servers`
entry; the fingerprint only changes if `identity/` is replaced. `server-ctl rollback` switches back by hand.

The first deploy also needs the game data (see "Game data" below) and the admin steps.

## Game data (once, and after a game data change)

The server reads a native layer folder, and only the files a game actually opens: **17,356 of 68,541 files,
218 MB of a 1.7 GB install**. `tools/server/game-data.sh` makes that folder on the CI server, which has the game
files at `/work/game`: it converts the install, runs the server's `--probe` (one game, a new character of every
class joins it over in-memory streams and is saved), keeps only the files the probe read (plus each layer's
`.toml`), probes the kept folder again, and packs it as `<out>.tar.xz`.

```sh
sh C:/Users/USER/.t3/projects/lc/scratch/remote.sh sh tools/server/game-data.sh /work/game <out dir>/game
```

`<out dir>` is cleaned before it is written on a re-run, so the script only replaces a folder it made itself: it
refuses an empty or relative path, `/`, `$HOME` and an existing folder without its `.d2rust-game-data` marker.
Keep it outside every worktree.

Carry only the `.tar.xz` (73 MB) from the CI server to BIGRIG (`scp -i ~/.ssh/lc_ci_ed25519`) and from BIGRIG
to the box (`scp -i ~/.ssh/lc_forge_ed25519`; there is no CI -> forge trust), then as `lc`:

```sh
mkdir -p ~/d2rust-server && chmod 700 ~/d2rust-server
tar -C ~/d2rust-server -xJf game.tar.xz        # makes ~/d2rust-server/game/
rm -f ~/d2rust-server/game.tar.xz
chmod -R a-w ~/d2rust-server/game               # the server only reads it
~/d2rust-server/current --config ~/d2rust-server/d2rust-server.cfg --probe
```

The probe (also available any time) prints `probe: one game, 7 characters joined and were saved (7 saves) in … ms`
and exits 0, or names the first thing that failed. It runs in memory, binds no port, changes no account data,
and writes its probe characters to a temp folder.

## How it keeps running

- **systemd user units** when lingering is on for `lc` (admin step 2): `d2rust-server.service`
  (`Restart=on-failure`, stops with SIGTERM and 60 s to save, `MemoryHigh=800M`, `MemoryMax=1G`,
  `CPUQuota=300%`) and `d2rust-server-backup.timer` (04:30 every night). The caps come from the
  measured use (83 MB idle, 92 MB peak on the one-game probe, under 1 s of one core for its join
  burst) with room for the configured games, and the live server runs at normal priority (`Nice=0`).
- **cron** until then: `server-ctl supervise` restarts the server when it exits (1 s, doubling to 60 s),
  and `server-ctl ensure` runs every minute to start the supervisor after a reboot. The backup runs at 04:30.

`server-ctl setup` (and every install) picks the mode and removes the other one's entries. `server-ctl stop` keeps it
stopped (also across cron and reboots) until `server-ctl start`.

## Health and logs

- `logs/status.json`, rewritten every 10 s: version, uptime, sessions, games, players, tick overruns. A file older
  than 30 s means the server is stuck or down. `server-ctl status` shows it.
- `logs/d2rust-server.log`: one line per event (start, stop, logins without passwords, games opened and closed,
  saves that failed). No passwords, tokens, tickets or hashes, ever. `server-ctl logs 100`.
- `logs/console.log`: what the process printed (panics end up here in cron mode; `journalctl --user -u
  d2rust-server` in systemd mode).

## Backups

Every night (and on `server-ctl backup`) a dated folder in `backups/` gets a copy of the accounts database
(`VACUUM INTO`) and of `data/characters/`. A running server makes it itself between saves (the request goes
through `run/backup.request`); a stopped one is backed up offline (`d2rust-server --backup`). The 14 newest are
kept. Restore: stop the server, copy the folder's `accounts.sqlite` and `characters/` into `data/`, start it.

## Admin steps (root; the server admin runs these)

These are the only steps that need root. Nothing else on the box changes.

1. **Open the game port, UDP 6112, to everyone.** As `lc` on 2026-10-19: ufw is installed but disabled
   (`/etc/ufw/ufw.conf` `ENABLED=no`) and the `nftables` service is inactive, so the box may have no host firewall
   for this port. Check, then use the line that matches:

   ```sh
   sudo nft list ruleset | grep -E 'hook input|policy drop'   # nothing with "policy drop": no host firewall
   sudo nft add rule inet filter input udp dport 6112 accept    # only if an input chain drops by default
   sudo ufw allow 6112/udp comment 'd2rust-server'              # only if ufw gets enabled later
   ```

   If the box has a provider firewall (Hetzner Robot "Firewall" tab or a Cloud firewall), add an incoming rule
   there: protocol UDP, destination port 6112, source any. No TCP port is needed.

2. **Keep `lc`'s services running without a login** (so the server runs as a systemd user service):

   ```sh
   sudo loginctl enable-linger lc
   ```

   Then, as `lc`: `~/d2rust-server/server-ctl setup` switches from cron to the units. Without this step the server
   still runs (cron mode); it only restarts a little slower after a crash or reboot.

3. **Optional: a DNS name**, for example `play.werdegames.com`, an A record to 37.27.64.176. Until then the manifest
   uses the IP. QUIC doesn't go through Caddy; nothing changes there.

Undo: `sudo ufw delete allow 6112/udp` (or remove the nft/iptables rule), `sudo loginctl disable-linger lc`.

## Tools on the CI server

`build.sh` needs zig and cargo-zigbuild for the `lc` user (no root):

```sh
mkdir -p ~/.local/opt ~/.local/bin && cd ~/.local/opt
curl -sSfL https://ziglang.org/download/0.13.0/zig-linux-x86_64-0.13.0.tar.xz | tar xJ && mv zig-linux-x86_64-0.13.0 zig
ln -sf ~/.local/opt/zig/zig ~/.local/bin/zig
cargo install --locked cargo-zigbuild
```
