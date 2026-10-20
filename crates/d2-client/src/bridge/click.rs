//! The world-click dispatcher ([`crate::controls::click`]) against the
//! client model: [`ModelClick`] answers its questions from the
//! [`ClientWorld`] and the frame's view facts, [`apply`] turns its
//! outputs into C→S messages on the model's `outgoing` list (the bridge
//! sends them) and the interact sender.
//!
//! What the model does not hold yet is answered by a stated reading,
//! each marked:
//! - no hover model (hover): the play
//!   preview hovers monsters near the click ([`super::combat`], D2RUST's own,
//!   unverified); hostility and melee range are its answers too;
//! - no client path record: see [`ModelClick::walk_path`];
//! - [`ModelClick::skill_row`] reads the client `skills` rows' flag
//!   columns and `range`; an id outside the table answers `None`, which
//!   reads as a point click.
//!
//! The `mods` word comes from [`RunMods`] (commands 34–36).
//! In the `play` preview the local player's position the dispatcher reads
//! may be the predicted one ([`ModelClick::local_at`],
//! [`super::predict`]).

use crate::controls::click::{
    self, ClickOut, ClickState, ClickWorld, Kind, Pending, SkillRef, SkillRowFacts,
};
use crate::rules::camera::{Camera, ClientPos, FrameSize, OpenMode};

use super::dispatch::HandlerError;
use super::objects::interact;
use super::output::{Output, Outputs};
use super::world::{ClientWorld, ModelInputs, UnitKey, PLAYER};

/// The frame's view facts the dispatcher reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClickView {
    pub size: FrameSize,
    pub open_mode: u8,
    pub right_panel_bottom: i32,
    /// The skill bar's y limit.
    pub skill_y_limit: i32,
    /// The current mouse position.
    pub mouse: (i32, i32),
    /// UI 9 (game menu) open.
    pub game_menu_open: bool,
    /// The `play` preview picks the hover target ([`super::hover::pick`],
    /// D2RUST's own, unverified); `false`: no hover model (strict).
    pub pick: bool,
    /// The frame's shake `(dx, dy)` (the frame
    /// anchor's): the pick inverts the shaken camera.
    /// `(0, 0)` when no shake runs.
    pub shake: (i32, i32),
    /// The hover target the previous pass's draw left (`Some(None)`: no
    /// unit): the base game reads the hovered unit while it draws, so
    /// a press sees the cursor of the pass before it, and a press posted in
    /// the same pass as its move is a point click. `None`: the pick
    /// reads the click's own
    /// position (strict tests, D2RUST's own).
    pub prev_hover: Option<Option<UnitKey>>,
    /// The local player's mode while the preview prediction moves it
    /// (`Predict::mode`: 2 / 3 / 6). The base game's walk request sets the
    /// player's mode at once, which the held-walk gate reads;
    /// the model's own mode only follows the server's message. `None`:
    /// the model's mode. D2RUST's own, unverified.
    pub local_mode: Option<u32>,
    /// The local player's class walk / run speeds: with them the walk
    /// click computes the client path ([`ModelClick::walk_path`]); `None`
    /// (no class row): the clamped target is the path's end.
    pub speeds: Option<super::predict::Speeds>,
}

/// The client model as the dispatcher reads it.
pub struct ModelClick<'a> {
    pub world: &'a ClientWorld,
    pub inputs: &'a ModelInputs,
    pub view: ClickView,
    /// The local player's predicted precise position (16.16 sub-tiles,
    /// [`super::predict::Predict::position`]); `None`: the model's cell.
    /// D2RUST's own plumbing. In the preview the server sends the walker
    /// nothing, so the model cell stays at the walk's start while the view
    /// follows the prediction; the camera and the walk clamp read the
    /// prediction so a click lands where the player sees it. The
    /// base-game client's own player has its own path record and stands
    /// at the server player's point at every tick and draw (1,721 ticks,
    /// 0 differences).
    pub local_at: Option<(u32, u32)>,
}

/// The modifier keys of the world click (commands 34 CfgRun, 35
/// CfgRunLock, 36 CfgStandStill).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RunMods {
    /// Run held (command 34 down / up).
    pub run_held: bool,
    /// The run lock (command 35 toggles it).
    pub run_lock: bool,
    /// Stand Still held (command 36 down / up).
    pub stand_still: bool,
}

impl RunMods {
    /// Command 35: run lock := not run lock.
    pub fn toggle_run(&mut self) {
        self.run_lock = !self.run_lock;
    }

    /// The `mods` word passed to the click: 8 when Run held
    /// or the run lock is set (no inversion), plus 4 when Stand Still is
    /// held.
    pub fn word(self) -> u32 {
        let mut m = 0;
        if self.run_held || self.run_lock {
            m |= click::mods::RUN;
        }
        if self.stand_still {
            m |= click::mods::STAND_STILL;
        }
        m
    }
}

/// The act test with (P, skill) (kind 0): 0 when P holds a cursor item
/// or is not a player, or in mode 0, 4, 7–12, 17 or 19; mode 13 with
/// class 3, mode 14 with class 6, modes 15–16 with classes 4–6; mode 18
/// (sequence) as [`sequence_can_act`] decides.
/// `used`: P's used skill's `(anim, seqinput)` (skill list +0x10), `None`
/// when there is none; `event_index`: unit +0x38 >> 8.
pub fn can_act(
    is_player: bool,
    cursor_item: bool,
    mode: u32,
    class: u32,
    used: Option<(u8, u8)>,
    event_index: u32,
) -> bool {
    if !is_player || cursor_item {
        return false;
    }
    if mode == 18 {
        return sequence_can_act(used, event_index);
    }
    !matches!(
        (mode, class),
        (0 | 4 | 7..=12 | 17 | 19, _) | (13, 3) | (14, 6) | (15 | 16, 4..=6)
    )
}

/// No used skill → act; its `anim`
/// not 18 → act; `seqinput` 0 or 255 → no; else act iff the frame event
/// index has reached `seqinput`.
pub fn sequence_can_act(used: Option<(u8, u8)>, event_index: u32) -> bool {
    match used {
        None => true,
        Some((anim, _)) if anim != 18 => true,
        Some((_, 0 | 255)) => false,
        Some((_, seq)) => event_index >= u32::from(seq),
    }
}

impl ModelClick<'_> {
    /// The local player's own position:
    /// the one the caller read for the frame ([`Self::local_at`]),
    /// else the model's ([`ClientWorld::local_position`]).
    fn own_position(&self) -> Option<(u32, u32)> {
        self.world.local()?;
        self.local_at.or_else(|| self.world.local_position())
    }

    pub(super) fn camera(&self) -> Option<Camera> {
        let (px, py) = self.own_position()?;
        let at = crate::rules::camera::moving_to_client(px, py);
        let mode = OpenMode::new(self.view.open_mode).unwrap_or(OpenMode::NONE);
        Some(Camera::new(self.view.size, mode, at, self.view.shake))
    }

    fn skill(&self, left: bool) -> Option<SkillRef> {
        let list = self.world.local()?.skills.as_ref()?;
        let i = if left { list.left } else { list.right }?;
        let e = list.entries.get(i)?;
        Some(SkillRef {
            id: e.skill,
            mode: e.mode,
        })
    }
}

