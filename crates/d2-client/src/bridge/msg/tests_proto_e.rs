//! Proto contract tests (q-proto-audit, part e): every C→S message the
//! client builds by hand has the bytes `d2_proto::client` encodes for the
//! same field values (`client-messages.tsv` layout), `d2_proto` decodes
//! the values back, and the server side (the dispatcher's exact-size
//! check, gate and point / unit parsers,–,
//! and the `d2-sim` / `d2-server` field parsers) reads the same values.

use std::cell::Cell;

use d2_proto::client::*;
use d2_proto::schema::{HandlerSize, Kind as TsvKind, SizeRule};
use d2_proto::{FixedMessage, CLIENT_MESSAGES};
use d2_server::adapters::ProtoSizes;
use d2_server::buffers::ClientBuffers;
use d2_server::dispatch::{self, dispatch};
use d2_server::seams::{
    ClientId, Intents, MessageSink, PlayerGate, PlayerLookup, PointState, Pos, ResultCode,
    UnitTarget,
};

use super::super::check::{check, Checked};
use super::super::intent::{self, encode};
use super::super::objects::interact::interact_bytes;
use super::super::predict::{walk_of, WalkTo};
use super::super::world::{ClientUnit, ClientWorld, ModelInputs, UnitKey, PLAYER};
use crate::controls::click::code_bytes;
use crate::ui::messages::msg_u32s;
use crate::ui::panel::ClientIntent;
use crate::ui::panels::npc::{msg_chat_end, msg_chat_start, msg_quest};
use crate::ui::panels::shop::{Pending, SendFacts, ShopEffect, ShopTx, TxKind, MK_SHIFT};

// ------------------------------------------------------------ sweeps

const EDGE32: [u32; 13] = [
    0,
    1,
    0x7F,
    0x80,
    0xFF,
    0x100,
    0x7FFF,
    0x8000,
    0xFFFF,
    0x1_0000,
    0x7FFF_FFFF,
    0x8000_0000,
    u32::MAX,
];

fn xorshift(mut s: u32, n: usize) -> Vec<u32> {
    (0..n)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            s
        })
        .collect()
}

fn u32s() -> Vec<u32> {
    let mut v = EDGE32.to_vec();
    v.extend(xorshift(0x2545_F491, 32));
    v
}

fn u16s() -> Vec<u16> {
    let mut v = vec![0, 1, 0x7F, 0x80, 0xFF, 0x100, 0x7FFF, 0x8000, 0xFFFF];
    v.extend(xorshift(0x9E37_79B9, 32).into_iter().map(|w| w as u16));
    v
}

/// Each value with itself and with its neighbour (both orders).
fn pairs<T: Copy>(v: &[T]) -> Vec<(T, T)> {
    let n = v.len();
    let mut out = Vec::new();
    for i in 0..n {
        out.push((v[i], v[i]));
        out.push((v[i], v[(i + 1) % n]));
        out.push((v[(i + 1) % n], v[i]));
    }
    out
}

/// The bytes of a typed message as the bridge sends them, after checking
/// the client's own route (game queue, size rule = length).
fn proto<M: FixedMessage>(m: &M) -> Vec<u8> {
    let b = encode(m);
    assert_eq!(b.len(), M::SIZE);
    assert!(intent::route(&b).is_ok(), "{:#04x} not sendable", M::ID);
    assert_eq!(
        d2_proto::transport::client_size(&b),
        d2_proto::schema::Size::Bytes(M::SIZE),
        "{:#04x}",
        M::ID
    );
    b
}

// ------------------------------------------------------- server fake

const ALIVE: PlayerGate = PlayerGate {
    mode: 1,
    uninterruptable: false,
};
const DEAD: PlayerGate = PlayerGate {
    mode: dispatch::MODE_DEAD,
    uninterruptable: false,
};

/// The dispatcher's game seam, recording what the parsers read.
#[derive(Default)]
struct Server {
    /// The player position of the point parser (`None`: no player data).
    player: Option<Pos>,
    last_accept: i32,
    frame: i32,
    /// (type, id) the unit parser looked up.
    unit: Cell<Option<(u32, u32)>>,
    accepted: Option<i32>,
    resyncs: usize,
    handled: Vec<Vec<u8>>,
}

