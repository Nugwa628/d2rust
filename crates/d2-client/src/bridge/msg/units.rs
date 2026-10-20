//! Unit messages: add (0x59 players, 0xAC monsters, 0x51 objects),
//! remove (0x0A), re-place (0x15), the queued movement and action
//! messages (0x0C–0x10, 0x4C, 0x4D, 0x67–0x72: a position check, then a
//! mode request) and the local player's vitals (0x18, 0x95, 0x96).
//!
//! What the base game also does that is client presentation (gfx, light, UI,
//! automap, music, the mode machines' effects) is Phase 6 and changes no
//! model field.

use std::collections::BTreeMap;

use d2_sim::rng::Seed;

use super::super::bits::BitReader;
use super::super::check::check;
use super::super::dispatch::{HandlerError, Message, UnitMessage};
use super::super::drlg::DrlgRoomId;
use super::super::modes::{mode_request, neutral_walk, player_mode, remove_unit_light};
use super::super::objects::interact::{mode_request_code_2, CODE_INTERACT};
use super::super::objects::FLAG_EX_EXPANSION;
use super::super::output::{Output, ShrineFxKind};
use super::super::player_anim;
use super::super::skills::SkillList;
use super::super::world::{
    ClientUnit, ClientWorld, KindData, MonsterData, MonsterSetup, ObjectData, PlayerData, UnitKey,
    INIT_SEED, MONSTER, OBJECT, PET_HIRELING, PLAYER,
};
use super::Bytes;

/// A unit being created and the room of its point.
struct Created {
    unit: ClientUnit,
    room: Option<DrlgRoomId>,
}

impl Created {
    /// Adds the unit and links it at the head of its creation
    /// room's list when it has one (the
    /// dynamic path set-up of players and monsters, the object init).
    fn add(self, w: &mut ClientWorld) {
        let key = self.unit.key;
        w.add(self.unit);
        if self.room.is_some() {
            w.room_units.insert(key, self.room);
        }
    }
}

/// Common creation fields (model): type, class, GUID, and the
/// seed: {1, 666} at (0, 0); at another point the room of the point
/// (fatal 0x13C when none) has its seed stepped once and the
/// unit seed is `init_low(lo')` (0x59, 0xAC and 0x51 alike,
/// Randomness). Without a client DRLG the room's seed is
/// not in the model, so the seed is `None`.
fn create(
    w: &mut ClientWorld,
    key: UnitKey,
    class: u32,
    x: u16,
    y: u16,
) -> Result<Created, HandlerError> {
    let mut u = ClientUnit::new(key);
    u.class = class;
    if w.expansion != 0 {
        u.flag_ex |= FLAG_EX_EXPANSION;
    }
    let placed = (x, y) != (0, 0);
    u.seed = (!placed).then_some(INIT_SEED);
    let mut room = None;
    if placed && w.drlg.is_some() {
        let r = w.room_at(x, y).ok_or(HandlerError::Fatal(0x13C))?;
        let seed = w
            .drlg
            .as_mut()
            .and_then(|d| d.unit_seed(r.room))
            .expect("a listed room is active");
        u.seed = Some((seed.lo, seed.hi));
        room = Some(r.room);
    }
    u.position = placed.then_some((x, y));
    Ok(Created { unit: u, room })
}

/// 0x59 AssignPlayer.
pub fn assign_player(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    let b = Bytes(msg.bytes);
    if msg.bytes.len() != 26 {
        return Err(HandlerError::Invalid("0x59 is 26 bytes"));
    }
    let key = UnitKey::new(PLAYER, b.u32(1)?);
    let (x, y) = (b.u16(0x16)?, b.u16(0x18)?);
    let mut c = create(w, key, u32::from(b.u8(5)?), x, y)?;
    let u = &mut c.unit;
    // Player init (step 3).
    for s in [68, 67, 69] {
        u.stats.insert(s, 100);
    }
    // A skill list (+0xA8),
    // then its native skills.
    let mut list = SkillList::default();
    let owner = crate::bridge::skills::Owner {
        unit_type: PLAYER,
        class: u.class,
    };
    let class_skills = msg.inputs.tables.class_skills.get(u.class as usize);
    crate::bridge::skills::init_player(&mut list, &msg.inputs.tables.skills, owner, class_skills)
        .map_err(HandlerError::from)?;
    u.skills = Some(list);
    u.mode = 5;
    // Randomness step 2: unless the new record is already the local
    // player (never: the local player pointer is the old record while the
    // new one is initialised), one step of the unit seed.
    u.seed = u.seed.map(|(lo, hi)| {
        let mut s = Seed::new(lo, hi);
        s.step();
        (s.lo, s.hi)
    });
    let mut name = [0u8; 16];
    name.copy_from_slice(b.slice(6, 16)?);
    u.kind = KindData::Player(PlayerData {
        name,
        ..PlayerData::default()
    });
    c.add(w);
    super::super::player_anim::mode_init(w, msg.inputs, key, 5);
    // Player init: the player light.
    super::lighting::player_light(w, key);
    Ok(())
}

/// Component bits of a choice count c: c < 3 → 1 bit; else
/// the bit length of c − 1.
fn component_bits(c: u8) -> u32 {
    if c < 3 {
        1
    } else {
        u8::BITS - (c - 1).leading_zeros()
    }
}

/// State 98 `sourceunit`.
const SOURCE_UNIT_STATE: u8 = 98;

