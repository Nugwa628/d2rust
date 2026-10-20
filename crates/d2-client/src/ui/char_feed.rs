//! The character panel's remaining inputs from the client model: the
//! class line (`charstats` class name), the resist / defense effect
//! colours (`states.txt` flags), next-level experience and the damage /
//! attack-rating block of the left and right skill (`char_details`).
//! Nothing here decides an outcome; it reads the model and draws.
//!
//! The damage value of the weapon-physical `descdam` entries (1, 7, 18, 19, 20) is
//! `weapon_phys`, plus the stat elements, then the `SrcDam` scale; the attack
//! rating of `descatt` 1 / 2 is `AR` with `attack_rating`.
//!
//! With the skills tables loaded the damage and attack-rating values come from
//! `char_desc` (the skill-side terms `ddam calc1` / `calc2`, `phys_min` /
//! `elem_min`, `to_hit(U, id, L)`, weapon mastery, the grip-2 secondary
//! damage), and so do the `finishing` charges, the item stat lists and the
//! dual-hand ranges. PROVISIONAL: `descatt` 2 on a dual-wielder (only the weapon
//! in use's list detached) draws one line of width 283; whether it should wrap
//! to two lines (width 436) is open. Other `descdam` entries draw the skill name
//! only.

use std::collections::{BTreeMap, BTreeSet};

use super::draw::UiDrawSink;
use super::layout::Screen;
use super::panel::StringLookup;
use super::panels::char_details::{self, BlockItem, HandBlock};
use super::panels::char_inputs::{self, ValueState};
use super::panels::character::ResistEffect;
use super::panels::{centered_in, text, utf16, TextMeasure};
use super::skill_desc::{ar_value, range_text};
use super::skill_desc_more::{pct, range_font};
use crate::bridge::world::{ClientWorld, UnitKey};

/// The skilldesc fields the damage block reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DescRow {
    /// `str alt` (+0x0E): the name drawn, e.g. "EXPLODING" = `skillan16`.
    pub name_id: u16,
    /// `descdam` (+0x12).
    pub descdam: u16,
    /// `descatt` (+0x14).
    pub descatt: u16,
    /// skills `SrcDam` (+0x1A5).
    pub src_dam: u8,
}

/// The weapon fields of the damage block (`weapons.txt`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WeaponRow {
    /// `StrBonus` / `DexBonus`.
    pub str_bonus: i32,
    pub dex_bonus: i32,
}

/// The tables of the character panel's extra inputs (empty without game
/// files: nothing extra is drawn).
#[derive(Clone, Debug, Default)]
pub struct CharTables {
    /// `charstats` `class` key per class (the class line's string key).
    pub class_keys: Vec<String>,
    /// Per `states` row: bit `i` = the flag of [`ValueState::ALL`]`[i]`.
    pub state_flags: Vec<u16>,
    /// Skill id → its skilldesc fields.
    pub skill_desc: BTreeMap<u16, DescRow>,
    /// `charstats` `ToHitFactor` per class (`attack_rating`).
    pub tohit_factor: Vec<i32>,
    /// `weapons.txt` rows by code.
    pub weapons: BTreeMap<[u8; 4], WeaponRow>,
    /// The `descdam` / `descatt` functions' tables (`char_desc`); without
    /// the skills tables (no install: unit tests only, a real install
    /// always loads them) the weapon-only sum below is drawn.
    pub desc: super::char_desc::DescTables,
}

/// `strchrskm`: "Damage".
const STR_DAMAGE: u16 = 4061;
/// `strchratr`: "%s\nAttack Rating".
const STR_ATTACK: u16 = 4063;
/// `strchratr` without a skills record: "%s\nRating".
const STR_ATTACK_NO_RECORD: u16 = 4065;
/// The skill name without a skilldesc row.
const STR_NO_NAME: u16 = 5382;
/// `holyshield`.
const STATE_HOLYSHIELD: u8 = 101;

