//! The control panel overlays in play ([`HudUi`]): the rules of
//! `ui::panels::control` bound to the client model each frame. Drawn
//! after the border and base art: life and mana globes,
//! experience and stamina bars, run / walk and menu buttons,
//! the skill buttons with the selected skills' icons, the mini panel
//! with state 0x15 open; the mouse input (run toggle, menu
//! button, skill buttons → state 3, mini panel functions).
//!
//! The new-stats / new-skills buttons (800 × 600) draw `Panel\Level`: the
//! glowing button while stat (skill) points are unspent, frame 2 otherwise; a
//! click opens the character panel (state 2) / skill tree (state 4). D2RUST's
//! own, unverified: states 6 and 7 are not driven by the level-up; the unspent
//! points (stats 4 and 5 of the model) stand for them.
//!
//! The belt is `hud_belt`. Not drawn: the 640 × 480 variant of the buttons; the
//! tool tips, globe numbers and the stamina tip are `hud_tips`.
//!
//! Preview fills (decision D1), each `// D2RUST's own, unverified`:
//! - the bars' lines and rectangle are cels of a synthetic one-colour file
//!   ([`FILL_FILE`]) clipped to the bar (no line / rectangle draw in the
//!   UI draw list); the stamina colour is the palette's nearest index;
//! - the skill select panel's input (state 3; its draw is `skill_select`): a click
//!   on an icon sends C→S 0x3C and closes state 3; the panel's hover
//!   overlays are not drawn.

use std::collections::BTreeMap;

use super::{class_u8, left, SharedRef, EMPTY};
use crate::bridge::world::{ClientWorld, UnitKey, PLAYER};
use crate::ui::draw::{ImageRef, ImageRequest, UiDraw, UiDrawSink};
use crate::ui::geom::{Point, Rect};
use crate::ui::layout::Screen;
use crate::ui::panel::{Panel, PanelId, UiCtx, UiEvent, UiResponse, WidgetId};
use crate::ui::panels::control::belt::BeltColor;
use crate::ui::panels::control::buttons::{
    draw_800, help_caption, menu_button, run_button, skill_icon_file, skill_icon_pos,
    skill_icon_state, BtnEffect, BtnEnv, ButtonCel, HelpButton, NewBtn, NewButtons, SkillSide,
    UI_HELP_BUTTON,
};
use crate::ui::panels::control::globes::{
    exp_bar, life_globe, mana_globe, stamina_bar, ExpIn, GlobeDraw, GlobeFile, GlobeSmoothing,
    LifeIn, ManaIn, NumbersIn, StaminaColor, StaminaIn, TextToggle,
};
use crate::ui::panels::control::input::{CtrlEffect, CtrlInput, InputEnv, UpFacts};
use crate::ui::panels::control::minipanel::{self, MiniAction, MiniPanel, PlayerFacts, UI_MINI};
use crate::ui::panels::{PanelOutput, UiFiles};

/// The HUD adapter's id: not a UI state, open for good (like the border).
pub const HUD_PANEL: PanelId = PanelId(0x101);

/// The skill select state.
pub const UI_SKILL_SELECT: u8 = 3;

/// The synthetic one-colour file of the bars (module doc). Frames: 0 red,
/// 1 gold, 2 blue (stamina), 3 index 0xFF (experience), 4 the
/// darkest colour (the Esc menu's box), 5–8 the rectangle primitive's belt
/// colours (red, green, blue, yellow: [`BELT_FILL_BASE`]).
/// The first of the four rectangle colours (`BeltColor` order).
pub const BELT_FILL_BASE: u32 = 5;
/// The fill frame of the unidentified tint (tint index 4).
pub const UNIDENTIFIED_FILL: u32 = 9;
pub const FILL_FILE: &str = "d2rust\\hudfill";
/// Each fill frame is this wide and high (covers both bars).
pub const FILL_W: u32 = 128;
pub const FILL_H: u32 = 18;

/// The files the HUD names, beside the layout's (lowercase, `UiFiles`).
pub fn hud_files() -> Vec<String> {
    let mut v: Vec<String> = [
        "panel\\hlthmana",
        "panel\\overlap",
        "panel\\runbutton",
        "panel\\menubutton",
        "panel\\level",
        "panel\\levelsocket",
        "panel\\minipanel",
        "panel\\minipanel_s",
        "panel\\minipanelbtn",
        "panel\\ctrlpnl_popbelt",
        "panel\\invwarn",
        "menu\\800helpborder",
        "panel\\buysellbtn",
        "menu\\helpyellowbullet",
        "menu\\helpwhitebullet",
        FILL_FILE,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    for c in 0..8u8 {
        v.push(skill_icon_file(c).to_ascii_lowercase());
    }
    v
}

/// Whether a missing archive file of `name` may draw nothing instead of
/// failing the frame (D1: HUD art only).
pub fn optional_file(name: &str) -> bool {
    hud_files().iter().any(|f| f == name) || super::esc_art::is_esc_file(name)
}

/// The fill frames for `palette` (module doc). D2RUST's own, unverified.
pub fn fill_frames(palette: &d2_formats::palette::Palette) -> Vec<crate::frames::IndexFrame> {
    let nearest = |(r, g, b): (u8, u8, u8)| -> u8 {
        let mut best = (u32::MAX, 0u8);
        // Index 0 is transparent: never chosen.
        for (i, c) in palette.colors.iter().enumerate().skip(1) {
            let d = |a: u8, b: u8| (i32::from(a) - i32::from(b)).pow(2) as u32;
            let dist = d(c.r, r) + d(c.g, g) + d(c.b, b);
            if dist < best.0 {
                best = (dist, i as u8);
            }
        }
        best.1
    };
    let colors = [
        nearest(StaminaColor::Red.rgb()),
        nearest(StaminaColor::Gold.rgb()),
        nearest(StaminaColor::Blue.rgb()),
        0xFF,
        nearest((0, 0, 0)),
        nearest(BeltColor::Red.rgb()),
        nearest(BeltColor::Green.rgb()),
        nearest(BeltColor::Blue.rgb()),
        nearest(BeltColor::Yellow.rgb()),
        nearest((0x80, 0x40, 0x40)),
    ];
    colors
        .iter()
        .map(|&c| {
            crate::frames::IndexFrame::new(
                FILL_W,
                FILL_H,
                0,
                0,
                vec![c; (FILL_W * FILL_H) as usize],
            )
            .expect("fill size")
        })
        .collect()
}

/// The tables the HUD reads (from the user's `skills`, `skilldesc` and
/// `experience`; empty without game files).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HudTables {
    /// Skill id → (`charclass`, skilldesc `IconCel`).
    pub icons: BTreeMap<u16, (u8, u8)>,
    /// `experience` rows: row 0 `MaxLvl`, row L + 1 level L, 7 classes.
    pub experience: Vec<[u32; 7]>,
    /// The states with flag bit 24 (`stambarblue`).
    pub stambarblue: Vec<u8>,
    /// Skill id → the `skills` flags the button state reads.
    pub flags: BTreeMap<u16, SkillButtonFlags>,
    /// Skill id → its `skilldesc` text ids (the hover box, `skill_hover`).
    pub descs: BTreeMap<u16, super::skill_hover::SkillText>,
    /// The item tables of the equipment warning icons.
    pub warn: WarnTables,
    /// The summon and curse tables of the hover box's lines
    /// (`skill_hover_lines`).
    pub hover: super::skill_hover_lines::HoverTables,
}

/// One `itemtypes` row as the warning icons read it: `Shoots` and the two
/// `equiv` links.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WarnType {
    pub shoots: u16,
    pub equiv1: u16,
    pub equiv2: u16,
    /// `throwable` (the type row's byte at +0x10).
    pub throwable: bool,
}

/// One item row (`weapons`, `armor`, `misc`) as the warning icons read it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WarnItem {
    pub type1: u16,
    pub type2: u16,
    /// A `weapons.txt` row.
    pub weapon: bool,
    /// `stackable` (the row's byte at +0x132).
    pub stackable: bool,
}

/// The tables of the equipment warning icons:
/// the `itemtypes` rows and the item rows by code.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WarnTables {
    pub itypes: Vec<WarnType>,
    pub items: BTreeMap<[u8; 4], WarnItem>,
}

impl WarnTables {
    /// A type row number as a link: 0 and 0xFFFF are "none".
    fn some(t: u16) -> Option<usize> {
        (t != 0 && t != 0xFFFF).then_some(usize::from(t))
    }

    /// `itype` is `parent` or nests under it through the `equiv` links.
    fn type_is(&self, itype: u16, parent: u16) -> bool {
        let mut todo = vec![itype];
        for _ in 0..256 {
            let Some(t) = todo.pop() else {
                return false;
            };
            if t == parent {
                return true;
            }
            if let Some(r) = Self::some(t).and_then(|i| self.itypes.get(i)) {
                todo.extend([r.equiv1, r.equiv2].into_iter().filter(|&e| e != 0xFFFF));
            }
        }
        false
    }

    /// The type `item` shoots (0 = none).
    fn shoots(&self, item: WarnItem) -> u16 {
        Self::some(item.type1)
            .and_then(|i| self.itypes.get(i))
            .map_or(0, |t| t.shoots)
    }

