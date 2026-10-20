//! Proto contract tests (q-proto-audit, part b): S→C 0x2D–0x66 without
//! 0x3E. For each id: the d2-sim (or d2-server) builder's bytes equal the
//! d2-proto encode of the same fields, d2-proto decodes them back, and
//! the d2-client handler (driven through the dispatch) reads the
//! same values. Ids without a D2RUST producer start from the d2-proto
//! encode.

use d2_proto::s2c::{self, messages as built, Message as S2c, ServerMsg};
use d2_proto::server as gen;
use d2_proto::FixedMessage;

use super::super::output::Output;
use super::super::world::{
    ItemData, KindData, ObjectData, PlayerData, UnitKey, ITEM, MONSTER, OBJECT, PLAYER,
};
use super::support::Model;

const U8S: [u8; 5] = [0, 1, 0x7F, 0x80, 0xFF];
const U16S: [u16; 5] = [0, 1, 0x7FFF, 0x8000, 0xFFFF];
const U32S: [u32; 6] = [0, 1, 0x1122_3344, 0x7FFF_FFFF, 0x8000_0000, u32::MAX];

/// A fixed xorshift32 sweep (same seed as `conformance/tests/s2c_builders.rs`).
fn sweep(n: usize) -> impl Iterator<Item = u32> {
    let mut s = 0x2545_F491_u32;
    (0..n).map(move |_| {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        s
    })
}

/// Edge u32s then the sweep.
fn u32s() -> Vec<u32> {
    U32S.iter().copied().chain(sweep(16)).collect()
}

/// A fresh model with no rejection after receiving `b`.
fn recv_ok(m: &mut Model, b: &[u8]) {
    m.recv(b);
    assert_eq!(m.rejected(), Vec::<(u8, String)>::new(), "{b:02x?}");
}

fn local(m: &mut Model) {
    let p = UnitKey::new(PLAYER, 1);
    m.put(p).kind = KindData::Player(PlayerData::default());
    m.w.local_player = Some(p);
}

// ------------------------------------------------------------ size 0

#[test]
fn size0_ids_0x2d_0x66_one_layout() {
    // TSV size 0 (never receivable): the transport gives 0 for each.
    for id in (0x2D..=0x3D).chain([0x41, 0x43, 0x44, 0x46, 0x49, 0x4A, 0x4B, 0x55, 0x56, 0x64]) {
        assert_eq!(
            s2c::parse(&[id, 0, 0, 0, 0, 0, 0, 0]),
            Err(s2c::ParseError::Invalid { id }),
            "0x{id:02X}"
        );
    }
}

// ------------------------------------------------------------ items

#[test]
fn use_stackable_item_0x3f_one_layout() {
    for &code in &U8S {
        for &arg in &U16S {
            for item in u32s() {
                let sim = d2_sim::items::moves::layouts::use_stackable(code, item, arg);
                let p = gen::UseStackableItem { code, item, arg };
                assert_eq!(sim, p.encode());
                assert_eq!(s2c::parse(&sim), Ok(S2c::UseStackableItem(p)));
                if (code, arg) == (0xFF, 0xFFFF) {
                    continue;
                }
                let mut m = Model::default();
                let k = UnitKey::new(ITEM, item);
                m.put(k).kind = KindData::Item(ItemData::default());
                recv_ok(&mut m, &sim);
                let u = m.w.use_cursor.expect("use cursor");
                assert_eq!((u.item, u.code), (k, code));
                let KindData::Item(d) = &m.unit(k).kind else {
                    panic!("item");
                };
                assert_eq!(d.flags4, arg == 0xFFFF);
            }
        }
    }
}

#[test]
fn item_flags_0x40_one_layout() {
    // No D2RUST producer (is not built): proto encode → client.
    for guid in u32s() {
        for &mask in &[4u32, 0x10, 0x4004, u32::MAX] {
            for &value in &[0u32, 1, u32::MAX] {
                let b = gen::ItemFlags { guid, mask, value }.encode();
                assert_eq!(gen::ItemFlags::decode(&b).unwrap().mask, mask);
                let mut m = Model::default();
                let k = UnitKey::new(ITEM, guid);
                m.put(k).kind = KindData::Item(ItemData::default());
                recv_ok(&mut m, &b);
                let mut want = ItemData::default();
                want.set_flags(mask, value != 0);
                assert_eq!(m.unit(k).kind, KindData::Item(want));
            }
        }
    }
}

