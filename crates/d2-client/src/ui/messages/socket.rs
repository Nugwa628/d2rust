//! The item-socket dialog (UI state 0x0E, 0x58 codes): the Horadric
//! orifice (mode 0) and the NPC services imbue / sockets / personalize
//! (mode 1).

use super::{msg_u32s, LineDraw, Ltrb, Metrics, RectDraw, FONT_16};
use crate::ui::panel::ClientIntent;
use crate::ui::panels::menu_box::{MenuBox, MenuParams, NoteHandler, STR_WAITING};
use crate::ui::panels::PanelOutput;

/// The UI state.
pub const UI_SOCKET: u8 = 0x0E;
/// A left button down is ignored within 400 ms of the last accepted one.
pub const CLICK_GAP_MS: u32 = 400;
/// The mouse window of an accepted open.
pub const MOUSE_WINDOW: (i32, i32, i32, i32) = (107, 91, 222, 278);
/// Strings.
pub const STR_OK: u16 = 3401;
pub const STR_ADD_SOCKETS: u16 = 22748;
pub const STR_PERSONALIZE: u16 = 22749;
/// NPC classes of mode 1.
pub const NPC_CHARSI: u32 = 154;
pub const NPC_LARZUK: u32 = 511;
pub const NPC_ANYA: u32 = 512;

/// The backgrounds (loaded at game start; freed at exit) and the button cel.
pub const BACKGROUND_UPGRADE: &str = "menu\\upgrade";
pub const BACKGROUND_HORADRIC: &str = "menu\\horadricback";
pub const BUTTON_CEL: &str = "panel\\buysellbtn";

/// The background file of a mode: `menu\upgrade` (mode 1), `menu\horadricback`
/// (mode 0).
pub fn background_file(mode: Mode) -> &'static str {
    match mode {
        Mode::Object => BACKGROUND_HORADRIC,
        Mode::Npc => BACKGROUND_UPGRADE,
    }
}

/// The socket source: 0 = object (0x58 code 0), 1 = NPC service.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Object,
    Npc,
}

/// The socket dialog's step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Closed = 0,
    Open = 1,
    Placed = 2,
    Waiting = 3,
}

/// A button record (40 bytes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Button {
    /// Caption u16 +0.
    pub caption: u16,
    /// x +2, y +6.
    pub x: i32,
    pub y: i32,
    /// Hit left, right, top, bottom (all inclusive).
    pub left: i32,
    pub right: i32,
    pub top: i32,
    pub bottom: i32,
    /// Base frame +0x22.
    pub base: u32,
    /// Pressed +0x26, armed +0x27.
    pub pressed: bool,
    pub armed: bool,
}

impl Button {
    /// The frame drawn: base + 1 while pressed.
    pub fn frame(&self) -> u32 {
        self.base + u32::from(self.pressed)
    }

    pub fn hit(&self, x: i32, y: i32) -> bool {
        (self.left..=self.right).contains(&x) && (self.top..=self.bottom).contains(&y)
    }
}

/// The two buttons: 0 "imbue" (4017) at (122, 256), hit 122–154 × 224–256,
/// frame 16; 1 "close" (4143) at (177, 256), hit 177–209 × 224–256, frame
/// 10.
pub const BUTTONS: [Button; 2] = [
    Button {
        caption: 4017,
        x: 122,
        y: 256,
        left: 122,
        right: 154,
        top: 224,
        bottom: 256,
        base: 16,
        pressed: false,
        armed: false,
    },
    Button {
        caption: 4143,
        x: 177,
        y: 256,
        left: 177,
        right: 209,
        top: 224,
        bottom: 256,
        base: 10,
        pressed: false,
        armed: false,
    },
];

/// What the dialog asks of the rest of the UI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SocketEffect {
    Send(ClientIntent),
    /// A UI sound.
    Sound(u32),
    /// `SetUIState`.
    Ui(PanelOutput),
    ResetInput,
    Prep,
    InputReset,
    MouseWindow {
        l: i32,
        t: i32,
        r: i32,
        b: i32,
    },
    MouseWindowOff,
    RegisterHandlers,
    UnregisterHandlers,
    /// At close.
    CursorItem,
    /// The NPC interaction ends (open refused).
    EndInteraction,
    /// Menu state 0, the interaction end (C→S 0x30)
    /// and `SetUIState(8, off, 0)`.
    EndInteractionFull,
    /// The note box is freed.
    NoteFreed,
    /// The placed item goes back on the cursor.
    ItemToCursor,
    /// The cursor item is emptied into the dialog.
    CursorEmptied,
}

