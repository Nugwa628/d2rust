//! The NPC menu (ui 8) in play: the box of [`build_npc_menu`] /
//! [`MenuBox`] anchored above the interaction's NPC, opened by S→C 0x28
//! (`msg_ui`) as `SetUIState(8, on)` so the ui-states gates apply, the
//! hire-list request C→S 0x38 action 3 for 252 / 198 / 515 / 150, the
//! record lookup by NPC class (no match → record 0), the waiting state of
//! the Identify / Resurrect sends and S→C 0x2A's rebuild, and the hire
//! list's Back rebuilding the menu.
//!
//! The adapter owns the interaction's state; the box, its captions, its
//! layout and its draw are the menu modules'. Draws go through the
//! present sink: the framed box is the sink's rectangle
//! ([`RectRequest::sized`]).
// D2RUST's own choices for the menu box's input:
// - an item's hit band is (pen y − height, pen y] across the box width;
// - the pointer over a selectable item selects it (style 1: color 3);
// - a left release on the item pressed runs its handler; a left press
//   outside the box ends the interaction (C→S 0x30, `SetUIState(8,
//   off)`) and is consumed;
// - Talk and the services are `npc_talk`'s (topic box, dialog panel,
//   item-socket dialog);
// - Charsi's Imbue row is inserted before the cancel;
// - the NPC's name line is empty (monstats name keys are not in the
//   client model) and the heal cost is 0 (not in the client model);
// - an Identify / Resurrect send closes the box until S→C 0x2A.

use std::cell::RefCell;
use std::rc::Rc;

use super::game_messages::Measure;
use super::SharedRef;
use crate::bridge::items;
use crate::bridge::world::{ClientWorld, UnitKey};
use crate::controls::Action;
use crate::rules::camera::{moving_to_client, Camera, FrameSize, OpenMode};
use crate::ui::draw::{RectRequest, TextRequest, TextStyle, UiDraw, UiDrawSink};
use crate::ui::geom::{Point, Rect};
use crate::ui::hire_list::hire_intent;
use crate::ui::layout::{MenuOption, NpcMenuRecord, OptionKind};
use crate::ui::messages::socket::NPC_CHARSI;
use crate::ui::messages::Ltrb;
use crate::ui::panel::ActionId;
use crate::ui::panel::WidgetId;
use crate::ui::panel::{ClientIntent, Panel, PanelId, StringLookup, UiCtx, UiEvent, UiResponse};
use crate::ui::panels::menu_box::{anchor, MenuBox, MenuDraw, WaitingNote, STR_WAITING};
use crate::ui::panels::npc::{
    cain_count_reset, msg_chat_end, option_intent, record_index, NpcMenus,
};
use crate::ui::panels::npc_menu::{
    build_npc_menu, hire_open, CaptionCtx, HireOpen, NpcMenuBuild, NpcMenuHandler, NpcMenuInput,
};
use crate::ui::panels::shop::{
    confirm_answer, confirm_box, ConfirmHandler, ConfirmResult, ShopEffect, TxKind,
};
use crate::ui::panels::PanelOutput;
use crate::ui::root::UiRoot;
use crate::ui::states::id;
use crate::ui::text::TextOpts;
use crate::ui::ui_ids::UiElement;
use crate::ui::PointerButton;

use super::OriginalUi;

/// A box item's text and color (tests).
pub type BoxItem = (Vec<u16>, u8);

/// The panel id: the UI state (ui 8), so the root mirrors its flag.
pub const NPC_MENU_PANEL: PanelId = PanelId(id::NPC_MENU as u16);
/// The menu state of a send that waits for S→C 0x2A.
pub const MENU_WAITING: u8 = 10;
/// The menu state after a failed transaction.
pub const MENU_FAILED: u8 = 12;
/// UI sound of a 0x2A result 1 outside the waiting states.
pub const SOUND_TX_DONE: i32 = 221;
/// `NPCImbue` string of D2RUST's own Imbue row.
const STR_IMBUE: u16 = 4017;

/// One selectable row of the box: its string id, the option it runs
/// (`None` = the cancel item) and the `%d` its caption carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub string: u16,
    pub kind: Option<OptionKind>,
    pub cost: Option<u32>,
}

/// The open menu, as tests and the host see it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Open {
    pub guid: u32,
    pub class: u32,
    /// The selectable items of the box in order (the options, then the
    /// cancel item).
    pub rows: Vec<Row>,
    /// Talk was chosen: the box is closed, the topic box or the dialog
    /// panel is up (`npc_talk`).
    pub talking: bool,
}

/// Cain's identify count: the
/// local player's items without flag 0x10 in the backpack (page 0), the
/// cube (page 3) or worn; the stash and the belt are skipped.
pub fn unidentified_count(world: &ClientWorld) -> u32 {
    items::local_items(world)
        .iter()
        .filter(|i| {
            i.flags & 0x10 == 0
                && match i.mode {
                    items::mode::STORED => i.page == 0 || i.page == 3,
                    items::mode::BODY => true,
                    _ => false,
                }
        })
        .count() as u32
}

/// The interaction the menu belongs to.
#[derive(Clone, Debug)]
struct Interaction {
    guid: u32,
    class: u32,
    identify_n: u32,
    /// The box was asked for and not built yet (needs the strings).
    build: bool,
}

