//! World clicks: what a left or right mouse click on the game world does.
//!
//! This module holds the left / right button handlers, the click dispatcher with its press /
//! held / release filter, the per-kind steps, the click action with its senders (attack, interact,
//! object, town player, approach), the held-button repeat and the target re-pick. It is plain Rust:
//! everything the dispatcher needs to know about the client model and the client-side tests it
//! runs comes in through [`ClickWorld`], and what the click does comes out as an ordered list of
//! [`ClickOut`]s, which the caller turns into protocol messages and sounds. The dispatcher never
//! decides an outcome: every result is a C->S message (or a client sound) and the server decides
//! (`CLAUDE.md` hard rule 7).
//!
//! Its sibling modules are `super::original` (the key table that decides which button does what)
//! and `super::keymap`; the bridge adapter implements [`ClickWorld`] from live client state.

use crate::bridge::world::UnitKey;

/// Unit types, as the client model numbers them.
const PLAYER: u8 = 0;
const MONSTER: u8 = 1;
const OBJECT: u8 = 2;
const ITEM: u8 = 4;
const TILE: u8 = 5;

/// The six click kinds: each button has a press, a held repeat and a release.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    /// The left button went down.
    LeftDown = 0,
    /// The left button is still down (the repeat while held).
    LeftHeld = 1,
    /// The left button was released.
    LeftUp = 2,
    /// The right button went down.
    RightDown = 3,
    /// The right button is still down.
    RightHeld = 4,
    /// The right button was released.
    RightUp = 5,
}

/// The flags of a click record: which button, which phase, and the modifiers in force.
pub mod flag {
    /// The click comes from the left button.
    pub const LEFT: u32 = 1;
    /// The click comes from the right button.
    pub const RIGHT: u32 = 2;
    /// The button was just pressed.
    pub const PRESS: u32 = 4;
    /// The button is being held (a repeat).
    pub const HELD: u32 = 8;
    /// The button was just released.
    pub const RELEASE: u32 = 0x10;
    /// The Stand Still modifier is active: the click uses the skill in place instead of walking.
    pub const STAND_STILL: u32 = 0x20;
    /// The Run modifier is active and the player has stamina.
    pub const RUN: u32 = 0x40;
    /// The click may walk to its point or unit.
    pub const PATH: u32 = 0x80;
    /// A right click landed on a tile unit (such as a warp) and is treated as a click on its point.
    pub const TILE: u32 = 0x100;
}

/// The modifier word passed with each click.
pub mod mods {
    /// The Stand Still key is down.
    pub const STAND_STILL: u32 = 4;
    /// The Run key is down.
    pub const RUN: u32 = 8;
}

/// `skills.txt` flag bits the click action reads. Bit `n` of the word is `1 << n`.
pub use d2_server::game::client_tables::skill_flag;

/// The reach class of a skill for the player: how close the player must be to use it.
pub mod range {
    /// No reach limit applies.
    pub const NONE: u8 = 0;
    /// Hand to hand: melee reach.
    pub const H2H: u8 = 1;
    /// Ranged.
    pub const RNG: u8 = 2;
    /// Targets a location.
    pub const LOC: u8 = 4;
}

/// The `srvdofunc` value that selects the approach sender for a melee-range skill.
pub const SRVDOFUNC_APPROACH: i16 = 0x13;
/// The interact code: the request to interact with a unit once the player is in reach.
pub const CODE_INTERACT: u8 = 0x13;
/// The town portal object class. Clicking a town portal while the player is in the
/// `just_portaled` state does not interact again.
pub const TOWN_PORTAL: u32 = 59;
/// Skill id 0 (Attack).
pub const ATTACK: u16 = 0;
/// The minimum gap between two walk re-sends of a held walk, in client updates.
pub const HELD_WALK_GAP: u32 = 7;
/// How far from the cursor, in subtiles, a fighting held click looks for its next enemy.
pub const FIGHT_RADIUS: i32 = 5;
/// The minimum gap before a warp (tile) is clicked again, in milliseconds.
pub const WARP_GAP: u32 = 500;
/// The bottom margin of the left panel for a right click in open mode 2: a right click in the left
/// half of the frame, above `frame height - 47`, belongs to the panel and not to the world.
pub const PANEL_BOTTOM: i32 = 47;

/// A skill entry of the player's skill list: its id and its mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SkillRef {
    /// The skill id.
    pub id: u16,
    /// The skill entry's mode value. A skill with mode 0 is not used in place by Stand Still.
    pub mode: u32,
}

/// The `skills.txt` columns the click action reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SkillRowFacts {
    /// Flag bits by [`skill_flag`].
    pub flags: u32,
    /// The skill's `range` column, as the right-down step reads it.
    pub range: u8,
    /// The skill's `srvdofunc` column.
    pub srvdofunc: i16,
}

impl SkillRowFacts {
    fn has(&self, f: u32) -> bool {
        self.flags & f != 0
    }
}

/// A pending interaction: the player is walking toward `target` and will carry out `code` on
/// arrival.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pending {
    /// The code to run on arrival (usually [`CODE_INTERACT`], or 6 / 0xD for a skill use).
    pub code: u8,
    /// The unit the player is walking to.
    pub target: UnitKey,
}

/// What a click does, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClickOut {
    /// A client mode request with arguments `a` and `b`. For codes 1 to 0x11 it also sends the
    /// C->S message whose id is the code; code 0x13 is the interact, which the client model
    /// sends itself.
    Code {
        /// The code (and, for codes 1-0x11, the C->S message id).
        code: u8,
        /// First argument: a subtile x for point codes, a unit type for unit codes.
        a: u32,
        /// Second argument: a subtile y for point codes, a unit GUID for unit codes.
        b: u32,
    },
    /// A C->S message built by the dispatcher itself (ids 0x17, 0x27, 0x4C and 0x59).
    Send(Vec<u8>),
    /// A player event sound to play (for example a refusal sound).
    Sound(u16),
    /// The pending interaction was set, or cleared with `None`.
    Pend(Option<Pending>),
    /// An NPC that can be talked to was clicked: the client holds it in place, after its path
    /// stop.
    NpcHold(UnitKey),
    /// The player's path is re-targeted (a held left click on a different unit, or after a speed
    /// change).
    Retarget,
    /// The hover was dropped (the hovered unit is a corpse the skill cannot target).
    DropHover,
    /// A hover / cursor-unit call, identified by its call id: [`HOVER_REFRESH`],
    /// [`HOVER_CLEAR`] or [`HOVER_HIRELING_STEP`].
    HoverCall(u32),
}

