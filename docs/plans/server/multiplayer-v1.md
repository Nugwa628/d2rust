---
id: multiplayer-v1
title: Multiplayer v1, the full online version
area: server
status: in-progress
order: 4
modules: P0-P17
light: P0-P12
depends_on: [multiplayer]
---

# Plan: Multiplayer v1, the full online version

Status: **in progress** (owner, 2026-10-19 evening): build the full multiplayer version as fast as possible, in
parallel lanes, with a multiplayer test tool that every multiplayer PR runs. It follows `docs/FEATURE-METHOD.md` and
builds on [`multiplayer.md`](multiplayer.md) (M0-M9), whose owner rules still hold. The interfaces every module
builds against (wire messages, account and lobby API, storage, difficulty rule, process layout, ports, pinning,
launcher start) are fixed in [`multiplayer-contracts.md`](multiplayer-contracts.md). This page says what is built,
in which order, by which lane, and when each part is done.

## Goal

Players open the launcher, press **Play online**, log in to their account (or register), pick or create an online
character, and in the lobby create a game (name, optional password, a difficulty the character has unlocked) or
join one from the list. They play together on the dedicated D2RUST server. Online play reaches release builds
only when the owner says it is fleshed out and fully tested.

## Owner decisions (2026-10-19, answers to the questions of the first version of this plan)

1. **Where:** the multiplayer server runs on the forge box (37.27.64.176). It may hold the game data the server
   needs (`CLAUDE.md` rule 1). Root steps there (firewall port, system services) are the admin's: lanes write the
   exact commands and stop.
2. **Releases (updated by the owner, 2026-10-19 evening):** multiplayer is not released until it is fleshed out
   and fully tested. Online play stays behind a build switch, the `online` cargo feature, that is off in release
   builds; the owner decides when it turns on. Game bug lanes and test tools have priority on the CI server:
   multiplayer builds run at a lower CPU priority and don't take CI or merge capacity from them.
3. **Accounts first:** register and login before the first game together. Characters belong to an account and are
   stored on the server.
4. **Difficulty from progression:** the player picks a character before the lobby. It can create a game at any
   difficulty it has unlocked and join only games of a difficulty it has unlocked. Full lobby UI: login and
   register, character select and create, game list (name, difficulty, players, password lock), create game,
   join, chat if cheap, back out, errors that say what went wrong.
5. **Passwords:** a game with a blank password is public; a game with a password needs it to join.
6. **Landing:** the `mp/*` work is refreshed onto main as one line. `mp/v0`, `mp/m0-transport`, `mp/m2-quic` and
   the duplicate lineage are retired once their parts have landed.

Rules kept from `multiplayer.md`: authoritative server; server-hosted only (no player hosting, listen server or
peer-to-peer); QUIC only; online characters live on the server only; 8 players per game, 64 a config change; one
multiplayer architecture for D2RUST and every mod game; `mp/*` PRs stay out of the merge train.

## How the work runs

- **Branches:** `mp/next` is the integration branch: `main` plus the refreshed line (P0). Every module PR targets
  `mp/next` (or a stack on it), branch `mp/<lane>-<part>`. The owner merges `mp/next` into `main` by hand.
  cl-mp-refresh keeps `mp/next` up to date with `main` until then.
- **Lanes:** cl-mp-plan (contracts, this plan, integration review), cl-mp-refresh (P0), cl-mp-testtool (P2),
  cl-mp-server (P1, P3, P11, P14, P15, P17), cl-mp-accounts (P4, P5), cl-mp-lobby (P6, P9), cl-mp-client (P7, P8,
  P10, P12, P13). Each keeps `E:/work/lanes/<lane>/STATUS.md` current.
- **Gates per PR,** run on the CI server through `remote.sh`, results pasted in the PR: `cargo fmt --check`, strict
  clippy and nextest of the touched crates, the multi-client tests, and the MP test tool gate (P2) once it exists.
  Single-player frame hashes and determinism baselines stay unchanged. Multiplayer builds run at a lower CPU priority
  than the game bug lanes and test tools (owner, 2026-10-19).
- **Contracts:** a lane that needs an interface changed asks under "Requests" in
  `E:/work/lanes/cl-mp-plan/STATUS.md`; it doesn't change one on its own.

## Where things are today

Checked on 2026-10-19 against `origin/main` at `808d8b63`. **No multiplayer code is on `main`.** On `main`, `d2-net`
is a stub, `d2-server` has the host and per-`ClientId` queues, and single player runs that server in-process
(`d2-client/src/app/server_thread.rs`, `bridge/local.rs`). `d2_proto::PROTOCOL_VERSION` is 2.