/// Screen → world subtile: the inverse of the unit draw
/// and of the static projection.
/// Screen (sx, sy) → client (sx + cx_u − shift_x, sy + cy_u), no row
/// constant (adds only;
/// settled) → subtile ((px + 2·py) >> 5, (2·py − px) >> 5)
/// floored. Measured 2026-10-11 on the Fire Ball scene: clicks 1 and 2
/// give the C→S points the base game sent, (4883,4220) and (4892,4213).
pub fn screen_to_world(cam: &Camera, sx: i32, sy: i32) -> (i32, i32) {
    let p = ClientPos {
        x: sx + cam.unit.x - cam.view.shift_x,
        y: sy + cam.unit.y,
    };
    (
        (p.x + 2 * p.y).div_euclid(32),
        (2 * p.y - p.x).div_euclid(32),
    )
}

impl ClickWorld for ModelClick<'_> {
    fn in_game(&self) -> bool {
        self.world.in_game
    }
    fn player(&self) -> Option<UnitKey> {
        self.world
            .local_player
            .filter(|k| k.unit_type == PLAYER && self.world.units.contains_key(k))
    }
    fn game_menu_open(&self) -> bool {
        self.view.game_menu_open
    }
    fn open_mode(&self) -> u8 {
        self.view.open_mode
    }
    fn frame_size(&self) -> (i32, i32) {
        (self.view.size.width, self.view.size.height)
    }
    fn right_panel_bottom(&self) -> i32 {
        self.view.right_panel_bottom
    }
    fn skill_y_limit(&self) -> i32 {
        self.view.skill_y_limit
    }
    fn mouse(&self) -> (i32, i32) {
        self.view.mouse
    }
    fn hover(&self) -> Option<UnitKey> {
        // TODO: no hover model;
        // The preview's pick (any unit under the cursor, screen space) and,
        // without it, the monster hover; both D2RUST's own, unverified.
        if let Some(h) = self.view.prev_hover {
            return h;
        }
        if self.view.pick {
            if let Some(k) = super::hover::pick(self.world, &self.camera()?, self.view.mouse) {
                return Some(k);
            }
        }
        let (x, y) = self.view.mouse;
        super::combat::hover_at(self.world, self.inputs, self.to_world(x, y))
    }
    fn to_world(&self, x: i32, y: i32) -> (i32, i32) {
        self.camera().map_or((0, 0), |c| screen_to_world(&c, x, y))
    }
    fn position(&self, u: UnitKey) -> Option<(i32, i32)> {
        if self.world.local_player == Some(u) {
            let (x, y) = self.own_position()?;
            return Some(((x >> 16) as i32, (y >> 16) as i32));
        }
        let u = self.world.units.get(&u)?;
        let (x, y) = u.position?;
        Some((i32::from(x), i32::from(y)))
    }
    fn mode(&self, u: UnitKey) -> u32 {
        if let Some(m) = self
            .view
            .local_mode
            .filter(|_| self.world.local().is_some_and(|p| p.key == u))
        {
            return m;
        }
        self.world.units.get(&u).map_or(0, |u| u.mode)
    }
    fn class(&self, u: UnitKey) -> u32 {
        self.world.units.get(&u).map_or(0, |u| u.class)
    }
    fn stamina(&self) -> i32 {
        self.world.local().map_or(0, |p| p.stat(10))
    }
    fn can_act(&self, _skill: Option<SkillRef>) -> bool {
        let Some(p) = self.world.local() else {
            return false;
        };
        let used = p
            .skills
            .as_ref()
            .and_then(|l| l.entries.get(l.current?))
            .map(|e| {
                let r = self.inputs.tables.skills.get(usize::from(e.skill));
                r.map_or((0, 0), |r| (r.anim, r.seqinput))
            });
        can_act(
            p.key.unit_type == PLAYER,
            self.world.use_cursor.is_some(),
            p.mode,
            p.class,
            used,
            p.event_index,
        )
    }
    fn left_skill(&self) -> Option<SkillRef> {
        self.skill(true)
    }
    fn right_skill(&self) -> Option<SkillRef> {
        self.skill(false)
    }
    fn attack_skill(&self) -> Option<SkillRef> {
        let list = self.world.local()?.skills.as_ref()?;
        list.entries
            .iter()
            .find(|e| e.skill == click::ATTACK)
            .map(|e| SkillRef {
                id: e.skill,
                mode: e.mode,
            })
    }
    fn skill_row(&self, id: u16) -> Option<SkillRowFacts> {
        // The client `skills` row when the table has it; otherwise the
        // preview's flag-less row (a click on a picked
        // unit takes the unit path). D2RUST's own, unverified (the preview).
        super::combat::row_facts(self.inputs, id)
            .or_else(|| self.view.pick.then(SkillRowFacts::default))
    }
    fn range(&self, skill: SkillRef) -> u8 {
        super::combat::range_of(self.inputs, self.world.local(), skill.id)
    }
    /// The client use state of P's entry of `skill`
    /// ([`super::use_state::use_state`]): the hand entry of that id, else
    /// the native one; no entry → 3.
    fn use_state(&self, skill: SkillRef) -> u32 {
        let Some(p) = self.world.local() else {
            return super::use_state::code::DISABLED;
        };
        let Some(list) = p.skills.as_ref() else {
            return super::use_state::code::DISABLED;
        };
        let hand = [list.left, list.right]
            .into_iter()
            .flatten()
            .filter_map(|i| list.entries.get(i))
            .find(|e| e.skill == skill.id);
        let e = hand.or_else(|| list.native(skill.id).map(|i| &list.entries[i]));
        e.map_or(super::use_state::code::DISABLED, |e| {
            super::use_state::use_state(self.world, self.inputs, p.key, e)
        })
    }
    fn refusal_sound(&self, _state: u32) -> Option<u16> {
        None
    }
    fn cursor_state(&self) -> u32 {
        0
    }
    fn cursor_unit(&self) -> Option<u32> {
        None
    }
    fn cursor_item(&self) -> Option<u32> {
        self.world.use_cursor.map(|c| c.item.guid)
    }
    fn has_hireling(&self) -> bool {
        false
    }
    fn cursor_unit_mode_record(&self) -> Option<i32> {
        None
    }
    fn in_town(&self, u: UnitKey) -> bool {
        super::modes::in_town(self.world, u)
    }
    fn is_dead(&self, u: UnitKey) -> bool {
        self.world.units.get(&u).is_some_and(|u| u.is_dead())
    }
    fn selectable(&self, u: UnitKey) -> bool {
        self.world.units.get(&u).is_some_and(|u| u.flag_4)
    }
    fn hostile(&self, u: UnitKey) -> bool {
        super::combat::hostile(self.world, self.inputs, u)
    }
    /// The `monstats` `npc` / `interact` bits of U's class
    /// ([`super::world::MonsterClass`]); no row: neither.
    fn monster_npc_interact(&self, u: UnitKey) -> (bool, bool) {
        self.world
            .units
            .get(&u)
            .and_then(|u| self.inputs.tables.monsters.get(u.class as usize))
            .and_then(|c| c.as_ref())
            .map_or((false, false), |c| (c.npc, c.interact))
    }
    fn object_has_row(&self, u: UnitKey) -> bool {
        self.world
            .units
            .get(&u)
            .is_some_and(|u| (u.class as usize) < self.inputs.tables.objects.len())
    }
    /// The interact test with (P, U) (the server's own
    /// function: the item, NPC and player decisions and the
    /// server's walk-or-act test of the same message agree on it), from
    /// the local player's own cell; sizes (a
    /// monster's `monstats2` `SizeX`).
    fn distance(&self, u: UnitKey) -> i32 {
        local_unit_distance(self.world, self.inputs, self.own_position(), u)
    }
    fn path_distance(&self, u: UnitKey) -> i32 {
        let rows = &self.inputs.objclient.rows;
        // From the local player's own cell, as its position.
        match (
            self.world.units.get(&u),
            self.world.local(),
            self.own_position(),
        ) {
            (Some(u), Some(p), Some((x, y))) => super::objects::distance_at(
                u.cell(),
                super::objects::unit_size(u, rows),
                ((x >> 16) as u16, (y >> 16) as u16),
                super::objects::unit_size(p, rows),
            ),
            _ => i32::MAX,
        }
    }
    fn clear(&self, _u: UnitKey) -> bool {
        true
    }
    fn at_object(&self, _u: UnitKey) -> bool {
        false
    }
    fn just_portaled(&self) -> bool {
        self.world.local().is_some_and(|p| p.states.contains(&102))
    }
    /// The melee range test for the local player ([`melee_in_reach`]).
    fn melee_range(&self, u: UnitKey) -> bool {
        melee_in_reach_at(self.world, self.inputs, self.own_position(), u)
    }
    fn moving(&self) -> bool {
        false
    }
    fn current_target(&self) -> Option<UnitKey> {
        None
    }
    fn speed_changed(&self, _running: bool) -> bool {
        false
    }
    fn pending(&self) -> Option<Pending> {
        None
    }
    fn approach_range(&self, _skill: SkillRef) -> i32 {
        0
    }
    /// The walk to a point: the
    /// player's path is computed over the client DRLG (the same code as
    /// the walk prediction, [`super::client_path::ClientPath`]) and the C→S
    /// point is the path's last point; no
    /// path → nothing, also when the player stands in no client room
    /// (returns 0 for a null path room or target room, or a
    /// target ≥ 100 sub-tiles off). Without the DRLG, the class speeds or
    /// the player's position (callers without a model) the raw point is
    /// sent: D2RUST's own, the base game always holds them.
    fn walk_path(&self, x: i32, y: i32) -> Option<(i32, i32)> {
        let (Some(speeds), Some(t), Some(drlg), Some((px, py))) = (
            self.view.speeds,
            super::predict::path_tables(),
            self.world.drlg.as_ref().map(|d| &d.drlg),
            self.own_position(),
        ) else {
            return Some((x, y));
        };
        let (Ok(tx), Ok(ty)) = (u16::try_from(x), u16::try_from(y)) else {
            return Some((x, y));
        };
        let mut path = super::client_path::ClientPath::new(self.world.tick_rate);
        if !path.place(t, drlg, (px >> 16) as u16, (py >> 16) as u16) {
            return None;
        }
        let others = crate::world_view::walk::other_units(self.world, &self.inputs.tables.monsters);
        let objects =
            crate::world_view::walk::other_objects(self.world, &self.inputs.objclient.rows);
        path.stamp_others(t, drlg, &others, &objects);
        let own = super::client_path::Own {
            stamina: self
                .world
                .local()
                .and_then(|p| p.stats.get(&10).copied())
                .unwrap_or(1),
            moves: super::predict::MoveStats::of_local(self.world),
        };
        let to = super::client_path::PathTo::Point(tx, ty);
        if !path.request(t, drlg, speeds, own, to, false) {
            return None;
        }
        path.path_end()
    }
    fn update_counter(&self) -> u32 {
        self.world.drlg_updates
    }
    fn now_ms(&self) -> u32 {
        self.inputs.now
    }
    fn direction(&self) -> u8 {
        0
    }
    fn search_enemy_near(
        &self,
        _row: SkillRowFacts,
        _skill: SkillRef,
        _u: Option<UnitKey>,
        _x: &mut i32,
        _y: &mut i32,
    ) -> Option<UnitKey> {
        None
    }
    fn search_enemy_xy(&self, _row: SkillRowFacts) -> Option<UnitKey> {
        None
    }
    fn enemy_near(&self, x: i32, y: i32, radius: i32) -> Option<UnitKey> {
        let r2 = i64::from(radius) * i64::from(radius);
        self.world
            .units
            .iter()
            .filter(|(k, u)| {
                matches!(k.unit_type, PLAYER | super::world::MONSTER)
                    && self.world.local_player != Some(**k)
                    && !u.is_dead()
                    && u.flag_4
                    && self.hostile(**k)
            })
            .filter_map(|(k, u)| {
                let (ux, uy) = u.position?;
                let (dx, dy) = (i64::from(ux) - i64::from(x), i64::from(uy) - i64::from(y));
                let d = dx * dx + dy * dy;
                (d <= r2).then_some((d, *k))
            })
            .min()
            .map(|(_, k)| k)
    }
    fn search_open_xy(
        &self,
        _u: Option<UnitKey>,
        _x: &mut i32,
        _y: &mut i32,
    ) -> Option<Option<UnitKey>> {
        None
    }
}

