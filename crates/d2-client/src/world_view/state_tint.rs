//! The unit state tint: a state with a `colorshift` recolours the unit it
//! is on through the unit palette index (unit `+0x6C`),
//! so the component's `P` is remap map `p − 1` of the act PL2.
//!
//! The index is the model's (`ClientUnit::palette_index`): the colour
//! call (`bridge::msg::lighting::state_colour_call`) sets it
//! when a state with `colorshift` ≠ 0 turns on or off, never at draw.

use crate::rules::shading::ShadeTables;
use crate::scene::MapId;

/// The remap `P` of a unit with palette index `p`: none for 0, when the
/// act tables are not resident, or for a `colorshift` past the 128 remap
/// maps (the table cannot hold it).
pub fn remap(p: u8, tables: Option<&ShadeTables>) -> Option<MapId> {
    tables?.unit_remap(p).ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tint_is_remap_map_shift_minus_one() {
        use crate::scene::MapTable;
        let pl2 =
            d2_formats::palette::Pl2::parse(&super::super::tile_assets::tests::pl2()).unwrap();
        let tables = ShadeTables::push(&mut MapTable::new(), &pl2);
        assert_eq!(remap(0, Some(&tables)), None);
        assert_eq!(remap(5, None), None);
        assert_eq!(remap(5, Some(&tables)), Some(MapId(tables.remap0.0 + 4)));
        assert_eq!(remap(200, Some(&tables)), None);
    }

    #[test]
    fn a_tinted_state_changes_the_units_palette_shift_in_the_draw_list() {
        use crate::bridge::world::{UnitKey, MONSTER};
        use crate::bridge::ClientUnit;
        use crate::rules::lighting::view::LitRules;
        use crate::rules::lighting::view_tests::{cof, frame_light, layer, request};
        use crate::world_view::preview_light::PreviewLook;
        use crate::world_view::{Unspecified, ViewRules};

        let light = frame_light(0xC8);
        let c = cof(layer(0, 0));
        let mut unit = ClientUnit::new(UnitKey::new(MONSTER, 7));
        unit.position = Some((101, 100));
        let look = PreviewLook {
            tables: Some(light.tables),
            ..PreviewLook::default()
        };
        let rules = LitRules {
            rules: &Unspecified,
            feed: &look,
            light: &light,
        };
        let plain = rules.shade(&unit, &request(&c)).unwrap();
        unit.palette_index = 5;
        let tinted = rules.shade(&unit, &request(&c)).unwrap();
        assert_ne!(plain, tinted);
        let remap = MapId(light.tables.remap0.0 + 4);
        assert!(tinted.maps().contains(&remap), "{tinted:?}");
        assert!(!plain.maps().contains(&remap));
        unit.palette_index = 0;
        assert_eq!(rules.shade(&unit, &request(&c)).unwrap(), plain);
    }
}
