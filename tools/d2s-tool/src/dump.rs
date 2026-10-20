//! A readable listing of a save: header fields, section summaries, stat
//! entries, skill bytes and every item's code and location.

use std::fmt::Write as _;

use d2_formats::d2s::{status, Body, D2s, ItemEntry, Stats, Waypoints};
use d2_proto::item_bits::Location;

use crate::items::code_str;
use crate::tables::Tables;

fn cstr(b: &[u8]) -> String {
    let n = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..n]).into_owned()
}

fn status_text(s: u16) -> String {
    let mut f = Vec::new();
    for (bit, n) in [
        (status::NEW, "new"),
        (status::REALM, "realm"),
        (status::HARDCORE, "hardcore"),
        (status::DEAD, "dead"),
        (status::EXPANSION, "expansion"),
        (status::LADDER, "ladder"),
    ] {
        if s & bit != 0 {
            f.push(n);
        }
    }
    format!(
        "{s:#06x} [{}] progression {}",
        f.join(" "),
        status::progression(s)
    )
}

/// `dump --items`: each item's decoded fields and its entry bytes (hex).
pub static ITEM_DETAIL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn items(out: &mut String, label: &str, list: &[ItemEntry], t: Option<&Tables>) {
    let _ = writeln!(out, "{label}: {} item(s)", list.len());
    for (i, e) in list.iter().enumerate() {
        let Some(t) = t else {
            let _ = writeln!(out, "  [{i}] {} bytes", e.bytes.len());
            continue;
        };
        match t.decode_entry(&e.bytes) {
            Ok(d) => {
                let it = &d.item;
                let loc = match it.location {
                    Some(Location::Slot { body, x, y, page1 }) => {
                        format!("body {body} x {x} y {y} page+1 {page1}")
                    }
                    Some(Location::Ground { x, y }) => format!("ground {x},{y}"),
                    None => "-".to_owned(),
                };
                let _ = writeln!(
                    out,
                    "  [{i}] {:<4} mode {} {loc} | version {} quality {} ilvl {} flags {:#010x} | {} bytes, {} socketed",
                    code_str(&it.code),
                    it.mode,
                    it.version,
                    it.quality,
                    it.ilvl,
                    it.flags,
                    e.bytes.len(),
                    d.children.len()
                );
                if ITEM_DETAIL.load(std::sync::atomic::Ordering::Relaxed) {
                    let _ = writeln!(out, "      {:?}", d.item);
                    let hex: String = e.bytes.iter().map(|b| format!("{b:02x}")).collect();
                    let _ = writeln!(out, "      bytes {hex}");
                }
            }
            Err(err) => {
                let _ = writeln!(out, "  [{i}] {} bytes, undecodable: {err}", e.bytes.len());
            }
        }
    }
}