/// Applies the dispatcher's outputs in order: codes 1–0x11 become their
/// C→S message on `outgoing`, code 0x13 the interact sender,
/// [`ClickOut::Send`] its bytes. A skill
/// code first runs the local player's mode request (0x15 or 0x16 with
/// the record, [`skill_request`]): the
/// attack or cast mode starts at the click, and the client player update
/// ends it ([`super::player_anim`]). The walk codes' mode requests are
/// not applied (the walk prediction moves the player).
/// PROVISIONAL (local player step 1): the
/// gate (current skill without `interrupt` and mode not 1 /
/// 5 → no request, no send) is not run; the dispatcher's
/// [`can_act`] refuses the attack and cast modes before it; settled by the
/// `skills.txt` `interrupt` column and the packets of a Charge click.
/// Note: the base game runs the gate for every code, the walk codes 1 /
/// 3 too.
/// Sounds, hover calls and the pending record are returned to the
/// caller (the UI layer).
pub fn apply(
    world: &mut ClientWorld,
    inputs: &ModelInputs,
    outs: Vec<ClickOut>,
) -> Result<(Vec<ClickOut>, Vec<Output>), HandlerError> {
    let mut rest = Vec::new();
    let mut outputs = Vec::new();
    for o in outs {
        match o {
            ClickOut::Code {
                code: click::CODE_INTERACT,
                a,
                b,
            } => outputs.extend(interact::send(world, inputs, a as u16, b)?),
            ClickOut::Code { code, a, b } => {
                // The C→S leaves only when the mode request returned
                // non-zero (local player step 2).
                let mut send = true;
                // The walk codes' request:
                // walk to a point 0x01 (run
                // 0x17), to a unit 0x00 (run 0x18); the mode is set at
                // the click, the prediction moves the player.
                if let Some((p, req, record)) = walk_request(world, code, a, b) {
                    let sink = Outputs::default();
                    super::modes::mode_request(world, inputs, p, req, record, &sink)?;
                    outputs.extend(sink.take());
                }
                if let Some((p, req, record)) = skill_request(world, code, a, b) {
                    let sink = Outputs::default();
                    send = super::modes::mode_request(world, inputs, p, req, record, &sink)?;
                    outputs.extend(sink.take());
                    // The base game runs the click before that client update: the
                    // update then turns and steps the new mode at once.
                    if send && world.late_input && world.local_player == Some(p) {
                        super::critter_path::player_turn(world, p);
                        super::player_anim::step(world, inputs, p)?;
                    }
                }
                if let Some(m) = click::code_bytes(code, a, b).filter(|_| send) {
                    world.outgoing.push(m);
                }
            }
            ClickOut::Send(m) => world.outgoing.push(m),
            other => rest.push(other),
        }
    }
    Ok((rest, outputs))
}

/// The local player's mode request of a walk code (click codes 1 walk,
/// 2 walk to a unit, 3 run, 4 run to a unit;
/// codes 0x01, 0x00, 0x17, 0x18). `None`: another code or no local player.
pub fn walk_request(
    world: &ClientWorld,
    code: u8,
    a: u32,
    b: u32,
) -> Option<(UnitKey, u8, [i32; 7])> {
    let req = match code {
        1 => 0x01,
        2 => 0x00,
        3 => 0x17,
        4 => 0x18,
        _ => return None,
    };
    let p = world.local()?;
    Some((p.key, req, [a as i32, b as i32, 0, 0, 0, 0, 0]))
}

