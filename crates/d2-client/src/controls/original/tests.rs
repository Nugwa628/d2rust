//! Tests for the classic key-binding scheme (`super`): the default table against the repo's
//! key-command table, the command rows, the `.key` file load order, keyboard / mouse / wheel
//! dispatch, key assignment, skill hotkeys, belt use, the key-config menu tables and the
//! pointer-button meanings.

use super::*;

const TSV: &str = include_str!("../../../../../tables/ui/key-commands.tsv");

fn key_of_name(s: &str) -> u16 {
    if s == "-" {
        UNBOUND
    } else {
        u16::from_str_radix(s.split(':').next().unwrap().trim_start_matches("0x"), 16).unwrap()
    }
}

fn addr(s: &str) -> u32 {
    if s == "-" {
        0
    } else {
        u32::from_str_radix(s.trim_start_matches("0x"), 16).unwrap()
    }
}

/// The 1140-byte table built from the TSV: entries in `file_pos` order, slot 1 then slot 0,
/// except command 1 (slot 0 first).
fn table_from_tsv() -> Vec<u8> {
    let mut entries = vec![(0i32, 0u16, 0i32); TABLE_LEN];
    for row in TSV.lines().skip(1) {
        let c: Vec<&str> = row.split('\t').collect();
        let cmd: i32 = c[0].parse().unwrap();
        let p: usize = c[8].parse().unwrap();
        let one = (cmd, key_of_name(c[3]), 1);
        let zero = (cmd, key_of_name(c[4]), 0);
        // Command 1 (inventory) lists its slot-0 entry first.
        let (first, second) = if cmd == 1 { (zero, one) } else { (one, zero) };
        entries[2 * p] = first;
        entries[2 * p + 1] = second;
    }
    let mut out = Vec::new();
    for (cmd, key, slot) in entries {
        out.extend(cmd.to_le_bytes());
        out.extend(key.to_le_bytes());
        out.extend(slot.to_le_bytes());
    }
    out
}

#[test]
fn command_table_matches_key_commands_tsv() {
    let rows: Vec<&str> = TSV.lines().skip(1).collect();
    assert_eq!(rows.len(), COMMAND_COUNT);
    for (row, c) in rows.iter().zip(COMMANDS.iter()) {
        let f: Vec<&str> = row.split('\t').collect();
        assert_eq!(f[0].parse::<u8>().unwrap(), c.cmd);
        assert_eq!(
            if f[1] == "-" {
                0
            } else {
                f[1].parse().unwrap()
            },
            c.string_id
        );
        assert_eq!(key_of_name(f[3]), c.key1, "cmd {}", c.cmd);
        assert_eq!(key_of_name(f[4]), c.key2, "cmd {}", c.cmd);
        assert_eq!(addr(f[5]), c.down);
        assert_eq!(addr(f[6]), c.up);
        assert_eq!(f[7] == "1", c.full_ok);
        assert_eq!(f[8].parse::<u8>().unwrap(), c.file_pos);
        let m = |s: &str| {
            if s == "-" {
                -1
            } else {
                s.parse::<i16>().unwrap()
            }
        };
        assert_eq!(m(f[9]), c.menu_classic);
        assert_eq!(m(f[10]), c.menu_exp);
    }
    let bytes = table_from_tsv();
    assert_eq!(bytes.len(), 1140);
    assert_eq!(BindingTable::defaults().to_bytes(), bytes);
    assert!(BindingTable::defaults().is_valid());
    // command 21 (F8) sits at entries 90-91, after command 45
    let d = BindingTable::defaults();
    assert_eq!((d.0[90].cmd, d.0[90].key, d.0[91].cmd), (21, 0x77, 21));
    assert_eq!(d.0[86].cmd, 44);
}

#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn defaults_equal_game_exe_table() {
    let dir = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR");
    let exe = std::fs::read(std::path::Path::new(&dir).join("Game.exe")).unwrap();
    assert_eq!(
        &exe[0x312220..0x312220 + TABLE_BYTES],
        &table_from_tsv()[..]
    );
}

