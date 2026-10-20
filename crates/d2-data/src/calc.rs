//! Formula fields: the expression compiler for formula cells in the game's `.txt` tables
//! (skills, missiles, items), the constant evaluator it folds with (no parameter
//! callbacks, no functions), and the validator for compiled code buffers.
//!
//! The compiler turns a formula string such as `lvl*2+ln12` into a bytecode expression
//! ending in 0x00; `compile_set` calls it for every formula column, and the `.bin` loader
//! uses [`validate_buffer`] to check buffers read from the player's own tables. The full
//! evaluator, which needs unit context, belongs to `d2-sim`.
//!
//! A mod game adds function keywords per family through a [`CalcRegistry`]
//! ([`compile_with`]); D2RUST's own loader compiles with an empty one ([`compile`]).

use crate::schema::CalcBuffer;

mod registry;
pub use registry::{CalcRegistry, ModKeyword, RegistryError, MAX_ARITY, NO_KEYWORDS};

/// Expression families. `skilldesc` shares every rule with `skills`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// Formulas in `skills.txt` and `skilldesc.txt`.
    Skills,
    /// Formulas in `missiles.txt`.
    Missiles,
    /// Formulas in the item tables.
    Items,
}

impl Family {
    /// The family whose rules compile a buffer's formulas.
    pub fn of(buffer: CalcBuffer) -> Family {
        match buffer {
            CalcBuffer::SkillsCode | CalcBuffer::SkillDescCode => Family::Skills,
            CalcBuffer::MissCode => Family::Missiles,
            CalcBuffer::ItemsCode => Family::Items,
        }
    }

    /// Function keywords of this family, in function-index order.
    pub fn keywords(self) -> &'static [Keyword] {
        use Keyword::*;
        match self {
            Family::Skills => &[Min, Max, Rand, Skill, Miss, Stat, Sklvl],
            Family::Missiles => &[Min, Max, Rand, Skill, Miss],
            Family::Items => &[Min, Max, Rand, Stat],
        }
    }

    /// Compiler arity of function `index`.
    pub fn arity(self, index: u8) -> i32 {
        if self == Family::Skills && index == 6 {
            3
        } else {
            2
        }
    }

    /// Number of entries in this family's evaluator function table.
    pub fn function_count(self) -> u8 {
        match self {
            Family::Skills => 7,
            Family::Missiles | Family::Items => 4,
        }
    }
}

/// Function keywords.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keyword {
    /// `min(a, b)`.
    Min,
    /// `max(a, b)`.
    Max,
    /// `rand(a, b)`.
    Rand,
    /// `skill(name.param)`.
    Skill,
    /// `miss(name.param)`.
    Miss,
    /// `stat(name)`.
    Stat,
    /// `sklvl(name.param, n)`, three-argument form.
    Sklvl,
}

impl Keyword {
    /// The keyword as written in formula text.
    pub fn text(self) -> &'static str {
        match self {
            Keyword::Min => "min",
            Keyword::Max => "max",
            Keyword::Rand => "rand",
            Keyword::Skill => "skill",
            Keyword::Miss => "miss",
            Keyword::Stat => "stat",
            Keyword::Sklvl => "sklvl",
        }
    }
}

/// The links a formula resolves names through. Name keys are
/// already normalized (first 31 bytes, `A`–`Z` lowercased); codes are the
/// 4-byte space-padded code as a little-endian u32. `None` = miss.
pub trait CalcLinks {
    /// Index of the skill with this name key.
    fn skill(&self, key: &[u8]) -> Option<u32>;
    /// Index of the missile with this name key.
    fn missile(&self, key: &[u8]) -> Option<u32>;
    /// Index of the stat with this name key.
    fn stat(&self, key: &[u8]) -> Option<u32>;
    /// Index of the skill-calc parameter with this code.
    fn skillcalc(&self, code: u32) -> Option<u32>;
    /// Index of the missile-calc parameter with this code.
    fn misscalc(&self, code: u32) -> Option<u32>;
}

/// Name key: first 31 bytes, `A`–`Z` lowercased.
fn name_key(name: &[u8]) -> Vec<u8> {
    name[..name.len().min(31)].to_ascii_lowercase()
}

