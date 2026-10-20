//! The byte layouts of the S→C messages a client gets when units come
//! into or leave its rooms and
//! during the single-player session sequence: pure builders, the
//! callers (`wiring::action::switch`, `d2-server`'s session join) pick
//! the values.
//!
//! Little-endian; bytes not listed are 0.

use crate::monsters::init::BitWriter;

/// S→C 0x00 GameLoading.
pub const GAME_LOADING: [u8; 1] = [0x00];
/// S→C 0x02 LoadSuccessful.
pub const LOAD_SUCCESSFUL: [u8; 1] = [0x02];
/// S→C 0x04 LoadComplete.
pub const LOAD_COMPLETE: [u8; 1] = [0x04];

/// S→C 0x01 GameFlags (8 bytes): difficulty
/// u8@1 (game +0x6D), the arena record's flags u32@2, expansion u8@6 (1
/// when game +0x70 ≠ 0), ladder u8@7 (1 when game +0x74 ≠ 0).
pub fn game_flags(difficulty: u8, arena_flags: u32, expansion: bool, ladder: bool) -> [u8; 8] {
    let f = arena_flags.to_le_bytes();
    [
        0x01,
        difficulty,
        f[0],
        f[1],
        f[2],
        f[3],
        u8::from(expansion),
        u8::from(ladder),
    ]
}

/// S→C 0x8D AssignPlayerToParty (7 bytes,
/// `server-messages.tsv`): GUID u32@1, party u16@5.
pub fn assign_player_to_party(guid: u32, party: u16) -> [u8; 7] {
    let g = guid.to_le_bytes();
    let p = party.to_le_bytes();
    [0x8D, g[0], g[1], g[2], g[3], p[0], p[1]]
}

/// No party (the answer for a player in none; measured in
/// 0x5B and 0x75 as 0xFFFF).
pub const NO_PARTY: u16 = 0xFFFF;

/// S→C 0x75 PlayerPartyInfo (13 bytes): GUID u32@1, party
/// u16@5 (0xFFFF in none), level u16@7 (stat 12), u16@9 and
/// u16@11 (0 for a player in no party). Sent
/// right after every 0x59 (measured `75 02000000 ffff 0000
/// 0000 0000` at frame 96).
pub fn player_party_info(guid: u32, level: u16, own: bool) -> [u8; 13] {
    let mut m = [0u8; 13];
    m[0] = 0x75;
    m[1..5].copy_from_slice(&guid.to_le_bytes());
    m[5..7].copy_from_slice(&NO_PARTY.to_le_bytes());
    m[7..9].copy_from_slice(&level.to_le_bytes());
    // u16@11: 1 for the receiver's own player, else its relation
    // entry (0 with none).
    m[11] = u8::from(own);
    m
}

/// The 6-byte (id, unit type u8@1, GUID u32@2) unit reference layout:
/// 0x0B GameHandshake, 0x76 PlayerInProximity.
pub fn unit_ref(id: u8, unit_type: u8, guid: u32) -> [u8; 6] {
    let g = guid.to_le_bytes();
    [id, unit_type, g[0], g[1], g[2], g[3]]
}

/// S→C 0x26 form 5, the overhead text of a unit: u8@1 5, u8@2 the
/// overhead record's byte +8, u8@3 unit type, u32@4 GUID, bytes 8–9 never
/// written (0 here), an empty name at 10, then the text and its NUL.
pub fn overhead_chat(byte8: u8, unit_type: u8, guid: u32, text: &[u8]) -> Vec<u8> {
    let mut m = Vec::with_capacity(12 + text.len());
    m.extend_from_slice(&[0x26, 5, byte8, unit_type]);
    m.extend_from_slice(&guid.to_le_bytes());
    m.extend_from_slice(&[0, 0, 0]);
    m.extend_from_slice(text);
    m.push(0);
    m
}

