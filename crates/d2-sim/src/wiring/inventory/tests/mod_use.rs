//! A mod game's item use functions (`docs/plans/mods/seams.md` MS7c):
//! asked first for the entries its item provider answers, also past
//! D2RUST's 30; `None` runs D2RUST's body.

use std::sync::Arc;

use super::*;
use crate::items::provider::{ItemProvider, UseCall, UseHost};
use crate::rules::{ModRules, Ruleset};

/// A use function id D2RUST has none for.
const MOD_ENTRY: u32 = 40;
/// Town Portal.
const TOWN_PORTAL: u32 = 2;
/// The stat the mod's function sets on the player: strength.
const MARK: u16 = 0;

/// Entry 40 sets the player's strength to 25;
/// entry 2 answers `portal`.
#[derive(Debug)]
struct Uses {
    portal: Option<bool>,
}

impl ItemProvider for Uses {
    fn has_use_fn(&self, entry: u32) -> bool {
        entry == MOD_ENTRY || entry == TOWN_PORTAL
    }
    fn use_fn(&self, host: &mut dyn UseHost, call: &UseCall) -> Option<bool> {
        if call.entry == TOWN_PORTAL {
            return self.portal;
        }
        let u = host.unit_of(call.player)?;
        host.set_stat(u, MARK, 0, 25);
        Some(true)
    }
}

#[derive(Debug)]
struct Game(Uses);

impl ModRules for Game {
    fn name(&self) -> &str {
        "use-test"
    }
    fn items(&self) -> Option<&dyn ItemProvider> {
        Some(&self.0)
    }
}

/// A world whose key has use function 40, under `rules`.
fn world(portal: Option<bool>, modded: bool) -> World {
    let mut t = inv_tables();
    t.items[KEY].pspell = MOD_ENTRY;
    t.items[KEY].useable = 1;
    let mut w = World::with_tables(t);
    if modded {
        w.hooks.rules = Ruleset::Mod(Arc::new(Game(Uses { portal })));
    }
    w
}

fn stored(w: &mut World, record: usize, x: u32, y: u32) -> Guid {
    let g = w.cursor_item(record);
    assert_eq!(w.handle(&insert(g, x, y, 0)), Ok(0));
    w.drain();
    g
}

/// Under D2RUST's rules entry 40 is no use entry: the key is not used.
#[test]
fn an_entry_past_d2rusts_is_none_under_original() {
    let mut w = world(None, false);
    let k = stored(&mut w, KEY, 0, 0);
    assert_eq!(w.desk(|d| d.use_entry(k)), None);
    assert_eq!(w.handle(&msg(0x20, &[k, 10, 10])), Ok(0));
    assert!(w.unit(k).is_some(), "the key stays");
}

/// A mod's entry 40 runs its function: the player's strength is set,
/// and the used key is consumed as any used item is.
#[test]
fn a_mod_entry_past_d2rusts_runs() {
    let mut w = world(None, true);
    let k = stored(&mut w, KEY, 0, 0);
    assert_eq!(w.desk(|d| d.use_entry(k)), Some((MOD_ENTRY, -1)));
    assert_eq!(w.handle(&msg(0x20, &[k, 10, 10])), Ok(0));
    assert!(w.unit(k).is_none(), "the key is used up");
    let p = w.player;
    assert_eq!(w.stats.unit_total(p, MARK, 0), 25);
}

/// A mod's refusal of entry 2 keeps the scroll and casts nothing; its
/// `None` runs D2RUST's cast.
#[test]
fn a_mod_overrides_town_portal_or_falls_through() {
    let mut w = world(Some(false), true);
    w.hooks.cast = Some((1, true));
    let s = stored(&mut w, TSC, 0, 0);
    assert_eq!(w.handle(&msg(0x20, &[s, 10, 10])), Ok(0));
    assert!(w.hooks.casts.is_empty(), "the mod refused before the cast");
    assert!(w.unit(s).is_some(), "the scroll stays");
    assert_eq!(w.data(s).flags & crate::items::moves::iflag::TARGETING, 0);

    let mut w = world(None, true);
    w.hooks.cast = Some((1, true));
    let s = stored(&mut w, TSC, 0, 0);
    assert_eq!(w.handle(&msg(0x20, &[s, 10, 10])), Ok(0));
    assert_eq!(w.hooks.casts, [w.player]);
    assert!(w.unit(s).is_none(), "the scroll is used up");
}
