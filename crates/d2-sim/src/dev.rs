// D2RUST's own: dev tools.
//! The dev switches the sim reads ([`DevSwitches`], on
//! `ActionHooks::dev`). Default: everything off, so a game nobody gives a
//! dev command to runs exactly the normal game paths. Only the dev handlers
//! of the interactive client write them.

use std::collections::BTreeSet;

use crate::units::UnitId;

/// The dev switches of one game. The default has every switch off.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DevSwitches {
    /// Units in god mode: starting their death is refused.
    pub god: BTreeSet<UnitId>,
    /// No skill cooldown (kept up by the dev command handlers).
    pub nocd: bool,
    /// Free mana (kept up by the dev command handlers).
    pub freemana: bool,
    /// The walk / run velocity bonus in percent.
    pub speed: u32,
}

impl DevSwitches {
    /// Whether `u` is in god mode.
    pub fn is_god(&self, u: UnitId) -> bool {
        self.god.contains(&u)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switches_default_off() {
        let d = DevSwitches::default();
        assert!(d.god.is_empty() && !d.nocd && !d.freemana && d.speed == 0);
        assert!(!d.is_god(UnitId(1)));
    }
}
