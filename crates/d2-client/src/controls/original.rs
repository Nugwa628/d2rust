//! The classic key-binding scheme (the `original` preset): the 114-entry binding table, the 57
//! numbered commands, key files, keyboard and mouse dispatch, key assignment, and the gates and
//! belt use that key commands go through.
//!
//! It covers:
//!
//! - the binding table and its lookups (command, slot and key),
//! - the character `.key` file and `default.key` formats and the load order at game start,
//! - the command table with each command's default keys, handlers and key-config menu row,
//! - keyboard and mouse dispatch, including the key modes, the wheel accumulator and the
//!   Run / Stand Still modifiers,
//! - key assignment on the key-config screen,
//! - skill hotkeys, the handler gate and belt use.
//!
//! It is plain Rust. The portable `controls` module (the `d2controls` file) is separate and does
//! not depend on it, except that [`super::keymap`] maps this scheme's command numbers and key
//! codes to the portable [`super::Action`] / [`super::Key`] names, and the `original` preset is
//! built from [`COMMANDS`]. [`super::click`] handles what the world buttons do.

/// Key 0xFFFF means unbound.
pub const UNBOUND: u16 = 0xFFFF;
/// Entries in the binding table.
pub const TABLE_LEN: usize = 114;
/// Table size in bytes: 114 x 10 (0x474).
pub const TABLE_BYTES: usize = TABLE_LEN * 10;
/// Commands are numbered 0-56.
pub const COMMAND_COUNT: usize = 57;
/// The version stored in `.key` files.
pub const FILE_VERSION: u16 = 0x25;
/// The magic of `default.key`: `"WS"` as a little-endian u16.
pub const DEFAULT_MAGIC: u16 = 0x5357;
/// Size of a character `.key` file: version + table.
pub const CHAR_FILE_BYTES: usize = 2 + TABLE_BYTES;
/// Size of `default.key`: magic, version and size words (6 bytes) + table.
pub const DEFAULT_FILE_BYTES: usize = 6 + TABLE_BYTES;

/// Key code of the middle mouse button.
pub const MIDDLE: u16 = 0x100;
/// Key code of the first extra mouse button.
pub const X1: u16 = 0x101;
/// Key code of the second extra mouse button.
pub const X2: u16 = 0x102;
/// Key code of a wheel step up.
pub const WHEEL_UP: u16 = 0x103;
/// Key code of a wheel step down.
pub const WHEEL_DOWN: u16 = 0x104;

/// One record of the command table, joined with the command's default bindings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CmdDef {
    /// The command number, 0-56.
    pub cmd: u8,
    /// `string.tbl` id of the command's name (0: none, command 56).
    pub string_id: u32,
    /// Default key of slot 1.
    pub key1: u16,
    /// Default key of slot 0.
    pub key2: u16,
    /// Key-down handler id (0 = no key-down handler). Dispatch only tests whether it is non-zero.
    pub down: u32,
    /// Key-up handler id (0 = no key-up handler). Dispatch only tests whether it is non-zero.
    pub up: u32,
    /// Runs in key mode 2.
    pub full_ok: bool,
    /// Position of the command's two default entries in the table (entry `2p` is slot 1, entry
    /// `2p + 1` is slot 0).
    pub file_pos: u8,
    /// Row in the classic key-config table; -1 means not listed.
    pub menu_classic: i16,
    /// Row in the expansion key-config table; -1 means not listed.
    pub menu_exp: i16,
}

