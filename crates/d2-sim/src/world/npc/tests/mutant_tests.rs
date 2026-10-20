// Spec: specs/world/npc.md §2, §3, §5
//! Mutation-testing kills (METHODS M08): each test pins an outcome the
//! spec decides that no earlier test checked. A child of `npc::tests`
//! for its fake world.

use super::*;

// From specs/world/npc.md §2: the handler's type test is "> 5".
#[test]
fn unit_type_5_is_not_refused() {
    let mut c = control(0);
    let mut w = Fake::new();
    w.npc(class::CHARSI, 6);
    let mut m = msg13(6);
    m[1] = 5;
    assert_eq!(c.interact(&mut w, PLAYER, &m).unwrap(), None);
}

// From specs/world/npc.md §2 r2: the AI halt needs both `npc` and
// `interact`.
#[test]
fn ai_halt_needs_npc_and_interact() {
    let blank = Monstats::decode(&[0u8; Monstats::SIZE]);
    let mut rows = vec![blank; 4];
    rows[1].npc = true; // npc only
    rows[2].interact = true; // interact only
    rows[3].npc = true;
    rows[3].interact = true;
    let mut game = Seed::init_low(7);
    let mut c = NpcControl::new(&rows, vec![], true, 0, &mut game).unwrap();
    for (class, halted) in [(1, false), (2, false), (3, true)] {
        let mut w = Fake::new();
        w.npc(class, 40);
        w.dist = 20;
        let _ = c.interact(&mut w, PLAYER, &msg13(40));
        assert_eq!(w.has("think 1040"), halted, "class {class}");
    }
}

// From specs/world/npc.md §2 r4: distance ≤ 6 starts the interaction.
#[test]
fn distance_6_starts() {
    let mut c = control(0);
    let mut w = Fake::new();
    let charsi = w.npc(class::CHARSI, 6);
    w.dist = 6;
    assert_eq!(c.interact(&mut w, PLAYER, &msg13(6)).unwrap(), Some(0));
    assert!(!w.log.iter().any(|l| l.starts_with("approach")));
    assert_eq!(w.list(charsi), [(PLAYER, 0)]);
}

// From specs/world/npc.md §3: a monster without an interaction list → 3.
#[test]
fn chat_needs_an_interaction_list() {
    let mut c = control(0);
    let mut w = Fake::new();
    let charsi = w.npc(class::CHARSI, 6);
    w.units.get_mut(&charsi).unwrap().interaction = None;
    assert_eq!(c.chat_open(&mut w, PLAYER, &msg9(0x2F, 6)), 3);
}

// From specs/world/npc.md §5 r3–r6: any one change alone plays the sound.
#[test]
fn heal_sound_for_each_single_change() {
    let pet = UnitId(50);
    type Setup<'a> = &'a dyn Fn(&mut Fake);
    let cases: [(&str, Setup); 5] = [
        ("player poison", &|w| {
            w.units.get_mut(&PLAYER).unwrap().lists = [2].into();
        }),
        ("player freeze", &|w| {
            w.units.get_mut(&PLAYER).unwrap().lists = [1].into();
        }),
        ("player curable", &|w| {
            w.states_count = 8;
            w.curable = [3].into();
            let p = w.units.get_mut(&PLAYER).unwrap();
            p.states = [3].into();
            p.lists = [3].into();
        }),
        ("pet poison", &|w| {
            w.units.get_mut(&pet).unwrap().lists = [2].into();
        }),
        ("pet freeze", &|w| {
            w.units.get_mut(&pet).unwrap().lists = [1].into();
        }),
    ];
    for (name, setup) in cases {
        let mut c = control(0);
        let mut w = Fake::new();
        w.npc(class::AKARA, 0x10);
        w.units.insert(
            pet,
            Unit {
                guid: 50,
                ..Unit::default()
            },
        );
        w.pets = vec![pet];
        setup(&mut w);
        c.interact(&mut w, PLAYER, &msg13(0x10)).unwrap();
        w.log.clear();
        c.chat_open(&mut w, PLAYER, &msg9(0x2F, 0x10));
        assert!(w.has("sound 1016 10"), "{name}: {:?}", w.log);
    }
}

// From specs/world/npc.md §7.1: the list holds n = last − first + 1 slots,
// 1 … 69. An empty or inverted range is refused (strict input, M07).
#[test]
fn hire_list_sizes_1_and_69() {
    for (last, ok) in [(1000, true), (1068, true), (999, false), (998, false)] {
        let mut c = control(0);
        c.hirelings[0].name_last = last;
        let r = c.make_hire_list(class::KASHYA);
        assert_eq!(r.is_ok(), ok, "last {last}");
        if ok {
            let n = usize::from(last - 1000 + 1);
            let h = c.record(class::KASHYA).unwrap().hire.as_ref().unwrap();
            assert_eq!(h.slots.len(), n);
        }
    }
}

// From specs/world/npc.md §7.3 r2, r3: the Kashya gate is lvl < 8; a name
// below `first` is refused with code 9.
#[test]
fn hire_gate_level_8_and_name_below_first() {
    let mut c = control(0);
    let mut w = Fake::new();
    let kashya = w.npc(class::KASHYA, 12);
    talk(&mut c, &mut w, kashya);
    let last = |w: &Fake| w.sent.last().unwrap()[2];
    w.set(PLAYER, stat::LEVEL, 8);
    w.set(PLAYER, stat::GOLD, 10_000);
    c.hire(&mut w, PLAYER, &msg36(12, 999)).unwrap();
    assert_eq!(last(&w), 9);
    c.hire(&mut w, PLAYER, &msg36(12, 1005)).unwrap();
    assert_eq!(last(&w), code::MERC);
}
