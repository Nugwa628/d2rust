//! The check host's `Diablo II` registry values as a check input (`play
//! --registry FILE`): the game reads them at start (`Mini Panel`, `Help Menu`, the
//! Options rows,...), so a check run replays the values its host had,
//! like its clock. Live play has no registry (`settings.toml` stands in for the
//! Options rows).
//!
//! The read: HKCU, then HKLM; a REG_DWORD
//! is the value, a REG_SZ is parsed with `strtoul(s, NULL, 0)`, any other
//! type is found but leaves the caller's variable as it was. D2RUST never
//! guesses that variable: a value of another type that D2RUST reads is an
//! error.

use super::config::{Settings, INT_KEYS};

/// The `--registry` file format version (`# registry 1`).
pub const REGISTRY_VERSION: u32 = 1;

/// One recorded value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegValue {
    /// REG_DWORD.
    Dword(u32),
    /// REG_SZ (the text before the terminator).
    Sz(String),
    /// Any other type: the registry type number and the bytes.
    Other(u32, Vec<u8>),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RegistryError {
    #[error("registry file line {0}: {1}")]
    Parse(usize, String),
    #[error("registry value `{0}` has type {1} (neither REG_DWORD nor REG_SZ): the registry reader leaves the value unset, which D2RUST does not guess")]
    Kind(String, u32),
    #[error("registry value `{0}` = {1} is outside {2}..={3}")]
    Range(String, u32, i64, i64),
}

/// The recorded `Diablo II` key: HKCU and HKLM values, in file order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HostRegistry {
    pub hkcu: Vec<(String, RegValue)>,
    pub hklm: Vec<(String, RegValue)>,
}

/// The registry names of the [`INT_KEYS`] rows, same order.
pub const INT_KEY_NAMES: [&str; 18] = [
    "Master Volume",
    "Music Volume",
    "Sound Mixer",
    "Positional Bias",
    "NPC Speech",
    "Resolution",
    "Light Quality",
    "Blended Shadows",
    "Perspective",
    "Gamma",
    "Contrast",
    "AutoMapMode",
    "AutoMapFade",
    "AutoMap Centers",
    "AutoMap Party",
    "AutoMap Party Names",
    "Show HP Text",
    "Show MP Text",
];

/// `strtoul(s, NULL, 0)` of the C runtime: leading white space, an
/// optional sign, base 16 after `0x` / `0X`, base 8 after `0`, else 10;
/// digits up to the first one outside the base; overflow gives
/// 0xFFFFFFFF; a minus sign negates (mod 2^32); no digits gives 0.
pub fn strtoul(s: &str) -> u32 {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\n' | b'\r' | 0x0B | 0x0C) {
        i += 1;
    }
    let mut neg = false;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        neg = b[i] == b'-';
        i += 1;
    }
    let digit = |c: u8| (c as char).to_digit(36);
    let base = if b.get(i) == Some(&b'0') {
        if matches!(b.get(i + 1), Some(b'x' | b'X'))
            && b.get(i + 2).and_then(|&c| digit(c)).is_some_and(|d| d < 16)
        {
            i += 2;
            16
        } else {
            8
        }
    } else {
        10
    };
    let mut v: u64 = 0;
    let mut over = false;
    while let Some(d) = b.get(i).and_then(|&c| digit(c)).filter(|&d| d < base) {
        v = v * u64::from(base) + u64::from(d);
        if v > u64::from(u32::MAX) {
            over = true;
            v = u64::from(u32::MAX);
        }
        i += 1;
    }
    if over {
        return u32::MAX;
    }
    let v = v as u32;
    if neg {
        v.wrapping_neg()
    } else {
        v
    }
}

fn unescape(s: &str, line: usize) -> Result<String, RegistryError> {
    let mut out = String::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('\\') => out.push('\\'),
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            o => {
                return Err(RegistryError::Parse(
                    line,
                    format!("bad escape \\{}", o.map(String::from).unwrap_or_default()),
                ))
            }
        }
    }
    Ok(out)
}

