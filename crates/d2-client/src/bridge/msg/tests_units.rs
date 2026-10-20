//! Test vectors of the unit messages: "A" and "B" are two recorded
//! packet traces, seq numbers in the comments; synthetic where marked.

use super::super::world::{
    ClientUnit, KindData, MonsterClass, ObjectData, StatSend, UnitKey, MISSILE, MONSTER, OBJECT,
    PLAYER,
};
use super::support::{hex, Model};
use super::units::queued;

const P1: UnitKey = UnitKey::new(PLAYER, 1);

fn with_monsters() -> Model {
    let mut m = Model::default();
    m.inputs.tables.monsters = vec![Some(MonsterClass::default()); 155];
    m
}

/// The local player (0, 1) at `pos`, mode `mode`.
fn local(m: &mut Model, pos: (u16, u16), mode: u32) {
    let u = m.put(P1);
    u.position = Some(pos);
    u.mode = mode;
    m.w.local_player = Some(P1);
}

fn request(m: &Model, k: UnitKey) -> Option<(u8, [i32; 7])> {
    m.unit(k).last_mode_request.map(|r| (r.code, r.record))
}

#[test]
fn assign_player_b102() {
    let mut m = Model::default();
    let mut b = hex("59 01 00 00 00 01 77 65 72 77 65 72 00");
    b.resize(26, 0);
    m.recv(&b);
    let u = m.unit(P1);
    assert_eq!((u.class, u.position, u.mode), (1, None, 5));
    assert_eq!(
        u.stats.iter().map(|(&k, &v)| (k, v)).collect::<Vec<_>>(),
        [(67, 100), (68, 100), (69, 100)]
    );
    assert_eq!(u.seed, Some((0x6AC6_935F, 0)));
    let KindData::Player(p) = &u.kind else {
        panic!("player data");
    };
    assert_eq!(&p.name[..7], b"werwer\0");
}

#[test]
fn assign_monster_b157() {
    let mut m = with_monsters();
    m.hex("ac 06 00 00 00 9a 00 1a 12 b9 11 80 0e 01");
    let u = m.unit(UnitKey::new(MONSTER, 6));
    assert_eq!(
        (u.class, u.position, u.mode),
        (154, Some((0x121A, 0x11B9)), 1)
    );
    assert_eq!(
        u.stats.iter().map(|(&k, &v)| (k, v)).collect::<Vec<_>>(),
        [(6, 0x8000), (7, 0x8000), (328, 0x23D3)]
    );
    let KindData::Monster(d) = &u.kind else {
        panic!("monster data");
    };
    assert_eq!(
        (d.flags, d.components, d.value, d.v31, &d.stat_list),
        (0, [0; 16], -1, None, &None)
    );
    // No `monstats` / `monstats2` row: nothing is created.
    let mut m = Model::default();
    m.hex("ac 06 00 00 00 9a 00 1a 12 b9 11 80 0e 01");
    assert!(m.w.units.is_empty());
    assert!(m.log.rejected.is_empty());
}

