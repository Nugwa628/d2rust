//! Extra tests for the ds1, dt1, dc6, cof, animdata, palette and tbl parsers, each
//! aimed at a mutant that `cargo mutants` reported as missed.
//!
//! Test-only module of d2-formats; the tests build small synthetic files and check
//! what the parsers decode or reject.

use crate::animdata::{self, AnimData};
use crate::cof::Cof;
use crate::dc6::Dc6;
use crate::ds1::{cell, Ds1};
use crate::dt1::{Dt1, Dt1Block};
use crate::palette::Pl2;
use crate::tbl::{key_hash, StringTable};

fn p32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_le_bytes());
}

fn p16(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_le_bytes());
}

// ---------------------------------------------------------------- ds1

#[test]
fn ds1_cell_fields() {
    // Cell fields: hidden = b3 & 0x80; sub index = b1;
    // main index = (b2 >> 4) | ((b3 & 3) << 4).
    assert!(!cell::hidden(0));
    assert!(!cell::hidden(0x7FFF_FFFF));
    assert!(cell::hidden(0x8000_0000));
    assert!(cell::hidden(0xFFFF_FFFF));
    assert_eq!(cell::sub_index(0x0000_AB00), 0xAB);
    assert_eq!(cell::main_index(0x0350_0000), 0x35);
    assert_eq!(cell::main_index(0xFCF0_0000), 0x0F);
}

/// Header through the floor count, for `v >= 4`.
fn ds1_head(v: u32, w: u32, h: u32, tag_type: u32, walls: u32, floors: u32) -> Vec<u8> {
    let mut d = Vec::new();
    p32(&mut d, v);
    p32(&mut d, w - 1);
    p32(&mut d, h - 1);
    if v >= 8 {
        p32(&mut d, 0); // act
    }
    if v >= 10 {
        p32(&mut d, tag_type);
    }
    p32(&mut d, 0); // file count (v >= 3)
    if (9..=13).contains(&v) {
        d.extend_from_slice(&[0; 8]);
    }
    p32(&mut d, walls);
    if v >= 16 {
        p32(&mut d, floors);
    }
    d
}

#[test]
fn ds1_v4_has_no_tag_layer() {
    // Step 8: tags only for v < 4 or tag_type 1/2; v >= 4 order is wall,
    // orientation, floors, shadow. 3x1 grid (W*H = 3 cells per layer).
    let mut d = ds1_head(4, 3, 1, 0, 1, 1);
    for x in [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12] {
        p32(&mut d, x);
    }
    p32(&mut d, 0); // objects
    let m = Ds1::parse(&d).unwrap();
    assert_eq!(m.walls, vec![vec![1, 2, 3]]);
    // v < 7: orientations through ORIENTATION_LOOKUP (4, 5, 6 -> 2, 3, 3).
    assert_eq!(m.orientations, vec![vec![0x02, 0x03, 0x03]]);
    assert_eq!(m.floors, vec![vec![7, 8, 9]]);
    assert_eq!(m.shadow, vec![10, 11, 12]);
    assert_eq!(m.tags, None);
    assert!(m.objects.is_empty());
    assert!(m.trailing.is_empty());
}

#[test]
fn ds1_zero_walls() {
    // Step 7 allows 0 walls; v15: one floor, then shadow.
    let mut d = ds1_head(15, 2, 2, 0, 0, 1);
    for x in 1..=8 {
        p32(&mut d, x);
    }
    p32(&mut d, 0); // objects
    let m = Ds1::parse(&d).unwrap();
    assert!(m.walls.is_empty() && m.orientations.is_empty());
    assert_eq!(m.floors, vec![vec![1, 2, 3, 4]]);
    assert_eq!(m.shadow, vec![5, 6, 7, 8]);
    assert!(m.paths.is_empty());
}

