// Spec: specs/world/quests.md
// Spec: specs/world/quests-act1.md (§10, split out of `quests.md`)
//! Gap tests: rules of the spec not yet claimed by other tests.

#[allow(unused_imports)]
use super::*;

use super::tables::MessageEntry;
use super::tests::{control, hex, Fake, Player, P1, P2};

// ------------------------------------------------------------ §1

// Covers: specs/world/quests.md §1.2
#[test]
fn slot_bit_numbers() {
    assert_eq!(
        [
            bit::REWARD_GRANTED,
            bit::REWARD_PENDING,
            bit::STARTED,
            bit::LEAVE_TOWN,
            bit::ENTER_AREA,
            bit::CUSTOM1,
            bit::UPDATE_QUEST_LOG,
            bit::PRIMARY_GOAL_DONE,
            bit::COMPLETED_NOW,
            bit::COMPLETED_BEFORE,
        ],
        [0, 1, 2, 3, 4, 5, 12, 13, 14, 15]
    );
    // Bit b of slot q is 1 << b of the slot's word.
    let mut r = QuestFlags::default();
    r.set(4, 10); // Cow King killed by this player
    r.set(4, bit::UPDATE_QUEST_LOG);
    assert_eq!(r.word(4), (1 << 10) | (1 << 12));
}

// Covers: specs/world/quests.md §1.3
#[test]
fn slot_assignment() {
    let t = QuestTables::load().unwrap();
    let flag = |chain: u8| t.rows.iter().find(|r| r.chain == chain).unwrap().flag;
    let filter = |chain: u8| t.rows.iter().find(|r| r.chain == chain).unwrap().filter;
    // Act I 0–6, Act II 8–14, Act III 16–22, Act IV 24–27.
    for (chains, first) in [(0..=6, 0), (7..=13, 8), (14..=20, 16), (21..=24, 24)] {
        for (k, c) in chains.enumerate() {
            assert_eq!(flag(c), Some(first + k as u8), "chain {c}");
        }
    }
    assert_eq!(flag(25), Some(29)); // Flavie
    assert_eq!((flag(26), flag(27)), (Some(30), Some(31))); // A2 guards
    assert_eq!((flag(28), filter(28)), (Some(32), Some(16))); // A3Q7
    assert_eq!(flag(29), Some(33)); // Malachai
    for (k, c) in (31..=36).enumerate() {
        assert_eq!(flag(c), Some(35 + k as u8)); // Act V
    }
    assert_eq!(flag(30), Some(41)); // respec
                                    // Act-completed slots 7, 15, 23, 28 and slot 34 have no record.
    for s in [7, 15, 23, 28, 34] {
        assert!(t.rows.iter().all(|r| r.flag != Some(s)), "slot {s}");
    }
    // Intros report in slot 42 (row 40 is not disassembled).
    for c in 37..=39 {
        assert_eq!((flag(c), filter(c)), (None, Some(42)));
    }
}

// Covers: specs/world/quests.md §1.4
#[test]
fn records_follow_the_game_difficulty() {
    let (ctl, _) = control();
    let mut f = Fake::new();
    f.difficulty = 2;
    assert_eq!(ctl.quest_completed(&mut f, P1, &[0x58, 5, 0]), 0);
    let q = &f.players[&P1].quests;
    assert!(q.flags[2].get(5, bit::UPDATE_QUEST_LOG));
    assert_eq!(q.flags[0], QuestFlags::default());
    assert_eq!(q.flags[1], QuestFlags::default());
    // The NPC intro record of the same difficulty.
    set_intro_flags(&mut f, P1, 0);
    let q = &f.players[&P1].quests;
    assert!(q.intro[2].contains(&148) && q.intro[0].is_empty());
    // The game record is separate from the player's.
    assert_eq!(ctl.game, QuestFlags::default());
}

// ------------------------------------------------------------ §2

// Covers: specs/world/quests.md §2.1
#[test]
fn quest_control_fields() {
    let (mut ctl, _) = control();
    assert_eq!(ctl.records[0].chain, 40); // newest record
    assert!(!ctl.executing && !ctl.picked);
    assert_eq!(ctl.game, QuestFlags::default());
    assert!(ctl.timers.is_empty());
    assert_eq!((ctl.tick, ctl.fx), (0, 0));
    // The FX byte is what 0x89 last carried.
    let mut f = Fake::new();
    ctl.unique_event(&mut f, 7);
    assert_eq!(ctl.fx, 7);
}