#[derive(Default)]
pub struct NpcMenuState {
    up: Option<Interaction>,
    bx: Option<MenuBox<NpcMenuHandler>>,
    /// The record the box was built from (its options by slot).
    rec: Option<NpcMenuRecord>,
    rows: Vec<Row>,
    /// The NPC's anchor, taken at the open / rebuild.
    anchor: Option<(i32, i32)>,
    /// The item index the left press went down on.
    pressed: Option<usize>,
    /// The menu's waiting state ([`MENU_WAITING`], [`MENU_FAILED`]).
    pub menu_state: u8,
    /// The waiting note.
    note: Option<WaitingNote>,
    /// Messages of the build (C→S 0x38 action 3) waiting for the root.
    pending: Vec<PanelOutput>,
    menus: Option<NpcMenus>,
    /// The talk, its dialog panel and the item-socket dialog
    /// (`npc_talk`).
    pub(super) talk: super::npc_talk::TalkState,
    /// The last Trade / Gamble choice was Gamble: the shop that opens next
    /// is a gamble window (the gamble shop flag).
    pub gamble: bool,
    /// The NPC (GUID, class) the last Trade / Gamble was chosen for and
    /// the model's store serial at the choice (`OriginalUi::shop_poll`).
    pub shop_for: Option<(u32, u32, u32)>,
    /// The Resurrect edit of the next open: the cost while the mercenary
    /// is dead in an expansion game.
    pub resurrect: Option<u32>,
    /// The mercenary's name string id (S→C 0x9B).
    pub merc_name: u16,
    /// The Trade / Gamble choice's NPC (GUID, class): the shop opens for
    /// it on the next shop poll. D2RUST's own.
    pub shop_request: Option<(u32, u32)>,
    pub screen: (i32, i32),
}

pub type SharedNpcMenu = Rc<RefCell<NpcMenuState>>;

/// The camera of the local player: the walk prediction's precise position
/// while it holds, else the model's cell centre (D2RUST's own: the view's
/// shake is not in the model).
fn camera(w: &ClientWorld, open_mode: u8) -> Option<Camera> {
    let me = w.local()?;
    let (px, py) = w.predicted(me).map_or_else(
        || {
            let (x, y) = me.cell();
            ((u32::from(x) << 16) | 0x8000, (u32::from(y) << 16) | 0x8000)
        },
        |p| p.pos,
    );
    let at = moving_to_client(px, py);
    let mode = OpenMode::new(open_mode).unwrap_or(OpenMode::NONE);
    // The low-resolution UI uses the standard menu camera; other canvases
    // use the same camera dimensions as the world.
    let play = FrameSize::play();
    let size = if play == FrameSize::LOW {
        FrameSize::D2RUST
    } else {
        play
    };
    Some(Camera::new(size, mode, at, (0, 0)))
}

/// The NPC's anchor: the NPC's client pixel point (the cell centre here,
/// D2RUST's own: the precise path point is not in the model)
/// less the tile origin (`view +0x24/+0x28`, [`Camera::tile`], not the unit
/// draw origin), raised by 150 (at least 20); none without the NPC.
fn npc_anchor(world: &ClientWorld, guid: u32, open_mode: u8) -> Option<(i32, i32)> {
    let (x, y) = world.units.get(&UnitKey::new(1, guid))?.position?;
    let cam = camera(world, open_mode)?;
    let c = moving_to_client((u32::from(x) << 16) | 0x8000, (u32::from(y) << 16) | 0x8000);
    // The camera's point is in frame space; the box is laid out in the UI
    // box, `ui_left` pixels into the frame (0 at 4:3).
    let ox = FrameSize::play().ui_left();
    anchor(Some((c.x, c.y)), (cam.tile.x, cam.tile.y)).map(|(x, y)| (x - ox, y))
}

fn string_fn(s: &dyn StringLookup) -> impl Fn(u16) -> Vec<u16> + '_ {
    move |id| s.get_id(id).map(<[u16]>::to_vec).unwrap_or_default()
}

/// An item's hit band (module doc: D2RUST's own).
pub(super) fn item_rect<H>(bx: &MenuBox<H>, i: usize) -> Ltrb {
    let top = bx.pos.1 + bx.items[..i].iter().map(|it| it.height).sum::<i32>();
    Ltrb::new(
        bx.pos.0,
        top + 1,
        bx.pos.0 + bx.size.0,
        top + bx.items[i].height + 1,
    )
}

fn ltrb_contains(r: &Ltrb, p: Point) -> bool {
    p.x >= r.l && p.x < r.r && p.y >= r.t && p.y < r.b
}

/// The selectable items of `bx` as `npc.<name>(N)` elements, N counting
/// only the selectable ones, top down.
pub(super) fn menu_elements<H: Clone + PartialEq>(bx: &MenuBox<H>, name: &str) -> Vec<UiElement> {
    (0..bx.items.len())
        .filter(|&i| bx.items[i].selectable)
        .enumerate()
        .map(|(n, i)| {
            let r = item_rect(bx, i);
            let w = u16::try_from(r.r - r.l).unwrap_or(0);
            let h = u16::try_from(r.b - r.t).unwrap_or(0);
            UiElement::new(format!("npc.{name}({n})"), Rect::new(r.l, r.t, w, h))
        })
        .filter(|e| e.rect.w > 0 && e.rect.h > 0)
        .collect()
}

/// The item of `bx` at `p` that is selectable.
pub(super) fn item_at<H: Clone + PartialEq>(bx: &MenuBox<H>, p: Point) -> Option<usize> {
    (0..bx.items.len()).find(|&i| bx.items[i].selectable && ltrb_contains(&item_rect(bx, i), p))
}

