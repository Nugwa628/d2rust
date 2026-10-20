//! Animation keys shared by client presentation and server mode starts.
//! Unit tables supply token and mode names; equipped items supply the
//! player's weapon class. Death and throw modes apply the composite-name
//! overrides from render/unit-composite.md section 2.

use d2_sim::skills::sequences::CLASSES;
use d2_sim::units::UnitType;

use std::collections::BTreeMap;
use std::sync::Arc;

use d2_formats::animdata::AnimData;
use d2_sim::items::inventory::tables::InvTables;

use super::weapons::ItemFacts;
use crate::bridge::player_anim::{PlayerAnim, PlayerAnims};
use crate::bridge::world::{ClientUnit, ClientWorld, UnitKey};
use crate::rules::unit_composite::{code, CofName, CompositeKind};
use crate::world_view::unit_assets::{unit_cof, UnitLooks};

/// The 8-byte NUL-padded AnimData key of a unit of `ty` and `class` in
/// `mode`; `None` for a unit without a composite or a name over 7 bytes.
/// `weapon` is the player's weapon class index (`sequences::CLASSES`);
/// other units take their class from the tables.
pub fn anim_key(
    looks: &UnitLooks,
    ty: UnitType,
    class: u32,
    mode: u32,
    weapon: i32,
) -> Option<[u8; 8]> {
    let mut unit = ClientUnit::new(UnitKey::new(ty as u8, 0));
    unit.class = class;
    unit.mode = mode;
    let mut cof = unit_cof(looks, &unit)?;
    if cof.kind == CompositeKind::Player && !cof.kind.is_death_mode(mode as u8) {
        let w = usize::try_from(weapon)
            .ok()
            .and_then(|i| CLASSES.get(i))
            .map_or(*b"hth ", |c| code(c.as_bytes()));
        cof = CofName::player(cof.token, mode as u8, cof.mode_token, w);
    }
    let name = cof.short().to_ascii_uppercase();
    let b = name.as_bytes();
    (b.len() <= 8).then(|| {
        let mut k = [0u8; 8];
        k[..b.len()].copy_from_slice(b);
        k
    })
}

/// Use the AnimData speed unchanged; the preview adds no rate stats or states.
/// D2RUST's own, unverified.
pub fn anim_rate(speed: Option<u32>) -> i16 {
    speed.map_or(0, |s| s as i16)
}

/// The client player update's animation lookup
/// ([`crate::bridge::player_anim::PlayerAnims`]): the AnimData record of
/// [`anim_key`] for a model player, its weapon class from the items the
/// model shows on its body locations 4 and 5 (their last 0x9C / 0x9D
/// records), resolved as the server's ([`super::weapons::cof_class_of`]).
#[derive(Debug)]
pub struct ClientPlayerAnims {
    looks: Arc<UnitLooks>,
    anim: Arc<AnimData>,
    /// [`super::weapons::facts_of`] of each items-table row, by code.
    items: BTreeMap<[u8; 4], ItemFacts>,
}

impl ClientPlayerAnims {
    pub fn new(looks: Arc<UnitLooks>, anim: Arc<AnimData>, tables: &InvTables) -> Self {
        let items = (0..)
            .map_while(|r| tables.item(r).map(|i| (r, i.code)))
            .map(|(r, code)| (code, super::weapons::facts_of(tables, r)))
            .collect();
        Self { looks, anim, items }
    }
}

impl PlayerAnims for ClientPlayerAnims {
    fn weapon(&self, w: &ClientWorld, key: UnitKey) -> Option<i32> {
        let class = w.units.get(&key)?.class;
        let held = |loc: u8| {
            crate::bridge::items::items(w)
                .into_iter()
                .find(|i| {
                    i.owner == Some(key)
                        && i.mode == crate::bridge::items::mode::BODY
                        && i.body == loc
                })
                .and_then(|i| self.items.get(&i.code?).cloned())
        };
        Some(super::weapons::cof_class_of(held(4), held(5), class))
    }

    fn anim(&self, w: &ClientWorld, key: UnitKey, mode: u32) -> Option<PlayerAnim> {
        let class = w.units.get(&key)?.class;
        let weapon = self.weapon(w, key)?;
        let name = anim_key(&self.looks, UnitType::Player, class, mode, weapon)?;
        let r = self.anim.record(&name).ok()?;
        Some(PlayerAnim {
            frames: r.frames,
            speed: i32::from(r.speed as i16),
            weapon,
            events: Some(r.events),
        })
    }

