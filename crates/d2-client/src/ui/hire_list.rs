//! The hire list of Kashya (and the other sellers): the offers the server
//! sends (S→C 0x4F resets the list, each 0x4E adds a slot), drawn as the
//! menu box, and the row choice that sends C→S 0x36.
//!
//! Seam to the NPC menu box (stitch-npc): its "Hire" option calls
//! [`OriginalUi::open_hire_list`](super::original::OriginalUi) with the
//! NPC's GUID; everything else (offers, gold line, rows, back, choose)
//! lives here.
//!
//! Preview fills (`// D2RUST's own, unverified`): the row's Life and Def
//! fields are 0 (no `hireling` stat source in the client), the box uses
//! plain text rows without the list widget's scroll, and the choose goes
//! straight to the 0x36 send (the confirm dialog is not built).

use std::cell::RefCell;
use std::rc::Rc;

use super::draw::{TextRequest, TextStyle, UiDraw, UiDrawSink};
use super::geom::{Point, Rect};
use super::messages::msg_u32s;
use super::messages::Metrics;
use super::original::game_messages::Measure;
use super::original::npc_box::push_menu_draws;
use super::original::{FontMeasure, OriginalUi};
use super::panel::{ActionId, ClientIntent, Panel, UiEvent};
use super::panel::{PanelId, UiCtx, UiResponse, WidgetId};
use super::panels::npc_menu::{
    hire_box, hire_choose, hire_geometry, hire_row_text, ChooseFacts, HireAction, HireHandler,
    HireStats, HIRE_BOX, HIRE_LIST,
};
use super::text::TextOpts;
use super::PointerButton;
use crate::bridge::world::{ClientWorld, KindData};
use crate::controls::Action;

/// The hire list panel id (one past the border panel).
pub const HIRE_PANEL: PanelId = PanelId(0x102);
/// Offers a list holds (10 records).
pub const MAX_OFFERS: usize = 10;
/// Row height of the list (item heights 21 / 15).
const ROW_H: i32 = 15;
/// "There are no Mercenaries left to hire."
const STR_NO_MERCS: u16 = 3365;

/// One offer: name id and slot seed (S→C 0x4E).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Offer {
    pub name: u16,
    pub seed: u32,
}

/// The stats of an offer for the row text: `(name, seed, player level)`.
pub type StatsFn = dyn Fn(u16, u32, u32) -> Option<HireStats>;

/// The list state, shared between [`OriginalUi`](super::original::OriginalUi)
/// (fed by the bridge outputs) and the panel.
pub struct HireState {
    pub offers: Vec<Offer>,
    /// The list is up for this NPC GUID.
    pub up: Option<u32>,
    stats: Option<Box<StatsFn>>,
    /// The list closed by a hire: the reset the server sends right after
    /// it does not open the list again.
    pub hired: bool,
    /// Back was chosen: the NPC menu is
    /// rebuilt at the next poll.
    pub back: bool,
    /// A C→S 0x36 went out: the menu state:= 10 and the waiting note opens at
    /// the next poll.
    pub sent: bool,
    /// A choose asked for the confirm dialog: (NPC
    /// GUID, record name), opened at the next poll.
    pub confirm: Option<(u32, u16)>,
    /// The hireling state (S→C 0x9B; 0xFFFF: no dead mercenary).
    pub merc_state: u16,
    pub screen: (i32, i32),
    /// The fonts the box measures (set at install).
    pub fonts: Option<FontMeasure>,
}

impl Default for HireState {
    fn default() -> Self {
        Self {
            offers: Vec::new(),
            up: None,
            stats: None,
            hired: false,
            back: false,
            sent: false,
            confirm: None,
            merc_state: 0xFFFF,
            screen: (0, 0),
            fonts: None,
        }
    }
}

impl HireState {
    /// The whole screen: the clip of every draw.
    fn screen_rect(&self) -> Rect {
        Rect::new(0, 0, self.screen.0 as u16, self.screen.1 as u16)
    }

    /// S→C 0x4F: the list is reset.
    pub fn reset(&mut self) {
        self.offers.clear();
    }

    /// S→C 0x4E: one more offer (more than ten are dropped).
    pub fn offer(&mut self, name: u16, seed: u32) {
        if self.offers.len() < MAX_OFFERS {
            self.offers.push(Offer { name, seed });
        }
    }