/// The draws of a menu box through the present sink.
/// `clip` is the screen (every UI text is clipped to it).
///
/// `border` is the `menu\boxpieces` file id the box border needs
/// (`None`: the file is not loaded, the border is left out).
pub(crate) fn push_menu_draws(
    draws: Vec<MenuDraw>,
    clip: Rect,
    border: Option<u32>,
    out: &mut dyn UiDrawSink,
) {
    for d in draws {
        match d {
            MenuDraw::Frame(r) => out.push(UiDraw::Rect(RectRequest::sized(
                r.x, r.y, r.w, r.h, r.color, r.mode,
            ))),
            MenuDraw::Text {
                text,
                x,
                y,
                color,
                font,
            } => out.push(UiDraw::Text(TextRequest {
                text,
                at: Point::new(x, y),
                style: TextStyle {
                    font,
                    color: u16::from(color),
                },
                opts: TextOpts::default(),
                clip,
            })),
            MenuDraw::Border { l, t, r, b } => {
                if let Some(file) = border {
                    crate::ui::box_border::push_border(file, (l, t, r, b), clip, out);
                }
            }
            // Cels and the pentagram need the cel draw of the sink
            // (q-fix-ui-draw-sink); the NPC menu (style 1) has neither.
            MenuDraw::Cel { .. } | MenuDraw::Pentspin { .. } => {}
        }
    }
}

/// Refresh only geometry: selection, handlers and timers remain intact.
pub(super) fn reflow_box<H: Clone + PartialEq>(
    bx: &mut MenuBox<H>,
    screen: (i32, i32),
    at: (i32, i32),
    metrics: &dyn crate::ui::messages::Metrics,
) {
    bx.anchor = at;
    // A retained box already passed layout validation at a fixed frame.
    // Keep its known size if changed metrics cannot lay it out again.
    if bx.layout(screen.0, screen.1, metrics).is_err() {
        bx.pos.0 = bx.pos.0.clamp(10, (screen.0 - bx.size.0 - 10).max(10));
        bx.pos.1 = bx.pos.1.clamp(10, (screen.1 - bx.size.1 - 48).max(10));
    }
}

impl NpcMenuState {
    /// The menus table, loaded once and edited per interaction.
    fn menus(&mut self) -> &mut NpcMenus {
        self.menus.get_or_insert_with(|| {
            NpcMenus::load().unwrap_or_else(|_| NpcMenus::from_records(Vec::new()))
        })
    }

    /// The interaction start: the table edits; the
    /// box is built by [`Self::build`] once the strings are at hand.
    fn start(&mut self, guid: u32, class: u32, level: i32, identify_n: u32) {
        let resurrect = self.resurrect.is_some();
        let menus = self.menus();
        menus.reset_for_interaction();
        menus.apply_builder(level);
        menus.apply_resurrect(resurrect, true);
        self.talk.topic = None;
        self.talk.active = false;
        self.note = None;
        self.menu_state = 1;
        self.up = Some(Interaction {
            guid,
            class,
            identify_n,
            build: true,
        });
    }

    /// The GUID of the NPC this interaction is with, when one is up.
    pub(super) fn interaction_guid(&self) -> Option<u32> {
        self.up.as_ref().map(|i| i.guid)
    }

    /// The menu box: the record of the NPC's class (no match → record 0)
    /// with its captions. `npc_name` is the title row
    /// ([`OriginalUi::npc_menu_poll`]).
    fn build(
        &mut self,
        world: &ClientWorld,
        strings: &dyn StringLookup,
        npc_name: Vec<u16>,
        m: &Measure<'_>,
    ) {
        let Some(it) = self.up.clone() else {
            return;
        };
        self.up.as_mut().unwrap().build = false;
        let (records, idx) = {
            let menus = self.menus();
            let idx = record_index(menus.records(), Some(it.class));
            (menus.records().to_vec(), idx)
        };
        let Some(mut rec) = idx.and_then(|i| records.get(i).cloned()) else {
            return;
        };
        let mut imbue = false;
        if it.class == NPC_CHARSI {
            // D2RUST's own: the imbue row.
            let slot = (rec.count as usize).saturating_sub(1);
            if slot < rec.options.len() {
                rec.options[slot] = Some(MenuOption {
                    string: STR_IMBUE,
                    kind: OptionKind::Imbue,
                });
                rec.count += 1;
                imbue = true;
            }
        }
        let count = rec.count;
        let player = world.local();
        let captions = CaptionCtx {
            merc_name_id: self.merc_name,
            resurrect_cost: self.resurrect.map_or(0, |c| c as i32),
            life_below_max: player
                .is_some_and(|p| world.total(p.key, 6, 0) < world.total(p.key, 7, 0)),
            heal_cost: 0,
            identify_n: it.identify_n as i32,
            // The quest-4 bits that waive the cost are not in the client
            // model (D2RUST's own, unverified).
            identify_quest_bits_clear: true,
            quest41_bit0: false,
            quest41_bit1: false,
            difficulty: world.difficulty,
        };
        let interaction_ok = world.units.contains_key(&UnitKey::new(1, it.guid));
        let anchor = self.anchor.or(Some((self.screen.0 / 2, self.screen.1 / 3)));
        let input = NpcMenuInput {
            npc_class: it.class,
            npc_guid: it.guid,
            interaction_ok,
            npc_name,
            player_guid: player.map(|p| p.key.guid),
            captions: &captions,
        };
        let strings = string_fn(strings);
        match build_npc_menu(
            &mut rec,
            &input,
            anchor.unwrap_or((0, 0)),
            &strings,
            self.screen,
            m,
        ) {
            Ok(NpcMenuBuild::Ended(out)) => {
                self.pending.push(PanelOutput::Intent(ClientIntent(
                    msg_chat_end(it.guid).to_vec(),
                )));
                self.pending.extend(out);
                self.close();
            }
            Ok(NpcMenuBuild::Built { send, bx }) => {
                self.pending.extend(send);
                self.rows = bx
                    .items
                    .iter()
                    .filter(|i| i.selectable)
                    .map(|i| match i.handler {
                        Some(NpcMenuHandler::Slot(s)) => {
                            let o = rec.options[s].expect("a built slot has an option");
                            Row {
                                string: o.string,
                                kind: Some(o.kind),
                                cost: match o.kind {
                                    OptionKind::Identify => Some(100 * it.identify_n),
                                    OptionKind::Resurrect => self.resurrect,
                                    _ => None,
                                },
                            }
                        }
                        _ => Row {
                            string: crate::ui::panels::npc_menu::STR_LOWER_CANCEL,
                            kind: None,
                            cost: None,
                        },
                    })
                    .collect();
                // The identify caption sets the table record's count.
                if rec.count != count {
                    let c = rec.count - u32::from(imbue);
                    if let Some(r) = idx.and_then(|i| self.menus().record_mut(i)) {
                        r.count = c;
                    }
                }
                self.rec = Some(rec);
                self.bx = Some(bx);
                self.pressed = None;
            }
            // A layout that does not fit: no box (D2RUST's own).
            Err(_) => self.bx = None,
        }
    }

