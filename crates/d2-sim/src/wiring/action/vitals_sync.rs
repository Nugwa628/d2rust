//! The client vitals sync (`combat::vitals::sync`) on the action wiring:
//! the per-client caches live in [`SyncState`]; the
//! current values come from the unit's stat lists (totals, max life /
//! mana, the state 100 / 106 potion lists), its path record (position
//! and path target; `ActionHooks::paths`) or the host's staged position,
//! the charstats row of its class (`ActionHooks::vitals`) and the game
//! frame. The caller (the host's flush) runs
//! [`run`] once per client at the end of each flush and sends what it
//! returns to that client.
//!
//! Opt-in per game, like the path provider: `ActionHooks::sync` is
//! `None` by default and [`ActionHooks::enable_vitals_sync`] turns it on.

use std::collections::BTreeMap;

use super::{ActionHooks, ActionSim};
use crate::combat::vitals::sync::{self, Current, ManaInputs, SyncCache};
use crate::game::Game;
use crate::stats::stat;
use crate::units::lists::client_state;
use crate::units::{ClientId, UnitId, UnitType};

/// `healthpot` / `manapot` list states.
const STATE_HEALTHPOT: u32 = 100;
const STATE_MANAPOT: u32 = 106;
/// Stat 74 `hpregen`; 26 `manarecovery`; 27 `manarecoverybonus`; 13
/// `experience`; 14 `gold`; 6 / 8 / 10 life, mana, stamina.
const STAT_HPREGEN: u16 = 74;
const STAT_MANARECOVERY: u16 = 26;
const STAT_MANARECOVERYBONUS: u16 = 27;
const STAT_EXPERIENCE: u16 = 13;
const STAT_GOLD: u16 = 14;

/// The per-client caches of a game.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyncState {
    /// A new client's cache starts all 0.
    pub caches: BTreeMap<ClientId, SyncCache>,
}

impl<X> ActionHooks<X> {
    /// Turns the client vitals sync on for this game.
    pub fn enable_vitals_sync(&mut self) {
        self.sync = Some(SyncState::default());
    }
}

/// The current values of `unit` that the sync compares against the cache.
/// `staged`: the host's position of a unit without a path record.
pub fn current<X>(sim: &ActionSim<X>, game: &Game, unit: UnitId, staged: (u16, u16)) -> Current {
    let s = &sim.sys.stats;
    let total = |k: u16| s.unit_total(unit, k, 0);
    let life_total = total(stat::HITPOINTS);
    let m = s.max_life(unit) >> 8;
    let path = sim.sys.hooks.paths.as_ref().and_then(|p| p.dynamic(unit));
    let (x, y, dx, dy) = match path {
        Some(p) => {
            let (x, y) = (p.x() as u16, p.y() as u16);
            (
                x,
                y,
                x.wrapping_sub(p.target_x) as u8,
                y.wrapping_sub(p.target_y) as u8,
            )
        }
        None => (staged.0, staged.1, 0, 0),
    };
    let list = s.unit_list(unit);
    let state_list = |st: u32| list.and_then(|r| s.list_of_state(r, st));
    // The predictions multiply a per-base-tick regeneration by the
    // potion's remaining time, so that time is counted in base ticks.
    let rate = game.tick_rate;
    let base_expire = |e: i32| {
        game.frame
            .wrapping_add(rate.base_ticks(e.wrapping_sub(game.frame)))
    };
    let potion = state_list(STATE_HEALTHPOT)
        .map(|l| (s.total(l, STAT_HPREGEN, 0), base_expire(s.expire(l))));
    let lp = sync::life_prediction(potion, game.frame, life_total, m);
    let class = sim.sys.units.get(unit).map(|r| r.class);
    let mana_regen = sim
        .sys
        .hooks
        .vitals
        .as_ref()
        .zip(class)
        .and_then(|(v, c)| v.charstats(c as i32))
        .map(|c| c.manaregen);
    let mp = sync::mana_prediction(
        state_list(STATE_MANAPOT).map(|l| base_expire(s.expire(l))),
        game.frame,
        &ManaInputs {
            max_mana: s.max_mana(unit),
            mana_regen,
            regen_pct: total(STAT_MANARECOVERYBONUS),
            regen_add: total(STAT_MANARECOVERY),
            mana_total: total(stat::MANA),
        },
    );
    Current {
        life: life_total >> 8,
        max_life: m,
        mana: total(stat::MANA) >> 8,
        stamina: total(stat::STAMINA) >> 8,
        lp,
        mp,
        x,
        y,
        dx,
        dy,
        gold: total(STAT_GOLD) as u32,
        exp: total(STAT_EXPERIENCE) as u32,
    }
}