| Branch | PR | What it does | How far | Conflicts with `main` |
|---|---|---|---|---|
| `mp/sync-main` | #165 | `d2-net`: `Transport` trait, memory transport, wire v2 (frames, `Hello`/`Welcome`/`Refusal`, resume token). `d2-quic`: QUIC in its own crate (TLS 1.3, `ServerTrust`, self-signed identity, keepalive, migration). `d2-server/hub.rs`: clients in `ClientId` order, reconnect grace. TCP removed. | complete, tested | `bridge/local.rs` |
| `mp/m0-transport`, `mp/m2-quic`, `mp/v0` | #128, #133 | M0 and M2 as first built | superseded by `mp/sync-main`; retire | `Cargo.lock`, `bridge/local.rs` |
| `mp/m1a-thread` | #282 | M1a-1: `GameThread` in `d2-server`, served or looped | complete | 5 files |
| `mp/m1a-seams` | #312 | M1a-2: `LocalSeams`, rest, weapon and monster-AI seams move to `d2-server` | complete; carries rebased copies of #165, M1b, M1d | 8 files |
| `mp/m1a-loader` | #331 | M1a-3: game builder, live data, save loading, file-source seam move to `d2-server` | complete | 9 files |
| `mp/m1b-join` | #166 | M1b: join and leave per client, teardown, a character per client, player count | complete | `bridge/local.rs`, `session_flow.rs` |
| `mp/m1c-slots` | #167 | M1c: monster target lists past 8 players | complete | `ai/bodies4.rs`, `ai/target.rs` |
| `mp/m1d-harness` | #272 | M1d: player limit setting, N-client synthetic runs to 64 | complete | `bridge/local.rs`, `session_flow.rs` |
| `mp/m1e-ext` | #224 | M1e: extension channel for mod messages | complete | `bridge/local.rs` |
| `mp/m3a-server` | #286 | M3a: `d2rust-server` crate: config, character store (D2RC files), log, `--check` | partial: no game serving | `bridge/local.rs`, `session_flow.rs` |
| `mp/m6a-backups` | #321 | M6a: offline character backups (`--backup`) | complete | `bridge/local.rs`, `session_flow.rs` |

Two lineages carry the same commits: A is `sync-main` → M1b → M1d → M3a (with the older M1a-1) → M6a; B is M1a-1 →
M1a-2 (with rebased copies) → M1a-3. P0 lands one line with every part once.

## Modules

Light tier (P0-P12) is everything needed for "two players with accounts meet in the lobby and play a game" (in
test builds with `--features online`; release builds keep it off until the owner says). The full tier (P13-P17) is built in the same pass, right after its dependencies. Sizes are lane time.

| Rank | Id | Module | Lane | Tier | Size | Depends on | Lands / finishes |
|---|---|---|---|---|---|---|---|
| 0 | P0 | Refresh the `mp/*` line onto main; `mp/next` | cl-mp-refresh | light | 1-1.5 days | none | M0, M2, M1a, M1b, M1c, M1d, M1e, M3a, M6a |
| 1 | P1 | Message codec, handshake v3, control and game streams | cl-mp-plan (codec), cl-mp-server | light | 4-6 h | P0 | wire v3 |
| 2 | P2 | MP test tool: bots, scenarios, network faults, report, gate | cl-mp-testtool | light | 1-2 days, grows | P0 (P1, P7 as they land) | the MP gate |
| 3 | P3 | `d2rust-server` runs many games | cl-mp-server | light | 1-1.5 days | P1 | M3 |
| 4 | P4 | Accounts: register, login, sessions, rate limits | cl-mp-accounts | light | 1 day | P1 | M6 |
| 5 | P5 | Online characters: create, list, delete, select, storage | cl-mp-accounts | light | 4-6 h | P4 | M3 characters, M6 |
| 6 | P6 | Lobby server: game list, create, join, password and difficulty rules, chat | cl-mp-lobby | light | 1 day | P3, P5 | M8 |
| 7 | P7 | `d2-online` client library and fingerprint pin | cl-mp-client | light | 4-6 h | P1 | M4 (connect) |
| 8 | P8 | The game plays on a server: remote bridge, smoothing, reconnect | cl-mp-client | light | 1-1.5 days | P3, P7 | M4 |
| 9 | P9 | Online screens: login, register, characters, lobby, create, join, errors | cl-mp-lobby | light | 1-1.5 days | P6, P7 | M4 (menu), M8 UI |
| 10 | P10 | Launcher: Play online, manifest `servers`, `servers.toml` | cl-mp-client | light | 4-6 h | P8 | M9 (launcher) |
| 11 | P11 | Deploy on the forge box: service, identity, backups, game data, admin steps | cl-mp-server | light | 4-6 h | P3 | M9 (server) |
| 12 | P12 | Online test build and first game on two PCs; release switch ready | cl-mp-client, owner | light | 2-3 h | P8-P11 | owner test |
| 13 | P13 | Latency, loss and jitter | cl-mp-client, cl-mp-testtool | full | 1-2 days | P8, P2 | M5 |
| 14 | P14 | Security hardening | cl-mp-server, cl-mp-accounts | full | 2 days | P4, P6 | M7 |
| 15 | P15 | More than 8 players | cl-mp-server | full | 4-6 h | P3 | M1c measurements |
| 16 | P16 | Mod games online | cl-mp-server, cl-mp-client | full | 1-2 days | P11 | M1e, mod servers |
| 17 | P17 | Operations: status, crash reports, updates with running games | cl-mp-server | full | 1 day | P11 | rest of M9 |