// The command rows: one check per row of the default table (key, handler, effect class).
#[test]
fn default_keys_per_row() {
    let k = |cmd: usize| (COMMANDS[cmd].key1, COMMANDS[cmd].key2);
    let none = UNBOUND;
    // row 1-3: A/C, I/B, P
    assert_eq!(k(0), (0x41, 0x43));
    assert_eq!(k(1), (0x49, 0x42));
    assert_eq!(k(2), (0x50, none));
    // 4-7: M, Q, Enter, H
    assert_eq!(k(3), (0x4D, none));
    assert_eq!(k(4), (0x51, none));
    assert_eq!(k(5), (0x0D, none));
    assert_eq!(k(6), (0x48, none));
    // 8-12: Tab/middle, F9-F12
    assert_eq!(k(7), (0x09, MIDDLE));
    assert_eq!(k(8), (0x78, none));
    assert_eq!(k(9), (0x79, none));
    assert_eq!(k(10), (0x7A, none));
    assert_eq!(k(11), (0x7B, none));
    // 13-14: T, S
    assert_eq!(k(12), (0x54, none));
    assert_eq!(k(13), (0x53, none));
    // 15: F1-F8
    for i in 0..8 {
        assert_eq!(k(14 + i), (0x70 + i as u16, none));
    }
    // 16: backquote
    assert_eq!(k(22), (0xC0, none));
    // 17: belt 1-4
    for i in 0..4 {
        assert_eq!(k(23 + i), (0x31 + i as u16, none));
    }
    // 18: say 0-6 on numpad
    for i in 0..7 {
        assert_eq!(k(27 + i), (0x60 + i as u16, none));
    }
    // 19-22: run Ctrl, run lock R/X2, stand still Shift, show items Alt/X1
    assert_eq!(k(34), (0x11, none));
    assert_eq!(k(35), (0x52, X2));
    assert_eq!(k(36), (0x10, none));
    assert_eq!(k(37), (0x12, X1));
    // 23-: space, wheel, N, PrintScreen, Z, W, V
    assert_eq!(k(38), (0x20, none));
    assert_eq!(k(39), (WHEEL_UP, none));
    assert_eq!(k(40), (WHEEL_DOWN, none));
    assert_eq!(k(41), (0x4E, none));
    assert_eq!(k(42), (0x2C, none));
    assert_eq!(k(43), (0x5A, none));
    assert_eq!(k(44), (0x57, none));
    assert_eq!(k(45), (0x56, none));
    // skills 9-16 have no default key; O, numpad 7, Esc
    for c in 46..=53 {
        assert_eq!(k(c), (none, none));
    }
    assert_eq!(k(54), (0x4F, none));
    assert_eq!(k(55), (0x67, none));
    assert_eq!(k(56), (0x1B, none));
    // handlers: Run has both, Print Screen only an up, wheel only down
    assert!(COMMANDS[34].down != 0 && COMMANDS[34].up != 0);
    assert!(COMMANDS[42].down == 0 && COMMANDS[42].up != 0);
    assert!(COMMANDS[36].down != 0 && COMMANDS[36].up != 0);
    assert!(COMMANDS[37].down != 0 && COMMANDS[37].up != 0);
    assert!(COMMANDS[0].up == 0);
    // full_ok flag: chat, belt show, say, cleartext, snapshot, swap, minimap
    let m2: Vec<u8> = COMMANDS
        .iter()
        .filter(|c| c.full_ok)
        .map(|c| c.cmd)
        .collect();
    assert_eq!(m2, [5, 22, 27, 28, 29, 30, 31, 32, 33, 41, 42, 44, 45, 55]);
}

