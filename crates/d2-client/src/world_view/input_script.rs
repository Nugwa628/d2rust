//! `play --input SCRIPT` and `state-dump --input SCRIPT`: scripted player
//! input. A script says what a player does (clicks, held buttons, keys,
//! bound actions, skill picks and casts); every step feeds the input path
//! the player's own mouse and keyboard feed. Client input only: nothing
//! here decides an outcome, and nothing runs without a script.
//!
//! - `play` ([`InputScript`]): the steps become the same [`UiEvent`]s the
//!   window would send, timed by server ticks (the window's own pointer
//!   input is ignored while a script runs); keys and actions take the key
//!   path, skill picks leave through the UI root as the skill menu's do.
//! - `state-dump` ([`Headless`]): no window and no UI; the steps' pointer
//!   events go to the world-click dispatcher through the bridge
//!   ([`world_clicks`], the same call the window path makes after the UI)
//!   with the hover of the pass before as `play` picks it
//!   ([`crate::bridge::hover::pick_rects`] over the rectangles
//!   [`HeadlessArt`] places), and the `key` / `action` steps' world
//!   actions to the handlers the window runs for the actions no panel
//!   took ([`HEADLESS_KEY_ACTIONS`]), at the poke point: after the
//!   snapshot of frame F − 1, before frame F's drain. A step the headless
//!   run cannot apply is refused when the script is set up, never skipped.
//! - `clickunit` / `rclickunit` (both): a click on the screen point of
//!   the nearest unit of a type and class ([`unit_point`]), or of a
//!   monster named by its `superuniques` row.
//! - `skill` / `cast` (both): the skill is selected with the message the
//!   skill menu sends ([`super::script_skills::select_skill`]); a cast
//!   then right-clicks a pass later, once the select has gone out.

use crate::bridge::click::ClickView;
use crate::bridge::hover::unit_feet;
use crate::bridge::link::ServerLink;
use crate::bridge::predict::{Predict, Speeds, WalkTap};
use crate::bridge::world::{ClientWorld, KindData, UnitKey};
use crate::bridge::{Bridge, BridgeError};
use crate::controls::click::ClickState;
use crate::controls::keymap::vk_to_key;
use crate::controls::{Action, Bindings, Key, Preset};
use crate::rules::camera::{moving_to_client, Camera, FrameSize, OpenMode};
use crate::ui::panel::ClientIntent;
use crate::ui::{ActionId, FramePos, Point, PointerButton, UiEvent};

use super::interact::PreviewInteract;
use super::script_skills::SkillName;
use super::ui_bind::world_clicks;

/// Classes a `clickunit` step can list.
pub const MAX_CLASSES: usize = 8;

/// The target of a `clickunit` / `rclickunit` step: the nearest unit of
/// `unit_type` whose class is one of `classes[..count]` (`count` 0: `*`,
/// any class, living units only), clicked at its screen point plus
/// `offset` (default `(0, −8)`: a little above the feet, on the body).
/// `superunique ROW` instead names the monster spawned as that
/// `superuniques` row, so a boss is clicked among the minions of its class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnitSel {
    pub button: PointerButton,
    pub unit_type: u8,
    pub classes: [u32; MAX_CLASSES],
    pub count: u8,
    pub offset: (i32, i32),
    pub superunique: Option<u16>,
}

impl UnitSel {
    /// `*`: any class.
    pub fn any(&self) -> bool {
        self.count == 0
    }

    fn takes(&self, class: u32) -> bool {
        self.any() || self.classes[..usize::from(self.count)].contains(&class)
    }

    /// The step as written (`clickunit 1 19,20 0 -8`, `rclickunit
    /// superunique 7`).
    pub fn text(&self) -> String {
        let op = if self.button == PointerButton::Left {
            "clickunit"
        } else {
            "rclickunit"
        };
        if let Some(row) = self.superunique {
            return format!("{op} superunique {row} {} {}", self.offset.0, self.offset.1);
        }
        let cls = if self.any() {
            "*".to_owned()
        } else {
            self.classes[..usize::from(self.count)]
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(",")
        };
        format!(
            "{op} {} {cls} {} {}",
            self.unit_type, self.offset.0, self.offset.1
        )
    }
}

/// The unit a `clickunit` step clicks and the frame point it clicks, or
/// why there is none: among the model's units of the step's type and
/// classes (`*`: any class, but not dead or dying, modes 0 and 12), the
/// one whose screen point is nearest the frame centre (squared distance;
/// ties: the lower unit key, so the pick is the same every run). The
/// screen point is where the camera draws the unit, with no extra offset
/// ([`unit_feet`]: static units by the static rule, moving units at the
/// centre of the cell the client model holds them in); the click is that
/// point plus the step's offset, inside the 800 × 600 frame.
pub fn unit_point(
    world: &ClientWorld,
    cam: &Camera,
    sel: &UnitSel,
) -> Result<(UnitKey, Point), String> {
    let size = FrameSize::play();
    let (cx, cy) = (size.width / 2, size.height / 2);
    let mut best: Option<(i64, UnitKey, (i32, i32))> = None;
    for (key, u) in &world.units {
        if let Some(row) = sel.superunique {
            let matches_row = key.unit_type == 1
                && matches!(&u.kind, KindData::Monster(m) if m.flags & 0x02 != 0 && m.hc_idx == row);
            if !matches_row {
                continue;
            }
        } else if key.unit_type != sel.unit_type || !sel.takes(u.class) {
            continue;
        }
        if sel.superunique.is_none() && sel.any() && matches!(u.mode, 0 | 12) {
            continue;
        }
        let Some(cell) = u.position else { continue };
        let s = unit_feet(cam, key.unit_type, cell);
        let (dx, dy) = (i64::from(s.0 - cx), i64::from(s.1 - cy));
        let d = dx * dx + dy * dy;
        if best.is_none_or(|(b, _, _)| d < b) {
            best = Some((d, *key, s));
        }
    }
    let Some((_, key, s)) = best else {
        return Err("no such unit".into());
    };
    let (x, y) = (s.0 + sel.offset.0, s.1 + sel.offset.1);
    if !(0..size.width).contains(&x) || !(0..size.height).contains(&y) {
        return Err(format!(
            "unit {}:{} at ({x}, {y}), outside the frame",
            key.unit_type, key.guid
        ));
    }
    Ok((key, Point { x, y }))
}

/// What answers a `clickunit` step: the unit and the frame point clicked
/// ([`unit_point`]), or why none.
pub type ResolveUnit<'a> = dyn FnMut(&UnitSel) -> Result<(UnitKey, Point), String> + 'a;

/// The camera a `clickunit` step reads: the local player at `local_at`
/// (16.16; `None`: the model's position), open mode 0, no shake.
pub fn script_camera(world: &ClientWorld, local_at: Option<(u32, u32)>) -> Option<Camera> {
    world.local()?;
    let (x, y) = local_at.or_else(|| world.local_position())?;
    Some(Camera::new(
        FrameSize::play(),
        OpenMode::NONE,
        moving_to_client(x, y),
        (0, 0),
    ))
}

/// The world actions a headless `key` step may run: the ones whose
/// handler the window path runs on an action no panel took
/// (`world_view::present`): the belt columns (commands 23–26,
/// [`crate::bridge::belt::send_keys`]), the run lock (command 35,
/// [`crate::bridge::click::RunMods::toggle_run`]), weapon swap (44,
/// [`super::swap_key::send_swaps`]) and the speech commands (27–33, 55,
/// [`super::swap_key::send_says`]), plus the panel toggles and Esc (the
/// game menu), which the headless UI takes, and the skill hotkeys and the
/// wheel commands (commands 14–21, 46–53, 39 / 40), which the HUD takes
/// before the world sees them. Every other key (chat, held modifiers) is
/// refused headless.
pub const HEADLESS_KEY_ACTIONS: [Action; 43] = [
    Action::BeltSlot1,
    Action::BeltSlot2,
    Action::BeltSlot3,
    Action::BeltSlot4,
    Action::ToggleRun,
    Action::SwapWeapons,
    Action::Say0,
    Action::Say1,
    Action::Say2,
    Action::Say3,
    Action::Say4,
    Action::Say5,
    Action::Say6,
    Action::Say7X,
    // The panel toggles: taken by the headless UI (`apply_with`).
    Action::ToggleCharacter,
    Action::ToggleInventory,
    Action::ToggleParty,
    Action::ToggleQuests,
    Action::ToggleHelp,
    Action::ToggleAutomap,
    Action::ToggleSkillTree,
    Action::ToggleSkillMenuRight,
    Action::ToggleBelt,
    Action::ToggleMessageLog,
    // Esc (command 56): the game menu (UI state 9), taken by the headless
    // UI; its clicks are the menu's.
    Action::GameMenu,
    // The skill hotkeys and the wheel: taken by the HUD (`hud_hotkeys`).
    Action::SkillSlot1,
    Action::SkillSlot2,
    Action::SkillSlot3,
    Action::SkillSlot4,
    Action::SkillSlot5,
    Action::SkillSlot6,
    Action::SkillSlot7,
    Action::SkillSlot8,
    Action::SkillSlot9,
    Action::SkillSlot10,
    Action::SkillSlot11,
    Action::SkillSlot12,
    Action::SkillSlot13,
    Action::SkillSlot14,
    Action::SkillSlot15,
    Action::SkillSlot16,
    Action::SkillUp,
    Action::SkillDown,
];

/// The world action a headless `key` step runs (the default `Original`
/// key preset, key mode 1: no panel is open), or why it is refused.
pub fn headless_key_action(bindings: &Bindings, vk: u32) -> Result<Action, String> {
    let key = play_key(vk).ok_or_else(|| format!("key 0x{vk:02X}: not a key"))?;
    let action = crate::ui::edge::input_actions(bindings, &[key])
        .into_iter()
        .find_map(|e| match e {
            UiEvent::Action(ActionId(a)) => Action::ALL.get(usize::from(a)).copied(),
            _ => None,
        });
    match action {
        Some(a) if HEADLESS_KEY_ACTIONS.contains(&a) => Ok(a),
        other => Err(format!(
            "key 0x{vk:02X} ({}): not applied headless ({}); headless keys: 1-4 (belt), \
             R (run lock), W (weapon swap), NumPad 0-7 (speech), the panel toggles, Esc (game menu), \
             F1-F8 (skill hotkeys); chat and held modifiers are not (use `input D2RUST` with \
             `play`, or a C->S `send` line)",
            key.name(),
            other.map_or("bound to nothing", |a| a.name())
        )),
    }
}

/// The UI between a pass's events and the world handlers ([`Headless::apply_with`]).
pub type UiRoute<'a, L> =
    dyn FnMut(&mut Bridge<L>, Vec<UiEvent>) -> Result<Vec<UiEvent>, BridgeError> + 'a;

/// One step of a script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// `wait N`: N server ticks (`play` only; side-specific).
    Wait(u64),
    /// `move X Y`: the cursor to frame pixel (X, Y).
    Move(Point),
    /// `click X Y` / `rclick X Y`: cursor there, press, and the release
    /// (`play`: on the next tick; framed: in the same pass).
    Click(PointerButton, Point),
    /// `wclick X Y [S]` / `wrclick X Y [S]`: as `Click`, but after the
    /// keys of its pass (`play`: after the pass's keys, in that pass, the
    /// steps after it in the same pass). It is the click that works while
    /// the game is paused (the Esc menu, the options); the optional wait S
    /// is accepted and not used.
    WClick(PointerButton, Point),
    /// `frame F`: the steps after it apply after server frame F − 1 ran,
    /// before frame F's drain.
    Frame(u32),
    /// `hold X Y N` (after a `frame` step): left press at (X, Y), its
    /// release N frames later (before frame F + N's drain).
    Hold(Point, u32),
    /// `key K`: a key down and up (Windows virtual-key code). `play`
    /// delivers it as the window's key press ([`play_key`]: its bound
    /// world action and its typed character); headless runs its world
    /// action ([`headless_key_action`]).
    Key(u32),
    /// `wkey K`: a `key` paced so its down and up halves land on passes
    /// of their own, for menus that read a key per drawn pass. `play`
    /// paces it over passes ([`InputScript::pace`]; the passes run while
    /// the game is paused, too); headless runs it as a `key`.
    WKey(u32),
    /// `keydown K` / `keyup K` (after a `frame` step): the key goes down
    /// and stays down until its `keyup`; `play` reads it as a held input
    /// ([`InputScript::held`]).
    KeyDown(u32),
    KeyUp(u32),
    /// `clickunit T C[,C..]|* [DX DY]` / `rclickunit ...`: a click on the
    /// screen point of a unit ([`unit_point`]).
    ClickUnit(UnitSel),
    /// `down L|R X Y`: the cursor to (X, Y) and that button pressed. It
    /// stays down until its `up` (no release of its own), so the other
    /// button can press and release while it is held.
    Down(PointerButton, Point),
    /// `up L|R X Y`: the cursor to (X, Y) and that button released.
    Up(PointerButton, Point),
    /// `move-held X Y`: the cursor to (X, Y) while a `down` button stays
    /// down (a drag).
    MoveHeld(Point),
    /// `skill L|R NAME`: select skill NAME on that hand, with the message
    /// the skill menu sends ([`super::script_skills::select_skill`]).
    Skill(PointerButton, SkillName),
    /// `cast NAME at unit T C[,C..]|* [DX DY]`: `skill R NAME`, then a
    /// pass later the `rclickunit` of the same target.
    CastUnit(SkillName, UnitSel),
    /// `cast NAME at X Y`: `skill R NAME`, then a pass later a right
    /// click at (X, Y).
    CastAt(SkillName, Point),
    /// `action NAME`: the bound action NAME (a controls name,
    /// [`crate::controls::action_by_name`]), as a key bound to it sends.
    Action(Action),
    /// `wait-until EXPR max N`: the steps after it wait until EXPR holds
    /// ([`WaitUntil`]).
    WaitUntil(WaitUntil),
    /// `click ui:ID`: a left click on the UI element ID
    /// ([`crate::ui::ui_ids`]), at its point in the list the host last
    /// gave ([`InputScript::set_ui_elements`]). An ID not in that list is
    /// a failure naming the ids that are. In game the open panels' ids
    /// are on screen (`npc.menu(N)`, `waypoint.row(N)`, `options.*` in the
    /// Esc menu); the front end's are clicked by `autoplay-host --front`.
    ClickUi(String),
}

/// A `wait-until EXPR max N` step: the script pauses until the check EXPR
/// (the check grammar of [`crate::app::repro::query`]) holds, read after
/// each tick from the one the step is reached on, for at most N ticks.
/// Once it holds, the steps after it run, and every later `frame F` step
/// moves later by the ticks the wait took, so their spacing stays as
/// written. If it never holds, the script stops there and says so
/// ([`InputScript::take_failures`]).
///
/// The check reads the client's `state` reply, which only the `repro`
/// runner builds: `play` and `state-dump` refuse the step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitUntil {
    /// The check, as written.
    pub expr: String,
    /// Ticks to wait at most (≥ 1).
    pub max: u32,
}