impl Intents for Server {
    fn player(&self, _: ClientId) -> PlayerLookup {
        PlayerLookup::Player(ALIVE)
    }
    fn frame(&self) -> i32 {
        self.frame
    }
    fn point_state(&self, _: ClientId) -> Option<PointState> {
        self.player.map(|player| PointState {
            player,
            last_accept: self.last_accept,
        })
    }
    fn set_point_accept(&mut self, _: ClientId, frame: i32) {
        self.accepted = Some(frame);
    }
    fn queue_resync(&mut self, _: ClientId, _: &mut dyn MessageSink) {
        self.resyncs += 1;
    }
    fn unit_target(&self, _: ClientId, unit_type: u32, unit_id: u32) -> UnitTarget {
        self.unit.set(Some((unit_type, unit_id)));
        let p = Pos { x: 0, y: 0 };
        UnitTarget::At {
            player: p,
            target: p,
        }
    }
    fn handle(
        &mut self,
        _: ClientId,
        msg: &[u8],
        size: usize,
        _: &mut dyn MessageSink,
    ) -> ResultCode {
        assert_eq!(msg.len(), size);
        self.handled.push(msg.to_vec());
        ResultCode::Done
    }
    fn clients(&self) -> Vec<ClientId> {
        vec![0]
    }
}

fn serve(s: &mut Server, gate: PlayerGate, msg: &[u8]) -> ResultCode {
    let mut out = ClientBuffers::new();
    dispatch(s, &ProtoSizes, &mut out, 0, gate, msg, msg.len())
}

/// The message reached the handler unchanged.
fn reaches_handler(msg: &[u8]) {
    let mut s = Server {
        player: Some(Pos { x: 0, y: 0 }),
        ..Server::default()
    };
    assert_eq!(serve(&mut s, ALIVE, msg), ResultCode::Done, "{msg:02x?}");
    assert_eq!(s.handled, vec![msg.to_vec()], "{msg:02x?}");
}

// ------------------------------------------------------------- tests

/// Point codes: `controls::click::code_bytes` =
/// the typed message; the point parser reads x u16@1, y
/// u16@3 (accepts at the player's own point, refuses at 51 and asks the
/// resync after 25 frames).
#[test]
fn point_messages_0x01_0x03_0x05_0x08_0x0c_0x0f_one_layout() {
    type Enc = fn(u16, u16) -> Vec<u8>;
    let cases: [(u8, Enc); 6] = [
        (0x01, |x, y| proto(&Walk { x, y })),
        (0x03, |x, y| proto(&Run { x, y })),
        (0x05, |x, y| proto(&ShiftLeftSkill { x, y })),
        (0x08, |x, y| proto(&ShiftLeftSkillHold { x, y })),
        (0x0C, |x, y| proto(&RightSkill { x, y })),
        (0x0F, |x, y| proto(&RightSkillHold { x, y })),
    ];
    for (id, enc) in cases {
        assert!(dispatch::is_point(id));
        for (x, y) in pairs(&u16s()) {
            let want = enc(x, y);
            // The click record carries u32s; the builder keeps the low 16.
            let hi = 0xABCD_0000;
            assert_eq!(
                code_bytes(id, u32::from(x), u32::from(y)),
                Some(want.clone())
            );
            assert_eq!(
                code_bytes(id, hi | u32::from(x), hi | u32::from(y)),
                Some(want.clone())
            );
            let back = Walk::decode(&[&[0x01], &want[1..]].concat()).unwrap();
            assert_eq!((back.x, back.y), (x, y));
            if id == 0x01 || id == 0x03 {
                let w = walk_of(&want).unwrap();
                assert_eq!((w.to, w.run), (WalkTo::Point(x, y), id == 0x03));
            }
            // In range at the player's own point.
            let mut s = Server {
                player: Some(Pos {
                    x: i32::from(x),
                    y: i32::from(y),
                }),
                frame: 7,
                ..Server::default()
            };
            assert_eq!(serve(&mut s, ALIVE, &want), ResultCode::Done);
            assert_eq!((s.accepted, s.handled.len()), (Some(7), 1));
            // 51 sub-tiles off on x, 26 frames since the last accept.
            let mut s = Server {
                player: Some(Pos {
                    x: i32::from(x) + 51,
                    y: i32::from(y),
                }),
                frame: 26,
                ..Server::default()
            };
            assert_eq!(serve(&mut s, ALIVE, &want), ResultCode::Refused);
            assert_eq!((s.accepted, s.resyncs, s.handled.len()), (None, 1, 0));
            // 50 off on y is in range.
            let mut s = Server {
                player: Some(Pos {
                    x: i32::from(x),
                    y: i32::from(y) - 50,
                }),
                ..Server::default()
            };
            assert_eq!(serve(&mut s, ALIVE, &want), ResultCode::Done);
        }
    }
}