#[test]
fn clear_cursor_0x42_one_layout() {
    for &ty in &U8S {
        for unit in u32s() {
            let sim = d2_sim::items::moves::layouts::clear_cursor(ty, unit);
            let p = gen::ClearCursor { type_: ty, unit };
            assert_eq!(sim, p.encode());
            assert_eq!(s2c::parse(&sim), Ok(S2c::ClearCursor(p)));
        }
    }
    // Client: the local player's cursor item is freed.
    for guid in u32s() {
        let mut m = Model::default();
        let p = UnitKey::new(PLAYER, guid);
        m.put(p).kind = KindData::Player(PlayerData {
            cursor_item: Some(9),
            ..PlayerData::default()
        });
        m.w.local_player = Some(p);
        m.put(UnitKey::new(ITEM, 9));
        recv_ok(
            &mut m,
            &d2_sim::items::moves::layouts::clear_cursor(PLAYER, guid),
        );
        assert!(!m.w.units.contains_key(&UnitKey::new(ITEM, 9)));
    }
}

#[test]
fn relators_0x47_0x48_one_layout() {
    use d2_sim::items::moves::layouts::{relator1, relator2};
    for &ty in &U8S {
        for &arg in &U8S {
            for unit in u32s() {
                let r1 = relator1(ty, unit);
                let p1 = gen::Relator1 {
                    type_: ty,
                    f2: 0,
                    unit,
                };
                assert_eq!(r1, p1.encode());
                assert_eq!(s2c::parse(&r1), Ok(S2c::Relator1(p1)));
                let r2 = relator2(ty, arg, unit);
                let p2 = gen::Relator2 {
                    type_: ty,
                    arg,
                    unit,
                };
                assert_eq!(r2, p2.encode());
                assert_eq!(s2c::parse(&r2), Ok(S2c::Relator2(p2)));
            }
        }
    }
    let mut m = Model::default();
    recv_ok(&mut m, &relator1(0, 1));
    recv_ok(&mut m, &relator2(4, 0xFF, u32::MAX));
}

// ------------------------------------------------------------ skills

#[test]
fn unit_skill_on_unit_0x4c_one_layout() {
    use d2_sim::wiring::action::unit_update::skill_message::skill_on_unit;
    for (i, guid) in u32s().into_iter().enumerate() {
        for &skill in &U16S {
            for &b in &U8S {
                let (ty, tt) = (i as u8 % 5, U8S[i % 5]);
                let target = guid.rotate_left(7);
                let w = U16S[i % 5];
                let sim = skill_on_unit(ty, guid, skill, b, tt, target, w);
                let p = gen::UnitSkillOnUnit {
                    type_: ty,
                    guid,
                    skill,
                    f8: b,
                    target_type: tt,
                    target,
                    f14: w,
                };
                assert_eq!(sim, p.encode());
                assert_eq!(s2c::parse(&sim), Ok(S2c::UnitSkillOnUnit(p)));
            }
        }
    }
    // Client: queued on the addressed unit (the handler reads
    // skill u16@6, b @8, target type @9, target u32@0xA).
    let mut m = Model::default();
    let k = UnitKey::new(MONSTER, 0x8000_0001);
    m.put(k);
    let b = skill_on_unit(MONSTER, k.guid, 0x1234, 7, PLAYER, 3, 0);
    recv_ok(&mut m, &b);
    assert_eq!(m.unit(k).queue, [b.to_vec()]);
}