/// The 57 commands, indexed by command number.
pub const COMMANDS: [CmdDef; COMMAND_COUNT] = [
    CmdDef {
        cmd: 0,
        string_id: 3924,
        key1: 0x0041,
        key2: 0x0043,
        down: 0x00468940,
        up: 0x00000000,
        full_ok: false,
        file_pos: 0,
        menu_classic: 0,
        menu_exp: 0,
    },
    CmdDef {
        cmd: 1,
        string_id: 3925,
        key1: 0x0049,
        key2: 0x0042,
        down: 0x00468950,
        up: 0x00000000,
        full_ok: false,
        file_pos: 1,
        menu_classic: 1,
        menu_exp: 1,
    },
    CmdDef {
        cmd: 2,
        string_id: 3926,
        key1: 0x0050,
        key2: 0xFFFF,
        down: 0x00468960,
        up: 0x00000000,
        full_ok: false,
        file_pos: 2,
        menu_classic: 2,
        menu_exp: 2,
    },
    CmdDef {
        cmd: 3,
        string_id: 3927,
        key1: 0x004D,
        key2: 0xFFFF,
        down: 0x00468980,
        up: 0x00000000,
        full_ok: false,
        file_pos: 3,
        menu_classic: 3,
        menu_exp: 4,
    },
    CmdDef {
        cmd: 4,
        string_id: 3928,
        key1: 0x0051,
        key2: 0xFFFF,
        down: 0x00468990,
        up: 0x00000000,
        full_ok: false,
        file_pos: 4,
        menu_classic: 4,
        menu_exp: 5,
    },
    CmdDef {
        cmd: 5,
        string_id: 3929,
        key1: 0x000D,
        key2: 0xFFFF,
        down: 0x004689B0,
        up: 0x00000000,
        full_ok: true,
        file_pos: 5,
        menu_classic: 26,
        menu_exp: 36,
    },
    CmdDef {
        cmd: 6,
        string_id: 3933,
        key1: 0x0048,
        key2: 0xFFFF,
        down: 0x004689D0,
        up: 0x00000000,
        full_ok: false,
        file_pos: 6,
        menu_classic: 5,
        menu_exp: 6,
    },
    CmdDef {
        cmd: 7,
        string_id: 3930,
        key1: 0x0009,
        key2: 0x0100,
        down: 0x00468A30,
        up: 0x00000000,
        full_ok: false,
        file_pos: 7,
        menu_classic: 33,
        menu_exp: 43,
    },
    CmdDef {
        cmd: 8,
        string_id: 3931,
        key1: 0x0078,
        key2: 0xFFFF,
        down: 0x00468A60,
        up: 0x00000000,
        full_ok: false,
        file_pos: 8,
        menu_classic: 34,
        menu_exp: 44,
    },
    CmdDef {
        cmd: 9,
        string_id: 3983,
        key1: 0x0079,
        key2: 0xFFFF,
        down: 0x00468A70,
        up: 0x00000000,
        full_ok: false,
        file_pos: 9,
        menu_classic: 35,
        menu_exp: 45,
    },
    CmdDef {
        cmd: 10,
        string_id: 3985,
        key1: 0x007A,
        key2: 0xFFFF,
        down: 0x00468A90,
        up: 0x00000000,
        full_ok: false,
        file_pos: 10,
        menu_classic: 36,
        menu_exp: 46,
    },
    CmdDef {
        cmd: 11,
        string_id: 3984,
        key1: 0x007B,
        key2: 0xFFFF,
        down: 0x00468AB0,
        up: 0x00000000,
        full_ok: false,
        file_pos: 11,
        menu_classic: 37,
        menu_exp: 47,
    },
    CmdDef {
        cmd: 12,
        string_id: 3934,
        key1: 0x0054,
        key2: 0xFFFF,
        down: 0x00468AF0,
        up: 0x00000000,
        full_ok: false,
        file_pos: 12,
        menu_classic: 7,
        menu_exp: 8,
    },
    CmdDef {
        cmd: 13,
        string_id: 10833,
        key1: 0x0053,
        key2: 0xFFFF,
        down: 0x00468B00,
        up: 0x00000000,
        full_ok: false,
        file_pos: 13,
        menu_classic: 8,
        menu_exp: 9,
    },
    CmdDef {
        cmd: 14,
        string_id: 3936,
        key1: 0x0070,
        key2: 0xFFFF,
        down: 0x00468B90,
        up: 0x00000000,
        full_ok: false,
        file_pos: 14,
        menu_classic: 9,
        menu_exp: 10,
    },
    CmdDef {
        cmd: 15,
        string_id: 3937,
        key1: 0x0071,
        key2: 0xFFFF,
        down: 0x00468BC0,
        up: 0x00000000,
        full_ok: false,
        file_pos: 15,
        menu_classic: 10,
        menu_exp: 11,
    },
    CmdDef {
        cmd: 16,
        string_id: 3938,
        key1: 0x0072,
        key2: 0xFFFF,
        down: 0x00468BF0,
        up: 0x00000000,
        full_ok: false,
        file_pos: 16,
        menu_classic: 11,
        menu_exp: 12,
    },
    CmdDef {
        cmd: 17,
        string_id: 3939,
        key1: 0x0073,
        key2: 0xFFFF,
        down: 0x00468C20,
        up: 0x00000000,
        full_ok: false,
        file_pos: 17,
        menu_classic: 12,
        menu_exp: 13,
    },
    CmdDef {
        cmd: 18,
        string_id: 3940,
        key1: 0x0074,
        key2: 0xFFFF,
        down: 0x00468C50,
        up: 0x00000000,
        full_ok: false,
        file_pos: 18,
        menu_classic: 13,
        menu_exp: 14,
    },
    CmdDef {
        cmd: 19,
        string_id: 3941,
        key1: 0x0075,
        key2: 0xFFFF,
        down: 0x00468C80,
        up: 0x00000000,
        full_ok: false,
        file_pos: 19,
        menu_classic: 14,
        menu_exp: 15,
    },
    CmdDef {
        cmd: 20,
        string_id: 3942,
        key1: 0x0076,
        key2: 0xFFFF,
        down: 0x00468CB0,
        up: 0x00000000,
        full_ok: false,
        file_pos: 20,
        menu_classic: 15,
        menu_exp: 16,
    },
    CmdDef {
        cmd: 21,
        string_id: 3943,
        key1: 0x0077,
        key2: 0xFFFF,
        down: 0x00468CE0,
        up: 0x00000000,
        full_ok: false,
        file_pos: 45,
        menu_classic: 16,
        menu_exp: 17,
    },
    CmdDef {
        cmd: 22,
        string_id: 3946,
        key1: 0x00C0,
        key2: 0xFFFF,
        down: 0x00468F00,
        up: 0x00000000,
        full_ok: true,
        file_pos: 21,
        menu_classic: 20,
        menu_exp: 29,
    },
    CmdDef {
        cmd: 23,
        string_id: 3947,
        key1: 0x0031,
        key2: 0xFFFF,
        down: 0x00468F40,
        up: 0x00000000,
        full_ok: false,
        file_pos: 22,
        menu_classic: 21,
        menu_exp: 30,
    },
    CmdDef {
        cmd: 24,
        string_id: 3948,
        key1: 0x0032,
        key2: 0xFFFF,
        down: 0x00468F50,
        up: 0x00000000,
        full_ok: false,
        file_pos: 23,
        menu_classic: 22,
        menu_exp: 31,
    },
    CmdDef {
        cmd: 25,
        string_id: 3949,
        key1: 0x0033,
        key2: 0xFFFF,
        down: 0x00468F60,
        up: 0x00000000,
        full_ok: false,
        file_pos: 24,
        menu_classic: 23,
        menu_exp: 32,
    },
    CmdDef {
        cmd: 26,
        string_id: 3950,
        key1: 0x0034,
        key2: 0xFFFF,
        down: 0x00468F70,
        up: 0x00000000,
        full_ok: false,
        file_pos: 25,
        menu_classic: 24,
        menu_exp: 33,
    },
    CmdDef {
        cmd: 27,
        string_id: 3959,
        key1: 0x0060,
        key2: 0xFFFF,
        down: 0x00468F80,
        up: 0x00000000,
        full_ok: true,
        file_pos: 26,
        menu_classic: 39,
        menu_exp: 50,
    },
    CmdDef {
        cmd: 28,
        string_id: 3960,
        key1: 0x0061,
        key2: 0xFFFF,
        down: 0x00468F90,
        up: 0x00000000,
        full_ok: true,
        file_pos: 27,
        menu_classic: 40,
        menu_exp: 51,
    },
    CmdDef {
        cmd: 29,
        string_id: 3961,
        key1: 0x0062,
        key2: 0xFFFF,
        down: 0x00468FA0,
        up: 0x00000000,
        full_ok: true,
        file_pos: 28,
        menu_classic: 41,
        menu_exp: 52,
    },
    CmdDef {
        cmd: 30,
        string_id: 3962,
        key1: 0x0063,
        key2: 0xFFFF,
        down: 0x00468FB0,
        up: 0x00000000,
        full_ok: true,
        file_pos: 29,
        menu_classic: 42,
        menu_exp: 53,
    },
    CmdDef {
        cmd: 31,
        string_id: 3963,
        key1: 0x0064,
        key2: 0xFFFF,
        down: 0x00468FC0,
        up: 0x00000000,
        full_ok: true,
        file_pos: 30,
        menu_classic: 43,
        menu_exp: 54,
    },
    CmdDef {
        cmd: 32,
        string_id: 3964,
        key1: 0x0065,
        key2: 0xFFFF,
        down: 0x00468FD0,
        up: 0x00000000,
        full_ok: true,
        file_pos: 31,
        menu_classic: 44,
        menu_exp: 55,
    },
    CmdDef {
        cmd: 33,
        string_id: 3965,
        key1: 0x0066,
        key2: 0xFFFF,
        down: 0x00468FE0,
        up: 0x00000000,
        full_ok: true,
        file_pos: 32,
        menu_classic: 45,
        menu_exp: 56,
    },
    CmdDef {
        cmd: 34,
        string_id: 3966,
        key1: 0x0011,
        key2: 0xFFFF,
        down: 0x00469000,
        up: 0x004691C0,
        full_ok: false,
        file_pos: 33,
        menu_classic: 27,
        menu_exp: 37,
    },
    CmdDef {
        cmd: 35,
        string_id: 3967,
        key1: 0x0052,
        key2: 0x0102,
        down: 0x00469060,
        up: 0x00000000,
        full_ok: false,
        file_pos: 34,
        menu_classic: 28,
        menu_exp: 38,
    },
    CmdDef {
        cmd: 36,
        string_id: 3968,
        key1: 0x0010,
        key2: 0xFFFF,
        down: 0x00469080,
        up: 0x00469200,
        full_ok: false,
        file_pos: 35,
        menu_classic: 29,
        menu_exp: 39,
    },
    CmdDef {
        cmd: 37,
        string_id: 3969,
        key1: 0x0012,
        key2: 0x0101,
        down: 0x00469090,
        up: 0x00469210,
        full_ok: false,
        file_pos: 36,
        menu_classic: 30,
        menu_exp: 40,
    },
    CmdDef {
        cmd: 38,
        string_id: 3970,
        key1: 0x0020,
        key2: 0xFFFF,
        down: 0x004690A0,
        up: 0x00000000,
        full_ok: false,
        file_pos: 37,
        menu_classic: 49,
        menu_exp: 60,
    },
    CmdDef {
        cmd: 39,
        string_id: 3944,
        key1: 0x0103,
        key2: 0xFFFF,
        down: 0x00469100,
        up: 0x00000000,
        full_ok: false,
        file_pos: 38,
        menu_classic: 17,
        menu_exp: 26,
    },
    CmdDef {
        cmd: 40,
        string_id: 3945,
        key1: 0x0104,
        key2: 0xFFFF,
        down: 0x00469120,
        up: 0x00000000,
        full_ok: false,
        file_pos: 39,
        menu_classic: 18,
        menu_exp: 27,
    },
    CmdDef {
        cmd: 41,
        string_id: 3981,
        key1: 0x004E,
        key2: 0xFFFF,
        down: 0x004691B0,
        up: 0x00000000,
        full_ok: true,
        file_pos: 40,
        menu_classic: 50,
        menu_exp: 61,
    },
    CmdDef {
        cmd: 42,
        string_id: 3971,
        key1: 0x002C,
        key2: 0xFFFF,
        down: 0x00000000,
        up: 0x004FA7A0,
        full_ok: true,
        file_pos: 41,
        menu_classic: 48,
        menu_exp: 59,
    },
    CmdDef {
        cmd: 43,
        string_id: 3982,
        key1: 0x005A,
        key2: 0xFFFF,
        down: 0x00493840,
        up: 0x00000000,
        full_ok: false,
        file_pos: 42,
        menu_classic: 31,
        menu_exp: 41,
    },
    CmdDef {
        cmd: 44,
        string_id: 22726,
        key1: 0x0057,
        key2: 0xFFFF,
        down: 0x00469140,
        up: 0x00000000,
        full_ok: true,
        file_pos: 43,
        menu_classic: -1,
        menu_exp: 34,
    },
    CmdDef {
        cmd: 45,
        string_id: 22725,
        key1: 0x0056,
        key2: 0xFFFF,
        down: 0x00468AD0,
        up: 0x00000000,
        full_ok: true,
        file_pos: 44,
        menu_classic: -1,
        menu_exp: 48,
    },
    CmdDef {
        cmd: 46,
        string_id: 22717,
        key1: 0xFFFF,
        key2: 0xFFFF,
        down: 0x00468D10,
        up: 0x00000000,
        full_ok: false,
        file_pos: 46,
        menu_classic: -1,
        menu_exp: 18,
    },
    CmdDef {
        cmd: 47,
        string_id: 22718,
        key1: 0xFFFF,
        key2: 0xFFFF,
        down: 0x00468D40,
        up: 0x00000000,
        full_ok: false,
        file_pos: 47,
        menu_classic: -1,
        menu_exp: 19,
    },
    CmdDef {
        cmd: 48,
        string_id: 22719,
        key1: 0xFFFF,
        key2: 0xFFFF,
        down: 0x00468D80,
        up: 0x00000000,
        full_ok: false,
        file_pos: 48,
        menu_classic: -1,
        menu_exp: 20,
    },
    CmdDef {
        cmd: 49,
        string_id: 22720,
        key1: 0xFFFF,
        key2: 0xFFFF,
        down: 0x00468DC0,
        up: 0x00000000,
        full_ok: false,
        file_pos: 49,
        menu_classic: -1,
        menu_exp: 21,
    },
    CmdDef {
        cmd: 50,
        string_id: 22721,
        key1: 0xFFFF,
        key2: 0xFFFF,
        down: 0x00468E00,
        up: 0x00000000,
        full_ok: false,
        file_pos: 50,
        menu_classic: -1,
        menu_exp: 22,
    },
    CmdDef {
        cmd: 51,
        string_id: 22722,
        key1: 0xFFFF,
        key2: 0xFFFF,
        down: 0x00468E40,
        up: 0x00000000,
        full_ok: false,
        file_pos: 51,
        menu_classic: -1,
        menu_exp: 23,
    },
    CmdDef {
        cmd: 52,
        string_id: 22723,
        key1: 0xFFFF,
        key2: 0xFFFF,
        down: 0x00468E80,
        up: 0x00000000,
        full_ok: false,
        file_pos: 52,
        menu_classic: -1,
        menu_exp: 24,
    },
    CmdDef {
        cmd: 53,
        string_id: 22724,
        key1: 0xFFFF,
        key2: 0xFFFF,
        down: 0x00468EC0,
        up: 0x00000000,
        full_ok: false,
        file_pos: 53,
        menu_classic: -1,
        menu_exp: 25,
    },
    CmdDef {
        cmd: 54,
        string_id: 22727,
        key1: 0x004F,
        key2: 0xFFFF,
        down: 0x00469170,
        up: 0x00000000,
        full_ok: false,
        file_pos: 54,
        menu_classic: -1,
        menu_exp: 3,
    },
    CmdDef {
        cmd: 55,
        string_id: 11083,
        key1: 0x0067,
        key2: 0xFFFF,
        down: 0x00468FF0,
        up: 0x00000000,
        full_ok: true,
        file_pos: 55,
        menu_classic: 46,
        menu_exp: 57,
    },
    CmdDef {
        cmd: 56,
        string_id: 0,
        key1: 0x001B,
        key2: 0xFFFF,
        down: 0x004690B0,
        up: 0x00000000,
        full_ok: false,
        file_pos: 56,
        menu_classic: -1,
        menu_exp: -1,
    },
];