fn ui(mode: u8) -> SocketEffect {
    SocketEffect::Ui(PanelOutput::SetUi {
        ui: UI_SOCKET,
        mode,
        jump: false,
    })
}

/// The item facts a click inside the item area needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CursorItem {
    pub guid: u32,
    /// Holds.
    pub type_ok: bool,
    /// The NPC's accept check.
    pub npc_accepts: bool,
    /// The item's code is `hst ` (the Horadric staff).
    pub is_staff: bool,
}

/// The dialog state.
#[derive(Clone, Debug)]
pub struct SocketDialog {
    pub mode: Mode,
    pub step: Step,
    /// The object's GUID.
    pub object_guid: u32,
    /// The placed item, 0 = none.
    pub placed: u32,
    /// The last click time.
    pub last_click: u32,
    pub note: Option<MenuBox<NoteHandler>>,
    pub buttons: [Button; 2],
    pub npc_class: u32,
    pub npc_guid: u32,
    pub handlers: bool,
}

/// Failure of a socket placement.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SocketError {
    /// Draw at step 0 is fatal 0x31D.
    #[error("draw at step 0 (fatal 0x31D)")]
    StepZero,
}

/// A draw request of the dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SocketDraw {
    /// `menu\upgrade` (mode 1) / `menu\horadricback` (mode 0) frame 0 at
    /// (105, 270), light 0xFF, mode 5.
    Background { mode: Mode, x: i32, y: i32 },
    /// `panel\buysellbtn` frame at (x, y).
    Button { frame: u32, x: i32, y: i32 },
    /// The hover rectangle.
    HoverBox(RectDraw),
    /// Text in font 1.
    Text(LineDraw),
    /// The placed item's cel (unit type 4) at (x, y).
    Item { x: i32, y: i32 },
}

/// The result of a draw: the dialog closes itself for a dead or absent
/// local player.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SocketFrame {
    Close(Vec<SocketEffect>),
    Draw(Vec<SocketDraw>),
}

/// The draw environment.
pub struct DrawEnv<'a> {
    pub w: i32,
    pub h: i32,
    pub mouse: (i32, i32),
    pub player_dead_or_absent: bool,
    /// The placed item's cel size.
    pub item_size: Option<(i32, i32)>,
    pub strings: &'a dyn Fn(u16) -> Vec<u16>,
}

impl SocketDialog {
    /// A closed dialog for the NPC/object of the open.
    pub fn new() -> Self {
        Self {
            mode: Mode::Object,
            step: Step::Closed,
            object_guid: 0,
            placed: 0,
            last_click: 0,
            note: None,
            buttons: BUTTONS,
            npc_class: 0,
            npc_guid: 0,
            handlers: false,
        }
    }

    fn player(player: Option<u32>) -> u32 {
        player.unwrap_or(0xFFFF_FFFF)
    }

    /// 0x58 code 0: `accepted` is the result of
    /// `SetUIState(0x0E, on, 0)`.
    pub fn open(
        &mut self,
        mode: Mode,
        object_guid: u32,
        npc: (u32, u32),
        player: Option<u32>,
        accepted: bool,
        now: u32,
    ) -> Vec<SocketEffect> {
        self.mode = mode;
        self.object_guid = object_guid;
        (self.npc_guid, self.npc_class) = npc;
        let mut e = vec![SocketEffect::ResetInput];
        self.step = Step::Open;
        self.last_click = now;
        self.handlers = true;
        e.push(SocketEffect::RegisterHandlers);
        e.push(SocketEffect::Prep);
        e.push(ui(0));
        if accepted {
            e.push(SocketEffect::InputReset);
            let (l, t, r, b) = MOUSE_WINDOW;
            e.push(SocketEffect::MouseWindow { l, t, r, b });
        } else {
            match mode {
                Mode::Object => e.push(SocketEffect::Send(msg_u32s(
                    0x44,
                    &[Self::player(player), object_guid, 0, 2],
                ))),
                Mode::Npc => e.push(SocketEffect::EndInteraction),
            }
            e.extend(self.close_core());
        }
        e
    }

    /// The close: step:= 0; the cursor item dropped back; handlers
    /// unregistered; note box freed; `SetUIState(0x0E, off, 0)`; mouse window
    /// off.
    fn close_core(&mut self) -> Vec<SocketEffect> {
        self.step = Step::Closed;
        self.handlers = false;
        self.note = None;
        vec![
            SocketEffect::CursorItem,
            SocketEffect::UnregisterHandlers,
            SocketEffect::NoteFreed,
            ui(1),
            SocketEffect::MouseWindowOff,
        ]
    }

