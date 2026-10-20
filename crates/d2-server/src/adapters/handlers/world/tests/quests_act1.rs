//! Every Act I quest chain (A1Q1–A1Q6) through its states on the wired
//! host (`WiredWorld`: the real `QuestControl` on `EconomyQuests` over the
//! action sim's own units and stat lists). Messages go through the real host frame
//! (C→S 0x31, 0x30); the events without a message path here (level
//! changes, kills, the updater, the object functions) are raised on the
//! wired quest world directly, as their callers would; quest objects are
//! stood in for by monster units (only their GUIDs are read, their modes
//! are staged). The seams the quest control calls but this host does not
//! provide are staged in `trade_quests::Rest`.
//!
//! Part of the world handler's test suite; shares its fixture with
//! `trade_quests`.

use d2_sim::items::tables::ItemRec;
use d2_sim::items::{q, ItemRequest, ItemTables};
use d2_sim::stats::StatLists;
use d2_sim::units::lifecycle::AllocRequest;
use d2_sim::units::{UnitId, UnitType};
use d2_sim::wiring::economy::{EconomyQuests, ItemSpawn};
use d2_sim::world::npc::class;
use d2_sim::world::quests::{act1, PlayerQuests, QuestChain};

use super::trade_quests::Fx;
use super::*;

/// Runs `$body` with `$ctl` (the quest control) and `$w` (the wired
/// quest world) bound, as the host's quest calls do.
macro_rules! quests {
    ($f:expr, |$ctl:ident, $w:ident| $body:expr) => {{
        let s = &mut $f.h.game;
        s.world.with_economy(&mut s.game, &mut s.events, |econ, p| {
            let mut $w = EconomyQuests::new(econ, &mut *p.rest);
            let $ctl = &mut *p.quests;
            $body
        })
    }};
}

/// A unit through the action sim's allocator.
fn alloc(f: &mut Fx, ty: UnitType, class: u32) -> UnitId {
    let req = AllocRequest {
        ty,
        class,
        room: None,
        add: true,
        fixed_guid: None,
        mode: 1,
        allied: false,
    };
    let s = &mut f.h.game;
    s.events
        .with(&mut s.game, |g, v| v.allocate(g, &req, 0, 0))
        .unwrap()
}

/// An NPC the 0x31 / 0x30 handlers and the text refresh can find.
fn npc(f: &mut Fx, class: u16) -> UnitId {
    let n = alloc(f, UnitType::Monster, u32::from(class));
    let g = f.guid(n);
    f.world().rest.guids.insert(n, g);
    f.world().state.add_npc(n);
    n
}

/// C→S 0x31 (NPC GUID, message, pad).
fn say(f: &mut Fx, n: UnitId, msg: u16) -> Vec<Vec<u8>> {
    let mut m = vec![0x31];
    m.extend_from_slice(&f.guid(n).to_le_bytes());
    m.extend_from_slice(&msg.to_le_bytes());
    m.extend_from_slice(&[0, 0]);
    let (code, got) = send(&mut f.h, &m);
    assert_eq!(code, ResultCode::Done);
    got
}

/// C→S 0x30 (chat end with the NPC).
fn chat_end(f: &mut Fx, n: UnitId) -> Vec<Vec<u8>> {
    let mut m = vec![0x30, 1, 0, 0, 0];
    m.extend_from_slice(&f.guid(n).to_le_bytes());
    let (code, got) = send(&mut f.h, &m);
    assert_eq!(code, ResultCode::Done);
    got
}

/// What the quest calls raised directly sent, in order.
fn taken(f: &mut Fx) -> Vec<Vec<u8>> {
    let sent = std::mem::take(&mut f.world().rest.sent);
    sent.into_iter().map(|m| m.1).collect()
}

fn word(f: &Fx, slot: usize) -> u16 {
    let r = f.record();
    u16::from_le_bytes([r[2 * slot], r[2 * slot + 1]])
}

fn state(f: &mut Fx, chain: u8) -> (u8, u8) {
    let r = f.world().quests.record(chain).unwrap();
    (r.state, r.status)
}

fn level_change(f: &mut Fx, old: u32, new: u32) -> Vec<Vec<u8>> {
    let p = f.player;
    quests!(f, |ctl, w| ctl.changed_level(&mut w, p, old, new));
    taken(f)
}

