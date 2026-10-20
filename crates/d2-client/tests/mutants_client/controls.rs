//! Mutation-testing gaps of the controls file.

use d2_client::controls::{parse, Action, Bindings, Clash, Context, ErrorKind, Key};

/// A clash is two actions on one input; one action listing an
/// input twice (possible only for bindings built in code: the parser
/// refuses a repeated input) triggers one action and is no clash.
#[test]
fn one_action_twice_on_an_input_is_not_a_clash() {
    let mut b = Bindings::empty();
    b.set(Action::ToggleInventory, &[Key::I, Key::I]);
    assert_eq!(b.find_clash(), None);
    b.set(Action::ToggleCharacter, &[Key::I]);
    assert_eq!(
        b.find_clash(),
        Some(Clash {
            context: Context::World,
            key: Key::I,
            first: Action::ToggleInventory,
            second: Action::ToggleCharacter,
        })
    );
}

/// Errors carry the line of the entry that causes them. With several
/// `[bindings]` entries, a clash is reported at the clashing entry's line,
/// not at another entry's.
#[test]
fn clash_is_reported_at_the_clashing_entry() {
    let text = "version = 1\npreset = \"dev\"\n\n[bindings]\ntoggle_character = [\"X\"]\ntoggle_skill_tree = [\"I\"]\n";
    let e = parse(text).unwrap_err();
    assert_eq!(
        e.kind,
        ErrorKind::Clash(Clash {
            context: Context::World,
            key: Key::I,
            first: Action::ToggleInventory,
            second: Action::ToggleSkillTree,
        })
    );
    assert_eq!(e.line, Some(6));
}