// Covers: specs/world/quests.md §2.2
#[test]
fn quest_record_fields() {
    let (ctl, _) = control();
    // flag2 (+0xE4): only A1Q1 sets it, to 41.
    let with_flag2: Vec<(u8, Option<u8>)> = ctl
        .records
        .iter()
        .filter(|r| r.flag2.is_some())
        .map(|r| (r.chain, r.flag2))
        .collect();
    assert_eq!(with_flag2, [(1, Some(41))]);
    let r1 = ctl.record(1).unwrap();
    assert_eq!(
        (r1.act, r1.init_no, r1.seq_id, r1.filter),
        (0, 4, Some(2), 1)
    );
    for ev in [0, 2, 3, 8, 10, 11, 13] {
        assert!(r1.has_callback(ev), "event {ev}");
    }
    assert!(!r1.has_callback(4));
    assert_eq!(r1.status_fn, None); // null: the default rule
    let r0 = ctl.record(0).unwrap();
    assert_eq!(
        (r0.status_fn, r0.active_fn, r0.seq_fn, r0.msgs),
        (
            Some(0x0058_FB40),
            Some(0x0058_FB50),
            None,
            Some(0x0073_6460)
        )
    );
    // A3Q7 reports in slot 16; intros are not-intro 0.
    assert_eq!(ctl.record(28).unwrap().filter, 16);
    assert!(!ctl.record(37).unwrap().not_intro);
    assert!(ctl.records.iter().all(|r| r.guids.0.is_empty()));
}

// ------------------------------------------------------------ §3

// Covers: specs/world/quests.md §3 r4
#[test]
fn every_entry_sends_steps_5_to_8() {
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    // First entry in mode 1.
    ctl.player_enters(&mut f, P1, 1).unwrap();
    assert_eq!(f.sent_ids(), [0x5E, 0x28, 0x29]);
    // A later entry after the Den of Evil was cleared: 0x89 too.
    ctl.record_mut(1).unwrap().state = 4;
    f.sent.clear();
    ctl.player_enters(&mut f, P1, 0).unwrap();
    assert_eq!(f.sent_ids(), [0x5E, 0x28, 0x29, 0x89]);
    assert_eq!(f.sent[3].1, [0x89, 0x00]);
}

// ------------------------------------------------------------ §4

// Covers: specs/world/quests.md §4.1
#[test]
fn event_ids_and_arguments() {
    assert_eq!(
        [
            event::NPC_ACTIVATE,
            event::NPC_DEACTIVATE,
            event::CHANGED_LEVEL,
            event::ITEM_PICKED_UP,
            event::ITEM_DROPPED,
            event::EVENT6,
            event::MONSTER_KILLED,
            event::PLAYER_DROPPED_WITH_QUEST_ITEM,
            event::PLAYER_LEAVES_GAME,
            event::SCROLL_MESSAGE,
            event::PLAYER_STARTED_GAME,
            event::PLAYER_JOINED_GAME,
        ],
        [0, 2, 3, 4, 5, 6, 8, 9, 10, 11, 13, 14]
    );
    // Event 3 carries the old and the new level: entering level 8.
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    ctl.changed_level(&mut f, P1, 2, 8);
    assert_eq!(ctl.record(1).unwrap().state, 3);
    // Event 11 carries NPC class 0 without an NPC (GUID −1): Akara's
    // message 64 does not start the Den, the tower tome's 127 needs no NPC.
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    ctl.quest_message(&mut f, P1, &hex("31 ffffffff 4000 0000")[..9]);
    assert_eq!(ctl.record(1).unwrap().state, 1);
    ctl.quest_message(&mut f, P1, &hex("31 ffffffff 7f00 0000")[..9]);
    assert_eq!(ctl.record(5).unwrap().state, 2);
}

// Covers: specs/world/quests.md §4.2
#[test]
fn dispatch_to_all_records() {
    let msg = hex("31 10000000 4000 0000");
    // ignore_active = 1: an inactive record still gets event 11.
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    ctl.record_mut(1).unwrap().active = false;
    ctl.quest_message(&mut f, P1, &msg[..9]);
    assert_eq!(ctl.record(1).unwrap().state, 2);
    // by_act = 1: a player in Act II reaches only Act II records.
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    f.p(P1).act = Some(1);
    ctl.quest_message(&mut f, P1, &msg[..9]);
    assert_eq!(ctl.record(1).unwrap().state, 1);
    assert!(f.log.iter().all(|l| {
        let chain: u8 = l.split(' ').nth(1).unwrap().parse().unwrap();
        ctl.record(chain).unwrap().act == 1
    }));
    // An Act II record is reached: chain 13's message 444 (any NPC) sets
    // 14.9 (quests-act2.md §8.11); chain 1 still sees nothing.
    ctl.quest_message(&mut f, P1, &hex("31 10000000 bc01 0000")[..9]);
    assert!(f.flags(P1).get(14, 9));
    assert_eq!(ctl.record(1).unwrap().state, 1);
    // No room: act = −1, every record.
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    f.p(P1).act = None;
    ctl.quest_message(&mut f, P1, &msg[..9]);
    assert_eq!(ctl.record(1).unwrap().state, 2);
    // Event 3 is (1, 0): no act filter.
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    f.p(P1).act = Some(1);
    ctl.changed_level(&mut f, P1, 2, 8);
    assert_eq!(ctl.record(1).unwrap().state, 3);
}

