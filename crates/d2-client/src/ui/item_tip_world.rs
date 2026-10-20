//! The tool tip's view of the client model: a model unit as a
//! [`TipUnit`] and the [`TipCtx`] of a hovered item (the local player as
//! P and U, the item's socket fillers, the set facts).
//!
//! The set-slot masks follow: every
//! worn (body) set piece of the hovered item's set that is not broken
//! (flags 0x4000 / 0x100 clear) sets the bit of its setitems `slot`
//! (+0x2E); no identified test. "Owned" follows over the model's
//! item modes (grid pages 0 / 3 / 4 and the body), identified pieces only.

use super::item_tip::ItemTips;
use super::item_tip_build::{SetCtx, TipCtx, TipUnit};
use super::item_tip_props::StatList;
use crate::bridge::items::{self, mode, ItemView};
use crate::bridge::world::{ClientWorld, KindData, UnitKey};
use d2_proto::item_bits::ItemBits;

/// A model unit read through `ClientWorld::total`.
pub struct WorldUnit<'a> {
    pub world: &'a ClientWorld,
    pub key: UnitKey,
}

impl<'a> WorldUnit<'a> {
    /// The local player, if any.
    pub fn local(world: &'a ClientWorld) -> Option<Self> {
        let key = world.local()?.key;
        Some(WorldUnit { world, key })
    }
}

impl TipUnit for WorldUnit<'_> {
    fn unit_type(&self) -> u8 {
        self.key.unit_type
    }
    fn class(&self) -> u32 {
        self.world.units.get(&self.key).map_or(0, |u| u.class)
    }
    fn stat(&self, id: u16, layer: u16) -> i32 {
        self.world.total(self.key, id, layer)
    }
    fn attack_anim(&self, weapon_class: [u8; 4]) -> Option<(i32, i32)> {
        let class = self.world.units.get(&self.key)?.class;
        self.world
            .tip_anims
            .0
            .as_ref()?
            .attack1(class, weapon_class)
    }
    fn has_state(&self, state: u8) -> bool {
        self.world
            .units
            .get(&self.key)
            .is_some_and(|u| u.states.contains(&state))
    }
    fn state_list(&self, state: u8) -> Option<StatList> {
        let list = self.world.units.get(&self.key)?.state_lists.get(&state)?;
        let mut l = StatList::default();
        for (&(s, layer), &v) in list {
            l.add(s, u32::from(layer), v);
        }
        Some(l)
    }
}

/// The decoded stream of model item `key` with the base values the model
/// changed since that record: S→C 0x3E sets the item's base array
/// (`ItemData::fields`), never its stream. The quantity is read from
/// there, so a tome shows its charges after a scroll goes in or a charge
/// is used.
pub fn model_bits(tips: &ItemTips, world: &ClientWorld, key: UnitKey) -> Option<ItemBits> {
    let mut b = tips.bits(items::stream(world, key)?)?;
    let fields = match world.units.get(&key).map(|u| &u.kind) {
        Some(KindData::Item(d)) => d.fields.as_ref(),
        _ => None,
    };
    if let (Some(f), Some(_)) = (fields, b.quantity) {
        // The base array leaves out zero values.
        let q = f
            .base
            .iter()
            .find(|p| p.stat == 70 && p.layer == 0)
            .map_or(0, |p| p.value);
        b.quantity = Some(q.clamp(0, i32::from(u16::MAX)) as u16);
    }
    Some(b)
}

/// The tip context of `item` hovered in the local player's panels:
/// P = U = `unit`, the item's fillers and the set facts.
pub fn hover_ctx<'a>(
    tips: &ItemTips,
    world: &ClientWorld,
    unit: Option<&'a WorldUnit<'a>>,
    item: &ItemView,
) -> TipCtx<'a> {
    let all = items::items(world);
    let fillers = all
        .iter()
        .filter(|i| i.mode == mode::SOCKETED && i.owner == Some(item.key))
        .filter_map(|f| items::stream(world, f.key).and_then(|s| tips.bits(s)))
        .collect();
    let mut set = SetCtx::default();
    let me = items::stream(world, item.key).and_then(|s| tips.bits(s));
    let set_of = |row: Option<u32>| {
        row.and_then(|r| {
            tips.tables()
                .setitems
                .get(r as usize)
                .map(|s| (s.set, s.slot))
        })
    };
    let my_set = me
        .as_ref()
        .filter(|b| b.quality == 5)
        .and_then(|b| set_of(b.quality_fields.file_index));
    if let (Some(u), Some((set_id, _))) = (unit, my_set) {
        for it in all
            .iter()
            .filter(|i| i.owner == Some(u.key) || i.owner.is_none())
        {
            let Some(b) = items::stream(world, it.key).and_then(|s| tips.bits(s)) else {
                continue;
            };
            if b.quality != 5 {
                continue;
            }
            let row = b.quality_fields.file_index;
            let owned = b.flags & 0x10 != 0
                && (it.mode == mode::BODY
                    || (it.mode == mode::STORED && matches!(it.page, 0 | 3 | 4)));
            if owned {
                set.owned.extend(row);
            }
            // Worn pieces only, broken ones (0x4000 / 0x100)
            // left out, identified or not.
            if it.mode != mode::BODY || b.flags & (0x4000 | 0x100) != 0 {
                continue;
            }
            if let Some((s, slot)) = set_of(row).filter(|(s, _)| *s == set_id) {
                let _ = s;
                let bit = 1u8.checked_shl(u32::from(slot)).unwrap_or(0);
                set.worn_with |= bit;
                if it.key != item.key {
                    set.worn_without |= bit;
                }
            }
        }
    }
    TipCtx {
        player: unit.map(|u| u as &dyn TipUnit),
        unit: unit.map(|u| u as &dyn TipUnit),
        fillers,
        set,
        ..TipCtx::default()
    }
}
