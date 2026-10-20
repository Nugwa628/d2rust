//! A player's death on the action wiring: the death penalties (run from
//! the DT start, [`crate::units::hooks::UnitHooks::player_death`]), the
//! corpse experience at corpse creation (run from the DD start,
//! [`crate::units::hooks::UnitHooks::player_corpse`]) and the corpse pickup.
//!
//! The killer K is the DT start's unit target: the host that starts DT puts
//! the attacker's type and GUID there ([`ActionHooks::mode_target`]).
//!
//! The client's "experience lost" is [`DeathState::exp_lost`], keyed by the
//! player (one client per player).

// The hooks also run the status bit, the dead clean-up, the 0x5A notice and
// the arena event.
// A player killed by a Fallen's hit: the DT frame sends 0x5A code 6 (killer class 19, type 1), 0x0D code 8
// and the 0x65 arena update equal on both sides. The ear drop needs a
// killer that is another player or another player's minion: not reachable
// in single player. Measured with the Horadric Cube open (check
// `w3-05-death-cube`, f30): the DT frame sends 0x5A, 0x0D code 8, 0x1D
// (stat 175) and 0x65 and nothing for the cube, equal on both sides (all
// 208 S→C records equal); the cube close on the wire is the client's C→S
// 0x4F 0x17 (a client cause, handled by the client-message wiring).
// PROVISIONAL, still missing: clearing unit flag 2 when the player has no
// state 7, the interaction end (silent on the wire) and the character save
// at the DD start.

use std::collections::BTreeMap;

use crate::combat::vitals::experience::{
    corpse_experience, corpse_pickup, death_experience, gold_penalty,
};
use crate::combat::vitals::{stat, VitalsUnits};
use crate::units::hooks::Sim;
use crate::units::{UnitId, UnitType};

use super::combat::CombatView;
use super::{ActionHooks, Pending, View};

/// Stats of the gold penalty.
pub mod gold_stat {
    /// Gold carried.
    pub const GOLD: u16 = 14;
    /// Gold in the stash.
    pub const GOLDBANK: u16 = 15;
    /// Gold lost to the death penalty.
    pub const GOLDLOST: u16 = 175;
}

/// State 7 `playerbody`.
pub const STATE_PLAYERBODY: u32 = 7;

/// The gold limit per level: level × 10000.
pub const GOLD_PER_LEVEL: i32 = 10_000;

/// The death state of the clients.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeathState {
    /// The client's experience loss per player from the last death (the
    /// death penalties write it, the corpse creation reads and clears it).
    pub exp_lost: BTreeMap<UnitId, u32>,
    /// The owner GUID of each corpse this wiring allocated (the
    /// inventory of a corpse is not modelled).
    pub owners: BTreeMap<UnitId, u32>,
    /// Allocate the corpse unit when [`Pending::create_corpse`] gives
    /// none (the preview host; off in the rule tests).
    pub allocate_corpses: bool,
    /// The last death code sent per player (8 DT, 9 DD): each goes out
    /// once ([`super::dying`]).
    pub announced: BTreeMap<UnitId, u8>,
    /// Players that have died: their dispatch gate follows the live mode
    /// (the host stages the others, `adapters::SimGame::set_player`).
    pub died: std::collections::BTreeSet<UnitId>,
    /// Corpses allocated and not yet announced to the clients.
    pub fresh: Vec<UnitId>,
    /// (player, corpse) pairs whose items have not been copied yet: the host
    /// that holds the inventory model runs `items::moves::ground::
    /// corpse_fill` for each.
    pub loot: Vec<(UnitId, UnitId)>,
    /// (player, amount) gold drops of the death penalty: the host that
    /// holds the inventory model
    /// makes the piles (`items::moves::ground::gold_piles`).
    pub gold_drops: Vec<(UnitId, i32)>,
}

