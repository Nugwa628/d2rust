//! The creation sequence after placement, the monster type init,
//! stats and skills, components, monprop,
//! monequip, normal and boss mods.

use d2_data::tables::{Monequip, Monprop};

use crate::rng::Seed;
use crate::units::UnitId;

use super::calc::{classic_scaling, hp_regen, monster_level, pct, stats_by_level};
use super::umods::{mark_unique, run_umod_init};
use super::{create_flag, mode, s16, stat, unit_flag, CreateRequest, Ctx, InitHost, HP_CAP};

/// The unit's seed. Init only runs on allocated units.
pub(super) fn seed<H: InitHost + ?Sized>(h: &mut H, unit: UnitId) -> &mut Seed {
    &mut h
        .units()
        .get_mut(unit)
        .expect("monster init on a unit without a record")
        .seed
}

fn class_of<H: InitHost + ?Sized>(h: &mut H, unit: UnitId) -> u32 {
    h.units().get(unit).map_or(0, |r| r.class)
}

/// The creation function after its request is built. Placement is the
/// host's; with flag 0x01
/// the call ends after placement and returns `Some(None)` (created, but no
/// unit). `None` = nothing placed or allocated.
pub fn create<H: InitHost + ?Sized>(
    cx: &Ctx<'_>,
    h: &mut H,
    req: &CreateRequest,
) -> Option<Option<UnitId>> {
    let (x, y) = h.place(req)?;
    if req.flags & create_flag::PROBE != 0 {
        return Some(None);
    }
    // Step 1: allocation runs the type init (`type_init`) inside.
    let unit = h.allocate(req, x, y)?;
    let m = cx.monstats(req.class);
    // Step 2: region count, coord list (population), alignment.
    let never_count = if m.is_some_and(|m| m.nevercount) {
        1
    } else {
        u32::from(req.flags & create_flag::NOT_COUNTED)
    };
    h.register_spawn(unit, req, never_count);
    // The spawn's not-counted flag, kept on the monster.
    h.monsters().entry(unit).not_counted = req.flags & create_flag::NOT_COUNTED != 0;
    let alignment = match m.map_or(0, |m| m.align) {
        1 => {
            if let Some(r) = h.units().get_mut(unit) {
                r.flags |= unit_flag::ALIGN1;
            }
            2
        }
        2 => 1,
        _ => 0,
    };
    h.set_alignment(unit, alignment);
    // Steps 3–4.
    if req.flags & create_flag::NO_NORMAL_MODS == 0 {
        normal_mods(cx, h, unit);
    }
    boss_mods(cx, h, unit);
    // Step 5.
    if req.flags & create_flag::NO_PARTY == 0 {
        h.party_minions(unit, req);
    }
    Some(Some(unit))
}

/// The monster type init, run by the allocator after
/// the unit record (GUID, mode, seed) and stat list exist.
pub fn type_init<H: InitHost + ?Sized>(cx: &Ctx<'_>, h: &mut H, unit: UnitId) {
    // Step 1 (GUID set by the allocator).
    if let Some(r) = h.units().get_mut(unit) {
        r.flags |= unit_flag::AT_INIT;
    }
    h.game().timers.cancel_unit_timers(unit);
    // Steps 2–3.
    h.alloc_ai(unit);
    let class = class_of(h, unit);
    h.monsters().entry(unit).class = class;
    h.game().coverage.hit("monsters_spawned", || class);
    // Step 4.
    let level_id = h.level_id(unit);
    stats_and_skills(cx, h, unit, level_id);
    // Step 5.
    h.ai_install(unit, 0);
    // Step 6.
    h.monsters().entry(unit).level_id = level_id;
    // Step 7.
    let m = h.units().get(unit).map_or(0, |r| r.mode);
    if m != mode::DEATH && m != mode::DEAD {
        h.attach_quest_chain(unit);
    } else {
        h.set_combat_mode(unit);
    }
    h.after_type_init(unit);
}

