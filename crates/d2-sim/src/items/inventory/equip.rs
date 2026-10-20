//! Equipping: body-location compatibility, requirements, the
//! equip check, hands compatible, the stack test,
//! equip from the cursor and auto-equip on pickup.

use super::{
    body, cmd, grid::place_at_body, iflag, mode, page, stat, targeting_reset, ty, InvTables,
    InvWorld, Inventory, UnitKind, NO_GUID,
};
use crate::combat::pct;
use crate::units::UnitId;

/// Itemtypes `class` "none".
pub const CLASS_NONE: u8 = 7;
/// Barbarian / assassin player classes.
pub const BARBARIAN: u8 = 4;
pub const ASSASSIN: u8 = 6;
/// Act 5 hireling monster classes.
pub const ACT5_HIRELINGS: [u32; 2] = [0x230, 0x231];
/// Monster classes that dual-wield `h2h`.
pub const H2H_MONSTERS: [u32; 2] = [0x1A1, 0x1A2];
/// Monster classes that dual-wield any weapons.
pub const DUAL_MONSTERS: [u32; 3] = [0x21C, 0x21D, 0x21E];

/// Equip-check results.
pub mod res {
    pub const NO: u8 = 0;
    pub const FREE: u8 = 1;
    pub const OTHER_HAND_BLOCKS: u8 = 2;
    pub const REMOVABLE: u8 = 3;
    pub const OTHER_HAND_TWO_HANDED: u8 = 4;
    pub const SWAP: u8 = 5;
    pub const STACK: u8 = 6;
    pub const SWAP_OTHER_TO_PAGE: u8 = 7;
}

/// Body-location compatibility: `loc` equals the
/// item type's `bodyloc1` or `bodyloc2`, or is 11 / 12 when
/// either is 4 or 5.
pub fn body_location_allowed(t: &InvTables, record: usize, loc: u8) -> bool {
    let Some(r) = t.itype_of(record) else {
        return false;
    };
    let (a, b) = (r.bodyloc1, r.bodyloc2);
    if loc == a || loc == b {
        return true;
    }
    let hand = |l: u8| l == body::RIGHT_HAND || l == body::LEFT_HAND;
    matches!(loc, body::SWAP_RIGHT | body::SWAP_LEFT) && (hand(a) || hand(b))
}

fn stat_ok<W: InvWorld + ?Sized>(
    w: &W,
    item: UnitId,
    unit: UnitId,
    equipping: bool,
    id: u16,
    req: i32,
) -> bool {
    let v = w.unit_stat(unit, id);
    if v < 1 || v < req {
        return false;
    }
    // Strength tests the stat-list link
    // before subtracting the socket contribution; dexterity
    // (step 4) goes straight to the dexterity stat read
    // with no link test. The asymmetry is reproduced.
    let linked = id != stat::STRENGTH || w.item_active_on(item, unit);
    if equipping && linked {
        let v = v - w.own_contribution(item, unit, id);
        if v < 1 || v < req {
            return false;
        }
    }
    true
}

/// Requirements of `item` for `unit`.
pub fn requirements_met<W: InvWorld + ?Sized>(
    w: &W,
    t: &InvTables,
    item: Option<UnitId>,
    unit: UnitId,
    equipping: bool,
) -> bool {
    // Step 1.
    let Some(item) = item else { return false };
    let Some(d) = w.item(item).copied() else {
        return false;
    };
    let Some(rec) = t.item(d.record) else {
        return false;
    };
    // Step 2.
    let (reqstr, reqdex) = (i32::from(rec.reqstr), i32::from(rec.reqdex));
    let p = w.req_percent(item);
    let (mut bonus_str, mut bonus_dex) = (0, 0);
    if p != 0 {
        // Apply the requirement percentage with signed, truncating arithmetic.
        bonus_str = pct(reqstr, p, 100);
        bonus_dex = pct(reqdex, p, 100);
    }
    if d.flags & iflag::ETHEREAL != 0 {
        bonus_str -= 10;
        bonus_dex -= 10;
    }
    // Steps 3–5.
    if !stat_ok(w, item, unit, equipping, stat::STRENGTH, reqstr + bonus_str) {
        return false;
    }
    if !stat_ok(
        w,
        item,
        unit,
        equipping,
        stat::DEXTERITY,
        reqdex + bonus_dex,
    ) {
        return false;
    }
    // Never returns a negative R, so the caller's "R = −1 → skip"
    // test never fires.
    if w.unit_stat(unit, stat::LEVEL) < w.level_requirement(item, unit) {
        return false;
    }
    // Step 6.
    if d.flags & iflag::IDENTIFIED == 0 {
        return false;
    }
    if rec.type_ == ty::BOOK && w.item_stat(item, stat::QUANTITY) <= 0 {
        return false;
    }
    let class = t.itype_of(d.record).map_or(CLASS_NONE, |r| r.class);
    if class == CLASS_NONE {
        return true;
    }
    match w.unit_kind(unit) {
        Some(UnitKind::Player { class: c }) => c == class,
        Some(UnitKind::Monster { class: m }) => ACT5_HIRELINGS.contains(&m) && class == BARBARIAN,
        _ => false,
    }
}