    /// The box and the interaction go (the interaction's end).
    fn close(&mut self) {
        self.talk.topic = None;
        self.talk.active = false;
        if self.talk.dialog.panel.is_some() {
            let _ = self.talk.dialog.close();
            // The text goes with the interaction, and its line with it.
            self.pending.push(PanelOutput::DialogSpeechEnd);
        }
        self.up = None;
        self.bx = None;
        self.rec = None;
        self.rows.clear();
        self.pressed = None;
        self.menu_state = 0;
    }

    /// The interaction's end: C→S 0x30
    /// [1][GUID], `SetUIState(8, off)`.
    fn end(&mut self, guid: u32) -> Vec<PanelOutput> {
        self.close();
        vec![
            PanelOutput::Intent(ClientIntent(msg_chat_end(guid).to_vec())),
            PanelOutput::SetUi {
                ui: id::NPC_MENU,
                mode: 1,
                jump: false,
            },
        ]
    }

    /// [`Self::end`] for the talk and the services.
    pub(super) fn end_interaction(&mut self, guid: u32) -> Vec<PanelOutput> {
        self.end(guid)
    }

    /// The B2 dialog flag is set (until the dialog's end callback): the
    /// flag clears and the interaction ends as in state 1, so the
    /// callback never opens the menu.
    /// `None` when no B2 dialog holds the flag.
    pub(super) fn end_b2_dialog(&mut self) -> Option<Vec<PanelOutput>> {
        let (b2, ..) = self.talk.menu_after.take()?;
        self.talk.menu_ready = None;
        self.talk.pending_open = None;
        let guid = self.interaction_guid().unwrap_or(b2);
        Some(self.end(guid))
    }

    /// The interaction NPC's class.
    pub(super) fn npc_class(&self) -> Option<u32> {
        self.up.as_ref().map(|it| it.class)
    }

    /// The built record's flag byte.
    pub(super) fn record_flag(&self) -> u8 {
        self.rec.as_ref().map_or(0, |r| r.flag)
    }

    /// The menu is built again at the next poll.
    pub(super) fn ask_build(&mut self) {
        if let Some(it) = self.up.as_mut() {
            it.build = true;
        }
    }

    /// Reposition retained boxes without replaying their build effects.
    pub fn resize_screen(
        &mut self,
        screen: crate::ui::layout::Screen,
        fonts: Option<&super::FontMeasure>,
    ) {
        let bounds = (screen.w, screen.h);
        if self.screen == bounds {
            return;
        }
        self.screen = bounds;
        self.anchor = None;
        self.pressed = None;
        let m = Measure(fonts);
        let anchor = self.anchor_or_centre();
        if let Some(bx) = self.bx.as_mut() {
            reflow_box(bx, bounds, anchor, &m);
        }
        if let Some(note) = self.note.as_mut() {
            reflow_box(&mut note.bx, bounds, anchor, &m);
        }
        self.talk.resize_screen(screen, &m);
    }

    /// The NPC's anchor, else the frame's centre (D2RUST's own).
    pub(super) fn anchor_or_centre(&self) -> (i32, i32) {
        self.anchor
            .unwrap_or((self.screen.0 / 2, self.screen.1 / 3))
    }

    /// The whole screen: the clip of every draw and the area of a box
    /// that takes every press.
    pub(super) fn screen_rect(&self) -> Rect {
        Rect::new(0, 0, self.screen.0 as u16, self.screen.1 as u16)
    }

    /// Outputs that wait for the next poll (outside an event).
    pub(super) fn push_pending(&mut self, o: Vec<PanelOutput>) {
        self.pending.extend(o);
    }