/// S→C 0x82 PortalOwnership (29 bytes): owner GUID u32@1, the owner's
/// name at 5 (at most
/// 15 characters + NUL; bytes after the NUL through 20 are never
/// written, 0 here), u32@21, u32@25.
pub fn portal_ownership(owner: u32, name: &[u8], portal: u32, portal2: u32) -> [u8; 29] {
    let mut m = [0u8; 29];
    m[0] = 0x82;
    m[1..5].copy_from_slice(&owner.to_le_bytes());
    let n = name
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(name.len())
        .min(15);
    m[5..5 + n].copy_from_slice(&name[..n]);
    m[21..25].copy_from_slice(&portal.to_le_bytes());
    m[25..29].copy_from_slice(&portal2.to_le_bytes());
    m
}

/// S→C 0x98 (7 bytes): GUID u32@1, u16@5.
pub fn unknown98(guid: u32, v: u16) -> [u8; 7] {
    let g = guid.to_le_bytes();
    let w = v.to_le_bytes();
    [0x98, g[0], g[1], g[2], g[3], w[0], w[1]]
}

/// S→C 0x21 UpdateItemOSkill (12 bytes):
/// unit type u8@1, remove u8@2, GUID u32@3,
/// skill u16@7, base level u8@9, bonus level u8@10; byte 11 is never
/// written (0 here).
pub fn update_oskill(
    unit_type: u8,
    remove: bool,
    guid: u32,
    skill: u16,
    base: u8,
    bonus: u8,
) -> [u8; 12] {
    let mut m = [0u8; 12];
    m[0] = 0x21;
    m[1] = unit_type;
    m[2] = u8::from(remove);
    m[3..7].copy_from_slice(&guid.to_le_bytes());
    m[7..9].copy_from_slice(&skill.to_le_bytes());
    m[9] = base;
    m[10] = bonus;
    m
}

/// S→C 0x20 StatUpdate (10 bytes,
/// R4): player GUID u32@1, stat u8@5, value u32@6.
pub fn stat_update(guid: u32, stat: u8, value: u32) -> [u8; 10] {
    let mut m = [0u8; 10];
    m[0] = 0x20;
    m[1..5].copy_from_slice(&guid.to_le_bytes());
    m[5] = stat;
    m[6..10].copy_from_slice(&value.to_le_bytes());
    m
}

/// S→C 0x93 (8 bytes): player
/// GUID u32@1, bonus u8@5 (signed on the client, 0x80 = +128), element
/// u8@6 (0 = any), page u8@7 (4 = any).
// Settled: the base game never sends 0x93
// (the sender has no caller), so no D2RUST code calls this
// builder; it exists for the contract test only.
pub fn skill_bonus(guid: u32, bonus: u8, element: u8, page: u8) -> [u8; 8] {
    let mut m = [0u8; 8];
    m[0] = 0x93;
    m[1..5].copy_from_slice(&guid.to_le_bytes());
    m[5] = bonus;
    m[6] = element;
    m[7] = page;
    m
}

/// S→C 0x73 (32 bytes;
/// the field roles are the client reader's): class u16@5, x u32@7,
/// y u32@0xB, the path's first point x u32@0xF
/// and y u32@0x13 (0 when none), the current frame u16@0x17, owner type
/// u8@0x19 and GUID u32@0x1A, level u8@0x1E, pierce index u8@0x1F;
/// bytes 1–4 are not written.
#[allow(clippy::too_many_arguments)]
pub fn client_missile(
    class: u16,
    pos: (u32, u32),
    first: (u32, u32),
    frame: u16,
    owner: (u8, u32),
    level: u8,
    pierce: u8,
) -> [u8; 32] {
    let mut m = [0u8; 32];
    m[0] = 0x73;
    m[5..7].copy_from_slice(&class.to_le_bytes());
    m[7..11].copy_from_slice(&pos.0.to_le_bytes());
    m[11..15].copy_from_slice(&pos.1.to_le_bytes());
    m[15..19].copy_from_slice(&first.0.to_le_bytes());
    m[19..23].copy_from_slice(&first.1.to_le_bytes());
    m[23..25].copy_from_slice(&frame.to_le_bytes());
    m[25] = owner.0;
    m[26..30].copy_from_slice(&owner.1.to_le_bytes());
    m[30] = level;
    m[31] = pierce;
    m
}