/// Hireling classes.
const HIRELINGS: [u32; 5] = [271, 338, 359, 560, 561];

/// Stats and skills. `level_id` is the room's level.
pub fn stats_and_skills<H: InitHost + ?Sized>(
    cx: &Ctx<'_>,
    h: &mut H,
    unit: UnitId,
    level_id: i32,
) {
    let class = class_of(h, unit);
    let Some(m) = cx.monstats(class) else {
        return;
    };
    // Steps 1–2.
    components(cx, h, unit);
    if class == 311 || class == 312 {
        let v = seed(h, unit).roll(4) as u8;
        let data = h.monsters().entry(unit);
        data.components[10] = v;
        if class == 312 {
            data.components[11] = v;
        }
    }
    // Step 3.
    let info = h.info();
    let bonus = h.rules().player_bonus(&info, m.align);
    if info.difficulty >= 3 {
        h.set_difficulty(2);
    }
    let info = h.info();
    let skill_bonus = cx
        .tables
        .difficultylevels
        .get(info.d())
        .map_or(0, |r| r.monsterskillbonus as i32);
    // Step 4.
    let d = if HIRELINGS.contains(&class) {
        0
    } else {
        info.d()
    };
    // Step 5.
    let level = monster_level(m, cx.tables.levels, &info, d, level_id);
    // Step 6. The stat list itself is the allocator's
    // (`units::lifecycle::allocate`, callback).
    // monster_playercount = the game's player count (at least 1).
    let col = |a: [u16; 3]| s16(a[d]);
    let col8 = |a: [u8; 3]| i32::from(a[d]);
    for (s, v) in [
        (stat::LEVEL, level),
        (stat::MONSTER_PLAYERCOUNT, bonus.players.max(1)),
        (stat::DAMAGERESIST, col([m.resdm, m.resdm_n, m.resdm_h])),
        (stat::MAGICRESIST, col([m.resma, m.resma_n, m.resma_h])),
        (stat::FIRERESIST, col([m.resfi, m.resfi_n, m.resfi_h])),
        (stat::LIGHTRESIST, col([m.resli, m.resli_n, m.resli_h])),
        (stat::COLDRESIST, col([m.resco, m.resco_n, m.resco_h])),
        (stat::POISONRESIST, col([m.respo, m.respo_n, m.respo_h])),
        (stat::TOBLOCK, col8([m.toblock, m.toblock_n, m.toblock_h])),
        (stat::ATTACKRATE, 100),
        (stat::VELOCITYPERCENT, 75),
        (stat::OTHER_ANIMRATE, 100),
        (stat::LAST_SENT_HP_PCT, 128),
    ] {
        h.set_stat(unit, s, v);
    }
    // Step 7.
    let base = stats_by_level(m, cx.tables.monlvl, info.l_flag(), d, level);
    // Step 8: minHP + roll(maxHP − minHP + 1), the plain roll helper.
    let rolled = base.min_hp.wrapping_add(
        seed(h, unit).roll(base.max_hp.wrapping_sub(base.min_hp).wrapping_add(1)) as i32,
    );
    let mut hp = rolled.wrapping_add(pct(rolled, bonus.hp, 100));
    if hp >= 0x80_0000 {
        hp = HP_CAP;
    }
    let maxhp = hp.wrapping_mul(256);
    h.set_stat(unit, stat::MAXHP, maxhp);
    h.set_stat(unit, stat::HITPOINTS, maxhp);
    // Step 9.
    let xp = base.xp.wrapping_add(pct(base.xp, bonus.xp, 100));
    h.set_stat(unit, stat::ARMORCLASS, base.ac);
    h.set_stat(unit, stat::EXPERIENCE, xp);
    // Step 10.
    let regen = hp_regen(maxhp, m.damageregen);
    assert!(
        regen.is_some(),
        "DamageRegen {} outside 0..0xFFF",
        m.damageregen
    );
    h.set_stat(unit, stat::HPREGEN, regen.unwrap_or(0));
    // Step 11.
    if let Some((mh, ac, x, lv)) = classic_scaling(&info, d, m, maxhp, base.ac, xp) {
        h.set_stat(unit, stat::MAXHP, mh);
        h.set_stat(unit, stat::ARMORCLASS, ac);
        h.set_stat(unit, stat::EXPERIENCE, x);
        h.set_stat(unit, stat::LEVEL, lv);
    }
    // Step 12.
    h.post_extra_list(unit);
    // Step 13.
    if m.inventory {
        h.new_inventory(unit, m.interact);
    }
    // Step 14 is empty.
    let skills = [
        (m.skill1, m.sk1lvl),
        (m.skill2, m.sk2lvl),
        (m.skill3, m.sk3lvl),
        (m.skill4, m.sk4lvl),
        (m.skill5, m.sk5lvl),
        (m.skill6, m.sk6lvl),
        (m.skill7, m.sk7lvl),
        (m.skill8, m.sk8lvl),
    ];
    let modes = cx
        .tables
        .monstats_extra
        .get(class as usize)
        .map_or([-1; 8], |e| e.skill_modes);
    for ((skill, lvl), md) in skills.into_iter().zip(modes) {
        if s16(skill) >= 0 && lvl > 0 {
            let md = u8::try_from(md).ok();
            h.give_skill(unit, skill, i32::from(lvl) + skill_bonus, md);
        }
    }
    // Step 15.
    let mut f = 0;
    if cx.monstats2(class).is_some_and(|m2| m2.isatt) {
        f |= unit_flag::IS_ATT;
    }
    if m.petignore {
        f |= unit_flag::PET_IGNORE;
    }
    if let Some(r) = h.units().get_mut(unit) {
        r.flags |= f;
    }
    // Steps 16–17.
    monprop(cx, h, unit, d);
    let level = h.stat(unit, stat::LEVEL);
    monequip(cx, h, unit, level);
}