/// Stats read here.
const STRENGTH: u16 = 0;
const DEX: u16 = 2;
const MAXDAMAGE_PERCENT: u16 = 17;
const MINDAMAGE_PERCENT: u16 = 18;
const TOHIT: u16 = 19;
const MINDAMAGE: u16 = 21;
const MAXDAMAGE: u16 = 22;
const DAMAGEPERCENT: u16 = 25;
/// Stat elements: (min, max, mastery or 0).
const ELEMENTS: [(u16, u16, u16); 4] = [(48, 49, 329), (50, 51, 330), (54, 55, 331), (52, 53, 0)];

/// The damage of a weapon-physical `descdam` entry (1 / 7):
/// `weapon_phys(U, …, F0 = 0, S, 128, 0, 0)` with E = 0, stat
/// elements, then × `SrcDam` / 128 when ≠ 128.
pub fn weapon_damage(
    stat: &dyn Fn(u16) -> i32,
    weapon: Option<WeaponRow>,
    src_dam: i32,
) -> (i32, i32) {
    use super::skill_desc_more::{
        clamp_elements, element_pair, poison_pair, weapon_phys_base, weapon_phys_final,
    };
    // Bare hand mindamage + 1, maxdamage + 2.
    let (b, big_b) = match weapon {
        None => (stat(MINDAMAGE) + 1, stat(MAXDAMAGE) + 2),
        Some(_) => (stat(MINDAMAGE), stat(MAXDAMAGE)),
    };
    let (mn, mx) = weapon_phys_base(b, big_b, 128, src_dam);
    // Rule 4.
    let mut p = stat(DAMAGEPERCENT);
    match weapon {
        None => p += stat(STRENGTH),
        Some(w) => {
            if w.str_bonus != 0 {
                p += stat(STRENGTH) * w.str_bonus / 100;
            }
            if w.dex_bonus != 0 {
                p += stat(DEX) * w.dex_bonus / 100;
            }
        }
    }
    let (mut mn, mut mx) = weapon_phys_final(
        mn,
        mx,
        p,
        stat(MINDAMAGE_PERCENT),
        stat(MAXDAMAGE_PERCENT),
        0,
        0,
    );
    // The elements add to the weapon sum; rule 3 clamps the whole.
    for (lo, hi, m) in ELEMENTS {
        let (a, b) = element_pair(stat(lo), stat(hi), if m == 0 { 0 } else { stat(m) });
        mn += a;
        mx += b;
    }
    let (a, b) = poison_pair(
        stat(57),
        stat(58),
        stat(332),
        stat(101),
        stat(59),
        stat(326),
    );
    (mn, mx) = clamp_elements(mn + a, mx + b);
    if src_dam != 128 {
        mn = pct(mn, src_dam, 128);
        mx = pct(mx, src_dam, 128);
    }
    (mn, mx)
}

/// Weapon-physical `descdam` entries this build computes.
fn descdam_known(n: u16) -> bool {
    matches!(n, 1 | 7 | 18 | 19 | 20)
}

impl CharTables {
    fn has(&self, states: &BTreeSet<u8>, v: ValueState) -> bool {
        let i = ValueState::ALL.iter().position(|&x| x == v).unwrap_or(0);
        states.iter().any(|&s| {
            self.state_flags
                .get(usize::from(s))
                .is_some_and(|f| f >> i & 1 != 0)
        })
    }

    /// The resist effect of resist stat `stat` (39, 43, 41, 45): a red
    /// state wins over a blue one (`character.rs` `ResistEffect`).
    pub fn resist_effect(&self, states: &BTreeSet<u8>, stat: u16) -> ResistEffect {
        let (blue, red) = match stat {
            39 => (ValueState::RfBlue, ValueState::RfRed),
            43 => (ValueState::RcBlue, ValueState::RcRed),
            41 => (ValueState::RlBlue, ValueState::RlRed),
            45 => (ValueState::RpBlue, ValueState::RpRed),
            _ => return ResistEffect::None,
        };
        if self.has(states, red) {
            ResistEffect::Lowered
        } else if self.has(states, blue) {
            ResistEffect::Raised
        } else {
            ResistEffect::None
        }
    }

    /// The defense colour; `None` without the
    /// `states` flags (the caller keeps the plain compare).
    pub fn defense_color(&self, states: &BTreeSet<u8>) -> Option<u16> {
        if self.state_flags.is_empty() {
            return None;
        }
        // D2RUST's own, unverified: the shield test (item type 51) is not
        // read; `holyshield` alone stands for it.
        Some(u16::from(char_inputs::defense_color(
            self.has(states, ValueState::ArmBlue),
            states.contains(&STATE_HOLYSHIELD),
            self.has(states, ValueState::ArmRed),
        )))
    }
}

