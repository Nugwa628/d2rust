//! The formula evaluator: runs a compiled formula (the bytecode the
//! compiler in `d2_data::calc` writes into a family's code buffer) for one
//! unit, skill, missile or item. The compiler and the constant evaluator
//! live in `d2_data::calc`; this module is the runtime half, where
//! parameters and functions need the game: a [`CalcContext`] supplies the
//! parameter callback and the family's function table (skills and
//! missiles contexts: [`super::levels`]; items: the potion desk).
//!
//! A mod game's formula functions (MS9) are
//! calls past the family's own table that the game's
//! [`crate::rules::CalcProvider`] registered; the evaluator hands them to
//! it. Every other call runs exactly as D2RUST's own rules say.
//!
//! Integer rules: arithmetic wraps, division by 0 gives 0, and
//! −2³¹ / −1 gives −2³¹ instead of faulting.

use d2_data::calc::Family;

use crate::rules::Ruleset;

/// The context behind one evaluation: the parameter callback and the
/// family's function table.
pub trait CalcContext {
    /// Size of the family's own function table (skills 7, missiles and
    /// items 4).
    fn function_count(&self) -> u8;
    /// Arity of function `index` (< [`CalcContext::function_count`]).
    fn arity(&self, index: u8) -> u8;
    /// `param(c)`: the value of parameter `c`, the operand as the push
    /// opcode sign-extends it.
    fn param(&mut self, c: i32) -> i32;
    /// Calls function `index` with `args` in source order.
    fn call(&mut self, index: u8, args: &[i32]) -> i32;
    /// The family this context evaluates and the game's rules, when the
    /// game runs a mod game's formula functions (see [`mod_calc`]). The
    /// default, `None`, is D2RUST's functions only.
    fn mod_calc(&self) -> Option<(Family, Ruleset)> {
        None
    }
}

/// [`CalcContext::mod_calc`] for a context of `family` in a game under
/// `rules`: `None` unless the rules carry formula functions.
pub fn mod_calc(rules: &Ruleset, family: Family) -> Option<(Family, Ruleset)> {
    rules.calc()?;
    Some((family, rules.clone()))
}

/// Stack depth: pushes past it are dropped.
const STACK: usize = 64;

fn push(s: &mut Vec<i32>, v: i32) {
    if s.len() < STACK {
        s.push(v);
    }
}

/// Pops a value; an empty stack gives 0.
fn pop(s: &mut Vec<i32>) -> i32 {
    s.pop().unwrap_or(0)
}

/// A call past the family's own table: the mod game's function `index`
/// takes its arguments off the stack and returns its value. `None` (no
/// mod function there) leaves the stack alone.
fn mod_function(ctx: &mut dyn CalcContext, index: u8, stack: &mut Vec<i32>) -> Option<i32> {
    let (family, rules) = ctx.mod_calc()?;
    let calc = rules.calc()?;
    let k = calc.registry().function(family, index)?.arity;
    let k = usize::from(k.min(d2_data::calc::MAX_ARITY));
    let mut args = [0i32; 3];
    for a in args[..k].iter_mut().rev() {
        *a = pop(stack);
    }
    Some(calc.call(family, index, &args[..k], ctx))
}

/// Evaluates the formula starting at `offset` in a family code buffer:
/// 0 when `offset` is at or past the end of `buffer` (0xFFFFFFFF, "no
/// formula", always is), else [`run`] from `offset`.
pub fn eval(buffer: &[u8], offset: u32, ctx: &mut dyn CalcContext) -> i32 {
    match usize::try_from(offset) {
        Ok(o) if o < buffer.len() => run(&buffer[o..], ctx),
        _ => 0,
    }
}

