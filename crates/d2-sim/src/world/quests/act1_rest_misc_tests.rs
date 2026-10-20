// Spec: specs/world/quests-act1-rest.md
use super::tests::*;
use super::*;
use crate::units::RoomId as RoomIdT;

// ------------------------------------------------------------ helpers

fn player(guid: u32, level: Option<u32>) -> Player {
    Player {
        guid,
        act: Some(0),
        level,
        ..Player::default()
    }
}

/// Calls one callback of `chain` directly.
fn call<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, chain: u8, args: EventArgs) {
    let i = ctl.find(chain).unwrap();
    act1::callback(ctl, w, i, args, None, false);
}

fn kill(ctl: &mut QuestControl, f: &mut Fake, chain: u8, victim: UnitId, killer: UnitId) {
    f.chains.insert(victim, QuestChain(vec![chain]));
    ctl.monster_killed(f, victim, Some(killer));
}

fn q5_mut(ctl: &mut QuestControl) -> &mut act1::q5::Extra5 {
    &mut ctl.record_mut(5).unwrap().extra.q5
}

fn q5(ctl: &QuestControl) -> &act1::q5::Extra5 {
    &ctl.record(5).unwrap().extra.q5
}

/// The fake world with failing item drops and failing portal creation
/// (both still logged); everything else is the fake's.
struct Failing {
    f: Fake,
}