    /// A 0x58 code other than 0: codes 1, 5, 6, 7 close; 6 and 7 then end the
    /// NPC interaction; 4 (server refused, item kept): step:= 2 and the note
    /// box freed; code 5 plays UI sound 5 (`arg` ≠ 0) or 3 first.
    pub fn code(&mut self, code: u8, arg: u32) -> Vec<SocketEffect> {
        match code {
            1 | 5 | 6 | 7 => {
                let mut e = Vec::new();
                if code == 5 {
                    e.push(SocketEffect::Sound(if arg != 0 { 5 } else { 3 }));
                }
                e.extend(self.close_core());
                if code == 6 || code == 7 {
                    e.push(SocketEffect::EndInteractionFull);
                }
                e
            }
            4 => {
                self.step = Step::Placed;
                self.note = None;
                vec![SocketEffect::NoteFreed]
            }
            _ => Vec::new(),
        }
    }

    /// The draw.
    pub fn draw(&mut self, env: &DrawEnv<'_>, m: &dyn Metrics) -> Result<SocketFrame, SocketError> {
        if self.step == Step::Closed {
            return Err(SocketError::StepZero);
        }
        if env.player_dead_or_absent {
            return Ok(SocketFrame::Close(self.close_core()));
        }
        let mut out = vec![SocketDraw::Background {
            mode: self.mode,
            x: 105,
            y: 270,
        }];
        for b in &self.buttons {
            out.push(SocketDraw::Button {
                frame: b.frame(),
                x: b.x,
                y: b.y,
            });
        }
        // Hover: mouse x < 320 and y < H − 48, inside a hit rectangle.
        let (mx, my) = env.mouse;
        if mx < 320 && my < env.h - 48 {
            for (i, b) in self.buttons.iter().enumerate() {
                if !b.hit(mx, my) {
                    continue;
                }
                let cap = match (i, self.mode, self.npc_class) {
                    (0, Mode::Object, _) => STR_OK,
                    (0, Mode::Npc, NPC_LARZUK) => STR_ADD_SOCKETS,
                    (0, Mode::Npc, NPC_ANYA) => STR_PERSONALIZE,
                    _ => b.caption,
                };
                let text = (env.strings)(cap);
                let w = m.width_a(FONT_16, &text);
                let h = m.font_height(FONT_16);
                let (bx, by) = (b.x, b.y);
                out.push(SocketDraw::HoverBox(RectDraw {
                    x: bx - w / 2 + 10,
                    y: by - h - 42,
                    w: w + 12,
                    h: h + 6,
                    color: 0,
                    mode: 2,
                }));
                out.push(SocketDraw::Text(LineDraw {
                    text,
                    x: bx - w / 2 + 16,
                    y: by - 38,
                    color: 0,
                }));
                break;
            }
        }
        // Mode 1: the instruction, wrapped to 300, centred, color 4.
        if self.mode == Mode::Npc {
            let id = match self.npc_class {
                NPC_CHARSI => Some(10076),
                NPC_LARZUK => Some(22750),
                NPC_ANYA => Some(22747),
                _ => None,
            };
            if let Some(id) = id {
                let text = (env.strings)(id);
                for (i, line) in m.wrap(FONT_16, &text, 300).into_iter().enumerate() {
                    let w = m.width_a(FONT_16, &line);
                    out.push(SocketDraw::Text(LineDraw {
                        text: line,
                        x: 160 - w / 2,
                        y: 30 + 20 * i as i32,
                        color: 4,
                    }));
                }
            }
        }
        // Step 2: the placed item at x = 167 − w / 2, y = 106 + (h < 112 ? (114
        // − h) / 2: 0).
        if self.step == Step::Placed {
            if let Some((w, h)) = env.item_size {
                out.push(SocketDraw::Item {
                    x: 167 - w / 2,
                    y: 106 + if h < 112 { (114 - h) / 2 } else { 0 },
                });
            }
        }
        Ok(SocketFrame::Draw(out))
    }

