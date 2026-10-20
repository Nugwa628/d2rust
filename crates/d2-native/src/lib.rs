//! d2rust-native asset formats: plain, moddable files that hold exactly what
//! the game's packed formats hold. The converter decodes each file of the
//! player's own install, writes its native form (PNG pictures, TOML
//! sidecars, TSV tables), reads that form straight back and compares the
//! two values; only a file that survives this round trip is kept. At run
//! time the native source reads the same files back, through a stack of
//! mod layers, and hands the loaders the very values the archives would
//! have given them.
//!
//! No Bevy: this crate is plain data code shared by the converter, the
//! client and the server.
//!
//! The modules, by area:
//! - the shared contract: [`kind`] (the write / read / compare API of a
//!   native kind) and [`native_toml`] (strict TOML sidecar reading);
//! - pictures and maps: [`png`], [`sheet`] (DC6 / DCC sprites), [`tileset`]
//!   (DT1), [`pal`] (palettes and palette transforms), [`expfield`];
//! - text and audio: [`toml_kinds`] (COF, DS1, fonts), [`tbl`] (string
//!   tables), [`animdata`], [`tables`] (excel tables), [`wav`];
//! - the converter's view: [`asset`] (one entry point per converted kind)
//!   and [`manifest`] (`manifest.toml` and `files.tsv`);
//! - the runtime view: [`layers`] (the base and mod layer stack) and
//!   [`source`] (decoded assets and compiled tables from a native root).

// ---- the shared contract
pub mod kind;
pub mod native_toml;

// ---- pictures and maps
pub mod expfield;
pub mod pal;
pub mod png;
pub mod sheet;
pub mod tileset;

// ---- text and audio
pub mod animdata;
pub mod tables;
pub mod tbl;
pub mod toml_kinds;
pub mod wav;

// ---- the converter's view
pub mod asset;
pub mod manifest;

// ---- the runtime view
pub mod layers;
pub mod source;
