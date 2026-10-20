//! `d2-convert`: converts the player's own D2 LoD install into the native
//! asset folder, checking every file by a round trip.
//!
//! The per-kind decode / write / read-back / compare steps live behind the
//! [`Kind`] trait (`kind.rs`); `kinds.rs` is the one place that lists the
//! kinds the binary converts. Everything else (name set, folder layout,
//! resume, report, `verify`) is kind-independent.

pub mod convert;
pub mod ctable;
pub mod fsutil;
pub mod kind;
pub mod kinds;
pub mod names;
pub mod report;
pub mod verify;

pub use convert::{convert, ConvertError, Options, RunSummary};
pub use ctable::TableSummary;
pub use kind::{Failure, Kind, NativeData, Written};
pub use verify::{verify, verify_full, VerifyOptions, VerifyOutcome};