    /// The item's type is `throwable`.
    fn throwable(&self, item: WarnItem) -> bool {
        Self::some(item.type1)
            .and_then(|i| self.itypes.get(i))
            .is_some_and(|t| t.throwable)
    }

    /// The item is of type `t` by either of its two types.
    fn item_is(&self, item: WarnItem, t: u16) -> bool {
        self.type_is(item.type1, t) || (item.type2 != 0 && self.type_is(item.type2, t))
    }
}

/// Whether the missing-ammunition icon (`invwarn.dc6` frame 2) is drawn for the
/// items in the right (4) and left (5) hand.
pub fn ammo_warning(t: &WarnTables, right: Option<WarnItem>, left: Option<WarnItem>) -> bool {
    let shoots = |i: Option<WarnItem>| i.map_or(0, |i| t.shoots(i));
    let left_needs_ammo = |t: &WarnTables| match left {
        Some(l) if t.shoots(l) != 0 => !right.is_some_and(|r| t.item_is(r, t.shoots(l))),
        _ => false,
    };
    match right {
        Some(r) if shoots(right) != 0 => match left {
            Some(l) if t.item_is(l, t.shoots(r)) => left_needs_ammo(t),
            _ => true,
        },
        _ => left_needs_ammo(t),
    }
}

/// The local player's item at body location `loc` with its table row and
/// quantity (stat 0x46, from the item stream's own values; 0 without the
/// item tables).
fn body_item(
    world: &ClientWorld,
    t: &WarnTables,
    tips: Option<&super::item_tip::ItemTips>,
    loc: u8,
) -> Option<(WarnItem, i32)> {
    use crate::bridge::items::{items, mode, stream};
    let key = local(world)?;
    let i = items(world)
        .into_iter()
        .find(|i| i.mode == mode::BODY && i.owner == Some(key) && i.body == loc)?;
    let quantity = tips
        .zip(stream(world, i.key))
        .and_then(|(tips, s)| tips.bits(s))
        .and_then(|b| b.quantity)
        .map_or(0, i32::from);
    Some((t.items.get(&i.code?).copied()?, quantity))
}

/// The frames of the equipment warning icons of the local player, in the order
/// of their slots (only the missing ammunition so far).
pub fn warning_frames(
    world: &ClientWorld,
    t: &WarnTables,
    tips: Option<&super::item_tip::ItemTips>,
) -> Vec<u32> {
    if local(world).is_none() {
        return Vec::new();
    }
    let body = |loc: u8| body_item(world, t, tips, loc).map(|(i, _)| i);
    let mut v = Vec::new();
    if ammo_warning(t, body(4), body(5)) {
        v.push(2);
    }
    v
}

/// The skill id of Left Hand Throw.
const SKILL_LEFT_HAND_THROW: u16 = 4;

/// The number a skill button prints at (x + 4, y − 1): on a Left Hand Throw
/// button the quantity of the stackable throwing weapon in the right hand, else
/// in the left (the weapon in use is not known; the Barbarian cases with a
/// throwing weapon in either hand print it that way); any other skill its item charges (0 for a
/// non-item skill). `hands` are the items of body locations 4 and 5 with their
/// quantity (stat 0x46).
///
/// The Throw button (2) is PROVISIONAL (the game prints the quantity
/// of the weapon in use there when it is a stackable throwable weapon):
/// it shows only on the right button with a potion in the right
/// hand and never with Throwing Axes, which fits a weapon in use that
/// is unset for them; to be settled once the writer of the unit's
/// current-weapon field is known.
pub fn button_count(
    t: &WarnTables,
    skill: u16,
    charges: i32,
    hands: [Option<(WarnItem, i32)>; 2],
) -> i32 {
    let [right, left] = hands;
    match skill {
        SKILL_LEFT_HAND_THROW => match right.or(left) {
            Some((i, q)) if i.weapon && t.throwable(i) && i.stackable => q,
            _ => charges,
        },
        // Throw (2): not modelled (see above).
        _ => charges,
    }
}

/// The icons as draws: slot `n` at x = screen width − 0x37 − 0x28·n, y = 0x9B
/// (six slots; a seventh icon takes slot 0's place).
pub fn warning_draws(files: &UiFiles, frames: &[u32], screen_w: i32) -> Vec<UiDraw> {
    frames
        .iter()
        .enumerate()
        .filter_map(|(n, &f)| {
            let slot = if n < 6 { n as i32 } else { 0 };
            image(
                files,
                "panel\\invwarn",
                f,
                screen_w - 0x37 - 0x28 * slot,
                0x9B,
                screen_clip(),
            )
        })
        .collect()
}

/// Panel id of the equipment warning icons (UI pass step 2).
pub const WARN_PANEL: PanelId = PanelId(0x115);
/// UI states whose open pass skips step 2: the inventory family (1, 0x0C,
/// 0x0E).
const WARN_SKIP: [u8; 3] = [1, 0x0C, 0x0E];

/// The equipment warning icons, drawn before every panel (UI pass step 2).
pub struct WarnUi {
    pub(super) sh: SharedRef,
}

impl Panel for WarnUi {
    fn id(&self) -> PanelId {
        WARN_PANEL
    }

    fn rect(&self) -> Rect {
        EMPTY
    }

    fn draw(&self, ctx: &UiCtx, out: &mut dyn UiDrawSink) {
        let sh = self.sh.borrow();
        if WARN_SKIP.iter().any(|&s| sh.states.is_open(s)) {
            return;
        }
        let frames = warning_frames(ctx.world, &sh.hud.tables.warn, sh.items.tips.as_ref());
        for d in warning_draws(&sh.tables.files, &frames, sh.config.screen.w) {
            out.push(d);
        }
    }

    fn hit(&self, _p: Point) -> Option<WidgetId> {
        None
    }

    fn event(&mut self, _e: UiEvent, _ctx: &UiCtx) -> UiResponse {
        UiResponse::Ignored
    }
}

/// The `skills` row flags of the skill button state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SkillButtonFlags {
    pub in_game: bool,
    pub aura: bool,
    pub passive: bool,
    /// `InTown` (flags bit 8).
    pub in_town: bool,
    /// `leftskill` (flags bit 30): listed by the left select panel.
    pub left_skill: bool,
    /// `scroll` (flags bit 36): a quantity skill of the right panel.
    pub scroll: bool,
    /// The `skilldesc` `ListRow` (signed byte) and `ListPool`.
    pub list_row: i8,
    pub list_pool: u8,
}

/// The state `k` of a skill button icon: the use state u 0 -> 0, aura -> 4, any
/// other -> 1; then 1 when the record lacks `InTown` and P stands in town. This
/// fallback (the character tables not loaded) runs the level, `InGame`, aura
/// and passive tests of the skill use rules only, as `app/skill_rest.rs`
/// `use_state`; the live path takes the model's use state (`model_k`: mana,
/// item, shape and cooldown tests), whose button colour also covers mana
/// below the cost and full mana. Before the flag table is filled only the level
/// test runs.
pub fn skill_button_state(
    tables: &HudTables,
    skill: u16,
    level: i32,
    in_town: bool,
    mouse: (i32, i32),
    at: (i32, i32),
) -> u8 {
    let base = use_state_k(tables, skill, level);
    let lacks_in_town = tables.flags.get(&skill).is_some_and(|f| !f.in_town);
    skill_icon_state(base, lacks_in_town, in_town, mouse, at)
}

/// The use state u as an icon colour: 0 -> 0, aura -> 4,
/// any other -> 1 (as [`skill_button_state`]).
fn use_state_k(tables: &HudTables, skill: u16, level: i32) -> u8 {
    let f = tables.flags.get(&skill).copied();
    // Not usable in game, or not learned.
    if (!tables.flags.is_empty() && !f.is_some_and(|f| f.in_game)) || level <= 0 {
        1
    } else if f.is_some_and(|f| f.aura) {
        4
    } else if f.is_some_and(|f| f.passive) {
        1
    } else {
        0
    }
}

/// One icon of the skill select panel: the skill, its cel position and
/// its colour before the hover rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectIcon {
    pub skill: u16,
    pub at: (i32, i32),
    pub k: u8,
    /// The entry's owner GUID (`NATIVE` = the unit's own skill), the
    /// record's fourth field.
    pub owner: u32,
    /// The quantity of a right-panel own skill (read only
    /// there, > 0): its hover shows the quantity pop-up.
    pub quantity: i32,
}

