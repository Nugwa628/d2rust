//! Game messages in play: the screen message list fed by
//! S→C 0x26 chat lines and the quest screen messages of S→C 0x5D, drawn
//! at the top left as dark-backed lines for 10 s. The rules are
//! [`crate::ui::messages::chat`]; this adapter only owns the list in the
//! UI's shared state, the font metrics and the draw requests.
// D2RUST's own, unverified: the `GetTickCount` of the expiry is the UI clock
// ([`super::ui_clock`]: the wall clock in live play, so a line lasts 10 s at
// any frame rate; 40 ms per bridge frame in check runs); the text of a chat line is read
// as Latin-1 (the chat code page); the backing is the HUD's dark fill
// tiles instead of `DrawRectangle`.

use super::hud::{FILL_FILE, FILL_H, FILL_W};
use crate::bridge::output::Output;
use crate::rules::camera::FrameSize;
use crate::ui::draw::{ImageRef, ImageRequest, TextRequest, TextStyle, UiDraw, UiDrawSink};
use crate::ui::geom::{Point, Rect};
use crate::ui::messages::chat::{
    chat_action, ChatAction, ChatMessage, ChatStrings, ScreenMessages,
};
use crate::ui::messages::{Metrics, FONT_CHAT};
use crate::ui::panel::{Panel, PanelId, UiCtx, UiEvent, UiResponse, WidgetId};
use crate::ui::text::TextOpts;

use super::{FontMeasure, SharedRef};

/// The adapter's id: not a UI state, open for good.
pub const MESSAGES_PANEL: PanelId = PanelId(0x111);

/// The fill file's dark frame (`hud::fill_frames`).
const DARK: u32 = 4;
/// UI states the draw gates read.
const UI_STATE_13: u8 = 0x13;
const UI_MESSAGE_LOG: u8 = 0x18;

/// The wide text of 8-bit message bytes (Latin-1, module doc).
pub(super) fn wide(b: &[u8]) -> Vec<u16> {
    b.iter().map(|&c| u16::from(c)).collect()
}

/// The screen message list, and the string ids (quest messages) waiting
/// for the string table of the next draw.
#[derive(Clone, Debug, Default)]
pub struct GameMessages {
    list: ScreenMessages,
    pending_ids: Vec<(u16, u32)>,
    strings: Option<ChatStrings>,
}

/// Font metrics from the loaded tables; without them a line is not
/// wrapped and a unit is 8 pixels wide.
pub(crate) struct Measure<'a>(pub(crate) Option<&'a FontMeasure>);

impl Metrics for Measure<'_> {
    fn wrap(&self, font: u16, text: &[u16], max: i32) -> Vec<Vec<u16>> {
        self.0
            .and_then(|f| f.wrap(font, text, max))
            .unwrap_or_else(|| vec![text.to_vec()])
    }
    fn width_a(&self, font: u16, text: &[u16]) -> i32 {
        self.0
            .and_then(|f| f.width_a(font, text))
            .unwrap_or(8 * text.len() as i32)
    }
    fn width_c(&self, font: u16, text: &[u16]) -> i32 {
        self.0
            .and_then(|f| f.width_c(font, text))
            .unwrap_or(8 * text.len() as i32)
    }
    fn font_height(&self, _font: u16) -> i32 {
        16
    }
}

impl GameMessages {
    /// The list (tests, the message log).
    pub fn list(&self) -> &ScreenMessages {
        &self.list
    }

    /// At UI clock `now` ms ([`super::ui_clock`]).
    pub fn add(
        &mut self,
        text: &[u16],
        color: u32,
        now: u32,
        w: i32,
        log_open: bool,
        fonts: Option<&FontMeasure>,
    ) {
        self.list
            .add(text, color, now, w, log_open, &Measure(fonts));
    }

    /// A screen message of string table entry `id` (S→C 0x5D rows), shown
    /// at the next draw, which has the string table.
    pub fn add_string_id(&mut self, id: u16, color: u32) {
        self.pending_ids.push((id, color));
    }