    fn open_view(&self) -> Option<Open> {
        let it = self.up.as_ref()?;
        let talking = self.talk.active;
        (self.bx.is_some() || talking).then(|| Open {
            guid: it.guid,
            class: it.class,
            rows: if self.bx.is_some() {
                self.rows.clone()
            } else {
                Vec::new()
            },
            talking,
        })
    }

    /// The selectable item `k` of the box, as an item index.
    fn row_item(&self, k: usize) -> Option<usize> {
        let bx = self.bx.as_ref()?;
        (0..bx.items.len())
            .filter(|&i| bx.items[i].selectable)
            .nth(k)
    }
}

/// The panel.
pub struct NpcMenuUi {
    pub(super) sh: SharedRef,
    /// The shop, whose confirm dialog this panel draws and answers (ui 8
    /// stays open under the shop).
    pub(super) shop: super::shop_ui::SharedShop,
    pub st: SharedNpcMenu,
    pub hire: crate::ui::hire_list::SharedHire,
}

impl NpcMenuUi {
    fn out(&self, o: Vec<PanelOutput>) {
        self.sh.borrow_mut().outputs.extend(o);
    }

    pub(super) fn out_pub(&self, o: Vec<PanelOutput>) {
        self.out(o);
    }

    fn run(&mut self, item: usize, ctx: &UiCtx) -> UiResponse {
        let (handler, guid, class) = {
            let st = self.st.borrow();
            let (Some(bx), Some(it)) = (st.bx.as_ref(), st.up.as_ref()) else {
                return UiResponse::Consumed;
            };
            (bx.items[item].handler, it.guid, it.class)
        };
        let kind = match handler {
            Some(NpcMenuHandler::Slot(s)) => self
                .st
                .borrow()
                .rec
                .as_ref()
                .and_then(|r| r.options[s])
                .map(|o| o.kind),
            Some(NpcMenuHandler::Cancel) => {
                let o = self.st.borrow_mut().end(guid);
                self.out(o);
                return UiResponse::Consumed;
            }
            _ => None,
        };
        let Some(kind) = kind else {
            return UiResponse::Consumed;
        };
        let mut st = self.st.borrow_mut();
        // Every option handler closes the box first.
        st.bx = None;
        st.pressed = None;
        match kind {
            OptionKind::Talk => {
                drop(st);
                self.talk_open(ctx);
                UiResponse::Consumed
            }
            OptionKind::Imbue => {
                let now = self.sh.borrow().ui_ms32(ctx.tick);
                let o = super::npc_talk::open_imbue(&mut st, guid, class, ctx.world, now);
                drop(st);
                self.out(o);
                UiResponse::Consumed
            }
            OptionKind::Hire => {
                let up = self.hire.borrow().up.is_some();
                match hire_open(up) {
                    HireOpen::CloseAndRebuild => {
                        self.hire.borrow_mut().up = None;
                        if let Some(it) = st.up.as_mut() {
                            it.build = true;
                        }
                    }
                    HireOpen::Build => self.hire.borrow_mut().up = Some(guid),
                }
                UiResponse::Consumed
            }
            kind => {
                if matches!(kind, OptionKind::Trade | OptionKind::Gamble) {
                    st.gamble = kind == OptionKind::Gamble;
                    st.shop_for = Some((guid, class, ctx.world.store_serial));
                    st.shop_request = Some((guid, class));
                }
                if matches!(kind, OptionKind::Identify | OptionKind::Resurrect) {
                    st.menu_state = MENU_WAITING;
                }
                drop(st);
                if let Some(o) = option_intent(kind, guid) {
                    self.out(o);
                }
                UiResponse::Consumed
            }
        }
    }

    /// The shop's confirm dialog: Yes / No by
    /// `confirm_answer`. The pointer and press handling are the menu
    /// box's (D2RUST's own, module doc).
    fn confirm_event(&mut self, e: UiEvent) -> UiResponse {
        let Some(at) = e.at() else {
            return UiResponse::Ignored;
        };
        let item = {
            let shop = self.shop.borrow();
            let Some(c) = shop.confirm.as_ref() else {
                return UiResponse::Ignored;
            };
            item_at(&c.bx, at)
        };
        let mut shop = self.shop.borrow_mut();
        let Some(c) = shop.confirm.as_mut() else {
            return UiResponse::Ignored;
        };
        match e {
            UiEvent::CursorMoved(_) => {
                if let Some(i) = item {
                    c.bx.selected = i as i32;
                }
                return UiResponse::Ignored;
            }
            UiEvent::Press {
                button: PointerButton::Left,
                ..
            } => {
                c.pressed = item;
                return UiResponse::Consumed;
            }
            UiEvent::Release {
                button: PointerButton::Left,
                ..
            } => {}
            _ => return UiResponse::Consumed,
        }
        let pressed = c.pressed.take();
        let Some(i) = item.filter(|&i| pressed == Some(i)) else {
            return UiResponse::Consumed;
        };
        let yes = c.bx.items[i].handler == Some(ConfirmHandler::Yes);
        let c = shop.confirm.take().expect("checked above");
        match confirm_answer(c.kind, shop.tx.menu_state, yes) {
            ConfirmResult::Send => {
                let effects = shop.tx.send_with(0, c.now, &c.facts);
                let mut out = Vec::new();
                for e in effects {
                    match e {
                        ShopEffect::Send(i) => out.push(PanelOutput::Intent(i)),
                        ShopEffect::Sound(id) => out.push(PanelOutput::Sound(id as i32)),
                        ShopEffect::WaitingNote => shop.note = true,
                        ShopEffect::Confirm(_) | ShopEffect::Cancel => {}
                    }
                }
                drop(shop);
                self.out(out);
            }
            ConfirmResult::Cancel => {
                shop.tx.cancel();
                shop.tx.menu_state = 3;
            }
            ConfirmResult::Hire => {
                drop(shop);
                if let Some((npc, name)) = c.hire {
                    let mut h = self.hire.borrow_mut();
                    h.hired = true;
                    h.sent = true;
                    drop(h);
                    self.out(vec![PanelOutput::Intent(hire_intent(npc, name))]);
                }
            }
            ConfirmResult::HireList => {
                if let Some((npc, _)) = c.hire {
                    self.hire.borrow_mut().up = Some(npc);
                }
            }
            ConfirmResult::Nothing => {}
        }
        UiResponse::Consumed
    }
}