impl<X: Pending> ActionHooks<X> {
    /// The death penalties of player `p`, with the killer K =
    /// [`Self::mode_target`].
    pub fn death_penalties(&mut self, sim: &mut Sim<'_>, p: UnitId) {
        let k = self.mode_target;
        let player_side = k.is_some_and(|k| self.player_side(sim, k));
        let pvp = k.is_some_and(|k| k != p) && player_side;
        let game_type = self.ai_info.game_type;
        let penalty = self
            .tables
            .combat
            .difficulty(usize::from(sim.data.difficulty))
            .map_or(0, |d| d.deathexppenalty);
        let vitals = self.vitals.clone();
        let mut cv = CombatView {
            game: &mut *sim.game,
            v: View::of(sim.units, sim.stats, sim.data, self),
        };
        // Gold.
        let l = VitalsUnits::stat(&cv, p, stat::LEVEL);
        let gi = VitalsUnits::stat(&cv, p, gold_stat::GOLD);
        let gs = VitalsUnits::stat(&cv, p, gold_stat::GOLDBANK);
        let stash = cv.v.h.x.stash_cap(p);
        let g = gold_penalty(
            l,
            gi,
            gs,
            pvp,
            game_type,
            stash,
            l.wrapping_mul(GOLD_PER_LEVEL),
        );
        if let Some(v) = g.goldbank {
            cv.set_base_stat(p, gold_stat::GOLDBANK, v);
        }
        if pvp {
            if let Some(v) = g.gold {
                cv.set_base_stat(p, gold_stat::GOLD, v);
            }
            if let Some(q) = g.drop {
                cv.v.h.x.death_drop_gold(cv.game, p, q);
                cv.v.h.death.gold_drops.push((p, q));
            }
        } else {
            if let Some(q) = g.drop {
                cv.v.h.x.death_drop_gold(cv.game, p, q);
                cv.v.h.death.gold_drops.push((p, q));
            }
            if let Some(v) = g.gold {
                cv.set_base_stat(p, gold_stat::GOLD, v);
            }
        }
        cv.set_base_stat(p, gold_stat::GOLDLOST, g.goldlost);
        // Experience, skipped for a player-side killer.
        if player_side {
            return;
        }
        let Some(t) = vitals else {
            return;
        };
        if let Some(loss) = death_experience(&mut cv, &t, p, penalty) {
            cv.v.h.death.exp_lost.insert(p, loss as u32);
        }
    }

    /// K a player, or a monster whose owner is a player.
    fn player_side(&self, sim: &Sim<'_>, k: UnitId) -> bool {
        let ty = |u: UnitId| sim.units.get(u).map(|r| r.ty);
        match ty(k) {
            Some(UnitType::Player) => true,
            Some(UnitType::Monster) => self
                .minion_owner_of(sim.game, k)
                .is_some_and(|o| ty(o) == Some(UnitType::Player)),
            _ => false,
        }
    }

    /// Corpse creation: the corpse ([`Pending::create_corpse`]) gets stat 13
    /// := `pct(v, 75, 100)` of the client's experience loss, then the loss is
    /// reset to 0. With no corpse the loss is left unchanged.
    pub fn corpse_creation(&mut self, sim: &mut Sim<'_>, p: UnitId) {
        let made = self.x.create_corpse(sim.game, p);
        let Some(c) = made.or_else(|| self.allocate_corpse(sim, p)) else {
            return;
        };
        let v = self.death.exp_lost.get(&p).copied().unwrap_or(0) as i32;
        let mut cv = CombatView {
            game: &mut *sim.game,
            v: View::of(sim.units, sim.stats, sim.data, self),
        };
        cv.set_base_stat(c, stat::EXPERIENCE, corpse_experience(v));
        cv.v.h.death.exp_lost.insert(p, 0);
    }

    /// Corpse pickup by player P of corpse C: C has state 7 (`playerbody`)
    /// and P may take it (C's owner GUID is P's, or
    /// [`Pending::corpse_loot_allowed`]); its own player gets the corpse's
    /// experience back. Returns the experience returned, or none when
    /// refused; the item take-back and the rest of the pickup are the
    /// inventory's (`items::moves::ground`).
    pub fn corpse_pickup(&mut self, sim: &mut Sim<'_>, p: UnitId, c: UnitId) -> Option<i32> {
        // A corpse this wiring allocated has state 7 by construction (the
        // state table of a bare fixture may not hold the row).
        if !sim.stats.has_state(c, STATE_PLAYERBODY) && !self.death.owners.contains_key(&c) {
            return None;
        }
        let guid = sim.units.get(p).map(|r| r.guid);
        let owner = self
            .x
            .corpse_owner_guid(c)
            .or_else(|| self.death.owners.get(&c).copied());
        let own = owner.is_some() && owner == guid;
        if !own && !self.x.corpse_loot_allowed(c, p) {
            return None;
        }
        let mut x = 0;
        if let Some(t) = self.vitals.clone() {
            let mut cv = CombatView {
                game: &mut *sim.game,
                v: View::of(sim.units, sim.stats, sim.data, self),
            };
            x = corpse_pickup(&mut cv, &t, p, c, own);
        }
        Some(x)
    }
}
