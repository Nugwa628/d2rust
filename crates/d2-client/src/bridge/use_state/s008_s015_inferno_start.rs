//! cltstfunc 8 and 15 (Blade Fury, Inferno): one function of the client skill start
//! table (`super::client_start_apply`); the shared helpers live in `super`.

use super::*;

/// The state part of `cltstfunc` 15 and 8 (see [`client_start_apply`]);
/// `blade_fury` selects function 8's differences.
pub(super) fn inferno_start(w: &mut ClientWorld, caster: UnitKey, blade_fury: bool, seqinput: i32) {
    let Some(u) = w.units.get_mut(&caster) else {
        return;
    };
    if let std::collections::btree_map::Entry::Vacant(e) = u.state_lists.entry(STATE_INFERNO) {
        e.insert(Default::default());
        u.timed_lists
            .insert(STATE_INFERNO, if blade_fury { 20 } else { 21 });
        u.states.insert(STATE_INFERNO);
        if blade_fury {
            if let Some(e) = u
                .skills
                .as_mut()
                .and_then(|l| l.current.and_then(|i| l.entries.get_mut(i)))
            {
                e.param = 0;
            }
        }
    } else {
        if let Some(q) = u.sequence.as_mut() {
            q.pos = if blade_fury {
                (q.pos & 0xFF) + (seqinput - 1) * 256
            } else {
                (q.pos & 0xFF) | 0x800
            };
        }
        u.frame_count = if blade_fury { 0x800 } else { 0x500 };
        if let Some(e) = u.timed_lists.get_mut(&STATE_INFERNO) {
            *e = 7;
        }
    }
}