/// 0xAC AssignMonster.
pub fn assign_monster(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    let b = Bytes(msg.bytes);
    let size = usize::from(b.u8(0xC)?);
    if size != msg.bytes.len() || size < 0xD {
        return Err(HandlerError::Invalid("0xAC size byte"));
    }
    let key = UnitKey::new(MONSTER, b.u32(1)?);
    let class = b.u16(5)?;
    let (x, y) = (b.u16(7)?, b.u16(9)?);
    let life = b.u8(0xB)?;
    let mut r = BitReader::new(&msg.bytes[0xD..]);
    let tables = &msg.inputs.tables;
    let class_row = tables.monsters.get(usize::from(class)).copied().flatten();
    let mode = r.read(4);
    let mut data = MonsterData {
        value: -1,
        ..MonsterData::default()
    };
    if r.read(1) == 1 {
        if let Some(row) = class_row {
            for (i, &c) in row.components.iter().enumerate() {
                data.components[i] = r.read(component_bits(c)) as u8;
            }
        }
    }
    if r.read(1) == 1 {
        for bit in [4u8, 8, 2, 0x10, 0x40] {
            if r.read(1) == 1 {
                data.flags |= bit;
            }
        }
        if data.flags & 2 != 0 {
            data.hc_idx = r.read_signed(16) as u16;
        }
        let mut n = 0;
        loop {
            let m = r.read(8) as u8;
            if m == 0 {
                break;
            }
            // The base game writes past its 9-byte buffer here (edge case).
            let slot = data.umods.get_mut(n).ok_or(HandlerError::Invalid(
                "0xAC: more umods than the 9-byte buffer",
            ))?;
            *slot = m;
            n += 1;
        }
        // Only the low byte is kept.
        data.name_seed = u16::from(r.read(16) as u8);
        if r.read(1) == 1 {
            data.value = r.read(32) as i32;
        }
    }
    // The owner relation of `Utrans` 255,
    // read from the pet list as it stands at the create.
    data.owner_relation = pet_shift_relation(w, key.guid);
    // Step 2: the local player's hireling (the local player's pet of
    // type 7, monster in S) is re-initialised, not created (graphics
    // rebuilt, the mode set of the 4-bit mode; nothing else of the
    // message is written); steps 3 and 4 then run on it:
    // the class's palshift block is loaded
    // here, after the class bound checks and
    // before the create or re-initialisation.
    if class_row.is_some() {
        w.palshift_classes.insert(u32::from(class));
    }
    let hireling = w.hireling_guid(w.local_player);
    let reinit = key.guid == hireling && w.units.contains_key(&key);
    let mut created = None;
    if reinit {
        // The mode set: a new mode gets its animation init;
        // the same mode keeps the running animation.
        if w.units[&key].mode != mode {
            super::super::monster_anim::mode_init(w, msg.inputs, key, mode);
        }
    } else {
        // Step 2: the class must be a `monstats` row with a `monstats2`
        // row.
        if class_row.is_none() {
            return Ok(());
        }
        let mut c = create(w, key, u32::from(class), x, y)?;
        let u = &mut c.unit;
        // A monster's seed is init_low(+0x28), {0, 666} without a room.
        if u.position.is_none() {
            u.seed = Some((0, INIT_SEED.1));
        }
        // Step 6: the monster set-up.
        let setup = class_row.and_then(|c| c.setup);
        if let Some(s) = &setup {
            setup_stats(u, s, w.difficulty, w.expansion != 0);
        }
        // Step 6.4: the mode argument is the 4-bit mode.
        u.mode = mode;
        // Step 6.5 (the frame draw on the unit seed with range +0x48)
        // runs once the unit is added ([`super::super::monster_anim`]).
        if let Some(s) = &setup {
            setup_flags(u, s);
        }
        // Step 6.7: `+0xA8` := a new skill list.
        u.skills = Some(SkillList::default());
        // Kept for the umod hooks and step 6.10 after the add.
        u.kind = KindData::Monster(Box::new(data.clone()));
        if let Some(s) = &setup {
            let bonus = tables.monster_skill_bonus[usize::from(w.difficulty.min(2))];
            setup_skills(u, s, &tables.skills, bonus)?;
        }
        created = Some(c);
    }
    // Step 3 on U (the new unit, or the re-initialised hireling).
    let apply3 = |u: &mut ClientUnit| {
        // tests U's own class.
        let own_hireling = matches!(u.class, 271 | 338 | 359 | 560 | 561);
        if !(own_hireling && key.guid == hireling) {
            u.stats.insert(7, 0x8000);
            u.stats.insert(6, i32::from(life) << 8);
        }
        if own_hireling && u.mode == 1 {
            u.flag_ex &= !0x40000;
        }
        u.stats.insert(328, i32::from(x.wrapping_add(y)));
    };
    // Step 4: the source-unit link and the 0x40 stat list.
    let link = (r.read(1) == 1).then(|| r.read(31));
    let mut list = BTreeMap::new();
    if r.read(1) == 1 {
        loop {
            let s = r.read(9);
            let Some(row) = tables.stats.get(s as usize) else {
                break;
            };
            if s >= 0x1FF || row.bits == 0 {
                break;
            }
            let param = if row.param_bits == 0 {
                0
            } else {
                r.read(u32::from(row.param_bits))
            };
            let bits = u32::from(row.bits);
            let value = if bits < 32 && row.signed {
                r.read_signed(bits)
            } else {
                r.read(bits) as i32
            };
            list.insert((s as u16, param as u16), value);
            if r.overflow {
                break;
            }
        }
    }
    let apply4 = |u: &mut ClientUnit| {
        if let Some(v) = link {
            // The owner link with (unit, 0, v):
            // +0x94 := 0 (owner type player), +0x98 := v, state
            // 98 `sourceunit` with stats 353 := 0, 354 := v (the unit has
            // a stat holder), flag-ex |= 0x400.
            u.states.insert(SOURCE_UNIT_STATE);
            let l = u.state_lists.entry(SOURCE_UNIT_STATE).or_default();
            l.remove(&(353, 0));
            l.insert((354, 0), v as i32);
            u.flag_ex |= 0x400;
        }
        if let KindData::Monster(d) = &mut u.kind {
            if let Some(v) = link {
                d.v31 = Some(v);
            }
            if !list.is_empty() {
                // The flag-0x40 list: an existing one is reused.
                d.stat_list
                    .get_or_insert_with(BTreeMap::new)
                    .extend(list.clone());
            }
        }
    };
    match created {
        Some(mut c) => {
            apply3(&mut c.unit);
            apply4(&mut c.unit);
            c.add(w);
            // Rules 6.4–6.5: the mode set's animation part (frame 0, the
            // mode's count, the rate), then the first frame drawn from the
            // unit seed.
            super::super::monster_anim::mode_init(w, msg.inputs, key, mode);
            super::super::monster_anim::first_frame(w, key);
            // Step 6.7's umod hooks, before step 6.9's
            // direction draw; the light they make replaces the init's
            // light below.
            let umod_light = match (&class_row, w.units.get_mut(&key)) {
                (Some(row), Some(u)) => match u.seed {
                    Some((lo, hi)) => {
                        let mut s = Seed::new(lo, hi);
                        let req = umod_hooks(&data, class, row.light_rgb, &mut s);
                        u.seed = Some((s.lo, s.hi));
                        req
                    }
                    None => None,
                },
                _ => None,
            };
            // Step 6.9: the initial path direction (after the frame draw:
            // the other order gives 40 / 29 where the base game draws
            // 57 / 46).
            if let Some(row) = &class_row {
                super::super::monster_anim::first_direction(
                    w,
                    &msg.inputs.tables.monsters,
                    key,
                    row,
                );
            }
            // The path of the add the single-player server
            // copy and the client step work on.
            if let Some((px, py)) = w.units.get(&key).and_then(|u| u.position) {
                super::super::critter_path::server_setup(
                    w,
                    msg.inputs,
                    key,
                    i32::from(px),
                    i32::from(py),
                );
            }
            // The monster init's light, in the monster's room.
            if let Some(row) = &class_row {
                super::lighting::monster_light(w, key, row);
            }
            if let Some(req) = umod_light {
                super::lighting::replace_unit_light(w, key, req);
            }
            // Step 6.8's assigns owe the passive-state parts of a passive
            // skill, applied on the added unit.
            let fx = w
                .units
                .get_mut(&key)
                .and_then(|u| u.skills.as_mut())
                .map(|l| std::mem::take(&mut l.fx))
                .unwrap_or_default();
            super::super::passive::apply(w, msg.inputs, key, fx)?;
            // Step 6.10: the unique reseed after the set-up.
            if let (Some(row), Some(u)) = (&class_row, w.units.get_mut(&key)) {
                if let Some(s) = unique_reseed(&data, class, row.no_unique_shift) {
                    u.seed = Some(s);
                }
            }
            // Step 6.11: a monster whose pet record (first by pet GUID)
            // has type 7 runs the client hireling set-up
            // with the record's {+0x28 low 16 bits, +0x24}
            // (a record no 0x81 filled holds zeros).
            let pet = w.pets.iter().find(|r| r.pet == key.guid);
            if let Some(r) = pet.filter(|r| r.pet_type == PET_HIRELING) {
                let [seed, hire_id, _] = r.extra.unwrap_or_default();
                super::pets::hireling_setup(w, msg.inputs, key, hire_id as u16, seed)?;
            }
        }
        None => {
            let u = w.units.get_mut(&key).expect("checked above");
            apply3(u);
            apply4(u);
        }
    }
    Ok(())
}