    pub fn set_stats(&mut self, f: Box<StatsFn>) {
        self.stats = Some(f);
    }
}

pub type SharedHire = Rc<RefCell<HireState>>;

/// The panel.
pub struct HireListUi {
    pub st: SharedHire,
    /// The `menu\boxpieces` file id of the box border (`None`: not loaded).
    pub border: Option<u32>,
}

fn local_gold(world: &ClientWorld) -> i32 {
    // Stats 14 (gold) + 15 (stash gold).
    world.local().map_or(0, |u| {
        world
            .base(u.key, 14, 0)
            .saturating_add(world.base(u.key, 15, 0))
    })
}

fn local_level(world: &ClientWorld) -> u32 {
    world
        .local()
        .map_or(1, |u| u32::try_from(world.base(u.key, 12, 0)).unwrap_or(1))
}

fn is_player(world: &ClientWorld) -> bool {
    world
        .local()
        .is_some_and(|u| matches!(u.kind, KindData::Player(_)))
}

impl HireListUi {
    fn box_rect(&self) -> Rect {
        let st = self.st.borrow();
        if st.up.is_none() {
            return Rect::new(0, 0, 0, 0);
        }
        let (pos, _) = hire_geometry(st.screen.0, st.screen.1);
        Rect::new(pos.0, pos.1, HIRE_BOX.0 as u16, HIRE_BOX.1 as u16)
    }

    /// The row index at `p`, or `Some(None)` for the Back item. The list
    /// widget (a child of the box) takes the point first; the Back
    /// item's band is the menu box's (`npc_box`, D2RUST's own hit band).
    fn row_at(&self, p: Point) -> Option<Option<usize>> {
        let st = self.st.borrow();
        let (pos, list) = hire_geometry(st.screen.0, st.screen.1);
        // Row i's text sits on y = list y + 15 (i + 1): its band is the 15
        // pixels above (D2RUST's own).
        let k = (p.y - list.1 - 1) / ROW_H;
        let in_list = p.x >= list.0 && p.x < list.0 + HIRE_LIST.0 && p.y > list.1;
        if in_list && (k as usize) < st.offers.len() {
            return Some(Some(k as usize));
        }
        let bx = hire_box(st.screen.0, st.screen.1, 0, &|_| Vec::new(), &NoMeasure).ok()?;
        let back = bx
            .items
            .iter()
            .position(|i| i.handler == Some(HireHandler::Back))?;
        let top = pos.1 + bx.items[..back].iter().map(|i| i.height).sum::<i32>();
        (p.y > top && p.y <= top + bx.items[back].height).then_some(None)
    }
}

/// Widths of 0: the hire box is fixed size (p5 = 0), so only the item x
/// offsets read the metrics.
struct NoMeasure;

impl Metrics for NoMeasure {
    fn wrap(&self, _: u16, t: &[u16], _: i32) -> Vec<Vec<u16>> {
        vec![t.to_vec()]
    }
    fn width_a(&self, _: u16, _: &[u16]) -> i32 {
        0
    }
    fn width_c(&self, _: u16, _: &[u16]) -> i32 {
        0
    }
    fn font_height(&self, _: u16) -> i32 {
        16
    }
}

impl Panel for HireListUi {
    fn ui_elements(&self, _ctx: &UiCtx) -> Vec<crate::ui::ui_ids::UiElement> {
        let st = self.st.borrow();
        if st.up.is_none() {
            return Vec::new();
        }
        let (_, list) = hire_geometry(st.screen.0, st.screen.1);
        (0..st.offers.len())
            .map(|row| {
                crate::ui::ui_ids::UiElement::new(
                    format!("hire.offer({row})"),
                    Rect::new(
                        list.0,
                        list.1 + 1 + ROW_H * row as i32,
                        HIRE_LIST.0 as u16,
                        (ROW_H - 1) as u16,
                    ),
                )
            })
            .collect()
    }

    fn id(&self) -> PanelId {
        HIRE_PANEL
    }

    fn rect(&self) -> Rect {
        self.box_rect()
    }