// Covers: specs/world/quests.md §4.4 r2
#[test]
fn kill_without_killer_in_game_type_3() {
    let victim = UnitId(0x40);
    for (game_type, want) in [(3, vec![1]), (0, vec![])] {
        let (mut ctl, _) = control();
        let mut f = Fake::new();
        f.game_type = game_type;
        f.chains.insert(victim, QuestChain(vec![1]));
        f.den = (10, 3, 5, 5);
        ctl.monster_killed(&mut f, victim, None);
        assert_eq!(ctl.record(1).unwrap().extra.guids.0, want);
    }
}

/// A [`Fake`] whose player carries quest items (§4.5).
struct Leaving {
    f: Fake,
    items: Vec<(UnitId, u8)>,
}

impl QuestWorld for Leaving {
    fn tick_rate(&self) -> crate::tick::TickRate {
        crate::tick::TickRate::BASE
    }
    fn frame(&self) -> i32 {
        self.f.frame()
    }
    fn difficulty(&self) -> u8 {
        self.f.difficulty()
    }
    fn expansion(&self) -> bool {
        self.f.expansion()
    }
    fn game_type(&self) -> u8 {
        self.f.game_type()
    }
    fn has_act2(&self) -> bool {
        self.f.has_act2()
    }
    fn players(&self) -> Vec<UnitId> {
        self.f.players()
    }
    fn first_client_player(&self) -> Option<UnitId> {
        self.f.first_client_player()
    }
    fn guid(&self, unit: UnitId) -> u32 {
        self.f.guid(unit)
    }
    fn player_by_guid(&self, guid: u32) -> Option<UnitId> {
        self.f.player_by_guid(guid)
    }
    fn quests(&mut self, player: UnitId) -> Option<&mut PlayerQuests> {
        self.f.quests(player)
    }
    fn unit_act(&self, unit: UnitId) -> Option<u8> {
        self.f.unit_act(unit)
    }
    fn unit_level(&self, unit: UnitId) -> Option<u32> {
        self.f.unit_level(unit)
    }
    fn player_class(&self, player: UnitId) -> u8 {
        self.f.player_class(player)
    }
    fn unit_seed(&mut self, unit: UnitId) -> &mut Seed {
        self.f.unit_seed(unit)
    }
    fn stat(&self, unit: UnitId, stat: u16) -> i32 {
        self.f.stat(unit, stat)
    }
    fn base_stat(&self, unit: UnitId, stat: u16) -> i32 {
        self.f.base_stat(unit, stat)
    }
    fn add_stat(&mut self, unit: UnitId, stat: u16, delta: i32) {
        self.f.add_stat(unit, stat, delta)
    }
    fn attach_sound(&mut self, player: UnitId, sound: u16) {
        self.f.attach_sound(player, sound)
    }
    fn player_byte_4c(&self, player: UnitId) -> u8 {
        self.f.player_byte_4c(player)
    }
    fn set_player_byte_4c(&mut self, player: UnitId, v: u8) {
        self.f.set_player_byte_4c(player, v)
    }
    fn quest_chain(&mut self, unit: UnitId) -> Option<&mut QuestChain> {
        self.f.quest_chain(unit)
    }
    fn unit_kind(&self, unit: UnitId) -> UnitKind {
        self.f.unit_kind(unit)
    }
    fn monster_by_guid(&self, guid: u32) -> Option<(UnitId, u16)> {
        self.f.monster_by_guid(guid)
    }
    fn monster_class(&self, unit: UnitId) -> Option<u16> {
        self.f.monster_class(unit)
    }
    fn players_near(&self, unit: UnitId) -> Vec<UnitId> {
        self.f.players_near(unit)
    }
    fn party_members(&self, player: UnitId) -> Option<Vec<UnitId>> {
        self.f.party_members(player)
    }
    fn send(&mut self, player: UnitId, msg: &[u8]) {
        self.f.send(player, msg)
    }
    fn send_text_list(&mut self, player: UnitId, npc: UnitId, list: &[(u16, u32)]) {
        self.f.send_text_list(player, npc, list)
    }
    fn has_item(&self, player: UnitId, code: [u8; 4]) -> bool {
        self.f.has_item(player, code)
    }
    fn delete_item(&mut self, player: UnitId, code: [u8; 4]) {
        self.f.delete_item(player, code)
    }
    fn reward_item(
        &mut self,
        player: UnitId,
        code: [u8; 4],
        level: i32,
        quality: u8,
        droppable: bool,
    ) -> Option<UnitId> {
        self.f.reward_item(player, code, level, quality, droppable)
    }
    fn drop_item_at(&mut self, unit: UnitId, code: [u8; 4], quality: u8) -> bool {
        self.f.drop_item_at(unit, code, quality)
    }
    fn quest_items(&self, _: UnitId) -> Vec<(UnitId, u8)> {
        self.items.clone()
    }
    fn den_region(&self) -> (u32, u32, u32, u32) {
        self.f.den_region()
    }
    fn true_tomb_level(&self) -> u32 {
        self.f.true_tomb_level()
    }
    fn free_spot(
        &mut self,
        player: UnitId,
        size: u32,
        mask: u32,
        radius: u32,
        limit: u32,
    ) -> Option<(i32, i32)> {
        self.f.free_spot(player, size, mask, radius, limit)
    }
    fn create_portal(&mut self, player: UnitId, x: i32, y: i32, class: u16, level: u32) -> bool {
        self.f.create_portal(player, x, y, class, level)
    }
    fn schedule_quest_event(&mut self, object: UnitId, frame: i32) {
        self.f.schedule_quest_event(object, frame)
    }
    fn object_mode(&self, object: UnitId) -> i32 {
        self.f.object_mode(object)
    }
    fn set_object_mode(&mut self, object: UnitId, mode: i32) {
        self.f.set_object_mode(object, mode)
    }
    fn object_by_guid(&self, guid: u32) -> Option<(UnitId, u16)> {
        self.f.object_by_guid(guid)
    }
    fn mercenary_reward(&mut self, player: UnitId, npc: u16) {
        self.f.mercenary_reward(player, npc)
    }
    fn unit_position(&self, u: UnitId) -> Option<(i32, i32, crate::units::RoomId)> {
        self.f.unit_position(u)
    }
    fn room_contains(&self, r: crate::units::RoomId, x: i32, y: i32) -> bool {
        self.f.room_contains(r, x, y)
    }
    fn room_at(&self, r: crate::units::RoomId, x: i32, y: i32) -> Option<crate::units::RoomId> {
        self.f.room_at(r, x, y)
    }
    fn free_spot_at(
        &mut self,
        r: crate::units::RoomId,
        x: i32,
        y: i32,
        s: u32,
        m: u32,
        rad: u32,
        l: u32,
    ) -> Option<(i32, i32, crate::units::RoomId)> {
        self.f.free_spot_at(r, x, y, s, m, rad, l)
    }
    fn spawn_monster(
        &mut self,
        r: crate::units::RoomId,
        x: i32,
        y: i32,
        c: u16,
        mode: u8,
        rad: u32,
    ) -> Option<UnitId> {
        self.f.spawn_monster(r, x, y, c, mode, rad)
    }
    fn or_unit_flags(&mut self, u: UnitId, f: u32) {
        self.f.or_unit_flags(u, f)
    }
    fn monsters(&self) -> Vec<UnitId> {
        self.f.monsters()
    }
    fn npc_chat_clients(&self, n: UnitId) -> Option<Vec<UnitId>> {
        self.f.npc_chat_clients(n)
    }
    fn remove_monster(&mut self, m: UnitId) {
        self.f.remove_monster(m)
    }
    fn drop_preset_monster(&mut self, a: u8, c: u16) {
        self.f.drop_preset_monster(a, c)
    }
    fn find_object_near(&self, o: UnitId, c: u16) -> Option<UnitId> {
        self.f.find_object_near(o, c)
    }
    fn create_object(&mut self, r: crate::units::RoomId, x: i32, y: i32, c: u16) -> Option<UnitId> {
        self.f.create_object(r, x, y, c)
    }
    fn object_anim_length(&self, o: UnitId) -> i32 {
        self.f.object_anim_length(o)
    }
    fn schedule_object_event(&mut self, o: UnitId, ev: u8, frame: i32) {
        self.f.schedule_object_event(o, ev, frame)
    }
    fn open_quest_message(&mut self, p: UnitId, o: UnitId, m: u16) {
        self.f.open_quest_message(p, o, m)
    }
    fn item_code(&self, i: UnitId) -> Option<[u8; 4]> {
        self.f.item_code(i)
    }
    fn unhandled(&mut self, chain: u8, function: u32) {
        self.f.unhandled(chain, function)
    }
    fn spawn_monster_flags(
        &mut self,
        r: crate::units::RoomId,
        x: i32,
        y: i32,
        c: u16,
        m: u8,
        s: i32,
        fl: u32,
    ) -> Option<UnitId> {
        self.f.spawn_monster_flags(r, x, y, c, m, s, fl)
    }
    fn open_portal(
        &mut self,
        o: Option<UnitId>,
        r: crate::units::RoomId,
        x: i32,
        y: i32,
        l: u32,
        c: u16,
        e: bool,
    ) -> Option<UnitId> {
        self.f.open_portal(o, r, x, y, l, c, e)
    }
    fn create_missile(
        &mut self,
        o: UnitId,
        s: u16,
        l: u8,
        c: u16,
        x: i32,
        y: i32,
    ) -> Option<UnitId> {
        self.f.create_missile(o, s, l, c, x, y)
    }
    fn set_missile_target(&mut self, m: UnitId, a: u32, b: u32) {
        self.f.set_missile_target(m, a, b)
    }
    fn refresh_room(&mut self, u: UnitId) {
        self.f.refresh_room(u)
    }
    fn spawn_object(
        &mut self,
        r: crate::units::RoomId,
        x: i32,
        y: i32,
        c: u16,
        m: i32,
    ) -> Option<UnitId> {
        self.f.spawn_object(r, x, y, c, m)
    }
    fn client_save_flags(&self, p: UnitId) -> Option<u16> {
        self.f.client_save_flags(p)
    }
    fn set_client_save_flags(&mut self, p: UnitId, fl: u16) {
        self.f.set_client_save_flags(p, fl)
    }
}