#[test]
fn unit_skill_on_point_0x4d_one_layout() {
    use d2_sim::wiring::action::unit_update::skill_message::skill_on_point;
    for (i, guid) in u32s().into_iter().enumerate() {
        for skill in u32s() {
            let (ty, b) = (i as u8 % 5, U8S[i % 5]);
            let (x, y, w) = (U16S[i % 5], U16S[(i + 2) % 5], U16S[(i + 4) % 5]);
            let sim = skill_on_point(ty, guid, skill, b, x, y, w);
            let p = gen::UnitSkillOnPoint {
                type_: ty,
                guid,
                skill,
                f10: b,
                x,
                y,
                f15: w,
            };
            assert_eq!(sim, p.encode());
            assert_eq!(s2c::parse(&sim), Ok(S2c::UnitSkillOnPoint(p)));
        }
    }
    // The shrine form: type 2, operator GUID in the
    // u32@6 (TSV `skill`), code @10, zeros.
    for object in u32s() {
        for &code in &U8S {
            let operator = object ^ 0x5A5A_5A5A;
            let sim = d2_sim::world::objects::shrine_message(object, operator, code);
            let p = gen::UnitSkillOnPoint {
                type_: OBJECT,
                guid: object,
                skill: operator,
                f10: code,
                ..Default::default()
            };
            assert_eq!(sim, p.encode());
        }
    }
    let mut m = Model::default();
    let k = UnitKey::new(PLAYER, 5);
    m.put(k);
    let b = skill_on_point(PLAYER, 5, u32::MAX, 1, 0x1234, 0x5678, 0);
    recv_ok(&mut m, &b);
    assert_eq!(m.unit(k).queue, [b.to_vec()]);
}

// ------------------------------------------------------------ NPC / quests

#[test]
fn merc_for_hire_0x4e_one_layout() {
    // The D2RUST builder is inline (`world/npc/hire.rs` send_hire_list):
    // name u16@1, seed u32@3.
    for &name in &U16S {
        for seed in u32s() {
            let mut sim = [0u8; 7];
            sim[0] = 0x4E;
            sim[1..3].copy_from_slice(&name.to_le_bytes());
            sim[3..7].copy_from_slice(&seed.to_le_bytes());
            assert_eq!(sim, gen::MercForHire { name, seed }.encode());
            assert_eq!(sim, built::MercForHire { name, seed }.encode());
            let mut m = Model::default();
            recv_ok(&mut m, &sim);
            assert_eq!(m.out, [Output::HireOffer { name, seed }]);
        }
    }
    let mut m = Model::default();
    recv_ok(&mut m, &gen::StartMercList.encode());
    assert_eq!(m.out, [Output::HireListReset]);
}

#[test]
fn quest_special_0x50_one_layout() {
    for code in 1u16..=4 {
        for (i, v) in u32s().into_iter().enumerate() {
            let w = [v as u16, (v >> 16) as u16, U16S[i % 5], !(v as u16), 0];
            let p = gen::QuestSpecial {
                code,
                v0: w[0],
                v1: w[1],
                v2: w[2],
                v3: w[3],
                v4: w[4],
            };
            let b = p.encode();
            assert_eq!(gen::QuestSpecial::decode(&b), Ok(p));
            let mut m = Model::default();
            recv_ok(&mut m, &b);
            // The client reads a sixth word @13 that no layout names
            // (bytes 13–14 are never written by the base game; 0 in D2RUST).
            assert_eq!(
                m.out,
                [Output::QuestSpecial {
                    code,
                    words: [w[0], w[1], w[2], w[3], w[4], 0],
                }]
            );
        }
    }
    // The quest form of the built type equals the generated one.
    let q = built::QuestSpecial {
        den_left: 7,
        staff_tomb: -1,
        barbarians_left: 0x8000,
    };
    let g = gen::QuestSpecial {
        code: 1,
        v0: 7,
        v1: 0xFFFF,
        v2: 0x8000,
        v3: 0,
        v4: 0,
    };
    assert_eq!(q.encode(), g.encode());
}