fn is_type<W: InvWorld + ?Sized>(w: &W, t: &InvTables, item: UnitId, ty: i16) -> bool {
    w.item(item).is_some_and(|d| t.is_type(d.record, ty))
}

/// One-or-two-handed for a unit of an item
/// of class `record`, for a unit of kind `unit`.
///
/// Items `1or2handed` ≠ 0 held by a player of class 4 (barbarian); every
/// other unit answers no (the base-game body).
pub fn one_or_two_handed_for(t: &InvTables, record: usize, unit: Option<UnitKind>) -> bool {
    matches!(unit, Some(UnitKind::Player { class: BARBARIAN }))
        && t.item(record).is_some_and(|r| r.onetwohanded != 0)
}

/// Hands compatible of `a` and `b` for `unit`.
pub fn hands_compatible<W: InvWorld + ?Sized>(
    w: &W,
    t: &InvTables,
    unit: UnitId,
    a: Option<UnitId>,
    b: Option<UnitId>,
) -> bool {
    // Step 1.
    let (Some(a), Some(b)) = (a, b) else {
        return true;
    };
    // Step 2.
    let ammo = |x: UnitId, y: UnitId| {
        w.ammo_type(x)
            .is_some_and(|am| am != 0 && is_type(w, t, y, am))
    };
    if ammo(a, b) || ammo(b, a) {
        return true;
    }
    // Step 3.
    if w.quiver_type(t, a) || w.quiver_type(t, b) {
        return false;
    }
    // Step 4.
    for x in [a, b] {
        if w.two_handed(x) && !w.one_or_two_handed(unit, x) {
            return false;
        }
    }
    // Step 5.
    let (wa, wb) = (is_type(w, t, a, ty::WEAP), is_type(w, t, b, ty::WEAP));
    if !(wa && wb) {
        return wa || wb;
    }
    // Step 6.
    let both_h2h = is_type(w, t, a, ty::H2H) && is_type(w, t, b, ty::H2H);
    match w.unit_kind(unit) {
        None => false,
        Some(UnitKind::Player { class }) => match class {
            BARBARIAN => true,
            ASSASSIN => both_h2h,
            _ => false,
        },
        Some(UnitKind::Monster { class }) => {
            (H2H_MONSTERS.contains(&class) && both_h2h) || DUAL_MONSTERS.contains(&class)
        }
        // Objects, missiles, items, tiles.
        Some(_) => false,
    }
}