// Covers: specs/world/quests.md §4.5
#[test]
fn player_leaving_with_quest_items() {
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    f.p(P1).act = Some(3); // event 10 is not filtered by act
    let mut w = Leaving {
        f,
        // quest 4 → chain 3 (callback 9 `0x00591A20`); quest 99 → no record.
        items: vec![(UnitId(0x80), 4), (UnitId(0x81), 99)],
    };
    // Chains 1–6 have bodies (§10.4–§10.8): the chain 1–3 lists lose P1.
    // So do the Act II chains with bodies (quests-act2.md §1.1 event 10);
    // their removal `0x00545530` needs s.0 and s.1 in the leaving player's
    // record (quests-act2-2.md §1 item 2), so P1 has them in slots 9–14.
    const BODIES: [u8; 7] = [7, 8, 9, 10, 11, 12, 13];
    ctl.record_mut(7).unwrap().extra.a2.q0.add(1);
    for c in [8, 9, 10, 11, 12, 13] {
        ctl.record_mut(c).unwrap().guids.add(1);
        w.f.p(P1).quests.flags[0].set(c + 1, bit::REWARD_GRANTED);
        w.f.p(P1).quests.flags[0].set(c + 1, bit::REWARD_PENDING);
    }
    ctl.record_mut(1).unwrap().guids.add(1);
    ctl.record_mut(2).unwrap().guids.add(1);
    ctl.record_mut(3).unwrap().extra.guids.add(1);
    // Chains 22 and 24 have bodies too (quests-act4.md §3.7, §4.5).
    ctl.record_mut(22).unwrap().guids.add(1);
    ctl.record_mut(24).unwrap().guids.add(1);
    // Chains 34 and 36 (quests-act5-2.md §6.5, §8.7) lose P1 from both
    // lists, chain 35 (§7.5) from the record list.
    for c in [34, 35, 36] {
        ctl.record_mut(c).unwrap().guids.add(1);
    }
    ctl.record_mut(34).unwrap().extra.a5.q4.guids.add(1);
    ctl.record_mut(36).unwrap().extra.a5.q6.guids.add(1);
    // Chains 31–33 have bodies too (quests-act5.md §3.7, §4.9, §5.10).
    for c in 31..=33 {
        ctl.record_mut(c).unwrap().guids.add(1);
    }
    ctl.record_mut(32).unwrap().extra.a5.q2.guids.add(1);
    ctl.record_mut(33).unwrap().extra.a5.q3.guids.add(1);
    // Act III chains 15, 18, 19, 20 remove P1 from their lists
    // (`quests-act3.md` §1.1); chain 16's event 10 is a bare `ret`.
    for c in [15, 16, 18, 19, 20] {
        ctl.record_mut(c).unwrap().guids.add(1);
    }
    ctl.player_leaves(&mut w, P1);
    for c in [34, 35, 36] {
        assert!(!ctl.record(c).unwrap().guids.contains(1), "chain {c}");
    }
    assert!(!ctl.record(34).unwrap().extra.a5.q4.guids.contains(1));
    assert!(!ctl.record(36).unwrap().extra.a5.q6.guids.contains(1));
    assert!(!ctl.record(22).unwrap().guids.contains(1));
    assert!(!ctl.record(24).unwrap().guids.contains(1));
    for c in [15, 18, 19, 20] {
        assert!(!ctl.record(c).unwrap().guids.contains(1), "chain {c}");
    }
    assert!(ctl.record(16).unwrap().guids.contains(1));
    let fn_of = |chain: u8, ev: u8| {
        ctl.rows
            .iter()
            .find(|r| r.chain == chain)
            .and_then(|r| r.callbacks.iter().find(|c| c.0 == ev))
            .unwrap()
            .1
    };
    // Chain 3's callback 9 (`0x00591A20`) ran first: one Malus fewer.
    assert_eq!(ctl.record(3).unwrap().extra.malus_items, -1);
    assert!(!ctl.record(1).unwrap().guids.contains(1));
    assert!(!ctl.record(2).unwrap().guids.contains(1));
    assert!(!ctl.record(3).unwrap().extra.guids.contains(1));
    for c in 31..=33 {
        assert!(!ctl.record(c).unwrap().guids.contains(1));
    }
    assert!(!ctl.record(32).unwrap().extra.a5.q2.guids.contains(1));
    assert!(!ctl.record(33).unwrap().extra.a5.q3.guids.contains(1));
    assert!(!ctl.record(7).unwrap().extra.a2.q0.contains(1));
    for c in [8, 9, 10, 11, 12, 13] {
        assert!(!ctl.record(c).unwrap().guids.contains(1), "chain {c}");
    }
    let mut want = Vec::new();
    for r in &ctl.records {
        if r.has_callback(event::PLAYER_LEAVES_GAME)
            && !(1..=6).contains(&r.chain)
            && !BODIES.contains(&r.chain)
            && !(14..=20).contains(&r.chain)
            && !matches!(r.chain, 22 | 24 | 31..=36)
        {
            want.push(format!(
                "unhandled {} {:#x}",
                r.chain,
                fn_of(r.chain, event::PLAYER_LEAVES_GAME)
            ));
        }
    }
    // Every event-10 callback of Acts I–V has a body now: none reported.
    assert!(want.is_empty(), "{want:?}");
    assert_eq!(w.f.log, want);
}

