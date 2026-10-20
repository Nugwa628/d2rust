//! The belt keys of the play preview (commands 23–26, `BeltSlot1`–`4`):
//! the column-ready byte gate, then the belt use
//! (`controls::original::belt_use`) on the model's belt
//! (`bridge::items::belt`), sending C→S 0x26 through the bridge.

use crate::controls::original::{belt_use, BeltUse, BeltUseFacts};
use crate::controls::Action;
use crate::ui::{ActionId, UiEvent};

use super::items;
use super::link::ServerLink;
use super::world::ClientWorld;
use super::{Bridge, BridgeError};

/// The belt column of a belt-key action event, 0–3.
pub fn column(e: &UiEvent) -> Option<u8> {
    let UiEvent::Action(ActionId(a)) = *e else {
        return None;
    };
    [
        Action::BeltSlot1,
        Action::BeltSlot2,
        Action::BeltSlot3,
        Action::BeltSlot4,
    ]
    .iter()
    .position(|b| b.index() as u16 == a)
    .map(|c| c as u8)
}

/// The UI facts a belt key reads besides the model.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyFacts {
    /// Shift held (`GetAsyncKeyState(VK_SHIFT) & 0x8000`): the hireling
    /// feed.
    pub shift: bool,
    /// Ui 9 (the game menu) is open.
    pub ui9_open: bool,
}

/// The C→S 0x26 bytes of a belt key on column `c`, if the key acts.
///
/// D2RUST's own, unverified: the item-table facts of the use test (`quest`,
/// `unique`, `useable`) and the busy test are not in the
/// client model; a belt item is taken as useable, not a quest or unique
/// item, and not busy. The game-exit flag of the gate is not in
/// the model (never set while the play loop runs).
pub fn key_message(w: &ClientWorld, c: u8, f: KeyFacts) -> Option<Vec<u8>> {
    // No local player, or the local player dead (mode 0x11).
    if w.local().is_none_or(|p| p.mode == 0x11) {
        return None;
    }
    // Only when the column-ready byte is 1.
    if !w.belt_ready.get(usize::from(c)).copied().unwrap_or(false) {
        return None;
    }
    let item = items::belt(w).get(&u16::from(c)).map(|i| i.key.guid);
    let facts = BeltUseFacts {
        expansion: w.expansion != 0,
        shift: f.shift,
        cursor_item: items::cursor_item(w).is_some(),
        ui9_open: f.ui9_open,
        item,
        passes_quest_unique: true,
        useable: true,
        busy: false,
    };
    match belt_use(&facts) {
        BeltUse::Send { item, shift } => {
            let mut m = super::intent::encode(&items::use_belt(item));
            m[5..9].copy_from_slice(&shift.to_le_bytes());
            Some(m)
        }
        _ => None,
    }
}

/// Sends the belt use of each belt-key action in `events` (the actions
/// no panel took); returns the messages sent.
pub fn send_keys<L: ServerLink>(
    bridge: &mut Bridge<L>,
    events: &[UiEvent],
    f: KeyFacts,
) -> Result<usize, BridgeError> {
    let mut n = 0;
    for c in events.iter().filter_map(column) {
        if let Some(m) = key_message(bridge.world(), c, f) {
            bridge.send_bytes(&m)?;
            n += 1;
        }
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::world::{ClientUnit, ItemData, ItemRecord, KindData, PlayerData, UnitKey};

    fn world() -> ClientWorld {
        let mut w = ClientWorld::default();
        let p = UnitKey::new(0, 1);
        let mut u = ClientUnit::new(p);
        u.kind = KindData::Player(PlayerData::default());
        w.units.insert(p, u);
        w.local_player = Some(p);
        // A potion in belt slot 1: flags 0x10, version 0x65, mode 2, x 1.
        let k = UnitKey::new(4, 9);
        let mut s = vec![0u8; 12];
        s[0] = 0x10;
        let mut bits: u64 = 0x65 | (2 << 10) | (1 << 17);
        for b in s.iter_mut().skip(4).take(4) {
            *b = bits as u8;
            bits >>= 8;
        }
        let mut u = ClientUnit::new(k);
        u.kind = KindData::Item(ItemData {
            last: Some(ItemRecord {
                id: 0x9C,
                action: 0x0E,
                category: 0x10,
                owner: None,
                seq: 0,
                stream: s,
            }),
            ..ItemData::default()
        });
        w.units.insert(k, u);
        w
    }

    #[test]
    fn a_ready_column_with_a_belt_item_sends_0x26() {
        let mut w = world();
        let none = KeyFacts::default();
        assert_eq!(key_message(&w, 1, none), None, "column 1 not ready");
        w.belt_ready[1] = true;
        assert_eq!(
            key_message(&w, 1, none),
            Some(vec![0x26, 9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
        );
        w.belt_ready[0] = true;
        assert_eq!(key_message(&w, 0, none), None, "slot 0 is empty");
        let ev = UiEvent::Action(ActionId(Action::BeltSlot2.index() as u16));
        assert_eq!(column(&ev), Some(1));
    }

    // Shift feeds the hireling: the shift word 0x8000 in an expansion
    // game, 0 in a classic one; ui 9 open or a dead player: nothing.
    #[test]
    fn shift_ui9_and_death_gate_the_belt_key() {
        let mut w = world();
        w.belt_ready[1] = true;
        let shift = KeyFacts {
            shift: true,
            ui9_open: false,
        };
        assert_eq!(
            key_message(&w, 1, shift),
            Some(vec![0x26, 9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
            "classic game: shift := 0"
        );
        w.expansion = 1;
        assert_eq!(
            key_message(&w, 1, shift),
            Some(vec![0x26, 9, 0, 0, 0, 0, 0x80, 0, 0, 0, 0, 0, 0])
        );
        let menu = KeyFacts {
            shift: false,
            ui9_open: true,
        };
        assert_eq!(key_message(&w, 1, menu), None, "ui 9 open");
        let p = w.local_player.unwrap();
        w.units.get_mut(&p).unwrap().mode = 0x11;
        assert_eq!(key_message(&w, 1, KeyFacts::default()), None, "dead");
    }
}