/// The class line: the class name in Font16,
/// colour 0, centred in [`sx + 193`, `sx + 310`].
pub fn class_line(
    class: u32,
    tables: &CharTables,
    strings: &dyn StringLookup,
    s: &Screen,
    measure: &dyn TextMeasure,
    out: &mut dyn UiDrawSink,
) {
    let Some(name) = usize::try_from(class)
        .ok()
        .and_then(|c| tables.class_keys.get(c))
        .and_then(|k| strings.get(k))
    else {
        return;
    };
    let Some(w) = measure.width(1, name) else {
        return;
    };
    let (a, b) = char_details::class_span(s);
    out.push(text(
        name.to_vec(),
        centered_in(a, b, w),
        char_details::line_y(s),
        1,
        0,
    ));
}

/// The next-level experience: `experience` rows
/// (row 0 `MaxLvl`, row L + 1 level L) for the player's base level.
pub fn next_level(experience: &[[u32; 7]], class: u32, level: u32, stat30: u32) -> Option<u32> {
    let max = |c: u32| experience.first().map_or(0, |r| r[c as usize]);
    experience.first()?;
    Some(char_details::next_level_value(
        level,
        class,
        &max,
        &|c, l| {
            experience
                .get(l as usize + 1)
                .map_or(stat30, |r| r[c as usize])
        },
        stat30,
    ))
}

fn lossy(s: &[u16]) -> String {
    String::from_utf16_lossy(s)
}

/// The block of one hand from the unit's stats.
pub(super) fn hand(
    skill: u16,
    row: &DescRow,
    strings: &dyn StringLookup,
    stat: &dyn Fn(u16) -> i32,
    tohit_factor: i32,
) -> HandBlock {
    let name = strings
        .get_id(row.name_id)
        .or_else(|| strings.get_id(STR_NO_NAME))
        .map(lossy)
        .unwrap_or_default();
    let mut h = HandBlock {
        name: name.clone(),
        damage_label: strings.get_id(STR_DAMAGE).map(lossy).unwrap_or_default(),
        ..HandBlock::default()
    };
    h.damage = descdam_known(row.descdam);
    if matches!(row.descatt, 1 | 2) {
        h.attack = true;
        // `attack_rating(U)`, then P =
        // `item_tohit_percent(119)`; `char_desc`
        // replaces it when the skills tables are loaded.
        let ar = stat(TOHIT) + 5 * (stat(DEX) - 7) + tohit_factor;
        h.v1 = ar_value(ar, stat(119));
    }
    if super::skill_desc::descatt_live(row.descatt) {
        // The label is formatted with the upper-cased name
        // (the Attack skill on the right hand draws "ATTACK" / "Rating"). The
        // skills record is null for
        // the Attack skill (id 0, never in the player's skill list), so 4065 is
        // used for it, as is label 4063 for the other descatt skills.
        let id = if skill == 0 {
            STR_ATTACK_NO_RECORD
        } else {
            STR_ATTACK
        };
        h.attack_label = strings
            .get_id(id)
            .map(|s| lossy(s).replace("%s", &super::panels::char_details::upper_name(&name)))
            .unwrap_or_default();
    }
    h
}

/// The weapon in use as the client model knows it: the
/// `weapons.txt` row of the item at body location 4 (right arm), else 5.
/// D2RUST's own, unverified: the weapon-switch slot and a shield in location 4
/// are not told apart.
fn weapon_in_use(world: &ClientWorld, key: UnitKey, tables: &CharTables) -> Option<WeaponRow> {
    use crate::bridge::items::{items, mode};
    let mine: Vec<_> = items(world)
        .into_iter()
        .filter(|i| i.mode == mode::BODY && i.owner == Some(key))
        .collect();
    [4u8, 5].iter().find_map(|&loc| {
        mine.iter()
            .find(|i| i.body == loc)
            .and_then(|i| i.code)
            .and_then(|c| tables.weapons.get(&c).copied())
    })
}

