//! Talk and the NPC services in play: the menu's Talk closes the box and
//! opens the talk topic box ([`npc_text::topic_box`]) at the NPC's
//! anchor; a topic plays its text in the dialog panel ([`DialogUi`]: top
//! centre, 325 wide, scrolled by the clock, skipped by a button press);
//! the topic box's cancel is the talk sequence's end (the menu is built
//! again, or the interaction ends). Charsi's imbue is the item-socket
//! dialog ([`SocketDialog`], ui 0x0E, NPC mode) with its own panel.
//!
//! Scripts click the topic box by id (`npc.topic(N)`, listed by
//! [`NpcMenuUi`]) and skip the dialog with `dialog.skip`.
// D2RUST's own choices:
// - the topic captions and the intro table with its text records come
//   from `tables/ui/npc-talk-*.tsv` (`messages::npc_facts`); "gossip"
//   plays the record the gossip index picks with a seed copied from the
//   local player's unit seed at the first use and advanced here only (not
//   written back to the unit), and game quest 12 bit 13 read as clear (no
//   stock effect); "introduction" picks its record by the NPC and the
//   player's class and runs the gossip body; the greeting of the menu
//   open gives a sound in mode 2 (whether the sound exists is the audio
//   layer's), so the C→S 0x4D goes out with it; the plain greeting
//   (mode 0) sends no 0x4D;
// - 0x28's B2 line ends through the dialog's end callback (next lines,
//   C→S 0x31, the mouse skip ending the talk); the interact NPC is taken
//   as set, and a button down always skips (the hovered-unit cases are
//   not modelled);
// - the end callback of an NPC topic opens the topic box again and
//   returns 0; the topic box's cancel is the talk end; "about the
//   merchants" and "Horadric Cube" do nothing;
// - the clock of the scroll and the skip is the client update count at
//   40 ms (the skip's 100 ms gate is not pinned); the item-socket
//   dialog's 400 ms click gate reads the UI clock (`super::ui_clock`); the
//   speech is not waited for (no speech playback yet);
// - the imbue dialog opens from the menu's Imbue row: ui 8 goes off first
//   (ui 8 refuses 0x0E), the cursor item stays on the model's cursor
//   while placed, the NPC accept check is the server's, and the placed
//   item is not drawn.

use d2_sim::rng::Seed;

use super::game_messages::Measure;
use super::npc_box::{push_menu_draws, NpcMenuState, NpcMenuUi, SharedNpcMenu};
use super::{OriginalUi, SharedRef};
use crate::bridge::items;
use crate::bridge::world::{ClientWorld, UnitKey};
use crate::controls::Action;
use crate::ui::draw::{CelLook, UiDrawSink};
use crate::ui::draw::{ImageRef, ImageRequest, RectRequest, TextRequest, TextStyle, UiDraw};
use crate::ui::geom::{Point, Rect};
use crate::ui::messages::dialog::{
    panel_draw, DialogDraw, DialogEffect, DialogOpen, DialogUi, PassInput, SkipEvent, PANEL_FONT,
};
use crate::ui::messages::intro::{GossipCtx, GossipOutcome, IntroTable};
use crate::ui::messages::npc_facts::{caption_pairs, intro_table};
use crate::ui::messages::npc_text::{topic_box, TextList, TopicHandler, TopicInput};
use crate::ui::messages::socket::{
    background_file, CursorItem, DrawEnv, Mode, SocketDialog, SocketDraw, SocketEffect,
    SocketFrame, Step, BUTTON_CEL, UI_SOCKET,
};
use crate::ui::messages::{Metrics, FONT_16};
use crate::ui::panel::{ActionId, WidgetId};
use crate::ui::panel::{ClientIntent, Panel, PanelId, StringLookup, UiCtx, UiEvent, UiResponse};
use crate::ui::panels::menu_box::MenuBox;
use crate::ui::panels::npc::{
    msg_chat_end, msg_quest, next_message, talk_end, TalkEnd, PLAYED_MAX,
};
use crate::ui::panels::PanelOutput;
use crate::ui::states::id;
use crate::ui::text::TextOpts;
use crate::ui::ui_ids::UiElement;
use crate::ui::PointerButton;

/// The item-socket dialog's panel id: its UI state (0x0E).
pub const SOCKET_PANEL: PanelId = PanelId(UI_SOCKET as u16);
/// The intro table's no-introduction classes.
const NO_INTRO: [u32; 5] = crate::ui::messages::intro::NO_INTRO;