/// A linked monster of `chain` dies, killed by the player.
fn kill(f: &mut Fx, chain: u8, victim: UnitId) -> Vec<Vec<u8>> {
    let p = f.player;
    f.world()
        .rest
        .chains
        .insert(victim, QuestChain(vec![chain]));
    quests!(f, |ctl, w| ctl.monster_killed(&mut w, victim, Some(p)));
    taken(f)
}

/// `n` updater ticks.
fn ticks(f: &mut Fx, n: u32) -> Vec<Vec<u8>> {
    for _ in 0..n {
        quests!(f, |ctl, w| ctl.update(&mut w));
    }
    taken(f)
}

/// The fixture's stat data has no stats: the player gets an extended
/// list on the items tests' synthetic itemstatcost (every stat plain).
fn real_stats(f: &mut Fx) {
    let p = f.player;
    let guid = f.guid(p);
    let sys = &mut f.h.game.events.sys;
    sys.stats = StatLists::new(crate::adapters::handlers::items::moves::tests::stat_data());
    sys.stats
        .alloc_extended(&mut sys.hooks, p, UnitType::Player, guid, 1, 0, None);
}

fn set_base_stat(f: &mut Fx, unit: UnitId, stat: u16, v: i32) {
    let sys = &mut f.h.game.events.sys;
    sys.stats.unit_set(&mut sys.hooks, unit, stat, v, 0);
}

/// The quest slots `slots` completed before (bit 0) in the entering
/// player's record: a completed quest switches its chain off.
fn done(slots: &[u8]) -> impl FnOnce(&mut PlayerQuests) + '_ {
    move |q| {
        for &s in slots {
            q.flags[0].set(s, 0);
        }
    }
}

// Den of Evil (A1Q1): every state from creation to the reward.
#[test]
fn den_of_evil_through_every_state() {
    let mut f = Fx::new(|_| {});
    real_stats(&mut f);
    let (akara, p) = (f.akara, f.player);
    // State 1 at creation; the sequence at game entry stops there.
    assert_eq!(state(&mut f, 1), (1, 0));
    assert_eq!(state(&mut f, 2), (0, 0));
    // 64 → state 2 (bit 2); the chat end → status 1 to the player.
    say(&mut f, akara, 64);
    assert_eq!(state(&mut f, 1), (2, 0));
    assert_eq!(chat_end(&mut f, akara), [hex("5d 01 00 01 0000")]);
    assert_eq!(word(&f, 1), 0x0004);
    // Leaving town: state 3, bit 3 (status already 1), nothing sent.
    assert!(level_change(&mut f, 1, 2).is_empty());
    assert_eq!(state(&mut f, 1), (3, 1));
    assert_eq!(word(&f, 1), 0x000C);
    // Entering the Den: status 2, bit 4.
    assert_eq!(level_change(&mut f, 2, 8), [hex("5d 01 00 02 0000")]);
    assert_eq!(word(&f, 1), 0x001C);
    // Three left in a fully visited Den: 0x20, status 4, left 3.
    let victim = alloc(&mut f, UnitType::Monster, 1);
    f.world().rest.den = (40, 37, 10, 10);
    assert_eq!(kill(&mut f, 1, victim), [hex("5d 01 20 04 0300")]);
    // Cleared: state 4, the killer 13 + 1, game 1.13, 0x28 + `89 00`,
    // sound 35, the timer.
    let victim = alloc(&mut f, UnitType::Monster, 1);
    f.world().rest.den = (40, 40, 10, 10);
    f.take_log();
    let got = kill(&mut f, 1, victim);
    assert_eq!(got.len(), 2);
    assert_eq!((got[0][0], &got[1][..]), (0x28, &[0x89u8, 0][..]));
    assert_eq!(state(&mut f, 1).0, 4);
    assert_eq!(word(&f, 1), 0x201E);
    assert!(f.game_record()[3] & 0x20 != 0);
    assert_eq!(f.take_log(), [format!("sound {} 35", p.0)]);
    // The timer: status 5 nine updater ticks later, once.
    assert!(ticks(&mut f, 8).is_empty());
    assert_eq!(ticks(&mut f, 1), [hex("5d 01 00 05 0000")]);
    assert!(ticks(&mut f, 30).is_empty());
    // 76: the reward on the real stat list, the sequence opens chain 2.
    let got = say(&mut f, akara, 76);
    assert_eq!(got.iter().map(|m| m[0]).collect::<Vec<_>>(), [0x27, 0x29]);
    assert_eq!(state(&mut f, 1), (5, 13));
    assert_eq!(state(&mut f, 2).0, 1);
    assert_eq!(word(&f, 1), 0x2001);
    assert_eq!(word(&f, 41), 0x2002);
    assert_eq!(f.h.game.events.sys.stats.unit_base(p, 5, 0), 1);
    assert!(f.world().quests.faults.is_empty());
    assert_eq!(f.errors(), Vec::<String>::new());
}