// Covers: specs/world/quests.md §edge-cases-original-bugs r3
#[test]
fn pick_up_and_drop_reach_only_active_records() {
    let item = UnitId(0x80);
    // Chain 9 (Act II) has callbacks 4 (`0x00599A30`: a `tr1 ` with 10.3
    // clear sets the record status to 1) and 5 (`0x00599B30`: a `vip `
    // clears 10.4), quests-act2.md §4.8.
    for (ev, code) in [
        (event::ITEM_PICKED_UP, *b"tr1 "),
        (event::ITEM_DROPPED, *b"vip "),
    ] {
        let (mut ctl, _) = control();
        let mut f = Fake::new();
        f.chains.insert(item, QuestChain(vec![9]));
        f.item_codes.insert(item, code);
        f.p(P1).quests.flags[0].set(10, 4);
        // Chain 9 is active from its init (quests-act2.md §2); switch it
        // off to test the gate.
        ctl.record_mut(9).unwrap().active = false;
        ctl.item_event(&mut f, ev, P1, item);
        assert!(f.log.is_empty());
        assert_eq!(ctl.record(9).unwrap().status, 13);
        assert!(f.flags(P1).get(10, 4));
        ctl.record_mut(9).unwrap().active = true;
        ctl.item_event(&mut f, ev, P1, item);
        assert!(f.log.is_empty());
        if ev == event::ITEM_PICKED_UP {
            assert_eq!(ctl.record(9).unwrap().status, 1);
            assert!(f.flags(P1).get(10, 4));
        } else {
            assert_eq!(ctl.record(9).unwrap().status, 13);
            assert!(!f.flags(P1).get(10, 4));
        }
    }
}