impl QuestWorld for Failing {
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
        // The drop is logged, then fails.
        self.f.drop_item_at(unit, code, quality);
        false
    }
    fn quest_items(&self, u: UnitId) -> Vec<(UnitId, u8)> {
        self.f.quest_items(u)
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
        // The creation is logged, then fails.
        self.f.create_portal(player, x, y, class, level);
        false
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

// ------------------------------------------------------------ §4 trap

const C1: UnitId = UnitId(0x72);
const C2: UnitId = UnitId(0x73);
const TRAP: UnitId = UnitId(0x50);

/// Two chests (GUIDs 0x72, 0x73) in room 2 (40..100); the Countess's
/// death position (7, 8) in room 1 (0..40); the Countess killed.
fn chest_fake(ctl: &mut QuestControl) -> Fake {
    let mut f = Fake::new();
    f.rooms.insert(RoomIdT(1), (0, 0, 40, 40));
    f.rooms.insert(RoomIdT(2), (40, 40, 100, 100));
    for (c, x, y) in [(C1, 50, 60), (C2, 70, 80)] {
        f.objects.insert(c, (c.0, 0x173, 0));
        f.pos.insert(c, (x, y, RoomIdT(2)));
    }
    let x = q5_mut(ctl);
    x.chests = vec![0x72, 0x73];
    x.killed = true;
    x.death_pos = (7, 8);
    f
}

/// The two chests' missiles owned by the trap (ids 0x9001, 0x9002).
fn chest_missiles() -> Vec<String> {
    [
        "missile 332 owner 80 skill 0 level 1 50 60",
        "missile data 36865 0x72 0",
        "refresh 36865",
        "missile 332 owner 80 skill 0 level 1 70 80",
        "missile data 36866 0x73 0",
        "refresh 36866",
    ]
    .map(String::from)
    .to_vec()
}

// Covers: specs/world/quests-act1-rest.md §4 text, §4 r1, §edge-cases-original-bugs r1, §edge-cases-original-bugs r2
#[test]
fn countess_trap_vector() {
    // Vector: 2 chests, killed, trapped 0, the first spawn at the death
    // position succeeds → 1 monster 326 in the death position's room,
    // a missile 332 per chest (data +0x28 = chest GUID), E +0x119 = 1.
    let (mut ctl, _) = control();
    let mut f = chest_fake(&mut ctl);
    f.spawns = vec![Some(TRAP)];
    act1::q5::chest_event(&mut ctl, &mut f, C1);
    let mut want = vec!["spawn 326 7 8 room 1 mode 12 spread -1 flags 0x8".to_string()];
    want.extend(chest_missiles());
    assert_eq!(f.log, want);
    assert!(q5(&ctl).trap_spawned);
    assert!(f.sent.is_empty());
    // Vector: the trap step again → nothing (no spawn, no missile, no
    // reschedule).
    f.log.clear();
    f.spawns = vec![Some(UnitId(0x51))];
    act1::q5::chest_event(&mut ctl, &mut f, C2);
    act1::q5::chest_init(&mut ctl, &mut f, C1);
    assert!(f.log.is_empty());
    assert_eq!(q5(&ctl).chests, [0x72, 0x73]);
}

// Covers: specs/world/quests-act1-rest.md §4 text, §4 r1, §edge-cases-original-bugs r2
#[test]
fn countess_kill_runs_the_trap() {
    // The chests are listed before the kill (not killed: nothing); the
    // Countess's event 8 stores her death position and runs the trap
    // step with it.
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    f.rooms.insert(RoomIdT(1), (0, 0, 40, 40));
    f.rooms.insert(RoomIdT(2), (40, 40, 100, 100));
    for (c, x, y) in [(C1, 50, 60), (C2, 70, 80)] {
        f.objects.insert(c, (c.0, 0x173, 0));
        f.pos.insert(c, (x, y, RoomIdT(2)));
        act1::q5::chest_init(&mut ctl, &mut f, c);
    }
    assert!(f.log.is_empty());
    assert_eq!(q5(&ctl).chests, [0x72, 0x73]);
    let countess = UnitId(0x40);
    f.pos.insert(countess, (7, 8, RoomIdT(1)));
    f.spawns = vec![Some(TRAP)];
    ctl.record_mut(5).unwrap().state = 3;
    kill(&mut ctl, &mut f, 5, countess, P1);
    let x = q5(&ctl);
    assert!(x.killed && x.trap_spawned && x.death_pos == (7, 8));
    let mut want = vec!["spawn 326 7 8 room 1 mode 12 spread -1 flags 0x8".to_string()];
    want.extend(chest_missiles());
    // Trapped: the Countess's event 7 is not rescheduled.
    assert_eq!(f.log, want);
}

// Covers: specs/world/quests-act1-rest.md §4 r2, §4 text
#[test]
fn countess_trap_retry_at_the_chest() {
    // No room holds the death position: the retry at chest + (5, 5).
    let (mut ctl, _) = control();
    let mut f = chest_fake(&mut ctl);
    q5_mut(&mut ctl).death_pos = (500, 500);
    f.spawns = vec![Some(TRAP)];
    act1::q5::chest_event(&mut ctl, &mut f, C1);
    let mut want = vec!["spawn 326 55 65 room 2 mode 12 spread -1 flags 0x8".to_string()];
    want.extend(chest_missiles());
    assert_eq!(f.log, want);
    assert!(q5(&ctl).trap_spawned);

    // A room, but the spawn there fails: the same retry.
    let (mut ctl, _) = control();
    let mut f = chest_fake(&mut ctl);
    f.spawns = vec![None, Some(TRAP)];
    act1::q5::chest_event(&mut ctl, &mut f, C1);
    let mut want = vec![
        "spawn 326 7 8 room 1 mode 12 spread -1 flags 0x8".to_string(),
        "spawn 326 55 65 room 2 mode 12 spread -1 flags 0x8".to_string(),
    ];
    want.extend(chest_missiles());
    assert_eq!(f.log, want);

    // Both tries fail for the first chest: nothing else for it; the
    // second chest tries again from the death position and gets the
    // only missile.
    let (mut ctl, _) = control();
    let mut f = chest_fake(&mut ctl);
    f.spawns = vec![None, None, Some(TRAP)];
    act1::q5::chest_event(&mut ctl, &mut f, C1);
    assert_eq!(
        f.log,
        [
            "spawn 326 7 8 room 1 mode 12 spread -1 flags 0x8",
            "spawn 326 55 65 room 2 mode 12 spread -1 flags 0x8",
            "spawn 326 7 8 room 1 mode 12 spread -1 flags 0x8",
            "missile 332 owner 80 skill 0 level 1 70 80",
            "missile data 36865 0x73 0",
            "refresh 36865",
        ]
    );
    assert!(q5(&ctl).trap_spawned);
}

// Covers: specs/world/quests-act1-rest.md §4 text, §4 r1
#[test]
fn countess_trap_skips_missing_chests() {
    // A listed GUID without an object is skipped: no spawn try, no
    // missile for it.
    let (mut ctl, _) = control();
    let mut f = chest_fake(&mut ctl);
    q5_mut(&mut ctl).chests = vec![0x99, 0x73];
    f.spawns = vec![Some(TRAP)];
    act1::q5::chest_event(&mut ctl, &mut f, C2);
    assert_eq!(
        f.log,
        [
            "spawn 326 7 8 room 1 mode 12 spread -1 flags 0x8",
            "missile 332 owner 80 skill 0 level 1 70 80",
            "missile data 36865 0x73 0",
            "refresh 36865",
        ]
    );
    // Only missing chests: nothing tried, not trapped, event 7 again.
    let (mut ctl, _) = control();
    let mut f = chest_fake(&mut ctl);
    q5_mut(&mut ctl).chests = vec![0x99];
    f.frame = 30;
    f.spawns = vec![Some(TRAP)];
    act1::q5::chest_event(&mut ctl, &mut f, C1);
    assert_eq!(f.log, ["event7 114 40"]);
    assert!(!q5(&ctl).trap_spawned);
}

// Covers: specs/world/quests-act1-rest.md §4 r1, §4 r2
#[test]
fn countess_trap_reschedule_stops_once_trapped() {
    // Every spawn fails: the chest's event 7 at frame + 10, each run.
    let (mut ctl, _) = control();
    let mut f = chest_fake(&mut ctl);
    f.frame = 100;
    act1::q5::chest_event(&mut ctl, &mut f, C1);
    let spawn_try = [
        "spawn 326 7 8 room 1 mode 12 spread -1 flags 0x8",
        "spawn 326 55 65 room 2 mode 12 spread -1 flags 0x8",
        "spawn 326 7 8 room 1 mode 12 spread -1 flags 0x8",
        "spawn 326 75 85 room 2 mode 12 spread -1 flags 0x8",
    ];
    let mut want: Vec<String> = spawn_try.map(String::from).to_vec();
    want.push("event7 114 110".into());
    assert_eq!(f.log, want);
    assert!(!q5(&ctl).trap_spawned);
    // The next run succeeds: missiles, no reschedule.
    f.log.clear();
    f.frame = 110;
    f.spawns = vec![Some(TRAP)];
    act1::q5::chest_event(&mut ctl, &mut f, C1);
    let mut want = vec!["spawn 326 7 8 room 1 mode 12 spread -1 flags 0x8".to_string()];
    want.extend(chest_missiles());
    assert_eq!(f.log, want);
    // Then nothing at all.
    f.log.clear();
    act1::q5::chest_event(&mut ctl, &mut f, C1);
    assert!(f.log.is_empty());
    // Not killed: no trap step and no reschedule.
    let (mut ctl, _) = control();
    let mut f = chest_fake(&mut ctl);
    q5_mut(&mut ctl).killed = false;
    f.spawns = vec![Some(TRAP)];
    act1::q5::chest_event(&mut ctl, &mut f, C1);
    assert!(f.log.is_empty() && !q5(&ctl).trap_spawned);
}

// ------------------------------------------------------------ §5

// Covers: specs/world/quests-act1-rest.md §5 text, §5 r1, §5 r2, §5 r3
#[test]
fn progression_vectors() {
    // Expansion, p 5, step 1, difficulty 2: n = 5 · 2 + 1 = 11.
    assert_eq!(progression(0x0520, 1, 2), 0x0B20);
    // Classic, step 1, difficulty 1: n = 4 · 1 + 1 = 5.
    assert_eq!(progression(0x0000, 1, 1), 0x0500);
    // Expansion, p 7 > n 1: unchanged.
    assert_eq!(progression(0xE720, 1, 0), 0xE720);
    // p = n: rewritten with the same value.
    assert_eq!(progression(0x0120, 1, 0), 0x0120);
    // Bits 0–7 and 13–15 kept.
    assert_eq!(progression(0xE0FF, 1, 2), 0xEBFF);
}

// Covers: specs/world/quests-act1-rest.md §5 text, §5 r3
#[test]
fn raise_progression_needs_a_client() {
    let mut f = Fake::new();
    raise_progression(&mut f, P1, 1, 0);
    assert!(f.log.is_empty() && f.client_flags.is_empty());
    f.client_flags.insert(P1, 0x0520);
    raise_progression(&mut f, P1, 1, 2);
    assert_eq!(f.log, ["progression 1 0x0b20"]);
    assert_eq!(f.client_flags[&P1], 0x0B20);
    // Never lowered: the same write with p above n.
    f.log.clear();
    raise_progression(&mut f, P1, 1, 0);
    assert_eq!(f.log, ["progression 1 0x0b20"]);
    assert_eq!(f.client_flags[&P1], 0x0B20);
}

// ------------------------------------------------------------ §8

// Covers: specs/world/quests-act1-rest.md §8 r2
#[test]
fn tree_operate_failed_drop() {
    // Vector: drop fails, state 3 → state 4; no 0x5D; object mode 0;
    // +0x47 = 1; +0x30 = the object's GUID.
    let (mut ctl, _) = control();
    let mut w = Failing { f: Fake::new() };
    let tree = UnitId(0x61);
    w.f.objects.insert(tree, (0x61, 30, 0));
    let r = ctl.record_mut(4).unwrap();
    r.state = 3;
    r.clear_callback(event::PLAYER_DROPPED_WITH_QUEST_ITEM);
    let (status, flags) = (r.status, r.flags);
    act1::q4::tree_operate(&mut ctl, &mut w, tree, P1);
    assert_eq!(w.f.log, ["sound 1 45", "drop bks  2"]);
    assert!(w.f.sent.is_empty());
    assert_eq!(w.f.objects[&tree].2, 0);
    let r = ctl.record(4).unwrap();
    assert_eq!((r.state, r.status, r.flags), (4, status, flags));
    assert!(!r.has_callback(event::PLAYER_DROPPED_WITH_QUEST_ITEM));
    let x = &r.extra.q4;
    assert!(x.tree_known && x.tree_guid == 0x61);
    assert_eq!(x.scrolls, 0);
}

// Covers: specs/world/quests-act1-rest.md §8 r5
#[test]
fn event0_without_a_player_is_fatal() {
    for (chain, code) in [(3u8, 0x0059_16B3u32), (4, 0x0059_25BB)] {
        let (mut ctl, _) = control();
        let mut f = Fake::new();
        let mut list = TextList::new();
        let i = ctl.find(chain).unwrap();
        let args = EventArgs {
            event: event::NPC_ACTIVATE,
            target: Some(AKARA_U),
            player: None,
            ..EventArgs::default()
        };
        act1::callback(&mut ctl, &mut f, i, args, Some(&mut list), false);
        assert_eq!(ctl.faults, [QuestError::Fatal(code)], "chain {chain}");
        assert!(list.is_empty() && f.log.is_empty() && f.sent.is_empty());
    }
}

fn level_change(ctl: &mut QuestControl, f: &mut Fake, a: u32, b: u32) {
    call(
        ctl,
        f,
        6,
        EventArgs {
            event: event::CHANGED_LEVEL,
            player: Some(P1),
            a,
            b,
            ..EventArgs::default()
        },
    );
}

// Covers: specs/world/quests-act1-rest.md §8 r6
#[test]
fn slaughter_catacombs_keeps_late_states() {
    // Vector: b = 34, not-intro, state 4, status 2 → state stays 4,
    // nothing sent.
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    f.players.insert(P2, player(2, Some(34)));
    let r = ctl.record_mut(6).unwrap();
    assert!(r.not_intro);
    (r.state, r.status) = (4, 2);
    level_change(&mut ctl, &mut f, 33, 34);
    let r = ctl.record(6).unwrap();
    assert_eq!((r.state, r.status), (4, 2));
    assert!(f.sent.is_empty() && f.log.is_empty());
    assert_eq!((f.flags(P1).word(6), f.flags(P2).word(6)), (0, 0));
    // Vector: b = 37, state 1, status 2 → state 3, every player O2
    // (state 3, status 2: bit 4), nothing broadcast.
    let r = ctl.record_mut(6).unwrap();
    (r.state, r.status) = (1, 2);
    level_change(&mut ctl, &mut f, 36, 37);
    let r = ctl.record(6).unwrap();
    assert_eq!((r.state, r.status), (3, 2));
    assert!(f.sent.is_empty());
    assert_eq!((f.flags(P1).word(6), f.flags(P2).word(6)), (0x0010, 0x0010));
    // State 3 or 5 kept, no O2 walk: with status 2 at Catacombs 4 an O2
    // walk at state 3 would set bit 4; with status 1 elsewhere, bit 3.
    for (state, status, b) in [(3u8, 2u8, 37u32), (3, 1, 34), (5, 2, 37), (5, 1, 35)] {
        let (mut ctl, _) = control();
        let mut f = Fake::new();
        let r = ctl.record_mut(6).unwrap();
        (r.state, r.status) = (state, status);
        level_change(&mut ctl, &mut f, b - 1, b);
        let r = ctl.record(6).unwrap();
        assert_eq!((r.state, r.status), (state, status), "state {state} b {b}");
        assert!(f.sent.is_empty() && f.log.is_empty());
        assert_eq!(f.flags(P1).word(6), 0);
    }
}

/// Runs the Andariel portal timer from counter 9 to the firing that
/// makes it 10 (O7).
fn portal_firing<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W) {
    ctl.add_timer(6, TimerFn::AndarielPortals, 1).unwrap();
    ctl.record_mut(6).unwrap().extra.q6.counter = 9;
    for _ in 0..8 {
        ctl.update(w);
        if ctl.record(6).unwrap().extra.q6.counter == 10 {
            return;
        }
    }
    panic!("the portal timer did not fire");
}