/// The skill select panel's icons: passes 0–3 list the unit's own skills whose
/// `ListRow` is the pass, pass 4 the item skills; x starts at 80 (left) or W −
/// 128 (right) every pass and steps 48 inward-out; y starts at H − 86 and rises
/// 48 at the first icon of a higher pass; a skill whose nonzero `ListPool` was
/// already listed in this call is skipped (after the y step). `k_of` gives an
/// entry's colour from its use state (0 -> 0, aura -> 4, any other -> 1).
pub fn select_layout(
    tables: &HudTables,
    list: &crate::bridge::skills::SkillList,
    left: bool,
    w: i32,
    h: i32,
    k_of: &mut dyn FnMut(&crate::bridge::skills::SkillEntry) -> u8,
) -> Vec<SelectIcon> {
    let mut out = Vec::new();
    let (start, step) = if left { (80, 48) } else { (w - 128, -48) };
    let mut y = h - 86;
    let mut last = 0;
    let mut pools = [false; 256];
    for pass in 0..5 {
        let mut x = start;
        for e in &list.entries {
            let Some(f) = tables.flags.get(&e.skill) else {
                continue;
            };
            if !tables.icons.contains_key(&e.skill) || (left && !f.left_skill) || f.passive {
                continue;
            }
            let item =
                e.owner != crate::bridge::skills::NATIVE || (!left && e.quantity > 0 && f.scroll);
            if (pass < 4 && (item || i32::from(f.list_row) != pass)) || (pass == 4 && !item) {
                continue;
            }
            let k = k_of(e);
            if last < pass {
                y -= 48;
                last = pass;
            }
            if f.list_pool != 0 {
                let seen = &mut pools[usize::from(f.list_pool)];
                if *seen {
                    continue;
                }
                *seen = true;
            }
            out.push(SelectIcon {
                skill: e.skill,
                at: (x, y),
                k,
                owner: e.owner,
                quantity: if !left && e.owner == crate::bridge::skills::NATIVE {
                    e.quantity.max(0)
                } else {
                    0
                },
            });
            x += step;
        }
    }
    out
}

impl HudTables {
    /// `threshold(class, L)` = row L + 1.
    fn threshold(&self, class: usize, level: u32) -> Option<u32> {
        let row = self.experience.get(level as usize + 1)?;
        row.get(class).copied()
    }

    fn max_level(&self, class: usize) -> Option<u32> {
        self.experience.first()?.get(class).copied()
    }
}

/// The HUD's own state (the control panel's).
#[derive(Clone, Debug)]
pub struct HudState {
    pub tables: HudTables,
    pub smoothing: GlobeSmoothing,
    pub input: CtrlInput,
    pub mini: MiniPanel,
    /// The run toggle as the walk holds it (set by the host each frame).
    pub running: bool,
    /// Run toggles asked by the run button since the host last read them.
    pub run_toggles: u32,
    /// The skill button that opened state 3.
    pub select_left: bool,
    /// The belt (`hud_belt`).
    pub belt: super::hud_belt::HudBelt,
    /// The new-stats / new-skills pressed flags.
    pub new_btns: NewButtons,
    /// The help button (state 0x22).
    pub help: HelpButton,
    /// The help screen's close button (state 0x21) is pressed.
    pub help_close_pressed: bool,
    /// The help screen's draw opened the mini panel.
    pub help_opened_mini: bool,
    /// The play bindings, for the tips' key names ([`key_names`]).
    pub bindings: Option<crate::controls::Bindings>,
    /// The stamina bar's red, gold and blue: the act palette's nearest
    /// indices; `None` until the
    /// host sets the palette (the bar then falls back to the fill cel).
    pub stamina_colors: Option<[u8; 3]>,
    /// Palette index of RGB (0, 0x40, 0x80): the hot Show Items label's box
    /// (`show_items`).
    pub show_items_blue: Option<u8>,
    /// The skill hotkeys (`hud_hotkeys`).
    pub hotkeys: super::hud_hotkeys::Hotkeys,
}

impl Default for HudState {
    fn default() -> Self {
        HudState {
            tables: HudTables::default(),
            smoothing: GlobeSmoothing::default(),
            input: CtrlInput::default(),
            // Single player (game type 0).
            mini: MiniPanel::new(0),
            running: false,
            run_toggles: 0,
            select_left: true,
            belt: Default::default(),
            new_btns: NewButtons::default(),
            help: HelpButton::default(),
            help_close_pressed: false,
            help_opened_mini: false,
            bindings: None,
            stamina_colors: None,
            show_items_blue: None,
            hotkeys: Default::default(),
        }
    }
}

/// The adapter (installed after the border).
pub struct HudUi {
    pub(super) sh: SharedRef,
}

/// The primary and secondary key names of command `cmd` (the command
/// numbering of the key bindings) for the run and mini-panel tips.
/// D2RUST's own, unverified: the play bindings' first two inputs and their
/// names, not the short / long key name strings (as the belt
/// `CfgHelp` (command 6): the help button's caption
/// keys.
const CMD_HELP: i32 = 6;
/// The help caption's font (font 1).
const HELP_FONT: u16 = 1;

/// The two key names of command `cmd` (the bound keys' labels); no bindings → none.
pub fn key_names(b: Option<&crate::controls::Bindings>, cmd: i32) -> [Option<Vec<u16>>; 2] {
    let inputs = b
        .zip(crate::controls::keymap::action_of_cmd(cmd))
        .map_or(&[][..], |(b, a)| b.inputs(a));
    let name = |i: usize| inputs.get(i).map(|k| k.name().encode_utf16().collect());
    [name(0), name(1)]
}

/// The belt facts the mini panel reads: the belt has extra rows and its row
/// count.
fn belt_rows(hud: &HudState) -> (bool, u8) {
    let st = &hud.belt.state;
    (
        st.extra_boxes,
        crate::ui::panels::control::belt::row_count(st.belt_type),
    )
}

/// The local player's key, when it is a player.
fn local(world: &ClientWorld) -> Option<UnitKey> {
    world
        .local()
        .filter(|u| u.key.unit_type == PLAYER)
        .map(|u| u.key)
}

fn image(files: &UiFiles, name: &str, frame: u32, x: i32, y: i32, clip: Rect) -> Option<UiDraw> {
    let file = files.id(name)?;
    Some(UiDraw::Image(ImageRequest {
        image: ImageRef { file, frame },
        at: Point::new(x, y),
        clip,
        look: crate::ui::CelLook::PLAIN,
        call: crate::ui::draw::CelCall::Draw,
    }))
}

/// `d` with the cel wrapper `call`.
fn with_call(d: Option<UiDraw>, call: crate::ui::draw::CelCall) -> Option<UiDraw> {
    d.map(|d| match d {
        UiDraw::Image(mut r) => {
            r.call = call;
            UiDraw::Image(r)
        }
        other => other,
    })
}

fn globe_file(f: GlobeFile) -> &'static str {
    match f {
        GlobeFile::Hlthmana => "panel\\hlthmana",
        GlobeFile::Overlap => "panel\\overlap",
    }
}

/// A globe request as an image draw: a window is the cel clipped to its
/// rows (counted from the cel's bottom row y).
/// The play screen as the clip of a full-screen draw.
fn screen_clip() -> Rect {
    Screen::play().rect()
}

fn globe_draw(files: &UiFiles, d: &GlobeDraw) -> Option<UiDraw> {
    match *d {
        GlobeDraw::Window {
            file,
            frame,
            x,
            y,
            skip,
            lines,
            mode,
        } => {
            let top = y - skip - lines + 1;
            let clip = Rect::new(0, top, screen_clip().w, u16::try_from(lines).ok()?);
            // The row window is the `Ex` cel draw (the `hlthmana` globe fill as
            // `CelDrawEx`), in the request's
            // draw mode: the potion window is mode 0 (25 % blend,
            // the blend modes), the fill mode 5.
            let d = with_call(
                image(files, globe_file(file), frame, x, y, clip),
                crate::ui::draw::CelCall::Ex,
            );
            d.map(|d| match d {
                UiDraw::Image(mut r) => {
                    r.look.mode = mode;
                    UiDraw::Image(r)
                }
                other => other,
            })
        }
        GlobeDraw::Cel { file, frame, x, y } => {
            image(files, globe_file(file), frame, x, y, screen_clip())
        }
    }
}

/// A fill cel clipped to x…x + w − 1, y…y + h − 1 (the frame's top-left is
/// its cel position minus its height − 1: synthetic frames anchor top).
fn fill(files: &UiFiles, frame: u32, x: i32, y: i32, w: i32, h: i32) -> Option<UiDraw> {
    if w <= 0 || h <= 0 {
        return None;
    }
    let clip = Rect::new(x, y, u16::try_from(w).ok()?, u16::try_from(h).ok()?);
    image(files, FILL_FILE, frame, x, y, clip)
}

/// The skills of the select panel and their icon positions (module doc).
pub(super) fn select_icons(
    world: &ClientWorld,
    tables: &HudTables,
    left: bool,
    w: i32,
    h: i32,
) -> Vec<SelectIcon> {
    select_icons_with(world, tables, None, left, w, h)
}

/// [`select_layout`] of the local player; the colour from the model's
/// use state ([`crate::ui::char_desc::hand_use_code`]) when the character
/// tables are loaded, else from [`use_state_k`].
fn select_icons_with(
    world: &ClientWorld,
    tables: &HudTables,
    chars: Option<&crate::ui::char_feed::CharTables>,
    left: bool,
    w: i32,
    h: i32,
) -> Vec<SelectIcon> {
    let (Some(key), Some(list)) = (local(world), world.local().and_then(|u| u.skills.as_ref()))
    else {
        return Vec::new();
    };
    let desc = chars.and_then(|c| c.desc.skills.clone());
    let Some((c, t)) = chars.zip(desc.as_deref()) else {
        return select_layout(tables, list, left, w, h, &mut |e| {
            use_state_k(tables, e.skill, e.base + e.level_bonus)
        });
    };
    let locked = skill_locked(world, key);
    let mut units = crate::ui::char_desc::DescUnits::new(world, key, &c.desc);
    select_layout(tables, list, left, w, h, &mut |e| {
        model_k(&mut units, c, t, key, e, locked)
    })
}

