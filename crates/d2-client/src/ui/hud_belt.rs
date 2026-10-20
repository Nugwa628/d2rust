//! The belt in play: the rules of `ui::panels::control::belt` bound to
//! the client model. The belt items are drawn in their boxes,
//! the pop-up rows show while the mouse is over the belt strip,
//! and a click on a box sends 0x23 / 0x24 / 0x25. The belt key
//! uses are `bridge::belt`.
//!
//! D2RUST's own, unverified (preview fills):
//! - the belt type is read from the worn belt's `armor` row (`belt`
//!   column), default type 2 without one;
//! - the resolution index of `belts.bin` is 0 below mode 2, else 1;
//! - every belt item counts as usable, nothing is blocked; the cursor
//!   item fits a belt (`BeltParts::beltable`, from the tables);
//! - the key labels are the play bindings' key names for the belt slot actions
//!   ([`HudBelt::set_keys`]): the first bound key; an unbound slot has no
//!   label; the default `1`–`4` until bindings are set; no cut to width 28; not
//!   the string ids 4049 / 4050;
//! - the hover text has no shop price;
//! - the highlight rectangles ([`BeltDraw::Box`]) take the palette's
//!   nearest colours from the inventory's tint colours (the same four
//!   triples).

use std::collections::BTreeMap;

use crate::bridge::items::{self, mode};
use crate::bridge::world::ClientWorld;
use crate::ui::draw::{RectRequest, UiDraw, UiDrawSink};
use crate::ui::item_tip::ItemTips;
use crate::ui::panel::ClientIntent;
use crate::ui::panels::control::belt::{
    hover_text, record_index, BeltDraw, BeltEffect, BeltItem, BeltRecord, BeltSlot8, BeltState,
    CursorInfo, CursorItem, HoverText, MoveGates, SlotInfo, FONT_AFTER_BELT,
};
use crate::ui::panels::inv_items::ItemsUi;
use crate::ui::panels::UiFiles;

/// The popped belt rows' art.
pub const POPBELT: &str = "panel\\ctrlpnl_popbelt";

/// The `belts.bin` records and the worn belts' types.
#[derive(Clone, Debug, Default)]
pub struct BeltParts {
    /// Records by `resolution · 7 + type`.
    pub records: Vec<BeltRecord>,
    /// Item code → `belts` row (`armor.txt` `belt`).
    pub types: BTreeMap<[u8; 4], u8>,
    /// Item codes that fit a belt box: the type's itemtypes `beltable` and
    /// a 1 x 1 size.
    pub beltable: std::collections::BTreeSet<[u8; 4]>,
}

/// The belt of the HUD: the rule state and the tables.
#[derive(Clone, Debug, Default)]
pub struct HudBelt {
    pub state: BeltState,
    pub parts: BeltParts,
    /// The key name of each belt slot's action: `None` = not set yet
    /// (the default label), `Some(None)` = unbound.
    pub keys: Option<[Option<String>; 4]>,
    /// UI state 0x1F (belt rows) is open (set by the HUD before the draw).
    pub state_1f_open: bool,
    /// UI state 9 (game menu) is open.
    pub state_9_open: bool,
}

fn belt_view(world: &ClientWorld) -> BTreeMap<u16, crate::bridge::items::ItemView> {
    items::belt(world)
}

impl HudBelt {
    /// The record of the current belt type at resolution `res2`.
    fn record(&self, res2: bool) -> Option<&BeltRecord> {
        self.parts
            .records
            .get(record_index(u32::from(res2), self.state.belt_type))
    }

    /// The type from the worn belt (body location 8).
    fn update_type(&mut self, world: &ClientWorld) {
        let worn = items::local_items(world)
            .into_iter()
            .find(|i| i.mode == mode::BODY && i.body == 8);
        let slot8 = match worn {
            None => BeltSlot8::Empty,
            Some(i) => match i.code.and_then(|c| self.parts.types.get(&c)) {
                Some(&t) => BeltSlot8::Belt(i32::from(t)),
                None => BeltSlot8::Other,
            },
        };
        let _ = self.state.update_type(slot8);
    }

