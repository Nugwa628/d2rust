//! The play app's item parts (gap G16): the item art rows (`weapons`,
//! `armor`, `misc`: `invwidth`, `invheight`, `invfile`, `flippyfile`)
//! and the `inventory.bin` layouts, read from the user's tables and handed
//! to the inventory panel (`ui::panels::inv_items`) and the ground items
//! (`world_view::ground_items`).

use std::collections::BTreeMap;
use std::sync::Arc;

use bevy::prelude::*;
use d2_data::bin::TableFiles;
use d2_data::tables::{decode_all, Armor, Inventory, Itemtypes, Misc, Record, Weapons};

use crate::assets::path::FileSource;
use crate::bridge::items::{ItemArtRow, ItemArtRows};
use crate::ui::original::hud_belt::BeltParts;
use crate::ui::original::OriginalUi;
use crate::ui::panels::inv_items::{equip_rects, inv_layout};
use crate::world_view::ground_items::GroundItems;
use crate::world_view::WorldViewState;

/// The item parts the play UI reads at install
/// ([`prepare_ui`]); inserted by [`add_items`].
#[derive(Resource, Clone, Debug, Default)]
pub struct ItemParts {
    pub art: ItemArtRows,
    pub inventory: Vec<Inventory>,
    /// The belt records and types (`ui::hud_belt`).
    pub belts: BeltParts,
    /// The item tool tips' tables ([`item_tips`]); none on synthetic data.
    pub tips: Option<crate::ui::item_tip::ItemTips>,
    /// The measured frame size of each inventory graphic ([`inv_frame_sizes`]).
    pub frame_sizes: BTreeMap<String, (u32, u32)>,
    /// The inventory tables of the equip-box click (`ui::panels::inv_items`
    /// `equip`); none on synthetic data.
    pub inv_tables: Option<Arc<d2_sim::items::inventory::InvTables>>,
}

/// The size of frame 0 of each art row's inventory graphic by `invfile`
/// (lower case): the inventory draws the cel at (x, top + h). A
/// file no archive holds, or that does not parse, is left out (its size
/// is then estimated, `inv_items`).
pub fn inv_frame_sizes(source: &dyn FileSource, art: &ItemArtRows) -> BTreeMap<String, (u32, u32)> {
    let mut out = BTreeMap::new();
    for row in art.0.values() {
        let key = row.inv_file.to_ascii_lowercase();
        if key.is_empty() || out.contains_key(&key) {
            continue;
        }
        let path = crate::ui::inv_grid::inventory_path(&key);
        let Some(Ok(dc6)) = crate::assets::path::read_dc6(source, &path) else {
            continue;
        };
        if let Some(f) = dc6.frames.first().map(|f| (f.width, f.height)) {
            out.insert(key, f);
        }
    }
    out
}

/// A table's string column (zero-terminated).
fn text(b: &[u8]) -> String {
    let n = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..n]).trim().to_owned()
}

fn add_rows<T: Record>(
    set: &d2_data::bin::BinSet,
    rows: &mut BTreeMap<[u8; 4], ItemArtRow>,
    f: impl Fn(&T) -> ([u8; 4], ItemArtRow),
) -> Result<(), String> {
    let table = set
        .table(T::TABLE)
        .ok_or_else(|| format!("{} not loaded", T::TABLE))?;
    let decoded: Vec<T> = decode_all(table).map_err(|e| e.to_string())?;
    for r in &decoded {
        let (code, row) = f(r);
        rows.entry(code).or_insert(row);
    }
    Ok(())
}

macro_rules! art {
    ($r:expr, $t:expr) => {
        (
            $r.code,
            ItemArtRow {
                inv_w: $r.invwidth,
                inv_h: $r.invheight,
                inv_file: text(&$r.invfile),
                flippy_file: text(&$r.flippyfile),
                beltable: $t
                    .get(usize::from($r.type_))
                    .is_some_and(|t| t.beltable != 0)
                    && $r.invwidth == 1
                    && $r.invheight == 1,
            },
        )
    };
}

