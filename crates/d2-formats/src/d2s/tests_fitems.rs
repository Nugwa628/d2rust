//! Unit tests for the header slot layouts and hireling block values in `d2s`.

use super::*;

#[test]
fn fresh_save_hotkeys_and_mouse_bytes() {
    let h = Header::default();
    let b = h.to_bytes();
    for i in 0..16 {
        assert_eq!(&b[0x38 + 4 * i..0x3C + 4 * i], &[0xFF, 0xFF, 0, 0]);
    }
    // Left: no left skill; right: skill 0, no item; swap pair zero.
    assert_eq!(&b[0x78..0x88], &[0u8; 16]);
    // Sorceress with Fire Bolt (skill 36) as right skill, item index 0.
    let mut h = Header::default();
    h.mouse[1] = Slot::encode(36, false, 0).unwrap();
    let b = h.to_bytes();
    assert_eq!(&b[0x7C..0x80], &[0x24, 0, 0, 0]);
    // Left skill with an owner item at 1-based position 5 goes to 0x78.
    h.mouse[0] = Slot::encode(36, true, 5).unwrap();
    h.mouse[2] = Slot::encode(3, false, 1).unwrap();
    h.mouse[3] = Slot::encode(4, false, 2).unwrap();
    let b = h.to_bytes();
    assert_eq!(&b[0x78..0x7C], &[0x24, 0x80, 5, 0]);
    assert_eq!(&b[0x80..0x84], &[3, 0, 1, 0]);
    assert_eq!(&b[0x84..0x88], &[4, 0, 2, 0]);
}

#[test]
fn hireling_block_bytes() {
    let h = Header {
        hireling: Hireling {
            flags: Hireling::DEAD,
            seed: 0xDEAD_BEEF,
            name_index: 21,
            id: 0,
            experience: 39_482,
            rest: [0; 16],
        },
        ..Header::default()
    };
    let b = h.to_bytes();
    assert_eq!(&b[0xAF..0xB3], &[0, 0, 1, 0]);
    assert_eq!(&b[0xB3..0xB7], &0xDEAD_BEEFu32.to_le_bytes());
    assert_eq!(&b[0xB7..0xB9], &[21, 0]);
    assert_eq!(&b[0xB9..0xBB], &[0, 0]);
    assert_eq!(&b[0xBB..0xBF], &39_482u32.to_le_bytes());
    assert_eq!(&b[0xBF..0xCF], &[0u8; 16]);
    // Absent hireling: the block stays zero.
    let b = Header::default().to_bytes();
    assert_eq!(&b[0xAF..0xCF], &[0u8; 32]);
}

#[test]
fn appearance_reset_is_all_ff() {
    let mut h = Header {
        components: [1; 16],
        colours: [2; 16],
        ..Header::default()
    };
    h.reset_appearance();
    let b = h.to_bytes();
    assert_eq!(&b[0x88..0xA8], &[0xFFu8; 32]);
}