    fn slot_item(world: &ClientWorld) -> impl Fn(usize) -> Option<BeltItem> {
        let belt = belt_view(world);
        move |i| {
            belt.get(&(i as u16)).map(|v| BeltItem {
                guid: v.key.guid,
                usable: true,
                has_use: true,
                blocked: false,
                pos_x: i,
                quality3: false,
            })
        }
    }

    fn cursor(&self, world: &ClientWorld) -> CursorInfo {
        CursorInfo {
            mode: 0,
            item: items::cursor_item(world).map(|c| CursorItem {
                guid: c.key.guid,
                fits_belt: c.code.is_some_and(|k| self.parts.beltable.contains(&k)),
                swap_ok: true,
                blocked: false,
                put_sound: 0,
            }),
        }
    }

    /// Current belt boxes, restricted to the same hit area used by input.
    pub fn ui_elements(&self, w: i32, h: i32, res2: bool) -> Vec<crate::ui::ui_ids::UiElement> {
        let Some(rec) = self.record(res2) else {
            return Vec::new();
        };
        rec.boxes
            .iter()
            .enumerate()
            .filter_map(|(i, b)| {
                let r = crate::ui::geom::Rect::new(
                    b.left,
                    b.top,
                    (b.right - b.left + 1).try_into().ok()?,
                    (b.bottom - b.top + 1).try_into().ok()?,
                );
                let e = crate::ui::ui_ids::UiElement::new(format!("belt.slot({i})"), r);
                (self.state.hit_area(rec, w, h, e.at.x, e.at.y)
                    && self.state.box_hit(rec, e.at.x, e.at.y) == Some(i))
                .then_some(e)
            })
            .collect()
    }

    /// The top of the area the belt takes while popped (for the panel's
    /// rectangle), when popped.
    pub fn popped_top(&self, res2: bool) -> Option<i32> {
        if !self.state.popped {
            return None;
        }
        self.record(res2)?.boxes.iter().map(|b| b.top).min()
    }

    /// The key label of belt slot `i`: the default binding's name (D2RUST's
    /// own, unverified).
    fn key_name(&self, i: usize) -> Option<Vec<u16>> {
        if i >= 4 {
            return None;
        }
        match &self.keys {
            None => Some(vec![u16::from(b'1') + i as u16]),
            Some(k) => k[i].as_ref().map(|n| n.encode_utf16().collect()),
        }
    }

    /// Takes the key names of the belt slot actions from `b` (the
    /// primary, i.e. first, bound key of each).
    pub fn set_keys(&mut self, b: &crate::controls::Bindings) {
        use crate::controls::Action;
        let acts = [
            Action::BeltSlot1,
            Action::BeltSlot2,
            Action::BeltSlot3,
            Action::BeltSlot4,
        ];
        self.keys = Some(acts.map(|a| b.inputs(a).first().map(|k| k.name().to_string())));
    }