/// The font of the hotkey labels and the charges count: none is set in
/// their chain, so the current one, font 1
/// after the belt (`ui-skill-hotkey-assign-ama` rows 232–236: font16).
const OVERLAY_FONT: u16 = 1;

/// An icon's hotkey labels (colour 4) then its charges count
/// (colour 3), for the HUD skill buttons and the select
/// panel's icons. A text the font cannot measure is left
/// out.
#[allow(clippy::too_many_arguments)]
fn hotkey_overlay(
    out: &mut dyn UiDrawSink,
    fonts: Option<&super::FontMeasure>,
    bindings: Option<&crate::controls::Bindings>,
    keys: &super::hud_hotkeys::Hotkeys,
    rec: super::hud_hotkeys::Record,
    left: bool,
    charges: i32,
    clip: Rect,
) {
    use super::hud_hotkeys as hk;
    let Some(fonts) = fonts else {
        return;
    };
    let width = |t: &[u16]| fonts.width_a(OVERLAY_FONT, t);
    let mut text = |t: Vec<u16>, at: (i32, i32), color: u16| {
        out.push(UiDraw::Text(crate::ui::draw::TextRequest {
            text: t,
            at: Point::new(at.0, at.1),
            style: crate::ui::draw::TextStyle {
                font: OVERLAY_FONT,
                color,
            },
            opts: crate::ui::text::TextOpts::default(),
            clip,
        }));
    };
    for i in hk::labels_of(keys, &rec, left) {
        let Some(name) = key_names(bindings, i32::from(hk::slot_cmd(i)))[0].clone() else {
            continue;
        };
        let Some(t) = hk::fit(&name, &width) else {
            continue;
        };
        let Some(w) = width(&t) else {
            continue;
        };
        text(t, hk::label_at((rec.x, rec.y), w), 4);
    }
    if let Some((t, at)) = hk::count_text(charges, (rec.x, rec.y)) {
        if let Some(t) = hk::fit(&t, &width) {
            text(t, at, 3);
        }
    }
}

/// The skill-use lock test of the use state: `key` is the local player and C is
/// below the skill lock time (unsigned).
fn skill_locked(world: &ClientWorld, key: UnitKey) -> bool {
    world.local_player == Some(key) && world.drlg_updates < world.skill_lock
}

/// The colour of entry `e` from the model's use state
/// ([`crate::ui::char_desc::hand_use_code`]): 0 -> 0, aura -> 4, any other ->
/// 1.
fn model_k(
    units: &mut crate::ui::char_desc::DescUnits,
    c: &crate::ui::char_feed::CharTables,
    t: &d2_sim::skills::SkillTables,
    key: UnitKey,
    e: &crate::bridge::skills::SkillEntry,
    locked: bool,
) -> u8 {
    let sim = d2_sim::skills::SkillEntry {
        skill: i32::from(e.skill),
        base: e.base,
        level_bonus: e.level_bonus,
        owner_guid: e.owner as i32,
        charges: e.charges,
        has_charges: e.has_charges,
    };
    let lvl = d2_sim::skills::levels::skill_level(&*units, t, Some(key), Some(&sim), true);
    let mut hd = crate::ui::char_desc::Hand {
        u: units,
        t,
        code: &c.desc.desc_code,
        key,
        skill: i32::from(e.skill),
        lvl,
    };
    match crate::ui::char_desc::hand_use_code(&mut hd) {
        // A 0 of the local player under the skill-use lock
        // reads 8.
        0 if locked => 1,
        0 => 0,
        6 => 4,
        _ => 1,
    }
}

impl HudUi {
    /// The skill hotkey commands: 14–21 and 46–53 (46–53 in an expansion game
    /// only) assign with state 3 open and use otherwise; 39 / 40 the wheel step
    /// −1 / +1. None runs without a living player.
    fn hotkey_action(&mut self, a: crate::ui::panel::ActionId, ctx: &UiCtx) -> UiResponse {
        use super::hud_hotkeys as hk;
        use crate::controls::Action;
        let Some(&act) = Action::ALL.get(usize::from(a.0)) else {
            return UiResponse::Ignored;
        };
        let slot = (0..hk::SLOTS).find(|&i| {
            crate::controls::keymap::action_of_cmd(i32::from(hk::slot_cmd(i))) == Some(act)
        });
        let step = match act {
            Action::SkillUp => Some(-1),
            Action::SkillDown => Some(1),
            _ => None,
        };
        if slot.is_none() && step.is_none() {
            return UiResponse::Ignored;
        }
        let mut sh = self.sh.borrow_mut();
        let sh = &mut *sh;
        let world = ctx.world;
        let unit = local(world).and_then(|k| world.units.get(&k));
        let Some(list) = unit
            .filter(|u| !u.is_dead())
            .and_then(|u| u.skills.as_ref())
        else {
            return UiResponse::Consumed;
        };
        let (w, h) = (sh.config.screen.w, sh.config.screen.h);
        let intents = match (slot, step) {
            (Some(k), _) if k >= 8 && !sh.facts.expansion_game => Vec::new(),
            (Some(k), _) if sh.states.is_open(UI_SKILL_SELECT) => {
                let icons = select_icons(world, &sh.hud.tables, sh.hud.select_left, w, h);
                let recs = hk::records(&icons);
                let m = (sh.mouse.x, sh.mouse.y);
                hk::assign(&mut sh.hud.hotkeys, k, &recs, sh.hud.select_left, m)
            }
            (Some(k), _) => hk::use_slot(&mut sh.hud.hotkeys, k, list)
                .into_iter()
                .collect(),
            (None, Some(d)) => hk::wheel(&mut sh.hud.hotkeys, d, list)
                .into_iter()
                .collect(),
            (None, None) => Vec::new(),
        };
        sh.outputs
            .extend(intents.into_iter().map(PanelOutput::Intent));
        UiResponse::Consumed
    }

    fn icon_draw(
        files: &UiFiles,
        tables: &HudTables,
        skill: u16,
        at: (i32, i32),
        k: u8,
    ) -> Option<UiDraw> {
        // The colored cel draw with the state of As `k`.
        let (class, cel) = *tables.icons.get(&skill)?;
        let name = skill_icon_file(if class > 6 { 7 } else { class }).to_ascii_lowercase();
        // The skill buttons' icons are the colour (palette) cel draw
        // (`a4-town-pandemonium-fortress` rows 69–70: `CelDrawColor`).
        let mut d = with_call(
            image(files, &name, u32::from(cel), at.0, at.1, screen_clip()),
            crate::ui::draw::CelCall::Color,
        );
        if let Some(UiDraw::Image(r)) = &mut d {
            r.look.remap = crate::ui::Remap::Palette(i32::from(k));
        }
        d
    }
}

impl Panel for HudUi {
    fn id(&self) -> PanelId {
        HUD_PANEL
    }

    /// The control panel strip (y > H − 48); with the mini panel open
    /// also its row; with state 3 open the screen (module doc), and with the
    /// help screen open the whole box it draws over.
    fn rect(&self) -> Rect {
        let sh = self.sh.borrow();
        let s = sh.config.screen;
        if sh.states.is_open(UI_SKILL_SELECT) || sh.states.is_open(super::help_screen::STATE) {
            return Rect::new(0, 0, s.w as u16, s.h as u16);
        }
        let mut top = if sh.states.is_open(UI_MINI) {
            s.h - 76
        } else {
            s.h - 47
        };
        if sh.states.is_open(UI_HELP_BUTTON) && !sh.hud.help.hidden.get() {
            top = top.min(s.h - 196);
        }
        if let Some(t) = sh.hud.belt.popped_top(s.res2()) {
            top = top.min(t);
        }
        if s.w <= 0 {
            return EMPTY;
        }
        Rect::new(0, top, s.w as u16, (s.h - top) as u16)
    }

