# mp-test: the multiplayer test tool

`mp-test` tests online play the way players use it: headless bot clients (up to 64) connect to a real server
process over loopback, register, pick characters, use the lobby and play together, while a relay between each bot
and the server adds latency, jitter and loss. Every failure names the **stated rule** it breaks (owner rule: a bug
is a broken stated rule, see [`test-suite.md`](test-suite.md) section 1). It is the suite's multiplayer layer and
the required gate for every multiplayer PR ([contracts section 10](../plans/server/multiplayer-contracts.md)).

- Code: `crates/mp-test/` (library, `mp-test` and the stand-in server `mp-test-synth`).
- Scenarios: `tools/mp-test/scenarios/*.mps`.
- Gate: `tools/mp-test/gate.sh`.

## 1. The gate (every multiplayer PR)

```sh
sh C:/Users/USER/.t3/projects/lc/scratch/remote.sh sh tools/mp-test/gate.sh
```

It builds `mp-test`, `mp-test-synth` and `d2rust-server` on the CI server (niced: multiplayer work runs below the
game-bug lanes and test tools), then runs every scenario against both servers:

- **`synth`**: the stand-in (the synthetic town of the M1d multi-client harness behind a real QUIC endpoint and the
  server's hub; no game files). It serves `connect` and `game` only.
- **`d2rust-server`**: the real server with the game data in `D2_GAME_DIR` (`/work/game` on the CI server).

Exit 0 when no rule broke; 1 when one did; 2 when the build failed or a scenario doesn't parse. Paste the summary
lines (`N passed, N failed, N pending`, per server) and every broken rule in the PR. Reports:
`target/mp-test/<server>/mp-test.md` and `mp-test.json` (fetch with `remote.sh --fetch`).

A **pending** scenario never fails the gate. It is pending when the tool has no driver yet for one of its verbs,
or when the server's ready line doesn't list a capability it needs, or when the server can't run games yet. The
report says which. **A pending scenario becomes required in the PR that builds its part**: that PR writes the
verb's driver (section 5), the server lists the capability, and the scenario must pass.

## 2. Running it by hand

```sh
mp-test [--server synth|d2rust-server] [--server-bin PATH] [--game-dir DIR]
        [--scenarios DIR|FILE].. [--filter TEXT] [--out DIR] [--work DIR] [--seed N] [--list] [--verbs]
```

- `--list`: every scenario and whether it runs or why it is pending, without running anything.
- `--verbs`: every verb, its capability, built or pending, and its arguments and facts.
- `--filter join`: only scenarios whose name contains `join`.
- `--work DIR`: keep each scenario's server folder (config, logs, data) for a look afterwards. Without it the
  work folder is removed after a run with no failure.

Run it on the CI server (`remote.sh`), never against anything but loopback: the relay and the server both bind
127.0.0.1, and no packet leaves the machine (owner rule: no network tests).

## 3. Scenario format (`.mps`)

```text
# Two players see each other and one leaves.
name   join-leave
about  Two bots enter one game, each sees the other, one leaves and the other sees it gone.
rule   SEE    every player in a game is told about every other player in it (M1b)
rule   LEAVE  a player that leaves is gone for the players who stay (M1b)
bots   2
server players=8 grace=0
net    latency=0

all connect
all enter game1
b0  see                expect players=2 b1=yes   within 3s   rule SEE
b1  leave
b0  see                expect players=1 b1=no    within 3s   rule LEAVE
```

- **Header:** `name`, `about`, `rule ID TEXT` (one or more; cite where the rule is stated: a contracts section,
  `multiplayer-v1.md`, an M-module or an owner decision), `bots N` (1-64), `server KEY=VALUE..` (`players`,
  `grace` in ms, any other key goes into `d2rust-server`'s config as is, e.g. `limits=test`), `net KEY=VALUE..`
  (starting conditions), `addresses per-bot|shared` (each bot's relay has its own loopback address, the default,
  or all bots share 127.0.0.1 for a per-IP limit; contracts 10.1), `needs CAP..`, `pending TEXT` (written ahead,
  never run), `seed N` (loss and jitter).
- **Steps:** `WHO VERB ARGS.. [expect PRED..] [within DURATION] [rule ID]`.
  - `WHO`: `b0`, `b1-3`, `all`; `server start|stop|kill|restart` (`stop` is SIGTERM, `kill` a crash);
    `server admin ARGS..` (an offline admin command of the server program, with its config; needs `admin`);
    `server dev CHARACTER COMMAND..` (a dev console command in that character's game on a test server; needs `dev`);
    `server progression CHARACTER N` (sets the character's unlocked difficulties on a running test server's
    stdin, `set-progression`; needs `dev`); the dev steps wait for the server's `dev ok`/`dev err` answer;
    `net [bN|bA-B|all] latency=MS jitter=MS loss=PCT cut=on|off`; `wait 2s`.
  - Predicates: `ok`, `refused`, `refused=CODE` (the contracts' `ErrorCode` names: `Build`, `WrongLogin`,
    `DifficultyLocked`, ...), `error`, `closed`, `KEY=VALUE`, `KEY!=VALUE`, `KEY>=N`, `KEY<=N` on the step's facts.
  - Without `expect` a step must succeed; if it doesn't, the scenario stops there and the failure breaks
    `core.step` (it couldn't reach the state it tests).
  - A step that expects more than `ok` names its `rule` when the scenario states several.
  - Queries (`see`, `stats`, `games`, ...) are retried until their expectations hold or `within` runs out
    (default 3 s); actions run once and may take until `within` (default 10 s).

`mp-test --verbs` lists every verb. Built today: `connect` (with `build=`, `protocol=`, `tick=`, `game=`, `trust=`
overrides), `reconnect` (wire version 4's session and game tokens), `disconnect`, `stats`, `enter GAME` (opens the
game stream and attaches it, then C→S 0x67 and 0x6B; refused as `no-player` when no player is assigned), `leave`,
`move X Y` / `move +DX +DY`, `frames N`, `see`, `agree`; the account and character verbs (`register`, `login`,
`logout`, `char-create`, `char-list`, `char-select`, `char-delete`); and the lobby's (`games`, `create-game`,
`join-game`, `leave-game`, `chat`, `heard`). Their drivers keep the session's account state on the control stream and
answer each request by its kind, a request's `ERROR` included (`wait_s` for a limit, the answer's `ms`).

Play in a game (`crates/mp-test/src/play.rs` keeps what a bot is told: monsters, the items it holds and the ones on
the ground, objects, its act and the revealed rooms' levels):

- `monster MONSTER` and `attack MONSTER`: MONSTER is the word a `server dev CHARACTER spawn MONSTER` step used; the
  dev answer's class (`spawned 1 x Fallen (19)`) and where that character stood pick the monster (of that class, the
  one first seen nearest to that spot). `attack` sends C→S 0x06 (left skill on the unit) from every bot of the step
  until each sees it die (S→C 0x69 death); it is ok for a bot only once that bot attacked at least once (`attacks`),
  so a bot that just watched the death fails it. `monster` gives `seen`, `life`, `dead`.
- `inventory`, `ground`: the items held (inventory, belt, cursor, worn) or seen on the ground, by item code
  (`has.cap=yes|no` for every code the bot has seen), from S→C 0x9C/0x9D and 0x0A. `drop ITEM` takes it to the cursor
  (0x19) and drops it (0x17); `pickup ITEM` walks to it and picks it up (0x16).
- `waypoint AREA` (a name such as `cold-plains`, or a level id) and `act N` (that act's town): the bot walks next to
  the nearest waypoint and sends C→S 0x49, which the server takes from a player in range of a waypoint it knows; ok
  once the bot stands in that level. `area` gives `area`, `level`, `act`. Players path greedily, so a long walk
  through a town can stop at a wall: a scenario starts where the waypoint is near (the act 4 town's start, or
  where a waypoint trip ended).
- Not built: `trade`, `trade-accept`, `trade-offer`, `trade-confirm` (the game server has no trade between players
  yet), and the party verbs.
- Known limits: the area is read from the revealed rooms (S→C 0x07 gives a room's tile origin and level, not its
  size), as the level of the room whose origin is nearest above and left of the player; next to a level's border it
  can name the neighbouring level. The bot doesn't learn its own position while it walks (the server corrects it only
  with 0x15), and players path greedily, so a long walk can stop at a wall unseen: drivers walk only short distances
  (next to a waypoint, to an item) and scenarios start their travel where a waypoint is near.

Bots speak wire version 4 (contracts 3.1): `ClientHello`/`SessionWelcome` on the control stream, then `Attach` as the
first frame of the game stream. The stand-in has no accounts or lobby, so `enter` attaches with a stand-in ticket and
keeps the stream for the next enter after a leave. A server with a lobby hands a seat out with `JoinOk`: `create-game`
and `join-game` attach the game stream with its one-time ticket and enter as the selected character. Such a server
takes no direct `enter`, so scenarios that only say `enter` stay pending on it.

## 4. The tool's own rules (`core.*`)

These hold in every scenario, on top of its stated rules:

| Rule | What it checks |
|---|---|
| `core.server-start` | the server starts with the scenario's settings and prints its ready line |
| `core.server-alive` | the server keeps running until the scenario stops it |
| `core.no-panic` | the server never prints a panic |
| `core.conn-kept` | a bot's connection stays open unless the scenario ends it (disconnect, cut link, server stop) or expects `closed` (a new login's takeover close counts as expected) |
| `core.messages-parse` | every message the server delivers parses with its size rule and layout (`d2_proto::s2c`) |
| `core.step` | a step without expectations succeeds |

## 5. Growing it as features land

When a multiplayer part lands, its PR makes the scenarios for it required:

1. **Driver:** write the verb in `crates/mp-test/src/bot.rs` (the session's requests and answers, on the message
   codec and wire in `d2-online`/`d2-net::msg`, the client library the game uses too) and in
   `crates/mp-test/src/runner.rs` (`action` or `query`), then set `built: true` in `crates/mp-test/src/verbs.rs`.
   Report refusals with the `ErrorCode` name and the server's facts (`wait_s`, ...).
2. **Server capability:** the server's ready line lists the capability (`accounts`, `lobby`, ...).
3. **Scenarios:** the pending ones for the part now run; fix them if the contract changed, add one for every
   rule the part states that no scenario checks yet.
4. Run the gate; the PR pastes it.

A new rule (owner, contracts, a fixed bug) becomes a `rule` line and the steps that check it, in the scenario of
its area or a new file.

**The server's ready line** (what `d2rust-server` prints once it listens; the stand-in prints the same):

```text
ready listen=127.0.0.1:50123 cert_sha256=<64 hex> game=d2rust build=0.1.2.0 protocol=1 tick=60 caps=connect,accounts,lobby
```

Only `listen` is required. `cert` (a DER file) or `cert_sha256` are pinned by the bots; `game`, `build`, `protocol`,
`tick` are what a bot's hello says unless a step overrides them. Without `caps` the tool assumes the server serves
everything the tool drives except the server-only capabilities (`persist`, `admin`, `dev`, `mods`), which a
server has to list. A server that exits before its ready line saying "not built yet" makes every scenario pending.
Some capabilities depend on the server's settings (`dev` needs `dev = test`): a scenario with `server KEY=VALUE..`
is checked against the server started with its settings (started once per distinct settings).

## 6. Network conditions

Each bot connects through its own UDP relay on loopback: the bot talks to 127.0.0.1, and the relay's socket
toward the server sits on the bot's own address (127.0.0.2, .3, ...; contracts 10.1) so the server's per-IP
limits see one address per bot. A scenario that tests a per-IP limit says `addresses shared`. `net` sets one-way
`latency` and `jitter` (ms) and `loss` (percent, each direction) for some or all bots, and `cut=on` drops
everything (a lost link: the connection times out after 3 s, the bots' QUIC timeout). Conditions change while
bots play. Loss and jitter come from a seeded generator (`seed` in the scenario, `--seed` for the run), so the
same traffic loses the same datagrams. The report counts datagrams relayed and dropped.

## 7. Findings so far

- **F1 (stand-in):** in the stand-in's one-room synthetic town, a C→S 0x01 walk is queued by the server's sender
  but nothing comes of it (no 0x0F to the other clients, no 0x15 to the walker). So `move-together` and
  `move-under-lag` need `walk`, which the stand-in doesn't list; they run on `d2rust-server` once it serves games.
