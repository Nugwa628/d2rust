//! Reads the game's own tables into a [`GameMap`]. The tables are the source
//! of truth; the override file adds only towns, story order, gates, rewards
//! and hand-written goals.

use std::collections::{BTreeMap, BTreeSet};

use d2_data::bin::{BinSet, BinTable};
use d2_data::strings::StringTables;
use d2_data::tables::{
    self, decode_all, Armor, Charstats, Itemtypes, Leveldefs, Levels, Misc, Monstats, Objects,
    Runes, Skills, Superuniques, Weapons,
};
use d2_sim::world::cube::{self, input_flags, output_flags, InputSlot, OutputSlot};
use d2_sim::world::quests::{MessageEntry, QuestRow, QuestTables};

use super::model::*;
use super::overrides::Overrides;

/// Everything the reader takes.
pub struct Inputs<'a> {
    pub bins: &'a BinSet,
    pub quests: &'a QuestTables,
    pub overrides: &'a Overrides,
    pub source: String,
    pub mod_name: Option<String>,
    pub overrides_path: Option<String>,
}

fn table<'a>(bins: &'a BinSet, name: &str) -> Result<&'a BinTable, String> {
    bins.table(name)
        .ok_or_else(|| format!("table {name}: not loaded"))
}

fn rows<T: tables::Record>(bins: &BinSet) -> Result<Vec<T>, String> {
    decode_all::<T>(table(bins, T::TABLE)?).map_err(|e| format!("table {}: {e}", T::TABLE))
}

fn name(strings: &StringTables, key: u16) -> String {
    strings
        .by_index(u32::from(key))
        .ok()
        .map(|v| String::from_utf16_lossy(&v))
        .unwrap_or_default()
}

fn named(strings: &StringTables, key: u16, fallback: String) -> String {
    let text = name(strings, key);
    if text.is_empty() {
        fallback
    } else {
        text
    }
}

fn item_code(items: &[[u8; 4]], index: u32) -> Option<String> {
    items
        .get(index as usize)
        .map(code_text)
        .filter(|c| !c.is_empty())
}

fn type_code(types: &[Itemtypes], index: u32) -> Option<String> {
    types
        .get(index as usize)
        .map(|t| code_text(&t.code))
        .filter(|c| !c.is_empty())
}

/// The direct type code and its `Equiv1`/`Equiv2` parents, recursively.
/// Runewords and cube recipes name both leaf types (`swor`) and category
/// types (`mele`, `weap`, `shld`, ...), and this is the set an item belongs to.
fn type_closure(types: &[Itemtypes], index: u16) -> Vec<String> {
    let mut out = BTreeSet::new();
    let mut stack = vec![index];
    while let Some(i) = stack.pop() {
        if i == u16::MAX {
            continue;
        }
        let Some(row) = types.get(usize::from(i)) else {
            continue;
        };
        let code = code_text(&row.code);
        if code.is_empty() || !out.insert(code) {
            continue;
        }
        for parent in [row.equiv1, row.equiv2] {
            if parent != u16::MAX {
                stack.push(parent);
            }
        }
    }
    out.into_iter().collect()
}

fn all_item_codes(weapons: &[Weapons], armor: &[Armor], misc: &[Misc]) -> Vec<[u8; 4]> {
    weapons
        .iter()
        .map(|r| r.code)
        .chain(armor.iter().map(|r| r.code))
        .chain(misc.iter().map(|r| r.code))
        .collect()
}

fn slots(
    rarm: u8,
    larm: u8,
    torso: u8,
    legs: u8,
    rspad: u8,
    lspad: u8,
    two_handed: u8,
) -> Option<String> {
    let mut out = Vec::new();
    if rarm != 0 || two_handed != 0 {
        out.push("right_hand");
    }
    if larm != 0 || two_handed != 0 {
        out.push("left_hand");
    }
    if torso != 0 {
        out.push("torso");
    }
    if legs != 0 {
        out.push("legs");
    }
    if rspad != 0 {
        out.push("right_shoulder");
    }
    if lspad != 0 {
        out.push("left_shoulder");
    }
    (!out.is_empty()).then(|| out.join(","))
}

