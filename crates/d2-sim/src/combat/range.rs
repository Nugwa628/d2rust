//! Range and hostility rules of combat: who may attack whom, how far a melee reach is,
//! and whether a target is within it.
//!
//! `hostile`, `in_melee_range` and `melee_range` are pure rules over [`RangeWorld`]. A host
//! implements [`RangeWorld`] from its unit data and answers the [`super::CombatWorld`]
//! seams (`hostile`, `in_melee_range`, `melee_range`) with these functions.

use crate::units::UnitType;

/// Alignment of the evil side (alignment values: evil 0, neutral 1, good 2).
pub const ALIGN_EVIL: i32 = 0;
/// Alignment of the good side.
pub const ALIGN_GOOD: i32 = 2;
/// `BaseId` of the first tentacle monster (see the tentacle rule in `in_melee_range`).
pub const BASE_TENTACLE1: i32 = 258;
/// `BaseId` of the tentacle head monster.
pub const BASE_TENTACLEHEAD1: i32 = 261;
/// The collision mask of the melee line-of-sight test.
pub const MELEE_LINE_MASK: u32 = 0x804;
/// Weapon class `2ht` (two-handed thrust/sword), read by `melee_range` for monsters.
pub const WEAPON_CLASS_2HT: i32 = 6;
/// The hostile bit of a player's relation entry for another player.
pub const RELATION_HOSTILE: u32 = 8;
/// State 105 (`alignment`) that carries the alignment stat.
pub const STATE_ALIGNMENT: u16 = 105;
/// Stat 172, the alignment value stored in state 105.
pub const STAT_ALIGNMENT: u16 = 172;

/// The unit queries the rules read.
pub trait RangeWorld {
    /// Handle for a unit in the host's world.
    type Unit: Copy + PartialEq;
    /// The kind of the unit.
    fn unit_type(&self, u: Self::Unit) -> UnitType;
    /// The owner of a unit, resolved.
    fn owner(&self, u: Self::Unit) -> Option<Self::Unit>;
    /// A missile's source-unit link (the unit stored by type and GUID on the missile),
    /// resolved; `None` without the link or when the unit does not exist.
    fn missile_source(&self, u: Self::Unit) -> Option<Self::Unit>;
    /// The unit's GUID.
    fn guid(&self, u: Self::Unit) -> u32;
    /// The flags of the first relation entry with `guid` in `a`'s player data; `None`
    /// without an entry.
    fn relation_flags(&self, a: Self::Unit, guid: u32) -> Option<u32>;
    /// Stat 172 of the unit's state-105 list; 0 without a stat holder or without the
    /// list.
    fn alignment_stat(&self, u: Self::Unit) -> i32;
    /// A monster's `BaseId`.
    fn base_id(&self, u: Self::Unit) -> i32;
    /// Distance between two units.
    fn distance(&self, a: Self::Unit, b: Self::Unit) -> i32;
    /// True when the collision line between `a` and `b` is blocked for `mask`.
    fn line_blocked(&self, a: Self::Unit, b: Self::Unit, mask: u32) -> bool;
    /// The `rangeadder` (weapons table) of the player's chosen weapon; 0 without one.
    fn weapon_range_adder(&self, u: Self::Unit) -> i32;
    /// `MeleeRng` (monstats2) of the monster's class row; `None` without a row.
    fn melee_rng(&self, u: Self::Unit) -> Option<u8>;
    /// The weapon class in the unit's current mode.
    fn weapon_class_now(&self, u: Self::Unit) -> i32;
}

/// Alignment of a unit: players and monsters read stat 172 of the state-105 list; other
/// unit types are 2 (good), and no unit counts as evil.
pub fn alignment<W: RangeWorld>(w: &W, u: Option<W::Unit>) -> i32 {
    let Some(u) = u else {
        return ALIGN_EVIL;
    };
    match w.unit_type(u) {
        UnitType::Player | UnitType::Monster => w.alignment_stat(u),
        _ => ALIGN_GOOD,
    }
}

/// Whether `a` may attack `d`. `a` is `None` for an attacker without a unit.
pub fn hostile<W: RangeWorld>(w: &W, a: Option<W::Unit>, d: W::Unit) -> bool {
    // Attackers act as their owner (monsters) or source (missiles).
    let a2 = a.map(|a| match w.unit_type(a) {
        UnitType::Monster => w.owner(a).unwrap_or(a),
        UnitType::Missile => w.missile_source(a).unwrap_or(a),
        _ => a,
    });
    // Climb the defender's owner chain until a monster has no owner or is its own
    // owner. The walk is bounded so a cycle of owners cannot hang it.
    let mut d2 = d;
    for _ in 0..256 {
        if w.unit_type(d2) != UnitType::Monster {
            break;
        }
        match w.owner(d2) {
            Some(o) if o != d2 => d2 = o,
            _ => break,
        }
    }
    // After resolving owners and sources, a unit is never hostile to itself.
    if a2 == Some(d2) {
        return false;
    }
    // Two players: only the relation entry's hostile bit counts.
    if let Some(a2) = a2 {
        if w.unit_type(a2) == UnitType::Player && w.unit_type(d2) == UnitType::Player {
            // The entry is looked up by the GUID of D (not D').
            return w
                .relation_flags(a2, w.guid(d))
                .is_some_and(|f| f & RELATION_HOSTILE != 0);
        }
    }
    // Otherwise opposite sides, or a neutral party, are hostile.
    let (aa, ad) = (alignment(w, a2), alignment(w, Some(d2)));
    !((aa == ALIGN_EVIL && ad == ALIGN_EVIL) || (aa == ALIGN_GOOD && ad == ALIGN_GOOD))
}