Parallel start: P0, P2 (on the M1d harness), and reading/preparing for P3-P10 start at once. P1 starts when `mp/next`
exists. P3, P4 and P7 run side by side on P1; P6 and P9 meet on P5 and P7.

### P0. Refresh the `mp/*` line onto main

- **What:** one lineage holding every part once, in order: `sync-main` → M1a-1 → M1a-2 → M1a-3 → M1b → M1c → M1d →
  M1e → M3a → M6a, rebased onto current `main` with the conflicts resolved against today's code (`bridge/local.rs`,
  `session_flow.rs`, the 60 Hz logic, the comment rewrite). `mp/next` = `main` + the whole line. Each part is also a
  stacked PR (`mp/*` names) for the owner to review. The old PRs are closed with a link to the new ones. `mp/v0`,
  `mp/m0-transport`, `mp/m2-quic` and the duplicate lineage are deleted once the new line is pushed and they are
  checked to be fully contained.
- **Done when:** every part's own tests (hub and game-thread tests, `d2-quic` loopback, `app_multi_client`,
  `synthetic_multi_client`, `d2rust-server` tests) and every existing test pass on the CI server; frame hashes and
  determinism baselines are unchanged; single-player autoplay routes behave as before.
- **Risks:** M1a-2/M1a-3 move about 5k lines of the single-player assembly; redo the move on today's code rather than
  replaying old hunks. A determinism change there is a bug to fix first.

### P1. Message codec, handshake v3, control and game streams

- **What:** `d2_net::msg` (`Writer`, `Reader`, the kind table, `ErrorCode`), landed first by cl-mp-plan.
  cl-mp-server then brings the v3 handshake: `NET_VERSION` 4, ALPN `d2rust/4`, `Hello` with `game` and session resume,
  `Welcome` with the session token, `Refused` with code and text, the control stream and the game stream with
  `Attach` (contracts sections 3.1-3.4, 3.7, 3.8).
- **Done when:** round-trip and malformed-input tests for every message and every error code; a loopback test opens
  a connection, does the handshake, attaches a game stream, plays packets, leaves the game and stays connected; a v2
  `Hello` is refused with `WireVersion`.

### P2. MP test tool

- **What:** a reusable part of the suite: headless bot clients (1-64) on `d2-online`, scripted scenarios, run
  against a real `d2rust-server` process on the CI server, with a loopback UDP proxy that adds latency, loss and
  jitter. Scenarios: register and login (good and bad password, rate limit, lock), character create, select and
  delete, the difficulty rule (create and join allowed and refused by progression), public vs password game (right,
  wrong, blank password), game list updates, join and leave, playing together (move, the same monster, party, trade,
  item drop and pickup seen by both, waypoints, act change), disconnect and reconnect, server restart with
  persistence, version and game mismatch refusal, network conditions. Report: JSON and markdown, each failure with
  the broken stated rule (a line of the contracts or this plan).
- **Gate:** `tools/mp-test/gate.sh` run through `remote.sh`, required for every multiplayer PR. Scenarios for parts
  not built yet are written early and marked pending; each becomes required in the PR that builds its part.
- **Done when:** the tool runs the harness scenarios that exist today (in-process N clients) and the pending set,
  the suite docs describe it, and the gate script is in the MP PR template.

### P3. `d2rust-server` runs many games

