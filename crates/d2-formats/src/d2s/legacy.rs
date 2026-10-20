//! Legacy character saves: `.d2s` files of versions 0x47–0x5B. They are
//! read into the same [`D2s`] model as current saves, so the rest of the
//! game never sees the difference. [`super::read`] hands every file below
//! version 0x5C to [`read`] here.
//!
//! A legacy file is a fixed 130-byte header followed by the sections:
//!
//! | offset  | size | field                                              |
//! |---------|------|----------------------------------------------------|
//! | +0x00   | 4    | file magic                                         |
//! | +0x04   | 4    | version                                            |
//! | +0x08   | 16   | character name (NUL-terminated, 15 characters max) |
//! | +0x18   | 4    | status word (+0x19 holds the progression bits)     |
//! | +0x1C   | 2    | class skill count                                  |
//! | +0x1E   | 2    | stat count                                         |
//! | +0x20   | 2    | header size (always 0x82)                          |
//! | +0x22   | 1    | class                                              |
//! | +0x46   | 16   | hotkeys 0–7: skill byte, left-hand flag            |
//! | +0x56   | 2    | right and left mouse skill bytes                   |
//! | +0x58   | 1    | current town: act (low nibble), difficulty (high)  |
//! | +0x7E   | 4    | map seed                                           |
//!
//! After the header come quests, waypoints, NPC flags, a 16-stat mask with
//! its values, the class skill bytes, the player item list, the corpses,
//! the hireling and a trailing block with hotkeys 8–15 and the weapon-swap
//! mouse skills.
//!
//! The legacy loader reports a failure as the load result the player sees,
//! so its errors are [`D2sError::LegacyResult`] rather than internal codes.
//! What it reads: the header, quests, waypoints, NPC flags, stats, skills,
//! an empty player item list, corpses without items, a hireling from
//! version 0x5A, or no hireling, and the trailing block. A save that needs
//! legacy item records, or a hireling below version 0x5A (whose experience
//! would have to be converted through the `hirelings` table), is reported
//! as [`D2sError::LegacyUnsupported`] instead of being guessed at.

use super::*;

/// Header size of a legacy save.
pub const LEGACY_HEADER: usize = 0x82;
/// Lowest legacy version. The header accepts 0x47–0x60; this loader takes
/// those below 0x5C.
pub const LEGACY_MIN: u32 = 0x47;
/// Skill bytes of every class list in the `skills` table.
const CLASS_SKILLS: usize = 30;
/// Stats a legacy file holds (the mask covers the first 16).
const STATS: usize = 16;

fn legacy(result: u8, what: &'static str, offset: usize) -> D2sError {
    D2sError::LegacyResult {
        result,
        what,
        offset,
        after_player: false,
    }
}

/// A failure after the header created and announced the player unit.
fn legacy_after(result: u8, what: &'static str, offset: usize) -> D2sError {
    D2sError::LegacyResult {
        result,
        what,
        offset,
        after_player: true,
    }
}

/// Skill byte → skill id. Legacy files store skill ids in one byte: bytes
/// 0x9C–0xD8 stand for the expansion skills 0xDD–0x119, 0xFF means "no
/// skill" (−1) and every other byte is the id itself.
pub fn skill_from_byte(v: u8) -> i32 {
    match v {
        0x9C..=0xD8 => i32::from(v) + 0x41,
        0xFF => -1,
        _ => i32::from(v),
    }
}

/// The inverse of [`skill_from_byte`] for the writer; `None` when no byte
/// reads back as `s`.
pub fn skill_to_byte(s: i32) -> Option<u8> {
    match s {
        -1 => Some(0xFF),
        0..=0x9B => Some(s as u8),
        0xDD..=0x119 => Some((s - 0x41) as u8),
        _ => None,
    }
}

fn slot(skill_byte: u8, flag: u8) -> Slot {
    let s = skill_from_byte(skill_byte);
    // item index −1 (0 in the model).
    Slot::encode(s, flag != 0, 0).unwrap_or(Slot::NONE)
}

