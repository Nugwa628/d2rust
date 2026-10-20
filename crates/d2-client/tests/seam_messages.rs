//! Contract checks of the d2-server ↔ d2-proto ↔ d2-client message seam,
//! without game files: every S→C message
//! is built with the builder the server sends it with (`d2-sim`,
//! `d2-server`), read back with the generated layout of
//! `server-messages.tsv` and with the client's receive path; every C→S
//! message the client builds by hand is read back with the generated
//! layout of `client-messages.tsv` and the server's size rule.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use d2_client::bridge::dispatch::Dispatch;
use d2_client::bridge::intent::{encode, route};
use d2_client::bridge::link::{LinkError, Sent, ServerLink};
use d2_client::bridge::objects::interact::interact_bytes;
use d2_client::bridge::receive::{receive_chunk, ReceiveLog};
use d2_client::bridge::world::{ClientWorld, KindData, ModelInputs, SkillRow, UnitKey};
use d2_client::bridge::{Bridge, Pumped, SendQueue};
use d2_client::ui::panels::npc;
use d2_proto::schema::Size;
use d2_proto::transport::{client_size, server_size, split_server_buffer};
use d2_proto::{client as c2s, server as s2c, FixedMessage, PROTOCOL_VERSION};
use d2_server::adapters::session::{load_act, stat_message};
use d2_server::adapters::session_flow::{game_list_entry, player_event};
use d2_sim::combat::vitals::sync as vitals;
use d2_sim::monsters::mode_message as mm;
use d2_sim::path::walk::messages as walk;
use d2_sim::units::messages as unit;
use d2_sim::wiring::action::switch;
use d2_sim::wiring::path::{act_change, place};
use d2_sim::world::hirelings::{level as merc_level, pets};

const GUID: u32 = 0x0A0B_0C0D;

/// Decodes `b` with `M`; the S→C size rule gives exactly `b.len()`.
fn read<M: FixedMessage>(b: &[u8]) -> M {
    assert_eq!(
        server_size(b),
        Size::Bytes(b.len()),
        "0x{:02X}: size rule",
        b[0]
    );
    M::decode(b).unwrap_or_else(|e| panic!("0x{:02X}: {e}", b[0]))
}