// ------------------------------------------------------------ §5

// Covers: specs/world/quests.md §5 r4
#[test]
fn updater_clears_executing() {
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    ctl.add_timer(9, TimerFn::Probe, 0).unwrap();
    ctl.update(&mut f);
    assert_eq!(f.log.len(), 1); // the timer ran
    assert!(!ctl.executing);
    assert!(ctl.add_timer(9, TimerFn::Probe, 1).is_ok());
}

// ------------------------------------------------------------ §6

// Covers: specs/world/quests.md §6.1 text
#[test]
fn status_function_false_reports_nothing() {
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    // A1Q0's status function returns false: slot 0 stays 0.
    ctl.record_mut(0).unwrap().status = 3;
    // A1Q1 has none: the default rule reports (s < n, L = 1 → 1).
    let r = ctl.record_mut(1).unwrap();
    (r.status, r.state) = (1, 2);
    ctl.request_quest_data(&mut f, P1).unwrap();
    let m = &f.sent.last().unwrap().1;
    assert_eq!(m[0], 0x52);
    assert_eq!((m[1], m[2]), (0, 1));
}

// Covers: specs/world/quests.md §6.6
#[test]
fn player_flags_message() {
    let mut f = Fake::new();
    f.difficulty = 1;
    f.p(P1).quests.flags[1].set(1, 2);
    f.p(P1).quests.flags[0].set(2, 2); // another difficulty: not sent
    send_player_flags(&mut f, P1, 6, 0);
    let mut want = hex("28 06 00000000 00");
    let mut rec = [0u8; 96];
    rec[2] = 0x04;
    want.extend(rec);
    assert_eq!(f.sent, [(P1, want)]);
}