/// One binding: i32 command, u16 key, i32 slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Binding {
    pub cmd: i32,
    pub key: u16,
    pub slot: i32,
}

/// The live binding table: 114 bindings in table order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BindingTable(pub Vec<Binding>);

/// The commands whose slot-0 entry precedes their slot-1 entry in the default table; every other
/// command lists slot 1 first. Only command 1 (inventory: `B`, then `I`) does.
const SLOT0_FIRST: [u8; 1] = [1];

impl BindingTable {
    /// The default table: entries in `file_pos` order, slot 1 then slot 0, except command 1
    /// (slot 0 first).
    pub fn defaults() -> BindingTable {
        let mut v = vec![
            Binding {
                cmd: 0,
                key: UNBOUND,
                slot: 0
            };
            TABLE_LEN
        ];
        for c in &COMMANDS {
            let p = usize::from(c.file_pos);
            let one = Binding {
                cmd: i32::from(c.cmd),
                key: c.key1,
                slot: 1,
            };
            let zero = Binding {
                cmd: i32::from(c.cmd),
                key: c.key2,
                slot: 0,
            };
            let (first, second) = if SLOT0_FIRST.contains(&c.cmd) {
                (zero, one)
            } else {
                (one, zero)
            };
            v[2 * p] = first;
            v[2 * p + 1] = second;
        }
        BindingTable(v)
    }