/// Why a script with a `wait-until` step cannot run outside `repro`
/// (`who` is the run that refuses it), or `None` when it has none.
pub fn refuse_wait_until(steps: &[Step], who: &str) -> Option<String> {
    steps.iter().find_map(|s| match s {
        Step::WaitUntil(w) => Some(format!(
            "{who} --input: `wait-until {} max {}` runs in `repro` only (it reads the \
             client state the repro runner builds)",
            w.expr, w.max
        )),
        _ => None,
    })
}

/// The letter of a hand (`L`, `R`).
fn hand(button: PointerButton) -> &'static str {
    if button == PointerButton::Left {
        "L"
    } else {
        "R"
    }
}

impl Step {
    /// The `skill` / `cast` step as written (for notes and logs).
    fn skill_text(&self) -> String {
        match self {
            Step::Skill(b, n) => format!("skill {} {n}", hand(*b)),
            Step::CastUnit(n, sel) => {
                let unit = sel.text();
                let rest = unit.strip_prefix("rclickunit ").unwrap_or(&unit);
                format!("cast {n} at unit {rest}")
            }
            Step::CastAt(n, at) => format!("cast {n} at {} {}", at.x, at.y),
            other => format!("{other:?}"),
        }
    }

    /// The skill a `skill` / `cast` step selects and the hand.
    fn selects(&self) -> Option<(SkillName, PointerButton)> {
        match *self {
            Step::Skill(b, n) => Some((n, b)),
            Step::CastUnit(n, _) | Step::CastAt(n, _) => Some((n, PointerButton::Right)),
            _ => None,
        }
    }
}

/// Whether a script has `skill` or `cast` steps (it needs the skill
/// names, [`super::script_skills::SkillNames`]).
pub fn uses_skills(steps: &[Step]) -> bool {
    steps.iter().any(|s| s.selects().is_some())
}

/// What answers a `skill` / `cast` step: the skill id and the
/// select-skill message for a skill on a hand
/// ([`super::script_skills::select_skill`]), or why the player cannot
/// select it.
pub type ResolveSkill<'a> =
    dyn FnMut(&SkillName, PointerButton) -> Result<(u16, ClientIntent), String> + 'a;

/// The virtual-key code of `key K`: a letter or digit, ESC, TAB, ENTER,
/// SPACE, SHIFT, CTRL, ALT, F1–F12 or a number (decimal or `0x` hex).
pub fn vk_code(k: &str) -> Option<u32> {
    let up = k.to_ascii_uppercase();
    let named = match up.as_str() {
        "ESC" => Some(0x1B),
        "TAB" => Some(0x09),
        "ENTER" => Some(0x0D),
        "SPACE" => Some(0x20),
        "SHIFT" => Some(0x10),
        "CTRL" => Some(0x11),
        "ALT" => Some(0x12),
        _ => None,
    };
    if named.is_some() {
        return named;
    }
    if let Some(n) = up.strip_prefix('F').and_then(|n| n.parse::<u32>().ok()) {
        if (1..=12).contains(&n) {
            return Some(0x6F + n);
        }
    }
    let b = up.as_bytes();
    if b.len() == 1 && b[0].is_ascii_alphanumeric() {
        return Some(u32::from(b[0]));
    }
    match up.strip_prefix("0X") {
        Some(h) => u32::from_str_radix(h, 16).ok(),
        None => up.parse().ok(),
    }
}

/// The window key of a `key` step's virtual-key code (`play`), through
/// the controls key table ([`vk_to_key`]); `None` for a code the table
/// cannot hold (the parser rejects those).
pub fn play_key(vk: u32) -> Option<Key> {
    u16::try_from(vk).ok().and_then(vk_to_key)
}

/// A decimal or `0x` hexadecimal integer, optionally negative.
fn int(s: &str) -> Option<i64> {
    let (neg, body) = match s.strip_prefix('-') {
        Some(b) => (true, b),
        None => (false, s),
    };
    let v = match body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
        Some(h) => i64::from_str_radix(h, 16).ok()?,
        None => body.parse().ok()?,
    };
    Some(if neg { -v } else { v })
}

/// The target of a `clickunit` / `rclickunit` / `cast ... at unit` step:
/// `T C[,C..]|* [DX DY]` (`bad`: the error of a malformed one).
fn unit_sel(
    button: PointerButton,
    args: &[&str],
    raw: &str,
    bad: &dyn Fn() -> String,
) -> Result<UnitSel, String> {
    let off = |s: &str| int(s).and_then(|v| i32::try_from(v).ok());
    let (t, cls, offset, superunique) = match args {
        ["superunique", row] => ("1", "*", (0, -8), Some(*row)),
        ["superunique", row, dx, dy] => (
            "1",
            "*",
            (off(dx).ok_or_else(bad)?, off(dy).ok_or_else(bad)?),
            Some(*row),
        ),
        [t, c] => (*t, *c, (0, -8), None),
        [t, c, dx, dy] => (
            *t,
            *c,
            (off(dx).ok_or_else(bad)?, off(dy).ok_or_else(bad)?),
            None,
        ),
        _ => return Err(bad()),
    };
    let superunique = superunique
        .map(|r| int(r).and_then(|v| u16::try_from(v).ok()).ok_or_else(bad))
        .transpose()?;
    let unit_type = int(t)
        .and_then(|v| u8::try_from(v).ok())
        .filter(|&v| v <= 5)
        .ok_or_else(|| format!("`{raw}`: T is a unit type 0-5"))?;
    let mut classes = [0u32; MAX_CLASSES];
    let mut count = 0u8;
    if cls != "*" {
        for (i, c) in cls.split(',').enumerate() {
            let v = int(c).and_then(|v| u32::try_from(v).ok()).ok_or_else(bad)?;
            *classes
                .get_mut(i)
                .ok_or_else(|| format!("`{raw}`: at most {MAX_CLASSES} classes"))? = v;
            count += 1;
        }
    }
    Ok(UnitSel {
        button,
        unit_type,
        classes,
        count,
        offset,
        superunique,
    })
}

/// The skill a step's NAME words write (joined by one space).
fn skill_name(words: &[&str], raw: &str) -> Result<SkillName, String> {
    let text = words.join(" ");
    SkillName::new(&text).ok_or_else(|| {
        format!(
            "`{raw}`: `{text}` is not a skill name (ASCII, at most {} bytes)",
            super::script_skills::MAX_SKILL_NAME
        )
    })
}

/// The button of an `L` / `R` word (any case).
fn hand_of(word: &str) -> Option<PointerButton> {
    match word {
        "L" | "l" => Some(PointerButton::Left),
        "R" | "r" => Some(PointerButton::Right),
        _ => None,
    }
}

/// Parses a script: steps separated by `;`, blank steps ignored.
///
/// - `wait N`: N server ticks (`play` only).
/// - `move X Y`: the cursor to (X, Y).
/// - `click X Y` / `rclick X Y`: a left / right click there.
/// - `wclick X Y [S]` / `wrclick X Y [S]`: a click after the keys of its
///   pass (S is accepted and not used).
/// - `frame F`: the steps after it apply before server frame F (F ≥ 1,
///   never going back).
/// - `hold X Y N` (after a `frame`): a left press, released N frames on.
/// - `key K` / `wkey K`: a key press (a letter, digit, `ESC`, `TAB`,
///   `ENTER`, `SPACE`, `SHIFT`, `CTRL`, `ALT`, `F1`–`F12` or a virtual-key
///   number the controls key table holds, [`play_key`]); `wkey` is paced
///   over passes.
/// - `keydown K` / `keyup K` (after a `frame`): a key held until its up.
/// - `clickunit T C[,C..]|* [DX DY]` / `rclickunit ...`: a click on the
///   nearest unit of type T (0–5) and one of up to [`MAX_CLASSES`]
///   classes (`*`: any living one), offset by (DX, DY) (default (0, −8)).
/// - `down L|R X Y` / `up L|R X Y`: that button pressed / released at
///   (X, Y), each on its own (the other button is free meanwhile); a
///   `down` needs its button up, an `up` needs its button down.
/// - `move-held X Y`: the cursor to (X, Y) while a `down` button is held.
/// - `skill L|R NAME`: select skill NAME on that hand. NAME is a
///   `skills.txt` name in any case, spaces, `_` and `-` ignored
///   (`fireball`, `fire_ball`, `Fire Ball`), or a skill id; it is resolved
///   when the step runs.
/// - `cast NAME at unit T C[,C..]|* [DX DY]`: `skill R NAME`, then a pass
///   later the `rclickunit` of that target.
/// - `cast NAME at X Y`: `skill R NAME`, then a pass later a right click
///   at (X, Y).
/// - `action NAME`: the bound action NAME by its controls name
///   (`toggle_inventory`, `toggle_run`; [`crate::controls::action_by_name`]).
///
/// Coordinates are pixels of the play frame
/// ([`FrameSize::play`](crate::rules::camera::FrameSize::play)).
pub fn parse(text: &str) -> Result<Vec<Step>, String> {
    let f = crate::scene::Rect::world_clip();
    parse_in(text, (f.width as i32, f.height as i32))
}

/// [`parse`] with points checked against a `(width, height)` frame: the
/// command line checks a script against the frame `--res` names, which is
/// chosen only when the game starts.
pub fn parse_in(text: &str, frame: (i32, i32)) -> Result<Vec<Step>, String> {
    parse_with(text, frame, true)
}

/// One step of a script on its own (a scenario's `at T input STEP` line),
/// checked as [`parse_in`] checks it except for the pairs a whole script
/// checks: a lone `up` or `move-held` is fine here, since its `down`
/// comes on another line. The runner parses the joined script again.
pub fn parse_step(step: &str, frame: (i32, i32)) -> Result<Step, String> {
    let mut steps = parse_with(&format!("frame 1; {step}"), frame, false)?;
    match (steps.len(), steps.pop()) {
        (2, Some(s)) => Ok(s),
        _ => Err(format!("`{step}`: one step")),
    }
}

/// [`parse_in`]; `paired`: check that every `up` and `move-held` has its
/// `down` before it.
fn parse_with(
    text: &str,
    (frame_w, frame_h): (i32, i32),
    paired: bool,
) -> Result<Vec<Step>, String> {
    let mut steps = Vec::new();
    let mut last_frame: Option<u32> = None;
    // The buttons a `down` step holds (left, right).
    let mut down = [false; 2];
    for raw in text.split(';') {
        let words: Vec<&str> = raw.split_whitespace().collect();
        let Some((&op, args)) = words.split_first() else {
            continue;
        };
        let num = |s: &str| -> Result<i64, String> {
            s.parse()
                .map_err(|_| format!("`{}`: `{s}` is not a number", raw.trim()))
        };
        let point = |args: &[&str]| -> Result<Point, String> {
            match args {
                [x, y] => {
                    let (x, y) = (num(x)?, num(y)?);
                    let (w, h) = (i64::from(frame_w), i64::from(frame_h));
                    if !(0..w).contains(&x) || !(0..h).contains(&y) {
                        return Err(format!("`{}`: outside the {w} × {h} frame", raw.trim()));
                    }
                    Ok(Point {
                        x: x as i32,
                        y: y as i32,
                    })
                }
                _ => Err(format!("`{}`: needs X Y", raw.trim())),
            }
        };
        steps.push(match op {
            "wait" => match args {
                [n] => Step::Wait(
                    u64::try_from(num(n)?)
                        .map_err(|_| format!("`{}`: negative wait", raw.trim()))?,
                ),
                _ => return Err(format!("`{}`: needs N (server ticks)", raw.trim())),
            },
            "move" => Step::Move(point(args)?),
            "click" if args.len() == 1 && args[0].starts_with(crate::ui::ui_ids::PREFIX) => {
                let id = crate::ui::ui_ids::parse_ref(args[0])
                    .ok_or_else(|| format!("`{}`: not a ui id", raw.trim()))?;
                Step::ClickUi(id.to_owned())
            }
            // `wclick` / `wrclick` (below) are clicks after the keys of their pass.
            "click" => Step::Click(PointerButton::Left, point(args)?),
            "rclick" => Step::Click(PointerButton::Right, point(args)?),
            // An optional third word (a wait in seconds) is accepted and not used.
            "wclick" if matches!(args.len(), 2 | 3) => {
                Step::WClick(PointerButton::Left, point(&args[..2])?)
            }
            "wrclick" if matches!(args.len(), 2 | 3) => {
                Step::WClick(PointerButton::Right, point(&args[..2])?)
            }
            "frame" => {
                let f = match args {
                    [f] => u32::try_from(num(f)?).ok().filter(|&f| f >= 1),
                    _ => None,
                }
                .ok_or_else(|| format!("`{}`: needs F ≥ 1 (server frame)", raw.trim()))?;
                if last_frame.is_some_and(|l| f < l) {
                    return Err(format!("`{}`: frame before the previous one", raw.trim()));
                }
                last_frame = Some(f);
                Step::Frame(f)
            }
            "hold" => {
                if last_frame.is_none() {
                    return Err(format!(
                        "`{}`: hold needs a `frame` step before it (N frames)",
                        raw.trim()
                    ));
                }
                let [x, y, n] = args else {
                    return Err(format!("`{}`: needs X Y N", raw.trim()));
                };
                let n = u32::try_from(num(n)?)
                    .ok()
                    .filter(|&n| n >= 1)
                    .ok_or_else(|| format!("`{}`: N ≥ 1 frames", raw.trim()))?;
                Step::Hold(point(&[x, y])?, n)
            }
            // `wkey K`: paced so each half lands on its own drawn pass.
            "key" | "wkey" => match args {
                [k] => {
                    let vk = vk_code(k)
                        .filter(|&vk| play_key(vk).is_some())
                        .ok_or_else(|| format!("`{}`: unknown key `{k}`", raw.trim()))?;
                    if op == "wkey" {
                        Step::WKey(vk)
                    } else {
                        Step::Key(vk)
                    }
                }
                _ => return Err(format!("`{}`: needs K", raw.trim())),
            },
            "keydown" | "keyup" => {
                if last_frame.is_none() {
                    return Err(format!(
                        "`{}`: {op} needs a `frame` step before it",
                        raw.trim()
                    ));
                }
                let [k] = args else {
                    return Err(format!("`{}`: needs K", raw.trim()));
                };
                let vk = vk_code(k)
                    .filter(|&vk| play_key(vk).is_some())
                    .ok_or_else(|| format!("`{}`: unknown key `{k}`", raw.trim()))?;
                if op == "keydown" {
                    Step::KeyDown(vk)
                } else {
                    Step::KeyUp(vk)
                }
            }
            "clickunit" | "rclickunit" => {
                let bad = || format!("`{}`: needs T C[,C..]|* [DX DY]", raw.trim());
                let button = if op == "clickunit" {
                    PointerButton::Left
                } else {
                    PointerButton::Right
                };
                Step::ClickUnit(unit_sel(button, args, raw.trim(), &bad)?)
            }
            "down" | "up" => {
                let [b, x, y] = args else {
                    return Err(format!("`{}`: needs L|R X Y", raw.trim()));
                };
                let button =
                    hand_of(b).ok_or_else(|| format!("`{}`: the button is L or R", raw.trim()))?;
                let i = usize::from(button == PointerButton::Right);
                let at = point(&[x, y])?;
                if op == "down" {
                    if paired && down[i] {
                        return Err(format!(
                            "`{}`: {} is already down (an `up` first)",
                            raw.trim(),
                            hand(button)
                        ));
                    }
                    down[i] = true;
                    Step::Down(button, at)
                } else {
                    if paired && !down[i] {
                        return Err(format!(
                            "`{}`: {} is not down (a `down` first)",
                            raw.trim(),
                            hand(button)
                        ));
                    }
                    down[i] = false;
                    Step::Up(button, at)
                }
            }
            "move-held" => {
                if paired && !down.contains(&true) {
                    return Err(format!(
                        "`{}`: move-held needs a button held by a `down` step",
                        raw.trim()
                    ));
                }
                Step::MoveHeld(point(args)?)
            }
            "skill" => {
                let Some((b, name)) = args.split_first().filter(|(_, n)| !n.is_empty()) else {
                    return Err(format!("`{}`: needs L|R NAME", raw.trim()));
                };
                let button =
                    hand_of(b).ok_or_else(|| format!("`{}`: the hand is L or R", raw.trim()))?;
                Step::Skill(button, skill_name(name, raw.trim())?)
            }
            "cast" => {
                let bad = || {
                    format!(
                        "`{}`: needs NAME at unit T C[,C..]|* [DX DY], or NAME at X Y",
                        raw.trim()
                    )
                };
                let at = args.iter().position(|w| *w == "at").ok_or_else(bad)?;
                let (name, target) = (&args[..at], &args[at + 1..]);
                if name.is_empty() {
                    return Err(bad());
                }
                let name = skill_name(name, raw.trim())?;
                match target {
                    ["unit", rest @ ..] => Step::CastUnit(
                        name,
                        unit_sel(PointerButton::Right, rest, raw.trim(), &bad)?,
                    ),
                    [_, _] => Step::CastAt(name, point(target)?),
                    _ => return Err(bad()),
                }
            }
            "action" => match args {
                [name] => Step::Action(crate::controls::action_by_name(name).ok_or_else(|| {
                    format!(
                        "`{}`: no action `{name}` (a controls name such as toggle_inventory)",
                        raw.trim()
                    )
                })?),
                _ => return Err(format!("`{}`: needs NAME", raw.trim())),
            },
            "wait-until" => {
                let bad = || format!("`{}`: needs EXPR max N", raw.trim());
                // The check as written (its own spacing kept), up to the
                // last `max` word.
                let body = raw.trim().strip_prefix("wait-until").unwrap_or("").trim();
                let (expr, n) = body.rsplit_once(" max ").ok_or_else(bad)?;
                let max = n
                    .trim()
                    .parse::<u32>()
                    .ok()
                    .filter(|&n| n >= 1)
                    .ok_or_else(|| format!("`{}`: max N ≥ 1 ticks", raw.trim()))?;
                let expr = expr.trim();
                crate::app::repro::query::parse_check(expr)
                    .map_err(|e| format!("`{}`: {e}", raw.trim()))?;
                Step::WaitUntil(WaitUntil {
                    expr: expr.to_owned(),
                    max,
                })
            }
            _ => return Err(format!("`{}`: unknown step `{op}`", raw.trim())),
        });
    }
    Ok(steps)
}