    fn draw(&self, ctx: &UiCtx, out: &mut dyn UiDrawSink) {
        let mut sh = self.sh.borrow_mut();
        let sh = &mut *sh;
        let (w, h) = (sh.config.screen.w, sh.config.screen.h);
        let files = &sh.tables.files;
        let world = ctx.world;
        let Some(key) = local(world) else {
            return;
        };
        let unit = world.units.get(&key);
        let living = unit.is_some_and(|u| !u.is_dead());
        let has = |s: u8| unit.is_some_and(|u| u.states.contains(&s));
        let stat = |id: u16| world.total(key, id, 0);
        // C = the client update counter (the sound trigger time base):
        // one per client update, so held draw frames do not
        // move the ramp.
        let c = world.drlg_updates;
        let hud = &mut sh.hud;
        // Life globe.
        let (life, life_max) = (stat(6), stat(7));
        let shown = hud.smoothing.records[0].shown(life, life_max, c, true);
        let life_shown = shown;
        let life_in = LifeIn {
            shown,
            max: life_max,
            living_player: living,
            health_potion: has(100),
            stat74: stat(74),
            poisoned: has(2),
        };
        for d in life_globe(&life_in, h, false) {
            out.extend_one(globe_draw(files, &d));
        }
        // Mana globe.
        let (mana, mana_max) = (stat(8), stat(9));
        let shown = hud.smoothing.records[1].shown(mana, mana_max, c, false);
        let mana_shown = shown.min(mana_max);
        let mana_in = ManaIn {
            shown,
            max: mana_max,
            mana_potion: has(106),
            stat26: stat(26),
        };
        for d in mana_globe(&mana_in, w, h, false) {
            out.extend_one(globe_draw(files, &d));
        }
        // Experience bar: two lines of index 0xFF.
        let class = class_u8(unit.map(|u| u.class)).map_or(0, usize::from);
        let level = stat(12).max(0) as u32;
        let exp_in = ExpIn {
            level,
            exp: stat(13) as u32,
            next: hud.tables.threshold(class, level).unwrap_or(0),
            prev: hud
                .tables
                .threshold(class, level.saturating_sub(1))
                .unwrap_or(0),
            max_level: hud.tables.max_level(class).unwrap_or(0),
        };
        for l in exp_bar(&exp_in, w, h) {
            out.extend_one(fill(files, 3, l.x0, l.y, l.x1 - l.x0, 1));
        }
        // Run button.
        let mouse = (sh.mouse.x, sh.mouse.y);
        let run = run_button(w, h, hud.running, hud.input.run_pressed, mouse);
        out.extend_one(cel(files, "panel\\runbutton", run));
        // Stamina bar: blue with a `stambarblue` state.
        let (stamina, stamina_max) = (stat(10), stat(11));
        let shown = hud.smoothing.records[2].shown(stamina, stamina_max, c, false);
        let stamina_shown = shown;
        let blue = hud.tables.stambarblue.iter().any(|&s| has(s));
        let bar = stamina_bar(shown, stamina_max, blue, w, h);
        let frame = match bar.color {
            StaminaColor::Red => 0,
            StaminaColor::Gold => 1,
            StaminaColor::Blue => 2,
        };
        // The rectangle
        // (`a4-town-pandemonium-fortress` row 254: `DrawBox` colour 109).
        match hud.stamina_colors {
            Some(c) if bar.w > 0 => out.push(UiDraw::Rect(crate::ui::draw::RectRequest::sized(
                bar.x,
                bar.y,
                bar.w,
                bar.h,
                c[frame as usize],
                bar.mode,
            ))),
            Some(_) => {}
            None => out.extend_one(fill(files, frame, bar.x, bar.y, bar.w, bar.h)),
        }
        // Menu button.
        let mini_open = sh.states.is_open(UI_MINI);
        let menu = menu_button(w, h, mini_open, hud.input.menu_pressed, mouse);
        out.extend_one(cel(files, "panel\\menubutton", menu));
        // The belt (before the skill buttons).
        let (res2, items_ui) = (sh.config.screen.res2(), &sh.items);
        hud.belt.state_1f_open = sh.states.is_open(0x1F);
        hud.belt.state_9_open = sh.states.is_open(9);
        hud.belt
            .draw(world, items_ui, files, (w, h), res2, mouse, living, out);
        // Skill buttons: the use state of the model when the
        // character tables are loaded (with the cooldown test 10).
        let list = unit.and_then(|u| u.skills.as_ref());
        let desc = sh.char_tables.desc.skills.clone();
        let locked = skill_locked(world, key);
        let mut units = crate::ui::char_desc::DescUnits::new(world, key, &sh.char_tables.desc);
        for (side, entry) in [
            (SkillSide::Left, list.and_then(|l| l.left_entry())),
            (SkillSide::Right, list.and_then(|l| l.right_entry())),
        ] {
            if let Some(e) = entry {
                let at = skill_icon_pos(side, w, h);
                let town = unit.is_some_and(|u| crate::bridge::modes::in_town(world, u.key));
                let k = match desc.as_deref() {
                    Some(t) => {
                        let base = model_k(&mut units, &sh.char_tables, t, key, e, locked);
                        let lacks_in_town =
                            hud.tables.flags.get(&e.skill).is_some_and(|f| !f.in_town);
                        skill_icon_state(base, lacks_in_town, town, mouse, at)
                    }
                    None => skill_button_state(
                        &hud.tables,
                        e.skill,
                        e.base + e.level_bonus,
                        town,
                        mouse,
                        at,
                    ),
                };
                out.extend_one(HudUi::icon_draw(files, &hud.tables, e.skill, at, k));
                // The button's labels (the left button the left-flagged slots)
                // and count.
                let rec = super::hud_hotkeys::Record {
                    x: at.0,
                    y: at.1,
                    skill: i32::from(e.skill),
                    owner: e.owner,
                };
                let charges = if e.owner != crate::bridge::skills::NATIVE {
                    e.charges
                } else {
                    0
                };
                let charges = button_count(
                    &hud.tables.warn,
                    e.skill,
                    charges,
                    [4u8, 5]
                        .map(|loc| body_item(world, &hud.tables.warn, sh.items.tips.as_ref(), loc)),
                );
                hotkey_overlay(
                    out,
                    sh.fonts.as_ref(),
                    hud.bindings.as_ref(),
                    &hud.hotkeys,
                    rec,
                    side == SkillSide::Left,
                    charges,
                    sh.config.screen.rect(),
                );
                // The pop-up while the mouse is in x…x+48, y−48…y.
                if (at.0..=at.0 + 0x30).contains(&mouse.0)
                    && (at.1 - 0x30..=at.1).contains(&mouse.1)
                {
                    sh.button_hover.set(Some(super::skill_hover::ButtonHover {
                        skill: e.skill,
                        at: Point::new(at.0, at.1),
                        k,
                        class: class as u8,
                        // An own skill's nonzero quantity (an item
                        // skill's charge lookup leaves it 0).
                        quantity: if e.owner == crate::bridge::skills::NATIVE {
                            e.quantity
                        } else {
                            0
                        },
                    }));
                }
            }
        }
        // The skill select panel (state 3): after the control panel, before the
        // help button.
        if sh.states.is_open(UI_SKILL_SELECT) {
            let chars = Some(&sh.char_tables);
            for i in select_icons_with(world, &hud.tables, chars, hud.select_left, w, h) {
                // The hover rule of the skill buttons, no `InTown`.
                let k = skill_icon_state(i.k, false, false, mouse, i.at);
                out.extend_one(HudUi::icon_draw(files, &hud.tables, i.skill, i.at, k));
                // The panel side's labels, then the
                // item skill's charges.
                let charges = (i.owner != crate::bridge::skills::NATIVE)
                    .then(|| {
                        list.and_then(|l| l.find(i.skill, i.owner).map(|n| l.entries[n].charges))
                    })
                    .flatten()
                    .unwrap_or(0);
                hotkey_overlay(
                    out,
                    sh.fonts.as_ref(),
                    hud.bindings.as_ref(),
                    &hud.hotkeys,
                    super::hud_hotkeys::records(&[i])[0],
                    hud.select_left,
                    charges,
                    sh.config.screen.rect(),
                );
                // The hover pass shows the skill
                // button's pop-up, not the tree's.
                if (i.at.0..=i.at.0 + 0x30).contains(&mouse.0)
                    && (i.at.1 - 0x30..=i.at.1).contains(&mouse.1)
                {
                    sh.button_hover.set(Some(super::skill_hover::ButtonHover {
                        skill: i.skill,
                        at: Point::new(i.at.0, i.at.1),
                        k,
                        class: class as u8,
                        quantity: i.quantity,
                    }));
                }
            }
        }
        // The help button: step 8's first entry, with state 0x22
        // open (`a4-town-pandemonium-fortress` rows 258–267).
        if sh.states.is_open(UI_HELP_BUTTON) {
            let s = |id: u16| {
                ctx.strings
                    .get_id(id)
                    .map(<[u16]>::to_vec)
                    .unwrap_or_default()
            };
            let text = help_caption(key_names(hud.bindings.as_ref(), CMD_HELP), &s);
            // Font 1, the current font after the belt; without
            // the font's width A the caption cannot be placed and is left
            // out.
            let text_w = sh.fonts.as_ref().and_then(|f| f.width_a(HELP_FONT, &text));
            let states = &sh.states;
            let drawn = hud
                .help
                .draw(w, h, &|ui| states.is_open(ui), text_w.unwrap_or(0));
            if let Some(d) = drawn {
                if text_w.is_some() && !text.is_empty() {
                    out.push(UiDraw::Text(crate::ui::draw::TextRequest {
                        text,
                        at: Point::new(d.caption.0, d.caption.1),
                        style: crate::ui::draw::TextStyle {
                            font: HELP_FONT,
                            color: 0,
                        },
                        opts: crate::ui::text::TextOpts::default(),
                        clip: sh.config.screen.rect(),
                    }));
                }
                out.extend_one(cel(files, "panel\\levelsocket", d.socket));
                out.extend_one(cel(files, "panel\\level", d.button));
            }
        }
        // New-stats / new-skills buttons: step 8 of the UI pass, after
        // the control panel's skill buttons.
        let benv = BtnEnv {
            w,
            h,
            res2: sh.config.screen.res2(),
            open_mode: 0,
        };
        for (which, points, pressed) in [
            (NewBtn::Stats, stat(4), hud.new_btns.stats_pressed),
            (NewBtn::Skills, stat(5), hud.new_btns.skills_pressed),
        ] {
            let c = draw_800(&benv, which, points > 0, pressed, mouse);
            out.extend_one(cel(files, "panel\\level", c));
        }
        // The quest-log alert button (ui 17),
        // step 8 after the new-skills button; nothing in open mode 3 or while
        // ui 15 is open. The row is the quest-log alert row, shifted by (W
        // − 640) / 2 in open mode 2; the button frame is always 0 (the press
        // latch has no handler yet).
        let open_mode = sh.states.open_mode().get();
        if sh.states.is_open(17) && open_mode != 3 && !sh.states.is_open(15) {
            let text = ctx
                .strings
                .get_id(3928)
                .map(<[u16]>::to_vec)
                .unwrap_or_default();
            let text_w = sh.fonts.as_ref().and_then(|f| f.width_a(HELP_FONT, &text));
            let row = quest_alert_row(open_mode, benv.res2, sh.states.is_open(2));
            // (x0, x1, y1, label y), y values before + H − 480.
            let (x0, x1, y1, label_y) = QUEST_ALERT_ROWS[row];
            let shift = if open_mode == 2 { (w - 640) / 2 } else { 0 };
            let (x0, x1) = (x0 + shift, x1 + shift);
            let (sock_y, label_y) = (y1 + h - 480, label_y + h - 480);
            if let Some(tw) = text_w {
                if !text.is_empty() {
                    out.push(UiDraw::Text(crate::ui::draw::TextRequest {
                        text,
                        at: Point::new(x0 + 1 + (x1 - x0) / 2 - tw / 2, label_y),
                        style: crate::ui::draw::TextStyle {
                            font: HELP_FONT,
                            color: 0,
                        },
                        opts: crate::ui::text::TextOpts::default(),
                        clip: sh.config.screen.rect(),
                    }));
                }
            }
            out.extend_one(cel(
                files,
                "panel\\levelsocket",
                ButtonCel {
                    frame: 0,
                    x: x0,
                    y: sock_y,
                },
            ));
            out.extend_one(cel(
                files,
                "panel\\level",
                ButtonCel {
                    frame: 0,
                    x: x0 + 3,
                    y: sock_y - 4,
                },
            ));
        }
        // Life / mana numbers: step 8, after the
        // new-stats / new-skills buttons and before the mini panel.
        let fonts = sh.fonts.as_ref();
        let globe_text = super::hud_tips::GlobeTextIn {
            w,
            h,
            numbers: NumbersIn {
                show_hp: hud.input.show_hp,
                show_mp: hud.input.show_mp,
                mouse,
                life_shown,
                life_max,
                mana_shown,
                mana_max,
                living_player: living,
            },
            stamina: StaminaIn {
                shown: stamina_shown,
                max: stamina_max,
                shrine: has(136),
            },
            strings: ctx.strings,
            width_a: &|t| fonts.and_then(|f| f.width_a(1, t)).unwrap_or(0),
            fonts,
        };
        super::hud_tips::draw_globe_numbers(&globe_text, out);
        // The mini panel with state 0x15 open.
        let mut mini_layout = None;
        if mini_open {
            let open = |ui: u8| sh.states.is_open(ui);
            let (extra, rows) = belt_rows(hud);
            let sides = minipanel::sides(&open, extra, rows);
            hud.mini.set_sides(&sides);
            mini_layout = minipanel::layout(sides.left_blocked, sides.right_blocked);
            if let Some(((ax, ay), buttons)) =
                hud.mini.draw(sides.left_blocked, sides.right_blocked, w, h)
            {
                out.extend_one(image(
                    files,
                    &hud.mini.art().to_ascii_lowercase(),
                    0,
                    ax,
                    ay,
                    screen_clip(),
                ));
                for b in buttons {
                    out.extend_one(image(
                        files,
                        "panel\\minipanelbtn",
                        b.frame,
                        b.x,
                        b.y,
                        screen_clip(),
                    ));
                }
            }
        }
        // The help screen (state 0x21): UI pass step 9, after the mini
        // panel. Its draw opens the mini panel when closed.
        if sh.states.is_open(super::help_screen::STATE) && sh.config.screen.res2() {
            if !sh.states.is_open(UI_MINI) {
                sh.states.force(UI_MINI, true);
                hud.help_opened_mini = true;
            }
            let strings = |id: u16| {
                ctx.strings
                    .get_id(id)
                    .map(<[u16]>::to_vec)
                    .unwrap_or_default()
            };
            let bindings = hud.bindings.as_ref();
            // D2RUST's own, unverified: the default Ctrl / Alt / Shift of the
            // original are the generic keys (VK_CONTROL …), named without
            // the side; D2RUST binds the left ones.
            let key = |cmd: i32| {
                let n = key_names(bindings, cmd)[0].clone().unwrap_or_default();
                let s = String::from_utf16_lossy(&n);
                match s.strip_prefix("Left") {
                    Some(r) if matches!(r, "Ctrl" | "Alt" | "Shift") => r.encode_utf16().collect(),
                    _ => n,
                }
            };
            super::help_screen::draw(
                &super::help_screen::HelpIn {
                    w,
                    h,
                    close_pressed: hud.help_close_pressed,
                    strings: &strings,
                    key: &key,
                    fonts: sh.fonts.as_ref(),
                    files,
                },
                out,
            );
        } else if hud.help_opened_mini && !sh.states.is_open(super::help_screen::STATE) {
            // The help screen closed; the mini panel it
            // opened closes again.
            sh.states.force(UI_MINI, false);
            hud.help_opened_mini = false;
        }
        if !sh.states.is_open(super::help_screen::STATE) {
            hud.help_close_pressed = false;
        }
        // Tool tips, last so they draw on
        // top.
        let bindings = hud.bindings.as_ref();
        super::hud_tips::draw_tips(
            &super::hud_tips::TipIn {
                w,
                h,
                mouse,
                res2,
                mini_open,
                state9_open: sh.states.is_open(9),
                exp: exp_in,
                strings: ctx.strings,
                fonts: sh.fonts.as_ref(),
                keys: &|cmd| key_names(bindings, cmd),
                mini: mini_layout.map(|l| (l, &hud.mini)),
            },
            out,
        );
        // The stamina tip (pop-up text).
        super::hud_tips::draw_stamina_tip(&globe_text, out);
    }