/// Unit codes: `code_bytes` = the typed message; the unit
/// parser looks up (type u32@1, id u32@5); type ≥ 6 → 2.
#[test]
fn unit_messages_0x02_0x04_0x06_0x07_0x09_0x0a_0x0d_0x0e_0x10_0x11_one_layout() {
    type Enc = fn(u32, u32) -> Vec<u8>;
    let cases: [(u8, Enc); 10] = [
        (0x02, |type_, id| proto(&WalkToUnit { type_, id })),
        (0x04, |type_, id| proto(&RunToUnit { type_, id })),
        (0x06, |type_, id| proto(&LeftSkillOnUnit { type_, id })),
        (0x07, |type_, id| proto(&ShiftLeftSkillOnUnit { type_, id })),
        (0x09, |type_, id| proto(&LeftSkillOnUnitHold { type_, id })),
        (0x0A, |type_, id| {
            proto(&ShiftLeftSkillOnUnitHold { type_, id })
        }),
        (0x0D, |type_, id| proto(&RightSkillOnUnit { type_, id })),
        (0x0E, |type_, id| {
            proto(&ShiftRightSkillOnUnit { type_, id })
        }),
        (0x10, |type_, id| proto(&RightSkillOnUnitHold { type_, id })),
        (0x11, |type_, id| {
            proto(&ShiftRightSkillOnUnitHold { type_, id })
        }),
    ];
    for (id, enc) in cases {
        assert!(dispatch::is_unit(id));
        for g in u32s() {
            for t in [0u32, 1, 2, 3, 4, 5, 6, 0x100, u32::MAX] {
                let want = enc(t, g);
                assert_eq!(code_bytes(id, t, g), Some(want.clone()));
                let back = WalkToUnit::decode(&[&[0x02], &want[1..]].concat()).unwrap();
                assert_eq!((back.type_, back.id), (t, g));
                if (id == 0x02 || id == 0x04) && t < 6 {
                    let w = walk_of(&want).unwrap();
                    assert_eq!(
                        (w.to, w.run),
                        (WalkTo::Unit(UnitKey::new(t as u8, g)), id == 0x04)
                    );
                }
                let mut s = Server::default();
                let r = serve(&mut s, ALIVE, &want);
                if t < 6 {
                    assert_eq!(r, ResultCode::Done);
                    assert_eq!(s.unit.get(), Some((t, g)));
                    assert_eq!(s.handled, vec![want]);
                } else {
                    assert_eq!(r, ResultCode::Invalid);
                    assert_eq!((s.unit.get(), s.handled.len()), (None, 0));
                }
            }
        }
    }
}

/// sends ids 1–0x11 only; 0x0B, 0x12 and 0x13 send nothing
/// through `code_bytes` (0x13 is the interact sender).
#[test]
fn click_codes_send_nothing_for_0x00_0x0b_0x12_0x13() {
    for id in [0x00, 0x0B, 0x12, 0x13, 0x14, 0xFF] {
        assert_eq!(code_bytes(id, 1, 2), None, "{id:#04x}");
    }
}