/// The talk of the open interaction.
#[derive(Default)]
pub struct TalkState {
    /// The talk topic box.
    pub topic: Option<MenuBox<TopicHandler>>,
    topic_pressed: Option<usize>,
    /// The dialog panel's globals.
    pub dialog: DialogUi,
    /// The NPC text list as the talk builds it.
    pub list: Option<TextList>,
    /// The talk is running: topic box or dialog (the menu box is closed).
    pub active: bool,
    /// The topic box is asked for (built at the next poll, it needs the
    /// strings).
    pub reopen: bool,
    /// The item-socket dialog.
    pub socket: Option<SocketDialog>,
    /// The intro table, loaded from the facts at the
    /// first use.
    intro: Option<IntroTable>,
    /// The local unit seed copied at the first gossip (D2RUST's own).
    seed: Option<Seed>,
    /// 0x28's dialog branch B2 asked for the dialog line `m` of the NPC
    /// `guid`; opened
    /// at the next poll (it needs the strings).
    pub pending_open: Option<(u32, u16)>,
    /// The menu the B2 dialog's end callback opens: NPC
    /// guid, class, level, unidentified count.
    pub menu_after: Option<(u32, u32, i32, u32)>,
    /// The dialog ended: the menu of `menu_after` opens at the next poll.
    pub(super) menu_ready: Option<(u32, u32, i32, u32)>,
    /// The B2 line playing.
    pub cur: u16,
    /// The lines played in this talk, at most
    /// [`PLAYED_MAX`].
    pub played: Vec<u16>,
    /// A speech request never starts playing (a headless run without an
    /// audio device): the dialog pass waits for it.
    pub speech_stalls: bool,
    /// The client update of the last dialog pass: the pass
    /// runs once per update, as the panel is drawn once per update.
    last_pass: Option<u32>,
}

impl TalkState {
    /// Preserve the talk and speech clock while refreshing its hit geometry.
    pub(super) fn resize_screen(&mut self, screen: crate::ui::layout::Screen, m: &dyn Metrics) {
        self.topic_pressed = None;
        let bounds = (screen.w, screen.h);
        let at = (screen.w / 2, screen.h / 3);
        if let Some(bx) = self.topic.as_mut() {
            super::npc_box::reflow_box(bx, bounds, at, m);
        }
        if let Some(socket) = self.socket.as_mut() {
            if let Some(note) = socket.note.as_mut() {
                super::npc_box::reflow_box(note, bounds, at, m);
            }
        }
        self.dialog.mouse_down = false;
        if self.dialog.place1 {
            self.dialog.x = screen.sx();
            self.dialog.y = 261 - screen.sy() + screen.panel_dy();
        } else {
            self.dialog.x = (screen.w - crate::ui::messages::dialog::PANEL_W) / 2;
            self.dialog.y = 12;
        }
    }

    /// The intro table, loaded from the facts on first use.
    pub fn intro(&mut self) -> &mut IntroTable {
        self.intro.get_or_insert_with(intro_table)
    }
}

/// The dialog clock: 40 ms per client update, the `timeGetTime` a pinned
/// run gives the scroll (the skip's `GetTickCount` is taken to be the
/// same; module doc).
fn now(world: &ClientWorld) -> u32 {
    world.drlg_updates.wrapping_mul(40)
}

fn strings_of(s: &dyn StringLookup) -> impl Fn(u16) -> Vec<u16> + '_ {
    move |id| s.get_id(id).map(<[u16]>::to_vec).unwrap_or_default()
}

/// The topic box for the interaction's NPC.
fn build_topic(
    st: &NpcMenuState,
    world: &ClientWorld,
    strings: &dyn StringLookup,
    m: &dyn Metrics,
) -> Option<MenuBox<TopicHandler>> {
    let class = st.npc_class()?;
    let has_cube = items::local_items(world)
        .iter()
        .any(|i| i.code == Some(*b"box ") && !i.store);
    let input = TopicInput {
        list: st.talk.list.as_ref(),
        no_intro: NO_INTRO.contains(&class),
        npc_class: class,
        has_cube,
        caption_table: caption_pairs(),
        box_up: false,
    };
    let s = strings_of(strings);
    topic_box(&input, st.anchor_or_centre(), &s, st.screen, m).ok()
}

/// The text id a topic plays: a replayed quest topic its list
/// entry; "introduction" the record `introduction_index` picks;
/// "gossip" the record the gossip index picks (the first click re-rolls
/// every entry's index).
fn topic_text(
    st: &mut NpcMenuState,
    h: TopicHandler,
    class: u32,
    player: (i32, Option<(u32, u32)>),
    quest: &[u8; 96],
) -> Option<u16> {
    let (player_class, seed) = player;
    match h {
        TopicHandler::Replay(k) => st.talk.list.as_ref()?.nth_of_kind(2, k).map(|e| e.string),
        TopicHandler::Introduction | TopicHandler::Gossip => {
            let talk = &mut st.talk;
            let seed = talk.seed.get_or_insert_with(|| {
                let (lo, hi) = seed.unwrap_or((1, 666));
                Seed::new(lo, hi)
            });
            let gate = |record: u32, bit: u32| {
                u32::from(crate::bridge::objects::quest_bit(
                    quest,
                    record as u8,
                    bit as u8,
                ))
            };
            let ctx = GossipCtx {
                class: player_class.max(0) as u8,
                quest_bit: &gate,
                game_quest12_bit13: false,
            };
            let table = talk.intro.get_or_insert_with(intro_table);
            let i = table.index_of(class)?;
            // "introduction" picks its index and runs the
            // gossip body with it; "gossip" uses the entry's re-rolled +0x0D.
            let index = matches!(h, TopicHandler::Introduction)
                .then(|| table.introduction_index(i, Some(class), player_class));
            match table.click_with_index(i, index, true, seed, &ctx) {
                GossipOutcome::Play { text } => text,
                GossipOutcome::Ended => None,
            }
        }
        _ => None,
    }
}

