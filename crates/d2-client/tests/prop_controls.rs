//! Property tests on the controls file parser (`d2controls 1`): arbitrary
//! text and line soups of TOML fragments never panic and every error
//! names a line of the file; files the writer produces parse back to the
//! same effective bindings and rewrite to the same text;
//! whatever parses is clash-free with at most two inputs per action
//! (no clashes, two inputs at most).

use crate::prop_support;

use d2_client::controls::{self, Action, Bindings, ControlsFile, Key, Preset, MAX_INPUTS};
use proptest::prelude::*;

use prop_support::{bounded, config};

/// Lines of the file (what an error line may name; one past the end
/// for a missing entry in an empty file).
fn line_count(text: &str) -> usize {
    text.split('\n').count().max(1)
}

/// Checks one parse result against the controls file invariants.
fn check(text: &str) {
    match controls::parse(text) {
        Ok((file, bindings)) => {
            assert!(bindings.find_clash().is_none(), "clash accepted:\n{text}");
            for &a in Action::ALL {
                let keys = bindings.inputs(a);
                assert!(keys.len() <= MAX_INPUTS, "{a:?} has {keys:?}");
                for (i, k) in keys.iter().enumerate() {
                    assert!(!keys[..i].contains(k), "{a:?} repeats {k:?}");
                }
            }
            assert_eq!(file.effective().as_ref(), Ok(&bindings));
            // The canonical rewrite parses to the same bindings.
            let again = controls::write(&file);
            let (_, b2) = controls::parse(&again).expect("rewrite parses");
            assert_eq!(b2, bindings);
        }
        Err(e) => {
            let line = e.line.expect("a parsed file's error names its line");
            assert!(
                (1..=line_count(text)).contains(&line),
                "line {line} outside 1..={}: {e}\n{text}",
                line_count(text)
            );
            let _ = e.to_string();
        }
    }
}

fn action_name() -> impl Strategy<Value = String> {
    prop_oneof![
        4 => proptest::sample::select(Action::ALL).prop_map(|a| a.name().to_string()),
        1 => "[a-z_]{1,12}",
    ]
}

fn key_name() -> impl Strategy<Value = String> {
    prop_oneof![
        4 => proptest::sample::select(Key::ALL).prop_map(|k| k.name().to_string()),
        1 => "[A-Za-z0-9]{0,8}",
    ]
}

fn string_list(item: BoxedStrategy<String>) -> impl Strategy<Value = String> {
    proptest::collection::vec(item, 0..4).prop_map(|v| {
        let quoted: Vec<String> = v.iter().map(|s| format!("\"{s}\"")).collect();
        format!("[{}]", quoted.join(", "))
    })
}

/// One line of a controls-like TOML file: valid entries, wrong types,
/// unknown keys, dotted keys, inline and nested tables, junk.
fn fragment() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("version = 1".to_string()),
        (any::<i64>()).prop_map(|v| format!("version = {v}")),
        Just("version = \"1\"".to_string()),
        Just("preset = \"dev\"".to_string()),
        Just("preset = \"original\"".to_string()),
        "[a-z]{0,8}".prop_map(|p| format!("preset = \"{p}\"")),
        Just("preset = 3".to_string()),
        Just("[bindings]".to_string()),
        Just("[unbind]".to_string()),
        Just("[bindings.move_attack]".to_string()),
        Just("[[bindings]]".to_string()),
        Just("[unbind.list]".to_string()),
        string_list(action_name().boxed()).prop_map(|l| format!("list = {l}")),
        (action_name(), string_list(key_name().boxed())).prop_map(|(a, l)| format!("{a} = {l}")),
        action_name().prop_map(|a| format!("{a} = \"A\"")),
        action_name().prop_map(|a| format!("{a} = [1, 2]")),
        action_name().prop_map(|a| format!("bindings.{a} = [\"A\"]")),
        Just("unbind.list = [\"show_items\"]".to_string()),
        Just("bindings = { move_attack = [\"A\"] }".to_string()),
        Just("unbind = { list = [] , other = 1 }".to_string()),
        Just("unbind = 3".to_string()),
        Just("bindings = []".to_string()),
        "[a-z]{1,6}".prop_map(|k| format!("{k} = 1")),
        Just("# comment".to_string()),
        Just(String::new()),
        "[ -~]{0,24}",
    ]
}