// Sisters' Burial Grounds (A1Q2): every state from creation to the reward.
#[test]
fn burial_grounds_through_every_state() {
    // Den of Evil done by the first player: switched off, so its
    // sequence passes the call on and opens chain 2.
    let mut f = Fx::new(done(&[1]));
    let (kashya, p) = (f.kashya, f.player);
    assert_eq!(state(&mut f, 2), (1, 0));
    say(&mut f, kashya, 81);
    assert_eq!(state(&mut f, 2).0, 2);
    assert_eq!(word(&f, 2), 0x0004);
    assert_eq!(chat_end(&mut f, kashya), [hex("5d 02 00 01 0000")]);
    assert!(level_change(&mut f, 1, 2).is_empty());
    assert_eq!((state(&mut f, 2), word(&f, 2)), ((3, 1), 0x000C));
    assert_eq!(level_change(&mut f, 2, 17), [hex("5d 02 00 02 0000")]);
    assert_eq!(word(&f, 2), 0x001C);
    // Blood Raven dies with the player near her: 13 + 1, sound 34, the
    // game's 2.13, timer 15.
    let raven = alloc(&mut f, UnitType::Monster, 267);
    f.world().rest.near = vec![p];
    f.take_log();
    assert!(kill(&mut f, 2, raven).is_empty());
    assert_eq!(state(&mut f, 2).0, 4);
    assert_eq!(word(&f, 2), 0x201E);
    assert_eq!(f.take_log(), [format!("sound {} 34", p.0)]);
    assert!(f.game_record()[5] & 0x20 != 0);
    assert!(ticks(&mut f, 15).is_empty());
    assert_eq!(ticks(&mut f, 1), [hex("5d 02 00 03 0000")]);
    // 92: state 5, status 13, chain 4 opens; 2.0 with bits 2–4 kept;
    // 0x28, the mercenary's 0x50 (NPC control), then the text refresh.
    let got = say(&mut f, kashya, 92);
    assert_eq!(
        got.iter().map(|m| m[0]).collect::<Vec<_>>(),
        [0x28, 0x50, 0x27, 0x29]
    );
    assert_eq!(state(&mut f, 2), (5, 13));
    assert_eq!(state(&mut f, 4).0, 1);
    assert_eq!(word(&f, 2), 0x201D);
    let hire = f.world().npc.record(class::KASHYA).unwrap().hire.as_ref();
    assert!(hire.unwrap().slots.iter().any(|s| s.hired));
    assert!(f.world().quests.faults.is_empty());
    assert_eq!(f.errors(), Vec::<String>::new());
}

/// Item tables with the Act I quest items: 0 Horadric Malus, 1 the
/// Inifuss scroll, 2 the deciphered scroll.
fn quest_tables() -> ItemTables {
    let rec = |code: &[u8; 4], quest| ItemRec {
        code: *code,
        level: 1,
        quest,
        ..ItemRec::default()
    };
    ItemTables {
        items: vec![rec(b"hdm ", 4), rec(b"bks ", 5), rec(b"bkd ", 5)],
        ..ItemTables::default()
    }
}

/// A real item of `quest_tables` row `index` (inventory mode).
fn quest_item(f: &mut Fx, index: i32) -> UnitId {
    f.world().tables = quest_tables();
    let mut rq = ItemRequest {
        item: index,
        format: 101,
        quality: q::NORMAL,
        ..ItemRequest::default()
    };
    let spawn = ItemSpawn {
        room: None,
        mode: 4,
        init_flags: 1,
    };
    let s = &mut f.h.game;
    s.world
        .with_economy(&mut s.game, &mut s.events, |econ, _| {
            econ.create_item(&mut rq, false, spawn)
        })
        .unwrap()
}