- **What:** the process layout of contracts section 1: the QUIC endpoint, sessions, dispatch of the control stream
  to `accounts` and `lobby`, and `games` (`Games`, `GameHandle`, `GameEvent`, contracts section 6). One looped
  `GameThread` per game at 60 Hz. A game is built on its first seat at its difficulty and closes when empty (after
  held slots expire). Character saves on leave, every 60 s, on close and on shutdown, write-then-rename. A panic in a
  game closes only that game.
- **Done when:** two bots in one game see each other, a third game-less bot sees the player count change; saves
  survive a server restart (level, items, position); a forced panic in one game leaves another game running; a
  30-minute soak with 2 games of 8 bots shows no tick overrun past budget and no memory growth (in the PR).
- **Risks:** the game isn't `Send`: everything goes through game-thread jobs (the M1a-1 design). Saves must not
  stall the tick: measure.

### P4. Accounts

- **What:** contracts sections 3.5 (`Register`, `Login`, `LoggedIn`, `Logout`), 4 and 5: SQLite accounts table,
  argon2id, in-memory sessions with takeover, every rate limit and the account lock, admin commands
  (`--set-password`, `--disable`, `--enable`), backup of the database with the server's backups. Server side and the
  `d2-online` side (`account.rs`).
- **Done when:** unit tests for name and password rules, hashing, locks and limits; MP tool scenarios for register,
  login, wrong password, rate limit, lock and takeover pass; no password, hash or token in any log (a test greps the
  log of a scenario run).

### P5. Online characters

- **What:** `CharList`, `CharCreate`, `CharDelete`, `CharSelect` and `CharSummary` (contracts 3.5, 4): the
  `characters` table, the D2RC file per account (the existing `characters.rs`), the summary written after every
  game save, delete with the 7-day name hold, `--reindex`.
- **Done when:** scenarios for create, list, select, delete, a taken name (any case) and the character limit pass; a
  character made and played by one account can't be selected by another; `--reindex` rebuilds the table from the
  files.

### P6. Lobby server

- **What:** contracts 3.6 and 6: the game list with snapshot and updates, create with name, password and difficulty,
  join with the password rule, the difficulty rule (`d2_formats::d2s::status::difficulties`, shared with character
  select), hardcore and expansion kinds, dead hardcore refused, player counts, one-time tickets, chat if cheap,
  every limit of contracts section 5.
- **Done when:** scenarios for public and password games (right, wrong, blank password), create and join refused for
  a locked difficulty and allowed for an unlocked one, wrong game kind refused, list updates on create, join, leave
  and close, ticket reuse refused; the shared rule's unit tests match today's character-select tests.

### P7. `d2-online` client library and fingerprint pin

- **What:** the new crate `d2-online` (contracts section 2): connect with `ServerTrust::Pinned` by SHA-256
  fingerprint (the `d2-quic` change), the handshake, a session object with async-free polling calls for the game and
  the bots, game-stream attach returning a `Transport`, session resume after a lost connection.
- **Done when:** a wrong fingerprint is refused before any message; the bots of P2 run on it; resume within the
  grace returns to the same game slot.

### P8. The game plays on a server

- **What:** `bridge/remote.rs`: the bridge talks to the attached game stream instead of the in-process server.
  Other players show and act as the bridge models them today; display smoothing for other units' movement; cursor
  and click feedback without waiting for the round trip (no prediction of game rules); "reconnecting" during the
  grace, then "connection lost" and back to the lobby or login. Nothing of the online character is written to the
  client's save folder. The `online` cargo feature of `d2-client` pulls in `d2-online`.
- **Done when:** two game processes under Xvfb on the CI server join one game on a real server and see each other
  (offscreen captures of both in the PR); an input script moves one and the other sees it; a dropped connection
  recovers inside the grace; single-player frame hashes are unchanged.
- **Risks:** client code that still assumes it runs the server; the M1a moves should have removed it, and anything
  left shows up here.

### P9. Online screens

- **What:** the screen flow of contracts section 9, in the game's existing menu style: "Online" in the main menu;
  login (account, password, log in, create account, back); register; online character select (characters from the
  server with their look, create, delete with password confirm, back = log out); character create in online mode;
  the lobby (game list with name, difficulty, players, lock, greyed entries with the reason; create game; join game;
  chat if built; back); the create-game panel (name, password, difficulty buttons enabled from the character's
  mask); the password prompt for a locked game; every error as its code's text in the game's message box. Leaving a
  game returns to the lobby. `play --online [--server ID]` opens the login screen directly.
- **Done when:** offscreen captures of every screen and every error state in the PR; a scripted run goes login →
  create character → create game → in game → leave → lobby against a real server; the single-player menus are
  unchanged (their frame hashes).

