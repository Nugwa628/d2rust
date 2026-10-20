//! Active class skills and legal target shapes, discovered from skills rows.
use super::content::{Catalog, Item};
use super::*;
use d2_data::tables::{Charstats, Leveldefs, Monstats, Skills};
use d2_sim::skills::use_::item_type::{self, ItemTypeSkill, ItemTypeView};

struct Hands<'a> {
    catalog: &'a Catalog,
    right: Option<usize>,
    left: Option<usize>,
}
impl ItemTypeView for Hands<'_> {
    type Item = usize;
    fn hand_item(&self, loc: u8) -> Option<usize> {
        if loc == 4 {
            self.right
        } else {
            self.left
        }
    }
    fn has_inventory(&self) -> bool {
        true
    }
    fn weapon_in_use(&self) -> Option<usize> {
        self.right.or(self.left)
    }
    fn is_a(&self, i: usize, ty: i32) -> bool {
        self.catalog.is_type(&self.catalog.items[i], ty)
    }
    fn ammo_free(&self, _: usize) -> bool {
        false
    }
    fn shoots(&self, i: usize) -> i32 {
        i32::from(self.catalog.types[usize::from(self.catalog.items[i].ty)].shoots as i16).max(0)
    }
    fn stat(&self, _: usize, stat: u16) -> i32 {
        i32::from(stat == 70)
    }
    fn stackable(&self, i: usize) -> bool {
        self.catalog.items[i].stackable
    }
}
fn equipment<'a>(catalog: &'a Catalog, skill: &Skills) -> Option<Vec<(&'a Item, u8)>> {
    let k = ItemTypeSkill::of(i32::from(skill.skill), skill);
    let mut hands = Hands {
        catalog,
        right: None,
        left: None,
    };
    if item_type::item_type_test(&hands, &k) && !skill.decquant {
        return Some(vec![]);
    }
    let choices = |body| {
        std::iter::once(None)
            .chain(
                catalog
                    .items
                    .iter()
                    .filter(move |i| {
                        catalog.fits(i, skill.charclass, body)
                            && i.strength <= 1023
                            && i.dexterity <= 1023
                    })
                    .map(|i| Some(i.class)),
            )
            .collect::<Vec<_>>()
    };
    for right in choices(4) {
        for left in choices(5) {
            if let (Some(r), Some(l)) = (right, left) {
                let weapon = catalog.is_type(&catalog.items[l], 45);
                if weapon && (!matches!(skill.charclass, 4 | 6) || catalog.items[r].two_handed) {
                    continue;
                }
                if catalog.items[r].two_handed && hands.shoots(r) == 0 {
                    continue;
                }
            }
            hands.right = right;
            hands.left = left;
            if item_type::item_type_test(&hands, &k)
                && (!skill.decquant
                    || right.is_some_and(|i| catalog.items[i].stackable || hands.shoots(i) > 0))
            {
                return Some(
                    right
                        .map(|i| (&catalog.items[i], 4))
                        .into_iter()
                        .chain(left.map(|i| (&catalog.items[i], 5)))
                        .collect(),
                );
            }
        }
    }
    None
}