    /// The belt's draw list: the type, the hover tracking, the
    /// pop-up rows, then the slots (rectangles, items, key labels) and the
    /// cursor-item highlight.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_list(
        &mut self,
        world: &ClientWorld,
        (w, h): (i32, i32),
        res2: bool,
        mouse: (i32, i32),
        alive: bool,
    ) -> Vec<BeltDraw> {
        self.update_type(world);
        // The belt draw clears the pop-up flag with state 0x1F closed (or state
        // 9 open); the state 0x1F draw sets it again.
        self.state
            .clear_popped(self.state_1f_open, self.state_9_open);
        let Some(rec) = self.record(res2).cloned() else {
            return Vec::new();
        };
        let slot_item = Self::slot_item(world);
        let cursor = self.cursor(world);
        let gates = MoveGates {
            in_game: true,
            input_blocked: false,
            alive,
            x44bfe0: false,
            state_1f_open: self.state_1f_open,
            state_9_open: self.state_9_open,
        };
        self.state
            .mouse_move(&rec, w, h, mouse.0, mouse.1, &gates, &cursor, &slot_item);
        let mut draws = self.state.popup_draws(w, h);
        if self.state_1f_open {
            self.state.state_1f_draw();
        }
        let slots: Vec<SlotInfo> = (0..rec.boxes.len())
            .map(|i| SlotInfo {
                item: slot_item(i),
                key_name: self.key_name(i),
            })
            .collect();
        draws.extend(self.state.slot_draws(&rec, &slots));
        // The cursor-item highlight, after every slot (over the
        // items).
        let occupied = |i: usize| slot_item(i).is_some();
        draws.extend(
            self.state
                .cursor_highlight(&rec, gates.state_1f_open, &cursor, &occupied),
        );
        draws
    }

    /// Draws the belt (`draw_list`). `mouse` is the pointer in the frame.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        world: &ClientWorld,
        items_ui: &ItemsUi,
        files: &UiFiles,
        size: (i32, i32),
        res2: bool,
        mouse: (i32, i32),
        alive: bool,
        out: &mut dyn UiDrawSink,
    ) {
        let draws = self.draw_list(world, size, res2, mouse, alive);
        let belt = belt_view(world);
        for d in draws {
            match d {
                BeltDraw::PopRow { x, y } => {
                    if let Some(f) = files.id(POPBELT) {
                        out.push(crate::ui::panels::cel(f, 0, x, y));
                    }
                }
                BeltDraw::Item { guid, x, y } => {
                    if let Some(v) = belt.values().find(|v| v.key.guid == guid) {
                        items_ui.draw_at(world, files, v, (x, y), out);
                    }
                }
                // Font 1, color 4.
                BeltDraw::Label(l) => {
                    out.push(crate::ui::panels::text(
                        l.text,
                        l.x,
                        l.y,
                        FONT_AFTER_BELT,
                        l.color as u16,
                    ));
                }
                // A box is `(left, top, 29, 29, colour, 0)`, the colour
                // the palette's nearest entry (the inventory's tint colours:
                // the same four triples); no palette given: no box.
                BeltDraw::Box { rect, color } => {
                    if let Some(c) = items_ui.tint_colors {
                        out.push(UiDraw::Rect(RectRequest::sized(
                            rect.x,
                            rect.y,
                            rect.w,
                            rect.h,
                            c[color as usize],
                            rect.mode,
                        )));
                    }
                }
            }
        }
    }

    /// The hover text of the hovered belt item: T and its
    /// pop-up point, colour and centring, as the pop-up draw
    /// draws it. None unless the hover gate of [`hover_text`] holds
    /// (belt hovered, an item, no cursor item, box ≤ 3 or popped) and the
    /// item's stream gives a tip.
    pub fn hover_tip(&self, world: &ClientWorld, tips: &ItemTips) -> Option<HoverText> {
        let belt = belt_view(world);
        let v = self
            .state
            .hover_item
            .and_then(|g| belt.values().find(|v| v.key.guid == g))?;
        let stream = items::stream(world, v.key)?;
        let me = crate::ui::item_tip_world::WorldUnit::local(world);
        let ctx = crate::ui::item_tip_world::hover_ctx(tips, world, me.as_ref(), v);
        // N, S.
        let (name, stats) = tips.name_and_properties(stream, &ctx)?;
        let item = BeltItem {
            guid: v.key.guid,
            usable: true,
            has_use: true,
            blocked: false,
            pos_x: 0,
            quality3: tips
                .bits(stream)
                .is_some_and(|b| b.quality == d2_sim::items::q::SUPERIOR),
        };
        let cursor = items::cursor_item(world).is_some();
        hover_text(&self.state, cursor, Some(&item), &name, &stats, None)
    }

    /// Whether the point is on the belt (hit area).
    pub fn over(
        &self,
        world: &ClientWorld,
        (w, h): (i32, i32),
        res2: bool,
        at: (i32, i32),
    ) -> bool {
        let _ = world;
        self.record(res2)
            .is_some_and(|r| self.state.hit_area(r, w, h, at.0, at.1))
    }

    /// The belt click on a release: the intents it sends.
    pub fn click(&self, world: &ClientWorld, res2: bool, at: (i32, i32)) -> Vec<ClientIntent> {
        let Some(rec) = self.record(res2) else {
            return Vec::new();
        };
        self.state
            .click(
                rec,
                at.0,
                at.1,
                &self.cursor(world),
                &Self::slot_item(world),
            )
            .into_iter()
            .filter_map(|e| match e {
                BeltEffect::Send(i) => Some(i),
                _ => None,
            })
            .collect()
    }
}
