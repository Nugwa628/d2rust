//! Gold amounts, the gold buttons and the gold dialog as plain state and
//! decisions: the gold line positions and hit rectangles,
//! the dialog kinds, open and close, and the dialog's controls: the
//! digits-only edit box, the spinner, the OK / Cancel buttons and the box input
//! rules. Coordinates are the 640 × 480 values the game passes.

use super::geom::Point;

// ---- gold line (panels-2 r1–r5) -----------------------------------

/// The three gold lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GoldLine {
    /// k = 0 (stash modes): stat 15.
    Stash,
    /// k = 1 (inventory): stat 14.
    Inventory,
    /// k = 2 (NPC trade): stat 15, right-aligned, no button.
    Shop,
}

/// Panel offsets and screen size the positions are relative to.
#[derive(Clone, Copy, Debug)]
pub struct GoldLayout {
    pub sx: i32,
    pub sy: i32,
    pub w: i32,
    pub h: i32,
    pub expansion: bool,
}

impl GoldLine {
    pub fn stat(self) -> u16 {
        match self {
            GoldLine::Inventory => 14,
            _ => 15,
        }
    }

    /// The value pen. For the shop the pen is right-aligned: x =
    /// `sx + 198 −` width A of the value, so `value_width` is needed.
    pub fn value_pen(self, l: &GoldLayout, value_width: i32) -> Point {
        match self {
            GoldLine::Stash => {
                Point::new(l.sx + 165, l.h + l.sy - if l.expansion { 440 } else { 244 })
            }
            GoldLine::Inventory => Point::new(l.w - l.sx - 212, l.h + l.sy - 72),
            GoldLine::Shop => Point::new(l.sx + 198 - value_width, l.h + l.sy - 106),
        }
    }

    /// The button cel position for pressed state `p`; the shop
    /// has none.
    pub fn button(self, l: &GoldLayout, pressed: bool) -> Option<Point> {
        let p = i32::from(pressed);
        match self {
            GoldLine::Stash => Some(Point::new(
                l.sx + 75,
                l.h + l.sy - if l.expansion { 440 } else { 244 } + p,
            )),
            GoldLine::Inventory => Some(Point::new(l.w - l.sx - 236, l.h + l.sy - 71 + p)),
            GoldLine::Shop => None,
        }
    }
}

/// The "Stash" label of the shop line (string 3315) position.
pub fn shop_label_pos(l: &GoldLayout) -> Point {
    Point::new(l.sx + 21, l.h + l.sy - 106)
}

/// The value text: `%d` of the full stat.
pub fn gold_text(v: i32) -> String {
    v.to_string()
}

/// Gold button frame index: pressed = frame 1, drawn one row lower.
pub fn gold_button_frame(pressed: bool) -> u32 {
    u32::from(pressed)
}

/// The stash hover test and tool tip: x in [sx + 73, sx + 93],
/// y in [Y − 18, Y] inclusive; the tool tip 4124 is queued at (sx + 53,
/// H + sy − 458 / − 262).
pub fn stash_hover(l: &GoldLayout, mouse: Point) -> Option<(u32, Point)> {
    let y = l.h + l.sy - if l.expansion { 438 } else { 242 };
    ((l.sx + 73..=l.sx + 93).contains(&mouse.x) && (y - 18..=y).contains(&mouse.y)).then(|| {
        (
            4124,
            Point::new(l.sx + 53, l.h + l.sy - if l.expansion { 458 } else { 262 }),
        )
    })
}

/// The inventory gold button hit rectangle: no
/// cursor item and x in [W − sx − 237, W − sx − 217], y in [H + sy − 87,
/// H + sy − 69] (inclusive).
pub fn inventory_gold_hit(l: &GoldLayout, mouse: Point, cursor_item: bool) -> bool {
    !cursor_item
        && (l.w - l.sx - 237..=l.w - l.sx - 217).contains(&mouse.x)
        && (l.h + l.sy - 87..=l.h + l.sy - 69).contains(&mouse.y)
}

/// The stash gold button rectangle (the same box as its hover test).
pub fn stash_gold_hit(l: &GoldLayout, mouse: Point, cursor_item: bool) -> bool {
    !cursor_item && stash_hover(l, mouse).is_some()
}

/// The gold button flags and the dialog-open flag.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GoldButtons {
    pub inv_pressed: bool,
    pub stash_pressed: bool,
    /// A gold dialog of kinds 1–4 is open.
    pub dialog_flag: bool,
}