/// The fixed hook ids runs before the message's umods.
const FIXED_UMOD_HOOKS: [u8; 4] = [1, 2, 3, 4];

/// Classes whose name hook takes a fixed name: no name
/// draws after the reseed.
const FIXED_NAME_CLASSES: [u16; 13] = [
    0x9C, 0xD3, 0xF2, 0xF3, 0x14D, 0x100, 0x220, 0x2C0, 0x2C1, 0x2C2, 0x2C3, 0x2C4, 0x2C5,
];

/// Step 6.7's umod hooks (run when the first umod is not 0):
/// the hooks of the fixed ids 1–4, then of the 9 umod bytes, each with
/// argument u = type flag 0x8. Only the parts that touch the unit seed
/// and the light are modelled: the name hook (ids 1, 12, 16, 36–39)
/// and the light hook (id 3).
/// Returns the light of the last light hook.
pub(crate) fn umod_hooks(
    data: &MonsterData,
    class: u16,
    monstats2_rgb: (u8, u8, u8),
    seed: &mut Seed,
) -> Option<crate::rules::lighting::sources::LightRequest> {
    if data.umods[0] == 0 {
        return None;
    }
    let u = u32::from(data.flags & 8 != 0);
    let champion = data.flags & 4 != 0;
    let mut light = None;
    for id in FIXED_UMOD_HOOKS.into_iter().chain(data.umods) {
        match id {
            1 | 12 | 16 | 36..=39 if u != 0 => {
                // A superunique or a champion keeps its seed.
                if data.flags & 2 != 0 || champion {
                    continue;
                }
                *seed = Seed::init_low(u32::from(data.name_seed));
                if id == 12 || FIXED_NAME_CLASSES.contains(&class) {
                    continue;
                }
                // Two name parts, then 1 in 2 three more (the name
                // tables are never empty: one step each).
                seed.step();
                seed.step();
                if seed.roll(100) < 50 {
                    seed.step();
                    seed.step();
                    seed.step();
                }
            }
            3 => {
                if let Some(req) =
                    crate::rules::lighting::sources::umod3_light(u, champion, seed, monstats2_rgb)
                {
                    light = Some(req);
                }
            }
            _ => {}
        }
    }
    light
}

