//! The calc registry: function keywords a mod game adds to the formula language
//! (`docs/plans/mods/seams.md` MS9).
//!
//! Each family (skills, missiles, items) has its own built-in keywords (`min`, `max`,
//! `rand`, `skill`, …), compiled as CALL with the keyword's function index. A mod game
//! registers more keywords per family here; they get the function indices right after
//! the family's built-ins, in the order they were added. The compiler reads the registry
//! only when a word followed by `(` is not a built-in keyword, so formulas that use no
//! mod keyword compile to the same bytes as with an empty registry, and D2RUST's own
//! loader always passes an empty one.
//!
//! The registry holds names and arities only. What a mod function returns is the mod
//! game's evaluator hook in `d2-sim` (`ModRules::calc`), which reads the same registry.

use super::{Family, Keyword};

/// Largest argument count the evaluator passes to a function.
pub const MAX_ARITY: u8 = 3;

/// A function keyword a mod game adds to one family.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModKeyword {
    /// The keyword as written in formula text: ASCII letters and digits, starting with
    /// a letter, matched without regard to case.
    pub name: String,
    /// Number of arguments, at most [`MAX_ARITY`].
    pub arity: u8,
    /// Names inside the call resolve as they do inside this built-in keyword (`skill`:
    /// a skill name or a skill parameter code; `stat`: a stat name or `base`/`mod`);
    /// `None`: as a bare name in the family.
    pub names_like: Option<Keyword>,
}

/// Why a keyword could not be registered.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistryError {
    /// The name is empty, does not start with a letter, or holds a byte other than an
    /// ASCII letter or digit (the formula tokenizer could never read it as one word).
    #[error("calc keyword {0:?}: not a word of ASCII letters and digits")]
    BadName(String),
    /// The family already has a keyword with this name (built-in or registered).
    #[error("calc keyword {0:?}: already a keyword of the family")]
    Taken(String),
    /// More arguments than the evaluator passes.
    #[error("calc keyword {name:?}: arity {arity} above {MAX_ARITY}")]
    Arity {
        /// The keyword.
        name: String,
        /// The arity asked for.
        arity: u8,
    },
    /// The family's function indices (one byte) are used up.
    #[error("calc keyword {0:?}: no function index left")]
    Full(String),
}

/// The keywords a mod game adds, per family. [`CalcRegistry::new`] (also the default)
/// is empty: D2RUST's own formula language.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CalcRegistry {
    skills: Vec<ModKeyword>,
    missiles: Vec<ModKeyword>,
    items: Vec<ModKeyword>,
}

/// An empty registry, for loaders that compile D2RUST's own formulas.
pub static NO_KEYWORDS: CalcRegistry = CalcRegistry::new();

impl CalcRegistry {
    /// An empty registry.
    pub const fn new() -> Self {
        CalcRegistry {
            skills: Vec::new(),
            missiles: Vec::new(),
            items: Vec::new(),
        }
    }

    /// No keyword in any family.
    pub fn is_empty(&self) -> bool {
        self.skills.is_empty() && self.missiles.is_empty() && self.items.is_empty()
    }

    /// The function index of the first registered keyword of `family`: the one after
    /// its built-in keywords.
    pub fn first_index(family: Family) -> u8 {
        family.keywords().len() as u8
    }

    /// Registers `keyword` in `family` and returns its function index.
    pub fn add(&mut self, family: Family, keyword: ModKeyword) -> Result<u8, RegistryError> {
        let name = keyword.name.as_bytes();
        if !name.first().is_some_and(u8::is_ascii_alphabetic)
            || !name.iter().all(u8::is_ascii_alphanumeric)
        {
            return Err(RegistryError::BadName(keyword.name));
        }
        if keyword.arity > MAX_ARITY {
            return Err(RegistryError::Arity {
                name: keyword.name,
                arity: keyword.arity,
            });
        }
        let builtin = family
            .keywords()
            .iter()
            .any(|k| k.text().as_bytes().eq_ignore_ascii_case(name));
        if builtin || self.position(family, name).is_some() {
            return Err(RegistryError::Taken(keyword.name));
        }
        let list = self.list_mut(family);
        let index = usize::from(Self::first_index(family)) + list.len();
        let Ok(index) = u8::try_from(index) else {
            return Err(RegistryError::Full(keyword.name));
        };
        list.push(keyword);
        Ok(index)
    }

    /// The keywords registered in `family`, in function-index order from
    /// [`CalcRegistry::first_index`].
    pub fn keywords(&self, family: Family) -> &[ModKeyword] {
        match family {
            Family::Skills => &self.skills,
            Family::Missiles => &self.missiles,
            Family::Items => &self.items,
        }
    }

    /// The registered keyword with function index `index` in `family`; `None` for a
    /// built-in index or one nothing was registered at.
    pub fn function(&self, family: Family, index: u8) -> Option<&ModKeyword> {
        let at = index.checked_sub(Self::first_index(family))?;
        self.keywords(family).get(usize::from(at))
    }

    /// Function index and keyword for the word `name` (any case) in `family`.
    pub fn find(&self, family: Family, name: &[u8]) -> Option<(u8, &ModKeyword)> {
        let at = self.position(family, name)?;
        let index = Self::first_index(family) + at as u8;
        Some((index, &self.keywords(family)[at]))
    }

    fn position(&self, family: Family, name: &[u8]) -> Option<usize> {
        self.keywords(family)
            .iter()
            .position(|k| k.name.as_bytes().eq_ignore_ascii_case(name))
    }