/// What a press did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GoldPress {
    pub consumed: bool,
    /// UI sound 4.
    pub sound: Option<u16>,
}

impl GoldButtons {
    /// Inventory mode 0 press: a press in the rectangle sets the
    /// flag, plays sound 4, consumed.
    pub fn press_inventory(&mut self, in_rect: bool) -> GoldPress {
        if in_rect {
            self.inv_pressed = true;
            GoldPress {
                consumed: true,
                sound: Some(4),
            }
        } else {
            GoldPress {
                consumed: false,
                sound: None,
            }
        }
    }

    /// Inventory mode 0 release: the
    /// pressed flag set and the inventory mode 0 → cleared, and a release
    /// in the rectangle with the dialog flag clear opens kind 1.
    pub fn release_inventory(&mut self, in_rect: bool) -> Option<GoldKind> {
        if !self.inv_pressed {
            return None;
        }
        self.inv_pressed = false;
        (in_rect && !self.dialog_flag).then_some(GoldKind::Drop)
    }

    /// Stash modes release (no pressed-flag check): in the inventory rectangle
    /// with the dialog flag clear → kind 3; then in the stash rectangle with it
    /// clear → kind 4; then, unless the mouse is over the belt, both pressed
    /// flags are cleared.
    pub fn release_stash(
        &mut self,
        in_inventory_rect: bool,
        in_stash_rect: bool,
        over_belt: bool,
    ) -> Vec<GoldKind> {
        let mut out = Vec::new();
        if in_inventory_rect && !self.dialog_flag {
            out.push(GoldKind::Deposit);
        }
        if in_stash_rect && !self.dialog_flag {
            out.push(GoldKind::Withdraw);
        }
        if !over_belt {
            self.stash_pressed = false;
            self.inv_pressed = false;
        }
        out
    }
}

// ---- dialog kinds, open, close, OK (panels-2 r6–r8) ---------------

/// The dialog kinds `k`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GoldKind {
    /// 0: the plain opener.
    Plain = 0,
    /// 1: drop.
    Drop = 1,
    /// 2: trade offer.
    Trade = 2,
    /// 3: deposit.
    Deposit = 3,
    /// 4: withdraw.
    Withdraw = 4,
}

impl GoldKind {
    /// k ≥ 5 closes at once (no dialog).
    pub fn from_u8(k: u8) -> Option<GoldKind> {
        Some(match k {
            0 => GoldKind::Plain,
            1 => GoldKind::Drop,
            2 => GoldKind::Trade,
            3 => GoldKind::Deposit,
            4 => GoldKind::Withdraw,
            _ => return None,
        })
    }

    /// The prompt string id.
    pub fn prompt_id(self) -> u32 {
        match self {
            GoldKind::Plain | GoldKind::Drop => 4033,
            GoldKind::Trade => 4046,
            GoldKind::Deposit => 4049,
            GoldKind::Withdraw => 4050,
        }
    }

    /// The stat of the maximum: stash gold (15) for kind 4, else gold (14).
    pub fn max_stat(self) -> u16 {
        if self == GoldKind::Withdraw {
            15
        } else {
            14
        }
    }

    /// Kind 3 pre-fills the edit box with the maximum.
    pub fn prefills(self) -> bool {
        self == GoldKind::Deposit
    }

    /// Kinds 1–4 set the dialog flag:= 1 when opened.
    pub fn sets_dialog_flag(self) -> bool {
        self != GoldKind::Plain
    }
}

/// The opener call sites.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GoldOpener {
    /// Inventory gold button release.
    InventoryButton {
        stash_open: bool,
        cube_open: bool,
    },
    StashButton,
    PlayerTrade,
    Other,
}

pub fn opener_kind(o: GoldOpener) -> GoldKind {
    match o {
        GoldOpener::InventoryButton {
            stash_open: true, ..
        } => GoldKind::Deposit,
        GoldOpener::InventoryButton { .. } => GoldKind::Drop,
        GoldOpener::StashButton => GoldKind::Withdraw,
        GoldOpener::PlayerTrade => GoldKind::Trade,
        GoldOpener::Other => GoldKind::Plain,
    }
}

