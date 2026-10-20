//! Mutation-testing gaps of `assets::size`: the parsed-files
//! pool counts `size_of` the parser output plus the bytes of its owned
//! buffers (each unit counted). Each
//! value is built with distinct buffer lengths, so every term of the sum
//! shows in the total.

use std::mem::size_of;

use d2_client::assets::size::ByteSize;
use d2_client::assets::TblAsset;
use d2_formats::cof::{Cof, CofLayer};
use d2_formats::dc6::{Dc6, Dc6Frame, Dc6Header};
use d2_formats::dcc::{Dcc, DccDirection, DccFrame};
use d2_formats::ds1::{Ds1, Ds1Group, Ds1Object, Ds1Path, Ds1PathPoint};
use d2_formats::dt1::{Dt1, Dt1Block, Dt1Tile};
use d2_formats::font::FontTable;
use d2_formats::tbl::{StringTable, TblEntry, TblHeader};

fn sz<T>() -> u64 {
    size_of::<T>() as u64
}

#[test]
fn cof_counts_every_buffer() {
    let layer = CofLayer {
        component: 0,
        shadow: 0,
        selectable: 0,
        override_translucency: 0,
        new_translucency: 0,
        weapon_class: *b"hth\0",
    };
    let cof = Cof {
        layers_count: 2,
        frames: 3,
        directions: 1,
        version: 20,
        unknown: [0; 4],
        x_min: 0,
        x_max: 0,
        y_min: 0,
        y_max: 0,
        animation_rate: 256,
        layers: vec![layer; 2],
        events: vec![0; 3],
        event_padding: vec![0; 5],
        draw_order: vec![0; 7],
    };
    assert_eq!(
        cof.byte_size(),
        sz::<Cof>() + 2 * sz::<CofLayer>() + 3 + 5 + 7
    );
}

fn strings() -> StringTable {
    let entry = |key: &[u8], value: &[u8]| TblEntry {
        used: 1,
        index: 0,
        hash: 0,
        key: key.to_vec(),
        value: value.to_vec(),
    };
    StringTable {
        header: TblHeader {
            crc: 0,
            num_elements: 2,
            hash_table_size: 2,
            version: 0,
            data_start: 0,
            max_tries: 0,
            file_size: 0,
        },
        indices: vec![0; 3],
        entries: vec![entry(b"ab", b"cdefg"), entry(b"hijk", b"lmnopqrstuv")],
    }
}

#[test]
fn string_table_counts_indices_entries_and_text() {
    let expected = sz::<StringTable>() + 3 * 2 + 2 * sz::<TblEntry>() + (2 + 5) + (4 + 11);
    assert_eq!(strings().byte_size(), expected);
    assert_eq!(TblAsset::Strings(strings()).byte_size(), expected);
}

#[test]
fn tbl_asset_font_is_the_font_size() {
    let mut v = b"Woo!".to_vec();
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&[0; 6]);
    v.extend_from_slice(&[0; 14 * 2]);
    let font = FontTable::parse(&v).unwrap();
    let expected = font.byte_size();
    assert!(expected > 1);
    assert_eq!(TblAsset::Font(font).byte_size(), expected);
}

fn dc6_frame(pixels: usize) -> Dc6Frame {
    Dc6Frame {
        flip: 0,
        width: 0,
        height: 0,
        offset_x: 0,
        offset_y: 0,
        unknown: 0,
        next_block: 0,
        pixels: vec![0; pixels],
    }
}

#[test]
fn dc6_counts_frames_and_pixels() {
    let dc6 = Dc6 {
        header: Dc6Header {
            version: 6,
            flags: 1,
            encoding: 0,
            termination: [0xEE; 4],
            directions: 1,
            frames_per_direction: 2,
        },
        frames: vec![dc6_frame(3), dc6_frame(10)],
    };
    assert_eq!(dc6.byte_size(), sz::<Dc6>() + 2 * sz::<Dc6Frame>() + 3 + 10);
}

fn dcc_frame(pixels: usize, optional: usize) -> DccFrame {
    DccFrame {
        variable0: 0,
        width: 0,
        height: 0,
        x_offset: 0,
        y_offset: 0,
        coded_bytes: 0,
        bottom_up: false,
        optional_data: vec![0; optional],
        x_min: 0,
        y_min: 0,
        pixels: vec![0; pixels],
    }
}

fn dcc_direction(frames: Vec<DccFrame>) -> DccDirection {
    DccDirection {
        outsize_coded: 0,
        compression_flags: 0,
        x_min: 0,
        y_min: 0,
        width: 0,
        height: 0,
        frames,
        pcd_leftover_bits: 0,
    }
}

#[test]
fn dcc_counts_directions_frames_pixels_and_optional_data() {
    let dcc = Dcc {
        version: 6,
        frames_per_direction: 2,
        tag: 0,
        final_dc6_size: 0,
        directions: vec![
            dcc_direction(vec![dcc_frame(5, 2), dcc_frame(11, 0)]),
            dcc_direction(vec![dcc_frame(17, 3)]),
        ],
    };
    let expected =
        sz::<Dcc>() + 2 * sz::<DccDirection>() + 3 * sz::<DccFrame>() + (5 + 2) + 11 + (17 + 3);
    assert_eq!(dcc.byte_size(), expected);
}