/// An input of the key path ([`InputScript::take_key_inputs`]): a `key`
/// step's key (its bound action and typed character follow, as a pressed
/// key's) or an `action` step's action (the event a key bound to it
/// sends).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyInput {
    Key(Key),
    Action(Action),
}

/// A running script: [`InputScript::events`] once per new server tick.
#[derive(bevy::prelude::Resource, Debug, Clone, PartialEq, Eq)]
pub struct InputScript {
    steps: Vec<Step>,
    next: usize,
    resume_at: u64,
    release: Option<(PointerButton, Point)>,
    release_at: u64,
    cursor: Option<Point>,
    /// The key-path inputs of the `key` and `action` steps run since the
    /// last call, in script order.
    keys: Vec<KeyInput>,
    /// The select-skill messages of the `skill` and `cast` steps run since
    /// the last call.
    selects: Vec<ClientIntent>,
    /// The events of the `wclick` steps run since the last call: after
    /// the pass's keys.
    late: Vec<UiEvent>,
    /// The keys down by `keydown` and not yet up.
    held: Vec<Key>,
    notes: Vec<String>,
    /// The steps that could not run, kept until taken by the runner
    /// ([`InputScript::take_failures`]); [`InputScript::take_notes`] has
    /// them too, for the log.
    failures: Vec<String>,
    /// The `wait-until` step the script is paused on, and the tick it
    /// was reached on.
    waiting: Option<(WaitUntil, u64)>,
    /// The runner saw the paused-on check hold ([`InputScript::wait_met`]).
    wait_met: bool,
    /// Ticks the `wait-until` steps so far waited: later `frame F` steps
    /// run that much later.
    delay: u64,
    /// The UI elements on screen, as the host last set them: what
    /// `click ui:ID` steps look up.
    ui: Vec<crate::ui::ui_ids::UiElement>,
    /// The script has `wkey` steps: its `wkey` and `wclick` steps run one
    /// at a time, paced over passes ([`InputScript::pace`]).
    paced: bool,
    /// Passes left before the next paced step runs.
    countdown: u32,
    /// The next step is a paced `wclick` whose wait has begun.
    armed: bool,
}

impl InputScript {
    pub fn new(steps: Vec<Step>) -> Self {
        InputScript {
            next: 0,
            resume_at: 0,
            release: None,
            release_at: 0,
            cursor: None,
            keys: Vec::new(),
            selects: Vec::new(),
            late: Vec::new(),
            held: Vec::new(),
            notes: Vec::new(),
            failures: Vec::new(),
            waiting: None,
            wait_met: false,
            delay: 0,
            ui: Vec::new(),
            paced: steps.iter().any(|s| matches!(s, Step::WKey(_))),
            countdown: 0,
            armed: false,
            steps,
        }
    }

    /// Whether the next step is one [`InputScript::pace`] runs.
    fn paced_next(&self) -> bool {
        self.paced
            && self.release.is_none()
            && matches!(
                self.steps.get(self.next),
                Some(Step::WKey(_) | Step::WClick(..))
            )
    }

    /// One pass of the paced steps: run on every paused pass (the Esc
    /// menu or the options open, when no server tick comes), and by
    /// [`InputScript::events_with`] when it reaches a paced step. A `key`
    /// before a paced step leaves two idle passes; a `wkey` runs on its
    /// pass and takes four (its down and up halves, two each; the menu
    /// acts on the first); a `wclick` takes six, its click on the third.
    /// This spacing is deliberate: the menu tests pin it. The run's last step comes a
    /// pass early (it ends the pause). Returns whether a step ran.
    pub fn pace(&mut self) -> bool {
        self.pace_pass(true)
    }

    /// [`InputScript::pace`]; `paused`: the pass is a paused one, where
    /// the run's last step comes early ([`InputScript::early_last`]).
    fn pace_pass(&mut self, paused: bool) -> bool {
        if !self.paced_next() {
            return false;
        }
        if self.countdown > 0 {
            self.countdown -= 1;
            return false;
        }
        match self.steps[self.next] {
            Step::WKey(vk) => {
                self.next += 1;
                self.keys.extend(play_key(vk).map(KeyInput::Key));
                self.countdown = 3;
            }
            Step::WClick(button, at) if self.armed => {
                self.next += 1;
                self.armed = false;
                self.late.push(UiEvent::CursorMoved(at));
                self.late.push(UiEvent::Press { button, at });
                self.late.push(UiEvent::Release { button, at });
                self.cursor = Some(at);
                self.countdown = 3;
            }
            Step::WClick(..) => {
                self.armed = true;
                self.countdown = 1;
                return false;
            }
            _ => return false,
        }
        if paused {
            self.early_last();
        }
        true
    }

    /// The run's last step ends the pause (a closing Esc): D2RUST's pass
    /// runs its game tick before its input, so that step comes a pass
    /// early and the first tick after the menu is the pass right after
    /// the menu closes.
    fn early_last(&mut self) {
        let last = !matches!(
            self.steps.get(self.next + 1),
            Some(Step::WKey(_) | Step::WClick(..))
        );
        if self.paced_next() && last {
            self.countdown -= 1;
        }
    }

    /// Whether a `keydown` step holds `key` down now.
    pub fn held(&self, key: Key) -> bool {
        self.held.contains(&key)
    }

    /// Why `clickunit`, `skill` and `cast` steps run since the last call
    /// were skipped.
    pub fn take_notes(&mut self) -> Vec<String> {
        std::mem::take(&mut self.notes)
    }

    /// The steps that could not run since the last call (a `clickunit`
    /// with no such unit, a skill the player lacks, a `wait-until` that
    /// never held): the `repro` runner fails the run on them.
    pub fn take_failures(&mut self) -> Vec<String> {
        std::mem::take(&mut self.failures)
    }

    /// Whether a `click ui:ID` step is still to run, so the host should
    /// give the UI elements on screen ([`Self::set_ui_elements`]).
    pub fn wants_ui(&self) -> bool {
        self.steps[self.next.min(self.steps.len())..]
            .iter()
            .any(|s| matches!(s, Step::ClickUi(_)))
    }

    /// The UI elements on screen now, for the `click ui:ID` steps: the
    /// host sets them each pass before it runs the script.
    pub fn set_ui_elements(&mut self, ui: Vec<crate::ui::ui_ids::UiElement>) {
        self.ui = ui;
    }

    /// A step that could not run: logged and kept as a failure.
    fn note(&mut self, why: String) {
        self.failures.push(why.clone());
        self.notes.push(why);
    }

    /// The `wait-until` step the script is paused on, if any: its check
    /// is to be read after this tick ([`InputScript::wait_met`]).
    pub fn waiting(&self) -> Option<&WaitUntil> {
        self.waiting.as_ref().map(|(w, _)| w)
    }

    /// The paused-on `wait-until` check held: the steps after it run from
    /// the next tick.
    pub fn wait_met(&mut self) {
        if self.waiting.is_some() {
            self.wait_met = true;
        }
    }

    /// Where the script put the cursor last.
    pub fn cursor(&self) -> Option<FramePos> {
        self.cursor.map(FramePos::Inside)
    }

    /// The keys and actions of the `key` and `action` steps run since the
    /// last call, in script order (the caller turns them into the window's
    /// key events).
    pub fn take_key_inputs(&mut self) -> Vec<KeyInput> {
        std::mem::take(&mut self.keys)
    }

    /// [`InputScript::take_key_inputs`]' keys (the checks of `key` steps).
    #[cfg(test)]
    fn take_keys(&mut self) -> Vec<Key> {
        self.take_key_inputs()
            .into_iter()
            .filter_map(|k| match k {
                KeyInput::Key(k) => Some(k),
                KeyInput::Action(_) => None,
            })
            .collect()
    }

    /// The select-skill messages of the `skill` and `cast` steps run
    /// since the last call, in order (the caller queues them on the UI
    /// root, which sends them as it sends the skill menu's).
    pub fn take_selects(&mut self) -> Vec<ClientIntent> {
        std::mem::take(&mut self.selects)
    }

    /// The events of the `wclick` steps run since the last call, which
    /// follow the pass's keys.
    pub fn take_late(&mut self) -> Vec<UiEvent> {
        std::mem::take(&mut self.late)
    }

    /// Whether every step has run.
    pub fn done(&self) -> bool {
        self.next >= self.steps.len() && self.release.is_none() && self.waiting.is_none()
    }

    /// Whether [`Self::events_with`] at `tick` would act: a release due,
    /// or a `frame F` step with F ≤ `tick` + 1 next (not consumed); not a
    /// paced step ([`Self::pace`], one per pass).
    pub fn due_at(&self, tick: u64) -> bool {
        if let Some((w, since)) = &self.waiting {
            return self.wait_met || tick >= since + u64::from(w.max);
        }
        if self.release.is_some() {
            return tick >= self.release_at;
        }
        if self.next >= self.steps.len() || tick < self.resume_at || self.paced_next() {
            return false;
        }
        match &self.steps[self.next] {
            Step::Frame(f) => tick + 1 >= u64::from(*f) + self.delay,
            _ => true,
        }
    }

    /// The events of server tick `tick`: a pending release first (and
    /// nothing else that tick; nothing at all while a hold lasts), then
    /// moves up to and including the next press or wait. `frame F` waits
    /// for tick F − 1 (`play` counts the bridge's server ticks, which
    /// equal the server frame from frame 1; the events reach the UI on
    /// the next loop pass, at the start of client frame F). Key and
    /// action steps run without using the tick up;
    /// [`InputScript::take_key_inputs`] hands them to the window's key
    /// path. A `down` / `up` uses the tick up as a click does and leaves
    /// its button as it is until the matching step (no release of its
    /// own). A `clickunit`, `skill` or `cast` step resolves to nothing
    /// here ([`InputScript::events_full`] resolves them).
    pub fn events(&mut self, tick: u64) -> Vec<UiEvent> {
        self.events_with(tick, &mut |_| Err("no unit resolver".into()))
    }

    /// [`InputScript::events`] with `resolve` answering `clickunit`
    /// steps ([`unit_point`]); one that does not resolve is skipped
    /// (noted in [`InputScript::take_notes`]).
    pub fn events_with(&mut self, tick: u64, resolve: &mut ResolveUnit<'_>) -> Vec<UiEvent> {
        self.events_full(tick, resolve, &mut |_, _| Err("no skill resolver".into()))
    }