/// The 640 × 480 positions of the dialog's pieces.
pub mod pos {
    use super::Point;
    pub const BOX: Point = Point::new(215, 140);
    /// Box art `dialogbackground`: 210 × 158.
    pub const BOX_SIZE: (i32, i32) = (210, 158);
    pub const SPINNER: Point = Point::new(223, 219);
    pub const EDIT: Point = Point::new(258, 228);
    pub const EDIT_WIDTH: i32 = 100;
    pub const EDIT_CAP: usize = 10;
    pub const OK: Point = Point::new(250, 287);
    pub const CANCEL: Point = Point::new(355, 287);
    /// The prompt wraps at this width.
    pub const PROMPT_WIDTH: i32 = 200;
}

/// The conditions to open: a player, no
/// cursor item and no other dialog open.
pub fn can_open(has_player: bool, cursor_item: bool, dialog_open: bool) -> bool {
    has_player && !cursor_item && !dialog_open
}

/// What to do after the OK button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GoldSend {
    Nothing,
    /// C→S 0x50 [player GUID (−1 without a player), amount] (kinds 0, 1).
    DropGold {
        guid: u32,
        amount: u32,
    },
    /// Player trade, with sound 0xDD unless the amount is 0.
    TradeOffer {
        sound: Option<u16>,
    },
    /// C→S 0x4F [button, v >> 16, v & 0xFFFF], sound 0xDD.
    StashButton {
        button: u16,
        p1: u16,
        p2: u16,
        sound: u16,
    },
}

/// OK: with `v` the value after close.
pub fn ok_action(kind: GoldKind, v: u32, player_guid: Option<u32>) -> GoldSend {
    match kind {
        GoldKind::Trade => GoldSend::TradeOffer {
            sound: (v != 0).then_some(0xDD),
        },
        _ if v == 0 => GoldSend::Nothing,
        GoldKind::Plain | GoldKind::Drop => GoldSend::DropGold {
            guid: player_guid.unwrap_or(u32::MAX),
            amount: v,
        },
        GoldKind::Deposit | GoldKind::Withdraw => GoldSend::StashButton {
            button: if kind == GoldKind::Deposit {
                0x14
            } else {
                0x13
            },
            p1: (v >> 16) as u16,
            p2: (v & 0xFFFF) as u16,
            sound: 0xDD,
        },
    }
}

/// The open dialog (kinds 0–4).
#[derive(Clone, Debug)]
pub struct GoldDialog {
    pub kind: GoldKind,
    /// The amount in the box.
    pub amount: u32,
    pub max: u32,
    pub edit: DigitEdit,
    pub spinner: Spinner,
    /// The key-mode latch.
    pub latch: bool,
}

/// What opening did to the world.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpenEffects {
    /// Key mode 0 with key-up kept, only when the
    /// latch was 0.
    pub key_mode_0: bool,
    /// Chat box closed, the dialog flag:= 1, mode 0x0D → 0x0C.
    pub set_dialog_flag: bool,
}

impl GoldDialog {
    /// Opens a dialog of `kind` with `max` = the player's full stat.
    pub fn open(kind: GoldKind, max: u32, latch_was_set: bool) -> (GoldDialog, OpenEffects) {
        let mut edit = DigitEdit::new(max);
        if kind.prefills() {
            edit.set_value(max);
        }
        let d = GoldDialog {
            kind,
            amount: 0,
            max,
            edit,
            spinner: Spinner::default(),
            latch: true,
        };
        (
            d,
            OpenEffects {
                key_mode_0: !latch_was_set,
                set_dialog_flag: kind.sets_dialog_flag(),
            },
        )
    }

    /// Close: the latch is cleared (key mode 1) and the value is the edit
    /// box's. Kinds 1–4 clear.
    pub fn close(&mut self) -> u32 {
        self.latch = false;
        self.amount = self.edit.value();
        self.amount
    }

    /// The spinner callback: the amount is the edit
    /// box value; up: min(v + step, m); down: 0 when step > v else v −
    /// step; the edit box gets the new value.
    pub fn spinner_apply(&mut self, dir: SpinDir, step: u32) {
        let mut v = self.edit.value();
        match dir {
            SpinDir::Up => v = v.saturating_add(step).min(self.max),
            SpinDir::Down => v = v.saturating_sub(step),
            SpinDir::None => {}
        }
        self.amount = v;
        self.edit.set_value(v);
    }
}

// ---- the digits-only edit box (panels-3) ------------------------