// Covers: specs/world/quests-act1-rest.md §8 r7
#[test]
fn andariel_portal_first_player_in_catacombs_4() {
    // P1 without a room, P2 in another level, P3 and P4 in Catacombs 4:
    // only P3 gets the portal (the walk stops there).
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    f.p(P1).level = None;
    f.players.insert(P2, player(2, Some(36)));
    f.pos.insert(P2, (10, 11, RoomIdT(1)));
    let (p3, p4) = (UnitId(3), UnitId(4));
    f.players.insert(p3, player(3, Some(37)));
    f.pos.insert(p3, (30, 40, RoomIdT(2)));
    f.players.insert(p4, player(4, Some(37)));
    f.pos.insert(p4, (50, 60, RoomIdT(2)));
    portal_firing(&mut ctl, &mut f);
    assert_eq!(f.log, ["portal 30 40 59 1"]);
    // The creation fails: the walk still stops at P3 (one try).
    let (mut ctl, _) = control();
    let mut w = Failing { f: Fake::new() };
    w.f.p(P1).level = None;
    w.f.players.insert(p3, player(3, Some(37)));
    w.f.pos.insert(p3, (30, 40, RoomIdT(2)));
    w.f.players.insert(p4, player(4, Some(37)));
    w.f.pos.insert(p4, (50, 60, RoomIdT(2)));
    portal_firing(&mut ctl, &mut w);
    assert_eq!(w.f.log, ["portal 30 40 59 1"]);
    // Nobody in Catacombs 4: no portal.
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    f.pos.insert(P1, (30, 40, RoomIdT(2)));
    portal_firing(&mut ctl, &mut f);
    assert!(f.log.is_empty());
}
