---
id: multiplayer-contracts
title: Multiplayer contracts, module split and interfaces
area: server
status: in-progress
order: 4
depends_on: [multiplayer, multiplayer-v1]
---

# Multiplayer contracts

Status: **CONTRACTS READY v13** (2026-10-20). This page fixes the interfaces every multiplayer lane builds against:
who owns which files, the wire messages, the account and character model, the difficulty rule, the server process
layout, ports, certificate pinning and how the launcher starts online play. The modules and their order are in
[`multiplayer-v1.md`](multiplayer-v1.md); the engine background is in [`multiplayer.md`](multiplayer.md).

A lane that needs a contract changed writes it under "Requests" in `E:/work/lanes/cl-mp-plan/STATUS.md` and goes on
with the rest. Only cl-mp-plan edits this page; every change gets a new contract version (the "Changes" list at the
end) and a note in that STATUS.md.

## Owner decisions this page builds on (2026-10-19)

1. The multiplayer server runs on the forge box (37.27.64.176). It may hold the game data the server needs. Root
   steps there (firewall, system services) are the admin's: lanes write the exact commands and stop.
2. **Updated by the owner (2026-10-19 evening):** multiplayer is not released until it is fleshed out and fully
   tested. Online play stays behind a build switch (the `online` cargo feature) that is **off in release builds**;
   the owner decides when it turns on. Game bug lanes and test tools have priority on the CI server: multiplayer
   builds run at a lower CPU priority and don't take CI or merge capacity from them.
3. Accounts (register, login) come before the first game together. Characters belong to an account and are stored
   on the server only.
4. The player picks an online character before the lobby. That character's progression decides which difficulties
   it can create and join. Full lobby UI: login and register, character select and create, game list, create game,
   join game, password prompt, chat if cheap, back out, errors that say what went wrong.
5. A game with a blank password is public; a game with a password needs it to join.
6. Rules kept from `multiplayer.md`: authoritative server, server-hosted only, QUIC only, online characters never
   written to the client, 8 players per game (64 is a config change), one architecture for D2RUST and every mod game.

## 1. Server process layout

**One service process, `d2rust-server`, holds accounts, the lobby and every running game.** It listens on one UDP
port with one QUIC endpoint.

```
d2rust-server (one process, one UDP port 6112)
├── net        QUIC endpoint, connections, sessions, control-stream dispatch      (tokio)
├── accounts   register, login, sessions, online characters, SQLite               (tokio + spawn_blocking)
├── lobby      game list, create, join, password and difficulty rules, chat       (tokio, in memory)
└── games      one GameThread per running game (own OS thread, the game is not Send)
               ├── game 1  (looped GameThread + Hub, 60 Hz)
               └── game 2  ...
```

Why one process and not split services:

- **One box, stay light.** The forge box runs other game servers and has ~26 GB free. One process means one
  binary, one config, one systemd unit, one log folder and one backup job.
- **One port to open.** The admin opens one UDP port. A split would need a second port or local IPC.
- **The lobby needs live game state.** Player counts, "game closing" and "character in a game" are shared memory
  in one process. Split services would need a protocol between them for no gain at this size.
- **Crash isolation where it matters.** Each game runs on its own `GameThread`. A panic in a game is caught at the
  thread boundary (`catch_unwind`): that game closes, its players go back to the lobby with error 70
  (`GameClosed`), the characters keep their last save, and the other games and the lobby keep running. A panic in
  `net`, `accounts` or `lobby` ends the process, and systemd restarts it.
- **Split later is possible.** `lobby` talks to `games` only through `GameHandle` / `GameEvent` (section 6). Those
  can become a socket protocol if games ever move to other boxes.

Threads: `net`, `accounts` and `lobby` run on one tokio runtime, 2 worker threads by default (config
`net_threads`). Password hashing runs on `spawn_blocking`, at most 2 at a time (a semaphore). Each game has its own
thread at the 60 Hz server tick.

## 2. Module split: which lane owns what

Every multiplayer PR targets `mp/next` (or a stack on it), branch `mp/<lane>-<part>`. A file has one owner. Another
lane that needs a change there asks the owner in their STATUS.md, or asks cl-mp-plan under "Requests".

| Area | Files (new unless noted) | Owner lane |
|---|---|---|
| The refreshed `mp/*` line (wire v2, `d2-quic`, hub, GameThread, M1a-M1e, M3a, M6a) | existing branches | cl-mp-refresh |
| Message codec and kind table | `crates/d2-net/src/msg/mod.rs` (`Writer`, `Reader`, `kind`, `ErrorCode`) | cl-mp-plan (lands first, section 3) |
| Handshake v3, game-stream attach | `crates/d2-net/src/wire.rs` (exists) | cl-mp-server |
| Auth and character messages | `crates/d2-net/src/msg/account.rs` | cl-mp-accounts |
| Lobby messages (game list, create, join, chat) | `crates/d2-net/src/msg/lobby.rs` | cl-mp-lobby |
| QUIC stream layer for both sides (section 3.9): `FramedStream`, `PacketStream`, `QuicClient`, `QuicServer` bind/accept, `QuicConn`, `ServerTrust` fingerprint pin, `Transport::flush` | `crates/d2-quic/src/lib.rs`, `crates/d2-net/src/lib.rs` (`flush` only) | cl-mp-client (one PR, first) |
| QUIC accept policy, retry, persistent identity, on top of the stream layer | `crates/d2-quic/src/server.rs` (new), `Identity` | cl-mp-server |
| Server: endpoint, sessions, dispatch, config, deploy | `crates/d2rust-server/src/{main,net,session,config,status}.rs`, `tools/server/` | cl-mp-server |
| Server: running games, lifecycle, saves | `crates/d2rust-server/src/games/` | cl-mp-server |
| Server: accounts, characters, SQLite, rate limits | `crates/d2rust-server/src/accounts/`, `characters.rs` (exists) | cl-mp-accounts |
| Server: lobby rules | `crates/d2rust-server/src/lobby/` | cl-mp-lobby |
| Difficulty rule (shared by client and server) | `crates/d2-formats/src/d2s.rs` `status::difficulties` | cl-mp-lobby |
| Client session library (used by the game and the test bots) | new crate `crates/d2-online/`: `lib.rs`, `connect.rs`, `game.rs` | cl-mp-client |
| | `crates/d2-online/src/account.rs` | cl-mp-accounts |
| | `crates/d2-online/src/lobby.rs` | cl-mp-lobby |
| Game client: remote bridge, reconnect, `--online` args, `servers.toml` | `crates/d2-client/src/bridge/remote.rs`, `launch.rs`, `app/server_thread.rs` | cl-mp-client |
| Game client: online screens (login, register, online char select/create, lobby, create, join, password, errors) | `crates/d2-client/src/ui/front_end/screens/online/` | cl-mp-lobby |
| Main menu "Online" button | `crates/d2-client/src/ui/front_end/screens/main_menu.rs` | cl-mp-lobby |
| Launcher "Play online", manifest `servers` | launcher repo `launcher-core/src/{manifest,launch,online}.rs`, `launcher/src/app.rs`; `scratch/publish-dl.py` | cl-mp-client |
| MP test tool (bots, scenarios, network faults, report, gate script) | `crates/mp-test/`, `tools/mp-test/`, suite docs | cl-mp-testtool |