fn item_type_name(types: &[Itemtypes], index: u16) -> String {
    type_code(types, u32::from(index)).unwrap_or_else(|| index.to_string())
}

fn skill_target(r: &Skills) -> &'static str {
    if r.targetally {
        "ally"
    } else if r.targetpet {
        "pet"
    } else if r.targetcorpse {
        "corpse"
    } else if r.targetitem {
        "item"
    } else if r.searchenemyxy || r.searchenemynear {
        "monster"
    } else if r.searchopenxy || r.tgtplacecheck {
        "ground"
    } else {
        "self"
    }
}

fn input_slot(slot: &InputSlot, items: &[[u8; 4]], types: &[Itemtypes]) -> CubeInput {
    if slot.flags & input_flags::USEANY != 0 {
        return CubeInput {
            kind: "any".into(),
            quality: u32::from(slot.quality),
            quantity: u32::from(slot.quantity),
            ..CubeInput::default()
        };
    }
    if slot.flags & input_flags::ITEMCODE != 0 {
        return CubeInput {
            kind: "item".into(),
            code: item_code(items, u32::from(slot.item)),
            quality: u32::from(slot.quality),
            quantity: u32::from(slot.quantity),
            ..CubeInput::default()
        };
    }
    let item_type = type_code(types, u32::from(slot.item));
    CubeInput {
        kind: if item_type.is_some() {
            "type".into()
        } else {
            "missing".into()
        },
        item_type,
        quality: u32::from(slot.quality),
        quantity: u32::from(slot.quantity),
        ..CubeInput::default()
    }
}

fn output_slot(slot: &OutputSlot, items: &[[u8; 4]], types: &[Itemtypes]) -> CubeOutput {
    let kind = match slot.kind {
        cube::kind::ITEMCODE => "item",
        cube::kind::ITEMTYPE => "type",
        // Both use the item that was put in: `USEITEM` keeps that item,
        // `USETYPE` rolls another item of its class. Neither names a type code.
        cube::kind::USEITEM => "input_item",
        cube::kind::USETYPE => "input_type",
        cube::kind::COW_PORTAL | cube::kind::PANDEMONIUM | cube::kind::PANDEMONIUM_FINALE => {
            "portal"
        }
        _ => "none",
    };
    let (code, item_type) = match slot.kind {
        cube::kind::ITEMCODE => (item_code(items, u32::from(slot.item)), None),
        cube::kind::ITEMTYPE => (None, type_code(types, u32::from(slot.item))),
        cube::kind::USEITEM | cube::kind::USETYPE => (None, None),
        _ if slot.flags & output_flags::SPECIAL == 0 && slot.item != u16::MAX => {
            (item_code(items, u32::from(slot.item)), None)
        }
        _ => (None, None),
    };
    CubeOutput {
        kind: kind.into(),
        code,
        item_type,
        quality: u32::from(slot.quality),
        quantity: u32::from(slot.quantity),
    }
}

fn vendor_services() -> BTreeMap<u32, (String, u32, Vec<String>)> {
    const VENDORS: &str = include_str!("../../../../../tables/world/vendors.tsv");
    let mut out = BTreeMap::new();
    for line in VENDORS.lines().skip(1) {
        let c: Vec<&str> = line.split('\t').collect();
        if c.len() < 13 {
            continue;
        }
        let Ok(id) = c[0].parse::<u32>() else {
            continue;
        };
        let flag = |i: usize| c[i] != "0";
        let mut services = Vec::new();
        if flag(3) || flag(8) {
            services.push("trade".to_owned());
        }
        if flag(6) || flag(9) {
            services.push("gamble".to_owned());
        }
        if flag(10) {
            services.push("heal".to_owned());
        }
        if flag(11) {
            services.push("identify".to_owned());
        }
        if flag(12) {
            services.push("hire".to_owned());
        }
        if c.get(13).is_some_and(|v| *v != "0") {
            services.push("resurrect".to_owned());
        }
        out.insert(id, (c[1].to_owned(), c[2].parse().unwrap_or(0), services));
    }
    out
}