/// The local player's mode request of a skill code
/// (local player step 1): point codes
/// 5, 8, 0xC, 0xF → 0x15; unit codes 6, 7, 9, 0xA, 0xD, 0xE, 0x10, 0x11
/// → 0x16; record := the side's skill (left for codes below 0xC),
/// its owner := a / b. `None`: another code, or no local
/// player or skill on that side.
pub fn skill_request(
    world: &ClientWorld,
    code: u8,
    a: u32,
    b: u32,
) -> Option<(UnitKey, u8, [i32; 7])> {
    let req = match code {
        5 | 8 | 0xC | 0xF => 0x15,
        6 | 7 | 9 | 0xA | 0xD | 0xE | 0x10 | 0x11 => 0x16,
        _ => return None,
    };
    let p = world.local()?;
    let list = p.skills.as_ref()?;
    let side = if code < 0xC { list.left } else { list.right };
    let e = list.entries.get(side?)?;
    let record = [
        i32::from(e.skill),
        e.owner as i32,
        a as i32,
        b as i32,
        0,
        0,
        0,
    ];
    Some((p.key, req, record))
}

/// What the pending action did in one client tick
/// ([`pending_step`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PendStep {
    /// No action: the record is gone (no target, code 0, a dead target).
    Stop,
    /// Out of reach: the walk to the unit was requested;
    /// the record is set again.
    Walk,
    /// In reach: the action with (code, P, type, GUID) ran; the record is gone.
    Act,
    /// A target type this step does not run (items, tiles): the caller
    /// keeps its own reading.
    Other,
}

/// The pending action of the local player for the record
/// `p`, once per client tick (player, monster, object): no target or code 0
/// → [`PendStep::Stop`]; a dead player or monster → stop; then the reach
/// test ([`out_of_reach`]): out of reach, the
/// walk to the unit (code 0, or 0x18 with `run`; C→S 0x02 / 0x04 [type]
/// [GUID]) and the record set again; in reach → the action itself
/// ([`apply`] of code `p.code` on (type, GUID); 0x13 is the interact
/// sender). The reach test reads the model's local player position (not
/// the walk prediction, which runs a tick ahead: with it the 0x13 left one
/// tick early). In the base game a click on the Act I waypoint sends
/// 0x01 and, in the same tick, 0x02
/// [2][GUID], 0x02 again every tick (the 200 ms duplicate filter lets
/// ticks 122, 128, 133 through), then 0x13 at the tick the player stands
/// in the waypoint's interact range.
/// PROVISIONAL: the target's flag +0xC4 bit 1 is read as set,
/// the request gate as passing (as [`apply`]), a
/// dead player's corpse test as 0 and a dead monster's
/// `TargetCorpse` side test as failing (the model holds none of them;
/// corpse skills run through `world_view::corpse_click`); the "clear" test
/// with (P, U, 0x804) as clear, as [`ModelClick`] does; settled by
/// the `objects.txt` `Selectable[mode]` columns (bit 0x2 by the object's
/// client mode; bit clear, no walk), the packets of a Charge click
/// (gate) and of a melee click across a door line (clear test); corpses
/// match through `world_view::corpse_click`.
pub fn pending_step(
    world: &mut ClientWorld,
    inputs: &ModelInputs,
    p: Pending,
    run: bool,
    local_at: Option<(u32, u32)>,
) -> Result<(PendStep, Vec<Output>), HandlerError> {
    let Some(u) = world.units.get(&p.target) else {
        return Ok((PendStep::Stop, Vec::new()));
    };
    if p.code == 0 {
        return Ok((PendStep::Stop, Vec::new()));
    }
    match p.target.unit_type {
        PLAYER | super::world::MONSTER if u.is_dead() => return Ok((PendStep::Stop, Vec::new())),
        PLAYER | super::world::MONSTER | super::world::OBJECT => {}
        _ => return Ok((PendStep::Other, Vec::new())),
    }
    let (t, g) = (u32::from(p.target.unit_type), p.target.guid);
    if out_of_reach(world, inputs, p.target, local_at) {
        // The player machine runs code 0x00
        // (0x18) through (the local path aims at the unit),
        // then the C→S leaves; [`apply`] of the walk code.
        let code = if run { 4 } else { 2 };
        let (_, outputs) = apply(world, inputs, vec![ClickOut::Code { code, a: t, b: g }])?;
        return Ok((PendStep::Walk, outputs));
    }
    let (_, outputs) = apply(
        world,
        inputs,
        vec![ClickOut::Code {
            code: p.code,
            a: t,
            b: g,
        }],
    )?;
    Ok((PendStep::Act, outputs))
}

/// The unit distance between `u` and the local player at
/// `own` (16.16 sub-tiles); `i32::MAX` without either.
fn local_unit_distance(
    world: &ClientWorld,
    inputs: &ModelInputs,
    own: Option<(u32, u32)>,
    u: UnitKey,
) -> i32 {
    let size = |c: &super::world::ClientUnit| match c.key.unit_type {
        super::world::MONSTER => inputs
            .tables
            .monsters
            .get(c.class as usize)
            .and_then(|r| r.as_ref())
            .map_or(0, |r| i32::from(r.size_x)),
        _ => super::objects::unit_size(c, &inputs.objclient.rows),
    };
    match (world.units.get(&u), world.local(), own) {
        (Some(u), Some(p), Some((x, y))) => super::objects::unit_distance_at(
            u.cell(),
            size(u),
            ((x >> 16) as u16, (y >> 16) as u16),
            size(p),
        ),
        _ => i32::MAX,
    }
}

/// The melee range test for the local player at `own`:
/// the unit distance against the weapon reach plus 1 (plus 1 more when
/// U moves); a distance below 1 is in range at once.
/// PROVISIONAL (the preview): the line test (mask 0x804) reads the
/// client collision, which the model does not hold; it reads clear (as
/// `ClickWorld::clear`); settled by the packets of a melee click across
/// a door line. The Tentacle test of step 2 is not applied: settled by
/// the `monstats.txt` BaseId rows 258 / 261 (tentacle1, tentaclehead1).
/// Without the item tables or a distance the test reads "in range" (the
/// preview's old answer).
pub fn melee_in_reach_at(
    world: &ClientWorld,
    inputs: &ModelInputs,
    own: Option<(u32, u32)>,
    u: UnitKey,
) -> bool {
    let d = local_unit_distance(world, inputs, own, u);
    let Some(tables) = world.item_tables.0.clone() else {
        return true;
    };
    if d == i32::MAX || d < 1 {
        return true;
    }
    // The weapon worn on body location 4, else 5.
    let reach = [4u8, 5]
        .into_iter()
        .find_map(|loc| {
            super::items::local_items(world)
                .into_iter()
                .find(|i| i.mode == super::items::mode::BODY && i.body == loc)
                .and_then(|i| i.code)
                .and_then(|c| tables.weapon_range_adder(c))
        })
        .unwrap_or(0);
    // The third argument is 1 when U moves (a player in
    // mode 2, 3 or 6, a monster in mode 2 or 0xF; the monster attack
    // modes 8 / 9 with their class test are not applied).
    let moving = world.units.get(&u).is_some_and(|c| match c.key.unit_type {
        super::world::PLAYER => matches!(c.mode, 2 | 3 | 6),
        super::world::MONSTER => matches!(c.mode, 2 | 0xF),
        _ => false,
    });
    d <= reach + 1 + i32::from(moving)
}