/// Call id: refresh the hover after a press on a unit or a point action.
pub const HOVER_REFRESH: u32 = 0x0046_7A70;
/// Call id: clear the hover on a press without a unit, and on every release.
pub const HOVER_CLEAR: u32 = 0x0046_6FE0;
/// Call id: the hireling / cursor-unit step of a release.
pub const HOVER_HIRELING_STEP: u32 = 0x0046_7410;

/// The state the click handlers keep between calls.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClickState {
    /// The left button is held and its click was accepted, so it repeats.
    pub left_held: bool,
    /// The right button is held and its click was accepted, so it repeats.
    pub right_held: bool,
    /// Which accepted press most recently selected the held repeat.
    /// The other button may still be down from walking or attacking.
    pub repeat_right: bool,
    /// An interaction (object, item) stopped the left repeat until the button is released.
    pub stop_repeat: bool,
    /// One positioned dispatch is allowed per client loop pass; this latches after the first.
    pub latch: bool,
    /// 0, or at a press 1 plus whether a unit was hovered (so 1 or 2). Held repeats use it to
    /// know a press started.
    pub press: u8,
    /// The client update counter at the last walk that was sent.
    pub last_walk: u32,
    /// The time (ms) before which a warp (tile) is not clicked again.
    pub warp_after: u32,
    /// Per button (left, right): the held click is fighting, it last acted on a living enemy.
    /// While it stays held, a repeat whose enemy died or left the cursor goes on with the next
    /// enemy near the cursor, or stands, instead of walking (D2RUST's own).
    pub fighting: [bool; 2],
}

impl ClickState {
    /// Clears the per-pass latch; called once at the end of each client loop pass.
    pub fn end_pass(&mut self) {
        self.latch = false;
    }

    /// The input reset: left and right held are cleared, so no held repeat follows. Only the
    /// click state is reset here; the button states (up) belong to the input edge, and the hover
    /// clear and the other input globals are not modelled.
    pub fn input_reset(&mut self) {
        self.left_held = false;
        self.right_held = false;
        self.fighting = [false; 2];
    }
}

/// The click record: everything one click knows about itself while it is being handled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Click {
    /// The [`flag`] bits.
    pub flags: u32,
    /// The local player.
    pub player: UnitKey,
    /// The unit under the mouse, if any.
    pub unit: Option<UnitKey>,
    /// The clicked world subtile x.
    pub x: i32,
    /// The clicked world subtile y.
    pub y: i32,
    /// The code for walking to a point: 1 (walk) or 3 (run).
    pub walk_point: u8,
    /// The code for walking to a unit: 2 (walk) or 4 (run).
    pub walk_unit: u8,
    /// The skill the clicked button has selected.
    pub skill: Option<SkillRef>,
}

