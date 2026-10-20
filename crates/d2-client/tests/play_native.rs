// (In d2-client's tests: d2-convert stays free of Bevy, CLAUDE.md rule 5.)
//! `play --native` loads what `play` loads from the archives: the synthetic
//! install is converted by `d2-convert`, then the play app's data load
//! (tables, level files, act palettes, typed art) runs on the archives and
//! on the native folder and gives the same result.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use d2_client::app::palette::ActPalettes;
use d2_client::app::single_player::{GameData, GameSource, Layers};
use d2_client::assets::game_files::GameFiles;
use d2_client::assets::path::{read_cof, read_dc6, FileSource};
use d2_convert::{convert, kinds, Options};
use d2_formats::mpq::writer::MpqWriter;
use d2_formats::mpq::ArchiveSet;
use test_fixtures::sprites::{dc6_file, dc6_frames, Dc6Shape};
use test_fixtures::synth::synthetic;

const SHEET: &str = r"data\global\ui\panel\n4test.dc6";
const COF: &str = r"data\global\monsters\zz\cof\zztrhth.cof";
const COLORMAP: &str = r"data\global\items\Palette\grey.dat";

/// A one-layer, two-frame COF.
fn cof() -> Vec<u8> {
    let mut v = vec![1u8, 2, 2, 20, 0, 0, 0, 0];
    for x in [-10i32, 10, -20, 0] {
        v.extend_from_slice(&x.to_le_bytes());
    }
    v.extend_from_slice(&256u32.to_le_bytes());
    v.extend_from_slice(&[0, 1, 1, 0, 0]);
    v.extend_from_slice(b"hth\0");
    v.extend_from_slice(&[0, 3]); // events
    v.extend_from_slice(&[0, 0, 0, 0]); // draw order
    v
}

/// A walk-back field: 256 × 256 cells, each the
/// direction one step toward the centre (128, 128), 8 at the centre. The
/// play app's data load reads it for the floor drop.
fn expfield() -> Vec<u8> {
    let mut v = vec![0x0A, 0x01];
    v.extend_from_slice(&256u32.to_le_bytes());
    v.extend_from_slice(&256u32.to_le_bytes());
    for y in 0..256i32 {
        for x in 0..256i32 {
            // 0 (0,-1), 1 (1,-1), 2 (1,0), 3 (1,1), 4 (0,1), 5 (-1,1), 6 (-1,0), 7 (-1,-1)
            v.push(match ((128 - x).signum(), (128 - y).signum()) {
                (0, -1) => 0,
                (1, -1) => 1,
                (1, 0) => 2,
                (1, 1) => 3,
                (0, 1) => 4,
                (-1, 1) => 5,
                (-1, 0) => 6,
                (-1, -1) => 7,
                _ => 8,
            });
        }
    }
    v
}