#[test]
fn ds1_zero_walls_zero_floors() {
    // v16: 0 walls and 0 floors leave the shadow layer only. 3x3 grid.
    let mut d = ds1_head(16, 3, 3, 0, 0, 0);
    for x in 1..=9 {
        p32(&mut d, x);
    }
    p32(&mut d, 0); // objects
    let m = Ds1::parse(&d).unwrap();
    assert!(m.walls.is_empty() && m.floors.is_empty());
    assert_eq!(m.shadow, (1..=9).collect::<Vec<u32>>());
    assert_eq!(m.tags, None);
}

#[test]
fn ds1_v1_layers_end_the_file() {
    // v1: no act/tag/files/objects; stream order wall, floor,
    // orientation, tag, shadow, and nothing after.
    let mut d = Vec::new();
    for x in [1, 0, 0, 0x11, 0x22, 7, 0x44, 0x55] {
        p32(&mut d, x);
    }
    let m = Ds1::parse(&d).unwrap();
    assert_eq!((m.width, m.height), (1, 1));
    assert_eq!(m.walls, vec![vec![0x11]]);
    assert_eq!(m.floors, vec![vec![0x22]]);
    assert_eq!(m.orientations, vec![vec![0x05]]);
    assert_eq!(m.tags, Some(vec![0x44]));
    assert_eq!(m.shadow, vec![0x55]);
    assert!(m.objects.is_empty() && m.trailing.is_empty());
}

#[test]
fn ds1_65536_groups_read_as_zero_past_the_end() {
    // Step 10: counts up to 65,536 are read; fields past the end are 0.
    let mut d = ds1_head(12, 1, 1, 1, 0, 1);
    for x in [1, 2, 3] {
        p32(&mut d, x); // floor, shadow, tag
    }
    p32(&mut d, 0); // objects
    p32(&mut d, 0x1_0000); // groups
    let m = Ds1::parse(&d).unwrap();
    assert_eq!(m.groups.len(), 0x1_0000);
    assert!(m
        .groups
        .iter()
        .all(|g| (g.x, g.y, g.width, g.height, g.unknown) == (0, 0, 0, 0, 0)));
    assert!(m.groups_truncated);
    assert_eq!(m.tags, Some(vec![3]));
}

#[test]
fn ds1_v13_trailing_bytes_are_not_paths() {
    // Step 11 needs v >= 14; step 12 keeps trailing bytes.
    let mut d = ds1_head(13, 1, 1, 0, 0, 1);
    p32(&mut d, 1); // floor
    p32(&mut d, 2); // shadow
    p32(&mut d, 0); // objects
    p32(&mut d, 0); // trailing
    let m = Ds1::parse(&d).unwrap();
    assert!(m.paths.is_empty());
    assert_eq!(m.trailing, vec![0; 4]);
}

#[test]
fn ds1_v14_without_path_section() {
    // Step 11: paths only if bytes remain.
    let mut d = ds1_head(14, 1, 1, 0, 0, 1);
    p32(&mut d, 1);
    p32(&mut d, 2);
    p32(&mut d, 0); // objects
    let m = Ds1::parse(&d).unwrap();
    assert!(m.paths.is_empty() && m.trailing.is_empty());
}

#[test]
fn ds1_v14_path_points_have_no_action() {
    // Step 11: v14 points are (x, y); action defaults to 1.
    let mut d = ds1_head(14, 1, 1, 0, 0, 1);
    p32(&mut d, 1);
    p32(&mut d, 2);
    p32(&mut d, 0); // objects
    for x in [1, 1, 7, 8, 3, 4] {
        p32(&mut d, x); // 1 path: 1 point at (7, 8): point (3, 4)
    }
    let m = Ds1::parse(&d).unwrap();
    assert_eq!(m.paths.len(), 1);
    assert_eq!((m.paths[0].x, m.paths[0].y), (7, 8));
    let p = &m.paths[0].points;
    assert_eq!(p.len(), 1);
    assert_eq!((p[0].x, p[0].y, p[0].action), (3, 4, 1));
    assert!(m.trailing.is_empty());
}

// ---------------------------------------------------------------- dt1

fn block(format: u16) -> Dt1Block {
    Dt1Block {
        x: 0,
        y: 0,
        unknown1: 0,
        grid_x: 0,
        grid_y: 0,
        format,
        unknown2: 0,
        pixels: Vec::new(),
    }
}

