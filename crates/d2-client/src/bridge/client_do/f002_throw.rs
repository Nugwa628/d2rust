//! cltdofunc 2 (Throw): one function of the client skill do
//! table (`super::client_do`); the shared helpers live in `super`.

use super::*;

/// Predict the equipped item's Throw missile; throwing potions use an arc.
/// Left Hand Throw selects the hand other than the current weapon.
pub(super) fn throw(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
) -> Result<i32, HandlerError> {
    if !w.units.contains_key(&key) {
        return Ok(0);
    }
    set_flag_40(w, key);
    let Some(tables) = w.item_tables.0.clone() else {
        return Ok(0);
    };
    let owned: Vec<_> = crate::bridge::items::items(w)
        .into_iter()
        .filter(|i| i.owner == Some(key) && !i.on_ground() && !i.store)
        .collect();
    let first = [4u8, 5].into_iter().find_map(|loc| {
        owned
            .iter()
            .find(|i| i.mode == crate::bridge::items::mode::BODY && i.body == loc)
            .filter(|i| {
                i.code
                    .is_some_and(|c| tables.item_is_type(c, 45) || tables.item_is_type(c, 38))
            })
    });
    let active = crate::bridge::attack_weapon::weapon_in_use(w, key).map(|item| item.key);
    let weapon = first
        .and_then(|item| {
            let in_use = active == Some(item.key);
            let other = if skill == 4 { in_use } else { !in_use };
            if other {
                owned
                    .iter()
                    .find(|other| {
                        other.mode == crate::bridge::items::mode::BODY
                            && other.body == if item.body == 4 { 5 } else { 4 }
                    })
                    .filter(|other| {
                        skill == 4
                            || active.is_some()
                            || other.code.is_some_and(|c| tables.type_throwable(c))
                    })
                    .or_else(|| (skill != 4 && active.is_none()).then_some(item))
            } else {
                Some(item)
            }
        })
        .and_then(|item| item.code);
    let Some((m, ty)) = weapon.and_then(|c| tables.missile_and_type(c)) else {
        return Ok(0);
    };
    if ty == 0x26 {
        return Ok(i32::from(lob(
            w,
            inputs,
            key,
            u32::from(m),
            (0, 0),
            skill,
            1,
        )?));
    }
    Ok(i32::from(
        cast_toward(w, inputs, key, i32::from(m), skill, 1)?.is_some(),
    ))
}

#[cfg(test)]
mod tests {
    use super::super::test_util::*;
    use crate::bridge::item_lists::{ItemProp, ItemTablesRef, StreamProps};
    use crate::bridge::world::{ItemData, ItemRecord, KindData};

    struct Throwable(i16);
    impl StreamProps for Throwable {
        fn props(&self, _: &[u8]) -> (Vec<ItemProp>, bool) {
            (Vec::new(), false)
        }
        fn item_is_type(&self, _: [u8; 4], ty: i16) -> bool {
            ty == self.0
        }
        fn missile_and_type(&self, _: [u8; 4]) -> Option<(u16, i16)> {
            Some((CLASS as u16, self.0))
        }
    }

    #[test]
    fn every_player_class_predicts_throwing_weapons_and_potions() {
        for class in 0..7 {
            for ty in [45, 38] {
                let (mut w, key) = world();
                let p = w.units.get_mut(&key).unwrap();
                p.class = class;
                p.cast_target = Some(CastTarget::Point(5012, 5000));
                w.item_tables = ItemTablesRef(Some(Arc::new(Throwable(ty))));
                let mut stream = Vec::new();
                let mut bit = 0;
                for (value, width) in [
                    (0x10, 32),
                    (0x65, 10),
                    (1, 3),
                    (4, 4),
                    (0, 4),
                    (0, 4),
                    (0, 3),
                    (u32::from_le_bytes(*b"pot "), 32),
                ] {
                    for shift in 0..width {
                        if bit / 8 == stream.len() {
                            stream.push(0);
                        }
                        stream[bit / 8] |= (((value >> shift) & 1) as u8) << (bit % 8);
                        bit += 1;
                    }
                }
                let item = UnitKey::new(crate::bridge::items::ITEM, 10);
                let mut u = ClientUnit::new(item);
                u.kind = KindData::Item(ItemData {
                    last: Some(ItemRecord {
                        id: 0x9D,
                        action: 6,
                        category: 0,
                        owner: Some(key),
                        stream,
                        seq: 0,
                    }),
                    ..ItemData::default()
                });
                w.units.insert(item, u);
                assert_eq!(
                    super::throw(&mut w, &inputs(2), key, FROST_NOVA).unwrap(),
                    1,
                    "class {class}, type {ty}"
                );
                let ms = missiles(&w);
                assert_eq!(ms.len(), 1);
                assert_eq!(ms[0].target_point, (5012, 5000));
            }
        }
    }
}