/// Code: first 4 bytes, space-padded, as u32 LE.
fn code_of(name: &[u8]) -> u32 {
    let mut c = [b' '; 4];
    let n = name.len().min(4);
    c[..n].copy_from_slice(&name[..n]);
    u32::from_le_bytes(c)
}

/// What a resolved name becomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Constant,
    Parameter,
}

/// Non-fatal outcomes reported for a formula.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CalcDiag {
    /// The formula failed to compile; the field is 0xFFFFFFFF.
    Fail,
    /// Tokenizing stopped before the end; the rest was ignored.
    Stop,
    /// A name resolved to nothing and compiled as constant 0.
    UnknownName,
    /// An unclosed `(` was emitted as 0x02.
    OpenParen,
    /// A function was still open at the end and was dropped.
    OpenFunction,
}

/// A formula D2RUST refuses to compile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalcError {
    /// A byte ≥ 0x80 in the formula text.
    NonAscii,
    /// A missile formula calls `rand`; the random-number source for missiles is not settled.
    MissileRand,
}

/// One compiled formula.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Compiled {
    /// Bytecode ending with 0x00; empty = no expression (field
    /// 0xFFFFFFFF).
    pub code: Vec<u8>,
    /// Non-fatal outcomes noted while compiling.
    pub diagnostics: Vec<CalcDiag>,
}

const OUT_LIMIT: usize = 1024;
const OPS_LIMIT: usize = 64;

/// Strength of a pending entry, by opcode 0x00–0x17.
const STRENGTH: [u8; 24] = [
    23, 1, 2, 3, 9, 9, 9, 9, 9, 9, 15, 15, 15, 15, 15, 15, 17, 17, 19, 19, 20, 21, 22, 0,
];

const PAREN: u8 = 0x02;
const OP_COND: u8 = 0x16;
const OP_NEG: u8 = 0x15;
const N_END: u8 = 0;
const N_COMMA: u8 = 3;
const N_COLON: u8 = 23;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    /// `(` or an operator 0x0A–0x16.
    Entry(u8),
    /// A function call: its index and the keyword whose name rules apply inside it.
    Func(u8, Option<Keyword>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tok {
    Value(i32, Kind),
    Func(u8, Option<Keyword>),
    LParen,
    RParen,
    Comma,
    Plus,
    Minus,
    Mul,
    Div,
    Pow,
    Cmp(u8),
    Question,
    Colon,
    Stop,
}

/// Failure: the field gets 0xFFFFFFFF.
struct Fail;

fn is_space(b: u8) -> bool {
    matches!(b, 0x09..=0x0D | 0x20)
}

struct Parser<'a> {
    family: Family,
    links: &'a dyn CalcLinks,
    registry: &'a CalcRegistry,
    text: &'a [u8],
    pos: usize,
    out: Vec<u8>,
    ops: Vec<Op>,
    count: i32,
    pending: bool,
    called: bool,
    diags: Vec<CalcDiag>,
}