    /// Left button down. `handled_by_inventory` is
    /// "the inventory handler took it first" (steps 1 and 2); `cursor` the
    /// item on the cursor. Returns the effects and whether the event is
    /// consumed.
    pub fn left_down(
        &mut self,
        now: u32,
        pos: (i32, i32),
        handled_by_inventory: bool,
        cursor: Option<CursorItem>,
    ) -> (Vec<SocketEffect>, bool) {
        // Ignored within 400 ms of the last click; every click not ignored
        // stamps the time first, accepted or not.
        if now.wrapping_sub(self.last_click) < CLICK_GAP_MS {
            return (Vec::new(), true);
        }
        self.last_click = now;
        let mut e = Vec::new();
        let live = matches!(self.step, Step::Open | Step::Placed);
        if live && handled_by_inventory {
            return (e, true);
        }
        // The button press (sound 1): steps 1 and 2 only.
        if live {
            for b in &mut self.buttons {
                if b.hit(pos.0, pos.1) {
                    b.pressed = true;
                    b.armed = true;
                    e.push(SocketEffect::Sound(1));
                }
            }
        }
        // The item area: x 123-211, y 106-220, keyed on the model's cursor
        // item (it stays the placed item while placed).
        if (123..=211).contains(&pos.0) && (106..=220).contains(&pos.1) {
            if let (Step::Open | Step::Placed, Some(c)) = (self.step, cursor) {
                let accepted = c.type_ok
                    && match self.mode {
                        Mode::Npc => c.npc_accepts,
                        Mode::Object => c.is_staff,
                    };
                if !accepted {
                    e.push(SocketEffect::Sound(3));
                } else if self.step == Step::Open {
                    self.step = Step::Placed;
                    self.placed = c.guid;
                    e.push(SocketEffect::CursorEmptied);
                } else {
                    self.step = Step::Open;
                    self.placed = 0;
                    e.push(SocketEffect::ItemToCursor);
                    e.push(SocketEffect::Sound(1));
                }
            }
        }
        (e, true)
    }

    /// Left button up: a release inside an armed button's hit rectangle plays
    /// sound 1 first; every button is then disarmed and released; the button
    /// then acts.
    pub fn left_up(
        &mut self,
        pos: (i32, i32),
        player: Option<u32>,
        strings: &dyn Fn(u16) -> Vec<u16>,
        m: &dyn Metrics,
    ) -> Vec<SocketEffect> {
        let mut e = Vec::new();
        let mut act = None;
        for (i, b) in self.buttons.iter().enumerate() {
            if b.armed && b.hit(pos.0, pos.1) {
                e.push(SocketEffect::Sound(1));
                act = Some(i);
            }
        }
        for b in &mut self.buttons {
            b.armed = false;
            b.pressed = false;
        }
        match act {
            Some(0) => e.extend(self.button0(player, strings, m)),
            Some(_) => e.extend(self.button1(player)),
            None => {}
        }
        e
    }

    /// Esc / Space: as button 1.
    pub fn esc_or_space(&mut self, player: Option<u32>) -> Vec<SocketEffect> {
        self.button1(player)
    }

    /// Button 0 (step 2 only): sound 2; the note box; step:= 3; the C→S
    /// message.
    fn button0(
        &mut self,
        player: Option<u32>,
        strings: &dyn Fn(u16) -> Vec<u16>,
        m: &dyn Metrics,
    ) -> Vec<SocketEffect> {
        if self.step != Step::Placed {
            return Vec::new();
        }
        let mut e = vec![SocketEffect::Sound(2)];
        // The note box (at (150, 130), p1, p5 = 1, p9 =
        // 1, style 0, one item 3353, height 15, font 1, color 0).
        let mut b = MenuBox::new(
            (150, 130),
            MenuParams {
                p1: Some(NoteHandler),
                p5: true,
                p9: 1,
                ..Default::default()
            },
        )
        .expect("p1 is set");
        b.set_style(0);
        // The caption item is the string 3353 (`TransactionResults1`).
        let _ = b.add_item(&strings(STR_WAITING), 15, 0, 0, FONT_16, None, false, m);
        self.note = Some(b);
        self.step = Step::Waiting;
        match self.mode {
            Mode::Object => e.push(SocketEffect::Send(msg_u32s(
                0x44,
                &[Self::player(player), self.object_guid, self.placed, 3],
            ))),
            // 0x38 [0 u32 @1][NPC GUID @5][placed item GUID @9].
            Mode::Npc => e.push(SocketEffect::Send(msg_u32s(
                0x38,
                &[0, self.npc_guid, self.placed],
            ))),
        }
        e
    }

    /// Button 1 and Esc / Space (not in step 3): step 2 → the item back on the
    /// cursor; sound 1; mode 0 → 0x44 [player] [object][0][2] and close; mode 1
    /// → end the interaction and close.
    fn button1(&mut self, player: Option<u32>) -> Vec<SocketEffect> {
        if self.step == Step::Waiting {
            return Vec::new();
        }
        let mut e = Vec::new();
        if self.step == Step::Placed {
            self.placed = 0;
            e.push(SocketEffect::ItemToCursor);
        }
        e.push(SocketEffect::Sound(1));
        match self.mode {
            Mode::Object => e.push(SocketEffect::Send(msg_u32s(
                0x44,
                &[Self::player(player), self.object_guid, 0, 2],
            ))),
            Mode::Npc => e.push(SocketEffect::EndInteraction),
        }
        e.extend(self.close_core());
        e
    }

