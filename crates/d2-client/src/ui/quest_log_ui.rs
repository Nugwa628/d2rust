//! The quest log (ui 0x0F, left slot) installed in the play UI: the
//! rows [`QuestLog::build_tab`] derives from the status list S→C 0x52
//! sent, the player's and the game's flag records (S→C 0x28 / 0x29) and
//! the Den of Evil count (S→C 0x50 type 1), drawn as the slot frames,
//! the icons and, for the selected row, the title and the text.
//!
//! Opening the log (`Q`) asks the server for the quest data with C→S 0x40; the
//! answer (0x28 type 6, 0x50, 0x52) fills the state.
//!
//! D2RUST's own, unverified (the open function, the slot
//! positions and the cel layout are not settled): C→S 0x40 on every open; the
//! act tab is the loaded act; slots on a 3 × 2 grid; the text is one line
//! (no wrap at 270 px, no measure); the just-completed animation and its
//! C→S 0x58 acknowledge are not run (the row draws as
//! completed); the speech replay button is not drawn.
//!
//! The description pane (after the icons): font 8, the title centred on `sx +
//! 160` at row 248 and the text wrapped at 270 px from `sx + 16`, row 270 + 20
//! i (rows in the 640 × 480 art space, shifted as the icons are); the close
//! button (`sx + 278`, row 422; frame 10, 11 while held) acts on the release
//! inside its rectangle (it closes the log).

use std::cell::RefCell;

use super::{OriginalUi, SharedRef};
use crate::ui::draw::UiDrawSink;
use crate::ui::geom::{Point, Rect};
use crate::ui::panel::{ClientIntent, Panel, PanelId, UiCtx, UiEvent, UiResponse, WidgetId};
use crate::ui::panels::{cel, text, UiFiles};
use crate::ui::quest_log::state::{QuestLog, TabGate, TabRow};
use crate::ui::quest_log::tables::ICON_NAMES;
use crate::ui::quest_log::{IconState, QuestFlags, RowText};

/// The description pane (globals.. read from the image): font 8 (FontFormal11),
/// the title centred on x 160 at row 248, the text at x 16 from row 270, 20
/// rows a line, wrapped at 270 px.
const PANE_FONT: u16 = 8;
const PANE_TITLE_X: i32 = 160;
const PANE_TITLE_Y: i32 = 248;
const PANE_TEXT_X: i32 = 16;
const PANE_TEXT_Y: i32 = 270;
const PANE_LINE: i32 = 20;
const PANE_WRAP: i32 = 270;

/// The close button's hit rectangle: x in
/// [0x116, 0x116 + 0x24), y in [0x188, 0x188 + 0x22) of the 640 x 480 art
/// space.
fn close_rect(sx: i32, base: i32) -> Rect {
    Rect::new(sx + 0x116, base + 0x188, 0x24, 0x22)
}

/// The state UI id of the quest screen.
pub const UI_QUEST_SCREEN: u8 = crate::ui::states::id::QUEST_SCREEN;

/// C→S 0x40 RequestQuestData (the handler wants size 1).
pub fn request_quest_data() -> ClientIntent {
    ClientIntent(vec![0x40])
}

/// Panel cels under `data\global\ui\`.
const BACKGROUND: &str = "menu\\questbackground";
const SOCKETS: &str = "menu\\questsockets";
const DONE: &str = "menu\\questdone";
const TABS: &str = "menu\\expquesttabs";
const LAST: &str = "menu\\questlast";

/// The files the log draws, for [`UiFiles::extend`].
pub fn quest_files() -> Vec<String> {
    let mut v = vec![
        BACKGROUND.to_string(),
        SOCKETS.to_string(),
        DONE.to_string(),
        TABS.to_string(),
        LAST.to_string(),
    ];
    v.extend(ICON_NAMES.iter().map(|n| format!("menu\\{n}")));
    v
}

/// What the log reads from the messages (copied by the controller after
/// each quest output).
#[derive(Clone, Debug)]
pub struct QuestInputs {
    /// S→C 0x52: `S[0..40]`.
    pub status: [u8; 41],
    /// P: S→C 0x28.
    pub player: QuestFlags,
    /// G: S→C 0x29.
    pub game: Option<QuestFlags>,
    /// D, Y, B of S→C 0x50 type 1.
    pub counters: [i32; 3],
}