    /// The 1140 table bytes: i32 cmd, u16 key, i32 slot per entry.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(TABLE_BYTES);
        for b in &self.0 {
            out.extend_from_slice(&b.cmd.to_le_bytes());
            out.extend_from_slice(&b.key.to_le_bytes());
            out.extend_from_slice(&b.slot.to_le_bytes());
        }
        out
    }

    /// Reads a table from exactly [`TABLE_BYTES`] bytes; any other length gives `None`.
    pub fn from_bytes(b: &[u8]) -> Option<BindingTable> {
        if b.len() != TABLE_BYTES {
            return None;
        }
        Some(BindingTable(
            b.as_chunks::<10>()
                .0
                .iter()
                .map(|e| Binding {
                    cmd: i32::from_le_bytes([e[0], e[1], e[2], e[3]]),
                    key: u16::from_le_bytes([e[4], e[5]]),
                    slot: i32::from_le_bytes([e[6], e[7], e[8], e[9]]),
                })
                .collect(),
        ))
    }

    /// Key of (command, slot): the first entry with that command and slot; 0xFFFF if none.
    pub fn key_of(&self, cmd: i32, slot: i32) -> u16 {
        self.0
            .iter()
            .find(|b| b.cmd == cmd && b.slot == slot)
            .map_or(UNBOUND, |b| b.key)
    }

    /// Whether (command, slot) has a key.
    pub fn is_bound(&self, cmd: i32, slot: i32) -> bool {
        self.key_of(cmd, slot) != UNBOUND
    }

    /// Unbind (command, slot): every matching entry gets key 0xFFFF.
    pub fn unbind(&mut self, cmd: i32, slot: i32) {
        for b in &mut self.0 {
            if b.cmd == cmd && b.slot == slot {
                b.key = UNBOUND;
            }
        }
    }

    /// The acceptance test for a loaded table: every command 0-56 occurs as the command of at
    /// least one entry, and no two entries hold the same key other than 0xFFFF. The slot is not
    /// checked.
    pub fn is_valid(&self) -> bool {
        if self.0.len() != TABLE_LEN {
            return false;
        }
        for c in 0..COMMAND_COUNT as i32 {
            if !self.0.iter().any(|b| b.cmd == c) {
                return false;
            }
        }
        for (i, a) in self.0.iter().enumerate() {
            if a.key != UNBOUND && self.0[i + 1..].iter().any(|b| b.key == a.key) {
                return false;
            }
        }
        true
    }
}