/// Corpse slot fit of X for
/// `unit`, with D (the unit's item at L) and A (its item at the paired
/// location, or D). Returns (fit, L): step 3 moves L to the free partner
/// slot. Differs from [`hands_compatible`]: a quiver next to a bow passes, two non-weapons
/// fail, any monster passes two weapons.
pub fn corpse_slot_fit<W: InvWorld + ?Sized>(
    w: &W,
    t: &InvTables,
    unit: UnitId,
    x: UnitId,
    d: Option<UnitId>,
    a: Option<UnitId>,
    l: u8,
) -> (bool, u8) {
    // Step 1.
    if w.item(x).is_none() {
        return (false, l);
    }
    // Step 2.
    let (d, a, l) = match (d, a) {
        (None, None) => return (true, l),
        (Some(_), Some(_)) => return (false, l),
        // Step 3: the free partner slot.
        (Some(old), None) => (None::<UnitId>, Some(old), pair_location(l)),
        (None, Some(o)) => (None, Some(o), l),
    };
    // Step 4.
    if ![4, 5, 11, 12].contains(&l) {
        return (d.is_none(), l);
    }
    // Step 5, with O := A (present).
    let Some(o) = a else {
        return (true, l);
    };
    let s = |i: UnitId| w.ammo_type(i).unwrap_or(-1);
    let q = |i: UnitId| {
        w.item(i)
            .and_then(|r| t.itype_of(r.record))
            .map_or(0, |r| r.quiver as i16)
    };
    let fit = if s(x) > 0 {
        is_type(w, t, o, s(x))
    } else if q(x) != 0 {
        s(o) > 0 && is_type(w, t, x, s(o))
    } else if s(o) > 0 {
        is_type(w, t, x, s(o))
    } else if q(o) > 0 {
        is_type(w, t, x, q(o))
    } else if [x, o]
        .iter()
        .any(|&i| w.two_handed(i) && !w.one_or_two_handed(unit, i))
    {
        false
    } else {
        let (wx, wo) = (is_type(w, t, x, ty::WEAP), is_type(w, t, o, ty::WEAP));
        if !(wx || wo) {
            false
        } else if wx != wo {
            true
        } else {
            match w.unit_kind(unit) {
                Some(UnitKind::Monster { .. }) => true,
                Some(UnitKind::Player { class: BARBARIAN }) => true,
                Some(UnitKind::Player { class: ASSASSIN }) => {
                    is_type(w, t, x, ty::H2H) && is_type(w, t, o, ty::H2H)
                }
                _ => false,
            }
        }
    };
    (fit, l)
}

/// The paired body location: 4 ↔ 5, 6 ↔ 7, 11 ↔ 12, else 0.
pub fn pair_location(l: u8) -> u8 {
    match l {
        4 => 5,
        5 => 4,
        6 => 7,
        7 => 6,
        11 => 12,
        12 => 11,
        _ => 0,
    }
}

/// Quality q ≠ 0 and q ∉ 4–9, so low, normal or
/// superior (1–3).
pub fn stack_quality_ok(q: u8) -> bool {
    (1..=3).contains(&q)
}

/// Stack test: same class, quality and file index
/// from the item data, stackable, equal ethereal bits, both
/// qualities in 1–3, equal damage ranges and no sockets.
pub fn stack_test<W: InvWorld + ?Sized>(w: &W, t: &InvTables, a: UnitId, b: UnitId) -> bool {
    let (Some(da), Some(db)) = (w.item(a), w.item(b)) else {
        return false;
    };
    let stackable = |r: usize| t.item(r).is_some_and(|r| r.stackable != 0);
    const DAMAGE: [u16; 6] = [
        stat::MINDAMAGE,
        stat::MAXDAMAGE,
        stat::SECONDARY_MINDAMAGE,
        stat::SECONDARY_MAXDAMAGE,
        stat::THROW_MINDAMAGE,
        stat::THROW_MAXDAMAGE,
    ];
    da.record == db.record
        && w.quality(a) == w.quality(b)
        && w.stack_file_index(a) == w.stack_file_index(b)
        && stackable(da.record)
        && (da.flags & iflag::ETHEREAL) == (db.flags & iflag::ETHEREAL)
        && stack_quality_ok(w.quality(a))
        && stack_quality_ok(w.quality(b))
        && DAMAGE
            .iter()
            .all(|&s| w.item_stat(a, s) == w.item_stat(b, s))
        && !w.has_sockets(a)
        && !w.has_sockets(b)
}

/// The other hand of a hand location.
pub fn other_hand(loc: u8) -> Option<u8> {
    match loc {
        body::RIGHT_HAND => Some(body::LEFT_HAND),
        body::LEFT_HAND => Some(body::RIGHT_HAND),
        body::SWAP_RIGHT => Some(body::SWAP_LEFT),
        body::SWAP_LEFT => Some(body::SWAP_RIGHT),
        _ => None,
    }
}

