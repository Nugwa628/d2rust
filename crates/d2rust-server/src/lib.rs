//! `d2rust-server`: the dedicated server. It runs online games with no
//! window. Clients reach it over QUIC; there is no player hosting.
//!
//! - [`config`]: the server's settings, read from a `key = value` file.
//! - [`characters`]: the character folder. Online characters are created
//!   on the server and stored here in D2RUST's own versioned format. They
//!   are never written to a client.
//! - [`log`]: the server's log file in the log folder.
//! - [`backup`]: validated, offline snapshots of server characters.
//! - [`accounts`]: register, login, rate limits and the online characters
//!   an account owns, in a SQLite database next to the character files.
//! - [`denied`]: a refused request (code, wait, words), the answer of
//!   accounts, the lobby and games alike.
//! - [`devcmd`]: `dev = test`, the dev commands on standard input.
//! - [`lobby`]: the game list, create and join rules, tickets and chat.
//! - [`games`]: the running games, one game thread each, with their
//!   players' characters loaded from and saved to the server.
//! - [`net`]: the QUIC endpoint's connections: the handshake, the
//!   sessions' requests and the game streams.
//! - [`session`]: sessions and the seam to the lobby.
//! - [`status`]: `status.json`, the server lock and live backups.

pub mod accounts;
pub mod backup;
pub mod characters;
pub mod config;
pub mod denied;
pub mod devcmd;
pub mod games;
pub mod lobby;
pub mod log;
pub mod net;
pub mod session;
pub mod status;