/// A character `.key` file: u16 version 0x25 + the table.
pub fn char_file_bytes(t: &BindingTable) -> Vec<u8> {
    let mut v = FILE_VERSION.to_le_bytes().to_vec();
    v.extend(t.to_bytes());
    v
}

/// `default.key`: u16 magic, u16 version, u16 size 0x47A, then the table.
pub fn default_file_bytes(t: &BindingTable) -> Vec<u8> {
    let mut v = DEFAULT_MAGIC.to_le_bytes().to_vec();
    v.extend(FILE_VERSION.to_le_bytes());
    v.extend((DEFAULT_FILE_BYTES as u16).to_le_bytes());
    v.extend(t.to_bytes());
    v
}

/// The accepted character file: exactly 0x476 bytes, version 0x25, and a valid table.
pub fn parse_char_file(b: &[u8]) -> Option<BindingTable> {
    if b.len() != CHAR_FILE_BYTES || u16::from_le_bytes([b[0], b[1]]) != FILE_VERSION {
        return None;
    }
    BindingTable::from_bytes(&b[2..]).filter(BindingTable::is_valid)
}

/// A `default.key` buffer is used when its magic, version and size match; any other buffer is
/// dropped.
pub fn parse_default_file(b: &[u8]) -> Option<BindingTable> {
    if b.len() != DEFAULT_FILE_BYTES {
        return None;
    }
    let w = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]);
    if w(0) != DEFAULT_MAGIC || w(2) != FILE_VERSION || usize::from(w(4)) != DEFAULT_FILE_BYTES {
        return None;
    }
    BindingTable::from_bytes(&b[6..])
}

/// What the game-start load produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyLoad {
    /// The table to use.
    pub table: BindingTable,
    /// Both key files are written back; this happens only after the fallback path (no valid
    /// character file).
    pub write_files: bool,
}

/// The game-start load: the character file, else `<install>default.key` (read in full), else
/// the archive `default.key`; the built-in defaults stay when the chosen buffer is invalid. A
/// `None` input means that file could not be opened or read.
pub fn load_at_start(
    char_file: Option<&[u8]>,
    install_default: Option<&[u8]>,
    archive_default: Option<&[u8]>,
) -> KeyLoad {
    if let Some(t) = char_file.and_then(parse_char_file) {
        return KeyLoad {
            table: t,
            write_files: false,
        };
    }
    // An install-directory file of the full size (0x47A bytes) is used even if it turns out
    // invalid: the archive file is not tried after it.
    let buf = match install_default {
        Some(b) if b.len() == DEFAULT_FILE_BYTES => Some(b),
        _ => archive_default,
    };
    let table = buf
        .and_then(parse_default_file)
        .unwrap_or_else(BindingTable::defaults);
    KeyLoad {
        table,
        write_files: true,
    }
}

// ---- Keyboard ------------------------------------------------------------

/// Key down: the command of the first entry (table order) whose key equals `vk`, whose command
/// is below 0x39 and has a down handler. In `key_mode` 2 only commands with `full_ok` run.
/// Auto-repeat is ignored, and F4 with Alt does nothing.
pub fn key_down_command(
    t: &BindingTable,
    vk: u16,
    auto_repeat: bool,
    alt_down: bool,
    key_mode: u8,
) -> Option<u8> {
    if auto_repeat {
        return None;
    }
    let b = t.0.iter().find(|b| {
        b.key == vk && (0..0x39).contains(&b.cmd) && COMMANDS[b.cmd as usize].down != 0
    })?;
    if vk == 0x73 && alt_down {
        return None;
    }
    let c = &COMMANDS[b.cmd as usize];
    (key_mode != 2 || c.full_ok).then_some(c.cmd)
}

/// Key up: the command of the first entry whose key equals `vk` and whose command has an up
/// handler; the repeat bit is not tested. `key_mode` 2 gates by `full_ok` as for key down.
pub fn key_up_command(t: &BindingTable, vk: u16, key_mode: u8) -> Option<u8> {
    let b = t
        .0
        .iter()
        .find(|b| b.key == vk && (0..0x39).contains(&b.cmd) && COMMANDS[b.cmd as usize].up != 0)?;
    let c = &COMMANDS[b.cmd as usize];
    (key_mode != 2 || c.full_ok).then_some(c.cmd)
}