#[test]
fn dt1_block_kind_and_size() {
    // Block kind: format 1 is a 32x15 iso block, any other a 32x32 RLE.
    assert!(block(1).is_iso());
    assert_eq!(block(1).size(), (32, 15));
    assert!(!block(0x1001).is_iso());
    assert_eq!(block(0x1001).size(), (32, 32));
}

#[test]
fn dt1_tile_headers_filling_the_whole_file() {
    // 3 consecutive 96-byte tile headers from offset 0 fill a 288-byte
    // file exactly; every header stays inside the file.
    let mut d = vec![0u8; 288];
    d[0..4].copy_from_slice(&7i32.to_le_bytes());
    d[4..8].copy_from_slice(&6i32.to_le_bytes());
    d[268..272].copy_from_slice(&3u32.to_le_bytes());
    let t = Dt1::parse(&d).unwrap();
    assert_eq!(t.tiles.len(), 3);
    assert_eq!(t.tiles[0].light_direction, 7);
    assert!(t.tiles.iter().all(|t| t.blocks.is_empty()));
}

/// One tile with one block of `format` and `encoded` data.
fn dt1_one_block(format: u16, encoded: &[u8]) -> Vec<u8> {
    let mut d = Vec::new();
    p32(&mut d, 7);
    p32(&mut d, 6);
    d.extend_from_slice(&[0; 260]);
    p32(&mut d, 1); // tiles
    p32(&mut d, 276); // first tile
    let mut tile = vec![0u8; 96];
    tile[0x48..0x4C].copy_from_slice(&372u32.to_le_bytes());
    tile[0x50..0x54].copy_from_slice(&1u32.to_le_bytes());
    d.extend_from_slice(&tile);
    p16(&mut d, 0); // x
    p16(&mut d, 0); // y
    p16(&mut d, 0);
    d.extend_from_slice(&[0, 0]); // grid x, y
    p16(&mut d, format);
    p32(&mut d, encoded.len() as u32);
    p16(&mut d, 0);
    p32(&mut d, 20); // data right after the header
    d.extend_from_slice(encoded);
    d
}

fn rle(encoded: &[u8]) -> Result<Vec<u8>, crate::cursor::FormatError> {
    Dt1::parse(&dt1_one_block(0x1001, encoded)).map(|t| t.tiles[0].blocks[0].pixels.clone())
}

#[test]
fn dt1_rle_zero_skip_pair_copies_pixels() {
    // Only (0, 0) moves to the next row; (0, 2) copies 2 pixels.
    let p = rle(&[0x00, 0x02, 0x0A, 0x0B]).unwrap();
    assert_eq!(&p[..3], &[0x0A, 0x0B, 0]);
    assert!(p[32..].iter().all(|&b| b == 0));
}

#[test]
fn dt1_rle_skip_past_last_row_writes_nothing() {
    // Moving past the last row is allowed if no pixel is written there.
    let mut e = [0u8; 64].to_vec();
    e.extend_from_slice(&[0x05, 0x00]);
    assert_eq!(rle(&e).unwrap(), vec![0; 32 * 32]);
    // A skip past the row end on the last row, with no pixels, too.
    let mut e = [0u8; 62].to_vec();
    e.extend_from_slice(&[0xFF, 0x00]);
    assert_eq!(rle(&e).unwrap(), vec![0; 32 * 32]);
}

#[test]
fn dt1_rle_run_ending_at_row_end() {
    // x 30 + 2 = 32 stays inside.
    let p = rle(&[0x1E, 0x02, 0x0A, 0x0B]).unwrap();
    assert_eq!(&p[29..33], &[0, 0x0A, 0x0B, 0]);
}

#[test]
fn dt1_rle_runs_advance_x() {
    // x += skip, copy, x += count: a b at 1..2, c at 4.
    let p = rle(&[0x01, 0x02, 0x0A, 0x0B, 0x01, 0x01, 0x0C]).unwrap();
    assert_eq!(&p[..6], &[0, 0x0A, 0x0B, 0, 0x0C, 0]);
}