`d2-online` is a library with no game code. It wraps `d2-quic` and the messages, so the game client and the test
tool's bots share one client implementation. The game exe depends on `d2-online` behind the cargo feature `online`.

## 3. Wire format

### 3.1 Connection and streams

- **Transport:** QUIC (`quinn`), TLS 1.3, ALPN `d2rust/4`, server name `d2rust` (as now). The client pins the
  server certificate (section 8). QUIC address validation (retry) is on.
- **Control stream:** the first bidirectional stream, opened by the client. It carries the handshake, then every
  account, character and lobby message for the whole session.
- **Game stream:** a second bidirectional stream the client opens after `JoinOk`. It starts with `Attach`, then
  carries the game packets exactly as wire v2 does today (`SEND`, `DELIVER`, `FRAME`, `PING`, `BYE`). Leaving the
  game ends the game stream; the control stream stays open and the player is back in the lobby. At most one game
  stream per connection.
- **Frames:** unchanged: a 4-byte little-endian body length, then the body. The first body byte is the kind.
  `MAX_FRAME` stays 0x1000. Lists that don't fit are sent as several messages (game list: `GameListBegin`, entries,
  `GameListEnd`).

### 3.2 Field encoding (`d2_net::msg`)

All integers little-endian. `str8` is a u8 byte length then UTF-8 (no NUL, no control characters); a decoder
refuses invalid UTF-8, control characters and over-long strings with `NetError::Malformed`. `bytes8` is a u8 length
then raw bytes. `bool` is one byte, 0 or 1 (anything else is malformed). `opt<T>` is one byte 0, or 1 then `T`.
A message with bytes left over after its last field is malformed.

`d2_net::msg::Writer` (`u8`, `u16`, `u32`, `u64`, `bool`, `str8`, `bytes8`, `opt`) and `Reader` (the same, each
returning `Result<_, NetError>`, plus `finish()` that fails on trailing bytes) are the only codec. Message files
implement `fn encode(&self, w: &mut Writer)` and `fn decode(r: &mut Reader) -> Result<Self, NetError>` per message,
plus one enum per direction with `fn body(&self) -> Vec<u8>` and `fn parse(body: &[u8]) -> Result<Self, NetError>`.

### 3.3 Kind table