/// Every 0x50 form d2-sim sends (mercenary 2 `world/npc/hire.rs`, quest
/// codes 4 `act1/q4.rs`, 13 `quests.rs`, 23 `act4/q2.rs`) parses: code 1
/// with bytes 9–14 zero as the code-1 view, the rest as the TSV layout.
// Checks: tables/sim/server-messages.tsv
#[test]
fn quest_special_0x50_parse_every_quest_form() {
    for code in [2u16, 4, 13, 23, 1] {
        for (i, v) in sweep(6).enumerate() {
            let g = gen::QuestSpecial {
                code,
                v0: v as u16,
                v1: (v >> 16) as u16,
                v2: i as u16,
                v3: if code == 1 { 0 } else { (v >> 8) as u16 },
                v4: if code == 1 { 0 } else { 5 },
            };
            let b = g.encode();
            let got = s2c::parse(&b);
            if code == 1 {
                let Ok(S2c::QuestSpecial(m)) = got else {
                    panic!("code 1: {got:?}");
                };
                assert_eq!(m.encode().to_vec(), b);
            } else {
                assert_eq!(got, Ok(S2c::QuestSpecialForm(g)), "code {code}");
            }
        }
    }
}

#[test]
fn quest_log_info_0x52_one_layout() {
    for seed in sweep(8) {
        let mut list = [0u8; 41];
        for (k, b) in list.iter_mut().enumerate() {
            *b = (seed >> (k % 25)) as u8;
        }
        // D2RUST `world/quests.rs` send_quest_log: `vec![0x52]` + list.
        let mut sim = vec![0x52];
        sim.extend_from_slice(&list);
        assert_eq!(sim, gen::QuestLogInfo { status: list }.encode());
        assert_eq!(sim, built::QuestLogInfo { list }.encode());
        let mut m = Model::default();
        recv_ok(&mut m, &sim);
        assert_eq!(m.out, [Output::QuestLog { status: list }]);
    }
}

#[test]
fn darkness_0x53_one_layout() {
    use d2_sim::world::environment::Environment;
    for period in u32s() {
        for ticks in U32S {
            for eclipse in [false, true] {
                let e = Environment {
                    period,
                    ticks,
                    eclipse,
                    ..Environment::CREATED
                };
                let p = gen::Darkness {
                    period,
                    ticks,
                    eclipse: u8::from(eclipse),
                };
                assert_eq!(e.message(), p.encode());
                assert_eq!(s2c::parse(&e.message()), Ok(S2c::Darkness(p)));
            }
        }
    }
}

#[test]
fn npc_enchants_0x57_one_layout() {
    // The d2-sim builder (`units::messages::npc_enchants`, sent by the
    // monster update's step 10) == the d2_proto encode for every form it
    // writes (umod2 one byte, flag 0 / 1).
    for guid in u32s() {
        for &name in &U16S {
            for &u in &U8S {
                for flag in [false, true] {
                    let sim =
                        d2_sim::units::messages::npc_enchants(guid, name, [0x7F, 0x80, u], flag);
                    let p = gen::NpcEnchants {
                        guid,
                        type_: 1,
                        name,
                        umod0: 0x7F,
                        umod1: 0x80,
                        umod2: u16::from(u),
                        flag: u16::from(flag),
                    };
                    assert_eq!(sim.to_vec(), p.encode());
                }
            }
        }
    }
    for guid in u32s() {
        for &name in &U16S {
            for &umod2 in &U16S {
                for &flag in &[0u16, 1, 0x8000] {
                    let p = gen::NpcEnchants {
                        guid,
                        type_: 1,
                        name,
                        umod0: 0x7F,
                        umod1: 0x80,
                        umod2,
                        flag,
                    };
                    let b = p.encode();
                    assert_eq!(s2c::parse(&b), Ok(S2c::NpcEnchants(p)));
                    let mut m = Model::default();
                    let k = UnitKey::new(MONSTER, guid);
                    m.put(k).kind = KindData::Monster(Box::default());
                    recv_ok(&mut m, &b);
                    let KindData::Monster(d) = &m.unit(k).kind else {
                        panic!("monster");
                    };
                    // u8@0xA = the low byte of u16@0xA.
                    assert_eq!(
                        (d.name_seed, d.umods[..3].to_vec(), d.flags),
                        (
                            name,
                            vec![0x7F, 0x80, umod2 as u8],
                            8 | if flag != 0 { 4 } else { 0 }
                        )
                    );
                }
            }
        }
    }
}