/// What the click dispatcher reads: the client model and the client-side tests it runs (can the
/// player act, is the unit hostile, how far away is it ...). The bridge adapter answers them from
/// live state; tests answer them from a fake world.
pub trait ClickWorld {
    /// Whether the client is in a game.
    fn in_game(&self) -> bool;
    /// The local player (type 0).
    fn player(&self) -> Option<UnitKey>;
    /// The game menu is open.
    fn game_menu_open(&self) -> bool;
    /// The UI open mode: 1 a right panel is open, 2 a left panel is open, 3 world clicks are
    /// ignored anywhere.
    fn open_mode(&self) -> u8;
    /// Frame width and height.
    fn frame_size(&self) -> (i32, i32);
    /// The bottom of the right panel (open mode 1).
    fn right_panel_bottom(&self) -> i32;
    /// The largest mouse y at which skill codes are still sent; with the mouse further down the
    /// screen they send nothing.
    fn skill_y_limit(&self) -> i32;
    /// The current mouse position.
    fn mouse(&self) -> (i32, i32);
    /// The hovered unit.
    fn hover(&self) -> Option<UnitKey>;
    /// Screen position to world subtile.
    fn to_world(&self, x: i32, y: i32) -> (i32, i32);
    /// A unit's world position, if it has one.
    fn position(&self, u: UnitKey) -> Option<(i32, i32)>;
    /// A unit's mode (animation state).
    fn mode(&self, u: UnitKey) -> u32;
    /// A unit's class (monster id, object id ...).
    fn class(&self, u: UnitKey) -> u32;
    /// The player's stamina (stat 10).
    fn stamina(&self) -> i32;
    /// Whether the player may act with `skill` right now.
    fn can_act(&self, skill: Option<SkillRef>) -> bool;
    /// The skill selected on the left button.
    fn left_skill(&self) -> Option<SkillRef>;
    /// The skill selected on the right button.
    fn right_skill(&self) -> Option<SkillRef>;
    /// The player's Attack skill entry.
    fn attack_skill(&self) -> Option<SkillRef>;
    /// The skill's `skills.txt` row.
    fn skill_row(&self, id: u16) -> Option<SkillRowFacts>;
    /// The skill's reach class for the player (a [`range`] value).
    fn range(&self, skill: SkillRef) -> u8;
    /// The use state of the skill: 0 and 5 let the click proceed; other values refuse it (and
    /// 1, 2, 4 fall back to Attack for skills with [`skill_flag::ATTACK_NO_MANA`]).
    fn use_state(&self, skill: SkillRef) -> u32;
    /// The refusal sound of a use state; `None` when the state is outside the sound table.
    fn refusal_sound(&self, state: u32) -> Option<u16>;
    /// The cursor state (6 and 8 send cursor messages on a left press).
    fn cursor_state(&self) -> u32;
    /// The GUID of the unit held on the cursor.
    fn cursor_unit(&self) -> Option<u32>;
    /// The GUID of the item held on the cursor.
    fn cursor_item(&self) -> Option<u32>;
    /// Whether the player has a hireling.
    fn has_hireling(&self) -> bool;
    /// The cursor unit's mode record value.
    fn cursor_unit_mode_record(&self) -> Option<i32>;
    /// Whether the unit stands in a town room.
    fn in_town(&self, u: UnitKey) -> bool;
    /// Whether the unit is dead.
    fn is_dead(&self, u: UnitKey) -> bool;
    /// Whether the unit can be selected (clicked on).
    fn selectable(&self, u: UnitKey) -> bool;
    /// Whether the unit is hostile to the player.
    fn hostile(&self, u: UnitKey) -> bool;
    /// The monster's `monstats` bits: (`npc`, `interact`).
    fn monster_npc_interact(&self, u: UnitKey) -> (bool, bool);
    /// Whether the object has an `objects.txt` row.
    fn object_has_row(&self, u: UnitKey) -> bool;
    /// The distance from the player to the unit.
    fn distance(&self, u: UnitKey) -> i32;
    /// The path distance from the player to the unit.
    fn path_distance(&self, u: UnitKey) -> i32;
    /// Whether the line between the player and the unit is clear.
    fn clear(&self, u: UnitKey) -> bool;
    /// Whether the player stands at the object.
    fn at_object(&self, u: UnitKey) -> bool;
    /// The player has the `just_portaled` state (state 102).
    fn just_portaled(&self) -> bool;
    /// Whether the player is in melee range of the unit.
    fn melee_range(&self, u: UnitKey) -> bool;
    /// Whether the player is moving.
    fn moving(&self) -> bool;
    /// The player's current target; the player counts as busy when there is one.
    fn current_target(&self) -> Option<UnitKey>;
    /// Whether the player's speed changed with the Run modifier `running`, so a held walk must
    /// re-target its path.
    fn speed_changed(&self, running: bool) -> bool;
    /// The pending interaction, if the player has one in progress.
    fn pending(&self) -> Option<Pending>;
    /// The skill range used when approaching a target.
    fn approach_range(&self, skill: SkillRef) -> i32;
    /// The path compute for a walk: target (x, y) gives the path's end point, `None` when there
    /// is no path.
    fn walk_path(&self, x: i32, y: i32) -> Option<(i32, i32)>;
    /// The client update counter.
    fn update_counter(&self) -> u32;
    /// The client clock in milliseconds.
    fn now_ms(&self) -> u32;
    /// The player's direction (low byte).
    fn direction(&self) -> u8;
    /// The target re-pick search for skills with [`skill_flag::SEARCH_ENEMY_NEAR`]: `Some(unit)`
    /// replaces the target, `None` leaves it unchanged. It may move the point `x`, `y`.
    fn search_enemy_near(
        &self,
        row: SkillRowFacts,
        skill: SkillRef,
        u: Option<UnitKey>,
        x: &mut i32,
        y: &mut i32,
    ) -> Option<UnitKey>;
    /// The target re-pick search for skills with [`skill_flag::SEARCH_ENEMY_XY`]: the enemy at
    /// the clicked point, if any.
    fn search_enemy_xy(&self, row: SkillRowFacts) -> Option<UnitKey>;
    /// The living, selectable enemy (a hostile player or monster) nearest the subtile (`x`,
    /// `y`) within `radius` subtiles, if any; ties go to the lower unit key.
    fn enemy_near(&self, x: i32, y: i32, radius: i32) -> Option<UnitKey>;
    /// The target re-pick search for skills with [`skill_flag::SEARCH_OPEN_XY`]. `None` leaves the
    /// target unchanged; `Some(answer)` replaces it with `answer` (which may be no unit). It may
    /// move the point `x`, `y`.
    fn search_open_xy(
        &self,
        u: Option<UnitKey>,
        x: &mut i32,
        y: &mut i32,
    ) -> Option<Option<UnitKey>>;
}

/// Whether `code` is a point code, whose C->S message carries `[x u16][y u16]`.
pub fn is_point_code(code: u8) -> bool {
    matches!(code, 1 | 3 | 5 | 8 | 0x0C | 0x0F)
}

/// The C->S bytes of a code with arguments `a` and `b`: point codes are `[code][a u16][b u16]`,
/// unit codes are `[code][a u32][b u32]`. Codes outside 1-0x11 (and 0x0B) send nothing here.
pub fn code_bytes(code: u8, a: u32, b: u32) -> Option<Vec<u8>> {
    if !(1..=0x11).contains(&code) || code == 0x0B {
        return None;
    }
    let mut m = vec![code];
    if is_point_code(code) {
        m.extend_from_slice(&(a as u16).to_le_bytes());
        m.extend_from_slice(&(b as u16).to_le_bytes());
    } else {
        m.extend_from_slice(&a.to_le_bytes());
        m.extend_from_slice(&b.to_le_bytes());
    }
    Some(m)
}

fn send_u32(id: u8, a: u32, b: Option<u32>) -> Vec<u8> {
    let mut m = vec![id];
    m.extend_from_slice(&a.to_le_bytes());
    if let Some(b) = b {
        m.extend_from_slice(&b.to_le_bytes());
    }
    m
}