// The Tools of the Trade (A1Q3): every state from creation to the reward.
#[test]
fn tools_of_the_trade_through_every_state() {
    // Quests 1, 2 and 4 done: the walk 1 → 2 → 4 → 3 opens chain 3.
    let mut f = Fx::new(done(&[1, 2, 4]));
    real_stats(&mut f);
    let p = f.player;
    assert_eq!(state(&mut f, 3), (1, 0));
    let charsi = npc(&mut f, class::CHARSI);
    say(&mut f, charsi, 146);
    assert_eq!(state(&mut f, 3).0, 2);
    assert_eq!(word(&f, 3), 0);
    assert_eq!(chat_end(&mut f, charsi), [hex("5d 03 00 01 0000")]);
    assert_eq!(word(&f, 3), 0x0004);
    assert!(level_change(&mut f, 1, 2).is_empty());
    assert_eq!((state(&mut f, 3), word(&f, 3)), ((3, 1), 0x000C));
    // The Malus object: known at init; operated at level 8 it drops `hdm `
    // and is shown taken; state 4.
    let malus = alloc(&mut f, UnitType::Monster, 2);
    quests!(f, |ctl, w| act1::malus_init(ctl, &mut w, malus));
    set_base_stat(&mut f, p, 12, 8);
    f.world().rest.drop_ok = true;
    f.take_log();
    quests!(f, |ctl, w| act1::malus_operate(ctl, &mut w, malus, p));
    assert_eq!(f.take_log(), [format!("drop {} hdm  2", malus.0)]);
    assert_eq!(f.world().rest.object_modes[&malus], 2);
    assert_eq!(state(&mut f, 3), (4, 1));
    // The player carries a real `hdm ` item: Charsi's 163 finishes it.
    let item = quest_item(&mut f, 0);
    f.world().rest.inventory.insert(p, vec![item]);
    let got = say(&mut f, charsi, 163);
    assert_eq!(
        got.iter().map(|m| m[0]).collect::<Vec<_>>(),
        [0x28, 0x27, 0x29]
    );
    let r = f.world().quests.record(3).unwrap();
    assert!(r.state == 5 && r.extra.rewarded && r.extra.malus_items == 0);
    assert_eq!(word(&f, 3), 0x200E);
    assert!(f.game_record()[7] & 0x20 != 0);
    let log = f.take_log();
    assert!(log.contains(&format!("delete {} hdm ", p.0)));
    // The sequence passed to chain 6: its opening timer (period 20).
    let timers = &f.world().quests.timers;
    assert_eq!(timers.last().map(|t| (t.chain, t.period)), Some((6, 20)));
    // The chat end: status 13 set; the status function reports 10 (3.1).
    assert_eq!(chat_end(&mut f, charsi), [hex("5d 03 00 0a 0000")]);
    assert_eq!(state(&mut f, 3), (5, 13));
    // The imbue: 3.0, the record inactive.
    quests!(f, |ctl, w| act1::imbue_granted(ctl, &mut w, p));
    assert_eq!(word(&f, 3), 0x200D);
    assert!(!f.world().quests.record(3).unwrap().active);
    assert!(f.world().quests.faults.is_empty());
    assert_eq!(f.errors(), Vec::<String>::new());
}