/// The item parts of the user's tables (a load error is an error).
pub fn item_parts(archives: &dyn TableFiles) -> Result<ItemParts, String> {
    let set = d2_data::bin::load_from(archives, "eng").map_err(|e| e.to_string())?;
    let mut rows = BTreeMap::new();
    let types: Vec<Itemtypes> =
        decode_all(set.table(Itemtypes::TABLE).ok_or("itemtypes not loaded")?)
            .map_err(|e| e.to_string())?;
    add_rows::<Weapons>(&set, &mut rows, |r| art!(r, types))?;
    add_rows::<Armor>(&set, &mut rows, |r| art!(r, types))?;
    add_rows::<Misc>(&set, &mut rows, |r| art!(r, types))?;
    let table = set.table("inventory").ok_or("inventory not loaded")?;
    let inventory: Vec<Inventory> = decode_all(table).map_err(|e| e.to_string())?;
    let mut belts = belt_parts(&set).unwrap_or_else(|e| {
        warn!("belt (D2RUST's own, unverified): {e}; no belt row");
        BeltParts::default()
    });
    belts.beltable = rows
        .iter()
        .filter(|(_, r)| r.beltable)
        .map(|(c, _)| *c)
        .collect();
    Ok(ItemParts {
        art: ItemArtRows(rows),
        inventory,
        belts,
        tips: None,
        frame_sizes: BTreeMap::new(),
        inv_tables: None,
    })
}

/// The item tool tips of the user's tables: the game's item tables for
/// the stream reader, the name and description columns, and the string
/// tables (`ui::item_tip`). D2RUST's own, unverified: a load error is
/// logged by the caller and leaves the preview without tips.
pub fn item_tips(
    archives: &dyn TableFiles,
    items: d2_sim::items::ItemTables,
) -> Result<crate::ui::item_tip::ItemTips, String> {
    let set = d2_data::bin::load_from(archives, "eng").map_err(|e| e.to_string())?;
    let strings = super::strings::TableStrings::load(archives, super::strings::LANG)
        .map_err(|e| e.to_string())?;
    crate::ui::item_tip::ItemTips::new(Arc::new(items), &set, Arc::new(strings))
}

/// The `belts.bin` records (`BeltRecord::from_bytes` of each 0x108-byte
/// record) and the worn belts' rows (`armor` `belt`).
fn belt_parts(set: &d2_data::bin::BinSet) -> Result<BeltParts, String> {
    use crate::ui::panels::control::belt::BeltRecord;
    let table = set.table("belts").ok_or("belts not loaded")?;
    let records = table
        .records
        .chunks(table.record_size.max(1))
        .filter_map(BeltRecord::from_bytes)
        .collect();
    let armor = set.table(Armor::TABLE).ok_or("armor not loaded")?;
    let armor: Vec<Armor> = decode_all(armor).map_err(|e| e.to_string())?;
    // `belt` 0 is also the default of non-belts: only belt items matter,
    // and the host looks up the worn item at body location 8.
    let types = armor.iter().map(|r| (r.code, r.belt)).collect();
    Ok(BeltParts {
        records,
        types,
        beltable: Default::default(),
    })
}

/// Hands the item parts to the world view's ground items (with `source`
/// for the flippy files) and keeps them for [`prepare_ui`]. Call after
/// the world view state exists and before the play UI is added.
pub fn add_items(
    app: &mut impl crate::app::session::Setup,
    source: Arc<dyn FileSource>,
    mut parts: ItemParts,
) {
    parts.frame_sizes = inv_frame_sizes(source.as_ref(), &parts.art);
    if let Some(mut state) = app.world_mut().get_resource_mut::<WorldViewState>() {
        state.ground_items = GroundItems::new(source, parts.art.clone());
    }
    app.world_mut().insert_resource(parts);
}

/// The inventory panel's item art and layouts, before the UI's file list
/// is taken (`OriginalUi::files`); nothing without [`ItemParts`].
pub fn prepare_ui(app: &impl crate::app::session::Setup, original: &mut OriginalUi) {
    let Some(parts) = app.world().get_resource::<ItemParts>() else {
        return;
    };
    original.set_item_art(parts.art.clone());
    original.set_inv_layouts(parts.inventory.iter().map(inv_layout).collect());
    original.set_equip_rects(parts.inventory.iter().map(equip_rects).collect());
    original.set_item_frame_sizes(parts.frame_sizes.clone());
    original.set_belt_parts(parts.belts.clone());
    if let Some(tips) = &parts.tips {
        original.set_item_tips(tips.clone());
    }
    if let Some(t) = &parts.inv_tables {
        original.set_inv_tables(t.clone());
    }
}

/// The item-stream decoder of the model over the game's item tables
/// (`bridge::item_lists`).
pub struct TableDecoder(pub Arc<d2_sim::items::ItemTables>);