/// Handles one world click. `at` is the event position for the press kinds (`LeftDown`,
/// `RightDown`) and `RightUp`, or `None` to use the current mouse (the held kinds and `LeftUp`).
/// Returns the dispatcher's result: whether the click was accepted (a press that is accepted
/// starts the held repeat).
pub fn click(
    st: &mut ClickState,
    w: &dyn ClickWorld,
    kind: Kind,
    at: Option<(i32, i32)>,
    mods_word: u32,
    out: &mut Vec<ClickOut>,
) -> bool {
    // A click needs a running game with a local player.
    if !w.in_game() {
        return false;
    }
    let Some(p) = w.player() else {
        return false;
    };
    let (x, y) = at.unwrap_or_else(|| w.mouse());
    match kind {
        Kind::LeftUp => {
            st.left_held = false;
            st.fighting[0] = false;
        }
        Kind::RightUp => {
            st.right_held = false;
            st.fighting[1] = false;
        }
        Kind::RightDown => {
            let (fw, fh) = w.frame_size();
            let mode = w.open_mode();
            let frame = crate::rules::camera::FrameSize {
                width: fw,
                height: fh,
            };
            let open = crate::rules::camera::OpenMode::new(mode).unwrap_or_default();
            if !frame.draws_world(open) {
                return false;
            }
            // A right press under an open side panel is not a world click;
            // a panel takes `span` columns from its edge (W / 2 at 4:3).
            let span = frame.side_panel_span();
            let in_left =
                matches!(mode, 2 | 3) && 0 < x && x < span && 0 < y && y < fh - PANEL_BOTTOM;
            let in_right = matches!(mode, 1 | 3)
                && fw - span < x
                && x < fw
                && 0 < y
                && y < w.right_panel_bottom();
            if in_left || in_right {
                return false;
            }
        }
        _ => {}
    }
    let r = dispatch(st, w, p, kind, x, y, mods_word, out);
    match kind {
        Kind::LeftDown => {
            st.left_held = r;
            if r {
                st.repeat_right = false;
            }
        }
        Kind::RightDown => {
            st.right_held = r;
            if r {
                st.repeat_right = true;
            }
        }
        Kind::LeftUp => st.stop_repeat = false,
        _ => {}
    }
    r
}

/// Repeat the most recently pressed button while it remains held. The
/// per-pass dispatch latch permits only one positioned action, so trying
/// the left button first would replace a held right-hand cast with an
/// older walk or attack. Releasing the active button resumes the other.
pub fn held_repeat(
    st: &mut ClickState,
    w: &dyn ClickWorld,
    mods_word: u32,
    out: &mut Vec<ClickOut>,
) {
    if st.right_held && (!st.left_held || st.repeat_right || st.stop_repeat) {
        click(st, w, Kind::RightHeld, None, mods_word, out);
    } else if st.left_held && !st.stop_repeat {
        click(st, w, Kind::LeftHeld, None, mods_word, out);
    }
}

/// The click dispatcher: builds the click record, filters it, then runs the step of its kind.
#[allow(clippy::too_many_arguments)]
fn dispatch(
    st: &mut ClickState,
    w: &dyn ClickWorld,
    p: UnitKey,
    kind: Kind,
    sx: i32,
    sy: i32,
    mods_word: u32,
    out: &mut Vec<ClickOut>,
) -> bool {
    if w.game_menu_open() {
        return false;
    }
    if st.latch && !(sx == 0 && sy == 0) {
        return false;
    }
    st.latch = true;
    let hover = w.hover();
    let (mut x, mut y) = w.to_world(sx, sy);
    if let Some(u) = hover.filter(|u| matches!(u.unit_type, OBJECT | ITEM)) {
        if let Some(pos) = w.position(u) {
            (x, y) = pos;
        }
    }
    let mut flags = match kind {
        Kind::LeftDown | Kind::LeftHeld | Kind::LeftUp => flag::LEFT,
        _ => flag::RIGHT,
    };
    flags |= match kind {
        Kind::LeftDown | Kind::RightDown => flag::PRESS,
        Kind::LeftHeld | Kind::RightHeld => flag::HELD,
        _ => flag::RELEASE,
    };
    if mods_word & mods::STAND_STILL != 0 {
        flags |= flag::STAND_STILL;
    }
    if mods_word & mods::RUN != 0 && w.stamina() != 0 {
        flags |= flag::RUN;
    }
    match kind {
        Kind::LeftDown | Kind::RightDown => {
            out.push(ClickOut::HoverCall(if hover.is_some() {
                HOVER_REFRESH
            } else {
                HOVER_CLEAR
            }));
        }
        Kind::LeftUp | Kind::RightUp => out.push(ClickOut::HoverCall(HOVER_CLEAR)),
        _ => {}
    }
    let mut c = Click {
        flags,
        player: p,
        unit: hover,
        x,
        y,
        walk_point: 0,
        walk_unit: 0,
        skill: None,
    };
    // A release only clears the press latch; the release kinds' own step (the hireling /
    // cursor-unit call and, for `LeftUp`, the path end) still runs and returns false.
    if !filter(st, w, &c) && c.flags & flag::RELEASE == 0 {
        return false;
    }
    match kind {
        Kind::LeftDown => {
            c.skill = w.left_skill();
            if !w.can_act(c.skill) {
                cursor_sends(w, out);
                if let Some(item) = w.cursor_item() {
                    out.push(ClickOut::Send(send_u32(0x17, item, None)));
                }
                return false;
            }
            c.flags |= flag::PATH;
            action(st, w, &mut c, out);
            st.fighting[0] = live_enemy(w, c.unit);
            true
        }
        Kind::LeftHeld => {
            c.skill = w.left_skill();
            if !w.can_act(c.skill) {
                return false;
            }
            if st.fighting[0] && !live_enemy(w, c.unit) {
                // The enemy died or left the cursor: on to the next one, or stand.
                match w.enemy_near(c.x, c.y, FIGHT_RADIUS) {
                    Some(u) => c.unit = Some(u),
                    None => return true,
                }
            }
            let running = c.flags & flag::RUN != 0;
            let changed = w.speed_changed(running);
            if let Some(u) = c.unit {
                if w.current_target() != Some(u) || changed {
                    out.push(ClickOut::Retarget);
                    c.flags |= flag::PATH;
                }
            } else if c.flags & flag::STAND_STILL == 0 {
                if w.pending().is_some() {
                    return true;
                }
                c.flags |= flag::PATH;
            }
            action(st, w, &mut c, out);
            st.fighting[0] = live_enemy(w, c.unit);
            true
        }
        Kind::LeftUp => {
            release_step(w, out);
            // If the player is not busy: walking (mode 2 or 6) sends walk code 1 and running
            // (mode 3) sends run code 3, both to the click record's point. So the path is
            // re-targeted at the release point and its new end is sent.
            let mode = w.mode(p);
            if w.current_target().is_none() {
                let code = match mode {
                    2 | 6 => Some(1),
                    3 => Some(3),
                    _ => None,
                };
                if let Some(code) = code {
                    walk_to(w, (c.x, c.y), code, out);
                }
            }
            false
        }
        Kind::RightDown | Kind::RightHeld => {
            c.skill = w.right_skill();
            if !w.can_act(c.skill) {
                return false;
            }
            let h2h = c
                .skill
                .and_then(|s| w.skill_row(s.id))
                .is_some_and(|r| r.range == range::H2H);
            if kind == Kind::RightHeld && st.fighting[1] && !live_enemy(w, c.unit) {
                // As the left repeat; a ranged skill with no enemy near casts at the cursor.
                match w.enemy_near(c.x, c.y, FIGHT_RADIUS) {
                    Some(u) => c.unit = Some(u),
                    None if h2h => return true,
                    None => {}
                }
            }
            if h2h && c.flags & flag::RUN == 0 {
                c.flags |= flag::PATH;
            }
            action(st, w, &mut c, out);
            st.fighting[1] = live_enemy(w, c.unit);
            true
        }
        Kind::RightUp => {
            release_step(w, out);
            false
        }
    }
}

