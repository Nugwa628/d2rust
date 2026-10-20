// Spec: specs/world/quests-act3.md (Test vectors); specs/world/quests-act3-2.md (Test vectors)
//! The Act III quests callback by callback, on the quests' fake world
//! wrapped with the Act III seams ([`Fake3`]). One file per quest.

use std::collections::{BTreeMap, BTreeSet};

use super::tests::*;
use super::*;

mod gossip;
mod part2;
mod q1;
mod q2;
mod q3;
mod q4;
mod q5;
mod q6;
mod wiring;

/// The shared fake plus the Act III seams. Units of other kinds get a
/// level / act from `levels` / `acts`; every seam logs to `f.log`.
pub(super) struct Fake3 {
    pub(super) f: Fake,
    /// Level and act of non-player units (objects, monsters).
    pub(super) levels: BTreeMap<UnitId, u32>,
    pub(super) acts: BTreeMap<UnitId, u8>,
    /// Results of `quest_drop` in order (empty: created).
    pub(super) drops: Vec<bool>,
    pub(super) act3: bool,
    pub(super) chest_gate: bool,
    pub(super) room_covering: Option<RoomId>,
    pub(super) player_in_rooms: bool,
    pub(super) player_near: Option<UnitId>,
    pub(super) special: bool,
    pub(super) blocked: BTreeSet<(i32, i32)>,
    pub(super) trading: BTreeSet<UnitId>,
    pub(super) weapons: BTreeMap<UnitId, [u8; 4]>,
}