/// S→C 0x11: unit
/// type u8@1, GUID u32@2, overlay id u16@6.
pub fn report_kill(unit_type: u8, guid: u32, overlay: u16) -> [u8; 8] {
    let mut b = [0u8; 8];
    b[0] = 0x11;
    b[1] = unit_type;
    b[2..6].copy_from_slice(&guid.to_le_bytes());
    b[6..8].copy_from_slice(&overlay.to_le_bytes());
    b
}

/// S→C 0x0A RemoveUnit:
/// type u8@1, GUID u32@2.
pub fn remove_unit(unit_type: u8, guid: u32) -> [u8; 6] {
    unit_ref(0x0A, unit_type, guid)
}

/// S→C 0x08 MapHide, the layout of 0x07:
/// the room's tile x u16@1, tile y u16@3, level id u8@5.
pub fn map_hide(x: u16, y: u16, level: u8) -> [u8; 6] {
    let [x0, x1] = x.to_le_bytes();
    let [y0, y1] = y.to_le_bytes();
    [0x08, x0, x1, y0, y1, level]
}

/// S→C 0x51 AssignObject (14 bytes, part A): type 2
/// u8@1, GUID u32@2, class u16@6, x u16@8, y u16@0xA, mode u8@0xC,
/// interact u8@0xD (the only caller passes
/// type 2).
pub fn assign_object(guid: u32, class: u16, x: u16, y: u16, mode: u8, interact: u8) -> [u8; 14] {
    let mut b = [0u8; 14];
    b[0] = 0x51;
    b[1] = 2;
    b[2..6].copy_from_slice(&guid.to_le_bytes());
    b[6..8].copy_from_slice(&class.to_le_bytes());
    b[8..10].copy_from_slice(&x.to_le_bytes());
    b[10..12].copy_from_slice(&y.to_le_bytes());
    b[12] = mode;
    b[13] = interact;
    b
}

/// S→C 0x09 AssignLevelWarp (11 bytes, part A, unit
/// type 5): type u8@1, GUID u32@2, class u8@6, x u16@7, y u16@9.
pub fn assign_warp(unit_type: u8, guid: u32, class: u8, x: u16, y: u16) -> [u8; 11] {
    let mut b = [0u8; 11];
    b[0] = 0x09;
    b[1] = unit_type;
    b[2..6].copy_from_slice(&guid.to_le_bytes());
    b[6] = class;
    b[7..9].copy_from_slice(&x.to_le_bytes());
    b[9..11].copy_from_slice(&y.to_le_bytes());
    b
}

/// S→C 0x5F PortalFlags (5 bytes): u32@1.
pub fn portal_flags(v: u32) -> [u8; 5] {
    let v = v.to_le_bytes();
    [0x5F, v[0], v[1], v[2], v[3]]
}

/// S→C 0x7B AssignHotkey (8 bytes): slot
/// u8@1, u16@2 = skill & 0xFFF, | 0x8000 when the slot's flag is set,
/// item u32@4.
pub fn assign_hotkey(slot: u8, skill: i16, flag: bool, item: u32) -> [u8; 8] {
    let s = (skill as u16 & 0xFFF) | if flag { 0x8000 } else { 0 };
    let [s0, s1] = s.to_le_bytes();
    let i = item.to_le_bytes();
    [0x7B, slot, s0, s1, i[0], i[1], i[2], i[3]]
}

/// S→C 0x23 SetSkill (13 bytes): type
/// u8@1, GUID u32@2, hand u8@6, skill u16@7, item u32@9.
pub fn set_skill(unit_type: u8, guid: u32, hand: u8, skill: u16, item: u32) -> [u8; 13] {
    let mut b = [0u8; 13];
    b[0] = 0x23;
    b[1] = unit_type;
    b[2..6].copy_from_slice(&guid.to_le_bytes());
    b[6] = hand;
    b[7..9].copy_from_slice(&skill.to_le_bytes());
    b[9..13].copy_from_slice(&item.to_le_bytes());
    b
}