fn hex_bytes(s: &str, line: usize) -> Result<Vec<u8>, RegistryError> {
    let bad = || RegistryError::Parse(line, format!("bad hex `{s}`"));
    if !s.len().is_multiple_of(2) {
        return Err(bad());
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|_| bad()))
        .collect()
}

impl HostRegistry {
    /// Parse a `registry 1` file: `# registry 1`, the header `scope name
    /// kind value`, then one tab-separated row per value: scope `hkcu` /
    /// `hklm`, the value name, kind `dword` (decimal) / `sz` (text with
    /// `\\`, `\t`, `\n`, `\r` escaped) / `type<N>` (hex bytes). Strict:
    /// anything else is an error.
    pub fn parse(text: &str) -> Result<Self, RegistryError> {
        let mut lines = text.lines().enumerate().map(|(i, l)| (i + 1, l));
        let want = format!("# registry {REGISTRY_VERSION}");
        match lines.next() {
            Some((_, l)) if l == want => {}
            _ => return Err(RegistryError::Parse(1, format!("expected `{want}`"))),
        }
        match lines.next() {
            Some((_, "scope\tname\tkind\tvalue")) => {}
            _ => {
                return Err(RegistryError::Parse(
                    2,
                    "expected the header `scope\tname\tkind\tvalue`".into(),
                ))
            }
        }
        let mut r = HostRegistry::default();
        for (n, l) in lines {
            if l.is_empty() {
                continue;
            }
            let f: Vec<&str> = l.split('\t').collect();
            let [scope, name, kind, value] = f[..] else {
                return Err(RegistryError::Parse(n, "expected 4 fields".into()));
            };
            let v = match kind {
                "dword" => RegValue::Dword(
                    value
                        .parse()
                        .map_err(|_| RegistryError::Parse(n, format!("bad dword `{value}`")))?,
                ),
                "sz" => RegValue::Sz(unescape(value, n)?),
                k => match k.strip_prefix("type").and_then(|t| t.parse().ok()) {
                    Some(t) => RegValue::Other(t, hex_bytes(value, n)?),
                    None => return Err(RegistryError::Parse(n, format!("bad kind `{k}`"))),
                },
            };
            let name = unescape(name, n)?;
            match scope {
                "hkcu" => r.hkcu.push((name, v)),
                "hklm" => r.hklm.push((name, v)),
                s => return Err(RegistryError::Parse(n, format!("bad scope `{s}`"))),
            }
        }
        Ok(r)
    }

    fn find(&self, name: &str) -> Option<&RegValue> {
        // Value names are case-insensitive; HKCU first.
        fn hit<'a>(list: &'a [(String, RegValue)], name: &str) -> Option<&'a RegValue> {
            list.iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(name))
                .map(|(_, v)| v)
        }
        hit(&self.hkcu, name).or_else(|| hit(&self.hklm, name))
    }

    /// The read of `Diablo II\<name>`: `None` when absent.
    pub fn read(&self, name: &str) -> Result<Option<u32>, RegistryError> {
        match self.find(name) {
            None => Ok(None),
            Some(RegValue::Dword(v)) => Ok(Some(*v)),
            Some(RegValue::Sz(s)) => Ok(Some(strtoul(s))),
            Some(RegValue::Other(t, _)) => Err(RegistryError::Kind(name.into(), *t)),
        }
    }

    /// The Options rows (the registry value read over the default). D2RUST's
    /// own: a value outside the row's range is an error (the rows' own clamps
    /// are not modelled).
    pub fn apply_settings(&self, s: &mut Settings) -> Result<(), RegistryError> {
        for (i, (name, (_, _, lo, hi))) in INT_KEY_NAMES.iter().zip(INT_KEYS).enumerate() {
            if let Some(v) = self.read(name)? {
                if !(lo..=hi).contains(&i64::from(v)) {
                    return Err(RegistryError::Range((*name).into(), v, lo, hi));
                }
                s.set(i, i64::from(v));
            }
        }
        Ok(())
    }

    /// The values the in-game UI reads.
    pub fn ui_values(&self) -> Result<UiRegistry, RegistryError> {
        Ok(UiRegistry {
            mini_panel: self.read("Mini Panel")?,
            help_menu: self.read("Help Menu")?,
            popup_hireling: self.read("PopupHireling")?,
        })
    }
}