fn object_labels() -> BTreeMap<u8, String> {
    const FUNCS: &str = include_str!("../../../../../tables/world/object-functions.tsv");
    FUNCS
        .lines()
        .skip(1)
        .filter_map(|line| {
            let c: Vec<&str> = line.split('\t').collect();
            if c.len() < 4 || c[0] != "operate" {
                return None;
            }
            Some((c[1].parse::<u8>().ok()?, c[3].to_owned()))
        })
        .collect()
}

fn quests(
    strings: &StringTables,
    monstats: &[Monstats],
    quests: &QuestTables,
    overrides: &Overrides,
) -> Vec<Quest> {
    let rewards: BTreeMap<u32, Vec<Reward>> = {
        let mut map: BTreeMap<u32, Vec<Reward>> = BTreeMap::new();
        for r in &overrides.rewards {
            map.entry(r.chain).or_default().push(Reward {
                kind: r.kind.clone(),
                name: r.name.clone(),
                item: r.item.clone(),
            });
        }
        map
    };
    let prereq: BTreeMap<u32, Vec<u32>> = overrides.prerequisites.iter().cloned().collect();
    quests
        .rows
        .iter()
        .filter_map(|row| {
            let flag = row.flag.map(u32::from)?;
            let entries: Vec<&MessageEntry> = row.msgs.map_or_else(Vec::new, |table| {
                quests
                    .messages
                    .iter()
                    .filter(|m| m.table == table)
                    .collect()
            });
            let mut by_state: BTreeMap<u32, Vec<&MessageEntry>> = BTreeMap::new();
            for entry in entries {
                by_state
                    .entry(u32::from(entry.state))
                    .or_default()
                    .push(entry);
            }
            let steps = by_state
                .into_iter()
                .map(|(state, entries)| {
                    let npcs: BTreeSet<u32> = entries.iter().map(|m| u32::from(m.npc)).collect();
                    let menus: BTreeSet<u32> = entries.iter().map(|m| m.menu).collect();
                    let messages: BTreeSet<u32> =
                        entries.iter().map(|m| u32::from(m.string)).collect();
                    let pairs: BTreeSet<(u32, u32)> = entries
                        .iter()
                        .map(|m| (u32::from(m.npc), u32::from(m.string)))
                        .collect();
                    let text = if npcs.is_empty() {
                        format!("state {state}")
                    } else {
                        let names: Vec<String> = npcs
                            .iter()
                            .map(|&n| {
                                monstats.get(n as usize).map_or_else(
                                    || format!("npc {n}"),
                                    |m| named(strings, m.namestr, format!("npc {n}")),
                                )
                            })
                            .collect();
                        format!("talk to {}", names.join(" or "))
                    };
                    QuestStep {
                        state,
                        npcs: npcs.into_iter().collect(),
                        menus: menus.into_iter().collect(),
                        messages: messages.into_iter().collect(),
                        npc_messages: pairs
                            .into_iter()
                            .map(|(npc, message)| NpcMessage { npc, message })
                            .collect(),
                        text,
                    }
                })
                .collect();
            Some(Quest {
                index: u32::from(row.index),
                chain: u32::from(row.chain),
                act: u32::from(row.act) + 1,
                name: row.name.clone(),
                flag: Some(flag),
                version: row.version,
                message_table: row.msgs,
                steps,
                rewards: rewards
                    .get(&u32::from(row.chain))
                    .cloned()
                    .unwrap_or_default(),
                prerequisites: prereq
                    .get(&u32::from(row.chain))
                    .cloned()
                    .unwrap_or_default(),
            })
        })
        .collect()
}