/// Events that change the key mode (0: no key commands, 1: all, 2: only `full_ok` commands).
/// UI states are identified by their numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyModeEvent {
    /// A game starts.
    GameStart,
    /// A game ends.
    GameEnd,
    /// The UI state with this number opened.
    UiOpen(u8),
    /// The UI state with this number closed.
    UiClose(u8),
    /// The key-config screen opened.
    KeyConfigOpen,
    /// The gold dialog closed.
    LatchClear,
    /// The gold dialog opened.
    LatchSet,
}

/// The key mode after an event: (mode, keep key-up). `None` means unchanged.
/// `open` tells whether a UI state is open (closing UI state 5 only returns to mode 1 when none of
/// the states 12, 23, 25-30 or 32 is open).
pub fn key_mode_for(ev: KeyModeEvent, open: &dyn Fn(u8) -> bool) -> Option<(u8, bool)> {
    use KeyModeEvent::*;
    match ev {
        GameStart => Some((1, false)),
        GameEnd => Some((0, false)),
        UiOpen(5) | UiOpen(23) => Some((0, true)),
        UiOpen(12) | UiOpen(25) | UiOpen(26) => Some((2, false)),
        UiClose(12) | UiClose(23) | UiClose(25) | UiClose(26) => Some((1, false)),
        UiClose(5) => {
            if [12, 23, 25, 26, 27, 28, 29, 30, 32]
                .iter()
                .any(|&u| open(u))
            {
                None
            } else {
                Some((1, false))
            }
        }
        KeyConfigOpen => Some((0, false)),
        LatchSet => Some((0, true)),
        LatchClear => Some((1, false)),
        _ => None,
    }
}

/// The Windows keys (0x5B-0x5D) are swallowed: they trigger nothing.
pub fn swallow_vk(vk: u16) -> bool {
    matches!(vk, 0x5B..=0x5D)
}

/// `WM_SYSCOMMAND` swallowing: `SC_KEYMENU` and `SC_SCREENSAVE` are always swallowed, and
/// `SC_MOVE` when the client flag `f5a30` is set (what that flag stands for is not documented
/// here).
pub fn swallow_syscommand(cmd: u16, f5a30: bool) -> bool {
    match cmd & 0xFFF0 {
        0xF100 | 0xF140 => true,
        0xF010 => f5a30,
        _ => false,
    }
}

// ---- Mouse buttons and wheel ---------------------------------------------

/// The command each configurable mouse button / wheel runs, if any.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MouseSlots {
    /// Command run on a middle-button press.
    pub middle_down: Option<u8>,
    /// Command run on a middle-button release.
    pub middle_up: Option<u8>,
    /// Command run on a press of the first extra button.
    pub x1_down: Option<u8>,
    /// Command run on a release of the first extra button.
    pub x1_up: Option<u8>,
    /// Command run on a press of the second extra button.
    pub x2_down: Option<u8>,
    /// Command run on a release of the second extra button.
    pub x2_up: Option<u8>,
    /// Command run on a wheel step up.
    pub wheel_up: Option<u8>,
    /// Command run on a wheel step down.
    pub wheel_down: Option<u8>,
}

/// Scans the table after a change to find each mouse button's commands. A wheel binding to a
/// command with an up handler is removed (its key becomes 0xFFFF); with several entries on one
/// button, the last in table order wins.
pub fn scan_mouse_slots(t: &mut BindingTable) -> MouseSlots {
    let mut s = MouseSlots::default();
    for b in &mut t.0 {
        if !(MIDDLE..=WHEEL_DOWN).contains(&b.key) || !(0..0x39).contains(&b.cmd) {
            continue;
        }
        let c = &COMMANDS[b.cmd as usize];
        let down = (c.down != 0).then_some(c.cmd);
        let up = (c.up != 0).then_some(c.cmd);
        match b.key {
            MIDDLE => {
                s.middle_down = down;
                s.middle_up = up;
            }
            X1 => {
                s.x1_down = down;
                s.x1_up = up;
            }
            X2 => {
                s.x2_down = down;
                s.x2_up = up;
            }
            wheel => {
                if c.up != 0 {
                    b.key = UNBOUND;
                } else if wheel == WHEEL_UP {
                    s.wheel_up = down;
                } else {
                    s.wheel_down = down;
                }
            }
        }
    }
    s
}

/// X button events: the high word of the event's `wparam` selects the button, 1 for 0x101 and 2
/// for 0x102; anything else is no button.
pub fn x_button_key(wparam: u32) -> Option<u16> {
    match wparam >> 16 {
        1 => Some(X1),
        2 => Some(X2),
        _ => None,
    }
}

/// The wheel accumulator: wheel deltas add up until they pass the threshold of one step.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WheelAccumulator(pub i32);

impl WheelAccumulator {
    /// One wheel event while in game: adds the signed delta. When the total's magnitude exceeds
    /// 0x77, the direction's key is returned (0x103 for a positive total, 0x104 for a negative
    /// one) and the total resets to 0; at most one action per event.
    pub fn event(&mut self, delta: i32, in_game: bool) -> Option<u16> {
        if !in_game {
            return None;
        }
        self.0 += delta;
        if self.0.abs() > 0x77 {
            let k = if self.0 > 0 { WHEEL_UP } else { WHEEL_DOWN };
            self.0 = 0;
            Some(k)
        } else {
            None
        }
    }
}

// ---- Modifiers -------------------------------------------------------------