/// Result of a character: `Some(true)` consumed (returns 1), `Some(false)`
/// leaves it to the box (returns 0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CharResult {
    Consumed,
    /// Space, Esc, CR: left to the box.
    LeftToBox,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DigitEdit {
    /// Text units (digits).
    pub text: Vec<u8>,
    pub caret: usize,
    pub max: u32,
    /// Key repeat allowed (+0x24, 1 after creation).
    pub key_repeat: bool,
    /// Blink counter (+0x46) and visible flag (+0x4A).
    pub counter: u32,
    pub visible: bool,
    pub pressed: bool,
}

impl DigitEdit {
    pub fn new(max: u32) -> Self {
        Self {
            text: Vec::new(),
            caret: 0,
            max,
            key_repeat: true,
            counter: 0,
            visible: true,
            pressed: false,
        }
    }

    /// Value (method 11): empty → 0; else `atol(text)`, 0 when > max.
    pub fn value(&self) -> u32 {
        if self.text.is_empty() {
            return 0;
        }
        let v = self.parse();
        if v > u64::from(self.max) {
            0
        } else {
            v as u32
        }
    }

    fn parse(&self) -> u64 {
        self.text.iter().fold(0u64, |a, d| {
            (a * 10 + u64::from(d - b'0')).min(u64::from(u32::MAX) + 1)
        })
    }

    /// Set value (method 15): `%d`, cut to cap units, caret:= length.
    pub fn set_value(&mut self, v: u32) {
        let mut s = v.to_string().into_bytes();
        s.truncate(pos::EDIT_CAP);
        self.caret = s.len();
        self.text = s;
    }

    /// Draw step (method 1): the counter and the blink: visible turns off
    /// when counter % 20 = 0; hidden turns on when counter % 10 = 0.
    pub fn blink(&mut self) {
        self.counter += 1;
        if self.visible {
            if self.counter.is_multiple_of(20) {
                self.visible = false;
            }
        } else if self.counter.is_multiple_of(10) {
            self.visible = true;
        }
    }

    /// The caret is drawn when caret < cap, visible and caret ≤ length.
    pub fn caret_drawn(&self) -> bool {
        self.caret < pos::EDIT_CAP && self.visible && self.caret <= self.text.len()
    }

    /// WM_CHAR (method 6). `repeat`: lParam bit 30; `wraps`: the text
    /// wraps to more lines than the box has at its width.
    pub fn char(&mut self, c: u32, repeat: bool, wraps: &dyn Fn(&[u8]) -> bool) -> CharResult {
        if repeat && !self.key_repeat {
            return CharResult::Consumed;
        }
        if c > 0x7F {
            return CharResult::Consumed;
        }
        let c = c as u8;
        let ok = c.is_ascii_digit();
        let len = self.text.len();
        if c == 8 {
            if len > 0 {
                if self.caret != len && self.caret != 0 {
                    self.text.remove(self.caret - 1);
                } else {
                    // with the caret at 0 the last character goes
                    self.text.pop();
                }
                if self.caret > 0 {
                    self.caret -= 1;
                }
            }
            return CharResult::Consumed;
        }
        if c == b'.' {
            if len > 0 && self.caret != len {
                self.text.remove(self.caret);
            }
            return CharResult::Consumed;
        }
        if !ok {
            return if matches!(c, b' ' | 0x1B | 0x0D) {
                CharResult::LeftToBox
            } else {
                CharResult::Consumed
            };
        }
        if len < pos::EDIT_CAP - 1 {
            if self.caret == len {
                self.text.push(c);
                self.caret += 1;
            } else {
                // shift right from the caret, c at the caret; the caret
                // does not advance
                self.text.insert(self.caret, c);
            }
            if wraps(&self.text) {
                self.text.pop();
                self.caret = self.caret.saturating_sub(1);
            }
            let v = self.parse();
            if v > u64::from(self.max) {
                self.set_value(self.max);
            }
        }
        CharResult::Consumed
    }

    /// Right / Left (methods 9 / 10): taken.
    pub fn arrow(&mut self, right: bool) {
        if right {
            if self.caret < self.text.len() {
                self.caret += 1;
            }
        } else if self.caret > 0 {
            self.caret -= 1;
        }
    }

    /// Mouse down (method 3): pressed:= 1, not taken. Mouse up (method 4): when
    /// pressed the blink counter:= 0 and pressed:= 0; not taken.
    pub fn mouse_down(&mut self) {
        self.pressed = true;
    }

    pub fn mouse_up(&mut self) {
        if self.pressed {
            self.counter = 0;
            self.pressed = false;
        }
    }
}