impl Panel for NpcMenuUi {
    fn id(&self) -> PanelId {
        NPC_MENU_PANEL
    }

    fn rect(&self) -> Rect {
        if self.shop.borrow().confirm.is_some() {
            return self.sh.borrow().config.screen.frame_rect();
        }
        let st = self.st.borrow();
        let talking = st.talk.active;
        if st.bx.is_some() || talking {
            // The whole frame: a press outside the box ends the chat; a
            // press while talking ends the talk.
            st.screen_rect()
        } else {
            Rect::new(0, 0, 0, 0)
        }
    }

    fn draw(&self, ctx: &UiCtx, out: &mut dyn UiDrawSink) {
        let st = self.st.borrow();
        let sh = self.sh.borrow();
        let m = Measure(sh.fonts.as_ref());
        let mut spin = 0;
        let border = sh.tables.files.id(crate::ui::box_border::BOXPIECES_FILE);
        if let Some(bx) = &st.bx {
            push_menu_draws(bx.draw(&mut spin, &m), st.screen_rect(), border, out);
        }
        if let Some(n) = &st.note {
            push_menu_draws(n.bx.draw(&mut spin, &m), st.screen_rect(), border, out);
        }
        if let Some(c) = &self.shop.borrow().confirm {
            push_menu_draws(c.bx.draw(&mut spin, &m), st.screen_rect(), border, out);
        }
        self.draw_talk(&st, &m, border, out);
        let _ = ctx;
    }

    fn hit(&self, p: Point) -> Option<WidgetId> {
        if self.shop.borrow().confirm.is_some() {
            return self.rect().contains(p).then_some(WidgetId(0));
        }
        let st = self.st.borrow();
        let bx = st.bx.as_ref()?;
        ltrb_contains(&bx.rect(), p).then(|| WidgetId(item_at(bx, p).map_or(0, |i| i as u16 + 1)))
    }

    /// `npc.menu(N)`: the menu box's selectable choices (Talk, Trade, ...,
    /// Cancel), N from 0 top down, at the bands [`item_at`] hit-tests;
    /// `npc.topic(N)`: the talk topic box's choices the same way. A shop
    /// confirmation exposes `shop.confirm.yes` / `shop.confirm.no` instead.
    fn ui_elements(&self, _ctx: &UiCtx) -> Vec<UiElement> {
        if let Some(c) = &self.shop.borrow().confirm {
            return super::shop_ui::confirm_elements(&c.bx);
        }
        let st = self.st.borrow();
        let mut out = st
            .bx
            .as_ref()
            .map_or_else(Vec::new, |bx| menu_elements(bx, "menu"));
        if st.talk.active {
            if let Some(bx) = &st.talk.topic {
                out.extend(menu_elements(bx, "topic"));
            }
        }
        out
    }

    fn event(&mut self, e: UiEvent, ctx: &UiCtx) -> UiResponse {
        let esc = matches!(e, UiEvent::Action(a) if [Action::GameMenu, Action::ClearScreen]
            .iter()
            .any(|k| a == ActionId(k.index() as u16)));
        if self.shop.borrow().confirm.is_some() {
            // The confirm box eats the key (the game menu stays shut).
            return if esc {
                UiResponse::Consumed
            } else {
                self.confirm_event(e)
            };
        }
        if esc && (self.shop.borrow().open.is_some() || self.sh.borrow().shop_flag_open()) {
            // With the shop up no NPC menu box exists, so the Esc command's
            // box test is clear and the close-all closes the shop (ui
            // 0x0C); the shop's close then ends the interaction.
            return UiResponse::Ignored;
        }
        if esc && self.hire.borrow().up.is_some() {
            // The hire list's box owns the key while it is up: its p1
            // (Back) closes the list and rebuilds the NPC menu.
            return UiResponse::Ignored;
        }
        let Some(guid) = self.st.borrow().up.as_ref().map(|it| it.guid) else {
            return UiResponse::Ignored;
        };
        if esc {
            if !self.esc_key(guid, ctx) && self.st.borrow().bx.is_some() {
                let o = self.st.borrow_mut().end(guid);
                self.out(o);
            }
            return UiResponse::Consumed;
        }
        if self.st.borrow().bx.is_none() {
            return self.talk_event(e, guid, ctx);
        }
        let (inside, item) = {
            let st = self.st.borrow();
            let Some(bx) = st.bx.as_ref() else {
                return UiResponse::Ignored;
            };
            match e.at() {
                Some(p) => (ltrb_contains(&bx.rect(), p), item_at(bx, p)),
                None => return UiResponse::Ignored,
            }
        };
        match e {
            UiEvent::CursorMoved(_) => {
                if let Some(i) = item {
                    if let Some(bx) = self.st.borrow_mut().bx.as_mut() {
                        bx.selected = i as i32;
                    }
                }
                UiResponse::Ignored
            }
            UiEvent::Press {
                button: PointerButton::Left,
                ..
            } => {
                if inside {
                    self.st.borrow_mut().pressed = item;
                    return UiResponse::Consumed;
                }
                let o = self.st.borrow_mut().end(guid);
                self.out(o);
                UiResponse::Consumed
            }
            UiEvent::Release {
                button: PointerButton::Left,
                ..
            } => {
                let pressed = self.st.borrow_mut().pressed.take();
                match item.filter(|&i| pressed == Some(i)) {
                    Some(i) => self.run(i, ctx),
                    None if inside => UiResponse::Consumed,
                    None => UiResponse::Ignored,
                }
            }
            _ if inside => UiResponse::Consumed,
            _ => UiResponse::Ignored,
        }
    }
}