/// C→S 0x13 `{0x13, type u32, GUID u32}` = the typed
/// message; not a unit-parser id, so the dispatcher passes it whole.
#[test]
fn interact_0x13_one_layout() {
    assert!(!dispatch::is_unit(0x13) && !dispatch::is_point(0x13));
    for (t, g) in pairs(&u32s()) {
        let want = proto(&InteractWithEntity { type_: t, id: g });
        assert_eq!(interact_bytes(t, g), want);
        let back = InteractWithEntity::decode(&want).unwrap();
        assert_eq!((back.type_, back.id), (t, g));
        reaches_handler(&want);
    }
}

/// C→S 0x16 from the bridge's item builders and the corpse click; the
/// `d2-sim` item handlers take the TSV size of every item-move id.
#[test]
fn item_moves_0x16_to_0x63_one_layout() {
    for g in u32s() {
        let p = proto(&super::super::items::pick(g, true));
        assert_eq!(
            p,
            [&[0x16, 4, 0, 0, 0][..], &g.to_le_bytes(), &[1, 0, 0, 0]].concat()
        );
        let c = proto(&crate::world_view::corpse_click::pick_corpse(g));
        assert_eq!(
            c,
            [&[0x16, 0, 0, 0, 0][..], &g.to_le_bytes(), &[0; 4]].concat()
        );
        reaches_handler(&p);
    }
    // Every id the item-move handler owns: its exact size is the TSV's.
    for &(id, size) in d2_sim::items::moves::HANDLED.iter() {
        let row = &CLIENT_MESSAGES[usize::from(id)];
        assert_eq!(row.transport_size.fixed(), Some(size), "{id:#04x}");
        assert_eq!(
            row.handler_size,
            HandlerSize::Exact(size as u16),
            "{id:#04x}"
        );
    }
}

/// The belt panel's `msg_u32s` 0x23 / 0x24 / 0x25 =
/// the typed messages.
// Checks: tables/sim/client-messages.tsv
#[test]
fn belt_0x23_0x24_0x25_one_layout() {
    for (a, b) in pairs(&u32s()) {
        let m = proto(&ItemToBelt { item: a, slot: b });
        assert_eq!(msg_u32s(0x23, &[a, b]).0, m);
        assert_eq!(
            ItemToBelt::decode(&m).unwrap(),
            ItemToBelt { item: a, slot: b }
        );
        let m = proto(&ItemFromBelt { item: a });
        assert_eq!(msg_u32s(0x24, &[a]).0, m);
        let m = proto(&SwitchBeltItem { cursor: a, belt: b });
        assert_eq!(msg_u32s(0x25, &[a, b]).0, m);
        reaches_handler(&m);
    }
}

/// The talk messages (`ui/panels/npc.rs`): 0x2F [type][GUID], 0x30 [1]
/// [GUID] (step 5), 0x31 [GUID][message u16 zero-extended];
/// the server reads id u32@5 (0x2F / 0x30) and npc u32@1, msg u16@5
/// (0x31).
#[test]
fn npc_talk_0x2f_0x30_0x31_one_layout() {
    for (t, g) in pairs(&u32s()) {
        let open = msg_chat_start(t, g).to_vec();
        assert_eq!(InitEntityChat::decode(&open).unwrap().id, g);
        assert_eq!(open[1..5], t.to_le_bytes());
        reaches_handler(&open);
        let close = msg_chat_end(g).to_vec();
        assert_eq!(TerminateEntityChat::decode(&close).unwrap().id, g);
        assert_eq!(close[1..5], 1u32.to_le_bytes());
        reaches_handler(&close);
    }
    for (g, q) in pairs(&u32s()).into_iter().zip(u16s().into_iter().cycle()) {
        let m = proto(&QuestMessage { npc: g.0, msg: q });
        assert_eq!(msg_quest(g.0, q).to_vec(), m);
        // The dialog close: [0xFFFFFFFF][id zero-extended].
        assert_eq!(
            msg_u32s(0x31, &[u32::MAX, u32::from(q)]).0,
            proto(&QuestMessage {
                npc: u32::MAX,
                msg: q
            })
        );
        reaches_handler(&m);
    }
}