impl Default for QuestInputs {
    fn default() -> Self {
        QuestInputs {
            status: [0; 41],
            player: QuestFlags::new(),
            game: None,
            counters: [0; 3],
        }
    }
}

/// A record of 96 bytes as 48 little-endian slots.
pub fn flags_of(record: &[u8; 96]) -> QuestFlags {
    let mut f = QuestFlags::new();
    for q in 0..48u8 {
        let i = usize::from(q) * 2;
        f.set_word(q, u16::from_le_bytes([record[i], record[i + 1]]));
    }
    f
}

/// The rows of the shown tab for `inputs`, deriving them on `log`.
pub fn rows(log: &mut QuestLog, inputs: &QuestInputs, tab: u8, multiplayer: bool) -> Vec<TabRow> {
    log.status = inputs.status;
    log.den = inputs.counters[0];
    log.tomb = inputs.counters[1];
    log.barbarians = inputs.counters[2];
    let gate = TabGate {
        expansion_installed: false,
        expansion_game: false,
        cel_loaded: &|_| true,
    };
    log.build_tab(
        tab,
        &inputs.player,
        inputs.game.as_ref(),
        multiplayer,
        &gate,
    )
}

/// The log's slot position (cel draw point), measured at 800 × 600:
/// the panel art sits in 640 × 480
/// coordinates shifted by (`sx`, `H + sy − 480`); icon (col, row) at (26 +
/// 97 col, 121 + 97 row), its socket 4 left and 5 down of it. `sx`, `base`:
/// the shifts.
pub fn slot_at(slot: u8, sx: i32, base: i32) -> Point {
    let (col, row) = (i32::from(slot % 3), i32::from(slot / 3));
    Point::new(sx + 26 + col * 97, base + 121 + row * 97)
}

/// The quest log adapter (ui 0x0F).
pub(super) struct QuestLogUi {
    pub(super) sh: SharedRef,
    pub(super) log: RefCell<QuestLog>,
    /// The act tab last opened (the tab rebuild runs when it changes).
    pub(super) shown: std::cell::Cell<Option<u8>>,
    /// The close button is held down.
    pub(super) close_down: std::cell::Cell<bool>,
}

/// Frame of an icon cel for a row state: 24
/// completed, 26 not available, 0 in progress.
fn icon_frame(s: IconState) -> u32 {
    match s {
        IconState::JustCompleted | IconState::Completed => 24,
        IconState::NotAvailable => 26,
        IconState::InProgress => 0,
    }
}

fn string_of(ctx: &UiCtx, id: u16) -> Option<Vec<u16>> {
    ctx.strings.get_id(id).map(<[u16]>::to_vec)
}

fn row_text(ctx: &UiCtx, t: RowText) -> Option<Vec<u16>> {
    match t {
        RowText::Empty => None,
        RowText::Id(id) => string_of(ctx, id),
        RowText::Append(id, n) => {
            let mut s = string_of(ctx, id)?;
            s.extend(n.to_string().encode_utf16());
            Some(s)
        }
        RowText::Format(id, n) => {
            let s = string_of(ctx, id)?;
            let f: String = String::from_utf16_lossy(&s).replacen("%d", &n.to_string(), 1);
            Some(f.encode_utf16().collect())
        }
    }
}

fn slot_rect(p: Point) -> Rect {
    Rect::new(p.x, p.y - 70, 70, 70)
}

impl Panel for QuestLogUi {
    fn id(&self) -> PanelId {
        PanelId(u16::from(UI_QUEST_SCREEN))
    }

    fn rect(&self) -> Rect {
        let s = self.sh.borrow().config.screen;
        s.left_side_rect()
    }