/// Packs `(value, bits)` fields low bits first.
fn pack(fields: &[(u32, u32)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    for &(v, n) in fields {
        for i in 0..n {
            if pos / 8 == out.len() {
                out.push(0);
            }
            out[pos / 8] |= (((v >> i) & 1) as u8) << (pos % 8);
            pos += 1;
        }
    }
    out
}

#[test]
fn assign_monster_bit_stream_synthetic() {
    let mut m = Model::default();
    let mut class = MonsterClass::default();
    class.components[0] = 2; // 1 bit
    class.components[1] = 5; // bit length of 4 = 3 bits
    class.components[2] = 1; // 1 bit
    m.inputs.tables.monsters = vec![Some(class)];
    m.inputs.tables.stats = vec![StatSend::default(); 3];
    // Stat 2: 6 bits, signed, 2 param bits.
    m.inputs.tables.stats[2] = StatSend {
        bits: 6,
        param_bits: 2,
        signed: true,
        update_anim_rate: false,
    };
    let mut comps = vec![(1, 1), (4, 3), (1, 1)];
    comps.extend(std::iter::repeat_n((0, 1), 13));
    let mut fields = vec![(3, 4), (1, 1)];
    fields.extend(comps);
    fields.extend([
        (1, 1),       // has type flags
        (0, 1),       // champion
        (1, 1),       // unique → 8
        (1, 1),       // superunique → 2
        (0, 1),       // minion
        (1, 1),       // ghostly → 0x40
        (0xFFFE, 16), // hcIdx
        (0x21, 8),
        (0x07, 8),
        (0, 8),       // umods end
        (0x1234, 16), // name seed
        (1, 1),       // has value
        (0xDEAD_BEEF, 32),
        (1, 1), // 31-bit value follows
        (0x5555, 31),
        (1, 1), // stat list follows
        (2, 9),
        (3, 2),     // param
        (0x3E, 6),  // value −2
        (0x1FF, 9), // end
    ]);
    let stream = pack(&fields);
    let mut b = hex("ac 09 00 00 00 00 00 0a 00 0b 00 10");
    b.push((0xD + stream.len()) as u8);
    b.extend(stream);
    m.recv(&b);
    assert!(m.log.rejected.is_empty(), "{:?}", m.rejected());
    let u = m.unit(UnitKey::new(MONSTER, 9));
    assert_eq!(u.mode, 3);
    assert_eq!(u.stat(6), 0x1000);
    assert_eq!(u.stat(328), 21);
    let KindData::Monster(d) = &u.kind else {
        panic!("monster data");
    };
    assert_eq!(&d.components[..4], &[1, 4, 1, 0]);
    assert_eq!(d.flags, 8 | 2 | 0x40);
    assert_eq!(d.hc_idx, 0xFFFE);
    assert_eq!(d.umods, [0x21, 7, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(d.name_seed, 0x34, "the low byte of the 16-bit field");
    assert_eq!(d.value, 0xDEAD_BEEF_u32 as i32);
    assert_eq!(d.v31, Some(0x5555));
    assert_eq!(
        d.stat_list.clone().unwrap().into_iter().collect::<Vec<_>>(),
        [((2, 3), -2)]
    );
}

#[test]
fn assign_object_b160() {
    let mut m = Model::default();
    m.hex("51 02 0d 00 00 00 25 00 14 12 c0 11 02 00");
    let u = m.unit(UnitKey::new(OBJECT, 13));
    assert_eq!(
        (u.class, u.position, u.mode, &u.kind),
        (
            37,
            Some((0x1214, 0x11C0)),
            2,
            &KindData::Object(ObjectData {
                interact: 0,
                shadow_mode: Some(2),
                ..ObjectData::default()
            })
        )
    );
    // Type 1: fatal 0x202.
    m.hex("51 01 0d 00 00 00 25 00 14 12 c0 11 02 00");
    assert_eq!(m.rejected(), [(0x51, "fatal assert 0x202".to_owned())]);
}

#[test]
fn assign_object_stamps_the_footprint_of_a_colliding_mode() {
    use crate::bridge::objects::ObjClientRow;
    let mut m = Model::default();
    let mut torch = ObjClientRow {
        has_collision: [1, 1, 1, 0, 0, 0, 0, 0],
        ..ObjClientRow::default()
    };
    torch.shape.has_collision = [true, true, true, false, false, false, false, false];
    torch.frame_cnt = [0x100; 8];
    m.inputs.objclient.rows = vec![ObjClientRow::default(); 38];
    m.inputs.objclient.rows[37] = torch;
    let footprint = |m: &Model, guid| match &m.w.units[&UnitKey::new(OBJECT, guid)].kind {
        KindData::Object(d) => d.footprint,
        _ => panic!("an object"),
    };
    // Class 37 in mode 2 (`HasCollision2` = 1): stamped.
    m.hex("51 02 0d 00 00 00 25 00 14 12 c0 11 02 00");
    assert!(footprint(&m, 13));
    // Mode 3 (`HasCollision3` = 0): not stamped.
    m.hex("51 02 0e 00 00 00 25 00 16 12 c0 11 03 00");
    assert!(!footprint(&m, 14));
}

#[test]
fn an_object_mode_update_frees_and_restamps_the_footprint() {
    use crate::bridge::objects::ObjClientRow;
    // A door row: blocked closed (mode 0), open in mode 2.
    let mut m = Model::default();
    let mut door = ObjClientRow {
        is_door: 1,
        ..ObjClientRow::default()
    };
    door.shape.has_collision = [true, true, false, false, false, false, false, false];
    door.frame_cnt = [0x100; 8];
    m.inputs.objclient.rows = vec![ObjClientRow::default(); 1];
    m.inputs.objclient.rows[0] = door;
    let key = UnitKey::new(OBJECT, 13);
    let footprint = |m: &Model| match &m.w.units[&key].kind {
        KindData::Object(d) => d.footprint,
        _ => panic!("an object"),
    };
    // Created closed: the 0x51 stamps the footprint.
    m.hex("51 02 0d 00 00 00 00 00 14 12 c0 11 00 00");
    assert!(footprint(&m));
    // A server mode update to mode 2 frees it (the door is open to this
    // client too, not only to the player who operated it). 0x0E is an
    // update-pass message: drain it.
    m.hex("0e 02 0d 00 00 00 03 01 02 00 00 00");
    assert_eq!(m.drain(), 1);
    assert!(!footprint(&m));
    // Closed again: stamped.
    m.hex("0e 02 0d 00 00 00 03 01 00 00 00 00");
    assert_eq!(m.drain(), 1);
    assert!(footprint(&m));
}

#[test]
fn remove_unit_a7228_a215606() {
    let mut m = Model::default();
    m.put(UnitKey::new(OBJECT, 12));
    m.put(UnitKey::new(MONSTER, 0x19));
    m.hex("0a 02 0c 00 00 00").hex("0a 01 19 00 00 00");
    assert!(m.w.units.is_empty());
    // A key not in the set: nothing.
    m.hex("0a 01 19 00 00 00");
    assert!(m.log.rejected.is_empty());
}

#[test]
fn reassign_player_b154() {
    let mut m = Model::default();
    m.put(P1).mode = 5;
    m.hex("15 00 01 00 00 00 41 12 c4 11 01");
    assert_eq!(m.unit(P1).position, Some((0x1241, 0x11C4)));
    // Not in the set: nothing.
    m.hex("15 00 02 00 00 00 41 12 c4 11 01");
    assert_eq!(m.w.units.len(), 1);
    // A dead monster stays.
    let k = UnitKey::new(MONSTER, 4);
    let u = m.put(k);
    u.mode = 0xC;
    u.position = Some((5, 5));
    m.hex("15 01 04 00 00 00 41 12 c4 11 00");
    assert_eq!(m.unit(k).position, Some((5, 5)));
    assert!(m.log.rejected.is_empty());
}

/// Receives `msg` for a unit already in the model, drains, returns the
/// request.
fn drained(m: &mut Model, msg: &str) -> Option<(u8, [i32; 7])> {
    let b = hex(msg);
    m.recv(&b);
    assert_eq!(m.drain(), 1, "{msg}");
    let k = super::super::world::addressed_unit(&b).unwrap();
    request(m, k)
}

#[test]
fn queued_messages_recorded() {
    let mut m = Model::default();
    let k13 = UnitKey::new(MONSTER, 0x13);
    let u = m.put(k13);
    u.mode = 1;
    u.position = Some((0x1294, 0x1555));
    // A 213893.
    assert_eq!(
        drained(&mut m, "67 13 00 00 00 01 95 12 55 15 01 00 0d 4b 00 05"),
        Some((1, [0x1295, 0x1555, 1, 0x0D, 0x004B, 5, 0]))
    );
    // A 218973.
    assert_eq!(
        drained(&mut m, "69 13 00 00 00 06 94 12 55 15 2f 06"),
        Some((6, [0x1294, 0x1555, 0x2F, 2, 0, 4, 6]))
    );
    // A 221956: check at the unit's own point (kept), then the request.
    assert_eq!(
        drained(&mut m, "6c 13 00 00 00 10 00 01 00 00 00 00 94 12 55 15"),
        Some((0x10, [0, 1, 0, 2, 0, 4, 0]))
    );
    assert_eq!(m.unit(k13).server_point, (0x1294, 0x1555));
    // A 220507: 0x0C addresses (type, GUID).
    assert_eq!(
        drained(&mut m, "0c 01 13 00 00 00 13 06 0f"),
        Some((0x13, [6, 0x0F, 0, 0, 0, 0, 0]))
    );
    // B 161: an object.
    let k = UnitKey::new(OBJECT, 13);
    m.put(k);
    assert_eq!(
        drained(&mut m, "0e 02 0d 00 00 00 03 00 02 00 00 00"),
        Some((3, [0, 2, 0, 0, 0, 0, 0]))
    );
    // B 25465: the local player; the roster is not in the model.
    local(&mut m, (0x12E4, 0x138E), 1);
    assert_eq!(
        drained(&mut m, "0d 00 01 00 00 00 13 e4 12 8e 13 11 57"),
        Some((0x13, [0x12E4, 0x138E, 0x11, 0, 0, 0, 0]))
    );
    // 0x6D adds 1 to stat 328 between the check and the request.
    m.unit_mut_stat(k13, 328, 9);
    drained(&mut m, "6d 13 00 00 00 94 12 55 15 80");
    assert_eq!(m.unit(k13).stat(328), 10);
    assert!(m.log.rejected.is_empty(), "{:?}", m.rejected());
}

#[test]
fn every_queued_row_reads_its_layout() {
    // Synthetic: each id, bytes 0..n, the unit (1, 0x04030201) /
    // (1 at +1, 0x05040302 at +2) without a position check reaching a
    // correction (check points 0 or equal).
    let cases: &[(&str, u8, [i32; 7])] = &[
        (
            "4c 01 02 03 04 05 06 07 08 09 0a 0b 0c 0d 0e 0f",
            0x16,
            [0x0706, -1, 9, 0x0D0C_0B0A, 8, 0, 0],
        ),
        (
            "4d 01 02 03 04 05 06 07 08 09 0a 0b 0c 0d 0e 0f 10",
            0x15,
            [0x0908_0706, -1, 0x0C0B, 0x0E0D, 0x0A, 0, 0],
        ),
        (
            "68 01 02 03 04 05 00 00 00 00 0a 0b 0c 0d 0e 0f 10 11 12 ff 14",
            5,
            [0x0A, 0x0E0D_0C0B, 0x0F, 0x11, -238, 0x14, 0x10],
        ),
        (
            "6a 01 02 03 04 05 06 07 08 09 0a 0b",
            5,
            [6, 0x0A09_0807, 0x0B, 2, 0, 4, 0],
        ),
        (
            "6b 01 02 03 04 05 06 07 08 09 0a 0b 00 00 00 00",
            5,
            [0x0706, 0x0908, 0x0A, 2, 0, 4, 0x0B],
        ),
        (
            "0f 01 02 03 04 05 06 07 08 09 0a 0b 00 00 00 00",
            6,
            [0x0807, 0x0A09, 0x0B, 0, 0, 0, 0],
        ),
        (
            "10 01 02 03 04 05 06 07 08 09 0a 0b 00 00 00 00",
            6,
            [7, 0x0B0A_0908, 0, 0, 0, 0, 0],
        ),
    ];
    for (msg, code, record) in cases {
        let mut m = Model::default();
        let b = hex(msg);
        let k = super::super::world::addressed_unit(&b).unwrap();
        m.put(k).mode = 1;
        m.recv(&b);
        assert_eq!(m.drain(), 1, "{msg}");
        assert_eq!(request(&m, k), Some((*code, *record)), "{msg}");
    }
    // A missile's request does nothing.
    let mut m = Model::default();
    let k = UnitKey::new(MISSILE, 2);
    m.put(k);
    let b = hex("0e 03 02 00 00 00 03 00 02 00 00 00");
    let msg = super::super::dispatch::UnitMessage {
        id: 0x0E,
        bytes: &b,
        unit: k,
        inputs: &m.inputs,
        out: &Default::default(),
    };
    let mut w = m.w.clone();
    queued(&mut w, &msg).unwrap();
    assert_eq!(w.units[&k], ClientUnit::new(k));
}

#[test]
fn vitals_b11145_b15933_b140_b32694_b12323() {
    let mut m = Model::default();
    // No local player: nothing.
    m.hex("96 4a 80 8e 89 c0 09 00 00");
    assert!(m.w.units.is_empty() && m.log.rejected.is_empty());
    // B 11145: at the stated point, no correction.
    local(&mut m, (0x131D, 0x1381), 2);
    m.hex("96 4a 80 8e 89 c0 09 00 00");
    let u = m.unit(P1);
    assert_eq!((u.stat(10), u.server_point), (0x4A00, (0x131D, 0x1381)));
    assert!(m.w.outgoing.is_empty());
    // B 15933: (0x1318, 0x1387), target (0x131B, 0x1383): dy 0xFC → −4.
    local(&mut m, (0x1318, 0x1387), 2);
    m.hex("96 48 00 8c 89 c3 89 01 7e");
    let u = m.unit(P1);
    assert_eq!((u.stat(10), u.server_point), (0x4800, (0x1318, 0x1387)));
    // B 140: x = 0, no check.
    local(&mut m, (0x10, 0x10), 2);
    m.hex("95 24 00 0c 80 12 00 00 00 00 00 00 00");
    let u = m.unit(P1);
    assert_eq!(
        (u.stat(6), u.stat(8), u.stat(10), u.server_point),
        (0x2400, 0x1800, 0x4A00, (0, 0))
    );
    // B 32694.
    local(&mut m, (0x12DD, 0x1391), 2);
    m.hex("95 1c 80 0a 80 12 a0 5b 22 72 c2 bf 00");
    let u = m.unit(P1);
    assert_eq!(
        (u.stat(6), u.stat(8), u.stat(10), u.server_point),
        (0x1C00, 0x1500, 0x4A00, (0x12DD, 0x1391))
    );
    // B 12323.
    local(&mut m, (0x131D, 0x1381), 2);
    m.hex("18 24 80 0c 80 12 80 0c e8 98 08 9c 00 00 00");
    let u = m.unit(P1);
    assert_eq!(
        (u.stat(6), u.stat(8), u.stat(10), u.stat(74), u.stat(26)),
        (0x2400, 0x1900, 0x4A00, 0x64, 0)
    );
    assert_eq!(u.server_point, (0x131D, 0x1381));
    assert!(m.log.rejected.is_empty() && m.w.outgoing.is_empty());
}

#[test]
fn vitals_correction_and_dx_0x80() {
    // Synthetic: the local player in mode 1 (T 3) at (100, 100); 0x96
    // states (110, 100) with dx 0x80: the target is x + 128 (d2 ≥ d1),
    // so the player asks with its own position.
    let mut m = Model::default();
    local(&mut m, (100, 100), 1);
    let mut b = vec![0x96];
    b.extend(pack(&[(0x10, 15), (110, 16), (100, 16), (0x80, 8), (0, 8)]));
    m.recv(&b);
    assert_eq!(m.w.outgoing, vec![hex("5f 64 00 64 00")]);
    // dx 0xFF → −1.
    let mut m = Model::default();
    local(&mut m, (104, 100), 1);
    let mut b = vec![0x96];
    b.extend(pack(&[(0x10, 15), (108, 16), (100, 16), (0xFF, 8), (0, 8)]));
    // (108, 100) from (104, 100): 4 > 3, d1 = 16, target (107, 100):
    // d2 = 9 < 16 → accepted; y = cy → visible: no correction.
    m.recv(&b);
    assert!(m.w.outgoing.is_empty());
    assert_eq!(m.unit(P1).position, Some((104, 100)));
    // A dead local player: stamina set, no check.
    let mut m = Model::default();
    local(&mut m, (100, 100), 0x11);
    let mut b = vec![0x96];
    b.extend(pack(&[(0x10, 15), (150, 16), (100, 16), (0, 8), (0, 8)]));
    m.recv(&b);
    assert_eq!(m.unit(P1).stat(10), 0x1000);
    assert!(m.w.outgoing.is_empty());
}

impl Model {
    fn unit_mut_stat(&mut self, k: UnitKey, stat: u16, v: i32) {
        self.w.units.get_mut(&k).unwrap().stats.insert(stat, v);
    }
}

#[test]
fn assign_object_shrine_record_and_never_sent_types() {
    use super::super::output::{Output, ShrineFxKind};
    use super::super::world::ObjectRow;
    let mut m = Model::default();
    m.inputs.tables.objects = vec![ObjectRow::default(); 40];
    m.inputs.tables.objects[37].subclass = 1;
    m.inputs.tables.shrines = vec![0, 2, 6];
    // Class 37 (a shrine), interact 2 → shrines row 2, code 6 (on-mode).
    m.hex("51 02 0d 00 00 00 25 00 14 12 c0 11 02 02");
    let k = UnitKey::new(OBJECT, 13);
    match &m.unit(k).kind {
        KindData::Object(d) => assert_eq!((d.interact, d.shrine), (2, Some(6))),
        k => panic!("{k:?}"),
    }
    assert!(m.out.iter().any(|o| matches!(
        o,
        Output::ShrineFx { kind: ShrineFxKind::OnMode, code: 6, object, .. } if *object == k
    )));
    // Interact past the shrines rows: fatal 0x15F.
    m.hex("51 02 0e 00 00 00 25 00 14 12 c0 11 02 09");
    assert_eq!(m.rejected(), [(0x51, "fatal assert 0x15F".to_owned())]);
    // Types 0, 3, 4, 5 are never sent: refused.
    m.hex("51 04 0f 00 00 00 25 00 14 12 c0 11 02 00");
    assert_eq!(m.rejected().len(), 2);
    assert!(!m.w.units.contains_key(&UnitKey::new(4, 15)));
}

#[test]
fn assign_monster_hireling_reinit_and_source_link() {
    use super::super::world::{PetRecord, PET_HIRELING};
    let mut m = with_monsters();
    m.inputs.tables.monsters = vec![Some(MonsterClass::default()); 400];
    local(&mut m, (0, 0), 1);
    m.w.pets.push(PetRecord {
        class: 338,
        pet_type: PET_HIRELING,
        pet: 9,
        owner: 1,
        f1c: 100,
        gone: false,
        extra: None,
    });
    let k = UnitKey::new(MONSTER, 9);
    let u = m.put(k);
    u.class = 338;
    u.stats.insert(6, 77);
    // Walking (mode 2) with a running walk animation.
    u.mode = 2;
    u.frame_count = 13 << 8;
    // Mode 1, no components, no flags, link bit 1 with v = 5, no list.
    let mut msg = vec![0xAC, 9, 0, 0, 0, 0x52, 0x01, 0, 0, 0, 0, 0x80, 0];
    msg.extend(pack(&[(1, 4), (0, 1), (0, 1), (1, 1), (5, 31), (0, 1)]));
    msg[0xC] = msg.len() as u8;
    m.recv(&msg);
    let u = m.unit(k);
    // Re-initialised: the existing unit keeps its class; mode := 1; a
    // hireling class with the hireling GUID keeps stats 6 / 7.
    assert_eq!((u.class, u.mode), (338, 1));
    // The mode set of a new mode restarts the animation: the
    // count of mode 1 (no AnimData row here: 0).
    assert_eq!(u.frame_count, 0);
    assert_eq!(u.stats.get(&6), Some(&77));
    assert_eq!(u.stats.get(&328), Some(&0));
    // Step 4: state 98 with stat 354 = v, flag-ex 0x400.
    assert!(u.states.contains(&98));
    assert_eq!(u.state_lists[&98].get(&(354, 0)), Some(&5));
    assert_eq!(u.flag_ex & 0x400, 0x400);
}

#[test]
fn assign_monster_hireling_setup_takes_the_hireling_modes() {
    use super::super::world::{HirelingRows, PetRecord, SkillRow, PET_HIRELING};
    use d2_sim::world::hirelings::{HirelingRow, RowSkill};
    let mut m = with_monsters();
    local(&mut m, (0, 0), 1);
    m.w.units.get_mut(&P1).unwrap().stats.insert(12, 30);
    m.w.expansion = 1;
    // Skills 7 and 8 have `monanim` 14 (a blank `xx` link); row count 12.
    m.inputs.tables.skills = vec![
        SkillRow {
            monanim: 14,
            ..SkillRow::default()
        };
        12
    ];
    let slot = |skill, mode, level, lvl_per_lvl| RowSkill {
        skill,
        mode,
        level,
        lvl_per_lvl,
    };
    let row = |level| HirelingRow {
        version: 100,
        id: 1,
        class: 7,
        act: 1,
        difficulty: 1,
        level,
        name_first: 0x0D00,
        name_last: 0x0DFF,
        skills: [
            slot(7, 4, 1, 32),
            slot(8, -1, 2, 0),
            slot(0, 4, 1, 0),
            slot(9, 4, 1, 0),
            RowSkill::default(),
            RowSkill::default(),
        ],
        ..HirelingRow::default()
    };
    let rows = HirelingRows::new(vec![row(3), row(30)]);
    let offer = rows.offer(true, 30, 0x5678, 0, 0).unwrap();
    m.inputs.tables.hirelings = std::sync::Arc::new(rows);
    m.w.pets.push(PetRecord {
        class: 7,
        pet_type: PET_HIRELING,
        pet: 9,
        owner: 1,
        f1c: 100,
        gone: false,
        extra: Some([0x5678, 0x0D53, 0]),
    });
    // A new monster (9, class 7): mode 1, nothing else.
    let mut msg = vec![0xAC, 9, 0, 0, 0, 7, 0, 0, 0, 0, 0, 0x80, 0];
    msg.extend(pack(&[(1, 4), (0, 1), (0, 1), (0, 1), (0, 1)]));
    msg[0xC] = msg.len() as u8;
    // The 0xAC loads the class's palshift block.
    assert!(!m.w.palshift_classes.contains(&7));
    m.recv(&msg);
    assert!(m.w.palshift_classes.contains(&7));
    let u = m.unit(UnitKey::new(MONSTER, 9));
    assert!(u.flag_200 && u.flag_2 == Some(true));
    let l = u.skills.as_ref().unwrap();
    // The bracket of the offer's level (L in 25…29 → the level-3 row):
    // slot 0 at 1 + 32·d >> 5, capped 0x20, mode 4 from the row; slot 1
    // keeps `monanim` (a negative mode); slot 2 (skill 0) ends the walk.
    let d = offer.level - 3;
    let e7 = &l.entries[l.native(7).unwrap()];
    assert_eq!((e7.base, e7.mode), ((1 + d).min(0x20), 4));
    let e8 = &l.entries[l.native(8).unwrap()];
    assert_eq!((e8.base, e8.mode), (2, 14));
    assert_eq!(l.native(9), None);
}

#[test]
fn units_get_their_lights_at_creation() {
    use super::super::world::ObjectRow;
    use crate::rules::lighting::records::{LightKind, LightRecord};
    let head = |m: &Model| m.w.lights.iter().next().unwrap().1.clone();
    let fields = |r: &LightRecord| {
        (
            (r.owner_type, r.owner_guid),
            r.kind,
            r.radius,
            (r.i, r.r, r.g, r.b),
        )
    };
    let mut m = with_monsters();
    // 0x59 (B102): the player init's light, kind 0 while no local player
    // exists, radius 13, white.
    let mut b = hex("59 01 00 00 00 01 77 65 72 77 65 72 00");
    b.resize(26, 0);
    m.recv(&b);
    assert_eq!(
        fields(&head(&m)),
        ((0, 1), LightKind::Shadowed, 8 * 13, (255, 255, 255, 255))
    );
    // Another player once the local one exists: kind 1.
    m.w.local_player = Some(P1);
    let mut b = hex("59 02 00 00 00 01 61 00");
    b.resize(26, 0);
    m.recv(&b);
    assert_eq!(
        fields(&head(&m)),
        ((0, 2), LightKind::Plain, 8 * 13, (255, 255, 255, 255))
    );
    // 0xAC (B157), class 154 with `monstats2` `Light` 5 and its colour:
    // kind 0, radius 5.
    m.inputs.tables.monsters[154] = Some(MonsterClass {
        light: 5,
        light_rgb: (230, 168, 255),
        ..MonsterClass::default()
    });
    m.hex("ac 06 00 00 00 9a 00 1a 12 b9 11 80 0e 01");
    assert_eq!(
        fields(&head(&m)),
        ((1, 6), LightKind::Shadowed, 8 * 5, (255, 230, 168, 255))
    );
    // `Light` 0: none.
    m.inputs.tables.monsters[153] = Some(MonsterClass::default());
    let n = m.w.lights.len();
    m.hex("ac 07 00 00 00 99 00 1a 12 b9 11 80 0e 01");
    assert_eq!(m.w.lights.len(), n);
    // 0x51: the object init's light, `Lit<mode>` / 2, kind 2.
    m.inputs.tables.objects = vec![ObjectRow::default(); 4];
    m.inputs.tables.objects[3] = ObjectRow {
        lit: [0, 0, 12, 0, 0, 0, 0, 0],
        rgb: (9, 8, 7),
        ..ObjectRow::default()
    };
    m.hex("51 02 07 00 00 00 03 00 10 00 10 00 02 00");
    assert_eq!(
        fields(&head(&m)),
        ((2, 7), LightKind::Cached, 8 * 6, (255, 9, 8, 7))
    );
    assert!(m.log.rejected.is_empty(), "{:?}", m.log.rejected);
}

#[test]
fn the_local_players_light_colour_follows_its_states_and_stats() {
    use super::super::world::StateRow;
    use crate::rules::lighting::records::LightRecord;
    // Live rows: 2 `poison` (colorpri 95,
    // colorshift 104, light 128, 255, 128), 90 `blue` (100, 108, 150,
    // 215, 255).
    let mut m = Model::default();
    m.inputs.tables.states = vec![StateRow::default(); 91];
    m.inputs.tables.states[2] = StateRow {
        colorpri: 95,
        colorshift: 104,
        light_rgb: (128, 255, 128),
        ..StateRow::default()
    };
    m.inputs.tables.states[90] = StateRow {
        colorpri: 100,
        colorshift: 108,
        light_rgb: (150, 215, 255),
        ..StateRow::default()
    };
    let mut b = hex("59 01 00 00 00 01 77 65 72 77 65 72 00");
    b.resize(26, 0);
    m.recv(&b);
    m.w.local_player = Some(P1);
    let light = |m: &Model| -> LightRecord {
        let id = super::lighting::unit_light(&m.w, P1).unwrap();
        m.w.lights.get(id).unwrap().clone()
    };
    let rgb = |m: &Model| {
        let l = light(m);
        (l.r, l.g, l.b)
    };
    // 0xA7 state on, 0xA9 state off.
    m.hex("a7 00 01 00 00 00 02");
    assert_eq!(rgb(&m), (128, 255, 128));
    m.hex("a7 00 01 00 00 00 5a");
    assert_eq!(rgb(&m), (150, 215, 255), "the greater colorpri wins");
    m.hex("a9 00 01 00 00 00 5a");
    assert_eq!(rgb(&m), (128, 255, 128));
    m.hex("a9 00 01 00 00 00 02");
    assert_eq!(rgb(&m), (255, 255, 255), "no state wins: white");
    // Stat 90 (0x20) sets the colour when it changes; a later state call
    // writes over it (the last writer stands).
    m.hex("20 01 00 00 00 5a 00 00 ff 00");
    assert_eq!(rgb(&m), (255, 0, 0));
    m.hex("a7 00 01 00 00 00 02");
    assert_eq!(rgb(&m), (128, 255, 128));
    // Stat 89: radius 13 + 3.
    m.hex("20 01 00 00 00 59 03 00 00 00");
    assert_eq!(light(&m).radius, 8 * 16);
    // A state with colorshift 0 runs no colour call.
    m.w.lights
        .set_color(super::lighting::unit_light(&m.w, P1).unwrap(), 255, 1, 2, 3);
    m.hex("a7 00 01 00 00 00 03");
    assert_eq!(rgb(&m), (1, 2, 3));
    assert!(m.log.rejected.is_empty(), "{:?}", m.log.rejected);
}

#[test]
fn the_palette_index_changes_only_on_a_tinted_state_toggle() {
    use super::super::world::StateRow;
    // The base game runs it only for a state
    // with `colorshift` ≠ 0; it writes unit +0x6C for any unit. Rows: 2
    // `poison` (95, 104), 90 `blue` (100, 108); synthetic 3 (pri 120,
    // shift 0) and 0 (pri 200, shift 9).
    let mut m = Model::default();
    m.inputs.tables.states = vec![StateRow::default(); 91];
    let row = |colorpri, colorshift, light_rgb| StateRow {
        colorpri,
        colorshift,
        light_rgb,
        ..StateRow::default()
    };
    m.inputs.tables.states[2] = row(95, 104, (128, 255, 128));
    m.inputs.tables.states[90] = row(100, 108, (150, 215, 255));
    m.inputs.tables.states[3] = row(120, 0, (1, 2, 3));
    m.inputs.tables.states[0] = row(200, 9, (4, 5, 6));
    let mut b = hex("59 01 00 00 00 01 77 65 72 77 65 72 00");
    b.resize(26, 0);
    m.recv(&b);
    let p = |m: &Model| m.w.units[&P1].palette_index;
    m.hex("a7 00 01 00 00 00 02");
    assert_eq!(p(&m), 104, "not the local player: still tinted");
    // State 3 (shift 0) turns on without a call, though it would win.
    m.hex("a7 00 01 00 00 00 03");
    assert_eq!(p(&m), 104);
    // The next call sees it: 3 wins with shift 0.
    m.hex("a7 00 01 00 00 00 5a");
    assert_eq!(p(&m), 0);
    m.hex("a9 00 01 00 00 00 03");
    assert_eq!(p(&m), 0, "state 3 off runs no call");
    m.hex("a9 00 01 00 00 00 02");
    assert_eq!(p(&m), 108);
    // A winner with id 0 is none: index 0 and, for the local player, white.
    m.w.local_player = Some(P1);
    m.hex("a7 00 01 00 00 00 00");
    m.hex("a7 00 01 00 00 00 02");
    assert_eq!(p(&m), 0);
    let id = super::lighting::unit_light(&m.w, P1).unwrap();
    let l = m.w.lights.get(id).unwrap();
    assert_eq!((l.r, l.g, l.b), (255, 255, 255));
    assert!(m.log.rejected.is_empty(), "{:?}", m.log.rejected);
}

#[test]
fn umod_hooks_draw_the_champion_light_and_reseed_named_uniques() {
    use super::super::world::MonsterData;
    use super::units::umod_hooks;
    use d2_sim::rng::Seed;
    let start = Seed::new(714_489_349, 666);
    let data = |flags: u8, umods: &[u8], name_seed: u16| {
        let mut d = MonsterData {
            flags,
            name_seed,
            ..MonsterData::default()
        };
        d.umods[..umods.len()].copy_from_slice(umods);
        d
    };
    // Corpsefire's pack (flags 4 | 8, umod 39): only the light hook
    // draws, three steps, colour from the third, second, first.
    let mut s = start;
    let req = umod_hooks(&data(0xC, &[39], 0x92), 6, (1, 2, 3), &mut s).unwrap();
    let mut e = start;
    let (a, b, c) = (e.step(), e.step(), e.step());
    assert_eq!(s, e);
    assert_eq!((req.r, req.g, req.b), (c as u8, b as u8, a as u8));
    // No umods: no hook runs.
    let mut s = start;
    assert!(umod_hooks(&data(0xC, &[], 0x92), 6, (1, 2, 3), &mut s).is_none());
    assert_eq!(s, start);
    // Not unique (u = 0): every hook is a no-op.
    let mut s = start;
    assert!(umod_hooks(&data(4, &[16], 0x92), 6, (1, 2, 3), &mut s).is_none());
    assert_eq!(s, start);
    // A superunique keeps its seed; its light takes the monstats2 colour.
    let mut s = start;
    let req = umod_hooks(&data(8 | 2, &[16], 0x92), 6, (1, 2, 3), &mut s).unwrap();
    assert_eq!(s, start);
    assert_eq!((req.r, req.g, req.b), (1, 2, 3));
    // A plain unique: the last name hook (umod 16) reseeds from the name
    // seed, then 2 draws and rnd(100) (< 50: 3 more).
    let mut s = start;
    umod_hooks(&data(8, &[16], 0x92), 6, (1, 2, 3), &mut s);
    let mut e = Seed::init_low(0x92);
    e.step();
    e.step();
    if e.roll(100) < 50 {
        e.step();
        e.step();
        e.step();
    }
    assert_eq!(s, e);
    // Umod 12 and the fixed-name classes: the reseed alone (umod 12 is
    // the last name hook here).
    let mut s = start;
    umod_hooks(&data(8, &[12], 0x92), 6, (1, 2, 3), &mut s);
    assert_eq!(s, Seed::init_low(0x92));
    let mut s = start;
    umod_hooks(&data(8, &[16], 0x92), 0x9C, (1, 2, 3), &mut s);
    assert_eq!(s, Seed::init_low(0x92));
}

#[test]
fn a_unique_is_reseeded_from_class_and_name_seed_byte() {
    use super::super::world::MonsterData;
    use super::units::unique_reseed;
    // Recorded (the Den of Evil): Corpsefire's pack, class 6, name
    // seed bytes 0x92 and 0x5B.
    let d = |flags: u8, name_seed: u16| MonsterData {
        flags,
        name_seed,
        ..MonsterData::default()
    };
    assert_eq!(
        unique_reseed(&d(0xC, 0x92), 6, false),
        Some((1_709_569_938, 63))
    );
    assert_eq!(
        unique_reseed(&d(0xC, 0x5B), 6, false),
        Some((1_966_923_071, 40))
    );
    assert_eq!(unique_reseed(&d(0xC, 0x92), 6, true), None, "noUniqueShift");
    assert_eq!(unique_reseed(&d(4, 0x92), 6, false), None, "not unique");
}

#[test]
fn component_light_is_the_largest_chosen_item_radius() {
    use super::lighting::component_radius;
    let mut class = MonsterClass::default();
    // RH: 3 choices, choice 2 is a torch (`tch`, `lightradius` 6).
    class.components[5] = 3;
    class.component_light[5] = [0, 0, 6, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    // SH: 2 choices, choice 1 radius 2; the radius 9 behind the count is
    // never read (no code).
    class.components[7] = 2;
    class.component_light[7] = [0, 2, 9, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    let mut choices = [0u8; 16];
    assert_eq!(component_radius(&class, &choices), 0);
    choices[7] = 1;
    assert_eq!(component_radius(&class, &choices), 2);
    choices[5] = 2;
    assert_eq!(component_radius(&class, &choices), 6);
    choices[5] = 1;
    choices[7] = 2;
    assert_eq!(component_radius(&class, &choices), 0);
}

#[test]
fn state_121_on_and_off_clear_the_skill_use_lock() {
    use super::super::world::StateRow;
    // Live `states` row 121 `skilldelay`: setfunc 19, remfunc 12.
    let mut m = Model::default();
    m.inputs.tables.states = vec![StateRow::default(); 122];
    m.inputs.tables.states[121] = StateRow {
        setfunc: 19,
        remfunc: 12,
        ..StateRow::default()
    };
    let mut b = hex("59 01 00 00 00 01 77 65 72 77 65 72 00");
    b.resize(26, 0);
    m.recv(&b);
    m.w.skill_lock = 40;
    m.hex("a7 00 01 00 00 00 79");
    assert_eq!(m.w.skill_lock, 0);
    m.w.skill_lock = 40;
    m.hex("a9 00 01 00 00 00 79");
    assert_eq!(m.w.skill_lock, 0);
    // Another state leaves it.
    m.w.skill_lock = 40;
    m.hex("a7 00 01 00 00 00 02");
    m.hex("a9 00 01 00 00 00 02");
    assert_eq!(m.w.skill_lock, 40);
}

/// The first pet record of the
/// GUID decides; none or owner −1 → 1, any other owner → 0.
#[test]
fn pet_shift_relation_reads_the_first_pet_record() {
    use super::super::world::{ClientWorld, PetRecord};
    use super::units::pet_shift_relation;
    let pet = |pet, owner| PetRecord {
        class: 363,
        pet_type: 4,
        pet,
        owner,
        f1c: 100,
        gone: false,
        extra: None,
    };
    let mut w = ClientWorld::default();
    assert!(pet_shift_relation(&w, 9));
    w.pets = vec![pet(9, 1), pet(9, u32::MAX)];
    assert!(!pet_shift_relation(&w, 9));
    w.pets = vec![pet(9, u32::MAX), pet(9, 1)];
    assert!(pet_shift_relation(&w, 9));
    w.pets = vec![pet(9, 5)];
    assert!(!pet_shift_relation(&w, 9));
    assert!(pet_shift_relation(&w, 10));
}