/// Runs the stack machine over `code` (the bytes from the expression
/// start to the buffer end) until the end opcode, which returns the top
/// of the stack. An opcode the compiler never writes ends the run the
/// same way.
pub fn run(code: &[u8], ctx: &mut dyn CalcContext) -> i32 {
    let mut stack: Vec<i32> = Vec::with_capacity(STACK);
    let mut i = 0;
    while i < code.len() {
        let op = code[i];
        i += 1;
        let len = match op {
            0x01 | 0x04 | 0x07 => 1,
            0x05 | 0x08 => 2,
            0x06 | 0x09 => 4,
            _ => 0,
        };
        if i + len > code.len() {
            // A cut operand stops with result 0.
            return 0;
        }
        let o = &code[i..i + len];
        i += len;
        match op {
            0x01 => {
                let index = o[0];
                if index >= ctx.function_count() {
                    // Past the family's own table: a mod game's function,
                    // else 0 with no arguments taken.
                    let v = mod_function(ctx, index, &mut stack).unwrap_or(0);
                    push(&mut stack, v);
                    continue;
                }
                let k = ctx.arity(index);
                if k > 3 {
                    push(&mut stack, 0);
                    continue;
                }
                let mut args = [0i32; 3];
                for a in args[..usize::from(k)].iter_mut().rev() {
                    *a = pop(&mut stack);
                }
                let v = ctx.call(index, &args[..usize::from(k)]);
                push(&mut stack, v);
            }
            0x04 => {
                let v = ctx.param(i32::from(o[0]));
                push(&mut stack, v);
            }
            0x05 => {
                let v = ctx.param(i32::from(i16::from_le_bytes([o[0], o[1]])));
                push(&mut stack, v);
            }
            0x06 => {
                let v = ctx.param(i32::from_le_bytes([o[0], o[1], o[2], o[3]]));
                push(&mut stack, v);
            }
            0x07 => push(&mut stack, i32::from(o[0] as i8)),
            0x08 => push(&mut stack, i32::from(i16::from_le_bytes([o[0], o[1]]))),
            0x09 => push(&mut stack, i32::from_le_bytes([o[0], o[1], o[2], o[3]])),
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

/// `rand(a, b)`: `a ≥ b` → `a`, no draw; else `a +
/// R(b − a + 1)` with `R` = `roll` (an `n` that wrapped below 1 gives 0
/// with no step).
pub fn rand(seed: &mut crate::rng::Seed, a: i32, b: i32) -> i32 {
    if a >= b {
        return a;
    }
    let n = b.wrapping_sub(a).wrapping_add(1);
    a.wrapping_add(seed.roll(n) as i32)
}

#[cfg(test)]
mod tests {
    // Test vectors: "Evaluator" and
    // "Randomness".
    use super::*;
    use crate::rng::Seed;

    /// The stub context: `param(c)` = c; `min`, `max` real;
    /// `skill(s, c)` = 100·s + c; `sklvl(s, a, b)` = 10,000·s + 100·a + b.
    struct Stub;
    impl CalcContext for Stub {
        fn function_count(&self) -> u8 {
            7
        }
        fn arity(&self, index: u8) -> u8 {
            if index == 6 {
                3
            } else {
                2
            }
        }
        fn param(&mut self, c: i32) -> i32 {
            c
        }
        fn call(&mut self, index: u8, a: &[i32]) -> i32 {
            match index {
                0 => a[0].min(a[1]),
                1 => a[0].max(a[1]),
                3 => 100 * a[0] + a[1],
                6 => 10_000 * a[0] + 100 * a[1] + a[2],
                _ => 0,
            }
        }
    }

    fn ev(bytes: &[u8]) -> i32 {
        eval(bytes, 0, &mut Stub)
    }

    #[test]
    fn evaluator_vectors() {
        assert_eq!(ev(&[0x04, 0x10, 0x07, 0x02, 0x12, 0x00]), 32);
        assert_eq!(
            ev(&[0x07, 0x66, 0x07, 0x04, 0x07, 0x11, 0x01, 0x06, 0x00]),
            1_020_417
        );
        assert_eq!(
            ev(&[0x04, 0x02, 0x08, 0x96, 0x00, 0x01, 0x00, 0x15, 0x00]),
            -2
        );
        assert_eq!(ev(&[0x07, 0x05, 0x07, 0x00, 0x13, 0x00]), 0);
        assert_eq!(ev(&[0x07, 0xF9, 0x07, 0x02, 0x13, 0x00]), -3);
        assert_eq!(ev(&[0x07, 0x07, 0x07, 0xFE, 0x13, 0x00]), -3);
        assert_eq!(ev(&[0x07, 0xFE, 0x07, 0x03, 0x14, 0x00]), -8);
        assert_eq!(ev(&[0x07, 0x02, 0x07, 0x00, 0x14, 0x00]), 1);
        assert_eq!(ev(&[0x07, 0x02, 0x07, 0xFF, 0x14, 0x00]), 1);
        assert_eq!(ev(&[0x09, 0x00, 0x00, 0x00, 0x80, 0x15, 0x00]), i32::MIN);
        assert_eq!(ev(&[0x07, 0x05, 0x07, 0x07, 0x11, 0x00]), -2);
        assert_eq!(ev(&[0x06, 0x00, 0x00, 0x01, 0x00, 0x00]), 65_536);
        assert_eq!(ev(&[0x07, 0x03, 0x07, 0x64, 0x14, 0x00]), -818_408_495);
        assert_eq!(ev(&[0x09, 0, 0, 1, 0, 0x09, 0, 0, 1, 0, 0x12, 0x00]), 0);
        assert_eq!(
            ev(&[0x09, 0x00, 0x00, 0x00, 0x80, 0x07, 0xFF, 0x13, 0x00]),
            i32::MIN
        );
        assert_eq!(ev(&[0x10, 0x00]), 0);
        assert_eq!(ev(&[0x07, 0x05, 0x07, 0x06, 0x16, 0x00]), 6);
        assert_eq!(ev(&[0x07, 0x00, 0x07, 0x01, 0x16, 0x07, 0x05, 0x00]), 5);
        assert_eq!(ev(&[0x07, 0x05, 0x07, 0x06, 0x01, 0x09, 0x00]), 0);
        assert_eq!(ev(&[0x04, 0xFF, 0x00]), 255);
        assert_eq!(ev(&[0x05, 0xFF, 0xFF, 0x00]), -1);
        assert_eq!(ev(&[0x07, 0xFF, 0x00]), -1);
        for stop in [0x03, 0x17, 0xFF] {
            assert_eq!(ev(&[0x07, 0x05, stop, 0x07, 0x06, 0x00]), 5);
        }
        assert_eq!(ev(&[0x07, 0x05, 0x02, 0x07, 0x06, 0x10, 0x00]), 5);
        for cut in [&[0x07][..], &[0x08, 0x05], &[0x01], &[0x07, 0x05]] {
            assert_eq!(ev(cut), 0, "{cut:02x?}");
        }
        let mut full = Vec::new();
        for _ in 0..64 {
            full.extend([0x07, 0x01]);
        }
        full.extend([0x07, 0x02, 0x00]);
        assert_eq!(ev(&full), 1);
        assert_eq!(eval(&[0x07, 0x05, 0x00], 0xFFFF_FFFF, &mut Stub), 0);
        assert_eq!(eval(&[0x07, 0x05, 0x00], 3, &mut Stub), 0);
    }

    #[test]
    fn comparisons() {
        // (a, b) = (−1, 1), (2, 2), (3, 2) per op 0x0A..0x0F.
        let want = [
            [1, 0, 0],
            [0, 0, 1],
            [1, 1, 0],
            [0, 1, 1],
            [0, 1, 0],
            [1, 0, 1],
        ];
        for (k, w) in want.iter().enumerate() {
            let op = 0x0A + k as u8;
            for (j, (a, b)) in [(0xFF, 1), (2, 2), (3, 2)].into_iter().enumerate() {
                assert_eq!(
                    ev(&[0x07, a, 0x07, b, op, 0x00]),
                    w[j],
                    "op {op:#x} case {j}"
                );
            }
        }
    }

    #[test]
    fn rand_vectors() {
        let mut s = Seed::new(1, 0);
        assert_eq!(rand(&mut s, 1, 6), 4);
        assert_eq!(s, Seed::new(0x6AC6_90C5, 0));
        let mut s = Seed::new(1, 0);
        assert_eq!(rand(&mut s, 0, 7), 5);
        let mut s = Seed::new(1, 0);
        assert_eq!(rand(&mut s, 5, 5), 5);
        assert_eq!(rand(&mut s, -2, i32::MAX), -2);
        assert_eq!(s, Seed::new(1, 0));
    }

    /// A mod game's formula functions: `twice(a)` = 2a (skills index 7),
    /// `low(a, b)` = D2RUST's own `min` through the context (8), `seven`
    /// = parameter 7 (9).
    #[derive(Debug)]
    struct Calc(d2_data::calc::CalcRegistry);

    impl crate::rules::CalcProvider for Calc {
        fn registry(&self) -> &d2_data::calc::CalcRegistry {
            &self.0
        }
        fn call(&self, family: Family, index: u8, a: &[i32], ctx: &mut dyn CalcContext) -> i32 {
            assert_eq!(family, Family::Skills);
            match index {
                7 => a[0] * 2,
                8 => ctx.call(0, a),
                9 => ctx.param(7),
                _ => unreachable!("only registered functions reach the provider"),
            }
        }
    }

    #[derive(Debug)]
    struct ModGame(Calc);

    impl crate::rules::ModRules for ModGame {
        fn name(&self) -> &str {
            "calc test"
        }
        fn calc(&self) -> Option<&dyn crate::rules::CalcProvider> {
            Some(&self.0)
        }
    }

    fn mod_rules() -> Ruleset {
        use d2_data::calc::ModKeyword;
        let mut r = d2_data::calc::CalcRegistry::new();
        for (name, arity) in [("twice", 1), ("low", 2), ("seven", 0)] {
            let kw = ModKeyword {
                name: name.into(),
                arity,
                names_like: None,
            };
            r.add(Family::Skills, kw).unwrap();
        }
        Ruleset::Mod(std::sync::Arc::new(ModGame(Calc(r))))
    }

    /// [`Stub`] in a game under `rules`.
    struct InGame(Ruleset);
    impl CalcContext for InGame {
        fn function_count(&self) -> u8 {
            Stub.function_count()
        }
        fn arity(&self, index: u8) -> u8 {
            Stub.arity(index)
        }
        fn param(&mut self, c: i32) -> i32 {
            Stub.param(c)
        }
        fn call(&mut self, index: u8, args: &[i32]) -> i32 {
            Stub.call(index, args)
        }
        fn mod_calc(&self) -> Option<(Family, Ruleset)> {
            mod_calc(&self.0, Family::Skills)
        }
    }

    /// Links for the compiled vectors: no names resolve.
    struct NoLinks;
    impl d2_data::calc::CalcLinks for NoLinks {
        fn skill(&self, _: &[u8]) -> Option<u32> {
            None
        }
        fn missile(&self, _: &[u8]) -> Option<u32> {
            None
        }
        fn stat(&self, _: &[u8]) -> Option<u32> {
            None
        }
        fn skillcalc(&self, _: u32) -> Option<u32> {
            None
        }
        fn misscalc(&self, _: u32) -> Option<u32> {
            None
        }
    }

    #[test]
    fn mod_functions_compile_and_evaluate() {
        let rules = mod_rules();
        let registry = rules.calc().unwrap().registry();
        let code = |text: &str| {
            d2_data::calc::compile_with(Family::Skills, &NoLinks, registry, text.as_bytes())
                .unwrap()
                .code
        };
        let ev_mod = |text: &str| eval(&code(text), 0, &mut InGame(mod_rules()));
        assert_eq!(code("twice(5)+1"), [0x07, 5, 0x01, 7, 0x07, 1, 0x10, 0x00]);
        assert_eq!(ev_mod("twice(5)+1"), 11);
        assert_eq!(ev_mod("low(9, 4) * 10"), 40);
        assert_eq!(ev_mod("seven() + twice(min(3, 8))"), 13);
        assert_eq!(ev_mod("twice(twice(low(2, 3)))"), 8);
    }

    #[test]
    fn calls_past_the_table_without_a_mod_function_stay_d2rusts() {
        // D2RUST's own rules, and a mod game with nothing registered at
        // the index: 0 pushed, no arguments taken (5 stays under it).
        let bytes = [0x07, 0x05, 0x07, 0x06, 0x01, 0x0A, 0x10, 0x00];
        assert_eq!(ev(&bytes), 6);
        assert_eq!(eval(&bytes, 0, &mut InGame(Ruleset::Original)), 6);
        assert_eq!(eval(&bytes, 0, &mut InGame(mod_rules())), 6);
        // With a mod function at 7, the same call shape takes its argument.
        let bytes = [0x07, 0x05, 0x07, 0x06, 0x01, 0x07, 0x10, 0x00];
        assert_eq!(ev(&bytes), 6);
        assert_eq!(eval(&bytes, 0, &mut InGame(mod_rules())), 17);
        assert!(mod_calc(&Ruleset::Original, Family::Skills).is_none());
    }
}