    /// [`InputScript::events_with`] with `skill` answering the `skill` and
    /// `cast` steps ([`super::script_skills::select_skill`]): the select
    /// goes to [`InputScript::take_selects`] and a cast's click follows a
    /// tick later. One that does not resolve is skipped and noted (a cast
    /// then does not click).
    pub fn events_full(
        &mut self,
        tick: u64,
        resolve: &mut ResolveUnit<'_>,
        skill: &mut ResolveSkill<'_>,
    ) -> Vec<UiEvent> {
        let mut out = Vec::new();
        if let Some((button, at)) = self.release {
            if tick >= self.release_at {
                self.release = None;
                out.push(UiEvent::Release { button, at });
            }
            return out;
        }
        if let Some((w, since)) = self.waiting.take() {
            if self.wait_met {
                // Read after tick `since` held: no delay; each tick
                // later adds one.
                self.wait_met = false;
                self.delay += tick.saturating_sub(since + 1);
            } else if tick >= since + u64::from(w.max) {
                self.note(format!(
                    "wait-until {} max {}: did not hold within {} ticks; the script stops",
                    w.expr, w.max, w.max
                ));
                self.next = self.steps.len();
                return out;
            } else {
                self.waiting = Some((w, since));
                return out;
            }
        }
        while self.next < self.steps.len() && tick >= self.resume_at {
            let step = self.steps[self.next].clone();
            if let Step::Frame(f) = step {
                if tick + 1 < u64::from(f) + self.delay {
                    break;
                }
            }
            if self.paced_next() {
                self.pace_pass(false);
                break;
            }
            self.next += 1;
            match step {
                Step::Wait(n) => {
                    self.resume_at = tick + n;
                    break;
                }
                Step::Frame(_) => {}
                Step::Move(p) => {
                    out.push(UiEvent::CursorMoved(p));
                    self.cursor = Some(p);
                }
                Step::Key(vk) => {
                    self.keys.extend(play_key(vk).map(KeyInput::Key));
                    if self.paced_next() {
                        // The pass is used: the paced steps start two
                        // passes on (`InputScript::pace`).
                        self.countdown = 2;
                        self.armed = false;
                        self.early_last();
                        break;
                    }
                }
                Step::WKey(vk) => self.keys.extend(play_key(vk).map(KeyInput::Key)),
                Step::KeyDown(vk) => {
                    if let Some(k) = play_key(vk).filter(|k| !self.held.contains(k)) {
                        self.held.push(k);
                    }
                }
                Step::KeyUp(vk) => {
                    if let Some(k) = play_key(vk) {
                        self.held.retain(|h| *h != k);
                    }
                }
                Step::ClickUnit(sel) => match resolve(&sel) {
                    // the hover frame: the cursor now, the click a tick later
                    Ok((_unit, at)) => {
                        out.push(UiEvent::CursorMoved(at));
                        self.cursor = Some(at);
                        self.steps.splice(
                            self.next..self.next,
                            [Step::Wait(1), Step::Click(sel.button, at)],
                        );
                    }
                    Err(e) => self.note(format!("{}: {e}", sel.text())),
                },
                Step::Click(button, at) => {
                    out.push(UiEvent::CursorMoved(at));
                    out.push(UiEvent::Press { button, at });
                    self.cursor = Some(at);
                    if matches!(self.steps.first(), Some(Step::Frame(_))) {
                        // The framed form: the press and the release in
                        // that one client pass (a waypoint row, which acts
                        // on the release, travels in the click's frame).
                        out.push(UiEvent::Release { button, at });
                    } else {
                        self.release = Some((button, at));
                        self.release_at = tick + 1;
                    }
                    break;
                }
                Step::WClick(button, at) => {
                    self.late.push(UiEvent::CursorMoved(at));
                    self.late.push(UiEvent::Press { button, at });
                    self.late.push(UiEvent::Release { button, at });
                    self.cursor = Some(at);
                }
                Step::Hold(at, n) => {
                    let button = PointerButton::Left;
                    out.push(UiEvent::CursorMoved(at));
                    out.push(UiEvent::Press { button, at });
                    self.cursor = Some(at);
                    self.release = Some((button, at));
                    self.release_at = tick + u64::from(n);
                    break;
                }
                Step::Down(button, at) => {
                    out.push(UiEvent::CursorMoved(at));
                    out.push(UiEvent::Press { button, at });
                    self.cursor = Some(at);
                    break;
                }
                Step::Up(button, at) => {
                    out.push(UiEvent::CursorMoved(at));
                    out.push(UiEvent::Release { button, at });
                    self.cursor = Some(at);
                    break;
                }
                Step::MoveHeld(p) => {
                    out.push(UiEvent::CursorMoved(p));
                    self.cursor = Some(p);
                }
                Step::ClickUi(id) => match crate::ui::ui_ids::find(&self.ui, &id) {
                    // A plain left click on the element's point, run next.
                    Ok(e) => {
                        let at = e.at;
                        self.steps
                            .insert(self.next, Step::Click(PointerButton::Left, at));
                    }
                    Err(e) => self.note(format!("click ui:{id}: {e}")),
                },
                Step::Action(a) => self.keys.push(KeyInput::Action(a)),
                Step::WaitUntil(w) => {
                    // The runner reads the check after this tick.
                    self.waiting = Some((w, tick));
                    break;
                }
                Step::Skill(..) | Step::CastUnit(..) | Step::CastAt(..) => {
                    let Some((name, hand)) = step.selects() else {
                        continue;
                    };
                    match skill(&name, hand) {
                        Ok((_, intent)) => {
                            self.selects.push(intent);
                            // A cast clicks a tick later, once the select
                            // has gone out.
                            let click = match step {
                                Step::CastUnit(_, sel) => Some(Step::ClickUnit(sel)),
                                Step::CastAt(_, at) => Some(Step::Click(PointerButton::Right, at)),
                                _ => None,
                            };
                            if let Some(click) = click {
                                self.steps
                                    .splice(self.next..self.next, [Step::Wait(1), click]);
                            }
                        }
                        Err(e) => self.note(format!("{}: {e}", step.skill_text())),
                    }
                }
            }
        }
        out
    }
}

/// The headless run's script (`state-dump --input`): the framed form only
/// (it starts with `frame`; no `wait`; a `key` or `action` only when its
/// action is one of [`HEADLESS_KEY_ACTIONS`]; no held keys), applied
/// through the bridge at the poke point. [`Headless::new`] refuses a
/// script with a step it cannot apply.
#[derive(Debug)]
pub struct Headless {
    steps: Vec<Step>,
    next: usize,
    /// F of the last `frame` step run.
    anchor: u32,
    /// The held left button: its point and the frame before whose drain
    /// it is released.
    held: Option<(Point, u32)>,
    /// The buttons a `down` step holds (left, right) until their `up`.
    down: [bool; 2],
    /// The skill names of `skill` / `cast` steps
    /// ([`Headless::with_skill_names`]); without them skills are named
    /// by id.
    skill_names: Option<std::sync::Arc<super::script_skills::SkillNames>>,
    /// The select-skill messages of the pass's `skill` / `cast` steps,
    /// sent before its UI pass ([`Headless::apply_with`]).
    selects: Vec<ClientIntent>,
    mouse: Point,
    click: ClickState,
    /// The key bindings: the `Original` preset (`play`'s default).
    bindings: Bindings,
    /// The run lock and modifier word of the world click (command 35).
    run: crate::bridge::click::RunMods,
    /// The pending interaction of a click on a unit out of reach
    /// ([`PreviewInteract`], as `play`'s preview).
    interact: PreviewInteract,
    /// The hover pick of the previous pass (`ClickView::prev_hover`).
    prev_pick: Option<crate::bridge::world::UnitKey>,
    /// The unit a `clickunit` step aimed at, for the pass of its click: the
    /// hover there when the pick finds no unit under the cursor.
    click_target: Option<crate::bridge::world::UnitKey>,
    /// The camera of the pass before (the last draw's): the pick places
    /// the current cels with it, as the hover reads the drawn frame.
    draw_cam: Option<Camera>,
    /// The screen open mode the UI left after the pass before
    /// ([`Headless::set_open_mode`]); the right button's panel-half gate
    /// (a click on the half a panel covers is the panel's) reads it.
    open_mode: u8,
    /// The local player's walk prediction (the walks its link records,
    /// the charstats speeds), as `play` keeps it (`world_view::walk`):
    /// the click reads the predicted position.
    walk: Option<(Predict, WalkTap, Option<Speeds>)>,
    /// The unit and ground item art of the drawn-rectangle pick; without
    /// it the feet-box pick.
    art: Option<Box<HeadlessArt>>,
}

/// The art the headless pick places the drawn rectangles from (the hover
/// reads what was drawn): the unit files of the model's units
/// ([`UnitArtLoader`]) and the ground items' flippy files
/// ([`GroundItems`]), read as `play` reads them, but no frame is built.
pub struct HeadlessArt {
    assets: super::ViewAssets,
    loader: super::unit_assets::UnitArtLoader,
    rules: super::unit_rules::UnitRules<super::Unspecified>,
    ground: super::ground_items::GroundItems,
    /// The preview feed: unit positions and offsets as `play` reads them.
    feed: super::model_feed::ModelFeed,
}

impl std::fmt::Debug for HeadlessArt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HeadlessArt").finish_non_exhaustive()
    }
}

impl HeadlessArt {
    /// The art of `source` with the unit tables `looks` and the item art
    /// rows `items`; `assets` holds what is read.
    pub fn new(
        source: std::sync::Arc<dyn crate::assets::FileSource>,
        looks: super::unit_assets::UnitLooks,
        items: crate::bridge::items::ItemArtRows,
        assets: super::ViewAssets,
    ) -> Self {
        let looks = std::sync::Arc::new(looks);
        let art = super::unit_assets::SharedUnitArt::default();
        HeadlessArt {
            assets,
            loader: super::unit_assets::UnitArtLoader {
                source: source.clone(),
                looks: looks.clone(),
                art: art.clone(),
            },
            rules: super::unit_rules::UnitRules {
                frame_source: Default::default(),
                rules: super::Unspecified,
                looks,
                art,
            },
            ground: super::ground_items::GroundItems::new(source, items),
            feed: super::model_feed::ModelFeed::default()
                .with_preview(super::preview::Preview::default()),
        }
    }

    /// The drawn rectangle of each unit and ground item under `cam`
    /// (`hover::unit_rects`' rectangles: the body cels' union, an item's
    /// flippy cel) and the warp tiles' select boxes, the body cels by
    /// layer (`hover::pick_layers`); log lines of what did not resolve.
    fn rects(
        &mut self,
        world: &ClientWorld,
        cam: &Camera,
    ) -> (
        crate::bridge::hover::UnitRects,
        crate::bridge::hover::UnitLayers,
        Vec<String>,
    ) {
        let mut log = self.loader.ensure(world, &mut self.assets);
        log.extend(self.ground.ensure(world, &mut self.assets));
        let view =
            crate::rules::OriginalView::new(*cam, &self.rules, &self.feed).with_edge_clip(true);
        let (mut rects, layers, more) = super::unit_body_rects(world, &view, &self.assets);
        log.extend(more);
        for d in self.ground.draws(world, cam, &self.assets, None) {
            let Some(f) = self.assets.frames.frame(d.item.frame) else {
                continue;
            };
            let (l, t) = (d.item.x, d.item.y);
            rects.insert(
                UnitKey::new(crate::bridge::items::ITEM, d.guid),
                (l, t, l + f.width as i32, t + f.height as i32),
            );
        }
        rects.extend(crate::bridge::hover::tile_rects(world, cam));
        (rects, layers, log)
    }

    /// A unit's client point as the drawn rectangles place it.
    fn point(&self, world: &ClientWorld, key: UnitKey) -> Option<(i32, i32)> {
        use crate::rules::ViewSource;
        let u = world.units.get(&key)?;
        let c = self.feed.unit_position(u).ok()?.client();
        Some((c.x, c.y))
    }
}

impl Headless {
    /// The script, or why the headless run cannot play it.
    pub fn new(steps: Vec<Step>) -> Result<Self, String> {
        if !matches!(steps.first(), Some(Step::Frame(_)) | None) {
            return Err("state-dump --input: the script starts with `frame F`".into());
        }
        let bindings = Preset::Original
            .bindings()
            .ok_or("state-dump --input: no original key configuration")?;
        for s in &steps {
            match *s {
                Step::WaitUntil(_) => {
                    return Err(refuse_wait_until(std::slice::from_ref(s), "state-dump")
                        .unwrap_or_default())
                }
                Step::Wait(_) => {
                    return Err(format!(
                        "state-dump --input: {s:?} is not applied headless (`wait` is \
                         side-specific)"
                    ))
                }
                Step::Key(vk) | Step::WKey(vk) => {
                    headless_key_action(&bindings, vk)
                        .map_err(|e| format!("state-dump --input: {e}"))?;
                }
                Step::KeyDown(_) | Step::KeyUp(_) => {
                    return Err(format!(
                        "state-dump --input: {s:?} is not applied headless (a held key \
                         is read by `play`'s window only)"
                    ))
                }
                Step::Action(a) if !HEADLESS_KEY_ACTIONS.contains(&a) => {
                    return Err(format!(
                        "state-dump --input: `action {}` is not available headless (headless \
                         actions: {})",
                        a.name(),
                        HEADLESS_KEY_ACTIONS
                            .iter()
                            .map(|a| a.name())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ))
                }
                _ => {}
            }
        }
        Ok(Self {
            steps,
            next: 0,
            anchor: 0,
            held: None,
            down: [false; 2],
            skill_names: None,
            selects: Vec::new(),
            mouse: Point { x: 320, y: 240 },
            click: ClickState::default(),
            bindings,
            run: Default::default(),
            interact: PreviewInteract::default(),
            prev_pick: None,
            click_target: None,
            draw_cam: None,
            open_mode: 0,
            walk: None,
            art: None,
        })
    }

    /// The screen open mode of the UI the pass runs with (0 when no panel
    /// model is attached).
    pub fn set_open_mode(&mut self, mode: u8) {
        self.open_mode = mode;
    }

    /// `skill` / `cast` steps resolve skill names against `names` (the
    /// player's `skills.txt`); without them only skill ids resolve.
    pub fn with_skill_names(mut self, names: super::script_skills::SkillNames) -> Self {
        self.skill_names = Some(std::sync::Arc::new(names));
        self
    }

    /// The hover reads the drawn rectangles of `art` (as `play`'s
    /// `pick_rects`) instead of the feet-box pick.
    pub fn with_art(mut self, art: HeadlessArt) -> Self {
        self.art = Some(Box::new(art));
        self
    }

    /// Clicks read the local player at the walk prediction of `play`
    /// ([`crate::bridge::predict::PredictLink`]'s `tap`, the character's
    /// `speeds`): the server sends the walker nothing while it walks, so
    /// the model's cell stays at the walk's start and the prediction is
    /// the better guess of where the player is. Without it a click reads
    /// the model's cell.
    pub fn with_prediction(mut self, tap: WalkTap, speeds: Option<Speeds>) -> Self {
        self.walk = Some((Predict::new(), tap, speeds));
        self
    }

    /// The direction the walk prediction's client path turns to next.
    pub fn predicted_new_direction(&self) -> Option<u8> {
        self.walk.as_ref().and_then(|(p, _, _)| p.new_direction())
    }

    /// After each bridge frame: the walks sent, then a prediction step if
    /// the server ticked (`PreviewWalk::frame`).
    pub fn observe(&mut self, world: &ClientWorld, ticked: bool) {
        if let Some((predict, tap, speeds)) = self.walk.as_mut() {
            let walks = tap.take();
            match *speeds {
                Some(sp) => predict.frame(world, walks, ticked, sp),
                None => predict.observe(world),
            }
        }
    }

    /// The prediction as `play` hands it to the model after its walk frame
    /// (`world_view::walk_room::preview_walk_room`): the local player's
    /// room recached at the predicted sub-tile, and the prediction
    /// recorded for the position check (`Bridge::set_local_walk`), so a
    /// server point the client part has not reached is taken, not
    /// answered with C→S 0x5F. No prediction: nothing.
    ///
    /// The headless state-dump client follows the server's walk and point
    /// as the play preview does, so its clicks read the same player
    /// position `play`'s do.
    pub fn sync_local<L: ServerLink>(&self, bridge: &mut Bridge<L>) {
        let Some((predict, _, _)) = self.walk.as_ref() else {
            return;
        };
        if let Some((x, y)) = predict.cell() {
            bridge.recache_local_room(x, y);
        }
        bridge.set_local_walk(predict.position(), predict.mode());
        // The facing is not set here: the client path aim and its one
        // dirdiff step per update turn it inside the frame
        // (`critter_path::player_aim`).
    }