/// The shift index test with (U),
/// called by the 0xAC create for the shift index: 1 when no
/// pet record has U's GUID as pet GUID (the first one in list order
/// counts) or that record's owner GUID is −1; else
/// with (owner, local player, 8), which is 0 for the local
/// player's own pet and, for another owner, the bit 8 of the relation
/// node of the owner's roster record. That node is never present in the
/// model (0x8B / 0x8C are out of scope), so it is
/// 0 there too.
pub(crate) fn pet_shift_relation(w: &ClientWorld, guid: u32) -> bool {
    w.pets
        .iter()
        .find(|p| p.pet == guid)
        .is_none_or(|p| p.owner == u32::MAX)
}

/// Step 6.10: a monster with type flag 0x8
/// whose class has no `noUniqueShift` gets the seed {class + name seed,
/// 666}, stepped once.
pub(crate) fn unique_reseed(
    data: &MonsterData,
    class: u16,
    no_unique_shift: bool,
) -> Option<(u32, u32)> {
    if data.flags & 8 == 0 || no_unique_shift {
        return None;
    }
    let mut s = Seed::init_low(u32::from(class) + u32::from(data.name_seed));
    s.step();
    Some((s.lo, s.hi))
}

/// Step 6.1 (first part): the base stats of the set-up, `d`
/// = the difficulty; the classic scaling
/// in a classic game with d > 0 and `Align` ≠ 1
/// gives level += 25·d (maxhp, armor and experience are not in the
/// model; step 3 overwrites 6 and 7).
pub(crate) fn setup_stats(u: &mut ClientUnit, s: &MonsterSetup, difficulty: u8, expansion: bool) {
    let d = usize::from(difficulty.min(2));
    let mut level = i32::from(s.level[d]);
    if !expansion && d > 0 && s.align != 1 {
        level += 25 * d as i32;
    }
    u.stats.insert(12, level);
    u.stats.insert(68, 100);
    u.stats.insert(67, 75);
    u.stats.insert(69, 100);
    for (stat, res) in [36u16, 37, 39, 41, 43, 45].into_iter().zip(s.res) {
        u.stats.insert(stat, i32::from(res[d] as i16));
    }
    u.stats.insert(7, 0x6400);
    u.stats.insert(6, 0x6400);
}

/// Step 6.6: the unit flags from `monstats2`: 0x2 := `isSel`, 0x20 :=
/// not `shadow`, 0x8 set (render), 0x4 := `isAtt`.
pub(crate) fn setup_flags(u: &mut ClientUnit, s: &MonsterSetup) {
    u.flag_2 = Some(s.is_sel);
    u.flag_20 = !s.shadow;
    u.flag_4 = s.is_att;
}

/// Step 6.8: `Skill`i ≥ 0 with level byte > 0 → assign at level byte +
/// the act level bonus, the entry's mode := `Sk`i`mode`.
fn setup_skills(
    u: &mut ClientUnit,
    s: &MonsterSetup,
    rows: &[super::super::world::SkillRow],
    bonus: i32,
) -> Result<(), HandlerError> {
    let owner = super::super::skills::Owner {
        unit_type: MONSTER,
        class: u.class,
    };
    let list = u.skills.get_or_insert_with(SkillList::default);
    for (skill, lvl, mode) in s.skills {
        if skill < 0 || lvl == 0 {
            continue;
        }
        let skill = skill as u16;
        super::super::skills::assign(list, rows, owner, skill, i32::from(lvl) + bonus, false)?;
        // A mode of 0x21 or more (a blank `SkNmode`, -1)
        // leaves the entry's `monanim`.
        if let Some(i) = list.native(skill).filter(|_| mode < 0x21) {
            list.entries[i].mode = u32::from(mode);
        }
    }
    Ok(())
}