impl Fake3 {
    /// P1 in Kurast Docks (Act III), the Act III NPCs as monsters with
    /// GUID = unit id.
    pub(super) fn new() -> Self {
        let mut f = Fake::new();
        {
            let p = f.p(P1);
            p.act = Some(2);
            p.level = Some(75);
        }
        for (u, class) in NPCS {
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
        Fake3 {
            f,
            levels: BTreeMap::new(),
            acts: BTreeMap::new(),
            drops: Vec::new(),
            act3: true,
            chest_gate: true,
            room_covering: None,
            player_in_rooms: false,
            player_near: None,
            special: false,
            blocked: BTreeSet::new(),
            trading: BTreeSet::new(),
            weapons: BTreeMap::new(),
        }
    }

    /// Add a player in Act III at `level` (GUID = unit id).
    pub(super) fn add_player(&mut self, u: UnitId, level: u32) {
        self.f.players.insert(
            u,
            Player {
                guid: u.0,
                act: Some(2),
                level: Some(level),
                ..Player::default()
            },
        );
    }

    pub(super) fn p(&mut self, u: UnitId) -> &mut Player {
        self.f.p(u)
    }

    pub(super) fn flags(&self, u: UnitId) -> QuestFlags {
        self.f.flags(u)
    }

    pub(super) fn log(&self) -> Vec<String> {
        self.f.log.clone()
    }
}

pub(super) const CAIN3_U: UnitId = UnitId(0x20);
pub(super) const ASHEARA_U: UnitId = UnitId(0x21);
pub(super) const HRATLI_U: UnitId = UnitId(0x22);
pub(super) const ALKOR_U: UnitId = UnitId(0x23);
pub(super) const ORMUS_U: UnitId = UnitId(0x24);
pub(super) const MESHIF2_U: UnitId = UnitId(0x25);
pub(super) const NATALYA_U: UnitId = UnitId(0x26);
const NPCS: [(UnitId, u16); 7] = [
    (CAIN3_U, act3::npc::CAIN3),
    (ASHEARA_U, act3::npc::ASHEARA),
    (HRATLI_U, act3::npc::HRATLI),
    (ALKOR_U, act3::npc::ALKOR),
    (ORMUS_U, act3::npc::ORMUS),
    (MESHIF2_U, act3::npc::MESHIF2),
    (NATALYA_U, act3::npc::NATALYA),
];

/// Event 0 to one chain's record only: the lines it adds.
pub(super) fn text(
    ctl: &mut QuestControl,
    f: &mut Fake3,
    chain: u8,
    p: UnitId,
    npc_u: UnitId,
) -> TextList {
    let mut list = TextList::new();
    let args = EventArgs {
        event: event::NPC_ACTIVATE,
        target: Some(npc_u),
        player: Some(p),
        ..EventArgs::default()
    };
    let i = ctl.find(chain).unwrap();
    act1::callback(ctl, f, i, args, Some(&mut list), false);
    list
}

/// One callback of one chain's record (any event) through the dispatch.
pub(super) fn call(ctl: &mut QuestControl, f: &mut Fake3, chain: u8, args: EventArgs) {
    let i = ctl.find(chain).unwrap();
    act1::callback(ctl, f, i, args, None, false);
}

/// C→S 0x31 from `p` to the NPC unit `n` (GUID = unit id).
pub(super) fn say(ctl: &mut QuestControl, f: &mut Fake3, p: UnitId, n: UnitId, msg: u16) {
    let mut m = vec![0x31];
    m.extend_from_slice(&n.0.to_le_bytes());
    m.extend_from_slice(&msg.to_le_bytes());
    m.extend_from_slice(&[0, 0]);
    assert_eq!(ctl.quest_message(f, p, &m), 0);
}

/// The 0x5D messages sent, as (player, bytes).
pub(super) fn sent_5d(f: &Fake3) -> Vec<(UnitId, Vec<u8>)> {
    f.f.sent
        .iter()
        .filter(|m| m.1[0] == 0x5D)
        .cloned()
        .collect()
}

pub(super) fn code(c: &[u8; 4]) -> String {
    String::from_utf8_lossy(c).into_owned()
}

impl QuestWorld for Fake3 {
    fn tick_rate(&self) -> crate::tick::TickRate {
        crate::tick::TickRate::BASE
    }
    // The Act I remainder seams: the shared fake's.
    #[allow(clippy::too_many_arguments)]
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
        self.f
            .spawn_monster_flags(room, x, y, class, mode, spread, flags)
    }
    #[allow(clippy::too_many_arguments)]
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
        self.f.open_portal(owner, room, x, y, level, class, exact)
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
        self.f.create_missile(owner, skill, level, class, x, y)
    }
    fn set_missile_target(&mut self, missile: UnitId, a: u32, b: u32) {
        self.f.set_missile_target(missile, a, b)
    }
    fn refresh_room(&mut self, unit: UnitId) {
        self.f.refresh_room(unit)
    }
    fn spawn_object(
        &mut self,
        room: RoomId,
        x: i32,
        y: i32,
        class: u16,
        mode: i32,
    ) -> Option<UnitId> {
        self.f.spawn_object(room, x, y, class, mode)
    }
    fn client_save_flags(&self, player: UnitId) -> Option<u16> {
        self.f.client_save_flags(player)
    }
    fn set_client_save_flags(&mut self, player: UnitId, flags: u16) {
        self.f.set_client_save_flags(player, flags)
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
    fn item_code(&self, item: UnitId) -> Option<[u8; 4]> {
        self.f.item_code(item)
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
    fn quest_items(&self, player: UnitId) -> Vec<(UnitId, u8)> {
        self.f.quest_items(player)
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
    fn unit_position(&self, unit: UnitId) -> Option<(i32, i32, RoomId)> {
        self.f.unit_position(unit)
    }
    fn room_contains(&self, room: RoomId, x: i32, y: i32) -> bool {
        self.f.room_contains(room, x, y)
    }
    fn room_at(&self, room: RoomId, x: i32, y: i32) -> Option<RoomId> {
        self.f.room_at(room, x, y)
    }
    #[allow(clippy::too_many_arguments)]
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
        self.f.free_spot_at(room, x, y, size, mask, radius, limit)
    }
    fn spawn_monster(
        &mut self,
        room: RoomId,
        x: i32,
        y: i32,
        class: u16,
        mode: u8,
        r: u32,
    ) -> Option<UnitId> {
        self.f.spawn_monster(room, x, y, class, mode, r)
    }
    fn or_unit_flags(&mut self, unit: UnitId, flags: u32) {
        self.f.or_unit_flags(unit, flags)
    }
    fn monsters(&self) -> Vec<UnitId> {
        self.f.monsters()
    }
    fn npc_chat_clients(&self, npc: UnitId) -> Option<Vec<UnitId>> {
        self.f.npc_chat_clients(npc)
    }
    fn remove_monster(&mut self, monster: UnitId) {
        self.f.remove_monster(monster)
    }
    fn drop_preset_monster(&mut self, act: u8, class: u16) {
        self.f.drop_preset_monster(act, class)
    }
    fn find_object_near(&self, object: UnitId, class: u16) -> Option<UnitId> {
        self.f.find_object_near(object, class)
    }
    fn create_object(&mut self, room: RoomId, x: i32, y: i32, class: u16) -> Option<UnitId> {
        self.f.create_object(room, x, y, class)
    }
    fn object_anim_length(&self, object: UnitId) -> i32 {
        self.f.object_anim_length(object)
    }
    fn schedule_object_event(&mut self, object: UnitId, ev: u8, frame: i32) {
        self.f.schedule_object_event(object, ev, frame)
    }
    fn open_quest_message(&mut self, player: UnitId, object: UnitId, msg: u16) {
        self.f.open_quest_message(player, object, msg)
    }
    fn unhandled(&mut self, chain: u8, function: u32) {
        self.f.unhandled(chain, function)
    }
    fn unit_level(&self, unit: UnitId) -> Option<u32> {
        self.f
            .unit_level(unit)
            .or_else(|| self.levels.get(&unit).copied())
    }
    fn unit_act(&self, unit: UnitId) -> Option<u8> {
        self.f
            .unit_act(unit)
            .or_else(|| self.acts.get(&unit).copied())
    }
    fn quest_drop(
        &mut self,
        unit: UnitId,
        c: [u8; 4],
        quality: u8,
        _: Option<i32>,
        droppable: bool,
    ) -> Option<UnitId> {
        self.f.log.push(format!(
            "qdrop {} {} {quality} {droppable}",
            unit.0,
            code(&c)
        ));
        let made = if self.drops.is_empty() {
            true
        } else {
            self.drops.remove(0)
        };
        made.then_some(UnitId(700))
    }
    fn has_act3(&mut self) -> bool {
        self.act3
    }
    fn quest_chest_gate(&mut self, _: UnitId, _: UnitId) -> bool {
        self.chest_gate
    }
    fn drop_gold(&mut self, object: UnitId) {
        self.f.log.push(format!("gold {}", object.0));
    }
    fn object_treasure(&mut self, object: UnitId, _: UnitId, kind: u8) {
        self.f.log.push(format!("treasure {} {kind}", object.0));
    }
    fn spawn_monster_in_room(&mut self, room: RoomId, class: u16) -> Option<UnitId> {
        self.f.log.push(format!("spawn in room {} {class}", room.0));
        if self.f.spawns.is_empty() {
            None
        } else {
            self.f.spawns.remove(0)
        }
    }
    fn spawn_monster_at_unit(&mut self, unit: UnitId, class: u16, mode: u8) -> Option<UnitId> {
        self.f
            .log
            .push(format!("spawn at {} {class} mode {mode}", unit.0));
        if self.f.spawns.is_empty() {
            None
        } else {
            self.f.spawns.remove(0)
        }
    }
    fn adjacent_units(&mut self, room: RoomId) -> Vec<UnitId> {
        self.f.adjacent_units(room)
    }
    fn spawn_missile(&mut self, r: super::helpers::QuestMissile) -> Option<UnitId> {
        self.f.spawn_missile(r)
    }
    fn set_missile_guid(&mut self, m: UnitId, v: u32) {
        self.f.set_missile_guid(m, v)
    }
    fn monster_mode_at(&mut self, monster: UnitId, mode: u8, x: i32, y: i32) {
        self.f
            .log
            .push(format!("mode request {} {mode} {x} {y}", monster.0));
    }
    fn room_covering(&mut self, x: i32, y: i32) -> Option<RoomId> {
        self.f.log.push(format!("room covering {x} {y}"));
        self.room_covering
    }
    fn player_in_rooms(&mut self, _: RoomId) -> bool {
        self.player_in_rooms
    }
    fn stairs_warp(&mut self, object: UnitId, player: UnitId) {
        self.f
            .log
            .push(format!("stairs warp {} {}", object.0, player.0));
    }
    fn player_near_object(&mut self, object: UnitId, dist: i32) -> Option<UnitId> {
        self.f.log.push(format!("near {} {dist}", object.0));
        self.player_near
    }
    fn free_object_collision(&mut self, object: UnitId) {
        self.f.log.push(format!("free collision {}", object.0));
    }
    fn special_monster(&mut self, _: UnitId) -> bool {
        self.special
    }
    fn blocked(&mut self, _: RoomId, x: i32, y: i32, mask: u32) -> bool {
        self.f.log.push(format!("blocked? {x} {y} {mask:#x}"));
        self.blocked.contains(&(x, y))
    }
    fn spawn_quest_object(&mut self, room: RoomId, x: i32, y: i32, class: u16) -> Option<UnitId> {
        self.f
            .log
            .push(format!("spawn object {class} {x} {y} room {}", room.0));
        Some(UnitId(900))
    }
    fn is_trading(&mut self, player: UnitId) -> bool {
        self.trading.contains(&player)
    }
    fn weapon_code(&mut self, player: UnitId) -> Option<[u8; 4]> {
        self.weapons.get(&player).copied()
    }
}