/// The hireling / cursor-unit step of the two release kinds.
fn release_step(w: &dyn ClickWorld, out: &mut Vec<ClickOut>) {
    if w.has_hireling() && w.cursor_unit().is_some() && w.cursor_unit_mode_record() == Some(2) {
        out.push(ClickOut::HoverCall(HOVER_HIRELING_STEP));
    }
}

/// The messages sent for cursor states 6 and 8 when a click is pressed.
fn cursor_sends(w: &dyn ClickWorld, out: &mut Vec<ClickOut>) {
    match w.cursor_state() {
        6 => {
            let g = w.cursor_unit().unwrap_or(u32::MAX);
            out.push(ClickOut::Send(send_u32(0x27, g, Some(g))));
        }
        8 => out.push(ClickOut::Send(send_u32(0x4C, u32::MAX, None))),
        _ => {}
    }
}

/// The press / held / release filter. A release clears the press latch and is rejected. A press
/// records whether a unit was hovered and is accepted. A held click is accepted only after a
/// press; with a unit under the mouse, only for players, monsters and tiles while the player is
/// not walking, running or in mode 6.
fn filter(st: &mut ClickState, w: &dyn ClickWorld, c: &Click) -> bool {
    if c.flags & flag::RELEASE != 0 {
        // Releasing one button ends its press, but a press of the other
        // button that is still held keeps going: walking with the left
        // button held carries on after a right-click cast.
        let other_held = if c.flags & flag::LEFT != 0 {
            st.right_held
        } else {
            st.left_held
        };
        if !other_held {
            st.press = 0;
        }
        return false;
    }
    if c.flags & flag::PRESS != 0 {
        st.press = 1 + u8::from(c.unit.is_some());
        return true;
    }
    if st.press == 0 {
        return false;
    }
    match c.unit {
        None => true,
        Some(u) => {
            matches!(u.unit_type, PLAYER | MONSTER | TILE) && !matches!(w.mode(c.player), 2 | 3 | 6)
        }
    }
}

/// Whether `u` is a living enemy: a hostile player or monster that is not dead.
fn live_enemy(w: &dyn ClickWorld, u: Option<UnitKey>) -> bool {
    u.is_some_and(|u| matches!(u.unit_type, PLAYER | MONSTER) && w.hostile(u) && !w.is_dead(u))
}

/// Emits a [`ClickOut::Code`].
fn code(out: &mut Vec<ClickOut>, code: u8, a: u32, b: u32) {
    out.push(ClickOut::Code { code, a, b });
}

fn pend(out: &mut Vec<ClickOut>, code: u8, target: UnitKey) {
    out.push(ClickOut::Pend(Some(Pending { code, target })));
}

/// The walk to a point: computes the path; with no path nothing is sent, otherwise the code is
/// emitted with the path's end point.
fn walk_to(w: &dyn ClickWorld, at: (i32, i32), c: u8, out: &mut Vec<ClickOut>) -> bool {
    match w.walk_path(at.0, at.1) {
        Some((ex, ey)) => {
            code(out, c, ex as u32, ey as u32);
            true
        }
        None => false,
    }
}

/// The skill codes: the use of the selected skill on a unit or at a point. The code depends on
/// the button, Stand Still and whether the click is a repeat; nothing is sent when the mouse is
/// below the skill y limit.
fn skill_code(w: &dyn ClickWorld, c: &Click, on_unit: bool, out: &mut Vec<ClickOut>) {
    if w.mouse().1 > w.skill_y_limit() {
        return;
    }
    let held = c.flags & flag::HELD != 0;
    let ss = c.flags & flag::STAND_STILL != 0;
    let right = c.flags & flag::RIGHT != 0;
    if on_unit {
        let (t, g) = c
            .unit
            .map_or((6, u32::MAX), |u| (u32::from(u.unit_type), u.guid));
        let id = match (right, ss, held) {
            (false, false, false) => 0x06,
            (false, false, true) => 0x09,
            (false, true, false) => 0x07,
            (false, true, true) => 0x0A,
            (true, false, false) => 0x0D,
            (true, false, true) => 0x10,
            (true, true, false) => 0x0E,
            (true, true, true) => 0x11,
        };
        code(out, id, t, g);
    } else {
        let id = match (right, held) {
            (false, false) => 0x05,
            (false, true) => 0x08,
            (true, false) => 0x0C,
            (true, true) => 0x0F,
        };
        code(out, id, c.x as u32, c.y as u32);
    }
}