#[test]
fn binding_table_and_lookups() {
    let mut t = BindingTable::defaults();
    assert_eq!(t.0.len(), 114);
    assert_eq!(t.to_bytes().len(), 0x474);
    // 10-byte entry: i32 cmd, u16 key, i32 slot
    let e = &t.to_bytes()[..10];
    assert_eq!(e, [0, 0, 0, 0, 0x41, 0, 1, 0, 0, 0]);
    assert_eq!(t.key_of(0, 1), 0x41);
    assert_eq!(t.key_of(0, 0), 0x43);
    assert_eq!(t.key_of(99, 0), UNBOUND);
    assert!(t.is_bound(7, 0));
    assert!(!t.is_bound(2, 0));
    // key values >= 0x100 are mouse inputs
    assert_eq!(t.key_of(7, 0), MIDDLE);
    assert_eq!(t.key_of(39, 1), WHEEL_UP);
    // unbind clears every matching entry
    t.0.push(Binding {
        cmd: 3,
        key: 0x99,
        slot: 1,
    });
    t.unbind(3, 1);
    assert_eq!(t.key_of(3, 1), UNBOUND);
    assert_eq!(t.0.last().unwrap().key, UNBOUND);
    // the command table: 57 records; the flag lets it run in key mode 2
    assert_eq!(COMMANDS.len(), 57);
    assert!(COMMANDS
        .iter()
        .enumerate()
        .all(|(i, c)| usize::from(c.cmd) == i));
    // a snapshot restores (the key-config screen cancel)
    let snap = t.clone();
    t.unbind(0, 1);
    assert_ne!(t, snap);
    t = snap.clone();
    assert_eq!(t, snap);
}

#[test]
fn key_files() {
    let d = BindingTable::defaults();
    let ch = char_file_bytes(&d);
    assert_eq!(ch.len(), 0x476);
    assert_eq!(&ch[..2], [0x25, 0]);
    let df = default_file_bytes(&d);
    assert_eq!(df.len(), 0x47A);
    assert_eq!(&df[..6], [0x57, 0x53, 0x25, 0, 0x7A, 0x04]);
    // accepted character file: loaded, nothing written
    let mut custom = d.clone();
    custom.0[0].key = 0x5A; // Z conflicts with cmd 43's Z -> invalid
    assert!(!custom.is_valid());
    custom.0[0].key = UNBOUND;
    assert!(custom.is_valid());
    let r = load_at_start(Some(&char_file_bytes(&custom)), None, None);
    assert_eq!((r.table.clone(), r.write_files), (custom.clone(), false));
    // wrong size / version / a missing command → defaults, files written
    let old_version = [&[0x24u8, 0][..], &ch[2..]].concat();
    for bad in [&ch[..ch.len() - 1], &old_version[..]] {
        let r = load_at_start(Some(bad), None, None);
        assert_eq!((r.table, r.write_files), (d.clone(), true));
    }
    let mut missing = d.clone();
    for b in &mut missing.0 {
        if b.cmd == 5 {
            b.cmd = 6;
        }
    }
    assert!(!missing.is_valid());
    assert_eq!(
        load_at_start(Some(&char_file_bytes(&missing)), None, None).table,
        d
    );
    // the save-directory default.key wins over the archive's
    let mut other = d.clone();
    other.0[0].key = 0x5B;
    let save = default_file_bytes(&other);
    let r = load_at_start(None, Some(&save), Some(&df));
    assert_eq!((r.table, r.write_files), (other.clone(), true));
    // a save-dir file of the right size but a bad header is dropped, and
    // the archive is not tried
    let mut bad = save.clone();
    bad[0] = 0;
    assert_eq!(load_at_start(None, Some(&bad), Some(&save)).table, d);
    // wrong-size save file: the archive is used
    assert_eq!(
        load_at_start(None, Some(&save[..10]), Some(&save)).table,
        other
    );
    // An archive `default.key` with an older version (0x22 / 0x24) is dropped.
    let mut old = df.clone();
    old[2] = 0x24;
    assert_eq!(load_at_start(None, None, Some(&old)).table, d);
    assert_eq!(parse_default_file(&df), Some(d.clone()));
    assert_eq!(parse_char_file(&ch), Some(d));
}