/// A topic box item's hit band (the menu box's, `npc_box`).
fn topic_item_at(bx: &MenuBox<TopicHandler>, p: Point) -> Option<usize> {
    super::npc_box::item_at(bx, p)
}

impl NpcMenuUi {
    /// Talk: the menu box is closed already; the
    /// topic box opens.
    pub(super) fn talk_open(&mut self, ctx: &UiCtx) {
        let sh = self.sh.borrow();
        let m = Measure(sh.fonts.as_ref());
        let mut st = self.st.borrow_mut();
        st.menu_state = 1;
        st.talk.active = true;
        st.talk.topic = build_topic(&st, ctx.world, ctx.strings, &m);
        st.talk.topic_pressed = None;
    }

    /// Events while the talk runs: the dialog's skip input (a button down skips,
    /// a right button up is only consumed), else the topic box. Esc / Space
    /// (command 56 / 38) during an NPC interaction (the box's Esc entries consume
    /// the key and run p1; the game's Esc command does nothing meanwhile): the
    /// dialog text skips (`SkipEvent::Key`), the topic box cancels (the talk's
    /// end, C→S 0x30), the menu box (npc_box) ends the interaction. Always
    /// consumed, so the game menu does not open. The box ignores the key until it
    /// was drawn more than 4 times (+0x54); that guard is not modelled:
    /// an Esc at draw 49 or 56 sends no C→S 0x30 either way, so the guard is not
    /// observable there; a draw counter would only matter for the
    /// timing of a key within 4 draws of the box.
    pub(super) fn esc_key(&mut self, guid: u32, ctx: &UiCtx) -> bool {
        // The dialog's own skip keys are [`DialogPanelUi`]'s.
        let talk = self.st.borrow().talk.active;
        if talk && self.st.borrow().talk.topic.is_some() {
            self.topic_chosen(TopicHandler::Cancel, guid, ctx);
        } else {
            return false;
        }
        true
    }

    pub(super) fn talk_event(&mut self, e: UiEvent, guid: u32, ctx: &UiCtx) -> UiResponse {
        if !self.st.borrow().talk.active {
            return UiResponse::Ignored;
        }
        let Some(at) = e.at() else {
            return UiResponse::Ignored;
        };
        let item = {
            let st = self.st.borrow();
            let Some(bx) = st.talk.topic.as_ref() else {
                return UiResponse::Ignored;
            };
            topic_item_at(bx, at)
        };
        match e {
            UiEvent::CursorMoved(_) => {
                if let (Some(i), Some(bx)) = (item, self.st.borrow_mut().talk.topic.as_mut()) {
                    bx.selected = i as i32;
                }
                UiResponse::Ignored
            }
            UiEvent::Press {
                button: PointerButton::Left,
                ..
            } => {
                self.st.borrow_mut().talk.topic_pressed = item;
                UiResponse::Consumed
            }
            UiEvent::Release {
                button: PointerButton::Left,
                ..
            } => {
                let pressed = self.st.borrow_mut().talk.topic_pressed.take();
                let Some(i) = item.filter(|&i| pressed == Some(i)) else {
                    return UiResponse::Consumed;
                };
                let h = self
                    .st
                    .borrow()
                    .talk
                    .topic
                    .as_ref()
                    .and_then(|b| b.items[i].handler);
                if let Some(h) = h {
                    self.topic_chosen(h, guid, ctx);
                }
                UiResponse::Consumed
            }
            _ => UiResponse::Consumed,
        }
    }

    fn topic_chosen(&mut self, h: TopicHandler, guid: u32, ctx: &UiCtx) {
        match h {
            TopicHandler::Cancel => self.talk_end(guid, ctx),
            TopicHandler::Introduction | TopicHandler::Gossip | TopicHandler::Replay(_) => {
                let class = self.st.borrow().npc_class().unwrap_or(0);
                let player = ctx
                    .world
                    .local()
                    .map_or((-1, None), |u| (u.class as i32, u.seed));
                let quest = self.sh.borrow().client_quest;
                let id = topic_text(&mut self.st.borrow_mut(), h, class, player, &quest);
                let Some(id) = id else {
                    return;
                };
                let text = strings_of(ctx.strings)(id);
                let (w, h) = self.st.borrow().screen;
                let s = self.sh.borrow().config.screen;
                let effects = {
                    let mut st = self.st.borrow_mut();
                    // The topic box is freed and the text plays with the NPC
                    // talk's end callback.
                    st.talk.topic = None;
                    let fx = st.talk.dialog.open(
                        DialogOpen::Npc {
                            unit: Some(guid),
                            id: u32::from(id),
                        },
                        &text,
                        w,
                        h,
                        s.sx(),
                        s.sy(),
                        now(ctx.world),
                    );
                    st.talk.dialog.end_callback = true;
                    fx
                };
                self.dialog_effects(effects, ctx.world);
            }
            _ => {}
        }
    }