// The Search for Cain (A1Q4): every state from creation to the reward.
#[test]
fn search_for_cain_through_every_state() {
    // Quests 1 and 2 done: the walk 1 → 2 → 4 opens chain 4.
    let mut f = Fx::new(done(&[1, 2]));
    let (akara, p) = (f.akara, f.player);
    assert_eq!(state(&mut f, 4), (1, 0));
    say(&mut f, akara, 97);
    assert_eq!(state(&mut f, 4).0, 2);
    assert_eq!(chat_end(&mut f, akara), [hex("5d 04 00 01 0000")]);
    assert_eq!(word(&f, 4), 0x0004);
    assert!(level_change(&mut f, 1, 2).is_empty());
    assert_eq!(state(&mut f, 4).0, 3);
    // The Inifuss tree (a unit standing in for the object; modes staged):
    // the scroll drops, state 4, status 2.
    let tree = alloc(&mut f, UnitType::Monster, 2);
    f.world().rest.drop_ok = true;
    f.take_log();
    quests!(f, |ctl, w| act1::q4::tree_operate(ctl, &mut w, tree, p));
    assert_eq!(
        f.take_log(),
        [
            format!("sound {} 45", p.0),
            format!("drop {} bks  2", tree.0)
        ]
    );
    assert_eq!(taken(&mut f), [hex("5d 04 00 02 0000")]);
    assert_eq!(state(&mut f, 4), (4, 2));
    // Akara deciphers the real `bks `: the staged `bkd ` comes back,
    // state 5, status 3 (sent at the chat end).
    let bks = quest_item(&mut f, 1);
    let bkd = quest_item(&mut f, 2);
    f.world().rest.inventory.insert(p, vec![bks]);
    f.world().rest.reward = Some(bkd);
    say(&mut f, akara, 112);
    assert_eq!(state(&mut f, 4), (5, 3));
    assert_eq!(f.world().rest.inventory[&p], [bkd]);
    assert_eq!(chat_end(&mut f, akara), [hex("5d 04 00 03 0000")]);
    assert_eq!(word(&f, 4), 0x000C);
    // The stones in the quest seed's order: status 4, 4.4, `89 01`.
    let order = quests!(f, |ctl, _w| act1::stone_order(ctl));
    let stones: Vec<UnitId> = (0..5)
        .map(|_| alloc(&mut f, UnitType::Monster, 2))
        .collect();
    // The k-th touch must be the stone of value order[k].
    for (k, &s) in stones.iter().enumerate() {
        let v = u16::from(order[k]);
        quests!(f, |ctl, w| act1::q4::stone_operate(ctl, &mut w, s, p, v));
    }
    let got = taken(&mut f);
    assert_eq!(got[0], hex("5d 04 00 04 0000"));
    assert_eq!((got[1][0], &got[2][..]), (0x28, &[0x89u8, 1][..]));
    assert!(f.world().quests.record(4).unwrap().extra.q4.b4f);
    assert_eq!(word(&f, 4), 0x001C);
    // Cain freed at the gibbet (the object function that sets those bits is
    // not run here; they are staged), then Akara's reward: state 6, status 13, the ring, the
    // sequence opens chain 3.
    f.world().rest.quests.get_mut(&p).unwrap().flags[0].set(4, 13);
    f.world().rest.quests.get_mut(&p).unwrap().flags[0].set(4, 1);
    f.take_log();
    let got = say(&mut f, akara, 118);
    assert_eq!(
        got.iter().map(|m| m[0]).collect::<Vec<_>>(),
        [0x28, 0x5D, 0x27, 0x29]
    );
    assert_eq!(got[1], hex("5d 04 02 00 0000"));
    assert_eq!(state(&mut f, 4), (6, 13));
    assert_eq!(state(&mut f, 3).0, 1);
    assert!(f.take_log().contains(&"reward rin  7 4".to_string()));
    assert_eq!(word(&f, 4) & 0x2003, 0x2001);
    assert!(f.world().quests.faults.is_empty());
    assert_eq!(f.errors(), Vec::<String>::new());
}

// The Forgotten Tower (A1Q5): every state from creation to the reward.
#[test]
fn forgotten_tower_through_every_state() {
    let mut f = Fx::new(|_| {});
    let (kashya, p) = (f.kashya, f.player);
    assert_eq!(state(&mut f, 5), (0, 0));
    // The tome (a unit standing in for the object): state 2, read before
    // any status; message 127 then sends status 1.
    let tome = alloc(&mut f, UnitType::Monster, 2);
    quests!(f, |ctl, w| act1::q5::tome_operate(ctl, &mut w, tome, p));
    assert_eq!(f.world().rest.object_modes[&tome], 1);
    assert_eq!(state(&mut f, 5).0, 2);
    let mut m = vec![0x31];
    m.extend_from_slice(&u32::MAX.to_le_bytes());
    m.extend_from_slice(&127u16.to_le_bytes());
    m.extend_from_slice(&[0, 0]);
    let (_, got) = send(&mut f.h, &m);
    // The tome's scroll text 127 (S→C 0x27 type 2) left with the operate,
    // then the status.
    let mut scroll = hex("27 02 03000000 01 00 00 00 7f00");
    scroll.resize(40, 0);
    assert_eq!(got, [scroll, hex("5d 05 00 01 0000")]);
    assert_eq!(word(&f, 5), 0x0004);
    // The tower (status 1 → 4), Tower Cellar 5 (state 3, status 2).
    assert_eq!(level_change(&mut f, 3, 20), [hex("5d 05 00 04 0000")]);
    assert_eq!(level_change(&mut f, 24, 25), [hex("5d 05 00 02 0000")]);
    assert_eq!((state(&mut f, 5), word(&f, 5)), ((3, 2), 0x0014));
    // The Countess dies with the player in the cellar: 5.13 and 5.0 at
    // once, sound 37, state 5, timer 7 → status 13.
    f.world().rest.levels.insert(p, 25);
    let countess = alloc(&mut f, UnitType::Monster, 45);
    f.take_log();
    assert!(kill(&mut f, 5, countess).is_empty());
    assert_eq!(state(&mut f, 5).0, 5);
    assert_eq!(word(&f, 5), 0x2015);
    let log = f.take_log();
    assert!(log.contains(&format!("sound {} 37", p.0)));
    // The trap step runs: no chest listed, so
    // nothing is spawned and nothing of chain 5 is reported.
    assert!(!log.iter().any(|l| l.starts_with("unhandled 5")));
    assert!(ticks(&mut f, 7).is_empty());
    assert_eq!(ticks(&mut f, 1), [hex("5d 05 00 0d 0000")]);
    // The report to Kashya (142): the sequence opens chain 3; the player
    // moves from list B to list A.
    say(&mut f, kashya, 142);
    assert_eq!(state(&mut f, 3).0, 1);
    let x = &f.world().quests.record(5).unwrap().extra.q5;
    assert!(x.credited.is_empty() && x.reported.len() == 1);
    assert!(f.world().quests.faults.is_empty());
    assert_eq!(f.errors(), Vec::<String>::new());
}