#[test]
fn key_config_menu_tables() {
    let classic = menu_table(false);
    let exp = menu_table(true);
    assert_eq!((classic.len(), exp.len()), (51, 62));
    for s in [6, 19, 25, 32, 38, 47] {
        assert_eq!(classic[s].cmd, SEPARATOR);
    }
    for s in [7, 28, 35, 42, 49, 58] {
        assert_eq!(exp[s].cmd, SEPARATOR);
    }
    assert_eq!(classic.iter().filter(|r| r.cmd != SEPARATOR).count(), 45);
    assert_eq!(exp.iter().filter(|r| r.cmd != SEPARATOR).count(), 56);
    // commands 44-54 only in the expansion table; 56 in neither
    assert!(classic
        .iter()
        .all(|r| !(44..=56).contains(&r.cmd) || r.cmd == 55));
    assert!(exp.iter().all(|r| r.cmd != 56));
    assert!(exp.iter().any(|r| r.cmd == 44) && exp.iter().any(|r| r.cmd == 54));
    // string ids: patchstring 10833 CfgSkillPick, 11083 CfgSay7X
    assert!(exp.iter().any(|r| r.string_id == 10833));
    assert!(exp.iter().any(|r| r.string_id == 11083));
    assert_eq!(
        exp[0],
        MenuRow {
            cmd: 0,
            string_id: 3924
        }
    );
}

#[test]
fn keyboard_dispatch() {
    let t = BindingTable::defaults();
    // key down: first entry with a down handler; auto-repeat ignored
    assert_eq!(key_down_command(&t, 0x41, false, false, 1), Some(0));
    assert_eq!(key_down_command(&t, 0x41, true, false, 1), None);
    // F4 with Alt: nothing
    assert_eq!(key_down_command(&t, 0x73, false, false, 1), Some(17));
    assert_eq!(key_down_command(&t, 0x73, false, true, 1), None);
    // Print Screen has no down handler; its up handler runs
    assert_eq!(key_down_command(&t, 0x2C, false, false, 1), None);
    assert_eq!(key_up_command(&t, 0x2C, 1), Some(42));
    // up: the repeat bit is not tested; Ctrl (run) has an up handler
    assert_eq!(key_up_command(&t, 0x11, 1), Some(34));
    assert_eq!(key_up_command(&t, 0x41, 1), None);
    // key mode 2: only the full_ok commands (both directions)
    assert_eq!(key_down_command(&t, 0x41, false, false, 2), None);
    assert_eq!(key_down_command(&t, 0x0D, false, false, 2), Some(5));
    assert_eq!(key_up_command(&t, 0x11, 2), None);
    assert_eq!(key_up_command(&t, 0x2C, 2), Some(42));
    // a first entry without a handler is skipped, the next one taken
    let mut t2 = t.clone();
    t2.0.insert(
        0,
        Binding {
            cmd: 42,
            key: 0x41,
            slot: 1,
        },
    );
    assert_eq!(key_down_command(&t2, 0x41, false, false, 1), Some(0));
    // key mode changes
    let none = |_: u8| false;
    use KeyModeEvent::*;
    assert_eq!(key_mode_for(GameStart, &none), Some((1, false)));
    assert_eq!(key_mode_for(GameEnd, &none), Some((0, false)));
    assert_eq!(key_mode_for(UiOpen(5), &none), Some((0, true)));
    assert_eq!(key_mode_for(UiOpen(23), &none), Some((0, true)));
    for u in [12, 25, 26] {
        assert_eq!(key_mode_for(UiOpen(u), &none), Some((2, false)));
    }
    for u in [12, 23, 25, 26] {
        assert_eq!(key_mode_for(UiClose(u), &none), Some((1, false)));
    }
    assert_eq!(key_mode_for(UiClose(5), &none), Some((1, false)));
    assert_eq!(key_mode_for(UiClose(5), &|u| u == 30), None);
    assert_eq!(key_mode_for(UiClose(5), &|u| u == 31), Some((1, false)));
    assert_eq!(key_mode_for(KeyConfigOpen, &none), Some((0, false)));
    assert_eq!(key_mode_for(LatchSet, &none), Some((0, true)));
    assert_eq!(key_mode_for(LatchClear, &none), Some((1, false)));
    assert_eq!(key_mode_for(UiOpen(1), &none), None);
    // Windows keys and the system commands are swallowed
    assert!(swallow_vk(0x5B) && swallow_vk(0x5C) && swallow_vk(0x5D));
    assert!(!swallow_vk(0x5A));
    assert!(swallow_syscommand(0xF100, false));
    assert!(swallow_syscommand(0xF140, false));
    assert!(!swallow_syscommand(0xF010, false));
    assert!(swallow_syscommand(0xF010, true));
}

