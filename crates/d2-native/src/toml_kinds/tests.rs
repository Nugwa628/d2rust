//! Round trips, perturbations and strict-reader checks of the TOML
//! sidecar kinds.

use d2_formats::cof::{Cof, CofLayer};
use d2_formats::ds1::{Ds1, Ds1Group, Ds1Object, Ds1Path, Ds1PathPoint};
use d2_formats::font::{FontTable, Glyph};

use super::*;

fn cof() -> Cof {
    let layer = |component, wc: &[u8; 4]| CofLayer {
        component,
        shadow: 1,
        selectable: 1,
        override_translucency: 0,
        new_translucency: 0,
        weapon_class: *wc,
    };
    Cof {
        layers_count: 2,
        frames: 3,
        directions: 2,
        version: 20,
        unknown: [1, 2, 3, 4],
        x_min: -10,
        x_max: 10,
        y_min: -20,
        y_max: 0,
        animation_rate: 256,
        layers: vec![layer(0, b"hth\0"), layer(15, b"\xe9\xffa ")],
        events: vec![0, 1, 3],
        event_padding: vec![9, 9],
        draw_order: (0..12).map(|i| (i % 16) as u8).collect(),
    }
}

#[test]
fn cof_round_trip() {
    let c = cof();
    let t = write_cof("a.cof.toml", &c).unwrap();
    assert!(t.starts_with(
        "native = \"cof\"\nnative_version = 1\nversion = 20\nunknown = \"0x01020304\"\n"
    ));
    assert!(t.contains("draw_order = [[0, 1, 2, 3, 4, 5], [6, 7, 8, 9, 10, 11]]"));
    assert_eq!(read_cof("a.cof.toml", &t).unwrap(), c);
    check_cof("a.cof.toml", &c).unwrap();
    assert_eq!(t, write_cof("a.cof.toml", &c).unwrap());
    // Edge: no layers, no frames, no directions.
    let e = Cof {
        layers_count: 0,
        frames: 0,
        directions: 0,
        layers: vec![],
        events: vec![],
        draw_order: vec![],
        ..cof()
    };
    check_cof("e.cof.toml", &e).unwrap();
    let e2 = Cof {
        frames: 0,
        events: vec![],
        draw_order: vec![],
        ..cof()
    };
    check_cof("e2.cof.toml", &e2).unwrap();
}

#[test]
fn cof_perturbation_and_strict_reader() {
    let c = cof();
    let t = write_cof("a.cof.toml", &c).unwrap();
    let back = read_cof("a.cof.toml", &t.replace("y_max = 0", "y_max = 1")).unwrap();
    assert_eq!(back.y_max, 1);
    assert!(first_difference(&c, &back).contains("1,"));
    for bad in [
        t.replace("native_version = 1", "native_version = 2"),
        t.replace("\"cof\"", "\"font\""),
        t.replace("0x01020304", "0x1020304"),
        t.replace("0x01020304", "0x0102030A"),
        t.replace("version = 20", "version = 256"),
        t.replace("9, 10, 11]", "9, 10, 16]"),
        t.replace("[6, 7, 8,", "[6, 7,"),
        t.replace("events = [0, 1, 3]", "events = [0, 1]"),
        t.replace("\"hth\\u0000\"", "\"hth\""),
        t.replace("\"éÿa \"", "\"Ā\u{100}a \""),
        format!("{t}zzz = 1\n"),
        t.replace("component = 15", "component = 16"),
    ] {
        assert_ne!(bad, t, "the perturbation changed nothing");
        let e = read_cof("a.cof.toml", &bad).unwrap_err();
        assert!(e.to_string().starts_with("a.cof.toml: "), "{e}\n{bad}");
    }
    let mut bad = cof();
    bad.events.pop();
    assert!(write_cof("a", &bad).is_err());
}

#[test]
fn font_round_trip_and_strict() {
    let f = FontTable {
        version: 1,
        unknown: 7,
        count: 2,
        height: 16,
        width: 12,
        glyphs: vec![
            Glyph {
                code: 0x41,
                unknown1: 1,
                width: 7,
                height: 16,
                unknown2: 2,
                unknown3: 3,
                frame: 33,
                unknown5: 0xdead_beef,
            },
            Glyph {
                code: 0xac10,
                unknown1: 0,
                width: 9,
                height: 16,
                unknown2: 0,
                unknown3: 0,
                frame: 200,
                unknown5: 0,
            },
        ],
    };
    let t = write_font("f.toml", &f).unwrap();
    assert_eq!(read_font("f.toml", &t).unwrap(), f);
    check_font("f.toml", &f).unwrap();
    let none = FontTable {
        glyphs: vec![],
        count: 0,
        ..f.clone()
    };
    check_font("f.toml", &none).unwrap();
    assert!(read_font("f.toml", &t.replace("frame = 33", "frame = 70000")).is_err());
    assert!(read_font("f.toml", &t.replace("code = 65\n", "")).is_err());
    let back = read_font("f.toml", &t.replace("frame = 200", "frame = 201")).unwrap();
    assert!(first_difference(&f, &back).contains("201"));
}