#[test]
fn server_builders_put_each_field_where_the_layout_reads_it() {
    let m: s2c::GameFlags = read(&unit::game_flags(2, 0x0010_0004, true, false));
    assert_eq!(
        (m.difficulty, m.unk, m.expansion, m.ladder),
        (2, 0x0010_0004, 1, 0)
    );

    let m: s2c::LoadAct = read(&act_change::load_act(3, 0x1122_3344, 0x55, 0x6677_8899));
    assert_eq!(
        (m.act, m.f2, m.f6, m.f8),
        (3, 0x1122_3344, 0x55, 0x6677_8899)
    );
    let m: s2c::LoadAct = read(&load_act(1, 0x1122_3344, 0x6677_8899).encode());
    assert_eq!((m.act, m.f2, m.f8), (1, 0x1122_3344, 0x6677_8899));

    let m: s2c::MapReveal = read(&place::map_reveal(0x1234, 0x5678, 9));
    assert_eq!((m.x, m.y, m.level), (0x1234, 0x5678, 9));
    let m: s2c::MapHide = read(&unit::map_hide(0x1234, 0x5678, 9));
    assert_eq!((m.x, m.y, m.level), (0x1234, 0x5678, 9));

    let m: s2c::AssignLevelWarp = read(&unit::assign_warp(5, GUID, 0x1F, 0x1111, 0x2222));
    assert_eq!(
        (m.type_, m.guid, m.class, m.x, m.y),
        (5, GUID, 0x1F, 0x1111, 0x2222)
    );
    let m: s2c::RemoveUnit = read(&unit::remove_unit(1, GUID));
    assert_eq!((m.type_, m.guid), (1, GUID));
    let m: s2c::GameHandshake = read(&unit::unit_ref(0x0B, 0, GUID));
    assert_eq!((m.type_, m.guid), (0, GUID));

    let m: s2c::PlayerStop = read(&walk::player_stop(0, GUID, 3, 0x1111, 0x2222, 4, 77));
    assert_eq!(
        (m.type_, m.guid, m.a, m.x, m.y, m.b, m.life_pct),
        (0, GUID, 3, 0x1111, 0x2222, 4, 77)
    );
    let m: s2c::PlayerMove = read(&walk::player_move(
        0, GUID, 1, 0x1111, 0x2222, 0x3333, 0x4444,
    ));
    assert_eq!(
        (m.type_, m.guid, m.code, m.target_x, m.target_y, m.x, m.y),
        (0, GUID, 1, 0x1111, 0x2222, 0x3333, 0x4444)
    );
    let m: s2c::PlayerToTarget = read(&walk::player_to_target(
        0,
        GUID,
        0x18,
        1,
        0x0102_0304,
        0x3333,
        0x4444,
    ));
    assert_eq!(
        (
            m.type_,
            m.guid,
            m.code,
            m.target_type,
            m.target_guid,
            m.x,
            m.y
        ),
        (0, GUID, 0x18, 1, 0x0102_0304, 0x3333, 0x4444)
    );
    let m: s2c::ReassignPlayer = read(&walk::reassign_player(0, GUID, 0x131D, 0x1381, 1));
    assert_eq!(
        (m.type_, m.guid, m.x, m.y, m.flag),
        (0, GUID, 0x131D, 0x1381, 1)
    );

    let m: s2c::SetSkill = read(&unit::set_skill(0, GUID, 1, 0x0123, 0xFFFF_FFFF));
    assert_eq!(
        (m.type_, m.guid, m.hand, m.skill, m.item),
        (0, GUID, 1, 0x0123, 0xFFFF_FFFF)
    );
    let m: s2c::UpdateItemOSkill = read(&unit::update_oskill(0, true, GUID, 0x0102, 7, 9));
    assert_eq!(
        (m.type_, m.remove, m.guid, m.skill, m.level, m.bonus),
        (0, 1, GUID, 0x0102, 7, 9)
    );
    let m: s2c::AssignObject = read(&unit::assign_object(GUID, 0x177, 0x1A2B, 0x3C4D, 2, 1));
    assert_eq!(
        (m.type_, m.guid, m.class, m.x, m.y, m.mode, m.interact),
        (2, GUID, 0x177, 0x1A2B, 0x3C4D, 2, 1)
    );
    let name = *b"werwer\0\0\0\0\0\0\0\0\0\0";
    let m: s2c::AssignPlayer = read(&switch::assign_player(GUID, 4, &name, 0x1111, 0x2222));
    assert_eq!(
        (m.guid, m.class, m.name, m.x, m.y),
        (GUID, 4, name, 0x1111, 0x2222)
    );
    let m: s2c::PortalFlags = read(&unit::portal_flags(0x8765_4321));
    assert_eq!(m.f1, 0x8765_4321);
    let m: s2c::AssignHotkey = read(&unit::assign_hotkey(3, 36, true, 0xCAFE_F00D));
    assert_eq!((m.slot, m.skill, m.item), (3, 36 | 0x8000, 0xCAFE_F00D));
    let m: s2c::Unknown98 = read(&unit::unknown98(GUID, 0xBEEF));
    assert_eq!((m.guid, m.f5), (GUID, 0xBEEF));
    let m: s2c::PortalOwnership = read(&unit::portal_ownership(GUID, b"abc", 7, 8));
    assert_eq!(
        (m.owner, &m.name[..4], m.portal, m.portal2),
        (GUID, &b"abc\0"[..], 7, 8)
    );
    let m: s2c::DelayedState = read(&unit::state_ref(0xA7, 1, GUID, 0x33));
    assert_eq!((m.type_, m.guid, m.state), (1, GUID, 0x33));
    let m: s2c::EndState = read(&unit::state_ref(0xA9, 1, GUID, 0x33));
    assert_eq!((m.type_, m.guid, m.state), (1, GUID, 0x33));

    let m: s2c::AssignMerc = read(&pets::assign_merc(0x152, 0x11, 0x22, 0x3344_5566, 0x77));
    assert_eq!(
        (m.pet_type, m.class, m.owner, m.merc, m.seed, m.name),
        (7, 0x152, 0x11, 0x22, 0x3344_5566, 0x77)
    );
    // Argument order (pet, owner) differs from the wire order (owner @5,
    // pet @9): the layout read must still give each its own value.
    let m: s2c::PetAction = read(&pets::pet_action(1, 7, 0x152, 0x22, 0x11));
    assert_eq!(
        (m.action, m.pet_type, m.class, m.owner, m.pet),
        (1, 7, 0x152, 0x11, 0x22)
    );
    let m: s2c::Unknown9B = read(&pets::merc_dead_message(0x0123, 50_000));
    assert_eq!((m.name, m.cost), (0x0123, 50_000));

    let m: s2c::MonsterStop = read(&mm::monster_stop(GUID, 0x1A2B, 0x3C4D, 0x80));
    assert_eq!((m.guid, m.x, m.y, m.life), (GUID, 0x1A2B, 0x3C4D, 0x80));
    let m: s2c::MonsterState = read(&mm::monster_state(GUID, 3, 0x1111, 0x2222, 4, 5));
    assert_eq!(
        (m.guid, m.code, m.a, m.b, m.d, m.e),
        (GUID, 3, 0x1111, 0x2222, 4, 5)
    );
    let m: s2c::MonsterAttack = read(&mm::monster_attack(
        GUID,
        4,
        1,
        0x0506_0708,
        6,
        0x1A2B,
        0x3C4D,
    ));
    assert_eq!(
        (m.guid, m.code, m.target_type, m.target, m.d, m.x, m.y),
        (GUID, 4, 1, 0x0506_0708, 6, 0x1A2B, 0x3C4D)
    );
}