impl Parser<'_> {
    /// The context for name lookup: the top entry when it is a function.
    fn context(&self) -> Option<Keyword> {
        match self.ops.last() {
            Some(Op::Func(_, k)) => *k,
            _ => None,
        }
    }

    /// Resolves a name to (index, kind), or `None` when it does not resolve (−1).
    fn resolve(&self, name: &[u8]) -> Option<(i32, Kind)> {
        let l = self.links;
        let constant = |i: Option<u32>| i.map(|i| (i as i32, Kind::Constant));
        let parameter = |i: Option<u32>| i.map(|i| (i as i32, Kind::Parameter));
        let stat_mode = || {
            let mode = if name.eq_ignore_ascii_case(b"base") {
                1
            } else if name.eq_ignore_ascii_case(b"mod") {
                2
            } else {
                0
            };
            Some((mode, Kind::Parameter))
        };
        let key = name_key(name);
        let code = code_of(name);
        match (self.family, self.context()) {
            (Family::Skills | Family::Missiles, Some(Keyword::Skill | Keyword::Sklvl)) => {
                constant(l.skill(&key)).or_else(|| parameter(l.skillcalc(code)))
            }
            (Family::Skills | Family::Missiles, Some(Keyword::Miss)) => {
                constant(l.missile(&key)).or_else(|| parameter(l.misscalc(code)))
            }
            (Family::Skills | Family::Items, Some(Keyword::Stat)) => {
                constant(l.stat(&key)).or_else(stat_mode)
            }
            (Family::Skills, _) => parameter(l.skillcalc(code)),
            (Family::Missiles, _) => parameter(l.misscalc(code)),
            (Family::Items, _) => Some((0, Kind::Parameter)),
        }
    }

    /// A name or quoted-name token: −1 is constant 0.
    fn name_token(&mut self, name: &[u8]) -> Tok {
        match self.resolve(name) {
            Some((i, kind)) => Tok::Value(i, kind),
            None => {
                self.diags.push(CalcDiag::UnknownName);
                Tok::Value(0, Kind::Constant)
            }
        }
    }

    fn stop_here(&mut self) -> Tok {
        self.diags.push(CalcDiag::Stop);
        Tok::Stop
    }

    /// Reads one token at the current position.
    fn next(&mut self) -> Tok {
        let t = self.text;
        while self.pos < t.len() && (is_space(t[self.pos]) || t[self.pos] == b'"') {
            self.pos += 1;
        }
        let Some(&b) = t.get(self.pos) else {
            return Tok::Stop;
        };
        let alnum_run = |from: usize| {
            let mut e = from;
            while e < t.len() && t[e].is_ascii_alphanumeric() {
                e += 1;
            }
            e
        };
        match b {
            b'0'..=b'9' => {
                let mut v: i32 = 0;
                while let Some(d @ b'0'..=b'9') = t.get(self.pos).copied() {
                    v = v.wrapping_mul(10).wrapping_add(i32::from(d - b'0'));
                    self.pos += 1;
                }
                Tok::Value(v, Kind::Constant)
            }
            b'\'' => {
                let start = self.pos + 1;
                let end = t[start..]
                    .iter()
                    .position(|&c| c == b'\'')
                    .map_or(t.len(), |p| start + p);
                self.pos = (end + 1).min(t.len());
                let name = &t[start..end.min(start + 255)];
                self.name_token(name)
            }
            b'A'..=b'Z' | b'a'..=b'z' => {
                let start = self.pos;
                let end = alnum_run(start);
                let word = &t[start..end.min(start + 255)];
                self.pos = end;
                while self.pos < t.len() && is_space(t[self.pos]) {
                    self.pos += 1;
                }
                if t.get(self.pos) == Some(&b'(') {
                    if let Some(i) = self
                        .family
                        .keywords()
                        .iter()
                        .position(|k| k.text().as_bytes().eq_ignore_ascii_case(word))
                    {
                        self.pos += 1;
                        return Tok::Func(i as u8, Some(self.family.keywords()[i]));
                    }
                    // A mod game's keyword, only when no built-in matched.
                    if let Some((i, k)) = self.registry.find(self.family, word) {
                        self.pos += 1;
                        return Tok::Func(i, k.names_like);
                    }
                }
                self.name_token(word)
            }
            b'.' => {
                let start = self.pos + 1;
                let end = alnum_run(start);
                self.pos = end;
                match self.resolve(&t[start..end]) {
                    Some((i, _)) => Tok::Value(i, Kind::Constant),
                    None => self.stop_here(),
                }
            }
            b'<' | b'>' | b'=' | b'!' => {
                let eq = t.get(self.pos + 1) == Some(&b'=');
                let op = match (b, eq) {
                    (b'<', false) => 0x0A,
                    (b'>', false) => 0x0B,
                    (b'<', true) => 0x0C,
                    (b'>', true) => 0x0D,
                    (b'=', true) => 0x0E,
                    (b'!', true) => 0x0F,
                    _ => return self.stop_here(),
                };
                self.pos += if eq { 2 } else { 1 };
                Tok::Cmp(op)
            }
            _ => {
                let tok = match b {
                    b'(' => Tok::LParen,
                    b')' => Tok::RParen,
                    b',' => Tok::Comma,
                    b'+' => Tok::Plus,
                    b'-' => Tok::Minus,
                    b'*' => Tok::Mul,
                    b'/' => Tok::Div,
                    b'^' => Tok::Pow,
                    b'?' => Tok::Question,
                    b':' => Tok::Colon,
                    _ => return self.stop_here(),
                };
                self.pos += 1;
                tok
            }
        }
    }

    fn arity(&self, op: Op) -> i32 {
        match op {
            Op::Func(i, _) => match self.registry.function(self.family, i) {
                Some(k) => i32::from(k.arity),
                None => self.family.arity(i),
            },
            Op::Entry(0x0A..=0x14) => 2,
            Op::Entry(OP_NEG) => 1,
            Op::Entry(OP_COND) => 3,
            Op::Entry(_) => 0,
        }
    }

    fn append(&mut self, byte: u8) -> Result<(), Fail> {
        if self.out.len() >= OUT_LIMIT {
            return Err(Fail);
        }
        self.out.push(byte);
        Ok(())
    }

    /// Emits opcode `op` and updates the operand count.
    fn emit(&mut self, op: Op) -> Result<(), Fail> {
        let arity = self.arity(op);
        if self.count < arity {
            return Err(Fail);
        }
        match op {
            Op::Entry(code) => {
                if code == PAREN {
                    self.diags.push(CalcDiag::OpenParen);
                }
                self.append(code)?;
            }
            Op::Func(i, _) => {
                self.append(0x01)?;
                self.append(i)?;
            }
        }
        self.count += 1 - arity;
        Ok(())
    }

    /// Emits the push instruction for a value.
    fn push_value(&mut self, v: i32, kind: Kind) -> Result<(), Fail> {
        let (small, mid, big) = match kind {
            Kind::Constant => (0x07, 0x08, 0x09),
            Kind::Parameter => (0x04, 0x05, 0x06),
        };
        let len = self.out.len();
        if (-128..=127).contains(&v) {
            if len >= OUT_LIMIT {
                return Err(Fail);
            }
            self.out.extend_from_slice(&[small, v as u8]);
        } else if (-32768..=32767).contains(&v) {
            if len + 3 >= OUT_LIMIT {
                return Err(Fail);
            }
            self.out.push(mid);
            self.out.extend_from_slice(&(v as i16).to_le_bytes());
        } else {
            if len + 5 >= OUT_LIMIT {
                return Err(Fail);
            }
            self.out.push(big);
            self.out.extend_from_slice(&v.to_le_bytes());
        }
        self.count += 1;
        Ok(())
    }

    fn strength(op: Op) -> u8 {
        match op {
            Op::Entry(code) => STRENGTH[usize::from(code)],
            Op::Func(..) => 0,
        }
    }

    /// Handles operator `N`: pops pending entries of equal or greater strength, then pushes.
    fn operator(&mut self, n: u8) -> Result<(), Fail> {
        while let Some(&top) = self.ops.last() {
            if matches!(top, Op::Func(..)) || Self::strength(top) < n {
                break;
            }
            self.ops.pop();
            self.emit(top)?;
        }
        match n {
            N_COMMA | N_COLON => Ok(()),
            N_END => {
                if self.out.is_empty() {
                    return Err(Fail);
                }
                self.append(0x00)
            }
            _ => self.push_op(Op::Entry(n)),
        }
    }

    fn push_op(&mut self, op: Op) -> Result<(), Fail> {
        if self.ops.len() >= OPS_LIMIT {
            return Err(Fail);
        }
        self.ops.push(op);
        Ok(())
    }

    /// Handles a closing `)`.
    fn close(&mut self) -> Result<(), Fail> {
        if self.ops.is_empty() {
            return Err(Fail);
        }
        loop {
            match self.ops.pop() {
                None => return Err(Fail),
                Some(Op::Entry(PAREN)) => return Ok(()),
                Some(f @ Op::Func(..)) => return self.emit(f),
                Some(op) => {
                    self.emit(op)?;
                    if self.ops.is_empty() {
                        return Err(Fail);
                    }
                }
            }
        }
    }

    fn run(&mut self) -> Result<(), Fail> {
        loop {
            match self.next() {
                Tok::Stop => break,
                Tok::Value(v, kind) => {
                    self.push_value(v, kind)?;
                    if kind == Kind::Parameter {
                        self.called = true;
                    }
                    self.pending = true;
                }
                Tok::Func(i, k) => {
                    self.push_op(Op::Func(i, k))?;
                    self.called = true;
                    self.pending = true;
                }
                Tok::LParen => self.push_op(Op::Entry(PAREN))?,
                Tok::RParen => self.close()?,
                Tok::Comma => self.operator(N_COMMA)?,
                Tok::Question => self.operator(OP_COND)?,
                Tok::Colon => self.operator(N_COLON)?,
                Tok::Plus => self.binary(0x10)?,
                Tok::Mul => self.binary(0x12)?,
                Tok::Div => self.binary(0x13)?,
                Tok::Pow => self.binary(0x14)?,
                Tok::Cmp(op) => self.binary(op)?,
                Tok::Minus => self.binary(if self.pending { 0x11 } else { OP_NEG })?,
            }
        }
        if self.ops.iter().any(|o| matches!(o, Op::Func(..))) {
            self.diags.push(CalcDiag::OpenFunction);
        }
        // End: Operator(0), then folding.
        self.operator(N_END)?;
        if !self.called {
            let v = eval_const(&self.out);
            self.out.clear();
            self.count = 0;
            self.push_value(v, Kind::Constant)?;
            self.append(0x00)?;
        }
        Ok(())
    }

    /// An operator token that clears `pending`.
    fn binary(&mut self, n: u8) -> Result<(), Fail> {
        self.operator(n)?;
        self.pending = false;
        Ok(())
    }
}

