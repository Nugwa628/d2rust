//! The text kinds: native files a person can read and edit in any text
//! editor.
//!
//! Three kinds are TOML sidecars and live here: animation layouts
//! ([`cof`]), font tables ([`font`]) and map pieces ([`ds1`]). The other
//! text kinds of this crate (string tables, the animation table, excel
//! tables, sounds) have their own modules but share the toolkit in
//! [`text`]: the hand-written TOML emitter, the strict TOML and TSV
//! readers, the cell escapes and the error type.
//!
//! Every text kind keeps the same promises:
//! - writers emit keys in a fixed order with one spelling per value, so the
//!   same decoded value always gives the same bytes;
//! - readers are strict: a wrong `native` kind or version, an unknown or
//!   missing key, a value out of range or a number in another spelling is
//!   an error naming the file, so an edited file is either exact or
//!   refused;
//! - a writer refuses a value it cannot represent instead of losing part
//!   of it.
//!
//! The runtime native source reads these files back through the same
//! readers.

mod cof;
mod ds1;
mod font;
mod text;

#[cfg(test)]
mod tests;

pub use cof::{check_cof, read_cof, write_cof};
pub use ds1::{check_ds1, read_ds1, write_ds1};
pub use font::{check_font, read_font, write_font};
pub use text::{TextError, NATIVE_VERSION};

pub(crate) use text::{
    check_round_trip, first_difference, hex_bytes, latin1, tsv_escape, tsv_rows, tsv_unescape,
    unhex_bytes, unlatin1, Out, Tab,
};