/// The shop's close (→ `E(G)` step 5) sends 0x30 [u32 1]
/// [u32 G] like every interaction end (the shop used to write u32@1 = 0).
#[test]
fn shop_close_0x30_one_layout() {
    let p = crate::ui::panels::shop::ShopPanel {
        tabs: crate::ui::panels::shop::DEFAULT_TABS,
        buttons: Vec::new(),
        captions_hidden: false,
        npc_guid: 7,
    };
    assert_eq!(
        p.close_intent(),
        vec![crate::ui::panels::PanelOutput::Intent(ClientIntent(
            msg_chat_end(7).to_vec()
        ))]
    );
}

fn shop_send(kind: TxKind, repair_all: bool, gamble: bool, flags: u32, f: SendFacts) -> Vec<u8> {
    let mut t = ShopTx {
        npc_guid: 0x1122_3344,
        gamble,
        pending: Some(Pending {
            kind,
            item_guid: if repair_all { 0 } else { 0x5566_7788 },
            item_class: 3,
            price: 0x99AA_BBCC,
            t: 0,
            repair_all,
        }),
        ..ShopTx::default()
    };
    let e = t.send_with(flags, 1_000_000, &f);
    let sent: Vec<Vec<u8>> = e
        .into_iter()
        .filter_map(|e| match e {
            ShopEffect::Send(ClientIntent(b)) => Some(b),
            _ => None,
        })
        .collect();
    assert_eq!(sent.len(), 1, "{kind:?}");
    sent[0].clone()
}

/// Buy / sell / repair = the typed
/// messages; the `d2-sim` parsers read
/// the transaction bits 0–15 and bit 31, the mode u16@9 and the repair
/// flag bit 31 of u32@13.
#[test]
fn shop_0x32_0x33_0x35_one_layout() {
    use d2_sim::world::vendors::trade::{BuyMsg, RepairMsg, SellMsg};
    for m in u16s() {
        for d in [0u32, 1, 41, 0x7FFF_FFFF] {
            let f = SendFacts {
                item_found: true,
                item_mode: m,
                durability: d,
                item_flag_1a5: true,
                item_is_cursor: false,
            };
            for (gamble, shift) in [(false, false), (true, false), (false, true), (true, true)] {
                let flags = if shift { MK_SHIFT } else { 0 };
                let b = shop_send(TxKind::Buy, false, gamble, flags, f);
                let w = (u32::from(m) << 16)
                    | if gamble { 2 } else { 0 }
                    | if shift { 0x8000_0000 } else { 0 };
                let want = proto(&BuyItem {
                    npc: 0x1122_3344,
                    item: 0x5566_7788,
                    transaction: w,
                    client_price: 0x99AA_BBCC,
                });
                assert_eq!(b, want);
                let s = BuyMsg::parse(&b).unwrap();
                assert_eq!(
                    (s.npc, s.item, s.txn, s.fill, s.client_price),
                    (
                        0x1122_3344,
                        0x5566_7788,
                        w & 0xFFFF,
                        w & 0x8000_0000 != 0,
                        0x99AA_BBCC
                    )
                );
                // A mode ≥ 0x8000 shifted left 16 sets bit 31 (fill) as
                // in the base game (the menu writes `m << 16`).
                assert_eq!(s.fill, shift || m >= 0x8000);
                // Bits 16–30 (the item mode) never reach the transaction.
                assert_eq!(s.txn, if gamble { 2 } else { 0 });
                reaches_handler(&b);
            }
            let b = shop_send(TxKind::Sell, false, false, 0, f);
            let want = proto(&SellItem {
                npc: 0x1122_3344,
                item: 0x5566_7788,
                item_mode: m,
                client_price: 0x99AA_BBCC,
            });
            assert_eq!(b, want);
            let s = SellMsg::parse(&b).unwrap();
            assert_eq!((s.npc, s.item, s.mode), (0x1122_3344, 0x5566_7788, m));
            let b = shop_send(TxKind::Repair, false, false, 0, f);
            let want = proto(&Repair {
                npc: 0x1122_3344,
                item: 0x5566_7788,
                unread: m,
                repair_flags: d,
            });
            assert_eq!(b, want);
            let r = RepairMsg::parse(&b).unwrap();
            assert_eq!((r.npc, r.item, r.all), (0x1122_3344, 0x5566_7788, false));
        }
    }
    let b = shop_send(TxKind::Repair, true, false, 0, SendFacts::default());
    assert_eq!(
        b,
        proto(&Repair {
            npc: 0x1122_3344,
            item: 0,
            unread: 0,
            repair_flags: 0x8000_0000,
        })
    );
    let r = RepairMsg::parse(&b).unwrap();
    assert_eq!((r.item, r.all), (0, true));
    reaches_handler(&b);
}

