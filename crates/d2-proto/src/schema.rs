//! Message table types: one descriptor per message id and direction, as
//! the machine tables `tables/sim/client-messages.tsv` and
//! `server-messages.tsv` define them. The generated tables in
//! [`crate::generated`] are built from these types; [`crate::tsv`] parses
//! the TSVs into the same pieces. [`crate::transport`] evaluates the size
//! rules to classify and split messages.

/// Width of the length field a size rule reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Width {
    /// One byte.
    U8,
    /// Two bytes, little-endian.
    U16,
}

impl Width {
    /// Number of bytes the field occupies.
    pub const fn bytes(self) -> usize {
        match self {
            Width::U8 => 1,
            Width::U16 => 2,
        }
    }
}

/// A size rule of the C→S or the S→C message table (the grammar is the
/// `size` column syntax of the TSVs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SizeRule {
    /// Fixed size; 0 = never valid.
    Fixed(u16),
    /// `<u8|u16>@<offset>[*mul][+add][;cap=n][;min=n]`: the little-endian
    /// field, replaced by 0 when it exceeds `cap`, times `mul`, plus `add`.
    /// The message needs at least `min` bytes and the field's bytes.
    Field {
        /// Width of the length field.
        width: Width,
        /// Byte offset of the length field.
        offset: u8,
        /// Factor applied to the field.
        mul: u16,
        /// Constant added after the multiplication.
        add: u16,
        /// Largest field value kept; a larger one counts as 0.
        cap: Option<u16>,
        /// Minimum number of bytes needed to evaluate the rule.
        min: u16,
    },
    /// C→S 0x14 / 0x15: two NUL-terminated strings; a trailing
    /// signed byte is added to the size.
    Chat,
    /// S→C 0x26: ten fixed bytes plus two NUL-terminated strings.
    Chat26,
    /// S→C 0xAF: a length byte (0 counts as 1) plus the id byte.
    Af,
}

/// Result of a size rule on the bytes at the start of a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    /// The message is this many bytes. It can exceed the bytes given: the
    /// rules only need their own fields (callers compare).
    Bytes(usize),
    /// The rule gives 0: not a valid message.
    Invalid,
    /// Too few bytes to evaluate the rule.
    Incomplete,
    /// The C→S chat rule adds a signed byte and can come out negative.
    /// What the classifier should do with a negative size is not settled
    /// (see `docs/HANDOFF.md`); callers must not treat it as either of the
    /// other results.
    Negative(i32),
}

fn cstrlen(b: &[u8], from: usize) -> Option<usize> {
    b.get(from..)?.iter().position(|&c| c == 0)
}

impl SizeRule {
    /// Whether the id can ever be valid (`Fixed(0)` never is).
    pub const fn is_never(&self) -> bool {
        matches!(self, SizeRule::Fixed(0))
    }

    /// The fixed size, if the rule is fixed and non-zero.
    pub const fn fixed(&self) -> Option<usize> {
        match *self {
            SizeRule::Fixed(0) => None,
            SizeRule::Fixed(n) => Some(n as usize),
            _ => None,
        }
    }