fn dt1_block(pixels: usize) -> Dt1Block {
    Dt1Block {
        x: 0,
        y: 0,
        unknown1: 0,
        grid_x: 0,
        grid_y: 0,
        format: 0,
        unknown2: 0,
        pixels: vec![0; pixels],
    }
}

fn dt1_tile(blocks: Vec<Dt1Block>) -> Dt1Tile {
    Dt1Tile {
        light_direction: 0,
        roof_height: 0,
        material_flags: 0,
        height: 0,
        width: 0,
        unknown_height: 0,
        orientation: 0,
        main_index: 0,
        sub_index: 0,
        rarity: 0,
        unknown_color: 0,
        subtile_flags: [0; 25],
        unknown_58: 0,
        cache_index: 0,
        unknown_5c: 0,
        blocks,
    }
}

#[test]
fn dt1_counts_tiles_blocks_and_pixels() {
    let dt1 = Dt1 {
        version: 7,
        minor_version: 6,
        tiles: vec![
            dt1_tile(vec![dt1_block(7), dt1_block(13)]),
            dt1_tile(vec![dt1_block(19)]),
        ],
    };
    let expected = sz::<Dt1>() + 2 * sz::<Dt1Tile>() + 3 * sz::<Dt1Block>() + 7 + 13 + 19;
    assert_eq!(dt1.byte_size(), expected);
}

#[test]
fn ds1_counts_every_buffer() {
    let point = Ds1PathPoint {
        x: 0,
        y: 0,
        action: 0,
    };
    let ds1 = Ds1 {
        version: 18,
        width: 1,
        height: 1,
        act: 0,
        tag_type: 0,
        files: vec![b"abc".to_vec(), b"defgh".to_vec()],
        unknown_header: None,
        walls: vec![vec![0; 2], vec![0; 3]],
        orientations: vec![vec![0; 5]],
        floors: vec![vec![0; 7], vec![0; 11], vec![0; 13]],
        shadow: vec![0; 17],
        tags: Some(vec![0; 19]),
        objects: vec![
            Ds1Object {
                kind: 0,
                id: 0,
                x: 0,
                y: 0,
                flags: 0,
            };
            2
        ],
        unknown_groups: None,
        groups: vec![
            Ds1Group {
                x: 0,
                y: 0,
                width: 0,
                height: 0,
                unknown: 0,
            };
            3
        ],
        groups_truncated: false,
        paths: vec![
            Ds1Path {
                x: 0,
                y: 0,
                points: vec![point.clone(); 2],
            },
            Ds1Path {
                x: 0,
                y: 0,
                points: vec![point; 5],
            },
        ],
        trailing: vec![0; 23],
    };
    let layer = sz::<Vec<u32>>();
    let expected = sz::<Ds1>()
        + 2 * sz::<Vec<u8>>()
        + (3 + 5)
        + (2 * layer + 4 * (2 + 3))
        + (layer + 4 * 5)
        + (3 * layer + 4 * (7 + 11 + 13))
        + 4 * 17
        + 4 * 19
        + 2 * sz::<Ds1Object>()
        + 3 * sz::<Ds1Group>()
        + 2 * sz::<Ds1Path>()
        + 7 * sz::<Ds1PathPoint>()
        + 23;
    assert_eq!(ds1.byte_size(), expected);
    let untagged = Ds1 { tags: None, ..ds1 };
    assert_eq!(untagged.byte_size(), expected - 4 * 19);
}

#[test]
fn palette_is_its_fixed_size() {
    use d2_formats::palette::{Palette, Rgb};

    let p = Palette {
        colors: [Rgb::default(); 256],
    };
    assert_eq!(p.byte_size(), sz::<Palette>());
    assert!(p.byte_size() > 1);
}

#[test]
fn pl2_counts_every_map_list() {
    use d2_formats::palette::{Palette, Pl2, Rgb};

    let maps = |n: usize| vec![[0u8; 256]; n];
    let pl2 = Pl2 {
        base_palette: Palette {
            colors: [Rgb::default(); 256],
        },
        light_levels: maps(1),
        inventory_variations: maps(2),
        selected_unit_shift: [0; 256],
        alpha_blend: vec![maps(3), maps(4)],
        additive_blend: maps(5),
        multiplicative_blend: maps(6),
        hue_variations: maps(7),
        red_tones: [0; 256],
        green_tones: [0; 256],
        blue_tones: [0; 256],
        unknown_variations: maps(8),
        max_component_blend: maps(9),
        darkened_shift: [0; 256],
        text_colors: vec![Rgb::default(); 10],
        text_color_shifts: maps(11),
    };
    let expected = sz::<Pl2>()
        + 256 * (1 + 2 + 3 + 4 + 5 + 6 + 7 + 8 + 9 + 11)
        + 2 * sz::<Vec<[u8; 256]>>()
        + 10 * sz::<Rgb>();
    assert_eq!(pl2.byte_size(), expected);
}