    /// The talk sequence ended with no next message.
    fn talk_end(&mut self, guid: u32, ctx: &UiCtx) {
        let o = self.st.borrow_mut().talk_end(guid, ctx.world);
        self.out_pub(o);
    }

    /// The dialog's effects that reach the rest of the UI.
    pub(super) fn dialog_effects(&mut self, effects: Vec<DialogEffect>, world: &ClientWorld) {
        let o = self.st.borrow_mut().dialog_effects(effects, world);
        self.out_pub(o);
    }

    /// The talk's topic box; the dialog panel is [`DialogPanelUi`]'s.
    pub(super) fn draw_talk(
        &self,
        st: &NpcMenuState,
        m: &dyn Metrics,
        border: Option<u32>,
        out: &mut dyn UiDrawSink,
    ) {
        let mut spin = 0;
        if let Some(bx) = &st.talk.topic {
            push_menu_draws(bx.draw(&mut spin, m), st.screen_rect(), border, out);
        }
    }
}

/// The dialog panel's id: not a UI state, open for good.
pub const DIALOG_PANEL: PanelId = PanelId(0x114);

/// The dialog panel, drawn in the text pass whether or not the NPC menu
/// is up: a talk that opens with a dialog (B2) shows it before the menu
/// exists, and its skip handlers are this panel's events.
pub struct DialogPanelUi {
    pub(super) sh: SharedRef,
    pub(super) st: SharedNpcMenu,
}

impl Panel for DialogPanelUi {
    fn id(&self) -> PanelId {
        DIALOG_PANEL
    }

    /// The whole screen while the skip handlers are registered (they sit
    /// on the game window, ahead of the world's click), else nothing.
    fn rect(&self) -> Rect {
        let st = self.st.borrow();
        if !st.talk.dialog.handlers {
            return Rect::new(0, 0, 0, 0);
        }
        let (w, h) = st.screen;
        Rect::new(0, 0, w.clamp(0, 0xFFFF) as u16, h.clamp(0, 0xFFFF) as u16)
    }

    fn draw(&self, _ctx: &UiCtx, out: &mut dyn UiDrawSink) {
        let st = self.st.borrow();
        let sh = self.sh.borrow();
        let m = Measure(sh.fonts.as_ref());
        let border = sh.tables.files.id(crate::ui::box_border::BOXPIECES_FILE);
        draw_dialog(&st, &m, border, out);
    }

    fn hit(&self, _p: Point) -> Option<WidgetId> {
        None
    }

    /// `dialog.skip` while the skip handlers are registered: a press
    /// anywhere skips, so it is the whole screen, clicked at its centre.
    fn ui_elements(&self, _ctx: &UiCtx) -> Vec<UiElement> {
        if !self.st.borrow().talk.dialog.handlers {
            return Vec::new();
        }
        vec![UiElement::new("dialog.skip", self.rect())]
    }

    /// The skip handlers: a left or right button down skips, a right
    /// button up is only consumed; a left button up has no entry and
    /// passes on; Esc and Space skip. They run whether or not an NPC menu or talk is up:
    /// the dialog a 0x28 B2 opens before the menu (`menu_after`) skips the
    /// same way.
    fn event(&mut self, e: UiEvent, ctx: &UiCtx) -> UiResponse {
        if !self.st.borrow().talk.dialog.handlers {
            return UiResponse::Ignored;
        }
        let ev = match e {
            UiEvent::Press { .. } => SkipEvent::ButtonDown,
            UiEvent::Release {
                button: PointerButton::Right,
                ..
            } => SkipEvent::RightUp,
            UiEvent::Action(a)
                if [Action::GameMenu, Action::ClearScreen]
                    .iter()
                    .any(|k| a == ActionId(k.index() as u16)) =>
            {
                SkipEvent::Key
            }
            _ => return UiResponse::Ignored,
        };
        let s = self.sh.borrow().config.screen;
        let out = {
            let mut st = self.st.borrow_mut();
            // What the end callback returns for a key skip and a mouse
            // skip.
            let cb = [false, true].map(|mouse| st.end_callback_result(true, mouse, ctx.world));
            let o = st.talk.dialog.skip_event(ev, now(ctx.world), cb);
            let consumed = o.consumed;
            // A skip that closes the text (Esc, Space, a click) stops the
            // line it was playing; a next line of the talk starts its own.
            let closes = o
                .effects
                .iter()
                .any(|e| matches!(e, DialogEffect::FadeSpeech | DialogEffect::UnitDialogEnd(_)));
            let mut out = if closes {
                vec![PanelOutput::DialogSpeechEnd]
            } else {
                Vec::new()
            };
            out.extend(st.dialog_effects(o.effects, ctx.world));
            out.extend(st.open_pending(ctx.strings, s.sx(), s.sy(), ctx.world));
            (consumed, out)
        };
        self.sh.borrow_mut().outputs.extend(out.1);
        if out.0 {
            UiResponse::Consumed
        } else {
            UiResponse::Ignored
        }
    }
}