/// The use check: asks the world for the skill's use state. States 0 and 5 pass. A refused use
/// plays the state's refusal sound. Skills with `ATTACK_NO_MANA` fall back to Attack in states
/// 1, 2 and 4. Returns (passes, state).
fn use_check(w: &dyn ClickWorld, c: &mut Click, out: &mut Vec<ClickOut>) -> (bool, u32) {
    if Some(c.player) != w.player() {
        return (true, 0);
    }
    let Some(skill) = c.skill else {
        return (false, 0);
    };
    let Some(row) = w.skill_row(skill.id) else {
        return (false, 0);
    };
    let mut s = w.use_state(skill);
    if matches!(s, 1 | 2 | 4) && row.has(skill_flag::ATTACK_NO_MANA) {
        c.skill = w.attack_skill();
        s = c.skill.map_or(s, |k| w.use_state(k));
    }
    if matches!(s, 0 | 5) {
        return (true, s);
    }
    if let Some(sound) = w.refusal_sound(s) {
        if sound != 0 {
            out.push(ClickOut::Sound(sound));
        }
    }
    (false, s)
}

/// The click action: checks the skill, re-picks the target, then acts on a unit or a point.
fn action(st: &mut ClickState, w: &dyn ClickWorld, c: &mut Click, out: &mut Vec<ClickOut>) {
    let right = c.flags & flag::RIGHT != 0;
    let ss = c.flags & flag::STAND_STILL != 0;
    let run = c.flags & flag::RUN != 0;
    // Set-up: a right click with a non-melee skill is refused in town unless the skill is allowed
    // there (sound 0x13), and passive skills do nothing.
    let rng = c.skill.map_or(range::NONE, |s| w.range(s));
    if let Some(skill) = c.skill {
        if right && rng != range::H2H {
            let Some(row) = w.skill_row(skill.id) else {
                return;
            };
            if !row.has(skill_flag::IN_TOWN) && w.in_town(c.player) {
                out.push(ClickOut::Sound(0x13));
                return;
            }
            if row.has(skill_flag::PASSIVE) {
                return;
            }
        }
    }
    (c.walk_point, c.walk_unit) = if run { (3, 4) } else { (1, 2) };
    if w.pending().is_some() {
        out.push(ClickOut::Pend(None));
    }
    let Some(skill) = c.skill else {
        return;
    };
    let row = w.skill_row(skill.id);
    repick(w, c, row, ss || (right && c.unit.is_none()), out);
    cursor_sends(w, out);
    // Decide between acting on the unit or on the point.
    let mut item_skill = false;
    let on_unit = match (row, c.unit) {
        (Some(row), Some(u)) => {
            let mut to_point = false;
            if right && rng != range::H2H {
                match u.unit_type {
                    OBJECT | ITEM => {
                        if row.has(skill_flag::TARGET_ITEM) {
                            item_skill = true;
                        } else {
                            to_point = true;
                        }
                    }
                    TILE => {
                        c.flags |= flag::TILE;
                        to_point = true;
                    }
                    _ => {}
                }
            }
            if !to_point
                && u.unit_type == MONSTER
                && w.is_dead(u)
                && !row.has(skill_flag::TARGET_CORPSE)
            {
                to_point = true;
            }
            !to_point
        }
        _ => false,
    };
    if on_unit {
        unit_action(st, w, c, row.expect("checked"), rng, item_skill, out);
    } else {
        point_action(st, w, c, rng, out);
    }
}

/// The click action on a unit.
#[allow(clippy::too_many_arguments)]
fn unit_action(
    st: &mut ClickState,
    w: &dyn ClickWorld,
    c: &mut Click,
    row: SkillRowFacts,
    rng: u8,
    item_skill: bool,
    out: &mut Vec<ClickOut>,
) {
    let u = c.unit.expect("unit action");
    let (t, g) = (u.unit_type, u.guid);
    let right = c.flags & flag::RIGHT != 0;
    let ss = c.flags & flag::STAND_STILL != 0;
    // A right click with a non-melee skill never acts on an object, item or tile, unless the
    // skill targets items.
    if !item_skill && matches!(t, OBJECT | ITEM | TILE) && right && rng != range::H2H {
        return;
    }
    let skill = c.skill.expect("action has a skill");
    // A skill with mode 0 cannot be used in place (Stand Still).
    let mode_zero = skill.mode == 0;
    if mode_zero && ss {
        return;
    }
    if !mode_zero {
        // Item-targeting skills use the skill on the unit once the use check passes.
        if row.has(skill_flag::TARGET_ITEM) {
            if use_check(w, c, out).0 {
                skill_code(w, c, true, out);
            }
            return;
        }
        // A selectable unit that is a pet or ally target, or hostile, is acted on.
        let act = w.selectable(u)
            && (row.has(skill_flag::TARGET_PET)
                || row.has(skill_flag::TARGET_ALLY)
                || w.hostile(u));
        if act {
            match t {
                OBJECT => {
                    if !ss {
                        object_sender(st, w, c, out);
                    }
                }
                PLAYER => {
                    if w.in_town(u) && w.in_town(c.player) {
                        town_player(w, c, out);
                    } else {
                        attack(w, c, row, rng, out);
                    }
                }
                _ => {
                    if w.in_town(u) && !row.has(skill_flag::IN_TOWN) {
                        return;
                    }
                    attack(w, c, row, rng, out);
                }
            }
            return;
        }
        // Stand Still on anything else uses the skill at the point.
        if ss {
            if use_check(w, c, out).0 {
                skill_code(w, c, false, out);
            }
            return;
        }
    }
    // Otherwise interact: non-monsters directly; a monster only on a press, by talking if it is
    // interactable and walking toward it if not.
    if t != MONSTER {
        interact(st, w, c, out);
    } else if c.flags & flag::PRESS != 0 {
        if w.monster_npc_interact(u).1 {
            interact(st, w, c, out);
        } else {
            code(out, c.walk_unit, u32::from(t), g);
        }
    }
}