/// The modifier word a world click passes to the click dispatcher (see [`super::click::mods`]):
/// 8 when Run is held or run lock is set, plus 4 when Stand Still is held.
pub fn world_action_flags(run_held: bool, run_lock: bool, stand_still: bool) -> u32 {
    (if run_held || run_lock { 8 } else { 0 }) + if stand_still { 4 } else { 0 }
}

/// When the window loses focus, Run held is cleared if the key of command 34 (Run) is bound and
/// not physically down.
pub fn deactivate_clears_run(t: &BindingTable, key_down: &dyn Fn(u16) -> bool) -> bool {
    [1, 0].iter().any(|&slot| {
        let k = t.key_of(34, slot);
        k != UNBOUND && !key_down(k)
    })
}

/// Stand Still re-sampled from the keys bound to command 36, both slots.
pub fn stand_still_sample(t: &BindingTable, key_down: &dyn Fn(u16) -> bool) -> bool {
    [1, 0].iter().any(|&slot| {
        let k = t.key_of(36, slot);
        k != UNBOUND && key_down(k)
    })
}

// ---- Key assignment ------------------------------------------------------

/// The error messages of the key-config screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssignError {
    /// `string.tbl` 3979 `CantAssignKey`: the key cannot be assigned to this command.
    CantAssignKey,
    /// `string.tbl` 3978 `CantAssignMW`: the mouse wheel cannot be assigned to this command.
    CantAssignMw,
}

impl AssignError {
    /// The `string.tbl` id of the message.
    pub fn string_id(self) -> u32 {
        match self {
            AssignError::CantAssignKey => 3979,
            AssignError::CantAssignMw => 3978,
        }
    }
}

/// Which keys the key-config screen accepts: every mouse input, and the digits, letters and the
/// listed editing, function, numpad and punctuation keys.
pub fn key_allowed(k: u16) -> bool {
    if k >= 0x100 {
        return true;
    }
    matches!(k,
        0x30..=0x39 | 0x41..=0x5A
        | 8 | 9 | 0xC | 0xD | 0x10..=0x14 | 0x20..=0x24 | 0x2A | 0x2C..=0x2F
        | 0x60..=0x87 | 0x91 | 0xBA..=0xC0 | 0xDB..=0xDE)
}

/// Assigns key `k` to (command, slot). The command must be below 0x38 and the key allowed; the
/// wheel on a command with an up handler and Print Screen on a command with a down handler are
/// refused. Then the first other entry holding `k` is unbound and `k` is written to the entry (if
/// the (command, slot) has no entry, `k` is written back where it was).
pub fn assign_key(t: &mut BindingTable, cmd: i32, slot: i32, k: u16) -> Result<(), AssignError> {
    if !(0..0x38).contains(&cmd) || !key_allowed(k) {
        return Err(AssignError::CantAssignKey);
    }
    let c = &COMMANDS[cmd as usize];
    if matches!(k, WHEEL_UP | WHEEL_DOWN) && c.up != 0 {
        return Err(AssignError::CantAssignMw);
    }
    if k == 0x2C && c.down != 0 {
        return Err(AssignError::CantAssignKey);
    }
    let target = t.0.iter().position(|b| b.cmd == cmd && b.slot == slot);
    let other =
        t.0.iter()
            .enumerate()
            .find(|(i, b)| b.key == k && Some(*i) != target)
            .map(|(i, _)| i);
    if let Some(o) = other {
        t.0[o].key = UNBOUND;
    }
    match target {
        Some(i) => t.0[i].key = k,
        None => {
            if let Some(o) = other {
                t.0[o].key = k;
            }
        }
    }
    Ok(())
}

// ---- Skill hotkeys, the handler gate and belt use ----------------------------

/// Skill hotkey of a command: commands 14-21 give hotkeys 0-7, commands 46-53 give hotkeys 8-15.
pub fn hotkey_of_command(cmd: u8) -> Option<u8> {
    match cmd {
        14..=21 => Some(cmd - 14),
        46..=53 => Some(cmd - 46 + 8),
        _ => None,
    }
}

/// What a skill hotkey does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HotkeyAction {
    /// The skill speed bar (UI state 3) is open: assign the hovered skill to hotkey `k`.
    Assign(u8),
    /// Otherwise use hotkey `k`.
    Use(u8),
}

/// What hotkey `k` does, depending on whether the skill speed bar is open.
pub fn hotkey_action(k: u8, speed_bar_open: bool) -> HotkeyAction {
    if speed_bar_open {
        HotkeyAction::Assign(k)
    } else {
        HotkeyAction::Use(k)
    }
}

/// The use result of hotkey `k`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HotkeyUse {
    /// The value sent to the server to select the skill.
    pub send_value: u32,
    /// The skill becomes the left skill (flag non-zero), else the right one.
    pub left: bool,
}

/// Using hotkey `k`: nothing when the hotkey's skill is -1 or `allowed` is false (the use check
/// refuses); otherwise the last used hotkey becomes `k`.
pub fn hotkey_use(
    k: usize,
    skill: &[i32; 16],
    send: &[u32; 16],
    left_flag: &[u8; 16],
    allowed: bool,
    last_hotkey: &mut usize,
) -> Option<HotkeyUse> {
    if skill[k] == -1 || !allowed {
        return None;
    }
    *last_hotkey = k;
    Some(HotkeyUse {
        send_value: send[k],
        left: left_flag[k] != 0,
    })
}

/// The gate of a key handler: it does nothing while the client's exit flag is set, or there is no
/// local player, or the player is dead (mode 0x11).
pub fn gate_blocks(exit_flag: bool, player_mode: Option<u8>) -> bool {
    exit_flag || player_mode.is_none_or(|m| m == 0x11)
}