/// 0x51 AssignObject.
pub fn assign_object(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    let b = Bytes(msg.bytes);
    if msg.bytes.len() != 14 {
        return Err(HandlerError::Invalid("0x51 is 14 bytes"));
    }
    let ty = b.u8(1)?;
    if ty == MONSTER {
        return Err(HandlerError::Fatal(0x202));
    }
    if ty > 5 {
        return Err(HandlerError::Invalid("0x51: unit type past 5"));
    }
    let key = UnitKey::new(ty, b.u32(2)?);
    let (x, y) = (b.u16(8)?, b.u16(0xA)?);
    let mut c = create(w, key, u32::from(b.u16(6)?), x, y)?;
    let u = &mut c.unit;
    if ty != OBJECT {
        // Step 5: the base game never sends types 0, 3, 4, 5 (the one builder
        // passes type 2); refused like type 1.
        return Err(HandlerError::Invalid(
            "0x51: unit type 0, 3, 4 or 5 (unsupported unit types, msg-units.md §1.3 rule 5)",
        ));
    }
    let class = u.class;
    let interact = b.u8(0xD)?;
    u.mode = u32::from(b.u8(0xC)?);
    // Step 3: a shrine (objects `SubClass` bit 0) gets the
    // shrines record of index interact (out of range →
    // fatal 0x15F / 0x160), then runs the code's on-mode
    // function. Without the class's `objects.txt` row the shrine test
    // cannot run: the shrine part is skipped (as `shrine_facts`).
    let is_shrine = msg
        .inputs
        .tables
        .objects
        .get(class as usize)
        .is_some_and(|r| r.subclass & 1 != 0);
    let shrine_code = if is_shrine {
        let code = *msg
            .inputs
            .tables
            .shrines
            .get(usize::from(interact))
            .ok_or(HandlerError::Fatal(0x15F))?;
        if code >= SHRINES {
            return Err(HandlerError::Fatal(0x37B));
        }
        Some(code)
    } else {
        None
    };
    u.kind = KindData::Object(ObjectData {
        interact,
        shrine: shrine_code,
        // writes flag 0x20 from `BlocksLight[mode]`.
        shadow_mode: Some(u.mode),
        ..ObjectData::default()
    });
    // The object init's animation set-up in the mode byte
    // (it runs in the 0x51 mode), on the
    // unit's client seed. Nothing without rows.
    if let Some(row) = msg.inputs.objclient.rows.get(class as usize) {
        crate::bridge::objects::anim_setup(u, row, u.mode)?;
        // `OrderFlag[mode]` = 1 sets the flat flag 0x100000.
        if row.order_flag.get(u.mode as usize) == Some(&1) {
            u.flag_flat = true;
        }
        // Step 2: the init stamps the footprint when
        // `HasCollision[mode]` is set (a mode past 7 reads as 0, as
        // `d2_sim::path::record::ObjectShape::collides_in`).
        let footprint = row.shape.collides_in(u.mode);
        if let KindData::Object(d) = &mut u.kind {
            d.footprint = footprint;
        }
    }
    let mode = u.mode;
    c.add(w);
    // Step 2's object init gives the object its light
    // (`Lit<mode>` / 2,
    // kind 2). Without the class's row the light cannot be read: nothing.
    if let Some(row) = msg.inputs.tables.objects.get(class as usize).copied() {
        let lit = *row.lit.get(mode as usize).ok_or(HandlerError::Invalid(
            "0x51: object mode past the eight objects.txt modes",
        ))?;
        super::lighting::object_light(w, key, lit, row.rgb);
    }
    if let Some(code) = shrine_code {
        let e = shrine(code);
        if e.on_mode {
            msg.out.push(Output::ShrineFx {
                kind: ShrineFxKind::OnMode,
                code,
                object: key,
                player: None,
                overlays: e.overlays,
                mode,
            });
        }
    }
    Ok(())
}

/// 0x09 AssignLevelWarp: type u8@1, GUID u32@2, class u8@6,
/// x u16@7, y u16@9. Created with the common fields (the room seed
/// step at a point other than (0, 0)) and added; the kind inits are not
/// modelled (the model is a unit with `class` and `position`). The
/// create for a type-1 unit (no base-game sender passes type 1) writes
/// no model field names (as after 0xAC).
pub fn assign_level_warp(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    let b = Bytes(msg.bytes);
    if msg.bytes.len() != 11 {
        return Err(HandlerError::Invalid("0x09 is 11 bytes"));
    }
    let ty = b.u8(1)?;
    if ty > 5 {
        return Err(HandlerError::Invalid("0x09: unit type past 5"));
    }
    let key = UnitKey::new(ty, b.u32(2)?);
    let (x, y) = (b.u16(7)?, b.u16(9)?);
    let c = create(w, key, u32::from(b.u8(6)?), x, y)?;
    c.add(w);
    Ok(())
}

/// 0x0A RemoveUnit: the local player's hireling (step 4) is never
/// removed.
pub fn remove_unit(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    let b = Bytes(msg.bytes);
    if msg.bytes.len() != 6 {
        return Err(HandlerError::Invalid("0x0A is 6 bytes"));
    }
    let key = UnitKey::new(b.u8(1)?, b.u32(2)?);
    if key.unit_type == MONSTER && key.guid == w.hireling_guid(w.local_player) {
        return Ok(());
    }
    w.remove(key);
    Ok(())
}

