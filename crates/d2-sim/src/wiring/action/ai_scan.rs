//! The secondary target search on the wired units: the forced target, else
//! room scan 6 (mode 0) and the alternative choice (its trial path and
//! scan 7); and the not-evil main search's room scan 5 (mode 2). Called by
//! the AI world implementation in `ai.rs`.

use crate::game::Game;
use crate::monsters::ai::{AiStore, AiWorld};
use crate::units::{RoomId, UnitId, UnitType};

use super::{Pending, View};

/// Scan 6 radius: full-size distance below 49 (`0x31`).
pub const SCAN6_RANGE: i32 = 49;
/// Scan 5 range of the not-evil main search (35 is in).
pub const SCAN5_RANGE: i32 = 35;
/// State 146 (`invis`).
pub const STATE_INVIS: u32 = 146;
/// Unit flag 0x4.
pub const UNIT_FLAG_4: u32 = 0x4;
/// Threat class of a player.
pub const PLAYER_THREAT: u8 = 14;
/// An alternative farther than this is refused by the alternative choice.
pub const ALT_MAX_DISTANCE: i32 = 5;
/// A scan 7 unit closer than this replaces main in the alternative choice.
pub const SCAN7_REPLACE: i32 = 20;
/// Steps of the D' owner walk in [`View::hostile_live`].
const OWNER_WALK_LIMIT: usize = 256;
/// No distance.
const NONE_D: i32 = 0x7FFF_FFFF;

/// The scan 6 context {main, main d, alt, alt d}.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scan6 {
    /// The nearest main-class candidate and its distance.
    pub main: Option<(UnitId, i32)>,
    /// The nearest alternative-class candidate and its distance.
    pub alt: Option<(UnitId, i32)>,
}

