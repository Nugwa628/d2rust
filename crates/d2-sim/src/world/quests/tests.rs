use std::collections::BTreeMap;

use super::act1;
use super::tables::{parse_messages, parse_quests, MESSAGES_TSV, QUESTS_TSV};
use super::*;
use crate::units::RoomId;
use crate::world::TsvError;

// ------------------------------------------------------------ flags

pub(super) fn hex(s: &str) -> Vec<u8> {
    let s: String = s.split_whitespace().collect();
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

#[test]
fn flag_vectors() {
    let mut r = QuestFlags::default();
    r.set(1, 2);
    assert_eq!(r.0[2], 0x04);
    assert!(r.0.iter().enumerate().all(|(i, &b)| i == 2 || b == 0));
    r.set(1, 3);
    assert_eq!(r.0[2..4], [0x0c, 0x00]);
    assert!(r.get(1, 3) && !r.get(1, 4));
    r.set(1, 15);
    assert_eq!(r.word(1), 0x800c);
    r.clear(1, 15);
    r.set(1, 11);
    r.set(1, 12);
    r.reset_progress(1);
    assert_eq!(r.word(1), 0x1000); // bits 2..11 cleared, 12 kept
                                   // copy_in normalize.
    let mut buf = [0u8; 96];
    buf[10..12].copy_from_slice(&0x6002u16.to_le_bytes());
    buf[12..14].copy_from_slice(&0x4001u16.to_le_bytes());
    buf[82..84].copy_from_slice(&0x6002u16.to_le_bytes()); // slot 41
    let n = QuestFlags::copy_in(&buf, true).unwrap();
    assert_eq!((n.word(5), n.word(6), n.word(41)), (0x8002, 0x0001, 0x8002));
    let raw = QuestFlags::copy_in(&buf, false).unwrap();
    assert_eq!(raw.copy_out(), buf);
    assert_eq!(
        QuestFlags::copy_in(&buf[..95], true),
        Err(QuestError::Size(95))
    );
}

#[test]
fn guid_lists() {
    let mut l = GuidList::default();
    for g in 0..40 {
        l.add(g);
    }
    l.add(3);
    assert_eq!(l.0.len(), 32);
    l.remove(1);
    assert_eq!(l.0[1], 31); // swapped with the last
    assert!(!l.contains(1) && l.contains(31));
}

// ------------------------------------------------------------ tables

/// Structural checks of the two tables; one line each.
fn check_tables(t: &QuestTables) -> Vec<String> {
    let mut e = Vec::new();
    if t.rows.len() != 41 {
        e.push(format!("{} rows", t.rows.len()));
    }
    let mut chains = std::collections::BTreeSet::new();
    let mut slots = std::collections::BTreeSet::new();
    for (i, r) in t.rows.iter().enumerate() {
        if usize::from(r.index) != i {
            e.push(format!("row {i}: index {}", r.index));
        }
        if !chains.insert(r.chain) {
            e.push(format!("row {i}: chain {} repeated", r.chain));
        }
        let intro = i >= 37;
        if intro != r.flag.is_none() || (intro && r.chain != i as u8) {
            e.push(format!("row {i}: intro shape"));
        }
        if let Some(f) = r.flag {
            if f >= SLOTS || !slots.insert(f) {
                e.push(format!("row {i}: flag {f}"));
            }
        }
        if r.filter.is_some_and(|f| f > 42) || (intro && !r.unknown && r.filter != Some(42)) {
            e.push(format!("row {i}: filter {:?}", r.filter));
        }
        if let Some(m) = r.msgs {
            if !t.messages.iter().any(|x| x.table == m) {
                e.push(format!("row {i}: no message table {m:#x}"));
            }
        }
    }
    let mut counts: BTreeMap<(u32, u8), u8> = BTreeMap::new();
    for m in &t.messages {
        *counts.entry((m.table, m.state)).or_default() += 1;
        if !t.rows.iter().any(|r| r.msgs == Some(m.table)) {
            e.push(format!("message table {:#x} unreferenced", m.table));
        }
    }
    for ((table, state), n) in counts {
        if n > 16 {
            e.push(format!("{table:#x} state {state}: {n} entries"));
        }
    }
    e
}

#[test]
fn tables_parse_and_check() {
    let t = QuestTables::load().unwrap();
    assert_eq!(t.messages.len(), 794);
    assert_eq!(check_tables(&t), Vec::<String>::new());
    let r1 = &t.rows[1];
    assert_eq!(
        (r1.chain, r1.filter, r1.flag2, r1.init_no, r1.seq_id),
        (1, Some(1), Some(41), Some(4), Some(2))
    );
    assert_eq!(r1.callbacks.len(), 7);
    assert!(r1.specified && r1.status_fn.is_none());
    // Row 40 (Act V intro) is specified.
    let r40 = &t.rows[40];
    assert!(!r40.unknown && r40.specified);
    assert_eq!(r40.callbacks, [(0, 0x0058_6B50), (11, 0x0058_E990)]);
    // Chains 25 and 30 share Flavie's table.
    assert_eq!(t.rows[7].msgs, t.rows[29].msgs);
    assert_eq!(t.rows[24].filter, Some(16));
}

/// The Act V intro's own init and table (`act5::intro`) equal row 40
/// and its message table of the TSVs.
#[test]
fn act5_intro_matches_its_rows() {
    use super::act5::intro;
    let t = QuestTables::load().unwrap();
    let row = &t.rows[40];
    let ctl = QuestControl::new(&t, &mut Seed::new(1, 2)).unwrap();
    let r = ctl.record(40).unwrap();
    let cbs: Vec<u8> = row.callbacks.iter().map(|c| c.0).collect();
    for ev in 0..16u8 {
        assert_eq!(
            r.callbacks & (1 << ev) != 0,
            cbs.contains(&ev),
            "event {ev}"
        );
    }
    assert_eq!(
        (r.status_fn, r.active_fn, r.msgs, Some(r.filter)),
        (row.status_fn, row.active_fn, row.msgs, row.filter)
    );
    let tsv: Vec<(u8, u16, u16)> = t
        .messages
        .iter()
        .filter(|m| m.table == intro::TABLE_ADDR)
        .map(|m| (m.state, m.npc, m.string))
        .collect();
    assert_eq!(tsv, intro::TABLE);
    assert!(t
        .messages
        .iter()
        .filter(|m| m.table == intro::TABLE_ADDR)
        .all(|m| m.menu == 0));
}

/// M08: the checks report a changed cell.
#[test]
fn tables_check_catches_perturbations() {
    let msgs = parse_messages(MESSAGES_TSV).unwrap();
    // A repeated flag slot.
    let bad = QUESTS_TSV.replacen("\n2\t2\t2\t", "\n2\t2\t1\t", 1);
    let t = QuestTables {
        rows: parse_quests(&bad).unwrap(),
        messages: msgs.clone(),
    };
    let e = check_tables(&t);
    assert_eq!(e, ["row 2: flag 1"]);
    // A message table no row references.
    let bad = MESSAGES_TSV.replacen("0x00736460", "0x00736464", 1);
    let t = QuestTables {
        rows: parse_quests(QUESTS_TSV).unwrap(),
        messages: parse_messages(&bad).unwrap(),
    };
    assert_eq!(check_tables(&t), ["message table 0x736464 unreferenced"]);
    // Strict parsing.
    let bad = QUESTS_TSV.replacen("0:0x0058FAD0", "15:0x0058FAD0", 1);
    assert!(matches!(parse_quests(&bad), Err(TsvError::Value { .. })));
    let bad = MESSAGES_TSV.replacen("\t155\t0\t0", "\t155\t0", 1);
    assert!(matches!(
        parse_messages(&bad),
        Err(TsvError::Columns { .. })
    ));
}

// ------------------------------------------------------- fake world

#[derive(Default, Clone)]
pub(super) struct Player {
    pub(super) guid: u32,
    pub(super) quests: PlayerQuests,
    pub(super) act: Option<u8>,
    pub(super) level: Option<u32>,
    pub(super) class: u8,
    pub(super) seed: Seed,
    pub(super) stats: BTreeMap<u16, i32>,
    pub(super) byte4c: u8,
    pub(super) items: Vec<[u8; 4]>,
}

#[derive(Default)]
pub(super) struct Fake {
    /// The object control's seed (`object_seed`; absent: none).
    pub(super) object_seed: Option<Seed>,
    pub(super) frame: i32,
    pub(super) difficulty: u8,
    pub(super) expansion: bool,
    pub(super) game_type: u8,
    pub(super) players: BTreeMap<UnitId, Player>,
    /// Monsters: (guid, class, kind).
    pub(super) monsters: BTreeMap<UnitId, (u32, u16, UnitKind)>,
    pub(super) chains: BTreeMap<UnitId, QuestChain>,
    pub(super) den: (u32, u32, u32, u32),
    pub(super) spot: Option<(i32, i32)>,
    pub(super) near: Vec<UnitId>,
    /// Party members by player (`None`: no party).
    pub(super) party: BTreeMap<UnitId, Vec<UnitId>>,
    /// Objects: (guid, class, mode).
    pub(super) objects: BTreeMap<UnitId, (u32, u16, i32)>,
    /// Unit positions (x, y, room).
    pub(super) pos: BTreeMap<UnitId, (i32, i32, RoomId)>,
    /// Room tile rectangles (x0, y0, x1, y1); `room_contains` excludes
    /// the last row and column.
    pub(super) rooms: BTreeMap<RoomId, (i32, i32, i32, i32)>,
    /// Spawn results in order (empty: fails).
    pub(super) spawns: Vec<Option<UnitId>>,
    /// Players chatting with an NPC.
    pub(super) chats: BTreeMap<UnitId, Vec<UnitId>>,
    /// Item codes of item units.
    pub(super) item_codes: BTreeMap<UnitId, [u8; 4]>,
    /// Items that are not their player's cursor item (`is_cursor_item`).
    pub(super) off_cursor: Vec<UnitId>,
    /// The `find_object_near` answer.
    pub(super) near_object: Option<UnitId>,
    pub(super) sent: Vec<(UnitId, Vec<u8>)>,
    pub(super) log: Vec<String>,
    /// `open_portal` results in order (empty: fails).
    pub(super) portals: Vec<Option<UnitId>>,
    /// Missiles created so far (ids 0x9001, 0x9002, …).
    pub(super) next_missile: u32,
    /// Client save flags by player (absent: no client).
    pub(super) client_flags: BTreeMap<UnitId, u16>,
    /// The game's rules (default `Original`; a mod's in the MS8d tests).
    pub(super) rules: crate::rules::Ruleset,
    // Act II seams.
    /// Levels of non-player units (objects, monsters).
    pub(super) unit_levels: BTreeMap<UnitId, u32>,
    /// The quest-chest gate is closed (`quest_chest_gate` false).
    pub(super) gate_closed: bool,
    /// `quest_drop` fails.
    pub(super) drop_fails: bool,
    /// `drop_item_at` creates nothing (it still logs the attempt).
    pub(super) drop_at_fails: bool,
    /// Items made by `quest_drop`, in order (ids from 600).
    pub(super) dropped: Vec<UnitId>,
    /// `spawn_quest_object` results in order (empty: fails).
    pub(super) object_spawns: Vec<Option<UnitId>>,
    /// Busy players.
    pub(super) busy: Vec<UnitId>,
    /// Interact units by player.
    pub(super) interact: BTreeMap<UnitId, (u8, u32)>,
    /// `missiles.txt` Range by row.
    pub(super) missile_ranges: BTreeMap<u32, i32>,
    /// Trading players.
    pub(super) trading: Vec<UnitId>,
    /// `npc_hold_chat` answer.
    pub(super) npc_held: bool,
    /// `spawn_location` answer.
    pub(super) spawn_loc: Option<(i32, i32, RoomId)>,
    // -- Act IV q1/q3 fake fields.
    /// `room_in_act_at`'s answer.
    pub(super) a4_room: Option<RoomId>,
    /// `unit_distance` by the first unit (a player); missing: `None`.
    pub(super) a4_dist: BTreeMap<UnitId, i32>,
    /// `wielded_weapon_code` by player.
    pub(super) a4_wield: BTreeMap<UnitId, [u8; 4]>,
    // -- end Act IV q1/q3 fake fields.

    // -- Act IV q2 fake fields.
    /// `FrameCnt1` of every object.
    pub(super) q2_fc1: i32,
    /// `spawn_superunique` results in order (empty: fails).
    pub(super) q2_superuniques: Vec<Option<UnitId>>,
    /// `level_monsters` answer.
    pub(super) q2_level_monsters: Vec<UnitId>,
    /// Dead units.
    pub(super) q2_dead: Vec<UnitId>,
    /// Non-zero alignments.
    pub(super) q2_align: BTreeMap<UnitId, u32>,
    /// Players whose client is busy (`client_idle` false).
    pub(super) q2_busy: Vec<UnitId>,
    // -- end Act IV q2 fake fields.

    // -- Act V part 1 fake fields.
    /// `adjacent_units` answers by room.
    pub(super) a5_adjacent: BTreeMap<RoomId, Vec<UnitId>>,
    /// `unit_mode` answers (default 0).
    pub(super) a5_modes: BTreeMap<UnitId, i32>,
    /// `unit_distance` answers by (a, b) (default 100).
    pub(super) a5_dist: BTreeMap<(UnitId, UnitId), i32>,
    /// `place_object` results in order (empty: fails).
    pub(super) a5_places: Vec<Option<UnitId>>,
    /// `critical_spawn` / `preset_superunique_spawn` results in order
    /// (empty: fails).
    pub(super) a5_crits: Vec<Option<UnitId>>,
    /// `apply_map_ai` answer.
    pub(super) a5_map_ai: bool,
    /// `quest_item_level` and `item_drop_sound` answers.
    pub(super) a5_item_level: i32,
    pub(super) a5_drop_sound: i32,
    // -- end Act V part 1 fake fields.

    // -- Act V part 2 fake fields.
    /// Active waypoints (player, level).
    pub(super) a5_waypoints: Vec<(UnitId, u32)>,
    /// `spawn_superunique` results in order (empty: fails).
    pub(super) a5_superuniques: Vec<Option<UnitId>>,
    /// `max_level` answer; `experience_threshold` table by level.
    pub(super) a5_max_level: i32,
    pub(super) a5_thresholds: BTreeMap<i32, u32>,
    /// `create_object_at` / `create_missile_at` answer.
    pub(super) a5_created: Option<UnitId>,
    /// `monstats_rows` answer; zoo-eligible classes.
    pub(super) a5_monstats_rows: u32,
    pub(super) a5_zoo: Vec<u32>,
    // -- end Act V part 2 fake fields.

    // -- Act I answers fake fields.
    /// Positions of units without a room (an object left in a freed
    /// room); `unit_xy` answers these, then [`Fake::pos`].
    pub(super) xy: BTreeMap<UnitId, (i32, i32)>,
    // -- end Act I answers fake fields.

    // -- Helper seam fake fields.
    /// `box_collides`: `Some(free)` → only these box origins are free;
    /// `None` → everything is free.
    pub(super) h_free: Option<Vec<(i32, i32)>>,
    /// `spawn_missile` fails.
    pub(super) h_missile_fails: bool,
    /// `objects.txt` `Mode1` by object (missing: 0).
    pub(super) h_mode1: BTreeMap<UnitId, bool>,
    /// `room_warp_tiles` by object.
    pub(super) h_tiles: BTreeMap<UnitId, Vec<UnitId>>,
    /// Town portal GUIDs by player; portal partners.
    pub(super) h_tp: BTreeMap<UnitId, u32>,
    pub(super) h_partner: BTreeMap<UnitId, UnitId>,
    // -- end helper seam fake fields.
}

pub(super) const P1: UnitId = UnitId(1);
pub(super) const P2: UnitId = UnitId(2);
pub(super) const AKARA_U: UnitId = UnitId(0x10);
pub(super) const NAVI_U: UnitId = UnitId(0x11);
pub(super) const WARRIV_U: UnitId = UnitId(0x12);

impl Fake {
    pub(super) fn new() -> Self {
        let mut f = Fake {
            expansion: true,
            ..Fake::default()
        };
        f.players.insert(
            P1,
            Player {
                guid: 1,
                act: Some(0),
                level: Some(1),
                seed: Seed::init_low(77),
                ..Player::default()
            },
        );
        for (u, class) in [
            (AKARA_U, npc::AKARA),
            (NAVI_U, npc::NAVI),
            (WARRIV_U, npc::WARRIV1),
        ] {
            f.monsters.insert(
                u,
                (
                    u.0,
                    class,
                    UnitKind::Monster {
                        class: u32::from(class),
                        superunique: None,
                        owner: None,
                    },
                ),
            );
        }
        f
    }

    pub(super) fn p(&mut self, u: UnitId) -> &mut Player {
        self.players.get_mut(&u).unwrap()
    }

    pub(super) fn flags(&self, u: UnitId) -> QuestFlags {
        self.players[&u].quests.flags[usize::from(self.difficulty)]
    }

    pub(super) fn sent_ids(&self) -> Vec<u8> {
        self.sent.iter().map(|m| m.1[0]).collect()
    }
}

impl QuestWorld for Fake {
    fn tick_rate(&self) -> crate::tick::TickRate {
        crate::tick::TickRate::BASE
    }
    fn rules(&self) -> &crate::rules::Ruleset {
        &self.rules
    }
    fn frame(&self) -> i32 {
        self.frame
    }
    fn difficulty(&self) -> u8 {
        self.difficulty
    }
    fn expansion(&self) -> bool {
        self.expansion
    }
    fn game_type(&self) -> u8 {
        self.game_type
    }
    fn has_act2(&self) -> bool {
        true
    }
    fn players(&self) -> Vec<UnitId> {
        self.players.keys().copied().collect()
    }
    fn first_client_player(&self) -> Option<UnitId> {
        self.players.keys().next().copied()
    }
    fn guid(&self, unit: UnitId) -> u32 {
        self.players
            .get(&unit)
            .map(|p| p.guid)
            .or_else(|| self.monsters.get(&unit).map(|m| m.0))
            .or_else(|| self.objects.get(&unit).map(|o| o.0))
            .unwrap_or(unit.0)
    }
    fn player_by_guid(&self, guid: u32) -> Option<UnitId> {
        self.players.iter().find(|p| p.1.guid == guid).map(|p| *p.0)
    }
    fn quests(&mut self, player: UnitId) -> Option<&mut PlayerQuests> {
        self.players.get_mut(&player).map(|p| &mut p.quests)
    }
    fn unit_act(&self, unit: UnitId) -> Option<u8> {
        self.players.get(&unit).and_then(|p| p.act)
    }
    fn unit_level(&self, unit: UnitId) -> Option<u32> {
        match self.players.get(&unit) {
            Some(p) => p.level,
            None => self.unit_levels.get(&unit).copied(),
        }
    }
    fn player_class(&self, player: UnitId) -> u8 {
        self.players[&player].class
    }
    fn unit_seed(&mut self, unit: UnitId) -> &mut Seed {
        &mut self.p(unit).seed
    }
    fn stat(&self, unit: UnitId, stat: u16) -> i32 {
        self.players[&unit].stats.get(&stat).copied().unwrap_or(0)
    }
    fn base_stat(&self, unit: UnitId, stat: u16) -> i32 {
        self.stat(unit, stat)
    }
    fn add_stat(&mut self, unit: UnitId, stat: u16, delta: i32) {
        *self.p(unit).stats.entry(stat).or_default() += delta;
    }
    fn attach_sound(&mut self, player: UnitId, sound: u16) {
        self.log.push(format!("sound {} {sound}", player.0));
    }
    fn player_byte_4c(&self, player: UnitId) -> u8 {
        self.players[&player].byte4c
    }
    fn set_player_byte_4c(&mut self, player: UnitId, v: u8) {
        self.p(player).byte4c = v;
    }
    fn quest_chain(&mut self, unit: UnitId) -> Option<&mut QuestChain> {
        self.chains.get_mut(&unit)
    }
    fn unit_kind(&self, unit: UnitId) -> UnitKind {
        if self.players.contains_key(&unit) {
            UnitKind::Player
        } else {
            self.monsters.get(&unit).map_or(UnitKind::Other, |m| m.2)
        }
    }
    fn monster_by_guid(&self, guid: u32) -> Option<(UnitId, u16)> {
        self.monsters
            .iter()
            .find(|m| m.1 .0 == guid)
            .map(|m| (*m.0, m.1 .1))
    }
    fn monster_class(&self, unit: UnitId) -> Option<u16> {
        self.monsters.get(&unit).map(|m| m.1)
    }
    fn players_near(&self, _: UnitId) -> Vec<UnitId> {
        self.near.clone()
    }
    fn party_members(&self, player: UnitId) -> Option<Vec<UnitId>> {
        self.party.get(&player).cloned()
    }
    fn send(&mut self, player: UnitId, msg: &[u8]) {
        self.sent.push((player, msg.to_vec()));
    }
    fn send_text_list(&mut self, player: UnitId, npc: UnitId, list: &[(u16, u32)]) {
        self.log.push(format!("0x27 {} {list:?}", npc.0));
        self.sent.push((player, vec![0x27]));
    }
    fn has_item(&self, player: UnitId, code: [u8; 4]) -> bool {
        self.players[&player].items.contains(&code)
    }
    fn item_code(&self, item: UnitId) -> Option<[u8; 4]> {
        self.item_codes.get(&item).copied()
    }
    fn is_cursor_item(&self, _: UnitId, item: UnitId) -> bool {
        !self.off_cursor.contains(&item)
    }
    fn delete_item(&mut self, player: UnitId, code: [u8; 4]) {
        self.p(player).items.retain(|&c| c != code);
        self.log
            .push(format!("delete {}", String::from_utf8_lossy(&code)));
    }
    fn reward_item(
        &mut self,
        player: UnitId,
        code: [u8; 4],
        level: i32,
        quality: u8,
        _: bool,
    ) -> Option<UnitId> {
        self.log.push(format!(
            "reward {} {level} {quality}",
            String::from_utf8_lossy(&code)
        ));
        self.p(player).items.push(code);
        Some(UnitId(500))
    }
    fn drop_item_at(&mut self, _: UnitId, code: [u8; 4], quality: u8) -> bool {
        self.log
            .push(format!("drop {} {quality}", String::from_utf8_lossy(&code)));
        !self.drop_at_fails
    }
    fn quest_items(&self, _: UnitId) -> Vec<(UnitId, u8)> {
        Vec::new()
    }
    fn den_region(&self) -> (u32, u32, u32, u32) {
        self.den
    }
    fn true_tomb_level(&self) -> u32 {
        68
    }
    fn free_spot(
        &mut self,
        _: UnitId,
        size: u32,
        mask: u32,
        radius: u32,
        limit: u32,
    ) -> Option<(i32, i32)> {
        self.log
            .push(format!("spot {size} {mask:#x} {radius} {limit}"));
        self.spot
    }
    fn create_portal(&mut self, _: UnitId, x: i32, y: i32, class: u16, level: u32) -> bool {
        self.log.push(format!("portal {x} {y} {class} {level}"));
        true
    }
    fn schedule_quest_event(&mut self, object: UnitId, frame: i32) {
        self.log.push(format!("event7 {} {frame}", object.0));
    }
    fn object_mode(&self, object: UnitId) -> i32 {
        self.objects.get(&object).map_or(0, |o| o.2)
    }
    fn set_object_mode(&mut self, object: UnitId, mode: i32) {
        self.log.push(format!("mode {} {mode}", object.0));
        if let Some(o) = self.objects.get_mut(&object) {
            o.2 = mode;
        }
    }
    fn object_by_guid(&self, guid: u32) -> Option<(UnitId, u16)> {
        self.objects
            .iter()
            .find(|o| o.1 .0 == guid)
            .map(|o| (*o.0, o.1 .1))
    }
    fn mercenary_reward(&mut self, _: UnitId, npc: u16) {
        self.log.push(format!("merc {npc}"));
    }
    fn unit_position(&self, u: UnitId) -> Option<(i32, i32, RoomId)> {
        self.pos.get(&u).copied()
    }
    fn unit_xy(&self, u: UnitId) -> Option<(i32, i32)> {
        self.xy
            .get(&u)
            .copied()
            .or_else(|| self.pos.get(&u).map(|p| (p.0, p.1)))
    }
    fn room_contains(&self, room: RoomId, x: i32, y: i32) -> bool {
        self.rooms
            .get(&room)
            .is_some_and(|r| x >= r.0 && y >= r.1 && x < r.2 - 1 && y < r.3 - 1)
    }
    fn room_at(&self, _: RoomId, x: i32, y: i32) -> Option<RoomId> {
        self.rooms
            .iter()
            .find(|r| x >= r.1 .0 && y >= r.1 .1 && x < r.1 .2 && y < r.1 .3)
            .map(|r| *r.0)
    }
    fn free_spot_at(
        &mut self,
        room: RoomId,
        x: i32,
        y: i32,
        size: u32,
        mask: u32,
        radius: u32,
        limit: u32,
    ) -> Option<(i32, i32, RoomId)> {
        self.log
            .push(format!("spot at {x} {y} {size} {mask:#x} {radius} {limit}"));
        self.spot.map(|(dx, dy)| (x + dx, y + dy, room))
    }
    fn spawn_monster(
        &mut self,
        _: RoomId,
        x: i32,
        y: i32,
        class: u16,
        mode: u8,
        r: u32,
    ) -> Option<UnitId> {
        self.log
            .push(format!("spawn {class} {x} {y} mode {mode} r {r}"));
        if self.spawns.is_empty() {
            None
        } else {
            self.spawns.remove(0)
        }
    }
    fn or_unit_flags(&mut self, u: UnitId, f: u32) {
        self.log.push(format!("flags {} {f:#x}", u.0));
    }
    fn monsters(&self) -> Vec<UnitId> {
        self.monsters.keys().copied().collect()
    }
    fn npc_chat_clients(&self, n: UnitId) -> Option<Vec<UnitId>> {
        self.chats.get(&n).cloned()
    }
    fn remove_monster(&mut self, m: UnitId) {
        self.log.push(format!("remove {}", m.0));
    }
    fn drop_preset_monster(&mut self, act: u8, class: u16) {
        self.log.push(format!("preset {act} {class}"));
    }
    fn find_object_near(&self, _: UnitId, _: u16) -> Option<UnitId> {
        self.near_object
    }
    fn create_object(&mut self, _: RoomId, x: i32, y: i32, class: u16) -> Option<UnitId> {
        self.log.push(format!("object {class} {x} {y}"));
        None
    }
    fn object_anim_length(&self, _: UnitId) -> i32 {
        0x1000
    }
    fn schedule_object_event(&mut self, o: UnitId, ev: u8, frame: i32) {
        self.log.push(format!("event{ev} {} {frame}", o.0));
    }
    fn open_quest_message(&mut self, p: UnitId, o: UnitId, msg: u16) {
        self.log.push(format!("message {} {} {msg}", p.0, o.0));
    }
    fn client_in_act(&mut self, p: UnitId, act: u8) -> bool {
        self.unit_act(p) == Some(act)
    }
    fn start_tainted_sun(&mut self, act: u8) {
        self.log.push(format!("sun start {act}"));
    }
    fn end_tainted_sun(&mut self) {
        self.log.push("sun end".into());
    }
    fn quest_chest_gate(&mut self, _: UnitId, _: UnitId) -> bool {
        !self.gate_closed
    }
    fn quest_drop(
        &mut self,
        u: UnitId,
        code: [u8; 4],
        quality: u8,
        level: Option<i32>,
        droppable: bool,
    ) -> Option<UnitId> {
        self.log.push(format!(
            "qdrop {} {} {quality} {level:?} {droppable}",
            u.0,
            String::from_utf8_lossy(&code)
        ));
        if self.drop_fails {
            return None;
        }
        let item = UnitId(600 + self.dropped.len() as u32);
        self.dropped.push(item);
        self.item_codes.insert(item, code);
        Some(item)
    }
    fn identify_item(&mut self, item: UnitId) {
        self.log.push(format!("identify {}", item.0));
    }
    fn object_treasure(&mut self, o: UnitId, _: UnitId, kind: u8) {
        self.log.push(format!("treasure {} {kind}", o.0));
    }
    fn drop_gold(&mut self, o: UnitId) {
        self.log.push(format!("gold {}", o.0));
    }
    fn set_room_portal(&mut self, room: RoomId, on: bool) {
        self.log.push(format!("room portal {room:?} {on}"));
    }
    fn spawn_quest_object(&mut self, _: RoomId, x: i32, y: i32, class: u16) -> Option<UnitId> {
        self.log.push(format!("spawn object {class} {x} {y}"));
        if self.object_spawns.is_empty() {
            None
        } else {
            self.object_spawns.remove(0)
        }
    }
    fn free_object_collision(&mut self, o: UnitId) {
        self.log.push(format!("free collision {}", o.0));
    }
    fn player_busy(&mut self, p: UnitId) -> bool {
        self.busy.contains(&p)
    }
    fn interact_unit(&mut self, p: UnitId) -> Option<(u8, u32)> {
        self.interact.get(&p).copied()
    }
    fn set_interact_unit(&mut self, p: UnitId, unit: Option<(u8, u32)>) {
        self.log.push(format!("interact {} {unit:?}", p.0));
        match unit {
            Some(u) => self.interact.insert(p, u),
            None => self.interact.remove(&p),
        };
    }
    fn open_insert_dialog(&mut self, p: UnitId, o: UnitId) {
        self.log.push(format!("insert dialog {} {}", p.0, o.0));
    }
    fn missile_range(&mut self, row: u32) -> Option<i32> {
        self.missile_ranges.get(&row).copied()
    }
    fn is_trading(&mut self, p: UnitId) -> bool {
        self.trading.contains(&p)
    }
    fn remove_unit(&mut self, u: UnitId) {
        self.log.push(format!("remove unit {}", u.0));
    }
    fn npc_hold_chat(&mut self, n: UnitId) -> bool {
        self.log.push(format!("hold chat {}", n.0));
        self.npc_held
    }
    fn spawn_location(&mut self, act: u8, level: u32, kind: u8) -> Option<(i32, i32, RoomId)> {
        self.log
            .push(format!("spawn location {act} {level} {kind}"));
        self.spawn_loc
    }
    fn free_spot_near(
        &mut self,
        room: RoomId,
        x: i32,
        y: i32,
        size: u32,
        mask: u32,
        radius: u32,
    ) -> Option<(i32, i32, RoomId)> {
        self.log
            .push(format!("spot near {x} {y} {size} {mask:#x} {radius}"));
        self.spot.map(|(dx, dy)| (x + dx, y + dy, room))
    }
    fn npc_intro_heard(&mut self, p: UnitId, class: u16) -> bool {
        let d = usize::from(self.difficulty);
        self.p(p).quests.intro[d].contains(&class)
    }
    fn set_npc_intro(&mut self, p: UnitId, class: u16) {
        let d = usize::from(self.difficulty);
        self.p(p).quests.intro[d].insert(class);
    }
    fn unhandled(&mut self, chain: u8, function: u32) {
        self.log.push(format!("unhandled {chain} {function:#x}"));
    }
    fn spawn_monster_flags(
        &mut self,
        room: RoomId,
        x: i32,
        y: i32,
        class: u16,
        mode: u8,
        spread: i32,
        flags: u32,
    ) -> Option<UnitId> {
        self.log.push(format!(
            "spawn {class} {x} {y} room {} mode {mode} spread {spread} flags {flags:#x}",
            room.0
        ));
        if self.spawns.is_empty() {
            None
        } else {
            self.spawns.remove(0)
        }
    }
    fn open_portal(
        &mut self,
        owner: Option<UnitId>,
        room: RoomId,
        x: i32,
        y: i32,
        level: u32,
        class: u16,
        exact: bool,
    ) -> Option<UnitId> {
        self.log.push(format!(
            "portal {:?} room {} {x} {y} {class} {level} exact {exact}",
            owner.map(|o| o.0),
            room.0
        ));
        if self.portals.is_empty() {
            None
        } else {
            self.portals.remove(0)
        }
    }
    fn create_missile(
        &mut self,
        owner: UnitId,
        skill: u16,
        level: u8,
        class: u16,
        x: i32,
        y: i32,
    ) -> Option<UnitId> {
        self.log.push(format!(
            "missile {class} owner {} skill {skill} level {level} {x} {y}",
            owner.0
        ));
        self.next_missile += 1;
        Some(UnitId(0x9000 + self.next_missile))
    }
    fn set_missile_target(&mut self, m: UnitId, a: u32, b: u32) {
        self.log.push(format!("missile data {} {a:#x} {b}", m.0));
    }
    fn refresh_room(&mut self, u: UnitId) {
        self.log.push(format!("refresh {}", u.0));
    }
    fn spawn_object(
        &mut self,
        room: RoomId,
        x: i32,
        y: i32,
        class: u16,
        mode: i32,
    ) -> Option<UnitId> {
        self.log.push(format!(
            "object {class} {x} {y} room {} mode {mode}",
            room.0
        ));
        let u = UnitId(0xA000 + self.objects.len() as u32);
        self.objects.insert(u, (u.0, class, mode));
        Some(u)
    }
    fn client_save_flags(&self, p: UnitId) -> Option<u16> {
        self.client_flags.get(&p).copied()
    }
    fn set_client_save_flags(&mut self, p: UnitId, flags: u16) {
        self.log.push(format!("progression {} {flags:#06x}", p.0));
        self.client_flags.insert(p, flags);
    }

    // -- Act IV q1/q3 seam fakes.
    fn room_in_act_at(&mut self, act: u8, x: i32, y: i32) -> Option<RoomId> {
        self.log.push(format!("room act {act} {x} {y}"));
        self.a4_room
    }
    fn distance_between(&mut self, a: UnitId, b: UnitId) -> Option<i32> {
        self.a5_dist
            .get(&(a, b))
            .or_else(|| self.a4_dist.get(&a))
            .copied()
    }
    fn wielded_weapon_code(&mut self, player: UnitId) -> Option<[u8; 4]> {
        self.a4_wield.get(&player).copied()
    }
    // -- end Act IV q1/q3 seam fakes.

    // -- Act IV q2 seam fakes.
    fn object_frame_count1(&mut self, _: UnitId) -> i32 {
        self.q2_fc1
    }
    fn superunique_id(&mut self, n: u8) -> u16 {
        // The loader fills the array by hcIdx.
        u16::from(n)
    }
    fn spawn_superunique(
        &mut self,
        d: UnitId,
        x: i32,
        y: i32,
        arg: u32,
        id: u16,
    ) -> Option<UnitId> {
        self.log
            .push(format!("superunique {id} {x} {y} at {} arg {arg}", d.0));
        if self.q2_superuniques.is_empty() {
            None
        } else {
            self.q2_superuniques.remove(0)
        }
    }
    fn level_monsters(&mut self, level: u32) -> Vec<UnitId> {
        self.log.push(format!("level monsters {level}"));
        self.q2_level_monsters.clone()
    }
    fn unit_dead(&mut self, u: UnitId) -> bool {
        self.q2_dead.contains(&u)
    }
    fn alignment(&mut self, u: UnitId) -> u32 {
        self.q2_align.get(&u).copied().unwrap_or(0)
    }
    fn end_interaction(&mut self, p: UnitId) {
        self.log.push(format!("end interaction {}", p.0));
    }
    fn warp_to_level(&mut self, p: UnitId, level: u32, arg: u32) {
        self.log.push(format!("warp {} {level} {arg}", p.0));
    }
    fn client_idle(&mut self, p: UnitId) -> bool {
        !self.q2_busy.contains(&p)
    }
    fn act_change(&mut self, p: UnitId, level: u32, arg: u32) {
        self.log.push(format!("act change {} {level} {arg}", p.0));
    }
    fn activate_waypoint(&mut self, p: UnitId, level: u32, d: u8) {
        self.log.push(format!("waypoint {} {level} {d}", p.0));
    }
    // -- end Act IV q2 seam fakes.

    // -- Act V part 1 seam fakes.
    fn place_object(
        &mut self,
        room: RoomId,
        x: i32,
        y: i32,
        class: u16,
        flags: [u8; 3],
    ) -> Option<UnitId> {
        self.log
            .push(format!("place {class} {x} {y} room {} {flags:?}", room.0));
        if self.a5_places.is_empty() {
            None
        } else {
            self.a5_places.remove(0)
        }
    }
    fn adjacent_units(&mut self, room: RoomId) -> Vec<UnitId> {
        self.a5_adjacent.get(&room).cloned().unwrap_or_default()
    }
    fn unit_mode(&mut self, unit: UnitId) -> i32 {
        self.a5_modes.get(&unit).copied().unwrap_or(0)
    }
    fn critical_spawn(&mut self, room: RoomId, x: i32, y: i32, class: u16) -> Option<UnitId> {
        self.log
            .push(format!("critical {class} {x} {y} room {}", room.0));
        if self.a5_crits.is_empty() {
            None
        } else {
            self.a5_crits.remove(0)
        }
    }
    fn kill_in_place(&mut self, unit: UnitId) {
        self.log.push(format!("kill {}", unit.0));
    }
    fn apply_map_ai(&mut self, unit: UnitId, map_ai: u32) -> bool {
        self.log.push(format!("map ai {} {map_ai:#x}", unit.0));
        self.a5_map_ai
    }
    fn npc_leave_town(&mut self, unit: UnitId) {
        self.log.push(format!("leave town {}", unit.0));
    }
    fn kill_in_town(&mut self, unit: UnitId) {
        self.log.push(format!("kill in town {}", unit.0));
    }
    fn object_leave_room(&mut self, object: UnitId) {
        self.log.push(format!("leave room {}", object.0));
    }
    fn quest_item_level(&mut self, _: UnitId) -> i32 {
        self.a5_item_level
    }
    fn item_drop_sound(&mut self, _: UnitId) -> i32 {
        self.a5_drop_sound
    }
    fn add_resist_list(&mut self, player: UnitId, v: i32) {
        self.log.push(format!("resist {} {v}", player.0));
    }
    fn preset_superunique_spawn(
        &mut self,
        room: RoomId,
        x: i32,
        y: i32,
        superunique: u16,
    ) -> Option<UnitId> {
        self.log
            .push(format!("superunique {superunique} {x} {y} room {}", room.0));
        if self.a5_crits.is_empty() {
            None
        } else {
            self.a5_crits.remove(0)
        }
    }
    // -- end Act V part 1 seam fakes.

    // -- Act V part 2 seam fakes.
    fn waypoint_active(&mut self, player: UnitId, level: u32) -> bool {
        self.log.push(format!("waypoint {} {level}", player.0));
        self.a5_waypoints.contains(&(player, level))
    }
    fn spawn_superunique_at_unit(&mut self, at: UnitId, superunique: u16) -> Option<UnitId> {
        self.log.push(format!("superunique {} {superunique}", at.0));
        if self.a5_superuniques.is_empty() {
            None
        } else {
            self.a5_superuniques.remove(0)
        }
    }
    fn quest_missile(&mut self, from: UnitId, to: UnitId, missile: u16, flags: u32, level: u8) {
        self.log.push(format!(
            "missile {missile} {} -> {} {flags:#x} {level}",
            from.0, to.0
        ));
    }
    fn remove_ancient(&mut self, monster: UnitId) {
        self.log.push(format!("remove ancient {}", monster.0));
    }
    fn max_level(&mut self, _: UnitId) -> i32 {
        self.a5_max_level
    }
    fn experience_threshold(&mut self, _: UnitId, level: i32) -> u32 {
        self.a5_thresholds.get(&level).copied().unwrap_or(0)
    }
    fn level_up(&mut self, player: UnitId) {
        self.log.push(format!("level up {}", player.0));
        let l = self.stat(player, 12) + 1;
        let next = self.a5_thresholds.get(&(l + 1)).copied().unwrap_or(0);
        let p = self.p(player);
        p.stats.insert(12, l);
        p.stats.insert(30, next as i32);
    }
    fn object_stairs_warp(&mut self, player: UnitId, object: UnitId) {
        self.log.push(format!("stairs {} {}", player.0, object.0));
    }
    fn create_object_at(&mut self, at: UnitId, class: u16, flags: u32) -> Option<UnitId> {
        self.log.push(format!("object at {} {class} {flags}", at.0));
        self.a5_created
    }
    fn create_missile_at(&mut self, at: UnitId, class: u16) -> Option<UnitId> {
        self.log.push(format!("missile at {} {class}", at.0));
        self.a5_created
    }
    fn drop_gold_amount(&mut self, at: UnitId, amount: u32) {
        self.log.push(format!("gold {} {amount}", at.0));
    }
    fn monstats_rows(&mut self) -> u32 {
        self.a5_monstats_rows
    }
    fn zoo_eligible(&mut self, class: u32) -> bool {
        self.a5_zoo.contains(&class)
    }
    // -- end Act V part 2 seam fakes.

    // -- Helper seam fakes.
    fn object_seed(&mut self) -> Option<&mut Seed> {
        self.object_seed.as_mut()
    }
    fn room_box(&mut self, room: RoomId) -> Option<crate::drlg::TileRect> {
        self.rooms
            .get(&room)
            .map(|r| crate::drlg::TileRect::new(r.0, r.1, r.2 - r.0, r.3 - r.1))
    }
    fn box_collides(&mut self, _: RoomId, x: i32, y: i32, _: i32, _: u32) -> bool {
        self.h_free.as_ref().is_some_and(|v| !v.contains(&(x, y)))
    }
    fn spawn_missile(&mut self, r: super::helpers::QuestMissile) -> Option<UnitId> {
        self.log.push(format!(
            "spawn missile {} owner {} origin {:?} flags {:#x} at {} {} target {} {} skill {} level {}",
            r.class,
            r.owner.0,
            r.origin.map(|o| o.0),
            r.flags,
            r.x,
            r.y,
            r.target_x,
            r.target_y,
            r.skill,
            r.level
        ));
        if self.h_missile_fails {
            return None;
        }
        self.next_missile += 1;
        Some(UnitId(0x9000 + self.next_missile))
    }
    fn set_missile_guid(&mut self, m: UnitId, v: u32) {
        self.log.push(format!("missile guid {} {v:#x}", m.0));
    }
    fn unit_by_guid(&mut self, kind: u8, guid: u32) -> Option<UnitId> {
        match kind {
            0 => self.player_by_guid(guid),
            1 => self.monster_by_guid(guid).map(|m| m.0),
            2 => self.object_by_guid(guid).map(|o| o.0),
            4 => self.item_codes.keys().find(|u| u.0 == guid).copied(),
            _ => None,
        }
    }
    fn monster_mode_at(&mut self, m: UnitId, mode: u8, x: i32, y: i32) {
        self.log
            .push(format!("mode request {} {mode} {x} {y}", m.0));
    }
    fn trade_button(&mut self, p: UnitId, button: u8) {
        self.log.push(format!("trade button {} {button}", p.0));
    }
    fn free_chat_node(&mut self, n: UnitId, p: UnitId) {
        self.log.push(format!("free chat {} {}", n.0, p.0));
    }
    fn clear_npc_chats(&mut self, n: UnitId) {
        self.log.push(format!("clear chats {}", n.0));
    }
    fn obelisk_close(&mut self, p: UnitId, o: UnitId) {
        self.log.push(format!("obelisk {} {}", p.0, o.0));
    }
    fn steeg_release(&mut self, o: UnitId, p: UnitId) {
        self.log.push(format!("steeg {} {}", o.0, p.0));
    }
    fn close_cube(&mut self, p: UnitId) {
        self.log.push(format!("close cube {}", p.0));
    }
    fn object_mode1(&mut self, o: UnitId) -> Option<bool> {
        Some(self.h_mode1.get(&o).copied().unwrap_or(false))
    }
    fn clear_unit_flags(&mut self, u: UnitId, f: u32) {
        self.log.push(format!("clear flags {} {f:#x}", u.0));
    }
    fn room_warp_tiles(&mut self, o: UnitId) -> Vec<UnitId> {
        self.h_tiles.get(&o).cloned().unwrap_or_default()
    }
    fn warp_through(&mut self, p: UnitId, tile: UnitId) {
        self.log.push(format!("warp tile {} {}", p.0, tile.0));
    }
    fn town_portal_guid(&mut self, p: UnitId) -> Option<u32> {
        self.h_tp.get(&p).copied()
    }
    fn portal_partner(&mut self, o: UnitId) -> Option<UnitId> {
        self.h_partner.get(&o).copied()
    }
    fn free_portal_object(&mut self, o: UnitId) {
        self.log.push(format!("free portal {}", o.0));
    }
    // -- end helper seam fakes.
}

pub(super) fn control() -> (QuestControl, Seed) {
    let mut game = Seed::init_low(0x1234);
    let ctl = QuestControl::new(&QuestTables::load().unwrap(), &mut game).unwrap();
    (ctl, game)
}

// ------------------------------------------------------------ records, game entry

#[test]
fn creation_order_and_seed() {
    let (ctl, game) = control();
    let order: Vec<u8> = ctl.records.iter().map(|r| r.chain).collect();
    assert_eq!(order[..5], [40, 39, 38, 37, 36]);
    assert_eq!(order[order.len() - 1], 0);
    assert_eq!(order.len(), 41);
    // Quest seed: init_low of one game-seed step.
    let mut g = Seed::init_low(0x1234);
    let lo = g.step();
    assert_eq!(game, g);
    assert_eq!(ctl.seed, Seed::init_low(lo));
    let r1 = ctl.record(1).unwrap();
    assert!(r1.active && r1.not_intro && r1.state == 1 && r1.init_no == 4 && r1.flag2 == Some(41));
    let intro = ctl.record(37).unwrap();
    assert!(intro.active && !intro.not_intro && intro.filter == 42);
    assert_eq!(ctl.game, QuestFlags::default());
}

#[test]
fn fresh_game_entry() {
    // A new game's first frame.
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    ctl.player_enters(&mut f, P1, 0).unwrap();
    let mut want_5e = vec![0x5E];
    want_5e.extend([1u8; 37]);
    let mut want_28 = hex("28 06 00000000 00");
    want_28.extend([0u8; 96]);
    let mut want_29 = vec![0x29];
    want_29.extend([0u8; 96]);
    let msgs: Vec<Vec<u8>> = f.sent.iter().map(|m| m.1.clone()).collect();
    assert_eq!(msgs, [want_5e, want_28, want_29]);
    assert!(ctl.picked);
    // The sequence functions of chains 1, 8, 18, 22, 31: chain 1 is at
    // state 1, not its pass state 5, so the walk stops there and
    // chain 2 stays at 0.
    assert_eq!(ctl.record(1).unwrap().state, 1);
    assert_eq!(ctl.record(2).unwrap().state, 0);
    // Chain 22 holds at its init state 1 and chain 31 at 0 (state ≠ 5
    // and not-intro → 1), so neither walk moves on.
    assert_eq!(ctl.record(22).unwrap().state, 1);
    assert_eq!(ctl.record(24).unwrap().state, 0);
    assert_eq!(ctl.record(31).unwrap().state, 0);
    assert_eq!(ctl.record(32).unwrap().state, 0);
    // Chain 8's is `act2::sequence` (state 1, not-intro → 1, chain 13
    // stays 0); chain 18's is `act3::sequence` (state 0 ≠ 5 → 1,
    // nothing moves).
    assert_eq!(ctl.record(13).unwrap().state, 0);
    assert_eq!(ctl.record(18).unwrap().state, 0);
    assert_eq!(ctl.record(16).unwrap().state, 0);
    assert!(f.log.is_empty(), "{:?}", f.log);
    // A second entry: callback 14 only, same messages.
    f.sent.clear();
    f.log.clear();
    ctl.player_enters(&mut f, P1, 0).unwrap();
    assert_eq!(f.sent_ids(), [0x5E, 0x28, 0x29]);
    assert!(f.log.iter().all(|l| l.starts_with("unhandled")));
}

#[test]
fn first_player_switches_quests_off() {
    // Slot 0 bit 0 (Warriv gossip, no_set_state 1): game record stays 0.
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    f.p(P1).quests.flags[0].set(0, 0);
    ctl.player_enters(&mut f, P1, 0).unwrap();
    assert_eq!(ctl.game, QuestFlags::default());
    // Slot 1 bit 15: chain 1 off for the game; 0x5E row 1 = 0.
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    f.p(P1).quests.flags[0].set(1, 15);
    ctl.player_enters(&mut f, P1, 0).unwrap();
    let r = ctl.record(1).unwrap();
    assert!(!r.not_intro && !r.active && r.status == 0);
    assert!(ctl.game.get(1, 15));
    assert_eq!(f.sent[0].1[2], 0);
    // Mode 1 skips step 2.
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    f.p(P1).quests.flags[0].set(1, 15);
    ctl.player_enters(&mut f, P1, 1).unwrap();
    assert!(ctl.record(1).unwrap().not_intro);
}

// ------------------------------------------------------------ updater

#[test]
fn timer_schedule() {
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    ctl.tick = 100;
    ctl.add_timer(9, TimerFn::Probe, 8).unwrap();
    for _ in 0..30 {
        ctl.update(&mut f);
    }
    assert_eq!(
        f.log,
        ["unhandled 9 0x6d", "unhandled 9 0x76", "unhandled 9 0x7f"]
    );
    // Fatal while executing.
    ctl.executing = true;
    assert_eq!(
        ctl.add_timer(9, TimerFn::Probe, 1),
        Err(QuestError::TimerWhileExecuting)
    );
}

#[test]
fn timer_wrap() {
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    ctl.tick = u32::MAX - 1;
    ctl.timers.push(QuestTimer {
        func: TimerFn::Probe,
        chain: 9,
        due: u32::MAX - 5,
        period: 3,
    });
    ctl.update(&mut f);
    assert_eq!(ctl.tick, u32::MAX);
    // due becomes 0xFFFFFFFF − due = 5 < tick: fires, then due = tick + 3.
    assert_eq!(f.log, ["unhandled 9 0xffffffff"]);
    assert_eq!(ctl.timers[0].due, 2);
}

// ------------------------------------------------------------ status

pub(super) fn rec_with(chain: u8, state: u8, init_no: u8, status: u8) -> QuestRecord {
    let (ctl, _) = control();
    let mut r = ctl.record(chain).unwrap().clone();
    (r.state, r.init_no, r.status) = (state, init_no, status);
    r
}

#[test]
fn default_status_rule() {
    let none = QuestFlags::default();
    let r = rec_with(10, 4, 4, 4);
    assert_eq!(QuestControl::default_status(&r, &none), Ok(4));
    let r = rec_with(10, 4, 4, 3);
    assert_eq!(QuestControl::default_status(&r, &none), Ok(12));
    let r = rec_with(1, 2, 4, 1);
    assert_eq!(QuestControl::default_status(&r, &none), Ok(1));
    let mut now = QuestFlags::default();
    now.set(1, bit::COMPLETED_NOW);
    assert_eq!(QuestControl::default_status(&r, &now), Ok(12));
    // s ≥ n and done → L.
    let mut done = QuestFlags::default();
    done.set(1, bit::PRIMARY_GOAL_DONE);
    let r = rec_with(1, 5, 4, 13);
    assert_eq!(QuestControl::default_status(&r, &done), Ok(13));
    assert_eq!(QuestControl::default_status(&r, &none), Ok(12));
    // Chain 4 cases.
    let r = rec_with(4, 2, 6, 6);
    assert_eq!(QuestControl::default_status(&r, &none), Ok(12));
    let mut now4 = QuestFlags::default();
    now4.set(4, bit::COMPLETED_NOW);
    let r = rec_with(4, 6, 6, 3);
    assert_eq!(QuestControl::default_status(&r, &now4), Ok(12));
    let r = rec_with(4, 7, 6, 3);
    assert_eq!(QuestControl::default_status(&r, &now4), Ok(3));
    // Filter 42 with done asserts (intros never reach it).
    let mut r = rec_with(37, 1, 0, 0);
    let mut d42 = QuestFlags::default();
    d42.set(42, bit::PRIMARY_GOAL_DONE);
    r.filter = 42;
    assert_eq!(
        QuestControl::default_status(&r, &d42),
        Err(QuestError::Filter(42))
    );
}

#[test]
fn akara_start_and_chat_end() {
    // `015956` frames 1729–1751.
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    ctl.player_enters(&mut f, P1, 0).unwrap();
    f.sent.clear();
    f.log.clear();
    let msg = hex("31 10000000 4000 0000");
    assert_eq!(ctl.quest_message(&mut f, P1, &msg[..9]), 0);
    assert_eq!(f.sent_ids(), [0x27, 0x29]);
    assert_eq!(ctl.record(1).unwrap().state, 2);
    f.sent.clear();
    ctl.npc_deactivate(&mut f, P1, AKARA_U);
    let msgs: Vec<Vec<u8>> = f.sent.iter().map(|m| m.1.clone()).collect();
    assert_eq!(msgs, [hex("5d 01 00 01 0000")]);
    assert_eq!(f.flags(P1).0[2..4], [0x04, 0x00]);
    // A second chat end sends nothing.
    f.sent.clear();
    ctl.npc_deactivate(&mut f, P1, AKARA_U);
    assert!(f.sent.is_empty());
    // Wrong size → 3.
    assert_eq!(ctl.quest_message(&mut f, P1, &msg[..8]), 3);
}

#[test]
fn request_quest_data() {
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    ctl.picked = true;
    ctl.request_quest_data(&mut f, P1).unwrap();
    assert_eq!(f.sent_ids(), [0x28, 0x52]);
    assert_eq!(f.sent[1].1, [&[0x52u8][..], &[0u8; 41][..]].concat());
    // Den of Evil status 1 → 0x50 with the monsters left.
    f.sent.clear();
    let r = ctl.record_mut(1).unwrap();
    (r.status, r.state) = (1, 2);
    r.extra.monsters_left = 9;
    ctl.request_quest_data(&mut f, P1).unwrap();
    assert_eq!(f.sent_ids(), [0x28, 0x50, 0x52]);
    assert_eq!(f.sent[1].1, hex("50 0100 0900 0000 0000 000000000000"));
    assert_eq!(f.sent[2].1[2], 1);
    // Staff tomb: slot 12 bit 0 and an Act II → level − 66.
    f.sent.clear();
    ctl.record_mut(1).unwrap().status = 0;
    f.p(P1).quests.flags[0].set(12, 0);
    ctl.request_quest_data(&mut f, P1).unwrap();
    assert_eq!(f.sent[1].1, hex("50 0100 0000 0200 0000 000000000000"));
}

#[test]
fn quest_completed_message() {
    let (ctl, _) = control();
    let mut f = Fake::new();
    assert_eq!(ctl.quest_completed(&mut f, P1, &[0x58, 5, 0]), 0);
    assert!(f.flags(P1).get(5, 12));
    assert_eq!(ctl.quest_completed(&mut f, P1, &[0x58, 42, 0]), 2);
    assert_eq!(ctl.quest_completed(&mut f, P1, &[0x58, 5]), 3);
    assert!(f.sent.is_empty());
}

#[test]
fn npc_wants_interact() {
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    assert_eq!(
        ctl.npc_wants_interact(&mut f, P1, WARRIV_U, npc::WARRIV1, true),
        Err(QuestError::NotPicked)
    );
    // Step 2 comes before the picked test: no interact flag, false.
    assert_eq!(
        ctl.npc_wants_interact(&mut f, P1, WARRIV_U, npc::WARRIV1, false),
        Ok(false)
    );
    ctl.picked = true;
    // Records are walked newest first: the Act I intro's active fn (not
    // specified) is reached before Warriv's gossip. A fresh player
    // (slot 0 bit 0 clear): true, one 8A 01 <Warriv GUID>.
    assert_eq!(
        ctl.npc_wants_interact(&mut f, P1, WARRIV_U, npc::WARRIV1, true),
        Ok(true)
    );
    assert_eq!(f.sent, [(P1, hex("8a 01 12000000"))]);
    f.sent.clear();
    // Slot 0 bit 0 set, no other Act I active fn true: false, no send.
    f.p(P1).quests.flags[0].set(0, 0);
    assert_eq!(
        ctl.npc_wants_interact(&mut f, P1, WARRIV_U, npc::WARRIV1, true),
        Ok(false)
    );
    assert!(f.sent.is_empty());
    // Without the interact flag the records are not walked.
    f.p(P1).quests.flags[0] = QuestFlags::default();
    assert_eq!(
        ctl.npc_wants_interact(&mut f, P1, WARRIV_U, npc::WARRIV1, false),
        Ok(false)
    );
    assert!(f.sent.is_empty());
}

#[test]
fn unique_event_and_gossip() {
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    f.players.insert(
        P2,
        Player {
            guid: 2,
            ..Player::default()
        },
    );
    ctl.unique_event(&mut f, 0);
    assert_eq!(f.sent_ids(), [0x28, 0x89, 0x28, 0x89]);
    assert_eq!(f.sent[1].1, [0x89, 0]);
    // 0x91: introduced NPCs packed at the front.
    f.sent.clear();
    f.p(P1).quests.intro[0].insert(150);
    f.p(P1).quests.intro[0].insert(265);
    npc_gossip(&mut f, P1, 0);
    let mut want = vec![0x91, 0];
    want.extend(150u16.to_le_bytes());
    want.extend(265u16.to_le_bytes());
    want.extend([0xFF; 20]);
    assert_eq!(f.sent[0].1, want);
    f.sent.clear();
    npc_gossip(&mut f, P1, 3);
    npc_gossip(&mut f, P1, 1);
    assert!(f.sent.is_empty());
}

// ------------------------------------------------------------ dispatch

#[test]
fn kill_parse_force_and_chain() {
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    let victim = UnitId(0x40);
    // No chain → nothing.
    ctl.monster_killed(&mut f, victim, Some(P1));
    assert!(f.log.is_empty());
    // add_link: unknown chain fails; duplicates refused; newest first.
    f.chains.insert(victim, QuestChain::default());
    assert!(!ctl.add_link(&mut f, victim, 99, None));
    assert!(ctl.add_link(&mut f, victim, 1, None));
    assert!(!ctl.add_link(&mut f, victim, 1, None));
    assert!(ctl.add_link(&mut f, victim, 6, None));
    assert_eq!(f.chains[&victim].0, [6, 1]);
    // Andariel (class 243 is diablo; Andariel is not forced): with chain
    // 6 made inactive, an unforced kill reaches only chain 1.
    ctl.record_mut(6).unwrap().active = false;
    f.monsters.insert(
        victim,
        (
            0x40,
            156,
            UnitKind::Monster {
                class: 156,
                superunique: None,
                owner: None,
            },
        ),
    );
    f.den = (10, 3, 5, 5);
    ctl.monster_killed(&mut f, victim, Some(P1));
    assert_eq!(ctl.record(1).unwrap().extra.monsters_left, 7);
    assert!(!f.log.iter().any(|l| l.contains("drop")));
    // A forced victim (superunique hcIdx 39, the Cow King) reaches chain 6
    // too: Andariel's gem drops.
    f.monsters.insert(
        victim,
        (
            0x40,
            391,
            UnitKind::Monster {
                class: 391,
                superunique: Some(39),
                owner: None,
            },
        ),
    );
    ctl.monster_killed(&mut f, victim, Some(P1));
    assert_eq!(f.log.iter().filter(|l| l.starts_with("drop")).count(), 3);
    // Killer = a monster owned by a player: that player.
    let minion = UnitId(0x41);
    f.monsters.insert(
        minion,
        (
            0x41,
            1,
            UnitKind::Monster {
                class: 1,
                superunique: None,
                owner: Some(P1),
            },
        ),
    );
    ctl.monster_killed(&mut f, victim, Some(minion));
    assert!(ctl.record(1).unwrap().extra.guids.contains(1));
}

#[test]
fn den_of_evil_cleared() {
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    f.players.insert(
        P2,
        Player {
            guid: 2,
            act: Some(0),
            ..Player::default()
        },
    );
    let victim = UnitId(0x40);
    f.chains.insert(victim, QuestChain(vec![1]));
    f.den = (10, 10, 5, 5);
    ctl.monster_killed(&mut f, victim, Some(P1));
    let r = ctl.record(1).unwrap();
    assert!(r.extra.done && r.extra.timer && r.state == 4);
    assert!(!r.has_callback(event::MONSTER_KILLED) && !r.has_callback(event::NPC_DEACTIVATE));
    assert!(ctl.game.get(1, 13));
    // Only the killer list gets 13 + 1; the others get 14, the
    // completed-now 0x5D and a 0x28 (I4), then everyone 0x28 + 0x89.
    assert!(f.flags(P1).get(1, 13) && f.flags(P1).get(1, 1));
    assert!(!f.flags(P1).get(1, 14));
    assert!(!f.flags(P2).get(1, 13) && f.flags(P2).get(1, 14));
    assert_eq!(f.sent_ids(), [0x5D, 0x28, 0x28, 0x89, 0x28, 0x89]);
    assert_eq!(f.sent[0], (P2, hex("5d 01 00 0c 0000")));
    // The timer fires 9 updater ticks later: 0x5D status 5 to everyone.
    f.sent.clear();
    for _ in 0..8 {
        ctl.update(&mut f);
    }
    assert!(f.sent.is_empty());
    ctl.update(&mut f);
    // P1 (goal done): the status byte 5; P2 (not done, s ≥ n): 12.
    assert_eq!(
        f.sent.iter().map(|m| m.1.clone()).collect::<Vec<_>>(),
        [hex("5d 01 00 05 0000"), hex("5d 01 00 0c 0000")]
    );
    assert!(ctl.timers.is_empty());
    // Game entry after the clear sends 0x89 00.
    f.sent.clear();
    ctl.player_enters(&mut f, P1, 0).unwrap();
    assert_eq!(f.sent_ids(), [0x5E, 0x28, 0x29, 0x89]);
}

#[test]
fn den_of_evil_few_left() {
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    let victim = UnitId(0x40);
    f.chains.insert(victim, QuestChain(vec![1]));
    f.den = (10, 5, 5, 5);
    ctl.monster_killed(&mut f, victim, Some(P1));
    assert_eq!(ctl.record(1).unwrap().flags, 0x20);
    assert_eq!(f.sent[0].1, hex("5d 01 20 04 0500"));
}

#[test]
fn den_of_evil_reward() {
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    let fl = &mut f.p(P1).quests.flags[0];
    fl.set(1, 1);
    fl.set(1, 13);
    fl.set(1, 2);
    ctl.record_mut(1).unwrap().state = 4;
    ctl.quest_message(&mut f, P1, &hex("31 10000000 4c00 0000")[..9]);
    let fl = f.flags(P1);
    assert!(fl.get(1, 0) && !fl.get(1, 1) && !fl.get(1, 2));
    assert!(fl.get(41, 13) && fl.get(41, 1));
    assert_eq!(f.players[&P1].stats[&5], 1);
    let r = ctl.record(1).unwrap();
    assert_eq!((r.state, r.status, r.flags), (5, 13, 0));
    assert!(r.guids.contains(1));
}

// ------------------------------------------------------------ Act I

#[test]
fn cairn_stone_order_vector() {
    let mut seq = [2u32, 2, 0, 4, 1, 3].into_iter();
    let order = act1::stone_order_from(|| seq.next().unwrap());
    assert_eq!(order, [18, 20, 17, 21, 19]);
    let values: Vec<u8> = order.iter().map(|o| o - 17).collect();
    assert_eq!(values, [1, 3, 0, 4, 2]);
    // Computed once, on the quest seed.
    let (mut ctl, _) = control();
    let first = act1::stone_order(&mut ctl);
    let seed = ctl.seed;
    assert_eq!(act1::stone_order(&mut ctl), first);
    assert_eq!(ctl.seed, seed);
}

#[test]
fn andariel_gem_vector() {
    assert_eq!(&act1::gem_code(&act1::CHIPPED_GEMS, 9), b"gcb ");
    assert_eq!(&act1::gem_code(&act1::CHIPPED_GEMS, 3), b"gcy ");
    assert_eq!(&act1::gem_code(&act1::NORMAL_GEMS, 13), b"sku ");
    // The kill draws three quest-seed steps, in order.
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    let victim = UnitId(0x40);
    f.chains.insert(victim, QuestChain(vec![6]));
    let mut s = ctl.seed;
    let want: Vec<String> = [&act1::CHIPPED_GEMS, &act1::CHIPPED_GEMS, &act1::NORMAL_GEMS]
        .iter()
        .map(|l| {
            format!(
                "drop {} 2",
                String::from_utf8_lossy(&act1::gem_code(l, s.step()))
            )
        })
        .collect();
    ctl.record_mut(6).unwrap().active = true;
    ctl.monster_killed(&mut f, victim, Some(P1));
    let drops: Vec<String> = f
        .log
        .iter()
        .filter(|l| l.starts_with("drop"))
        .cloned()
        .collect();
    assert_eq!(drops, want);
    assert_eq!(ctl.seed, s);
    // A player with 6.1 gets no drops.
    f.log.clear();
    f.p(P1).quests.flags[0].set(6, 1);
    ctl.monster_killed(&mut f, victim, Some(P1));
    assert!(!f.log.iter().any(|l| l.starts_with("drop")));
}

#[test]
fn flavie_draws_per_record() {
    // Chains 25 and 30 both handle event 0: two draws.
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    let mut s = f.players[&P1].seed;
    let a = s.roll(2) as u8;
    let b = s.roll(2) as u8;
    let mut list = TextList::new();
    ctl.npc_activate(&mut f, P1, NAVI_U, &mut list);
    assert_eq!(f.players[&P1].seed, s);
    let line = |st: u8| (59 + u16::from(st), 0);
    assert_eq!(list, [line(a), line(b)]);
    // Den done for the game: lo' mod 3 + 2.
    ctl.game.set(1, 13);
    let mut s = f.players[&P1].seed;
    let a = (s.step() % 3) as u8 + 2;
    let mut list = TextList::new();
    ctl.npc_activate(&mut f, P1, NAVI_U, &mut list);
    assert_eq!(list[0], line(a));
}

#[test]
fn warriv_gossip() {
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    let mut list = TextList::new();
    ctl.npc_activate(&mut f, P1, WARRIV_U, &mut list);
    assert!(list.contains(&(0, 0)));
    f.p(P1).class = act1::PALADIN;
    let mut list = TextList::new();
    ctl.npc_activate(&mut f, P1, WARRIV_U, &mut list);
    assert!(list.contains(&(1, 0)));
    ctl.quest_message(&mut f, P1, &hex("31 12000000 0100 0000")[..9]);
    assert!(f.flags(P1).get(0, 0));
}

#[test]
fn cain_rewards() {
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    f.p(P1).items.push(*b"bks ");
    ctl.quest_message(&mut f, P1, &hex("31 10000000 7000 0000")[..9]);
    f.log.retain(|l| !l.starts_with("unhandled 37")); // intro event 11
    assert_eq!(f.log, ["delete bks ", "reward bkd  0 2"]);
    assert_eq!(ctl.record(4).unwrap().state, 5);
    // 118 with 4.1: ring per difficulty, then `5D 04 02 00 0000`.
    for (d, want) in [
        (0, "reward rin  7 4"),
        (1, "reward rin  30 6"),
        (2, "reward rin  60 6"),
    ] {
        let mut f = Fake::new();
        f.difficulty = d;
        f.p(P1).quests.flags[usize::from(d)].set(4, 1);
        ctl.quest_message(&mut f, P1, &hex("31 10000000 7600 0000")[..9]);
        f.log.retain(|l| !l.starts_with("unhandled 37"));
        assert_eq!(f.log[0], want);
        // Then the text refresh.
        assert_eq!(f.sent_ids(), [0x28, 0x5D, 0x27, 0x29]);
        assert_eq!(f.sent[1].1, hex("5d 04 02 00 0000"));
        assert!(f.flags(P1).get(4, 0) && !f.flags(P1).get(4, 1));
    }
}

#[test]
fn wirt_body_gold() {
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    f.frame = 500;
    let body = UnitId(0x70);
    let mut s = ctl.seed;
    let piles = s.roll_range(10, 10);
    object_event(&mut ctl, &mut f, body, act1::WIRT_BODY);
    assert_eq!(ctl.seed, s);
    assert_eq!(f.log, ["drop gld  2", "event7 112 510"]);
    assert_eq!(ctl.record(4).unwrap().extra.wirt_piles, Some(piles - 1));
    // Runs until the piles are gone; no further draws.
    for _ in 1..piles {
        object_event(&mut ctl, &mut f, body, act1::WIRT_BODY);
    }
    assert_eq!(ctl.seed, s);
    assert_eq!(
        f.log.iter().filter(|l| l.starts_with("drop")).count() as i32,
        piles
    );
    assert_eq!(
        f.log.iter().filter(|l| l.starts_with("event7")).count() as i32,
        piles - 1
    );
}

#[test]
fn malus_level_gate() {
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    let malus = UnitId(0x71);
    f.p(P1).stats.insert(12, 7);
    act1::malus_operate(&mut ctl, &mut f, malus, P1);
    // Level 7: sound event 19, nothing drops.
    assert_eq!(f.log, ["sound 1 19"]);
    f.log.clear();
    f.p(P1).stats.insert(12, 8);
    act1::malus_operate(&mut ctl, &mut f, malus, P1);
    assert_eq!(f.log, ["drop hdm  2", "mode 113 2"]);
    let r = ctl.record(3).unwrap();
    assert_eq!((r.state, r.status, r.flags), (4, 1, 0));
    assert_eq!((r.extra.malus_mode, r.extra.malus_items), (2, 1));
    assert!(r.extra.malus_known && r.extra.malus_guid == 0x71);
    // K2 at state 4: bit 3.
    assert!(f.flags(P1).get(3, bit::LEAVE_TOWN));
}

#[test]
fn cow_portal_rules() {
    // V22: Rogue Encampment, expansion, slot 40 bit 0.
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    f.spot = Some((10, 20));
    f.p(P1).quests.flags[0].set(40, 0);
    assert!(cow_portal(&mut ctl, &mut f, P1));
    assert_eq!(f.log, ["spot 3 0x400 4 100", "portal 10 20 60 39"]);
    assert!(ctl.game.get(4, 11));
    // Opened already → refused with sound 20.
    f.log.clear();
    assert!(!cow_portal(&mut ctl, &mut f, P1));
    assert_eq!(f.log, ["sound 1 20"]);
    // V21: not in Rogue Encampment.
    let (mut ctl, _) = control();
    f.log.clear();
    f.p(P1).level = Some(40);
    assert!(!cow_portal(&mut ctl, &mut f, P1));
    // Classic needs slot 26 bit 0; Cow King killed refuses.
    f.p(P1).level = Some(1);
    f.expansion = false;
    assert!(!cow_portal(&mut ctl, &mut f, P1));
    f.p(P1).quests.flags[0].set(26, 0);
    assert!(cow_portal(&mut ctl, &mut f, P1));
    let (mut ctl, _) = control();
    f.p(P1).quests.flags[0].set(4, 10);
    assert!(!cow_portal(&mut ctl, &mut f, P1));
}

#[test]
fn act_transitions() {
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    ctl.act_completion(&mut f, P1, npc::WARRIV1).unwrap();
    let fl = f.flags(P1);
    assert!(fl.get(7, 0) && fl.get(7, 13));
    assert_eq!(f.sent_ids(), [0x28, 0x61]);
    assert_eq!(f.sent[1].1, [0x61, 2]);
    // The travel's town-leave refresh for Lut Gholein runs
    // A1Q4's act-change hook: Cain was never freed, so status
    // 5 goes out and the quest moves to state 7.
    ctl.town_leave(&mut f, P1, 40);
    assert_eq!(f.sent_ids(), [0x28, 0x61, 0x5D]);
    assert_eq!(ctl.record(4).unwrap().state, 7);
    assert_eq!(f.players[&P1].byte4c, 1);
    assert!(f.players[&P1].quests.intro[0].contains(&148));
    // Meshif: slot 10 too, staff parts deleted.
    let mut f = Fake::new();
    ctl.act_completion(&mut f, P1, npc::MESHIF1).unwrap();
    let fl = f.flags(P1);
    assert!(fl.get(10, 0) && fl.get(10, 13) && fl.get(15, 0) && fl.get(15, 13));
    assert_eq!(f.log, ["delete msf ", "delete vip "]);
    // Tyrael needs the expansion.
    let mut f = Fake::new();
    f.expansion = false;
    ctl.act_completion(&mut f, P1, npc::TYRAEL2).unwrap();
    assert!(!f.flags(P1).get(28, 0));
    // Durance warp.
    let mut f = Fake::new();
    ctl.object_warp(&mut f, P1, 102);
    let fl = f.flags(P1);
    assert!(fl.get(18, 0) && fl.get(23, 0) && fl.get(23, 13));
    assert_eq!(f.log.len(), 5);
    assert_eq!(f.sent_ids(), [0x28, 0x61]);
}

#[test]
fn warp_and_portal_checks() {
    assert_eq!(warp_check(1, 73), WarpCheck::Delegate(0x0059_DB20));
    assert_eq!(warp_check(1, 118), WarpCheck::Open);
    assert_eq!(warp_check(120, 118), WarpCheck::Delegate(0x0058_D090));
    assert_eq!(warp_check(120, 128), WarpCheck::Delegate(0x0058_D090));
    assert_eq!(warp_check(1, 3), WarpCheck::Open);
    assert_eq!(portal_check(73), WarpCheck::Delegate(0x0059_DFD0));
    assert_eq!(portal_check(74), WarpCheck::Open);
}

#[test]
fn respec_flags() {
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    act1::respec_offer(&mut f, P1);
    assert!(f.flags(P1).get(41, 13) && f.flags(P1).get(41, 1));
    ctl.record_mut(30).unwrap().active = true;
    act1::respec_done(&mut ctl, &mut f, P1);
    assert!(f.flags(P1).get(41, 0) && !f.flags(P1).get(41, 1));
    assert!(!ctl.record(30).unwrap().active);
}

#[test]
fn restore_from_bits() {
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    f.p(P1).quests.flags[0].set(2, bit::ENTER_AREA);
    f.p(P1).quests.flags[0].set(5, bit::STARTED);
    f.p(P1).quests.flags[0].set(6, bit::STARTED);
    f.p(P1).quests.flags[0].set(6, bit::COMPLETED_BEFORE);
    ctl.player_enters(&mut f, P1, 0).unwrap();
    let st = |c: u8| {
        let r = ctl.record(c).unwrap();
        (r.state, r.status)
    };
    assert_eq!(st(2), (3, 2));
    assert_eq!(st(5), (2, 1));
    // Slot 6 has bit 15: switched off and not restored.
    assert_eq!(st(6), (0, 0));
}

#[test]
fn not_intro_test_0x005444b0() {
    let (mut ctl, _) = control();
    // The quest set is not picked: the fatal.
    assert_eq!(ctl.not_intro_test(1), Err(QuestError::NotPicked));
    ctl.picked = true;
    // A quest row: its not-intro byte is 1; an intro record
    // (chains 37–40): 0; a chain without a record: true. No draw.
    let seed = ctl.seed;
    assert_eq!(ctl.not_intro_test(1), Ok(true));
    assert_eq!(ctl.not_intro_test(37), Ok(false));
    assert_eq!(ctl.not_intro_test(40), Ok(false));
    assert_eq!(ctl.not_intro_test(200), Ok(true));
    // The byte itself is what is read.
    ctl.record_mut(1).unwrap().not_intro = false;
    assert_eq!(ctl.not_intro_test(1), Ok(false));
    assert_eq!(ctl.seed, seed);
}

#[test]
fn spawn_at_unit_is_the_flags_spawn_with_spread_minus_1() {
    // At the unit's room and position, spread
    // −1, flags 0; a unit without a position spawns nothing.
    let mut f = Fake::new();
    let orb = UnitId(0x70);
    assert_eq!(f.spawn_monster_at_unit(orb, 366, 1), None);
    assert!(f.log.is_empty());
    f.pos.insert(orb, (4496, 1811, RoomId(3)));
    f.spawn_monster_at_unit(orb, 366, 1);
    assert_eq!(
        f.log,
        ["spawn 366 4496 1811 room 3 mode 1 spread -1 flags 0x0"]
    );
}

#[test]
fn den_waits_for_unpopulated_rooms_and_the_last_monster_and_completes_once() {
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    let victim = UnitId(0x40);
    f.chains.insert(victim, QuestChain(vec![1]));
    // Killing every monster in the streamed rooms cannot clear the level.
    f.den = (10, 10, 26, 27);
    ctl.monster_killed(&mut f, victim, Some(P1));
    assert!(!ctl.record(1).unwrap().extra.done);
    ctl.changed_level(&mut f, P1, 8, 2);
    ctl.changed_level(&mut f, P1, 2, 8);
    assert!(!ctl.record(1).unwrap().extra.done);
    // The final room is populated, with one monster still alive.
    f.den = (11, 10, 27, 27);
    ctl.monster_killed(&mut f, victim, Some(P1));
    assert!(!ctl.record(1).unwrap().extra.done);
    assert!(!f.flags(P1).get(1, bit::REWARD_PENDING));
    f.den = (11, 11, 27, 27);
    ctl.monster_killed(&mut f, victim, Some(P1));
    assert!(ctl.record(1).unwrap().extra.done);
    assert!(f.flags(P1).get(1, bit::REWARD_PENDING));
    let timers = ctl.timers.len();
    f.sent.clear();
    ctl.monster_killed(&mut f, victim, Some(P1));
    ctl.changed_level(&mut f, P1, 8, 2);
    ctl.changed_level(&mut f, P1, 2, 8);
    assert_eq!(ctl.record(1).unwrap().state, 4);
    assert_eq!(ctl.timers.len(), timers);
    assert!(f.sent.is_empty());
}

#[test]
fn den_empty_placeholder_cannot_complete_the_quest() {
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    let victim = UnitId(0x40);
    f.chains.insert(victim, QuestChain(vec![1]));
    f.den = (0, 0, 0, 0);
    ctl.monster_killed(&mut f, victim, Some(P1));
    assert!(!ctl.record(1).unwrap().extra.done);
}