/// 0x15 ReassignPlayer: place the unit at (x, y).
pub fn reassign_player(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    let b = Bytes(msg.bytes);
    if msg.bytes.len() != 11 {
        return Err(HandlerError::Invalid("0x15 is 11 bytes"));
    }
    let key = UnitKey::new(b.u8(1)?, b.u32(2)?);
    let (x, y) = (b.u16(6)?, b.u16(8)?);
    // Step 2: a unit not in S → nothing.
    if !w.units.contains_key(&key) {
        return Ok(());
    }
    // Step 4.2: room' := room of (x, y) (the cell
    // lookup from the local player's room, then the act lookup); a
    // non-zero point with no room' is fatal 0x168. Without the client DRLG
    // (`active_rooms` none) the point is taken as in a room.
    let new_room = match &w.active_rooms {
        Some(_) if (x, y) == (0, 0) => None,
        Some(_) => Some(w.room_at(x, y).ok_or(HandlerError::Fatal(0x168))?),
        None => None,
    };
    // Step 4.3: a dead unit stays where it is. Step 2: it must then have
    // a room (fatal 0x538): a dead local player whose room went with the
    // act it left has none (the base game dies on the second 0x15
    // after the warp back to town, id 0x538).
    if w.units[&key].is_dead() {
        if w.active_rooms.is_some() && w.room_units.room_of(key).is_none() {
            return Err(HandlerError::Fatal(0x538));
        }
        return Ok(());
    }
    // Step 2: at (0, 0) the unit ends without a room (fatal 0x538).
    if (x, y) == (0, 0) {
        return Err(HandlerError::Fatal(0x538));
    }
    // Step 4.4: the palette follows the `Pal` column (+0x02, not `Act`)
    // of the local player's level. Moving to a room whose level's `Pal`
    // differs from the old room's switches the palette to `Pal`. The
    // first placement after an act load (no old room) keeps the act's
    // palette from 0x03 unless the level's `Pal` differs from it: a warp
    // straight into a level drawn with another act's palette (125-127,
    // 133-136) shows that palette, as entering it from its town does.
    if w.local_player == Some(key) {
        if let Some(new) = new_room {
            let pal = |level: u16| {
                msg.inputs
                    .tables
                    .levels
                    .get(usize::from(level))
                    .map(|l| l.pal)
                    .ok_or(HandlerError::Invalid("room level past the Levels rows"))
            };
            let new_pal = pal(new.level)?;
            let shown = match w.local_room().copied() {
                Some(old) => Some(pal(old.level)?),
                None => w.palette_act,
            };
            if shown != Some(new_pal) {
                w.palette_act = Some(new_pal);
            }
        }
    }
    // Step 4.5. TODO:
    // a failed teleport falls back to the nearest free point,
    // which needs the client's collision map.
    if let Some(u) = w.units.get_mut(&key) {
        u.position = Some((x, y));
        u.placements = u.placements.wrapping_add(1);
    }
    // Step 4.5: the teleport places the unit's path (a
    // server monster's client path) at (x, y).
    super::super::critter_path::server_place(w, msg.inputs, key, i32::from(x), i32::from(y));
    // Step 4.6: the automap's reveal countdown (any unit).
    w.placed_units = w.placed_units.wrapping_add(1);
    // Step 4.6: the local player's placement ends in the placement
    // flag call.
    if w.local_player == Some(key) {
        w.local_places.push(b.u8(10)?);
    }
    // The teleport's room recache: leave
    // the old room's list, head of room''s.
    if w.active_rooms.is_some() {
        w.room_units.place(key, new_room.map(|r| r.room));
    }
    // Step 6, the last call: a player with a room in mode 1 or
    // 5 gets the mode request code 7 with no record, which
    // sets the neutral mode of the room it now stands in (5 in town, else
    // 1): a warp out of town leaves the town neutral mode at once.
    if key.unit_type == PLAYER && w.room_units.room_of(key).is_some() {
        let mode = w.units[&key].mode;
        if mode == player_mode::NEUTRAL || mode == player_mode::TOWN_NEUTRAL {
            remove_unit_light(w, key);
            let (neutral, _) = neutral_walk(w, key);
            w.units.get_mut(&key).expect("checked above").flag_2 = Some(true);
            player_anim::mode_set(w, msg.inputs, key, neutral);
        }
    }
    Ok(())
}

/// One record entry of step 1.
#[derive(Clone, Copy)]
enum F {
    U8(usize),
    U16(usize),
    U32(usize),
    I16(usize),
    K(i32),
    /// Left unset by the base game (stack contents): 0 in the model.
    Unset,
}

/// One row of step 1: check point, code, record.
struct Row {
    check: Option<(usize, usize)>,
    code: F,
    record: [F; 7],
}

use F::{Unset as X, I16, K, U16, U32, U8};

/// The table of step 1.
fn row(id: u8) -> Option<Row> {
    let r = |check, code, record| {
        Some(Row {
            check,
            code,
            record,
        })
    };
    match id {
        0x0C => r(None, U8(6), [U8(7), U8(8), K(0), K(0), K(0), K(0), K(0)]),
        0x0D => r(None, U8(6), [U16(7), U16(9), U8(0xB), X, X, X, X]),
        0x0E => r(None, U8(6), [U8(7), U32(8), X, X, X, X, X]),
        0x0F => r(
            Some((0xC, 0xE)),
            U8(6),
            [U16(7), U16(9), U8(0xB), X, X, X, X],
        ),
        0x10 => r(Some((0xC, 0xE)), U8(6), [U8(7), U32(8), X, X, X, X, X]),
        0x4C => r(
            None,
            K(0x16),
            [U16(6), K(-1), U8(9), U32(0xA), U8(8), K(0), K(0)],
        ),
        0x4D => r(
            None,
            K(0x15),
            [U32(6), K(-1), U16(0xB), U16(0xD), U8(0xA), K(0), K(0)],
        ),
        0x67 => r(
            None,
            U8(5),
            [U16(6), U16(8), U8(0xA), U8(0xC), I16(0xD), U8(0xF), U8(0xB)],
        ),
        0x68 => r(
            Some((6, 8)),
            U8(5),
            [
                U8(0xA),
                U32(0xB),
                U8(0xF),
                U8(0x11),
                I16(0x12),
                U8(0x14),
                U8(0x10),
            ],
        ),
        0x69 => r(
            None,
            U8(5),
            [U16(6), U16(8), U8(0xA), K(2), K(0), K(4), U8(0xB)],
        ),
        0x6A => r(None, U8(5), [U8(6), U32(7), U8(0xB), K(2), K(0), K(4), X]),
        0x6B => r(
            Some((0xC, 0xE)),
            U8(5),
            [U16(6), U16(8), U8(0xA), K(2), K(0), K(4), U8(0xB)],
        ),
        0x6C => r(
            Some((0xC, 0xE)),
            U8(5),
            [U8(6), U32(7), U8(0xB), K(2), K(0), K(4), X],
        ),
        0x6D => r(
            Some((5, 7)),
            K(7),
            [U16(5), U16(7), U8(9), K(2), K(0), K(4), K(0)],
        ),
        _ => None,
    }
}

