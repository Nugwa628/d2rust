//! Points of the provisional index that the install's tables settle
//! on the user's game files.

use crate::app_support;

// FrameCnt0–7 << 8; the quest host's `object_frame_count1` (`>> 8`) reads
// back the txt value.
// Measured with `data-tool excel-dir` + the live Objects.txt of Patch_D2:
// row 1 (Casket) FrameCnt1 = 7, row 2 (Shrine) = 21.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn objects_framecnt1_is_the_txt_value_times_256() {
    let d = app_support::live();
    let t = d.tables.table("objects").unwrap();
    let u32_at = |row: usize, off: usize| {
        let o = row * t.record_size + off;
        u32::from_le_bytes(t.records[o..o + 4].try_into().unwrap())
    };
    assert_eq!(u32_at(1, 220), 7 << 8);
    assert_eq!(u32_at(2, 220), 21 << 8);
}

// The stash and cube fallbacks equal the install's `inventory.bin` rows 8 / 9 / 12 and 24 / 25 / 28 (the grid
// rectangle, 640 × 480 and 800 × 600).
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn stash_and_cube_fallback_grids_equal_the_inventory_rows() {
    use d2_client::ui::layout::Screen;
    use d2_client::ui::panels::cube_items::{cube_record, fallback_cube_grid};
    use d2_client::ui::panels::stash_items::{fallback_stash_grid, stash_record};
    let d = app_support::live();
    let t = d.tables.table("inventory").unwrap();
    let rec = |i: usize| {
        let r = &t.records[i * t.record_size..(i + 1) * t.record_size];
        let u = |o: usize| u32::from_le_bytes(r[o..o + 4].try_into().unwrap()) as i32;
        (r[16], r[17], u(20), u(24), u(28), u(32), r[36], r[37])
    };
    let key = |g: d2_client::ui::inv_grid::GridRecord| {
        (
            g.grid_x, g.grid_y, g.left, g.right, g.top, g.bottom, g.cell_w, g.cell_h,
        )
    };
    for screen in [Screen::R640, Screen::R800] {
        for expansion in [false, true] {
            let i = stash_record(expansion, &screen);
            assert_eq!(
                key(fallback_stash_grid(expansion, &screen)),
                rec(i),
                "stash record {i}"
            );
        }
        let i = cube_record(&screen);
        assert_eq!(key(fallback_cube_grid(&screen)), rec(i), "cube record {i}");
    }
}