// ---------------------------------------------------------------- dc6

/// D x F frames whose pointers all reach one frame.
fn dc6_shared_frame(d: u32, f: u32, w: u32, h: u32, flip: u32, enc: &[u8]) -> Vec<u8> {
    let mut v = Vec::new();
    p32(&mut v, 6);
    p32(&mut v, 0);
    p32(&mut v, 0);
    v.extend_from_slice(&[0xEE; 4]);
    p32(&mut v, d);
    p32(&mut v, f);
    let n = d * f;
    let at = 24 + 4 * n;
    for _ in 0..n {
        p32(&mut v, at);
    }
    for x in [flip, w, h, 0, 0, 0, 0, enc.len() as u32] {
        p32(&mut v, x);
    }
    v.extend_from_slice(enc);
    v
}

#[test]
fn dc6_0x10000_frames_allowed() {
    // D x F must not exceed 0x10000: exactly 0x10000 parses.
    let d = Dc6::parse(&dc6_shared_frame(1, 0x1_0000, 0, 0, 0, &[])).unwrap();
    assert_eq!(d.frames.len(), 0x1_0000);
}

#[test]
fn dc6_flip_selects_row_order() {
    // flip != 0: first encoded row is the top row; flip 0: the bottom row.
    let top = Dc6::parse(&dc6_shared_frame(1, 1, 1, 2, 1, &[0x01, 0x05])).unwrap();
    assert_eq!(top.frames[0].pixels, vec![5, 0]);
    let bottom = Dc6::parse(&dc6_shared_frame(1, 1, 1, 2, 0, &[0x01, 0x05])).unwrap();
    assert_eq!(bottom.frames[0].pixels, vec![0, 5]);
}

#[test]
fn dc6_runs_in_a_row_follow_each_other() {
    // x += b after each copy.
    let d = Dc6::parse(&dc6_shared_frame(1, 1, 2, 1, 0, &[0x01, 0x05, 0x01, 0x06])).unwrap();
    assert_eq!(d.frames[0].pixels, vec![5, 6]);
}

// ---------------------------------------------------------------- cof

fn cof(l: u8, f: u8, d: u8, components: &[u8], events: &[u8], order: &[u8]) -> Vec<u8> {
    let mut v = vec![l, f, d, 20, 0, 0, 0, 0];
    v.extend_from_slice(&[0; 20]);
    for &c in components {
        v.extend_from_slice(&[c, 1, 1, 0, 0]);
        v.extend_from_slice(b"hth\0");
    }
    v.extend_from_slice(events);
    v.extend_from_slice(order);
    v
}

#[test]
fn cof_draw_order_has_d_f_l_bytes() {
    // L=2, F=1, D=1: K = 1 event, 2 draw-order bytes.
    let c = Cof::parse(&cof(2, 1, 1, &[1, 0], &[1], &[1, 0])).unwrap();
    assert_eq!(c.events, vec![1]);
    assert!(c.event_padding.is_empty());
    assert_eq!(c.draw_order, vec![1, 0]);
}

#[test]
fn cof_component_at_rejects_slot_past_layers() {
    // Slots are 0..L-1: L=1, F=2 has no slot 1 in frame 0.
    let c = Cof::parse(&cof(1, 2, 1, &[1], &[0, 0], &[1, 2])).unwrap();
    assert_eq!(c.component_at(0, 0, 0), Some(1));
    assert_eq!(c.component_at(0, 1, 0), Some(2));
    assert_eq!(c.component_at(0, 0, 1), None);
}

// ---------------------------------------------------------------- animdata

