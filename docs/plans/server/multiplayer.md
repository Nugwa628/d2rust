---
id: multiplayer
title: Multiplayer v0
area: server
status: in-progress
order: 4
modules: M0-M9
light: M0-M4
depends_on: []
---

# Plan: Multiplayer v0

Status: **in progress: the refreshed line is on `mp/next` (stacked PRs #449–#458), not built into releases yet** (owner, 2026-10-17). It's built with
`docs/FEATURE-METHOD.md`. Multiplayer modules merge into `main` like other work, so they don't drift from it.
Release builds leave them out (behind a cargo feature that release builds don't enable, or in crates the game
exe doesn't depend on) until the owner says to ship multiplayer. The `mp/v0` branch will be brought into `main`
once by #165 and then retired. This replaces the 2026-10-16 rule "kept on `mp/v0`, not merged into `main`".

## Goal

Several players play one D2RUST game together over the internet, on one authoritative
dedicated server, with the same rules and behaviour as single player.

v0 is the smallest version that proves this: up to eight players join one game on a server by
address, play through it together with characters stored on that server, and leave. Accounts,
a game list and a lobby come after.

## Owner decisions (2026-10-16)

- **Authoritative server.** Clients send intents only; the server decides everything (as `docs/ARCHITECTURE.md` already requires). Client reports and messages from other players are untrusted input.
- **Server-hosted only.** Online games run on a dedicated server (`d2rust-server`). There is no player hosting and no listen server: a client never hosts a game for others.
- **One code path.** Single player stays "the same server, in-process" over the in-memory transport; there's no separate offline game.
- **QUIC for every connection.** The network transport is QUIC (the `quinn` crate), the same for a server reached by IP address and for an official server. Connection migration (for example Wi-Fi to mobile data) is QUIC's job; the game adds a reconnect grace period on top.
- **Server-only characters.** Online characters are created on and kept by the server. Local (single-player) characters can't go online, and online characters are never written to the client.
- **Full pass (owner, 2026-10-17).** Every module, M0-M9, is built now, not just the light tier. The modules are cheap to write and are easy to change later, when they can be tested for real and stress-tested.
- **No TCP (owner, 2026-10-17).** QUIC only; no TCP transport or fallback anywhere.
- **Player count.** At most **8 players per game** now. A later raise to **64** must be a config and limit change, not a redesign (see "Scaling to 64").

## Owner decisions (2026-10-17)

- **One multiplayer architecture for D2RUST and every mod game.** A mod game is its own game, but it runs on the same server, sessions and protocol framework as D2RUST. A mod adds its own messages through an extension channel (M1e) and never forks the netcode: transport, handshake, sessions, join and leave stay shared.
- **Full pass.** Every module, M0 to M9, is in scope; #165 integrates `mp/v0`, then module PRs target main: modules are cheap to write and are changed later, when they can be tested for real. One PR per module, in plan order.
- **QUIC only.** QUIC is the only network transport. The TCP fallback M2 added is removed by the open integration PR #165.

## Where things are today

- **Design:** single player is `d2-server` running in-process, connected through the same `d2-proto` messages over an in-memory channel. Online is the same server, remote (`docs/ARCHITECTURE.md`).
- **`d2-proto`:** complete for one client. Every message id and layout in both directions is generated from `tables/sim/client-messages.tsv` and `server-messages.tsv`.
- **`d2-server`:**
  - `host.rs` runs the tick from an injected clock, drains the client-to-server queues and flushes server-to-client buffers;
  - `transport.rs` and `buffers.rs` already key queues and buffers by `ClientId`;
  - accounts and character storage are not there yet.
- **`d2-net`:** a stub on `main`. #165 brings the transport seam and in-memory transport (M0) into main, with QUIC isolated in `d2-quic` (M2). #128/#133 merged into `mp/v0`, not main.
- **`d2-verify`:** a stub (the ownership check: the player has a real Diablo II install).
- **Client:** the bridge mirrors server state and doesn't predict.

## Modules

| Rank | Id | Module | Tier | Size | Depends on | Status |
|---|---|---|---|---|---|---|
| 0 | M0 | Transport seam in `d2-net` | light | 3–4 h | none | on `mp/next`: #449 (merged on mp/v0: #128) |
| 1 | M1 | Several clients in one game, in-process (M1a–M1e below) | light | 2–3 days | M0 | on `mp/next`: #450 (M1b), #451 (M1d), #452–#454 (M1a), #455 (M1c), #456 (M1e) |
| 2 | M2 | QUIC transport, handshake and reconnect | light | 1–2 days | M0 | on `mp/next`: #449 (merged on mp/v0: #133; TCP removed) |
| 3 | M3 | Dedicated server `d2rust-server` with server-side characters | light | 1 day | M1, M2 | on `mp/next`: #457 (M3a; game serving remains open), #458 (M6a backups) |
| 4 | M4 | Join a server by address, in the client | light | 3–4 h | M3 | open |
| 5 | M5 | Latency and loss handling | full | 1–2 days | M4 | open |
| 6 | M6 | Accounts and login | full | 2 days | M3 | open |
| 7 | M7 | Security hardening | full | 2–3 days | M2, M6 | open |
| 8 | M8 | Game list and lobby | full | 1–2 days | M6 | open |
| 9 | M9 | Official server operations and launcher integration | full | 1–2 days | M3, M8 | open |

Light tier (M0–M4) is v0: two to eight players on separate PCs join one game on a dedicated
server by address and play together. The dedicated server (M3) is the main target; the client side
(M4) is a thin "connect and play" on top. The full tier (M5–M9) is built in the same pass (owner, 2026-10-17).

### M0. Transport seam in `d2-net`

- **What:** one `Transport` trait for sending and receiving `d2-proto` messages per client, with the current in-memory channel as its first implementation.
- **Seam:** `d2-server`'s host and `d2-client`'s bridge talk through the trait instead of calling each other's queues.
- **Done when:** single player behaves identically (every existing test, autoplay route and frame hash unchanged).

### M1. Several clients in one game, in-process

- **What:** the host runs one game with up to `max_players` clients (8 by default), each with its own intents, buffers and view.
- **Includes:**
  - join and leave mid-game;
  - per-player state (party, trade, waypoints, quest state per player);
  - deterministic intent order per tick: by `ClientId`, then arrival.
- **Seam:** `d2-server` host and dispatch, plus a test harness that drives N in-process clients.
- **Done when:**
  - two and eight scripted clients play one game in-process, and a test with a raised limit (for example 64) passes too;
  - replays with the same intents give the same game (determinism test);
  - single player is unchanged.
- **Risks:** this is where most hidden "one player" assumptions in `d2-sim` and the server show up. M1 starts with a study that lists them, and the size is revised after it.
- **Split after the study (2026-10-16).** The engine core is mostly ready for N players (client records are a list, player state is keyed by unit id, players are 32-bit ids on the wire). The one-player wiring sits mostly in the client's single-player assembly. Parties and player trade move to the full tier.

| Id | Part | Status |
|---|---|---|
| M1a | The server assembly moves from `d2-client` into `d2-server`, with a game built on its own thread and a server-owned frame loop | on `mp/next`: #452 (M1a-1), #454 (M1a-3: game builder, live data, file source) |
| M1a-2 | Move LocalSeams, NPC/vendor/hireling/quest rests, skill state and equipment snapshots into `d2-server`; keep client projections and animation lookup adapters | on `mp/next`: #453 |
| M1b | Join and leave per client: the game's own settings for every joiner, full teardown of a leaving player (items, mercenary, summons, portal), a character per client, the player count for monster scaling, a lost connection leaves like a leave | on `mp/next`: #450 |
| M1c | Monster target lists for more than 8 players: the first eight keep their lists, later players get lists past them | on `mp/next`: #455 |
| M1d | N-client harness and replay determinism, including a run at a raised limit of 64 | on `mp/next`: #451 |
| M1e | Extension channel: a mod game's own messages ride a framed channel in the shared protocol, routed to the mod's server and client handlers, with no change to the netcode | on `mp/next`: #456 |

### M2. QUIC transport, handshake and reconnect

- **What:** `d2-proto` messages over QUIC (`quinn`), as a second `Transport` in `d2-net`: framing on a QUIC stream, a versioned handshake, keepalive, idle timeouts and clean disconnects.
- **Handshake:**
  - protocol version, game build, and the tick rate as a value (not a constant), so a change of tick rate needs no protocol change;
  - the client must match the server's protocol version and build exactly in v0;
  - the server sends the client its `ClientId` and the game's player limit.
- **Connection migration:** a client whose address changes (Wi-Fi to mobile data, NAT rebinding) keeps its QUIC connection; the game doesn't notice.
- **Reconnect grace period:** if a connection is lost anyway, the player's slot and character stay in the game for a configured time (default 30 s). A client that reconnects within it with its reconnect token gets the same slot back; after it the player leaves the game normally.
- **No TCP (owner, 2026-10-17):** QUIC is the only network transport. There is no TCP implementation or fallback; the one M2 added is removed.
- **Done when:** the M1 multi-client tests pass over loopback QUIC; a disconnect mid-game removes the player cleanly after the grace period; a reconnect inside it restores the slot.

### M3. Dedicated server `d2rust-server` with server-side characters

- **What:** `d2rust-server`, a headless binary that runs games with no window, configured by a file (listen address and port, max players per game, game data path, character folder, log folder, reconnect grace).
- **Characters:**
  - characters are created on the server and stored in its character folder, in D2RUST's own versioned format (`docs/ARCHITECTURE.md` "Persistence");
  - in v0 a player picks a character by name on the server (no password; accounts are M6);
  - the server saves at safe points and when a player leaves; local characters are never uploaded.
- **Notes:**
  - the server needs game data to run the rules, so it reads a Diablo II install on the server machine;
  - nothing from it is ever served to clients.
- **Done when:** it runs on Windows and Linux, survives a 1-hour soak with scripted clients over QUIC, keeps characters across a restart, and logs to files.

### M4. Join a server by address, in the client

- **What:** in the main menu, "Join game": a server address and port, then a character on that server (create or pick). In-game, other players appear and act.
- **Not included:** hosting a game from the client.
- **Done when:** two people on two PCs (LAN, then internet) play one game together on a dedicated server. The owner tests it.

### M5. Latency and loss handling

- **What:** measured play at 50, 100 and 200 ms round-trip with some loss.
- **Includes:**
  - input feedback that doesn't wait for the round trip where that's safe (cursor, click markers);
  - display-side smoothing tuned to absorb jitter;
  - no client-side prediction of game rules in v0.
- **Done when:** it plays acceptably at 100 ms (owner tests), and a latency test with network emulation passes.

### M6. Accounts and login

- **What:** accounts on the server (name and password hash, or a launcher-issued token); characters belong to an account.
- **Includes:** create, list and delete characters per account; backups of the character folder.
- **Done when:** a player can only see and play their own characters, and characters survive a server restart and a crash with at most the last few seconds lost.

### M7. Security hardening

- **What:**
  - every client message is decoded strictly (fuzz `d2-proto` decode);
  - every intent is validated by the server;
  - rate limits and size limits;
  - items are server-authoritative, with no client-side item creation;
  - anti-duplication tests;
  - login brute-force limits;
  - the server never trusts client clocks.
- **Done when:** a fuzz corpus and abuse tests (floods, malformed packets, impossible intents, duplication attempts) are all rejected, and the server stays up.

### M8. Game list and lobby

- **What:** create a game on the server (name, optional password, difficulty), list open games, join from the list, and simple chat.
- **Done when:** players find and join each other's games without typing addresses.

### M9. Official server operations and launcher integration

- **What:**
  - where official servers run;
  - deploy and update steps, logs, crash reports (with the bug-report service, `launcher/bug-reports.md` B2);
  - the launcher shows servers and passes login to the game.
- **Done when:** a release can be deployed to a server with one documented command, and players connect from the launcher.

## Scaling to 64

The limit is 8 per game now. Everything is built so that 64 is a config and limit change:

- the player limit is a server config value (`max_players`), sent in the handshake; no protocol message, table or array has a fixed 8-player size;
- `ClientId` and player ids on the wire are at least 16 bits wide; player lists in messages carry a count;
- per-player state is kept in maps or vectors keyed by id, never in fixed-size slot arrays;
- per-tick work that grows with players (visibility, broadcasts) is measured in M1 and M3 tests at 64 scripted clients, so the cost is known before the limit is raised;
- game rules that depend on the player count (for example monster scaling) read the count, not a constant.

## Not planned in v0

- Player-hosted games, listen servers, or peer-to-peer games.
- Bringing local characters online.
- Ladder, seasons, trading markets.
- Cross-version play (client and server must match).
- Client-side prediction of game rules.