    fn draw(&self, ctx: &UiCtx, out: &mut dyn UiDrawSink) {
        let st = self.st.borrow();
        if st.up.is_none() || !is_player(ctx.world) {
            return;
        }
        let strings = |id: u16| {
            ctx.strings
                .get_id(id)
                .map(<[u16]>::to_vec)
                .unwrap_or_default()
        };
        let (_, list) = hire_geometry(st.screen.0, st.screen.1);
        let text = |text: Vec<u16>, at: Point, color: u16| {
            UiDraw::Text(TextRequest {
                text,
                at,
                style: TextStyle { font: 1, color },
                opts: TextOpts::default(),
                clip: st.screen_rect(),
            })
        };
        // The box (gold line, Back) is the spec menu box.
        let m = Measure(st.fonts.as_ref());
        if let Ok(bx) = hire_box(
            st.screen.0,
            st.screen.1,
            local_gold(ctx.world),
            &strings,
            &m,
        ) {
            let mut spin = 0;
            push_menu_draws(bx.draw(&mut spin, &m), st.screen_rect(), self.border, out);
        }
        // The list widget's rows (its scroll and columns are not
        // drawn: D2RUST's own).
        let level = local_level(ctx.world);
        if st.offers.is_empty() {
            out.push(text(
                strings(STR_NO_MERCS),
                Point::new(list.0 + 10, list.1 + ROW_H),
                0,
            ));
        }
        for (i, o) in st.offers.iter().enumerate() {
            let Some(stats) = st.stats.as_ref().and_then(|f| f(o.name, o.seed, level)) else {
                continue;
            };
            let (left, _right) = hire_row_text(o.name, &stats, None, &strings);
            out.push(text(
                left,
                Point::new(list.0 + 10, list.1 + ROW_H * (i as i32 + 1)),
                0,
            ));
        }
    }

    fn hit(&self, p: Point) -> Option<WidgetId> {
        self.box_rect().contains(p).then_some(WidgetId(0))
    }

    fn event(&mut self, e: UiEvent, ctx: &UiCtx) -> UiResponse {
        let up = self.st.borrow().up;
        let Some(npc) = up else {
            return UiResponse::Ignored;
        };
        // The box's p1 (Back) is Esc and Space: the list closes and the
        // NPC menu is built again, as the Back row does.
        if let UiEvent::Action(a) = e {
            let esc = [Action::GameMenu, Action::ClearScreen]
                .iter()
                .any(|k| a == ActionId(k.index() as u16));
            if esc {
                let mut st = self.st.borrow_mut();
                st.up = None;
                st.back = true;
                return UiResponse::Consumed;
            }
            return UiResponse::Ignored;
        }
        let UiEvent::Release {
            button: PointerButton::Left,
            at,
        } = e
        else {
            // The box takes every press inside it.
            return match e {
                UiEvent::Press {
                    button: PointerButton::Left,
                    ..
                } => UiResponse::Consumed,
                _ => UiResponse::Ignored,
            };
        };
        let Some(hit) = self.row_at(at) else {
            return UiResponse::Consumed;
        };
        let Some(row) = hit else {
            // Back: close both, rebuild the NPC menu.
            let mut st = self.st.borrow_mut();
            st.up = None;
            st.back = true;
            return UiResponse::Consumed;
        };
        let offer = self.st.borrow().offers.get(row).copied();
        let Some(offer) = offer else {
            return UiResponse::Consumed;
        };
        let expansion = ctx.world.expansion != 0;
        // = −1 (no hireling record of the
        // player); the merc state reads 0xFFFF only
        // (the hireling lookup is not in the client model: D2RUST's own).
        let me = ctx.world.local().map(|u| u.key);
        let c = hire_choose(&ChooseFacts {
            row,
            item_value: row,
            npc_guid: npc,
            record_name: offer.name,
            no_hireling: ctx.world.hireling_guid(me) == u32::MAX,
            classic_game: !expansion,
            merc_state_clear: self.st.borrow().merc_state == 0xFFFF,
        });
        match c.action {
            HireAction::Hire { send } => {
                {
                    let mut st = self.st.borrow_mut();
                    st.up = None;
                    st.hired = true;
                    st.sent = true;
                }
                match send {
                    super::panels::PanelOutput::Intent(i) => UiResponse::Intent(i),
                    _ => UiResponse::Consumed,
                }
            }
            HireAction::Confirm => {
                let mut st = self.st.borrow_mut();
                st.up = None;
                st.confirm = Some((npc, offer.name));
                UiResponse::Consumed
            }
            HireAction::None => UiResponse::Consumed,
        }
    }
}