/// Reads a legacy save (version below 0x5C). `opts.game` runs the header
/// checks: name, expansion, hardcore and difficulty.
pub fn read(buf: &[u8], opts: &ReadOptions) -> Result<D2s, D2sError> {
    let version = u32_at(buf, 4);
    // The whole header must be there.
    if buf.len() < LEGACY_HEADER {
        return Err(legacy(9, "file shorter than the header", 0));
    }
    // Header size field and version range.
    if u16_at(buf, 0x20) != LEGACY_HEADER as u16 {
        return Err(legacy(14, "header size", 0x20));
    }
    if !(LEGACY_MIN..=0x60).contains(&version) {
        return Err(legacy(1, "version", 4));
    }
    let status32 = u32_at(buf, 0x18);
    let status = status32 as u16;
    let mut name = [0u8; 16];
    name.copy_from_slice(&buf[0x08..0x18]);
    name[15] = 0;
    // Name and status checks against the game.
    if let Some(g) = &opts.game {
        let n = name[..15].iter().position(|&c| c == 0).unwrap_or(15);
        let want = &g.client_name[..g.client_name.len().min(15)];
        if !name[..n].eq_ignore_ascii_case(want) {
            return Err(legacy(1, "name differs from the client's", 0x08));
        }
        let exp = status & status::EXPANSION != 0;
        if exp && !g.expansion {
            return Err(legacy(24, "expansion character, classic game", 0x18));
        }
        if !exp && g.expansion {
            return Err(legacy(23, "classic character, expansion game", 0x18));
        }
        if status & status::HARDCORE != 0 {
            if status & status::DEAD != 0 {
                return Err(legacy(21, "dead hardcore character", 0x18));
            }
            if !g.hardcore {
                return Err(legacy(20, "hardcore character, softcore game", 0x18));
            }
        } else if g.hardcore {
            return Err(legacy(19, "softcore character, hardcore game", 0x18));
        }
        let p = status::progression(status);
        if p < 8 && g.difficulty >= 2 {
            return Err(legacy(18, "Hell not unlocked", 0x19));
        }
        if (if exp { p < 5 } else { p < 4 }) && g.difficulty >= 1 {
            return Err(legacy(17, "Nightmare not unlocked", 0x19));
        }
    }
    let mut header = Header {
        version,
        name,
        status,
        class: buf[0x22],
        map_seed: u32_at(buf, 0x7E),
        weapon_switch: u32::from(status & 0x2000 != 0),
        ..Header::default()
    };
    // A character that has never entered a game has nothing after the
    // header.
    if status & status::NEW != 0 {
        return Ok(D2s { header, body: None });
    }
    // Current act and difficulty.
    let act = buf[0x58] & 0xF;
    let diff = buf[0x58] >> 4;
    if act >= 5 || diff >= 3 {
        return Err(legacy(9, "act or difficulty", 0x58));
    }
    header.towns[usize::from(diff)] = act | 0x80;
    // Stat count.
    if u16_at(buf, 0x1E) > STATS as u16 {
        return Err(legacy_after(1, "stat count", 0x1E));
    }
    // Hotkeys 0–7 and the mouse skills.
    for i in 0..8 {
        header.hotkeys[i] = slot(buf[0x46 + 2 * i], buf[0x47 + 2 * i]);
    }
    header.mouse[0] = slot(buf[0x56], 0);
    header.mouse[1] = slot(buf[0x57], 0);
    header.skill_count = CLASS_SKILLS as u8;
    header.stat_count = STATS as u8;

    let mut pos = LEGACY_HEADER;

    // Quests: magic, version, size, then one record per difficulty.
    if buf.len() < pos + QUEST_SIZE
        || u32_at(buf, pos) != QUEST_MAGIC
        || u32_at(buf, pos + 4) != QUEST_VERSION
    {
        return Err(legacy_after(2, "quest section", pos));
    }
    let q = &buf[pos..pos + QUEST_SIZE];
    let quests = Quests {
        size: u16_at(q, 8),
        records: [arr(q, 10), arr(q, 106), arr(q, 202)],
    };
    pos += QUEST_SIZE;

    // Waypoints: magic, version, size, then per difficulty a 16-byte record
    // (with its own magic) and 8 spare bytes.
    if buf.len() < pos + WAYPOINT_SIZE || u16_at(buf, pos) != WAYPOINT_MAGIC {
        return Err(legacy_after(3, "waypoint section", pos));
    }
    let w = &buf[pos..pos + WAYPOINT_SIZE];
    let mut waypoints = Waypoints {
        version: u32_at(w, 2),
        size: u16_at(w, 6),
        records: [[0; 16]; 3],
        pads: [[0; 8]; 3],
    };
    for d in 0..3 {
        let rec: [u8; 16] = arr(w, 8 + 24 * d);
        if !matches!(u16_at(&rec, 0), 0x0102 | 0x0101 | 0x0000) {
            return Err(legacy_after(3, "waypoint record magic", pos + 8 + 24 * d));
        }
        waypoints.records[d] = rec;
        waypoints.pads[d] = arr(w, 24 + 24 * d);
    }
    pos += WAYPOINT_SIZE;

    // NPC flags: magic, size, then two sets of per-difficulty flags.
    if buf.len() < pos + NPC_SIZE || u16_at(buf, pos) != NPC_MAGIC {
        return Err(legacy_after(11, "NPC section", pos));
    }
    let n = &buf[pos..pos + NPC_SIZE];
    let npcs = Npcs {
        size: u16_at(n, 2),
        a: [arr(n, 4), arr(n, 12), arr(n, 20)],
        b: [arr(n, 0x1C), arr(n, 0x24), arr(n, 0x2C)],
    };
    pos += NPC_SIZE;

    // Stats: magic "gf", a 3-byte mask, then one u32 for every set bit
    // among the first 16 stats, in stat order.
    if buf.len() < pos + 5 || u16_at(buf, pos) != STATS_MAGIC {
        return Err(legacy_after(4, "stats section", pos));
    }
    let mask = buf[pos + 2..pos + 5].to_vec();
    let set: usize = (0..STATS)
        .filter(|i| mask[i >> 3] & (1 << (i & 7)) != 0)
        .count();
    if buf.len() < pos + 5 + 4 * set {
        return Err(legacy_after(4, "stats values", pos));
    }
    let mut cur = pos + 5;
    let mut values = Vec::new();
    for i in 0..STATS {
        if mask[i >> 3] & (1 << (i & 7)) != 0 {
            values.push((i as u16, u32_at(buf, cur)));
            cur += 4;
        }
    }
    pos = cur;
    let level = values
        .iter()
        .find(|&&(id, _)| id == 12)
        .map_or(1, |&(_, v)| v.clamp(1, 99) as u8);
    header.level = level;
    // Skills.
    if buf.len() < pos + 2 + CLASS_SKILLS || u16_at(buf, pos) != SKILLS_MAGIC {
        return Err(legacy_after(5, "skills section", pos));
    }
    let skills = buf[pos + 2..pos + 2 + CLASS_SKILLS].to_vec();
    pos += 2 + CLASS_SKILLS;
    // The player item list. Items would be the legacy item record, which is
    // not read.
    if buf.len() < pos + 4 || u16_at(buf, pos) != ITEMS_MAGIC {
        return Err(legacy(14, "player item list", pos));
    }
    if u16_at(buf, pos + 2) != 0 {
        return Err(D2sError::LegacyUnsupported("legacy item records"));
    }
    pos += 4;
    // Corpses.
    if buf.len() < pos + 4 || u16_at(buf, pos) != ITEMS_MAGIC {
        return Err(legacy(14, "corpse list", pos));
    }
    let corpses_n = usize::from(u16_at(buf, pos + 2));
    pos += 4;
    let mut corpses = Vec::new();
    for _ in 0..corpses_n {
        if buf.len() < pos + 16 {
            return Err(legacy(14, "corpse", pos));
        }
        let c = &buf[pos..pos + 12];
        if u16_at(buf, pos + 12) != ITEMS_MAGIC || u16_at(buf, pos + 14) != 0 {
            return Err(D2sError::LegacyUnsupported("legacy corpse items"));
        }
        corpses.push(Corpse {
            unk: u32_at(c, 0),
            x: u32_at(c, 4),
            y: u32_at(c, 8),
            items: Vec::new(),
        });
        pos += 16;
    }

    // Hireling (versions 0x48–0x5B): a first byte of 0 means none.
    if version == LEGACY_MIN {
        return Err(D2sError::LegacyUnsupported("version 0x47 hireling form"));
    }
    if buf.len() <= pos {
        return Err(legacy(14, "hireling", pos));
    }
    let mut hireling_items = None;
    if buf[pos] == 0 {
        pos += 1;
    } else {
        // Fewer than 0x12 bytes left is result 14.
        if buf.len() < pos + 0x12 {
            return Err(legacy(14, "hireling block", pos));
        }
        if version < 0x5A {
            return Err(D2sError::LegacyUnsupported(
                "hireling experience conversion below version 0x5A",
            ));
        }
        let b = &buf[pos..pos + 0x12];
        header.hireling = Hireling {
            flags: u32_at(b, 2) & Hireling::DEAD,
            seed: u32_at(b, 6),
            name_index: u16_at(b, 0xA),
            id: u16_at(b, 0xC),
            experience: u32_at(b, 0xE),
            rest: [0; 16],
        };
        pos += 0x12;
        // The hireling's item list (at +0x12): empty only.
        if buf.len() < pos + 4 || u16_at(buf, pos) != ITEMS_MAGIC {
            return Err(legacy(14, "hireling item list", pos));
        }
        if u16_at(buf, pos + 2) != 0 {
            return Err(D2sError::LegacyUnsupported("legacy hireling items"));
        }
        pos += 4;
        hireling_items = Some(Some(Vec::new()));
    }
    // Trailing block.
    if version > 0x48 && buf.len() >= pos + 0x30 {
        for i in 0..8 {
            header.hotkeys[8 + i] = slot(buf[pos + 6 * i], buf[pos + 6 * i + 1]);
        }
        if version > 0x51 && buf.len() >= pos + 0x38 {
            header.mouse[2] =
                Slot::encode(skill_from_byte(u32_at(buf, pos + 0x30) as u8), false, 0)
                    .unwrap_or(Slot::NONE);
            header.mouse[3] =
                Slot::encode(skill_from_byte(u32_at(buf, pos + 0x34) as u8), false, 0)
                    .unwrap_or(Slot::NONE);
        }
    }
    Ok(D2s {
        header,
        body: Some(Body {
            quests,
            waypoints,
            npcs,
            stats: Stats::Mask { mask, values },
            skills,
            items: Vec::new(),
            corpses,
            hireling_items,
            golem: None,
            trailing: Vec::new(),
        }),
    })
}