impl crate::bridge::item_lists::StreamProps for TableDecoder {
    fn item_is_type(&self, code: [u8; 4], t: i16) -> bool {
        self.0.find_code(code).is_some_and(|i| self.0.is_type(i, t))
    }
    fn weapon_range_adder(&self, code: [u8; 4]) -> Option<i32> {
        let i = self.0.find_code(code)?;
        // Type 45 `weap`.
        self.0
            .is_type(i, 45)
            .then(|| self.0.item(i).map_or(0, |r| i32::from(r.rangeadder)))
    }
    fn missile_and_type(&self, code: [u8; 4]) -> Option<(u16, i16)> {
        let r = self.0.item(self.0.find_code(code)?)?;
        Some((r.missiletype, r.type_))
    }
    fn type_shoots(&self, code: [u8; 4]) -> i16 {
        self.0
            .find_code(code)
            .and_then(|i| self.0.itype_of(i))
            .map_or(0, |t| t.shoots as i16)
    }
    fn stackable(&self, code: [u8; 4]) -> bool {
        self.0
            .find_code(code)
            .and_then(|i| self.0.item(i))
            .is_some_and(|r| r.stackable != 0)
    }
    fn type_class(&self, code: [u8; 4]) -> u32 {
        let Some(r) = self.0.find_code(code).and_then(|i| self.0.item(i)) else {
            return 0;
        };
        let c = r
            .wclass
            .split(|&b| b == 0 || b == b' ')
            .next()
            .unwrap_or(&[]);
        match d2_sim::skills::sequences::CLASSES
            .iter()
            .position(|k| k.as_bytes() == c)
        {
            Some(i @ (1..=7 | 12)) => i as u32,
            _ => 0,
        }
    }
    fn component(&self, code: [u8; 4]) -> u8 {
        self.0
            .find_code(code)
            .and_then(|i| self.0.item(i))
            .map_or(0, |r| r.component)
    }
    fn type_throwable(&self, code: [u8; 4]) -> bool {
        self.0
            .find_code(code)
            .and_then(|i| self.0.itype_of(i))
            .is_some_and(|t| t.throwable != 0)
    }
    fn props(&self, stream: &[u8]) -> (Vec<crate::bridge::item_lists::ItemProp>, bool) {
        use d2_proto::item_bits::{decode, ItemLookup};
        let t = &*self.0;
        let lookup = d2_server::adapters::item_bits::TablesLookup(t);
        // A failed record makes no item.
        let Some(b) = decode(stream, &lookup).ok().filter(|b| !b.failed) else {
            return (Vec::new(), false);
        };
        // An alt-code record carries its base code.
        let charm = lookup
            .code(b.base_code.unwrap_or(b.code))
            .is_some_and(|c| c.charm);
        let shift = |s: u16| t.valshift.get(usize::from(s)).copied().unwrap_or(0);
        (stream_props(&b, shift), charm)
    }
    fn fields(&self, stream: &[u8]) -> Option<crate::bridge::item_lists::ItemFields> {
        client_item_fields(&self.0, stream)
    }
}