/// One run of the vitals sync for `client`: the
/// messages to send to that client, in order. `None` (nothing runs):
/// the sync is off, the client is not in game (state 4) or has no player
/// unit. `queued`: the client has a queued buffer; `staged`: the host's
/// position of a player without a path record.
///
/// A client whose unit is not a player is a fatal assert in the game:
/// here a debug assertion, and nothing runs in release. A routine the
/// game runs before the sync is not run under `Ruleset::Original`.
pub fn run<X>(
    sim: &mut ActionSim<X>,
    game: &mut Game,
    client: ClientId,
    staged: (u16, u16),
    queued: bool,
) -> Option<Vec<Vec<u8>>> {
    sim.sys.hooks.sync.as_ref()?;
    let entry = game.lists.client(client)?;
    if entry.state != client_state::IN_GAME {
        return None;
    }
    let unit = entry.player?;
    let is_player = sim.sys.units.get(unit)?.ty == UnitType::Player;
    debug_assert!(is_player, "vitals sync: client unit is not a player");
    if !is_player {
        return None;
    }
    let force = sync::force(entry.update_count, queued);
    let now = current(sim, game, unit, staged);
    let caches = &mut sim.sys.hooks.sync.as_mut()?.caches;
    let r = sync::sync(caches.entry(client).or_default(), &now, force);
    if r.done {
        if let Some(e) = game.lists.client_mut(client) {
            e.update_count = 0;
        }
    }
    Some(r.messages)
}

/// The stat messages of the changed-stat array flush: one stat message
/// per (key, base value) of `StatLists::mod_values`, in key order; the
/// key's layer is not sent. A stat id above 0xFE sends nothing (the
/// `Saved` stats are 0–15). Sent by the per-client update, not by this
/// sync.
pub fn mod_stat_messages(values: &[(i32, i32)]) -> Vec<Vec<u8>> {
    values
        .iter()
        .filter_map(|&(k, v)| stat_message(crate::stats::key_stat(k), v))
        .collect()
}

/// One stat message for stat `s` with value `v`: 0x1D below 0xFF, 0x1E
/// below 0xFFFF, else 0x1F (`None` for s > 0xFE).
pub(crate) fn stat_message(stat: u16, value: i32) -> Option<Vec<u8>> {
    let s = u8::try_from(stat).ok().filter(|&s| s <= 0xFE)?;
    let v = value as u32;
    Some(if v < 0xFF {
        vec![0x1D, s, v as u8]
    } else if v < 0xFFFF {
        let w = (v as u16).to_le_bytes();
        vec![0x1E, s, w[0], w[1]]
    } else {
        let d = v.to_le_bytes();
        vec![0x1F, s, d[0], d[1], d[2], d[3]]
    })
}

/// The join's vitals message ([`sync::join`]) for `client`'s player: 0x95,
/// then the gold and experience messages against the client's cache. With
/// the sync off the cache is a scratch one (all zero, the cache's value at
/// creation, see [`SyncState::caches`]). Empty without a player unit.
pub fn join_run<X>(
    sim: &mut ActionSim<X>,
    game: &Game,
    client: ClientId,
    staged: (u16, u16),
) -> Vec<Vec<u8>> {
    let Some(unit) = game.lists.client(client).and_then(|e| e.player) else {
        return Vec::new();
    };
    if sim.sys.units.get(unit).map(|r| r.ty) != Some(UnitType::Player) {
        return Vec::new();
    }
    let now = current(sim, game, unit, staged);
    let mut scratch = SyncCache::default();
    let cache = match sim.sys.hooks.sync.as_mut() {
        Some(s) => s.caches.entry(client).or_default(),
        None => &mut scratch,
    };
    sync::join(cache, &now).messages
}

#[cfg(test)]
mod tests;