/// Components.
pub fn components<H: InitHost + ?Sized>(cx: &Ctx<'_>, h: &mut H, unit: UnitId) {
    let class = class_of(h, unit);
    let n = h.region_variant_count(unit, class);
    let comps = if n > 0 {
        let i = seed(h, unit).roll(i32::from(n));
        h.region_variant(unit, class, i)
    } else {
        let counts = cx
            .monstats(class)
            .and_then(|m| cx.tables.components.get(usize::from(m.monstatsex)))
            .copied()
            .unwrap_or([0; 16]);
        let s = seed(h, unit);
        counts.map(|c| s.roll(i32::from(c)) as u8)
    };
    h.monsters().entry(unit).components = comps;
}

/// The six (prop, par, chance, min, max) of a monprop row for d.
fn monprop_slots(r: &Monprop, d: usize) -> [(u32, u32, u8, u32, u32); 6] {
    match d {
        0 => [
            (r.prop1, r.par1, r.chance1, r.min1, r.max1),
            (r.prop2, r.par2, r.chance2, r.min2, r.max2),
            (r.prop3, r.par3, r.chance3, r.min3, r.max3),
            (r.prop4, r.par4, r.chance4, r.min4, r.max4),
            (r.prop5, r.par5, r.chance5, r.min5, r.max5),
            (r.prop6, r.par6, r.chance6, r.min6, r.max6),
        ],
        1 => [
            (r.prop1_n, r.par1_n, r.chance1_n, r.min1_n, r.max1_n),
            (r.prop2_n, r.par2_n, r.chance2_n, r.min2_n, r.max2_n),
            (r.prop3_n, r.par3_n, r.chance3_n, r.min3_n, r.max3_n),
            (r.prop4_n, r.par4_n, r.chance4_n, r.min4_n, r.max4_n),
            (r.prop5_n, r.par5_n, r.chance5_n, r.min5_n, r.max5_n),
            (r.prop6_n, r.par6_n, r.chance6_n, r.min6_n, r.max6_n),
        ],
        _ => [
            (r.prop1_h, r.par1_h, r.chance1_h, r.min1_h, r.max1_h),
            (r.prop2_h, r.par2_h, r.chance2_h, r.min2_h, r.max2_h),
            (r.prop3_h, r.par3_h, r.chance3_h, r.min3_h, r.max3_h),
            (r.prop4_h, r.par4_h, r.chance4_h, r.min4_h, r.max4_h),
            (r.prop5_h, r.par5_h, r.chance5_h, r.min5_h, r.max5_h),
            (r.prop6_h, r.par6_h, r.chance6_h, r.min6_h, r.max6_h),
        ],
    }
}