/// The dialog panel's draws.
fn draw_dialog(st: &NpcMenuState, m: &dyn Metrics, border: Option<u32>, out: &mut dyn UiDrawSink) {
    let d = &st.talk.dialog;
    let Some(p) = d.panel.as_ref() else {
        return;
    };
    let font_h = m.font_height(PANEL_FONT);
    let clip = st.screen_rect();
    for dr in panel_draw(d.x, d.y, !d.place1, p.scroll.p, &p.lines, font_h) {
        match dr {
            DialogDraw::Backing(r) => {
                out.push(UiDraw::Rect(RectRequest::sized(
                    r.x, r.y, r.w, r.h, r.color, r.mode,
                )));
            }
            DialogDraw::Border { l, t, r, b } => {
                if let Some(file) = border {
                    crate::ui::box_border::push_border(file, (l, t, r, b), clip, out);
                }
            }
            DialogDraw::Line(l, _) => out.push(UiDraw::Text(TextRequest {
                text: l.text,
                at: Point::new(l.x, l.y),
                style: TextStyle {
                    font: PANEL_FONT,
                    color: 0,
                },
                opts: TextOpts::default(),
                clip,
            })),
            DialogDraw::Window {
                line,
                x,
                y,
                skip,
                lines,
            } => out.push(UiDraw::Text(TextRequest {
                text: p.lines[line].clone(),
                at: Point::new(x, y),
                style: TextStyle {
                    font: PANEL_FONT,
                    color: 0,
                },
                opts: TextOpts::Vertical { skip, lines },
                clip,
            })),
        }
    }
}

impl NpcMenuState {
    /// Rebuild the menu or end the interaction.
    pub(super) fn talk_end(&mut self, guid: u32, world: &ClientWorld) -> Vec<PanelOutput> {
        let class = self.npc_class().unwrap_or(0);
        let flag = self.record_flag();
        let present = world.units.contains_key(&UnitKey::new(1, guid));
        self.talk.topic = None;
        self.talk.active = false;
        match talk_end(present, class, flag) {
            TalkEnd::RebuildMenu => {
                self.menu_state = 1;
                self.ask_build();
                Vec::new()
            }
            TalkEnd::EndInteraction => self.end_interaction(guid),
        }
    }

    /// The kind-0 entries of the text list in list order.
    fn kind0(&self) -> Vec<u16> {
        self.talk.list.as_ref().map_or_else(Vec::new, |l| {
            l.entries
                .iter()
                .filter(|e| e.kind == 0)
                .map(|e| e.string)
                .collect()
        })
    }

    /// The B2 talk's next line.
    fn next_line(&self) -> u16 {
        next_message(self.talk.cur, self.kind0(), &self.talk.played)
    }

    /// What the B2 dialog's end callback returns: 1 when it plays
    /// the next line or rebuilds the menu. `skip` is EDX, `mouse` the stack
    /// argument. The topic talk's callback is D2RUST's own (module doc) and
    /// returns 0. The B2 line is the greeting before the menu, so its end
    /// rebuilds the menu for every class: the classes whose talk-topic end
    /// ends the interaction keep that rule for the topic, not the greeting.
    pub(super) fn end_callback_result(
        &mut self,
        skip: bool,
        mouse: bool,
        world: &ClientWorld,
    ) -> u32 {
        let Some((guid, ..)) = self.talk.menu_after else {
            return 0;
        };
        let present = world.units.contains_key(&UnitKey::new(1, guid));
        if self.next_line() != 0xFFFF && present {
            return 1;
        }
        u32::from(present && (!skip || !mouse))
    }