impl OriginalUi {
    /// Opens the NPC menu for a delivered 0x28 (`msg_ui`):
    /// `SetUIState(8, on)`, the table edits, the anchor; the box
    /// is built at the next [`Self::npc_menu_poll`] (it needs the
    /// strings).
    pub fn open_npc_menu(&mut self, guid: u32, class: u32, level: i32, world: &ClientWorld) {
        let n = unidentified_count(world);
        self.open_npc_menu_with(guid, class, level, n, world);
    }

    /// [`Self::open_npc_menu`] with Cain's count of items to identify.
    pub fn open_npc_menu_with(
        &mut self,
        guid: u32,
        class: u32,
        level: i32,
        identify_n: u32,
        world: &ClientWorld,
    ) {
        // The menu open sets the interact NPC.
        self.more.interact_active = true;
        self.more.interact_npc = guid;
        self.talk_list();
        // A seller's hire list (S→C 0x4F) opens from the Hire option.
        self.hire.borrow_mut().up = None;
        let _ = self.set_ui(u32::from(id::NPC_MENU), 0, false);
        let open_mode = self.open_mode().get();
        let mut st = self.npcm.borrow_mut();
        st.anchor = npc_anchor(world, guid, open_mode);
        st.start(guid, class, level, identify_n);
        // The greeting of the menu open: in mode 2 (return) it gives a
        // sound (`npc_talk` module doc) and C→S 0x4D goes.
        if let Some(mode) = st.talk.intro().menu_open(class) {
            let played = st.talk.intro().greeting_played(mode, class);
            if let Some(i) = played.send_4d {
                st.push_pending(vec![PanelOutput::Intent(i)]);
            }
        }
    }

    /// Per UI frame, before the events: builds an asked-for box with the
    /// frame's strings, queues its messages on the root, rebuilds
    /// the box the hire list's Back asked for and drops the menu
    /// when ui 8 went off.
    pub fn npc_menu_poll(
        &mut self,
        world: &ClientWorld,
        root: &mut UiRoot,
        strings: &dyn StringLookup,
    ) {
        // The interaction's end clears the interact NPC.
        if root.take_chat_end() {
            self.more.interact_active = false;
        }
        let (back, sent) = {
            let mut h = self.hire.borrow_mut();
            (std::mem::take(&mut h.back), std::mem::take(&mut h.sent))
        };
        if back {
            if let Some(it) = self.npcm.borrow_mut().up.as_mut() {
                it.build = true;
            }
        }
        let now = self.shared.borrow().ui_ms32(world.frames);
        if sent {
            // The menu state goes to 10 with the waiting note. The
            // clock is the UI clock (D2RUST's own, as `game_messages`).
            self.npcm.borrow_mut().menu_state = MENU_WAITING;
            self.open_waiting_note(strings, now);
        }
        // A shop send's waiting note.
        if std::mem::take(&mut self.shop.borrow_mut().note) {
            self.open_waiting_note(strings, now);
        }
        // The list closed, kind 5, the confirm dialog
        // at the mouse.
        let hire_confirm = self.hire.borrow_mut().confirm.take();
        if let Some((npc, name)) = hire_confirm {
            let sh = self.shared.borrow();
            let m = Measure(sh.fonts.as_ref());
            let s = string_fn(strings);
            let frame = (sh.config.screen.w, sh.config.screen.h);
            let mouse = (sh.mouse.x, sh.mouse.y);
            if let Ok(bx) = confirm_box(TxKind::Hire, None, mouse, &s, frame, &m) {
                self.shop.borrow_mut().confirm = Some(super::shop_ui::ConfirmUp {
                    bx,
                    kind: TxKind::Hire,
                    facts: Default::default(),
                    now,
                    hire: Some((npc, name)),
                    pressed: None,
                });
            }
        }
        if !self.is_open(id::NPC_MENU) {
            let mut st = self.npcm.borrow_mut();
            if st.up.is_some() {
                st.close();
            }
        }
        {
            let sh = self.shared.borrow();
            let m = Measure(sh.fonts.as_ref());
            let mut st = self.npcm.borrow_mut();
            if let Some(class) = st.up.as_ref().filter(|it| it.build).map(|it| it.class) {
                // The title: a monster's own name (monster
                // data, not in the client model; town NPCs have
                // none) else its class's monstats `NameStr`.
                let name = sh
                    .items
                    .tips
                    .as_ref()
                    .and_then(|t| t.monstats.get(class as usize).copied())
                    .and_then(|id| strings.get_id(id))
                    .map(<[u16]>::to_vec)
                    .unwrap_or_default();
                st.build(world, strings, name, &m);
            }
        }
        self.talk_poll(world, strings);
        let pending = std::mem::take(&mut self.npcm.borrow_mut().pending);
        for o in pending {
            match o {
                PanelOutput::Intent(i) => root.queue_intent(i),
                PanelOutput::SetUi { ui, mode, jump } => {
                    let _ = self.set_ui(u32::from(ui), u32::from(mode), jump);
                }
                PanelOutput::Sound(id) => self
                    .outcome
                    .sounds
                    .push(crate::audio::driver::SoundRequest::Ui(id)),
                PanelOutput::DialogSpeechEnd => self
                    .outcome
                    .sounds
                    .push(crate::audio::driver::SoundRequest::NpcDialogClose),
                PanelOutput::PlayerEvent(event) => {
                    let local = self.shared.borrow().facts.local;
                    if let Some(unit) = local {
                        self.outcome
                            .sounds
                            .push(crate::audio::driver::SoundRequest::PlayerEvent { unit, event });
                    }
                }
            }
        }
        self.sync_root(root);
    }

