//! The client quest check (c) the level-entry lines ask:
//! quest `c` is open and not done for this
//! player, from the client quest state the bridge keeps (the 0x5E bytes,
//! the records of S→C 0x28 / 0x29, the status list 0x52).

use crate::bridge::world::ClientWorld;
use crate::ui::quest_log::tables::chain_entry;
use crate::ui::quest_log::{derive_row, QuestFlags, RowCtx};

/// 96 bytes as 48 little-endian slots.
fn flags(record: &[u8; 96]) -> QuestFlags {
    let mut f = QuestFlags::new();
    for q in 0..48u8 {
        let i = usize::from(q) * 2;
        f.set_word(q, u16::from_le_bytes([record[i], record[i + 1]]));
    }
    f
}

/// Result 1 of the check (c). Where the game is fatal (no 0x5E yet, `c`
/// ≥ 37) the check answers no.
pub fn client_quest_check(w: &ClientWorld, c: u8) -> bool {
    // 1. The 0x5E byte c is non-zero.
    if w.client_quest_byte(usize::from(c)).is_none_or(|b| b == 0) {
        return false;
    }
    // 2. The first entry whose server chain is c.
    let Some(q) = chain_entry(c) else {
        return false;
    };
    // 3. G received and G.13 (slot q) clear.
    let Some(g) = w.quest_game.as_ref().map(flags) else {
        return false;
    };
    if g.bit(q, 13) {
        return false;
    }
    // 4. P received and P.1, P.0, P.14 (slot q) clear.
    let Some(p) = w.quest_player.as_ref().map(flags) else {
        return false;
    };
    if p.bit(q, 1) || p.bit(q, 0) || p.bit(q, 14) {
        return false;
    }
    // 5. Den of Evil only: the shown status of its row is below 5.
    if c == 1 {
        let status = w.quest_status.unwrap_or([0; 41]);
        let ctx = RowCtx {
            p: &p,
            g: Some(&g),
            multiplayer: false,
            den: w.quest_counters[0],
            barbarians: w.quest_counters[2],
        };
        // The row's `last` mark is the quest log's own state, which this
        // check shares in the game (a level entry can consume the
        // "changed" mark); the sound layer keeps a scratch copy.
        let mut last = 0;
        let row = derive_row(
            q,
            status.get(usize::from(q)).copied().unwrap_or(0),
            &ctx,
            &mut last,
        );
        if row.shown >= 5 {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world() -> ClientWorld {
        let mut w = ClientWorld::default();
        let mut avail = [0u8; 37];
        avail[1] = 1;
        avail[2] = 1;
        w.quest_availability = Some(avail);
        w.quest_game = Some([0; 96]);
        w.quest_player = Some([0; 96]);
        w.quest_status = Some([0; 41]);
        w
    }

    #[test]
    fn open_and_not_done_passes_each_gate() {
        let mut w = world();
        // Burial Grounds (chain 2): the 0x5E byte, G and P all clear.
        assert!(client_quest_check(&w, 2));
        // A zero 0x5E byte, no 0x5E at all, or a chain past the table: no.
        w.quest_availability.as_mut().unwrap()[2] = 0;
        assert!(!client_quest_check(&w, 2));
        w.quest_availability = None;
        assert!(!client_quest_check(&w, 2));
        assert!(!client_quest_check(&world(), 40));
        // G.13 of slot q (2) set: done in the game.
        let mut w = world();
        w.quest_game.as_mut().unwrap()[4] = 0x00;
        w.quest_game.as_mut().unwrap()[5] = 1 << 5;
        assert!(!client_quest_check(&w, 2));
        // P.0 / P.1 / P.14 of slot 2 set: no.
        for bit in [0u8, 1, 14] {
            let mut w = world();
            let (lo, hi) = (4, 5);
            let word = 1u16 << bit;
            w.quest_player.as_mut().unwrap()[lo] = word as u8;
            w.quest_player.as_mut().unwrap()[hi] = (word >> 8) as u8;
            assert!(!client_quest_check(&w, 2), "P.{bit}");
        }
        // G or P not received: no.
        let mut w = world();
        w.quest_game = None;
        assert!(!client_quest_check(&w, 2));
    }

    #[test]
    fn den_of_evil_needs_a_shown_status_below_5() {
        let w = world();
        assert!(client_quest_check(&w, 1));
    }
}
