use std::collections::{BTreeMap, BTreeSet};

use d2_data::bin::BinTable;
use d2_data::tables::{Monstats, Record};

use super::hire::{capped_level, hire_init, price, resurrect_cost};
use super::services::{can_imbue, can_personalize, can_socket, imbue_level, socket_count};
use super::*;

mod mod_npc_tests;
mod mutant_tests;

// ------------------------------------------------------------ fake

const PLAYER: UnitId = UnitId(1);

fn hex(s: &str) -> Vec<u8> {
    let s: String = s.split_whitespace().collect();
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

#[derive(Default)]
struct Unit {
    guid: u32,
    ty: u8,
    class: Option<u16>,
    mode: u8,
    stats: BTreeMap<u16, u32>,
    max: [u32; 3],
    states: BTreeSet<u16>,
    lists: BTreeSet<u16>,
    interaction: Option<InteractionList>,
    flags: u32,
}

#[derive(Default)]
struct Fake {
    units: BTreeMap<UnitId, Unit>,
    dist: i32,
    axis: u32,
    other_act: bool,
    unit_check: u32,
    busy: u32,
    refuse_start: bool,
    cain_busy: bool,
    interact: BTreeMap<UnitId, (u8, u32)>,
    pets: Vec<UnitId>,
    hireling: Option<UnitId>,
    /// The hireling's node is living (bit 0 clear): `(7, 0)` finds it.
    hireling_living: bool,
    curable: BTreeSet<u16>,
    states_count: u16,
    flags: QuestFlags,
    inv: Vec<InvEntry>,
    cursor: Option<UnitId>,
    facts: BTreeMap<UnitId, ItemFacts>,
    no_dup: bool,
    no_remove: bool,
    no_create: bool,
    seeds: BTreeMap<UnitId, Seed>,
    spawn_fails: usize,
    next: u32,
    sent: Vec<Vec<u8>>,
    log: Vec<String>,
    /// The game's rules (default `Original`; a mod's in the MS8c tests).
    rules: crate::rules::Ruleset,
}

impl Fake {
    fn new() -> Self {
        let mut f = Fake {
            dist: 3,
            next: 100,
            ..Fake::default()
        };
        f.units.insert(
            PLAYER,
            Unit {
                guid: 1,
                mode: 1,
                max: [100, 50, 80],
                ..Unit::default()
            },
        );
        f.set(PLAYER, stat::LIFE, 100);
        f.set(PLAYER, stat::MANA, 50);
        f.set(PLAYER, stat::STAMINA, 80);
        f.set(PLAYER, stat::LEVEL, 10);
        f.set(PLAYER, stat::GOLD, 500);
        f
    }

    fn set(&mut self, u: UnitId, s: u16, v: u32) {
        self.units.get_mut(&u).unwrap().stats.insert(s, v);
    }

    fn get(&self, u: UnitId, s: u16) -> u32 {
        self.units[&u].stats.get(&s).copied().unwrap_or(0)
    }

    fn npc(&mut self, class: u16, guid: u32) -> UnitId {
        let id = UnitId(guid + 1000);
        self.units.insert(
            id,
            Unit {
                guid,
                ty: 1,
                class: Some(class),
                mode: 1,
                interaction: Some(InteractionList::default()),
                ..Unit::default()
            },
        );
        id
    }

    fn item(&mut self, facts: ItemFacts) -> UnitId {
        self.next += 1;
        let id = UnitId(self.next);
        self.units.insert(
            id,
            Unit {
                guid: self.next,
                ty: 4,
                ..Unit::default()
            },
        );
        self.facts.insert(id, facts);
        id
    }

    fn ids(&self) -> Vec<u8> {
        self.sent.iter().map(|m| m[0]).collect()
    }

    fn list(&self, npc: UnitId) -> Vec<(UnitId, u8)> {
        self.units[&npc].interaction.as_ref().unwrap().nodes.clone()
    }

    fn has(&self, s: &str) -> bool {
        self.log.iter().any(|l| l == s)
    }
}

impl NpcWorld for Fake {
    fn rules(&self) -> &crate::rules::Ruleset {
        &self.rules
    }
    fn item_format(&self) -> u16 {
        0x65
    }
    fn guid(&self, unit: UnitId) -> u32 {
        self.units[&unit].guid
    }
    fn monster_by_guid(&self, guid: u32) -> Option<UnitId> {
        self.units
            .iter()
            .find(|(_, u)| u.ty == 1 && u.guid == guid)
            .map(|(&id, _)| id)
    }
    fn unit_by_guid(&self, guid: u32) -> Option<(u8, UnitId)> {
        self.units
            .iter()
            .find(|(_, u)| u.guid == guid)
            .map(|(&id, u)| (u.ty, id))
    }
    fn monster_class(&self, unit: UnitId) -> Option<u16> {
        self.units.get(&unit).and_then(|u| u.class)
    }
    fn mode(&self, unit: UnitId) -> u8 {
        self.units[&unit].mode
    }
    fn set_mode(&mut self, unit: UnitId, mode: u8) {
        self.units.get_mut(&unit).unwrap().mode = mode;
        self.log.push(format!("mode {mode}"));
    }
    fn clear_unit_flag(&mut self, unit: UnitId, flag: u32) {
        self.units.get_mut(&unit).unwrap().flags &= !flag;
        self.log.push(format!("clear flag {flag:#x}"));
    }
    fn distance(&self, _: UnitId, _: UnitId) -> i32 {
        self.dist
    }
    fn axis_check(&self, _: UnitId, _: UnitId) -> u32 {
        self.axis
    }
    fn same_act(&self, _: UnitId, _: UnitId) -> bool {
        !self.other_act
    }
    fn unit_check(&self, _: UnitId, _: u32) -> u32 {
        self.unit_check
    }
    fn player_busy(&self, _: UnitId) -> u32 {
        self.busy
    }
    fn start_allowed(&self, _: UnitId, _: UnitId) -> bool {
        !self.refuse_start
    }
    fn tristram_cain_busy(&self, _: UnitId, _: UnitId) -> bool {
        self.cain_busy
    }
    fn clear_path(&mut self, unit: UnitId) {
        self.log.push(format!("path {}", unit.0));
    }
    fn npc_ai_param(&mut self, npc: UnitId, param: u32) {
        self.log.push(format!("ai {} {param:#x}", npc.0));
    }
    fn npc_ai_point(&mut self, npc: UnitId, x: u32, y: u32) {
        self.log.push(format!("aipt {} {x} {y}", npc.0));
    }
    fn reschedule_ai_think(&mut self, npc: UnitId) {
        self.log.push(format!("think {}", npc.0));
    }
    fn approach(&mut self, _: UnitId, npc: UnitId) {
        self.log.push(format!("approach {}", npc.0));
    }
    fn interaction(&mut self, npc: UnitId) -> Option<&mut InteractionList> {
        self.units.get_mut(&npc)?.interaction.as_mut()
    }
    fn interact_unit(&self, player: UnitId) -> Option<(u8, u32)> {
        self.interact.get(&player).copied()
    }
    fn set_interact(&mut self, player: UnitId, unit_type: u8, guid: u32) {
        self.interact.insert(player, (unit_type, guid));
    }
    fn reset_interact(&mut self, player: UnitId) {
        self.interact.remove(&player);
        self.log.push("reset interact".into());
    }
    fn pet(&self, _: UnitId, kind: u8, arg: u8) -> Option<UnitId> {
        // The fake answers (7, 1) with the one hireling, and (7, 0) with
        // it only while its node is living.
        assert_eq!(kind, 7);
        if arg == 0 && !self.hireling_living {
            return None;
        }
        self.hireling
    }
    fn pets(&self, _: UnitId) -> Vec<UnitId> {
        self.pets.clone()
    }
    fn stat(&self, unit: UnitId, stat: u16) -> u32 {
        self.get(unit, stat)
    }
    fn base_stat(&self, unit: UnitId, stat: u16) -> u32 {
        self.get(unit, stat)
    }
    fn set_stat(&mut self, unit: UnitId, stat: u16, value: u32) {
        self.set(unit, stat, value);
        self.log.push(format!("set {} {stat} {value}", unit.0));
    }
    fn set_stat_send(&mut self, player: UnitId, stat: u16, value: u32) {
        self.set(player, stat, value);
        self.log.push(format!("send stat {stat} {value}"));
    }
    fn max_life(&self, unit: UnitId) -> u32 {
        self.units[&unit].max[0]
    }
    fn max_mana(&self, unit: UnitId) -> u32 {
        self.units[&unit].max[1]
    }
    fn max_stamina(&self, unit: UnitId) -> u32 {
        self.units[&unit].max[2]
    }
    fn states_count(&self) -> u16 {
        self.states_count
    }
    fn has_state(&self, unit: UnitId, state: u16) -> bool {
        self.units[&unit].states.contains(&state)
    }
    fn curable(&self, state: u16) -> bool {
        self.curable.contains(&state)
    }
    fn has_state_list(&self, unit: UnitId, state: u16) -> bool {
        self.units[&unit].lists.contains(&state)
    }
    fn remove_state_list(&mut self, unit: UnitId, state: u16) {
        self.units.get_mut(&unit).unwrap().lists.remove(&state);
        self.log.push(format!("remove list {} {state}", unit.0));
    }
    fn attach_sound(&mut self, unit: UnitId, sound: u16) {
        self.log.push(format!("sound {} {sound}", unit.0));
    }
    fn send(&mut self, _: UnitId, msg: &[u8]) {
        self.sent.push(msg.to_vec());
    }
    fn quest_flags(&self, _: UnitId) -> QuestFlags {
        self.flags
    }
    fn quest_text_list(&mut self, _: UnitId, _: UnitId) -> TextList {
        vec![(0x25, 0)]
    }
    fn encode_text_list(&self, list: &TextList) -> [u8; 34] {
        // Fake encoding: u32 count, then u16 string ids.
        let mut b = [0u8; 34];
        b[0..4].copy_from_slice(&(list.len() as u32).to_le_bytes());
        for (i, (s, _)) in list.iter().enumerate() {
            b[4 + 2 * i..6 + 2 * i].copy_from_slice(&s.to_le_bytes());
        }
        b
    }
    fn send_game_quests(&mut self, _: UnitId) {
        self.sent.push(vec![0x29]);
    }
    fn send_player_quests(&mut self, _: UnitId, unit_type: u8, guid: u32) {
        let mut m = vec![0x28, unit_type];
        m.extend_from_slice(&guid.to_le_bytes());
        self.sent.push(m);
    }
    fn quest_chat_end(&mut self, _: UnitId, npc: UnitId) {
        self.log.push(format!("chat end {}", npc.0));
    }
    fn respec_offer(&mut self, _: UnitId) {
        self.flags.set(41, 13);
        self.flags.set(41, 1);
        self.log.push("respec offer".into());
    }
    fn respec_done(&mut self, _: UnitId) {
        self.flags.set(41, 0);
        self.flags.clear(41, 1);
        self.log.push("respec done".into());
    }
    fn imbue_granted(&mut self, _: UnitId) {
        self.log.push("imbue granted".into());
    }
    fn socket_granted(&mut self, _: UnitId) {
        self.log.push("socket granted".into());
    }
    fn personalize_granted(&mut self, _: UnitId) {
        self.log.push("personalize granted".into());
    }
    fn act_completion(&mut self, _: UnitId, _: UnitId, level: u32, from: u32) {
        self.log.push(format!("act completion {level} {from}"));
    }
    fn reset_stats(&mut self, _: UnitId) {
        self.log.push("reset stats".into());
    }
    fn reset_skills(&mut self, _: UnitId) {
        self.log.push("reset skills".into());
    }
    fn respec_sound(&mut self, _: UnitId) {
        self.log.push("respec sound".into());
    }
    fn player_name(&self, _: UnitId) -> Vec<u8> {
        b"Hero".to_vec()
    }
    fn act_change(&mut self, _: UnitId, level: u32, arg: u32) {
        self.log.push(format!("act change {level} {arg}"));
    }
    fn activate_waypoint(&mut self, _: UnitId, level: u32) {
        self.log.push(format!("waypoint {level}"));
    }
    fn inventory(&self, _: UnitId) -> Vec<InvEntry> {
        self.inv.clone()
    }
    fn identify(&mut self, item: UnitId) {
        self.log.push(format!("identify {}", item.0));
    }
    fn cursor_item(&self, _: UnitId) -> Option<UnitId> {
        self.cursor
    }
    fn item_facts(&self, item: UnitId) -> ItemFacts {
        self.facts[&item]
    }
    fn put_back(&mut self, _: UnitId, item: UnitId) {
        self.log.push(format!("put back {}", item.0));
    }
    fn remove_cursor_item(&mut self, _: UnitId, item: UnitId) -> bool {
        self.log.push(format!("remove {}", item.0));
        !self.no_remove
    }
    fn duplicate(&mut self, _: UnitId, item: UnitId) -> Option<UnitId> {
        if self.no_dup {
            return None;
        }
        let f = self.facts[&item];
        let d = self.item(f);
        self.log.push(format!("dup {} -> {}", item.0, d.0));
        Some(d)
    }
    fn create_imbued(&mut self, _: UnitId, input: UnitId, mods: &ImbueMods) -> Option<UnitId> {
        self.log.push(format!(
            "create {} flags {:#x} format {:#x} quality {} level {}",
            input.0, mods.flags, mods.format, mods.quality, mods.level
        ));
        if self.no_create {
            return None;
        }
        Some(self.item(ItemFacts::default()))
    }
    fn item_refresh(&mut self, item: UnitId) {
        self.log.push(format!("refresh {}", item.0));
    }
    fn set_item_page(&mut self, item: UnitId, page: u8) {
        self.log.push(format!("page {} {page}", item.0));
    }
    fn set_item_flag(&mut self, item: UnitId, flag: u32) {
        self.log.push(format!("flag {} {flag:#x}", item.0));
    }
    fn personal_name(&self, _: UnitId) -> Vec<u8> {
        b"Old".to_vec()
    }
    fn set_personal_name(&mut self, item: UnitId, name: &[u8]) {
        self.log
            .push(format!("name {} {}", item.0, String::from_utf8_lossy(name)));
    }
    fn place_or_drop(&mut self, _: UnitId, item: UnitId) {
        self.log.push(format!("place {}", item.0));
    }
    fn max_sockets(&self, item: UnitId) -> u32 {
        self.facts[&item].max_sockets
    }
    fn add_sockets(&mut self, item: UnitId, n: u32) {
        self.log.push(format!("sockets {} {n}", item.0));
    }
    fn item_seed(&mut self, item: UnitId) -> &mut Seed {
        self.seeds.entry(item).or_insert(Seed::init_low(12345))
    }
    fn spawn_mercenary_spread(
        &mut self,
        near: UnitId,
        class: u32,
        mode: u8,
        spread: i32,
    ) -> Option<UnitId> {
        self.log
            .push(format!("spawn {} {class} {mode} spread {spread}", near.0));
        self.spawn_unit()
    }
    fn spawn_mercenary(&mut self, near: UnitId, class: u32, mode: u8) -> Option<UnitId> {
        self.log.push(format!("spawn {} {class} {mode}", near.0));
        self.spawn_unit()
    }
    fn init_mercenary(&mut self, _: UnitId, merc: UnitId, init: &MercInit) {
        self.log.push(format!(
            "init {} row {} name {} seed {}",
            merc.0, init.row, init.name, init.seed
        ));
    }
    fn revive_mercenary(&mut self, _: UnitId, merc: UnitId) {
        self.log.push(format!("revive {}", merc.0));
    }
}

impl Fake {
    /// The fake's unit creation behind both spawn calls.
    fn spawn_unit(&mut self) -> Option<UnitId> {
        if self.spawn_fails > 0 {
            self.spawn_fails -= 1;
            return None;
        }
        self.next += 1;
        let id = UnitId(self.next);
        self.units.insert(
            id,
            Unit {
                guid: self.next,
                ty: 1,
                ..Unit::default()
            },
        );
        Some(id)
    }
}

impl NpcVendors for Fake {
    fn open_trade(
        &mut self,
        ctl: &mut NpcControl,
        _: UnitId,
        npc: UnitId,
        single: bool,
        gamble: bool,
    ) -> Result<(), NpcError> {
        self.log
            .push(format!("trade {} single {single} gamble {gamble}", npc.0));
        let class = self.units[&npc].class.unwrap();
        if SELLERS.contains(&class) {
            ctl.make_hire_list(class)?;
        }
        Ok(())
    }
    fn drop_gamble_list(&mut self, _: UnitId, npc: UnitId) {
        self.log.push(format!("drop gamble {}", npc.0));
    }
    fn pay(&mut self, player: UnitId, cost: u32) -> bool {
        let gold = self.get(player, stat::GOLD);
        if gold < cost {
            return false;
        }
        self.set(player, stat::GOLD, gold - cost);
        self.log.push(format!("pay {cost}"));
        true
    }
    fn repair(&mut self, item: UnitId) {
        self.log.push(format!("repair {}", item.0));
    }
}

// ------------------------------------------------------------ data

fn monstats(interact: &[u16]) -> Vec<Monstats> {
    let blank = Monstats::decode(&[0u8; Monstats::SIZE]);
    (0..560u16)
        .map(|c| {
            let mut m = blank.clone();
            if interact.contains(&c) {
                m.npc = true;
                m.interact = true;
            }
            m
        })
        .collect()
}

/// Every NPC of `vendors.tsv` plus two unlisted interact classes.
fn all_npcs() -> Vec<u16> {
    let mut v: Vec<u16> = parse_npc_table(VENDORS_TSV)
        .unwrap()
        .iter()
        .map(|r| r.class)
        .collect();
    v.extend([527, 537]);
    v
}

fn row(
    seller: u16,
    act: u32,
    difficulty: u32,
    level: u32,
    gold: u32,
    names: (u16, u16),
) -> HireRow {
    HireRow {
        version: 100,
        class: 271,
        act,
        difficulty,
        seller: u32::from(seller),
        gold,
        level,
        name_first: names.0,
        name_last: names.1,
    }
}

fn hirelings() -> Vec<HireRow> {
    vec![
        row(class::KASHYA, 1, 1, 3, 100, (1000, 1040)),
        row(class::KASHYA, 1, 1, 3, 120, (1000, 1040)),
        row(class::KASHYA, 1, 2, 30, 1000, (1000, 1040)),
        row(class::GREIZ, 2, 1, 9, 200, (2000, 2002)),
        row(class::ASHEARA, 3, 1, 15, 300, (3000, 3002)),
        row(class::QUAL_KEHK, 5, 1, 25, 400, (5000, 5002)),
    ]
}

fn control(difficulty: u8) -> NpcControl {
    let mut game = Seed::init_low(7);
    let mut c = NpcControl::new(
        &monstats(&all_npcs()),
        hirelings(),
        true,
        difficulty,
        &mut game,
    )
    .unwrap();
    c.seed = Seed::new(12345, 666);
    c
}

fn msg13(guid: u32) -> Vec<u8> {
    let mut m = vec![0x13];
    m.extend_from_slice(&1u32.to_le_bytes());
    m.extend_from_slice(&guid.to_le_bytes());
    m
}

fn msg9(id: u8, guid: u32) -> Vec<u8> {
    let mut m = vec![id, 1, 0, 0, 0];
    m.extend_from_slice(&guid.to_le_bytes());
    m
}

fn msg38(action: u32, guid: u32, item: u32) -> Vec<u8> {
    let mut m = vec![0x38];
    for v in [action, guid, item] {
        m.extend_from_slice(&v.to_le_bytes());
    }
    m
}

fn msg5(id: u8, guid: u32) -> Vec<u8> {
    let mut m = vec![id];
    m.extend_from_slice(&guid.to_le_bytes());
    m
}

fn msg36(guid: u32, name: u16) -> Vec<u8> {
    let mut m = vec![0x36];
    m.extend_from_slice(&guid.to_le_bytes());
    m.extend_from_slice(&name.to_le_bytes());
    m.extend_from_slice(&[0, 0]);
    m
}

/// Talk (0x13) and chat (0x2F) with an NPC; clears the sent messages.
fn talk(c: &mut NpcControl, w: &mut Fake, npc: UnitId) {
    let g = w.guid(npc);
    // A chat elsewhere is left without its 0x30.
    w.interact.remove(&PLAYER);
    assert_eq!(c.interact(w, PLAYER, &msg13(g)).unwrap(), Some(0));
    assert_eq!(c.chat_open(w, PLAYER, &msg9(0x2F, g)), 0);
    assert_eq!(w.list(npc)[0], (PLAYER, talk::CHATTING));
    w.sent.clear();
    w.log.clear();
}

// ------------------------------------------------------------ records

#[test]
fn vendors_tsv_matches_class_lists() {
    let t = parse_npc_table(VENDORS_TSV).unwrap();
    assert_eq!(t.len(), 43);
    let pick = |f: fn(&NpcTableRow) -> bool| -> Vec<u16> {
        let mut v: Vec<u16> = t.iter().filter(|r| f(r)).map(|r| r.class).collect();
        v.sort_unstable();
        v
    };
    let sorted = |s: &[u16]| {
        let mut v = s.to_vec();
        v.sort_unstable();
        v
    };
    assert_eq!(pick(|r| r.trade_action), sorted(&TRADERS));
    assert_eq!(pick(|r| r.gamble_action), sorted(&GAMBLERS));
    assert_eq!(pick(|r| r.heals), sorted(&HEALERS));
    assert_eq!(pick(|r| r.identifies), sorted(&IDENTIFIERS));
    assert_eq!(pick(|r| r.hire_list), sorted(&SELLERS));
    assert_eq!(pick(|r| r.resurrects), sorted(&RESURRECTORS));
    let cain = |c: u16| t.iter().find(|r| r.class == c).unwrap().act;
    assert_eq!(
        [
            cain(265),
            cain(244),
            cain(245),
            cain(246),
            cain(520),
            cain(146)
        ],
        [0, 1, 2, 3, 4, 0]
    );
    assert_eq!(t.iter().find(|r| r.class == 146).unwrap().trader, 0);
}

#[test]
fn vendors_tsv_check_catches_perturbations() {
    // A changed role cell no longer matches the code's list.
    let changed = VENDORS_TSV.replacen(
        "150\tkashya\t0\t0\t0\t-\t0\t0\t0\t0\t0\t0\t1\t1",
        "150\tkashya\t0\t0\t0\t-\t0\t0\t0\t0\t1\t0\t1\t1",
        1,
    );
    assert_ne!(changed, VENDORS_TSV);
    let t = parse_npc_table(&changed).unwrap();
    let healers: Vec<u16> = t.iter().filter(|r| r.heals).map(|r| r.class).collect();
    assert!(healers.contains(&class::KASHYA) && !HEALERS.contains(&class::KASHYA));
    // Strict parsing: an out-of-range act and a short row are errors.
    let bad_act = VENDORS_TSV.replacen("146\tcain1\t0", "146\tcain1\t5", 1);
    assert!(matches!(
        parse_npc_table(&bad_act),
        Err(TsvError::Value { .. })
    ));
    let short = VENDORS_TSV.replacen("\t0\n", "\n", 1);
    assert!(matches!(
        parse_npc_table(&short),
        Err(TsvError::Columns { .. })
    ));
    let header = VENDORS_TSV.replacen("npc", "class", 1);
    assert!(matches!(
        parse_npc_table(&header),
        Err(TsvError::Header { .. })
    ));
}

#[test]
fn records_follow_monstats_order_and_table() {
    let mut game = Seed::init_low(7);
    let mut want = game;
    let c = NpcControl::new(&monstats(&all_npcs()), vec![], true, 0, &mut game).unwrap();
    // One game-seed step; seed = init_low(lo').
    assert_eq!(c.seed, Seed::init_low(want.step()));
    assert_eq!(game, want);
    assert_eq!(c.records.len(), 45);
    let classes: Vec<u16> = c.records.iter().map(|r| r.class).collect();
    let mut sorted = classes.clone();
    sorted.sort_unstable();
    assert_eq!(classes, sorted, "row order");
    let charsi = c.record(class::CHARSI).unwrap();
    assert_eq!((charsi.act, charsi.trader, charsi.byte6), (0, 1, 0));
    let malah = c.record(class::MALAH).unwrap();
    assert_eq!((malah.act, malah.trader, malah.byte6), (4, 1, 1));
    let cain1 = c.record(class::CAIN1).unwrap();
    assert_eq!((cain1.act, cain1.trader), (0, 0));
    // Unlisted classes keep act 0, trader 0.
    for k in [527, 537] {
        let r = c.record(k).unwrap();
        assert_eq!((r.act, r.trader, r.byte6), (0, 0, 0));
    }
    assert!(c.record(1).is_none());
    assert!(c.interacts(class::CHARSI) && c.is_npc(class::CHARSI) && !c.interacts(1));
    assert_eq!(c.last_chat_npc, u32::MAX);
    // More than 64 interact rows: fatal.
    let many: Vec<u16> = (0..65).collect();
    let mut g = Seed::init_low(7);
    assert_eq!(
        NpcControl::new(&monstats(&many), vec![], true, 0, &mut g).unwrap_err(),
        NpcError::TooManyRecords
    );
    let exactly: Vec<u16> = (0..64).collect();
    assert!(NpcControl::new(&monstats(&exactly), vec![], true, 0, &mut g).is_ok());
}

#[test]
fn hireling_rows_read_the_fixed_up_name_ids() {
    let mut rec = vec![0u8; 280];
    rec[0..2].copy_from_slice(&100u16.to_le_bytes());
    rec[0x08..0x0C].copy_from_slice(&271u32.to_le_bytes());
    rec[0x0C..0x10].copy_from_slice(&1u32.to_le_bytes());
    rec[0x10..0x14].copy_from_slice(&1u32.to_le_bytes());
    rec[0x14..0x18].copy_from_slice(&150u32.to_le_bytes());
    rec[0x18..0x1C].copy_from_slice(&100u32.to_le_bytes());
    rec[0x1C..0x20].copy_from_slice(&3u32.to_le_bytes());
    rec[0x114..0x116].copy_from_slice(&1000u16.to_le_bytes());
    rec[0x116..0x118].copy_from_slice(&1040u16.to_le_bytes());
    let t = BinTable {
        name: "hireling".into(),
        source: "test".into(),
        count: 1,
        record_size: 280,
        records: rec,
    };
    let rows = HireRow::from_table(&t).unwrap();
    assert_eq!(rows, vec![row(150, 1, 1, 3, 100, (1000, 1040))]);
    let wrong = BinTable {
        name: "monstats".into(),
        ..t
    };
    assert!(HireRow::from_table(&wrong).is_err());
}

// ------------------------------------------------------------ hire list

#[test]
fn hire_list_vector_41() {
    let mut c = control(0);
    c.make_hire_list(class::KASHYA).unwrap();
    let r = c.record(class::KASHYA).unwrap();
    assert!(r.hire_made);
    let slots = &r.hire.as_ref().unwrap().slots;
    assert_eq!(slots.len(), 41);
    assert_eq!(
        [slots[0].seed, slots[1].seed, slots[2].seed],
        [22_752_887, 2_337_785_264, 1_617_882_871]
    );
    assert!(slots
        .iter()
        .enumerate()
        .all(|(i, s)| s.name == 1000 + i as u16 && !s.hired));
    let offered: Vec<usize> = (0..41).filter(|&i| slots[i].offered).collect();
    let mut want = vec![24, 29, 19, 10, 8, 11, 27, 5, 22, 28];
    want.sort_unstable();
    assert_eq!(offered, want);
    assert_eq!(c.seed, Seed::new(1_296_536_796, 747_986_489));
    // Made once: no more draws.
    let s = c.seed;
    c.make_hire_list(class::KASHYA).unwrap();
    assert_eq!(c.seed, s);
}

#[test]
fn hire_list_vector_3_stops_on_wrap() {
    let mut c = control(0);
    c.make_hire_list(class::GREIZ).unwrap();
    let slots = &c.record(class::GREIZ).unwrap().hire.as_ref().unwrap().slots;
    assert_eq!(
        slots.iter().map(|s| s.seed).collect::<Vec<_>>(),
        [22_752_887, 2_337_785_264, 1_617_882_871]
    );
    assert!(slots.iter().all(|s| s.offered));
    assert_eq!(c.seed, Seed::new(4_247_383_538, 1_001_282_318));
}

#[test]
fn hire_list_uses_the_normal_row_in_every_difficulty() {
    let mut c = control(2);
    c.make_hire_list(class::KASHYA).unwrap();
    assert_eq!(
        c.record(class::KASHYA)
            .unwrap()
            .hire
            .as_ref()
            .unwrap()
            .slots
            .len(),
        41
    );
    // No row for a seller: fatal.
    let mut c = control(0);
    c.hirelings.retain(|r| r.seller != u32::from(class::GREIZ));
    assert_eq!(
        c.make_hire_list(class::GREIZ).unwrap_err(),
        NpcError::NoHirelingRow {
            seller: class::GREIZ,
            difficulty: 1
        }
    );
    // Classic games read version 0 rows.
    let mut c = control(0);
    c.expansion = false;
    assert!(c.make_hire_list(class::KASHYA).is_err());
    // A name range wider than 69 slots.
    let mut c = control(0);
    c.hirelings[0].name_last = 1069;
    assert!(matches!(
        c.make_hire_list(class::KASHYA),
        Err(NpcError::HireListSize { .. })
    ));
}

// ------------------------------------------------------------ hire and resurrect

#[test]
fn hire_init_and_price_vectors() {
    let rows = [
        row(150, 1, 1, 3, 100, (1000, 1040)),
        row(150, 1, 1, 3, 120, (1000, 1040)),
        row(150, 1, 1, 4, 999, (1000, 1040)),
    ];
    let o = hire_init(&rows, 100, 22_752_887, 0, 0, 10).unwrap();
    assert_eq!((o.row, o.level), (1, 6));
    assert_eq!(o.price, price(&rows[1], 6));
    let r = row(150, 1, 1, 3, 100, (0, 0));
    assert_eq!([price(&r, 5), price(&r, 7), price(&r, 9)], [130, 160, 190]);
    // Below the row level: at least the row's gold.
    assert_eq!(price(&r, 2), 100);
    // L at least 2.
    let o = hire_init(&rows, 100, 22_752_887, 0, 0, 1).unwrap();
    assert_eq!(o.level, 2);
    // No rows for the act / difficulty / version.
    assert!(hire_init(&rows, 0, 1, 0, 0, 10).is_none());
    assert!(hire_init(&rows, 100, 1, 1, 0, 10).is_none());
    // Only rows with the first row's level are candidates (row 2 never).
    for seed in 0..64 {
        assert_ne!(hire_init(&rows, 100, seed, 0, 0, 10).unwrap().row, 2);
    }
}

#[test]
fn resurrect_cost_and_level_cap_vectors() {
    assert_eq!(
        [resurrect_cost(10), resurrect_cost(30), resurrect_cost(82)],
        [750, 6750, 50_000]
    );
    assert_eq!(resurrect_cost(0), 0);
    assert_eq!(capped_level(40, 0, 0), 12);
    assert_eq!(
        (0..5).map(|a| capped_level(99, 0, a)).collect::<Vec<_>>(),
        [12, 20, 28, 36, 45]
    );
    assert_eq!(capped_level(40, 1, 0), 40);
    assert_eq!(capped_level(5, 0, 4), 5);
}

// ------------------------------------------------------------ talk

#[test]
fn talk_to_charsi_sends_27_29_28() {
    let mut c = control(0);
    let mut w = Fake::new();
    let charsi = w.npc(class::CHARSI, 6);
    let m = msg13(6);
    assert_eq!(m, hex("13 01000000 06000000"));
    assert_eq!(c.interact(&mut w, PLAYER, &m).unwrap(), Some(0));
    assert_eq!(w.ids(), [0x27, 0x29, 0x28]);
    assert_eq!(w.sent[0].len(), 40);
    assert_eq!(w.sent[0][..12], hex("27 01 06000000 01000000 25 00")[..]);
    assert_eq!(w.sent[2], hex("28 01 06000000"));
    assert_eq!(w.list(charsi), [(PLAYER, talk::TALKING)]);
    assert_eq!(w.interact_unit(PLAYER), Some((1, 6)));
    // NPC halted (npc + interact), then the player's path cleared.
    assert_eq!(
        w.log,
        ["path 1006", "ai 1006 0x28", "think 1006", "path 1"].map(String::from)
    );
}

#[test]
fn talk_distances() {
    let mut c = control(0);
    let mut w = Fake::new();
    let charsi = w.npc(class::CHARSI, 6);
    w.dist = 51;
    assert_eq!(c.interact(&mut w, PLAYER, &msg13(6)).unwrap(), Some(1));
    assert!(w.log.is_empty() && w.sent.is_empty());
    assert_eq!(c.interact(&mut w, PLAYER, &msg13(99)).unwrap(), Some(1));
    for (d, approach) in [(50, false), (9, false), (8, true), (7, true)] {
        w.dist = d;
        w.log.clear();
        assert_eq!(c.interact(&mut w, PLAYER, &msg13(6)).unwrap(), Some(0));
        assert!(w.has("think 1006"), "halted at {d}");
        assert_eq!(w.has("approach 1006"), approach, "{d}");
        assert!(w.sent.is_empty() && w.list(charsi).is_empty());
    }
    // A monster without npc + interact is not halted.
    let other = w.npc(1, 30);
    w.dist = 3;
    w.log.clear();
    c.interact(&mut w, PLAYER, &msg13(30)).unwrap();
    assert!(!w.has("think 1030"));
    let _ = other;
    // Size, type checks; other unit types are not NPCs.
    assert_eq!(c.interact(&mut w, PLAYER, &[0x13, 1]).unwrap(), Some(3));
    let mut m = msg13(6);
    m[1] = 6;
    assert_eq!(c.interact(&mut w, PLAYER, &m).unwrap(), Some(2));
    m[1] = 2;
    assert_eq!(c.interact(&mut w, PLAYER, &m).unwrap(), None);
}

#[test]
fn start_requirements() {
    let mut c = control(0);
    let mut w = Fake::new();
    let charsi = w.npc(class::CHARSI, 6);
    // Busy player: no start.
    w.busy = 2;
    assert_eq!(c.interact(&mut w, PLAYER, &msg13(6)).unwrap(), Some(0));
    assert!(w.sent.is_empty());
    w.busy = 0;
    // The start check fails → 1.
    w.refuse_start = true;
    assert_eq!(c.interact(&mut w, PLAYER, &msg13(6)).unwrap(), Some(1));
    w.refuse_start = false;
    // Dead NPC.
    for mode in [0, 12] {
        w.units.get_mut(&charsi).unwrap().mode = mode;
        assert_eq!(c.interact(&mut w, PLAYER, &msg13(6)).unwrap(), Some(0));
        assert!(w.sent.is_empty());
    }
    w.units.get_mut(&charsi).unwrap().mode = 1;
    // Tristram Cain.
    w.npc(class::CAIN1, 7);
    w.cain_busy = true;
    assert_eq!(c.interact(&mut w, PLAYER, &msg13(7)).unwrap(), Some(0));
    assert!(w.sent.is_empty());
    // Already in the list → 1.
    c.interact(&mut w, PLAYER, &msg13(6)).unwrap();
    w.interact.clear();
    w.sent.clear();
    assert_eq!(c.interact(&mut w, PLAYER, &msg13(6)).unwrap(), Some(1));
    assert!(w.sent.is_empty());
    // A player with an interact unit cannot start.
    let p2 = UnitId(2);
    w.units.insert(
        p2,
        Unit {
            guid: 2,
            ..Unit::default()
        },
    );
    w.interact.insert(p2, (2, 55));
    assert_eq!(c.interact(&mut w, p2, &msg13(6)).unwrap(), Some(0));
    assert_eq!(w.list(charsi).len(), 1);
    // A second player is prepended.
    w.interact.clear();
    c.interact(&mut w, p2, &msg13(6)).unwrap();
    assert_eq!(w.list(charsi), [(p2, 0), (PLAYER, 0)]);
}

#[test]
fn talk_to_a_seller_sends_the_hire_list() {
    let mut c = control(0);
    let mut w = Fake::new();
    w.npc(class::GREIZ, 9);
    c.interact(&mut w, PLAYER, &msg13(9)).unwrap();
    assert_eq!(w.ids(), [0x4F, 0x4E, 0x4E, 0x4E, 0x27, 0x29, 0x28]);
    assert_eq!(w.sent[0], [0x4F]);
    assert_eq!(w.sent[1], hex("4e d007 772e5b01"));
    assert_eq!(&w.sent[1][3..], &22_752_887u32.to_le_bytes());
}

// ------------------------------------------------------------ chat

#[test]
fn chat_open_and_close() {
    let mut c = control(0);
    let mut w = Fake::new();
    let charsi = w.npc(class::CHARSI, 6);
    c.interact(&mut w, PLAYER, &msg13(6)).unwrap();
    w.sent.clear();
    let open = msg9(0x2F, 6);
    assert_eq!(open, hex("2f 01000000 06000000"));
    assert_eq!(c.chat_open(&mut w, PLAYER, &open), 0);
    assert!(w.sent.is_empty(), "Charsi is not a healer");
    assert_eq!(w.list(charsi), [(PLAYER, talk::CHATTING)]);
    assert_eq!(c.last_chat_npc, 6);
    // Bytes 1–4 are not read.
    let mut other = open.clone();
    other[1] = 0xEE;
    assert_eq!(c.chat_open(&mut w, PLAYER, &other), 0);
    // Checks.
    assert_eq!(c.chat_open(&mut w, PLAYER, &open[..8]), 3);
    assert_eq!(c.chat_open(&mut w, PLAYER, &msg9(0x2F, 77)), 1);
    assert_eq!(c.chat_open(&mut w, PLAYER, &msg9(0x2F, 1)), 3, "a player");
    w.other_act = true;
    assert_eq!(c.chat_open(&mut w, PLAYER, &open), 2);
    assert_eq!(c.chat_close(&mut w, PLAYER, &msg9(0x30, 6)), 2);
    w.other_act = false;
    w.axis = 1;
    assert_eq!(c.chat_open(&mut w, PLAYER, &open), 1);
    w.axis = 0;
    // Close (no distance test): unlink, reset, drop gamble list.
    w.axis = 1;
    w.log.clear();
    assert_eq!(c.chat_close(&mut w, PLAYER, &msg9(0x30, 6)), 0);
    assert!(w.list(charsi).is_empty());
    assert_eq!(
        w.log,
        ["chat end 1006", "reset interact", "drop gamble 1006"].map(String::from)
    );
    assert!(w.sent.is_empty());
}

#[test]
fn chat_close_states() {
    let mut c = control(0);
    let mut w = Fake::new();
    let akara = w.npc(class::AKARA, 0x10);
    // State 0: unlinked, gamble list kept.
    c.interact(&mut w, PLAYER, &msg13(0x10)).unwrap();
    w.log.clear();
    w.sent.clear();
    let close = msg9(0x30, 0x10);
    assert_eq!(close, hex("30 01000000 10000000"));
    assert_eq!(c.chat_close(&mut w, PLAYER, &close), 0);
    assert!(w.sent.is_empty(), "no message");
    assert_eq!(w.log, ["chat end 1016", "reset interact"].map(String::from));
    // State ≥ 1 with another player still in the list: kept too.
    let p2 = UnitId(2);
    w.units.insert(
        p2,
        Unit {
            guid: 2,
            ..Unit::default()
        },
    );
    w.interaction(akara).unwrap().nodes = vec![(p2, 0), (PLAYER, 2)];
    w.interact.insert(PLAYER, (1, 0x10));
    w.log.clear();
    c.chat_close(&mut w, PLAYER, &close);
    assert!(!w.has("drop gamble 1016"));
    assert_eq!(w.list(akara), [(p2, 0)]);
    // Not in the list: only the quest event; an object interact unit is kept.
    w.interact.insert(PLAYER, (2, 5));
    w.log.clear();
    c.chat_close(&mut w, PLAYER, &close);
    assert_eq!(w.log, ["chat end 1016"].map(String::from));
    assert_eq!(w.interact_unit(PLAYER), Some((2, 5)));
}

// ------------------------------------------------------------ menu actions

#[test]
fn menu_trade_and_gamble() {
    let mut c = control(0);
    let mut w = Fake::new();
    let charsi = w.npc(class::CHARSI, 6);
    talk(&mut c, &mut w, charsi);
    let m = msg38(1, 6, 0);
    assert_eq!(m, hex("38 01000000 06000000 00000000"));
    assert_eq!(c.menu_action(&mut w, PLAYER, &m).unwrap(), 0);
    assert!(w.sent.is_empty(), "no 0x2A");
    assert_eq!(
        w.log,
        ["trade 1006 single true gamble false"].map(String::from)
    );
    assert_eq!(w.list(charsi), [(PLAYER, talk::TRADING)]);
    // Charsi has no gamble action.
    w.log.clear();
    c.menu_action(&mut w, PLAYER, &msg38(2, 6, 0)).unwrap();
    assert!(w.log.is_empty());
    // Gamble at Gheed; a player not in the list still opens (edge 7).
    let gheed = w.npc(class::GHEED, 8);
    let p2 = UnitId(2);
    w.units.insert(
        p2,
        Unit {
            guid: 2,
            ..Unit::default()
        },
    );
    w.interaction(gheed).unwrap().nodes = vec![(p2, 1), (UnitId(3), 1)];
    c.menu_action(&mut w, PLAYER, &msg38(2, 8, 0)).unwrap();
    assert!(w.has("trade 1008 single false gamble true"));
    assert_eq!(w.list(gheed), [(p2, 1), (UnitId(3), 1)]);
    // Nihlathak: gamble, no trade (edge 9).
    let nih = w.npc(class::NIHLATHAK, 11);
    talk(&mut c, &mut w, nih);
    c.menu_action(&mut w, PLAYER, &msg38(1, 11, 0)).unwrap();
    assert!(w.log.is_empty());
    c.menu_action(&mut w, PLAYER, &msg38(2, 11, 0)).unwrap();
    assert_eq!(
        w.log,
        ["trade 1011 single true gamble true"].map(String::from)
    );
    // Checks: size, unit check, no interaction list.
    assert_eq!(c.menu_action(&mut w, PLAYER, &m[..12]).unwrap(), 3);
    w.unit_check = 2;
    assert_eq!(c.menu_action(&mut w, PLAYER, &m).unwrap(), 2);
    w.unit_check = 0;
    w.units.get_mut(&charsi).unwrap().interaction = None;
    w.log.clear();
    assert_eq!(c.menu_action(&mut w, PLAYER, &m).unwrap(), 0);
    assert!(w.log.is_empty());
}

#[test]
fn menu_hire_list() {
    let mut c = control(0);
    let mut w = Fake::new();
    let kashya = w.npc(class::KASHYA, 12);
    talk(&mut c, &mut w, kashya);
    c.menu_action(&mut w, PLAYER, &msg38(3, 12, 0)).unwrap();
    assert_eq!(w.ids()[0], 0x4F);
    assert_eq!(w.sent.len(), 11, "0x4F + 10 offers");
    // In slot order.
    let names: Vec<u16> = w.sent[1..]
        .iter()
        .map(|m| u16::from_le_bytes([m[1], m[2]]))
        .collect();
    assert_eq!(
        names,
        [1005, 1008, 1010, 1011, 1019, 1022, 1024, 1027, 1028, 1029]
    );
    // Any NPC accepts action 3; only sellers send.
    let charsi = w.npc(class::CHARSI, 6);
    talk(&mut c, &mut w, charsi);
    c.menu_action(&mut w, PLAYER, &msg38(3, 6, 0)).unwrap();
    assert!(w.sent.is_empty());
}

// ------------------------------------------------------------ healing

#[test]
fn healing_at_akara() {
    let mut c = control(0);
    let mut w = Fake::new();
    let akara = w.npc(class::AKARA, 0x10);
    w.set(PLAYER, stat::LIFE, 40);
    w.set(PLAYER, stat::STAMINA, 10);
    w.states_count = 8;
    w.curable = [3, 5].into();
    let p = w.units.get_mut(&PLAYER).unwrap();
    p.states = [3, 4, 5].into();
    p.lists = [1, 2, 3, 4].into();
    let pet = UnitId(50);
    w.units.insert(
        pet,
        Unit {
            guid: 50,
            max: [70, 0, 0],
            states: [5].into(),
            lists: [5, 2].into(),
            ..Unit::default()
        },
    );
    w.pets = vec![pet];
    c.interact(&mut w, PLAYER, &msg13(0x10)).unwrap();
    w.log.clear();
    w.sent.clear();
    c.chat_open(&mut w, PLAYER, &msg9(0x2F, 0x10));
    assert_eq!(
        w.log,
        [
            "send stat 6 100",
            "send stat 10 80",
            "remove list 1 2",
            "remove list 1 1",
            "remove list 1 3",
            "set 50 6 70",
            "remove list 50 5",
            "remove list 50 2",
            "sound 1016 10",
        ]
        .map(String::from)
    );
    // State 4 (not curable) and state 5 without a list are kept.
    assert_eq!(w.units[&PLAYER].lists, [4].into());
    assert_eq!(w.get(PLAYER, stat::MANA), 50);
    // A second 0x2F in the same interaction does not heal (edge 8).
    w.set(PLAYER, stat::LIFE, 1);
    w.log.clear();
    c.chat_open(&mut w, PLAYER, &msg9(0x2F, 0x10));
    assert!(w.log.is_empty());
    // Nothing to heal: no sound.
    c.chat_close(&mut w, PLAYER, &msg9(0x30, 0x10));
    w.set(PLAYER, stat::LIFE, 100);
    talk(&mut c, &mut w, akara);
    c.chat_close(&mut w, PLAYER, &msg9(0x30, 0x10));
    w.log.clear();
    c.interact(&mut w, PLAYER, &msg13(0x10)).unwrap();
    w.log.clear();
    c.chat_open(&mut w, PLAYER, &msg9(0x2F, 0x10));
    assert!(w.log.is_empty());
}

#[test]
fn heal_needs_the_interact_unit() {
    let mut c = control(0);
    let mut w = Fake::new();
    let atma = w.npc(class::ATMA, 0x20);
    w.set(PLAYER, stat::LIFE, 1);
    c.interact(&mut w, PLAYER, &msg13(0x20)).unwrap();
    w.interact.insert(PLAYER, (1, 0x99));
    w.log.clear();
    c.chat_open(&mut w, PLAYER, &msg9(0x2F, 0x20));
    assert!(w.log.is_empty());
    assert_eq!(c.last_chat_npc, 0x20);
    assert_eq!(w.list(atma), [(PLAYER, talk::CHATTING)]);
}

// ------------------------------------------------------------ identify

fn unid(w: &mut Fake, place: Place, flags: u32) -> UnitId {
    let i = w.item(ItemFacts::default());
    w.inv.push(InvEntry {
        item: i,
        place,
        flags,
    });
    i
}

#[test]
fn cain_identifies() {
    let mut c = control(0);
    let mut w = Fake::new();
    let cain = w.npc(class::CAIN5, 0x21);
    talk(&mut c, &mut w, cain);
    let a = unid(&mut w, Place::Grid(0), 0);
    unid(&mut w, Place::Grid(4), 0);
    let b = unid(&mut w, Place::Equipped, 0);
    unid(&mut w, Place::Belt, 0);
    unid(&mut w, Place::Grid(0), 0x10);
    let d = unid(&mut w, Place::Grid(3), 0);
    assert_eq!(c.identify(&mut w, PLAYER, &msg5(0x34, 0x21)), 0);
    assert_eq!(w.get(PLAYER, stat::GOLD), 200, "100 per item");
    // Each identify ends with sound event 6 on the player.
    let ids: Vec<String> = [a, b, d]
        .iter()
        .flat_map(|i| [format!("identify {}", i.0), format!("sound {} 6", PLAYER.0)])
        .collect();
    assert_eq!(w.log[1..], ids[..]);
    assert_eq!(w.sent, [transaction(0, 3, u32::MAX, 200).to_vec()]);
    // Quest slot 4 bit 0 or 1: free.
    for bit in [0, 1] {
        w.flags = QuestFlags::default();
        w.flags.set(4, bit);
        w.sent.clear();
        c.identify(&mut w, PLAYER, &msg5(0x34, 0x21));
        assert_eq!(w.get(PLAYER, stat::GOLD), 200);
        assert_eq!(w.sent[0][2], code::IDENTIFIED);
    }
    // Not enough gold → 12.
    w.flags = QuestFlags::default();
    w.set(PLAYER, stat::GOLD, 299);
    w.sent.clear();
    w.log.clear();
    c.identify(&mut w, PLAYER, &msg5(0x34, 0x21));
    assert_eq!(w.sent, [transaction(0, 12, u32::MAX, 299).to_vec()]);
    assert!(w.log.is_empty());
}

#[test]
fn identify_refusals() {
    let mut c = control(0);
    let mut w = Fake::new();
    let cain = w.npc(class::CAIN2, 0x21);
    // Not the interact unit → 9; missing NPC → 9.
    c.identify(&mut w, PLAYER, &msg5(0x34, 0x21));
    c.identify(&mut w, PLAYER, &msg5(0x34, 0x77));
    assert_eq!(w.sent.iter().map(|m| m[2]).collect::<Vec<_>>(), [9, 9]);
    // Nothing unidentified → 9.
    talk(&mut c, &mut w, cain);
    c.identify(&mut w, PLAYER, &msg5(0x34, 0x21));
    assert_eq!(w.sent, [transaction(0, 9, u32::MAX, 500).to_vec()]);
    // cain1 and other NPCs: no message at all.
    for (k, g) in [(class::CAIN1, 0x22), (class::CHARSI, 0x23)] {
        let n = w.npc(k, g);
        talk(&mut c, &mut w, n);
        unid(&mut w, Place::Grid(0), 0);
        c.identify(&mut w, PLAYER, &msg5(0x34, g));
        assert!(w.sent.is_empty());
        c.chat_close(&mut w, PLAYER, &msg9(0x30, g));
    }
    assert_eq!(c.identify(&mut w, PLAYER, &[0x34]), 3);
}

// ------------------------------------------------------------ hiring

#[test]
fn hire_greiz_and_refill() {
    let mut c = control(0);
    let mut w = Fake::new();
    let greiz = w.npc(class::GREIZ, 9);
    talk(&mut c, &mut w, greiz);
    w.set(PLAYER, stat::GOLD, 10_000);
    let slots = c.record(class::GREIZ).unwrap().hire.clone().unwrap().slots;
    // Hire slot 0 (name 2000).
    assert_eq!(c.hire(&mut w, PLAYER, &msg36(9, 2000)).unwrap(), 0);
    let o = hire_init(&c.hirelings, 100, slots[0].seed, 1, 0, 10).unwrap();
    assert_eq!(o.row, 3);
    assert_eq!(w.get(PLAYER, stat::GOLD), 10_000 - o.price);
    assert_eq!(w.ids(), [0x4F, 0x4E, 0x4E, 0x2A]);
    let merc = UnitId(101);
    assert_eq!(
        *w.sent.last().unwrap(),
        transaction(0, 5, 101, 10_000 - o.price)
    );
    assert!(w.has("spawn 1009 271 1"));
    assert!(w.has(&format!(
        "init {} row 3 name 2000 seed {}",
        merc.0, slots[0].seed
    )));
    let after = c.record(class::GREIZ).unwrap().hire.clone().unwrap();
    assert!(after.slots[0].hired);
    // Hire the other two: the offer empties and a new list is drawn.
    c.hire(&mut w, PLAYER, &msg36(9, 2001)).unwrap();
    let seed = c.seed;
    c.hire(&mut w, PLAYER, &msg36(9, 2002)).unwrap();
    let r = c.record(class::GREIZ).unwrap();
    assert!(r.hire_made);
    let fresh = &r.hire.as_ref().unwrap().slots;
    assert!(fresh.iter().all(|s| !s.hired && s.offered));
    let mut s = seed;
    assert_eq!(fresh[0].seed, s.step());
    // Hired slot → 9.
    w.sent.clear();
    let mut c2 = control(0);
    c2.make_hire_list(class::GREIZ).unwrap();
    c2.record_mut(class::GREIZ)
        .unwrap()
        .hire
        .as_mut()
        .unwrap()
        .slots[1]
        .hired = true;
    c2.hire(&mut w, PLAYER, &msg36(9, 2001)).unwrap();
    assert_eq!(w.sent.last().unwrap()[2], code::REFUSED);
}

#[test]
fn hire_refusals() {
    let mut c = control(0);
    let mut w = Fake::new();
    let kashya = w.npc(class::KASHYA, 12);
    let last = |w: &Fake| w.sent.last().unwrap()[2];
    // Not the interact unit, missing NPC, bad size.
    c.hire(&mut w, PLAYER, &msg36(12, 1005)).unwrap();
    assert_eq!(last(&w), 9);
    c.hire(&mut w, PLAYER, &msg36(77, 1005)).unwrap();
    assert_eq!(last(&w), 9);
    assert_eq!(c.hire(&mut w, PLAYER, &[0x36]).unwrap(), 3);
    talk(&mut c, &mut w, kashya);
    // Kashya below level 8 without slot 2 bit 0 → 11.
    w.set(PLAYER, stat::LEVEL, 7);
    c.hire(&mut w, PLAYER, &msg36(12, 1005)).unwrap();
    assert_eq!(last(&w), code::GATE);
    w.flags.set(2, 0);
    // Name outside the range → 9.
    c.hire(&mut w, PLAYER, &msg36(12, 1041)).unwrap();
    assert_eq!(last(&w), 9);
    // Not enough gold → 12.
    w.set(PLAYER, stat::GOLD, 0);
    c.hire(&mut w, PLAYER, &msg36(12, 1005)).unwrap();
    assert_eq!(last(&w), code::NO_GOLD);
    // Never-offered slot is accepted (edge 3); placement fails (edge 4):
    // code 15, gold kept by the NPC.
    w.set(PLAYER, stat::GOLD, 10_000);
    assert!(
        !c.record(class::KASHYA)
            .unwrap()
            .hire
            .as_ref()
            .unwrap()
            .slots[0]
            .offered
    );
    w.spawn_fails = 2;
    w.log.clear();
    c.hire(&mut w, PLAYER, &msg36(12, 1000)).unwrap();
    assert_eq!(last(&w), code::NOT_PLACED);
    assert!(w.get(PLAYER, stat::GOLD) < 10_000);
    assert!(w.has("spawn 1012 271 1") && w.has("spawn 1 271 1"));
    // Second spawn near the player succeeds.
    w.spawn_fails = 1;
    c.hire(&mut w, PLAYER, &msg36(12, 1000)).unwrap();
    assert_eq!(last(&w), code::MERC);
    // Qual-Kehk without slot 36 bit 0 → 11 (at any level).
    let qk = w.npc(class::QUAL_KEHK, 13);
    c.chat_close(&mut w, PLAYER, &msg9(0x30, 12));
    talk(&mut c, &mut w, qk);
    w.set(PLAYER, stat::LEVEL, 50);
    c.hire(&mut w, PLAYER, &msg36(13, 5000)).unwrap();
    assert_eq!(last(&w), code::GATE);
    // No hire rows for the act / difficulty: no message.
    w.flags.set(36, 0);
    c.difficulty = 1;
    w.sent.clear();
    c.hire(&mut w, PLAYER, &msg36(13, 5000)).unwrap();
    assert!(w.sent.is_empty());
}

#[test]
fn hire_cap_feeds_only_the_kashya_gate() {
    let mut c = control(0);
    let mut w = Fake::new();
    let kashya = w.npc(class::KASHYA, 12);
    talk(&mut c, &mut w, kashya);
    w.set(PLAYER, stat::LEVEL, 30);
    w.set(PLAYER, stat::GOLD, 100_000);
    // Capped level 12 is not < 8: the gate passes without slot 2 bit 0.
    let seed = c.record(class::KASHYA).unwrap().hire.clone().unwrap().slots[5].seed;
    let uncapped = hire_init(&c.hirelings, 100, seed, 0, 0, 30).unwrap();
    let capped = hire_init(&c.hirelings, 100, seed, 0, 0, 12).unwrap();
    assert_ne!(uncapped.price, capped.price);
    assert_eq!(c.hire(&mut w, PLAYER, &msg36(12, 1005)).unwrap(), 0);
    assert_eq!(w.get(PLAYER, stat::GOLD), 100_000 - uncapped.price);
}

// ------------------------------------------------------------ resurrection

#[test]
fn resurrect_at_tyrael() {
    let mut c = control(0);
    let mut w = Fake::new();
    let tyrael = w.npc(class::TYRAEL2, 0x30);
    let merc = UnitId(60);
    w.units.insert(
        merc,
        Unit {
            guid: 60,
            ty: 1,
            max: [300, 0, 0],
            flags: 0x10000,
            ..Unit::default()
        },
    );
    w.set(merc, stat::LEVEL, 10);
    w.hireling = Some(merc);
    // Not the interact unit → 9.
    c.resurrect(&mut w, PLAYER, &msg5(0x62, 0x30));
    assert_eq!(w.sent.last().unwrap()[2], 9);
    talk(&mut c, &mut w, tyrael);
    // 500 gold < 750 → 12.
    assert_eq!(c.resurrect(&mut w, PLAYER, &msg5(0x62, 0x30)), 0);
    assert_eq!(w.sent, [transaction(0, 12, u32::MAX, 500).to_vec()]);
    w.set(PLAYER, stat::GOLD, 1000);
    w.sent.clear();
    c.resurrect(&mut w, PLAYER, &msg5(0x62, 0x30));
    assert_eq!(w.sent[0], hex("9b ffff 00000000"));
    assert_eq!(w.sent[1], transaction(0, 5, 60, 250).to_vec());
    assert_eq!(
        w.log[w.log.len() - 4..],
        ["clear flag 0x10000", "mode 1", "set 60 6 300", "revive 60"].map(String::from)
    );
    // No dead hireling → 9; wrong NPC → 9; classic or bad size → 3.
    w.hireling = None;
    c.resurrect(&mut w, PLAYER, &msg5(0x62, 0x30));
    assert_eq!(w.sent.last().unwrap()[2], 9);
    let charsi = w.npc(class::CHARSI, 6);
    w.hireling = Some(merc);
    c.chat_close(&mut w, PLAYER, &msg9(0x30, 0x30));
    talk(&mut c, &mut w, charsi);
    c.resurrect(&mut w, PLAYER, &msg5(0x62, 6));
    assert_eq!(w.sent.last().unwrap()[2], 9);
    assert_eq!(c.resurrect(&mut w, PLAYER, &[0x62, 6]), 3);
    c.expansion = false;
    assert_eq!(c.resurrect(&mut w, PLAYER, &msg5(0x62, 6)), 3);
}

#[test]
fn resurrect_revives_a_living_hireling() {
    // The dead bit is not tested (a hireling with life poked to 0 that never
    // died is revived): the cost is paid, 0x9B and 0x2A code 5 follow, as for
    // a dead one.
    let mut c = control(0);
    let mut w = Fake::new();
    let kashya = w.npc(class::KASHYA, 0x30);
    let merc = UnitId(60);
    w.units.insert(
        merc,
        Unit {
            guid: 60,
            ty: 1,
            max: [300, 0, 0],
            ..Unit::default()
        },
    );
    w.set(merc, stat::LEVEL, 10);
    w.set(PLAYER, stat::GOLD, 1000);
    w.hireling = Some(merc);
    w.hireling_living = true;
    talk(&mut c, &mut w, kashya);
    w.sent.clear();
    w.log.clear();
    assert_eq!(c.resurrect(&mut w, PLAYER, &msg5(0x62, 0x30)), 0);
    assert_eq!(w.sent[0], hex("9b ffff 00000000"));
    // The tail reads the GUID of the record the revive freed (−1).
    assert_eq!(w.sent[1], transaction(0, 5, u32::MAX, 250).to_vec());
    assert_eq!(w.get(PLAYER, stat::GOLD), 250);
}

// ------------------------------------------------------------ quest mercenary

#[test]
fn quest_mercenary() {
    let mut c = control(0);
    let mut w = Fake::new();
    // No hire list yet: nothing.
    c.quest_mercenary(&mut w, PLAYER, class::KASHYA).unwrap();
    assert!(w.sent.is_empty() && w.log.is_empty());
    c.make_hire_list(class::KASHYA).unwrap();
    w.spawn_fails = 2;
    c.quest_mercenary(&mut w, PLAYER, class::KASHYA).unwrap();
    let slots = &c
        .record(class::KASHYA)
        .unwrap()
        .hire
        .as_ref()
        .unwrap()
        .slots;
    assert!(slots[5].hired, "first offered slot");
    assert_eq!(w.sent, [hex("50 0200 ed03 00000000000000000000")]);
    assert_eq!(
        w.log,
        [
            // Mode 1, spreads 4, 6, 12.
            "spawn 1 271 1 spread 4".to_string(),
            "spawn 1 271 1 spread 6".into(),
            "spawn 1 271 1 spread 12".into(),
            format!("init 101 row 0 name 1005 seed {}", slots[5].seed),
        ]
    );
    // All spawns fail: stop (slot still hired, no refill check).
    w.log.clear();
    w.spawn_fails = 3;
    c.quest_mercenary(&mut w, PLAYER, class::KASHYA).unwrap();
    assert_eq!(w.log.len(), 3);
    // Expansion player with a hireling: only the refill check.
    w.hireling = Some(UnitId(60));
    w.hireling_living = true;
    w.sent.clear();
    c.quest_mercenary(&mut w, PLAYER, class::KASHYA).unwrap();
    assert!(w.sent.is_empty());
    // Classic with a hireling: nothing.
    c.expansion = false;
    c.quest_mercenary(&mut w, PLAYER, class::KASHYA).unwrap();
    assert!(w.sent.is_empty());
    // The row is the game difficulty's (Nightmare row 2, its class).
    let mut c = control(1);
    c.hirelings[2].class = 338;
    c.make_hire_list(class::KASHYA).unwrap();
    let mut w = Fake::new();
    c.quest_mercenary(&mut w, PLAYER, class::KASHYA).unwrap();
    assert!(w.has("spawn 1 338 1 spread 4"));
}

// ------------------------------------------------------------ item services

fn imbuable() -> ItemFacts {
    ItemFacts {
        bitfield1: 1,
        quality: 2,
        code: *b"axe ",
        max_sockets: 3,
        nameable: true,
        item_type: 28,
        ..ItemFacts::default()
    }
}

#[test]
fn service_predicates() {
    let ok = imbuable();
    assert!(can_imbue(&ok) && can_socket(&ok) && can_personalize(&ok));
    let with = |f: fn(&mut ItemFacts)| {
        let mut x = imbuable();
        f(&mut x);
        (can_imbue(&x), can_socket(&x), can_personalize(&x))
    };
    assert_eq!(with(|f| f.gold = true), (false, false, false));
    assert_eq!(with(|f| f.flags = 0x1000), (false, false, false));
    assert_eq!(with(|f| f.has_socketed = true), (false, false, false));
    assert_eq!(with(|f| f.bitfield1 = 0), (false, true, true));
    assert_eq!(with(|f| f.throwable = true), (false, true, true));
    assert_eq!(
        with(|f| {
            f.throwable = true;
            f.unit_flags = 1 << 25
        }),
        (true, true, true)
    );
    assert_eq!(with(|f| f.quest = true), (false, false, true));
    assert_eq!(
        with(|f| {
            f.quest = true;
            f.code = *b"leg "
        }),
        (true, true, true)
    );
    assert_eq!(with(|f| f.flags = 0x800), (false, false, true));
    assert_eq!(with(|f| f.quality = 4), (false, true, true));
    assert_eq!(with(|f| f.quality = 0), (false, true, true));
    assert_eq!(with(|f| f.flags = 0x100), (true, false, false));
    assert_eq!(with(|f| f.max_sockets = 0), (true, false, true));
    assert_eq!(with(|f| f.stat194 = 1), (true, false, true));
    for t in [5, 6, 7] {
        let mut x = imbuable();
        x.item_type = t;
        assert!(!can_personalize(&x));
    }
    assert_eq!(with(|f| f.flags = 0x0100_0000), (true, true, false));
    assert_eq!(with(|f| f.nameable = false), (true, true, false));
    assert_eq!(
        [
            imbue_level(0),
            imbue_level(5),
            imbue_level(6),
            imbue_level(30)
        ],
        [1, 5, 10, 34]
    );
}

#[test]
fn imbue_at_charsi() {
    let mut c = control(0);
    let mut w = Fake::new();
    let charsi = w.npc(class::CHARSI, 6);
    talk(&mut c, &mut w, charsi);
    let mut facts = imbuable();
    facts.ethereal = true;
    let item = w.item(facts);
    w.cursor = Some(item);
    let g = item.0;
    // Gate 3.1 clear → refuse, item put back.
    c.menu_action(&mut w, PLAYER, &msg38(0x94, 6, g)).unwrap();
    assert_eq!(w.sent, [service_result(6, 7).to_vec()]);
    assert_eq!(w.log, [format!("put back {g}")]);
    // Not the cursor item: silent.
    w.flags.set(3, 1);
    w.sent.clear();
    w.log.clear();
    c.menu_action(&mut w, PLAYER, &msg38(0x94, 6, g + 1))
        .unwrap();
    assert!(w.sent.is_empty() && w.log.is_empty());
    // Done.
    c.menu_action(&mut w, PLAYER, &msg38(0, 6, g)).unwrap();
    let new = g + 1;
    assert_eq!(
        w.log,
        [
            format!("remove {g}"),
            format!("create {g} flags 0x24 format 0x65 quality 6 level 14"),
            format!("repair {new}"),
            format!("refresh {new}"),
            format!("page {new} 0"),
            format!("name {new} Old"),
            format!("place {new}"),
            "imbue granted".into(),
        ]
    );
    assert_eq!(w.sent, [hex("58 06000000 06 00")]);
    // Creation fails: result 7, the input is lost (no put back).
    w.no_create = true;
    w.sent.clear();
    w.log.clear();
    c.menu_action(&mut w, PLAYER, &msg38(0, 6, g)).unwrap();
    assert_eq!(w.sent, [service_result(6, 7).to_vec()]);
    assert!(!w.log.iter().any(|l| l.starts_with("put back")));
    // Removal fails: refuse.
    w.no_remove = true;
    w.log.clear();
    c.menu_action(&mut w, PLAYER, &msg38(0, 6, g)).unwrap();
    assert_eq!(w.log, [format!("remove {g}"), format!("put back {g}")]);
}

#[test]
fn socket_at_larzuk() {
    let mut c = control(0);
    let mut w = Fake::new();
    let larzuk = w.npc(class::LARZUK, 0x40);
    talk(&mut c, &mut w, larzuk);
    w.flags.set(35, 1);
    for (quality, max, want) in [(2, 3, 3), (5, 4, 1), (7, 2, 1)] {
        let mut f = imbuable();
        f.quality = quality;
        f.max_sockets = max;
        let item = w.item(f);
        w.cursor = Some(item);
        w.log.clear();
        w.sent.clear();
        c.menu_action(&mut w, PLAYER, &msg38(0, 0x40, item.0))
            .unwrap();
        let dup = item.0 + 1;
        assert_eq!(
            w.log,
            [
                format!("dup {} -> {dup}", item.0),
                format!("remove {}", item.0),
                format!("flag {dup} 0x800"),
                format!("sockets {dup} {want}"),
                format!("repair {dup}"),
                format!("refresh {dup}"),
                format!("page {dup} 0"),
                format!("place {dup}"),
                "socket granted".into(),
            ]
        );
        assert_eq!(w.sent, [service_result(0x40, 6).to_vec()]);
    }
    // Quality 4: roll(item seed of the duplicate, min(s, 2)) + 1.
    let mut s = Seed::init_low(12345);
    assert_eq!(
        socket_count(&mut s, 4, 3),
        Seed::init_low(12345).roll(2) + 1
    );
    let mut s = Seed::init_low(12345);
    assert_eq!(socket_count(&mut s, 4, 1), 1);
    assert_eq!(s, {
        let mut t = Seed::init_low(12345);
        t.step();
        t
    });
    let mut s = Seed::init_low(1);
    assert_eq!(socket_count(&mut s, 2, 5), 5);
    assert_eq!(s, Seed::init_low(1), "no draw");
    assert_eq!(socket_count(&mut s, 6, 0), 0);
    // Duplicate fails: refuse before the removal.
    w.no_dup = true;
    w.log.clear();
    let item = w.cursor.unwrap();
    c.menu_action(&mut w, PLAYER, &msg38(0, 0x40, item.0))
        .unwrap();
    assert_eq!(w.log, [format!("put back {}", item.0)]);
}

#[test]
fn personalize_at_drehya() {
    let mut c = control(0);
    let mut w = Fake::new();
    let drehya = w.npc(class::DREHYA, 0x41);
    talk(&mut c, &mut w, drehya);
    w.flags.set(38, 1);
    let item = w.item(imbuable());
    w.cursor = Some(item);
    c.menu_action(&mut w, PLAYER, &msg38(0, 0x41, item.0))
        .unwrap();
    let dup = item.0 + 1;
    assert_eq!(
        w.log,
        [
            format!("dup {} -> {dup}", item.0),
            format!("remove {}", item.0),
            format!("repair {dup}"),
            format!("page {dup} 0"),
            format!("place {dup}"),
            format!("flag {dup} 0x1000000"),
            format!("name {dup} Hero"),
            "personalize granted".into(),
        ]
    );
    assert_eq!(w.sent, [service_result(0x41, 6).to_vec()]);
    // Failed duplicate: result 7, put back, stop (edge 6, D2RUST).
    w.no_dup = true;
    w.log.clear();
    w.sent.clear();
    c.menu_action(&mut w, PLAYER, &msg38(0, 0x41, item.0))
        .unwrap();
    assert_eq!(w.sent, [service_result(0x41, 7).to_vec()]);
    assert_eq!(w.log, [format!("put back {}", item.0)]);
}

// ------------------------------------------------------------ respec and act travel

#[test]
fn akara_respec() {
    let mut w = Fake::new();
    // Normal: nothing without 41.1.
    super::services::respec(&mut w, PLAYER, 0);
    assert!(w.log.is_empty());
    // Hell with 1.0: offered, then done.
    w.flags.set(1, 0);
    super::services::respec(&mut w, PLAYER, 0);
    assert!(w.log.is_empty(), "the offer is Hell only");
    super::services::respec(&mut w, PLAYER, 2);
    assert_eq!(
        w.log,
        [
            "respec offer",
            "reset skills",
            "reset stats",
            "respec sound",
            "respec done"
        ]
        .map(String::from)
    );
    // Already done (41.0): no new offer in Hell.
    w.log.clear();
    super::services::respec(&mut w, PLAYER, 2);
    assert!(w.log.is_empty());
    // Through 0x38 at Akara (any other action).
    let mut c = control(0);
    let akara = w.npc(class::AKARA, 0x10);
    talk(&mut c, &mut w, akara);
    w.flags.set(41, 1);
    c.menu_action(&mut w, PLAYER, &msg38(0x94, 0x10, 0))
        .unwrap();
    assert!(w.has("respec done"));
}

#[test]
fn act_travel() {
    let mut c = control(0);
    let mut w = Fake::new();
    let run = |c: &mut NpcControl, w: &mut Fake, class: u16, guid: u32, action: u32| {
        let n = w.npc(class, guid);
        talk(c, w, n);
        c.menu_action(w, PLAYER, &msg38(action, guid, 0)).unwrap();
        let log = std::mem::take(&mut w.log);
        c.chat_close(w, PLAYER, &msg9(0x30, guid));
        w.log.clear();
        w.sent.clear();
        log
    };
    // Warriv1 needs slot 6 bit 0.
    assert!(run(&mut c, &mut w, class::WARRIV1, 0x50, 0).is_empty());
    w.flags.set(6, 0);
    assert_eq!(
        run(&mut c, &mut w, class::WARRIV1, 0x51, 7),
        ["act change 40 0", "act completion 40 1", "waypoint 40"].map(String::from)
    );
    assert!(
        run(&mut c, &mut w, class::WARRIV1, 0x52, 3).is_empty(),
        "hire list action"
    );
    assert_eq!(
        run(&mut c, &mut w, class::WARRIV2, 0x53, 9),
        ["act change 1 5"]
    );
    // Meshif1: action 0 and slot 14 bit 0.
    assert!(run(&mut c, &mut w, class::MESHIF1, 0x54, 0).is_empty());
    w.flags.set(14, 0);
    assert!(run(&mut c, &mut w, class::MESHIF1, 0x55, 4).is_empty());
    assert_eq!(
        run(&mut c, &mut w, class::MESHIF1, 0x56, 0),
        ["act change 75 0", "act completion 75 40", "waypoint 75"].map(String::from)
    );
    assert_eq!(
        run(&mut c, &mut w, class::MESHIF2, 0x57, 0),
        ["act change 40 5"]
    );
    assert!(run(&mut c, &mut w, class::MESHIF2, 0x58, 4).is_empty());
    // Tyrael2: expansion and slot 26 bit 0.
    assert!(run(&mut c, &mut w, class::TYRAEL2, 0x59, 0).is_empty());
    w.flags.set(26, 0);
    assert_eq!(
        run(&mut c, &mut w, class::TYRAEL2, 0x5A, 0),
        ["act change 109 0", "act completion 109 103", "waypoint 109"].map(String::from)
    );
    c.expansion = false;
    assert!(run(&mut c, &mut w, class::TYRAEL2, 0x5B, 0).is_empty());
    assert_eq!(
        run(&mut c, &mut w, class::CAIN6, 0x5C, 0),
        ["act change 103 5"]
    );
    // Any other NPC / action pair: nothing.
    assert!(run(&mut c, &mut w, class::GHEED, 0x5D, 0).is_empty());
}

// ------------------------------------------------------------ messages

#[test]
fn message_bytes() {
    // Recorded sell (frame 1157) with bytes 3–6 masked.
    let rec = hex("2a 03 01 05a4f619 07000000 f4010000");
    let m = transaction(3, code::SOLD, 7, 500);
    let mask = |b: &[u8]| {
        let mut b = b.to_vec();
        b[3..7].fill(0);
        b
    };
    assert_eq!(mask(&rec), m);
    assert_eq!(m[3..7], [0, 0, 0, 0], "D2RUST writes 0");
    let rec = hex("2a 04 00 056cf619 36000000 bc010000");
    assert_eq!(mask(&rec), transaction(4, code::BOUGHT, 0x36, 0x1BC));
    assert_eq!(service_result(0x41, 6), [0x58, 0x41, 0, 0, 0, 6, 0]);
    assert_eq!(resurrect_message().to_vec(), hex("9b ffff 00000000"));
}

// ------------------------------------------------------------ game files

#[test]
#[ignore = "needs extracted game tables in D2_GAME_DIR"]
fn live_monstats_records() {
    use crate::skills::tests_game as game;
    use d2_data::tables::decode_all;
    let ms: Vec<Monstats> = decode_all(&game::table("monstats", Monstats::SIZE)).unwrap();
    let hire = HireRow::from_table(&game::table("hireling", 280)).unwrap();
    let mut seed = Seed::init_low(1);
    let c = NpcControl::new(&ms, hire, true, 0, &mut seed).unwrap();
    assert_eq!(c.records.len(), 47);
    let table = parse_npc_table(VENDORS_TSV).unwrap();
    for t in &table {
        let r = c
            .record(t.class)
            .unwrap_or_else(|| panic!("{} has a record", t.name));
        assert_eq!(
            (r.act, r.trader, r.byte6),
            (t.act, t.trader, t.byte6),
            "{}",
            t.name
        );
    }
    for s in SELLERS {
        assert!(c.seller_row(s, 1).is_some(), "Normal row of {s}");
    }
}

fn msg59(ty: u32, guid: u32, x: u32, y: u32) -> Vec<u8> {
    let mut m = vec![0x59];
    for v in [ty, guid, x, y] {
        m.extend_from_slice(&v.to_le_bytes());
    }
    m
}

#[test]
fn make_entity_move_sets_the_npc_ai_params() {
    let mut c = control(0);
    let mut w = Fake::new();
    w.npc(class::CHARSI, 6);
    w.dist = 50;
    assert_eq!(
        c.make_entity_move(&mut w, PLAYER, &msg59(1, 6, 4865, 4241)),
        0
    );
    assert_eq!(
        w.log,
        [
            "path 1006",
            "think 1006",
            "ai 1006 0x28",
            "aipt 1006 4865 4241"
        ]
        .map(String::from)
    );
    // 51 or farther, an unknown unit, a monster that is not npc + interact.
    w.dist = 51;
    w.log.clear();
    assert_eq!(c.make_entity_move(&mut w, PLAYER, &msg59(1, 6, 1, 2)), 1);
    assert_eq!(c.make_entity_move(&mut w, PLAYER, &msg59(1, 99, 1, 2)), 1);
    w.dist = 3;
    w.npc(1, 30);
    assert_eq!(c.make_entity_move(&mut w, PLAYER, &msg59(1, 30, 1, 2)), 1);
    assert!(w.log.is_empty());
    // Size 17 only; unit type above 5.
    assert_eq!(c.make_entity_move(&mut w, PLAYER, &[0x59, 1]), 3);
    assert_eq!(c.make_entity_move(&mut w, PLAYER, &msg59(6, 6, 1, 2)), 2);
}
