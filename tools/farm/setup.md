# Ubuntu 24.04 farm and CI setup

Target: Ryzen 9 7950X, 96 GB RAM, two 1 TB NVMe drives in RAID 0. This procedure
is for the operator when the host is online (planned 2026-10-19). Q5's local
validation uses fake SSH and synthetic game data; it makes no host connection.

## Accounts and packages

Create separate `farm` and CI-runner users. Give the `farm` user ownership of
`/srv/d2farm` and `/cache`, with a shared read-only game group for the CI user.
Keep SSH credentials out of the repository. Install the public half of LC's
`~/.ssh/lc_ci_ed25519` in the farm account's `authorized_keys`; verify the server
host-key fingerprint out of band and put it in BIGRIG's `known_hosts`. Disable
password authentication and root login. Restrict the key to the operator's
network and disable agent/X11/port forwarding. No farm test needs public sockets.

```sh
sudo apt-get update
sudo apt-get install -y build-essential pkg-config clang lld cmake git curl \
  python3 openssh-server rsync util-linux ca-certificates \
  libasound2-dev libudev-dev libx11-dev libxi-dev libxrandr-dev libxcursor-dev \
  libxkbcommon-dev libwayland-dev libvulkan-dev mesa-vulkan-drivers
sudo useradd --create-home --shell /bin/bash farm
sudo install -d -o farm -g farm /srv/d2farm
sudo install -d -o farm -g farm /cache/game /cache/bin /cache/golden \
  /cache/candidates /cache/target /cache/soak
```

As `farm`, install rustup using the official rustup installer, then use the
repository's `rust-toolchain.toml` (do not pin a second compiler version here).
Install nextest and provision a forge read credential using the operator's
credential helper when necessary:

```sh
git clone https://git.werdegames.com/WerdeGames/D2RUST /srv/d2farm/repo
cd /srv/d2farm/repo
rustup show
rustup component add rustfmt clippy
cargo install cargo-nextest --locked
```

The gate scripts fetch origin with an explicit branch refspec. They never
upload a lane's local checkout. Confirm origin is the URL above. Keep this
provisioned checkout on the tested farm-tool revision; gate worktrees are
detached and cannot change its HEAD.

## Private game files and disk layout

Run the repository helper on the server; it assembles the owner's files there
from the private forge parts. Do not SCP BIGRIG's game tree, commit it, attach it
to a PR, or upload it as a CI artifact:

```sh
cd /srv/d2farm/repo
python3 tools/ci/game_files.py /cache/game
export D2_GAME_DIR=/cache/game
```

`/cache/game` is shared read-only after assembly; `/cache/golden` holds local
reference pictures and is excluded from every artifact upload/pull. The farm
only transfers binaries and candidate scenarios/saves/metadata. `/cache/bin`
is keyed by full commit, `/cache/target` by full commit and contains the
per-SHA locks; `/cache/candidates` is keyed by signature hash. `/cache/soak`
contains campaign logs and reductions. `/srv/d2farm/worktrees` contains each
gate's detached tree, and `/srv/d2farm/{checkout,soak}.lock` are advisory locks.

For soak, build the provisioned checkout's `d2-client` and `d2s-tool` in release
under the farm resource slice below, using 12 jobs. With the default target
location no path override is needed. If using `/cache/target/<sha>`, set
`D2_SOAK_CLIENT=/cache/target/<sha>/release/D2RUST-Game` and
`D2_SOAK_D2S=/cache/target/<sha>/release/d2s-tool` in the SSH login environment
(`~/.ssh/environment` only if explicitly enabled, or a forced-command wrapper).
Ordinary SSH noninteractive commands do not source an interactive shell rc.

## Resource isolation and runner labels

Run forge CI runners beside the farm under the CI account. Register distinct
labels **gate** (fast synthetic gates) and **gate-heavy** (real-data/scenarios,
nextest, release builds). Configure one heavy job at a time. Do not register
untrusted public repositories on the game-data runner.

Use cgroup v2/systemd slices to reserve capacity. For example set the farm SSH
user's slice to `CPUQuota=1200%` (12 cores), `CPUWeight=25`,
`MemoryHigh=32G`, `MemoryMax=40G`, `TasksMax=4096`, `IOWeight=25`. Set the CI
runner's slice to `CPUQuota=1800%`, `CPUWeight=100`, `MemoryHigh=40G`,
`MemoryMax=48G`, `TasksMax=4096`, `IOWeight=100`. This leaves RAM and CPU for
the OS while the farm and CI can each progress. Put limits on the user slices
(for example `user-<farm-uid>.slice.d/limits.conf`) so **all SSH commands**, not
only a manually launched service, inherit them. Verify using `systemd-cgls`
and `systemctl show user-<uid>.slice` after a test SSH command. Reload systemd
after installing drop-ins. Keep the CI service in its own limited slice.

Only one soak is allowed by `soak.lock`; same-commit gates serialize through
the target lock. Different commits can run concurrently inside the farm slice.
The slice is the hard resource guard, including concurrent cargo invocations.
Use operator timers to stop accepting heavy work at 80% disk usage and prune
old, unlocked target directories/worktrees and binaries. Preserve signatures,
Unverified candidates and golden references until the owner reviews them.
RAID 0 has no redundancy; keep code on the forge and back up metadata/private
references only to owner-approved storage. Do not prune active locks/targets.

## Operator acceptance once online

Run `status`, one gate of each kind for a pushed SHA, `bin`, a short synthetic
soak, and `candidates pull`. Confirm streaming and exact failing-gate exit codes,
host-key enforcement, Linux cache metadata, and one Unverified summary line
after two pulls of the same candidate. Check the resource slices while CI and
farm jobs overlap. Overnight real-file soak begins only after this acceptance.