/// S→C 0x94 BaseSkillLevels (`sim/server-messages.tsv`
/// `u8@1*3+6;min=9`): count n u8@1,
/// player GUID u32@2, then n entries from @6 of skill u16, level u8.
/// `None` for no entry (below the table's minimum of 9 bytes) or more
/// than 255 (n is a byte). Which entries and levels the sender takes is
/// the caller's (`crate::skills::list::SkillList::base_levels`).
pub fn base_skill_levels(guid: u32, entries: &[(u16, u8)]) -> Option<Vec<u8>> {
    let n = u8::try_from(entries.len()).ok().filter(|&n| n > 0)?;
    let mut m = Vec::with_capacity(6 + 3 * entries.len());
    m.push(0x94);
    m.push(n);
    m.extend_from_slice(&guid.to_le_bytes());
    for &(skill, level) in entries {
        m.extend_from_slice(&skill.to_le_bytes());
        m.push(level);
    }
    Some(m)
}

/// S→C 0x7E (5 bytes): the base game
/// writes only the id byte; bytes 1–4 are uninitialised stack memory
/// there. D2RUST sends zeros (edge case 10; the scenario comparison masks
/// them, `specs/tools/scenario-masks.tsv`).
pub const GAME_ENTRY_DONE: [u8; 5] = [0x7E, 0, 0, 0, 0];

/// One `itemstatcost` row as 0xAA reads it: `send bits`
/// (+0x08), `send param bits` (+0x09), `signed` (+0x04 bit 1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SendStat {
    pub bits: u8,
    pub param_bits: u8,
    pub signed: bool,
}

/// One state of the unit for 0xAA: its id and its stat list's entries
/// {param, id, value} in list order; `None` when the
/// state has no list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SentState {
    pub state: u16,
    pub entries: Option<Vec<(u16, u16, i32)>>,
}

/// Entries of a state's list written at most.
pub const STATE_LIST_ENTRIES: usize = 16;
/// The stream stops before a state when its byte length + 7 would exceed
/// this.
pub const STATE_STREAM_LIMIT: usize = 218;
/// The bit writer's buffer (past it the rest is dropped).
pub const STATE_WRITER_BYTES: usize = 0xF4;

/// `value` clamped for `n` send bits: only when n < 32;
/// signed rows to −2^(n−1)..2^(n−1)−1, others to 0..2^n−1.
pub fn clamp_send(value: i32, n: u8, signed: bool) -> i32 {
    if n >= 32 {
        return value;
    }
    let n = u32::from(n);
    if signed {
        let (lo, hi) = if n == 0 {
            (0, 0)
        } else {
            (-(1i64 << (n - 1)), (1i64 << (n - 1)) - 1)
        };
        i64::from(value).clamp(lo, hi) as i32
    } else {
        i64::from(value).clamp(0, (1i64 << n) - 1) as i32
    }
}

/// A state's list entries then the 0x1FF end.
fn write_entries(
    w: &mut BitWriter,
    entries: &[(u16, u16, i32)],
    stat: &impl Fn(u16) -> Option<SendStat>,
) {
    for &(param, id, value) in entries.iter().take(STATE_LIST_ENTRIES) {
        let Some(row) = stat(id).filter(|r| r.bits != 0) else {
            continue;
        };
        w.write(u32::from(id), 9);
        if row.param_bits != 0 {
            w.write(u32::from(param), u32::from(row.param_bits));
        }
        let v = clamp_send(value, row.bits, row.signed);
        w.write(v as u32, u32::from(row.bits).min(32));
    }
    w.write(0x1FF, 9);
}

/// S→C 0xA7 DelayedState / 0xA9 EndState: `id`, unit type, GUID u32@2,
/// state u8@6 (7 bytes).
pub fn state_ref(id: u8, unit_type: u8, guid: u32, state: u8) -> [u8; 7] {
    let g = guid.to_le_bytes();
    [id, unit_type, g[0], g[1], g[2], g[3], state]
}

/// S→C 0xA8 SetState: type, GUID, size u8@6
/// = 8 + the stream's bytes, state u8@7, the entries stream from @8
/// (the stat-list stream form after the list bit, then 0x1FF).
pub fn set_state(
    unit_type: u8,
    guid: u32,
    state: u8,
    entries: &[(u16, u16, i32)],
    stat: impl Fn(u16) -> Option<SendStat>,
) -> Vec<u8> {
    let mut w = BitWriter::new();
    write_entries(&mut w, entries, &stat);
    let g = guid.to_le_bytes();
    let mut b = vec![
        0xA8,
        unit_type,
        g[0],
        g[1],
        g[2],
        g[3],
        (8 + w.bytes.len()) as u8,
        state,
    ];
    b.extend(w.bytes);
    b
}