// ---- the spinner (panels-3) -------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpinDir {
    None,
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Spinner {
    pub up: bool,
    pub down: bool,
    pub pulse: bool,
    pub held: bool,
    /// Draws since the press.
    pub n: u32,
    pub t0: u32,
    pub t1: u32,
    first: bool,
}

/// Arrow cel size (`ui\menu\spinner`, 4 frames of 15 × 12).
pub const SPINNER_FRAME: (i32, i32) = (15, 12);

impl Spinner {
    /// The up and down arrow points (orientation 0).
    pub fn arrow_points(origin: Point) -> (Point, Point) {
        let (_, h) = SPINNER_FRAME;
        (
            Point::new(origin.x + 1, origin.y),
            Point::new(origin.x + 1, origin.y + h + 1),
        )
    }

    /// Draw (method 1): while an arrow is pressed n += 1 and the callback
    /// runs before drawing; the frames drawn are 2 × up at the up point
    /// and 2 × down + 1 at the down point.
    pub fn draw_frames(&mut self) -> (u32, u32, bool) {
        let cb = self.up || self.down;
        if cb {
            self.n += 1;
        }
        (2 * u32::from(self.up), 2 * u32::from(self.down) + 1, cb)
    }

    /// Mouse down (method 3): up rectangle x in (ux, ux + w), y in (uy −
    /// h, uy]; down rectangle x in (dx, dx + w), y in (dy − h, dy)
    /// (exclusive). Returns whether it was taken.
    pub fn mouse_down(&mut self, origin: Point, mouse: Point, now: u32) -> bool {
        self.held = true;
        let (w, h) = SPINNER_FRAME;
        let (u, d) = Self::arrow_points(origin);
        if mouse.x > u.x && mouse.x < u.x + w && mouse.y > u.y - h && mouse.y <= u.y {
            self.t0 = now;
            self.up = true;
            self.first = true;
            true
        } else if mouse.x > d.x && mouse.x < d.x + w && mouse.y > d.y - h && mouse.y < d.y {
            self.t0 = now;
            self.down = true;
            self.first = true;
            true
        } else {
            false
        }
    }

    /// Mouse up (method 4): when held, n:= 0, held:= 0, clears up (else down)
    /// and returns 1 when one was set.
    pub fn mouse_up(&mut self) -> bool {
        if !self.held {
            return false;
        }
        self.n = 0;
        self.held = false;
        if self.up {
            self.up = false;
            true
        } else if self.down {
            self.down = false;
            true
        } else {
            false
        }
    }

    /// Wheel (method 2): delta > 0 up, < 0 down; either sets the pulse;
    /// consumed.
    pub fn wheel(&mut self, delta: i32) -> bool {
        if delta > 0 {
            self.up = true;
            self.down = false;
        } else if delta < 0 {
            self.down = true;
            self.up = false;
        }
        if delta != 0 {
            self.pulse = true;
        }
        true
    }

    /// Direction (method 11).
    pub fn direction(&self) -> SpinDir {
        if self.up {
            SpinDir::Up
        } else if self.down {
            SpinDir::Down
        } else {
            SpinDir::None
        }
    }

    /// Step (method 12): none pressed → 0; pulse → clears up, down and pulse,
    /// 1; first → t1:= now, 1; now − t1 < 70 ms → 0; else t1:= now and, with d
    /// = now − t0: d > 4096 → (d >> 11) × n; d > 3000 → n >> 2; d > 2000 → n >>
    /// 4; d > 1000 → n >> 5; else 1.
    pub fn step(&mut self, now: u32) -> u32 {
        if !self.up && !self.down {
            return 0;
        }
        if self.pulse {
            self.up = false;
            self.down = false;
            self.pulse = false;
            return 1;
        }
        if self.first {
            self.first = false;
            self.t1 = now;
            return 1;
        }
        if now.wrapping_sub(self.t1) < 70 {
            return 0;
        }
        self.t1 = now;
        let d = now.wrapping_sub(self.t0);
        if d > 4096 {
            (d >> 11) * self.n
        } else if d > 3000 {
            self.n >> 2
        } else if d > 2000 {
            self.n >> 4
        } else if d > 1000 {
            self.n >> 5
        } else {
            1
        }
    }
}

// ---- buttons (panels-3) -----------------------------------------