/// Draws the damage block for the left then the
/// right skill of the local player.
#[allow(clippy::too_many_arguments)]
pub fn damage_block(
    world: &ClientWorld,
    key: UnitKey,
    tables: &CharTables,
    strings: &dyn StringLookup,
    s: &Screen,
    measure: &dyn TextMeasure,
    out: &mut dyn UiDrawSink,
) {
    let Some(list) = world.units.get(&key).and_then(|u| u.skills.as_ref()) else {
        return;
    };
    let stat = |id: u16| world.total(key, id, 0);
    let class = world.units.get(&key).map_or(0, |u| u.class);
    let tohit_factor = tables
        .tohit_factor
        .get(class as usize)
        .copied()
        .unwrap_or(0);
    let weapon = weapon_in_use(world, key, tables);
    let mut once = false;
    let desc = tables.desc.skills.clone();
    let mut units = super::char_desc::DescUnits::new(world, key, &tables.desc);
    for (p, entry) in [(0usize, list.left_entry()), (6, list.right_entry())] {
        let Some(e) = entry else { continue };
        let Some(row) = tables.skill_desc.get(&e.skill) else {
            continue;
        };
        let mut h = hand(e.skill, row, strings, &stat, tohit_factor);
        let mut draw = None;
        if let Some(t) = desc.as_deref() {
            use super::char_desc::{descatt, descdam, hand_state_draws, Hand};
            let sim = d2_sim::skills::SkillEntry {
                skill: i32::from(e.skill),
                base: e.base,
                level_bonus: e.level_bonus,
                owner_guid: e.owner as i32,
                charges: e.charges,
                has_charges: e.has_charges,
            };
            let lvl = d2_sim::skills::levels::skill_level(&units, t, Some(key), Some(&sim), true);
            let mut hd = Hand {
                u: &mut units,
                t,
                code: &tables.desc.desc_code,
                key,
                skill: i32::from(e.skill),
                lvl,
            };
            let ok = hand_state_draws(&mut hd);
            h.damage = ok && super::skill_desc::descdam_live(row.descdam);
            h.attack = ok && super::skill_desc::descatt_live(row.descatt);
            if h.damage {
                draw = Some(descdam(&mut hd, row.descdam));
            }
            if h.attack {
                let (v1, c1, v2, c2) = descatt(&mut hd, row.descatt, tohit_factor);
                (h.v1, h.c1, h.v2, h.c2) = (v1, c1 as u8, v2, c2 as u8);
            } else {
                (h.v1, h.v2) = (0, 0);
            }
        }
        for item in char_details::hand_block(&h, p, s, false, &mut once) {
            match item {
                BlockItem::Text(t) => {
                    // The label lines: Font6 as `hand_block` names them.
                    let s16 = utf16(&t.text);
                    if let Some(w) = measure.width(t.font, &s16) {
                        out.push(text(
                            s16,
                            centered_in(t.x1, t.x2, w),
                            t.y,
                            t.font,
                            u16::from(t.color),
                        ));
                    }
                }
                BlockItem::DamageValue { x1, x2, y } => {
                    let lines = match &draw {
                        Some(d) => d.lines(super::skill_desc_more::two_line_top_offset(0)),
                        None => {
                            let (mn, mx) = weapon_damage(&stat, weapon, i32::from(row.src_dam));
                            vec![(range_text(mn, mx), 0, 0, false)]
                        }
                    };
                    for (txt, color, dy, two) in lines {
                        let s16 = utf16(&txt);
                        if two {
                            if let Some(w) = measure.width(6, &s16) {
                                out.push(text(
                                    s16,
                                    centered_in(x1, x2, w),
                                    y + dy,
                                    6,
                                    color as u16,
                                ));
                            }
                            continue;
                        }
                        // Measured in the caller's font (Font6).
                        if let Some(w) = measure.width(6, &s16) {
                            let (small, y) = range_font(w, x1, x2, y);
                            let font = if small { 6 } else { 1 };
                            if let Some(w) = measure.width(font, &s16) {
                                out.push(text(s16, centered_in(x1, x2, w), y, font, color as u16));
                            }
                        }
                    }
                }
            }
        }
    }
}