/// Equip check: may `item` (none: "may `loc` be
/// emptied") go to `loc` of `unit`, whose inventory is `inv`. Returns a
/// [`res`] code.
pub fn equip_check<W: InvWorld + ?Sized>(
    inv: &Inventory,
    w: &W,
    t: &InvTables,
    unit: UnitId,
    loc: u8,
    item: Option<UnitId>,
    skip: bool,
) -> u8 {
    // Step 1.
    if let Some(n) = item {
        let Some(d) = w.item(n) else { return res::NO };
        if !body_location_allowed(t, d.record, loc) {
            return res::NO;
        }
        if !skip && !requirements_met(w, t, Some(n), unit, true) {
            return res::NO;
        }
    }
    // Step 2.
    if loc == body::NONE {
        return res::NO;
    }
    let target = inv.body_item(loc);
    // Step 3.
    let Some(o) = other_hand(loc) else {
        return match (item, target) {
            (Some(_), Some(_)) => res::SWAP,
            (Some(_), None) => res::FREE,
            (None, Some(_)) => res::REMOVABLE,
            (None, None) => res::NO,
        };
    };
    // Step 4.
    let other = inv.body_item(o);
    match (item, target, other) {
        (None, Some(_), _) => res::REMOVABLE,
        (None, None, Some(x)) if w.two_handed(x) => res::OTHER_HAND_TWO_HANDED,
        (None, None, _) => res::NO,
        (Some(n), Some(tt), x) => {
            if stack_test(w, t, tt, n) {
                res::STACK
            } else if hands_compatible(w, t, unit, Some(n), x) {
                res::SWAP
            } else if x.is_some_and(|x| w.fits_free_page0(inv, x)) {
                res::SWAP_OTHER_TO_PAGE
            } else {
                res::NO
            }
        }
        (Some(n), None, Some(x)) => {
            if hands_compatible(w, t, unit, Some(n), Some(x)) {
                res::FREE
            } else {
                res::OTHER_HAND_BLOCKS
            }
        }
        (Some(_), None, None) => res::FREE,
    }
}

/// A helper's result and its refusal flag "out" (result 0 with out ≠ 0
/// → handler 3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EquipOutcome {
    pub ok: bool,
    pub out: bool,
}

impl EquipOutcome {
    const REFUSED: EquipOutcome = EquipOutcome {
        ok: false,
        out: true,
    };
    const NO: EquipOutcome = EquipOutcome {
        ok: false,
        out: false,
    };
    const OK: EquipOutcome = EquipOutcome {
        ok: true,
        out: false,
    };
}

/// Put `item` at `loc` of the inventory's owner and mark it
/// with command flag `flag` (0x8 for [`equip_from_cursor`]; 0x10000 for 0x1B).
/// False when the put or the link check fails (out := 1).
pub fn equip_put<W: InvWorld + ?Sized>(
    inv: &mut Inventory,
    w: &mut W,
    t: &InvTables,
    item: UnitId,
    loc: u8,
    flag: u32,
) -> bool {
    let unit = inv.owner;
    let swap = matches!(loc, body::SWAP_RIGHT | body::SWAP_LEFT);
    if !w.link_check(unit, item, if swap { 4 } else { 3 }) || !place_at_body(inv, w, item, loc) {
        return false;
    }
    if let Some(d) = w.item_mut(item) {
        d.body_loc = loc;
    }
    if !swap {
        // The weapon-in-use link, then
        // the stat link and refresh.
        super::weapon::weapon_link(inv, w, t, item);
        w.stat_link_in_use(unit, item, inv.weapon_guid);
        w.stat_refresh(unit);
    }
    inv.put_cursor(w, None);
    w.clear_targetable(item);
    let mut guid = NO_GUID;
    if let Some(d) = w.item_mut(item) {
        d.mode = mode::EQUIPPED;
        d.page = page::NONE;
        d.cmd_flags |= flag;
        d.flags |= iflag::CHANGED;
        d.flags &= !iflag::F4000;
        guid = d.guid;
    }
    inv.push_update(guid);
    w.owner_refresh(unit);
    w.weapon_bookkeeping(unit, item);
    w.inventory_pass(unit);
    true
}