#[test]
fn bit_packed_vitals_read_back_in_the_layout_units() {
    // Life / mana / stamina cross as whole points (stat >> 8); position
    // in sub-tiles.
    let m: s2c::LifeManaUpdate = read(&vitals::life_mana_update(
        0x1234, 0x0321, 0x7FFF, 55, 66, 0x131D, 0x1381, 0xFE, 0x02,
    ));
    assert_eq!(
        (m.life, m.mana, m.stamina, m.life_pred, m.mana_pred),
        (0x1234, 0x0321, 0x7FFF, 55, 66)
    );
    assert_eq!((m.x, m.y, m.dx, m.dy), (0x131D, 0x1381, 0xFE, 0x02));
    let m: s2c::LifeManaUpdate2 = read(&vitals::life_mana_update2(
        0x1234, 0x0321, 0x7FFF, 0x131D, 0x1381, 0xFE, 0x02,
    ));
    assert_eq!(
        (m.life, m.mana, m.stamina, m.x, m.y, m.dx, m.dy),
        (0x1234, 0x0321, 0x7FFF, 0x131D, 0x1381, 0xFE, 0x02)
    );
    let m: s2c::WalkVerify = read(&walk::walk_verify(0x4A, 0x131D, 0x1381, -2, 2));
    assert_eq!(
        (m.stamina, m.x, m.y, m.dx, m.dy),
        (0x4A, 0x131D, 0x1381, 0xFE, 2)
    );
}

#[test]
fn vitals_dx_dy_round_trip_byte_exact_for_every_target_offset() {
    // dx = (X - target) & 0xFF for every target offset -128 ..= 127.
    for off in -128i32..=127 {
        let (x, t) = (0x131Du16, (0x131Di32 - off) as u16);
        let (dx, dy) = (x.wrapping_sub(t) as u8, 0x1381u16.wrapping_sub(t) as u8);
        let m: s2c::LifeManaUpdate =
            read(&vitals::life_mana_update(1, 2, 3, 4, 5, x, 0x1381, dx, dy));
        assert_eq!((m.dx, m.dy), (dx, dy), "offset {off}");
        let m: s2c::LifeManaUpdate2 = read(&vitals::life_mana_update2(1, 2, 3, x, 0x1381, dx, dy));
        assert_eq!((m.dx, m.dy), (dx, dy), "offset {off}");
        let m: s2c::WalkVerify = read(&walk::walk_verify(1, x, 0x1381, dx as i8, dy as i8));
        assert_eq!((m.dx, m.dy), (dx, dy), "offset {off}");
    }
}