/// Facts of a belt-use attempt.
#[derive(Clone, Copy, Debug)]
pub struct BeltUseFacts {
    /// The game is the expansion (Shift on a belt use only counts there).
    pub expansion: bool,
    /// Shift is held.
    pub shift: bool,
    /// An item is held on the cursor.
    pub cursor_item: bool,
    /// UI state 9 is open.
    pub ui9_open: bool,
    /// The item in the belt slot (GUID), if any.
    pub item: Option<u32>,
    /// The item's `quest` and `unique` flags are not 1.
    pub passes_quest_unique: bool,
    /// The item's `useable` flag is 1.
    pub useable: bool,
    /// The item is busy (the busy test is non-zero), so it cannot be used now.
    pub busy: bool,
}

/// What a belt-use attempt does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BeltUse {
    /// Nothing happens.
    Nothing,
    /// The quest / unique item handler runs and the attempt stops.
    QuestUniqueHandler,
    /// The player event sound for an item that cannot be used plays and the attempt stops.
    NotUseableSound,
    /// C->S 0x26 `[item, shift 0 or 0x8000, 0]`.
    Send {
        /// The item's GUID.
        item: u32,
        /// 0x8000 when Shift counts, else 0.
        shift: u32,
    },
}

/// Decides what using the belt item does: nothing with an item on the cursor or UI state 9 open,
/// or without an item; the quest / unique handler for quest or unique items; the not-useable sound
/// for items that are not useable; nothing while the item is busy; otherwise the use message.
pub fn belt_use(f: &BeltUseFacts) -> BeltUse {
    let shift = f.shift && f.expansion;
    if f.cursor_item || f.ui9_open {
        return BeltUse::Nothing;
    }
    let Some(item) = f.item else {
        return BeltUse::Nothing;
    };
    if !f.passes_quest_unique {
        return BeltUse::QuestUniqueHandler;
    }
    if !f.useable {
        return BeltUse::NotUseableSound;
    }
    if f.busy {
        return BeltUse::Nothing;
    }
    BeltUse::Send {
        item,
        shift: if shift { 0x8000 } else { 0 },
    }
}

/// A belt key fires only when the column-ready byte is 1. Returns the Shift flag: bit 0x8000 of
/// the Shift key state.
pub fn belt_key(column_ready: u8, shift_state: i16) -> Option<bool> {
    (column_ready == 1).then_some(shift_state as u16 & 0x8000 != 0)
}

/// One row of the key-config menu tables: command (57 = separator) and its string id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MenuRow {
    /// The row's command, or [`SEPARATOR`].
    pub cmd: i32,
    /// `string.tbl` id of the row's name (0 for a separator).
    pub string_id: u32,
}

/// Separator row command.
pub const SEPARATOR: i32 = 57;

/// The key-config table of a game: 62 rows for the expansion (separators at 7, 28, 35, 42, 49,
/// 58) and 51 for classic (separators at 6, 19, 25, 32, 38, 47).
pub fn menu_table(expansion: bool) -> Vec<MenuRow> {
    let (len, seps): (usize, &[usize]) = if expansion {
        (62, &[7, 28, 35, 42, 49, 58])
    } else {
        (51, &[6, 19, 25, 32, 38, 47])
    };
    let mut rows = vec![
        MenuRow {
            cmd: SEPARATOR,
            string_id: 0
        };
        len
    ];
    for c in &COMMANDS {
        let r = if expansion {
            c.menu_exp
        } else {
            c.menu_classic
        };
        if r >= 0 {
            rows[r as usize] = MenuRow {
                cmd: i32::from(c.cmd),
                string_id: c.string_id,
            };
        }
    }
    debug_assert!(seps.iter().all(|&s| rows[s].cmd == SEPARATOR));
    rows
}

// ---- Pointer buttons -------------------------------------------------------

/// A pointer button (`PointerButton` of the UI, `UiFrame::unhandled`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    /// The left button.
    Left,
    /// The right button.
    Right,
    /// The middle button.
    Middle,
    /// The first extra button.
    X1,
    /// The second extra button.
    X2,
}

/// What a pointer button means for D2RUST's input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerMeaning {
    /// The left button is a fixed world button, never rebindable (see [`super::click`]).
    WorldLeft,
    /// The right button is a fixed world button, never rebindable.
    WorldRight,
    /// Middle and the X buttons run whatever command is bound to keys 0x100-0x102; the default
    /// middle button is command 7.
    Command(u8),
    /// Bound to nothing.
    Unbound,
}

/// The meaning of `b` under the table `t`; with several entries on one button, the last in table
/// order wins.
pub fn pointer_meaning(t: &BindingTable, b: Button) -> PointerMeaning {
    let key = match b {
        Button::Left => return PointerMeaning::WorldLeft,
        Button::Right => return PointerMeaning::WorldRight,
        Button::Middle => MIDDLE,
        Button::X1 => X1,
        Button::X2 => X2,
    };
    t.0.iter()
        .rev()
        .find(|e| e.key == key && (0..0x39).contains(&e.cmd))
        .map_or(PointerMeaning::Unbound, |e| {
            PointerMeaning::Command(e.cmd as u8)
        })
}

/// The modifiers a command can give. What Shift / Ctrl / Alt mean comes only from the bindings of
/// commands 36 / 34 / 37, except on a right-button release, which reads the event's Shift /
/// Control state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Modifier {
    /// Stand Still (command 36).
    StandStill,
    /// Run (command 34).
    Run,
    /// Show items (command 37).
    ShowItems,
}

/// The command number that gives the modifier.
pub fn modifier_command(m: Modifier) -> u8 {
    match m {
        Modifier::StandStill => 36,
        Modifier::Run => 34,
        Modifier::ShowItems => 37,
    }
}

#[cfg(test)]
mod tests;