/// The local player's position as the server last placed it: the precise
/// position of its copied server path. The walk prediction runs a tick
/// ahead, and a pending action sent from it leaves the record before the
/// server is in reach; the stale model cell is not a position either. The
/// model cell centre is the fallback before the first path copy.
fn server_reach_position(world: &ClientWorld) -> Option<(u32, u32)> {
    let key = world.local_player?;
    if let Some(p) = world.objclient.s_paths.get(&key) {
        if p.precise_x != 0 || p.precise_y != 0 {
            return Some((p.precise_x, p.precise_y));
        }
    }
    let (x, y) = world.local()?.position?;
    Some(((u32::from(x) << 16) | 0x8000, (u32::from(y) << 16) | 0x8000))
}

/// The reach test of the pending action:
/// an object is reached when the local player stands in its
/// interact range and the line is
/// clear; another unit when it is in melee range
/// ([`melee_in_reach`]). The `srvdofunc`
/// 0x13 approach (codes 6 / 0xD) is not taken here: the
/// pending records of the preview carry the interact code.
fn out_of_reach(
    world: &ClientWorld,
    inputs: &ModelInputs,
    target: UnitKey,
    local_at: Option<(u32, u32)>,
) -> bool {
    if target.unit_type != super::world::OBJECT {
        // The server sends a walking player nothing: the copied server path
        // and the model cell stay at the walk's start, so a pending attack
        // judged at either re-sent its walk every tick. The caller's walked
        // position decides, else the last server placement.
        let own = local_at.or_else(|| server_reach_position(world));
        return !melee_in_reach_at(world, inputs, own, target);
    }
    // The server sends a walking player nothing, so the model's cell of the
    // local player stays where the walk began: the caller's position (the
    // player's own walk) decides, else the model's.
    let own = match local_at {
        Some((x, y)) => Some(((x >> 16) as u16, (y >> 16) as u16)),
        None => world.local().and_then(|p| p.position),
    };
    let (Some(o), Some((px, py))) = (world.units.get(&target), own) else {
        return true;
    };
    let Some(row) = inputs.objclient.rows.get(o.class as usize) else {
        return true;
    };
    let (ox, oy) = o.cell();
    let dist = super::objects::unit_distance_at((px, py), 2, (ox, oy), row.size_x as i32);
    !d2_sim::world::objects::interact_range(
        (i32::from(px), i32::from(py)),
        2,
        (i32::from(ox), i32::from(oy)),
        (row.size_x as i32, row.size_y as i32),
        dist,
    )
}

/// One world click against the model ([`apply`]).
pub fn world_click(
    world: &mut ClientWorld,
    inputs: &ModelInputs,
    st: &mut ClickState,
    view: ClickView,
    kind: Kind,
    at: Option<(i32, i32)>,
    mods: u32,
) -> Result<(Vec<ClickOut>, Vec<Output>), HandlerError> {
    world_click_at(world, inputs, st, view, kind, at, mods, None)
}

/// [`world_click`] with the local player read at `local_at`
/// ([`ModelClick::local_at`]; the `play` preview's predicted position).
#[allow(clippy::too_many_arguments)]
pub fn world_click_at(
    world: &mut ClientWorld,
    inputs: &ModelInputs,
    st: &mut ClickState,
    view: ClickView,
    kind: Kind,
    at: Option<(i32, i32)>,
    mods: u32,
    local_at: Option<(u32, u32)>,
) -> Result<(Vec<ClickOut>, Vec<Output>), HandlerError> {
    let mut outs = Vec::new();
    {
        let m = ModelClick {
            world,
            inputs,
            view,
            local_at,
        };
        click::click(st, &m, kind, at, mods, &mut outs);
    }
    apply(world, inputs, outs)
}

/// The held repeat of a loop pass against the model.
pub fn held_repeat(
    world: &mut ClientWorld,
    inputs: &ModelInputs,
    st: &mut ClickState,
    view: ClickView,
    mods: u32,
) -> Result<(Vec<ClickOut>, Vec<Output>), HandlerError> {
    held_repeat_at(world, inputs, st, view, mods, None)
}