/// Compiles one formula cell (`text` = first 256 cell bytes) with the
/// rules of `family`. An empty `code` means no expression.
pub fn compile(family: Family, links: &dyn CalcLinks, text: &[u8]) -> Result<Compiled, CalcError> {
    compile_with(family, links, &NO_KEYWORDS, text)
}

/// [`compile`] with a mod game's keywords: a word followed by `(` that is no built-in
/// keyword of `family` but is in `registry` compiles as CALL to the registered function
/// index. Formulas that use no registered keyword compile exactly as [`compile`] does.
pub fn compile_with(
    family: Family,
    links: &dyn CalcLinks,
    registry: &CalcRegistry,
    text: &[u8],
) -> Result<Compiled, CalcError> {
    if text.iter().any(|&b| b >= 0x80) {
        return Err(CalcError::NonAscii);
    }
    let mut p = Parser {
        family,
        links,
        registry,
        text,
        pos: 0,
        out: Vec::new(),
        ops: Vec::new(),
        count: 0,
        pending: false,
        called: false,
        diags: Vec::new(),
    };
    let ok = p.run().is_ok();
    let mut diagnostics = p.diags;
    let code = if ok {
        p.out
    } else {
        diagnostics.push(CalcDiag::Fail);
        Vec::new()
    };
    if family == Family::Missiles && code_calls(&code, 2) {
        return Err(CalcError::MissileRand);
    }
    Ok(Compiled { code, diagnostics })
}