#[test]
fn mouse_buttons_and_wheel() {
    let mut t = BindingTable::defaults();
    let s = scan_mouse_slots(&mut t);
    // middle = automap (cmd 7, down only); X1 show items (down + up); X2 run lock
    assert_eq!((s.middle_down, s.middle_up), (Some(7), None));
    assert_eq!((s.x1_down, s.x1_up), (Some(37), Some(37)));
    assert_eq!((s.x2_down, s.x2_up), (Some(35), None));
    assert_eq!((s.wheel_up, s.wheel_down), (Some(39), Some(40)));
    // a wheel binding to a command with an up handler is removed
    t.0[0] = Binding {
        cmd: 34,
        key: WHEEL_UP,
        slot: 1,
    };
    let s = scan_mouse_slots(&mut t);
    assert_eq!(t.0[0].key, UNBOUND);
    assert_eq!(s.wheel_up, Some(39));
    // several entries on one button: the last wins
    t.0.push(Binding {
        cmd: 3,
        key: MIDDLE,
        slot: 1,
    });
    assert_eq!(scan_mouse_slots(&mut t).middle_down, Some(3));
    // X button events: high word 1 / 2
    assert_eq!(x_button_key(1 << 16), Some(X1));
    assert_eq!(x_button_key(2 << 16 | 5), Some(X2));
    assert_eq!(x_button_key(3 << 16), None);
    // the wheel accumulator: > 0x77 fires once, whatever the delta
    let mut w = WheelAccumulator::default();
    assert_eq!(w.event(120, true), Some(WHEEL_UP));
    assert_eq!(w, WheelAccumulator(0));
    assert_eq!(w.event(-50, true), None);
    assert_eq!(w.event(-50, true), None);
    assert_eq!(w.event(-50, true), Some(WHEEL_DOWN));
    assert_eq!(w.event(-1200, true), Some(WHEEL_DOWN));
    assert_eq!(w.event(0x77, true), None);
    assert_eq!(w.event(500, false), None);
}

#[test]
fn modifiers() {
    assert_eq!(world_action_flags(false, false, false), 0);
    assert_eq!(world_action_flags(true, false, false), 8);
    // no inversion: Run with the lock on still runs
    assert_eq!(world_action_flags(true, true, false), 8);
    assert_eq!(world_action_flags(false, true, true), 12);
    assert_eq!(world_action_flags(false, false, true), 4);
    let t = BindingTable::defaults();
    // Stand Still re-sampled from command 36's keys
    assert!(stand_still_sample(&t, &|k| k == 0x10));
    assert!(!stand_still_sample(&t, &|_| false));
    // deactivation clears Run when its key is bound and not down
    assert!(deactivate_clears_run(&t, &|_| false));
    assert!(!deactivate_clears_run(&t, &|k| k == 0x11));
    let mut t2 = t.clone();
    t2.unbind(34, 1);
    assert!(!deactivate_clears_run(&t2, &|_| false));
}