/// [`held_repeat`] with the local player read at `local_at`
/// ([`ModelClick::local_at`]).
pub fn held_repeat_at(
    world: &mut ClientWorld,
    inputs: &ModelInputs,
    st: &mut ClickState,
    view: ClickView,
    mods: u32,
    local_at: Option<(u32, u32)>,
) -> Result<(Vec<ClickOut>, Vec<Output>), HandlerError> {
    let mut outs = Vec::new();
    {
        let m = ModelClick {
            world,
            inputs,
            view,
            local_at,
        };
        click::held_repeat(st, &m, mods, &mut outs);
    }
    apply(world, inputs, outs)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A walk north: the player at client (10320, 72816), a click
    /// at (400, 184) walks to subtile (4868, 4222), the preparation's
    /// probe of the blocked (4867, 4222): the click maps to (4867, 4222).
    #[test]
    fn a_click_maps_to_the_rounded_subtile() {
        let cam = Camera::new(
            FrameSize::D2RUST,
            OpenMode::NONE,
            ClientPos { x: 10320, y: 72816 },
            (0, 0),
        );
        assert_eq!(screen_to_world(&cam, 400, 184), (4867, 4222));
        // The player's draw point (400, 292) is 8 rows below its pick
        // point: one step down on screen.
        assert_eq!(screen_to_world(&cam, 400, 292), (4874, 4229));
    }
    use crate::bridge::predict::{walk_of, Walk, WalkTo};
    use crate::bridge::skills::{SkillEntry, SkillList};
    use crate::bridge::update::update_pass;
    use crate::bridge::world::ClientUnit;

    // Synthetic fixture: in game, the local player in town mode 1 at
    // (100, 100) with stamina, its left skill Attack (mode 0).
    fn world() -> ClientWorld {
        let key = UnitKey::new(PLAYER, 1);
        let mut u = ClientUnit::new(key);
        u.position = Some((100, 100));
        u.server_point = (100, 100);
        u.mode = 1;
        u.stats.insert(10, 100 << 8);
        u.skills = Some(SkillList {
            entries: vec![SkillEntry {
                skill: click::ATTACK,
                base: 1,
                owner: crate::bridge::skills::NATIVE,
                ..SkillEntry::default()
            }],
            left: Some(0),
            ..SkillList::default()
        });
        let mut w = ClientWorld::default();
        w.units.insert(key, u);
        w.local_player = Some(key);
        w.in_game = true;
        w
    }

    fn view(mouse: (i32, i32)) -> ClickView {
        ClickView {
            size: FrameSize::D2RUST,
            open_mode: 0,
            right_panel_bottom: FrameSize::D2RUST.play_height(),
            skill_y_limit: FrameSize::D2RUST.skill_y_limit(),
            mouse,
            game_menu_open: false,
            pick: false,
            shake: (0, 0),
            prev_hover: None,
            local_mode: None,
            speeds: None,
        }
    }

    /// One left press at `at` with `mods`; the walk the client sent.
    fn press(mods: u32, at: (i32, i32), local_at: Option<(u32, u32)>) -> Option<Walk> {
        let mut w = world();
        let inputs = ModelInputs::default();
        let mut st = ClickState::default();
        world_click_at(
            &mut w,
            &inputs,
            &mut st,
            view(at),
            Kind::LeftDown,
            Some(at),
            mods,
            local_at,
        )
        .unwrap();
        let walks: Vec<Walk> = w.outgoing.iter().filter_map(|m| walk_of(m)).collect();
        assert!(walks.len() <= 1, "{walks:?}");
        walks.first().copied()
    }

    #[test]
    fn a_walk_click_starts_the_walk_mode_and_puts_its_point_on_the_path() {
        let w = world();
        let key = UnitKey::new(PLAYER, 1);
        assert_eq!(
            walk_request(&w, 1, 5154, 4255),
            Some((key, 0x01, [5154, 4255, 0, 0, 0, 0, 0]))
        );
        assert_eq!(walk_request(&w, 2, 1, 21).map(|r| r.1), Some(0x00));
        assert_eq!(walk_request(&w, 3, 5, 6).map(|r| r.1), Some(0x17));
        assert_eq!(walk_request(&w, 4, 1, 21).map(|r| r.1), Some(0x18));
        assert_eq!(walk_request(&w, 6, 1, 21), None);
        // The click sets the mode and the path target in the same pass.
        let mut w = world();
        let inputs = ModelInputs::default();
        apply(
            &mut w,
            &inputs,
            vec![ClickOut::Code {
                code: 1,
                a: 5154,
                b: 4255,
            }],
        )
        .unwrap();
        let u = &w.units[&key];
        assert_eq!(u.mode, 2);
        assert_eq!(u.path_target, Some((5154, 4255)));
        assert_eq!(w.local_turn, 4);
    }

    #[test]
    fn the_skill_code_bound_is_h_minus_49() {
        // − 0x31, 9 rows above the panel.
        assert_eq!(FrameSize::D2RUST.skill_y_limit(), 600 - 49);
        assert_eq!(FrameSize::LOW.skill_y_limit(), 480 - 49);
        assert_eq!(view((0, 0)).skill_y_limit, 551);
    }

    #[test]
    fn run_mods_word() {
        let mut m = RunMods::default();
        assert_eq!(m.word(), 0);
        m.toggle_run();
        assert_eq!(m.word(), click::mods::RUN);
        // No inversion: Run held with the lock on still runs.
        m.run_held = true;
        assert_eq!(m.word(), click::mods::RUN);
        m.toggle_run();
        assert_eq!(m.word(), click::mods::RUN);
        m.run_held = false;
        m.stand_still = true;
        assert_eq!(m.word(), click::mods::STAND_STILL);
        m.toggle_run();
        assert_eq!(m.word(), click::mods::STAND_STILL | click::mods::RUN);
    }

    #[test]
    fn ground_click_walks_or_runs_with_the_toggle() {
        let at = (500, 200);
        let walk = press(0, at, None).expect("a walk");
        assert!(!walk.run);
        let mut run = RunMods::default();
        run.toggle_run();
        let r = press(run.word(), at, None).expect("a run");
        assert!(r.run);
        assert_eq!(r.to, walk.to);
        // Stand Still: no walk.
        let ss = RunMods {
            stand_still: true,
            ..RunMods::default()
        };
        assert_eq!(press(ss.word(), at, None), None);
    }

    #[test]
    fn the_pick_inverts_the_shaken_camera() {
        let (w, inputs) = (world(), ModelInputs::default());
        let cam = |shake| {
            ModelClick {
                world: &w,
                inputs: &inputs,
                view: ClickView {
                    shake,
                    ..view((0, 0))
                },
                local_at: None,
            }
            .camera()
            .unwrap()
        };
        let shaken = cam((3, -5));
        let (px, py) = w.local_position().unwrap();
        let player = crate::rules::camera::moving_to_client(px, py);
        assert_eq!(
            shaken,
            Camera::new(FrameSize::D2RUST, OpenMode::NONE, player, (3, -5))
        );
        assert_ne!(shaken, cam((0, 0)));
        // A pixel whose world point the shake moves: the click's world
        // point is the shaken camera's.
        let at = (0..800)
            .flat_map(|x| (0..550).map(move |y| (x, y)))
            .find(|&(x, y)| screen_to_world(&shaken, x, y) != screen_to_world(&cam((0, 0)), x, y))
            .expect("the shake moves the world under some pixel");
        assert_ne!(
            screen_to_world(&shaken, at.0, at.1),
            screen_to_world(&cam((0, 0)), at.0, at.1)
        );
    }

    #[test]
    fn screen_to_world_adds_the_camera_with_no_row_constant() {
        // Client (sx + cx_u - shift_x, sy + cy_u).
        let cam = |x, y| {
            Camera::new(
                FrameSize::D2RUST,
                OpenMode::NONE,
                crate::rules::camera::ClientPos { x, y },
                (0, 0),
            )
        };
        let (w, h) = (FrameSize::D2RUST.width, FrameSize::D2RUST.height);
        let shift = cam(0, 0).view.shift_x;
        let want = |x: i32, y: i32| ((x + 2 * y) >> 5, (2 * y - x) >> 5);
        for (px, py) in [(0, 0), (16, 0), (-40, -24), (7, -3)] {
            let c = cam(px, py);
            assert_eq!(
                screen_to_world(&c, w / 2 + shift, h / 2 - 8),
                want(px, py + 8)
            );
        }
    }

    #[test]
    fn sequence_mode_gates_on_the_frame_event_index() {
        let act = |used, ev| can_act(true, false, 18, 0, used, ev);
        assert!(!act(Some((18, 4)), 3));
        assert!(act(Some((18, 4)), 4));
        assert!(!act(Some((18, 0)), 9));
        assert!(!act(Some((18, 255)), 9));
        assert!(act(Some((7, 4)), 0), "a non-SQ used skill acts");
        assert!(act(None, 0), "no used skill acts");
        assert!(!can_act(true, true, 18, 0, None, 0), "cursor item wins");
    }

    #[test]
    fn left_click_on_a_hostile_monster_sends_the_skill_on_the_unit() {
        use crate::bridge::world::{SkillRow, MONSTER};
        let mut w = world();
        let m = UnitKey::new(MONSTER, 9);
        let mut u = ClientUnit::new(m);
        u.position = Some((104, 104));
        u.flag_4 = true;
        u.mode = 1;
        w.units.insert(m, u);
        // Attack: anim A1 (mode 7), range h2h (synthetic rows).
        if let Some(e) = w.units.get_mut(&UnitKey::new(PLAYER, 1)) {
            e.skills.as_mut().unwrap().entries[0].mode = 7;
        }
        let mut inputs = ModelInputs::default();
        inputs.tables.skills = vec![SkillRow {
            anim: 7,
            range: 1,
            ingame: true,
            ..SkillRow::default()
        }];
        // The mouse on the monster's feet.
        let cam = ModelClick {
            world: &w,
            inputs: &inputs,
            view: view((0, 0)),
            local_at: None,
        }
        .camera()
        .unwrap();
        let at = (0..800)
            .flat_map(|x| (0..550).map(move |y| (x, y)))
            .find(|&(x, y)| screen_to_world(&cam, x, y) == (104, 104))
            .expect("the monster is on screen");
        let mut st = ClickState::default();
        world_click(
            &mut w,
            &inputs,
            &mut st,
            view(at),
            Kind::LeftDown,
            Some(at),
            0,
        )
        .unwrap();
        let mut want = vec![0x06];
        want.extend_from_slice(&1u32.to_le_bytes());
        want.extend_from_slice(&9u32.to_le_bytes());
        assert_eq!(w.outgoing, vec![want], "C→S 0x06 [type 1][guid 9]");
    }

    #[test]
    fn right_click_on_the_ground_casts_the_right_skill_at_the_point() {
        use crate::bridge::world::SkillRow;
        let mut w = world();
        // The right skill: id 3, cast mode SC (10), range rng (synthetic rows).
        if let Some(list) = w
            .units
            .get_mut(&UnitKey::new(PLAYER, 1))
            .and_then(|e| e.skills.as_mut())
        {
            list.entries.push(SkillEntry {
                skill: 3,
                mode: 10,
                base: 1,
                owner: crate::bridge::skills::NATIVE,
                ..SkillEntry::default()
            });
            list.right = Some(1);
        }
        let mut inputs = ModelInputs::default();
        inputs.tables.skills = vec![
            SkillRow::default(),
            SkillRow::default(),
            SkillRow::default(),
            SkillRow {
                anim: 10,
                range: 2,
                ingame: true,
                ..SkillRow::default()
            },
        ];
        let at = (500, 200);
        let mut st = ClickState::default();
        world_click(
            &mut w,
            &inputs,
            &mut st,
            view(at),
            Kind::RightDown,
            Some(at),
            0,
        )
        .unwrap();
        assert_eq!(w.outgoing.len(), 1, "{:?}", w.outgoing);
        assert_eq!(w.outgoing[0][0], 0x0C, "C→S 0x0C RightSkill at a point");
        assert_eq!(w.outgoing[0].len(), 5);
    }

    /// A synthetic animation lookup: every player mode 4 frames at
    /// speed 256.
    #[derive(Debug)]
    struct Four;
    impl crate::bridge::player_anim::PlayerAnims for Four {
        fn anim(
            &self,
            _: &ClientWorld,
            _: UnitKey,
            _: u32,
        ) -> Option<crate::bridge::player_anim::PlayerAnim> {
            Some(crate::bridge::player_anim::PlayerAnim {
                frames: 4,
                speed: 256,
                weapon: 0,
                events: None,
            })
        }
    }

    #[test]
    fn distant_non_melee_unit_casts_keep_the_client_position_through_animation() {
        use crate::bridge::dispatch::Dispatch;
        use crate::bridge::receive::ReceiveLog;
        use crate::bridge::world::{SkillRow, MONSTER};
        let player = UnitKey::new(PLAYER, 1);
        let target = UnitKey::new(MONSTER, 9);
        for range in [0, 2, 4] {
            let mut w = world();
            let mut monster = ClientUnit::new(target);
            monster.mode = 1;
            monster.position = Some((120, 100));
            monster.flag_4 = true;
            w.units.insert(target, monster);
            let list = w.units.get_mut(&player).unwrap().skills.as_mut().unwrap();
            list.entries.push(SkillEntry {
                skill: 3,
                mode: 10,
                base: 1,
                owner: crate::bridge::skills::NATIVE,
                ..Default::default()
            });
            list.right = Some(1);
            let mut inputs = ModelInputs::default();
            inputs.tables.skills = vec![SkillRow::default(); 4];
            inputs.tables.skills[3] = SkillRow {
                anim: 10,
                range,
                ingame: true,
                ..Default::default()
            };
            inputs.player_anims = Some(std::sync::Arc::new(Four));
            let mut v = view((400, 300));
            v.prev_hover = Some(Some(target));
            world_click(
                &mut w,
                &inputs,
                &mut ClickState::default(),
                v,
                Kind::RightDown,
                Some((400, 300)),
                0,
            )
            .unwrap();
            assert_eq!(w.outgoing.iter().map(|m| m[0]).collect::<Vec<_>>(), [0x0D]);
            assert_eq!(w.units[&player].mode, 10);
            let dispatch = Dispatch::from_spec().unwrap();
            for _ in 0..6 {
                update_pass(
                    &mut w,
                    &inputs,
                    &dispatch,
                    &mut ReceiveLog::default(),
                    &mut Vec::new(),
                );
                assert_eq!(w.units[&player].position, Some((100, 100)));
            }
            assert_eq!(w.units[&player].mode, 1);
        }
    }

    // The local player rules and the client mode machine.
    #[test]
    fn a_cast_ends_in_neutral_and_the_next_right_click_casts_again() {
        use crate::bridge::dispatch::Dispatch;
        use crate::bridge::receive::ReceiveLog;
        use crate::bridge::world::SkillRow;
        let key = UnitKey::new(PLAYER, 1);
        let mut w = world();
        if let Some(list) = w.units.get_mut(&key).and_then(|e| e.skills.as_mut()) {
            list.entries.push(SkillEntry {
                skill: 3,
                mode: 10,
                base: 1,
                owner: crate::bridge::skills::NATIVE,
                ..SkillEntry::default()
            });
            list.right = Some(1);
        }
        let mut inputs = ModelInputs::default();
        inputs.tables.skills = vec![
            SkillRow::default(),
            SkillRow::default(),
            SkillRow::default(),
            SkillRow {
                anim: 10,
                range: 2,
                ingame: true,
                ..SkillRow::default()
            },
        ];
        inputs.player_anims = Some(std::sync::Arc::new(Four));
        let dispatch = Dispatch::from_spec().unwrap();
        let mut st = ClickState::default();
        let at = (500, 200);
        let click = |w: &mut ClientWorld, st: &mut ClickState| {
            w.outgoing.clear();
            world_click(w, &inputs, st, view(at), Kind::RightDown, Some(at), 0).unwrap();
            world_click(w, &inputs, st, view(at), Kind::RightUp, Some(at), 0).unwrap();
            // One loop pass per click (the per-pass latch).
            st.end_pass();
            w.outgoing.iter().filter(|m| m[0] == 0x0C).count()
        };
        let update = |w: &mut ClientWorld| {
            let (mut log, mut out) = (ReceiveLog::default(), Vec::new());
            update_pass(w, &inputs, &dispatch, &mut log, &mut out);
            assert!(log.rejected.is_empty(), "{:?}", log.rejected);
            w.units[&key].mode
        };
        assert_eq!(click(&mut w, &mut st), 1, "the first cast");
        // The click starts the cast mode SC locally (frame 0 of 4).
        assert_eq!(w.units[&key].mode, 10);
        // While it runs, a right click sends nothing (`can_act`).
        assert_eq!(click(&mut w, &mut st), 0);
        // Updates 1–2 advance to frame 2; after update 3's advance frame
        // 3 + speed 1 reaches 4: neutral on update 3 (mode 1: the player
        // is in no town room).
        assert_eq!(update(&mut w), 10);
        assert_eq!(update(&mut w), 10);
        assert_eq!(update(&mut w), 1);
        assert_eq!(
            click(&mut w, &mut st),
            1,
            "the second cast after the first ended"
        );
        assert_eq!(w.units[&key].mode, 10);
    }

    #[test]
    fn ground_click_reads_the_predicted_position() {
        let at = (500, 200);
        let WalkTo::Point(x, y) = press(0, at, None).unwrap().to else {
            panic!("a point walk");
        };
        // The player predicted 3 sub-tiles further in x: the camera and
        // the clamp follow it, so the target moves by the same 3.
        let local = ((103 << 16) | 0x8000, (100 << 16) | 0x8000);
        let WalkTo::Point(px, py) = press(0, at, Some(local)).unwrap().to else {
            panic!("a point walk");
        };
        assert_eq!((px, py), (x + 3, y));
        // The model's cell read as the prediction: the same as none.
        let same = ((100 << 16) | 0x8000, (100 << 16) | 0x8000);
        assert_eq!(press(0, at, Some(same)).unwrap().to, WalkTo::Point(x, y));
    }
    #[test]
    fn an_npc_click_measures_from_the_predicted_position() {
        use crate::bridge::world::{MonsterClass, MONSTER};
        // Model (100, 100), the walk prediction at (121, 100), a town NPC
        // at (122, 100): next to the player's own position, so the click
        // interacts at once instead of walking there.
        let mut w = world();
        w.set_local_walk(Some(((121 << 16) | 0x8000, (100 << 16) | 0x8000)), None);
        let npc = UnitKey::new(MONSTER, 9);
        let mut u = ClientUnit::new(npc);
        u.position = Some((122, 100));
        u.flag_4 = true;
        u.mode = 1;
        w.units.insert(npc, u);
        let mut inputs = ModelInputs::default();
        inputs.tables.monsters = vec![Some(MonsterClass {
            npc: true,
            interact: true,
            ..MonsterClass::default()
        })];
        let c = ModelClick {
            world: &w,
            inputs: &inputs,
            view: view((0, 0)),
            local_at: None,
        };
        assert!(c.path_distance(npc) <= 2, "{}", c.path_distance(npc));
        let cam = c.camera().unwrap();
        let at = crate::bridge::hover::unit_feet(&cam, MONSTER, (122, 100));
        // The play preview's pick (`bridge::hover`) finds the NPC under
        // the press.
        let mut v = view(at);
        v.pick = true;
        let mut st = ClickState::default();
        world_click(&mut w, &inputs, &mut st, v, Kind::LeftDown, Some(at), 0).unwrap();
        // The NPC hold (C→S 0x59 with the NPC's cell) and the interact
        // (reach 2, the interact sender), no walk to the unit:
        // from the model cell (distance 21) a C→S 0x02 would follow.
        let mut hold = vec![0x59];
        hold.extend_from_slice(&1u32.to_le_bytes());
        hold.extend_from_slice(&9u32.to_le_bytes());
        hold.extend_from_slice(&122u32.to_le_bytes());
        hold.extend_from_slice(&100u32.to_le_bytes());
        assert_eq!(w.outgoing, vec![hold], "no walk to the NPC");
    }

    #[test]
    fn a_walk_click_from_outside_every_client_room_sends_nothing() {
        use crate::bridge::drlg::{ClientDrlg, DrlgSource};
        use d2_sim::drlg::{DrlgData, LevelDef, LevelTypes};
        use std::sync::Arc;
        struct NoTypes;
        impl LevelTypes for NoTypes {}
        struct NoTiles;
        impl d2_sim::drlg::TileSource for NoTiles {
            fn dt1(&self, _: &[u8]) -> Option<&[d2_sim::drlg::TileInfo]> {
                None
            }
        }
        // returns 0 for a null path room: no point is sent.
        let src = DrlgSource {
            data: Arc::new(DrlgData {
                levels: vec![LevelDef::default(); 4],
                ..DrlgData::default()
            }),
            tiles: Arc::new(NoTiles),
            types: Arc::new(|| Box::new(NoTypes)),
        };
        let mut w = world();
        w.drlg = Some(ClientDrlg::build(&src, 0, 1, 0).unwrap());
        let inputs = ModelInputs::default();
        let mut v = view((0, 0));
        v.speeds = Some(super::super::predict::Speeds { walk: 6, run: 9 });
        let c = ModelClick {
            world: &w,
            inputs: &inputs,
            view: v,
            local_at: Some((100 << 16, 100 << 16)),
        };
        assert_eq!(c.walk_path(105, 100), None);
        // Without the DRLG (no model): the raw point.
        w.drlg = None;
        let c = ModelClick {
            world: &w,
            inputs: &inputs,
            view: v,
            local_at: Some((100 << 16, 100 << 16)),
        };
        assert_eq!(c.walk_path(105, 100), Some((105, 100)));
    }

    #[test]
    fn the_pending_reach_reads_the_server_path_not_the_prediction() {
        let mut w = world();
        let key = UnitKey::new(PLAYER, 1);
        // The walk prediction is ahead of the server; the copied server
        // path holds the position a pending action must test its reach
        // from.
        w.local_predicted = Some(((109u32) << 16, (100u32) << 16));
        let at = ((104u32 << 16) | 0x8000, (100u32 << 16) | 0x8000);
        w.objclient.s_paths.insert(
            key,
            d2_sim::path::record::DynamicPath {
                precise_x: at.0,
                precise_y: at.1,
                ..Default::default()
            },
        );
        assert_eq!(server_reach_position(&w), Some(at));
        // Before the first server path copy: the model cell centre.
        w.objclient.s_paths.clear();
        assert_eq!(
            server_reach_position(&w),
            Some(((100u32 << 16) | 0x8000, (100u32 << 16) | 0x8000))
        );
    }

    #[test]
    fn an_object_reach_reads_the_walked_position_not_the_model_cell() {
        use super::super::objects::ObjClientRow;
        // A door (size 3 x 1) at (100, 90); the model still holds the
        // player at its start cell (100, 100) because the server sends a
        // walking player nothing. Out of reach from there, in reach from
        // the position the walk got to (100, 93).
        let mut w = world();
        let door = UnitKey::new(super::super::world::OBJECT, 9);
        let mut u = ClientUnit::new(door);
        u.position = Some((100, 90));
        u.class = 0;
        w.units.insert(door, u);
        let mut inputs = ModelInputs::default();
        inputs.objclient.rows = vec![ObjClientRow {
            size_x: 3,
            size_y: 1,
            is_door: 1,
            ..ObjClientRow::default()
        }];
        assert!(out_of_reach(&w, &inputs, door, None));
        assert!(out_of_reach(
            &w,
            &inputs,
            door,
            Some((100 << 16, 100 << 16))
        ));
        assert!(!out_of_reach(
            &w,
            &inputs,
            door,
            Some((100 << 16, 93 << 16))
        ));
    }

    #[test]
    fn a_unit_reach_reads_the_walked_position_not_the_model_cell() {
        use crate::bridge::item_lists::{ItemProp, ItemTablesRef, StreamProps};
        use crate::bridge::world::MONSTER;
        // Without a weapon table row the melee reach is 1 (the moving
        // target adds 1). The model still holds the player at its start
        // cell (100, 100); the walk got to (103, 100). A monster at
        // (104, 100) is out of reach from the model and in reach from the
        // walked position.
        struct NoWeapon;
        impl StreamProps for NoWeapon {
            fn props(&self, _: &[u8]) -> (Vec<ItemProp>, bool) {
                (Vec::new(), false)
            }
        }
        let mut w = world();
        w.item_tables = ItemTablesRef(Some(std::sync::Arc::new(NoWeapon)));
        let m = UnitKey::new(MONSTER, 8);
        let mut u = ClientUnit::new(m);
        u.position = Some((104, 100));
        u.mode = 2;
        w.units.insert(m, u);
        let inputs = ModelInputs::default();
        assert!(out_of_reach(&w, &inputs, m, None));
        assert!(!out_of_reach(&w, &inputs, m, Some((103 << 16, 100 << 16))));
    }

    #[test]
    fn the_interact_distance_is_the_unit_distance() {
        use crate::bridge::world::ITEM;
        // The player at (100, 100), a ground item at (101, 105): the
        // size-reduced reads 4 (an at-once pick-up), the unit
        // distance of the decision reads more than 4, so the
        // client walks to the item first, as the server's 0x16 test
        // would walk the player.
        let mut w = world();
        let item = UnitKey::new(ITEM, 6);
        let mut u = ClientUnit::new(item);
        u.position = Some((101, 105));
        u.mode = 3;
        w.units.insert(item, u);
        let inputs = ModelInputs::default();
        let c = ModelClick {
            world: &w,
            inputs: &inputs,
            view: view((0, 0)),
            local_at: None,
        };
        assert_eq!(c.path_distance(item), 4);
        let t = super::super::predict::path_tables().unwrap();
        let d = d2_sim::path::walk::geom::unit_distance(
            t,
            d2_sim::path::coords::Point::new(101, 105),
            1,
            d2_sim::path::coords::Point::new(100, 100),
            2,
        );
        assert_eq!(c.distance(item), d);
        assert!(d > 4, "{d}");
    }
}