/// The click action on a point.
fn point_action(
    st: &mut ClickState,
    w: &dyn ClickWorld,
    c: &mut Click,
    rng: u8,
    out: &mut Vec<ClickOut>,
) {
    let right = c.flags & flag::RIGHT != 0;
    let ss = c.flags & flag::STAND_STILL != 0;
    if ss || (right && rng != range::H2H) {
        let (ok, s) = use_check(w, c, out);
        if !ok {
            return;
        }
        let attack_no_mana = c.skill.is_some_and(|k| k.id == ATTACK) && s == 1 && !ss;
        if !attack_no_mana {
            skill_code(w, c, false, out);
            out.push(ClickOut::HoverCall(HOVER_REFRESH));
            return;
        }
    }
    if walk_clamp(st, w, c) && walk_to(w, (c.x, c.y), c.walk_point, out) {
        out.push(ClickOut::HoverCall(HOVER_REFRESH));
    }
}

/// The walk clamp: limits how far a walk click reaches. Points 0x100 or more subtiles away (or
/// the player's own point) are rejected. A point nearer than the minimum step (3 on a press,
/// 4 on a held walk, 5 when running) is pushed out to that step along the same direction. A held
/// walk is re-sent at most once per [`HELD_WALK_GAP`] client updates. Returns whether the walk
/// should be sent.
pub fn walk_clamp(st: &mut ClickState, w: &dyn ClickWorld, c: &mut Click) -> bool {
    let Some((px, py)) = w.position(c.player) else {
        return false;
    };
    let (mut dx, mut dy) = (c.x - px, c.y - py);
    if dx.abs() >= 0x100 || dy.abs() >= 0x100 || (dx == 0 && dy == 0) {
        return false;
    }
    let (ax, ay) = (dx.abs(), dy.abs());
    let len = ((ax.max(ay) * 0x3D7 + ax.min(ay) * 0x197) >> 10).max(1);
    let run = c.flags & flag::RUN != 0;
    let t = if c.flags & flag::PRESS != 0 {
        if run {
            5
        } else {
            3
        }
    } else if c.flags & flag::HELD != 0 {
        if run {
            5
        } else {
            4
        }
    } else {
        0
    };
    if len < t {
        // The scale is a 32-bit float and the products truncate toward zero.
        let f = t as f32 / len as f32;
        dx = (dx as f32 * f) as i32;
        dy = (dy as f32 * f) as i32;
    }
    c.x = px + dx;
    c.y = py + dy;
    let n = w.update_counter();
    if c.player.unit_type == PLAYER
        && c.flags & flag::HELD != 0
        && matches!(w.mode(c.player), 2 | 3 | 6)
        && n.wrapping_sub(st.last_walk) < HELD_WALK_GAP
    {
        return false;
    }
    st.last_walk = n;
    true
}

/// The interact sender: interacts with the clicked unit directly when in reach, otherwise walks
/// toward it and leaves a pending interaction to run on arrival. Reach and behaviour differ per
/// unit type.
fn interact(st: &mut ClickState, w: &dyn ClickWorld, c: &Click, out: &mut Vec<ClickOut>) {
    let u = c.unit.expect("interact target");
    let (t, g) = (u32::from(u.unit_type), u.guid);
    let held = c.flags & flag::HELD != 0;
    let d = w.distance(u);
    let clear = w.clear(u);
    let pending_same = w.pending().is_some_and(|p| p.target == u);
    match u.unit_type {
        PLAYER => {
            if d <= 4 && clear {
                code(out, CODE_INTERACT, t, g);
            } else {
                code(out, c.walk_unit, t, g);
                pend(out, CODE_INTERACT, u);
            }
        }
        OBJECT => {
            if held {
                return;
            }
            let notify = !(w.class(u) == TOWN_PORTAL && w.just_portaled());
            if w.at_object(u) && clear {
                if notify {
                    code(out, CODE_INTERACT, t, g);
                    st.stop_repeat = true;
                }
                return;
            }
            if pending_same {
                return;
            }
            let at = w.position(u).unwrap_or((c.x, c.y));
            walk_to(w, at, c.walk_point, out);
            if notify {
                pend(out, CODE_INTERACT, u);
                st.stop_repeat = true;
            }
        }
        ITEM => {
            if held {
                return;
            }
            if d <= 4 && clear {
                code(out, CODE_INTERACT, t, g);
                st.stop_repeat = true;
                return;
            }
            if pending_same {
                return;
            }
            code(out, c.walk_unit, t, g);
            pend(out, CODE_INTERACT, u);
            st.stop_repeat = true;
        }
        TILE => {
            let now = w.now_ms();
            if now < st.warp_after {
                return;
            }
            if d > 4 {
                code(out, c.walk_unit, t, g);
                pend(out, CODE_INTERACT, u);
            } else {
                code(out, CODE_INTERACT, t, g);
            }
            st.warp_after = now.wrapping_add(WARP_GAP);
        }
        _ => {
            // A monster (reach 2 when it can be interacted with, else 5) or another unit such as a
            // missile (reach 5); an NPC that can be talked to also gets its position sent and is
            // held.
            let (npc, inter) = if u.unit_type == MONSTER {
                w.monster_npc_interact(u)
            } else {
                (false, false)
            };
            let reach = if u.unit_type == MONSTER && inter {
                2
            } else {
                5
            };
            if u.unit_type == MONSTER && npc && inter {
                if let Some((x, y)) = w.position(u) {
                    let mut m = vec![0x59];
                    m.extend_from_slice(&t.to_le_bytes());
                    m.extend_from_slice(&g.to_le_bytes());
                    m.extend_from_slice(&(x as u32).to_le_bytes());
                    m.extend_from_slice(&(y as u32).to_le_bytes());
                    out.push(ClickOut::Send(m));
                }
                out.push(ClickOut::NpcHold(u));
            }
            if d > reach {
                code(out, c.walk_unit, t, g);
                pend(out, CODE_INTERACT, u);
            } else {
                code(out, CODE_INTERACT, t, g);
            }
        }
    }
}

