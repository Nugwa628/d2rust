//! The wide formatter: the D2Lang
//! `swprintf` that UI code uses with string-table formats. `%d`, `%u`,
//! `%s` and `%%` only; `%%` consumes an argument slot like every other
//! conversion.

/// One argument of [`format`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arg<'a> {
    Int(i32),
    UInt(u32),
    Str(&'a [u16]),
    /// A null `%s` pointer: an error unless the buffer has no room left.
    NullStr,
}

/// What the game does fatally.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FormatError {
    #[error("unsupported conversion %{0:#06x}")]
    BadConversion(u16),
    #[error("conversion {0} has no argument")]
    MissingArg(usize),
    #[error("conversion {0}: argument is not what the conversion reads")]
    WrongArg(usize),
    #[error("null %s pointer dereferenced")]
    NullString,
}

fn units(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// `format(max, dest, fmt, args…)`: the formatted units (without the
/// terminator). `max` counts UTF-16 units including the terminator.
/// `fmt` `None` gives an empty result. Rule 4's unterminated
/// return is replaced by ending at the current position.
pub fn format(max: usize, fmt: Option<&[u16]>, args: &[Arg<'_>]) -> Result<Vec<u16>, FormatError> {
    let mut out: Vec<u16> = Vec::new();
    let Some(fmt) = fmt else { return Ok(out) };
    let mut i = 0usize;
    let mut next_arg = 0usize;
    loop {
        // r1: literal run up to the next `%` (or the end of fmt).
        let lit_end = fmt[i..]
            .iter()
            .position(|&u| u == u16::from(b'%'))
            .map_or(fmt.len(), |p| i + p);
        out.extend_from_slice(&fmt[i..lit_end]);
        if out.len() >= max {
            out.truncate(max.saturating_sub(1));
            return Ok(out);
        }
        if lit_end >= fmt.len() {
            return Ok(out);
        }
        // r2: the unit after `%` selects the conversion.
        let conv = fmt.get(lit_end + 1).copied().unwrap_or(0);
        let arg = args.get(next_arg).copied();
        let slot = next_arg;
        next_arg += 1;
        let fits = |len: usize, count: usize| len + count + 1 < max;
        match conv {
            0 => {
                // a lone `%` at the end: one `%`, terminated, return
                if out.len() + 1 < max {
                    out.push(u16::from(b'%'));
                }
                return Ok(out);
            }
            c if c == u16::from(b'd') || c == u16::from(b'u') => {
                let text = match (c == u16::from(b'd'), arg) {
                    (true, Some(Arg::Int(v))) => units(&v.to_string()),
                    (false, Some(Arg::UInt(v))) => units(&v.to_string()),
                    (_, None) => return Err(FormatError::MissingArg(slot)),
                    _ => return Err(FormatError::WrongArg(slot)),
                };
                // at most 15 units; r4: a number that does not fit ends
                // formatting
                if !fits(text.len(), out.len()) {
                    return Ok(out);
                }
                out.extend_from_slice(&text);
            }
            c if c == u16::from(b's') => {
                let s = match arg {
                    Some(Arg::Str(s)) => s,
                    Some(Arg::NullStr) => {
                        if max.saturating_sub(out.len() + 1) == 0 {
                            return Ok(out);
                        }
                        return Err(FormatError::NullString);
                    }
                    None => return Err(FormatError::MissingArg(slot)),
                    _ => return Err(FormatError::WrongArg(slot)),
                };
                let s = &s[..s.iter().position(|&u| u == 0).unwrap_or(s.len())];
                if s.is_empty() || !fits(s.len(), out.len()) {
                    // r5: bounded concatenation, then formatting ends
                    let room = max.saturating_sub(out.len() + 1);
                    out.extend_from_slice(&s[..s.len().min(room)]);
                    return Ok(out);
                }
                out.extend_from_slice(s);
            }
            c if c == u16::from(b'%') => {
                // the next argument is consumed (a missing one is the
                // never-pushed slot after the list)
                if !fits(1, out.len()) {
                    return Ok(out);
                }
                out.push(u16::from(b'%'));
            }
            other => return Err(FormatError::BadConversion(other)),
        }
        i = lit_end + 2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }
    fn f(max: usize, fmt: &str, args: &[Arg<'_>]) -> Result<String, FormatError> {
        let fmt = u(fmt);
        format(max, Some(&fmt), args).map(|v| String::from_utf16(&v).unwrap())
    }

    #[test]
    fn block_chance_line() {
        let name = u("Zombie");
        // (block, chance, name, chance); each `%%` consumes a slot.
        let s = f(
            200,
            "Chance to Block: %d%%\nAverage chance %s will hit you: %d%%",
            &[Arg::Int(30), Arg::Int(77), Arg::Str(&name), Arg::Int(45)],
        );
        assert_eq!(
            s.unwrap(),
            "Chance to Block: 30%\nAverage chance Zombie will hit you: 45%"
        );
        // (name, chance) with the second text
        let s = f(
            200,
            "Average chance %s will hit you: %d%%",
            &[Arg::Str(&name), Arg::Int(45)],
        );
        assert_eq!(s.unwrap(), "Average chance Zombie will hit you: 45%");
        // %u unsigned, %d signed
        assert_eq!(
            f(50, "%u/%d", &[Arg::UInt(4_000_000_000), Arg::Int(-7)]).unwrap(),
            "4000000000/-7"
        );
        // null fmt: destination untouched/empty
        assert_eq!(format(10, None, &[]).unwrap(), Vec::<u16>::new());
        // lone `%` at the end
        assert_eq!(f(10, "50%", &[]).unwrap(), "50%");
        // anything else is fatal
        assert_eq!(
            f(10, "%x", &[Arg::Int(1)]),
            Err(FormatError::BadConversion(u16::from(b'x')))
        );
        assert_eq!(f(10, "%d", &[Arg::UInt(1)]), Err(FormatError::WrongArg(0)));
    }

    #[test]
    fn overflow_ends_formatting() {
        // literal overflow: terminated at max - 1 units
        assert_eq!(f(4, "abcdef%d", &[Arg::Int(1)]).unwrap(), "abc");
        // a number that does not fit (len + count + 1 >= max) ends it
        assert_eq!(f(6, "ab%d!", &[Arg::Int(123)]).unwrap(), "ab");
        assert_eq!(f(7, "ab%d!", &[Arg::Int(123)]).unwrap(), "ab123!");
        // `%%` that does not fit
        assert_eq!(f(3, "ab%%c", &[Arg::Int(0)]).unwrap(), "ab");
        // %s that does not fit: bounded concatenation, rest dropped
        let s = u("WXYZ");
        assert_eq!(f(6, "ab%sc", &[Arg::Str(&s)]).unwrap(), "abWXY");
        // %s with an empty string ends formatting too
        let e = u("");
        assert_eq!(f(20, "ab%scd", &[Arg::Str(&e)]).unwrap(), "ab");
        // a null %s is a fault unless no room is left
        assert_eq!(f(20, "ab%s", &[Arg::NullStr]), Err(FormatError::NullString));
        assert_eq!(f(3, "ab%s", &[Arg::NullStr]).unwrap(), "ab");
    }
}