    /// Applies the dialog's effects: its sends; the end callback (the
    /// B2 chain of next lines, or the topic box opened again, module doc).
    pub(super) fn dialog_effects(
        &mut self,
        effects: Vec<DialogEffect>,
        world: &ClientWorld,
    ) -> Vec<PanelOutput> {
        let mut out = Vec::new();
        for e in effects {
            match e {
                DialogEffect::Send(i) => out.push(PanelOutput::Intent(i)),
                DialogEffect::EndCallback { skip, mouse } => {
                    self.talk.dialog.end_callback = false;
                    self.talk.dialog.panel = None;
                    match self.talk.menu_after {
                        Some(after) => out.extend(self.b2_end(after, skip, mouse, world)),
                        None => {
                            if self.talk.active {
                                self.talk.reopen = true;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// For the B2 dialog: the next
    /// unplayed line opens (C→S 0x31), else the menu opens or the
    /// interaction ends; a mouse skip never opens the menu.
    fn b2_end(
        &mut self,
        after: (u32, u32, i32, u32),
        skip: bool,
        mouse: bool,
        world: &ClientWorld,
    ) -> Vec<PanelOutput> {
        let (guid, ..) = after;
        let present = world.units.contains_key(&UnitKey::new(1, guid));
        let next = self.next_line();
        if next != 0xFFFF && present {
            // Right after this step
            // ([`Self::open_pending`]).
            self.talk.pending_open = Some((guid, next));
            if self.talk.played.len() < PLAYED_MAX {
                self.talk.played.push(next);
            }
            self.talk.cur = next;
            return vec![PanelOutput::Intent(ClientIntent(
                msg_quest(guid, next).to_vec(),
            ))];
        }
        self.talk.menu_after = None;
        self.talk.cur = 0xFFFF;
        // The B2 line is the greeting before the menu, so its end opens the
        // menu for every class; the classes whose talk-topic end ends the
        // interaction keep that rule for the topic only.
        if present && (!skip || !mouse) {
            self.talk.menu_ready = Some(after);
            Vec::new()
        } else {
            let g = self.interaction_guid().unwrap_or(guid);
            self.end_interaction(g)
        }
    }

    /// Opens the dialog line asked for (`pending_open`): B2's first line
    /// and each next line of the B2 chain, with the end callback.
    pub(super) fn open_pending(
        &mut self,
        strings: &dyn StringLookup,
        sx: i32,
        sy: i32,
        world: &ClientWorld,
    ) -> Vec<PanelOutput> {
        let Some((guid, m)) = self.talk.pending_open.take() else {
            return Vec::new();
        };
        let text = strings_of(strings)(m);
        let (w, h) = self.screen;
        let fx = self.talk.dialog.open(
            DialogOpen::Npc {
                unit: Some(guid),
                id: u32::from(m),
            },
            &text,
            w,
            h,
            sx,
            sy,
            now(world),
        );
        self.talk.dialog.end_callback = true;
        self.dialog_effects(fx, world)
    }
}

impl OriginalUi {
    /// The talk's frame step (from [`Self::npc_menu_poll`]): the dialog
    /// pass and a topic box asked for again.
    pub(super) fn talk_poll(&mut self, world: &ClientWorld, strings: &dyn StringLookup) {
        let ready = self.npcm.borrow_mut().talk.menu_ready.take();
        if let Some((guid, class, level, n)) = ready {
            self.open_npc_menu_with(guid, class, level, n, world);
        }
        let s = self.shared.borrow().config.screen;
        let out = self
            .npcm
            .borrow_mut()
            .open_pending(strings, s.sx(), s.sy(), world);
        self.npcm.borrow_mut().push_pending(out);
        let pending = {
            let mut st = self.npcm.borrow_mut();
            let mut out = Vec::new();
            if st.talk.dialog.up && st.talk.last_pass != Some(world.drlg_updates) {
                st.talk.last_pass = Some(world.drlg_updates);
                let end_callback_result = st.end_callback_result(false, false, world);
                let speech_waiting = st.talk.speech_stalls && st.talk.dialog.speech_pending;
                let mode12 = st.talk.dialog.unit_dialog.is_some_and(|g| {
                    world
                        .units
                        .get(&UnitKey::new(1, g))
                        .is_some_and(|u| u.mode == crate::bridge::modes::monster_mode::DEAD)
                });
                let r = st.talk.dialog.pass(&PassInput {
                    now: now(world),
                    speech_waiting,
                    end_callback_result,
                    timed_box_drawn: false,
                    // No local player, or its mode is 0x11 (dead, not the
                    // death animation of mode 0).
                    local_dead_or_absent: world
                        .local()
                        .is_none_or(|u| u.mode == crate::bridge::modes::player_mode::DEAD),
                    // The dialog's unit looked up as a monster (type 1);
                    // mode 12 is its death mode (`monster_mode::DEAD`).
                    unit_in_mode12: mode12,
                });
                // The dialog is destroyed (the player or the NPC died): its
                // line stops with it.
                if r.effects.contains(&DialogEffect::Run4a0880) {
                    out.push(PanelOutput::DialogSpeechEnd);
                }
                out.extend(st.dialog_effects(r.effects, world));
                // The next line opens inside the callback, in this pass.
                out.extend(st.open_pending(strings, s.sx(), s.sy(), world));
            }
            out
        };
        self.npcm.borrow_mut().push_pending(pending);
        let reopen = std::mem::take(&mut self.npcm.borrow_mut().talk.reopen);
        if reopen {
            let sh = self.shared.borrow();
            let m = Measure(sh.fonts.as_ref());
            let mut st = self.npcm.borrow_mut();
            st.talk.topic = build_topic(&st, world, strings, &m);
        }
    }

    /// Headless run without audio: speech requests never play.
    pub fn set_speech_stalls(&mut self, on: bool) {
        self.npcm.borrow_mut().talk.speech_stalls = on;
    }

    /// 0x27 type 1 built a new text list, or the list was freed.
    pub(super) fn talk_list(&mut self) {
        let list = self.npc_text().and_then(|l| TextList::build(l).ok());
        self.npcm.borrow_mut().talk.list = list;
    }

    /// A delivered S→C 0x58 (the item-socket result): codes other than 0 reach the open
    /// dialog; its effects leave at the next poll.
    pub(super) fn imbue_output(&mut self, o: &crate::bridge::output::Output) {
        let crate::bridge::output::Output::OpenUi {
            guid: msg_guid,
            code,
            arg,
        } = *o
        else {
            return;
        };
        let mut st = self.npcm.borrow_mut();
        let Some(d) = st.talk.socket.as_mut() else {
            // Codes 6 and 7 run the socket dialog's close (nothing
            // open here) and then always end the NPC interaction
            // (C→S 0x30 for the interact
            // unit), dialog or not: after the 0x58 result 6 of a scripted imbue the
            // client sends `30 01000000 06000000`.
            if matches!(code, 6 | 7) {
                let npc = st.interaction_guid().unwrap_or(msg_guid);
                let out = socket_effects(vec![SocketEffect::EndInteractionFull], npc);
                st.push_pending(out);
            }
            return;
        };
        let guid = d.npc_guid;
        let fx = d.code(code, u32::from(arg));
        let out = socket_effects(fx, guid);
        if st
            .talk
            .socket
            .as_ref()
            .is_some_and(|d| d.step == Step::Closed)
        {
            st.talk.socket = None;
        }
        st.push_pending(out);
    }

    /// Places `guid` in the open item-socket dialog (headless-test seam:
    /// the fixture's model has no item stream for the cursor item).
    pub fn imbue_place(&mut self, guid: u32) {
        if let Some(d) = self.npcm.borrow_mut().talk.socket.as_mut() {
            d.step = Step::Placed;
            d.placed = guid;
        }
    }

    /// The talk's topic box items' text, when one is up (tests).
    pub fn npc_topics(&self) -> Option<Vec<Vec<u16>>> {
        let st = self.npcm.borrow();
        let bx = st.talk.topic.as_ref()?;
        Some(bx.items.iter().map(|i| i.text.clone()).collect())
    }

    /// The centre of the topic box's selectable item `k` (tests).
    pub fn npc_topic_point(&self, k: usize) -> Option<Point> {
        let st = self.npcm.borrow();
        let bx = st.talk.topic.as_ref()?;
        let i = (0..bx.items.len())
            .filter(|&i| bx.items[i].selectable)
            .nth(k)?;
        let r = super::npc_box::item_rect(bx, i);
        Some(Point::new((r.l + r.r) / 2, (r.t + r.b) / 2))
    }

    /// The centre of the topic box's cancel (its last selectable item),
    /// when one is up (tests).
    pub fn npc_topic_cancel_point(&self) -> Option<Point> {
        let n = {
            let st = self.npcm.borrow();
            let bx = st.talk.topic.as_ref()?;
            bx.items.iter().filter(|i| i.selectable).count()
        };
        self.npc_topic_point(n.checked_sub(1)?)
    }

    /// The dialog panel's lines, when it is up (tests).
    pub fn npc_dialog_lines(&self) -> Option<Vec<Vec<u16>>> {
        let st = self.npcm.borrow();
        Some(st.talk.dialog.panel.as_ref()?.lines.clone())
    }

    /// The item-socket dialog's step, when it is up (tests).
    pub fn socket_step(&self) -> Option<Step> {
        Some(self.npcm.borrow().talk.socket.as_ref()?.step)
    }
}

/// The socket dialog's effects for the UI (`msg_u32s`, sounds, states).
fn socket_effects(fx: Vec<SocketEffect>, npc: u32) -> Vec<PanelOutput> {
    let mut out = Vec::new();
    for e in fx {
        match e {
            SocketEffect::Send(i) => out.push(PanelOutput::Intent(i)),
            SocketEffect::Sound(id) => out.push(PanelOutput::Sound(id as i32)),
            SocketEffect::Ui(o) => out.push(o),
            // The interaction ends.
            SocketEffect::EndInteraction | SocketEffect::EndInteractionFull => {
                out.push(PanelOutput::Intent(ClientIntent(
                    msg_chat_end(npc).to_vec(),
                )));
                out.push(PanelOutput::SetUi {
                    ui: id::NPC_MENU,
                    mode: 1,
                    jump: false,
                });
            }
            _ => {}
        }
    }
    out
}

/// Opens the item-socket dialog for the NPC (module doc: the Imbue row):
/// ui 8 off, then the dialog opens with the open accepted.
pub(super) fn open_imbue(
    st: &mut NpcMenuState,
    guid: u32,
    class: u32,
    world: &ClientWorld,
    now: u32,
) -> Vec<PanelOutput> {
    let mut d = SocketDialog::new();
    let player = world.local().map(|u| u.key.guid);
    let fx = d.open(Mode::Npc, 0, (guid, class), player, true, now);
    st.talk.socket = Some(d);
    let mut out = vec![PanelOutput::SetUi {
        ui: id::NPC_MENU,
        mode: 1,
        jump: false,
    }];
    out.extend(socket_effects(fx, guid));
    out
}

/// The files the socket dialog draws.
pub(super) fn socket_files() -> [String; 3] {
    [
        background_file(Mode::Npc).to_string(),
        background_file(Mode::Object).to_string(),
        BUTTON_CEL.to_string(),
    ]
}

/// The item-socket dialog's panel (ui 0x0E): the left half above the control
/// panel; the inventory keeps the right half (the inventory handler sees a
/// click first).
pub struct SocketUi {
    pub(super) sh: SharedRef,
    pub(super) st: SharedNpcMenu,
}

impl SocketUi {
    fn apply(&self, fx: Vec<SocketEffect>) {
        let mut st = self.st.borrow_mut();
        let guid = st.talk.socket.as_ref().map_or(0, |d| d.npc_guid);
        let out = socket_effects(fx, guid);
        if st
            .talk
            .socket
            .as_ref()
            .is_some_and(|d| d.step == Step::Closed)
        {
            st.talk.socket = None;
        }
        drop(st);
        self.sh.borrow_mut().outputs.extend(out);
    }
}

impl Panel for SocketUi {
    fn id(&self) -> PanelId {
        SOCKET_PANEL
    }

    /// The dialog is laid out from the screen's left edge; in a wider
    /// frame it stays at the frame's first column (`ox`, box space).
    fn rect(&self) -> Rect {
        let s = self.sh.borrow().config.screen;
        Rect::new(s.frame_left(), 0, 320, (s.h - 48) as u16)
    }

    fn draw(&self, ctx: &UiCtx, out: &mut dyn UiDrawSink) {
        let sh = self.sh.borrow();
        let ox = sh.config.screen.frame_left();
        let mut out = crate::ui::draw::ShiftSink(out, ox);
        let out = &mut out;
        let m = Measure(sh.fonts.as_ref());
        let strings = strings_of(ctx.strings);
        let frame = {
            let mut st = self.st.borrow_mut();
            let Some(d) = st.talk.socket.as_mut() else {
                return;
            };
            let env = DrawEnv {
                w: sh.config.screen.w,
                h: sh.config.screen.h,
                mouse: (sh.mouse.x - ox, sh.mouse.y),
                player_dead_or_absent: ctx.world.local().is_none_or(|u| u.is_dead()),
                item_size: None,
                strings: &strings,
            };
            d.draw(&env, &m)
        };
        let draws = match frame {
            Ok(SocketFrame::Draw(d)) => d,
            Ok(SocketFrame::Close(fx)) => {
                drop(sh);
                self.apply(fx);
                return;
            }
            Err(_) => return,
        };
        let file = |n: &str| sh.tables.files.id(n);
        let clip = sh.config.screen.rect();
        for d in draws {
            match d {
                SocketDraw::Background { mode, x, y } => {
                    if let Some(f) = file(background_file(mode)) {
                        out.push(cel(f, 0, x, y, clip));
                    }
                }
                SocketDraw::Button { frame, x, y } => {
                    if let Some(f) = file(BUTTON_CEL) {
                        out.push(cel(f, frame, x, y, clip));
                    }
                }
                SocketDraw::HoverBox(r) => out.push(UiDraw::Rect(RectRequest::sized(
                    r.x, r.y, r.w, r.h, r.color, r.mode,
                ))),
                SocketDraw::Text(l) => out.push(UiDraw::Text(TextRequest {
                    text: l.text,
                    at: Point::new(l.x, l.y),
                    style: TextStyle {
                        font: FONT_16,
                        color: u16::try_from(l.color).unwrap_or(0),
                    },
                    opts: TextOpts::default(),
                    clip,
                })),
                // The placed item's cel (module doc: not drawn).
                SocketDraw::Item { .. } => {}
            }
        }
    }

    fn hit(&self, p: Point) -> Option<WidgetId> {
        self.rect().contains(p).then_some(WidgetId(0))
    }

    fn event(&mut self, e: UiEvent, ctx: &UiCtx) -> UiResponse {
        if self.st.borrow().talk.socket.is_none() {
            return UiResponse::Ignored;
        }
        let player = ctx.world.local().map(|u| u.key.guid);
        let e = e.shifted(-self.sh.borrow().config.screen.frame_left());
        let fx = match e {
            UiEvent::Press {
                button: PointerButton::Left,
                at,
            } => {
                let cursor = items::cursor_item(ctx.world).map(|c| CursorItem {
                    guid: c.key.guid,
                    // The server checks the type and the NPC's accept
                    // (module doc).
                    type_ok: true,
                    npc_accepts: true,
                    is_staff: c.code == Some(*b"hst "),
                });
                // The socket dialog's clock: the UI clock, so the 400 ms
                // click gate holds at any frame rate.
                let now = self.sh.borrow().ui_ms32(ctx.tick);
                let mut st = self.st.borrow_mut();
                let d = st.talk.socket.as_mut().expect("checked above");
                d.left_down(now, (at.x, at.y), false, cursor).0
            }
            UiEvent::Release {
                button: PointerButton::Left,
                at,
            } => {
                let sh = self.sh.borrow();
                let m = Measure(sh.fonts.as_ref());
                let strings = strings_of(ctx.strings);
                let mut st = self.st.borrow_mut();
                let d = st.talk.socket.as_mut().expect("checked above");
                d.left_up((at.x, at.y), player, &strings, &m)
            }
            UiEvent::Press { .. } | UiEvent::Release { .. } => Vec::new(),
            _ => return UiResponse::Ignored,
        };
        self.apply(fx);
        UiResponse::Consumed
    }
}

fn cel(file: u32, frame: u32, x: i32, y: i32, clip: Rect) -> UiDraw {
    UiDraw::Image(ImageRequest {
        image: ImageRef { file, frame },
        at: Point::new(x, y),
        clip,
        look: CelLook::PLAIN,
        call: crate::ui::draw::CelCall::Draw,
    })
}

#[cfg(test)]
#[path = "npc_talk_tests.rs"]
mod tests;