/// The item data the client reader writes from `stream` (readers; Compact).
pub fn client_item_fields(
    t: &d2_sim::items::ItemTables,
    stream: &[u8],
) -> Option<crate::bridge::item_lists::ItemFields> {
    use crate::bridge::item_lists::{ItemFields, ItemProp};
    use d2_proto::item_bits::{decode, hflag};
    use d2_sim::items::bitstream::{AUTO_OFFSET, PREFIX_OFFSET};
    let lookup = d2_server::adapters::item_bits::TablesLookup(t);
    let b = decode(stream, &lookup).ok().filter(|b| !b.failed)?;
    let code = b.base_code.unwrap_or(b.code);
    let class = t.find_code(code)?;
    // The header word without 0x2000000, then 0x80000
    // cleared; the client clears bit 2 when the header has it.
    let mut flags = b.flags & !0x0200_0000 & !0x0008_0000;
    if b.flags & 2 != 0 {
        flags &= !2;
    }
    let mut f = ItemFields {
        class: class as u32,
        quality: 2,
        flags,
        ilvl: 1,
        ..ItemFields::default()
    };
    let prop = |stat: u16, value: i64| ItemProp {
        stat,
        layer: 0,
        value: value as i32,
    };
    let mut base: Vec<ItemProp> = Vec::new();
    if b.flags & hflag::ALT_CODE != 0 {
        f.quality = 1;
        f.flags = flags;
        return Some(f);
    }
    if b.flags & hflag::COMPACT != 0 {
        if let Some(g) = b.gold {
            base.push(prop(14, i64::from(g)));
        }
        // Scroll codes: `isc ` puts 1 in the first suffix slot.
        if code == *b"isc " {
            f.suffix[0] = 1;
        }
        f.base = base;
        return Some(f);
    }
    let ident = b.flags & hflag::IDENTIFIED != 0;
    f.quality = u32::from(b.quality);
    f.ilvl = u32::from(b.ilvl).max(1);
    let add = |v: u16, off: u16| if v == 0 { 0 } else { v.wrapping_add(off) };
    f.auto = b.auto_affix.map_or(0, |a| add(a, AUTO_OFFSET));
    let q = &b.quality_fields;
    if let Some(n) = q.file_index {
        f.file_index = n as i32;
    }
    match b.quality {
        4 => {
            if let Some((p, s)) = q.magic {
                f.prefix[0] = add(p, PREFIX_OFFSET);
                f.suffix[0] = s;
            }
        }
        6 | 8 => {
            if let Some((p, s)) = q.rare_names.filter(|_| ident) {
                f.rare = (u16::from(p), u16::from(s));
            }
            if let Some(slots) = q.rare_slots {
                for (i, (p, s)) in slots.iter().enumerate() {
                    f.prefix[i] = add(*p, PREFIX_OFFSET);
                    f.suffix[i] = *s;
                }
            }
        }
        9 => {
            if let Some((p, s)) = q.rare_names.filter(|_| ident) {
                f.rare = (u16::from(p), u16::from(s));
            }
        }
        7 if !ident => f.file_index = 0,
        2 => {
            if let Some((is_prefix, id)) = q.charm {
                if is_prefix {
                    f.prefix[0] = add(id, PREFIX_OFFSET);
                } else {
                    f.suffix[0] = id;
                }
            }
            if let Some(sp) = q.spell {
                f.suffix[0] = u16::from(sp);
            }
        }
        _ => {}
    }
    if b.quality == 7 && f.file_index >= t.uniques.len() as i32 {
        f.file_index = -1;
    }
    // The base array (after the quality part).
    let rec = t.item(class)?;
    let low = b.quality == 1;
    let weap = t.is_type(class, 45);
    let armo = t.is_type(class, 50);
    let plain = |s: Option<d2_proto::item_bits::Stat>| s.map(|s| s.value());
    if armo {
        if let Some(d) = plain(b.defense) {
            base.push(prop(31, d));
        }
        base.push(prop(20, i64::from(rec.block)));
        base.push(prop(67, i64::from(rec.speed.wrapping_neg())));
    } else if weap {
        base.push(prop(68, i64::from(rec.speed.wrapping_neg())));
        let eth = flags & 0x0040_0000 != 0;
        let dmg = |v: u8, floor: i64| -> i64 {
            let v = i64::from(v);
            let v = if low { ((v * 3) >> 2).max(floor) } else { v };
            if eth {
                v * 3 / 2
            } else {
                v
            }
        };
        let thrown = rec.maxmisdam != 0;
        let list = [
            (22u16, rec.maxdam, 2),
            (21, rec.mindam, 1),
            (24, rec.maxdam2, 2),
            (23, rec.mindam2, 1),
        ];
        for (s, v, floor) in list {
            if v != 0 || !low {
                base.push(prop(s, dmg(v, floor)));
            }
        }
        if thrown {
            base.push(prop(159, dmg(rec.minmisdam, 1)));
            base.push(prop(160, dmg(rec.maxmisdam, 2)));
        }
    }
    if let Some(m) = plain(b.max_durability) {
        base.push(prop(73, m));
        if m != 0 {
            if let Some(d) = plain(b.durability) {
                base.push(prop(72, d));
            }
        }
    }
    if let Some(n) = b.quantity {
        base.push(prop(70, i64::from(n)));
    }
    if let Some(n) = b.sockets {
        base.push(prop(194, i64::from(n)));
    }
    base.retain(|p| p.value != 0);
    base.sort_by_key(|p| (p.stat, p.layer));
    f.base = base;
    Some(f)
}