/// monprop.
pub fn monprop<H: InitHost + ?Sized>(cx: &Ctx<'_>, h: &mut H, unit: UnitId, d: usize) {
    let class = class_of(h, unit);
    let Some(m) = cx.monstats(class) else {
        return;
    };
    let Some(row) = usize::try_from(s16(m.monprop))
        .ok()
        .and_then(|i| cx.tables.monprop.get(i))
    else {
        return;
    };
    for (prop, par, chance, min, max) in monprop_slots(row, d) {
        if (prop as i32) < 0 {
            break;
        }
        let apply = chance == 0 || seed(h, unit).step() % 100 < u32::from(chance);
        if apply {
            h.apply_property(unit, prop as i32, par as i32, min as i32, max as i32);
        }
    }
}

/// monequip.
pub fn monequip<H: InitHost + ?Sized>(cx: &Ctx<'_>, h: &mut H, unit: UnitId, level: i32) {
    // Only for units with an inventory.
    if !h.has_inventory(unit) {
        return;
    }
    monequip_rows(cx.tables.monequip, h, unit, None, level, level, true);
}

/// monequip (also used for summons): rows of the
/// unit's class are skipped while their `level` > `cutoff`; items are
/// made at item level `ilvl`; `check_oninit` is the last argument (1:
/// the first row's `oninit` 0 ends it, 0: no test). An item code of four
/// spaces copies the base code of the `owner`'s item at that location
/// ([`InitHost::owner_item_code`]).
pub fn monequip_rows<H: InitHost + ?Sized>(
    rows: &[Monequip],
    h: &mut H,
    unit: UnitId,
    owner: Option<UnitId>,
    cutoff: i32,
    ilvl: i32,
    check_oninit: bool,
) {
    let class = class_of(h, unit);
    let Some(first) = rows.iter().position(|r| u32::from(r.monster) == class) else {
        return;
    };
    // Step 1.
    if check_oninit && rows[first].oninit == 0 {
        return;
    }
    let same = |i: usize| rows.get(i).is_some_and(|r| u32::from(r.monster) == class);
    // Step 2.
    let mut i = first;
    while same(i) && i32::from(rows[i].level) > cutoff {
        i += 1;
    }
    // Step 3.
    while same(i) {
        let r = &rows[i];
        let slots = [
            (r.item1, r.loc1, r.mod1),
            (r.item2, r.loc2, r.mod2),
            (r.item3, r.loc3, r.mod3),
        ];
        let count = slots.iter().take_while(|s| (1..=10).contains(&s.1)).count();
        if count > 0 {
            let k = seed(h, unit).roll(count as i32) as usize;
            let (mut item, loc, md) = slots[k];
            if !h.has_item_at(unit, loc) {
                let md = if md <= 7 { md } else { 0 };
                if item == *b"    " {
                    // The owner's item code at `loc`; no owner item
                    // there, no item.
                    match owner.and_then(|o| h.owner_item_code(o, loc)) {
                        Some(c) => item = c,
                        None => {
                            i += 1;
                            continue;
                        }
                    }
                }
                h.create_equip_item(unit, item, loc, md, ilvl);
            }
        }
        i += 1;
    }
}