    /// The line a delivered 0x26 adds, `None`
    /// for the types that add none (overhead text, recipe scroll, the
    /// refused rows).
    pub fn chat_line_text(&mut self, o: &Output) -> Option<(Vec<u16>, u32)> {
        let Output::ChatLine {
            kind,
            unit,
            b8,
            name,
            text,
            present,
            ..
        } = o
        else {
            return None;
        };
        let (name, text) = (wide(name), wide(text));
        let msg = ChatMessage {
            kind: *kind,
            b3: unit.unit_type,
            c8: *b8,
            name: &name,
            name_bytes: name.len(),
            text: &text,
            text_bytes: text.len(),
            raw_text: &text,
            unit_present: *present,
        };
        let strings = self.strings.get_or_insert_with(ChatStrings::default);
        match chat_action(&msg, strings) {
            Ok(ChatAction::Line { text, color }) => Some((text, color)),
            _ => None,
        }
    }
}

impl super::OriginalUi {
    /// The game messages the delivered `o` shows (the chat lines and the
    /// game messages). The 0x26 gates (squelch, conversion, the overhead record) stay in
    /// [`Self::chat_line`].
    pub(super) fn game_message(&mut self, o: &Output, frames: u64) {
        let (w, log_open, now) = {
            let sh = self.shared.borrow();
            (
                sh.config.screen.w,
                sh.states.is_open(UI_MESSAGE_LOG),
                sh.ui_ms32(frames),
            )
        };
        if matches!(o, Output::ChatLine { .. }) {
            if self.chat_line(o, false).is_none() {
                return;
            }
            let mut sh = self.shared.borrow_mut();
            let sh = &mut *sh;
            if let Some((text, color)) = sh.messages.chat_line_text(o) {
                sh.messages
                    .add(&text, color, now, w, log_open, sh.fonts.as_ref());
            }
        }
    }

    /// A screen message of string `id`, colour 0.
    pub(super) fn game_message_id(&mut self, id: u16) {
        self.shared.borrow_mut().messages.add_string_id(id, 0);
    }
}

/// The panel that draws the list (module doc).
pub(super) struct MessagesUi {
    pub(super) sh: SharedRef,
}

impl Panel for MessagesUi {
    fn id(&self) -> PanelId {
        MESSAGES_PANEL
    }

    fn rect(&self) -> Rect {
        Rect::new(0, 0, 0, 0)
    }

    fn draw(&self, ctx: &UiCtx, out: &mut dyn UiDrawSink) {
        let mut sh = self.sh.borrow_mut();
        let sh = &mut *sh;
        let now = sh.ui_ms32(ctx.tick);
        let (w, open_mode) = (sh.config.screen.w, sh.states.open_mode().get());
        // In a frame wider than the 800-wide layout the list keeps to the
        // frame's own top-left corner, `ui_left` pixels left of the UI box;
        // with a left panel open (open mode 2) it starts right of the
        // panel, whose side ends at `left_end` (W / 2 at 4:3). The clip is
        // the whole frame. At 4:3 nothing moves.
        let (s, ui_left) = (sh.config.screen, FrameSize::play().ui_left());
        let dx = if open_mode == 2 {
            s.left_end() - s.w / 2
        } else {
            -ui_left
        };
        let clip = Rect::new(-ui_left, 0, (s.w + 2 * ui_left) as u16, s.h as u16);
        let (state_13, log_open) = (
            sh.states.is_open(UI_STATE_13),
            sh.states.is_open(UI_MESSAGE_LOG),
        );
        let m = Measure(sh.fonts.as_ref());
        for (id, color) in std::mem::take(&mut sh.messages.pending_ids) {
            if let Some(t) = ctx.strings.get_id(id) {
                sh.messages.list.add(t, color, now, w, log_open, &m);
            }
        }
        // The list is drawn only while the message log is closed.
        if !log_open {
            let fill = sh.tables.files.id(FILL_FILE);
            for (mut r, l) in sh.messages.list.draw(w, open_mode, state_13, &m) {
                r.x += dx;
                if let Some(file) = fill {
                    backing(file, &r, out);
                }
                out.push(UiDraw::Text(TextRequest {
                    text: l.text,
                    at: Point::new(l.x + dx, l.y),
                    style: TextStyle {
                        font: FONT_CHAT,
                        color: u16::try_from(l.color).unwrap_or(0),
                    },
                    opts: TextOpts::default(),
                    clip,
                }));
            }
        }
        sh.messages.list.expire(now);
    }

