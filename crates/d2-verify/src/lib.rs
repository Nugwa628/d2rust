//! The ownership gate, client side: checks that the folder the player
//! points D2RUST at holds a usable Diablo II: Lord of Destruction install.
//! The required archives must exist, open as valid archives and contain the
//! files the engine loads. The checks are not written yet; the crate is the
//! place they belong.
//!
//! The gate checks that the player *has* the game, not that their copy is
//! one exact release: installs differ between releases, so nothing is
//! compared against fixed hashes. It reads only the chosen game folder.