/// S→C 0xAA, unit states: byte 0 0xAA, unit
/// type u8@1, GUID u32@2, size u8@6 = 7 + the stream's byte length, the
/// bit stream from byte 7 (LSB first). `states`: the unit's states in
/// ascending order that pass step 1.1 (below the `states` count, no
/// `nosend` bit; the caller's filter); `stat(id)`: the row of `id`
/// (`None`: no row).
pub fn unit_states(
    unit_type: u8,
    guid: u32,
    states: &[SentState],
    stat: impl Fn(u16) -> Option<SendStat>,
) -> Vec<u8> {
    let mut w = BitWriter::new();
    for s in states {
        // Step 1.2.
        if w.bytes.len() + 7 > STATE_STREAM_LIMIT {
            break;
        }
        // Step 1.3.
        w.write(u32::from(s.state), 8);
        match s.entries.as_deref() {
            None | Some([]) => w.write(0, 1),
            Some(entries) => {
                w.write(1, 1);
                write_entries(&mut w, entries, &stat);
            }
        }
    }
    // Step 1.4.
    w.write(0xFF, 8);
    let mut stream = w.bytes;
    stream.truncate(STATE_WRITER_BYTES);
    let g = guid.to_le_bytes();
    let mut b = vec![
        0xAA,
        unit_type,
        g[0],
        g[1],
        g[2],
        g[3],
        (7 + stream.len()) as u8,
    ];
    b.extend(stream);
    b
}

/// The width field of S→C 0x3E: 1 bit
/// a; a = 0 → 8 bits; else 1 bit b, then 16 (b = 0) or 32 bits.
///
/// Settled: the
/// sender picks the narrowest width that holds the value (≤ 0xFF → 8,
/// ≤ 0xFFFF → 16, else 32).
fn write_sized(w: &mut BitWriter, v: u32) {
    if v <= 0xFF {
        w.write(0, 1);
        w.write(v, 8);
    } else if v <= 0xFFFF {
        w.write(1, 1);
        w.write(0, 1);
        w.write(v, 16);
    } else {
        w.write(1, 1);
        w.write(1, 1);
        w.write(v, 32);
    }
}

/// S→C 0x3E UpdateItemStats (`(client, item, 1, stat, value, param)`):
/// size u8@1 (2 + the
/// stream's bytes), then the LSB-first stream from byte 2, zero-padded
/// to 0x22 bytes as the sender queues them: item GUID
/// (sized), set flag 1, stat 9 bits, value (sized, two's complement for a
/// negative value), param (1 bit c: 8 (c = 0) or 16 bits).
///
/// Settled: widths
/// are the narrowest that hold the field ([`write_sized`]); the param is
/// 8 bits when it is ≤ 0xFF.
pub fn update_item_stat(guid: u32, stat: u16, value: i32, param: u16) -> Vec<u8> {
    let mut w = BitWriter::new();
    write_sized(&mut w, guid);
    w.write(1, 1);
    w.write(u32::from(stat) & 0x1FF, 9);
    write_sized(&mut w, value as u32);
    if param <= 0xFF {
        w.write(0, 1);
        w.write(u32::from(param), 8);
    } else {
        w.write(1, 1);
        w.write(u32::from(param), 16);
    }
    let mut b = vec![0x3E, (2 + w.bytes.len()) as u8];
    b.extend(w.bytes);
    // Always queues 0x22 bytes: the zeros after the stream
    // (edge case 13, 0x3E padding).
    b.resize(0x22, 0);
    b
}