/// OK / Cancel: art `buysellbtn`, 32 × 32 frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DialogButton {
    pub origin: Point,
    /// 0 = OK (frame 16, string 3401), 1 = Cancel (frame 10, 4142).
    pub kind: u8,
    pub pressed: bool,
    pub enter_enabled: bool,
}

pub const BUTTON_SIZE: i32 = 32;

impl DialogButton {
    pub fn ok() -> Self {
        Self {
            origin: pos::OK,
            kind: 0,
            pressed: false,
            enter_enabled: true,
        }
    }

    pub fn cancel() -> Self {
        Self {
            origin: pos::CANCEL,
            kind: 1,
            pressed: false,
            enter_enabled: false,
        }
    }

    /// Frame (pressed adds 1): OK 16 / 17, Cancel 10 / 11.
    pub fn frame(&self) -> u32 {
        (if self.kind == 0 { 16 } else { 10 }) + u32::from(self.pressed)
    }

    pub fn string_id(&self) -> u32 {
        if self.kind == 0 {
            3401
        } else {
            4142
        }
    }

    /// Rectangle: x in [x, x + w], y in [y − h, y].
    pub fn contains(&self, p: Point) -> bool {
        (self.origin.x..=self.origin.x + BUTTON_SIZE).contains(&p.x)
            && (self.origin.y - BUTTON_SIZE..=self.origin.y).contains(&p.y)
    }

    /// The caption position when the cursor is inside: (x + (w − width
    /// A) / 2, y − h − 5), C division.
    pub fn caption_pos(&self, width_a: i32) -> Point {
        Point::new(
            self.origin.x + (BUTTON_SIZE - width_a) / 2,
            self.origin.y - BUTTON_SIZE - 5,
        )
    }

    /// Mouse down: inside → pressed, UI sound 4, taken.
    pub fn mouse_down(&mut self, p: Point) -> Option<u16> {
        if self.contains(p) {
            self.pressed = true;
            Some(4)
        } else {
            None
        }
    }

    /// Mouse up: not pressed → false; inside → pressed:= 0 and the callback
    /// runs (returns true); outside → pressed:= 0, false.
    pub fn mouse_up(&mut self, p: Point) -> bool {
        let was = self.pressed;
        self.pressed = false;
        was && self.contains(p)
    }

    /// WM_CHAR: Enter enabled, not a repeat and character 0x0D → the
    /// callback.
    pub fn char(&self, c: u32, repeat: bool) -> bool {
        self.enter_enabled && !repeat && c == 0x0D
    }
}

// ---- the box input (panels-3) ----------------------------

/// The order controls are offered events: the list is prepended to, so
/// with the add order spinner, edit box, OK, Cancel it is Cancel, OK, edit
/// box, spinner.
pub const CONTROL_ORDER: [&str; 4] = ["cancel", "ok", "edit", "spinner"];

/// Events within this long of the box's creation are only consumed.
pub const GRACE_MS: u32 = 80;

pub fn in_grace(now: u32, created: u32) -> bool {
    now.wrapping_sub(created) < GRACE_MS
}

/// What a mouse down outside every control does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoxDown {
    /// A control took it.
    Control,
    /// Passes on untouched (mini panel strip).
    PassOn,
    /// A click outside the box (grown by 4 pixels): close.
    Close,
    /// Consumed (pressed flag set).
    Consumed,
}

pub fn box_mouse_down(control_took: bool, outside_grown_box: bool, in_mini_strip: bool) -> BoxDown {
    if control_took {
        BoxDown::Control
    } else if outside_grown_box {
        if in_mini_strip {
            BoxDown::PassOn
        } else {
            BoxDown::Close
        }
    } else {
        BoxDown::Consumed
    }
}

/// Esc key down: nothing while the chat is open; otherwise
/// consumed and, once the box was drawn more than 4 times, close.
pub fn box_esc(chat_open: bool, draws: u32) -> (bool, bool) {
    if chat_open {
        (false, false)
    } else {
        (true, draws > 4)
    }
}

/// The mini panel strip: W/2 − 74 < x < W/2 + 71,
/// H − 69 < y < H − 50.
pub fn in_mini_strip(w: i32, h: i32, p: Point) -> bool {
    p.x > w / 2 - 74 && p.x < w / 2 + 71 && p.y > h - 69 && p.y < h - 50
}

/// Wheel on the box: |delta| ≥ 120 → offer method 2.
pub fn wheel_offered(delta: i32) -> bool {
    delta.abs() >= 120
}

#[cfg(test)]
mod tests;