    fn ui_elements(&self, ctx: &UiCtx) -> Vec<crate::ui::ui_ids::UiElement> {
        use crate::ui::panels::control::buttons::{menu_rect, new_rect, run_rect, skill_rect};
        use crate::ui::ui_ids::UiElement;
        let sh = self.sh.borrow();
        if !ctx
            .world
            .local()
            .is_some_and(|u| u.key.unit_type == PLAYER && !u.is_dead())
            // The Esc menu is modal: it takes every press, so nothing of
            // the HUD can be clicked while it is open.
            || sh.states.is_open(9)
        {
            return Vec::new();
        }
        let s = sh.config.screen;
        let (w, h) = (s.w, s.h);
        let mut out = vec![
            UiElement::new("hud.run", run_rect(w, h)),
            UiElement::new("hud.minipanel", menu_rect(w, h)),
            UiElement::new("hud.skill_left", skill_rect(SkillSide::Left, w, h)),
            UiElement::new("hud.skill_right", skill_rect(SkillSide::Right, w, h)),
            // Only the strip below H-48 reaches the globe text handler.
            UiElement::new(
                "hud.hp_text",
                crate::ui::panels::control::input::text_toggle_rect(w, h, TextToggle::Hp),
            ),
            UiElement::new(
                "hud.mp_text",
                crate::ui::panels::control::input::text_toggle_rect(w, h, TextToggle::Mp),
            ),
        ];
        if sh.states.is_open(super::help_screen::STATE) && s.res2() {
            out.push(UiElement::new(
                "help.close",
                super::help_screen::close_rect(),
            ));
        }
        let env = BtnEnv {
            w,
            h,
            res2: s.res2(),
            open_mode: sh.states.open_mode().get(),
        };
        if s.res2() && !sh.states.is_open(9) {
            if let Some(k) = local(ctx.world) {
                for (which, stat, name) in
                    [(NewBtn::Stats, 4, "stats"), (NewBtn::Skills, 5, "skills")]
                {
                    if ctx.world.total(k, stat, 0) > 0 {
                        out.push(UiElement::new(
                            format!("hud.new_{name}"),
                            new_rect(&env, which),
                        ));
                    }
                }
            }
        }
        out.extend(
            sh.hud
                .belt
                .ui_elements(w, h, s.res2())
                .into_iter()
                .filter(|e| {
                    // Both overlay branches return before the belt handler above the strip.
                    e.at.y > h - 48
                        || (!sh.states.is_open(UI_MINI) && !sh.states.is_open(UI_SKILL_SELECT))
                }),
        );
        if sh.states.is_open(UI_MINI)
            && !sh.states.is_open(UI_SKILL_SELECT)
            && crate::bridge::items::cursor_item(ctx.world).is_none()
            && !sh.states.is_open(9)
        {
            let (extra, rows) = belt_rows(&sh.hud);
            out.extend(sh.hud.mini.ui_elements(w, h, extra, rows));
        }
        if sh.states.is_open(UI_HELP_BUTTON)
            && !sh.hud.help.hidden.get()
            && sh.hud.help.setting == 0
            && !sh.states.is_open(UI_SKILL_SELECT)
        {
            out.push(UiElement::new("hud.help", HelpButton::rect(w, h)));
        }
        let rect = self.rect();
        out.retain(|e| rect.contains(e.at));
        out
    }