fn body(out: &mut String, b: &Body, class: u8, t: Option<&Tables>) {
    let _ = writeln!(out, "quests (size field {:#x}):", b.quests.size);
    for (d, r) in b.quests.records.iter().enumerate() {
        let set: Vec<String> = (0..48)
            .filter_map(|q| {
                let w = u16::from_le_bytes([r[2 * q], r[2 * q + 1]]);
                (w != 0).then(|| format!("{q}:{w:#06x}"))
            })
            .collect();
        let _ = writeln!(out, "  difficulty {d}: {}", set.join(" "));
    }
    let _ = writeln!(out, "waypoints:");
    for (d, r) in b.waypoints.records.iter().enumerate() {
        let known: Vec<String> = (0..0x70u8)
            .filter(|&n| Waypoints::bit(n).is_some_and(|(by, m)| r[by] & m != 0))
            .map(|n| n.to_string())
            .collect();
        let _ = writeln!(
            out,
            "  difficulty {d}: magic {:#06x} known [{}]",
            u16::from_le_bytes([r[0], r[1]]),
            known.join(",")
        );
    }
    let hex = |f: &[u8; 8]| f.iter().map(|b| format!("{b:02x}")).collect::<String>();
    for d in 0..3 {
        let _ = writeln!(
            out,
            "npc flags {d}: A {} B {}",
            hex(&b.npcs.a[d]),
            hex(&b.npcs.b[d])
        );
    }
    match &b.stats {
        Stats::Bits(v) => {
            let _ = writeln!(out, "stats ({} entries):", v.len());
            for e in v {
                let _ = writeln!(out, "  stat {:>3} layer {} = {}", e.id, e.layer, e.value);
            }
        }
        Stats::Mask { mask, values } => {
            let _ = writeln!(out, "stats (mask layout {mask:02x?}):");
            for (id, v) in values {
                let _ = writeln!(out, "  stat {id:>3} = {v}");
            }
        }
    }
    let list = t.map(|t| t.class_skills(class));
    let sk: Vec<String> = b
        .skills
        .iter()
        .enumerate()
        .map(|(i, &l)| match list.and_then(|l| l.get(i)) {
            Some(id) => format!("{i}(skill {id})={l}"),
            None => format!("{i}={l}"),
        })
        .collect();
    let _ = writeln!(out, "skills ({} bytes): {}", b.skills.len(), sk.join(" "));
    items(out, "player items", &b.items, t);
    let _ = writeln!(out, "corpses: {}", b.corpses.len());
    for c in &b.corpses {
        let _ = writeln!(out, "  unk {:#x} at {},{}", c.unk, c.x, c.y);
        items(out, "  corpse items", &c.items, t);
    }
    match &b.hireling_items {
        None => {
            let _ = writeln!(out, "jf: absent");
        }
        Some(None) => {
            let _ = writeln!(out, "jf: marker, no list");
        }
        Some(Some(l)) => items(out, "jf hireling items", l, t),
    }
    match &b.golem {
        None => {
            let _ = writeln!(out, "kf: absent");
        }
        Some(g) => {
            let _ = writeln!(out, "kf: flag {}", g.flag);
            if let Some(it) = &g.item {
                items(out, "  golem item", std::slice::from_ref(it), t);
            }
        }
    }
    if !b.trailing.is_empty() {
        let _ = writeln!(out, "trailing bytes: {}", b.trailing.len());
    }
}

/// The listing of a parsed save.
pub fn dump(s: &D2s, t: Option<&Tables>) -> String {
    let h = &s.header;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "version {:#x}, file size {}, checksum {:#010x}",
        h.version, h.file_size, h.checksum
    );
    let _ = writeln!(
        out,
        "name {:?} class {} level {}",
        cstr(&h.name),
        h.class,
        h.level
    );
    let _ = writeln!(out, "status {}", status_text(h.status));
    let _ = writeln!(
        out,
        "stat count {:#x}, skill count {}, weapon switch {:#x}, +0x26 {:#x}, +0x34 {:#x}",
        h.stat_count, h.skill_count, h.weapon_switch, h.unk26, h.unk34
    );
    let _ = writeln!(
        out,
        "create time {}, save time {}",
        h.create_time, h.save_time
    );
    let _ = writeln!(
        out,
        "towns {:02x?}, map seed {:#010x}, +0xCF {:#x}",
        h.towns, h.map_seed, h.client_cf
    );
    let slots: Vec<String> = h
        .mouse
        .iter()
        .map(|m| {
            let (s, l, i) = m.decode();
            format!("{s}{}@{i}", if l { "L" } else { "" })
        })
        .collect();
    let _ = writeln!(
        out,
        "mouse skills (left, right, swap left, swap right): {}",
        slots.join(" ")
    );
    let hk: Vec<String> = h
        .hotkeys
        .iter()
        .enumerate()
        .filter(|(_, k)| k.code != 0xFFFF)
        .map(|(i, k)| format!("{i}:{:#06x}@{}", k.code, k.item))
        .collect();
    let _ = writeln!(out, "hotkeys: {}", hk.join(" "));
    let _ = writeln!(out, "components {:02x?}", h.components);
    let _ = writeln!(out, "colours    {:02x?}", h.colours);
    let hl = &h.hireling;
    let _ = writeln!(
        out,
        "hireling: present {} flags {:#x} seed {:#x} name index {} id {} experience {}",
        hl.is_present(),
        hl.flags,
        hl.seed,
        hl.name_index,
        hl.id,
        hl.experience
    );
    match &s.body {
        None => {
            let _ = writeln!(out, "new-character stub (335 bytes, no sections)");
        }
        Some(b) => body(&mut out, b, h.class, t),
    }
    out
}