/// C→S 0x36 for tests that skip the panel.
pub fn hire_intent(npc: u32, name: u16) -> ClientIntent {
    msg_u32s(0x36, &[npc, u32::from(name)])
}

impl OriginalUi {
    /// The NPC menu's "Hire" option: the hire list opens
    /// for the NPC with this GUID. Seam for the NPC menu box (stitch-npc):
    /// call it from the Hire option's handler; the offers arrive as
    /// 0x4F / 0x4E.
    pub fn open_hire_list(&mut self, npc_guid: u32) {
        self.hire.borrow_mut().up = Some(npc_guid);
    }

    /// Row stats for the hire list (`(name, seed, player level)`).
    pub fn set_hire_stats(&mut self, f: Box<StatsFn>) {
        self.hire.borrow_mut().set_stats(f);
    }

    /// The hire list state (offers, whether it is up).
    pub fn hire_list(&self) -> std::cell::Ref<'_, HireState> {
        self.hire.borrow()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::output::Output;
    use crate::bridge::world::{ClientUnit, UnitKey, MONSTER, PLAYER};
    use crate::ui::layout::Screen;
    use crate::ui::original::UiConfig;
    use crate::ui::{NoPanelRules, NoStrings, UiRoot};

    const KASHYA: u32 = 150;

    fn world() -> ClientWorld {
        let mut w = ClientWorld::default();
        let pk = UnitKey::new(PLAYER, 1);
        let mut p = ClientUnit::new(pk);
        p.position = Some((100, 100));
        w.units.insert(pk, p);
        w.local_player = Some(pk);
        for (guid, at) in [(70, (400, 400)), (77, (104, 100))] {
            let mut n = ClientUnit::new(UnitKey::new(MONSTER, guid));
            n.class = KASHYA;
            n.position = Some(at);
            w.units.insert(n.key, n);
        }
        w
    }

    /// [`world`] with the local unit a player (the list draws only then).
    fn player_world() -> ClientWorld {
        let mut w = world();
        let pk = w.local_player.unwrap();
        w.units.get_mut(&pk).unwrap().kind =
            KindData::Player(crate::bridge::world::PlayerData::default());
        w
    }

    fn setup() -> (OriginalUi, UiRoot) {
        setup_at(Screen::R800)
    }

    fn setup_at(screen: Screen) -> (OriginalUi, UiRoot) {
        let config = UiConfig {
            screen,
            expansion_installed: true,
        };
        let ui = OriginalUi::new(config, None).unwrap();
        let mut root = UiRoot::new(Box::new(NoPanelRules));
        ui.install(&mut root).unwrap();
        (ui, root)
    }

    fn release(root: &mut UiRoot, w: &ClientWorld, x: i32, y: i32) {
        let ctx = UiCtx {
            tick: 0,
            world: w,
            strings: &NoStrings,
        };
        let e = UiEvent::Release {
            button: PointerButton::Left,
            at: Point::new(x, y),
        };
        root.dispatch(e, &ctx);
    }

    // The list opens from the menu's Hire option only (the server's
    // 0x4F / 0x4E fill it, they no longer open it: D2RUST's own auto-open
    // for the nearest seller is gone with the spec NPC menu).
    #[test]
    fn the_hire_option_opens_the_list_and_a_row_sends_0x36() {
        let w = world();
        let (mut ui, mut root) = setup();
        ui.apply_output(&Output::HireListReset, &w).unwrap();
        assert_eq!(ui.hire_list().up, None, "the server's reset opens nothing");
        ui.open_hire_list(77);
        ui.apply_output(
            &Output::HireOffer {
                name: 3000,
                seed: 5,
            },
            &w,
        )
        .unwrap();
        ui.apply_output(
            &Output::HireOffer {
                name: 3001,
                seed: 6,
            },
            &w,
        )
        .unwrap();
        assert_eq!(ui.hire_list().up, Some(77));
        assert_eq!(ui.hire_list().offers.len(), 2);
        // Row 1 of the list at 800 × 600: list y 120, rows 15 high.
        release(&mut root, &w, 200, 120 + 15 + 3);
        assert_eq!(root.take_intents(), vec![hire_intent(77, 3001)]);
        assert_eq!(ui.hire_list().up, None, "the list closes on a hire");
        // The reset the server sends after the hire does not reopen it.
        ui.apply_output(&Output::HireListReset, &w).unwrap();
        assert_eq!(ui.hire_list().up, None);
    }