    /// After each bridge frame: [`Headless::observe`]. The pending action
    /// runs in the pass's input part ([`Headless::apply_with`]). Returns
    /// no log lines (kept for the callers' shape).
    pub fn after_frame<L: ServerLink>(
        &mut self,
        bridge: &mut Bridge<L>,
        ticked: bool,
    ) -> Result<Vec<String>, BridgeError> {
        self.observe(bridge.world(), ticked);
        Ok(Vec::new())
    }

    /// The pending action's step of this pass (after the input, before
    /// the server tick), once per server tick.
    fn pending_step<L: ServerLink>(
        &mut self,
        bridge: &mut Bridge<L>,
        log: &mut Vec<String>,
    ) -> Result<(), BridgeError> {
        let target = self.interact.pending.map(|p| p.target);
        let walking = self
            .walk
            .as_ref()
            .is_some_and(|(p, _, _)| p.walking().is_some());
        let run = self.run.word() & crate::controls::click::mods::RUN != 0;
        let tick = bridge.world().server_ticks;
        let local_at = bridge
            .world()
            .local()
            .and_then(|p| p.precise)
            .or_else(|| self.walk.as_ref().and_then(|(p, _, _)| p.position()));
        self.interact.frame(bridge, walking, tick, run, local_at)?;
        if let (Some(t), None) = (target, self.interact.pending) {
            log.push(format!("interact {}:{}", t.unit_type, t.guid));
        }
        Ok(())
    }

    /// Steps not run yet (the footer's "not reached").
    pub fn pending(&self) -> usize {
        self.steps.len() - self.next + usize::from(self.held.is_some())
    }

    /// Whether a button is held, by a `hold` or a `down` step (the held
    /// repeat runs every pass).
    pub fn holding(&self) -> bool {
        self.held.is_some() || self.down.contains(&true)
    }

    /// The events due after server frame `last` ran, and a log line per
    /// step (`frame F: click X Y after frame L`). Pointer steps give
    /// pointer events, `key` and `action` steps their world action
    /// ([`UiEvent::Action`]); `resolve` answers `clickunit` steps
    /// ([`unit_point`]): one that does not resolve gives no event, only
    /// its log line. `skill` / `cast` steps resolve to nothing here
    /// ([`Headless::events_full`]).
    pub fn events(
        &mut self,
        last: i32,
        resolve: &mut ResolveUnit<'_>,
    ) -> (Vec<UiEvent>, Vec<String>) {
        self.events_full(last, resolve, &mut |_, _| Err("no skill resolver".into()))
    }

    /// [`Headless::events`] with `skill` answering `skill` / `cast` steps
    /// ([`super::script_skills::select_skill`]): the select is sent in
    /// this pass, a cast's click a frame later (`cast ... at unit`: its
    /// hover before that frame, the click before the next). One that does
    /// not resolve gives only its log line (a cast then does not click).
    pub fn events_full(
        &mut self,
        last: i32,
        resolve: &mut ResolveUnit<'_>,
        skill: &mut ResolveSkill<'_>,
    ) -> (Vec<UiEvent>, Vec<String>) {
        let (mut out, mut log) = (Vec::new(), Vec::new());
        let last = i64::from(last);
        if let Some((at, until)) = self.held {
            if last < i64::from(until) - 1 {
                return (out, log);
            }
            self.held = None;
            out.push(UiEvent::Release {
                button: PointerButton::Left,
                at,
            });
            log.push(format!(
                "frame {until}: release {} {} after frame {last}",
                at.x, at.y
            ));
            self.anchor = until;
        }
        while let Some(step) = self.steps.get(self.next).cloned() {
            if let Step::Frame(f) = step {
                if last < i64::from(f) - 1 {
                    break;
                }
                self.anchor = f;
                self.next += 1;
                if last > i64::from(f) - 1 {
                    log.push(format!("frame {f} late: after frame {last}"));
                }
                continue;
            }
            self.next += 1;
            let f = self.anchor;
            match step {
                Step::Move(p) => {
                    self.mouse = p;
                    out.push(UiEvent::CursorMoved(p));
                    log.push(format!(
                        "frame {f}: move {} {} after frame {last}",
                        p.x, p.y
                    ));
                }
                Step::Click(button, at) | Step::WClick(button, at) => {
                    self.mouse = at;
                    out.push(UiEvent::CursorMoved(at));
                    out.push(UiEvent::Press { button, at });
                    out.push(UiEvent::Release { button, at });
                    let name = if button == PointerButton::Left {
                        "click"
                    } else {
                        "rclick"
                    };
                    log.push(format!(
                        "frame {f}: {name} {} {} after frame {last}",
                        at.x, at.y
                    ));
                }
                Step::ClickUnit(sel) => match resolve(&sel) {
                    // The hover frame: the cursor before frame F's drain, the click before
                    // frame F + 1's; the next steps wait for it.
                    Ok((unit, at)) => {
                        self.mouse = at;
                        self.click_target = Some(unit);
                        out.push(UiEvent::CursorMoved(at));
                        log.push(format!(
                            "frame {f}: {}: unit {}:{} at ({}, {}), clicked before frame {} \
                             after frame {last}",
                            sel.text(),
                            unit.unit_type,
                            unit.guid,
                            at.x,
                            at.y,
                            f + 1
                        ));
                        self.steps.splice(
                            self.next..self.next,
                            [Step::Frame(f + 1), Step::Click(sel.button, at)],
                        );
                        continue;
                    }
                    Err(e) => log.push(format!(
                        "frame {f}: {}: not clicked ({e}) after frame {last}",
                        sel.text()
                    )),
                },
                Step::Key(vk) | Step::WKey(vk) => match headless_key_action(&self.bindings, vk) {
                    Ok(a) => {
                        out.push(UiEvent::Action(ActionId(a.index() as u16)));
                        log.push(format!(
                            "frame {f}: key 0x{vk:02X} ({}) after frame {last}",
                            a.name()
                        ));
                    }
                    Err(e) => log.push(format!("frame {f}: {e}")),
                },
                Step::Action(a) => {
                    // `Headless::new` admits only the headless actions.
                    out.push(UiEvent::Action(ActionId(a.index() as u16)));
                    log.push(format!("frame {f}: action {} after frame {last}", a.name()));
                }
                Step::Down(button, at) | Step::Up(button, at) => {
                    let press = matches!(step, Step::Down(..));
                    self.down[usize::from(button == PointerButton::Right)] = press;
                    self.mouse = at;
                    out.push(UiEvent::CursorMoved(at));
                    out.push(if press {
                        UiEvent::Press { button, at }
                    } else {
                        UiEvent::Release { button, at }
                    });
                    log.push(format!(
                        "frame {f}: {} {} {} {} after frame {last}",
                        if press { "down" } else { "up" },
                        hand(button),
                        at.x,
                        at.y
                    ));
                }
                Step::MoveHeld(p) => {
                    self.mouse = p;
                    out.push(UiEvent::CursorMoved(p));
                    log.push(format!(
                        "frame {f}: move-held {} {} after frame {last}",
                        p.x, p.y
                    ));
                }
                // No UI runs headless: no element has an id here.
                Step::ClickUi(id) => log.push(format!(
                    "frame {f}: click ui:{id}: not clicked (no ui headless) after frame {last}"
                )),
                Step::Skill(..) | Step::CastUnit(..) | Step::CastAt(..) => {
                    let Some((name, hand)) = step.selects() else {
                        continue;
                    };
                    match skill(&name, hand) {
                        Ok((id, intent)) => {
                            self.selects.push(intent);
                            log.push(format!(
                                "frame {f}: {}: skill {id} selected after frame {last}",
                                step.skill_text()
                            ));
                            let click = match step {
                                Step::CastUnit(_, sel) => Some(Step::ClickUnit(sel)),
                                Step::CastAt(_, at) => Some(Step::Click(PointerButton::Right, at)),
                                _ => None,
                            };
                            if let Some(click) = click {
                                self.steps
                                    .splice(self.next..self.next, [Step::Frame(f + 1), click]);
                            }
                        }
                        Err(e) => log.push(format!(
                            "frame {f}: {}: not selected ({e}) after frame {last}",
                            step.skill_text()
                        )),
                    }
                }
                Step::Hold(at, n) => {
                    self.mouse = at;
                    out.push(UiEvent::CursorMoved(at));
                    out.push(UiEvent::Press {
                        button: PointerButton::Left,
                        at,
                    });
                    self.held = Some((at, f + n));
                    log.push(format!(
                        "frame {f}: hold {} {} {n} after frame {last}",
                        at.x, at.y
                    ));
                    break;
                }
                // Refused by `Headless::new`.
                Step::Frame(_)
                | Step::Wait(_)
                | Step::KeyDown(_)
                | Step::KeyUp(_)
                | Step::WaitUntil(_) => {}
            }
        }
        (out, log)
    }

    /// The headless click view: the 800 × 600 play frame, no panel open,
    /// the hover of the pass before (the drawn rectangles' pick with
    /// [`HeadlessArt`], else the feet-box pick
    /// [`crate::bridge::hover::pick`]), no shake.
    fn view(&self) -> ClickView {
        let size = FrameSize::play();
        ClickView {
            size,
            open_mode: self.open_mode,
            right_panel_bottom: size.play_height(),
            skill_y_limit: size.skill_y_limit(),
            mouse: (self.mouse.x, self.mouse.y),
            game_menu_open: false,
            pick: true,
            shake: (0, 0),
            prev_hover: Some(self.prev_pick),
            local_mode: self.walk.as_ref().and_then(|(p, _, _)| p.mode()),
            speeds: self.walk.as_ref().and_then(|(_, _, s)| *s),
        }
    }

    /// One client pass after server frame `last`: the due steps' events
    /// through the handlers the window path runs, in its order
    /// (`world_view::present`): the run lock toggles, the belt keys
    /// (C→S 0x26), the world-click dispatcher (C→S messages sent at once;
    /// the held repeat), the pending interaction noted, then weapon swap
    /// (0x60) and speech (0x3F). Returns the log lines.
    pub fn apply<L: ServerLink>(
        &mut self,
        bridge: &mut Bridge<L>,
        last: i32,
    ) -> Result<Vec<String>, BridgeError> {
        self.apply_with(bridge, last, &mut |_, e| Ok(e))
    }

    /// [`Headless::apply`] with the UI between the events and the world:
    /// `ui` routes the due events through the panels (as `play`'s
    /// `run_ui_with`: a panel key or a click on a panel is taken there and
    /// sends its own messages, e.g. the quest log's 0x40) and returns the
    /// events no panel took, which go on to the world handlers.
    pub fn apply_with<L: ServerLink>(
        &mut self,
        bridge: &mut Bridge<L>,
        last: i32,
        ui: &mut UiRoute<'_, L>,
    ) -> Result<Vec<String>, BridgeError> {
        // The click reads the client player's position: the server copy of
        // the last tick, one tick fresher than the walk prediction's step
        // (a held button's repeats read the position after the previous
        // tick).
        let local_at = bridge
            .world()
            .local()
            .and_then(|p| p.precise)
            .or_else(|| self.walk.as_ref().and_then(|(p, _, _)| p.position()));
        let cam = script_camera(bridge.world(), local_at);
        let world = bridge.world();
        // The hover a press of this pass reads: the previous pass's draw,
        // which shows the world after the last update with the cursor
        // where it was before this pass's input (a pass is input, server
        // tick, receive, update, draw).
        // So the pick runs here, on the current world, before the script's
        // events move the cursor.
        let mouse = (self.mouse.x, self.mouse.y);
        let mut art_log = Vec::new();
        // The camera origin is set only by the draw, so the pick reads the
        // last drawn frame's (the pass before), with the current cels.
        let pick_cam = self.draw_cam.or(cam);
        self.draw_cam = cam;
        let pick = match (pick_cam.as_ref(), self.art.as_mut()) {
            (Some(c), Some(art)) => {
                let (rects, layers, l) = art.rects(world, c);
                art_log = l;
                let player = local_at.map(|(x, y)| {
                    let p = moving_to_client(x, y);
                    (p.x, p.y)
                });
                let art = &**art;
                crate::bridge::hover::pick_layers(
                    world,
                    &rects,
                    &layers,
                    mouse,
                    self.prev_pick,
                    player,
                    &|k| art.point(world, k),
                )
            }
            (Some(c), None) => crate::bridge::hover::pick(world, c, mouse),
            (None, _) => None,
        };
        // A `clickunit` click is on its unit: when the pick misses it, the
        // step's target is the hover, else the click would turn into a
        // ground click (a point skill cast that a unit-target skill
        // refuses). Any other press keeps the drawn frame's hover.
        self.prev_pick = pick.or(self.click_target.take());
        let names = self.skill_names.clone();
        let (events, mut log) = self.events_full(
            last,
            &mut |sel| match &cam {
                Some(c) => unit_point(world, c, sel),
                None => Err("no local player".into()),
            },
            &mut |name, hand| {
                super::script_skills::select_skill(world, names.as_deref(), name, hand)
            },
        );
        let alive = world.local().is_some_and(|u| !u.is_dead());
        // The skill selects go out before the pass's UI, as the skill
        // menu's leave with the UI pass that took their click.
        for s in std::mem::take(&mut self.selects) {
            bridge.send_bytes(&s.0)?;
        }
        let strip_y = FrameSize::play().play_height();
        for l in art_log {
            log.push(format!("hover art: {l}"));
        }
        // The UI's frame step runs every pass, with or without events
        // (the NPC poll, the dialog pass).
        let mut events = ui(bridge, events)?;
        // The control panel strip: with a
        // live local player a left press or release below y = H − 48 is
        // the control panel's (consumed), so it never reaches the world
        // dispatcher. The UI route sees it first (the HUD's skill
        // buttons); what it leaves (mini panel, run button, belt) is not
        // modelled headless.
        events.retain(|e| {
            let taken = alive
                && matches!(e, UiEvent::Press { button: PointerButton::Left, at }
                    | UiEvent::Release { button: PointerButton::Left, at } if at.y > strip_y);
            if taken {
                log.push(format!(
                    "control panel strip takes {e:?} (not modelled headless)"
                ));
            }
            !taken
        });
        if events.is_empty() && !self.holding() {
            self.pending_step(bridge, &mut log)?;
            return Ok(log);
        }
        let toggle = UiEvent::Action(ActionId(Action::ToggleRun.index() as u16));
        for _ in events.iter().filter(|e| **e == toggle) {
            self.run.toggle_run();
        }
        crate::bridge::belt::send_keys(bridge, &events, Default::default())?;
        let view = self.view();
        let (outs, _) = world_clicks(
            bridge,
            &mut self.click,
            view,
            &events,
            self.run.word(),
            local_at,
        )?;
        let pressed = events.iter().any(|e| matches!(e, UiEvent::Press { .. }));
        self.interact.note(&outs, pressed);
        self.pending_step(bridge, &mut log)?;
        super::swap_key::send_swaps(&events, bridge)?;
        super::swap_key::send_says(&events, bridge)?;
        Ok(log)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: i32, y: i32) -> Point {
        Point { x, y }
    }