#[test]
fn variable_and_choice_builders_agree_with_the_size_rules() {
    let mut all: Vec<Vec<u8>> = Vec::new();
    // Value-dependent choice of id: 0x1D / 0x1E / 0x1F.
    for (v, id, len) in [(0x12, 0x1D, 3), (0x1234, 0x1E, 4), (0x0012_3456, 0x1F, 6)] {
        let b = stat_message(12, v).unwrap();
        assert_eq!((b[0], b.len()), (id, len));
        all.push(b);
    }
    // 0x1A / 0x1B / 0x1C (experience) and 0xA0..0xA2 (hireling).
    for (old, new, id) in [(0, 0x10, 0x1A), (0, 0x1234, 0x1B), (0, 0x0012_3456, 0x1C)] {
        let b = vitals::exp_message(new, old).unwrap();
        assert_eq!(b[0], id);
        all.push(b);
    }
    for (old, new, id) in [(0, 0x10, 0xA1), (0, 0x1234, 0xA2), (0, 0x0012_3456, 0xA0)] {
        let b = merc_level::exp_delta_message(13, GUID, old, new);
        assert_eq!(b[0], id);
        all.push(b);
    }
    // Strings and counted lists.
    all.push(unit::overhead_chat(3, 1, GUID, b"hello"));
    all.push(unit::overhead_chat(3, 1, GUID, b""));
    all.push(unit::base_skill_levels(GUID, &[(0, 1), (36, 20), (0x0102, 3)]).unwrap());
    all.push(player_event(2, b"werwer\0\0\0\0\0\0\0\0\0\0").to_vec());
    all.push(game_list_entry(b"game\0\0\0\0\0\0\0\0\0\0\0\0", 1, 2).to_vec());
    all.push(unit::GAME_ENTRY_DONE.to_vec());
    for b in &all {
        assert_eq!(server_size(b), Size::Bytes(b.len()), "0x{:02X}", b[0]);
    }
    // One chunk of all of them splits back into the same messages.
    let chunk: Vec<u8> = all.concat();
    let split = split_server_buffer(&chunk).unwrap();
    assert!(split.discarded.is_empty());
    let got: Vec<Vec<u8>> = split.messages.iter().map(|m| m.to_vec()).collect();
    assert_eq!(got, all);

    let m: s2c::EventMessage = read(&player_event(2, b"werwer\0\0\0\0\0\0\0\0\0\0"));
    assert_eq!(
        (m.code, m.f2, m.f3, &m.name[..7]),
        (2, 4, 0, &b"werwer\0"[..])
    );
    let m: s2c::GameList = read(&game_list_entry(b"game\0\0\0\0\0\0\0\0\0\0\0\0", 1, 2));
    assert_eq!((m.f49, m.f51), (1, 2));
}

/// The client model after receiving `chunk` with the spec's dispatch.
fn receive(w: &mut ClientWorld, inputs: &ModelInputs, log: &mut ReceiveLog, chunk: &[u8]) {
    let dispatch = Dispatch::from_spec().unwrap();
    let mut out = Vec::new();
    receive_chunk(w, inputs, &dispatch, log, &mut out, chunk).unwrap();
}