/// Equip from the cursor: the cursor item `item` of
/// the inventory's owner to body location `loc`.
pub fn equip_from_cursor<W: InvWorld + ?Sized>(
    inv: &mut Inventory,
    w: &mut W,
    t: &InvTables,
    item: Option<UnitId>,
    loc: u8,
    skip: bool,
) -> EquipOutcome {
    let unit = inv.owner;
    // Step 1.
    targeting_reset(inv, w);
    let Some(item) = item else {
        return EquipOutcome::REFUSED;
    };
    let Some(d) = w.item(item).copied() else {
        return EquipOutcome::NO;
    };
    if d.mode != mode::CURSOR {
        return EquipOutcome::REFUSED;
    }
    // Step 2.
    if equip_check(inv, w, t, unit, loc, Some(item), skip) != res::FREE {
        return EquipOutcome::NO;
    }
    // Step 3.
    if !skip && !requirements_met(w, t, Some(item), unit, false) {
        return EquipOutcome::REFUSED;
    }
    // Step 4.
    if !matches!(loc, body::SWAP_RIGHT | body::SWAP_LEFT) && inv.weapon_guid != NO_GUID {
        w.weapon_in_use_update(unit);
    }
    // Step 5.
    if !equip_put(inv, w, t, item, loc, cmd::EQUIP) {
        return EquipOutcome::REFUSED;
    }
    EquipOutcome::OK
}

/// Equip without the cursor (skip `skip`):
/// gives the location L (none → false, nothing changed); put at L on the
/// body grid and link (kind 3); body location := L, stat link, cursor :=
/// none, stat refresh, unit flag 0x2 cleared, mode 1, page 0xFF, command
/// flag 0x200, update list, owner refresh, weapon bookkeeping. No item
/// flag 0x1, item flag 0x4000 kept, no inventory pass (the differences
/// from [`equip_from_cursor`]). The fatal asserts of step 2 return false. Unit flag
/// 0x2000000 is the caller's (it holds the unit flags).
pub fn equip_without_cursor<W: InvWorld + ?Sized>(
    inv: &mut Inventory,
    w: &mut W,
    t: &InvTables,
    item: UnitId,
    skip: bool,
) -> bool {
    let unit = inv.owner;
    // Step 1.
    let Some(loc) = auto_equip_location(inv, w, t, item, skip) else {
        return false;
    };
    // Step 2.
    if matches!(loc, body::SWAP_RIGHT | body::SWAP_LEFT) || !w.link_check(unit, item, 3) {
        return false;
    }
    // A ground item is still in its room: the pickup's leave_room takes it
    // out after a successful equip. Mark it equipped for the placement so
    // grid placement does not remove it from the room itself, which would
    // record a ground removal and send a 0x0A ahead of the equip message.
    let ground = w.item(item).is_some_and(|d| d.mode == mode::GROUND);
    if ground {
        if let Some(d) = w.item_mut(item) {
            d.mode = mode::EQUIPPED;
        }
    }
    if !place_at_body(inv, w, item, loc) {
        if ground {
            if let Some(d) = w.item_mut(item) {
                d.mode = mode::GROUND;
            }
        }
        return false;
    }
    // Step 3.
    if let Some(d) = w.item_mut(item) {
        d.body_loc = loc;
    }
    super::weapon::weapon_link(inv, w, t, item);
    w.stat_link_in_use(unit, item, inv.weapon_guid);
    inv.put_cursor(w, None);
    w.stat_refresh(unit);
    w.clear_targetable(item);
    let mut guid = NO_GUID;
    if let Some(d) = w.item_mut(item) {
        d.mode = mode::EQUIPPED;
        d.cmd_flags |= cmd::EQUIP2;
        d.page = page::NONE;
        guid = d.guid;
    }
    inv.push_update(guid);
    w.owner_refresh(unit);
    w.weapon_bookkeeping(unit, item);
    true
}