    fn hit(&self, _p: Point) -> Option<WidgetId> {
        None
    }

    fn event(&mut self, _e: UiEvent, _ctx: &UiCtx) -> UiResponse {
        UiResponse::Ignored
    }
}

/// The dark text backing of rectangle `r` as fill tiles.
pub(super) fn backing(file: u32, r: &crate::ui::messages::RectDraw, out: &mut dyn UiDrawSink) {
    let (tw, th) = (FILL_W as i32, FILL_H as i32);
    let mut y = r.y;
    while y < r.y + r.h {
        let mut x = r.x;
        while x < r.x + r.w {
            let (cw, ch) = ((r.x + r.w - x).min(tw), (r.y + r.h - y).min(th));
            out.push(UiDraw::Image(ImageRequest {
                image: ImageRef { file, frame: DARK },
                at: Point::new(x, y),
                clip: Rect::new(x, y, cw as u16, ch as u16),
                look: crate::ui::CelLook::PLAIN,
                call: crate::ui::draw::CelCall::Draw,
            }));
            x += tw;
        }
        y += th;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::world::{ClientWorld, UnitKey, PLAYER};
    use crate::ui::layout::Screen;
    use crate::ui::original::{OriginalUi, UiConfig};
    use crate::ui::{NoPanelRules, StringLookup, UiRoot};

    struct Strs;
    impl StringLookup for Strs {
        fn get(&self, _: &str) -> Option<&[u16]> {
            None
        }
        fn get_id(&self, id: u16) -> Option<&[u16]> {
            const T: [u16; 4] = [b'Q' as u16, b'u' as u16, b'e' as u16, b's' as u16];
            (id == 3708).then_some(&T[..])
        }
    }

    fn setup() -> (OriginalUi, UiRoot, ClientWorld) {
        setup_at(Screen::R800)
    }

    fn setup_at(screen: Screen) -> (OriginalUi, UiRoot, ClientWorld) {
        let config = UiConfig {
            screen,
            expansion_installed: true,
        };
        let ui = OriginalUi::new(config, None).unwrap();
        let mut root = UiRoot::new(Box::new(NoPanelRules));
        ui.install(&mut root).unwrap();
        (ui, root, ClientWorld::default())
    }

    fn chat(kind: u8, unit_type: u8, c8: u8, name: &str, text: &str) -> Output {
        Output::ChatLine {
            kind,
            lang: 0,
            unit: UnitKey::new(unit_type, 4),
            b8: c8,
            b9: 0,
            name: name.as_bytes().to_vec(),
            text: text.as_bytes().to_vec(),
            present: false,
            player_name: None,
        }
    }

    fn lines(root: &UiRoot, w: &ClientWorld, tick: u64) -> Vec<(String, i32, i32, u16)> {
        let ctx = UiCtx {
            tick,
            world: w,
            strings: &Strs,
        };
        let mut out: Vec<UiDraw> = Vec::new();
        root.draw(&ctx, &mut out);
        out.iter()
            .filter_map(|d| match d {
                UiDraw::Text(t) if t.style.font == FONT_CHAT => Some((
                    String::from_utf16_lossy(&t.text),
                    t.at.x,
                    t.at.y,
                    t.style.color,
                )),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_list_is_clipped_to_the_screen_at_640_and_800() {
        // X 15, y 20 on both frames; only the clip is the screen.
        for screen in [Screen::R800, Screen::R640] {
            let (mut ui, root, mut w) = setup_at(screen);
            w.frames = 10;
            ui.apply_output(&chat(4, PLAYER, 1, "", "Hi"), &w).unwrap();
            let ctx = UiCtx {
                tick: 10,
                world: &w,
                strings: &Strs,
            };
            let mut out: Vec<UiDraw> = Vec::new();
            root.draw(&ctx, &mut out);
            let t: Vec<_> = out
                .iter()
                .filter_map(|d| match d {
                    UiDraw::Text(t) if t.style.font == FONT_CHAT => Some((t.at, t.clip)),
                    _ => None,
                })
                .collect();
            assert_eq!(t, [(Point::new(15, 20), screen.rect())], "{screen:?}");
        }
    }

    #[test]
    fn a_chat_line_is_drawn_at_the_top_left_for_ten_seconds() {
        let (mut ui, root, mut w) = setup();
        assert!(lines(&root, &w, 0).is_empty());
        w.frames = 10;
        ui.apply_output(&chat(4, PLAYER, 1, "", "You cannot use this"), &w)
            .unwrap();
        // Font 13, x 15, y 20, colour c8; the 8 px fallback metric.
        let v = lines(&root, &w, 10);
        assert_eq!(v, vec![("You cannot use this".to_string(), 15, 20, 1)]);
        // A second line steps 15 px.
        ui.apply_output(&chat(4, PLAYER, 0, "", "Second"), &w)
            .unwrap();
        assert_eq!(lines(&root, &w, 11).len(), 2);
        assert_eq!(lines(&root, &w, 11)[1].2, 35);
        // 10 s = 250 frames of 40 ms: gone after the expiry.
        // The expiry runs at the end of the draw that finds it.
        assert_eq!(lines(&root, &w, 10 + 251).len(), 2);
        assert!(lines(&root, &w, 10 + 252).is_empty());
    }

    // The 10 s is wall time: at 60 or 144 draws a second a frame count of
    // 40 ms each ended it after 4.2 s or 1.7 s.
    #[test]
    fn a_chat_line_lasts_ten_seconds_at_25_60_and_144_draws_a_second() {
        use crate::ui::ui_clock::{draw_ms, period_ms, RATES};
        for fps in RATES {
            let (mut ui, root, mut w) = setup();
            ui.set_menu_wall_ms(Some(draw_ms(fps, 0)));
            ui.apply_output(&chat(4, PLAYER, 1, "", "Hi"), &w).unwrap();
            let mut i = 0;
            while !lines(&root, &w, i).is_empty() {
                i += 1;
                w.frames = i;
                ui.set_menu_wall_ms(Some(draw_ms(fps, i)));
                assert!(i < 100_000);
            }
            let ms = draw_ms(fps, i) - draw_ms(fps, 0);
            // Gone at the draw after the one past the expiry.
            assert!(
                (10_000..=10_000 + 2 * period_ms(fps)).contains(&ms),
                "{fps}/s: {ms} ms"
            );
        }
    }

    #[test]
    fn a_named_line_and_a_whisper_take_the_formats_of_the_table() {
        let (mut ui, root, w) = setup();
        ui.apply_output(&chat(1, 2, 2, "Bob", "hi"), &w).unwrap();
        ui.apply_output(&chat(2, 0, 0, "Eve", "psst"), &w).unwrap();
        let v = lines(&root, &w, 0);
        assert_eq!(v[0].0, "\u{ff}c4Bob\u{ff}c0: hi");
        assert_eq!(v[1].0, "Eve whispers: psst");
        assert_eq!(v[1].3, 2);
    }

    #[test]
    fn a_quest_screen_message_shows_its_string() {
        let (mut ui, root, w) = setup();
        ui.apply_output(
            &Output::QuestUi {
                chain: 3,
                flags: 1,
                status: 0,
                extra: 0,
            },
            &w,
        )
        .unwrap();
        assert_eq!(lines(&root, &w, 0), vec![("Ques".to_string(), 15, 20, 0)]);
    }
}