fn read(b: &Bytes<'_>, f: F) -> Result<i32, HandlerError> {
    Ok(match f {
        U8(o) => i32::from(b.u8(o)?),
        U16(o) => i32::from(b.u16(o)?),
        U32(o) => b.u32(o)? as i32,
        I16(o) => i32::from(b.u16(o)? as i16),
        K(v) => v,
        X => 0,
    })
}

/// The unit handlers of step 1 (with steps 2 and 3): optional position
/// check, then the mode request (model).
pub fn queued(w: &mut ClientWorld, msg: &UnitMessage<'_>) -> Result<(), HandlerError> {
    let row = row(msg.id).ok_or(HandlerError::Invalid("no §4 row for this id"))?;
    let b = Bytes(msg.bytes);
    if let Some((xo, yo)) = row.check {
        let (x, y) = (b.u16(xo)?, b.u16(yo)?);
        check(w, msg.inputs, msg.unit, x, y, 0, 0, 0)?;
    }
    // Step 2 (0x0D, type 0): the party roster's life percent.
    if msg.id == 0x0D && msg.unit.unit_type == PLAYER {
        let life = b.u8(0xC)?;
        if let Some(i) = w.roster_find(msg.unit.guid) {
            w.roster[i].life = u32::from(life);
        }
    }
    if msg.id == 0x6D {
        // Step 3: stat 328 := base(328) + 1.
        if let Some(u) = w.units.get_mut(&msg.unit) {
            let v = u.stat(328).wrapping_add(1);
            u.stats.insert(328, v);
        }
    }
    let code = read(&b, row.code)? as u8;
    let mut record = [0i32; 7];
    for (slot, f) in record.iter_mut().zip(row.record) {
        *slot = read(&b, f)?;
    }
    mode_request(w, msg.inputs, msg.unit, code, record, msg.out)?;
    // Player codes 0x01 / 0x17 (model): the path compute of
    // aims the player at the point (a waypoint arrival turns
    // toward its x + 3, y + 3). A click's walk request is aimed by the
    // server copy instead (`critter_path::player_aim`).
    if msg.unit.unit_type == PLAYER && matches!(code, 0x01 | 0x17) {
        if let (Ok(x), Ok(y)) = (u16::try_from(record[0]), u16::try_from(record[1])) {
            super::super::critter_path::aim_walk(w, msg.unit, x, y);
        }
    }
    // Player code 0x02 (model): the interact sender
    // with (& 0xFFFF).
    if msg.unit.unit_type == PLAYER && code == CODE_INTERACT && w.units.contains_key(&msg.unit) {
        for o in mode_request_code_2(w, msg.inputs, record)? {
            msg.out.push(o);
        }
    }
    // Objects (step 5): the shrine part of codes 3 and
    // 0x15, after the stored request.
    if msg.unit.unit_type == OBJECT {
        match code {
            3 => shrine_on_mode(w, msg)?,
            0x15 => shrine_on_use(w, msg, record[0] as u32)?,
            _ => {}
        }
    }
    Ok(())
}

/// One entry of the shrine table.
#[derive(Clone, Copy)]
struct Shrine {
    on_mode: bool,
    on_use: bool,
    overlays: [i32; 2],
    sound: u32,
}

/// The shrine count.
const SHRINES: u8 = 23;

/// The shrine table, by shrine code.
fn shrine(code: u8) -> Shrine {
    const N: [i32; 2] = [-1, -1];
    let (on_mode, on_use, overlays, sound) = match code {
        0 => (false, false, N, 0),
        1 | 2 => (false, false, N, 0xA71),
        3 => (false, false, N, 0xA70),
        4 | 5 => (false, false, N, 0xA6A),
        6 => (true, false, [0x3B, 0x39], 0xA68),
        7 => (true, false, [0x3C, 0x39], 0xA69),
        8 => (true, false, [0x3E, 0x39], 0xA73),
        9 => (true, false, [0x3F, 0x3A], 0xA72),
        10 => (true, false, [0x3D, 0x3A], 0xA74),
        11 => (true, false, [0x40, 0x3A], 0xA75),
        12 => (true, false, [0x41, 0x39], 0xA77),
        13 => (true, false, [0x42, 0x3A], 0xA70),
        14 => (true, false, [0x43, 0x3A], 0xA70),
        15 => (true, false, [0x44, 0x39], 0xA6B),
        16 => (false, true, N, 0xA76),
        17 | 20 => (false, false, N, 0xA6F),
        18 => (false, false, N, 0xA6D),
        19 => (false, true, N, 0xA78),
        21 => (false, true, N, 0xA6C),
        _ => (false, true, N, 0xA6E),
    };
    Shrine {
        on_mode,
        on_use,
        overlays,
        sound,
    }
}

/// The object's `objects.txt` row and its shrine code (step 1): `(is shrine, shrine data Code, ShrineFunction)`. `None` when
/// the client tables hold no row for the class (as 0xAC without
/// `monstats2` rows, the shrine part is skipped); the app supplies the
/// user's `objects.txt` and `shrines.txt` rows (`ClientTables`).
fn shrine_facts(w: &ClientWorld, msg: &UnitMessage<'_>) -> Option<(bool, Option<u8>, u8)> {
    let u = w.units.get(&msg.unit)?;
    let row = msg.inputs.tables.objects.get(u.class as usize)?;
    let data = match &u.kind {
        KindData::Object(d) => d.shrine,
        _ => None,
    };
    Some((row.subclass & 1 != 0, data, row.shrine_function))
}

