//! The item type test of `use_state` test 5: a skill's
//! item-type sets against the items in the two hands. One function for
//! every caller (the skill use pipeline's view and the inventory desk);
//! each answers the item questions through [`ItemTypeView`].

/// One item-type set of a skill row (i16 columns, ≤ 0 = none).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HandSet {
    pub itypes: [i32; 3],
    pub etypes: [i32; 2],
}

/// The skill row parts the test reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ItemTypeSkill {
    /// The skill id (4 Left Hand Throw and 5 Left Hand Swing leave the
    /// weapon in use out; 0 Attack has the matched-item exceptions).
    pub skill: i32,
    pub a: HandSet,
    pub b: HandSet,
    /// skills.txt `noammo` (flags bit 16).
    pub noammo: bool,
}

impl ItemTypeSkill {
    /// The sets and flag of a skills row.
    pub fn of(skill: i32, r: &d2_data::tables::Skills) -> Self {
        // i16 columns, ≤ 0 = none.
        let v = |x: u16| i32::from(x as i16);
        ItemTypeSkill {
            skill,
            a: HandSet {
                itypes: [v(r.itypea1), v(r.itypea2), v(r.itypea3)],
                etypes: [v(r.etypea1), v(r.etypea2)],
            },
            b: HandSet {
                itypes: [v(r.itypeb1), v(r.itypeb2), v(r.itypeb3)],
                etypes: [v(r.etypeb1), v(r.etypeb2)],
            },
            noammo: r.noammo,
        }
    }
}

/// What the test asks about the unit's hands.
pub trait ItemTypeView {
    type Item: Copy + PartialEq;
    /// The item at body location 4 or 5; none without an
    /// inventory.
    fn hand_item(&self, loc: u8) -> Option<Self::Item>;
    /// The unit has an inventory (unit +0x60).
    fn has_inventory(&self) -> bool;
    /// The weapon in use.
    fn weapon_in_use(&self) -> Option<Self::Item>;
    /// The item is of the itemtypes row `itype`, equivalence included.
    fn is_a(&self, item: Self::Item, itype: i32) -> bool;
    /// Item flag 0x4000 or 0x100 (item data +0x18).
    fn ammo_free(&self, item: Self::Item) -> bool;
    /// The `shoots` type of the item's type; 0: none.
    fn shoots(&self, item: Self::Item) -> i32;
    /// The item's total stat `stat` (157 `item_magicarrow`, 70
    /// `quantity`).
    fn stat(&self, item: Self::Item, stat: u16) -> i32;
    /// The item stacks (> 0).
    fn stackable(&self, item: Self::Item) -> bool;
}

const WEAP: i32 = 45;
const MELE: i32 = 46;
const H2H: i32 = 67;
const STAT_MAGICARROW: u16 = 157;
const STAT_QUANTITY: u16 = 70;

fn listed(t: &[i32]) -> impl Iterator<Item = i32> + '_ {
    t.iter().copied().take_while(|&t| t > 0)
}

/// `hand(s, X, Y)`.
fn hand<V: ItemTypeView>(
    v: &V,
    k: &ItemTypeSkill,
    s: &HandSet,
    x: Option<V::Item>,
    y: Option<V::Item>,
) -> bool {
    let Some(x) = x else {
        if s.etypes[0] <= 0 && s.itypes[0] <= 0 {
            return true;
        }
        // An empty hand meets a weapon-type set when the other hand holds
        // no weapon (bare-handed use); `itypea1` is always set a's.
        return [WEAP, MELE, H2H].contains(&k.a.itypes[0])
            && k.b.itypes[0] <= 0
            && !y.is_some_and(|y| v.is_a(y, WEAP));
    };
    if listed(&s.etypes).any(|t| v.is_a(x, t)) {
        return false;
    }
    // No type listed passes at once: the matched item's rules below do
    // not run.
    let mut want = listed(&s.itypes).peekable();
    if want.peek().is_none() {
        return true;
    }
    if !want.any(|t| v.is_a(x, t)) {
        return false;
    }
    // X matched.
    if v.ammo_free(x) {
        return k.skill == 0;
    }
    let ammo = v.shoots(x);
    if ammo != 0 && !k.noammo {
        if k.skill == 0 && v.stat(x, STAT_MAGICARROW) != 0 {
            return true;
        }
        let Some(y) = y.filter(|&y| v.is_a(y, ammo)) else {
            return false;
        };
        if k.skill == 0 && v.stat(y, STAT_MAGICARROW) != 0 {
            return true;
        }
        if v.stackable(y) && v.stat(y, STAT_QUANTITY) < 1 {
            return false;
        }
    }
    true
}

/// Whether the hands meet the skill's sets (rules 1–4 of
/// "Item type test"; fail → use state 2).
pub fn item_type_test<V: ItemTypeView>(v: &V, k: &ItemTypeSkill) -> bool {
    // Step 1.
    if k.a.etypes[0] <= 0 && k.a.itypes[0] <= 0 {
        return true;
    }
    // Step 2.
    if !v.has_inventory() {
        return false;
    }
    let (mut ha, mut hb) = (v.hand_item(4), v.hand_item(5));
    // Step 3: Left Hand Throw / Swing leave the weapon in use out.
    if k.skill == 4 || k.skill == 5 {
        let w = v.weapon_in_use();
        if ha.is_some() && ha == w {
            ha = None;
        } else if hb.is_some() && hb == w {
            hb = None;
        }
    }
    // Step 4: the swapped order only when A fails set a.
    if hand(v, k, &k.a, ha, hb) {
        hand(v, k, &k.b, hb, ha)
    } else {
        hand(v, k, &k.b, ha, hb) && hand(v, k, &k.a, hb, ha)
    }
}

#[cfg(test)]
mod tests;