### P10. Launcher: Play online

- **What:** in the launcher repo: the manifest's `servers` list (contracts section 8), **Play online** for a channel
  that has a server, update to the server's version first, write `servers.toml`, start `play --online --server ID`.
  `scratch/publish-dl.py` writes `servers`. Coordinated with cl-launcher-update and cl-launcher-mods (their
  STATUS.md).
- **Done when:** manifest parse and signature tests with `servers` pass and a format-1 reader ignores the key; a test
  against an in-process HTTP server starts the game with the right arguments and file; the owner sees Play online in
  a test build.

### P11. Deploy on the forge box

- **What:** contracts section 7: the server folder under `~lc/d2rust-server/`, the identity made once,
  `tools/server/deploy.sh <version>` (install, swap `current`, restart, print version and fingerprint), a systemd
  user unit with restart on crash, the nightly backup timer (14 kept), the game data the server needs (the minimum,
  read-only, never under the download folder), `status.json`. Root steps (UDP 6112 in the firewall, linger if
  needed, an optional DNS name) are written as exact commands in `tools/server/README.md` and the lane stops there
  for the admin.
- **Done when:** a deploy from scratch is one documented command; `--check` after deploy reports the right version;
  idle and loaded CPU and RAM are measured on the box and stay light; a bot from the CI server connects once the
  port is open.

### P12. Online test build and first game on two PCs; release switch ready

- **What:** a test build with `--features online` (a test channel in the manifest with the server entry and its
  fingerprint); the server deployed at the same version. Release builds keep `online` off: no "Online" button, no
  server entry for release channels. The release steps for turning it on (feature on, server entry for the release
  channel, server deployed at the release version) are written in `docs/RELEASES.md` and wait for the owner's word.
- **Done when:** the owner and a friend, on two PCs at home, log in through Play online in the test build and play
  one game together; a release build is checked to have no online code path (`cargo tree` without `d2-quic`).

### P13. Latency, loss and jitter (M5)

- **What:** play measured through the P2 proxy at 50, 100 and 200 ms round trip with loss and jitter; tune display
  smoothing and the reconnect grace.
- **Done when:** the latency scenarios pass at 100 ms with 1% loss, and the owner says it plays acceptably from home.

### P14. Security hardening (M7)

- **What:** fuzzing of `d2-proto`, the wire and the `msg` decoders; per-client rate and size limits on game messages;
  every intent validated by the server; anti-duplication tests (trade, drop and pickup races, disconnect during a
  trade); server-side item creation only; no trust in client clocks.
- **Done when:** the fuzz corpus and the abuse scenarios (floods, malformed frames, impossible intents, duplication
  attempts) are rejected and the server stays up.

### P15. More than 8 players

- **What:** measure 16, 32 and 64 bots in one game on the CI server with M1c in place. Raising `max_players` is the
  owner's call.

### P16. Mod games online

- **What:** a mod game's server is its own `d2rust-server` build on the shared crates, with the mod's rules and its
  M1e extension handlers; the handshake's `game` field keeps D2RUST and mod clients apart; the manifest lists a
  server per mod channel.

### P17. Operations (rest of M9)

- **What:** crash reports from the server, the health file watched, more than one server in the manifest, updates
  with running games (warn in the lobby and in games, save everyone, restart on the new version).

## Where it runs

- **Server:** the forge box (37.27.64.176): 20 cores, 62 GB, about 26 GB free, other game servers run there. One
  `d2rust-server` process, one UDP port, a small footprint (contracts section 7). Finland, so about 20-60 ms for
  players in Europe.
- **Tests and builds:** the CI server (CI-SERVER) through `remote.sh`; game data at `/work/game`. The MP test
  tool runs there against a loopback server.

## Security

TLS 1.3 over QUIC with the server certificate pinned by its SHA-256 from the signed manifest; passwords hashed with
argon2id; rate limits on login, register, join and chat; game passwords in memory only; intents only from clients
and strict decode; one UDP port; the server runs as its own user with read-only game data; no secrets in repos,
the manifest, logs or crash reports. Details in contracts section 5; fuzzing and abuse tests in P14.

## Not planned

- Player hosting, listen servers, peer-to-peer, NAT traversal or relays (owner rule: server-hosted only).
- TCP or WebSocket fallback (owner rule: QUIC only).
- Bringing local characters online (owner rule).
- Email, password reset by mail (admin command instead), ladder, seasons, trading markets.
- Cross-version play: client and server must match exactly.
- Client-side prediction of game rules.
