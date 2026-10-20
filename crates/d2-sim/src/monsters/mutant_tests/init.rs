// (rules the mutation run found unchecked; fakes from the parent test
// module)
use super::*;

fn request(class: u32, flags: u16) -> CreateRequest {
    CreateRequest {
        class,
        mode: 1,
        x: 10,
        y: 20,
        flags,
        ..CreateRequest::default()
    }
}

#[test]
fn region_count_nc_is_flags_and_8() {
    // nc = flags & 8 (other bits do not leak in), or 1 with `neverCount`.
    let mut ms = vec![mon(1, 1, 1, 1); 2];
    ms[1].nevercount = true;
    let mut f = fake(ms);
    let cx = f.cx;
    for (class, flags, nc) in [
        (0, 0x00, 0),
        (0, 0x02, 0),
        (0, 0x0A, 8),
        (0, 0x08, 8),
        (1, 0x02, 1),
    ] {
        f.log.clear();
        let u = create(&cx, &mut f, &request(class, flags))
            .unwrap()
            .unwrap();
        let line = format!("register {} {nc}", u.0);
        assert!(f.log.contains(&line), "class {class} flags {flags:#x}");
    }
}

#[test]
fn flag_8_marks_the_monster_not_counted() {
    let mut f = fake(vec![mon(1, 1, 1, 1)]);
    let cx = f.cx;
    for (flags, want) in [(0x00, false), (0x02, false), (0x08, true), (0x0A, true)] {
        let u = create(&cx, &mut f, &request(0, flags)).unwrap().unwrap();
        assert_eq!(f.data(u).not_counted, want, "flags {flags:#x}");
    }
}

#[test]
fn alignment_mapping() {
    // `Align` 1 → 2 (and unit flag 0x20000), 2 → 1, else 0.
    let mut ms = vec![mon(1, 1, 1, 1); 4];
    (ms[1].align, ms[2].align, ms[3].align) = (1, 2, 3);
    let mut f = fake(ms);
    let cx = f.cx;
    for (class, want, flag) in [(0, 0, false), (1, 2, true), (2, 1, false), (3, 0, false)] {
        f.log.clear();
        let u = create(&cx, &mut f, &request(class, 0x40)).unwrap().unwrap();
        assert!(
            f.log.contains(&format!("align {} {want}", u.0)),
            "Align class {class}"
        );
        let has = f.units.get(u).unwrap().flags & unit_flag::ALIGN1 != 0;
        assert_eq!(has, flag, "class {class}");
    }
}

#[test]
fn monprop_id_0_is_a_property_and_chance_is_strict() {
    // Property id 0 is applied (only ids < 0 stop); with a chance, the
    // draw must be below it.
    let lo = (1..1_000_000u32)
        .find(|&lo| {
            let mut r = Seed::init_low(lo);
            r.step(); // HP roll
            r.step() % 100 == 40
        })
        .unwrap();
    let mut m = mon(1, 1, 1, 1);
    m.monprop = 0;
    let mut p: Monprop = zero();
    p.prop1 = 0;
    p.par1 = 1;
    p.prop2 = 6;
    p.chance2 = 40;
    p.prop3 = u32::MAX;
    let mut t = Tables::new(vec![m]);
    t.monprop = vec![p];
    let mut f = fake_with(t);
    f.monster(0, lo);
    let props: Vec<&String> = f.log.iter().filter(|l| l.starts_with("prop")).collect();
    assert_eq!(props, ["prop 0 1 0 0"]);
}

#[test]
fn monequip_row_at_the_monster_level_is_used() {
    // Skip rows while row level > monster level: a row at exactly the
    // level (5) is the one used.
    let mut t = Tables::new(vec![mon(1, 1, 1, 1), mon(5, 1, 1, 1)]);
    t.monequip = vec![
        equip(1, 6, 1, &[(b"aaa ", 4)]),
        equip(1, 5, 1, &[(b"bbb ", 5)]),
        equip(1, 0, 1, &[(b"ccc ", 7)]),
    ];
    let mut f = fake_with(t);
    f.inventory = true;
    f.monster(1, 1);
    let items: Vec<&String> = f.log.iter().filter(|l| l.starts_with("item")).collect();
    // Slot 1 of `equip` has modifier 9: outside 0..7, so 0.
    assert_eq!(items, ["item bbb  5 0 5", "item ccc  7 0 5"]);
}

#[test]
fn unique_name_roll_50_has_no_appellation() {
    // `roll(100)` < 50 is strict: a third draw of 50 keeps the first pair.
    let s = (0..=u16::MAX)
        .find(|&s| {
            let mut r = Seed::init_low(u32::from(s));
            r.roll(20);
            r.roll(30);
            r.roll(100) == 50
        })
        .unwrap();
    let mut r = Seed::init_low(u32::from(s));
    let (suf, pre) = (r.roll(20), r.roll(30));
    assert_eq!(
        unique_name(s, 20, 30, 10),
        UniqueName {
            prefix: pre,
            suffix: suf,
            appellation: None
        }
    );
}

#[test]
fn fast_reads_the_units_own_class() {
    // Umod 6 reads `Velocity` of the unit's class: class 1 (Velocity 4)
    // gets +100, while class 0 has Velocity 0.
    let mut f = fake(vec![velocity_mon(0, 1), velocity_mon(4, 1)]);
    let u = f.monster(1, 1);
    let before = f.s(u, stat::VELOCITYPERCENT);
    let cx = f.cx;
    run_umod_init(&cx, &mut f, u, 6, false);
    assert_eq!(f.s(u, stat::VELOCITYPERCENT), before + 100);
}