/// Builds the map.
pub fn build(input: &Inputs<'_>) -> Result<GameMap, String> {
    let bins = input.bins;
    let strings = &bins.strings;
    let levels: Vec<Levels> = rows(bins)?;
    let defs: Vec<Leveldefs> = rows(bins)?;
    let monstats: Vec<Monstats> = rows(bins)?;
    let superuniques: Vec<Superuniques> = rows(bins)?;
    let charstats: Vec<Charstats> = rows(bins)?;
    let skills: Vec<Skills> = rows(bins)?;
    let itemtypes: Vec<Itemtypes> = rows(bins)?;
    let weapons: Vec<Weapons> = rows(bins)?;
    let armor: Vec<Armor> = rows(bins)?;
    let misc: Vec<Misc> = rows(bins)?;
    let objects: Vec<Objects> = rows(bins)?;
    let runes: Vec<Runes> = rows(bins)?;

    let towns: BTreeSet<u32> = input.overrides.towns.iter().copied().collect();
    let mut areas = Vec::new();
    for (id, level) in levels.iter().enumerate() {
        if id == 0 {
            continue;
        }
        let size = defs.get(id).map(|d| [d.sizex, d.sizey]).unwrap_or([0, 0]);
        areas.push(Area {
            id: id as u32,
            name: bytes_text(&level.levelname),
            act: u32::from(level.act) + 1,
            // The `Waypoint` column is the waypoint index; 255 is "none".
            waypoint: level.waypoint != u8::MAX,
            town: towns.contains(&(id as u32)),
            size,
            monster_levels: [
                u32::from(level.monlvl1),
                u32::from(level.monlvl2),
                u32::from(level.monlvl3),
            ],
        });
    }
    let area_ids: BTreeSet<u32> = areas.iter().map(|a| a.id).collect();
    let area_act: BTreeMap<u32, u32> = areas.iter().map(|a| (a.id, a.act)).collect();
    let town_of_act: BTreeMap<u32, u32> = areas
        .iter()
        .filter(|a| a.town)
        .map(|a| (a.act, a.id))
        .collect();

    let mut links: BTreeMap<(u32, u32), Link> = BTreeMap::new();
    let mut add_link =
        |from: u32, to: u32, kind: &str, reason: Option<&str>, defs: &[Leveldefs]| {
            let key = (from.min(to), from.max(to));
            let entry = links.entry(key).or_insert_with(|| Link {
                from: key.0,
                to: key.1,
                kinds: Vec::new(),
                portal: defs.get(key.0 as usize).is_some_and(|d| d.portal != 0)
                    || defs.get(key.1 as usize).is_some_and(|d| d.portal != 0),
                gate: None,
                reason: None,
            });
            if !entry.kinds.iter().any(|k| k == kind) {
                entry.kinds.push(kind.to_owned());
            }
            if let Some(reason) = reason {
                entry.reason = Some(match entry.reason.take() {
                    Some(old) if old != reason => format!("{old}; {reason}"),
                    _ => reason.to_owned(),
                });
            }
        };
    // `Vis*` names the neighbouring level; `Warp*` names the lvlwarp row used
    // for that slot, not a destination level. An outdoor level's warp slots
    // may be -1 while its `Vis*` slot still joins the next wilderness area.
    for (id, def) in defs.iter().enumerate() {
        let from = id as u32;
        let vis = [
            def.vis0, def.vis1, def.vis2, def.vis3, def.vis4, def.vis5, def.vis6, def.vis7,
        ];
        let warp = [
            def.warp0, def.warp1, def.warp2, def.warp3, def.warp4, def.warp5, def.warp6, def.warp7,
        ];
        for (vis, warp) in vis.iter().zip(warp.iter()) {
            let Some(to) = link_u32(*vis) else {
                continue;
            };
            if to == from || !area_ids.contains(&to) {
                continue;
            }
            // Level visibility never crosses an act; a cross-act transition
            // is the overrides' business (it is a portal, not a walk edge).
            if area_act.get(&to) != area_act.get(&from) {
                continue;
            }
            add_link(from, to, "vis", None, &defs);
            if *warp != u32::MAX {
                add_link(from, to, "warp", None, &defs);
            }
        }
    }
    // Edges the tables cannot name: the outdoor placer joins wilderness
    // levels by geometry and the act towns by gates/portals.
    for link in &input.overrides.links {
        add_link(link.from, link.to, &link.kind, Some(&link.reason), &defs);
    }
    for path in &input.overrides.paths {
        for pair in path.areas.windows(2) {
            add_link(pair[0], pair[1], &path.kind, Some(&path.reason), &defs);
        }
    }
    // An act transition is a one-way portal in play; the graph keeps it as a
    // link from the source act's town to the destination town or area.
    for transition in &input.overrides.transitions {
        let Some(&from) = town_of_act.get(&transition.from) else {
            continue;
        };
        let to = transition
            .area
            .or_else(|| town_of_act.get(&transition.to).copied());
        let Some(to) = to else {
            continue;
        };
        add_link(from, to, "act", Some("act transition portal"), &defs);
    }
    for gate in &input.overrides.gates {
        let key = (gate.from.min(gate.to), gate.from.max(gate.to));
        if let Some(link) = links.get_mut(&key) {
            link.gate = Some(gate.kind.clone());
        }
    }
    let mut world_links: Vec<Link> = links.into_values().collect();
    world_links.retain(|l| area_ids.contains(&l.from) && area_ids.contains(&l.to));
    world_links.sort_by_key(|l| (l.from, l.to));

    let waypoints = areas
        .iter()
        .filter(|a| a.waypoint)
        .map(|a| Waypoint {
            level: a.id,
            act: a.act,
            town: a.town,
            name: a.name.clone(),
        })
        .collect::<Vec<_>>();

    let mut classes = Vec::new();
    for (id, c) in charstats.iter().enumerate() {
        let class_name = bytes_text(&c.class);
        if class_name.is_empty() {
            continue;
        }
        classes.push(Class {
            id: id as u32,
            name: class_name,
            skills: skills
                .iter()
                .enumerate()
                .filter(|(_, s)| u32::from(s.charclass) == id as u32)
                .map(|(i, _)| i as u32)
                .collect(),
        });
    }

    let all_items = all_item_codes(&weapons, &armor, &misc);
    let mut items = Vec::new();
    for (index, r) in weapons.iter().enumerate() {
        items.push(Item {
            code: code_text(&r.code),
            name: named(strings, r.namestr, format!("weapon {index}")),
            kind: "weapon".into(),
            item_type: item_type_name(&itemtypes, r.type_),
            types: type_closure(&itemtypes, r.type_),
            slot: slots(
                r.rarm,
                r.larm,
                r.torso,
                r.legs,
                r.rspad,
                r.lspad,
                r.f_2handed,
            ),
            max_sockets: u32::from(r.gemsockets),
            level_req: u32::from(r.levelreq),
            level: u32::from(r.level),
            stackable: r.stackable != 0,
        });
    }
    for (index, r) in armor.iter().enumerate() {
        items.push(Item {
            code: code_text(&r.code),
            name: named(strings, r.namestr, format!("armor {index}")),
            kind: "armor".into(),
            item_type: item_type_name(&itemtypes, r.type_),
            types: type_closure(&itemtypes, r.type_),
            slot: slots(
                r.rarm,
                r.larm,
                r.torso,
                r.legs,
                r.rspad,
                r.lspad,
                r.f_2handed,
            ),
            max_sockets: u32::from(r.gemsockets),
            level_req: u32::from(r.levelreq),
            level: u32::from(r.level),
            stackable: r.stackable != 0,
        });
    }
    for (index, r) in misc.iter().enumerate() {
        items.push(Item {
            code: code_text(&r.code),
            name: named(strings, r.namestr, format!("misc {index}")),
            kind: "misc".into(),
            item_type: item_type_name(&itemtypes, r.type_),
            types: type_closure(&itemtypes, r.type_),
            slot: slots(
                r.rarm,
                r.larm,
                r.torso,
                r.legs,
                r.rspad,
                r.lspad,
                r.f_2handed,
            ),
            max_sockets: u32::from(r.gemsockets),
            level_req: u32::from(r.levelreq),
            level: u32::from(r.level),
            stackable: r.stackable != 0,
        });
    }

    let skill_rows = skills
        .iter()
        .enumerate()
        .map(|(id, r)| Skill {
            id: id as u32,
            name: named(strings, r.skill, format!("skill {id}")),
            class: u32::from(r.charclass),
            req_level: u32::from(r.reqlevel),
            max_level: u32::from(r.maxlvl),
            mana: u32::from(r.mana),
            mana_per_level: u32::from(r.lvlmana),
            passive: r.passive,
            aura: r.aura,
            target: skill_target(r).into(),
            requires: [r.reqskill1, r.reqskill2, r.reqskill3]
                .iter()
                .filter_map(|&s| link_u16(s))
                .collect(),
        })
        .collect::<Vec<_>>();

    let mut boss_kills: Vec<BossKill> = Vec::new();
    // Where a boss stands is in the level maps, not the tables, so the
    // overrides name each level boss. (A level's `umon` columns are the
    // monster classes it may roll as random uniques, not its bosses.)
    let monster_name = |class: u32| {
        monstats
            .get(class as usize)
            .map(|m| named(strings, m.namestr, format!("monster {class}")))
            .unwrap_or_else(|| format!("monster {class}"))
    };
    for boss in &input.overrides.bosses {
        let (class, name) = match (boss.superunique, boss.monster) {
            (Some(row), _) => {
                let su = superuniques.get(row as usize).ok_or_else(|| {
                    format!("boss in area {}: no superuniques row {row}", boss.area)
                })?;
                (su.class, named(strings, su.name, monster_name(su.class)))
            }
            (None, Some(class)) => {
                if class as usize >= monstats.len() {
                    return Err(format!(
                        "boss in area {}: no monstats row {class}",
                        boss.area
                    ));
                }
                (class, monster_name(class))
            }
            (None, None) => return Err(format!("boss in area {}: no boss named", boss.area)),
        };
        boss_kills.push(BossKill {
            name,
            class,
            area: boss.area,
            act: levels
                .get(boss.area as usize)
                .map_or(0, |l| u32::from(l.act) + 1),
            superunique: boss.superunique,
            setup: boss.setup.clone(),
            from_town: boss.from_town,
            fight: boss.fight,
            pending: boss.pending.clone(),
        });
    }

    let vendors = vendor_services();
    let quest_npcs: BTreeMap<u32, BTreeSet<u32>> = {
        let mut map: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
        for m in &input.quests.messages {
            map.entry(u32::from(m.npc)).or_default().insert(m.table);
        }
        map
    };
    let mut npcs = Vec::new();
    for (id, (vendor_name, act, mut services)) in vendors {
        let row = monstats.get(id as usize);
        if quest_npcs.contains_key(&id) {
            services.push("quest".to_owned());
        }
        services.sort();
        services.dedup();
        let chains: Vec<u32> = quest_npcs
            .get(&id)
            .map(|tables| {
                input
                    .quests
                    .rows
                    .iter()
                    .filter(|r: &&QuestRow| r.msgs.is_some_and(|t| tables.contains(&t)))
                    .map(|r| u32::from(r.chain))
                    .collect()
            })
            .unwrap_or_default();
        npcs.push(Npc {
            class: id,
            name: row
                .map(|m| named(strings, m.namestr, vendor_name.clone()))
                .unwrap_or(vendor_name),
            act: act + 1,
            services,
            quest_chains: chains,
        });
    }

    let labels = object_labels();
    let objects = objects
        .iter()
        .enumerate()
        .map(|(class, o)| GameObject {
            class: class as u32,
            token: bytes_text(&o.token),
            name: bytes_text(&o.name),
            kind: labels
                .get(&o.operatefn)
                .cloned()
                .unwrap_or_else(|| format!("operate {}", o.operatefn)),
            act: u32::from(o.act) + 1,
            door: o.isdoor != 0,
            portal: o.openwarp != 0,
            lockable: o.lockable != 0,
        })
        .collect::<Vec<_>>();

    let mut cube_recipes = Vec::new();
    if let Ok(t) = table(bins, "cubemain") {
        if let Ok(recipes) = cube::recipes(t) {
            for (index, r) in recipes.iter().enumerate() {
                let inputs = r.inputs[..usize::from(r.numinputs).min(7)]
                    .iter()
                    .map(|s| input_slot(s, &all_items, &itemtypes))
                    .collect();
                let outputs = r
                    .outputs
                    .iter()
                    .filter(|o| o.kind != 0 || o.item != u16::MAX)
                    .map(|o| output_slot(o, &all_items, &itemtypes))
                    .collect();
                cube_recipes.push(CubeRecipe {
                    index: index as u32,
                    enabled: r.enabled != 0,
                    min_difficulty: u32::from(r.min_diff),
                    num_inputs: u32::from(r.numinputs),
                    inputs,
                    outputs,
                });
            }
        }
    }

    let runewords = runes
        .iter()
        .map(|r| {
            let rune_list = [r.rune1, r.rune2, r.rune3, r.rune4, r.rune5, r.rune6]
                .iter()
                .filter_map(|&i| item_code(&all_items, i))
                .collect();
            let types = [r.itype1, r.itype2, r.itype3, r.itype4, r.itype5, r.itype6]
                .iter()
                .filter_map(|&i| type_code(&itemtypes, u32::from(i)))
                .collect();
            Runeword {
                name: bytes_text(&r.name),
                complete: r.complete != 0,
                runes: rune_list,
                item_types: types,
            }
        })
        .collect::<Vec<_>>();

    let mut interactions = Vec::new();
    for skill in &skill_rows {
        if skill.target != "self" || !skill.passive {
            interactions.push(Interaction {
                actor: format!("skill:{}", skill.id),
                action: if skill.passive { "passive" } else { "cast" }.into(),
                target: skill.target.clone(),
                detail: skill.name.clone(),
            });
        }
    }
    for npc in &npcs {
        for service in &npc.services {
            interactions.push(Interaction {
                actor: format!("npc:{}", npc.class),
                action: service.clone(),
                target: "player".into(),
                detail: npc.name.clone(),
            });
        }
    }
    for _recipe in &cube_recipes {
        interactions.push(Interaction {
            actor: "item".into(),
            action: "cube".into(),
            target: "item".into(),
            detail: "cubemain".into(),
        });
    }
    for object in objects.iter().filter(|o| !o.token.is_empty()) {
        interactions.push(Interaction {
            actor: format!("object:{}", object.class),
            action: "operate".into(),
            target: "player".into(),
            detail: object.kind.clone(),
        });
    }

    let objectives = Objectives {
        quests: quests(strings, &monstats, input.quests, input.overrides),
        boss_kills,
        act_transitions: input.overrides.transitions.clone(),
        endgame: input
            .overrides
            .final_objective
            .as_ref()
            .map(|name| Endgame {
                name: name.clone(),
                area: None,
                boss: None,
            }),
    };
    let summary = Summary {
        acts: areas.iter().map(|a| a.act).max().unwrap_or(0),
        areas: areas.len() as u32,
        links: world_links.len() as u32,
        waypoints: waypoints.len() as u32,
        towns: areas.iter().filter(|a| a.town).count() as u32,
        quests: objectives.quests.len() as u32,
        boss_kills: objectives.boss_kills.len() as u32,
        classes: classes.len() as u32,
        skills: skill_rows.len() as u32,
        items: items.len() as u32,
        npcs: npcs.len() as u32,
        objects: objects.len() as u32,
        cube_recipes: cube_recipes.len() as u32,
        runewords: runewords.len() as u32,
    };
    let game = GameInfo {
        source: input.source.clone(),
        mod_name: input.mod_name.clone(),
        overrides: input.overrides_path.clone(),
        final_objective: input.overrides.final_objective.clone(),
    };
    Ok(GameMap {
        format: FORMAT.to_owned(),
        game,
        summary,
        objectives,
        world: World {
            areas,
            links: world_links,
            waypoints,
            towns: towns.into_iter().collect(),
            unreachable: input
                .overrides
                .unreachable
                .iter()
                .map(|u| UnreachableArea {
                    area: u.area,
                    reason: u.reason.clone(),
                })
                .collect(),
        },
        playables: Playables {
            classes,
            skills: skill_rows,
            items,
            npcs,
            objects,
            cube_recipes,
            runewords,
        },
        mechanics: Mechanics { interactions },
    })
}