    fn draw(&self, ctx: &UiCtx, out: &mut dyn UiDrawSink) {
        let sh = self.sh.borrow();
        let files: &UiFiles = &sh.tables.files;
        let (Some(back), Some(sockets)) = (files.id(BACKGROUND), files.id(SOCKETS)) else {
            return;
        };
        let scr = sh.config.screen;
        let (sx, base) = (scr.sx(), scr.h + scr.sy() - 480);
        // The four background frames (2 × 2, 256 apart), the tab strip, the
        // close button and the replay button, as the game draws them.
        for (f, (x, y)) in [(0, 256), (256, 256), (0, 432), (256, 432)]
            .into_iter()
            .enumerate()
        {
            out.push(cel(back, f as u32, sx + x, base + y));
        }
        let tab = ctx.world.act.map_or(0, |a| a.act).min(4);
        let rows = rows(
            &mut self.log.borrow_mut(),
            &sh.quest,
            tab,
            ctx.world.expansion == 0,
        );
        // The tab strip (`quest_log::state::tab_strip`): one cel per tab, frame
        // 2k for the shown act and 2k + 1 for a reachable other (measured
        // `w1-w2-quest-log-act2`: Act II draws frame 1 at x 5 and frame 2 at x
        // 0x43). Without the expansion one cel of the shown act as before (the
        // classic art is not read). The close button is `panel\buysellbtn`
        // frame 10 at (278, 422), the replay button `questlast` frame 0 at
        // (226, 422).
        if let Some(t) = files.id(TABS) {
            if ctx.world.expansion != 0 {
                for (x, frame, dy) in
                    crate::ui::quest_log::state::tab_strip(tab, &sh.quest.player, true)
                {
                    out.push(cel(t, frame, sx + x, base + 32 + dy));
                }
            } else {
                out.push(cel(t, u32::from(tab), sx + 5, base + 33));
            }
        }
        let shown = self.shown.get();
        if shown != Some(tab) {
            self.shown.set(Some(tab));
            let gate = TabGate {
                expansion_installed: false,
                expansion_game: false,
                cel_loaded: &|_| true,
            };
            let _ = self.log.borrow_mut().open_tab(
                tab,
                true,
                &sh.quest.player,
                sh.quest.game.as_ref(),
                ctx.world.expansion == 0,
                &gate,
            );
        }
        let selected = self.log.borrow().selected;
        for r in &rows {
            let at = slot_at(r.slot, sx, base);
            // The icon, then its selection socket (the game's draw order).
            let name = format!("menu\\{}", ICON_NAMES[usize::from(r.icon)]);
            if let Some(icon) = files.id(&name) {
                out.push(cel(icon, icon_frame(r.row.icon), at.x, at.y));
            }
            out.push(cel(
                sockets,
                u32::from(Some(r.slot) == selected),
                at.x - 4,
                at.y + 5,
            ));
        }
        if let Some(b) = files.id("panel\\buysellbtn") {
            out.push(cel(
                b,
                10 + u32::from(self.close_down.get()),
                sx + 278,
                base + 422,
            ));
        }
        if let Some(l) = files.id(LAST) {
            out.push(cel(l, 0, sx + 226, base + 422));
        }
        // The selected row: title, then the text (no wrap).
        let pick = rows.iter().find(|r| Some(r.slot) == selected);
        let Some(r) = pick else {
            return;
        };
        let measure = |t: &[u16]| sh.fonts.as_ref().and_then(|f| f.width_a(PANE_FONT, t));
        if crate::ui::quest_log::state::title_drawn(r.row.title) {
            if let Some(t) = string_of(ctx, r.row.title) {
                // x = sx - width / 2 + 160
                // (C division).
                let w = measure(&t).unwrap_or(0);
                out.push(text(
                    t,
                    sx + PANE_TITLE_X - w / 2,
                    base + PANE_TITLE_Y,
                    PANE_FONT,
                    0,
                ));
            }
        }
        if let Some(t) = row_text(ctx, r.row.text) {
            let lines = sh
                .fonts
                .as_ref()
                .and_then(|f| f.wrap(PANE_FONT, &t, PANE_WRAP))
                .unwrap_or_else(|| vec![t]);
            for (i, l) in lines.into_iter().enumerate() {
                out.push(text(
                    l,
                    sx + PANE_TEXT_X,
                    base + PANE_TEXT_Y + PANE_LINE * i as i32,
                    PANE_FONT,
                    0,
                ));
            }
        }
    }

