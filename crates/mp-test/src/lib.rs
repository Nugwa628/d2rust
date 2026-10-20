//! `mp-test`: the multiplayer test tool.
//!
//! Headless bot clients (up to 64) run scripted scenarios against a real
//! server process on loopback, through relays that inject latency, jitter
//! and loss. Every failure names the stated rule it breaks; the run writes
//! a JSON and a markdown report. Scenarios for features that aren't built
//! yet are written ahead and reported as pending until the tool can drive
//! them and the server serves them. The guide is `docs/tools/mp-test.md`.
//!
//! - [`scenario`]: the `.mps` scenario format and its parser.
//! - [`verbs`]: every step verb, the capability it needs, and whether the
//!   tool drives it yet.
//! - [`runner`]: runs a scenario, with the tool's own `core.*` rules.
//! - [`bot`]: one headless client and what it knows of the game.
//! - [`play`]: what a bot knows about play: monsters, items, objects,
//!   act and area.
//! - [`net`]: the loopback relay that injects network conditions.
//! - [`server`]: the server process under test and its ready line.
//! - [`report`]: the JSON and markdown report.
//! - [`synth`]: the stand-in server (`mp-test-synth`): the synthetic town
//!   behind a real QUIC endpoint.

pub mod bot;
pub mod net;
pub mod play;
pub mod report;
pub mod runner;
pub mod scenario;
pub mod server;
pub mod synth;
pub mod verbs;
