//! Unit tests of [`super::view`]: the component ops from the frame's light
//! map and the draw-mode decision, through [`LitRules`].

use d2_formats::cof::{Cof, CofLayer};
use d2_formats::palette::{Palette, Pl2, Rgb};

use super::super::blend::{cel_ops, OverrideInput, UnitKind};
use super::super::shading::{hover_light, ShadeTables};
use super::map::LightMap;
use super::view::{
    component_ops, layer_mode, override_of, unit_light_byte, ComponentLook, FrameLight, LitRules,
    LookFeed,
};
use crate::bridge::world::UnitKey;
use crate::bridge::ClientUnit;
use crate::composite::{ComponentRequest, CompositeError, Slot};
use crate::scene::{MapId, MapTable};
use crate::world_view::{Unspecified, ViewRules};

fn map(f: impl Fn(u32) -> u32) -> [u8; 256] {
    let mut m = [0u8; 256];
    for (i, v) in m.iter_mut().enumerate() {
        *v = (f(i as u32) % 256) as u8;
    }
    m
}

fn table(k: u32) -> Vec<[u8; 256]> {
    (0..256u32)
        .map(|d| map(|s| d * (3 + k) + s * (5 + 2 * k) + k + 1))
        .collect()
}

pub(crate) fn pl2() -> Pl2 {
    let mut colors = [Rgb::default(); 256];
    for (i, c) in colors.iter_mut().enumerate() {
        *c = Rgb {
            r: i as u8,
            g: i as u8,
            b: i as u8,
        };
    }
    Pl2 {
        base_palette: Palette { colors },
        light_levels: (0..32).map(|k| map(move |i| i * (k + 1) / 32)).collect(),
        inventory_variations: (0..16).map(|k| map(move |i| i + 100 + k)).collect(),
        selected_unit_shift: map(|i| i + 7),
        alpha_blend: (0..3).map(table).collect(),
        additive_blend: table(3),
        multiplicative_blend: table(4),
        hue_variations: (0..111).map(|k| map(move |i| i + k + 1)).collect(),
        red_tones: map(|i| i ^ 0x11),
        green_tones: map(|i| i ^ 0x22),
        blue_tones: map(|i| i ^ 0x33),
        unknown_variations: (0..14).map(|k| map(move |i| i ^ (0x40 + k))).collect(),
        max_component_blend: table(5),
        darkened_shift: map(|i| i / 2),
        text_colors: Vec::new(),
        text_color_shifts: Vec::new(),
    }
}

/// A frame light with the player at sub-tile (100, 100) and cell (101,
/// 100) at intensity `i`; every other cell 0.
pub(crate) fn frame_light(i: u8) -> FrameLight {
    let mut maps = MapTable::new();
    let tables = ShadeTables::push(&mut maps, &pl2());
    let mut m = LightMap::new((100, 100));
    let c = m.cell_mut(101 - m.origin.0, 100 - m.origin.1).unwrap();
    c.i = i;
    c.r = 1;
    FrameLight { tables, map: m }
}

pub(crate) fn layer(override_translucency: u8, new_translucency: u8) -> CofLayer {
    CofLayer {
        component: 0,
        shadow: 0,
        selectable: 1,
        override_translucency,
        new_translucency,
        weapon_class: *b"hth\0",
    }
}

pub(crate) fn cof(l: CofLayer) -> Cof {
    Cof {
        layers_count: 1,
        frames: 1,
        directions: 1,
        version: 20,
        unknown: [0; 4],
        x_min: 0,
        x_max: 0,
        y_min: 0,
        y_max: 0,
        animation_rate: 256,
        layers: vec![l],
        events: vec![0],
        event_padding: Vec::new(),
        draw_order: vec![0],
    }
}

pub(crate) fn request<'a>(cof: &'a Cof) -> ComponentRequest<'a> {
    ComponentRequest {
        cof,
        dir: 0,
        frame: 0,
        slot: Slot {
            slot: 0,
            component: 0,
            layer: 0,
        },
        layer: &cof.layers[0],
    }
}

fn look() -> ComponentLook {
    ComponentLook {
        ghostly: false,
        override_input: None,
        hovered: false,
        remap: None,
        pal: None,
    }
}

#[test]
fn layer_mode_reads_the_cof_override_bytes() {
    let plain = cof(layer(0, 3));
    let over = cof(layer(1, 3));
    assert_eq!(
        layer_mode(&look(), &request(&plain)),
        5,
        "byte 3 = 0: lv ignored"
    );
    assert_eq!(layer_mode(&look(), &request(&over)), 3);
    let hovered = ComponentLook {
        hovered: true,
        ..look()
    };
    assert_eq!(layer_mode(&hovered, &request(&plain)), 7);
    assert_eq!(
        layer_mode(&hovered, &request(&over)),
        3,
        "override layers are never highlighted"
    );
    let ghostly = ComponentLook {
        ghostly: true,
        ..hovered
    };
    assert_eq!(layer_mode(&ghostly, &request(&over)), 1);
    let faded = ComponentLook {
        override_input: Some(OverrideInput {
            kind: UnitKind::Player,
            fade: 1,
            monster_has_ethereal: false,
            item_trans: None,
            component: 0,
            item_ethereal: false,
        }),
        ..hovered
    };
    assert_eq!(
        layer_mode(&faded, &request(&plain)),
        1,
        "r before the highlight"
    );
}