| Range | Stream | Owner | Kinds |
|---|---|---|---|
| 0x01-0x08 | both | wire v2 (exists) | `HELLO` 1, `WELCOME` 2, `REFUSED` 3, `SEND` 4, `DELIVER` 5, `FRAME` 6, `PING` 7, `BYE` 8 |
| 0x09 | game | M1e (exists on the refreshed line) | `EXT` 9 (mod extension messages) |
| 0x0A | game | cl-mp-server | `ATTACH` 0x0A |
| 0x0B | game | cl-mp-client | `FRAME_END` 0x0B (S→C, empty: the frame's deliveries are complete, section 3.7) |
| 0x10-0x1F | control | cl-mp-plan | `ERROR` 0x10 (S→C: answer to any request that failed) |
| 0x20-0x3F | control | cl-mp-accounts | account and character messages (3.5) |
| 0x40-0x5F | control | cl-mp-lobby | lobby messages (3.6) |
| 0x60-0x6F | control | free | reserved for mod games' lobby extensions |
| 0x70-0xFF | | | reserved |

`PING` and `BYE` are valid on both streams. `BYE` on the control stream ends the session (logout).

### 3.4 Handshake v3 (control stream)

`NET_VERSION` becomes **4** (3 is taken by the refreshed line's M1e wire with the old `Hello`). An older client
is refused with `WireVersion`, and the client shows "update the game".

`Hello` (C→S, kind 1): `MAGIC`, `u16 net_version`, then

| Field | Type | Notes |
|---|---|---|
| `protocol` | u32 | `d2_proto::PROTOCOL_VERSION` |
| `game` | str8 | `d2rust`, or a mod game's id |
| `build` | str8 | the four-part game version, for example `0.1.2.0` |
| `tick_rate` | u32 | ticks per second the client was built for |
| `resume` | opt<[u8;16]> | the session token of a lost connection, else none |

`Welcome` (S→C, kind 2) on the control stream: `u32 tick_rate`, `u32 max_players`, `[u8;16] session` (the session
resume token, a secret), `str8 server_name`, `bool registration_open`, `opt<ResumeState>`. `ResumeState` is present
only when `resume` named a live session: `str8 account`, `opt<str8> character`, `opt<u32 game_id>` (a game slot still
held for this session; the client re-attaches with `Attach { resume }`).

`Refused` (S→C, kind 3): `u16 code`, `str8 text` (section 3.7). The server closes the connection after it.

Checks, in order: magic and net version, protocol, game id, build (exact), tick rate, server full (`max_sessions`).

### 3.5 Account and character messages (cl-mp-accounts, 0x20-0x3F)

All need a handshake done. Every request but `Register` and `Login` needs a logged-in session (else
`ERROR NotLoggedIn`). Every failure is answered with `ERROR` (kind 0x10: `u8 request_kind`, `u16 code`,
`u32 wait_s`, `str8 text`). `wait_s` is 0 unless the code is a rate limit or lock.

| Kind | Dir | Message | Fields |
|---|---|---|---|
| 0x20 | C→S | `Register` | `str8 account`, `str8 password` |
| 0x21 | C→S | `Login` | `str8 account`, `str8 password` |
| 0x22 | S→C | `LoggedIn` | `str8 account` (as stored, with its case), `u8 max_characters` |
| 0x23 | C→S | `Logout` | (empty); the server answers with `BYE` |
| 0x24 | C→S | `CharList` | (empty) |
| 0x25 | S→C | `CharListAnswer` | `u8 count`, then `count` × `CharSummary` |
| 0x26 | C→S | `CharCreate` | `str8 name`, `u8 class` (0-6), `bool hardcore`, `bool expansion` |
| 0x27 | S→C | `CharCreated` | `CharSummary` |
| 0x28 | C→S | `CharDelete` | `str8 name`, `str8 password` (the account password, to confirm) |
| 0x29 | S→C | `CharDeleted` | `str8 name` |
| 0x2A | C→S | `CharSelect` | `str8 name` |
| 0x2B | S→C | `CharSelected` | `CharSummary` (the lobby uses its `difficulties`) |

`CharSummary`: `str8 name`, `u8 class`, `u8 level`, `u16 status` (the save status word: hardcore, dead, expansion,
progression), `u8 difficulties` (bit 0 Normal, bit 1 Nightmare, bit 2 Hell: `status::difficulties(status)`),
`bytes8 look` (the save header's appearance bytes, for the character-select picture), `u64 last_played` (Unix
seconds, 0 if never).

`Register` logs the new account in at once (answer `LoggedIn`). A new login to an account that already has a
session **takes over**: the old session's game character is saved, the old connection gets `ERROR SessionTaken` and
is closed.

### 3.6 Lobby messages (cl-mp-lobby, 0x40-0x5F)

All need a selected character (else `ERROR NoCharSelected`).

| Kind | Dir | Message | Fields |
|---|---|---|---|
| 0x40 | C→S | `ListGames` | (empty): subscribe to the list; the server sends a snapshot, then updates while the client is in the lobby |
| 0x41 | S→C | `GameListBegin` | `u16 count` |
| 0x42 | S→C | `GameEntry` | `GameInfo` (in a snapshot, and as an update for a new or changed game) |
| 0x43 | S→C | `GameListEnd` | (empty) |
| 0x44 | S→C | `GameGone` | `u32 game_id` |
| 0x45 | C→S | `CreateGame` | `str8 name`, `str8 password` (empty = public), `u8 difficulty` (0 Normal, 1 Nightmare, 2 Hell), `u8 max_players` (0 = server default) |
| 0x46 | C→S | `JoinGame` | `u32 game_id`, `str8 password` (empty for a public game) |
| 0x47 | S→C | `JoinOk` | `u32 game_id`, `[u8;16] ticket`, `u8 difficulty` |
| 0x48 | C→S | `LeaveLobby` | (empty): stop list updates (sent when entering a game and on back-out to character select) |
| 0x49 | C→S | `Chat` | `str8 text` (1-200 bytes) |
| 0x4A | S→C | `ChatLine` | `str8 from` (character name), `str8 text`, `u64 at` (Unix seconds) |

`GameInfo`: `u32 game_id`, `str8 name`, `u8 difficulty`, `u8 players`, `u8 max_players`, `bool locked` (has a
password), `bool hardcore`, `bool expansion`, `u16 joinable` (0 = the selected character can join; else the error
code why not, so the list greys it out with a reason).

The list shows every open game of the same hardcore and expansion kind as the selected character, including games
it can't join (with `joinable` set). It never sends passwords. A game's hardcore and expansion kind is the creator's
character's.

Chat (if the lobby lane finds it cheap): one lobby-wide channel for every session in the lobby, no history, the
rate limit in section 5. If it isn't built, the server answers `Chat` with `ERROR NotSupported` and the UI hides the
chat box.

### 3.7 Game stream

`Attach` (C→S, kind 0x0A, the first frame of the game stream): `u8 mode` (0 = ticket, 1 = resume), then `[u8;16]`
(the `JoinOk` ticket, or the game resume token from that game's `Welcome`). The server answers on the game stream
with wire v2's `Welcome` (`client`, `tick_rate`, `max_players`, game `resume` token) or `Refused`. A ticket is valid
once, for 30 s, and only on the connection of the session it was given to.

After `Welcome` the game stream is wire v2 unchanged. The game takes the character from the ticket (account,
character name). The character name and class in the client's 0x67 join must match it, or the join is refused (code
45 `CharMismatch`). The client never sends a character record.

**The seat filter (v8).** On a game stream the server rewrites the client's 0x67 game join from the seat before the
game reads it: difficulty, game kind (hardcore, expansion), game name, the stored character name, and the class of a
new character. Whatever the client put there is ignored. It drops every 0x6C (character save upload): an online
character's record only ever comes from the server's store.

**Resume is a re-entry (v4).** Game data written into a connection that then dies is lost without the server
knowing (up to the idle timeout), so replaying the hub's parked backlog can leave the client out of step with no way
to notice. So `Attach` mode 1 doesn't replay a backlog: the server puts the player back into the game as on a join,
keeping its slot, its player unit, its character state in memory and its position (nothing is reloaded from the
file), and sends the full game state again, as the join path does for a new player. The client clears its bridge
world before it applies that state. The resumed `Welcome` has the same `client` id and a new game resume token.
Server side: cl-mp-server (the game's join-state path, used for an existing seat); client side: cl-mp-client. A QUIC
connection migration (network change without a lost connection) needs none of this. The hub's backlog replay stays
only where no data can be lost (the memory transport), or is removed by cl-mp-server.

What a re-entry resends (v12). Lost data can touch anything the server tells the client, so the resend covers
everything the client clears. The client clears its whole bridge world: rooms, every unit (its own player too),
ground and owned items, the mercenary, other players, party and trade state, and its own character state (stats,
skills, inventory, belt, stash, quests, waypoints). The server then sends the same messages a fresh join sends for
an existing character, built from the live game: the player unit with its position, its items, stats and skills,
quest and waypoint state, its mercenary, the party and relation state of every other player, the rooms in view
and their units. It does not reload anything from the file, and it does not repeat one-off join effects (no new
character setup, no "joined the game" message to the others). An open trade or an open store window is closed for
both sides before the resend. Checked by a test in which a bot drops and resumes while another never drops: after
the resume, both hold the same view of the game.

**Frame order on the game stream (v2).** The hub keeps today's order (`poll` → host frame → `FRAME { ticked }` mark
→ `deliver_all`), which the single-player link relies on. `d2_net::Transport` gains `fn flush(&mut self) ->
Result<(), NetError>` with a default that does nothing; the hub calls it for every client after `deliver_all`. The
memory transport keeps the default, so single player is unchanged. The QUIC game stream's `flush` writes
`FRAME_END` and pushes the frame's bytes in one write. So on the wire one server frame is `FRAME` N, its `DELIVER`s,
`FRAME_END`; the client knows frame N is whole when its `FRAME_END` arrives, without waiting for frame N+1.
The client may hold a small buffer of whole frames to absorb network jitter (v10): it starts at 2 ticks, never
holds more than 8, and shrinks back when the network is steady. Its added delay is measured in the latency tests
(plan P13), which may lower these numbers. On a resume the buffer is emptied with the bridge world.

**Shop prices online (v2).** The client computes the prices it shows from its own game data (the handshake
guarantees the same build), the same way the in-process server computes `Started::prices`. They are display only:
the server prices every buy, sell and repair itself, and refuses one the player can't pay.

### 3.9 QUIC stream layer (`d2-quic`, v2)

`d2-quic` becomes a stream layer only; it no longer contains a handshake. The handshake bodies are encoded by
`d2_net::wire` / `d2_net::msg` and driven by `d2-online` (client) and `d2rust-server` `net.rs` (server).

```rust
pub enum ServerTrust { Any, Pinned([u8; 32]) }      // SHA-256 of the certificate DER (section 8)
pub fn fingerprint(cert_der: &[u8]) -> [u8; 32];
pub struct FramedStream;   // one bidi stream with wire framing; channel-backed, so the calls never block
impl FramedStream {
    pub fn send(&mut self, body: Vec<u8>) -> Result<(), NetError>;
    pub fn recv(&mut self) -> Result<Option<Vec<u8>>, NetError>;            // PING eaten, BYE -> Closed
    pub fn recv_timeout(&mut self, d: Duration) -> Result<Option<Vec<u8>>, NetError>;
    pub async fn recv_async(&mut self) -> Result<Vec<u8>, NetError>;        // for the server's tokio loop
    pub fn close(&mut self);                                                // BYE, then finish the stream
}
pub struct PacketStream;   // a FramedStream as a d2_net::Transport (wire v2 packets + FRAME_END on flush)
pub struct QuicClient;     // endpoint, its own small runtime, the connection
impl QuicClient {
    pub fn connect(addr: SocketAddr, trust: ServerTrust, timing: Timing) -> Result<(QuicClient, FramedStream), ConnectError>;
    pub fn open_stream(&self) -> Result<FramedStream, ConnectError>;         // the game stream
    pub fn rebind(&self, local: SocketAddr) -> Result<(), ConnectError>;
    pub fn close(&mut self);
}
pub struct QuicServer;     // bind(addr, Identity, Timing, runtime: tokio::runtime::Handle): runs on the server's runtime
impl QuicServer {
    pub fn local_addr(&self) -> io::Result<SocketAddr>;
    pub fn cert(&self) -> &[u8];
    pub async fn accept(&self) -> Option<(QuicConn, FramedStream)>;          // a connection and its control stream
}
pub struct QuicConn;
impl QuicConn {
    pub fn peer(&self) -> SocketAddr;
    pub async fn accept_stream(&self) -> Option<FramedStream>;              // the game stream
    pub fn close(&self, code: u32, reason: &str);
}
```

The test tool's bots and the game both use `QuicClient` through `d2-online`. Small changes to these signatures
during the PR are fine if both cl-mp-client and cl-mp-server agree in their STATUS.md; the shape (control stream +
game stream, fingerprint pin, no handshake inside) is fixed.

### 3.8 Error codes (`d2_net::msg::ErrorCode`, u16)

The server always sends English `text` with the code. The client shows its own string for a known code, and the
server's text for an unknown one. Codes never change meaning; new ones are added at the end of their block.

| Code | Name | When |
|---|---|---|
| 1 | `WireVersion` | not D2RUST, or another wire version |
| 2 | `Protocol` | protocol version differs |
| 3 | `Build` | game version differs: "The server runs version X; you have Y. Update through the launcher." |
| 4 | `TickRate` | tick rate differs |
| 5 | `WrongGame` | the server runs another game (D2RUST vs a mod) |
| 6 | `ServerFull` | no free session |
| 7 | `UnknownResume` | the session or game slot is gone |
| 8 | `NotSupported` | a message the server doesn't handle (yet) |
| 9 | `Malformed` | a message didn't decode; the connection is closed |
| 20 | `AccountNameTaken` | |
| 21 | `BadAccountName` | text says which rule |
| 22 | `BadPassword` | too short or too long (on register) |
| 23 | `WrongLogin` | unknown account or wrong password (one message for both) |
| 24 | `RateLimited` | too many attempts; `wait_s` says how long |
| 25 | `AccountLocked` | too many failed logins; `wait_s` |
| 26 | `AccountDisabled` | by the admin |
| 27 | `NotLoggedIn` | |
| 28 | `RegistrationClosed` | config `registration = closed` |
| 29 | `SessionTaken` | someone logged in to this account elsewhere |
| 40 | `CharNameTaken` | names are unique on the server, any case |
| 41 | `BadCharName` | text says which rule |
| 42 | `CharLimit` | the account has `max_characters` |
| 43 | `UnknownChar` | not this account's character |
| 44 | `CharInGame` | the character is in a game (or its slot is held) |
| 45 | `CharMismatch` | the 0x67 join doesn't match the ticket's character |
| 46 | `DeadHardcore` | "You cannot create or join games with a dead hardcore character." |
| 47 | `NoCharSelected` | |
| 48 | `ExpansionClass` | a classic character of an expansion-only class (Druid, Assassin) (v8) |
| 60 | `GameNameTaken` | among open games, any case |
| 61 | `BadGameName` | |
| 62 | `GameNotFound` | closed or never existed |
| 63 | `GameFull` | |
| 64 | `PasswordNeeded` | joining a locked game with an empty password |
| 65 | `WrongPassword` | |
| 66 | `DifficultyLocked` | the character hasn't unlocked that difficulty |
| 67 | `WrongGameKind` | hardcore vs softcore, or expansion vs classic |
| 68 | `TooManyGames` | the server runs `max_games` |
| 69 | `AlreadyInGame` | the session is in a game |
| 70 | `GameClosed` | the game closed (empty, crashed or server shutdown) |
| 71 | `TicketInvalid` | unknown, used or expired ticket |
| 99 | `Other` | text only |

## 4. Accounts and character storage

**Storage: one SQLite database (WAL mode, `rusqlite` with the bundled SQLite) plus one file per character.**

```
<server dir>/data/accounts.sqlite
<server dir>/data/characters/<account_id>/<name>.d2rc     (the existing D2RC envelope, characters.rs)
```

Tables (cl-mp-accounts may add columns and indexes; these are fixed):

```sql
accounts(id INTEGER PRIMARY KEY, name TEXT NOT NULL, name_key TEXT NOT NULL UNIQUE,
         pw_hash TEXT NOT NULL, created INTEGER NOT NULL, last_login INTEGER, disabled INTEGER NOT NULL DEFAULT 0)
characters(name_key TEXT PRIMARY KEY, name TEXT NOT NULL, account_id INTEGER NOT NULL REFERENCES accounts(id),
           class INTEGER NOT NULL, level INTEGER NOT NULL, status INTEGER NOT NULL, look BLOB NOT NULL,
           created INTEGER NOT NULL, last_played INTEGER NOT NULL DEFAULT 0, deleted INTEGER)
```

- `name_key` is the name in lowercase ASCII; names are unique without regard to case.
- The `.d2rc` file is the truth for a character. The `characters` row is its summary for character select and the
  lobby, written after every save by the game (section 6). `d2rust-server --reindex` rebuilds the rows from the files.
- **Delete** marks the row `deleted` and moves the file to `data/deleted/<account_id>/`; the name stays taken for 7
  days, then it is freed (the files stay with the backups).
- **Account names:** 2-15 characters, ASCII letters, digits, `-` and `_`, starting with a letter. **Passwords:** 8-64
  bytes of UTF-8. **Character names:** the existing `characters::check_name` rule. `max_characters` = 16 per account
  (config).
- **Hashing:** argon2id (`argon2` crate), PHC string in `pw_hash`, parameters m = 19456 KiB, t = 2, p = 1. A config
  `argon2 = test` uses m = 256, t = 1 for test servers only; the server logs a warning when it is set, and the
  deploy config never sets it.
- **Sessions** live in memory only: the 16-byte random token from `Welcome`, the account, the selected character,
  the game. A server restart ends every session; clients log in again.
- **No email in v1.** Password reset is an admin command: `d2rust-server --set-password ACCOUNT` (reads the new
  password from stdin). Also `--disable ACCOUNT`, `--enable ACCOUNT`.
- **Backups:** the nightly backup (M6a `--backup`) writes `VACUUM INTO` of the database and a copy of
  `data/characters/` into one dated folder, 14 kept.
- Logs never contain passwords, tokens, tickets or hashes. Tokens print as their first 2 bytes (as `ResumeToken`
  does now).

Server-side Rust API (`d2rust-server::accounts`, cl-mp-accounts; v8 as built):

```rust
/// A refused request: the code, the wait for limits and locks, and the text
/// ("which rule" for the Bad* codes). Used by accounts, lobby and games alike.
pub struct Denied { pub code: ErrorCode, pub wait_s: u32, pub text: String }
impl From<ErrorCode> for Denied { /* code's own text, wait 0 */ }
impl Denied { pub fn to_msg(&self, request_kind: u8) -> ErrorMsg; }

pub struct AccountId(pub i64);
pub struct Account { pub id: AccountId, pub name: String /* as stored */ }
#[derive(Clone)] // an Arc inside
pub struct Accounts { /* db pool, rate limiter, hash semaphore */ }
impl Accounts {
    pub fn open_with(cfg: AccountsConfig) -> Result<Self, AccountError>;
    pub fn with_log(self, log: Arc<ServerLog>) -> Self;
    pub async fn register(&self, name: &str, password: &str, ip: IpAddr) -> Result<Account, Denied>;
    pub async fn login(&self, name: &str, password: &str, ip: IpAddr) -> Result<Account, Denied>;
    pub async fn characters(&self, a: AccountId) -> Vec<CharSummary>;
    pub async fn create_character(&self, a: AccountId, new: NewChar) -> Result<CharSummary, Denied>;
    /// The caller checks `Lobby::character_in_game` first (CharInGame).
    pub async fn delete_character(&self, a: AccountId, name: &str, password: &str) -> Result<(), Denied>;
    pub async fn character(&self, a: AccountId, name: &str) -> Result<CharSummary, Denied>;
    /// What the lobby needs for a `Seat`: the summary, the record file, and the new-character data if never played.
    pub async fn seat(&self, a: AccountId, name: &str) -> Result<CharSeat, Denied>;
    pub fn character_store(&self, a: AccountId) -> Result<CharacterStore, CharacterError>; // creates the folder
    /// Called by the lobby when a game reports `GameEvent::Saved`.
    pub async fn saved(&self, a: AccountId, summary: CharSummary);
    // admin (blocking): set_password, set_disabled, reindex, backup_into(dest)
}
pub mod summary { pub fn of_record(record: &[u8], last_played: u64) -> Result<CharSummary, &'static str>; } // games build Saved with it
```

Config: `ServerConfig.accounts: AccountsConfig` with the keys `data` (default `data`), `argon2` (`normal` | `test`),
`max_characters` (1-32, default 16; `MAX_CHARACTERS_LIMIT` = 32 so a full `CharListAnswer` fits `MAX_FRAME`) and
`registration` (`open` | `closed`). `Denied` lives in `d2rust-server/src/denied.rs` (cl-mp-accounts); the lobby and
games use it too wherever this page said `Result<_, ErrorCode>`.

A new character gets its first record from the same new-character path single player uses (class, name, hardcore,
expansion, `new_flags`), made by the game the first time it joins: the store holds only the summary row until then.

## 5. Rate limits and security

| What | Limit | Answer |
|---|---|---|
| Unauthenticated sessions per IP | 4 at a time | `ServerFull` |
| `Register` per IP | 3 per hour | `RateLimited` + `wait_s` |
| `Login` per IP | 10 per minute (token bucket) | `RateLimited` |
| Failed logins per account | 10 in 15 min, then locked 15 min | `AccountLocked` + `wait_s` |
| Every failed login | answered after a fixed 500 ms | (no timing hint) |
| `JoinGame` wrong password per session | 5 per minute | `RateLimited` |
| `CreateGame` per session | 5 per minute | `RateLimited` |
| `Chat` per session | 5 lines per 10 s | `RateLimited` |
| Sessions in total | `max_sessions` (config, 256) | `ServerFull` |
| Games in total | `max_games` (config, 16) | `TooManyGames` |

**Order of the checks (v6).** A `Login` is checked in this order: the per-IP limit (`RateLimited`), the account lock
(`AccountLocked`, without checking the password), then the password (`WrongLogin`, after the fixed delay; it counts
toward the lock). A `CharDelete` with a wrong account password is `WrongLogin` and counts toward the lock too. Per-IP
limits key on the IP address only (not the port); an IPv6 address counts by its /64 network (v9).

The other rules: TLS with the pinned certificate (section 8); argon2id; strict decode, `MAX_FRAME`; the server
decides everything; game passwords are kept in memory only (they live as long as the game) and compared in constant
time; no secrets in the repos, the manifest, logs or crash reports.

## 6. Games and the lobby (server internals)

Owned by cl-mp-server (`games`) and cl-mp-lobby (`lobby`):

```rust
pub struct GameSpec { pub id: u32, pub name: String, pub difficulty: u8, pub hardcore: bool,
                      pub expansion: bool, pub max_players: u8 }
pub struct Seat { pub account: AccountId, pub character: String, pub file: PathBuf, pub new: Option<NewChar> }

// games (cl-mp-server; v8 as built)
pub struct Games { /* ... */ }
impl Games {
    /// Games never touch the database: `net.rs` forwards `GameEvent::Saved` to `Accounts::saved` (v13).
    pub fn new(cfg: GamesConfig, data: GameData, log: Arc<ServerLog>) -> Games;
    pub fn start(&self, spec: GameSpec, events: mpsc::Sender<GameEvent>) -> Result<GameHandle, Denied>;
}
/// A game stream as the game sees it: a `Transport` that can also send one raw body (the game `Welcome`)
/// before the packets. `net.rs` implements it for `d2_quic::PacketStream`.
pub trait AttachedStream: Transport + Send { fn send_body(&mut self, body: Vec<u8>) -> Result<(), NetError>; }
impl GameHandle {
    /// Hands an attached game stream and its seat to the game thread.
    pub fn admit(&self, seat: Seat, stream: impl AttachedStream) -> Result<(), Denied>;
    /// `Attach` mode 1: the re-entry of section 3.7 (full game state, no backlog replay).
    pub fn resume(&self, token: ResumeToken, stream: impl AttachedStream) -> Result<(), Denied>;
    pub fn players(&self) -> u8;
    pub fn shutdown(&self); // saves everyone, then GameEvent::Closed
}
pub enum GameEvent {
    Players { game: u32, players: u8 },
    Saved { game: u32, account: AccountId, summary: CharSummary },
    Left { game: u32, account: AccountId, character: String },
    Closed { game: u32, reason: ErrorCode },
}
```

- **Lifecycle:** the lobby calls `Games::start` on `CreateGame` and gives the creator a ticket. The game is built
  when the first seat is admitted, at the spec's difficulty. It closes when it has no players and no held slots (a
  lost player's slot is held `grace_s`, 30 s). The lobby removes it from the list on `Closed` and sends `GameGone`.
- **Saves:** on leave, every `save_interval_s` (60 s) at a safe point, on game close and on server shutdown
  (SIGTERM, Ctrl+C). Write-then-rename (as `characters.rs` does), so a crash leaves the last good file. After each
  save the game sends `GameEvent::Saved`, and `net.rs` calls `Accounts::saved` (v13; the lobby ignores `Saved`).
- **A character is in one game at most;** a session is in one game at most.

### Sessions, dispatch and the lobby seam (v2)

`d2rust-server` `session.rs` and `net.rs` are cl-mp-server's; `lobby/` is cl-mp-lobby's; `accounts/` and
`msg/account.rs` are cl-mp-accounts'. `CharSummary` is the `d2_net::msg::account` type, used on both sides.

```rust
// session.rs (cl-mp-server)
pub struct SessionCtx {
    pub id: u64, pub ip: IpAddr,
    pub account: Option<(AccountId, String)>,
    pub character: Option<CharSummary>,
    pub game: Option<u32>,
    pub out: ControlTx,
}
impl ControlTx {
    pub fn send(&self, body: Vec<u8>);                                       // one S→C control frame
    pub fn error(&self, request_kind: u8, code: ErrorCode, wait_s: u32, text: &str); // ERROR 0x10
}
pub struct Sessions; // id -> ControlTx, so the lobby can broadcast (list updates, chat)

// lobby/ (cl-mp-lobby)
impl Lobby {
    pub fn new(cfg: &ServerConfig, games: Games, sessions: Sessions) -> Lobby;
    /// 0x40-0x5F, raw; decodes msg::lobby itself and answers through s.out. An Err becomes ERROR for the request kind.
    pub async fn handle(&self, s: &mut SessionCtx, body: &[u8]) -> Result<(), Denied>;
    /// Attach mode 0: once, 30 s, same session; else TicketInvalid.
    pub fn redeem(&self, s: &SessionCtx, ticket: [u8; 16]) -> Result<(GameHandle, Seat), Denied>;
    pub fn on_game_event(&self, ev: GameEvent);
    pub fn session_ended(&self, s: &SessionCtx);
    pub fn character_in_game(&self, name_key: &str) -> bool;                 // CharInGame on select and delete
}

// games (cl-mp-server): see `Games::new` and `AttachedStream` above (v8).
```

Dispatch: 0x20-0x3F are decoded with `msg::account` and handled in `session.rs` on the `Accounts` API (cl-mp-server
writes that glue); 0x40-0x5F go raw to `Lobby::handle`.

### Difficulty rule (cl-mp-lobby, shared code)

`d2_formats::d2s::status::difficulties(status: u16) -> u8` returns the unlocked mask from the save status word,
with the same rule character select uses today (`ui/front_end/screens/difficulty.rs`, which then calls it):

- Normal: always.
- Nightmare: progression ≥ 5 for an expansion character, ≥ 4 for a classic one.
- Hell: progression ≥ 10 for an expansion character, ≥ 8 for a classic one.

The server decides with it:

- `CreateGame` at difficulty `d` needs bit `d` in the character's mask, else `DifficultyLocked`.
- `JoinGame` needs the game's difficulty bit in the mask, else `DifficultyLocked`; the same hardcore and expansion
  kind, else `WrongGameKind`; a dead hardcore character can neither create nor join (`DeadHardcore`).
- The mask comes from the stored summary, never from the client.

Game names: 1-15 characters, letters, digits, space, `-`, `_`; unique among open games, any case. Game passwords:
0-15 bytes; empty = public.

## 7. Ports, server folder, deploy (forge box)

- **One UDP port, 6112** (config `listen = 0.0.0.0:6112`). No TCP port. Health is a file, not a port:
  `logs/status.json`, rewritten every 10 s (version, uptime, sessions, games, players, tick overruns).
- **Admin steps (root; cl-mp-server writes the exact commands, the admin runs them):** open UDP 6112 in the box's
  firewall; `loginctl enable-linger lc` if the user service needs it; a DNS name (for example
  `play.werdegames.com`) if wanted, until then the IP.
- **Server folder** (`~lc/d2rust-server/`, no root):

```
bin/d2rust-server-<version>    current -> bin/d2rust-server-<version>
d2rust-server.cfg              (0600; no secrets in it besides paths)
identity/cert.der  key.der     (key 0600; made once, kept across deploys and versions)
data/accounts.sqlite  data/characters/  data/deleted/
game/                          the game data the server needs, read-only, never under /srv/d2rust/dl
logs/  backups/
```

- **Service:** a systemd user unit `d2rust-server.service` (`Restart=on-failure`, `Nice=-5` only if allowed, else
  0), plus a user timer for the nightly backup. `tools/server/deploy.sh <binary>` (built by `tools/server/build.sh`)
  installs, swaps `current`, restarts and prints the version and the certificate fingerprint. Until the admin
  enables lingering, a user-crontab supervisor keeps the server running instead of the user unit (v11).
- **Light:** the server's own limits keep it small: `max_games` 16, 8 players each, 2 net threads; measured idle
  and loaded CPU and RAM go in the server PR.

## 8. Certificate pinning through the signed manifest

- The server's identity is a self-signed certificate made once (`d2rust-server --new-identity`), kept in `identity/`.
- **Fingerprint** = SHA-256 of the certificate's DER bytes, written as 64 lowercase hex digits.
- `d2_quic::ServerTrust::Pinned` takes the fingerprint (`[u8; 32]`) instead of the whole certificate; the client
  compares the SHA-256 of the presented end-entity certificate. `ServerTrust::Any` stays for tests only (the game
  never uses it outside tests and the dev console).
- The launcher manifest gains a top-level list:

```json
"servers": [
  { "id": "eu1", "name": "D2RUST Europe", "channel": "beta", "game": "d2rust",
    "address": "37.27.64.176", "port": 6112,
    "cert_sha256": "<64 hex>", "version": "0.1.2.0" }
]
```

  It is covered by the manifest signature like everything else. Older launchers must ignore the unknown key
  (cl-mp-client checks the `format` 1 reader). A changed identity means a manifest update.

## 9. How online play starts

- **From the launcher ("Play online"):** shown for a channel that has a server in the manifest. The launcher first
  makes sure the installed game version equals the server's `version` (it updates if not, and refuses with a clear
  message if it can't). It writes `servers.toml` next to the game exe from the manifest, then starts:

```
D2RUST-Game play --online [--server eu1]
```

  The game skips the main menu and opens the online login screen for that server (the first one when `--server` is
  missing).

- **From the game's main menu ("Online"):** opens the same login screen with the servers in `servers.toml`. If the
  file is missing, the screen says "Start the game once from the launcher to get the server list." The dev console
  can connect to a typed address and fingerprint (dev tools stay on).

- `servers.toml` (written by the launcher, read by the game):

```toml
[[server]]
id = "eu1"
name = "D2RUST Europe"
game = "d2rust"
address = "37.27.64.176:6112"
cert_sha256 = "<64 hex>"
version = "0.1.2.0"
```

- **Screen flow** (cl-mp-lobby, matching the existing menu style): Online login (account, password, Log in,
  Create account, Back) → online character select (the account's characters, Create new, Delete, OK, Back = log out)
  → character create (the existing create screen in online mode: class, name, hardcore, expansion) → lobby (game
  list with name, difficulty, players, lock; Create game; Join game; chat if built; Back = character select) →
  create-game panel (name, password, difficulty buttons enabled from the character's mask) / join with a password
  prompt for a locked game → loading → game. Leaving the game returns to the lobby. Every failure shows the error
  code's text in the game's message box; the screen stays where it was.
- The online character never touches the client's save folder.
- **Release builds:** the `online` cargo feature of `d2-client` (which pulls `d2-online` and `d2-quic`) is **off in
  release builds** until the owner says to turn it on (owner decision 2, updated). Test builds for online play
  (CI, the MP test tool, the owner's two-PC test) build with `--features online`. Without the feature, the main menu's
  second button keeps its current label and does nothing (single-player menus unchanged), and `play --online` says "this build has no online play". The launcher shows "Play online"
  only when the manifest lists a server for the channel, so release channels get no server entry until then.

## 10. The MP test tool gate

- cl-mp-testtool's bots use `d2-online`, so they speak exactly the client's protocol.
- The tool starts a real `d2rust-server` (`--config` in a temp folder, `argon2 = test`, a fresh identity, port 0 on
  loopback) and puts its own UDP proxy between bots and server for latency, loss and jitter (no root, no `tc`).
- Every MP PR runs `sh C:/Users/USER/.t3/projects/lc/scratch/remote.sh tools/mp-test/gate.sh` once the gate exists
  and pastes the report summary in the PR. Scenarios for parts not built yet are present and marked pending; a
  pending scenario becomes required in the PR that builds its part.
- Each scenario names the stated rule it checks (a line of this page or of `multiplayer-v1.md`), so a failure reports
  the broken rule.

### 10.1 What the server and `d2-online` give the test tool (v5)

- **Ready line** (cl-mp-server): once listening, `d2rust-server` prints one stdout line
  `ready listen=ADDR cert_sha256=HEX64 game=ID build=B protocol=N tick=N caps=LIST` (`listen` is the bound address,
  so `listen = 127.0.0.1:0` works). `caps` is a comma list of what this build serves, from: `connect`, `accounts`,
  `characters`, `lobby`, `chat`, `game`, `walk`, `party`, `trade`, `items`, `combat`, `waypoints`, `acts`,
  `persist`, `admin`, `dev`. A scenario that needs a cap the server doesn't list is reported pending. Each lane adds
  its cap in the PR that makes it work.
- **Test-only config** (each prints a warning line in the log at start; the deploy config never sets them):
  - `argon2 = test` (section 4; cl-mp-accounts);
  - `limits = test` (cl-mp-accounts, cl-mp-lobby): every window and lock length of section 5 is divided by 60
    (15 min → 15 s, 1 min → 1 s, 1 h → 60 s), the counts stay; the fixed 500 ms failed-login delay becomes 10 ms;
  - `grace_s = 0` is allowed (no held slot);
  - `dev = test` (cl-mp-server): the server reads stdin lines `dev CHARACTER COMMAND` (the dev console's text
    commands, `d2_proto::dev::parse`: `spawn fallen`, `item hp1`, `waypoints`, `acts`) and runs the command in that
    character's game, answering one stdout line `dev ok TEXT` or `dev err TEXT`. Clients never get dev rights.
    It also takes `set-progression CHARACTER N` on stdin (v6): the same change as the offline command below, made
    by the running server (cl-mp-accounts gives the function, cl-mp-server the stdin line); refused with
    `dev err` while the character is in a game.
  - The server refuses an unknown key, so the tool adds a key only once the server knows it; each lane notes a new
    key in cl-mp-testtool's STATUS.md.
- **Loopback addresses (v6):** every bot shares 127.0.0.1 unless the tool gives it its own address. Per-IP limits
  (4 unauthenticated sessions, login and register rates) would then stop scenarios with many bots. The tool binds
  each bot's relay socket toward the server on its own address in 127.0.0.0/8 (127.0.0.2, .3, ...; Linux routes the
  whole block to loopback), and puts bots on one address only in scenarios that test a per-IP limit.
- **Progression for tests** (cl-mp-accounts): `d2rust-server --config C --set-progression CHARACTER N` sets the
  progression bits of an online character's status word (file and summary row; also before its first join, where it
  seeds the new record), exit 0. Offline only: it refuses while the server runs (the server holds a lock file in
  `data/`). Also usable by the admin.
- **`d2-online` for bots** (cl-mp-client, cl-mp-accounts, cl-mp-lobby):
  1. a non-blocking `poll` of the control stream (64 sessions on one thread);
  2. failures as `OnlineError` with `code() -> ErrorCode`, `wait_s()` and `text()` (an enum as built, v10);
     `ErrorCode::name()` gives the names
     of section 3.8 (`WrongLogin`, `DifficultyLocked`), which scenarios use;
  3. the game link as a `d2_net::Transport` (bots read S→C with `d2_proto`);
  4. `ConnectOptions { hello: Option<Hello> }` to override the `Hello` (build, protocol, tick rate, game) for refusal
     tests; the game never sets it.

## Changes

- v1 (2026-10-19): first version.
- v13 (2026-10-20, from the #484/#502 reviews): `net.rs` forwards `GameEvent::Saved` to `Accounts::saved`; the lobby
  ignores it (6).
- v12 (2026-10-20, from the #499 review): what a resume re-entry resends and what the client clears (3.7).
- v11 (2026-10-19, from the #496 review): `deploy.sh` takes the built binary; the cron supervisor as the stop-gap
  before lingering (7).
- v10 (2026-10-19, from the #487 review): the client's jitter buffer of whole frames (start 2, at most 8 ticks,
  measured in P13; emptied on resume) (3.7); `OnlineError` as an enum with accessors (10.1).
- v9 (2026-10-19, from the #469 review): IPv6 per-IP limits by /64; `character_store` (`CharacterError`) and `summary::of_record` return
  `Result` (as built).
- v8 (2026-10-19): the accounts API as built (`Denied`, `Account`, `seat`, `AccountsConfig`, `MAX_CHARACTERS_LIMIT`
  32; cl-mp-accounts request), code 48 `ExpansionClass`; games as built (`Games::new` without `Accounts`,
  `AttachedStream`, `GameHandle::resume` = re-entry; cl-mp-server request); the seat filter rule (3.7).
- v7 (2026-10-19, from the #473 review): without `online` the main menu keeps its inert second button (9).
- v6 (2026-10-19, from the #466 review): order of the login checks, `CharDelete` wrong password = `WrongLogin`
  (5); `set-progression` on a running test server's stdin; one loopback address per bot (10.1). There is no
  `lock_s` key: `limits = test` shortens the lock.
- v5 (2026-10-19): what the test tool gets (10.1, cl-mp-testtool request): ready line with caps, test-only config
  (`argon2`, `limits`, `grace_s = 0`, `dev`), `--set-progression`, `d2-online` bot needs; `ErrorCode::name()` gives
  the table's names (#459).
- v4 (2026-10-19): resume is a re-entry with the full game state, not a backlog replay (3.7, cl-mp-client request).
- v3 (2026-10-19): kind clash with M1e (cl-mp-client request): `EXT` keeps 9, `ATTACH` is 0x0A, `FRAME_END` 0x0B;
  `NET_VERSION` 4 and ALPN `d2rust/4` for the v3 handshake.
- v2 (2026-10-19): owner update: online stays off in release builds until the owner says (decision 2, section 9);
  MP builds at lower CPU priority. Requests answered: game-stream frame order with `Transport::flush` and
  `FRAME_END` (3.7, cl-mp-client a); client-side display prices (3.7, cl-mp-client b); `d2-quic` stream layer for
  both sides by cl-mp-client (2, 3.9, cl-mp-client c); session, dispatch and lobby seam (6, cl-mp-server).