/// The registry values the in-game UI reads at start; `None` = absent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UiRegistry {
    pub mini_panel: Option<u32>,
    pub help_menu: Option<u32>,
    pub popup_hireling: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEAD: &str = "# registry 1\nscope\tname\tkind\tvalue\n";

    #[test]
    fn strtoul_follows_the_c_runtime() {
        assert_eq!(strtoul("1"), 1);
        assert_eq!(strtoul("  42abc"), 42);
        assert_eq!(strtoul("0x1F"), 31);
        assert_eq!(strtoul("010"), 8);
        assert_eq!(strtoul("09"), 0);
        assert_eq!(strtoul("0x"), 0);
        assert_eq!(strtoul("-1"), u32::MAX);
        assert_eq!(strtoul("99999999999"), u32::MAX);
        assert_eq!(strtoul(""), 0);
        assert_eq!(strtoul("+7"), 7);
    }

    #[test]
    fn hkcu_wins_and_names_ignore_case() {
        let r = HostRegistry::parse(&format!(
            "{HEAD}hklm\tMini Panel\tdword\t0\nhkcu\tmini panel\tsz\t1\nhklm\tHelp Menu\tdword\t1\n"
        ))
        .unwrap();
        assert_eq!(r.read("Mini Panel"), Ok(Some(1)));
        assert_eq!(r.read("Help Menu"), Ok(Some(1)));
        assert_eq!(r.read("PopupHireling"), Ok(None));
    }

    #[test]
    fn other_types_are_an_error_only_when_read() {
        let r = HostRegistry::parse(&format!("{HEAD}hkcu\tMini Panel\ttype3\t0100\n")).unwrap();
        assert_eq!(r.hkcu[0].1, RegValue::Other(3, vec![1, 0]));
        assert!(matches!(
            r.read("Mini Panel"),
            Err(RegistryError::Kind(_, 3))
        ));
        assert_eq!(r.read("Help Menu"), Ok(None));
    }

    #[test]
    fn strict_format() {
        assert!(HostRegistry::parse("scope\tname\tkind\tvalue\n").is_err());
        assert!(HostRegistry::parse(&format!("{HEAD}hkcr\ta\tdword\t1\n")).is_err());
        assert!(HostRegistry::parse(&format!("{HEAD}hkcu\ta\tqword\t1\n")).is_err());
        assert!(HostRegistry::parse(&format!("{HEAD}hkcu\ta\tdword\n")).is_err());
        assert!(HostRegistry::parse(&format!("{HEAD}hkcu\ta\tsz\tx\\q\n")).is_err());
        let r = HostRegistry::parse(&format!("{HEAD}hkcu\tCmdLine\tsz\t-w\\t-ns\\\\\n")).unwrap();
        assert_eq!(r.hkcu[0].1, RegValue::Sz("-w\t-ns\\".into()));
    }

    #[test]
    fn options_rows_take_the_registry_values() {
        let r = HostRegistry::parse(&format!(
            "{HEAD}hkcu\tContrast\tdword\t100\nhkcu\tAutoMapFade\tdword\t2\nhkcu\tShow HP Text\tsz\t1\n"
        ))
        .unwrap();
        let mut s = Settings::default();
        r.apply_settings(&mut s).unwrap();
        assert_eq!((s.contrast, s.automap_fade, s.show_hp_text), (100, 2, 1));
        let bad = HostRegistry::parse(&format!("{HEAD}hkcu\tGamma\tdword\t20\n")).unwrap();
        assert!(matches!(
            bad.apply_settings(&mut Settings::default()),
            Err(RegistryError::Range(..))
        ));
    }

    #[test]
    fn ui_values() {
        let r = HostRegistry::parse(&format!(
            "{HEAD}hkcu\tMini Panel\tdword\t1\nhkcu\tPopupHireling\tdword\t1\n"
        ))
        .unwrap();
        assert_eq!(
            r.ui_values(),
            Ok(UiRegistry {
                mini_panel: Some(1),
                help_menu: None,
                popup_hireling: Some(1),
            })
        );
    }
}