/// S→C 0x22 UpdateItemSkill (12 bytes):
/// unit type u8@1, GUID u32@3, skill
/// u16@7, quantity u8@9, flag u8@11 = 1 when the unit has state 7 at
/// send. Bytes 2 and 10 are not written by the sender (0 here).
pub fn update_item_skill(
    unit_type: u8,
    guid: u32,
    skill: u16,
    quantity: u8,
    state7: bool,
) -> [u8; 12] {
    let g = guid.to_le_bytes();
    let s = skill.to_le_bytes();
    [
        0x22,
        unit_type,
        0,
        g[0],
        g[1],
        g[2],
        g[3],
        s[0],
        s[1],
        quantity,
        0,
        u8::from(state7),
    ]
}

/// S→C 0x5A EventMessage of a join (code 2) or leave (code 3), 40 bytes:
/// u8@2 = 4, u32@3 = 0, u8@7 = 0, the
/// character name @8 (16 bytes); single player has no account name, so
/// @0x18–@0x27 stay 0.
pub fn player_event(code: u8, name: &[u8; 16]) -> [u8; 40] {
    let mut m = [0u8; 40];
    m[0] = 0x5A;
    m[1] = code;
    m[2] = 4;
    m[8..24].copy_from_slice(name);
    m
}

/// S→C 0x5A code 6 (40 bytes): u8@2 4, the killer's GUID
/// u32@3 (unit +0x04: the class, not the GUID), its unit type u8@7, the victim's name @8, and @0x18 either the
/// killer player's name or, for a monster with type flag 2, its u16
/// `boss_hc_idx`.
pub fn death_notice(
    killer_class: u32,
    killer_type: u8,
    victim: &[u8; 16],
    killer_name: &[u8; 16],
    boss_hc_idx: Option<u16>,
) -> [u8; 40] {
    let mut m = player_event(6, victim);
    m[3..7].copy_from_slice(&killer_class.to_le_bytes());
    m[7] = killer_type;
    m[0x18..0x28].copy_from_slice(killer_name);
    if let Some(i) = boss_hc_idx {
        m[0x18..0x1A].copy_from_slice(&i.to_le_bytes());
    }
    m
}

/// S→C 0x5B PlayerJoined (36
/// bytes here): size u16@1, GUID u32@3, class u8@7, name @8 (16 bytes),
/// level u16@0x18 (stat 12), party id u16@0x1A (0xFFFF: no party),
/// u16@0x1C = u16@0x1E = 0, u16@0x20 = 0 and the two strings empty (the
/// client fields +0x45E / +0x460 / +0x464 only the legacy save loader
/// writes).
pub fn player_joined(guid: u32, class: u8, name: &[u8; 16], level: u16, party: u16) -> Vec<u8> {
    let mut m = vec![0u8; 36];
    m[0] = 0x5B;
    m[1..3].copy_from_slice(&36u16.to_le_bytes());
    m[3..7].copy_from_slice(&guid.to_le_bytes());
    m[7] = class;
    m[8..24].copy_from_slice(name);
    m[0x18..0x1A].copy_from_slice(&level.to_le_bytes());
    m[0x1A..0x1C].copy_from_slice(&party.to_le_bytes());
    m
}

/// S→C 0x5C PlayerLeft (5 bytes): GUID u32@1
/// of the leaving player.
pub fn player_left(guid: u32) -> [u8; 5] {
    let g = guid.to_le_bytes();
    [0x5C, g[0], g[1], g[2], g[3]]
}

/// S→C 0x65 PlayerKillCount (7 bytes):
/// GUID u32@1, count u16@5.
pub fn player_kill_count(guid: u32, count: u16) -> [u8; 7] {
    let g = guid.to_le_bytes();
    let c = count.to_le_bytes();
    [0x65, g[0], g[1], g[2], g[3], c[0], c[1]]
}

