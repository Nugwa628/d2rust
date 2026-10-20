//! Monsters: the AI think ([`ai`]), creation and unique-monster mods
//! ([`init`]), mode messages ([`mode_message`]), population, and the
//! overrides a mod game installs for them ([`provider`]).

pub mod ai;
pub mod init;
pub mod mode_message;
#[cfg(test)]
mod mutant_tests;
pub mod population;
pub mod provider;