    /// S→C 0x2A at the UI: in
    /// the waiting state the note closes and, on result 3 or 6, Cain's
    /// counts reset and the menu is rebuilt; result 5 changes nothing
    /// here; any other result sets state 12 (its note caption is open).
    /// Outside it, result 1 plays UI sound 221.
    pub(super) fn npc_transaction(&mut self, bytes: &[u8; 15]) {
        let result = bytes[2];
        let mut st = self.npcm.borrow_mut();
        if st.menu_state != MENU_WAITING {
            drop(st);
            if bytes[2] == 1 {
                self.outcome
                    .sounds
                    .push(crate::audio::driver::SoundRequest::Ui(SOUND_TX_DONE));
            }
            return;
        }
        st.note = None;
        match result {
            5 => {}
            3 | 6 => {
                let class = st.up.as_ref().map_or(0, |it| it.class);
                let menus = st.menus();
                let mut recs = menus.records().to_vec();
                cain_count_reset(&mut recs, class, result);
                *menus = NpcMenus::from_records(recs);
                st.menu_state = 1;
                if let Some(it) = st.up.as_mut() {
                    // The rebuild's identify caption counts again.
                    it.build = true;
                }
            }
            _ => st.menu_state = MENU_FAILED,
        }
    }

    /// The NPC menu (built, asked for, or talking) or the imbue dialog is
    /// up (the preview's automatic chat close waits for them).
    pub fn npc_menu_up(&self) -> bool {
        let st = self.npcm.borrow();
        let menu = st
            .up
            .as_ref()
            .is_some_and(|it| it.build || st.talk.active || st.bx.is_some());
        menu || st.talk.socket.is_some()
    }

    /// An NPC interaction is open, including its B2 greeting before the
    /// menu: the preview's automatic chat close must not end it.
    pub fn npc_interaction_up(&self) -> bool {
        let st = self.npcm.borrow();
        st.up.is_some() || st.talk.menu_after.is_some() || st.talk.socket.is_some()
    }

    /// The open menu (built box or talk), for tests and the host.
    pub fn npc_menu(&self) -> Option<Open> {
        self.npcm.borrow().open_view()
    }

    /// The centre of the selectable row `k` of the built box (tests: the
    /// box sits above the NPC).
    pub fn npc_menu_row_point(&self, k: usize) -> Option<Point> {
        let st = self.npcm.borrow();
        let i = st.row_item(k)?;
        let r = item_rect(st.bx.as_ref()?, i);
        Some(Point::new((r.l + r.r) / 2, (r.t + r.b) / 2))
    }

    /// The built box's rectangle and its items' (text, color), for tests.
    pub fn npc_menu_box(&self) -> Option<(Ltrb, Vec<BoxItem>)> {
        let st = self.npcm.borrow();
        let bx = st.bx.as_ref()?;
        Some((
            bx.rect(),
            bx.items.iter().map(|i| (i.text.clone(), i.color)).collect(),
        ))
    }

    /// The menu state (tests).
    pub fn npc_menu_state(&self) -> u8 {
        self.npcm.borrow().menu_state
    }

    /// The text of the waiting note's item, when one is up.
    pub fn npc_waiting_note(&self) -> bool {
        self.npcm.borrow().note.is_some()
    }

    /// The waiting note at the NPC's anchor (also after 0x36); a second
    /// one is refused.
    pub(super) fn open_waiting_note(&mut self, strings: &dyn StringLookup, now: u32) {
        let sh = self.shared.borrow();
        let m = Measure(sh.fonts.as_ref());
        let mut st = self.npcm.borrow_mut();
        let at = st.anchor.unwrap_or((st.screen.0 / 2, st.screen.1 / 3));
        let text = string_fn(strings)(STR_WAITING);
        if let Ok(n) = WaitingNote::open(st.note.as_ref(), at, &text, now, &m) {
            st.note = Some(n);
        }
    }
}

#[cfg(test)]
#[path = "npc_box_tests.rs"]
mod tests;