/// Operand size of an opcode.
fn operand_len(op: u8) -> usize {
    match op {
        0x01 | 0x04 | 0x07 => 1,
        0x05 | 0x08 => 2,
        0x06 | 0x09 => 4,
        _ => 0,
    }
}

/// Whether the expression `code` contains CALL `index`.
fn code_calls(code: &[u8], index: u8) -> bool {
    let mut i = 0;
    while i < code.len() {
        let op = code[i];
        if op == 0x01 && code.get(i + 1) == Some(&index) {
            return true;
        }
        i += 1 + operand_len(op);
    }
    false
}

/// Evaluates `code` with no parameter callback and no functions: the evaluation used
/// for constant folding when a formula has no names or calls.
pub fn eval_const(code: &[u8]) -> i32 {
    let mut stack: Vec<i32> = Vec::with_capacity(64);
    let push = |s: &mut Vec<i32>, v: i32| {
        if s.len() < 64 {
            s.push(v);
        }
    };
    let pop = |s: &mut Vec<i32>| s.pop().unwrap_or(0);
    let mut i = 0;
    while i < code.len() {
        let op = code[i];
        i += 1;
        let operand = &code[i..];
        match op {
            0x01 => {
                // No functions: index ≥ count pushes 0, pops nothing.
                i += 1;
                push(&mut stack, 0);
            }
            0x04..=0x06 => return 0,
            0x07..=0x09 => {
                if operand.is_empty() {
                    return 0;
                }
                let v = match op {
                    0x07 => i32::from(operand[0] as i8),
                    0x08 => i32::from(i16::from_le_bytes([
                        operand[0],
                        operand.get(1).copied().unwrap_or(0),
                    ])),
                    _ => {
                        let mut b = [0u8; 4];
                        for (k, x) in b.iter_mut().enumerate() {
                            *x = operand.get(k).copied().unwrap_or(0);
                        }
                        i32::from_le_bytes(b)
                    }
                };
                i += operand_len(op);
                push(&mut stack, v);
            }
            0x0A..=0x14 => {
                let b = pop(&mut stack);
                let a = pop(&mut stack);
                let v = match op {
                    0x0A => i32::from(a < b),
                    0x0B => i32::from(a > b),
                    0x0C => i32::from(a <= b),
                    0x0D => i32::from(a >= b),
                    0x0E => i32::from(a == b),
                    0x0F => i32::from(a != b),
                    0x10 => a.wrapping_add(b),
                    0x11 => a.wrapping_sub(b),
                    0x12 => a.wrapping_mul(b),
                    0x13 => {
                        if b == 0 {
                            0
                        } else {
                            a.wrapping_div(b)
                        }
                    }
                    _ => {
                        if b <= 0 {
                            1
                        } else {
                            a.wrapping_pow(b as u32)
                        }
                    }
                };
                push(&mut stack, v);
            }
            0x15 => {
                let v = pop(&mut stack);
                push(&mut stack, v.wrapping_neg());
            }
            0x16 => {
                let f = pop(&mut stack);
                let t = pop(&mut stack);
                let c = pop(&mut stack);
                push(&mut stack, if c != 0 { t } else { f });
            }
            _ => return pop(&mut stack),
        }
    }
    0
}

