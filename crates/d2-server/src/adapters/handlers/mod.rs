//! Intent handlers per system: the server side of the client's C→S messages.
//!
//! Each submodule owns the C→S ids of one game system (items, player,
//! skills, walking, world), is called from [`super::SimGame`]'s `handle`
//! before the stub, and routes those ids into the `d2-sim` module that owns
//! their behaviour.

pub mod items;
pub mod player;
pub mod skills;
pub mod walk;
pub mod world;