#[test]
fn assigning_keys() {
    let mut t = BindingTable::defaults();
    // Esc (56) cannot be reassigned
    assert_eq!(
        assign_key(&mut t, 56, 1, 0x41),
        Err(AssignError::CantAssignKey)
    );
    // refused keys
    for k in [
        1u16, 4, 0x15, 0x17, 0x1B, 0x1F, 0x25, 0x29, 0x2B, 0x5B, 0x5D, 0x90,
    ] {
        assert!(!key_allowed(k), "{k:#x}");
        assert_eq!(assign_key(&mut t, 0, 1, k), Err(AssignError::CantAssignKey));
    }
    for k in [
        8u16, 9, 0xC, 0xD, 0x10, 0x14, 0x20, 0x24, 0x2A, 0x2C, 0x2F, 0x30, 0x39, 0x41, 0x5A, 0x60,
        0x87, 0x91, 0xBA, 0xC0, 0xDB, 0xDE, 0x100, 0x104,
    ] {
        assert!(key_allowed(k), "{k:#x}");
    }
    assert_eq!(AssignError::CantAssignKey.string_id(), 3979);
    assert_eq!(AssignError::CantAssignMw.string_id(), 3978);
    // wheel on a command with an up handler: 3978
    assert_eq!(
        assign_key(&mut t, 34, 1, WHEEL_UP),
        Err(AssignError::CantAssignMw)
    );
    // Print Screen on a command with a down handler: 3979
    assert_eq!(
        assign_key(&mut t, 0, 1, 0x2C),
        Err(AssignError::CantAssignKey)
    );
    // ... but fine on one without (Print Screen itself)
    assert_eq!(assign_key(&mut t, 42, 0, 0x2C), Ok(()));
    // the first other entry holding K is unbound; K lands on (c, s)
    assert_eq!(assign_key(&mut t, 0, 1, 0x49), Ok(())); // I was cmd 1 slot 1
    assert_eq!(t.key_of(0, 1), 0x49);
    assert_eq!(t.key_of(1, 1), UNBOUND);
    // no entry for (c, s): K is written back where it was
    let mut t = BindingTable::defaults();
    t.0.retain(|b| !(b.cmd == 0 && b.slot == 1));
    assert_eq!(assign_key(&mut t, 0, 1, 0x49), Ok(()));
    assert_eq!(t.key_of(1, 1), 0x49);
}

#[test]
fn hotkeys_and_belt_keys() {
    assert_eq!(hotkey_of_command(14), Some(0));
    assert_eq!(hotkey_of_command(21), Some(7));
    assert_eq!(hotkey_of_command(46), Some(8));
    assert_eq!(hotkey_of_command(53), Some(15));
    assert_eq!(hotkey_of_command(22), None);
    assert_eq!(hotkey_action(3, true), HotkeyAction::Assign(3));
    assert_eq!(hotkey_action(3, false), HotkeyAction::Use(3));
    let mut skill = [-1i32; 16];
    let mut send = [0u32; 16];
    let mut left = [0u8; 16];
    skill[2] = 54;
    send[2] = 0x1234;
    left[2] = 1;
    skill[5] = 40;
    send[5] = 0x55;
    let mut last = 0usize;
    // no skill, or refused: nothing and the last hotkey stays
    assert_eq!(hotkey_use(1, &skill, &send, &left, true, &mut last), None);
    assert_eq!(hotkey_use(2, &skill, &send, &left, false, &mut last), None);
    assert_eq!(last, 0);
    assert_eq!(
        hotkey_use(2, &skill, &send, &left, true, &mut last),
        Some(HotkeyUse {
            send_value: 0x1234,
            left: true
        })
    );
    assert_eq!(last, 2);
    assert_eq!(
        hotkey_use(5, &skill, &send, &left, true, &mut last),
        Some(HotkeyUse {
            send_value: 0x55,
            left: false
        })
    );
    assert_eq!(last, 5);
    // belt keys: only when the column-ready byte is 1; shift = bit 15
    assert_eq!(belt_key(0, 0), None);
    assert_eq!(belt_key(1, 0), Some(false));
    assert_eq!(belt_key(1, i16::MIN), Some(true));
    // Hotkeys 9-16 have no default key.
    for c in &COMMANDS[46..=53] {
        assert_eq!(c.key1, UNBOUND);
    }
}