/// Code 3 for an object: after the mode change
/// (the stored request), a shrine's on-mode function.
fn shrine_on_mode(w: &mut ClientWorld, msg: &UnitMessage<'_>) -> Result<(), HandlerError> {
    let Some((is_shrine, data, _)) = shrine_facts(w, msg) else {
        return Ok(());
    };
    if !is_shrine {
        return Ok(());
    }
    let code = data.ok_or(HandlerError::Fatal(0x37A))?;
    if code >= SHRINES {
        return Err(HandlerError::Fatal(0x37B));
    }
    let e = shrine(code);
    if e.on_mode {
        msg.out.push(Output::ShrineFx {
            kind: ShrineFxKind::OnMode,
            code,
            object: msg.unit,
            player: None,
            overlays: e.overlays,
            mode: w.units.get(&msg.unit).map_or(0, |u| u.mode),
        });
    }
    Ok(())
}

/// Code 0x15 for an object: is
/// the operator's GUID.
fn shrine_on_use(
    w: &mut ClientWorld,
    msg: &UnitMessage<'_>,
    operator: u32,
) -> Result<(), HandlerError> {
    // Step 1.
    let Some((is_shrine, data, function)) = shrine_facts(w, msg) else {
        return Ok(());
    };
    let (code, data) = if is_shrine {
        let code = data.ok_or(HandlerError::Invalid(
            "model.md §15 rule 4.1: a shrine without shrine data",
        ))?;
        (code, data)
    } else {
        (function, None)
    };
    // Step 2.
    let player = UnitKey::new(PLAYER, operator);
    if !w.units.contains_key(&player) {
        return Ok(());
    }
    // Step 3.
    if code > 0 && code < SHRINES && shrine(code).on_use {
        msg.out.push(Output::ShrineFx {
            kind: ShrineFxKind::OnUse,
            code,
            object: msg.unit,
            player: Some(player),
            overlays: shrine(code).overlays,
            mode: w.units.get(&msg.unit).map_or(0, |u| u.mode),
        });
    }
    // Step 4.
    let code = data.ok_or(HandlerError::Fatal(0x34D))?;
    if code >= SHRINES {
        return Err(HandlerError::Fatal(0x34E));
    }
    let sound = shrine(code).sound;
    if sound != 0 {
        msg.out.push(Output::ShrineSound { sound, player });
    }
    Ok(())
}

/// 0x6E–0x72: a bare `ret`.
pub fn no_effect(_: &mut ClientWorld, _: &UnitMessage<'_>) -> Result<(), HandlerError> {
    Ok(())
}

/// The fields of 0x18 / 0x95 / 0x96.
struct Vitals {
    life: Option<u32>,
    mana: Option<u32>,
    stamina: u32,
    ab: Option<(u32, u32)>,
    x: u16,
    y: u16,
    dx: u32,
    dy: u32,
}

/// The signed step of step 3: dx − 0x100 when dx > 0x80.
fn sdelta(d: u32) -> i32 {
    if d > 0x80 {
        d as i32 - 0x100
    } else {
        d as i32
    }
}

/// 0x18 LifeManaUpdate, 0x95 LifeManaUpdate2, 0x96 WalkVerify.
pub fn vitals(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    let expected = match msg.id {
        0x18 => 15,
        0x95 => 13,
        _ => 9,
    };
    if msg.bytes.len() != expected {
        return Err(HandlerError::Invalid("vitals message size"));
    }
    let mut r = BitReader::new(msg.bytes);
    r.read(8);
    let v = if msg.id == 0x96 {
        let stamina = r.read(15);
        Vitals {
            life: None,
            mana: None,
            stamina,
            ab: None,
            x: r.read(16) as u16,
            y: r.read(16) as u16,
            dx: r.read(8),
            dy: r.read(8),
        }
    } else {
        let (life, mana, stamina) = (r.read(15), r.read(15), r.read(15));
        let ab = (msg.id == 0x18).then(|| (r.read(7), r.read(7)));
        Vitals {
            life: Some(life),
            mana: Some(mana),
            stamina,
            ab,
            x: r.read(16) as u16,
            y: r.read(16) as u16,
            dx: r.read(8),
            dy: r.read(8),
        }
    };
    let Some(key) = w.local_player.filter(|k| w.units.contains_key(k)) else {
        return Ok(());
    };
    let u = w.units.get_mut(&key).expect("checked above");
    // Step 2.
    if let Some(life) = v.life {
        u.stats.insert(6, (life << 8) as i32);
    }
    if let Some(mana) = v.mana {
        u.stats.insert(8, (mana << 8) as i32);
    }
    u.stats.insert(10, (v.stamina << 8) as i32);
    if let Some((a, b)) = v.ab {
        u.stats.insert(74, a as i32);
        u.stats.insert(26, b as i32);
    }
    // Step 3.
    let tx = i32::from(v.x.wrapping_add(sdelta(v.dx) as u16));
    let ty = i32::from(v.y.wrapping_add(sdelta(v.dy) as u16));
    check(w, msg.inputs, key, v.x, v.y, 0, tx, ty)?;
    // Step 4: leaving the dead mode is the player mode machine's
    // (open question 1): no model
    // field changes.
    Ok(())
}