    /// The dialog's hit rectangle of a button as an [`Ltrb`] (inclusive
    /// edges made exclusive).
    pub fn hit_rect(i: usize) -> Ltrb {
        let b = &BUTTONS[i];
        Ltrb::new(b.left, b.top, b.right + 1, b.bottom + 1)
    }
}

impl Default for SocketDialog {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::messages::testutil::{w, Fixed};

    fn strings(id: u16) -> Vec<u16> {
        w(&format!("s{id}"))
    }

    fn open_obj(accepted: bool) -> (SocketDialog, Vec<SocketEffect>) {
        let mut d = SocketDialog::new();
        let e = d.open(Mode::Object, 7, (0, 0), Some(1), accepted, 1000);
        (d, e)
    }

    // Test vector "0x58 code 0 for object GUID 7, state 0x0E refused".
    #[test]
    fn open_accepted_and_refused() {
        let (d, e) = open_obj(true);
        assert_eq!((d.step, d.last_click, d.handlers), (Step::Open, 1000, true));
        assert_eq!(
            e,
            vec![
                SocketEffect::ResetInput,
                SocketEffect::RegisterHandlers,
                SocketEffect::Prep,
                ui(0),
                SocketEffect::InputReset,
                SocketEffect::MouseWindow {
                    l: 107,
                    t: 91,
                    r: 222,
                    b: 278
                },
            ]
        );
        // Refused, mode 0: C→S 0x44 [player][object][0][2] and close.
        let (d, e) = open_obj(false);
        assert!(e.contains(&SocketEffect::Send(ClientIntent(vec![
            0x44, 1, 0, 0, 0, 7, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0
        ]))));
        assert_eq!(d.step, Step::Closed);
        assert!(e.contains(&ui(1)));
        // No local player: −1.
        let mut d = SocketDialog::new();
        let e = d.open(Mode::Object, 7, (0, 0), None, false, 0);
        assert!(e.contains(&SocketEffect::Send(msg_u32s(0x44, &[0xFFFF_FFFF, 7, 0, 2]))));
        // Refused, mode 1: the NPC interaction ends and the dialog closes.
        let mut d = SocketDialog::new();
        let e = d.open(Mode::Npc, 0, (5, 154), Some(1), false, 0);
        assert!(e.contains(&SocketEffect::EndInteraction));
        assert_eq!(d.step, Step::Closed);
    }

    #[test]
    fn close_codes() {
        for code in [1u8, 5, 6, 7] {
            let (mut d, _) = open_obj(true);
            let e = d.code(code, 1);
            assert_eq!(d.step, Step::Closed, "code {code}");
            assert!(e.contains(&SocketEffect::CursorItem));
            assert!(e.contains(&SocketEffect::UnregisterHandlers));
            assert!(e.contains(&SocketEffect::NoteFreed));
            assert!(e.contains(&ui(1)));
            assert!(e.contains(&SocketEffect::MouseWindowOff));
            assert_eq!(
                e.contains(&SocketEffect::EndInteractionFull),
                code == 6 || code == 7
            );
        }
        // Code 5: UI sound 5 with an argument, else 3, before the close.
        let (mut d, _) = open_obj(true);
        assert_eq!(d.code(5, 9)[0], SocketEffect::Sound(5));
        let (mut d, _) = open_obj(true);
        assert_eq!(d.code(5, 0)[0], SocketEffect::Sound(3));
        // Code 4: step:= 2 and the note box freed.
        let (mut d, _) = open_obj(true);
        d.step = Step::Waiting;
        d.note = Some(
            MenuBox::new(
                (0, 0),
                MenuParams {
                    p1: Some(NoteHandler),
                    ..Default::default()
                },
            )
            .unwrap(),
        );
        let e = d.code(4, 0);
        assert_eq!((d.step, d.note.is_none()), (Step::Placed, true));
        assert_eq!(e, vec![SocketEffect::NoteFreed]);
    }