fn scratch(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

/// A PL2 whose base palette's unused fourth byte per color is 0 (what
/// `Pl2::to_bytes` writes), the rest a pattern.
fn pl2(seed: usize) -> Vec<u8> {
    let mut v: Vec<u8> = (0..1024 + 1714 * 256)
        .map(|i| (i * seed % 251) as u8)
        .collect();
    for c in v[..1024].chunks_mut(4) {
        c[3] = 0;
    }
    v
}

#[test]
fn play_data_on_native_equals_play_data_on_the_archives() {
    let dir = scratch("play-native");
    let mut data = synthetic();
    // The fix-up of monstats record 707 reads monmode 15.
    let m = data.tables.files.get_mut("monmode.txt").unwrap();
    let id = m.columns.iter().position(|c| c == "name").unwrap();
    let mut row = m.rows[0].clone();
    while m.rows.len() < 16 {
        row[id] = format!("pad{}", m.rows.len());
        m.rows.push(row.clone());
    }
    // The hireling table reads pettype row 7.
    let m = data.tables.files.get_mut("pettype.txt").unwrap();
    let id = m.columns.iter().position(|c| c == "pet type").unwrap();
    let mut row = m.rows[0].clone();
    while m.rows.len() < 8 {
        row[id] = format!("pad{}", m.rows.len());
        m.rows.push(row.clone());
    }
    // `WaypointTables::live` needs a waypoint object (operatefn 23, initfn 17).
    let o = data.tables.files.get_mut("objects.txt").unwrap();
    for (col, v) in [("OperateFn", "23"), ("InitFn", "17")] {
        let c = o.columns.iter().position(|x| x == col).unwrap();
        o.rows[0][c] = v.into();
    }
    // The converter's C-TABLE check expects the live monstats' 708 records.
    let f = data.tables.files.get_mut("monstats.txt").unwrap();
    let id = f.columns.iter().position(|c| c == "Id").unwrap();
    let mut row = f.rows[0].clone();
    while f.rows.len() < 708 {
        row[id] = format!("pad{}", f.rows.len());
        f.rows.push(row.clone());
    }
    let shape = Dc6Shape {
        directions: 1,
        frames: 2,
        width: 12,
        height: 9,
        panel: false,
    };
    data.files
        .push((SHEET.into(), dc6_file(&dc6_frames(shape, 7), 1, 2)));
    data.files.push((COF.into(), cof()));
    // An item colour map: 21 rows of 256 indices.
    let colormap: Vec<u8> = (0..21 * 256).map(|i| (i * 7 % 256) as u8).collect();
    data.files.push((COLORMAP.into(), colormap.clone()));
    data.files
        .push((r"data\global\ExpField.D2".into(), expfield()));
    for act in 1..=5 {
        data.files.push((
            format!(r"data\global\palette\act{act}\pal.pl2"),
            pl2(act + 2),
        ));
    }
    test_fixtures::cache::copy_install(
        std::path::Path::new(env!("CARGO_TARGET_TMPDIR")),
        &dir.join("install"),
        &data,
    )
    .unwrap();
    // The converter wants the whole install's archive names.
    for name in d2_convert::convert::REQUIRED_ARCHIVES {
        let path = dir.join("install").join(name);
        if !path.exists() {
            MpqWriter::new().write(path).unwrap();
        }
    }

    let out = dir.join("native");
    let mut o = Options::new(dir.join("install"), &out);
    o.converter_commit = "test".into();
    let summary = convert(&o, &kinds::builtin()).unwrap();
    assert_eq!(summary.failures, Vec::new());

    // The play app's own data load, both ways.
    let GameData::Live(mpq) = GameData::select(Some(&dir.join("install"))).unwrap();
    let GameData::Live(nat) = GameData::select_native(&out).unwrap();
    assert!(!mpq.archives.is_native() && nat.archives.is_native());
    assert_eq!(
        format!("{:?}", mpq.waypoints),
        format!("{:?}", nat.waypoints)
    );
    assert!(!nat.files.ds1.0.is_empty() && !nat.files.dt1.0.is_empty());
    assert_eq!(format!("{:?}", mpq.files), format!("{:?}", nat.files));
    assert_eq!(*mpq.expfield, *nat.expfield);
    assert_eq!(mpq.expfield.byte(128, 128), Some(8));
    // The fixed-up tables: same names, counts and record bytes (the
    // origin label differs by design).
    let cells = |d: &d2_client::app::single_player::LiveData| {
        d.tables
            .fixed
            .tables
            .iter()
            .map(|t| (t.name.clone(), t.count, t.record_size, t.records.clone()))
            .collect::<Vec<_>>()
    };
    assert!(!cells(&nat).is_empty());
    assert_eq!(cells(&mpq), cells(&nat));

    // The act palettes (the first frame's colors) and a typed art file.
    let a = ActPalettes::live(mpq.archives.as_ref()).unwrap();
    let b = ActPalettes::live(nat.archives.as_ref()).unwrap();
    assert_eq!(a.pl2, b.pl2);
    let sheet = |f: &GameFiles| read_dc6(f, SHEET).unwrap().unwrap();
    assert_eq!(sheet(&mpq.archives), sheet(&nat.archives));
    // A COF read raw on native is only the presence mark; the typed read
    // is the decoded file (the unit art's COF load).
    let cof = |f: &GameFiles| read_cof(f, COF).unwrap().unwrap();
    assert_eq!(cof(&mpq.archives), cof(&nat.archives));
    assert_eq!(nat.archives.read_file(COF), Some(Ok(Vec::new())));

    // Colour maps are copied verbatim: the raw read is the input bytes.
    assert_eq!(mpq.archives.read_file(COLORMAP), Some(Ok(colormap.clone())));
    assert_eq!(nat.archives.read_file(COLORMAP), Some(Ok(colormap)));

    // No sound on native: `.wav` reads are absent, not an error.
    let _: Arc<dyn FileSource> = nat.archives.source();
    assert!(nat.archives.read_file(r"data\global\sfx\x.wav").is_none());

    // A game's own layer list (a mod seam): the order
    // file names a mod that is not there, so opening with it fails, while
    // a game that stacks `base/` alone never reads it.
    std::fs::create_dir_all(out.join("mods")).unwrap();
    std::fs::write(out.join("mods/order.toml"), "order = [\"absent\"]\n").unwrap();
    let e = GameData::select_native(&out).unwrap_err().to_string();
    assert!(e.contains("absent"), "{e}");
    let source = GameSource::Native(out.clone(), Layers::base());
    let GameData::Live(base) = GameData::open(&source).unwrap();
    assert_eq!(
        format!("{:?}", base.waypoints),
        format!("{:?}", nat.waypoints)
    );
}

#[allow(dead_code)]
fn _archive_set_is_a_source(set: &ArchiveSet) -> &dyn FileSource {
    set
}