    fn hit(&self, _p: Point) -> Option<WidgetId> {
        None
    }

    fn event(&mut self, e: UiEvent, ctx: &UiCtx) -> UiResponse {
        if let UiEvent::Action(a) = e {
            return self.hotkey_action(a, ctx);
        }
        let Some((down, at)) = left(e) else {
            return UiResponse::Ignored;
        };
        let mut sh = self.sh.borrow_mut();
        let sh = &mut *sh;
        let (w, h) = (sh.config.screen.w, sh.config.screen.h);
        let world = ctx.world;
        let alive = local(world)
            .and_then(|k| world.units.get(&k))
            .is_some_and(|u| !u.is_dead());
        let select_open = sh.states.is_open(UI_SKILL_SELECT);
        let mini_open = sh.states.is_open(UI_MINI);
        // The help screen's Close button: the same close as Escape
        // (the close-all: mode 1, jump), ahead of the HUD behind it.
        if sh.states.is_open(super::help_screen::STATE) && sh.config.screen.res2() {
            let close = super::help_screen::close_rect();
            if down {
                if close.contains(at) {
                    sh.hud.help_close_pressed = true;
                    return UiResponse::Consumed;
                }
            } else if sh.hud.help_close_pressed {
                sh.hud.help_close_pressed = false;
                if close.contains(at) {
                    sh.outputs.push(PanelOutput::SetUi {
                        ui: super::help_screen::STATE,
                        mode: 1,
                        jump: true,
                    });
                }
                return UiResponse::Consumed;
            }
        }
        // The select panel's left button down,
        // ahead of the HUD; the release is consumed too (D2RUST's own).
        if select_open && at.y <= h - 48 {
            if down {
                let icons = select_icons(world, &sh.hud.tables, sh.hud.select_left, w, h);
                let recs = super::hud_hotkeys::records(&icons);
                let empty = crate::bridge::skills::SkillList::default();
                let list = world
                    .local()
                    .and_then(|u| u.skills.as_ref())
                    .unwrap_or(&empty);
                let c = super::hud_hotkeys::click(
                    &recs,
                    list,
                    &mut sh.hud.select_left,
                    (at.x, at.y),
                    h,
                );
                sh.outputs
                    .extend(c.intents.into_iter().map(PanelOutput::Intent));
                if c.close {
                    sh.outputs.push(PanelOutput::SetUi {
                        ui: UI_SKILL_SELECT,
                        mode: 1,
                        jump: false,
                    });
                }
            }
            return UiResponse::Consumed;
        }
        // R4, r5 the help button: window handlers of state 0x22, ahead
        // of the panel rows (its box lies over the mini panel's row).
        if sh.states.is_open(UI_HELP_BUTTON) {
            let mouse = (at.x, at.y);
            if down {
                if sh.hud.help.press(w, h, mouse) {
                    return UiResponse::Consumed;
                }
            } else if let Some(outs) = sh.hud.help.release(w, h, mouse) {
                sh.outputs.extend(outs);
                return UiResponse::Consumed;
            }
        }
        let player = PlayerFacts {
            living: alive,
            blocked: false,
            dead: !alive,
        };
        // R7, r8: the mini panel row.
        if mini_open && at.y < h - 47 {
            let open = |ui: u8| sh.states.is_open(ui);
            let mouse = (at.x, at.y);
            let cursor_item = crate::bridge::items::cursor_item(world).is_some();
            let (extra, rows) = belt_rows(&sh.hud);
            let state9 = open(9);
            // D2RUST's own, unverified: the cursor mode is read as 7 (no
            // cursor mode in the model; with no cursor item the reset
            // has nothing to do).
            let (acts, consumed) = if down {
                sh.hud
                    .mini
                    .press(w, h, mouse, cursor_item, extra, rows, state9, &player)
            } else {
                sh.hud
                    .mini
                    .release(w, h, mouse, cursor_item, 7, extra, rows, &player, &open)
            };
            for a in acts {
                match a {
                    MiniAction::Ui(o) => sh.outputs.push(o),
                    MiniAction::Sound(id) => sh.outputs.push(PanelOutput::Sound(id as i32)),
                    // The game menu opens through
                    // (`OriginalUi::open_game_menu`).
                    MiniAction::GameMenu => sh.outputs.push(PanelOutput::SetUi {
                        ui: 9,
                        mode: 0,
                        jump: false,
                    }),
                    // After a release that ran a function.
                    MiniAction::InputReset => sh.input_reset = true,
                    // The quest log toggles like the Q
                    // key, asking for the quest data when it opens
                    // (`quest_log_ui`; D2RUST's own, unverified).
                    MiniAction::QuestLog => {
                        let ui = super::quest_log_ui::UI_QUEST_SCREEN;
                        let opening = !sh.states.is_open(ui);
                        sh.outputs.push(PanelOutput::SetUi {
                            ui,
                            mode: 2,
                            jump: false,
                        });
                        if opening {
                            sh.outputs.push(PanelOutput::Intent(
                                super::quest_log_ui::request_quest_data(),
                            ));
                        }
                    }
                    MiniAction::CursorReset => {}
                }
            }
            return if consumed {
                UiResponse::Consumed
            } else {
                UiResponse::Ignored
            };
        }
        // The belt click (the box hit on release; a press over the belt
        // is consumed like `over_belt`).
        let res2 = sh.config.screen.res2();
        let at_px = (at.x, at.y);
        if alive && sh.hud.belt.over(world, (w, h), res2, at_px) {
            if down {
                // A press over the belt records.
                sh.hud.input.press_recorded = true;
            } else {
                // No press recorded → no belt click; always
                // cleared after the release.
                if sh.hud.input.press_recorded {
                    for i in sh.hud.belt.click(world, res2, at_px) {
                        sh.outputs.push(PanelOutput::Intent(i));
                    }
                }
                sh.hud.input.press_recorded = false;
                sh.hud.input.menu_pressed = false;
                sh.hud.input.run_pressed = false;
            }
            return UiResponse::Consumed;
        }
        // R4, r5 the new-stats / new-skills buttons, while points are unspent.
        let benv = BtnEnv {
            w,
            h,
            res2: sh.config.screen.res2(),
            open_mode: 0,
        };
        let unspent = local(world).map(|k| (world.total(k, 4, 0), world.total(k, 5, 0)));
        for (which, points) in [
            (NewBtn::Stats, unspent.map_or(0, |u| u.0)),
            (NewBtn::Skills, unspent.map_or(0, |u| u.1)),
        ] {
            let mouse = (at.x, at.y);
            let (effects, consumed) = if down && points > 0 {
                sh.hud
                    .new_btns
                    .press(&benv, which, at.x, at.y, mouse, false, false)
            } else if !down {
                sh.hud
                    .new_btns
                    .release(&benv, which, at.x, at.y, mouse, false)
            } else {
                continue;
            };
            for eff in effects {
                match eff {
                    BtnEffect::Out(o) => sh.outputs.push(o),
                    // The new-stats / new-skills press runs the
                    // cursor press; the up is the window handler's.
                    BtnEffect::CursorPress => super::cursor_ui::cursor_press(sh, world, at),
                    BtnEffect::CursorRelease => {}
                }
            }
            if consumed {
                return UiResponse::Consumed;
            }
        }
        // The control panel strip.
        let env = InputEnv {
            w,
            h,
            blocked: false,
            alive,
            over_belt: false,
        };
        let (effects, consumed) = if down {
            sh.hud.input.mouse_down(&env, at.x, at.y)
        } else {
            let facts = UpFacts {
                belt_popped: false,
                cursor_mode: 0,
                state3_open: select_open,
                state15_open: mini_open,
            };
            sh.hud.input.mouse_up(&env, &facts, at.x, at.y)
        };
        for eff in effects {
            match eff {
                CtrlEffect::Ui(o) => sh.outputs.push(o),
                CtrlEffect::Sound(id) => sh.outputs.push(PanelOutput::Sound(id as i32)),
                CtrlEffect::Send(i) => sh.outputs.push(PanelOutput::Intent(i)),
                CtrlEffect::SkillSelect(l) => sh.hud.select_left = l,
                CtrlEffect::ToggleRun => sh.hud.run_toggles += 1,
                // Stored at once (`settings.toml`, written by
                // the host as an Options change).
                CtrlEffect::StoreRegistry { which, on } => {
                    let s = &mut sh.esc.menu.settings;
                    match which {
                        TextToggle::Hp => s.show_hp_text = u8::from(on),
                        TextToggle::Mp => s.show_mp_text = u8::from(on),
                    }
                    sh.esc.menu.changed = true;
                    // The globe text follows the toggle on the next pass, not
                    // only after the settings are loaded again.
                    sh.hud.input.show_hp = sh.esc.menu.settings.show_hp_text != 0;
                    sh.hud.input.show_mp = sh.esc.menu.settings.show_mp_text != 0;
                }
                CtrlEffect::CursorMode6 | CtrlEffect::BeltClick => {}
            }
        }
        if consumed {
            UiResponse::Consumed
        } else {
            UiResponse::Ignored
        }
    }
}

fn cel(files: &UiFiles, name: &str, b: ButtonCel) -> Option<UiDraw> {
    image(files, name, b.frame, b.x, b.y, screen_clip())
}