fn ds1() -> Ds1 {
    let grid = |seed: u32| -> Vec<u32> { (0..6).map(|i| seed * 0x0100_0000 + i * 0x101).collect() };
    Ds1 {
        version: 18,
        width: 3,
        height: 2,
        act: 1,
        tag_type: 1,
        files: vec![b"data\\tiles\\floor.tg1".to_vec(), vec![b'a', 0xe9, 0xff]],
        unknown_header: None,
        walls: vec![grid(1), grid(2)],
        orientations: vec![grid(3), grid(4)],
        floors: vec![grid(5)],
        shadow: grid(6),
        tags: Some(grid(7)),
        objects: vec![Ds1Object {
            kind: 1,
            id: 2,
            x: 3,
            y: 4,
            flags: 0x0000_00ff,
        }],
        unknown_groups: Some([1, 0, 0, 255]),
        groups: vec![Ds1Group {
            x: 1,
            y: 2,
            width: 3,
            height: 4,
            unknown: 5,
        }],
        groups_truncated: true,
        paths: vec![Ds1Path {
            x: 9,
            y: 8,
            points: vec![
                Ds1PathPoint {
                    x: 1,
                    y: 2,
                    action: 3,
                },
                Ds1PathPoint {
                    x: 4,
                    y: 5,
                    action: 1,
                },
            ],
        }],
        trailing: vec![0, 0xab, 0xff],
    }
}

#[test]
fn ds1_round_trip() {
    let d = ds1();
    let t = write_ds1("a.ds1.toml", &d).unwrap();
    assert!(t.contains("rows = [\"01000000 01000101 01000202\", \"01000303 01000404 01000505\"]"));
    assert!(t.contains("trailing = \"00abff\""));
    assert!(t.contains("flags = \"0x000000ff\""));
    assert_eq!(read_ds1("a.ds1.toml", &t).unwrap(), d);
    check_ds1("a.ds1.toml", &d).unwrap();
    assert_eq!(t, write_ds1("a.ds1.toml", &d).unwrap());
    // Old version: one wall, no tags, header unknown, nothing optional.
    let old = Ds1 {
        version: 9,
        tag_type: 0,
        unknown_header: Some([1, 2, 3, 4, 5, 6, 7, 8]),
        walls: vec![d.walls[0].clone()],
        orientations: vec![d.orientations[0].clone()],
        tags: None,
        unknown_groups: None,
        groups: vec![],
        groups_truncated: false,
        paths: vec![],
        objects: vec![],
        files: vec![],
        trailing: vec![],
        ..d.clone()
    };
    let t = write_ds1("o.ds1.toml", &old).unwrap();
    assert!(t.contains("unknown_header = \"0x0102030405060708\""));
    check_ds1("o.ds1.toml", &old).unwrap();
    // Empty grid.
    let empty = Ds1 {
        width: 0,
        height: 0,
        walls: vec![vec![]],
        orientations: vec![vec![]],
        floors: vec![vec![]],
        shadow: vec![],
        tags: Some(vec![]),
        ..d.clone()
    };
    check_ds1("z.ds1.toml", &empty).unwrap();
}

#[test]
fn ds1_perturbation_and_strict_reader() {
    let d = ds1();
    let t = write_ds1("a.ds1.toml", &d).unwrap();
    let back = read_ds1("a.ds1.toml", &t.replacen("01000303", "01000304", 1)).unwrap();
    assert_ne!(back, d);
    assert!(first_difference(&d, &back).contains("difference"));
    for bad in [
        t.replace("native_version = 1", "native_version = 9"),
        t.replacen("01000303", "0100030", 1),
        t.replacen("01000303", "0100030A", 1),
        t.replacen("01000303", "010003030", 1),
        t.replace("0x000000ff", "0xff"),
        t.replace("0x000000ff", "0x000000FF"),
        t.replace("00abff", "00abf"),
        t.replace("\"data\\\\tiles", "\"Ā\\\\tiles"),
        t.replace("kind = \"shadow\"", "kind = \"roof\""),
        t.replace("index = 1\nrows", "index = 2\nrows"),
        t.replacen("rows = [\"01000000 01000101 01000202\", ", "rows = [", 1),
        format!("{t}\n[[path]]\nx = 1\ny = 2\nbogus = 3\n"),
    ] {
        assert_ne!(bad, t, "the perturbation changed nothing");
        let e = read_ds1("a.ds1.toml", &bad).unwrap_err();
        assert!(e.to_string().starts_with("a.ds1.toml: "), "{e}");
    }
    let mut bad = d.clone();
    bad.shadow.pop();
    assert!(write_ds1("a", &bad).is_err());
    let mut bad = d;
    bad.orientations.pop();
    assert!(write_ds1("a", &bad).is_err());
}