#[test]
fn open_ui_0x58_one_layout() {
    for guid in u32s() {
        for result in [0u8, 1, 4, 5, 6, 7] {
            for &effect in &U8S {
                // `world/npc.rs` service_result (effect 0) and the object
                // insert literal `[0x58, guid, result, effect]`.
                let mut sim = d2_sim::world::npc::service_result(guid, result);
                sim[6] = effect;
                let g = gen::OpenUi {
                    guid,
                    code: result,
                    effect,
                };
                assert_eq!(sim, g.encode());
                let bt = built::OpenUi {
                    npc_guid: guid,
                    result,
                    effect,
                };
                assert_eq!(sim, bt.encode());
                assert_eq!(s2c::parse(&sim), Ok(S2c::OpenUi(bt)));
                let mut m = Model::default();
                recv_ok(&mut m, &sim);
                assert_eq!(
                    m.out,
                    [Output::OpenUi {
                        guid,
                        code: result,
                        arg: effect,
                    }]
                );
            }
        }
    }
}

#[test]
fn quest_item_state_0x5d_one_layout() {
    for &extra in &U16S {
        for &status in &U8S {
            // D2RUST `world/quests.rs` send_status: chain, flags, status,
            // extra u16@4.
            let sim = [0x5D, 3, 1, status, extra as u8, (extra >> 8) as u8];
            let g = gen::QuestItemState {
                chain: 3,
                flags: 1,
                status,
                extra,
            };
            assert_eq!(sim, g.encode());
            let bt = built::QuestItemState {
                chain: 3,
                flags: 1,
                status,
                extra,
            };
            assert_eq!(sim, bt.encode());
            let mut m = Model::default();
            recv_ok(&mut m, &sim);
            // Chain 3, flags bit 0: an output row; extra read as i16.
            assert_eq!(
                m.out,
                [Output::QuestUi {
                    chain: 3,
                    flags: 1,
                    status,
                    extra: extra as i16,
                }]
            );
        }
    }
}

#[test]
fn game_quest_availability_0x5e_one_layout() {
    for seed in sweep(8) {
        let mut available = [0u8; 37];
        for (k, b) in available.iter_mut().enumerate() {
            *b = (seed >> (k % 31)) as u8 & 1;
        }
        let mut sim = vec![0x5E];
        sim.extend_from_slice(&available);
        assert_eq!(sim, gen::GameQuestAvailability { available }.encode());
        let mut m = Model::default();
        recv_ok(&mut m, &sim);
        assert_eq!(m.out, [Output::QuestAvailability { bytes: available }]);
    }
}

#[test]
fn can_go_to_act_0x61_one_layout() {
    for &video in &U8S {
        let sim = [0x61, video];
        assert_eq!(sim, gen::CanGoToAct { video }.encode());
        let mut m = Model::default();
        recv_ok(&mut m, &sim);
        assert_eq!(m.out, [Output::ActVideo { video }]);
    }
}

#[test]
fn make_unit_targetable_0x62_one_layout() {
    use d2_sim::world::quests::helpers::msg_end_interaction;
    for &kind in &U8S {
        for guid in u32s() {
            let sim = msg_end_interaction(kind, guid);
            let p = gen::MakeUnitTargetable { type_: kind, guid };
            assert_eq!(sim, p.encode());
            assert_eq!(s2c::parse(&sim), Ok(S2c::MakeUnitTargetable(p)));
        }
    }
    for guid in u32s() {
        let mut m = Model::default();
        let k = UnitKey::new(MONSTER, guid);
        m.put(k).quest_untargetable = true;
        recv_ok(&mut m, &msg_end_interaction(1, guid));
        assert_eq!(
            (m.unit(k).flag_2, m.unit(k).quest_untargetable),
            (Some(true), false)
        );
        recv_ok(&mut m, &msg_end_interaction(6, 0));
        recv_ok(&mut m, &msg_end_interaction(3, guid));
        assert_eq!(
            m.out,
            [
                Output::NpcDialogEnd { kind: 1 },
                Output::NpcDialogEnd { kind: 6 }
            ]
        );
    }
}