// Sisters to the Slaughter (A1Q6): every state from creation to the reward.
#[test]
fn sisters_to_the_slaughter_through_every_state() {
    // Quests 1–4 done: the walk reaches chain 6, whose timer (period 20)
    // opens it 21 updater ticks later.
    let mut f = Fx::new(done(&[1, 2, 3, 4]));
    let p = f.player;
    assert_eq!(state(&mut f, 6), (0, 0));
    ticks(&mut f, 20);
    assert_eq!(state(&mut f, 6).0, 0);
    ticks(&mut f, 1);
    assert_eq!(state(&mut f, 6).0, 1);
    let cain = npc(&mut f, class::CAIN5);
    let warriv = npc(&mut f, class::WARRIV1);
    say(&mut f, cain, 166);
    assert_eq!((state(&mut f, 6).0, word(&f, 6)), (2, 0x0004));
    assert_eq!(chat_end(&mut f, cain), [hex("5d 06 00 01 0000")]);
    assert!(level_change(&mut f, 1, 34).is_empty());
    assert_eq!(state(&mut f, 6).0, 3);
    assert_eq!(level_change(&mut f, 36, 37), [hex("5d 06 00 02 0000")]);
    // Andariel dies, killed by the player in Catacombs 4: the credit, the
    // three gem drops, state 4, the portal timer.
    f.world().rest.levels.insert(p, 37);
    let andariel = alloc(&mut f, UnitType::Monster, 156);
    f.take_log();
    assert!(kill(&mut f, 6, andariel).is_empty());
    assert_eq!(state(&mut f, 6).0, 4);
    assert_eq!(word(&f, 6), 0x201E);
    let log = f.take_log();
    assert_eq!(log.iter().filter(|l| l.starts_with("drop ")).count(), 3);
    assert!(log.contains(&format!("sound {} 33", p.0)));
    // The credit's progression is applied to the player's client (classic,
    // difficulty 0: n = 1).
    assert_eq!(f.world().rest.client_flags.get(&p), Some(&0x0100));
    // Status 3 at the 11th firing (22 updater ticks); the portal step
    // (counter 10) needs the player's position (no path seam here).
    assert!(ticks(&mut f, 21).is_empty());
    assert_eq!(ticks(&mut f, 1), [hex("5d 06 00 03 0000")]);
    // Warriv 183: status 13, state 5, game 6.13, 6.0.
    let got = say(&mut f, warriv, 183);
    assert_eq!(
        got.iter().map(|m| m[0]).collect::<Vec<_>>(),
        [0x27, 0x29, 0x28]
    );
    assert_eq!(state(&mut f, 6), (5, 13));
    assert_eq!(word(&f, 6), 0x201D);
    assert!(f.game_record()[13] & 0x20 != 0);
    assert!(f.world().quests.faults.is_empty());
    assert_eq!(f.errors(), Vec::<String>::new());
}
