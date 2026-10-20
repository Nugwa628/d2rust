//! Shared character restoration and snapshot helpers.
pub use d2_server::game::save_gaps::*;

#[cfg(test)]
mod record_tests {
    use super::*;
    use d2_formats::d2s::Slot;
    use d2_server::adapters::session::SkillHand;

    /// A gas potion's bookkeeping saved
    /// left:= (Attack, −1); the post-load selects only the right (Throw) and
    /// the join pair sends L Attack, R Throw.
    #[test]
    fn the_join_reads_the_slots_the_bookkeeping_left() {
        let saved = [(0, -1), (2, -1)];
        assert_eq!(saved_selects(saved), ([None, Some(2)], [None, None]));
        let r = saved_record(saved, &[7], 0);
        let hand = |skill| SkillHand {
            skill,
            item: u32::MAX,
        };
        assert_eq!(r.hands, [hand(2), hand(0)]);
        // An item-owned slot: index 1 → the first GUID, selected as an
        // item entry.
        let saved = [(4, 1), (2, -1)];
        assert_eq!(saved_selects(saved), ([None, Some(2)], [Some(4), None]));
        assert_eq!(saved_record(saved, &[7], 0).hands[1].item, 7);
    }

    #[test]
    fn a_loaded_record_sends_the_decoded_mouse_pair() {
        // The fresh saves' pair (`00 00 00 00` twice): skill 0,
        // item −1 in both hands; recorded `23 00 01000000 01 0000
        // ffffffff` then `... 00 0000 ffffffff`.
        let r = loaded_record(&[Slot::default(); 2], &[7, 8], 1);
        assert_eq!(r.portal_flags, 1);
        let none = SkillHand {
            skill: 0,
            item: u32::MAX,
        };
        assert_eq!(r.hands, [none, none]);
        // Right = hand 0, left = hand 1; index 2 → the second GUID, past
        // the end → −1, 0xFFFF → skill −1.
        let left = Slot { code: 36, item: 2 };
        let right = Slot { code: 49, item: 3 };
        let r = loaded_record(&[left, right], &[7, 8], 0);
        assert_eq!(
            r.hands,
            [
                SkillHand {
                    skill: 49,
                    item: u32::MAX
                },
                SkillHand { skill: 36, item: 8 },
            ]
        );
        let r = loaded_record(&[Slot::NONE, Slot::NONE], &[], 0);
        assert_eq!(r.hands[0].skill, 0xFFFF);
    }
}