#[test]
fn waypoint_menu_0x63_one_layout() {
    for guid in u32s() {
        let mut record = [0u8; 16];
        for (k, b) in record.iter_mut().enumerate() {
            *b = guid.rotate_left(k as u32) as u8;
        }
        let sim = d2_sim::world::waypoints::menu_message(guid, &record);
        let bt = built::WaypointMenu {
            object_guid: guid,
            record,
        };
        assert_eq!(sim, bt.encode());
        let w =
            |o: usize| u32::from_le_bytes([record[o], record[o + 1], record[o + 2], record[o + 3]]);
        let g = gen::WaypointMenu {
            guid,
            magic: u16::from_le_bytes([record[0], record[1]]),
            bits0: w(2),
            bits1: w(6),
            bits2: w(10),
            bits3: u16::from_le_bytes([record[14], record[15]]),
        };
        assert_eq!(sim, g.encode());
        assert_eq!(s2c::parse(&sim), Ok(S2c::WaypointMenu(bt)));
        let mut m = Model::default();
        recv_ok(&mut m, &sim);
        assert_eq!(m.out, [Output::WaypointMenu { guid, record }]);
    }
}

// ------------------------------------------------------------ units

#[test]
fn assign_object_0x51_one_layout() {
    use d2_sim::units::messages::assign_object;
    for (i, guid) in u32s().into_iter().enumerate() {
        for &x in &U16S {
            let (class, y, mode, interact) =
                (U16S[i % 5], U16S[(i + 1) % 5], U8S[i % 5], U8S[(i + 3) % 5]);
            let sim = assign_object(guid, class, x, y, mode, interact);
            let p = gen::AssignObject {
                type_: OBJECT,
                guid,
                class,
                x,
                y,
                mode,
                interact,
            };
            assert_eq!(sim, p.encode());
            assert_eq!(s2c::parse(&sim), Ok(S2c::AssignObject(p)));
            let mut m = Model::default();
            recv_ok(&mut m, &sim);
            let u = m.unit(UnitKey::new(OBJECT, guid));
            assert_eq!(
                (u.class, u.position, u.mode),
                (
                    u32::from(class),
                    ((x, y) != (0, 0)).then_some((x, y)),
                    u32::from(mode)
                )
            );
            let KindData::Object(d) = &u.kind else {
                panic!("object");
            };
            assert_eq!(d.interact, interact);
        }
    }
}

#[test]
fn assign_player_0x59_one_layout() {
    use d2_sim::wiring::action::switch::assign_player;
    for (i, guid) in u32s().into_iter().enumerate() {
        let class = (i % 7) as u8;
        let (x, y) = (U16S[i % 5], U16S[(i + 1) % 5]);
        let mut name = [0u8; 16];
        name[..(i % 16)].fill(b'a' + class);
        let sim = assign_player(guid, class, &name, x, y);
        let bt = built::AssignPlayer {
            guid,
            class,
            name,
            x,
            y,
        };
        assert_eq!(sim, bt.encode());
        let g = gen::AssignPlayer {
            guid,
            class,
            name,
            x,
            y,
        };
        assert_eq!(sim, g.encode());
        assert_eq!(s2c::parse(&sim), Ok(S2c::AssignPlayer(bt)));
        let mut m = Model::default();
        recv_ok(&mut m, &sim);
        let u = m.unit(UnitKey::new(PLAYER, guid));
        assert_eq!(
            (u.class, u.position),
            (u32::from(class), ((x, y) != (0, 0)).then_some((x, y)))
        );
        let KindData::Player(d) = &u.kind else {
            panic!("player");
        };
        assert_eq!(d.name, name);
    }
}

