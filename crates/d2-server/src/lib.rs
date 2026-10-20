//! Authoritative server. Runs `d2-sim`, validates every intent, owns
//! accounts and character storage. Runs in-process for single player.
//!
//! This crate holds the server's message queues, per-client output
//! buffers, the dispatcher and the host loop that drives one game tick per
//! frame. What other crates provide comes in through the traits in
//! [`seams`]. Clients reach the host through [`hub::Hub`], which carries
//! their messages over a `d2_net::Transport` (the in-memory one in single
//! player); tests and the scenario tools may also drive [`host::Host`]
//! directly. [`game_thread::GameThread`] keeps a game on a thread of its
//! own: served for single player, or looped over
//! [`hub::Hub::serve_frame`] for a server that owns its frame loop.
//! Accounts and storage are not here yet.

pub mod adapters;
pub mod buffers;
pub mod dev;
pub mod dispatch;
pub mod extension;
pub mod file_source;
pub mod game;
pub mod game_files;
pub mod game_thread;
pub mod host;
pub mod hub;
pub mod local_seams;
pub mod monster_ai;
pub mod monster_drop;
pub mod npc_seams;
pub mod packets;
pub mod perf;
pub mod rest;
pub mod seams;
pub mod skill_rest;
pub mod transport;
pub mod weapons;
pub mod world_data;

#[cfg(test)]
mod tests;