    /// Evaluates the rule on `b` (the message, id at `b[0]`, possibly
    /// followed by more bytes).
    pub fn eval(&self, b: &[u8]) -> Size {
        let n: i64 = match *self {
            SizeRule::Fixed(n) => n as i64,
            SizeRule::Field {
                width,
                offset,
                mul,
                add,
                cap,
                min,
            } => {
                let off = offset as usize;
                let need = (min as usize).max(off + width.bytes());
                if b.len() < need {
                    return Size::Incomplete;
                }
                let mut v = match width {
                    Width::U8 => b[off] as u32,
                    Width::U16 => u16::from_le_bytes([b[off], b[off + 1]]) as u32,
                };
                if cap.is_some_and(|c| v > c as u32) {
                    v = 0;
                }
                v as i64 * mul as i64 + add as i64
            }
            SizeRule::Chat => {
                if b.len() < 3 {
                    return Size::Incomplete;
                }
                let Some(l1) = cstrlen(b, 3) else {
                    return Size::Incomplete;
                };
                let Some(l2) = cstrlen(b, l1 + 4) else {
                    return Size::Incomplete;
                };
                let k = l1 + l2 + 5;
                let Some(&c) = b.get(k) else {
                    return Size::Incomplete;
                };
                (l1 + l2 + 6) as i64 + c as i8 as i64
            }
            SizeRule::Chat26 => {
                if b.len() < 10 {
                    return Size::Incomplete;
                }
                let Some(l1) = cstrlen(b, 10) else {
                    return Size::Incomplete;
                };
                let Some(l2) = cstrlen(b, 11 + l1) else {
                    return Size::Incomplete;
                };
                (10 + l1 + 1 + l2 + 1) as i64
            }
            SizeRule::Af => match b.get(1) {
                None => return Size::Incomplete,
                Some(0) => 2,
                Some(&v) => v as i64 + 1,
            },
        };
        match n {
            0 => Size::Invalid,
            n if n < 0 => Size::Negative(n as i32),
            n => Size::Bytes(n as usize),
        }
    }
}

/// The C→S handler's own size check (`handler_size` column).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandlerSize {
    /// `==N`.
    Exact(u16),
    /// `A..B`, both inclusive.
    Range(u16, u16),
    /// `chat`: the handler checks the 0x15 strings itself.
    Chat,
    /// `any`: no check.
    Any,
    /// `-`: stub or no handler.
    None,
}

/// Field type of a layout entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldType {
    /// One byte.
    U8,
    /// Two bytes, little-endian.
    U16,
    /// Four bytes, little-endian.
    U32,
    /// NUL-terminated string.
    Cstr,
    /// 16-byte string field.
    Cstr16,
    /// `bytesN`: N raw bytes at the offset (N ≥ 1; a record the message
    /// carries whole, e.g. S→C 0x29's 96-byte quest record).
    Bytes(u16),
    /// `uN`: bits 0..N of the u32 at the offset (N not 8, 16, 32).
    Bits(u8),
    /// `bitN`: bit N of the u32 at the offset.
    Bit(u8),
    /// `name@off` without a type: the bytes from the offset to the end.
    Tail,
    /// A field of a `bits:` layout (server-messages.tsv `layout` column):
    /// `width` bits starting at absolute message bit `bit`, counted
    /// LSB-first from bit 0 of byte 0 (the id byte included). The
    /// field's [`Field::offset`] is `None`.
    Packed {
        /// Absolute message bit of the field's lowest bit.
        bit: u16,
        /// Number of bits, 1..=[`PACKED_MAX_WIDTH`].
        width: u8,
    },
}

/// Bits a packed field can hold at most.
pub const PACKED_MAX_WIDTH: u8 = 32;

/// Reads `width` (1..=32) bits of `b` starting at message bit `bit`,
/// LSB-first from bit 0 of byte 0: field bit `i` is bit `(bit + i) % 8`
/// of byte `(bit + i) / 8`. Panics if the bits run past `b`.
pub fn packed_get(b: &[u8], bit: usize, width: u32) -> u32 {
    assert!(
        (1..=PACKED_MAX_WIDTH as u32).contains(&width),
        "width {width}"
    );
    (0..width as usize).fold(0, |v, i| {
        let p = bit + i;
        v | ((b[p / 8] >> (p % 8)) as u32 & 1) << i
    })
}

/// ORs the low `width` bits of `v` into `out` starting at message bit
/// `bit` (the order of [`packed_get`]). Fields share no bit, so ORing
/// into a zeroed buffer builds the message in any order. A value wider
/// than its field is cut to the field (each value is cut to its width, so
/// a field never spills into its neighbour). Panics if the bits run past
/// `out`.
pub fn packed_put(out: &mut [u8], bit: usize, width: u32, v: u32) {
    assert!(
        (1..=PACKED_MAX_WIDTH as u32).contains(&width),
        "width {width}"
    );
    for i in 0..width as usize {
        let p = bit + i;
        out[p / 8] |= ((v >> i & 1) as u8) << (p % 8);
    }
}