    fn list_mut(&mut self, family: Family) -> &mut Vec<ModKeyword> {
        match family {
            Family::Skills => &mut self.skills,
            Family::Missiles => &mut self.missiles,
            Family::Items => &mut self.items,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::Links;
    use super::super::{compile, compile_with};
    use super::*;

    fn kw(name: &str, arity: u8, names_like: Option<Keyword>) -> ModKeyword {
        ModKeyword {
            name: name.into(),
            arity,
            names_like,
        }
    }

    fn registry() -> CalcRegistry {
        let mut r = CalcRegistry::new();
        assert_eq!(r.add(Family::Skills, kw("twice", 1, None)), Ok(7));
        assert_eq!(
            r.add(Family::Skills, kw("spark", 2, Some(Keyword::Skill))),
            Ok(8)
        );
        assert_eq!(r.add(Family::Missiles, kw("twice", 1, None)), Ok(5));
        assert_eq!(r.add(Family::Items, kw("none", 0, None)), Ok(4));
        r
    }

    fn code(r: &CalcRegistry, family: Family, text: &str) -> Vec<u8> {
        compile_with(family, &Links, r, text.as_bytes())
            .unwrap()
            .code
    }

    #[test]
    fn indices_follow_the_builtins() {
        let r = registry();
        assert_eq!(CalcRegistry::first_index(Family::Skills), 7);
        assert_eq!(CalcRegistry::first_index(Family::Missiles), 5);
        assert_eq!(CalcRegistry::first_index(Family::Items), 4);
        assert_eq!(r.function(Family::Skills, 8).map(|k| k.arity), Some(2));
        assert_eq!(r.function(Family::Skills, 6), None);
        assert_eq!(r.function(Family::Skills, 9), None);
        assert_eq!(r.find(Family::Missiles, b"TWICE").map(|(i, _)| i), Some(5));
        assert!(r.find(Family::Items, b"twice").is_none());
        assert!(!r.is_empty());
        assert!(NO_KEYWORDS.is_empty());
    }

    #[test]
    fn refusals() {
        let mut r = registry();
        for name in ["", "2x", "a_b", "a b"] {
            assert!(matches!(
                r.add(Family::Skills, kw(name, 1, None)),
                Err(RegistryError::BadName(_))
            ));
        }
        for name in ["MIN", "sklvl", "Twice"] {
            assert!(matches!(
                r.add(Family::Skills, kw(name, 1, None)),
                Err(RegistryError::Taken(_))
            ));
        }
        // `sklvl` is a skills keyword only.
        assert_eq!(r.add(Family::Items, kw("sklvl", 1, None)), Ok(5));
        assert!(matches!(
            r.add(Family::Skills, kw("four", 4, None)),
            Err(RegistryError::Arity { arity: 4, .. })
        ));
        let mut full = CalcRegistry::new();
        for i in 0..=(255 - 4) {
            assert!(full
                .add(Family::Items, kw(&format!("f{i}"), 0, None))
                .is_ok());
        }
        assert!(matches!(
            full.add(Family::Items, kw("last", 0, None)),
            Err(RegistryError::Full(_))
        ));
    }

    #[test]
    fn mod_keywords_compile_to_calls() {
        let r = registry();
        // twice(lvl): param 16, CALL 7.
        assert_eq!(
            code(&r, Family::Skills, "twice(lvl)"),
            [0x04, 16, 0x01, 7, 0x00]
        );
        // Any case; an operand after the call.
        assert_eq!(
            code(&r, Family::Skills, "TWICE(3)+1"),
            [0x07, 3, 0x01, 7, 0x07, 1, 0x10, 0x00]
        );
        // Names inside `spark(` resolve like inside `skill(`: a skill name, then a
        // skill parameter code.
        assert_eq!(
            code(&r, Family::Skills, "spark('Fire Bolt'.ln12)"),
            [0x07, 36, 0x07, 0, 0x01, 8, 0x00]
        );
        assert_eq!(
            code(&r, Family::Missiles, "twice(lvl)"),
            [0x04, 18, 0x01, 5, 0x00]
        );
        // Zero arguments: never folded to a constant.
        assert_eq!(code(&r, Family::Items, "none()"), [0x01, 4, 0x00]);
        // A keyword of another family is a plain name there.
        assert_eq!(
            code(&r, Family::Items, "twice(2)"),
            code(&NO_KEYWORDS, Family::Items, "twice(2)")
        );
    }

    #[test]
    fn d2rust_formulas_compile_the_same() {
        let r = registry();
        let cases = [
            (
                Family::Skills,
                "min(ln12,par1)+sklvl('Holy Fire'.ln34, lvl+1, 2)",
            ),
            (
                Family::Skills,
                "skill('Fire Bolt'.dm12)*stat('strength'.base)/100",
            ),
            (Family::Skills, "twice + 3"),
            (Family::Skills, "rand(1,6) ? edmn : twicex(2)"),
            (
                Family::Missiles,
                "miss('firebolt'.par1) + skill('Fire Bolt'.lvl)",
            ),
            (Family::Items, "max(stat('strength'.mod), 7)"),
            (Family::Items, "4*5-2"),
        ];
        for (family, text) in cases {
            let plain = compile(family, &Links, text.as_bytes());
            assert_eq!(
                compile_with(family, &Links, &r, text.as_bytes()),
                plain,
                "{text}"
            );
            assert_eq!(
                compile_with(family, &Links, &NO_KEYWORDS, text.as_bytes()),
                plain,
                "{text}"
            );
        }
    }
}
