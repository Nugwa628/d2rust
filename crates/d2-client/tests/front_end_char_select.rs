//! Character select on synthetic save folders, no game files.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use d2_client::ui::front_end::screens::char_select::*;
use d2_client::ui::front_end::*;
use d2_client::ui::geom::Point;
use d2_formats::d2s::{checksum, status};

fn folder(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("d2rust-cs-{}-{tag}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

/// A synthetic save header; `age` = seconds before a fixed epoch.
fn save(dir: &std::path::Path, file: &str, class: u8, level: u8, st: u16, age: u64) {
    let mut b = vec![0u8; 0x14F];
    b[0..4].copy_from_slice(&0xAA55_AA55u32.to_le_bytes());
    b[4..8].copy_from_slice(&0x60u32.to_le_bytes());
    b[0x24..0x26].copy_from_slice(&st.to_le_bytes());
    b[0x28] = class;
    b[0x2B] = level;
    b[0x88] = 1;
    let p = dir.join(file);
    fs::write(&p, &b).unwrap();
    let t = SystemTime::UNIX_EPOCH + Duration::from_secs(2_000_000_000 - age);
    fs::File::options()
        .write(true)
        .open(&p)
        .unwrap()
        .set_modified(t)
        .unwrap();
}

fn names(m: &Model) -> Vec<&str> {
    m.entries.iter().map(|e| e.name.as_str()).collect()
}

#[test]
fn filter_sort_and_spec_vector() {
    let d = folder("vec");
    save(&d, "Bob.d2s", 4, 30, 0x20, 0);
    save(&d, "Al.d2s", 0, 5, 0, 100);
    save(&d, "_x.d2s", 0, 5, 0, 1);
    save(&d, "A.d2s", 0, 5, 0, 2);
    save(&d, "toolongname1234567.d2s", 0, 5, 0, 3);
    save(&d, "Ann-.d2s", 0, 5, 0, 4);
    fs::write(d.join("junk.d2s"), b"short").unwrap();
    let m = Model::new(d.clone(), true);
    assert_eq!(names(&m), ["Bob", "Al"]);
    assert_eq!(m.sel, 0);
    // Classic front end: scan (name) order, not time.
    assert_eq!(names(&Model::new(d, false)), ["Al", "Bob"]);
}

#[test]
fn zero_and_one_save_button_states() {
    let m = Model::new(folder("zero"), true);
    assert!(m.entries.is_empty());
    assert_eq!(m.sel, -1);
    assert_eq!(
        m.buttons(),
        Buttons {
            ok: false,
            delete: false,
            convert: false,
            create: true
        }
    );
    let d = folder("one");
    save(&d, "Solo.d2s", 1, 9, 0, 0);
    let m = Model::new(d.clone(), true);
    assert_eq!(m.sel, 0);
    assert_eq!(
        m.buttons(),
        Buttons {
            ok: true,
            delete: true,
            convert: true,
            create: true
        }
    );
    // An expansion character cannot be converted again.
    save(&d, "Solo.d2s", 1, 9, 0x20, 0);
    assert!(!Model::new(d, true).buttons().convert);
}

#[test]
fn nine_and_twenty_saves_scroll() {
    let d = folder("nine");
    for i in 0..9u64 {
        save(&d, &format!("Hero{i}.d2s"), 0, 1, 0, i);
    }
    let mut m = Model::new(d, true);
    assert_eq!(m.entries.len(), 9);
    assert_eq!(m.scroll_range(), Some(1));
    assert_eq!(m.slot(7).unwrap().name, "Hero7");
    assert!(m.slot(8).is_none());
    m.key(40); // down
    assert_eq!(m.sel, 2);
    m.key(35); // end
    assert_eq!((m.sel, m.first), (8, 1));
    assert_eq!(m.slot(0).unwrap().name, "Hero1");

    let d = folder("twenty");
    for i in 0..20u64 {
        save(&d, &format!("Hero{i:02}.d2s"), 0, 1, 0, i);
    }
    let mut m = Model::new(d, true);
    assert_eq!(m.scroll_range(), Some(6));
    m.key(35);
    assert_eq!((m.sel, m.first), (19, 12));
    m.key(36); // home
    assert_eq!((m.sel, m.first), (0, 0));
}

#[test]
fn spec_scroll_vectors_n11() {
    let d = folder("n11");
    for i in 0..11u64 {
        save(&d, &format!("Hero{i:02}.d2s"), 0, 1, 0, i);
    }
    let mut m = Model::new(d, true);
    m.sel = 7;
    m.key(40);
    assert_eq!((m.sel, m.first), (9, 2));
    m.key(35);
    assert_eq!((m.sel, m.first), (10, 3));
    // Left/Right inside a row; Up above first scrolls.
    m.sel = 9;
    m.key(37);
    assert_eq!(m.sel, 8);
    m.key(39);
    assert_eq!(m.sel, 9);
}

#[test]
fn click_selects_and_double_click_runs_ok() {
    let d = folder("click");
    for i in 0..3u64 {
        save(&d, &format!("Hero{i}.d2s"), 0, 1, 0, i);
    }
    let mut m = Model::new(d, true);
    assert!(!m.click(1, 1000));
    assert_eq!(m.sel, 1);
    assert!(m.click(1, 1400));
    assert!(!m.click(2, 1500));
    assert!(!m.click(2, 2100)); // outside 500 ms
                                // Slot 3 is the empty slot (k = n): ignored by the game.
    m.click(3, 3000);
    assert_eq!(m.sel, 2);
    assert!(m.buttons().ok);
    // Beyond n: ignored.
    m.click(4, 3100);
    assert_eq!(m.sel, 2);
}

#[test]
fn dead_hardcore_message_and_doll() {
    let d = folder("dead");
    save(&d, "Gone.d2s", 1, 40, 0x0C, 0);
    let mut m = Model::new(d, true);
    assert_eq!(m.ok(), OkResult::Message(5304));
    assert_eq!(m.popup, Popup::Message(5304));
    let e = &m.entries[0];
    assert_eq!(e.paper_doll(true), (8, 5));
    let alive = Entry {
        status: status::HARDCORE,
        ..e.clone()
    };
    assert_eq!(alive.paper_doll(true), (1, 1));
    assert_eq!(
        Entry {
            class: 4,
            ..e.clone()
        }
        .paper_doll(true),
        (9, 5)
    );
}

#[test]
fn ok_difficulty_thresholds() {
    let d = folder("diff");
    let p = |p: u16| 0x20 | (p << 8);
    save(&d, "ExpP5.d2s", 0, 1, p(5), 1);
    save(&d, "ExpP4.d2s", 0, 1, p(4), 2);
    save(&d, "ClsP8.d2s", 0, 1, 8 << 8, 3);
    save(&d, "ExpP10.d2s", 0, 1, p(10), 4);
    let m = Model::new(d, true);
    let open = |n: &str| {
        m.entries
            .iter()
            .find(|e| e.name == n)
            .unwrap()
            .difficulties_open()
    };
    assert_eq!(open("ExpP5"), 2);
    assert_eq!(open("ExpP4"), 1);
    assert_eq!(open("ClsP8"), 3);
    assert_eq!(open("ExpP10"), 3);
}

#[test]
fn renamed_file_and_dotted_name() {
    let d = folder("ren");
    save(&d, "Renamed.d2s", 2, 7, 0, 0);
    // The header's name field is ignored: the file name is the name.
    let m = Model::new(d.clone(), true);
    assert_eq!(names(&m), ["Renamed"]);
    // `a.b.d2s` reads `ab`-less stem `Ab`'s file `Ab.d2s`, which is absent.
    fs::copy(d.join("Renamed.d2s"), d.join("Ab.cd.d2s")).unwrap();
    assert_eq!(names(&Model::new(d, true)), ["Renamed"]);
}

#[test]
fn labels() {
    let d = folder("lab");
    save(&d, "Sir.d2s", 4, 12, 4 << 8, 0);
    save(&d, "Hc.d2s", 4, 12, 0x04 | (4 << 8), 1);
    save(&d, "Exp.d2s", 1, 90, 0x20, 2);
    let m = Model::new(d, true);
    let l = m.slot_lines(0).unwrap();
    assert_eq!((l.title.as_str(), l.name.as_str()), ("Sir", "Sir"));
    assert_eq!(l.name_colour, 4);
    assert_eq!(l.level, "Level 12 Barbarian");
    let l = m.slot_lines(1).unwrap();
    assert_eq!(
        (l.title.as_str(), l.name.as_str(), l.name_colour),
        ("Count", "Hc", 1)
    );
    let l = m.slot_lines(2).unwrap();
    assert_eq!(l.expansion, Some(22731));
}

fn front_end(dir: &Path, expansion: bool) -> (FrontEnd, SelectionHandle) {
    let handle = SelectionHandle::default();
    let mut reg = Registry::default();
    screens::register_all(&mut reg);
    register_with(&mut reg, Some(dir.to_path_buf()), handle.clone());
    let mut f = FrontEnd::new(
        expansion,
        Box::new(DirSaves {
            dir: dir.to_path_buf(),
            expansion,
        }),
        reg,
    );
    f.start(false);
    f.trigger(Trigger::SinglePlayer);
    (f, handle)
}

fn click(f: &mut FrontEnd, x: i32, y: i32) {
    let p = Point::new(x, y);
    f.input(FrontInput::Down(p));
    f.input(FrontInput::Up(p));
    f.tick();
}

#[test]
fn screen_flow_exit_create_ok() {
    let d = folder("flow");
    save(&d, "Low.d2s", 0, 5, 0x20 | (4 << 8), 0);
    let (mut f, handle) = front_end(&d, true);
    assert_eq!(f.current(), CHAR_SELECT);
    f.input(FrontInput::Key(27));
    f.tick();
    assert_eq!(f.current(), MAIN_MENU);

    let (mut f, _) = front_end(&d, true);
    click(&mut f, 100, 500); // CREATE NEW (33..201, y 468..528)
    assert_eq!(f.current(), CHAR_CREATE);

    let (mut f, handle2) = front_end(&d, true);
    f.input(FrontInput::Key(13));
    f.tick();
    assert!(matches!(f.outcome(), Some(Outcome::GameLoad(_))));
    assert_eq!(handle2.lock().unwrap().as_ref().unwrap().entry.name, "Low");
    assert!(handle.lock().unwrap().is_none());
}

#[test]
fn screen_ok_opens_difficulty_box() {
    let d = folder("nm");
    save(&d, "Hi.d2s", 0, 60, 0x20 | (5 << 8), 0);
    let (mut f, handle) = front_end(&d, true);
    f.input(FrontInput::Key(13));
    f.tick();
    // The box is a pop-up of the character select (`ui-frontend-charsel-ok`).
    assert_eq!(f.current(), CHAR_SELECT);
    assert!(f.controls().iter().any(|c| c.string_id == 10019));
    assert_eq!(
        handle.lock().unwrap().as_ref().unwrap().difficulties_open,
        2
    );
    f.input(FrontInput::Key(27)); // Esc closes it
    f.tick();
    assert!(!f.controls().iter().any(|c| c.string_id == 10019));
}

#[test]
fn screen_double_click_plays_and_delete_confirm() {
    let d = folder("del");
    save(&d, "Keep.d2s", 0, 5, 0x20, 0);
    save(&d, "Drop.d2s", 0, 5, 0x20, 10);
    fs::write(d.join("Drop.key"), b"k").unwrap();
    fs::write(d.join("Drop.map"), b"m").unwrap();
    let (mut f, _) = front_end(&d, true);
    // Select Drop (slot 1, text at x 309), then Delete, then NO.
    click(&mut f, 400, 100);
    click(&mut f, 500, 500); // DELETE (433..601, y 468..528)
    assert!(f.controls().iter().any(|c| c.string_id == 5163));
    f.input(FrontInput::Key(27)); // NO
    f.tick();
    assert!(!f.controls().iter().any(|c| c.string_id == 5163));
    assert!(d.join("Drop.d2s").exists());
    // Delete again, YES (421..517, y 305..337).
    click(&mut f, 500, 500);
    click(&mut f, 450, 320);
    assert!(!d.join("Drop.d2s").exists());
    assert!(!d.join("Drop.key").exists());
    assert!(!d.join("Drop.map").exists());
    assert!(d.join("Keep.d2s").exists());
    assert_eq!(f.current(), CHAR_SELECT);
    // Double click on Keep (slot 0) starts the game.
    click(&mut f, 100, 100);
    click(&mut f, 100, 100);
    assert!(matches!(f.outcome(), Some(Outcome::GameLoad(_))));
}

#[test]
fn screen_convert_rewrites_checksum() {
    let d = folder("conv");
    save(&d, "Cls.d2s", 0, 5, 0, 0);
    let (mut f, _) = front_end(&d, true);
    click(&mut f, 300, 500); // CONVERT (233..401)
    assert!(f.controls().iter().any(|c| c.string_id == 22734));
    click(&mut f, 450, 320); // YES
    let b = fs::read(d.join("Cls.d2s")).unwrap();
    assert_eq!(u16::from_le_bytes([b[0x24], b[0x25]]) & 0x20, 0x20);
    let mut z = b.clone();
    z[0x0C..0x10].fill(0);
    assert_eq!(
        u32::from_le_bytes(b[0x0C..0x10].try_into().unwrap()),
        checksum(&z)
    );
    // Convert is now off.
    let conv = f.controls().iter().find(|c| c.string_id == 22732).unwrap();
    assert!(!conv.enabled);
}

#[test]
fn save_folder_probe() {
    let d = folder("probe");
    let probe = DirSaves {
        dir: d.clone(),
        expansion: true,
    };
    assert!(!probe.has_saves());
    save(&d, "Ok.d2s", 0, 1, 0, 0);
    assert!(probe.has_saves());
}
