//! A synthetic install the whole play client starts on ([`playable`]):
//! the five-act set ([`crate::acts::all_acts`]) plus the few files and
//! rows the client's own start reads beyond what the server needs. With
//! it, the headless play client (the `repro` runner, `autoplay-host`)
//! joins a town and runs ticks in CI, where there are no game files.
//!
//! What the client reads on top of the server's set, all made up here:
//!
//! - the five act palettes (`pal.pl2`, full size with the text-colour
//!   maps the UI pushes per act);
//! - the walk-back field (`ExpField.D2`) the floor-drop search reads;
//! - the 14 UI fonts, each a `.tbl` of 256 glyphs and a 256-cel DC6;
//! - the UI images a town frame draws with no panel open: the control
//!   panel and the cursor;
//! - rows the client's table checks expect: `pettype` up to row 7 (the
//!   hireling table reads it) and a waypoint object (operate 23, init 17).
//!
//! Every byte is generated: a pattern for the palettes, flat 8 × 8 cels
//! for the art. The files go in [`Synthetic::stored`]. Opening a panel
//! draws more images than listed here; a test that opens one adds its
//! own stand-ins with [`stand_in_dc6`].

use crate::sprites::{dc6_file, dc6_frames, Dc6Shape};
use crate::synth::Synthetic;

/// The UI font files (in `data\local\font\latin\`), without extension.
pub const FONTS: [&str; 14] = [
    "font8",
    "font16",
    "font30",
    "font42",
    "fontformal10",
    "fontformal12",
    "font6",
    "font24",
    "fontformal11",
    "fontexocet10",
    "fontridiculous",
    "fontexocet8",
    "reallythelastsucker",
    "fontingamechat",
];

/// The UI images a town frame draws with no panel open.
pub const HUD_IMAGES: [&str; 2] = [
    r"data\global\ui\panel\800ctrlpnl7.dc6",
    r"data\global\ui\cursor\protate.dc6",
];

/// Bytes of an act palette: the 256-colour base, the 1714 shift and blend
/// maps, then the 13 text colours and their 13 maps the UI reads.
pub const PL2_BYTES: usize = 443_175;

/// The cels per stand-in UI image: more than any HUD file's frame index.
const STAND_IN_FRAMES: u32 = 64;

/// The play client's install: [`crate::acts::all_acts`] plus the files
/// and rows of the module docs.
pub fn playable() -> Synthetic {
    let mut data = crate::acts::all_acts();
    pad_rows(&mut data, "pettype.txt", "pet type", 8);
    let objects = data.tables.files.get_mut("objects.txt").expect("objects");
    for (name, value) in [("OperateFn", "23"), ("InitFn", "17")] {
        let c = column(&objects.columns, name);
        objects.rows[0][c] = value.into();
    }
    for act in 1..=5 {
        data.stored.push((
            format!(r"data\global\palette\act{act}\pal.pl2"),
            palette(act),
        ));
    }
    data.stored
        .push((r"data\global\ExpField.D2".into(), walk_back_field()));
    let glyphs = font_table();
    let cels = dc6_file(&dc6_frames(cel_shape(256), 7), 1, 256);
    for f in FONTS {
        let dir = r"data\local\font\latin\";
        data.stored.push((format!("{dir}{f}.tbl"), glyphs.clone()));
        data.stored.push((format!("{dir}{f}.dc6"), cels.clone()));
    }
    for name in HUD_IMAGES {
        data.stored.push((name.into(), stand_in_dc6()));
    }
    data
}

/// A stand-in UI image: one direction of [`STAND_IN_FRAMES`] 8 × 8 cels.
pub fn stand_in_dc6() -> Vec<u8> {
    dc6_file(
        &dc6_frames(cel_shape(STAND_IN_FRAMES), 7),
        1,
        STAND_IN_FRAMES,
    )
}

fn cel_shape(frames: u32) -> Dc6Shape {
    Dc6Shape {
        directions: 1,
        frames,
        width: 8,
        height: 8,
        panel: true,
    }
}

fn column(columns: &[String], name: &str) -> usize {
    columns
        .iter()
        .position(|c| c == name)
        .unwrap_or_else(|| panic!("no column {name}"))
}

/// Copies row 0 of `file` until it has `n` rows, each with its own `key`.
fn pad_rows(data: &mut Synthetic, file: &str, key: &str, n: usize) {
    let t = data.tables.files.get_mut(file).expect(file);
    let k = column(&t.columns, key);
    let mut row = t.rows[0].clone();
    while t.rows.len() < n {
        row[k] = format!("pad{}", t.rows.len());
        t.rows.push(row.clone());
    }
}

/// An act palette: a byte pattern per act, the base colours' unused
/// fourth byte 0.
fn palette(act: usize) -> Vec<u8> {
    let mut v: Vec<u8> = (0..PL2_BYTES)
        .map(|i| (i * (act + 2) % 251) as u8)
        .collect();
    for c in v[..1024].chunks_mut(4) {
        c[3] = 0;
    }
    v
}

/// A walk-back field of 256 × 256 cells, every cell 8 (no step: the
/// floor drop stays where it is).
fn walk_back_field() -> Vec<u8> {
    let mut v = vec![0x0A, 0x01];
    v.extend_from_slice(&256u32.to_le_bytes());
    v.extend_from_slice(&256u32.to_le_bytes());
    v.extend(std::iter::repeat_n(8u8, 256 * 256));
    v
}

/// A font table: 256 glyphs, each 6 wide and 10 high, glyph `i` drawn
/// with cel `i`.
fn font_table() -> Vec<u8> {
    let mut d = b"Woo!".to_vec();
    d.extend_from_slice(&1u16.to_le_bytes());
    d.extend_from_slice(&0u16.to_le_bytes());
    d.extend_from_slice(&256u16.to_le_bytes());
    d.extend_from_slice(&[10, 0]);
    for i in 0..256u16 {
        d.extend_from_slice(&i.to_le_bytes());
        d.extend_from_slice(&[0, 6, 10, 0, 0, 0]);
        d.extend_from_slice(&i.to_le_bytes());
        d.extend_from_slice(&[0; 4]);
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_playable_set_carries_the_client_files() {
        let d = playable();
        let has = |n: &str| d.stored.iter().any(|(f, _)| f == n);
        assert!(has(r"data\global\ExpField.D2"));
        assert!(has(r"data\local\font\latin\font8.tbl"));
        for n in HUD_IMAGES {
            assert!(has(n), "{n}");
        }
        let pal = d
            .stored
            .iter()
            .find(|(f, _)| f.ends_with(r"act3\pal.pl2"))
            .expect("act 3 palette");
        assert_eq!(pal.1.len(), PL2_BYTES);
        assert_eq!(d.tables.file("pettype.txt").rows.len(), 8);
    }
}