/// Melee reach of a unit: a player's weapon range adder, a monster's `MeleeRng`.
pub fn melee_range<W: RangeWorld>(w: &W, u: W::Unit) -> i32 {
    match w.unit_type(u) {
        UnitType::Player => w.weapon_range_adder(u),
        UnitType::Monster => match w.melee_rng(u) {
            None => 0,
            Some(255) => {
                if w.weapon_class_now(u) == WEAPON_CLASS_2HT {
                    2
                } else {
                    0
                }
            }
            Some(r) => i32::from(r),
        },
        _ => 0,
    }
}

/// Whether `a` can melee `b` with `extra` added reach.
pub fn in_melee_range<W: RangeWorld>(
    w: &W,
    a: Option<W::Unit>,
    b: Option<W::Unit>,
    extra: i32,
) -> bool {
    let (Some(a), Some(b)) = (a, b) else {
        return false;
    };
    // Tentacles count as in range from a wider reach, with no line test.
    if w.unit_type(b) == UnitType::Monster
        && matches!(w.base_id(b), BASE_TENTACLE1 | BASE_TENTACLEHEAD1)
    {
        let d = w.distance(a, b);
        if melee_range(w, a) + 8 > d {
            return true;
        }
    }
    // Plain rule: within reach plus one, and a clear line.
    let r = melee_range(w, a).wrapping_add(extra).wrapping_add(1);
    let d = w.distance(a, b);
    if d <= 0 {
        return true;
    }
    if r < d {
        return false;
    }
    !w.line_blocked(a, b, MELEE_LINE_MASK)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct W {
        ty: BTreeMap<usize, UnitType>,
        owner: BTreeMap<usize, usize>,
        msrc: BTreeMap<usize, usize>,
        rel: BTreeMap<(usize, u32), u32>,
        align: BTreeMap<usize, i32>,
        base: BTreeMap<usize, i32>,
        dist: i32,
        blocked: bool,
        adder: i32,
        rng: BTreeMap<usize, u8>,
        wclass: i32,
    }

    impl W {
        fn add(&mut self, u: usize, ty: UnitType, align: i32) {
            self.ty.insert(u, ty);
            self.align.insert(u, align);
        }
    }

    impl RangeWorld for W {
        type Unit = usize;
        fn unit_type(&self, u: usize) -> UnitType {
            self.ty[&u]
        }
        fn owner(&self, u: usize) -> Option<usize> {
            self.owner.get(&u).copied()
        }
        fn missile_source(&self, u: usize) -> Option<usize> {
            self.msrc.get(&u).copied()
        }
        fn guid(&self, u: usize) -> u32 {
            u as u32 + 100
        }
        fn relation_flags(&self, a: usize, guid: u32) -> Option<u32> {
            self.rel.get(&(a, guid)).copied()
        }
        fn alignment_stat(&self, u: usize) -> i32 {
            self.align.get(&u).copied().unwrap_or(0)
        }
        fn base_id(&self, u: usize) -> i32 {
            self.base.get(&u).copied().unwrap_or(0)
        }
        fn distance(&self, _: usize, _: usize) -> i32 {
            self.dist
        }
        fn line_blocked(&self, _: usize, _: usize, mask: u32) -> bool {
            assert_eq!(mask, 0x804);
            self.blocked
        }
        fn weapon_range_adder(&self, _: usize) -> i32 {
            self.adder
        }
        fn melee_rng(&self, u: usize) -> Option<u8> {
            self.rng.get(&u).copied()
        }
        fn weapon_class_now(&self, _: usize) -> i32 {
            self.wclass
        }
    }

    #[test]
    fn hostility() {
        let mut w = W::default();
        w.add(0, UnitType::Player, 2); // player A
        w.add(1, UnitType::Player, 2); // player B
        w.add(2, UnitType::Monster, 0); // evil monster
        w.add(3, UnitType::Monster, 0); // evil monster
        w.add(4, UnitType::Monster, 2); // good monster
        w.add(5, UnitType::Monster, 1); // neutral monster
        w.add(6, UnitType::Missile, 0);
        w.add(7, UnitType::Object, 0);
        // Alignment: a player and an evil monster are on opposite sides; two evil
        // monsters and two good units are on the same side; a neutral unit
        // is hostile to everyone.
        assert!(hostile(&w, Some(0), 2));
        assert!(hostile(&w, Some(2), 0));
        assert!(!hostile(&w, Some(2), 3));
        assert!(!hostile(&w, Some(4), 0));
        assert!(hostile(&w, Some(5), 2));
        assert!(hostile(&w, Some(2), 5));
        assert!(!hostile(&w, Some(5), 5)); // 3: A' = D'
                                           // Other unit types count as good (2).
        assert!(!hostile(&w, Some(0), 7));
        assert!(hostile(&w, Some(2), 7));
        // The same unit -> 0; both none -> 0 (here: no attacker, evil target
        // has alignment 0 like "none").
        assert!(!hostile(&w, Some(2), 2));
        assert!(!hostile(&w, None, 2));
        // A monster with an owner attacks as its owner; a monster
        // defender with an owner is judged as the owner.
        w.owner.insert(4, 2); // good monster 4 owned by evil monster 2
        assert!(hostile(&w, Some(4), 0)); // as unit 2 vs player 0
        assert!(!hostile(&w, Some(4), 3)); // as unit 2 vs evil 3
        w.owner.insert(3, 0); // evil monster 3 is player 0's minion
        assert!(!hostile(&w, Some(0), 3)); // A' = D' = player 0
        assert!(hostile(&w, Some(2), 3)); // evil 2 vs player 0
                                          // A missile attacks as its source unit.
        w.msrc.insert(6, 2);
        assert!(hostile(&w, Some(6), 0));
        // ...and the good monster 4 is judged as its owner 2: same unit.
        assert!(!hostile(&w, Some(6), 4));
        // Two players: only the relation entry's hostile bit; no entry
        // (single player) -> never hostile.
        assert!(!hostile(&w, Some(0), 1));
        w.rel.insert((0, 101), 0x7);
        assert!(!hostile(&w, Some(0), 1));
        w.rel.insert((0, 101), 0x8);
        assert!(hostile(&w, Some(0), 1));
        assert!(!hostile(&w, Some(1), 0));
    }

    #[test]
    fn melee_range_by_unit_type() {
        let mut w = W::default();
        w.add(0, UnitType::Player, 2);
        w.add(1, UnitType::Monster, 0);
        w.add(2, UnitType::Monster, 0);
        w.add(3, UnitType::Object, 0);
        // Player: the weapon's rangeadder.
        w.adder = 3;
        assert_eq!(melee_range(&w, 0), 3);
        // Monster: MeleeRng; 255 -> 2 with a `2ht` weapon, else 0; no row
        // -> 0.
        w.rng.insert(1, 4);
        w.rng.insert(2, 255);
        assert_eq!(melee_range(&w, 1), 4);
        assert_eq!(melee_range(&w, 2), 0);
        w.wclass = WEAPON_CLASS_2HT;
        assert_eq!(melee_range(&w, 2), 2);
        w.ty.insert(9, UnitType::Monster);
        assert_eq!(melee_range(&w, 9), 0);
        // Other unit types: 0.
        assert_eq!(melee_range(&w, 3), 0);
    }

    #[test]
    fn in_range() {
        let mut w = W::default();
        w.add(0, UnitType::Player, 2);
        w.add(1, UnitType::Monster, 0);
        w.add(2, UnitType::Monster, 0);
        w.adder = 1;
        // A unit missing -> false.
        assert!(!in_melee_range(&w, None, Some(1), 1));
        assert!(!in_melee_range(&w, Some(0), None, 1));
        // r = range 1 + extra 1 + 1 = 3. d <= 0 -> 1 (even blocked).
        w.blocked = true;
        w.dist = 0;
        assert!(in_melee_range(&w, Some(0), Some(1), 1));
        w.dist = -4;
        assert!(in_melee_range(&w, Some(0), Some(1), 1));
        // r < d -> 0; r >= d needs a clear line.
        w.dist = 4;
        assert!(!in_melee_range(&w, Some(0), Some(1), 1));
        w.dist = 3;
        assert!(!in_melee_range(&w, Some(0), Some(1), 1)); // blocked
        w.blocked = false;
        assert!(in_melee_range(&w, Some(0), Some(1), 1));
        // A tentacle target is in range when melee_range + 8 > d, with
        // no line test.
        w.base.insert(2, 258);
        w.blocked = true;
        w.dist = 8;
        assert!(in_melee_range(&w, Some(0), Some(2), 1)); // 1 + 8 > 8
        w.dist = 9;
        assert!(!in_melee_range(&w, Some(0), Some(2), 1)); // falls through
        w.base.insert(2, 261);
        w.dist = 8;
        assert!(in_melee_range(&w, Some(0), Some(2), 1));
        // Any other base id takes the plain rule.
        w.base.insert(2, 259);
        assert!(!in_melee_range(&w, Some(0), Some(2), 1));
    }
}