/// A code buffer rejected by [`validate_buffer`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BufferError {
    /// Opcode the compiler cannot emit, at this offset.
    BadOpcode { offset: usize, opcode: u8 },
    /// Operand or expression runs past the end, starting at this offset.
    Truncated { offset: usize },
    /// A formula field is neither 0xFFFFFFFF nor an expression start.
    BadField { value: u32 },
    /// Missile formula calling `rand`, at this offset.
    MissileRand { offset: usize },
}

/// Counts the validator collects while decoding a buffer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BufferReport {
    /// Expression start offsets.
    pub starts: Vec<usize>,
    /// Expressions no formula field points at.
    pub unreferenced: usize,
    /// Expressions pointed at by more than one formula field.
    pub shared: usize,
    /// Count of opcode 0x02.
    pub paren: usize,
    /// CALL indices without an evaluator entry.
    pub unknown_call: usize,
}

/// Decodes a buffer and checks its formula fields.
pub fn validate_buffer(
    family: Family,
    buffer: &[u8],
    fields: impl IntoIterator<Item = u32>,
) -> Result<BufferReport, BufferError> {
    let mut report = BufferReport::default();
    let mut i = 0;
    let mut start = 0;
    while i < buffer.len() {
        let op = buffer[i];
        if !matches!(op, 0x00 | 0x01 | 0x02 | 0x04..=0x16) {
            return Err(BufferError::BadOpcode {
                offset: i,
                opcode: op,
            });
        }
        let n = operand_len(op);
        if i + 1 + n > buffer.len() {
            return Err(BufferError::Truncated { offset: i });
        }
        match op {
            0x00 => {
                report.starts.push(start);
                start = i + 1;
            }
            0x01 => {
                let index = buffer[i + 1];
                if family == Family::Missiles && index == 2 {
                    return Err(BufferError::MissileRand { offset: i });
                }
                if index >= family.function_count() {
                    report.unknown_call += 1;
                }
            }
            0x02 => report.paren += 1,
            _ => {}
        }
        i += 1 + n;
    }
    if start != buffer.len() {
        return Err(BufferError::Truncated { offset: start });
    }
    let mut refs = vec![0usize; report.starts.len()];
    for value in fields {
        if value == u32::MAX {
            continue;
        }
        match report.starts.binary_search(&(value as usize)) {
            Ok(k) => refs[k] += 1,
            Err(_) => return Err(BufferError::BadField { value }),
        }
    }
    report.unreferenced = refs.iter().filter(|&&r| r == 0).count();
    report.shared = refs.iter().filter(|&&r| r > 1).count();
    Ok(report)
}

#[cfg(test)]
mod robust_tests;
#[cfg(test)]
mod tests;