    #[test]
    fn the_list_sits_by_the_screen_at_640_and_800() {
        // List x = (W − 490) / 2, y = (H − 40) / 2 − 160.
        for (screen, x, y) in [(Screen::R800, 155, 120), (Screen::R640, 75, 60)] {
            let w = player_world();
            let (mut ui, mut root) = setup_at(screen);
            ui.open_hire_list(77);
            ui.apply_output(&Output::HireListReset, &w).unwrap();
            for name in [3000, 3001] {
                ui.apply_output(&Output::HireOffer { name, seed: 5 }, &w)
                    .unwrap();
            }
            assert_eq!(ui.hire_list().screen, (screen.w, screen.h));
            // Every draw is clipped to the screen, not to 800 × 600.
            let ctx = UiCtx {
                tick: 0,
                world: &w,
                strings: &NoStrings,
            };
            let mut out: Vec<crate::ui::UiDraw> = Vec::new();
            root.draw(&ctx, &mut out);
            let texts: Vec<_> = out
                .iter()
                .filter_map(|d| match d {
                    crate::ui::UiDraw::Text(t) => Some(t.clip),
                    _ => None,
                })
                .collect();
            assert!(!texts.is_empty());
            assert!(texts.iter().all(|c| *c == screen.rect()));
            // Row 1 of the list under the list's own origin.
            release(&mut root, &w, x + 45, y + 15 + 3);
            assert_eq!(root.take_intents(), vec![hire_intent(77, 3001)]);
        }
    }

    #[test]
    fn the_npc_menu_clips_to_the_screen_at_640_and_800() {
        for screen in [Screen::R800, Screen::R640] {
            let w = player_world();
            let (mut ui, mut root) = setup_at(screen);
            ui.open_npc_menu(77, KASHYA, 1, &w);
            ui.npc_menu_poll(&w, &mut root, &NoStrings);
            assert!(ui.npc_menu().is_some(), "the box is built");
            let ctx = UiCtx {
                tick: 0,
                world: &w,
                strings: &NoStrings,
            };
            let mut out: Vec<crate::ui::UiDraw> = Vec::new();
            root.draw(&ctx, &mut out);
            let clips: Vec<_> = out
                .iter()
                .filter_map(|d| match d {
                    crate::ui::UiDraw::Text(t) => Some(t.clip),
                    _ => None,
                })
                .collect();
            assert!(!clips.is_empty(), "{screen:?}");
            assert!(clips.iter().all(|c| *c == screen.rect()), "{screen:?}");
        }
    }

    #[test]
    fn back_closes_without_a_message() {
        let w = world();
        let (mut ui, mut root) = setup();
        ui.open_hire_list(77);
        ui.apply_output(&Output::HireListReset, &w).unwrap();
        ui.apply_output(
            &Output::HireOffer {
                name: 3000,
                seed: 5,
            },
            &w,
        )
        .unwrap();
        release(&mut root, &w, 200, 85 + 315 + 5);
        assert!(root.take_intents().is_empty());
        assert_eq!(ui.hire_list().up, None);
        assert!(ui.hire_list().back, "the NPC menu is rebuilt");
    }

    // The hire box's p1 (Back) runs on Esc and Space: the list closes and
    // the NPC menu is asked for again.
    #[test]
    fn esc_closes_the_hire_list_and_rebuilds_the_menu() {
        let w = world();
        let (mut ui, mut root) = setup();
        ui.open_hire_list(77);
        let ctx = UiCtx {
            tick: 0,
            world: &w,
            strings: &NoStrings,
        };
        let e = UiEvent::Action(ActionId(Action::ClearScreen.index() as u16));
        root.dispatch(e, &ctx);
        assert_eq!(ui.hire_list().up, None, "the list closes");
        assert!(ui.hire_list().back, "the NPC menu is rebuilt");
    }

    #[test]
    fn the_npc_menu_hook_opens_the_list() {
        let (mut ui, _root) = setup();
        ui.open_hire_list(9);
        assert_eq!(ui.hire_list().up, Some(9));
    }
}