#[test]
fn gates_and_belt_use() {
    assert!(gate_blocks(true, Some(1)));
    assert!(gate_blocks(false, None));
    assert!(gate_blocks(false, Some(0x11)));
    assert!(!gate_blocks(false, Some(1)));
    let f = BeltUseFacts {
        expansion: true,
        shift: true,
        cursor_item: false,
        ui9_open: false,
        item: Some(77),
        passes_quest_unique: true,
        useable: true,
        busy: false,
    };
    assert_eq!(
        belt_use(&f),
        BeltUse::Send {
            item: 77,
            shift: 0x8000
        }
    );
    // shift is 0 in a classic game
    assert_eq!(
        belt_use(&BeltUseFacts {
            expansion: false,
            ..f
        }),
        BeltUse::Send { item: 77, shift: 0 }
    );
    assert_eq!(
        belt_use(&BeltUseFacts { shift: false, ..f }),
        BeltUse::Send { item: 77, shift: 0 }
    );
    for g in [
        BeltUseFacts {
            cursor_item: true,
            ..f
        },
        BeltUseFacts {
            ui9_open: true,
            ..f
        },
        BeltUseFacts { item: None, ..f },
        BeltUseFacts { busy: true, ..f },
    ] {
        assert_eq!(belt_use(&g), BeltUse::Nothing);
    }
    assert_eq!(
        belt_use(&BeltUseFacts {
            passes_quest_unique: false,
            ..f
        }),
        BeltUse::QuestUniqueHandler
    );
    assert_eq!(
        belt_use(&BeltUseFacts {
            useable: false,
            ..f
        }),
        BeltUse::NotUseableSound
    );
    // The gold dialog latch: open gives key mode 0 with key-up kept.
    assert_eq!(
        key_mode_for(KeyModeEvent::LatchSet, &|_| false),
        Some((0, true))
    );
    assert_eq!(
        key_mode_for(KeyModeEvent::LatchClear, &|_| false),
        Some((1, false))
    );
}

#[test]
fn pointer_button_meanings() {
    let t = BindingTable::defaults();
    assert_eq!(pointer_meaning(&t, Button::Left), PointerMeaning::WorldLeft);
    assert_eq!(
        pointer_meaning(&t, Button::Right),
        PointerMeaning::WorldRight
    );
    // default middle = command 7 (automap); X1 show items; X2 run lock
    assert_eq!(
        pointer_meaning(&t, Button::Middle),
        PointerMeaning::Command(7)
    );
    assert_eq!(pointer_meaning(&t, Button::X1), PointerMeaning::Command(37));
    assert_eq!(pointer_meaning(&t, Button::X2), PointerMeaning::Command(35));
    // rebinding: the left and right buttons never change; unbinding frees one
    let mut u = t.clone();
    u.unbind(7, 0);
    assert_eq!(pointer_meaning(&u, Button::Middle), PointerMeaning::Unbound);
    assert_eq!(pointer_meaning(&u, Button::Left), PointerMeaning::WorldLeft);
    // modifiers come from commands 36 / 34 / 37
    assert_eq!(modifier_command(Modifier::StandStill), 36);
    assert_eq!(modifier_command(Modifier::Run), 34);
    assert_eq!(modifier_command(Modifier::ShowItems), 37);
    assert_eq!(COMMANDS[36].key1, 0x10);
    assert_eq!(COMMANDS[34].key1, 0x11);
    assert_eq!(COMMANDS[37].key1, 0x12);
}
