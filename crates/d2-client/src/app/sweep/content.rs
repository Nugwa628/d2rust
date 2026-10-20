//! Content identities shared by generators, in the game's combined item order.
use super::*;
use d2_data::tables::{Armor, Itemtypes, Misc, Weapons};

#[derive(Clone)]
pub(super) struct Item {
    pub class: usize,
    pub code: String,
    /// The ground art file, without folder or extension.
    pub flippy: String,
    pub ty: u16,
    pub ty2: u16,
    pub level: u8,
    pub strength: u16,
    pub dexterity: u16,
    pub width: u8,
    pub height: u8,
    pub durability: u8,
    pub sockets: u8,
    pub stackable: bool,
    pub usable: bool,
    pub two_handed: bool,
}
pub(super) struct Catalog {
    pub items: Vec<Item>,
    pub types: Vec<Itemtypes>,
}
impl Catalog {
    pub fn load(live: &LiveData) -> Result<Self> {
        let mut items = Vec::new();
        macro_rules! rows {
            ($table:ty) => {
                for r in live.tables.rows::<$table>()? {
                    let code = String::from_utf8_lossy(&r.code)
                        .trim_matches([' ', '\0'])
                        .to_owned();
                    let flippy = String::from_utf8_lossy(&r.flippyfile)
                        .trim_matches([' ', '\0'])
                        .to_owned();
                    items.push(Item {
                        class: items.len(),
                        code,
                        flippy,
                        ty: r.type_,
                        ty2: r.type2,
                        level: r.levelreq,
                        strength: r.reqstr,
                        dexterity: r.reqdex,
                        width: r.invwidth,
                        height: r.invheight,
                        durability: if r.nodurability == 0 { r.durability } else { 0 },
                        sockets: r.gemsockets,
                        stackable: r.stackable != 0,
                        usable: r.useable != 0,
                        two_handed: r.f_2handed != 0,
                    });
                }
            };
        }
        rows!(Weapons);
        rows!(Armor);
        rows!(Misc);
        Ok(Self {
            items,
            types: live.tables.rows::<Itemtypes>()?,
        })
    }
    pub fn valid(&self, item: &Item) -> bool {
        (3..=4).contains(&item.code.len())
            && item
                .code
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
            && item.width > 0
            && item.height > 0
            && item.level <= 99
            && usize::from(item.ty) < self.types.len()
    }
    pub fn is_type(&self, item: &Item, wanted: i32) -> bool {
        let mut pending = vec![item.ty, item.ty2];
        let mut seen = std::collections::BTreeSet::new();
        while let Some(t) = pending.pop() {
            if !seen.insert(t) {
                continue;
            }
            if i32::from(t) == wanted {
                return true;
            }
            if let Some(row) = self.types.get(usize::from(t)) {
                pending.extend(
                    [row.equiv1, row.equiv2]
                        .into_iter()
                        .filter(|t| *t != 0 && *t != u16::MAX),
                );
            }
        }
        false
    }
    pub fn fits(&self, item: &Item, class: u8, body: u8) -> bool {
        self.valid(item)
            && self.types.get(usize::from(item.ty)).is_some_and(|t| {
                (t.class > 6 || t.class == class) && [t.bodyloc1, t.bodyloc2].contains(&body)
            })
    }
}

/// The scenario grammar omits the first occurrence rather than spelling `#0`.
pub(super) fn reference(class: usize, occurrence: usize) -> String {
    if occurrence == 0 {
        format!("@4:{class}")
    } else {
        format!("@4:{class}#{occurrence}")
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn repeated_item_references_are_valid_scenario_syntax() {
        for occurrence in [0, 1] {
            let reference = super::reference(68, occurrence);
            let text = format!("scenario 2\ntick_rate 60\nname reference\nend 4\nchar class 0\nat 1 msg PickItem type=4 id={reference} cursor=1\n");
            crate::scenario_script::Scenario::parse(&text).unwrap();
        }
    }
}