impl<X: Pending> View<'_, X> {
    /// Scan mode 0: every unit of every room of the
    /// unit's room's near-room list (own room included), in list order.
    pub(crate) fn near_room_units(&self, game: &Game, unit: UnitId) -> Vec<UnitId> {
        use crate::path::collision::CollisionRooms;
        let Some(room) = game.lists.unit(unit).and_then(|e| e.room()) else {
            return Vec::new();
        };
        let d = &self.h.drlg;
        let mut rooms: Vec<RoomId> = (0..d.adjacent_count(room))
            .filter_map(|i| d.adjacent(room, i))
            .collect();
        if !rooms.contains(&room) {
            rooms.insert(0, room);
        }
        rooms
            .into_iter()
            .flat_map(|r| game.lists.room_units(r))
            .collect()
    }

    /// Scan mode 2: a living player or monster scanner with a room; the
    /// rooms of its room's near-room list (own room included) in list
    /// order, skipping a room in town or with client count 0; every unit
    /// of each.
    pub(crate) fn near_room_units_with_clients(&self, game: &Game, unit: UnitId) -> Vec<UnitId> {
        use crate::path::collision::CollisionRooms;
        if !self.living_actor(unit) {
            return Vec::new();
        }
        let Some(room) = game.lists.unit(unit).and_then(|e| e.room()) else {
            return Vec::new();
        };
        let d = &self.h.drlg;
        let mut rooms: Vec<RoomId> = (0..d.adjacent_count(room))
            .filter_map(|i| d.adjacent(room, i))
            .collect();
        if !rooms.contains(&room) {
            rooms.insert(0, room);
        }
        rooms
            .into_iter()
            .filter(|&r| !d.in_town(game, r) && d.client_count(game, r) != 0)
            .flat_map(|r| game.lists.room_units(r))
            .collect()
    }

    /// The threat class: 14 for a player, monstats `threat`
    /// for a monster, else 0.
    fn threat(&self, c: UnitId) -> u8 {
        match self.units.get(c).map(|r| (r.ty, r.class)) {
            Some((UnitType::Player, _)) => PLAYER_THREAT,
            Some((UnitType::Monster, class)) => self
                .h
                .tables
                .combat
                .monstats
                .get(class as usize)
                .map_or(0, |m| m.threat),
            _ => 0,
        }
    }

    /// Room scan 5: candidates that are living players or monsters with
    /// unit flag 4 and hostile to the scanner, at full-size distance with
    /// the candidate's size subtracted of at most 35; the nearest main
    /// (threat ≥ 2) and alternative (threat < 2), ties keep the earlier
    /// unit; with `los`, a blocked line (mask 4) skips. The game's extra
    /// rule that clears the line-of-sight flag needs alignment 0, which the
    /// not-evil search never has.
    pub fn scan5(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        los: bool,
        ai: Option<&AiStore>,
    ) -> Scan6 {
        let mut out = Scan6 {
            main: None,
            alt: None,
        };
        let at = self.h.path_position(unit);
        for c in self.near_room_units_with_clients(game, unit) {
            // A living, flagged, hostile candidate.
            if !self.living_actor(c)
                || self.units.get(c).map_or(0, |r| r.flags) & UNIT_FLAG_4 == 0
                || !self.hostile_live(game, ai, unit, c)
            {
                continue;
            }
            // Within range, with the candidate's size subtracted.
            let size = self.path_size(c);
            let d = crate::monsters::ai::distance_full_size(self.h.path_position(c), size, at);
            if d > SCAN5_RANGE {
                continue;
            }
            // Nearer than the one held for its class.
            let main = self.threat(c) >= 2;
            let held = if main { out.main } else { out.alt };
            if d >= held.map_or(NONE_D, |(_, hd)| hd) {
                continue;
            }
            // A clear line, when asked for.
            if los && self.line_blocked(game, c, unit) {
                continue;
            }
            if main {
                out.main = Some((c, d));
            } else {
                out.alt = Some((c, d));
            }
        }
        out
    }

    /// The not-evil main search: scan 5, then the alternative choice when
    /// an alternative was found.
    pub(crate) fn good_search(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        los: bool,
        ai: Option<&AiStore>,
    ) -> Option<(UnitId, i32)> {
        let s = self.scan5(game, unit, los, ai);
        self.alternative_choice(game, unit, s, ai)
    }

    /// Minion owner: a monster's AI control owner (the unit of the stored
    /// owner type and GUID), looked up at every call;
    /// `ai` is the store when it is lent out of the view.
    fn minion_owner_live(&self, game: &Game, ai: Option<&AiStore>, u: UnitId) -> Option<UnitId> {
        if self.units.get(u)?.ty != UnitType::Monster {
            return None;
        }
        let o = ai.or(self.h.ai.as_ref())?.control(u)?.minion_owner?;
        game.lists.find_unit(o.ty, o.guid)
    }

    /// Alignment as hostility reads it. A player without the state-105
    /// list reads 2: in the game every player has it, since the player init
    /// ends by setting alignment 2, which the sim's player allocation does
    /// not run yet.
    fn hostility_alignment(&self, u: UnitId) -> u8 {
        let player = self.units.get(u).is_some_and(|r| r.ty == UnitType::Player);
        if player
            && self
                .state_list(u, crate::combat::range::STATE_ALIGNMENT)
                .is_none()
        {
            return crate::combat::range::ALIGN_GOOD as u8;
        }
        self.alignment_stat(u)
    }

    /// Hostility of `a` toward `d`, read live:
    /// A' := a monster's minion owner (a missile with the source link:
    /// its source), when there is one; D' climbs minion owners while it
    /// is a monster whose owner exists and is not itself. A' = D' → not
    /// hostile; two players → the players' relation
    /// ([`Pending::may_attack`]); else hostile unless both alignments are
    /// 0 or both are 2. So a confused monster's temporary
    /// alignment counts.
    pub(crate) fn hostile_live(
        &self,
        game: &Game,
        ai: Option<&AiStore>,
        a: UnitId,
        d: UnitId,
    ) -> bool {
        let a = match self.units.get(a) {
            Some(r) if r.ty == UnitType::Monster => self.minion_owner_live(game, ai, a),
            Some(r) if r.ty == UnitType::Missile && r.flags2 & 0x400 != 0 => {
                let (ty, guid) = r.source;
                UnitType::ALL
                    .get(ty as usize)
                    .and_then(|&ty| game.lists.find_unit(ty, guid))
            }
            _ => None,
        }
        .unwrap_or(a);
        let mut d = d;
        // The game walks until a self-owner or none; the bound only stops a
        // longer owner cycle, which the pack links never form.
        for _ in 0..OWNER_WALK_LIMIT {
            match self.minion_owner_live(game, ai, d) {
                Some(o) if o != d => d = o,
                _ => break,
            }
        }
        if a == d {
            return false;
        }
        let players = [a, d]
            .iter()
            .all(|&u| self.units.get(u).is_some_and(|r| r.ty == UnitType::Player));
        if players {
            return self.h.x.may_attack(a, d);
        }
        let (aa, ad) = (self.hostility_alignment(a), self.hostility_alignment(d));
        !((aa == 0 && ad == 0) || (aa == 2 && ad == 2))
    }

    fn living_actor(&self, u: UnitId) -> bool {
        self.units
            .get(u)
            .is_some_and(|r| matches!(r.ty, UnitType::Player | UnitType::Monster))
            && !self.units.is_dead(u)
    }

    /// The scan 6 filter for scanner `s` and candidate `c`.
    fn scan6_filter(
        &mut self,
        game: &mut Game,
        ai: Option<&AiStore>,
        s: UnitId,
        c: UnitId,
    ) -> bool {
        if !self.living_actor(s) || !self.living_actor(c) {
            return false;
        }
        let Some(room) = game.lists.unit(c).and_then(|e| e.room()) else {
            return false;
        };
        if self.h.drlg.in_town(game, room) {
            return false;
        }
        if self.units.get(c).map_or(0, |r| r.flags) & UNIT_FLAG_4 == 0 {
            return false;
        }
        if self.stats.has_state(c, STATE_INVIS) {
            let player = self.units.get(c).is_some_and(|r| r.ty == UnitType::Player);
            if player {
                // The roll on C's seed comes before the melee-range test;
                // 80 or more skips it.
                let r = self.seed(c).roll(100);
                if r < 80 && !self.in_melee_range(game, s, c) {
                    return false;
                }
            } else if self.in_melee_range(game, s, c) {
                return false;
            }
        }
        self.hostile_live(game, ai, s, c)
    }

    /// Room scan 6: the nearest main
    /// (threat ≥ 2) and alternative (threat < 2) candidates closer than
    /// 49 by the full-size distance; ties keep the earlier unit.
    pub fn scan6(&mut self, game: &mut Game, unit: UnitId, ai: Option<&AiStore>) -> Scan6 {
        let mut out = Scan6 {
            main: None,
            alt: None,
        };
        let at = self.h.path_position(unit);
        for c in self.near_room_units(game, unit) {
            // Passes the scan 6 filter.
            if !self.scan6_filter(game, ai, unit, c) {
                continue;
            }
            // Within range, with C's size subtracted.
            let size = self.path_size(c);
            let d = crate::monsters::ai::distance_full_size(self.h.path_position(c), size, at);
            if d >= SCAN6_RANGE {
                continue;
            }
            // Nearer than the one held for its threat class.
            let threat = match self.units.get(c).map(|r| (r.ty, r.class)) {
                Some((UnitType::Player, _)) => PLAYER_THREAT,
                Some((_, class)) => self
                    .h
                    .tables
                    .combat
                    .monstats
                    .get(class as usize)
                    .map_or(0, |m| m.threat),
                None => 0,
            };
            let main = threat >= 2;
            let held = if main { out.main } else { out.alt };
            if d >= held.map_or(NONE_D, |(_, hd)| hd) {
                continue;
            }
            // A clear line.
            if self.line_blocked(game, c, unit) {
                continue;
            }
            if main {
                out.main = Some((c, d));
            } else {
                out.alt = Some((c, d));
            }
        }
        out
    }

    /// The secondary search: the forced target, else scan 6 and the
    /// alternative choice; target, distance (0x7FFFFFFF
    /// when none) and the melee-range flag.
    pub(crate) fn secondary_search(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        ai: Option<&AiStore>,
    ) -> (Option<UnitId>, i32, bool) {
        // The forced target first.
        let picked = match self.forced(game, unit, false, true, ai) {
            Some(f) => Some(f),
            None => {
                let s = self.scan6(game, unit, ai);
                self.alternative_choice(game, unit, s, ai)
            }
        };
        match picked {
            Some((t, d)) => (Some(t), d, self.in_melee_range(game, unit, t)),
            None => (None, NONE_D, false),
        }
    }

    /// The alternative choice on a scan's main and alternative: never for a
    /// player scanner or without an alternative; the alternative when
    /// there is no main target; refused when farther than 5. Else a trial
    /// path toward main ([`Self::trial_path_points`]) with points keeps
    /// main; without points, scan 7 ([`Self::scan7`]) finds a nearer
    /// main-class unit: closer than 20 it replaces main, else the
    /// alternative is taken.
    pub(crate) fn alternative_choice(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        s: Scan6,
        ai: Option<&AiStore>,
    ) -> Option<(UnitId, i32)> {
        let is_player = self
            .units
            .get(unit)
            .is_some_and(|r| r.ty == UnitType::Player);
        let Some((a, ad)) = s.alt.filter(|_| !is_player) else {
            return s.main;
        };
        let Some((m, _)) = s.main else {
            return Some((a, ad));
        };
        if ad > ALT_MAX_DISTANCE {
            return s.main;
        }
        if self.trial_path_points(game, unit, m) != 0 {
            return s.main;
        }
        match self.scan7(game, unit, m, ai) {
            Some((c, d)) if d < SCAN7_REPLACE => Some((c, d)),
            _ => Some((a, ad)),
        }
    }

    /// The AI's trial path, step 5: the unit's path record aimed at `main` (type
    /// 2), computed `(path, unit,
    /// 0)`, then type, target unit and target point put back; the point
    /// setter clears the target unit, and the trial's points
    /// stay. Its point count. The game always runs the
    /// trial; the fallback
    /// without the path provider (trial not run, main kept) is D2RUST's own
    /// and reached only by harnesses, the full engine always has one.
    fn trial_path_points(&mut self, game: &mut Game, unit: UnitId, main: UnitId) -> u32 {
        use crate::path::record::TargetUnit;
        let Some(tu) = self.units.get(main).map(|r| TargetUnit {
            unit: main,
            ty: r.ty,
            guid: r.guid,
        }) else {
            return u32::MAX;
        };
        let Some(p) = self.h.paths.as_mut() else {
            return u32::MAX;
        };
        let tables = p.tables.clone();
        let Some(d) = p.dynamic_mut(unit) else {
            return u32::MAX;
        };
        let saved = (d.path_type, d.target_unit, d.target_x, d.target_y);
        d.target_unit = Some(tu);
        let r = d.set_path_type(&tables, false, crate::path::record::path_types::TOWARD);
        if let Err(e) = r {
            self.h.errors.push(super::WiringError::Path(e));
        }
        crate::wiring::path::walk::build_path(self, game, unit);
        let Some(p) = self.h.paths.as_mut() else {
            return u32::MAX;
        };
        let Some(d) = p.dynamic_mut(unit) else {
            return u32::MAX;
        };
        d.target_unit = saved.1;
        d.set_target_point(saved.2, saved.3);
        let r = d.set_path_type(&tables, false, saved.0);
        let n = d.point_count;
        if let Err(e) = r {
            self.h.errors.push(super::WiringError::Path(e));
        }
        n
    }

    /// Room scan 7 (mode 0): `main`
    /// itself skipped (before the filter), then the scan 6 filter, a
    /// full-size distance closer than 49, threat class ≥ 2, strictly
    /// nearer than the best so far, and a clear line (mask 4).
    fn scan7(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        main: UnitId,
        ai: Option<&AiStore>,
    ) -> Option<(UnitId, i32)> {
        let at = self.h.path_position(unit);
        let mut best: Option<(UnitId, i32)> = None;
        for c in self.near_room_units(game, unit) {
            if c == main || !self.scan6_filter(game, ai, unit, c) {
                continue;
            }
            let size = self.path_size(c);
            let d = crate::monsters::ai::distance_full_size(self.h.path_position(c), size, at);
            if d >= SCAN6_RANGE
                || self.threat(c) < 2
                || d >= best.map_or(NONE_D, |(_, bd)| bd)
                || self.line_blocked(game, c, unit)
            {
                continue;
            }
            best = Some((c, d));
        }
        best
    }
}