#[test]
fn event_message_0x5a_one_layout() {
    // "Can't do that" (`items/moves/layouts.rs`): code 0x0E, u8@2 1.
    let sim = d2_sim::items::moves::layouts::cant_do_that();
    let g = gen::EventMessage {
        code: 0x0E,
        f2: 1,
        ..Default::default()
    };
    assert_eq!(sim, g.encode());
    assert_eq!(s2c::parse(&sim), Ok(S2c::EventMessage(g)));
    // The leave / join form (`d2-server` session_flow): u8@2 4, name @8.
    for code in [2u8, 3] {
        for n in 0..16 {
            let mut name = [0u8; 16];
            name[..n].fill(b'x');
            let srv = d2_server::adapters::session_flow::player_event(code, &name);
            let g = gen::EventMessage {
                code,
                f2: 4,
                f3: 0,
                name,
                account: [0; 16],
            };
            assert_eq!(srv, g.encode());
            let mut m = Model::default();
            recv_ok(&mut m, &srv);
            assert_eq!(
                m.out,
                [Output::EventText {
                    bytes: srv,
                    local_name: None,
                }]
            );
        }
    }
    // A hosted game's account name @0x18 (byte
    // 0x27 := 0): the client copies all 40 bytes.
    let mut account = [0u8; 16];
    account[..15].fill(b'a');
    let g = gen::EventMessage {
        code: 2,
        f2: 4,
        f3: 0,
        name: *b"name\0\0\0\0\0\0\0\0\0\0\0\0",
        account,
    };
    let b = g.encode();
    assert_eq!((&b[24..39], b[39]), (&[b'a'; 15][..], 0));
    assert_eq!(s2c::parse(&b), Ok(S2c::EventMessage(g)));
    let mut m = Model::default();
    recv_ok(&mut m, &b);
    assert_eq!(
        m.out,
        [Output::EventText {
            bytes: b,
            local_name: None,
        }]
    );
}

/// 0x5B from the TSV layout, by hand.
fn player_joined(
    guid: u32,
    class: u8,
    name: &[u8; 16],
    w: [u16; 5],
    s1: &[u8],
    s2: &[u8],
) -> Vec<u8> {
    let mut b = vec![0x5B, 0, 0];
    b.extend_from_slice(&guid.to_le_bytes());
    b.push(class);
    b.extend_from_slice(name);
    for v in w {
        b.extend_from_slice(&v.to_le_bytes());
    }
    assert_eq!(b.len(), 34);
    b.extend_from_slice(s1);
    b.push(0);
    b.extend_from_slice(s2);
    b.push(0);
    let n = b.len() as u16;
    b[1..3].copy_from_slice(&n.to_le_bytes());
    b
}

#[test]
fn roster_0x5b_0x5c_0x65_one_layout() {
    for (i, guid) in u32s().into_iter().filter(|&g| g != u32::MAX).enumerate() {
        let mut name = [0u8; 16];
        name[..1 + i % 15].fill(b'n');
        let w = [
            U16S[i % 5],
            U16S[(i + 1) % 5],
            0x1C1C,
            U16S[(i + 2) % 5],
            U16S[(i + 3) % 5],
        ];
        let b = player_joined(guid, 4, &name, w, b"ab", b"");
        assert_eq!(
            d2_proto::transport::server_size(&b),
            d2_proto::schema::Size::Bytes(b.len())
        );
        let mut m = Model::default();
        recv_ok(&mut m, &b);
        let r = &m.w.roster[0];
        assert_eq!(
            (r.guid, r.class, r.name, r.f20, r.f22, r.f30, r.f44),
            (guid, 4, name, w[0], w[1], w[3], w[4])
        );
        // 0x65: count u16@5, sign-extended.
        for &count in &U16S {
            let k = gen::PlayerKillCount { guid, count }.encode();
            assert_eq!(
                s2c::parse(&k),
                Ok(S2c::PlayerKillCount(gen::PlayerKillCount { guid, count }))
            );
            recv_ok(&mut m, &k);
            assert_eq!(m.w.roster[0].kills, i32::from(count as i16));
        }
        // 0x5C: GUID u32@1.
        let l = gen::PlayerLeft { guid }.encode();
        recv_ok(&mut m, &l);
        assert!(m.w.roster.is_empty());
    }
}