/// Effective bindings: per action, the preset's inputs, none, or one or
/// two random distinct keys.
fn effective() -> impl Strategy<Value = Bindings> {
    let per_action = proptest::collection::vec(
        prop_oneof![
            2 => Just(None),
            1 => Just(Some(Vec::new())),
            1 => proptest::collection::vec(proptest::sample::select(Key::ALL), 1..=MAX_INPUTS)
                .prop_map(Some),
        ],
        Action::ALL.len(),
    );
    per_action.prop_map(|v| {
        let mut b = Preset::Dev.bindings().unwrap();
        for (&a, keys) in Action::ALL.iter().zip(v) {
            if let Some(mut keys) = keys {
                keys.dedup();
                let mut uniq: Vec<Key> = Vec::new();
                for k in keys {
                    if !uniq.contains(&k) {
                        uniq.push(k);
                    }
                }
                b.set(a, &uniq);
            }
        }
        b
    })
}

/// [`effective`] with clashes removed: the later action of each clash
/// loses its inputs (the clash rule holds by construction).
fn clash_free() -> impl Strategy<Value = Bindings> {
    effective().prop_map(|mut b| {
        while let Some(c) = b.find_clash() {
            b.set(c.second, &[]);
        }
        b
    })
}

proptest! {
    #![proptest_config(config(512))]

    /// Arbitrary text: an error with a line in the file, or bindings that
    /// satisfy the controls file invariants.
    #[test]
    fn arbitrary_text(text in "\\PC{0,200}") {
        bounded(move || check(&text));
    }

    /// Arbitrary bytes, lossily decoded.
    #[test]
    fn arbitrary_bytes(data in proptest::collection::vec(any::<u8>(), 0..256)) {
        bounded(move || check(&String::from_utf8_lossy(&data)));
    }

    /// Line soups of TOML fragments: the structure paths (wrong types,
    /// unknown keys, dotted keys, inline tables) never panic.
    #[test]
    fn fragment_soup(lines in proptest::collection::vec(fragment(), 0..12), head in any::<bool>()) {
        let mut text = String::new();
        if head {
            text.push_str("version = 1\npreset = \"dev\"\n");
        }
        text.push_str(&lines.join("\n"));
        bounded(move || check(&text));
    }

    /// Writer → parser round trip: a clash-free effective binding set,
    /// written as the smallest file over `dev`, parses back to itself, and
    /// the parsed file rewrites to the same text.
    #[test]
    fn write_parse_round_trip(b in clash_free()) {
        let file = ControlsFile::from_effective(Preset::Dev, &b).unwrap();
        prop_assert_eq!(file.effective(), Ok(b.clone()));
        let text = controls::write(&file);
        let (parsed, got) = controls::parse(&text).map_err(|e| TestCaseError::fail(format!("{e}\n{text}")))?;
        prop_assert_eq!(&got, &b);
        prop_assert_eq!(controls::write(&parsed), text);
    }

    /// A clashing effective set is refused by the file path too, naming
    /// a line of the file.
    #[test]
    fn clashes_are_refused(b in effective()) {
        prop_assume!(b.find_clash().is_some());
        let file = ControlsFile::from_effective(Preset::Dev, &b).unwrap();
        let text = controls::write(&file);
        prop_assert!(controls::parse(&text).is_err());
        check(&text);
    }

    /// Line edits of a valid file (delete, duplicate, replace with a
    /// fragment, swap): never a panic, the invariants on success.
    #[test]
    fn mutated_valid_file(
        b in effective(),
        edits in proptest::collection::vec((0usize..64, 0u8..4, fragment()), 1..4),
    ) {
        let file = ControlsFile::from_effective(Preset::Dev, &b).unwrap();
        let mut lines: Vec<String> = controls::write(&file).lines().map(str::to_string).collect();
        for (at, kind, frag) in edits {
            let n = lines.len();
            let i = at % n.max(1);
            match kind {
                0 if n > 0 => {
                    lines.remove(i);
                }
                1 if n > 0 => {
                    let l = lines[i].clone();
                    lines.insert(i, l);
                }
                2 if n > 0 => lines[i] = frag,
                _ => lines.insert(at % (n + 1), frag),
            }
        }
        let text = lines.join("\n");
        bounded(move || check(&text));
    }
}