/// `quantity`.
const STAT_QUANTITY: u16 = 70;
/// `item_numsockets`.
const STAT_SOCKETS: u16 = 194;

/// The values one item stream puts in the item's list: the item's own
/// Values (defense 31, durability 72 / max durability 73, quantity
/// 70, sockets 194: its base array) and its property lists, each stored as
/// (field − `Save Add`) << `ValShift`; grouped partners are sent unshifted.
pub fn stream_props(
    b: &d2_proto::item_bits::ItemBits,
    shift: impl Fn(u16) -> u8 + Copy,
) -> Vec<crate::bridge::item_lists::ItemProp> {
    use crate::bridge::item_lists::ItemProp;
    let own = [b.defense, b.max_durability, b.durability];
    let mut props: Vec<ItemProp> = own
        .iter()
        .flatten()
        .map(|d| vec![*d])
        .chain(b.lists.iter().flatten().cloned())
        .flat_map(|list| crate::ui::item_tip_props::stream_values(&list, shift))
        .map(|(stat, layer, value)| ItemProp {
            stat,
            layer: layer as u16,
            value,
        })
        .collect();
    let plain = [
        (STAT_QUANTITY, b.quantity.map(i32::from)),
        (STAT_SOCKETS, b.sockets.map(i32::from)),
    ];
    for (stat, v) in plain {
        if let Some(v) = v {
            props.push(ItemProp {
                stat,
                layer: 0,
                value: v << shift(stat),
            });
        }
    }
    props
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::path::MemorySource;

    /// A one-frame DC6 of `w` × `h`, every row one
    /// literal run.
    fn dc6(w: u32, h: u32) -> Vec<u8> {
        let mut rows = Vec::new();
        for _ in 0..h {
            rows.push(w as u8);
            rows.extend((0..w).map(|i| 1 + i as u8));
            rows.push(0x80);
        }
        let mut d = Vec::new();
        for v in [6i32, 1, 0] {
            d.extend_from_slice(&v.to_le_bytes());
        }
        d.extend_from_slice(&[0xEE; 4]);
        d.extend_from_slice(&1u32.to_le_bytes());
        d.extend_from_slice(&1u32.to_le_bytes());
        d.extend_from_slice(&((d.len() + 4) as u32).to_le_bytes());
        for v in [0u32, w, h, 0, 0, 0, 0, rows.len() as u32] {
            d.extend_from_slice(&v.to_le_bytes());
        }
        d.extend_from_slice(&rows);
        d.extend_from_slice(&[0xEE; 3]);
        d
    }

    // The item's own values (durability, max durability, quantity,
    // sockets, defense) land in its list beside the property list.
    #[test]
    fn the_items_own_values_are_in_its_list() {
        use d2_proto::item_bits::{ItemBits, Stat};
        let st = |stat: u16, raw: u32, save_add: u32| Stat {
            stat,
            param: 0,
            raw,
            save_add,
        };
        let b = ItemBits {
            defense: Some(st(31, 30, 10)),
            max_durability: Some(st(73, 20, 0)),
            durability: Some(st(72, 17, 0)),
            quantity: Some(250),
            sockets: Some(2),
            lists: vec![Some(vec![st(0, 5, 0)])],
            ..ItemBits::default()
        };
        let got: Vec<(u16, i32)> = stream_props(&b, |s| if s == 70 { 1 } else { 0 })
            .iter()
            .map(|p| (p.stat, p.value))
            .collect();
        assert_eq!(
            got,
            vec![(31, 20), (73, 20), (72, 17), (0, 5), (70, 500), (194, 2)]
        );
    }

    // The item cel is drawn at (x, top + h) with its frame's own height: the
    // host measures frame 0 of each invfile; a file no archive holds is left to
    // the estimate.
    #[test]
    fn inventory_graphics_are_measured() {
        let mut src = MemorySource::default();
        src.insert(r"data\global\items\invsst.dc6", dc6(28, 84));
        let row = |f: &str| ItemArtRow {
            inv_w: 1,
            inv_h: 4,
            inv_file: f.into(),
            flippy_file: String::new(),
            beltable: false,
        };
        let art = ItemArtRows(BTreeMap::from([
            (*b"sst ", row("invSST")),
            (*b"xxx ", row("invxxx")),
        ]));
        assert_eq!(
            inv_frame_sizes(&src, &art),
            BTreeMap::from([("invsst".to_string(), (28, 84))])
        );
    }
}