/// Writes `save` as a legacy file of `version` (0x48–0x5B) so a test or a
/// check can feed the legacy loader: the inverse of [`read`] for what it
/// covers (no items; a hireling only from version 0x5A).
pub fn write(save: &D2s, version: u32) -> Result<Vec<u8>, WriteError> {
    if !(0x48..0x5C).contains(&version) {
        return Err(WriteError::Model("legacy version 0x48..0x5B"));
    }
    let h = &save.header;
    let b = save
        .body
        .as_ref()
        .ok_or(WriteError::Model("legacy new-character stub"))?;
    if !b.items.is_empty() || b.corpses.iter().any(|c| !c.items.is_empty()) {
        return Err(WriteError::Model("legacy items"));
    }
    let byte = |slot: Slot| -> Result<u8, WriteError> {
        skill_to_byte(slot.decode().0).ok_or(WriteError::Model("legacy skill byte"))
    };
    let mut out = vec![0u8; LEGACY_HEADER];
    out[0..4].copy_from_slice(&MAGIC.to_le_bytes());
    out[4..8].copy_from_slice(&version.to_le_bytes());
    out[0x08..0x17].copy_from_slice(&h.name[..15]);
    let mut status = u32::from(h.status & !0x2000);
    if h.weapon_switch & 1 != 0 {
        status |= 0x2000;
    }
    out[0x18..0x1C].copy_from_slice(&status.to_le_bytes());
    out[0x1C..0x1E].copy_from_slice(&(CLASS_SKILLS as u16).to_le_bytes());
    out[0x1E..0x20].copy_from_slice(&(STATS as u16).to_le_bytes());
    out[0x20..0x22].copy_from_slice(&(LEGACY_HEADER as u16).to_le_bytes());
    out[0x22] = h.class;
    for i in 0..8 {
        out[0x46 + 2 * i] = byte(h.hotkeys[i])?;
        out[0x47 + 2 * i] = u8::from(h.hotkeys[i].decode().1);
    }
    out[0x56] = byte(h.mouse[0])?;
    out[0x57] = byte(h.mouse[1])?;
    let (d, t) = h
        .towns
        .iter()
        .enumerate()
        .find(|(_, &t)| t & 0x80 != 0)
        .map_or((0, 0), |(d, &t)| (d as u8, t & 0xF));
    out[0x58] = t | (d << 4);
    out[0x7E..0x82].copy_from_slice(&h.map_seed.to_le_bytes());
    out.extend_from_slice(&QUEST_MAGIC.to_le_bytes());
    out.extend_from_slice(&QUEST_VERSION.to_le_bytes());
    out.extend_from_slice(&b.quests.size.to_le_bytes());
    for rec in &b.quests.records {
        out.extend_from_slice(rec);
    }

    // Waypoints.
    out.extend_from_slice(&WAYPOINT_MAGIC.to_le_bytes());
    out.extend_from_slice(&b.waypoints.version.to_le_bytes());
    out.extend_from_slice(&b.waypoints.size.to_le_bytes());
    for d in 0..3 {
        out.extend_from_slice(&b.waypoints.records[d]);
        out.extend_from_slice(&b.waypoints.pads[d]);
    }

    // NPC flags.
    out.extend_from_slice(&NPC_MAGIC.to_le_bytes());
    out.extend_from_slice(&b.npcs.size.to_le_bytes());
    for f in b.npcs.a.iter().chain(b.npcs.b.iter()) {
        out.extend_from_slice(f);
    }
    let entries = b.stats.entries();
    let mut vals = [None::<i32>; STATS];
    for e in entries
        .iter()
        .filter(|e| e.layer == 0 && usize::from(e.id) < STATS)
    {
        vals[usize::from(e.id)] = Some(e.value);
    }
    let mut mask = [0u8; 3];
    for (i, v) in vals.iter().enumerate() {
        if v.is_some() {
            mask[i >> 3] |= 1 << (i & 7);
        }
    }
    out.extend_from_slice(&STATS_MAGIC.to_le_bytes());
    out.extend_from_slice(&mask);
    for v in vals.iter().flatten() {
        out.extend_from_slice(&(*v as u32).to_le_bytes());
    }

    // Skills, padded or cut to the class list length.
    out.extend_from_slice(&SKILLS_MAGIC.to_le_bytes());
    let mut sk = b.skills.clone();
    sk.resize(CLASS_SKILLS, 0);
    out.extend_from_slice(&sk);
    out.extend_from_slice(&ITEMS_MAGIC.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&ITEMS_MAGIC.to_le_bytes());
    let n = u16::try_from(b.corpses.len()).map_err(|_| WriteError::Model("corpse count"))?;
    out.extend_from_slice(&n.to_le_bytes());
    for c in &b.corpses {
        out.extend_from_slice(&c.unk.to_le_bytes());
        out.extend_from_slice(&c.x.to_le_bytes());
        out.extend_from_slice(&c.y.to_le_bytes());
        out.extend_from_slice(&ITEMS_MAGIC.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
    }
    if h.hireling.is_present() && version >= 0x5A {
        let mut blk = [0u8; 0x12];
        blk[0..2].copy_from_slice(&ITEMS_MAGIC.to_le_bytes());
        blk[2..6].copy_from_slice(&(h.hireling.flags & Hireling::DEAD).to_le_bytes());
        blk[6..10].copy_from_slice(&h.hireling.seed.to_le_bytes());
        blk[0xA..0xC].copy_from_slice(&h.hireling.name_index.to_le_bytes());
        blk[0xC..0xE].copy_from_slice(&h.hireling.id.to_le_bytes());
        blk[0xE..0x12].copy_from_slice(&h.hireling.experience.to_le_bytes());
        out.extend_from_slice(&blk);
        out.extend_from_slice(&ITEMS_MAGIC.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
    } else {
        out.push(0); // no hireling
    }
    if version > 0x48 {
        for i in 8..16 {
            out.push(byte(h.hotkeys[i])?);
            out.push(u8::from(h.hotkeys[i].decode().1));
            out.extend_from_slice(&[0; 4]);
        }
        if version > 0x51 {
            for m in [h.mouse[2], h.mouse[3]] {
                let v = byte(m)?;
                out.extend_from_slice(&u32::from(v).to_le_bytes());
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_bytes() {
        // An expansion skill byte, "no skill" and a plain id, and back.
        assert_eq!(skill_from_byte(0x9C), 221);
        assert_eq!(skill_from_byte(0xFF), -1);
        assert_eq!(skill_from_byte(0x24), 36);
        assert_eq!(skill_to_byte(221), Some(0x9C));
    }

    /// An expansion character in act 2 of Normal with two stats.
    fn model() -> D2s {
        let mut s = D2s::new_stub(b"Kara", 0, status::EXPANSION, 0).unwrap();
        s.header.status &= !status::NEW;
        s.header.towns[0] = 0x80 | 1;
        s.body = Some(Body {
            stats: Stats::Mask {
                mask: vec![3, 0],
                values: vec![(0, 10), (1, 20)],
            },
            skills: vec![0; 30],
            ..Body::default()
        });
        s
    }

    #[test]
    fn framing_errors() {
        // A written file reads back; a wrong header size, a version below
        // the legacy range, a short file and an act out of range fail.
        let bytes = write(&model(), 0x59).unwrap();
        let opts = ReadOptions::default();
        let d = read(&bytes, &opts).unwrap();
        assert_eq!(d.header.towns[0], 0x81);
        assert_eq!(d.header.name_bytes(), b"Kara");
        let mut b = bytes.clone();
        b[0x20] = 0x80;
        assert!(matches!(
            read(&b, &opts),
            Err(D2sError::LegacyResult { result: 14, .. })
        ));
        let mut b = bytes.clone();
        b[4] = 0x46;
        assert!(matches!(
            read(&b, &opts),
            Err(D2sError::LegacyResult { result: 1, .. })
        ));
        assert!(matches!(
            read(&bytes[..0x81], &opts),
            Err(D2sError::LegacyResult { result: 9, .. })
        ));
        let mut b = bytes.clone();
        b[0x58] = 0x15;
        assert!(matches!(
            read(&b, &opts),
            Err(D2sError::LegacyResult { result: 9, .. })
        ));
    }

    #[test]
    fn stats_round_trip() {
        // Mask 03 00 00 with values 10 and 20 survives a write and a read.
        let bytes = write(&model(), 0x59).unwrap();
        let d = read(&bytes, &ReadOptions::default()).unwrap();
        let b = d.body.unwrap();
        assert_eq!(b.stats.entries().len(), 2);
        assert_eq!(b.stats.entries()[1].value, 20);
    }
}