/// The attack sender: uses the skill on the unit when in melee range, approaches first for
/// melee approach skills, walks toward the unit for other melee skills, and otherwise uses the
/// skill on the unit at range.
fn attack(w: &dyn ClickWorld, c: &mut Click, row: SkillRowFacts, rng: u8, out: &mut Vec<ClickOut>) {
    if !use_check(w, c, out).0 {
        return;
    }
    let Some(u) = c.unit else {
        return;
    };
    if w.position(u).is_none() {
        return;
    }
    if w.melee_range(u) {
        skill_code(w, c, true, out);
        return;
    }
    if rng == range::H2H && row.srvdofunc == SRVDOFUNC_APPROACH {
        approach(w, c, out);
        return;
    }
    if rng == range::H2H {
        if c.flags & flag::STAND_STILL != 0 {
            skill_code(w, c, false, out);
        } else {
            code(out, c.walk_unit, u32::from(u.unit_type), u.guid);
            if c.flags & flag::LEFT != 0 {
                pend(out, 6, u);
            }
            if c.flags & flag::RIGHT != 0 {
                pend(out, 0xD, u);
            }
        }
        return;
    }
    skill_code(w, c, true, out);
}

/// The approach: uses the skill when the path distance is within the skill's range, otherwise
/// leaves a pending skill use and walks to the point at skill range along the line to the
/// unit.
fn approach(w: &dyn ClickWorld, c: &Click, out: &mut Vec<ClickOut>) {
    if c.flags & flag::STAND_STILL != 0 {
        skill_code(w, c, true, out);
        return;
    }
    let u = c.unit.expect("approach target");
    let skill = c.skill.expect("approach skill");
    let range = w.approach_range(skill);
    let dist = w.path_distance(u);
    if dist <= range {
        skill_code(w, c, true, out);
        return;
    }
    if c.flags & flag::LEFT != 0 {
        pend(out, 6, u);
    }
    if c.flags & flag::RIGHT != 0 {
        pend(out, 0xD, u);
    }
    let (Some((px, py)), Some((ux, uy))) = (w.position(c.player), w.position(u)) else {
        return;
    };
    let x = px + (ux - px) * (dist - range) / dist;
    let y = py + (uy - py) * (dist - range) / dist;
    walk_to(w, (x, y), c.walk_point, out);
}

/// The object sender: interacts with the object when the player stands at it, otherwise walks
/// to it with a pending interaction. An object without an `objects.txt` row sends nothing.
fn object_sender(st: &mut ClickState, w: &dyn ClickWorld, c: &Click, out: &mut Vec<ClickOut>) {
    let u = c.unit.expect("object target");
    if w.position(u).is_none() || !w.object_has_row(u) {
        return;
    }
    let _ = st;
    let notify = !(w.class(u) == TOWN_PORTAL && w.just_portaled());
    if w.at_object(u) && w.clear(u) {
        if notify {
            code(out, CODE_INTERACT, u32::from(OBJECT), u.guid);
        }
        return;
    }
    let at = w.position(u).expect("checked");
    walk_to(w, at, c.walk_point, out);
    if notify {
        pend(out, CODE_INTERACT, u);
    }
}

/// The town player sender: interacts with a player in town when within 3 and clear, otherwise
/// walks to them with a pending interaction.
fn town_player(w: &dyn ClickWorld, c: &Click, out: &mut Vec<ClickOut>) {
    let u = c.unit.expect("player target");
    if w.distance(u) < 3 && w.clear(u) {
        code(out, CODE_INTERACT, 0, u.guid);
    } else {
        code(out, c.walk_unit, 0, u.guid);
        pend(out, CODE_INTERACT, u);
    }
}

/// The nudge: a click on the player's own point is moved two subtiles ahead in the player's
/// facing direction (the direction byte divided into eight steps).
pub fn nudge(p: (i32, i32), direction: u8) -> (i32, i32) {
    const A: [i32; 8] = [0, -1, -2, -1, 0, 1, 2, 1];
    const B: [i32; 8] = [2, 1, 0, -1, -2, -1, 0, 1];
    let d = usize::from(direction >> 3);
    (p.0 + A[d], p.1 + B[d])
}

/// The target re-pick: drops a dead hovered unit the skill cannot target, nudges a click on the
/// player's own point, then lets the skill's search flags choose a new target or point.
fn repick(
    w: &dyn ClickWorld,
    c: &mut Click,
    row: Option<SkillRowFacts>,
    force: bool,
    out: &mut Vec<ClickOut>,
) {
    let row = row.unwrap_or_default();
    if let Some(u) = c.unit {
        if u.unit_type == PLAYER && w.is_dead(u) {
            return;
        }
        if w.is_dead(u) && !row.has(skill_flag::TARGET_CORPSE) {
            out.push(ClickOut::DropHover);
            if let Some(pos) = w.position(u) {
                (c.x, c.y) = pos;
            }
            c.unit = None;
        }
    } else if Some((c.x, c.y)) == w.position(c.player) {
        (c.x, c.y) = nudge((c.x, c.y), w.direction());
    }
    let held_left = c.flags & flag::HELD != 0 && c.flags & flag::LEFT != 0;
    if held_left && c.flags & flag::STAND_STILL == 0 && w.moving() {
        return;
    }
    let Some(skill) = c.skill else {
        return;
    };
    if w.use_state(skill) != 0 {
        return;
    }
    if row.has(skill_flag::SEARCH_ENEMY_NEAR) {
        if let Some(n) = w.search_enemy_near(row, skill, c.unit, &mut c.x, &mut c.y) {
            c.unit = Some(n);
        }
    } else if c.unit.is_none() && row.has(skill_flag::SEARCH_ENEMY_XY) && force {
        c.unit = w.search_enemy_xy(row);
    } else if row.has(skill_flag::SEARCH_OPEN_XY) {
        if let Some(u) = w.search_open_xy(c.unit, &mut c.x, &mut c.y) {
            c.unit = u;
        }
    }
}