pub(super) fn generate(live: &LiveData, g: &mut Generated) -> Result<()> {
    let catalog = Catalog::load(live)?;
    let classes = live.tables.rows::<Charstats>()?;
    let monsters = live.tables.rows::<Monstats>()?;
    let target = monsters
        .iter()
        .position(|m| m.enabled && !m.npc && m.killable);
    let fields = live.tables.rows::<Leveldefs>()?;
    let area = fields
        .iter()
        .enumerate()
        .find(|(id, r)| {
            *id > 1 && live.waypoints.levels.get(*id).is_some_and(|l| l.act == 0) && r.drlgtype > 0
        })
        .map(|(id, _)| id);
    let skills = live.tables.rows::<Skills>()?;
    for skill in &skills {
        if skill.charclass > 6 || usize::from(skill.charclass) >= classes.len() {
            g.skip("skills:not-class-skill");
            continue;
        }
        if skill.passive {
            g.skip("skills:passive-no-cast");
            continue;
        }
        if skill.aura {
            g.skip("skills:aura-selection-no-cast");
            continue;
        }
        if skill.reqlevel > 99 {
            g.skip("skills:level-out-of-range");
            continue;
        }
        if skill.targetcorpse || skill.targetitem || skill.targetpet || skill.targetally {
            g.skip("skills:special-target-required");
            continue;
        }
        let Some(gear) = equipment(&catalog, skill) else {
            g.skip("skills:no-eligible-equipment-or-ammunition");
            continue;
        };
        let Some(area) = area else {
            g.skip("skills:no-nontown-area");
            continue;
        };
        if skill.anim == u8::MAX {
            g.skip("skills:no-action-animation");
            continue;
        }
        // A skill restricted to states (shape forms) first casts the class
        // skill that grants one, and runs the rest of the case later.
        let form = if skill.restrict == 2 {
            let states = [skill.state1, skill.state2, skill.state3];
            let Some(form) = skills.iter().find(|r| {
                r.charclass == skill.charclass
                    && r.aurastate != u16::MAX
                    && states.contains(&r.aurastate)
            }) else {
                g.skip("skills:required-state-not-castable");
                continue;
            };
            Some(form.skill)
        } else {
            None
        };
        let shift = if form.is_some() { 64 } else { 0 };
        for shape in ["monster", "ground", "self"] {
            // Hand-to-hand skills aimed at the ground or self only walk, and a
            // skill whose cast starts from a nearby enemy search (Fist of the
            // Heavens, Taunt, Bone Prison, Dragon Flight) has no target for a
            // ground or self click: the game refuses it, pays no mana and
            // plays nothing. A point or open search can find a target there.
            let unit_target = skill.targetableonly
                || skill.range == 1
                || (skill.searchenemynear && !skill.searchenemyxy && !skill.searchopenxy);
            if unit_target && shape != "monster" {
                g.skip("skills:unit-target-required");
                continue;
            }
            let mut steps = String::from("at 0 dev god on\n");
            for (n, (item, body)) in gear.iter().enumerate() {
                let t = 1 + n * 4;
                let occurrence = usize::from(n == 1 && gear[0].0.class == item.class);
                let reference = super::content::reference(item.class, occurrence);
                steps.push_str(&format!("at {t} poke item {} @x @y quality normal ilvl 99\nat {} msg PickItem type=4 id={reference} cursor=1\nat {} msg EquipItem item={reference} bodyloc={body}\nat 14 check unit(item, {}).mode_id == 1\n", item.code, t+1, t+2, item.class));
            }
            let cast = match shape {
                "monster" => {
                    let Some(target) = target else {
                        g.skip("skills:no-eligible-monster");
                        continue;
                    };
                    // Hand-to-hand skills (range h2h/both) start next to the
                    // target so the case tests the skill, not the approach.
                    let gap = if matches!(skill.range, 1 | 3) { 2 } else { 6 };
                    steps.push_str(&format!("at 12 spawn {target} @x+{gap} @y normal\n"));
                    format!("cast {} at unit 1 {target}", skill.skill)
                }
                "ground" => format!("cast {} at 500 260", skill.skill),
                _ => format!("cast {} at unit 0 {}", skill.skill, skill.charclass),
            };
            if let Some(form) = form {
                steps.push_str(&format!(
                    "at 16 input cast {form} at unit 0 {}\n",
                    skill.charclass
                ));
            }
            steps.push_str(&format!("at {} input {cast}\nby {} check player.mode_id == {}\nat {} check player.mode == neutral\n", 16 + shift, 48 + shift, skill.anim, 180 + shift));
            if skill.mana > 0 || skill.minmana > 0 || skill.startmana > 0 {
                // Mana starts at exactly 1000; a skill that also raises the
                // maximum (warcries) can leave it above the start once paid.
                steps.push_str(&format!("by {} check player.mana != 1000\n", 72 + shift));
            }
            // Keep checks in source tick order, including by-deadlines.
            let mut lines: Vec<_> = steps.lines().collect();
            lines.sort_by_key(|l| {
                l.split_whitespace()
                    .nth(1)
                    .and_then(|t| t.parse::<u32>().ok())
                    .unwrap_or(0)
            });
            let header = format!("char class {}\nchar level {}\nchar area 0 {area}\n{}char skill {} 1\nchar hand right {}\nchar stat 8 256000\nchar stat 9 256000", skill.charclass, skill.reqlevel.max(1).max(u16::from(gear.iter().map(|(i,_)| i.level).max().unwrap_or(1))), form.map(|s| format!("char skill {s} 1\n")).unwrap_or_default(), skill.skill, skill.skill);
            // Raise strength/dexterity to the requirements, never below the
            // class's own starting values (lower stats can refuse the gear).
            let class = &classes[usize::from(skill.charclass)];
            let header = format!(
                "{header}\nchar stat 0 {}\nchar stat 2 {}",
                skill
                    .reqstr
                    .max(u16::from(class.str))
                    .max(gear.iter().map(|(i, _)| i.strength).max().unwrap_or(0)),
                skill
                    .reqdex
                    .max(u16::from(class.dex))
                    .max(gear.iter().map(|(i, _)| i.dexterity).max().unwrap_or(0))
            );
            g.add(
                "skills",
                format!("class-{}-skill-{}-{shape}", skill.charclass, skill.skill),
                &header,
                &lines.join("\n"),
                180 + shift,
            )?;
        }
    }
    Ok(())
}