// Covers: specs/world/quests.md §edge-cases-original-bugs r8
#[test]
fn quest_data_unused_counters_are_zero() {
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    let r = ctl.record_mut(1).unwrap();
    (r.status, r.state) = (1, 2);
    r.extra.monsters_left = 0x1234;
    ctl.request_quest_data(&mut f, P1).unwrap();
    let m = &f.sent[1].1;
    assert_eq!(m.len(), 15);
    assert_eq!(m[..5], hex("50 0100 3412")[..]);
    assert_eq!(m[5..], [0u8; 10]);
}

// ------------------------------------------------------------ §7

// Covers: specs/world/quests.md §7.1
#[test]
fn npc_message_table_lookup() {
    let (mut ctl, _) = control();
    let e = |state, slot, npc, string, menu| MessageEntry {
        table: 0x1000,
        state,
        slot,
        npc,
        string,
        menu,
    };
    ctl.messages = vec![
        e(1, 2, 148, 30, 0),
        e(1, 0, 148, 10, 1), // menu 1 → 0
        e(1, 1, 150, 20, 5), // another NPC
        e(2, 3, 148, 40, 5), // another state
        e(1, 1, 148, 15, 7),
        MessageEntry {
            table: 0x2000,
            ..e(1, 4, 148, 50, 0)
        },
    ];
    let mut list = TextList::new();
    ctl.add_messages(0x1000, &mut list, 148, 1);
    assert_eq!(list, [(10, 0), (15, 7), (30, 0)]);
    // The 1.14d tables: at most 16 entries per state, slots unique.
    let t = QuestTables::load().unwrap();
    let mut per_state = std::collections::BTreeMap::new();
    for m in &t.messages {
        per_state
            .entry((m.table, m.state))
            .or_insert_with(Vec::new)
            .push(m.slot);
    }
    for (k, mut slots) in per_state {
        assert!(slots.len() <= 16 && slots.iter().all(|&s| s < 16), "{k:?}");
        let n = slots.len();
        slots.sort_unstable();
        slots.dedup();
        assert_eq!(slots.len(), n, "{k:?}");
    }
}

// ------------------------------------------------------------ §9

// Covers: specs/world/quests.md §9.3
#[test]
fn guid_lists_and_pending_grant() {
    let mut l = GuidList::default();
    for g in 0..40 {
        l.add(g);
    }
    l.add(5);
    assert_eq!(l.0.len(), 32);
    l.remove(0);
    assert_eq!(l.0[0], 31);
    assert!(!l.contains(0) && l.contains(31));
    // 0x005455F0: list order; missing players and players with bit 0 or
    // 1 are skipped.
    let mut f = Fake::new();
    f.players.insert(
        P2,
        Player {
            guid: 2,
            ..Player::default()
        },
    );
    f.p(P1).quests.flags[0].set(2, bit::REWARD_GRANTED);
    let list = GuidList(vec![2, 99, 1]);
    grant_pending(&mut f, &list, 2, 34);
    assert!(f.flags(P2).get(2, 13) && f.flags(P2).get(2, 1));
    assert!(!f.flags(P1).get(2, 13) && !f.flags(P1).get(2, 1));
    assert_eq!(f.log, ["sound 2 34"]);
    // Sound 0: none attached.
    let mut f = Fake::new();
    grant_pending(&mut f, &GuidList(vec![1]), 2, 0);
    assert!(f.flags(P1).get(2, 1) && f.log.is_empty());
}