/// Class reinit: the monster
/// becomes `class` in place. False (nothing changed) for a non-monster
/// or a class outside monstats or without `enabled`.
pub fn reinit<H: InitHost + ?Sized>(
    cx: &Ctx<'_>,
    h: &mut H,
    unit: UnitId,
    class: i32,
    mode: u32,
) -> bool {
    // Step 1.
    let monster = h
        .units()
        .get(unit)
        .is_some_and(|r| r.ty == crate::units::UnitType::Monster);
    if !monster {
        return false;
    }
    // Step 2.
    let Some(row) = u32::try_from(class).ok().and_then(|c| cx.monstats(c)) else {
        return false;
    };
    if !row.enabled {
        return false;
    }
    // Step 4 with the old class: an `interact` monster keeps its
    // inventory.
    let old = class_of(h, unit);
    let keep = cx.monstats(old).is_some_and(|m| m.interact);
    h.monster_teardown(unit, !keep);
    h.game().timers.cancel_unit_timers(unit);
    // Step 5.
    if let Some(r) = h.units().get_mut(unit) {
        r.class = class as u32;
    }
    // Step 6: the type init on the unit's room and GUID; mode
    // setting reads the old mode.
    type_init(cx, h, unit);
    // Step 7.
    h.set_mode_plain(unit, mode);
    true
}

/// Normal mods by `BaseId`: (umod, unique arg).
pub fn normal_mods_for(base_id: u16) -> &'static [(u8, bool)] {
    match base_id {
        24 => &[(13, false)],
        91 => &[(20, false)],
        96 => &[(10, false)],
        211 => &[(11, false), (22, true)],
        326..=330 | 354 => &[(14, false)],
        340..=343 => &[(15, false)],
        436 => &[(34, false)],
        461 => &[(33, false)],
        501 => &[(35, false)],
        540..=542 => &[(22, true)],
        _ => &[],
    }
}

/// Normal mods.
pub fn normal_mods<H: InitHost + ?Sized>(cx: &Ctx<'_>, h: &mut H, unit: UnitId) {
    let class = class_of(h, unit);
    let Some(m) = cx.monstats(class) else {
        return;
    };
    for &(umod, unique) in normal_mods_for(m.baseid) {
        assign_umod(cx, h, unit, umod, unique);
    }
}

/// One step of a boss-mods case, in code order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BossStep {
    Umod(u8, bool),
    /// Quest chain record.
    Chain(u32),
    /// Unit flags |= mask (skipped for a null unit).
    UnitFlags(u32),
    /// Sets the monster data flag bit 0.
    DataFlag1,
    /// State 118 corpse_noselect on.
    CorpseNoselect,
    /// The ancient barbarian equipment.
    AncientEquip,
}

/// The boss-mods steps of for a class with
/// `BaseId` `base_id`, in code order. The warriv2 (175) case's class
/// hook acts only for classes 201 and 331, so it has no step.
pub fn boss_mods_for(base_id: i32, class: u32) -> &'static [BossStep] {
    use BossStep::*;
    match base_id {
        156 if class == 707 => &[Umod(23, true), Umod(6, true), Umod(29, true)],
        156 => &[Umod(22, true), Chain(6)],
        211 if class == 708 => &[Umod(6, true), Umod(18, true)],
        211 => &[Chain(13), Chain(9)],
        229 => &[Chain(8)],
        242 if class == 704 => &[
            Umod(22, true),
            Umod(30, true),
            Umod(17, true),
            Umod(8, true),
            Umod(6, true),
        ],
        242 => &[Chain(20), Umod(22, true)],
        243 if class == 705 => &[Umod(22, true), Umod(8, true), Umod(6, true)],
        243 => &[Umod(22, true), Chain(23)],
        250 => &[Chain(12), UnitFlags(0x800), DataFlag1],
        256 if class == 706 => &[Umod(6, true), Umod(18, true)],
        256 => &[Umod(22, true), Chain(22)],
        267 => &[Umod(12, true), Umod(22, true), Chain(2), CorpseNoselect],
        284 => &[Umod(23, true), Umod(22, true)],
        292 => &[Umod(31, false)],
        340..=343 => &[UnitFlags(0x20000)],
        366 => &[Chain(19), UnitFlags(0x20000), Umod(22, true)],
        402 => &[Umod(22, true)],
        407 => &[Chain(17)],
        409 => &[Chain(24)],
        434 => &[Chain(32)],
        526 => &[Chain(34), Umod(22, true)],
        540 => &[AncientEquip],
        544 if class == 709 => &[Umod(22, true), Umod(18, true), Umod(8, true), Umod(6, true)],
        544 => &[Chain(36), Umod(22, true)],
        _ => &[],
    }
}