/// Hire 0x36 [NPC][name u16
/// zero-extended] = the typed message (the server reads u16@5).
#[test]
fn hire_0x36_one_layout() {
    for (g, n) in u32s().into_iter().zip(u16s().into_iter().cycle()) {
        let m = proto(&HireMerc { npc: g, merc: n });
        assert_eq!(crate::ui::hire_list::hire_intent(g, n).0, m);
        assert_eq!(HireMerc::decode(&m).unwrap(), HireMerc { npc: g, merc: n });
        reaches_handler(&m);
    }
}

/// 0x3A [stat][count − 1] and the server's 3-byte
/// handler (it reads bytes 1 and 2).
#[test]
fn add_stat_point_0x3a_one_layout() {
    use crate::ui::panels::character::add_stat_point;
    for stat in [0u16, 1, 2, 3, 0x7F, 0xFF] {
        for n in [1, 2, 5, 32, 100, 256] {
            let m = proto(&AddStatPoint {
                stat: stat as u8,
                repeat: (n - 1) as u8,
            });
            assert_eq!(add_stat_point(stat, n).0, m);
            assert_eq!((m[1], m[2]), (stat as u8, (n - 1) as u8));
            reaches_handler(&m);
        }
    }
}

/// 0x3C and 0x51 bit fields: `d2_proto` encode = the
/// dispatcher's decoders; 0x3C has gate none (a dead player's select
/// still reaches the handler).
#[test]
fn select_skill_0x3c_bind_hotkey_0x51_one_layout() {
    for (skill, item) in pairs(&u32s()) {
        for left in [false, true] {
            let s = skill & 0x7FFF_FFFF;
            let m = proto(&SelectSkill {
                skill: s,
                left,
                item,
            });
            let d = dispatch::select_skill(&m).unwrap();
            assert_eq!((d.skill, d.left, d.item), (s, left, item));
            assert_eq!(
                u32::from_le_bytes(m[1..5].try_into().unwrap()),
                s | if left { 0x8000_0000 } else { 0 }
            );
            let mut srv = Server::default();
            assert_eq!(serve(&mut srv, DEAD, &m), ResultCode::Done);
            assert_eq!(srv.handled, vec![m]);
            let k = (skill & 0x7FFF) as u16;
            let slot = (item >> 16) as u16;
            let m = proto(&BindHotkey {
                skill: k,
                left,
                slot,
                item,
            });
            let d = dispatch::bind_hotkey(&m).unwrap();
            assert_eq!((d.skill, d.left, d.slot, d.item), (k, left, slot, item));
            reaches_handler(&m);
        }
    }
}

/// The one-field `msg_u32s` senders: 0x3E Inifuss (`hire.rs`), 0x4C
/// Transmogrify [−1], 0x44 orifice
/// ([player][object][item][action]), 0x49 waypoint
/// [wp][level u32].
#[test]
fn small_senders_0x3e_0x44_0x49_0x4c_one_layout() {
    for (a, b) in pairs(&u32s()) {
        let m = proto(&ActivateInifussScroll { item: a });
        assert_eq!(
            crate::ui::messages::hire::InifussScroll::default()
                .open_bkd(a)
                .0,
            m
        );
        reaches_handler(&m);
        let m = msg_u32s(0x44, &[a, b, a ^ b, u32::from(b as u16)]).0;
        assert_eq!(
            StaffInOrifice::decode(&m).unwrap(),
            StaffInOrifice {
                orifice: b,
                item: a ^ b,
                action: b as u16,
            }
        );
        reaches_handler(&m);
        let level = b & 0xFFFF;
        let m = proto(&TakeOrCloseWp {
            wp: a,
            level: level as u16,
        });
        assert_eq!(msg_u32s(0x49, &[a, level]).0, m);
        reaches_handler(&m);
    }
    let m = proto(&Transmogrify { item: u32::MAX });
    assert_eq!(msg_u32s(0x4C, &[u32::MAX]).0, m);
    reaches_handler(&m);
}