/// S→C 0x57 NpcEnchants (14 bytes,
/// Step 10): GUID u32@1, type u8@5 = 1, the name seed u16@6,
/// u16@8 = umod list bytes 0 | 1 << 8, u16@10 = byte 2, u16@12 = 1 when
/// type flag 4 is set.
pub fn npc_enchants(guid: u32, name_seed: u16, umods: [u8; 3], flag4: bool) -> [u8; 14] {
    let g = guid.to_le_bytes();
    let n = name_seed.to_le_bytes();
    [
        0x57,
        g[0],
        g[1],
        g[2],
        g[3],
        1,
        n[0],
        n[1],
        umods[0],
        umods[1],
        umods[2],
        0,
        u8::from(flag4),
        0,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_bonus_layout() {
        assert_eq!(skill_bonus(1, 0x80, 0, 4), [0x93, 1, 0, 0, 0, 0x80, 0, 4]);
    }

    #[test]
    fn update_item_stat_widths_match_the_sender() {
        // LSB-first bit packer, written out independently of BitWriter.
        let pack = |fields: &[(u32, u32)]| {
            let mut bytes = vec![0u8; 16];
            let mut at = 0usize;
            for &(v, n) in fields {
                for i in 0..n {
                    if (v >> i) & 1 == 1 {
                        bytes[at / 8] |= 1 << (at % 8);
                    }
                    at += 1;
                }
            }
            bytes.truncate(at.div_ceil(8));
            let mut out = vec![0x3E, (2 + bytes.len()) as u8];
            out.extend(bytes);
            out.resize(0x22, 0);
            out
        };
        // 0x100 → 16 bits (1, 0); 0xFF → 8 bits (0); param 0 → 8 bits.
        assert_eq!(
            update_item_stat(0x100, 70, 0xFF, 0),
            pack(&[
                (1, 1),
                (0, 1),
                (0x100, 16),
                (1, 1),
                (70, 9),
                (0, 1),
                (0xFF, 8),
                (0, 1),
                (0, 8)
            ])
        );
        // 0x10000 → 32 bits (1, 1); negative value = 32-bit two's complement.
        assert_eq!(
            update_item_stat(0x1_0000, 9, -1, 0x100),
            pack(&[
                (1, 1),
                (1, 1),
                (0x1_0000, 32),
                (1, 1),
                (9, 9),
                (1, 1),
                (1, 1),
                (u32::MAX, 32),
                (1, 1),
                (0x100, 16)
            ])
        );
        // 0xFFFF stays 16 bits.
        assert_eq!(
            update_item_stat(0xFFFF, 1, 0xFFFF, 0xFF),
            pack(&[
                (1, 1),
                (0, 1),
                (0xFFFF, 16),
                (1, 1),
                (1, 9),
                (1, 1),
                (0, 1),
                (0xFFFF, 16),
                (0, 1),
                (0xFF, 8)
            ])
        );
    }

    #[test]
    fn unit_states_matches_the_recorded_join() {
        // `-022633` seq 103: state 105 with stat 172 (send bits 2) = 2.
        let states = [SentState {
            state: 105,
            entries: Some(vec![(0, 172, 2)]),
        }];
        let row = |id| {
            (id == 172).then_some(SendStat {
                bits: 2,
                param_bits: 0,
                signed: false,
            })
        };
        assert_eq!(
            unit_states(0, 1, &states, row),
            [0xAA, 0x00, 0x01, 0, 0, 0, 0x0C, 0x69, 0x59, 0xF9, 0xFF, 0x1F]
        );
        // No state: the end marker only.
        assert_eq!(unit_states(0, 1, &[], row), [0xAA, 0, 1, 0, 0, 0, 8, 0xFF]);
        // A state without a list: 8 bits, bit 0.
        let none = [SentState {
            state: 3,
            entries: None,
        }];
        assert_eq!(
            unit_states(1, 2, &none, row),
            [0xAA, 1, 2, 0, 0, 0, 10, 0x03, 0xFE, 0x01]
        );
    }

    #[test]
    fn unit_states_skips_rows_without_send_bits_and_clamps() {
        let row = |id: u16| match id {
            1 => Some(SendStat {
                bits: 3,
                param_bits: 0,
                signed: true,
            }),
            2 => Some(SendStat {
                bits: 0,
                param_bits: 0,
                signed: false,
            }),
            _ => None,
        };
        // Stat 1 = 100 → clamped to 3; stat 2 (no send bits) and stat 9
        // (no row) are skipped.
        let s = [SentState {
            state: 0,
            entries: Some(vec![(0, 2, 5), (0, 9, 5), (0, 1, 100)]),
        }];
        let mut w = BitWriter::new();
        w.write(0, 8);
        w.write(1, 1);
        w.write(1, 9);
        w.write(3, 3);
        w.write(0x1FF, 9);
        w.write(0xFF, 8);
        let mut expect = vec![0xAA, 0, 0, 0, 0, 0, (7 + w.bytes.len()) as u8];
        expect.extend(w.bytes);
        assert_eq!(unit_states(0, 0, &s, row), expect);
        assert_eq!(clamp_send(-9, 3, true), -4);
        assert_eq!(clamp_send(-9, 3, false), 0);
        assert_eq!(clamp_send(9, 3, false), 7);
        assert_eq!(clamp_send(-9, 32, false), -9);
    }

    #[test]
    fn unit_states_stops_at_the_stream_limit() {
        // States without lists: 9 bits each; the stream stops once its
        // length + 7 exceeds 218 bytes.
        let states: Vec<SentState> = (0..250)
            .map(|s| SentState {
                state: s,
                entries: None,
            })
            .collect();
        let m = unit_states(0, 0, &states, |_| None);
        let len = usize::from(m[6]) - 7;
        assert_eq!(m.len(), 7 + len);
        // 9 bits a state: after k states the length is ⌈9k / 8⌉; the
        // last state starts at a length ≤ 211, the end marker follows.
        assert!(len <= STATE_STREAM_LIMIT - 7 + 3, "{len}");
        assert_eq!(len, (9 * 188 + 8usize).div_ceil(8));
    }

    #[test]
    fn session_and_room_layouts() {
        // Measured seq 5–7 of both measured games.
        assert_eq!(
            game_flags(0, 0x0010_0004, true, false),
            [0x01, 0x00, 0x04, 0x00, 0x10, 0x00, 0x01, 0x00]
        );
        // Vector: slot 3, skill 36, flag set, item −1.
        assert_eq!(
            assign_hotkey(3, 36, true, u32::MAX),
            [0x7B, 0x03, 0x24, 0x80, 0xFF, 0xFF, 0xFF, 0xFF]
        );
        // Recorded `08 c803 6004 01` (`-015956` frame 149).
        assert_eq!(map_hide(968, 1120, 1), [0x08, 0xC8, 0x03, 0x60, 0x04, 0x01]);
        // Recorded `76 00 01000000` (the join).
        assert_eq!(unit_ref(0x76, 0, 1), [0x76, 0, 1, 0, 0, 0]);
        assert_eq!(remove_unit(1, 0x13), [0x0A, 1, 0x13, 0, 0, 0]);
        assert_eq!(
            assign_object(0x10, 0x77, 0x1234, 0x0567, 1, 2),
            [0x51, 2, 0x10, 0, 0, 0, 0x77, 0, 0x34, 0x12, 0x67, 0x05, 1, 2]
        );
        assert_eq!(
            assign_warp(5, 7, 3, 0x0102, 0x0304),
            [0x09, 5, 7, 0, 0, 0, 3, 0x02, 0x01, 0x04, 0x03]
        );
        assert_eq!(portal_flags(0x8000_0001), [0x5F, 1, 0, 0, 0x80]);
        assert_eq!(
            set_skill(0, 1, 0, 36, u32::MAX),
            [0x23, 0, 1, 0, 0, 0, 0, 36, 0, 0xFF, 0xFF, 0xFF, 0xFF]
        );
        assert_eq!(GAME_ENTRY_DONE, [0x7E, 0, 0, 0, 0]);
    }
}

#[cfg(test)]
mod death_notice_tests {
    use super::death_notice;

    #[test]
    fn the_death_notice_matches_the_recorded_layout() {
        // Measured at frame 73: `5a 06 04 0a000000 01 name`.
        let mut name = [0u8; 16];
        name[..6].copy_from_slice(b"ScnAma");
        let m = death_notice(10, 1, &name, &[0; 16], None);
        assert_eq!(&m[..8], &[0x5a, 6, 4, 10, 0, 0, 0, 1]);
        assert_eq!(&m[8..14], b"ScnAma");
        assert!(m[0x18..].iter().all(|&b| b == 0));
        let m = death_notice(10, 1, &name, &[0; 16], Some(0x0102));
        assert_eq!(&m[0x18..0x1a], &[2, 1]);
    }
}