    fn env<'a>(mouse: (i32, i32), strings: &'a dyn Fn(u16) -> Vec<u16>) -> DrawEnv<'a> {
        DrawEnv {
            w: 800,
            h: 600,
            mouse,
            player_dead_or_absent: false,
            item_size: Some((60, 40)),
            strings,
        }
    }

    #[test]
    fn state_and_buttons() {
        let d = SocketDialog::new();
        assert_eq!((d.mode, d.step), (Mode::Object, Step::Closed));
        let b0 = BUTTONS[0];
        assert_eq!((b0.caption, b0.x, b0.y, b0.base), (4017, 122, 256, 16));
        assert_eq!((b0.left, b0.right, b0.top, b0.bottom), (122, 154, 224, 256));
        let b1 = BUTTONS[1];
        assert_eq!((b1.caption, b1.x, b1.y, b1.base), (4143, 177, 256, 10));
        assert_eq!((b1.left, b1.right, b1.top, b1.bottom), (177, 209, 224, 256));
        // Frame + 1 while pressed; the hit rectangle is inclusive.
        let mut b = b0;
        assert_eq!(b.frame(), 16);
        b.pressed = true;
        assert_eq!(b.frame(), 17);
        assert!(b.hit(122, 224) && b.hit(154, 256));
        assert!(!b.hit(155, 256) && !b.hit(122, 223));
        assert_eq!((NPC_CHARSI, NPC_LARZUK, NPC_ANYA), (154, 511, 512));
        // The backgrounds by mode and the button cel.
        assert_eq!(background_file(Mode::Object), "menu\\horadricback");
        assert_eq!(background_file(Mode::Npc), "menu\\upgrade");
        assert_eq!(BUTTON_CEL, "panel\\buysellbtn");
    }

    #[test]
    fn draw_background_hover_instruction_and_item() {
        let m = Fixed;
        let (mut d, _) = open_obj(true);
        // Step 0 is fatal (a closed dialog).
        let mut closed = SocketDialog::new();
        assert_eq!(
            closed.draw(&env((0, 0), &strings), &m),
            Err(SocketError::StepZero)
        );
        // A dead or absent player closes the dialog.
        let mut e = env((0, 0), &strings);
        e.player_dead_or_absent = true;
        assert!(matches!(d.clone().draw(&e, &m), Ok(SocketFrame::Close(_))));
        // Mouse outside: the background and both buttons.
        let SocketFrame::Draw(out) = d.draw(&env((300, 300), &strings), &m).unwrap() else {
            panic!()
        };
        assert_eq!(
            out,
            vec![
                SocketDraw::Background {
                    mode: Mode::Object,
                    x: 105,
                    y: 270
                },
                SocketDraw::Button {
                    frame: 16,
                    x: 122,
                    y: 256
                },
                SocketDraw::Button {
                    frame: 10,
                    x: 177,
                    y: 256
                },
            ]
        );
        // Hover on button 0 in mode 0: "ok" (3401), a 2-wide box.
        let SocketFrame::Draw(out) = d.draw(&env((130, 230), &strings), &m).unwrap() else {
            panic!()
        };
        // "s3401": 5 units → w 35, h 16: rect (122 − 17 + 10, 256 − 16 −
        // 42, 47, 22), text at (122 − 17 + 16, 218).
        assert_eq!(
            out[3],
            SocketDraw::HoverBox(RectDraw {
                x: 115,
                y: 198,
                w: 47,
                h: 22,
                color: 0,
                mode: 2
            })
        );
        assert_eq!(
            out[4],
            SocketDraw::Text(LineDraw {
                text: w("s3401"),
                x: 121,
                y: 218,
                color: 0
            })
        );
        // Button 1 shows its own caption 4143.
        let SocketFrame::Draw(out) = d.draw(&env((180, 230), &strings), &m).unwrap() else {
            panic!()
        };
        assert!(matches!(&out[4], SocketDraw::Text(t) if t.text == w("s4143")));
        // Hover is only for x < 320 and y < H − 48.
        let SocketFrame::Draw(out) = d.draw(&env((130, 230), &strings), &m).unwrap() else {
            panic!()
        };
        assert_eq!(out.len(), 5);
        // Mode 1, NPC 511: the button-0 caption is 22748 and the
        // instruction 22750 centred at x = 160 − w / 2, y = 30, color 4.
        let mut n = SocketDialog::new();
        n.open(Mode::Npc, 0, (5, 511), Some(1), true, 0);
        let SocketFrame::Draw(out) = n.draw(&env((130, 230), &strings), &m).unwrap() else {
            panic!()
        };
        assert!(matches!(
            &out[0],
            SocketDraw::Background {
                mode: Mode::Npc,
                ..
            }
        ));
        assert!(matches!(&out[4], SocketDraw::Text(t) if t.text == w("s22748")));
        assert_eq!(
            out[5],
            SocketDraw::Text(LineDraw {
                text: w("s22750"),
                x: 160 - 21,
                y: 30,
                color: 4
            })
        );
        for (class, hover, instr) in [(512, "s22749", "s22747"), (154, "s4017", "s10076")] {
            let mut n = SocketDialog::new();
            n.open(Mode::Npc, 0, (5, class), Some(1), true, 0);
            let SocketFrame::Draw(out) = n.draw(&env((130, 230), &strings), &m).unwrap() else {
                panic!()
            };
            assert!(matches!(&out[4], SocketDraw::Text(t) if t.text == w(hover)));
            assert!(matches!(&out[5], SocketDraw::Text(t) if t.text == w(instr)));
        }
        // Step 2: the placed item at x = 167 − w / 2, y = 106 + (114 − h) / 2.
        d.step = Step::Placed;
        let SocketFrame::Draw(out) = d.draw(&env((300, 300), &strings), &m).unwrap() else {
            panic!()
        };
        assert_eq!(out.last(), Some(&SocketDraw::Item { x: 137, y: 143 }));
        let mut e = env((300, 300), &strings);
        e.item_size = Some((60, 120));
        let SocketFrame::Draw(out) = d.draw(&e, &m).unwrap() else {
            panic!()
        };
        assert_eq!(out.last(), Some(&SocketDraw::Item { x: 137, y: 106 }));
    }

    fn staff(guid: u32) -> CursorItem {
        CursorItem {
            guid,
            type_ok: true,
            npc_accepts: true,
            is_staff: true,
        }
    }

    #[test]
    fn place_and_take() {
        let (mut d, _) = open_obj(true);
        // Within 400 ms of the last accepted click: ignored.
        let (e, _) = d.left_down(1399, (150, 150), false, Some(staff(9)));
        assert!(e.is_empty() && d.step == Step::Open);
        // The inventory handler first.
        let (e, _) = d.left_down(2000, (150, 150), true, Some(staff(9)));
        assert!(e.is_empty() && d.step == Step::Open);
        // A staff dropped in the area (mode 0): placed; its drop sound is
        // the cursor's.
        // (The inventory-handled click at 2000 stamped the time already.)
        let (e, c) = d.left_down(2500, (150, 150), false, Some(staff(9)));
        assert!(c);
        assert_eq!(e, vec![SocketEffect::CursorEmptied]);
        assert_eq!((d.step, d.placed, d.last_click), (Step::Placed, 9, 2500));
        // An item other than the staff in mode 0: refused, sound 3.
        let (mut d, _) = open_obj(true);
        let mut c = staff(4);
        c.is_staff = false;
        let (e, _) = d.left_down(2000, (150, 150), false, Some(c));
        assert_eq!(e, vec![SocketEffect::Sound(3)]);
        assert_eq!(d.step, Step::Open);
        // A type that fails: refused.
        let mut c = staff(4);
        c.type_ok = false;
        let (e, _) = d.left_down(2500, (150, 150), false, Some(c));
        assert_eq!(e, vec![SocketEffect::Sound(3)]);
        // Mode 1: the NPC must accept it.
        let mut n = SocketDialog::new();
        n.open(Mode::Npc, 0, (5, 511), Some(1), true, 0);
        let mut c = staff(4);
        c.is_staff = false;
        c.npc_accepts = false;
        assert_eq!(
            n.left_down(2000, (150, 150), false, Some(c)).0,
            vec![SocketEffect::Sound(3)]
        );
        c.npc_accepts = true;
        n.left_down(2500, (150, 150), false, Some(c));
        assert_eq!((n.step, n.placed), (Step::Placed, 4));
        // Step 2: the model's cursor item stays the placed item; a click in the
        // area takes it back (placed 0, sound 1).
        let (e, _) = n.left_down(3000, (150, 150), false, Some(c));
        assert_eq!(e, vec![SocketEffect::ItemToCursor, SocketEffect::Sound(1)]);
        assert_eq!((n.step, n.placed), (Step::Open, 0));
        // Placed + a refused cursor item: sound 3; no cursor item: nothing.
        n.step = Step::Placed;
        n.placed = 4;
        let mut bad = c;
        bad.npc_accepts = false;
        assert_eq!(
            n.left_down(3500, (150, 150), false, Some(bad)).0,
            vec![SocketEffect::Sound(3)]
        );
        assert!(n.left_down(4000, (150, 150), false, None).0.is_empty());
        assert_eq!((n.step, n.placed), (Step::Placed, 4));
        n.step = Step::Open;
        n.placed = 0;
        // Outside the area (x 123–211, y 106–220): nothing placed.
        let (e, _) = n.left_down(4500, (122, 150), false, Some(staff(1)));
        assert!(e.is_empty());
        // A refused click stamps the time: 450 ms later a click is accepted
        // by the gate, one 350 ms after that is ignored.
        let (mut d, _) = open_obj(true);
        d.left_down(2000, (150, 150), false, Some(staff(9)));
        d.left_down(2450, (10, 10), false, None);
        assert_eq!(d.last_click, 2450);
        d.step = Step::Open;
        let (e, _) = d.left_down(2800, (150, 150), false, Some(staff(9)));
        assert!(e.is_empty() && d.step == Step::Open);
        // A press inside a button arms it (sound 1).
        let (e, _) = n.left_down(5000, (130, 230), false, None);
        assert_eq!(e, vec![SocketEffect::Sound(1)]);
        assert!(n.buttons[0].armed && n.buttons[0].pressed);
        // Waiting: the button loop is skipped.
        let (mut w3, _) = open_obj(true);
        w3.step = Step::Waiting;
        assert!(w3.left_down(5000, (130, 230), false, None).0.is_empty());
    }

    // Test vector "item dialog step 2, mode 1, NPC GUID 5, item GUID 9,
    // button 0 released".
    #[test]
    fn buttons_send_and_close() {
        let mut n = SocketDialog::new();
        n.open(Mode::Npc, 0, (5, 511), Some(1), true, 0);
        n.step = Step::Placed;
        n.placed = 9;
        n.left_down(5000, (130, 230), false, None);
        let e = n.left_up((130, 230), Some(1), &strings, &Fixed);
        assert_eq!(
            e,
            vec![
                SocketEffect::Sound(1),
                SocketEffect::Sound(2),
                SocketEffect::Send(ClientIntent(vec![0x38, 0, 0, 0, 0, 5, 0, 0, 0, 9, 0, 0, 0])),
            ]
        );
        assert_eq!(n.step, Step::Waiting);
        // The note box: p5 = 1, p9 = 1, style 0, anchor (150, 130), one
        // item (3353), height 15, font 1, color 0, not selectable.
        let b = n.note.as_ref().unwrap();
        assert_eq!(
            (b.anchor, b.style, b.params.p5, b.params.p9),
            ((150, 130), 0, true, 1)
        );
        assert_eq!(b.items.len(), 1);
        assert_eq!(b.items[0].text, w("s3353"));
        assert_eq!(
            (
                b.items[0].height,
                b.items[0].font,
                b.items[0].color,
                b.items[0].selectable
            ),
            (15, 1, 0, false)
        );
        // Every button is disarmed and released.
        assert!(n.buttons.iter().all(|b| !b.armed && !b.pressed));
        // Not in step 3: Esc / Space and button 1 do nothing.
        assert!(n.esc_or_space(Some(1)).is_empty());
        // Mode 0: C→S 0x44 [player][object][placed][3].
        let (mut d, _) = open_obj(true);
        d.step = Step::Placed;
        d.placed = 9;
        d.left_down(5000, (130, 230), false, None);
        let e = d.left_up((130, 230), Some(1), &strings, &Fixed);
        assert!(e.contains(&SocketEffect::Send(msg_u32s(0x44, &[1, 7, 9, 3]))));
        // Button 0 outside step 2: nothing but the release sound.
        let (mut d, _) = open_obj(true);
        d.left_down(5000, (130, 230), false, None);
        assert_eq!(
            d.left_up((130, 230), Some(1), &strings, &Fixed),
            vec![SocketEffect::Sound(1)]
        );
        // A release outside the armed button: no sound, nothing.
        d.left_down(5000, (130, 230), false, None);
        assert!(d.left_up((300, 300), Some(1), &strings, &Fixed).is_empty());
        assert!(d.buttons.iter().all(|b| !b.armed && !b.pressed));
        // Button 1 (and Esc), step 2, mode 0: the item back on the cursor,
        // sound 1, 0x44 [player][object][0][2], close.
        let (mut d, _) = open_obj(true);
        d.step = Step::Placed;
        d.placed = 9;
        let e = d.esc_or_space(Some(1));
        assert_eq!(e[0], SocketEffect::ItemToCursor);
        assert_eq!(e[1], SocketEffect::Sound(1));
        assert_eq!(e[2], SocketEffect::Send(msg_u32s(0x44, &[1, 7, 0, 2])));
        assert_eq!((d.step, d.placed), (Step::Closed, 0));
        assert!(e.contains(&ui(1)));
        // Mode 1: end the interaction and close.
        let mut n = SocketDialog::new();
        n.open(Mode::Npc, 0, (5, 154), Some(1), true, 0);
        n.left_down(5000, (180, 230), false, None);
        let e = n.left_up((180, 230), Some(1), &strings, &Fixed);
        assert!(e.contains(&SocketEffect::EndInteraction));
        assert_eq!(n.step, Step::Closed);
        assert_eq!(SocketDialog::hit_rect(1), Ltrb::new(177, 224, 210, 257));
    }
}