// Covers: specs/world/quests.md §9.5
#[test]
fn object_quest_functions_by_class() {
    let obj = UnitId(0x70);
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    object_event(&mut ctl, &mut f, obj, 0x10C); // Wirt's body
    assert!(f.log[0].starts_with("drop gld"));
    // Each class's function (the record's chain, or 255 without one);
    // the object has no room here: 0x83 does nothing, 0xBD is not in Act I.
    // Classes 0x16F (lever) and 0x155 (bridge) run Act III code, tested in
    // `act3_tests` (`lever_event`, `bridge_event`).
    // 0x1CD (dummy 461, `0x00589540`, `quests-act5.md` §5.9): no monster
    // with GUID +0x9C → nothing.
    let mut f = Fake::new();
    object_event(&mut ctl, &mut f, obj, 0x1CD);
    assert!(f.log.is_empty());
    // 0x178, the Hellforge (`0x005B6710`, quests-act4.md §4.7): a fresh
    // record (nothing smashed, no gems pending) does nothing; once
    // smashed, mode 4.
    let mut f = Fake::new();
    object_event(&mut ctl, &mut f, obj, 0x178);
    assert!(f.log.is_empty() && f.sent.is_empty());
    ctl.record_mut(24).unwrap().extra.a4.q3.smashed = true;
    object_event(&mut ctl, &mut f, obj, 0x178);
    assert_eq!(f.log, ["mode 112 4"]);
    ctl.record_mut(24).unwrap().extra.a4.q3.smashed = false;
    // 0x1CB (dummy 459, quests-act5-2.md §6.7): nothing until chain 34
    // wants the temple portal; then the portal at the dummy + (10, 5).
    let mut f = Fake::new();
    object_event(&mut ctl, &mut f, obj, 0x1CB);
    assert!(f.log.is_empty());
    ctl.record_mut(34).unwrap().extra.a5.q4.portal_wanted = true;
    f.pos.insert(obj, (100, 200, crate::units::RoomId(1)));
    object_event(&mut ctl, &mut f, obj, 0x1CB);
    assert_eq!(f.log, ["portal 110 205 60 121"]);
    // 0x1DA–0x1DC (the statues, §7.6): each releases its superunique.
    for (class, su) in [(0x1DA, 45), (0x1DB, 43), (0x1DC, 44)] {
        let (mut ctl, _) = control();
        let mut f = Fake::new();
        f.objects.insert(obj, (0x70, class, 3));
        f.a5_superuniques = vec![Some(UnitId(0x90))];
        object_event(&mut ctl, &mut f, obj, class);
        assert_eq!(
            f.log,
            [format!("superunique 112 {su}"), "mode 112 4".into()]
        );
    }
    // 0xBD outside Act I, levels 109 / ≥ 113 excluded: chain 32's rescue
    // portal `0x00588CA0` (quests-act5.md §4.8): mode 1 → 2, event 7
    // again at frame + 25, for a group's portal only.
    let mut f = Fake::new();
    f.objects.insert(obj, (0x70, 0xBD, 1));
    object_event(&mut ctl, &mut f, obj, 0xBD);
    assert!(f.log.is_empty());
    ctl.record_mut(32).unwrap().extra.a5.q2.portal_guid[1] = 0x70;
    object_event(&mut ctl, &mut f, obj, 0xBD);
    assert_eq!(f.log, ["mode 112 2", "event7 112 25"]);
    ctl.record_mut(32).unwrap().extra.a5.q2.portal_guid[1] = 0;
    // 0x1CC: dummy 460 `0x0058A500` (§5.6): object 558 at the dummy,
    // else event 7 again at frame + 25.
    let mut f = Fake::new();
    f.pos.insert(obj, (3, 4, crate::units::RoomId(2)));
    object_event(&mut ctl, &mut f, obj, 0x1CC);
    assert_eq!(f.log, ["place 558 3 4 room 2 [1, 0, 0]", "event7 112 25"]);
    // 0x83 in a room: mode 1 → 2; level 76 → `0x005B23C0`.
    let mut f = Fake::new();
    f.objects.insert(obj, (0x70, 0x83, 1));
    f.players.insert(
        obj,
        Player {
            level: Some(76),
            ..Player::default()
        },
    );
    object_event(&mut ctl, &mut f, obj, 0x83);
    assert_eq!(f.log, ["mode 112 2", "unhandled 255 0x5b23c0"]);
    // 0xBD in Act I: chain 4's `0x005942C0` (stated since
    // `quests-act1-rest.md` §9 item 10): mode 0 changes nothing, event 7
    // again at frame + 25.
    let mut f = Fake::new();
    f.players.insert(
        obj,
        Player {
            level: Some(5),
            act: Some(0),
            ..Player::default()
        },
    );
    object_event(&mut ctl, &mut f, obj, 0xBD);
    assert_eq!(f.log, ["event7 112 25"]);
    for class in [0x10B, 0x1CE, 0x1DD, 0] {
        let mut f = Fake::new();
        object_event(&mut ctl, &mut f, obj, class);
        assert!(f.log.is_empty() && f.sent.is_empty(), "class {class:#x}");
    }
}