#[test]
fn the_server_join_sequence_builds_the_client_model() {
    let name = *b"werwer\0\0\0\0\0\0\0\0\0\0";
    let (x, y) = (0x131D_u16, 0x1381_u16);
    // The join messages (the act, darkness and room messages
    // need a client DRLG source and are left out): built by the
    // server's builders in the server's order.
    let seq: Vec<Vec<u8>> = vec![
        unit::game_flags(1, 0x0010_0004, true, false).to_vec(),
        unit::GAME_LOADING.to_vec(),
        unit::LOAD_SUCCESSFUL.to_vec(),
        switch::assign_player(GUID, 4, &name, 0, 0).to_vec(),
        unit::unit_ref(0x0B, 0, GUID).to_vec(),
        unit::portal_flags(0).to_vec(),
        stat_message(12, 1).unwrap(),
        stat_message(0, 30).unwrap(),
        unit::set_skill(0, GUID, 1, 0, 0).to_vec(),
        unit::set_skill(0, GUID, 0, 0, 0).to_vec(),
        // The join's vitals: the player is not placed yet (0, 0).
        vitals::life_mana_update2(55, 10, 92, 0, 0, 0, 0).to_vec(),
        vitals::exp_message(500, 0).unwrap(),
        walk::reassign_player(0, GUID, x, y, 1).to_vec(),
        unit::GAME_ENTRY_DONE.to_vec(),
        unit::LOAD_COMPLETE.to_vec(),
        player_event(2, &name).to_vec(),
    ];
    let mut w = ClientWorld::default();
    let mut log = ReceiveLog::default();
    // The join's two 0x23 select skill 0 (Attack, row 0 of every
    // `skills.txt`); a model without any skill row refuses them (fatal
    // 0x668), so the inputs hold the one row the sequence names.
    let mut inputs = ModelInputs::default();
    inputs.tables.skills = vec![SkillRow::default()];
    receive(&mut w, &inputs, &mut log, &seq.concat());

    assert!(log.discarded.is_empty(), "{:?}", log.discarded);
    assert!(log.rejected.is_empty(), "{:?}", log.rejected);
    assert!(log.unowned.is_empty(), "{:?}", log.unowned);
    assert!(log.dropped.is_empty(), "{:?}", log.dropped);
    assert_eq!(log.handled, seq.len() as u64);

    // 0x01.
    assert_eq!((w.difficulty, w.expansion, w.ladder), (1, 1, 0));
    assert_eq!(w.game_flags, 0x0010_0004);
    // 0x02: the client answers C→S 0x6B, which the server's session
    // flow takes as the join.
    assert_eq!(w.outgoing.first().map(Vec::as_slice), Some(&[0x6B][..]));
    // 0x59 + 0x0B: the player unit with its name is the local player.
    let key = UnitKey::new(0, GUID);
    assert_eq!(w.local_player, Some(key));
    let u = &w.units[&key];
    assert_eq!(u.class, 4);
    match &u.kind {
        KindData::Player(p) => assert_eq!(p.name, name),
        k => panic!("not a player: {k:?}"),
    }
    // 0x1D: stats; 0x95: whole points, stored << 8; 0x1A adds.
    assert_eq!(u.stat(12), 1);
    assert_eq!(u.stat(0), 30);
    assert_eq!(u.stat(6), 55 << 8);
    assert_eq!(u.stat(8), 10 << 8);
    assert_eq!(u.stat(10), 92 << 8);
    assert_eq!(u.stat(13), 500);
    // 0x15: the server's sub-tile position; 0x04 found it.
    assert_eq!(u.position, Some((x, y)));
    assert!(w.in_game);
}

#[test]
fn client_built_requests_read_back_with_the_server_layouts() {
    fn back<M: FixedMessage>(b: &[u8]) -> M {
        assert_eq!(client_size(b), Size::Bytes(b.len()), "0x{:02X}", b[0]);
        assert_eq!(route(b), Ok(SendQueue::Game), "0x{:02X}", b[0]);
        M::decode(b).unwrap_or_else(|e| panic!("0x{:02X}: {e}", b[0]))
    }
    let m: c2s::InteractWithEntity = back(&interact_bytes(2, GUID));
    assert_eq!((m.type_, m.id), (2, GUID));
    let m: c2s::InitEntityChat = back(&npc::msg_chat_start(1, GUID));
    assert_eq!(m.id, GUID);
    let m: c2s::TerminateEntityChat = back(&npc::msg_chat_end(GUID));
    assert_eq!(m.id, GUID);
    let m: c2s::TerminateEntityChat = back(&npc::msg_chat_not_found(1, GUID));
    assert_eq!(m.id, GUID);
    let m: c2s::QuestMessage = back(&npc::msg_quest(GUID, 0x0123));
    assert_eq!((m.npc, m.msg), (GUID, 0x0123));
    let m: c2s::UseBeltItem = back(&encode(&d2_client::bridge::items::use_belt(GUID)));
    assert_eq!(m.item, GUID);
    // The one-byte system messages the client sends go to the system
    // queue.
    for id in [0x6B_u8, 0x69] {
        assert_eq!(route(&[id]), Ok(SendQueue::System));
    }
}

/// A link that says a resume happened when the test asks, and nothing else.
#[derive(Clone, Default)]
struct ResumeLink {
    resume: Arc<AtomicBool>,
}