/// The quest chains the boss mods link for `class`
/// (its `Chain` steps, in code order), keyed on the class's `BaseId` as
/// [`boss_mods`] is. For units placed without the creation function.
pub fn boss_chains(
    monstats: &[d2_data::tables::Monstats],
    class: u32,
) -> impl Iterator<Item = u32> {
    let base = monstats
        .get(class as usize)
        .map_or(class as i32, |m| i32::from(m.baseid));
    boss_mods_for(base, class).iter().filter_map(|&s| match s {
        BossStep::Chain(n) => Some(n),
        _ => None,
    })
}

/// Boss mods: the switch key is the class's
/// `BaseId` (the class itself when its monstats row is missing).
pub fn boss_mods<H: InitHost + ?Sized>(cx: &Ctx<'_>, h: &mut H, unit: UnitId) {
    let class = class_of(h, unit);
    let base = cx
        .monstats(class)
        .map_or(class as i32, |m| i32::from(m.baseid));
    for &step in boss_mods_for(base, class) {
        match step {
            BossStep::Umod(n, unique) => assign_umod(cx, h, unit, n, unique),
            BossStep::Chain(n) => h.quest_chain(unit, n),
            BossStep::UnitFlags(mask) => {
                if let Some(r) = h.units().get_mut(unit) {
                    r.flags |= mask;
                }
            }
            BossStep::DataFlag1 => h.monsters().entry(unit).data_flag1 = true,
            BossStep::CorpseNoselect => h.set_corpse_noselect(unit),
            BossStep::AncientEquip => ancient_equip(h, unit, class),
        }
    }
}

/// Table: (item code, body location) for k = 0..3 of the
/// ancient barbarians 540–542.
const ANCIENT_EQUIP: [[(&[u8; 4], u8); 4]; 3] = [
    [(b"bsd ", 4), (b"tow ", 5), (b"fld ", 3), (b"hbt ", 9)],
    [(b"tax ", 4), (b"tax ", 5), (b"hgl ", 10), (b"hbt ", 9)],
    [(b"vou ", 4), (b"rin ", 6), (b"fld ", 3), (b"crn ", 1)],
];

/// The ancient barbarian equipment: per entry the
/// code, upgraded to `ubercode` / `ultracode` on difficulty 1 / 2, then
/// the monster equip helper at ilvl = the unit's level.
/// The table is keyed by the class, so only 540–542 read a row.
fn ancient_equip<H: InitHost + ?Sized>(h: &mut H, unit: UnitId, class: u32) {
    let Some(row) = class
        .checked_sub(540)
        .and_then(|i| ANCIENT_EQUIP.get(i as usize))
    else {
        return;
    };
    let level = h.stat(unit, stat::LEVEL);
    let difficulty = h.info().difficulty;
    for &(code, loc) in row {
        let code = h.item_tier_code(*code, difficulty);
        h.create_boss_item(unit, code, loc, level);
    }
}

/// Append the umod and run its init; with `unique` also
/// mark the monster unique.
pub fn assign_umod<H: InitHost + ?Sized>(
    cx: &Ctx<'_>,
    h: &mut H,
    unit: UnitId,
    umod: u8,
    unique: bool,
) {
    h.monsters().entry(unit).push_umod(umod);
    run_umod_init(cx, h, unit, umod, unique);
    if unique {
        mark_unique(h, unit);
    }
}