#[test]
fn animdata_query_of_eight_characters_is_not_too_long() {
    // Only a query longer than 8 is an error (and only in a non-empty bucket).
    let name = b"AAAAAAA"; // bucket 455 % 256 = 199
    let b = animdata::hash(name);
    assert_eq!(b, 199);
    let mut d = Vec::new();
    for i in 0..animdata::BUCKETS {
        if i == b {
            p32(&mut d, 1);
            let mut r = vec![0u8; animdata::RECORD_SIZE];
            r[..7].copy_from_slice(name);
            d.extend_from_slice(&r);
        } else {
            p32(&mut d, 0);
        }
    }
    let a = AnimData::parse(&d).unwrap();
    assert_eq!(animdata::hash(b"YYYYYYYX"), 199);
    assert_eq!(a.find(b"YYYYYYYX").unwrap(), None);
    assert_eq!(animdata::hash(b"OOOOOOOOO"), 199);
    assert!(a.find(b"OOOOOOOOO").is_err());
}

// ---------------------------------------------------------------- palette

const PL2_FIXED: usize = 439_808;

fn pl2(text_colors: usize) -> Vec<u8> {
    (0..PL2_FIXED + 259 * text_colors)
        .map(|i| (i % 251) as u8)
        .collect()
}

#[test]
fn pl2_with_no_text_colors() {
    // remaining = 259 x T with T = 0.
    let p = Pl2::parse(&pl2(0)).unwrap();
    assert!(p.text_colors.is_empty() && p.text_color_shifts.is_empty());
}

#[test]
fn pl2_with_one_text_color() {
    let p = Pl2::parse(&pl2(1)).unwrap();
    assert_eq!(p.text_colors.len(), 1);
    assert_eq!(p.text_color_shifts.len(), 1);
}

#[test]
fn pl2_text_color_shifts_follow_the_colors() {
    // T colors of 3 bytes, then T shift maps of 256.
    let d = pl2(2);
    let p = Pl2::parse(&d).unwrap();
    let f = PL2_FIXED;
    assert_eq!(
        (p.text_colors[0].r, p.text_colors[0].g, p.text_colors[0].b),
        (d[f], d[f + 1], d[f + 2])
    );
    assert_eq!(p.text_colors[1].b, d[f + 5]);
    assert_eq!(&p.text_color_shifts[0][..], &d[f + 6..f + 262]);
    assert_eq!(&p.text_color_shifts[1][..], &d[f + 262..f + 518]);
}

// ---------------------------------------------------------------- tbl

#[test]
fn tbl_key_hash_folds_high_nibble() {
    // Key hash: expected values computed by hand.
    assert_eq!(key_hash(b"WarrivAct1Intro"), 0x0ED0_A1AF);
    assert_eq!(key_hash(b"ABCDEFGHIJ"), 0x089E_EAAA);
}

fn tbl_header(num_elements: u16, size: u32, file_size: u32) -> Vec<u8> {
    let mut d = Vec::new();
    p16(&mut d, 0);
    p16(&mut d, num_elements);
    p32(&mut d, size);
    d.push(1);
    p32(&mut d, 0);
    p32(&mut d, 1);
    p32(&mut d, file_size);
    d
}

#[test]
fn tbl_hash_table_ending_the_file() {
    // One empty 17-byte slot right up to the end of the file.
    let mut d = tbl_header(0, 1, 38);
    d.extend_from_slice(&[0; 17]);
    let t = StringTable::parse(&d).unwrap();
    assert_eq!(t.entries.len(), 1);
    assert_eq!(t.entries[0].used, 0);
}

#[test]
fn tbl_strings_adding_up_to_the_file_length() {
    // Strings: total key+value bytes may equal the file length. Two slots
    // share one 56-byte key; the file is 21 + 2 x 17 + 57 = 112 bytes.
    let mut d = tbl_header(0, 2, 112);
    for _ in 0..2 {
        d.push(1);
        p16(&mut d, 0);
        p32(&mut d, 0);
        p32(&mut d, 55); // key offset
        p32(&mut d, 55); // value offset
        p16(&mut d, 0); // empty value
    }
    d.extend_from_slice(&[b'K'; 56]);
    d.push(0);
    assert_eq!(d.len(), 112);
    let t = StringTable::parse(&d).unwrap();
    assert!(t
        .entries
        .iter()
        .all(|e| e.used == 1 && e.key == [b'K'; 56] && e.value.is_empty()));
}