/// One layout entry: `name:type@offset`. `name` is empty for an unnamed
/// entry (`type@offset`); `offset` is `None` for a `cstr` that follows the
/// previous string or for a [`FieldType::Packed`] field (its bit offset
/// is in the type).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Field<'a> {
    /// Field name; empty for an unnamed entry.
    pub name: &'a str,
    /// Field type.
    pub ty: FieldType,
    /// Byte offset in the message, if the type has one.
    pub offset: Option<u16>,
}

/// `kind` column of the C→S table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A game message with a real handler.
    Handler,
    /// Stub returning 0.
    Stub0,
    /// Stub returning 3.
    Stub3,
    /// A system id (0x67..0x70).
    System,
    /// No handler.
    None,
}

/// `gate` column of the C→S table: the player-state condition the handler
/// checks before acting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    /// Handled only while the player is alive.
    Alive,
    /// Handled only while the player is dead.
    Dead,
    /// No player-state condition.
    None,
    /// System message: no player gate applies.
    System,
    /// `-`: no handler.
    Unset,
}

/// `scope` column of the C→S table: which layer handles the message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// The simulation (`d2-sim`).
    Sim,
    /// The server's session layer.
    Session,
    /// Handled outside the sim and session layers (e.g. party, anti-cheat).
    Out,
    /// No handler.
    None,
}

/// `produced_by` column of the S→C table: which layer emits the message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProducedBy {
    /// The simulation (`d2-sim`).
    Sim,
    /// The server's session layer.
    Session,
    /// The transport layer.
    Transport,
    /// Produced outside the sim and session layers.
    Out,
    /// Not produced.
    None,
}

/// `confirmed` column of both tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Confirmed {
    /// The row is fully measured.
    Yes,
    /// Part of the row is still unmeasured.
    Partial,
}

/// One C→S message id (a `client-messages.tsv` row).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClientMessage {
    /// Message id (first byte).
    pub id: u8,
    /// Handler name, `UnusedNN` / `SysNN`, or `-`.
    pub name: &'static str,
    /// Size rule the transport uses to cut the message.
    pub transport_size: SizeRule,
    /// Size check the handler applies itself.
    pub handler_size: HandlerSize,
    /// Known fields.
    pub layout: &'static [Field<'static>],
    /// Handler key from the TSV, if any: ids with the same key share one
    /// handler (all the system messages, for example).
    pub handler: Option<u32>,
    /// What kind of entry the id is.
    pub kind: Kind,
    /// Player-state condition on the handler.
    pub gate: Gate,
    /// What the message asks for (free text from the TSV).
    pub request: &'static str,
    /// Which layer handles the message.
    pub scope: Scope,
    /// How completely the row is measured.
    pub confirmed: Confirmed,
}

/// One S→C message id (a `server-messages.tsv` row).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServerMessage {
    /// Message id (first byte).
    pub id: u8,
    /// Message name, or `-`.
    pub name: &'static str,
    /// Size rule that cuts the message.
    pub size: SizeRule,
    /// Known fields. Empty = unconfirmed.
    pub layout: &'static [Field<'static>],
    /// Builder keys from the TSV: one per code path that builds the
    /// message.
    pub senders: &'static [u32],
    /// Client handler key from the TSV, if any.
    pub client_handler: Option<u32>,
    /// Client unit-update handler key from the TSV, if any: `Some` marks a
    /// message the client applies to one unit.
    pub client_unit_handler: Option<u32>,
    /// Which layer emits the message.
    pub produced_by: ProducedBy,
    /// How completely the row is measured.
    pub confirmed: Confirmed,
}