#[test]
fn component_light_is_the_low_byte_of_the_units_cell() {
    let light = frame_light(0x7F);
    let plain = cof(layer(0, 0));
    let remap = Some(MapId(light.tables.remap0.0 + 1));
    let l = ComponentLook { remap, ..look() };
    assert_eq!(
        component_ops(&light, (101, 100), &l, &request(&plain)),
        cel_ops(&light.tables, 5, remap, 0x7F)
    );
    // Another cell: intensity 0 (map 0), not the 0x7F of (101, 100).
    assert_eq!(
        component_ops(&light, (100, 100), &l, &request(&plain)),
        cel_ops(&light.tables, 5, remap, 0)
    );
    // Far outside the map: the read clamps to the edge cell (0).
    assert_eq!(
        component_ops(&light, (1000, 100), &l, &request(&plain)),
        cel_ops(&light.tables, 5, remap, 0)
    );
    // The highlight mode (7) doubles the light byte.
    let hovered = ComponentLook { hovered: true, ..l };
    assert_eq!(
        component_ops(&light, (101, 100), &hovered, &request(&plain)),
        cel_ops(&light.tables, 7, remap, hover_light(0x7F))
    );
}

struct Feed {
    at: Result<(i32, i32), String>,
    look: ComponentLook,
}

impl LookFeed for Feed {
    fn light_subtile(&self, _: &ClientUnit) -> Result<(i32, i32), String> {
        self.at.clone()
    }

    fn look(&self, _: &ClientUnit, _: &ComponentRequest<'_>) -> Result<ComponentLook, String> {
        Ok(self.look)
    }

    fn hovered(&self, _: &ClientUnit) -> bool {
        self.look.hovered
    }
}

/// An overlay's light byte is its host's unit-draw light: the cell's low
/// byte (a speech balloon is lit like the NPC speaking it),
/// doubled and clamped when the host is hovered; a host without a
/// sub-tile draws it at full light.
#[test]
fn overlay_light_is_the_hosts_unit_draw_light() {
    let light = frame_light(0x6B);
    let unit = ClientUnit::new(UnitKey {
        unit_type: 1,
        guid: 7,
    });
    let feed = Feed {
        at: Ok((101, 100)),
        look: look(),
    };
    assert_eq!(unit_light_byte(&light, &feed, &unit), 0x6B);
    let hovered = Feed {
        at: Ok((101, 100)),
        look: ComponentLook {
            hovered: true,
            ..look()
        },
    };
    assert_eq!(unit_light_byte(&light, &hovered, &unit), hover_light(0x6B));
    assert_eq!(hover_light(0x6B), 0xD6);
    let missing = Feed {
        at: Err("no position".into()),
        look: look(),
    };
    assert_eq!(unit_light_byte(&light, &missing, &unit), 0xFF);
}

#[test]
fn lit_rules_answer_shade_and_blend_and_refuse_without_inputs() {
    let light = frame_light(0xC8);
    let c = cof(layer(1, 3));
    let unit = ClientUnit::new(UnitKey {
        unit_type: 0,
        guid: 1,
    });
    let feed = Feed {
        at: Ok((101, 100)),
        look: look(),
    };
    let rules = LitRules {
        rules: &Unspecified,
        feed: &feed,
        light: &light,
    };
    let (shade, blend) = cel_ops(&light.tables, 3, None, 0xC8);
    assert_eq!(rules.shade(&unit, &request(&c)).unwrap(), shade);
    assert_eq!(rules.blend(&unit, &request(&c)).unwrap(), blend);
    let missing = Feed {
        at: Err("no position".into()),
        look: look(),
    };
    let rules = LitRules {
        rules: &Unspecified,
        feed: &missing,
        light: &light,
    };
    assert!(matches!(
        rules.shade(&unit, &request(&c)),
        Err(CompositeError::Unresolved {
            what: "unit light",
            ..
        })
    ));
}

/// A monster component whose armor-class token names an item with
/// `transparent` set draws in that item's `transtbl` mode: the Fallen's
/// `tch` layer (misc Torch, `transparent` 1, `transtbl` 3) is mode 3, not
/// the COF's 5. A `fade` stat above 15 decides the mode before the item
/// is looked at.
#[test]
fn torch_item_sets_the_monster_component_mode() {
    let light = frame_light(0x80);
    let c = cof(layer(1, 5));
    let mut unit = ClientUnit::new(UnitKey {
        unit_type: 1,
        guid: 34,
    });
    let input = override_of(&unit, &request(&c), Some((1, 3)));
    assert_eq!(input.kind, UnitKind::Monster);
    let torch = ComponentLook {
        override_input: Some(input),
        ..look()
    };
    assert_eq!(
        component_ops(&light, (101, 100), &torch, &request(&c)),
        cel_ops(&light.tables, 3, None, 0x80)
    );
    let plain = ComponentLook {
        override_input: Some(override_of(&unit, &request(&c), Some((0, 3)))),
        ..look()
    };
    assert_eq!(
        component_ops(&light, (101, 100), &plain, &request(&c)),
        cel_ops(&light.tables, 5, None, 0x80)
    );
    unit.stats.insert(181, 16);
    let faded = override_of(&unit, &request(&c), Some((1, 3)));
    assert_eq!(super::super::blend::unit_override(&faded), None);
}