/// Pushes an optional draw.
trait ExtendOne {
    fn extend_one(&mut self, d: Option<UiDraw>);
}

impl ExtendOne for dyn UiDrawSink + '_ {
    fn extend_one(&mut self, d: Option<UiDraw>) {
        if let Some(d) = d {
            self.push(d);
        }
    }
}

/// The frame set the panel art loader makes itself for `name` (D1,
/// D2RUST's own, unverified): the fill cels ([`fill_frames`]); for a HUD file
/// no archive holds (`missing`) 256 transparent 1 × 1 frames, logged.
/// `None`: read the file from the archives.
pub fn preview_set(
    name: &str,
    missing: impl FnOnce() -> bool,
    palette: &d2_formats::palette::Palette,
) -> Option<crate::frames::FrameSet> {
    if name == FILL_FILE {
        return Some(crate::frames::FrameSet {
            frames: fill_frames(palette),
        });
    }
    if optional_file(name) && missing() {
        bevy::log::warn!("hud (D2RUST's own, unverified): {name}.dc6 in no archive, drawn empty");
        let blank = crate::frames::IndexFrame::new(1, 1, 0, 0, vec![0]).expect("1 × 1");
        return Some(crate::frames::FrameSet {
            frames: vec![blank; 256],
        });
    }
    None
}

/// The quest-log alert rows as (x0, x1, y1, label y),
/// y values before + H − 480.
const QUEST_ALERT_ROWS: [(i32, i32, i32, i32); 4] = [
    (40, 75, 375, 337),
    (360, 395, 375, 337),
    (40, 75, 320, 282),
    (360, 395, 320, 282),
];

/// The quest-log alert row: open
/// mode 2 → row 1 at 800 × 600 or with ui 2 open, else row 3; any other
/// mode → row 0 at 800 × 600, else row 2.
fn quest_alert_row(open_mode: u8, res2: bool, ui2_open: bool) -> usize {
    match (open_mode == 2, res2) {
        (true, true) => 1,
        (true, false) if ui2_open => 1,
        (true, false) => 3,
        (false, true) => 0,
        (false, false) => 2,
    }
}

#[cfg(test)]
mod skill_button_tests {
    use super::*;

    #[test]
    fn quest_alert_row_follows_open_mode_resolution_and_ui_2() {
        assert_eq!(quest_alert_row(0, true, false), 0);
        assert_eq!(quest_alert_row(1, false, true), 2);
        assert_eq!(quest_alert_row(2, true, false), 1);
        assert_eq!(quest_alert_row(2, false, true), 1);
        assert_eq!(quest_alert_row(2, false, false), 3);
    }

    fn tables() -> HudTables {
        let mut t = HudTables::default();
        let row = |aura, passive, in_town| SkillButtonFlags {
            in_game: true,
            aura,
            passive,
            in_town,
            ..SkillButtonFlags::default()
        };
        t.flags.insert(1, row(false, false, true)); // usable, InTown
        t.flags.insert(2, row(true, false, true)); // aura
        t.flags.insert(3, row(false, true, true)); // passive
        t.flags.insert(4, row(false, false, false)); // usable, no InTown
        t.flags.insert(5, SkillButtonFlags::default()); // InGame clear
        t
    }

    fn k(t: &HudTables, skill: u16, level: i32, town: bool) -> u8 {
        skill_button_state(t, skill, level, town, (0, 0), (117, 600))
    }

    #[test]
    fn the_button_state_is_0_usable_4_aura_1_otherwise() {
        let t = tables();
        assert_eq!(k(&t, 1, 3, false), 0, "usable");
        assert_eq!(k(&t, 2, 3, false), 4, "aura");
        assert_eq!(k(&t, 1, 0, false), 1, "no level");
        assert_eq!(k(&t, 3, 3, false), 1, "passive");
        assert_eq!(k(&t, 5, 3, false), 1, "InGame clear");
    }

    #[test]
    fn a_skill_without_in_town_is_1_in_town() {
        let t = tables();
        assert_eq!(k(&t, 4, 3, true), 1);
        assert_eq!(k(&t, 4, 3, false), 0);
        assert_eq!(k(&t, 1, 3, true), 0, "with InTown");
        assert_eq!(k(&t, 2, 3, true), 4, "an aura with InTown");
    }

    #[test]
    fn the_potion_window_keeps_its_draw_mode_0() {
        use crate::ui::panels::control::globes::{life_globe, LifeIn};
        let mut files = UiFiles::new(&[]);
        files.extend(["panel\\hlthmana".to_string(), "panel\\overlap".to_string()]);
        // Life 10 of 30 rows' worth, a healing potion predicting 30 %.
        let i = LifeIn {
            shown: 12_800,
            max: 102_400,
            living_player: true,
            health_potion: true,
            stat74: 30,
            poisoned: false,
        };
        let modes: Vec<u8> = life_globe(&i, 600, false)
            .iter()
            .filter_map(|d| match globe_draw(&files, d)? {
                UiDraw::Image(r) => Some(r.look.mode),
                _ => None,
            })
            .collect();
        // Potion window mode 0 (25 % blend), fill mode 5, cover mode 5.
        assert_eq!(modes, vec![0, 5, 5]);
    }

    fn item(type1: u16, weapon: bool, stackable: bool) -> WarnItem {
        WarnItem {
            type1,
            type2: 0,
            weapon,
            stackable,
        }
    }

    fn warn_tables() -> WarnTables {
        // Type 1 a bow (shoots arrows, 3), 2 a sword, 3 arrows, 4 a quiver
        // nested under arrows through `equiv1`.
        let ty = |shoots, equiv1| WarnType {
            shoots,
            equiv1,
            equiv2: 0xFFFF,
            throwable: false,
        };
        let mut t = WarnTables {
            itypes: vec![
                ty(0, 0xFFFF),
                ty(3, 0xFFFF),
                ty(0, 0xFFFF),
                ty(0, 0xFFFF),
                ty(0, 3),
            ],
            ..WarnTables::default()
        };
        t.items.insert(*b"bow ", item(1, true, false));
        t.items.insert(*b"swd ", item(2, true, false));
        t.items.insert(*b"aqv ", item(4, false, false));
        t
    }

    #[test]
    fn the_ammo_icon_shows_for_an_ammunition_user_without_its_ammunition() {
        let t = warn_tables();
        let it = |c: &[u8; 4]| t.items.get(c).copied();
        let (bow, sword, quiver) = (it(b"bow "), it(b"swd "), it(b"aqv "));
        // A bow with an empty or wrong other hand: the icon.
        assert!(ammo_warning(&t, bow, None));
        assert!(ammo_warning(&t, bow, sword));
        // The same bow with a quiver (a type nested under arrows): none.
        assert!(!ammo_warning(&t, bow, quiver));
        // The left hand holding the bow is the same test, mirrored.
        assert!(ammo_warning(&t, None, bow));
        assert!(ammo_warning(&t, sword, bow));
        assert!(!ammo_warning(&t, quiver, bow));
        // Nothing needing ammunition: none (a quiver alone, a sword, empty).
        assert!(!ammo_warning(&t, None, quiver));
        assert!(!ammo_warning(&t, sword, None));
        assert!(!ammo_warning(&t, None, None));
    }

    #[test]
    fn the_warning_icons_take_the_slots_from_the_right_edge() {
        let mut files = UiFiles::new(&[]);
        files.extend(["panel\\invwarn".to_string()]);
        let at = |d: &UiDraw| match d {
            UiDraw::Image(r) => (r.image.frame, r.at.x, r.at.y),
            _ => unreachable!(),
        };
        let d = warning_draws(&files, &[2, 1], 800);
        // 800 - 0x37 = 745 at slot 0, then 40 further left; y = 0x9B.
        assert_eq!(
            d.iter().map(at).collect::<Vec<_>>(),
            [(2, 745, 155), (1, 705, 155)]
        );
        // The seventh icon takes slot 0's place again.
        let d = warning_draws(&files, &[0; 7], 800);
        assert_eq!(at(&d[6]), (0, 745, 155));
    }

    #[test]
    fn a_left_hand_throw_button_prints_the_quantity_of_the_thrown_stack() {
        let mut t = warn_tables();
        // Type 5: throwing axes (throwable); a stackable weapon row.
        t.itypes.push(WarnType {
            throwable: true,
            ..WarnType::default()
        });
        let axes = item(5, true, true);
        let sword = item(2, true, false);
        // The right hand's stack first (w2-02-throw-t3), else the left's
        // (t4).
        assert_eq!(
            button_count(&t, 4, 0, [Some((axes, 130)), Some((axes, 25))]),
            130
        );
        assert_eq!(button_count(&t, 4, 0, [None, Some((axes, 130))]), 130);
        // No stackable throwable weapon there, or another skill: charges.
        assert_eq!(button_count(&t, 4, 7, [Some((sword, 1)), None]), 7);
        assert_eq!(
            button_count(&t, 4, 0, [Some((item(5, true, false), 1)), None]),
            0
        );
        // Throw (2) is not modelled: charges (w2-02-throw-t1 / -t2).
        assert_eq!(button_count(&t, 2, 0, [Some((axes, 130)), None]), 0);
        assert_eq!(button_count(&t, 36, 3, [Some((axes, 130)), None]), 3);
    }
}