/// 0x40 and 0x41: one byte each; 0x41 has the dead gate.
#[test]
fn quest_data_0x40_resurrect_0x41_one_layout() {
    let q = proto(&RequestQuestData);
    assert_eq!(crate::ui::original::quest_log_ui::request_quest_data().0, q);
    reaches_handler(&q);
    let r = proto(&Resurrect);
    assert_eq!(crate::app::death::RESURRECT.to_vec(), r);
    let mut s = Server::default();
    assert_eq!(serve(&mut s, DEAD, &r), ResultCode::Done);
    assert_eq!(s.handled, vec![r.clone()]);
    // Alive: the gate returns 0 before the handler.
    let mut s = Server::default();
    assert_eq!(serve(&mut s, ALIVE, &r), ResultCode::Done);
    assert!(s.handled.is_empty());
}

/// The position check's C→S 0x5F = the typed
/// message; 0x5F is not a point-parser id, so a far
/// position still reaches the handler.
#[test]
fn resync_0x5f_one_layout() {
    assert!(!dispatch::is_point(0x5F));
    let p1 = UnitKey::new(PLAYER, 1);
    for (x, y) in pairs(&u16s()) {
        if x < 100 || y < 100 || x > 0xFF00 {
            continue;
        }
        let mut w = ClientWorld::default();
        let mut u = ClientUnit::new(p1);
        u.mode = 1;
        u.position = Some((x, y));
        w.units.insert(p1, u);
        w.local_player = Some(p1);
        let r = check(&mut w, &ModelInputs::default(), p1, x + 20, y, 0, 0, 0).unwrap();
        assert_eq!(r, Checked::Asked);
        let want = proto(&UpdatePlayerPos { x, y });
        assert_eq!(w.outgoing, vec![want.clone()]);
        let mut s = Server {
            player: Some(Pos {
                x: i32::from(x) + 1000,
                y: 0,
            }),
            ..Server::default()
        };
        assert_eq!(serve(&mut s, ALIVE, &want), ResultCode::Done);
        assert_eq!(s.handled, vec![want]);
    }
}

/// Every C→S id with a real handler: the dispatcher's exact-size check
/// (through `ProtoSizes`) passes exactly the TSV size and
/// refuses one byte more or less; transport and handler sizes agree.
#[test]
fn every_handler_id_exact_size_is_the_tsv_size_one_layout() {
    for row in CLIENT_MESSAGES
        .iter()
        .filter(|r| r.kind == TsvKind::Handler)
    {
        let id = row.id;
        if id == 0x14 || id == 0x15 {
            continue;
        }
        let SizeRule::Fixed(n) = row.transport_size else {
            panic!("{id:#04x}: handler with a variable size");
        };
        let n = usize::from(n);
        assert_eq!(row.handler_size, HandlerSize::Exact(n as u16), "{id:#04x}");
        assert_eq!(dispatch::kind(id), dispatch::Kind::Handler, "{id:#04x}");
        let gate = if id == 0x41 { DEAD } else { ALIVE };
        let mut m = vec![0u8; n];
        m[0] = id;
        let mut s = Server {
            player: Some(Pos { x: 0, y: 0 }),
            ..Server::default()
        };
        assert_eq!(serve(&mut s, gate, &m), ResultCode::Done, "{id:#04x}");
        assert_eq!(s.handled, vec![m.clone()], "{id:#04x}");
        for len in [n + 1, n.saturating_sub(1)] {
            if len == 0 {
                continue;
            }
            let mut b = vec![0u8; len];
            b[0] = id;
            let mut s = Server {
                player: Some(Pos { x: 0, y: 0 }),
                ..Server::default()
            };
            assert_eq!(
                serve(&mut s, gate, &b),
                ResultCode::Malformed,
                "{id:#04x} {len}"
            );
            assert!(s.handled.is_empty(), "{id:#04x} {len}");
        }
    }
}