/// Auto-equip on pickup: the body location `item`
/// goes to on the inventory owner's body, none = no.
pub fn auto_equip_location<W: InvWorld + ?Sized>(
    inv: &Inventory,
    w: &W,
    t: &InvTables,
    item: UnitId,
    skip: bool,
) -> Option<u8> {
    let unit = inv.owner;
    let d = *w.item(item)?;
    let rec = t.item(d.record)?;
    let ityp = t.itype_of(d.record)?;
    // Step 1.
    if !skip && !requirements_met(w, t, Some(item), unit, false) {
        return None;
    }
    if ityp.body == 0 || !w.has_allowed_location(item) {
        return None;
    }
    if d.flags & iflag::IDENTIFIED == 0 {
        return None;
    }
    // Not broken and no item flag 0x4000.
    if d.flags & (iflag::BROKEN | iflag::F4000) != 0 {
        return None;
    }
    // The primary type, not the equivalence test.
    if rec.type_ == ty::TPOT {
        return None;
    }
    // Step 2.
    if w.quiver_kind(item) {
        // The right hand (4) first, then the left (5): the hand weapon's
        // primary type `shoots`, equivalence test.
        let fed = [body::RIGHT_HAND, body::LEFT_HAND].iter().any(|&l| {
            inv.body_item(l)
                .and_then(|h| w.ammo_type(h))
                .is_some_and(|am| t.is_type(d.record, am))
        });
        if !fed {
            return None;
        }
    }
    // Step 3.
    let (l1, l2) = (ityp.bodyloc1, ityp.bodyloc2);
    if l1 == l2 {
        return inv.body_item(l1).is_none().then_some(l1);
    }
    // Step 4.
    match (inv.body_item(l1), inv.body_item(l2)) {
        (None, None) => Some(l1),
        (None, Some(e)) => auto_equip_compatible(w, t, unit, item, e).then_some(l1),
        (Some(e), None) => auto_equip_compatible(w, t, unit, item, e).then_some(l2),
        (Some(_), Some(_)) => None,
    }
}

/// Itemtypes rows of the auto-equip profile.
mod pty {
    pub const BOWQ: i16 = 5;
    pub const XBOQ: i16 = 6;
    pub const RING: i16 = 10;
    pub const BOW: i16 = 27;
    pub const XBOW: i16 = 35;
    pub const SHLD: i16 = 51;
}

/// The profile of an item X for a unit U: the flags
/// the compatibility test reads (`throw` is computed by the base game but
/// never read, so it is left out).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EquipProfile {
    pub bow: bool,
    pub xbow: bool,
    pub bowq: bool,
    pub xboq: bool,
    pub shield: bool,
    pub weapon: bool,
    pub two_handed: bool,
    pub dual: bool,
    pub ring: bool,
}

/// The profile of `x` for `unit` ("type T" is the
/// equivalence test).
pub fn equip_profile<W: InvWorld + ?Sized>(
    w: &W,
    t: &InvTables,
    unit: UnitId,
    x: UnitId,
) -> EquipProfile {
    let is = |ty: i16| is_type(w, t, x, ty);
    let dual = match w.unit_kind(unit) {
        Some(UnitKind::Player { class: BARBARIAN }) => true,
        Some(UnitKind::Player { class: ASSASSIN }) => is(ty::H2H),
        _ => false,
    };
    EquipProfile {
        bow: is(pty::BOW),
        xbow: is(pty::XBOW),
        bowq: is(pty::BOWQ),
        xboq: is(pty::XBOQ),
        shield: is(pty::SHLD),
        weapon: is(ty::WEAP),
        two_handed: w.two_handed(x),
        dual,
        ring: is(pty::RING),
    }
}

/// Compatibility of the new item `n` with the equipped item `e`.
pub fn auto_equip_compatible<W: InvWorld + ?Sized>(
    w: &W,
    t: &InvTables,
    unit: UnitId,
    n: UnitId,
    e: UnitId,
) -> bool {
    let (a, b) = (equip_profile(w, t, unit, n), equip_profile(w, t, unit, e));
    let one_hand = |p: &EquipProfile| p.weapon && !p.two_handed;
    (a.bow && b.bowq)
        || (a.bowq && b.bow)
        || (a.xbow && b.xboq)
        || (a.xboq && b.xbow)
        || (one_hand(&a) && b.shield)
        || (one_hand(&b) && a.shield)
        || (one_hand(&a) && one_hand(&b) && a.dual && b.dual)
        || (a.ring && b.ring)
}