    fn ui_elements(&self, ctx: &UiCtx) -> Vec<crate::ui::ui_ids::UiElement> {
        use crate::ui::ui_ids::UiElement;
        let sh = self.sh.borrow();
        let scr = sh.config.screen;
        let (sx, base) = (scr.sx(), scr.h + scr.sy() - 480);
        let mut out = vec![UiElement::new("quests.close", close_rect(sx, base))];
        let tab = ctx.world.act.map_or(0, |a| a.act).min(4);
        let mut log = self.log.borrow().clone();
        out.extend(
            rows(&mut log, &sh.quest, tab, ctx.world.expansion == 0)
                .into_iter()
                .map(|r| {
                    let p = slot_at(r.slot, sx, base);
                    UiElement::new(format!("quests.slot({})", r.slot), slot_rect(p))
                }),
        );
        out
    }

    fn hit(&self, _p: Point) -> Option<WidgetId> {
        None
    }

    /// D2RUST's own, unverified: a press inside the slot's frame selects it;
    /// the panel takes every press inside its rectangle.
    fn event(&mut self, e: UiEvent, ctx: &UiCtx) -> UiResponse {
        let (press, at) = match e {
            UiEvent::Press {
                button: crate::ui::PointerButton::Left,
                at,
            } => (true, at),
            UiEvent::Release {
                button: crate::ui::PointerButton::Left,
                at,
            } => (false, at),
            _ => return UiResponse::Ignored,
        };
        let mut sh = self.sh.borrow_mut();
        let scr = sh.config.screen;
        let (sx, base) = (scr.sx(), scr.h + scr.sy() - 480);
        let in_close = close_rect(sx, base).contains(at);
        if !press {
            // The release inside the rectangle of a held
            // close button closes the log.
            let held = self.close_down.replace(false);
            if held && in_close {
                sh.outputs.push(crate::ui::panels::PanelOutput::SetUi {
                    ui: UI_QUEST_SCREEN,
                    mode: 1,
                    jump: false,
                });
                return UiResponse::Consumed;
            }
            return UiResponse::Ignored;
        }
        if in_close {
            // The click sound of the press.
            sh.outputs.push(crate::ui::panels::PanelOutput::Sound(
                crate::ui::original::CLICK_SOUND_ID,
            ));
            self.close_down.set(true);
            return UiResponse::Consumed;
        }
        let tab = ctx.world.act.map_or(0, |a| a.act).min(4);
        let rows = rows(
            &mut self.log.borrow_mut(),
            &sh.quest,
            tab,
            ctx.world.expansion == 0,
        );
        for r in rows {
            let p = slot_at(r.slot, sx, base);
            // The icon cels are about 70 × 70 (unmeasured).
            if slot_rect(p).contains(at) {
                self.log.borrow_mut().selected = Some(r.slot);
                return UiResponse::Consumed;
            }
        }
        UiResponse::Ignored
    }
}

impl OriginalUi {
    /// Copies what a quest output set into the message state to the
    /// log's inputs ([`QuestInputs`]); other outputs change nothing.
    pub(super) fn sync_quest_inputs(&mut self, o: &crate::bridge::output::Output) {
        use crate::bridge::output::Output;
        let mut sh = self.shared.borrow_mut();
        match o {
            Output::QuestLog { status } => sh.quest.status = *status,
            Output::QuestFlags { .. } | Output::NpcDialog(_) => {
                sh.quest.player = flags_of(&self.more.client_quest);
            }
            Output::GameQuestFlags { record } => sh.quest.game = Some(flags_of(record)),
            Output::QuestSpecial { code: 1, .. } => {
                let w = self.more.quest_words;
                sh.quest.counters = [w[0], w[1], w[2]];
            }
            _ => {}
        }
    }

    /// The rows of the quest log's shown tab (`tab`), derived on a
    /// scratch log; empty before any quest message.
    pub fn quest_rows(&self, tab: u8, multiplayer: bool) -> Vec<TabRow> {
        rows(
            &mut QuestLog::new(),
            &self.shared.borrow().quest,
            tab,
            multiplayer,
        )
    }
}