impl ServerLink for ResumeLink {
    fn protocol_version(&self) -> u32 {
        PROTOCOL_VERSION
    }
    fn tick_rate(&self) -> d2_sim::tick::TickRate {
        d2_sim::tick::TickRate::BASE
    }
    fn send(&mut self, _queue: SendQueue, _msg: &[u8]) -> Result<Sent, LinkError> {
        Ok(Sent::Queued)
    }
    fn pump(&mut self) -> Result<Pumped, LinkError> {
        Ok(Pumped::default())
    }
    fn receive(&mut self) -> Vec<Vec<u8>> {
        Vec::new()
    }
    fn take_resume(&mut self) -> bool {
        self.resume.swap(false, Ordering::SeqCst)
    }
}

/// The join messages of `the_server_join_sequence_builds_the_client_model`
/// for `name` at (x, y).
fn join_messages(name: &[u8; 16], guid: u32, x: u16, y: u16) -> Vec<Vec<u8>> {
    vec![
        unit::game_flags(1, 0x0010_0004, true, false).to_vec(),
        unit::GAME_LOADING.to_vec(),
        unit::LOAD_SUCCESSFUL.to_vec(),
        switch::assign_player(guid, 4, name, 0, 0).to_vec(),
        unit::unit_ref(0x0B, 0, guid).to_vec(),
        stat_message(12, 1).unwrap(),
        stat_message(0, 30).unwrap(),
        unit::set_skill(0, guid, 1, 0, 0).to_vec(),
        unit::set_skill(0, guid, 0, 0, 0).to_vec(),
        vitals::life_mana_update2(55, 10, 92, 0, 0, 0, 0).to_vec(),
        vitals::exp_message(500, 0).unwrap(),
        walk::reassign_player(0, guid, x, y, 1).to_vec(),
        unit::GAME_ENTRY_DONE.to_vec(),
        unit::LOAD_COMPLETE.to_vec(),
        player_event(2, name).to_vec(),
    ]
}

/// A bridge's view of the game, without its frame/tick bookkeeping (the
/// resumed game ran a resume frame the fresh one did not).
fn view(w: &ClientWorld) -> ClientWorld {
    let mut v = w.clone();
    v.session_total = 0;
    v.session_log.clear();
    v.frames = 0;
    v.server_ticks = 0;
    v
}

#[test]
fn a_resumed_client_view_equals_a_fresh_joins() {
    let name = *b"werwer\0\0\0\0\0\0\0\0\0\0";
    let (x, y) = (0x131D_u16, 0x1381_u16);
    let join = join_messages(&name, GUID, x, y);

    let mut fresh = Bridge::new(ResumeLink::default()).unwrap();
    fresh.set_skill_rows(vec![SkillRow::default()]);
    fresh.receive_chunk(&join.concat()).unwrap();

    // A client that played on, saw another player and a state, then lost the
    // connection: on the resume the server re-enters it and sends the game
    // again (contracts 3.7).
    let link = ResumeLink::default();
    let resume = link.resume.clone();
    let mut resumed = Bridge::new(link).unwrap();
    resumed.set_skill_rows(vec![SkillRow::default()]);
    let stale = 0x0EDC_BA98;
    let stale_name = *b"stale\0\0\0\0\0\0\0\0\0\0\0";
    let stale_seq = [
        switch::assign_player(stale, 4, &stale_name, 0x0101, 0x0202).to_vec(),
        unit::state_ref(0xA7, 1, stale, 0x33).to_vec(),
        stat_message(0, 3).unwrap(),
        walk::reassign_player(0, stale, 0x0303, 0x0404, 1).to_vec(),
    ];
    resumed.receive_chunk(&stale_seq.concat()).unwrap();
    assert_eq!(resumed.world().units.len(), 1, "the stale player is in");
    resume.store(true, Ordering::SeqCst);
    resumed.frame().unwrap();
    resumed.receive_chunk(&join.concat()).unwrap();

    assert_eq!(view(fresh.world()), view(resumed.world()));
    assert_eq!(resumed.world().local_player, Some(UnitKey::new(0, GUID)));
    assert_eq!(resumed.world().units.len(), 1, "the stale player is gone");
}