/// The d2-sim builders the join sequence and the leave send
/// (`units::messages::{player_joined, player_kill_count, player_left,
/// player_event}`) == the TSV layout / d2_proto encode == the client:
/// the roster record (+0x20 level, +0x22 party,
/// +0x30 / +0x44 0, strings empty), the kill count, the removal, and the
/// join 0x5A's event text.
#[test]
fn roster_join_and_leave_sim_builders_to_client() {
    use d2_sim::units::messages as sim;
    for (i, guid) in u32s().into_iter().filter(|&g| g != u32::MAX).enumerate() {
        let mut name = [0u8; 16];
        name[..1 + i % 15].fill(b'n');
        let class = (i % 7) as u8;
        let level = U16S[i % 5];
        let party = [0xFFFF, U16S[(i + 1) % 5]][i % 2];
        let b = sim::player_joined(guid, class, &name, level, party);
        assert_eq!(
            b,
            player_joined(guid, class, &name, [level, party, 0, 0, 0], b"", b"")
        );
        assert_eq!(
            d2_proto::transport::server_size(&b),
            d2_proto::schema::Size::Bytes(b.len())
        );
        let mut m = Model::default();
        recv_ok(&mut m, &b);
        let r = &m.w.roster[0];
        assert_eq!(
            (r.guid, r.class, r.name, r.f20, r.f22, r.f30, r.f44),
            (guid, u32::from(class), name, level, party, 0, 0)
        );
        for &count in &U16S {
            let k = sim::player_kill_count(guid, count);
            assert_eq!(k.to_vec(), gen::PlayerKillCount { guid, count }.encode());
            recv_ok(&mut m, &k);
            assert_eq!(m.w.roster[0].kills, i32::from(count as i16));
        }
        let ev = sim::player_event(2, &name);
        let g = gen::EventMessage {
            code: 2,
            f2: 4,
            f3: 0,
            name,
            account: [0; 16],
        };
        assert_eq!(ev.to_vec(), g.encode());
        recv_ok(&mut m, &ev);
        let l = sim::player_left(guid);
        assert_eq!(l.to_vec(), gen::PlayerLeft { guid }.encode());
        recv_ok(&mut m, &l);
        assert!(m.w.roster.is_empty());
    }
}

#[test]
fn portal_flags_0x5f_one_layout() {
    for v in u32s() {
        let sim = d2_sim::units::messages::portal_flags(v);
        assert_eq!(sim, gen::PortalFlags { f1: v }.encode());
        let mut m = Model::default();
        local(&mut m);
        recv_ok(&mut m, &sim);
        let KindData::Player(p) = &m.unit(UnitKey::new(PLAYER, 1)).kind else {
            panic!("player");
        };
        assert_eq!(p.f2c, v);
    }
}

#[test]
fn town_portal_state_0x60_one_layout() {
    use d2_sim::world::objects::{portal_message, ObjectData as SimObject};
    for guid in u32s() {
        for &flags in &U8S {
            for &level in &U8S {
                let d = SimObject {
                    guid,
                    portal_flags: flags,
                    interact: level,
                    ..SimObject::default()
                };
                let sim = portal_message(&d);
                let p = gen::TownPortalState { flags, level, guid };
                assert_eq!(sim, p.encode());
                assert_eq!(s2c::parse(&sim), Ok(S2c::TownPortalState(p)));
                let mut m = Model::default();
                local(&mut m);
                let k = UnitKey::new(OBJECT, guid);
                let u = m.put(k);
                u.class = 59;
                u.kind = KindData::Object(ObjectData::default());
                recv_ok(&mut m, &sim);
                let KindData::Object(o) = &m.unit(k).kind else {
                    panic!("object");
                };
                assert_eq!((o.interact, o.portal_flags), (level, flags & 3));
            }
        }
    }
}

#[test]
fn built_types_match_their_ids_0x4e_0x63() {
    // The hand-written types of this range share the TSV size.
    assert_eq!(built::MercForHire::SIZE, gen::MercForHire::SIZE);
    assert_eq!(built::QuestSpecial::SIZE, gen::QuestSpecial::SIZE);
    assert_eq!(built::QuestLogInfo::SIZE, gen::QuestLogInfo::SIZE);
    assert_eq!(built::OpenUi::SIZE, gen::OpenUi::SIZE);
    assert_eq!(built::AssignPlayer::SIZE, gen::AssignPlayer::SIZE);
    assert_eq!(built::QuestItemState::SIZE, gen::QuestItemState::SIZE);
    assert_eq!(built::WaypointMenu::SIZE, gen::WaypointMenu::SIZE);
}