    fn attack1(&self, class: u32, weapon: [u8; 4]) -> Option<(i32, i32)> {
        let w = std::str::from_utf8(&weapon).ok()?.trim_end();
        let idx = CLASSES.iter().position(|c| *c == w)?;
        let name = anim_key(
            &self.looks,
            UnitType::Player,
            class,
            7,
            i32::try_from(idx).ok()?,
        )?;
        let r = self.anim.record(&name).ok()?;
        Some((i32::try_from(r.frames).ok()?, i32::from(r.speed as i16)))
    }
}

impl d2_server::local_seams::UnitAppearance for UnitLooks {
    fn anim_key(&self, ty: UnitType, class: u32, mode: u32, weapon: i32) -> Option<[u8; 8]> {
        anim_key(self, ty, class, mode, weapon)
    }

    fn shield_choices(&self, class: u32) -> Option<u16> {
        self.shield_choices.get(&class).copied()
    }
}

impl d2_server::game::client_tables::MonsterAppearance for UnitLooks {
    fn monster_cof(&self, class: u32, mode: u32) -> Option<CofName> {
        let mut unit = ClientUnit::new(UnitKey::new(1, 0));
        (unit.class, unit.mode) = (class, mode);
        unit_cof(self, &unit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn looks() -> UnitLooks {
        let mut modes = vec![code(b""); 20];
        modes[6] = code(b"A1");
        modes[11] = code(b"TH");
        modes[18] = code(b"SQ");
        modes[17] = code(b"DD");
        UnitLooks {
            player_tokens: vec![code(b"AM")],
            player_modes: modes,
            ..UnitLooks::default()
        }
    }

    fn key(mode: u32, weapon: i32) -> String {
        let k = anim_key(&looks(), UnitType::Player, 0, mode, weapon).unwrap();
        String::from_utf8(k.iter().copied().take_while(|&b| b != 0).collect()).unwrap()
    }

    #[test]
    fn headless_animation_names_keep_the_client_keys() {
        use d2_server::game::appearance::AnimationNames;
        use d2_server::local_seams::UnitAppearance;
        let mut looks = looks();
        looks.monster_modes = (0..16).map(|_| code(b"A1")).collect();
        looks.object_modes = vec![code(b"NU"), code(b"OP")];
        looks.objects.insert(0, code(b"wz"));
        for (class, death) in [(0, false), (1, true)] {
            looks.monsters.insert(
                class,
                crate::world_view::unit_assets::MonsterRow {
                    token: code(b"sk"),
                    base_w: Some(code(b"1hs")),
                    composite_death: death,
                    directions: [8; 16],
                },
            );
        }
        let names = AnimationNames {
            players: looks.player_tokens.clone(),
            player_modes: looks.player_modes.clone(),
            monsters: looks
                .monsters
                .values()
                .map(|m| (m.token, m.base_w, m.composite_death))
                .collect(),
            monster_modes: looks.monster_modes.clone(),
            objects: vec![code(b"wz")],
            object_modes: looks.object_modes.clone(),
            ..AnimationNames::default()
        };
        for ty in [
            UnitType::Player,
            UnitType::Monster,
            UnitType::Object,
            UnitType::Missile,
            UnitType::Item,
            UnitType::Tile,
        ] {
            for class in 0..3 {
                for mode in 0..=256 {
                    for weapon in -1..=14 {
                        assert_eq!(
                            names.anim_key(ty, class, mode, weapon),
                            anim_key(&looks, ty, class, mode, weapon),
                            "{ty:?} class {class} mode {mode} weapon {weapon}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_key_uses_the_equipped_weapon_class() {
        // 1hs = class 2: A1 → `AMA11HS`; no item (0): `hth`.
        assert_eq!(key(6, 2), "AMA11HS");
        assert_eq!(key(6, 0), "AMA1HTH");
        // TH with a bow (1): not a throwing class → `hth`; with 1hs it stays.
        assert_eq!(key(11, 1), "AMTHHTH");
        assert_eq!(key(11, 2), "AMTH1HS");
        // SQ (18) uses the GH token and the weapon class.
        assert_eq!(key(18, 2), "AMGH1HS");
        // The death modes name `hth` whatever is held.
        assert_eq!(key(17, 2), "AMDDHTH");
    }
}