    #[test]
    fn parses_every_step_and_rejects_bad_ones() {
        assert_eq!(
            parse("move 320 240; wait 50;click 600 300 ; rclick 1 2;").unwrap(),
            [
                Step::Move(p(320, 240)),
                Step::Wait(50),
                Step::Click(PointerButton::Left, p(600, 300)),
                Step::Click(PointerButton::Right, p(1, 2)),
            ]
        );
        for bad in [
            "jump 1 2",
            "click 1",
            "click 800 0",
            "move -1 0",
            "wait -3",
            "wait",
            "click a b",
            "key",
            "key F1X",
            "key 0xZZ",
            "key I J",
            "wkey",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        assert_eq!(parse(" ; ").unwrap(), []);
        assert_eq!(
            parse("key I; key esc; key TAB; key 0xC0; key 7").unwrap(),
            [
                Step::Key(0x49),
                Step::Key(0x1B),
                Step::Key(0x09),
                Step::Key(0xC0),
                Step::Key(0x37),
            ]
        );
        assert_eq!(
            parse("wkey 0x25; wkey ENTER").unwrap(),
            [Step::WKey(0x25), Step::WKey(0x0D)]
        );
        assert_eq!(
            [0x49, 0x1B, 0x09, 0xC0, 0x37].map(play_key),
            [
                Some(Key::I),
                Some(Key::Escape),
                Some(Key::Tab),
                Some(Key::Grave),
                Some(Key::Digit7),
            ]
        );
    }

    #[test]
    fn keys_run_on_their_tick_without_using_it_up() {
        let mut s = InputScript::new(parse("wait 2; key I; move 1 2; wait 1; key C").unwrap());
        assert_eq!(s.events(1), []);
        assert_eq!(s.take_keys(), []);
        assert_eq!(s.events(3), [UiEvent::CursorMoved(p(1, 2))]);
        assert_eq!(s.take_keys(), [Key::I]);
        assert_eq!(s.take_keys(), []);
        assert_eq!(s.events(4), []);
        assert_eq!(s.take_keys(), [Key::C]);
        assert!(s.done());
    }

    #[test]
    fn a_keydown_holds_the_key_until_its_keyup() {
        let steps = parse("frame 3; keydown ALT; frame 6; keyup alt").unwrap();
        assert_eq!(
            steps,
            [
                Step::Frame(3),
                Step::KeyDown(0x12),
                Step::Frame(6),
                Step::KeyUp(0x12)
            ]
        );
        let mut s = InputScript::new(steps);
        let _ = s.events(1);
        assert!(!s.held(Key::LeftAlt));
        let _ = s.events(2);
        assert!(s.held(Key::LeftAlt));
        assert_eq!(s.take_keys(), []);
        let _ = s.events(4);
        assert!(s.held(Key::LeftAlt));
        let _ = s.events(5);
        assert!(!s.held(Key::LeftAlt));
        assert!(s.done());
        assert!(parse("keydown ALT").is_err());
        assert!(Headless::new(parse("frame 1; keydown ALT").unwrap()).is_err());
    }

    #[test]
    fn parses_the_shared_frame_form() {
        assert_eq!(
            parse("frame 10; click 600 300; hold 1 2 3; frame 20; key r; key F2; key 0x41")
                .unwrap(),
            [
                Step::Frame(10),
                Step::Click(PointerButton::Left, p(600, 300)),
                Step::Hold(p(1, 2), 3),
                Step::Frame(20),
                Step::Key(u32::from(b'R')),
                Step::Key(0x71),
                Step::Key(0x41),
            ]
        );
        for bad in [
            "frame 0",
            "frame",
            "frame 5; frame 4",
            "hold 1 2 3",
            "frame 3; hold 1 2 0",
            "frame 3; hold 1 2",
            "key",
            "key nope",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        assert_eq!(vk_code("esc"), Some(0x1B));
        assert_eq!(vk_code("F12"), Some(0x7B));
        assert_eq!(vk_code("7"), Some(u32::from(b'7')));
    }

    #[test]
    fn steps_run_on_their_ticks_and_a_click_releases_on_the_next() {
        let mut s =
            InputScript::new(parse("move 320 240; wait 10; click 600 300; move 5 6").unwrap());
        assert_eq!(s.events(1), [UiEvent::CursorMoved(p(320, 240))]);
        assert_eq!(s.cursor(), Some(FramePos::Inside(p(320, 240))));
        for t in 2..11 {
            assert_eq!(s.events(t), [], "tick {t}");
        }
        let at = p(600, 300);
        let button = PointerButton::Left;
        assert_eq!(
            s.events(11),
            [UiEvent::CursorMoved(at), UiEvent::Press { button, at }]
        );
        assert_eq!(s.events(12), [UiEvent::Release { button, at }]);
        assert!(!s.done());
        assert_eq!(s.events(13), [UiEvent::CursorMoved(p(5, 6))]);
        assert!(s.done());
        assert_eq!(s.events(14), []);
    }

    #[test]
    fn wclicks_follow_the_keys_of_their_pass_and_do_not_end_it() {
        let mut s = InputScript::new(
            parse("frame 20; key ESC; wclick 400 210 6; wclick 400 310 6").unwrap(),
        );
        assert_eq!(s.events(18), []);
        assert_eq!(s.events(19), []);
        assert_eq!(s.take_keys().len(), 1);
        let button = PointerButton::Left;
        let click = |at| {
            [
                UiEvent::CursorMoved(at),
                UiEvent::Press { button, at },
                UiEvent::Release { button, at },
            ]
        };
        let late: Vec<UiEvent> = click(p(400, 210))
            .into_iter()
            .chain(click(p(400, 310)))
            .collect();
        assert_eq!(s.take_late(), late);
        assert_eq!(s.take_late(), []);
        assert!(s.done());
    }

    #[test]
    fn wkeys_and_wclicks_are_paced_over_passes() {
        let mut s = InputScript::new(
            parse("frame 10; key ESC; wkey 0x28; wkey ENTER; wclick 300 145; wkey ESC; frame 20; click 1 2")
                .unwrap(),
        );
        let key = |vk| play_key(vk).into_iter().collect::<Vec<_>>();
        assert_eq!(s.events(9), []);
        assert_eq!(s.take_keys(), key(0x1B), "the key, alone in its pass");
        assert!(!s.due_at(9), "a paced step is not due");
        // Pass by pass from the key's: what ran.
        let mut ran = Vec::new();
        for pass in 1..=20 {
            if s.pace() {
                ran.push((pass, s.take_keys(), s.take_late().len()));
            }
        }
        assert_eq!(
            ran,
            [
                (3, key(0x28), 0),
                (7, key(0x0D), 0),
                (13, vec![], 3),
                (16, key(0x1B), 0),
            ],
            "two idle passes after a key, four per wkey, the wclick on the third of six, \
             the last a pass early"
        );
        let at = p(1, 2);
        let button = PointerButton::Left;
        assert_eq!(s.events(18), []);
        assert_eq!(
            s.events(19),
            [
                UiEvent::CursorMoved(at),
                UiEvent::Press { button, at },
                UiEvent::Release { button, at },
            ]
        );
    }

    #[test]
    fn unpaused_wkeys_pace_once_per_tick() {
        let mut s = InputScript::new(parse("frame 2; wkey I; wkey I").unwrap());
        let ticks: Vec<u64> = (1..12)
            .filter(|&t| {
                s.events(t);
                !s.take_keys().is_empty()
            })
            .collect();
        assert_eq!(ticks, [1, 5]);
        assert!(s.done());
    }

    #[test]
    fn play_waits_for_frame_minus_one_and_holds_n_ticks() {
        let mut s = InputScript::new(parse("frame 5; hold 10 20 3; move 1 1").unwrap());
        for t in 1..4 {
            assert_eq!(s.events(t), [], "tick {t}");
        }
        let (at, button) = (p(10, 20), PointerButton::Left);
        assert_eq!(
            s.events(4),
            [UiEvent::CursorMoved(at), UiEvent::Press { button, at }]
        );
        assert_eq!(s.events(5), []);
        assert_eq!(s.events(6), []);
        assert_eq!(s.events(7), [UiEvent::Release { button, at }]);
        assert_eq!(s.events(8), [UiEvent::CursorMoved(p(1, 1))]);
        assert!(s.done());
    }

    #[test]
    fn headless_applies_each_step_after_frame_f_minus_one() {
        assert!(
            Headless::new(parse("click 1 1").unwrap()).is_err(),
            "no frame"
        );
        assert!(Headless::new(parse("frame 2; wait 3").unwrap()).is_err());
        assert!(Headless::new(parse("frame 2; key i").unwrap()).is_ok());
        assert!(Headless::new(parse("frame 2; key F1").unwrap()).is_ok());
        let mut none = |_: &UnitSel| -> Result<(UnitKey, Point), String> { Err("none".into()) };
        let mut h = Headless::new(
            parse("frame 10; click 600 300; hold 1 2 3; frame 20; move 5 6; frame 21; rclick 7 8")
                .unwrap(),
        )
        .unwrap();
        let left = PointerButton::Left;
        for f in 0..9 {
            assert_eq!(h.events(f, &mut none).0, [], "frame {f}");
        }
        let (at, hold) = (p(600, 300), p(1, 2));
        let (ev, log) = h.events(9, &mut none);
        assert_eq!(
            ev,
            [
                UiEvent::CursorMoved(at),
                UiEvent::Press { button: left, at },
                UiEvent::Release { button: left, at },
                UiEvent::CursorMoved(hold),
                UiEvent::Press {
                    button: left,
                    at: hold
                },
            ]
        );
        assert_eq!(
            log,
            [
                "frame 10: click 600 300 after frame 9",
                "frame 10: hold 1 2 3 after frame 9"
            ]
        );
        assert!(h.holding());
        assert_eq!(h.events(10, &mut none).0, []);
        assert_eq!(h.events(11, &mut none).0, []);
        // hold 3 from frame 10: released before frame 13's drain
        assert_eq!(
            h.events(12, &mut none).0,
            [UiEvent::Release {
                button: left,
                at: hold
            }]
        );
        assert!(!h.holding());
        assert_eq!(h.events(18, &mut none).0, []);
        assert_eq!(h.events(19, &mut none).0, [UiEvent::CursorMoved(p(5, 6))]);
        // a late stop: the step still runs, noted
        let (ev, log) = h.events(25, &mut none);
        assert_eq!(ev.len(), 3);
        assert_eq!(log[0], "frame 21 late: after frame 25");
        assert_eq!(h.pending(), 0);
    }

    #[test]
    fn parses_down_up_and_move_held() {
        let (l, r) = (PointerButton::Left, PointerButton::Right);
        assert_eq!(
            parse("down L 10 20; move-held 30 40; down r 5 6; up R 5 6; up l 30 40").unwrap(),
            [
                Step::Down(l, p(10, 20)),
                Step::MoveHeld(p(30, 40)),
                Step::Down(r, p(5, 6)),
                Step::Up(r, p(5, 6)),
                Step::Up(l, p(30, 40)),
            ]
        );
        for bad in [
            "down L 1",
            "down X 1 2",
            "down L 800 0",
            "down L 1 2; down L 1 2",
            "up L 1 2",
            "down L 1 2; up R 1 2",
            "move-held 1 2",
            "down L 1 2; up L 1 2; move-held 1 2",
            "down L 1 2; move-held 1",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_lone_step_parses_without_its_pair() {
        let big = (4096, 4096);
        assert_eq!(
            parse_step("up L 30 40", big).unwrap(),
            Step::Up(PointerButton::Left, p(30, 40))
        );
        assert_eq!(
            parse_step("move-held 1 2", big).unwrap(),
            Step::MoveHeld(p(1, 2))
        );
        assert!(parse_step("up X 1 2", big).is_err());
        assert!(parse_step("click 1 2; click 3 4", big).is_err());
        assert!(parse_step("", big).is_err());
    }

    #[test]
    fn parses_wait_until_with_its_check_as_written() {
        let w = |expr: &str, max| {
            Step::WaitUntil(WaitUntil {
                expr: expr.into(),
                max,
            })
        };
        assert_eq!(
            parse("frame 3; wait-until player.mode == cast max 20").unwrap(),
            [Step::Frame(3), w("player.mode == cast", 20)]
        );
        // The check keeps its own spacing; the last `max` ends it.
        assert_eq!(
            parse(r#"wait-until ground_item(gold).label == "max  1" max 4"#).unwrap(),
            [w(r#"ground_item(gold).label == "max  1""#, 4)]
        );
        for bad in [
            "wait-until player.mode == cast",
            "wait-until player.mode == cast max 0",
            "wait-until player.mode == cast max x",
            "wait-until max 3",
            "wait-until player.speed == 1 max 3",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn wait_until_pauses_the_script_and_moves_later_frames() {
        let mut s = InputScript::new(
            parse("frame 2; wait-until hud.life == 1 max 5; frame 4; click 10 10").unwrap(),
        );
        assert!(s.events(0).is_empty() && s.waiting().is_none());
        // Reached on tick 1 (frame 2's step); read after it.
        assert!(s.events(1).is_empty());
        assert_eq!(s.waiting().map(|w| w.max), Some(5));
        assert!(!s.done());
        assert!(s.events(2).is_empty() && !s.due_at(2));
        // Held after tick 2: one tick late, so `frame 4` runs at tick 4.
        s.wait_met();
        assert!(s.due_at(3));
        assert!(s.events(3).is_empty() && s.waiting().is_none());
        assert!(!s.due_at(3));
        let (l, at) = (PointerButton::Left, p(10, 10));
        assert_eq!(
            s.events(4),
            [
                UiEvent::CursorMoved(at),
                UiEvent::Press { button: l, at },
                UiEvent::Release { button: l, at },
            ]
        );
        assert!(s.take_failures().is_empty());

        // A check that holds after the tick it is reached on moves nothing.
        let mut s = InputScript::new(
            parse("frame 1; wait-until hud.life == 1 max 5; frame 3; click 10 10").unwrap(),
        );
        assert!(s.events(0).is_empty());
        s.wait_met();
        assert!(s.events(1).is_empty());
        assert!(!s.events(2).is_empty());
    }

    #[test]
    fn a_wait_until_that_never_holds_stops_the_script() {
        let mut s =
            InputScript::new(parse("frame 1; wait-until hud.life == 1 max 3; click 5 5").unwrap());
        assert!(s.events(0).is_empty());
        assert!(s.events(1).is_empty() && s.events(2).is_empty());
        assert!(s.take_failures().is_empty());
        assert!(s.due_at(3));
        assert!(s.events(3).is_empty());
        assert_eq!(
            s.take_failures(),
            ["wait-until hud.life == 1 max 3: did not hold within 3 ticks; the script stops"]
        );
        assert_eq!(s.take_notes().len(), 1);
        assert!(s.done() && s.waiting().is_none());
        assert!(s.events(4).is_empty());
    }

    #[test]
    fn steps_that_cannot_run_are_failures() {
        let mut s = InputScript::new(parse("frame 1; clickunit 1 19; skill R fireball").unwrap());
        s.events(0);
        assert_eq!(
            s.take_failures(),
            [
                "clickunit 1 19 0 -8: no unit resolver",
                "skill R fireball: no skill resolver"
            ]
        );
    }

    #[test]
    fn only_repro_runs_wait_until() {
        let steps = parse("frame 1; wait-until hud.life == 1 max 3").unwrap();
        let e = refuse_wait_until(&steps, "play").unwrap();
        assert!(
            e.starts_with("play --input:") && e.contains("`repro` only"),
            "{e}"
        );
        let e = Headless::new(steps).unwrap_err();
        assert!(
            e.starts_with("state-dump --input:") && e.contains("`repro` only"),
            "{e}"
        );
        assert_eq!(
            refuse_wait_until(&parse("click 1 1").unwrap(), "play"),
            None
        );
    }

    #[test]
    fn parses_skill_cast_and_action() {
        let name = |s| SkillName::new(s).unwrap();
        let steps = parse(
            "skill R fireball; skill l Fire Ball; skill R 36; cast fire_ball at unit 1 *; \
             cast Fire Ball at unit 1 19,20 0 -4; cast 47 at 300 200; action toggle_inventory; \
             action Toggle-Run",
        )
        .unwrap();
        let any = match steps[3] {
            Step::CastUnit(_, s) => s,
            ref o => panic!("{o:?}"),
        };
        assert_eq!(
            (any.button, any.unit_type, any.any(), any.offset),
            (PointerButton::Right, 1, true, (0, -8))
        );
        let listed = match steps[4] {
            Step::CastUnit(n, s) => {
                assert_eq!(n, name("Fire Ball"));
                s
            }
            ref o => panic!("{o:?}"),
        };
        assert_eq!(
            (&listed.classes[..2], listed.count, listed.offset),
            (&[19, 20][..], 2, (0, -4))
        );
        assert_eq!(
            [0, 1, 2, 3, 5, 6, 7].map(|i| steps[i].clone()),
            [
                Step::Skill(PointerButton::Right, name("fireball")),
                Step::Skill(PointerButton::Left, name("Fire Ball")),
                Step::Skill(PointerButton::Right, name("36")),
                Step::CastUnit(name("fire_ball"), any),
                Step::CastAt(name("47"), p(300, 200)),
                Step::Action(Action::ToggleInventory),
                Step::Action(Action::ToggleRun),
            ]
        );
        assert_eq!(steps[3].skill_text(), "cast fire_ball at unit 1 * 0 -8");
        assert_eq!(steps[1].skill_text(), "skill L Fire Ball");
        assert!(uses_skills(&steps));
        assert!(!uses_skills(
            &parse("click 1 2; action toggle_run").unwrap()
        ));
        for bad in [
            "skill",
            "skill R",
            "skill X fireball",
            "skill fireball",
            "cast fireball",
            "cast at 1 2",
            "cast fireball at",
            "cast fireball at 1",
            "cast fireball at 900 2",
            "cast fireball at unit 1",
            "cast fireball at unit 6 *",
            "cast fireball at unit 1 * 4",
            "action",
            "action nope",
            "action toggle inventory",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        let long = format!(
            "skill R {}",
            "x".repeat(crate::world_view::script_skills::MAX_SKILL_NAME + 1)
        );
        assert!(parse(&long).is_err());
    }

    // The shape of BUG-009: left held (a walk), a right click meanwhile,
    // the left still held after it, released only by its `up`.
    #[test]
    fn down_holds_its_button_while_the_other_clicks() {
        let mut s = InputScript::new(
            parse("down L 100 100; wait 2; rclick 200 200; move-held 120 120; up L 120 120")
                .unwrap(),
        );
        let (l, r) = (PointerButton::Left, PointerButton::Right);
        let (a, b, c) = (p(100, 100), p(200, 200), p(120, 120));
        assert_eq!(
            s.events(1),
            [UiEvent::CursorMoved(a), UiEvent::Press { button: l, at: a }]
        );
        assert_eq!(s.events(2), [], "no release of its own");
        assert_eq!(s.events(3), [], "the wait");
        assert_eq!(
            s.events(4),
            [UiEvent::CursorMoved(b), UiEvent::Press { button: r, at: b }]
        );
        assert_eq!(s.events(5), [UiEvent::Release { button: r, at: b }]);
        assert_eq!(
            s.events(6),
            [
                UiEvent::CursorMoved(c),
                UiEvent::CursorMoved(c),
                UiEvent::Release { button: l, at: c }
            ]
        );
        assert!(s.done());
    }

    #[test]
    fn actions_take_the_key_path_in_script_order() {
        let mut s = InputScript::new(parse("key I; action toggle_run; key C").unwrap());
        assert_eq!(s.events(1), []);
        assert_eq!(
            s.take_key_inputs(),
            [
                KeyInput::Key(Key::I),
                KeyInput::Action(Action::ToggleRun),
                KeyInput::Key(Key::C)
            ]
        );
        assert!(s.done());
    }

    #[test]
    fn skill_selects_and_cast_clicks_a_tick_later() {
        let intent = |id: u16, hand: PointerButton| {
            crate::ui::original::hud_hotkeys::select(
                i32::from(id),
                hand == PointerButton::Left,
                u32::MAX,
            )
        };
        let mut known = |n: &SkillName, hand| -> Result<(u16, ClientIntent), String> {
            match n.as_str() {
                "fireball" => Ok((47, intent(47, hand))),
                _ => Err("not known".into()),
            }
        };
        let at = p(300, 200);
        let mut unit =
            |_: &UnitSel| -> Result<(UnitKey, Point), String> { Ok((UnitKey::new(1, 9), at)) };
        let r = PointerButton::Right;

        // `skill`: the select only, the tick not used up.
        let mut s = InputScript::new(parse("skill L fireball; move 1 1").unwrap());
        assert_eq!(
            s.events_full(1, &mut unit, &mut known),
            [UiEvent::CursorMoved(p(1, 1))]
        );
        assert_eq!(s.take_selects(), [intent(47, PointerButton::Left)]);

        // `cast ... at X Y`: the select, then the right click a tick later.
        let mut s = InputScript::new(parse("cast fireball at 300 200").unwrap());
        assert_eq!(s.events_full(1, &mut unit, &mut known), []);
        assert_eq!(s.take_selects(), [intent(47, r)]);
        assert_eq!(
            s.events_full(2, &mut unit, &mut known),
            [UiEvent::CursorMoved(at), UiEvent::Press { button: r, at }]
        );
        assert_eq!(
            s.events_full(3, &mut unit, &mut known),
            [UiEvent::Release { button: r, at }]
        );
        assert!(s.done());

        // `cast ... at unit`: the select, the hover a tick later, the
        // right click the tick after.
        let mut s = InputScript::new(parse("cast fireball at unit 1 *").unwrap());
        assert_eq!(s.events_full(1, &mut unit, &mut known), []);
        assert_eq!(s.take_selects().len(), 1);
        assert_eq!(
            s.events_full(2, &mut unit, &mut known),
            [UiEvent::CursorMoved(at)]
        );
        assert_eq!(
            s.events_full(3, &mut unit, &mut known),
            [UiEvent::CursorMoved(at), UiEvent::Press { button: r, at }]
        );

        // Unknown: noted, no select, no click.
        let mut s = InputScript::new(parse("cast nova at unit 1 *; skill R nova").unwrap());
        assert_eq!(s.events_full(1, &mut unit, &mut known), []);
        assert_eq!(s.take_selects(), []);
        assert_eq!(
            s.take_notes(),
            [
                "cast nova at unit 1 * 0 -8: not known",
                "skill R nova: not known"
            ]
        );
        assert!(s.done());
        // Without a skill resolver (`events_with`): noted too.
        let mut s = InputScript::new(parse("skill R fireball").unwrap());
        assert_eq!(s.events(1), []);
        assert_eq!(s.take_notes(), ["skill R fireball: no skill resolver"]);
    }

    #[test]
    fn headless_refuses_what_it_cannot_apply_and_holds_down_buttons() {
        let e =
            Headless::new(parse("frame 1; action toggle_skill_menu_left").unwrap()).unwrap_err();
        assert!(e.contains("is not available headless"), "{e}");
        assert!(Headless::new(parse("frame 1; action toggle_inventory").unwrap()).is_ok());
        let mut none = |_: &UnitSel| -> Result<(UnitKey, Point), String> { Err("none".into()) };
        let mut h = Headless::new(
            parse(
                "frame 2; down L 10 10; action toggle_run; frame 3; move-held 20 20; \
                 rclick 5 5; frame 4; up L 20 20",
            )
            .unwrap(),
        )
        .unwrap();
        let (l, r) = (PointerButton::Left, PointerButton::Right);
        let (ev, log) = h.events(1, &mut none);
        assert_eq!(
            ev,
            [
                UiEvent::CursorMoved(p(10, 10)),
                UiEvent::Press {
                    button: l,
                    at: p(10, 10)
                },
                UiEvent::Action(ActionId(Action::ToggleRun.index() as u16)),
            ]
        );
        assert_eq!(
            log,
            [
                "frame 2: down L 10 10 after frame 1",
                "frame 2: action toggle_run after frame 1"
            ]
        );
        assert!(h.holding());
        let (ev, _) = h.events(2, &mut none);
        assert_eq!(
            ev,
            [
                UiEvent::CursorMoved(p(20, 20)),
                UiEvent::CursorMoved(p(5, 5)),
                UiEvent::Press {
                    button: r,
                    at: p(5, 5)
                },
                UiEvent::Release {
                    button: r,
                    at: p(5, 5)
                },
            ]
        );
        assert!(h.holding(), "left still down after the right click");
        let (ev, _) = h.events(3, &mut none);
        assert_eq!(
            ev,
            [
                UiEvent::CursorMoved(p(20, 20)),
                UiEvent::Release {
                    button: l,
                    at: p(20, 20)
                }
            ]
        );
        assert!(!h.holding());
    }

    mod headless_units {
        use std::sync::{Arc, Mutex};

        use d2_proto::PROTOCOL_VERSION;

        use super::super::*;
        use crate::bridge::dispatch::Dispatch;
        use crate::bridge::link::{LinkError, Pumped, SendQueue, Sent};
        use crate::bridge::skills::{SkillEntry, SkillList};
        use crate::bridge::world::{
            ClientUnit, ItemData, ItemRecord, KindData, PlayerData, SkillRow, MONSTER, PLAYER,
        };

        #[derive(Clone, Default)]
        struct RecordingLink {
            sent: Arc<Mutex<Vec<Vec<u8>>>>,
        }

        impl ServerLink for RecordingLink {
            fn protocol_version(&self) -> u32 {
                PROTOCOL_VERSION
            }
            fn tick_rate(&self) -> d2_sim::tick::TickRate {
                d2_sim::tick::TickRate::BASE
            }
            fn send(&mut self, _: SendQueue, msg: &[u8]) -> Result<Sent, LinkError> {
                self.sent.lock().unwrap().push(msg.to_vec());
                Ok(Sent::Queued)
            }
            fn pump(&mut self) -> Result<Pumped, LinkError> {
                Ok(Pumped { ticked: false })
            }
            fn receive(&mut self) -> Vec<Vec<u8>> {
                Vec::new()
            }
        }

        const ME: UnitKey = UnitKey {
            unit_type: PLAYER,
            guid: 1,
        };

        fn monster(guid: u32, class: u32, cell: (u16, u16), mode: u32) -> ClientUnit {
            let mut u = ClientUnit::new(UnitKey::new(MONSTER, guid));
            u.class = class;
            u.position = Some(cell);
            u.mode = mode;
            u.flag_4 = true;
            u
        }

        /// Synthetic fixture: in game (out of town), the local player at
        /// (100, 100) in mode 1 with stamina and Attack (anim 7, range 1)
        /// as its left skill; monsters 9 (class 19, at (103, 100)), 10
        /// (class 19, at (110, 100)), 11 (class 20, dead, at (101, 101)).
        fn scene() -> (Bridge<RecordingLink>, RecordingLink) {
            let link = RecordingLink::default();
            let mut b = Bridge::with_dispatch(link.clone(), Dispatch::empty()).unwrap();
            b.set_skill_rows(vec![SkillRow {
                anim: 7,
                range: 1,
                ingame: true,
                ..SkillRow::default()
            }]);
            let w = b.world_mut();
            let mut p = ClientUnit::new(ME);
            p.position = Some((100, 100));
            p.server_point = (100, 100);
            p.mode = 1;
            p.kind = KindData::Player(PlayerData::default());
            p.stats.insert(10, 100 << 8);
            p.skills = Some(SkillList {
                entries: vec![SkillEntry {
                    skill: crate::controls::click::ATTACK,
                    mode: 7,
                    base: 1,
                    owner: crate::bridge::skills::NATIVE,
                    ..SkillEntry::default()
                }],
                left: Some(0),
                ..SkillList::default()
            });
            w.units.insert(ME, p);
            w.local_player = Some(ME);
            w.in_game = true;
            for u in [
                monster(9, 19, (103, 100), 1),
                monster(10, 19, (110, 100), 1),
                monster(11, 20, (101, 101), 12),
            ] {
                w.units.insert(u.key, u);
            }
            (b, link)
        }

        fn sel(text: &str) -> UnitSel {
            match parse(text).unwrap()[..] {
                [Step::ClickUnit(s)] => s,
                ref other => panic!("{other:?}"),
            }
        }

        #[test]
        fn clickunit_superunique_names_the_monster_by_its_row() {
            use crate::bridge::world::MonsterData;
            let (mut b, _) = scene();
            let w = b.world_mut();
            let key = UnitKey::new(MONSTER, 9);
            w.units.get_mut(&key).unwrap().kind = KindData::Monster(Box::new(MonsterData {
                flags: 0x02,
                hc_idx: 7,
                ..MonsterData::default()
            }));
            let cam = script_camera(w, None).unwrap();
            let s = sel("rclickunit superunique 7 0 -8");
            assert_eq!(s.superunique, Some(7));
            assert_eq!(s.text(), "rclickunit superunique 7 0 -8");
            let (k, _) = unit_point(w, &cam, &s).unwrap();
            assert_eq!(k.guid, 9, "the superunique, not its class neighbours");
            assert!(unit_point(w, &cam, &sel("rclickunit superunique 8")).is_err());
            assert!(parse("rclickunit superunique x").is_err());
        }

        #[test]
        fn clickunit_parses_its_target_form() {
            let s = sel("clickunit 1 19,0x14");
            assert_eq!(
                (s.button, s.unit_type, &s.classes[..2], s.count, s.offset),
                (PointerButton::Left, 1, &[19, 20][..], 2, (0, -8))
            );
            let r = sel("rclickunit 2 * 3 -12");
            assert_eq!(
                (r.button, r.any(), r.offset),
                (PointerButton::Right, true, (3, -12))
            );
            assert_eq!(r.text(), "rclickunit 2 * 3 -12");
            for bad in [
                "clickunit 1",
                "clickunit 6 19",
                "clickunit 1 x",
                "clickunit 1 19 4",
                "clickunit 1 1,2,3,4,5,6,7,8,9",
            ] {
                assert!(parse(bad).is_err(), "{bad}");
            }
        }

        #[test]
        fn unit_point_is_the_nearest_units_draw_point_plus_the_offset() {
            let (b, _) = scene();
            let w = b.world();
            let cam = script_camera(w, None).unwrap();
            let (k, at) = unit_point(w, &cam, &sel("clickunit 1 19")).unwrap();
            assert_eq!(k, UnitKey::new(MONSTER, 9), "the nearer of the two");
            let (fx, fy) = unit_feet(&cam, MONSTER, (103, 100));
            assert_eq!((at.x, at.y), (fx, fy - 8));
            // the local player stands at the frame centre's draw point
            assert_eq!(unit_feet(&cam, PLAYER, (100, 100)), (400, 292));
            // `*`: living units only (the dead 11 is nearer)
            let (k, _) = unit_point(w, &cam, &sel("clickunit 1 *")).unwrap();
            assert_eq!(k.guid, 9);
            // a listed class keeps the dead
            let (k, _) = unit_point(w, &cam, &sel("clickunit 1 20")).unwrap();
            assert_eq!(k.guid, 11);
            assert!(unit_point(w, &cam, &sel("clickunit 1 99")).is_err());
            assert!(unit_point(w, &cam, &sel("clickunit 1 19 0 -600")).is_err());
        }

        #[test]
        fn headless_clickunit_on_a_monster_sends_the_skill_on_the_unit() {
            let (mut b, link) = scene();
            let mut h = Headless::new(parse("frame 3; clickunit 1 19").unwrap()).unwrap();
            assert!(h.apply(&mut b, 1).unwrap().is_empty());
            // the hover frame: the cursor before frame 3, the click before frame 4
            let log = h.apply(&mut b, 2).unwrap();
            assert!(
                log[0].starts_with("frame 3: clickunit 1 19 0 -8: unit 1:9 at ("),
                "{log:?}"
            );
            assert!(link.sent.lock().unwrap().is_empty(), "no click yet");
            assert_eq!(h.pending(), 2);
            let log = h.apply(&mut b, 3).unwrap();
            assert!(log[0].starts_with("frame 4: click "), "{log:?}");
            let mut want = vec![0x06];
            want.extend_from_slice(&1u32.to_le_bytes());
            want.extend_from_slice(&9u32.to_le_bytes());
            assert_eq!(link.sent.lock().unwrap().as_slice(), [want]);
            assert_eq!(h.pending(), 0);
        }

        // `cast fireball at unit 1 *` headless: C→S 0x3C (the skill
        // menu's select, right hand) before frame 3, the hover before
        // frame 4, the right click on the monster before frame 5.
        #[test]
        fn headless_cast_selects_then_right_clicks_the_unit() {
            use crate::world_view::script_skills::SkillNames;
            let (mut b, link) = scene();
            let me = b.world_mut().units.get_mut(&ME).unwrap();
            me.skills.as_mut().unwrap().entries.push(SkillEntry {
                skill: 47,
                owner: crate::bridge::skills::NATIVE,
                ..SkillEntry::default()
            });
            let names = SkillNames::from_text("skill\tId\nAttack\t0\nFire Ball\t47\n").unwrap();
            let mut h = Headless::new(parse("frame 3; cast fireball at unit 1 *").unwrap())
                .unwrap()
                .with_skill_names(names);
            assert!(h.apply(&mut b, 1).unwrap().is_empty());
            let log = h.apply(&mut b, 2).unwrap();
            assert_eq!(
                log,
                ["frame 3: cast fireball at unit 1 * 0 -8: skill 47 selected after frame 2"]
            );
            let select =
                crate::ui::original::hud_hotkeys::select(47, false, crate::bridge::skills::NATIVE);
            assert_eq!(
                link.sent.lock().unwrap().as_slice(),
                std::slice::from_ref(&select.0)
            );
            // This scene has no server: play its part and put Fire Ball
            // on the right hand, as the server's reply to the select does.
            let list = b.world_mut().units.get_mut(&ME).unwrap();
            let list = list.skills.as_mut().unwrap();
            list.right = list.entries.iter().position(|e| e.skill == 47);
            let log = h.apply(&mut b, 3).unwrap();
            assert!(
                log[0].starts_with("frame 4: rclickunit 1 * 0 -8: unit 1:9 at ("),
                "{log:?}"
            );
            let log = h.apply(&mut b, 4).unwrap();
            assert!(log[0].starts_with("frame 5: rclick "), "{log:?}");
            let sent = link.sent.lock().unwrap();
            assert!(sent.len() >= 2, "the right click sends: {sent:?}");
            assert_eq!(sent[0], select.0);
            assert_eq!(h.pending(), 0);

            // A skill the player lacks: logged, nothing sent, no click.
            let (mut b, link) = scene();
            let names = SkillNames::from_text("skill\tId\nFire Ball\t47\n").unwrap();
            let mut h = Headless::new(parse("frame 1; cast fireball at unit 1 *").unwrap())
                .unwrap()
                .with_skill_names(names);
            let log = h.apply(&mut b, 0).unwrap();
            assert!(
                log[0].contains("not selected (the player does not have skill 47"),
                "{log:?}"
            );
            assert!(link.sent.lock().unwrap().is_empty());
            assert_eq!(h.pending(), 0);
        }

        #[test]
        fn headless_click_on_the_control_panel_strip_never_walks() {
            let (mut b, link) = scene();
            let mut h = Headless::new(parse("frame 1; click 400 590").unwrap()).unwrap();
            let log = h.apply(&mut b, 0).unwrap();
            assert!(link.sent.lock().unwrap().is_empty(), "{log:?}");
            assert!(log
                .iter()
                .any(|l| l.starts_with("control panel strip takes")));
            // the row above the strip is the world's
            let mut h = Headless::new(parse("frame 1; click 400 552").unwrap()).unwrap();
            h.apply(&mut b, 0).unwrap();
            assert_eq!(link.sent.lock().unwrap().len(), 1);
        }

        #[test]
        fn headless_click_on_a_monsters_body_picks_it() {
            let (mut b, link) = scene();
            let cam = script_camera(b.world(), None).unwrap();
            // 40 rows above the feet: the cell under the point is not the
            // monster's, the hover box (hover::pick) still holds it. The
            // cursor moves a frame before the press (the hover is read
            // from the drawn frame).
            let (fx, fy) = unit_feet(&cam, MONSTER, (110, 100));
            let script = format!(
                "frame 1; move {fx} {}; frame 2; click {fx} {}",
                fy - 40,
                fy - 40
            );
            let mut h = Headless::new(parse(&script).unwrap()).unwrap();
            h.apply(&mut b, 0).unwrap();
            h.apply(&mut b, 1).unwrap();
            let sent = link.sent.lock().unwrap();
            assert_eq!(sent.len(), 1, "{sent:?}");
            assert_eq!(sent[0][5..9], 10u32.to_le_bytes(), "{sent:?}");
        }

        #[test]
        fn headless_press_in_the_pass_of_its_move_is_a_point_click() {
            let (mut b, link) = scene();
            let cam = script_camera(b.world(), None).unwrap();
            let (fx, fy) = unit_feet(&cam, MONSTER, (110, 100));
            let script = format!("frame 1; click {fx} {}", fy - 40);
            let mut h = Headless::new(parse(&script).unwrap()).unwrap();
            h.apply(&mut b, 0).unwrap();
            let sent = link.sent.lock().unwrap();
            assert!(
                sent.iter().all(|m| m[0] != 0x06),
                "no skill on a unit nothing hovered: {sent:?}"
            );
        }

        // The quest key sends C→S 0x40 on
        // every open, nothing on the close; other panel keys send nothing.
        #[test]
        fn the_quest_key_asks_for_quest_data_on_each_open() {
            use crate::ui::original::{OriginalUi, UiConfig};
            use crate::ui::panel::NoStrings;
            use crate::ui::NoPanelRules;
            use crate::world_view::ui_bind::{run_ui_with, UiQueue};
            let (mut br, link) = scene();
            let mut ui = OriginalUi::new(
                UiConfig {
                    screen: crate::ui::layout::Screen::play(),
                    expansion_installed: true,
                },
                None,
            )
            .unwrap();
            let mut root = crate::ui::root::UiRoot::new(Box::new(NoPanelRules));
            ui.install(&mut root).unwrap();
            let mut h = Headless::new(
                parse("frame 1; key q; key i; frame 2; key q; frame 3; key q").unwrap(),
            )
            .unwrap();
            for last in 0..3 {
                h.apply_with(&mut br, last, &mut |b, ev| {
                    let mut q = UiQueue(ev);
                    Ok(run_ui_with(&mut root, &mut q, b, &NoStrings, Some(&mut ui))
                        .unwrap()
                        .unhandled)
                })
                .unwrap();
            }
            br.send_outgoing().unwrap();
            let n = link
                .sent
                .lock()
                .unwrap()
                .iter()
                .filter(|m| m.first() == Some(&0x40))
                .count();
            assert_eq!(n, 2);
        }

        // A left click inside the open
        // inventory's `inv` rectangle is the panel's; the world dispatcher
        // never sees it, so the player does not walk. Without the
        // rectangles the panel's area is empty and
        // the same click walks.
        #[test]
        fn a_click_inside_the_open_inventory_never_walks() {
            use crate::ui::original::{InvArea, OriginalUi, UiConfig};
            use crate::ui::panel::NoStrings;
            use crate::ui::NoPanelRules;
            use crate::world_view::ui_bind::{run_ui_with, UiQueue};
            let walks = |inv_areas: Option<Vec<InvArea>>| {
                let (mut br, link) = scene();
                let mut ui = OriginalUi::new(
                    UiConfig {
                        screen: crate::ui::layout::Screen::play(),
                        expansion_installed: true,
                    },
                    inv_areas,
                )
                .unwrap();
                let mut root = crate::ui::root::UiRoot::new(Box::new(NoPanelRules));
                ui.install(&mut root).unwrap();
                let mut h = Headless::new(parse("frame 1; key i; frame 2; click 678 330").unwrap())
                    .unwrap();
                for last in 0..2 {
                    h.set_open_mode(ui.open_mode().get());
                    h.apply_with(&mut br, last, &mut |b, ev| {
                        let mut q = UiQueue(ev);
                        Ok(run_ui_with(&mut root, &mut q, b, &NoStrings, Some(&mut ui))
                            .unwrap()
                            .unhandled)
                    })
                    .unwrap();
                }
                br.send_outgoing().unwrap();
                let sent = link.sent.lock().unwrap();
                sent.iter()
                    .filter(|m| crate::bridge::predict::walk_of(m).is_some())
                    .count()
            };
            // Every record the right panel's half of the 800 × 600 frame.
            let half = InvArea {
                left: 400,
                right: 800,
                top: 0,
                bottom: 440,
            };
            assert_eq!(walks(Some(vec![half; 32])), 0);
            assert_eq!(walks(None), 1);
        }

        #[test]
        fn headless_keys_run_their_world_actions() {
            let b = Preset::Original.bindings().unwrap();
            let act = |k: &str| headless_key_action(&b, vk_code(k).unwrap());
            assert_eq!(act("1"), Ok(Action::BeltSlot1));
            assert_eq!(act("4"), Ok(Action::BeltSlot4));
            assert_eq!(act("R"), Ok(Action::ToggleRun));
            assert_eq!(act("W"), Ok(Action::SwapWeapons));
            // The panel toggles run headless.
            assert_eq!(act("I"), Ok(Action::ToggleInventory));
            assert_eq!(act("Q"), Ok(Action::ToggleQuests));
            // Esc: the game menu, taken by the headless UI.
            assert_eq!(act("ESC"), Ok(Action::GameMenu));
            // The skill hotkeys: taken by the HUD.
            assert_eq!(act("F1"), Ok(Action::SkillSlot1));
            assert_eq!(act("F8"), Ok(Action::SkillSlot8));
            for refused in ["SHIFT", "5"] {
                assert!(act(refused).is_err(), "{refused}");
            }
            assert!(Headless::new(parse("frame 1; key SHIFT").unwrap()).is_err());

            // `key w`: C→S 0x60; `key r` then a ground click: a run.
            let (mut br, link) = scene();
            let mut h =
                Headless::new(parse("frame 1; key w; key r; click 600 450").unwrap()).unwrap();
            let log = h.apply(&mut br, 0).unwrap();
            assert_eq!(log.len(), 3, "{log:?}");
            let sent = link.sent.lock().unwrap();
            let walks: Vec<_> = sent
                .iter()
                .filter_map(|m| crate::bridge::predict::walk_of(m))
                .collect();
            assert_eq!(walks.len(), 1, "{sent:?}");
            assert!(walks[0].run, "the run lock");
            assert_eq!(
                sent.last().unwrap(),
                &vec![0x60],
                "the swap after the click"
            );
        }

        #[test]
        fn play_clickunit_moves_then_clicks_a_tick_later() {
            let at = Point { x: 300, y: 200 };
            let mut found =
                |_: &UnitSel| -> Result<(UnitKey, Point), String> { Ok((UnitKey::new(1, 9), at)) };
            let mut s = InputScript::new(parse("clickunit 1 19; move 1 1").unwrap());
            assert_eq!(s.events_with(1, &mut found), [UiEvent::CursorMoved(at)]);
            let button = PointerButton::Left;
            assert_eq!(
                s.events_with(2, &mut found),
                [UiEvent::CursorMoved(at), UiEvent::Press { button, at }]
            );
            assert_eq!(
                s.events_with(3, &mut found),
                [UiEvent::Release { button, at }]
            );
            let mut none = |_: &UnitSel| -> Result<(UnitKey, Point), String> { Err("x".into()) };
            let mut t = InputScript::new(parse("clickunit 1 19").unwrap());
            assert_eq!(t.events_with(1, &mut none), []);
            assert_eq!(t.take_notes(), ["clickunit 1 19 0 -8: x"]);
            assert!(t.done());
        }

        #[test]
        fn headless_belt_key_sends_0x26() {
            let (mut b, link) = scene();
            let w = b.world_mut();
            // A potion in belt slot 0: flags 0x10, version 0x65, mode 2, x 0.
            let k = UnitKey::new(4, 7);
            let mut s = vec![0u8; 12];
            s[0] = 0x10;
            let mut bits: u64 = 0x65 | (2 << 10);
            for byte in s.iter_mut().skip(4).take(4) {
                *byte = bits as u8;
                bits >>= 8;
            }
            let mut u = ClientUnit::new(k);
            u.kind = KindData::Item(ItemData {
                last: Some(ItemRecord {
                    id: 0x9C,
                    action: 0x0E,
                    category: 0x10,
                    owner: None,
                    seq: 0,
                    stream: s,
                }),
                ..ItemData::default()
            });
            w.units.insert(k, u);
            w.belt_ready[0] = true;
            let mut h = Headless::new(parse("frame 1; key 1").unwrap()).unwrap();
            h.apply(&mut b, 0).unwrap();
            assert_eq!(
                link.sent.lock().unwrap().as_slice(),
                [vec![0x26, 7, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]]
            );
        }

        /// With a prediction the model records it for the position check
        /// (rule 8 takes the server's point); without one nothing is set.
        #[test]
        fn sync_local_records_the_prediction_for_the_check() {
            let (mut b, _) = scene();
            let plain = Headless::new(Vec::new()).unwrap();
            plain.sync_local(&mut b);
            assert!(b.world().predicted(b.world().local().unwrap()).is_none());
            let mut h = Headless::new(Vec::new())
                .unwrap()
                .with_prediction(WalkTap::default(), None);
            h.observe(b.world(), false);
            h.sync_local(&mut b);
            assert!(b.world().predicted(b.world().local().unwrap()).is_some());
        }
    }
}
